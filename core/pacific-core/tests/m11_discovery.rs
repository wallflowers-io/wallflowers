//! M11 — hop-bounded discovery, and publishing a note at a Place.
//!
//! The scenario the Yellow Circle runs: ask the graph for nearby Places
//! (`discover` → sync ticks → `discover_results`), then publish a note to one —
//! held, adopted-from-discovery, or freshly minted. Three real devices, a real
//! relay, nothing mocked: a passing test is a proof the mesh converges.

mod common;

use common::Harness;
use pacific_core::place::Access;
use pacific_core::visibility::Visibility;

/// Mint a place with a live fix and set its VISIBILITY dial — the thing that
/// decides whether it answers discovery. Access (joinability) is a separate dial
/// the one test that needs it sets itself.
async fn mint_place(
    h: &Harness,
    u: usize,
    name: &str,
    vis: Visibility,
    lat: f64,
    lng: f64,
) -> String {
    h.with(u, |n| async move {
        let point = pacific_core::geo::GeoPoint::from_degrees(lat, lng);
        let id = n
            .place_mint(
                name,
                "",
                Some(pacific_core::geo::LocationSource::Fixed { point }),
            )
            .await
            .unwrap();
        if vis != Visibility::Private {
            n.place_set_visibility(&id, vis).await.unwrap();
        }
        id
    })
    .await
}

fn discovered_names(h: &Harness, u: usize, hops: u32) -> Vec<(String, u32)> {
    let mut v: Vec<(String, u32)> = h
        .with_sync(u, |n| n.discover_results("place", hops).unwrap())
        .into_iter()
        .filter(|r| r.holder.is_some()) // remote rows only; local rows are hops 0
        .map(|r| (r.name, r.hops))
        .collect();
    v.sort();
    v
}

/// One hop: my peer's VISIBLE places answer; a private-visibility one never travels.
#[tokio::test(flavor = "multi_thread")]
async fn a_peers_public_places_answer_and_private_ones_stay_home() {
    let h = Harness::new(&["ada", "ben"]).await;
    h.pair(0, 1).await;

    mint_place(&h, 1, "Blue Bottle", Visibility::Network, 51.4545, -2.5879).await;
    mint_place(
        &h,
        1,
        "Ben's kitchen",
        Visibility::Private,
        51.4546,
        -2.5880,
    )
    .await;

    let rid = h
        .with(
            0,
            |n| async move { n.discover("place", &[], 2).await.unwrap() },
        )
        .await;
    assert!(!rid.is_empty());
    h.settle().await;

    let got = discovered_names(&h, 0, 2);
    assert_eq!(got, vec![("Blue Bottle".to_string(), 1)]);

    // The answer carries a renderable reference row, not a bare id.
    let rows = h.with_sync(0, |n| n.discover_results("place", 2).unwrap());
    let row = rows.iter().find(|r| r.name == "Blue Bottle").unwrap();
    assert_eq!(row.holder, Some(h.id(1)));
    let p = row.point.as_ref().expect("the fix travels");
    assert_eq!(p.lat_e7, 514_545_000);
}

/// Two hops: a friend-of-friend's place reaches the asker via the middle node,
/// attributed to its true holder; budget 1 keeps the question from travelling.
#[tokio::test(flavor = "multi_thread")]
async fn a_friend_of_a_friends_place_arrives_at_two_hops_within_budget() {
    let h = Harness::new(&["ada", "ben", "cyn"]).await;
    h.pair(0, 1).await;
    h.pair(1, 2).await; // ada–ben–cyn: a line, no ada–cyn edge

    mint_place(&h, 2, "Cyn's studio", Visibility::Network, 51.5000, -2.6000).await;

    // Budget 1: the question stops at ben, so cyn's studio never surfaces.
    h.with(
        0,
        |n| async move { n.discover("place", &[], 1).await.unwrap() },
    )
    .await;
    h.settle().await;
    assert!(
        discovered_names(&h, 0, 2).is_empty(),
        "budget 1 must not reach a friend-of-friend"
    );

    // Budget 2: ben forwards, cyn answers, ben relays the answer back.
    h.with(
        0,
        |n| async move { n.discover("place", &[], 2).await.unwrap() },
    )
    .await;
    h.settle().await;
    h.settle().await; // two waves: question out, answers back

    let got = discovered_names(&h, 0, 2);
    assert_eq!(got, vec![("Cyn's studio".to_string(), 2)]);
    let rows = h.with_sync(0, |n| n.discover_results("place", 2).unwrap());
    let row = rows.iter().find(|r| r.name == "Cyn's studio").unwrap();
    assert_eq!(row.holder, Some(h.id(2)), "attributed to its true holder");

    // ...and the hops filter hides it again without a fresh question.
    assert!(discovered_names(&h, 0, 1).is_empty());
}

