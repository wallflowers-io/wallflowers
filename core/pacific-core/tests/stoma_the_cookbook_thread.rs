//! The cookbook-zine thread, as a **Forum (19)** — four leaves, two people.
//!
//! Forum is the one kind whose whole op set is `anyMember`/`commutative`: `post` (0),
//! `react` (1), `receipt` (2) and `vote` (3). That makes it the right place to ask the
//! question multidevice actually raises — **is a person one voice or two?** — because
//! every one of those four folds into a map keyed on `op.author`, and `op.author` is the
//! identity key, which both of a person's devices carry.
//!
//! The answer, proved four times below: ONE. A reaction from the laptop replaces the one
//! from the phone, a vote from the laptop replaces the vote from the phone, and one of
//! szonja's devices reading a message marks it read for szonja.
//!
//! THE DISCIPLINE, and it is not tidiness: a device settles before the same person's
//! other device authors. `m21::a_joining_leaf_can_mint_a_message_ref_its_own_person_already_used`
//! is what happens otherwise — `gen` is per-person and derived per-device, so two leaves
//! with different logs mint the same `(author, gen)` for different content.
//!
//! Cast and copy from `app/web/docs/seeds/stoma.js`, thread t1.

mod common;

use common::Harness;

/// Four leaves belonging to two people hold one byte-identical transcript.
///
/// Every device authors, so no leaf is a passenger, and the assertion is on the CONTENT
/// of the fold (`assert_forum_converges` excludes the sender-relative receipt code, which
/// is the one field two devices are never expected to agree on).
#[tokio::test]
async fn the_cookbook_thread_converges_across_four_leaves() {
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

    // Every leaf is in from the start, so no device is reading past its own join.
    let thread = h.form_forum(axel_phone, &[sz_phone]).await;
    h.add_to_forum(axel_phone, axel_laptop, &thread).await;
    h.add_to_forum(axel_phone, sz_studio, &thread).await;
    h.settle().await;

    let everyone = [axel_phone, axel_laptop, sz_phone, sz_studio];
    assert_eq!(h.device_leaves(axel_phone, &thread), 4, "four leaves");
    let mut people = h.roster_people(axel_phone, &thread);
    people.sort();
    assert_eq!(people, vec!["axel", "szonja"], "two people");

    // szonja opens the thread from the studio; each device takes its turn.
    h.obj_post(
        sz_studio,
        &thread,
        "Printer quoted 300 at A4 and 150 at US paper.",
    )
    .await;
    h.settle().await;
    h.obj_post(axel_phone, &thread, "Split it — 200 Spanish, 100 English.")
        .await;
    h.settle().await;
    h.obj_post(sz_phone, &thread, "Plates held.").await;
    h.settle().await;
    h.obj_post(
        axel_laptop,
        &thread,
        "Then 200/100 and we reprint English if it moves.",
    )
    .await;
    h.settle().await;

    // A reply, authored from the laptop against a ref minted on the studio machine.
    let first = h.find_forum(axel_laptop, &thread, "Printer quoted 300 at A4 and 150 at US paper.");
    h.obj_reply(axel_laptop, &thread, "Szonja, hold the plates.", first)
        .await;
    h.settle().await;

    let lines = h.assert_forum_converges(&thread, &everyone);
    let texts: Vec<&str> = lines.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(
        texts,
        vec![
            "Printer quoted 300 at A4 and 150 at US paper.",
            "Split it — 200 Spanish, 100 English.",
            "Plates held.",
            "Then 200/100 and we reprint English if it moves.",
            "Szonja, hold the plates.",
        ],
        "the four leaves do not agree on the thread"
    );

    // AUTHORSHIP IS THE PERSON. Nobody can tell which of axel's devices wrote which
    // line, and nothing above MLS should be able to.
    let axel = h.id(axel_phone);
    let szonja = h.id(sz_phone);
    assert_eq!(h.id(axel_laptop), axel, "one account, one author key");
    assert_eq!(h.id(sz_studio), szonja);
    assert_eq!(
        lines.iter().filter(|m| m.author == axel).count(),
        3,
        "axel's three lines carry one author key"
    );
    assert_eq!(lines.iter().filter(|m| m.author == szonja).count(), 2);
    assert_eq!(
        lines[4].reply_to,
        Some(first),
        "the reply linkage survives being minted on one device and folded on four"
    );

    // One group, one epoch.
    let e = h.epoch(axel_phone, &thread);
    for u in everyone {
        assert_eq!(h.epoch(u, &thread), e, "{} forked", h.device_name(u));
    }
}

