//! store — the relay's DURABLE mailbox. SQLite behind the blind interface.
//!
//! Replaces the M1 `HashMap<String, Vec<(u64, String)>>`. The relay stays blind:
//! this module stores opaque `tag`/`blob` strings and never decodes either. What
//! changes is that they survive the process.
//!
//! Since 18 Sep 2026 this is ONE BACKEND rather than the only one: the interface is
//! `mailbox::Mailbox` (implemented at the bottom of this file), and `offload.rs` is
//! a second implementation that keeps this index and puts the bodies in R2. The
//! transactional promises below are what that second backend leans on — it does not
//! reimplement them, because object storage cannot.
//!
//! ## Why this is not just "don't lose mail"
//!
//! A restart used to forget TWO things, and the second is worse than the first:
//!
//!   1. UNDRAINED BLOBS. A redeploy dropped everything not yet pulled. The
//!      publisher had already seen `Ack{ok:true}`, so the loss was silent on both
//!      sides.
//!   2. THE COMMIT SLOTS. `commit_taken` shared the store's lifetime, so after a
//!      restart the tag's single slot was free again and a SECOND device could win
//!      a commit for a group-epoch that already had one. That is not a dropped
//!      packet, it is a GROUP FORK — and the relay's first-writer-wins arbitration
//!      is the only thing standing between two devices and exactly that
//!      (SCALE-UNBLOCKS U1 TRAP 1). Durability fixes both in one move.
//!
//! ## seq IS the timestamp
//!
//! `seq` is microseconds since the Unix epoch (see `Hub::next_seq`), so retention
//! needs no second clock column: a blob is expired when `seq < now_us - window`.
//! Under a burst `seq` can run slightly ahead of the wall clock, which can only
//! make a blob live marginally LONGER than the window. Never shorter.
//!
//! ## The floor is what makes loss visible
//!
//! Every eviction raises that tag's `retention_floor` to the highest seq it
//! removed, in the SAME transaction as the delete. A subscriber arriving with a
//! cursor below the floor has provably missed something, and `Sub` says so with a
//! `Gap` instead of quietly replaying less than it should. A blind relay cannot
//! know whether anyone drained, so retention can only ever be time-based — but a
//! hole in someone's history can at least be ANNOUNCED rather than inferred.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::mailbox::{Appended, Mailbox, Result};

/// Durable mailbox. One connection behind a mutex — every caller already
/// serialises on the `Hub` mutex, so a pool would buy nothing here.
pub struct Store {
    conn: std::sync::Mutex<Connection>,
}

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous  = NORMAL;

-- The mailbox itself. (tag, seq) is unique: seq is globally monotonic, so a
-- repeat pair cannot occur except on replay of an identical write.
--
-- `digest` is SHA-256 of the blob as published and `size` its length. They exist
-- for two rules the column `body` cannot answer on the offload backend, where it
-- holds an object key: ONE COPY PER (tag, blob) — a republished signed blob must
-- add nothing — and the store's fail-closed byte ceiling. Both are NULL on rows
-- written before 18 Sep 2026; see `migrate`.
CREATE TABLE IF NOT EXISTS blob (
  tag    TEXT    NOT NULL,
  seq    INTEGER NOT NULL,
  body   TEXT    NOT NULL,
  digest BLOB,
  size   INTEGER,
  PRIMARY KEY (tag, seq)
) WITHOUT ROWID;

-- Eviction sweeps by age across all tags; replay reads one tag in seq order.
CREATE INDEX IF NOT EXISTS blob_by_seq ON blob(seq);

-- The blind sequencer's single slot per tag. PRIMARY KEY does the arbitration:
-- the second INSERT for a tag fails, and that failure IS the rejection.
CREATE TABLE IF NOT EXISTS commit_slot (
  tag TEXT    PRIMARY KEY,
  seq INTEGER NOT NULL
) WITHOUT ROWID;

-- Highest seq ever EVICTED per tag. Monotonic; only ever raised.
CREATE TABLE IF NOT EXISTS retention_floor (
  tag   TEXT    PRIMARY KEY,
  floor INTEGER NOT NULL
) WITHOUT ROWID;

