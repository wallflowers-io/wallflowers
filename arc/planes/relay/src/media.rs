//! media — large-payload offload to S3-compatible object storage (Cloudflare R2).
//!
//! ## The shape of this, and why it is not a proxy
//!
//! The mailbox in `store.rs` is the right home for MLS packets: they are small,
//! ordered, and their retention is a promise to an offline device. It is the
//! wrong home for a photo. Media is large, unordered, and wants a different
//! lifetime — putting it in the SQLite blob table would trade the mailbox's
//! bounded size for someone's camera roll.
//!
//! So media does not go through the relay at all. A client asks for a presigned
//! URL, uploads ciphertext **directly** to R2, and then publishes an ordinary
//! sealed packet naming the key. Recipients ask for a presigned GET and fetch it
//! themselves. The relay mints URLs; it never sees a byte of media, never buffers
//! one, and never pays egress for one.
//!
//! That is deliberate on three counts:
//!
//!   1. BLINDNESS. The relay's contract is that it cannot see what it carries.
//!      Proxying media would make it the one component that touches every photo.
//!   2. THE TUNNEL. Media that never enters the process never crosses the deploy's
//!      ingress either — the relay's uplink stops being the bottleneck for a
//!      50 MB video.
//!   3. COST. R2 charges no egress. A proxied byte would be paid for twice.
//!
//! ## What the relay learns
//!
//! A key, an object size, and that *someone* wants a URL. Not who, not what, not
//! which group — exactly the posture `Frame::Pub` already has, where the commit
//! flag is the only thing the relay is allowed to notice. Keys are opaque 32-byte
//! hex, generated client-side like tags, and this module validates their SHAPE
//! and nothing else.
//!
//! ## Keys are validated, not trusted
//!
//! `require_valid_key` is the only thing standing between a presigning endpoint
//! and an arbitrary-path write oracle for the bucket. A key that is not exactly
//! 64 lowercase hex characters is refused before it can reach a canonical URI —
//! that rules out traversal (`../`), collisions with any future prefix, and
//! objects whose names carry meaning the relay should not be able to read.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tracing::warn;

use crate::budget::{self, Budget, Verdict};
use crate::sigv4::{presign, Credentials, Presign};

/// 25 MiB. A placeholder in the same sense as `Retention`'s window: large enough
/// for a phone photo or a short clip, small enough that one client cannot fill a
/// bucket by accident. It has NOT been ruled on as a product limit.
const DEFAULT_MAX_BYTES: u64 = 25 * 1024 * 1024;

/// Five minutes. Long enough to start an upload on a poor connection, short
/// enough that a leaked URL is worth little. The object stays encrypted either
/// way — the URL is a capability to move ciphertext, not to read anything.
const DEFAULT_TTL: Duration = Duration::from_secs(300);

/// Why a presign was refused. These strings cross the wire to the client, so they
/// are stable, short, and carry nothing about other clients or the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaError {
    /// The relay has no object-storage configured; media offload is off.
    Unconfigured,
    /// The key was not exactly 64 lowercase hex characters.
    BadKey,
    /// The declared length exceeded `max_bytes`.
    TooLarge,
    /// Storing this object would push the bucket past its storage budget.
    OverBudget,
    /// The storage meter has no recent reading, so nothing is being authorised.
    /// Distinct from `OverBudget` on purpose: the bucket may have room, but the
    /// relay cannot currently prove it, and a cost guard that cannot see says no.
    BudgetStale,
}

impl MediaError {
    /// The stable wire token for `Frame::MediaErr { reason }`.
    pub fn reason(self) -> &'static str {
        match self {
            MediaError::Unconfigured => "unconfigured",
            MediaError::BadKey => "bad_key",
            MediaError::TooLarge => "too_large",
            MediaError::OverBudget => "over_budget",
            MediaError::BudgetStale => "budget_stale",
        }
    }
}

/// Object-storage configuration. Absent (`None` from [`Media::from_env`]) means
/// media offload is disabled and every request is refused with `unconfigured` —
/// never silently accepted and dropped.
#[derive(Debug, Clone)]
pub struct Media {
    /// Host only — the scheme is always https and is added by the presigner.
    host: String,
    bucket: String,
    region: String,
    creds: Credentials,
    max_bytes: u64,
    ttl: Duration,
    /// The storage safety net. `None` only when explicitly uncapped.
    budget: Option<Arc<Budget>>,
    /// How often `lib.rs` should re-measure real usage.
    poll: Duration,
}

