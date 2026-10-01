//! m12 — a Group's Forums: the chatrooms a Group hosts.
//!
//! The edge lives on the GROUP's own log (`group.setForum`/`group.clearForum`),
//! so every member folds the same room set out of a log they already carry; who
//! is IN each room stays the room's own MLS roster (no-dual-source). These
//! scenarios drive the same `Node` verbs the FFI exports — `group_forum_new`,
//! `group_forum_attach`, `group_forum_detach`, `group_forums`, `group_rows`.

mod common;
use common::Harness;

const T0: i64 = 1_700_000_000_000;

/// The solo lifecycle: mint-and-attach in one call, read it back, live-rename
/// through the room's own GroupContext, attach an existing room, detach.
#[tokio::test]
async fn a_groups_rooms_fold_from_its_own_log() {
    let h = Harness::new(&["Alice"]).await;
    let alice = h.node(0);

    let group = h.mint_group(0);
    h.group_set_profile(0, &group, "Aries Climbing", "team")
        .await;

    // Mint + attach in one call.
    let general = h
        .with(0, |n| {
            let group = group.clone();
            async move { n.group_forum_new(&group, "general", T0).await }
        })
        .await
        .expect("mint the room");

    let rooms = h.node(0).group_forums(&group).expect("read rooms");
    assert_eq!(rooms.len(), 1);
    assert_eq!(rooms[0].0, general);
    assert_eq!(rooms[0].1, "general");
    assert_eq!(rooms[0].2, T0);
    assert!(rooms[0].3, "the minter holds the room");

    // The room is a real forum object — posting works through the same verbs
    // every chat surface uses.
    h.obj_post(0, &general, "first!").await;
    assert_eq!(h.obj_view(0, &general).len(), 1);

    // A rename lands in the room's GroupContext, and the listing reads the LIVE
    // name — the edge's label is only the fallback for rooms we don't hold.
    h.node(0)
        .object_rename(&general, "announcements")
        .await
        .expect("rename");
    let rooms = h.node(0).group_forums(&group).expect("re-read");
    assert_eq!(rooms[0].1, "announcements");

    // Attach an EXISTING room (the set half over a room minted elsewhere).
    let lounge = h.form_named_forum(0, "lounge");
    h.with(0, |n| {
        let group = group.clone();
        let lounge = lounge.clone();
        async move { n.group_forum_attach(&group, &lounge, T0 + 1).await }
    })
    .await
    .expect("attach existing");
    let rooms = h.node(0).group_forums(&group).expect("read both");
    assert_eq!(
        rooms.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(),
        vec!["announcements", "lounge"],
        "attach order (by `at`), live names"
    );

    // Detach clears the edge and ONLY the edge — the room and its log survive.
    h.with(0, |n| {
        let group = group.clone();
        let lounge = lounge.clone();
        async move { n.group_forum_detach(&group, &lounge).await }
    })
    .await
    .expect("detach");
    let rooms = h.node(0).group_forums(&group).expect("read after detach");
    assert_eq!(rooms.len(), 1);
    assert_eq!(
        h.object_name(0, &lounge),
        "lounge",
        "the room object survives"
    );

    // The GROUPS listing folds the same truth: one group, one room. Filtered to
    // SPACES — `group_rows` also returns identity records, flagged, and since the
    // self record landed (m30) every account has one of those from birth.
    let listed: Vec<_> = alice
        .group_rows()
        .expect("group_rows")
        .into_iter()
        .filter(|r| r.3)
        .collect();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].0, group);
    assert_eq!(listed[0].1.display_name, "Aries Climbing");
    assert_eq!(listed[0].1.parts.len(), 1);
    assert_eq!(listed[0].2, 1, "roster of one");

    // And the REVERSE edge resolves: the room knows its host — what the CHAT
    // list joins on to render "Aries Climbing / announcements".
    let hosts = alice.forum_hosts().expect("forum_hosts");
    assert_eq!(
        hosts,
        vec![(general.clone(), group.clone(), "Aries Climbing".to_string())],
        "one edge: the surviving room, hosted by the group, wearing its name"
    );
}

