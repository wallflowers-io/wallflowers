//! Semaphore — the Pacific blind relay: a dumb, store-and-forward WebSocket mailbox.
//!
//! It speaks the `crate::wire::Frame` protocol over plain `ws` (TLS is terminated at
//! the tunnel edge, at arc.kenjin.cc — so clients reach it as `wss://`). It knows NOTHING: tags
//! and blobs are opaque strings. `Pub` stores a blob under a tag and pushes it to
//! that tag's subscribers; `Sub` replays the backlog since a cursor, emits `Eose`,
//! then streams live.
//!
//! In-memory store for M1 (a sled/sqlite-backed store with TTL slots in here for
//! M2 — see the `Hub` TODO). `serve(listener)` runs the accept loop forever.
//!
//! ## Telemetry privacy invariant
//!
//! This relay is *blind*, and so is its telemetry. The ops plane is instrumented
//! richly (connection lifetimes, publish/drain latencies, counters), but the
//! content/routing plane is NEVER observed. No tag, blob, frame body, client IP, or
//! `SocketAddr` is ever passed into a `tracing` field/message, a span field, or any
//! Sentry call. The only per-connection identifier that reaches telemetry is the
//! opaque, process-local `ConnId` (a `u64`) — it carries no routing meaning.
//!
//! ## Traffic observation seam (NOT telemetry)
//!
//! [`serve_with_observer`] accepts an OPTIONAL in-process [`Observer`] (an mpsc
//! sender of [`RelayEvent`]). It is the ONLY path by which frame-level data —
//! including tags — leaves the relay, and it is wired up SOLELY by the local
//! `pacific-traffic` dev/visualisation tool. The production binary calls [`serve`]
//! (observer = `None`), so with no observer not a single tag or blob is ever
//! emitted anywhere: the blind-telemetry invariant above is fully intact in
//! production. The observer never touches `tracing` or Sentry.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info, info_span, warn, Instrument};

pub mod wire;
/// THE storage interface. One trait; the relay talks to nothing else.
pub mod mailbox;
// What the relay accepts and how fast: the v1 admission posture (see its header).
pub mod limits;
pub use limits::Limits;
/// The DURABLE mailbox behind the blind interface — persistence, retention, floors.
pub mod store;
/// A store for blob BODIES, and the R2 implementation of it.
pub mod blobs;
/// The R2 backend: SQLite index, object-storage bodies, and why not pure R2.
pub mod offload;
/// The storage safety net — refuses uploads before R2 leaves the free tier.
pub mod budget;
/// Large-payload offload — presigned URLs to S3-compatible object storage.
pub mod media;
/// The SigV4 presigner behind [`media`]. Pure string construction, no I/O.
pub mod sigv4;
use crate::mailbox::{Appended, Mailbox};
use crate::media::{Media, MediaError};
use crate::store::Store;
use crate::wire::{may_send_to, may_service, Dir, Frame, V_BASE, V_GAP};

type ConnId = u64;

/// An in-process, traffic-only sink for relay observations. See the module doc:
/// production ([`serve`]) never constructs one.
pub type Observer = mpsc::UnboundedSender<RelayEvent>;

/// A frame-level relay observation for the local traffic. This is NOT telemetry —
/// it never reaches `tracing`/Sentry — and is emitted only when an [`Observer`] is
/// wired by `pacific-traffic`.
#[derive(Clone, Debug)]
pub enum RelayEvent {
    ConnOpened { conn: u64 },
    ConnClosed { conn: u64 },
    Published {
        conn: u64,
        tag: String,
        blob_len: usize,
        seq: u64,
        fanout: u64,
    },
    Subscribed {
        conn: u64,
        tags: Vec<String>,
        since: u64,
        replayed: u64,
    },
    Snapshot(RelaySnapshot),
}

/// A point-in-time view of the mailbox for the traffic's "stack" panel.
#[derive(Clone, Debug, Default)]
pub struct RelaySnapshot {
    pub active_conns: u64,
    pub total_publishes: u64,
    pub total_deliveries: u64,
    pub tags: Vec<TagStat>,
}

/// Per-tag cardinalities: stored blobs and live subscribers.
#[derive(Clone, Debug)]
pub struct TagStat {
    pub tag: String,
    pub blobs: u64,
    pub subs: u64,
}

/// Process-lifetime operational counters. These are pure cardinalities and totals —
/// never a tag, blob, or address. Safe to emit to logs and Sentry.
#[derive(Default)]
struct Metrics {
    /// Connections accepted minus connections closed (live sockets).
    active_conns: AtomicU64,
    /// Total `Pub` frames stored over the process lifetime.
    total_publishes: AtomicU64,
    /// Total `Msg` frames fanned out to subscribers (live + backlog replay).
    total_deliveries: AtomicU64,
    /// Total inbound frames that failed to parse and were dropped.
    bad_frames: AtomicU64,
    /// Commit-flagged publishes rejected because the tag's slot was taken —
    /// each one is a commit race a client lost (a pure count, no routing data).
    rejected_commits: AtomicU64,
    /// Store reads/writes that failed. Non-zero means blobs were refused (never
    /// silently dropped) — page on it.
    store_errors: AtomicU64,
    /// `Gap` frames sent. Each one is a subscriber that had fallen below the
    /// retention floor: the honest count of how often the window is too short.
    gaps_announced: AtomicU64,
    /// Media URLs minted. A count of capabilities issued — never a key or a URL.
    media_presigned: AtomicU64,
    /// Media presigns refused (unconfigured / bad key / oversized). Persistently
    /// non-zero means clients are asking for something this deploy cannot do.
    media_refused: AtomicU64,
    /// Client requests DROPPED IN SILENCE because they sit above the vocabulary the
    /// connection declared in `Sub.v` — today, a media frame from a connection that
    /// never said `v >= 2`.
    ///
    /// Not a refusal: a refusal would be a `MediaErr`, which is itself a level-2
    /// frame and would be exactly the unknown frame that kills the client's drain.
    /// Non-zero means something is speaking a vocabulary it has not claimed to be
    /// able to read the answers in — a client bug, and this counter is the only
    /// place it is visible.
    gated_requests: AtomicU64,
    /// Publishes REFUSED — a bad or missing signature, too large, over a pace or
    /// the ingest ceiling, or a full store. Not a lost commit race (that is
    /// `rejected_commits`). A count, never a tag.
    refused_publishes: AtomicU64,
    /// Publishes of a blob already stored at that tag: acked with the original seq,
    /// stored and fanned out never. Persistently high is a client retry loop or
    /// someone replaying captured publishes, which this is what makes harmless.
    duplicate_publishes: AtomicU64,
}

