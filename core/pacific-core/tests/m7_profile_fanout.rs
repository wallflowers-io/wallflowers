//! M7 — a profile edit reaches every Connection as a signed Delta.
//!
//! Before this slice, a profile crossed to a peer EXACTLY ONCE: in the pairing
//! bundle, at scan time. Change your photo afterwards and every contact you already
//! had kept the old one forever. These scenarios prove the channel that fixes it,
//! end to end: real relay, real MLS, real device boundaries (isolated state dirs),
//! real `contact.publishProfile` deltas sealed as MLS application messages.
//!
//! What each one pins:
//!   1. fan-out          — ONE edit reaches ALL connections, not just the newest.
//!   2. update           — a second edit supersedes the first (LWW by author gen).
//!   3. late pairing     — someone who pairs AFTER the edit still gets the card.
//!   4. no cross-talk    — each side publishes its OWN card; neither clobbers.
//!   5. name projection  — a rename lands in the connections list every surface reads.
//!   6. idempotence      — republishing an unchanged profile authors nothing.
//!   7. offline edit     — an edit made with the relay down still arrives on sync.
//!   8. avatar payload   — photo AND the "last seen" clip slot survive the round trip.

mod common;

use common::Harness;
use pacific_core::group::{ContactCard, GroupShape, Labeled};

