//! #68 (Ralph, 9 Nov 2026: "they get everything"): a person's own device that joins an object
//! late, through a pool leaf, holds everything their other devices held (ICD `mls.poolHistory`,
//! `mls.historyPolicy.ownDevices`). A pool leaf reads from the epoch it was added (RFC 9420
//! §9.2), so no row of an object is sent before it has one, and every later leaf is given a
//! segment of what came before it.

use crate::common;

use common::Harness;
use pacific_core::resumption::HeadInput;

fn head(h: &Harness, u: usize) -> Option<HeadInput> {
    let up = h.with_sync(u, |n| n.head_update().unwrap());
    Some(HeadInput { blob: up.sealed, declared_position: up.position })
}

fn texts(h: &Harness, u: usize, obj: &str) -> Vec<String> {
    h.obj_view(u, obj).into_iter().map(|m| m.text).collect()
}

/// The Host's case (RED_SPACES's dump): an object with a second member before any sync sent
/// its rows at once, before upkeep added its pool leaf, and a restore read none of them. Now
/// the rows wait for the leaf, and the restore reads them all.
#[tokio::test]
async fn an_object_with_another_member_waits_for_its_pool_leaf() {
    let mut h = Harness::people(&[("ada", &["phone"]), ("bo", &["sim"])]).await;
    let (phone, bo) = (h.device("ada", "phone"), h.device("bo", "sim"));
    let site = h.mint_group(phone);
    h.group_set_profile(phone, &site, "Mill Road Allotments", "community").await;
    h.add_to_forum(phone, bo, &site).await;
    assert_eq!(h.node(bo).group_state(&site).unwrap().display_name, "Mill Road Allotments", "bo reads it");

    let words = h.with_sync(phone, |n| n.recovery_key()).expect("words");
    let at = head(&h, phone);
    let (sim, _) = h.sign_in_from_words("ada", "restored", &words, at).await.unwrap();
    h.settle().await;
    assert_eq!(
        h.node(sim).group_state(&site).unwrap().display_name,
        "Mill Road Allotments",
        "the restored device reads what was written before any sync"
    );
}

/// Ralph's phone: a device that signs in after another has taken the first pool leaf joins
/// through the refill, added later still. Its segment brings what the leaf cannot read: every
/// post, the other person's included.
#[tokio::test]
async fn a_second_restore_reads_every_post_from_the_start() {
    let mut h = Harness::people(&[("ada", &["phone"]), ("bo", &["sim"])]).await;
    let (phone, bo) = (h.device("ada", "phone"), h.device("bo", "sim"));
    let room = h.form_forum(phone, &[bo]).await;
    h.obj_post(bo, &room, "one, from bo").await;
    h.obj_post(phone, &room, "two, from ada").await;
    h.settle().await;

    let words = h.with_sync(phone, |n| n.recovery_key()).expect("words");
    let at = head(&h, phone);
    let (first, _) = h.sign_in_from_words("ada", "first", &words, at).await.unwrap();
    h.partition(first);
    h.settle_among(&[phone, bo]).await; // the phone refills the pool, with its segment
    h.obj_post(bo, &room, "three, from bo").await;
    h.settle_among(&[phone, bo]).await;

    let at = head(&h, phone);
    let (second, _) = h.sign_in_from_words("ada", "second", &words, at).await.unwrap();
    h.settle_among(&[phone, bo, second]).await;
    let seen = texts(&h, second, &room);
    for want in ["one, from bo", "two, from ada", "three, from bo"] {
        assert!(seen.contains(&want.to_string()), "{want:?} missing from {seen:?}");
    }
}

/// A segment is written once, at its leaf's Add, and a settled mesh writes nothing more: no
/// segment, no restatement, on any tick (the relay-write gate).
#[tokio::test]
async fn a_segment_is_written_once_and_a_settled_mesh_sends_nothing() {
    let mut h = Harness::people(&[("ada", &["phone"]), ("bo", &["sim"])]).await;
    let (phone, bo) = (h.device("ada", "phone"), h.device("bo", "sim"));
    let room = h.form_forum(phone, &[bo]).await;
    h.obj_post(bo, &room, "before").await;
    h.settle().await;

    let words = h.with_sync(phone, |n| n.recovery_key()).expect("words");
    let at = head(&h, phone);
    let (first, _) = h.sign_in_from_words("ada", "first", &words, at).await.unwrap();
    h.partition(first);
    h.settle_among(&[phone, bo]).await;

    let parts: Vec<u32> = h.with_sync(phone, |n| {
        n.spine_entries()
            .unwrap()
            .into_iter()
            .filter_map(|(_, e)| match e.body {
                pacific_core::spine::Body::Pool(p) if p.group_id == hex::decode(&room).unwrap() => Some(p.pools),
                _ => None,
            })
            .flatten()
            .map(|l| l.history)
            .collect()
    });
    assert!(parts.iter().any(|&n| n > 0), "the refill carries a segment: {parts:?}");

    let before = h.with_sync(phone, |n| n.published());
    for _ in 0..3 {
        h.node(phone).sync_once().await.unwrap();
        h.node(bo).sync_once().await.unwrap();
    }
    assert_eq!(h.with_sync(phone, |n| n.published()), before, "a settled mesh publishes nothing");
}
