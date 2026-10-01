//! AN OBJECT THAT WILL NOT FOLD IS NAMED, NOT VANISHED.
//!
//! Every sweep in `node.rs` that lists objects ends a failed fold with `continue`.
//! That isolation is right — one poisoned object must not blank the whole screen —
//! and on its own it is the most dangerous line in the core, because it makes a
//! non-folding object INDISTINGUISHABLE from an object that was never created. The
//! member is shown a shorter list. Nothing is said.
//!
//! This binary pins both halves of the ruling at once:
//!
//!   1. the skip is REAL — a group carrying an op this build does not know drops
//!      out of `group_rows`, exactly as before;
//!   2. and it is no longer SILENT — `noncompliant_objects` names that same group,
//!      with its kind and the reducer's own words for why.
//!
//! Neither assertion means anything without the other. (1) alone is the bug. (2)
//! alone could pass against a sweep that never skipped anything. Together they are
//! the statement the doctrine actually makes: an object is either compliant or
//! non-compliant, and a device can be asked which.
//!
//! THE UNKNOWN OP IS THE HONEST FIXTURE. It is not a corrupted byte or a truncated
//! blob — those are the drain's problem and the drain already has a quarantine for
//! them. This delta decodes perfectly, is signed by a real member, rides a real
//! epoch, and is refused at the REDUCER, which is precisely what a device one
//! version behind receives the day a new op ships. That is the case that has to be
//! visible, because it is the one that will actually happen.

mod common;

use common::Harness;
use pacific_core::coordinator::{Args, Delta};
use pacific_core::object::ObjectKind;
use pacific_core::visibility::Visibility;

/// An op id no table in this build declares. Deliberately far above every
/// catalogue rather than "one past the end", so it cannot become real by accident
/// the next time a kind grows an op.
const AN_OP_FROM_A_LATER_BUILD: u32 = 9_001;

#[tokio::test]
async fn a_group_carrying_an_op_this_build_does_not_know_is_named_rather_than_dropped() {
    let h = Harness::people(&[("ada", &["phone"])]).await;
    let phone = h.device("ada", "phone");

    // TWO groups, because the claim is about TELLING THEM APART. One is left
    // alone and must stay on the screen; the other is poisoned. A test with one
    // object cannot distinguish "the bad one is reported" from "everything is".
    let healthy = h.mint_group(phone);
    h.group_set_profile(phone, &healthy, "Ada's people", "team").await;
    let poisoned = h.mint_group(phone);
    h.group_set_profile(phone, &poisoned, "The reading room", "team").await;

    // Before: both fold, and the node says it can reduce everything it holds.
    let node = h.node(phone);
    assert!(
        node.noncompliant_objects().unwrap().is_empty(),
        "a freshly minted pair must both fold — if this fails the fixture is wrong, \
         not the feature"
    );
    let rows = node.group_rows().unwrap();
    assert!(
        rows.iter().any(|(id, ..)| id == &poisoned) && rows.iter().any(|(id, ..)| id == &healthy),
        "both groups are on the screen before anything is planted"
    );
    assert!(
        node.object_compliance(&poisoned).is_ok(),
        "and the object itself reports compliant"
    );

    // ── the fixture: one delta from a build that is ahead of this one ─────────
    //
    // Appended to the log the way an inbound delta arrives — the SAME table the
    // drain writes, under this member's own key — so nothing about the poison is
    // special except its op id.
    let gid = hex::decode(&poisoned).unwrap();
    let delta = Delta {
        type_id: ObjectKind::Group.type_id() as u32,
        op_id: AN_OP_FROM_A_LATER_BUILD,
        op_version: 1,
        args: Args::new(),
        epoch: 0,
        prev: [0u8; 32],
        // SEQUENCED, not commutative: a commutative delta with no `gen` would be
        // refused for the wrong reason and the test would pass without proving
        // anything about unknown ops.
        seq: Some(9_001),
        gen: None,
        visibility: Visibility::Private,
    };
    let envelope = delta.canonical_bytes();
    let delta_id = delta.id();
    let author = h.id(phone);
    assert!(
        node.dir
            .append_delta(&gid, &delta_id, &author, &envelope, 1_757_000_000_000)
            .unwrap(),
        "the fixture delta must actually land in the log"
    );

    // ── 1 · THE SKIP IS REAL ──────────────────────────────────────────────────
    let rows = node.group_rows().unwrap();
    assert!(
        !rows.iter().any(|(id, ..)| id == &poisoned),
        "the poisoned group is gone from the sweep — this is the behaviour the \
         ruling is ABOUT, and a build where this assertion fails has changed the \
         isolation rule and needs a decision, not a green test"
    );
    assert!(
        rows.iter().any(|(id, ..)| id == &healthy),
        "and the healthy group is untouched — one bad object must not blank the list"
    );

    // ── 2 · AND IT IS NOT SILENT ──────────────────────────────────────────────
    let bad = node.noncompliant_objects().unwrap();
    assert_eq!(
        bad.len(),
        1,
        "exactly the one object we poisoned, and no other: {bad:?}"
    );
    let it = &bad[0];
    assert_eq!(it.object_id, poisoned, "it is named");
    assert_eq!(it.kind, "group", "with the kind that chose the lens");
    assert!(
        it.reason.contains(&AN_OP_FROM_A_LATER_BUILD.to_string()),
        "and the reducer's OWN WORDS, carrying the op id that was refused — a \
         summary would not tell an operator whether this is an old build, a wrong \
         op id, or a corrupt log. got: {}",
        it.reason
    );

    // …and asked about that one object directly, the node gives the same answer.
    // The sweep and the single probe must not be able to disagree.
    let direct = node.object_compliance(&poisoned).unwrap_err().to_string();
    assert_eq!(
        direct, it.reason,
        "one object, one answer, whichever door you ask through"
    );
    assert!(
        node.object_compliance(&healthy).is_ok(),
        "and the healthy group is still compliant"
    );
}

