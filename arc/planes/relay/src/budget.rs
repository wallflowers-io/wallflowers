//! budget — the storage safety net. Refuses uploads before R2 leaves the free tier.
//!
//! ## Why the relay has to be the one to say no
//!
//! Every byte that can enter the bucket is authorised by this process: nothing
//! else holds a key, and a client cannot write without a presigned URL. That makes
//! the relay the only place a cap can actually be ENFORCED rather than observed
//! after the fact. A dashboard alert tells you the bill already happened.
//!
//! ## Measured, plus what is in flight
//!
//! Two numbers decide every request:
//!
//!   * `measured` — the bucket's real size, summed from `ListObjectsV2`.
//!
//!     NOT Cloudflare's R2 usage API, though that endpoint is far cheaper and was
//!     the obvious first choice. It is a lagged ANALYTICS surface: measured against
//!     this bucket it reported `payloadSize: 0, objectCount: 0` while a listing of
//!     the same bucket at the same moment showed 2 objects and 8192 bytes. A
//!     measurement is not merely informational here — it RESETS `reserved` — so a
//!     stale zero would repeatedly wipe the in-flight accounting and re-open the
//!     gate. A guard fed by a lagging meter is a guard that fails open under
//!     exactly the sustained upload it exists to stop. `ListObjectsV2` is
//!     authoritative and immediate, and reuses the S3 credentials that already
//!     have to be configured, so it needs no second token.
//!   * `reserved` — bytes authorised since that measurement. A presign is a
//!     promise the client may redeem at any moment up to the URL's expiry, so a
//!     burst of them can outrun a periodic poll. Counting every authorised byte as
//!     though it were already stored is deliberately pessimistic: the error is
//!     always in the direction of refusing too early, never of overshooting.
//!
//! Each successful measurement replaces `measured` and clears `reserved`, so
//! unredeemed presigns wash out on the next poll rather than accumulating forever.
//!
//! ## It fails CLOSED
//!
//! If the usage API has not answered within `stale_after`, presigning stops. That
//! is the entire point of a cost guard: a meter that cannot see is not permission
//! to keep spending. This trades availability for a bounded bill, deliberately —
//! media stops, the mailbox is untouched, and the refusal is loud in both the logs
//! and the `media_refused` counter.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 9 GB against a 10 GB free tier. The GB of headroom absorbs the lag between a
/// reservation and the poll that confirms it, plus the metadata R2 bills that no
/// client ever sees.
pub const DEFAULT_BUDGET_BYTES: u64 = 9_000_000_000;

/// How often to ask R2 how big it actually is.
///
/// Fifteen minutes, chosen against the OTHER free-tier limit: listing costs a
/// Class A operation per page, and those are capped at 1M/month. At this cadence
/// even a worst-case walk (`MAX_LIST_PAGES`) spends well under a fifth of that
/// allowance, so the meter cannot become the thing that busts the tier it guards.
/// The cost of the longer interval is only that reservations accumulate for longer
/// between reconciliations, which errs towards refusing early.
pub const DEFAULT_POLL: Duration = Duration::from_secs(900);

/// Verdict on a reservation. Kept separate from `MediaError` so this module owes
/// nothing to the wire vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    /// The bucket is full enough that this upload would cross the budget.
    OverBudget,
    /// No recent measurement — refusing rather than spending blind.
    Stale,
}

/// Objects per `ListObjectsV2` page. 1000 is the S3 maximum, and fewer pages means
/// fewer Class A operations spent measuring.
pub const LIST_PAGE_SIZE: u32 = 1000;

/// Hard ceiling on pages walked in one measurement.
///
/// Listing costs a Class A operation per page, so an unbounded walk over a bucket
/// with millions of small objects could itself consume the free tier it is meant to
/// protect. Hitting this cap is treated as "at budget" rather than as a number:
/// a measurement we refused to finish is not evidence of headroom.
pub const MAX_LIST_PAGES: u32 = 64;

/// The meter. Cheap to consult: a reservation is a load plus one CAS, so the hot
/// path never blocks on the network or takes a lock.
#[derive(Debug)]
pub struct Budget {
    max_bytes: u64,
    stale_after: Duration,
    measured: AtomicU64,
    reserved: AtomicU64,
    /// Unix seconds of the last successful measurement; 0 means "never", which is
    /// stale by definition — a relay that has not yet heard from R2 does not
    /// presign.
    measured_at: AtomicU64,
}

