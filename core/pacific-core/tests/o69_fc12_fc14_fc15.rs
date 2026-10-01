//! O-69, FC-12, FC-14 and FC-15 (mdr/fold-cache.md § Tests).
//!
//! FC-12: `G` moves on what a graph shows (a post ingested, a write here, a member added, an
//! object joined) and not on a read; a scope's generation moves when an object inside it does
//! and not when one outside it does (NC-41).
//! FC-14: the link down and back. What was published while the held connection was down is
//! delivered when it returns, and ingested with no drain: its tags stay complete across the
//! reconnect.
//! FC-15: the reconciler (`sync_once`) drains no tag the held connection vouches for. Its
//! upkeep (leases, the pool) still reads the relay; the tick itself does not sync.

mod common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args, FORUM_POST};
use pacific_core::object::ObjectKind;
use pacific_core::router::Routes;
use pacific_core::Node;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};

fn cache_on() {
    std::env::set_var("PACIFIC_FOLD_CACHE_MODEL", "o69-fc12");
    std::env::set_var("PACIFIC_FOLD_CACHE_VERIFY", "1");
}

fn post(text: &str) -> Args {
    let mut a = Args::new();
    a.insert("text".into(), ArgVal::Text(text.into()));
    a
}

fn until(secs: u64, what: impl Fn() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if what() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// Bo's own node, held (its connection lives in it), pointed at before each use.
struct Held<'a> {
    h: &'a Harness,
    n: Node,
}

impl Held<'_> {
    fn at(&self) -> &Node {
        let _ = self.h.node(1);
        &self.n
    }

    fn live(h: &Harness) -> Held<'_> {
        let bo = Held { h, n: h.node(1) };
        assert!(bo.at().go_live(Box::new(|| {}), Duration::from_secs(10)).unwrap(), "the relay confirms the held connection");
        bo
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fc12_g_moves_with_what_a_graph_shows_and_a_scope_with_its_own() {
    cache_on();
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let room = h.mint_object(0, ObjectKind::Forum, "Talk", &[1]).await;
    let other = h.mint_object(0, ObjectKind::Forum, "Elsewhere", &[1]).await;
    h.settle().await;
    let bo = Held::live(&h);
    bo.at().sync_once().await.unwrap();
    let (room_id, other_id) = (hex::decode(&room).unwrap(), hex::decode(&other).unwrap());
    let g = || bo.at().dir.generation().unwrap();
    let scope = |id: &Vec<u8>| bo.at().dir.generation_of(&[id.clone()]).unwrap();

    let (g0, r0, o0) = (g(), scope(&room_id), scope(&other_id));
    bo.at().object_view(&room).unwrap();
    bo.at().ingest_delivered().unwrap();
    assert_eq!(g(), g0, "a read moves nothing");

    h.node(0).apply(&room, FORUM_POST, post("in the room")).await.unwrap();
    assert!(until(10, || bo.at().live_pending()));
    bo.at().ingest_delivered().unwrap();
    assert_ne!(g(), g0, "a post ingested moves G");
    assert_ne!(scope(&room_id), r0, "and its object's scope");
    assert_eq!(scope(&other_id), o0, "and no other scope");

    let g1 = g();
    bo.at().apply(&room, FORUM_POST, post("bo writes")).await.unwrap();
    assert_ne!(g(), g1, "a write here moves G");

    let (g2, o2) = (g(), scope(&other_id));
    let bundle = h.node(2).build_contact_bundle().unwrap();
    h.node(0).group_add_member(&other, &bundle).await.unwrap();
    assert!(until(10, || bo.at().live_pending()));
    bo.at().ingest_delivered().unwrap();
    while bo.at().needs_catch_up().unwrap() {
        bo.at().catch_up().await.unwrap();
    }
    assert_ne!(g(), g2, "a member added moves G");
    assert_ne!(scope(&other_id), o2, "and the scope it is in");

    let g3 = g();
    let third = h.mint_object(0, ObjectKind::Forum, "Third", &[]).await;
    let bundle = h.node(1).build_contact_bundle().unwrap();
    h.node(0).group_add_member(&third, &bundle).await.unwrap();
    assert!(until(10, || bo.at().live_pending()));
    bo.at().ingest_delivered().unwrap();
    while bo.at().needs_catch_up().unwrap() {
        bo.at().catch_up().await.unwrap();
    }
    assert_ne!(g(), g3, "an object joined moves G");
}

