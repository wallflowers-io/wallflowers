//! O-69 option B, the folds KEPT ON DISK (`fold_cache::set_persist`; Ralph, 8 Nov): what a
//! long-lived device folded is in `fold_store`, so a new open reads it rather than
//! refolding and re-verifying. The binary runs with `PACIFIC_FOLD_CACHE_VERIFY=1` (FC-2),
//! so every disk hit below is also refolded and compared.
//!
//! - a reopen reads what the last open folded, from disk, and it is what a fold gives;
//! - a new Delta moves the key: that object misses, and folds afresh;
//! - another build's model keeps nothing it can use;
//! - `refold_moved` keeps what a sync brought, before anything reads it;
//! - generations that start again take the kept folds with them.
//!
//! FC-5 (B), a restore folding the same with `fold_store` and without it, is in
//! `o69_fc5_seal.rs`; FC-8, a process with no model keeping nothing, in
//! `o69_fold_cache_off.rs`.

use crate::common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args, FORUM_POST};
use pacific_core::object::ObjectKind;
use pacific_core::Node;

/// Before anything reads the model, the verify switch or the persist switch: each is read
/// once per process.
fn kept_on() {
    std::env::set_var("PACIFIC_FOLD_CACHE_MODEL", "o69-kept");
    std::env::set_var("PACIFIC_FOLD_CACHE_VERIFY", "1");
    std::env::set_var("PACIFIC_FOLD_CACHE_PERSIST", "1");
}

fn post(text: &str) -> Args {
    let mut a = Args::new();
    a.insert("text".into(), ArgVal::Text(text.into()));
    a
}

/// (hits, misses, of the misses those read from disk).
fn counts(n: &Node) -> (u64, u64, u64) {
    let c = n.dir.folds.lock().unwrap();
    (c.hits, c.misses, c.kept)
}

fn kept_rows(n: &Node) -> i64 {
    n.dir.conn.query_row("SELECT count(*) FROM fold_store", [], |r| r.get(0)).unwrap()
}

/// What a reader is given: the room's view, the group's state, the forum→host map.
fn read(n: &Node, room: &str, group: &str) -> String {
    format!("{:?} | {:?} | {:?}", n.object_view(room), n.group_state(group).map(|s| s.display_name), n.forum_hosts())
}

#[tokio::test]
async fn kept_folds_survive_a_reopen_and_every_change_moves_the_key() {
    kept_on();
    let h = Harness::new(&["ada", "bo"]).await;
    let room = h.mint_object(0, ObjectKind::Forum, "Talk", &[1]).await;
    let group = h.mint_object(0, ObjectKind::Group, "Us", &[1]).await;
    h.node(1).apply(&room, FORUM_POST, post("first")).await.expect("bo posts");
    h.settle().await;

    let first = h.node(0);
    let seen = read(&first, &room, &group);
    assert!(seen.contains("first"), "{seen}");
    assert!(kept_rows(&first) > 0, "the folds are kept on disk");
    drop(first);

    // A new open: a new nonce, an empty memory. What it reads comes from disk: the room's
    // view, the group's state. (FC-2 refolds each disk hit, so misses count its refolds.)
    let again = h.node(0);
    assert_eq!(read(&again, &room, &group), seen, "the reopen reads what was folded");
    let (_, _, kept) = counts(&again);
    assert!(kept >= 2, "the view and the state from disk: {kept}");
    drop(again);

    // The same open with nothing kept reads nothing from disk, and the same.
    let cold = h.node(0);
    cold.dir.conn.execute("DELETE FROM fold_store", []).unwrap();
    assert_eq!(read(&cold, &room, &group), seen, "folded from the log, the same");
    assert_eq!(counts(&cold).2, 0, "nothing from disk");
    drop(cold);

    // A new Delta moves the room's key: it misses on disk too, and folds afresh.
    h.node(1).apply(&room, FORUM_POST, post("second")).await.expect("bo posts again");
    h.settle().await;
    let after = h.node(0);
    let now = read(&after, &room, &group);
    assert!(now.contains("second"), "the moved object folds afresh: {now}");
    let g = hex::decode(&room).unwrap();
    let gen = after.dir.fold_gen(&g).unwrap() as i64;
    let stale: i64 = after.dir.conn.query_row("SELECT count(*) FROM fold_store WHERE group_id = ?1 AND gen < ?2", (&g, gen), |r| r.get(0)).unwrap();
    let current: i64 = after.dir.conn.query_row("SELECT count(*) FROM fold_store WHERE group_id = ?1 AND gen = ?2", (&g, gen), |r| r.get(0)).unwrap();
    assert!(current > 0 && stale == 0, "the room's kept fold is the new one, the old replaced: {current} current, {stale} stale");

    // Another build's model keeps nothing it can use: every object has moved under it.
    let other = pacific_core::fold_cache::model_of("aa  delta-graph.icd.json\n", "another-build");
    assert!(!after.dir.moved_objects(&other).unwrap().is_empty(), "a new model reads nothing kept");
}

#[tokio::test]
async fn refold_moved_keeps_what_a_sync_brought_before_anything_reads_it() {
    kept_on();
    let h = Harness::new(&["ada", "bo"]).await;
    let room = h.mint_object(0, ObjectKind::Forum, "Talk", &[1]).await;
    let group = h.mint_object(0, ObjectKind::Group, "Us", &[1]).await;
    h.node(1).apply(&room, FORUM_POST, post("while you were away")).await.expect("bo posts");
    h.settle().await;

    let model = pacific_core::fold_cache::model().expect("a model");
    let synced = h.node(0);
    // As a device that has read nothing since the sync: the harness's own reads kept some.
    synced.dir.conn.execute("DELETE FROM fold_store", []).unwrap();
    assert!(!synced.dir.moved_objects(&model).unwrap().is_empty(), "the sync left objects to fold");
    assert!(synced.refold_moved().unwrap() > 0, "it folds them");
    assert!(synced.dir.moved_objects(&model).unwrap().is_empty(), "and nothing is left");
    drop(synced);

    // The next open reads the room and the group from disk, folded before it was asked.
    let opened = h.node(0);
    let seen = read(&opened, &room, &group);
    assert!(seen.contains("while you were away"), "{seen}");
    let (_, _, kept) = counts(&opened);
    assert!(kept > 0, "read from what refold_moved kept");
}

#[tokio::test]
async fn generations_that_start_again_take_the_kept_folds_with_them() {
    kept_on();
    let h = Harness::new(&["ada"]).await;
    let room = h.mint_object(0, ObjectKind::Forum, "Talk", &[]).await;
    let n = h.node(0);
    n.object_view(&room).unwrap();
    assert!(kept_rows(&n) > 0);
    n.dir.conn.execute_batch("DROP TABLE fold_gen").unwrap();
    drop(n);

    let reopened = h.node(0);
    assert_eq!(kept_rows(&reopened), 0, "fold_gen begun again, fold_store emptied");
    assert!(reopened.object_view(&room).is_ok(), "and the object folds from its log");
}
