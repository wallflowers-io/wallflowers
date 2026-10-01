//! blobs — a content store for blob BODIES, and the R2 implementation of it.
//!
//! ## This is the arc authoring blobs to R2, and it is not `media.rs`
//!
//! `media.rs` already talks to R2 and must not be duplicated, so read this first:
//! the two paths touch the same bucket and have opposite postures.
//!
//! | | `media.rs` | this module |
//! |---|---|---|
//! | who writes the object | THE CLIENT | THE ARC |
//! | what the relay does | mints a presigned URL and gets out of the way | issues the PUT/GET itself |
//! | bytes through this process | never | every body it stores |
//! | what names the object | a client-chosen opaque key | `sha256(domain ‖ tag ‖ seq)` |
//! | lifetime | a photo's | a mailbox entry's |
//!
//! Media offload exists so a 25 MB video never enters the relay. This exists for
//! the opposite reason: a mailbox body is small, it is already in the process
//! (it arrived in a `Pub` frame), and the question is only where it comes to rest.
//! The presigner is shared because presigning is a pure function; nothing else is.
//!
//! ## Why the arc presigns its own requests
//!
//! It has the credentials, so it could sign an `Authorization` header instead. It
//! presigns because `sigv4.rs` already does exactly that, is tested against AWS's
//! own published vector, and produces a URL a plain HTTP client can use with no
//! auth header at all. One signer, one set of tests.
//!
//! ## Keys leak nothing the bucket did not already have
//!
//! An object is named `sha256("pacific/relay/blob/v1" ‖ len(tag) ‖ tag ‖ seq)`,
//! rendered as 64 lowercase hex — deliberately the same SHAPE `media.rs` validates,
//! for the same reason: the bucket operator must not be able to read group
//! structure out of a listing. Anyone who already knows a tag can compute its
//! keys, which is no new capability: a tag is the routing capability, and holding
//! one already lets you subscribe.
//!
//! ## What this module does NOT promise
//!
//! Nothing here arbitrates anything. `put` is last-writer-wins, `delete` is
//! idempotent, and there is no compare-and-set. That is not an oversight — it is
//! the honest surface of object storage, and it is why the commit slot lives in
//! the index and not here. See `offload.rs`.

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use sha2::{Digest, Sha256};

use crate::sigv4::{presign, Credentials, Presign};

/// Domain separator for object keys. Present so a key can never collide with a
/// hash computed for another purpose over the same bytes.
const KEY_DOMAIN: &[u8] = b"pacific/relay/blob/v1";

/// How long a self-issued presigned URL is good for. Short: the relay redeems it
/// in the same function that mints it, and the only reason it is not seconds is to
/// absorb a slow TCP handshake.
const SELF_URL_TTL: Duration = Duration::from_secs(60);

/// Wall-clock ceiling on one object-storage request. Deliberately tight: this call
/// happens while the hub mutex is held, so a hung request is not one slow publish,
/// it is a stalled relay. Failing fast and refusing to Ack beats blocking.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// The object name for one mailbox entry. Injective in `(tag, seq)`: the tag's
/// length is hashed before the tag, so no two distinct pairs can produce the same
/// preimage by re-splitting the same bytes.
pub fn body_key(tag: &str, seq: u64) -> String {
    let mut h = Sha256::new();
    h.update(KEY_DOMAIN);
    h.update((tag.len() as u64).to_be_bytes());
    h.update(tag.as_bytes());
    h.update(seq.to_be_bytes());
    hex::encode(h.finalize())
}

/// A store for blob bodies, addressed by opaque key.
///
/// Three operations, no transaction, no ordering guarantee between them. Anything
/// that needs to be atomic has to live somewhere else.
#[async_trait]
pub trait Blobs: Send + Sync {
    /// Store `body` at `key`, overwriting whatever was there. Returns once the
    /// store has acknowledged the write, never before.
    async fn put(&self, key: &str, body: &[u8]) -> std::result::Result<(), String>;

    /// Fetch `key`. `Ok(None)` is a definite "the store does not have this object"
    /// — distinct from `Err`, which is "the store did not answer". The caller
    /// treats the first as corruption and the second as transient, so collapsing
    /// them would hide a real hole behind a retry.
    async fn get(&self, key: &str) -> std::result::Result<Option<Vec<u8>>, String>;

    /// Remove `key`. Idempotent: deleting an absent object is success.
    async fn delete(&self, key: &str) -> std::result::Result<(), String>;

