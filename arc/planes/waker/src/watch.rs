//! The relay client — the half of the waker that makes it a THIRD TIER rather than a
//! change to the relay.
//!
//! THE WHOLE IDEA: the waker subscribes for the tags it was asked to watch exactly like a
//! phone does. It speaks the ordinary `Frame` protocol over an ordinary WebSocket, and
//! the relay cannot tell it apart from any other subscriber — so the relay learns NOTHING
//! it did not already know, and in particular never learns a push token. When a blob
//! lands on a watched tag the waker looks up who asked to be woken for that tag and sends
//! a content-free push. It never decrypts (it holds no key and the blobs are sealed),
//! never sees a group id, and never names a sender. See RINGFENCE.txt §6.
//!
//! WHY POLL THE STORE. Registrations change constantly (tags are per-(group, epoch), and
//! the app re-registers its current set every sync tick), and the relay's `Sub` REPLACES a
//! connection's subscriptions rather than adding to them. Re-reading `tags_watched()`
//! every [`DEFAULT_POLL_MS`] and re-SUBbing when the set actually changed is the simplest
//! thing that is also robust: no notification plumbing between the HTTP handler and this
//! loop, no way to leak a subscription, and a restart converges on its own.
//!
//! THE CURSOR. Relay `seq` is a wall-clock microsecond stamp, so a fresh waker SUBs from
//! `since = now` — it wakes on NEW traffic and never replays a backlog into a burst of
//! buzzes for messages the phone read days ago. Across a reconnect it resumes from the
//! highest seq it saw, so a blob that landed during the drop is not missed.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use pacific_wire::Frame;
use tokio_tungstenite::tungstenite::Message;

use crate::apns::{PushOutcome, Pusher};
use crate::store::{Store, STALE_AFTER_MS};

/// At most one push per (token, tag) in this window: a burst of deltas — a sender typing
/// three messages, a commit plus its application messages — is ONE buzz.
pub const DEFAULT_DEDUPE_MS: u64 = 10_000;
/// How often the registration set is re-read (and stale rows pruned).
pub const DEFAULT_POLL_MS: u64 = 15_000;
/// The public relay this plane watches when nothing says otherwise. arcup overrides it
/// with the Arc's own local relay.
pub const DEFAULT_RELAY_URL: &str = "wss://arc.wallflowers.io/v1/relay";
/// Reconnect backoff ceiling. A relay outage must not become a hot loop.
const BACKOFF_MAX_S: u64 = 30;

#[derive(Debug, Clone)]
pub struct WatchConfig {
    pub relay_url: String,
    pub dedupe_ms: u64,
    pub poll_ms: u64,
    /// Registrations not refreshed within this window are pruned (tag rotation GC).
    pub stale_after_ms: i64,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            relay_url: DEFAULT_RELAY_URL.to_string(),
            dedupe_ms: DEFAULT_DEDUPE_MS,
            poll_ms: DEFAULT_POLL_MS,
            stale_after_ms: STALE_AFTER_MS,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error("relay socket: {0}")]
    Socket(String),
    #[error("store: {0}")]
    Store(#[from] rusqlite::Error),
}

pub struct Watcher {
    store: Arc<Store>,
    /// `None` = no APNs credential. The loop still runs (so a misconfiguration is visible
    /// as "would have woken N devices" rather than silence), it just cannot push.
    pusher: Option<Arc<dyn Pusher>>,
    cfg: WatchConfig,
    /// (token, tag) → last push ms. The one-buzz gate.
    dedupe: std::sync::Mutex<HashMap<(String, String), u64>>,
    /// Highest relay `seq` seen — the resume point across reconnects.
    cursor: AtomicU64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Relay `seq` is microseconds since the epoch (relay `Hub::next_seq`), so this is the
/// "everything from here on" cursor a fresh subscriber starts at.
fn now_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// Exponential backoff, capped. Pure so the shape is pinned by a test rather than by
/// watching a production log.
fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(2u64.saturating_pow(attempt.min(16)).min(BACKOFF_MAX_S))
}

