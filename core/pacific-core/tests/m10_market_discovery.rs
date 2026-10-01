//! M10 — a listing reaches people you have never met.
//!
//! The Thing object was already there: you could mint one, put it in a posture, price it,
//! and set how far it may travel. What was missing was the travelling. A minted Thing is a
//! group of one — it never left the device — so the market was an author surface and a
//! mirror of your own inventory. These scenarios prove the channel that fixes it, end to
//! end: real relay, real MLS, real device boundaries, real `contact.publishListing` deltas
//! sealed as MLS application messages.
//!
//! MULTIHOP is the point. A `network` listing is relayed by each recipient into their own
//! connections, up to `MAX_LISTING_HOPS`, carrying its ORIGIN unchanged and its RELAYER as
//! the delta author. That is what turns a set of private 1:1 channels into a market: Cleo
//! finds Ada's ladder without ever having met Ada, and without anyone's contact list being
//! shared with anyone.
//!
//! What each one pins:
//!   1. fan-out       — a posture reaches every connection, not just the newest.
//!   2. multihop      — it travels on to a friend-of-a-friend, attributed and attenuated.
//!   3. consent       — a `private` listing stops dead at the first hop.
//!   4. the ceiling   — a listing does not outrun the trust that carried it.
//!   5. withdrawal    — clearing a posture chases the listing down every path it took.
//!   6. edits         — a price change overtakes the copy already out there.
//!   7. idempotence   — an unchanged market authors nothing and the flood terminates.
//!   8. no self-echo  — my own listings never come back to me as discoveries.

mod common;

use common::Harness;
use pacific_core::contact::MAX_LISTING_HOPS;
use pacific_core::thing::{Posture, Reach};

/// Mint a thing, put it in a posture, and fan it out. Returns the thing's object id.
async fn list(
    h: &Harness,
    u: usize,
    name: &str,
    posture: Posture,
    price: Option<&str>,
    reach: Reach,
) -> String {
    let id = h.mint_thing(u, name).await;
    h.set_posture(u, &id, posture, price, reach).await;
    h.publish_market(u).await;
    id
}

/// Every live listing `u` has discovered, by title.
fn discovered(h: &Harness, u: usize) -> Vec<String> {
    h.discovered_titles(u)
}

#[tokio::test]
async fn a_posture_reaches_every_connection() {
    let h = Harness::new(&["Ada", "Bob", "Cleo"]).await;
    let (ada, bob, cleo) = (0, 1, 2);
    h.pair(ada, bob).await;
    h.pair(ada, cleo).await;
    h.settle().await;

    list(
        &h,
        ada,
        "Extension ladder",
        Posture::Offers,
        None,
        Reach::Network,
    )
    .await;
    h.settle().await;

    for peer in [bob, cleo] {
        assert_eq!(
            discovered(&h, peer),
            vec!["Extension ladder"],
            "user {peer} must discover a listing from their connection"
        );
    }
}

