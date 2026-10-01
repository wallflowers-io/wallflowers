//! O-69, FC-1 (mdr/icd-pin.md § Acceptance tests): the fold cache on against off, over a
//! scripted and a randomised sequence of every event in § What invalidates it, for every
//! kind: equal state or equal failure at every step.
//!
//! "Off" is the same Node with its cache set aside for one that keeps nothing
//! (`FoldCache::with_limit(0)`), so both answers come from one directory, one model and
//! one fold implementation: the cache is the only difference. "On" is read twice, the
//! second read warm. The binary runs with `PACIFIC_FOLD_CACHE_VERIFY=1` (FC-2) besides.
//!
//! Every kind: those a draft mints; Field, System and Topic through `object_new` (no draft
//! mints them); Contact and Conversation from a pairing. The randomised sequence prints its
//! seed; `FC1_SEED` replays one, `FC1_STEPS` sets its length.

mod common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args, FORUM_POST};
use pacific_core::fold_cache::FoldCache;
use pacific_core::object::ObjectKind;
use pacific_core::Node;

fn cache_on() {
    std::env::set_var("PACIFIC_FOLD_CACHE_MODEL", "o69-fc1");
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

/// The process's state directory back on ada's, for the Node held across the test.
fn at_ada(h: &Harness) {
    drop(h.node(0));
}

/// Everything a reader is given about one object, through the cached paths: its view, its
/// compliance, and its chat thread (a refusal, for a kind with no chat lens).
fn products(n: &Node, obj: &str) -> String {
    let view = n.object_view(obj).map_err(|e| e.to_string());
    let compliance = n.object_compliance(obj).map_err(|e| e.to_string());
    let thread = n.forum_thread(&gid(obj)).map(|t| format!("{t:?}")).map_err(|e| e.to_string());
    format!("view {view:?}\ncompliance {compliance:?}\nthread {thread:?}")
}

fn noncompliant(n: &Node) -> Vec<String> {
    let mut v: Vec<String> = n.noncompliant_objects().unwrap().into_iter().map(|x| format!("{x:?}")).collect();
    v.sort();
    v
}

/// One step's check: every object, on (cold, then warm) against off, and the sweep.
fn check(n: &Node, objs: &[(String, String)], step: &str) {
    let on: Vec<String> = objs.iter().map(|(o, _)| products(n, o)).collect();
    let hits = n.dir.folds.lock().unwrap().hits;
    let warm: Vec<String> = objs.iter().map(|(o, _)| products(n, o)).collect();
    let warmed = n.dir.folds.lock().unwrap().hits - hits;
    let swept = noncompliant(n);
    let aside = std::mem::replace(&mut *n.dir.folds.lock().unwrap(), FoldCache::with_limit(0));
    let off: Vec<String> = objs.iter().map(|(o, _)| products(n, o)).collect();
    let swept_off = noncompliant(n);
    *n.dir.folds.lock().unwrap() = aside;
    for (i, (o, k)) in objs.iter().enumerate() {
        assert_eq!(on[i], off[i], "{step}: {k} {o}: on (a first read) against off");
        assert_eq!(warm[i], off[i], "{step}: {k} {o}: on (warm) against off");
    }
    assert_eq!(swept, swept_off, "{step}: noncompliant_objects, on against off");
    assert!(warmed > 0, "{step}: the second pass hit nothing, so it was not warm");
}

/// One object of every kind, bo a member of each ada's group-typed objects. Returns
/// `(object, kind)` for each, and says which kinds it could not make, and why.
async fn every_kind(h: &Harness) -> Vec<(String, String)> {
    let mut objs = vec![];
    for &k in ObjectKind::ALL {
        let mut draft = pacific_core::mint::MintDraft { name: format!("{k:?}"), ..Default::default() };
        if k == ObjectKind::Event {
            draft.start_ms = 1_790_000_000_000;
        }
        match h.node(0).mint(k, &draft).await {
            Ok(obj) => {
                h.add_to_forum(0, 1, &obj).await;
                objs.push((obj, k.name().to_string()));
            }
            Err(minted) => {
                // No draft mints it: the primitive, as the product makes one of these.
                match h.node(0).object_new(k.name(), &format!("{k:?}")) {
                    Ok(obj) => {
                        h.add_to_forum(0, 1, &obj).await;
                        objs.push((obj, k.name().to_string()));
                    }
                    Err(e) => println!("FC-1: no {k:?} by mint ({minted}) or object_new ({e}); from a pairing below, if it is one"),
                }
            }
        }
    }
    h.pair(0, 1).await;
    h.dm_post(1, 0, "hello over the pairing").await;
    h.settle().await;
    let named = h.node(0).objects_named().unwrap();
    for (id, kind, _) in named {
        if !objs.iter().any(|(o, _)| *o == id) {
            objs.push((id, kind));
        }
    }
    let kinds: std::collections::BTreeSet<&str> = objs.iter().map(|(_, k)| k.as_str()).collect();
    println!("FC-1: {} objects, kinds {kinds:?}", objs.len());
    for &k in ObjectKind::ALL {
        if !kinds.contains(k.name()) {
            println!("FC-1: kind {:?} has no object here", k);
        }
    }
    objs
}

/// Forge a row's signature, or put it back: a quarantine's rows, written directly (no write
/// path rewrites a stored row; A-10 refuses at ingest).
fn forge(n: &Node, obj: &str) -> Option<Vec<u8>> {
    let g = gid(obj);
    let sig: Option<Vec<u8>> = n
        .dir
        .conn
        .query_row("SELECT author_sig FROM delta_log WHERE group_id = ?1 AND author_sig IS NOT NULL ORDER BY rowid LIMIT 1", [&g], |r| r.get(0))
        .ok();
    if sig.is_some() {
        n.dir
            .conn
            .execute("UPDATE delta_log SET author_sig = zeroblob(64) WHERE rowid = (SELECT rowid FROM delta_log WHERE group_id = ?1 AND author_sig IS NOT NULL ORDER BY rowid LIMIT 1)", [&g])
            .unwrap();
    }
    sig
}

fn unforge(n: &Node, obj: &str, sig: &[u8]) {
    n.dir
        .conn
        .execute("UPDATE delta_log SET author_sig = ?2 WHERE rowid = (SELECT rowid FROM delta_log WHERE group_id = ?1 AND author_sig = zeroblob(64) ORDER BY rowid LIMIT 1)", (&gid(obj), sig))
        .unwrap();
}

fn drop_row(n: &Node, obj: &str) -> usize {
    n.dir.conn.execute("DELETE FROM delta_log WHERE rowid = (SELECT max(rowid) FROM delta_log WHERE group_id = ?1)", [&gid(obj)]).unwrap()
}

fn record_owner(n: &Node, obj: &str, who: &[u8; 32]) {
    let g = gid(obj);
    let epoch = n.dir.owner_history(&g).unwrap().iter().map(|(e, _)| *e).max().unwrap_or(0) + 1;
    n.dir.record_owner(&g, epoch, who).unwrap();
}

fn parent_args(parent: &str) -> Args {
    pacific_core::parent::set_parent_args(parent, "part", 1)
}

fn group_typed(objs: &[(String, String)]) -> Vec<String> {
    objs.iter().filter(|(_, k)| k != "contact" && k != "conversation" && k != "connection").map(|(o, _)| o.clone()).collect()
}

#[tokio::test]
async fn fc1_scripted_every_event_every_kind() {
    cache_on();
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let objs = every_kind(&h).await;
    let groups = group_typed(&objs);
    let forum = objs.iter().find(|(_, k)| k == "forum").map(|(o, _)| o.clone()).expect("a forum");
    let (bo, cy) = (h.id(1), h.id(2));
    at_ada(&h);
    let mut n = h.node(0);
    check(&n, &objs, "the start");

    // Each event must land on at least one object, or it was not covered.
    let mut landed = std::collections::BTreeMap::<&str, usize>::new();
    let mut tally = |what: &'static str, ok: bool| *landed.entry(what).or_default() += ok as usize;
    for o in &groups {
        let parent = if *o == forum { &groups[0] } else { &forum };
        let r = n.apply(o, pacific_core::parent::OP_SET_PARENT, parent_args(parent)).await;
        if let Err(e) = &r {
            println!("FC-1: setParent on {o}: {e}");
        }
        tally("a Delta authored here", r.is_ok());
    }
    check(&n, &objs, "a Delta authored here, on every object");

    h.node(1).apply(&forum, FORUM_POST, post("bo, from the relay")).await.expect("bo posts");
    h.dm_post(1, 0, "bo, over the pairing").await;
    at_ada(&h);
    n.sync_once().await.expect("ada drains");
    tally("a Delta ingested", true);
    check(&n, &objs, "Deltas ingested from the relay");

    for o in &groups {
        h.add_to_forum(0, 2, o).await;
        tally("a member added", true);
    }
    at_ada(&h);
    check(&n, &objs, "a member added to every object");

    for o in &groups {
        let r = n.group_remove_member(o, &hex::encode(cy), None).await;
        if let Err(e) = &r {
            println!("FC-1: removing cy from {o}: {e}");
        }
        tally("a member removed", r.is_ok());
    }
    h.settle().await;
    at_ada(&h);
    check(&n, &objs, "a member removed from every object");

    for o in &groups {
        let r = h.try_rename(0, o, "renamed").await;
        if let Err(e) = &r {
            println!("FC-1: renaming {o}: {e}");
        }
        tally("an epoch change", r.is_ok());
    }
    h.settle().await;
    at_ada(&h);
    check(&n, &objs, "an epoch change on every object");

    for o in &groups {
        let r = n.group_hand_over(o, &hex::encode(bo)).await;
        if let Err(e) = &r {
            println!("FC-1: handing {o} to bo: {e}");
        }
        tally("an owner handover", r.is_ok());
    }
    h.settle().await;
    at_ada(&h);
    check(&n, &objs, "an owner handover on every object");
    for o in &groups {
        record_owner(&n, o, &cy);
        tally("an owner recorded, no Delta", true);
    }
    check(&n, &objs, "an owner recorded on every object, with no Delta of its lens");

    for (o, _) in &objs {
        if let Some(sig) = forge(&n, o) {
            tally("a row quarantined", true);
            check(&n, &objs, &format!("a row of {o} quarantined (its signature forged)"));
            unforge(&n, o, &sig);
        }
    }
    check(&n, &objs, "every forged row put back");
    for (o, _) in &objs {
        tally("a row removed", drop_row(&n, o) == 1);
    }
    check(&n, &objs, "a row removed from every object");

    // A new build or a new pin is a new process: a new model id (FC-3's), and a cache that
    // starts empty, which is what this puts in place.
    *n.dir.folds.lock().unwrap() = FoldCache::default();
    tally("a new build", true);
    check(&n, &objs, "a new build: the cache empty");
    n = h.node(0);
    tally("a restore", true);
    check(&n, &objs, "a restore: the directory opened again");
    drop(n);
    println!("FC-1 scripted, objects each event landed on: {landed:?}");
    for (what, count) in &landed {
        assert!(*count > 0, "{what} landed on no object: not covered");
    }
    assert_eq!(landed.len(), 11, "every event of the table was run: {landed:?}");
}

/// xorshift64*: a seed in, a sequence out, the same every time.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn pick(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[tokio::test]
async fn fc1_randomised_every_event_every_kind() {
    cache_on();
    let seed = std::env::var("FC1_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or_else(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64 | 1
    });
    let steps: usize = std::env::var("FC1_STEPS").ok().and_then(|s| s.parse().ok()).unwrap_or(40);
    println!("FC-1 randomised: seed {seed}, {steps} steps (FC1_SEED={seed} replays it)");
    let mut rng = Rng(seed);
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let objs = every_kind(&h).await;
    let groups = group_typed(&objs);
    let forum = objs.iter().find(|(_, k)| k == "forum").map(|(o, _)| o.clone()).expect("a forum");
    let people = [h.id(1), h.id(2)];
    at_ada(&h);
    let mut n = h.node(0);
    let mut forged: Vec<(String, Vec<u8>)> = vec![];
    let mut seen = std::collections::BTreeSet::new();
    check(&n, &objs, "the start");
    for step in 0..steps {
        let o = groups[rng.pick(groups.len())].clone();
        let who = people[rng.pick(2)];
        let event = rng.pick(11);
        let what = match event {
            0 => {
                let p = &groups[rng.pick(groups.len())];
                let r = n.apply(&o, pacific_core::parent::OP_SET_PARENT, parent_args(p)).await;
                format!("ada authors setParent on {o}: {}", if r.is_ok() { "ok" } else { "refused" })
            }
            1 => {
                let r = h.node(1).apply(&forum, FORUM_POST, post(&format!("bo, step {step}"))).await;
                at_ada(&h);
                let _ = n.sync_once().await;
                format!("bo posts, ada drains: {}", if r.is_ok() { "ok" } else { "refused" })
            }
            2 => {
                let u = if who == people[0] { 1 } else { 2 };
                let bundle = h.node(u).build_contact_bundle().unwrap();
                at_ada(&h);
                let r = n.group_add_member(&o, &bundle).await;
                let _ = h.node(u).sync_once().await;
                h.settle().await;
                at_ada(&h);
                format!("a member added to {o}: {}", if r.is_ok() { "ok" } else { "refused" })
            }
            3 => {
                let r = n.group_remove_member(&o, &hex::encode(who), None).await;
                h.settle().await;
                at_ada(&h);
                format!("a member removed from {o}: {}", if r.is_ok() { "ok" } else { "refused" })
            }
            4 => {
                let r = h.try_rename(0, &o, &format!("step {step}")).await;
                h.settle().await;
                at_ada(&h);
                format!("an epoch change on {o}: {}", if r.is_ok() { "ok" } else { "refused" })
            }
            5 => {
                let r = n.group_hand_over(&o, &hex::encode(who)).await;
                h.settle().await;
                at_ada(&h);
                format!("an owner handover of {o}: {}", if r.is_ok() { "ok" } else { "refused" })
            }
            6 => {
                record_owner(&n, &o, &who);
                format!("an owner recorded on {o}, no Delta")
            }
            7 => match forged.pop() {
                Some((f, sig)) if rng.pick(2) == 0 => {
                    unforge(&n, &f, &sig);
                    format!("a forged row of {f} put back")
                }
                back => {
                    forged.extend(back);
                    match forge(&n, &o) {
                        Some(sig) => {
                            forged.push((o.clone(), sig));
                            format!("a row of {o} quarantined")
                        }
                        None => format!("{o} has no signed row to forge"),
                    }
                }
            },
            8 => format!("a row removed from {o}: {}", drop_row(&n, &o)),
            9 => {
                *n.dir.folds.lock().unwrap() = FoldCache::default();
                "a new build: the cache empty".to_string()
            }
            _ => {
                n = h.node(0);
                "a restore: the directory opened again".to_string()
            }
        };
        seen.insert(event);
        check(&n, &objs, &format!("seed {seed}, step {step}: {what}"));
    }
    println!("FC-1 randomised: seed {seed}, events drawn {seen:?} of 0..=10");
}
