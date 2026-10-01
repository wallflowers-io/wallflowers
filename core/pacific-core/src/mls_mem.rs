//! In-memory MLS storage — the platform pair for targets without SQLite.
//!
//! `mls.rs` is generic over its two storage providers (`PacificConfigWith<G, K>`),
//! so a platform supplies these two `mls-rs` trait impls and gets Pacific's whole
//! MLS protocol for free. SQLite is the native pair; this is the browser's.
//!
//! WHY IN MEMORY AND NOT INDEXEDDB DIRECTLY. `GroupStateStorage` and
//! `KeyPackageStorage` are SYNCHRONOUS in this build (mls-rs's `maybe_async` with
//! the sync feature — see `mls_store.rs`, whose impls are plain `fn`). IndexedDB
//! has no synchronous API at all, in any browser. The two cannot be welded
//! together, and making them fit would mean taking mls-rs async, which turns every
//! wrapper in `mls.rs` async with it.
//!
//! So the seam moves out one layer: MLS operates against memory synchronously, and
//! the HOST persists around the call — [`Snapshot`] out after a mutation, back in
//! at startup. That is one async boundary in the page's own code instead of an
//! async boundary threaded through the protocol, and it keeps the browser running
//! byte-identical MLS to the phone.
//!
//! THE SNAPSHOT IS SECRET. It carries ratchet state and epoch secrets in the clear.
//! A host that persists it MUST seal it first — in the browser, under a
//! non-extractable key on Pacific's own origin. Handing this to anything that is
//! not the device itself hands over the ability to decrypt the group.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use mls_rs_core::group::{EpochRecord, GroupState, GroupStateStorage};
use mls_rs_core::key_package::{KeyPackageData, KeyPackageStorage};
use mls_rs_core::mls_rs_codec::{MlsDecode, MlsEncode};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Storage failure. Only ever a poisoned lock — memory does not run out of rows.
#[derive(Debug)]
pub struct MemStoreError(pub String);

impl std::fmt::Display for MemStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mls memory store: {}", self.0)
    }
}
impl std::error::Error for MemStoreError {}
impl mls_rs_core::error::IntoAnyError for MemStoreError {
    fn into_dyn_error(self) -> Result<Box<dyn std::error::Error + Send + Sync>, Self> {
        Ok(Box::new(self))
    }
}

// --------------------------------------------------------------- group state ---

#[derive(Default)]
struct Groups {
    /// group_id -> the current `GroupState` blob.
    states: BTreeMap<Vec<u8>, Vec<u8>>,
    /// (group_id, epoch_id) -> a retained prior-epoch blob.
    epochs: BTreeMap<(Vec<u8>, u64), Vec<u8>>,
}

/// The browser's `GroupStateStorage`. `Clone` is shallow and deliberate — every
/// clone is the same store, because `mls-rs` clones the provider into the client
/// and both halves must see one another's writes.
#[derive(Clone, Default)]
pub struct MemGroupStateStorage {
    inner: Arc<Mutex<Groups>>,
}

impl MemGroupStateStorage {
    pub fn new() -> Self {
        Self::default()
    }

