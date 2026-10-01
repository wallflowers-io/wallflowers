//! O-69, FC-3 (mdr/icd-pin.md § What invalidates it): one test per event, each showing the
//! fold cache's key moves, and the mutation half: each fold_gen trigger dropped in turn on
//! the node's own connection, its event run, the key left where it was, and a warm read
//! that now differs from a fresh fold. The binary runs with `PACIFIC_FOLD_CACHE_VERIFY=1`
//! (FC-2), so a stale hit through the cached path panics, and the mutation half shows that
//! too.
//!
//! Every key is read through `Directory::fold_key`, the one the cache itself uses.

mod common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args, Coordinator, ForumType, FORUM_POST};
use pacific_core::fold_cache::{self, FoldCache, Key};
use pacific_core::object::{ObjectKind, ObjectType};
use pacific_core::object_store::{GroupObjectStore, Refusal};
use pacific_core::Node;

/// Before anything reads the model or the verify switch: both are read once per process.
fn cache_on() {
    std::env::set_var("PACIFIC_FOLD_CACHE_MODEL", "o69-fc3");
    std::env::set_var("PACIFIC_FOLD_CACHE_VERIFY", "1");
}

fn post(text: &str) -> Args {
    let mut a = Args::new();
    a.insert("text".into(), ArgVal::Text(text.into()));
    a
}

fn gid(obj: &str) -> Vec<u8> {
    hex::decode(obj).unwrap()
}

/// The key of the forum's typed fold, as the cache computes it.
fn key(n: &Node, obj: &str) -> Key {
    n.dir.fold_key(&gid(obj), ForumType::KIND.name(), "fold").unwrap().expect("a model is set, so there is a key")
}

/// What a fold accepted, or its refusal: what FC-2 compares a hit and a refold by.
fn digest(f: &Result<Coordinator<ForumType>, pacific_core::CoreError>) -> Result<[u8; 32], String> {
    f.as_ref().map(|c| c.accepted_digest()).map_err(|e| e.to_string())
}

fn fold(n: &Node, obj: &str) -> Result<Coordinator<ForumType>, pacific_core::CoreError> {
    GroupObjectStore::new(&n.dir).folded::<ForumType>(&gid(obj))
}

/// A fresh fold: the cache set aside, one that keeps nothing in its place, then put back.
fn fresh<T>(n: &Node, f: impl FnOnce() -> T) -> T {
    let warm = std::mem::replace(&mut *n.dir.folds.lock().unwrap(), FoldCache::with_limit(0));
    let out = f();
    *n.dir.folds.lock().unwrap() = warm;
    out
}

/// A room of ada's with bo in it and two of bo's posts, and ada's Node, held so its cache
/// lives across the test as a Door session's does.
async fn room() -> (Harness, String, Node) {
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let room = h.mint_object(0, ObjectKind::Forum, "Talk", &[1]).await;
    for t in ["first", "second"] {
        h.node(1).apply(&room, FORUM_POST, post(t)).await.expect("bo posts");
    }
    h.settle().await;
    let n = h.node(0);
    (h, room, n)
}

/// The key moved, and the read after is what a fresh fold gives.
fn moved(n: &Node, obj: &str, before: &Key, why: &str) {
    let after = key(n, obj);
    assert_ne!(&after, before, "{why}: the key did not move");
    assert_eq!(digest(&fold(n, obj)), digest(&fresh(n, || fold(n, obj))), "{why}: the read after is not a fresh fold");
}

#[tokio::test]
async fn fc3_a_delta_authored_here_moves_the_key() {
    cache_on();
    let (h, room, n) = room().await;
    fold(&n, &room).unwrap();
    let k = key(&n, &room);
    n.apply(&room, FORUM_POST, post("ada's")).await.expect("ada posts");
    moved(&n, &room, &k, "a Delta authored here");
    drop(h);
}

#[tokio::test]
async fn fc3_a_delta_ingested_from_the_relay_moves_the_key() {
    cache_on();
    let (h, room, n) = room().await;
    fold(&n, &room).unwrap();
    let k = key(&n, &room);
    h.node(1).apply(&room, FORUM_POST, post("bo's third")).await.expect("bo posts");
    drop(h.node(0)); // the process's state directory back on ada's
    n.sync_once().await.expect("ada drains it");
    moved(&n, &room, &k, "a Delta ingested from the relay");
}

#[tokio::test]
async fn fc3_a_member_added_moves_the_key() {
    cache_on();
    let (h, room, n) = room().await;
    fold(&n, &room).unwrap();
    let k = key(&n, &room);
    h.add_to_forum(0, 2, &room).await;
    drop(h.node(0));
    moved(&n, &room, &k, "a member added");
}

