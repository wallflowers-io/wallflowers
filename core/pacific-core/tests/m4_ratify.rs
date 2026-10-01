//! RATIFY end-to-end over REAL MLS + the relay, four nodes — the note's family
//! scenario. Sophia OWNS a shared object; Nye, Tom and Lauren are members. Sophia
//! proposes a change to the shared thing; the members vote; Sophia (owner) closes;
//! and ALL FOUR devices converge on the same committed outcome — no server ever
//! holding keys. This is the protocol proof under `run.sh Aries`'s UI demo.

mod common;

use common::Harness;
use pacific_core::coordinator::{Ballot, Rule};

#[tokio::test]
async fn owner_proposes_members_ratify_all_four_converge() {
    let h = Harness::new(&["Sophia", "Nye", "Tom", "Lauren"]).await;
    let (sophia, nye, tom, lauren) = (0, 1, 2, 3);

    // Sophia forms the shared object and owner-adds the other three.
    let obj = h.form_forum(sophia, &[nye, tom, lauren]).await;

    // Sophia proposes a change to the shared thing (consent rule).
    let pid = h
        .obj_propose(sophia, &obj, "blue car -> Wales, Thu-Sun", Rule::Consent)
        .await;
    h.settle().await;

    // Every member sees the pending proposal (it synced to all four phones).
    for u in [sophia, nye, tom, lauren] {
        let rows = h.obj_ratify(u, &obj);
        assert_eq!(rows.len(), 1, "user {u} holds the proposal");
        assert_eq!(rows[0].outcome, "pending", "user {u}: pending until close");
    }

    // The three members each approve.
    h.obj_vote(nye, &obj, pid, Ballot::Approve).await;
    h.obj_vote(tom, &obj, pid, Ballot::Approve).await;
    h.obj_vote(lauren, &obj, pid, Ballot::Approve).await;
    h.settle().await;
    assert_eq!(
        h.obj_ratify(sophia, &obj)[0].outcome,
        "pending",
        "still pending pre-close"
    );

    // Sophia (owner) closes — the single serialization point.
    h.obj_close(sophia, &obj, pid).await;
    h.settle().await;

    // ALL FOUR converge: passed, 4 approvals (Sophia's implicit + three members),
    // the deferred fact committed.
    for u in [sophia, nye, tom, lauren] {
        let rows = h.obj_ratify(u, &obj);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outcome, "passed", "user {u} converged on PASSED");
        assert_eq!(rows[0].approve, 4, "user {u}: 4 approvals");
        assert_eq!(rows[0].reject, 0);
        assert_eq!(rows[0].payload, "blue car -> Wales, Thu-Sun");
    }
}

#[tokio::test]
async fn a_member_veto_fails_the_ratification_for_everyone() {
    let h = Harness::new(&["Sophia", "Nye", "Tom", "Lauren"]).await;
    let (sophia, nye, tom, lauren) = (0, 1, 2, 3);
    let obj = h.form_forum(sophia, &[nye, tom, lauren]).await;

    let pid = h
        .obj_propose(sophia, &obj, "sell the blue car", Rule::Consent)
        .await;
    h.settle().await;

    h.obj_vote(nye, &obj, pid, Ballot::Approve).await;
    h.obj_vote(tom, &obj, pid, Ballot::Reject).await; // Tom vetoes
    h.obj_vote(lauren, &obj, pid, Ballot::Approve).await;
    h.settle().await;

    h.obj_close(sophia, &obj, pid).await;
    h.settle().await;

    for u in [sophia, nye, tom, lauren] {
        let rows = h.obj_ratify(u, &obj);
        assert_eq!(rows[0].outcome, "failed", "user {u} converged on FAILED");
        assert_eq!(rows[0].reject, 1);
    }
}
