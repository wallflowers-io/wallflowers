//! W-98 LINKS: `contact.setLink`, each side's own wish for the link on a Connection (ICD
//! `kinds.contact.ops."contact.setLink"`). Per-author LWW by gen: one side's active 1 alone is
//! an invitation, both active 1 is a link, either side's active 0 withdraws or ends it.
//!
//! The op id is the ICD's, read here, so the test cannot state a second one.

mod common;

use common::Harness;
use pacific_core::authoring::{self, Ctx, Refusal};
use pacific_core::coordinator::{ArgVal, Args, Delta};
use pacific_core::object::{build_delta, DeltaRejection, MemberId, ObjectKind};
use serde_json::{json, Value};

const ADA: MemberId = [1; 32];
const BO: MemberId = [2; 32];
const STRANGER: MemberId = [9; 32];

fn icd() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("the ICD")).expect("the ICD is JSON")
}

/// `contact.setLink`'s op id, from the ICD.
fn op() -> u32 {
    icd()["kinds"]["contact"]["ops"]["contact.setLink"]["op"].as_u64().expect("the ICD declares contact.setLink") as u32
}

fn args(active: i64, gen: u64) -> Args {
    let mut a = Args::new();
    a.insert("active".into(), ArgVal::Int(active));
    a.insert("gen".into(), ArgVal::Int(gen as i64));
    a
}

fn set_link(active: i64, gen: u64) -> Delta {
    build_delta(ObjectKind::Contact, op(), args(active, gen), 0, Some(gen))
}

/// The connection's view as `reader` reads it: its `link`.
fn link(log: &[(MemberId, Delta)], reader: MemberId) -> Value {
    let view = pacific_core::fold::view_of(
        "connection",
        ADA,
        vec![ADA, BO],
        vec![(0, ADA)],
        log.iter().map(|(a, d)| (*a, d.canonical_bytes())),
        Some(&reader),
    )
    .expect("a connection folds");
    serde_json::from_str::<Value>(&view).expect("the view is JSON")["link"].clone()
}

fn ctx<'a>(me: MemberId, log: &'a [(Delta, MemberId)]) -> Ctx<'a> {
    Ctx { me, epoch: 0, members: &[ADA, BO], owners: &[(0, ADA)], log, watermark: Some(0) }
}

/// A roster member's wish folds, through the one write path and in the fold.
#[test]
fn set_link_member_counts() {
    let d = authoring::build("connection", op(), args(1, 0), &ctx(BO, &[])).expect("a member of the connection states their own wish");
    assert_eq!(d.op_id, op());
    assert_eq!(d.args.get("active"), Some(&ArgVal::Int(1)));
    assert_eq!(link(&[(BO, d)], BO), json!({ "mine": 1, "theirs": null, "state": "invited" }));
}

/// A non-member's is refused at the write path, and inert in the fold.
#[test]
fn set_link_non_member_refused() {
    let refused = authoring::build("connection", op(), args(1, 0), &ctx(STRANGER, &[])).unwrap_err();
    assert_eq!(refused, Refusal::Reducer { op_id: op(), why: DeltaRejection::Unauthorized });
    assert_eq!(link(&[(STRANGER, set_link(1, 1))], ADA), json!({ "mine": null, "theirs": null, "state": "none" }));
}

/// Each state, from each side.
#[test]
fn the_link_from_each_side() {
    let none: Vec<(MemberId, Delta)> = vec![];
    assert_eq!(link(&none, ADA), json!({ "mine": null, "theirs": null, "state": "none" }));

    let invited = vec![(ADA, set_link(1, 1))];
    assert_eq!(link(&invited, ADA), json!({ "mine": 1, "theirs": null, "state": "invited" }));
    assert_eq!(link(&invited, BO), json!({ "mine": null, "theirs": 1, "state": "invited_me" }));

    let linked = vec![(ADA, set_link(1, 1)), (BO, set_link(1, 2))];
    assert_eq!(link(&linked, ADA), json!({ "mine": 1, "theirs": 1, "state": "linked" }));
    assert_eq!(link(&linked, BO), json!({ "mine": 1, "theirs": 1, "state": "linked" }));

    // Bo declines Ada's invitation; Ada withdraws hers; Bo ends a link.
    let declined = vec![(ADA, set_link(1, 1)), (BO, set_link(0, 2))];
    assert_eq!(link(&declined, ADA), json!({ "mine": 1, "theirs": 0, "state": "ended" }));
    let withdrawn = vec![(ADA, set_link(1, 1)), (ADA, set_link(0, 2))];
    assert_eq!(link(&withdrawn, BO), json!({ "mine": null, "theirs": 0, "state": "ended" }));
    let ended = vec![(ADA, set_link(1, 1)), (BO, set_link(1, 2)), (BO, set_link(0, 3))];
    assert_eq!(link(&ended, ADA), json!({ "mine": 1, "theirs": 0, "state": "ended" }));
    assert_eq!(link(&ended, BO), json!({ "mine": 0, "theirs": 1, "state": "ended" }));
}