#[tokio::test]
async fn fc3_a_member_removed_moves_the_key() {
    cache_on();
    let (h, room, n) = room().await;
    fold(&n, &room).unwrap();
    let k = key(&n, &room);
    n.group_remove_member(&room, &hex::encode(h.id(1)), None).await.expect("ada removes bo");
    h.settle().await;
    drop(h.node(0));
    moved(&n, &room, &k, "a member removed");
}

/// Both halves of a handover: the owner's own, which authors its record and commits, and a
/// member recording the new owner from the commit, which writes the owner history and no
/// Delta of this lens. That second is the case `fold_digest` (the rows alone) does not see.
#[tokio::test]
async fn fc3_an_owner_handover_moves_the_key_with_or_without_a_delta_of_the_lens() {
    cache_on();
    let (h, room, n) = room().await;
    fold(&n, &room).unwrap();
    let k = key(&n, &room);
    n.group_hand_over(&room, &hex::encode(h.id(1))).await.expect("ada hands the room to bo");
    moved(&n, &room, &k, "an owner handover by the owner");

    let g = gid(&room);
    let store = GroupObjectStore::new(&n.dir);
    let rows_before = n.dir.load_log(&g).unwrap().len();
    let digest_before = store.fold_digest(&g, ObjectKind::Forum).unwrap();
    fold(&n, &room).unwrap();
    let k = key(&n, &room);
    let epoch = n.dir.owner_history(&g).unwrap().iter().map(|(e, _)| *e).max().unwrap_or(0) + 1;
    n.dir.record_owner(&g, epoch, &h.id(2)).unwrap();
    assert_eq!(n.dir.load_log(&g).unwrap().len(), rows_before, "no Delta of the lens was written");
    assert_eq!(store.fold_digest(&g, ObjectKind::Forum).unwrap(), digest_before, "fold_digest does not see it");
    moved(&n, &room, &k, "an owner recorded from a commit, with no Delta of this lens");
}

/// No write path removes or rewrites a stored row (A-10 refuses a bad signature at ingest,
/// before it is stored), so these are direct writes: the triggers cover a path not yet
/// written, as they are meant to.
#[tokio::test]
async fn fc3_a_row_quarantined_or_removed_moves_the_key() {
    cache_on();
    let (_h, room, n) = room().await;
    let g = gid(&room);
    fold(&n, &room).unwrap();
    let k = key(&n, &room);
    let forged = n
        .dir
        .conn
        .execute(
            "UPDATE delta_log SET author_sig = zeroblob(64) WHERE rowid = (SELECT rowid FROM delta_log WHERE group_id = ?1 AND author_sig IS NOT NULL LIMIT 1)",
            [&g],
        )
        .unwrap();
    assert_eq!(forged, 1, "the room holds a signed row");
    moved(&n, &room, &k, "a row whose signature no longer proves its author");
    assert!(fold(&n, &room).is_err(), "and the fold fails, as a refold does");

    let k = key(&n, &room);
    n.dir.conn.execute("DELETE FROM delta_log WHERE rowid = (SELECT max(rowid) FROM delta_log WHERE group_id = ?1)", [&g]).unwrap();
    moved(&n, &room, &k, "a row removed");
}

/// The table's "none needed": the fold reads the epoch only through the rows. An epoch
/// change (a rename's GroupContext commit) rewrites the roster as every commit does, so the
/// key does move; either way the read after is a fresh fold's, never a stale one.
#[tokio::test]
async fn fc3_an_epoch_change_alone_gives_a_fresh_fold_whether_or_not_the_key_moves() {
    cache_on();
    let (h, room, n) = room().await;
    fold(&n, &room).unwrap();
    let k = key(&n, &room);
    let epoch = h.epoch(0, &room);
    h.rename_object(0, &room, "Talk, renamed").await;
    drop(h.node(0));
    assert!(h.epoch(0, &room) > epoch, "the rename moved the epoch");
    let after = key(&n, &room);
    println!("FC-3 epoch change: the key {}", if after != k { "moved (the roster rewritten)" } else { "stayed" });
    assert_eq!(digest(&fold(&n, &room)), digest(&fresh(&n, || fold(&n, &room))), "the read after an epoch change");
}