/// How long the mailbox keeps a blob, and how much of it.
///
/// THIS IS A PRODUCT DECISION WEARING OPS CLOTHES. The relay is blind: it cannot
/// know whether a subscriber drained, so it can never expire on "everyone got it"
/// — only on age. An age window is therefore not a storage setting, it is a
/// deadline on being offline, and there is none by default (ruled 13 Sep 2026):
/// a phone left off for a week comes back to all of its mail.
#[derive(Clone, Copy, Debug)]
pub struct Retention {
    /// Age past which a blob is dropped, in microseconds. 0 disables age-based
    /// eviction entirely (the store then grows without bound).
    pub window_us: u64,
    /// Cap on stored blobs per tag; the oldest above the cap go. 0 = uncapped.
    pub max_per_tag: u64,
    /// How often the eviction sweep runs.
    pub sweep: Duration,
}

/// NOTHING EXPIRES: no age window, uncapped per tag.
///
/// This replaces a 7-day window that was never ruled on (SCALE-UNBLOCKS O-1) and
/// should not have been the default while unruled. A week is an ordinary gap — a
/// phone in a drawer, a trip, a flat battery — and a mailbox that deletes the
/// messages a device has not collected yet is broken rather than tuned. The disk
/// it was saving was never measured to be short.
///
/// Keeping everything is the ORDINARY case and not a hazard: no production
/// database drops rows for being old, and this one holds less than most — sealed
/// blobs, bounded by what people actually send. Growth is a capacity question,
/// answered by provisioning the volume and watching it.
///
/// If storage ever does need giving back, the lever is a member-driven delete once
/// a delta has provably reached everyone — only members can know that, which is
/// exactly what a blind relay cannot — never a clock the relay reads alone.
///
/// The eviction machinery is unchanged and stays tested for a deploy that sets a
/// limit anyway, including the property that matters whenever it IS on: a client
/// below a tag's floor is told (`Frame::Gap`) rather than handed a short replay it
/// cannot tell from a whole one.
///
/// `sweep` keeps its five minutes so a relay that opts back in runs the cadence it
/// always did; with both limits at 0 the sweep never starts at all.
impl Default for Retention {
    fn default() -> Self {
        Self {
            window_us: 0,
            max_per_tag: 0,
            sweep: Duration::from_secs(300),
        }
    }
}

impl Retention {
    /// Read the policy from the environment, falling back to [`Default`] per field
    /// — which is to keep everything. `RELAY_RETENTION_SECS` is how an operator OPTS
    /// IN to an age window; 0, the default, means "never expire by age".
    pub fn from_env() -> Self {
        let d = Self::default();
        let secs = env_u64("RELAY_RETENTION_SECS").map(|s| s.saturating_mul(1_000_000));
        Self {
            window_us: secs.unwrap_or(d.window_us),
            max_per_tag: env_u64("RELAY_MAX_BLOBS_PER_TAG").unwrap_or(d.max_per_tag),
            sweep: env_u64("RELAY_SWEEP_SECS")
                .map(Duration::from_secs)
                .unwrap_or(d.sweep),
        }
    }
}

fn env_u64(k: &str) -> Option<u64> {
    std::env::var(k).ok()?.trim().parse().ok()
}

/// The shared mailbox. The store is DURABLE (see [`store`]): blobs and commit
/// slots both survive the process, which is what stops a redeploy from dropping
/// undrained mail AND what stops it from re-opening a commit slot that a group
/// has already used.
///
/// WHICH store is a deployment choice as of 18 Sep 2026 — the hub holds a
/// [`Mailbox`], not a SQLite handle. Today that is either `store::Store` (SQLite,
/// the default) or `offload::OffloadMailbox` (SQLite index, bodies in R2). The hub
/// cannot tell them apart and must not learn how: everything it passes through this
/// boundary is an opaque tag and an opaque blob.
struct Hub {
    /// See [`next_seq`](Hub::next_seq): microseconds since the Unix epoch, NOT a
    /// counter from 1.
    seq: u64,
    store: Box<dyn Mailbox>,
    retention: Retention,
    conns: HashMap<ConnId, Conn>,
    metrics: Arc<Metrics>,
    limits: Limits,
    /// Bytes of blobs held, as published — seeded from the store at boot, raised on
    /// every stored publish, re-read after a sweep. What `limits.max_store_bytes`
    /// is measured against.
    stored_bytes: u64,
    /// The whole relay's ingest ceiling.
    ingest: limits::Bucket,
    /// When the store-full alarm last fired, so a flood logs once a minute rather
    /// than once per refused publish.
    full_alarm_at: Option<Instant>,
}

struct Conn {
    tags: HashSet<String>,
    out: mpsc::UnboundedSender<Frame>,
    /// This connection's pace: publishes, bytes and NEW addresses.
    pace: limits::ConnLimiter,
}

/// What one publish came to. `reason` is the refusal token sent on `Ack`, and is
/// `None` both on success and on the one refusal that is not an error: a commit
/// that lost its tag's slot.
struct Published {
    seq: u64,
    fanout: u64,
    ok: bool,
    reason: Option<&'static str>,
}

impl Published {
    fn refused(reason: &'static str) -> Self {
        Self { seq: 0, fanout: 0, ok: false, reason: Some(reason) }
    }
}

/// Wall clock in microseconds. A clock before the Unix epoch (impossible on any
/// real host) degrades to the old counter-from-zero behaviour rather than panicking
/// in the relay's hot path.
fn now_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// Boot headroom for the one case [`Hub::next_seq`]'s clock tracking cannot cover:
/// a burst publishing more than a million blobs in a second pushes `seq` ahead of
/// the clock, and a restart inside that window could otherwise reissue numbers the
/// clock has not yet reached. A second of slack per boot costs nothing against a
/// u64 of microseconds (~584,000 years).
const BOOT_SKEW_US: u64 = 1_000_000;