-- meta['last_seq'] — the durable seq high-water. See `resume_seq`.
CREATE TABLE IF NOT EXISTS meta (
  k TEXT    PRIMARY KEY,
  v INTEGER NOT NULL
) WITHOUT ROWID;
"#;

/// Bring a store written before 18 Sep 2026 up to the schema above: add the two
/// columns if they are missing, and the index that makes (tag, digest) unique.
///
/// Old rows keep NULL in both. The unique index is PARTIAL — `digest IS NOT NULL` —
/// so they never collide with each other, and the byte total falls back to the
/// body's length for them (exact on this backend; an object key's length on the
/// offload backend, which undercounts only rows that predate the column).
fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let cols: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('blob')")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<_>>()?;
    if !cols.iter().any(|c| c == "digest") {
        conn.execute_batch("ALTER TABLE blob ADD COLUMN digest BLOB;")?;
    }
    if !cols.iter().any(|c| c == "size") {
        conn.execute_batch("ALTER TABLE blob ADD COLUMN size INTEGER;")?;
    }
    conn.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS blob_by_digest ON blob(tag, digest)
           WHERE digest IS NOT NULL;",
    )
}

/// SHA-256 of a blob as published — the identity ONE COPY PER (tag, blob) is kept by.
pub fn blob_digest(body: &str) -> [u8; 32] {
    Sha256::digest(body.as_bytes()).into()
}

