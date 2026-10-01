//! m13 — a Forum's Rooms: the constituent tabs a Channel hosts.
//!
//! The edge lives on the PARENT forum's own log (`forum.setRoom`/`clearRoom`,
//! owner-sequenced — the Group→Forum edge one level down), so every member folds
//! the same tab set out of a log they already carry; who is IN each room stays
//! the room's own MLS roster (no-dual-source). Each Room is a full `forum`-kind
//! GroupObject in its own right. These scenarios drive the same `Node` verbs the
//! FFI exports — `forum_room_new`, `forum_room_attach`, `forum_room_detach`,
//! `forum_rooms`, `room_hosts`.

mod common;
use common::Harness;

const T0: i64 = 1_700_000_000_000;

/// The solo lifecycle: mint-and-attach in one call, read it back, live-rename
/// through the room's own GroupContext, attach an existing forum as a room,
/// detach — and the reverse edge the CHAT list joins on.
#[tokio::test]
async fn a_channels_rooms_fold_from_its_own_log() {
    let h = Harness::new(&["Alice"]).await;
    let alice = h.node(0);

    // The Channel — itself a plain forum-kind object.
    let channel = h.form_named_forum(0, "aries");

    // Mint + attach in one call.
    let general = h
        .with(0, |n| {
            let channel = channel.clone();
            async move { n.forum_room_new(&channel, "general", T0).await }
        })
        .await
        .expect("mint the room");

    let rooms = h.node(0).forum_rooms(&channel).expect("read rooms");
    assert_eq!(rooms.len(), 1);
    assert_eq!(rooms[0].0, general);
    assert_eq!(rooms[0].1, "general");
    assert_eq!(rooms[0].2, T0);
    assert!(rooms[0].3, "the minter holds the room");

    // The room is a real forum object — posting works through the same verbs
    // every chat surface uses, and the PARENT's transcript stays untouched.
    h.obj_post(0, &general, "first!").await;
    assert_eq!(h.obj_view(0, &general).len(), 1);
    assert_eq!(h.obj_view(0, &channel).len(), 0);

    // A rename lands in the room's GroupContext, and the listing reads the LIVE
    // name — the edge's label is only the fallback for rooms we don't hold.
    h.node(0)
        .object_rename(&general, "announcements")
        .await
        .expect("rename");
    let rooms = h.node(0).forum_rooms(&channel).expect("re-read");
    assert_eq!(rooms[0].1, "announcements");

    // Attach an EXISTING forum (the set half over a room minted elsewhere).
    let lounge = h.form_named_forum(0, "lounge");
    h.with(0, |n| {
        let channel = channel.clone();
        let lounge = lounge.clone();
        async move { n.forum_room_attach(&channel, &lounge, T0 + 1).await }
    })
    .await
    .expect("attach existing");
    let rooms = h.node(0).forum_rooms(&channel).expect("read both");
    assert_eq!(
        rooms.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(),
        vec!["announcements", "lounge"],
        "attach order (by `at`), live names"
    );

    // Detach clears the edge and ONLY the edge — the room and its log survive.
    h.with(0, |n| {
        let channel = channel.clone();
        let lounge = lounge.clone();
        async move { n.forum_room_detach(&channel, &lounge).await }
    })
    .await
    .expect("detach");
    let rooms = h.node(0).forum_rooms(&channel).expect("read after detach");
    assert_eq!(rooms.len(), 1);
    assert_eq!(
        h.object_name(0, &lounge),
        "lounge",
        "the room object survives"
    );

    // Detaching a room that was never attached is refused at author time (the
    // dry-run probe), never written as an inert delta.
    let err = h
        .with(0, |n| {
            let channel = channel.clone();
            let lounge = lounge.clone();
            async move { n.forum_room_detach(&channel, &lounge).await }
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("PreconditionFailed"),
        "refused for the right reason: {err}"
    );

    // And the REVERSE edge resolves: the room knows its host — what the CHAT
    // list joins on to render "aries / announcements".
    let hosts = alice.room_hosts().expect("room_hosts");
    assert_eq!(
        hosts,
        vec![(general.clone(), channel.clone(), "aries".to_string())],
        "one edge: the surviving room, hosted by the channel, wearing its name"
    );
}

/// The kind gates are loud: rooms hang off forum-kind objects only, only
/// forum-kind objects are rooms, and a channel can never host itself.
#[tokio::test]
async fn room_edges_refuse_wrong_kinds_and_self() {
    let h = Harness::new(&["Alice"]).await;

    let channel = h.form_named_forum(0, "aries");
    let project = h.node(0).object_new("project", "not a room").unwrap();

    // A project is not a room.
    let err = h
        .with(0, |n| {
            let channel = channel.clone();
            let project = project.clone();
            async move { n.forum_room_attach(&channel, &project, T0).await }
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("'project'"),
        "refused for the right reason: {err}"
    );

    // A project hosts no rooms either.
    let err = h
        .with(0, |n| {
            let project = project.clone();
            async move { n.forum_room_new(&project, "general", T0).await }
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("'project'"),
        "refused for the right reason: {err}"
    );

    // A channel cannot be its own tab.
    let err = h
        .with(0, |n| {
            let channel = channel.clone();
            async move {
                let c2 = channel.clone();
                n.forum_room_attach(&channel, &c2, T0).await
            }
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("cannot host itself"),
        "refused for the right reason: {err}"
    );
}

/// The N-member truth: every channel member folds the same tab set from the
/// parent's log, the minter's `forum_room_new` pulls connected members into the
/// room by consuming their stocked prekeys, and the room then converges like
/// any chat. A non-owner cannot attach rooms (owner/sequenced, refused loudly).
#[tokio::test]
async fn members_fold_the_tab_set_and_join_the_room() {
    let h = Harness::new(&["Alice", "Bob"]).await;
    h.pair(0, 1).await;

    // Alice constitutes the channel and owner-adds Bob to IT (the parent).
    let channel = h.form_named_forum(0, "aries");
    h.add_to_forum(0, 1, &channel).await;

    // One call: mint the room, record the edge, add every reachable member.
    let room = h
        .with(0, |n| {
            let channel = channel.clone();
            async move { n.forum_room_new(&channel, "general", T0).await }
        })
        .await
        .expect("mint the room");
    h.settle().await;

    // Bob folds the edge out of the parent log he already carries…
    let bobs = h.node(1).forum_rooms(&channel).expect("bob reads rooms");
    assert_eq!(bobs.len(), 1);
    assert_eq!(bobs[0].0, room);
    assert_eq!(bobs[0].1, "general");
    assert!(
        bobs[0].3,
        "…and HOLDS the room: his prekey was consumed to add him"
    );

    // The room is a working chatroom for both.
    h.obj_post(0, &room, "welcome").await;
    h.settle().await;
    h.obj_post(1, &room, "hi!").await;
    h.settle().await;
    h.assert_forum_converges(&room, &[0, 1]);

    // A NON-owner's attach is refused loudly before anything is written — an
    // owner-sequenced delta from a non-owner would poison the spine.
    let err = h
        .with(1, |n| {
            let channel = channel.clone();
            async move { n.forum_room_new(&channel, "coup", T0 + 1).await }
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("owner-only"),
        "refused for the right reason: {err}"
    );
}
