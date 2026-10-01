//! Three real users converge on a NESTED thread — the Reddit-style ChatRoom
//! projection over the real relay + MLS. `m4_three_user_group` proves the FLAT
//! transcript converges; this rung proves `ForumState::thread()` — the
//! arbitrary-depth tree the ChatRoom UI binds to — converges too, with a reply
//! nested to depth 2. If this is green, a 3-person threaded ChatRoom works
//! end-to-end: the crypto, the relay, AND the tree fold.

mod common;

use common::Harness;

/// Alice owns a ChatRoom (a forum object), adds Bob and Carol; the three build a
/// nested thread — Bob and Carol each reply to Alice's root, then Alice replies to
/// Bob (depth 2) — and every device folds the identical TREE.
#[tokio::test]
async fn three_user_nested_thread_converges() {
    let h = Harness::new(&["Alice", "Bob", "Carol"]).await;
    let (alice, bob, carol) = (0, 1, 2);

    // Alice owns the ChatRoom; Bob and Carol join as real MLS members.
    let room = h.form_forum(alice, &[bob, carol]).await;

    // Alice opens the thread.
    h.obj_post(alice, &room, "AW26 shoot — kickoff").await;
    h.settle().await;

    // Bob and Carol each reply to the root (depth 1). Each targets the root's
    // MsgRef as THEY folded it — the real "reply to a synced post" path.
    let root_b = h.find_forum(bob, &room, "AW26 shoot — kickoff");
    h.obj_reply(bob, &room, "I'll lock the studio for the 14th", root_b)
        .await;
    h.settle().await;
    let root_c = h.find_forum(carol, &room, "AW26 shoot — kickoff");
    h.obj_reply(carol, &room, "sampling runs late — heads up", root_c)
        .await;
    h.settle().await;

    // Alice replies to BOB's reply — genuine depth-2 nesting.
    let bobs_reply = h.find_forum(alice, &room, "I'll lock the studio for the 14th");
    h.obj_reply(alice, &room, "perfect, booking it", bobs_reply)
        .await;
    h.settle().await;

    // The proof: every device folds a byte-identical thread TREE (order, depth,
    // descendants), not merely the flat transcript.
    let tree = |u: usize| h.with_sync(u, |n| n.object_thread(&room).unwrap());
    let (a, b, c) = (tree(alice), tree(bob), tree(carol));
    assert_eq!(a, b, "Alice and Bob diverged on the thread tree");
    assert_eq!(a, c, "Alice and Carol diverged on the thread tree");
    assert_eq!(
        a.len(),
        4,
        "root + 2 replies + 1 nested reply, every post once"
    );

    // The tree genuinely nests to depth 2, and the root owns all three replies.
    let root = &a[0];
    assert_eq!(root.depth, 0);
    assert_eq!(root.msg.text, "AW26 shoot — kickoff");
    assert_eq!(root.descendants, 3, "root sees all three descendants");

    let deep_idx = a
        .iter()
        .position(|n| n.depth == 2)
        .expect("a reply nested to depth 2");
    assert_eq!(a[deep_idx].msg.text, "perfect, booking it");
    assert_eq!(
        a[deep_idx].msg.author,
        h.id(alice),
        "depth-2 reply is Alice's"
    );
    // pre-order places a node immediately after its parent, so depth-2's parent is
    // the preceding depth-1 row — Bob's reply.
    assert_eq!(a[deep_idx - 1].depth, 1);
    assert_eq!(
        a[deep_idx - 1].msg.author,
        h.id(bob),
        "nested under Bob's reply"
    );
}