/// A TCP hop to the relay that can be cut: while down it takes no connection, and cutting
/// drops every one it holds.
struct Cut {
    url: String,
    up: Arc<AtomicBool>,
    conns: Arc<Mutex<Vec<tokio::task::AbortHandle>>>,
}

impl Cut {
    async fn to(relay: &str) -> Cut {
        let target = relay.trim_start_matches("ws://").split('/').next().unwrap().to_string();
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", l.local_addr().unwrap());
        let (up, conns) = (Arc::new(AtomicBool::new(true)), Arc::new(Mutex::new(Vec::new())));
        let (u, c) = (up.clone(), conns.clone());
        tokio::spawn(async move {
            loop {
                let Ok((mut down, _)) = l.accept().await else { return };
                if !u.load(Ordering::SeqCst) {
                    continue;
                }
                let t = target.clone();
                let task = tokio::spawn(async move {
                    if let Ok(mut upstream) = TcpStream::connect(&t).await {
                        let _ = tokio::io::copy_bidirectional(&mut down, &mut upstream).await;
                    }
                });
                c.lock().unwrap().push(task.abort_handle());
            }
        });
        Cut { url, up, conns }
    }

    fn down(&self) {
        self.up.store(false, Ordering::SeqCst);
        for c in self.conns.lock().unwrap().drain(..) {
            c.abort();
        }
    }

    fn restore(&self) {
        self.up.store(true, Ordering::SeqCst);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fc14_what_was_published_while_the_link_was_down_is_read_when_it_returns() {
    cache_on();
    let h = Harness::new(&["ada", "bo"]).await;
    let room = h.mint_object(0, ObjectKind::Forum, "Talk", &[1]).await;
    h.settle().await;
    let cut = Cut::to(&h.relay).await;
    h.node(1).set_routes(Routes::parse(&cut.url)).unwrap();
    let bo = Held::live(&h);
    bo.at().sync_once().await.unwrap();
    assert!(!bo.at().needs_catch_up().unwrap());

    cut.down();
    assert!(until(10, || !bo.at().live_up()), "the held connection saw the link go");
    h.node(0).apply(&room, FORUM_POST, post("while down, one")).await.unwrap();
    h.node(0).apply(&room, FORUM_POST, post("while down, two")).await.unwrap();
    assert_eq!(bo.at().ingest_delivered().unwrap().frames, 0, "nothing arrives while it is down");
    assert!(!bo.at().object_view(&room).unwrap().contains("while down"));

    cut.restore();
    assert!(until(20, || bo.at().live_up()), "it comes back by itself");
    assert!(until(10, || bo.at().live_pending()), "and what was published meanwhile is delivered");
    std::thread::sleep(Duration::from_millis(200));
    let got = bo.at().ingest_delivered().unwrap();
    assert_eq!((got.frames, got.waiting), (2, 0), "ingested at once, the tag still complete: {got:?}");
    let view = bo.at().object_view(&room).unwrap();
    assert!(view.contains("while down, one") && view.contains("while down, two"));
    assert!(!bo.at().needs_catch_up().unwrap(), "nothing owed a drain");
}

#[tokio::test(flavor = "multi_thread")]
async fn fc15_the_reconciler_drains_nothing_the_held_connection_vouches_for() {
    cache_on();
    let h = Harness::new(&["ada", "bo"]).await;
    let room = h.mint_object(0, ObjectKind::Forum, "Talk", &[1]).await;
    h.settle().await;
    let bo = Held::live(&h);
    bo.at().sync_once().await.unwrap();
    h.node(0).apply(&room, FORUM_POST, post("heard")).await.unwrap();
    assert!(until(10, || bo.at().live_pending()));
    bo.at().ingest_delivered().unwrap();

    let before = bo.at().tag_drains();
    bo.at().sync_once().await.unwrap();
    assert_eq!(bo.at().tag_drains() - before, 0, "the reconciler drained no tag");
    assert!(bo.at().object_view(&room).unwrap().contains("heard"));
}
