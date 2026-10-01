//! Ed25519 stable identity + KERI pre-rotation commitment, persisted to a 0600
//! key file. The Ed25519 public key is the stable peer id used everywhere
//! (IdentityKey, pairwise canonicalisation, bundle signatures).
//!
//! THE KEY AND ITS KEEPING ARE TWO THINGS, and this module is split along that
//! line by `feature = "storage"`. Deriving, signing, verifying, the recovery
//! phrase and `parse_identity_key` are arithmetic and travel anywhere. The 0600
//! file does not: a 0600 mode is a unix guarantee, and a target without a
//! filesystem cannot make it. So the doors that touch disk — `init`,
//! `init_from_recovery_key`, `load` — are gated, and a browser keeps its device
//! key by a different mechanism entirely (a non-extractable CryptoKey on an
//! origin the host page cannot read). Same key type, same 32 bytes on the wire,
//! different custody.

#[cfg(feature = "storage")]
use std::io::Write;
#[cfg(feature = "storage")]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(feature = "storage")]
use std::path::Path;

use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
#[cfg(feature = "storage")]
use rand_core::OsRng;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

#[cfg(feature = "storage")]
use crate::paths;
use crate::CoreError;

/// The locally-held identity. `current` is the active Ed25519 key (the stable
/// id); `next` is the PRE-ROTATED key whose hash we already committed to (KERI).
///
/// `seed` is the 32 bytes BOTH keys are derived from — present on every identity
/// minted from 12 Aug on, absent on a legacy key file that stored the two secrets
/// directly. It is what makes an identity restorable: hold the seed and the whole
/// identity comes back, which is exactly what a recovery key is
/// ([`Identity::recovery_key`]).
pub struct Identity {
    pub current: SigningKey,
    pub next: SigningKey,
    seed: Option<Zeroizing<[u8; 32]>>,
}

impl Identity {
    /// 32-byte Ed25519 public key = the stable peer id used everywhere.
    pub fn identity_pk(&self) -> [u8; 32] {
        self.current.verifying_key().to_bytes()
    }

    /// KERI pre-rotation commitment = SHA-256(next public key).
    pub fn next_key_commit(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(self.next.verifying_key().to_bytes());
        h.finalize().into()
    }

    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        self.current.sign(msg).to_bytes()
    }

    /// User-facing identifier: the Ed25519 identity PUBLIC KEY, rendered.
    /// (Writer moves to `ed25519:` in pass 5; the reader already takes both.)
    pub fn identity_key(&self) -> String {
        format!("{IDENTITY_KEY_PREFIX}{}", hex::encode(self.identity_pk()))
    }

    /// Human SAS readout: first 8 bytes, colon-grouped.
    pub fn fingerprint(&self) -> String {
        let pk = self.identity_pk();
        pk[..8]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":")
    }

    /// A read-aloud "verify you match" rendering of the SAME identity as the hex
    /// [`fingerprint`], as three deterministic words. Same identity → same words
    /// on every device, so two people confirm by reading them aloud. It is a
    /// FRIENDLY view, not a replacement: three BIP-39 words carry 33 bits, vs the
    /// hex fingerprint's 64 — keep the hex available for the full check.
    pub fn fingerprint_words(&self) -> String {
        words_from_pk(&self.identity_pk())
    }

    /// The RECOVERY KEY: the 32-byte seed this identity was derived from, written
    /// as a 24-word BIP-39 mnemonic. Hand it back and [`init_from_recovery_key`]
    /// reconstitutes the same identity — same public key, same id, every peer's
    /// pin still matching. `None` for a legacy identity minted before seeds
    /// existed: those two secrets were generated independently and there is no
    /// seed to show. Never stored anywhere but the key file; nothing transmits it.
    ///
    /// TWENTY-FOUR words, not three. The three-word [`fingerprint_words`] are
    /// PUBLIC (derived from the public key, read aloud to compare during pairing);
    /// these are SECRET and restore the whole identity on their own, so they carry
    /// the seed's full 256 bits rather than a 33-bit view of it. The difference in
    /// shape is the safeguard — a person cannot mistake a page of words for the
    /// three they were asked to say out loud.
    pub fn recovery_key(&self) -> Option<String> {
        Some(recovery_key_from_seed(self.seed.as_ref()?))
    }

    /// Whether this identity can emit a recovery key at all (see
    /// [`recovery_key`](Self::recovery_key)) — the app asks before offering the step.
    pub fn is_recoverable(&self) -> bool {
        self.seed.is_some()
    }

    /// The 32-byte seed itself — what the wrap seals (`Node::export_wrap`) so a
    /// restore comes back as the SAME identity, and what the storage root is
    /// derived from. `None` for a legacy identity that stored its two secrets
    /// directly: there is no seed behind it, so it cannot be wrapped, and the
    /// caller must refuse loudly rather than ship a wrap that would restore as a
    /// stranger. Zeroizing, as every copy of the
    /// seed is; it is handed out only to be sealed.
    pub fn seed_bytes(&self) -> Option<Zeroizing<[u8; 32]>> {
        self.seed.clone()
    }

    /// A deterministic identity held only in memory — nothing is read from or
    /// written to disk. For tests and fixtures that need a signer, so they need
    /// no key file and no state dir.
    pub fn in_memory(seed: [u8; 32]) -> Self {
        let seed = Zeroizing::new(seed);
        let (current, next) = keys_from_seed(&seed);
        Self {
            current,
            next,
            seed: Some(seed),
        }
    }
}

