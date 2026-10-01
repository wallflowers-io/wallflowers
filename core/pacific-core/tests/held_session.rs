//! One relay session per Node, held between syncs (K-33; mdr/door.md §4 step 5).
//! A pass that ends well keeps it; changed routes and a dead relay drop it.

mod common;

use common::Harness;
use pacific_core::router::Routes;

#[tokio::test]
async fn a_node_that_syncs_holds_one_session_and_still_delivers_on_it() {
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let obj = h.form_forum(ada, &[bo]).await;
    h.settle().await;

    let mut node = h.node(ada);
    assert!(!node.holds_session());
    node.sync_once().await.unwrap();
    assert!(node.holds_session(), "a pass that ends well keeps its session");

    // Mail that arrives while it is held is drained on it.
    h.obj_post(bo, &obj, "while held").await;
    drop(h.node(ada)); // the state dir is read per call: point it back at ada
    node.sync_once().await.unwrap();
    assert!(node.holds_session());
    let seen: Vec<String> = h.obj_view(ada, &obj).into_iter().map(|m| m.text).collect();
    assert!(seen.contains(&"while held".to_string()), "{seen:?}");

    // Other routes, other place: the held session goes.
    drop(h.node(ada));
    node.set_routes(Routes::parse("ws://127.0.0.1:1")).unwrap();
    assert!(!node.holds_session());
    assert!(node.sync_once().await.is_err(), "no relay at the new routes");
    assert!(!node.holds_session(), "a pass that failed keeps nothing");
    node.set_routes(Routes::parse(&h.relay)).unwrap();
}
