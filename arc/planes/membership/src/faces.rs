//! WHO HOLDS AN ADDRESS. The claim made at signup does (ruled 22 Sep 2026).
//!
//! A person claims an address on wallflowers.io when they sign up, and binds it to the
//! Host that serves it when they publish — both at the auth service, the second signed
//! by the claimant. The Arc asks that service which Host an address is bound to and
//! serves it from that Host and no other. So a Host asking for an address somebody
//! else claimed is refused, and so is a Host asking for one nobody claimed at all.
//!
//! WHAT THIS REPLACED. The Arc used to arbitrate on its own: the first Host it folded
//! saying a slug kept it, recorded in `face-claims.json` in the state dir. That made an
//! address reserved at signup and an address served by the Arc two different facts,
//! decided by two parties that never talked — a slug held at signup could be served
//! for somebody else's Host (journeys.md, Journey 1, disagreement 4). The file is no
//! longer read or written; one left on disk is inert.
//!
//! WHAT CROSSES. The question names a slug and the answer names a Host's object id —
//! never a person. The auth service's own check route refuses to say who holds an
//! address, and this one does not add it.
//!
//! FAILING CLOSED. An Arc with no authority configured serves nothing and says why. An
//! Arc whose authority stops answering keeps serving what it last confirmed, for
//! [`STALE_OK`], and then stops: a face must not vanish because the auth service
//! restarted, and it must not stay up for ever on an answer nobody can re-check.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a binding is used without asking again. The gateway caches the whole face
/// for ten seconds in front of this, so a page load is one question a minute per address
/// at most.
pub const FRESH: Duration = Duration::from_secs(60);

/// How long any OTHER answer is used — unclaimed, or claimed and not yet bound. Short,
/// because the owner binds their address as the last step of publishing, and a face
/// that stays down for a minute after that looks broken to the person who just did it.
pub const FRESH_REFUSAL: Duration = Duration::from_secs(5);

/// How long a confirmed answer outlives an authority that has stopped answering.
pub const STALE_OK: Duration = Duration::from_secs(15 * 60);

/// What the authority said about one address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Claimed, and bound to this Host (hex object id, lower case).
    Bound(String),
    /// Claimed at signup, and no Host bound yet: reserved, not published.
    Reserved,
    /// Never claimed.
    Unclaimed,
}

/// May `host` serve `slug`, given the authority's answer? `Err(why)` is in the words the
/// Host's own log gets back as a report, so it names the fix where there is one.
pub fn decide(slug: &str, host: &str, answer: &Answer) -> Result<(), String> {
    match answer {
        Answer::Bound(h) if h.eq_ignore_ascii_case(host) => Ok(()),
        Answer::Bound(_) => Err(format!("the address '{slug}' is bound to another Host by the account that claimed it")),
        Answer::Reserved => Err(format!(
            "the address '{slug}' is claimed but bound to no Host yet — its owner binds it when they publish"
        )),
        Answer::Unclaimed => Err(format!(
            "the address '{slug}' was never claimed — claim it at signup on wallflowers.io first"
        )),
    }
}

/// Read the authority's JSON for one address. `status` 404 is `Unclaimed`; a 200 must
/// name the slug it was asked about and a host that is null or 64 hex.
pub fn parse(slug: &str, status: u16, body: &str) -> Result<Answer, String> {
    if status == 404 {
        return Ok(Answer::Unclaimed);
    }
    if status != 200 {
        return Err(format!("the address authority answered {status}"));
    }
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| format!("the address authority sent bad JSON: {e}"))?;
    if v.get("slug").and_then(|s| s.as_str()) != Some(slug) {
        return Err(format!("the address authority answered for a different address than '{slug}'"));
    }
    match v.get("host") {
        Some(serde_json::Value::Null) => Ok(Answer::Reserved),
        Some(serde_json::Value::String(h)) if h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()) => {
            Ok(Answer::Bound(h.to_ascii_lowercase()))
        }
        _ => Err("the address authority named a host that is not an object id".into()),
    }
}

struct Cached {
    at: Instant,
    answer: Answer,
}

static CACHE: Mutex<Option<HashMap<String, Cached>>> = Mutex::new(None);

