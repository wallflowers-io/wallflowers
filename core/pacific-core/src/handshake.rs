//! The pairing Contact Bundle — defined inside pacific-core (the on-disk pacific-wire
//! has no bundle type). It carries the identity pubkey, a fresh one-time MLS
//! KeyPackage, the intro mailbox tag, the display name, and the KERI next-key
//! commitment, all signed by the Ed25519 stable key. Encoded as canonical CBOR,
//! base64-wrapped for paste.

use serde::{Deserialize, Serialize};

use crate::identity::{self, Identity};
use crate::CoreError;

pub const PROTOCOL_VERSION: u32 = 1;

/// A pasteable, self-signed pairing bundle.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContactBundle {
    pub v: u32,
    #[serde(with = "serde_bytes_array32")]
    pub identity_pk: [u8; 32],
    #[serde(with = "serde_bytes_vec")]
    pub key_package: Vec<u8>,
    #[serde(with = "serde_bytes_array32")]
    pub intro_tag: [u8; 32],
    pub display_name: String,
    #[serde(with = "serde_bytes_array32")]
    pub next_key_commit: [u8; 32],
    #[serde(with = "serde_bytes_array64")]
    pub sig: [u8; 64],
}

impl ContactBundle {
    /// The canonical bytes that the signature covers (sig field zeroed).
    fn signing_bytes(&self) -> Result<Vec<u8>, CoreError> {
        let unsigned = ContactBundle {
            sig: [0u8; 64],
            ..self.clone()
        };
        let mut buf = Vec::new();
        ciborium::ser::into_writer(&unsigned, &mut buf)
            .map_err(|e| CoreError::Handshake(format!("bundle encode: {e}")))?;
        Ok(buf)
    }

    /// Base64 of the full (signed) CBOR bundle — the paste string.
    pub fn encode(&self) -> Result<String, CoreError> {
        use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
        let mut buf = Vec::new();
        ciborium::ser::into_writer(self, &mut buf)
            .map_err(|e| CoreError::Handshake(format!("bundle encode: {e}")))?;
        Ok(B64.encode(buf))
    }
}

/// The bootstrap payload sealed to a peer's intro mailbox: the scanner's identity
/// pubkey (so the scannee learns who paired) + the MLS Welcome bytes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntroPayload {
    #[serde(with = "serde_bytes_array32")]
    pub scanner_pk: [u8; 32],
    pub scanner_name: String,
    #[serde(with = "serde_bytes_vec")]
    pub welcome: Vec<u8>,
    /// The "why" of the connection (Customer/Investor/…), captured at scan time —
    /// the differentiator. Optional + default so older payloads still decode and the
    /// wire stays byte-compatible with the n+1 web client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// The object kind of the group this Welcome joins ("connection", "forum", …).
    /// Optional + default so the pair flow and the n+1 web client stay byte-
    /// compatible; a missing/None kind means "connection".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// The Arc that governs the joined object (moderation/reports/safety + the
    /// Semaphore relay its traffic routes through). Set by the sharer so the joiner
    /// routes to the SAME Arc the creator chose, rather than falling back to its own
    /// default. Optional + default so older payloads/the n+1 web client stay
    /// byte-compatible; a missing/None arc means "use the device default".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arc: Option<String>,
    /// The joined OBJECT's owner (owner-as-sequencer) — NOT the sender. Any member
    /// may perform an add at a public door, so "who sealed this Welcome" and "whose
    /// owner-sequenced deltas the fold must honour" are different people; recording
    /// the sender as owner made every second-generation admission fold nothing
    /// (m14). Optional + default so older payloads still decode; a missing/None
    /// owner keeps the historical meaning (sender = owner, true for the pair flow
    /// and for owner-performed adds).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_opt_bytes_array32")]
    pub owner: Option<[u8; 32]>,
    /// THE GEN FLOOR the joiner must start above — the adder's own high-water mark
    /// for this group at the moment of the add.
    ///
    /// A `MsgRef` is (author_pk, gen) and `author_pk` is the PERSON, so two devices
    /// of one person agree on the author half and used to derive the counter half
    /// from their own logs. A joining leaf's log is necessarily EMPTY — forward
    /// secrecy, and `add_member_core` refuses to re-encrypt history so it stays
    /// that way — so it restarted at 0 and re-minted refs its person already used.
    /// The fold keys commutative deltas by ref, so the later one REPLACED the
    /// earlier on every device holding both, and `delta_id` (a hash of the text)
    /// decided which survived. Measured 15 Sep 2026.
    ///
    /// The adder holds the log and therefore knows the mark; it rides here because
    /// this is the one message that reaches a joiner before it can author anything.
    /// Optional + default so older payloads still decode and the n+1 web client
    /// stays byte-compatible; a missing/None mark means "no floor", which is the
    /// historical behaviour and correct for a device that has the history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gen_watermark: Option<u64>,
}

