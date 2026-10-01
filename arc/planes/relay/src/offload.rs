//! offload — the R2 backend: SQLite holds the index, R2 holds the bodies.
//!
//! # WHY THERE IS NO PURE-R2 MAILBOX, AND THERE WILL NOT BE ONE
//!
//! The ruling is that storage is pluggable and the arc's job is to author blobs to
//! R2. This module does that. What it does NOT do is move the whole mailbox into
//! object storage, and the reason is one property, not a preference:
//!
//! > `append(tag, seq, body, commit = true)` must CLAIM THE TAG'S SINGLE SLOT,
//! > STORE THE BLOB and ADVANCE THE DURABLE HIGH-WATER as one indivisible act.
//!
//! In SQLite that is four lines and a `tx.commit()`. In object storage it is three
//! objects and no transaction, and every way of faking it fails somewhere that
//! matters:
//!
//!   * **Claim the slot, then write the body.** A crash between them leaves a
//!     group-epoch whose slot is taken by a commit that does not exist. Nobody can
//!     ever claim it again, so the group cannot advance an epoch — permanently, and
//!     without a single error anywhere. `store.rs` names this exact hazard.
//!   * **Write the body, then claim the slot.** Now the atomicity is gone in the
//!     other direction: a reader that lists the bucket between the two sees a
//!     sequenced-looking blob for a commit that lost, or has not won yet.
//!   * **Make the body BE the claim** (one conditional PUT, `If-None-Match: *`).
//!     This is the closest object storage gets, and it still does not close: the
//!     high-water is a separate object, and a conditional PUT is per-object — it
//!     cannot also express "and only if this tag has no other commit at this
//!     epoch". It would also make the arbitration depend on a conditional-write
//!     semantic that is a vendor guarantee rather than a local invariant, on the
//!     one code path where being wrong means a GROUP FORK rather than a retry.
//!
//! **So the arbitration stays in SQLite. That is the design, stated rather than
//! discovered.** An object store is a good place to put bytes and a bad place to
//! put a decision, and the commit slot is a decision.
//!
//! ## The other four things R2 will not do, listed so nobody re-derives them
//!
//!   1. **No multi-object transaction.** Eviction raises a tag's retention floor in
//!      the SAME transaction as the delete — that is what makes a hole announceable
//!      instead of inferable. Two objects cannot do it.
//!   2. **A round trip per body.** `replay` becomes N GETs. Batched at
//!      [`REPLAY_FETCH_CONCURRENCY`] below, but still N, and the hub mutex is held
//!      while they run: a big drain on this backend stalls every other connection.
//!      THIS IS THE REAL COST OF THIS BACKEND and it is why SQLite remains the
//!      default.
//!   3. **No cheap cardinality.** `counts()` and `tag_counts()` are `COUNT(*)` in
//!      SQLite and a full bucket walk on R2 — `budget.rs` already concedes this,
//!      capping its own walk at `MAX_LIST_PAGES` and rounding to "full" when it
//!      hits the cap. A stats line must not cost a hundred Class A operations.
//!   4. **No monotonic counter.** The seq high-water is a single mutable value;
//!      last-writer-wins on an object can move it BACKWARDS, and a high-water that
//!      goes backwards reissues numbers clients have already passed, which makes
//!      every one of them silently deaf on that tag (`Hub::next_seq`'s essay).
//!
//! All four are index concerns, and all four stay in SQLite. What goes to R2 is the
//! one thing R2 is good at: opaque bytes, addressed by name.
//!
//! ## The ordering rule, in one sentence
//!
//! **An object with no index row is garbage; an index row with no object is
//! corruption.** So a write goes body-first and a delete goes index-first, and
//! every failure lands on the garbage side of that line:
//!
//! | | order | what a crash in the middle leaves |
//! |---|---|---|
//! | append | R2 PUT, then index tx | an unreferenced object (garbage) |
//! | evict  | index tx, then R2 DELETE | an unreferenced object (garbage) |
//!
//! The publish is Acked only after the index transaction commits, so a body that
//! did not reach R2 is never acked — it returns `Err`, the client sees a transport
//! error and retries, and nothing is lost in silence.
//!
//! ## The cost guard does NOT gate this, on purpose
//!
//! `budget.rs` refuses a media presign when the bucket is near its cap, and its own
//! doc states the trade: "media stops, the mailbox is untouched". That stays true
//! here and it is the reason mailbox bodies are not budget-gated — refusing to
//! store a blob on a cost budget is not thrift, it is refusing mail, and the
//! publish would go un-acked while the client retried forever.
//!
//! The consequence, which is real and must not be discovered in production: mailbox
//! bodies land in the SAME bucket the meter walks, so they DO consume the budget
//! that media is gated on. A busy mailbox on this backend is a plausible reason for
//! photo uploads to start being refused. If that becomes the live failure, the
//! answer is a second bucket, not a gate on the mailbox.
//!
//! ## What is still true, and what is not
//!
//! This backend keeps every property `store.rs` documents, because the index is
//! still `store.rs`. What it changes is where the bytes rest, what a replay costs,
//! and the existence of one new failure mode — [`StoreError::BodyMissing`], an
//! index row whose object is gone — which is reported as an error and never as an
//! empty backlog.

