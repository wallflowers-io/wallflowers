//! The APNs adapter — thin, hand-rolled reqwest (the same call the boxoffice made about
//! Stripe in `stripe.rs`: one POST and four headers do not justify an APNs SDK).
//!
//! THE PUSH SAYS NOTHING. The waker cannot decrypt — it never holds a key, and the blobs
//! it wakes on are sealed — so the payload is a CONSTANT: a generic alert with no sender,
//! no preview, no group, no count, nothing derived from the blob. The app does the real
//! work after it is woken: it syncs, decrypts, and raises a LOCAL notification carrying
//! the real sender and preview (RINGFENCE §6). A payload that pretended to know more
//! would be a lie the waker is structurally incapable of telling.
//!
//! AUTH is Apple's provider token: an ES256 JWT signed with the APNs auth key (.p8,
//! PKCS#8 PEM), `kid` = key id, `iss` = team id, `iat` = now. Apple refuses a token older
//! than 60 minutes and rate-limits minting more often than once per 20 — so it is cached
//! and re-minted at [`TOKEN_TTL_S`], safely inside both fences.
//!
//! Env (all fail-SOFT: unset ⇒ no pusher, `/v1/wake/health` says `waker:false`, and
//! registration keeps working so the store is ready the moment a key arrives):
//!   APNS_KEY_P8    the .p8 PKCS#8 PEM CONTENTS (what systemd-creds injects on machine0)
//!   APNS_KEY_PATH  alternative for local dev: a path to that file, read at startup
//!   APNS_KEY_ID    the key id (the `kid` claim)
//!   APNS_TEAM_ID   the Apple developer team id (the `iss` claim)
//!   APNS_TOPIC     the app bundle id (default `network.pacific`)
//!   APNS_SANDBOX   "0" ⇒ production; ANYTHING ELSE (including unset) ⇒ sandbox
//!
//! THE ENVIRONMENT IS THE FLAG, NOTHING ELSE. An APNs auth key can be restricted to one
//! environment at creation, and a device token is only valid against the environment its
//! app was signed for (`aps-environment`) — but the .p8 carries NO marker saying which,
//! so nothing here tries to infer it. `APNS_SANDBOX` picks the host, full stop, and the
//! chosen host is logged loudly once at startup. Pacific is development-signed today, so
//! the default is SANDBOX: getting this wrong answers `BadDeviceToken` on every push and
//! looks exactly like a broken token, which is why [`diagnose`] exists.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::{
    engine::general_purpose::STANDARD as B64, engine::general_purpose::URL_SAFE_NO_PAD as B64URL,
    Engine as _,
};
use p256::ecdsa::{signature::Signer, Signature, SigningKey};

/// Apple's production and development push hosts. Which one a token is valid for is
/// decided by the provisioning profile the app was signed with, NOT by the token —
/// pushing a development token at production answers 400 BadDeviceToken.
const HOST_PROD: &str = "https://api.push.apple.com";
const HOST_SANDBOX: &str = "https://api.sandbox.push.apple.com";

/// Re-mint the provider token after 50 minutes: inside Apple's 60-minute maximum age and
/// well outside its 20-minute minimum minting interval.
pub const TOKEN_TTL_S: i64 = 50 * 60;

/// THE PAYLOAD. A literal, not a builder, because there is nothing to build: every push
/// this plane will ever send is byte-identical. `loc-key` lets the app localize the
/// generic string; `mutable-content`/`content-available` are what let the woken app
/// replace this banner with a real, decrypted local notification.
pub const PAYLOAD: &str = r#"{"aps":{"alert":{"loc-key":"NEW_MESSAGE"},"sound":"default","mutable-content":1,"content-available":1}}"#;

/// Collapse every outstanding wake for a device into ONE banner. The in-process dedupe
/// (watch.rs) can only quiet a burst while the device is reachable; a phone that was
/// offline for an hour would otherwise be handed every stored push at once. The id is a
/// CONSTANT — deliberately carrying no tag and no group, so it tells Apple nothing.
const COLLAPSE_ID: &str = "pacific-wake";

