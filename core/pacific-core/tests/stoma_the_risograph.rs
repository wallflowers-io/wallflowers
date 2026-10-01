//! The Risograph, as a **Thing (27)** — and a correction to the map.
//!
//! THE BRIEF SAID: "`setProfile` and `setPhoto` implemented; `setPosture`/`clearPosture`
//! are SPEC ONLY, so the market half cannot be driven. Confirm that rather than working
//! around it."
//!
//! IT IS NOT SO, and the confusion is worth naming precisely because it would otherwise
//! be read off the ICD again. `x-graph.status` in `coordination/delta-graph.icd.json` is
//! a statement about the GRAPH PROJECTOR, not about pacific-core. The document says so
//! itself:
//!
//! > STATUS. `implemented` means the projector does this today. `specified` means this
//! > document is ahead of the code, and the gap is the work list. The conformance test
//! > enforces the op inventory, never the status field.
//!
//! The projector is `arc-resolver/src/project.rs`, a different crate, and it mints four
//! relations today (`member_of`, `affiliated_with`, `happens_at`, `created`). So
//! `status: "specified"` on `thing.setPosture` means "the graph does not yet mint an
//! `offers` edge for it" — it says nothing about the reducer, which has been live all
//! along. `catalogue_reach.rs` sweeps all 43 of them and finds a reducer for every one.
//!
//! So the market half DOES drive, and this binary drives it: `setProfile` (0),
//! `setPosture` (1), `clearPosture` (2) and `setPhoto` (3), across two of axel's devices.
//!
//! Cast from `app/web/docs/seeds/stoma.js`: the zine press, listed out of Cádiz.

mod common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::thing::{Posture, Reach, OP_SET_PHOTO};

const PHOTO: &str = "cmlzby1waG90bw==";