impl Store {
    /// Open (creating if absent) the durable mailbox at `path`.
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn)?;
        Ok(Self { conn: std::sync::Mutex::new(conn) })
    }

    /// An ephemeral store with the identical schema. Used by the in-memory
    /// default and by tests that are not exercising restart.
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn)?;
        Ok(Self { conn: std::sync::Mutex::new(conn) })
    }

    /// The durable seq high-water, or 0 when this is a fresh store.
    ///
    /// SCALE-UNBLOCKS U1 TRAP 2: seeding `seq` from the wall clock alone is only
    /// monotone while the clock behaves. An NTP step backwards after a restart
    /// would reissue numbers already handed out, and every client holding a
    /// cursor above them goes silently deaf on that tag. Seeding from
    /// `max(clock, last_issued + 1)` removes the dependency on the clock being
    /// honest across a restart.
    pub fn resume_seq(&self) -> rusqlite::Result<u64> {
        let conn = self.conn.lock().expect("store mutex");
        let v: Option<i64> = conn
            .query_row("SELECT v FROM meta WHERE k = 'last_seq'", [], |r| r.get(0))
            .optional()?;
        Ok(v.unwrap_or(0) as u64)
    }

    /// Store one blob, optionally claiming the tag's commit slot, and advance the
    /// durable high-water — ONE transaction, so no crash can leave a slot claimed
    /// for a blob that was never stored (which would wedge that group-epoch
    /// permanently: the slot taken, the winning commit absent).
    ///
    /// The blob is what is stored and the blob is what is identified: this is the
    /// SQLite backend, where the body column holds the blob itself.
    pub fn append(&self, tag: &str, seq: u64, body: &str, commit: bool) -> rusqlite::Result<Appended> {
        self.append_row(tag, seq, body, &blob_digest(body), body.len() as u64, commit)
    }

    /// The row write both backends share. `stored` is what the body column holds —
    /// the blob here, an object key on the offload backend — while `digest` and
    /// `size` always describe the BLOB AS PUBLISHED, so one copy per (tag, blob) and
    /// the byte ceiling mean the same thing whichever backend is underneath.
    ///
    /// A duplicate is checked FIRST, before the commit slot: republishing a commit
    /// that already won must be acked as the win it was, not refused as a loser.
    pub fn append_row(
        &self,
        tag: &str,
        seq: u64,
        stored: &str,
        digest: &[u8; 32],
        size: u64,
        commit: bool,
    ) -> rusqlite::Result<Appended> {
        let mut conn = self.conn.lock().expect("store mutex");
        let tx = conn.transaction()?;
        let prior: Option<i64> = tx
            .query_row(
                "SELECT seq FROM blob WHERE tag = ?1 AND digest = ?2",
                params![tag, &digest[..]],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(prior) = prior {
            return Ok(Appended::Duplicate { seq: prior as u64 });
        }
        if commit {
            // The PRIMARY KEY is the arbitration; a conflict means we lost.
            let claimed = tx.execute(
                "INSERT INTO commit_slot (tag, seq) VALUES (?1, ?2) ON CONFLICT(tag) DO NOTHING",
                params![tag, seq as i64],
            )?;
            if claimed == 0 {
                return Ok(Appended::SlotTaken); // tx drops = rollback; nothing stored.
            }
        }
        tx.execute(
            "INSERT INTO blob (tag, seq, body, digest, size) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(tag, seq) DO NOTHING",
            params![tag, seq as i64, stored, &digest[..], size as i64],
        )?;
        tx.execute(
            "INSERT INTO meta (k, v) VALUES ('last_seq', ?1)
             ON CONFLICT(k) DO UPDATE SET v = MAX(v, excluded.v)",
            params![seq as i64],
        )?;
        tx.commit()?;
        Ok(Appended::Stored)
    }

    /// The seq this exact blob is already stored at under `tag`, if it is. Lets the
    /// offload backend answer a republish without writing the body again.
    pub fn find(&self, tag: &str, digest: &[u8; 32]) -> rusqlite::Result<Option<u64>> {
        let conn = self.conn.lock().expect("store mutex");
        let s: Option<i64> = conn
            .query_row(
                "SELECT seq FROM blob WHERE tag = ?1 AND digest = ?2",
                params![tag, &digest[..]],
                |r| r.get(0),
            )
            .optional()?;
        Ok(s.map(|s| s as u64))
    }

    /// Has anything ever been stored at `tag`?
    pub fn has_tag(&self, tag: &str) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().expect("store mutex");
        let hit: Option<i64> = conn
            .query_row("SELECT 1 FROM blob WHERE tag = ?1 LIMIT 1", [tag], |r| r.get(0))
            .optional()?;
        Ok(hit.is_some())
    }

    /// Total bytes of blobs as published. Rows from before the size column count
    /// their body's length (see [`migrate`]).
    pub fn stored_bytes(&self) -> rusqlite::Result<u64> {
        let conn = self.conn.lock().expect("store mutex");
        let n: i64 = conn.query_row(
            "SELECT COALESCE(SUM(COALESCE(size, LENGTH(body))), 0) FROM blob",
            [],
            |r| r.get(0),
        )?;
        Ok(n as u64)
    }

    /// Backlog for one tag: every stored blob with `seq > since`, in seq order.
    pub fn replay(&self, tag: &str, since: u64) -> rusqlite::Result<Vec<(u64, String)>> {
        let conn = self.conn.lock().expect("store mutex");
        let mut stmt = conn
            .prepare("SELECT seq, body FROM blob WHERE tag = ?1 AND seq > ?2 ORDER BY seq")?;
        let rows = stmt.query_map(params![tag, since as i64], |r| {
            Ok((r.get::<_, i64>(0)? as u64, r.get::<_, String>(1)?))
        })?;
        rows.collect()
    }

    /// Retention floors for `tags`, omitting tags that have never evicted.
    pub fn floors(&self, tags: &[String]) -> rusqlite::Result<HashMap<String, u64>> {
        let conn = self.conn.lock().expect("store mutex");
        let mut stmt = conn.prepare("SELECT floor FROM retention_floor WHERE tag = ?1")?;
        let mut out = HashMap::new();
        for t in tags {
            let f: Option<i64> = stmt.query_row([t], |r| r.get(0)).optional()?;
            if let Some(f) = f {
                out.insert(t.clone(), f as u64);
            }
        }
        Ok(out)
    }

    /// Drop blobs older than `window_us`, and trim any tag holding more than
    /// `max_per_tag` (0 = uncapped), raising each affected tag's floor in the same
    /// transaction. Returns how many blobs were removed.
    ///
    /// A blob that holds a commit slot is NEVER evicted, and its tag's floor never
    /// reaches it (resumption.md §7): chain entries, lease cells and every MLS commit a
    /// joining pool leaf replays are commit-flagged, and evicting one breaks recovery
    /// silently.
    ///
    /// The floor is raised to the highest seq REMOVED, never to a surviving one —
    /// a subscriber whose cursor sits exactly at the floor has missed nothing.
    pub fn evict(&self, now_us: u64, window_us: u64, max_per_tag: u64) -> rusqlite::Result<u64> {
        Ok(self.sweep(now_us, window_us, max_per_tag, false)?.0)
    }

    /// The same sweep, returning the `body` COLUMN of every row it removed.
    ///
    /// For this store the body column IS the blob, and nothing wants a sweep to
    /// materialise every evicted blob in memory — so `evict` above does not collect.
    /// The offload backend stores an object KEY in that column instead, and cannot
    /// delete the objects it just orphaned without knowing their names. One SQL
    /// body, two front doors: a second implementation of this sweep is how the two
    /// would come to disagree about what the floor means.
    pub fn evict_keys(
        &self,
        now_us: u64,
        window_us: u64,
        max_per_tag: u64,
    ) -> rusqlite::Result<Vec<String>> {
        Ok(self.sweep(now_us, window_us, max_per_tag, true)?.1)
    }

    /// `collect` decides only whether the removed bodies are read back; the floors,
    /// the deletes and the transaction are identical either way.
    fn sweep(
        &self,
        now_us: u64,
        window_us: u64,
        max_per_tag: u64,
        collect: bool,
    ) -> rusqlite::Result<(u64, Vec<String>)> {
        let mut conn = self.conn.lock().expect("store mutex");
        let tx = conn.transaction()?;
        let mut removed = 0u64;
        let mut bodies: Vec<String> = Vec::new();

        if window_us > 0 {
            let cutoff = now_us.saturating_sub(window_us) as i64;
            // Record the floor BEFORE deleting: once the rows are gone we cannot
            // know what the highest removed seq was.
            tx.execute(
                "INSERT INTO retention_floor (tag, floor)
                 SELECT tag, MAX(seq) FROM blob WHERE seq <= ?1 AND NOT (EXISTS (SELECT 1 FROM commit_slot c WHERE c.tag = blob.tag AND c.seq = blob.seq)) GROUP BY tag
                 ON CONFLICT(tag) DO UPDATE SET floor = MAX(floor, excluded.floor)",
                params![cutoff],
            )?;
            if collect {
                // Inside the transaction, so what is collected is exactly what the
                // DELETE on the next line removes.
                let mut stmt = tx.prepare(
                    "SELECT body FROM blob WHERE seq <= ?1 AND NOT EXISTS
                       (SELECT 1 FROM commit_slot c WHERE c.tag = blob.tag AND c.seq = blob.seq)",
                )?;
                let rows = stmt.query_map(params![cutoff], |r| r.get::<_, String>(0))?;
                for b in rows {
                    bodies.push(b?);
                }
            }
            removed += tx.execute(
                "DELETE FROM blob WHERE seq <= ?1 AND NOT EXISTS
                   (SELECT 1 FROM commit_slot c WHERE c.tag = blob.tag AND c.seq = blob.seq)",
                params![cutoff],
            )? as u64;
        }

        if max_per_tag > 0 {
            // Per tag, everything below the newest `max_per_tag` rows goes.
            tx.execute(
                "INSERT INTO retention_floor (tag, floor)
                 SELECT tag, MAX(seq) FROM (
                   SELECT tag, seq, ROW_NUMBER() OVER (PARTITION BY tag ORDER BY seq DESC) AS rn
                   FROM blob WHERE NOT EXISTS
                     (SELECT 1 FROM commit_slot c WHERE c.tag = blob.tag AND c.seq = blob.seq)
                 ) WHERE rn > ?1 GROUP BY tag
                 ON CONFLICT(tag) DO UPDATE SET floor = MAX(floor, excluded.floor)",
                params![max_per_tag as i64],
            )?;
            if collect {
                let mut stmt = tx.prepare(
                    "SELECT body FROM (
                       SELECT body, ROW_NUMBER() OVER (PARTITION BY tag ORDER BY seq DESC) AS rn
                       FROM blob WHERE NOT EXISTS
                         (SELECT 1 FROM commit_slot c WHERE c.tag = blob.tag AND c.seq = blob.seq)
                     ) WHERE rn > ?1",
                )?;
                let rows = stmt.query_map(params![max_per_tag as i64], |r| r.get::<_, String>(0))?;
                for b in rows {
                    bodies.push(b?);
                }
            }
            removed += tx.execute(
                "DELETE FROM blob WHERE (tag, seq) IN (
                   SELECT tag, seq FROM (
                     SELECT tag, seq, ROW_NUMBER() OVER (PARTITION BY tag ORDER BY seq DESC) AS rn
                     FROM blob WHERE NOT EXISTS
                       (SELECT 1 FROM commit_slot c WHERE c.tag = blob.tag AND c.seq = blob.seq)
                   ) WHERE rn > ?1
                 )",
                params![max_per_tag as i64],
            )? as u64;
        }

        // A COMMIT IS NEVER BEHIND THE FLOOR. A floor at or past a surviving commit
        // would tell a subscriber below it that it had missed something it can still
        // replay — and the commit is the one blob a joining pool leaf must replay.
        tx.execute(
            "UPDATE retention_floor SET floor = (
               SELECT MIN(retention_floor.floor, c.seq - 1) FROM commit_slot c
               WHERE c.tag = retention_floor.tag)
             WHERE EXISTS (SELECT 1 FROM commit_slot c
                           WHERE c.tag = retention_floor.tag AND c.seq <= retention_floor.floor)",
            [],
        )?;

        tx.commit()?;
        Ok((removed, bodies))
    }

    /// Distinct tags and total stored blobs — two pure cardinalities for the ops
    /// stats line. No tag names, no bodies.
    pub fn counts(&self) -> rusqlite::Result<(u64, u64)> {
        let conn = self.conn.lock().expect("store mutex");
        let tags: i64 =
            conn.query_row("SELECT COUNT(DISTINCT tag) FROM blob", [], |r| r.get(0))?;
        let blobs: i64 = conn.query_row("SELECT COUNT(*) FROM blob", [], |r| r.get(0))?;
        Ok((tags as u64, blobs as u64))
    }

    /// Per-tag blob counts. Traffic-only — this is the one read that returns tag
    /// names, and it feeds the local visualiser, never telemetry.
    pub fn tag_counts(&self) -> rusqlite::Result<Vec<(String, u64)>> {
        let conn = self.conn.lock().expect("store mutex");
        let mut stmt = conn.prepare("SELECT tag, COUNT(*) FROM blob GROUP BY tag")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64)))?;
        rows.collect()
    }
}