#[derive(Debug, thiserror::Error)]
pub enum ApnsError {
    #[error("APNs key is not a usable PKCS#8 P-256 private key: {0}")]
    BadKey(String),
}

/// What happened to one push. The two that MATTER to the rest of the plane are
/// `Unregistered` (the token is dead — delete it, Apple treats retries as abuse) and
/// `Throttled` (back off; the device is fine).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushOutcome {
    Delivered,
    /// HTTP 410: the token no longer belongs to an install. Caller MUST forget it.
    Unregistered,
    /// HTTP 429/503: Apple is shedding load. Back off, keep the registration.
    Throttled,
    /// Any other refusal, carrying Apple's own `reason` so a misconfiguration is legible
    /// (`BadDeviceToken`, `TopicDisallowed`, `ExpiredProviderToken`, …).
    Failed {
        status: u16,
        reason: String,
    },
    /// The request never got an answer (DNS, TLS, timeout).
    Transport(String),
}

/// The push SEAM — the same shape as boxoffice's `Rail`. The watch loop depends on this,
/// never on `Apns`, so its tests can drive a recording pusher and NEVER touch the network.
#[async_trait::async_trait]
pub trait Pusher: Send + Sync {
    /// Wake one device. Content-free by contract: the only argument is the token.
    async fn push(&self, token: &str) -> PushOutcome;
    /// For `/v1/wake/health` — which Apple environment this pusher talks to.
    fn environment(&self) -> &'static str;
}

pub struct Apns {
    http: reqwest::Client,
    key: SigningKey,
    key_id: String,
    team_id: String,
    topic: String,
    host: &'static str,
    /// The cached provider token and the `iat` it was minted with.
    jwt: std::sync::Mutex<Option<(String, i64)>>,
}

fn now_s() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Strip PEM armour (or accept bare base64 DER) and load the P-256 signing key.
///
/// Tolerates the two shapes a secret takes on its way through a deployment: real
/// newlines (systemd-creds, a mounted file) and LITERAL `\n` two-character escapes (what
/// a one-line env var in a dashboard usually becomes). A key that survives neither is a
/// loud error, never a silent dummy signer.
pub fn parse_p8(raw: &str) -> Result<SigningKey, ApnsError> {
    let normalized = raw.trim().replace("\\n", "\n");
    if normalized.contains("BEGIN") {
        use p256::pkcs8::DecodePrivateKey;
        return SigningKey::from_pkcs8_pem(&normalized)
            .map_err(|e| ApnsError::BadKey(e.to_string()));
    }
    // No armour: treat the whole thing as base64 DER (whitespace tolerated).
    let compact: String = normalized.chars().filter(|c| !c.is_whitespace()).collect();
    let der = B64
        .decode(compact.as_bytes())
        .map_err(|e| ApnsError::BadKey(format!("not PEM and not base64 DER: {e}")))?;
    use p256::pkcs8::DecodePrivateKey;
    SigningKey::from_pkcs8_der(&der).map_err(|e| ApnsError::BadKey(e.to_string()))
}

/// The JWS header — exactly Apple's documented pair, nothing else.
fn header_json(key_id: &str) -> String {
    serde_json::json!({ "alg": "ES256", "kid": key_id }).to_string()
}

/// The claim set — the only two claims Apple reads.
fn claims_json(team_id: &str, iat: i64) -> String {
    serde_json::json!({ "iss": team_id, "iat": iat }).to_string()
}

