//! The wrap — the identity seed, sealed under the passkey, and nothing else.
//!
//! RULED 13 Sep 2026 (D1). The account is two artefacts, not one, and the split
//! is what makes the public half safe to serve:
//!
//!   - THE WRAP (this module) — the 32-byte seed under a key derived from the
//!     passkey's PRF output. Served to ANYONE who asks for it by public key,
//!     because that read is how a new device obtains the key it would
//!     authenticate with. It is a 76-byte envelope (magic, nonce, the sealed
//!     seed and its tag) whose key never leaves an authenticator.
//!   - THE HISTORY — the archive and the MLS snapshot, sealed under a key
//!     derived from the SEED, and served only against a signature. Retired on
//!     14 Sep 2026 as a key escrow; its module is gone (a58798c).
//!
//! What the split buys, stated plainly: the previous design sealed the seed, the
//! archive and the live MLS state into one blob under the PRF, which is why
//! releasing it needed a second factor — handing that blob to whoever presents a
//! synced passkey would make one compromised platform account the whole story.
//! Split, the public artefact carries no history and no ratchet state, so it can
//! be published without a gate and the second factor has nothing left to guard.
//!
//! THE KEY. `wrap_key(prf)` — HKDF-SHA256 off the PRF output under this module's
//! own domain, distinct from the history's and from the account channel's, so no
//! two of the three can ever be confused for one another.
//!
//! THE HOST IS IN THE AAD, and that is the one thing this module does that a
//! bare AEAD would not. A wrap lifted onto an arc the attacker controls will not
//! open, even with the right passkey — so a hostile arc cannot replay someone
//! else's wrap onto a domain it owns and watch them unlock it.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

use crate::CoreError;

/// Domain for the wrap key — the PRF `saltInput` AND the HKDF salt, the same
/// construction the history and the account channel use, deliberately a
/// different string from both.
pub const WRAP_DOMAIN: &[u8] = b"pacific/wrap/v1";
const NONCE_LEN: usize = 24;
const MAGIC: &[u8; 4] = b"PWR1";
const SEED_LEN: usize = 32;

/// The wrap key from the passkey's PRF output:
/// HKDF-SHA256(salt = domain, ikm = prf, info = "kek").
pub fn wrap_key(prf: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let hk = hkdf::Hkdf::<sha2::Sha256>::new(Some(WRAP_DOMAIN), prf);
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(b"kek", out.as_mut()).expect("32-byte OKM");
    out
}

/// What the seal is bound to. The host is the arc the wrap is stored on, exactly
/// as the user handle names it — port included, so a dev arc on `localhost:1400`
/// binds distinctly from one on `:1401`.
fn aad(host: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(WRAP_DOMAIN.len() + 1 + host.len());
    v.extend_from_slice(WRAP_DOMAIN);
    v.push(b'|');
    v.extend_from_slice(host.as_bytes());
    v
}

