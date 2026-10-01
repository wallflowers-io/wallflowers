//! THE KIOSK'S CLAIM TOKEN (A-3, SEC-A1): one visitor's admission to a Site, signed by
//! the kiosk that showed it, spent once, by whoever brings it first.
//!
//! The shape is agreed with the issuer (Software Engineering (Egregore), egregore
//! `egg/kiosk/claim.mjs`): `v1.<payload>.<sig>`, each base64url without padding (RFC
//! 4648 §5). The payload is the UTF-8 JSON `{"k","s","n","iat","exp","c","a"}`; the
//! signature is Ed25519 over the ASCII bytes of `v1.<payload>`. `k` is the first 8 bytes
//! of sha256 of the kiosk's 32-byte public key, as hex; `n` is 16 random bytes; `c` and
//! `a` are for the webapp at landing, and the Add ignores them.
//!
//! This checks a token. Spending it once is the admitter's, where the Add happens.

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// The longest a token may live, iat to exp (TBD-8, open: the issuer's default).
pub const LIFE_MAX: u64 = 15 * 60;
/// How far the kiosk's clock may be from this one. The Pi has no RTC: a kiosk further
/// out has every token refused, visibly.
pub const SKEW: u64 = 5 * 60;
/// What `c` may say.
pub const CHOICES: &[&str] = &["resources", "skills", "financial"];
/// The alphabet the Egregore site draws a share id from (site/lib/commons.ts: lower
/// case and 2–9, without i, l, o, 0 or 1), twenty of them.
pub const SHARE_ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";

/// A share id: twenty characters of SHARE_ALPHABET.
pub fn is_share(a: &str) -> bool {
    a.len() == 20 && a.bytes().all(|b| SHARE_ALPHABET.contains(&b))
}

/// A token that checked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub kid: String,
    /// The Site it admits to, hex.
    pub site: String,
    pub nonce: [u8; 16],
    pub iat: u64,
    pub exp: u64,
    /// The visitor's choice, for the webapp.
    pub choice: Option<String>,
    /// The Artifact's share id on the paid route, for the webapp.
    pub artifact: Option<String>,
}

impl Claim {
    /// What a spent claim is recorded as: never the token, and not the nonce itself.
    pub fn spent_key(&self) -> String {
        hex::encode(Sha256::digest(self.nonce))
    }
}

/// A kiosk's key id: the first 8 bytes of sha256 of its public key, hex.
pub fn kid_of(key: &[u8; 32]) -> String {
    hex::encode(&Sha256::digest(key)[..8])
}

#[derive(serde::Deserialize)]
struct Payload {
    k: String,
    s: String,
    n: String,
    iat: u64,
    exp: u64,
    c: Option<String>,
    a: Option<String>,
}

/// The kiosk keys a deploy trusts, from its file: `{kid: base64url 32-byte key}`. The
/// Arc node and the Door read the same one (Software Security: one list, no drift), until
/// the founders register the key on the Site (`group.setClaimIssuer`). A kid that is not
/// its key's is refused, as the Site's record refuses it.
pub fn parse_keys(json: &str) -> Result<HashMap<String, [u8; 32]>, String> {
    let raw: HashMap<String, String> = serde_json::from_str(json).map_err(|e| format!("not {{kid: key}} JSON: {e}"))?;
    raw.into_iter()
        .map(|(kid, key)| {
            let k: [u8; 32] = B64
                .decode(&key)
                .ok()
                .and_then(|b| b.try_into().ok())
                .ok_or_else(|| format!("{kid} is not a 32-byte base64url key"))?;
            if kid_of(&k) != kid {
                return Err(format!("{kid} is not the id of its key ({})", kid_of(&k)));
            }
            Ok((kid, k))
        })
        .collect()
}

/// [`parse_keys`] of the file at `path`; no path, no keys.
pub fn load_keys(path: Option<&str>) -> Result<HashMap<String, [u8; 32]>, String> {
    match path.filter(|p| !p.trim().is_empty()) {
        None => Ok(HashMap::new()),
        Some(p) => parse_keys(&std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?).map_err(|e| format!("{p}: {e}")),
    }
}

