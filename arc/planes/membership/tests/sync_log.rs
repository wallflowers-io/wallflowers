//! The arc-node binary's sync loop, run against a real relay. It printed every room it folds to
//! stderr, and so to syslog in plaintext, and dialled the relay afresh every pass, hanging up
//! without a close (8 Nov 2026).

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Duration;

use pacific_core::node::Node;
use relay::RelayEvent;

const MARKER: &str = "a room's words, never the log's";

#[test]
fn a_pass_logs_no_room_and_keeps_one_connection() {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
    let l = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let relay = format!("ws://{}", l.local_addr().unwrap());
    let (obs, mut events) = tokio::sync::mpsc::unbounded_channel();
    rt.spawn(relay::serve_with_observer(l, Some(obs)));

    // The Arc's own state: a forum it holds, one post in it, folded.
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("arc-sync-log-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("PACIFIC_STATE_DIR", &dir);
    pacific_core::router::Routes::parse(&relay).save().unwrap();
    Node::init_identity("arc").unwrap();
    let node = Node::open().unwrap();
    let room = node.object_new("forum", "Room").unwrap();
    rt.block_on(node.object_post(&room, MARKER, None)).unwrap();
    rt.block_on(node.sync_once()).unwrap();
    drop(node);
    while events.try_recv().is_ok() {}

    let mut arc = Command::new(env!("CARGO_BIN_EXE_node"))
        .env("PACIFIC_STATE_DIR", &dir)
        .env("ARC_NAME", "arc")
        .env("ARC_RELAY_URL", &relay)
        .env("ARC_SIGNUP_BIND", "127.0.0.1:0")
        .env("ARC_SYNC_MS", "100")
        .env_remove("ARC_PUBLIC_URL")
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_secs(3));
    arc.kill().unwrap();
    arc.wait().unwrap();
    let mut err = String::new();
    arc.stderr.take().unwrap().read_to_string(&mut err).unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    let mut opened = 0;
    while let Ok(e) = events.try_recv() {
        if matches!(e, RelayEvent::ConnOpened { .. }) {
            opened += 1;
        }
    }
    assert!(!err.contains(MARKER), "a room's post reached arc-node's log:\n{err}");
    assert!(!err.contains("sync failed"), "the passes ran:\n{err}");
    // ~30 passes in 3s; one held session, not one each.
    assert!((1..=2).contains(&opened), "{opened} relay connections in 3s of 100ms passes:\n{err}");
}
