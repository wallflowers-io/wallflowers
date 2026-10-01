//! Feast Xalapa, as an **Event (29)** — the whole implemented op set, from three devices.
//!
//! All six of the Event's own ops are driven here: `setProfile` (0), `setTickets` (1),
//! `recordSale` (2), `redeem` (3), `setMedia` (4) and `setVenue` (7). Four are
//! owner/sequenced and two are anyMember/commutative, so this is the kind where the two
//! folds meet on one object — and the kind where multidevice is sharpest, because the
//! owner-sequenced spine is the one place two of a person's devices can genuinely hurt
//! each other.
//!
//! WHY THERE IS NO OP 5 AND NO OP 6, which the brief asked about. They are not missing
//! and they are not reserved: they are LENT. `event.rs` declares them as the two
//! Contact-spliced ticket legs —
//!
//! ```text
//! pub const OP_REQUEST_TICKET: u32 = 5;   // contact.requestTicket
//! pub const OP_DELIVER_TICKET: u32 = 6;   // contact.deliverTicket
//! ```
//!
//! — and they carry CONTACT's op ids, not the Event's, because a buyer never joins the
//! Event GroupObject. The request and the delivery ride the pairwise Contact channel;
//! `CONTACT_TICKET_OPS` in event.rs is spliced verbatim into `contact.rs::CONTACT_OPS`,
//! and 9/10/11 (`invite`, `inviteReply`, `admitTicket`) are lent the same way. So the
//! Event's own table steps over 5 and 6 to avoid claiming ids it has already spent. That
//! is asserted below rather than left as a comment.
//!
//! Cast and copy from `app/web/docs/seeds/stoma.js`: paola runs the Xalapa residency,
//! axel holds the phone and the laptop, szonja works the door.

mod common;

use common::Harness;
use pacific_core::fold;
use pacific_core::event::{
    self, OP_RECORD_SALE, OP_REDEEM, OP_SET_MEDIA, OP_SET_PROFILE, OP_SET_TICKETS, OP_SET_VENUE,
};
use pacific_core::object::ObjectType;

/// 12 Dec 2026, 19:00 — the long table.
const START_MS: i64 = 1_797_100_000_000;
const BANNER: &str = "ZmVhc3QtYmFubmVy";

