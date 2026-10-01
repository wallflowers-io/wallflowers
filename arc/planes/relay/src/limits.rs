//! limits — how much the relay accepts, and how fast.
//!
//! WHAT THE SIGNATURE ALREADY SETTLED. Every `Pub` is signed by the key its tag IS
//! (`pacific_wire::address`), so nobody can write where they hold no key: not to
//! someone's head, not to their wrap, not to a group they are not in. Flooding a
//! tag to bury or evict what is there needs that tag's secret.
//!
//! WHAT IT DOES NOT SETTLE, AND WHY THESE LIMITS ARE THE ANSWER. An address costs
//! one keypair, so an anonymous caller can mint addresses of its own and fill the
//! store under them. Both admission schemes that would price that were evaluated on
//! 18 Sep 2026 and rejected for v1: proof of work cannot make junk cost an attacker
//! more than it costs us at a delay a phone will bear (a GPU fills the store for
//! cents), and anonymous write tokens only move the question to an issuer that has
//! nothing to count — there are no accounts, and a new identity costs microseconds.
//!
//! So the relay bounds HOW MUCH and HOW FAST, which is all a blind relay can do:
//!
//!   * one publish is at most `max_blob_bytes`, and the websocket refuses a larger
//!     message before it is buffered (tungstenite's own default is 64 MiB);
//!   * one connection gets a paced budget of publishes, of bytes, and of NEW
//!     addresses;
//!   * the whole relay has an ingest ceiling;
//!   * and the store FAILS CLOSED at `max_store_bytes`: it refuses, it never
//!     evicts. The worst a flood can do is stop writes. It cannot destroy anyone's
//!     records, which is the property the relay must keep now that it is the store
//!     of record rather than a transport.
//!
//! None of this can tell junk from mail. Every refusal is counted and named on the
//! wire (`Ack.reason`), and none of it is logged beside a tag.

use std::time::Instant;

/// Everything the relay will accept. Read once at boot; see [`Limits::from_env`].
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Largest blob one `Pub` may carry, as sent (base64). Media never travels
    /// through the mailbox — it has its own presigned path — so this is a bound on
    /// sealed deltas, commits and Welcomes.
    pub max_blob_bytes: usize,
    /// The store's ceiling. At it, every publish is refused as `full` until an
    /// operator acts. 0 = no ceiling, which only an operator setting
    /// `RELAY_MAX_STORE_BYTES=0` on purpose produces.
    pub max_store_bytes: u64,
    /// Publishes per second per connection, and the burst allowed above it.
    pub conn_pubs_per_sec: f64,
    pub conn_pub_burst: f64,
    /// Bytes per second per connection, and the burst.
    pub conn_bytes_per_sec: f64,
    pub conn_bytes_burst: f64,
    /// NEW addresses — tags with nothing stored yet — per minute per connection, and
    /// the burst. The cheap thing an attacker has is keys; this is where they are
    /// spent. An honest device creates one per group epoch, per intro, per chain
    /// entry: a handful a minute at the very most.
    pub conn_new_tags_per_min: f64,
    pub conn_new_tag_burst: f64,
    /// Bytes per second across every connection, and the burst. The last line: a
    /// thousand connections each within their own pace still meet this.
    pub ingest_bytes_per_sec: f64,
    pub ingest_bytes_burst: f64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_blob_bytes: 256 * 1024,
            max_store_bytes: 8 * 1024 * 1024 * 1024,
            conn_pubs_per_sec: 50.0,
            conn_pub_burst: 200.0,
            conn_bytes_per_sec: 2.0 * 1024.0 * 1024.0,
            conn_bytes_burst: 16.0 * 1024.0 * 1024.0,
            conn_new_tags_per_min: 120.0,
            conn_new_tag_burst: 300.0,
            ingest_bytes_per_sec: 32.0 * 1024.0 * 1024.0,
            ingest_bytes_burst: 128.0 * 1024.0 * 1024.0,
        }
    }
}

