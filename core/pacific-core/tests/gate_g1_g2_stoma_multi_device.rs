//! THE ACCEPTANCE SUITE FOR TRACK 1 — G1 and G2, and they go green TOGETHER.
//!
//! Written from `docs/the-build.html` §02 (A1a, A1b) and `docs/execution.html` §02
//! BEFORE the implementation was read. Nothing in this binary names an interface
//! that A1a/A1b introduce: every verb it calls either exists today or is pinned by
//! the harness spec (`add_device`, `settle_among`, `epoch`). That is deliberate —
//! a disagreement about what `group_leaf_count` is CALLED must not be able to stop
//! the two behavioural gates from running. The interface pins live in
//! `gate_a1b_leaf_count_interface.rs`, where a compile error costs only itself.
//!
//! ## The two gates, and why they are in one binary
//!
//! G1 — axel writes in a thread from his phone; that afternoon he opens his laptop
//!      and the line is there.
//!
//! G2 — STOMA's electorate is SEVEN PEOPLE. Its top treasury band is consent,
//!      quorum seven, for anything above €5,000. axel opening a second device must
//!      not make the electorate read eight, because only seven humans can vote and
//!      every large release would then fall one short — permanently, silently.
//!
//! G1 alone is satisfiable by a change that also inflates the roster: point the
//! five `node.rs` gates at a `group_members` that has stopped deduplicating, and
//! axel's laptop gets its line while STOMA's treasury quietly stops being able to
//! spend. So every test here that adds a second device asserts BOTH numbers — the
//! leaf count that moved and the people count that must not have. They are in one
//! binary so that "green" is one word.
//!
//! ## What each test would look like if it were faked
//!
//! Stated per test, in its own docstring, under HOW THIS COULD BE FAKED. A gate
//! whose author cannot say how an implementor would satisfy it without doing the
//! work has not been finished.

mod common;

use common::Harness;
use pacific_core::coordinator::{Ballot, Rule};

/// STOMA's electorate — seven people, and the one who holds two devices is first
/// because he is the one the gates are about.
const ELECTORATE: usize = 7;