/// The SQLite backend of [`Mailbox`] — index and bodies in one file, one
/// transaction, one fsync. This is the DEFAULT and it is the one that needs no
/// argument: every property the trait asks for is a local invariant here, not a
/// promise made by a service on the other side of a network.
///
/// The methods are `async` because the trait is (an object store is I/O; see
/// `mailbox.rs`). Nothing here awaits — SQLite answers from the page cache — so
/// these are ordinary blocking calls wearing an async signature, exactly as they
/// were before the seam existed.
#[async_trait::async_trait]
impl Mailbox for Store {
    async fn resume_seq(&self) -> Result<u64> {
        Ok(Store::resume_seq(self)?)
    }

    async fn append(&self, tag: &str, seq: u64, body: &str, commit: bool) -> Result<Appended> {
        Ok(Store::append(self, tag, seq, body, commit)?)
    }

    async fn has_tag(&self, tag: &str) -> Result<bool> {
        Ok(Store::has_tag(self, tag)?)
    }

    async fn stored_bytes(&self) -> Result<u64> {
        Ok(Store::stored_bytes(self)?)
    }

    async fn replay(&self, tag: &str, since: u64) -> Result<Vec<(u64, String)>> {
        Ok(Store::replay(self, tag, since)?)
    }

    async fn floors(&self, tags: &[String]) -> Result<HashMap<String, u64>> {
        Ok(Store::floors(self, tags)?)
    }

