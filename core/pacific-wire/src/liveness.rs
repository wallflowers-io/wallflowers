//! liveness — when a connection owes a ping, and when its peer is dead.
//!
//! SANS-IO, as `tunnel`: time is an argument, never read from a clock. One rule for
//! every long-lived connection a device holds (the Tunnel, the link's relay sessions):
//! silence past `dead_after_ms` is DETECTED, so a half-open TCP carcass is found by
//! the protocol, not by a send that hangs.

/// What a connection owes at `now_ms`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    /// Nothing: it has spoken or heard recently enough.
    Nothing,
    /// Idle past `idle_ping_ms`: send a ping.
    Ping,
    /// Nothing heard past `dead_after_ms`: drop the transport.
    Dead,
}

/// The two clocks a connection keeps, and its two thresholds.
#[derive(Debug, Clone)]
pub struct Liveness {
    idle_ping_ms: i64,
    dead_after_ms: i64,
    last_rx: i64,
    last_tx: i64,
}

impl Liveness {
    /// A connection opened at `now_ms`: heard and spoken to then.
    pub fn new(idle_ping_ms: i64, dead_after_ms: i64, now_ms: i64) -> Self {
        Self { idle_ping_ms, dead_after_ms, last_rx: now_ms, last_tx: now_ms }
    }

    /// Something arrived from the peer.
    pub fn heard(&mut self, now_ms: i64) {
        self.last_rx = now_ms;
    }

    /// Something was sent to the peer.
    pub fn sent(&mut self, now_ms: i64) {
        self.last_tx = now_ms;
    }

    /// When something was last sent.
    pub fn last_sent(&self) -> i64 {
        self.last_tx
    }

    /// What is owed now. Dead takes precedence over a ping.
    pub fn due(&self, now_ms: i64) -> Due {
        if now_ms - self.last_rx >= self.dead_after_ms {
            Due::Dead
        } else if now_ms - self.last_tx >= self.idle_ping_ms {
            Due::Ping
        } else {
            Due::Nothing
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_owes_a_ping_and_silence_past_the_deadline_is_dead() {
        let mut l = Liveness::new(15_000, 45_000, 0);
        assert_eq!(l.due(14_999), Due::Nothing);
        assert_eq!(l.due(15_000), Due::Ping);
        l.sent(15_000);
        assert_eq!(l.due(20_000), Due::Nothing);
        assert_eq!(l.due(45_000), Due::Dead, "a ping sent is not a pong heard");
        l.heard(44_000);
        assert_eq!(l.due(45_000), Due::Ping, "heard, but idle since the last send: a ping is still owed");
        l.sent(45_000);
        assert_eq!(l.due(45_001), Due::Nothing);
    }
}
