//! Locators — the addresses a person's records live at, and the one key that
//! writes them.
//!
//! WHAT THIS REPLACES. Every private record at the arc is addressed today by
//! `identity_pk`: `/auth/users/{pk}`, `/auth/users/{pk}/head`, and so on. The pk
//! is the account's NAME — it signs challenges, it is the MLS credential, it is
//! what peers pin — and using the name as the address makes the arc a census.
//! Anyone holding a public key can probe for the person behind it, and a breach
//! of the arc yields the roll of everyone on it, joinable to whatever else the
//! same key appears beside.
//!
//! A locator is 32 bytes of HKDF output over the seed. It carries no identity,
//! it is not computable from the pk, and the arc cannot go the other way: it
//! learns where to put bytes and never whose they are. Holding both halves of a
//! person — their pk from a public roster, their locator from a stolen database
//! — the arc still cannot tell they are the same person.
//!
//! `pk` does not go away. It stops being an ADDRESS and stays the identity.
//!
//! ## The two roots, and why the labels look the way they do
//!
//! ```text
//!          ┌─► identity root ──► pk · next · the rotation ladder
//!   seed ──┤
//!          └─► storage root ──► locators · acct · head key · history key
//! ```
//!
//! `five-things.html` §06, ruled 16 September 2026 and carved out of the deferred
//! BYO work as the one part that must NOT be deferred: **choose the two HKDF
//! label families now — `pacific/identity/…` and `pacific/storage/…`, disjoint —
//! because labels that mix them force a derivation version on every existing
//! account the day BYO is picked up.** Nothing needs splitting for an ordinary
//! account today; the point is that the labels are already the right shape when
//! it does.
//!
//! So everything here hangs off the STORAGE ROOT, and the entry point takes a
//! storage root rather than a seed. That is what makes BYO additive: an account
//! that brought its own keypair has no seed and still has a storage root, and
//! downstream of the two roots nothing differs between the account types.
//!
//! ## The set
//!
//! ```text
//!   storage_root = HKDF(seed, "pacific/storage/root/v1")   or minted, for BYO
//!   acct         = Ed25519(HKDF(storage_root, "pacific/storage/acct/v1"))
//!
//!              read address                                written by
//!   wrap       HKDF(prf,          "…/storage/wrap/v1")     acct  ← the exception
//!   head       HKDF(storage_root, "…/storage/head/v1")     acct
//!   history    HKDF(storage_root, "…/storage/history/v1")  —     ← RETIRED; nothing writes
//!   index      HKDF(storage_root, "…/storage/index/v1")    acct
//!   chain      HKDF(storage_root, "…/storage/chain/v1" ‖ g ‖ i) acct  ← entry i of generation g
//! ```
//!
//! THE WRAP IS DERIVED FROM THE PRF AND NOT THE STORAGE ROOT, and that asymmetry
//! is the point rather than an oversight. The wrap exists for exactly one caller:
//! a device holding a passkey and no seed. It cannot be found at a root-derived
//! address by someone who does not have the root — which is everyone the wrap is
//! for. Read from the PRF, write from the root: you need only the passkey to USE
//! the account, and the root to enrol or revoke a credential.
//!
//! THE CHAIN IS ADDRESSED PER GENERATION. The head carries `chain_gen`, and a
//! restarted chain — pruned, begun again after a key concern, or abandoned at a
//! lost middle — starts at a fresh address rather than appending to the old one.
//! So there is no plain chain locator: [`locator`] refuses [`Record::Chain`] the
//! way it refuses the wrap, and [`chain_locator`] takes the generation explicitly.
//! An address that silently meant "generation 0" would put a device reading
//! generation 3 on an empty row with nothing to say it was the wrong one.
//!
//! THE CHAIN LOCATOR ADDRESSES EVERY ENTRY. Entry `i` of generation `g` is at
//! [`chain_locator`]`(root, g, i)` — arithmetic, ruled 20 September 2026, so a lost
//! entry costs its own index and not the tail. The unlinkability the earlier
//! linked form was chosen for still holds: each index lands at an unrelated HKDF
//! output, so the blind relay sees no stable per-user tag. What `Keys` §6 forbade
//! was ONE derived tag accumulating an entry per epoch; a family of addresses is
//! not that. [`next_tag`] is left over from the linked form — still exported to
//! both platforms, called by nothing in core.
//!
//! ## Why this lives in core and nowhere else
//!
//! Two derivations of one address put two of a person's devices on different rows
//! and NOTHING ANYWHERE REPORTS AN ERROR — the second device simply finds an empty
//! address and concludes the account is new. There is no test that catches it and
//! no log line that names it. So the label set has exactly one definition, here,
//! and every client reaches it through the wasm or the FFI rather than computing
//! HKDF for itself. That discipline is the whole reason the module exists.
//!
//! ## What a locator does not do
//!
//! It is an ADDRESS, not a capability and not a key. Learning one lets you fetch
//! the ciphertext stored there, which is what the wrap's own unauthenticated read
//! already concedes on purpose. Nothing at a locator opens without the seed or
//! the PRF, and nothing at a locator can be overwritten without `acct`.

