//! live — the held connection (O-69; mdr/fold-cache.md § How a read converges). One socket to
//! the relay per Node, subscribed to every tag the Node listens on, and read always: the relay
//! pushes what it stores, and nothing else here was listening between passes.
//!
//! IT NEVER INGESTS. A frame is queued and the owner rung; the Node's own thread turns frames
//! into rows (`Node::ingest_delivered`). It parses the framing and nothing more, holds tags and
//! never a secret, and its queue is bounded: past `QUEUE_BYTES` the queue is dropped and the
//! owner told to drain instead, so a member flooding a shared tag costs one drain, never memory.
//! It dials through `RelaySession::connect`, the one dial point.
//!
//! COMPLETE FROM `since`. The relay's `Sub` replaces a connection's tag set and replays every
//! stored blob after `since` before it streams live, in seq order (arc/planes/relay lib.rs:479).
//! So once the `Sub` that first carried a tag has ended in its `Eose`, every blob on that tag
//! after that `since` has been or will be delivered here, in order, for as long as this socket
//! lives; and a new socket that subscribes from the highest seq delivered carries it on. A tag
//! whose `Sub` has not been confirmed claims nothing.

use crate::transport::RelaySession;
use futures_util::{SinkExt, StreamExt};
use pacific_wire::Frame;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message as Ws;

/// What delivered frames may hold before they are dropped and the owner drains instead.
const QUEUE_BYTES: usize = 8 << 20;
/// A socket with nothing to say pings, so an edge does not close it as idle.
const KEEPALIVE: Duration = Duration::from_secs(25);

/// One blob the relay delivered: its tag (hex), its seq, the sealed blob (base64).
#[derive(Debug, Clone)]
pub struct Delivered {
    pub tag: String,
    pub seq: u64,
    pub blob: String,
}

/// What `take` hands over: the frames since the last take, and whether any were dropped.
#[derive(Debug, Default)]
pub struct Taken {
    pub frames: Vec<Delivered>,
    pub overflowed: bool,
}

struct Tag {
    /// The `since` of the first `Sub` that carried it on this chain of sockets.
    since: u64,
    /// That `Sub`'s number on the current socket; 0 when not yet sent on it.
    sub: u64,
    /// Its `Eose` has arrived: the tag is complete from `since`.
    confirmed: bool,
}

#[derive(Default)]
struct State {
    queue: Vec<Delivered>,
    bytes: usize,
    overflowed: bool,
    tags: HashMap<String, Tag>,
    /// The highest seq delivered: a (re)subscription starts after it.
    high: u64,
    /// Subs sent on the current socket, and Eoses received on it.
    sent: u64,
    ended: u64,
    /// The set changed and is not yet sent.
    dirty: bool,
}

struct Inner {
    state: Mutex<State>,
    up: AtomicBool,
    heard: AtomicU64,
    stop: AtomicBool,
    wake: tokio::sync::Notify,
    ring: Box<dyn Fn() + Send + Sync>,
}

impl Inner {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// The handle. Its thread ends when the last clone goes.
#[derive(Clone)]
pub struct Live(Arc<Handle>);

struct Handle(Arc<Inner>);

impl Drop for Handle {
    fn drop(&mut self) {
        self.0.stop.store(true, Ordering::SeqCst);
        self.0.wake.notify_one();
    }
}

impl Live {
    /// Dial `url` on a thread of its own and keep a socket open to it, reconnecting as it
    /// drops. `ring` is called after each frame is queued and after each `Sub` is confirmed;
    /// the caller coalesces. `after` is the highest seq the caller already holds: the first
    /// `Sub` replays only what came after it.
    pub fn start(url: String, after: u64, ring: Box<dyn Fn() + Send + Sync>) -> Live {
        let inner = Arc::new(Inner {
            state: Mutex::new(State { high: after, ..Default::default() }),
            up: AtomicBool::new(false),
            heard: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            wake: tokio::sync::Notify::new(),
            ring,
        });
        let run = inner.clone();
        std::thread::Builder::new()
            .name("live".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("the live runtime");
                rt.block_on(held(run, url));
            })
            .expect("the live thread");
        Live(Arc::new(Handle(inner)))
    }

    /// The whole set this socket listens on. It replaces the last: so does the relay's `Sub`.
    pub fn set_tags(&self, tags: impl IntoIterator<Item = String>) {
        let inner = &self.0 .0;
        let mut st = inner.state();
        let want: std::collections::HashSet<String> = tags.into_iter().collect();
        let before = st.tags.len();
        st.tags.retain(|t, _| want.contains(t));
        let mut changed = st.tags.len() != before;
        for t in want {
            st.tags.entry(t).or_insert_with(|| {
                changed = true;
                Tag { since: 0, sub: 0, confirmed: false }
            });
        }
        if changed {
            st.dirty = true;
            drop(st);
            inner.wake.notify_one();
        }
    }

    /// Every frame delivered since the last take.
    pub fn take(&self) -> Taken {
        let mut st = self.0 .0.state();
        st.bytes = 0;
        Taken { frames: std::mem::take(&mut st.queue), overflowed: std::mem::take(&mut st.overflowed) }
    }

