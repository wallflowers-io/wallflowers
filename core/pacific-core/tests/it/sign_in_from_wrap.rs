//! The Door's sign-in (mdr/door.md §4 step 3): a device holding nothing opens the
//! wrap under the passkey's PRF, resumes every object the spine names with the head
//! the auth service holds, and only then has a self record. No test called
//! `Node::restore_from_wrap` before this one (ICD-3).

use crate::common;

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

/// The iOS words door: a device holding nothing signs in from the 24 words alone,
/// rejoins every object the spine names, and keeps the spine's own self record:
/// after a sync pass too, which is where an identity-only restore minted a second.
#[tokio::test]
async fn the_words_sign_in_resume_and_mint_no_second_self_record() {
    let mut h = Harness::new(&["ada", "bo"]).await;
    let (phone, bo) = (0, 1);
    let obj = h.form_forum(phone, &[bo]).await;
    h.settle().await;
    h.obj_post(bo, &obj, "before the words").await;
    h.settle().await;

    let key = h.with_sync(phone, |n| n.identity_key());
    let spine = h.with_sync(phone, |n| n.spine().unwrap()).expect("A2: every identity has a spine");
    let words = h.with_sync(phone, |n| n.recovery_key()).expect("a minted identity has words");
    let head = head_of(&h, phone);
    h.partition(phone);

    let (sim, r) = h.sign_in_from_words("ada", "sim", &words, Some(head)).await.unwrap();

    assert_eq!(r.verdict, Some(ChainVerdict::Whole));
    assert_eq!(r.objects[0].object, spine, "the spine is first (A6.5)");
    assert!(matches!(outcome_of(&r, &obj), Outcome::Joined { .. }), "{r:?}");
    assert_eq!(h.with_sync(sim, |n| n.identity_key()), key);
    assert_eq!(h.with_sync(sim, |n| n.self_object().unwrap()), Some(spine.clone()));

    // A sync pass reconciles the self profile: the step that minted a second
    // record after an identity-only restore (simulator, 28 Oct, group 5a36e81c…).
    h.settle_among(&[sim, bo]).await;
    assert_eq!(
        h.with_sync(sim, |n| n.self_object().unwrap()),
        Some(spine.clone()),
        "still the spine's own record after a sync"
    );

    h.obj_post(sim, &obj, "from the words").await;
    h.settle_among(&[sim, bo]).await;
    let bo_sees: Vec<String> = h.obj_view(bo, &obj).into_iter().map(|m| m.text).collect();
    assert!(bo_sees.contains(&"from the words".to_string()), "{bo_sees:?}");
}

/// The `pacific::resume` lines, as a subscriber on this thread sees them.
#[derive(Clone, Default)]
struct Said(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Said {
    fn on_event(&self, e: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        struct Message(String);
        impl tracing::field::Visit for Message {
            fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
                if f.name() == "message" {
                    self.0 = format!("{v:?}");
                }
            }
        }
        if e.metadata().target() == "pacific::resume" {
            let mut m = Message(String::new());
            e.record(&mut m);
            self.0.lock().unwrap().push(m.0);
        }
    }
}

/// A sign-in says what it reached as it goes: the walk, then one line per object the
/// spine names, with the outcome `Resumed` carries for it.
#[tokio::test]
async fn a_sign_in_says_what_it_reached() {
    use tracing_subscriber::layer::SubscriberExt;
    let mut h = Harness::new(&["ada", "bo"]).await;
    let (phone, bo) = (0, 1);
    let obj = h.form_forum(phone, &[bo]).await;
    h.settle().await;
    let key = h.with_sync(phone, |n| n.identity_key());
    let wrap = h.with_sync(phone, |n| n.export_wrap(&PRF, HOST).unwrap());
    let head = head_of(&h, phone);
    h.partition(phone);

    let said = Said::default();
    let _on = tracing::subscriber::set_default(tracing_subscriber::registry().with(said.clone()));
    let (_, r) = h.sign_in_from_wrap("ada", "said", &PRF, &wrap, HOST, &key, Some(head)).await.unwrap();

    let lines = said.0.lock().unwrap().clone();
    let n = r.objects.len();
    assert!(n >= 2, "the spine and the forum: {r:?}");
    assert_eq!(lines.len(), n + 1, "the walk, then each object: {lines:#?}");
    assert!(lines[0].ends_with(&format!("{n} objects")), "{}", lines[0]);
    for (i, (line, o)) in lines[1..].iter().zip(&r.objects).enumerate() {
        assert!(line.starts_with(&format!("{}/{n} {} {}: ", i + 1, o.kind, &o.object[..8])), "{line}");
        assert!(line.ends_with(&format!("{:?}", o.outcome)), "{line}");
    }
    assert!(lines.iter().any(|l| l.contains(&obj[..8])), "the forum is named: {lines:#?}");
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