use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use zeroize::Zeroizing;
use crate::CoreError;

/// HKDF label for the storage root. The `pacific/storage/` family's own root —
/// everything else in this module is derived from what this produces.
pub const STORAGE_ROOT_LABEL: &[u8] = b"pacific/storage/root/v1";

/// HKDF label for the identity root. Here only so the two families are declared
/// in one place and can be seen to be disjoint; `identity.rs` owns what is
/// derived from it.
pub const IDENTITY_ROOT_LABEL: &[u8] = b"pacific/identity/root/v1";

/// HKDF label for the arc write key. Under `pacific/storage/` like everything
/// else off this root: `acct` is a storage-lane authority and has no relation to
/// the identity key, which is the property the whole scheme is chosen for.
pub const ACCT_LABEL: &[u8] = b"pacific/storage/acct/v1";

/// The records a person has at an arc. The enum, rather than loose strings, is
/// what stops a caller inventing a sixth label that only their build agrees with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Record {
    /// The seed under the passkey. Addressed from the PRF, not the seed.
    Wrap,
    /// Where the chain ends: a position and the tail hash. See [`crate::head`].
    Head,
    /// RETIRED, 20 September 2026 — an address with nothing to put at it.
    ///
    /// This named where the account backup went. What went there was
    /// `backup::History`: the whole plaintext archive and `mls_mem::Snapshot`,
    /// every group's ratchet tree, epoch secrets and key-package private halves.
    /// Re-admission §1 called it "a complete key escrow"; the register retired it
    /// on 14 September (Leaves §02, §04) and the code that built, sealed, opened
    /// and unpacked it is now gone from `pacific-core`, `pacific-ffi` and
    /// `core-wasm` alike.
    ///
    /// The variant stays because it is in the locators bundle that Swift and the
    /// browser both read, and dropping it is an API change rather than a removal.
    /// It computes an address and nothing writes there. **Nothing may.** If a
    /// future pass takes it out of the bundle, take it out of here too; until
    /// then, treat a non-empty blob at this address as a stale artefact to delete
    /// at the Arc, never as something to read.
    History,
    /// The list of live wrap locators — how a seed-holder enumerates and revokes
    /// credentials it has never held the PRF for.
    Index,
    /// EVERY chain entry of one generation, each at its own computed address. Addressed
    /// by [`chain_locator`] with the generation the head names and the entry's
    /// index — [`locator`] refuses it, because one address cannot answer for a
    /// family.
    ///
    /// Arithmetic, not linked, ruled 20 September 2026: an entry that is lost
    /// costs ITS OWN INDEX and not the tail. The earlier form made only the first
    /// address derivable and carried the next one inside each entry, so a break at
    /// entry k took every entry after it — which `Catching Up` §01 objected to in
    /// exactly those words: "a break at entry k costs every entry after it. That
    /// is membership."
    Chain,
}

impl Record {
    /// The HKDF info string. These bytes are wire: changing one moves every
    /// account's record to a new address with no migration and no error.
    pub fn label(self) -> &'static [u8] {
        match self {
            Record::Wrap => b"pacific/storage/wrap/v1",
            Record::Head => b"pacific/storage/head/v1",
            Record::History => b"pacific/storage/history/v1",
            Record::Index => b"pacific/storage/index/v1",
            Record::Chain => b"pacific/storage/chain/v1",
        }
    }

    /// Is this record addressed from the PRF rather than the seed? Exactly one
    /// is, and [`locator`] refuses it rather than deriving a second address for
    /// the wrap that no passkey-only device could ever find.
    pub fn from_prf(self) -> bool {
        matches!(self, Record::Wrap)
    }

    /// Is this record addressed per chain generation? Exactly one is, and
    /// [`locator`] refuses it rather than quietly answering for generation 0.
    pub fn per_generation(self) -> bool {
        matches!(self, Record::Chain)
    }

    /// Every record, for callers that enumerate — a test, an arc migration, a
    /// device deleting an account. Ordered as declared.
    pub fn all() -> [Record; 5] {
        [Record::Wrap, Record::Head, Record::History, Record::Index, Record::Chain]
    }

    /// The record's short name, for a message a person reads. The LABEL is the
    /// wire value and is deliberately not this — a label carries its family and
    /// its version, and an error saying `pacific/storage/head/v1` where it meant
    /// "head" is an error nobody finishes reading.
    pub fn as_str(self) -> &'static str {
        match self {
            Record::Wrap => "wrap",
            Record::Head => "head",
            Record::History => "history",
            Record::Index => "index",
            Record::Chain => "chain",
        }
    }
}

