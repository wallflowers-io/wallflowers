//! Event ticketing over real Nodes, real MLS, and the real relay — nothing mocked.
//!
//! Two journeys:
//!   1. The FREE-ticket loop end to end (the no-rail path every priced flow shares its
//!      Delta skeleton with): request → recordSale → issue → deliver → wallet → door.
//!   2. SLICE 0's exit criterion — a door-staff device that joins the Event group LATE
//!      first proves the pinned no-backfill doctrine (it holds NO ledger), then
//!      converges on the COMPLETE ledger via `event_reemit` (state re-emission by the
//!      ledger's legitimate holder at the new epoch — never history decryption).

mod common;

use common::Harness;
use pacific_core::event::{
    self, DoorVerdict, OP_RECORD_SALE, OP_REDEEM, OP_REQUEST_TICKET, OP_SET_PROFILE, OP_SET_TICKETS,
};

const START_MS: i64 = 1_800_000_000_000;

/// Alice organizes a free event; Bob (a connection, NEVER a member of the event
/// group) requests, is recorded, receives a signed credential he can verify against
/// Alice's identity, and is admitted exactly once at the door.
#[tokio::test]
async fn free_ticket_loop_end_to_end() {
    let h = Harness::new(&["Alice", "Bob"]).await;
    let (alice, bob) = (0, 1);
    h.pair(alice, bob).await;

    // The event is a group of 1 — Bob deliberately never joins it.
    let obj = h.form_object(alice, "event", &[]).await;
    h.node(alice)
        .apply(
            &obj,
            OP_SET_PROFILE,
            event::set_profile_args(
                "Garden gig",
                None,
                START_MS,
                None,
                Some("the allotment"),
                None,
                None,
            ),
        )
        .await
        .unwrap();
    h.node(alice)
        .apply(
            &obj,
            OP_SET_TICKETS,
            event::set_tickets_args(0, "usd", 10, true, None, None, None, 1),
        )
        .await
        .unwrap();

    let listing = h.node(alice).event_listing_id(&obj).unwrap();

    // Bob claims a slot over the CONNECTION channel (request-before-checkout shape;
    // a free event's "pi" is a client-minted claim id).
    // The leg rides the CONNECTION object, named as an object like anything else:
    // `apply` takes what is written to, not who it is with.
    let bobs_channel = h.node(bob).connection_object(&h.id(alice)).unwrap().unwrap().0;
    h.node(bob)
        .apply(
            &bobs_channel,
            OP_REQUEST_TICKET,
            event::request_ticket_args(&listing, 1, "free-claim-1"),
        )
        .await
        .unwrap();
    h.settle().await;

    // Alice's device folds the request off the shared channel.
    let seen = h.node(alice).contact_state_with(&h.id(bob)).unwrap();
    let req = seen
        .ticket_requests
        .get("free-claim-1")
        .expect("request folded");
    assert_eq!(req.qty, 1);
    assert_eq!(req.author, h.id(bob), "the MLS author IS the buyer");

    // Record the sale on the event ledger, issue, deliver.
    h.node(alice)
        .apply(
            &obj,
            OP_RECORD_SALE,
            event::record_sale_args(
                "free-claim-1",
                &h.id(bob),
                1,
                0,
                0,
                "usd",
                &["tkt-1".into()],
            ),
        )
        .await
        .unwrap();
    let creds = h
        .node(alice)
        .event_issue_credentials(&obj, "free-claim-1")
        .unwrap();
    assert_eq!(creds.len(), 1);
    let alices_channel = h.node(alice).connection_object(&h.id(bob)).unwrap().unwrap().0;
    h.node(alice)
        .apply(
            &alices_channel,
            event::OP_DELIVER_TICKET,
            event::deliver_ticket_args(&listing, &creds),
        )
        .await
        .unwrap();
    h.settle().await;

    // Bob's wallet holds the credential VERBATIM, and it verifies against Alice's
    // identity pk — which Bob knows from the connection, not from the QR.
    let wallet = h.node(bob).contact_state_with(&h.id(alice)).unwrap().wallet;
    let t = wallet.get("tkt-1").expect("ticket delivered");
    assert_eq!(t.listing, listing);
    let verified = event::verify_ticket(&t.core, &t.sig, &h.id(alice)).expect("signature holds");
    assert_eq!(verified.buyer, hex::encode(h.id(bob)));

    // The QR round-trip a door would scan.
    let qr = event::qr_payload(&t.core, &t.sig);
    let (core, sig) = event::parse_qr_payload(&qr).unwrap();
    event::verify_ticket(&core, &sig, &h.id(alice)).expect("scanned credential verifies");

    // Door: valid once, flagged after.
    let st = h.node(alice).event_state(&obj).unwrap();
    assert_eq!(st.door_check("tkt-1"), DoorVerdict::Valid);
    h.node(alice)
        .apply(&obj, OP_REDEEM, event::redeem_args("tkt-1"))
        .await
        .unwrap();
    let st = h.node(alice).event_state(&obj).unwrap();
    match st.door_check("tkt-1") {
        DoorVerdict::AlreadyRedeemed { double_scanned } => assert!(!double_scanned),
        v => panic!("expected AlreadyRedeemed, got {v:?}"),
    }
}

