//! Self-certifying relay addresses: an address IS a public key, and a write to it
//! is a signature by that key.
//!
//! WHY. The relay is anonymous and blind, and until this module a `Pub` carried
//! nothing but a tag and a blob. Anyone who learned a tag — the operator, anyone
//! watching a subscriber, any past member of a group — could append to it, and one
//! actor could wedge a group for ever by taking its epoch's commit slot with junk.
//! Sealing protects what a blob SAYS; nothing protected WHERE it could be put.
//!
//! THE RULE, and it is the only one. A tag is an Ed25519 verifying key. A `Pub` to
//! it carries a signature over [`pub_payload`] by the matching signing key. The
//! relay checks the signature against the tag itself — no registration, no claim
//! table, no first write to race, no round trip — and refuses anything else. So a
//! party can publish only to addresses whose secret it holds:
//!
//! ```text
//!   seed (a secret the writer already has) ──HKDF──► signing key ──► verifying key = the tag
//! ```
//!
//! Who holds a seed is decided where the seed comes from, not here: an MLS epoch's
//! exporter (every member at that epoch, nobody else), a contact bundle's intro
//! secret (anyone the person gave the bundle to), the storage root (the person's
//! own devices). The address is exactly as unknowable as before — it exists the
//! moment its seed does — and exactly as opaque to the relay: 32 bytes that name no
//! one.
//!
//! WHY NO NONCE AND NO AUDIENCE, unlike `identity::auth_payload`. A challenge
//! exists to stop a captured signature being replayed. Here replay is harmless ON
//! ONE CONDITION THE RELAY MUST KEEP: it stores one copy of a given blob at a given
//! address, so the same signed blob republished adds nothing. That rule is the
//! relay's, beside the signature check (`store.rs`); without it one captured
//! publish is an unlimited number of stored ones. Otherwise a signature cannot be
//! moved to another address, because the tag is inside the signed bytes, and
//! carrying it to another arc only copies the owner's data there — which re-homing
//! an account needs anyway. So the signature is made once, when the blob is
//! sealed, and a publish stays one message.
//!
//! THE BLOB IS SIGNED AS IT TRAVELS — the base64 string — because the relay never
//! decodes a blob and must not start: what is signed is exactly what is stored and
//! replayed.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use crate::Frame;

/// The first bytes of everything an address key signs, so a relay-write signature
/// can never be taken for any other signature the system produces.
pub const PUB_SIG_DOMAIN: &[u8] = b"pacific-relay-pub:v1\n";

/// HKDF salt turning a seed into an address's signing key. The seed is a secret
/// the writer already holds for another reason (an exporter output, an intro
/// secret, a root-derived value), so the key is separated from every other use of
/// those bytes rather than being them.
pub const ADDRESS_KEY_LABEL: &[u8] = b"pacific/relay/address/v1";

/// An address, with the key that writes it.
///
/// `Clone` is deliberately absent: a signing key is copied only by deriving it
/// again from its seed, which keeps the one place it can come from obvious.
pub struct Address {
    key: SigningKey,
}

impl Address {
    /// The address a seed names. Deterministic: every holder of the seed derives
    /// the same address and the same key.
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        let hk = hkdf::Hkdf::<sha2::Sha256>::new(Some(ADDRESS_KEY_LABEL), seed);
        let mut sk = [0u8; 32];
        hk.expand(b"v1", &mut sk).expect("32-byte OKM");
        let key = SigningKey::from_bytes(&sk);
        sk.iter_mut().for_each(|b| *b = 0);
        Address { key }
    }

    /// The tag: the verifying key's 32 bytes. What a subscriber asks for and what
    /// the relay files the blob under.
    pub fn tag(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }

    /// The tag as the `Frame` field spells it.
    pub fn tag_hex(&self) -> String {
        crate::tag_hex(&self.tag())
    }

    /// The signature a `Pub` of `blob_b64` to this address carries, hex.
    pub fn sign_pub(&self, blob_b64: &str) -> String {
        hex::encode(self.key.sign(&pub_payload(&self.tag(), blob_b64)).to_bytes())
    }

    /// A complete, signed `Pub` of `blob_b64` to this address.
    pub fn pub_frame(&self, blob_b64: String, commit: bool) -> Frame {
        let sig = self.sign_pub(&blob_b64);
        Frame::Pub { tag: self.tag_hex(), blob: blob_b64, commit, sig }
    }
}

