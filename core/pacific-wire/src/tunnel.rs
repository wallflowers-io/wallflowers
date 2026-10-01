//! tunnel — ONE object that owns the Arc↔client stream, on both ends.
//!
//! The same `Tunnel` type runs as `Role::Client` (the phone's pacific-core) and
//! `Role::Arc` (the Arc's plane supervisor). It is SANS-IO in the quinn-proto/h2
//! tradition: the engine owns every byte of protocol state and never touches a
//! socket. A transport — WebSocket today, the BLE mesh later, an in-memory pump
//! in tests — feeds frames in with `handle()`, drains frames out with
//! `poll_out()`, and reads what happened with `poll_event()`. Time is always an
//! argument (`now_ms`), never read from a clock, so every liveness rule is
//! deterministic and testable.
//!
//! Grounding, property by property, in the streaming protocols that proved them:
//!
//!   MULTIPLEXING (HTTP/2 streams, MQTT topics). One connection carries many
//!   channels — MLS envelopes, telemetry, app-defined lanes — as `Data{chan, ..}`
//!   frames. The engine is channel-agnostic: any non-reserved u32 is a lane,
//!   per-channel state is created on first use, and lanes are ordered
//!   independently (a burst of telemetry never blocks an MLS commit).
//!
//!   RESUMABILITY (Kafka consumer offsets, SSE Last-Event-ID). Each direction
//!   of each channel is an append-only sequence numbered from 1. The receiver
//!   holds a cumulative `delivered` cursor; the sender retains everything
//!   unacked. HELLO carries the client's cursors, HELLO-ACK the Arc's, and each
//!   side replays what the other has not seen. Result: exactly-once, in-order
//!   delivery per channel across any number of transport drops — the cursors,
//!   not the socket, are the stream.
//!
//!   BACKPRESSURE (HTTP/2 WINDOW_UPDATE, reactive-streams demand). A sender
//!   spends CREDIT granted by the receiver and blocks (bounded, loudly) at
//!   zero. The receiver replenishes with `consumed()` as the app actually
//!   processes — an Arc can never flood a phone into the jetsam killer.
//!
//!   LIVENESS (WebSocket ping/pong, gRPC keepalive). `tick(now)` emits a ping
//!   after idle and declares `PeerDead` past the deadline — a half-open TCP
//!   carcass is detected by the protocol, not discovered by a hung send.
//!
//!   AUTHENTICATED ATTACH (TLS handshake placement). The FIRST frame is the
//!   client's HELLO carrying its space id and an opaque credential (the
//!   member-tether credential the membership plane mints). The Arc side surfaces
//!   `CredentialPresented` and the embedding plane answers `accept()` or
//!   `reject()`; not one data frame moves before the verdict. Auth happens ONCE
//!   for the whole stream — no lane needs a gate of its own, because everything
//!   on it rides inside a tunnel that was already admitted.
//!
//! The engine never allocates a thread, never sleeps, never retries on its own:
//! policy (when to reconnect, what backoff) belongs to the owner. What the
//! engine guarantees is that WHENEVER a transport exists, the stream picks up
//! exactly where it left off.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::{blob_b64, blob_unb64};

// ---- channels (reserved lanes; anything else is app-defined) -----------------

/// MLS envelopes (deltas, welcomes, commits) — the messaging spine.
pub const CHAN_MLS: u32 = 1;
// Lane 2 is RETIRED: never reuse it. It was `CHAN_INFER`, which carried inference
// bodies until the Arc stopped serving inference. Lane numbers are wire-visible, so
// a build from before the retirement still reads 2 as inference, and reassigning the
// number would feed a new lane's bytes to that old parser. This engine never
// advertises lane 2 and never grants credit on it, so a peer attaching now cannot
// open it. That is the whole enforcement: `handle` does not police lane numbers on
// receive, so a frame that reaches 2 anyway (an older peer spending credit it still
// held from before the retirement) is still delivered, and refusing it is the
// owner's call, not the engine's.
/// Telemetry: the Arc's heartbeat/console feed, folded client-side.
pub const CHAN_TELEM: u32 = 3;

// ---- wire frames -------------------------------------------------------------

