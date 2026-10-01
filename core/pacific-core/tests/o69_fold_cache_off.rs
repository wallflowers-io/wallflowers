//! O-69, FC-8: a process with no model id (a dirty or unstamped build that names no commit,
//! and no test model) folds every time, and keeps nothing.

mod common;

use common::Harness;
use pacific_core::object::ObjectKind;

#[tokio::test]
async fn no_model_no_cache() {
    std::env::remove_var("PACIFIC_FOLD_CACHE_MODEL");
    let h = Harness::new(&["ada"]).await;
    let room = h.mint_object(0, ObjectKind::Forum, "Talk", &[]).await;
    let n = h.node(0);
    for _ in 0..3 {
        n.object_view(&room).expect("the view");
    }
    let c = n.dir.folds.lock().unwrap();
    assert_eq!((c.len(), c.hits, c.misses), (0, 0, 0), "nothing kept, nothing looked up");
}
