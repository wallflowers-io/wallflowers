//! seal — the PRF output, opened by the one session process it was sealed to
//! (SEC-37; mdr/door.md §4 step 2). The construction is `app/web/door/seal.js`'s
//! and is pinned in ICD-4: X25519 to this process's key for this attempt,
//! HKDF-SHA256 (salt `wallflowers/door/prf/v1`, info ephemeral ‖ recipient),
//! AES-256-GCM with the attempt id as AAD. The supervisor and the edge carry the
//! sealed bytes and hold no key that opens them.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

const SALT: &[u8] = b"wallflowers/door/prf/v1";

/// What the window sends: base64url without padding, as `seal.js` writes it.
#[derive(Deserialize, Serialize, Clone)]
pub struct Sealed {
    pub epk: String,
    pub iv: String,
    pub ct: String,
}

/// This process's key for one attempt. The secret half never leaves it, and is
/// zeroed when the key is dropped.
pub struct SessionKey {
    secret: StaticSecret,
    public: PublicKey,
}

impl SessionKey {
    pub fn new() -> Self {
        let secret = StaticSecret::random_from_rng(rand_core::OsRng);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    pub fn public(&self) -> String {
        B64.encode(self.public.as_bytes())
    }

    /// The sealed bytes, or a refusal that says which part failed. A wrong key,
    /// a wrong attempt or a changed byte all fail at the tag.
    pub fn open(&self, sealed: &Sealed, attempt: &str) -> Result<Zeroizing<Vec<u8>>, String> {
        let epk: [u8; 32] = B64
            .decode(&sealed.epk)
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or("the sealed PRF's ephemeral key is not 32 bytes of base64url")?;
        let iv: [u8; 12] = B64
            .decode(&sealed.iv)
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or("the sealed PRF's iv is not 12 bytes of base64url")?;
        let ct = B64.decode(&sealed.ct).map_err(|_| "the sealed PRF is not base64url")?;
        let shared = Zeroizing::new(self.secret.diffie_hellman(&PublicKey::from(epk)).to_bytes());
        let mut info = [0u8; 64];
        info[..32].copy_from_slice(&epk);
        info[32..].copy_from_slice(self.public.as_bytes());
        let mut key = Zeroizing::new([0u8; 32]);
        hkdf::Hkdf::<sha2::Sha256>::new(Some(SALT), shared.as_ref())
            .expand(&info, key.as_mut())
            .map_err(|_| "hkdf")?;
        let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| "aes key")?;
        cipher
            .decrypt(Nonce::from_slice(&iv), Payload { msg: &ct, aad: attempt.as_bytes() })
            .map(Zeroizing::new)
            .map_err(|_| "the sealed PRF does not open with this session's key and attempt".to_string())
    }
}