/// Everything that crosses the wire. JSON-encoded like the relay's `Frame` (one
/// codec discipline for the whole wire crate); bodies are b64 like relay blobs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub enum TunnelFrame {
    /// Client → Arc, first frame. `resume` is the client's delivered cursor per
    /// channel (what it has already seen of the Arc's sequences); `windows` is
    /// how many frames the peer may send us on each lane (see [`TunnelFrame`]
    /// docs on lane opening).
    Hello {
        space: String,
        /// Opaque credential bytes (b64) — minted by the membership plane onto
        /// the member-tether; verified by the Arc side's owner, never here.
        credential: String,
        /// (chan, delivered) pairs — a Vec, not a map, because JSON object keys
        /// are strings and serde's tagged enums cannot round-trip integer keys.
        resume: Vec<(u32, u64)>,
        /// (chan, window) — the receive window WE grant the peer per lane.
        #[serde(default)]
        windows: Vec<(u32, u32)>,
    },
    /// Arc → client on acceptance. `resume` mirrors back the Arc's delivered
    /// cursors so the client knows what to replay; `windows` opens the client's
    /// send lanes.
    HelloAck {
        session: String,
        resume: Vec<(u32, u64)>,
        #[serde(default)]
        windows: Vec<(u32, u32)>,
    },
    /// One datum on one lane. `seq` starts at 1 per (direction, channel).
    Data { chan: u32, seq: u64, body: String },
    /// Cumulative: every seq ≤ `upto` on `chan` is delivered and may be dropped
    /// from the sender's replay buffer.
    Ack { chan: u32, upto: u64 },
    /// Grant `n` more data frames of credit on `chan`.
    Credit { chan: u32, n: u32 },
    Ping { at: i64 },
    Pong { at: i64 },
    /// Terminal, with the reason stated — a tunnel never just goes quiet.
    Close { reason: String },
}

impl TunnelFrame {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{\"t\":\"close\",\"reason\":\"encode\"}".into())
    }
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
}

// ---- events (what the owner reacts to) ---------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum TunnelEvent {
    /// Arc side: a client presented itself; answer with `accept()`/`reject()`.
    CredentialPresented { space: String, credential: Vec<u8> },
    /// Both sides: the handshake completed; channels are open.
    Attached { session: String },
    /// A datum arrived on a lane, exactly once, in order.
    Data { chan: u32, seq: u64, body: Vec<u8> },
    /// The liveness deadline passed with nothing heard: drop the transport and
    /// reconnect when policy allows — state is retained, resume is free.
    PeerDead,
    /// The peer closed, with its reason.
    Closed { reason: String },
}

// ---- config ------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TunnelConfig {
    /// The receive window this side advertises per lane at attach — how many
    /// frames the peer may send us before its credit must be replenished by our
    /// `consumed()`. Carried in HELLO/HELLO-ACK `windows`, so LANE OPENING IS
    /// PART OF THE HANDSHAKE: no owner has to hand-open a lane, and no lane is
    /// silently unopenable (the bug that stalled lane 2 in production before that
    /// lane was retired, and that CHAN_MLS/CHAN_TELEM would have hit next).
    ///
    /// Applied CURSOR-RELATIVE (`window - unacked`), so re-attaching cannot
    /// inflate the window without bound — the invariant `replay_unacked` keeps.
    pub initial_credit: u32,
    /// Sends held locally waiting on credit before `send` fails loudly.
    pub blocked_cap: usize,
    /// Idle time after which a ping is owed.
    pub idle_ping_ms: i64,
    /// Silence after which the peer is declared dead.
    pub dead_after_ms: i64,
}

impl Default for TunnelConfig {
    fn default() -> Self {
        Self { initial_credit: 32, blocked_cap: 1024, idle_ping_ms: 15_000, dead_after_ms: 45_000 }
    }
}

// ---- errors ------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum TunnelError {
    /// No completed handshake — nothing but HELLO may move.
    NotAttached,
    /// The blocked queue hit its cap: the peer is not consuming. Loud by design.
    CreditExhausted,
    /// The tunnel saw a protocol violation or a Close and will not carry more.
    Closed,
}

// ---- the object --------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Client,
    Arc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Attach {
    /// Fresh or dropped transport: nothing negotiated on this connection yet.
    Idle,
    /// Client: HELLO sent. Arc: HELLO surfaced, awaiting the owner's verdict.
    Pending,
    Attached,
    Closed,
}

/// Send half of one lane: what is unacked (the replay buffer), what waits on
/// credit, and where the sequence stands.
#[derive(Debug, Default)]
struct SendLane {
    next_seq: u64,               // last assigned; 0 = none yet
    unacked: VecDeque<(u64, Vec<u8>)>,
    blocked: VecDeque<Vec<u8>>,
    credit: u32,
}

