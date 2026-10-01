//! M6 — pairing establishes a Contact: it stocks prekeys into the shared channel
//! (so the peer can be added to future Conversations with no live code exchange) and
//! writes the directory EDGE peer → a seeded identity Group ("who they are"), all
//! while the same channel still carries DMs. Real relay, real MLS, real device
//! boundaries (isolated state dirs).

mod common;

use common::Harness;

#[tokio::test]
async fn pairing_stocks_prekeys_and_names_the_peer() {
    let h = Harness::new(&["Alice", "Bob"]).await;
    let (alice, bob) = (0, 1);

    // `pair(a, b)`: b SCANS a's bundle → b is the scanner (stocks its prekeys); a
    // drains b's Welcome + prekey offers on sync. So `alice` ends up holding `bob`'s.
    h.pair(alice, bob).await;
    h.settle().await;

    // 1. DMs still fold over the SAME channel — prekey deltas (Contact op-group) and
    //    message deltas (Forum op-group) coexist in one log, folded by type lens.
    h.dm_post(alice, bob, "hello over the contact channel")
        .await;
    h.settle().await;
    let seen: Vec<String> = h
        .dm_view(bob, alice)
        .iter()
        .map(|m| m.text.clone())
        .collect();
    assert!(
        seen.contains(&"hello over the contact channel".to_string()),
        "DM must still fold alongside prekeys, saw {seen:?}",
    );

    // 2. Prekeys: alice holds the offers bob stocked (ready to add bob to a group).
    assert!(
        h.contact_prekey_count(alice, bob) >= 1,
        "alice should hold bob's stocked prekeys, has {}",
        h.contact_prekey_count(alice, bob),
    );

    // 3. The NAME: each side resolves the peer's name from the peer's own profile,
    //    with the Contact channel itself storing no identity. Pairing no longer
    //    mints a group ABOUT the peer to read a name out of — an individual's
    //    Group is only for them (m30).
    assert_eq!(h.peer_identity_name(alice, bob), "Bob");
    assert_eq!(h.peer_identity_name(bob, alice), "Alice");
}

/// BOTH halves of a pairing must stock, and either side must be able to add the
/// other. The scanner stocks in `pair_scan`; the SCANNED side stocks in
/// `pair_accept` — and until it did, the scanner held zero offers from its own
/// contact and `add_contact_to_object` failed forever with "ask them to
/// replenish", a verb nothing implemented. The suite asserted only the working
/// direction (`contact_prekey_count(alice, bob)`), so CI stayed green over a
/// permanently broken half. This asserts the other one, and proves the add.
#[tokio::test]
async fn both_sides_stock_prekeys_and_either_can_add_the_other() {
    let h = Harness::new(&["Alice", "Bob"]).await;
    let (alice, bob) = (0, 1);

    h.pair(alice, bob).await; // bob is the scanner, alice the scanned side
    h.settle().await;

    // The direction that always worked: the scanner stocked at scan time.
    assert!(
        h.contact_prekey_count(alice, bob) >= 1,
        "alice should hold the scanner's offers, has {}",
        h.contact_prekey_count(alice, bob),
    );
    // The direction that never did: the scanned side stocks on accept.
    assert!(
        h.contact_prekey_count(bob, alice) >= 1,
        "the SCANNED side never stocked — bob holds {} of alice's offers, so bob \
         could never add his own contact to anything",
        h.contact_prekey_count(bob, alice),
    );

    // And the reverse add works end to end: the scanner adds the scanned side.
    let obj = h.form_named_forum(bob, "Reverse Add");
    h.add_contact_to(bob, alice, &obj).await;
    assert_eq!(
        h.object_name(alice, &obj),
        "Reverse Add",
        "alice joined the group bob added her to with her own stocked prekey",
    );
}

/// Offers are spent one per add and must be replaced, or a long-lived connection
/// silently becomes un-addable once the initial batch runs out. `sync_once`'s
/// replenishment pass tops each channel back up to PREKEY_STOCK.
#[tokio::test]
async fn spent_prekeys_are_replenished_by_sync() {
    let h = Harness::new(&["Alice", "Bob"]).await;
    let (alice, bob) = (0, 1);
    h.pair(alice, bob).await;
    h.settle().await;

    let full = h.contact_prekey_count(alice, bob);
    assert!(full >= 1, "alice must hold bob's offers, has {full}");

    // Spend one.
    let obj = h.form_named_forum(alice, "Spend One");
    h.add_contact_to(alice, bob, &obj).await;

    // Bob's node notices the shortfall on its own sync and authors replacements.
    h.settle().await;
    assert_eq!(
        h.contact_prekey_count(alice, bob),
        full,
        "the spent offer must be replaced, leaving the channel back at full stock",
    );
}

/// The payoff: add an existing contact to a group by CONSUMING a stocked prekey —
/// no fresh code, no paste. The contact joins, and the prekey is tombstoned.
#[tokio::test]
async fn add_a_contact_to_a_group_by_consuming_a_prekey() {
    let h = Harness::new(&["Alice", "Bob"]).await;
    let (alice, bob) = (0, 1);
    h.pair(alice, bob).await; // alice ends up holding bob's stocked prekeys
    h.settle().await;
    let before = h.contact_prekey_count(alice, bob);
    assert!(
        before >= 1,
        "alice must hold bob's prekeys to add him, has {before}"
    );

    // Alice makes a group and adds Bob with NO code exchange — just his stocked prekey.
    let obj = h.form_named_forum(alice, "Design Review");
    h.add_contact_to(alice, bob, &obj).await;

    // Bob joined: he reads the group name from his Welcome, and can post to it.
    assert_eq!(
        h.object_name(bob, &obj),
        "Design Review",
        "bob joined the group via the prekey"
    );
    h.obj_post(bob, &obj, "bob is here via prekey").await;
    h.settle().await;
    let alice_sees: Vec<String> = h
        .obj_view(alice, &obj)
        .iter()
        .map(|m| m.text.clone())
        .collect();
    assert!(
        alice_sees.contains(&"bob is here via prekey".to_string()),
        "alice sees bob's post — he is a real member, saw {alice_sees:?}",
    );

    // The consumed prekey is tombstoned. Asserted on the SPENT set rather than on
    // the available count: since sync replenishes a short channel back to
    // PREKEY_STOCK, "one fewer usable offer" is no longer a stable observation.
    assert!(
        h.contact_prekeys_spent(alice, bob) >= 1,
        "the consumed prekey must carry a terminal tombstone (before: {before} usable)",
    );
}
