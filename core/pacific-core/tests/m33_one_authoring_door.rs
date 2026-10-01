//! m33 — `Node::apply`: one write path, every kind.
//!
//! `mint` brings a GroupObject into existence; `apply` is everything that happens
//! to one afterwards. Ruled 24 September 2026, replacing the nine `*_author`
//! doors — nine copies of one sequence wrapped around nine `T`s, which had
//! drifted: eight stamped a commutative delta's `gen` with a per-author counter
//! that `post_to_group`'s own comment names as a bug it had already fixed in
//! itself and nowhere else.
//!
//! `apply` does the I/O a pure door cannot — converge, gather the context,
//! append — and `authoring::build` does the rest, so the rule is stated once and
//! the browser authoring through that same door gets every gate the native
//! caller does.

mod common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args};

/// It reaches every kind that has a door, through the kind's own door.
#[tokio::test]
async fn it_writes_through_each_kinds_own_door() {
    let h = Harness::new(&["Ana"]).await;
    let n = h.node(0);

    // A forum's own op table.
    let forum = n.object_new("forum", "friday").expect("mint");
    let room = pacific_core::parts::set_part_args(&forum, "room", 1_790_000_000_000);
    n.apply(&forum, pacific_core::parts::OP_SET_PART, room)
        .await
        .expect("a forum op, through forum_author");

    // A group's.
    let group = n.object_new("group", "the studio").expect("mint");
    let mut profile = Args::new();
    profile.insert("displayName".into(), ArgVal::Text("The Studio".into()));
    profile.insert("shape".into(), ArgVal::Text("community".into()));
    n.apply(&group, pacific_core::group::OP_SET_PROFILE, profile)
        .await
        .expect("a group op, through group_author");

    let v: serde_json::Value =
        serde_json::from_str(&n.object_view(&group).unwrap()).unwrap();
    assert_eq!(v["display_name"], "The Studio", "and it folded");
}

/// A CONVERSATION takes the forum table under its own wire type — the door that
/// required kind == "forum" left the kind the ICD names for DMs unwritable.
#[tokio::test]
async fn it_writes_a_conversation_too() {
    let h = Harness::new(&["Ana"]).await;
    let n = h.node(0);
    let convo = n.object_new("conversation", "thursday").expect("mint");
    let mut post = Args::new();
    post.insert("text".into(), ArgVal::Text("through the one door".into()));
    n.apply(&convo, pacific_core::coordinator::FORUM_POST, post)
        .await
        .expect("a chat op on a conversation");
    let seen = n.object_transcript(&convo).expect("fold");
    assert!(seen.iter().any(|l| l.contains("through the one door")), "{seen:?}");
}

/// EVERY COMMUTATIVE OP, through every door that probes. The doors compute the
/// Lamport `gen` themselves and used to do it AFTER the dry run, so the probe saw
/// args without a field the reducer requires and refused the op for a value the
/// door was about to supply. Four doors had it; nothing had ever called them with
/// a commutative op.
#[tokio::test]
async fn a_commutative_op_survives_its_own_doors_probe() {
    let h = Harness::new(&["Ana"]).await;
    let n = h.node(0);

    let post = n.object_new("post", "a piece").expect("mint");
    let mut react = Args::new();
    react.insert("emoji".into(), ArgVal::Text("\u{1f44f}".into()));
    react.insert("at".into(), ArgVal::Int(1_790_000_000_000));
    n.apply(&post, pacific_core::post::OP_REACT, react)
        .await
        .expect("post.react is commutative");

    let convo = n.object_new("conversation", "thursday").expect("mint");
    let mut text = Args::new();
    text.insert("text".into(), ArgVal::Text("and so is a message".into()));
    n.apply(&convo, pacific_core::coordinator::FORUM_POST, text)
        .await
        .expect("forum.post is commutative");
}

/// It adds no authority: an op the kind does not declare is refused, in the
/// door's own words, not the dispatcher's.
#[tokio::test]
async fn it_refuses_an_op_the_kind_does_not_declare() {
    let h = Harness::new(&["Ana"]).await;
    let n = h.node(0);
    let thing = n.object_new("thing", "a ladder").expect("mint");
    let refused = n
        .apply(&thing, pacific_core::roles::OP_SET_ROLE, Args::new())
        .await;
    assert!(refused.is_err(), "a Group op is not a Thing op: {refused:?}");
}

/// A PAIRING CHANNEL TAKES THE LEGS THAT RIDE IT, and nothing else. The
/// allowlist used to live inside one `Node` method, so it held for the native
/// caller and not for the browser; it is `authorable_as`'s now, beside the
/// MLS-doors and note gates. The prekey and profile ops are written by the
/// pairing and sync paths, which are not authoring doors at all.
#[tokio::test]
async fn a_connections_door_takes_only_the_legs_that_ride_it() {
    let h = Harness::new(&["ana", "ben"]).await;
    h.pair(0, 1).await;
    let n = h.node(0);
    let connection = n.connection_object(&h.id(1)).unwrap().unwrap().0;
    let err = n
        .apply(&connection, pacific_core::contact::OP_PREKEY_SUPPLY, Args::new())
        .await
        .expect_err("a prekey is stocked, never authored");
    assert!(
        format!("{err}").contains("not a contact leg"),
        "and the allowlist says so in its own words: {err}"
    );
}
