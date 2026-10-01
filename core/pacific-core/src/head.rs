//! The head — where the per-user archive chain currently ends, sealed under the
//! seed and anchored OUTSIDE the chain.
//!
//! THE THIRD ARTEFACT. The account record at `/auth/users/<pk>` already holds two:
//!
//!   - THE WRAP ([`crate::wrap`]) — the 32-byte seed under the passkey's PRF.
//!     Served unauthenticated by public key, because that read is how a new device
//!     obtains the key it would authenticate with.
//!   - THE HISTORY — archive, MLS state and intro tag, sealed under a key
//!     derived from the SEED. Retired on 14 Sep 2026; its module is gone
//!     (a58798c), and the auth service still carries its routes.
//!
//! This is the third, and it is the smallest: a tail hash and a chain position,
//! nothing else. It exists because a chain of sealed entries catches modification,
//! reordering, dropped middles and forks, and does NOT catch truncation of the
//! tail — a prefix of a valid chain is itself a valid chain. Detecting that needs
//! a head the reader already knows. A device that persists state between runs has
//! one. A stateless browser does not, and a borrowed laptop is a stateless
//! browser.
//!
//! SEALED UNDER THE STORAGE ROOT, NOT THE PRF, and that is the load-bearing half.
//! The wrap is the seed sealed under the passkey; a head folded into that envelope
//! would be PRF-sealed, and the words door could never reach it — the person who
//! has their 24 words and no passkey at all would have no anchor to check the chain
//! against, which is precisely the person who most needs one. A separate blob under
//! a root-derived key means BOTH doors arrive:
//!
//! ```text
//! passkey → PRF → wrap → seed → storage root → head
//! words   →              seed → storage root → head
//! ```
//!
//! THE STORAGE ROOT, NOT THE SEED ITSELF (v2, 18 Sep 2026). `five-things.html` §06
//! put the head key under the storage root — `storage root → locators · acct · head
//! key · history key` — and ruled the two HKDF families disjoint BEFORE anything is
//! written, because a BYO account has no seed and would otherwise need a second
//! derivation of its own anchor. v1 took the seed directly under `pacific/head/v1`,
//! outside both families. No v1 head was ever written at a locator, so moving it
//! cost nothing; after the first write it would have cost a version on every
//! account.
//!
//! A MONOTONIC POSITION BESIDE THE HASH, and that is the other half. Two hashes
//! cannot be compared for recency: given two heads, nothing in either says which
//! one is later. The position is what lets the arc enforce a compare-and-set on a
//! client's write, and what lets a device with memory SEE a head go backwards.
//! Without it the head write has exactly the stale-clobber shape that
//! `PUT /history` needed guarding against on 13 Sep — two sessions racing, last
//! writer wins, and the anchor silently moves back.
//!
//! THE POSITION TRAVELS TWICE, in the clear and under the seal. The arc cannot
//! read this blob, so it cannot enforce a compare-and-set on a number it can only
//! find inside — the client therefore declares the position in a header and the
//! arc stores and orders on that. The same number is sealed in here, so a client
//! can catch an arc that reports one position and serves the ciphertext of
//! another: see [`open_declared`].
//!
//! WHAT THIS CLOSES, AND WHAT IT DOES NOT. It does not defeat a deliberate rollback
//! by the arc, which serves both artefacts and can hand a stateless browser an old
//! head together with the matching truncated chain — a self-consistent pair from
//! one trust domain. What it closes is everything that actually happens: replica
//! lag, a restore of the arc's database, a partial sync, a bug. Those truncate the
//! chain without moving the head, and [`Head::check`] sees the mismatch on the
//! first walk. And it raises the malicious case from silence to sustained,
//! coordinated deceit across two independently sealed artefacts, which fails the
//! moment any device with local memory reads — or any witness does, because the
//! head record is served unauthenticated by public key and so anything watching
//! sees the ciphertext revert without ever being able to read it.
//!
//! WHAT THE ARC LEARNS: the size, the write times, and the chain position. The
//! position in the clear is an activity counter, and it is the price of the two
//! properties above — a witness needs to see the anchor move, and the arc needs to
//! refuse a write that moves it backwards. Both require a number neither of them
//! can decrypt.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::CoreError;

