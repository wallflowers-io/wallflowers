//! THE SWEEP (`Node::object_rows`, `Node::people`). Who made an object is the MLS
//! group's first owner, so a member reads it from the group, not from a label; and a
//! device that has never seen the object reads the same after a sign-in from the
//! words alone. A person's avatar is the profile they published, folded.

use crate::common;

use common::Harness;
use pacific_core::group::ContactCard;
use pacific_core::object::{ObjectKind, COMMENTS};
use pacific_core::resumption::HeadInput;

#[tokio::test]
async fn a_post_names_its_maker_on_every_members_device_and_a_restored_one() {
    let mut h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let post = h.mint_object(ada, ObjectKind::Post, "a post", &[bo]).await;

    let seen = |h: &Harness, u: usize| {
        let rows = h.with_sync(u, |n| n.object_rows(Some(ObjectKind::Post))).unwrap();
        let row = rows.into_iter().find(|r| r.id == post).expect("the post is held");
        assert_eq!(row.kind, "post");
        (row.creator, row.members.len())
    };
    assert_eq!(seen(&h, ada), (Some(h.id(ada)), 2), "Ada made it");
    assert_eq!(seen(&h, bo), (Some(h.id(ada)), 2), "Bo reads Ada as its maker, not himself");

    // The comments section names its post, from its own fold. Its roster is its own
    // (ICD `kinds.post.parts`: role-gateable), so it is the maker's until someone adds.
    let forums = h.with_sync(ada, |n| n.object_rows(Some(ObjectKind::Forum))).unwrap();
    let comments: Vec<_> = forums
        .iter()
        .filter(|r| r.parent.as_ref().is_some_and(|p| p.parent == post && p.role == COMMENTS))
        .collect();
    assert_eq!(comments.len(), 1, "one comments Forum for the post");

    // A device of Bo's that has never seen any of it.
    let words = h.with_sync(bo, |n| n.recovery_key()).expect("a minted identity has words");
    let head = h.with_sync(bo, |n| n.head_update().unwrap());
    h.partition(bo);
    let (fresh, _) = h
        .sign_in_from_words("bo", "fresh", &words, Some(HeadInput { blob: head.sealed, declared_position: head.position }))
        .await
        .unwrap();
    assert_eq!(seen(&h, fresh).0, Some(h.id(ada)), "the restored device reads the same maker");
}

#[tokio::test]
async fn people_carries_each_connections_published_avatar() {
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    h.pair(ada, bo).await;
    h.settle().await;
    let card = ContactCard {
        photo: "cGhvdG8=".into(),
        photo_mime: "image/jpeg".into(),
        captured_at: 1_794_000_000,
        avatar_color: "33C08C".into(),
        ..Default::default()
    };
    h.set_profile(ada, "Ada", &card).await;
    h.settle().await;

    let people = h.with_sync(bo, |n| n.people()).unwrap();
    assert!(people[0].is_me && people[0].identity == h.id(bo), "Bo first");
    let her = people.iter().find(|p| p.identity == h.id(ada)).expect("Ada is Bo's connection");
    assert_eq!((her.color.as_str(), her.photo_at), ("33C08C", 1_794_000_000));
    assert_eq!(
        h.with_sync(bo, |n| n.person_photo(&h.id(ada))).unwrap(),
        Some(("cGhvdG8=".into(), "image/jpeg".into())),
        "the photo is read once, by identity"
    );
}
