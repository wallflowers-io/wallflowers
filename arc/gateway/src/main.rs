//! arc-gateway — the thin `/v1` router, the Arc's single public origin.
//!
//! It owns the `/v1` contract and nothing else: capability discovery (`/v1/health`,
//! `/v1/arc`), build provenance (`/v1/version`) and reverse-proxying each request to the plane
//! process behind it —
//! relay (WebSocket), systems, membership. Planes stay independent
//! processes (independent failure and scale); the gateway is the only public port.
//! See docs/arc-api-icd-v1.html for the normative contract.
//!
//! Upstreams are configured by env (defaults match arcup's local ports):
//!   ARC_RELAY_UPSTREAM       ws://127.0.0.1:8787
//!   ARC_SYSTEMS_UPSTREAM     http://127.0.0.1:8795
//!   ARC_MEMBERSHIP_UPSTREAM  http://127.0.0.1:8790
//! A plane whose upstream is unset reports `live:false` and answers 501 if called —
//! honest capability, never a fabricated plane.
//!
//!   ARC_BUILD_MANIFEST       /opt/arc/build.json   (the last deploy's five-plane stamp)

use std::sync::Arc;

use axum::{
    body::Body,
    extract::ws::{Message as AxMsg, WebSocket},
    extract::{Path, Request, State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get, post},
    Router,
};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message as WsMsg;

/// What arc-node last told us it serves, and when. The node folds every Host under one
/// lock and re-opens its store per request, so the gateway asks at most every
/// FACES_TTL and renders from memory: a face is small, and a public page must not
/// queue behind a sync.
#[derive(Default)]
struct Faces {
    at: Option<std::time::Instant>,
    doc: serde_json::Value,
}

const FACES_TTL: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Clone)]
struct Gateway {
    http: reqwest::Client,
    /// The rate bound on /v1/admit.
    admit_rate: Arc<Rate>,
    relay: Option<String>,
    systems: Option<String>,
    membership: Option<String>,
    /// The `/v1/land` plane — UK land tenure for Place anchors. Unset on most Arcs: it needs
    /// gigabytes of ingested HM Land Registry data, so it is expected to live on the central
    /// Pacific Arc and report `live:false` everywhere else rather than be faked.
    land: Option<String>,
    /// The `/v1/box` plane — event ticketing (the box office). The central box office is
    /// stopgap infra by the relay precedent; nothing in the protocol assumes a singleton.
    boxoffice: Option<String>,
    /// The `/v1/wake` plane — APNs push WITHOUT un-blinding the relay (RINGFENCE §6). A
    /// separate tier holding the token↔tag registrations and the APNs credential; it
    /// subscribes to the relay as an ordinary client and never decrypts anything.
    waker: Option<String>,
    /// Path to the curated directory of Arcs this Arc knows (ARC_ARCS_FILE). The central
    /// Pacific Arc carries the full official list; a community Arc its own + its peers.
    arcs_file: Option<String>,
    /// The faces this Arc serves, as arc-node last folded them (see [`Faces`]).
    faces: Arc<tokio::sync::Mutex<Faces>>,
    /// Directory holding the built console SPA (ARC_CONSOLE_DIR; the image bakes it at
    /// /opt/arc/console). Served by the fallback: this Arc IS the console's origin.
    console_dir: Option<String>,
    /// Path to the deploy manifest served at `GET /v1/version` (ARC_BUILD_MANIFEST). Written by
    /// hosting/deploy-arc.sh AFTER the binary swap, by asking each newly-installed plane
    /// `--version`, so it reports what is on disk rather than what the deploy meant to put
    /// there. Unreadable or absent → the route answers `"deploy": null` and says why: the
    /// gateway's own compiled-in stamp is served either way, so /v1/version is never blank.
    build_manifest: Option<String>,
    /// The console gate's expected `Authorization` value — `Basic <b64(user:pass)>`, precomputed
    /// from ARC_CONSOLE_USER/ARC_CONSOLE_PASS at startup so the check is a single constant-time
    /// compare and the plaintext never sits in the struct. `None` (either var unset) fails
    /// CLOSED: the console surface answers 401 until the operator sets both.
    console_basic: Option<String>,
    /// The webapp's origin (ARC_APP_ORIGIN, e.g. https://app.wallflowers.io): the one origin
    /// GET /v1/bundle answers CORS for, so the webapp can hand a fresh bundle to the Door's
    /// /v2/add. Unset: no CORS on it, as before.
    app_origin: Option<String>,
}

fn env_opt(key: &str) -> Option<String> {
    std::env::var(key).ok().map(|s| s.trim().trim_end_matches('/').to_string()).filter(|s| !s.is_empty())
}

fn env_or(key: &str, default: &str) -> Option<String> {
    Some(env_opt(key).unwrap_or_else(|| default.to_string()))
}

/// A `{ "error": { "code", "message" } }` envelope with an HTTP status (ICD §11).
fn err(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    let body = serde_json::json!({ "error": { "code": code, "message": message.into() } });
    (status, [(axum::http::header::CONTENT_TYPE, "application/json")], body.to_string()).into_response()
}

impl Gateway {
    /// Forward `method + headers + body` to `base + upstream_path (+ query)`, streaming the
    /// response straight back. The single reverse-proxy primitive under every proxied route.
    async fn forward(&self, base: &Option<String>, upstream_path: &str, req: Request) -> Response {
        let Some(base) = base else {
            return err(StatusCode::NOT_IMPLEMENTED, "not_configured", "this plane is not provisioned on this Arc");
        };
        let (parts, body) = req.into_parts();
        let query = parts.uri.query().map(|q| format!("?{q}")).unwrap_or_default();
        let url = format!("{base}{upstream_path}{query}");

        let bytes = match axum::body::to_bytes(body, 32 * 1024 * 1024).await {
            Ok(b) => b,
            Err(_) => return err(StatusCode::BAD_REQUEST, "bad_request", "could not read request body"),
        };
        let mut rb = self.http.request(parts.method, &url).body(bytes);
        for (k, v) in parts.headers.iter() {
            // Host is set by reqwest for the upstream; content-length is recomputed.
            if k == axum::http::header::HOST || k == axum::http::header::CONTENT_LENGTH {
                continue;
            }
            rb = rb.header(k, v);
        }
        match rb.send().await {
            Ok(resp) => {
                let status = resp.status();
                let mut out = Response::builder().status(status);
                if let Some(h) = out.headers_mut() {
                    for (k, v) in resp.headers().iter() {
                        if k == axum::http::header::TRANSFER_ENCODING {
                            continue; // axum re-frames the body
                        }
                        h.insert(k.clone(), v.clone());
                    }
                }
                out.body(Body::from_stream(resp.bytes_stream())).unwrap_or_else(|_| {
                    err(StatusCode::BAD_GATEWAY, "upstream_failed", "malformed upstream response")
                })
            }
            Err(e) => err(StatusCode::BAD_GATEWAY, "upstream_failed", e.to_string()),
        }
    }