/// G1, at the exact point where it breaks today: A PERSON'S OWN OBJECT.
///
/// axel's notebook is a group of ONE MEMBER and (after the laptop joins) TWO
/// LEAVES. Five gates in `node.rs` decide whether a group touches the relay from
/// `dir.group_members(group_id)?.len()`, and that table is
/// `PRIMARY KEY (group_id, member_pk)` — a set of PEOPLE. So the notebook reads
/// "solo: no peers, no commits, never touches the relay", `append_and_flush`
/// returns before it publishes, and the laptop never sees what the phone wrote.
///
/// THE AFTERNOON IS `settle_among(&[laptop])`, and that is the whole design of
/// this test. The phone is in axel's pocket: it does not sync again after the
/// post. So the line can only reach the laptop if it was published AT POST TIME,
/// by `append_and_flush` itself. A fix that only repairs the outbox re-flush in
/// `sync_group` cannot pass this.
///
/// TODAY: fails — the laptop's view is empty.
/// AFTER:  passes.
///
/// HOW THIS COULD BE FAKED. (a) Use `settle()` so the phone syncs too and some
/// other path carries the line — closed by `settle_among(&[laptop])`. (b) Make
/// `group_members` count leaves so `len() > 1` — closed by asserting the roster is
/// still ONE row after the laptop joins, and by `g2_*` in this same binary.
/// (c) Give the laptop the line by re-encrypting history to the joiner — closed
/// because the post is authored AFTER the join, so history never enters it.
#[tokio::test]
async fn g1_axel_writes_on_the_phone_and_his_own_notebook_reaches_the_laptop() {
    let mut h = Harness::people(&[("axel", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.add_device("axel", "laptop");

    // axel's own notebook: nobody else is in it, and nobody else ever will be.
    let notebook = h.form_named_forum(phone, "axel's notebook");
    assert_eq!(
        h.roster(phone, &notebook).len(),
        1,
        "a notebook starts as one person"
    );
    assert_eq!(h.leaves(phone, &notebook), 1, "and as one leaf");

    // He signs in on the laptop and puts it in the notebook — the same act as
    // adding anyone else, which is the point: a device joins the way a member does.
    h.add_to_forum(phone, laptop, &notebook).await;

    // ── the two numbers, and they must move in opposite directions ────────────
    for u in [phone, laptop] {
        assert_eq!(
            h.roster(u, &notebook).len(),
            1,
            "{}: the notebook is still ONE PERSON's. `group_members` is \
             PRIMARY KEY (group_id, member_pk) and must stay a deduplicated set of \
             people — if it now reads 2 this is A1b implemented by breaking the \
             dedup, and every membership count in the system (quorum bands, \
             is_member, MembershipLog::divergence) has just inflated. See g2.",
            h.device_name(u)
        );
        assert_eq!(
            h.device_leaves(u, &notebook),
            2,
            "{}: and it now has TWO LEAVES in the ratchet tree — the number the \
             five relay gates in node.rs have to be reading",
            h.device_name(u)
        );
        assert_eq!(
            h.roster_people(u, &notebook),
            vec!["axel"],
            "{}: one name in the roster, read as people",
            h.device_name(u)
        );
    }

    // ── 09:58, from the phone ─────────────────────────────────────────────────
    let line = "Then 200/100 and we reprint English if it moves.";
    h.obj_post(phone, &notebook, line).await;

    // ── that afternoon, he opens the laptop. ONLY the laptop syncs. ───────────
    h.settle_among(&[laptop]).await;

    let seen: Vec<String> = h
        .obj_view(laptop, &notebook)
        .into_iter()
        .map(|m| m.text)
        .collect();
    assert_eq!(
        seen,
        vec![line.to_string()],
        "G1 FAILED. axel's laptop opened and the line is not there. The phone \
         authored it into a group whose `group_members` row count is 1, so \
         `append_and_flush` took the solo early-return and the delta was never \
         published. This is A1b: the five gates at node.rs must ask how many \
         LEAVES the group has, not how many PEOPLE."
    );

    // One group, one ratchet tree, one epoch — two devices on diverging epochs
    // would be two notebooks wearing one name and the line above a coincidence.
    assert_eq!(
        h.epoch(phone, &notebook),
        h.epoch(laptop, &notebook),
        "the phone and the laptop are at different epochs — the notebook forked"
    );

    // ── and back the other way ────────────────────────────────────────────────
    // The laptop has synced, so its Lamport counter is past the phone's line and
    // this is not the `m21` message-ref collision. Now only the PHONE syncs.
    let reply = "reprint English, and bring the Xalapa list forward";
    h.obj_post(laptop, &notebook, reply).await;
    h.settle_among(&[phone]).await;

    let both: Vec<String> = h
        .obj_view(phone, &notebook)
        .into_iter()
        .map(|m| m.text)
        .collect();
    assert_eq!(
        both,
        vec![line.to_string(), reply.to_string()],
        "the notebook has to flow in BOTH directions — a fix that only publishes \
         from the device that owns the group is half a fix"
    );
    assert_eq!(
        h.roster(phone, &notebook).len(),
        1,
        "and after all of that it is still one person's notebook"
    );
}

/// G1 as `docs/execution.html` tells it: THE COOKBOOK THREAD, which has other
/// people in it.
///
/// This half is A1a rather than A1b — the thread's member count is already 2, so
/// the five gates were never the obstacle here; the obstacle is that mls-rs
/// refused the second leaf at all. Both halves are G1 and both must hold, so both
/// are tested.
///
/// TODAY: fails — `group_add_member` refuses the laptop with
/// `MlsError::DuplicateLeafData`, because `BasicIdentityProvider::identity()`
/// returns the credential verbatim and both of axel's leaves carry his.
/// AFTER: passes.
///
/// HOW THIS COULD BE FAKED. (a) Give the laptop its own credential id — a wire
/// change A1a exists to avoid, and it would make szonja see TWO axels; closed by
/// the roster-as-people assertion and by `gate_a1a_*`, which pins `cred_id`.
/// (b) Sync the phone in the afternoon so some other path delivers the line;
/// closed by `settle_among(&[laptop])`.
#[tokio::test]
async fn g1_the_cookbook_thread_is_there_when_he_opens_the_laptop() {
    let mut h = Harness::people(&[("axel", &["phone"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let szonja = h.device("szonja", "phone");
    let laptop = h.add_device("axel", "laptop");

    h.pair(phone, szonja).await;
    let cookbook = h.form_forum(phone, &[szonja]).await;
    assert_eq!(h.device_leaves(phone, &cookbook), 2, "two leaves before the laptop");

    h.add_to_forum(phone, laptop, &cookbook).await;

    for u in [phone, laptop, szonja] {
        let mut people = h.roster_people(u, &cookbook);
        people.sort();
        assert_eq!(
            people,
            vec!["axel", "szonja"],
            "{}: TWO PEOPLE in the cookbook thread. szonja must not start seeing \
             two axels — authorship, authority and space_id all key on the \
             credential, which A1a leaves alone",
            h.device_name(u)
        );
        assert_eq!(
            h.roster(u, &cookbook).len(),
            2,
            "{}: and the directory holds TWO member rows, not three",
            h.device_name(u)
        );
        assert_eq!(
            h.device_leaves(u, &cookbook),
            3,
            "{}: three leaves — axel's two and szonja's one",
            h.device_name(u)
        );
    }

    let line = "Then 200/100 and we reprint English if it moves.";
    h.obj_post(phone, &cookbook, line).await;

    // The afternoon. The phone stays in his pocket; szonja is asleep.
    h.settle_among(&[laptop]).await;
    let seen: Vec<String> = h
        .obj_view(laptop, &cookbook)
        .into_iter()
        .map(|m| m.text)
        .collect();
    assert_eq!(
        seen,
        vec![line.to_string()],
        "G1 FAILED on the shared thread: the laptop is a leaf and still has no line"
    );

    // Everyone catches up and agrees, and axel is ONE author.
    h.settle().await;
    let lines = h.assert_forum_converges(&cookbook, &[phone, laptop, szonja]);
    assert_eq!(lines.len(), 1);
    assert_eq!(
        lines[0].author,
        h.id(phone),
        "the line carries axel's identity key, whichever device typed it"
    );
    assert_eq!(
        h.id(laptop),
        h.id(phone),
        "the premise — one account, so one author key"
    );
}

/// G2 — THE CONSENT BAND STILL WORKS.
///
/// STOMA's electorate is seven people: axel, szonja, emilio, david, kazy, vlada,
/// mateo. Its top treasury band is consent, quorum SEVEN, for anything above
/// €5,000. Releasing the full €10,000 Fundación Cádiz grant needs all seven.
///
/// axel opens a laptop. The tree gains a leaf. If the roster gains a ROW, the
/// electorate reads eight, the quorum for the top band becomes eight, and only
/// seven humans exist to vote — so every large release falls one short, forever,
/// with nothing on screen explaining why. That is the failure this test exists to
/// make loud on the day it is introduced rather than six months later.
///
/// THE BAND IS COMPUTED FROM THE SYSTEM'S OWN ELECTORATE, not from a literal 7 —
/// `quorum = h.roster(..).len()`, because STOMA's top band is unanimity of the
/// electorate. That is what couples the assertion to the regression: inflate the
/// roster and the quorum inflates with it, and the release fails arithmetic.
/// The literal seven is asserted separately, as the count of humans.
///
/// TODAY: fails — `group_add_member` refuses axel's laptop (A1a), so the test
/// never reaches the band.
/// AFTER: passes.
///
/// HOW THIS COULD BE FAKED. (a) Drop the `PRIMARY KEY` dedup so the leaf count
/// falls out of `group_members().len()` for free — this test's whole point; the
/// electorate would read 8 and the band would fall one short. (b) Let axel's
/// laptop cast a second ballot to make the numbers add up — closed below: the
/// laptop votes, and `approve` must still be 7. (c) Special-case the tally to
/// ignore a second device — closed because the assertion is on the ROSTER, which
/// is the authorisation set every other surface reads, not on the tally.
#[tokio::test]
async fn g2_stomas_consent_band_still_needs_seven_and_still_gets_them() {
    let mut h = Harness::people(&[
        ("axel", &["phone"]),
        ("szonja", &["phone"]),
        ("emilio", &["phone"]),
        ("david", &["phone"]),
        ("kazy", &["phone"]),
        ("vlada", &["phone"]),
        ("mateo", &["phone"]),
    ])
    .await;

    let phone = h.device("axel", "phone");
    let others: Vec<usize> = ["szonja", "emilio", "david", "kazy", "vlada", "mateo"]
        .iter()
        .map(|n| h.device(n, "phone"))
        .collect();

    let treasury = h.form_forum(phone, &others).await;
    assert_eq!(
        h.roster(phone, &treasury).len(),
        ELECTORATE,
        "STOMA's electorate is seven"
    );
    assert_eq!(h.device_leaves(phone, &treasury), ELECTORATE, "and seven leaves");

    // ── axel opens a laptop ───────────────────────────────────────────────────
    let laptop = h.add_device("axel", "laptop");
    h.add_to_forum(phone, laptop, &treasury).await;

    let mut everyone: Vec<usize> = vec![phone, laptop];
    everyone.extend(others.iter().copied());

    for &u in &everyone {
        let roster = h.roster(u, &treasury);
        assert_eq!(
            roster.len(),
            ELECTORATE,
            "{}: THE ELECTORATE READS {} AND STOMA HAS SEVEN MEMBERS. axel's \
             laptop is a leaf, not a member. If this is 8, the top treasury band \
             (consent, quorum 7, above €5,000) can never pass again — seven humans \
             cannot make eight approvals — and nothing on screen will say why.",
            h.device_name(u),
            roster.len()
        );
        assert_eq!(
            h.device_leaves(u, &treasury),
            ELECTORATE + 1,
            "{}: eight LEAVES though — the tree really did gain axel's laptop",
            h.device_name(u)
        );

        // The rows are a SET: no identity twice, and axel exactly once.
        let mut sorted = roster.clone();
        sorted.sort();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            before,
            "{}: the roster holds a duplicate identity — `group_members` has \
             stopped being a set of people",
            h.device_name(u)
        );
        assert_eq!(
            roster.iter().filter(|pk| **pk == h.id(phone)).count(),
            1,
            "{}: axel appears exactly ONCE in the electorate",
            h.device_name(u)
        );

        let mut people = h.roster_people(u, &treasury);
        people.sort();
        assert_eq!(
            people,
            vec!["axel", "david", "emilio", "kazy", "mateo", "szonja", "vlada"],
            "{}: the seven of STOMA, by name",
            h.device_name(u)
        );
    }

    // ── the €10,000 release, under the top band ───────────────────────────────
    let release = "Release the full €10,000 Fundación Cádiz grant";
    let pid = h.obj_propose(phone, &treasury, release, Rule::Consent).await;
    h.settle().await;

    for &u in &others {
        h.obj_vote(u, &treasury, pid, Ballot::Approve).await;
        h.settle().await;
    }

    // axel votes from the LAPTOP. One person, one vote — the ballot is keyed on
    // the identity key, so this must OVERWRITE his implicit proposer approval and
    // not add a second. An eighth approval appearing here would paper over an
    // inflated electorate, which is the most likely way this gate gets faked.
    h.obj_vote(laptop, &treasury, pid, Ballot::Approve).await;
    h.settle().await;

    h.obj_close(phone, &treasury, pid).await;
    h.settle().await;

    // THE TALLY IS READ ON THE DEVICES THAT HOLD THE WHOLE LOG — axel's phone and
    // the six others. The laptop is deliberately NOT in this loop: it joined the
    // treasury mid-life, so its log begins at its join epoch (forward secrecy,
    // working as designed — `m21`'s third test documents the same empty-log
    // consequence). Its part in this gate is that it VOTED, above, and that its
    // ballot did not become an eighth approval. Its roster and leaf numbers are
    // asserted in the loop over `everyone` further up.
    let mut full_log: Vec<usize> = vec![phone];
    full_log.extend(others.iter().copied());
    for &u in &full_log {
        let rows = h.obj_ratify(u, &treasury);
        let row = rows
            .iter()
            .find(|r| r.payload == release)
            .unwrap_or_else(|| panic!("{} does not hold the release proposal", h.device_name(u)));

        let electorate = h.roster(u, &treasury).len();
        // STOMA's top band: consent, and the quorum IS the electorate.
        let quorum = electorate;
        let approvals = row.approve as usize;

        assert_eq!(
            electorate, ELECTORATE,
            "{}: the electorate must be seven for the band to be satisfiable",
            h.device_name(u)
        );
        assert_eq!(
            approvals,
            ELECTORATE,
            "{}: seven approvals — six ballots plus axel's. It is {} instead. If \
             it is 8, axel's laptop got him a second vote; if it is fewer, one of \
             the seven did not reach the tally.",
            h.device_name(u),
            approvals
        );
        assert_eq!(row.reject, 0, "{}: nobody vetoed", h.device_name(u));
        assert!(
            approvals >= quorum,
            "{}: THE €10,000 RELEASE FALLS {} SHORT of quorum {quorum}. The \
             electorate reads {electorate} and only seven humans can vote, so no \
             consent decision will ever pass again — this is G2, and it is the \
             failure that gets noticed six months later in a treasury that has \
             quietly stopped being able to spend.",
            h.device_name(u),
            quorum.saturating_sub(approvals)
        );
        assert_eq!(
            row.outcome,
            "passed",
            "{}: the release is ratified",
            h.device_name(u)
        );
    }
}

/// G1 AND G2, IN ONE RUN — because `docs/execution.html` says they must go green
/// together and a suite that never puts them in the same scenario has not said so.
///
/// One STOMA afternoon: the treasury passes its band with axel holding two
/// devices, and in the same harness, at the same time, axel's own notebook
/// reaches his laptop. The first needs the roster to count PEOPLE; the second
/// needs the relay gates to count LEAVES. A change that satisfies either one by
/// conflating them fails here.
///
/// TODAY: fails — A1a refuses the laptop before either half is reached.
/// AFTER: passes.
///
/// HOW THIS COULD BE FAKED. It is the conjunction, so the only way to fake it is
/// to fake both halves at once; the individual tests above close each of those.
/// The one thing it adds is that they must hold SIMULTANEOUSLY, in one directory,
/// for one person — which is what rules out "leaf count for solo groups, member
/// count for shared ones" as a shortcut.
#[tokio::test]
async fn g1_and_g2_are_green_together_in_one_stoma_afternoon() {
    let mut h = Harness::people(&[
        ("axel", &["phone"]),
        ("szonja", &["phone"]),
        ("emilio", &["phone"]),
        ("david", &["phone"]),
        ("kazy", &["phone"]),
        ("vlada", &["phone"]),
        ("mateo", &["phone"]),
    ])
    .await;

    let phone = h.device("axel", "phone");
    let others: Vec<usize> = ["szonja", "emilio", "david", "kazy", "vlada", "mateo"]
        .iter()
        .map(|n| h.device(n, "phone"))
        .collect();

    let treasury = h.form_forum(phone, &others).await;
    let notebook = h.form_named_forum(phone, "axel's notebook");

    let laptop = h.add_device("axel", "laptop");
    h.add_to_forum(phone, laptop, &treasury).await;
    h.add_to_forum(phone, laptop, &notebook).await;

    // ── G2's number, on both objects at once ──────────────────────────────────
    assert_eq!(
        h.roster(phone, &treasury).len(),
        ELECTORATE,
        "the electorate is seven people even though the tree has eight leaves"
    );
    assert_eq!(h.device_leaves(phone, &treasury), ELECTORATE + 1);
    assert_eq!(
        h.roster(phone, &notebook).len(),
        1,
        "and the notebook is one person even though it has two leaves"
    );
    assert_eq!(h.device_leaves(phone, &notebook), 2);

    // ── G1's line, written from the phone, read on the laptop ─────────────────
    let line = "Then 200/100 and we reprint English if it moves.";
    h.obj_post(phone, &notebook, line).await;
    h.settle_among(&[laptop]).await;
    assert_eq!(
        h.obj_view(laptop, &notebook)
            .into_iter()
            .map(|m| m.text)
            .collect::<Vec<_>>(),
        vec![line.to_string()],
        "G1: the notebook line did not reach the laptop"
    );

    // ── G2's decision, in the same afternoon ──────────────────────────────────
    let release = "Release the full €10,000 Fundación Cádiz grant";
    let pid = h.obj_propose(phone, &treasury, release, Rule::Consent).await;
    h.settle().await;
    for &u in &others {
        h.obj_vote(u, &treasury, pid, Ballot::Approve).await;
        h.settle().await;
    }
    h.obj_close(phone, &treasury, pid).await;
    h.settle().await;

    let rows = h.obj_ratify(phone, &treasury);
    let row = rows
        .iter()
        .find(|r| r.payload == release)
        .expect("the release proposal");
    let quorum = h.roster(phone, &treasury).len();
    assert_eq!(quorum, ELECTORATE, "G2: the electorate is seven");
    assert!(
        row.approve as usize >= quorum,
        "G2: the release falls {} short — G1 was bought by inflating the roster",
        quorum.saturating_sub(row.approve as usize)
    );
    assert_eq!(row.outcome, "passed", "G2: the release is ratified");

    // And the notebook is STILL one person's after a treasury decision went
    // through the same directory.
    assert_eq!(h.roster(phone, &notebook).len(), 1);
    assert_eq!(h.device_leaves(phone, &notebook), 2);
}