impl Budget {
    pub fn new(max_bytes: u64, stale_after: Duration) -> Self {
        Self {
            max_bytes,
            stale_after,
            measured: AtomicU64::new(0),
            reserved: AtomicU64::new(0),
            measured_at: AtomicU64::new(0),
        }
    }

    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    /// Bytes accounted for right now: last measurement plus everything authorised
    /// since. Reporting only.
    pub fn committed(&self) -> u64 {
        self.measured
            .load(Ordering::Relaxed)
            .saturating_add(self.reserved.load(Ordering::Relaxed))
    }

    /// Record a fresh measurement and clear the in-flight reservations it now
    /// accounts for.
    pub fn record(&self, bytes: u64, now_secs: u64) {
        self.measured.store(bytes, Ordering::Relaxed);
        self.reserved.store(0, Ordering::Relaxed);
        self.measured_at.store(now_secs, Ordering::Relaxed);
    }

    /// Try to claim `len` bytes of headroom.
    ///
    /// The CAS loop matters: two connections presigning at once must not both read
    /// the same headroom and both be told yes. Under contention the loser retries
    /// against the updated total, so the sum of granted reservations can never
    /// exceed what was available.
    pub fn reserve(&self, len: u64, now_secs: u64) -> Verdict {
        let at = self.measured_at.load(Ordering::Relaxed);
        if at == 0 || now_secs.saturating_sub(at) > self.stale_after.as_secs() {
            return Verdict::Stale;
        }
        let measured = self.measured.load(Ordering::Relaxed);
        loop {
            let reserved = self.reserved.load(Ordering::Relaxed);
            if measured.saturating_add(reserved).saturating_add(len) > self.max_bytes {
                return Verdict::OverBudget;
            }
            if self
                .reserved
                .compare_exchange_weak(
                    reserved,
                    reserved + len,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                return Verdict::Ok;
            }
        }
    }
}

/// One page of a `ListObjectsV2` response: the bytes it accounts for, and the
/// continuation token if the listing is truncated.
#[derive(Debug, PartialEq, Eq)]
pub struct ListPage {
    pub bytes: u64,
    pub next: Option<String>,
}

