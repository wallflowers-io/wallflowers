//! RESUME PROFILE: what a sign-in costs (core/docs/launch/mdr/margins.md G-7; DQ-3).
//!
//! Measurement, not a test. One person's graph is built through the real write path on
//! the test harness's in-process relay, and the two ways a sign-in can go (D-34) are
//! timed against it:
//!
//!   restore  a new Node from the wrap (`Node::restore_from_wrap`, the Door's path), then
//!            `resume_at` with the head: it joins each object through its pool leaf and
//!            climbs every epoch added since that leaf was provisioned.
//!   return   the person's own device, away while those epochs were added, put back on
//!            the relay and synced: D-34 (c)'s ordinary climb from the last session.
//!
//! Then a second restore before any device of the person syncs again: DQ-1's case, where
//! the pool has no idle leaf left.
//!
//!   cargo run --release -p pacific-core --example resume_profile -- <N> <M> <D> [L] [P]
//!
//! N objects the person is in; M member joins in each after the pool leaf is provisioned,
//! each one Add and so one epoch; D posts in each after them. L pads each joiner's name to
//! L characters; P names each object with P bytes, which rides in its GroupContext. Each
//! Add is first staged on a copy of the owner's group and dropped, to weigh its commit and
//! its Welcome apart; the copy is never written, so the real Add that follows is unchanged. The returning device is
//! away for the same M and D. Prints one JSON line. State lives in temp dirs; nothing
//! leaves 127.0.0.1.

#[path = "../tests/common/mod.rs"]
#[allow(dead_code)]
mod common;

use common::Harness;
use pacific_core::resumption::{HeadInput, Outcome};
use std::time::Instant;

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

/// This process's CPU time so far, from `ps`: no dependency beyond the crate's own.
fn cpu_s() -> f64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "time=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps");
    let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let (days, t) = match t.split_once('-') {
        Some((d, rest)) => (d.parse::<f64>().unwrap_or(0.0), rest.to_string()),
        None => (0.0, t),
    };
    days * 86400.0 + t.split(':').fold(0.0, |acc, p| acc * 60.0 + p.parse::<f64>().unwrap_or(0.0))
}

/// The commit and the Welcome an Add of `bundle` to `obj` would publish, in bytes: staged on
/// the owner's group as loaded from its store, then dropped unwritten.
fn add_bytes(h: &Harness, owner: usize, obj: &str, bundle: &str) -> (usize, usize) {
    let node = h.node(owner);
    let (sk, pk) = node.dir.mls_signing_keypair().unwrap();
    let sid = pacific_core::mls::signing_identity(&node.id.identity_pk(), &pk);
    let client = pacific_core::mls::build_client_sqlite(
        &pacific_core::paths::db_path(),
        sid,
        pacific_core::mls::SecretKey::new(sk),
    )
    .unwrap();
    let mut group = pacific_core::mls::load_group(&client, &hex::decode(obj).unwrap()).unwrap();
    let kp = pacific_core::handshake::parse_and_verify(bundle).unwrap().key_package;
    let (commit, welcome) = pacific_core::mls::stage_add_member(&mut group, &kp).unwrap();
    (commit.len(), welcome.len())
}

