//! `GET /health` on the relay's port: whether it can keep what it is sent. On 8 Nov 2026 the
//! store refused every write for want of disk while health said `ok`.

use std::collections::HashMap;

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use relay::mailbox::{Appended, Mailbox, Result, StoreError};
use relay::store::Store;
use relay::{Limits, Retention};

fn scratch(name: &str) -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let p = std::env::temp_dir().join(format!("relay-health-{name}-{}-{nanos}.db", std::process::id()));
    p.to_string_lossy().into_owned()
}

async fn boot(store: Box<dyn Mailbox>, limits: Limits) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = listener.local_addr().unwrap();
    tokio::spawn(relay::serve_with_policy(listener, store, Retention::default(), limits, None));
    at
}

/// Status and body of `GET /health`, once the first check has run.
async fn health(at: std::net::SocketAddr) -> (u16, serde_json::Value) {
    let mut last = (0, serde_json::Value::Null);
    for _ in 0..50 {
        let mut s = TcpStream::connect(at).await.unwrap();
        s.write_all(b"GET /health HTTP/1.1\r\nHost: relay\r\n\r\n").await.unwrap();
        let mut res = String::new();
        s.read_to_string(&mut res).await.unwrap();
        let status: u16 = res.split(' ').nth(1).unwrap().parse().unwrap();
        let body = serde_json::from_str(res.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        last = (status, body);
        if last.1["store"] == true || status == 503 && last.1["disk_free_pct"].is_u64() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    last
}

#[tokio::test]
async fn a_store_that_takes_writes_on_a_disk_with_room_is_healthy() {
    let at = boot(Box::new(Store::open(&scratch("ok")).unwrap()), Limits { min_disk_free_pct: 0, ..Limits::default() }).await;
    let (status, body) = health(at).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["store"], true);
    assert!(body["disk_free_pct"].is_u64(), "a file store reports its disk: {body}");
}

#[tokio::test]
async fn a_disk_below_the_floor_is_unhealthy() {
    let at = boot(Box::new(Store::open(&scratch("full")).unwrap()), Limits { min_disk_free_pct: 101, ..Limits::default() }).await;
    let (status, body) = health(at).await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(body["store"], true);
}

/// A store that reads but can no longer write: what a full disk did to `relay.db`.
struct Unwritable(Store);

#[async_trait]
impl Mailbox for Unwritable {
    async fn resume_seq(&self) -> Result<u64> {
        Mailbox::resume_seq(&self.0).await
    }
    async fn append(&self, _: &str, _: u64, _: &str, _: bool) -> Result<Appended> {
        Err(StoreError::Index("database or disk is full".into()))
    }
    async fn has_tag(&self, tag: &str) -> Result<bool> {
        Mailbox::has_tag(&self.0, tag).await
    }
    async fn stored_bytes(&self) -> Result<u64> {
        Mailbox::stored_bytes(&self.0).await
    }
    async fn replay(&self, tag: &str, since: u64) -> Result<Vec<(u64, String)>> {
        Mailbox::replay(&self.0, tag, since).await
    }
    async fn floors(&self, tags: &[String]) -> Result<HashMap<String, u64>> {
        Mailbox::floors(&self.0, tags).await
    }
    async fn evict(&self, now_us: u64, window_us: u64, max_per_tag: u64) -> Result<u64> {
        Mailbox::evict(&self.0, now_us, window_us, max_per_tag).await
    }
    async fn counts(&self) -> Result<(u64, u64)> {
        Mailbox::counts(&self.0).await
    }
    async fn tag_counts(&self) -> Result<Vec<(String, u64)>> {
        Mailbox::tag_counts(&self.0).await
    }
    async fn probe(&self) -> Result<()> {
        Err(StoreError::Index("database or disk is full".into()))
    }
    fn dir(&self) -> Option<std::path::PathBuf> {
        Mailbox::dir(&self.0)
    }
    fn describe(&self) -> String {
        "unwritable".into()
    }
}

#[tokio::test]
async fn a_store_that_cannot_write_is_unhealthy() {
    // A file store, so `disk_free_pct` shows the check has run.
    let at = boot(Box::new(Unwritable(Store::open(&scratch("ro")).unwrap())), Limits { min_disk_free_pct: 0, ..Limits::default() }).await;
    let (status, body) = health(at).await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(body["store"], false);
    assert!(body["disk_free_pct"].is_u64(), "the check ran: {body}");
}

#[tokio::test]
async fn the_websocket_still_upgrades_on_the_same_port() {
    let at = boot(Box::new(Store::open_in_memory().unwrap()), Limits::default()).await;
    tokio_tungstenite::connect_async(format!("ws://{at}")).await.expect("upgrade");
}