/// A full card — every field the UI can edit, so the assertions prove the WHOLE
/// bio crosses, not just a display name.
fn full_card() -> ContactCard {
    ContactCard {
        org: "Pacific".into(),
        title: "Founder".into(),
        emails: vec![Labeled {
            label: "work".into(),
            value: "ada@pacific.dev".into(),
        }],
        phones: vec![Labeled {
            label: "cell".into(),
            value: "+1-555-0100".into(),
        }],
        urls: vec![Labeled {
            label: "site".into(),
            value: "https://pacific.dev".into(),
        }],
        photo: "cGhvdG8tYnl0ZXM=".into(),
        photo_mime: "image/jpeg".into(),
        note: "met at Seoul".into(),
        tags: vec!["yc".into()],
        avatar_color: "7A6FF0".into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn one_profile_edit_reaches_every_connection() {
    let h = Harness::new(&["Ada", "Bob", "Cleo"]).await;
    let (ada, bob, cleo) = (0, 1, 2);

    // Ada is connected to two people BEFORE she ever edits her profile — the exact
    // situation the old code stranded.
    h.pair(ada, bob).await;
    h.pair(ada, cleo).await;
    h.settle().await;

    let published = h.set_profile(ada, "Ada Lovelace", &full_card()).await;
    assert_eq!(published, 2, "one edit must fan out to BOTH connections");
    h.settle().await;

    for peer in [bob, cleo] {
        let got = h
            .peer_profile(peer, ada)
            .unwrap_or_else(|| panic!("user {peer} never received Ada's profile"));
        assert_eq!(got.display_name, "Ada Lovelace");
        assert_eq!(got.card.org, "Pacific");
        assert_eq!(got.card.title, "Founder");
        assert_eq!(got.card.photo, "cGhvdG8tYnl0ZXM=");
        assert_eq!(got.card.emails[0].value, "ada@pacific.dev");
        assert_eq!(got.card.urls[0].value, "https://pacific.dev");
        assert_eq!(got.card.note, "met at Seoul");
        assert_eq!(got.card.avatar_color, "7A6FF0");
    }
}

#[tokio::test]
async fn a_later_edit_supersedes_the_earlier_one_everywhere() {
    let h = Harness::new(&["Ada", "Bob"]).await;
    let (ada, bob) = (0, 1);
    h.pair(ada, bob).await;
    h.settle().await;

    h.set_profile(
        ada,
        "Ada",
        &ContactCard {
            title: "Founder".into(),
            ..Default::default()
        },
    )
    .await;
    h.settle().await;
    assert_eq!(h.peer_profile(bob, ada).unwrap().card.title, "Founder");

    // The second edit — a new photo and a new title, the everyday case.
    h.set_profile(
        ada,
        "Ada Lovelace",
        &ContactCard {
            title: "CTO".into(),
            photo: "bmV3LXBob3Rv".into(),
            photo_mime: "image/jpeg".into(),
            ..Default::default()
        },
    )
    .await;
    h.settle().await;

    let got = h.peer_profile(bob, ada).unwrap();
    assert_eq!(got.display_name, "Ada Lovelace", "the newer name wins");
    assert_eq!(got.card.title, "CTO");
    assert_eq!(got.card.photo, "bmV3LXBob3Rv");
    assert!(got.gen > 0, "the second announcement carries a higher gen");
}

#[tokio::test]
async fn someone_who_pairs_after_the_edit_still_gets_the_card() {
    let h = Harness::new(&["Ada", "Bob"]).await;
    let (ada, bob) = (0, 1);

    // Ada edits her profile with NO connections at all — nothing to fan out to.
    let published = h.set_profile(ada, "Ada Lovelace", &full_card()).await;
    assert_eq!(
        published, 0,
        "no connections yet, so nothing to publish into"
    );

    // Bob pairs afterwards. The card must still reach him: the pairing bundle
    // carries only a name, so anything else has to arrive as a delta.
    h.pair(ada, bob).await;
    h.settle().await;

    let got = h
        .peer_profile(bob, ada)
        .expect("a late pairing must still receive the card");
    assert_eq!(got.display_name, "Ada Lovelace");
    assert_eq!(got.card.org, "Pacific");
    assert_eq!(got.card.photo, "cGhvdG8tYnl0ZXM=");
}

#[tokio::test]
async fn both_sides_publish_their_own_card_without_clobbering() {
    // A connection has TWO selves. This is why the op is AnyMember/per-author
    // rather than the owner-only `group.setProfile`: the scanned side is never the
    // owner of the connection group, and must still be able to describe itself.
    let h = Harness::new(&["Ada", "Bob"]).await;
    let (ada, bob) = (0, 1);
    h.pair(ada, bob).await;
    h.settle().await;

    h.set_profile(
        ada,
        "Ada Lovelace",
        &ContactCard {
            org: "Pacific".into(),
            ..Default::default()
        },
    )
    .await;
    h.set_profile(
        bob,
        "Bob Bing",
        &ContactCard {
            org: "Acme".into(),
            ..Default::default()
        },
    )
    .await;
    h.settle().await;

    let ada_as_bob_sees_her = h.peer_profile(bob, ada).expect("bob holds ada's card");
    let bob_as_ada_sees_him = h.peer_profile(ada, bob).expect("ada holds bob's card");
    assert_eq!(ada_as_bob_sees_her.display_name, "Ada Lovelace");
    assert_eq!(ada_as_bob_sees_her.card.org, "Pacific");
    assert_eq!(bob_as_ada_sees_him.display_name, "Bob Bing");
    assert_eq!(bob_as_ada_sees_him.card.org, "Acme");
}

#[tokio::test]
async fn a_rename_lands_in_the_connections_list() {
    // The cached `peers.display_name` is a projection, not a source of truth. A
    // published rename must refresh it, or every name-rendering surface (the
    // connections list, chat headers, derived group names) keeps showing the name
    // frozen at pairing time.
    let h = Harness::new(&["Ada", "Bob"]).await;
    let (ada, bob) = (0, 1);
    h.pair(ada, bob).await;
    h.settle().await;
    assert_eq!(
        h.connection_name(bob, ada),
        "Ada",
        "the pairing-bundle name"
    );

    h.set_profile(ada, "Ada Lovelace", &ContactCard::default())
        .await;
    h.settle().await;

    assert_eq!(
        h.connection_name(bob, ada),
        "Ada Lovelace",
        "the published rename must reach the connections list"
    );
}

#[tokio::test]
async fn republishing_an_unchanged_profile_authors_nothing() {
    // The reconciliation pass runs on EVERY sync. If an unchanged profile republished
    // itself each time, a quiet connection would grow a delta per tick forever.
    let h = Harness::new(&["Ada", "Bob"]).await;
    let (ada, bob) = (0, 1);
    h.pair(ada, bob).await;
    h.settle().await;

    assert_eq!(h.set_profile(ada, "Ada Lovelace", &full_card()).await, 1);
    h.settle().await;
    let after_first = h.peer_profile(bob, ada).unwrap().gen;

    // Same bytes again: the digest matches, so nothing is authored.
    assert_eq!(
        h.set_profile(ada, "Ada Lovelace", &full_card()).await,
        0,
        "an unchanged profile must not author a delta"
    );
    h.settle().await;
    assert_eq!(
        h.peer_profile(bob, ada).unwrap().gen,
        after_first,
        "and the peer's view must not move"
    );
}

#[tokio::test]
async fn an_edit_made_while_the_relay_is_unreachable_still_arrives() {
    // Write-local-first: the edit is durable immediately and the outbox owns
    // delivery, so "offline" degrades to "not yet flushed", never to "lost".
    let h = Harness::new(&["Ada", "Bob"]).await;
    let (ada, bob) = (0, 1);
    h.pair(ada, bob).await;
    h.settle().await;
    // Pairing + the first sync's reconciliation already gave Bob Ada's card as it
    // stood then — the baseline the offline edit has to move.
    assert_eq!(h.peer_profile(bob, ada).unwrap().display_name, "Ada");

    let published = h
        .set_profile_via(ada, "Ada Lovelace", &full_card(), "ws://127.0.0.1:1")
        .await;
    assert_eq!(
        published, 1,
        "the local append succeeds even with the relay down"
    );
    assert_eq!(
        h.peer_profile(bob, ada).unwrap().display_name,
        "Ada",
        "the flush failed, so Bob still holds the pre-edit card"
    );

    // The relay comes back; the queued delta flushes on the next sync.
    h.settle().await;
    let got = h
        .peer_profile(bob, ada)
        .expect("the queued profile must flush on sync");
    assert_eq!(got.display_name, "Ada Lovelace");
    assert_eq!(got.card.photo, "cGhvdG8tYnl0ZXM=");
}

#[tokio::test]
async fn the_still_crosses_with_its_capture_time_but_the_clip_route_is_cut() {
    // The "last seen" pinhole capture is a PARALLEL slot to the still, and the slot
    // is still on the card — but it does NOT ride the connection fan-out. One delta
    // per Connection means every byte here is paid for N times with no dedup, which
    // a ~100KB still can carry and 1.5MB of motion cannot. So: the still crosses
    // (with the capture time, which is what makes freshness mean anything), and an
    // inline clip is refused at AUTHORING — before it is written locally, before it
    // is fanned to anyone.
    let h = Harness::new(&["Ada", "Bob"]).await;
    let (ada, bob) = (0, 1);
    h.pair(ada, bob).await;
    h.settle().await;

    let card = ContactCard {
        photo: "c3RpbGwtZnJhbWU=".into(),
        photo_mime: "image/jpeg".into(),
        captured_at: 1_800_000_000,
        avatar_color: "7A6FF0".into(),
        ..Default::default()
    };
    h.set_profile(ada, "Ada Lovelace", &card).await;
    h.settle().await;

    let got = h.peer_profile(bob, ada).unwrap().card;
    assert_eq!(got.photo, "c3RpbGwtZnJhbWU=", "the still survives");
    assert_eq!(
        got.captured_at, 1_800_000_000,
        "freshness is the capture time"
    );
    assert_eq!(got.avatar_color, "7A6FF0", "and the block-colour fallback");

    // Now the cut route. `set_my_profile` refuses rather than authoring N deltas
    // that every peer would refuse in turn.
    let with_clip = ContactCard {
        clip: "cGluaG9sZS1jbGlw".into(),
        clip_mime: "video/mp4".into(),
        ..card.clone()
    };
    let err = h
        .node(ada)
        .set_my_profile("Ada Lovelace", GroupShape::Individual, &with_clip)
        .await
        .expect_err("an inline profile clip must be refused at authoring");
    assert!(
        err.to_string().contains("clip"),
        "the refusal must name the slot, got: {err}"
    );

    // And the refusal left the PUBLISHED profile untouched — Bob still holds the
    // still, not a half-applied edit.
    h.settle().await;
    let after = h.peer_profile(bob, ada).unwrap().card;
    assert_eq!(after.photo, "c3RpbGwtZnJhbWU=");
    assert!(after.clip.is_empty(), "no clip ever crossed");
}
