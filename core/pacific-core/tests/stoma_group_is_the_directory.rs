//! STOMA, as a **Group (18)** — the real directory object, driven from real devices.
//!
//! Everything before this binary proved multidevice on Forums and notes. This drives
//! `ObjectKind::Group` itself: `setProfile`, `setPresence`, `setCover`,
//! `setAffiliation`/`clearAffiliation` and `setMemberRole`, through `Node::group_author`,
//! from two people and three devices, with axel holding a phone AND a laptop.
//!
//! THE CAST is the one from `app/web/docs/seeds/stoma.js`: axel owns STOMA, szonja is
//! its admin, FACC Cádiz is the partner space in Andalucía. The names are there so a
//! failure reads as something that happened rather than as `test_group_3`.
//!
//! WHAT A GROUP OP NEEDS, and it is the thing every scenario here turns on: all twelve
//! Group ops are `owner`/`sequenced`, and `group_author` refuses an owner-only op from
//! a non-owner (node.rs, "group op '…' is owner-only"). A person's SECOND DEVICE is not
//! a second person — `Harness::people` restores it from the first device's 24 words, so
//! `identity_pk()` is byte-identical — which means the laptop passes the owner gate on
//! the phone's object. That is asserted here rather than assumed, because it is the
//! whole reason multidevice is worth having on an owner-sequenced kind.

mod common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::group::{
    OP_CLEAR_AFFILIATION, OP_SET_AFFILIATION, OP_SET_COVER, OP_SET_PRESENCE,
};
// Standing is a facet now, carried by every kind that wants it — see `m32`/`roles`.
use pacific_core::roles::OP_SET_ROLE;

/// A tiny base64 stand-in for a cover image. The reducer caps SIZE and closes the MIME
/// vocabulary; it does not care whether the bytes decode to a picture.
const COVER: &str = "Y292ZXItYnl0ZXM=";

