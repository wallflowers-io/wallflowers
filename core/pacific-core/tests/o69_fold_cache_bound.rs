//! O-69, FC-10: the cache at its bound under a large holding, through a real Node, its
//! bytes counted by the allocator on the thread that builds each entry (common::counting).
//!
//! - what the cache accounts is at least what it retains: the bytes freed when it is
//!   dropped, on a 250-row room alone and on the whole holding;
//! - at a bound below the holding it stays within the bound, retains no more, and every
//!   answer is the unbounded cache's (an eviction is only a miss).
//!
//! The binary runs with `PACIFIC_FOLD_CACHE_VERIFY=1` (FC-2): every hit is also refolded.

mod common;

use common::{counting::allocated, Harness};
use pacific_core::coordinator::{ArgVal, Args, FORUM_POST};
use pacific_core::fold_cache::FoldCache;
use pacific_core::object::ObjectKind;
use pacific_core::Node;

fn cache_on() {
    std::env::set_var("PACIFIC_FOLD_CACHE_MODEL", "o69-bound");
    std::env::set_var("PACIFIC_FOLD_CACHE_VERIFY", "1");
}

fn post(text: &str) -> Args {
    let mut a = Args::new();
    a.insert("text".into(), ArgVal::Text(text.into()));
    a
}

/// Every room's view and thread: the cache's two products of each.
fn read(n: &Node, rooms: &[String]) -> Vec<String> {
    rooms
        .iter()
        .flat_map(|r| {
            let gid = hex::decode(r).unwrap();
            [n.object_view(r).expect("the view"), format!("{:?}", n.forum_thread(&gid).expect("the thread"))]
        })
        .collect()
}

/// (accounted, retained): the cache's own figure, and the bytes this thread frees when the
/// cache goes. It was built on this thread, so nothing else moves the count between.
fn account(n: &Node) -> (usize, usize) {
    let mut c = n.dir.folds.lock().unwrap();
    let accounted = c.bytes();
    let before = allocated();
    drop(std::mem::take(&mut *c));
    (accounted, (before - allocated()).max(0) as usize)
}

#[tokio::test]
async fn fc10_what_is_accounted_is_what_is_retained_and_the_bound_holds() {
    cache_on();
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let mut rooms = Vec::new();
    for (name, posts) in [("One", 250), ("Two", 60), ("Three", 60)] {
        let room = h.mint_object(ada, ObjectKind::Forum, name, &[bo]).await;
        let poster = h.node(bo);
        for i in 0..posts {
            poster.apply(&room, FORUM_POST, post(&format!("{name} {i}"))).await.expect("bo posts");
        }
        drop(poster);
        rooms.push(room);
    }
    h.settle().await;
    let n = h.node(ada);

    // The 250-row room alone.
    *n.dir.folds.lock().unwrap() = FoldCache::with_limit(256 << 20);
    read(&n, &rooms[..1]);
    let (big, retained) = account(&n);
    println!("FC-10 the 250-row room: accounted {big} B, retained {retained} B");
    assert!(retained > 0, "a fold retains something");
    assert!(big >= retained, "accounted {big} < retained {retained}");

    // The whole holding, unbounded: the answers every bound must give.
    *n.dir.folds.lock().unwrap() = FoldCache::with_limit(256 << 20);
    let want = read(&n, &rooms);
    assert_eq!(read(&n, &rooms), want, "warm");
    let (holding, retained) = account(&n);
    println!("FC-10 the holding: accounted {holding} B, retained {retained} B");
    assert!(holding >= retained, "accounted {holding} < retained {retained}");

    // A bound below the holding and above its largest room: it stays within, and nothing
    // answers differently. Each room read twice, so the second is a hit, then the next
    // room evicts it.
    let bound = (big + holding) / 2;
    *n.dir.folds.lock().unwrap() = FoldCache::with_limit(bound);
    for round in 0..3 {
        for (i, r) in rooms.iter().enumerate() {
            for _ in 0..2 {
                assert_eq!(read(&n, std::slice::from_ref(r)), want[2 * i..2 * i + 2], "round {round}, room {i}");
                let at = n.dir.folds.lock().unwrap().bytes();
                assert!(at <= bound, "round {round}, room {i}: {at} B over the bound {bound} B");
            }
        }
    }
    let (hits, misses) = {
        let c = n.dir.folds.lock().unwrap();
        (c.hits, c.misses)
    };
    // 3 rounds × 3 rooms × 2 products: a miss at each room's first read (the last room
    // evicted it), a hit at its second.
    assert_eq!((hits, misses), (18, 18), "every first read missed and every second hit");
    let (accounted, retained) = account(&n);
    println!("FC-10 at the bound {bound} B: accounted {accounted} B, retained {retained} B");
    assert!(accounted >= retained && retained <= bound, "accounted {accounted}, retained {retained}, bound {bound}");
}