impl Hub {
    /// Adopt a store. `seq` is seeded from `max(clock + skew, durable high-water + 1)`
    /// so it is monotone across a restart even if the clock is not — a store that
    /// remembers what it issued no longer has to trust NTP (SCALE-UNBLOCKS U1 TRAP 2).
    async fn new(store: Box<dyn Mailbox>, retention: Retention, limits: Limits) -> Self {
        let resumed = store.resume_seq().await.unwrap_or(0);
        let stored_bytes = store.stored_bytes().await.unwrap_or(0);
        Self {
            seq: now_micros()
                .saturating_add(BOOT_SKEW_US)
                .max(resumed.saturating_add(1)),
            store,
            retention,
            conns: HashMap::new(),
            metrics: Arc::default(),
            limits,
            stored_bytes,
            ingest: limits.ingest(),
            full_alarm_at: None,
        }
    }
}

impl Hub {
    /// The next publish sequence number: the wall clock in microseconds, or one
    /// past the last issued number when the clock has not advanced (or has gone
    /// backwards). Strictly increasing within a process AND across restarts.
    ///
    /// WHY THIS IS NOT A COUNTER FROM 1. Clients persist a per-`(tag, source)`
    /// cursor that never rewinds — `inbox_cursor_v2`, written with
    /// `MAX(last_seen_id, excluded.last_seen_id)` — while this process's `seq`
    /// used to restart at 0 on every boot. A redeploy therefore left every client
    /// holding a cursor above the restarted counter unable to see ANY subsequent
    /// blob on that tag: `subscribe` replays only `seq > since`, live fanout
    /// carries the same low `seq`, and the cursor never comes back down. The
    /// publisher saw `Ack{ok:true}` and `mark_delivered` ran, so nothing surfaced
    /// the loss. Every member of every group went silently deaf on each deploy
    /// until the counter climbed back past their cursor — and everything published
    /// in that window stayed invisible even though it sat in the store.
    ///
    /// Seeding from the clock fixes that with no persistence, no wire change and
    /// no client state. Sparse, large `seq` values were already normal (the
    /// counter is global across tags, so any single tag's sequence has gaps), and
    /// microseconds stay well inside the `i64` the client cursor is stored as.
    ///
    /// This does NOT make the store durable: a restart still forgets undelivered
    /// blobs (the M2 TODO above). It makes the numbering survive, which is what
    /// the cursor depends on.
    fn next_seq(&mut self) -> u64 {
        self.seq = now_micros().max(self.seq.saturating_add(1));
        self.seq
    }

    /// Store a blob under `tag` and fan it out to every live subscriber. Returns
    /// `(seq, fanout, accepted)` — the assigned monotonic seq, the number of live
    /// subscribers it was delivered to, and whether the blob was accepted at all.
    /// A commit-flagged publish to a tag whose slot is already taken is REJECTED:
    /// not stored, not fanned out, `(0, 0, false)` — the loser must rebase.
    /// NB: none of the return values is routing data.
    /// A publish is Acked ok ONLY once it is durably stored. A storage failure
    /// returns `accepted = false`, which the client surfaces as a hard transport
    /// error and retries — the one thing we must never do is Ack a blob we did
    /// not keep, because that is precisely the silent loss this work exists to
    /// remove.
    ///
    /// AWAITS THE STORE WHILE THE HUB MUTEX IS HELD. On the SQLite backend that is
    /// a page-cache write and the await never yields. On the offload backend it is
    /// a round trip to object storage, and every other connection waits for it —
    /// stated here rather than left to be found, because it is the price of that
    /// backend and `offload.rs` is where the trade is argued.
    async fn publish(&mut self, id: ConnId, tag: String, blob: String, commit: bool) -> Published {
        let len = blob.len() as u64;

        // THIS CONNECTION'S PACE — publishes and bytes. The signature has already
        // been checked by the caller, so what is being paced is a key holder.
        if let Some(c) = self.conns.get_mut(&id) {
            if !c.pace.pubs.take(1.0) || !c.pace.bytes.take(len as f64) {
                self.metrics.refused_publishes.fetch_add(1, Ordering::Relaxed);
                return Published::refused("rate_limited");
            }
        }
        // THE WHOLE RELAY'S INGEST — the line a thousand paced connections still meet.
        if !self.ingest.take(len as f64) {
            self.metrics.refused_publishes.fetch_add(1, Ordering::Relaxed);
            return Published::refused("busy");
        }
        // THE STORE'S CEILING, FAIL-CLOSED. Refuse; never evict. At the ceiling the
        // worst a flood has done is stop writes — nobody's records are touched.
        if self.limits.max_store_bytes > 0
            && self.stored_bytes.saturating_add(len) > self.limits.max_store_bytes
        {
            self.metrics.refused_publishes.fetch_add(1, Ordering::Relaxed);
            let due = self.full_alarm_at.map_or(true, |t| t.elapsed() >= Duration::from_secs(60));
            if due {
                self.full_alarm_at = Some(Instant::now());
                error!(
                    stored_bytes = self.stored_bytes,
                    max_store_bytes = self.limits.max_store_bytes,
                    "STORE FULL — refusing every publish until an operator acts"
                );
            }
            return Published::refused("full");
        }
        // A NEW ADDRESS spends from its own, slower budget: keys are the cheap thing
        // an anonymous caller has, and this is where minting them costs something.
        let paced_new = self.conns.get(&id).map_or(false, |c| c.pace.new_tags.is_limited());
        if paced_new {
            match self.store.has_tag(&tag).await {
                Ok(true) => {}
                Ok(false) => {
                    let ok = self.conns.get_mut(&id).map_or(true, |c| c.pace.new_tags.take(1.0));
                    if !ok {
                        self.metrics.refused_publishes.fetch_add(1, Ordering::Relaxed);
                        return Published::refused("rate_limited");
                    }
                }
                Err(e) => {
                    error!(error = %e, "store read failed — publish NOT acked");
                    self.metrics.store_errors.fetch_add(1, Ordering::Relaxed);
                    return Published::refused("unavailable");
                }
            }
        }

        let seq = self.next_seq();
        match self.store.append(&tag, seq, &blob, commit).await {
            Ok(Appended::Stored) => {}
            Ok(Appended::Duplicate { seq: prior }) => {
                // Already here: acked as the publish it was, sent to nobody twice.
                self.metrics.duplicate_publishes.fetch_add(1, Ordering::Relaxed);
                return Published { seq: prior, fanout: 0, ok: true, reason: None };
            }
            Ok(Appended::SlotTaken) => {
                // Commit slot already taken — the loser must rebase.
                self.metrics.rejected_commits.fetch_add(1, Ordering::Relaxed);
                return Published { seq: 0, fanout: 0, ok: false, reason: None };
            }
            Err(e) => {
                // The error kind only; it never embeds tag or blob data. Named on the
                // wire as `unavailable`, so a client can never read a storage failure
                // as a lost commit race and rebase onto a winner that does not exist.
                error!(error = %e, "store write failed — publish NOT acked");
                self.metrics.store_errors.fetch_add(1, Ordering::Relaxed);
                return Published::refused("unavailable");
            }
        }
        self.stored_bytes = self.stored_bytes.saturating_add(len);
        let mut fanout = 0u64;
        for conn in self.conns.values() {
            if conn.tags.contains(&tag) {
                let _ = conn.out.send(Frame::Msg {
                    tag: tag.clone(),
                    seq,
                    blob: blob.clone(),
                });
                fanout += 1;
            }
        }
        self.metrics.total_publishes.fetch_add(1, Ordering::Relaxed);
        self.metrics
            .total_deliveries
            .fetch_add(fanout, Ordering::Relaxed);
        Published { seq, fanout, ok: true, reason: None }
    }