/// The bytes an address key signs: domain, the tag's 32 bytes, then the blob as it
/// travels. The domain and the tag are fixed length and the blob is last, so the
/// encoding is injective without a length prefix.
pub fn pub_payload(tag: &[u8; 32], blob_b64: &str) -> Vec<u8> {
    let mut m = Vec::with_capacity(PUB_SIG_DOMAIN.len() + 32 + blob_b64.len());
    m.extend_from_slice(PUB_SIG_DOMAIN);
    m.extend_from_slice(tag);
    m.extend_from_slice(blob_b64.as_bytes());
    m
}

/// Why a `Pub` was refused. The relay reports the token and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PubRefusal {
    /// The tag is not 64 lowercase hex characters. Lowercase only, so one key has
    /// exactly one spelling and one set of rows.
    MalformedTag,
    /// The tag's 32 bytes are not an Ed25519 public key, so nothing can sign for it.
    NotAKey,
    /// No signature, or not 128 hex characters.
    MalformedSig,
    /// A well-formed signature that this address's key did not make.
    BadSignature,
}

impl PubRefusal {
    /// The stable token a refusal travels as.
    pub fn as_str(self) -> &'static str {
        match self {
            PubRefusal::MalformedTag => "malformed_tag",
            PubRefusal::NotAKey => "not_a_key",
            PubRefusal::MalformedSig => "malformed_sig",
            PubRefusal::BadSignature => "bad_signature",
        }
    }
}

