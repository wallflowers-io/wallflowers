//! O-69, FC-5 (A) (mdr/icd-pin.md § Acceptance tests): no folded state is written to
//! `pacific.db`. The seal and a backup, with `fold_gen` set aside, are byte-identical with
//! the cache on and off, and a restore folds the same. A restore that brings back an older
//! `fold_gen` gets no stale hit: the open nonce.
//!
//! The seal's database is `Node::seal_state`'s own copy, opened from the seal. The backup
//! is a copy through a second connection, as K-48's is (`VACUUM INTO` here: this crate's
//! SQLite has no backup API). "Set aside" is the copy with the `fold_gen` table dropped.

mod common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args, FORUM_POST};
use pacific_core::fold_cache::{FoldCache, Key};
use pacific_core::object::ObjectKind;
use pacific_core::{devstate, paths, Node};

const NODE: &str = "door.example";

fn cache_on() {
    std::env::set_var("PACIFIC_FOLD_CACHE_MODEL", "o69-fc5");
    std::env::set_var("PACIFIC_FOLD_CACHE_VERIFY", "1");
}

/// The Door sets an at-rest key before any identity exists (`account.rs`); so does this.
fn at_rest() {
    pacific_core::atrest::set_device_key([0x5a; 32]);
}

fn post(text: &str) -> Args {
    let mut a = Args::new();
    a.insert("text".into(), ArgVal::Text(text.into()));
    a
}

/// The database a seal carries, opened from the seal with the person's key.
fn sealed_db(n: &Node, counter: u64) -> Vec<u8> {
    let blob = n.seal_state(NODE, counter).unwrap();
    let key = devstate::seal_key(&pacific_core::locator::storage_root(&n.id.seed_bytes().unwrap()));
    let (_, contents) = devstate::open(&blob, &n.id.identity_pk(), NODE, &key).unwrap();
    contents.files.iter().find(|(name, _)| name == "pacific.db").expect("the seal carries pacific.db").1.to_vec()
}

/// A backup: a copy of the live database through a second connection.
fn backup_db(scratch: &std::path::Path, name: &str) -> Vec<u8> {
    let to = scratch.join(name);
    let c = rusqlite::Connection::open(paths::db_path()).unwrap();
    c.execute("VACUUM INTO ?1", [to.to_string_lossy().as_ref()]).unwrap();
    std::fs::read(&to).unwrap()
}

/// A database's bytes with `fold_gen` set aside: the table dropped, the file compacted.
fn set_aside(scratch: &std::path::Path, name: &str, db: &[u8]) -> Vec<u8> {
    let (from, to) = (scratch.join(format!("{name}.in")), scratch.join(format!("{name}.out")));
    std::fs::write(&from, db).unwrap();
    let c = rusqlite::Connection::open(&from).unwrap();
    c.execute_batch("DROP TABLE fold_gen").unwrap();
    c.execute("VACUUM INTO ?1", [to.to_string_lossy().as_ref()]).unwrap();
    std::fs::read(&to).unwrap()
}

/// What a reader is given about each object.
fn read_all(n: &Node, objs: &[String]) -> Vec<String> {
    objs.iter().map(|o| format!("{:?} {:?}", n.object_view(o).map_err(|e| e.to_string()), n.object_compliance(o).map_err(|e| e.to_string()))).collect()
}

async fn fixture(h: &Harness) -> Vec<String> {
    let room = h.mint_object(0, ObjectKind::Forum, "Talk", &[1]).await;
    let group = h.mint_object(0, ObjectKind::Group, "Us", &[1]).await;
    let post_obj = h.mint_object(0, ObjectKind::Post, "A post", &[1]).await;
    for t in ["one", "two", "three"] {
        h.node(1).apply(&room, FORUM_POST, post(t)).await.expect("bo posts");
    }
    h.settle().await;
    vec![room, group, post_obj]
}

