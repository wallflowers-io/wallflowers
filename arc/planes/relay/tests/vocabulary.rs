//! ONE FRAME VOCABULARY — the gate, over a real socket.
//!
//! `relay::wire`'s unit tests hold the enum and the gate function to the Semaphore
//! wire spec. These hold the RUNNING RELAY to them: what actually comes back down a
//! socket when a client asks for something its declared vocabulary cannot read.
//!
//! The bug being pinned was live. Against `wss://arc.kenjin.cc`, sending `media_get`
//! returned `media_url`; the client enum had never heard of `media_url`, so
//! `Frame::from_json` failed, `core/pacific-core/src/transport.rs` turned that into
//! `CoreError::Transport`, and the sync drain died. Four of the relay's ten frames
//! were server-only and no client anywhere emits one — so the media capability was
//! not earning the outage it could cause.
//!
//! The ruling is the one `Gap` already follows: a frame may only reach a client that
//! declared it can read it (`Sub.v`). Media is level 2. The capability is GATED, not
//! removed — `a_client_that_declares_level_two_still_gets_the_capability` is the
//! half of this that proves nothing was deleted.

use std::time::Duration;

use futures_util::{SinkExt, Stream, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::{Error as WsErr, Message};

use relay::store::Store;
use relay::wire::{Frame, V_BASE, V_GAP, V_MEDIA};
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

/// Boot a relay on an ephemeral port with an in-memory store and no retention, so
/// these tests depend on nothing but the vocabulary rules. Media offload is
/// deliberately UNCONFIGURED (no `RELAY_R2_*`), which is what makes the level-2
/// path observable: a client that has declared level 2 gets a `MediaErr` naming
/// `unconfigured`, which is a real answer from the real presign path.
async fn boot() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = Store::open_in_memory().expect("in-memory store");
    tokio::spawn(relay::serve_with_store(
        listener,
        store,
        Retention {
            window_us: 0,
            max_per_tag: 0,
            sweep: Duration::from_secs(300),
        },
        None,
    ));
    format!("ws://{addr}")
}

/// THE REGRESSION. A client that never declared the media vocabulary asks for a
/// presign and gets SILENCE — not a `MediaUrl`, not a `MediaErr`, nothing.
///
/// Silence is not laziness, it is the only safe answer. There is no "you are too
/// old" frame available: any such frame would sit at the same level as the request
/// and would itself be the unknown variant that kills the drain.
///
/// Proving a negative needs a positive after it: the `Pub` that follows must be
/// Acked, and the Ack must be the VERY NEXT frame on the socket. If the relay had
/// answered the media request, that answer would be sitting in front of the Ack.
#[tokio::test]
async fn a_pre_media_client_gets_no_answer_to_a_media_request() {
    let url = boot().await;
    let (mut c, _) = connect_async(&url).await.unwrap();

    // v0: the vocabulary every build ever shipped has.
    send(&mut c, Frame::Sub { tags: vec![t("aa")], since: 0, v: V_BASE }).await;
    assert_eq!(recv(&mut c).await, Frame::Eose, "empty backlog should Eose");

    // Both media requests, neither of which this connection can read an answer to.
    send(&mut c, Frame::MediaGet { key: "ab".into() }).await;
    send(&mut c, Frame::MediaPut { key: "ab".into(), len: 1024 }).await;

    // The next frame on this socket is the Ack for the publish that follows —
    // nothing the relay produced for the media requests is queued ahead of it.
    // Deliberately to "bb", a tag this connection is NOT subscribed to, so its own
    // live fan-out cannot arrive first and mask the thing being measured.
    send(&mut c, publish("bb", "aGk", false)).await;
    match recv_within(&mut c, 2_000).await {
        Some(Frame::Ack { ok: true, .. }) => {}
        Some(other) => panic!("a v0 client must be answered NOTHING for media, got {other:?}"),
        None => panic!("the relay stopped answering a v0 client altogether"),
    }
}

/// `Gap`'s level-1 rule is the precedent, and level 1 is not enough for media: a
/// client that opted into retention announcements has still said nothing about
/// media, so it is answered nothing too.
#[tokio::test]
async fn declaring_level_one_does_not_buy_the_media_vocabulary() {
    let url = boot().await;
    let (mut c, _) = connect_async(&url).await.unwrap();
    send(&mut c, Frame::Sub { tags: vec![t("aa")], since: 0, v: V_GAP }).await;
    assert_eq!(recv(&mut c).await, Frame::Eose);

    send(&mut c, Frame::MediaGet { key: "ab".into() }).await;
    // "bb" again: not subscribed here, so the Ack is the only frame that can come.
    send(&mut c, publish("bb", "aGk", false)).await;
    match recv_within(&mut c, 2_000).await {
        Some(Frame::Ack { ok: true, .. }) => {}
        Some(other) => panic!("a v1 client must be answered NOTHING for media, got {other:?}"),
        None => panic!("the relay stopped answering a v1 client altogether"),
    }
}

