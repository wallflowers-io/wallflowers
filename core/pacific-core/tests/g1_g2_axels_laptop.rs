//! G1 AND G2 — the two gates `docs/execution.html` §02 puts on Track 1, written
//! as things that happen to STOMA rather than as assertions about a member count.
//!
//! They ship together and they must go green together, which is why they are one
//! binary. The reason is in the gate text itself:
//!
//!   * G1 — "axel writes in the cookbook thread from his phone at 09:58. That
//!     afternoon he opens his laptop at the residency. The line is there."
//!   * G2 — "STOMA's top band is consent, quorum 7. The electorate is seven. axel
//!     opens a laptop. If the roster counts leaves, the electorate reads 8 — and
//!     no consent decision can ever pass again, because only seven humans exist
//!     to vote."
//!
//! G1 alone is satisfiable by a change that also inflates the roster. G2 alone is
//! satisfiable by doing nothing at all. Together they pin the split A1b is: WHO IS
//! AUTHORISED HERE stays people, deduplicated; DO I NEED THE RELAY becomes leaves.
//!
//! WHAT FAILS WITHOUT WHICH HALF, so a red run is diagnosable:
//!   * without A1a (`mls::PerLeafIdentity`) the laptop cannot become a leaf at
//!     all — every add below fails as `MlsError::DuplicateLeafData`;
//!   * without A1b (`Directory::group_leaf_count`) the laptop becomes a leaf and
//!     `g1_a` still fails, because axel's OWN object has one member and two
//!     leaves, every relay gate in `node.rs` reads "solo", and nothing is sent;
//!   * with A1a but without the dedup, `g2` fails: the electorate reads eight and
//!     the treasury has quietly stopped being able to spend.

mod common;

use common::Harness;
use pacific_core::coordinator::{Ballot, Rule};

/// The line axel writes at 09:58, from the gate text.
const THE_LINE: &str = "Then 200/100 and we reprint English if it moves.";

/// G1, first half — AXEL'S OWN OBJECT, which is the half that is broken today.
///
/// A person's own objects — their identity record, their notes, a draft nobody
/// else is in — have exactly ONE member. Sign in on a second device and they have
/// one member and TWO LEAVES. Every one of the five relay gates in `node.rs` used
/// to ask `dir.group_members(group_id)?.len()`, read "solo", and send nothing: the
/// laptop never saw what the phone wrote in axel's own notes, forever, silently.
///
/// Note what carries the line across. Nothing re-encrypts history: the post was
/// made while the group was genuinely solo, so it never left the OUTBOX, and the
/// first sync after the laptop becomes a leaf flushes it at the CURRENT epoch.
/// That is the existing "a post made on a dead link stays in the outbox and
/// flushes on the next sync" contract doing exactly what it says, once the gate
/// stops lying about whether there is a link.
#[tokio::test]
async fn g1_a_axel_writes_in_his_own_draft_then_opens_his_laptop() {
    let mut h = Harness::people(&[("axel", &["phone"])]).await;
    let phone = h.device("axel", "phone");

    // 09:58, Cádiz. A private draft — axel is the only member.
    let draft = h.form_named_forum(phone, "cookbook draft");
    h.obj_post(phone, &draft, THE_LINE).await;
    assert_eq!(h.leaves(phone, &draft), 1, "one leaf while it is just the phone");

    // That afternoon, the residency. He opens a laptop — restored from his own 24
    // words, so it is the same account and NOT a second person.
    let laptop = h.add_device("axel", "laptop");
    assert_eq!(h.id(laptop), h.id(phone), "one account, two devices");

    // A device joins the way a member joins: the owner adds its bundle.
    h.add_to_forum(phone, laptop, &draft).await;
    h.settle().await;

    // ── THE SPLIT, stated as two numbers that must disagree ───────────────────
    for u in [phone, laptop] {
        assert_eq!(
            h.roster(u, &draft).len(),
            1,
            "{}: the draft has ONE member — axel. This is the number the fold, the \
             quorum bands and `is_member` read, and two leaves must never inflate it",
            h.device_name(u)
        );
        assert_eq!(
            h.device_leaves(u, &draft),
            2,
            "{}: and TWO leaves. This is the number that decides whether the relay \
             is touched. If it reads 1 the laptop is never told anything",
            h.device_name(u)
        );
    }

    // ── THE GATE ──────────────────────────────────────────────────────────────
    let on_the_laptop: Vec<String> = h
        .obj_view(laptop, &draft)
        .into_iter()
        .map(|m| m.text)
        .collect();
    assert!(
        on_the_laptop.iter().any(|t| t == THE_LINE),
        "THE LINE IS NOT THERE. axel wrote it on his phone and his laptop cannot \
         see it — which is the whole of G1. The laptop holds {on_the_laptop:?}"
    );

    // And it goes the other way too: the laptop is a full leaf, not a reader.
    h.obj_post(laptop, &draft, "reprint English, then.").await;
    h.settle().await;
    h.assert_forum_converges(&draft, &[phone, laptop]);
    assert_eq!(
        h.epoch(phone, &draft),
        h.epoch(laptop, &draft),
        "one object, one epoch — axel's two devices must not fork his own notes"
    );
}