    /// The `since` a tag is complete from, once its `Sub` is confirmed.
    pub fn since(&self, tag: &str) -> Option<u64> {
        self.0 .0.state().tags.get(tag).filter(|t| t.confirmed).map(|t| t.since)
    }

    /// Whether frames are queued and not yet taken.
    pub fn pending(&self) -> bool {
        let st = self.0 .0.state();
        !st.queue.is_empty() || st.overflowed
    }

    /// Whether frames on `tag` are queued and not yet taken, or may have been dropped.
    pub fn pending_on(&self, tag: &str) -> bool {
        let st = self.0 .0.state();
        st.overflowed || st.queue.iter().any(|f| f.tag == tag)
    }

    /// The set as asked, sorted.
    pub fn tags(&self) -> Vec<String> {
        let mut t: Vec<String> = self.0 .0.state().tags.keys().cloned().collect();
        t.sort();
        t
    }

    /// A socket is open.
    pub fn up(&self) -> bool {
        self.0 .0.up.load(Ordering::SeqCst)
    }

    /// Frames delivered, ever.
    pub fn heard(&self) -> u64 {
        self.0 .0.heard.load(Ordering::SeqCst)
    }

    /// Wait until every tag in the set is confirmed, up to `within`.
    pub fn wait_confirmed(&self, within: Duration) -> bool {
        let until = std::time::Instant::now() + within;
        loop {
            if self.0 .0.state().tags.values().all(|t| t.confirmed) {
                return true;
            }
            if std::time::Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

async fn held(inner: Arc<Inner>, url: String) {
    let mut backoff = Duration::from_millis(250);
    while !inner.stop.load(Ordering::SeqCst) {
        match RelaySession::connect(&url).await {
            Ok(s) => {
                backoff = Duration::from_millis(250);
                inner.up.store(true, Ordering::SeqCst);
                read(&inner, s).await;
                inner.up.store(false, Ordering::SeqCst);
                (inner.ring)();
            }
            Err(e) => {
                tracing::warn!(target: "pacific::live", error = %e, "the held connection could not be opened; retrying");
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = inner.wake.notified() => {}
                }
                backoff = (backoff * 2).min(Duration::from_secs(10));
            }
        }
    }
}

async fn read(inner: &Inner, session: RelaySession) {
    let (mut tx, mut rx) = session.into_socket().split();
    {
        // A NEW SOCKET SUBSCRIBES AFRESH, after the highest seq delivered. A tag confirmed on
        // the last socket stays complete from its `since`: this one replays everything after
        // `high`, and the last delivered everything up to it in order. One not confirmed
        // starts again.
        let mut st = inner.state();
        st.sent = 0;
        st.ended = 0;
        st.dirty = true;
        for t in st.tags.values_mut() {
            t.sub = 0;
        }
    }
    loop {
        if inner.stop.load(Ordering::SeqCst) {
            let _ = tx.send(Ws::Close(None)).await;
            return;
        }
        let sub = {
            let mut st = inner.state();
            if std::mem::take(&mut st.dirty) {
                st.sent += 1;
                let (n, high) = (st.sent, st.high);
                for t in st.tags.values_mut().filter(|t| !t.confirmed && t.sub == 0) {
                    t.since = high;
                    t.sub = n;
                }
                let mut tags: Vec<String> = st.tags.keys().cloned().collect();
                tags.sort();
                // v:1 — a tag whose backlog the relay no longer holds from `since` is said with
                // a `Gap`, and every tag is then drained at once (condition 6).
                Some(Frame::Sub { tags, since: high, v: 1 })
            } else {
                None
            }
        };
        if let Some(f) = sub {
            if tx.send(Ws::Text(f.to_json())).await.is_err() {
                return;
            }
        }
        tokio::select! {
            m = rx.next() => match m {
                Some(Ok(Ws::Text(t))) => on_frame(inner, &t),
                Some(Ok(Ws::Binary(b))) => on_frame(inner, &String::from_utf8_lossy(&b)),
                Some(Ok(Ws::Ping(p))) => {
                    if tx.send(Ws::Pong(p)).await.is_err() {
                        return;
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(_)) | None => return,
            },
            _ = inner.wake.notified() => {}
            _ = tokio::time::sleep(KEEPALIVE) => {
                if tx.send(Ws::Ping(Vec::new())).await.is_err() {
                    return;
                }
            }
        }
    }
}

fn on_frame(inner: &Inner, text: &str) {
    let Ok(frame) = Frame::from_json(text) else { return };
    match frame {
        Frame::Msg { tag, seq, blob } => {
            {
                let mut st = inner.state();
                st.high = st.high.max(seq);
                if !st.tags.contains_key(&tag) {
                    return;
                }
                if st.overflowed || st.bytes + blob.len() > QUEUE_BYTES {
                    st.overflowed = true;
                    st.queue.clear();
                    st.bytes = 0;
                } else {
                    st.bytes += blob.len();
                    st.queue.push(Delivered { tag, seq, blob });
                }
            }
            inner.heard.fetch_add(1, Ordering::SeqCst);
            (inner.ring)();
        }
        // THE RELAY NO LONGER HOLDS WHAT A TAG OWED (its retention floor is above `since`):
        // as a dropped queue, so no tag is complete until drained, and the owner is rung now.
        Frame::Gap { .. } => {
            {
                let mut st = inner.state();
                st.overflowed = true;
                st.queue.clear();
                st.bytes = 0;
            }
            (inner.ring)();
        }
        Frame::Eose => {
            {
                let mut st = inner.state();
                st.ended += 1;
                let ended = st.ended;
                for t in st.tags.values_mut().filter(|t| !t.confirmed && t.sub != 0 && t.sub <= ended) {
                    t.confirmed = true;
                }
            }
            (inner.ring)();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pacific_wire::address::Address;
    use tokio::net::TcpListener;

    fn relay() -> String {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
            rt.block_on(async move {
                let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
                tx.send(l.local_addr().unwrap()).unwrap();
                relay::serve(l).await
            });
        });
        format!("ws://{}", rx.recv().unwrap())
    }

    /// A relay that keeps `max_per_tag` blobs a tag, swept every 50 ms.
    fn relay_keeping(max_per_tag: u64) -> String {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
            rt.block_on(async move {
                let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
                tx.send(l.local_addr().unwrap()).unwrap();
                let keep = relay::Retention { window_us: 0, max_per_tag, sweep: Duration::from_millis(50) };
                relay::serve_with_mailbox(l, relay::mailbox_from_env(), keep, None).await
            });
        });
        format!("ws://{}", rx.recv().unwrap())
    }

    fn publish(url: &str, at: &Address, blob: &[u8]) -> u64 {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let mut s = RelaySession::connect(url).await.unwrap();
            let seq = s.publish(at, pacific_wire::blob_b64(blob)).await.unwrap();
            s.close().await;
            seq
        })
    }

    fn until(what: impl Fn() -> bool) -> bool {
        let end = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < end {
            if what() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    /// FC-14, THE RETENTION FLOOR: a tag whose backlog from `since` the relay no longer holds
    /// is said with a `Gap`, and the connection reports it as a dropped queue, so every tag is
    /// drained at once rather than trusted (condition 6).
    #[test]
    fn a_backlog_the_relay_dropped_is_said_as_a_dropped_queue() {
        let url = relay_keeping(1);
        let a = Address::from_seed(&[3u8; 32]);
        for b in [&b"one"[..], b"two", b"three"] {
            publish(&url, &a, b);
        }
        std::thread::sleep(Duration::from_millis(400));
        let rang = Arc::new(AtomicU64::new(0));
        let r = rang.clone();
        let live = Live::start(url, 0, Box::new(move || {
            r.fetch_add(1, Ordering::SeqCst);
        }));
        live.set_tags([a.tag_hex()]);
        assert!(until(|| live.pending()), "the Gap is heard");
        assert!(live.take().overflowed, "and reported as a dropped queue");
        assert!(rang.load(Ordering::SeqCst) >= 1, "and rung");
    }

    /// A blob published after the Sub is confirmed is delivered, rung, and on its tag only;
    /// the set follows `set_tags`; a tag added later is complete from its own `since`.
    #[test]
    fn it_hears_what_the_relay_stores_on_the_tags_it_holds() {
        let url = relay();
        let (a, b) = (Address::from_seed(&[1u8; 32]), Address::from_seed(&[2u8; 32]));
        let early = publish(&url, &a, b"before");
        let rang = Arc::new(AtomicU64::new(0));
        let r = rang.clone();
        let live = Live::start(url.clone(), early, Box::new(move || {
            r.fetch_add(1, Ordering::SeqCst);
        }));
        live.set_tags([a.tag_hex()]);
        assert!(live.wait_confirmed(Duration::from_secs(10)), "the Sub is confirmed");
        assert_eq!(live.since(&a.tag_hex()), Some(early), "complete from after what the caller held");
        assert!(live.take().frames.is_empty(), "nothing at or before `after` is replayed");

        let s1 = publish(&url, &a, b"one");
        publish(&url, &b, b"not ours");
        assert!(until(|| live.pending()));
        std::thread::sleep(Duration::from_millis(100));
        let got = live.take();
        assert_eq!(got.frames.iter().map(|f| f.seq).collect::<Vec<_>>(), vec![s1], "its own tag, once");
        assert!(rang.load(Ordering::SeqCst) >= 2, "rung for the Eose and the frame");

        live.set_tags([a.tag_hex(), b.tag_hex()]);
        assert!(live.wait_confirmed(Duration::from_secs(10)));
        let since_b = live.since(&b.tag_hex()).unwrap();
        assert!(since_b >= s1, "a tag added later is complete from the Sub that added it");
        assert_eq!(live.since(&a.tag_hex()), Some(early), "a tag kept keeps its since");
        let s2 = publish(&url, &b, b"two");
        assert!(until(|| live.take().frames.iter().any(|f| f.seq == s2)));
    }
}
