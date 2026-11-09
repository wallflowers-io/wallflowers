//! m28 — the drain: a minted object becomes NAMEABLE from the seed.
//!
//! `m27` pins the state before this existed: `object_new` seals a spine entry and
//! queues it, and a queued entry lives in `pacific.db`, which is exactly what a
//! recovering device does not have. This file pins the step that moves it from
//! "this device knows" to "the seed can find it".
//!
//! WHAT IT DOES NOT CLAIM. Naming is one of three things a recoverable object
//! needs, and it is the only one the drain supplies. The archive key records and
//! seed-reachable MLS state are still unwritten, so these objects remain
//! unrecoverable and `m27`'s sweep still lists them — deliberately. When the other
//! two land, that sweep empties and m27 gets inverted. `named` flipping here is
//! the first of the three, not the migration finishing.
//!
//! WHAT `named` PROVES, AND WHAT IT DOES NOT. `recoverability` reads the
//! DELIVERED rows and opens each under the storage root, so a green assertion
//! there means the entry sealed correctly under the account's own key and names
//! this group — not that a boolean was flipped nearby. But the blob it opens is
//! the one in `pacific.db`. NOTHING IS FETCHED. The evidence is a past `publish`
//! ack recorded locally, while the claim is present tense, so any relay that acks
//! and then does not keep — a retention window, an eviction, an ack before a
//! durable write — leaves `named` true for ever while the account comes back
//! holding nothing.
//!
//! `Node::verify_recoverability` closes that by fetching the entry back from the
//! address its index derives. The last two tests here are the ones that earn the
//! word "published": one that the fetch agrees when the relay kept it, and one
//! that it DISAGREES when the relay does not have it — which is the case the
//! local answer cannot see and the reason the fetch exists.

use crate::common;

use common::Harness;
use pacific_core::router::Routes;

/// The move: queued before the sync, published after it, and the object can be
/// named from the seed rather than from this device's database.
#[tokio::test]
async fn syncing_publishes_the_spine_and_the_object_becomes_nameable() {
    let h = Harness::new(&["Ana"]).await;
    let n = h.node(0);

    // The self record's own vertebra is already owed (m30), so this counts the
    // DELTA the mint adds rather than the whole queue.
    let before = n.pending_spine_entries().unwrap().len();
    let object = n.object_new("forum", "thursday").expect("mint a forum");

    assert_eq!(
        n.pending_spine_entries().unwrap().len(),
        before + 1,
        "the mint queues exactly one entry"
    );
    assert!(
        !n.recoverability(&object).unwrap().named,
        "a queued entry is not a published one — this is m27's assertion"
    );

    h.sync(0).await;

    let n = h.node(0);
    assert!(
        n.pending_spine_entries().unwrap().is_empty(),
        "the queue drained — nothing is still owed"
    );
    assert!(
        n.recoverability(&object).unwrap().named,
        "the entry is published, so a device with the seed can NAME this object"
    );
}

/// The drain is not the migration. Naming is one gap of three.
#[tokio::test]
async fn naming_an_object_does_not_make_it_recoverable() {
    let h = Harness::new(&["Ben"]).await;
    let object = h.node(0).object_new("forum", "one").expect("mint");
    h.sync(0).await;

    let n = h.node(0);
    let r = n.recoverability(&object).unwrap();

    assert!(r.named, "the drain ran");
    assert!(
        !r.recoverable(),
        "named is not recoverable: the archive key records and the MLS state are \
         still unwritten, and claiming otherwise is the lie this path exists to \
         stop telling — {r:?}"
    );
    assert!(!r.speakable, "MLS state is still only in pacific.db");
    assert!(
        r.unreadable_epochs.contains(&0),
        "epoch 0 still has no archive key record"
    );
    assert!(
        !n.unrecoverable_objects().unwrap().is_empty(),
        "m27's sweep still lists it, and must keep doing so until all three land"
    );
}

/// Draining twice publishes nothing the second time, and a later mint takes the
/// next index rather than re-using one already spent.
#[tokio::test]
async fn the_drain_is_idempotent_and_later_mints_take_the_next_index() {
    let h = Harness::new(&["Cass"]).await;
    let first = h.node(0).object_new("forum", "friday").expect("mint");
    h.sync(0).await;

    // A second sync with nothing owed must be a no-op, not a republish.
    assert_eq!(
        h.node(0).drain_spine().await.unwrap(),
        0,
        "nothing queued, nothing published"
    );

    let delivered = h.node(0).spine_entries().unwrap().len();
    let second = h.node(0).object_new("thing", "kettle").expect("mint a second");
    let queued = h.node(0).pending_spine_entries().unwrap();
    assert_eq!(queued.len(), 1, "only the new one is owed");
    assert_eq!(
        queued[0].0 as usize, delivered,
        "the next index — the delivered entries did not release theirs"
    );

    h.sync(0).await;

    let n = h.node(0);
    for o in [&first, &second] {
        assert!(
            n.recoverability(o).unwrap().named,
            "both objects are named once both entries are published"
        );
    }
}