/// Receive half of one lane: the cumulative cursor.
#[derive(Debug, Default)]
struct RecvLane {
    delivered: u64,
}

/// THE tunnel. One per peer relationship, LONG-LIVED: it survives transport
/// drops (state is the stream; the socket is a detail). Owner pattern per role:
///
///   Client: `connect()` on every fresh transport → pump frames → on `PeerDead`
///           drop the socket, keep the object, `connect()` again when policy
///           says so. Cursors make the re-attach a replay, not a restart.
///   Arc:    one `Tunnel` per attached space, held by the plane supervisor;
///           `CredentialPresented` is answered by the membership plane.
pub struct Tunnel {
    role: Role,
    cfg: TunnelConfig,
    state: Attach,
    session: String,
    /// Arc side only: the pending HELLO's resume cursors, held until accept().
    pending_resume: Vec<(u32, u64)>,
    /// Arc side only: the pending HELLO's advertised receive windows.
    pending_windows: Vec<(u32, u32)>,
    send: BTreeMap<u32, SendLane>,
    recv: BTreeMap<u32, RecvLane>,
    out: VecDeque<TunnelFrame>,
    events: VecDeque<TunnelEvent>,
    last_rx: i64,
    last_tx: i64,
}

impl Tunnel {
    pub fn new(role: Role, cfg: TunnelConfig) -> Self {
        Self {
            role,
            cfg,
            state: Attach::Idle,
            session: String::new(),
            pending_resume: Vec::new(),
            pending_windows: Vec::new(),
            send: BTreeMap::new(),
            recv: BTreeMap::new(),
            out: VecDeque::new(),
            events: VecDeque::new(),
            last_rx: 0,
            last_tx: 0,
        }
    }

    pub fn is_attached(&self) -> bool {
        self.state == Attach::Attached
    }

    // -- client: (re)connect ---------------------------------------------------

    /// Client only: a fresh transport exists — present ourselves. Carries the
    /// delivered cursors, so this is both first-connect AND resume; the engine
    /// keeps every lane's state across calls (drop the socket, never the object).
    pub fn connect(&mut self, space: &str, credential: &[u8], now_ms: i64) {
        debug_assert_eq!(self.role, Role::Client, "connect() is the client's verb");
        let resume: Vec<(u32, u64)> =
            self.recv.iter().map(|(c, l)| (*c, l.delivered)).collect();
        // VOID WHATEVER WAS QUEUED FOR THE DEAD SOCKET. `out` is per-CONNECTION
        // state, not stream state: a frame that never reached the old transport must
        // not be written to the new one ahead of HELLO — the peer would see data
        // before attach and close on us. Nothing is lost, because everything that
        // matters is unacked and replays after the handshake (`replay_unacked`).
        self.out.clear();
        self.state = Attach::Pending;
        self.push(
            TunnelFrame::Hello {
                space: space.to_string(),
                credential: blob_b64(credential),
                resume,
                windows: self.windows(),
            },
            now_ms,
        );
    }

    // -- arc: the verdict ------------------------------------------------------

    /// Arc only: the membership plane vouched for the presented credential.
    /// Replies HELLO-ACK with our cursors and replays what the client missed.
    pub fn accept(&mut self, session: &str, now_ms: i64) {
        debug_assert_eq!(self.role, Role::Arc, "accept() is the Arc's verb");
        if self.state != Attach::Pending {
            return;
        }
        self.session = session.to_string();
        let resume: Vec<(u32, u64)> =
            self.recv.iter().map(|(c, l)| (*c, l.delivered)).collect();
        self.push(
            TunnelFrame::HelloAck { session: session.to_string(), resume, windows: self.windows() },
            now_ms,
        );
        self.state = Attach::Attached;
        self.events.push_back(TunnelEvent::Attached { session: session.to_string() });
        let peer_windows = std::mem::take(&mut self.pending_windows);
        self.apply_windows(&peer_windows);
        let peer_cursors = std::mem::take(&mut self.pending_resume);
        self.replay_unacked(&peer_cursors, now_ms);
    }

    /// Arc only: the credential did not verify. States the reason and closes —
    /// an unauthenticated peer learns WHY, not silence.
    pub fn reject(&mut self, reason: &str, now_ms: i64) {
        debug_assert_eq!(self.role, Role::Arc, "reject() is the Arc's verb");
        self.push(TunnelFrame::Close { reason: reason.to_string() }, now_ms);
        self.state = Attach::Closed;
    }

