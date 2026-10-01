//! The FFI seam for the device-to-device mesh.
//!
//! Deliberately tiny, and deliberately dumb on the Swift side. Core Bluetooth is the only part
//! of the mesh that HAS to be Swift — everything else (framing, reassembly, dedupe, carrying,
//! eviction, the exchange itself) stays in Rust where it is testable without a radio. So this
//! boundary is two calls: "what do I say to a peer that just appeared", and "here is what a peer
//! said, what do I say back". A `MeshLink` is one conversation with one peer in range; the store
//! it reads and writes is the device's single shared one, so a blob picked up from a stranger on
//! the train is there for the next encounter and for this device's own sync.

use std::sync::Mutex;

use pacific_core::mesh;
use pacific_core::mesh_link::{self, FrameReader};
use pacific_core::mesh_policy::{Admit, Close, MeshPolicy};

use crate::FfiError;

/// One live conversation with one peer in range. Create on connect, drop on disconnect — the
/// reassembly buffer is per-peer, because two peers chunk independently and interleaving their
/// bytes into one buffer would corrupt both.
#[derive(uniffi::Object)]
pub struct MeshLink {
    reader: Mutex<FrameReader>,
}

#[uniffi::export]
impl MeshLink {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self {
            reader: Mutex::new(FrameReader::default()),
        }
    }

    /// The opening frame to write when a peer comes into range: what this device is carrying.
    /// Bytes are already length-prefixed; the radio may chunk them however it likes.
    pub fn greeting(&self) -> Result<Vec<u8>, FfiError> {
        let mut store = mesh::shared()
            .lock()
            .map_err(|_| FfiError::Core("mesh store lock poisoned".into()))?;
        Ok(mesh_link::greet(&mut store).encode())
    }

    /// Feed whatever the radio just received from this peer; get back whatever to write in reply
    /// (empty when there is nothing to say, which is the normal end of an exchange).
    ///
    /// Chunk boundaries are irrelevant — frames are length-prefixed and reassembled here, so the
    /// caller passes on exactly what arrived without buffering or interpreting any of it.
    pub fn receive(&self, bytes: Vec<u8>) -> Result<Vec<u8>, FfiError> {
        let frames = {
            let mut reader = self
                .reader
                .lock()
                .map_err(|_| FfiError::Core("mesh link reader poisoned".into()))?;
            reader.push(&bytes)
        };
        if frames.is_empty() {
            return Ok(Vec::new());
        }
        let now = now_secs();
        let mut store = mesh::shared()
            .lock()
            .map_err(|_| FfiError::Core("mesh store lock poisoned".into()))?;
        let mut out = Vec::new();
        for f in frames {
            if let Some(reply) = mesh_link::respond(&mut store, f, now) {
                out.extend_from_slice(&reply.encode());
            }
        }
        Ok(out)
    }

    /// True once this peer has sent something that cannot be a frame. The radio should drop the
    /// connection: a byte stream that lost sync can never regain it, and continuing to read one
    /// is how a hostile peer keeps a link (and a radio) busy for nothing.
    pub fn is_poisoned(&self) -> bool {
        self.reader.lock().map(|r| r.is_poisoned()).unwrap_or(true)
    }
}

impl Default for MeshLink {
    fn default() -> Self {
        Self::new()
    }
}

/// The link scheduler — the battery half of the mesh. Every decision about whether to connect,
/// how many links to hold, and when to hang up is made in Rust against `mesh_policy`, so the
/// radio never has to encode a power trade-off it cannot test. Clocks are read here rather than
/// passed from Swift: one clock, no skew between a decision and its record.
#[derive(uniffi::Object)]
pub struct MeshPolicyHandle {
    inner: Mutex<MeshPolicy>,
}