/// The press goes on the market from axel's phone, is photographed from his laptop, and
/// comes off the market from the phone again. All four Thing ops, both devices.
#[tokio::test]
async fn the_risograph_goes_on_the_market_and_comes_off_again() {
    let h = Harness::people(&[("axel", &["phone", "laptop"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");

    // `mint_thing` is the swipe deck's ACCEPT: a group of one, then `setProfile`.
    let riso = h.mint_thing(phone, "Risograph MZ 1070").await;
    h.add_to_forum(phone, laptop, &riso).await;
    h.settle().await;
    assert_eq!(h.device_leaves(phone, &riso), 2, "two leaves, one person");

    // ── setPosture (1) — the op the brief said could not be driven ────────────
    // A price is only legal on an ACTIVE posture (`Posture::is_active` — buying or
    // selling); a standing `offers`/`wants` carries none, and the reducer refuses one
    // rather than dropping it.
    h.set_posture(
        phone,
        &riso,
        Posture::Selling,
        Some("EUR 900 or swap for a guillotine"),
        Reach::Network,
    )
    .await;
    h.settle().await;

    for u in [phone, laptop] {
        let st = h.node(u).thing_state(&riso).unwrap();
        assert_eq!(st.name, "Risograph MZ 1070", "{}", h.device_name(u));
        assert_eq!(
            st.posture,
            Some(Posture::Selling),
            "{} does not hold the posture — thing.setPosture is owner/sequenced and \
             must fold on every leaf. The ICD's `specified` status is about the graph \
             projector, not this reducer",
            h.device_name(u)
        );
        assert_eq!(st.price.as_deref(), Some("EUR 900 or swap for a guillotine"));
        assert_eq!(st.reach, Reach::Network);
    }

    // ── setPhoto (3), from the SECOND device ──────────────────────────────────
    let mut a = Args::new();
    a.insert("photo".into(), ArgVal::Text(PHOTO.into()));
    a.insert("mime".into(), ArgVal::Text("image/jpeg".into()));
    h.node(laptop)
        .apply(&riso, OP_SET_PHOTO, a)
        .await
        .expect("axel's laptop carries his owner authority on a Thing spine too");
    h.settle().await;
    for u in [phone, laptop] {
        let st = h.node(u).thing_state(&riso).unwrap();
        assert_eq!(st.photo, PHOTO, "{}", h.device_name(u));
        assert_eq!(st.photo_mime, "image/jpeg");
        assert_eq!(
            st.posture,
            Some(Posture::Selling),
            "{} — a photo must not disturb the posture",
            h.device_name(u)
        );
    }

    // ── clearPosture (2) — sold, and it takes the qualifiers with it ──────────
    h.clear_posture(phone, &riso).await;
    h.settle().await;
    for u in [phone, laptop] {
        let st = h.node(u).thing_state(&riso).unwrap();
        assert_eq!(st.posture, None, "{} still lists it", h.device_name(u));
        assert_eq!(st.price, None, "the price qualified the posture and goes with it");
        assert_eq!(
            st.reach,
            Reach::Private,
            "and the reach falls back to Private, so a later posture cannot inherit an \
             audience nobody stated"
        );
        assert_eq!(
            st.name, "Risograph MZ 1070",
            "{} — the THING is still there. Clearing a posture is not a deletion: you \
             can know about a thing without taking a market position on it",
            h.device_name(u)
        );
        assert_eq!(st.photo, PHOTO, "and it keeps its picture");
    }

    // One object, one epoch.
    assert_eq!(h.epoch(phone, &riso), h.epoch(laptop, &riso));
}

/// THE MINT'S OPENING PROFILE REACHES THE FIRST PERSON YOU SHARE WITH AND NOBODY AFTER.
///
/// Every kind `Node::mint` can create is minted as a GROUP OF ONE and then given its op 0
/// immediately (`mint::profile_args` → `author_profile`). `append_and_flush` returns
/// early while `group_leaf_count <= 1` — with one leaf there is nowhere to publish — so
/// that delta sits in the outbox until the next `sync_once`, which flushes it AT THE
/// THEN-CURRENT EPOCH.
///
/// So the arithmetic is: adding the FIRST member takes the object to two leaves, the
/// outbox drains at that epoch, and they can read it. Adding a SECOND member advances
/// the epoch again — and the opening profile was flushed, acked and forgotten an epoch
/// ago. MLS forward secrecy does the rest: `add_member_core` re-encrypts nothing, by
/// design. The second person you share a minted object with gets a NAMELESS object.
///
/// RESOLVED 25 Sep 2026: `add_member_core` now ends every Add with state re-emission
/// (`reemit_own_spine`), so the adder's durable state reaches each newcomer at the
/// epoch it holds. This test now pins that.
///
/// It is not a Thing problem. The same shape is in every `Node::mint` path — a Group, an
/// Event, a Place, a Post — and it is invisible from the outside because the owner's own
/// devices, being added first, always look fine.
#[tokio::test]
async fn the_second_person_added_to_a_minted_thing_learns_its_name_by_re_emission() {
    let h = Harness::people(&[("axel", &["phone", "laptop"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");
    let szonja = h.device("szonja", "phone");
    h.pair(phone, szonja).await;

    let riso = h.mint_thing(phone, "Risograph MZ 1070").await;
    assert_eq!(h.leaves(phone, &riso), 1, "minted as a group of one");
    assert_eq!(
        h.node(phone).dir.load_log(&hex::decode(&riso).unwrap()).unwrap().len(),
        1,
        "and its op 0 is already written locally"
    );

    // FIRST add: the outbox drains at this epoch, so the laptop reads the name.
    h.add_to_forum(phone, laptop, &riso).await;
    h.settle().await;
    assert_eq!(
        h.node(laptop).thing_state(&riso).unwrap().name,
        "Risograph MZ 1070",
        "the FIRST device added must get the opening profile — if this is empty the \
         outbox gate in `append_and_flush` changed and every mint is now silent"
    );

    // SECOND add: one epoch too late.
    h.add_to_forum(phone, szonja, &riso).await;
    h.settle().await;
    assert_eq!(
        h.device_leaves(szonja, &riso),
        3,
        "szonja really is a leaf — this is not a failed add"
    );
    // RESOLVED (25 Sep): every Add now restates the adder's durable state at the
    // newcomer's epoch (`reemit_own_spine`, the doorbell's discipline), so the second
    // person is no longer handed a nameless object.
    assert!(
        !h.node(szonja).dir.load_log(&hex::decode(&riso).unwrap()).unwrap().is_empty(),
        "the Add re-emitted the opening profile to her"
    );
    assert_eq!(
        h.node(szonja).thing_state(&riso).unwrap().name,
        "Risograph MZ 1070",
        "so szonja holds a named Risograph"
    );

    // A later profile write still reaches everyone, as any write does.
    h.node(phone)
        .apply(&riso, pacific_core::thing::OP_SET_PROFILE, {
            let mut a = Args::new();
            a.insert("name".into(), ArgVal::Text("Risograph MZ 1070".into()));
            a.insert("descriptor".into(), ArgVal::Text("drum needs a clean".into()));
            a
        })
        .await
        .unwrap();
    h.settle().await;
    for u in [phone, laptop, szonja] {
        assert_eq!(
            h.node(u).thing_state(&riso).unwrap().name,
            "Risograph MZ 1070",
            "{} converges once the owner re-states the profile",
            h.device_name(u)
        );
    }
}

/// THE ONE REAL GATE ON THE MARKET HALF, and it is a reducer rule rather than a missing
/// op: a price on a STANDING intent is refused, because a Want has no price until it
/// becomes Buying.
///
/// `thing_author` dry-runs the reducer before it persists, so this arrives as an error
/// instead of as a signed delta that does nothing forever — the discipline
/// `group_author` is missing (see `stoma_group_is_the_directory`).
#[tokio::test]
async fn a_price_on_a_standing_intent_is_refused_at_author_time() {
    let h = Harness::people(&[("axel", &["phone", "laptop"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");

    let riso = h.mint_thing(phone, "Risograph MZ 1070").await;
    h.add_to_forum(phone, laptop, &riso).await;
    h.settle().await;

    let before = h
        .node(phone)
        .dir
        .load_log(&hex::decode(&riso).unwrap())
        .unwrap()
        .len();
    let mut a = Args::new();
    a.insert("posture".into(), ArgVal::Text(Posture::Wants.as_str().into()));
    a.insert("price".into(), ArgVal::Text("EUR 900".into()));
    a.insert("reach".into(), ArgVal::Text(Reach::Network.as_str().into()));
    let err = h
        .node(phone)
        .apply(&riso, pacific_core::thing::OP_SET_POSTURE, a)
        .await
        .expect_err("a price on `wants` must be refused, not silently dropped");
    assert!(
        format!("{err}").contains("PreconditionFailed"),
        "the refusal must be the posture rule: {err}"
    );

    assert_eq!(
        h.node(phone).dir.load_log(&hex::decode(&riso).unwrap()).unwrap().len(),
        before,
        "and NO DELTA WAS WRITTEN. `thing_author` dry-runs the reducer before it \
         persists, so a precondition failure costs nothing; `group_author` has no such \
         probe and would have signed this into the log forever (see \
         `stoma_group_is_the_directory::stomas_edge_to_facc_cadiz_is_set_and_then_cleared`)"
    );
    assert_eq!(
        h.node(laptop).thing_state(&riso).unwrap().posture,
        None,
        "so nothing folded either"
    );

    // The same posture WITHOUT a price is fine, from either device.
    h.set_posture(laptop, &riso, Posture::Wants, None, Reach::Network)
        .await;
    h.settle().await;
    assert_eq!(
        h.node(phone).thing_state(&riso).unwrap().posture,
        Some(Posture::Wants)
    );
    assert_eq!(h.node(phone).thing_state(&riso).unwrap().price, None);
}