/// Mint a provider token: `b64url(header).b64url(claims).b64url(r‖s)`. ES256 signatures
/// are RAW r‖s (64 bytes) in JWS — NOT the DER encoding OpenSSL prints, which is the
/// classic reason a hand-rolled APNs client gets 403 InvalidProviderToken.
pub fn mint_jwt(key: &SigningKey, key_id: &str, team_id: &str, iat: i64) -> String {
    let signing_input = format!(
        "{}.{}",
        B64URL.encode(header_json(key_id)),
        B64URL.encode(claims_json(team_id, iat))
    );
    let sig: Signature = key.sign(signing_input.as_bytes());
    format!("{signing_input}.{}", B64URL.encode(sig.to_bytes()))
}

/// Is a token minted at `iat` too old to reuse? (Apple's hard fence is 60 minutes.)
fn jwt_expired(iat: i64, now: i64) -> bool {
    now - iat >= TOKEN_TTL_S
}

/// THE HOUR-SAVER. `BadDeviceToken` and `InvalidProviderToken` are the two APNs failures
/// that look like a broken token and are almost always an ENVIRONMENT MISMATCH — a
/// development-signed app's token pushed at production, or a key created for the other
/// environment. A bare status code sends you hunting the wrong bug, so every one of these
/// is logged with the host it was sent to and the thing to check. Pure, so it is tested.
fn diagnose(status: u16, reason: &str, host: &str) -> Option<String> {
    let env = if host == HOST_SANDBOX {
        "sandbox"
    } else {
        "production"
    };
    let other = if host == HOST_SANDBOX {
        "production"
    } else {
        "sandbox"
    };
    match (status, reason) {
        (400, "BadDeviceToken") | (400, "DeviceTokenNotForTopic") => Some(format!(
            "ENVIRONMENT MISMATCH is the likely cause: this push went to {env} ({host}). A device \
             token is only valid for the environment its app was signed for (aps-environment) — a \
             development build's token works ONLY against sandbox. Check APNS_SANDBOX (try the \
             {other} host) and that APNS_TOPIC is the exact bundle id, BEFORE suspecting the token."
        )),
        (403, "InvalidProviderToken") | (403, "ExpiredProviderToken") => Some(format!(
            "the AUTH KEY, not the device: an APNs key can be restricted to one environment at \
             creation and this request went to {env} ({host}). Check APNS_KEY_ID matches the .p8, \
             APNS_TEAM_ID is the team that owns it, the key is valid for {env}, and this host's \
             clock (Apple refuses a provider token older than 60 minutes)."
        )),
        (403, "TopicDisallowed") | (400, "TopicDisallowed") => Some(format!(
            "APNS_TOPIC is not a topic this key may push to — it must be the app's exact bundle id \
             ({env})."
        )),
        _ => None,
    }
}

/// Map an APNs answer onto the outcome the plane acts on. Pure, so the whole status
/// policy is tested without a socket.
fn outcome_for(status: u16, body: &str) -> PushOutcome {
    let reason = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["reason"].as_str().map(str::to_string))
        .unwrap_or_default();
    match status {
        200 => PushOutcome::Delivered,
        // The token is dead — the install is gone. Delete it; Apple counts continued
        // pushes to an unregistered token as abuse.
        410 => PushOutcome::Unregistered,
        // 400 BadDeviceToken is the SAME fact for our purposes when it names the token:
        // the token cannot ever work (usually a sandbox/production mismatch), so keeping
        // it would mean retrying forever. Every other 400 is a real misconfiguration.
        400 if reason == "BadDeviceToken" || reason == "DeviceTokenNotForTopic" => {
            PushOutcome::Unregistered
        }
        429 | 503 => PushOutcome::Throttled,
        _ => PushOutcome::Failed { status, reason },
    }
}

