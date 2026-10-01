//! Ed25519 request auth for the signed routes.
//!
//! The callers of /listing, /fulfill, /refund and /door are NOT members of the central
//! Arc — the Arc roster is the wrong authority here (design §5 v1.1) — so the plane
//! authenticates them itself against the pks REGISTERED on the listing. The scheme:
//!
//!   canonical string  {method}|{path}|{body_sha256_hex}|{ts_ms}
//!   header            X-Pacific-Sig: {pk_hex}:{ts_ms}:{sig_hex}
//!
//! The signature covers method, exact public path, body digest and timestamp — so a
//! captured header replays onto nothing else, and ±300s bounds how long onto the same
//! thing. WHICH pk may do what is the route's decision (owner for /listing and /refund,
//! owner-or-delegate for /fulfill and /door): this module only proves the caller holds
//! the key they claim.
//!
//! 2B TODO: door routes should accept ANY member of the Event group, which needs the
//! group's fold — until the embedded pacific-core node lands, doors are owner-or-delegate.

use axum::http::HeaderMap;
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

pub const SIG_HEADER: &str = "x-pacific-sig";
/// ±5 minutes, matching the rail webhook fence and the gateway's credential window.
pub const MAX_SKEW_MS: i64 = 300_000;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("missing {SIG_HEADER} header")]
    Missing,
    #[error("malformed {SIG_HEADER} header (want pk_hex:ts_ms:sig_hex)")]
    Malformed,
    #[error("timestamp outside ±300s")]
    Stale,
    #[error("not a valid Ed25519 public key")]
    BadKey,
    #[error("signature does not verify")]
    BadSignature,
}

fn canonical(method: &str, path: &str, body: &[u8], ts_ms: i64) -> String {
    format!(
        "{method}|{path}|{}|{ts_ms}",
        hex::encode(Sha256::digest(body))
    )
}

/// Verify the request's signature and return the SIGNER's pk (hex). Proves key
/// possession only — authorization against the listing's owner/delegate pks is the
/// caller's next line.
pub fn verify(
    headers: &HeaderMap,
    method: &str,
    path: &str,
    body: &[u8],
    now_ms: i64,
) -> Result<String, AuthError> {
    let header = headers
        .get(SIG_HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or(AuthError::Missing)?;
    let mut it = header.trim().splitn(3, ':');
    let (Some(pk_hex), Some(ts), Some(sig_hex)) = (it.next(), it.next(), it.next()) else {
        return Err(AuthError::Malformed);
    };
    let ts_ms: i64 = ts.parse().map_err(|_| AuthError::Malformed)?;
    if (now_ms - ts_ms).abs() > MAX_SKEW_MS {
        return Err(AuthError::Stale);
    }
    let pk_bytes: [u8; 32] = hex::decode(pk_hex)
        .map_err(|_| AuthError::BadKey)?
        .try_into()
        .map_err(|_| AuthError::BadKey)?;
    let pk = VerifyingKey::from_bytes(&pk_bytes).map_err(|_| AuthError::BadKey)?;
    let sig_bytes: [u8; 64] = hex::decode(sig_hex)
        .map_err(|_| AuthError::Malformed)?
        .try_into()
        .map_err(|_| AuthError::Malformed)?;
    let sig = Signature::from_bytes(&sig_bytes);
    pk.verify_strict(canonical(method, path, body, ts_ms).as_bytes(), &sig)
        .map_err(|_| AuthError::BadSignature)?;
    Ok(pk_hex.to_string())
}

/// Mint the header a client sends — used by this crate's tests (and it documents, in
/// code, exactly what the app must sign on-device; the in-repo harness copies it).
#[cfg_attr(not(test), allow(dead_code))]
pub fn sign_header(
    sk: &ed25519_dalek::SigningKey,
    method: &str,
    path: &str,
    body: &[u8],
    ts_ms: i64,
) -> String {
    use ed25519_dalek::Signer;
    let sig = sk.sign(canonical(method, path, body, ts_ms).as_bytes());
    format!(
        "{}:{ts_ms}:{}",
        hex::encode(sk.verifying_key().to_bytes()),
        hex::encode(sig.to_bytes())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    fn sk(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn headers_with(v: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(SIG_HEADER, v.parse().unwrap());
        h
    }

    #[test]
    fn valid_roundtrip() {
        let key = sk(1);
        let now = 1_750_000_000_000;
        let body = br#"{"listing":"l1"}"#;
        let h = headers_with(&sign_header(&key, "POST", "/v1/box/listing", body, now));
        let pk = verify(&h, "POST", "/v1/box/listing", body, now).expect("must verify");
        assert_eq!(pk, hex::encode(key.verifying_key().to_bytes()));
    }

    #[test]
    fn wrong_key_fails() {
        let now = 1_750_000_000_000;
        let body = b"x";
        // Signed by key 1 but claiming key 2's pk: forge the header by hand.
        let sig_part = sign_header(&sk(1), "POST", "/v1/box/listing", body, now)
            .split(':')
            .nth(2)
            .unwrap()
            .to_string();
        let forged = format!(
            "{}:{now}:{sig_part}",
            hex::encode(sk(2).verifying_key().to_bytes())
        );
        assert!(matches!(
            verify(&headers_with(&forged), "POST", "/v1/box/listing", body, now),
            Err(AuthError::BadSignature)
        ));
    }

    #[test]
    fn stale_timestamp_fails() {
        let key = sk(1);
        let now = 1_750_000_000_000;
        let body = b"x";
        let old = now - MAX_SKEW_MS - 1;
        let h = headers_with(&sign_header(&key, "POST", "/v1/box/listing", body, old));
        assert!(matches!(
            verify(&h, "POST", "/v1/box/listing", body, now),
            Err(AuthError::Stale)
        ));
        // Inside the window it passes — the fence was the refusal, not the sig.
        let ok = headers_with(&sign_header(
            &key,
            "POST",
            "/v1/box/listing",
            body,
            now - MAX_SKEW_MS + 5,
        ));
        verify(&ok, "POST", "/v1/box/listing", body, now).expect("inside the window");
    }

    #[test]
    fn tampered_body_fails() {
        let key = sk(1);
        let now = 1_750_000_000_000;
        let h = headers_with(&sign_header(&key, "POST", "/v1/box/listing", b"qty=1", now));
        assert!(matches!(
            verify(&h, "POST", "/v1/box/listing", b"qty=9", now),
            Err(AuthError::BadSignature)
        ));
        // And a tampered PATH fails the same way — the canonical string pins both.
        assert!(matches!(
            verify(&h, "POST", "/v1/box/refund/pi_1", b"qty=1", now),
            Err(AuthError::BadSignature)
        ));
    }

    #[test]
    fn missing_and_malformed_headers_fail() {
        let now = 0;
        assert!(matches!(
            verify(&HeaderMap::new(), "GET", "/x", b"", now),
            Err(AuthError::Missing)
        ));
        assert!(matches!(
            verify(&headers_with("junk"), "GET", "/x", b"", now),
            Err(AuthError::Malformed)
        ));
    }
}