impl Watcher {
    pub fn new(store: Arc<Store>, pusher: Option<Arc<dyn Pusher>>, cfg: WatchConfig) -> Self {
        Self {
            store,
            pusher,
            cfg,
            dedupe: std::sync::Mutex::new(HashMap::new()),
            cursor: AtomicU64::new(0),
        }
    }

    /// Forever: connect, watch, reconnect with backoff. Never returns.
    pub async fn run(self: Arc<Self>) {
        let mut attempt = 0u32;
        loop {
            match self.clone().session().await {
                Ok(()) => {
                    eprintln!("waker: relay closed the connection — reconnecting");
                    attempt = 0;
                }
                Err(e) => {
                    eprintln!("waker: relay session ended: {e}");
                    attempt = attempt.saturating_add(1);
                }
            }
            tokio::time::sleep(backoff(attempt)).await;
        }
    }

    /// ONE connection: subscribe, then pump until the socket closes.
    pub async fn session(self: Arc<Self>) -> Result<(), WatchError> {
        let (ws, _) = tokio_tungstenite::connect_async(&self.cfg.relay_url)
            .await
            .map_err(|e| WatchError::Socket(e.to_string()))?;
        let (mut tx, mut rx) = ws.split();

        // First connection of this process starts at NOW; a reconnect resumes from what we
        // already saw, so the gap is covered without replaying history.
        if self.cursor.load(Ordering::Relaxed) == 0 {
            self.cursor.store(now_micros(), Ordering::Relaxed);
        }
        let mut tags = self.store.tags_watched()?;
        eprintln!(
            "waker: subscribed to {} tag(s) on {}",
            tags.len(),
            self.cfg.relay_url
        );
        let sub = Frame::Sub {
            tags: tags.clone(),
            since: self.cursor.load(Ordering::Relaxed),
            // v = 0 on purpose. The waker does not reconstruct anyone's history — it
            // watches for the ARRIVAL of a blob and turns that into a push. A gap in
            // the backlog is not actionable here (there is no one to tell and nothing
            // to re-fetch); the device that owns the tag learns about it from its own
            // drain. Asking for `Gap` would only add a frame this loop must skip.
            v: 0,
        };
        tx.send(Message::Text(sub.to_json()))
            .await
            .map_err(|e| WatchError::Socket(e.to_string()))?;

        let period = Duration::from_millis(self.cfg.poll_ms);
        let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        loop {
            tokio::select! {
                incoming = rx.next() => {
                    let Some(msg) = incoming else { return Ok(()) };
                    match msg {
                        Ok(Message::Text(t)) => match Frame::from_json(&t) {
                            // The ONLY frame that matters. `blob` is deliberately not
                            // bound: it is sealed, and this plane has no business touching
                            // even the ciphertext.
                            Ok(Frame::Msg { tag, seq, .. }) => {
                                self.cursor.fetch_max(seq, Ordering::Relaxed);
                                self.wake(&tag);
                            }
                            Ok(_) => {}      // Eose / Ack — nothing for a watcher to do
                            Err(e) => eprintln!("waker: unparsable relay frame: {e}"),
                        },
                        Ok(Message::Close(_)) => return Ok(()),
                        Ok(_) => {}          // ping/pong/binary — tungstenite answers pings
                        Err(e) => return Err(WatchError::Socket(e.to_string())),
                    }
                }
                _ = ticker.tick() => {
                    // Rotation GC first, so a pruned tag leaves the Sub set in the same pass.
                    let cutoff = now_ms() as i64 - self.cfg.stale_after_ms;
                    match self.store.prune_stale(cutoff) {
                        Ok(n) if n > 0 => eprintln!("waker: pruned {n} stale registration(s)"),
                        Ok(_) => {}
                        Err(e) => eprintln!("waker: prune failed: {e}"),
                    }
                    let current = self.store.tags_watched()?;
                    if current != tags {
                        tags = current;
                        eprintln!("waker: registration set changed — re-SUB ({} tags)", tags.len());
                        let sub = Frame::Sub {
                            tags: tags.clone(),
                            since: self.cursor.load(Ordering::Relaxed),
                            v: 0, // as above: arrivals, not history
                        };
                        tx.send(Message::Text(sub.to_json()))
                            .await
                            .map_err(|e| WatchError::Socket(e.to_string()))?;
                    }
                }
            }
        }
    }