impl IntroPayload {
    pub fn encode(&self) -> Result<Vec<u8>, CoreError> {
        let mut buf = Vec::new();
        ciborium::ser::into_writer(self, &mut buf)
            .map_err(|e| CoreError::Handshake(format!("intro encode: {e}")))?;
        Ok(buf)
    }
    pub fn decode(b: &[u8]) -> Result<Self, CoreError> {
        ciborium::de::from_reader(b).map_err(|e| CoreError::Handshake(format!("intro decode: {e}")))
    }
}

/// Build OUR signed ContactBundle.
pub fn build_bundle(
    id: &Identity,
    key_package: Vec<u8>,
    intro_tag: [u8; 32],
    display_name: &str,
) -> Result<ContactBundle, CoreError> {
    let mut b = ContactBundle {
        v: PROTOCOL_VERSION,
        identity_pk: id.identity_pk(),
        key_package,
        intro_tag,
        display_name: display_name.to_string(),
        next_key_commit: id.next_key_commit(),
        sig: [0u8; 64],
    };
    b.sig = id.sign(&b.signing_bytes()?);
    Ok(b)
}

/// Parse a pasted base64 bundle AND verify its self-signature. Loud on tamper.
pub fn parse_and_verify(s: &str) -> Result<ContactBundle, CoreError> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    let raw = B64
        .decode(s.trim())
        .map_err(|e| CoreError::Handshake(format!("bundle base64: {e}")))?;
    let b: ContactBundle = ciborium::de::from_reader(&raw[..])
        .map_err(|e| CoreError::Handshake(format!("bundle decode: {e}")))?;
    if b.v != PROTOCOL_VERSION {
        return Err(CoreError::Handshake(format!(
            "unsupported bundle version {}",
            b.v
        )));
    }
    identity::verify_sig(&b.identity_pk, &b.signing_bytes()?, &b.sig)?;
    Ok(b)
}

// ── THE KEY-PACKAGE BINDING CHECK ────────────────────────────────────────────
//
// WHAT [`parse_and_verify`] DOES NOT ATTEST, stated here because the omission is
// invisible at the call site. Its signature check proves exactly one thing: the
// holder of `identity_pk` signed these bundle bytes. It proves NOTHING about what
// is inside `key_package` — the two are independent fields, and anyone holding
// their OWN identity key can sign a perfectly well-formed bundle wrapped around
// SOMEBODY ELSE'S credential. The leaf that add then grafts into the ratchet tree
// carries the victim's identity pubkey as its BasicCredential id, and that is the
// value `mls::roster_identities` and `mls::member_identity` read, which is the
// value the commutative fold keys AUTHORSHIP and AUTHORITY on. The attacker does
// not merely get in; they get in AS SOMEONE ELSE, with that person's authority,
// and every delta they author is attributed to the victim for ever.
//
// BEFORE A1a THIS WAS COVERED BY ACCIDENT. `BasicIdentityProvider::identity()`
// returns the credential verbatim, so mls-rs's tree-index uniqueness map refused
// any leaf whose credential was already in the tree (`MlsError::DuplicateLeafData`)
// — which happened to refuse the forgery whenever the victim was already a member.
// `mls::PerLeafIdentity` returns credential ‖ signature_key PRECISELY so that two
// leaves may share one credential (one person, two devices), and the accidental
// refusal went with it — and it never covered the case where the victim was not
// yet in the group at all. So the check has to be stated. Here it is stated, and
// [`crate::node::Node::add_member_core`] is the one place it is enforced.

/// The refusal a mis-bound key package earns. A named constant, not prose at the
/// throw site, so callers and tests can recognise this specific refusal without
/// matching on a sentence that may be reworded.
pub const ERR_KEY_PACKAGE_IDENTITY: &str = "key package identity mismatch";

