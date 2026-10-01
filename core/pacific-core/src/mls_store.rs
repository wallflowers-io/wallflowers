//! SQLite-backed MLS storage providers — the real cross-process seam.
//!
//! Each `pacific <verb>` is a fresh process, so the default in-memory
//! `InMemoryGroupStateStorage` (a process-local `HashMap`) would lose the MLS
//! group the moment `pair --scan` exits. Instead we persist group state and key
//! packages to the SAME SQLite database the directory uses, and rebuild the
//! `Client` each process with these providers; group continuity is then
//! `Group::write_to_storage()` + `Client::load_group(group_id)`.
//!
//! `mls-rs` 0.55.2 is sync here (no `mls_build_async`), so these trait impls are
//! sync. rusqlite `Connection` is not `Sync`, so each store owns an
//! `Arc<Mutex<Connection>>` over the same DB path.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mls_rs::error::IntoAnyError;
use mls_rs::mls_rs_codec::{MlsDecode, MlsEncode};
use mls_rs::storage_provider::KeyPackageData;
use mls_rs::{GroupStateStorage, KeyPackageStorage};
use mls_rs_core::group::{EpochRecord, GroupState};
use rusqlite::{params, Connection, OptionalExtension};
use zeroize::Zeroizing;

/// A loud storage error. `IntoAnyError`'s default impls turn `Debug` into the
/// wrapper mls-rs expects.
#[derive(Debug)]
pub struct StoreError(pub String);

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mls store: {}", self.0)
    }
}
impl std::error::Error for StoreError {}
impl IntoAnyError for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError(e.to_string())
    }
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS mls_group_state (
  group_id BLOB PRIMARY KEY,
  data     BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS mls_epoch (
  group_id BLOB NOT NULL,
  epoch_id INTEGER NOT NULL,
  data     BLOB NOT NULL,
  PRIMARY KEY (group_id, epoch_id)
);
CREATE TABLE IF NOT EXISTS mls_key_package (
  id   BLOB PRIMARY KEY,
  data BLOB NOT NULL
);
"#;

/// Ensure the MLS storage tables exist on `path` (idempotent).
pub fn migrate(path: &Path) -> Result<(), StoreError> {
    let conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

fn open(path: &Path) -> Result<Connection, StoreError> {
    let conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

// ------------------------------------------------------- at-rest sealing ------
// The MLS group ratchet state, per-epoch secrets and key packages ARE secret key
// material. Seal each `data` BLOB under the device master key (S5) when one is
// configured; a sealed BLOB can only be reopened with the key (fail loud, no
// plaintext fallback). Lookup KEYS (group_id / epoch_id / id) stay in the clear so
// queries still work — and they are folded into the seal context, binding each
// ciphertext to its row so a sealed blob cannot be relocated to another slot.

fn ctx_group_state(group_id: &[u8]) -> Vec<u8> {
    let mut c = b"pacific/mls/group_state".to_vec();
    c.extend_from_slice(group_id);
    c
}
fn ctx_epoch(group_id: &[u8], epoch_id: u64) -> Vec<u8> {
    let mut c = b"pacific/mls/epoch".to_vec();
    c.extend_from_slice(group_id);
    c.extend_from_slice(&epoch_id.to_le_bytes());
    c
}
fn ctx_key_package(id: &[u8]) -> Vec<u8> {
    let mut c = b"pacific/mls/key_package".to_vec();
    c.extend_from_slice(id);
    c
}

/// Seal a secret BLOB for storage when a device key is configured; else plaintext.
fn seal_blob(context: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, StoreError> {
    match crate::atrest::device_key().map_err(|e| StoreError(e.to_string()))? {
        Some(key) => crate::atrest::seal_at_rest(context, plaintext, &key)
            .map_err(|e| StoreError(e.to_string())),
        None => Ok(plaintext.to_vec()),
    }
}

/// Open a stored BLOB: if sealed, require the key (fail loud); else plaintext (legacy).
fn open_blob(context: &[u8], stored: Vec<u8>) -> Result<Zeroizing<Vec<u8>>, StoreError> {
    if crate::atrest::is_sealed(&stored) {
        match crate::atrest::device_key().map_err(|e| StoreError(e.to_string()))? {
            Some(key) => crate::atrest::open_at_rest(context, &stored, &key)
                .map(Zeroizing::new)
                .map_err(|e| StoreError(e.to_string())),
            None => Err(StoreError(
                "at-rest key unavailable — a sealed MLS blob cannot be opened".into(),
            )),
        }
    } else {
        Ok(Zeroizing::new(stored))
    }
}

// --------------------------------------------------------- group state -------

#[derive(Clone)]
pub struct SqliteGroupStateStorage {
    conn: Arc<Mutex<Connection>>,
    path: PathBuf,
}

impl SqliteGroupStateStorage {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Ok(Self {
            conn: Arc::new(Mutex::new(open(path)?)),
            path: path.to_path_buf(),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, StoreError> {
        self.conn
            .lock()
            .map_err(|_| StoreError("group-state mutex poisoned".into()))
    }

    /// Visible for tests: the DB path this store writes to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// DELETE one group's MLS state — its snapshot and every epoch record, which hold
    /// the ratchet tree and the epoch secrets. RFC 9420 §12.4.2: a removed member
    /// "SHOULD promptly delete its group state and secret tree". mls-rs's storage
    /// trait has no delete, so Pacific's store provides it
    /// (membership-through-mls.md §8.3). Returns how many rows went.
    pub fn delete_group(&self, group_id: &[u8]) -> Result<usize, StoreError> {
        let conn = self.lock()?;
        let a = conn.execute("DELETE FROM mls_epoch WHERE group_id=?1", rusqlite::params![group_id])?;
        let b = conn.execute("DELETE FROM mls_group_state WHERE group_id=?1", rusqlite::params![group_id])?;
        Ok(a + b)
    }
}

impl GroupStateStorage for SqliteGroupStateStorage {
    type Error = StoreError;

    fn state(&self, group_id: &[u8]) -> Result<Option<Zeroizing<Vec<u8>>>, Self::Error> {
        let conn = self.lock()?;
        let data: Option<Vec<u8>> = conn
            .query_row(
                "SELECT data FROM mls_group_state WHERE group_id=?1",
                params![group_id],
                |r| r.get(0),
            )
            .optional()?;
        drop(conn);
        match data {
            None => Ok(None),
            Some(d) => Ok(Some(open_blob(&ctx_group_state(group_id), d)?)),
        }
    }

    fn epoch(
        &self,
        group_id: &[u8],
        epoch_id: u64,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, Self::Error> {
        let conn = self.lock()?;
        let data: Option<Vec<u8>> = conn
            .query_row(
                "SELECT data FROM mls_epoch WHERE group_id=?1 AND epoch_id=?2",
                params![group_id, epoch_id as i64],
                |r| r.get(0),
            )
            .optional()?;
        drop(conn);
        match data {
            None => Ok(None),
            Some(d) => Ok(Some(open_blob(&ctx_epoch(group_id, epoch_id), d)?)),
        }
    }

    fn write(
        &mut self,
        state: GroupState,
        epoch_inserts: Vec<EpochRecord>,
        epoch_updates: Vec<EpochRecord>,
    ) -> Result<(), Self::Error> {
        // Seal every secret BLOB BEFORE taking the DB lock — keep crypto off the mutex.
        let sealed_state = seal_blob(&ctx_group_state(&state.id), &state.data[..])?;
        let sealed_inserts: Vec<(i64, Vec<u8>)> = epoch_inserts
            .into_iter()
            .map(|e| {
                Ok((
                    e.id as i64,
                    seal_blob(&ctx_epoch(&state.id, e.id), &e.data[..])?,
                ))
            })
            .collect::<Result<_, StoreError>>()?;
        let sealed_updates: Vec<(i64, Vec<u8>)> = epoch_updates
            .into_iter()
            .map(|e| {
                Ok((
                    e.id as i64,
                    seal_blob(&ctx_epoch(&state.id, e.id), &e.data[..])?,
                ))
            })
            .collect::<Result<_, StoreError>>()?;

        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO mls_group_state (group_id, data) VALUES (?1, ?2)
             ON CONFLICT(group_id) DO UPDATE SET data=excluded.data",
            params![&state.id, &sealed_state],
        )?;
        for (epoch_id, data) in &sealed_inserts {
            tx.execute(
                "INSERT INTO mls_epoch (group_id, epoch_id, data) VALUES (?1, ?2, ?3)
                 ON CONFLICT(group_id, epoch_id) DO UPDATE SET data=excluded.data",
                params![&state.id, *epoch_id, data.as_slice()],
            )?;
        }
        for (epoch_id, data) in &sealed_updates {
            tx.execute(
                "UPDATE mls_epoch SET data=?3 WHERE group_id=?1 AND epoch_id=?2",
                params![&state.id, *epoch_id, data.as_slice()],
            )?;
        }
        // Bounded retention (crate::EPOCH_RETENTION): keep the newest N epochs'
        // secrets so late application messages still decrypt, delete the rest —
        // retention was previously UNBOUNDED here, which is a forward-secrecy
        // leak, not a feature. The directory prunes the matching mailbox tags.
        tx.execute(
            "DELETE FROM mls_epoch WHERE group_id=?1 AND epoch_id <
               (SELECT COALESCE(MAX(epoch_id),0) FROM mls_epoch WHERE group_id=?1) - ?2",
            params![&state.id, crate::EPOCH_RETENTION as i64],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn max_epoch_id(&self, group_id: &[u8]) -> Result<Option<u64>, Self::Error> {
        let conn = self.lock()?;
        let id: Option<i64> = conn
            .query_row(
                "SELECT MAX(epoch_id) FROM mls_epoch WHERE group_id=?1",
                params![group_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        Ok(id.map(|v| v as u64))
    }
}

// --------------------------------------------------------- key packages ------

#[derive(Clone)]
pub struct SqliteKeyPackageStorage {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteKeyPackageStorage {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Ok(Self {
            conn: Arc::new(Mutex::new(open(path)?)),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, StoreError> {
        self.conn
            .lock()
            .map_err(|_| StoreError("key-package mutex poisoned".into()))
    }
}

impl KeyPackageStorage for SqliteKeyPackageStorage {
    type Error = StoreError;

    fn delete(&mut self, id: &[u8]) -> Result<(), Self::Error> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM mls_key_package WHERE id=?1", params![id])?;
        Ok(())
    }

    fn insert(&mut self, id: Vec<u8>, pkg: KeyPackageData) -> Result<(), Self::Error> {
        let bytes = pkg
            .mls_encode_to_vec()
            .map_err(|e| StoreError(format!("key package encode: {e}")))?;
        let sealed = seal_blob(&ctx_key_package(&id), &bytes)?;
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO mls_key_package (id, data) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET data=excluded.data",
            params![&id, &sealed],
        )?;
        Ok(())
    }

    fn get(&self, id: &[u8]) -> Result<Option<KeyPackageData>, Self::Error> {
        let conn = self.lock()?;
        let bytes: Option<Vec<u8>> = conn
            .query_row(
                "SELECT data FROM mls_key_package WHERE id=?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?;
        drop(conn);
        match bytes {
            None => Ok(None),
            Some(b) => {
                let plain = open_blob(&ctx_key_package(id), b)?;
                let pkg = KeyPackageData::mls_decode(&mut &plain[..])
                    .map_err(|e| StoreError(format!("key package decode: {e}")))?;
                Ok(Some(pkg))
            }
        }
    }
}

// ------------------------------------------------------------- the snapshot ---
// The MLS half of the account backup, retired with the escrow (a58798c): nothing
// calls `dump` or `restore` now. `dump` reads every row of
// the three tables back to plaintext, into the SAME `Snapshot` shape the browser
// captures from its in-memory stores (`mls_mem::Snapshot::capture`), so one
// backup format serves both platforms and a phone can be restored from what a
// browser exported, or the reverse. `restore` writes a snapshot back, sealed
// exactly as the live write path seals.

/// Every row of `mls_group_state`, `mls_epoch` and `mls_key_package`, opened
/// under the device key, as one [`crate::mls_mem::Snapshot`]. SECRET IN FULL —
/// ratchet state, epoch secrets, key-package private halves; the caller seals it
/// before it goes anywhere.
///
/// A sealed row that will not open is an ERROR, not a gap: a backup with a hole
/// in it would restore as a device that silently cannot read one of its groups,
/// and `open_blob` already refuses a plaintext fallback. Rows are read under the
/// lock and opened after it — crypto stays off the mutex, as the reads do.
pub fn dump(
    gss: &SqliteGroupStateStorage,
    kps: &SqliteKeyPackageStorage,
) -> Result<crate::mls_mem::Snapshot, StoreError> {
    let (states, epochs): (Vec<(Vec<u8>, Vec<u8>)>, Vec<(Vec<u8>, u64, Vec<u8>)>) = {
        let conn = gss.lock()?;
        let mut st = conn.prepare("SELECT group_id, data FROM mls_group_state ORDER BY group_id")?;
        let states = st
            .query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut ep = conn.prepare(
            "SELECT group_id, epoch_id, data FROM mls_epoch ORDER BY group_id, epoch_id",
        )?;
        let epochs = ep
            .query_map([], |r| {
                Ok((
                    r.get::<_, Vec<u8>>(0)?,
                    r.get::<_, i64>(1)? as u64,
                    r.get::<_, Vec<u8>>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        (states, epochs)
    };
    let key_packages: Vec<(Vec<u8>, Vec<u8>)> = {
        let conn = kps.lock()?;
        let mut st = conn.prepare("SELECT id, data FROM mls_key_package ORDER BY id")?;
        // Bound to a local, not left as the block's tail: the mapped-rows
        // iterator borrows `st`, and a tail temporary outlives the block's locals.
        let rows = st
            .query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    let mut snap = crate::mls_mem::Snapshot::default();
    for (gid, data) in states {
        let mut plain = open_blob(&ctx_group_state(&gid), data)?;
        snap.states.push((gid, std::mem::take(&mut *plain)));
    }
    for (gid, epoch_id, data) in epochs {
        let mut plain = open_blob(&ctx_epoch(&gid, epoch_id), data)?;
        snap.epochs.push((gid, epoch_id, std::mem::take(&mut *plain)));
    }
    for (id, data) in key_packages {
        let mut plain = open_blob(&ctx_key_package(&id), data)?;
        snap.key_packages.push((id, std::mem::take(&mut *plain)));
    }
    Ok(snap)
}

/// Write a [`crate::mls_mem::Snapshot`] into the stores at `db_path`, each blob
/// sealed under the device key exactly as the live write path seals it
/// (`seal_blob`, same per-row context), replacing any row already there. One
/// transaction: a restore is whole or it is nothing. Sealing happens before the
/// connection is opened — crypto off the connection, as `write` keeps it.
///
/// The caller restores BEFORE any MLS operation on this device, and follows it
/// with a rekey of every group (the retired backup's "fork"): a snapshot written
/// over a live group would strand what that group already held.
pub fn restore(db_path: &Path, snap: &crate::mls_mem::Snapshot) -> Result<(), StoreError> {
    let states = snap
        .states
        .iter()
        .map(|(gid, d)| Ok((gid.clone(), seal_blob(&ctx_group_state(gid), d)?)))
        .collect::<Result<Vec<(Vec<u8>, Vec<u8>)>, StoreError>>()?;
    let epochs = snap
        .epochs
        .iter()
        .map(|(gid, e, d)| Ok((gid.clone(), *e as i64, seal_blob(&ctx_epoch(gid, *e), d)?)))
        .collect::<Result<Vec<(Vec<u8>, i64, Vec<u8>)>, StoreError>>()?;
    let key_packages = snap
        .key_packages
        .iter()
        .map(|(id, d)| Ok((id.clone(), seal_blob(&ctx_key_package(id), d)?)))
        .collect::<Result<Vec<(Vec<u8>, Vec<u8>)>, StoreError>>()?;

    let mut conn = open(db_path)?;
    let tx = conn.transaction()?;
    for (gid, data) in &states {
        tx.execute(
            "INSERT INTO mls_group_state (group_id, data) VALUES (?1, ?2)
             ON CONFLICT(group_id) DO UPDATE SET data=excluded.data",
            params![gid, data],
        )?;
    }
    for (gid, epoch_id, data) in &epochs {
        tx.execute(
            "INSERT INTO mls_epoch (group_id, epoch_id, data) VALUES (?1, ?2, ?3)
             ON CONFLICT(group_id, epoch_id) DO UPDATE SET data=excluded.data",
            params![gid, *epoch_id, data],
        )?;
    }
    for (id, data) in &key_packages {
        tx.execute(
            "INSERT INTO mls_key_package (id, data) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET data=excluded.data",
            params![id, data],
        )?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_state_write_state_epoch_roundtrip() {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        crate::atrest::clear_device_key();
        std::env::remove_var("PACIFIC_ATREST_KEY");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mls.db");
        let mut s = SqliteGroupStateStorage::open(&path).unwrap();

        let gid = b"group-xyz".to_vec();
        let state = GroupState {
            id: gid.clone(),
            data: Zeroizing::new(b"state-bytes-v1".to_vec()),
        };
        let inserts = vec![EpochRecord::new(0, Zeroizing::new(b"epoch0".to_vec()))];
        s.write(state, inserts, Vec::new()).unwrap();

        assert_eq!(&s.state(&gid).unwrap().unwrap()[..], b"state-bytes-v1");
        assert_eq!(&s.epoch(&gid, 0).unwrap().unwrap()[..], b"epoch0");
        assert_eq!(s.max_epoch_id(&gid).unwrap(), Some(0));

        // overwrite state + add a second epoch; a fresh handle on the same path
        // sees both (proves cross-process durability).
        let state2 = GroupState {
            id: gid.clone(),
            data: Zeroizing::new(b"state-bytes-v2".to_vec()),
        };
        s.write(
            state2,
            vec![EpochRecord::new(1, Zeroizing::new(b"epoch1".to_vec()))],
            Vec::new(),
        )
        .unwrap();

        let s2 = SqliteGroupStateStorage::open(&path).unwrap();
        assert_eq!(&s2.state(&gid).unwrap().unwrap()[..], b"state-bytes-v2");
        assert_eq!(s2.max_epoch_id(&gid).unwrap(), Some(1));
        assert!(s2.state(b"absent").unwrap().is_none());
    }

    #[test]
    fn sealed_at_rest_when_key_present_and_fails_loud_without() {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        crate::atrest::clear_device_key();
        std::env::set_var("PACIFIC_ATREST_KEY", "ab".repeat(32));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mls.db");
        let mut s = SqliteGroupStateStorage::open(&path).unwrap();
        let gid = b"grp-sealed".to_vec();
        let secret = b"very-secret-ratchet-state".to_vec();
        s.write(
            GroupState {
                id: gid.clone(),
                data: Zeroizing::new(secret.clone()),
            },
            vec![EpochRecord::new(
                0,
                Zeroizing::new(b"epoch-secret-0".to_vec()),
            )],
            Vec::new(),
        )
        .unwrap();

        // On disk the `data` column is sealed and does NOT contain the plaintext.
        {
            let conn = Connection::open(&path).unwrap();
            let raw: Vec<u8> = conn
                .query_row(
                    "SELECT data FROM mls_group_state WHERE group_id=?1",
                    params![&gid],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(crate::atrest::is_sealed(&raw));
            assert!(!raw.windows(secret.len()).any(|w| w == &secret[..]));
        }

        // With the key, reads round-trip through decryption.
        assert_eq!(&s.state(&gid).unwrap().unwrap()[..], &secret[..]);
        assert_eq!(&s.epoch(&gid, 0).unwrap().unwrap()[..], b"epoch-secret-0");

        // Without the key, a sealed blob fails loud — never a plaintext fallback.
        crate::atrest::clear_device_key();
        std::env::remove_var("PACIFIC_ATREST_KEY");
        let s2 = SqliteGroupStateStorage::open(&path).unwrap();
        assert!(s2.state(&gid).is_err());
        assert!(s2.epoch(&gid, 0).is_err());
    }
}
