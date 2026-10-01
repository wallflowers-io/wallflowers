//! Durability — the relay's promise that a restart is not a data-loss event.
//!
//! The M1 relay kept the mailbox in a `HashMap` and the commit slots in a
//! `HashSet` beside it, so a redeploy silently forgot both. These tests pin the
//! two consequences that mattered, and the third property that makes bounded
//! retention honest:
//!
//!   1. UNDRAINED BLOBS SURVIVE. A publisher's `Ack{ok:true}` now means the blob
//!      is on disk, so a device that was offline across a redeploy still gets it.
//!   2. COMMIT SLOTS SURVIVE. This is the one that is worse than lost mail: a
//!      forgotten slot lets a SECOND device win a commit for a group-epoch that
//!      already had one, and the group forks.
//!   3. EVICTION IS ANNOUNCED. Nothing expires by default now (see
//!      `the_default_retention_keeps_everything`), but an operator who turns a
//!      limit on can still lose history — and a subscriber below the floor is told
//!      with a `Gap` rather than handed a short replay it cannot distinguish from a
//!      complete one.

use std::time::Duration;

use futures_util::{SinkExt, Stream, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::{Error as WsErr, Message};

use relay::store::Store;
use relay::wire::Frame;
use relay::Retention;

/// Every tag in these tests is a REAL ADDRESS: the relay refuses a publish its tag
/// did not sign (pacific_wire::address). A short name stands for the address of a
/// seed spelled by that name, so the tests still read "aa", "bb".
fn addr(name: &str) -> relay::wire::address::Address {
    let mut seed = [0u8; 32];
    seed[..name.len()].copy_from_slice(name.as_bytes());
    relay::wire::address::Address::from_seed(&seed)
}

/// The tag `name` stands for, as the wire spells it.
fn t(name: &str) -> String {
    addr(name).tag_hex()
}

/// A `Pub` of `blob` to `name`'s address, signed by that address's key.
#[allow(dead_code)]
fn publish(name: &str, blob: &str, commit: bool) -> Frame {
    addr(name).pub_frame(blob.into(), commit)
}


async fn send(ws: &mut (impl SinkExt<Message, Error = WsErr> + Unpin), f: Frame) {
    ws.send(Message::Text(f.to_json())).await.expect("send frame");
}

async fn recv<S: Stream<Item = Result<Message, WsErr>> + Unpin>(ws: &mut S) -> Frame {
    loop {
        match ws.next().await.expect("stream open").expect("ws ok") {
            Message::Text(t) => return Frame::from_json(&t.to_string()).expect("parse frame"),
            Message::Close(_) => panic!("closed before a frame arrived"),
            _ => continue,
        }
    }
}

/// Receive with a deadline — for asserting that a frame does NOT arrive.
async fn recv_within<S: Stream<Item = Result<Message, WsErr>> + Unpin>(
    ws: &mut S,
    ms: u64,
) -> Option<Frame> {
    tokio::time::timeout(Duration::from_millis(ms), recv(ws)).await.ok()
}

/// A unique scratch path. No tempfile dev-dep for one string.
fn scratch(name: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!("relay-{name}-{}-{nanos}.db", std::process::id()));
    p.to_string_lossy().into_owned()
}

/// Boot a relay on an ephemeral port over `path`. Returns (url, task handle).
async fn boot(path: &str, retention: Retention) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = Store::open(path).expect("open store");
    let h = tokio::spawn(relay::serve_with_store(listener, store, retention, None));
    (format!("ws://{addr}"), h)
}

/// Retention that never evicts — these tests are about the restart, not the window.
fn forever() -> Retention {
    Retention { window_us: 0, max_per_tag: 0, sweep: Duration::from_secs(3600) }
}

/// THE DEFAULT KEEPS EVERYTHING (ruled 13 Sep 2026, replacing a 7-day window).
/// A week with the phone off is an ordinary gap, so the mailbox may not carry a
/// deadline nobody set: an age window and a per-tag cap are both opt-in, and while
/// neither is set the sweep never starts. Pinned because the cost of this quietly
/// regressing is undrained mail deleted on a timer.
#[test]
fn the_default_retention_keeps_everything() {
    let d = Retention::default();
    assert_eq!(d.window_us, 0, "no age window unless an operator sets one");
    assert_eq!(d.max_per_tag, 0, "no per-tag cap unless an operator sets one");
}

