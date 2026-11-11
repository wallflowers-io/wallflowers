//! Media presigns are paced per connection (ARC_NETWORK, #56): every `MediaPut` may create an
//! object that is never deleted, so a connection asks only so many times a minute.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use relay::media::Media;
use relay::sigv4::Credentials;
use relay::store::Store;
use relay::wire::{Frame, V_MEDIA};
use relay::{Limits, Retention};

#[tokio::test]
async fn a_connection_past_its_presigns_is_refused_rate_limited() {
    let media = Media::new("127.0.0.1:9", "b", Credentials { access_key_id: "k".into(), secret_access_key: "s".into() }, 1024, Duration::from_secs(900));
    let limits = Limits { conn_presigns_per_min: 1.0, conn_presign_burst: 2.0, ..Limits::default() };
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", l.local_addr().unwrap());
    tokio::spawn(relay::serve_with_media(l, Box::new(Store::open_in_memory().unwrap()), Retention::default(), limits, None, Some(media)));

    let (mut ws, _) = connect_async(&url).await.unwrap();
    let mut next = async |ws: &mut _| -> Frame {
        let ws: &mut tokio_tungstenite::WebSocketStream<_> = ws;
        loop {
            if let Some(Ok(Message::Text(t))) = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.unwrap() {
                return Frame::from_json(&t.to_string()).unwrap();
            }
        }
    };
    ws.send(Message::Text(Frame::Sub { tags: vec![], since: 0, v: V_MEDIA }.to_json().into())).await.unwrap();
    assert_eq!(next(&mut ws).await, Frame::Eose);
    let key = "ab".repeat(32);
    for (i, put) in [false, true, false].into_iter().enumerate() {
        let f = if put { Frame::MediaPut { key: key.clone(), len: 10 } } else { Frame::MediaGet { key: key.clone() } };
        ws.send(Message::Text(f.to_json().into())).await.unwrap();
        match (i, next(&mut ws).await) {
            (0 | 1, Frame::MediaUrl { .. }) => {}
            (2, Frame::MediaErr { reason, .. }) => assert_eq!(reason, "rate_limited"),
            (i, other) => panic!("presign {i}: {other:?}"),
        }
    }
}
