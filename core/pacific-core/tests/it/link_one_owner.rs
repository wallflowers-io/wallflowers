//! #144: one owner of the device's network. Every relay operation goes through the
//! process's link, so a run of ordinary operations, each on a fresh Node as the FFI
//! makes them, dials a bounded number of connections, not one or more per operation.

use crate::common::Harness;

/// The request pool's size per relay (`link::POOL`), plus the listen socket.
const BOUND: u64 = 3;

#[tokio::test]
async fn twenty_operations_on_fresh_nodes_dial_a_bounded_number_of_times() {
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let forum = h.form_forum(ada, &[bo]).await;
    h.settle().await;

    let before = pacific_core::transport::dials();
    for i in 0..5 {
        h.obj_post(ada, &forum, &format!("post {i}")).await;
        h.sync(bo).await;
        h.rename_object(ada, &forum, &format!("forum {i}")).await;
        h.sync(ada).await;
    }
    let dialled = pacific_core::transport::dials() - before;
    assert!(dialled <= BOUND, "20 operations dialled the relay {dialled} times; the link holds at most {BOUND}");
}
