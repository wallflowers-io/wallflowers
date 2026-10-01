//! O-69, the fold cache, through a real Node (mdr/icd-pin.md § O-69). The whole binary runs
//! with `PACIFIC_FOLD_CACHE_VERIFY=1` (FC-2): every hit is refolded and compared, and a
//! difference panics. So each read below is also a check that a hit equals a refold.
//!
//! - repeated reads of one object hit, and give what a fresh fold gives;
//! - another member's post, a new member, each moves the key: the next read misses, and
//!   gives the fresh fold (FC-3);
//! - a fold that fails is the same failure from the cache as from a refold (FC-4).
//!
//! FC-8 (a process with no model folds uncached) is `o69_fold_cache_off.rs`: the model
//! is per process.

mod common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args, FORUM_POST};
use pacific_core::object::ObjectKind;

/// Before anything reads the model or the verify switch: both are read once per process.
fn cache_on() {
    std::env::set_var("PACIFIC_FOLD_CACHE_MODEL", "o69");
    std::env::set_var("PACIFIC_FOLD_CACHE_VERIFY", "1");
}

fn post(text: &str) -> Args {
    let mut a = Args::new();
    a.insert("text".into(), ArgVal::Text(text.into()));
    a
}

fn counts(n: &pacific_core::Node) -> (u64, u64) {
    let c = n.dir.folds.lock().unwrap();
    (c.hits, c.misses)
}

#[tokio::test]
async fn a_hit_equals_a_refold_and_every_change_moves_the_key() {
    cache_on();
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let (ada, bo, cy) = (0, 1, 2);
    let room = h.mint_object(ada, ObjectKind::Forum, "Talk", &[bo]).await;
    for t in ["first", "second"] {
        h.node(bo).apply(&room, FORUM_POST, post(t)).await.expect("bo posts");
    }
    h.settle().await;

    // ONE Node held, as a Door's session holds one: its cache lives as long as it does.
    let n = h.node(ada);
    let v1 = n.object_view(&room).expect("the room's view");
    let (h0, m0) = counts(&n);
    let v2 = n.object_view(&room).expect("again");
    assert_eq!(v1, v2);
    let (h1, m1) = counts(&n);
    assert_eq!((h1 - h0, m1 - m0), (1, 0), "the second read is a hit");
    assert!(v1.contains("second"), "{v1}");

    // Another member's post reaches ada's store: the key moves, the read misses, and it
    // is what a fresh Node folds.
    h.node(bo).apply(&room, FORUM_POST, post("third")).await.expect("bo posts again");
    h.settle().await;
    let _ = h.node(ada); // the process env back on ada; `n` keeps its cache
    let v3 = n.object_view(&room).expect("after the post");
    let (_, m2) = counts(&n);
    assert_eq!(m2 - m1, 1, "a new Delta is a miss");
    assert!(v3.contains("third") && v3 != v1, "{v3}");
    assert_eq!(v3, h.node(ada).object_view(&room).unwrap(), "the miss gives the fresh fold");

    // A new member moves the key too (the roster is a fold input).
    h.add_to_forum(ada, cy, &room).await;
    let _ = h.node(ada);
    let (_, m3) = counts(&n);
    let v4 = n.object_view(&room).expect("after the new member");
    assert_eq!(counts(&n).1 - m3, 1, "a new member is a miss");
    assert_eq!(v4, h.node(ada).object_view(&room).unwrap());
    assert!(n.noncompliant_objects().unwrap().is_empty(), "everything folds, cached or not");

    // FC-4: a row whose signature no longer proves its author fails the fold. The same
    // failure comes from a refold and from the cache, and the object is named.
    let changed = n
        .dir
        .conn
        .execute("UPDATE delta_log SET author_sig = zeroblob(64) WHERE group_id = ?1 AND author_sig IS NOT NULL", [hex::decode(&room).unwrap()])
        .unwrap();
    assert!(changed > 0, "the room holds signed rows");
    let e1 = n.object_view(&room).expect_err("a forged signature fails the fold").to_string();
    let (hb, _) = counts(&n);
    let e2 = n.object_view(&room).expect_err("and fails again, from the cache").to_string();
    assert_eq!(e1, e2, "the same failure, cached or refolded");
    assert_eq!(counts(&n).0 - hb, 1, "the failure is a cached hit, not a success");
    let named: Vec<String> = n.noncompliant_objects().unwrap().into_iter().map(|x| format!("{x:?}")).collect();
    assert!(named.iter().any(|x| x.contains(&room)), "the failing object is named: {named:?}");
}
