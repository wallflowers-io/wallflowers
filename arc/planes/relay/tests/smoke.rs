//! Relay smoke test — drives the blind mailbox over a real loopback socket.
//!
//! Proves: live fan-out (a subscriber gets a `Pub` that lands after it subscribed),
//! and backlog replay (a LATE subscriber gets the stored blob + `Eose`). This is
//! the relay's M1 fitness check.

use futures_util::{SinkExt, Stream, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::{Error as WsErr, Message};

use relay::wire::Frame;

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
            _ => continue, // ping/pong
        }
    }
}

#[tokio::test]
async fn live_fanout_and_backlog_replay() {
    // Boot the relay on an ephemeral loopback port.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(relay::serve(listener));
    let url = format!("ws://{addr}");

    // Subscriber connects + subscribes to tag "aa"; empty backlog -> immediate Eose.
    let (mut sub, _) = connect_async(&url).await.unwrap();
    send(&mut sub, Frame::Sub { tags: vec![t("aa")], since: 0, v: 0 }).await;
    assert_eq!(recv(&mut sub).await, Frame::Eose, "empty backlog should Eose");

    // Publisher connects + publishes to "aa".
    let (mut publisher, _) = connect_async(&url).await.unwrap();
    send(&mut publisher, publish("aa", "aGk", false)).await; // "hi"
    let first_seq = match recv(&mut publisher).await {
        Frame::Ack { seq, ok: true, .. } => seq,
        other => panic!("plain pub should Ack ok, got {other:?}"),
    };

    // The live subscriber receives it.
    match recv(&mut sub).await {
        Frame::Msg { tag, blob, .. } => {
            assert_eq!(tag, t("aa"));
            assert_eq!(blob, "aGk");
        }
        other => panic!("expected live Msg, got {other:?}"),
    }

    // A LATE subscriber gets the stored backlog, then Eose.
    let (mut late, _) = connect_async(&url).await.unwrap();
    send(&mut late, Frame::Sub { tags: vec![t("aa")], since: 0, v: 0 }).await;
    match recv(&mut late).await {
        Frame::Msg { blob, .. } => assert_eq!(blob, "aGk", "backlog replay"),
        other => panic!("expected backlog Msg, got {other:?}"),
    }
    assert_eq!(recv(&mut late).await, Frame::Eose, "Eose after backlog");

    // And `since` skips already-seen blobs. The cursor comes from the Ack rather
    // than a literal, because `seq` is a wall-clock microsecond stamp and not a
    // counter from 1 (see `Hub::next_seq`) — which is also how a real client
    // learns it.
    let (mut caught_up, _) = connect_async(&url).await.unwrap();
    send(&mut caught_up, Frame::Sub { tags: vec![t("aa")], since: first_seq, v: 0 }).await;
    assert_eq!(recv(&mut caught_up).await, Frame::Eose, "since=<last seq> skips it");
}

