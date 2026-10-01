//! The waker's ONE table: which device tokens want waking on which relay tags.
//!
//! WHAT THIS COSTS, stated because it is the real price of push (RINGFENCE §6, and
//! PushRegistrar.swift says the same thing on the app side): a row here binds a STABLE
//! device token to a tag. The relay's tags are opaque and rotating and link to nothing;
//! this table links a set of them to each other and to one install, across rotations.
//! Signal makes the identical trade — its server knows which account to wake, never what
//! was said. There is no push without something knowing who to poke.
//!
//! WHAT IS DELIBERATELY ABSENT: no group id, no sender, no MLS state, no blob, no
//! plaintext, no account. A row is (token, tag, platform, updated_ms) and nothing else.
//! The waker cannot decrypt anything it wakes on and does not try.
//!
//! TAG ROTATION is why `updated_ms` exists. Relay tags are per-(group, epoch), so a
//! registration goes stale the moment a group advances. The app re-registers its CURRENT
//! tag set on every sync tick (idempotent, cheap), which refreshes `updated_ms`; anything
//! not refreshed inside [`STALE_AFTER_MS`] is pruned by the watch loop and simply stops
//! being watched. That is the whole rotation story — no invalidation protocol.
//!
//! BACKEND NOTE: rusqlite at `$ARC_STATE_DIR/waker.db`, exactly like boxoffice. Nothing
//! outside this module writes SQL, so a Postgres swap is a deploy-time backend change.

use rusqlite::{params, Connection};

/// A registration that has not been refreshed in this long is dropped. The app
/// re-registers every sync tick, so a live install refreshes far inside this window; a
/// tag whose epoch rotated (or an install that stopped syncing) ages out on its own.
pub const STALE_AFTER_MS: i64 = 14 * 24 * 60 * 60 * 1000; // 14 days

/// A device to wake. `platform` is carried, not interpreted, by the store — the pusher
/// decides what it can deliver to (today: `ios` only; APNs is the only credential we hold).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Device {
    pub token: String,
    pub platform: String,
}