/// Tags narrow the answer at the RESPONDER: only matching public places travel.
#[tokio::test(flavor = "multi_thread")]
async fn tags_filter_what_a_responder_shares() {
    let h = Harness::new(&["ada", "ben"]).await;
    h.pair(0, 1).await;

    mint_place(
        &h,
        1,
        "Harbourside coffee",
        Visibility::Network,
        51.4500,
        -2.5980,
    )
    .await;
    mint_place(
        &h,
        1,
        "The climbing gym",
        Visibility::Network,
        51.4510,
        -2.5990,
    )
    .await;

    h.with(0, |n| async move {
        n.discover("place", &["coffee".to_string()], 1)
            .await
            .unwrap()
    })
    .await;
    h.settle().await;

    let got = discovered_names(&h, 0, 2);
    assert_eq!(got, vec![("Harbourside coffee".to_string(), 1)]);
}

/// A triangle must quiesce: once everyone has synced, another pass authors
/// nothing on any device — the folds themselves are the flood terminator.
#[tokio::test(flavor = "multi_thread")]
async fn discovery_quiesces_on_a_cyclic_graph() {
    let h = Harness::new(&["ada", "ben", "cyn"]).await;
    h.pair(0, 1).await;
    h.pair(1, 2).await;
    h.pair(0, 2).await; // the cycle

    mint_place(&h, 1, "Ben's bench", Visibility::Network, 51.0, -2.0).await;

    h.with(
        0,
        |n| async move { n.discover("place", &[], 2).await.unwrap() },
    )
    .await;
    h.settle().await;
    h.settle().await;

    for u in 0..3 {
        let n = h
            .with(u, |n| async move { n.reconcile_discovery().await.unwrap() })
            .await;
        assert_eq!(n, 0, "user {u} still had discovery deltas to author");
    }
}

/// Visibility grades by DISTANCE, not by yes/no: a `connections` place answers a
/// friend and never a friend-of-friend, on the same question.
#[tokio::test(flavor = "multi_thread")]
async fn visibility_grades_the_answer_by_the_askers_distance() {
    let h = Harness::new(&["ada", "ben", "cyn"]).await;
    h.pair(0, 1).await;
    h.pair(1, 2).await; // ada–ben–cyn: cyn is ada's friend-of-friend

    mint_place(&h, 2, "For the town", Visibility::Network, 51.60, -2.60).await;
    mint_place(&h, 2, "For friends", Visibility::Connections, 51.61, -2.61).await;

    // ben asks: he is cyn's DIRECT connection, so he sees both.
    h.with(
        1,
        |n| async move { n.discover("place", &[], 2).await.unwrap() },
    )
    .await;
    h.settle().await;
    assert_eq!(
        discovered_names(&h, 1, 2),
        vec![
            ("For friends".to_string(), 1),
            ("For the town".to_string(), 1)
        ]
    );

    // ada asks: cyn is two handshakes out, so only the network place travels —
    // the responder grades by her distance, and every fold on the path would
    // refuse a copy that claimed otherwise.
    h.with(
        0,
        |n| async move { n.discover("place", &[], 2).await.unwrap() },
    )
    .await;
    h.settle().await;
    h.settle().await;
    assert_eq!(
        discovered_names(&h, 0, 2),
        vec![("For the town".to_string(), 2)],
        "a friend-of-friend never learns the connections-only place exists"
    );
}

/// The Trade face reads through the same unified cache: a peer's listing
/// surfaces as a `DiscoveredRow` with its holder, one hop out — no question asked
/// (listings are push gossip).
#[tokio::test(flavor = "multi_thread")]
async fn trade_rows_read_through_the_unified_cache() {
    let h = Harness::new(&["ada", "ben"]).await;
    h.pair(0, 1).await;

    let thing = h.mint_thing(1, "Extension ladder").await;
    h.set_posture(
        1,
        &thing,
        pacific_core::thing::Posture::Offers,
        None,
        pacific_core::thing::Reach::Network,
    )
    .await;
    h.publish_market(1).await;
    h.settle().await;

    let rows = h.with_sync(0, |n| n.discover_results("thing", 2).unwrap());
    let row = rows
        .iter()
        .find(|r| r.name == "Extension ladder")
        .expect("the listing surfaces as a row");
    assert_eq!(row.kind, "thing");
    assert_eq!(row.holder, Some(h.id(1)));
    assert_eq!(
        row.hops, 1,
        "a direct listing's holder is one handshake out"
    );
}

