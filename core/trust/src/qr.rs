//! The in-person QR: a self-signed contact bundle.
//!
//! It carries the identity pubkey, a fresh one-time MLS KeyPackage, the intro mailbox
//! tag, the display name, and the KERI next-key commitment, all signed by the Ed25519
//! stable key. Canonical CBOR, base64-wrapped. The same string is the QR, the pasted
//! connect code, the Door's `/v2/bundle` and a Site member's offer.
//!
//! WHAT IT GUARANTEES:
//! - [`parse_and_verify`]: the holder of `identity_pk` signed these bytes.
//! - [`verify_key_package_identity`]: the key package speaks for that same signer.
//! - Scanned in person, the camera is the channel, so the scanner holds the
//!   presenter's real key.
//!
//! WHAT IT DOES NOT:
//! - Tell the presenter who scanned. The scanner's key reaches them over the network,
//!   sealed to `intro_tag`, which any holder of the bundle can seal to.
//! - Know which channel delivered it. A pasted bundle earns no more than first use.
//! - Give the two people anything to compare.

use serde::{Deserialize, Serialize};

use crate::{cbor, sig, Error};

pub const PROTOCOL_VERSION: u32 = 1;

/// A pasteable, self-signed pairing bundle.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContactBundle {
    pub v: u32,
    #[serde(with = "cbor::bytes32")]
    pub identity_pk: [u8; 32],
    #[serde(with = "cbor::bytes_vec")]
    pub key_package: Vec<u8>,
    #[serde(with = "cbor::bytes32")]
    pub intro_tag: [u8; 32],
    pub display_name: String,
    #[serde(with = "cbor::bytes32")]
    pub next_key_commit: [u8; 32],
    #[serde(with = "cbor::bytes64")]
    pub sig: [u8; 64],
}

impl ContactBundle {
    /// The canonical bytes that the signature covers (sig field zeroed).
    fn signing_bytes(&self) -> Result<Vec<u8>, Error> {
        let unsigned = ContactBundle {
            sig: [0u8; 64],
            ..self.clone()
        };
        let mut buf = Vec::new();
        ciborium::ser::into_writer(&unsigned, &mut buf)
            .map_err(|e| Error::Bundle(format!("bundle encode: {e}")))?;
        Ok(buf)
    }

    /// Base64 of the full (signed) CBOR bundle — the paste string.
    pub fn encode(&self) -> Result<String, Error> {
        use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
        let mut buf = Vec::new();
        ciborium::ser::into_writer(self, &mut buf)
            .map_err(|e| Error::Bundle(format!("bundle encode: {e}")))?;
        Ok(B64.encode(buf))
    }
}

/// Build and sign a bundle. `sign` is the identity key's signature over the bytes it is
/// given; the secret key never enters this crate.
pub fn build(
    identity_pk: [u8; 32],
    next_key_commit: [u8; 32],
    key_package: Vec<u8>,
    intro_tag: [u8; 32],
    display_name: &str,
    sign: impl FnOnce(&[u8]) -> [u8; 64],
) -> Result<ContactBundle, Error> {
    let mut b = ContactBundle {
        v: PROTOCOL_VERSION,
        identity_pk,
        key_package,
        intro_tag,
        display_name: display_name.to_string(),
        next_key_commit,
        sig: [0u8; 64],
    };
    b.sig = sign(&b.signing_bytes()?);
    Ok(b)
}

