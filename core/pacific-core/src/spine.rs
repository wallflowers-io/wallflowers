//! The spine — what an account is, written where its seed can reach.
//!
//! A device that has lost everything holds 24 words. From them it derives the
//! storage root, and from that it must be able to NAME the objects it belongs to.
//! Nothing else in this crate can tell it: the group id is the input to every
//! address the archive uses, so without a list of them there is nothing to derive.
//! `pacific.db` holds that list and is exactly what was lost.
//!
//! ## One implementation, every platform
//!
//! **This module is the entry format, and it is imported rather than reproduced.**
//! iOS reaches it through `pacific-ffi`, the browser through `core-wasm`, and
//! neither writes its own encoder. That is not tidiness: two implementations that
//! must agree byte for byte are two sources of truth, which is the same error the
//! ICD rule forbids for the delta model. A spine written on a phone and read in a
//! browser is the ordinary case, not the exotic one.
//!
//! It is therefore PURE — encode, address, seal, and their inverses. No I/O, no
//! clock, no platform. Where the bytes go is the caller's; what they are is here.
//!
//! ## Addressing: arithmetic, ruled 20 September 2026
//!
//! Entry `i` of generation `g` is at [`crate::locator::chain_locator`]`(root, g, i)`
//! — computed, never followed. A lost entry costs its own index and the ones
//! either side survive. The form it replaced made only the first address derivable
//! and carried the next inside each entry, so a break at `k` took everything after
//! it; `Catching Up` §01 named that cost precisely — "a break at entry k costs
//! every entry after it. That is membership."
//!
//! Each index lands at an unrelated address, so the store sees no stable per-account
//! tag and cannot count an account's objects from the addresses. `Keys` §6's
//! objection was to ONE seed-derived tag accumulating an entry per epoch; a family
//! of addresses under HKDF is not that.
//!
//! ## What an entry is not
//!
//! Not a backup. It carries the group id and the epoch this account joined at —
//! enough to NAME an object and to know the floor of what it may read. The content
//! is reached from there by arithmetic ([`crate::locator::archive_locator`]), and
//! the MLS state by its own storage. An entry holds no ratchet, no epoch secret and
//! no key package: that was `backup::History`, and it was an escrow.

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use crate::CoreError;

/// Entry version. Bumped when the SHAPE changes, never when a value does.
pub const ENTRY_VERSION: u32 = 1;

/// The seal's domain, in the storage family off the root — so someone restoring
/// from their words alone, having never held a passkey, can open it.
pub const ENTRY_DOMAIN: &[u8] = b"pacific/storage/chain/seal/v1";

/// "Pacific Chain Entry v1" — the same envelope shape as the head and the wrap,
/// so one magic and one length check serve all of them at the arc.
const MAGIC: &[u8; 4] = b"PCE1";
const NONCE_LEN: usize = 24;

/// This account joined `group_id`, and `first_epoch` is the floor of what it may
/// ever read there.
///
/// The floor is recorded because it is not recoverable later: a member added at
/// epoch 12 cannot derive epoch 11's content key, so a reader that did not know
/// where to start would report the epochs below as missing rather than as never
/// theirs. Those are different facts and a person is owed the difference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Joined {
    #[serde(with = "serde_bytes")]
    pub group_id: Vec<u8>,
    pub first_epoch: u64,
}

/// This account's membership of `group_id` ended at `last_epoch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Departed {
    #[serde(with = "serde_bytes")]
    pub group_id: Vec<u8>,
    pub last_epoch: u64,
}

/// One pooled leaf of this person in one object: everything a device with nothing
/// needs to join through it (resumption.md §5.3, carried on the spine by A1).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolLeaf {
    /// The leaf's signature public key — its name in the tree and in its lease cells.
    pub pool: [u8; 32],
    #[serde(with = "serde_bytes")]
    pub welcome: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub sig_sk: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub kp_id: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub kp_data: Vec<u8>,
    /// The epoch the leaf joined at: the floor of what it reads.
    pub epoch: u64,
}

/// Never the private halves: a log line is not a place for a signing key.
impl std::fmt::Debug for PoolLeaf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PoolLeaf")
            .field("pool", &hex::encode(self.pool))
            .field("epoch", &self.epoch)
            .finish_non_exhaustive()
    }
}

/// The way into one object: its idle pool leaves. A later `Pool` for the same group
/// supersedes an earlier one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pool {
    #[serde(with = "serde_bytes")]
    pub group_id: Vec<u8>,
    pub kind: String,
    pub arc: String,
    pub pools: Vec<PoolLeaf>,
}

/// What an entry says.
///
/// Appended, never edited: the spine is a log, so leaving is a fact added to it
/// rather than a row rewritten. Everything recorded up to and including
/// `last_epoch` stays readable — removal is not retroactive — and the departure is
/// what stops a reader probing forward for epochs it was never going to derive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Body {
    Group(Joined),
    Left(Departed),
    Pool(Pool),
}

impl Body {
    /// The object this entry is about, whichever kind it is.
    pub fn group_id(&self) -> &[u8] {
        match self {
            Body::Group(j) => &j.group_id,
            Body::Left(d) => &d.group_id,
            Body::Pool(p) => &p.group_id,
        }
    }
}