/// One KDF, one shape: `HKDF-SHA256(salt = label, ikm, info = "v1")`.
///
/// The LABEL is the salt and the info is a constant. That is deliberate and it is
/// the shape `p1_resume_from_words` already uses: it puts the whole of the
/// domain separation in one string, so a label can be read off a line of code and
/// compared against the ruling without also having to know what was passed as
/// info somewhere else.
fn kdf(ikm: &[u8], label: &[u8]) -> [u8; 32] {
    let hk = hkdf::Hkdf::<sha2::Sha256>::new(Some(label), ikm);
    let mut out = [0u8; 32];
    hk.expand(b"v1", &mut out).expect("32-byte OKM");
    out
}

/// The storage root: everything in this module descends from it.
///
/// An ordinary account derives it from the seed. A BYO account MINTS one and
/// wraps it, having no seed to derive from — which is the whole reason the rest
/// of this module takes a root rather than a seed, and the whole reason §06 says
/// to choose the families before anything is written rather than after.
pub fn storage_root(seed: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(kdf(seed, STORAGE_ROOT_LABEL))
}

/// The identity root — `pk`, its successor, and the rotation ladder.
///
/// Here so the two families are declared together and can be SEEN to be disjoint.
/// Nothing in this module derives from it, and nothing ever should: a storage
/// root cannot reconstruct an identity key and an identity key cannot reach a
/// storage row, which is the asymmetry BYO depends on.
pub fn identity_root(seed: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(kdf(seed, IDENTITY_ROOT_LABEL))
}

/// The address of a record, from the storage root.
///
/// Refuses [`Record::Wrap`]: a root-derived wrap address would be one no
/// passkey-only device could compute, which is the only kind of device the wrap
/// exists for. Use [`wrap_locator`].
pub fn locator(storage_root: &[u8; 32], record: Record) -> Result<[u8; 32], CoreError> {
    if record.from_prf() {
        return Err(CoreError::Seal(format!(
            "the {} locator comes from the passkey PRF, not the storage root",
            record.as_str()
        )));
    }
    if record.per_generation() {
        return Err(CoreError::Seal(format!(
            "the {} locator is per generation — use chain_locator(root, head.chain_gen, index)",
            record.as_str()
        )));
    }
    Ok(kdf(storage_root, record.label()))
}

/// The address of the FIRST entry of chain generation `gen`, from the storage root.
///
/// The label is [`Record::Chain`]'s followed by the generation as four big-endian
/// bytes — still the one KDF shape, with the whole of the domain separation in the
/// salt. `gen` comes from the head, which a returning device fetches first anyway:
/// that is what makes a restarted chain findable from nothing but the root.
/// WHERE AN EPOCH'S ARCHIVE KEY RECORD LIVES, computed rather than followed.
///
/// `HKDF(storage_root, "pacific/storage/archive/v1" ‖ group_id ‖ epoch_be)`. A
/// reader with the seed derives the address of any epoch directly, so nothing is
/// reached by following a link from anything else and a record that is lost costs
/// ITS OWN EPOCH and not the tail. That is the 18 September ruling — "content keys
/// by arithmetic ... a break past the prefix costs one object, not the tail".
///
/// Each epoch lands at an independent pseudorandom address, so the store sees no
/// stable per-account tag and cannot build the per-user activity profile `Keys` §6
/// objected to. That objection was aimed at ONE seed-derived tag accumulating an
/// entry per epoch; a family of addresses under HKDF is not that.
///
/// NOTHING WRITES HERE YET. `Node::recoverability` reports every epoch whose record
/// is missing, and until the writer exists that is every epoch of every object.
pub const ARCHIVE_RECORD_LABEL: &[u8] = b"pacific/storage/archive/v1";