    async fn evict(&self, now_us: u64, window_us: u64, max_per_tag: u64) -> Result<u64> {
        Ok(Store::evict(self, now_us, window_us, max_per_tag)?)
    }

    async fn counts(&self) -> Result<(u64, u64)> {
        Ok(Store::counts(self)?)
    }

    async fn tag_counts(&self) -> Result<Vec<(String, u64)>> {
        Ok(Store::tag_counts(self)?)
    }

    fn describe(&self) -> String {
        "SQLite (index and bodies in one file)".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store where `now` is an argument, so the age window can be tested as an
    /// assertion rather than a sleep.
    fn store() -> Store {
        Store::open_in_memory().expect("store")
    }

    #[test]
    fn age_eviction_raises_the_floor_to_the_highest_seq_removed() {
        let s = store();
        s.append("aa", 100, "a", false).unwrap();
        s.append("aa", 200, "b", false).unwrap();
        s.append("aa", 300, "c", false).unwrap();

        // now = 350, window = 100 ⇒ cutoff 250: seqs 100 and 200 go, 300 stays.
        assert_eq!(s.evict(350, 100, 0).unwrap(), 2);
        assert_eq!(s.replay("aa", 0).unwrap(), vec![(300, "c".to_string())]);

        // The floor is the highest seq REMOVED (200), never the lowest surviving
        // one (300). A subscriber whose cursor is exactly 200 has missed nothing,
        // and must not be told otherwise.
        assert_eq!(s.floors(&["aa".into()]).unwrap().get("aa"), Some(&200));
    }

    #[test]
    fn the_floor_only_ever_rises() {
        let s = store();
        s.append("aa", 100, "a", false).unwrap();
        s.evict(150, 10, 0).unwrap();
        assert_eq!(s.floors(&["aa".into()]).unwrap()["aa"], 100);

        // A later sweep that removes nothing must not lower or clear the floor —
        // the hole it describes is permanent.
        s.append("aa", 400, "b", false).unwrap();
        s.evict(401, 100_000, 0).unwrap();
        assert_eq!(s.floors(&["aa".into()]).unwrap()["aa"], 100);
    }

    #[test]
    fn a_tag_that_never_evicted_has_no_floor() {
        let s = store();
        s.append("aa", 100, "a", false).unwrap();
        s.evict(101, 1_000_000, 0).unwrap();
        // Absent, not zero: "nothing was ever dropped here" is a different claim
        // from "the floor is 0", and only the first one suppresses a Gap.
        assert!(s.floors(&["aa".into()]).unwrap().is_empty());
    }

    #[test]
    fn the_cap_keeps_the_newest_and_floors_the_rest() {
        let s = store();
        for seq in [10, 20, 30, 40] {
            s.append("aa", seq, &format!("x{seq}"), false).unwrap();
        }
        assert_eq!(s.evict(0, 0, 2).unwrap(), 2);
        let kept: Vec<u64> = s.replay("aa", 0).unwrap().into_iter().map(|(q, _)| q).collect();
        assert_eq!(kept, vec![30, 40]);
        assert_eq!(s.floors(&["aa".into()]).unwrap()["aa"], 20);
    }

    #[test]
    fn eviction_is_per_tag() {
        let s = store();
        s.append("aa", 10, "x", false).unwrap();
        s.append("bb", 10, "y", false).unwrap();
        s.append("bb", 20, "y2", false).unwrap();
        assert_eq!(s.evict(0, 0, 1).unwrap(), 1, "only bb is over the cap");
        assert_eq!(s.replay("aa", 0).unwrap().len(), 1);
        let floors = s.floors(&["aa".into(), "bb".into()]).unwrap();
        assert!(!floors.contains_key("aa"));
        assert_eq!(floors["bb"], 10);
    }

    /// resumption.md §7: a window that covers a commit evicts the application blobs
    /// of the same age and spares the commit, its slot, and the floor below it.
    #[test]
    fn eviction_never_removes_a_commit_slot_blob() {
        let s = store();
        s.append("aa", 10, "app-early", false).unwrap();
        s.append("aa", 20, "the-commit", true).unwrap();
        s.append("aa", 30, "app-late", false).unwrap();
        s.append("bb", 15, "other", false).unwrap();
        assert_eq!(s.evict(1_000, 100, 0).unwrap(), 3, "every application blob went");
        assert_eq!(s.replay("aa", 0).unwrap(), vec![(20, "the-commit".to_string())]);
        assert!(s.replay("bb", 0).unwrap().is_empty());
        assert_eq!(
            s.append("aa", 40, "a-second-commit", true).unwrap(),
            Appended::SlotTaken,
            "the slot is still held"
        );
        let floors = s.floors(&["aa".into(), "bb".into()]).unwrap();
        assert!(floors["aa"] < 20, "a replayable commit is never behind the floor: {}", floors["aa"]);
        assert_eq!(floors["bb"], 15);
    }

    #[test]
    fn the_cap_spares_a_commit_slot_blob() {
        let s = store();
        s.append("aa", 10, "the-commit", true).unwrap();
        for seq in [20, 30, 40] {
            s.append("aa", seq, &format!("x{seq}"), false).unwrap();
        }
        assert_eq!(s.evict(0, 0, 1).unwrap(), 2);
        let kept: Vec<u64> = s.replay("aa", 0).unwrap().into_iter().map(|(q, _)| q).collect();
        assert_eq!(kept, vec![10, 40], "the commit and the newest application blob");
    }

    #[test]
    fn the_commit_slot_is_one_per_tag_and_the_loser_stores_nothing() {
        let s = store();
        assert_eq!(s.append("aa", 10, "winner", true).unwrap(), Appended::Stored);
        assert_eq!(s.append("aa", 20, "loser", true).unwrap(), Appended::SlotTaken);

        // The loser's blob must not be in the mailbox: a rejected commit that was
        // still stored would be replayed to members as if it had been sequenced.
        assert_eq!(s.replay("aa", 0).unwrap(), vec![(10, "winner".to_string())]);
    }

    #[test]
    fn a_lost_commit_race_does_not_advance_the_seq_high_water() {
        let s = store();
        s.append("aa", 10, "winner", true).unwrap();
        s.append("aa", 20, "loser", true).unwrap();
        // The rollback covers the meta row too — the whole append is one
        // transaction, so a rejected commit leaves no trace at all.
        assert_eq!(s.resume_seq().unwrap(), 10);
    }

    #[test]
    fn plain_publishes_to_a_claimed_tag_still_land() {
        let s = store();
        assert_eq!(s.append("aa", 10, "commit", true).unwrap(), Appended::Stored);
        // The slot gates COMMITS, not the tag. Ordinary traffic on a group-epoch
        // tag continues after its commit has been sequenced.
        assert_eq!(s.append("aa", 20, "chatter", false).unwrap(), Appended::Stored);
        assert_eq!(s.replay("aa", 10).unwrap(), vec![(20, "chatter".to_string())]);
    }

    /// ONE COPY PER (tag, blob). A republished signed blob is harmless only because
    /// it adds nothing: it is acked with the seq it already has, and not stored again.
    #[test]
    fn the_same_blob_at_the_same_tag_is_stored_once() {
        let s = store();
        assert_eq!(s.append("aa", 10, "sealed", false).unwrap(), Appended::Stored);
        assert_eq!(s.append("aa", 20, "sealed", false).unwrap(), Appended::Duplicate { seq: 10 });
        assert_eq!(s.replay("aa", 0).unwrap(), vec![(10, "sealed".to_string())]);
        assert_eq!(s.resume_seq().unwrap(), 10, "a duplicate leaves no trace, not even in the high-water");
        // The same blob at ANOTHER tag is another write.
        assert_eq!(s.append("bb", 30, "sealed", false).unwrap(), Appended::Stored);
    }

    /// A retried commit that already WON is a duplicate, not a lost race: the
    /// duplicate check runs before the slot, so the winner is not told it lost.
    #[test]
    fn a_retried_winning_commit_is_acked_as_the_win_it_was() {
        let s = store();
        assert_eq!(s.append("aa", 10, "commit", true).unwrap(), Appended::Stored);
        assert_eq!(s.append("aa", 20, "commit", true).unwrap(), Appended::Duplicate { seq: 10 });
        assert_eq!(s.append("aa", 30, "rival", true).unwrap(), Appended::SlotTaken);
    }

    #[test]
    fn bytes_and_addresses_are_accounted() {
        let s = store();
        assert!(!s.has_tag("aa").unwrap());
        assert_eq!(s.stored_bytes().unwrap(), 0);
        s.append("aa", 10, "12345", false).unwrap();
        s.append("aa", 20, "123", false).unwrap();
        s.append("aa", 30, "123", false).unwrap(); // a duplicate adds no bytes
        assert!(s.has_tag("aa").unwrap());
        assert!(!s.has_tag("bb").unwrap());
        assert_eq!(s.stored_bytes().unwrap(), 8);
    }

    /// A store written before the digest and size columns existed opens, keeps its
    /// rows, and applies the new rules to everything written after.
    #[test]
    fn a_store_from_before_the_columns_is_migrated_in_place() {
        let path = std::env::temp_dir().join(format!("relay-migrate-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE blob (tag TEXT NOT NULL, seq INTEGER NOT NULL, body TEXT NOT NULL,
                   PRIMARY KEY (tag, seq)) WITHOUT ROWID;
                 INSERT INTO blob VALUES ('aa', 5, 'old');",
            )
            .unwrap();
        }
        let s = Store::open(path.to_str().unwrap()).unwrap();
        assert_eq!(s.replay("aa", 0).unwrap(), vec![(5, "old".to_string())], "old rows survive");
        assert_eq!(s.stored_bytes().unwrap(), 3, "an old row counts its body's length");
        assert_eq!(s.append("aa", 10, "new", false).unwrap(), Appended::Stored);
        assert_eq!(s.append("aa", 20, "new", false).unwrap(), Appended::Duplicate { seq: 10 });
        drop(s);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn replay_is_exclusive_of_the_cursor() {
        let s = store();
        s.append("aa", 10, "a", false).unwrap();
        s.append("aa", 20, "b", false).unwrap();
        // `since` is the last seq the client HAS — replaying it again would
        // duplicate, and the client cursor never rewinds to catch it.
        assert_eq!(s.replay("aa", 10).unwrap(), vec![(20, "b".to_string())]);
        assert!(s.replay("aa", 20).unwrap().is_empty());
    }

    #[test]
    fn the_seq_high_water_is_durable_and_monotonic() {
        let s = store();
        assert_eq!(s.resume_seq().unwrap(), 0, "a fresh store resumes from nothing");
        s.append("aa", 500, "a", false).unwrap();
        assert_eq!(s.resume_seq().unwrap(), 500);
        // An out-of-order write must not drag the high-water backwards: it is what
        // a restart seeds from, and seeding low reissues numbers clients have
        // already passed, which makes them silently deaf on that tag.
        s.append("bb", 400, "b", false).unwrap();
        assert_eq!(s.resume_seq().unwrap(), 500);
    }

    #[test]
    fn counts_are_cardinalities_only() {
        let s = store();
        s.append("aa", 10, "x", false).unwrap();
        s.append("aa", 20, "y", false).unwrap();
        s.append("bb", 30, "x", false).unwrap();
        assert_eq!(s.counts().unwrap(), (2, 3));
        let mut per_tag = s.tag_counts().unwrap();
        per_tag.sort();
        assert_eq!(per_tag, vec![("aa".to_string(), 2), ("bb".to_string(), 1)]);
    }
}
