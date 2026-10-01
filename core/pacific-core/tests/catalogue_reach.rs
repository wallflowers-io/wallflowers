//! HOW FAR THE REAL OBJECT CATALOGUE ACTUALLY REACHES — swept, not asserted by hand.
//!
//! `pacific-core/src/icd.rs` already pins the op INVENTORY against
//! `coordination/delta-graph.icd.json` in both directions, and says what it deliberately
//! leaves alone:
//!
//! > What is deliberately NOT asserted: the `x-graph` clauses themselves … This module
//! > pins the INVENTORY — the part that silently drifts — and the `status` field carries
//! > the honest implemented / specified split.
//!
//! This binary asserts the thing that gap leaves open, because reading `status` as a
//! statement about pacific-core is the single easiest mistake to make about this
//! catalogue and it changes what anyone thinks is buildable. The ICD's own preamble:
//!
//! > STATUS. `implemented` means the PROJECTOR does this today. `specified` means this
//! > document is ahead of the code, and the gap is the work list.
//!
//! The projector is `arc-resolver/src/project.rs`, in another crate. So `specified` means
//! "the graph does not mint this edge yet", NOT "the reducer is missing". The sweep below
//! walks all 92 type ops and shows a live reducer arm behind every one of them — including
//! all 45 the document marks `specified`.
//!
//! THE THREE RATIFY OPS ARE SWEPT SEPARATELY (16 Sep 2026), by the test under this one.
//! `ratify.propose`/`ratify.vote`/`ratify.close` are a base op-group `Coordinator::deliver`
//! recognises AHEAD of the type's op table and `Coordinator::ratify_state` folds;
//! `T::reduce` has no arm for them and must not have one, because the domain does not learn
//! about voting. Probing them with `has_reducer_arm` would therefore report a hole that is
//! not a hole, so the question is put to the fold that actually owns them instead. They are
//! on every channel because `deliver` accepts them on every object — which is how they rode
//! the wire through M4 documented nowhere at all.
//!
//! WHAT *IS* MISSING is one rung further out and cannot be read off the ICD at all: an
//! author path on `Node`. The last test names the three ops that have a declaration, a
//! reducer and a read model, and nothing anywhere that emits them.

mod common;

use std::collections::BTreeMap;

use common::Harness;
use pacific_core::coordinator::{Args, ForumType};
use pacific_core::object::{DeltaRejection, MemberId, ObjectType, Op, ReduceContext};
use serde_json::Value;

const OWNER: MemberId = [1u8; 32];
const OTHER: MemberId = [2u8; 32];

fn icd() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../coordination/delta-graph.icd.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("the ICD is readable"))
        .expect("the ICD is valid JSON")
}

/// Every op one KIND declares — its own, and the facet ops it carries — as
/// name -> opId. There is no `status` any more: it was a claim about the
/// arc-resolver projector that read as a claim about core, and the sweep below is
/// stronger without it.
fn ops_of(doc: &Value, kind: &str) -> BTreeMap<String, u32> {
    let mut out = BTreeMap::new();
    let k = doc["kinds"]
        .get(kind)
        .unwrap_or_else(|| panic!("the model declares a kind `{kind}`"));
    for (name, m) in k["ops"].as_object().expect("kind.ops") {
        out.insert(name.clone(), m["op"].as_u64().unwrap() as u32);
    }
    for (_f, facet) in doc["facets"].as_object().expect("facets") {
        if facet["on"].as_array().unwrap().iter().any(|c| c.as_str() == Some(kind)) {
            for (name, m) in facet["ops"].as_object().expect("facet.ops") {
                out.insert(name.clone(), m["op"].as_u64().unwrap() as u32);
            }
        }
    }
    out
}