/// The Site a token names, UNCHECKED: which Site's keys to check it against. Nothing
/// may be granted on this alone.
pub fn site_of(token: &str) -> Option<String> {
    let p = token.split('.').nth(1)?;
    let pay: Payload = serde_json::from_slice(&B64.decode(p).ok()?).ok()?;
    Some(pay.s)
}

/// Check `token` against the kiosk keys this admitter trusts, for `site`, at `now`
/// (unix seconds). Every refusal says what failed and never repeats the token.
pub fn verify(token: &str, keys: &HashMap<String, [u8; 32]>, site: &str, now: u64) -> Result<Claim, String> {
    let mut parts = token.split('.');
    let (Some(v), Some(p), Some(s), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err("a claim is v1.<payload>.<sig>".into());
    };
    if v != "v1" {
        return Err(format!("a claim of version {v:?} is not one this admitter reads"));
    }
    let body = B64.decode(p).map_err(|_| "the claim's payload is not base64url")?;
    let pay: Payload = serde_json::from_slice(&body).map_err(|e| format!("the claim's payload is not its JSON: {e}"))?;
    let key = keys.get(&pay.k).ok_or_else(|| format!("no kiosk key {} is trusted here", pay.k))?;
    if kid_of(key) != pay.k {
        return Err("the kiosk key registered under this id is not the one it names".into());
    }
    let sig: [u8; 64] = B64
        .decode(s)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or("the claim's signature is not 64 bytes of base64url")?;
    let vk = ed25519_dalek::VerifyingKey::from_bytes(key).map_err(|_| "the kiosk key is not an Ed25519 key")?;
    vk.verify_strict(format!("v1.{p}").as_bytes(), &ed25519_dalek::Signature::from_bytes(&sig))
        .map_err(|_| "the claim's signature does not verify")?;
    if site.is_empty() || pay.s != site {
        return Err("the claim is not for this Site".into());
    }
    let nonce: [u8; 16] = B64
        .decode(&pay.n)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or("the claim's nonce is not 16 bytes")?;
    if pay.exp <= pay.iat || pay.exp - pay.iat > LIFE_MAX {
        return Err(format!("a claim lives at most {LIFE_MAX} s"));
    }
    if pay.iat > now + SKEW {
        return Err("the claim is dated in the future: the kiosk's clock is wrong".into());
    }
    if now > pay.exp + SKEW {
        return Err("the claim has expired".into());
    }
    if let Some(c) = &pay.c {
        if !CHOICES.contains(&c.as_str()) {
            return Err(format!("the claim's choice {c:?} is not one of {CHOICES:?}"));
        }
    }
    if let Some(a) = &pay.a {
        if !is_share(a) {
            return Err("the claim's artifact is not a 20-character share id".into());
        }
    }
    Ok(Claim { kid: pay.k, site: pay.s, nonce, iat: pay.iat, exp: pay.exp, choice: pay.c, artifact: pay.a })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Signer;

    const SITE: &str = "5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e";

    fn kiosk() -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&[9u8; 32])
    }

    /// As the issuer writes one: the payload's keys in order, no spaces.
    fn issue(payload: &str) -> String {
        let p = B64.encode(payload.as_bytes());
        let sig = kiosk().sign(format!("v1.{p}").as_bytes());
        format!("v1.{p}.{}", B64.encode(sig.to_bytes()))
    }

    fn keys() -> HashMap<String, [u8; 32]> {
        let pk = kiosk().verifying_key().to_bytes();
        HashMap::from([(kid_of(&pk), pk)])
    }

    fn payload(iat: u64, exp: u64) -> String {
        let pk = kiosk().verifying_key().to_bytes();
        format!(
            r#"{{"k":"{}","s":"{SITE}","n":"{}","iat":{iat},"exp":{exp},"c":"skills","a":"k2m3n4p5q6r7s8t9uvwx"}}"#,
            kid_of(&pk),
            B64.encode([7u8; 16])
        )
    }

    /// THE SHARED VECTOR: the kiosk's key from the seed [9; 32], and this payload,
    /// give exactly this token, so the issuer and this checker pin the same bytes.
    #[test]
    fn the_shared_test_vector() {
        let pk = kiosk().verifying_key().to_bytes();
        assert_eq!(kid_of(&pk), "dbc298251c51321b", "kid of the seed-[9;32] key (public {})", B64.encode(pk));
        let token = issue(&payload(1_790_000_000, 1_790_000_900));
        assert_eq!(token, VECTOR, "the vector moved");
        assert_eq!(site_of(&token).as_deref(), Some(SITE));
        let c = verify(&token, &keys(), SITE, 1_790_000_100).expect("the vector verifies");
        assert_eq!(c.spent_key(), hex::encode(Sha256::digest([7u8; 16])));
        assert_eq!((c.choice.as_deref(), c.artifact.as_deref()), (Some("skills"), Some("k2m3n4p5q6r7s8t9uvwx")));
    }

    /// One list for the Arc and the Door: a kid must be its key's.
    #[test]
    fn a_key_list_names_each_key_by_its_own_id() {
        let pk = kiosk().verifying_key().to_bytes();
        let good = format!(r#"{{"{}":"{}"}}"#, kid_of(&pk), B64.encode(pk));
        assert_eq!(parse_keys(&good).unwrap(), keys());
        let misnamed = format!(r#"{{"0000000000000000":"{}"}}"#, B64.encode(pk));
        assert!(parse_keys(&misnamed).unwrap_err().contains("not the id of its key"));
        assert!(parse_keys(r#"{"ab":"short"}"#).unwrap_err().contains("32-byte"));
        assert!(load_keys(None).unwrap().is_empty());
    }

    const VECTOR: &str ="v1.eyJrIjoiZGJjMjk4MjUxYzUxMzIxYiIsInMiOiI1ZTdlNWU3ZTVlN2U1ZTdlNWU3ZTVlN2U1ZTdlNWU3ZTVlN2U1ZTdlNWU3ZTVlN2U1ZTdlNWU3ZTVlN2U1ZTdlIiwibiI6IkJ3Y0hCd2NIQndjSEJ3Y0hCd2NIQnciLCJpYXQiOjE3OTAwMDAwMDAsImV4cCI6MTc5MDAwMDkwMCwiYyI6InNraWxscyIsImEiOiJrMm0zbjRwNXE2cjdzOHQ5dXZ3eCJ9.XHMDkyux2VKE5dbEaoBVZCaHQaYjlb1LrODTz9nk1u5pPfaEH7OOFylQ8IommibeMs7koCG1Ugn4D2W5cKNBAA";

    #[test]
    fn every_refusal_is_named() {
        let now = 1_790_000_100;
        let good = issue(&payload(1_790_000_000, 1_790_000_900));
        assert!(verify(&good, &keys(), SITE, now).is_ok());
        assert!(verify(&good, &keys(), "00", now).unwrap_err().contains("not for this Site"));
        assert!(verify(&good, &HashMap::new(), SITE, now).unwrap_err().contains("no kiosk key"));
        assert!(verify(&good, &keys(), SITE, 1_790_000_900 + SKEW + 1).unwrap_err().contains("expired"));
        assert!(verify(&good, &keys(), SITE, 1_790_000_000 - SKEW - 1).unwrap_err().contains("future"));
        let long = issue(&payload(1_790_000_000, 1_790_000_000 + LIFE_MAX + 1));
        assert!(verify(&long, &keys(), SITE, now).unwrap_err().contains("at most"));
        let (head, sig) = good.rsplit_once('.').unwrap();
        let mut bad = B64.decode(sig).unwrap();
        bad[0] ^= 1;
        assert!(verify(&format!("{head}.{}", B64.encode(bad)), &keys(), SITE, now).unwrap_err().contains("does not verify"));
        let odd = issue(&payload(1_790_000_000, 1_790_000_900).replace("skills", "gold"));
        assert!(verify(&odd, &keys(), SITE, now).unwrap_err().contains("choice"));
        assert!(verify("v2.a.b", &keys(), SITE, now).unwrap_err().contains("version"));
        // A share id as the site makes one; upper case, or a letter it never uses, is not one.
        for (a, ok) in [("k2m3n4p5q6r7s8t9uvwx", true), ("ABCDEFGHIJKLMNOPQRST", false), ("k2m3n4p5q6r7s8t9uvwi", false), ("k2m3", false)] {
            let t = issue(&payload(1_790_000_000, 1_790_000_900).replace("k2m3n4p5q6r7s8t9uvwx", a));
            assert_eq!(verify(&t, &keys(), SITE, now).is_ok(), ok, "share id {a}");
        }
    }
}
