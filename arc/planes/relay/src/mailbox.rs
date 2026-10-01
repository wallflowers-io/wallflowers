//! mailbox — THE storage interface. The relay is written against this and nothing
//! else.
//!
//! ## What the boundary was before this module
//!
//! There was no boundary. `store::Store` — a concrete SQLite struct — was held BY
//! VALUE inside the `Hub`, and its `rusqlite::Result` reached all the way up into
//! `Hub::publish`, `Hub::subscribe` and `Hub::evict`. "Pluggable" was not a thing
//! that had been given up on; it had never been expressed. Storage was SQLite,
//! spelled out at every call site.
//!
//! [`Mailbox`] is that seam, and it is drawn so that two properties survive every
//! backend. Neither is negotiable and neither may be weakened by an implementation
//! that finds it inconvenient:
//!
//!   * **BLINDNESS.** `tag` and `body` are opaque strings going in and coming out.
//!     No method takes a decoded anything; no implementation may look inside
//!     either; no backend may derive a storage layout from what a tag MEANS. It is
//!     the relay's entire contract, and a backend is the easiest place to lose it
//!     by accident (a bucket prefix per group would do it).
//!   * **ARBITRATION.** [`Mailbox::append`] with `commit = true` is
//!     FIRST-WRITER-WINS per tag, and the claim is ATOMIC with storing the blob and
//!     advancing the durable high-water. This is not tidiness. A second device
//!     winning a commit for a group-epoch that already had one is a GROUP FORK —
//!     see `store.rs`'s module doc, which is where the cost is written out.
//!
//! ## Why the interface is async, and what that admits
//!
//! SQLite answers from a page cache a few microseconds away. An object store
//! answers over a network. Async is the honest shape for an interface that has to
//! cover both — but it also makes visible a cost the synchronous version hid:
//! `Hub::publish` and `Hub::subscribe` run while the hub mutex is held, so a
//! backend that goes to the network holds up every other connection for the
//! duration. `offload.rs` is the backend that actually pays that, and its module
//! doc says what it costs rather than leaving it to be discovered.
//!
//! ## Errors say WHICH HALF failed
//!
//! A backend that splits the index from the bodies has two ways to fail and they
//! are not the same event. [`StoreError::BodyMissing`] in particular is not "no
//! data": it is the index asserting a blob exists and the object store disagreeing,
//! which is corruption, and it must never be rounded down to an empty backlog.

use std::collections::HashMap;

use async_trait::async_trait;

/// Why a storage operation failed, in terms the relay can act on.
///
/// Every variant is a SHAPE plus a message from the layer beneath. None of them
/// may carry a tag, a body or an object key: these strings reach `tracing` (and
/// therefore Sentry) through `error!(error = %e, ...)`, and the blind-telemetry
/// invariant at the top of `lib.rs` admits no exception for an error path.
#[derive(Debug)]
pub enum StoreError {
    /// The index — rows, commit slots, retention floors, the seq high-water —
    /// could not be read or written.
    Index(String),
    /// A blob body could not be stored in, or fetched from, its backing store.
    /// Transient by assumption: a timeout, a 5xx, a broken connection.
    Body(String),
    /// The index says this blob exists and the body store does not have it.
    ///
    /// NOT the same as an empty backlog, and the difference is the whole reason
    /// this variant exists. An absent object behind a present index row means
    /// either an eviction that deleted the body before the row (which is why
    /// `offload.rs` deletes in the other order) or an out-of-band deletion in the
    /// bucket. Either way a subscriber must not be handed a short replay it cannot
    /// tell from a complete one.
    BodyMissing,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Index(m) => write!(f, "index: {m}"),
            StoreError::Body(m) => write!(f, "object storage: {m}"),
            StoreError::BodyMissing => write!(
                f,
                "object storage is missing a body the index still references \
                 (corruption, not an empty backlog)"
            ),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Index(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// What one [`Mailbox::append`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appended {
    /// Stored, and the high-water advanced.
    Stored,
    /// This exact blob is already at this tag, stored at `seq`. Nothing changed; the
    /// publisher is acked with the original `seq`, and nobody is sent it twice.
    Duplicate { seq: u64 },
    /// Commit-flagged, and the tag's slot was already taken. Nothing stored.
    SlotTaken,
}

/// The durable blind mailbox, whatever is underneath it.
///
/// Implementors: `store::Store` (SQLite, index and bodies together) and
/// `offload::OffloadMailbox` (SQLite index, bodies in S3-compatible object
/// storage). There is deliberately NO object-storage-only implementation; the
/// reason is written out in `offload.rs` and it is the commit slot.
#[async_trait]
pub trait Mailbox: Send + Sync {
    /// The durable seq high-water, or 0 on a fresh store. Seeds `Hub::seq` so the
    /// numbering is monotone across a restart even when the clock is not.
    async fn resume_seq(&self) -> Result<u64>;

