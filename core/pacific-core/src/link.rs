//! link — the device's one owner of the network (#144).
//!
//! Every relay operation of this process goes through here: one `Station` per relay URL,
//! each a pool of `POOL` long-lived sessions on the link's own thread and runtime, shared by
//! every Node the process opens (the FFI opens one per call). Before it, each operation
//! dialled a fresh websocket (DNS, TCP, TLS, upgrade) and hung up.
//!
//! ONE EXCHANGE AT A TIME PER SESSION. The relay matches every answer to its request by
//! order on the connection (`Ack` and `Eose` carry no id), and a `Sub` replaces the
//! connection's subscriptions. So a session serves one exchange, start to finish, before the
//! next; concurrency is the pool's width, never interleaving on a socket. A session whose
//! exchange goes wrong mid-way (silent past `IDLE`, closed, a frame out of place) cannot be
//! resynchronised and is dropped; the next exchange dials again.
//!
//! RETRY ONCE, HERE, NOT IN CALLERS. A transient failure (the socket, or a refusal the relay
//! names `rate_limited`, `busy` or `unavailable`) is resent once, with the same bytes. The
//! relay acks a duplicate of the same blob at the same tag as the publish it was, before it
//! looks at a commit slot (relay store.rs, `append_capped_row`), so a resend never publishes
//! twice and a commit that won still reads as won.
//!
//! Callers reach a station through channels, so it is driven from any runtime: the FFI's
//! per-call `block_on`, the Door's actor, a test's.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use pacific_wire::liveness::{Due, Liveness};
use pacific_wire::Frame;
use tokio::sync::{mpsc, oneshot, watch};

use crate::transport::RelaySession;
use crate::CoreError;

/// Sessions per relay: two, so a sync is not queued behind a mint. `LINK_POOL` sets it.
pub const POOL: usize = 2;
/// The longest an exchange waits for its next frame. `LINK_IDLE_MS` sets it (tests).
const IDLE: Duration = Duration::from_secs(15);

fn idle() -> Duration {
    std::env::var("LINK_IDLE_MS").ok().and_then(|s| s.parse().ok()).map(Duration::from_millis).unwrap_or(IDLE)
}
/// The longest a dial may take.
const DIAL: Duration = Duration::from_secs(10);
/// A held session's keepalive: a ping after this much quiet (as `live.rs`), and dead after
/// three of them unanswered.
const PING_MS: i64 = 25_000;
const DEAD_MS: i64 = 75_000;
/// Waits before the one retry: after a socket failure, and after a pacing refusal.
const RETRY_AFTER: Duration = Duration::from_millis(250);
const PACED_AFTER: Duration = Duration::from_secs(2);

/// What a station is asked to do: each one exchange on one session. A publish carries its
/// frame already signed by the caller, so no key reaches the link and a resend is the same
/// bytes.
#[derive(Debug, Clone)]
pub enum Ask {
    /// Be connected: the session a `Router::open` used to dial.
    Ready,
    Publish(Frame),
    Commit(Frame),
    Drain(Vec<String>, u64),
    DrainMany(Vec<(String, u64)>),
    Door(Frame),
    DrainDoors(Vec<(String, u64)>),
}

type Msgs = Vec<(String, u64, String)>;

/// What it answers.
#[derive(Debug)]
pub enum Answer {
    Ready,
    Seq(u64),
    Slot(Option<u64>),
    Msgs(Msgs),
    Many(Vec<Msgs>),
    Doors(Vec<Msgs>, BTreeMap<String, u64>),
}

/// Whether this process can reach a relay: one state, observed, not inferred from errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkState {
    /// Dialling, with no session up.
    Connecting { attempt: u32 },
    /// At least one session is up.
    Connected,
    /// The last dial failed and no session is up.
    Offline { since_ms: i64, why: String },
}

type Job = (Ask, oneshot::Sender<Result<Answer, CoreError>>);

struct Inner {
    url: String,
    jobs: mpsc::UnboundedSender<Job>,
    /// Sessions up now.
    up: AtomicUsize,
    state: watch::Sender<LinkState>,
}

/// One relay, as this process holds it. Cheap to clone: a handle.
#[derive(Clone)]
pub struct Station(Arc<Inner>);

struct Link {
    rt: tokio::runtime::Handle,
    stations: Mutex<HashMap<String, Station>>,
}

fn link() -> &'static Link {
    static LINK: OnceLock<Link> = OnceLock::new();
    LINK.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("link".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("the link's runtime");
                tx.send(rt.handle().clone()).expect("the link's handle");
                rt.block_on(std::future::pending::<()>());
            })
            .expect("the link's thread");
        Link { rt: rx.recv().expect("the link's handle"), stations: Mutex::default() }
    })
}

