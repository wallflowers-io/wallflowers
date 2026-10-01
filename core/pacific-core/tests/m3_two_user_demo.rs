//! M3 — the demo rebuilt as real operations between two users.
//!
//! The north star (founder): the demo must be the RESIDUE of real operations
//! through the actual pipe, not data we display. Two independently-minted
//! identities pair over a real relay, then EACH device authors only its own
//! scripted lines (real `author_post` Deltas), and both fold to a byte-identical
//! transcript with every line attributed to its true MLS author. No faked
//! attribution, no hardcoded transcript — if this passes, the chat demo is
//! reconstructable through the system.
//!
//! Now on the shared multi-user `Harness` (real relay, real device boundaries via
//! isolated state dirs); the scripted seed is driven through the `with` hatch.

mod common;

use common::Harness;
use pacific_core::demo::{self, NYE_HARVEY};

#[tokio::test]
async fn chat_demo_is_rebuilt_through_real_ops() {
    let h = Harness::new(&["Nye Thompson", "Harvey Deane"]).await;
    let (nye, harvey) = (0, 1);
    h.pair(nye, harvey).await;

    // Each device authors ONLY its own scripted lines (real Deltas), then syncs.
    let harvey_id = h.id(harvey);
    h.with(nye, |n| async move {
        n.seed_dm_script(&harvey_id, &NYE_HARVEY, "nye")
            .await
            .unwrap();
    })
    .await;
    let nye_id = h.id(nye);
    h.with(harvey, |n| async move {
        n.seed_dm_script(&nye_id, &NYE_HARVEY, "harvey")
            .await
            .unwrap();
    })
    .await;
    h.settle().await;

    // CONVERGENCE: both replicas fold to the identical transcript.
    let view = h.assert_dm_converges(nye, harvey);
    assert_eq!(
        view.len(),
        NYE_HARVEY.lines.len(),
        "every scripted line is present"
    );

    // Each scripted text appears exactly once, carrying its TRUE MLS author — no
    // faked attribution. (Order is the system's real (gen, author) order.)
    for line in NYE_HARVEY.lines {
        assert_eq!(
            view.iter().filter(|m| m.text == line.text).count(),
            1,
            "line present exactly once: {}",
            line.text
        );
        let m = view.iter().find(|m| m.text == line.text).unwrap();
        let expected = if line.actor == "nye" {
            h.id(nye)
        } else {
            h.id(harvey)
        };
        assert_eq!(
            m.author, expected,
            "line '{}' carries its real author",
            line.text
        );
    }

    // The actor-name resolver the app uses to pick its half.
    assert_eq!(demo::actor_for_display_name("Nye Thompson"), Some("nye"));
    assert_eq!(demo::actor_for_display_name("Harvey Deane"), Some("harvey"));
    assert_eq!(demo::actor_for_display_name("Someone Else"), None);
}