#[tokio::test]
async fn fc5_the_seal_and_a_backup_hold_no_folded_state_and_a_restore_folds_the_same() {
    at_rest();
    cache_on();
    let h = Harness::new(&["ada", "bo"]).await;
    let objs = fixture(&h).await;
    let scratch = tempfile::tempdir().unwrap();
    let n = h.node(0);

    // Off: a cache that keeps nothing, every read a fold.
    *n.dir.folds.lock().unwrap() = FoldCache::with_limit(0);
    let read_off = read_all(&n, &objs);
    let (seal_off, backup_off) = (sealed_db(&n, 1), backup_db(scratch.path(), "off.db"));

    // On: the cache filled, then every read a hit.
    *n.dir.folds.lock().unwrap() = FoldCache::default();
    read_all(&n, &objs);
    let hits = n.dir.folds.lock().unwrap().hits;
    let read_on = read_all(&n, &objs);
    assert!(n.dir.folds.lock().unwrap().hits > hits, "the second pass was warm");
    assert!(!n.dir.folds.lock().unwrap().is_empty(), "the cache holds folds");
    assert_eq!(read_on, read_off, "the reads, on against off");
    let (seal_on, backup_on) = (sealed_db(&n, 2), backup_db(scratch.path(), "on.db"));

    assert_eq!(
        set_aside(scratch.path(), "seal-on", &seal_on),
        set_aside(scratch.path(), "seal-off", &seal_off),
        "the seal's database, fold_gen set aside, differs with the cache on"
    );
    assert_eq!(
        set_aside(scratch.path(), "backup-on", &backup_on),
        set_aside(scratch.path(), "backup-off", &backup_off),
        "a backup, fold_gen set aside, differs with the cache on"
    );
    println!(
        "FC-5: with fold_gen in place too, the seal's database {} and the backup {}",
        if seal_on == seal_off { "is byte-identical" } else { "differs" },
        if backup_on == backup_off { "is byte-identical" } else { "differs" }
    );

    // The restore: the seal made with the cache on opens into an empty directory and
    // folds what the sealed device folds, cold.
    let blob = n.seal_state(NODE, 3).unwrap();
    let seed = n.id.seed_bytes().unwrap();
    *n.dir.folds.lock().unwrap() = FoldCache::with_limit(0);
    let before = read_all(&n, &objs);
    let restored_dir = tempfile::tempdir().unwrap();
    std::env::set_var("PACIFIC_STATE_DIR", restored_dir.path());
    pacific_core::router::Routes::parse(&h.relay).save().unwrap();
    pacific_core::node::set_default_arc(&h.relay).unwrap();
    let (r, _) = Node::open_restored(&blob, &seed, NODE).unwrap();
    assert_eq!(read_all(&r, &objs), before, "the restore folds the same");
    assert_eq!(read_all(&r, &objs), before, "and the same again, warm");
    drop(r);
    drop(n);
}

/// A restore brings back the database as sealed, its fold_gen older than a cache has
/// seen. Even a cache carried across, at the very generation the restored history reaches
/// by other rows, gets no hit: the restored directory draws its own nonce.
#[tokio::test]
async fn fc5_a_restore_that_brings_back_an_older_fold_gen_gets_no_stale_hit() {
    at_rest();
    cache_on();
    let h = Harness::new(&["ada", "bo"]).await;
    let objs = fixture(&h).await;
    let room = objs[0].clone();
    let g = hex::decode(&room).unwrap();
    let n = h.node(0);
    let blob = n.seal_state(NODE, 1).unwrap();
    let seed = n.id.seed_bytes().unwrap();
    let sealed_gen = n.dir.fold_gen(&g).unwrap();

    // After the seal: bo posts again, and ada's warm view holds it.
    h.node(1).apply(&room, FORUM_POST, post("after the seal")).await.expect("bo posts");
    drop(h.node(0));
    n.sync_once().await.unwrap();
    let seen = n.object_view(&room).unwrap();
    assert!(seen.contains("after the seal"), "{seen}");
    let held: Key = n.dir.fold_key(&g, "forum", "view").unwrap().unwrap();
    assert!(held.gen > sealed_gen);

    // The restore, and ada's cache carried into it.
    let restored_dir = tempfile::tempdir().unwrap();
    std::env::set_var("PACIFIC_STATE_DIR", restored_dir.path());
    pacific_core::router::Routes::parse(&h.relay).save().unwrap();
    pacific_core::node::set_default_arc(&h.relay).unwrap();
    let (r, _) = Node::open_restored(&blob, &seed, NODE).unwrap();
    assert_eq!(r.dir.fold_gen(&g).unwrap(), sealed_gen, "the restore brings back the older generation");
    *r.dir.folds.lock().unwrap() = std::mem::take(&mut *n.dir.folds.lock().unwrap());

    // The restored history reaches the held generation by rows of its own (a count, set
    // here as other writes would raise it), without bo's post.
    r.dir.conn.execute("UPDATE fold_gen SET gen = ?2 WHERE group_id = ?1", (&g, held.gen as i64)).unwrap();
    let key = r.dir.fold_key(&g, "forum", "view").unwrap().unwrap();
    assert_eq!(key.gen, held.gen, "the same generation");
    assert_ne!(key.nonce, held.nonce, "a nonce of its own");

    // Without the nonce this is the hit it would be: the view from after the seal.
    let stale: Option<Result<String, pacific_core::object_store::Refusal>> = r.dir.folds.lock().unwrap().get(&Key { nonce: held.nonce, ..key.clone() });
    assert!(stale.as_ref().is_some_and(|v| v.as_ref().is_ok_and(|v| v.contains("after the seal"))), "the carried entry is there: {stale:?}");

    let hits = r.dir.folds.lock().unwrap().hits;
    let view = r.object_view(&room).unwrap();
    assert_eq!(r.dir.folds.lock().unwrap().hits, hits, "no hit on the restored directory");
    assert!(!view.contains("after the seal"), "the restore reads its own rows, not the carried fold: {view}");
    let cold = {
        let aside = std::mem::replace(&mut *r.dir.folds.lock().unwrap(), FoldCache::with_limit(0));
        let v = r.object_view(&room).unwrap();
        *r.dir.folds.lock().unwrap() = aside;
        v
    };
    assert_eq!(view, cold, "what a fresh fold of the restored rows gives");
    drop(r);
    drop(n);
}