    // -- sending ---------------------------------------------------------------

    /// Queue one datum on a lane. Returns its sequence number. Blocked-on-credit
    /// sends are buffered (bounded) and flushed the moment credit arrives.
    pub fn send(&mut self, chan: u32, body: &[u8], now_ms: i64) -> Result<u64, TunnelError> {
        match self.state {
            Attach::Attached => {}
            Attach::Closed => return Err(TunnelError::Closed),
            _ => return Err(TunnelError::NotAttached),
        }
        let cap = self.cfg.blocked_cap;
        let lane = self.send.entry(chan).or_default();
        if lane.credit == 0 && lane.blocked.len() >= cap {
            return Err(TunnelError::CreditExhausted);
        }
        // ONE emit path: queue, then drain what credit allows. The seq is assigned
        // at emit, so ordering is the queue's order and there is a single place
        // where a sequence number is minted.
        lane.blocked.push_back(body.to_vec());
        let predicted = lane.next_seq + lane.unacked.len() as u64 + lane.blocked.len() as u64;
        self.flush_blocked(chan, now_ms);
        Ok(predicted)
    }

    /// The app has actually processed `n` frames from `chan`: grant the peer
    /// that much more room. THIS is the backpressure loop — call it from the
    /// consumer, not the transport.
    pub fn consumed(&mut self, chan: u32, n: u32, now_ms: i64) {
        if self.state != Attach::Attached || n == 0 {
            return;
        }
        self.push(TunnelFrame::Credit { chan, n }, now_ms);
    }

    // -- receiving -------------------------------------------------------------

    /// Feed one frame off the transport.
    pub fn handle(&mut self, frame: TunnelFrame, now_ms: i64) {
        if self.state == Attach::Closed {
            return;
        }
        self.last_rx = now_ms;
        match frame {
            TunnelFrame::Hello { space, credential, resume, windows } => {
                if self.role != Role::Arc {
                    return self.violation("hello sent to a client", now_ms);
                }
                let cred = blob_unb64(&credential).unwrap_or_default();
                // Same rule on this side: a re-attaching peer means the previous
                // socket is gone, so anything still queued for it is void. The
                // replay after `accept` is what actually restores the stream.
                self.out.clear();
                self.pending_resume = resume;
                self.pending_windows = windows;
                self.state = Attach::Pending;
                self.events.push_back(TunnelEvent::CredentialPresented { space, credential: cred });
            }
            TunnelFrame::HelloAck { session, resume, windows } => {
                if self.role != Role::Client || self.state != Attach::Pending {
                    return self.violation("unexpected helloAck", now_ms);
                }
                self.session = session.clone();
                self.state = Attach::Attached;
                self.events.push_back(TunnelEvent::Attached { session });
                self.apply_windows(&windows);
                self.replay_unacked(&resume, now_ms);
            }
            TunnelFrame::Data { chan, seq, body } => {
                if self.state != Attach::Attached {
                    return self.violation("data before attach", now_ms);
                }
                let lane = self.recv.entry(chan).or_default();
                if seq <= lane.delivered {
                    return; // replay duplicate after a resume: already ours, drop silently
                }
                if seq != lane.delivered + 1 {
                    // An ordered transport cannot reorder; a gap means lost state.
                    return self.violation("sequence gap", now_ms);
                }
                lane.delivered = seq;
                let bytes = blob_unb64(&body).unwrap_or_default();
                self.events.push_back(TunnelEvent::Data { chan, seq, body: bytes });
                self.push(TunnelFrame::Ack { chan, upto: seq }, now_ms);
            }
            TunnelFrame::Ack { chan, upto } => {
                if let Some(lane) = self.send.get_mut(&chan) {
                    while lane.unacked.front().is_some_and(|(s, _)| *s <= upto) {
                        lane.unacked.pop_front();
                    }
                }
            }
            TunnelFrame::Credit { chan, n } => {
                let lane = self.send.entry(chan).or_default();
                lane.credit = lane.credit.saturating_add(n);
                self.flush_blocked(chan, now_ms);
            }
            TunnelFrame::Ping { at } => self.push(TunnelFrame::Pong { at }, now_ms),
            TunnelFrame::Pong { .. } => {} // last_rx already updated: that was the point
            TunnelFrame::Close { reason } => {
                self.state = Attach::Closed;
                self.events.push_back(TunnelEvent::Closed { reason });
            }
        }
    }