#[tokio::test]
async fn a_network_listing_travels_to_a_friend_of_a_friend() {
    // The headline: Cleo and Ada have never met. Bob knows them both. Ada's listing
    // reaches Cleo anyway — attributed to Ada, vouched for by Bob.
    let h = Harness::new(&["Ada", "Bob", "Cleo"]).await;
    let (ada, bob, cleo) = (0, 1, 2);
    h.pair(ada, bob).await;
    h.pair(bob, cleo).await;
    h.settle().await;

    let bike = h.mint_thing(ada, "Cargo bike").await;
    h.set_posture(ada, &bike, Posture::Selling, Some("£300"), Reach::Network)
        .await;
    // The listing's FACE rides the announcement — one still, base64-in-delta.
    h.with(ada, |n| {
        let bike = bike.clone();
        async move {
            let mut a = pacific_core::coordinator::Args::new();
            a.insert(
                "photo".into(),
                pacific_core::coordinator::ArgVal::Text("QkFTRTY0".into()),
            );
            a.insert(
                "mime".into(),
                pacific_core::coordinator::ArgVal::Text("image/jpeg".into()),
            );
            n.apply(&bike, pacific_core::thing::OP_SET_PHOTO, a)
                .await
        }
    })
    .await
    .expect("photo lands");
    h.publish_market(ada).await;
    h.settle().await;
    // Bob relays what he now holds; the second settle carries it to Cleo.
    h.publish_market(bob).await;
    h.settle().await;

    let found = h.node(cleo).inbound_listings().unwrap();
    let l = found
        .iter()
        .find(|l| l.title == "Cargo bike")
        .expect("Cleo must discover Ada's listing through Bob");
    assert_eq!(l.origin, h.id(ada), "attributed to Ada, who made it");
    assert_eq!(l.via, h.id(bob), "and vouched for by Bob, who passed it on");
    assert_eq!(l.hops, 1, "one relay");
    assert_eq!(
        l.price.as_deref(),
        Some("£300"),
        "the whole listing crossed, not a stub"
    );
    assert_eq!(
        (l.photo.as_str(), l.photo_mime.as_str()),
        ("QkFTRTY0", "image/jpeg"),
        "the face crossed with it — set by Ada, carried unchanged through Bob"
    );
}

#[tokio::test]
async fn a_private_listing_stops_at_the_first_hop() {
    // The consent guarantee. `private` means "my connections, and no further" — and it is
    // the recipients' reducers that enforce it, not the sender's good manners.
    let h = Harness::new(&["Ada", "Bob", "Cleo"]).await;
    let (ada, bob, cleo) = (0, 1, 2);
    h.pair(ada, bob).await;
    h.pair(bob, cleo).await;
    h.settle().await;

    list(
        &h,
        ada,
        "Grandmother's ring",
        Posture::Selling,
        Some("£20"),
        Reach::Private,
    )
    .await;
    h.settle().await;
    h.publish_market(bob).await;
    h.settle().await;

    assert_eq!(
        discovered(&h, bob),
        vec!["Grandmother's ring"],
        "Bob, a direct connection, sees it"
    );
    assert!(
        discovered(&h, cleo).is_empty(),
        "Cleo must NOT: a private listing is never relayed"
    );
}

#[tokio::test]
async fn a_listing_does_not_outrun_the_trust_that_carried_it() {
    // A chain longer than the ceiling. The listing propagates to exactly MAX_LISTING_HOPS
    // and stops, so a market stays a neighbourhood rather than becoming a broadcast.
    let names: Vec<String> = (0..=(MAX_LISTING_HOPS as usize + 2))
        .map(|i| format!("U{i}"))
        .collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let h = Harness::new(&refs).await;

    // A line: 0 — 1 — 2 — ... — N
    for i in 0..refs.len() - 1 {
        h.pair(i, i + 1).await;
    }
    h.settle().await;

    list(
        &h,
        0,
        "Post-hole digger",
        Posture::Offers,
        None,
        Reach::Network,
    )
    .await;
    h.settle().await;
    // Relay repeatedly so the listing walks as far down the line as it is allowed.
    for _ in 0..refs.len() {
        for u in 0..refs.len() {
            h.publish_market(u).await;
        }
        h.settle().await;
    }

    for (u, _) in refs.iter().enumerate().skip(1) {
        let found = discovered(&h, u);
        if u <= MAX_LISTING_HOPS as usize + 1 {
            assert_eq!(found, vec!["Post-hole digger"], "user {u} is within reach");
        } else {
            assert!(
                found.is_empty(),
                "user {u} is past the ceiling and must not see it"
            );
        }
    }
}