/// A new build or a new pin is a new model id: the key carries it, and an entry under one
/// model is never found under another.
#[tokio::test]
async fn fc3_a_new_build_or_a_new_pin_moves_the_key() {
    cache_on();
    let (_h, room, n) = room().await;
    let pin = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.sha256"));
    let k = key(&n, &room);
    assert_eq!(Some(k.model), fold_cache::model(), "the key carries this process's model");
    assert_eq!(k.model, fold_cache::model_of(pin, "test:o69-fc3"), "the model is the pin and the build");
    let build = Key { model: fold_cache::model_of(pin, "test:another-build"), ..k.clone() };
    let repin = Key { model: fold_cache::model_of("0000  delta-graph.icd.json\n", "test:o69-fc3"), ..k.clone() };
    assert!(build != k && repin != k && build != repin, "a new build and a new pin are new keys");
    fold(&n, &room).unwrap();
    let mut c = n.dir.folds.lock().unwrap();
    assert!(c.get::<Result<Coordinator<ForumType>, Refusal>>(&k).is_some(), "the control: held under its own model");
    assert!(c.get::<Result<Coordinator<ForumType>, Refusal>>(&build).is_none(), "never under another build's");
    assert!(c.get::<Result<Coordinator<ForumType>, Refusal>>(&repin).is_none(), "never under another pin's");
}

/// A restore opens the directory again: a new nonce, so a new key, and a cache that starts
/// empty.
#[tokio::test]
async fn fc3_a_restore_starts_empty_under_a_new_key() {
    cache_on();
    let (h, room, n) = room().await;
    fold(&n, &room).unwrap();
    let k = key(&n, &room);
    let again = h.node(0);
    let k2 = key(&again, &room);
    assert_ne!(k2.nonce, k.nonce, "every open draws its own nonce");
    assert_ne!(k2, k);
    let c = again.dir.folds.lock().unwrap();
    assert_eq!((c.len(), c.hits), (0, 0), "an opened directory's cache starts empty");
}

// ─── the mutation half ────────────────────────────────────────────────────────

/// A panic hook that keeps FC-2's expected panics out of the output, and passes on the rest.
fn quiet_fc2() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let said = info.payload().downcast_ref::<String>().map(String::as_str).unwrap_or("");
        if !said.contains("the fold cache differs from a refold") {
            prev(info);
        }
    }));
}

/// The warm read through the cached path: under FC-2 a stale hit panics. Its words, if so.
fn fc2(n: &Node, obj: &str) -> Option<String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fold(n, obj)))
        .err()
        .map(|p| p.downcast_ref::<String>().cloned().unwrap_or_default())
}

struct Outcome {
    trigger: &'static str,
    key_stayed: bool,
    stale: bool,
    fc2: bool,
}

/// One trigger dropped: warm the cache, drop it, run `event`, and measure; then the
/// trigger back (every open creates the missing ones), `undo`, and a cold cache.
fn mutate(n: &Node, obj: &str, trigger: &'static str, event: &str, undo: &str) -> Outcome {
    let g = gid(obj);
    *n.dir.folds.lock().unwrap() = FoldCache::default();
    fold(n, obj).unwrap();
    let k = key(n, obj);
    n.dir.conn.execute_batch(&format!("DROP TRIGGER {trigger}")).unwrap();
    let changed = n.dir.conn.execute(event, [&g]).unwrap();
    assert_eq!(changed, 1, "{trigger}: the event changed one row");
    let key_stayed = key(n, obj) == k;
    let warm = n.dir.folds.lock().unwrap().get::<Result<Coordinator<ForumType>, Refusal>>(&k).map(|w| w.map_err(Refusal::into_error));
    let stale = warm.as_ref().is_some_and(|w| digest(w) != digest(&fresh(n, || fold(n, obj))));
    let fc2 = key_stayed && fc2(n, obj).is_some_and(|w| w.contains(&hex::encode(&g)));
    drop(pacific_core::directory::Directory::open_at(&pacific_core::paths::db_path()).unwrap());
    let back: i64 = n.dir.conn.query_row("SELECT count(*) FROM sqlite_master WHERE type = 'trigger' AND name = ?1", [trigger], |r| r.get(0)).unwrap();
    assert_eq!(back, 1, "{trigger} is back");
    n.dir.conn.execute(undo, [&g]).unwrap();
    *n.dir.folds.lock().unwrap() = FoldCache::default();
    Outcome { trigger, key_stayed, stale, fc2 }
}