/// The address of this account's archive key record for `(group_id, epoch)`.
pub fn archive_locator(storage_root: &[u8; 32], group_id: &[u8], epoch: u64) -> [u8; 32] {
    let mut label = ARCHIVE_RECORD_LABEL.to_vec();
    label.extend_from_slice(group_id);
    label.extend_from_slice(&epoch.to_be_bytes());
    kdf(storage_root, &label)
}

pub fn chain_locator(storage_root: &[u8; 32], gen: u32, index: u64) -> [u8; 32] {
    let mut label = Record::Chain.label().to_vec();
    label.extend_from_slice(&gen.to_be_bytes());
    label.extend_from_slice(&index.to_be_bytes());
    kdf(storage_root, &label)
}

/// The address of the wrap, from the passkey's PRF output.
///
/// ONE PER CREDENTIAL, which is what makes enrolment and revocation separable:
/// each passkey lands at its own address, so enrolling a second credential does
/// not overwrite the first. What a new device cannot then do is FIND an old wrap,
/// never having held its PRF — which is what [`Record::Index`] is for, and why
/// the index must be written in the same act as the wrap. Hold a wrap you cannot
/// enumerate and you hold one you can no longer revoke.
pub fn wrap_locator(prf: &[u8; 32]) -> [u8; 32] {
    kdf(prf, Record::Wrap.label())
}

/// The arc write key, from the storage root.
///
/// FIRST-WRITE-CLAIMS is the model this key is shaped for. The arc holds no root,
/// so it cannot verify that an opaque locator belongs to anybody: the first PUT
/// to a locator records the verifying key, and every later write to that locator
/// must carry a signature under the same one. `acct` appears nowhere else in the
/// system and has no computable relation to `identity_pk` — the arc can hold both
/// halves of a person and be unable to tell.
pub fn acct_key(storage_root: &[u8; 32]) -> SigningKey {
    // Not wrapped in `Zeroizing`: ed25519-dalek's `SigningKey` is not `Zeroize`
    // (it zeroizes its own scalar on drop), and `identity.rs` holds its two keys
    // the same bare way. The 32 bytes it is built from are a temporary here.
    SigningKey::from_bytes(&kdf(storage_root, ACCT_LABEL))
}

/// The public half the arc records on a first write.
pub fn acct_pk(storage_root: &[u8; 32]) -> [u8; 32] {
    acct_key(storage_root).verifying_key().to_bytes()
}

/// Domain for what `acct` signs — the first line of the payload, exactly as
/// `identity::auth_payload`'s is, so an arc-write signature can never be replayed
/// as any other signature this system produces.
pub const ACCT_SIG_DOMAIN: &str = "pacific-arc-write:v1";

/// One write, and every field that must be inside the signature.
///
/// THIS IS A CHALLENGE-RESPONSE, NOT A DETACHED SIGNATURE, and it is shaped like
/// `identity::auth_payload` on purpose — same domain-first layout, same audience,
/// same single-use nonce. The first version of this bound only the locator and
/// the body, which looked sufficient because the body is what gets stored. It was
/// not, and both missing fields cost something real:
///
/// * **No audience** — a signature captured at one arc verifies at another. An
///   account is meant to be re-homeable, so the same locator legitimately exists
///   at more than one arc, and a write authorised for one of them was authorised
///   for all of them.
/// * **No nonce** — the signature verifies forever. Because the body is bound the
///   replay is idempotent, so this is not forgery; it is ROLLBACK. Replay a
///   captured history write and the account's stored history moves backwards, and
///   `put_history`'s guard does not stop it: that guard compares an `exported_at`
///   HEADER, which is not inside any signature, and it accepts a write carrying no
///   stamp at all. The head survives this because the arc compare-and-sets on the
///   chain position, which is a count it enforces itself. Nothing else does.
///
/// So every write takes a fresh nonce from the arc, like every other authenticated
/// route, and the arc spends it.
#[derive(Debug, Clone, Copy)]
pub struct ArcWrite<'a> {
    /// The arc this write is for. Inside the signature, so it cannot travel.
    pub audience: &'a str,
    /// The address being written.
    pub locator: &'a [u8; 32],
    /// The arc's single-use challenge. Inside the signature, so it cannot be
    /// replayed once the arc has spent it.
    pub nonce: &'a str,
    /// The bytes being stored. Inside the signature, so one captured signature is
    /// not a standing licence to replace this record with anything later.
    pub body: &'a [u8],
}