    // -- liveness --------------------------------------------------------------

    /// Drive the clock. Emits a ping when idle; declares the peer dead past the
    /// deadline. On `PeerDead` the OWNER drops the transport; the tunnel object
    /// itself returns to `Idle`, keeping every cursor for the resume.
    pub fn tick(&mut self, now_ms: i64) {
        if self.state != Attach::Attached {
            return;
        }
        if now_ms - self.last_rx >= self.cfg.dead_after_ms {
            // Same rule as `transport_lost`: whatever was queued was for a socket
            // that is demonstrably not carrying anything.
            self.out.clear();
            self.state = Attach::Idle;
            self.events.push_back(TunnelEvent::PeerDead);
            return;
        }
        if now_ms - self.last_tx >= self.cfg.idle_ping_ms {
            self.push(TunnelFrame::Ping { at: now_ms }, now_ms);
        }
    }

    /// THE TRANSPORT IS GONE — the owner's socket died, was superseded, or is being
    /// replaced. Discards anything queued for it and returns to Idle, KEEPING every
    /// cursor and every unacked frame so the next attach replays them.
    ///
    /// This must be called by whichever side notices first, on BOTH ends. `out` is
    /// per-connection state: a frame written to a fresh socket ahead of the handshake
    /// makes the peer see data before attach and close — which is exactly how a
    /// mid-stream drop used to kill the whole tunnel instead of resuming it.
    pub fn transport_lost(&mut self) {
        self.out.clear();
        if self.state == Attach::Attached || self.state == Attach::Pending {
            self.state = Attach::Idle;
        }
    }

    // -- draining --------------------------------------------------------------

    pub fn poll_out(&mut self) -> Option<TunnelFrame> {
        self.out.pop_front()
    }
    pub fn poll_event(&mut self) -> Option<TunnelEvent> {
        self.events.pop_front()
    }
    /// Drain everything pending in one call — what every transport actually wants
    /// (each was hand-writing the same `while let Some(..)` pair under a lock).
    pub fn drain_out(&mut self) -> Vec<TunnelFrame> {
        self.out.drain(..).collect()
    }
    pub fn drain_events(&mut self) -> Vec<TunnelEvent> {
        self.events.drain(..).collect()
    }

    // -- internals -------------------------------------------------------------

    /// The receive windows we advertise: the reserved lanes plus any lane already
    /// in play. Advertising all reserved lanes is what makes every lane openable
    /// without an owner reaching into the protocol.
    fn windows(&self) -> Vec<(u32, u32)> {
        // Lane 2 is retired (see the channel constants), so it is deliberately absent.
        let mut chans: BTreeSet<u32> = [CHAN_MLS, CHAN_TELEM].into_iter().collect();
        chans.extend(self.recv.keys().copied());
        chans.into_iter().map(|c| (c, self.cfg.initial_credit)).collect()
    }

    /// Adopt the peer's advertised windows onto our SEND lanes. Cursor-relative:
    /// frames already sent and unacked count against the window, so a reconnect
    /// re-opens the lane without inflating it.
    fn apply_windows(&mut self, windows: &[(u32, u32)]) {
        for (chan, window) in windows {
            let lane = self.send.entry(*chan).or_default();
            lane.credit = window.saturating_sub(lane.unacked.len() as u32);
        }
        let chans: Vec<u32> = windows.iter().map(|(c, _)| *c).collect();
        for chan in chans {
            self.flush_blocked(chan, self.last_tx);
        }
    }

    /// After an attach, retransmit every send-lane frame the peer's cursor has
    /// not covered. Sequence numbers are original — the receiver's dup/ordering
    /// rules make the replay exactly-once. NO credit is granted here: the engine
    /// never invents credit (a re-grant per reconnect would inflate the window
    /// without bound); only the receiver's `consumed()` opens or widens a lane.
    fn replay_unacked(&mut self, peer_delivered: &[(u32, u64)], now_ms: i64) {
        let frames: Vec<TunnelFrame> = self
            .send
            .iter()
            .flat_map(|(chan, lane)| {
                let seen = peer_delivered
                    .iter()
                    .find(|(c, _)| c == chan)
                    .map(|(_, s)| *s)
                    .unwrap_or(0);
                lane.unacked
                    .iter()
                    .filter(move |(s, _)| *s > seen)
                    .map(|(s, b)| TunnelFrame::Data { chan: *chan, seq: *s, body: blob_b64(b) })
            })
            .collect();
        for f in frames {
            self.push(f, now_ms);
        }
    }