fn pool() -> usize {
    std::env::var("LINK_POOL").ok().and_then(|s| s.parse().ok()).filter(|n| *n > 0).unwrap_or(POOL)
}

/// This process's station for the relay at `url`: made on first use, kept for the process.
pub fn station(url: &str) -> Station {
    let l = link();
    let mut stations = l.stations.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(s) = stations.get(url) {
        return s.clone();
    }
    let (jobs, rx) = mpsc::unbounded_channel();
    let (state, _) = watch::channel(LinkState::Connecting { attempt: 0 });
    let s = Station(Arc::new(Inner { url: url.to_string(), jobs, up: AtomicUsize::new(0), state }));
    let rx = Arc::new(tokio::sync::Mutex::new(rx));
    for _ in 0..pool() {
        l.rt.spawn(worker(s.clone(), rx.clone()));
    }
    stations.insert(url.to_string(), s.clone());
    s
}

/// Every station's URL and state.
pub fn states() -> Vec<(String, LinkState)> {
    let l = link();
    let stations = l.stations.lock().unwrap_or_else(|p| p.into_inner());
    stations.values().map(|s| (s.0.url.clone(), s.state())).collect()
}

impl Station {
    pub fn url(&self) -> &str {
        &self.0.url
    }

    pub fn state(&self) -> LinkState {
        self.0.state.borrow().clone()
    }

    /// The state as it changes.
    pub fn watch(&self) -> watch::Receiver<LinkState> {
        self.0.state.subscribe()
    }

    /// Run one exchange.
    pub async fn ask(&self, ask: Ask) -> Result<Answer, CoreError> {
        let (tx, rx) = oneshot::channel();
        self.0.jobs.send((ask, tx)).map_err(|_| CoreError::Transport("the link has stopped".into()))?;
        rx.await.map_err(|_| CoreError::Transport("the link dropped an exchange".into()))?
    }

    /// Connected now, or after one dial: what opening a session meant before the link. A
    /// station with a session up answers at once.
    pub async fn ready(&self) -> Result<(), CoreError> {
        if self.0.up.load(Ordering::Relaxed) > 0 {
            return Ok(());
        }
        self.ask(Ask::Ready).await.map(|_| ())
    }

    fn set(&self, state: LinkState) {
        self.0.state.send_if_modified(|s| {
            let changed = *s != state;
            *s = state;
            changed
        });
    }

    fn session_up(&self) {
        self.0.up.fetch_add(1, Ordering::Relaxed);
        self.set(LinkState::Connected);
    }

    fn session_down(&self) {
        if self.0.up.fetch_sub(1, Ordering::Relaxed) == 1 {
            self.set(LinkState::Connecting { attempt: 0 });
        }
    }

