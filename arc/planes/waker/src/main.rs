//! waker — the Arc's `/v1/wake` plane: push notifications WITHOUT un-blinding the relay.
//!
//! THE TENSION (RINGFENCE.txt §6). A push needs something that knows WHICH DEVICE to
//! poke. The relay must never know that: it addresses by opaque rotating tags and holds
//! no identity — that is the whole point of the public tier. So the push sender cannot be
//! the relay. It is a THIRD TIER, the same separation the rail already makes:
//!
//!   relay   blind   tags + sealed blobs, knows nobody            (public, anyone runs one)
//!   rail    money   Stripe secrets, per-sale ledger              (Pacific-run)
//!   waker   wake    token↔tag registrations, APNs credential     (Pacific-run)
//!
//! The waker is JUST ANOTHER RELAY CLIENT (watch.rs): it SUBs to the tags it was asked to
//! watch, exactly like a phone does, and the relay cannot tell it apart from any other
//! subscriber. When a blob lands it sends a CONTENT-FREE push (apns.rs) to the tokens
//! registered for that tag. It never decrypts, never sees a group id, never names a
//! sender — and the push says nothing, because there is nothing it could truthfully say.
//! The woken app syncs, decrypts, and raises the real local notification itself.
//!
//! WHAT THIS COSTS, stated because it is real: a stable device token bound to a set of
//! tags links those tags to each other and to one install across rotations. The mailbox
//! alone never permitted that. Signal makes the identical trade. There is no push without
//! something knowing who to poke; the choice is only whether that is stated or discovered.
//!
//! Fail-loud posture (boxoffice's, exactly): with no APNs credential the plane STILL
//! accepts and stores registrations — so no device has to re-register when a key finally
//! arrives — `/v1/wake/health` reports `waker:false`, and every skipped wake is logged.
//! What it never does is invent a signer or pretend a push went out.
//!
//! Env:
//!   PORT            bind port (default 8794; arcup passes ARC_WAKER_PORT)
//!   ARC_STATE_DIR   sqlite lives at $ARC_STATE_DIR/waker.db (default .)
//!   ARC_RELAY_URL   the relay to watch (default wss://arc.wallflowers.io/v1/relay)
//!   ARC_WAKER_DEDUPE_MS / ARC_WAKER_POLL_MS   one-buzz window / re-SUB poll
//!   APNS_KEY_P8 · APNS_KEY_PATH · APNS_KEY_ID · APNS_TEAM_ID · APNS_TOPIC · APNS_SANDBOX
//!
//! Routes (proxied by arc-gateway at Guest — deliberately enrolled, like /v1/box: the
//! caller is a phone that may not hold a member credential for THIS Arc, and the payload
//! is its own APNs token plus opaque tags):
//!   POST /v1/wake/register   {token, tags[], platform} → {watched: n}
//!   POST /v1/wake/forget     {token}                   → {forgotten: n}
//!   GET  /v1/wake/health (+ /health)                   → {waker, watching, devices, …}

mod apns;
mod store;
mod watch;

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde_json::json;

use apns::Pusher;
use store::Store;
use watch::{WatchConfig, Watcher};

/// A single register call may not claim more than this many tags. A phone watches one tag
/// per group-epoch, so a few hundred is already generous; the cap exists so one caller
/// cannot make the plane SUB to an unbounded set on everyone else's behalf.
const MAX_TAGS_PER_REGISTER: usize = 512;

struct Waker {
    store: Arc<Store>,
    /// `None` = no APNs credential. Registration still works (see the module doc).
    pusher: Option<Arc<dyn Pusher>>,
    relay_url: String,
}