impl Apns {
    pub fn new(
        key: SigningKey,
        key_id: String,
        team_id: String,
        topic: String,
        sandbox: bool,
    ) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            key,
            key_id,
            team_id,
            topic,
            host: if sandbox { HOST_SANDBOX } else { HOST_PROD },
            jwt: std::sync::Mutex::new(None),
        }
    }

    /// Build from the environment, or `None` when the credential set is incomplete.
    ///
    /// FAIL-LOUD, NOT FAIL-DEAD (the boxoffice posture): a missing key logs an error and
    /// leaves the plane serving registrations — the store fills up, `/health` says
    /// `waker:false`, and pushes start the moment a key is supplied. What it never does
    /// is invent a signer: there is no dummy credential path.
    pub fn from_env() -> Option<Self> {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        // APNS_KEY_P8 (the deployed shape — systemd-creds injects the contents) wins;
        // APNS_KEY_PATH is the local-dev convenience of pointing at the .p8 on disk.
        let pem = match (env("APNS_KEY_P8"), env("APNS_KEY_PATH")) {
            (Some(p8), _) => Some(p8),
            (None, Some(path)) => match std::fs::read_to_string(&path) {
                Ok(s) => Some(s),
                Err(e) => {
                    eprintln!("waker: ERROR — APNS_KEY_PATH={path} unreadable: {e}");
                    None
                }
            },
            (None, None) => None,
        };
        let (Some(pem), Some(key_id), Some(team_id)) =
            (pem, env("APNS_KEY_ID"), env("APNS_TEAM_ID"))
        else {
            eprintln!(
                "waker: ERROR — APNS_KEY_P8 (or APNS_KEY_PATH) / APNS_KEY_ID / APNS_TEAM_ID unset: \
                 the push credential is DOWN. Registration still works and is still stored, so no \
                 device has to re-register when a key arrives — but NOTHING WILL BE WOKEN until \
                 then, and /v1/wake/health reports waker:false."
            );
            return None;
        };
        let key = match parse_p8(&pem) {
            Ok(k) => k,
            Err(e) => {
                // Never the key material, only the verdict.
                eprintln!("waker: ERROR — {e}; pushes are DISABLED (waker:false)");
                return None;
            }
        };
        let topic = env("APNS_TOPIC").unwrap_or_else(|| "network.pacific".to_string());
        // SANDBOX BY DEFAULT: Pacific is development-signed today, and a development
        // token pushed at production answers BadDeviceToken on every single send. Only an
        // explicit APNS_SANDBOX=0 chooses production — an unset variable must not silently
        // pick the environment that cannot work.
        let sandbox = std::env::var("APNS_SANDBOX").as_deref() != Ok("0");
        let apns = Self::new(key, key_id.clone(), team_id, topic.clone(), sandbox);
        // The ONE loud line that answers "which environment am I pushing to?".
        eprintln!(
            "waker: APNs -> {} ({}) topic={topic} key={key_id} — the device tokens registered here \
             MUST come from an app signed for this environment (aps-environment); set APNS_SANDBOX=0 \
             for a production-signed build",
            apns.host,
            apns.environment()
        );
        Some(apns)
    }

    /// The cached provider token, re-minted when it ages past [`TOKEN_TTL_S`].
    fn provider_token(&self) -> String {
        let now = now_s();
        let mut slot = self.jwt.lock().expect("jwt mutex");
        if let Some((tok, iat)) = slot.as_ref() {
            if !jwt_expired(*iat, now) {
                return tok.clone();
            }
        }
        let tok = mint_jwt(&self.key, &self.key_id, &self.team_id, now);
        *slot = Some((tok.clone(), now));
        tok
    }

    /// Drop the cached token so the next push mints a fresh one (403 ExpiredProviderToken
    /// — clock skew, or a token that outlived Apple's fence).
    fn invalidate_token(&self) {
        *self.jwt.lock().expect("jwt mutex") = None;
    }
}