/// One entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub v: u32,
    /// Its own index, inside the sealed plaintext as well as in the address it is
    /// stored at. An entry that opens but names a different index is a mis-served
    /// object, and saying so is cheaper than folding it into the wrong place.
    pub index: u64,
    pub body: Body,
}

impl Entry {
    pub fn new(index: u64, body: Body) -> Self {
        Entry {
            v: ENTRY_VERSION,
            index,
            body,
        }
    }

    /// Canonical CBOR, the encoding the delta log already uses — so one encoder
    /// serves the envelopes and the spine and there is no second opinion about
    /// what a byte means.
    pub fn to_cbor(&self) -> Result<Zeroizing<Vec<u8>>, CoreError> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf)
            .map_err(|e| CoreError::Directory(format!("spine entry encode: {e}")))?;
        Ok(Zeroizing::new(buf))
    }

    pub fn from_cbor(bytes: &[u8]) -> Result<Self, CoreError> {
        let e: Entry = ciborium::from_reader(bytes)
            .map_err(|err| CoreError::Directory(format!("spine entry decode: {err}")))?;
        if e.v != ENTRY_VERSION {
            return Err(CoreError::Directory(format!(
                "spine entry version {} (this build reads {ENTRY_VERSION})",
                e.v
            )));
        }
        Ok(e)
    }
}

/// The key every entry of this account is sealed under.
///
/// From the ROOT, never the PRF — the same property `head_key` exists to hold. A
/// person restoring from 24 words on a borrowed laptop, having never had a
/// passkey, must be able to open their own spine, or the words door has no anchor.
pub fn entry_key(storage_root: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let hk = hkdf::Hkdf::<sha2::Sha256>::new(Some(ENTRY_DOMAIN), storage_root);
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(b"v1", out.as_mut()).expect("32-byte OKM");
    out
}

