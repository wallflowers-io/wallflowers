//! THE DELTA TAP (training_wheels P0): every Delta this session's Node holds, reported to the
//! supervisor on the leash as `tap {row}` lines, for training_wheels' `delta` table and the
//! backfill. It feeds the integrity checks, never state. The first flush after a start
//! reports the whole held log; each later one, what arrived since. The supervisor's writes
//! are idempotent on (object, Delta), so a restart reporting again costs rows, not truth.

use std::collections::HashMap;
use std::io::Write;
use std::os::fd::FromRawFd;
use std::sync::OnceLock;

use pacific_core::Node;

/// Rows read from the log per query.
const PAGE: i64 = 1000;

pub struct Tap {
    /// The last delta_log rowid reported.
    last: i64,
    out: std::sync::mpsc::Sender<String>,
}

impl Tap {
    /// A writer thread that owns a duplicate of the leash (fd 0), so the actor never waits
    /// on the supervisor: a line is queued, and written when the socket takes it.
    pub fn on_leash() -> std::io::Result<Tap> {
        // SAFETY: fd 0 is the leash, open for the life of this process; dup gives this
        // thread its own descriptor to own and close.
        let fd = unsafe { libc::dup(0) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut leash = unsafe { std::fs::File::from_raw_fd(fd) };
        let (out, rx) = std::sync::mpsc::channel::<String>();
        std::thread::Builder::new().name("tw-tap".into()).spawn(move || {
            for line in rx {
                if writeln!(leash, "tap {line}").is_err() {
                    break;
                }
            }
        })?;
        Ok(Tap { last: 0, out })
    }

    /// Every row of the log past the last reported, in order. How many were sent.
    pub fn flush(&mut self, node: &Node) -> Result<usize, String> {
        let mut sent = 0;
        loop {
            let page = rows_after(node, self.last)?;
            let n = page.len();
            for (rowid, row) in page {
                self.last = rowid;
                self.out.send(row.to_string()).map_err(|_| "the tap's writer has ended".to_string())?;
                sent += 1;
            }
            if (n as i64) < PAGE {
                return Ok(sent);
            }
        }
    }
}

/// What the ICD names, by a Delta's wire ids: each kind by its type id, and each op by (type id,
/// op id), over the kinds' own ops and every facet's on each kind in its `on`. Read from the model
/// /v2/icd serves, not the authoring catalogue: that holds only what `build` writes, and a
/// membership record is on none of it (only the MLS doors write one). A retired id names nothing.
struct Names {
    kinds: HashMap<u16, &'static str>,
    ops: HashMap<(u16, u32), &'static str>,
    /// Two names for one (type id, op id): the ICD at odds with itself. Empty.
    twice: Vec<String>,
}

fn names_of(icd: &'static serde_json::Value) -> Names {
    let mut n = Names { kinds: HashMap::new(), ops: HashMap::new(), twice: Vec::new() };
    let Some(kinds) = icd["kinds"].as_object() else { return n };
    let type_of = |k: &str| kinds.get(k).and_then(|v| v["typeId"].as_u64()).map(|t| t as u16);
    let name = |n: &mut Names, t: u16, ops: &'static serde_json::Value| {
        for (op, o) in ops.as_object().into_iter().flatten() {
            let Some(id) = o["op"].as_u64() else { continue };
            if let Some(was) = n.ops.insert((t, id as u32), op.as_str()).filter(|w| *w != op) {
                n.twice.push(format!("type {t} op {id}: {was} and {op}"));
            }
        }
    };
    for (k, v) in kinds {
        if let Some(t) = type_of(k) {
            n.kinds.insert(t, k.as_str());
            name(&mut n, t, &v["ops"]);
        }
    }
    for f in icd["facets"].as_object().into_iter().flat_map(|f| f.values()) {
        for t in f["on"].as_array().into_iter().flatten().filter_map(|k| k.as_str().and_then(type_of)) {
            name(&mut n, t, &f["ops"]);
        }
    }
    n
}

fn names() -> &'static Names {
    static N: OnceLock<Names> = OnceLock::new();
    N.get_or_init(|| names_of(crate::icd::model()))
}