/// axel's phone writes STOMA's profile; axel's laptop and szonja's phone read it.
///
/// The first half of the brief's Group claim: a person's second device sees a profile
/// change made on the first. It is NOT a claim about sharing a store — the two devices
/// have separate SQLite files and separate MLS signing keys — it is a claim that one
/// owner-sequenced delta, authored on one leaf, folds identically on the other two.
#[tokio::test]
async fn axels_laptop_reads_the_profile_his_phone_wrote() {
    let h = Harness::people(&[("axel", &["phone", "laptop"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");
    let szonja = h.device("szonja", "phone");
    h.pair(phone, szonja).await;

    // STOMA is minted as a group of ONE and then opened: the laptop and szonja are
    // added the same way, through a real staged MLS commit.
    let stoma = h.mint_group(phone);
    h.add_to_forum(phone, laptop, &stoma).await;
    h.add_to_forum(phone, szonja, &stoma).await;

    // The profile is written AFTER both are leaves, so nothing here can be explained
    // by the outbox (see `a_solo_objects_deltas_wait_for_somewhere_to_send_them`).
    h.group_set_profile(phone, &stoma, "STOMA", "organisation")
        .await;
    h.settle().await;

    for u in [phone, laptop, szonja] {
        let v = h.group_view(u, &stoma);
        assert_eq!(
            v.display_name,
            "STOMA",
            "{} does not hold the profile axel's phone authored — an owner-sequenced \
             delta must fold identically on every leaf, including the owner's other device",
            h.device_name(u)
        );
        assert_eq!(v.shape, pacific_core::group::GroupShape::Organisation);
    }

    // And a SECOND edit lands on top: LWW on the sequenced spine, not a first-write-wins.
    h.group_set_profile(phone, &stoma, "STOMA (Cádiz · Xalapa)", "organisation")
        .await;
    h.settle().await;
    assert_eq!(
        h.group_view(laptop, &stoma).display_name,
        "STOMA (Cádiz · Xalapa)",
        "the laptop follows the phone's later edit too"
    );
}

/// THE OWNER GATE AND THE SECOND DEVICE. axel's laptop authors `group.setCover` onto an
/// object axel's PHONE minted, and it is accepted — because authority is keyed on the
/// person's identity key, which both devices carry.
///
/// szonja is a full member of the same object and is refused the same op, which is what
/// makes the first half mean anything: the laptop is not passing because the gate is
/// open, it is passing because it is axel.
#[tokio::test]
async fn the_laptop_carries_axels_authority_and_szonjas_phone_does_not() {
    let h = Harness::people(&[("axel", &["phone", "laptop"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");
    let szonja = h.device("szonja", "phone");
    h.pair(phone, szonja).await;

    let stoma = h.mint_group(phone);
    h.add_to_forum(phone, laptop, &stoma).await;
    h.add_to_forum(phone, szonja, &stoma).await;
    h.group_set_profile(phone, &stoma, "STOMA", "organisation")
        .await;
    h.settle().await;

    // ── the laptop authors, and it is axel who authored ───────────────────────
    let mut a = Args::new();
    a.insert("data".into(), ArgVal::Text(COVER.into()));
    a.insert("mime".into(), ArgVal::Text("image/jpeg".into()));
    h.node(laptop)
        .apply(&stoma, OP_SET_COVER, a)
        .await
        .expect(
            "axel's LAPTOP was refused an owner-only Group op. Both devices restore one \
             identity, so `group_owner_id() == identity_pk()` must hold on both — if this \
             is the owner-only refusal, per-device identity has leaked into authority",
        );
    h.settle().await;

    for u in [phone, laptop, szonja] {
        assert_eq!(
            h.group_view(u, &stoma).cover,
            COVER,
            "{} does not hold the cover axel set from the laptop",
            h.device_name(u)
        );
        assert_eq!(h.group_view(u, &stoma).cover_mime, "image/jpeg");
    }

    // ── szonja is a member, and still cannot ──────────────────────────────────
    let mut a = Args::new();
    a.insert("data".into(), ArgVal::Text(String::new()));
    let err = h
        .node(szonja)
        .apply(&stoma, OP_SET_COVER, a)
        .await
        .expect_err(
            "szonja cleared STOMA's cover. `group.setCover` is owner/sequenced and she \
             is a MEMBER, not the owner — membership is not authority",
        );
    assert!(
        format!("{err}").contains("owner-only"),
        "the refusal must be the owner gate, not an unrelated failure: {err}"
    );
    assert_eq!(
        h.group_view(phone, &stoma).cover,
        COVER,
        "and the cover is untouched — a refused op leaves no delta behind"
    );
}

/// FOUR LEAVES, TWO PEOPLE. The roster of a Group folds to PEOPLE; the ratchet tree is
/// what has leaves. Both numbers are asserted for the reason `m21` gives: the leaf count
/// alone would pass on a tree that admitted a stranger, and the people count alone would
/// pass on devices that never joined.
#[tokio::test]
async fn stomas_roster_folds_to_people_not_leaves() {
    let h = Harness::people(&[
        ("axel", &["phone", "laptop"]),
        ("szonja", &["phone", "studio"]),
    ])
    .await;
    let axel_phone = h.device("axel", "phone");
    let axel_laptop = h.device("axel", "laptop");
    let sz_phone = h.device("szonja", "phone");
    let sz_studio = h.device("szonja", "studio");
    h.pair(axel_phone, sz_phone).await;

    let stoma = h.mint_group(axel_phone);
    for d in [axel_laptop, sz_phone, sz_studio] {
        h.add_to_forum(axel_phone, d, &stoma).await;
    }
    h.group_set_profile(axel_phone, &stoma, "STOMA", "organisation")
        .await;
    h.settle().await;

    let everyone = [axel_phone, axel_laptop, sz_phone, sz_studio];
    for u in everyone {
        let mut people = h.roster_people(u, &stoma);
        people.sort();
        assert_eq!(
            people,
            vec!["axel", "szonja"],
            "{} reads STOMA as {} people — four leaves must fold to two members, or every \
             count in the system (quorum bands, is_member, MembershipLog::divergence) is \
             two too high",
            h.device_name(u),
            people.len()
        );
        assert_eq!(
            h.device_leaves(u, &stoma),
            4,
            "{} sees the wrong number of LEAVES in STOMA's ratchet tree",
            h.device_name(u)
        );
    }

    // One group, one epoch. Two devices on diverging epochs would be two groups
    // wearing one name, and every agreement above would be a coincidence.
    let e = h.epoch(axel_phone, &stoma);
    for u in everyone {
        assert_eq!(
            h.epoch(u, &stoma),
            e,
            "{} is at a different epoch — STOMA forked",
            h.device_name(u)
        );
    }
}

/// STOMA records its edge to FACC Cádiz, then takes it down.
///
/// `group.setAffiliation` / `clearAffiliation` carry `x-graph.status: "specified"` in the
/// ICD, and this is what that field actually means: the GRAPH PROJECTOR (arc-resolver)
/// does not mint `affiliated_with` yet. The pacific-core reducer is fully live, which is
/// why this test can drive both halves of the edge and read them back.
///
/// The vocabulary is CLOSED — `AffiliationRel` is parent|child|peer|anchored|created|performs_at —
/// and `clearAffiliation` on an edge that was never recorded is a real
/// `PreconditionFailed`, not a no-op. Both are pinned below.
#[tokio::test]
async fn stomas_edge_to_facc_cadiz_is_set_and_then_cleared() {
    let h = Harness::people(&[("axel", &["phone", "laptop"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");

    let stoma = h.mint_group(phone);
    let facc = h.mint_group(phone); // FACC Cádiz, a sovereign group of its own
    h.add_to_forum(phone, laptop, &stoma).await;
    h.group_set_profile(phone, &stoma, "STOMA", "organisation")
        .await;
    h.settle().await;

    let at = 1_713_657_600_000; // 21 Apr 2024, the day lucía's guest tenure starts
    let mut a = Args::new();
    a.insert("peer".into(), ArgVal::Text(facc.clone()));
    a.insert("rel".into(), ArgVal::Text("peer".into()));
    a.insert("name".into(), ArgVal::Text("FACC Cádiz".into()));
    a.insert("at".into(), ArgVal::Int(at));
    h.node(phone)
        .apply(&stoma, OP_SET_AFFILIATION, a)
        .await
        .unwrap();
    h.settle().await;

    let affs = h.group_view(laptop, &stoma).affiliations;
    assert_eq!(
        affs.len(),
        1,
        "axel's laptop does not hold the affiliation his phone authored — \
         group.setAffiliation is owner/sequenced and should fold on every leaf"
    );
    assert_eq!(affs[0].0, facc, "the edge names FACC Cádiz's object id");
    assert_eq!(affs[0].1.name, "FACC Cádiz");
    assert_eq!(affs[0].1.rel, pacific_core::group::AffiliationRel::Peer);
    assert_eq!(affs[0].1.at, at);

    // THE CLOSED VOCABULARY. "partner" is not in `AffiliationRel`, so the reducer
    // rejects the delta. This used to be ACCEPTED, SIGNED, WRITTEN and then INERT at
    // fold, with success reported to the caller: five of the seven author paths probed
    // before persisting and `group_author`/`project_author` did not, and that asymmetry
    // was what this assertion pinned.
    //
    // There is one write path now (`apply` → `authoring::build`), and it probes for
    // every kind, so the asymmetry cannot exist: the write is REFUSED and the log does
    // not grow. What this assertion pins now is the absence of the brick.
    let before = h.node(phone).dir.load_log(&hex::decode(&stoma).unwrap()).unwrap().len();
    let mut a = Args::new();
    a.insert("peer".into(), ArgVal::Text(facc.clone()));
    a.insert("rel".into(), ArgVal::Text("partner".into()));
    a.insert("name".into(), ArgVal::Text("FACC Cádiz".into()));
    a.insert("at".into(), ArgVal::Int(at));
    let refused = h
        .node(phone)
        .apply(&stoma, OP_SET_AFFILIATION, a)
        .await
        .expect_err("the probe must refuse a delta the reducer would reject");
    assert!(
        format!("{refused}").contains("MalformedArgs"),
        "refused, but for the wrong reason: {refused}"
    );
    h.settle().await;
    assert_eq!(
        h.node(phone).dir.load_log(&hex::decode(&stoma).unwrap()).unwrap().len(),
        before,
        "nothing was persisted — the log did not grow by the brick it used to take"
    );
    assert_eq!(
        h.group_view(phone, &stoma).affiliations[0].1.rel,
        pacific_core::group::AffiliationRel::Peer,
        "and the standing edge is untouched"
    );

    // ── taking the edge down ──────────────────────────────────────────────────
    let mut a = Args::new();
    a.insert("peer".into(), ArgVal::Text(facc.clone()));
    h.node(laptop) // from the OTHER device, which is the same owner
        .apply(&stoma, OP_CLEAR_AFFILIATION, a)
        .await
        .unwrap();
    h.settle().await;
    for u in [phone, laptop] {
        assert!(
            h.group_view(u, &stoma).affiliations.is_empty(),
            "{} still holds the edge STOMA cleared",
            h.device_name(u)
        );
    }
}

/// szonja is made STOMA's admin, and the role is an OVERLAY on the live roster — never
/// a grant of membership.
///
/// `group.setMemberRole` is another op the ICD marks `specified`; the reducer has been
/// live all along. Note the identity rendering: `space` takes `ed25519:<hex>` or bare
/// hex, and the retired `space1<hex>` form is refused (`identity::parse_identity_key`).
#[tokio::test]
async fn szonja_is_made_stomas_admin_from_axels_laptop() {
    let h = Harness::people(&[("axel", &["phone", "laptop"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");
    let szonja = h.device("szonja", "phone");
    h.pair(phone, szonja).await;

    let stoma = h.mint_group(phone);
    h.add_to_forum(phone, laptop, &stoma).await;
    h.add_to_forum(phone, szonja, &stoma).await;
    h.group_set_profile(phone, &stoma, "STOMA", "organisation")
        .await;
    h.settle().await;

    let szonja_key = h.with_sync(szonja, |n| n.identity_key());
    assert!(
        szonja_key.starts_with("ed25519:"),
        "the identity rendering moved; `space` args and this assertion move with it"
    );

    let mut a = Args::new();
    a.insert("member".into(), ArgVal::Text(szonja_key.clone()));
    a.insert("role".into(), ArgVal::Text("admin".into()));
    h.node(laptop)
        .apply(&stoma, OP_SET_ROLE, a)
        .await
        .unwrap();
    h.settle().await;

    for u in [phone, laptop, szonja] {
        let roles = h.group_view(u, &stoma).roles;
        assert_eq!(
            roles.len(),
            1,
            "{} sees {} role overlays on STOMA, expected exactly szonja's",
            h.device_name(u),
            roles.len()
        );
        assert_eq!(roles[0].0, h.id(szonja));
        assert_eq!(roles[0].1, pacific_core::group::GroupRole::Admin);
    }

    // A role cannot confer membership: emilio is nobody's leaf here, so `reduce_roles`
    // refuses the overlay (PreconditionFailed) — and `apply` PROBES before it seals, so
    // the write is refused and the delta never enters the log. It used to be authored
    // and fold to nothing, which left a permanent brick on an append-only log for a
    // claim nothing would ever honour.
    let outsider = [9u8; 32];
    let mut a = Args::new();
    a.insert(
        "member".into(),
        ArgVal::Text(format!("ed25519:{}", hex::encode(outsider))),
    );
    a.insert("role".into(), ArgVal::Text("admin".into()));
    let refused = h
        .node(phone)
        .apply(&stoma, OP_SET_ROLE, a)
        .await
        .expect_err("a standing on a non-member must not be writable");
    assert!(
        format!("{refused}").contains("PreconditionFailed"),
        "refused, but for the wrong reason: {refused}"
    );
    h.settle().await;
    assert_eq!(
        h.group_view(phone, &stoma).roles.len(),
        1,
        "a role on a non-member must not land — an overlay the roster does not back \
         is an assertion, not a fact"
    );
}

/// PRESENCE, and the dead vocabulary that still compiles.
///
/// `group.setPresence` onPlatform carries an IdentityKey, and the reducer is the single
/// gate: `parse_identity_key` accepts `ed25519:<hex>` and bare hex and REFUSES the
/// retired `space1<hex>` form. The harness's own `Harness::identity_key()` still renders
/// `space1<hex>`, so a scenario that feeds it to `group_presence_on` writes a delta that
/// is silently inert. Both halves are pinned here so the wart cannot be rediscovered.
#[tokio::test]
async fn presence_takes_an_identity_key_and_refuses_the_retired_rendering() {
    let h = Harness::people(&[("axel", &["phone", "laptop"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");
    let szonja = h.device("szonja", "phone");
    h.pair(phone, szonja).await;

    // paola's directory entry, as STOMA holds it: a Group of shape .individual that is
    // NOT paola's own space — it is what axel records about her.
    let paola = h.mint_group(phone);
    h.add_to_forum(phone, laptop, &paola).await;
    h.group_set_profile(phone, &paola, "paola", "individual")
        .await;
    h.settle().await;

    // Off-platform first: a known contact with no space yet, named and counted.
    h.group_presence_off(phone, &paola, "paola@…").await;
    h.settle().await;
    assert!(
        !h.group_view(laptop, &paola).can_sync,
        "an off-platform entry is not reachable over MLS"
    );

    // On-platform, with the CURRENT rendering.
    let key = h.with_sync(szonja, |n| n.identity_key());
    h.group_presence_on(phone, &paola, &key).await;
    h.settle().await;
    assert!(
        h.group_view(laptop, &paola).can_sync,
        "axel's laptop does not see paola as reachable — setPresence(onPlatform) with a \
         real IdentityKey must fold to can_sync on every leaf"
    );

    // THE RETIRED FORM. This used to be accepted and written, with `parse_identity_key`
    // refusing it only at fold — the wart `group_author`'s missing probe left. `apply`
    // probes, so it is refused at the door and never reaches the log.
    let retired = h.identity_key(szonja);
    assert!(
        retired.starts_with("space1"),
        "Harness::identity_key no longer renders the retired form — if it was fixed, \
         delete this half of the test rather than keeping a claim that is no longer true"
    );
    let mut a = Args::new();
    a.insert("kind".into(), ArgVal::Text("onPlatform".into()));
    a.insert("identityKey".into(), ArgVal::Text(retired));
    let refused = h
        .node(phone)
        .apply(&paola, OP_SET_PRESENCE, a)
        .await
        .expect_err("the retired rendering must not be writable");
    assert!(
        format!("{refused}").contains("MalformedArgs"),
        "refused, but for the wrong reason: {refused}"
    );
    h.settle().await;
    assert_eq!(
        h.group_view(laptop, &paola).presence,
        pacific_core::group::Presence::OnPlatform(key),
        "the space1 delta must be INERT — if presence moved, the reducer started \
         accepting the retired rendering and the ICD's identity note is out of date"
    );
}

/// EVERY MEMBERSHIP TRANSITION IS RECORDED, ON EVERY DEVICE.
///
/// This test used to state the GAP: nothing in `Node` authored `base.memberJoined`,
/// so `MembershipLog::divergence` reported every member as `unrecorded`, on every
/// device, forever — "the day an author path lands the assertion fails and somebody
/// has to come back here and say what it means now."
///
/// What it means now (membership-through-mls.md §10.3): the founder's tenure is
/// written at creation, and each Add door writes the arrival it committed, at the epoch
/// the newcomer can read. So the log and the ratchet tree AGREE on the devices that
/// were there — the laptop included, because the founder's record was written while
/// the group had one leaf and waited in the outbox like any other solo delta.
///
/// AND ONE LIMIT, stated rather than hidden (§17): a member who joins LATER cannot read
/// the records sealed before its own join — forward secrecy — so on szonja's device
/// axel reads as `unrecorded`. The tree still says he is there; the records a late
/// joiner holds simply begin at its join.
#[tokio::test]
async fn every_member_is_recorded_and_the_log_agrees_with_the_tree() {
    let h = Harness::people(&[("axel", &["phone", "laptop"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");
    let szonja = h.device("szonja", "phone");
    h.pair(phone, szonja).await;

    let stoma = h.mint_group(phone);
    h.add_to_forum(phone, laptop, &stoma).await;
    h.add_to_forum(phone, szonja, &stoma).await;
    h.group_set_profile(phone, &stoma, "STOMA", "organisation")
        .await;
    h.settle().await;

    // The ratchet tree knows exactly who is here.
    assert_eq!(h.device_leaves(phone, &stoma), 3);
    let roster = h.roster(phone, &stoma);
    assert_eq!(roster.len(), 2, "two members, three leaves");

    for u in [phone, laptop] {
        let st = h.node(u).group_state(&stoma).unwrap();
        assert_eq!(
            st.membership.current().len(),
            2,
            "{} records both people — axel once, however many devices he has",
            h.device_name(u)
        );
        assert_eq!(
            st.membership.divergence(&roster),
            None,
            "{} — the records and the tree agree",
            h.device_name(u)
        );
    }
    // The late joiner holds its own arrival, and nothing sealed before it.
    let st = h.node(szonja).group_state(&stoma).unwrap();
    assert!(st.membership.is_present(&hex::encode(h.id(szonja))));
    let d = st.membership.divergence(&roster).expect("the late joiner lacks the earlier records");
    assert_eq!(
        d.unrecorded,
        vec![hex::encode(h.id(phone))],
        "axel's records predate szonja's epoch — forward secrecy, not a forgery"
    );
    assert!(d.phantom.is_empty());
}

/// A SOLO OBJECT'S DELTAS WAIT FOR SOMEWHERE TO SEND THEM — which is why a device added
/// later can still read state written before it existed, on a Group but not on a Forum.
///
/// `Node::append_and_flush` returns EARLY when `group_leaf_count <= 1`: with one leaf
/// there is nowhere to publish, so the delta sits in the outbox and is flushed by the
/// next `sync_once` — AT THE THEN-CURRENT EPOCH. So the no-backfill doctrine
/// (`add_member_core`: "NO message backfill") is about deltas that were ALREADY SEALED
/// at an earlier epoch. A group of one has sealed nothing.
///
/// This is worth pinning because it is the difference between "the laptop sees STOMA's
/// name" (it does) and "the laptop sees the conversation" (it does not, and `m21` proves
/// that) — and the two look identical from the outside until you know which side of the
/// leaf-count gate the delta was on.
#[tokio::test]
async fn a_solo_objects_deltas_wait_for_somewhere_to_send_them() {
    let h = Harness::people(&[("axel", &["phone", "laptop"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");

    // Written while STOMA has exactly one leaf.
    let stoma = h.mint_group(phone);
    assert_eq!(h.leaves(phone, &stoma), 1, "a group of one");
    h.group_set_profile(phone, &stoma, "STOMA", "organisation")
        .await;
    h.settle_among(&[phone]).await;

    // The laptop joins afterwards and still folds the name.
    h.add_to_forum(phone, laptop, &stoma).await;
    h.settle().await;
    assert_eq!(
        h.group_view(laptop, &stoma).display_name,
        "STOMA",
        "the laptop missed a delta authored before it was a leaf. If this is now empty, \
         the outbox gate in `append_and_flush` changed and a group-of-one's opening \
         profile no longer reaches anyone — which is the mint path for every kind"
    );
    // It holds the delta itself, not just a projection of it, and the laptop's own
    // arrival record, written by the Add door at the epoch the laptop joined. The
    // founder's tenure is NOT here: since every object carries a pool leaf
    // (resumption.md §5) the group is solo only until the first sync, so the tenure
    // was published at an epoch the laptop never held, and the profile reaches it
    // by the Add's state re-emission instead — the late joiner's case above.
    // Beside them, since O-77, the member's own card (`base.publishProfile`), which the
    // phone's sync publishes into every group it holds: one of each, and one arrival only.
    let log = h.node(laptop).dir.load_log(&hex::decode(&stoma).unwrap()).unwrap();
    let mut ops: Vec<u32> = log.iter().map(|(_, e)| pacific_core::coordinator::decode_delta(e).unwrap().op_id).collect();
    ops.sort();
    let mut want = vec![
        pacific_core::group::OP_SET_PROFILE,
        pacific_core::membership::OP_MEMBER_JOINED,
        pacific_core::profiles::OP_PUBLISH_PROFILE,
    ];
    want.sort();
    assert_eq!(ops, want, "the profile, the laptop's arrival and the member's card");
}