/// The fetch agrees with the local answer when the relay did keep it.
#[tokio::test]
async fn verify_fetches_the_entry_back_and_agrees() {
    let h = Harness::new(&["Dot"]).await;
    let object = h.node(0).object_new("forum", "saturday").expect("mint");
    h.sync(0).await;

    let n = h.node(0);
    assert!(n.recoverability(&object).unwrap().named, "the local answer");
    assert!(
        n.verify_recoverability(&object).await.unwrap().named,
        "and the fetched one — the bytes are at the address the index derives"
    );
}

/// THE CASE THE LOCAL ANSWER CANNOT SEE. A relay that acked and then does not
/// have it: `named` stays true locally for ever, and only the fetch catches it.
///
/// Simulated by pointing the account at a relay that never received the entry,
/// which is what a retention window, an eviction or a lost volume looks like from
/// the account's side. This is the failure the drain documents as "silent and
/// total"; the assertion below is it being neither.
#[tokio::test]
async fn a_relay_that_does_not_have_it_is_caught_only_by_the_fetch() {
    let h = Harness::new(&["Eve"]).await;
    let object = h.node(0).object_new("forum", "sunday").expect("mint");
    h.sync(0).await;

    // The local claim survives, because it is a record of a past ack.
    assert!(
        h.node(0).recoverability(&object).unwrap().named,
        "locally this account still believes the entry was published"
    );

    // A relay with nothing in it — the bytes the account would recover from are
    // gone, and nothing on this device knows.
    let empty = {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { relay::serve(l).await });
        format!("ws://{addr}")
    };
    let mut n = h.node(0);
    n.set_routes(Routes::parse(&empty)).unwrap();

    let r = n.verify_recoverability(&object).await.unwrap();
    assert!(
        !r.named,
        "the fetch must not find an entry the relay does not have — if this is \
         true, `named` is reporting a past ack as a present fact and the account \
         would be told it can recover something it cannot: {r:?}"
    );
}

/// A relay in front of `relay` that drops the connection at the first subscribe after a
/// commit-flagged publish: the walk after a lost index, failing.
async fn cut_at_the_walk(relay: &str) -> String {
    use futures_util::{SinkExt, StreamExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let upstream = relay.to_string();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = l.accept().await {
            let upstream = upstream.clone();
            tokio::spawn(async move {
                let Ok(client) = tokio_tungstenite::accept_async(tcp).await else { return };
                let Ok((server, _)) = tokio_tungstenite::connect_async(upstream.as_str()).await else { return };
                let ((mut c_tx, mut c_rx), (mut s_tx, mut s_rx)) = (client.split(), server.split());
                let up = async {
                    let mut committed = false;
                    while let Some(Ok(m)) = c_rx.next().await {
                        match m.to_text().ok().and_then(|t| pacific_wire::Frame::from_json(t).ok()) {
                            Some(pacific_wire::Frame::Pub { commit: true, .. }) => committed = true,
                            Some(pacific_wire::Frame::Sub { .. }) if committed => return,
                            _ => {}
                        }
                        if s_tx.send(m).await.is_err() {
                            return;
                        }
                    }
                };
                let down = async {
                    while let Some(Ok(m)) = s_rx.next().await {
                        if c_tx.send(m).await.is_err() {
                            return;
                        }
                    }
                };
                tokio::select! { _ = up => {}, _ = down => {} }
            });
        }
    });
    format!("ws://{addr}")
}

/// THE SPINE IS APPEND-ONLY, AND SO IS WHAT IS QUEUED FOR IT. The laptop takes the index the
/// phone's entry was queued for; the phone's entry moves past it, after a walk to the end.
/// A relay that fails in that walk leaves the phone's queue as it was, and the next drain
/// appends it, with the laptop's entry known at the index it won.
#[tokio::test]
async fn a_relay_that_fails_while_an_index_is_lost_leaves_the_queue_whole() {
    let h = Harness::people(&[("ana", &["phone", "laptop"])]).await;
    let (phone, laptop) = (h.device("ana", "phone"), h.device("ana", "laptop"));
    h.settle().await;

    let thursday = h.node(phone).object_new("forum", "thursday").expect("the phone mints");
    h.node(laptop).object_new("forum", "friday").expect("the laptop mints");
    let won = h.node(laptop).pending_spine_entries().unwrap()[0].0;
    h.node(laptop).drain_spine().await.expect("the laptop appends first");

    let mut n = h.node(phone);
    let queued = n.dir.pending_spine(0).unwrap();
    assert_eq!(queued.first().map(|q| q.0), Some(won), "the phone's entry was queued for the index the laptop won");

    n.set_routes(Routes::parse(&cut_at_the_walk(&h.relay).await)).unwrap();
    assert!(n.drain_spine().await.is_err(), "the walk failed");
    assert_eq!(n.dir.pending_spine(0).unwrap(), queued, "every queued entry, at its index, as it was");

    n.set_routes(Routes::parse(&h.relay)).unwrap();
    n.drain_spine().await.expect("the next drain appends it");
    assert!(n.dir.pending_spine(0).unwrap().is_empty(), "nothing still owed");
    assert!(n.recoverability(&thursday).unwrap().named, "the phone's object is named on the spine");
    let laptops = h.node(laptop).dir.delivered_spine(0).unwrap().into_iter().find(|e| e.0 == won);
    let phones = h.node(phone).dir.delivered_spine(0).unwrap().into_iter().find(|e| e.0 == won);
    assert_eq!(phones, laptops, "the phone holds the laptop's entry at the index it won");
}