/// The bytes `acct` signs.
///
/// `domain \n audience \n <locator hex> \n nonce \n <body>`, with the body last
/// and raw so any bytes can be signed.
///
/// A NEWLINE IN THE AUDIENCE OR THE NONCE IS REFUSED, and that is not tidiness —
/// without it the encoding is not injective and the signature does not mean what
/// it appears to mean. `nonce = "n\n", body = "\n"` and `nonce = "n", body =
/// "\n\n"` produce identical bytes, so one signature authorises BOTH writes. The
/// nonce comes from the arc, which is precisely the party this scheme is defending
/// against: an arc that issued a nonce containing a newline could re-present a
/// signature the device made for one body as authorising a different one. The
/// locator is hex and the domain is ours, so these two fields are the whole of
/// the exposure.
pub fn write_payload(w: &ArcWrite) -> Result<Vec<u8>, CoreError> {
    for (what, field) in [("audience", w.audience), ("nonce", w.nonce)] {
        if field.contains('\n') {
            return Err(CoreError::Seal(format!(
                "an arc-write {what} may not contain a newline: it would make two \
                 different writes sign to the same bytes"
            )));
        }
    }
    let head = format!(
        "{ACCT_SIG_DOMAIN}\n{}\n{}\n{}\n",
        w.audience,
        hex::encode(w.locator),
        w.nonce
    );
    let mut v = Vec::with_capacity(head.len() + w.body.len());
    v.extend_from_slice(head.as_bytes());
    v.extend_from_slice(w.body);
    Ok(v)
}

/// Why a write was refused. Carried so the ARC CAN LOG IT — and only the arc.
///
/// The response must stay opaque for the reason `auth.py::verify` already gives:
/// *"a caller that could tell 'wrong key' from 'wrong nonce' could enumerate
/// accounts."* The log is the opposite case. First-write-claims fails silently by
/// design — the person simply sees a refusal — so without a record naming both
/// keys there is no way to tell a squatted address from a client bug, and that
/// distinction is the whole difference between "someone is attacking this" and
/// "we shipped a regression".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteRefusal {
    /// A different `acct` key entirely. The offered key is presented by the
    /// writer and is opaque, so logging it costs nothing and it is what separates
    /// a squat from a bug.
    NotTheClaimant { recorded: [u8; 32], offered: [u8; 32] },
    /// The right key, but the bytes do not verify — a stale nonce, the wrong
    /// audience, a mangled body, or a client signing the old payload.
    BadSignature,
    /// The recorded key is not a point on the curve. Only reachable if the stored
    /// row is corrupt, which is worth its own line in the log.
    MalformedKey(String),
    /// The request cannot be encoded unambiguously — a newline in the audience or
    /// the nonce. Its own case because it is the ARC that supplies both, so this
    /// names a broken or hostile arc rather than a bad client.
    MalformedRequest(String),
}

impl core::fmt::Display for WriteRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            WriteRefusal::NotTheClaimant { recorded, offered } => write!(
                f,
                "this locator is claimed by {}… and the write offered {}…",
                &hex::encode(recorded)[..16],
                &hex::encode(offered)[..16]
            ),
            WriteRefusal::BadSignature => {
                write!(f, "this write is not signed by the key that claimed the locator")
            }
            WriteRefusal::MalformedKey(e) => write!(f, "the recorded arc-write key is unusable: {e}"),
            WriteRefusal::MalformedRequest(e) => write!(f, "{e}"),
        }
    }
}

impl From<WriteRefusal> for CoreError {
    fn from(r: WriteRefusal) -> Self {
        CoreError::Seal(r.to_string())
    }
}

/// Sign a write. 64 bytes, to travel beside the body.
pub fn sign_write(storage_root: &[u8; 32], w: &ArcWrite) -> Result<[u8; 64], CoreError> {
    Ok(acct_key(storage_root).sign(&write_payload(w)?).to_bytes())
}