type App = Arc<Waker>;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The `{ "error": { "code", "message" } }` envelope the gateway uses (ICD §11), so a
/// caller sees one error shape on both sides of the proxy.
fn err(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    let body = json!({ "error": { "code": code, "message": message.into() } });
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

fn ok(body: serde_json::Value) -> Response {
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

fn storage_err(e: rusqlite::Error) -> Response {
    eprintln!("waker: storage error: {e}");
    err(
        StatusCode::INTERNAL_SERVER_ERROR,
        "storage",
        "storage error",
    )
}

/// A device token is hex (APNs) and a tag is hex(32 bytes) by relay convention — both are
/// OPAQUE to this plane, which only checks that they are plausible and bounded. The point
/// is not validation for its own sake: it is that an unbounded string in a primary key is
/// how a store becomes a dumping ground.
fn plausible(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && s.chars().all(|c| c.is_ascii_alphanumeric())
}

#[derive(serde::Deserialize)]
struct RegisterReq {
    token: String,
    tags: Vec<String>,
    #[serde(default = "default_platform")]
    platform: String,
}

fn default_platform() -> String {
    "ios".to_string()
}

/// POST /v1/wake/register — bind this device token to the tags it wants waking for.
///
/// IDEMPOTENT AND RE-SENT CONSTANTLY, by design: relay tags are per-(group, epoch), so
/// the app re-registers its CURRENT set on every sync tick. That is what keeps live
/// registrations fresh and lets rotated ones age out of their own accord (store.rs).
async fn register(State(app): State<App>, body: Bytes) -> Response {
    let req: RegisterReq = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return err(StatusCode::BAD_REQUEST, "bad_request", e.to_string()),
    };
    if !plausible(&req.token, 256) {
        return err(
            StatusCode::BAD_REQUEST,
            "bad_token",
            "token must be a non-empty alphanumeric device token",
        );
    }
    if req.tags.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            "no_tags",
            "register with at least one tag (to stop being woken, POST /v1/wake/forget)",
        );
    }
    if req.tags.len() > MAX_TAGS_PER_REGISTER {
        return err(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too_many_tags",
            format!("at most {MAX_TAGS_PER_REGISTER} tags per call"),
        );
    }
    if let Some(bad) = req.tags.iter().find(|t| !plausible(t, 128)) {
        return err(
            StatusCode::BAD_REQUEST,
            "bad_tag",
            format!("'{bad}' is not a plausible relay tag"),
        );
    }
    let platform = if req.platform.trim().is_empty() {
        default_platform()
    } else {
        req.platform.trim().to_lowercase()
    };
    let now = now_ms();
    for tag in &req.tags {
        if let Err(e) = app
            .store
            .upsert_registration(&req.token, tag, &platform, now)
        {
            return storage_err(e);
        }
    }
    match app.store.tags_for_token(&req.token) {
        // The honest number: how many tags this device is watched for NOW (not how many
        // this call carried), so a client can tell whether its view matches the plane's.
        Ok(n) => ok(json!({ "watched": n, "waker": app.pusher.is_some() })),
        Err(e) => storage_err(e),
    }
}

#[derive(serde::Deserialize)]
struct ForgetReq {
    token: String,
}

/// POST /v1/wake/forget — stop waking this device, everywhere. Sign-out, notifications
/// switched off, or an app that no longer wants to be reachable. Idempotent.
async fn forget(State(app): State<App>, body: Bytes) -> Response {
    let req: ForgetReq = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return err(StatusCode::BAD_REQUEST, "bad_request", e.to_string()),
    };
    if !plausible(&req.token, 256) {
        return err(StatusCode::BAD_REQUEST, "bad_token", "implausible token");
    }
    match app.store.forget_token(&req.token) {
        Ok(n) => ok(json!({ "forgotten": n })),
        Err(e) => storage_err(e),
    }
}

/// GET /v1/wake/health — `waker:false` is the honest answer when no APNs credential is
/// configured: registrations are being stored, and nothing is being woken.
async fn health(State(app): State<App>) -> Response {
    let (tags, devices) = app.store.counts().unwrap_or((-1, -1));
    ok(json!({
        "status": "ok",
        "waker": app.pusher.is_some(),
        "environment": app.pusher.as_ref().map(|p| p.environment()),
        "watching": tags,
        "devices": devices,
        "relay": app.relay_url,
    }))
}

fn router(app: App) -> Router {
    Router::new()
        // Bare /health too: arcup and the gateway's liveness probe expect it on planes.
        .route("/health", get(health))
        .route("/v1/wake/health", get(health))
        .route("/v1/wake/register", post(register))
        .route("/v1/wake/forget", post(forget))
        .with_state(app)
}