/// A relay restart must not rewind `seq`, because client cursors never rewind.
///
/// Before the clock-seeded sequence this was the single worst bug in the
/// transport: `Hub::seq` restarted at 0 on every boot (redeploy, OOM
/// kill) while `inbox_cursor_v2` advances with `MAX(last_seen_id, …)`. Every
/// client holding a cursor above the restarted counter went silently deaf on that
/// tag — `subscribe` replays only `seq > since`, and live fanout carries the same
/// low `seq` — for as long as it took the counter to climb back. Publishers saw
/// `Ack{ok:true}` throughout, so nothing surfaced the loss.
///
/// This drives the real failure: publish to relay A, note the seq a client would
/// persist, then bring up a FRESH Hub (a redeploy) and assert both that its
/// numbering continues above A's and that a client resuming from the old cursor
/// still receives what is published afterwards.
#[tokio::test]
async fn seq_survives_a_relay_restart() {
    async fn boot() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(relay::serve(listener));
        format!("ws://{addr}")
    }

    // --- relay A: a client publishes and persists the resulting cursor.
    let url_a = boot().await;
    let (mut a, _) = connect_async(&url_a).await.unwrap();
    send(&mut a, publish("ee", "YmVmb3Jl", false)).await;
    let cursor = match recv(&mut a).await {
        Frame::Ack { seq, ok: true, .. } => seq,
        other => panic!("expected ok Ack, got {other:?}"),
    };
    assert!(cursor > 0, "seq must be non-zero");

    // --- the redeploy: a brand-new Hub, with an empty store and a fresh counter.
    let url_b = boot().await;
    let (mut b, _) = connect_async(&url_b).await.unwrap();
    send(&mut b, publish("ee", "YWZ0ZXI", false)).await;
    let after = match recv(&mut b).await {
        Frame::Ack { seq, ok: true, .. } => seq,
        other => panic!("expected ok Ack, got {other:?}"),
    };
    assert!(
        after > cursor,
        "a restarted relay reissued seq {after} at or below a live client cursor {cursor} — \
         every such blob is invisible to that client forever"
    );

    // --- and the consequence that actually bit: resuming from the old cursor
    // against the restarted relay still delivers.
    let (mut resumed, _) = connect_async(&url_b).await.unwrap();
    send(&mut resumed, Frame::Sub { tags: vec![t("ee")], since: cursor, v: 0 }).await;
    match recv(&mut resumed).await {
        Frame::Msg { blob, seq, .. } => {
            assert_eq!(blob, "YWZ0ZXI", "post-restart blob must reach a resuming client");
            assert!(seq > cursor, "delivered seq must be above the client's cursor");
        }
        other => panic!("resuming client got {other:?} — it went deaf across the restart"),
    }
    assert_eq!(recv(&mut resumed).await, Frame::Eose);
}

/// The blind sequencer: exactly ONE commit-flagged blob is accepted per tag
/// (one tag per group-epoch ⇒ one MLS commit per epoch, first-writer-wins).
/// Losers get `Ack{ok:false}` and their blob is neither stored nor fanned out;
/// plain publishes on the same tag are untouched by the slot.
#[tokio::test]
async fn commit_slot_first_writer_wins() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(relay::serve(listener));
    let url = format!("ws://{addr}");

    let (mut a, _) = connect_async(&url).await.unwrap();
    let (mut b, _) = connect_async(&url).await.unwrap();

    // A wins the slot on "cc".
    send(&mut a, publish("cc", "Y29tbWl0QQ", true)).await;
    assert!(matches!(recv(&mut a).await, Frame::Ack { ok: true, .. }), "first commit wins");

    // B loses the same slot — rejected, not stored.
    send(&mut b, publish("cc", "Y29tbWl0Qg", true)).await;
    assert!(matches!(recv(&mut b).await, Frame::Ack { ok: false, .. }), "second commit rejected");

    // Plain publishes are never slot-gated, before or after.
    send(&mut b, publish("cc", "YXBw", false)).await;
    assert!(matches!(recv(&mut b).await, Frame::Ack { ok: true, .. }), "plain pub unaffected");

    // The mailbox holds exactly the winner + the plain blob, in order.
    let (mut sub, _) = connect_async(&url).await.unwrap();
    send(&mut sub, Frame::Sub { tags: vec![t("cc")], since: 0, v: 0 }).await;
    match recv(&mut sub).await {
        Frame::Msg { blob, .. } => assert_eq!(blob, "Y29tbWl0QQ", "winner's commit stored"),
        other => panic!("expected winner commit, got {other:?}"),
    }
    match recv(&mut sub).await {
        Frame::Msg { blob, .. } => assert_eq!(blob, "YXBw", "plain blob stored"),
        other => panic!("expected plain blob, got {other:?}"),
    }
    assert_eq!(recv(&mut sub).await, Frame::Eose, "loser's commit was never stored");

    // A different tag has its own independent slot.
    send(&mut b, publish("dd", "Y29tbWl0Qg", true)).await;
    assert!(matches!(recv(&mut b).await, Frame::Ack { ok: true, .. }), "slots are per-tag");
}