    /// Set a connection's subscriptions, replay the backlog (seq > since) in order,
    /// then emit `Eose`. Returns `(replayed, gaps)` — two counts, never content.
    ///
    /// Any tag whose retention floor sits above the caller's cursor gets a `Gap`
    /// first: that backlog is INCOMPLETE and the client is entitled to know before
    /// it treats the replay as its history. `Gap` goes only to `v >= 1` clients —
    /// an unknown frame is fatal on older builds, so silence is the only safe
    /// thing to send them.
    ///
    /// `v` is the connection's declared vocabulary high-water mark, tracked by
    /// [`handle`]. The floor lookup is skipped below [`V_GAP`] purely to save the
    /// store read; the gate that actually decides is [`may_send_to`], so the
    /// vocabulary rule lives in ONE place and `relay::wire`'s spec tests are about
    /// the code that runs.
    async fn subscribe(&mut self, id: ConnId, tags: Vec<String>, since: u64, v: u8) -> (u64, u64) {
        let floors = if v >= V_GAP {
            self.store.floors(&tags).await.unwrap_or_default()
        } else {
            HashMap::new()
        };
        let mut backlog: Vec<(u64, String, String)> = Vec::new();
        for t in &tags {
            match self.store.replay(t, since).await {
                Ok(items) => backlog.extend(items.into_iter().map(|(seq, b)| (seq, t.clone(), b))),
                Err(e) => {
                    // A read failure must not masquerade as an empty backlog: that
                    // is the silent hole again. Announce it as a gap at the
                    // caller's own cursor when we can, and always log it.
                    error!(error = %e, "store read failed during subscribe drain");
                    self.metrics.store_errors.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        backlog.sort_by_key(|(seq, _, _)| *seq);
        let replayed = backlog.len() as u64;
        let mut gaps = 0u64;
        if let Some(conn) = self.conns.get_mut(&id) {
            for t in &tags {
                if let Some(&floor) = floors.get(t) {
                    if since < floor {
                        let gap = Frame::Gap { tag: t.clone(), floor };
                        if may_send_to(&gap, v) {
                            let _ = conn.out.send(gap);
                            gaps += 1;
                        }
                    }
                }
            }
            conn.tags = tags.into_iter().collect();
            for (seq, tag, blob) in backlog {
                let _ = conn.out.send(Frame::Msg { tag, seq, blob });
            }
            let _ = conn.out.send(Frame::Eose);
        }
        self.metrics
            .total_deliveries
            .fetch_add(replayed, Ordering::Relaxed);
        self.metrics.gaps_announced.fetch_add(gaps, Ordering::Relaxed);
        (replayed, gaps)
    }

    /// Run one eviction sweep. Returns blobs removed.
    async fn evict(&mut self) -> u64 {
        let r = self.retention;
        match self.store.evict(now_micros(), r.window_us, r.max_per_tag).await {
            Ok(n) => {
                if n > 0 {
                    if let Ok(b) = self.store.stored_bytes().await {
                        self.stored_bytes = b;
                    }
                }
                n
            }
            Err(e) => {
                error!(error = %e, "eviction sweep failed");
                self.metrics.store_errors.fetch_add(1, Ordering::Relaxed);
                0
            }
        }
    }

    /// Current store size as two pure cardinalities: distinct tags and total stored
    /// blobs. No tag names, no blob bytes — just counts.
    async fn store_size(&self) -> (u64, u64) {
        self.store.counts().await.unwrap_or((0, 0))
    }

    /// Traffic-only per-tag snapshot across stored tags ∪ subscribed tags. This is
    /// the one place tag names are read out, and only into a [`RelaySnapshot`] for
    /// the local traffic (never telemetry).
    async fn snapshot(&self, metrics: &Metrics) -> RelaySnapshot {
        let stored: HashMap<String, u64> =
            self.store.tag_counts().await.unwrap_or_default().into_iter().collect();
        let mut names: HashSet<&String> = stored.keys().collect();
        for c in self.conns.values() {
            for t in &c.tags {
                names.insert(t);
            }
        }
        let mut tags: Vec<TagStat> = names
            .into_iter()
            .map(|t| {
                let blobs = stored.get(t).copied().unwrap_or(0);
                let subs = self.conns.values().filter(|c| c.tags.contains(t)).count() as u64;
                TagStat {
                    tag: t.clone(),
                    blobs,
                    subs,
                }
            })
            .collect();
        tags.sort_by(|a, b| a.tag.cmp(&b.tag));
        RelaySnapshot {
            active_conns: metrics.active_conns.load(Ordering::Relaxed),
            total_publishes: metrics.total_publishes.load(Ordering::Relaxed),
            total_deliveries: metrics.total_deliveries.load(Ordering::Relaxed),
            tags,
        }
    }
}

/// Run the relay accept loop on `listener` forever. PRODUCTION entrypoint — blind,
/// no observer (see the module doc).
///
/// The store comes from [`mailbox_from_env`]. Retention comes from
/// [`Retention::from_env`].
///
/// DEPLOYMENT: on a container host the path must live on a MOUNTED VOLUME.
/// Persisting to the image's own filesystem buys nothing, because the filesystem
/// is what a redeploy replaces.
pub async fn serve(listener: TcpListener) {
    let store = mailbox_from_env();
    serve_with_mailbox(listener, store, Retention::from_env(), None).await
}

/// Choose the storage backend from the environment, and SAY WHICH ONE.
///
/// * `RELAY_STORE` — path to the SQLite index. Unset means an in-memory mailbox,
///   which is the pre-durability behaviour and is warned about loudly rather than
///   quietly accepted.
/// * `RELAY_MAILBOX_BLOBS` — `sqlite` (the default) or `r2`. On `r2` the index
///   stays in SQLite and the blob BODIES go to the object storage already
///   configured by `RELAY_R2_*` (the same four variables `media.rs` reads).
///
/// Three things are refused rather than degraded, because each one would run
/// happily while being wrong:
///
///   1. `RELAY_MAILBOX_BLOBS=r2` with no `RELAY_STORE`. The bodies would be durable
///      and the COMMIT SLOTS would not, so a restart would re-open a slot a group
///      has already used — a fork, hidden behind a backend that looks more durable
///      than the default.
///   2. `RELAY_MAILBOX_BLOBS=r2` with `RELAY_R2_*` absent or half-set. There is
///      nowhere to put the bodies; `media.rs` already refuses a partial R2 config
///      for the same reason.
///   3. A value of `RELAY_MAILBOX_BLOBS` that is neither. A typo must not silently
///      select the default.
pub fn mailbox_from_env() -> Box<dyn Mailbox> {
    let path = std::env::var("RELAY_STORE")
        .ok()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());

    let backend = std::env::var("RELAY_MAILBOX_BLOBS")
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_else(|_| "sqlite".into());
    let offload = match backend.as_str() {
        "sqlite" | "" => false,
        "r2" => true,
        other => panic!(
            "RELAY_MAILBOX_BLOBS={other:?} is not a backend. Use 'sqlite' (default) or 'r2'."
        ),
    };

    let index = match &path {
        Some(p) => match Store::open(p) {
            Ok(s) => {
                info!("durable store opened");
                s
            }
            // Refusing to start beats silently running ephemeral: an operator who
            // asked for durability and got amnesia would not find out until a
            // redeploy ate someone's messages.
            Err(e) => panic!("RELAY_STORE could not be opened: {e}"),
        },
        None => {
            if offload {
                panic!(
                    "RELAY_MAILBOX_BLOBS=r2 needs RELAY_STORE: the commit slots, the retention \
                     floors and the seq high-water live in the index, and an in-memory index \
                     re-opens a claimed commit slot on every restart. Durable bodies do not \
                     make that safe — they hide it."
                );
            }
            warn!(
                "RELAY_STORE unset — mailbox is IN-MEMORY and will not survive a restart \
                 (undrained blobs lost, commit slots re-opened). Set RELAY_STORE to a path \
                 on a mounted volume."
            );
            Store::open_in_memory().expect("in-memory store")
        }
    };

    if !offload {
        info!(backend = %index.describe(), "mailbox backend");
        return Box::new(index);
    }

    let blobs = blobs_from_env()
        .unwrap_or_else(|e| panic!("RELAY_MAILBOX_BLOBS=r2 but object storage is unusable: {e}"));
    let store = offload::OffloadMailbox::new(index, blobs);
    info!(backend = %store.describe(), "mailbox backend");
    Box::new(store)
}

/// Build the body store from the same `RELAY_R2_*` variables `media.rs` reads, so
/// enabling blob offload needs no second bucket and no second credential.
fn blobs_from_env() -> std::result::Result<Box<dyn blobs::Blobs>, String> {
    let get = |k: &str| {
        std::env::var(k)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let endpoint = get("RELAY_R2_ENDPOINT").ok_or("RELAY_R2_ENDPOINT is unset")?;
    let bucket = get("RELAY_R2_BUCKET").ok_or("RELAY_R2_BUCKET is unset")?;
    let access_key_id = get("RELAY_R2_ACCESS_KEY_ID").ok_or("RELAY_R2_ACCESS_KEY_ID is unset")?;
    let secret_access_key =
        get("RELAY_R2_SECRET_ACCESS_KEY").ok_or("RELAY_R2_SECRET_ACCESS_KEY is unset")?;
    let region = get("RELAY_R2_REGION").unwrap_or_else(|| "auto".into());
    let r2 = blobs::R2Blobs::from_endpoint(
        &endpoint,
        bucket,
        region,
        sigv4::Credentials {
            access_key_id,
            secret_access_key,
        },
    )?;
    Ok(Box::new(r2))
}

/// Like [`serve`], but additionally streams [`RelayEvent`]s to a traffic `obs`.
/// Used ONLY by `pacific-traffic`. With `obs = None` behaves identically to
/// [`serve`] (zero extra work, nothing observed).
pub async fn serve_with_observer(listener: TcpListener, obs: Option<Observer>) {
    let store = Store::open_in_memory().expect("in-memory store");
    serve_with_store(listener, store, Retention::from_env(), obs).await
}

/// The accept loop over an explicit store and policy. Tests use this to drive a
/// real file across a simulated restart.
///
/// Generic over the backend since 18 Sep 2026 — `Store` still passes unchanged,
/// and so does any other [`Mailbox`].
pub async fn serve_with_store(
    listener: TcpListener,
    store: impl Mailbox + 'static,
    retention: Retention,
    obs: Option<Observer>,
) {
    serve_with_mailbox(listener, Box::new(store), retention, obs).await
}

/// The accept loop over a boxed backend — what [`serve`] uses once
/// [`mailbox_from_env`] has chosen one.
pub async fn serve_with_mailbox(
    listener: TcpListener,
    store: Box<dyn Mailbox>,
    retention: Retention,
    obs: Option<Observer>,
) {
    serve_with_policy(listener, store, retention, Limits::from_env(), obs).await
}

/// The accept loop with every policy explicit — backend, retention AND limits. The
/// other entry points read the limits from the environment; tests that exercise a
/// limit pass their own here.
pub async fn serve_with_policy(
    listener: TcpListener,
    store: Box<dyn Mailbox>,
    retention: Retention,
    limits: Limits,
    obs: Option<Observer>,
) {
    info!(
        max_blob_bytes = limits.max_blob_bytes,
        max_store_bytes = limits.max_store_bytes,
        conn_pubs_per_sec = limits.conn_pubs_per_sec,
        conn_new_tags_per_min = limits.conn_new_tags_per_min,
        ingest_bytes_per_sec = limits.ingest_bytes_per_sec,
        "limits: every Pub is signature-checked; the store refuses when full and never evicts"
    );
    let hub: Arc<Mutex<Hub>> = Arc::new(Mutex::new(Hub::new(store, retention, limits).await));
    let metrics = hub.lock().await.metrics.clone();
    // Object storage is read from the environment here rather than taken as a
    // parameter, so the existing `serve_with_store` signature (and every test that
    // drives it) is unchanged. Unset means media offload is simply off, and every
    // request for it is refused with `unconfigured` rather than quietly dropped.
    let media: Option<Arc<Media>> = Media::from_env().map(Arc::new);
    if let Some(m) = &media {
        info!(
            max_object_bytes = m.max_bytes(),
            budget_bytes = m.budget().map(|b| b.max_bytes()).unwrap_or(0),
            "media offload enabled"
        );
        // The meter runs whether or not anyone uploads: the relay starts FAIL-CLOSED
        // (no measurement yet = no presigning), so this loop is what opens it.
        if m.budget().is_some() {
            tokio::spawn(usage_loop(m.clone()));
        }
    }
    let mut next_id: ConnId = 0;

    // Periodic ops snapshot — counts and store cardinalities only. INFO so it lands
    // in the container log and (when enabled) Sentry Logs.
    tokio::spawn(stats_loop(hub.clone(), metrics.clone(), obs.clone()));
    tokio::spawn(evict_loop(hub.clone(), retention));

    loop {
        let (stream, _peer) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => {
                // Accept errors carry no client data worth redacting, but we still
                // keep it to the error kind — never the peer.
                error!(error = %e, "accept error");
                continue;
            }
        };
        let id = next_id;
        next_id += 1;
        let hub = hub.clone();
        let metrics = metrics.clone();
        let obs = obs.clone();
        let media = media.clone();
        // One span per connection, carrying ONLY the opaque connection id.
        let span = info_span!("conn", id);
        tokio::spawn(
            async move {
                metrics.active_conns.fetch_add(1, Ordering::Relaxed);
                info!("connection opened");
                emit(&obs, RelayEvent::ConnOpened { conn: id });
                emit_snapshot(&obs, &hub, &metrics).await;
                if let Err(e) =
                    handle(id, stream, hub.clone(), metrics.clone(), obs.clone(), media, limits).await
                {
                    // No client IP in logs — keep the relay blind even in telemetry.
                    // `e` is a ws/io error kind/message; it never embeds tag/blob data.
                    warn!(error = %e, "connection closed with error");
                } else {
                    info!("connection closed");
                }
                hub.lock().await.conns.remove(&id);
                metrics.active_conns.fetch_sub(1, Ordering::Relaxed);
                emit(&obs, RelayEvent::ConnClosed { conn: id });
                emit_snapshot(&obs, &hub, &metrics).await;
            }
            .instrument(span),
        );
    }
}

/// Send an event if an observer is wired (traffic only). A dropped receiver is fine.
fn emit(obs: &Option<Observer>, ev: RelayEvent) {
    if let Some(o) = obs {
        let _ = o.send(ev);
    }
}

/// Lock the hub, build a snapshot, and send it — only when an observer is present.
async fn emit_snapshot(obs: &Option<Observer>, hub: &Arc<Mutex<Hub>>, metrics: &Arc<Metrics>) {
    if let Some(o) = obs {
        let snap = hub.lock().await.snapshot(metrics).await;
        let _ = o.send(RelayEvent::Snapshot(snap));
    }
}

/// Emit an INFO `stats` event roughly every 30s with the operational counters and
/// store cardinalities. This is the heartbeat of the ops plane; it contains no
/// routing or content data. Also nudges the traffic snapshot when observed.
async fn stats_loop(hub: Arc<Mutex<Hub>>, metrics: Arc<Metrics>, obs: Option<Observer>) {
    let mut ticker = tokio::time::interval(Duration::from_secs(30));
    // The first tick fires immediately; skip it so we don't log an all-zero line.
    ticker.tick().await;
    loop {
        ticker.tick().await;
        let (tag_count, blob_count) = hub.lock().await.store_size().await;
        info!(
            active_conns = metrics.active_conns.load(Ordering::Relaxed),
            total_publishes = metrics.total_publishes.load(Ordering::Relaxed),
            total_deliveries = metrics.total_deliveries.load(Ordering::Relaxed),
            bad_frames = metrics.bad_frames.load(Ordering::Relaxed),
            rejected_commits = metrics.rejected_commits.load(Ordering::Relaxed),
            store_errors = metrics.store_errors.load(Ordering::Relaxed),
            gaps_announced = metrics.gaps_announced.load(Ordering::Relaxed),
            media_presigned = metrics.media_presigned.load(Ordering::Relaxed),
            media_refused = metrics.media_refused.load(Ordering::Relaxed),
            gated_requests = metrics.gated_requests.load(Ordering::Relaxed),
            refused_publishes = metrics.refused_publishes.load(Ordering::Relaxed),
            duplicate_publishes = metrics.duplicate_publishes.load(Ordering::Relaxed),
            store_tags = tag_count,
            store_blobs = blob_count,
            "stats"
        );
        emit_snapshot(&obs, &hub, &metrics).await;
    }
}

/// Sweep expired blobs on a timer. Every removal raises the affected tag's
/// retention floor in the same transaction, so nothing this loop drops can ever
/// become an invisible hole — a subscriber that needed those blobs is told.
async fn evict_loop(hub: Arc<Mutex<Hub>>, retention: Retention) {
    // The DEFAULT, not a misconfiguration — so this is an ops fact, not a warning.
    // Said once at boot, in the same log as the stats line, so a store that grows
    // has its explanation next to it.
    if retention.window_us == 0 && retention.max_per_tag == 0 {
        info!("no retention limits — the mailbox keeps everything until told otherwise");
        return;
    }
    let mut ticker = tokio::time::interval(retention.sweep);
    ticker.tick().await; // the immediate first tick has nothing to do
    loop {
        ticker.tick().await;
        let started = Instant::now();
        let removed = hub.lock().await.evict().await;
        if removed > 0 {
            info!(
                removed,
                latency_us = started.elapsed().as_micros() as u64,
                "eviction sweep"
            );
        }
    }
}

async fn handle(
    id: ConnId,
    stream: TcpStream,
    hub: Arc<Mutex<Hub>>,
    metrics: Arc<Metrics>,
    obs: Option<Observer>,
    media: Option<Arc<Media>>,
    limits: Limits,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // A message larger than one maximal blob plus its frame is refused by the
    // websocket layer before it is buffered. tungstenite's own default is 64 MiB,
    // which let about 140 anonymous frames fill the store.
    let mut ws_cfg = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default();
    ws_cfg.max_message_size = Some(limits.max_message_bytes());
    ws_cfg.max_frame_size = Some(limits.max_message_bytes());
    let ws = tokio_tungstenite::accept_async_with_config(stream, Some(ws_cfg)).await?;
    let (mut sink, mut source) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Frame>();
    // Kept here as well as in the hub, so a publish refused before the hub is
    // touched (too large, not signed by its tag) is answered without taking the lock.
    let me = tx.clone();

    hub.lock().await.conns.insert(
        id,
        Conn {
            tags: HashSet::new(),
            out: tx,
            pace: limits.conn_limiter(),
        },
    );

    // writer task: drain outbound frames to the socket as JSON text.
    let writer = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            if sink.send(Message::Text(frame.to_json())).await.is_err() {
                break;
            }
        }
    });

    // THE CONNECTION'S DECLARED VOCABULARY — the high-water mark of every `Sub.v`
    // this socket has sent, and the single thing that decides what the relay may
    // say back to it.
    //
    // It is a local, not a field on `Conn`, on purpose. Only this task ever sends a
    // vocabulary-gated frame to this socket (the hub's own fan-out is `Msg`, which
    // is level 0 and always safe), so keeping it here means the gate costs no hub
    // lock on the hot path.
    //
    // It only ever RISES: a client cannot un-learn a frame mid-connection, so a
    // later `Sub { v: 0 }` from a build that has already declared level 2 does not
    // un-declare it.
    let mut declared_v: u8 = V_BASE;

    // reader loop: parse inbound frames and act on them.
    while let Some(msg) = source.next().await {
        let text = match msg? {
            Message::Text(t) => t.to_string(),
            Message::Binary(b) => String::from_utf8_lossy(b.as_ref()).into_owned(),
            Message::Close(_) => break,
            _ => continue, // ping/pong/etc.
        };
        let frame = match Frame::from_json(&text) {
            Ok(f) => f,
            Err(e) => {
                metrics.bad_frames.fetch_add(1, Ordering::Relaxed);
                // `e` is a serde error describing the parse failure shape; we never
                // log `text` itself (it could embed a tag/blob).
                warn!(error = %e, "dropping bad frame");
                continue;
            }
        };
        // THE SERVICE GATE. A request above the connection's declared vocabulary is
        // dropped in SILENCE, because every answer to it is at its own level: a
        // `MediaPut` is answered with `MediaUrl` or `MediaErr`, and both are level 2.
        // Replying "you are too old" is not available — that reply would itself be
        // the unknown frame that kills the client's drain. This is the same ruling
        // as `Gap`, which a v0 client is simply never sent.
        //
        // Only REQUESTS are gated. A relay→client frame arriving inbound is nonsense
        // the dispatch already ignores, and counting it here would be noise.
        //
        // No client emits a media frame today, so nothing legitimate lands here; a
        // non-zero `gated_requests` is a client asking for an answer it cannot read.
        if frame.dir() == Dir::ToRelay && !may_service(&frame, declared_v) {
            metrics.gated_requests.fetch_add(1, Ordering::Relaxed);
            // The frame TAG is vocabulary, not routing data — it names a protocol
            // verb, never a tag, key or blob — so it is safe under the blind
            // telemetry invariant at the top of this file.
            warn!(
                frame = frame.wire_tag(),
                declared_v,
                min_v = frame.min_v(),
                "dropping a frame above the connection's declared vocabulary"
            );
            continue;
        }
        match frame {
            // `sig` is carried and not yet checked: the check lands with the client that
            // signs every publish (pacific_wire::address). Until then it is ignored.
            Frame::Pub { tag, blob, commit, sig } => {
                let started = Instant::now();
                let blob_len = blob.len();
                // WHAT CAN BE REFUSED WITHOUT THE HUB: size, and whether this
                // connection holds the key the tag IS (pacific_wire::address — the
                // one check the client runs too). Only a key holder reaches the hub.
                let early = if blob_len > limits.max_blob_bytes {
                    Some("too_large")
                } else {
                    crate::wire::address::verify_pub(&tag, &blob, &sig).err().map(|r| r.as_str())
                };
                if let Some(reason) = early {
                    metrics.refused_publishes.fetch_add(1, Ordering::Relaxed);
                    let _ = me.send(Frame::Ack { seq: 0, ok: false, reason: Some(reason.into()) });
                    // The token names a rule, never a tag or a blob.
                    info!(reason, "publish refused");
                    continue;
                }
                let mut h = hub.lock().await;
                let Published { seq, fanout, ok, reason } =
                    h.publish(id, tag.clone(), blob, commit).await;
                if let Some(conn) = h.conns.get(&id) {
                    let _ = conn.out.send(Frame::Ack { seq, ok, reason: reason.map(Into::into) });
                }
                let snap = if obs.is_some() {
                    Some(h.snapshot(&metrics).await)
                } else {
                    None
                };
                drop(h);
                // fanout + latency + verdict are pure ops metrics; `tag`/`blob`
                // never reach this INFO event (they go ONLY to the traffic
                // observer, if any).
                info!(
                    fanout,
                    ok,
                    reason = reason.unwrap_or(""),
                    latency_us = started.elapsed().as_micros() as u64,
                    "publish"
                );
                if ok {
                    emit(
                        &obs,
                        RelayEvent::Published {
                            conn: id,
                            tag,
                            blob_len,
                            seq,
                            fanout,
                        },
                    );
                }
                if let Some(s) = snap {
                    emit(&obs, RelayEvent::Snapshot(s));
                }
            }
            Frame::Sub { tags, since, v } => {
                let started = Instant::now();
                declared_v = declared_v.max(v);
                // `tags.len()` is a cardinality, not the tag values.
                let tag_count = tags.len() as u64;
                let ev_tags = if obs.is_some() { tags.clone() } else { Vec::new() };
                let mut h = hub.lock().await;
                let (replayed, gaps) = h.subscribe(id, tags, since, declared_v).await;
                let snap = if obs.is_some() {
                    Some(h.snapshot(&metrics).await)
                } else {
                    None
                };
                drop(h);
                info!(
                    tag_count,
                    replayed,
                    gaps,
                    latency_us = started.elapsed().as_micros() as u64,
                    "subscribe drain"
                );
                emit(
                    &obs,
                    RelayEvent::Subscribed {
                        conn: id,
                        tags: ev_tags,
                        since,
                        replayed,
                    },
                );
                if let Some(s) = snap {
                    emit(&obs, RelayEvent::Snapshot(s));
                }
            }
            // Reachable ONLY at vocabulary level 2 — the service gate above has
            // already dropped a media request from a connection that never declared
            // it, because both replies below (`MediaUrl`, `MediaErr`) are level-2
            // frames that such a client could not parse.
            //
            // Presigning is pure CPU (four HMACs, no I/O), so it runs inline and
            // never takes the hub lock — a photo upload cannot stall the mailbox.
            //
            // The reply carries the key and a signed URL, so neither is logged: a
            // media key is the same class of secret as a `tag`, and the telemetry
            // invariant at the top of this file admits no exceptions for it.
            Frame::MediaPut { key, len } => {
                let (reply, ok) = presign_reply(&media, &key, Some(len));
                if let Some(conn) = hub.lock().await.conns.get(&id) {
                    let _ = conn.out.send(reply);
                }
                bump(&metrics, ok);
                info!(ok, len, "media presign put");
            }
            Frame::MediaGet { key } => {
                let (reply, ok) = presign_reply(&media, &key, None);
                if let Some(conn) = hub.lock().await.conns.get(&id) {
                    let _ = conn.out.send(reply);
                }
                bump(&metrics, ok);
                info!(ok, "media presign get");
            }
            // relay→client frames are never expected inbound; ignore.
            Frame::Msg { .. }
            | Frame::Ack { .. }
            | Frame::Gap { .. }
            | Frame::MediaUrl { .. }
            | Frame::MediaErr { .. }
            | Frame::Eose => {}
        }
    }
    writer.abort();
    Ok(())
}