/// Derive the identity keypairs from a 32-byte seed. Deterministic and
/// domain-separated: the same seed always yields the same `current`/`next` pair,
/// and neither info string can produce the other's key.
fn keys_from_seed(seed: &[u8; 32]) -> (SigningKey, SigningKey) {
    let hk = hkdf::Hkdf::<Sha256>::new(Some(SEED_SALT), seed);
    let mut cur = Zeroizing::new([0u8; 32]);
    let mut nxt = Zeroizing::new([0u8; 32]);
    // HKDF-Expand cannot fail for a 32-byte output.
    hk.expand(SEED_INFO_CURRENT, cur.as_mut()).expect("32-byte OKM");
    hk.expand(SEED_INFO_NEXT, nxt.as_mut()).expect("32-byte OKM");
    (SigningKey::from_bytes(&cur), SigningKey::from_bytes(&nxt))
}

/// THE ACCOUNT CHANNEL — the one derivation a passkey's PRF secret feeds.
///
/// A person signed into the same passkey on a phone and in a browser gets the
/// same 32 bytes out of the authenticator, and this turns those into the two
/// values the channel needs: a `tag` (where on the relay the two devices meet —
/// public, and never transmitted, only derived on both sides) and a `seal` (the
/// AES-GCM key the bytes travel under).
///
/// # Why this is in the core and not in each client
///
/// It was in `keyholder.js::split()` alone, and iOS was about to grow a second
/// copy in Swift. That is the failure `core-wasm`'s own header names about SAS
/// words — a client that derives by its own route and drifts "does not fail
/// loudly". Here it is worse than that: two derivations of one channel key put
/// the phone and the browser on DIFFERENT TAGS, where neither ever hears the
/// other and nothing anywhere reports an error. Silence is the whole failure.
///
/// So: one implementation, exported through the FFI for iOS and through wasm for
/// the browser, and no client computes it.
///
/// # The salt is used twice, deliberately
///
/// [`ACCOUNT_CHANNEL_DOMAIN`] is both the PRF `saltInput` (what the authenticator
/// evaluates its secret over) and the HKDF salt here. Both sides must use the
/// identical bytes for both, which is the second reason this constant lives in
/// one place rather than being retyped per client.
pub const ACCOUNT_CHANNEL_DOMAIN: &[u8] = b"pacific/account-channel/v0";
const CHANNEL_INFO_TAG: &[u8] = b"tag";
const CHANNEL_INFO_SEAL: &[u8] = b"seal";

/// The OTHER channel: two devices hand-paired over X25519, held at once. Same
/// split, different domain, so a device channel and an account channel derived
/// from the same bytes could never collide.
///
/// It has no second platform today — both ends are the keyholder — but it lives
/// here for the same reason the account channel does: the moment a second
/// implementation exists, a mismatched domain is silent.
pub const DEVICE_CHANNEL_DOMAIN: &[u8] = b"pacific/device-channel/v0";

/// Split a hand-paired X25519 shared secret into `(address seed, seal)`.
///
/// The first half was the relay tag itself until 18 Sep 2026. It is now the SEED
/// of the channel's relay address (`pacific_wire::address::Address::from_seed`):
/// the tag on the wire is that address's public key, and only a holder of the seed
/// can publish there. The bytes are unchanged; only what they are used as moved.
pub fn device_channel(shared: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    split_channel(DEVICE_CHANNEL_DOMAIN, shared)
}

fn split_channel(domain: &[u8], secret: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let hk = hkdf::Hkdf::<Sha256>::new(Some(domain), secret);
    let mut tag = [0u8; 32];
    let mut seal = [0u8; 32];
    // HKDF-Expand cannot fail for a 32-byte output.
    hk.expand(CHANNEL_INFO_TAG, &mut tag).expect("32-byte OKM");
    hk.expand(CHANNEL_INFO_SEAL, &mut seal).expect("32-byte OKM");
    (tag, seal)
}

