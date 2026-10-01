//! O-75, CARDS, N = 20 (BUILD, 29 Sep): twenty members each with an icon near the card's
//! 16 KiB, so their cards cannot travel in one bundle (`history::CAP`, one relay blob). The Arc,
//! admitting by claim with the owner offline, sends them in further bundles (row 5's cards
//! variant, `mls.intro.kinds.history.carriage`); the visitor holds every one.

mod common;

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD as B64};
use base64::Engine;
use common::Harness;
use ed25519_dalek::Signer;
use pacific_core::coordinator::{decode_delta, ArgVal, Args};
use pacific_core::group::{ContactCard, GroupShape};
use pacific_core::object::ObjectKind;
use pacific_core::profiles::OP_PUBLISH_PROFILE;
use serde_json::Value;
use std::collections::HashMap;

const N: usize = 20;
const ADA: usize = 0;
const ARC: usize = 1;
const VIS: usize = 2;
/// The members: 3, 4, … 3 + N − 1.
const FIRST: usize = 3;

fn kiosk() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[9u8; 32])
}

fn claim(site: &str, n: u8) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let pk = kiosk().verifying_key().to_bytes();
    let payload = format!(
        r#"{{"k":"{}","s":"{site}","n":"{}","iat":{now},"exp":{},"c":"skills"}}"#,
        pacific_core::claim::kid_of(&pk),
        B64.encode([n; 16]),
        now + 600
    );
    let p = B64.encode(payload);
    format!("v1.{p}.{}", B64.encode(kiosk().sign(format!("v1.{p}").as_bytes()).to_bytes()))
}

fn text(pairs: &[(&str, &str)]) -> Args {
    pairs.iter().map(|(k, v)| (k.to_string(), ArgVal::Text(v.to_string()))).collect()
}

/// `who` names themselves with an icon near the 16 KiB a card holds, on their self record, and
/// the card walk publishes it.
async fn big_card(h: &Harness, who: usize, name: &str, seed: u8) {
    let mut png = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==").unwrap();
    png.extend((0..12_000).map(|i| seed.wrapping_add(i as u8)));
    let photo = ContactCard { photo: STANDARD.encode(png), photo_mime: "image/png".into(), ..Default::default() };
    let n = h.node(who);
    n.set_my_profile(name, GroupShape::Individual, &photo).await.expect("a self record's card");
    assert_eq!(n.my_card().unwrap().1, None, "the icon is within the card");
    n.reconcile_profiles().await.expect("the card walk");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_member_joining_after_twenty_members_with_icons_sees_every_card_over_several_bundles() {
    let names: Vec<String> = ["ada", "arc", "vis"].iter().map(|s| s.to_string()).chain((0..N).map(|i| format!("m{i:02}"))).collect();
    let h = Harness::new(&names.iter().map(String::as_str).collect::<Vec<_>>()).await;
    let members: Vec<usize> = (FIRST..FIRST + N).collect();
    let site = h.mint_object(ADA, ObjectKind::Group, "Egregore", &[&[ARC], members.as_slice()].concat()).await;
    h.node(ADA).apply(&site, pacific_core::roles::OP_SET_ROLE, text(&[("member", &hex::encode(h.id(ARC))), ("role", "admitter")])).await.unwrap();
    h.settle().await;
    for (i, &m) in members.iter().enumerate() {
        big_card(&h, m, &format!("Member {i:02}"), i as u8).await;
    }
    h.settle().await;

    // THE PREMISE: the members' latest cards alone are over what one bundle carries.
    let gid = hex::decode(&site).unwrap();
    let mut latest: HashMap<[u8; 32], (u64, usize)> = HashMap::new();
    for (a, e, s) in h.node(ARC).dir.load_log_signed(&gid).unwrap() {
        let d = decode_delta(&e).unwrap();
        if d.op_id != OP_PUBLISH_PROFILE {
            continue;
        }
        let size = pacific_core::history::Row { envelope: e, author: a, sig: s.expect("signed") }.payload().len();
        let gen = d.gen.unwrap_or(0);
        if latest.get(&a).is_none_or(|(g, _)| gen > *g) {
            latest.insert(a, (gen, size));
        }
    }
    let card_bytes: usize = latest.values().map(|(_, s)| s).sum();
    assert!(card_bytes > pacific_core::history::CAP, "{card_bytes} bytes of cards, over one bundle's {}", pacific_core::history::CAP);

    // The owner offline; the Arc admits the visitor by claim.
    let packages: Vec<String> = (0..2).map(|_| h.node(VIS).build_contact_bundle().unwrap()).collect();
    let pk = kiosk().verifying_key().to_bytes();
    let keys = HashMap::from([(pacific_core::claim::kid_of(&pk), pk)]);
    // Every current member's card the Arc holds, the visitor's own aside: the twenty's, and the
    // owner's and the Arc's, which the card walk published from their self records.
    let theirs: Value = serde_json::from_str(&h.node(ARC).object_view(&site).unwrap()).unwrap();
    let mut cards = theirs["profiles"].as_object().expect("the Arc's cards").clone();
    cards.remove(&hex::encode(h.id(VIS)));
    for (i, &m) in members.iter().enumerate() {
        assert_eq!(cards[&hex::encode(h.id(m))]["name"], format!("Member {i:02}"), "the premise: the Arc holds each member's card");
    }
    let a = h.node(ARC).admit_by_claim(&site, &claim(&site, 1), &packages, &keys).await.expect("admitted by claim");
    let report = a.history.iter().find(|r| r.object == site).expect("the Site's history report");
    assert_eq!((report.cards, report.cards_unsent), (cards.len(), 0), "every current member's card sent: {report:?}");
    assert!(report.card_bundles >= 2, "over several bundles: {report:?}");
    let present: Vec<usize> = (1..FIRST + N).collect();
    h.settle_among(&present).await;

    let v: Value = serde_json::from_str(&h.node(VIS).object_view(&site).unwrap()).unwrap();
    let missing: Vec<&String> = cards.iter().filter(|(k, c)| &v["profiles"][k.as_str()] != *c).map(|(k, _)| k).collect();
    assert!(missing.is_empty(), "every current member's card, as the Arc holds it; missing {} of {}: {missing:?}", missing.len(), cards.len());
    h.node(VIS).object_compliance(&site).expect("everything the visitor holds folds");
}
