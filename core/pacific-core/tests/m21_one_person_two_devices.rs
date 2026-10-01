//! ONE PERSON, TWO DEVICES — the account signed in more than once.
//!
//! `m15_identity_restore` pins the REPLACEMENT case: a key comes back on a device
//! that never held it, with no history, because the old phone is gone. This binary
//! is about the other case, which is the one people are actually in every day —
//! the phone is still in their pocket and the laptop is open too.
//!
//! WHAT IT ESTABLISHES, in order:
//!
//!   1. the premise — two devices restored from one mint really are one account:
//!      same identity key, same fingerprint words, independent state
//!   2. the devices are independent — each has its own store and its own MLS
//!      signing key, so this is two devices and not one counted twice
//!   3. THE GATE — what happens when both of them try to be in one group
//!
//! Step 3 is the point of the binary. It does not currently work, and this test is
//! how that stops being a paragraph in a plan and becomes something that runs: the
//! failure is asserted exactly, with the reason named, so the day the credential
//! scheme changes the assertion fails and someone has to come back here and say
//! what it means now.

mod common;

use common::Harness;

/// Two devices of one person are ONE ACCOUNT and TWO DEVICES at the same time, and
/// both halves of that need saying: the identity is shared, everything else is not.
#[tokio::test]
async fn two_devices_of_one_person_share_an_identity_and_nothing_else() {
    let h = Harness::people(&[("ada", &["phone", "laptop"]), ("bo", &["sim"])]).await;

    let phone = h.device("ada", "phone");
    let laptop = h.device("ada", "laptop");
    let sim = h.device("bo", "sim");

    // ── the premise ───────────────────────────────────────────────────────────
    assert_eq!(
        h.id(phone),
        h.id(laptop),
        "ada's two devices must carry the same identity — a 'second device' with a \
         different key is just a second person with the same display name"
    );
    assert_ne!(h.id(phone), h.id(sim), "ada and bo are different people");
    assert_eq!(h.person_of(phone), "ada");
    assert_eq!(h.person_of(laptop), "ada");
    assert_eq!(h.device_name(laptop), "ada/laptop");

    // The public fingerprint is a property of the ACCOUNT, so both devices show the
    // same three words — this is what lets bo verify ada without caring which of
    // ada's devices they are looking at.
    let phone_words = h.with_sync(phone, |n| n.sas_words());
    let laptop_words = h.with_sync(laptop, |n| n.sas_words());
    assert_eq!(phone_words, laptop_words, "one account, one fingerprint");
    assert_eq!(phone_words.split_whitespace().count(), 3);

    // ── and they are genuinely two devices ────────────────────────────────────
    // Separate state dirs, so separate stores. A contact bundle carries a
    // single-use key package minted from THIS device's own MLS material; two
    // devices handing out byte-identical bundles would mean one store, not two.
    let phone_bundle = h.with_sync(phone, |n| n.build_contact_bundle().unwrap());
    let laptop_bundle = h.with_sync(laptop, |n| n.build_contact_bundle().unwrap());
    assert_ne!(
        phone_bundle, laptop_bundle,
        "each device must mint its own key package — identical bundles would mean \
         the harness handed one store to two names"
    );
}

