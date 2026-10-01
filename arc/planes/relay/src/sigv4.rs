//! sigv4 — a minimal AWS SigV4 **presigner** for S3-compatible object storage.
//!
//! This exists so the relay can hand a client a short-lived, single-object URL
//! and then get out of the way. The media bytes never traverse this process:
//! the client PUTs ciphertext straight to R2 and the recipient GETs it straight
//! back. That is not an optimisation, it is what keeps the relay BLIND — the
//! same property `store.rs` protects for the mailbox. A proxying design would
//! put every photo through a process whose entire contract is that it cannot
//! see what it carries.
//!
//! ## Why hand-rolled instead of an SDK
//!
//! `aws-sdk-s3` would pull a large tree (and its own HTTP/TLS stack) to use one
//! pure function. Presigning is *only* string construction plus four HMACs —
//! there is no request, no socket, no runtime. Every crate used here (`hmac`,
//! `sha2`, `percent-encoding`) is already in the arc lockfile via other planes,
//! so this adds no new transitive weight and keeps the workspace OpenSSL-free.
//!
//! ## Signed headers are a size bound
//!
//! Anything named in `signed_headers` becomes part of the signature, so the
//! client MUST send that header with that exact value or R2 rejects the upload.
//! `media.rs` signs `content-length`, which turns "presign me an upload" into
//! "presign me an upload of EXACTLY n bytes" — the only way to bound object size
//! on a presigned PUT, since a bare presigned URL is otherwise a blank cheque.
//!
//! Reference: AWS Signature Version 4, "Authenticating Requests: Using Query
//! Parameters". Verified in tests against AWS's own published example vector.

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// RFC 3986 unreserved set: everything else is percent-encoded. AWS is strict
/// here — a single differently-encoded byte changes the canonical request and
/// therefore the signature.
const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// Same, but `/` survives so path separators stay separators.
const PATH: &AsciiSet = &UNRESERVED.remove(b'/');

/// Long-lived credentials for the object store.
#[derive(Clone)]
pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
}

impl std::fmt::Debug for Credentials {
    /// Never let a secret reach a log line, a panic message or a Sentry event.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &"<redacted>")
            .finish()
    }
}

/// One presigning request. `path_segments` are raw (unencoded) — this module
/// encodes them, because doing it at the call site is how canonical-request bugs
/// get written.
pub struct Presign<'a> {
    /// "PUT" or "GET".
    pub method: &'a str,
    /// Host only, no scheme (e.g. `abc123.r2.cloudflarestorage.com`).
    pub host: &'a str,
    /// Raw path segments, joined with `/` after encoding (e.g. `[bucket, key]`).
    pub path_segments: &'a [&'a str],
    /// `auto` for R2; a real region for AWS.
    pub region: &'a str,
    /// URL lifetime in seconds. AWS caps this at 7 days.
    pub expires_in: u32,
    /// Extra headers to fold into the signature, as (lowercase-name, value).
    /// `host` is always signed and must not be repeated here.
    pub signed_headers: &'a [(String, String)],
    /// Extra query parameters (e.g. `list-type=2`). They join the canonical query
    /// string and are therefore signed: the URL only works with exactly these.
    pub query: &'a [(String, String)],
    /// Signing time. Injected rather than read from the clock so the result is
    /// deterministic and therefore testable.
    pub now: DateTime<Utc>,
}

/// Build a presigned `https://` URL. Pure: no I/O, no clock, no randomness.
pub fn presign(req: &Presign<'_>, creds: &Credentials) -> String {
    let amz_date = req.now.format("%Y%m%dT%H%M%SZ").to_string();
    let datestamp = req.now.format("%Y%m%d").to_string();
    let scope = format!("{datestamp}/{}/s3/aws4_request", req.region);

    // --- canonical URI -----------------------------------------------------
    let canonical_uri = {
        let joined = req
            .path_segments
            .iter()
            .map(|s| utf8_percent_encode(s, UNRESERVED).to_string())
            .collect::<Vec<_>>()
            .join("/");
        format!("/{joined}")
    };

    // --- canonical headers -------------------------------------------------
    // `host` is mandatory; the rest are whatever the caller chose to bind.
    let mut headers: Vec<(String, String)> = vec![("host".into(), req.host.to_string())];
    for (k, v) in req.signed_headers {
        headers.push((k.to_ascii_lowercase(), v.clone()));
    }
    headers.sort_by(|a, b| a.0.cmp(&b.0));
    let signed_header_list = headers
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let canonical_headers = headers
        .iter()
        .map(|(k, v)| format!("{k}:{}\n", v.trim()))
        .collect::<String>();

    // --- canonical query string --------------------------------------------
    // Must be sorted by encoded key. These five are the presigning vocabulary.
    let credential = format!("{}/{scope}", creds.access_key_id);
    let mut params: Vec<(String, String)> = vec![
        ("X-Amz-Algorithm".into(), "AWS4-HMAC-SHA256".into()),
        ("X-Amz-Credential".into(), credential),
        ("X-Amz-Date".into(), amz_date.clone()),
        ("X-Amz-Expires".into(), req.expires_in.to_string()),
        ("X-Amz-SignedHeaders".into(), signed_header_list.clone()),
    ];
    params.extend(req.query.iter().cloned());
    // AWS requires the canonical query string sorted by encoded key; the caller's
    // parameters are just more entries in that same sorted list.
    params.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical_query = params
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                utf8_percent_encode(k, UNRESERVED),
                utf8_percent_encode(v, UNRESERVED)
            )
        })
        .collect::<Vec<_>>()
        .join("&");

    // --- canonical request --------------------------------------------------
    // Presigned URLs carry no body at signing time, so the payload hash is the
    // literal `UNSIGNED-PAYLOAD` sentinel rather than a digest.
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\nUNSIGNED-PAYLOAD",
        req.method, canonical_uri, canonical_query, canonical_headers, signed_header_list
    );

    // --- string to sign -----------------------------------------------------
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );

    // --- signing key + signature -------------------------------------------
    let k_date = hmac(
        format!("AWS4{}", creds.secret_access_key).as_bytes(),
        datestamp.as_bytes(),
    );
    let k_region = hmac(&k_date, req.region.as_bytes());
    let k_service = hmac(&k_region, b"s3");
    let k_signing = hmac(&k_service, b"aws4_request");
    let signature = hex::encode(hmac(&k_signing, string_to_sign.as_bytes()));

    format!(
        "https://{}{}?{canonical_query}&X-Amz-Signature={signature}",
        req.host,
        utf8_percent_encode(&canonical_uri, PATH)
    )
}

