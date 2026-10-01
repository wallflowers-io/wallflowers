//! The claim (resumption.md §0): a device holding only the seed, with no other
//! device of the person online, reaches every object the person is in, reads from
//! each pool leaf's join epoch, and speaks — publishing no commit, proposal or
//! Welcome to any group while it resumes.

mod common;

use common::Harness;
use pacific_core::resumption::{HeadInput, Outcome};

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

#[tokio::test]
async fn a_device_with_only_the_words_resumes_and_speaks() {
    let mut h = Harness::new(&["ada", "bo"]).await;
    let (phone, bo) = (0, 1);
    let obj = h.form_forum(phone, &[bo]).await;
    h.settle().await; // upkeep provisions a pool leaf into the spine and the forum
    h.obj_post(bo, &obj, "before the new device").await;
    h.settle().await;

    let spine = h.with_sync(phone, |n| n.spine().unwrap()).expect("A2: every identity has a spine");
    assert!(h.with_sync(phone, |n| n.spine_pending()).is_empty(), "one sync drains the queue");
    let head = h.with_sync(phone, |n| n.head_update().unwrap());

    // Every other device of the person is offline.
    h.partition(phone);
    let laptop = h.add_device("ada", "laptop");
    let input = HeadInput { blob: head.sealed.clone(), declared_position: head.position };
    let r = h.with(laptop, |n| async move { n.resume_at(Some(input), now()).await.unwrap() }).await;

    assert_eq!(r.verdict, Some(pacific_core::head::ChainVerdict::Whole));
    assert_eq!(r.spine.as_deref(), Some(spine.as_str()));
    assert_eq!(r.objects[0].object, spine, "the spine is first (A6.5)");
    for o in &r.objects {
        assert!(matches!(o.outcome, Outcome::Joined { .. }), "{o:?}");
    }
    assert!(r.objects.iter().any(|o| o.object == obj), "the forum is named: {:?}", r.objects);
    assert_eq!(h.with_sync(laptop, |n| n.spine().unwrap()), Some(spine), "resume set the spine");

    // It speaks, and bo reads it; bo speaks, and it reads bo.
    h.obj_post(laptop, &obj, "from the words alone").await;
    h.settle_among(&[laptop, bo]).await;
    let bo_sees: Vec<String> = h.obj_view(bo, &obj).into_iter().map(|m| m.text).collect();
    assert!(bo_sees.contains(&"from the words alone".to_string()), "{bo_sees:?}");
    h.obj_post(bo, &obj, "welcome back").await;
    h.settle_among(&[laptop, bo]).await;
    let lap_sees: Vec<String> = h.obj_view(laptop, &obj).into_iter().map(|m| m.text).collect();
    assert!(lap_sees.contains(&"welcome back".to_string()), "{lap_sees:?}");

    // The people did not change; the laptop is a leaf of ada's.
    assert_eq!(h.roster_people(bo, &obj).len(), 2);
    assert!(h.with_sync(laptop, |n| n.noncompliant_objects().unwrap()).is_empty());
}

#[tokio::test]
async fn a_second_new_device_finds_the_pool_exhausted_until_it_is_refilled() {
    let mut h = Harness::new(&["ada"]).await;
    let phone = 0;
    let obj = h.form_named_forum(phone, "solo room");
    h.settle().await;
    let head = h.with_sync(phone, |n| n.head_update().unwrap());
    h.partition(phone);

    let a = h.add_device("ada", "tablet");
    let b = h.add_device("ada", "desk");
    let hi = || HeadInput { blob: head.sealed.clone(), declared_position: head.position };
    let ra = h.with(a, |n| { let i = hi(); async move { n.resume_at(Some(i), now()).await.unwrap() } }).await;
    let rb = h.with(b, |n| { let i = hi(); async move { n.resume_at(Some(i), now()).await.unwrap() } }).await;
    let of = |r: &pacific_core::resumption::Resumed| r.objects.iter().find(|o| o.object == obj).unwrap().outcome.clone();
    assert!(matches!(of(&ra), Outcome::Joined { .. }));
    assert_eq!(of(&rb), Outcome::PoolExhausted, "one idle leaf per object, and a took it");

    // a's next ordinary sync refills; then b gets in.
    h.sync(a).await;
    let head = h.with_sync(a, |n| n.head_update().unwrap());
    let i = HeadInput { blob: head.sealed.clone(), declared_position: head.position };
    let rb = h.with(b, |n| async move { n.resume_at(Some(i), now()).await.unwrap() }).await;
    assert!(matches!(of(&rb), Outcome::Joined { .. }), "{rb:?}");
}
