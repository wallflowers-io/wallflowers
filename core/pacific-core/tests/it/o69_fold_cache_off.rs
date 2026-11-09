//! O-69, FC-8: a process with no model id (a dirty or unstamped build that names no commit,
//! and no test model) folds every time, and keeps nothing: not in memory, and not on disk
//! where the embedding keeps folds (option B).

use crate::common;

use common::Harness;
use pacific_core::object::ObjectKind;

#[tokio::test]
async fn no_model_no_cache() {
    std::env::remove_var("PACIFIC_FOLD_CACHE_MODEL");
    std::env::set_var("PACIFIC_FOLD_CACHE_PERSIST", "1");
    let h = Harness::new(&["ada"]).await;
    let room = h.mint_object(0, ObjectKind::Forum, "Talk", &[]).await;
    let n = h.node(0);
    for _ in 0..3 {
        n.object_view(&room).expect("the view");
    }
    let kept: i64 = n.dir.conn.query_row("SELECT count(*) FROM fold_store", [], |r| r.get(0)).unwrap();
    assert_eq!(kept, 0, "nothing kept on disk");
    let c = n.dir.folds.lock().unwrap();
    assert_eq!((c.len(), c.hits, c.misses), (0, 0, 0), "nothing kept, nothing looked up");
}
