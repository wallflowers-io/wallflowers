//! Admission — what the relay refuses, and that a refusal stores nothing.
//!
//! The relay is anonymous, so everything it accepts is decided here: a publish is
//! signed by the key its tag IS, or it is refused (`pacific_wire::address`), and a
//! signed publish is then held to the limits in `limits.rs`. Each case below drives
//! the real relay over a real socket and checks two things — the `Ack` names the
//! rule that refused it, and a subscriber never sees what was refused.

use std::time::Duration;

use futures_util::{SinkExt, Stream, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::{Error as WsErr, Message};

use relay::store::Store;
use relay::wire::address::Address;
use relay::wire::Frame;
use relay::{Limits, Retention};

fn addr(name: &str) -> Address {
    let mut seed = [0u8; 32];
    seed[..name.len()].copy_from_slice(name.as_bytes());
    Address::from_seed(&seed)
}

async fn send(ws: &mut (impl SinkExt<Message, Error = WsErr> + Unpin), f: Frame) {
    ws.send(Message::Text(f.to_json())).await.expect("send frame");
}

async fn recv<S: Stream<Item = Result<Message, WsErr>> + Unpin>(ws: &mut S) -> Frame {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => return Frame::from_json(&t.to_string()).expect("parse"),
            Ok(Some(Ok(_))) => continue,
            other => panic!("no frame: {other:?}"),
        }
    }
}

/// The Ack for the publish just sent: `(seq, ok, reason)`.
async fn ack<S: Stream<Item = Result<Message, WsErr>> + Unpin>(ws: &mut S) -> (u64, bool, Option<String>) {
    match recv(ws).await {
        Frame::Ack { seq, ok, reason } => (seq, ok, reason),
        other => panic!("expected an Ack, got {other:?}"),
    }
}

async fn boot(limits: Limits) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let store = Store::open_in_memory().unwrap();
    tokio::spawn(relay::serve_with_policy(listener, Box::new(store), Retention::default(), limits, None));
    url
}