    /// Probe an HTTP plane's `/health` — honest liveness for the capability document.
    async fn probe(&self, base: &Option<String>) -> bool {
        let Some(base) = base else { return false };
        self.http
            .get(format!("{base}/health"))
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

/// GET /v1/health — aggregate liveness of the Arc and each plane.
async fn health(State(gw): State<Arc<Gateway>>) -> Response {
    let (systems, membership, land, boxoffice, waker) = tokio::join!(
        gw.probe(&gw.systems),
        gw.probe(&gw.membership),
        gw.probe(&gw.land),
        gw.probe(&gw.boxoffice),
        gw.probe(&gw.waker)
    );
    let body = serde_json::json!({
        "status": "ok",
        "planes": {
            "relay":     { "live": gw.relay.is_some() },
            "systems":   { "live": systems },
            "land":      { "live": land },
            "box":       { "live": boxoffice },
            // `live` here is the PROCESS, not the credential — the waker's own
            // /v1/wake/health reports `waker:false` when APNs is unconfigured.
            "wake":      { "live": waker },
            "membership":{ "live": membership },
        }
    });
    (StatusCode::OK, [(axum::http::header::CONTENT_TYPE, "application/json")], body.to_string()).into_response()
}

/// GET /v1/arc — the Arc's identity (proxied from membership) augmented with a live
/// `planes` capability block, so a client learns everything from one call (ICD §4).
// ---- a group's public face ---------------------------------------------------------
//
// The Arc serves what a Host published, at the group's slug: `wallflowers.io/<slug>`
// proxies here (core/docs/the-face-on-the-wire.md). Everything on the page came off the
// Host's own log, put there by the group's owner on purpose — the Arc is a member of the
// Host and never of the group — and it is drawn by `face-render`, which emits no script.
//
// ONE 404 FOR EVERY MISS: no such slug, not published, not ours, the Host will not fold.
// A distinct answer would let anyone enumerate what this Arc holds.

/// Ask arc-node what it serves, at most every [`FACES_TTL`].
async fn faces_doc(gw: &Gateway) -> serde_json::Value {
    let mut cache = gw.faces.lock().await;
    let fresh = cache.at.map(|t| t.elapsed() < FACES_TTL).unwrap_or(false);
    if !fresh {
        if let Some(base) = &gw.membership {
            match gw.http.get(format!("{base}/faces")).send().await {
                Ok(r) => {
                    if let Ok(doc) = r.json::<serde_json::Value>().await {
                        cache.doc = doc;
                        cache.at = Some(std::time::Instant::now());
                    }
                }
                Err(e) => eprintln!("arc-gateway: faces: arc-node did not answer: {e}"),
            }
        }
    }
    cache.doc.clone()
}

/// The site at `slug`, if this Arc serves it.
async fn face_of(gw: &Gateway, slug: &str) -> Option<serde_json::Value> {
    let doc = faces_doc(gw).await;
    doc["sites"].as_array()?.iter().find(|s| s["slug"].as_str() == Some(slug)).cloned()
}

/// A face's short cache: a new publish shows within the minute.
const FACE_SHORT: &str = "public, max-age=60";

fn face_headers(cache: &str) -> [(axum::http::HeaderName, &'static str); 6] {
    use axum::http::header::*;
    [
        (CONTENT_TYPE, "text/html; charset=utf-8"),
        (CACHE_CONTROL, if cache == "long" { "public, max-age=31536000, immutable" } else { FACE_SHORT }),
        (X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (REFERRER_POLICY, "strict-origin-when-cross-origin"),
        // The page has no script of its own and loads nothing from another origin.
        (CONTENT_SECURITY_POLICY, "default-src 'none'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"),
        (HeaderName::from_static("permissions-policy"), "camera=(), microphone=(), geolocation=(), payment=(), usb=()"),
    ]
}

/// The page's miss, one 404 for every face route, readable from any origin as a hit is: a browser
/// app asking for a community not yet published sees the 404, not "Failed to fetch".
fn no_face() -> Response {
    use axum::http::header::{ACCESS_CONTROL_ALLOW_ORIGIN, CONTENT_TYPE};
    (StatusCode::NOT_FOUND, [(CONTENT_TYPE, "text/plain; charset=utf-8"), (ACCESS_CONTROL_ALLOW_ORIGIN, "*")], "no such page\n").into_response()
}

/// `(the parsed bundle, the slots it has, its live items)`.
fn parse_site(site: &serde_json::Value) -> Option<(face_render::Face, Vec<String>, Vec<(String, String)>)> {
    let name = site["name"].as_str().unwrap_or("");
    let face = match face_render::parse(site["bundle"].as_str().unwrap_or(""), name) {
        Ok(f) => f,
        Err(e) => {
            eprintln!(
                "arc-gateway: faces: the bundle published at /{} will not parse: {e}",
                site["slug"].as_str().unwrap_or("?")
            );
            return None;
        }
    };
    let media = site["media"].as_array().map(|a| a.iter().filter_map(|m| m.as_str().map(String::from)).collect()).unwrap_or_default();
    let items = site["items"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|i| Some((i["key"].as_str()?.to_string(), i["payload"].as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    Some((face, media, items))
}

fn in_app(headers: &HeaderMap) -> bool {
    let ua = headers.get(axum::http::header::USER_AGENT).and_then(|h| h.to_str().ok()).unwrap_or("");
    let xrw = headers.get("x-requested-with").and_then(|h| h.to_str().ok());
    face_render::is_in_app_browser(ua, xrw)
}

async fn face_page(State(gw): State<Arc<Gateway>>, Path(slug): Path<String>, headers: HeaderMap) -> Response {
    let Some(site) = face_of(&gw, &slug).await else { return no_face() };
    let Some((face, media, items)) = parse_site(&site) else { return no_face() };
    let slots: Vec<&str> = media.iter().map(String::as_str).collect();
    let ctx = face_render::Ctx {
        slug: &slug,
        media: &slots,
        items: &items,
        members: None,
        in_app: in_app(&headers),
        now_ms: now_ms() as i64,
    };
    (StatusCode::OK, face_headers("short"), face_render::render_face(&face, &ctx)).into_response()
}

/// The Site's live items as data, for a site of its own to draw (EGREGORE's): what
/// `face_page` draws from, by key, each its source op's args by ICD name plus `at` (core
/// face_items). Public, as the page is, so any origin may read it.
async fn face_items(State(gw): State<Arc<Gateway>>, Path(slug): Path<String>) -> Response {
    use axum::http::header::*;
    let Some(site) = face_of(&gw, &slug).await else { return no_face() };
    let Some((_, _, items)) = parse_site(&site) else { return no_face() };
    let mut out = serde_json::Map::new();
    for (key, payload) in items.into_iter().filter(|(k, _)| face_render::is_item_key(k)) {
        match serde_json::from_str::<serde_json::Value>(&payload) {
            Ok(v @ serde_json::Value::Object(_)) => {
                out.insert(key, v);
            }
            _ => eprintln!("arc-gateway: faces: /{slug}'s item {key} is not a JSON object; left out"),
        }
    }
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "application/json"), (CACHE_CONTROL, FACE_SHORT), (X_CONTENT_TYPE_OPTIONS, "nosniff"), (ACCESS_CONTROL_ALLOW_ORIGIN, "*")],
        serde_json::json!({ "items": out }).to_string(),
    )
        .into_response()
}

/// A post's public page (W-98 Resources): `wallflowers.io/<slug>/post/<id>`, drawn by
/// face-render from the Host's items for that post, served as the Face is.
async fn face_post(State(gw): State<Arc<Gateway>>, Path((slug, id)): Path<(String, String)>, headers: HeaderMap) -> Response {
    let Some(site) = face_of(&gw, &slug).await else { return no_face() };
    let Some((face, media, items)) = parse_site(&site) else { return no_face() };
    let slots: Vec<&str> = media.iter().map(String::as_str).collect();
    let ctx = face_render::Ctx {
        slug: &slug,
        media: &slots,
        items: &items,
        members: None,
        in_app: in_app(&headers),
        now_ms: now_ms() as i64,
    };
    match face_render::render_post(&face, &ctx, &id) {
        Some(html) => (StatusCode::OK, face_headers("short"), html).into_response(),
        None => no_face(),
    }
}

/// A post's document, the bytes its Host item holds, to be saved: served as an attachment,
/// so a PDF from a member is never drawn on this origin.
async fn face_post_document(State(gw): State<Arc<Gateway>>, Path((slug, id)): Path<(String, String)>) -> Response {
    use axum::http::header::*;
    use base64::Engine as _;
    let Some(site) = face_of(&gw, &slug).await else { return no_face() };
    let Some((_, media, items)) = parse_site(&site) else { return no_face() };
    let slots: Vec<&str> = media.iter().map(String::as_str).collect();
    let ctx = face_render::Ctx { slug: &slug, media: &slots, items: &items, members: None, in_app: false, now_ms: now_ms() as i64 };
    let Some(d) = face_render::post_document(&ctx, &id) else { return no_face() };
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&d.data) else { return no_face() };
    let name: String = d.name.chars().map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' { c } else { '_' }).collect();
    let name = if name.trim().is_empty() { "document.pdf".to_string() } else { name };
    (
        StatusCode::OK,
        [
            (CONTENT_TYPE, "application/pdf".to_string()),
            (CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\"")),
            (CACHE_CONTROL, FACE_SHORT.to_string()),
            (X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
        ],
        bytes,
    )
        .into_response()
}

/// The way OUT of an app's own browser, with this site's name and mark on it.
async fn face_escape(State(gw): State<Arc<Gateway>>, Path(slug): Path<String>) -> Response {
    let Some(site) = face_of(&gw, &slug).await else { return no_face() };
    let Some((face, media, _)) = parse_site(&site) else { return no_face() };
    let html = face_render::render_escape(&slug, &face.name, media.iter().any(|m| m == "mark"));
    (StatusCode::OK, face_headers("short"), html).into_response()
}

async fn face_door(State(gw): State<Arc<Gateway>>, Path(slug): Path<String>) -> Response {
    let Some(site) = face_of(&gw, &slug).await else { return no_face() };
    let Some((face, _, _)) = parse_site(&site) else { return no_face() };
    (StatusCode::OK, face_headers("short"), face_render::render_door(&face, &slug)).into_response()
}

/// What a sign-in window elsewhere wears for this Site (the Door's, when the Site's own page
/// signs a visitor in): the Face's name, its look as CSS custom properties, and whether it has
/// a mark to show. Public, as the page is; the window's server asks, never the visitor's browser.
async fn face_brand(State(gw): State<Arc<Gateway>>, Path(slug): Path<String>) -> Response {
    let Some(site) = face_of(&gw, &slug).await else { return no_face() };
    let Some((face, media, _)) = parse_site(&site) else { return no_face() };
    let body = serde_json::json!({
        "name": face.name,
        "style": face_render::look_style(&face),
        "mark": media.iter().any(|m| m == "mark"),
    });
    (
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, "application/json"),
            (axum::http::header::CACHE_CONTROL, "public, max-age=60"),
            (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        body.to_string(),
    )
        .into_response()
}

/// One of the Host's pictures. Content-addressed it is not, so it is cached briefly
/// rather than for ever: a new publish replaces a slot in place.
async fn face_media(State(gw): State<Arc<Gateway>>, Path((slug, slot)): Path<(String, String)>) -> Response {
    if !matches!(slot.as_str(), "mark" | "cover" | "logo" | "wallpaper") {
        return no_face();
    }
    let Some(site) = face_of(&gw, &slug).await else { return no_face() };
    let (Some(host), Some(base)) = (site["host"].as_str(), gw.membership.as_ref()) else { return no_face() };
    match gw.http.get(format!("{base}/face/{host}/m/{slot}")).send().await {
        Ok(r) if r.status().is_success() => {
            let ctype = r
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|h| h.to_str().ok())
                .filter(|t| matches!(*t, "image/jpeg" | "image/png" | "image/webp"))
                .unwrap_or("application/octet-stream")
                .to_string();
            match r.bytes().await {
                Ok(body) => (
                    StatusCode::OK,
                    [
                        (axum::http::header::CONTENT_TYPE, ctype),
                        (axum::http::header::CACHE_CONTROL, "public, max-age=300".into()),
                        (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
                    ],
                    body,
                )
                    .into_response(),
                Err(_) => no_face(),
            }
        }
        _ => no_face(),
    }
}

async fn arc_identity(State(gw): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    let mut doc = match &gw.membership {
        Some(base) => match gw.http.get(format!("{base}/arc")).send().await {
            Ok(r) => r.json::<serde_json::Value>().await.unwrap_or_else(|_| serde_json::json!({})),
            Err(_) => serde_json::json!({}),
        },
        None => serde_json::json!({}),
    };
    let (systems, land) = tokio::join!(gw.probe(&gw.systems), gw.probe(&gw.land));
    // Advertise the PUBLIC relay path derived from the Host the client reached us on — never
    // the internal upstream. The client reconnects to this same origin's /v1/relay.
    let relay_url = match (gw.relay.is_some(), headers.get(axum::http::header::HOST).and_then(|h| h.to_str().ok())) {
        (true, Some(host)) => serde_json::Value::String(format!("wss://{host}/v1/relay")),
        _ => serde_json::Value::Null,
    };
    // The proxied membership doc carries arc-node's own `relay_url`, which is the INTERNAL
    // upstream ARC_RELAY_URL (`ws://127.0.0.1:8787` under arcup) — useless, and actively
    // misleading, to a client on the other side of the gateway. Mirror the public path over
    // it so both fields say the same reachable thing rather than disagreeing.
    doc["relay_url"] = relay_url.clone();
    doc["planes"] = serde_json::json!({
        "relay":     { "live": gw.relay.is_some(), "url": relay_url },
        "systems":   { "live": systems },
        "land":      { "live": land },
    });
    (StatusCode::OK, [(axum::http::header::CONTENT_TYPE, "application/json")], doc.to_string()).into_response()
}

/// GET /v1/version — WHICH BUILD IS THIS? Provenance, not capability.
///
/// PUBLIC, and that is a deliberate widening of a router whose comment says "identity/discovery +
/// the membership ENTRANCE. Nothing else." The case: this IS identity — not "who is this Arc"
/// (`/v1/arc`) or "what can it do" (`/v1/health`) but "what is running", the third question an
/// operator asks and the only one this Arc could not answer at all. On 18 Sep 2026 the deployed
/// binaries matched no commit in the arc repository and the only way to establish what was live
/// was to probe the wire for behavioural tells. A route that removes that guesswork belongs with
/// the other two, and `site` already serves the same thing unauthenticated at
/// /version.json — gating it here would make the Arc the harder of the two to account for.
///
/// What it discloses is a git SHA, a branch name and a build time. The repository is not public,
/// but a SHA is not a credential and nothing here is reachable by knowing one.
///
/// Two blocks, because they are two different kinds of claim and must not be conflated:
///   `gateway` — compiled INTO this process (lib/arc-build). It cannot be edited on the box
///               without replacing the binary, so it is the gateway's own word about itself.
///   `deploy`  — the manifest the last deploy left on disk, covering all five planes. Each entry
///               is that binary's own `--version` output, collected after the swap. `null` when
///               the file is missing — an Arc that was never deployed by the script, or a hand-
///               placed binary, which is precisely the situation worth being able to see.
///
/// A `gateway` block reading `"stamped": false` means this binary does not know what it is. That
/// is normal for a local `cargo build` and a red flag anywhere else.
async fn version(State(gw): State<Arc<Gateway>>) -> Response {
    let mine: serde_json::Value =
        serde_json::from_str(&arc_build::json("arc-gateway")).unwrap_or_else(|_| serde_json::json!(null));
    // Small, low-volume read; std::fs keeps the gateway's tokio feature set minimal (as /v1/arcs).
    let (deploy, note) = match gw.build_manifest.as_ref().map(|p| (p, std::fs::read_to_string(p))) {
        Some((_, Ok(s))) => match serde_json::from_str::<serde_json::Value>(&s) {
            Ok(v) => (v, serde_json::Value::Null),
            Err(e) => (
                serde_json::Value::Null,
                serde_json::json!(format!("deploy manifest is not valid JSON: {e}")),
            ),
        },
        Some((p, Err(e))) => (
            serde_json::Value::Null,
            serde_json::json!(format!("no deploy manifest at {p}: {e} — these binaries were not placed by hosting/deploy-arc.sh, or predate it")),
        ),
        None => (serde_json::Value::Null, serde_json::json!("ARC_BUILD_MANIFEST unset")),
    };
    let mut body = serde_json::json!({ "gateway": mine, "deploy": deploy });
    if !note.is_null() {
        body["deploy_note"] = note;
    }
    (StatusCode::OK, [(axum::http::header::CONTENT_TYPE, "application/json")], body.to_string()).into_response()
}

/// GET /v1/arcs — the directory of Arcs this Arc knows (member-gated). The central Pacific
/// Arc carries the full official list; a community Arc its own + its peers. Served from the
/// curated `ARC_ARCS_FILE`; honest `{"arcs":[]}` when unset or unreadable.
async fn arcs(State(gw): State<Arc<Gateway>>) -> Response {
    // Small, low-volume (gated) read; std::fs keeps the gateway's tokio feature set minimal.
    let body = gw
        .arcs_file
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| "{\"arcs\":[]}".to_string());
    (StatusCode::OK, [(axum::http::header::CONTENT_TYPE, "application/json")], body).into_response()
}

// -- reverse-proxy route handlers ---------------------------------------------

/// `/v1/land/*` — land tenure lookup. Proxied verbatim; the plane owns its own honesty
/// about what the loaded datasets can and cannot answer (see planes/land).
async fn land(State(gw): State<Arc<Gateway>>, Path(rest): Path<String>, req: Request) -> Response {
    gw.forward(&gw.land.clone(), &format!("/land/{rest}"), req).await
}

/// `/v1/box/*` — the box office (event ticketing). Proxied verbatim WITH its `/v1/box`
/// prefix kept: the plane's signed routes hash the exact public path into the Ed25519
/// canonical string, so the path the client signed must be the path the plane sees.
async fn boxoffice(State(gw): State<Arc<Gateway>>, Path(rest): Path<String>, req: Request) -> Response {
    gw.forward(&gw.boxoffice.clone(), &format!("/v1/box/{rest}"), req).await
}

/// `/v1/wake/*` — the waker (push registration). Proxied verbatim WITH its `/v1/wake`
/// prefix, like the box office, so the plane sees the exact public path.
async fn waker(State(gw): State<Arc<Gateway>>, Path(rest): Path<String>, req: Request) -> Response {
    gw.forward(&gw.waker.clone(), &format!("/v1/wake/{rest}"), req).await
}

async fn systems(State(gw): State<Arc<Gateway>>, Path(rest): Path<String>, req: Request) -> Response {
    gw.forward(&gw.systems.clone(), &format!("/systems/{rest}"), req).await
}
async fn signup(State(gw): State<Arc<Gateway>>, req: Request) -> Response {
    gw.forward(&gw.membership.clone(), "/signup", req).await
}
/// A fresh contact bundle from this Arc's node. CORS for the webapp's origin alone
/// (ARC_APP_ORIGIN, W-103), so the webapp can hand it to the Door's /v2/add: that exact origin,
/// Vary: Origin, no credentials. Any other origin is answered as before.
async fn bundle(State(gw): State<Arc<Gateway>>, req: Request) -> Response {
    use axum::http::{header::{ACCESS_CONTROL_ALLOW_ORIGIN, ORIGIN, VARY}, HeaderValue};
    let theirs = req.headers().get(ORIGIN).and_then(|v| v.to_str().ok()).map(str::to_string);
    let mut r = gw.forward(&gw.membership.clone(), "/bundle", req).await;
    if let (Some(app), Some(o)) = (gw.app_origin.as_deref(), theirs.as_deref()) {
        if o == app {
            if let Ok(v) = HeaderValue::from_str(app) {
                r.headers_mut().insert(ACCESS_CONTROL_ALLOW_ORIGIN, v);
                r.headers_mut().append(VARY, HeaderValue::from_static("Origin"));
            }
        }
    }
    r
}
async fn console(State(gw): State<Arc<Gateway>>, Path(rest): Path<String>, req: Request) -> Response {
    gw.forward(&gw.membership.clone(), &format!("/console/{rest}"), req).await
}
/// Admission by a kiosk claim (A-3). Public: the claim IS the credential, and the first
/// to present it wins (DV-5). Held to ADMIT_PER_MINUTE per client network, since every
/// call runs a signature check.
async fn admit(State(gw): State<Arc<Gateway>>, req: Request) -> Response {
    let who = req
        .headers()
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("direct")
        .to_string();
    if !gw.admit_rate.allow(&who) {
        return err(StatusCode::TOO_MANY_REQUESTS, "rate_limited", "too many admissions from this network; wait a minute");
    }
    gw.forward(&gw.membership.clone(), "/admit", req).await
}

/// At most `ADMIT_PER_MINUTE` admissions per client network per minute. Every visitor's
/// admission comes from the Door's one address, so this is the whole show's rate after a
/// performance; it bounds one Ed25519 verify per call, which costs microseconds
/// (Software Security, 27 Sep: about 600).
const ADMIT_PER_MINUTE: u32 = 600;

#[derive(Default)]
struct Rate {
    seen: std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, u32)>>,
}

impl Rate {
    fn allow(&self, who: &str) -> bool {
        let mut seen = self.seen.lock().unwrap();
        let now = std::time::Instant::now();
        seen.retain(|_, (at, _)| now.duration_since(*at) < std::time::Duration::from_secs(60));
        let e = seen.entry(who.to_string()).or_insert((now, 0));
        e.1 += 1;
        e.1 <= ADMIT_PER_MINUTE
    }
}

async fn tether(State(gw): State<Arc<Gateway>>, req: Request) -> Response {
    gw.forward(&gw.membership.clone(), "/tether", req).await
}

fn not_found() -> Response {
    err(StatusCode::NOT_FOUND, "not_found", "unknown route — see /v1/arc for this Arc's capabilities")
}

// -- console gate (HTTP Basic) ------------------------------------------------

/// Constant-time equality — the compare never short-circuits on a matching prefix, so response
/// timing leaks nothing about how much of the credential was right. (Length still gates entry,
/// but the encoded length only reveals roughly how long user+pass are, which is not secret.)
fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Does this request carry the console credential? The header is compared VERBATIM against the
/// precomputed `Basic <b64>` — no decode, no parse, nothing allocated from attacker input.
fn console_cred_ok(gw: &Gateway, headers: &HeaderMap) -> bool {
    let Some(expect) = &gw.console_basic else { return false }; // unset → fail closed
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|got| ct_eq(got.trim(), expect))
        .unwrap_or(false)
}

