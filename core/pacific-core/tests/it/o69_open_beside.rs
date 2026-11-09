//! O-69, two Nodes on one open (`Node::open_beside`): what a long-lived embedding holds, a
//! reader and a writer (the iOS FFI). A fold on either is a hit on the other, and a write
//! through either moves the key for both. The binary runs with `PACIFIC_FOLD_CACHE_VERIFY=1`
//! (FC-2), so every hit below is also refolded and compared.

use crate::common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args, FORUM_POST};
use pacific_core::object::ObjectKind;
use pacific_core::Node;

fn cache_on() {
    std::env::set_var("PACIFIC_FOLD_CACHE_MODEL", "o69");
    std::env::set_var("PACIFIC_FOLD_CACHE_VERIFY", "1");
}

fn post(text: &str) -> Args {
    let mut a = Args::new();
    a.insert("text".into(), ArgVal::Text(text.into()));
    a
}

fn counts(n: &Node) -> (u64, u64) {
    let c = n.dir.folds.lock().unwrap();
    (c.hits, c.misses)
}

#[tokio::test]
async fn a_node_beside_shares_its_folds_and_sees_every_write() {
    cache_on();
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let room = h.mint_object(ada, ObjectKind::Forum, "Talk", &[bo]).await;
    h.node(bo).apply(&room, FORUM_POST, post("first")).await.expect("bo posts");
    h.settle().await;

    let reader = h.node(ada);
    let writer = Node::open_beside(&reader.dir.shared().unwrap()).expect("a second Node on the same open");
    assert_eq!(reader.dir.nonce, writer.dir.nonce, "one open, one nonce");

    let v1 = writer.object_view(&room).expect("the writer folds");
    let (h0, m0) = counts(&reader);
    assert_eq!(reader.object_view(&room).unwrap(), v1);
    let (h1, m1) = counts(&reader);
    assert_eq!((h1 - h0, m1 - m0), (1, 0), "the writer's fold is the reader's hit");

    // A write through the writer moves the key the reader reads under.
    writer.apply(&room, FORUM_POST, post("second")).await.expect("ada posts through the writer");
    let v2 = reader.object_view(&room).expect("after the write");
    assert_eq!(counts(&reader).1 - m1, 1, "the writer's write is the reader's miss");
    assert!(v2.contains("second"), "{v2}");
    assert_eq!(v2, h.node(ada).object_view(&room).unwrap(), "the miss gives the fresh fold");
}
