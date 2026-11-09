//! A door's tag (`mls.door.relay`, B3, ruled 9 Nov 2026). Anyone holding an exterior can
//! derive its door and knock, so the relay bounds what a door keeps: a cap that evicts the
//! oldest as the newest lands, a TTL, a knock size, and a few knocks an hour per connection.
//! Members are told how many knocks a door has lost to its cap, so a flood is seen by the
//! door it targets. Every other tag keeps everything, as before.

use std::time::Duration;

use futures_util::{SinkExt, Stream, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::{Error as WsErr, Message};

use relay::mailbox::Mailbox;
use relay::store::Store;
use relay::wire::address::Address;
use relay::wire::{Frame, DOOR_CAP, DOOR_MAX_BYTES, DOOR_PER_CONN_PER_HOUR, DOOR_PREFIX, DOOR_TTL_DAYS, V_DOOR, V_MEDIA};
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

async fn ack<S: Stream<Item = Result<Message, WsErr>> + Unpin>(ws: &mut S) -> (bool, Option<String>) {
    match recv(ws).await {
        Frame::Ack { ok, reason, .. } => (ok, reason),
        other => panic!("expected an Ack, got {other:?}"),
    }
}

async fn boot() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    tokio::spawn(relay::serve_with_policy(
        listener,
        Box::new(Store::open_in_memory().unwrap()),
        Retention::default(),
        Limits::default(),
        None,
    ));
    url
}

/// `n` distinct publishes to `a`, as a door's or as its bare tag, a fresh connection for each
/// door's worth of knocks an hour so the per-connection quota is not what is tested.
async fn publish_n(url: &str, a: &Address, n: u64, door: bool) {
    let per = DOOR_PER_CONN_PER_HOUR;
    for chunk in 0..n.div_ceil(per) {
        let (mut ws, _) = connect_async(url).await.unwrap();
        for i in chunk * per..((chunk + 1) * per).min(n) {
            let blob = relay::wire::blob_b64(format!("knock {i}").as_bytes());
            let f = if door { a.door_pub_frame(blob) } else { a.pub_frame(blob, false) };
            send(&mut ws, f).await;
            assert_eq!(ack(&mut ws).await, (true, None), "publish {i}");
        }
    }
}

/// What a subscriber at level `v` is sent for `tag`: frames up to and including `Eose`.
async fn drain(url: &str, tag: &str, v: u8) -> Vec<Frame> {
    let (mut ws, _) = connect_async(url).await.unwrap();
    send(&mut ws, Frame::Sub { tags: vec![tag.into()], since: 0, v }).await;
    let mut out = Vec::new();
    loop {
        let f = recv(&mut ws).await;
        let end = f == Frame::Eose;
        out.push(f);
        if end {
            return out;
        }
    }
}

#[tokio::test]
async fn a_door_holds_its_cap_evicting_the_oldest_and_says_how_many() {
    let url = boot().await;
    let door = addr("door");
    publish_n(&url, &door, DOOR_CAP + 1, true).await;

    let got = drain(&url, &door.door_tag(), V_DOOR).await;
    let msgs: Vec<&String> = got.iter().filter_map(|f| if let Frame::Msg { blob, .. } = f { Some(blob) } else { None }).collect();
    assert_eq!(msgs.len() as u64, DOOR_CAP, "the cap holds at append, with no sweep");
    assert_eq!(msgs[0], &relay::wire::blob_b64(b"knock 1"), "the oldest went");
    assert!(matches!(got[0], Frame::Gap { .. }), "the hole is announced first: {:?}", got[0]);
    assert_eq!(got[got.len() - 2], Frame::Evicted { tag: door.door_tag(), total: 1 });
    assert_eq!(got.last(), Some(&Frame::Eose));

    let below = drain(&url, &door.door_tag(), V_MEDIA).await;
    assert!(!below.iter().any(|f| matches!(f, Frame::Evicted { .. })), "never sent below v3");
}

#[tokio::test]
async fn a_bare_tag_keeps_everything() {
    let url = boot().await;
    let a = addr("bare");
    publish_n(&url, &a, DOOR_CAP + 1, false).await;
    let got = drain(&url, &a.tag_hex(), V_DOOR).await;
    assert_eq!(got.iter().filter(|f| matches!(f, Frame::Msg { .. })).count() as u64, DOOR_CAP + 1);
    assert!(!got.iter().any(|f| matches!(f, Frame::Gap { .. } | Frame::Evicted { .. })));
}

#[tokio::test]
async fn a_knock_over_the_door_cap_is_too_large() {
    let url = boot().await;
    let door = addr("big");
    let (mut ws, _) = connect_async(&url).await.unwrap();
    send(&mut ws, door.door_pub_frame("A".repeat(DOOR_MAX_BYTES as usize + 1))).await;
    assert_eq!(ack(&mut ws).await, (false, Some("too_large".into())));
    send(&mut ws, door.door_pub_frame("A".repeat(DOOR_MAX_BYTES as usize))).await;
    assert_eq!(ack(&mut ws).await, (true, None));
    // The same size to a bare tag is under the relay's own cap.
    send(&mut ws, door.pub_frame("B".repeat(DOOR_MAX_BYTES as usize + 1), false)).await;
    assert_eq!(ack(&mut ws).await, (true, None));
}

#[tokio::test]
async fn one_connection_knocks_a_few_times_an_hour_at_one_door() {
    let url = boot().await;
    let (one, two) = (addr("one"), addr("two"));
    let (mut ws, _) = connect_async(&url).await.unwrap();
    for i in 0..DOOR_PER_CONN_PER_HOUR {
        send(&mut ws, one.door_pub_frame(relay::wire::blob_b64(format!("k{i}").as_bytes()))).await;
        assert_eq!(ack(&mut ws).await, (true, None));
    }
    send(&mut ws, one.door_pub_frame(relay::wire::blob_b64(b"one too many"))).await;
    assert_eq!(ack(&mut ws).await, (false, Some("rate_limited".into())));
    send(&mut ws, two.door_pub_frame(relay::wire::blob_b64(b"another door"))).await;
    assert_eq!(ack(&mut ws).await, (true, None), "the quota is per door");
    assert_eq!(drain(&url, &one.door_tag(), V_DOOR).await.len() as u64, DOOR_PER_CONN_PER_HOUR + 1, "the refused knock was not stored");
}

#[tokio::test]
async fn a_knock_expires_after_the_door_ttl_and_nothing_else_does() {
    let store = Store::open_in_memory().unwrap();
    let door = format!("{DOOR_PREFIX}{}", addr("old").tag_hex());
    let bare = addr("old").tag_hex();
    let day_us = 86_400_000_000u64;
    let t0 = 10 * day_us;
    store.append(&door, t0, "a", false).unwrap();
    store.append(&bare, t0 + 1, "b", false).unwrap();
    let ttl = DOOR_TTL_DAYS * day_us;

    assert_eq!(Mailbox::expire(&store, DOOR_PREFIX, t0 + ttl - 1, ttl).await.unwrap(), 0, "not yet");
    assert_eq!(Mailbox::expire(&store, DOOR_PREFIX, t0 + ttl + 2, ttl).await.unwrap(), 1);
    assert!(store.replay(&door, 0).unwrap().is_empty());
    assert_eq!(store.replay(&bare, 0).unwrap().len(), 1, "a bare tag of the same age stays");
    assert_eq!(store.floors(&[door.clone()]).unwrap().get(&door), Some(&t0), "the expiry is announced as a floor");
    assert!(store.evicted_totals(&[door.clone()]).unwrap().is_empty(), "expiry is not a flood");
}