/// v2, 18 Sep 2026: `chain_gen` in, `updated_at` out, and the key off the storage
/// root. v1 was never written at a locator, and this build reads v2 only.
pub const HEAD_VERSION: u32 = 2;
/// Domain for the head key — the HKDF salt AND the AEAD's associated data. In the
/// `pacific/storage/` family with everything else off the storage root, and a
/// different string from [`crate::wrap::WRAP_DOMAIN`] and the retired history's,
/// so that one root yields unrelated keys and a party holding any one of them can
/// open nothing else.
pub const HEAD_DOMAIN: &[u8] = b"pacific/storage/head/seal/v1";
const NONCE_LEN: usize = 24;
const MAGIC: &[u8; 4] = b"PHD1";
const TAIL_LEN: usize = 32;

/// The tail of a chain with no entries in it. Paired with position 0, and refused
/// in any other combination — a chain that has a tail has a position, and one that
/// has a position has a tail.
pub const NO_TAIL: [u8; TAIL_LEN] = [0u8; TAIL_LEN];

/// The head, in the clear. Small on purpose: it is an ANCHOR, not a summary, and
/// every field a reader does not strictly need is a field an arc gets to observe
/// the length of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Head {
    pub v: u32,
    /// The arc this head is stored at. Provenance only — not bound in the AAD,
    /// for the same reason the history is not: an account may legitimately be
    /// re-homed, and a head that stopped opening when it moved would make moving
    /// arcs a data loss. (The WRAP does bind its host, because a wrap replayed
    /// onto a hostile domain is an unlock the attacker gets to watch. Nothing
    /// analogous applies here: this blob unlocks nothing.)
    pub arc: String,
    /// HOW MANY ENTRIES THE CHAIN HOLDS. Entries are numbered from 1, so the tail
    /// is entry number `position`, and `position == 0` is an empty chain. This is
    /// the number the arc compare-and-sets on.
    pub position: u64,
    /// The hash of the tail entry's CIPHERTEXT — the arc can compute it over bytes
    /// it already holds without reading anything, which is what lets it reject a
    /// non-linear append as cheaply as it rejects a stale head.
    #[serde(with = "serde_bytes")]
    pub tail: Vec<u8>,
    /// WHICH CHAIN this head anchors. Entry `i` of generation `g` lives at
    /// [`crate::locator::chain_locator`]`(storage_root, g, i)` — arithmetic, so a
    /// fresh device learns from the one record it fetches first which family of
    /// addresses to walk, and can ask for any index without walking to it.
    ///
    /// Without it there is exactly one chain per account, for ever, append-only and
    /// unprunable: `chain_tag₀` would be a pure function of the root. With it a chain
    /// can be RESTARTED — pruned once it has grown long, begun again after a concern
    /// about its seal key, or abandoned when its middle is lost — by writing a new
    /// generation and moving the head to it. Four bytes.
    pub chain_gen: u32,
    // `updated_at` was here in v1 and is gone on purpose. Nothing read it: the arc
    // cannot open this blob, and its own `heads.updated_at` column (served on
    // `/head/meta`) is what a witness watches. A timestamp is not monotone, so for
    // noticing an old head `position` was strictly better — and every field in here
    // is a length the arc gets to observe on a row anyone may poll.
}

/// What a walk of the chain says about the head it was checked against. This is
/// the whole question a returning device asks, and it has four answers rather than
/// two because "the chain is short" and "the anchor is old" are different events
/// with different responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainVerdict {
    /// Position and tail both agree: the arc showed the whole chain.
    Whole,
    /// The walk ended BEFORE the head. Entries are missing from the tail — replica
    /// lag, a restored database, a partial sync, a bug, or a truncating arc.
    /// Whatever the cause, the reader has not been shown everything and must not
    /// treat what it has as complete.
    Truncated { missing: u64 },
    /// The same position, a different tail. Two writers appended against one
    /// predecessor, or someone rewrote an entry. Not recoverable by reading
    /// further: the two chains disagree about what happened.
    Forked,
    /// The walk went PAST the head — more entries exist than the anchor knows
    /// about. USUALLY BENIGN and must not be reported as an attack: it is what a
    /// client that appended and then died before updating the head leaves behind.
    /// The response is to advance the head, not to distrust the arc.
    HeadBehind { extra: u64 },
}

