//! TRAINING_WHEELS P0 (core/docs/launch/mdr/training-wheels.md; the doctrine § Exemption:
//! training_wheels, W-79). Every action is recorded at the supervisor after the session
//! process answers and before the client hears: in PostgreSQL, or in the spool when the
//! database cannot take the row. Shadow only: nothing here is read back as state, and nothing
//! here fails an action except a spool that cannot be written.
//!
//! WHAT A ROW HOLDS (§ Capture, 654f4e3; Software Security's B1 and B2): a route's body as
//! `keep` names it, never a credential; `session` a keyed hash of the session, never the
//! cookie or a token; `outcome` the status, the Delta or object id, and a refusal's class,
//! never an answer body (sign-up's words and the token exchange's access token ride those).

pub mod pg;
pub mod spool;
pub mod tap;

use std::sync::{Arc, OnceLock};

use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    http::{Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// One row, tagged with its table: "action" or "delta" (the spool's). No clicks (Ralph, 28 Sep:
/// "clicks are not needed, only door api calls").
pub use spool::Row;

/// Who an action was, attached to the answer by the handler that knows: the account's key,
/// and the session's own id, which is hashed before it reaches a row.
#[derive(Clone)]
pub struct Who {
    pub account: String,
    pub session: String,
}

/// What a row keeps beside its body, attached to the answer by the handler that verified it:
/// at /v2/join, an admitted kiosk claim's choice (`c`) and share (`a`), as the Door checked
/// them; never its nonce, signature, kid or the token (B2).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Kept(pub serde_json::Map<String, serde_json::Value>);

/// What a route's row keeps of its request body. A route not named records nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Keep {
    Whole,
    WithoutClaim,
    Nothing,
}

/// The table of § Capture (654f4e3): the writes keep their bodies, whose ICD arguments hold no
/// secret (Software Security); a join keeps all but the claim; the account routes, whose
/// bodies carry the wrap, assertions, proofs, codes and verifiers, keep none.
pub fn keep(route: &str) -> Option<Keep> {
    keep_table(route)
}

/// The Door's POST routes that record no row, each with why. Every other POST route is in
/// `keep`; a new route in neither fails `every_acting_route_is_kept_or_named`, so nothing
/// silently falls out of the backup (Ralph, 29 Sep: "treat it as a backup").
pub const NOT_KEPT: &[(&str, &str)] = &[
    ("/v2/signup", "a ceremony's start; its finish is kept"),
    ("/v2/signin", "a ceremony's start; its finish is kept"),
];

fn keep_table(route: &str) -> Option<Keep> {
    match route {
        "/v2/mint" | "/v2/apply" | "/v2/add" | "/v2/history" | "/v2/batch" | "/v2/site/address" => Some(Keep::Whole),
        "/v2/join" => Some(Keep::WithoutClaim),
        "/v2/signup/finish" | "/v2/signup/continue" | "/v2/signin/finish" | "/v2/signout" | "/v2/token" | "/signin/site" | "/v2/bundle" => Some(Keep::Nothing),
        _ => None,
    }
}

/// Where a row goes when PostgreSQL cannot take it: the spool (build-worker-a's spool.rs).
/// `pending` while anything spooled is not yet in the database, so rows arrive in order.
pub trait Outbox: Send + Sync {
    fn pending(&self) -> bool;
    fn append(&self, row: &Row) -> Result<(), String>;
}

/// Where rows are written: the database, and the outbox it falls back to.
#[derive(Clone)]
pub struct Tw {
    pub db: Option<Arc<pg::Pg>>,
    pub outbox: Arc<dyn Outbox>,
}

impl Tw {
    /// Into the database when nothing waits in the outbox and it takes the row; into the
    /// outbox otherwise. Only an outbox that cannot be written is an error.
    pub async fn record(&self, row: Row) -> Result<(), String> {
        let tw = self.clone();
        tokio::task::spawn_blocking(move || {
            if let Some(db) = tw.db.as_ref().filter(|_| !tw.outbox.pending()) {
                match db.put(std::slice::from_ref(&row)) {
                    Ok(()) => return Ok(()),
                    Err(e) => eprintln!("door: training_wheels spools: {e}"),
                }
            }
            tw.outbox.append(&row)
        })
        .await
        .map_err(|e| e.to_string())?
    }
}