/// Split a secret into `(address seed, seal)`. Deterministic, and
/// domain-separated so neither info string can produce the other's bytes. The
/// first half is the seed of the channel's relay address, as for
/// [`device_channel`].
///
/// Matches WebCrypto's HKDF exactly — `deriveBits({name:'HKDF', hash:'SHA-256',
/// salt, info}, key, 256)` is RFC 5869 extract-and-expand over the same inputs,
/// which is what makes one implementation servable to a browser at all.
pub fn account_channel(prf: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    split_channel(ACCOUNT_CHANNEL_DOMAIN, prf)
}

/// A seed written as its recovery key — the other direction from
/// [`seed_from_recovery_key`], and the one every surface needs at signup. Free
/// function rather than a method because the words are a property of the SEED,
/// not of an identity: the web page holds a seed for a few milliseconds before
/// any identity exists, and it still has to show the person their words.
///
/// Infallible: 32 bytes of entropy is always a valid BIP-39 input (24 words).
pub fn recovery_key_from_seed(seed: &[u8; 32]) -> String {
    bip39::Mnemonic::from_entropy(seed)
        .expect("32 bytes is a valid BIP-39 entropy length")
        .to_string()
}

/// Read a recovery key (a 24-word BIP-39 mnemonic) back into the 32-byte seed it
/// encodes. Loud on a bad phrase — a mistyped word fails the mnemonic checksum
/// here rather than silently reconstituting a DIFFERENT identity, which is the
/// whole reason the phrase is checksummed.
pub fn seed_from_recovery_key(phrase: &str) -> Result<[u8; 32], CoreError> {
    let normalised = phrase.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let m = bip39::Mnemonic::parse_normalized(&normalised)
        .map_err(|e| CoreError::Identity(format!("bad recovery key: {e}")))?;
    let (entropy, len) = m.to_entropy_array();
    if len != 32 {
        return Err(CoreError::Identity(format!(
            "recovery key must be 24 words (got {} carrying {len} bytes)",
            normalised.split_whitespace().count()
        )));
    }
    Ok(entropy[..32].try_into().expect("checked 32"))
}

/// Deterministic three-word SAS for an identity pubkey. Both parties derive the
/// same words from the same key. Uses SHA-256(pk) for uniform bits, then the
/// first three words of the BIP-39 mnemonic over 16 bytes of that hash (each word
/// = 11 bits → 33 bits shown).
pub fn words_from_pk(pk: &[u8; 32]) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(pk);
    let mnemonic = bip39::Mnemonic::from_entropy(&hash[..16])
        .expect("16-byte entropy is always a valid BIP-39 input");
    mnemonic
        .to_string()
        .split_whitespace()
        .take(3)
        .collect::<Vec<_>>()
        .join(" ")
}

/// THE AUTH PAYLOAD — the bytes an account signs to prove itself to a server.
///
/// ONE definition, because the signer (a device) and the verifier (an Arc, in
/// another language) must agree byte for byte or every sign-in fails in a way
/// neither side can see. Two properties come from the shape:
///
///   - the AUDIENCE is inside the signed bytes, so a signature captured by one
///     server cannot be replayed against another;
///   - the NONCE is the server's own, single-use, so there is no replay window
///     at all — where a timestamp would leave one open and need a clock both
///     sides agree on.
///
/// The passkey is nowhere in it. The server knows an account by its identity
/// public key and challenges it on that; the passkey only unwraps the seed on
/// the device, and never leaves the authenticator.
pub fn auth_payload(audience: &str, identity_key: &str, nonce: &str) -> Vec<u8> {
    format!("{AUTH_DOMAIN}\n{audience}\n{identity_key}\n{nonce}").into_bytes()
}

/// The first line of [`auth_payload`], and its domain separation.
///
/// A NAMED CONSTANT because it could not otherwise be PINNED. This string is
/// written out again in `business/website/auth/app/auth.py`, in
/// `app/web/account/pacific-account.js` and in `app/web/smoke/e2e.py` — two
/// repositories, no atomic commit — and a signer and a verifier that disagree by one
/// byte fail in a way neither side can see, because the verifier refuses without
/// saying which part was wrong. While it lived inline in the format string above
/// there was nothing for a cross-repo test to read.
/// Pinned by `business/website/auth/tests/test_cross_repo_domains.py`.
pub const AUTH_DOMAIN: &str = "pacific-auth:v1";