/// THE AUTHORSHIP GATE, SWEPT — no door will write an op its kind does not declare.
///
/// This is the other half of the ruling, and it is the half that keeps the first
/// half honest: `noncompliant_objects` reports a poisoned log, and this is what
/// stops THIS DEVICE being the one that poisoned it. A device that authors an op
/// nobody else can fold has not written a feature; it has written a delta that
/// every other member will refuse, on an append-only log that nothing deletes from.
///
/// There is one door now (`authoring::build`, reached by `mint` and `apply`), so
/// there is one refusal. The sweep outlived the nine it was written for: it is
/// not testing nine `ok_or_else` calls, it is testing that the rule holds for
/// every kind, including the next one.
/// THE TENTH DOOR. `authoring::build` is the one the sweep above was written for:
/// the generic door the browser authors through, which has no directory, no
/// SQLite and no MLS group to lean on — only the log it is handed. It must refuse
/// the op no table declares for EVERY kind it can fold, and say which refusal.
#[test]
fn the_generic_door_refuses_an_undeclared_op_for_every_kind_it_folds() {
    use pacific_core::authoring::{build, Ctx, Refusal};
    let me = [1u8; 32];
    let (members, owners) = ([me], [(0u64, me)]);
    let ctx = Ctx { me, epoch: 0, members: &members, owners: &owners, log: &[], watermark: Some(0) };
    let bad = AN_OP_FROM_A_LATER_BUILD;
    // EVERY KIND THE DOOR WILL WRITE TO, asked of the door rather than listed
    // here: a transcription of `authoring::kinds()` is a second source of truth,
    // and this one rotted the day `contact` stopped being a kind string.
    for kind in pacific_core::authoring::kinds() {
        match build(kind, bad, Args::new(), &ctx) {
            Err(Refusal::NotDeclared { op_id, .. }) => assert_eq!(op_id, bad),
            other => panic!(
                "the generic door on a '{kind}' object gave {other:?} for op {bad}, which no \
                 table in this build declares — it must refuse it as undeclared"
            ),
        }
    }
}

#[tokio::test]
async fn apply_will_not_write_an_op_the_kind_does_not_declare() {
    // ONE DOOR NOW. This swept nine `*_author` methods, which were nine copies of
    // one sequence around nine `T`s; they are `Node::apply`, so the sweep is over
    // KINDS — every kind an object can be, through the one path that writes them.
    let h = Harness::people(&[("ada", &["phone"])]).await;
    let phone = h.device("ada", "phone");
    let node = h.node(phone);

    let bad = AN_OP_FROM_A_LATER_BUILD;
    for kind in [
        "group", "notebook", "note", "forum", "conversation", "thing", "place",
        "event", "post", "project",
    ] {
        let id = node
            .object_new(kind, "swept")
            .unwrap_or_else(|e| panic!("mint a {kind}: {e}"));
        let outcome = node.apply(&id, bad, Args::new()).await;
        assert!(
            outcome.is_err(),
            "apply accepted op {bad} on a '{kind}', which no table in this build \
             declares — a delta nobody else can fold, on a log nothing deletes from"
        );
    }
}

#[tokio::test]
async fn a_sites_host_folds_and_nothing_of_the_site_is_named() {
    // A Host is its own kind since 25 Sep 2026, and `fold_probe` had no arm for it: every
    // Site's Host was reported NON-COMPLIANT ("'host' has no lens in this build").
    let h = Harness::people(&[("ada", &["phone"])]).await;
    let phone = h.device("ada", "phone");
    let site = h.mint_group(phone);
    h.group_set_profile(phone, &site, "Mill Road Allotments", "community").await;
    let host = h.node(phone).host_new(&site, "Mill Road Allotments").await.unwrap();

    let node = h.node(phone);
    node.object_compliance(&host).unwrap_or_else(|e| panic!("the Host does not fold: {e}"));
    let named: Vec<(String, String)> =
        node.noncompliant_objects().unwrap().into_iter().map(|c| (c.kind, c.reason)).collect();
    assert!(named.is_empty(), "a Site and its Host are compliant: {named:?}");
}

#[tokio::test]
async fn a_treasury_and_a_note_fold_and_are_not_named() {
    // NC-66: `fold_probe` had no arm for either, so every Treasury and every Note was reported
    // NON-COMPLIANT ("'treasury' has no lens in this build"), while /v2/graph folded its view.
    let h = Harness::people(&[("ada", &["phone"])]).await;
    let phone = h.device("ada", "phone");
    let node = h.node(phone);
    for kind in ["treasury", "note"] {
        let id = node.object_new(kind, "the fund").unwrap_or_else(|e| panic!("mint a {kind}: {e}"));
        node.object_compliance(&id).unwrap_or_else(|e| panic!("the {kind} does not fold: {e}"));
    }
    let named: Vec<(String, String)> =
        node.noncompliant_objects().unwrap().into_iter().map(|c| (c.kind, c.reason)).collect();
    assert!(named.is_empty(), "a Treasury and a Note are compliant: {named:?}");
}