#[tokio::test]
async fn withdrawing_a_posture_chases_it_down_every_path() {
    let h = Harness::new(&["Ada", "Bob", "Cleo"]).await;
    let (ada, bob, cleo) = (0, 1, 2);
    h.pair(ada, bob).await;
    h.pair(bob, cleo).await;
    h.settle().await;

    let id = list(
        &h,
        ada,
        "Cargo bike",
        Posture::Selling,
        Some("£300"),
        Reach::Network,
    )
    .await;
    h.settle().await;
    h.publish_market(bob).await;
    h.settle().await;
    assert_eq!(
        discovered(&h, cleo),
        vec!["Cargo bike"],
        "it got there first"
    );

    // Sold. Clearing the posture must reach BOTH the direct connection and the stranger.
    h.clear_posture(ada, &id).await;
    h.publish_market(ada).await;
    h.settle().await;
    h.publish_market(bob).await;
    h.settle().await;

    assert!(
        discovered(&h, bob).is_empty(),
        "the direct connection sees it withdrawn"
    );
    assert!(
        discovered(&h, cleo).is_empty(),
        "and so does the friend-of-a-friend — the tombstone travels the same path"
    );
}

#[tokio::test]
async fn an_edit_overtakes_the_copy_already_out_there() {
    let h = Harness::new(&["Ada", "Bob", "Cleo"]).await;
    let (ada, bob, cleo) = (0, 1, 2);
    h.pair(ada, bob).await;
    h.pair(bob, cleo).await;
    h.settle().await;

    let id = list(
        &h,
        ada,
        "Cargo bike",
        Posture::Selling,
        Some("£300"),
        Reach::Network,
    )
    .await;
    h.settle().await;
    h.publish_market(bob).await;
    h.settle().await;

    h.set_posture(ada, &id, Posture::Selling, Some("£250"), Reach::Network)
        .await;
    h.publish_market(ada).await;
    h.settle().await;
    h.publish_market(bob).await;
    h.settle().await;

    let l = h
        .node(cleo)
        .inbound_listings()
        .unwrap()
        .into_iter()
        .find(|l| l.title == "Cargo bike")
        .expect("still discoverable");
    assert_eq!(
        l.price.as_deref(),
        Some("£250"),
        "the newer revision won at two removes"
    );
}

#[tokio::test]
async fn an_unchanged_market_authors_nothing() {
    // Idempotence is what makes it safe to call on every sync — and it is also the flood
    // terminator: a relay that has nothing new to say goes quiet, so a cyclic graph
    // quiesces instead of echoing forever.
    let h = Harness::new(&["Ada", "Bob", "Cleo"]).await;
    let (ada, bob, cleo) = (0, 1, 2);
    h.pair(ada, bob).await;
    h.pair(bob, cleo).await;
    h.pair(ada, cleo).await; // a CYCLE: every pair connected
    h.settle().await;

    list(
        &h,
        ada,
        "Extension ladder",
        Posture::Offers,
        None,
        Reach::Network,
    )
    .await;
    h.settle().await;
    for _ in 0..3 {
        for u in [ada, bob, cleo] {
            h.publish_market(u).await;
        }
        h.settle().await;
    }

    // Everything has now been said. Another full round must author nothing at all.
    let mut authored = 0usize;
    for u in [ada, bob, cleo] {
        authored += h.publish_market(u).await;
    }
    assert_eq!(authored, 0, "a settled market is silent");

    for peer in [bob, cleo] {
        assert_eq!(
            discovered(&h, peer),
            vec!["Extension ladder"],
            "and still discovered once"
        );
    }
}

#[tokio::test]
async fn my_own_listing_never_comes_back_to_me_as_a_discovery() {
    let h = Harness::new(&["Ada", "Bob"]).await;
    let (ada, bob) = (0, 1);
    h.pair(ada, bob).await;
    h.settle().await;

    list(
        &h,
        ada,
        "Extension ladder",
        Posture::Offers,
        None,
        Reach::Network,
    )
    .await;
    h.settle().await;
    h.publish_market(bob).await;
    h.settle().await;

    assert!(
        discovered(&h, ada).is_empty(),
        "the market lists what OTHERS offer; my own things are read from things()"
    );
    assert_eq!(discovered(&h, bob), vec!["Extension ladder"]);
}
