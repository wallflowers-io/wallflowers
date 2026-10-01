//! At-rest encryption of on-device secrets (workstream S5 / Inf.1).
//!
//! The crown-jewel secrets — the Ed25519 identity key file, the MLS leaf signing
//! secret and every group's ratchet/epoch state — are otherwise plaintext on disk
//! (only a `0600` file mode + implicit OS file protection). This module seals them
//! under a DEVICE MASTER KEY that lives in the OS secure store (iOS Keychain /
//! Secure Enclave), injected by the host at startup, or in `$PACIFIC_ATREST_KEY`
//! (64 hex chars) for the CLI and tests — the SAME mechanism both places.
//!
//! Contract — **fail loud, never a plaintext fallback**: a file written *sealed*
//! can only be read with the key. If the key is missing it errors
//! ([`CoreError::AtRestKeyUnavailable`]); a wrong key fails the AEAD tag check. When
//! NO key is configured at all the secret is written in the clear (today's
//! behaviour, so the CLI keeps working); provisioning a key transparently upgrades
//! existing plaintext on next load. A *sealed* file never degrades to plaintext.
//!
//! Construction mirrors [`crate::seal`]: XChaCha20Poly1305 keyed by
//! HKDF-SHA256(salt = device_key, info = "pacific/atrest/v1" || context), random
//! 24-byte nonce, AAD = context. Envelope: `MAGIC(4) || nonce(24) || ciphertext`.
//! `context` domain-separates each secret (e.g. `b"pacific/id_ed25519"`), so a blob
//! sealed for one purpose can never be opened as another.

use std::sync::RwLock;