/// Mint the reply to a media request. `len = Some(n)` presigns a PUT bounded to
/// exactly `n` bytes; `None` presigns a GET.
///
/// Split out of `handle` so the refusal path is written once: an unconfigured
/// relay, a malformed key and an oversized upload all answer with `MediaErr` and
/// a stable reason, and none of them can be mistaken for success.
fn presign_reply(media: &Option<Arc<Media>>, key: &str, len: Option<u64>) -> (Frame, bool) {
    let Some(m) = media.as_ref() else {
        return (err_frame(key, MediaError::Unconfigured), false);
    };
    let signed = match len {
        Some(n) => m.presign_put(key, n),
        None => m.presign_get(key),
    };
    match signed {
        Ok(url) => (
            Frame::MediaUrl {
                key: key.to_string(),
                method: if len.is_some() { "PUT" } else { "GET" }.into(),
                url,
                expires_in: m.ttl_secs(),
            },
            true,
        ),
        Err(e) => (err_frame(key, e), false),
    }
}

fn err_frame(key: &str, e: MediaError) -> Frame {
    Frame::MediaErr {
        key: key.to_string(),
        reason: e.reason().to_string(),
    }
}

fn bump(metrics: &Metrics, ok: bool) {
    if ok {
        metrics.media_presigned.fetch_add(1, Ordering::Relaxed);
    } else {
        metrics.media_refused.fetch_add(1, Ordering::Relaxed);
    }
}