/// training_wheels as this Door's env names it, or off, and why. Off never refuses a start:
/// shadow mode must not stand in the MLS path's way. Without DOOR_TW_SPOOL_PUB it is off,
/// so nothing is ever spooled unsealed.
pub fn from_env() -> Result<Tw, String> {
    let dsn = std::env::var("DOOR_TW_DSN").map_err(|_| "DOOR_TW_DSN unset".to_string())?;
    let spool = Arc::new(spool::Spool::from_env().map_err(|e| e.to_string())?);
    let db = Arc::new(pg::Pg::new(&dsn));
    // The drainer: whatever waits in the spool goes to the database, in order, when it can.
    let (s, d) = (spool.clone(), db.clone());
    std::thread::Builder::new()
        .name("tw-drain".into())
        .spawn(move || {
            let stop = std::sync::atomic::AtomicBool::new(false);
            loop {
                if s.pending() {
                    let _ = s.drain_until_empty(d.as_ref(), &stop);
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(Tw { db: Some(db), outbox: spool })
}

impl spool::Sink for pg::Pg {
    fn put(&self, rows: &[Row]) -> Result<(), String> {
        pg::Pg::put(self, rows)
    }
}

/// A session's Delta tap, read from its leash: each `tap {row}` line recorded as any row is.
pub async fn listen(tw: Tw, leash: tokio::io::BufReader<tokio::net::UnixStream>) {
    use tokio::io::AsyncBufReadExt;
    let mut lines = leash.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Some(row) = line.strip_prefix("tap ") else { continue };
        let Ok(row) = serde_json::from_str::<Row>(row) else {
            eprintln!("door: training_wheels: a tap line that is not a row");
            continue;
        };
        if row.get("table").and_then(|t| t.as_str()) != Some("delta") {
            continue;
        }
        if let Err(e) = tw.record(row).await {
            eprintln!("door: training_wheels LOST a Delta row: {e}");
        }
    }
}

/// A session's handle in a row: HMAC-SHA256 under a key drawn at this start, cut to 16 bytes.
/// The same session reads the same within one supervisor's life, which is a session's; the
/// id itself, the cookie's value, never leaves this process (B1).
pub fn session_handle(session: &str) -> String {
    static KEY: OnceLock<[u8; 32]> = OnceLock::new();
    let key = KEY.get_or_init(|| {
        let mut k = [0u8; 32];
        rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut k);
        k
    });
    let (prk, _) = hkdf::Hkdf::<sha2::Sha256>::extract(Some(key), session.as_bytes());
    hex::encode(&prk[..16])
}

/// Milliseconds since the epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// The `action` row for one request and its answer. The answer is read for its ids alone; what
/// the handler verified (`kept`) joins the body's args.
pub fn action_row(route: &str, who: Option<&Who>, body: &[u8], keep: Keep, status: StatusCode, answer: &[u8], kept: Option<&Kept>) -> Row {
    let sent: serde_json::Value = serde_json::from_slice(body).unwrap_or(serde_json::Value::Null);
    let args = match keep {
        Keep::Whole => sent.clone(),
        Keep::WithoutClaim => match sent.clone() {
            serde_json::Value::Object(mut m) => {
                m.remove("claim");
                serde_json::Value::Object(m)
            }
            other => other,
        },
        Keep::Nothing => serde_json::Value::Null,
    };
    let args = match (args, kept) {
        (args, None) => args,
        (serde_json::Value::Object(mut m), Some(k)) => {
            m.extend(k.0.clone());
            serde_json::Value::Object(m)
        }
        (serde_json::Value::Null, Some(k)) => serde_json::Value::Object(k.0.clone()),
        (other, Some(k)) => {
            let mut m = k.0.clone();
            m.insert("body".into(), other);
            serde_json::Value::Object(m)
        }
    };
    let said: serde_json::Value = if status.is_success() { serde_json::from_slice(answer).unwrap_or_default() } else { serde_json::Value::Null };
    let text = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
    // A batch's ids, refused or not: what landed before a refusal is state. Ids only, never
    // the refusal's words.
    let made: Option<Vec<String>> = serde_json::from_slice::<serde_json::Value>(answer)
        .ok()
        .and_then(|v| v.get("made").and_then(|m| m.as_array()).map(|m| m.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()));
    let object = text(&sent, "object").filter(|_| keep == Keep::Whole).or_else(|| text(&said, "object_id"));
    serde_json::json!({
        "table": "action",
        "id": uuid::Uuid::now_v7().to_string(),
        "at": now_ms(),
        "account": who.map(|w| w.account.clone()),
        "session": who.map(|w| session_handle(&w.session)),
        "route": route,
        "kind": text(&sent, "kind").filter(|_| keep == Keep::Whole),
        "op": text(&sent, "op").filter(|_| keep == Keep::Whole),
        "object": object,
        "args": args,
        "outcome": {
            "status": status.as_u16(),
            "delta": text(&said, "delta"),
            "object": text(&said, "object_id"),
            "made": made,
            "refusal": (!status.is_success()).then(|| status.canonical_reason().unwrap_or("refused")),
        },
        "backfilled": false,
    })
}

/// The capture: on a recorded route, the body is held, the handler answers, the row is
/// written, and only then does the answer leave. A row that can be neither written nor
/// spooled fails the action (the outbox pattern), after the write it records was made.
pub async fn capture(State(tw): State<Tw>, req: Request, next: Next) -> Response {
    let route = req.uri().path().to_string();
    let Some(keep) = keep(&route).filter(|_| req.method() == Method::POST) else {
        return next.run(req).await;
    };
    let (parts, body) = req.into_parts();
    let Ok(sent) = to_bytes(body, 1 << 20).await else {
        return (StatusCode::PAYLOAD_TOO_LARGE, "the body is over 1 MiB").into_response();
    };
    let answer = next.run(Request::from_parts(parts, Body::from(sent.clone()))).await;
    let (mut head, out) = answer.into_parts();
    let Ok(out) = to_bytes(out, usize::MAX).await else {
        return (StatusCode::BAD_GATEWAY, "the answer could not be read").into_response();
    };
    let who = head.extensions.remove::<Who>();
    let kept = head.extensions.remove::<Kept>();
    let row = action_row(&route, who.as_ref(), &sent, keep, head.status, &out, kept.as_ref());
    match tw.record(row).await {
        Ok(()) => Response::from_parts(head, Body::from(out)),
        // An account route's answer is never withheld (Software Security, C1): a sign-up's
        // carries the only copy of the new account's words. The row is lost, and said so.
        Err(e) if keep == Keep::Nothing => {
            eprintln!("door: training_wheels LOST the row of {route}: {e}");
            Response::from_parts(head, Body::from(out))
        }
        Err(e) => {
            eprintln!("door: training_wheels could not record {route}: {e}");
            (StatusCode::SERVICE_UNAVAILABLE, "the action was made and could not be recorded").into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_route_not_named_records_nothing() {
        for r in ["/v2/me", "/v2/graph", "/v2/signin", "/v2/signup", "/v2/work", "/v2/events", "/v2/new"] {
            assert_eq!(keep(r), None, "{r}");
        }
    }

    /// B1, B2: no row holds the session id, a token, the words, or the claim.
    #[test]
    fn a_row_holds_no_credential() {
        let who = Who { account: "ab".repeat(32), session: "c0ffee".repeat(10) };
        let words = "abandon ability able about above absent absorb abstract absurd abuse access accident";
        let answer = serde_json::json!({ "pk": "ab", "words": words, "access_token": "tok-SECRET", "saved": false }).to_string();
        for route in ["/v2/signup/finish", "/v2/signin/finish", "/v2/token", "/v2/signout", "/v2/signup/continue"] {
            let body = serde_json::json!({ "attempt": "att-SECRET", "sealed": { "c": "wrap-SECRET" }, "code": "code-SECRET", "code_verifier": "ver-SECRET" }).to_string();
            let row = action_row(route, Some(&who), body.as_bytes(), keep(route).unwrap(), StatusCode::OK, answer.as_bytes(), None).to_string();
            for secret in ["SECRET", words, &who.session] {
                assert!(!row.contains(secret), "{route} holds {secret}: {row}");
            }
        }
        let join = serde_json::json!({ "claim": "claim-SECRET", "site": "s" }).to_string();
        let row = action_row("/v2/join", Some(&who), join.as_bytes(), Keep::WithoutClaim, StatusCode::OK, b"{}", None).to_string();
        assert!(!row.contains("claim-SECRET") && row.contains("\"site\":\"s\""), "{row}");
    }

    /// An admitted join keeps the claim's choice and share, as the Door verified them, beside
    /// the body; nothing else of the claim, which the body brought, is kept.
    #[test]
    fn a_join_keeps_the_claims_choice_and_share_and_nothing_else_of_it() {
        let who = Who { account: "ab".repeat(32), session: "s1".into() };
        let token = "v1.cGF5bG9hZA.c2lnbmF0dXJl";
        let body = serde_json::json!({ "claim": token, "site": "s" }).to_string();
        let kept = Kept(serde_json::json!({ "c": "skills", "a": "k2m3n4p5q6r7s8t9uvwx" }).as_object().unwrap().clone());
        let row = action_row("/v2/join", Some(&who), body.as_bytes(), Keep::WithoutClaim, StatusCode::OK, b"{}", Some(&kept));
        assert_eq!(row["args"], serde_json::json!({ "site": "s", "c": "skills", "a": "k2m3n4p5q6r7s8t9uvwx" }));
        assert!(!row.to_string().contains(token), "{row}");
        // No body at all, as the webapp's join sends: the args are what was kept.
        let row = action_row("/v2/join", Some(&who), b"", Keep::WithoutClaim, StatusCode::OK, b"{}", Some(&kept));
        assert_eq!(row["args"], serde_json::json!({ "c": "skills", "a": "k2m3n4p5q6r7s8t9uvwx" }));
        // Nothing kept, nothing added.
        let row = action_row("/v2/join", Some(&who), b"", Keep::WithoutClaim, StatusCode::CONFLICT, b"", None);
        assert_eq!(row["args"], serde_json::Value::Null);
    }

    /// A write keeps its whole body and names what it made; a refusal keeps its class only.
    #[test]
    fn a_write_keeps_its_body_and_its_ids() {
        let who = Who { account: "ab".repeat(32), session: "s1".into() };
        let body = serde_json::json!({ "object": "o1", "op": "forum.post", "args": { "text": "hello" } }).to_string();
        let row = action_row("/v2/apply", Some(&who), body.as_bytes(), Keep::Whole, StatusCode::OK, br#"{"delta":"d1"}"#, None);
        assert_eq!((row["op"].as_str(), row["object"].as_str(), row["outcome"]["delta"].as_str()), (Some("forum.post"), Some("o1"), Some("d1")));
        assert_eq!(row["args"]["args"]["text"], "hello");
        let mint = serde_json::json!({ "kind": "group", "draft": { "name": "Mill Road" } }).to_string();
        let row = action_row("/v2/mint", Some(&who), mint.as_bytes(), Keep::Whole, StatusCode::OK, br#"{"object_id":"o2"}"#, None);
        assert_eq!((row["kind"].as_str(), row["object"].as_str()), (Some("group"), Some("o2")));
        let row = action_row("/v2/apply", Some(&who), body.as_bytes(), Keep::Whole, StatusCode::CONFLICT, b"PreconditionFailed: the author's words", None);
        assert_eq!(row["outcome"]["refusal"], "Conflict");
        assert!(!row.to_string().contains("the author's words"), "no answer body");
    }

    /// NC-100: a batch (the Register, a new section) keeps its steps whole, as mint and apply
    /// do, and what it made, by id; a refused batch keeps what it made before the refusal,
    /// and not the refusal's words.
    #[test]
    fn a_batch_keeps_its_steps_and_what_it_made() {
        let who = Who { account: "ab".repeat(32), session: "s1".into() };
        let body = serde_json::json!({ "steps": [
            { "do": "mint", "kind": "forum", "draft": { "name": "Resources" } },
            { "do": "apply", "object": "site1", "op": "base.setPart", "args": { "part": { "$step": 0 }, "role": "room", "at": 1 } },
        ] }).to_string();
        let keep = keep("/v2/batch").expect("a batch is recorded");
        assert_eq!(keep, Keep::Whole);
        let row = action_row("/v2/batch", Some(&who), body.as_bytes(), keep, StatusCode::OK, br#"{"made":["o1","d1"],"refused":null}"#, None);
        assert_eq!(row["args"]["steps"][1]["op"], "base.setPart");
        assert_eq!(row["outcome"]["made"], serde_json::json!(["o1", "d1"]));
        let row = action_row("/v2/batch", Some(&who), body.as_bytes(), keep, StatusCode::UNPROCESSABLE_ENTITY,
            br#"{"made":["o1"],"refused":{"step":1,"why":"the author's words"}}"#, None);
        assert_eq!(row["outcome"]["made"], serde_json::json!(["o1"]), "what landed before the refusal");
        assert!(!row.to_string().contains("the author's words"), "no refusal's words");
    }

    /// Every POST route the Door's router serves is kept, or named in NOT_KEPT with why. A
    /// route added to the router and to neither goes red here, in check-door.
    #[test]
    fn every_acting_route_is_kept_or_named() {
        let router = include_str!("../main.rs");
        let posts: Vec<&str> = router
            .lines()
            .filter_map(|l| l.trim().strip_prefix(".route(\""))
            .filter_map(|l| l.split_once("\", post(").map(|(r, _)| r))
            .filter(|r| r.starts_with("/v2/"))
            .collect();
        assert!(posts.contains(&"/v2/apply") && posts.contains(&"/v2/join"), "the router was read: {posts:?}");
        let loose: Vec<&&str> = posts.iter().filter(|r| keep(r).is_none() && !NOT_KEPT.iter().any(|(n, _)| n == *r)).collect();
        assert!(loose.is_empty(), "POST routes neither kept nor named in NOT_KEPT: {loose:?}");
        for (n, why) in NOT_KEPT {
            assert!(keep(n).is_none() && !why.is_empty() && posts.contains(n), "{n}: named, not kept, and served");
        }
    }

    /// An outbox that takes nothing: every row fails.
    struct Broken;
    impl Outbox for Broken {
        fn pending(&self) -> bool {
            true
        }
        fn append(&self, _: &Row) -> Result<(), String> {
            Err("the disk is full".into())
        }
    }

    /// C1: with nothing able to take the row, a sign-up's answer still reaches its person,
    /// words and status unchanged.
    #[tokio::test]
    async fn a_sign_up_answers_whether_or_not_its_row_is_kept() {
        use tower::ServiceExt;
        let words = "abandon ability able about above absent absorb abstract absurd abuse access accident";
        let app = axum::Router::new()
            .route(
                "/v2/signup/finish",
                axum::routing::post(move || async move {
                    (StatusCode::OK, axum::Json(serde_json::json!({ "pk": "ab", "words": words, "saved": false })))
                }),
            )
            .route("/v2/apply", axum::routing::post(|| async { axum::Json(serde_json::json!({ "delta": "d1" })) }))
            .layer(axum::middleware::from_fn_with_state(Tw { db: None, outbox: Arc::new(Broken) }, capture));
        let ask = |route: &str| Request::builder().method("POST").uri(route).body(Body::from("{}")).unwrap();
        let r = app.clone().oneshot(ask("/v2/signup/finish")).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let said = to_bytes(r.into_body(), usize::MAX).await.unwrap();
        assert!(String::from_utf8_lossy(&said).contains(words), "the words reach their person");
        // A write whose row is lost is, until ruled otherwise, the outbox pattern's 503.
        let r = app.oneshot(ask("/v2/apply")).await.unwrap();
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    /// One session reads the same handle each time; two sessions differ.
    #[test]
    fn the_session_handle_is_stable_and_keyed() {
        assert_eq!(session_handle("a"), session_handle("a"));
        assert_ne!(session_handle("a"), session_handle("b"));
        assert_eq!(session_handle("a").len(), 32);
    }
}