    /// One line naming this store for the boot log. Must not carry credentials.
    fn describe(&self) -> String;
}

/// S3-compatible object storage (Cloudflare R2), reached over HTTP with presigned
/// URLs this process mints for itself.
pub struct R2Blobs {
    /// `https` in production. `http` exists only for an S3-compatible endpoint on
    /// the local network (MinIO, or the stand-in server the tests run) — see
    /// [`R2Blobs::from_endpoint`], which refuses to be quiet about it.
    scheme: String,
    /// Host, with port if the endpoint carried one. This is the exact value that
    /// goes into the signature AND into the `Host` header, and they have to agree
    /// or the store answers 403.
    host: String,
    bucket: String,
    region: String,
    creds: Credentials,
    client: reqwest::Client,
}

impl R2Blobs {
    /// Build from an endpoint URL (`https://acct.r2.cloudflarestorage.com`), a
    /// bucket, a region (`auto` for R2) and credentials.
    ///
    /// An `http://` endpoint is accepted and WARNED ABOUT: a presigned URL is a
    /// bearer capability, and over plain HTTP anyone on the path can lift it. It is
    /// supported because a local S3-compatible endpoint is a real deployment and
    /// the alternative — a test that cannot exercise the real client — is worse.
    pub fn from_endpoint(
        endpoint: &str,
        bucket: impl Into<String>,
        region: impl Into<String>,
        creds: Credentials,
    ) -> std::result::Result<Self, String> {
        let endpoint = endpoint.trim();
        let (scheme, rest) = match endpoint.split_once("://") {
            Some(("http", rest)) => {
                tracing::warn!(
                    "object storage endpoint is plain HTTP — presigned URLs are bearer \
                     capabilities and are exposed in transit. Use https unless this is a \
                     local S3-compatible endpoint."
                );
                ("http", rest)
            }
            Some(("https", rest)) => ("https", rest),
            Some((other, _)) => return Err(format!("unsupported endpoint scheme: {other}")),
            None => ("https", endpoint),
        };
        let host = rest.trim_end_matches('/').to_string();
        if host.is_empty() {
            return Err("endpoint has no host".into());
        }
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| format!("could not build HTTP client: {e}"))?;
        Ok(Self {
            scheme: scheme.to_string(),
            host,
            bucket: bucket.into(),
            region: region.into(),
            creds,
            client,
        })
    }

    /// Presign one request against `bucket/key`.
    ///
    /// `sigv4::presign` always renders `https://`, because that is the only scheme
    /// production ever uses and giving the signer a scheme parameter would mean
    /// touching every existing call site for one local case. The scheme is NOT part
    /// of a SigV4 signature — the signed elements are method, canonical URI,
    /// canonical query, headers (including `host`) and the payload sentinel — so
    /// rewriting it afterwards changes the transport and not the credential.
    fn url(&self, method: &str, key: &str, signed_headers: &[(String, String)]) -> String {
        let signed = presign(
            &Presign {
                method,
                host: &self.host,
                path_segments: &[&self.bucket, key],
                region: &self.region,
                expires_in: SELF_URL_TTL.as_secs() as u32,
                signed_headers,
                query: &[],
                now: Utc::now(),
            },
            &self.creds,
        );
        match signed.strip_prefix("https://") {
            Some(rest) => format!("{}://{rest}", self.scheme),
            None => signed,
        }
    }
}

#[async_trait]
impl Blobs for R2Blobs {
    async fn put(&self, key: &str, body: &[u8]) -> std::result::Result<(), String> {
        let len = body.len() as u64;
        // `content-length` is folded into the signature for the same reason
        // `media.rs` folds it: it turns the URL from a blank cheque into an
        // authorisation for exactly these bytes.
        let url = self.url("PUT", key, &[("content-length".into(), len.to_string())]);
        let resp = self
            .client
            .put(url)
            .header("content-length", len.to_string())
            .body(body.to_vec())
            .send()
            .await
            .map_err(describe_transport)?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            // The status only. A body from the object store can quote the key back.
            Err(format!("PUT returned HTTP {}", status.as_u16()))
        }
    }

    async fn get(&self, key: &str) -> std::result::Result<Option<Vec<u8>>, String> {
        let url = self.url("GET", key, &[]);
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(describe_transport)?;
        let status = resp.status();
        if status.as_u16() == 404 {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(format!("GET returned HTTP {}", status.as_u16()));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|_| "GET body was unreadable".to_string())?;
        Ok(Some(bytes.to_vec()))
    }

    async fn delete(&self, key: &str) -> std::result::Result<(), String> {
        let url = self.url("DELETE", key, &[]);
        let resp = self
            .client
            .delete(url)
            .send()
            .await
            .map_err(describe_transport)?;
        let status = resp.status();
        // 404 is success: this operation's promise is "the object is not there",
        // and an already-absent object satisfies it.
        if status.is_success() || status.as_u16() == 404 {
            Ok(())
        } else {
            Err(format!("DELETE returned HTTP {}", status.as_u16()))
        }
    }

    fn describe(&self) -> String {
        // Scheme only — no host, no bucket, no key id. The boot line says WHAT the
        // backend is; it is not a place to print configuration.
        format!("S3-compatible object storage over {}", self.scheme)
    }
}