/// Sum `<Size>` across a `ListObjectsV2` page and pull out the continuation token.
///
/// A deliberate scan rather than a full XML parser: the two elements wanted here
/// appear nowhere else in the response, and an XML crate would be a new dependency
/// for a fifteen-line extraction. The cost of that choice is that malformed input
/// must be an ERROR, never a zero — a silent zero would read as "empty bucket" and
/// open the gate, so `<Size>` values that do not parse are refused outright.
pub fn parse_list_page(xml: &str) -> Result<ListPage, String> {
    if !xml.contains("<ListBucketResult") {
        // An S3 <Error> document, an HTML error page, or a truncated body.
        return Err("not a ListBucketResult response".into());
    }
    let mut bytes: u64 = 0;
    for chunk in xml.split("<Size>").skip(1) {
        let raw = chunk
            .split_once("</Size>")
            .map(|(v, _)| v.trim())
            .ok_or("unterminated <Size> element")?;
        let n: u64 = raw
            .parse()
            .map_err(|_| format!("unparseable <Size> value: {raw:?}"))?;
        bytes = bytes.saturating_add(n);
    }
    let next = xml
        .split_once("<NextContinuationToken>")
        .and_then(|(_, rest)| rest.split_once("</NextContinuationToken>"))
        .map(|(tok, _)| tok.trim().to_string())
        .filter(|t| !t.is_empty());
    Ok(ListPage { bytes, next })
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STALE: Duration = Duration::from_secs(900);

    fn budget(max: u64) -> Budget {
        Budget::new(max, STALE)
    }

    /// Before the first measurement the meter is blind, and blind means no.
    #[test]
    fn refuses_until_it_has_measured() {
        let b = budget(1000);
        assert_eq!(b.reserve(1, now_secs()), Verdict::Stale);
    }

    #[test]
    fn allows_within_budget_once_measured() {
        let b = budget(1000);
        b.record(400, now_secs());
        assert_eq!(b.reserve(500, now_secs()), Verdict::Ok);
    }

    /// The headroom is measured + in-flight, not measured alone — otherwise a
    /// burst of presigns between two polls could each look affordable on their own.
    #[test]
    fn reservations_accumulate_against_the_cap() {
        let b = budget(1000);
        b.record(0, now_secs());
        assert_eq!(b.reserve(600, now_secs()), Verdict::Ok);
        assert_eq!(b.reserve(500, now_secs()), Verdict::OverBudget);
        assert_eq!(b.reserve(400, now_secs()), Verdict::Ok);
        assert_eq!(b.committed(), 1000);
    }

    /// A measurement is the truth; unredeemed presigns must not accumulate forever.
    #[test]
    fn a_measurement_clears_in_flight_reservations() {
        let b = budget(1000);
        b.record(0, now_secs());
        assert_eq!(b.reserve(900, now_secs()), Verdict::Ok);
        assert_eq!(b.reserve(200, now_secs()), Verdict::OverBudget);
        // Most of those uploads never happened; R2 says the bucket holds 100.
        b.record(100, now_secs());
        assert_eq!(b.committed(), 100);
        assert_eq!(b.reserve(200, now_secs()), Verdict::Ok);
    }

    /// The fail-closed property, stated as a test so it cannot be softened by
    /// accident.
    #[test]
    fn goes_closed_when_the_measurement_goes_stale() {
        let b = budget(1_000_000);
        let t = now_secs();
        b.record(0, t);
        assert_eq!(b.reserve(1, t), Verdict::Ok);
        // 901s later, with no fresh measurement, nothing is authorised.
        assert_eq!(b.reserve(1, t + 901), Verdict::Stale);
    }

    #[test]
    fn an_exactly_full_bucket_still_refuses_one_more_byte() {
        let b = budget(1000);
        b.record(1000, now_secs());
        assert_eq!(b.reserve(1, now_secs()), Verdict::OverBudget);
        assert_eq!(b.reserve(0, now_secs()), Verdict::Ok);
    }

    /// A verbatim response from R2 (captured from the live `pacific-media` bucket),
    /// so the parser is pinned to what the service actually emits rather than to
    /// what the S3 documentation says it should.
    const REAL_R2_PAGE: &str = r#"<?xml version="1.0" encoding="UTF-8"?><ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Name>pacific-media</Name><Contents><Key>5d17b0cddb29d850c67fee40c8a52380642d4753402403c02caef01a9f6e1724</Key><Size>4096</Size><LastModified>2026-09-09T07:32:25.202Z</LastModified><ETag>&quot;3e3f3e89c827d4c031b1837e62804d20&quot;</ETag><StorageClass>STANDARD</StorageClass></Contents><Contents><Key>e375c19e66900d400b4f344acb7a391d61f453947704ddb612b057c699a0e8ba</Key><Size>4096</Size><LastModified>2026-09-09T07:32:22.991Z</LastModified><ETag>&quot;5c7c7615bbb7acdec430799dd3990de1&quot;</ETag><StorageClass>STANDARD</StorageClass></Contents><IsTruncated>false</IsTruncated><MaxKeys>2</MaxKeys><KeyCount>2</KeyCount><EncodingType>url</EncodingType></ListBucketResult>"#;

    #[test]
    fn sums_a_real_r2_listing() {
        let page = parse_list_page(REAL_R2_PAGE).unwrap();
        assert_eq!(page.bytes, 8192);
        assert_eq!(page.next, None);
    }

    #[test]
    fn an_empty_bucket_sums_to_zero() {
        let xml = r#"<ListBucketResult><Name>b</Name><KeyCount>0</KeyCount></ListBucketResult>"#;
        assert_eq!(
            parse_list_page(xml).unwrap(),
            ListPage {
                bytes: 0,
                next: None
            }
        );
    }

    #[test]
    fn carries_the_continuation_token_when_truncated() {
        let xml = r#"<ListBucketResult><Contents><Size>10</Size></Contents><IsTruncated>true</IsTruncated><NextContinuationToken>abc/def+123=</NextContinuationToken></ListBucketResult>"#;
        let page = parse_list_page(xml).unwrap();
        assert_eq!(page.bytes, 10);
        assert_eq!(page.next.as_deref(), Some("abc/def+123="));
    }

    /// The failure that would matter: anything unparseable must NOT read as an
    /// empty bucket, because an empty bucket means "plenty of room".
    #[test]
    fn a_malformed_response_is_an_error_not_an_empty_bucket() {
        for body in [
            r#"<?xml version="1.0"?><Error><Code>AccessDenied</Code></Error>"#,
            "<html><body>502 Bad Gateway</body></html>",
            "",
            r#"<ListBucketResult><Contents><Size>notanumber</Size></Contents></ListBucketResult>"#,
            r#"<ListBucketResult><Contents><Size>4096"#,
        ] {
            assert!(parse_list_page(body).is_err(), "accepted: {body:?}");
        }
    }
}
