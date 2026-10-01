//! m31 — the fork: a Conversation is its own GroupObject, not a Forum in disguise.
//!
//! Ruled 24 September 2026. Before it, `authoring::LENSES` said a `conversation`
//! writes `ConversationType` (wire type 26) and `fold::lens_for` said it folds as
//! `Forum` (19). Nothing had ever created one, so nothing had failed — but the
//! first delta authored through the generic door would have been stored and then
//! filtered out of every fold, silently, because an empty fold is a valid fold.
//!
//! The first test is the one that matters: it is a SWEEP, so this class of drift
//! cannot come back on some other kind while this file stays green.

mod common;

use common::Harness;
use pacific_core::object::ObjectKind;

/// THE PIN. What a kind is written as and what it is read as are the same lens,
/// for every kind the door will write to.
#[test]
fn every_kind_authors_and_folds_under_the_same_lens() {
    for kind in pacific_core::authoring::kinds() {
        let written = pacific_core::authoring::channel_of(kind);
        let read = pacific_core::fold::lens_for(kind);
        assert_eq!(
            written, read,
            "'{kind}' is authored as {written:?} and folded as {read:?} — a delta \
             written under one and filtered under the other is stored and invisible"
        );
    }
}

/// The two chat kinds are two objects, with two wire types.
#[test]
fn a_conversation_and_a_forum_are_different_types() {
    assert_eq!(
        pacific_core::fold::lens_for("conversation"),
        Some(ObjectKind::Conversation)
    );
    assert_eq!(pacific_core::fold::lens_for("forum"), Some(ObjectKind::Forum));
    assert_ne!(
        ObjectKind::Conversation.type_id(),
        ObjectKind::Forum.type_id(),
        "same type id would make the fork cosmetic"
    );
}

/// A message posted into a conversation FOLDS. The fold filters on the type id,
/// so an empty transcript here is the old bug: the post went in under 19 and the
/// conversation reads 26.
#[tokio::test]
async fn a_post_into_a_conversation_folds() {
    let h = Harness::new(&["Ana"]).await;
    let convo = h
        .node(0)
        .object_new("conversation", "thursday")
        .expect("a conversation is a group of one until someone is added");

    h.node(0)
        .object_post_reply(&convo, "first", None, None)
        .await
        .expect("post");

    let seen = h.node(0).object_transcript(&convo).expect("fold");
    assert!(
        seen.iter().any(|l| l.contains("first")),
        "the post folds under the conversation's own type: {seen:?}"
    );
    h.node(0)
        .object_compliance(&convo)
        .expect("and the object is compliant");
}

/// The same message into a forum still folds — the fork did not move the forum.
#[tokio::test]
async fn a_post_into_a_forum_still_folds() {
    let h = Harness::new(&["Ben"]).await;
    let forum = h.node(0).object_new("forum", "friday").expect("mint");
    h.node(0)
        .object_post_reply(&forum, "still here", None, None)
        .await
        .expect("post");
    let seen = h.node(0).object_transcript(&forum).expect("fold");
    assert!(seen.iter().any(|l| l.contains("still here")), "{seen:?}");
}

/// AN EVENT'S DISCUSSION IS STILL FORUM-TYPED. The chat op-group is not the
/// object's own type: an Event folds as an Event and carries a forum thread
/// beside it. The fork moved `conversation` and nothing else, and this is the
/// regression it caused on the first run.
#[tokio::test]
async fn an_events_discussion_is_forum_typed() {
    let h = Harness::new(&["Dee"]).await;
    let event = h.node(0).object_new("event", "the gig").expect("mint");
    h.node(0)
        .object_post(&event, "who's going early?", None)
        .await
        .expect("an event carries a discussion");
    let seen = h.node(0).object_transcript(&event).expect("fold");
    assert!(seen.iter().any(|l| l.contains("going early")), "{seen:?}");
    h.node(0)
        .object_compliance(&event)
        .expect("and the event still folds as an Event");
}

/// Nothing this device holds is unfoldable — the sweep the doctrine asks for.
#[tokio::test]
async fn neither_kind_leaves_a_noncompliant_object() {
    let h = Harness::new(&["Cass"]).await;
    h.node(0).object_new("conversation", "one").expect("mint");
    h.node(0).object_new("forum", "two").expect("mint");
    assert!(
        h.node(0).noncompliant_objects().expect("sweep").is_empty(),
        "everything I hold, I can reduce"
    );
}
