//! The pairing handshake. The contact bundle and its checks are `trust::qr`'s; this
//! module signs with the device's identity, and carries the intro payload a Welcome
//! rides in.

use serde::{Deserialize, Serialize};
use trust::cbor::{bytes32 as serde_bytes_array32, bytes_vec as serde_bytes_vec};

use crate::identity::Identity;
use crate::CoreError;

pub use trust::qr::{ContactBundle, ERR_KEY_PACKAGE_IDENTITY, PROTOCOL_VERSION};

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
    Ok(trust::qr::build(
        id.identity_pk(),
        id.next_key_commit(),
        key_package,
        intro_tag,
        display_name,
        |m| id.sign(m),
    )?)
}

/// Parse a pasted base64 bundle AND verify its self-signature. Loud on tamper.
pub fn parse_and_verify(s: &str) -> Result<ContactBundle, CoreError> {
    Ok(trust::qr::parse_and_verify(s)?)
}

/// WHO a key package will become once added: see [`trust::qr::key_package_cred_id`].
#[cfg(feature = "mls")]
pub fn key_package_cred_id(key_package: &[u8]) -> Result<[u8; 32], CoreError> {
    Ok(trust::qr::key_package_cred_id(key_package)?)
}

/// REFUSE a key package that speaks for someone other than `identity_pk`: see
/// [`trust::qr::verify_key_package_identity`], which says why.
#[cfg(feature = "mls")]
pub fn verify_key_package_identity(identity_pk: &[u8; 32], key_package: &[u8]) -> Result<(), CoreError> {
    Ok(trust::qr::verify_key_package_identity(identity_pk, key_package)?)
}

// --- serde helpers ---------------------------------------------------------



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


#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity;

    fn with_state_dir<F: FnOnce()>(f: F) {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("PACIFIC_STATE_DIR", dir.path());
        f();
        std::env::remove_var("PACIFIC_STATE_DIR");
    }

    /// THE BUNDLE'S BYTES, pinned. Ed25519 signs deterministically, so a fixed seed and
    /// fixed fields give one encoding; a change to it is a change to the QR every
    /// installed build reads.
    const GOLDEN: &str = "\
        p2F2AWtpZGVudGl0eV9wa1ggk0FNc+VoSAhJZzqxKeLI19ziTuuQ3wH7tfIRyExhKzVra2V5X3BhY2th\
        Z2VEAQIDBGlpbnRyb190YWdYIAkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJbGRpc3BsYXlf\
        bmFtZWNhZGFvbmV4dF9rZXlfY29tbWl0WCB2PnsQl0+UYsXv3gw/mY25PAPy1ldGWDUvVbmZ0rcxtmNz\
        aWdYQFaOWD2StkRhG6Ulau6mLHiHag7hlmXHF1FLgG9IcbZ6MxSsNSmw0d80RynyKI2+MjHkYSbbLoG4\
        FigzuSFwDAg=";

    #[test]
    fn bundle_bytes_are_pinned() {
        let id = Identity::in_memory([7u8; 32]);
        let b = build_bundle(&id, vec![1, 2, 3, 4], [9u8; 32], "ada").unwrap();
        assert_eq!(b.encode().unwrap(), GOLDEN);
        let parsed = parse_and_verify(GOLDEN).unwrap();
        assert_eq!(parsed.identity_pk, id.identity_pk());
        assert_eq!(parsed.next_key_commit, id.next_key_commit());
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
