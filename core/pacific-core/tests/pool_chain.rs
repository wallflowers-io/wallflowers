//! The chain is the spine (resumption.md A1): arithmetic addresses, and every
//! append takes its index's first-writer slot, so two devices of one person can
//! race for the same index and neither entry is lost.

mod common;

use common::Harness;
use pacific_core::spine::{Body, Departed};

fn left(n: u8) -> Body {
    Body::Left(Departed { group_id: vec![n; 32], last_epoch: n as u64 })
}

#[tokio::test]
async fn a_lost_append_rewalks_and_lands_after_the_winner() {
    let mut h = Harness::new(&["ada"]).await;
    let phone = 0;
    let laptop = h.add_device("ada", "laptop");
    h.settle().await;

    let before = h.with(phone, |n| async move { n.chain_walk(0).await.unwrap() }).await;

    // The phone queues an entry at its local next index and does not publish it.
    let obj = h.form_named_forum(phone, "queued, not sent");
    let _ = obj;

    // The laptop appends first, at the same index.
    let hu = h
        .with(laptop, |n| async move { n.chain_append(vec![left(7)]).await.unwrap() })
        .await;
    assert!(hu.position > before.position, "the laptop appended");

    // The phone's drain loses that slot, re-walks, and lands after the winner.
    h.sync(phone).await;
    let after = h.with(phone, |n| async move { n.chain_walk(0).await.unwrap() }).await;
    assert!(after.position >= before.position + 2, "both entries are on the chain: {:?}", after.entries);
    let bodies: Vec<&Body> = after.entries.iter().map(|(_, e)| &e.body).collect();
    assert!(bodies.contains(&&left(7)), "the laptop's entry survived");
    let indices: Vec<u64> = after.entries.iter().map(|(i, _)| *i).collect();
    assert_eq!(indices, (0..after.position).collect::<Vec<_>>(), "no hole, no duplicate");

    // And the head each device would store now agrees with the walk.
    let head = h.with_sync(phone, |n| n.head_update().unwrap());
    assert_eq!(head.position, after.position);
    assert_eq!(head.tail, after.tail);
}

#[tokio::test]
async fn an_empty_chain_walks_to_nothing() {
    let h = Harness::new(&["bo"]).await;
    let w = h.with(0, |n| async move { n.chain_walk(9).await.unwrap() }).await;
    assert_eq!(w.position, 0);
    assert_eq!(w.tail, pacific_core::head::NO_TAIL);
}