/// The hash a head's `tail` holds: SHA-256 over the tail entry's SEALED bytes, the
/// ciphertext exactly as stored. Computed over ciphertext so the arc can take it
/// without a key; defined once, here, so the writer that advances a head and the
/// reader that judges one cannot disagree about what "the tail" means.
pub fn tail_hash(entry_blob: &[u8]) -> [u8; TAIL_LEN] {
    use sha2::Digest;
    sha2::Sha256::digest(entry_blob).into()
}

/// The entry the head names as its tail — index `position − 1` — as a walk found it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TailState {
    /// Present, and its ciphertext hashes to the head's tail.
    Intact,
    /// Nothing opened at the head's tail index. The one case that is truncation: the
    /// head vouches for an entry the store no longer shows.
    Missing,
    /// Entries opened at the tail index and NONE of them is the one the head names.
    /// Two writers, or a rewrite — not repaired by reading further.
    Forked,
    /// The head vouches for nothing (`position == 0`), so there is no tail to check
    /// and no way to tell a complete walk from a cut one. See [`Head::judge`].
    Unknown,
}

impl TailState {
    /// The wire word. Pinned by `app/web/shared/recovery-contract.json`.
    pub fn as_str(self) -> &'static str {
        match self {
            TailState::Intact => "intact",
            TailState::Missing => "missing",
            TailState::Forked => "forked",
            TailState::Unknown => "unknown",
        }
    }
}

/// A walk of an ARITHMETIC chain, judged against its head BY INDEX.
///
/// A record rather than one verdict, because under arithmetic addressing the facts
/// are independent: the tail can be intact with a hole below it, and a lagging
/// head can sit on top of either. [`ChainVerdict`] collapses them into one word,
/// which was right when a walk was always a prefix and is wrong now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Walked {
    pub tail: TailState,
    /// Indices below `position` that did not open: ordinary loss, survivable
    /// because every index has its own address, and named rather than counted.
    pub holes: Vec<u64>,
    /// Indices at or past `position` that opened. The head is behind, which is
    /// normal — a device appended and has not advanced it yet.
    pub extra: u64,
    /// Distinct indices that opened. BY INDEX, not by entry: until indices are
    /// claimed account-wide, two of a person's devices can publish at one index,
    /// and a tag can hold more than one entry.
    pub found: u64,
}

impl Head {
    /// A head naming an empty chain. The anchor an account starts with: writing it
    /// establishes the floor the arc's compare-and-set enforces from then on.
    pub fn empty(arc: impl Into<String>, chain_gen: u32) -> Self {
        Head {
            v: HEAD_VERSION,
            arc: arc.into(),
            position: 0,
            tail: NO_TAIL.to_vec(),
            chain_gen,
        }
    }

    pub fn new(arc: impl Into<String>, position: u64, tail: [u8; TAIL_LEN], chain_gen: u32) -> Self {
        Head {
            v: HEAD_VERSION,
            arc: arc.into(),
            position,
            tail: tail.to_vec(),
            chain_gen,
        }
    }

    pub fn tail(&self) -> [u8; TAIL_LEN] {
        self.tail.as_slice().try_into().expect("checked on decode")
    }

    /// Is the chain I just walked the whole chain this head anchors?
    ///
    /// COUNT-BASED, for the linked chain. The spine is addressed arithmetically and
    /// is judged with [`Head::judge`]; this stays because the web still calls it.
    ///
    /// `walked_position` is how many entries the walk actually yielded and
    /// `walked_tail` the hash of the last one's ciphertext. This is the ONE call a
    /// returning device makes, and the reason the head is testable before the chain
    /// exists: the verdict is a function of two numbers and two hashes.
    pub fn check(&self, walked_position: u64, walked_tail: &[u8; TAIL_LEN]) -> ChainVerdict {
        if walked_position < self.position {
            return ChainVerdict::Truncated { missing: self.position - walked_position };
        }
        if walked_position > self.position {
            return ChainVerdict::HeadBehind { extra: walked_position - self.position };
        }
        // Compared as slices rather than through `tail()`, which would panic on a
        // hand-built `Head` whose tail is the wrong length. A malformed tail can
        // never equal a 32-byte walk, so it falls out as `Forked` — which is the
        // honest answer: this anchor does not agree with that chain.
        if self.tail.as_slice() == walked_tail.as_slice() {
            ChainVerdict::Whole
        } else {
            ChainVerdict::Forked
        }
    }

