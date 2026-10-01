//! A-3 (Ralph, 27 Sep): "The Arc's always-on node holds a role the founders grant, and
//! adds anyone presenting a valid single-use claim token from the kiosk (SEC-A1)." D-53
//! approved the ops: the `admitter` role, `base.claimSpent`, `group.setClaimIssuer`.
//!
//! A founder makes the Site, puts the Arc's node and a member on it, grants the node the
//! admitter role and registers the kiosk's key. A visitor's claim admits them, once.

mod common;

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use common::Harness;
use ed25519_dalek::Signer;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::object::ObjectKind;
use std::collections::HashMap;

fn kiosk() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[9u8; 32])
}

/// A claim as the kiosk issues one (pacific_core::claim), for `site`, with nonce `n`.
fn claim(site: &str, n: u8) -> String {
    token(site, n, r#","c":"skills""#)
}

/// A claim whose payload ends with `tail`: its `c` and `a`, or nothing.
fn token(site: &str, n: u8, tail: &str) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let pk = kiosk().verifying_key().to_bytes();
    let payload = format!(
        r#"{{"k":"{}","s":"{site}","n":"{}","iat":{now},"exp":{}{tail}}}"#,
        pacific_core::claim::kid_of(&pk),
        B64.encode([n; 16]),
        now + 600
    );
    let p = B64.encode(payload);
    let sig = kiosk().sign(format!("v1.{p}").as_bytes());
    format!("v1.{p}.{}", B64.encode(sig.to_bytes()))
}

fn args(pairs: &[(&str, &str)]) -> Args {
    pairs.iter().map(|(k, v)| (k.to_string(), ArgVal::Text(v.to_string()))).collect()
}

#[tokio::test]
async fn a_claim_admits_its_visitor_once_through_the_admitter() {
    let h = Harness::new(&["ada", "arc", "mal", "vis", "eve"]).await;
    let (ada, arc, mal, vis, eve) = (0, 1, 2, 3, 4);
    let site = h.mint_object(ada, ObjectKind::Group, "Egregore", &[arc, mal]).await;

    // The founder's two acts: the admitter role, and the kiosk's key.
    h.node(ada)
        .apply(&site, pacific_core::roles::OP_SET_ROLE, args(&[("member", &hex::encode(h.id(arc))), ("role", "admitter")]))
        .await
        .expect("the founder grants the admitter role");
    let pk = kiosk().verifying_key().to_bytes();
    h.node(ada)
        .apply(
            &site,
            pacific_core::group::OP_SET_CLAIM_ISSUER,
            args(&[("kid", &pacific_core::claim::kid_of(&pk)), ("key", &B64.encode(pk))]),
        )
        .await
        .expect("the founder registers the kiosk");
    h.settle().await;

    // A member without the role cannot admit, whatever the claim.
    let bundle = h.node(vis).build_contact_bundle().unwrap();
    assert!(
        h.node(mal).admit_by_claim(&site, &claim(&site, 1), std::slice::from_ref(&bundle), &HashMap::new()).await.is_err(),
        "a member who is not an admitter admits no one"
    );

    // The admitter, the visitor's claim, the visitor admitted: exactly their identity.
    let admitted = h
        .node(arc)
        .admit_by_claim(&site, &claim(&site, 2), std::slice::from_ref(&bundle), &HashMap::new())
        .await
        .expect("the admitter admits a valid claim");
    assert_eq!(admitted.member, h.id(vis));
    assert_eq!(admitted.choice.as_deref(), Some("skills"));
    h.settle().await;
    assert_eq!(h.node(vis).object_kind(&site).expect("the visitor holds the Site"), "group");
    let spent = h.node(ada).group_state(&site).unwrap().claims_spent;
    assert_eq!(spent.values().collect::<Vec<_>>(), vec![&h.id(vis)], "the spend is on the Site, naming who it admitted");

    // Once: the same claim, brought by anyone, is refused.
    let other = h.node(eve).build_contact_bundle().unwrap();
    let again = h.node(arc).admit_by_claim(&site, &claim(&site, 2), std::slice::from_ref(&other), &HashMap::new()).await;
    assert!(again.is_err_and(|e| e.to_string().contains("used")), "a claim admits once");

    // A claim for another Site is not this one's.
    assert!(
        h.node(arc).admit_by_claim(&site, &claim(&"ab".repeat(32), 3), std::slice::from_ref(&other), &HashMap::new()).await.is_err(),
        "a claim admits only to the Site it names"
    );
}