/// Does `T`'s reducer have an ARM for this op, as opposed to merely a row in its table?
///
/// Run the real reducer against a genesis state with empty args. Anything at all except
/// `UnknownType` means control reached a match arm and the arm parsed — which is the
/// question. `MalformedArgs` is the usual answer and is a PASS: it is the arm saying
/// "these args are empty", which an absent arm could never say.
fn has_reducer_arm<T: ObjectType>(op_id: u32) -> Result<(), DeltaRejection> {
    let members = [OWNER, OTHER];
    let ctx = ReduceContext {
        members: &members,
        owner: OWNER,
        epoch: 1,
    };
    let args = Args::new();
    let mut st = T::State::default();
    match T::reduce(
        &mut st,
        &Op {
            op_id,
            args: &args,
            author: &OWNER,
            pos: None,
            ctx: &ctx,
        },
    ) {
        Err(DeltaRejection::UnknownType) => Err(DeltaRejection::UnknownType),
        _ => Ok(()),
    }
}

/// EVERY op in the ICD — all 88, `implemented` and `specified` alike — has a live reducer
/// arm in pacific-core. The `status` field says nothing about this crate.
#[test]
fn every_specified_op_still_has_a_reducer_in_core() {
    use pacific_core::contact::ContactType;
    use pacific_core::event::EventType;
    use pacific_core::group::GroupType;
    use pacific_core::place::PlaceType;
    use pacific_core::post::PostType;
    use pacific_core::project::ProjectType;
    use pacific_core::thing::ThingType;

    let doc = icd();
    let mut swept = 0usize;
    let mut missing: Vec<String> = Vec::new();

    // `conversation` is deliberately absent: it declares no ops of its own and
    // carries forum's, which are checked on the forum kind.
    for (name, check) in [
        (
            "group",
            &has_reducer_arm::<GroupType> as &dyn Fn(u32) -> Result<(), DeltaRejection>,
        ),
        ("forum", &has_reducer_arm::<ForumType>),
        ("project", &has_reducer_arm::<ProjectType>),
        ("contact", &has_reducer_arm::<ContactType>),
        ("thing", &has_reducer_arm::<ThingType>),
        ("place", &has_reducer_arm::<PlaceType>),
        ("event", &has_reducer_arm::<EventType>),
        ("post", &has_reducer_arm::<PostType>),
    ] {
        for (msg, op_id) in ops_of(&doc, name) {
            // The ratify ops every kind carries are folded by the Coordinator, not
            // by `T::reduce` — the test below asks them the same question at the
            // place that can answer it.
            if pacific_core::coordinator::is_ratify_op(op_id) {
                continue;
            }
            swept += 1;
            if check(op_id).is_err() {
                missing.push(format!("{name}.{msg} (op {op_id})"));
            }
        }
    }

    // EVERY OP THE MODEL DECLARES HAS A REDUCER. That is the whole claim now.
    // It used to be filtered through `x-graph.status` and paired with a pinned
    // `(implemented, specified)` count that broke three times on 24 September
    // alone — once per structural change. The status is gone with the projector's
    // fields, and what is left is the invariant that was always the point.
    assert!(
        missing.is_empty(),
        "these model ops have NO reducer arm in pacific-core, so the document \
         declares what the code cannot fold:\n  {}",
        missing.join("\n  ")
    );
    assert!(swept > 50, "the sweep visited only {swept} ops — it is reading nothing");
}