/// The kind gates are loud: rooms hang off Group identity records only, and only
/// forum-kind objects are rooms.
#[tokio::test]
async fn forum_edges_refuse_wrong_kinds() {
    let h = Harness::new(&["Alice"]).await;

    let group = h.mint_group(0);
    let project = h.node(0).object_new("project", "not a room").unwrap();

    // A project is not a room.
    let err = h
        .with(0, |n| {
            let group = group.clone();
            let project = project.clone();
            async move { n.group_forum_attach(&group, &project, T0).await }
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("'project'"),
        "refused for the right reason: {err}"
    );

    // A project hosts no rooms either (its ChatRoom rides project.subscribe).
    let err = h
        .with(0, |n| {
            let project = project.clone();
            async move { n.group_forum_new(&project, "general", T0).await }
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("'project'"),
        "refused for the right reason: {err}"
    );
}

/// The N-member truth: every group member folds the same room set from the
/// group's log, the minter's `group_forum_new` pulls connected members into the
/// room by consuming their stocked prekeys, and the room then converges like any
/// chat. A non-owner cannot attach rooms (owner/sequenced, refused loudly).
#[tokio::test]
async fn members_fold_the_room_set_and_join_the_room() {
    let h = Harness::new(&["Alice", "Bob"]).await;
    h.pair(0, 1).await;

    // Alice constitutes the group and owner-adds Bob to IT (the identity record).
    let group = h.mint_group(0);
    h.group_set_profile(0, &group, "Aries Climbing", "team")
        .await;
    h.add_to_forum(0, 1, &group).await;

    // One call: mint the room, record the edge, add every reachable member.
    let room = h
        .with(0, |n| {
            let group = group.clone();
            async move { n.group_forum_new(&group, "general", T0).await }
        })
        .await
        .expect("mint the room");
    h.settle().await;

    // Bob folds the edge out of the group log he already carries…
    let bobs = h.node(1).group_forums(&group).expect("bob reads rooms");
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

    // VOTES — the Reddit half. Bob upvotes Alice's "welcome": both replicas fold
    // the same score, a flip replaces (one voter, one direction — LWW), and a
    // clear empties both lists. All real `forum.vote` deltas over the relay.
    let welcome = h.find_forum(1, &room, "welcome");
    async fn cast(h: &Harness, room: &str, target: ([u8; 32], u64), u: usize, dir: i8) {
        let room_owned = room.to_string();
        h.with(u, |n| async move {
            n.object_vote_post(&room_owned, target, dir).await
        })
        .await
        .expect("vote lands");
        h.settle().await;
    }
    cast(&h, &room, welcome, 1, 1).await;
    let agreed = h.assert_forum_converges(&room, &[0, 1]);
    let m = agreed.iter().find(|m| m.text == "welcome").unwrap();
    assert_eq!(
        (m.up.len(), m.down.len()),
        (1, 0),
        "Bob's upvote folds everywhere"
    );

    cast(&h, &room, welcome, 1, -1).await;
    let agreed = h.assert_forum_converges(&room, &[0, 1]);
    let m = agreed.iter().find(|m| m.text == "welcome").unwrap();
    assert_eq!(
        (m.up.len(), m.down.len()),
        (0, 1),
        "a flip replaces — never double-counts"
    );

    cast(&h, &room, welcome, 1, 0).await;
    let agreed = h.assert_forum_converges(&room, &[0, 1]);
    let m = agreed.iter().find(|m| m.text == "welcome").unwrap();
    assert_eq!(
        (m.up.len(), m.down.len()),
        (0, 0),
        "cleared on both replicas"
    );

    // Owner-only: Bob authoring the edge is refused before anything lands.
    let err = h
        .with(1, |n| {
            let group = group.clone();
            async move { n.group_forum_new(&group, "bob's room", T0 + 1).await }
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("owner"), "refused for the right reason: {err}");
    h.settle().await;
    assert_eq!(
        h.node(0).group_forums(&group).unwrap().len(),
        1,
        "nothing landed from the refused attach"
    );
}