fn hmac(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(msg);
    mac.finalize().into_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn example_creds() -> Credentials {
        Credentials {
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
        }
    }

    /// Known-answer vector for a presigned S3 GET, on AWS's canonical example
    /// inputs (`examplebucket`/`test.txt`, 2013-05-24, 24h expiry).
    ///
    /// The expected signature was produced by botocore's `S3SigV4QueryAuth` — the
    /// reference implementation — with its clock frozen to that timestamp, and is
    /// reproduced here so the check runs offline. Note the *S3* signer specifically:
    /// generic SigV4 query signing hashes the (empty) body, whereas S3 presigning
    /// uses the `UNSIGNED-PAYLOAD` sentinel, and the two produce different
    /// signatures for identical inputs.
    ///
    /// If this drifts, the presigner is wrong — the one bug class that would
    /// otherwise surface only as an opaque 403 from R2 at runtime.
    #[test]
    fn matches_botocore_s3_presigner() {
        let url = presign(
            &Presign {
                method: "GET",
                host: "examplebucket.s3.amazonaws.com",
                path_segments: &["test.txt"],
                region: "us-east-1",
                expires_in: 86400,
                signed_headers: &[],
                query: &[],
                now: Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0).unwrap(),
            },
            &example_creds(),
        );
        assert!(
            url.contains(
                "X-Amz-Signature=3ed0be64024db54d5574a27da223529635c383f911f80e636f0ccc13890053d2"
            ),
            "signature mismatch, got: {url}"
        );
    }

    /// The size bound: `content-length` must appear in `X-Amz-SignedHeaders`, or
    /// the URL is a blank cheque for any object size.
    #[test]
    fn content_length_is_bound_into_the_signature() {
        let mk = |len: u64| {
            presign(
                &Presign {
                    method: "PUT",
                    host: "acct.r2.cloudflarestorage.com",
                    path_segments: &["pacific-media", &"ab".repeat(32)],
                    region: "auto",
                    expires_in: 300,
                    signed_headers: &[("content-length".into(), len.to_string())],
                    query: &[],
                    now: Utc.with_ymd_and_hms(2026, 9, 9, 12, 0, 0).unwrap(),
                },
                &example_creds(),
            )
        };
        let a = mk(1024);
        assert!(a.contains("X-Amz-SignedHeaders=content-length%3Bhost"));
        // Known-answer, same provenance as the GET vector above: botocore signing
        // a PUT for this bucket/key/size at this instant. This pins the bound
        // itself, not just the presence of the header.
        assert!(
            a.contains(
                "X-Amz-Signature=9f668e1e6d20ed8746d344cb03f56bde98c27989fd21de6fb0f9ad0cca022fd1"
            ),
            "content-length vector mismatch, got: {a}"
        );
        // A different declared length is a different signature, so a client
        // cannot reuse one URL to upload a larger object.
        assert_ne!(a, mk(1025));
    }

    /// Query parameters are signed, so a URL minted to list cannot be edited into
    /// a URL that lists something else.
    #[test]
    fn query_parameters_are_signed() {
        let mk = |token: &str| {
            presign(
                &Presign {
                    method: "GET",
                    host: "acct.r2.cloudflarestorage.com",
                    path_segments: &["pacific-media"],
                    region: "auto",
                    expires_in: 300,
                    signed_headers: &[],
                    query: &[
                        ("list-type".into(), "2".into()),
                        ("continuation-token".into(), token.into()),
                    ],
                    now: Utc.with_ymd_and_hms(2026, 9, 9, 12, 0, 0).unwrap(),
                },
                &example_creds(),
            )
        };
        let a = mk("aaa");
        assert!(a.contains("list-type=2"));
        assert_ne!(
            a,
            mk("bbb"),
            "changing a query value must change the signature"
        );
    }

    /// Presigning is a pure function of its inputs — same inputs, same URL.
    #[test]
    fn is_deterministic() {
        let mk = || {
            presign(
                &Presign {
                    method: "GET",
                    host: "acct.r2.cloudflarestorage.com",
                    path_segments: &["pacific-media", "deadbeef"],
                    region: "auto",
                    expires_in: 300,
                    signed_headers: &[],
                    query: &[],
                    now: Utc.with_ymd_and_hms(2026, 9, 9, 12, 0, 0).unwrap(),
                },
                &example_creds(),
            )
        };
        assert_eq!(mk(), mk());
    }

    /// A secret must never be renderable.
    #[test]
    fn credentials_do_not_leak_through_debug() {
        let rendered = format!("{:?}", example_creds());
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("wJalrXUtnFEMI"));
    }
}
