//! m29 — every way an account comes to belong to a group, or stops, is on the spine.
//!
//! Until now only the MINT wrote a spine entry, so a recovering account could name
//! what it had made and nothing it had been invited to — no group it joined, no
//! connection. And nothing recorded leaving, so a recovering device had no way to
//! know which of the groups it named were ones it had already exited.
//!
//! WHAT THIS PINS, one door per test:
//!   - a JOIN through a Welcome writes a `Group` entry at the epoch it joined, and
//!     that epoch is the account's floor: `recoverability` lists nothing below it,
//!     because an epoch before you arrived was never yours to lose;
//!   - a CONNECTION writes one on each side — the scanner, who created the group,
//!     from epoch 0, and the scanned, who joined it, from theirs;
//!   - a person's SECOND DEVICE joining a group the account is already in writes
//!     nothing, because the account did not join anything;
//!   - LEAVING — removed, or by its own choice — writes a `Left` entry at the last
//!     epoch the account was a member, which is what lets a recovering device skip
//!     folding the group;
//!   - a REJOIN after leaving is a new `Group` entry, and the object is no longer
//!     left.

mod common;

use common::Harness;
use pacific_core::spine::Body;

/// The (index, body) pairs this device holds that name `obj`.
fn about(h: &Harness, u: usize, obj: &str) -> Vec<Body> {
    let gid = hex::decode(obj).unwrap();
    h.node(u)
        .spine_entries()
        .unwrap()
        .into_iter()
        .map(|(_, e)| e.body)
        .filter(|b| b.group_id() == gid.as_slice())
        .collect()
}

fn joined_at(bodies: &[Body]) -> Vec<u64> {
    bodies
        .iter()
        .filter_map(|b| match b {
            Body::Group(j) => Some(j.first_epoch),
            _ => None,
        })
        .collect()
}

fn left_at(bodies: &[Body]) -> Vec<u64> {
    bodies
        .iter()
        .filter_map(|b| match b {
            Body::Left(d) => Some(d.last_epoch),
            _ => None,
        })
        .collect()
}

async fn remove(h: &Harness, owner: usize, obj: &str, member: usize) {
    let (o, m) = (obj.to_string(), hex::encode(h.id(member)));
    h.with(owner, |n| async move { n.group_remove_member(&o, &m, None).await })
        .await
        .expect("the owner's remove");
    h.settle().await;
}

/// A member added late joins at a later epoch, writes it, and that is its floor.
#[tokio::test]
async fn a_late_joiner_is_named_from_the_epoch_it_joined() {
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let (ada, bo, cy) = (0, 1, 2);
    let obj = h.form_forum(ada, &[bo]).await;
    h.add_to_forum(ada, cy, &obj).await;

    // The owner minted it, from epoch 0.
    assert_eq!(joined_at(&about(&h, ada, &obj)), vec![0], "the mint is the owner's entry");

    let floor = joined_at(&about(&h, cy, &obj));
    assert_eq!(floor.len(), 1, "cy's join is one entry: {floor:?}");
    assert!(floor[0] > 0, "cy was added after bo, so it joined past epoch 0: {floor:?}");

    let r = h.node(cy).recoverability(&obj).unwrap();
    assert_eq!(r.first_epoch, Some(floor[0]));
    assert!(
        !r.unreadable_epochs.iter().any(|&e| e < floor[0]),
        "nothing below the floor is listed as unreadable — before cy arrived was never \
         cy's to lose: floor {} vs {:?}",
        floor[0],
        r.unreadable_epochs
    );
    assert_eq!(r.unreadable_epochs.first(), Some(&floor[0]), "and the list starts AT the floor");
    assert_eq!(r.left_epoch, None);
}