impl Limits {
    /// The defaults, overridden per field by `RELAY_MAX_BLOB_BYTES`,
    /// `RELAY_MAX_STORE_BYTES`, `RELAY_CONN_PUBS_PER_SEC`, `RELAY_CONN_PUB_BURST`,
    /// `RELAY_CONN_BYTES_PER_SEC`, `RELAY_CONN_BYTES_BURST`,
    /// `RELAY_CONN_NEW_TAGS_PER_MIN`, `RELAY_CONN_NEW_TAG_BURST`,
    /// `RELAY_INGEST_BYTES_PER_SEC` and `RELAY_INGEST_BYTES_BURST`.
    ///
    /// A byte burst smaller than one maximal blob could never admit that blob, so
    /// both byte bursts are raised to at least `max_blob_bytes`.
    pub fn from_env() -> Self {
        let d = Self::default();
        let u = |k: &str| std::env::var(k).ok().and_then(|v| v.trim().parse::<u64>().ok());
        let f = |k: &str, dv: f64| u(k).map(|v| v as f64).unwrap_or(dv);
        let mut l = Self {
            max_blob_bytes: u("RELAY_MAX_BLOB_BYTES").map(|v| v as usize).unwrap_or(d.max_blob_bytes),
            max_store_bytes: u("RELAY_MAX_STORE_BYTES").unwrap_or(d.max_store_bytes),
            conn_pubs_per_sec: f("RELAY_CONN_PUBS_PER_SEC", d.conn_pubs_per_sec),
            conn_pub_burst: f("RELAY_CONN_PUB_BURST", d.conn_pub_burst),
            conn_bytes_per_sec: f("RELAY_CONN_BYTES_PER_SEC", d.conn_bytes_per_sec),
            conn_bytes_burst: f("RELAY_CONN_BYTES_BURST", d.conn_bytes_burst),
            conn_new_tags_per_min: f("RELAY_CONN_NEW_TAGS_PER_MIN", d.conn_new_tags_per_min),
            conn_new_tag_burst: f("RELAY_CONN_NEW_TAG_BURST", d.conn_new_tag_burst),
            ingest_bytes_per_sec: f("RELAY_INGEST_BYTES_PER_SEC", d.ingest_bytes_per_sec),
            ingest_bytes_burst: f("RELAY_INGEST_BYTES_BURST", d.ingest_bytes_burst),
        };
        l.conn_bytes_burst = l.conn_bytes_burst.max(l.max_blob_bytes as f64);
        l.ingest_bytes_burst = l.ingest_bytes_burst.max(l.max_blob_bytes as f64);
        l
    }

    /// The largest websocket message the relay will read: one maximal blob plus
    /// room for the frame around it (tag, signature, JSON). A larger message is
    /// refused by the websocket layer before it is buffered.
    pub fn max_message_bytes(&self) -> usize {
        self.max_blob_bytes + 16 * 1024
    }

    pub(crate) fn conn_limiter(&self) -> ConnLimiter {
        ConnLimiter {
            pubs: Bucket::new(self.conn_pubs_per_sec, self.conn_pub_burst),
            bytes: Bucket::new(self.conn_bytes_per_sec, self.conn_bytes_burst),
            new_tags: Bucket::new(self.conn_new_tags_per_min / 60.0, self.conn_new_tag_burst),
        }
    }

    pub(crate) fn ingest(&self) -> Bucket {
        Bucket::new(self.ingest_bytes_per_sec, self.ingest_bytes_burst)
    }
}

/// One connection's pace.
pub(crate) struct ConnLimiter {
    pub pubs: Bucket,
    pub bytes: Bucket,
    pub new_tags: Bucket,
}

/// A token bucket. A rate of 0 means unlimited.
pub(crate) struct Bucket {
    rate: f64,
    capacity: f64,
    tokens: f64,
    at: Instant,
}

impl Bucket {
    pub fn new(rate_per_sec: f64, burst: f64) -> Self {
        Self { rate: rate_per_sec, capacity: burst, tokens: burst, at: Instant::now() }
    }

    /// Take `n` tokens if they are there. Refills by elapsed time first.
    pub fn take(&mut self, n: f64) -> bool {
        if self.rate <= 0.0 {
            return true;
        }
        let now = Instant::now();
        let dt = now.duration_since(self.at).as_secs_f64();
        self.at = now;
        self.tokens = (self.tokens + dt * self.rate).min(self.capacity);
        if self.tokens >= n {
            self.tokens -= n;
            true
        } else {
            false
        }
    }

    pub fn is_limited(&self) -> bool {
        self.rate > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bucket_admits_its_burst_and_then_refuses() {
        let mut b = Bucket::new(1.0, 3.0);
        assert!(b.take(1.0) && b.take(1.0) && b.take(1.0));
        assert!(!b.take(1.0), "the burst is spent and a second has not passed");
    }

    #[test]
    fn a_zero_rate_is_unlimited() {
        let mut b = Bucket::new(0.0, 0.0);
        assert!((0..10_000).all(|_| b.take(1.0)));
        assert!(!b.is_limited());
    }

    #[test]
    fn no_byte_burst_is_smaller_than_one_blob() {
        // A burst below the blob cap could never admit a maximal blob, which would
        // make the cap a lie.
        let l = Limits::from_env();
        assert!(l.conn_bytes_burst >= l.max_blob_bytes as f64);
        assert!(l.ingest_bytes_burst >= l.max_blob_bytes as f64);
    }
}