/// 401 + `WWW-Authenticate: Basic` — the browser answers this with its native login prompt,
/// which is what lets the SPA stay credential-unaware: the prompt happens at document load and
/// the browser attaches the header to every same-origin fetch after it.
fn basic_challenge() -> Response {
    let body = serde_json::json!({ "error": { "code": "unauthorized", "message": "console credentials required" } });
    (
        StatusCode::UNAUTHORIZED,
        [
            (axum::http::header::WWW_AUTHENTICATE, r#"Basic realm="Arc Console", charset="UTF-8""#),
            (axum::http::header::CONTENT_TYPE, "application/json"),
        ],
        body.to_string(),
    )
        .into_response()
}

/// The console surface's gate: one shared username/password (ARC_CONSOLE_USER/PASS service
/// vars) in front of the SPA and its same-origin API. A stopgap credential — one secret, no
/// identity, no roles — until the SPA can hold a member credential and ride the Admin-gated
/// /v1/console/* front door like any other client.
async fn console_gate(State(gw): State<Arc<Gateway>>, req: Request, next: Next) -> Response {
    if console_cred_ok(&gw, req.headers()) {
        next.run(req).await
    } else {
        basic_challenge()
    }
}

// -- console SPA (static) -----------------------------------------------------

/// Map a request path to a file inside the console dist dir. `/` maps to `index.html`;
/// any path with an empty/`.`/`..`/backslash segment is rejected (the raw path is not
/// percent-decoded, so an encoded `..` stays a literal filename and cannot traverse).
fn console_asset(dir: &str, uri_path: &str) -> Option<std::path::PathBuf> {
    // Exactly ONE leading slash comes off — a second becomes an empty segment and is refused.
    let rel = uri_path.strip_prefix('/').unwrap_or(uri_path);
    if rel.is_empty() {
        return Some(std::path::Path::new(dir).join("index.html"));
    }
    if rel.split('/').any(|s| s.is_empty() || s == "." || s == ".." || s.contains('\\')) {
        return None;
    }
    Some(std::path::Path::new(dir).join(rel))
}

fn mime_of(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "json" | "czml" | "geojson" => "application/json",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "map" | "txt" => "text/plain; charset=utf-8",
        "xml" => "application/xml",
        "glb" => "model/gltf-binary",
        "gltf" => "model/gltf+json",
        "ktx2" => "image/ktx2",
        _ => "application/octet-stream",
    }
}

