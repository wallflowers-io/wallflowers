//! SESSION COST: what one member of a large group costs the Door (NC-48; RX.6; NC-27).
//!
//! Measurement, not a test. One forum is built on the test harness's in-process relay:
//! its owner adds M joiners, one Add each, with Ada a member from the start and synced to
//! the head. Then Ada's device is weighed as a Door session would carry it:
//!
//!   state  every byte of Ada's state directory: the tmpfs a session's cgroup is charged.
//!   rss    a child process opens that directory as a session would (`Node::open`, the
//!          group's MLS state loaded, the object folded) and reports its resident size.
//!          The same probe on a device that holds nothing is the baseline; the
//!          difference is what the group adds to a session.
//!   add    the commit and the Welcome of the last Add: the relay's cap on one publish
//!          (`RELAY_MAX_BLOB_BYTES`, 256 KiB by default) is where a group stops (NC-27).
//!
//!   RELAY_MAX_BLOB_BYTES=1048576 cargo run --release -p pacific-core --example session_cost -- <M> [D]
//!
//! M joiners, D posts in the forum (default 10). Prints one JSON line. State lives in
//! temp dirs; nothing leaves 127.0.0.1.

#[path = "../tests/common/mod.rs"]
#[allow(dead_code)]
mod common;

use common::Harness;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn rss_kib(pid: u32) -> u64 {
    let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &pid.to_string()]).output().expect("ps");
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0)
}

fn bytes_under(p: &Path) -> u64 {
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let m = match e.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            n += if m.is_dir() { bytes_under(&e.path()) } else { m.len() };
        }
    }
    n
}

/// `probe <state dir> [object]`: open the device as a session would, load and fold, and
/// print this process's resident size in KiB.
fn probe(dir: &str, obj: Option<&str>) {
    std::env::set_var("PACIFIC_STATE_DIR", dir);
    let node = pacific_core::node::Node::open().expect("open the device");
    if let Some(obj) = obj {
        let (sk, pk) = node.dir.mls_signing_keypair().unwrap();
        let sid = pacific_core::mls::signing_identity(&node.id.identity_pk(), &pk);
        let client = pacific_core::mls::build_client_sqlite(&pacific_core::paths::db_path(), sid, pacific_core::mls::SecretKey::new(sk)).unwrap();
        let group = pacific_core::mls::load_group(&client, &hex::decode(obj).unwrap()).expect("the group");
        let members = group.roster().members().len();
        let _folded = node.object_detailed(obj).expect("the object folds");
        println!("{} {}", rss_kib(std::process::id()), members);
    } else {
        let _ = node.noncompliant_objects();
        println!("{} 0", rss_kib(std::process::id()));
    }
}

/// Device `u`'s state directory: `Harness::node` points PACIFIC_STATE_DIR at it.
fn state_dir(h: &Harness, u: usize) -> PathBuf {
    let _ = h.node(u);
    PathBuf::from(std::env::var("PACIFIC_STATE_DIR").expect("the harness sets it"))
}

fn run_probe(dir: &Path, obj: Option<&str>) -> (u64, usize) {
    let me = std::env::current_exe().unwrap();
    let mut cmd = std::process::Command::new(me);
    cmd.arg("probe").arg(dir);
    if let Some(o) = obj {
        cmd.arg(o);
    }
    let out = cmd.output().expect("the probe");
    let s = String::from_utf8_lossy(&out.stdout);
    let mut it = s.split_whitespace();
    (it.next().and_then(|v| v.parse().ok()).unwrap_or(0), it.next().and_then(|v| v.parse().ok()).unwrap_or(0))
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("probe") {
        return probe(&args[1], args.get(2).map(String::as_str));
    }
    let m: usize = args.first().map(|s| s.parse().expect("M, an integer")).unwrap_or(100);
    let d: usize = args.get(1).map(|s| s.parse().expect("D, an integer")).unwrap_or(10);

    let mut names = vec!["ada".to_string(), "owner".to_string(), "nobody".to_string()];
    names.extend((0..m).map(|i| format!("j{i}")));
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let h = Harness::new(&refs).await;
    let (ada, owner, nobody) = (0usize, 1usize, 2usize);

    let t = Instant::now();
    let obj = h.form_forum(owner, &[ada]).await;
    h.settle_among(&[owner, ada]).await;
    let mut last = serde_json::Value::Null;
    let mut refused = serde_json::Value::Null;
    // A key package is valid from its maker's current second, with no margin. The VM's
    // clock is stepped back about 0.5 s every 10 s, which can put the adder's "now" before
    // it; that refusal is retried once with a fresh bundle, and counted (it is a finding).
    let mut lifetime_retries = 0;
    for j in 0..m {
        let joiner = 3 + j;
        let mut added = false;
        for attempt in 0..2 {
            let bundle = h.node(joiner).build_contact_bundle().unwrap();
            match h.node(owner).group_add_member(&obj, &bundle).await {
                Ok(_) => {
                    added = true;
                    break;
                }
                Err(e) if attempt == 0 && e.to_string().contains("key package lifetime") => {
                    lifetime_retries += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
                }
                Err(e) => {
                    refused = serde_json::json!({ "join": j + 1, "error": e.to_string() });
                    break;
                }
            }
        }
        if !added {
            break;
        }
        last = serde_json::json!({ "join": j + 1 });
    }
    for i in 0..d {
        h.obj_post(owner, &obj, &format!("post {i}")).await;
    }
    h.settle_among(&[owner, ada]).await;
    let build_s = t.elapsed().as_secs_f64();

    let members = h.roster(ada, &obj).len();
    let (ada_dir, nobody_dir) = (state_dir(&h, ada), state_dir(&h, nobody));
    let state = bytes_under(&ada_dir);
    let (rss, leaves) = run_probe(&ada_dir, Some(&obj));
    let (rss_base, _) = run_probe(&nobody_dir, None);
    let bundle = h.node(nobody).build_contact_bundle().unwrap();
    let (commit, welcome) = {
        let node = h.node(owner);
        let (sk, pk) = node.dir.mls_signing_keypair().unwrap();
        let sid = pacific_core::mls::signing_identity(&node.id.identity_pk(), &pk);
        let client = pacific_core::mls::build_client_sqlite(&pacific_core::paths::db_path(), sid, pacific_core::mls::SecretKey::new(sk)).unwrap();
        let mut g = pacific_core::mls::load_group(&client, &hex::decode(&obj).unwrap()).unwrap();
        let kp = pacific_core::handshake::parse_and_verify(&bundle).unwrap().key_package;
        let (c, w) = pacific_core::mls::stage_add_member(&mut g, &kp).unwrap();
        (c.len(), w.len())
    };
    println!(
        "{}",
        serde_json::json!({
            "joiners": m, "posts": d, "members": members, "leaves": leaves, "last": last, "refused": refused,
            "key_package_lifetime_retries": lifetime_retries,
            "state_bytes": state, "rss_kib": rss, "rss_baseline_kib": rss_base, "rss_for_the_group_kib": rss.saturating_sub(rss_base),
            "next_add": { "commit": commit, "welcome": welcome },
            "relay_max_blob_bytes": std::env::var("RELAY_MAX_BLOB_BYTES").unwrap_or_else(|_| "default (262144)".into()),
            "build_s": build_s,
        })
    );
}
