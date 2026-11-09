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

use crate::common;

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
    // sync; when it is missing, the relay's slot settles the seat instead
    // (`two_unsynced_devices_of_the_owner_do_not_fork_the_spine`).
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

/// TWO OF ONE PERSON'S DEVICES THAT HAVE NOT SYNCED DO NOT FORK AN OWNER-SEQUENCED SPINE.
///
/// `event_next_seq` derives `(seq, prev)` from THIS DEVICE's fold of the spine. Both of
/// axel's devices are the owner, so both pass every authority check, and a laptop that
/// has not folded the phone's delta computes the same seat. Until 8 Nov this test pinned
/// what followed: both deltas reached everyone at one `(epoch, seq)`, the fold stopped on
/// `ForkDetected`, and the feast was unreadable for good, for szonja too, who did nothing.
/// On Ralph's own record that became a week-long write loop.
///
/// THE SCHEME CHANGED, as this comment said it would have to: a sequenced delta claims
/// its seat's slot on the relay before it is sent, first writer wins (ICD
/// `mls.sequencing.slot`; RFC 9750 §5.2.1), as the spine's links and every MLS commit
/// already did. The laptop's rename waits in its outbox while it is asleep. Back on the
/// network it finds the phone holding the seat, takes the phone's rename, and is authored
/// again at the next one: both renames stand, in the order the relay saw them, and the
/// laptop's first attempt never left it.
///
/// SINCE 2b3746e (NC-65's guard) THE LAPTOP MUST HOLD THE CHAIN FIRST
/// (`an_owners_late_device_is_refused_rather_than_forking_an_empty_spine`, below), which is
/// why the phone writes once, and both devices fold it, before the laptop goes to sleep.
#[tokio::test]
async fn two_unsynced_devices_of_the_owner_do_not_fork_the_spine() {
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

    // The laptop renames it too, from a fold that has never seen the phone's rename. It is
    // accepted, and waits: no relay, so no slot, so nothing sent.
    h.node(laptop)
        .apply(
            &feast,
            OP_SET_PROFILE,
            event::set_profile_args("Feast Cádiz", None, START_MS, None, None, None, None),
        )
        .await
        .expect("a write made offline is accepted, and queued");

    // Back on the network: the seat is the phone's, and the laptop's rename follows it.
    h.rejoin(laptop);
    h.settle().await;
    h.settle().await;

    let renames = |u| {
        h.node(u)
            .object_log(&feast)
            .unwrap()
            .iter()
            .filter(|(op, ..)| *op == OP_SET_PROFILE)
            .count()
    };
    for u in [phone, laptop, szonja] {
        let st = h.node(u).event_state(&feast).unwrap_or_else(|e| {
            panic!("{} cannot fold the feast: {e}", h.device_name(u))
        });
        assert_eq!(
            st.title, "Feast Cádiz",
            "{}: the laptop's rename, authored again after the phone's, is the last",
            h.device_name(u)
        );
        assert_eq!(
            renames(u),
            3,
            "{}: Feast, Feast Xalapa and Feast Cádiz, one seat each; the laptop's first \
             attempt at the phone's seat never left it",
            h.device_name(u)
        );
    }
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

/// TWO OF THE OWNER'S DEVICES, BOTH ONLINE, AT ONE SEAT: each commits its rename locally
/// from the same head (the Door's write path, `apply_local`), then sends. The relay gives
/// the seat to whichever claims it first; the other finds it taken, takes the winner, and
/// is authored again at the next seat. Both renames stand; neither is lost.
#[tokio::test]
async fn the_loser_of_a_seat_is_authored_again_not_lost() {
    let h = Harness::people(&[("axel", &["phone", "laptop"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");
    let szonja = h.device("szonja", "phone");
    h.pair(phone, szonja).await;
    let feast = h.form_object(phone, "event", &[szonja]).await;
    h.add_to_forum(phone, laptop, &feast).await;
    let rename = |title: &str| event::set_profile_args(title, None, START_MS, None, None, None, None);
    h.node(phone).apply(&feast, OP_SET_PROFILE, rename("Feast")).await.unwrap();
    h.settle().await;

    h.node(phone).apply_local(&feast, OP_SET_PROFILE, rename("Feast Xalapa")).await.unwrap();
    h.node(laptop).apply_local(&feast, OP_SET_PROFILE, rename("Feast Cádiz")).await.unwrap();
    h.node(phone).publish_pending(&feast).await.expect("the phone takes the seat");
    h.node(laptop).publish_pending(&feast).await.expect("the laptop finds it taken, and goes after it");
    h.settle().await;
    h.settle().await;

    for u in [phone, laptop, szonja] {
        let st = h.node(u).event_state(&feast).unwrap_or_else(|e| panic!("{}: {e}", h.device_name(u)));
        assert_eq!(st.title, "Feast Cádiz", "{}: the laptop's rename follows the phone's", h.device_name(u));
        let renames = h.node(u).object_log(&feast).unwrap().iter().filter(|(op, ..)| *op == OP_SET_PROFILE).count();
        assert_eq!(renames, 3, "{}: one seat each", h.device_name(u));
    }
}

/// A FORK FROM BEFORE THE SLOT, or from a client without it: two of the owner's deltas
/// already at one seat in everyone's log. Every device folds the same feast, the lowest
/// DeltaId's (ICD `mls.sequencing.tiebreak`; RFC 9750 §5.2.2), and the next write goes on
/// from it.
#[tokio::test]
async fn a_fork_already_written_folds_the_same_everywhere_and_is_written_past() {
    let h = Harness::people(&[("axel", &["phone", "laptop"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");
    let szonja = h.device("szonja", "phone");
    h.pair(phone, szonja).await;
    let feast = h.form_object(phone, "event", &[szonja]).await;
    h.add_to_forum(phone, laptop, &feast).await;
    h.settle().await;

    let epoch = h.epoch(phone, &feast);
    let tid = pacific_core::object::ObjectKind::Event.type_id() as u32;
    let at_genesis = |title: &str| {
        pacific_core::coordinator::sequenced_delta(
            tid,
            OP_SET_PROFILE,
            event::set_profile_args(title, None, START_MS, None, None, None, None),
            epoch,
            0,
            pacific_core::coordinator::GENESIS_PREV,
        )
    };
    let (a, b) = (at_genesis("Feast Xalapa"), at_genesis("Feast Cádiz"));
    let stands = if a.id() < b.id() { "Feast Xalapa" } else { "Feast Cádiz" };
    let axel = h.id(phone);
    for u in [phone, laptop, szonja] {
        let n = h.node(u);
        let gid = hex::decode(&feast).unwrap();
        for d in [&a, &b] {
            n.dir.append_delta(&gid, &d.id(), &axel, &d.canonical_bytes(), 0).unwrap();
            n.dir.mark_delivered(&gid, &d.id()).unwrap();
        }
    }
    for u in [phone, laptop, szonja] {
        let st = h.node(u).event_state(&feast).unwrap_or_else(|e| panic!("{}: {e}", h.device_name(u)));
        assert_eq!(st.title, stands, "{}: the lowest DeltaId stands", h.device_name(u));
    }

    h.node(laptop)
        .apply(&feast, OP_SET_PROFILE, event::set_profile_args("Feast", None, START_MS, None, None, None, None))
        .await
        .expect("the owner writes on from the seat that stands");
    h.settle().await;
    for u in [phone, laptop, szonja] {
        assert_eq!(h.node(u).event_state(&feast).unwrap().title, "Feast", "{}", h.device_name(u));
    }
}

/// THE OUTBOX WALK SENDS EACH QUEUED SEQUENCED DELTA ONCE (#72 review, B1). The walk goes
/// over a snapshot of the outbox; the first sequenced delta sends its whole queue through
/// the slots, and the later ones in the snapshot are done, not published again. Two
/// writes: two claims and two publishes, exactly.
#[tokio::test]
async fn a_queue_of_sequenced_writes_costs_one_claim_and_one_publish_each() {
    let h = Harness::people(&[("axel", &["phone", "laptop"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");
    let feast = h.form_object(phone, "event", &[]).await;
    h.add_to_forum(phone, laptop, &feast).await;
    let rename = |title: &str| event::set_profile_args(title, None, START_MS, None, None, None, None);
    h.node(phone).apply(&feast, OP_SET_PROFILE, rename("Feast")).await.unwrap();
    h.settle().await;

    h.node(phone).apply_local(&feast, OP_SET_PROFILE, rename("Feast Xalapa")).await.unwrap();
    h.node(phone).apply_local(&feast, OP_SET_PROFILE, rename("Feast Cádiz")).await.unwrap();
    let before = pacific_core::router::published();
    h.node(phone).publish_pending(&feast).await.expect("the queue goes");
    assert_eq!(pacific_core::router::published() - before, 4, "a claim and a publish for each of the two");
    h.settle().await;
    assert_eq!(h.node(laptop).event_state(&feast).unwrap().title, "Feast Cádiz");
}

/// AFTER A LOST SEAT, ONLY THE RE-AUTHORED QUEUE GOES (#72 review, B1). Two renames queued
/// on the laptop while it slept; the phone took the seat after their predecessor. The
/// laptop's queue is authored again from the phone's rename and sent; the snapshot's
/// first copies, dropped, are never published, so nothing forks and nothing is set aside.
#[tokio::test]
async fn after_a_lost_seat_the_dropped_copies_are_never_published() {
    let h = Harness::people(&[("axel", &["phone", "laptop"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");
    let szonja = h.device("szonja", "phone");
    h.pair(phone, szonja).await;
    let feast = h.form_object(phone, "event", &[szonja]).await;
    h.add_to_forum(phone, laptop, &feast).await;
    let rename = |title: &str| event::set_profile_args(title, None, START_MS, None, None, None, None);
    h.node(phone).apply(&feast, OP_SET_PROFILE, rename("Feast")).await.unwrap();
    h.settle().await;

    h.partition(laptop);
    h.node(phone).apply(&feast, OP_SET_PROFILE, rename("Feast Xalapa")).await.unwrap();
    h.node(laptop).apply(&feast, OP_SET_PROFILE, rename("Feast Cádiz")).await.unwrap();
    h.node(laptop).apply(&feast, OP_SET_PROFILE, rename("Feast Cádiz, again")).await.unwrap();
    h.rejoin(laptop);
    h.settle().await;
    h.settle().await;

    for u in [phone, laptop, szonja] {
        let st = h.node(u).event_state(&feast).unwrap_or_else(|e| panic!("{}: {e}", h.device_name(u)));
        assert_eq!(st.title, "Feast Cádiz, again", "{}", h.device_name(u));
        let renames = h.node(u).object_log(&feast).unwrap().iter().filter(|(op, ..)| *op == OP_SET_PROFILE).count();
        assert_eq!(renames, 4, "{}: one of each, and no dropped copy", h.device_name(u));
    }
}

/// A BACKED-OFF OBJECT STILL TAKES THE PERSON'S OWN WRITE (#75). A slot held by a claim this
/// device cannot read fails the queue and backs the object off. A sweep (`publish_all_pending`,
/// as the Door's tail and a sync tick do) then waits, sends nothing and counts nothing; the
/// person's own `publish_pending` on that object claims on that call all the same, and its
/// failure is an error — the Door answers a write as relayed on it (NC-93). RFC 9420 §14:
/// back off the loser, not the person.
#[tokio::test]
async fn a_backed_off_object_still_takes_the_persons_own_write() {
    let h = Harness::people(&[("axel", &["phone", "laptop"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");
    let feast = h.form_object(phone, "event", &[]).await;
    h.add_to_forum(phone, laptop, &feast).await;
    let rename = |title: &str| event::set_profile_args(title, None, START_MS, None, None, None, None);
    h.node(phone).apply(&feast, OP_SET_PROFILE, rename("Feast")).await.unwrap();
    h.settle().await;

    let n = h.node(phone);
    n.apply_local(&feast, OP_SET_PROFILE, rename("Feast Xalapa")).await.unwrap();
    let gid = hex::decode(&feast).unwrap();
    let queued = n.dir.pending_outbox(&gid, &n.id.identity_pk()).unwrap();
    let (did, env) = queued.first().cloned().expect("the rename waits in the outbox");
    let prev = pacific_core::coordinator::decode_delta(&env).unwrap().prev;

    // Someone else's bytes in the place after the head: a claim this device cannot read.
    let root = pacific_core::locator::storage_root(&n.id.seed_bytes().expect("a seed"));
    let slot = pacific_core::node::sequenced_slot(&root, &gid, &prev);
    let mut relay = pacific_core::transport::RelaySession::connect(&h.relay).await.unwrap();
    assert!(relay.publish_commit(&slot, pacific_wire::blob_b64(b"not a claim")).await.unwrap().is_some());

    let wire = || pacific_core::router::published();
    let before = wire();
    assert!(n.publish_pending(&feast).await.is_err(), "the person's write is not at the relay, and says so");
    assert_eq!(wire() - before, 1, "it claimed, once");

    let before = wire();
    assert_eq!(n.publish_all_pending().await.unwrap(), 0, "a sweep inside the backoff counts nothing");
    assert_eq!(wire() - before, 0, "and sends nothing");
    assert!(n.dir.pending_outbox(&gid, &n.id.identity_pk()).unwrap().iter().any(|(d, _)| *d == did), "the rename still waits");

    // A sync tick inside the backoff, on the same Node (the backoff is held in memory, as a
    // Door session or the app holds one Node): the object's queue waits, and nothing goes out.
    let before = wire();
    n.sync_once().await.unwrap();
    assert_eq!(wire() - before, 0, "a tick sends nothing for a backed-off object");
    assert!(n.dir.pending_outbox(&gid, &n.id.identity_pk()).unwrap().iter().any(|(d, _)| *d == did), "the rename still waits");

    let before = wire();
    assert!(n.publish_pending(&feast).await.is_err(), "inside the backoff, the person's write is tried again");
    assert_eq!(wire() - before, 1, "it claimed again on that call");
}
