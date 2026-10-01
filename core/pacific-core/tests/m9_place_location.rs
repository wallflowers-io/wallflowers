//! A PLACE, over a real Node, a real MLS group and real Deltas.
//!
//! What this pins is the seam LIFE's places rest on: that somewhere you have been is a
//! DURABLE GroupObject carrying the common location facet — not a device-local row. The
//! coordinate has to survive the whole path (author → append → fold → read) and come back
//! out in degrees having been stored as e7 fixed-point, or a Place is just a name.
//!
//! It also pins the thing that made Place worth adding as a kind: it is the first object to
//! splice a BASE op-group (`geo::LOCATION_OPS`) into its own `ops()`. If that splice ever
//! breaks, `setLocation` stops being authorable and every Place silently unplaces itself.

mod common;

use common::Harness;
use pacific_core::geo::{GeoPoint, LocationSource, OP_CLEAR_LOCATION, OP_SET_LOCATION};
use pacific_core::place::{set_profile_args, Access, OP_SET_PROFILE};

/// Bristol, to 7 decimal places.
const LAT: f64 = 51.4545;
const LNG: f64 = -2.5879;

/// Mint a Place where you are standing → it reads back placed, at that coordinate.
#[tokio::test]
async fn mint_a_place_and_it_keeps_its_coordinate() {
    let h = Harness::new(&["Alice"]).await;
    let alice = h.node(0);

    let source = LocationSource::Fixed {
        point: GeoPoint::from_degrees(LAT, LNG),
    };
    let id = alice
        .place_mint("the kitchen", "top floor", Some(source))
        .await
        .expect("mint the place");

    let st = alice.place_state(&id).expect("fold the place");
    assert_eq!(st.name, "the kitchen");
    assert_eq!(st.descriptor, "top floor");

    // e7 in, degrees out — the round trip the FFI boundary depends on.
    let p = st.point().expect("a minted place is placed");
    assert_eq!(p.lat_e7, 514_545_000);
    assert_eq!(p.lng_e7, -25_879_000);
    assert!((p.lat() - LAT).abs() < 1e-6);
    assert!((p.lng() - LNG).abs() < 1e-6);

    // The list LIFE's PLACES filter actually binds to.
    let all = alice.places().expect("list places");
    let row = all
        .iter()
        .find(|(oid, _)| *oid == id)
        .expect("our place is listed");
    assert_eq!(row.1.name, "the kitchen");
    assert!(row.1.point().is_some());
}

/// A Place minted without a fix is a real object that is honestly unplaced, and a later
/// `setLocation` places it. GPS is slow indoors; losing the user's input to that is worse
/// than an object that says it has no coordinate yet.
#[tokio::test]
async fn an_unplaced_place_can_be_placed_later() {
    let h = Harness::new(&["Alice"]).await;
    let alice = h.node(0);

    let id = alice
        .place_mint("the bar", "", None)
        .await
        .expect("mint unplaced");
    let st = alice.place_state(&id).expect("fold");
    assert_eq!(st.name, "the bar");
    assert!(
        st.point().is_none(),
        "no fix means honestly unplaced, not (0,0)"
    );

    let source = LocationSource::Fixed {
        point: GeoPoint::from_degrees(LAT, LNG),
    };
    alice
        .apply(
            &id,
            OP_SET_LOCATION,
            pacific_core::geo::set_location_args(&source),
        )
        .await
        .expect("place it");

    let st = alice.place_state(&id).expect("re-fold");
    assert_eq!(st.point().expect("now placed").lat_e7, 514_545_000);
    assert_eq!(st.name, "the bar", "placing must not disturb the profile");
}