/// Everything stored at `tag`, as a fresh subscriber sees it.
async fn backlog(url: &str, tag: &str) -> Vec<String> {
    let (mut ws, _) = connect_async(url).await.unwrap();
    send(&mut ws, Frame::Sub { tags: vec![tag.into()], since: 0, v: 0 }).await;
    let mut out = Vec::new();
    loop {
        match recv(&mut ws).await {
            Frame::Msg { blob, .. } => out.push(blob),
            Frame::Eose => return out,
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[tokio::test]
async fn only_the_key_an_address_is_can_write_to_it() {
    let url = boot(Limits::default()).await;
    let (mut ws, _) = connect_async(&url).await.unwrap();
    let victim = addr("victim");
    let mallory = addr("mallory");

    // Unsigned — what every client sent before signatures.
    send(&mut ws, Frame::Pub { tag: victim.tag_hex(), blob: "anVuaw".into(), commit: false, sig: String::new() }).await;
    assert_eq!(ack(&mut ws).await, (0, false, Some("malformed_sig".into())));

    // Signed by the wrong key.
    let sig = mallory.sign_pub("anVuaw");
    send(&mut ws, Frame::Pub { tag: victim.tag_hex(), blob: "anVuaw".into(), commit: false, sig }).await;
    assert_eq!(ack(&mut ws).await, (0, false, Some("bad_signature".into())));

    // A commit, to take the slot — refused the same way.
    let sig = mallory.sign_pub("Y29tbWl0");
    send(&mut ws, Frame::Pub { tag: victim.tag_hex(), blob: "Y29tbWl0".into(), commit: true, sig }).await;
    assert_eq!(ack(&mut ws).await, (0, false, Some("bad_signature".into())));

    // A tag that is not a key: nobody can write there at all.
    send(&mut ws, Frame::Pub { tag: "aa".into(), blob: "anVuaw".into(), commit: false, sig: "00".repeat(64) }).await;
    assert_eq!(ack(&mut ws).await, (0, false, Some("malformed_tag".into())));

    assert!(backlog(&url, &victim.tag_hex()).await.is_empty(), "nothing refused was stored");

    // And the holder of the key writes as always — the slot is still free for it.
    send(&mut ws, victim.pub_frame("Y29tbWl0".into(), true)).await;
    let (seq, ok, reason) = ack(&mut ws).await;
    assert!(ok && seq > 0 && reason.is_none());
    assert_eq!(backlog(&url, &victim.tag_hex()).await, vec!["Y29tbWl0".to_string()]);
}

#[tokio::test]
async fn a_republished_blob_is_acked_as_before_and_stored_once() {
    let url = boot(Limits::default()).await;
    let a = addr("head");
    let (mut sub, _) = connect_async(&url).await.unwrap();
    send(&mut sub, Frame::Sub { tags: vec![a.tag_hex()], since: 0, v: 0 }).await;
    assert_eq!(recv(&mut sub).await, Frame::Eose);

    let (mut ws, _) = connect_async(&url).await.unwrap();
    let frame = a.pub_frame("c2VhbGVk".into(), false);
    send(&mut ws, frame.clone()).await;
    let (first, ok, _) = ack(&mut ws).await;
    assert!(ok);
    // A captured publish, replayed verbatim.
    send(&mut ws, frame).await;
    assert_eq!(ack(&mut ws).await, (first, true, None), "the same seq: it is the same publish");

    assert!(matches!(recv(&mut sub).await, Frame::Msg { seq, .. } if seq == first));
    // The replay is not fanned out: the next thing the subscriber sees is the next
    // real publish, not a second copy.
    send(&mut ws, a.pub_frame("bmV4dA".into(), false)).await;
    let (next, _, _) = ack(&mut ws).await;
    assert!(matches!(recv(&mut sub).await, Frame::Msg { seq, .. } if seq == next));
    assert_eq!(backlog(&url, &a.tag_hex()).await.len(), 2);
}

#[tokio::test]
async fn an_oversized_blob_is_refused_before_it_is_stored() {
    let url = boot(Limits { max_blob_bytes: 16, ..Limits::default() }).await;
    let a = addr("big");
    let (mut ws, _) = connect_async(&url).await.unwrap();
    send(&mut ws, a.pub_frame("x".repeat(17), false)).await;
    assert_eq!(ack(&mut ws).await, (0, false, Some("too_large".into())));
    send(&mut ws, a.pub_frame("x".repeat(16), false)).await;
    assert!(ack(&mut ws).await.1, "at the limit is within it");
    assert_eq!(backlog(&url, &a.tag_hex()).await.len(), 1);
}

/// FAIL CLOSED. At the ceiling the store refuses; it never makes room by evicting,
/// so what was already there is exactly what was there.
#[tokio::test]
async fn a_full_store_refuses_and_keeps_what_it_has() {
    let url = boot(Limits { max_store_bytes: 10, ..Limits::default() }).await;
    let a = addr("full");
    let (mut ws, _) = connect_async(&url).await.unwrap();
    send(&mut ws, a.pub_frame("12345678".into(), false)).await;
    assert!(ack(&mut ws).await.1);
    send(&mut ws, a.pub_frame("abcdefgh".into(), false)).await;
    assert_eq!(ack(&mut ws).await, (0, false, Some("full".into())));
    assert_eq!(backlog(&url, &a.tag_hex()).await, vec!["12345678".to_string()]);
}

#[tokio::test]
async fn a_connection_that_publishes_too_fast_is_paced() {
    let url = boot(Limits { conn_pubs_per_sec: 0.001, conn_pub_burst: 2.0, ..Limits::default() }).await;
    let a = addr("fast");
    let (mut ws, _) = connect_async(&url).await.unwrap();
    for blob in ["b25l", "dHdv"] {
        send(&mut ws, a.pub_frame(blob.into(), false)).await;
        assert!(ack(&mut ws).await.1);
    }
    send(&mut ws, a.pub_frame("dGhyZWU".into(), false)).await;
    assert_eq!(ack(&mut ws).await, (0, false, Some("rate_limited".into())));

    // The pace is per connection: another one is unaffected.
    let (mut other, _) = connect_async(&url).await.unwrap();
    send(&mut other, a.pub_frame("dGhyZWU".into(), false)).await;
    assert!(ack(&mut other).await.1);
}

/// Keys are the cheap thing an anonymous caller has, so NEW addresses spend from a
/// budget of their own. Writing again to an address that exists costs none of it.
#[tokio::test]
async fn new_addresses_spend_from_their_own_budget() {
    let url = boot(Limits { conn_new_tags_per_min: 0.001, conn_new_tag_burst: 1.0, ..Limits::default() }).await;
    let (mut ws, _) = connect_async(&url).await.unwrap();
    send(&mut ws, addr("one").pub_frame("YQ".into(), false)).await;
    assert!(ack(&mut ws).await.1);
    send(&mut ws, addr("one").pub_frame("Yg".into(), false)).await;
    assert!(ack(&mut ws).await.1, "an existing address is not a new one");
    send(&mut ws, addr("two").pub_frame("YQ".into(), false)).await;
    assert_eq!(ack(&mut ws).await, (0, false, Some("rate_limited".into())));
    assert!(backlog(&url, &addr("two").tag_hex()).await.is_empty());
}

#[tokio::test]
async fn the_whole_relay_has_an_ingest_ceiling() {
    let url = boot(Limits {
        max_blob_bytes: 8,
        ingest_bytes_per_sec: 0.001,
        ingest_bytes_burst: 8.0,
        ..Limits::default()
    })
    .await;
    let (mut a, _) = connect_async(&url).await.unwrap();
    let (mut b, _) = connect_async(&url).await.unwrap();
    send(&mut a, addr("a").pub_frame("12345678".into(), false)).await;
    assert!(ack(&mut a).await.1);
    // A different connection, within its own pace, still meets the ceiling.
    send(&mut b, addr("b").pub_frame("1".into(), false)).await;
    assert_eq!(ack(&mut b).await, (0, false, Some("busy".into())));
}
