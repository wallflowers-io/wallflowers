//! A relay that presigns media (O-79, #56) for an object store standing in for R2 on this
//! machine: what a scenario needs to put and get detached media through a real relay.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

pub type Objects = Arc<Mutex<HashMap<String, Vec<u8>>>>;

/// GETs the stand-in has answered.
pub static FETCHES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// PUT stores the body at the path, GET returns it. Signatures are not checked: the relay's
/// presign is what the test is about. A PUT must be conditional (`If-None-Match: *`) and is
/// refused 412 over an object already there, as R2 refuses it.
pub async fn object_store() -> (String, Objects) {
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
                    "GET" => {
                        FETCHES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        match held.lock().unwrap().get(&path) {
                            Some(b) => ("200 OK", b.clone()),
                            None => ("404 Not Found", Vec::new()),
                        }
                    }
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

pub async fn relay_presigning_for(store: &str) -> String {
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