    fn dial_failed(&self, attempt: u32, why: &CoreError) {
        if self.0.up.load(Ordering::Relaxed) == 0 {
            let since_ms = match self.state() {
                LinkState::Offline { since_ms, .. } => since_ms,
                _ => now_ms(),
            };
            self.set(if attempt == 0 {
                LinkState::Connecting { attempt: 1 }
            } else {
                LinkState::Offline { since_ms, why: why.to_string() }
            });
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// A session this worker holds, and its keepalive clocks.
struct Held {
    sess: RelaySession,
    live: Liveness,
}

async fn dial(url: &str) -> Result<Held, CoreError> {
    let mut sess = tokio::time::timeout(DIAL, RelaySession::connect(url))
        .await
        .map_err(|_| CoreError::Transport(format!("connect {url}: no answer in {}s", DIAL.as_secs())))??
        .with_idle(idle());
    sess.declare(pacific_wire::V_DOOR).await?;
    Ok(Held { sess, live: Liveness::new(PING_MS, DEAD_MS, now_ms()) })
}

/// How a failed exchange is to be taken.
enum Failure {
    /// The socket is unusable: drop it, and retry on a new one.
    Socket,
    /// The relay named a pace or availability refusal: the socket is fine; retry later.
    Paced,
    /// Anything else: the answer, as it is.
    Final,
}

/// Transient pacing refusals the relay names (`limits.rs`, relay `lib.rs`).
const PACED: [&str; 3] = ["rate_limited", "busy", "unavailable"];

fn classify(e: &CoreError) -> Failure {
    let CoreError::Transport(m) = e else { return Failure::Final };
    if let Some((_, reason)) = m.split_once("the relay refused").and_then(|(_, r)| r.split_once(": ")) {
        return if PACED.contains(&reason.trim()) { Failure::Paced } else { Failure::Final };
    }
    Failure::Socket
}

async fn exchange(h: &mut Held, ask: &Ask) -> Result<Answer, CoreError> {
    let s = &mut h.sess;
    let answer = match ask {
        Ask::Ready => Answer::Ready,
        Ask::Publish(f) => Answer::Seq(s.publish_frame(f).await?),
        Ask::Commit(f) => Answer::Slot(s.publish_commit_frame(f).await?),
        Ask::Drain(tags, since) => Answer::Msgs(s.drain(tags.clone(), *since).await?),
        Ask::DrainMany(subs) => Answer::Many(s.drain_many(subs).await?),
        Ask::Door(f) => Answer::Seq(s.publish_door_frame(f).await?),
        Ask::DrainDoors(subs) => {
            let (m, e) = s.drain_doors(subs).await?;
            Answer::Doors(m, e)
        }
    };
    let now = now_ms();
    h.live.heard(now);
    h.live.sent(now);
    Ok(answer)
}

/// One of a station's sessions: takes the next exchange, keeps its socket alive between them.
async fn worker(st: Station, jobs: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<Job>>>) {
    let mut held: Option<Held> = None;
    loop {
        let (ask, reply) = match next_job(&st, &jobs, &mut held).await {
            Some(job) => job,
            None => return,
        };
        let _ = reply.send(run(&st, &mut held, &ask).await);
    }
}

/// Wait for the next exchange; meanwhile hear the held socket, ping it when due, and drop it
/// when its peer has gone quiet past the deadline.
async fn next_job(
    st: &Station,
    jobs: &tokio::sync::Mutex<mpsc::UnboundedReceiver<Job>>,
    held: &mut Option<Held>,
) -> Option<Job> {
    loop {
        let Some(h) = held.as_mut() else {
            return jobs.lock().await.recv().await;
        };
        match h.live.due(now_ms()) {
            Due::Dead => {
                *held = None;
                st.session_down();
                continue;
            }
            Due::Ping => {
                if h.sess.ping().await.is_err() {
                    *held = None;
                    st.session_down();
                    continue;
                }
                h.live.sent(now_ms());
            }
            Due::Nothing => {}
        }
        tokio::select! {
            job = async { jobs.lock().await.recv().await } => return job,
            heard = h.sess.idle_read() => match heard {
                Ok(()) => h.live.heard(now_ms()),
                Err(_) => {
                    *held = None;
                    st.session_down();
                }
            },
            _ = tokio::time::sleep(Duration::from_millis(PING_MS as u64)) => {}
        }
    }
}

/// One exchange, retried once if it failed transiently.
async fn run(st: &Station, held: &mut Option<Held>, ask: &Ask) -> Result<Answer, CoreError> {
    let mut last = None;
    for attempt in 0..2u32 {
        if held.is_none() {
            match dial(st.url()).await {
                Ok(h) => {
                    *held = Some(h);
                    st.session_up();
                }
                Err(e) => {
                    st.dial_failed(attempt, &e);
                    // `Ready` is a question about now: answered by one dial, never retried.
                    if attempt == 0 && !matches!(ask, Ask::Ready) {
                        tokio::time::sleep(RETRY_AFTER).await;
                        last = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }
        let h = held.as_mut().expect("held after the dial");
        match exchange(h, ask).await {
            Ok(a) => return Ok(a),
            Err(e) => {
                let failure = classify(&e);
                if matches!(failure, Failure::Socket) {
                    *held = None;
                    st.session_down();
                }
                match failure {
                    Failure::Final => return Err(e),
                    _ if attempt == 1 => return Err(e),
                    Failure::Socket => tokio::time::sleep(RETRY_AFTER).await,
                    Failure::Paced => tokio::time::sleep(PACED_AFTER).await,
                }
                last = Some(e);
            }
        }
    }
    Err(last.unwrap_or_else(|| CoreError::Transport("the link gave no answer".into())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_pace_refusal_is_retried_and_a_full_store_is_not() {
        let t = |m: &str| CoreError::Transport(m.to_string());
        assert!(matches!(classify(&t("the relay refused a publish: rate_limited")), Failure::Paced));
        assert!(matches!(classify(&t("the relay refused a commit: busy")), Failure::Paced));
        assert!(matches!(classify(&t("the relay refused a publish: full")), Failure::Final));
        assert!(matches!(classify(&t("the relay refused a knock: too_large")), Failure::Final));
        assert!(matches!(classify(&t("relay closed")), Failure::Socket));
        assert!(matches!(classify(&t("the relay was silent for 15s")), Failure::Socket));
        assert!(matches!(classify(&CoreError::Directory("x".into())), Failure::Final));
    }
}