/// THE check, the one both sides call: may this blob be published to this tag?
///
/// `verify_strict`, so a small-order key or a malleated signature is refused rather
/// than accepted by a looser reading of the same bytes.
pub fn verify_pub(tag_hex: &str, blob_b64: &str, sig_hex: &str) -> Result<(), PubRefusal> {
    let lower = tag_hex.len() == 64
        && tag_hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !lower {
        return Err(PubRefusal::MalformedTag);
    }
    let tag = crate::tag_unhex(tag_hex).map_err(|_| PubRefusal::MalformedTag)?;
    let key = VerifyingKey::from_bytes(&tag).map_err(|_| PubRefusal::NotAKey)?;
    if sig_hex.len() != 128 {
        return Err(PubRefusal::MalformedSig);
    }
    let raw: [u8; 64] = hex::decode(sig_hex)
        .map_err(|_| PubRefusal::MalformedSig)?
        .try_into()
        .map_err(|_| PubRefusal::MalformedSig)?;
    key.verify_strict(&pub_payload(&tag, blob_b64), &Signature::from_bytes(&raw))
        .map_err(|_| PubRefusal::BadSignature)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(n: u8) -> [u8; 32] {
        [n; 32]
    }

    #[test]
    fn a_holder_of_the_seed_can_publish_and_the_relay_can_tell() {
        let a = Address::from_seed(&seed(1));
        let Frame::Pub { tag, blob, sig, .. } = a.pub_frame(crate::blob_b64(b"hello"), false) else {
            panic!("pub_frame builds a Pub");
        };
        assert_eq!(tag, a.tag_hex());
        assert_eq!(verify_pub(&tag, &blob, &sig), Ok(()));
    }

    #[test]
    fn every_holder_of_one_seed_lands_on_one_address() {
        assert_eq!(Address::from_seed(&seed(2)).tag(), Address::from_seed(&seed(2)).tag());
        assert_ne!(Address::from_seed(&seed(2)).tag(), Address::from_seed(&seed(3)).tag());
    }

    #[test]
    fn the_address_is_not_the_seed() {
        // The seed is a secret the writer holds for another reason; the tag is
        // public. If they were equal, publishing would hand the relay the secret.
        let s = seed(4);
        assert_ne!(Address::from_seed(&s).tag(), s);
    }

    /// THE CASE THIS MODULE EXISTS FOR. Mallory knows the tag — it is on the wire —
    /// and does not hold the seed. Every way she can build a frame is refused.
    #[test]
    fn someone_who_only_knows_the_address_cannot_write_to_it() {
        let victim = Address::from_seed(&seed(5));
        let mallory = Address::from_seed(&seed(6));
        let tag = victim.tag_hex();
        let junk = crate::blob_b64(b"junk");

        // Her own key, the victim's tag.
        assert_eq!(verify_pub(&tag, &junk, &mallory.sign_pub(&junk)), Err(PubRefusal::BadSignature));
        // No signature at all — what every client sent before this module.
        assert_eq!(verify_pub(&tag, &junk, ""), Err(PubRefusal::MalformedSig));
        // A genuine signature from the victim, lifted onto different bytes.
        let real = crate::blob_b64(b"the real post");
        let lifted = victim.sign_pub(&real);
        assert_eq!(verify_pub(&tag, &junk, &lifted), Err(PubRefusal::BadSignature));
        // ...and onto a different address.
        assert_eq!(verify_pub(&mallory.tag_hex(), &real, &lifted), Err(PubRefusal::BadSignature));
    }

    #[test]
    fn one_key_has_one_spelling() {
        // An uppercase tag would be the same key filed under a second set of rows.
        let a = Address::from_seed(&seed(7));
        let blob = crate::blob_b64(b"x");
        let sig = a.sign_pub(&blob);
        assert_eq!(verify_pub(&a.tag_hex().to_uppercase(), &blob, &sig), Err(PubRefusal::MalformedTag));
        assert_eq!(verify_pub("aa", &blob, &sig), Err(PubRefusal::MalformedTag), "the old test tags are not keys");
    }

    #[test]
    fn a_tag_that_is_not_a_public_key_can_never_be_written() {
        // Bytes that decompress to no curve point have no signing key, so nobody
        // can publish there — including whoever chose them. Found, not assumed:
        // roughly half of all 32-byte strings are such, and the first is used.
        let raw = (1u8..=255)
            .map(|n| [n; 32])
            .find(|b| VerifyingKey::from_bytes(b).is_err())
            .expect("some constant byte string is not a curve point");
        let blob = crate::blob_b64(b"x");
        assert_eq!(verify_pub(&hex::encode(raw), &blob, &"00".repeat(64)), Err(PubRefusal::NotAKey));
    }

    #[test]
    fn the_blob_is_signed_as_it_travels() {
        // Two base64 spellings of one payload are two different stored blobs, and
        // the relay must never decode to decide which it has.
        let a = Address::from_seed(&seed(8));
        let sig = a.sign_pub("aGk=");
        assert_eq!(verify_pub(&a.tag_hex(), "aGk=", &sig), Ok(()));
        assert_eq!(verify_pub(&a.tag_hex(), "aGk", &sig), Err(PubRefusal::BadSignature));
    }

    /// The rule as DATA, in the fixture both repos are held to. The relay's Rust
    /// calls this module; the Python harness, which drives the real relay over a
    /// socket, has to sign too and implements the rule a second time — so this
    /// vector is what keeps the two the same rule rather than two that agree today.
    #[test]
    fn the_rule_matches_the_vector_in_the_wire_spec() {
        let spec: serde_json::Value = serde_json::from_str(crate::WIRE_SPEC_JSON).unwrap();
        let a = &spec["address"];
        assert_eq!(a["sig_domain"].as_str().unwrap().as_bytes(), PUB_SIG_DOMAIN);
        assert_eq!(a["key_label"].as_str().unwrap().as_bytes(), ADDRESS_KEY_LABEL);
        let vectors = a["vectors"].as_array().expect("address.vectors");
        assert!(!vectors.is_empty());
        for v in vectors {
            let seed: [u8; 32] = hex::decode(v["seed"].as_str().unwrap()).unwrap().try_into().unwrap();
            let addr = Address::from_seed(&seed);
            assert_eq!(addr.tag_hex(), v["tag"].as_str().unwrap());
            assert_eq!(addr.sign_pub(v["blob"].as_str().unwrap()), v["sig"].as_str().unwrap());
            assert_eq!(verify_pub(&addr.tag_hex(), v["blob"].as_str().unwrap(), v["sig"].as_str().unwrap()), Ok(()));
        }
    }

    #[test]
    fn a_signed_pub_round_trips_the_wire() {
        let a = Address::from_seed(&seed(9));
        let f = a.pub_frame(crate::blob_b64(b"x"), false);
        assert_eq!(Frame::from_json(&f.to_json()).unwrap(), f);
    }
}