/// Re-measure the bucket on a timer so the budget in `media` reflects reality.
///
/// The first tick fires immediately, on purpose: until a measurement lands the
/// relay refuses every upload (`budget_stale`), so the shorter that window is, the
/// less a cold start looks like an outage.
///
/// A failed poll is NOT treated as zero usage. It leaves the previous reading in
/// place, and once readings stop for long enough the budget goes closed on its own
/// — the failure mode of a cost guard should be "stop spending", not "assume the
/// bucket is empty".
async fn usage_loop(media: Arc<Media>) {
    let Some(budget) = media.budget().cloned() else {
        return;
    };
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, "usage meter disabled: could not build HTTP client");
            return;
        }
    };

    let mut ticker = tokio::time::interval(media.poll());
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let max = budget.max_bytes();
        match measure_usage(&client, &media, max).await {
            Ok(bytes) => {
                budget.record(bytes, budget::now_secs());
                let pct = bytes.saturating_mul(100).checked_div(max).unwrap_or(0);
                // Byte totals and percentages only — never a key or an object.
                info!(used_bytes = bytes, budget_bytes = max, used_pct = pct, "storage measured");
                if pct >= 80 {
                    warn!(
                        used_bytes = bytes,
                        budget_bytes = max,
                        "storage budget above 80% — uploads will start being refused"
                    );
                }
            }
            // Terse by design: the request URL is presigned (it carries a signature)
            // and the body can name objects, so neither is ever logged.
            Err(e) => warn!(error = %e, "storage measurement failed; budget left unchanged"),
        }
    }
}