/// The fallback: any GET outside the API namespaces serves the console SPA from
/// ARC_CONSOLE_DIR — this Arc is the console's origin, per the mesh model. API paths
/// (`/v1/*`, `/api/*`, `/traffic/*`), non-GETs, and Arcs with no console dir keep the
/// JSON 404 so a client probing the contract never receives HTML dressed as an answer.
async fn console_spa(State(gw): State<Arc<Gateway>>, req: Request) -> Response {
    let path = req.uri().path().to_string();
    let api = path.starts_with("/v1") || path.starts_with("/api") || path.starts_with("/traffic");
    if req.method() != axum::http::Method::GET || api {
        return not_found();
    }
    // The fallback is not a route, so the console_gate route_layer can't cover it — the same
    // check runs here inline. This is the document load, so this 401 is the browser's prompt.
    if !console_cred_ok(&gw, req.headers()) {
        return basic_challenge();
    }
    let Some(dir) = gw.console_dir.clone() else { return not_found() };
    let Some(file) = console_asset(&dir, &path) else { return not_found() };
    // Small local reads; std::fs keeps the gateway's tokio feature set minimal (as /v1/arcs).
    let (file, bytes) = match std::fs::read(&file) {
        Ok(b) => (file, b),
        // A miss with no extension is an SPA client-side route — serve the shell. An asset
        // miss stays a 404 so a broken build fails loud, never blank.
        Err(_) if !path.rsplit('/').next().unwrap_or("").contains('.') => {
            let index = std::path::Path::new(&dir).join("index.html");
            match std::fs::read(&index) {
                Ok(b) => (index, b),
                Err(_) => return not_found(),
            }
        }
        Err(_) => return not_found(),
    };
    (StatusCode::OK, [(axum::http::header::CONTENT_TYPE, mime_of(&file))], bytes).into_response()
}

/// The membership gate (ICD §10). Only identity/discovery (`/v1/health`, `/v1/arc`) and the
/// membership entrance (`/v1/signup`, `/v1/bundle`) are open; EVERY other plane is gated behind
/// membership. Membership IS the {user, Arc} MLS tether — no key is issued — so the credential
/// (in `Authorization`) is an identity signature proving a live tether, which the membership
/// plane verifies (`/verify`): absent → 401, unverifiable → 401, plane unreachable → 503. The
/// gate fails CLOSED — an Arc with no membership plane serves only its identity.
///
/// The ONE exception is a path whose minimum is `Guest` (see `required_role`): Guest is the
/// floor of the ladder, so such a path grants nothing a caller could not already have, and
/// demanding a credential to reach it makes the tier unreachable rather than safe. Those pass
/// without one. Everything above Guest still needs the signature.
/// The member's standing, least → most privileged, so `>=` IS the gate check. Mirrors
/// pacific-core `GroupRole` and arc-node's wire `Role`; an unknown string parses to the most
/// restrictive value, so a vocabulary the gateway doesn't understand can never over-grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Role {
    Guest,
    Viewer,
    Member,
    Admin,
    Owner,
}

impl Role {
    fn parse(s: &str) -> Role {
        match s {
            "owner" => Role::Owner,
            "admin" => Role::Admin,
            "member" => Role::Member,
            "viewer" => Role::Viewer,
            _ => Role::Guest,
        }
    }
}

/// The MINIMUM role a path demands. The line that matters is cost: a route that spends the Arc's
/// own resources or credentials on the caller's behalf needs Member, while the relay is free to
/// carry and only needs Guest. Unknown gated paths default to Member — a new route is private
/// until it deliberately opts down, never accidentally wide open.
///
/// `Guest` here means ANONYMOUS, not "a member whose role we couldn't read" — it is the one
/// value the gate honours without a credential. Returning it is therefore a deliberate act of
/// opening a route to the world; nothing reaches it by default.
fn required_role(path: &str) -> Role {
    if path.starts_with("/v1/relay") {
        Role::Guest // blind messaging — costs us nothing to carry
    } else if path == "/v1/box" || path.starts_with("/v1/box/") {
        // DELIBERATELY enrolled at Guest (EVENT-TICKETING §5 v1.1): the box office's
        // callers — buyers, organizers, door staff, Stripe's webhook — are NOT members
        // of this Arc, so the Arc roster is the wrong authority. Authentication is
        // PLANE-LEVEL: owner/delegate Ed25519 signatures on the signed routes, rail
        // HMAC on the webhook; /checkout and /verify are open by design.
        Role::Guest
    } else if path == "/v1/wake" || path.starts_with("/v1/wake/") {
        // DELIBERATELY enrolled at Guest (RINGFENCE §6), on the relay's own reasoning: a
        // phone registering for pushes carries its APNs token and a set of OPAQUE relay
        // tags — nothing this Arc's roster is the authority on, and the same tags it may
        // already SUB to anonymously at /v1/relay. Demanding a member credential to be
        // woken for blind mailbox traffic would gate the wake more tightly than the
        // message it wakes you for. The waker cannot decrypt, cannot name a sender, and
        // learns no group id; what it holds (token↔tag) is what the caller just told it.
        Role::Guest
    } else if path.starts_with("/v1/arcs") {
        Role::Viewer // read the mesh directory
    } else if path.starts_with("/v1/console") {
        Role::Admin // assign roles / operate the Arc
    } else {
        Role::Member // systems, land, and anything new
    }
}

