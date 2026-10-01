//! Three real users — the group scales past two, over the real relay.
//!
//! Unlike m1_nmember (which proves the N-member MLS mechanics IN-PROCESS), this
//! drives three full `Node` devices through the actual pairing/commit/relay path
//! the app uses, and asserts they converge byte-for-byte on the modern-chat
//! surface (posts, reactions, replies). If this is green, a 3-person group works
//! end-to-end — not just the crypto, the whole pipe.

mod common;

use common::Harness;

/// Alice owns a forum, adds Bob and Carol; all three post, one reacts, one
/// replies, and every device folds the identical transcript.
#[tokio::test]
async fn three_member_forum_converges_on_posts_reactions_replies() {
    let h = Harness::new(&["Alice", "Bob", "Carol"]).await;
    let (alice, bob, carol) = (0, 1, 2);

    // Alice owns the group; Bob and Carol are added as real MLS members.
    let obj = h.form_forum(alice, &[bob, carol]).await;

    // All three speak.
    h.obj_post(alice, &obj, "kicking off the Drop 1 sync").await;
    h.settle().await;
    h.obj_post(bob, &obj, "the tags don't match the rail").await;
    h.settle().await;
    h.obj_post(carol, &obj, "on it — pulling the sheet now")
        .await;
    h.settle().await;

    // Bob 👍 Alice's opener; Carol replies to Bob.
    let alice_msg = h.find_forum(bob, &obj, "kicking off the Drop 1 sync");
    h.obj_react(bob, &obj, alice_msg, "👍", true).await;
    let bob_msg = h.find_forum(carol, &obj, "the tags don't match the rail");
    h.obj_reply(carol, &obj, "which SKUs are wrong?", bob_msg)
        .await;
    h.settle().await;

    // Every device agrees, byte-for-byte.
    let view = h.assert_forum_converges(&obj, &[alice, bob, carol]);
    assert_eq!(view.len(), 4, "3 posts + 1 reply");

    // Alice's opener carries exactly Bob's 👍.
    let opener = view
        .iter()
        .find(|m| (m.author, m.gen) == alice_msg)
        .unwrap();
    assert_eq!(opener.reactions.len(), 1, "one emoji group");
    assert_eq!(opener.reactions[0].0, "👍");
    assert_eq!(opener.reactions[0].1, vec![h.id(bob)], "reactor is Bob");

    // Carol's reply links to Bob's message.
    let reply = view.iter().find(|m| m.reply_to == Some(bob_msg)).unwrap();
    assert_eq!(reply.text, "which SKUs are wrong?");
    assert_eq!(reply.author, h.id(carol), "authored by Carol");
}

/// A member added AFTER a conversation started sees FORWARD traffic and converges
/// with the group on it — but does NOT receive pre-join history. MLS gives no keys
/// for epochs you weren't a member of, so the prior deltas (sent to past epoch
/// tags) are unreadable to a newcomer. This is the same semantics as Signal, where
/// new group members don't see old messages. This test PINS that behaviour; full
/// history backfill would be a separate replay feature (flagged to the user, not
/// silently assumed).
#[tokio::test]
async fn late_member_sees_forward_traffic_not_history() {
    let h = Harness::new(&["Alice", "Bob", "Carol"]).await;
    let (alice, bob, carol) = (0, 1, 2);

    // Alice + Bob start a group and exchange messages BEFORE Carol exists in it.
    let obj = h.form_forum(alice, &[bob]).await;
    h.obj_post(alice, &obj, "first").await;
    h.settle().await;
    h.obj_post(bob, &obj, "second").await;
    h.settle().await;
    // The two original members already agree on the full history.
    assert_eq!(h.assert_forum_converges(&obj, &[alice, bob]).len(), 2);

    // Carol joins the EXISTING object late.
    h.add_to_forum(alice, carol, &obj).await;

    // Forward traffic after Carol joins.
    h.obj_post(alice, &obj, "after Carol joined").await;
    h.settle().await;
    h.obj_post(carol, &obj, "Carol here").await;
    h.settle().await;

    let carol_texts: Vec<String> = h
        .obj_view(carol, &obj)
        .iter()
        .map(|m| m.text.clone())
        .collect();
    // Carol SEES the forward traffic (both her own and Alice's post-join message)…
    assert!(
        carol_texts.contains(&"after Carol joined".to_string()),
        "carol got forward traffic, saw {carol_texts:?}"
    );
    assert!(carol_texts.contains(&"Carol here".to_string()));
    // …but NOT the pre-join history — the pinned no-backfill semantics.
    assert!(
        !carol_texts.contains(&"first".to_string()),
        "no pre-join backfill, saw {carol_texts:?}"
    );
    assert!(!carol_texts.contains(&"second".to_string()));

    // The original members see EVERYTHING, including Carol's post-join message.
    for u in [alice, bob] {
        let t: Vec<String> = h.obj_view(u, &obj).iter().map(|m| m.text.clone()).collect();
        for expected in ["first", "second", "after Carol joined", "Carol here"] {
            assert!(
                t.contains(&expected.to_string()),
                "original member {u} missing {expected:?}, saw {t:?}"
            );
        }
    }
}