    /// Judge a walk of an ARITHMETIC chain against this head, by index.
    ///
    /// `opened` is every entry that opened under the account's key, as
    /// `(index, tail_hash(sealed bytes))`. An index may appear more than once.
    ///
    /// WHY NOT [`Head::check`]. `check` compares COUNTS — how many entries the walk
    /// yielded, then the hash of the last one. That is exact for a linked chain,
    /// where a walk is always a prefix. Under arithmetic addressing a walk is a set
    /// of indices, and counting misreports it: one lost entry mid-chain reads as
    /// `Truncated` though the tail is intact, and a hole plus a lagging head can
    /// make the counts agree and compare the wrong entry, reading as `Forked` — an
    /// accusation. This checks the entry AT `position − 1`, never "the last one
    /// that opened", and reports holes and extra as their own facts.
    ///
    /// POSITION 0 IS UNKNOWN, NOT EMPTY. A head at 0 vouches for nothing. Today
    /// that is every head: the only one written is the first head at account
    /// creation, and nothing advances it. Reading 0 as "the chain is empty" would
    /// make every walk look complete, including one whose tail was cut. So until a
    /// head vouches for something, the answer is that it cannot be told.
    ///
    /// THE WALK THIS EXPECTS: every index below `position`, THROUGH misses — a
    /// stop at the first miss would turn one lost entry into a lost tail, which is
    /// the cost arithmetic addressing was chosen to avoid — then onward past
    /// `position` until the first miss, since nothing vouches for what lies beyond.
    pub fn judge(&self, opened: &[(u64, [u8; TAIL_LEN])]) -> Walked {
        let indices: std::collections::BTreeSet<u64> = opened.iter().map(|(i, _)| *i).collect();
        let found = indices.len() as u64;
        let extra = indices.range(self.position..).count() as u64;

        if self.position == 0 {
            return Walked { tail: TailState::Unknown, holes: Vec::new(), extra, found };
        }

        let at = self.position - 1;
        let mut at_tail = opened.iter().filter(|(i, _)| *i == at).peekable();
        let tail = if at_tail.peek().is_none() {
            TailState::Missing
        } else if at_tail.any(|(_, h)| h.as_slice() == self.tail.as_slice()) {
            TailState::Intact
        } else {
            TailState::Forked
        };

        // The tail index is judged above, as the tail; listing it again as a hole
        // would report one missing entry twice.
        let holes = (0..at).filter(|i| !indices.contains(i)).collect();
        Walked { tail, holes, extra, found }
    }

    pub fn to_cbor(&self) -> Result<Zeroizing<Vec<u8>>, CoreError> {
        if self.tail.len() != TAIL_LEN {
            return Err(CoreError::Seal(format!(
                "a head's tail is {TAIL_LEN} bytes, got {}",
                self.tail.len()
            )));
        }
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf)
            .map_err(|e| CoreError::Seal(format!("head encode: {e}")))?;
        Ok(Zeroizing::new(buf))
    }

    pub fn from_cbor(bytes: &[u8]) -> Result<Self, CoreError> {
        let h: Head = ciborium::from_reader(bytes)
            .map_err(|e| CoreError::Seal(format!("head decode: {e}")))?;
        if h.v != HEAD_VERSION {
            return Err(CoreError::Seal(format!(
                "head version {} (this build reads {HEAD_VERSION})",
                h.v
            )));
        }
        if h.tail.len() != TAIL_LEN {
            return Err(CoreError::Seal(format!(
                "head tail is {} bytes, not {TAIL_LEN}",
                h.tail.len()
            )));
        }
        // Position and tail are two halves of one statement. A head at position 0
        // with a tail names an entry it also says does not exist; a head past 0
        // with no tail anchors nothing, and `check` would call every walk Forked.
        // Both are refused here rather than surviving to confuse a returning
        // device about whether it has been truncated.
        let empty = h.tail() == NO_TAIL;
        if (h.position == 0) != empty {
            return Err(CoreError::Seal(format!(
                "head position {} does not agree with its tail (empty={empty})",
                h.position
            )));
        }
        Ok(h)
    }
}