/// Per-author LWW by gen: the log's order does not decide, and one side never overwrites the
/// other's.
#[test]
fn the_highest_gen_of_each_side_wins_in_any_order() {
    let a = vec![(BO, set_link(1, 5)), (BO, set_link(0, 3)), (ADA, set_link(1, 1))];
    let b = vec![(ADA, set_link(1, 1)), (BO, set_link(0, 3)), (BO, set_link(1, 5))];
    for log in [&a, &b] {
        assert_eq!(link(log, ADA), json!({ "mine": 1, "theirs": 1, "state": "linked" }));
    }
    // Bo's 0 at gen 7 ends it however it arrives.
    let mut c = b.clone();
    c.insert(0, (BO, set_link(0, 7)));
    assert_eq!(link(&c, ADA)["state"], "ended");
}

/// `active` is 1 or 0 and nothing else; `gen` is required (the ICD's args).
#[test]
fn a_malformed_wish_is_refused() {
    for bad in [args(2, 0), args(-1, 0), {
        let mut a = Args::new();
        a.insert("active".into(), ArgVal::Text("1".into()));
        a
    }] {
        let refused = authoring::build("connection", op(), bad.clone(), &ctx(ADA, &[])).unwrap_err();
        assert_eq!(refused, Refusal::Reducer { op_id: op(), why: DeltaRejection::MalformedArgs }, "{bad:?}");
    }
    let no_gen = build_delta(ObjectKind::Contact, op(), { let mut a = Args::new(); a.insert("active".into(), ArgVal::Int(1)); a }, 0, Some(1));
    assert_eq!(link(&[(ADA, no_gen)], ADA)["state"], "none");
}

/// The connection's `link`, as device `u` folds it with `peer`.
fn link_on(h: &Harness, u: usize, peer: usize) -> Value {
    let n = h.node(u);
    let (object, _) = n.connection_object(&h.id(peer)).unwrap().expect("a connection");
    serde_json::from_str::<Value>(&n.object_view(&object).unwrap()).unwrap()["link"].clone()
}

/// THE CARD WAITS FOR ITS OWN SIDE (ICD contact.setLink): a side's card is published into a
/// Connection only once that side's own active is 1. The scan is the scanner's; the scanned
/// side's is its accept, a Delta on the connection, so it survives a new device.
#[tokio::test]
async fn a_card_waits_for_its_own_side() {
    let h = Harness::new(&["Ada", "Bo"]).await;
    let (ada, bo) = (0, 1);

    let code = h.node(ada).build_contact_bundle().unwrap();
    h.node(bo).pair_scan(&code).await.unwrap();
    h.node(ada).sync_once().await.unwrap();
    assert_eq!(h.node(ada).publish_profile(true).await.unwrap(), 0, "Ada has not accepted: no card of hers goes out");
    h.node(bo).sync_once().await.unwrap();
    assert!(h.node(bo).peer_profile(&h.id(ada)).unwrap().is_none(), "Ada's card reached Bo before Ada accepted");
    assert!(h.node(ada).peer_profile(&h.id(bo)).unwrap().is_some(), "the scan is Bo's consent, and his card goes with it");
    assert_eq!(link_on(&h, bo, ada), json!({ "mine": 1, "theirs": null, "state": "invited" }));
    assert_eq!(link_on(&h, ada, bo), json!({ "mine": null, "theirs": 1, "state": "invited_me" }));

    h.node(ada).pair_accept(&h.id(bo)).await.unwrap();
    assert_eq!(link_on(&h, ada, bo), json!({ "mine": 1, "theirs": 1, "state": "linked" }));
    h.node(ada).sync_once().await.unwrap();
    h.node(bo).sync_once().await.unwrap();
    assert!(h.node(bo).peer_profile(&h.id(ada)).unwrap().is_some(), "accepted: Ada's card reaches Bo");
    assert_eq!(link_on(&h, bo, ada), json!({ "mine": 1, "theirs": 1, "state": "linked" }));

    // Accepting again writes no second wish.
    h.node(bo).pair_accept(&h.id(ada)).await.unwrap();
    let n = h.node(bo);
    let (object, _) = n.connection_object(&h.id(ada)).unwrap().unwrap();
    let wishes = n.object_log(&object).unwrap().iter().filter(|(op_id, ..)| *op_id == op()).count();
    assert_eq!(wishes, 2, "one wish each");
}