/// Reduce a transport failure to a short, stable phrase. Never the URL: a presigned
/// URL carries a signature, and `reqwest`'s own Display includes the URL it tried.
fn describe_transport(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "request timed out".into()
    } else if e.is_connect() {
        "object store unreachable".into()
    } else {
        "request failed".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds() -> Credentials {
        Credentials {
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
        }
    }

    /// A key must look exactly like the opaque keys `media.rs` already validates,
    /// so a listing of the bucket cannot be read as group structure.
    #[test]
    fn keys_are_sixty_four_lowercase_hex() {
        let k = body_key("aa", 17);
        assert_eq!(k.len(), 64);
        assert!(k
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    }

    /// Two different mailbox entries must never share an object, or one overwrites
    /// the other and a blob the publisher was acked for disappears.
    #[test]
    fn keys_are_injective_in_tag_and_seq() {
        assert_ne!(body_key("aa", 1), body_key("aa", 2));
        assert_ne!(body_key("aa", 1), body_key("bb", 1));
        // The length prefix is what stops a re-split of the same bytes colliding:
        // without it, ("a","ab"+seq) and ("aa","b"+seq) would hash the same run.
        assert_ne!(body_key("ab", 1), body_key("a", 1));
        assert_eq!(body_key("aa", 1), body_key("aa", 1), "and it is a function");
    }

    #[test]
    fn an_endpoint_without_a_scheme_is_https() {
        let b = R2Blobs::from_endpoint("acct.r2.cloudflarestorage.com", "x", "auto", creds())
            .unwrap();
        assert!(b.url("GET", "k", &[]).starts_with("https://"));
    }

    /// The scheme rewrite must change the transport and nothing else: the signature
    /// is over method/URI/query/headers, so an http and an https URL for the same
    /// request differ ONLY in those eight characters.
    #[test]
    fn rewriting_the_scheme_leaves_the_signature_alone() {
        let https =
            R2Blobs::from_endpoint("https://localhost:9000", "b", "auto", creds()).unwrap();
        let http = R2Blobs::from_endpoint("http://localhost:9000", "b", "auto", creds()).unwrap();
        let a = https.url("GET", "k", &[]);
        let b = http.url("GET", "k", &[]);
        assert!(b.starts_with("http://localhost:9000/b/k?"));
        assert_eq!(
            a.strip_prefix("https://"),
            b.strip_prefix("http://"),
            "the scheme is not a signed element and must not perturb the URL"
        );
    }

    #[test]
    fn a_nonsense_scheme_is_refused_rather_than_guessed() {
        assert!(R2Blobs::from_endpoint("ftp://x", "b", "auto", creds()).is_err());
        assert!(R2Blobs::from_endpoint("https://", "b", "auto", creds()).is_err());
    }

    /// Different verbs must not produce the same capability.
    #[test]
    fn each_verb_gets_its_own_url() {
        let b = R2Blobs::from_endpoint("https://h", "b", "auto", creds()).unwrap();
        let put = b.url("PUT", "k", &[("content-length".into(), "3".into())]);
        let get = b.url("GET", "k", &[]);
        assert_ne!(put, get);
        assert!(put.contains("content-length"), "the size bound must be signed");
    }

    #[test]
    fn credentials_never_reach_the_boot_line() {
        let b = R2Blobs::from_endpoint("https://acct.example", "secret-bucket", "auto", creds())
            .unwrap();
        let d = b.describe();
        assert!(!d.contains("AKIA"), "{d}");
        assert!(!d.contains("secret-bucket"), "{d}");
    }
}
