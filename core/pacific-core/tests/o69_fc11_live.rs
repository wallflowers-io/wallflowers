//! O-69, FC-11 and FC-13 (mdr/fold-cache.md § Tests): a read converges without a sync.
//!
//! FC-11: a frame the relay delivers on the held connection is in the next read, ingested by
//! `ingest_delivered`, which is not async and so cannot make a round trip; a read with nothing
//! delivered ingests nothing; every answer equals a fresh fold (FC-2's verify is on).
//! FC-13: the connection follows the tags. After a commit (a member added), the new epoch's
//! messages ring and are ingested; after the drain that records the new epoch, nothing waits.

mod common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args, FORUM_POST};
use pacific_core::object::ObjectKind;
use pacific_core::Node;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn cache_on() {
    std::env::set_var("PACIFIC_FOLD_CACHE_MODEL", "o69-fc11");
    std::env::set_var("PACIFIC_FOLD_CACHE_VERIFY", "1");
}

fn post(text: &str) -> Args {
    let mut a = Args::new();
    a.insert("text".into(), ArgVal::Text(text.into()));
    a
}

/// Until `what`, or 10 s.
fn until(what: impl Fn() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if what() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// Bo's own node, held: its held connection lives in it. `h.node(1)` points the process's
/// state at bo before each use (paths are read per call).
struct Held<'a> {
    h: &'a Harness,
    n: Node,
    rang: Arc<AtomicU64>,
}

