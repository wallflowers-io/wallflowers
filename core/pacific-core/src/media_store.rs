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
//! An upload only creates (`If-None-Match: *`, signed into the URL): with nothing ever deleted,
//! an overwrite would be a deletion anyone holding a reference could make.
//!
//! XChaCha20-Poly1305 does not commit to its key: one object can be crafted to open to different
//! bytes under different keys. A digest therefore names an object, never its content: nothing
//! may dedupe, match or report media by digest without first adding a key commitment here.

use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::media::{Delivery, MediaRef};
use crate::seal;
use crate::transport::RelaySession;
use crate::CoreError;

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn random32() -> Result<[u8; 32], CoreError> {
    let mut b = [0u8; 32];
    getrandom::getrandom(&mut b).map_err(|e| CoreError::Seal(format!("no randomness: {e}")))?;
    Ok(b)
}

/// Seal `bytes`, upload them through `relay`'s presign, and return the reference a Delta carries.
pub async fn put(relay: &str, bytes: &[u8], mime: &str) -> Result<MediaRef, CoreError> {
    let (k, key) = (random32()?, hex::encode(random32()?));
    let object = seal::seal_with_key(bytes, &k, key.as_bytes())?;
    let digest: [u8; 32] = Sha256::digest(&object).into();
    let reference = MediaRef::sealed(mime, digest, object.len() as u64, key.clone(), b64().encode(k))
        .map_err(|e| CoreError::Coordinator(format!("media: {e}")))?;
    let mut s = RelaySession::connect(relay).await?;
    let url = s.media_url(&key, Some(object.len() as u64)).await;
    s.close().await;
    let url = presigned(relay, url?)?;
    let res = http()?
        .put(url)
        .header(reqwest::header::IF_NONE_MATCH, "*")
        .body(object)
        .send()
        .await
        .map_err(|e| CoreError::Transport(format!("media upload: {e}")))?;
    if !res.status().is_success() {
        return Err(CoreError::Transport(format!("media upload: object storage answered {}", res.status())));
    }
    Ok(reference)
}

/// Fetch a sealed reference's object through `relay`'s presign, check it against its digest
/// and length, and open it. Anything that does not check is an error, never partial bytes.
pub async fn get(relay: &str, r: &MediaRef) -> Result<Vec<u8>, CoreError> {
    let Delivery::Sealed { digest, bytes, key, secret } = &r.delivery else {
        return Err(CoreError::Coordinator("media: not a detached, sealed reference".into()));
    };
    let k: [u8; 32] = b64()
        .decode(secret)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| CoreError::Coordinator("media: the content key is not 32 bytes".into()))?;
    let mut s = RelaySession::connect(relay).await?;
    let url = s.media_url(key, None).await;
    s.close().await;
    let url = presigned(relay, url?)?;
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
    let got: [u8; 32] = Sha256::digest(&object).into();
    if object.len() as u64 != *bytes || got != *digest {
        return Err(wrong());
    }
    seal::open_with_key(&object, &k, key.as_bytes())
}

/// The relay's presigned URL, taken only as https, unless the relay itself is on this machine
/// (a local stand-in): a relay is not trusted to send a device anywhere it likes in clear.
fn presigned(relay: &str, url: String) -> Result<String, CoreError> {
    let local = ["ws://127.0.0.1:", "ws://localhost:", "ws://[::1]:"].iter().any(|p| relay.starts_with(p));
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