    fn flush_blocked(&mut self, chan: u32, now_ms: i64) {
        loop {
            let Some(lane) = self.send.get_mut(&chan) else { return };
            if lane.credit == 0 || lane.blocked.is_empty() {
                return;
            }
            let body = lane.blocked.pop_front().unwrap();
            lane.credit -= 1;
            lane.next_seq += 1;
            let seq = lane.next_seq;
            lane.unacked.push_back((seq, body.clone()));
            let frame = TunnelFrame::Data { chan, seq, body: blob_b64(&body) };
            self.push(frame, now_ms);
        }
    }

    fn violation(&mut self, reason: &str, now_ms: i64) {
        self.push(TunnelFrame::Close { reason: reason.to_string() }, now_ms);
        self.state = Attach::Closed;
        self.events.push_back(TunnelEvent::Closed { reason: reason.to_string() });
    }

    fn push(&mut self, frame: TunnelFrame, now_ms: i64) {
        self.last_tx = now_ms;
        self.out.push_back(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000;

    /// Move every queued frame a→b and b→a until both are quiet, optionally
    /// dropping everything in flight (a transport failure between two pumps).
    fn pump(a: &mut Tunnel, b: &mut Tunnel, now: i64) {
        loop {
            let mut moved = false;
            while let Some(f) = a.poll_out() {
                b.handle(f, now);
                moved = true;
            }
            while let Some(f) = b.poll_out() {
                a.handle(f, now);
                moved = true;
            }
            if !moved {
                break;
            }
        }
    }

    fn drop_in_flight(t: &mut Tunnel) {
        while t.poll_out().is_some() {}
    }

    fn attach(client: &mut Tunnel, arc: &mut Tunnel, now: i64) {
        client.connect("space1aa", b"member-cred", now);
        pump(client, arc, now);
        match arc.poll_event() {
            Some(TunnelEvent::CredentialPresented { space, credential }) => {
                assert_eq!(space, "space1aa");
                assert_eq!(credential, b"member-cred");
            }
            other => panic!("expected credential, got {other:?}"),
        }
        arc.accept("sess-1", now);
        pump(client, arc, now);
        assert!(matches!(arc.poll_event(), Some(TunnelEvent::Attached { .. })));
        assert!(matches!(client.poll_event(), Some(TunnelEvent::Attached { .. })));
        // NO hand-opening: lane windows ride the handshake, so every reserved lane
        // is usable the moment attach completes. (The rig used to open them here,
        // which is exactly why production's unopened lane went unnoticed.)
    }

    fn drain_data(t: &mut Tunnel) -> Vec<(u32, u64, Vec<u8>)> {
        let mut out = Vec::new();
        while let Some(e) = t.poll_event() {
            if let TunnelEvent::Data { chan, seq, body } = e {
                out.push((chan, seq, body));
            }
        }
        out
    }

    /// The TLS-placement property: not one data frame before the membership
    /// verdict, and the verdict is the OWNER's, not the engine's.
    #[test]
    fn attach_authenticates_before_any_data() {
        let mut client = Tunnel::new(Role::Client, TunnelConfig::default());
        let mut arc = Tunnel::new(Role::Arc, TunnelConfig::default());

        assert_eq!(client.send(CHAN_MLS, b"early", NOW), Err(TunnelError::NotAttached));

        client.connect("space1aa", b"bad-cred", NOW);
        pump(&mut client, &mut arc, NOW);
        assert!(matches!(arc.poll_event(), Some(TunnelEvent::CredentialPresented { .. })));
        arc.reject("not a member of this Arc", NOW);
        pump(&mut client, &mut arc, NOW);
        assert!(matches!(client.poll_event(), Some(TunnelEvent::Closed { reason }) if reason.contains("member")));
        assert_eq!(client.send(CHAN_MLS, b"still early", NOW), Err(TunnelError::Closed));
    }

    /// The Kafka-offset property: a transport death in mid-stream costs
    /// nothing. The SAME two objects re-attach and the cursors replay exactly
    /// the unseen suffix — exactly-once, in order, both directions.
    #[test]
    fn resume_replays_exactly_the_unseen_suffix() {
        let mut client = Tunnel::new(Role::Client, TunnelConfig::default());
        let mut arc = Tunnel::new(Role::Arc, TunnelConfig::default());
        attach(&mut client, &mut arc, NOW);

        // Three delivered envelopes...
        for n in 1..=3u8 {
            client.send(CHAN_MLS, &[n], NOW).unwrap();
        }
        pump(&mut client, &mut arc, NOW);
        assert_eq!(drain_data(&mut arc).len(), 3);

        // ...then two more queued while the transport dies with them in flight.
        client.send(CHAN_MLS, &[4], NOW).unwrap();
        client.send(CHAN_MLS, &[5], NOW).unwrap();
        drop_in_flight(&mut client); // the socket ate them
        // Arc replies in the same outage: telemetry the client never saw.
        arc.send(CHAN_TELEM, b"beat-1", NOW).unwrap();
        drop_in_flight(&mut arc);

        // Same objects, new transport: connect carries cursors, accept replays.
        client.connect("space1aa", b"member-cred", NOW + 10);
        pump(&mut client, &mut arc, NOW + 10);
        assert!(matches!(arc.poll_event(), Some(TunnelEvent::CredentialPresented { .. })));
        arc.accept("sess-2", NOW + 10);
        pump(&mut client, &mut arc, NOW + 10);

        let got = drain_data(&mut arc);
        let mls: Vec<_> = got.iter().filter(|(c, ..)| *c == CHAN_MLS).collect();
        assert_eq!(mls.len(), 2, "exactly the unseen suffix, not a restart: {got:?}");
        assert_eq!((mls[0].1, mls[0].2[0]), (4, 4u8));
        assert_eq!((mls[1].1, mls[1].2[0]), (5, 5u8));

        let back = drain_data(&mut client);
        let telem: Vec<_> = back.iter().filter(|(c, ..)| *c == CHAN_TELEM).collect();
        assert_eq!(telem.len(), 1, "the Arc's unseen frame replays too");
        assert_eq!(telem[0].2, b"beat-1");
    }

    /// The WINDOW_UPDATE property: a sender spends credit and stalls at zero;
    /// the receiver's consumption — not the sender's eagerness — reopens flow.
    #[test]
    fn credit_backpressure_blocks_until_consumed() {
        let cfg = TunnelConfig { initial_credit: 2, ..TunnelConfig::default() };
        let mut client = Tunnel::new(Role::Client, cfg.clone());
        let mut arc = Tunnel::new(Role::Arc, cfg);

        client.connect("space1aa", b"member-cred", NOW);
        pump(&mut client, &mut arc, NOW);
        arc.poll_event();
        arc.accept("sess", NOW);
        pump(&mut client, &mut arc, NOW);
        client.poll_event();
        arc.poll_event();
        // NO manual grant: the attach handshake carried a window of 2 (cfg), which
        // is the whole point — a lane is usable the moment it is attached. The Arc
        // is the sender because the flood this rule exists to stop is an Arc
        // outpacing a phone.
        arc.send(CHAN_TELEM, b"beat-1", NOW).unwrap();
        arc.send(CHAN_TELEM, b"beat-2", NOW).unwrap();
        arc.send(CHAN_TELEM, b"beat-3", NOW).unwrap(); // over credit: buffered
        pump(&mut client, &mut arc, NOW);
        assert_eq!(drain_data(&mut client).len(), 2, "third waits on consumption");

        client.consumed(CHAN_TELEM, 2, NOW);
        pump(&mut client, &mut arc, NOW);
        let late = drain_data(&mut client);
        assert_eq!(late.len(), 1);
        assert_eq!(late[0].2, b"beat-3");

        // BOTH directions, because they open by different paths: the Arc's lanes were
        // bounded by the window it read off HELLO (`accept`), the client's by the one
        // it read off HELLO-ACK. Only this half covers the HELLO-ACK path, and it is
        // the phone's own flow control on the MLS spine.
        client.send(CHAN_MLS, b"m1", NOW).unwrap();
        client.send(CHAN_MLS, b"m2", NOW).unwrap();
        client.send(CHAN_MLS, b"m3", NOW).unwrap(); // over credit: buffered
        pump(&mut client, &mut arc, NOW);
        assert_eq!(drain_data(&mut arc).len(), 2, "HELLO-ACK bounded the client's lane");

        arc.consumed(CHAN_MLS, 2, NOW);
        pump(&mut client, &mut arc, NOW);
        let late_mls = drain_data(&mut arc);
        assert_eq!(late_mls.len(), 1);
        assert_eq!(late_mls[0].2, b"m3");
    }

    /// The keepalive property: silence past the deadline is DETECTED — the
    /// engine says the peer is dead and keeps its state for the resume.
    #[test]
    fn a_half_open_peer_is_declared_dead() {
        let mut client = Tunnel::new(Role::Client, TunnelConfig::default());
        let mut arc = Tunnel::new(Role::Arc, TunnelConfig::default());
        attach(&mut client, &mut arc, NOW);

        client.tick(NOW + 16_000); // idle → ping owed
        assert!(matches!(client.poll_out(), Some(TunnelFrame::Ping { .. })));
        // The pong never comes (half-open socket). Past the deadline: dead.
        client.tick(NOW + 46_000);
        assert!(matches!(client.poll_event(), Some(TunnelEvent::PeerDead)));
        assert!(!client.is_attached(), "back to Idle, cursors intact, ready to connect()");
    }

    /// The HTTP/2-streams property: lanes are independent — sequences, credit,
    /// and ordering are per-channel, and interleaving never cross-blocks.
    #[test]
    fn channels_are_independent_lanes() {
        let mut client = Tunnel::new(Role::Client, TunnelConfig::default());
        let mut arc = Tunnel::new(Role::Arc, TunnelConfig::default());
        attach(&mut client, &mut arc, NOW);

        arc.send(CHAN_MLS, b"m1", NOW).unwrap();
        arc.send(CHAN_TELEM, b"t1", NOW).unwrap();
        arc.send(CHAN_MLS, b"m2", NOW).unwrap();
        pump(&mut client, &mut arc, NOW);

        let got = drain_data(&mut client);
        let mls: Vec<_> = got.iter().filter(|(c, ..)| *c == CHAN_MLS).map(|(_, s, _)| *s).collect();
        let telem: Vec<_> = got.iter().filter(|(c, ..)| *c == CHAN_TELEM).map(|(_, s, _)| *s).collect();
        assert_eq!(mls, vec![1, 2], "MLS numbers its own lane");
        assert_eq!(telem, vec![1], "telemetry numbers its own lane");
    }

    /// Replay duplicates are dropped silently; a true gap is a protocol
    /// violation and closes LOUDLY — an ordered transport cannot reorder.
    #[test]
    fn duplicates_drop_and_gaps_close() {
        let mut client = Tunnel::new(Role::Client, TunnelConfig::default());
        let mut arc = Tunnel::new(Role::Arc, TunnelConfig::default());
        attach(&mut client, &mut arc, NOW);

        client.send(CHAN_MLS, b"m1", NOW).unwrap();
        pump(&mut client, &mut arc, NOW);
        drain_data(&mut arc);

        // A duplicate of seq 1: silent drop, still attached.
        arc.handle(TunnelFrame::Data { chan: CHAN_MLS, seq: 1, body: blob_b64(b"m1") }, NOW);
        assert!(drain_data(&mut arc).is_empty());
        assert!(arc.is_attached());

        // Seq 5 with 2..4 missing: state is lost, the tunnel says so and closes.
        arc.handle(TunnelFrame::Data { chan: CHAN_MLS, seq: 5, body: blob_b64(b"m5") }, NOW);
        assert!(matches!(arc.poll_event(), Some(TunnelEvent::Closed { reason }) if reason.contains("gap")));
    }

    /// Wire round-trip: frames survive the JSON codec byte-for-byte.
    #[test]
    fn frames_round_trip_the_codec() {
        let frames = vec![
            TunnelFrame::Hello {
                space: "space1aa".into(),
                credential: blob_b64(b"cred"),
                resume: vec![(CHAN_MLS, 7u64)],
                windows: vec![(CHAN_MLS, 32), (CHAN_TELEM, 32)],
            },
            TunnelFrame::Data { chan: CHAN_TELEM, seq: 3, body: blob_b64(b"beat") },
            TunnelFrame::Ack { chan: CHAN_MLS, upto: 9 },
            TunnelFrame::Credit { chan: CHAN_TELEM, n: 16 },
            TunnelFrame::Close { reason: "sequence gap".into() },
        ];
        for f in frames {
            assert_eq!(TunnelFrame::from_json(&f.to_json()).unwrap(), f);
        }
    }
}
