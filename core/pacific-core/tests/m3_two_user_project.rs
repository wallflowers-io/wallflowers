//! M3 (Project) — the AW26 Work-tab demo rebuilt as real operations between two
//! users. The sibling of `m3_two_user_demo` for a Project GroupObject: Nye mints a
//! real Project, adds Harvey to its MLS group (no prior connection needed — just
//! his bundle), then the OWNER authors the owner-sequenced structure and EACH
//! device authors its own any-member progress. Both fold to a byte-identical
//! board — proving the mockup's Work-tab content is reconstructable through the
//! real pipe, and that the mixed (sequenced + commutative) fold converges across
//! real process boundaries. `blocked` is DERIVED from the tagging→shoot
//! dependency, never asserted.
//!
//! Now on the shared multi-user `Harness`: `form_object` builds the N-member
//! project group; the scripted project seed rides the `with` hatch.

mod common;

use common::Harness;
use pacific_core::demo_project::AW26_PROJECT;
use pacific_core::project::ItemStatus;

#[tokio::test]
async fn project_demo_is_rebuilt_through_real_ops() {
    let h = Harness::new(&["Nye Thompson", "Harvey Deane"]).await;
    let (nye, harvey) = (0, 1);

    // Nye mints the PROJECT and adds Harvey to its group (via his bundle). No prior
    // connection needed — the object path adds any member by key package.
    let project = h.form_object(nye, "project", &[harvey]).await;

    // Sanity: Harvey joined the project group of the same id.
    assert!(
        h.with_sync(harvey, |n| n.objects().unwrap())
            .iter()
            .any(|(id, kind)| id == &project && kind == "project"),
        "Harvey joined the project group of the same id"
    );

    // Nye authors the owner-sequenced structure (+ his own progress), flushes.
    let (harvey_id, p) = (h.id(harvey), project.clone());
    h.with(nye, |n| async move {
        n.seed_project_script(&p, &AW26_PROJECT, "nye", "harvey", &harvey_id)
            .await
            .unwrap();
    })
    .await;
    h.settle().await; // Harvey drains Nye's structure before authoring his progress.

    // Harvey authors his own progress, flushes.
    let (nye_id, p) = (h.id(nye), project.clone());
    h.with(harvey, |n| async move {
        n.seed_project_script(&p, &AW26_PROJECT, "harvey", "nye", &nye_id)
            .await
            .unwrap();
    })
    .await;
    h.settle().await;

    // CONVERGENCE: both replicas fold to the identical board.
    let nye_state = h.with_sync(nye, |n| n.project_state(&project).unwrap());
    let harvey_board = h.with_sync(harvey, |n| n.project_state(&project).unwrap().board());
    assert_eq!(
        nye_state.board(),
        harvey_board,
        "both users converge on the identical board"
    );
    assert_eq!(nye_state.board().len(), 3, "three timeline items rendered");

    // REAL, DERIVED state (not asserted mockup flags).
    let (nye_id, harvey_id) = (h.id(nye), h.id(harvey));
    // tagging: Harvey's real progress, assigned to Harvey, not itself blocked.
    let tagging = nye_state.item_view("tagging").unwrap();
    assert_eq!(
        tagging.status,
        Some(ItemStatus::InProgress),
        "Harvey's progress folded in"
    );
    assert!(
        tagging.assignees.contains(&harvey_id),
        "tagging assigned to Harvey"
    );
    assert!(!tagging.blocked, "tagging itself is not blocked");

    // shoot: DERIVED blocked because tagging isn't done; assigned to Nye.
    let shoot = nye_state.item_view("shoot").unwrap();
    assert!(shoot.blocked, "shoot is DERIVED-blocked (tagging not done)");
    assert!(shoot.assignees.contains(&nye_id), "shoot assigned to Nye");

    // the derived piece count is 0 (no child-Project deliverables in this slice).
    assert_eq!(nye_state.piece_count(), 0);
}