/// THE RATIFY THREE HAVE A LIVE FOLD — in `Coordinator::ratify_state`, which is where
/// theirs lives. The same question the sweep above asks of every other op in the document,
/// asked of the one op-group that is in no op table.
///
/// `harness/objects/cases.py` recorded the finding this answers: "RATIFY (propose/vote/close,
/// 0xF000-0xF002) is accepted by the Coordinator on EVERY object and appears in NO op table
/// and NO ICD channel — so icd.rs cannot see it and neither can this harness." All three were
/// real and all three were undocumented. They are in the document now (16 Sep 2026), pinned
/// by `icd.rs` against `coordinator::RATIFY_OPS` on every channel; this is the behavioural
/// half — the document says these three exist, and here they are, folding.
#[test]
fn the_three_ratify_ops_fold_through_the_coordinator() {
    use pacific_core::coordinator::{
        ratify_close, ratify_propose, ratify_vote, Ballot, Coordinator, Outcome, Rule,
        FORUM_TYPE_ID, GENESIS_PREV,
    };

    let mut c = Coordinator::<ForumType>::new(vec![OWNER, OTHER], OWNER);
    c.deliver(
        ratify_propose("buy the timber", Rule::Consent, 0, 1, FORUM_TYPE_ID),
        OWNER,
    )
    .expect("propose is accepted — no op table declares it and it is accepted anyway");
    let pid = (OWNER, 0);
    c.deliver(ratify_vote(pid, Ballot::Approve, 1, 1, FORUM_TYPE_ID), OTHER)
        .expect("a member's ballot");

    // Pending until the owner freezes the tally: the outcome is DERIVED from the frozen
    // roster x ballots x rule, never stored, so an unclosed proposal cannot read as agreed.
    assert_eq!(
        c.ratify_state().outcome(&pid, &OWNER),
        Outcome::Pending,
        "a proposal with every ballot in but no close is still a question"
    );

    c.deliver(
        ratify_close(pid, 1, 0, GENESIS_PREV, FORUM_TYPE_ID),
        OWNER,
    )
    .expect("the owner closes on the sequenced spine");

    let rs = c.ratify_state();
    assert_eq!(rs.outcome(&pid, &OWNER), Outcome::Passed);
    assert_eq!(
        rs.ratified(&OWNER),
        vec!["buy the timber".to_string()],
        "the payload the close committed is what `ratified` returns"
    );

    // And the domain never saw any of it: `ForumType::reduce` has no arm for these three,
    // which is why the sweep above must skip them rather than probe them.
    for d in pacific_core::coordinator::RATIFY_OPS.iter() {
        assert!(
            has_reducer_arm::<ForumType>(d.op_id).is_err(),
            "`{}` now has a `T::reduce` arm — if that is deliberate, the domain has started \
             learning about voting and this whole separation needs revisiting",
            d.name
        );
    }
}

/// The same claim, spelled out for the five ops this track was told were out of reach —
/// so the correction is legible without running the sweep.
#[test]
fn the_ops_the_brief_called_spec_only_are_live_reducers() {
    use pacific_core::group::GroupType;
    use pacific_core::thing::{ThingType, OP_CLEAR_POSTURE, OP_SET_POSTURE};
    use pacific_core::wallet;

    // The market half of Thing.
    for op in [OP_SET_POSTURE, OP_CLEAR_POSTURE] {
        assert!(
            ThingType::op(op).is_some(),
            "thing op {op} is not even declared"
        );
        has_reducer_arm::<ThingType>(op).unwrap_or_else(|_| panic!("thing op {op} has no arm"));
    }

    // The whole WALLET facet, which the brief said was spec-only in all four legs.
    // It is live — and since 25 Sep 2026 it is a KIND, not a facet on Group: a
    // Treasury's roster is narrower than the body's on purpose, so a member of the
    // site need not be a member of the money. `wallet::reduce_wallet` is unchanged
    // and `TreasuryType::reduce` routes to it; the op ids are the kind's own, from 0.
    for op in [
        pacific_core::treasury::OP_SET_POLICY,
        pacific_core::treasury::OP_RECORD_DEPOSIT,
        pacific_core::treasury::OP_ATTEST_SETTLEMENT,
        pacific_core::treasury::OP_ATTEST_BALANCE,
    ] {
        assert!(
            pacific_core::treasury::TreasuryType::op(op).is_some(),
            "treasury op {op} is not declared"
        );
        has_reducer_arm::<pacific_core::treasury::TreasuryType>(op)
            .unwrap_or_else(|_| panic!("treasury op {op} has no arm"));
    }
    assert!(
        !wallet::WALLET_OPS.iter().any(|d| GroupType::op(d.op_id).is_some()),
        "a Group must no longer declare the wallet ops — they are Treasury's"
    );
}