/// G1, second half — THE SHARED COOKBOOK THREAD, which is the gate as written.
///
/// A thread with other people in it already had more than one member, so the five
/// gates were never the blocker here: A1a was. The laptop could not be a leaf at
/// all, because both of axel's devices carry the same identity pubkey as their
/// BasicCredential id and `BasicIdentityProvider` made that a duplicate.
///
/// The laptop is added BEFORE the line is written, deliberately. MLS forward
/// secrecy means a leaf joining at epoch N cannot read anything sealed at N-1, and
/// `add_member_core` refuses to re-encrypt history precisely so that keeps being
/// true — so "he opens his laptop and yesterday's thread is all there" is NOT what
/// this system promises, and a test asserting it would be asserting a wish. What
/// it promises is that a device you already hold sees what you write from now on.
#[tokio::test]
async fn g1_b_axel_writes_in_the_cookbook_thread_and_opens_his_laptop() {
    let mut h = Harness::people(&[("axel", &["phone"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let szonja = h.device("szonja", "phone");

    h.pair(phone, szonja).await;
    let cookbook = h.form_forum(phone, &[szonja]).await;

    // axel signs in on the laptop at the residency and it joins the thread.
    let laptop = h.add_device("axel", "laptop");
    h.add_to_forum(phone, laptop, &cookbook).await;
    h.settle().await;

    assert_eq!(
        {
            let mut p = h.roster_people(phone, &cookbook);
            p.sort();
            p
        },
        vec!["axel", "szonja"],
        "TWO people in the cookbook thread, however many devices they hold"
    );
    assert_eq!(h.device_leaves(phone, &cookbook), 3, "three leaves: two of axel's");

    // 09:58. He writes from the phone, and the laptop is SHUT — `settle_among`
    // rather than `settle`, so the laptop genuinely syncs nothing while the line
    // is written. A gate that quietly had the laptop online the whole time would
    // be testing a much easier thing.
    h.obj_post(phone, &cookbook, THE_LINE).await;
    h.settle_among(&[phone, szonja]).await;
    assert!(
        !h.obj_view(laptop, &cookbook)
            .iter()
            .any(|m| m.text == THE_LINE),
        "the laptop was shut; it cannot have the line yet"
    );

    // That afternoon he opens the laptop. The line is there — and szonja, who
    // cannot tell axel's devices apart and should not be able to, sees one line
    // from one person.
    h.settle().await;
    for u in [laptop, szonja] {
        let texts: Vec<String> = h.obj_view(u, &cookbook).into_iter().map(|m| m.text).collect();
        assert!(
            texts.iter().any(|t| t == THE_LINE),
            "{} does not have the line; it holds {texts:?}",
            h.device_name(u)
        );
    }
    let from_axel = h
        .obj_view(szonja, &cookbook)
        .into_iter()
        .filter(|m| m.author == h.id(phone))
        .count();
    assert_eq!(
        from_axel, 1,
        "szonja sees ONE line from axel, not one per device of his"
    );
    assert_eq!(h.epoch(laptop, &cookbook), h.epoch(szonja, &cookbook));
}

/// G2 — THE CONSENT BAND STILL WORKS.
///
/// STOMA's electorate is seven: axel, szonja, emilio, david, kazy, vlada, mateo.
/// The top band is consent with a quorum of seven, for anything above €5,000, and
/// releasing the full €10,000 Fundación Cádiz grant needs all seven.
///
/// axel opens a laptop. If the roster counted LEAVES the electorate would read
/// eight, every large release would fall one vote short forever, and nothing on
/// screen would explain why — a treasury that has quietly stopped being able to
/// spend, noticed six months later. The dedup on `group_members`'s primary key is
/// the only thing standing between here and there, which is why A1b keeps it and
/// puts the leaf count in a scalar beside the rows instead.
///
/// The two numbers are asserted TOGETHER and from every device: eight leaves, and
/// an electorate of seven. Asserting only the second would pass on a build where
/// the laptop never joined at all.
#[tokio::test]
async fn g2_stomas_electorate_of_seven_still_reads_seven() {
    let stoma_members = ["szonja", "emilio", "david", "kazy", "vlada", "mateo"];
    let mut spec: Vec<(&str, &[&str])> = vec![("axel", &["phone"][..])];
    spec.extend(stoma_members.iter().map(|n| (*n, &["phone"][..])));
    let mut h = Harness::people(&spec).await;

    let axel = h.device("axel", "phone");
    let others: Vec<usize> = stoma_members.iter().map(|n| h.device(n, "phone")).collect();
    let stoma = h.form_forum(axel, &others).await;

    // The electorate, before any device multiplies.
    assert_eq!(h.roster(axel, &stoma).len(), 7, "seven members to begin with");
    assert_eq!(h.device_leaves(axel, &stoma), 7, "and seven leaves");

    // ── axel opens a laptop ───────────────────────────────────────────────────
    // Added BEFORE the ballot, so it holds the whole log and its Lamport counter
    // is current — see `m21`'s `a_joining_leaf_can_mint_a_message_ref_...` for
    // what a device that joins mid-vote would do to the tally.
    let laptop = h.add_device("axel", "laptop");
    h.add_to_forum(axel, laptop, &stoma).await;
    h.settle().await;

    let everyone: Vec<usize> = std::iter::once(axel)
        .chain(others.iter().copied())
        .chain(std::iter::once(laptop))
        .collect();
    for &u in &everyone {
        assert_eq!(
            h.device_leaves(u, &stoma),
            8,
            "{}: EIGHT leaves — axel really is signed in twice",
            h.device_name(u)
        );
        assert_eq!(
            h.roster(u, &stoma).len(),
            7,
            "{}: SEVEN in the electorate. This is the number the frozen RATIFY \
             roster and every quorum band divide by; eight here is a treasury that \
             can never spend again",
            h.device_name(u)
        );
        let mut people = h.roster_people(u, &stoma);
        people.sort();
        assert_eq!(
            people,
            vec!["axel", "david", "emilio", "kazy", "mateo", "szonja", "vlada"],
            "{}: and they are the seven humans, by name",
            h.device_name(u)
        );
    }

    // ── the €10,000 release, under the consent band ───────────────────────────
    let pid = h
        .obj_propose(
            axel,
            &stoma,
            "release the full €10,000 Fundación Cádiz grant",
            Rule::Consent,
        )
        .await;
    h.settle().await;

    for &u in &others {
        h.obj_vote(u, &stoma, pid, Ballot::Approve).await;
        h.settle().await;
    }
    // axel casts his from the LAPTOP, not the phone he proposed from. One person,
    // one ballot: the tally must not move because he used the other device.
    h.obj_vote(laptop, &stoma, pid, Ballot::Approve).await;
    h.settle().await;

    h.obj_close(axel, &stoma, pid).await;
    h.settle().await;

    for &u in &everyone {
        let rows = h.obj_ratify(u, &stoma);
        assert_eq!(rows.len(), 1, "{} holds the one proposal", h.device_name(u));
        assert_eq!(
            rows[0].outcome,
            "passed",
            "{}: THE CONSENT DECISION MUST STILL PASS",
            h.device_name(u)
        );
        assert_eq!(
            rows[0].approve,
            7,
            "{}: seven approvals from seven people — axel's second device must not \
             add an eighth ballot, and must not fail to cast his one",
            h.device_name(u)
        );
        assert_eq!(rows[0].reject, 0);

        // The arithmetic a quorum band actually performs, written out: ballots
        // cast against the size of the electorate. Quorum 7 of 7 is reachable.
        let electorate = h.roster(u, &stoma).len();
        assert!(
            rows[0].approve as usize >= electorate,
            "{}: a quorum of {electorate} cannot be met by {} ballots — this is the \
             failure G2 exists to catch, and it is silent in production",
            h.device_name(u),
            rows[0].approve
        );
    }
}
