//! The on-device directory: one SQLite database holding identity, peers,
//! connections, the delta log, relay cursors, and pending intro mailboxes.
//!
//! MLS group state does NOT live here — it lives in the dedicated `mls_store`
//! tables (a real `GroupStateStorage` + `KeyPackageStorage`) in the same DB, so
//! group continuity across the per-command CLI is `client.load_group(group_id)`,
//! not a snapshot blob. The `groups` table here records only the directory facts
//! (group_id, kind), never the MLS secrets.

use rusqlite::{params, Connection, OptionalExtension};
use zeroize::Zeroizing;

use crate::{paths, CoreError};

/// (author_pk, envelope) rows from the delta log.
type AuthoredLog = Vec<([u8; 32], Vec<u8>)>;
/// (epoch, shared group tag, seal secret) for one epoch of one group. The secret
/// is `Zeroizing` because it is live key material: holding it plus the adjacent
/// plaintext `tag` strips the metadata seal off any captured relay blob for that
/// group/epoch.
type EpochTag = (u64, [u8; 32], Zeroizing<[u8; 32]>);

/// Pairing/connection lifecycle. The double-opt-in gate is `Connected`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerStatus {
    Contact,
    PendingIn,
    PendingOut,
    Connected,
    Revoked,
}

impl PeerStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            PeerStatus::Contact => "contact",
            PeerStatus::PendingIn => "pending_in",
            PeerStatus::PendingOut => "pending_out",
            PeerStatus::Connected => "connected",
            PeerStatus::Revoked => "revoked",
        }
    }
    pub fn parse(s: &str) -> Result<Self, CoreError> {
        Ok(match s {
            "contact" => Self::Contact,
            "pending_in" => Self::PendingIn,
            "pending_out" => Self::PendingOut,
            "connected" => Self::Connected,
            "revoked" => Self::Revoked,
            other => return Err(CoreError::Directory(format!("bad peer status '{other}'"))),
        })
    }
}

/// THE FOLD GENERATION (O-69, mdr/icd-pin.md § O-69): one integer per object, raised by
/// triggers inside the writing transaction whenever a row the fold reads changes (the
/// object's log, its roster, its owner history, its owner). A cache keyed on it cannot miss
/// a write path, including ones not yet written. A count of changes, never read as state.
/// `delta_log`'s `delivered` flag is left out: the relay's acknowledgement changes nothing
/// a fold reads. The triggers are dropped and made again at every open, so a changed body
/// reaches a database made before it.
const FOLD_GEN: &str = r#"
CREATE TABLE IF NOT EXISTS fold_gen (
  group_id BLOB PRIMARY KEY,
  gen      INTEGER NOT NULL
);
DROP TRIGGER IF EXISTS fold_gen_log_ins;
DROP TRIGGER IF EXISTS fold_gen_log_upd;
DROP TRIGGER IF EXISTS fold_gen_log_del;
DROP TRIGGER IF EXISTS fold_gen_members_ins;
DROP TRIGGER IF EXISTS fold_gen_members_upd;
DROP TRIGGER IF EXISTS fold_gen_members_del;
DROP TRIGGER IF EXISTS fold_gen_owners_ins;
DROP TRIGGER IF EXISTS fold_gen_owners_upd;
DROP TRIGGER IF EXISTS fold_gen_owners_del;
DROP TRIGGER IF EXISTS fold_gen_owner_upd;
CREATE TRIGGER fold_gen_log_ins AFTER INSERT ON delta_log BEGIN
  INSERT INTO fold_gen (group_id, gen) VALUES (NEW.group_id, 1)
    ON CONFLICT(group_id) DO UPDATE SET gen = gen + 1;
END;
CREATE TRIGGER fold_gen_log_upd AFTER UPDATE OF group_id, delta_id, author_pk, envelope, author_sig ON delta_log BEGIN
  INSERT INTO fold_gen (group_id, gen) VALUES (NEW.group_id, 1)
    ON CONFLICT(group_id) DO UPDATE SET gen = gen + 1;
  INSERT INTO fold_gen (group_id, gen) VALUES (OLD.group_id, 1)
    ON CONFLICT(group_id) DO UPDATE SET gen = gen + 1;
END;
CREATE TRIGGER fold_gen_log_del AFTER DELETE ON delta_log BEGIN
  INSERT INTO fold_gen (group_id, gen) VALUES (OLD.group_id, 1)
    ON CONFLICT(group_id) DO UPDATE SET gen = gen + 1;
END;
CREATE TRIGGER fold_gen_members_ins AFTER INSERT ON group_members BEGIN
  INSERT INTO fold_gen (group_id, gen) VALUES (NEW.group_id, 1)
    ON CONFLICT(group_id) DO UPDATE SET gen = gen + 1;
END;
CREATE TRIGGER fold_gen_members_upd AFTER UPDATE ON group_members BEGIN
  INSERT INTO fold_gen (group_id, gen) VALUES (NEW.group_id, 1)
    ON CONFLICT(group_id) DO UPDATE SET gen = gen + 1;
  INSERT INTO fold_gen (group_id, gen) VALUES (OLD.group_id, 1)
    ON CONFLICT(group_id) DO UPDATE SET gen = gen + 1;
END;
CREATE TRIGGER fold_gen_members_del AFTER DELETE ON group_members BEGIN
  INSERT INTO fold_gen (group_id, gen) VALUES (OLD.group_id, 1)
    ON CONFLICT(group_id) DO UPDATE SET gen = gen + 1;
END;
CREATE TRIGGER fold_gen_owners_ins AFTER INSERT ON group_owner_history BEGIN
  INSERT INTO fold_gen (group_id, gen) VALUES (NEW.group_id, 1)
    ON CONFLICT(group_id) DO UPDATE SET gen = gen + 1;
END;
CREATE TRIGGER fold_gen_owners_upd AFTER UPDATE ON group_owner_history BEGIN
  INSERT INTO fold_gen (group_id, gen) VALUES (NEW.group_id, 1)
    ON CONFLICT(group_id) DO UPDATE SET gen = gen + 1;
  INSERT INTO fold_gen (group_id, gen) VALUES (OLD.group_id, 1)
    ON CONFLICT(group_id) DO UPDATE SET gen = gen + 1;
END;
CREATE TRIGGER fold_gen_owners_del AFTER DELETE ON group_owner_history BEGIN
  INSERT INTO fold_gen (group_id, gen) VALUES (OLD.group_id, 1)
    ON CONFLICT(group_id) DO UPDATE SET gen = gen + 1;
END;
CREATE TRIGGER fold_gen_owner_upd AFTER UPDATE OF owner_pk, kind ON groups BEGIN
  INSERT INTO fold_gen (group_id, gen) VALUES (NEW.group_id, 1)
    ON CONFLICT(group_id) DO UPDATE SET gen = gen + 1;
END;
CREATE TABLE IF NOT EXISTS store_gen (
  id  INTEGER PRIMARY KEY CHECK (id = 0),
  gen INTEGER NOT NULL
);
DROP TRIGGER IF EXISTS store_gen_folds_ins;
DROP TRIGGER IF EXISTS store_gen_folds_upd;
DROP TRIGGER IF EXISTS store_gen_groups_ins;
DROP TRIGGER IF EXISTS store_gen_groups_del;
DROP TRIGGER IF EXISTS store_gen_groups_upd;
DROP TRIGGER IF EXISTS store_gen_spine_ins;
DROP TRIGGER IF EXISTS store_gen_spine_del;
DROP TRIGGER IF EXISTS store_gen_epoch_ins;
DROP TRIGGER IF EXISTS store_gen_peers_ins;
DROP TRIGGER IF EXISTS store_gen_peers_del;
DROP TRIGGER IF EXISTS store_gen_peers_upd;
CREATE TRIGGER store_gen_folds_ins AFTER INSERT ON fold_gen BEGIN
  INSERT INTO store_gen (id, gen) VALUES (0, 1) ON CONFLICT(id) DO UPDATE SET gen = gen + 1;
END;
CREATE TRIGGER store_gen_folds_upd AFTER UPDATE ON fold_gen BEGIN
  INSERT INTO store_gen (id, gen) VALUES (0, 1) ON CONFLICT(id) DO UPDATE SET gen = gen + 1;
END;
CREATE TRIGGER store_gen_groups_ins AFTER INSERT ON groups BEGIN
  INSERT INTO store_gen (id, gen) VALUES (0, 1) ON CONFLICT(id) DO UPDATE SET gen = gen + 1;
END;
CREATE TRIGGER store_gen_groups_del AFTER DELETE ON groups BEGIN
  INSERT INTO store_gen (id, gen) VALUES (0, 1) ON CONFLICT(id) DO UPDATE SET gen = gen + 1;
END;
CREATE TRIGGER store_gen_groups_upd AFTER UPDATE OF kind, departed_at ON groups BEGIN
  INSERT INTO store_gen (id, gen) VALUES (0, 1) ON CONFLICT(id) DO UPDATE SET gen = gen + 1;
END;
CREATE TRIGGER store_gen_spine_ins AFTER INSERT ON spine_outbox BEGIN
  INSERT INTO store_gen (id, gen) VALUES (0, 1) ON CONFLICT(id) DO UPDATE SET gen = gen + 1;
END;
CREATE TRIGGER store_gen_spine_del AFTER DELETE ON spine_outbox BEGIN
  INSERT INTO store_gen (id, gen) VALUES (0, 1) ON CONFLICT(id) DO UPDATE SET gen = gen + 1;
END;
CREATE TRIGGER store_gen_epoch_ins AFTER INSERT ON epoch_tag BEGIN
  INSERT INTO store_gen (id, gen) VALUES (0, 1) ON CONFLICT(id) DO UPDATE SET gen = gen + 1;
END;
CREATE TRIGGER store_gen_peers_ins AFTER INSERT ON peers BEGIN
  INSERT INTO store_gen (id, gen) VALUES (0, 1) ON CONFLICT(id) DO UPDATE SET gen = gen + 1;
END;
CREATE TRIGGER store_gen_peers_del AFTER DELETE ON peers BEGIN
  INSERT INTO store_gen (id, gen) VALUES (0, 1) ON CONFLICT(id) DO UPDATE SET gen = gen + 1;
END;
CREATE TRIGGER store_gen_peers_upd AFTER UPDATE OF display_name, status, identity_group_id ON peers BEGIN
  INSERT INTO store_gen (id, gen) VALUES (0, 1) ON CONFLICT(id) DO UPDATE SET gen = gen + 1;
END;
"#;

/// THE SENT LEDGER (D-34 (c); srr/security.md step 7): a hash of every blob this state
/// has published, so a drain can tell an echo of its own from a message that another
/// copy of this device, sealed later, sent. Kept `LEASE_SECS`: past that the leaf it
/// speaks for may be evicted in any case.
const OWN_SENT: &str = r#"
CREATE TABLE IF NOT EXISTS own_sent (
  hash BLOB PRIMARY KEY,
  at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS own_sent_at ON own_sent(at);
"#;

/// Set when a device's store opens in this process; publishes are ledgered from then
/// on. A `Router` with no store beside it keeps none.
static LEDGER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn sent_hash(blob_b64: &str) -> [u8; 32] {
    <sha2::Sha256 as sha2::Digest>::digest(blob_b64.as_bytes()).into()
}

/// Ledger a blob this state is about to publish. `Router` publishes and holds no
/// Directory, so this goes through a connection of its own, and before the send, so
/// no echo can come back ahead of it. The blob is the string the relay stores and
/// hands back unchanged.
pub fn record_sent(blob_b64: &str) -> Result<(), CoreError> {
    if !LEDGER.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(());
    }
    let conn = Connection::open(paths::db_path())?;
    conn.busy_timeout(std::time::Duration::from_secs(2))?;
    conn.execute_batch(OWN_SENT)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    conn.execute(
        "INSERT OR IGNORE INTO own_sent (hash, at) VALUES (?1, ?2)",
        rusqlite::params![&sent_hash(blob_b64)[..], now],
    )?;
    conn.execute("DELETE FROM own_sent WHERE at < ?1", [now - crate::lease::LEASE_SECS])?;
    Ok(())
}

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA secure_delete = ON;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS me (
  identity_pk      BLOB PRIMARY KEY,
  next_key_commit  BLOB NOT NULL,
  intro_inbox_tag  BLOB NOT NULL,
  display_name     TEXT NOT NULL,
  -- the MLS leaf signature keypair, persisted so every fresh CLI process
  -- rebuilds the SAME client identity and can load_group() the stored state.
  mls_sig_sk       BLOB NOT NULL,
  mls_sig_pk       BLOB NOT NULL
);

-- What we last PUBLISHED into each connection, as a digest of the profile bytes.
-- Not a copy of the profile (the connection's own log is that) — just enough to
-- answer "does this peer already hold my current card?" in O(1), without folding.
-- That is what makes the fan-out self-healing: any connection whose digest differs
-- from the live profile (a new pairing, a failed append, a restored backup) is
-- re-published on the next sync rather than being silently left behind.
CREATE TABLE IF NOT EXISTS profile_publish (
  group_id  BLOB PRIMARY KEY,
  digest    BLOB NOT NULL,
  at        INTEGER NOT NULL
);

-- The same self-healing ledger, per LISTING rather than per profile: what we last
-- published (or relayed) into each connection, keyed by the listing's network-wide
-- identity `(origin, thing_id)`.
--
-- Per-listing rather than one digest per connection, because a market has many listings
-- and they change independently: a price edit on one must not re-publish the other nine.
-- `rev` and `hops` ride alongside the digest so the forwarding pass can answer "have I
-- already told this peer about this listing, at this revision, by a path at least this
-- short?" without folding their log.
-- `reach` is recorded because a WITHDRAWAL has to travel as far as the listing did, and by
-- the time you withdraw, the Thing's own reach has been reset (clearing a posture drops the
-- audience, so a later posture cannot inherit an unstated one). The ledger is therefore the
-- only place that still remembers how far the announcement was allowed to go.
CREATE TABLE IF NOT EXISTS listing_publish (
  group_id  BLOB NOT NULL,
  origin    BLOB NOT NULL,
  thing_id  TEXT NOT NULL,
  digest    BLOB NOT NULL,
  rev       INTEGER NOT NULL,
  hops      INTEGER NOT NULL,
  reach     TEXT NOT NULL DEFAULT 'private',
  at        INTEGER NOT NULL,
  PRIMARY KEY (group_id, origin, thing_id)
);