/// Check a write against the `acct` key the arc recorded on the first one.
///
/// `offered` is the key the writer presents on every write, not just the first.
/// It is checked against `recorded` BEFORE the signature, and the signature is
/// then verified against `recorded` and never against `offered` — verifying
/// against the key the writer supplied would authorise everybody.
///
/// Here rather than only in the arc so the two sides cannot drift: an arc checking
/// a slightly different payload accepts writes the signer never authorised, or
/// rejects ones it did, and either way the person is locked out of their own
/// record with no way to say why they should not be.
pub fn verify_write(
    recorded: &[u8; 32],
    offered: &[u8; 32],
    w: &ArcWrite,
    sig: &[u8; 64],
) -> Result<(), WriteRefusal> {
    if recorded != offered {
        return Err(WriteRefusal::NotTheClaimant { recorded: *recorded, offered: *offered });
    }
    let bytes = write_payload(w).map_err(|e| WriteRefusal::MalformedRequest(e.to_string()))?;
    let vk = VerifyingKey::from_bytes(recorded)
        .map_err(|e| WriteRefusal::MalformedKey(e.to_string()))?;
    vk.verify_strict(&bytes, &ed25519_dalek::Signature::from_bytes(sig))
        .map_err(|_| WriteRefusal::BadSignature)
}

