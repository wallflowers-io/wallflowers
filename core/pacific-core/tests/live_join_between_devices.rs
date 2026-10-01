//! LIVE SYNC BETWEEN A PERSON'S DEVICES (Ralph, 27 Sep 2026: "Live sync between sessions
//! is required"). Software Testing's E12 (str.md run 35): an object one open session
//! made reached the same person's other open session only when it signed in again,
//! because only `resume` joins a new object, and it ran only at sign-in.
//!
//! The object is named on the self record, which both devices hold; the other device's
//! next sync joins it, through its pool leaf, with no sign-in.

mod common;

use common::Harness;
use pacific_core::object::ObjectKind;

#[tokio::test]
async fn an_object_one_device_makes_reaches_the_other_on_its_next_sync() {
    let h = Harness::people(&[("ada", &["phone", "laptop"])]).await;
    let (phone, laptop) = (h.device("ada", "phone"), h.device("ada", "laptop"));

    // Both hold the account: the phone has its self record, and the laptop resumes
    // into it from the spine, as a device that signed in does.
    h.with_sync(phone, |n| n.ensure_self_object()).expect("the phone's self record");
    h.sync(phone).await;
    h.sync(phone).await;
    h.with(laptop, |n| async move { n.resume(None).await }).await.expect("the laptop resumes");
    h.sync(laptop).await;
    let record = h.with_sync(phone, |n| n.self_object()).unwrap().expect("a self record");
    assert_eq!(
        h.with_sync(laptop, |n| n.self_object()).unwrap().as_deref(),
        Some(record.as_str()),
        "the laptop holds the phone's self record"
    );

    // The phone makes a group, and ONLY the phone syncs: its syncs name it on the self
    // record and provision the pool leaf a second device joins through.
    let draft = pacific_core::mint::MintDraft { name: "made on the phone".into(), ..Default::default() };
    let obj = h.with(phone, |n| async move { n.mint(ObjectKind::Group, &draft).await }).await.expect("the phone mints");
    h.sync(phone).await;
    h.sync(phone).await;
    assert!(
        h.with_sync(laptop, |n| n.object_kind(&obj)).is_err(),
        "the precondition: the laptop does not hold it yet"
    );

    // The laptop's next sync, and no sign-in.
    h.sync(laptop).await;
    assert_eq!(
        h.with_sync(laptop, |n| n.object_kind(&obj)).expect("the laptop holds what the phone made"),
        "group"
    );
    assert!(
        h.with_sync(laptop, |n| n.unheld_joined()).unwrap().is_empty(),
        "nothing the record names is left unheld"
    );
}