/// The log's rows after `after`, each as a `delta` row. A row that will not decode is still
/// reported, by its ids, with its kind and op left empty: the checks name it.
fn rows_after(node: &Node, after: i64) -> Result<Vec<(i64, serde_json::Value)>, String> {
    let mut stmt = node
        .dir
        .conn
        .prepare("SELECT rowid, group_id, delta_id, author_pk, envelope, received_at FROM delta_log WHERE rowid > ?1 ORDER BY rowid LIMIT ?2")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([after, PAGE], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?, r.get::<_, Vec<u8>>(2)?, r.get::<_, Vec<u8>>(3)?, r.get::<_, Vec<u8>>(4)?, r.get::<_, i64>(5)?))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (rowid, group, delta_id, author, envelope, received) = row.map_err(|e| e.to_string())?;
        let d = pacific_core::coordinator::decode_delta(&envelope).ok();
        let kind = d.as_ref().and_then(|d| names().kinds.get(&(d.type_id as u16)).copied());
        let op = d.as_ref().and_then(|d| names().ops.get(&(d.type_id as u16, d.op_id)).copied());
        let args = d.as_ref().map(|d| {
            serde_json::Value::Object(
                d.args
                    .iter()
                    .map(|(k, v)| {
                        let v = match v {
                            pacific_core::coordinator::ArgVal::Int(i) => serde_json::json!(i),
                            pacific_core::coordinator::ArgVal::Text(t) => serde_json::json!(t),
                        };
                        (k.clone(), v)
                    })
                    .collect(),
            )
        });
        out.push((
            rowid,
            serde_json::json!({
                "table": "delta",
                "object": hex::encode(&group),
                "delta_id": hex::encode(&delta_id),
                "author": hex::encode(&author),
                "kind": kind,
                "op": op,
                "args": args,
                "epoch": d.as_ref().map(|d| d.epoch as i64),
                "seq": d.as_ref().and_then(|d| d.seq).map(|s| s as i64),
                "gen": d.as_ref().and_then(|d| d.gen).map(|g| g as i64),
                "at": received * 1000,
            }),
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
    use base64::Engine;
    use ed25519_dalek::Signer;
    use pacific_core::object::ObjectKind;

    /// PACIFIC_STATE_DIR is the process's: one device at a time, as core's harness keeps it.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// The relay the Door's tests run (arc's semaphore, built by check-door first). Its process
    /// group ends with it.
    struct Relay {
        child: std::process::Child,
        url: String,
    }

    impl Relay {
        fn start(root: &std::path::Path) -> Relay {
            use std::os::unix::process::CommandExt;
            let bin = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../arc/target/debug/semaphore");
            assert!(bin.exists(), "this test needs the relay: `cargo build -p relay` in arc ({} is absent)", bin.display());
            let free = || std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
            let (port, tunnel) = (free(), free());
            let child = std::process::Command::new(bin)
                .current_dir(root)
                .env("RELAY_BIND", format!("127.0.0.1:{port}"))
                .env("RELAY_TUNNEL_BIND", format!("127.0.0.1:{tunnel}"))
                .env_remove("SENTRY_DSN")
                .env_remove("RELAY_STORE")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .process_group(0)
                .spawn()
                .expect("the relay");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
            while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
                assert!(std::time::Instant::now() < deadline, "the relay did not listen within 120 s");
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Relay { child, url: format!("ws://127.0.0.1:{port}") }
        }
    }

    impl Drop for Relay {
        fn drop(&mut self) {
            // SAFETY: a signal to the process group this test started.
            unsafe { libc::kill(-(self.child.id() as i32), libc::SIGKILL) };
            let _ = self.child.wait();
        }
    }

    /// A device at `dir`, pinned to `relay` before its Node exists (as core's harness pins).
    fn device(dir: &std::path::Path, relay: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::env::set_var("PACIFIC_STATE_DIR", dir);
        pacific_core::router::Routes::parse(relay).save().unwrap();
        pacific_core::node::set_default_arc(relay).unwrap();
    }

    fn open(dir: &std::path::Path) -> Node {
        std::env::set_var("PACIFIC_STATE_DIR", dir);
        Node::open().unwrap()
    }

    /// A kiosk's claim for `site`, as the kiosk signs one (pacific_core::claim).
    fn claim(kiosk: &ed25519_dalek::SigningKey, site: &str) -> String {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let pk = kiosk.verifying_key().to_bytes();
        let p = B64.encode(format!(
            r#"{{"k":"{}","s":"{site}","n":"{}","iat":{now},"exp":{}}}"#,
            pacific_core::claim::kid_of(&pk),
            B64.encode([5u8; 16]),
            now + 600
        ));
        format!("v1.{p}.{}", B64.encode(kiosk.sign(format!("v1.{p}").as_bytes()).to_bytes()))
    }

    /// The tap names a kind as the ICD does, by its type id; core's ObjectKind names it the same
    /// wherever it has one. A difference is drift, said here, not a vocabulary the tap moves to.
    #[test]
    fn every_kind_core_names_is_the_icds_by_its_type_id() {
        let n = names();
        assert!(!n.kinds.is_empty());
        for (t, k) in &n.kinds {
            if let Some(core) = ObjectKind::from_type_id(*t) {
                assert_eq!(core.name(), *k, "type {t}: core says {:?}, the ICD {k:?}", core.name());
            }
        }
    }

    /// One name for each (type id, op id), over the kinds' ops and the facets'.
    #[test]
    fn the_icd_names_each_op_once() {
        assert_eq!(names().twice, Vec::<String>::new());
        assert_eq!(names().ops.get(&(ObjectKind::Group.type_id(), pacific_core::membership::OP_CLAIM_SPENT)), Some(&"base.claimSpent"));
    }

    /// IC-3 (training_wheels' integrity checks) compares the tap's rows by op. A Site's
    /// membership records, which only the MLS doors write and so are in no authoring
    /// catalogue, are named as the ICD names them: base.memberJoined, base.claimSpent. And
    /// every row of an object whose log folds names its op.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_tap_names_a_sites_membership_records() {
        let _one = ENV.lock().unwrap_or_else(|p| p.into_inner());
        let root = std::env::temp_dir().join(format!("door-tap-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&root).unwrap();
        let relay = Relay::start(&root);
        let (owner, visitor) = (root.join("owner"), root.join("visitor"));
        device(&owner, &relay.url);
        Node::init_identity("owner").unwrap();
        let site = open(&owner).mint(ObjectKind::Group, &pacific_core::mint::MintDraft { name: "Site".into(), ..Default::default() }).await.unwrap();
        device(&visitor, &relay.url);
        let v = Node::init_identity("visitor").unwrap();
        let (who, bundle) = (hex::encode(v.id.identity_pk()), v.build_contact_bundle().unwrap());
        drop(v);
        // The owner's Node spends a kiosk claim on its own Site: an Add, its base.memberJoined,
        // and the claim's base.claimSpent, through the one door that writes them.
        let kiosk = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
        let keys = std::collections::HashMap::from([(pacific_core::claim::kid_of(&kiosk.verifying_key().to_bytes()), kiosk.verifying_key().to_bytes())]);
        let n = open(&owner);
        n.admit_by_claim(&site, &claim(&kiosk, &site), &[bundle], &keys).await.expect("admitted");
        let nc = n.noncompliant_objects().unwrap();
        assert!(nc.is_empty(), "every object this Node holds folds: {} do not", nc.len());

        let rows: Vec<serde_json::Value> = rows_after(&n, 0).unwrap().into_iter().map(|(_, r)| r).collect();
        let unnamed: Vec<String> = rows
            .iter()
            .filter(|r| r["op"].is_null())
            .map(|r| format!("{} {} {:?}", r["object"].as_str().unwrap_or("?"), r["kind"], r["args"].as_object().map(|a| a.keys().cloned().collect::<Vec<_>>())))
            .collect();
        assert!(unnamed.is_empty(), "{} row(s) of objects that fold name no op:\n{}", unnamed.len(), unnamed.join("\n"));
        let on_site = |op: &str| rows.iter().filter(|r| r["object"] == site.as_str() && r["op"] == op && r["args"]["member"] == who.as_str()).count();
        assert_eq!(on_site("base.memberJoined"), 1, "the visitor's Add, recorded and named");
        assert_eq!(on_site("base.claimSpent"), 1, "the claim's spend on the visitor, named");
        drop(n);
        drop(relay);
        let _ = std::fs::remove_dir_all(&root);
    }
}