/// The head key, from the storage root:
/// HKDF-SHA256(salt = domain, ikm = storage_root, info = "v1") — the one shape
/// every derivation in [`crate::locator`] takes.
///
/// From the ROOT, never the PRF, restated because it is the one property this
/// module exists to hold: someone restoring from their 24 words alone, on a
/// borrowed laptop, having never had a passkey, must be able to open this. If it is
/// ever re-derived from the PRF the words door loses its anchor and the truncation
/// check silently becomes a passkey-only feature. An ordinary account gets its
/// root from [`crate::locator::storage_root`]`(seed)`.
pub fn head_key(storage_root: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let hk = hkdf::Hkdf::<sha2::Sha256>::new(Some(HEAD_DOMAIN), storage_root);
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(b"v1", out.as_mut()).expect("32-byte OKM");
    out
}

/// Seal head bytes. Envelope: `MAGIC(4) || nonce(24) || ciphertext`, the same
/// shape as the wrap and the history so that one length check and one magic
/// serves all three at the arc.
pub fn seal_head(plain: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, CoreError> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let mut nonce = [0u8; NONCE_LEN];
    getrandom_fill(&mut nonce)?;
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad: HEAD_DOMAIN })
        .map_err(|_| CoreError::Seal("head seal failed".into()))?;
    let mut out = Vec::with_capacity(4 + NONCE_LEN + ct.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Open sealed head bytes. A wrong seed or a tampered byte fails the AEAD tag.
pub fn open_head(blob: &[u8], key: &[u8; 32]) -> Result<Zeroizing<Vec<u8>>, CoreError> {
    if blob.len() < 4 + NONCE_LEN || &blob[..4] != MAGIC {
        return Err(CoreError::Seal("not a sealed head".into()));
    }
    let cipher = XChaCha20Poly1305::new(key.into());
    cipher
        .decrypt(
            XNonce::from_slice(&blob[4..4 + NONCE_LEN]),
            Payload { msg: &blob[4 + NONCE_LEN..], aad: HEAD_DOMAIN },
        )
        .map(Zeroizing::new)
        .map_err(|_| CoreError::Seal("head open failed (wrong seed or tampered)".into()))
}

/// Seal a head under the storage root. The call a device makes before `PUT`.
pub fn seal(head: &Head, storage_root: &[u8; 32]) -> Result<Vec<u8>, CoreError> {
    seal_head(&head.to_cbor()?, &head_key(storage_root))
}

/// Open a head under the storage root. The call a returning device makes after
/// `GET` — and the one that needs nothing but the 24 words.
pub fn open(blob: &[u8], storage_root: &[u8; 32]) -> Result<Head, CoreError> {
    Head::from_cbor(&open_head(blob, &head_key(storage_root))?)
}

/// Open a head and hold the arc to the position it declared in the clear.
///
/// The arc orders writes on a number it cannot decrypt, so a dishonest or broken
/// one could serve position 41 in the header and the ciphertext of position 17 in
/// the body — and a client that trusted the header would believe its chain was
/// current. Checking the two against each other costs one comparison and removes
/// the entire class.
pub fn open_declared(
    blob: &[u8],
    storage_root: &[u8; 32],
    declared_position: u64,
) -> Result<Head, CoreError> {
    let head = open(blob, storage_root)?;
    if head.position != declared_position {
        return Err(CoreError::Seal(format!(
            "the arc declared head position {declared_position} and served position {}",
            head.position
        )));
    }
    Ok(head)
}

/// Entropy without dragging `rand_core` into a no-`storage` build — the same route
/// [`crate::wrap`] and [`crate::spine`] take, for the same reason. (It said
/// `backup` until 20 September 2026; that module was the history escrow and is
/// gone.)
pub(crate) fn getrandom_fill(buf: &mut [u8]) -> Result<(), CoreError> {
    getrandom::getrandom(buf).map_err(|e| CoreError::Seal(format!("entropy: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARC: &str = "arc.kenjin.cc";

    fn sample(position: u64, tail: u8) -> Head {
        Head::new(ARC, position, [tail; TAIL_LEN], 0)
    }

    #[test]
    fn round_trips_under_the_root_and_refuses_the_wrong_one() {
        let root = [0x42u8; 32];
        let sealed = seal(&Head::new(ARC, 41, [0x7b; TAIL_LEN], 3), &root).unwrap();
        assert_eq!(&sealed[..4], b"PHD1");

        let back = open(&sealed, &root).unwrap();
        assert_eq!(back.position, 41);
        assert_eq!(back.tail(), [0x7bu8; 32]);
        assert_eq!(back.arc, ARC);
        assert_eq!(back.chain_gen, 3, "the generation survives the seal");

        assert!(open(&sealed, &[0x43u8; 32]).is_err(), "wrong root, no head");
        let mut tampered = sealed.clone();
        let n = tampered.len();
        tampered[n - 1] ^= 1;
        assert!(open(&tampered, &root).is_err(), "one flipped byte, no head");
    }

    #[test]
    fn the_head_key_is_neither_of_the_other_two() {
        // Pinned as inequalities off ONE set of 32 bytes: no two domain keys may
        // converge, or a party holding one opens the others. The history key was
        // the third arm and went with the blob it sealed; the account key is a
        // live domain off the same root and stands in its place.
        let bytes = [0x51u8; 32];
        assert_ne!(*head_key(&bytes), *crate::wrap::wrap_key(&bytes));
        assert_ne!(
            head_key(&bytes).to_vec(),
            crate::locator::acct_pk(&bytes).to_vec()
        );
    }

    #[test]
    fn the_head_key_is_in_the_storage_family_and_hangs_off_the_root() {
        // five-things.html §06: `storage root → locators · acct · head key · history
        // key`, and the two HKDF families disjoint. A head sealed under the seed
        // itself would open for an ordinary account and never exist for a BYO one,
        // which has a root and no seed — the quiet two-answers failure §06 was
        // written to prevent before anything is stored.
        assert!(HEAD_DOMAIN.starts_with(b"pacific/storage/"), "the head key is a storage-lane key");
        let seed = [0x61u8; 32];
        let root = crate::locator::storage_root(&seed);
        assert_ne!(*head_key(&root), *head_key(&seed), "the root, not the seed, is the input");
        let sealed = seal(&sample(2, 0x01), &root).unwrap();
        assert!(open(&sealed, &seed).is_err(), "the seed itself opens nothing");
    }

    #[test]
    fn both_doors_arrive_at_the_same_head() {
        // G6, in the core: axel has lost his phone. The words door and the passkey
        // door are two routes to ONE seed, therefore one storage root, therefore one
        // head key — so the laptop that only ever saw 24 words opens the same anchor
        // as the phone that had a passkey. If a head were ever sealed under the PRF
        // instead, the first of these two opens would be impossible and this test is
        // what would say so.
        use crate::locator::storage_root;
        let seed = [0xa5u8; 32];
        let prf = [0x0fu8; 32];

        let sealed_head = seal(&sample(41, 0x7b), &storage_root(&seed)).unwrap();
        let wrap_blob = crate::wrap::seal(&prf, &seed, ARC).unwrap();

        // Door one: words → seed → root → head. No passkey anywhere in this line.
        let by_words = open(&sealed_head, &storage_root(&seed)).unwrap();

        // Door two: passkey → PRF → wrap → seed → root → head.
        let recovered_seed = crate::wrap::open(&prf, &wrap_blob, ARC).unwrap();
        let by_passkey = open(&sealed_head, &storage_root(&recovered_seed)).unwrap();

        assert_eq!(by_words, by_passkey);
        assert_eq!(by_words.position, 41);
    }

    #[test]
    fn a_v1_head_is_refused_by_name() {
        // This build reads v2 only, and must say so rather than misread a v1 map:
        // v1 carried `updated_at` and no `chain_gen`. None was ever written at a
        // locator, so there is nothing to migrate — only a message to get right.
        #[derive(Serialize)]
        struct V1 {
            v: u32,
            arc: String,
            position: u64,
            #[serde(with = "serde_bytes")]
            tail: Vec<u8>,
            updated_at: u64,
        }
        let mut buf = Vec::new();
        ciborium::into_writer(
            &V1 { v: 1, arc: ARC.into(), position: 0, tail: NO_TAIL.to_vec(), updated_at: 1 },
            &mut buf,
        )
        .unwrap();
        let err = Head::from_cbor(&buf).unwrap_err().to_string();
        assert!(err.contains("head"), "refused, and says what: {err}");
    }

    #[test]
    fn a_walk_that_stops_short_is_truncation_and_says_how_short() {
        // The thing a chain cannot catch about itself: a prefix of a valid chain
        // is a valid chain, so only an anchor outside it can say "there was more".
        let head = sample(41, 0x7b);
        assert_eq!(head.check(41, &[0x7b; 32]), ChainVerdict::Whole);
        assert_eq!(head.check(17, &[0x99; 32]), ChainVerdict::Truncated { missing: 24 });
        assert_eq!(head.check(40, &[0x7b; 32]), ChainVerdict::Truncated { missing: 1 });
    }

    #[test]
    fn one_position_with_two_tails_is_a_fork_not_a_truncation() {
        let head = sample(41, 0x7b);
        assert_eq!(head.check(41, &[0x7c; 32]), ChainVerdict::Forked);
    }

    #[test]
    fn a_walk_past_the_head_is_a_stale_anchor_and_not_an_accusation() {
        // A client that appended and died before writing the head leaves exactly
        // this. Reporting it as tampering would train people to ignore the alarm.
        let head = sample(41, 0x7b);
        assert_eq!(head.check(43, &[0xaa; 32]), ChainVerdict::HeadBehind { extra: 2 });
    }

    #[test]
    fn an_empty_chain_has_position_zero_and_no_tail_and_the_two_must_agree() {
        let root = [0x11u8; 32];
        let empty = Head::empty(ARC, 1);
        let sealed = seal(&empty, &root).unwrap();
        assert_eq!(open(&sealed, &root).unwrap().position, 0);

        // A position with no tail anchors nothing; a tail at position 0 names an
        // entry the same head says does not exist. Neither survives decode.
        let mut lying = Head::empty(ARC, 1);
        lying.position = 9;
        let plain = lying.to_cbor().unwrap();
        assert!(Head::from_cbor(&plain).is_err(), "position 9 with no tail");

        let plain = sample(0, 0x7b).to_cbor().unwrap();
        assert!(Head::from_cbor(&plain).is_err(), "a tail at position 0");
    }

    #[test]
    fn a_lying_declared_position_is_caught_against_the_sealed_one() {
        // The arc orders on a number it cannot read. This is the check that keeps
        // it honest about the one it hands back.
        let root = [0x33u8; 32];
        let sealed = seal(&sample(41, 0x7b), &root).unwrap();
        assert!(open_declared(&sealed, &root, 41).is_ok());
        assert!(
            open_declared(&sealed, &root, 42).is_err(),
            "header said 42, the ciphertext says 41"
        );
    }

    #[test]
    fn the_three_magics_do_not_open_each_others_envelopes() {
        let root = [0u8; 32];
        let wrap = crate::wrap::seal(&[0u8; 32], &[1u8; 32], ARC).unwrap();
        assert!(open(&wrap, &root).is_err(), "PWR1 is not PHD1");
        assert!(open(b"", &root).is_err(), "an empty body is not a head");
        assert!(open(b"PHD1short", &root).is_err(), "the magic alone is not an envelope");
        // PHS1 was the history envelope and is gone. What the arm proved —
        // another magic does not open as a head — is still proved by PWR1 above,
        // and by the bare magic below it.
    }

    #[test]
    fn two_seals_of_one_head_differ() {
        // Random nonce per seal. Without it, re-writing the same head produces
        // byte-identical ciphertext and a WITNESS watching the account record
        // cannot tell "written again" from "never moved".
        let root = [0x88u8; 32];
        let head = sample(41, 0x7b);
        assert_ne!(seal(&head, &root).unwrap(), seal(&head, &root).unwrap());
    }

    /// A head sealed by THIS code, frozen. Every other reader must agree with it byte
    /// for byte — the label, the magic, the nonce length, the AAD and the CBOR field
    /// names are all pinned by this one constant.
    /// `business/website/auth/tests/test_head.py` opens the same bytes from the
    /// other side with an independent Python implementation; if either drifts, one
    /// of the two goes red. (The auth SERVICE never opens a head — it judges magic,
    /// length and the declared position. It is that test which reads one.)
    ///
    /// Seed: `a5` × 32 — axel's words — through `locator::storage_root`. Head: v2,
    /// arc `arc.kenjin.cc`, position 41, tail `7b` × 32, chain_gen 3.
    const FIXTURE: &str = "50484431ea9cf745c6a793ef06107373e7cb64518274defaa191fe02b41fffe7f5ad780f25740dea8a2828ed624fdd5f6a49363971ebeaba1830ff3a8218f284b27c588c3387ecd02caee4fd636c8ac17876f7fd645e178f6d2a6361d2ca2a7effdd4d45d1dc67e9ebe5d7a3b8c019c1b57504996d48baaf069a0eca467e6a";

    #[test]
    fn the_frozen_fixture_still_opens() {
        let blob = hex::decode(FIXTURE).expect("fixture is hex");
        let root = crate::locator::storage_root(&[0xa5u8; 32]);
        let head = open(&blob, &root).expect("the frozen head still opens");
        assert_eq!(head.v, HEAD_VERSION);
        assert_eq!(head.arc, ARC);
        assert_eq!(head.position, 41);
        assert_eq!(head.tail(), [0x7bu8; 32]);
        assert_eq!(head.chain_gen, 3);
    }
}

#[cfg(test)]
mod judge_tests {
    use super::*;

    fn h(i: u8) -> [u8; TAIL_LEN] {
        tail_hash(&[i; 40])
    }
    fn head(position: u64, tail_of: u8) -> Head {
        Head::new("arc", position, h(tail_of), 0)
    }
    fn walk(idx: &[u8]) -> Vec<(u64, [u8; TAIL_LEN])> {
        idx.iter().map(|&i| (i as u64, h(i))).collect()
    }

    #[test]
    fn position_zero_is_unknown_even_when_entries_opened() {
        // THE HEAD-0 TRAP. Every account's head is the first one, at 0. If 0 meant
        // "empty", five entries past it would read as a lagging head — benign —
        // and a cut tail would read as complete.
        let w = Head::empty("arc", 0).judge(&walk(&[0, 1, 2, 3, 4]));
        assert_eq!(w.tail, TailState::Unknown);
        assert_eq!(w.found, 5);
        assert!(w.holes.is_empty());
    }

    #[test]
    fn a_whole_chain() {
        let w = head(3, 2).judge(&walk(&[0, 1, 2]));
        assert_eq!(w, Walked { tail: TailState::Intact, holes: vec![], extra: 0, found: 3 });
    }

    #[test]
    fn a_hole_in_the_middle_is_a_hole_and_not_a_truncation() {
        let w = head(3, 2).judge(&walk(&[0, 2]));
        assert_eq!(w.tail, TailState::Intact, "the tail the head names is present");
        assert_eq!(w.holes, vec![1], "and index 1 is named as lost, not counted");
        // What the count-based check says of the same walk — why `judge` exists.
        assert_eq!(head(3, 2).check(2, &h(2)), ChainVerdict::Truncated { missing: 1 });
    }

    #[test]
    fn a_missing_tail_is_the_truncation() {
        let w = head(3, 2).judge(&walk(&[0, 1]));
        assert_eq!(w.tail, TailState::Missing);
        assert!(w.holes.is_empty(), "the tail is not also listed as a hole");
    }

    #[test]
    fn a_different_entry_at_the_tail_is_a_fork() {
        let w = head(3, 2).judge(&[(0, h(0)), (1, h(1)), (2, h(9))]);
        assert_eq!(w.tail, TailState::Forked);
    }

    #[test]
    fn entries_past_the_head_are_extra_and_not_an_alarm() {
        let w = head(3, 2).judge(&walk(&[0, 1, 2, 3, 4]));
        assert_eq!(w.tail, TailState::Intact);
        assert_eq!(w.extra, 2);
        assert_eq!(w.found, 5);
    }

    #[test]
    fn a_hole_under_a_lagging_head_is_not_a_fork() {
        let w = head(3, 2).judge(&walk(&[0, 2, 3]));
        assert_eq!(w, Walked { tail: TailState::Intact, holes: vec![1], extra: 1, found: 3 });
        // The counts agree (3 walked, position 3), so the count-based check compares
        // the LAST entry that opened — index 3 — against the head's tail, and accuses.
        assert_eq!(head(3, 2).check(3, &h(3)), ChainVerdict::Forked);
    }

    #[test]
    fn two_entries_at_the_tail_index_are_intact_if_either_is_the_one() {
        // Two of a person's devices can publish at one index until indices are
        // claimed account-wide. The head names one of them; the other is not a fork.
        let w = head(3, 2).judge(&[(0, h(0)), (1, h(1)), (2, h(7)), (2, h(2))]);
        assert_eq!(w.tail, TailState::Intact);
        assert_eq!(w.found, 3, "counted by index, not by entry");
    }
}
