//! m30 — the self record: a group of one, at the base of the spine.
//!
//! Ruled 23 September 2026. An account's profile lived in the `me` table, which is
//! device-local SQLite — the "local table" the doctrine names as scaffolding, and
//! it failed the one question: a device that has never seen it got the key back
//! and not the name. The record is a real GroupObject, so the profile is a folded
//! Delta like everything else.
//!
//! WHAT THIS DOES NOT CLAIM, and m28 says the same thing about its own step: the
//! record is NAMED from the seed and is neither readable nor speakable there. The
//! archive key records and seed-reachable MLS state are still unwritten, so a
//! device holding only the words finds the record's id and cannot open it. The
//! last test here pins exactly that, so nobody reads "portable" into this file.

use crate::common;

use common::Harness;
use pacific_core::group::{ContactCard, GroupShape};
use pacific_core::resumption::HeadInput;

/// The record exists from the identity's first breath, and it is the FIRST
/// vertebra — everything else hangs off a spine whose base is the person.
#[tokio::test]
async fn an_identity_mints_its_own_record_at_the_base_of_the_spine() {
    let h = Harness::new(&["Ana"]).await;
    let n = h.node(0);

    let me = n.self_object().expect("read").expect("an identity has a record");
    let spine = n.spine_entries().expect("spine");

    assert_eq!(spine.len(), 1, "the record is the only object yet: {spine:?}");
    assert_eq!(spine[0].0, 0, "it takes index 0");
    assert_eq!(
        hex::encode(spine[0].1.body.group_id()),
        me,
        "the base of the spine IS the self record"
    );
    assert_eq!(
        n.object_members(&me).unwrap().len(),
        1,
        "a group of one: them"
    );
}

/// The profile is a Delta on that record, not a row. `my_profile` reads the fold.
#[tokio::test]
async fn the_profile_is_folded_from_the_record() {
    let h = Harness::new(&["Ana"]).await;

    let card = ContactCard { org: "Wallflowers".into(), ..Default::default() };
    h.node(0)
        .set_my_profile("Ana Ruiz", GroupShape::Individual, &card)
        .await
        .expect("set the profile");

    let n = h.node(0);
    let me = n.self_object().unwrap().unwrap();
    let view = n.object_view(&me).expect("the record folds");
    let v: serde_json::Value = serde_json::from_str(&view).unwrap();

    assert_eq!(v["display_name"], "Ana Ruiz", "folded from the record: {view}");
    assert_eq!(v["shape"], "individual");

    let (name, shape, got) = n.my_profile().expect("read it back");
    assert_eq!(name, "Ana Ruiz");
    assert_eq!(shape, GroupShape::Individual);
    assert_eq!(got.org, "Wallflowers", "the card rides the same delta");
}

/// Every object the spine names, the record names — the readable half of the
/// same fact, and the only enumeration a client can fold.
#[tokio::test]
async fn the_record_names_every_object_the_spine_names() {
    let h = Harness::new(&["Ana"]).await;
    let forum = h.node(0).object_new("forum", "thursday").expect("mint");
    let thing = h.node(0).object_new("thing", "a ladder").expect("mint");

    h.sync(0).await;

    let n = h.node(0);
    let me = n.self_object().unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_str(&n.object_view(&me).unwrap()).unwrap();
    let named: Vec<String> = v["joined"]
        .as_array()
        .expect("the group view carries joined")
        .iter()
        .map(|j| j["object"].as_str().unwrap_or_default().to_string())
        .collect();

    assert!(named.contains(&forum), "the forum is named: {named:?}");
    assert!(named.contains(&thing), "the thing is named: {named:?}");
    assert!(
        !named.contains(&me),
        "the record does not name itself — it is what does the naming"
    );

    // The two halves agree, which is the whole point of reconciling them.
    let spined: Vec<String> = n
        .spine_entries()
        .unwrap()
        .iter()
        // The JOINS: a `Pool` entry (resumption.md A1) names an object the spine
        // already joined, and the fold's `joined` is the list of joins.
        .filter(|(_, e)| matches!(e.body, pacific_core::spine::Body::Group(_)))
        .map(|(_, e)| hex::encode(e.body.group_id()))
        .filter(|o| *o != me)
        .collect();
    let mut a = named.clone();
    let mut b = spined.clone();
    a.sort();
    b.sort();
    assert_eq!(a, b, "the fold and the spine name the same set");
}

/// Reconciling twice authors nothing the second time.
#[tokio::test]
async fn naming_is_idempotent() {
    let h = Harness::new(&["Ana"]).await;
    h.node(0).object_new("forum", "one").expect("mint");
    h.sync(0).await;

    let n = h.node(0);
    assert_eq!(
        n.reconcile_joined().await.expect("reconcile"),
        0,
        "the sync already named it; a second pass writes nothing"
    );
}

