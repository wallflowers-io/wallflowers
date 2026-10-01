//! The lease on a pool leaf (resumption.md §6) — who may speak through it.
//!
//! One pool leaf, one sequence of CELLS at the home Arc's relay. Cell `n` is an
//! address off the storage root; a claim is a first-writer commit slot, so the
//! relay's arbitration IS the compare-and-set the 19 Sep ruling asked for, and the
//! cell counter is both the fence and the high-water mark. The current cell is the
//! highest `n` that holds a body; `n = 0`, nothing claimed, means the leaf is idle.
//!
//! PURE, like `spine`: addresses, seal and open. Where the bytes go is `Node`'s.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use pacific_wire::address::Address;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::CoreError;

/// How long a lease lasts from its last take or renew.
pub const LEASE_SECS: i64 = 30 * 24 * 3600;
/// Renew once fewer than this many seconds remain — so a device in use renews weekly.
pub const LEASE_RENEW_BELOW: i64 = 23 * 24 * 3600;

const CELL_LABEL: &[u8] = b"pacific/storage/lease/v1";
const SEAL_LABEL: &[u8] = b"pacific/storage/lease/seal/v1";
const MAGIC: &[u8; 4] = b"PLS1";
const NONCE_LEN: usize = 24;

/// What a cell records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Act {
    /// The leaf was idle and this holder took it.
    Take,
    /// The holder extended its own lease.
    Renew,
    /// Another device of the person found the lease expired and is removing the leaf.
    Evict,
}

/// One cell's body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    /// The install that wrote it — `Directory::device_id`, not the identity: every
    /// device of a person shares that.
    pub holder: [u8; 32],
    /// Unix seconds. Meaningless on an `Evict`, which ends the leaf.
    pub until: i64,
    pub act: Act,
}

fn hkdf(ikm: &[u8], label: &[u8]) -> Zeroizing<[u8; 32]> {
    let hk = hkdf::Hkdf::<sha2::Sha256>::new(Some(label), ikm);
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(b"v1", out.as_mut()).expect("32-byte OKM");
    out
}

/// Cell `n` of pool leaf `pool`: `Address::from_seed(HKDF(root, label ‖ pool ‖ be64(n)))`.
pub fn cell_address(storage_root: &[u8; 32], pool: &[u8; 32], n: u64) -> Address {
    let mut label = CELL_LABEL.to_vec();
    label.extend_from_slice(pool);
    label.extend_from_slice(&n.to_be_bytes());
    Address::from_seed(&hkdf(storage_root, &label))
}

fn seal_key(storage_root: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    hkdf(storage_root, SEAL_LABEL)
}

/// `PLS1 ‖ nonce(24) ‖ XChaCha20-Poly1305`, the magic as AAD.
pub fn seal_cell(lease: &Lease, storage_root: &[u8; 32]) -> Result<Vec<u8>, CoreError> {
    let mut plain = Vec::new();
    ciborium::into_writer(lease, &mut plain)
        .map_err(|e| CoreError::Directory(format!("lease encode: {e}")))?;
    let key = seal_key(storage_root);
    let cipher = XChaCha20Poly1305::new((&*key).into());
    let mut nonce = [0u8; NONCE_LEN];
    crate::head::getrandom_fill(&mut nonce)?;
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: &plain, aad: MAGIC })
        .map_err(|_| CoreError::Seal("lease seal failed".into()))?;
    let mut out = Vec::with_capacity(4 + NONCE_LEN + ct.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn open_cell(blob: &[u8], storage_root: &[u8; 32]) -> Result<Lease, CoreError> {
    if blob.len() < 4 + NONCE_LEN || &blob[..4] != MAGIC {
        return Err(CoreError::Seal("not a sealed lease cell".into()));
    }
    let key = seal_key(storage_root);
    let cipher = XChaCha20Poly1305::new((&*key).into());
    let plain = cipher
        .decrypt(
            XNonce::from_slice(&blob[4..4 + NONCE_LEN]),
            Payload { msg: &blob[4 + NONCE_LEN..], aad: MAGIC },
        )
        .map_err(|_| CoreError::Seal("lease cell open failed (wrong seed or tampered)".into()))?;
    ciborium::from_reader(plain.as_slice())
        .map_err(|e| CoreError::Directory(format!("lease decode: {e}")))
}

/// Where a leaf stands, given its cells in index order from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// No cell claimed: the leaf may be taken.
    Idle,
    /// Cell `n` is the current one.
    Held { n: u64, lease: Lease },
    /// Cell `n` evicted the leaf. It is never taken again (§6.3).
    Evicted { n: u64 },
}

pub fn standing(cells: &[(u64, Lease)]) -> Standing {
    match cells.iter().max_by_key(|(n, _)| *n) {
        None => Standing::Idle,
        Some((n, l)) if l.act == Act::Evict => Standing::Evicted { n: *n },
        Some((n, l)) => Standing::Held { n: *n, lease: *l },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> [u8; 32] {
        *crate::locator::storage_root(&[0x42u8; 32])
    }

    #[test]
    fn a_cell_round_trips_and_a_wrong_root_opens_nothing() {
        let l = Lease { holder: [3; 32], until: 99, act: Act::Take };
        let blob = seal_cell(&l, &root()).unwrap();
        assert_eq!(open_cell(&blob, &root()).unwrap(), l);
        let other = *crate::locator::storage_root(&[0x43u8; 32]);
        assert!(open_cell(&blob, &other).is_err());
    }

    #[test]
    fn every_cell_and_every_leaf_lands_somewhere_unrelated() {
        let r = root();
        let a = cell_address(&r, &[1; 32], 1).tag();
        assert_ne!(a, cell_address(&r, &[1; 32], 2).tag());
        assert_ne!(a, cell_address(&r, &[2; 32], 1).tag());
        assert_ne!(a, crate::locator::chain_locator(&r, 0, 1));
    }

    #[test]
    fn standing_reads_the_highest_cell() {
        let h = [9; 32];
        assert_eq!(standing(&[]), Standing::Idle);
        let take = Lease { holder: h, until: 10, act: Act::Take };
        let renew = Lease { holder: h, until: 20, act: Act::Renew };
        assert_eq!(standing(&[(1, take), (2, renew)]), Standing::Held { n: 2, lease: renew });
        let evict = Lease { holder: [8; 32], until: 0, act: Act::Evict };
        assert_eq!(standing(&[(1, take), (2, evict)]), Standing::Evicted { n: 2 });
    }

    #[test]
    fn a_device_in_use_renews_weekly() {
        assert_eq!(LEASE_SECS - LEASE_RENEW_BELOW, 7 * 24 * 3600);
    }
}