impl Media {
    /// Read configuration from the environment.
    ///
    /// Returns `None` when the four required variables are not all present. Like
    /// the Sentry DSN in `main.rs`, a partial configuration is announced loudly
    /// rather than half-applied: running with three of four set is a deploy
    /// mistake, and the quiet version of it is a feature that silently does not
    /// exist.
    pub fn from_env() -> Option<Self> {
        let endpoint = env_str("RELAY_R2_ENDPOINT");
        let bucket = env_str("RELAY_R2_BUCKET");
        let access_key_id = env_str("RELAY_R2_ACCESS_KEY_ID");
        let secret_access_key = env_str("RELAY_R2_SECRET_ACCESS_KEY");

        let present = [&endpoint, &bucket, &access_key_id, &secret_access_key]
            .iter()
            .filter(|v| v.is_some())
            .count();
        if present == 0 {
            warn!("media offload disabled (RELAY_R2_* unset) — MediaPut/MediaGet will be refused");
            return None;
        }
        if present < 4 {
            warn!(
                configured = present,
                "media offload DISABLED: RELAY_R2_* is partially configured (need endpoint, \
                 bucket, access key id, secret). Refusing to run half-enabled."
            );
            return None;
        }

        // Accept a full URL or a bare host; the presigner signs the host only, and
        // a stray scheme or trailing slash folded into it is an unsigned-request bug
        // that only shows up as a 403 from R2.
        let host = endpoint
            .as_deref()
            .unwrap()
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .to_string();

        // The storage safety net. `RELAY_R2_BUDGET_BYTES=0` disables the cap, using
        // the same "0 means off" idiom as `RELAY_RETENTION_SECS` — and gets the same
        // loud warning, because an uncapped bucket bills without limit.
        let bucket_name = bucket.unwrap();
        let budget_bytes = env_str("RELAY_R2_BUDGET_BYTES")
            .and_then(|v| v.parse().ok())
            .unwrap_or(budget::DEFAULT_BUDGET_BYTES);
        let poll = env_str("RELAY_R2_USAGE_POLL_SECS")
            .and_then(|v| v.parse().ok())
            .map(Duration::from_secs)
            .unwrap_or(budget::DEFAULT_POLL);

        let budget_obj = if budget_bytes == 0 {
            warn!(
                "RELAY_R2_BUDGET_BYTES=0 — object storage is UNCAPPED. Nothing stops this \
                 relay filling the bucket and leaving the free tier."
            );
            None
        } else {
            // Stale after three missed measurements: tolerant of one blip, not of an
            // outage. Measurement reuses the S3 credentials configured above, so
            // enabling the cap costs no extra secret.
            Some(Arc::new(Budget::new(budget_bytes, poll * 3)))
        };

        Some(Self {
            host,
            bucket: bucket_name,
            region: env_str("RELAY_R2_REGION").unwrap_or_else(|| "auto".into()),
            creds: Credentials {
                access_key_id: access_key_id.unwrap(),
                secret_access_key: secret_access_key.unwrap(),
            },
            max_bytes: env_str("RELAY_MEDIA_MAX_BYTES")
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_MAX_BYTES),
            ttl: env_str("RELAY_MEDIA_URL_TTL_SECS")
                .and_then(|v| v.parse().ok())
                .map(Duration::from_secs)
                .unwrap_or(DEFAULT_TTL),
            budget: budget_obj,
            poll,
        })
    }

    /// Build a config directly. Tests and callers that do not read the process
    /// environment use this; `from_env` is a thin wrapper over it.
    pub fn new(
        host: impl Into<String>,
        bucket: impl Into<String>,
        creds: Credentials,
        max_bytes: u64,
        ttl: Duration,
    ) -> Self {
        Self {
            host: host.into(),
            bucket: bucket.into(),
            region: "auto".into(),
            creds,
            max_bytes,
            ttl,
            budget: None,
            poll: budget::DEFAULT_POLL,
        }
    }

    /// Attach a storage budget. Used by `from_env` and by tests that exercise the
    /// safety net without touching the process environment.
    pub fn with_budget(mut self, budget: Arc<Budget>) -> Self {
        self.budget = Some(budget);
        self
    }

    /// The meter, for the poll loop and for stats.
    pub fn budget(&self) -> Option<&Arc<Budget>> {
        self.budget.as_ref()
    }

    /// Presign one page of a `ListObjectsV2` over the bucket — how the meter reads
    /// real usage. Signed like any other request, so the URL is good for this one
    /// page and nothing else.
    pub fn presign_list(&self, continuation: Option<&str>) -> String {
        let mut query = vec![
            ("list-type".to_string(), "2".to_string()),
            ("max-keys".to_string(), budget::LIST_PAGE_SIZE.to_string()),
        ];
        if let Some(token) = continuation {
            query.push(("continuation-token".to_string(), token.to_string()));
        }
        presign(
            &Presign {
                method: "GET",
                host: &self.host,
                path_segments: &[&self.bucket],
                region: &self.region,
                expires_in: self.ttl_secs(),
                signed_headers: &[],
                query: &query,
                now: Utc::now(),
            },
            &self.creds,
        )
    }

    /// Interval between usage measurements.
    pub fn poll(&self) -> Duration {
        self.poll
    }

    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    pub fn ttl_secs(&self) -> u32 {
        self.ttl.as_secs() as u32
    }

    /// Presign an upload of EXACTLY `len` bytes.
    ///
    /// `content-length` is folded into the signature, so the returned URL cannot
    /// be replayed to store a larger object than the one that was authorised.
    pub fn presign_put(&self, key: &str, len: u64) -> Result<String, MediaError> {
        require_valid_key(key)?;
        if len > self.max_bytes {
            return Err(MediaError::TooLarge);
        }
        // Claim the headroom BEFORE signing: a URL that exists is a URL that may be
        // redeemed, so the reservation has to happen on the authorising side of the
        // decision, not after it.
        if let Some(b) = &self.budget {
            match b.reserve(len, budget::now_secs()) {
                Verdict::Ok => {}
                Verdict::OverBudget => return Err(MediaError::OverBudget),
                Verdict::Stale => return Err(MediaError::BudgetStale),
            }
        }
        Ok(self.sign("PUT", key, &[("content-length".into(), len.to_string())]))
    }

    /// Presign a download. The relay does not check that the object exists — that
    /// would be an I/O round-trip to answer a question the client finds out anyway,
    /// and a blind relay has no business tracking which keys are live.
    pub fn presign_get(&self, key: &str) -> Result<String, MediaError> {
        require_valid_key(key)?;
        Ok(self.sign("GET", key, &[]))
    }

    fn sign(&self, method: &str, key: &str, extra: &[(String, String)]) -> String {
        presign(
            &Presign {
                method,
                host: &self.host,
                path_segments: &[&self.bucket, key],
                region: &self.region,
                expires_in: self.ttl_secs(),
                signed_headers: extra,
                query: &[],
                now: Utc::now(),
            },
            &self.creds,
        )
    }
}