/// THE ACTUAL DEAD END, and it is not in the ICD's vocabulary at all: three ops with a
/// declaration, a reducer, a read model and a documented duty — and no way to emit one.
///
/// The ICD is explicit that these are not optional:
///
/// > MEMBERSHIP TRANSITIONS are deltas — base.memberJoined / base.memberLeft … It is NOT
/// > legal for a member to join or leave without emitting one.
///
/// This test used to prove the GAP: `Node::add_member_core` committed the MLS Add and
/// stopped, so a real object with real members held an empty `MembershipLog` and
/// `divergence()` read the whole roster as `unrecorded`. Its own message said what to do
/// when that changed — "find it, and rewrite this test to assert the tenures it records".
///
/// It changed with membership-through-mls.md §10.3: each record is now written by the
/// door that performs the MLS operation, alongside it — the founder's by `object_new`,
/// each arrival by the Add door. So the log and the tree AGREE, and `group_author`
/// refuses to write a record with no operation behind it.
#[tokio::test]
async fn every_membership_transition_is_recorded_by_the_door_that_made_it() {
    use pacific_core::group::GroupType;
    use pacific_core::membership;

    // Declared on GroupType…
    for (op, name) in [
        (membership::OP_MEMBER_JOINED, "base.memberJoined"),
        (membership::OP_MEMBER_LEFT, "base.memberLeft"),
        (membership::OP_OWNER_HANDOVER, "base.ownerHandover"),
    ] {
        assert_eq!(GroupType::op(op).expect("declared").name, name);
        // …and reduced. (`MalformedArgs` on empty args — the arm, parsing.)
        has_reducer_arm::<GroupType>(op).expect("reduced");
    }

    // …and recorded, by the doors. A real object, real members.
    let h = Harness::people(&[("axel", &["phone", "laptop"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");
    let szonja = h.device("szonja", "phone");
    h.pair(phone, szonja).await;

    let stoma = h.mint_group(phone);
    h.add_to_forum(phone, laptop, &stoma).await;
    h.add_to_forum(phone, szonja, &stoma).await;
    h.group_set_profile(phone, &stoma, "STOMA", "organisation")
        .await;
    h.settle().await;

    let roster = h.roster(phone, &stoma);
    assert_eq!(roster.len(), 2);
    let log = h.node(phone).group_state(&stoma).unwrap().membership;
    let axel = hex::encode(h.id(phone));
    let szonja_hex = hex::encode(h.id(szonja));
    assert!(log.is_present(&axel), "the founder's tenure is written at creation");
    assert!(log.is_present(&szonja_hex), "the Add door records the arrival");
    assert_eq!(
        log.tenures(&axel).len(),
        1,
        "a second device of the same person is not a second tenure"
    );
    assert_eq!(log.divergence(&roster), None, "the records and the tree agree");

    // And nothing may write a record without the operation behind it.
    let mut args = pacific_core::coordinator::Args::new();
    args.insert("member".into(), pacific_core::coordinator::ArgVal::Text(szonja_hex.clone()));
    args.insert("at".into(), pacific_core::coordinator::ArgVal::Int(1));
    let refused = h
        .node(phone)
        .apply(&stoma, membership::OP_MEMBER_LEFT, args)
        .await
        .unwrap_err()
        .to_string();
    // The refusal is `Refusal::ByTheMlsDoors`'s own words now: one door, one
    // message, and the browser authoring through `authoring::build` reads the
    // same sentence the native caller does. Matched on the invariant half of the
    // sentence, not the whole of it — a test that pins prose re-breaks on every
    // rewording, which is how this one went red.
    assert!(refused.contains("is a membership record"), "{refused}");
}