/// A REACTION IS PER PERSON, NOT PER LEAF. axel reacts from the phone, then reacts again
/// from the laptop; the fold holds ONE reaction by axel, the later one.
///
/// `forum.react` keys its map on `op.author` and resolves ties by the react's own `gen`
/// (the fold applies the commutative set in `(gen, author, id)` order). Both of axel's
/// devices author under his identity key, so the second react overwrites the first
/// instead of stacking — which is the behaviour a person expects and is NOT what you get
/// if authorship is ever allowed to be per-leaf.
#[tokio::test]
async fn a_reaction_from_the_laptop_replaces_the_one_from_the_phone() {
    let h = Harness::people(&[("axel", &["phone", "laptop"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");
    let szonja = h.device("szonja", "phone");
    h.pair(phone, szonja).await;

    let thread = h.form_forum(phone, &[szonja]).await;
    h.add_to_forum(phone, laptop, &thread).await;
    h.settle().await;

    h.obj_post(szonja, &thread, "Confirmed from Xalapa: 120 at the residency.")
        .await;
    h.settle().await;
    let target = h.find_forum(phone, &thread, "Confirmed from Xalapa: 120 at the residency.");

    h.obj_react(phone, &thread, target, "👀", true).await;
    h.settle().await;
    h.obj_react(laptop, &thread, target, "🔥", true).await;
    h.settle().await;
    // szonja reacts too, so "one entry" cannot pass by the map simply being small.
    h.obj_react(szonja, &thread, target, "🔥", true).await;
    h.settle().await;

    let lines = h.assert_forum_converges(&thread, &[phone, laptop, szonja]);
    let m = lines.iter().find(|m| m.gen == target.1).unwrap();
    let reactors: Vec<[u8; 32]> = m.reactions.iter().flat_map(|(_, who)| who.clone()).collect();
    assert_eq!(
        reactors.len(),
        2,
        "expected two REACTORS (axel, szonja) — axel's phone and laptop must not \
         count twice, or a person with three devices can triple a reaction: {:?}",
        m.reactions
    );
    assert_eq!(
        m.reactions,
        vec![("🔥".to_string(), {
            let mut who = vec![h.id(phone), h.id(szonja)];
            who.sort();
            who
        })],
        "axel's later 🔥 from the laptop must REPLACE his 👀 from the phone, and land \
         in the same bucket as szonja's"
    );

    // And taking it back from the third device works the same way.
    h.obj_react(laptop, &thread, target, "🔥", false).await;
    h.settle().await;
    let lines = h.assert_forum_converges(&thread, &[phone, laptop, szonja]);
    let m = lines.iter().find(|m| m.gen == target.1).unwrap();
    assert_eq!(
        m.reactions,
        vec![("🔥".to_string(), vec![h.id(szonja)])],
        "axel cleared his reaction from the laptop; szonja's stands"
    );
}

/// A VOTE IS ONE PERSON, ONE VOICE. axel votes the print run up from the phone and then
/// down from the laptop; the score is −1, not 0, and he appears exactly once.
#[tokio::test]
async fn the_print_run_vote_is_one_person_one_voice() {
    let h = Harness::people(&[("axel", &["phone", "laptop"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");
    let szonja = h.device("szonja", "phone");
    h.pair(phone, szonja).await;

    let thread = h.form_forum(phone, &[szonja]).await;
    h.add_to_forum(phone, laptop, &thread).await;
    h.settle().await;

    h.obj_post(szonja, &thread, "Proposal: 200 Spanish / 100 English.")
        .await;
    h.settle().await;
    let target = h.find_forum(phone, &thread, "Proposal: 200 Spanish / 100 English.");

    h.with(phone, |n| {
        let t = thread.clone();
        async move { n.object_vote_post(&t, target, 1).await }
    })
    .await
    .unwrap();
    h.settle().await;
    h.with(laptop, |n| {
        let t = thread.clone();
        async move { n.object_vote_post(&t, target, -1).await }
    })
    .await
    .unwrap();
    h.settle().await;

    let lines = h.assert_forum_converges(&thread, &[phone, laptop, szonja]);
    let m = lines.iter().find(|m| m.gen == target.1).unwrap();
    assert!(
        m.up.is_empty(),
        "axel's up-vote from the phone must be REPLACED, not kept alongside: up={:?}",
        m.up
    );
    assert_eq!(
        m.down,
        vec![h.id(phone)],
        "axel votes once, and the later device's vote is the one that stands"
    );

    // Clearing (dir 0) from the third device removes him entirely.
    h.with(phone, |n| {
        let t = thread.clone();
        async move { n.object_vote_post(&t, target, 0).await }
    })
    .await
    .unwrap();
    h.settle().await;
    let lines = h.assert_forum_converges(&thread, &[phone, laptop, szonja]);
    let m = lines.iter().find(|m| m.gen == target.1).unwrap();
    assert!(m.up.is_empty() && m.down.is_empty(), "the vote is withdrawn");
}

/// A READ RECEIPT IS PER PERSON TOO — and that is worth stating plainly, because it is
/// the one place where "a person is one voice" reads as a weaker guarantee rather than a
/// stronger one.
///
/// `forum_detailed` computes the sender's ✓✓ against `dir.group_members` MINUS self —
/// the deduplicated PEOPLE projection, not the ratchet tree — so when szonja reads on
/// her studio machine, axel's message shows as read by szonja even though her phone has
/// never opened it. There is no per-device receipt state anywhere in the fold.
#[tokio::test]
async fn one_of_szonjas_devices_reading_marks_it_read_for_szonja() {
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

    let thread = h.form_forum(axel_phone, &[sz_phone]).await;
    h.add_to_forum(axel_phone, axel_laptop, &thread).await;
    h.add_to_forum(axel_phone, sz_studio, &thread).await;
    h.settle().await;

    h.obj_post(axel_phone, &thread, "Recipe order goes in the channel tonight.")
        .await;
    h.settle().await;

    let sent = h
        .obj_view(axel_phone, &thread)
        .into_iter()
        .find(|m| m.text == "Recipe order goes in the channel tonight.")
        .unwrap();
    assert!(
        sent.receipt < 3,
        "nobody has read it yet, so it cannot already be ✓✓-blue (got {})",
        sent.receipt
    );

    // ONE of szonja's two devices reads it. Her phone never opens the thread.
    let acked = h
        .with(sz_studio, |n| {
            let t = thread.clone();
            async move { n.object_mark_read(&t).await }
        })
        .await
        .unwrap();
    assert_eq!(acked, 1, "the studio receipted axel's one message");
    h.settle().await;

    let sent = h
        .obj_view(axel_phone, &thread)
        .into_iter()
        .find(|m| m.text == "Recipe order goes in the channel tonight.")
        .unwrap();
    assert_eq!(
        sent.receipt, 3,
        "axel's message should read as READ BY EVERY RECIPIENT: his only recipient is \
         the PERSON szonja, and one of her devices receipted it. If this is 1 or 2, \
         receipts have become per-leaf and every multi-device user's ticks will now \
         hang until they open every device they own"
    );

    // And the same message on axel's OTHER device carries no receipt code at all —
    // the code is sender-relative and each device fills it for its own leaf's view.
    let on_laptop = h
        .obj_view(axel_laptop, &thread)
        .into_iter()
        .find(|m| m.text == "Recipe order goes in the channel tonight.")
        .unwrap();
    assert_eq!(
        on_laptop.receipt, 3,
        "axel's laptop is the same PERSON, so it shows his own bubble's ticks too"
    );
}