/// Several deltas on ONE spine: the base location ops share the sequence with the Place's
/// own ops, so a profile edit after a location must not fork the log.
#[tokio::test]
async fn base_and_own_ops_share_one_spine() {
    let h = Harness::new(&["Alice"]).await;
    let alice = h.node(0);

    let id = alice.object_new("place", "site").expect("mint");
    alice
        .apply(&id, OP_SET_PROFILE, set_profile_args("site", "phase 1"))
        .await
        .expect("profile");

    let a = LocationSource::Fixed {
        point: GeoPoint::from_degrees(51.0, -2.0),
    };
    alice
        .apply(
            &id,
            OP_SET_LOCATION,
            pacific_core::geo::set_location_args(&a),
        )
        .await
        .expect("locate");

    // Profile again AFTER a base op — this is the interleave that a per-type `next_seq`
    // bug would fork.
    alice
        .apply(&id, OP_SET_PROFILE, set_profile_args("site", "phase 2"))
        .await
        .expect("re-profile");

    let b = LocationSource::Fixed {
        point: GeoPoint::from_degrees(52.0, -1.0),
    };
    alice
        .apply(
            &id,
            OP_SET_LOCATION,
            pacific_core::geo::set_location_args(&b),
        )
        .await
        .expect("move it");

    let st = alice.place_state(&id).expect("fold all four");
    assert_eq!(st.descriptor, "phase 2", "last profile wins");
    assert_eq!(
        st.point().expect("placed").lat_e7,
        520_000_000,
        "last location wins"
    );

    // Clearing unplaces without destroying the Place.
    alice
        .apply(
            &id,
            OP_CLEAR_LOCATION,
            pacific_core::coordinator::Args::new(),
        )
        .await
        .expect("clear");
    let st = alice.place_state(&id).expect("fold");
    assert!(st.point().is_none());
    assert_eq!(st.name, "site");
}

/// A PUBLIC PLACE is a posture on a Place, not a kind: opening one changes what the QR
/// means and nothing else about the object.
#[tokio::test]
async fn opening_a_place_makes_it_requestable_and_vends_a_qr() {
    let h = Harness::new(&["Alice"]).await;
    let alice = h.node(0);

    let id = alice.place_mint("the bar", "", None).await.expect("mint");

    // Private by default — and a private place must not vend a sign at all.
    assert_eq!(alice.place_state(&id).unwrap().access, Access::Private);
    assert!(
        alice.place_join_card(&id).is_err(),
        "a private place must not hand out a way in"
    );

    alice
        .place_set_access(&id, Access::Public)
        .await
        .expect("open it");
    assert_eq!(alice.place_state(&id).unwrap().access, Access::Public);
    let doorbell = alice.place_fit_doorbell(&id).await.expect("fit a doorbell");

    let card = alice.place_join_card(&id).expect("public places vend a QR");
    assert!(card.starts_with("pacific://place-join?id="));
    assert!(card.contains(&id), "the QR must name the place it joins");
    assert!(
        card.contains(&hex::encode(doorbell)),
        "and carry the doorbell"
    );
    assert!(
        !card.contains("bundle="),
        "and NOT anyone's personal pairing bundle — the sign belongs to the place"
    );

    // Closing it again revokes the meaning of any poster already printed.
    alice
        .place_set_access(&id, Access::Private)
        .await
        .expect("close it");
    assert!(alice.place_join_card(&id).is_err());
}

/// Admission is guarded on the CURRENT posture, so an intent recorded while a Place was
/// open cannot be honoured after the host has closed it.
#[tokio::test]
async fn a_closed_place_refuses_to_admit() {
    let h = Harness::new(&["Alice", "Bob"]).await;
    let alice = h.node(0);
    let bob_id = h.id(1);

    let id = alice.place_mint("the bar", "", None).await.expect("mint");
    // Never opened: admission is refused on the posture, before it ever reaches the
    // "are they a contact?" question.
    let err = alice
        .place_admit(&id, &bob_id)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("private"),
        "refused for the right reason: {err}"
    );
}