/// Verify a signature against a claimed identity pubkey. Loud on failure.
pub fn verify_sig(identity_pk: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> Result<(), CoreError> {
    let vk = VerifyingKey::from_bytes(identity_pk)
        .map_err(|e| CoreError::Identity(format!("bad pubkey: {e}")))?;
    let s = ed25519_dalek::Signature::from_bytes(sig);
    // Strict: a small-order key or R is refused. The identity point with R = identity,
    // s = 0 otherwise verifies over every message (NC-46).
    vk.verify_strict(msg, &s)
        .map_err(|_| CoreError::Identity("signature verify failed".into()))
}

/// Parse any rendering of an IdentityKey — `ed25519:<hex>`, legacy
/// `space1<hex>`, or bare hex — into the 32 bytes it names.
pub fn parse_identity_key(s: &str) -> Result<[u8; 32], CoreError> {
    // ONE rendering, plus the bare hex the wire and the directory natively carry.
    // The pre-12-Aug `space1` form is NOT accepted: backwards compatibility is
    // expressly out of scope for this change (Ralph, 12 Aug) — there are no users
    // to protect and a lenient parser would keep the dead vocabulary alive in
    // every fold written from here on. Dev devices carrying the old form are
    // wiped, not migrated.
    let hexpart = s.strip_prefix(IDENTITY_KEY_PREFIX).unwrap_or(s);
    let raw = hex::decode(hexpart)
        .map_err(|e| CoreError::Identity(format!("bad IdentityKey '{s}': {e}")))?;
    raw.try_into()
        .map_err(|_| CoreError::Identity(format!("IdentityKey '{s}' is not 32 bytes")))
}

/// The rendering: an Ed25519 identity public key, self-describing so a second
/// curve could ever be added without ambiguity.
pub const IDENTITY_KEY_PREFIX: &str = "ed25519:";


/// On-disk key file, LEGACY form: 32 bytes current secret || 32 bytes next
/// secret, 0600. Still read; never written any more.
#[cfg(feature = "storage")]
const KEY_FILE_LEN: usize = 64;

/// On-disk key file, CURRENT form (12 Aug): the 32-byte SEED, 0600. `current` and
/// `next` are derived from it on every load, so the file holds one secret instead
/// of two and that one secret is a recovery key. Length is what distinguishes the
/// two forms — 32 vs 64 — which needs no version byte and cannot be ambiguous.
#[cfg(feature = "storage")]
const SEED_FILE_LEN: usize = 32;

/// Seed derivation, domain-separated so `current` and `next` can never collide
/// and neither can be confused with any other key derived from the same seed.
const SEED_SALT: &[u8] = b"pacific/identity/seed/v1";
const SEED_INFO_CURRENT: &[u8] = b"pacific/identity/current/v1";
const SEED_INFO_NEXT: &[u8] = b"pacific/identity/next/v1";

/// At-rest domain-separation context for the identity key file.
#[cfg(feature = "storage")]
const ATREST_CTX: &[u8] = b"pacific/id_ed25519";

/// Create a fresh identity and persist it. Loud refusal if one already exists —
/// never silently overwrite a key (that would destroy the stable id).
#[cfg(feature = "storage")]
pub fn init() -> Result<Identity, CoreError> {
    let mut seed = Zeroizing::new([0u8; 32]);
    rand_core::RngCore::fill_bytes(&mut OsRng, seed.as_mut());
    write_seed(&seed)
}

/// Bring an identity BACK from its recovery key — the RESTORE door. The same
/// seed re-derives the same keys, so this returns you as unmistakably yourself:
/// same public key, same id, every peer's pin still matching.
///
/// It restores the IDENTITY and nothing else. MLS group state and history cannot
/// be re-derived from a key (they live in `pacific.db`), so a restored device
/// comes back with no groups until it is re-admitted or a backup is opened
/// beside this. Same loud refusal as [`init`] when a key file already exists.
#[cfg(feature = "storage")]
pub fn init_from_recovery_key(phrase: &str) -> Result<Identity, CoreError> {
    let seed = Zeroizing::new(seed_from_recovery_key(phrase)?);
    write_seed(&seed)
}

/// Bring an identity back from its raw SEED — the WRAP door, the twin of
/// [`init_from_recovery_key`] for a seed that arrived out of a wrap
/// (`Node::restore_from_wrap`) rather than as 24 typed words. Same
/// derivation, same 0600 key file, same loud refusal when a key file already
/// exists: a device that holds an identity is never silently overwritten by a
/// restore, however the seed arrived.
#[cfg(feature = "storage")]
pub fn init_from_seed(seed: &[u8; 32]) -> Result<Identity, CoreError> {
    write_seed(&Zeroizing::new(*seed))
}

/// The shared tail of both doors: derive from `seed`, persist it 0600 (sealed
/// when a device key is configured), hand back the identity.
#[cfg(feature = "storage")]
fn write_seed(seed: &Zeroizing<[u8; 32]>) -> Result<Identity, CoreError> {
    let path = paths::id_key_path();
    if path.exists() {
        return Err(CoreError::IdentityExists(path.display().to_string()));
    }
    std::fs::create_dir_all(paths::state_dir())?;

    let (current, next) = keys_from_seed(seed);

    let blob = Zeroizing::new(seed.to_vec());
    // Seal at rest under the device master key when one is configured; otherwise
    // write plaintext (legacy / CLI). A sealed file can only be reopened with the key.
    let on_disk: Zeroizing<Vec<u8>> = match crate::atrest::device_key()? {
        Some(key) => Zeroizing::new(crate::atrest::seal_at_rest(ATREST_CTX, &blob, &key)?),
        None => blob,
    };

    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true) // race-safe; refuses to clobber
        .mode(0o600)
        .open(&path)?;
    f.write_all(&on_disk)?;
    f.sync_all()?;

    Ok(Identity {
        current,
        next,
        seed: Some(seed.clone()),
    })
}