use std::collections::HashMap;

use async_trait::async_trait;
use tracing::{error, warn};

use crate::blobs::{body_key, Blobs};
use crate::mailbox::{Appended, Mailbox, Result, StoreError};
use crate::store::{blob_digest, Store};

/// How many bodies to fetch at once during a replay.
///
/// One at a time turns a 500-blob backlog into 500 serial round trips under the hub
/// mutex, which is minutes. Unbounded opens a connection per blob at a store that
/// rate-limits. Sixteen is a floor-of-the-range choice, not a measured optimum, and
/// it is called out as such rather than dressed up: the honest fix for a large
/// drain on this backend is not to use this backend.
const REPLAY_FETCH_CONCURRENCY: usize = 16;

/// SQLite index + object-storage bodies. The index row's `body` column holds the
/// object KEY, not the blob — the schema is untouched, only the meaning of that one
/// column changes, and it changes only for this backend.
pub struct OffloadMailbox {
    index: Store,
    blobs: Box<dyn Blobs>,
}

impl OffloadMailbox {
    /// `index` MUST be a file-backed [`Store`]. An in-memory index would put the
    /// commit slots back in the process's lifetime, which is the exact defect
    /// durability was added to fix — and it would be invisible, because the bodies
    /// would still be durable in R2. `lib.rs` is where that is refused; this
    /// constructor cannot tell the two apart.
    pub fn new(index: Store, blobs: Box<dyn Blobs>) -> Self {
        Self { index, blobs }
    }

    /// Fetch one body, mapping "the store does not have it" to `BodyMissing` rather
    /// than to an absence the caller could round down to "no backlog".
    async fn body(&self, key: &str) -> Result<String> {
        match self.blobs.get(key).await {
            Ok(Some(bytes)) => String::from_utf8(bytes).map_err(|_| {
                StoreError::Body("stored body was not valid UTF-8".into())
            }),
            Ok(None) => Err(StoreError::BodyMissing),
            Err(e) => Err(StoreError::Body(e)),
        }
    }
}

#[async_trait]
impl Mailbox for OffloadMailbox {
    async fn resume_seq(&self) -> Result<u64> {
        Ok(self.index.resume_seq()?)
    }

