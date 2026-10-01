//! The Door's sign-in (mdr/door.md §4 step 3): a device holding nothing opens the
//! wrap under the passkey's PRF, resumes every object the spine names with the head
//! the auth service holds, and only then has a self record. No test called
//! `Node::restore_from_wrap` before this one (ICD-3).

mod common;

use common::Harness;
use pacific_core::head::ChainVerdict;
use pacific_core::identity::Identity;
use pacific_core::resumption::{HeadInput, Outcome, Resumed};
use pacific_core::Node;

const HOST: &str = "arc.example";
const PRF: [u8; 32] = [0x11; 32];

fn head_of(h: &Harness, u: usize) -> HeadInput {
    let head = h.with_sync(u, |n| n.head_update().unwrap());
    HeadInput { blob: head.sealed, declared_position: head.position }
}

fn outcome_of(r: &Resumed, obj: &str) -> Outcome {
    r.objects.iter().find(|o| o.object == obj).unwrap_or_else(|| panic!("{obj} not named: {r:?}")).outcome.clone()
}

#[tokio::test]
async fn a_wrap_signs_in_resumes_every_object_and_speaks() {
    let mut h = Harness::new(&["ada", "bo"]).await;
    let (phone, bo) = (0, 1);
    let obj = h.form_forum(phone, &[bo]).await;
    h.settle().await; // upkeep provisions a pool leaf into the spine and the forum
    h.obj_post(bo, &obj, "before the sign-in").await;
    h.settle().await;

    let key = h.with_sync(phone, |n| n.identity_key());
    let spine = h.with_sync(phone, |n| n.spine().unwrap()).expect("A2: every identity has a spine");
    let wrap = h.with_sync(phone, |n| n.export_wrap(&PRF, HOST).unwrap());
    let head = head_of(&h, phone);
    let signed_up_at = head.declared_position;
    h.partition(phone);

    let (door, r) = h.sign_in_from_wrap("ada", "door", &PRF, &wrap, HOST, &key, Some(head)).await.unwrap();

    assert_eq!(r.verdict, Some(ChainVerdict::Whole));
    assert_eq!(r.spine.as_deref(), Some(spine.as_str()));
    assert_eq!(r.objects[0].object, spine, "the spine is first (A6.5)");
    assert!(matches!(outcome_of(&r, &obj), Outcome::Joined { .. }), "{r:?}");
    assert_eq!(h.with_sync(door, |n| n.identity_key()), key);
    assert_eq!(
        h.with_sync(door, |n| n.self_object().unwrap()),
        Some(spine.clone()),
        "the self record is the spine's, not a second one"
    );

    // It speaks, and bo reads it; bo speaks, and it reads bo.
    h.obj_post(door, &obj, "from the wrap").await;
    h.settle_among(&[door, bo]).await;
    let bo_sees: Vec<String> = h.obj_view(bo, &obj).into_iter().map(|m| m.text).collect();
    assert!(bo_sees.contains(&"from the wrap".to_string()), "{bo_sees:?}");
    h.obj_post(bo, &obj, "welcome").await;
    h.settle_among(&[door, bo]).await;
    let door_sees: Vec<String> = h.obj_view(door, &obj).into_iter().map(|m| m.text).collect();
    assert!(door_sees.contains(&"welcome".to_string()), "{door_sees:?}");
    assert!(h.with_sync(door, |n| n.noncompliant_objects().unwrap()).is_empty());

    // THE HEAD THE DOOR MUST STORE (NC-26). Its upkeep refilled the leaves it took,
    // and each refill is a spine entry, so the head moved. The next sign-in is
    // judged Whole against the moved head.
    let moved = head_of(&h, door);
    assert!(moved.declared_position > signed_up_at, "{} ≤ {signed_up_at}", moved.declared_position);
    let (_, next) = h.sign_in_from_wrap("ada", "second", &PRF, &wrap, HOST, &key, Some(moved)).await.unwrap();
    assert_eq!(next.verdict, Some(ChainVerdict::Whole));
    assert!(matches!(outcome_of(&next, &obj), Outcome::Joined { .. }), "{next:?}");
}

