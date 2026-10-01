//! The lease on a pool leaf (resumption.md §6): numbered cells at the relay, each a
//! first-writer slot, so the relay's arbitration is the compare-and-set.

mod common;

use common::Harness;
use pacific_core::lease::{Act, LEASE_RENEW_BELOW, LEASE_SECS};

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

#[tokio::test]
async fn take_lose_renew_evict_fence() {
    let mut h = Harness::new(&["ada"]).await;
    let phone = 0;
    let laptop = h.add_device("ada", "laptop");
    let obj = h.form_named_forum(phone, "a room");
    let pool = [0x5au8; 32];
    let t0 = now();

    // TAKE from idle; the laptop LOSES, because the leaf is held.
    let o = obj.clone();
    assert!(h.with(phone, |n| async move { n.lease_take(&o, &pool, t0).await.unwrap() }).await);
    let o = obj.clone();
    assert!(!h.with(laptop, |n| async move { n.lease_take(&o, &pool, t0).await.unwrap() }).await);

    // RENEW once it is due, and not before.
    let early = t0 + 60;
    h.with(phone, |n| async move { n.lease_renew_due(early).await.unwrap() }).await;
    let cells = h.with(phone, |n| async move { n.lease_cells(&pool).await.unwrap() }).await;
    assert_eq!(cells.len(), 1, "nothing to renew yet");
    let due = t0 + (LEASE_SECS - LEASE_RENEW_BELOW) + 1;
    h.with(phone, |n| async move { n.lease_renew_due(due).await.unwrap() }).await;
    let cells = h.with(phone, |n| async move { n.lease_cells(&pool).await.unwrap() }).await;
    assert_eq!(cells.len(), 2);
    assert_eq!(cells[1].1.act, Act::Renew);
    assert_eq!(cells[1].1.until, due + LEASE_SECS);

    // Not expired yet: the laptop may not evict.
    assert!(!h.with(laptop, |n| async move { n.lease_evict(&pool, due + 10).await.unwrap() }).await);

    // EVICT after expiry, by another device of the person.
    let later = due + LEASE_SECS + 1;
    assert!(h.with(laptop, |n| async move { n.lease_evict(&pool, later).await.unwrap() }).await);
    let cells = h.with(laptop, |n| async move { n.lease_cells(&pool).await.unwrap() }).await;
    assert_eq!(cells.last().unwrap().1.act, Act::Evict);

    // The stale holder wakes, tries to renew, loses the cell: FENCED.
    h.with(phone, |n| async move { n.lease_renew_due(later).await.unwrap() }).await;
    assert_eq!(h.with_sync(phone, |n| n.fenced().unwrap()), vec![obj.clone()]);

    // An evicted leaf is never taken again (§6.3).
    let o = obj.clone();
    assert!(!h.with(laptop, |n| async move { n.lease_take(&o, &pool, later).await.unwrap() }).await);
}