/// App load warms every discoverable kind in one call, idempotently: a still-live
/// question is reused, never re-flooded.
#[tokio::test(flavor = "multi_thread")]
async fn app_load_warm_asks_once_for_every_discover_kind() {
    let h = Harness::new(&["ada", "ben"]).await;
    h.pair(0, 1).await;
    mint_place(&h, 1, "Blue Bottle", Visibility::Network, 51.4545, -2.5879).await;

    let r1 = h
        .with(0, |n| async move { n.warm_gossip_caches().await.unwrap() })
        .await;
    assert_eq!(r1.len(), 1, "one question per discoverable kind");
    h.settle().await;
    assert_eq!(
        discovered_names(&h, 0, 2),
        vec![("Blue Bottle".to_string(), 1)],
        "the warm pass fills the cache without a sheet ever opening"
    );

    // A second app load inside the TTL reuses the live question.
    let r2 = h
        .with(0, |n| async move { n.warm_gossip_caches().await.unwrap() })
        .await;
    assert_eq!(r1, r2, "warming again re-floods nothing");
}

/// Re-asking the same live question reuses its rid instead of re-flooding.
#[tokio::test(flavor = "multi_thread")]
async fn reasking_a_live_question_reuses_its_rid() {
    let h = Harness::new(&["ada", "ben"]).await;
    h.pair(0, 1).await;

    let r1 = h
        .with(
            0,
            |n| async move { n.discover("place", &[], 2).await.unwrap() },
        )
        .await;
    let r2 = h
        .with(
            0,
            |n| async move { n.discover("place", &[], 2).await.unwrap() },
        )
        .await;
    assert_eq!(r1, r2, "an open sheet re-asks without re-flooding");
}

/// The publish leg end-to-end: a member posts a note at a shared Place and the
/// owner folds it; adopting a discovered place mints once and dedups forever.
#[tokio::test(flavor = "multi_thread")]
async fn a_note_publishes_to_a_shared_place_and_adoption_dedups() {
    let h = Harness::new(&["ada", "ben"]).await;
    h.pair(0, 1).await;
    h.settle().await; // stock the prekey pools so the admit can consume one

    // ben opens a public place and admits ada (the doorbell path's outcome).
    let place = mint_place(&h, 1, "The allotment", Visibility::Network, 51.2, -2.2).await;
    {
        let p = place.clone();
        h.with(1, |n| async move {
            n.place_set_access(&p, Access::Public).await.unwrap()
        })
        .await;
    }
    {
        let p = place.clone();
        let ada = h.id(0);
        h.with(1, |n| async move { n.place_admit(&p, &ada).await.unwrap() })
            .await;
    }
    h.settle().await;

    // ada publishes her note there.
    {
        let p = place.clone();
        h.with(0, |n| async move {
            n.place_post(
                &p,
                "note-42",
                "Watering",
                "Courgettes need water Tuesday",
                1_722_600_000_000,
            )
            .await
            .unwrap()
        })
        .await;
    }
    h.settle().await;

    // ben folds ada's note, attributed to her.
    let posts = h.with_sync(1, |n| n.place_posts(&place).unwrap());
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].author, h.id(0));
    assert_eq!(posts[0].text, "Courgettes need water Tuesday");

    // Adoption: ada takes a discovered reference as a local Place; twice is once.
    let a1 = h
        .with(0, |n| async move {
            n.place_adopt(
                "Blue Bottle",
                "",
                Some(51.4545),
                Some(-2.5879),
                "cafe-remote-id",
            )
            .await
            .unwrap()
        })
        .await;
    let a2 = h
        .with(0, |n| async move {
            n.place_adopt(
                "Blue Bottle",
                "",
                Some(51.4545),
                Some(-2.5879),
                "cafe-remote-id",
            )
            .await
            .unwrap()
        })
        .await;
    assert_eq!(a1, a2, "minted wins — the same discovery adopts once");

    // ...and the adopted place is a real Place she can post into immediately.
    {
        let a = a1.clone();
        h.with(0, |n| async move {
            n.place_post(&a, "note-43", "", "Flat white was great", 1_722_600_100_000)
                .await
                .unwrap()
        })
        .await;
    }
    let posts = h.with_sync(0, |n| n.place_posts(&a1).unwrap());
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].text, "Flat white was great");
}
