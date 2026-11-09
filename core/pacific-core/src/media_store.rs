//! Detached media (O-79, the ICD's `media.detached`, #56, ruled 9 Nov 2026): the bytes go to
//! the relay's object storage, sealed, and a Delta carries only the reference.
//!
//! Every new upload has a fresh 32-byte content key `K` and a fresh object key (32 random
//! bytes, 64 lowercase hex: the only name the blob store presigns). The object is
//! [`seal::seal_with_key`] of the bytes under `K`, with the object key's 64 hex characters as
//! the AAD, so an object cannot be passed off under another name. The digest is SHA-256 of the
//! object as stored, so anyone can check a fetch without `K`. Fetching needs only the key: a
//! device holding no group for the object (a carried exterior) reads it as a member does.
//!
//! Each call dials a session of its own: declaring the media level ends a session's
//! subscriptions ([`RelaySession::media_url`]).
//!
//! An object never changes (it only creates, and is checked against its digest), so a device
//! that has it never fetches it again: `kept`, a directory of objects as stored (sealed), named
//! by key and checked against the digest each time they are read back (ARC_NETWORK).
//!
//! An upload only creates (`If-None-Match: *`, signed into the URL): with nothing ever deleted,
//! an overwrite would be a deletion anyone holding a reference could make.
//!
//! XChaCha20-Poly1305 does not commit to its key: one object can be crafted to open to different
//! bytes under different keys. A digest therefore names an object, never its content: nothing
//! may dedupe, match or report media by digest without first adding a key commitment here.

use std::path::Path;

use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::media::{Delivery, MediaRef};
use crate::seal;
use crate::transport::RelaySession;
use crate::CoreError;

static PRESIGNED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many presigns this process has asked a relay for. Not a relay blob, so not in
/// `router::published()`, but a settled node that keeps presigning still spends battery and
/// object storage: the quiescence tests count it beside that (ARC_NETWORK).
pub fn presigned() -> u64 {
    PRESIGNED.load(std::sync::atomic::Ordering::Relaxed)
}

/// A presigned URL for `key` on a session of its own, counted.
async fn presign(relay: &str, key: &str, put_len: Option<u64>) -> Result<String, CoreError> {
    PRESIGNED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut s = RelaySession::connect(relay).await?;
    let url = s.media_url(key, put_len).await;
    s.close().await;
    presigned_url(relay, url?)
}

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

// The ICD's media.detached is the construction this module runs: a content key and a nonce
// of the sizes it states.
const _: () = assert!(crate::icd_consts::MEDIA_KEY_BYTES == 32 && crate::icd_consts::MEDIA_NONCE_BYTES == seal::NONCE_LEN);

fn random32() -> Result<[u8; 32], CoreError> {
    let mut b = [0u8; 32];
    getrandom::getrandom(&mut b).map_err(|e| CoreError::Seal(format!("no randomness: {e}")))?;
    Ok(b)
}

/// Seal `bytes`, upload them through `relay`'s presign, and return the reference a Delta carries.
/// The object is kept in `kept`, so its uploader never fetches it.
pub async fn put(relay: &str, kept: Option<&Path>, bytes: &[u8], mime: &str) -> Result<MediaRef, CoreError> {
    let (k, key) = (random32()?, hex::encode(random32()?));
    let object = seal::seal_with_key(bytes, &k, key.as_bytes())?;
    let digest: [u8; 32] = Sha256::digest(&object).into();
    let reference = MediaRef::sealed(mime, digest, object.len() as u64, key.clone(), b64().encode(k))
        .map_err(|e| CoreError::Coordinator(format!("media: {e}")))?;
    let url = presign(relay, &key, Some(object.len() as u64)).await?;
    let res = http()?
        .put(url)
        .header(reqwest::header::IF_NONE_MATCH, "*")
        .body(object.clone())
        .send()
        .await
        .map_err(|e| CoreError::Transport(format!("media upload: {e}")))?;
    if !res.status().is_success() {
        return Err(CoreError::Transport(format!("media upload: object storage answered {}", res.status())));
    }
    if let Some(dir) = kept {
        keep(dir, &key, &object);
    }
    Ok(reference)
}