/// ICD 2.1.0 row 2: the spend records which message the visitor chose (`c`) and which
/// gift is theirs (`a`), on the Site. A claim without them records neither.
#[tokio::test]
async fn a_spent_claim_records_the_visitors_choice_and_share_on_the_site() {
    let h = Harness::new(&["ada", "arc", "vis", "eve"]).await;
    let (ada, arc, vis, eve) = (0, 1, 2, 3);
    let site = h.mint_object(ada, ObjectKind::Group, "Egregore", &[arc]).await;
    h.node(ada)
        .apply(&site, pacific_core::roles::OP_SET_ROLE, args(&[("member", &hex::encode(h.id(arc))), ("role", "admitter")]))
        .await
        .expect("the founder grants the admitter role");
    let pk = kiosk().verifying_key().to_bytes();
    h.node(ada)
        .apply(
            &site,
            pacific_core::group::OP_SET_CLAIM_ISSUER,
            args(&[("kid", &pacific_core::claim::kid_of(&pk)), ("key", &B64.encode(pk))]),
        )
        .await
        .expect("the founder registers the kiosk");
    h.settle().await;

    let share = "abcdefghjkmnpqrstuvw";
    let paid = token(&site, 1, &format!(r#","c":"financial","a":"{share}""#));
    let bundle = h.node(vis).build_contact_bundle().unwrap();
    h.node(arc).admit_by_claim(&site, &paid, std::slice::from_ref(&bundle), &HashMap::new()).await.expect("admitted");
    let bare = token(&site, 2, "");
    let bundle = h.node(eve).build_contact_bundle().unwrap();
    h.node(arc).admit_by_claim(&site, &bare, std::slice::from_ref(&bundle), &HashMap::new()).await.expect("admitted");
    h.settle().await;

    let st = h.node(ada).group_state(&site).unwrap();
    let of = |who: usize| st.claims_spent.iter().find(|(_, m)| **m == h.id(who)).map(|(k, _)| st.claims_picked.get(k)).expect("spent");
    let v = of(vis).expect("the paid claim's pick");
    assert_eq!((v.choice.as_deref(), v.share.as_deref()), (Some("financial"), Some(share)), "the choice and the gift, on the Site");
    assert_eq!(of(eve), None, "a claim without them records neither");
}

/// W-98 MEMBERS (Ralph, 30 Sep: "Their choice of community will be used to publish their
/// intention on the W-98 Members page"; BUILD: one field on the Site's view). The Site's view
/// carries, per spent claim that chose, its member and the choice: `intentions`,
/// `[[member, choice]]`. Never the claim, and never the share, which is the gift's; a claim
/// with a share and no choice, or with neither, says nothing.
#[tokio::test]
async fn the_sites_view_carries_each_members_intention_and_never_the_claim_or_the_share() {
    let h = Harness::new(&["ada", "arc", "vis", "eve", "bob"]).await;
    let (ada, arc, vis, eve, bob) = (0, 1, 2, 3, 4);
    let site = h.mint_object(ada, ObjectKind::Group, "Egregore", &[arc]).await;
    h.node(ada)
        .apply(&site, pacific_core::roles::OP_SET_ROLE, args(&[("member", &hex::encode(h.id(arc))), ("role", "admitter")]))
        .await
        .expect("the founder grants the admitter role");
    let pk = kiosk().verifying_key().to_bytes();
    h.node(ada)
        .apply(
            &site,
            pacific_core::group::OP_SET_CLAIM_ISSUER,
            args(&[("kid", &pacific_core::claim::kid_of(&pk)), ("key", &B64.encode(pk))]),
        )
        .await
        .expect("the founder registers the kiosk");
    h.settle().await;

    let share = "abcdefghjkmnpqrstuvw";
    for (who, n, tail) in [
        (vis, 1, format!(r#","c":"financial","a":"{share}""#)),
        (bob, 2, r#","c":"skills""#.to_string()),
        (eve, 3, String::new()),
    ] {
        let bundle = h.node(who).build_contact_bundle().unwrap();
        h.node(arc).admit_by_claim(&site, &token(&site, n, &tail), std::slice::from_ref(&bundle), &HashMap::new()).await.expect("admitted");
    }
    h.settle().await;

    let view: serde_json::Value = serde_json::from_str(&h.node(ada).object_view(&site).unwrap()).unwrap();
    let mut want = vec![serde_json::json!([hex::encode(h.id(vis)), "financial"]), serde_json::json!([hex::encode(h.id(bob)), "skills"])];
    want.sort_by_key(|p| p.to_string());
    assert_eq!(view["intentions"], serde_json::Value::Array(want), "each chooser's member and choice, in member order");
    let text = view.to_string();
    assert!(!text.contains(share), "never the share");
    for spent in h.node(ada).group_state(&site).unwrap().claims_spent.keys() {
        assert!(!text.contains(spent.as_str()), "never the claim");
    }
}