    /// Which devices are DUE a push for this tag right now — the dedupe decision, taken
    /// synchronously so two blobs landing in the same millisecond cannot both pass it.
    fn due_devices(&self, tag: &str, now: u64) -> Vec<String> {
        let devices = match self.store.tokens_for_tag(tag) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("waker: tokens_for_tag failed: {e}");
                return Vec::new();
            }
        };
        let mut seen = self.dedupe.lock().expect("dedupe mutex");
        // Keep the map from growing without bound; anything older than a few windows can
        // never suppress anything.
        seen.retain(|_, t| now.saturating_sub(*t) < self.cfg.dedupe_ms.saturating_mul(4));
        let mut due = Vec::new();
        for d in devices {
            // APNs is the only credential this plane holds. Another platform's rows are
            // stored (so the day an FCM tier exists nothing has to re-register) and
            // skipped, never pushed at.
            if d.platform != "ios" {
                continue;
            }
            let key = (d.token.clone(), tag.to_string());
            let fresh = seen
                .get(&key)
                .map(|last| now.saturating_sub(*last) < self.cfg.dedupe_ms)
                .unwrap_or(false);
            if fresh {
                continue;
            }
            seen.insert(key, now);
            due.push(d.token);
        }
        due
    }

    /// Wake everyone registered for `tag`. Detached, so a slow APNs round-trip never
    /// stalls the read loop and blinds the plane to the next blob.
    fn wake(self: &Arc<Self>, tag: &str) {
        let due = self.due_devices(tag, now_ms());
        if due.is_empty() {
            return;
        }
        let Some(pusher) = self.pusher.clone() else {
            // LOUD, per the fail-loud posture: registration works without a credential, so
            // this is the line that says the wake did not.
            eprintln!(
                "waker: {} device(s) due a wake but NO APNs CREDENTIAL is configured — \
                 nothing was sent (see /v1/wake/health: waker:false)",
                due.len()
            );
            return;
        };
        let store = self.store.clone();
        tokio::spawn(async move {
            for token in due {
                match pusher.push(&token).await {
                    PushOutcome::Delivered => {}
                    // The install is gone. Deleting is not an optimisation — Apple treats
                    // continued pushes to an unregistered token as abuse.
                    PushOutcome::Unregistered => match store.forget_token(&token) {
                        Ok(n) => {
                            eprintln!("waker: token unregistered by APNs — dropped {n} row(s)")
                        }
                        Err(e) => eprintln!("waker: could not forget token: {e}"),
                    },
                    PushOutcome::Throttled => {
                        // Apple is shedding load; the registration is fine. Give the next
                        // token in this batch a moment rather than hammering.
                        eprintln!("waker: APNs throttled — backing off");
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                    // apns.rs has already logged the diagnosis for these.
                    PushOutcome::Failed { .. } => {}
                    PushOutcome::Transport(e) => eprintln!("waker: APNs transport error: {e}"),
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;

    // -- a fake relay, on loopback, exactly like planes/relay/tests/smoke.rs ------------

    /// Accepts connections forever. On each connection it reports every `Sub` it receives
    /// down `subs` as `(tags, since)`, answers the FIRST one with `Eose` + the scripted
    /// frames, and (if `close_after`) hangs up — which is how the reconnect/cursor
    /// behaviour is driven.
    struct FakeRelay {
        addr: SocketAddr,
        subs: mpsc::UnboundedReceiver<(Vec<String>, u64)>,
    }

    impl FakeRelay {
        async fn spawn(reply: Vec<Frame>, close_after: bool) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let (subs_tx, subs) = mpsc::unbounded_channel();
            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let reply = reply.clone();
                    let subs_tx = subs_tx.clone();
                    tokio::spawn(async move {
                        let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                            return;
                        };
                        let mut first = true;
                        while let Some(Ok(msg)) = ws.next().await {
                            let Message::Text(t) = msg else { continue };
                            let Ok(Frame::Sub { tags, since, .. }) = Frame::from_json(&t) else {
                                continue;
                            };
                            let _ = subs_tx.send((tags, since));
                            if !first {
                                continue;
                            }
                            first = false;
                            let _ = ws.send(Message::Text(Frame::Eose.to_json())).await;
                            for f in &reply {
                                let _ = ws.send(Message::Text(f.to_json())).await;
                            }
                            if close_after {
                                let _ = ws.close(None).await;
                                return;
                            }
                        }
                    });
                }
            });
            Self { addr, subs }
        }

        fn url(&self) -> String {
            format!("ws://{}", self.addr)
        }
    }

    // -- a recording pusher: the seam that keeps every test off the network -------------

    struct Recorder {
        calls: std::sync::Mutex<Vec<String>>,
        /// The one token Apple answers 410 for; every other token is delivered.
        dead: Option<String>,
    }

    impl Recorder {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: std::sync::Mutex::new(Vec::new()),
                dead: None,
            })
        }
        fn with_dead(token: &str) -> Arc<Self> {
            Arc::new(Self {
                calls: std::sync::Mutex::new(Vec::new()),
                dead: Some(token.to_string()),
            })
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl Pusher for Recorder {
        async fn push(&self, token: &str) -> PushOutcome {
            self.calls.lock().unwrap().push(token.to_string());
            if self.dead.as_deref() == Some(token) {
                PushOutcome::Unregistered
            } else {
                PushOutcome::Delivered
            }
        }
        fn environment(&self) -> &'static str {
            "test"
        }
    }

    /// Poll a condition to a deadline — the pushes are detached, so the assertion has to
    /// wait for them rather than assume a scheduling order.
    async fn until(mut f: impl FnMut() -> bool) -> bool {
        for _ in 0..200 {
            if f() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        f()
    }

    fn cfg(url: String) -> WatchConfig {
        WatchConfig {
            relay_url: url,
            dedupe_ms: DEFAULT_DEDUPE_MS,
            // Fast poll so the re-SUB behaviour is testable in milliseconds.
            poll_ms: 30,
            stale_after_ms: STALE_AFTER_MS,
        }
    }

    fn msg(tag: &str, seq: u64) -> Frame {
        Frame::Msg {
            tag: tag.into(),
            seq,
            blob: "c2VhbGVk".into(), // "sealed" — never read by this plane
        }
    }

    fn store_with(rows: &[(&str, &str)]) -> Arc<Store> {
        let s = Store::open_in_memory().unwrap();
        for (token, tag) in rows {
            s.upsert_registration(token, tag, "ios", now_ms() as i64)
                .unwrap();
        }
        Arc::new(s)
    }

    /// THE PLANE, END TO END (minus Apple): the waker SUBs for the registered tag, a blob
    /// lands on it, and the tokens registered for that tag are pushed — once each.
    #[tokio::test]
    async fn blob_on_a_watched_tag_wakes_its_tokens() {
        let mut relay = FakeRelay::spawn(vec![msg("tag1", 100)], false).await;
        let store = store_with(&[("tok_a", "tag1"), ("tok_b", "tag1"), ("tok_c", "other")]);
        let rec = Recorder::new();
        let w = Arc::new(Watcher::new(store, Some(rec.clone()), cfg(relay.url())));
        tokio::spawn(w.session());

        let (subbed, since) = relay.subs.recv().await.expect("a Sub must arrive");
        assert_eq!(subbed, vec!["other", "tag1"], "SUBs the whole watched set");
        assert!(
            since > 0,
            "a fresh waker starts at NOW, never replays the backlog"
        );
        assert!(until(|| rec.calls().len() == 2).await, "both tokens woken");
        let mut calls = rec.calls();
        calls.sort();
        assert_eq!(calls, vec!["tok_a", "tok_b"]);
        assert!(
            !calls.contains(&"tok_c".to_string()),
            "a device registered for a DIFFERENT tag is not woken"
        );
    }

    /// ONE BUZZ. Three blobs on the same tag inside the dedupe window is a single push per
    /// device — a burst of deltas must not be a burst of vibrations.
    #[tokio::test]
    async fn a_burst_is_one_buzz() {
        let mut relay = FakeRelay::spawn(
            vec![msg("tag1", 101), msg("tag1", 102), msg("tag1", 103)],
            false,
        )
        .await;
        let store = store_with(&[("tok_a", "tag1")]);
        let rec = Recorder::new();
        let w = Arc::new(Watcher::new(store, Some(rec.clone()), cfg(relay.url())));
        tokio::spawn(w.session());
        relay.subs.recv().await.unwrap();

        assert!(until(|| rec.calls().len() == 1).await, "at least one push");
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(rec.calls(), vec!["tok_a"], "and exactly one, not three");
    }

    /// A different TAG is a different conversation — the dedupe is per (token, tag), so a
    /// second group waking the same device still gets through.
    #[tokio::test]
    async fn dedupe_is_per_tag_not_per_token() {
        let mut relay = FakeRelay::spawn(vec![msg("tag1", 101), msg("tag2", 102)], false).await;
        let store = store_with(&[("tok_a", "tag1"), ("tok_a", "tag2")]);
        let rec = Recorder::new();
        let w = Arc::new(Watcher::new(store, Some(rec.clone()), cfg(relay.url())));
        tokio::spawn(w.session());
        relay.subs.recv().await.unwrap();
        assert!(until(|| rec.calls().len() == 2).await, "one push per tag");
    }

    /// APNs 410: the token is dead and every one of its registrations goes with it.
    #[tokio::test]
    async fn unregistered_token_is_forgotten() {
        let mut relay = FakeRelay::spawn(vec![msg("tag1", 101)], false).await;
        let store = store_with(&[
            ("tok_dead", "tag1"),
            ("tok_dead", "tag2"),
            ("tok_live", "tag1"),
        ]);
        let rec = Recorder::with_dead("tok_dead");
        let w = Arc::new(Watcher::new(
            store.clone(),
            Some(rec.clone()),
            cfg(relay.url()),
        ));
        tokio::spawn(w.session());
        relay.subs.recv().await.unwrap();

        assert!(
            until(|| store.tags_for_token("tok_dead").unwrap() == 0).await,
            "a 410 must delete the token's registrations"
        );
        assert_eq!(
            store.tags_for_token("tok_live").unwrap(),
            1,
            "the other device on that tag is untouched"
        );
    }

    /// NO CREDENTIAL: the loop still runs and the session stays healthy — the wake is
    /// skipped loudly, nothing crashes, and no registration is lost.
    #[tokio::test]
    async fn without_a_pusher_the_session_survives() {
        let mut relay = FakeRelay::spawn(vec![msg("tag1", 101)], false).await;
        let store = store_with(&[("tok_a", "tag1")]);
        let w = Arc::new(Watcher::new(store.clone(), None, cfg(relay.url())));
        let h = tokio::spawn(w.session());
        relay.subs.recv().await.unwrap();
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert!(!h.is_finished(), "the session must not die without APNs");
        assert_eq!(store.tags_for_token("tok_a").unwrap(), 1);
        h.abort();
    }

    /// A new registration must reach the relay WITHOUT a restart: the poll notices the
    /// changed set and re-SUBs with the full set (the relay's Sub replaces, not adds).
    #[tokio::test]
    async fn registration_change_triggers_a_resub() {
        let mut relay = FakeRelay::spawn(vec![], false).await;
        let store = store_with(&[("tok_a", "tag1")]);
        let w = Arc::new(Watcher::new(store.clone(), None, cfg(relay.url())));
        tokio::spawn(w.session());
        assert_eq!(relay.subs.recv().await.unwrap().0, vec!["tag1"]);

        store
            .upsert_registration("tok_a", "tag2", "ios", now_ms() as i64)
            .unwrap();
        let (second, _) = relay.subs.recv().await.expect("a second Sub must arrive");
        assert_eq!(
            second,
            vec!["tag1", "tag2"],
            "the re-SUB carries the WHOLE set — the relay replaces subscriptions"
        );
    }

    /// The cursor: a fresh waker starts at "now" (no backlog replay = no buzz storm), and
    /// a reconnect resumes from the highest seq it saw, so the gap is covered.
    #[tokio::test]
    async fn cursor_starts_at_now_and_resumes_after_a_drop() {
        let seq = now_micros() + 5_000_000;
        let mut relay = FakeRelay::spawn(vec![msg("tag1", seq)], true).await;
        let store = store_with(&[("tok_a", "tag1")]);
        let rec = Recorder::new();
        let w = Arc::new(Watcher::new(store, Some(rec.clone()), cfg(relay.url())));

        w.clone()
            .session()
            .await
            .expect("first session ends at the close");
        let (first, since) = relay.subs.recv().await.unwrap();
        assert_eq!(first, vec!["tag1"]);
        assert!(since > 0 && since < seq, "started at now, before this blob");
        assert!(until(|| rec.calls().len() == 1).await);

        // Reconnect: the Sub must now carry the seq we saw, not zero and not a new "now".
        w.clone().session().await.ok();
        let (_, resumed) = relay.subs.recv().await.unwrap();
        assert_eq!(resumed, seq, "the reconnect resumes from the last seq seen");
    }

    /// A platform this plane holds no credential for is stored and SKIPPED, never pushed.
    #[tokio::test]
    async fn non_ios_registrations_are_not_pushed() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_registration("tok_android", "tag1", "android", now_ms() as i64)
            .unwrap();
        s.upsert_registration("tok_ios", "tag1", "ios", now_ms() as i64)
            .unwrap();
        let w = Watcher::new(Arc::new(s), None, WatchConfig::default());
        assert_eq!(w.due_devices("tag1", now_ms()), vec!["tok_ios"]);
    }

    /// The dedupe window itself, without a socket: inside it suppresses, past it releases.
    #[tokio::test]
    async fn dedupe_window_boundaries() {
        let store = store_with(&[("tok_a", "tag1")]);
        let w = Watcher::new(store, None, WatchConfig::default());
        let t0 = 1_000_000_u64;
        assert_eq!(w.due_devices("tag1", t0), vec!["tok_a"]);
        assert!(w.due_devices("tag1", t0 + DEFAULT_DEDUPE_MS - 1).is_empty());
        assert_eq!(
            w.due_devices("tag1", t0 + DEFAULT_DEDUPE_MS),
            vec!["tok_a"],
            "past the window the next blob buzzes again"
        );
    }

    /// A relay outage must not become a hot loop, and must not back off into next week.
    #[test]
    fn backoff_is_exponential_and_capped() {
        assert_eq!(backoff(0), Duration::from_secs(1));
        assert_eq!(backoff(1), Duration::from_secs(2));
        assert_eq!(backoff(4), Duration::from_secs(16));
        assert_eq!(backoff(5), Duration::from_secs(BACKOFF_MAX_S));
        assert_eq!(backoff(u32::MAX), Duration::from_secs(BACKOFF_MAX_S));
    }
}
