//! W-98: performs_at, the PERFORMER's half. The ICD's group.setAffiliation declares it among
//! `peer`'s rels (target->arg, mint) and in `rel`'s vocabulary: a person's self record or an
//! organisation's group states it performs at the Event `peer`, which is the act's consent.

use pacific_core::authoring::{self, Ctx};
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::group::{AffiliationRel, OP_SET_AFFILIATION};
use serde_json::Value;

const ACT: [u8; 32] = [1; 32];

fn icd() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("the ICD")).expect("the ICD is JSON")
}

/// The word, as the ICD's `rel` vocabulary on group.setAffiliation has it.
fn performs_at() -> String {
    let icd = icd();
    let vocab = icd["kinds"]["group"]["ops"]["group.setAffiliation"]["args"]["rel"]["vocabulary"]
        .as_object()
        .expect("group.setAffiliation's rel vocabulary");
    let word = "performs_at";
    assert!(vocab.contains_key(word), "the ICD's rel vocabulary names {word}");
    word.to_string()
}

#[test]
fn performs_at_parses_as_its_own_rel() {
    let rel = AffiliationRel::parse(&performs_at()).expect("performs_at is a rel");
    assert_eq!(rel.as_str(), performs_at());
    assert!(!rel.answers_for_peer(), "an act answers no one's door at the Event");
}

/// The performer's own group (a self record, an organisation) writes its half, and it folds.
#[test]
fn a_performer_states_it_performs_at_an_event() {
    let event = hex::encode([0xe7u8; 32]);
    let mut a = Args::new();
    a.insert("peer".into(), ArgVal::Text(event.clone()));
    a.insert("rel".into(), ArgVal::Text(performs_at()));
    a.insert("name".into(), ArgVal::Text("Night market".into()));
    a.insert("at".into(), ArgVal::Int(1_758_000_000_000));
    let ctx = Ctx { me: ACT, epoch: 0, members: &[ACT], owners: &[(0, ACT)], log: &[], watermark: Some(0) };
    let d = authoring::build("group", OP_SET_AFFILIATION, a, &ctx).expect("the performer's half is written");

    let view = pacific_core::fold::view_of("group", ACT, vec![ACT], vec![(0, ACT)], [(ACT, d.canonical_bytes())], Some(&ACT))
        .expect("the group folds");
    let v: Value = serde_json::from_str(&view).unwrap();
    let edge = v["affiliations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["peer"] == event.as_str())
        .expect("the edge is on the performer's group");
    assert_eq!(edge["rel"], performs_at().as_str());
}