/// SLICE 0 EXIT CRITERION — the two history regimes, then convergence.
///
/// Regime 1 (first add): deltas authored in the GROUP-OF-1 era sit in the outbox
/// (roster 1 never flushes) and reach the FIRST added member at the join epoch —
/// sharing a private object shares its log. Carol gets everything.
/// Regime 2 (later adds): deltas flushed while the group was ≥2 are sealed at epochs
/// a later joiner has no keys for (the pinned m4 forward-secrecy semantics). Dave
/// joins after two relay-era sales and holds NOTHING — until `event_reemit`, where
/// the ledger's legitimate holder restates its own state at the NEW epoch and Dave
/// converges on the complete ledger. Then the two doors' redemptions converge.
#[tokio::test]
async fn late_joining_door_staff_converge_via_reemission() {
    let h = Harness::new(&["Alice", "Bob", "Carol", "Dave"]).await;
    let (alice, bob, carol, dave) = (0, 1, 2, 3);
    h.pair(alice, bob).await;
    h.pair(alice, carol).await;
    h.pair(alice, dave).await;

    let obj = h.form_object(alice, "event", &[]).await;
    h.node(alice)
        .apply(
            &obj,
            OP_SET_PROFILE,
            event::set_profile_args(
                "Launch party",
                None,
                START_MS,
                None,
                Some("ARC Gangnam"),
                None,
                None,
            ),
        )
        .await
        .unwrap();
    h.node(alice)
        .apply(
            &obj,
            OP_SET_TICKETS,
            event::set_tickets_args(0, "usd", 50, true, None, None, None, 1),
        )
        .await
        .unwrap();

    // REGIME 1: Carol is the FIRST add — the solo-era log (profile + listing)
    // flushes to her at the join epoch. Sharing the object shares its log.
    h.add_to_forum(alice, carol, &obj).await;
    let carol_st = h.node(carol).event_state(&obj).unwrap();
    assert_eq!(
        carol_st.title, "Launch party",
        "first add receives the solo-era log"
    );
    assert!(carol_st.listing.is_some());

    // Two sales land while the group is ≥2 — relay-epoch traffic Dave will miss.
    for (pi, tkt) in [("claim-1", "tkt-1"), ("claim-2", "tkt-2")] {
        h.node(alice)
            .apply(
                &obj,
                OP_RECORD_SALE,
                event::record_sale_args(pi, &h.id(bob), 1, 0, 0, "usd", &[tkt.into()]),
            )
            .await
            .unwrap();
    }
    h.settle().await;
    assert_eq!(
        h.node(carol).event_state(&obj).unwrap().ledger.len(),
        2,
        "a member at the sale epochs folds the sales"
    );

    // REGIME 2: Dave joins late. The sales were sealed at epochs he has no keys
    // for — the pinned no-backfill doctrine (m4). He holds no ledger.
    h.add_to_forum(alice, dave, &obj).await;
    let before = h.node(dave).event_state(&obj).unwrap();
    assert_eq!(
        before.ledger.len(),
        0,
        "a later joiner must NOT decrypt ≥2-member-era history (forward secrecy)"
    );

    // RE-EMISSION: the holder restates its own current state at the new epoch.
    // Idempotent for every existing member; converging for Dave.
    let emitted = h.node(alice).event_reemit(&obj).await.unwrap();
    assert!(
        emitted >= 4,
        "profile + listing + two sales re-emitted, got {emitted}"
    );
    h.settle().await;

    let after = h.node(dave).event_state(&obj).unwrap();
    assert_eq!(
        after.ledger.len(),
        2,
        "the COMPLETE ledger, post re-emission"
    );
    assert_eq!(
        after.title, "Launch party",
        "profile restated for the newcomer"
    );
    assert!(after.listing.is_some(), "listing restated for the newcomer");
    assert_eq!(after.door_check("tkt-1"), DoorVerdict::Valid);

    // Existing members' state is unchanged by the re-emission (idempotence by pi).
    for u in [alice, carol] {
        let st = h.node(u).event_state(&obj).unwrap();
        assert_eq!(st.ledger.len(), 2);
        assert_eq!(st.sold(), 2);
        assert!(!st.over_capacity);
    }

    // Dave scans tkt-1 at his door; Alice scans the same ticket at hers. After a
    // settle every device agrees: redeemed, and flagged as TWO different doors.
    h.node(dave)
        .apply(&obj, OP_REDEEM, event::redeem_args("tkt-1"))
        .await
        .unwrap();
    h.settle().await;
    h.node(alice)
        .apply(&obj, OP_REDEEM, event::redeem_args("tkt-1"))
        .await
        .unwrap();
    h.settle().await;

    for u in [alice, carol, dave] {
        let st = h.node(u).event_state(&obj).unwrap();
        match st.door_check("tkt-1") {
            DoorVerdict::AlreadyRedeemed { double_scanned } => {
                assert!(
                    double_scanned,
                    "two DIFFERENT doors admitted the same ticket"
                )
            }
            v => panic!("expected AlreadyRedeemed on device {u}, got {v:?}"),
        }
    }
}

