//! A COMMENTS SECTION IS ITS PARENT'S (#34). A Post is minted with a Forum in role
//! `comments`; the Forum's own fold says so (`base.setParent`), so a device that has
//! never seen either reads the same edge after a sign-in from the words alone, and a
//! list of conversations can leave the Forum out by reading the fold, not its name.

use crate::common;

use common::Harness;
use pacific_core::object::{ObjectKind, COMMENTS};
use pacific_core::resumption::HeadInput;

fn comments_of(rows: &[(String, String, String)], parent: &str) -> Vec<String> {
    rows.iter().filter(|(_, p, r)| p == parent && r == COMMENTS).map(|(f, _, _)| f.clone()).collect()
}

#[tokio::test]
async fn a_posts_comments_forum_names_its_post_on_a_fresh_device() {
    let mut h = Harness::new(&["ada", "bo"]).await;
    let phone = 0;
    let draft = pacific_core::mint::MintDraft { name: "a post".into(), ..Default::default() };
    let post = h.with(phone, |n| async move { n.mint(ObjectKind::Post, &draft).await }).await.expect("the phone mints");
    h.settle().await;

    let on_phone = comments_of(&h.with_sync(phone, |n| n.forum_parents()).unwrap(), &post);
    assert_eq!(on_phone.len(), 1, "one comments Forum, from its own fold: {on_phone:?}");
    assert_eq!(h.with_sync(phone, |n| n.object_kind(&on_phone[0])).unwrap(), "forum");

    let words = h.with_sync(phone, |n| n.recovery_key()).expect("a minted identity has words");
    let head = h.with_sync(phone, |n| n.head_update().unwrap());
    h.partition(phone);
    let (sim, _) = h
        .sign_in_from_words("ada", "sim", &words, Some(HeadInput { blob: head.sealed, declared_position: head.position }))
        .await
        .unwrap();

    assert_eq!(
        comments_of(&h.with_sync(sim, |n| n.forum_parents()).unwrap(), &post),
        on_phone,
        "the device that has never seen it reads the same edge"
    );
}