    /// DELETE one group's MLS state — the browser's half of §8.3 (a removed member
    /// "SHOULD promptly delete its group state and secret tree", RFC 9420 §12.4.2).
    /// Returns whether anything was held.
    pub fn delete_group(&self, group_id: &[u8]) -> bool {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let had = g.states.remove(group_id).is_some();
        g.epochs.retain(|(gid, _), _| gid != group_id);
        had
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Groups>, MemStoreError> {
        // A poisoned lock means another thread panicked mid-write. Recover rather
        // than propagate: on wasm there is one thread, so this cannot mean a torn
        // write from a concurrent mutator.
        Ok(self.inner.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

impl GroupStateStorage for MemGroupStateStorage {
    type Error = MemStoreError;

    fn state(&self, group_id: &[u8]) -> Result<Option<Zeroizing<Vec<u8>>>, Self::Error> {
        Ok(self
            .lock()?
            .states
            .get(group_id)
            .map(|d| Zeroizing::new(d.clone())))
    }

    fn epoch(&self, group_id: &[u8], epoch_id: u64) -> Result<Option<Zeroizing<Vec<u8>>>, Self::Error> {
        Ok(self
            .lock()?
            .epochs
            .get(&(group_id.to_vec(), epoch_id))
            .map(|d| Zeroizing::new(d.clone())))
    }

    fn write(
        &mut self,
        state: GroupState,
        epoch_inserts: Vec<EpochRecord>,
        epoch_updates: Vec<EpochRecord>,
    ) -> Result<(), Self::Error> {
        let mut g = self.lock()?;
        let gid = state.id.clone();
        g.states.insert(gid.clone(), state.data.to_vec());
        for e in epoch_inserts.into_iter().chain(epoch_updates) {
            g.epochs.insert((gid.clone(), e.id), e.data.to_vec());
        }
        Ok(())
    }

    fn max_epoch_id(&self, group_id: &[u8]) -> Result<Option<u64>, Self::Error> {
        Ok(self
            .lock()?
            .epochs
            .keys()
            .filter(|(g, _)| g == group_id)
            .map(|(_, e)| *e)
            .max())
    }
}

// --------------------------------------------------------------- key packages ---

/// The browser's `KeyPackageStorage`. Holds the PRIVATE halves of published key
/// packages until they are consumed by a join, so losing it means every offer this
/// device published becomes unusable.
#[derive(Clone, Default)]
pub struct MemKeyPackageStorage {
    inner: Arc<Mutex<BTreeMap<Vec<u8>, Vec<u8>>>>,
}

impl MemKeyPackageStorage {
    pub fn new() -> Self {
        Self::default()
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, BTreeMap<Vec<u8>, Vec<u8>>>, MemStoreError> {
        Ok(self.inner.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

impl KeyPackageStorage for MemKeyPackageStorage {
    type Error = MemStoreError;

    fn insert(&mut self, id: Vec<u8>, pkg: KeyPackageData) -> Result<(), Self::Error> {
        let bytes = pkg
            .mls_encode_to_vec()
            .map_err(|e| MemStoreError(format!("encode key package: {e}")))?;
        self.lock()?.insert(id, bytes);
        Ok(())
    }

    fn get(&self, id: &[u8]) -> Result<Option<KeyPackageData>, Self::Error> {
        match self.lock()?.get(id) {
            None => Ok(None),
            Some(b) => KeyPackageData::mls_decode(&mut &b[..])
                .map(Some)
                .map_err(|e| MemStoreError(format!("decode key package: {e}"))),
        }
    }

    fn delete(&mut self, id: &[u8]) -> Result<(), Self::Error> {
        self.lock()?.remove(id);
        Ok(())
    }
}

// ------------------------------------------------------------------ snapshot ---

/// The whole MLS state of a device, in one serialisable blob.
///
/// SECRET IN FULL. Ratchet state, epoch secrets and key-package private halves.
/// Seal it before it touches any storage — see the module header.
#[derive(Serialize, Deserialize, Default)]
pub struct Snapshot {
    /// (group_id, state_blob)
    pub states: Vec<(Vec<u8>, Vec<u8>)>,
    /// (group_id, epoch_id, epoch_blob)
    pub epochs: Vec<(Vec<u8>, u64, Vec<u8>)>,
    /// (key_package_id, key_package_blob)
    pub key_packages: Vec<(Vec<u8>, Vec<u8>)>,
}

impl Snapshot {
    /// Capture both stores. Call after any operation that mutated MLS state — a
    /// commit, a join, a key-package publish — and persist the result sealed.
    pub fn capture(
        gss: &MemGroupStateStorage,
        kps: &MemKeyPackageStorage,
    ) -> Result<Self, MemStoreError> {
        let g = gss.lock()?;
        let k = kps.lock()?;
        Ok(Self {
            states: g.states.iter().map(|(a, b)| (a.clone(), b.clone())).collect(),
            epochs: g
                .epochs
                .iter()
                .map(|((gid, e), d)| (gid.clone(), *e, d.clone()))
                .collect(),
            key_packages: k.iter().map(|(a, b)| (a.clone(), b.clone())).collect(),
        })
    }

    /// Rebuild both stores. Call once at startup, BEFORE any MLS operation —
    /// restoring over a live client would strand whatever it already held.
    pub fn restore(&self) -> (MemGroupStateStorage, MemKeyPackageStorage) {
        let gss = MemGroupStateStorage::new();
        let kps = MemKeyPackageStorage::new();
        {
            let mut g = gss.inner.lock().unwrap_or_else(|p| p.into_inner());
            for (gid, d) in &self.states {
                g.states.insert(gid.clone(), d.clone());
            }
            for (gid, e, d) in &self.epochs {
                g.epochs.insert((gid.clone(), *e), d.clone());
            }
        }
        {
            let mut k = kps.inner.lock().unwrap_or_else(|p| p.into_inner());
            for (id, d) in &self.key_packages {
                k.insert(id.clone(), d.clone());
            }
        }
        (gss, kps)
    }
}

/// The browser's instantiation of Pacific's MLS config.
pub type MemConfig = crate::mls::PacificConfigWith<MemGroupStateStorage, MemKeyPackageStorage>;
/// A client over in-memory storage.
pub type MemClient = crate::mls::ClientWith<MemGroupStateStorage, MemKeyPackageStorage>;
/// A group over in-memory storage.
pub type MemGroup = crate::mls::GroupWith<MemGroupStateStorage, MemKeyPackageStorage>;

/// Build a client over a pair of in-memory stores. The caller keeps the two store
/// handles: they are what [`Snapshot::capture`] reads, and cloning them is free
/// because every clone is the same store.
pub fn build_client_mem(
    gss: MemGroupStateStorage,
    kps: MemKeyPackageStorage,
    sid: crate::mls::SigningIdentity,
    sk: crate::mls::SecretKey,
) -> Result<MemClient, crate::CoreError> {
    crate::mls::build_client(gss, kps, sid, sk)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mls;

    /// One device: its own signing key and its own pair of stores, exactly as a
    /// browser tab would hold them.
    struct Device {
        gss: MemGroupStateStorage,
        kps: MemKeyPackageStorage,
        client: MemClient,
        cred: [u8; 32],
        /// Kept so a device can be REBUILT from a snapshot as its true self — a
        /// fresh signing key would be a different leaf, not the same device.
        sk: mls::SecretKey,
        pk: Vec<u8>,
    }

    fn device(seed: u8) -> Device {
        let crypto = mls::crypto();
        let (sk, pk) = mls::generate_signing_key(&crypto).unwrap();
        let cred = [seed; 32];
        let sid = mls::signing_identity(&cred, pk.as_bytes());
        let gss = MemGroupStateStorage::new();
        let kps = MemKeyPackageStorage::new();
        let client = build_client_mem(gss.clone(), kps.clone(), sid, sk.clone()).unwrap();
        Device { gss, kps, client, cred, sk, pk: pk.as_bytes().to_vec() }
    }

    /// THE PROOF THAT A BROWSER CAN BE A LEAF. Two devices over in-memory storage
    /// create a group, add one another, and exchange a delta — the same
    /// `mls.rs` code path the phone runs, with only the two storage impls swapped.
    #[test]
    fn two_devices_form_a_group_and_exchange_a_delta_over_memory_storage() {
        let a = device(0xA1);
        let b = device(0xB2);

        // A stands up the group; B publishes a key package.
        let mut ga = mls::create_group_named(&a.client, "supper").unwrap();
        assert_eq!(ga.current_epoch(), 0);
        let kp_b = mls::make_key_package_bytes(&b.client).unwrap();

        // A adds B: one commit, one welcome, epoch advances.
        let (_commit, welcome) = mls::add_member(&mut ga, &kp_b).unwrap();
        assert_eq!(ga.current_epoch(), 1, "an Add advances the epoch");

        // B joins from the welcome alone.
        let mut gb = mls::join_group(&b.client, &welcome).unwrap();
        assert_eq!(gb.current_epoch(), 1);
        assert_eq!(mls::group_name(&gb), "supper", "the name rides the GroupInfo");

        // Both see the same two-person roster.
        let mut ra = mls::roster_identities(&ga).unwrap();
        let mut rb = mls::roster_identities(&gb).unwrap();
        ra.sort();
        rb.sort();
        assert_eq!(ra, rb);
        assert!(ra.contains(&a.cred) && ra.contains(&b.cred));

        // The exporter agrees — this is what addresses the mailbox and keys the seal.
        let eb = mls::epoch_be(gb.current_epoch());
        assert_eq!(
            mls::group_tag(&ga, &eb).unwrap(),
            mls::group_tag(&gb, &eb).unwrap(),
            "both devices derive the same relay tag"
        );
        assert_eq!(
            mls::seal_conn_secret(&ga, &eb).unwrap(),
            mls::seal_conn_secret(&gb, &eb).unwrap(),
            "and the same seal secret"
        );

        // A delta crosses.
        let delta = b"\xa1\x64test\x01";
        let ct = mls::encrypt_delta(&mut ga, delta).unwrap();
        assert_ne!(&ct[..], &delta[..], "it is on the wire encrypted");
        match mls::decrypt_message(&mut gb, &ct).unwrap() {
            mls::Incoming::Application { sender, data } => {
                assert_eq!(data, delta, "B reads exactly what A wrote");
                assert_eq!(sender, a.cred, "attributed to A's leaf, not carried in the payload");
            }
            _ => panic!("expected an application message"),
        }
    }

    /// The persistence seam, which is what lets a browser tab survive a reload:
    /// snapshot out, rebuild from nothing but that blob plus the device's own
    /// signing key, and the group still opens and still decrypts.
    #[test]
    fn a_snapshot_restores_a_device_that_can_still_use_its_group() {
        let a = device(0xC3);
        let b = device(0xD4);
        let mut ga = mls::create_group_named(&a.client, "room").unwrap();
        let kp_b = mls::make_key_package_bytes(&b.client).unwrap();
        let (_c, welcome) = mls::add_member(&mut ga, &kp_b).unwrap();
        let mut gb = mls::join_group(&b.client, &welcome).unwrap();
        let gid = ga.group_id().to_vec();

        // Persist A and throw the live client away, as a page unload would.
        let blob = serde_json::to_vec(&Snapshot::capture(&a.gss, &a.kps).unwrap()).unwrap();
        drop(ga);
        drop(a.client);

        // Rebuild A from the snapshot and its signing key alone.
        let restored: Snapshot = serde_json::from_slice(&blob).unwrap();
        let (gss2, kps2) = restored.restore();
        let client2 = build_client_mem(
            gss2,
            kps2,
            mls::signing_identity(&a.cred, &a.pk),
            a.sk.clone(),
        )
        .unwrap();
        let mut ga2 = mls::load_group(&client2, &gid).unwrap();
        assert_eq!(ga2.current_epoch(), 1, "the restored device is at the same epoch");

        // And it is really the same leaf: a delta it writes decrypts for B, and B's
        // reply decrypts for it.
        let ct = mls::encrypt_delta(&mut ga2, b"after the reload").unwrap();
        match mls::decrypt_message(&mut gb, &ct).unwrap() {
            mls::Incoming::Application { sender, data } => {
                assert_eq!(data, b"after the reload");
                assert_eq!(sender, a.cred, "still A's leaf, not a new member");
            }
            _ => panic!("expected an application message"),
        }
        let back = mls::encrypt_delta(&mut gb, b"and back").unwrap();
        match mls::decrypt_message(&mut ga2, &back).unwrap() {
            mls::Incoming::Application { data, .. } => assert_eq!(data, b"and back"),
            _ => panic!("expected an application message"),
        }
    }
}