/// THE OTHER HALF: the capability was gated, not deleted.
///
/// A client that declares level 2 reaches the real presign path and gets a real
/// level-2 answer back. Object storage is unconfigured in this test, so the honest
/// answer is `MediaErr { reason: "unconfigured" }` — which is the presigner
/// speaking, not the gate.
#[tokio::test]
async fn a_client_that_declares_level_two_still_gets_the_capability() {
    let url = boot().await;
    let (mut c, _) = connect_async(&url).await.unwrap();
    send(&mut c, Frame::Sub { tags: vec![t("aa")], since: 0, v: V_MEDIA }).await;
    assert_eq!(recv(&mut c).await, Frame::Eose);

    send(&mut c, Frame::MediaGet { key: "ab".into() }).await;
    match recv_within(&mut c, 2_000).await {
        Some(Frame::MediaErr { key, reason }) => {
            assert_eq!(key, "ab");
            assert_eq!(
                reason, "unconfigured",
                "this relay has no object storage, and says so"
            );
        }
        other => panic!("a v2 client must reach the presign path, got {other:?}"),
    }

    send(&mut c, Frame::MediaPut { key: "ab".into(), len: 1024 }).await;
    match recv_within(&mut c, 2_000).await {
        Some(Frame::MediaErr { reason, .. }) => assert_eq!(reason, "unconfigured"),
        other => panic!("a v2 client must reach the presign path for PUT too, got {other:?}"),
    }
}

/// A gated request is DROPPED, never fatal: the connection keeps working, and the
/// mailbox behind it is untouched. A relay that hung up on an old client would have
/// converted a client bug into the outage this whole change exists to prevent.
#[tokio::test]
async fn a_gated_request_does_not_break_the_connection() {
    let url = boot().await;

    // A v0 subscriber, live on "aa".
    let (mut sub, _) = connect_async(&url).await.unwrap();
    send(&mut sub, Frame::Sub { tags: vec![t("aa")], since: 0, v: V_BASE }).await;
    assert_eq!(recv(&mut sub).await, Frame::Eose);

    // It asks for something it cannot read the answer to, twice.
    send(&mut sub, Frame::MediaGet { key: "ab".into() }).await;
    send(&mut sub, Frame::MediaPut { key: "cd".into(), len: 7 }).await;

    // A separate publisher lands a blob; the gated subscriber still receives it.
    let (mut publisher, _) = connect_async(&url).await.unwrap();
    send(&mut publisher, publish("aa", "aGk", false)).await;
    match recv_within(&mut publisher, 2_000).await {
        Some(Frame::Ack { ok: true, .. }) => {}
        other => panic!("publish should Ack ok, got {other:?}"),
    }
    match recv_within(&mut sub, 2_000).await {
        Some(Frame::Msg { tag, blob, .. }) => {
            assert_eq!(tag, t("aa"));
            assert_eq!(blob, "aGk", "live fan-out survives a gated request");
        }
        other => panic!("the gated subscriber must still be live, got {other:?}"),
    }
}

/// The declared vocabulary only RISES. A client cannot un-learn a frame variant
/// mid-connection, so a later `Sub { v: 0 }` — a re-subscribe from a build that has
/// already said it speaks level 2 — does not revoke what it declared.
#[tokio::test]
async fn the_declared_vocabulary_is_a_high_water_mark() {
    let url = boot().await;
    let (mut c, _) = connect_async(&url).await.unwrap();

    send(&mut c, Frame::Sub { tags: vec![t("aa")], since: 0, v: V_MEDIA }).await;
    assert_eq!(recv(&mut c).await, Frame::Eose);

    // Re-subscribe with the field omitted (v = 0 on the wire).
    send(&mut c, Frame::Sub { tags: vec![t("aa"), t("bb")], since: 0, v: V_BASE }).await;
    assert_eq!(recv(&mut c).await, Frame::Eose);

    // Still level 2: the build did not change, only this one frame's field.
    send(&mut c, Frame::MediaGet { key: "ab".into() }).await;
    match recv_within(&mut c, 2_000).await {
        Some(Frame::MediaErr { reason, .. }) => assert_eq!(reason, "unconfigured"),
        other => panic!("a declared level must not be revoked by a later Sub, got {other:?}"),
    }
}