#[tokio::test]
async fn a_second_sign_in_before_a_refill_mints_no_second_self_record() {
    let mut h = Harness::new(&["ada"]).await;
    let phone = 0;
    h.form_named_forum(phone, "solo room");
    h.settle().await;
    let key = h.with_sync(phone, |n| n.identity_key());
    let wrap = h.with_sync(phone, |n| n.export_wrap(&PRF, HOST).unwrap());
    let head = head_of(&h, phone);
    h.partition(phone);

    let hi = || HeadInput { blob: head.blob.clone(), declared_position: head.declared_position };
    let (a, ra) = h.sign_in_from_wrap("ada", "tablet", &PRF, &wrap, HOST, &key, Some(hi())).await.unwrap();
    let (b, rb) = h.sign_in_from_wrap("ada", "desk", &PRF, &wrap, HOST, &key, Some(hi())).await.unwrap();
    assert!(matches!(ra.objects[0].outcome, Outcome::Joined { .. }), "{ra:?}");
    assert_eq!(rb.objects[0].outcome, Outcome::PoolExhausted, "one idle leaf, and the tablet took it");

    assert!(h.with_sync(a, |n| n.self_object().unwrap()).is_some());
    assert_eq!(
        h.with_sync(b, |n| n.self_object().unwrap()),
        None,
        "a spine the device could not join is reported, not replaced"
    );
    assert!(h.with_sync(b, |n| n.objects_named().unwrap()).is_empty(), "nothing minted");
}

#[tokio::test]
async fn an_account_made_where_no_node_ran_gets_its_self_record_at_first_sign_in() {
    let mut h = Harness::new(&["bo"]).await;
    // wallflowers.io's sign-up page: a seed, a wrap, and no Node, so no spine.
    let seed = [0x42u8; 32];
    let key = Identity::in_memory(seed).identity_key();
    let wrap = pacific_core::wrap::seal(&PRF, &seed, HOST).unwrap();

    let (door, r) = h.sign_in_from_wrap("cy", "door", &PRF, &wrap, HOST, &key, None).await.unwrap();
    assert_eq!(r.spine, None);
    assert!(r.objects.is_empty());
    let own = h.with_sync(door, |n| n.self_object().unwrap()).expect("the first device mints it");

    // Its upkeep writes the spine, so the head moves off zero and has to be stored.
    h.sync(door).await;
    assert_eq!(h.with_sync(door, |n| n.spine().unwrap()), Some(own));
    assert!(h.with_sync(door, |n| n.head_update().unwrap()).position >= 1);
}

#[tokio::test]
async fn a_wrap_that_will_not_sign_in_writes_nothing() {
    let h = Harness::new(&["ada", "bo"]).await;
    let ada = h.with_sync(0, |n| n.identity_key());
    let bo = h.with_sync(1, |n| n.identity_key());
    let wrap = h.with_sync(0, |n| n.export_wrap(&PRF, HOST).unwrap());

    // Each refusal, on a device that holds nothing: a wrong passkey, a wrap lifted
    // onto another host, and a wrap that opens to a key the handle did not name.
    let cases: [(&[u8; 32], &str, &str); 3] =
        [(&[0x22; 32], HOST, ada.as_str()), (&PRF, "arc.attacker.example", ada.as_str()), (&PRF, HOST, bo.as_str())];
    for (prf, host, expect) in cases {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("PACIFIC_STATE_DIR", dir.path());
        let r = Node::sign_in_from_wrap(prf, &wrap, host, "ada", expect, None).await;
        assert!(r.is_err(), "{host} {expect}: signed in");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "{host} {expect}: something was written");
    }
}

#[tokio::test]
async fn a_head_sealed_twice_is_still_the_same_head() {
    // NC-28: every seal takes a fresh nonce, so the bytes of one head differ between
    // two seals, and only opening it can say whether a stored head is this chain's.
    let h = Harness::new(&["ada"]).await;
    let a = h.with_sync(0, |n| n.head_update().unwrap());
    let b = h.with_sync(0, |n| n.head_update().unwrap());
    assert_ne!(a.sealed, b.sealed, "a fresh nonce each seal");
    assert!(h.with_sync(0, |n| n.head_is_mine(&a.sealed, a.position).unwrap()));

    // Once the chain moves on, the older head no longer names it.
    h.form_named_forum(0, "room");
    h.settle().await;
    assert!(h.with_sync(0, |n| n.head_update().unwrap()).position > a.position);
    assert!(!h.with_sync(0, |n| n.head_is_mine(&a.sealed, a.position).unwrap()));
}