/// AUTO-ACCEPT. Someone scans the sign and knocks; the place lets them in on the next
/// sync with no approval step. Pinned end to end over real nodes rather than at the reducer.
///
/// NOTE the harness rule this test exists to respect: `Harness::node` sets the process-wide
/// `PACIFIC_STATE_DIR`, so a held `Node` goes stale the moment another user's node is
/// fetched. Every call is therefore re-acquired inline.
#[tokio::test]
async fn a_public_place_admits_a_knocker_automatically() {
    let mut h = Harness::new(&["Alice"]).await;
    let bob = h.add_user("Bob");

    let id = h
        .node(0)
        .place_mint("the bar", "", None)
        .await
        .expect("mint");
    h.node(0)
        .place_set_access(&id, Access::Public)
        .await
        .expect("open");
    let doorbell = h.node(0).place_fit_doorbell(&id).await.expect("doorbell");

    // Bob scans the sign and knocks — no pairing with anyone.
    h.node(bob)
        .place_knock(&id, &doorbell)
        .await
        .expect("knock");

    h.settle().await;
    let _ = h.node(0).answer_all_doors().await.expect("answer");
    h.settle().await;

    assert!(
        h.roster(0, &id).contains(&h.id(bob)),
        "a public place must let a knocker in with nobody approving anything"
    );
}

/// The reconcile is idempotent and self-limiting: running it twice must not re-add, and a
/// place that has since closed must stop admitting.
#[tokio::test]
async fn reconcile_is_idempotent_and_stops_when_closed() {
    let h = Harness::new(&["Alice"]).await;
    let alice = h.node(0);

    let id = alice.place_mint("the bar", "", None).await.expect("mint");
    alice
        .place_set_access(&id, Access::Public)
        .await
        .expect("open");

    // Nothing pending: a free no-op, which is what makes it safe on the sync tick.
    assert!(alice
        .reconcile_place_joins()
        .await
        .expect("first")
        .is_empty());
    assert!(alice
        .reconcile_place_joins()
        .await
        .expect("second")
        .is_empty());

    alice
        .place_set_access(&id, Access::Private)
        .await
        .expect("close");
    assert!(
        alice
            .reconcile_place_joins()
            .await
            .expect("closed")
            .is_empty(),
        "a closed place admits nobody"
    );
}

/// The location facet and the access posture are independent slots — opening a Place
/// must not disturb where it is, and moving it must not reopen or close it.
#[tokio::test]
async fn access_and_location_are_independent() {
    let h = Harness::new(&["Alice"]).await;
    let alice = h.node(0);

    let source = LocationSource::Fixed {
        point: GeoPoint::from_degrees(LAT, LNG),
    };
    let id = alice
        .place_mint("the bar", "back room", Some(source))
        .await
        .expect("mint");

    alice
        .place_set_access(&id, Access::Public)
        .await
        .expect("open");
    let st = alice.place_state(&id).expect("fold");
    assert_eq!(st.access, Access::Public);
    assert_eq!(st.point().expect("still placed").lat_e7, 514_545_000);
    assert_eq!(st.name, "the bar");

    // Move it — the posture rides through.
    let moved = LocationSource::Fixed {
        point: GeoPoint::from_degrees(52.0, -1.0),
    };
    alice
        .apply(
            &id,
            OP_SET_LOCATION,
            pacific_core::geo::set_location_args(&moved),
        )
        .await
        .expect("move");
    let st = alice.place_state(&id).expect("re-fold");
    assert_eq!(
        st.access,
        Access::Public,
        "moving a place must not close it"
    );
    assert_eq!(st.point().unwrap().lat_e7, 520_000_000);
}

