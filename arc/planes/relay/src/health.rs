//! health — whether the relay can keep what it is sent.
//!
//! On 8 Nov 2026 wallflowers-01's disk filled. Every publish failed in the store and
//! was refused `unavailable`, while `/v1/health` said `ok`: it asked only whether a
//! relay was configured. Health is now a write to the store and the disk's free
//! space, checked on a timer, served as a plain `GET /health` on the relay's own port
//! (the gateway asks it), and said once an hour at ERROR while it is bad, which is
//! what reaches Sentry.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const UNKNOWN: u64 = u64::MAX;

pub struct Health {
    store: AtomicBool,
    free_pct: AtomicU64,
    min_free_pct: u64,
}

impl Health {
    /// Unhealthy until the first check has run.
    pub fn new(min_free_pct: u64) -> Self {
        Self { store: AtomicBool::new(false), free_pct: AtomicU64::new(UNKNOWN), min_free_pct }
    }

    pub fn set(&self, store: bool, free_pct: Option<u64>) {
        self.store.store(store, Ordering::Relaxed);
        self.free_pct.store(free_pct.unwrap_or(UNKNOWN), Ordering::Relaxed);
    }

    pub fn store(&self) -> bool {
        self.store.load(Ordering::Relaxed)
    }

    /// `None` for a store with no file (in memory).
    pub fn free_pct(&self) -> Option<u64> {
        Some(self.free_pct.load(Ordering::Relaxed)).filter(|&p| p != UNKNOWN)
    }

    pub fn ok(&self) -> bool {
        self.store() && self.free_pct().map_or(true, |p| p >= self.min_free_pct)
    }

    pub fn body(&self) -> String {
        serde_json::json!({ "store": self.store(), "disk_free_pct": self.free_pct() }).to_string()
    }
}

/// Percent of the filesystem holding `dir` that is free to an unprivileged writer.
pub fn free_pct(dir: &Path) -> Option<u64> {
    let s = nix::sys::statvfs::statvfs(dir).ok()?;
    let total = s.blocks() as u64;
    (total > 0).then(|| s.blocks_available() as u64 * 100 / total)
}

/// Answer a plain `GET /health`, told apart from a websocket upgrade by its first
/// bytes. True when answered; otherwise nothing has been read and the handshake goes on.
pub async fn answer(stream: &mut TcpStream, h: &Health) -> std::io::Result<bool> {
    const ASK: &[u8] = b"GET /health ";
    let mut first = [0u8; ASK.len()];
    let peeked = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let n = stream.peek(&mut first).await?;
            if n == 0 || n == ASK.len() || !ASK.starts_with(&first[..n]) {
                return Ok::<_, std::io::Error>(n);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    match peeked {
        Ok(Ok(n)) if n == ASK.len() && first == *ASK => {}
        Ok(Err(e)) => return Err(e),
        _ => return Ok(false),
    }
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 8192 {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        head.extend_from_slice(&buf[..n]);
    }
    let body = h.body();
    let status = if h.ok() { "200 OK" } else { "503 Service Unavailable" };
    let res = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(res.as_bytes()).await?;
    let _ = stream.shutdown().await;
    Ok(true)
}
