//! Detached media end to end (O-79, `media.detached`, #56): sealed bytes up through the relay's
//! presign to object storage, and back, by a reader that needs only the reference. Object
//! storage is a stand-in on this machine; the relay presigns for it over plain http
//! (`Media::plain_on_loopback`), with the same signatures it gives R2.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pacific_core::media::Delivery;
use pacific_core::media_store;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

type Objects = Arc<Mutex<HashMap<String, Vec<u8>>>>;

/// PUT stores the body at the path, GET returns it. Signatures are not checked: the relay's
/// presign is what the test is about. A PUT must be conditional (`If-None-Match: *`) and is
/// refused 412 over an object already there, as R2 refuses it.
async fn object_store() -> (String, Objects) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = l.local_addr().unwrap().to_string();
    let objects: Objects = Default::default();
    let held = objects.clone();
    tokio::spawn(async move {
        loop {
            let (sock, _) = l.accept().await.unwrap();
            let held = held.clone();
            tokio::spawn(async move {
                let mut r = BufReader::new(sock);
                let mut line = String::new();
                r.read_line(&mut line).await.unwrap();
                let mut parts = line.split_whitespace();
                let (method, target) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
                let path = target.split('?').next().unwrap().to_string();
                let mut len = 0usize;
                let mut only_create = false;
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).await.unwrap();
                    if h == "\r\n" || h.is_empty() {
                        break;
                    }
                    let h = h.to_ascii_lowercase();
                    if let Some(v) = h.strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                    only_create |= h.strip_prefix("if-none-match:").is_some_and(|v| v.trim() == "*");
                }
                let (status, body) = match method.as_str() {
                    "PUT" => {
                        let mut b = vec![0u8; len];
                        r.read_exact(&mut b).await.unwrap();
                        let mut held = held.lock().unwrap();
                        if !only_create {
                            ("400 Bad Request", Vec::new())
                        } else if held.contains_key(&path) {
                            ("412 Precondition Failed", Vec::new())
                        } else {
                            held.insert(path, b);
                            ("200 OK", Vec::new())
                        }
                    }
                    "GET" => match held.lock().unwrap().get(&path) {
                        Some(b) => ("200 OK", b.clone()),
                        None => ("404 Not Found", Vec::new()),
                    },
                    _ => ("405 Method Not Allowed", Vec::new()),
                };
                let mut sock = r.into_inner();
                let head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                sock.write_all(head.as_bytes()).await.unwrap();
                sock.write_all(&body).await.unwrap();
            });
        }
    });
    (at, objects)
}

async fn relay_presigning_for(store: &str) -> String {
    let media = relay::media::Media::new(
        store,
        "pacific-media",
        relay::sigv4::Credentials { access_key_id: "k".into(), secret_access_key: "s".into() },
        25 * 1024 * 1024,
        Duration::from_secs(900),
    )
    .plain_on_loopback();
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", l.local_addr().unwrap());
    let store = Box::new(relay::store::Store::open_in_memory().unwrap());
    tokio::spawn(relay::serve_with_media(l, store, relay::Retention::default(), relay::Limits::default(), None, Some(media)));
    url
}

#[tokio::test]
async fn sealed_bytes_go_up_and_come_back_to_anyone_holding_the_reference() {
    let (store, objects) = object_store().await;
    let relay = relay_presigning_for(&store).await;
    let photo = b"\xff\xd8\xff\xe0 a jpeg's bytes, as far as anyone storing them can tell".repeat(1000);

    let before = media_store::presigned();
    let r = media_store::put(&relay, &photo, "image/jpeg").await.unwrap();
    assert!(media_store::presigned() > before, "a presign is counted");
    let Delivery::Sealed { ref key, bytes, .. } = r.delivery else { panic!("a sealed reference: {r:?}") };
    let held = objects.lock().unwrap().get(&format!("/pacific-media/{key}")).cloned().expect("the object is stored under its key");
    assert_eq!(held.len() as u64, bytes);
    assert!(!held.windows(16).any(|w| photo.windows(16).next() == Some(w)), "what is stored is sealed");

    // Read by a reader with nothing but the reference: no Node, no group.
    assert_eq!(media_store::get(&relay, &r).await.unwrap(), photo);

    // An upload only creates: a second presigned PUT to the same key is refused.
    let mut s = pacific_core::transport::RelaySession::connect(&relay).await.unwrap();
    let again = s.media_url(key, Some(4)).await.unwrap();
    let res = reqwest::Client::new().put(again).header("If-None-Match", "*").body("junk").send().await.unwrap();
    assert_eq!(res.status(), 412);
    assert_eq!(media_store::get(&relay, &r).await.unwrap(), photo, "still the referenced object");

    // A longer object is refused by its length, before it is held.
    objects.lock().unwrap().get_mut(&format!("/pacific-media/{key}")).unwrap().push(0);
    assert!(media_store::get(&relay, &r).await.is_err());
    objects.lock().unwrap().get_mut(&format!("/pacific-media/{key}")).unwrap().pop();

    // A changed object is not the one referenced.
    objects.lock().unwrap().get_mut(&format!("/pacific-media/{key}")).unwrap()[30] ^= 1;
    assert!(media_store::get(&relay, &r).await.is_err());
}
