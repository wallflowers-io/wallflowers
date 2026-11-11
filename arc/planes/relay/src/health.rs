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
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use tracing::{error, info};

const UNKNOWN: u64 = u64::MAX;

/// The share of the media budget at which the relay says so at ERROR, once an hour. Media
/// is never deleted (#56, ruled 9 Nov 2026), so this is the warning before uploads stop.
pub const MEDIA_ALARM_PCT: u64 = 80;

const SAY_EVERY: Duration = Duration::from_secs(3600);

pub struct Health {
    store: AtomicBool,
    free_pct: AtomicU64,
    min_free_pct: u64,
    media_used_pct: AtomicU64,
}

impl Health {
    /// Unhealthy until the first check has run.
    pub fn new(min_free_pct: u64) -> Self {
        Self {
            store: AtomicBool::new(false),
            free_pct: AtomicU64::new(UNKNOWN),
            min_free_pct,
            media_used_pct: AtomicU64::new(UNKNOWN),
        }
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

    /// `None` until the media meter has read the bucket, or with no media budget.
    pub fn media_used_pct(&self) -> Option<u64> {
        Some(self.media_used_pct.load(Ordering::Relaxed)).filter(|&p| p != UNKNOWN)
    }

    /// One reading of the media bucket; returns its share of the budget. At or above
    /// [`MEDIA_ALARM_PCT`] it is said at ERROR (Sentry) once an hour, and once when it
    /// falls back. It does not make the relay unhealthy: uploads still work.
    pub fn media_measured(&self, used_bytes: u64, budget_bytes: u64, said: &mut Option<Instant>, now: Instant) -> u64 {
        let pct = used_bytes.saturating_mul(100).checked_div(budget_bytes).unwrap_or(0);
        self.media_used_pct.store(pct, Ordering::Relaxed);
        if pct < MEDIA_ALARM_PCT {
            if said.take().is_some() {
                info!(used_pct = pct, "media storage back below its alarm");
            }
        } else if said.map_or(true, |t| now.duration_since(t) >= SAY_EVERY) {
            *said = Some(now);
            error!(
                used_bytes,
                budget_bytes,
                used_pct = pct,
                alarm_pct = MEDIA_ALARM_PCT,
                "MEDIA STORAGE NEAR ITS BUDGET — uploads are refused when it is full"
            );
        }
        pct
    }

    pub fn ok(&self) -> bool {
        self.store() && self.free_pct().map_or(true, |p| p >= self.min_free_pct)
    }

    pub fn body(&self) -> String {
        serde_json::json!({
            "store": self.store(),
            "disk_free_pct": self.free_pct(),
            "media_used_pct": self.media_used_pct(),
        })
        .to_string()
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