CREATE TABLE IF NOT EXISTS peers (
  peer_id         BLOB PRIMARY KEY,
  display_name    TEXT NOT NULL DEFAULT '',
  next_key_commit BLOB,
  intro_inbox_tag BLOB,
  status          TEXT NOT NULL,
  first_seen      INTEGER NOT NULL,
  last_seen       INTEGER NOT NULL,
  why             TEXT,         -- the captured "why" of the connection (n+1 flow)
  identity_group_id BLOB       -- EDGE: this peer's shareable identity Group (their "who").
                                -- The Contact channel is keyed by peer_id (the pairing group);
                                -- this row is the hub linking peer -> identity Group.
);

CREATE TABLE IF NOT EXISTS groups (
  group_id  BLOB PRIMARY KEY,
  kind      TEXT NOT NULL,
  owner_pk  BLOB,             -- the creator's identity pubkey (owner-as-sequencer)
  arc_url   TEXT              -- the Arc that governs this object (moderation/reports/
                              -- safety) and whose Semaphore relay its traffic routes
                              -- through. Set by the creator; swapped by supermajority.
);

-- Projection of the MLS ratchet-tree roster: (group_id, member_pk). The MLS group
-- IS the source of truth for membership; this cache is refreshed on every change
-- so folds/enumeration never need to load the group. Rebuildable at any time from
-- mls::roster_identities. (Replaces the old pairwise `connections` table, whose
-- `is_lo` only made sense for exactly two parties.)
CREATE TABLE IF NOT EXISTS group_members (
  group_id  BLOB NOT NULL,
  member_pk BLOB NOT NULL,
  PRIMARY KEY (group_id, member_pk)
);

-- THE LEAF COUNT, kept beside the rows above because it cannot be recovered from
-- them. `group_members` is PRIMARY KEY (group_id, member_pk): a set of PEOPLE,
-- and that dedup is load-bearing — it is what stops one person's several devices
-- inflating every membership count in the system (the wallet's quorum bands,
-- `is_member`, `MembershipLog::divergence`). Since `mls::PerLeafIdentity` landed,
-- one person may hold SEVERAL LEAVES of one ratchet tree, so "how many people are
-- authorised here" and "how many leaves are there" became different questions.
-- Only the second one answers "does this group need the relay": a group of one
-- person holding two devices has one row here and two leaves, and counting rows
-- would tell it to stay silent — which is the laptop never seeing what the phone
-- wrote in your own notes.
CREATE TABLE IF NOT EXISTS group_leaves (
  group_id BLOB PRIMARY KEY,
  leaves   INTEGER NOT NULL
);

-- THE GEN FLOOR — the lowest Lamport `gen` a delta we author here may take.
--
-- A `MsgRef` is (author_pk, gen), and `author_pk` is the PERSON, which is the
-- property multi-device exists to preserve. `gen` was derived purely from the
-- local log — max + 1 — so a device that joined late derived it from a log that
-- necessarily starts EMPTY: MLS forward secrecy means a joining leaf cannot read
-- anything sealed before its epoch, and `add_member_core` refuses to re-encrypt
-- history precisely so that keeps working.
--
-- So the second device restarted at 0 and re-minted refs its own person had
-- already used. That is not a display collision: commutative deltas are keyed by
-- ref in the fold, so the later delta REPLACED the earlier one on every device
-- that held both — and which one survived was decided by `delta_id`, a hash of
-- the content. Measured 15 Sep 2026: three devices ended up permanently
-- disagreeing about the transcript, with one of the original messages destroyed
-- everywhere.
--
-- The floor is carried in the sealed Welcome (`IntroPayload::gen_watermark`) by
-- the device performing the add, which DOES hold the log and therefore knows the
-- high-water mark. The joiner stores it here and `Node::next_lamport` returns
-- max(local log, floor). Nothing about the ref changes shape, and no message is
-- decrypted that forward secrecy says it may not be.
CREATE TABLE IF NOT EXISTS group_gen_floor (
  group_id BLOB PRIMARY KEY,
  gen      INTEGER NOT NULL
);

-- Per-epoch shared group tag + seal secret. The MLS exporter can only derive the
-- CURRENT epoch's tag, so we persist each epoch's (tag, secret) as we hold it —
-- letting `sync` re-drain a SUPERSEDED epoch's mailbox for application messages
-- that raced past the epoch fence (mls-rs retains the epoch secrets to decrypt).
-- Without this a Delta posted to an epoch the group already left is lost forever.
CREATE TABLE IF NOT EXISTS epoch_tag (
  group_id BLOB NOT NULL,
  epoch    INTEGER NOT NULL,
  tag      BLOB NOT NULL,
  secret   BLOB NOT NULL,
  PRIMARY KEY (group_id, epoch)
);

-- THE SPINE'S OUTBOX. One row per entry this account has minted and not yet
-- published to the address the seed derives for it.
--
-- Separate from `delta_log.delivered`, which is the outbox for DELTAS to a
-- group's mailbox. A spine entry goes to an ACCOUNT-addressed slot — an address
-- only this account can compute and only this account may write — so it has a
-- different destination, a different key and a different claim rule, and folding
-- it into the delta outbox would have made one drain answer for two.
--
-- `blob` is already sealed, by `spine::place`, under a key off the storage root.
-- This table never holds plaintext and the arc never holds the key. The row is
-- kept after delivery rather than deleted, so a device can tell "published" from
-- "never minted" — which are different answers to "can I get this back".
-- THIS INSTALL, as a lease holder (resumption.md §6.1): 32 random bytes minted
-- once. Not the identity — every device of a person shares that.
CREATE TABLE IF NOT EXISTS device (
  k  INTEGER PRIMARY KEY CHECK (k = 0),
  id BLOB NOT NULL
);

-- THE POOL LEAVES THIS DEVICE HOLDS: which leaf, in which object, at which cell,
-- until when, and whether a later cell fenced it out (§6.2).
CREATE TABLE IF NOT EXISTS pool_lease (
  group_id BLOB    NOT NULL,
  pool     BLOB    NOT NULL,
  cell     INTEGER NOT NULL,
  until    INTEGER NOT NULL,
  fenced   INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (group_id, pool)
);

CREATE TABLE IF NOT EXISTS spine_outbox (
  gen       INTEGER NOT NULL,
  idx       INTEGER NOT NULL,
  address   BLOB NOT NULL,
  blob      BLOB NOT NULL,
  delivered INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (gen, idx)
);

-- OUR intro mailbox is always live (set at id init); `sync` drains it, and for
-- each sealed Welcome it joins the group and records the new connection. The
-- scannee learns the peer's identity from the sealed IntroPayload, so no
-- per-peer pending row is needed — the intro tag is in the `me` table.


-- A drain position is only meaningful inside ONE transport's sequence space: `seq` is assigned
-- by whichever relay accepted the publish and means nothing to a device that never saw it. So the
-- cursor is keyed by (tag, source) — without that, a router draining both a relay and the mesh
-- would have them overwrite each other's position and silently skip or replay messages.
-- `source` is the transport KIND ("relay" / "mesh"), not the relay's URL, so switching relays
-- inherits the position exactly as it did before this table was split. Making it per-URL is the
-- upgrade if we ever run two relays at once.
CREATE TABLE IF NOT EXISTS inbox_cursor_v2 (
  tag          BLOB NOT NULL,
  source       TEXT NOT NULL,
  last_seen_id INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (tag, source)
);

-- The substrate source of truth: the per-object append-only delta log.
-- `delivered` is the outbox flag: 0 = authored locally but not yet confirmed PUB'd
-- to the relay (flush on next reconnect), 1 = relay-ack'd. Only meaningful for our
-- OWN deltas; received deltas keep the default and are never re-sent.
CREATE TABLE IF NOT EXISTS delta_log (
  group_id    BLOB NOT NULL,
  delta_id    BLOB NOT NULL,
  author_pk   BLOB NOT NULL,
  envelope    BLOB NOT NULL,        -- canonical CBOR bytes
  -- The AUTHOR'S OWN signature over (domain, group, author, delta_id) --
  -- see `crate::delta_sig`. NULL for rows written before authorship
  -- signatures existed, and for deltas received from a peer that has not
  -- started sending them. A NULL here is "unproven", never "trusted".
  author_sig  BLOB,
  received_at INTEGER NOT NULL,
  delivered   INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (group_id, delta_id)
);

-- Permanently-unprocessable blobs the ingest taxonomy skipped past (bounded to
-- QUARANTINE_CAP rows). Sync always advances its cursor over these; they are
-- kept as queryable evidence — loud, not silent — never retried forever.
CREATE TABLE IF NOT EXISTS quarantine (
  tag      BLOB NOT NULL,
  seq      INTEGER NOT NULL,
  group_id BLOB,               -- NULL for intro-mailbox (pre-group) blobs
  reason   TEXT NOT NULL,
  blob     BLOB NOT NULL,
  at       INTEGER NOT NULL,
  PRIMARY KEY (tag, seq)
);