/// THE GATE, and it is now green. Both of ada's devices ARE members of one object.
///
/// This test used to assert the refusal — `MlsError::DuplicateLeafData`, with the
/// reasoning that the fix was a wire change — and it said, in the message on its
/// own `expect_err`, exactly what to do if the add ever started working:
///
/// > If this is now the behaviour, multi-device has moved and this test is the
/// > thing that has to be rewritten rather than deleted: assert the roster still
/// > folds to two PEOPLE, that both of ada's leaves can author, and that neither
/// > forks the epoch.
///
/// That is this test, and those are its three assertions.
///
/// WHAT MOVED. Nothing on the wire. RFC 9420 §7.3 requires `signature_key` and
/// `encryption_key` to be unique among members and NOT the credential — two leaves
/// carrying one person's credential were always legal MLS. mls-rs refused them
/// because `tree_index.rs` keeps a third uniqueness map keyed on whatever the
/// configured `IdentityProvider::identity()` returns, and `BasicIdentityProvider`
/// returns the credential identifier verbatim. `mls::PerLeafIdentity` returns
/// credential ‖ signature_key instead. `cred_id` is untouched — still the person's
/// Ed25519 identity pubkey — so the fold, authorship, authority and `space_id` all
/// read ada as one person, which is the second assertion's whole point.
///
/// `m24_leaf_pool` proves the same thing at the library level with none of
/// Pacific's machinery in the way; this proves it through the real path — a real
/// bundle, a real staged commit through the relay's commit slot, a real Welcome to
/// the laptop's own intro mailbox.
///
/// THE DEVICES TAKE TURNS here, with a settle between each post, and that is now
/// tidiness rather than necessity — corrected 15 Sep 2026, because the previous
/// version of this note said the opposite and pointed at a test that no longer
/// exists.
///
/// Two of one person's devices CAN author without syncing. `post_to_group` drains
/// to head before it allocates (`node.rs`, `drain_to_head`), so a device pulls in
/// whatever its peers have published before choosing its `gen` — measured over two
/// rounds of genuinely un-settled concurrent posts, with no collision and all three
/// devices agreeing.
///
/// That drain is also exactly why the JOINING case was the one that could not be
/// fixed by syncing, and needed `IntroPayload::gen_watermark` instead: a joining
/// leaf may not read the epochs it is draining past. See
/// `a_joining_leaf_never_re_mints_a_ref_its_own_person_already_used` below.
#[tokio::test]
async fn a_second_device_joins_the_group_its_first_device_is_in() {
    let h = Harness::people(&[("ada", &["phone", "laptop"]), ("bo", &["sim"])]).await;

    let phone = h.device("ada", "phone");
    let laptop = h.device("ada", "laptop");
    let sim = h.device("bo", "sim");

    // ada's phone and bo pair and form a forum the normal way. This part works and
    // is asserted, so nothing below can be blamed on a broken set-up.
    h.pair(phone, sim).await;
    let obj = h.form_forum(phone, &[sim]).await;

    // Sorted, because roster order is MLS LEAF order — an artefact of where the
    // tree put each member, not something Pacific promises. Asserting it unsorted
    // pins an mls-rs implementation detail and fails the day a leaf lands elsewhere.
    assert_eq!(
        people_in(&h, phone, &obj),
        vec!["ada", "bo"],
        "two people in the forum"
    );
    assert_eq!(h.device_leaves(phone, &obj), 2, "two leaves before the laptop");

    // ── now ada opens the laptop ──────────────────────────────────────────────
    // The owner adds their own second device, exactly as they would add anyone
    // else. No special path: a device joins "the way members join", so this is the
    // call that has to work — and it is the call that used to fail.
    let laptop_bundle = h.with_sync(laptop, |n| n.build_contact_bundle().unwrap());
    let target = obj.clone();
    h.with(phone, |n| {
        let b = laptop_bundle.clone();
        async move { n.group_add_member(&target, &b).await }
    })
    .await
    .expect(
        "ADDING A SECOND DEVICE FAILED. `mls::PerLeafIdentity` is what makes this \
         legal — if this is a DuplicateLeafData refusal again, the client builder \
         has gone back to BasicIdentityProvider",
    );
    h.settle().await;

    // ── 1. the roster folds to two PEOPLE ─────────────────────────────────────
    // THREE leaves, TWO people, and both numbers have to be asserted: the leaf
    // count alone would pass on a tree that admitted a stranger, and the people
    // count alone would pass on a laptop that never joined.
    for u in [phone, laptop, sim] {
        assert_eq!(
            people_in(&h, u, &obj),
            vec!["ada", "bo"],
            "{} reads the roster as two PEOPLE — ada's two leaves must not fold to \
             two members, or every count in the system (quorum bands, is_member, \
             MembershipLog::divergence) is one too high",
            h.device_name(u)
        );
        assert_eq!(
            h.device_leaves(u, &obj),
            3,
            "{} sees three LEAVES in the ratchet tree",
            h.device_name(u)
        );
    }

    // ── 2. both of ada's leaves author ────────────────────────────────────────
    // Each from its own ratchet, read by the other two. No coordination, no lease
    // and no lock: two leaves means two ratchets, which is the entire reason for
    // holding two.
    h.obj_post(phone, &obj, "from the phone").await;
    h.settle().await;
    h.obj_post(laptop, &obj, "from the laptop").await;
    h.settle().await;
    h.obj_post(sim, &obj, "and bo replies").await;
    h.settle().await;

    let lines = h.assert_forum_converges(&obj, &[phone, laptop, sim]);
    let texts: Vec<&str> = lines.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(
        texts,
        vec!["from the phone", "from the laptop", "and bo replies"],
        "every device holds the whole conversation, both of ada's leaves included"
    );

    // AUTHORSHIP RESOLVES TO THE PERSON, not to the leaf — this is what keeps
    // everything above MLS unchanged. bo's device cannot tell which of ada's
    // devices wrote which line, and should not be able to.
    let ada = h.id(phone);
    assert_eq!(
        h.id(laptop),
        ada,
        "the premise: one account, so one author key"
    );
    assert_eq!(
        lines.iter().filter(|m| m.author == ada).count(),
        2,
        "both of ada's lines carry ONE author key — hers"
    );
    assert_eq!(lines.iter().filter(|m| m.author == h.id(sim)).count(), 1);

    // ── 3. neither forks the epoch ────────────────────────────────────────────
    // One group, one ratchet tree, one epoch number. Two devices sitting on
    // diverging epochs would be two groups wearing one name, and the converged
    // transcript above would be a coincidence of ordering rather than a proof.
    let e = h.epoch(phone, &obj);
    for u in [laptop, sim] {
        assert_eq!(
            h.epoch(u, &obj),
            e,
            "{} is at a different epoch from ada's phone — the group forked",
            h.device_name(u)
        );
    }
}