/// ANCHORING IS AN AGREEMENT. Putting your group on a bench records the undertaking on
/// the GROUP's own log, so every member folds it — the duty is the membership's, not the
/// recollection of whoever printed the QR.
#[tokio::test]
async fn anchoring_a_group_records_the_duty_on_the_group() {
    let h = Harness::new(&["Alice"]).await;
    let alice = h.node(0);

    let place = alice
        .place_mint("the bench", "", None)
        .await
        .expect("mint place");
    let group = h.mint_group(0);

    // You cannot agree to answer for a place that admits nobody.
    let err = alice
        .group_anchor_at_place(&group, &place, 1_700_000_000_000)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("private"),
        "refused for the right reason: {err}"
    );

    alice
        .place_set_access(&place, Access::Public)
        .await
        .expect("open");
    alice
        .group_anchor_at_place(&group, &place, 1_700_000_000_000)
        .await
        .expect("anchor");

    // The commitment is legible from the group's folded state.
    let anchored = alice.group_anchored_places(&group).expect("read duties");
    assert_eq!(anchored.len(), 1);
    assert_eq!(anchored[0].0, place);
    assert_eq!(
        anchored[0].1, "the bench",
        "the place's name rides the edge as a label"
    );

    // And the place resolves back to the groups that answer for it.
    assert_eq!(
        alice.our_groups_anchored_at(&place).expect("reverse"),
        vec![group.clone()]
    );

    // Revocable: you can stop answering.
    alice
        .group_unanchor(&group, &place)
        .await
        .expect("unanchor");
    assert!(alice
        .group_anchored_places(&group)
        .expect("read")
        .is_empty());
    assert!(alice
        .our_groups_anchored_at(&place)
        .expect("reverse")
        .is_empty());
}

/// A federation edge carries no duty — only `anchored` obliges anyone.
#[tokio::test]
async fn only_an_anchored_edge_answers_for_its_peer() {
    use pacific_core::group::AffiliationRel;
    assert!(AffiliationRel::Anchored.answers_for_peer());
    assert!(!AffiliationRel::Parent.answers_for_peer());
    assert!(!AffiliationRel::Child.answers_for_peer());
    assert!(!AffiliationRel::Peer.answers_for_peer());
    assert_eq!(
        AffiliationRel::parse("anchored").unwrap(),
        AffiliationRel::Anchored
    );
}

/// THE ALLOTMENT, END TO END — and the point of the whole design: the founder's phone is
/// NOT on the critical path.
///
/// Alice fits the gate with a doorbell and then plays no further part. Bob (already a
/// plot-holder, NOT the owner) is the one who answers when Carol scans the sign.
#[tokio::test]
async fn any_member_can_answer_the_door_not_just_the_founder() {
    let mut h = Harness::new(&["Alice"]).await;
    let bob = h.add_user("Bob");
    let carol = h.add_user("Carol");

    // Alice puts a sign on the allotment gate. (Every node re-acquired inline —
    // `Harness::node` sets a process-wide state dir, so a held handle goes stale.)
    let place = h
        .node(0)
        .place_mint("the allotment", "", None)
        .await
        .expect("mint");
    h.node(0)
        .place_set_access(&place, Access::Public)
        .await
        .expect("open");
    let doorbell = h
        .node(0)
        .place_fit_doorbell(&place)
        .await
        .expect("fit doorbell");
    let card = h.node(0).place_join_card(&place).expect("poster");
    assert!(
        card.contains("&db="),
        "the sign carries a doorbell, not a person"
    );
    assert!(
        !card.contains("bundle="),
        "and no longer anyone's pairing bundle"
    );

    // Bob is already in — he scanned last week.
    let bob_bundle = h.node(bob).build_contact_bundle().expect("bob bundle");
    h.node(0)
        .group_add_member(&place, &bob_bundle)
        .await
        .expect("alice admits bob");
    h.settle().await;
    assert!(
        h.roster(0, &place).contains(&h.id(bob)),
        "bob is a plot-holder"
    );

    // Carol scans the gate. She pairs with NOBODY — she just knocks.
    h.node(carol)
        .place_knock(&place, &doorbell)
        .await
        .expect("carol knocks");
    h.settle().await;

    // BOB answers, not Alice. He is a member, not the owner — which is the whole point.
    let admitted = h.node(bob).answer_all_doors().await.expect("bob answers");
    h.settle().await;

    assert!(
        !admitted.is_empty() || h.roster(bob, &place).contains(&h.id(carol)),
        "a non-owner member must be able to let a stranger in"
    );
    assert!(
        h.roster(bob, &place).contains(&h.id(carol)),
        "carol is in the place itself, admitted by someone who does not own it"
    );
}