/// Durable group STATE (the name) is the ONE thing a late joiner DOES converge on —
/// not via a backfilled message delta (forward secrecy forbids that), but via the
/// Welcome's GroupInfo, where the name rides an MLS GroupContext extension.
#[tokio::test]
async fn group_name_reaches_a_late_joiner_via_the_welcome() {
    let h = Harness::new(&["Alice", "Bob", "Carol"]).await;
    let (alice, bob, carol) = (0, 1, 2);

    // Alice creates a NAMED group and messages before Carol exists in it.
    let obj = h.form_named_forum(alice, "Design Review");
    h.add_to_forum(alice, bob, &obj).await;
    h.obj_post(alice, &obj, "first").await;
    h.settle().await;
    // Both original members read the name.
    assert_eq!(h.object_name(alice, &obj), "Design Review");
    assert_eq!(h.object_name(bob, &obj), "Design Review");

    // Carol joins LATE, at a later epoch.
    h.add_to_forum(alice, carol, &obj).await;

    // Forward secrecy: Carol gets NO pre-join message…
    let carol_texts: Vec<String> = h
        .obj_view(carol, &obj)
        .iter()
        .map(|m| m.text.clone())
        .collect();
    assert!(
        !carol_texts.contains(&"first".to_string()),
        "no message backfill, saw {carol_texts:?}"
    );
    // …but she DOES read the current group NAME straight from her Welcome.
    assert_eq!(
        h.object_name(carol, &obj),
        "Design Review",
        "late joiner reads the group name from the GroupContext in her Welcome",
    );
}

/// A RENAME is durable group state too: an owner GroupContextExtensions commit that
/// existing members fold off the epoch tag, and that anyone added afterwards reads
/// from their Welcome. The name has exactly one home at every epoch.
#[tokio::test]
async fn owner_rename_reaches_existing_members_and_later_joiners() {
    let h = Harness::new(&["Alice", "Bob", "Carol"]).await;
    let (alice, bob, carol) = (0, 1, 2);

    let obj = h.form_named_forum(alice, "Design Review");
    h.add_to_forum(alice, bob, &obj).await;
    assert_eq!(h.object_name(bob, &obj), "Design Review");

    // Alice renames at a live epoch, with Bob already in the group.
    h.rename_object(alice, &obj, "Paris 27").await;
    assert_eq!(h.object_name(alice, &obj), "Paris 27");
    assert_eq!(
        h.object_name(bob, &obj),
        "Paris 27",
        "an existing member folds the rename commit off the epoch tag",
    );

    // Carol is added AFTER the rename — her Welcome carries the CURRENT name.
    h.add_to_forum(alice, carol, &obj).await;
    assert_eq!(
        h.object_name(carol, &obj),
        "Paris 27",
        "a member added after the rename reads the new name, never the old one",
    );

    // And the rename did not disturb the roster or the transcript.
    h.obj_post(alice, &obj, "renamed and still talking").await;
    h.settle().await;
    for u in [alice, bob, carol] {
        let texts: Vec<String> = h.obj_view(u, &obj).iter().map(|m| m.text.clone()).collect();
        assert!(
            texts.contains(&"renamed and still talking".to_string()),
            "member {u} lost the group after the rename, saw {texts:?}",
        );
    }
}

/// Renaming is owner-sequenced, exactly like adding a member: a non-owner is
/// rejected before any commit is staged, and the name is left untouched.
#[tokio::test]
async fn only_the_owner_can_rename() {
    let h = Harness::new(&["Alice", "Bob"]).await;
    let (alice, bob) = (0, 1);

    let obj = h.form_named_forum(alice, "Design Review");
    h.add_to_forum(alice, bob, &obj).await;

    assert!(
        h.try_rename(bob, &obj, "hijacked").await.is_err(),
        "a non-owner must not be able to rename the group",
    );
    h.settle().await;
    assert_eq!(h.object_name(alice, &obj), "Design Review");
    assert_eq!(h.object_name(bob, &obj), "Design Review");
}

/// A 3-way DM mesh: three pairwise connections, each converges independently.
#[tokio::test]
async fn three_user_dm_mesh_converges() {
    let h = Harness::new(&["Alice", "Bob", "Carol"]).await;
    let (alice, bob, carol) = (0, 1, 2);

    h.pair(alice, bob).await;
    h.pair(alice, carol).await;
    h.pair(bob, carol).await;

    h.dm_post(alice, bob, "A→B").await;
    h.dm_post(bob, carol, "B→C").await;
    h.dm_post(carol, alice, "C→A").await;
    h.settle().await;

    // Each pairwise DM is its own group and converges on both ends.
    let ab = h.assert_dm_converges(alice, bob);
    let bc = h.assert_dm_converges(bob, carol);
    let ca = h.assert_dm_converges(carol, alice);
    assert_eq!(ab.last().unwrap().text, "A→B");
    assert_eq!(bc.last().unwrap().text, "B→C");
    assert_eq!(ca.last().unwrap().text, "C→A");
}