/// Seal one entry for its own slot. Envelope: `MAGIC(4) || nonce(24) || ciphertext`.
///
/// The index goes into the AEAD's additional data as well as into the plaintext,
/// so a store that serves entry 7's bytes as entry 3 makes them fail to open
/// rather than open wrongly.
pub fn seal_entry(entry: &Entry, key: &[u8; 32]) -> Result<Vec<u8>, CoreError> {
    let plain = entry.to_cbor()?;
    let cipher = XChaCha20Poly1305::new(key.into());
    let mut nonce = [0u8; NONCE_LEN];
    crate::head::getrandom_fill(&mut nonce)?;
    let ct = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &plain,
                aad: &aad(entry.index),
            },
        )
        .map_err(|_| CoreError::Seal("spine entry seal failed".into()))?;
    let mut out = Vec::with_capacity(4 + NONCE_LEN + ct.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Open the entry stored at `index`. A wrong root, a moved blob or a tampered
/// byte fails the AEAD tag.
pub fn open_entry(blob: &[u8], key: &[u8; 32], index: u64) -> Result<Entry, CoreError> {
    if blob.len() < 4 + NONCE_LEN || &blob[..4] != MAGIC {
        return Err(CoreError::Seal("not a sealed spine entry".into()));
    }
    let cipher = XChaCha20Poly1305::new(key.into());
    let plain = cipher
        .decrypt(
            XNonce::from_slice(&blob[4..4 + NONCE_LEN]),
            Payload {
                msg: &blob[4 + NONCE_LEN..],
                aad: &aad(index),
            },
        )
        .map_err(|_| {
            CoreError::Seal("spine entry open failed (wrong seed, wrong slot, or tampered)".into())
        })?;
    let entry = Entry::from_cbor(&plain)?;
    if entry.index != index {
        return Err(CoreError::Directory(format!(
            "spine entry at index {index} names index {}",
            entry.index
        )));
    }
    Ok(entry)
}

fn aad(index: u64) -> Vec<u8> {
    let mut a = ENTRY_DOMAIN.to_vec();
    a.extend_from_slice(&index.to_be_bytes());
    a
}

/// Where entry `index` of generation `gen` lives, and the bytes to put there.
///
/// The whole of what a platform needs: an address and a blob. What carries them
/// differs — a relay publish on the phone, a fetch in a browser — and neither
/// re-derives either.
pub fn place(
    storage_root: &[u8; 32],
    gen: u32,
    index: u64,
    body: Body,
) -> Result<([u8; 32], Vec<u8>), CoreError> {
    let entry = Entry::new(index, body);
    let blob = seal_entry(&entry, &entry_key(storage_root))?;
    Ok((crate::locator::chain_locator(storage_root, gen, index), blob))
}

/// The tail a head names — `head::tail_hash`, the one definition, re-exported so
/// the spine's readers need not reach into the head for it.
pub fn tail_hash(blob: &[u8]) -> [u8; 32] {
    crate::head::tail_hash(blob)
}

/// A generation as the relay holds it: every entry from index 0 to the first empty
/// one, and the head that describes it.
#[derive(Debug, Clone)]
pub struct Walk {
    pub entries: Vec<(u64, Entry)>,
    pub position: u64,
    pub tail: [u8; 32],
}

/// What a platform stores after the chain moved: the sealed head, and its fields.
#[derive(Debug, Clone)]
pub struct HeadUpdate {
    pub position: u64,
    pub tail: [u8; 32],
    pub sealed: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> [u8; 32] {
        *crate::locator::storage_root(&[0x5au8; 32])
    }

    fn joined(n: u8) -> Body {
        Body::Group(Joined {
            group_id: vec![n; 32],
            first_epoch: n as u64,
        })
    }

    #[test]
    fn an_entry_round_trips_through_its_own_slot() {
        let k = entry_key(&root());
        let e = Entry::new(3, joined(7));
        let blob = seal_entry(&e, &k).unwrap();
        assert_eq!(&blob[..4], MAGIC, "the envelope announces itself");
        assert_eq!(open_entry(&blob, &k, 3).unwrap(), e);
    }

    #[test]
    fn a_wrong_root_opens_nothing() {
        let e = Entry::new(0, joined(1));
        let blob = seal_entry(&e, &entry_key(&root())).unwrap();
        let other = *crate::locator::storage_root(&[0x11u8; 32]);
        assert!(open_entry(&blob, &entry_key(&other), 0).is_err());
    }

    #[test]
    fn an_entry_served_from_the_wrong_slot_refuses() {
        // The index is in the AAD, so this fails at the tag rather than decoding
        // into the wrong place and being believed.
        let k = entry_key(&root());
        let blob = seal_entry(&Entry::new(7, joined(2)), &k).unwrap();
        assert!(
            open_entry(&blob, &k, 3).is_err(),
            "entry 7 must not open as entry 3"
        );
        assert!(open_entry(&blob, &k, 7).is_ok());
    }

    #[test]
    fn every_index_lands_somewhere_unrelated() {
        // The property the arithmetic form exists for: each entry is independently
        // addressable, so losing one costs one — and the addresses share no
        // structure a store could correlate into a count of the account's objects.
        let r = root();
        let a0 = crate::locator::chain_locator(&r, 0, 0);
        let a1 = crate::locator::chain_locator(&r, 0, 1);
        let a2 = crate::locator::chain_locator(&r, 0, 2);
        assert_ne!(a0, a1);
        assert_ne!(a1, a2);
        assert!(
            a0.iter().zip(a1.iter()).filter(|(x, y)| x == y).count() < 8,
            "consecutive entries must share no prefix"
        );
        // And a generation is its own family.
        assert_ne!(a0, crate::locator::chain_locator(&r, 1, 0));
    }

    #[test]
    fn place_gives_an_address_and_a_blob_that_belong_together() {
        let r = root();
        let (addr, blob) = place(&r, 0, 2, joined(9)).unwrap();
        assert_eq!(addr, crate::locator::chain_locator(&r, 0, 2));
        let e = open_entry(&blob, &entry_key(&r), 2).unwrap();
        assert_eq!(e.body.group_id(), &vec![9u8; 32][..]);
    }

    #[test]
    fn leaving_is_appended_and_not_an_edit() {
        // A departure is its own entry. The join stays exactly as it was written,
        // because removal is not retroactive and the history below the cut is
        // still this person's.
        let k = entry_key(&root());
        let join = Entry::new(0, joined(4));
        let left = Entry::new(1, Body::Left(Departed { group_id: vec![4u8; 32], last_epoch: 9 }));
        let jb = seal_entry(&join, &k).unwrap();
        let lb = seal_entry(&left, &k).unwrap();
        assert_eq!(open_entry(&jb, &k, 0).unwrap(), join, "the join is untouched");
        assert_eq!(open_entry(&lb, &k, 1).unwrap().body.group_id(), join.body.group_id());
    }

    #[test]
    fn a_pool_entry_round_trips_and_never_prints_its_key() {
        let k = entry_key(&root());
        let leaf = PoolLeaf {
            pool: [7; 32],
            welcome: vec![1, 2, 3],
            sig_sk: vec![0xAB; 32],
            kp_id: vec![4],
            kp_data: vec![5, 6],
            epoch: 3,
        };
        let e = Entry::new(
            5,
            Body::Pool(Pool { group_id: vec![9; 32], kind: "post".into(), arc: "wss://a".into(), pools: vec![leaf] }),
        );
        let blob = seal_entry(&e, &k).unwrap();
        assert_eq!(open_entry(&blob, &k, 5).unwrap(), e);
        assert!(!format!("{e:?}").contains("171, 171"), "the signing key is not in Debug");
        assert_eq!(tail_hash(&blob), tail_hash(&blob.clone()));
        assert_ne!(tail_hash(&blob), tail_hash(&seal_entry(&e, &k).unwrap()), "each seal is fresh");
    }

    #[test]
    fn the_entry_key_is_not_the_head_key_and_not_the_wrap_key() {
        // One root, many domains, and no two may converge — a party holding one
        // must not thereby open the others.
        let r = root();
        assert_ne!(*entry_key(&r), *crate::head::head_key(&r));
        assert_ne!(*entry_key(&r), *crate::wrap::wrap_key(&r));
    }
}