pub struct Store {
    // One connection behind a mutex: the register path is a low-volume request/response
    // and the watch loop's reads are small. A pool replaces this field, not the API.
    conn: std::sync::Mutex<Connection>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS registrations (
  token      TEXT NOT NULL,
  tag        TEXT NOT NULL,
  platform   TEXT NOT NULL,
  updated_ms INTEGER NOT NULL,
  PRIMARY KEY (token, tag)
);
CREATE INDEX IF NOT EXISTS registrations_by_tag ON registrations(tag);
"#;

impl Store {
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: std::sync::Mutex::new(conn),
        })
    }

    #[cfg(test)]
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: std::sync::Mutex::new(conn),
        })
    }

    /// Register (or refresh) one (token, tag) pair. Idempotent by construction — the app
    /// re-sends its whole current tag set on every sync tick and this is what makes that
    /// cheap: a repeat is one UPDATE of `updated_ms`, which is also what keeps the row
    /// alive past [`prune_stale`].
    pub fn upsert_registration(
        &self,
        token: &str,
        tag: &str,
        platform: &str,
        now_ms: i64,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("store mutex");
        conn.execute(
            "INSERT INTO registrations (token, tag, platform, updated_ms)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(token, tag) DO UPDATE SET platform = ?3, updated_ms = ?4",
            params![token, tag, platform, now_ms],
        )?;
        Ok(())
    }

    /// Who asked to be woken for this tag. The ONLY read the push path makes — it takes a
    /// tag (opaque, from the relay) and answers with tokens, never the other way round.
    pub fn tokens_for_tag(&self, tag: &str) -> rusqlite::Result<Vec<Device>> {
        let conn = self.conn.lock().expect("store mutex");
        let mut stmt = conn
            .prepare("SELECT token, platform FROM registrations WHERE tag = ?1 ORDER BY token")?;
        let rows = stmt.query_map([tag], |r| {
            Ok(Device {
                token: r.get(0)?,
                platform: r.get(1)?,
            })
        })?;
        rows.collect()
    }

    /// Every tag anyone wants watched, distinct — this IS the relay `Sub` set. Sorted so
    /// the watch loop can compare it against the last-sent set with a plain `!=`.
    pub fn tags_watched(&self) -> rusqlite::Result<Vec<String>> {
        let conn = self.conn.lock().expect("store mutex");
        let mut stmt = conn.prepare("SELECT DISTINCT tag FROM registrations ORDER BY tag")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect()
    }

    /// Drop registrations older than a cutoff (absolute ms, not a duration). The rotation
    /// garbage collector: an epoch-rotated tag stops being refreshed and leaves.
    pub fn prune_stale(&self, older_than_ms: i64) -> rusqlite::Result<usize> {
        let conn = self.conn.lock().expect("store mutex");
        conn.execute(
            "DELETE FROM registrations WHERE updated_ms < ?1",
            [older_than_ms],
        )
    }

    /// Forget a token entirely — every tag it was registered for. Two callers: the app's
    /// explicit `POST /v1/wake/forget` (sign-out, notifications off), and APNs answering
    /// 410 Unregistered, which means the token is dead and MUST NOT be retried (Apple
    /// treats continued pushes to a dead token as abuse).
    pub fn forget_token(&self, token: &str) -> rusqlite::Result<usize> {
        let conn = self.conn.lock().expect("store mutex");
        conn.execute("DELETE FROM registrations WHERE token = ?1", [token])
    }

    /// How many tags this token is now watched for — the honest number `register` returns
    /// (the total after the upsert, not the size of the request).
    pub fn tags_for_token(&self, token: &str) -> rusqlite::Result<i64> {
        let conn = self.conn.lock().expect("store mutex");
        conn.query_row(
            "SELECT COUNT(*) FROM registrations WHERE token = ?1",
            [token],
            |r| r.get(0),
        )
    }

    /// (distinct tags, distinct tokens) — the two cardinalities `/health` reports. Counts
    /// only: nothing here is routing data.
    pub fn counts(&self) -> rusqlite::Result<(i64, i64)> {
        let conn = self.conn.lock().expect("store mutex");
        conn.query_row(
            "SELECT COUNT(DISTINCT tag), COUNT(DISTINCT token) FROM registrations",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(token: &str) -> Device {
        Device {
            token: token.into(),
            platform: "ios".into(),
        }
    }

    /// The re-registration path: a repeat is not a duplicate row, it is a refresh — which
    /// is exactly what keeps a live install ahead of `prune_stale`.
    #[test]
    fn upsert_is_idempotent_and_refreshes() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_registration("tok_a", "tag1", "ios", 1_000)
            .unwrap();
        s.upsert_registration("tok_a", "tag1", "ios", 2_000)
            .unwrap();
        assert_eq!(s.tags_for_token("tok_a").unwrap(), 1, "no duplicate row");
        // The refresh really moved updated_ms: a prune at 1_500 would have taken the
        // first write and leaves the refreshed one.
        assert_eq!(s.prune_stale(1_500).unwrap(), 0);
        assert_eq!(s.tags_for_token("tok_a").unwrap(), 1);
        assert_eq!(s.prune_stale(2_500).unwrap(), 1);
        assert_eq!(s.tags_for_token("tok_a").unwrap(), 0);
    }

    /// The push path's only read: tag → tokens. Two devices in the same group share a
    /// tag and both get woken; a tag nobody registered for wakes nobody.
    #[test]
    fn tokens_for_tag() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_registration("tok_a", "tag1", "ios", 1).unwrap();
        s.upsert_registration("tok_b", "tag1", "ios", 1).unwrap();
        s.upsert_registration("tok_b", "tag2", "ios", 1).unwrap();
        assert_eq!(
            s.tokens_for_tag("tag1").unwrap(),
            vec![dev("tok_a"), dev("tok_b")]
        );
        assert_eq!(s.tokens_for_tag("tag2").unwrap(), vec![dev("tok_b")]);
        assert!(s.tokens_for_tag("tag_nobody_watches").unwrap().is_empty());
    }

    /// The `Sub` set is DISTINCT tags, sorted — two devices on one tag is one subscription,
    /// and the ordering is what lets the watch loop diff sets by equality.
    #[test]
    fn tags_watched_is_distinct_and_sorted() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_registration("tok_a", "zz", "ios", 1).unwrap();
        s.upsert_registration("tok_b", "zz", "ios", 1).unwrap();
        s.upsert_registration("tok_a", "aa", "ios", 1).unwrap();
        assert_eq!(s.tags_watched().unwrap(), vec!["aa", "zz"]);
        assert!(Store::open_in_memory()
            .unwrap()
            .tags_watched()
            .unwrap()
            .is_empty());
    }

    /// Rotation GC: a tag that stopped being refreshed leaves, and the still-fresh
    /// registrations of the SAME token survive — rotation is per-tag, not per-device.
    #[test]
    fn prune_stale_drops_only_the_unrefreshed() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_registration("tok_a", "old_epoch", "ios", 1_000)
            .unwrap();
        s.upsert_registration("tok_a", "new_epoch", "ios", 9_000)
            .unwrap();
        assert_eq!(s.prune_stale(5_000).unwrap(), 1);
        assert_eq!(s.tags_watched().unwrap(), vec!["new_epoch"]);
        assert_eq!(s.tags_for_token("tok_a").unwrap(), 1);
    }

    /// APNs 410 Unregistered (and the app's own sign-out): the token goes, everywhere.
    /// Another device on the same tag is untouched — one dead phone is not a group's
    /// worth of lost pushes.
    #[test]
    fn forget_token_removes_every_tag_for_that_token_only() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_registration("tok_dead", "tag1", "ios", 1).unwrap();
        s.upsert_registration("tok_dead", "tag2", "ios", 1).unwrap();
        s.upsert_registration("tok_live", "tag1", "ios", 1).unwrap();
        assert_eq!(s.forget_token("tok_dead").unwrap(), 2);
        assert_eq!(s.tokens_for_tag("tag1").unwrap(), vec![dev("tok_live")]);
        assert!(s.tokens_for_tag("tag2").unwrap().is_empty());
        assert_eq!(s.tags_watched().unwrap(), vec!["tag1"]);
        // Forgetting an unknown token is a no-op, never an error.
        assert_eq!(s.forget_token("tok_never_seen").unwrap(), 0);
    }

    /// The health cardinalities are DISTINCT counts, not row counts.
    #[test]
    fn counts_are_distinct() {
        let s = Store::open_in_memory().unwrap();
        assert_eq!(s.counts().unwrap(), (0, 0));
        s.upsert_registration("tok_a", "tag1", "ios", 1).unwrap();
        s.upsert_registration("tok_b", "tag1", "ios", 1).unwrap();
        s.upsert_registration("tok_a", "tag2", "ios", 1).unwrap();
        assert_eq!(s.counts().unwrap(), (2, 2));
    }

    /// Registrations survive a reopen — a waker restart must not silently stop waking
    /// every phone that registered before it (they only re-register on their own tick).
    #[test]
    fn registrations_persist_across_reopen() {
        let dir = std::env::temp_dir().join(format!("waker-store-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("waker.db");
        let p = path.to_str().unwrap();
        {
            let s = Store::open(p).unwrap();
            s.upsert_registration("tok_a", "tag1", "ios", 1).unwrap();
        }
        let s = Store::open(p).unwrap();
        assert_eq!(s.tags_watched().unwrap(), vec!["tag1"]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
