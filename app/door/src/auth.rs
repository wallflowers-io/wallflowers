//! auth — the session process's calls to the auth service (ICD-3): the wrap and
//! the head are read unauthenticated, by design; both are written under the
//! identity's signature over a nonce the service issued. `pk` is 64 lowercase hex,
//! the service's one spelling of an address.

use pacific_core::resumption::HeadInput;

/// How long the auth-facing pool keeps an idle connection: below the shortest idle close in
/// front of the service (door-test's uvicorn, 5 s; production's edge, 400 s), so a pooled
/// connection is dropped before the service drops it (NC-130).
pub const IDLE: std::time::Duration = std::time::Duration::from_secs(4);

const CHALLENGE: &str = "X-Pacific-Challenge";
const SIGNATURE: &str = "X-Pacific-Signature";
const POSITION: &str = "X-Chain-Position";

/// The auth service's client: an idle connection kept at most IDLE. hyper drops an expired one
/// at checkout, by its timestamp and with no I/O, so this holds on a runtime polled only inside
/// `block_on`, which never sees the service close a connection (NC-130).
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .pool_idle_timeout(IDLE)
        .build()
        .expect("http")
}

const UNREACHED: &str = "the auth service could not be reached";

/// No status came back.
fn reach(e: reqwest::Error) -> String {
    format!("{UNREACHED}: {e}")
}

/// An answer's body: one that came back and will not parse is the service's answer, not a miss.
fn read(e: reqwest::Error) -> String {
    if e.is_decode() {
        format!("the auth service's answer would not parse: {e}")
    } else {
        reach(e)
    }
}

/// Once more when no status came back, never on an answer; each attempt signs its own
/// challenge. A repeat moves nothing twice: the head is a compare-and-set on position
/// (Stored, Behind or Fork) and the seal one on its base and counter (NC-130).
pub fn again<T>(mut f: impl FnMut() -> Result<T, String>) -> Result<T, String> {
    match f() {
        Err(e) if e.starts_with(UNREACHED) => f(),
        r => r,
    }
}

/// The wrap stored under `pk`.
pub async fn wrap(http: &reqwest::Client, auth: &str, pk: &str) -> Result<Vec<u8>, String> {
    let r = http.get(format!("{auth}/auth/users/{pk}")).send().await.map_err(reach)?;
    match r.status().as_u16() {
        200 => Ok(r.bytes().await.map_err(reach)?.to_vec()),
        404 => Err("no account is stored under that key".into()),
        s => Err(format!("the auth service answered {s} for the wrap")),
    }
}

/// The head the service holds, with the position it declares; `None` when it holds
/// none (an account made before heads, or one whose first head was never written).
pub async fn head(http: &reqwest::Client, auth: &str, pk: &str) -> Result<Option<HeadInput>, String> {
    let r = http.get(format!("{auth}/auth/users/{pk}/head")).send().await.map_err(reach)?;
    match r.status().as_u16() {
        404 => Ok(None),
        200 => {
            let declared_position = r
                .headers()
                .get(POSITION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok())
                .ok_or("the auth service served a head with no position")?;
            let blob = r.bytes().await.map_err(reach)?.to_vec();
            Ok(Some(HeadInput { blob, declared_position }))
        }
        s => Err(format!("the auth service answered {s} for the head")),
    }
}

/// A nonce to sign, from a service that calls itself `audience`. A service that
/// names another audience is refused: a signature for another host is one this one
/// could replay there.
pub async fn challenge(http: &reqwest::Client, auth: &str, audience: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct C {
        nonce: String,
        audience: String,
    }
    let r = http.get(format!("{auth}/auth/challenge")).send().await.map_err(reach)?;
    if !r.status().is_success() {
        return Err(format!("the auth service issued no challenge ({})", r.status()));
    }
    let c: C = r.json().await.map_err(read)?;
    if c.audience != audience {
        return Err(format!(
            "the auth service calls itself {:?} but it is {audience:?}; refusing to sign",
            c.audience
        ));
    }
    Ok(c.nonce)
}

/// Store the wrap: registration on the first, a re-enrolment after.
pub async fn put_wrap(
    http: &reqwest::Client,
    auth: &str,
    pk: &str,
    nonce: &str,
    sig: &str,
    wrap: Vec<u8>,
) -> Result<(), String> {
    let r = http
        .put(format!("{auth}/auth/users/{pk}"))
        .header("content-type", "application/octet-stream")
        .header(CHALLENGE, nonce)
        .header(SIGNATURE, sig)
        .body(wrap)
        .send()
        .await
        .map_err(reach)?;
    if r.status().is_success() {
        return Ok(());
    }
    let s = r.status();
    Err(format!("the auth service refused the wrap ({s}): {}", r.text().await.unwrap_or_default()))
}

/// What storing a head came to. `Behind` is not a failure of this session: the
/// service holds a later position than this Node has walked.
pub enum Stored {
    Stored,
    Behind { held: u64 },
    /// Another head is stored at this position. It may be this chain sealed by
    /// another session (the bytes differ every seal), or a real fork.
    Fork,
}