/// Each trigger, dropped in turn, under a one-row write of what it watches: the key stays,
/// a warm read is stale, and FC-2 turns red. Then each production write path with one
/// trigger dropped: where another trigger in the same transaction still moves the key, that
/// is printed as a finding, not hidden.
#[tokio::test]
async fn fc3_each_trigger_dropped_in_turn_leaves_a_stale_hit() {
    cache_on();
    quiet_fc2();
    let (h, room, n) = room().await;
    let g = gid(&room);
    let (ada, bo, cy) = (h.id(0), h.id(1), h.id(2));
    let x = |b: &[u8; 32]| hex::encode(b);
    // Two owners on record (ada at 0, bo later), and one row put aside, for the events that
    // need them; each change here moves the generation with every trigger in place.
    n.dir.record_owner(&g, 900, &bo).unwrap();
    n.dir.conn.execute_batch("CREATE TEMP TABLE aside AS SELECT * FROM delta_log WHERE 0").unwrap();
    let top = "(SELECT max(rowid) FROM delta_log WHERE group_id = ?1)";
    // A Delta in, as an ingest writes one: ada's own post, authored with every trigger in
    // place, its row set aside and taken out, to go back in with the trigger dropped.
    n.apply(&room, FORUM_POST, post("set aside")).await.expect("ada posts");
    n.dir.conn.execute(&format!("INSERT INTO aside SELECT * FROM delta_log WHERE rowid = {top}"), [&g]).unwrap();
    n.dir.conn.execute(&format!("DELETE FROM delta_log WHERE rowid = {top}"), [&g]).unwrap();
    let sig: Vec<u8> = n.dir.conn.query_row(&format!("SELECT author_sig FROM delta_log WHERE rowid = {top}"), [&g], |r| r.get(0)).unwrap();

    let out = vec![
        mutate(
            &n,
            &room,
            "fold_gen_log_ins",
            "INSERT INTO delta_log SELECT * FROM aside WHERE group_id = ?1",
            "DELETE FROM delta_log WHERE group_id = ?1 AND delta_id IN (SELECT delta_id FROM aside)",
        ),
        mutate(
            &n,
            &room,
            "fold_gen_log_upd",
            &format!("UPDATE delta_log SET author_sig = zeroblob(64) WHERE rowid = {top}"),
            &format!("UPDATE delta_log SET author_sig = x'{}' WHERE rowid = {top}", hex::encode(&sig)),
        ),
        {
            n.dir.conn.execute_batch("DELETE FROM aside").unwrap();
            n.dir.conn.execute(&format!("INSERT INTO aside SELECT * FROM delta_log WHERE rowid = {top}"), [&g]).unwrap();
            mutate(&n, &room, "fold_gen_log_del", &format!("DELETE FROM delta_log WHERE rowid = {top}"), "INSERT INTO delta_log SELECT * FROM aside WHERE group_id = ?1")
        },
        mutate(
            &n,
            &room,
            "fold_gen_members_ins",
            &format!("INSERT INTO group_members (group_id, member_pk) VALUES (?1, x'{}')", x(&cy)),
            &format!("DELETE FROM group_members WHERE group_id = ?1 AND member_pk = x'{}'", x(&cy)),
        ),
        mutate(
            &n,
            &room,
            "fold_gen_members_upd",
            &format!("UPDATE group_members SET member_pk = x'{}' WHERE group_id = ?1 AND member_pk = x'{}'", x(&cy), x(&bo)),
            &format!("UPDATE group_members SET member_pk = x'{}' WHERE group_id = ?1 AND member_pk = x'{}'", x(&bo), x(&cy)),
        ),
        mutate(
            &n,
            &room,
            "fold_gen_members_del",
            &format!("DELETE FROM group_members WHERE group_id = ?1 AND member_pk = x'{}'", x(&bo)),
            &format!("INSERT INTO group_members (group_id, member_pk) VALUES (?1, x'{}')", x(&bo)),
        ),
        mutate(
            &n,
            &room,
            "fold_gen_owners_ins",
            &format!("INSERT INTO group_owner_history (group_id, from_epoch, owner_pk) VALUES (?1, 901, x'{}')", x(&cy)),
            "DELETE FROM group_owner_history WHERE group_id = ?1 AND from_epoch = 901",
        ),
        mutate(
            &n,
            &room,
            "fold_gen_owners_upd",
            &format!("UPDATE group_owner_history SET owner_pk = x'{}' WHERE group_id = ?1 AND from_epoch = 900", x(&cy)),
            &format!("UPDATE group_owner_history SET owner_pk = x'{}' WHERE group_id = ?1 AND from_epoch = 900", x(&bo)),
        ),
        mutate(
            &n,
            &room,
            "fold_gen_owners_del",
            "DELETE FROM group_owner_history WHERE group_id = ?1 AND from_epoch = 900",
            &format!("INSERT INTO group_owner_history (group_id, from_epoch, owner_pk) VALUES (?1, 900, x'{}')", x(&bo)),
        ),
        {
            // `owner_pk` is what the fold reads when the object has no owner history.
            n.dir.conn.execute_batch("CREATE TEMP TABLE owners_aside AS SELECT * FROM group_owner_history WHERE 0").unwrap();
            n.dir.conn.execute("INSERT INTO owners_aside SELECT * FROM group_owner_history WHERE group_id = ?1", [&g]).unwrap();
            n.dir.conn.execute("DELETE FROM group_owner_history WHERE group_id = ?1", [&g]).unwrap();
            let o = mutate(
                &n,
                &room,
                "fold_gen_owner_upd",
                &format!("UPDATE groups SET owner_pk = x'{}' WHERE group_id = ?1", x(&cy)),
                &format!("UPDATE groups SET owner_pk = x'{}' WHERE group_id = ?1", x(&bo)),
            );
            n.dir.conn.execute("INSERT INTO group_owner_history SELECT * FROM owners_aside WHERE group_id = ?1", [&g]).unwrap();
            o
        },
    ];
    println!("FC-3 mutation, one-row writes: trigger | key stayed | warm read stale | FC-2 red");
    for o in &out {
        println!("  {} | {} | {} | {}", o.trigger, o.key_stayed, o.stale, o.fc2);
    }
    for o in &out {
        assert!(o.key_stayed && o.stale && o.fc2, "{} dropped: the key stayed {}, a warm read stale {}, FC-2 red {}", o.trigger, o.key_stayed, o.stale, o.fc2);
    }

    // The production write paths, one trigger dropped at a time: which still move the key.
    println!("FC-3 mutation, production writes with one trigger dropped:");
    let paths: [(&str, &str); 5] = [
        ("fold_gen_members_ins", "set_group_members (DELETE all, INSERT each)"),
        ("fold_gen_members_del", "set_group_members (DELETE all, INSERT each)"),
        ("fold_gen_owners_ins", "record_owner (INSERT OR REPLACE history, UPDATE owner_pk)"),
        ("fold_gen_owner_upd", "record_owner (INSERT OR REPLACE history, UPDATE owner_pk)"),
        ("fold_gen_log_ins", "append_delta (INSERT OR IGNORE)"),
    ];
    let mut findings = vec![];
    for (trigger, path) in paths {
        *n.dir.folds.lock().unwrap() = FoldCache::default();
        fold(&n, &room).unwrap();
        let k = key(&n, &room);
        n.dir.conn.execute_batch(&format!("DROP TRIGGER {trigger}")).unwrap();
        match trigger {
            "fold_gen_members_ins" => n.dir.set_group_members(&g, &[ada, bo, cy]).unwrap(),
            "fold_gen_members_del" => n.dir.set_group_members(&g, &[ada]).unwrap(),
            "fold_gen_owners_ins" | "fold_gen_owner_upd" => n.dir.record_owner(&g, 950, &cy).unwrap(),
            _ => {
                n.dir.append_delta(&g, &[0x77; 32], &ada, b"not a delta", 0).unwrap();
            }
        }
        let still_moves = key(&n, &room) != k;
        println!("  {trigger} dropped, {path}: the key {}", if still_moves { "STILL MOVES (another trigger in the write covers it)" } else { "stays" });
        if still_moves {
            findings.push(trigger);
        }
        drop(pacific_core::directory::Directory::open_at(&pacific_core::paths::db_path()).unwrap());
        match trigger {
            "fold_gen_members_ins" | "fold_gen_members_del" => n.dir.set_group_members(&g, &[ada, bo]).unwrap(),
            "fold_gen_owners_ins" | "fold_gen_owner_upd" => {
                n.dir.conn.execute("DELETE FROM group_owner_history WHERE group_id = ?1 AND from_epoch = 950", [&g]).unwrap();
                n.dir.record_owner(&g, 900, &bo).unwrap();
            }
            _ => {
                n.dir.conn.execute("DELETE FROM delta_log WHERE group_id = ?1 AND delta_id = ?2", (&g, &[0x77u8; 32][..])).unwrap();
            }
        }
    }
    println!("FC-3 findings, triggers that stay green on their production path when dropped alone: {findings:?}");
    // What the code shows, stated rather than hidden: each roster write fires both member
    // triggers, and each owner record fires the history's and owner_pk's.
    assert_eq!(findings, vec!["fold_gen_members_ins", "fold_gen_members_del", "fold_gen_owners_ins", "fold_gen_owner_upd"]);
    drop(h);
}