/// A sealed reference's bytes: from `kept` when the object is there and checks, else fetched
/// through `relay`'s presign, checked against its digest and length, kept, and opened.
/// Anything that does not check is an error, never partial bytes.
pub async fn get(relay: &str, kept: Option<&Path>, r: &MediaRef) -> Result<Vec<u8>, CoreError> {
    let Delivery::Sealed { digest, bytes, key, secret } = &r.delivery else {
        return Err(CoreError::Coordinator("media: not a detached, sealed reference".into()));
    };
    let k: [u8; 32] = b64()
        .decode(secret)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| CoreError::Coordinator("media: the content key is not 32 bytes".into()))?;
    // The key names a file in `kept`: 64 lowercase hex and nothing else, checked here as well as
    // where a reference is parsed, since a MediaRef can be built by hand.
    if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(CoreError::Coordinator("media: the object key is not 64 lowercase hex".into()));
    }
    let checks = |o: &[u8]| o.len() as u64 == *bytes && <[u8; 32]>::from(Sha256::digest(o)) == *digest;
    if let Some(o) = kept.and_then(|d| std::fs::read(d.join(key)).ok()).filter(|o| checks(o)) {
        return seal::open_with_key(&o, &k, key.as_bytes());
    }
    let url = presign(relay, key, None).await?;
    let mut res = http()?.get(url).send().await.map_err(|e| CoreError::Transport(format!("media fetch: {e}")))?;
    if !res.status().is_success() {
        return Err(CoreError::Transport(format!("media fetch: object storage answered {}", res.status())));
    }
    let wrong = || CoreError::Seal("media: the object fetched is not the one referenced".into());
    // Never more than the reference says: a body is refused by its declared length, and
    // abandoned the moment it runs past, before a device holds any more of it.
    if res.content_length().is_some_and(|n| n != *bytes) {
        return Err(wrong());
    }
    let mut object = Vec::with_capacity((*bytes).min(32 << 20) as usize);
    while let Some(chunk) = res.chunk().await.map_err(|e| CoreError::Transport(format!("media fetch: {e}")))? {
        if (object.len() + chunk.len()) as u64 > *bytes {
            return Err(wrong());
        }
        object.extend_from_slice(&chunk);
    }
    if !checks(&object) {
        return Err(wrong());
    }
    if let Some(dir) = kept {
        keep(dir, key, &object);
    }
    seal::open_with_key(&object, &k, key.as_bytes())
}

/// Keep an object under its key: written beside and renamed, so a reader never sees half of one.
/// A failure costs a fetch next time, not the bytes now.
fn keep(dir: &Path, key: &str, object: &[u8]) {
    let tmp = dir.join(format!(".{key}.part"));
    let kept = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&tmp, object)).and_then(|_| std::fs::rename(&tmp, dir.join(key)));
    if let Err(e) = kept {
        tracing::warn!(error = %e, "media: a fetched object could not be kept");
    }
}

/// The relay's presigned URL, taken only as https, unless the relay itself is on this machine
/// (a local stand-in): a relay is not trusted to send a device anywhere it likes in clear.
fn presigned_url(relay: &str, url: String) -> Result<String, CoreError> {
    let local = reqwest::Url::parse(relay).is_ok_and(|u| {
        u.scheme() == "ws"
            && u.username().is_empty()
            && u.password().is_none()
            && matches!(u.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
    });
    if url.starts_with("https://") || (local && url.starts_with("http://")) {
        Ok(url)
    } else {
        Err(CoreError::Transport("media: the relay presigned a URL that is not https".into()))
    }
}

/// No redirects: the presigned URL is the one place the object is.
fn http() -> Result<reqwest::Client, CoreError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(900))
        .build()
        .map_err(|e| CoreError::Transport(format!("media: {e}")))
}

#[cfg(test)]
mod tests {
    use super::presigned_url as presigned;

    /// Plain http only from a relay on this machine, judged by its parsed host, not a prefix.
    #[test]
    fn plain_http_only_from_a_local_relay() {
        let http = || "http://127.0.0.1:9000/b/k".to_string();
        assert!(presigned("ws://127.0.0.1:4000", http()).is_ok());
        assert!(presigned("ws://[::1]:4000/v1/relay", http()).is_ok());
        assert!(presigned("ws://localhost:1@evil.example", http()).is_err(), "userinfo is not a local relay");
        assert!(presigned("wss://arc.wallflowers.io/v1/relay", http()).is_err());
        assert!(presigned("wss://arc.wallflowers.io/v1/relay", "https://acct.r2.cloudflarestorage.com/b/k".into()).is_ok());
    }
}