    async fn append(&self, tag: &str, seq: u64, body: &str, commit: bool) -> Result<Appended> {
        let digest = blob_digest(body);
        // A republish of a blob already here adds nothing, and on this backend it
        // must not even cost an object write: answer it from the index first.
        if let Some(prior) = self.index.find(tag, &digest)? {
            return Ok(Appended::Duplicate { seq: prior });
        }
        let key = body_key(tag, seq);
        // BODY FIRST. If this fails the caller gets an Err and does not Ack, and
        // the index never learns the blob existed. The other order would ack a
        // publish whose body is not there — a row that replays as nothing.
        self.blobs
            .put(&key, body.as_bytes())
            .await
            .map_err(StoreError::Body)?;
        // Then the one transaction that actually decides anything: the duplicate
        // check again (a concurrent republish), the slot claim, the row and the
        // high-water, atomically, in SQLite. The row records the digest and size of
        // the blob as published, not of the key it holds.
        match self.index.append_row(tag, seq, &key, &digest, body.len() as u64, commit) {
            Ok(Appended::Stored) => Ok(Appended::Stored),
            Ok(other) => {
                // A lost commit race, or a duplicate that landed between the check
                // above and here. Nothing references the object we just wrote, so
                // it is garbage by the rule above — remove it while we still know
                // its name. A failure here is a leak, not a correctness problem,
                // so it is warned about and swallowed.
                if let Err(e) = self.blobs.delete(&key).await {
                    warn!(
                        error = %e,
                        "a refused append left an unreferenced object in the bucket"
                    );
                }
                Ok(other)
            }
            Err(e) => {
                if let Err(e2) = self.blobs.delete(&key).await {
                    warn!(error = %e2, "an index failure left an unreferenced object in the bucket");
                }
                Err(e.into())
            }
        }
    }

    async fn has_tag(&self, tag: &str) -> Result<bool> {
        Ok(self.index.has_tag(tag)?)
    }

    async fn stored_bytes(&self) -> Result<u64> {
        Ok(self.index.stored_bytes()?)
    }

    async fn replay(&self, tag: &str, since: u64) -> Result<Vec<(u64, String)>> {
        let rows = self.index.replay(tag, since)?;
        let mut out = Vec::with_capacity(rows.len());
        for chunk in rows.chunks(REPLAY_FETCH_CONCURRENCY) {
            // `try_join_all` preserves input order, and the chunks are walked in
            // order, so the seq ordering the index established survives the fetch.
            let fetched = futures_util::future::try_join_all(chunk.iter().map(|(seq, key)| {
                let seq = *seq;
                async move { self.body(key).await.map(|b| (seq, b)) }
            }))
            .await?;
            out.extend(fetched);
        }
        Ok(out)
    }

    async fn floors(&self, tags: &[String]) -> Result<HashMap<String, u64>> {
        Ok(self.index.floors(tags)?)
    }

    async fn evict(&self, now_us: u64, window_us: u64, max_per_tag: u64) -> Result<u64> {
        // INDEX FIRST on the way out — the mirror of append. Once the rows are gone
        // the objects are unreferenced, so a failure below leaks bytes rather than
        // punching a hole in someone's backlog.
        let keys = self.index.evict_keys(now_us, window_us, max_per_tag)?;
        let removed = keys.len() as u64;
        let mut leaked = 0u64;
        for key in &keys {
            if self.blobs.delete(key).await.is_err() {
                leaked += 1;
            }
        }
        if leaked > 0 {
            // A count, never a key. Persistently non-zero means the bucket is
            // growing with objects nothing will ever read again.
            error!(
                leaked,
                "eviction dropped index rows whose objects could not be deleted — \
                 they are now unreferenced garbage in the bucket"
            );
        }
        Ok(removed)
    }

    async fn counts(&self) -> Result<(u64, u64)> {
        Ok(self.index.counts()?)
    }

    async fn tag_counts(&self) -> Result<Vec<(String, u64)>> {
        Ok(self.index.tag_counts()?)
    }

    fn describe(&self) -> String {
        format!(
            "index in SQLite (commit slots, floors, seq high-water), bodies in {}",
            self.blobs.describe()
        )
    }
}
