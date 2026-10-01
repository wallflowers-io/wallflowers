//! Regression for the "my reply jumped to the top" ordering bug.
//!
//! Two connected users; A speaks, B reads it, then B replies. B's reply must
//! sort AFTER A's message (causal send order), not above it. The old per-author
//! `gen` counter put B's first post (gen 0) above A's first post (gen 0); the
//! Lamport clock (node.rs::next_lamport) fixes it: B, having SEEN A's gen-0
//! message, stamps its reply gen 1, so it sorts below.

mod common;

use common::Harness;

#[tokio::test]
async fn a_reply_seen_before_sending_sorts_after() {
    let h = Harness::new(&["Nye Thompson", "Harvey Deane"]).await;
    let (a, b) = (0, 1);
    h.pair(a, b).await;

    // A speaks first.
    h.dm_post(a, b, "Hey Nye").await;

    // B SYNCS (sees "Hey Nye"), THEN replies "Hi" — so B's Lamport gen is 1.
    h.sync(b).await;
    assert_eq!(h.dm_view(b, a).len(), 1, "B has seen A's message");
    h.dm_post(b, a, "Hi").await;

    h.settle().await;

    // Both converge, and the reply is BELOW the message it replied to on BOTH.
    let a_view = h.assert_dm_converges(a, b);
    let texts: Vec<&str> = a_view.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(
        texts,
        vec!["Hey Nye", "Hi"],
        "reply sorts after the seen message, not above it"
    );
}