#[async_trait::async_trait]
impl Pusher for Apns {
    async fn push(&self, token: &str) -> PushOutcome {
        let url = format!("{}/3/device/{token}", self.host);
        let resp = self
            .http
            .post(&url)
            .header("authorization", format!("bearer {}", self.provider_token()))
            .header("apns-topic", &self.topic)
            .header("apns-push-type", "alert")
            // 10 = deliver immediately. A wake that arrives after the user has put the
            // phone down is worth nothing; this is the whole point of the plane.
            .header("apns-priority", "10")
            .header("apns-collapse-id", COLLAPSE_ID)
            .body(PAYLOAD)
            .send()
            .await;
        let resp = match resp {
            Ok(r) => r,
            Err(e) => return PushOutcome::Transport(e.to_string()),
        };
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        if status == 403 {
            self.invalidate_token();
        }
        let outcome = outcome_for(status, &body);
        // The token is NEVER logged (it names one install); the status, Apple's reason and
        // the environment diagnosis are what actually make a failure fixable.
        if !matches!(outcome, PushOutcome::Delivered) {
            let reason = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v["reason"].as_str().map(str::to_string))
                .unwrap_or_default();
            match diagnose(status, &reason, self.host) {
                Some(hint) => eprintln!("waker: APNs {status} {reason} — {hint}"),
                None => eprintln!("waker: APNs {status} {reason} (host {})", self.host),
            }
        }
        outcome
    }

    fn environment(&self) -> &'static str {
        if self.host == HOST_SANDBOX {
            "sandbox"
        } else {
            "production"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::{signature::Verifier, VerifyingKey};

    /// A FIXED, non-secret test key — deterministic, so nothing here needs an RNG and the
    /// JWT under test is reproducible.
    fn test_key() -> SigningKey {
        let mut raw = [0u8; 32];
        for (i, b) in raw.iter_mut().enumerate() {
            *b = i as u8 + 1;
        }
        let fb: p256::FieldBytes = raw.into();
        SigningKey::from_bytes(&fb).expect("valid scalar")
    }

    fn b64url_decode(s: &str) -> Vec<u8> {
        B64URL.decode(s.as_bytes()).expect("b64url")
    }

    /// The provider token Apple will accept: three unpadded b64url parts, an ES256 header
    /// carrying the key id, `iss`/`iat` claims, and a RAW 64-byte r‖s signature over
    /// `header.claims` that verifies under the key's public half.
    #[test]
    fn jwt_shape_header_claims_and_signature() {
        let key = test_key();
        let jwt = mint_jwt(&key, "ABC1234567", "TEAM123456", 1_750_000_000);

        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3, "a JWS is exactly three parts");
        assert!(!jwt.contains('='), "b64url in a JWT is UNPADDED");
        assert!(
            !jwt.contains('+') && !jwt.contains('/'),
            "url-safe alphabet"
        );

        let header: serde_json::Value =
            serde_json::from_slice(&b64url_decode(parts[0])).expect("header json");
        assert_eq!(header["alg"], "ES256");
        assert_eq!(header["kid"], "ABC1234567");
        assert_eq!(
            header.as_object().unwrap().len(),
            2,
            "header carries alg + kid and nothing else"
        );

        let claims: serde_json::Value =
            serde_json::from_slice(&b64url_decode(parts[1])).expect("claims json");
        assert_eq!(claims["iss"], "TEAM123456");
        assert_eq!(claims["iat"], 1_750_000_000);
        assert_eq!(claims.as_object().unwrap().len(), 2);

        let sig_bytes = b64url_decode(parts[2]);
        assert_eq!(sig_bytes.len(), 64, "ES256 is raw r‖s, never DER");
        let sig = Signature::from_slice(&sig_bytes).expect("signature");
        let vk = VerifyingKey::from(&key);
        let signing_input = format!("{}.{}", parts[0], parts[1]);
        vk.verify(signing_input.as_bytes(), &sig)
            .expect("the signature must cover exactly header.claims");
    }

    /// The cache fence: reused inside Apple's 60-minute window, re-minted before it.
    #[test]
    fn provider_token_expiry_fence() {
        assert!(!jwt_expired(1_000, 1_000), "fresh");
        assert!(!jwt_expired(1_000, 1_000 + TOKEN_TTL_S - 1));
        assert!(jwt_expired(1_000, 1_000 + TOKEN_TTL_S));
        assert!(
            TOKEN_TTL_S < 60 * 60,
            "must re-mint INSIDE Apple's 60-minute maximum"
        );
        assert!(
            TOKEN_TTL_S > 20 * 60,
            "must not mint more often than Apple's 20-minute floor"
        );
    }

    /// GOLDEN: the payload is content-free. If this test has to change, someone tried to
    /// put something in a push the waker cannot possibly know.
    #[test]
    fn payload_is_content_free() {
        assert_eq!(
            PAYLOAD,
            r#"{"aps":{"alert":{"loc-key":"NEW_MESSAGE"},"sound":"default","mutable-content":1,"content-available":1}}"#
        );
        let v: serde_json::Value = serde_json::from_str(PAYLOAD).expect("valid JSON");
        let aps = v["aps"].as_object().expect("aps");
        assert_eq!(aps.len(), 4, "aps carries exactly the four wake keys");
        assert_eq!(aps["mutable-content"], 1);
        assert_eq!(aps["content-available"], 1, "wakes the app to sync");
        let alert = aps["alert"].as_object().expect("alert");
        assert_eq!(alert.len(), 1, "the alert is ONE localization key");
        assert_eq!(alert["loc-key"], "NEW_MESSAGE");
        // The negative that matters: nothing derived from a blob the waker cannot read.
        for forbidden in ["body", "title", "subtitle", "loc-args", "badge", "sender"] {
            assert!(
                !PAYLOAD.contains(forbidden),
                "'{forbidden}' has no business in a blind push"
            );
        }
    }

    /// 410 is the one status that MUTATES the store — it must never be mistaken for a
    /// transient failure, and a transient failure must never be mistaken for it.
    #[test]
    fn status_policy() {
        assert_eq!(outcome_for(200, ""), PushOutcome::Delivered);
        assert_eq!(
            outcome_for(410, r#"{"reason":"Unregistered"}"#),
            PushOutcome::Unregistered
        );
        assert_eq!(
            outcome_for(400, r#"{"reason":"BadDeviceToken"}"#),
            PushOutcome::Unregistered,
            "a token that can never work is as dead as an unregistered one"
        );
        assert_eq!(outcome_for(429, ""), PushOutcome::Throttled);
        assert_eq!(outcome_for(503, ""), PushOutcome::Throttled);
        assert_eq!(
            outcome_for(403, r#"{"reason":"ExpiredProviderToken"}"#),
            PushOutcome::Failed {
                status: 403,
                reason: "ExpiredProviderToken".into()
            }
        );
        assert_eq!(
            outcome_for(400, r#"{"reason":"TopicDisallowed"}"#),
            PushOutcome::Failed {
                status: 400,
                reason: "TopicDisallowed".into()
            },
            "a topic misconfiguration must stay LOUD, not delete registrations"
        );
    }

    /// The key arrives in three shapes across local dev, systemd-creds and dashboards.
    /// All three must load; a broken one must be an error, never a silent dummy signer.
    #[test]
    fn p8_parses_pem_escaped_pem_and_bare_der() {
        use p256::pkcs8::{EncodePrivateKey, LineEnding};
        let key = test_key();
        let pem = key.to_pkcs8_pem(LineEnding::LF).unwrap().to_string();
        let der = key.to_pkcs8_der().unwrap();

        for (what, raw) in [
            ("real newlines", pem.clone()),
            ("literal \\n escapes", pem.replace('\n', "\\n")),
            ("bare base64 DER", B64.encode(der.as_bytes())),
        ] {
            let parsed = parse_p8(&raw).unwrap_or_else(|e| panic!("{what} must parse: {e}"));
            assert_eq!(
                mint_jwt(&parsed, "k", "t", 1),
                mint_jwt(&key, "k", "t", 1),
                "{what} must load the SAME key"
            );
        }
        assert!(parse_p8("not a key at all !!!").is_err());
        assert!(parse_p8("").is_err());
    }

    /// The REAL Apple key, when this workstation has it: proves the parser handles a
    /// genuine `AuthKey_*.p8` (Apple's own P-256 PKCS#8), not just one we generated. The
    /// SANDBOX key, because that is the environment the development-signed app's tokens
    /// belong to. Skipped — never failed — where the file is absent (CI, the VM, another
    /// machine). NO network call, and no key material is ever printed: the assertions are
    /// on the JWT's header and claims only.
    #[test]
    fn real_apple_auth_key_signs_a_valid_provider_token() {
        // The gitignored env/ dir of the sibling Pacific checkout (local dev only).
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../pacific/env/AuthKey_5W68RMT84R.p8");
        let Ok(pem) = std::fs::read_to_string(&path) else {
            eprintln!("skipping: no local AuthKey_5W68RMT84R.p8 (expected off this workstation)");
            return;
        };
        let key = parse_p8(&pem).expect("a genuine Apple APNs key must parse");
        let jwt = mint_jwt(&key, "5W68RMT84R", "9LFA85922G", 1_750_000_000);
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let header: serde_json::Value = serde_json::from_slice(&b64url_decode(parts[0])).unwrap();
        assert_eq!(header["alg"], "ES256");
        assert_eq!(header["kid"], "5W68RMT84R");
        let claims: serde_json::Value = serde_json::from_slice(&b64url_decode(parts[1])).unwrap();
        assert_eq!(claims["iss"], "9LFA85922G");
        assert_eq!(claims["iat"], 1_750_000_000);
        let sig = Signature::from_slice(&b64url_decode(parts[2])).expect("raw r‖s");
        VerifyingKey::from(&key)
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig)
            .expect("Apple's key must produce a self-consistent ES256 signature");
    }

    /// The two failures that cost an hour must NAME the environment mismatch, and the
    /// host they name must be the host the push actually went to.
    #[test]
    fn diagnose_names_the_environment_mismatch() {
        let d = diagnose(400, "BadDeviceToken", HOST_SANDBOX).expect("must diagnose");
        assert!(d.contains("sandbox") && d.contains("production"));
        assert!(d.contains("aps-environment"), "names the real cause");
        let d = diagnose(400, "BadDeviceToken", HOST_PROD).expect("must diagnose");
        assert!(d.contains(HOST_PROD));
        let d = diagnose(403, "InvalidProviderToken", HOST_SANDBOX).expect("must diagnose");
        assert!(d.contains("APNS_KEY_ID") && d.contains("60 minutes"));
        assert!(
            diagnose(200, "", HOST_SANDBOX).is_none(),
            "success is not a diagnosis"
        );
        assert!(diagnose(429, "TooManyRequests", HOST_SANDBOX).is_none());
    }

    /// The environment is chosen by the FLAG, never inferred: the host follows
    /// `sandbox`, and `environment()` reports what the host says.
    #[test]
    fn host_follows_the_sandbox_flag() {
        let mk = |sandbox| Apns::new(test_key(), "k".into(), "t".into(), "top".into(), sandbox);
        assert_eq!(mk(true).host, HOST_SANDBOX);
        assert_eq!(mk(true).environment(), "sandbox");
        assert_eq!(mk(false).host, HOST_PROD);
        assert_eq!(mk(false).environment(), "production");
    }

    /// `from_env` with nothing set is `None` — the fail-soft state the plane serves in.
    /// (Env is process-global; this asserts the no-credential branch only, without
    /// setting anything, so it cannot race a parallel test.)
    #[test]
    fn no_credentials_means_no_pusher() {
        if std::env::var("APNS_KEY_P8").is_ok()
            || std::env::var("APNS_KEY_PATH").is_ok()
            || std::env::var("APNS_KEY_ID").is_ok()
        {
            return; // a developer's shell has real creds — nothing to assert here
        }
        assert!(Apns::from_env().is_none());
    }
}