/// The 32-byte BasicCredential identifier carried by the leaf inside an encoded
/// KeyPackage — i.e. WHO this key package will become once it is added to a group.
///
/// Read WITHOUT joining anything: an `MlsMessage` carrying a KeyPackage exposes
/// its `LeafNode`'s `SigningIdentity` directly, so the identity a bundle is
/// offering can be inspected before a single byte of group state moves.
#[cfg(feature = "mls")]
pub fn key_package_cred_id(key_package: &[u8]) -> Result<[u8; 32], CoreError> {
    use mls_rs::MlsMessage;
    let bad = |what: &str| CoreError::Handshake(format!("{ERR_KEY_PACKAGE_IDENTITY}: {what}"));
    let msg = MlsMessage::from_bytes(key_package)
        .map_err(|e| bad(&format!("key package does not decode ({e})")))?;
    let kp = msg
        .as_key_package()
        .ok_or_else(|| bad("bundle payload is not a KeyPackage"))?;
    let basic = kp
        .signing_identity()
        .credential
        .as_basic()
        .ok_or_else(|| bad("key package credential is not a BasicCredential"))?;
    basic
        .identifier
        .as_slice()
        .try_into()
        .map_err(|_| bad("key package cred_id is not 32 bytes"))
}

/// REFUSE unless the leaf inside `key_package` carries `identity_pk` as its
/// BasicCredential identifier — the binding [`parse_and_verify`] leaves open.
///
/// Deliberately an EQUALITY on the credential and nothing else. It must NOT look
/// at the leaf's signature key: one person's second device brings a fresh
/// signature key and the SAME credential, which is exactly what A1a exists to
/// permit. Credential equality admits every honest leaf — a first device, an Nth
/// device, a stocked prekey — and refuses only the one thing no honest client ever
/// produces: a key package speaking for somebody other than the bundle's signer.
#[cfg(feature = "mls")]
pub fn verify_key_package_identity(
    identity_pk: &[u8; 32],
    key_package: &[u8],
) -> Result<(), CoreError> {
    let cred = key_package_cred_id(key_package)?;
    if &cred != identity_pk {
        return Err(CoreError::Handshake(format!(
            "{ERR_KEY_PACKAGE_IDENTITY}: bundle is signed by {} but its key package \
             speaks for {} — refusing to graft a leaf that would author as somebody \
             other than the party that signed for it",
            hex::encode(identity_pk),
            hex::encode(cred),
        )));
    }
    Ok(())
}

// --- serde helpers for fixed-width byte arrays as CBOR byte strings ----------

mod serde_bytes_vec {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(v)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let b = serde_bytes::ByteBuf::deserialize(d)?;
        Ok(b.into_vec())
    }
}

mod serde_bytes_array32 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(v)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let b = serde_bytes::ByteBuf::deserialize(d)?;
        b.into_vec()
            .try_into()
            .map_err(|_| serde::de::Error::custom("expected 32 bytes"))
    }
}

/// [`serde_bytes_array32`] for an OPTIONAL field: the value, when present, is the
/// same CBOR byte string; absence is handled by the field's `#[serde(default)]`.
mod serde_opt_bytes_array32 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &Option<[u8; 32]>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(a) => s.serialize_bytes(a),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<[u8; 32]>, D::Error> {
        let b = Option::<serde_bytes::ByteBuf>::deserialize(d)?;
        match b {
            None => Ok(None),
            Some(b) => b
                .into_vec()
                .try_into()
                .map(Some)
                .map_err(|_| serde::de::Error::custom("expected 32 bytes")),
        }
    }
}

mod serde_bytes_array64 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8; 64], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(v)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 64], D::Error> {
        let b = serde_bytes::ByteBuf::deserialize(d)?;
        b.into_vec()
            .try_into()
            .map_err(|_| serde::de::Error::custom("expected 64 bytes"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_state_dir<F: FnOnce()>(f: F) {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("PACIFIC_STATE_DIR", dir.path());
        f();
        std::env::remove_var("PACIFIC_STATE_DIR");
    }

    #[test]
    fn bundle_roundtrip_and_tamper_detection() {
        with_state_dir(|| {
            let id = identity::init().unwrap();
            let kp = vec![1u8, 2, 3, 4];
            let intro = [7u8; 32];
            let bundle = build_bundle(&id, kp.clone(), intro, "alice").unwrap();
            let encoded = bundle.encode().unwrap();

            let parsed = parse_and_verify(&encoded).unwrap();
            assert_eq!(parsed.identity_pk, id.identity_pk());
            assert_eq!(parsed.key_package, kp);
            assert_eq!(parsed.intro_tag, intro);

            // tamper: flip a byte in the base64 payload -> verify must fail
            let mut bad = bundle;
            bad.display_name = "mallory".into(); // changes signing bytes, sig stale
            let bad_encoded = bad.encode().unwrap();
            assert!(parse_and_verify(&bad_encoded).is_err());
        });
    }
}