/// Move the anchor forward: compare-and-set on position at the service.
pub async fn put_head(
    http: &reqwest::Client,
    auth: &str,
    pk: &str,
    nonce: &str,
    sig: &str,
    position: u64,
    sealed: Vec<u8>,
) -> Result<Stored, String> {
    let r = http
        .put(format!("{auth}/auth/users/{pk}/head"))
        .header("content-type", "application/octet-stream")
        .header(CHALLENGE, nonce)
        .header(SIGNATURE, sig)
        .header(POSITION, position.to_string())
        .body(sealed)
        .send()
        .await
        .map_err(reach)?;
    let s = r.status();
    if s.is_success() {
        return Ok(Stored::Stored);
    }
    let body: serde_json::Value = r.json().await.unwrap_or_default();
    if s.as_u16() == 409 && body["reason"] == "behind" {
        return Ok(Stored::Behind { held: body["held_position"].as_u64().unwrap_or(0) });
    }
    if s.as_u16() == 409 && body["reason"] == "fork" {
        return Ok(Stored::Fork);
    }
    Err(format!("the auth service refused the head ({s}): {body}"))
}

/// What a seal's counter came to at the service (D-34 (c)): `Moved` means another writer
/// of this person on this node, and the process stops sealing and ends.
pub enum Counted {
    Stored,
    Moved { held: u64 },
}

/// The counter the service holds for this person's state on `node`; `None` when it
/// holds none. Signed, so an account's write count is not anyone's who knows its pk.
pub async fn seal_counter(http: &reqwest::Client, auth: &str, pk: &str, node: &str, nonce: &str, sig: &str) -> Result<Option<u64>, String> {
    let r = http
        .get(format!("{auth}/auth/users/{pk}/seals/{node}"))
        .header(CHALLENGE, nonce)
        .header(SIGNATURE, sig)
        .send()
        .await
        .map_err(reach)?;
    let s = r.status();
    let body: serde_json::Value = r.json().await.unwrap_or_default();
    match s.as_u16() {
        200 => body["counter"].as_u64().map(Some).ok_or_else(|| "the auth service served a seal with no counter".into()),
        404 if body["error"] == "no_seal" => Ok(None),
        _ => Err(format!("the auth service refused the seal's counter ({s}): {body}")),
    }
}

/// Move the counter from `held` to `counter`: compare-and-set, idempotent on a retry.
pub async fn put_seal(
    http: &reqwest::Client,
    auth: &str,
    pk: &str,
    node: &str,
    nonce: &str,
    sig: &str,
    held: u64,
    counter: u64,
) -> Result<Counted, String> {
    let r = http
        .put(format!("{auth}/auth/users/{pk}/seals/{node}"))
        .header(CHALLENGE, nonce)
        .header(SIGNATURE, sig)
        .header("X-Seal-Held", held.to_string())
        .header("X-Seal-Counter", counter.to_string())
        .send()
        .await
        .map_err(reach)?;
    let s = r.status();
    let body: serde_json::Value = r.json().await.unwrap_or_default();
    if s.is_success() {
        return Ok(Counted::Stored);
    }
    if s.as_u16() == 409 && body["reason"] == "moved" {
        return Ok(Counted::Moved { held: body["held_counter"].as_u64().unwrap_or(0) });
    }
    Err(format!("the auth service refused the seal ({s}): {body}"))
}

/// Claim `slug` for this account at the auth service: the name a Site is served at
/// (D-52). The same account claiming its own again is not a refusal.
pub async fn claim_site(http: &reqwest::Client, auth: &str, pk: &str, nonce: &str, sig: &str, slug: &str) -> Result<(), String> {
    let r = http
        .post(format!("{auth}/auth/users/{pk}/sites"))
        .header(CHALLENGE, nonce)
        .header(SIGNATURE, sig)
        .json(&serde_json::json!({ "slug": slug }))
        .send()
        .await
        .map_err(reach)?;
    let s = r.status();
    if s.is_success() {
        return Ok(());
    }
    Err(format!("the auth service refused the address ({s}): {}", r.text().await.unwrap_or_default()))
}

/// Bind a claimed `slug` to the Host that serves it: the Arc serves an address from
/// the Host it is bound to, and no other.
pub async fn bind_host(http: &reqwest::Client, auth: &str, pk: &str, nonce: &str, sig: &str, slug: &str, host: &str) -> Result<(), String> {
    let r = http
        .put(format!("{auth}/auth/users/{pk}/sites/{slug}/host"))
        .header(CHALLENGE, nonce)
        .header(SIGNATURE, sig)
        .json(&serde_json::json!({ "host": host }))
        .send()
        .await
        .map_err(reach)?;
    let s = r.status();
    if s.is_success() {
        return Ok(());
    }
    Err(format!("the auth service would not bind the address ({s}): {}", r.text().await.unwrap_or_default()))
}