fn outcome(o: &Outcome) -> String {
    match o {
        Outcome::Joined { from_epoch } => format!("Joined from epoch {from_epoch}"),
        other => format!("{other:?}"),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let a: Vec<usize> = std::env::args().skip(1).map(|s| s.parse().expect("N M D, integers")).collect();
    let (n, m, d) = (a.first().copied().unwrap_or(1), a.get(1).copied().unwrap_or(10), a.get(2).copied().unwrap_or(10));
    let (l, p) = (a.get(3).copied().unwrap_or(0), a.get(4).copied().unwrap_or(0));

    let mut names = vec!["ada".to_string(), "owner".to_string()];
    names.extend((0..m).map(|i| {
        let base = format!("j{i}");
        if l > base.len() { format!("{base}{}", "x".repeat(l - base.len())) } else { base }
    }));
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut h = Harness::new(&refs).await;
    let (ada, owner) = (0usize, 1usize);

    // The person is in N objects; the first settle provisions each one's pool leaf.
    let t_build = Instant::now();
    let mut objs = Vec::new();
    for _ in 0..n {
        let o = h.form_forum(owner, &[ada]).await;
        if p > 0 {
            h.try_rename(owner, &o, &"n".repeat(p)).await.unwrap();
        }
        objs.push(o);
    }
    h.settle().await;
    let epoch_at_leaf: Vec<u64> = objs.iter().map(|o| h.epoch(owner, o)).collect();

    // The head and the wrap, as the auth service holds them; then the device goes away.
    let prf = [7u8; 32];
    let host = "arc.test";
    let head = h.with_sync(ada, |n| n.head_update().unwrap());
    let wrap = h.with_sync(ada, |n| n.export_wrap(&prf, host).unwrap());
    h.partition(ada);

    // M joins and D posts in each object, while the person is away. An Add the relay
    // refuses ends that object's joins, and is reported: where a group stops growing is
    // itself a result.
    let mut refused: Vec<serde_json::Value> = Vec::new();
    let mut sizes: Vec<serde_json::Value> = Vec::new();
    for o in &objs {
        let mut last = serde_json::Value::Null;
        for j in 0..m {
            let joiner = 2 + j;
            let bundle = h.node(joiner).build_contact_bundle().unwrap();
            let (cb, wb) = add_bytes(&h, owner, o, &bundle);
            let at = serde_json::json!({ "join": j + 1, "commit": cb, "welcome": wb });
            if let Err(e) = h.node(owner).group_add_member(o, &bundle).await {
                let members = h.roster(owner, o).len();
                refused.push(serde_json::json!({ "join": j + 1, "members": members, "commit": cb, "welcome": wb, "error": e.to_string() }));
                last = serde_json::Value::Null;
                break;
            }
            if (j + 1) % 100 == 0 || j == 0 { sizes.push(at.clone()); }
            last = at;
        }
        if !last.is_null() { sizes.push(last); }
        for i in 0..d {
            h.obj_post(owner, o, &format!("post {i}")).await;
        }
    }
    let epoch_at_head: Vec<u64> = objs.iter().map(|o| h.epoch(owner, o)).collect();
    let build_s = t_build.elapsed().as_secs_f64();

    // Restore: a new Node from the wrap, then resume with the head.
    let input = HeadInput { blob: head.sealed.clone(), declared_position: head.position };
    let (c0, t0) = (cpu_s(), Instant::now());
    let door = h.add_user_from_wrap("ada-door", &prf, &wrap, host);
    let r = h.with(door, |n| async move { n.resume_at(Some(input.clone()), now()).await.unwrap() }).await;
    let (restore_wall, restore_cpu) = (t0.elapsed().as_secs_f64(), cpu_s() - c0);
    let restore_outcomes: Vec<String> = r.objects.iter().map(|o| format!("{}: {}", o.kind, outcome(&o.outcome))).collect();
    let restore_epochs: Vec<u64> = objs.iter().map(|o| h.epoch(door, o)).collect();
    let restore_posts: Vec<usize> = objs.iter().map(|o| h.obj_view(door, o).len()).collect();
    let restore_noncompliant = h.with_sync(door, |n| n.noncompliant_objects().unwrap().len());

    // A second restore before the person's devices sync again: the pool's idle leaf is taken.
    let input2 = HeadInput { blob: head.sealed.clone(), declared_position: head.position };
    let door2 = h.add_user_from_wrap("ada-door-2", &prf, &wrap, host);
    let r2 = h.with(door2, |n| async move { n.resume_at(Some(input2), now()).await.unwrap() }).await;
    let second_outcomes: Vec<String> = r2.objects.iter().map(|o| format!("{}: {}", o.kind, outcome(&o.outcome))).collect();

    // Return: the person's own device, back on the relay, syncing until every object is at
    // or past the head. Past, because its own upkeep provisions a new pool leaf on the way,
    // and that Add is an epoch of its own.
    h.rejoin(ada);
    let (c1, t1) = (cpu_s(), Instant::now());
    h.sync(ada).await;
    let first_sync_s = t1.elapsed().as_secs_f64();
    let mut syncs = 1;
    while syncs < 20 && !objs.iter().zip(&epoch_at_head).all(|(o, &top)| h.epoch(ada, o) >= top) {
        h.sync(ada).await;
        syncs += 1;
    }
    let (return_wall, return_cpu) = (t1.elapsed().as_secs_f64(), cpu_s() - c1);
    let return_epochs: Vec<u64> = objs.iter().map(|o| h.epoch(ada, o)).collect();
    let return_posts: Vec<usize> = objs.iter().map(|o| h.obj_view(ada, o).len()).collect();

    let out = serde_json::json!({
        "n": n, "m": m, "d": d, "l": l, "p": p, "add_bytes": sizes,
        "epoch_at_leaf": epoch_at_leaf, "epoch_at_head": epoch_at_head, "build_s": build_s, "refused": refused,
        "restore": {
            "wall_s": restore_wall, "cpu_s": restore_cpu, "outcomes": restore_outcomes,
            "epochs": restore_epochs, "posts_read": restore_posts, "noncompliant": restore_noncompliant,
        },
        "second_restore": { "outcomes": second_outcomes },
        "return": {
            "wall_s": return_wall, "cpu_s": return_cpu, "first_sync_s": first_sync_s, "syncs": syncs,
            "epochs": return_epochs, "posts_read": return_posts,
        },
    });
    println!("{out}");
}
