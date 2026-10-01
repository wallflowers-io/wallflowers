//! arc-node — the Arc's membership / signup server.
//!
//! Signing up to an Arc IS the Pacific signup, and it ESTABLISHES THE TETHER: the Arc scans
//! the user's contact bundle and, as owner, forms a 2-member MLS group {user, Arc} (pacific-
//! core `pair_scan`), sealing the Welcome to the user's intro mailbox on the relay. On the
//! user's next `sync` they fold the tether and become a member of the Arc. The set of tethers
//! an Arc owns is its membership.
//!
//! Because the Arc protocol is open source, anyone can run this and stand up an Arc. This
//! binary is baked into the machine0 image (see library/machine0/arc-node-bootstrap.sh).
//!
//! Endpoints:
//!   GET  /health  -> "ok"
//!   GET  /arc     -> { name, identity_key, fingerprint, relay_url }   (discover + verify the Arc)
//!   POST /signup  -> body: the user's contact bundle string; establishes the tether, returns
//!                    { status, tethered, arc:{name,identity_key}, relay_url }
//!
//! On startup it also ANNOUNCES its sovereign identity to the control plane (if ARC_ID +
//! CONTROL_PLANE_URL are set): a signed `POST /api/arcs/:id/announce` proving possession of
//! the Ed25519 key, which TOFU-binds identity_key → the fleet slug. This is best-effort — the
//! peer-facing identity at GET /arc works regardless of whether the control plane is reachable.
//!
//! Fail loud: a missing relay, a malformed bundle, or an identity error returns an HTTP error
//! with the real message — never a fabricated OK.

use pacific_core::{CoreError, Node};
use std::sync::{Arc, Mutex};

mod console;
mod systems;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Open the Arc's node, minting its sovereign identity on first boot. The identity lives in
/// `$PACIFIC_STATE_DIR` (pacific-core's resolver); presence of the Ed25519 key file decides.
fn open_or_init(name: &str) -> Result<Node, CoreError> {
    let key = pacific_core::paths::state_dir().join("id_ed25519");
    if key.exists() {
        Node::open()
    } else {
        Node::init_identity(name)
    }
}

/// Bridge pacific-core's async relay ops into this sync server, one runtime per call (the
/// signup server is low-volume + pacific-core is a per-call, single-connection engine).
fn block_on<F, T>(fut: F) -> Result<T, CoreError>
where
    F: std::future::Future<Output = Result<T, CoreError>>,
{
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CoreError::Transport(format!("runtime: {e}")))?
        .block_on(fut)
}

fn err_json(msg: &str) -> String {
    serde_json::json!({ "error": msg }).to_string()
}

mod faces;

fn respond(req: tiny_http::Request, code: u16, ctype: &str, body: String) {
    let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], ctype.as_bytes())
        .expect("static header");
    let resp = tiny_http::Response::from_string(body)
        .with_status_code(code)
        .with_header(header);
    let _ = req.respond(resp);
}

/// Author the signup default role ONTO the member's tether, so the role reaches the device by
/// the same path as membership. Called right after `pair_scan_kind`, while we hold the node lock.
fn grant_default_role(node: &Node, peer: &[u8; 32], relay_url: &str) -> Result<(), CoreError> {
    assign_role(node, peer, console::Role::DEFAULT, relay_url)
}