-- WHO OWNED THE OBJECT AT EACH EPOCH (membership-through-mls.md §10.2). The owner
-- lives in the MLS GroupContext (mls::OWNER_EXT) and changes only by a handover
-- commit, so it is a function of the epoch. The spine accepts a sequenced delta at
-- epoch e only from owner_at(e): the last row with from_epoch <= e. Written at
-- creation (0, creator), at join (join epoch, the context's owner), and on every
-- commit that moves the owner. groups.owner_pk stays as the cache of the LAST row.
CREATE TABLE IF NOT EXISTS group_owner_history (
  group_id   BLOB NOT NULL,
  from_epoch INTEGER NOT NULL,
  owner_pk   BLOB NOT NULL,
  PRIMARY KEY (group_id, from_epoch)
);

-- WHEN THIS DEVICE LAST REFRESHED ITS OWN LEAF in a group (§11). Written whenever a
-- commit of ours wins its slot — every commit carries a path, so every win is a
-- refresh — and at creation and join. Nothing schedules from it since the weekly
-- self-update was dropped (19 Sep); kept, because a shipped table costs nothing.
CREATE TABLE IF NOT EXISTS group_self_update (
  group_id BLOB PRIMARY KEY,
  at       INTEGER NOT NULL,
  epoch    INTEGER NOT NULL
);

-- A LEAVE IN FLIGHT (§6). The leaver proposed its own removal at `epoch`, and another
-- member must commit it. A proposal is valid only in the epoch it was sent (RFC 9420
-- §12.1), so while this row stands and the device is still in the tree, sync proposes
-- again at every later epoch. In-flight state, like the outbox's `delivered` flag: the
-- LASTING fact is the MLS removal itself (and, on Group kinds, the memberLeft record).
-- Deleted when the removal lands (§8.3).
-- WHO A KNOCK EXPECTS TO FIND AS OWNER (§8.5 step 2). Written when this device
-- knocks on a join card that names the owner; checked when the Welcome arrives. A
-- Welcome for this group whose context names a different owner is refused — the one
-- defence against a member who adds a newcomer and rewrites the owner in one commit.
CREATE TABLE IF NOT EXISTS expected_owner (
  group_id BLOB PRIMARY KEY,
  owner_pk BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS pending_departure (
  group_id BLOB PRIMARY KEY,
  epoch    INTEGER NOT NULL,
  since    INTEGER NOT NULL
);
"#;

/// Upper bound on retained quarantine rows (oldest evicted first) — a flooding
/// peer must not be able to grow local disk unboundedly.
pub const QUARANTINE_CAP: i64 = 256;

/// The on-device directory. Owns a SQLite connection.
pub struct Directory {
    pub conn: Connection,
    /// Rows whose authorship signature this Directory has already checked, keyed by
    /// a hash of group, author, signature and envelope: a row that changes in any of
    /// them is checked again (A-10; RX.8).
    verified: std::sync::Mutex<std::collections::HashSet<[u8; 32]>>,
    /// Random at every open (O-69): a restore that brings back an older `fold_gen` never
    /// meets a cache entry made before it.
    pub nonce: u64,
    /// The folds of this open (O-69): keyed as `crate::fold_cache::Key` says.
    pub folds: std::sync::Mutex<crate::fold_cache::FoldCache>,
}

impl Directory {
    /// Open + migrate the DB at the resolved state path. Idempotent.
    pub fn open() -> Result<Self, CoreError> {
        std::fs::create_dir_all(paths::state_dir())?;
        let conn = Connection::open(paths::db_path())?;
        Self::init(&conn)?;
        LEDGER.store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(Self::with(conn))
    }

    /// Open an explicit path (tests). Idempotent.
    pub fn open_at(path: &std::path::Path) -> Result<Self, CoreError> {
        let conn = Connection::open(path)?;
        Self::init(&conn)?;
        Ok(Self::with(conn))
    }

    fn with(conn: Connection) -> Self {
        let mut nonce = [0u8; 8];
        getrandom::getrandom(&mut nonce).expect("the OS's randomness");
        Self { conn, verified: Default::default(), nonce: u64::from_le_bytes(nonce), folds: Default::default() }
    }

    /// The fold cache's key for this object's fold under `lens` (O-69), or none when this
    /// process has no model id and folds uncached.
    pub fn fold_key(&self, id: &[u8], lens: &str, what: &'static str) -> Result<Option<crate::fold_cache::Key>, CoreError> {
        let Some(model) = crate::fold_cache::model() else { return Ok(None) };
        Ok(Some(crate::fold_cache::Key { object: id.to_vec(), lens: lens.to_string(), what, nonce: self.nonce, gen: self.fold_gen(id)?, model }))
    }

    /// One product of one object's fold, through the fold cache (O-69): THE cached path,
    /// for every fold and view. The key is read before `fresh` reads its inputs, so a write
    /// between them leaves an entry only under a key already stale. `same` is what FC-2's
    /// check compares a hit and a refold by. A failure of the rows is kept as that failure;
    /// one of this device (its storage, its IO) is not, and is folded again next time.
    pub fn cached<V: Clone + Send + Sync + 'static>(
        &self,
        id: &[u8],
        lens: &str,
        what: &'static str,
        same: impl Fn(&V, &V) -> bool,
        fresh: impl Fn() -> Result<V, CoreError>,
    ) -> Result<V, CoreError> {
        use crate::object_store::Refusal;
        let Some(key) = self.fold_key(id, lens, what)? else { return fresh() };
        let hit = self.folds.lock().ok().and_then(|mut c| c.get::<Result<V, Refusal>>(&key));
        if let Some(hit) = hit {
            if crate::fold_cache::verify() {
                let again = fresh();
                let ok = match (&hit, &again) {
                    (Ok(a), Ok(b)) => same(a, b),
                    (Err(a), Err(b)) => Refusal::of(b).as_ref() == Some(a),
                    _ => false,
                };
                assert!(ok, "the fold cache differs from a refold of {} under {key:?}", hex::encode(id));
            }
            return hit.map_err(Refusal::into_error);
        }
        let out = fresh();
        let kept = match &out {
            Ok(v) => Some(Ok(v.clone())),
            Err(e) => Refusal::of(e).map(Err),
        };
        if let (Some(kept), Ok(mut c)) = (kept, self.folds.lock()) {
            c.put::<Result<V, Refusal>>(key, &kept);
        }
        out
    }

    /// The object's fold generation: 0 when nothing the fold reads was ever written.
    pub fn fold_gen(&self, group_id: &[u8]) -> Result<u64, CoreError> {
        Ok(self
            .conn
            .query_row("SELECT gen FROM fold_gen WHERE group_id = ?1", params![group_id], |r| r.get::<_, i64>(0))
            .optional()?
            .unwrap_or(0) as u64)
    }

    /// THE STORE'S GENERATION, G (O-69; mdr/fold-cache.md § Four layers): raised by triggers,
    /// inside the writing transaction, whenever a row a graph shows is written: a fold
    /// generation, the object list and its departures, the spine, a new epoch, a peer, its
    /// name or standing. Not by what a graph never shows (a cursor, the relay's
    /// acknowledgement, an epoch's key re-sealed, a peer's last-seen). Equal means nothing a
    /// graph shows has moved. A count of changes, never read as state.
    pub fn generation(&self) -> Result<u64, CoreError> {
        Ok(self
            .conn
            .query_row("SELECT gen FROM store_gen WHERE id = 0", [], |r| r.get::<_, i64>(0))
            .optional()?
            .unwrap_or(0) as u64)
    }

    /// The same, over the given objects only: their fold generations and epochs. A public
    /// site's stream moves when its scope does and at no other time (NC-41).
    pub fn generation_of(&self, ids: &[Vec<u8>]) -> Result<u64, CoreError> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for id in ids {
            let epoch: i64 = self
                .conn
                .query_row("SELECT COALESCE(MAX(epoch), -1) FROM epoch_tag WHERE group_id = ?1", params![id], |r| r.get(0))?;
            (id, self.fold_gen(id)?, epoch, self.is_departed(id)?).hash(&mut h);
        }
        Ok(h.finish())
    }

    /// The highest seq drained from `source` on any tag: a subscription opened now replays
    /// nothing at or before it that a drain has not already brought.
    pub fn highest_cursor(&self, source: &str) -> Result<u64, CoreError> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(MAX(last_seen_id), 0) FROM inbox_cursor_v2 WHERE source = ?1",
            params![source],
            |r| r.get::<_, i64>(0),
        )? as u64)
    }

    fn init(conn: &Connection) -> Result<(), CoreError> {
        conn.execute_batch(SCHEMA)?;
        conn.execute_batch(OWN_SENT)?;
        // Best-effort migration for DBs created before the outbox column existed.
        // Errors (duplicate column) are expected on already-migrated DBs — ignore.
        let _ = conn.execute(
            "ALTER TABLE delta_log ADD COLUMN delivered INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute("ALTER TABLE delta_log ADD COLUMN author_sig BLOB", []);
        // O-75: the admitter a row came via, when it came in a history bundle rather than
        // from its author's own message; NULL for every row received directly.
        let _ = conn.execute("ALTER TABLE delta_log ADD COLUMN via BLOB", []);
        // O-75: who committed the Add that let this device into each object (MLS's
        // NewMemberInfo.sender), and history bundles held for an object not yet joined.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS admitted_by (
               group_id  BLOB PRIMARY KEY,
               committer BLOB NOT NULL,
               at        INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS held_history (
               id     INTEGER PRIMARY KEY AUTOINCREMENT,
               object BLOB NOT NULL,
               bundle BLOB NOT NULL,
               at     INTEGER NOT NULL
             );",
        )?;
        let _ = conn.execute("ALTER TABLE peers ADD COLUMN why TEXT", []);
        let _ = conn.execute("ALTER TABLE peers ADD COLUMN identity_group_id BLOB", []);
        let _ = conn.execute("ALTER TABLE groups ADD COLUMN owner_pk BLOB", []);
        // Per-object governing Arc (moderation/reports/safety + routing). Added after
        // owner_pk; expected-duplicate on already-migrated DBs — ignore.
        let _ = conn.execute("ALTER TABLE groups ADD COLUMN arc_url TEXT", []);
        // DEPARTED (membership-through-mls.md §8.3): a commit removed this device. The
        // row and the delta log stay — history this person legitimately received — but
        // the MLS state is deleted, and sync, the doors and rekey skip the group.
        let _ = conn.execute("ALTER TABLE groups ADD COLUMN departed_at INTEGER", []);
        let _ = conn.execute("ALTER TABLE groups ADD COLUMN departed_by BLOB", []);
        // MY OWN profile beyond the bare display name: the vCard-shaped ContactCard
        // (org/title/emails/phones/urls/photo/clip/note/tags) + its shape. Before
        // this, the app's avatar lived in iOS UserDefaults and never crossed the FFI,
        // so it could not be published to anyone. The core owns it now — one source.
        let _ = conn.execute(
            "ALTER TABLE me ADD COLUMN card_json TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE me ADD COLUMN profile_shape TEXT NOT NULL DEFAULT 'individual'",
            [],
        );
        // THE SELF RECORD: this account's own GroupObject, a group of one, at the
        // base of its spine. `card_json`/`profile_shape` above become its cache —
        // the record is the source, because a row in this file is the one thing a
        // device holding only the words does not have.
        let _ = conn.execute("ALTER TABLE me ADD COLUMN self_object BLOB", []);
        // The pairwise `connections` table is fully superseded by group_members +
        // groups.owner_pk; drop it so no dual source of truth lingers on an
        // upgraded M1 DB (group_members self-heals from the MLS roster on sync).
        let _ = conn.execute("DROP TABLE IF EXISTS connections", []);
        // Cursors used to be keyed by tag alone, which is the same thing as "keyed by the one
        // relay". Carry them over as relay cursors so an upgrade does not re-drain (and quarantine)
        // every mailbox it has already read, then retire the old table.
        let _ = conn.execute(
            "INSERT OR IGNORE INTO inbox_cursor_v2 (tag, source, last_seen_id)
             SELECT tag, 'relay', last_seen_id FROM inbox_cursor",
            [],
        );
        let _ = conn.execute("DROP TABLE IF EXISTS inbox_cursor", []);
        // After the migrations: the triggers name columns some of them add.
        conn.execute_batch(FOLD_GEN)?;
        Ok(())
    }

    // ---- me ----
    /// At-rest domain-separation context for the MLS signing SECRET key (the public
    /// key stays in the clear — only the secret is sealed under the device key).
    const MLS_SIG_SK_CTX: &'static [u8] = b"pacific/mls/sig_sk";

    #[allow(clippy::too_many_arguments)]
    pub fn put_me(
        &self,
        identity_pk: &[u8; 32],
        next_key_commit: &[u8; 32],
        intro_inbox_tag: &[u8; 32],
        display_name: &str,
        mls_sig_sk: &[u8],
        mls_sig_pk: &[u8],
    ) -> Result<(), CoreError> {
        // Seal the MLS signing secret at rest under the device key (S5); plaintext
        // only when no key is configured (legacy/CLI).
        let sealed_sk = match crate::atrest::device_key()? {
            Some(key) => crate::atrest::seal_at_rest(Self::MLS_SIG_SK_CTX, mls_sig_sk, &key)?,
            None => mls_sig_sk.to_vec(),
        };
        self.conn.execute(
            "INSERT OR REPLACE INTO me
               (identity_pk, next_key_commit, intro_inbox_tag, display_name, mls_sig_sk, mls_sig_pk)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                &identity_pk[..],
                &next_key_commit[..],
                &intro_inbox_tag[..],
                display_name,
                &sealed_sk,
                mls_sig_pk
            ],
        )?;
        Ok(())
    }

    /// The persisted MLS signing keypair (sk_bytes, pk_bytes).
    pub fn mls_signing_keypair(&self) -> Result<(Vec<u8>, Vec<u8>), CoreError> {
        let (sk_stored, pk): (Vec<u8>, Vec<u8>) = self
            .conn
            .query_row("SELECT mls_sig_sk, mls_sig_pk FROM me LIMIT 1", [], |r| {
                Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))
            })
            .optional()?
            .ok_or(CoreError::NoIdentity)?;
        // Open the sealed signing secret; a sealed value without the key fails loud.
        let sk = if crate::atrest::is_sealed(&sk_stored) {
            match crate::atrest::device_key()? {
                Some(key) => crate::atrest::open_at_rest(Self::MLS_SIG_SK_CTX, &sk_stored, &key)?,
                None => return Err(CoreError::AtRestKeyUnavailable),
            }
        } else {
            sk_stored
        };
        Ok((sk, pk))
    }

    pub fn my_display_name(&self) -> Result<String, CoreError> {
        self.conn
            .query_row("SELECT display_name FROM me LIMIT 1", [], |r| r.get(0))
            .optional()?
            .ok_or(CoreError::NoIdentity)
    }

    /// Rename MYSELF. The display name is identity-adjacent but not identity: the
    /// key is the identity, the name is a profile field the owner may change and
    /// re-publish. Nothing else keys on it (the MLS credential id is the identity
    /// pubkey — see `Node::signer`), so a rename never invalidates a group or a
    /// pairing.
    pub fn set_my_display_name(&self, display_name: &str) -> Result<(), CoreError> {
        let n = self
            .conn
            .execute("UPDATE me SET display_name=?1", params![display_name])?;
        if n == 0 {
            return Err(CoreError::NoIdentity);
        }
        Ok(())
    }

    /// My own `(card_json, shape)`. `card_json` is "" on a DB that predates the
    /// column, which the caller reads as "an empty card", never as an error.
    pub fn my_profile(&self) -> Result<(String, String), CoreError> {
        self.conn
            .query_row("SELECT card_json, profile_shape FROM me LIMIT 1", [], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .optional()?
            .ok_or(CoreError::NoIdentity)
    }

    /// Persist my own card + shape. The fan-out is the NODE's job (it needs the
    /// relay); this is only the local write.
    /// This account's own GroupObject, if it has one.
    pub fn my_object(&self) -> Result<Option<Vec<u8>>, CoreError> {
        Ok(self
            .conn
            .query_row("SELECT self_object FROM me LIMIT 1", [], |r| {
                r.get::<_, Option<Vec<u8>>>(0)
            })
            .optional()?
            .flatten())
    }

    pub fn set_my_object(&self, group_id: &[u8]) -> Result<(), CoreError> {
        self.conn
            .execute("UPDATE me SET self_object=?1", params![group_id])?;
        Ok(())
    }

    pub fn set_my_profile(&self, card_json: &str, shape: &str) -> Result<(), CoreError> {
        let n = self.conn.execute(
            "UPDATE me SET card_json=?1, profile_shape=?2",
            params![card_json, shape],
        )?;
        if n == 0 {
            return Err(CoreError::NoIdentity);
        }
        Ok(())
    }

    /// The digest of the profile we last published into `group_id` (None = never).
    pub fn published_profile_digest(&self, group_id: &[u8]) -> Result<Option<Vec<u8>>, CoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT digest FROM profile_publish WHERE group_id=?1",
                params![group_id],
                |r| r.get::<_, Vec<u8>>(0),
            )
            .optional()?)
    }

    /// Record that `digest` is now published into `group_id`.
    pub fn set_published_profile_digest(
        &self,
        group_id: &[u8],
        digest: &[u8],
        at: i64,
    ) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT INTO profile_publish (group_id, digest, at) VALUES (?1, ?2, ?3)
             ON CONFLICT(group_id) DO UPDATE SET digest=excluded.digest, at=excluded.at",
            params![group_id, digest, at],
        )?;
        Ok(())
    }

    /// What we last told `group_id` about the listing `(origin, thing_id)`:
    /// `(digest, rev, hops)`. None = never told them.
    pub fn published_listing(
        &self,
        group_id: &[u8],
        origin: &[u8; 32],
        thing_id: &str,
    ) -> Result<Option<(Vec<u8>, u64, u32)>, CoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT digest, rev, hops FROM listing_publish
                 WHERE group_id=?1 AND origin=?2 AND thing_id=?3",
                params![group_id, &origin[..], thing_id],
                |r| {
                    Ok((
                        r.get::<_, Vec<u8>>(0)?,
                        r.get::<_, i64>(1)? as u64,
                        r.get::<_, i64>(2)? as u32,
                    ))
                },
            )
            .optional()?)
    }

    /// How far this listing was last allowed to travel, or None if it was never published.
    ///
    /// Answers both questions a withdrawal needs at once: "was this thing ever on the
    /// market?" (None → stay silent, there is nothing to retract) and "how far did the
    /// announcement go?" (Some → the tombstone must be allowed to follow it). Takes the
    /// widest reach on record, so a listing that was ever public is retracted publicly.
    pub fn last_published_reach(
        &self,
        origin: &[u8; 32],
        thing_id: &str,
    ) -> Result<Option<String>, CoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT reach FROM listing_publish
                 WHERE origin=?1 AND thing_id=?2
                 ORDER BY reach='network' DESC LIMIT 1",
                params![&origin[..], thing_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }

    /// How many deltas are on `group_id`'s log. Used as a Thing's monotone revision counter:
    /// every edit appends exactly one delta, and every device folding the same log computes
    /// the same number — which is what lets copies arriving by different paths be ordered.
    pub fn log_len(&self, group_id: &[u8]) -> Result<usize, CoreError> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM delta_log WHERE group_id=?1",
            params![group_id],
            |r| r.get::<_, i64>(0),
        )? as usize)
    }

    /// Record that `(origin, thing_id)` at `rev`/`hops` is now published into `group_id`.
    #[allow(clippy::too_many_arguments)]
    pub fn set_published_listing(
        &self,
        group_id: &[u8],
        origin: &[u8; 32],
        thing_id: &str,
        digest: &[u8],
        rev: u64,
        hops: u32,
        reach: &str,
        at: i64,
    ) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT INTO listing_publish (group_id, origin, thing_id, digest, rev, hops, reach, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(group_id, origin, thing_id) DO UPDATE SET
               digest=excluded.digest, rev=excluded.rev, hops=excluded.hops,
               reach=excluded.reach, at=excluded.at",
            params![
                group_id,
                &origin[..],
                thing_id,
                digest,
                rev as i64,
                hops as i64,
                reach,
                at
            ],
        )?;
        Ok(())
    }

    /// Restore this device's intro mailbox tag (D3). Its caller was the backup
    /// restore, retired with the escrow (a58798c); nothing calls it now.
    ///
    /// Only a restore calls this. A fresh install mints its tag in `put_me` and
    /// keeps it for life; a restored one has to come back at the address its
    /// contacts already hold, because their stocked prekey offers name it and
    /// their Welcomes are sealed to it. No row to update means no identity here,
    /// which is a caller error rather than a silent no-op.
    pub fn set_intro_tag(&self, tag: &[u8; 32]) -> Result<(), CoreError> {
        let n = self
            .conn
            .execute("UPDATE me SET intro_inbox_tag = ?1", params![&tag[..]])?;
        if n == 0 {
            return Err(CoreError::NoIdentity);
        }
        Ok(())
    }

    pub fn my_intro_tag(&self) -> Result<[u8; 32], CoreError> {
        let t: Vec<u8> = self
            .conn
            .query_row("SELECT intro_inbox_tag FROM me LIMIT 1", [], |r| r.get(0))
            .optional()?
            .ok_or(CoreError::NoIdentity)?;
        t.try_into()
            .map_err(|_| CoreError::Directory("intro tag width".into()))
    }

    // ---- peers ----
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_peer(
        &self,
        peer_id: &[u8; 32],
        display_name: &str,
        status: PeerStatus,
        next_key_commit: Option<&[u8; 32]>,
        intro_inbox_tag: Option<&[u8; 32]>,
        now: i64,
    ) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT INTO peers
               (peer_id, display_name, next_key_commit, intro_inbox_tag, status, first_seen, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(peer_id) DO UPDATE SET
               display_name=excluded.display_name, status=excluded.status, last_seen=excluded.last_seen",
            params![
                &peer_id[..],
                display_name,
                next_key_commit.map(|c| &c[..]),
                intro_inbox_tag.map(|t| &t[..]),
                status.as_str(),
                now
            ],
        )?;
        Ok(())
    }

    pub fn set_peer_status(&self, peer_id: &[u8; 32], status: PeerStatus) -> Result<(), CoreError> {
        let n = self.conn.execute(
            "UPDATE peers SET status=?2 WHERE peer_id=?1",
            params![&peer_id[..], status.as_str()],
        )?;
        if n == 0 {
            return Err(CoreError::UnknownPeer(hex::encode(peer_id)));
        }
        Ok(())
    }

    /// Refresh a peer's cached display name from the profile THEY published.
    ///
    /// `peers.display_name` is a projection, not a source of truth — the same status
    /// `group_members` has against the MLS roster. Its original value is whatever the
    /// pairing bundle carried at scan time; once the peer publishes a profile on the
    /// Contact channel, that fold is authoritative and this row is refreshed from it.
    /// A no-op when unchanged, so the periodic sync doesn't churn writes.
    pub fn set_peer_display_name(
        &self,
        peer_id: &[u8; 32],
        display_name: &str,
    ) -> Result<bool, CoreError> {
        let n = self.conn.execute(
            "UPDATE peers SET display_name=?2 WHERE peer_id=?1 AND display_name<>?2",
            params![&peer_id[..], display_name],
        )?;
        Ok(n > 0)
    }

    /// Every peer whose captured "why" starts with `prefix`, as `(peer_pk, rest)` where
    /// `rest` is the why with the prefix stripped.
    ///
    /// Used to reconcile PUBLIC PLACE joins: the scanner records *why* it paired
    /// ("place-join:<id>"), and the host replays that intent on every sync until it can
    /// be honoured. Durable rather than in-memory because the admit usually CANNOT run at
    /// intro time — the contact's prekeys are not stocked until `pair_accept` — so the
    /// intent has to outlive the sync that first saw it, and a restart.
    pub fn peers_why_prefixed(&self, prefix: &str) -> Result<Vec<([u8; 32], String)>, CoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT peer_id, why FROM peers WHERE why LIKE ?1 || '%'")?;
        let rows = stmt.query_map(params![prefix], |r| {
            Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (pk, why) = r?;
            out.push((
                pk.try_into()
                    .map_err(|_| CoreError::Directory("peer_pk width".into()))?,
                why[prefix.len()..].to_string(),
            ));
        }
        Ok(out)
    }

    /// Record the captured "why" of a connection (the n+1 flow's differentiator).
    pub fn set_peer_why(&self, peer_id: &[u8; 32], why: &str) -> Result<(), CoreError> {
        self.conn.execute(
            "UPDATE peers SET why=?2 WHERE peer_id=?1",
            params![&peer_id[..], why],
        )?;
        Ok(())
    }

    pub fn peer_status(&self, peer_id: &[u8; 32]) -> Result<PeerStatus, CoreError> {
        let s: String = self
            .conn
            .query_row(
                "SELECT status FROM peers WHERE peer_id=?1",
                params![&peer_id[..]],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| CoreError::UnknownPeer(hex::encode(peer_id)))?;
        PeerStatus::parse(&s)
    }

    /// Every peer this device knows: `(peer_id, display_name, status)`, ordered by
    /// peer id for a stable list. The archive's names-against-keys roster reads
    /// this — exactly the three projection columns, and deliberately not
    /// `next_key_commit` or `intro_inbox_tag`, which are this device's own pairing
    /// state and no other device's business. Status is the raw wire string
    /// (`PeerStatus::as_str`) rather than the enum: an archive is read by builds
    /// other than this one, and an unknown status should render, not fail to parse.
    pub fn peers(&self) -> Result<Vec<([u8; 32], String, String, Vec<u8>)>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT peer_id, display_name, status, identity_group_id FROM peers ORDER BY peer_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<Vec<u8>>>(3)?.unwrap_or_default(),
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (id, name, status, ig) = r?;
            out.push((
                id.try_into()
                    .map_err(|_| CoreError::Directory("peer_id width".into()))?,
                name,
                status,
                ig,
            ));
        }
        Ok(out)
    }

    // ---- groups ----
    /// Record a group's directory facts. `owner_pk` is the creator's identity
    /// pubkey (the owner-as-sequencer for membership changes). `arc_url` is the Arc
    /// that governs this object — set by the creator at mint (`Some`) or left for a
    /// joiner to inherit later (`None`). Idempotent on group_id; a `None` arc_url on
    /// a re-put NEVER clobbers an already-recorded Arc (COALESCE), so re-recording a
    /// group's facts can't silently drop its governing Arc.
    pub fn put_group(
        &self,
        group_id: &[u8],
        kind: &str,
        owner_pk: &[u8],
        arc_url: Option<&str>,
    ) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT INTO groups (group_id, kind, owner_pk, arc_url) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(group_id) DO UPDATE SET
               kind=excluded.kind,
               owner_pk=excluded.owner_pk,
               arc_url=COALESCE(excluded.arc_url, groups.arc_url)",
            params![group_id, kind, owner_pk, arc_url],
        )?;
        Ok(())
    }

    /// The Arc that governs this object (moderation/reports/safety + routing), if
    /// recorded. `None` means we have not learned it yet — the caller falls back to
    /// the device's default Arc for routing.
    pub fn group_arc(&self, group_id: &[u8]) -> Result<Option<String>, CoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT arc_url FROM groups WHERE group_id=?1",
                params![group_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    /// Set (or swap) the Arc that governs this object. The membership-supermajority
    /// gate lives in `node::swap_group_arc`; this is the persistence primitive.
    pub fn set_group_arc(&self, group_id: &[u8], arc_url: &str) -> Result<(), CoreError> {
        let n = self.conn.execute(
            "UPDATE groups SET arc_url=?2 WHERE group_id=?1",
            params![group_id, arc_url],
        )?;
        if n == 0 {
            return Err(CoreError::Directory(format!(
                "set_group_arc: no group {}",
                hex::encode(group_id)
            )));
        }
        Ok(())
    }

    /// The group owner's identity pubkey, if recorded.
    pub fn group_owner(&self, group_id: &[u8]) -> Result<Option<Vec<u8>>, CoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT owner_pk FROM groups WHERE group_id=?1",
                params![group_id],
                |r| r.get::<_, Option<Vec<u8>>>(0),
            )
            .optional()?
            .flatten())
    }

    // ── the owner through the epochs (membership-through-mls.md §10.2) ─────────

    /// Record that `owner` owns the group from `from_epoch` on, and refresh the
    /// `owner_pk` cache when this row is the latest. Idempotent: the same row twice is
    /// the same row.
    pub fn record_owner(&self, group_id: &[u8], from_epoch: u64, owner: &[u8; 32]) -> Result<(), CoreError> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO group_owner_history (group_id, from_epoch, owner_pk) VALUES (?1, ?2, ?3)",
            params![group_id, from_epoch as i64, &owner[..]],
        )?;
        let latest: Option<Vec<u8>> = tx
            .query_row(
                "SELECT owner_pk FROM group_owner_history WHERE group_id=?1 ORDER BY from_epoch DESC LIMIT 1",
                params![group_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(l) = latest {
            tx.execute("UPDATE groups SET owner_pk=?2 WHERE group_id=?1", params![group_id, l])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Every owner the group has had, ascending by the epoch they took over. A group
    /// with no recorded history (made before the table existed, or restored from an
    /// archive without one) answers with its `owner_pk` from epoch 0 — which is true
    /// for every such group, because no handover could take effect before now.
    pub fn owner_history(&self, group_id: &[u8]) -> Result<Vec<(u64, [u8; 32])>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT from_epoch, owner_pk FROM group_owner_history WHERE group_id=?1 ORDER BY from_epoch",
        )?;
        let mut out = Vec::new();
        for row in stmt.query_map(params![group_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))? {
            let (e, pk) = row?;
            let pk: [u8; 32] = pk
                .as_slice()
                .try_into()
                .map_err(|_| CoreError::Directory("owner history width".into()))?;
            out.push((e.max(0) as u64, pk));
        }
        if out.is_empty() {
            if let Some(pk) = self.group_owner(group_id)? {
                let pk: [u8; 32] = pk
                    .as_slice()
                    .try_into()
                    .map_err(|_| CoreError::Directory("owner_pk width".into()))?;
                out.push((0, pk));
            }
        }
        Ok(out)
    }

    // ── departure (§8.3) ─────────────────────────────────────────────────────

    /// A commit removed this device from the group: mark it, and delete every secret
    /// this device held for it outside mls-rs — the epoch tags and their seal secrets,
    /// and the cursors that pointed at them. The row, the roster as last known and the
    /// delta log stay: they are history this person legitimately received.
    pub fn mark_departed(&self, group_id: &[u8], by: &[u8; 32], at: i64) -> Result<(), CoreError> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE groups SET departed_at=?2, departed_by=?3 WHERE group_id=?1",
            params![group_id, at, &by[..]],
        )?;
        tx.execute(
            "DELETE FROM inbox_cursor_v2 WHERE tag IN (SELECT tag FROM epoch_tag WHERE group_id=?1)",
            params![group_id],
        )?;
        tx.execute("DELETE FROM epoch_tag WHERE group_id=?1", params![group_id])?;
        tx.execute("DELETE FROM pending_departure WHERE group_id=?1", params![group_id])?;
        tx.commit()?;
        Ok(())
    }

    /// Has a commit removed this device from the group?
    pub fn is_departed(&self, group_id: &[u8]) -> Result<bool, CoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT departed_at IS NOT NULL FROM groups WHERE group_id=?1",
                params![group_id],
                |r| r.get::<_, bool>(0),
            )
            .optional()?
            .unwrap_or(false))
    }

    /// A fresh Add brought this device back (§8.3 step 6): it is a member again.
    pub fn clear_departed(&self, group_id: &[u8]) -> Result<(), CoreError> {
        self.conn.execute(
            "UPDATE groups SET departed_at=NULL, departed_by=NULL WHERE group_id=?1",
            params![group_id],
        )?;
        Ok(())
    }

    // ── the owner a knock expects (§8.5) ─────────────────────────────────────

    /// Record that a knock on a card expects `owner` to own `group_id`.
    pub fn set_expected_owner(&self, group_id: &[u8], owner: &[u8; 32]) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO expected_owner (group_id, owner_pk) VALUES (?1, ?2)",
            params![group_id, &owner[..]],
        )?;
        Ok(())
    }

    /// The owner a knock on this group's card expects, if one was named.
    pub fn expected_owner(&self, group_id: &[u8]) -> Result<Option<[u8; 32]>, CoreError> {
        let v: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT owner_pk FROM expected_owner WHERE group_id=?1",
                params![group_id],
                |r| r.get(0),
            )
            .optional()?;
        v.map(|b| {
            b.as_slice()
                .try_into()
                .map_err(|_| CoreError::Directory("expected owner width".into()))
        })
        .transpose()
    }

    // ── a leave in flight (§6) ───────────────────────────────────────────────

    /// This device has proposed its person's removal at `epoch`.
    pub fn set_pending_departure(&self, group_id: &[u8], epoch: u64, since: i64) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT INTO pending_departure (group_id, epoch, since) VALUES (?1, ?2, ?3)
             ON CONFLICT(group_id) DO UPDATE SET epoch=excluded.epoch",
            params![group_id, epoch as i64, since],
        )?;
        Ok(())
    }

    /// The epoch this device last proposed its person's removal at, if a leave is in
    /// flight.
    pub fn pending_departure(&self, group_id: &[u8]) -> Result<Option<u64>, CoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT epoch FROM pending_departure WHERE group_id=?1",
                params![group_id],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .map(|e| e.max(0) as u64))
    }

    // ── routine self-update (§11) ────────────────────────────────────────────

    /// This device's leaf was refreshed in the group at `at` (unix seconds), epoch `epoch`.
    pub fn record_self_update(&self, group_id: &[u8], at: i64, epoch: u64) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO group_self_update (group_id, at, epoch) VALUES (?1, ?2, ?3)",
            params![group_id, at, epoch as i64],
        )?;
        Ok(())
    }

    /// When this device's leaf was last refreshed in the group (unix seconds), if ever
    /// recorded.
    pub fn last_self_update(&self, group_id: &[u8]) -> Result<Option<i64>, CoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT at FROM group_self_update WHERE group_id=?1",
                params![group_id],
                |r| r.get::<_, i64>(0),
            )
            .optional()?)
    }

    /// The object kind for a group ("forum", "poll", … or "connection").
    pub fn group_kind(&self, group_id: &[u8]) -> Result<Option<String>, CoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT kind FROM groups WHERE group_id=?1",
                params![group_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Every standalone OBJECT we hold (a group whose kind is a real object type,
    /// i.e. not a bare 2-party "connection"). Returns (group_id, kind).
    pub fn list_objects(&self) -> Result<Vec<(Vec<u8>, String)>, CoreError> {
        let mut stmt = self.conn.prepare(
            // `notebook` and `note` are excluded for the same reason `connection` is:
            // they are not user-facing "objects" in the LIFE sense, and every
            // consumer of this list filters by kind anyway. A notebook is plumbing.
            "SELECT group_id, kind FROM groups \
             WHERE kind NOT IN ('connection', 'notebook', 'note') ORDER BY group_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Every group we hold (group_id, kind) — `sync` iterates these. Includes
    /// connection groups and standalone objects alike (all are MLS groups).
    pub fn all_groups(&self) -> Result<Vec<(Vec<u8>, String)>, CoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT group_id, kind FROM groups ORDER BY group_id")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Replace a group's cached member set with the current MLS roster. Called
    /// after every membership change (create/add/join) — the projection is only
    /// ever a mirror of `mls::roster_identities`, never an independent truth.
    ///
    /// `members` IS THE LEAF LIST, duplicates included — `mls::roster_identities`
    /// walks the ratchet tree and dedups nothing, and since `mls::PerLeafIdentity`
    /// one person may occupy several leaves. Two things are recorded from it, and
    /// they are different numbers:
    ///
    ///   * the ROWS, deduplicated by the primary key — who is authorised here.
    ///     Every fold, quorum band and `is_member` check reads these, and the
    ///     dedup is the only thing making a roster a set of people.
    ///   * the COUNT, written whole — how many leaves are in the tree. This is
    ///     what [`group_leaf_count`] serves and what decides whether the group
    ///     has anyone to send to.
    ///
    /// [`group_leaf_count`]: Self::group_leaf_count
    pub fn set_group_members(
        &self,
        group_id: &[u8],
        members: &[[u8; 32]],
    ) -> Result<(), CoreError> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM group_members WHERE group_id=?1",
            params![group_id],
        )?;
        for m in members {
            tx.execute(
                "INSERT OR IGNORE INTO group_members (group_id, member_pk) VALUES (?1, ?2)",
                params![group_id, &m[..]],
            )?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO group_leaves (group_id, leaves) VALUES (?1, ?2)",
            params![group_id, members.len() as i64],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Raise this group's gen floor — the lowest `gen` a delta authored here may
    /// take. See the `group_gen_floor` schema note for why it exists.
    ///
    /// MONOTONIC BY CONSTRUCTION: it takes the max of what is stored and what is
    /// offered, so a late or replayed Welcome can only ever move it forward. A
    /// floor that could go DOWN would re-open the exact collision it closes.
    pub fn raise_gen_floor(&self, group_id: &[u8], gen: u64) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT INTO group_gen_floor (group_id, gen) VALUES (?1, ?2)
             ON CONFLICT(group_id) DO UPDATE SET gen = MAX(gen, excluded.gen)",
            params![group_id, gen as i64],
        )?;
        Ok(())
    }

    /// This group's gen floor, or 0 when none was recorded — which is every group
    /// this device created itself, and every group joined before the floor existed.
    /// Zero is the correct answer for both: a device that has the history derives
    /// the right `gen` from its own log.
    pub fn gen_floor(&self, group_id: &[u8]) -> Result<u64, CoreError> {
        let g: Option<i64> = self
            .conn
            .query_row(
                "SELECT gen FROM group_gen_floor WHERE group_id=?1",
                params![group_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(g.unwrap_or(0).max(0) as u64)
    }

    /// How many LEAVES the group's ratchet tree holds — NOT how many people.
    ///
    /// The one question the five relay gates in `node.rs` ask. A person's own
    /// objects (their identity record, their notes) have exactly one member and,
    /// once they sign in on a second device, two leaves: counting members says
    /// "solo, never touch the relay" and the second device is never told anything.
    /// Counting leaves says "there is someone to send to", which is the truth.
    ///
    /// FALLS BACK TO THE ROW COUNT when no scalar has been written yet — a
    /// directory that predates this table, or one restored from an archive (see
    /// `Node::import_archive`). The fallback is a FLOOR, never an over-count, so
    /// the worst it can do is leave a group as quiet as it was before; the next
    /// `set_group_members` from a live roster corrects it.
    pub fn group_leaf_count(&self, group_id: &[u8]) -> Result<usize, CoreError> {
        let stored: Option<i64> = self
            .conn
            .query_row(
                "SELECT leaves FROM group_leaves WHERE group_id=?1",
                params![group_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(n) = stored {
            return Ok(n.max(0) as usize);
        }
        let rows: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM group_members WHERE group_id=?1",
            params![group_id],
            |r| r.get(0),
        )?;
        Ok(rows.max(0) as usize)
    }

    /// A group's cached member set (identity pubkeys). Feeds the OR-Set fold.
    pub fn group_members(&self, group_id: &[u8]) -> Result<Vec<[u8; 32]>, CoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT member_pk FROM group_members WHERE group_id=?1 ORDER BY member_pk")?;
        let rows = stmt.query_map(params![group_id], |r| r.get::<_, Vec<u8>>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(
                r?.try_into()
                    .map_err(|_| CoreError::Directory("member_pk width".into()))?,
            );
        }
        Ok(out)
    }

    /// Every 1:1 connection we hold, addressed by the peer's identity pubkey.
    /// A connection is a `kind='connection'` group; the peer is the member of
    /// that group who is not us. `display_name`/`status` come from the `peers`
    /// row when present (the projection of the intro exchange) — a connection
    /// whose peer row hasn't landed yet (e.g. we scanned but the double-opt-in
    /// intro is mid-flight) surfaces with an empty name and no status rather
    /// than being hidden. Returns (peer_pk, display_name, status_str_opt),
    /// ordered by peer_pk for a stable list.
    #[allow(clippy::type_complexity)]
    pub fn connections(
        &self,
        me: &[u8; 32],
    ) -> Result<Vec<([u8; 32], String, Option<String>)>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT m.member_pk, COALESCE(p.display_name, ''), p.status
               FROM groups g
               JOIN group_members m ON g.group_id = m.group_id
               LEFT JOIN peers p ON p.peer_id = m.member_pk
              WHERE g.kind = 'connection' AND m.member_pk != ?1
              ORDER BY m.member_pk",
        )?;
        let rows = stmt.query_map(params![&me[..]], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (pk, name, status) = r?;
            out.push((
                pk.try_into()
                    .map_err(|_| CoreError::Directory("peer_pk width".into()))?,
                name,
                status,
            ));
        }
        Ok(out)
    }

    /// The connection-kind group that contains `peer` — the peer-addressed
    /// convenience for `pacific post <peer>` (a connection is just a 2-member group).
    /// EDGE: link a peer to their shareable identity Group (their "who"). Directory-
    /// The stored display name for a peer ("" if unknown) — used when adding a known
    /// contact to a group via a prekey (we already have their name from pairing).
    pub fn peer_display_name(&self, peer: &[u8; 32]) -> Result<String, CoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT display_name FROM peers WHERE peer_id=?1",
                params![&peer[..]],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .unwrap_or_default())
    }

    /// The CONVERSATION with `peer` — the chat, which is not the connection.
    /// Same derivation as the connection edge, on the kind that carries messages.
    pub fn conversation_group_for(&self, peer: &[u8; 32]) -> Result<Option<Vec<u8>>, CoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT g.group_id FROM groups g
                   JOIN group_members m ON g.group_id = m.group_id
                 WHERE g.kind='conversation' AND m.member_pk=?1 LIMIT 1",
                params![&peer[..]],
                |r| r.get::<_, Vec<u8>>(0),
            )
            .optional()?)
    }

    pub fn connection_group_for(&self, peer: &[u8; 32]) -> Result<Option<Vec<u8>>, CoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT g.group_id FROM groups g
                   JOIN group_members m ON g.group_id = m.group_id
                 WHERE g.kind='connection' AND m.member_pk=?1 LIMIT 1",
                params![&peer[..]],
                |r| r.get::<_, Vec<u8>>(0),
            )
            .optional()?)
    }

    /// Persist a group's shared tag + seal secret for one epoch (idempotent). We
    /// can only derive the CURRENT epoch's tag from the MLS exporter, so we record
    /// each epoch as we pass through it and re-drain them all on every sync.
    ///
    /// Retention is BOUNDED to the newest [`crate::EPOCH_RETENTION`] + current
    /// epochs, in lockstep with `mls_store`'s epoch-secret pruning: a tag we can
    /// no longer decrypt is a tag we must no longer drain.
    /// Binds each sealed secret to its own row, so a ciphertext cannot be relocated
    /// to another (group, epoch) slot. Mirrors `mls_store`'s per-row context. Both
    /// bound values are the PRIMARY KEY and already in the clear.
    fn ctx_epoch_secret(group_id: &[u8], epoch: u64) -> Vec<u8> {
        let mut c = b"pacific/dir/epoch_tag_secret".to_vec();
        c.extend_from_slice(group_id);
        c.extend_from_slice(&epoch.to_le_bytes());
        c
    }

    /// Open a stored epoch secret. Three outcomes, and the difference between the
    /// last two is the whole safety of this column:
    ///   plaintext (legacy)   -> returned as-is
    ///   sealed, key present  -> opened, or `Err` if the AEAD tag fails
    ///   sealed, NO key       -> `Err(AtRestKeyUnavailable)` — TRANSIENT, never fatal
    /// A caller may discard a row that fails the AEAD check; it must NEVER discard
    /// one that merely had no key to try. "A key we cannot read is not a key that is
    /// absent" — the lesson `DeviceKey.swift` paid for.
    fn open_epoch_secret(context: &[u8], stored: Vec<u8>) -> Result<Zeroizing<[u8; 32]>, CoreError> {
        let raw = if crate::atrest::is_sealed(&stored) {
            let key = crate::atrest::device_key()?.ok_or(CoreError::AtRestKeyUnavailable)?;
            crate::atrest::open_at_rest(context, &stored, &key)?
        } else {
            stored
        };
        let arr: [u8; 32] = raw
            .try_into()
            .map_err(|_| CoreError::Directory("epoch secret width".into()))?;
        Ok(Zeroizing::new(arr))
    }

    pub fn record_epoch_tag(
        &self,
        group_id: &[u8],
        epoch: u64,
        tag: &[u8; 32],
        secret: &[u8; 32],
    ) -> Result<(), CoreError> {
        // SEALED WRITES UPSERT, PLAINTEXT WRITES DO NOT. With a device key the value
        // is sealed and an upsert is always an upgrade, so a legacy plaintext row is
        // replaced in place on the next tick — that IS the migration, bounded by
        // EPOCH_RETENTION, with no bulk UPDATE to tear. Without a key (the CLI, and
        // keyless tests) we fall back to `INSERT OR IGNORE`, which cannot overwrite
        // anything — so a sealed row can never degrade to plaintext, which is
        // `atrest`'s standing contract.
        match crate::atrest::device_key()? {
            Some(key) => {
                let sealed = crate::atrest::seal_at_rest(
                    &Self::ctx_epoch_secret(group_id, epoch),
                    &secret[..],
                    &key,
                )?;
                self.conn.execute(
                    "INSERT INTO epoch_tag (group_id, epoch, tag, secret) VALUES (?1, ?2, ?3, ?4)
                       ON CONFLICT(group_id, epoch) DO UPDATE SET tag=excluded.tag, secret=excluded.secret",
                    params![group_id, epoch as i64, &tag[..], &sealed[..]],
                )?;
            }
            None => {
                self.conn.execute(
                    "INSERT OR IGNORE INTO epoch_tag (group_id, epoch, tag, secret) VALUES (?1, ?2, ?3, ?4)",
                    params![group_id, epoch as i64, &tag[..], &secret[..]],
                )?;
            }
        }
        self.conn.execute(
            "DELETE FROM epoch_tag WHERE group_id=?1 AND epoch <
               (SELECT COALESCE(MAX(epoch),0) FROM epoch_tag WHERE group_id=?1) - ?2",
            params![group_id, crate::EPOCH_RETENTION as i64],
        )?;
        Ok(())
    }

    // ---- quarantine (the ingest taxonomy's terminal state for poison blobs) ----
    //
    // A blob that PERMANENTLY cannot be processed (orphan Welcome whose key
    // package was already consumed, garbage bytes, an undecryptable or replayed
    // MLS message) is recorded here and the mailbox cursor advances past it —
    // the poison never wedges sync, and never silently vanishes either: it is
    // queryable evidence with a bounded footprint. This is the field-consensus
    // contract (Wire's typed decrypt taxonomy, XMTP's non-retryable-advances
    // rule, Marmot's stale/deferred dispositions, Signal's consume-always
    // queue) — see _research/mls-transport-case-study.txt §4.1.

    /// Record one permanently-unprocessable blob. Bounded: the oldest rows are
    /// evicted beyond [`QUARANTINE_CAP`] so a flooding peer cannot grow disk.
    pub fn quarantine_put(
        &self,
        group_id: Option<&[u8]>,
        tag: &[u8; 32],
        seq: u64,
        reason: &str,
        blob: &[u8],
        now: i64,
    ) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO quarantine (tag, seq, group_id, reason, blob, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![&tag[..], seq as i64, group_id, reason, blob, now],
        )?;
        self.conn.execute(
            "DELETE FROM quarantine WHERE rowid NOT IN
               (SELECT rowid FROM quarantine ORDER BY at DESC, rowid DESC LIMIT ?1)",
            params![QUARANTINE_CAP],
        )?;
        Ok(())
    }

    /// The quarantined blobs, newest first: (group_id?, seq, reason, at).
    #[allow(clippy::type_complexity)]
    pub fn quarantine_list(&self) -> Result<Vec<(Option<Vec<u8>>, u64, String, i64)>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT group_id, seq, reason, at FROM quarantine ORDER BY at DESC, rowid DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, Option<Vec<u8>>>(0)?,
                r.get::<_, i64>(1)? as u64,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn quarantine_count(&self) -> Result<u64, CoreError> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM quarantine", [], |r| {
                r.get::<_, i64>(0)
            })? as u64)
    }

    /// Just the mailbox ADDRESSES a group listens on. Reads no secret, so it needs
    /// no device key and cannot fail on a sealed row. `wake_tags` calls this on
    /// every tick across every group and only ever wanted the tags — paying an AEAD
    /// open per row for a value it discarded would put the device's whole
    /// wake/subscribe set behind the at-rest key for no reason.
    pub fn epoch_tag_addresses(&self, group_id: &[u8]) -> Result<Vec<[u8; 32]>, CoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT tag FROM epoch_tag WHERE group_id=?1 ORDER BY epoch")?;
        let rows = stmt.query_map(params![group_id], |r| r.get::<_, Vec<u8>>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(
                r?.try_into()
                    .map_err(|_| CoreError::Directory("epoch tag width".into()))?,
            );
        }
        Ok(out)
    }

    /// Every (epoch, tag, secret) this device has held for a group — the mailboxes
    /// `sync` re-drains so a message that raced a superseded epoch is never lost.
    ///
    /// PER-ROW TOLERANT, DELIBERATELY. This read sits in front of `sync_group` steps
    /// (b) through (d), so returning `Err` for the whole group stops its outbox
    /// flushing and its log folding — on every tick, forever, behind a log line. One
    /// corrupt 76-byte blob must not be able to do that. So a row whose AEAD check
    /// fails is SKIPPED — it is undrainable by definition (a past epoch secret cannot
    /// be re-derived: mls-rs keeps enough to DECRYPT a past epoch, never enough to
    /// re-EXPORT from it), and forfeiting one superseded mailbox is a far smaller
    /// loss than wedging the group.
    ///
    /// SKIPPED, NOT DELETED. A failed AEAD check does not prove the row is rubbish —
    /// it equally means this is the WRONG key, and the right one may come back (an
    /// unlucky Keychain read, a restore, a re-signed build). Deleting would foreclose
    /// that; skipping costs only that the row lingers until `EPOCH_RETENTION` prunes
    /// it. Nothing here ever destroys key material it merely failed to open.
    ///
    /// A MISSING KEY IS NOT CORRUPTION EITHER. With no key at all the error
    /// propagates untouched and the caller is told plainly.
    pub fn epoch_tags(&self, group_id: &[u8]) -> Result<Vec<EpochTag>, CoreError> {
        // Collect first so the statement is dropped before any DELETE runs.
        let raw: Vec<(u64, Vec<u8>, Vec<u8>)> = {
            let mut stmt = self.conn.prepare(
                "SELECT epoch, tag, secret FROM epoch_tag WHERE group_id=?1 ORDER BY epoch",
            )?;
            let rows = stmt.query_map(params![group_id], |r| {
                Ok((
                    r.get::<_, i64>(0)? as u64,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                ))
            })?;
            let mut v = Vec::new();
            for r in rows {
                v.push(r?);
            }
            v
        };

        let mut out = Vec::new();
        for (epoch, t, s) in raw {
            let tag: [u8; 32] = match t.try_into() {
                Ok(t) => t,
                Err(_) => continue,
            };
            match Self::open_epoch_secret(&Self::ctx_epoch_secret(group_id, epoch), s) {
                Ok(secret) => out.push((epoch, tag, secret)),
                // Transient: no key to try. Say so and change nothing.
                Err(CoreError::AtRestKeyUnavailable) => return Err(CoreError::AtRestKeyUnavailable),
                Err(e) => {
                    tracing::warn!(
                        group = %hex::encode(group_id), epoch, error = %e,
                        "epoch_tag secret will not open — skipping this superseded \
                         mailbox; the row is left alone in case the key returns"
                    );
                }
            }
        }
        Ok(out)
    }

    // ---- inbox cursor (per transport: see the schema note on inbox_cursor_v2) ----
    pub fn cursor(&self, tag: &[u8; 32], source: &str) -> Result<u64, CoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT last_seen_id FROM inbox_cursor_v2 WHERE tag=?1 AND source=?2",
                params![&tag[..], source],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0) as u64)
    }

    /// Never rewinds — a cursor only ever moves forward, so an out-of-order or duplicate delivery
    /// cannot make a mailbox re-read what it has already processed.
    pub fn advance_cursor(
        &self,
        tag: &[u8; 32],
        source: &str,
        last_seen_id: u64,
    ) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT INTO inbox_cursor_v2 (tag, source, last_seen_id) VALUES (?1, ?2, ?3)
             ON CONFLICT(tag, source) DO UPDATE SET last_seen_id=MAX(last_seen_id, excluded.last_seen_id)",
            params![&tag[..], source, last_seen_id as i64],
        )?;
        Ok(())
    }

    // ---- delta log (source of truth) ----
    /// INSERT OR IGNORE => idempotent dedup by (group_id, delta_id).
    /// Returns true if newly appended.
    pub fn append_delta(
        &self,
        group_id: &[u8],
        delta_id: &[u8; 32],
        author_pk: &[u8; 32],
        envelope: &[u8],
        now: i64,
    ) -> Result<bool, CoreError> {
        self.append_delta_signed(group_id, delta_id, author_pk, envelope, None, now)
    }

    /// As [`append_delta`](Self::append_delta), carrying the author's own
    /// signature over the delta (see [`crate::delta_sig`]).
    ///
    /// `sig` is an OPTION because it genuinely is one: no build sends a signature
    /// with a delta today, so every delta received from a peer is stored without
    /// one (K-36), and every row written before this column existed has none. The distinction that matters is not
    /// signed/unsigned but PROVEN/UNPROVEN — a caller reconstructing history from
    /// a shared blob must refuse an unproven row, while a caller folding this
    /// device's own live log may accept one, because MLS already authenticated
    /// it on the way in.
    pub fn append_delta_signed(
        &self,
        group_id: &[u8],
        delta_id: &[u8; 32],
        author_pk: &[u8; 32],
        envelope: &[u8],
        sig: Option<&[u8; 64]>,
        now: i64,
    ) -> Result<bool, CoreError> {
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO delta_log              (group_id, delta_id, author_pk, envelope, author_sig, received_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                group_id,
                &delta_id[..],
                &author_pk[..],
                envelope,
                sig.map(|s| &s[..]),
                now
            ],
        )?;
        Ok(n == 1)
    }

    /// A row that came in a history bundle, stored with the admitter it came `via` (O-75).
    /// A row already held is ignored, as a direct one is: `false`.
    #[allow(clippy::too_many_arguments)]
    pub fn append_delta_via(
        &self,
        group_id: &[u8],
        delta_id: &[u8; 32],
        author_pk: &[u8; 32],
        envelope: &[u8],
        sig: &[u8; 64],
        via: &[u8; 32],
        now: i64,
    ) -> Result<bool, CoreError> {
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO delta_log (group_id, delta_id, author_pk, envelope, author_sig, received_at, via)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![group_id, &delta_id[..], &author_pk[..], envelope, &sig[..], now, &via[..]],
        )?;
        Ok(n == 1)
    }

    /// The admitter a held row came via, or None for a row its author's own message brought.
    pub fn delta_via(&self, group_id: &[u8], delta_id: &[u8; 32]) -> Result<Option<[u8; 32]>, CoreError> {
        let v: Option<Vec<u8>> = self
            .conn
            .query_row("SELECT via FROM delta_log WHERE group_id = ?1 AND delta_id = ?2", params![group_id, &delta_id[..]], |r| r.get(0))
            .optional()?
            .flatten();
        Ok(v.and_then(|b| b.try_into().ok()))
    }

    /// Who committed the Add that let this device into `group_id`, recorded at its Welcome.
    pub fn set_admitted_by(&self, group_id: &[u8], committer: &[u8; 32], now: i64) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO admitted_by (group_id, committer, at) VALUES (?1, ?2, ?3)",
            params![group_id, &committer[..], now],
        )?;
        Ok(())
    }

    pub fn admitted_by(&self, group_id: &[u8]) -> Result<Option<[u8; 32]>, CoreError> {
        let v: Option<Vec<u8>> = self
            .conn
            .query_row("SELECT committer FROM admitted_by WHERE group_id = ?1", params![group_id], |r| r.get(0))
            .optional()?;
        Ok(v.and_then(|b| b.try_into().ok()))
    }

    /// A history bundle for an object not joined yet, kept until its Welcome is.
    pub fn hold_history(&self, object: &[u8], bundle: &[u8], now: i64) -> Result<(), CoreError> {
        self.conn.execute("INSERT INTO held_history (object, bundle, at) VALUES (?1, ?2, ?3)", params![object, bundle, now])?;
        Ok(())
    }

    /// Every held bundle: (id, object, bundle, held since).
    pub fn held_history(&self) -> Result<Vec<(i64, Vec<u8>, Vec<u8>, i64)>, CoreError> {
        let mut stmt = self.conn.prepare("SELECT id, object, bundle, at FROM held_history ORDER BY id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn drop_held_history(&self, id: i64) -> Result<(), CoreError> {
        self.conn.execute("DELETE FROM held_history WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// As [`load_log`](Self::load_log), carrying each row's authorship signature
    /// where one was recorded. Used by the archive, which travels: a reader that
    /// did not witness these deltas arrive needs the proof, and a reader of its
    /// own live log does not.
    pub fn load_log_signed(
        &self,
        group_id: &[u8],
    ) -> Result<Vec<(crate::object::MemberId, Vec<u8>, Option<[u8; 64]>)>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT author_pk, envelope, author_sig FROM delta_log              WHERE group_id=?1 ORDER BY delta_id",
        )?;
        let rows = stmt.query_map(params![group_id], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, Option<Vec<u8>>>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (a, e, sig) = r?;
            let author: crate::object::MemberId = a
                .try_into()
                .map_err(|_| CoreError::Directory("author width".into()))?;
            let sig = match sig {
                None => None,
                Some(b) => Some(
                    <[u8; 64]>::try_from(&b[..])
                        .map_err(|_| CoreError::Directory("signature width".into()))?,
                ),
            };
            out.push((author, e, sig));
        }
        Ok(out)
    }

    /// (author_pk, envelope) for every delta on an object, deterministic order.
    /// (author, envelope) for every delta on an object, in order, THE FOLD'S READ.
    ///
    /// A ROW'S SIGNATURE IS CHECKED BEFORE IT IS FOLDED (A-10; SEC-35, RX.8): against
    /// the id the envelope computes, never the id the row claims. A row whose
    /// signature does not prove its author makes the whole read fail, in words that
    /// name the signature, so the object is named by `noncompliant_objects` rather
    /// than folded with a Delta nobody can vouch for. A row stored before signatures
    /// were kept for peers' Deltas (a build that predates A-10) carries none, and
    /// folds as it always has: no Delta already written is dropped.
    pub fn load_log(&self, group_id: &[u8]) -> Result<AuthoredLog, CoreError> {
        use sha2::{Digest, Sha256};
        let mut out = Vec::new();
        for (author, envelope, sig) in self.load_log_signed(group_id)? {
            if let Some(sig) = sig {
                let mut h = Sha256::new();
                h.update(group_id);
                h.update(author);
                h.update(sig);
                h.update(&envelope);
                let key: [u8; 32] = h.finalize().into();
                let known = self.verified.lock().map(|v| v.contains(&key)).unwrap_or(false);
                if !known {
                    let did = crate::coordinator::decode_delta(&envelope)?.id();
                    crate::delta_sig::verify_delta(group_id, &author, &did, &sig).map_err(|_| {
                        CoreError::Identity(format!(
                            "delta {} in this log claims author {} and its signature does not prove it (A-10)",
                            hex::encode(&did[..6]),
                            hex::encode(&author[..6])
                        ))
                    })?;
                    if let Ok(mut v) = self.verified.lock() {
                        v.insert(key);
                    }
                }
            }
            out.push((author, envelope));
        }
        Ok(out)
    }

    /// MY un-delivered deltas on a group, oldest first — the outbox to flush once
    /// the relay is reachable again. Returns (delta_id, envelope) per queued delta.
    pub fn pending_outbox(
        &self,
        group_id: &[u8],
        author_pk: &[u8; 32],
    ) -> Result<AuthoredLog, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT delta_id, envelope FROM delta_log
             WHERE group_id=?1 AND author_pk=?2 AND delivered=0 ORDER BY delta_id",
        )?;
        let rows = stmt.query_map(params![group_id, &author_pk[..]], |r| {
            Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (d, e) = r?;
            out.push((
                d.try_into()
                    .map_err(|_| CoreError::Directory("delta_id width".into()))?,
                e,
            ));
        }
        Ok(out)
    }

    // ---- the spine's outbox ----

    /// Queue one sealed spine entry at the next free index of `gen`.
    ///
    /// Returns the index it took. The PRIMARY KEY is the claim: two sessions of
    /// one account that both mint at the same moment cannot both take an index,
    /// and the loser is told rather than silently overwriting. That matters more
    /// here than in the delta outbox — a delta that lands twice is a duplicate the
    /// fold drops, whereas two entries at one address would make the spine name
    /// one object and lose the other.
    pub fn queue_spine_entry(
        &self,
        gen: u32,
        index: u64,
        address: &[u8; 32],
        blob: &[u8],
    ) -> Result<(), CoreError> {
        self.conn
            .execute(
                "INSERT INTO spine_outbox (gen, idx, address, blob, delivered) \
                 VALUES (?1, ?2, ?3, ?4, 0)",
                params![gen as i64, index as i64, &address[..], blob],
            )
            .map_err(|e| {
                CoreError::Directory(format!(
                    "spine index {index} of generation {gen} is already taken: {e}"
                ))
            })?;
        Ok(())
    }

    /// This install's lease-holder id, minted on first ask and kept.
    /// Did this state publish this blob? The sent ledger's one question.
    pub fn was_sent(&self, blob_b64: &str) -> Result<bool, CoreError> {
        Ok(self
            .conn
            .query_row("SELECT 1 FROM own_sent WHERE hash = ?1", [&sent_hash(blob_b64)[..]], |_| Ok(()))
            .optional()?
            .is_some())
    }

    pub fn device_id(&self) -> Result<[u8; 32], CoreError> {
        let got: Option<Vec<u8>> = self
            .conn
            .query_row("SELECT id FROM device WHERE k=0", [], |r| r.get(0))
            .optional()?;
        if let Some(v) = got {
            if let Ok(a) = <[u8; 32]>::try_from(v.as_slice()) {
                return Ok(a);
            }
        }
        let mut id = [0u8; 32];
        crate::head::getrandom_fill(&mut id)?;
        self.conn.execute("INSERT OR REPLACE INTO device (k, id) VALUES (0, ?1)", params![&id[..]])?;
        Ok(id)
    }

    /// Record a lease this device now holds at `cell`.
    pub fn put_lease(&self, group_id: &[u8], pool: &[u8; 32], cell: u64, until: i64) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT INTO pool_lease (group_id, pool, cell, until, fenced) VALUES (?1, ?2, ?3, ?4, 0) \
             ON CONFLICT(group_id, pool) DO UPDATE SET cell=excluded.cell, until=excluded.until",
            params![group_id, &pool[..], cell as i64, until],
        )?;
        Ok(())
    }

    /// Every lease this device holds: (group, pool, cell, until, fenced).
    pub fn leases(&self) -> Result<Vec<(Vec<u8>, [u8; 32], u64, i64, bool)>, CoreError> {
        let mut stmt = self.conn.prepare("SELECT group_id, pool, cell, until, fenced FROM pool_lease")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (g, p, c, u, f) = r?;
            if let Ok(p) = <[u8; 32]>::try_from(p.as_slice()) {
                out.push((g, p, c as u64, u, f != 0));
            }
        }
        Ok(out)
    }

    pub fn mark_fenced(&self, group_id: &[u8], pool: &[u8; 32]) -> Result<(), CoreError> {
        self.conn.execute(
            "UPDATE pool_lease SET fenced=1 WHERE group_id=?1 AND pool=?2",
            params![group_id, &pool[..]],
        )?;
        Ok(())
    }

    /// Objects in which this device was fenced out of the leaf it spoke through.
    pub fn fenced_groups(&self) -> Result<Vec<Vec<u8>>, CoreError> {
        let mut stmt = self.conn.prepare("SELECT DISTINCT group_id FROM pool_lease WHERE fenced=1")?;
        let rows = stmt.query_map([], |r| r.get::<_, Vec<u8>>(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Record an entry found on the relay at `index` — written by another device of
    /// this account, or by this one before its directory was lost. It is delivered by
    /// definition, and knowing it keeps [`Self::next_spine_index`] past it.
    pub fn record_walked_spine_entry(
        &self,
        gen: u32,
        index: u64,
        address: &[u8; 32],
        blob: &[u8],
    ) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT OR IGNORE INTO spine_outbox (gen, idx, address, blob, delivered) \
             VALUES (?1, ?2, ?3, ?4, 1)",
            params![gen as i64, index as i64, &address[..], blob],
        )?;
        Ok(())
    }

    /// Take every UNDELIVERED entry of `gen` at or after `from` out of the queue, in
    /// index order — for re-placing them past an index another device won.
    /// Each with the index it was sealed at: walked entries from another device can
    /// sit between them, so the queue is not contiguous.
    pub fn take_pending_spine_from(&self, gen: u32, from: u64) -> Result<Vec<(u64, Vec<u8>)>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT idx, blob FROM spine_outbox WHERE gen=?1 AND idx>=?2 AND delivered=0 ORDER BY idx",
        )?;
        let blobs: Vec<(u64, Vec<u8>)> = stmt
            .query_map(params![gen as i64, from as i64], |r| {
                Ok((r.get::<_, i64>(0)? as u64, r.get::<_, Vec<u8>>(1)?))
            })?
            .collect::<Result<_, _>>()?;
        self.conn.execute(
            "DELETE FROM spine_outbox WHERE gen=?1 AND idx>=?2 AND delivered=0",
            params![gen as i64, from as i64],
        )?;
        Ok(blobs)
    }

    /// The next index to mint into for `gen` — one past the highest taken.
    ///
    /// Taken, not delivered: an entry that is queued and unpublished still owns
    /// its index, or a retry would mint a second entry for the same object.
    pub fn next_spine_index(&self, gen: u32) -> Result<u64, CoreError> {
        let n: Option<i64> = self.conn.query_row(
            "SELECT MAX(idx) FROM spine_outbox WHERE gen=?1",
            params![gen as i64],
            |r| r.get(0),
        )?;
        Ok(n.map_or(0, |m| m as u64 + 1))
    }

    /// Entries minted and not yet published, oldest first.
    pub fn pending_spine(&self, gen: u32) -> Result<Vec<(u64, [u8; 32], Vec<u8>)>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT idx, address, blob FROM spine_outbox \
             WHERE gen=?1 AND delivered=0 ORDER BY idx",
        )?;
        let rows = stmt.query_map(params![gen as i64], |r| {
            Ok((
                r.get::<_, i64>(0)? as u64,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (i, a, b) = r?;
            out.push((
                i,
                a.try_into()
                    .map_err(|_| CoreError::Directory("spine address width".into()))?,
                b,
            ));
        }
        Ok(out)
    }

    /// Entries this account has PUBLISHED, sealed as they are stored.
    ///
    /// The caller opens them; this table holds no plaintext, so it cannot answer
    /// "which object is this" itself and does not pretend to.
    pub fn delivered_spine(&self, gen: u32) -> Result<Vec<(u64, Vec<u8>)>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT idx, blob FROM spine_outbox WHERE gen=?1 AND delivered=1 ORDER BY idx",
        )?;
        let rows = stmt.query_map(params![gen as i64], |r| {
            Ok((r.get::<_, i64>(0)? as u64, r.get::<_, Vec<u8>>(1)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (i, b) = r?;
            out.push((i, b));
        }
        Ok(out)
    }

    /// Mark one spine entry published.
    pub fn mark_spine_delivered(&self, gen: u32, index: u64) -> Result<(), CoreError> {
        self.conn.execute(
            "UPDATE spine_outbox SET delivered=1 WHERE gen=?1 AND idx=?2",
            params![gen as i64, index as i64],
        )?;
        Ok(())
    }

    /// Mark one of our deltas relay-ack'd (no longer in the outbox).
    pub fn mark_delivered(&self, group_id: &[u8], delta_id: &[u8; 32]) -> Result<(), CoreError> {
        self.conn.execute(
            "UPDATE delta_log SET delivered=1 WHERE group_id=?1 AND delta_id=?2",
            params![group_id, &delta_id[..]],
        )?;
        Ok(())
    }

    /// (group_id, most recent `received_at`) for every object with deltas — the
    /// recency index behind `GroupObjectStore::by_recency` / `active_since`. One
    /// grouped query rather than a per-object scan.
    pub fn log_recency(&self) -> Result<Vec<(Vec<u8>, i64)>, CoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT group_id, MAX(received_at) FROM delta_log GROUP BY group_id")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Count of an author's deltas on a group — the next `gen` for that author.
    pub fn author_delta_count(
        &self,
        group_id: &[u8],
        author_pk: &[u8; 32],
    ) -> Result<u64, CoreError> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM delta_log WHERE group_id=?1 AND author_pk=?2",
            params![group_id, &author_pk[..]],
            |r| r.get(0),
        )?;
        Ok(n as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Directory {
        let conn = Connection::open_in_memory().unwrap();
        Directory::init(&conn).unwrap();
        Directory::with(conn)
    }

    /// O-69: the fold generation moves on every change to a row the fold reads (a Delta
    /// in, out, or its author or signature changed; the roster; the owner history; the
    /// owner), and not on the relay's acknowledgement or another object's writes. A roster
    /// rewritten unchanged, as every commit rewrites it, does move it: a miss, never a
    /// stale hit.
    #[test]
    fn the_fold_generation_moves_exactly_when_a_fold_could() {
        let dir = mem();
        let (g, other) = (b"g-one".to_vec(), b"g-two".to_vec());
        let (a, b) = ([1u8; 32], [2u8; 32]);
        let gen = |d: &Directory| d.fold_gen(&g).unwrap();
        assert_eq!(gen(&dir), 0, "never written");
        dir.put_group(&g, "forum", &a, None).unwrap();
        let mut last = gen(&dir);
        let mut moved = |d: &Directory, why: &str| {
            let now = gen(d);
            assert!(now > last, "{why}: the generation did not move ({last} -> {now})");
            last = now;
        };
        dir.append_delta(&g, &[9; 32], &a, b"env-1", 0).unwrap();
        moved(&dir, "a Delta in");
        dir.conn.execute("UPDATE delta_log SET author_sig = x'01' WHERE group_id = ?1", params![g]).unwrap();
        moved(&dir, "a signature changed");
        dir.set_group_members(&g, &[a, b]).unwrap();
        moved(&dir, "the roster");
        dir.record_owner(&g, 3, &b).unwrap();
        moved(&dir, "the owner history");
        dir.conn.execute("UPDATE groups SET owner_pk = ?2 WHERE group_id = ?1", params![g, &b[..]]).unwrap();
        moved(&dir, "the owner");
        dir.conn.execute("DELETE FROM delta_log WHERE group_id = ?1", params![g]).unwrap();
        moved(&dir, "a Delta out");

        dir.append_delta(&g, &[8; 32], &a, b"env-2", 0).unwrap();
        let before = gen(&dir);
        dir.mark_delivered(&g, &[8; 32]).unwrap();
        assert_eq!(gen(&dir), before, "the relay's acknowledgement is not a fold input");
        dir.put_group(&other, "forum", &a, None).unwrap();
        dir.append_delta(&other, &[7; 32], &a, b"env-3", 0).unwrap();
        assert_eq!(gen(&dir), before, "another object's writes");
    }

    /// G moves on each row a graph shows, by its own trigger, and on nothing else (FC-12):
    /// each assertion below fails with its trigger dropped, and each "still" with a trigger
    /// widened to a column no graph reads.
    #[test]
    fn the_store_generation_moves_exactly_when_a_graph_could() {
        let dir = mem();
        let (g, a) = (b"g-one".to_vec(), [1u8; 32]);
        let mut last = dir.generation().unwrap();
        let mut moved = |d: &Directory, why: &str, should: bool| {
            let now = d.generation().unwrap();
            if should {
                assert_ne!(now, last, "{why}: G did not move");
            } else {
                assert_eq!(now, last, "{why}: G moved");
            }
            last = now;
        };
        dir.put_group(&g, "forum", &a, None).unwrap();
        moved(&dir, "an object in the list", true);
        dir.append_delta(&g, &[9; 32], &a, b"env-1", 0).unwrap();
        moved(&dir, "a fold generation, first", true);
        dir.append_delta(&g, &[8; 32], &a, b"env-2", 0).unwrap();
        moved(&dir, "a fold generation, again", true);
        dir.mark_delivered(&g, &[8; 32]).unwrap();
        moved(&dir, "still: the relay's acknowledgement", false);
        dir.conn.execute("UPDATE groups SET kind = 'group' WHERE group_id = ?1", params![g]).unwrap();
        moved(&dir, "an object's kind", true);
        dir.conn.execute("UPDATE groups SET arc_url = 'wss://x' WHERE group_id = ?1", params![g]).unwrap();
        moved(&dir, "still: an object's Arc", false);
        dir.mark_departed(&g, &a, 1).unwrap();
        moved(&dir, "a departure", true);
        dir.conn.execute("DELETE FROM groups WHERE group_id = ?1", params![g]).unwrap();
        moved(&dir, "an object out of the list", true);
        dir.conn.execute("INSERT INTO epoch_tag (group_id, epoch, tag, secret) VALUES (?1, 4, x'00', x'00')", params![g]).unwrap();
        moved(&dir, "a new epoch", true);
        dir.conn.execute("UPDATE epoch_tag SET secret = x'01' WHERE group_id = ?1", params![g]).unwrap();
        moved(&dir, "still: an epoch's key re-sealed", false);
        dir.conn.execute("INSERT INTO spine_outbox (gen, idx, address, blob) VALUES (1, 0, x'00', x'00')", []).unwrap();
        moved(&dir, "a spine entry", true);
        dir.conn.execute("UPDATE spine_outbox SET delivered = 1", []).unwrap();
        moved(&dir, "still: a spine entry delivered", false);
        dir.conn.execute("DELETE FROM spine_outbox", []).unwrap();
        moved(&dir, "a spine entry withdrawn", true);
        dir.conn
            .execute("INSERT INTO peers (peer_id, display_name, status, first_seen, last_seen) VALUES (x'aa', 'ab', 'pending', 0, 0)", [])
            .unwrap();
        moved(&dir, "a peer", true);
        dir.conn.execute("UPDATE peers SET display_name = 'cd'", []).unwrap();
        moved(&dir, "a peer renamed, the same length", true);
        dir.conn.execute("UPDATE peers SET status = 'connected'", []).unwrap();
        moved(&dir, "a peer's standing", true);
        dir.conn.execute("UPDATE peers SET identity_group_id = x'bb'", []).unwrap();
        moved(&dir, "a peer's identity object", true);
        dir.conn.execute("UPDATE peers SET last_seen = 9, why = 'met'", []).unwrap();
        moved(&dir, "still: a peer's last-seen", false);
        dir.conn.execute("DELETE FROM peers", []).unwrap();
        moved(&dir, "a peer gone", true);
        dir.advance_cursor(&[3; 32], "relay", 7).unwrap();
        moved(&dir, "still: a cursor", false);
    }

    /// Every open draws its own nonce: a restore that brings back an older generation
    /// never meets a key made before it (FC-5).
    #[test]
    fn every_open_draws_its_own_nonce() {
        assert_ne!(mem().nonce, mem().nonce);
    }

    #[test]
    fn open_is_idempotent_and_delta_log_dedups() {
        let dir = mem();
        // re-init is a no-op
        Directory::init(&dir.conn).unwrap();

        let g = [9u8; 8];
        let did = [1u8; 32];
        let author = [2u8; 32];
        assert!(dir.append_delta(&g, &did, &author, b"env", 0).unwrap());
        // same delta id => idempotent, not newly appended
        assert!(!dir.append_delta(&g, &did, &author, b"env", 0).unwrap());
        assert_eq!(dir.load_log(&g).unwrap().len(), 1);
        assert_eq!(dir.author_delta_count(&g, &author).unwrap(), 1);
    }

    #[test]
    fn outbox_holds_my_undelivered_deltas_and_clears_on_ack() {
        let dir = mem();
        let g = [1u8; 8];
        let me = [2u8; 32];
        let peer = [3u8; 32];
        let (d1, d2, dp) = ([10u8; 32], [11u8; 32], [12u8; 32]);
        // two of mine (queued) + one received from a peer
        dir.append_delta(&g, &d1, &me, b"e1", 0).unwrap();
        dir.append_delta(&g, &d2, &me, b"e2", 0).unwrap();
        dir.append_delta(&g, &dp, &peer, b"ep", 0).unwrap();

        let ob = dir.pending_outbox(&g, &me).unwrap();
        assert_eq!(ob.len(), 2, "both my deltas are queued");
        assert!(
            ob.iter().all(|(id, _)| id == &d1 || id == &d2),
            "a received (peer-authored) delta is never in MY outbox"
        );

        // a relay ack clears one from the outbox
        dir.mark_delivered(&g, &d1).unwrap();
        let ob = dir.pending_outbox(&g, &me).unwrap();
        assert_eq!(
            ob,
            vec![(d2, b"e2".to_vec())],
            "only the still-unacked delta remains"
        );
    }

    #[test]
    fn peer_status_machine_and_cursor() {
        let dir = mem();
        let peer = [7u8; 32];
        dir.upsert_peer(&peer, "bob", PeerStatus::PendingOut, None, None, 0)
            .unwrap();
        assert_eq!(dir.peer_status(&peer).unwrap(), PeerStatus::PendingOut);
        dir.set_peer_status(&peer, PeerStatus::Connected).unwrap();
        assert_eq!(dir.peer_status(&peer).unwrap(), PeerStatus::Connected);

        let tag = [3u8; 32];
        assert_eq!(dir.cursor(&tag, "relay").unwrap(), 0);
        dir.advance_cursor(&tag, "relay", 5).unwrap();
        dir.advance_cursor(&tag, "relay", 2).unwrap(); // never rewinds
        assert_eq!(dir.cursor(&tag, "relay").unwrap(), 5);
        // A second transport keeps its OWN position on the same tag — the whole point of the
        // split. Before it, a mesh delivery could drag the relay cursor forward and skip mail.
        assert_eq!(dir.cursor(&tag, "mesh").unwrap(), 0);
        dir.advance_cursor(&tag, "mesh", 3).unwrap();
        assert_eq!(dir.cursor(&tag, "mesh").unwrap(), 3);
        assert_eq!(dir.cursor(&tag, "relay").unwrap(), 5, "untouched");
    }

    #[test]
    fn group_arc_set_at_mint_swapped_and_never_clobbered_by_reput() {
        let dir = mem();
        let g = [5u8; 8];
        let owner = [1u8; 32];
        // the creator stamps the governing Arc at mint.
        dir.put_group(&g, "forum", &owner, Some("wss://uk.arc.example"))
            .unwrap();
        assert_eq!(
            dir.group_arc(&g).unwrap().as_deref(),
            Some("wss://uk.arc.example")
        );
        // an idempotent re-put WITHOUT an arc must never drop the configured one (COALESCE).
        dir.put_group(&g, "forum", &owner, None).unwrap();
        assert_eq!(
            dir.group_arc(&g).unwrap().as_deref(),
            Some("wss://uk.arc.example"),
            "a re-put without an arc must not clobber the object's governing Arc"
        );
        // an explicit swap moves the jurisdiction.
        dir.set_group_arc(&g, "wss://asia.arc.example").unwrap();
        assert_eq!(
            dir.group_arc(&g).unwrap().as_deref(),
            Some("wss://asia.arc.example")
        );
        // an object joined before we learned its Arc reports None (routing falls back).
        let h = [6u8; 8];
        dir.put_group(&h, "forum", &owner, None).unwrap();
        assert_eq!(dir.group_arc(&h).unwrap(), None);
        // swapping a group we don't hold is loud, never a silent no-op.
        assert!(dir.set_group_arc(&[0xFFu8; 8], "wss://x.example").is_err());
    }

    #[test]
    fn epoch_tags_roundtrip_ordered_and_idempotent() {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        crate::atrest::clear_device_key();
        std::env::remove_var("PACIFIC_ATREST_KEY");
        let dir = mem();
        let g = [5u8; 8];
        let (t0, s0) = ([0xAAu8; 32], [0x11u8; 32]);
        let (t1, s1) = ([0xBBu8; 32], [0x22u8; 32]);
        dir.record_epoch_tag(&g, 0, &t0, &s0).unwrap();
        dir.record_epoch_tag(&g, 1, &t1, &s1).unwrap();
        // idempotent, first-write-wins: re-recording epoch 0 must not clobber it.
        dir.record_epoch_tag(&g, 0, &[0xFFu8; 32], &[0xFFu8; 32])
            .unwrap();
        let got: Vec<(u64, [u8; 32], [u8; 32])> = dir
            .epoch_tags(&g)
            .unwrap()
            .into_iter()
            .map(|(e, t, s)| (e, t, *s))
            .collect();
        assert_eq!(
            got,
            vec![(0u64, t0, s0), (1u64, t1, s1)],
            "every held epoch tag, ascending, first-write-wins"
        );
        // a different group is isolated.
        assert!(dir.epoch_tags(&[9u8; 8]).unwrap().is_empty());
    }

    /// The column this whole change exists for. A device key means the secret is
    /// SEALED on disk, opens transparently, and — the part that matters — a keyless
    /// writer can never put plaintext back over it.
    #[test]
    fn epoch_secret_is_sealed_at_rest_and_never_downgrades() {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        crate::atrest::clear_device_key();
        std::env::set_var("PACIFIC_ATREST_KEY", "cd".repeat(32));

        let dir = mem();
        let g = [7u8; 8];
        let (tag, secret) = ([0xA1u8; 32], [0x5Cu8; 32]);
        dir.record_epoch_tag(&g, 3, &tag, &secret).unwrap();

        // On disk it is a sealed envelope, not the 32 raw bytes.
        let stored: Vec<u8> = dir
            .conn
            .query_row("SELECT secret FROM epoch_tag WHERE group_id=?1", params![&g[..]], |r| r.get(0))
            .unwrap();
        assert!(crate::atrest::is_sealed(&stored), "must be sealed on disk");
        assert_ne!(stored, secret.to_vec(), "the raw secret must not be on disk");
        assert_eq!(*dir.epoch_tags(&g).unwrap()[0].2, secret, "and it opens back");

        // A keyless writer must not overwrite a sealed row with plaintext.
        crate::atrest::clear_device_key();
        std::env::remove_var("PACIFIC_ATREST_KEY");
        dir.record_epoch_tag(&g, 3, &tag, &[0xEEu8; 32]).unwrap();
        let after: Vec<u8> = dir
            .conn
            .query_row("SELECT secret FROM epoch_tag WHERE group_id=?1", params![&g[..]], |r| r.get(0))
            .unwrap();
        assert_eq!(after, stored, "a sealed row never degrades to plaintext");

        // …and with no key, reading it is a LOUD, TRANSIENT error — not a deletion.
        assert!(matches!(
            dir.epoch_tags(&g),
            Err(CoreError::AtRestKeyUnavailable)
        ));
        let survived: i64 = dir
            .conn
            .query_row("SELECT COUNT(*) FROM epoch_tag WHERE group_id=?1", params![&g[..]], |r| r.get(0))
            .unwrap();
        assert_eq!(survived, 1, "a missing key must never delete a good row");

        crate::atrest::clear_device_key();
        std::env::remove_var("PACIFIC_ATREST_KEY");
    }

    /// A genuinely corrupt row is dropped rather than wedging the group: `epoch_tags`
    /// feeds `sync_group` steps (b)-(d), so one bad blob must not stop the outbox
    /// flushing forever.
    #[test]
    fn a_corrupt_epoch_secret_is_dropped_not_fatal() {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        crate::atrest::clear_device_key();
        std::env::set_var("PACIFIC_ATREST_KEY", "ef".repeat(32));

        let dir = mem();
        let g = [8u8; 8];
        dir.record_epoch_tag(&g, 1, &[0xB1u8; 32], &[0x11u8; 32]).unwrap();
        dir.record_epoch_tag(&g, 2, &[0xB2u8; 32], &[0x22u8; 32]).unwrap();

        // Corrupt epoch 1's ciphertext, leaving the magic intact so it still parses
        // as sealed and fails the AEAD tag check.
        let mut bad: Vec<u8> = dir
            .conn
            .query_row("SELECT secret FROM epoch_tag WHERE group_id=?1 AND epoch=1", params![&g[..]], |r| r.get(0))
            .unwrap();
        let n = bad.len();
        bad[n - 1] ^= 0xFF;
        dir.conn
            .execute("UPDATE epoch_tag SET secret=?1 WHERE group_id=?2 AND epoch=1", params![&bad[..], &g[..]])
            .unwrap();

        let got = dir.epoch_tags(&g).unwrap();
        assert_eq!(got.len(), 1, "the good row still comes back");
        assert_eq!(got[0].0, 2);
        let left: i64 = dir
            .conn
            .query_row("SELECT COUNT(*) FROM epoch_tag WHERE group_id=?1", params![&g[..]], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 2, "skipped, NOT deleted — a wrong key must not destroy the row");

        crate::atrest::clear_device_key();
        std::env::remove_var("PACIFIC_ATREST_KEY");
    }

    /// `wake_tags`' reader needs no key at all — that is the point of it.
    #[test]
    fn epoch_tag_addresses_needs_no_device_key() {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        crate::atrest::clear_device_key();
        std::env::set_var("PACIFIC_ATREST_KEY", "12".repeat(32));
        let dir = mem();
        let g = [3u8; 8];
        dir.record_epoch_tag(&g, 0, &[0xC0u8; 32], &[0x01u8; 32]).unwrap();
        dir.record_epoch_tag(&g, 1, &[0xC1u8; 32], &[0x02u8; 32]).unwrap();

        crate::atrest::clear_device_key();
        std::env::remove_var("PACIFIC_ATREST_KEY");
        assert_eq!(
            dir.epoch_tag_addresses(&g).unwrap(),
            vec![[0xC0u8; 32], [0xC1u8; 32]],
            "addresses read fine with no key, so the wake set never depends on one"
        );
    }
}
