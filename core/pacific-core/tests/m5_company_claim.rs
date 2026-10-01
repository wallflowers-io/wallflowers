//! Company-claim scenario, part 1 — chat about a company, then mint its
//! GroupObject as an off-platform directory entry. Over the real relay, real
//! Nodes, real Deltas.
//!
//! The full scenario is: two users chat about a company → the company is
//! identified as a stable entity and a GroupObject is minted → the mention
//! becomes long-pressable → one option is "invite the company to represent
//! itself in Pacific" (the off→on claim).
//!
//! THIS FILE builds the DETERMINISTIC foundation (scenario steps 1–3) that needs
//! no new protocol: the chat converges and Alice mints a solo, off-platform
//! `organisation` Group she owns. It deliberately STOPS before the claim.
//!
//! Steps 4–9 (a typed chat→Group reference to re-home, the out-of-band invite +
//! claim proof, the company minting its own canonical identity, and retiring the
//! stub) are GATED on two open design decisions — the claim topology and the
//! claim-proof crypto — because guessing either would bake in a partial migration
//! (a dual source of truth) the invariants forbid. When those land, this file
//! grows the `company_claims_and_represents_itself` test below its `#[ignore]`.

mod common;

use common::Harness;
use pacific_core::group::{GroupShape, Presence};

/// Steps 1–3: Alice & Bob pair and chat about "Acme"; the chat converges
/// byte-for-byte; then Alice mints Acme as a solo, off-platform `organisation`
/// Group she owns — the exact `object_new → setProfile → setPresence(offPlatform)`
/// sequence the vCard importer uses, driven straight from the directory.
#[tokio::test]
async fn chat_then_mint_offplatform_company_group() {
    let h = Harness::new(&["Alice", "Bob"]).await;
    let (alice, bob) = (0, 1);

    // 1–2. Alice & Bob pair and talk about the company in a shared forum.
    h.pair(alice, bob).await;
    let room = h.form_forum(alice, &[bob]).await;
    h.obj_post(
        alice,
        &room,
        "we keep hearing about Acme — solar racking, YC S26",
    )
    .await;
    h.settle().await;
    h.obj_post(bob, &room, "yeah, Acme is worth tracking as a real entity")
        .await;
    h.settle().await;

    // The chat is real and converges on both devices.
    let chat = h.assert_forum_converges(&room, &[alice, bob]);
    assert_eq!(chat.len(), 2, "two forum posts");
    assert!(chat.iter().all(|m| m.text.contains("Acme")));

    // 3. Alice identifies "Acme" as a stable entity and mints its directory entry:
    //    a solo group-of-1 she owns, off-platform (no space yet — reached out of
    //    band at founders@acme.com).
    let stub = h.mint_group(alice);
    h.group_set_profile(alice, &stub, "Acme", "organisation")
        .await;
    h.group_presence_off(alice, &stub, "founders@acme.com")
        .await;

    // The folded identity is exactly what the reducer promises.
    let v = h.group_view(alice, &stub);
    assert_eq!(v.display_name, "Acme");
    assert_eq!(v.shape, GroupShape::Organisation);
    assert_eq!(
        v.presence,
        Presence::OffPlatform(Some("founders@acme.com".into()))
    );
    assert!(
        !v.can_sync,
        "an off-platform contact is not reachable over MLS"
    );
    assert!(v.roles.is_empty(), "no roles on a group-of-1");

    // Membership is the MLS ratchet tree (never the log): the stub holds ONLY
    // Alice, and it is HER private entry — Bob is not a member and cannot fold it.
    let roster = h.roster(alice, &stub);
    assert_eq!(roster, vec![h.id(alice)], "a solo group owned by Alice");
    // Bob has no such object at all — the stub never replicated to him. Assert both
    // the empty roster and that folding errors for the RIGHT reason (no owner row),
    // so this can't pass because of some unrelated failure.
    assert!(
        h.roster(bob, &stub).is_empty(),
        "Bob has no members for Alice's private stub"
    );
    let bob_err = h
        .with_sync(bob, |n| n.group_view(&stub))
        .unwrap_err()
        .to_string();
    assert!(
        bob_err.contains("has no owner"),
        "Bob can't fold Alice's private stub (no owner row); got: {bob_err}"
    );
}

/// Step 6 building block — the company installs Pacific AFTER the conversation
/// started: a fresh, isolated identity introduced mid-scenario, which pairs and
/// DMs like any other user. This de-risks the eventual claim (the company is a
/// REAL second identity, not a pre-seeded fake) without touching claim semantics.
#[tokio::test]
async fn company_joins_as_a_real_mid_scenario_identity() {
    let mut h = Harness::new(&["Alice", "Bob"]).await;
    let (alice, _bob) = (0, 1);

    // The company shows up later and installs Pacific.
    let company = h.add_user("Acme");
    assert_eq!(company, 2);

    // It's a genuine, pair-able identity: Alice and the company connect and DM.
    h.pair(alice, company).await;
    h.dm_post(alice, company, "welcome to Pacific — claim your page?")
        .await;
    h.settle().await;
    let dm = h.assert_dm_converges(alice, company);
    assert_eq!(dm.len(), 1);
    assert_ne!(h.id(company), h.id(alice), "a distinct, real identity");
}

/// Steps 5–9 (the off→on CLAIM). GATED on two decisions before any crypto is
/// written — see the module header. Kept as an explicit, running-but-ignored
/// placeholder so the sequencing is visible in the test binary, not a TODO.
#[tokio::test]
#[ignore = "blocked on claim-topology + claim-proof design decisions (see module header)"]
async fn company_claims_and_represents_itself() {
    // When DD1 (canonical-merge + re-home) and DD2 (claim proof) are signed off:
    //   - Alice issues the invite carrying the proof that names `stub`.
    //   - the company mints its OWN canonical org group (owner=company, genuinely
    //     OnPlatform at its own space), and presents the proof.
    //   - on a VALID proof: Alice's stub is retired and every core ref re-homes
    //     stub → canonical in the same change; the surviving group is OnPlatform.
    //   - on a FORGED proof: the claim fails loudly with zero state change.
    // Fails loud if ever run before it's implemented (so an --ignored CI sweep
    // can't report the claim scenario as "covered").
    unimplemented!("blocked on claim-topology + claim-proof decisions — see module header");
}