/// THE BLOCKER THIS FILE PINNED LAST, NOW CLOSED — a joining leaf can no longer
/// mint a message ref its own person already used, and the history that was there
/// before it joined survives.
///
/// WHAT THE BUG WAS. A `MsgRef` is `(author_pk, gen)`. `author_pk` is the PERSON —
/// that is the property multi-device exists to preserve — and `gen` came from
/// `Node::next_lamport`: the maximum `gen` in THIS DEVICE'S copy of the group log,
/// plus one. Two devices of one person therefore agreed on the author half and
/// derived the counter half from logs that need not match.
///
/// And for a JOINING device they never match, unavoidably. A leaf joining an
/// existing group cannot read anything sealed at an earlier epoch — that is MLS
/// forward secrecy working, and `add_member_core` refuses to re-encrypt history
/// precisely so it keeps working. So the joiner's log started EMPTY, its counter
/// started at zero, and its first post took the ref its own person's first post
/// already held.
///
/// THE DAMAGE WAS NOT A DISPLAY COLLISION. The fold keys commutative deltas BY
/// REF, so the second delta did not sit beside the first — it REPLACED it, on
/// every device holding both. Measured 15 Sep 2026, before the fix: of four
/// messages that existed on two devices, one was destroyed on both, the joiner's
/// own posts never arrived at either, and all three devices ended up permanently
/// disagreeing about the transcript. Which message died was decided by
/// `delta_id`, a hash of the text.
///
/// THE FIX. `IntroPayload::gen_watermark` — the adder's high-water mark, sealed
/// into the Welcome. The adder holds the log and the joiner cannot, and the
/// Welcome is the one message that reaches a joiner before it can author
/// anything, so that is where the number travels. The joiner stores it
/// (`Directory::raise_gen_floor`) and `next_lamport` returns `max(local log,
/// floor)`. An optional field with `skip_serializing_if`, the fifth on this
/// payload — no wire break, no change to the shape of a ref.
///
/// This test asserts what the old one's own failure message asked for: "assert
/// that two leaves of one person can no longer mint the same ref". It also
/// asserts the thing the old test did not look at and which is the actual damage
/// — that the EARLIER messages are still there afterwards.
#[tokio::test]
async fn a_joining_leaf_never_re_mints_a_ref_its_own_person_already_used() {
    let h = Harness::people(&[("ada", &["phone", "laptop"]), ("bo", &["sim"])]).await;
    let phone = h.device("ada", "phone");
    let laptop = h.device("ada", "laptop");
    let sim = h.device("bo", "sim");

    h.pair(phone, sim).await;
    let obj = h.form_forum(phone, &[sim]).await;

    // ada writes from the phone while the laptop is not yet a leaf. These are
    // sealed at the pre-add epoch and the laptop will never be able to read them.
    h.obj_post(phone, &obj, "before the laptop existed").await;
    h.obj_post(phone, &obj, "and a second line").await;
    h.obj_post(sim, &obj, "bo was here too").await;
    h.settle().await;

    let before: Vec<(String, u64)> = h
        .obj_view(phone, &obj)
        .into_iter()
        .map(|m| (m.text, m.gen))
        .collect();
    assert_eq!(before.len(), 3, "three messages exist before the laptop does");

    // The laptop joins — a real add, a real Welcome, three leaves.
    h.add_to_forum(phone, laptop, &obj).await;
    assert_eq!(h.device_leaves(laptop, &obj), 3, "the laptop really is a leaf");
    assert!(
        h.obj_view(laptop, &obj).is_empty(),
        "and it holds NO prior history — forward secrecy, working as designed. \
         That empty log is what used to make its counter restart at zero."
    );

    h.obj_post(laptop, &obj, "written on the laptop").await;
    h.settle().await;

    // ── 1 · NO REF IS RE-MINTED ───────────────────────────────────────────────
    let mine = h
        .obj_view(laptop, &obj)
        .into_iter()
        .find(|m| m.text == "written on the laptop")
        .expect("the laptop holds its own post");
    for (text, gen) in &before {
        assert_ne!(
            (mine.author, mine.gen),
            (h.id(phone), *gen),
            "the laptop minted the ref ada's earlier post {text:?} already holds — \
             the gen floor did not reach it. Check that `add_member_core` sets \
             `IntroPayload::gen_watermark` and that `process_intro_blob` stores it."
        );
    }
    assert!(
        mine.gen >= before.iter().map(|(_, g)| *g).max().unwrap(),
        "and it is at or above the high-water mark it was handed, not below it"
    );

    // ── 2 · AND THE EARLIER HISTORY IS STILL THERE ────────────────────────────
    // The assertion the old test was missing. A ref collision is only interesting
    // because of what the fold does with it, and what it did was delete a message
    // from every device that had it.
    for u in [phone, sim] {
        let now: Vec<String> = h.obj_view(u, &obj).into_iter().map(|m| m.text).collect();
        for (text, _) in &before {
            assert!(
                now.contains(text),
                "{} LOST {text:?} when the laptop joined and posted. It holds: {now:?}",
                h.device_name(u)
            );
        }
        assert!(
            now.iter().any(|t| t == "written on the laptop"),
            "{} never received the laptop's post. It holds: {now:?}",
            h.device_name(u)
        );
    }
}

/// The roster of `obj` as people, sorted — see the note at the first call site.
fn people_in(h: &Harness, u: usize, obj: &str) -> Vec<String> {
    let mut v = h.roster_people(u, obj);
    v.sort();
    v
}

// THE RESTORE DOOR IS NOT RE-TESTED HERE, deliberately. `m15_identity_restore`
// owns it, and the first test above already exercises it for real — `Harness::people`
// builds ada's laptop by calling `Node::restore_identity` with the phone's own 24
// words, so "the same words bring the same key back" is a precondition of this
// binary rather than a claim it needs to make separately.
//
// A hand-rolled version here also broke the file's own rule and was removed for it:
// `PACIFIC_STATE_DIR` is process-global and `Harness` serialises on `ENV_LOCK`, so a
// test that sets the env var itself races every other test in the binary. It did,
// and it failed as `Directory("database is locked")` — which is the harness
// discipline catching exactly what it is there to catch.