/// Walk `ListObjectsV2` and sum the bucket.
///
/// Two early exits, both of which round TOWARDS refusing:
///
///   * Once the running total passes the budget the walk stops — more precision
///     past "over" buys nothing and costs Class A operations.
///   * If the listing needs more than `MAX_LIST_PAGES`, it reports the budget as
///     fully consumed rather than the partial sum. A measurement we declined to
///     finish is not evidence of headroom, and a partial sum would read as exactly
///     that.
async fn measure_usage(
    client: &reqwest::Client,
    media: &Media,
    budget_max: u64,
) -> Result<u64, String> {
    let mut total: u64 = 0;
    let mut token: Option<String> = None;

    for _ in 0..budget::MAX_LIST_PAGES {
        let url = media.presign_list(token.as_deref());
        let resp = client.get(url).send().await.map_err(|e| {
            if e.is_timeout() {
                "listing timed out".to_string()
            } else if e.is_connect() {
                "object store unreachable".to_string()
            } else {
                "listing request failed".to_string()
            }
        })?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|_| "listing response was unreadable".to_string())?;
        if !status.is_success() {
            return Err(format!("listing returned HTTP {}", status.as_u16()));
        }

        let page = budget::parse_list_page(&body)?;
        total = total.saturating_add(page.bytes);
        if total > budget_max {
            return Ok(total);
        }
        match page.next {
            Some(t) => token = Some(t),
            None => return Ok(total),
        }
    }

    warn!(
        pages = budget::MAX_LIST_PAGES,
        "bucket listing hit the page cap — treating storage as full rather than \
         spending more operations to measure it"
    );
    Ok(budget_max)
}