/// Keys are opaque 32-byte identifiers rendered as lowercase hex — the same shape
/// as a routing `tag`, for the same reason: the relay must not be able to read
/// meaning into one, and must not be usable to write anywhere it chooses.
fn require_valid_key(key: &str) -> Result<(), MediaError> {
    let ok = key.len() == 64
        && key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if ok {
        Ok(())
    } else {
        Err(MediaError::BadKey)
    }
}

fn env_str(k: &str) -> Option<String> {
    std::env::var(k)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media() -> Media {
        Media::new(
            "acct.r2.cloudflarestorage.com",
            "pacific-media",
            Credentials {
                access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
                secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            },
            1024,
            Duration::from_secs(300),
        )
    }

    fn key() -> String {
        "ab".repeat(32)
    }

    #[test]
    fn presigns_put_and_get() {
        let m = media();
        let put = m.presign_put(&key(), 512).unwrap();
        assert!(put.starts_with("https://acct.r2.cloudflarestorage.com/pacific-media/"));
        assert!(put.contains("X-Amz-Signature="));
        assert!(put.contains("X-Amz-Expires=300"));
        let get = m.presign_get(&key()).unwrap();
        assert!(get.contains("X-Amz-Signature="));
        // Different verbs must not produce the same capability.
        assert_ne!(put, get);
    }

    /// The bound that stops a presigned PUT being a blank cheque.
    #[test]
    fn refuses_oversized_uploads() {
        assert_eq!(media().presign_put(&key(), 1025), Err(MediaError::TooLarge));
        assert!(media().presign_put(&key(), 1024).is_ok());
    }

    /// The check that stops the relay being an arbitrary-path write oracle.
    #[test]
    fn refuses_keys_that_are_not_opaque_hex() {
        let m = media();
        for bad in [
            "",
            "short",
            &"ab".repeat(31),                  // 62 chars
            &"ab".repeat(33),                  // 66 chars
            &format!("{}/x", "ab".repeat(31)), // embedded separator
            &"AB".repeat(32),                  // uppercase
            &"zz".repeat(32),                  // non-hex
            "../../etc/passwd",
        ] {
            assert_eq!(
                m.presign_put(bad, 1).unwrap_err(),
                MediaError::BadKey,
                "accepted {bad:?}"
            );
            assert_eq!(
                m.presign_get(bad).unwrap_err(),
                MediaError::BadKey,
                "accepted {bad:?}"
            );
        }
    }

    /// A traversal attempt must be refused by shape, never encoded and passed on.
    #[test]
    fn traversal_cannot_escape_the_bucket_prefix() {
        assert_eq!(
            media().presign_get("../pacific-media-other/x").unwrap_err(),
            MediaError::BadKey
        );
    }

    #[test]
    fn wire_reasons_are_stable() {
        assert_eq!(MediaError::Unconfigured.reason(), "unconfigured");
        assert_eq!(MediaError::BadKey.reason(), "bad_key");
        assert_eq!(MediaError::TooLarge.reason(), "too_large");
    }

    /// A download stores nothing, so it must not be gated on storage headroom —
    /// otherwise a full bucket would also become an unreadable one.
    #[test]
    fn downloads_are_not_budget_gated() {
        let b = Arc::new(Budget::new(100, Duration::from_secs(900)));
        b.record(100, budget::now_secs()); // completely full
        let m = media().with_budget(b);
        assert_eq!(
            m.presign_put(&key(), 1).unwrap_err(),
            MediaError::OverBudget
        );
        assert!(
            m.presign_get(&key()).is_ok(),
            "GET must survive a full bucket"
        );
    }

    #[test]
    fn uploads_are_refused_when_the_bucket_is_full() {
        let b = Arc::new(Budget::new(1000, Duration::from_secs(900)));
        b.record(900, budget::now_secs());
        let m = media().with_budget(b);
        assert!(m.presign_put(&key(), 100).is_ok());
        // The first reservation consumed the remaining headroom.
        assert_eq!(
            m.presign_put(&key(), 1).unwrap_err(),
            MediaError::OverBudget
        );
    }

    /// Fail-closed, surfaced through the wire vocabulary rather than as a success.
    #[test]
    fn uploads_are_refused_while_the_meter_is_blind() {
        let b = Arc::new(Budget::new(1_000_000, Duration::from_secs(900)));
        // No `record` has happened, so the relay has never heard from R2.
        let m = media().with_budget(b);
        assert_eq!(
            m.presign_put(&key(), 1).unwrap_err(),
            MediaError::BudgetStale
        );
    }

    /// An oversized request must be refused on size alone, without consuming any
    /// of the budget it was never going to be allowed to use.
    #[test]
    fn an_oversized_request_does_not_consume_headroom() {
        let b = Arc::new(Budget::new(1_000_000, Duration::from_secs(900)));
        b.record(0, budget::now_secs());
        let m = media().with_budget(b.clone());
        assert_eq!(
            m.presign_put(&key(), 99_999).unwrap_err(),
            MediaError::TooLarge
        );
        assert_eq!(b.committed(), 0, "a refused request reserved bytes anyway");
    }

    /// Likewise a malformed key: validation precedes accounting.
    #[test]
    fn a_bad_key_does_not_consume_headroom() {
        let b = Arc::new(Budget::new(1_000_000, Duration::from_secs(900)));
        b.record(0, budget::now_secs());
        let m = media().with_budget(b.clone());
        assert_eq!(m.presign_put("nope", 10).unwrap_err(), MediaError::BadKey);
        assert_eq!(b.committed(), 0);
    }

    #[test]
    fn presigns_a_listing_for_the_meter() {
        let m = media();
        let first = m.presign_list(None);
        assert!(first.contains("list-type=2"));
        assert!(first.contains("max-keys=1000"));
        assert!(first.starts_with("https://acct.r2.cloudflarestorage.com/pacific-media?"));
        let next = m.presign_list(Some("tok/en+1="));
        assert!(next.contains("continuation-token="));
        assert_ne!(first, next);
    }

    #[test]
    fn secrets_do_not_leak_through_debug() {
        let rendered = format!("{:?}", media());
        assert!(
            !rendered.contains("wJalrXUtnFEMI"),
            "secret leaked: {rendered}"
        );
    }
}