#[tokio::test]
async fn undrained_blobs_and_commit_slots_survive_a_restart() {
    let path = scratch("restart");

    // ---- first boot: publish a plain blob and claim the tag's commit slot ----
    let (url, relay1) = boot(&path, forever()).await;
    let (mut publisher, _) = connect_async(&url).await.unwrap();

    send(&mut publisher, publish("aa", "aGk", false)).await;
    let plain_seq = match recv(&mut publisher).await {
        Frame::Ack { seq, ok: true, .. } => seq,
        other => panic!("plain pub should Ack ok, got {other:?}"),
    };

    send(&mut publisher, publish("bb", "Y29tbWl0", true)).await;
    match recv(&mut publisher).await {
        Frame::Ack { ok: true, .. } => {}
        other => panic!("first commit should win the slot, got {other:?}"),
    }

    // ---- the redeploy ----
    drop(publisher);
    relay1.abort();
    let (url2, _relay2) = boot(&path, forever()).await;

    // 1. THE BLOB IS STILL THERE. A subscriber that was offline across the restart
    //    drains it exactly as if nothing had happened.
    let (mut late, _) = connect_async(&url2).await.unwrap();
    send(&mut late, Frame::Sub { tags: vec![t("aa")], since: 0, v: 0 }).await;
    match recv(&mut late).await {
        Frame::Msg { tag, seq, blob } => {
            assert_eq!(tag, t("aa"));
            assert_eq!(blob, "aGk", "the blob published before the restart");
            assert_eq!(seq, plain_seq, "and it keeps the seq the publisher was acked");
        }
        other => panic!("expected the pre-restart blob, got {other:?}"),
    }
    assert_eq!(recv(&mut late).await, Frame::Eose);

    // 2. THE COMMIT SLOT IS STILL TAKEN. This is the fork guard: before durability
    //    the restart freed the slot and a second device could win the same epoch.
    let (mut racer, _) = connect_async(&url2).await.unwrap();
    send(&mut racer, publish("bb", "cml2YWw", true)).await;
    match recv(&mut racer).await {
        Frame::Ack { ok: false, .. } => {}
        other => panic!("a restart must NOT re-open a claimed commit slot, got {other:?}"),
    }

    // ...and the rejected commit was not stored either.
    let (mut reader, _) = connect_async(&url2).await.unwrap();
    send(&mut reader, Frame::Sub { tags: vec![t("bb")], since: 0, v: 0 }).await;
    let mut bodies = Vec::new();
    loop {
        match recv(&mut reader).await {
            Frame::Msg { blob, .. } => bodies.push(blob),
            Frame::Eose => break,
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(bodies, vec!["Y29tbWl0".to_string()], "only the winning commit survives");

    // 3. seq keeps climbing past everything the old process issued.
    send(&mut racer, publish("aa", "bmV3", false)).await;
    match recv(&mut racer).await {
        Frame::Ack { seq, ok: true, .. } => {
            assert!(seq > plain_seq, "seq must be monotone across a restart: {seq} <= {plain_seq}")
        }
        other => panic!("expected ok ack, got {other:?}"),
    }

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn eviction_announces_a_gap_instead_of_a_silent_hole() {
    let path = scratch("gap");
    // Evict by CAP, not by age. The age path is exercised at the store level
    // (`store::tests`) where the clock is an argument: `seq` is seeded a whole
    // BOOT_SKEW_US ahead of the wall clock, so an age window only starts biting a
    // second after boot and a socket test of it would be a sleep, not an assertion.
    let retention = Retention {
        window_us: 0,
        max_per_tag: 1,
        sweep: Duration::from_millis(50),
    };
    let (url, _relay) = boot(&path, retention).await;

    let (mut publisher, _) = connect_async(&url).await.unwrap();
    send(&mut publisher, publish("cc", "b2xk", false)).await;
    let evicted_seq = match recv(&mut publisher).await {
        Frame::Ack { seq, ok: true, .. } => seq,
        other => panic!("expected ok ack, got {other:?}"),
    };
    send(&mut publisher, publish("cc", "bmV3", false)).await;
    let kept_seq = match recv(&mut publisher).await {
        Frame::Ack { seq, ok: true, .. } => seq,
        other => panic!("expected ok ack, got {other:?}"),
    };

    // Let a sweep run: the cap is 1, so the older blob goes.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // A v>=1 subscriber whose cursor predates the eviction is TOLD, before the
    // backlog. Without this frame it receives only the surviving blob and cannot
    // tell a complete replay from one with a hole punched in it.
    let (mut sub, _) = connect_async(&url).await.unwrap();
    send(&mut sub, Frame::Sub { tags: vec![t("cc")], since: 0, v: 1 }).await;
    match recv(&mut sub).await {
        Frame::Gap { tag, floor } => {
            assert_eq!(tag, t("cc"));
            assert_eq!(floor, evicted_seq, "the floor is the highest seq actually removed");
        }
        other => panic!("expected a Gap ahead of the backlog, got {other:?}"),
    }
    match recv(&mut sub).await {
        Frame::Msg { seq, blob, .. } => {
            assert_eq!(seq, kept_seq);
            assert_eq!(blob, "bmV3", "the surviving blob still replays");
        }
        other => panic!("expected the surviving blob, got {other:?}"),
    }
    assert_eq!(recv(&mut sub).await, Frame::Eose);

    // A cursor AT the floor has missed nothing — no gap, just the survivor.
    let (mut caught_up, _) = connect_async(&url).await.unwrap();
    send(&mut caught_up, Frame::Sub { tags: vec![t("cc")], since: evicted_seq, v: 1 }).await;
    match recv(&mut caught_up).await {
        Frame::Msg { seq, .. } => assert_eq!(seq, kept_seq),
        other => panic!("a cursor at the floor lost nothing and must not be alarmed: {other:?}"),
    }
    assert_eq!(recv(&mut caught_up).await, Frame::Eose);

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn pre_retention_clients_are_never_sent_a_gap() {
    let path = scratch("compat");
    let retention = Retention {
        window_us: 0,
        max_per_tag: 1,
        sweep: Duration::from_millis(50),
    };
    let (url, _relay) = boot(&path, retention).await;

    let (mut publisher, _) = connect_async(&url).await.unwrap();
    send(&mut publisher, publish("dd", "b2xk", false)).await;
    assert!(matches!(recv(&mut publisher).await, Frame::Ack { ok: true, .. }));
    send(&mut publisher, publish("dd", "bmV3", false)).await;
    assert!(matches!(recv(&mut publisher).await, Frame::Ack { ok: true, .. }));
    tokio::time::sleep(Duration::from_millis(200)).await;

    // v = 0 is every build shipped before retention existed. `Gap` would be an
    // unknown frame there, and an unknown frame is FATAL on that client
    // (`next_frame` maps a parse failure to a transport error and the drain dies).
    // So the old contract must hold exactly: the surviving blob, then Eose.
    let (mut old, _) = connect_async(&url).await.unwrap();
    send(&mut old, Frame::Sub { tags: vec![t("dd")], since: 0, v: 0 }).await;
    match recv_within(&mut old, 500).await {
        Some(Frame::Msg { blob, .. }) => assert_eq!(blob, "bmV3"),
        other => panic!("expected the surviving blob, got {other:?}"),
    }
    assert_eq!(
        recv_within(&mut old, 500).await,
        Some(Frame::Eose),
        "a pre-retention client must receive exactly what it always did — never a Gap"
    );

    let _ = std::fs::remove_file(&path);
}

/// The PRODUCTION path, end to end: the real `semaphore` binary, configured the way
/// `arcup` configures it (`RELAY_BIND` + `RELAY_STORE`), killed the way a container runtime kills
/// it, and restarted on the same volume path.
///
/// The other tests drive `serve_with_store` directly, which skips the two things most
/// likely to be got wrong in a deploy: whether `serve()` reads `RELAY_STORE` at all,
/// and whether the file it writes is the file the next boot opens. A durability story
/// that only holds when a test hands the store in is not a durability story.
#[tokio::test]
async fn the_real_binary_survives_a_kill_on_the_same_store_path() {
    let path = scratch("binary");

    async fn boot_binary(path: &str) -> (String, std::process::Child) {
        // Claim a port, then release it for the child. A race here would surface as
        // a connect failure below, not as a false pass.
        let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);
        let child = std::process::Command::new(env!("CARGO_BIN_EXE_semaphore"))
            .env("RELAY_BIND", addr.to_string())
            .env("RELAY_STORE", path)
            .env("RELAY_RETENTION_SECS", "0") // no eviction; this is about the restart
            .spawn()
            .expect("spawn semaphore");
        let url = format!("ws://{addr}");
        for _ in 0..100 {
            if connect_async(&url).await.is_ok() {
                return (url, child);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("semaphore never came up on {addr}");
    }

    let (url, mut relay) = boot_binary(&path).await;
    let (mut publisher, _) = connect_async(&url).await.unwrap();
    send(&mut publisher, publish("ff", "cGVyc2lzdA", false)).await;
    let seq = match recv(&mut publisher).await {
        Frame::Ack { seq, ok: true, .. } => seq,
        other => panic!("expected ok ack, got {other:?}"),
    };
    send(&mut publisher, publish("gg", "Y21bWl0", true)).await;
    assert!(matches!(recv(&mut publisher).await, Frame::Ack { ok: true, .. }));

    // Kill it outright — no graceful shutdown, no flush hook. SIGKILL is what a
    // container runtime eventually sends, so it is what durability has to hold under.
    drop(publisher);
    relay.kill().expect("kill relay");
    relay.wait().expect("reap relay");

    let (url2, mut relay2) = boot_binary(&path).await;
    let (mut sub, _) = connect_async(&url2).await.unwrap();
    send(&mut sub, Frame::Sub { tags: vec![t("ff")], since: 0, v: 0 }).await;
    match recv(&mut sub).await {
        Frame::Msg { seq: got, blob, .. } => {
            assert_eq!(blob, "cGVyc2lzdA", "the blob outlived a SIGKILL");
            assert_eq!(got, seq);
        }
        other => panic!("expected the pre-kill blob, got {other:?}"),
    }
    assert_eq!(recv(&mut sub).await, Frame::Eose);

    // And the slot is still claimed — the fork guard holds across a hard kill.
    send(&mut sub, publish("gg", "cml2YWw", true)).await;
    match recv(&mut sub).await {
        Frame::Ack { ok: false, .. } => {}
        other => panic!("a killed relay must not re-open a claimed commit slot, got {other:?}"),
    }

    relay2.kill().ok();
    relay2.wait().ok();
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{path}{suffix}"));
    }
}
