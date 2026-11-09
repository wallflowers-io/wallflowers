//! The one strict Ed25519 verify.

use ed25519_dalek::{Signature, VerifyingKey};

use crate::Error;

/// Verify `sig` over `msg` under `pk`. Strict: a small-order key or R is refused. The
/// identity point with R = identity, s = 0 otherwise verifies over every message (NC-46).
pub fn verify_strict(pk: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> Result<(), Error> {
    let vk = VerifyingKey::from_bytes(pk).map_err(|e| Error::Signature(format!("bad pubkey: {e}")))?;
    vk.verify_strict(msg, &Signature::from_bytes(sig))
        .map_err(|_| Error::Signature("signature verify failed".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn a_good_signature_verifies_and_a_stale_one_does_not() {
        let sk = SigningKey::from_bytes(&[3u8; 32]);
        let pk = sk.verifying_key().to_bytes();
        let sig = sk.sign(b"ada").to_bytes();
        verify_strict(&pk, b"ada", &sig).unwrap();
        assert!(verify_strict(&pk, b"bob", &sig).is_err());
    }

    #[test]
    fn a_non_canonical_s_is_refused() {
        // s + ℓ verifies the same equation; strict (RFC 8032 §5.1.7) refuses s ≥ ℓ.
        const L: [u8; 32] = [
            0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10,
        ];
        let sk = SigningKey::from_bytes(&[3u8; 32]);
        let pk = sk.verifying_key().to_bytes();
        let mut sig = sk.sign(b"ada").to_bytes();
        let mut carry = 0u16;
        for i in 0..32 {
            let v = sig[32 + i] as u16 + L[i] as u16 + carry;
            sig[32 + i] = v as u8;
            carry = v >> 8;
        }
        assert_eq!(carry, 0);
        assert!(verify_strict(&pk, b"ada", &sig).is_err());
    }

    #[test]
    fn the_identity_point_is_refused() {
        // A = identity, R = identity, s = 0: verifies over every message unless strict.
        let mut pk = [0u8; 32];
        pk[0] = 1;
        let mut sig = [0u8; 64];
        sig[0] = 1;
        assert!(verify_strict(&pk, b"anything", &sig).is_err());
    }
}