/// The event is set up from axel's phone, amended from his laptop, worked by szonja, and
/// all three devices agree on every field.
#[tokio::test]
async fn the_feast_is_set_up_on_one_device_and_amended_from_another() {
    let h = Harness::people(&[("axel", &["phone", "laptop"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");
    let szonja = h.device("szonja", "phone");
    h.pair(phone, szonja).await;

    let feast = h.form_object(phone, "event", &[szonja]).await;
    h.add_to_forum(phone, laptop, &feast).await;
    h.settle().await;
    assert_eq!(h.device_leaves(phone, &feast), 3, "three leaves, two people");

    // ── setProfile (0), owner/sequenced ───────────────────────────────────────
    h.node(phone)
        .apply(
            &feast,
            OP_SET_PROFILE,
            event::set_profile_args(
                "Feast Xalapa",
                Some("Long table seats 40. Two burners only."),
                START_MS,
                Some(START_MS + 4 * 3_600_000),
                Some("the residency"),
                None,
                Some("paola\nemilio\ndavid"),
            ),
        )
        .await
        .unwrap();

    // ── setTickets (1) — free, capped at the table ────────────────────────────
    h.node(phone)
        .apply(
            &feast,
            OP_SET_TICKETS,
            event::set_tickets_args(0, "eur", 40, true, None, None, None, 1),
        )
        .await
        .unwrap();

    // ── setMedia (4) and setVenue (7) ─────────────────────────────────────────
    h.node(phone)
        .apply(
            &feast,
            OP_SET_MEDIA,
            event::set_media_args(Some((BANNER, "image/jpeg")), &[], None),
        )
        .await
        .unwrap();
    let residency = h.form_object(phone, "place", &[]).await;
    h.node(phone)
        .apply(
            &feast,
            OP_SET_VENUE,
            event::set_venue_args(Some(&residency), "Xalapa Residency", START_MS),
        )
        .await
        .unwrap();
    h.settle().await;

    // ── THE AMENDMENT, from the LAPTOP ────────────────────────────────────────
    // axel's laptop passes the owner gate for the same reason it does on a Group: both
    // devices restore one identity. What it must do FIRST is sync — the spine positions
    // deltas by `(epoch, seq)` computed from THIS DEVICE's fold, so a laptop that has
    // not seen the phone's four deltas will reuse their seats. `settle` above is that
    // sync, and `two_unsynced_devices_of_the_owner_fork_the_spine_for_everyone` is what
    // happens when it is missing.
    h.node(laptop)
        .apply(
            &feast,
            OP_SET_PROFILE,
            event::set_profile_args(
                "Feast Xalapa — one hour later",
                Some("Long table seats 40. Two burners only."),
                START_MS + 3_600_000,
                Some(START_MS + 5 * 3_600_000),
                Some("the residency"),
                None,
                Some("paola\nemilio\ndavid"),
            ),
        )
        .await
        .expect("axel's laptop must carry his owner authority on the Event spine too");
    h.settle().await;

    for u in [phone, laptop, szonja] {
        let st = h.node(u).event_state(&feast).unwrap();
        assert_eq!(
            st.title,
            "Feast Xalapa — one hour later",
            "{} holds the wrong title — the laptop's later spine delta is LWW and must \
             win on every leaf",
            h.device_name(u)
        );
        assert_eq!(st.start_ms, START_MS + 3_600_000, "{}", h.device_name(u));
        assert_eq!(st.end_ms, Some(START_MS + 5 * 3_600_000));
        assert_eq!(st.lineup, vec!["paola", "emilio", "david"]);
        assert_eq!(st.media.banner, BANNER, "{}", h.device_name(u));
        assert_eq!(st.media.banner_mime, "image/jpeg");
        let v = st.venue_ref.as_ref().expect("the venue reference folded");
        assert_eq!(v.place, residency);
        assert_eq!(v.name, "Xalapa Residency");
        let l = st.listing.as_ref().expect("the ticket listing folded");
        assert_eq!((l.price_cents, l.capacity, l.open), (0, 40, true));
    }
}

/// The door: a sale recorded by the owner, a ticket redeemed by szonja — who is a MEMBER
/// and not the owner, which is exactly what `recordSale`/`redeem` being anyMember buys.
///
/// Also pins the seller gate: szonja may redeem, but she may NOT record a sale, because
/// `recordSale`'s reducer only accepts an author in `state.sellers` (the owner plus any
/// delegate a listing revision named). `event_author` dry-runs the reducer before it
/// persists, so that refusal arrives as an error rather than as a delta that is inert
/// forever.
#[tokio::test]
async fn szonja_works_the_door_and_cannot_sell() {
    let h = Harness::people(&[("axel", &["phone", "laptop"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");
    let szonja = h.device("szonja", "phone");
    h.pair(phone, szonja).await;

    let feast = h.form_object(phone, "event", &[szonja]).await;
    h.add_to_forum(phone, laptop, &feast).await;
    h.settle().await;
    h.node(phone)
        .apply(
            &feast,
            OP_SET_PROFILE,
            event::set_profile_args("Feast Xalapa", None, START_MS, None, None, None, None),
        )
        .await
        .unwrap();
    h.node(phone)
        .apply(
            &feast,
            OP_SET_TICKETS,
            event::set_tickets_args(0, "eur", 40, true, None, None, None, 1),
        )
        .await
        .unwrap();
    h.settle().await;

    // The owner records paola's two seats — from the LAPTOP, to keep the multidevice
    // claim on the commutative arm as well as the spine.
    h.node(laptop)
        .apply(
            &feast,
            OP_RECORD_SALE,
            event::record_sale_args(
                "claim-paola",
                &h.id(szonja),
                2,
                0,
                0,
                "eur",
                &["tkt-paola-1".into(), "tkt-paola-2".into()],
            ),
        )
        .await
        .unwrap();
    h.settle().await;

    // szonja redeems one at the door. anyMember/commutative: she is on the roster.
    h.node(szonja)
        .apply(&feast, OP_REDEEM, event::redeem_args("tkt-paola-1"))
        .await
        .expect("event.redeem is anyMember — a member working the door must be able to");
    h.settle().await;

    for u in [phone, laptop, szonja] {
        let st = h.node(u).event_state(&feast).unwrap();
        assert_eq!(st.sold(), 2, "{} disagrees on the ledger", h.device_name(u));
        assert_eq!(st.remaining(), Some(38), "{}", h.device_name(u));
        assert_eq!(st.redemptions.len(), 1, "{}", h.device_name(u));
        let r = st.redemptions.get("tkt-paola-1").unwrap();
        assert_eq!(
            r.first,
            Some(h.id(szonja)),
            "{} — the door that scanned it is recorded",
            h.device_name(u)
        );
        assert!(!st.over_capacity);
    }

    // ── the seller gate ───────────────────────────────────────────────────────
    let err = h
        .node(szonja)
        .apply(
            &feast,
            OP_RECORD_SALE,
            event::record_sale_args("claim-szonja", &h.id(szonja), 1, 0, 0, "eur", &["x".into()]),
        )
        .await
        .expect_err(
            "szonja recorded a sale. `recordSale` is anyMember on the WIRE but the \
             reducer gates on `state.sellers`, and event_author dry-runs it — if this \
             now succeeds, the money mirror will accept rows from anyone on the roster",
        );
    assert!(
        format!("{err}").contains("Unauthorized"),
        "the refusal must be the seller gate: {err}"
    );
    assert_eq!(
        h.node(phone).event_state(&feast).unwrap().sold(),
        2,
        "and nothing was written"
    );
}

/// THE HAZARD, pinned exactly the way `m21` pins the message-ref collision: TWO OF ONE
/// PERSON'S DEVICES THAT HAVE NOT SYNCED WILL FORK AN OWNER-SEQUENCED SPINE, AND THE
/// DAMAGE FALLS ON EVERYONE.
///
/// `event_next_seq` derives `(seq, prev)` from THIS DEVICE's fold of the spine. Both of
/// axel's devices are the owner, so both pass every authority check; if the laptop has
/// not folded the phone's delta it computes the same seat, and the two deltas collide at
/// `(epoch, seq)`. `SequencedSpine::deliver` returns `ForkDetected`, and
/// `fold::fold_entries` propagates that with `?` — so `event_state` HARD-ERRORS.
/// Not a lost edit: the object stops being readable, for the owner's other device, for
/// the owner's own phone, and for szonja, who did nothing.
///
/// THREE THINGS MAKE IT WORSE THAN THE `m21` COLLISION, and they are why it is worth a
/// test of its own:
///   * the commutative collision loses one message; this loses the whole object;
///   * `event_author`'s dry-run probe cannot see it — the probe folds the AUTHOR's log,
///     which has no conflict, so the write is accepted and reported as success;
///   * there is no repair path in core. The log is append-only and nothing prunes it.
///
/// SINCE 2b3746e (NC-65's guard) THE LAPTOP MUST HOLD THE CHAIN FIRST. A device that joined
/// after the object began and holds no head is refused its sequenced ops
/// (`an_owners_late_device_is_refused_rather_than_forking_an_empty_spine`, below). So the
/// phone writes once, and both devices fold that head, before the laptop goes to sleep:
/// the guard cannot see a fork between two devices that each hold the chain, and this is
/// that fork.
///
/// WHAT WOULD FIX IT is a per-leaf discriminator in the spine position, or a lease on the
/// spine, or a relay commit-slot for sequenced deltas the way there already is one for
/// MLS commits. That is a decision, not a patch, so this states the cost instead of
/// hiding it. The day the scheme changes, this test fails and somebody comes back here.
#[tokio::test]
async fn two_unsynced_devices_of_the_owner_fork_the_spine_for_everyone() {
    let h = Harness::people(&[("axel", &["phone", "laptop"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");
    let szonja = h.device("szonja", "phone");
    h.pair(phone, szonja).await;

    let feast = h.form_object(phone, "event", &[szonja]).await;
    h.add_to_forum(phone, laptop, &feast).await;
    h.settle().await;

    // The spine's first delta, folded by both devices: each now holds the chain.
    h.node(phone)
        .apply(
            &feast,
            OP_SET_PROFILE,
            event::set_profile_args("Feast", None, START_MS, None, None, None, None),
        )
        .await
        .unwrap();
    h.settle().await;
    assert_eq!(h.epoch(phone, &feast), h.epoch(laptop, &feast));
    assert_eq!(h.node(laptop).event_state(&feast).unwrap().title, "Feast");

    // ASLEEP, for real. `apply` drains to head before it seals ("converge before
    // sealing", node.rs) — which the nine typed doors it replaced did not do. So a
    // laptop that can still reach the relay picks up the phone's delta and lands at
    // the NEXT seq, and no two deltas ever share an (epoch, seq). The fork this test
    // is about needs the laptop genuinely off the network, not merely un-settled.
    h.partition(laptop);

    // The phone renames the feast. NO SETTLE — the laptop is still asleep.
    h.node(phone)
        .apply(
            &feast,
            OP_SET_PROFILE,
            event::set_profile_args("Feast Xalapa", None, START_MS, None, None, None, None),
        )
        .await
        .unwrap();

    // The laptop renames it too, from a fold that has never seen the phone's rename.
    h.node(laptop)
        .apply(
            &feast,
            OP_SET_PROFILE,
            event::set_profile_args("Feast Cádiz", None, START_MS, None, None, None, None),
        )
        .await
        .expect(
            "the write is ACCEPTED — the dry-run probe folds the laptop's own log, where \
             there is no conflict yet. If this now fails, a spine-position check was \
             added at author time and the rest of this test is the thing to rewrite",
        );

    // Back on the network: now both deltas reach everyone, at one (epoch, seq).
    h.rejoin(laptop);
    h.settle().await;

    for u in [phone, laptop, szonja] {
        let err = h.node(u).event_state(&feast).expect_err(&format!(
            "{} still folds Feast Xalapa. If the spine now tolerates two deltas at one \
             (epoch, seq), delete this test and say so in m21 as well",
            h.device_name(u)
        ));
        assert!(
            format!("{err}").contains("ForkDetected"),
            "{} failed for some other reason: {err}",
            h.device_name(u)
        );
    }

    // And it is PERMANENT: the log is append-only, nothing prunes it, and syncing more
    // only spreads it. szonja is collateral — she never authored anything.
    h.settle().await;
    assert!(
        h.node(szonja).event_state(&feast).is_err(),
        "another sync does not repair a forked spine"
    );
}

/// WHAT 2b3746e (NC-65's guard) CHANGED. This was the scenario above until then: the
/// laptop, added after the feast began, still holding an EMPTY spine, wrote from its own
/// fold while asleep, and the two deltas forked the spine for everyone. The laptop cannot
/// tell an empty chain from one it cannot see, so its sequenced write is now refused, in
/// the guard's words, and nothing forks: the phone's write stands everywhere, and once
/// the laptop has synced and holds the chain, it writes at the next seat.
#[tokio::test]
async fn an_owners_late_device_is_refused_rather_than_forking_an_empty_spine() {
    let h = Harness::people(&[("axel", &["phone", "laptop"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");
    let szonja = h.device("szonja", "phone");
    h.pair(phone, szonja).await;

    let feast = h.form_object(phone, "event", &[szonja]).await;
    h.add_to_forum(phone, laptop, &feast).await;
    h.settle().await;

    // Both devices are at the same epoch with the same (empty) spine.
    assert_eq!(h.epoch(phone, &feast), h.epoch(laptop, &feast));
    assert!(h.node(laptop).event_state(&feast).unwrap().title.is_empty());

    h.partition(laptop);
    h.node(phone)
        .apply(
            &feast,
            OP_SET_PROFILE,
            event::set_profile_args("Feast Xalapa", None, START_MS, None, None, None, None),
        )
        .await
        .unwrap();

    let refused = h
        .node(laptop)
        .apply(
            &feast,
            OP_SET_PROFILE,
            event::set_profile_args("Feast Cádiz", None, START_MS, None, None, None, None),
        )
        .await
        .expect_err("the laptop holds no head and joined after the feast began: refused, since 2b3746e");
    assert!(
        refused.to_string().contains(
            "this device joined after this object began, and can't see its current state; \
             sign in on a device that holds it"
        ),
        "refused for some other reason: {refused}"
    );

    // Back on the network: nothing forked, and the phone's write stands everywhere.
    h.rejoin(laptop);
    h.settle().await;
    for u in [phone, laptop, szonja] {
        let state = h.node(u).event_state(&feast).unwrap_or_else(|e| {
            panic!("{} does not fold the feast: {e}", h.device_name(u))
        });
        assert_eq!(state.title, "Feast Xalapa", "{}", h.device_name(u));
    }

    // Holding the chain now, the laptop writes at the next seat, and everyone folds it.
    h.node(laptop)
        .apply(
            &feast,
            OP_SET_PROFILE,
            event::set_profile_args("Feast Cádiz", None, START_MS, None, None, None, None),
        )
        .await
        .unwrap();
    h.settle().await;
    for u in [phone, laptop, szonja] {
        assert_eq!(h.node(u).event_state(&feast).unwrap().title, "Feast Cádiz", "{}", h.device_name(u));
    }
}

/// EVENT OP IDS 5 AND 6 ARE LENT, NOT MISSING. Asserted against both op tables so the
/// claim in this file's header cannot rot.
#[tokio::test]
async fn event_ops_five_and_six_are_the_contact_spliced_ticket_legs() {
    use pacific_core::contact::ContactType;
    use pacific_core::event::EventType;

    // The Event's OWN table has 0,1,2,3,4,7, the co-host ops 8-13 (W-98, ICD 2.3.1), and
    // nothing at 5 or 6. FACET ops are
    // excluded deliberately: they live in reserved high bands and belong to the facet,
    // not to Event. Pinning the whole list meant this assertion re-broke every time a
    // facet reached Event (geo, then hosting) — a guard that rots on unrelated work is
    // drift with a delay on it, so it now pins the kind-declared ids it is about.
    let own: Vec<u32> =
        EventType::ops().iter().map(|d| d.op_id).filter(|id| *id < 0x1000).collect();
    assert_eq!(
        own,
        vec![0, 1, 2, 3, 4, 7, 8, 9, 10, 11, 12, 13],
        "the Event op table changed shape; this file's header explains 5/6 and must move with it"
    );
    assert!(EventType::op(5).is_none(), "the Event does not own op 5");
    assert!(EventType::op(6).is_none(), "the Event does not own op 6");

    // Because event.rs spent them in CONTACT's op space, where they are live.
    assert_eq!(event::OP_REQUEST_TICKET, 5);
    assert_eq!(event::OP_DELIVER_TICKET, 6);
    assert_eq!(
        ContactType::op(5).expect("contact op 5").name,
        "contact.requestTicket"
    );
    assert_eq!(
        ContactType::op(6).expect("contact op 6").name,
        "contact.deliverTicket"
    );
    // The same lending covers invite / inviteReply / admitTicket.
    assert_eq!(event::OP_INVITE, 9);
    assert_eq!(event::OP_INVITE_REPLY, 10);
    assert_eq!(event::OP_ADMIT_TICKET, 11);
    for (id, name) in [
        (9u32, "contact.invite"),
        (10, "contact.inviteReply"),
        (11, "contact.admitTicket"),
    ] {
        assert_eq!(ContactType::op(id).expect("contact op").name, name);
    }
}