/// Seal the seed for `host`. Envelope: `MAGIC(4) || nonce(24) || ciphertext`.
pub fn seal(prf: &[u8; 32], seed: &[u8; SEED_LEN], host: &str) -> Result<Vec<u8>, CoreError> {
    if host.is_empty() {
        return Err(CoreError::Seal(
            "a wrap must name the host it is stored on, or it can be replayed onto another".into(),
        ));
    }
    let key = wrap_key(prf);
    let cipher = XChaCha20Poly1305::new((&*key).into());
    let mut nonce = [0u8; NONCE_LEN];
    getrandom_fill(&mut nonce)?;
    let ct = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload { msg: &seed[..], aad: &aad(host) },
        )
        .map_err(|_| CoreError::Seal("wrap seal failed".into()))?;
    let mut out = Vec::with_capacity(4 + NONCE_LEN + ct.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Open a wrap. A wrong passkey, a tampered byte, or a wrap lifted from another
/// arc all fail the AEAD tag — loudly, and indistinguishably, which is correct:
/// none of the three is a state the caller can act on differently.
pub fn open(prf: &[u8; 32], blob: &[u8], host: &str) -> Result<Zeroizing<[u8; SEED_LEN]>, CoreError> {
    if blob.len() < 4 + NONCE_LEN || &blob[..4] != MAGIC {
        return Err(CoreError::Seal("not a wrap".into()));
    }
    let key = wrap_key(prf);
    let cipher = XChaCha20Poly1305::new((&*key).into());
    let plain = cipher
        .decrypt(
            XNonce::from_slice(&blob[4..4 + NONCE_LEN]),
            Payload { msg: &blob[4 + NONCE_LEN..], aad: &aad(host) },
        )
        .map_err(|_| CoreError::Seal("wrap open failed (wrong passkey, wrong host, or tampered)".into()))?;

    // A wrap that opens but is not seed-shaped is a wrap of something else. The
    // AEAD proves who sealed it, not what they sealed.
    if plain.len() != SEED_LEN {
        return Err(CoreError::Seal(format!(
            "wrap opened to {} bytes, not a {SEED_LEN}-byte seed",
            plain.len()
        )));
    }
    let mut seed = Zeroizing::new([0u8; SEED_LEN]);
    seed.copy_from_slice(&plain);
    Ok(seed)
}

/// Entropy without dragging `rand_core` into a no-`storage` build.
fn getrandom_fill(buf: &mut [u8]) -> Result<(), CoreError> {
    getrandom::getrandom(buf).map_err(|e| CoreError::Seal(format!("entropy: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST: &str = "arc.kenjin.cc";

    #[test]
    fn round_trips_under_the_passkey_and_its_host() {
        let prf = [0x11u8; 32];
        let seed = [0x22u8; 32];
        let sealed = seal(&prf, &seed, HOST).unwrap();

        assert_eq!(&sealed[..4], b"PWR1");
        assert_ne!(&sealed[4 + NONCE_LEN..], &seed[..], "the seed is not in the clear");
        assert_eq!(*open(&prf, &sealed, HOST).unwrap(), seed);
    }

    #[test]
    fn a_wrap_lifted_to_another_arc_does_not_open() {
        // The whole reason the host is in the AAD: a hostile arc that serves
        // someone else's wrap from its own domain cannot watch them unlock it.
        let prf = [0x33u8; 32];
        let sealed = seal(&prf, &[0x44u8; 32], HOST).unwrap();
        assert!(open(&prf, &sealed, "arc.attacker.example").is_err());
    }

    #[test]
    fn the_wrong_passkey_and_a_flipped_byte_both_fail() {
        let prf = [0x55u8; 32];
        let sealed = seal(&prf, &[0x66u8; 32], HOST).unwrap();

        assert!(open(&[0x56u8; 32], &sealed, HOST).is_err(), "wrong passkey, no seed");

        let mut tampered = sealed.clone();
        let n = tampered.len();
        tampered[n - 1] ^= 1;
        assert!(open(&prf, &tampered, HOST).is_err(), "one flipped byte, no seed");
    }

    #[test]
    fn a_wrap_is_recognisable_and_a_short_blob_is_refused() {
        assert!(open(&[0u8; 32], b"", HOST).is_err());
        assert!(open(&[0u8; 32], b"PBK1nonsense", HOST).is_err(), "a history blob is not a wrap");
    }

    #[test]
    fn sealing_without_a_host_is_refused_rather_than_defaulted() {
        assert!(seal(&[0u8; 32], &[0u8; 32], "").is_err());
    }

    #[test]
    fn the_wrap_key_is_neither_the_history_key_nor_the_account_channel() {
        // One PRF output, three domains, three unrelated keys. A page holding any
        // one of them can open nothing else.
        let prf = [0x77u8; 32];
        let (_tag, chan_seal) = crate::identity::account_channel(&prf);
        assert_ne!(*wrap_key(&prf), chan_seal);
        // The third arm used to be the history key. That domain is gone with the
        // blob it sealed, so the inequality is pinned against the head key
        // instead — a live storage-family domain off the same bytes. The archive
        // content key joins this list when it has a writer.
        assert_ne!(*wrap_key(&prf), *crate::head::head_key(&prf));
    }

    #[test]
    fn two_seals_of_one_seed_differ() {
        // Random nonce per seal: re-wrapping after enrolling a new passkey must
        // not produce a byte-identical record an observer could match.
        let prf = [0x88u8; 32];
        let seed = [0x99u8; 32];
        assert_ne!(
            seal(&prf, &seed, HOST).unwrap(),
            seal(&prf, &seed, HOST).unwrap()
        );
    }
}