/// Assign `role` to `peer` by authoring `group.setMemberRole` on the member-tether we share with
/// them. Owner-only at the core (we own every membership tether we minted), and the reducer drops
/// a role aimed at a non-member — so this can raise or lower standing, never grant membership.
fn assign_role(
    node: &Node,
    peer: &[u8; 32],
    role: console::Role,
    relay_url: &str,
) -> Result<(), CoreError> {
    let tether = console::member_tether_of(node, peer)?.ok_or_else(|| {
        CoreError::Directory(format!("no member-tether with {}", hex::encode(peer)))
    })?;
    let identity_key = format!("{}{}", pacific_core::identity::IDENTITY_KEY_PREFIX, hex::encode(peer));
    let (op_id, args) = console::set_member_role_op(&identity_key, role);
    block_on(node.apply(&tether, op_id, args)).map(|_| ())
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Announce this Arc's sovereign identity to the control plane and TOFU-bind it to the fleet
/// slug. Signs `pacific-arc-announce:v1\n<arc_id>\n<identity_key>\n<ts>` with the identity key
/// (re-loaded from `id_ed25519`) so the server can verify possession against the identity_key.
/// Best-effort with a short retry: the control plane may not be up yet when the Arc boots.
fn announce_loop(control_plane: &str, arc_id: &str, name: &str) {
    let id = match pacific_core::identity::load() {
        Ok(i) => i,
        Err(e) => {
            eprintln!("[announce] cannot load identity to announce: {e}");
            return;
        }
    };
    let identity_key = id.identity_key();
    let fingerprint = id.fingerprint();
    let url = format!("{}/api/arcs/{}/announce", control_plane.trim_end_matches('/'), arc_id);

    for attempt in 1..=5u32 {
        let ts = now_millis();
        let msg = format!("pacific-arc-announce:v1\n{arc_id}\n{identity_key}\n{ts}");
        let sig = id.sign(msg.as_bytes());
        let body = serde_json::json!({
            "identity_key": identity_key,
            "name": name,
            "fingerprint": fingerprint,
            "ts": ts,
            "sig": hex::encode(sig),
        });
        let body_str = body.to_string();
        match ureq::post(url.as_str())
            .header("content-type", "application/json")
            .send(body_str.as_str())
        {
            Ok(resp) => {
                eprintln!("[announce] {arc_id} -> {} ({identity_key})", resp.status());
                return;
            }
            Err(e) => eprintln!("[announce] {arc_id} attempt {attempt}/5 failed: {e}"),
        }
        std::thread::sleep(std::time::Duration::from_secs(3));
    }
    eprintln!("[announce] {arc_id}: gave up after 5 attempts (peer identity via /arc unaffected)");
}

fn main() {
    // PROVENANCE, first thing and before any side effect: log which commit this binary is,
    // or answer `--version` and exit. An unstamped build says so in words. See lib/arc-build.
    arc_build::stamp!();

    // THE CORE'S LOG. pacific-core reports through `tracing` — every membership door,
    // commit, refusal, removal and sync — and a process with no subscriber drops all of
    // it. `RUST_LOG` narrows or widens it (e.g. `pacific::membership=debug`); the default
    // is `info`, to stderr, which journald keeps.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let name = env_or("ARC_NAME", "Arc");
    // THE KIOSK KEYS, a labelled stand-in until the founders register them on the Site
    // (group.setClaimIssuer, D-53): the file the Door reads too (hosting/claim-keys.json).
    let claim_keys = pacific_core::claim::load_keys(std::env::var("ARC_CLAIM_KEYS").ok().as_deref())
        .unwrap_or_else(|e| panic!("ARC_CLAIM_KEYS: {e}"));
    // O-75 cards: a test's PACIFIC_HISTORY_TOTAL_CAP may only lower the ICD's total; above it,
    // this node does not start.
    let cards = pacific_core::node::cards_max().unwrap_or_else(|e| panic!("{e}"));
    if cards != pacific_core::node::CARDS_MAX {
        eprintln!("arc-node: cards' total lowered to {cards} bytes by PACIFIC_HISTORY_TOTAL_CAP, a test's");
    }
    if !claim_keys.is_empty() {
        eprintln!("arc-node: {} kiosk key(s) from ARC_CLAIM_KEYS, a stand-in (D-53)", claim_keys.len());
    }
    let relay_url = env_or("ARC_RELAY_URL", "ws://127.0.0.1:8787");
    let bind = env_or("ARC_SIGNUP_BIND", "0.0.0.0:8790");

    // Mint / load the Arc's sovereign identity (once). Fail loud if it can't.
    let mut node = open_or_init(&name).unwrap_or_else(|e| {
        eprintln!("arc-node: could not open/init identity: {e}");
        std::process::exit(1);
    });

    // POINT THIS DEVICE'S MESSAGING AT ARC_RELAY_URL. The core takes its transport from
    // the device's own route file, not from a parameter — so until this line, a fresh
    // Arc announced ARC_RELAY_URL at GET /arc and then synced against the core's
    // built-in default, which is a different relay. Nothing said so: the Arc simply
    // never saw a Welcome. (19 Sep 2026)
    if !node.routes().urls().any(|u| u == relay_url) {
        let routes = node.routes().with_relay(&relay_url);
        if let Err(e) = node.set_routes(routes) {
            eprintln!("arc-node: could not point messaging at {relay_url}: {e}");
            std::process::exit(1);
        }
        eprintln!("arc-node: messaging now routed via {relay_url}");
    }
    let identity_key = node.identity_key();
    let fingerprint = node.sas();

    // Stamp this Arc's PUBLIC url as the governing Arc so the tethers we mint route the user
    // back to us. pacific-core reads this file in `my_default_arc()` and carries it in the
    // Welcome, so the joining device knows which relay to sync against. A route that is
    // not a relay URL is refused by name: every tether would carry it, and be refused.
    if let Ok(public) = std::env::var("ARC_PUBLIC_URL") {
        if let Err(e) = node.set_default_arc(&public) {
            eprintln!("arc-node: ARC_PUBLIC_URL: {e}");
            std::process::exit(1);
        }
    }
    drop(node); // per-call model: each request re-opens Node::open()

    // Announce our sovereign identity to the control plane (best-effort, backgrounded), so it
    // can TOFU-bind identity_key → the fleet slug. Skipped (with a log) when unconfigured — the
    // peer-facing identity at GET /arc does not depend on the control plane.
    match (std::env::var("ARC_ID"), std::env::var("CONTROL_PLANE_URL")) {
        (Ok(arc_id), Ok(cp)) if !arc_id.is_empty() && !cp.is_empty() => {
            let name_c = name.clone();
            std::thread::spawn(move || announce_loop(&cp, &arc_id, &name_c));
        }
        _ => eprintln!(
            "arc-node: ARC_ID/CONTROL_PLANE_URL unset — skipping control-plane announce (GET /arc still serves identity)"
        ),
    }

    let arc_info = serde_json::json!({
        "name": name,
        "identity_key": identity_key,
        "fingerprint": fingerprint,
        "relay_url": relay_url,
    })
    .to_string();

    // Serialize ALL Node access (the sync loop + the signup handler) — the directory DB is a
    // single-writer store, so concurrent pair_scan + sync_once writes must not collide.
    let node_lock = Arc::new(Mutex::new(()));

    // Background sync loop. Every user's Pacific node runs this; so must the Arc — it is a
    // full MLS member, not a write-only signup server. sync_once drains our intro mailbox (so
    // we RECEIVE Welcomes = get added to other GroupObjects) and folds incoming deltas on the
    // groups we belong to (so we receive messages / FL updates on our tethers). Without it the
    // Arc can create tethers but never sees anything come back.
    {
        let relay = relay_url.clone();
        let lock = Arc::clone(&node_lock);
        let interval_ms: u64 = std::env::var("ARC_SYNC_MS").ok().and_then(|s| s.parse().ok()).unwrap_or(2000);
        std::thread::spawn(move || loop {
            {
                let _g = lock.lock().unwrap();
                match Node::open().and_then(|n| block_on(n.sync_once())) {
                    Ok(lines) if !lines.is_empty() => eprintln!("[sync] {}", lines.join(" · ")),
                    Ok(_) => {}
                    Err(e) => eprintln!("[sync] {e}"),
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(interval_ms));
        });
    }

    let server = tiny_http::Server::http(&bind).unwrap_or_else(|e| {
        eprintln!("arc-node: could not bind {bind}: {e}");
        std::process::exit(1);
    });
    eprintln!("arc-node: {name}  ({identity_key})");
    eprintln!("arc-node: fingerprint {fingerprint}");
    eprintln!("arc-node: signup on http://{bind}   ·   relay {relay_url}");

    for mut req in server.incoming_requests() {
        let method = req.method().clone();
        let path = req.url().split('?').next().unwrap_or("/").to_string();

        if method == tiny_http::Method::Get && path == "/health" {
            respond(req, 200, "text/plain", "ok".into());
        } else if method == tiny_http::Method::Get && path == "/arc" {
            respond(req, 200, "application/json", arc_info.clone());
        } else if method == tiny_http::Method::Post && path == "/signup" {
            let mut body = String::new();
            if req.as_reader().read_to_string(&mut body).is_err() {
                respond(req, 400, "application/json", err_json("could not read request body"));
                continue;
            }
            let bundle = body.trim().to_string();
            if bundle.is_empty() {
                respond(req, 400, "application/json", err_json("empty contact bundle"));
                continue;
            }

            // THE TETHER: scan the user's bundle, own the 2-member group, seal the Welcome.
            //
            // A `member-tether`, NOT a `connection`: membership carries the member's ROLE, and a
            // role must ride a log folded as GroupType. Owning the group is what lets us author
            // the owner-only `group.setMemberRole` below (and later, from /console/role).
            //
            // The default role is authored as a delta ON the tether, so the member folds it and
            // learns their own standing — the role travels the same path as membership itself.
            // Best-effort: a failure here leaves them on the roster, where `verify_member` reads
            // them as the signup default anyway, so we never strand a signup over the overlay.
            let outcome = {
                let _g = node_lock.lock().unwrap(); // serialize with the background sync loop
                (|| -> Result<[u8; 32], CoreError> {
                    let node = Node::open()?;
                    let peer = block_on(node.pair_scan_kind(
                        &bundle,
                        console::MEMBER_TETHER_KIND,
                    ))?;
                    if let Err(e) = grant_default_role(&node, &peer, &relay_url) {
                        eprintln!("arc-node: signup role overlay failed (member still admitted): {e}");
                    }
                    Ok(peer)
                })()
            };

            match outcome {
                Ok(peer) => {
                    let ok = serde_json::json!({
                        "status": "welcome-sealed",
                        "tethered": hex::encode(peer),
                        "arc": { "name": name, "identity_key": identity_key },
                        "relay_url": relay_url,
                        "note": "sync your Pacific app to fold the tether and complete membership",
                    })
                    .to_string();
                    respond(req, 200, "application/json", ok);
                }
                Err(e) => respond(
                    req,
                    502,
                    "application/json",
                    err_json(&format!("signup failed: {e}")),
                ),
            }
        } else if method == tiny_http::Method::Post && path == "/admit" {
            // ADMISSION BY A KIOSK CLAIM (A-3, D-58): the claim checked against the Site's
            // kiosk keys (ARC_CLAIM_KEYS until the founders register one), spent once under
            // the node lock, and the bundles' one identity added to the Site and to every
            // room of it this node admits to, a fresh key package each. The token is the
            // bearer credential: it is never logged. A definite refusal is 409, so the Door
            // ends the attempt; anything else is 503, so it keeps the claim for a retry.
            let mut body = String::new();
            if req.as_reader().read_to_string(&mut body).is_err() {
                respond(req, 400, "application/json", err_json("could not read request body"));
                continue;
            }
            #[derive(serde::Deserialize)]
            struct AdmitIn {
                claim: String,
                #[serde(default)]
                bundles: Vec<String>,
                #[serde(default)]
                bundle: Option<String>,
            }
            let Ok(mut ask) = serde_json::from_str::<AdmitIn>(&body) else {
                respond(req, 400, "application/json", err_json("expected {claim, bundles}"));
                continue;
            };
            ask.bundles.extend(ask.bundle.take());
            let Some(site) = pacific_core::claim::site_of(&ask.claim) else {
                respond(req, 409, "application/json", err_json("not a claim"));
                continue;
            };
            let outcome = {
                let _g = node_lock.lock().unwrap();
                Node::open().and_then(|node| block_on(node.admit_by_claim(&site, &ask.claim, &ask.bundles, &claim_keys)))
            };
            match outcome {
                Ok(a) => {
                    if let Some(c) = &a.fallback {
                        eprintln!("arc-node: Site {site}: no room carries the choice {c}; admitted to the rooms without one");
                    }
                    // O-75: a history that could not go is named, one line each.
                    for h in a.history.iter().filter(|h| h.unsent.is_some()) {
                        eprintln!("arc-node: Site {site}: no history of {} sent: {}", h.object, h.unsent.as_deref().unwrap_or_default());
                    }
                    for h in a.history.iter().filter(|h| h.cards_unsent > 0) {
                        eprintln!("arc-node: Site {site}: {} card(s) of {} not sent, past the {} bytes an admission carries or alone over a blob", h.cards_unsent, h.object, pacific_core::node::CARDS_MAX);
                    }
                    respond(
                        req,
                        200,
                        "application/json",
                        serde_json::json!({
                            "admitted": hex::encode(a.member),
                            "c": a.choice,
                            "a": a.artifact,
                            "rooms": a.rooms,
                            "unjoined": a.unjoined.iter().map(|(room, why)| serde_json::json!({ "room": room, "why": why })).collect::<Vec<_>>(),
                            "fallback": a.fallback,
                        })
                        .to_string(),
                    )
                }
                Err(CoreError::Membership(why)) => {
                    eprintln!("arc-node: a claim refused: {why}");
                    respond(req, 409, "application/json", err_json(&why));
                }
                Err(e) => {
                    eprintln!("arc-node: an admission failed, the claim kept: {e}");
                    respond(req, 503, "application/json", err_json("the admission could not complete; try again"));
                }
            }
        } else if method == tiny_http::Method::Get && path == "/faces" {
            // EVERY FACE THIS ARC SERVES, folded from the Hosts it is on. The gateway
            // renders from this; the pictures come one at a time, below. Who may use an
            // address is the signup claim's to say (faces.rs): a Host asking for one it
            // is not bound to is listed as refused, with the reason, and the reason is
            // what its own log gets back (a Host report op, when one lands). The question
            // is asked AFTER the node lock is released — it is a network call, and a
            // slow authority must not hold up sync.
            let out = {
                let _g = node_lock.lock().unwrap();
                Node::open().and_then(|n| {
                    let me = n.id.identity_pk();
                    n.host_sites(&me).map(|scan| (scan, hex::encode(me)))
                })
            };
            match out {
                Ok((scan, me)) => {
                    let mut sites = Vec::new();
                    let mut refused: Vec<(String, String)> = scan.refused.clone();
                    for site in &scan.sites {
                        let verdict = faces::answer(&site.slug)
                            .and_then(|a| faces::decide(&site.slug, &site.object_id, &a));
                        match verdict {
                            Ok(()) => sites.push(serde_json::json!({
                                "slug": site.slug,
                                "host": site.object_id,
                                "name": site.name,
                                "bundle": site.bundle,
                                "media": site.media.iter().map(|(s, _)| s.clone()).collect::<Vec<_>>(),
                                "items": site.items.iter().map(|i| serde_json::json!({
                                    "key": i.key, "payload": i.payload
                                })).collect::<Vec<_>>(),
                            })),
                            Err(why) => refused.push((site.object_id.clone(), why)),
                        }
                    }
                    respond(
                        req,
                        200,
                        "application/json",
                        serde_json::json!({ "arc": me, "sites": sites, "refused": refused }).to_string(),
                    );
                }
                Err(e) => respond(req, 500, "application/json", err_json(&format!("faces: {e}"))),
            }
        } else if method == tiny_http::Method::Get && path.starts_with("/face/") && path.contains("/m/") {
            // ONE PICTURE, by Host id and slot. The same 404 for a Host this Arc does
            // not serve and a slot that is empty: a caller must not be able to tell
            // which objects exist by asking.
            let rest = &path["/face/".len()..];
            let (host, slot) = rest.split_once("/m/").unwrap_or(("", ""));
            let out = {
                let _g = node_lock.lock().unwrap();
                Node::open().and_then(|n| {
                    let me = n.id.identity_pk();
                    n.host_media(host, slot, &me)
                })
            };
            match out {
                Ok(Some((mime, bytes))) => {
                    let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], mime.as_bytes())
                        .unwrap_or_else(|_| {
                            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/octet-stream"[..]).unwrap()
                        });
                    let len = bytes.len();
                    let _ = req.respond(
                        tiny_http::Response::new(
                            tiny_http::StatusCode(200),
                            vec![header],
                            std::io::Cursor::new(bytes),
                            Some(len),
                            None,
                        ),
                    );
                }
                Ok(None) => respond(req, 404, "application/json", err_json("no such picture")),
                Err(e) => respond(req, 500, "application/json", err_json(&format!("picture: {e}"))),
            }
        } else if method == tiny_http::Method::Get && path == "/bundle" {
            // This Arc's contact bundle — a peer Arc GETs it, then pair-scans it to tether to us.
            let out = {
                let _g = node_lock.lock().unwrap();
                Node::open().and_then(|n| n.build_contact_bundle())
            };
            match out {
                Ok(bundle) => respond(req, 200, "text/plain", bundle),
                Err(e) => respond(req, 500, "application/json", err_json(&format!("bundle: {e}"))),
            }
        } else if method == tiny_http::Method::Get && path == "/console/state" {
            // The Arc-served console read model (console/v1), folded from our own directory.
            let out = {
                let _g = node_lock.lock().unwrap();
                Node::open().and_then(|n| console::project(&n))
            };
            match out {
                Ok(state) => match serde_json::to_string(&state) {
                    Ok(body) => respond(req, 200, "application/json", body),
                    Err(e) => respond(req, 500, "application/json", err_json(&format!("encode: {e}"))),
                },
                Err(e) => respond(req, 500, "application/json", err_json(&format!("console state: {e}"))),
            }
        } else if method == tiny_http::Method::Post && path == "/tether" {
            // Form an Arc↔Arc tether: fetch the peer's bundle and pair-scan it into a 2-member
            // `arc-tether` GroupObject, sealing the Welcome to the peer's intro mailbox.
            let mut body = String::new();
            if req.as_reader().read_to_string(&mut body).is_err() {
                respond(req, 400, "application/json", err_json("could not read request body"));
                continue;
            }
            let peer_url = match serde_json::from_str::<console::TetherRequest>(&body) {
                Ok(r) => r.peer_url.trim().trim_end_matches('/').to_string(),
                Err(e) => {
                    respond(req, 400, "application/json", err_json(&format!("bad body (want {{\"peer_url\":..}}): {e}")));
                    continue;
                }
            };
            let bundle = match ureq::get(format!("{peer_url}/bundle").as_str()).call() {
                Ok(mut resp) => resp.body_mut().read_to_string().unwrap_or_default(),
                Err(e) => {
                    respond(req, 502, "application/json", err_json(&format!("fetch {peer_url}/bundle: {e}")));
                    continue;
                }
            };
            if bundle.trim().is_empty() {
                respond(req, 502, "application/json", err_json("peer returned an empty bundle"));
                continue;
            }
            let outcome = {
                let _g = node_lock.lock().unwrap();
                (|| -> Result<[u8; 32], CoreError> {
                    let n = Node::open()?;
                    block_on(n.pair_scan_kind(bundle.trim(), console::ARC_TETHER_KIND))
                })()
            };
            match outcome {
                Ok(peer) => {
                    let ok = serde_json::json!({
                        "status": "tether-sealed",
                        "kind": console::ARC_TETHER_KIND,
                        "peer": hex::encode(peer),
                        "peer_url": peer_url,
                        "note": "the peer folds this on its next sync; GET /console/state to see it",
                    })
                    .to_string();
                    respond(req, 200, "application/json", ok);
                }
                Err(e) => respond(req, 502, "application/json", err_json(&format!("tether failed: {e}"))),
            }
        } else if method == tiny_http::Method::Get && path == "/verify" {
            // The membership gate the gateway calls before proxying any member-only plane.
            // Read the caller's credential from Authorization and check it proves a live
            // {user, Arc} tether on this Arc (an identity signature — the same key that
            // established the tether). No Node write, but reads the directory under the lock.
            let cred = req
                .headers()
                .iter()
                .find(|h| h.field.equiv("Authorization"))
                .map(|h| h.value.as_str().to_string())
                .unwrap_or_default();
            let role = {
                let _g = node_lock.lock().unwrap();
                // An ERROR here (Node::open failed, or a roster read inside verify_member threw)
                // is NOT the same as "not a member" — but both used to collapse to None → 401,
                // unlogged. That silent mapping is exactly what hid a valid-signature request
                // erroring on the roster read. Log the error before it becomes a 401, so an
                // operational fault (can't open the node, can't read a group) is distinguishable
                // from a genuine non-member.
                match Node::open().and_then(|n| console::verify_member(&n, &cred)) {
                    Ok(role) => role,
                    Err(e) => {
                        eprintln!("[verify] ERROR (not a membership decision — surfaced as 401): {e}");
                        None
                    }
                }
            };
            // The gate answers with the member's ROLE, so the gateway can gate by capability
            // (a member plane spends the Arc's own resources; blind messaging does not) instead
            // of merely by identity.
            match role {
                Some(r) => respond(
                    req,
                    200,
                    "application/json",
                    serde_json::json!({ "member": true, "role": r }).to_string(),
                ),
                None => respond(req, 401, "application/json", err_json("not a member of this Arc")),
            }
        } else if method == tiny_http::Method::Post && path == "/systems/gmail/search" {
            // The /systems plane: run a Nango connector for a device and return citable RAG
            // text for the CLA. The Nango secret is Arc-side (env); the device supplies only
            // its connection id. No Node/directory access, so no node_lock is taken.
            let mut body = String::new();
            if req.as_reader().read_to_string(&mut body).is_err() {
                respond(req, 400, "application/json", err_json("could not read request body"));
                continue;
            }
            match systems::gmail_search(&body) {
                Ok(json) => respond(req, 200, "application/json", json),
                Err((code, msg)) => respond(req, code, "application/json", err_json(&msg)),
            }
        } else if method == tiny_http::Method::Post && path == "/console/role" {
            // Assign a member's role. The gateway has already established that the CALLER is an
            // Admin+ member (it gates this path); here we only author the delta. The role rides
            // the tether to the member — there is no second store to keep in step.
            let mut body = String::new();
            if req.as_reader().read_to_string(&mut body).is_err() {
                respond(req, 400, "application/json", err_json("could not read request body"));
                continue;
            }
            let parsed = serde_json::from_str::<console::RoleRequest>(&body);
            let Ok(rr) = parsed else {
                respond(
                    req,
                    400,
                    "application/json",
                    err_json("bad body (want {member, role})"),
                );
                continue;
            };
            let member_pk = pacific_core::identity::parse_identity_key(rr.member.trim());
            let Ok(member_pk) = member_pk else {
                respond(req, 400, "application/json", err_json("member is not a space id"));
                continue;
            };
            let outcome = {
                let _g = node_lock.lock().unwrap();
                Node::open().and_then(|n| assign_role(&n, &member_pk, rr.role, &relay_url))
            };
            match outcome {
                Ok(()) => respond(
                    req,
                    200,
                    "application/json",
                    serde_json::json!({
                        "ok": true, "member": rr.member, "role": rr.role,
                        "note": "authored on the tether; the member folds it on next sync",
                    })
                    .to_string(),
                ),
                Err(e) => respond(req, 409, "application/json", err_json(&format!("role not assigned: {e}"))),
            }
        } else if method == tiny_http::Method::Post && path == "/console/command" {
            respond(
                req,
                501,
                "application/json",
                err_json("not yet implemented — Arc↔Arc command ops land in the next backend increment"),
            );
        } else {
            respond(
                req,
                404,
                "application/json",
                err_json("not found — try GET /arc, GET /bundle, GET /console/state, POST /signup, or POST /tether"),
            );
        }
    }
}
