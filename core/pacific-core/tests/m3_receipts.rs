//! Delivery + read receipts (WhatsApp's ✓ / ✓✓ / ✓✓-blue) end-to-end through the
//! real two-device harness and relay — not the isolated coordinator fold, but the
//! whole path: author a message, the recipient's `sync` auto-emits a DELIVERY
//! receipt, opening the chat emits a READ receipt, and the sender's own view climbs
//! sent → delivered → read as those receipts fold back.
//!
//! `receipt` is filled per-device for the reader's OWN messages only: 1 = sent,
//! 2 = delivered to every recipient, 3 = read by every recipient.

mod common;

use common::Harness;

/// The receipt code on the first message `u` sees in the DM with `peer`.
fn first_receipt(h: &Harness, u: usize, peer: usize) -> u8 {
    h.dm_view(u, peer).first().expect("a message").receipt
}

#[tokio::test]
async fn dm_receipt_climbs_sent_then_delivered_then_read() {
    let h = Harness::new(&["Nye Thompson", "Harvey Deane"]).await;
    let (a, b) = (0, 1);
    h.pair(a, b).await;

    // A sends. Before B has seen it, A's own bubble is at most "sent" (1) — nobody
    // has acknowledged it yet.
    h.dm_post(a, b, "Hey Nye").await;
    h.sync(a).await;
    assert_eq!(first_receipt(&h, a, b), 1, "sent — no receipts back yet");

    // B syncs: it now HOLDS the message, and `sync` auto-emits a DELIVERY receipt.
    h.sync(b).await;
    assert_eq!(
        first_receipt(&h, b, a),
        0,
        "B's view of A's message carries no receipt (it isn't B's)"
    );

    // A syncs and folds B's delivery receipt → A's bubble shows ✓✓ (delivered, 2).
    h.sync(a).await;
    assert_eq!(first_receipt(&h, a, b), 2, "delivered to B");

    // B opens the conversation → emits a READ receipt. A folds it → ✓✓ blue (3).
    let n = h.dm_mark_read(b, a).await;
    assert_eq!(n, 1, "one unread message → one read receipt");
    h.sync(a).await;
    assert_eq!(first_receipt(&h, a, b), 3, "read by B");
}

#[tokio::test]
async fn note_to_self_shows_as_read() {
    // A group with only you (a self-chat / notes-to-self) has no other recipient, so
    // "delivered to / read by everyone" is vacuously true and you authored it — your
    // own message reads as ✓✓-blue (3), not a stuck single ✓.
    let h = Harness::new(&["Nye Thompson"]).await;
    let obj = h.form_forum(0, &[]).await; // solo forum, no other members
    h.obj_post(0, &obj, "remember the milk").await;

    let view = h.obj_view(0, &obj);
    assert_eq!(view.len(), 1);
    assert_eq!(view[0].receipt, 3, "a self-chat message reads as read");
}

#[tokio::test]
async fn a_burst_of_messages_is_acknowledged_in_one_batch() {
    // Signal-style batching, end-to-end: A fires three messages; a single sync by B
    // delivers all three, and one open reads all three. Every one of A's bubbles
    // reaches ✓✓-blue.
    let h = Harness::new(&["Nye Thompson", "Harvey Deane"]).await;
    let (a, b) = (0, 1);
    h.pair(a, b).await;

    h.dm_post(a, b, "one").await;
    h.dm_post(a, b, "two").await;
    h.dm_post(a, b, "three").await;
    h.settle().await;

    // B opens once → one batched read receipt covering all three.
    assert_eq!(
        h.dm_mark_read(b, a).await,
        3,
        "three messages read in one batch"
    );
    h.sync(a).await;

    let view = h.dm_view(a, b);
    assert_eq!(view.len(), 3);
    assert!(view.iter().all(|m| m.receipt == 3), "all three read");
}

#[tokio::test]
async fn read_receipt_emission_is_idempotent() {
    let h = Harness::new(&["Nye Thompson", "Harvey Deane"]).await;
    let (a, b) = (0, 1);
    h.pair(a, b).await;

    h.dm_post(a, b, "one").await;
    h.dm_post(a, b, "two").await;
    h.settle().await;

    // First open marks both messages read; a second open owes nothing (idempotent).
    assert_eq!(h.dm_mark_read(b, a).await, 2, "two unread → two receipts");
    assert_eq!(
        h.dm_mark_read(b, a).await,
        0,
        "already read → nothing re-emitted"
    );

    h.sync(a).await;
    assert!(
        h.dm_view(a, b).iter().all(|m| m.receipt == 3),
        "every one of A's messages reads as read"
    );
}
