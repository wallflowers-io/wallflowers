//! The minimal M1 metadata seal.
//!
//! MLS already provides confidentiality + sender authentication. The seal's only
//! M1 job is to HIDE the MLS header (the cleartext `group_id`/`epoch` a raw
//! `MlsMessage` would otherwise leak on the relay) and bind the destination tag.
//!
//! Construction: XChaCha20Poly1305 keyed by HKDF-SHA256(salt = `conn_secret`
//! (the MLS exporter), info = "pacific/seal/v1" || dest_tag), random 24-byte
//! nonce, AAD = dest_tag. Wire layout: `nonce(24) || ciphertext`.
//!
//! No X25519, no per-message ephemeral DH — that gift-wrap/unlinkability layer is
//! M2. A wrong `conn_secret` fails the AEAD tag check loudly (never a fallback).

use chacha20poly1305::aead::{Aead, KeyInit, OsRng, Payload};
use chacha20poly1305::{AeadCore, XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use sha2::Sha256;

use crate::CoreError;

/// The MLS exporter label for the seal key (distinct from the relay-tag labels).
pub const SEAL_LABEL: &[u8] = b"pacific/seal/v1";
const SEAL_INFO_PREFIX: &[u8] = b"pacific/seal/v1";
const NONCE_LEN: usize = 24;

/// Derive the 32-byte XChaCha key from the shared conn_secret + dest_tag.
fn derive_key(conn_secret: &[u8; 32], dest_tag: &[u8; 32]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(Some(&conn_secret[..]), &[]);
    let mut info = Vec::with_capacity(SEAL_INFO_PREFIX.len() + 32);
    info.extend_from_slice(SEAL_INFO_PREFIX);
    info.extend_from_slice(&dest_tag[..]);
    let mut key = [0u8; 32];
    hk.expand(&info, &mut key)
        .expect("hkdf expand of 32 bytes never fails");
    key
}

/// Seal inner MLS bytes for `dest_tag`. Returns `nonce(24) || ct`.
pub fn seal(
    inner_mls: &[u8],
    dest_tag: &[u8; 32],
    conn_secret: &[u8; 32],
) -> Result<Vec<u8>, CoreError> {
    let key = derive_key(conn_secret, dest_tag);
    let cipher = XChaCha20Poly1305::new((&key).into());
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ct = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: inner_mls,
                aad: &dest_tag[..],
            },
        )
        .map_err(|e| CoreError::Seal(format!("seal encrypt: {e}")))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(nonce.as_slice());
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Open a sealed blob -> inner MLS bytes. Loud on a wrong `conn_secret`/tamper.
pub fn open(
    blob: &[u8],
    dest_tag: &[u8; 32],
    conn_secret: &[u8; 32],
) -> Result<Vec<u8>, CoreError> {
    if blob.len() < NONCE_LEN {
        return Err(CoreError::Seal(format!(
            "sealed blob too short: {} bytes",
            blob.len()
        )));
    }
    let (nonce_bytes, ct) = blob.split_at(NONCE_LEN);
    let key = derive_key(conn_secret, dest_tag);
    let cipher = XChaCha20Poly1305::new((&key).into());
    let nonce = XNonce::from_slice(nonce_bytes);
    cipher
        .decrypt(
            nonce,
            Payload {
                msg: ct,
                aad: &dest_tag[..],
            },
        )
        .map_err(|_| CoreError::Seal("seal open failed (wrong key or tampered)".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_roundtrip() {
        let dest = [0xABu8; 32];
        let secret = [0x11u8; 32];
        let inner = b"the inner MLS application message bytes";
        let blob = seal(inner, &dest, &secret).unwrap();
        // ciphertext does not contain the plaintext
        assert!(!blob.windows(inner.len()).any(|w| w == &inner[..]));
        let got = open(&blob, &dest, &secret).unwrap();
        assert_eq!(got, inner);
    }

    #[test]
    fn wrong_secret_fails_loudly() {
        let dest = [0x01u8; 32];
        let blob = seal(b"hi", &dest, &[0x22u8; 32]).unwrap();
        assert!(open(&blob, &dest, &[0x33u8; 32]).is_err());
        // wrong dest_tag (AAD) also fails
        assert!(open(&blob, &[0x99u8; 32], &[0x22u8; 32]).is_err());
    }
}