/// A closed sign is inert: knocks are ignored rather than queued up to admit people the
/// moment somebody reopens.
#[tokio::test]
async fn a_closed_door_is_not_answered() {
    let mut h = Harness::new(&["Alice"]).await;
    let carol = h.add_user("Carol");

    let place = h
        .node(0)
        .place_mint("the allotment", "", None)
        .await
        .expect("mint");
    h.node(0)
        .place_set_access(&place, Access::Public)
        .await
        .expect("open");
    let doorbell = h
        .node(0)
        .place_fit_doorbell(&place)
        .await
        .expect("doorbell");
    h.node(carol)
        .place_knock(&place, &doorbell)
        .await
        .expect("knock");

    h.node(0)
        .place_set_access(&place, Access::Private)
        .await
        .expect("close");
    h.settle().await;
    assert!(
        h.node(0)
            .place_answer_door(&place)
            .await
            .expect("answer")
            .is_empty(),
        "a closed place admits nobody, even with a knock already waiting"
    );
    assert!(!h.roster(0, &place).contains(&h.id(carol)));
}

/// Rotation is the only revocation there is: a new doorbell supersedes the old, so a sign
/// already printed stops resolving.
#[tokio::test]
async fn rotating_the_doorbell_retires_the_printed_sign() {
    let h = Harness::new(&["Alice"]).await;
    let alice = h.node(0);

    let place = alice
        .place_mint("the allotment", "", None)
        .await
        .expect("mint");
    alice
        .place_set_access(&place, Access::Public)
        .await
        .expect("open");
    let first = alice.place_fit_doorbell(&place).await.expect("first");
    let old_card = alice.place_join_card(&place).expect("old poster");

    let second = alice.place_fit_doorbell(&place).await.expect("rotate");
    assert_ne!(first, second, "rotation mints a fresh capability");

    let new_card = alice.place_join_card(&place).expect("new poster");
    assert_ne!(old_card, new_card, "the printed sign is now out of date");
    assert!(new_card.contains(&hex::encode(second)));
    assert!(
        !new_card.contains(&hex::encode(first)),
        "the old doorbell is gone"
    );

    // The folded state carries only the current one — an ex-member holding `first` is
    // reading a mailbox nobody delivers to any more.
    assert_eq!(
        alice.place_state(&place).expect("fold").doorbell,
        Some(second)
    );
}

/// A place with no doorbell fitted must not vend a sign that cannot be knocked at.
#[tokio::test]
async fn no_doorbell_no_poster() {
    let h = Harness::new(&["Alice"]).await;
    let alice = h.node(0);
    let place = alice
        .place_mint("the allotment", "", None)
        .await
        .expect("mint");
    alice
        .place_set_access(&place, Access::Public)
        .await
        .expect("open");
    let err = alice.place_join_card(&place).unwrap_err().to_string();
    assert!(err.contains("doorbell"), "said why: {err}");
}

/// `place_author` must refuse a non-place object rather than writing a Place delta into
/// someone else's log, where it would fold as `UnknownType` forever.
#[tokio::test]
async fn place_ops_are_refused_on_another_kind() {
    let h = Harness::new(&["Alice"]).await;
    let alice = h.node(0);

    let forum = alice
        .object_new("forum", "not a place")
        .expect("mint forum");
    let err = alice
        .apply(&forum, OP_SET_PROFILE, set_profile_args("x", ""))
        .await;
    assert!(err.is_err(), "a forum must not accept a place delta");

    // And `places()` must not leak it.
    let all = alice.places().expect("list");
    assert!(all.iter().all(|(oid, _)| *oid != forum));
}