/// THE RECORD IS NAMED AND IT IS NOT YET PORTABLE. Same three gaps m28 names,
/// stated here so this file cannot be read as the migration finishing.
#[tokio::test]
async fn the_record_is_nameable_and_not_yet_recoverable() {
    let h = Harness::new(&["Ana"]).await;
    h.sync(0).await;

    let n = h.node(0);
    let me = n.self_object().unwrap().unwrap();
    let r = n.recoverability(&me).expect("recoverability");

    assert!(r.named, "the drain published the base vertebra");
    assert!(
        !r.recoverable(),
        "a device holding only the words finds this record's id and cannot open \
         it: no archive key record, no seed-reachable MLS state — {r:?}"
    );
    assert!(!r.speakable);
}

/// THE 28 OCTOBER CASE (RCA, 8 Nov). A device recovered from the words joins the record
/// and writes to it while the first device has written since. On Ralph's record the two
/// took one seat, the fold stopped, and every sync re-named every object for a week. The
/// relay's slot now orders them (ICD `mls.sequencing.slot`): the record folds on both,
/// says the same on both, and a settled record is not written again.
#[tokio::test]
async fn a_recovered_device_does_not_fork_a_record_written_since() {
    let mut h = Harness::new(&["Ana"]).await;
    let phone = 0;
    h.node(phone).object_new("forum", "thursday").expect("mint");
    h.sync(phone).await;
    let words = h.with_sync(phone, |n| n.recovery_key()).expect("a minted identity has words");
    let head = h.with_sync(phone, |n| n.head_update().unwrap());

    h.node(phone)
        .set_my_profile("Ana Ruiz", GroupShape::Individual, &ContactCard::default())
        .await
        .expect("the phone writes to the record");
    let (sim, _) = h
        .sign_in_from_words("Ana", "sim", &words, Some(HeadInput { blob: head.sealed, declared_position: head.position }))
        .await
        .expect("the sim signs in from the words");
    h.node(sim)
        .set_my_profile("Ana R.", GroupShape::Individual, &ContactCard::default())
        .await
        .expect("and writes to it too");
    h.settle().await;
    h.settle().await;

    let me = h.node(phone).self_object().unwrap().unwrap();
    assert_eq!(h.node(sim).self_object().unwrap().unwrap(), me, "one record, on both");
    let name = |u: usize| {
        let view = h.node(u).object_view(&me).unwrap_or_else(|e| panic!("{} cannot fold the record: {e}", h.device_name(u)));
        serde_json::from_str::<serde_json::Value>(&view).unwrap()["display_name"].as_str().unwrap_or_default().to_string()
    };
    assert_eq!(name(phone), name(sim), "one record says one thing");
    assert!(["Ana Ruiz", "Ana R."].contains(&name(phone).as_str()), "{}", name(phone));

    let rows = |u: usize| h.node(u).object_log(&me).unwrap().len();
    let settled = (rows(phone), rows(sim));
    h.settle().await;
    assert_eq!((rows(phone), rows(sim)), settled, "a settled record is not written again");
}

/// A RECORD THAT WILL NOT FOLD IS NOT RE-NAMED ON EVERY SYNC. Read as an empty list, a
/// failed fold had each pass name every object again onto the log that would not fold.
#[tokio::test]
async fn a_record_that_will_not_fold_is_not_written_to() {
    let h = Harness::new(&["Ana"]).await;
    h.node(0).object_new("forum", "one").expect("mint");
    h.sync(0).await;
    let n = h.node(0);
    let me = n.self_object().unwrap().unwrap();

    // A link no fork explains: a sequenced delta chained to one this device never held.
    let mut a = pacific_core::coordinator::Args::new();
    a.insert("displayName".into(), pacific_core::coordinator::ArgVal::Text("from nowhere".into()));
    a.insert("shape".into(), pacific_core::coordinator::ArgVal::Text("individual".into()));
    let orphan = pacific_core::coordinator::sequenced_delta(
        pacific_core::object::ObjectKind::Group.type_id() as u32,
        pacific_core::group::OP_SET_PROFILE,
        a,
        0,
        999,
        [0xEE; 32],
    );
    // Stored as received (sent, so not this device's to author again), unsigned, as
    // compliance_noncompliant_objects_are_named does.
    let gid = hex::decode(&me).unwrap();
    n.dir.append_delta(&gid, &orphan.id(), &n.id.identity_pk(), &orphan.canonical_bytes(), 0).unwrap();
    n.dir.mark_delivered(&gid, &orphan.id()).unwrap();
    assert!(n.object_view(&me).is_err(), "the record no longer folds");

    let before = n.object_log(&me).unwrap().len();
    let err = n.reconcile_joined().await.expect_err("nothing is named on a record that will not fold");
    assert!(format!("{err}").contains("ChainBroken"), "{err}");
    h.sync(0).await;
    h.sync(0).await;
    let log = n.object_log(&me).unwrap();
    assert_eq!(log.len(), before, "and no sync writes to it: {:?}", &log[before.min(log.len())..]);
}
