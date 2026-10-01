//! O-75, CARDS, THE TOTAL (row 5's cards variant, `mls.intro.kinds.history.cardsTotal`, 2 MiB):
//! the card bundles for one object at one admission, together. At the total the newest cards
//! arrive and the Arc names how many did not (`HistorySent::cards_unsent`).
//!
//! 2 MiB of cards is about 130 members at the card's 16 KiB, too many real nodes for one test, so
//! the admission here runs under PACIFIC_HISTORY_TOTAL_CAP, which may only lower the ICD's total
//! (BUILD, 29 Sep), as PACIFIC_HISTORY_HOLD_SECS shortens the hold, in this binary alone. The
//! ICD's own total, which the code reads, is held by the first test.

mod common;

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD as B64};
use base64::Engine;
use common::Harness;
use ed25519_dalek::Signer;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::group::{ContactCard, GroupShape};
use pacific_core::object::ObjectKind;
use serde_json::Value;
use std::collections::HashMap;

/// The total a test admission runs under: one card bundle's worth (a bundle is at most
/// `history::CAP`), so of twenty cards near 16 KiB, about twelve go in the first card bundle and
/// the rest, which need a second, do not.
const TEST_TOTAL: usize = 200_000;
const M: usize = 20;
const ADA: usize = 0;
const ARC: usize = 1;
const VIS: usize = 2;
const FIRST: usize = 3;

#[test]
fn the_total_is_the_icds_two_mib() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json");
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(doc["mls"]["intro"]["kinds"]["history"]["cardsTotal"].as_u64(), Some(2 * 1024 * 1024), "mls.intro.kinds.history.cardsTotal");
}

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

/// `who` names themselves with an icon near the card's 16 KiB, and the card walk publishes it.
async fn big_card(h: &Harness, who: usize, name: &str, seed: u8) {
    let mut png = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==").unwrap();
    png.extend((0..12_000).map(|i| seed.wrapping_add(i as u8)));
    let photo = ContactCard { photo: STANDARD.encode(png), photo_mime: "image/png".into(), ..Default::default() };
    let n = h.node(who);
    n.set_my_profile(name, GroupShape::Individual, &photo).await.expect("a self record's card");
    assert_eq!(n.my_card().unwrap().1, None, "the icon is within the card");
    n.reconcile_profiles().await.expect("the card walk");
}

/// M members publish cards one after another, each seeing the last, so each card's gen is newer
/// than the one before. Under a total that holds some of them, the visitor holds only cards newer
/// than every card it lacks, at least one of each; and the Arc's report counts the rest.
#[tokio::test(flavor = "multi_thread")]
async fn at_the_total_the_newest_cards_arrive_and_the_unsent_count_is_named() {
    std::env::set_var("PACIFIC_HISTORY_TOTAL_CAP", TEST_TOTAL.to_string());
    let names: Vec<String> = ["ada", "arc", "vis"].iter().map(|s| s.to_string()).chain((0..M).map(|i| format!("m{i}"))).collect();
    let h = Harness::new(&names.iter().map(String::as_str).collect::<Vec<_>>()).await;
    let members: Vec<usize> = (FIRST..FIRST + M).collect();
    let site = h.mint_object(ADA, ObjectKind::Group, "Egregore", &[&[ARC], members.as_slice()].concat()).await;
    h.node(ADA).apply(&site, pacific_core::roles::OP_SET_ROLE, text(&[("member", &hex::encode(h.id(ARC))), ("role", "admitter")])).await.unwrap();
    h.settle().await;
    for (i, &m) in members.iter().enumerate() {
        big_card(&h, m, &format!("m{i}"), i as u8).await;
        h.settle().await;
    }
    let gen_at = |reader: usize, m: usize| -> Option<u64> {
        let v: Value = serde_json::from_str(&h.node(reader).object_view(&site).unwrap()).unwrap();
        v["profiles"][hex::encode(h.id(m))]["gen"].as_u64()
    };
    let gens: Vec<u64> = members.iter().map(|&m| gen_at(ARC, m).expect("the Arc holds every card")).collect();
    assert!(gens.windows(2).all(|w| w[0] < w[1]), "the premise: each card newer than the last: {gens:?}");
    assert!(M * 16_000 > TEST_TOTAL, "the premise: the cards are over the test's total");

    // Every current member's card the Arc holds, the visitor's own aside: the twenty's, and the
    // owner's and the Arc's (their self records' on entry, the oldest).
    let profiles = |reader: usize| -> serde_json::Map<String, Value> {
        let v: Value = serde_json::from_str(&h.node(reader).object_view(&site).unwrap()).unwrap();
        v["profiles"].as_object().cloned().unwrap_or_default()
    };
    let mut cards = profiles(ARC);
    cards.remove(&hex::encode(h.id(VIS)));
    let gen_of = |k: &str| cards[k]["gen"].as_u64().unwrap();

    let packages: Vec<String> = (0..2).map(|_| h.node(VIS).build_contact_bundle().unwrap()).collect();
    let pk = kiosk().verifying_key().to_bytes();
    let keys = HashMap::from([(pacific_core::claim::kid_of(&pk), pk)]);
    let a = h.node(ARC).admit_by_claim(&site, &claim(&site, 1), &packages, &keys).await.expect("admitted by claim");
    // THE SEND, as the Arc reports it before anything reaches the visitor: every card counted,
    // some sent and some not.
    let report = a.history.iter().find(|r| r.object == site).expect("the Site's history report").clone();
    assert_eq!(report.cards + report.cards_unsent, cards.len(), "every current member's card, sent or counted unsent: {report:?}");
    assert!(report.cards > 0 && report.cards_unsent > 0, "the total holds some and not all: {report:?}");
    h.settle_among(&(1..FIRST + M).collect::<Vec<_>>()).await;

    // THE ARRIVAL: the newest, and exactly as many missing as the Arc named.
    let at_vis = profiles(VIS);
    let (held, missing): (Vec<&String>, Vec<&String>) = cards.keys().partition(|k| at_vis.get(k.as_str()) == cards.get(k.as_str()));
    assert!(!held.is_empty(), "some cards arrive under the total");
    assert!(!missing.is_empty(), "and not all: the total holds");
    let oldest_held = held.iter().map(|k| gen_of(k)).min().unwrap();
    let newest_missing = missing.iter().map(|k| gen_of(k)).max().unwrap();
    assert!(oldest_held > newest_missing, "the newest arrive: held {held:?}, missing {missing:?}");
    assert_eq!(report.cards_unsent, missing.len(), "the unsent count, named in the report: {report:?}");
}