/// The shared-event journey: connections converge on ONE canonical Event
/// GroupObject (two MLS groups never merge — the second person JOINS, object.rs
/// doctrine), and the event's OWN log carries the discussion: `forum.post` deltas
/// fold beside the event deltas, so members talk where the event lives, no second
/// Forum object and no link to lose. Both lenses stay clean on both devices.
#[tokio::test]
async fn shared_event_carries_its_own_discussion() {
    let h = Harness::new(&["Alice", "Bob"]).await;
    let (alice, bob) = (0, 1);
    h.pair(alice, bob).await;

    // Alice mints the event (from a listing, in the app's case) and adds Bob —
    // membership, not merging, is how two devices come to hold one event.
    let obj = h.form_object(alice, "event", &[bob]).await;
    h.node(alice)
        .apply(
            &obj,
            OP_SET_PROFILE,
            event::set_profile_args("Opening Night", None, START_MS, None, Some("Tillage"), None, None),
        )
        .await
        .unwrap();
    h.settle().await;

    // Both members post into the event's own thread.
    h.node(alice)
        .object_post(&obj, "who's going early?", None)
        .await
        .unwrap();
    h.settle().await;
    h.node(bob)
        .object_post(&obj, "me — doors at eight", None)
        .await
        .unwrap();
    h.settle().await;

    for u in [alice, bob] {
        // The discussion lens: both posts, in fold order, on both devices.
        let t = h.node(u).object_transcript(&obj).unwrap();
        assert_eq!(t.len(), 2, "device {u} folds both posts");
        assert!(t[0].contains("who's going early?"), "device {u}: {t:?}");
        assert!(t[1].contains("doors at eight"), "device {u}: {t:?}");

        // The event lens is untouched by the chatter beside it.
        let events = h.node(u).events().unwrap();
        let (_, st) = events
            .iter()
            .find(|(id, _)| *id == obj)
            .expect("the shared event folds on both devices");
        assert_eq!(st.title, "Opening Night");
        assert_eq!(st.venue, "Tillage");
        assert_eq!(st.start_ms, START_MS);
    }

    // Any other kind still refuses a post — the gate narrowed, it did not vanish.
    let thing = h.node(alice).object_new("thing", "hammer").unwrap();
    assert!(h.node(alice).object_post(&thing, "no", None).await.is_err());
}