/// A credential's SAFE digest for logs. The credential is `<identity_key>:<ts_ms>:<sig_hex>`, and
/// the signature is a live bearer credential for its ±5min window — so it is NEVER logged. We
/// keep the identity_key (a public identity, the thing you correlate against the Arc's roster) and
/// the timestamp (to see freshness / clock skew at a glance), and drop the signature entirely.
/// A value that is not in that shape (no colons) is reported ONLY by shape — never its bytes.
fn cred_digest(cred: &str) -> String {
    let mut it = cred.trim().splitn(3, ':');
    match (it.next(), it.next(), it.next()) {
        (Some(space), Some(ts), Some(_sig)) => {
            let short = if space.len() > 20 { &space[..20] } else { space };
            format!("member space={short}… ts={ts} sig=<redacted>")
        }
        _ => "malformed-cred <redacted>".to_string(),
    }
}

// -- request trace ------------------------------------------------------------

/// The correlation headers a Pacific client may stamp on a call. They are the
/// difference between "a call was allowed" and "THIS step of THIS client flow took
/// 4.2s and came back 502" — the question the gate log alone could never answer.
/// `x-pacific-step` is a free-form client flow label; nothing else is read.
const H_REQ_ID: &str = "x-pacific-request-id";
const H_STEP: &str = "x-pacific-step";

/// One structured line per request, emitted AFTER the handler returns, carrying the
/// client's correlation ids, the outcome, and the wall time. Applied to the whole
/// router (public + gated) so a refused call is as visible as a served one.
///
/// TRACE, not gate: the gate decides, this reports. It never inspects a body, so
/// proxied responses stream through untouched — reading one here would mean
/// buffering the very thing the proxy exists not to buffer.
async fn trace(req: Request, next: Next) -> Response {
    let started = std::time::Instant::now();
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let h = req.headers();
    let get = |k: &str| h.get(k).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    // A client that stamps no id still gets a correlatable line: the id is minted here
    // and returned on the response, so the caller can log the same value.
    let req_id = {
        let given = get(H_REQ_ID);
        if given.is_empty() { format!("gw-{}", now_ms()) } else { given }
    };
    let step = get(H_STEP);

    let mut resp = next.run(req).await;
    let ms = started.elapsed().as_millis();
    let status = resp.status().as_u16();
    // Fields are key=value so the line greps cleanly (`grep 'req=<request-id>'`).
    eprintln!(
        "arc-gateway: TRACE {method} {path} status={status} ms={ms} req={req_id}{}",
        if step.is_empty() { String::new() } else { format!(" step={step}") },
    );
    if let Ok(v) = axum::http::HeaderValue::from_str(&req_id) {
        resp.headers_mut().insert(H_REQ_ID, v);
    }
    resp
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

async fn membership_gate(State(gw): State<Arc<Gateway>>, req: Request, next: Next) -> Response {
    // Captured up front: `next.run(req)` consumes `req`, so the decision log can't read them after.
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let need = required_role(&path);

    // `Authorization` is the ONLY credential header (ICD §10). There is no key-header
    // fallback: membership IS the credential.
    let cred = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty());
    let Some(cred) = cred else {
        // A Guest-minimum path is open (see `required_role`) — the relay is the only one, and
        // Semaphore is blind, so a stranger's ciphertext costs no more to carry than a member's.
        // Without this the tier is dead code: the core dials the relay with a bare WebSocket
        // and no Authorization header, and folding the signup Welcome needs a relay sync — so
        // gating it made joining depend on the very leg that joining unlocks.
        if need == Role::Guest {
            eprintln!("arc-gateway: GATE {method} {path} -> ALLOW anonymous (need {need:?})");
            return next.run(req).await;
        }
        eprintln!("arc-gateway: GATE {method} {path} -> 401 no-credential (need {need:?})");
        return err(StatusCode::UNAUTHORIZED, "unauthorized", "membership required — join this Arc (POST /v1/signup) first");
    };
    let digest = cred_digest(&cred);
    let Some(base) = gw.membership.clone() else {
        eprintln!("arc-gateway: GATE {method} {path} [{digest}] -> 503 membership-plane-unprovisioned");
        return err(StatusCode::SERVICE_UNAVAILABLE, "plane_down", "membership plane not provisioned — cannot verify membership");
    };
    // Reachable-but-rejected → 401 (not a member); UNREACHABLE → 503 (can't verify, so the
    // gate fails closed — an Arc whose membership plane is down serves only its identity).
    match gw
        .http
        .get(format!("{base}/verify"))
        .header(axum::http::header::AUTHORIZATION, &cred)
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            // Membership is established; now check STANDING. The role rode the tether to the
            // member and back to us from the same folded source, so the Arc and the device
            // agree on it. A member whose role the plane omits is treated as Guest — the
            // least privilege, so a malformed answer can never widen access.
            let held = r
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| v.get("role").and_then(|r| r.as_str()).map(Role::parse))
                .unwrap_or(Role::Guest);
            if held >= need {
                eprintln!("arc-gateway: GATE {method} {path} [{digest}] -> ALLOW (need {need:?}, hold {held:?})");
                next.run(req).await
            } else {
                eprintln!("arc-gateway: GATE {method} {path} [{digest}] -> 403 insufficient-role (need {need:?}, hold {held:?})");
                err(
                    StatusCode::FORBIDDEN,
                    "insufficient_role",
                    &format!("this Arc requires the {need:?} role here; you hold {held:?}"),
                )
            }
        }
        // The membership plane REACHED us and said no. This is the line that separates "the
        // signature/roster check failed" (here) from "we never got to ask" (the Err arm) —
        // the distinction we could not make from the client side, where both look like 401.
        Ok(r) => {
            eprintln!("arc-gateway: GATE {method} {path} [{digest}] -> 401 verify-rejected (membership /verify returned {})", r.status());
            err(StatusCode::UNAUTHORIZED, "unauthorized", "not a member of this Arc")
        }
        Err(e) => {
            eprintln!("arc-gateway: GATE {method} {path} [{digest}] -> 503 verify-unreachable ({e})");
            err(StatusCode::SERVICE_UNAVAILABLE, "plane_down", "cannot reach the membership plane to verify membership")
        }
    }
}

// -- relay WebSocket proxy ----------------------------------------------------

/// GET /v1/relay — upgrade and reverse-proxy the WebSocket to the Semaphore relay,
/// pumping frames both ways. The relay stays blind; the gateway forwards, never inspects.
async fn relay(State(gw): State<Arc<Gateway>>, ws: WebSocketUpgrade) -> Response {
    let Some(upstream) = gw.relay.clone() else {
        return err(StatusCode::NOT_IMPLEMENTED, "not_configured", "the relay plane is not provisioned on this Arc");
    };
    ws.on_upgrade(move |socket| proxy_ws(socket, upstream))
}

async fn proxy_ws(client: WebSocket, upstream: String) {
    let up = match tokio_tungstenite::connect_async(&upstream).await {
        Ok((s, _)) => s,
        Err(_) => return, // upstream unreachable; the client socket closes
    };
    let (mut up_tx, mut up_rx) = up.split();
    let (mut cl_tx, mut cl_rx) = client.split();

    // client → upstream
    let c2u = async {
        while let Some(Ok(msg)) = cl_rx.next().await {
            let fwd = match msg {
                AxMsg::Text(t) => WsMsg::Text(t.into()),
                AxMsg::Binary(b) => WsMsg::Binary(b.into()),
                AxMsg::Ping(p) => WsMsg::Ping(p.into()),
                AxMsg::Pong(p) => WsMsg::Pong(p.into()),
                AxMsg::Close(_) => { let _ = up_tx.send(WsMsg::Close(None)).await; break; }
            };
            if up_tx.send(fwd).await.is_err() { break; }
        }
    };
    // upstream → client
    let u2c = async {
        while let Some(Ok(msg)) = up_rx.next().await {
            let fwd = match msg {
                WsMsg::Text(t) => AxMsg::Text(t.to_string()),
                WsMsg::Binary(b) => AxMsg::Binary(b.into()),
                WsMsg::Ping(p) => AxMsg::Ping(p.into()),
                WsMsg::Pong(p) => AxMsg::Pong(p.into()),
                WsMsg::Close(_) => { let _ = cl_tx.send(AxMsg::Close(None)).await; break; }
                WsMsg::Frame(_) => continue,
            };
            if cl_tx.send(fwd).await.is_err() { break; }
        }
    };
    tokio::select! { _ = c2u => {}, _ = u2c => {} }
}

