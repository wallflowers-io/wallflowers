//! THE TAGS FACET (`facets.tags`; Ralph, 12 Nov 2026). Members tag an object, each tag a label
//! and optionally a linked object; the owner hides a label for everyone; a linked object's own
//! member writes its half. All of it is Deltas, so a device restored from the words alone
//! folds the same tags.

use crate::common;

use common::Harness;
use pacific_core::object::ObjectKind;
use pacific_core::resumption::HeadInput;
use pacific_core::tags::{self, MAX_LIVE, MAX_PER_AUTHOR, OP_HIDE_TAG, OP_SET_TAGGED, OP_TAG, OP_UNTAG};

const AT: i64 = 1_794_000_000_000;

fn view(h: &Harness, u: usize, obj: &str, key: &str) -> serde_json::Value {
    let v: serde_json::Value = serde_json::from_str(&h.with_sync(u, |n| n.object_view(obj)).unwrap()).unwrap();
    v[key].clone()
}

fn row<'a>(tags: &'a serde_json::Value, label: &str) -> Option<&'a serde_json::Value> {
    tags.as_array().unwrap().iter().find(|t| t["label"] == label)
}

#[tokio::test]
async fn members_tag_the_owner_hides_a_link_has_two_halves_and_a_fresh_device_folds_the_same() {
    let mut h = Harness::new(&["ada", "bo", "cy"]).await;
    let (ada, bo, cy) = (0, 1, 2);
    let post = h.mint_object(ada, ObjectKind::Post, "allotment open day", &[bo, cy]).await;
    let tag = |u: usize, label: &str, object: Option<&str>| {
        let n = h.node(u);
        let (post, args) = (post.clone(), tags::tag_args(label, object, AT));
        async move { n.apply(&post, OP_TAG, args).await }
    };

    // Two members, one label: one row, counted twice.
    tag(ada, "parsnips", None).await.expect("a member tags");
    tag(bo, "parsnips", None).await.expect("another member tags the same label");
    h.settle().await;
    let t = view(&h, cy, &post, "tags");
    assert_eq!(row(&t, "parsnips").unwrap()["count"], 2, "{t}");

    // An untag is the author's own; the other's stands.
    h.node(bo).apply(&post, OP_UNTAG, tags::untag_args("parsnips")).await.expect("a member untags their own");
    h.settle().await;
    assert_eq!(row(&view(&h, cy, &post, "tags"), "parsnips").unwrap()["count"], 1);

    // The label is the hashtag grammar, refused rather than rewritten.
    for bad in ["Parsnips", "seed swap", "#parsnips", ""] {
        assert!(tag(bo, bad, None).await.is_err(), "{bad:?} is refused");
    }

    // Only the owner (a Post has no roles facet) hides; a hidden label stays listed, marked,
    // and never leaves the object.
    assert!(h.node(bo).apply(&post, OP_HIDE_TAG, tags::hide_tag_args("parsnips", true)).await.is_err(), "a member does not hide");
    h.node(ada).apply(&post, OP_HIDE_TAG, tags::hide_tag_args("parsnips", true)).await.expect("the owner hides");
    h.settle().await;
    assert_eq!(row(&view(&h, cy, &post, "tags"), "parsnips").unwrap()["hidden"], true);
    let st = h.with_sync(cy, |n| n.posts()).unwrap().into_iter().find(|(id, _)| *id == post).unwrap().1;
    assert!(tags::outside(&st.tags).is_empty(), "a hidden label is not for the outside");

    // A linked tag: unconfirmed until the linked object's own member writes its half.
    let place = h.mint_object(ada, ObjectKind::Place, "riverside plots", &[]).await;
    tag(ada, "venue", Some(&place)).await.expect("a tag links a held object");
    h.settle().await;
    assert_eq!(row(&view(&h, ada, &post, "tags"), "venue").unwrap()["object"], place.as_str());
    assert_eq!(view(&h, ada, &place, "taggedBy"), serde_json::json!([]), "no half yet: unconfirmed");
    h.node(ada).apply(&place, OP_SET_TAGGED, tags::set_tagged_args(&post, "venue", AT)).await.expect("the Place's member writes its half");
    h.settle().await;
    assert_eq!(view(&h, ada, &place, "taggedBy"), serde_json::json!([{ "object": post, "label": "venue" }]));

    // Node::tag writes both halves in one call, and only for a linked object held here;
    // Node::untag takes the half back with the last tag naming it.
    assert!(h.node(bo).tag(&post, "elsewhere", Some(&place)).await.is_err(), "bo holds no Place: no half, no write");
    assert!(row(&view(&h, ada, &post, "tags"), "elsewhere").is_none(), "nothing was written");
    h.node(ada).tag(&post, "plot", Some(&place)).await.expect("ada holds both");
    h.settle().await;
    let plot = h.with_sync(ada, |n| n.tags_of(&post)).unwrap().into_iter().find(|r| r.label == "plot").unwrap();
    assert!(plot.mine && plot.confirmed && plot.object == place, "{plot:?}");
    h.node(ada).untag(&post, "plot").await.expect("ada untags");
    h.settle().await;
    assert_eq!(view(&h, ada, &place, "taggedBy"), serde_json::json!([{ "object": post, "label": "venue" }]), "the half went with it");

    // The caps, as the ICD states them: per author, then per object. Refused, never evicted.
    assert!(2 * MAX_PER_AUTHOR >= MAX_LIVE, "two authors reach the object's cap");
    let held = |u: usize| {
        let t = view(&h, ada, &post, "tags");
        let me = hex::encode(h.id(u));
        t.as_array().unwrap().iter().filter(|r| r["by"].as_array().unwrap().iter().any(|b| *b == me)).count()
    };
    for i in held(ada)..MAX_PER_AUTHOR {
        tag(ada, &format!("a{i}"), None).await.unwrap();
    }
    h.settle().await;
    assert!(tag(ada, "one_more", None).await.is_err(), "an author's {}th tag is refused", MAX_PER_AUTHOR + 1);
    let live = |h: &Harness| view(h, ada, &post, "tags").as_array().unwrap().len();
    for i in live(&h)..MAX_LIVE {
        tag(bo, &format!("b{i}"), None).await.unwrap();
    }
    h.settle().await;
    assert_eq!(live(&h), MAX_LIVE);
    assert!(tag(cy, "past_the_cap", None).await.is_err(), "the object's {}th label is refused", MAX_LIVE + 1);
    tag(cy, "parsnips", None).await.expect("a label already live is no new label");
    h.settle().await;

    // A device that has never seen the Post folds the same tags from the words alone.
    let before = view(&h, ada, &post, "tags");
    let words = h.with_sync(ada, |n| n.recovery_key()).expect("a minted identity has words");
    let head = h.with_sync(ada, |n| n.head_update().unwrap());
    h.partition(ada);
    let (sim, _) = h
        .sign_in_from_words("ada", "sim", &words, Some(HeadInput { blob: head.sealed, declared_position: head.position }))
        .await
        .unwrap();
    assert_eq!(view(&h, sim, &post, "tags"), before, "the same tags, from the log alone");
    assert_eq!(view(&h, sim, &place, "taggedBy"), serde_json::json!([{ "object": post, "label": "venue" }]));
}