    /// Store one blob under `tag` at `seq`, optionally claiming that tag's single
    /// commit slot, and advance the durable high-water.
    ///
    /// [`Appended::SlotTaken`] — storing NOTHING — iff `commit` was set and the slot
    /// was already taken; the loser must rebase. [`Appended::Duplicate`] — storing
    /// nothing — iff this exact blob is already at this tag: a republished signed
    /// blob is harmless only because it adds nothing, so ONE COPY PER (tag, blob) is
    /// a rule every backend keeps (see `pacific_wire::address`). Any other failure
    /// is an `Err`, and the caller must not Ack such a publish as ok: acking a blob
    /// that was not kept is precisely the silent loss durability exists to remove.
    ///
    /// The claim, the blob and the high-water advance either all happen or none
    /// do. A backend that cannot promise that cannot implement this trait
    /// honestly — a slot claimed for a blob that was never stored wedges that
    /// group-epoch permanently.
    async fn append(&self, tag: &str, seq: u64, body: &str, commit: bool) -> Result<Appended>;

    /// Has anything ever been stored at `tag`? What makes a publish a NEW address,
    /// which is the thing a connection's pace counts separately.
    async fn has_tag(&self, tag: &str) -> Result<bool>;

    /// Total bytes of blob bodies held, as published. What the store's fail-closed
    /// ceiling is measured against.
    async fn stored_bytes(&self) -> Result<u64>;

    /// Backlog for one tag: every stored blob with `seq > since`, in seq order.
    async fn replay(&self, tag: &str, since: u64) -> Result<Vec<(u64, String)>>;

    /// Retention floors for `tags`, omitting tags that have never evicted. An
    /// ABSENT entry is the claim "nothing was ever dropped here", which is a
    /// different statement from a floor of 0 and only the first suppresses a `Gap`.
    async fn floors(&self, tags: &[String]) -> Result<HashMap<String, u64>>;

    /// Drop blobs older than `window_us`, and trim any tag over `max_per_tag`
    /// (0 = uncapped), raising each affected tag's floor as it goes. Returns how
    /// many blobs were removed.
    async fn evict(&self, now_us: u64, window_us: u64, max_per_tag: u64) -> Result<u64>;

    /// Distinct tags and total stored blobs — two pure cardinalities for the ops
    /// stats line. No tag names, no bodies.
    async fn counts(&self) -> Result<(u64, u64)>;

    /// Per-tag blob counts. Traffic-only: this is the one read that returns tag
    /// names, and it feeds the local visualiser, never telemetry.
    async fn tag_counts(&self) -> Result<Vec<(String, u64)>>;

    /// One line naming WHICH BACKEND this is, for the boot log.
    ///
    /// Required rather than defaulted. An operator reading the log must never have
    /// to infer where the blobs went, and a backend that declines to say is exactly
    /// the one whose configuration was got wrong.
    fn describe(&self) -> String;
}