impl Held<'_> {
    fn at(&self) -> &Node {
        let _ = self.h.node(1);
        &self.n
    }

    /// As the Door's bell does: ingest, then drain what the connection cannot vouch for.
    async fn rang(&self) {
        self.at().ingest_delivered().unwrap();
        while self.at().needs_catch_up().unwrap() {
            self.at().catch_up().await.unwrap();
            std::thread::sleep(Duration::from_millis(50));
            self.at().ingest_delivered().unwrap();
        }
    }

    fn view(&self, obj: &str) -> String {
        self.at().object_view(obj).unwrap()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fc11_fc13_what_the_relay_delivers_is_read_without_a_sync() {
    cache_on();
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let room = h.mint_object(0, ObjectKind::Forum, "Talk", &[1]).await;
    h.settle().await;

    let rang = Arc::new(AtomicU64::new(0));
    let r = rang.clone();
    let bo = Held { h: &h, n: h.node(1), rang: rang.clone() };
    assert!(
        bo.at().go_live(Box::new(move || { r.fetch_add(1, Ordering::SeqCst); }), Duration::from_secs(10)).unwrap(),
        "the relay confirms the held connection"
    );
    // The first pass after the connection is confirmed leaves every tag complete.
    bo.at().sync_once().await.unwrap();
    assert!(!bo.at().needs_catch_up().unwrap(), "after the first pass nothing is owed a drain");
    assert_eq!(bo.at().ingest_delivered().unwrap().frames, 0, "a read with nothing delivered ingests nothing");

    // FC-11: ada posts; bo hears it and reads it, with no sync.
    let before = bo.rang.load(Ordering::SeqCst);
    h.node(0).apply(&room, FORUM_POST, post("heard, not drained")).await.unwrap();
    assert!(until(|| bo.at().live_pending()), "the relay pushed ada's post to bo's held connection");
    assert!(bo.rang.load(Ordering::SeqCst) > before, "and rang");
    let wire = || (pacific_core::transport::dials(), pacific_core::transport::drains(), pacific_core::router::published());
    let at_read = wire();
    let got = bo.at().ingest_delivered().unwrap();
    assert_eq!((got.frames, got.waiting), (1, 0), "ingested at once, nothing held back: {got:?}");
    assert!(bo.view(&room).contains("heard, not drained"), "bo's next read holds it");
    assert_eq!(wire(), at_read, "the read made no round trip: no dial, no drain, no publish");
    let group = hex::decode(&room).unwrap();
    assert!(bo.at().caught_up(&group).unwrap(), "a writer here could author with no drain");

    // FC-13: ada adds cy; the commit moves the epoch on bo's side too.
    // Not `add_to_forum`: its settle would sync bo, and bo must hear this, not drain it.
    let bundle = h.node(2).build_contact_bundle().unwrap();
    h.node(0).group_add_member(&room, &bundle).await.unwrap();
    h.node(2).sync_once().await.unwrap();
    assert!(until(|| bo.at().live_pending()), "the commit was pushed to bo");
    let got = bo.at().ingest_delivered().unwrap();
    assert_eq!(got.commits, 1, "the commit ingested: {got:?}");
    assert!(bo.at().needs_catch_up().unwrap(), "an epoch moved: the object is drained epoch by epoch");
    bo.rang().await;
    assert!(!bo.at().needs_catch_up().unwrap(), "the new epoch's tag is held and complete");

    // A message at the new epoch rings and is read, again with no sync.
    h.node(0).apply(&room, FORUM_POST, post("at the new epoch")).await.unwrap();
    assert!(until(|| bo.at().live_pending()), "the new epoch's tag is on the held connection");
    let got = bo.at().ingest_delivered().unwrap();
    assert_eq!((got.frames, got.waiting), (1, 0), "{got:?}");
    assert!(bo.view(&room).contains("at the new epoch"));

    // FC-13: a Welcome on bo's intro mailbox is heard, and bo joins, with no sync of its own.
    let second = h.mint_object(0, ObjectKind::Forum, "Second", &[]).await;
    let bundle = h.node(1).build_contact_bundle().unwrap();
    h.node(0).group_add_member(&second, &bundle).await.unwrap();
    assert!(until(|| bo.at().live_pending()), "the Welcome was pushed to bo's intro mailbox");
    let got = bo.at().ingest_delivered().unwrap();
    assert_eq!(got.joined, 1, "the Welcome joined: {got:?}");
    bo.rang().await;
    assert!(bo.at().objects_named().unwrap().iter().any(|(id, _, _)| *id == second), "bo holds the new object");
    h.node(0).apply(&second, FORUM_POST, post("in the second room")).await.unwrap();
    assert!(until(|| bo.at().live_pending()), "the new object's tag is on the held connection");
    bo.at().ingest_delivered().unwrap();
    assert!(bo.view(&second).contains("in the second room"));

    // What bo read equals what a full pass would hold.
    let at_read = wire();
    bo.at().ingest_delivered().unwrap();
    bo.view(&room);
    assert_eq!(wire(), at_read, "a read with nothing delivered makes no round trip");
    let held = bo.view(&room);
    h.sync(1).await;
    assert_eq!(held, h.node(1).object_view(&room).unwrap(), "the held connection's reads equal a sync's");
}

/// FC-13: the connection follows the tags down as well as up. An epoch past EPOCH_RETENTION
/// is no longer listened on; an object bo is removed from leaves the set, and what is said in
/// it afterwards is not delivered to bo.
#[tokio::test(flavor = "multi_thread")]
async fn fc13_old_epochs_and_a_departed_object_leave_the_set() {
    cache_on();
    let h = Harness::new(&["ada", "bo", "cy", "dy", "ey", "fy"]).await;
    let room = h.mint_object(0, ObjectKind::Forum, "Talk", &[1]).await;
    h.settle().await;
    let bo = Held { h: &h, n: h.node(1), rang: Arc::new(AtomicU64::new(0)) };
    assert!(bo.at().go_live(Box::new(|| {}), Duration::from_secs(10)).unwrap());
    bo.at().sync_once().await.unwrap();
    let group = hex::decode(&room).unwrap();
    let room_tags = |b: &Held| -> Vec<String> {
        b.at().dir.epoch_tags(&group).unwrap().into_iter().map(|(_, t, _)| pacific_wire::tag_hex(&t)).collect()
    };
    let first = room_tags(&bo);
    for u in 2..6 {
        let bundle = h.node(u).build_contact_bundle().unwrap();
        h.node(0).group_add_member(&room, &bundle).await.unwrap();
        h.node(u).sync_once().await.unwrap();
        assert!(until(|| bo.at().live_pending()), "commit {u} pushed to bo");
        bo.rang().await;
    }
    let held = bo.at().live_tags_held();
    let now = room_tags(&bo);
    assert!(now.iter().all(|t| held.contains(t)), "every retained epoch is listened on");
    assert!(first.iter().any(|t| !held.contains(t)), "an epoch past retention is not: {first:?} -> {held:?}");

    h.node(0).group_remove_member(&room, &hex::encode(h.id(1)), None).await.unwrap();
    assert!(until(|| bo.at().live_pending()), "the removal was pushed to bo");
    bo.rang().await;
    let held = bo.at().live_tags_held();
    assert!(now.iter().all(|t| !held.contains(t)), "a departed object's tags leave the set: {held:?}");
    h.node(0).apply(&room, FORUM_POST, post("after bo left")).await.unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert!(!bo.at().live_pending(), "nothing said in it afterwards reaches bo");
}