/// Load the identity, or `NoIdentity` if `id init` hasn't run.
#[cfg(feature = "storage")]
pub fn load() -> Result<Identity, CoreError> {
    let path = paths::id_key_path();
    let raw = match std::fs::read(&path) {
        Ok(b) => Zeroizing::new(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(CoreError::NoIdentity),
        Err(e) => return Err(e.into()),
    };

    // A sealed file can ONLY be opened with the device key — no plaintext fallback.
    let bytes: Zeroizing<Vec<u8>> = if crate::atrest::is_sealed(&raw) {
        match crate::atrest::device_key()? {
            Some(key) => Zeroizing::new(crate::atrest::open_at_rest(ATREST_CTX, &raw, &key)?),
            None => return Err(CoreError::AtRestKeyUnavailable),
        }
    } else {
        // Legacy plaintext. If a device key is now configured, transparently upgrade
        // the file to sealed (one-time migration); either way, read it as plaintext.
        if raw.len() == KEY_FILE_LEN || raw.len() == SEED_FILE_LEN {
            if let Some(key) = crate::atrest::device_key()? {
                migrate_plaintext_to_sealed(&path, &raw, &key);
            }
        }
        raw
    };

    // The two on-disk forms, told apart by length. SEED is what is written now;
    // the 64-byte pair is read for identities minted before seeds existed and is
    // marked unrecoverable (there is no seed behind it to show as a recovery key).
    match bytes.len() {
        SEED_FILE_LEN => {
            let seed = Zeroizing::new(<[u8; 32]>::try_from(&bytes[..]).unwrap());
            let (current, next) = keys_from_seed(&seed);
            Ok(Identity {
                current,
                next,
                seed: Some(seed),
            })
        }
        KEY_FILE_LEN => Ok(Identity {
            current: SigningKey::from_bytes(&bytes[..32].try_into().unwrap()),
            next: SigningKey::from_bytes(&bytes[32..].try_into().unwrap()),
            seed: None,
        }),
        n => Err(CoreError::Identity(format!("key file corrupt: {n} bytes"))),
    }
}

/// One-time upgrade of a legacy plaintext key file to a sealed envelope, once a
/// device key is provisioned. Best-effort: writes a sibling temp then renames, so a
/// failure never corrupts the original (the identity is still returned either way).
#[cfg(feature = "storage")]
fn migrate_plaintext_to_sealed(path: &Path, plaintext: &[u8], key: &[u8; 32]) {
    let Ok(sealed) = crate::atrest::seal_at_rest(ATREST_CTX, plaintext, key) else {
        return;
    };
    let tmp = path.with_extension("resealing");
    let wrote = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .and_then(|mut f| {
            f.write_all(&sealed)?;
            f.sync_all()
        });
    if wrote.is_ok() {
        let _ = std::fs::rename(&tmp, path);
    } else {
        let _ = std::fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    /// THE CROSS-IMPLEMENTATION VECTOR. These bytes were produced by WebCrypto —
    /// `deriveBits({name:'HKDF', hash:'SHA-256', salt, info}, key, 256)` over the
    /// same PRF secret — and are pinned here so the Rust and the browser cannot
    /// drift apart silently.
    ///
    /// Silence is exactly what drift looks like: two devices on different tags
    /// simply never hear each other, and nothing anywhere raises an error. A
    /// vector one side computed and the other asserts is the only thing that
    /// catches it, and it is the same safeguard `pacific-wasm-test.html` applies
    /// to canonical bytes and the DeltaId.
    #[test]
    fn account_channel_matches_webcrypto() {
        let mut prf = [0u8; 32];
        for (i, b) in prf.iter_mut().enumerate() {
            *b = i as u8;
        }
        let (tag, seal) = account_channel(&prf);
        assert_eq!(
            hex::encode(tag),
            "2f414e3c196b67f875015d8508166181e4fe7eff51ec377c020938d71e46359a",
            "tag drifted from WebCrypto's HKDF"
        );
        assert_eq!(
            hex::encode(seal),
            "841ad168aea4ea48d9872e207a26827e7481dc0b0457484a35c13a5459f997dd",
            "seal drifted from WebCrypto's HKDF"
        );
    }

    /// The two halves are domain-separated, and the domain is the constant both
    /// the PRF salt and the HKDF salt are taken from.
    #[test]
    fn the_two_halves_are_separated_and_deterministic() {
        let prf = [7u8; 32];
        let (tag, seal) = account_channel(&prf);
        assert_ne!(tag, seal, "one info string produced the other's key");
        assert_eq!((tag, seal), account_channel(&prf), "not deterministic");
        let (other_tag, _) = account_channel(&[8u8; 32]);
        assert_ne!(tag, other_tag, "a different secret gave the same tag");
        assert_eq!(ACCOUNT_CHANNEL_DOMAIN, b"pacific/account-channel/v0");
        assert_eq!(DEVICE_CHANNEL_DOMAIN, b"pacific/device-channel/v0");
        // The SAME secret through the two doors must not give the same channel:
        // a device tether and an account link are different facts about who is
        // on the other end, and one must never be mistaken for the other.
        assert_ne!(account_channel(&prf), device_channel(&prf));
    }

    use super::*;

    /// Each test pins its own PACIFIC_STATE_DIR so the key file is isolated.
    fn with_state_dir<F: FnOnce()>(f: F) {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("PACIFIC_STATE_DIR", dir.path());
        f();
        std::env::remove_var("PACIFIC_STATE_DIR");
    }

    /// `with_state_dir` that hands a value back out (the closure cannot return).
    fn with_state_dir_value<T, F: FnOnce() -> T>(f: F) -> T {
        let mut out = None;
        with_state_dir(|| out = Some(f()));
        out.unwrap()
    }

    #[test]
    fn init_load_roundtrip_and_refuses_overwrite() {
        with_state_dir(|| {
            let id = init().unwrap();
            let pk = id.identity_pk();
            let commit = id.next_key_commit();

            // load round-trips the same keys.
            let loaded = load().unwrap();
            assert_eq!(loaded.identity_pk(), pk);
            assert_eq!(loaded.next_key_commit(), commit);

            // a second init refuses (loud), does NOT overwrite.
            assert!(matches!(init(), Err(CoreError::IdentityExists(_))));
        });
    }

    /// ONE rendering, plus bare hex. The retired `space1` form is REFUSED —
    /// backwards compatibility is expressly out of scope (12 Aug), so the old
    /// vocabulary cannot survive in newly authored folds.
    #[test]
    fn parse_takes_the_one_rendering_and_refuses_the_retired_one() {
        let pk = [0xABu8; 32];
        let hexed = hex::encode(pk);
        assert_eq!(
            parse_identity_key(&format!("{IDENTITY_KEY_PREFIX}{hexed}")).unwrap(),
            pk
        );
        assert_eq!(parse_identity_key(&hexed).unwrap(), pk, "bare hex is the wire form");

        // The retired rendering is not a parse error by accident — `space1` is
        // not valid hex, so it fails loudly, which is the point.
        assert!(parse_identity_key(&format!("space1{hexed}")).is_err());
        assert!(parse_identity_key("ed25519:nothex").is_err());
        assert!(parse_identity_key(&format!("{IDENTITY_KEY_PREFIX}ab")).is_err()); // short
        assert!(parse_identity_key("").is_err());
    }

    /// The whole point of a recovery key: the words come back as the SAME
    /// identity — same public key, same id, same pre-rotation commitment — on a
    /// device that has never seen this key before.
    #[test]
    fn recovery_key_restores_the_same_identity() {
        let (pk, commit, phrase) = {
            let mut out = None;
            with_state_dir(|| {
                let id = init().unwrap();
                out = Some((
                    id.identity_pk(),
                    id.next_key_commit(),
                    id.recovery_key().expect("a fresh identity is recoverable"),
                ));
            });
            out.unwrap()
        };

        assert_eq!(phrase.split_whitespace().count(), 24, "full 256-bit seed");

        // A DIFFERENT device (a fresh state dir, no key file) takes the words.
        with_state_dir(|| {
            let back = init_from_recovery_key(&phrase).unwrap();
            assert_eq!(back.identity_pk(), pk, "same identity key");
            assert_eq!(back.next_key_commit(), commit, "same rotation commitment");
            assert_eq!(back.recovery_key().as_deref(), Some(phrase.as_str()));
            // …and it survives a reload from the seed file it just wrote.
            assert_eq!(load().unwrap().identity_pk(), pk);
        });
    }

    /// A mistyped word must fail the BIP-39 checksum, LOUDLY — silently deriving
    /// some other identity is the one outcome a recovery flow cannot have.
    #[test]
    fn a_corrupt_recovery_key_is_refused_not_reinterpreted() {
        let phrase = with_state_dir_value(|| init().unwrap().recovery_key().unwrap());

        let mut words: Vec<&str> = phrase.split_whitespace().collect();
        words[0] = if words[0] == "zoo" { "zone" } else { "zoo" };
        let tampered = words.join(" ");

        assert!(seed_from_recovery_key(&tampered).is_err(), "checksum catches it");
        assert!(seed_from_recovery_key("not even words").is_err());
        assert!(seed_from_recovery_key("").is_err());
        // A VALID 12-word mnemonic is a valid phrase but only 16 bytes — refused,
        // because half a seed would derive a confidently wrong identity.
        let short = bip39::Mnemonic::from_entropy(&[7u8; 16]).unwrap().to_string();
        assert!(seed_from_recovery_key(&short).is_err());
        // Case and stray whitespace are the user's, not an error.
        assert!(seed_from_recovery_key(&format!("  {}  ", phrase.to_uppercase())).is_ok());
    }

    /// Seed derivation is domain-separated: `current` and `next` are different
    /// keys, and neither is the seed itself.
    #[test]
    fn seed_derives_two_distinct_keys_deterministically() {
        let seed = [0x5Au8; 32];
        let (c1, n1) = keys_from_seed(&seed);
        let (c2, n2) = keys_from_seed(&seed);
        assert_eq!(c1.to_bytes(), c2.to_bytes(), "deterministic");
        assert_eq!(n1.to_bytes(), n2.to_bytes(), "deterministic");
        assert_ne!(c1.to_bytes(), n1.to_bytes(), "domain-separated");
        assert_ne!(c1.to_bytes(), seed, "not the seed in the clear");
        assert_ne!(n1.to_bytes(), seed);
        let (c3, _) = keys_from_seed(&[0x5Bu8; 32]);
        assert_ne!(c1.to_bytes(), c3.to_bytes());
    }

    /// The seed file is 32 bytes and holds only the seed; a pre-seed 64-byte file
    /// still loads, and honestly reports that it has no recovery key to give.
    #[test]
    fn legacy_pair_file_loads_but_is_not_recoverable() {
        with_state_dir(|| {
            assert_eq!(
                std::fs::read(paths::id_key_path()).map(|b| b.len()).ok(),
                None
            );
            let id = init().unwrap();
            assert_eq!(
                std::fs::read(paths::id_key_path()).unwrap().len(),
                SEED_FILE_LEN
            );
            assert!(id.is_recoverable());
        });

        with_state_dir(|| {
            // Hand-write the legacy form: two independently generated secrets.
            std::fs::create_dir_all(paths::state_dir()).unwrap();
            let current = SigningKey::generate(&mut OsRng);
            let next = SigningKey::generate(&mut OsRng);
            let mut blob = Vec::with_capacity(KEY_FILE_LEN);
            blob.extend_from_slice(&current.to_bytes());
            blob.extend_from_slice(&next.to_bytes());
            std::fs::write(paths::id_key_path(), &blob).unwrap();

            let id = load().unwrap();
            assert_eq!(id.identity_pk(), current.verifying_key().to_bytes());
            assert!(!id.is_recoverable(), "no seed behind a legacy pair");
            assert!(id.recovery_key().is_none());
        });
    }

    /// The restore door observes the same never-overwrite rule as the mint: a
    /// device that already holds an identity refuses, loudly.
    #[test]
    fn restore_refuses_over_an_existing_identity() {
        with_state_dir(|| {
            let phrase = init().unwrap().recovery_key().unwrap();
            assert!(matches!(
                init_from_recovery_key(&phrase),
                Err(CoreError::IdentityExists(_))
            ));
        });
    }

    #[test]
    fn next_key_commit_is_sha256_of_next_pk() {
        with_state_dir(|| {
            let id = init().unwrap();
            let mut h = Sha256::new();
            h.update(id.next.verifying_key().to_bytes());
            let expect: [u8; 32] = h.finalize().into();
            assert_eq!(id.next_key_commit(), expect);
        });
    }

    #[test]
    fn sign_verify_roundtrip_and_space_id() {
        with_state_dir(|| {
            let id = init().unwrap();
            let msg = b"pacific bundle bytes";
            let sig = id.sign(msg);
            verify_sig(&id.identity_pk(), msg, &sig).unwrap();
            // tamper -> loud failure.
            assert!(verify_sig(&id.identity_pk(), b"other", &sig).is_err());
            // identity_key round-trips through parse_identity_key.
            assert_eq!(parse_identity_key(&id.identity_key()).unwrap(), id.identity_pk());
        });
    }

    #[test]
    fn an_auth_signature_binds_its_audience_and_its_nonce() {
        // The two properties the Arc's verifier relies on. Neither is provable
        // by reading the signature: they hold only because both sides build the
        // payload from `auth_payload` and nothing else.
        let id = Identity::in_memory([5u8; 32]);
        let key = id.identity_key();
        let sig = id.sign(&auth_payload("arc.kenjin.cc", &key, "nonce-1"));

        verify_sig(
            &id.identity_pk(),
            &auth_payload("arc.kenjin.cc", &key, "nonce-1"),
            &sig,
        )
        .expect("the signature verifies against the payload it was made for");

        // Captured by one Arc, useless at another.
        assert!(verify_sig(
            &id.identity_pk(),
            &auth_payload("arc.attacker.example", &key, "nonce-1"),
            &sig
        )
        .is_err());

        // Replayed against a second challenge, useless again.
        assert!(verify_sig(
            &id.identity_pk(),
            &auth_payload("arc.kenjin.cc", &key, "nonce-2"),
            &sig
        )
        .is_err());
    }

    /// Pin PACIFIC_STATE_DIR + PACIFIC_ATREST_KEY together, cleaning both up.
    fn with_state_dir_and_key<F: FnOnce()>(hexkey: &str, f: F) {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        crate::atrest::clear_device_key();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("PACIFIC_STATE_DIR", dir.path());
        std::env::set_var("PACIFIC_ATREST_KEY", hexkey);
        f();
        std::env::remove_var("PACIFIC_ATREST_KEY");
        std::env::remove_var("PACIFIC_STATE_DIR");
    }

    #[test]
    fn at_rest_sealed_when_key_present_and_round_trips() {
        with_state_dir_and_key(&"11".repeat(32), || {
            let id = init().unwrap();
            let pk = id.identity_pk();

            // On disk it is sealed: has the magic, is not the plaintext seed, and
            // carries neither the secret scalar NOR the seed in the clear — the
            // seed is the whole identity now, so it is the one that matters most.
            let raw = std::fs::read(paths::id_key_path()).unwrap();
            assert!(crate::atrest::is_sealed(&raw));
            assert_ne!(raw.len(), SEED_FILE_LEN);
            let secret = id.current.to_bytes();
            assert!(!raw.windows(secret.len()).any(|w| w == &secret[..]));
            let seed = seed_from_recovery_key(&id.recovery_key().unwrap()).unwrap();
            assert!(!raw.windows(seed.len()).any(|w| w == &seed[..]), "the seed is sealed");

            // load round-trips the same identity with the key.
            assert_eq!(load().unwrap().identity_pk(), pk);
        });
    }

    #[test]
    fn sealed_without_key_fails_loud_and_wrong_key_fails() {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        crate::atrest::clear_device_key();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("PACIFIC_STATE_DIR", dir.path());

        std::env::set_var("PACIFIC_ATREST_KEY", "33".repeat(32));
        let pk = init().unwrap().identity_pk();

        // No key on a sealed file -> loud, never a plaintext fallback.
        std::env::remove_var("PACIFIC_ATREST_KEY");
        assert!(matches!(load(), Err(CoreError::AtRestKeyUnavailable)));

        // Wrong key -> loud AEAD failure.
        std::env::set_var("PACIFIC_ATREST_KEY", "44".repeat(32));
        assert!(matches!(load(), Err(CoreError::AtRest(_))));

        // Right key -> back.
        std::env::set_var("PACIFIC_ATREST_KEY", "33".repeat(32));
        assert_eq!(load().unwrap().identity_pk(), pk);

        std::env::remove_var("PACIFIC_ATREST_KEY");
        std::env::remove_var("PACIFIC_STATE_DIR");
    }

    #[test]
    fn legacy_plaintext_migrates_when_key_provisioned() {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        crate::atrest::clear_device_key();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("PACIFIC_STATE_DIR", dir.path());
        std::env::remove_var("PACIFIC_ATREST_KEY");

        // Init with NO key -> an unsealed plaintext key file.
        let pk = init().unwrap().identity_pk();
        let raw = std::fs::read(paths::id_key_path()).unwrap();
        assert_eq!(raw.len(), SEED_FILE_LEN);
        assert!(!crate::atrest::is_sealed(&raw));

        // Provision a key -> load upgrades the file in place, and still loads.
        std::env::set_var("PACIFIC_ATREST_KEY", "55".repeat(32));
        assert_eq!(load().unwrap().identity_pk(), pk);
        assert!(crate::atrest::is_sealed(
            &std::fs::read(paths::id_key_path()).unwrap()
        ));
        assert_eq!(load().unwrap().identity_pk(), pk);

        std::env::remove_var("PACIFIC_ATREST_KEY");
        std::env::remove_var("PACIFIC_STATE_DIR");
    }
}
