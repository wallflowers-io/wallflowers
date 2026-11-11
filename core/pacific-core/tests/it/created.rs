//! WHEN AN OBJECT WAS MADE (`mls.groupContextExtensions.createdAt`). The time rides the
//! GroupContext, so a member added later and a device restored from the words read it
//! without holding any log from before they joined. (A Site listing carries its Thing's
//! time: `w98_site_listings.rs`.)

use crate::common;

use common::Harness;
use pacific_core::mint::MintDraft;
use pacific_core::object::{ObjectKind, COMMENTS};
use pacific_core::resumption::HeadInput;

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

fn created(h: &Harness, u: usize, kind: ObjectKind, id: &str) -> Option<i64> {
    let rows = h.with_sync(u, |n| n.object_rows(Some(kind))).unwrap();
    rows.into_iter().find(|r| r.id == id).expect("the object is held").created_at
}

#[tokio::test]
async fn a_member_added_later_and_a_restored_device_read_when_it_was_made() {
    let mut h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let before = now_ms();
    let post = h.mint_object(ada, ObjectKind::Post, "a post", &[bo]).await;
    let draft = MintDraft { name: "an event".into(), start_ms: before + 86_400_000, ..Default::default() };
    let event = h.node(ada).mint(ObjectKind::Event, &draft).await.unwrap();
    h.add_to_forum(ada, bo, &event).await;
    let after = now_ms();

    for (kind, id) in [(ObjectKind::Post, &post), (ObjectKind::Event, &event)] {
        let at = created(&h, ada, kind, id).expect("made with its time");
        assert!(before <= at && at <= after, "by the maker's clock");
        assert_eq!(created(&h, bo, kind, id), Some(at), "Bo, added after, reads the same");
    }

    // The comments section is a group of its own, and was made in the same act.
    let forums = h.with_sync(ada, |n| n.object_rows(Some(ObjectKind::Forum))).unwrap();
    let comments = forums
        .iter()
        .find(|r| r.parent.as_ref().is_some_and(|p| p.parent == post && p.role == COMMENTS))
        .expect("the post's comments Forum");
    assert!(comments.created_at.is_some_and(|t| before <= t && t <= after));

    // A device of Bo's that has never seen any of it.
    let words = h.with_sync(bo, |n| n.recovery_key()).expect("a minted identity has words");
    let head = h.with_sync(bo, |n| n.head_update().unwrap());
    let held = (created(&h, bo, ObjectKind::Post, &post), created(&h, bo, ObjectKind::Event, &event));
    h.partition(bo);
    let (fresh, _) = h
        .sign_in_from_words("bo", "fresh", &words, Some(HeadInput { blob: head.sealed, declared_position: head.position }))
        .await
        .unwrap();
    assert_eq!(
        (created(&h, fresh, ObjectKind::Post, &post), created(&h, fresh, ObjectKind::Event, &event)),
        held,
        "the restored device reads the same times"
    );
}