#[uniffi::export]
impl MeshPolicyHandle {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(MeshPolicy::default()),
        }
    }

    /// Connect to this freshly discovered peer? `rssi` is the discovery reading in dBm.
    /// False for a peer on cooldown, one too weak to be worth the attempt, one already being
    /// handled, a connect made too recently, or a radio already at its link ceiling.
    pub fn admit(&self, peer: String, rssi: i16) -> bool {
        let yes = self
            .with(|p, now| matches!(p.admit(&peer, rssi, now), Admit::Yes))
            .unwrap_or(false);
        if yes {
            // Start the global rate-limit clock on the ATTEMPT, not on success — a connect that
            // times out costs the same radio as one that works.
            let _ = self.with(|p, now| p.attempting(now));
        }
        yes
    }

    pub fn opened(&self, peer: String) {
        let _ = self.with(|p, now| p.opened(&peer, now));
    }

    /// Bytes moved — keeps the link alive and cancels any pending hang-up.
    pub fn activity(&self, peer: String) {
        let _ = self.with(|p, now| p.activity(&peer, now));
    }

    /// Nothing further to say. Starts the linger, not an immediate close.
    pub fn quiet(&self, peer: String) {
        let _ = self.with(|p, now| p.quiet(&peer, now));
    }

    /// Peers whose linger has elapsed — disconnect these. Poll on the radio's tick.
    pub fn hangups(&self) -> Vec<String> {
        self.with(|p, now| {
            p.prune(now);
            p.hangups(now)
        })
        .unwrap_or_default()
    }

    /// A link ended. `reason` decides how long before the peer is worth dialling again:
    /// "completed" (we finished and hung up), "dropped" (they went out of range — they usually
    /// come back, so barely any wait) or "timeout" (never answered — back off, progressively).
    pub fn closed(&self, peer: String, reason: String) {
        let how = match reason.as_str() {
            "timeout" => Close::TimedOut,
            "dropped" => Close::Dropped,
            _ => Close::Completed,
        };
        let _ = self.with(|p, now| p.closed(&peer, how, now));
    }

    pub fn open_links(&self) -> u32 {
        self.with(|p, _| p.open_links() as u32).unwrap_or(0)
    }

    /// Should the radio be scanning right now? Duty-cycled in EVERY state — the background is the
    /// mode that runs all day, so leaving it flat out was the expensive mistake. Forced fully on
    /// for a window after any traffic, and tightened once the graph is dense.
    pub fn should_scan(&self) -> bool {
        self.with(|p, now| p.should_scan(now)).unwrap_or(true)
    }

    /// The signal floor the radio should apply to discoveries right now, in dBm. Rises when we
    /// have company and falls the longer we are alone.
    pub fn rssi_gate(&self) -> i16 {
        self.with(|p, now| p.rssi_gate(now)).unwrap_or(-90)
    }
}

impl MeshPolicyHandle {
    /// Take the lock and stamp ONE clock reading for the whole decision. Generic, so it cannot
    /// live in the `#[uniffi::export]` block above — UniFFI has no way to expose a generic.
    fn with<T>(&self, f: impl FnOnce(&mut MeshPolicy, u64) -> T) -> Option<T> {
        let mut guard = self.inner.lock().ok()?;
        Some(f(&mut guard, now_ms()))
    }
}

impl Default for MeshPolicyHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// How many blobs this device is currently carrying for the mesh — its own and other people's.
/// The one honest number for a "mesh is working" indicator: it counts what this phone would
/// hand on if it met someone right now.
#[uniffi::export]
pub fn mesh_carried() -> u64 {
    mesh::shared().lock().map(|s| s.len() as u64).unwrap_or(0) as u64
}

/// Milliseconds to leave between consecutive BLE writes.
///
/// bitchat's hard-won number: "Conservative spacing to prevent BLE buffer overflow. Aggressive
/// pacing causes packet loss; needs 25-30ms between fragments for reliable delivery." iOS's own
/// `canSendWriteWithoutResponse` back-pressure is NOT sufficient in the field, and a dropped chunk
/// does not merely lose a message here — it desynchronises a length-prefixed stream and poisons
/// the link.
#[uniffi::export]
pub fn mesh_write_spacing_ms() -> u32 {
    30
}

/// The largest write the radio should attempt per chunk, given a negotiated MTU. Kept here so
/// the chunking rule lives with the framing rather than being guessed at each call site.
#[uniffi::export]
pub fn mesh_chunk(bytes: Vec<u8>, mtu: u32) -> Vec<Vec<u8>> {
    mesh_link::chunks(&bytes, mtu.max(1) as usize)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The scheduler's clock. Milliseconds, because the connect rate limit is sub-second.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