/// The authority's base URL — the auth service, `…/auth/site/<slug>/host` beneath it.
fn authority() -> Option<String> {
    std::env::var("ARC_SLUG_AUTHORITY")
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
}

fn fetch(base: &str, slug: &str) -> Result<Answer, String> {
    let url = format!("{base}/auth/site/{slug}/host");
    let mut resp = ureq::get(url.as_str())
        .config()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(3)))
        .build()
        .call()
        .map_err(|e| format!("the address authority could not be reached: {e}"))?;
    let status = resp.status().as_u16();
    let body = resp.body_mut().read_to_string().unwrap_or_default();
    parse(slug, status, &body)
}

/// The authority's answer for `slug`: cached while fresh, re-asked when not, and the
/// last confirmed answer while the authority is down — for [`STALE_OK`] and no longer.
pub fn answer(slug: &str) -> Result<Answer, String> {
    let Some(base) = authority() else {
        return Err("this Arc has no address authority (ARC_SLUG_AUTHORITY), so it serves no address".into());
    };
    {
        let cache = CACHE.lock().unwrap();
        if let Some(c) = cache.as_ref().and_then(|m| m.get(slug)) {
            let fresh = if matches!(c.answer, Answer::Bound(_)) { FRESH } else { FRESH_REFUSAL };
            if c.at.elapsed() < fresh {
                return Ok(c.answer.clone());
            }
        }
    }
    match fetch(&base, slug) {
        Ok(a) => {
            let mut cache = CACHE.lock().unwrap();
            cache
                .get_or_insert_with(HashMap::new)
                .insert(slug.to_string(), Cached { at: Instant::now(), answer: a.clone() });
            Ok(a)
        }
        Err(why) => {
            let cache = CACHE.lock().unwrap();
            match cache.as_ref().and_then(|m| m.get(slug)) {
                Some(c) if c.at.elapsed() < STALE_OK => {
                    eprintln!("[faces] {why} — using the answer confirmed {}s ago for '{slug}'", c.at.elapsed().as_secs());
                    Ok(c.answer.clone())
                }
                _ => Err(why),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";
    const B: &str = "b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2";

    #[test]
    fn the_host_the_claimant_bound_serves_and_no_other() {
        let bound = Answer::Bound(A.into());
        assert!(decide("allotments", A, &bound).is_ok());
        let e = decide("allotments", B, &bound).unwrap_err();
        assert!(e.contains("bound to another Host"), "{e}");
    }

    #[test]
    fn a_first_host_no_longer_takes_an_address_nobody_claimed() {
        // The rule this file replaced: the first Host to ask kept the name.
        let e = decide("allotments", A, &Answer::Unclaimed).unwrap_err();
        assert!(e.contains("never claimed"), "{e}");
    }

    #[test]
    fn a_reserved_address_serves_nothing_until_its_owner_binds_it() {
        let e = decide("allotments", A, &Answer::Reserved).unwrap_err();
        assert!(e.contains("bound to no Host yet"), "{e}");
    }

    #[test]
    fn the_authoritys_answers_parse_and_its_mistakes_do_not() {
        assert_eq!(parse("allotments", 404, ""), Ok(Answer::Unclaimed));
        assert_eq!(parse("allotments", 200, r#"{"slug":"allotments","host":null}"#), Ok(Answer::Reserved));
        assert_eq!(
            parse("allotments", 200, &format!(r#"{{"slug":"allotments","host":"{}"}}"#, A.to_uppercase())),
            Ok(Answer::Bound(A.into())),
            "hex is hex, and it is compared lower case"
        );
        assert!(parse("allotments", 200, &format!(r#"{{"slug":"orchard","host":"{A}"}}"#)).is_err(),
                "an answer about a different address is not an answer");
        assert!(parse("allotments", 200, r#"{"slug":"allotments","host":"abc"}"#).is_err());
        assert!(parse("allotments", 200, "not json").is_err());
        assert!(parse("allotments", 500, "").is_err(), "an outage is not 'unclaimed'");
    }

    #[test]
    fn an_arc_with_no_authority_serves_nothing_and_says_why() {
        // Process-global env: this is the only test here that touches it.
        std::env::remove_var("ARC_SLUG_AUTHORITY");
        let e = answer("allotments").unwrap_err();
        assert!(e.contains("ARC_SLUG_AUTHORITY"), "{e}");
    }
}