/// A connection is a group of two, and each side names it.
///
/// Found by what the two sides SHARE rather than by counting: each spine also
/// names that account's own self record, which the other side never sees. The
/// connection is the one group both spines name.
#[tokio::test]
async fn a_connection_is_named_on_both_sides() {
    let h = Harness::new(&["ana", "ben"]).await;
    let (ana, ben) = (0, 1);
    h.pair(ana, ben).await; // ben scans ana: ben creates, ana joins

    let named = |u: usize| -> Vec<(Vec<u8>, u64)> {
        h.node(u)
            .spine_entries()
            .unwrap()
            .into_iter()
            .filter_map(|(_, e)| match e.body {
                Body::Group(j) => Some((j.group_id, j.first_epoch)),
                _ => None,
            })
            .collect()
    };
    let (by_ben, by_ana) = (named(ben), named(ana));
    let shared: Vec<&(Vec<u8>, u64)> = by_ben
        .iter()
        .filter(|(g, _)| by_ana.iter().any(|(a, _)| a == g))
        .collect();
    assert_eq!(shared.len(), 1, "exactly one group is on both spines — the connection: ben {by_ben:?} ana {by_ana:?}");

    let conn = &shared[0].0;
    let ben_floor = shared[0].1;
    let ana_floor = by_ana.iter().find(|(g, _)| g == conn).unwrap().1;
    assert_eq!(ben_floor, 0, "the scanner created it, so it was there at 0");
    assert!(ana_floor >= ben_floor, "the scanned side joined it, no earlier");
}

/// Adding a person's second device to a group the account is already in is not
/// the account joining anything, and writes nothing.
#[tokio::test]
async fn a_second_device_joining_writes_no_second_entry() {
    let mut h = Harness::new(&["ana"]).await;
    let phone = 0;
    let laptop = h.add_device("ana", "laptop");
    let obj = h.node(phone).object_new("forum", "ours").expect("mint");
    h.add_to_forum(phone, laptop, &obj).await;

    assert_eq!(h.device_leaves(laptop, &obj), 2, "both devices are in the group");
    assert_eq!(joined_at(&about(&h, phone, &obj)), vec![0], "the phone minted it");
    // The account's spine is the chain on the relay, and a walk from either device
    // records what it finds — so the claim is about the CHAIN: one entry, not two.
    h.settle().await;
    let gid = hex::decode(&obj).unwrap();
    let walk = h.with(laptop, |n| async move { n.chain_walk(0).await.unwrap() }).await;
    let entries = walk
        .entries
        .iter()
        .filter(|(_, e)| matches!(&e.body, Body::Group(j) if j.group_id == gid))
        .count();
    assert_eq!(
        entries, 1,
        "the laptop joined a group its account already belonged to — no second entry"
    );
}

/// Removed: the member's device records the last epoch it could read.
#[tokio::test]
async fn a_removal_is_recorded_at_the_last_epoch_the_member_had() {
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let obj = h.form_forum(ada, &[bo]).await;
    let before = h.node(bo).recoverability(&obj).unwrap().current_epoch;

    remove(&h, ada, &obj, bo).await;
    assert!(h.node(bo).is_departed(&obj).unwrap(), "bo's device learned it was removed");

    let bodies = about(&h, bo, &obj);
    assert_eq!(left_at(&bodies), vec![before], "the departure, at the epoch bo was removed from");
    assert_eq!(h.node(bo).recoverability(&obj).unwrap().left_epoch, Some(before));
    assert!(left_at(&about(&h, ada, &obj)).is_empty(), "the owner did not leave");
}

/// Leaving by one's own choice goes the same way.
#[tokio::test]
async fn leaving_by_choice_is_recorded_too() {
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let obj = h.form_forum(ada, &[bo]).await;

    let o = obj.clone();
    h.with(bo, |n| async move { n.group_leave(&o).await }).await.expect("the leave");
    h.settle().await;

    assert!(h.node(bo).is_departed(&obj).unwrap(), "the leave was committed and bo is out");
    assert_eq!(left_at(&about(&h, bo, &obj)).len(), 1, "one departure on bo's spine");
}

/// Re-added after leaving: a new join, and the object is no longer left.
#[tokio::test]
async fn a_rejoin_is_a_new_entry_and_clears_the_departure() {
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let obj = h.form_forum(ada, &[bo]).await;
    remove(&h, ada, &obj, bo).await;
    h.add_to_forum(ada, bo, &obj).await;

    let bodies = about(&h, bo, &obj);
    let joins = joined_at(&bodies);
    let lefts = left_at(&bodies);
    assert_eq!(joins.len(), 2, "the first join and the rejoin: {bodies:?}");
    assert_eq!(lefts.len(), 1);
    assert!(joins[1] > lefts[0], "the rejoin is after the departure: {bodies:?}");

    let r = h.node(bo).recoverability(&obj).unwrap();
    assert_eq!(r.left_epoch, None, "rejoined, so not left");
    assert_eq!(r.first_epoch, Some(joins[0]), "the floor is still the first join");
}