#[tokio::main]
async fn main() {
    // PROVENANCE, first thing and before any side effect: log which commit this binary is,
    // or answer `--version` and exit. An unstamped build says so in words. See lib/arc-build.
    arc_build::stamp!();

    let state_dir = std::env::var("ARC_STATE_DIR").unwrap_or_else(|_| ".".to_string());
    let db = format!("{state_dir}/waker.db");
    let store = Arc::new(Store::open(&db).unwrap_or_else(|e| {
        eprintln!("waker: cannot open {db}: {e}");
        std::process::exit(1);
    }));

    // Fail-LOUD, not fail-dead: `from_env` logs the whole story when the credential is
    // missing and returns None; the plane serves on and stores every registration.
    let pusher: Option<Arc<dyn Pusher>> = apns::Apns::from_env().map(|a| Arc::new(a) as _);

    let relay_url = std::env::var("ARC_RELAY_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| watch::DEFAULT_RELAY_URL.to_string());
    let num = |k: &str, d: u64| {
        std::env::var(k)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(d)
    };
    let cfg = WatchConfig {
        relay_url: relay_url.clone(),
        dedupe_ms: num("ARC_WAKER_DEDUPE_MS", watch::DEFAULT_DEDUPE_MS),
        poll_ms: num("ARC_WAKER_POLL_MS", watch::DEFAULT_POLL_MS),
        stale_after_ms: store::STALE_AFTER_MS,
    };
    // The relay client runs for the life of the process, reconnecting on its own.
    tokio::spawn(Arc::new(Watcher::new(store.clone(), pusher.clone(), cfg)).run());

    let app = router(Arc::new(Waker {
        store,
        pusher,
        relay_url: relay_url.clone(),
    }));
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(8794);
    let bind = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .unwrap_or_else(|e| {
            eprintln!("waker: could not bind {bind}: {e}");
            std::process::exit(1);
        });
    eprintln!("waker: /v1/wake plane on http://{bind} (db {db}, relay {relay_url})");
    axum::serve(listener, app)
        .await
        .unwrap_or_else(|e| eprintln!("waker: serve error: {e}"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use apns::PushOutcome;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    struct NoopPusher;

    #[async_trait::async_trait]
    impl Pusher for NoopPusher {
        async fn push(&self, _token: &str) -> PushOutcome {
            PushOutcome::Delivered
        }
        fn environment(&self) -> &'static str {
            "sandbox"
        }
    }

    fn app_with(pusher: Option<Arc<dyn Pusher>>) -> (Router, Arc<Store>) {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let router = router(Arc::new(Waker {
            store: store.clone(),
            pusher,
            relay_url: "ws://127.0.0.1:0".into(),
        }));
        (router, store)
    }

    /// Drive the router in-process — no sockets, no network.
    async fn call(
        app: &Router,
        method: &str,
        path: &str,
        body: serde_json::Value,
    ) -> (u16, serde_json::Value) {
        let bytes = if body.is_null() {
            Vec::new()
        } else {
            body.to_string().into_bytes()
        };
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(bytes))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status().as_u16();
        let b = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        let v = if b.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&b).unwrap_or(serde_json::Value::Null)
        };
        (status, v)
    }

    /// The whole surface: register → the tags are watched → forget → they are not.
    #[tokio::test]
    async fn register_then_forget() {
        let (app, store) = app_with(Some(Arc::new(NoopPusher)));
        let (st, v) = call(
            &app,
            "POST",
            "/v1/wake/register",
            json!({"token": "deadbeef", "tags": ["aa11", "bb22"], "platform": "ios"}),
        )
        .await;
        assert_eq!(st, 200);
        assert_eq!(v["watched"], 2);
        assert_eq!(store.tags_watched().unwrap(), vec!["aa11", "bb22"]);

        // Re-registering the SAME set is the app's every-sync-tick behaviour: idempotent.
        let (st, v) = call(
            &app,
            "POST",
            "/v1/wake/register",
            json!({"token": "deadbeef", "tags": ["aa11", "bb22"]}),
        )
        .await;
        assert_eq!(st, 200);
        assert_eq!(v["watched"], 2, "a repeat adds nothing");

        // A rotated epoch: the new tag joins; the old one ages out on its own (store.rs).
        let (_, v) = call(
            &app,
            "POST",
            "/v1/wake/register",
            json!({"token": "deadbeef", "tags": ["cc33"]}),
        )
        .await;
        assert_eq!(v["watched"], 3);

        let (st, v) = call(
            &app,
            "POST",
            "/v1/wake/forget",
            json!({"token": "deadbeef"}),
        )
        .await;
        assert_eq!(st, 200);
        assert_eq!(v["forgotten"], 3);
        assert!(store.tags_watched().unwrap().is_empty());
        // Forgetting twice is a 200, not an error.
        let (st, v) = call(&app, "POST", "/v1/wake/forget", json!({"token": "nope"})).await;
        assert_eq!(st, 200);
        assert_eq!(v["forgotten"], 0);
    }

    /// THE FAIL-LOUD CASE. No APNs credential: registration still works and is still
    /// STORED (so no device has to re-register when a key arrives), and health says so.
    #[tokio::test]
    async fn registration_works_without_apns_credentials() {
        let (app, store) = app_with(None);
        let (st, v) = call(
            &app,
            "POST",
            "/v1/wake/register",
            json!({"token": "deadbeef", "tags": ["aa11"]}),
        )
        .await;
        assert_eq!(
            st, 200,
            "registration must NEVER be blocked by a missing key"
        );
        assert_eq!(v["watched"], 1);
        assert_eq!(v["waker"], false, "and it says the push half is down");
        assert_eq!(store.tags_watched().unwrap(), vec!["aa11"]);

        let (st, v) = call(&app, "GET", "/v1/wake/health", serde_json::Value::Null).await;
        assert_eq!(st, 200);
        assert_eq!(v["waker"], false, "health is honest about the missing key");
        assert_eq!(v["environment"], serde_json::Value::Null);
        assert_eq!(v["watching"], 1);
        assert_eq!(v["devices"], 1);
    }

    /// With a credential, health reports the environment it will push to — the one fact
    /// that makes a BadDeviceToken hunt short.
    #[tokio::test]
    async fn health_reports_the_apns_environment() {
        let (app, _) = app_with(Some(Arc::new(NoopPusher)));
        let (st, v) = call(&app, "GET", "/health", serde_json::Value::Null).await;
        assert_eq!(st, 200, "the bare /health arcup probes must exist too");
        assert_eq!(v["waker"], true);
        assert_eq!(v["environment"], "sandbox");
        assert_eq!(v["watching"], 0);
    }

    /// The register surface refuses junk rather than storing it: a store keyed on
    /// unbounded client strings is a dumping ground.
    #[tokio::test]
    async fn register_input_guards() {
        let (app, store) = app_with(None);
        for (body, code) in [
            (json!({"token": "", "tags": ["aa"]}), "bad_token"),
            (
                json!({"token": "tok:with:colons", "tags": ["aa"]}),
                "bad_token",
            ),
            (json!({"token": "deadbeef", "tags": []}), "no_tags"),
            (
                json!({"token": "deadbeef", "tags": ["not a tag"]}),
                "bad_tag",
            ),
        ] {
            let (st, v) = call(&app, "POST", "/v1/wake/register", body).await;
            assert_eq!(st, 400, "{code} must be a 400");
            assert_eq!(v["error"]["code"], code);
        }
        let many: Vec<String> = (0..MAX_TAGS_PER_REGISTER + 1)
            .map(|i| format!("tag{i}"))
            .collect();
        let (st, v) = call(
            &app,
            "POST",
            "/v1/wake/register",
            json!({"token": "deadbeef", "tags": many}),
        )
        .await;
        assert_eq!(st, 413);
        assert_eq!(v["error"]["code"], "too_many_tags");

        let (st, _) = call(&app, "POST", "/v1/wake/register", json!({"nope": 1})).await;
        assert_eq!(st, 400, "a malformed body is a 400, not a panic");
        assert!(
            store.tags_watched().unwrap().is_empty(),
            "not one refused call wrote a row"
        );
    }

    /// `platform` defaults to ios and is normalised — the store keeps what the pusher
    /// will later filter on, so it must not depend on a client's capitalisation.
    #[tokio::test]
    async fn platform_defaults_and_normalises() {
        let (app, store) = app_with(None);
        call(
            &app,
            "POST",
            "/v1/wake/register",
            json!({"token": "aaa", "tags": ["t1"]}),
        )
        .await;
        call(
            &app,
            "POST",
            "/v1/wake/register",
            json!({"token": "bbb", "tags": ["t1"], "platform": "  IOS "}),
        )
        .await;
        let devices = store.tokens_for_tag("t1").unwrap();
        assert_eq!(devices.len(), 2);
        assert!(devices.iter().all(|d| d.platform == "ios"));
    }
}