/// A fresh address for the NEXT chain entry, to be sealed inside the current one.
///
/// RANDOM, NOT DERIVED, and that is the whole mechanism. A derived next-tag would
/// be computable by anyone who could compute the first, which is to say the
/// sequence would collapse back into one seed-derived address and hand the relay
/// a per-user activity profile. Unguessable-without-the-predecessor is the
/// property, and only entropy gives it.
pub fn next_tag() -> Result<[u8; 32], CoreError> {
    let mut t = [0u8; 32];
    getrandom::getrandom(&mut t).map_err(|e| CoreError::Seal(format!("entropy: {e}")))?;
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: [u8; 32] = [7u8; 32];
    const OTHER: [u8; 32] = [8u8; 32];
    const PRF: [u8; 32] = [9u8; 32];

    fn root() -> [u8; 32] { *storage_root(&SEED) }
    fn other_root() -> [u8; 32] { *storage_root(&OTHER) }

    /// The address of a root-derived record, taking the chain at generation 0.
    fn at(root: &[u8; 32], r: Record) -> [u8; 32] {
        if r.per_generation() { chain_locator(root, 0, 0) } else { locator(root, r).unwrap() }
    }

    /// §06, ruled 16 Sep 2026: the two families disjoint, and everything in this
    /// module hanging off the STORAGE root. Mixing them forces a derivation
    /// version on every existing account the day BYO lands — which is why the
    /// ruling is carved out of the deferred work as the part that must not be
    /// deferred. This is that ruling, as an assertion.
    #[test]
    fn every_label_is_in_the_storage_family() {
        for r in Record::all() {
            let l = core::str::from_utf8(r.label()).unwrap();
            assert!(l.starts_with("pacific/storage/"), "{} is labelled {l}", r.as_str());
        }
        for l in [STORAGE_ROOT_LABEL, ACCT_LABEL] {
            assert!(core::str::from_utf8(l).unwrap().starts_with("pacific/storage/"));
        }
        assert!(core::str::from_utf8(IDENTITY_ROOT_LABEL).unwrap()
                    .starts_with("pacific/identity/"));
    }

    /// The asymmetry BYO depends on: nothing derives one root from the other, so
    /// a storage root cannot reconstruct an identity key and an identity key
    /// cannot reach a storage row.
    #[test]
    fn the_two_roots_are_unrelated() {
        assert_ne!(*storage_root(&SEED), *identity_root(&SEED));
        assert_ne!(*storage_root(&SEED), SEED);
        assert_ne!(*identity_root(&SEED), SEED);
        assert_ne!(*storage_root(&SEED), *storage_root(&OTHER));
    }

    /// A BYO account has no seed and a minted root, and everything downstream
    /// must work from the root alone — which is the property that makes BYO
    /// additive rather than a second account type.
    #[test]
    fn a_minted_root_with_no_seed_behind_it_works() {
        let minted = [0x5Au8; 32];
        let l = locator(&minted, Record::Head).unwrap();
        let pk = acct_pk(&minted);
        let w = ArcWrite { audience: "arc.example", locator: &l, nonce: "n", body: b"x" };
        let sig = sign_write(&minted, &w).unwrap();
        verify_write(&pk, &pk, &w, &sig).expect("a BYO account can write its own rows");
    }

    /// The property the module exists for: an address that says nothing about
    /// whose it is, and no route back.
    #[test]
    fn a_locator_is_not_the_identity_key() {
        let id = crate::identity::Identity::in_memory(SEED).identity_pk();
        for r in Record::all() {
            if r.from_prf() {
                continue;
            }
            let l = at(&root(), r);
            assert_ne!(l, id, "{} locator is the identity key", r.as_str());
            assert_ne!(l, root(), "{} locator is the storage root", r.as_str());
        }
    }

    /// Five records, five unrelated addresses. One label colliding with another
    /// would file two records at one address and nothing would report it.
    #[test]
    fn every_record_gets_its_own_address() {
        let mut seen = std::collections::BTreeSet::new();
        for r in Record::all() {
            let l = if r.from_prf() { wrap_locator(&PRF) } else { at(&root(), r) };
            assert!(seen.insert(l), "{} collides with another record", r.as_str());
        }
        assert_eq!(seen.len(), 5);
    }

    /// Deterministic, because every one of a person's devices has to arrive at
    /// the same address from the same 24 words — and a different seed must not.
    #[test]
    fn the_same_seed_always_lands_on_the_same_address() {
        for r in Record::all() {
            if r.from_prf() {
                continue;
            }
            assert_eq!(at(&root(), r), at(&root(), r));
            assert_ne!(at(&root(), r), at(&other_root(), r));
        }
    }

    /// The wrap is addressed from the PRF and asking for it from the seed is a
    /// refusal, not a second address: a seed-derived wrap address is one the
    /// passkey-only device the wrap exists for could never compute.
    #[test]
    fn the_wrap_is_not_reachable_from_the_seed() {
        let e = locator(&root(), Record::Wrap).unwrap_err();
        assert!(format!("{e}").contains("passkey PRF"), "{e}");
        // And one per credential, so enrolling never silently overwrites.
        assert_ne!(wrap_locator(&PRF), wrap_locator(&[10u8; 32]));
    }

    /// A restarted chain starts somewhere new, and nothing answers for "the chain"
    /// without saying which. The refusal names the call to make instead.
    #[test]
    fn the_chain_is_addressed_per_generation() {
        let e = locator(&root(), Record::Chain).unwrap_err();
        assert!(format!("{e}").contains("chain_locator"), "{e}");
        let g0 = chain_locator(&root(), 0, 0);
        assert_eq!(g0, chain_locator(&root(), 0, 0), "every device lands on the same first entry");
        assert_ne!(g0, chain_locator(&root(), 1, 0), "a new generation is a new address");
        assert_ne!(g0, chain_locator(&root(), 0, 1), "and a new index is a new address");
        assert_ne!(g0, chain_locator(&other_root(), 0, 0));
        for r in [Record::Head, Record::History, Record::Index] {
            assert_ne!(chain_locator(&root(), 1, 0), locator(&root(), r).unwrap());
        }
    }

    /// `acct` governs every write and must not be computable from anything the
    /// arc already holds — the identity key included.
    #[test]
    fn the_write_key_is_unrelated_to_the_identity_key() {
        let id = crate::identity::Identity::in_memory(SEED).identity_pk();
        assert_ne!(acct_pk(&root()), id);
        assert_ne!(acct_pk(&root()), root());
        assert_ne!(acct_pk(&root()), acct_pk(&other_root()));
        for r in Record::all() {
            if !r.from_prf() {
                assert_ne!(acct_pk(&root()), at(&root(), r));
            }
        }
    }

    fn w<'a>(loc: &'a [u8; 32], nonce: &'a str, body: &'a [u8]) -> ArcWrite<'a> {
        ArcWrite { audience: "arc.example", locator: loc, nonce, body }
    }

    /// First-write-claims, both halves.
    #[test]
    fn a_second_writer_cannot_move_a_claimed_record() {
        let l = locator(&root(), Record::Head).unwrap();
        let mine = acct_pk(&root());
        let sig = sign_write(&root(), &w(&l, "n1", b"a sealed head")).unwrap();
        verify_write(&mine, &mine, &w(&l, "n1", b"a sealed head"), &sig).expect("the claimant writes");

        // Somebody else, signing the same bytes at the same address with their key.
        let theirs = acct_pk(&other_root());
        let their_sig = sign_write(&other_root(), &w(&l, "n1", b"a sealed head")).unwrap();
        assert_eq!(
            verify_write(&mine, &theirs, &w(&l, "n1", b"a sealed head"), &their_sig),
            Err(WriteRefusal::NotTheClaimant { recorded: mine, offered: theirs }),
            "a squat must be distinguishable from a bug in the log"
        );
        // And presenting the claimant's key while holding someone else's secret
        // must not work either — the signature is checked against `recorded`.
        assert_eq!(
            verify_write(&mine, &mine, &w(&l, "n1", b"a sealed head"), &their_sig),
            Err(WriteRefusal::BadSignature)
        );
    }

    /// The body is bound in, so a captured signature does not become a licence to
    /// replace the record with anything later.
    #[test]
    fn a_signature_does_not_travel_to_other_bytes_or_other_records() {
        let head = locator(&root(), Record::Head).unwrap();
        let hist = locator(&root(), Record::History).unwrap();
        let me = acct_pk(&root());
        let sig = sign_write(&root(), &w(&head, "n1", b"one")).unwrap();
        assert!(verify_write(&me, &me, &w(&head, "n1", b"two"), &sig).is_err(), "body not bound");
        assert!(verify_write(&me, &me, &w(&hist, "n1", b"one"), &sig).is_err(), "address not bound");
    }

    /// THE ROLLBACK GUARD. Without the nonce a captured write verifies forever,
    /// and because the body is bound the replay is idempotent — which is not
    /// forgery but IS a rollback: replay a captured history write and the stored
    /// history moves backwards. `put_history` cannot stop it, because its guard
    /// reads an unsigned header and accepts a write carrying no stamp at all.
    #[test]
    fn a_spent_nonce_cannot_be_replayed() {
        let l = locator(&root(), Record::History).unwrap();
        let me = acct_pk(&root());
        let sig = sign_write(&root(), &w(&l, "challenge-1", b"yesterday")).unwrap();
        verify_write(&me, &me, &w(&l, "challenge-1", b"yesterday"), &sig).unwrap();
        assert_eq!(
            verify_write(&me, &me, &w(&l, "challenge-2", b"yesterday"), &sig),
            Err(WriteRefusal::BadSignature),
            "the same bytes replayed under a fresh challenge must not verify"
        );
    }

    /// An account is re-homeable, so one locator legitimately exists at more than
    /// one arc — and a write authorised for one of them was authorised for all of
    /// them until the audience went inside the signature.
    #[test]
    fn a_write_authorised_for_one_arc_does_not_verify_at_another() {
        let l = locator(&root(), Record::Head).unwrap();
        let me = acct_pk(&root());
        let here = ArcWrite { audience: "arc.example", locator: &l, nonce: "n1", body: b"h" };
        let there = ArcWrite { audience: "other.example", locator: &l, nonce: "n1", body: b"h" };
        let sig = sign_write(&root(), &here).unwrap();
        verify_write(&me, &me, &here, &sig).unwrap();
        assert_eq!(verify_write(&me, &me, &there, &sig), Err(WriteRefusal::BadSignature));
    }

    /// THE ENCODING MUST BE INJECTIVE, and it is not for free. `nonce = "n\n",
    /// body = "\n"` and `nonce = "n", body = "\n\n"` build the same bytes, so one
    /// signature would authorise both writes — and the NONCE COMES FROM THE ARC,
    /// which is the party this scheme defends against. An arc issuing a nonce with
    /// a newline in it could re-present a signature the device made for one body
    /// as authorising a different one. Found by the Python mirror's own
    /// injectivity assertion, which is why that assertion is in the test and not
    /// just in a comment.
    #[test]
    fn a_newline_in_the_nonce_or_the_audience_is_refused() {
        let l = locator(&root(), Record::Head).unwrap();
        let sneaky = ArcWrite { audience: "a", locator: &l, nonce: "n\n", body: b"\n" };
        let plain = ArcWrite { audience: "a", locator: &l, nonce: "n", body: b"\n\n" };
        assert!(write_payload(&sneaky).is_err(), "an ambiguous nonce must not encode");
        assert!(write_payload(&plain).is_ok(), "and the unambiguous one still must");
        assert!(sign_write(&root(), &sneaky).is_err());
        let me = acct_pk(&root());
        assert!(matches!(
            verify_write(&me, &me, &sneaky, &[0u8; 64]),
            Err(WriteRefusal::MalformedRequest(_))
        ), "a broken arc is its own case, not a bad client");
        assert!(write_payload(&ArcWrite { audience: "a\nb", locator: &l, nonce: "n", body: b"" }).is_err());
    }

    /// Unguessable without the predecessor. A derived next-tag would collapse the
    /// chain back into one address and hand the relay a per-user profile.
    #[test]
    fn next_tags_are_not_a_sequence_anyone_can_follow() {
        let a = next_tag().unwrap();
        let b = next_tag().unwrap();
        assert_ne!(a, b);
        assert_ne!(a, [0u8; 32]);
        for r in Record::all() {
            if !r.from_prf() {
                assert_ne!(a, at(&root(), r));
            }
        }
    }
}