use chacha20poly1305::aead::{Aead, KeyInit, OsRng, Payload};
use chacha20poly1305::{AeadCore, XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use sha2::Sha256;

use crate::CoreError;

/// Magic prefix marking a sealed at-rest envelope (distinguishes it from a legacy
/// plaintext secret). "PxS1" = Pacific at-rest Seal v1.
pub const MAGIC: &[u8; 4] = b"PxS1";
const INFO_PREFIX: &[u8] = b"pacific/atrest/v1";
const NONCE_LEN: usize = 24;

/// Host-injected device master key (from the iOS Keychain / Secure Enclave). Set
/// once at startup via [`set_device_key`]; takes precedence over the env var.
static DEVICE_KEY: RwLock<Option<[u8; 32]>> = RwLock::new(None);

/// Inject the device master key held in the OS secure store. The host (iOS) calls
/// this at startup, before any identity / DB load.
pub fn set_device_key(key: [u8; 32]) {
    *DEVICE_KEY.write().unwrap_or_else(|p| p.into_inner()) = Some(key);
}

/// Clear the injected key (host sign-out / tests).
pub fn clear_device_key() {
    *DEVICE_KEY.write().unwrap_or_else(|p| p.into_inner()) = None;
}

/// Resolve the device master key: injected key first, then `$PACIFIC_ATREST_KEY`
/// (64 hex chars). `Ok(None)` when none is configured; `Err` when one is present
/// but malformed — a bad key is never silently ignored.
pub fn device_key() -> Result<Option<[u8; 32]>, CoreError> {
    if let Some(k) = *DEVICE_KEY.read().unwrap_or_else(|p| p.into_inner()) {
        return Ok(Some(k));
    }
    match std::env::var("PACIFIC_ATREST_KEY") {
        Ok(hexed) => {
            let raw = hex::decode(hexed.trim())
                .map_err(|e| CoreError::AtRest(format!("PACIFIC_ATREST_KEY not hex: {e}")))?;
            let key: [u8; 32] = raw.as_slice().try_into().map_err(|_| {
                CoreError::AtRest(format!(
                    "PACIFIC_ATREST_KEY must be 32 bytes, got {}",
                    raw.len()
                ))
            })?;
            Ok(Some(key))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(e) => Err(CoreError::AtRest(format!(
            "PACIFIC_ATREST_KEY unreadable: {e}"
        ))),
    }
}

fn derive_key(device_key: &[u8; 32], context: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(Some(&device_key[..]), &[]);
    let mut info = Vec::with_capacity(INFO_PREFIX.len() + context.len());
    info.extend_from_slice(INFO_PREFIX);
    info.extend_from_slice(context);
    let mut key = [0u8; 32];
    hk.expand(&info, &mut key)
        .expect("hkdf expand of 32 bytes never fails");
    key
}

/// True if `bytes` is a sealed at-rest envelope (vs a legacy plaintext secret).
pub fn is_sealed(bytes: &[u8]) -> bool {
    bytes.len() >= MAGIC.len() && &bytes[..MAGIC.len()] == &MAGIC[..]
}

/// Seal `plaintext` under `device_key`, domain-separated by `context`. Returns
/// `MAGIC || nonce(24) || ciphertext`.
pub fn seal_at_rest(
    context: &[u8],
    plaintext: &[u8],
    device_key: &[u8; 32],
) -> Result<Vec<u8>, CoreError> {
    let key = derive_key(device_key, context);
    let cipher = XChaCha20Poly1305::new((&key).into());
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ct = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad: context,
            },
        )
        .map_err(|e| CoreError::AtRest(format!("seal encrypt: {e}")))?;
    let mut out = Vec::with_capacity(MAGIC.len() + NONCE_LEN + ct.len());
    out.extend_from_slice(&MAGIC[..]);
    out.extend_from_slice(nonce.as_slice());
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Open a sealed envelope back to plaintext. Loud on wrong key / tamper / bad framing.
pub fn open_at_rest(
    context: &[u8],
    blob: &[u8],
    device_key: &[u8; 32],
) -> Result<Vec<u8>, CoreError> {
    if !is_sealed(blob) {
        return Err(CoreError::AtRest(
            "not a sealed envelope (bad magic)".into(),
        ));
    }
    let rest = &blob[MAGIC.len()..];
    if rest.len() < NONCE_LEN {
        return Err(CoreError::AtRest(format!(
            "sealed blob too short: {} bytes",
            blob.len()
        )));
    }
    let (nonce_bytes, ct) = rest.split_at(NONCE_LEN);
    let key = derive_key(device_key, context);
    let cipher = XChaCha20Poly1305::new((&key).into());
    let nonce = XNonce::from_slice(nonce_bytes);
    cipher
        .decrypt(
            nonce,
            Payload {
                msg: ct,
                aad: context,
            },
        )
        .map_err(|_| CoreError::AtRest("open failed (wrong key or tampered)".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_roundtrip_and_hides_plaintext() {
        let key = [0x11u8; 32];
        let ctx = b"pacific/id_ed25519";
        let secret = b"64-bytes-of-very-secret-signing-key-material............";
        let blob = seal_at_rest(ctx, secret, &key).unwrap();
        assert!(is_sealed(&blob));
        assert!(!blob.windows(secret.len()).any(|w| w == &secret[..]));
        assert_eq!(open_at_rest(ctx, &blob, &key).unwrap(), secret);
    }

    #[test]
    fn wrong_key_and_wrong_context_fail_loud() {
        let ctx = b"pacific/id_ed25519";
        let blob = seal_at_rest(ctx, b"hi", &[0x22u8; 32]).unwrap();
        assert!(open_at_rest(ctx, &blob, &[0x33u8; 32]).is_err()); // wrong key
        assert!(open_at_rest(b"pacific/other", &blob, &[0x22u8; 32]).is_err()); // wrong ctx (AAD)
        let mut tampered = blob.clone();
        *tampered.last_mut().unwrap() ^= 0xff;
        assert!(open_at_rest(ctx, &tampered, &[0x22u8; 32]).is_err()); // tamper
    }

    #[test]
    fn is_sealed_rejects_legacy_plaintext() {
        assert!(!is_sealed(&[0u8; 64])); // a 64-byte plaintext key file
        assert!(!is_sealed(b"Px")); // too short
        assert!(is_sealed(b"PxS1\x00\x00"));
    }

    #[test]
    fn env_key_parses_and_rejects_malformed() {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        clear_device_key();
        std::env::set_var("PACIFIC_ATREST_KEY", "aa".repeat(32));
        assert!(matches!(device_key(), Ok(Some(_))));
        std::env::set_var("PACIFIC_ATREST_KEY", "not-hex");
        assert!(matches!(device_key(), Err(CoreError::AtRest(_))));
        std::env::set_var("PACIFIC_ATREST_KEY", "aabb"); // valid hex, wrong length
        assert!(matches!(device_key(), Err(CoreError::AtRest(_))));
        std::env::remove_var("PACIFIC_ATREST_KEY");
        assert!(matches!(device_key(), Ok(None)));
    }

    #[test]
    fn injected_key_takes_precedence() {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("PACIFIC_ATREST_KEY");
        set_device_key([0x7au8; 32]);
        assert_eq!(device_key().unwrap(), Some([0x7au8; 32]));
        clear_device_key();
        assert_eq!(device_key().unwrap(), None);
    }
}