/// Parse a pasted base64 bundle AND verify its self-signature. Loud on tamper.
pub fn parse_and_verify(s: &str) -> Result<ContactBundle, Error> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    let raw = B64
        .decode(s.trim())
        .map_err(|e| Error::Bundle(format!("bundle base64: {e}")))?;
    let b: ContactBundle = ciborium::de::from_reader(&raw[..])
        .map_err(|e| Error::Bundle(format!("bundle decode: {e}")))?;
    if b.v != PROTOCOL_VERSION {
        return Err(Error::Bundle(format!("unsupported bundle version {}", b.v)));
    }
    sig::verify_strict(&b.identity_pk, &b.signing_bytes()?, &b.sig)?;
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
// value pacific-core's `mls::roster_identities` and `mls::member_identity` read,
// which is the value the commutative fold keys AUTHORSHIP and AUTHORITY on. The
// attacker does not merely get in; they get in AS SOMEONE ELSE, with that person's
// authority, and every delta they author is attributed to the victim for ever.
//
// BEFORE A1a THIS WAS COVERED BY ACCIDENT. `BasicIdentityProvider::identity()`
// returns the credential verbatim, so mls-rs's tree-index uniqueness map refused
// any leaf whose credential was already in the tree (`MlsError::DuplicateLeafData`)
// — which happened to refuse the forgery whenever the victim was already a member.
// pacific-core's `mls::PerLeafIdentity` returns credential ‖ signature_key PRECISELY
// so that two leaves may share one credential (one person, two devices), and the
// accidental refusal went with it — and it never covered the case where the victim
// was not yet in the group at all. So the check has to be stated. Here it is stated,
// and pacific-core's `Node::add_member_core` and `Node::pair_scan` enforce it.

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
pub fn key_package_cred_id(key_package: &[u8]) -> Result<[u8; 32], Error> {
    use mls_rs::MlsMessage;
    let bad = |what: &str| Error::Bundle(format!("{ERR_KEY_PACKAGE_IDENTITY}: {what}"));
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
pub fn verify_key_package_identity(identity_pk: &[u8; 32], key_package: &[u8]) -> Result<(), Error> {
    let cred = key_package_cred_id(key_package)?;
    if &cred != identity_pk {
        return Err(Error::Bundle(format!(
            "{ERR_KEY_PACKAGE_IDENTITY}: bundle is signed by {} but its key package \
             speaks for {} — refusing to graft a leaf that would author as somebody \
             other than the party that signed for it",
            hex::encode(identity_pk),
            hex::encode(cred),
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn bundle(sk: &SigningKey, name: &str) -> ContactBundle {
        let pk = sk.verifying_key().to_bytes();
        build(pk, [8u8; 32], vec![1, 2, 3, 4], [9u8; 32], name, |m| sk.sign(m).to_bytes()).unwrap()
    }

    /// The bytes, pinned: a change here is a change to the QR every installed build reads.
    const GOLDEN: &str = "\
        p2F2AWtpZGVudGl0eV9wa1gg6kpsY+KcUgq+9VB7Ey7F+ZVHdq6+vnuSQh7qaRRG0ixra2V5X3BhY2th\
        Z2VEAQIDBGlpbnRyb190YWdYIAkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJbGRpc3BsYXlf\
        bmFtZWNhZGFvbmV4dF9rZXlfY29tbWl0WCAICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICGNz\
        aWdYQG69vJ72YdyWpQVk2YRQ2CGQV/5iutO2RScmVj06tVg40NDfjcAj0JGkopF0j7jgKLfwZwIRH/1M\
        +Y1m5grXfgY=";

    #[test]
    fn bundle_bytes_are_pinned() {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        assert_eq!(bundle(&sk, "ada").encode().unwrap(), GOLDEN);
        assert_eq!(parse_and_verify(GOLDEN).unwrap().identity_pk, sk.verifying_key().to_bytes());
    }

    #[test]
    fn bundle_roundtrip_and_tamper_detection() {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let b = bundle(&sk, "ada");
        let parsed = parse_and_verify(&b.encode().unwrap()).unwrap();
        assert_eq!(parsed.identity_pk, sk.verifying_key().to_bytes());
        assert_eq!(parsed.key_package, vec![1, 2, 3, 4]);
        assert_eq!(parsed.intro_tag, [9u8; 32]);

        // Changes the signing bytes; the signature is stale.
        let mut bad = b;
        bad.display_name = "mallory".into();
        assert!(matches!(parse_and_verify(&bad.encode().unwrap()), Err(Error::Signature(_))));
    }

    /// A real key package whose leaf carries `cred_id` as its BasicCredential.
    #[cfg(feature = "mls")]
    fn key_package_for(cred_id: [u8; 32]) -> Vec<u8> {
        use mls_rs::identity::basic::{BasicCredential, BasicIdentityProvider};
        use mls_rs::identity::SigningIdentity;
        use mls_rs::{CipherSuite, CipherSuiteProvider, Client, CryptoProvider, ExtensionList};
        use mls_rs_crypto_rustcrypto::RustCryptoProvider;
        let suite = CipherSuite::CURVE25519_AES128;
        let crypto = RustCryptoProvider::new();
        let (sk, pk) = crypto.cipher_suite_provider(suite).unwrap().signature_key_generate().unwrap();
        let id = SigningIdentity::new(BasicCredential::new(cred_id.to_vec()).into_credential(), pk);
        let client = Client::builder()
            .crypto_provider(crypto)
            .identity_provider(BasicIdentityProvider)
            .signing_identity(id, sk, suite)
            .build();
        client
            .generate_key_package_message(ExtensionList::default(), ExtensionList::default(), None)
            .unwrap()
            .to_bytes()
            .unwrap()
    }

    #[cfg(feature = "mls")]
    #[test]
    fn a_key_package_binds_to_its_signer_and_to_no_one_else() {
        let ada = [5u8; 32];
        verify_key_package_identity(&ada, &key_package_for(ada)).unwrap();

        let e = verify_key_package_identity(&ada, &key_package_for([6u8; 32])).unwrap_err();
        assert!(matches!(&e, Error::Bundle(m) if m.starts_with(ERR_KEY_PACKAGE_IDENTITY) && m.contains("speaks for")), "{e}");

        let e = verify_key_package_identity(&ada, &[1, 2, 3, 4]).unwrap_err();
        assert!(matches!(&e, Error::Bundle(m) if m.starts_with(ERR_KEY_PACKAGE_IDENTITY)), "{e}");
    }

    #[test]
    fn another_version_is_refused() {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let mut b = bundle(&sk, "ada");
        b.v = PROTOCOL_VERSION + 1;
        b.sig = sk.sign(&b.signing_bytes().unwrap()).to_bytes();
        let e = parse_and_verify(&b.encode().unwrap()).unwrap_err();
        assert_eq!(e.to_string(), format!("unsupported bundle version {}", PROTOCOL_VERSION + 1));
    }
}