#[tokio::main]
async fn main() {
    // PROVENANCE, first thing and before any side effect: log which commit this binary is, or
    // answer `--version` and exit. An unstamped build says so in words. See lib/arc-build.
    arc_build::stamp!();

    let gw = Arc::new(Gateway {
        admit_rate: Arc::default(),
        http: reqwest::Client::new(),
        faces: Arc::new(tokio::sync::Mutex::new(Faces::default())),
        relay: env_or("ARC_RELAY_UPSTREAM", "ws://127.0.0.1:8787"),
        systems: env_or("ARC_SYSTEMS_UPSTREAM", "http://127.0.0.1:8795"),
        membership: env_or("ARC_MEMBERSHIP_UPSTREAM", "http://127.0.0.1:8790"),
        // NO DEFAULT, unlike the planes above. The land plane needs gigabytes of ingested
        // HM Land Registry data, so it exists on the central Pacific Arc and nowhere else.
        // Defaulting to a localhost port would make every community Arc advertise a
        // capability it does not have — the one thing this gateway refuses to do.
        land: env_opt("ARC_LAND_UPSTREAM"),
        boxoffice: env_or("ARC_BOXOFFICE_UPSTREAM", "http://127.0.0.1:8793"),
        waker: env_or("ARC_WAKER_UPSTREAM", "http://127.0.0.1:8794"),
        arcs_file: env_opt("ARC_ARCS_FILE"),
        console_dir: env_or("ARC_CONSOLE_DIR", "/opt/arc/console"),
        // DEFAULTED, unlike arcs_file above, and for the opposite reason to `land`'s no-default:
        // a missing manifest advertises no capability — /v1/version still serves the gateway's
        // own compiled-in stamp and reports `deploy: null` with the path it looked in. Defaulting
        // means a deploy is self-describing without an edit to /etc/arc/arc.env on the box.
        build_manifest: env_or("ARC_BUILD_MANIFEST", "/opt/arc/build.json"),
        console_basic: match (env_opt("ARC_CONSOLE_USER"), env_opt("ARC_CONSOLE_PASS")) {
            (Some(u), Some(p)) => {
                use base64::{engine::general_purpose::STANDARD, Engine as _};
                Some(format!("Basic {}", STANDARD.encode(format!("{u}:{p}"))))
            }
            _ => {
                eprintln!("arc-gateway: ARC_CONSOLE_USER/ARC_CONSOLE_PASS unset — console surface answers 401");
                None
            }
        },
        app_origin: env_opt("ARC_APP_ORIGIN"),
    });

    // Public: identity/discovery + the membership ENTRANCE (how you join). Nothing else.
    // WIDENED 18 Sep 2026 by exactly one route — /v1/version, the build this process is. It is
    // discovery of the third kind (who · what can it do · WHAT IS RUNNING) and it is the answer
    // to a question the Arc previously could not answer at all; see `version` for the argument.
    let public = Router::new()
        .route("/v1/face/:slug", get(face_page))
        .route("/v1/face/:slug/items", get(face_items))
        .route("/v1/face/:slug/escape", get(face_escape))
        .route("/v1/face/:slug/door", get(face_door))
        .route("/v1/face/:slug/brand", get(face_brand))
        .route("/v1/face/:slug/m/:slot", get(face_media))
        .route("/v1/face/:slug/post/:id", get(face_post))
        .route("/v1/face/:slug/post/:id/document", get(face_post_document))
        .route("/v1/health", get(health))
        .route("/v1/arc", get(arc_identity))
        .route("/v1/version", get(version))
        .route("/v1/signup", post(signup))
        .route("/v1/admit", post(admit))
        .route("/v1/bundle", get(bundle));
    // The console surface, behind the Basic gate: the same-origin API the SPA fetches
    // (the same paths the Vite dev proxy forwards; the SPA itself is served by the
    // fallback, which runs the same check inline). A stopgap credential — the proper
    // front door stays the Admin-gated /v1/console/* below, and these routes retire
    // once the SPA can hold a member credential.
    let console_surface = Router::new()
        .route("/console/*rest", any(console))
        .route("/tether", post(tether))
        .route("/bundle", get(bundle))
        .route_layer(middleware::from_fn_with_state(gw.clone(), console_gate));
    // Gated: every other plane sits behind the membership gate (ICD §10).
    let gated = Router::new()
        .route("/v1/arcs", get(arcs))
        .route("/v1/systems/*rest", any(systems))
        .route("/v1/land/*rest", any(land))
        // Guest at the gate (see `required_role`) — auth is the box office's own.
        .route("/v1/box/*rest", any(boxoffice))
        // Guest at the gate (see `required_role`) — a push registration carries only the
        // caller's own APNs token and opaque relay tags.
        .route("/v1/wake/*rest", any(waker))
        .route("/v1/console/*rest", any(console))
        .route("/v1/relay", get(relay))
        .route_layer(middleware::from_fn_with_state(gw.clone(), membership_gate));
    // TRACE wraps everything — public and gated alike — so a 401 at the gate is as
    // traceable as a served request. Outermost on purpose: it times the real thing,
    // including the membership round-trip.
    let app = public
        .merge(console_surface)
        .merge(gated)
        .fallback(console_spa)
        .layer(middleware::from_fn(trace))
        .with_state(gw);

    let port = std::env::var("PORT").ok().and_then(|p| p.parse::<u16>().ok()).unwrap_or(8080);
    let bind = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&bind).await.unwrap_or_else(|e| {
        eprintln!("arc-gateway: could not bind {bind}: {e}");
        std::process::exit(1);
    });
    eprintln!("arc-gateway: /v1 router on http://{bind}");
    axum::serve(listener, app).await.unwrap_or_else(|e| eprintln!("arc-gateway: serve error: {e}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Guest` is now the gate's anonymous tier, so `required_role` decides what the world can
    /// reach WITHOUT a credential. That makes this the security boundary: a route that drifts to
    /// Guest is a route silently opened. Pin the whole answer, not just the anonymous pair.
    ///
    /// AMENDED for Slice 2 of event ticketing (EVENT-TICKETING §5 v1.1): /v1/box/* joins the
    /// relay at Guest — a DELIBERATE act, which is exactly the friction this pin exists to
    /// create. The box office's callers (buyers, organizers, door staff, Stripe's webhook)
    /// are not members of this Arc; its auth is plane-level (Ed25519 signatures, rail HMAC).
    ///
    /// AMENDED AGAIN for the waker (RINGFENCE §6): /v1/wake/* joins them, on the relay's own
    /// reasoning rather than the box office's. A phone registering to be woken hands over its
    /// APNs token and a set of OPAQUE relay tags — the same tags it may already subscribe to
    /// anonymously at /v1/relay — so gating the wake behind membership would be stricter than
    /// the message it wakes you for, while protecting nothing the caller did not just supply.
    ///
    /// CAVEAT: the bare /console/*, /tether and /bundle routes bypass this gate — they sit
    /// behind the console's own Basic gate instead (`console_gate`, ARC_CONSOLE_USER/PASS).
    /// This test still pins the membership gate itself — when the console learns to hold a
    /// member credential and rides /v1/console/*, the Basic tier retires and this is what remains.
    /// /v1/admit runs a signature check for anyone: a network gets ADMIT_PER_MINUTE a minute.
    #[test]
    fn admission_is_rate_bound_per_network() {
        let r = Rate::default();
        for _ in 0..ADMIT_PER_MINUTE {
            assert!(r.allow("198.51.100.7"));
        }
        assert!(!r.allow("198.51.100.7"), "over the bound");
        assert!(r.allow("203.0.113.9"), "another network has its own");
    }

    #[test]
    fn anonymous_paths_are_exactly_relay_box_and_wake() {
        assert_eq!(required_role("/v1/relay"), Role::Guest);
        for path in ["/v1/box/checkout", "/v1/box/stripe", "/v1/box/verify/pi_1", "/v1/box/door/l/t"] {
            assert_eq!(required_role(path), Role::Guest, "{path} is deliberately Guest");
        }
        for path in ["/v1/wake/register", "/v1/wake/forget", "/v1/wake/health"] {
            assert_eq!(required_role(path), Role::Guest, "{path} is deliberately Guest");
        }
        for path in [
            "/v1/arcs",
            "/v1/systems/gmail/threads",
            "/v1/console/roles",
            "/v1/boxoffice", // NOT /v1/box — the prefix match must not bleed
            "/v1/waker",     // NOT /v1/wake — same
            "/v1/wakeup",
            "/v1/something-invented-tomorrow",
        ] {
            assert_ne!(required_role(path), Role::Guest, "{path} must not be anonymous");
        }
    }

    /// A new gated route must be private by default — never wide open because nobody listed it.
    #[test]
    fn unknown_routes_default_to_member() {
        assert_eq!(required_role("/v1/whatever"), Role::Member);
    }

    /// `held >= need` IS the check, so the ladder's order is load-bearing.
    #[test]
    fn roles_order_least_to_most_privileged() {
        assert!(Role::Guest < Role::Viewer);
        assert!(Role::Viewer < Role::Member);
        assert!(Role::Member < Role::Admin);
        assert!(Role::Admin < Role::Owner);
    }

    /// An unrecognised role string must land on the FLOOR, never over-grant. Note the floor is
    /// now also the anonymous tier — which is safe only because a parsed role is compared with
    /// `>=` against the path's minimum, so a Guest still cannot reach a Member route.
    #[test]
    fn unknown_role_parses_to_the_floor() {
        assert_eq!(Role::parse("wizard"), Role::Guest);
        assert_eq!(Role::parse(""), Role::Guest);
        assert_eq!(Role::parse("owner"), Role::Owner);
    }

    /// The SPA handler serves ONLY from inside the console dir: `/` is the shell, a clean
    /// nested path passes through, and any empty/dot/dotdot segment is refused outright.
    #[test]
    fn console_asset_stays_inside_the_dir() {
        let d = "/opt/arc/console";
        assert_eq!(console_asset(d, "/"), Some(std::path::PathBuf::from("/opt/arc/console/index.html")));
        assert_eq!(console_asset(d, "/assets/app.js"), Some(std::path::PathBuf::from("/opt/arc/console/assets/app.js")));
        for path in ["/../etc/passwd", "/a/../b", "//etc/passwd", "/a//b", "/./a", "/a\\..\\b"] {
            assert_eq!(console_asset(d, path), None, "{path} must be refused");
        }
    }

    /// The console gate fails CLOSED (no configured credential admits nobody) and compares
    /// the whole header verbatim — a well-formed Basic value with the wrong secret is refused.
    #[test]
    fn console_gate_fails_closed_and_verbatim() {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let gw = |basic: Option<String>| Gateway {
            admit_rate: Arc::default(),
            http: reqwest::Client::new(),
            faces: Arc::new(tokio::sync::Mutex::new(Faces::default())),
            relay: None,
            systems: None,
            membership: None,
            land: None,
            boxoffice: None,
            waker: None,
            arcs_file: None,
            console_dir: None,
            build_manifest: None,
            console_basic: basic,
            app_origin: None,
        };
        let expect = format!("Basic {}", STANDARD.encode("arc:right-password"));
        let with = |v: &str| {
            let mut h = HeaderMap::new();
            h.insert(axum::http::header::AUTHORIZATION, v.parse().unwrap());
            h
        };
        // Unset → closed, even for a caller presenting something.
        assert!(!console_cred_ok(&gw(None), &with(&expect)));
        // Set → only the exact credential passes.
        let g = gw(Some(expect.clone()));
        assert!(console_cred_ok(&g, &with(&expect)));
        assert!(!console_cred_ok(&g, &HeaderMap::new()));
        let wrong = format!("Basic {}", STANDARD.encode("arc:wrong-password"));
        assert!(!console_cred_ok(&g, &with(&wrong)));
        assert!(!console_cred_ok(&g, &with(expect.trim_start_matches("Basic "))));
    }

    /// ct_eq is equality, not "starts with" — and never panics on length mismatch.
    #[test]
    fn ct_eq_is_exact() {
        assert!(ct_eq("abc", "abc"));
        assert!(!ct_eq("abc", "abd"));
        assert!(!ct_eq("abc", "ab"));
        assert!(!ct_eq("", "a"));
        assert!(ct_eq("", ""));
    }

    /// The credential digest must never leak the signature — it is a live bearer token.
    #[test]
    fn cred_digest_redacts_the_signature() {
        let d = cred_digest("space1234567890abcdefghij:1750000000000:deadbeefcafe");
        assert!(d.contains("ts=1750000000000"));
        assert!(!d.contains("deadbeefcafe"));
        assert_eq!(cred_digest("not-a-membership-credential"), "malformed-cred <redacted>");
    }

    // -- /v1/face/:slug/items -------------------------------------------------

    /// A gateway whose arc-node answer is `doc`, fresh, so no plane is asked.
    async fn serving(doc: serde_json::Value) -> Arc<Gateway> {
        let gw = bare_gateway(None);
        *gw.faces.lock().await = Faces { at: Some(std::time::Instant::now()), doc };
        gw
    }

    /// arc-node's `/faces` for one Site at `egregore`, carrying `items`.
    fn egregore(items: serde_json::Value) -> serde_json::Value {
        let bundle = serde_json::json!({
            "v": 1,
            "profile": { "displayName": "Egregore", "shape": "community" },
            "face": { "v": 1, "widgets": { "root": { "type": "root", "version": 1, "children": [
                {"version":1,"type":"wf-feed","source":"events","mode":"list","heading":"","fields":["venue"],"empty":"hide","emptyNote":"","size":"m"},
                {"version":1,"type":"wf-feed","source":"publications","mode":"list","heading":"","fields":["link"],"empty":"hide","emptyNote":"","size":"m"}
            ] } } }
        })
        .to_string();
        serde_json::json!({ "sites": [{
            "slug": "egregore", "host": "ab".repeat(32), "name": "Egregore", "bundle": bundle, "media": [], "items": items,
        }] })
    }

    async fn body_of(r: Response) -> Vec<u8> {
        axum::body::to_bytes(r.into_body(), 1 << 20).await.expect("body").to_vec()
    }

    /// GET /v1/face/:slug/items: the Site's live items, what `face_page` draws from, by key,
    /// each its source op's args by ICD name plus `at` (core face_items), parsed. Only the kinds
    /// the Face draws; a payload that is not a JSON object is left out. Public data, so any
    /// origin may read it. What reaches arc-node's answer at all (declared at both ends, not
    /// withdrawn) is core's, pinned by o48_a_sites_items_reach_its_host.
    #[tokio::test]
    async fn a_sites_items_are_served_as_json_by_key() {
        use axum::http::header::*;
        let (ev, pdf) = (format!("event:{}", "e1".repeat(32)), format!("post:{}", "f1".repeat(32)));
        let event = serde_json::json!({ "title": "Echoes night", "startMs": 1_790_000_000_000i64, "venue": "The Crypt", "at": 1_789_000_000_000i64 });
        let post = serde_json::json!({ "title": "Zine #1", "form": "pdf", "link": "https://egregores-echoes.com/zine-1.pdf", "at": 1_789_000_000_001i64 });
        let doc = egregore(serde_json::json!([
            { "key": ev, "payload": event.to_string() },
            { "key": pdf, "payload": post.to_string() },
            { "key": format!("post:{}", "0b".repeat(32)), "payload": "{not json" },
            { "key": "face", "payload": "{}" },
        ]));
        assert!(parse_site(&doc["sites"][0]).is_some(), "the Site's bundle parses, as the page's must");
        let r = face_items(State(serving(doc).await), Path("egregore".into())).await;
        assert_eq!(r.status(), StatusCode::OK);
        let h = r.headers().clone();
        assert_eq!(h[CONTENT_TYPE], "application/json");
        assert_eq!(h[CACHE_CONTROL], "public, max-age=60", "as the page's");
        assert_eq!(h[X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(h[ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        let body: serde_json::Value = serde_json::from_slice(&body_of(r).await).expect("JSON");
        assert_eq!(body, serde_json::json!({ "items": { ev: event, pdf: post } }));
    }

    // -- /v1/face/:slug/post/:id (W-98 Resources) ------------------------------

    const PID: &str = "d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1";

    /// arc-node's `/faces` for Egregore carrying one network post's row, body and document,
    /// as core face_items writes them to the Host.
    fn with_post(document: Option<serde_json::Value>) -> serde_json::Value {
        let mut items = vec![
            serde_json::json!({ "key": format!("post:{PID}"), "payload": serde_json::json!({ "title": "Zine #2", "body": "The **second**…", "bodyFormat": "markdown", "form": "pdf", "excerpt": "Out now.", "at": 1_789_000_000_001i64 }).to_string() }),
            serde_json::json!({ "key": format!("post:{PID}:body:0"), "payload": serde_json::json!({ "text": "The **second** issue." }).to_string() }),
        ];
        if let Some(d) = document {
            items.push(serde_json::json!({ "key": format!("post:{PID}:document"), "payload": d.to_string() }));
        }
        egregore(serde_json::Value::Array(items))
    }

    /// GET /v1/face/:slug/post/:id: the post's page, drawn by face-render from the Host's items
    /// and served as the Face is, headers and all; every miss the Face's one 404.
    #[tokio::test]
    async fn a_posts_page_is_served_as_the_face_is() {
        let doc = with_post(None);
        let r = face_post(State(serving(doc.clone()).await), Path(("egregore".into(), PID.into())), HeaderMap::new()).await;
        assert_eq!(r.status(), StatusCode::OK);
        let face = face_page(State(serving(doc.clone()).await), Path("egregore".into()), HeaderMap::new()).await;
        for (k, v) in face.headers() {
            assert_eq!(r.headers().get(k), Some(v), "{k} as the Face's");
        }
        let html = String::from_utf8(body_of(r).await).unwrap();
        assert!(html.contains("<h1 class=\"wf-post-title\">Zine #2</h1>") && html.contains("<strong>second</strong> issue."), "{html}");
        for (slug, id) in [("egregore", "e".repeat(64)), ("elsewhere", PID.to_string()), ("egregore", format!("{PID}:body:0"))] {
            let r = face_post(State(serving(doc.clone()).await), Path((slug.into(), id.clone())), HeaderMap::new()).await;
            assert_eq!(r.status(), StatusCode::NOT_FOUND, "{slug} {id}");
            assert_eq!(body_of(r).await, b"no such page\n");
        }
    }

    /// GET /v1/face/:slug/post/:id/document: the post's PDF, the bytes its Host item holds,
    /// served to be saved, never drawn on this origin.
    #[tokio::test]
    async fn a_posts_document_is_served_as_a_file_to_save() {
        use axum::http::header::*;
        let pdf = b"%PDF-1.4\n%%EOF\n";
        use base64::Engine as _;
        let data = base64::engine::general_purpose::STANDARD.encode(pdf);
        let d = serde_json::json!({ "document": data, "documentMime": "application/pdf", "documentVia": "inline", "documentKind": "document", "name": "zine \"2\".pdf" });
        let r = face_post_document(State(serving(with_post(Some(d))).await), Path(("egregore".into(), PID.into()))).await;
        assert_eq!(r.status(), StatusCode::OK);
        let h = r.headers().clone();
        assert_eq!(h[CONTENT_TYPE], "application/pdf");
        assert_eq!(h[CONTENT_DISPOSITION], "attachment; filename=\"zine _2_.pdf\"");
        assert_eq!(h[X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(h[CACHE_CONTROL], "public, max-age=60");
        assert_eq!(body_of(r).await, pdf.to_vec());
        let r = face_post_document(State(serving(with_post(None)).await), Path(("egregore".into(), PID.into()))).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
    }

    /// An unknown slug is the page's miss, word for word: one 404 for every miss.
    #[tokio::test]
    async fn an_unknown_slug_has_no_items() {
        let r = face_items(State(serving(egregore(serde_json::json!([]))).await), Path("elsewhere".into())).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        assert_eq!(body_of(r).await, b"no such page\n");
    }

    /// A miss is readable from any origin, as a hit is (R&D, 1 Oct): a browser app asking for the
    /// items of a community not yet published must see the 404 the docs promise, not "Failed to
    /// fetch". The page's miss is one 404 for every face route, so every miss carries it.
    #[tokio::test]
    async fn a_faces_miss_is_readable_from_any_origin_as_its_hit_is() {
        use axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN;
        let hit = face_items(State(serving(egregore(serde_json::json!([]))).await), Path("egregore".into())).await;
        let miss = face_items(State(serving(egregore(serde_json::json!([]))).await), Path("elsewhere".into())).await;
        assert_eq!(miss.status(), StatusCode::NOT_FOUND);
        assert_eq!(miss.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN), hit.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN), "the miss as the hit: {:?}", hit.headers());
        assert_eq!(miss.headers()[ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        assert_eq!(body_of(miss).await, b"no such page\n", "the same words");
    }

    // -- /v1/bundle's CORS, for the webapp (W-103) ------------------------------

    /// A membership plane whose /bundle answers one bundle.
    async fn bundling() -> String {
        let app = axum::Router::new().route("/bundle", axum::routing::get(|| async { "a-contact-bundle" }));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        url
    }

    use axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN;

    async fn bundle_from(app_origin: Option<&str>, origin: Option<&str>) -> Response {
        let mut gw = Arc::try_unwrap(bare_gateway(None)).ok().unwrap();
        gw.membership = Some(bundling().await);
        gw.app_origin = app_origin.map(String::from);
        let mut rq = Request::builder().method("GET").uri("/v1/bundle");
        if let Some(o) = origin {
            rq = rq.header(axum::http::header::ORIGIN, o);
        }
        bundle(State(Arc::new(gw)), rq.body(axum::body::Body::empty()).unwrap()).await
    }

    /// W-103: GET /v1/bundle answers CORS for the webapp's origin (ARC_APP_ORIGIN) alone: that
    /// exact origin, Vary: Origin, no credentials. Any other origin, no origin, and an Arc with
    /// none set, answer as before: no CORS at all.
    #[tokio::test]
    async fn the_bundle_answers_cors_for_the_webapps_origin_alone() {
        let app = "https://app.wallflowers.io";
        let r = bundle_from(Some(app), Some(app)).await;
        assert_eq!(r.status(), StatusCode::OK);
        let h = r.headers().clone();
        assert_eq!(h.get(ACCESS_CONTROL_ALLOW_ORIGIN).map(|v| v.to_str().unwrap()), Some(app), "the webapp's origin, exactly");
        assert!(h.get_all(axum::http::header::VARY).iter().any(|v| v.to_str().unwrap().split(',').any(|x| x.trim().eq_ignore_ascii_case("origin"))), "Vary: Origin");
        assert!(h.get(axum::http::header::ACCESS_CONTROL_ALLOW_CREDENTIALS).is_none(), "no credentials");
        assert_eq!(body_of(r).await, b"a-contact-bundle", "the bundle itself, unchanged");
        for (set, asked) in [(Some(app), Some("https://elsewhere.example")), (Some(app), Some("null")), (Some(app), None), (None, Some(app))] {
            let r = bundle_from(set, asked).await;
            assert_eq!(r.status(), StatusCode::OK);
            assert!(r.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).is_none(), "no CORS for Origin {asked:?} on an Arc set to {set:?}");
        }
    }

    // -- /v1/version --------------------------------------------------------

    /// A Gateway with nothing provisioned except the manifest path under test.
    fn bare_gateway(manifest: Option<String>) -> Arc<Gateway> {
        Arc::new(Gateway {
            admit_rate: Arc::default(),
            http: reqwest::Client::new(),
            faces: Arc::new(tokio::sync::Mutex::new(Faces::default())),
            relay: None,
            systems: None,
            membership: None,
            land: None,
            boxoffice: None,
            waker: None,
            arcs_file: None,
            console_dir: None,
            build_manifest: manifest,
            console_basic: None,
            app_origin: None,
        })
    }

    async fn version_body(manifest: Option<String>) -> serde_json::Value {
        let resp = version(State(bare_gateway(manifest))).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.expect("body");
        serde_json::from_slice(&bytes).expect("/v1/version must always be valid JSON")
    }

    /// The gateway's own stamp is compiled in, so it is served whatever the box looks like —
    /// /v1/version must never be blank, and must never claim a provenance it does not have.
    /// This test passes in BOTH build states on purpose: unstamped is the honest answer for a
    /// plain `cargo build`, and the point is that the route SAYS which state it is in.
    #[tokio::test]
    async fn version_always_answers_for_the_gateway_itself() {
        let body = version_body(None).await;
        let g = &body["gateway"];
        assert_eq!(g["plane"], "arc-gateway");
        assert!(g["stamped"].is_boolean(), "stamped must be stated, not omitted: {body}");
        if g["stamped"] == serde_json::json!(true) {
            assert_ne!(g["commit"], "unknown", "a stamped build must name its commit");
        } else {
            assert_eq!(g["commit"], "unknown", "an unstamped build must not invent one");
        }
    }

    /// No manifest is a REPORTABLE state, not an error and not silence: binaries placed by hand,
    /// or predating hosting/deploy-arc.sh, are exactly what this route exists to expose. A null
    /// `deploy` with no explanation would read as "nothing deployed", so a note is required.
    #[tokio::test]
    async fn a_missing_deploy_manifest_is_reported_not_hidden() {
        let unset = version_body(None).await;
        assert!(unset["deploy"].is_null());
        assert!(unset["deploy_note"].as_str().unwrap_or("").contains("ARC_BUILD_MANIFEST"));

        let absent = version_body(Some("/nonexistent/arc/build.json".into())).await;
        assert!(absent["deploy"].is_null());
        let note = absent["deploy_note"].as_str().unwrap_or_default();
        assert!(note.contains("/nonexistent/arc/build.json"), "the note must name the path it tried: {note}");
        assert!(note.contains("deploy-arc.sh"), "and say what that means: {note}");
    }

    /// A manifest that is present and well-formed is served verbatim; one that is present and
    /// CORRUPT must not be reported as absent — "no deploy" and "unreadable deploy" are
    /// different facts about the box and only one of them is a missing script run.
    #[tokio::test]
    async fn a_present_manifest_is_served_and_a_corrupt_one_is_named() {
        let dir = std::env::temp_dir().join(format!("arc-version-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tmpdir");

        let good = dir.join("good.json");
        std::fs::write(&good, r#"{"commit":"abc123","planes":{"semaphore":{"short":"abc123"}}}"#).unwrap();
        let body = version_body(Some(good.to_string_lossy().into())).await;
        assert_eq!(body["deploy"]["commit"], "abc123");
        assert_eq!(body["deploy"]["planes"]["semaphore"]["short"], "abc123");
        assert!(body.get("deploy_note").is_none(), "a good manifest needs no excuse: {body}");

        let bad = dir.join("bad.json");
        std::fs::write(&bad, "{ this is not json").unwrap();
        let body = version_body(Some(bad.to_string_lossy().into())).await;
        assert!(body["deploy"].is_null());
        assert!(body["deploy_note"].as_str().unwrap_or("").contains("not valid JSON"));

        std::fs::remove_dir_all(&dir).ok();
    }
}
