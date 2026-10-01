//! mesh_policy — when to look, who to talk to, and when to hang up.
//!
//! The mesh EXCHANGE is already cheap: two converged devices trade a couple of frames and fall
//! silent. What costs battery is everything around it — how often the radio looks for peers, how
//! many links are open, which peers are worth dialling at all, and how long a link is held after
//! there is nothing left to say. None of that is protocol, all of it is scheduling, and scheduling
//! written directly against Core Bluetooth is scheduling nobody can test. So it lives here.
//!
//! THE NUMBERS ARE BITCHAT'S, not invented. bitchat (`bitchat/Services/TransportConfig.swift` and
//! `BLEConnectionScheduler.swift`) is the most-shipped iOS BLE mesh there is, and its constants are
//! field-tuned rather than reasoned out. Four of its lessons are load-bearing here:
//!
//!   * **A failed connect and a lost connection are different events.** bitchat tracks
//!     `recentConnectTimeouts` apart from `recentDisconnects`, because a peer "we held a connection
//!     with and lost (walked out of range) usually comes back, so it only gets a brief rediscovery
//!     ignore — not the timeout backoff/cooldown treatment reserved for peers that never answered".
//!     One undifferentiated cooldown punishes the person sitting next to you on the bus for
//!     briefly losing signal.
//!   * **Filter by signal before spending a connect.** A peer at −100 dBm mostly times out, and a
//!     timed-out connect costs more radio than the exchange would have. bitchat gates on RSSI and
//!     RELAXES the gate when isolated — tight when you have company, desperate when you are alone.
//!   * **Rate-limit connects globally**, not just cap them. A cap bounds concurrency; a room full
//!     of phones still produces a burst of attempts without a floor on the interval between them.
//!   * **Duty-cycle the scan always, and force it on when something is happening.** A fixed cycle
//!     is blind; bitchat keeps the scan fully on for 10 s after any traffic and tightens the cycle
//!     when the graph is dense.
//!
//! Time is MILLISECONDS here, because the connect rate limit is sub-second.

use std::collections::HashMap;

/// After a completed exchange. Short: the value of meeting the same person again is low for a
/// while, but "a while" is a bus stop, not a lunch break. (bitchat's weak-link cooldown is 30 s.)
pub const COOLDOWN_MS: u64 = 30_000;
/// After a peer simply went away. They usually come back — this is a rediscovery debounce, not a
/// punishment, and it exists only so a flapping link does not reconnect every discovery callback.
pub const REDISCOVERY_IGNORE_MS: u64 = 5_000;
/// After a connect that never answered. The peer is out of usable range even if we can hear it.
pub const WEAK_LINK_BACKOFF_MS: u64 = 30_000;
/// Concurrent links. bitchat runs 6; BLE links contend, so this is a real ceiling, not a target.
pub const MAX_LINKS: usize = 6;
/// Floor on the interval between connect ATTEMPTS, globally. bitchat: 0.5 s.
pub const CONNECT_RATE_LIMIT_MS: u64 = 500;
/// Grace after we run out of things to say, in case the peer has not.
pub const LINGER_MS: u64 = 2_000;
/// A link that has moved no bytes at all for this long is dead weight — a peer that connected and
/// then said nothing (crashed, wedged, or lost its own radio) otherwise holds a slot forever,
/// because the linger only ever starts once WE have spoken and fallen silent.
pub const STALL_MS: u64 = 15_000;

/// Signal gates (dBm), from bitchat's `bleDynamicRSSIThreshold*` family.
pub const RSSI_DEFAULT: i16 = -90;
/// Once we have company, be choosier — a marginal peer is not worth a link slot.
pub const RSSI_CONNECTED: i16 = -85;
/// Alone, take what we can get, and get less fussy the longer it lasts.
pub const RSSI_ISOLATED_BASE: i16 = -95;
pub const RSSI_ISOLATED_RELAXED: i16 = -100;
/// How long isolated before the gate opens all the way.
pub const ISOLATION_RELAX_MS: u64 = 60_000;

/// Scan duty cycle. bitchat: 5 s on / 10 s off, tightening to 3 s / 15 s when the graph is dense.
pub const SCAN_ON_MS: u64 = 5_000;
pub const SCAN_PERIOD_MS: u64 = 15_000;
pub const SCAN_ON_DENSE_MS: u64 = 3_000;
pub const SCAN_PERIOD_DENSE_MS: u64 = 18_000;
/// Links at which the graph counts as dense enough to scan less.
pub const DENSE_LINKS: usize = 3;
/// Keep the scan fully ON this long after any traffic — something is happening, and a duty cycle
/// that sleeps through an active encounter costs more (a missed peer) than it saves.
pub const RECENT_TRAFFIC_MS: u64 = 10_000;

/// Whether a discovered peer is worth connecting to right now, and why not when it isn't. The
/// reason is not decoration: "too weak" and "at capacity" want different responses from the radio
/// (keep scanning vs stop bothering), and a policy that only says no cannot express that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admit {
    Yes,
    /// Talked to them recently, or they only just left.
    Cooldown,
    /// Already handling as many peers as the radio should.
    AtCapacity,
    /// Signal too weak to be worth a connect attempt at the moment.
    TooWeak,
    /// A connect was attempted very recently; pace them out.
    RateLimited,
}

/// Why a link ended. The distinction is the whole point — see the module note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Close {
    /// The exchange finished and we hung up. Normal, and the peer is fine.
    Completed,
    /// They went out of range, or the link dropped mid-conversation.
    Dropped,
    /// We dialled and they never answered. The link is not viable right now.
    TimedOut,
}

#[derive(Debug, Clone, Copy)]
struct Link {
    active_at: u64,
    quiet_at: Option<u64>,
}

/// The device's link scheduler. One per radio.
pub struct MeshPolicy {
    open: HashMap<String, Link>,
    /// Peer -> (when it ended, how). Drives the cooldown, whose length depends on the how.
    ended: HashMap<String, (u64, Close)>,
    /// Consecutive failed connects per peer, so a persistently unreachable peer backs off further
    /// rather than being retried at a fixed interval forever.
    failures: HashMap<String, u32>,
    last_connect_ms: u64,
    /// When we last had any link at all — the isolation clock.
    last_company_ms: u64,
    /// When bytes last moved anywhere. Drives force-on scanning.
    last_traffic_ms: u64,
    started_ms: Option<u64>,
}

impl Default for MeshPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl MeshPolicy {
    pub fn new() -> Self {
        Self {
            open: HashMap::new(),
            ended: HashMap::new(),
            failures: HashMap::new(),
            last_connect_ms: 0,
            last_company_ms: 0,
            last_traffic_ms: 0,
            started_ms: None,
        }
    }

    pub fn open_links(&self) -> usize {
        self.open.len()
    }

    /// The RSSI floor right now. Choosier with company; progressively less so when alone, because
    /// a marginal peer is worth a try when it is the only peer there is.
    pub fn rssi_gate(&self, now: u64) -> i16 {
        if !self.open.is_empty() {
            return RSSI_CONNECTED;
        }
        let base = self.started_ms.unwrap_or(now).max(self.last_company_ms);
        let alone_for = now.saturating_sub(base);
        if alone_for >= ISOLATION_RELAX_MS {
            RSSI_ISOLATED_RELAXED
        } else if alone_for >= ISOLATION_RELAX_MS / 2 {
            RSSI_ISOLATED_BASE
        } else {
            RSSI_DEFAULT
        }
    }

    /// Should we connect to this freshly discovered peer? `rssi` is the discovery reading in dBm.
    pub fn admit(&self, peer: &str, rssi: i16, now: u64) -> Admit {
        if self.open.contains_key(peer) {
            return Admit::Cooldown; // already talking to them
        }
        if let Some(&(at, how)) = self.ended.get(peer) {
            let wait = match how {
                // A completed exchange: nothing to gain from an immediate rerun.
                Close::Completed => COOLDOWN_MS,
                // They walked off. Debounce the rediscovery, then welcome them straight back.
                Close::Dropped => REDISCOVERY_IGNORE_MS,
                // Never answered — and back off further each consecutive time.
                Close::TimedOut => {
                    let n = (*self.failures.get(peer).unwrap_or(&1)).clamp(1, 4) as u64;
                    WEAK_LINK_BACKOFF_MS * n
                }
            };
            if now.saturating_sub(at) < wait {
                return Admit::Cooldown;
            }
        }
        if rssi < self.rssi_gate(now) {
            return Admit::TooWeak;
        }
        if self.open.len() >= MAX_LINKS {
            return Admit::AtCapacity;
        }
        if now.saturating_sub(self.last_connect_ms) < CONNECT_RATE_LIMIT_MS {
            return Admit::RateLimited;
        }
        Admit::Yes
    }

    /// A connect is being attempted — starts the global rate-limit clock.
    pub fn attempting(&mut self, now: u64) {
        self.last_connect_ms = now;
        self.started_ms.get_or_insert(now);
    }

    /// A link came up.
    pub fn opened(&mut self, peer: &str, now: u64) {
        self.open.insert(
            peer.to_string(),
            Link {
                active_at: now,
                quiet_at: None,
            },
        );
        self.failures.remove(peer);
        self.ended.remove(peer);
        self.last_company_ms = now;
        self.started_ms.get_or_insert(now);
    }

    /// Bytes moved — the exchange is alive, so cancel any pending hang-up and hold the scan on.
    pub fn activity(&mut self, peer: &str, now: u64) {
        if let Some(l) = self.open.get_mut(peer) {
            l.active_at = now;
            l.quiet_at = None;
        }
        self.last_traffic_ms = now;
    }

    /// We have nothing further to say. Starts the linger, does NOT close: the peer may still be
    /// mid-delivery, and truncating that would cost the whole exchange to save two seconds.
    pub fn quiet(&mut self, peer: &str, now: u64) {
        if let Some(l) = self.open.get_mut(peer) {
            if l.quiet_at.is_none() {
                l.quiet_at = Some(now);
            }
        }
    }

    /// Peers to hang up on: those whose linger has elapsed, and those that have gone silent
    /// altogether. The second is not the same as the first — the linger begins when WE run out of
    /// things to say, so a peer that connects and never speaks would never trigger it.
    pub fn hangups(&self, now: u64) -> Vec<String> {
        self.open
            .iter()
            .filter(|(_, l)| {
                l.quiet_at
                    .is_some_and(|q| now.saturating_sub(q) >= LINGER_MS)
                    || now.saturating_sub(l.active_at) >= STALL_MS
            })
            .map(|(p, _)| p.clone())
            .collect()
    }

    /// A link ended. `how` decides how long before this peer is worth dialling again.
    pub fn closed(&mut self, peer: &str, how: Close, now: u64) {
        if self.open.remove(peer).is_some() {
            self.last_company_ms = now;
        }
        match how {
            Close::TimedOut => *self.failures.entry(peer.to_string()).or_insert(0) += 1,
            _ => {
                self.failures.remove(peer);
            }
        }
        self.ended.insert(peer.to_string(), (now, how));
        if self.open.is_empty() {
            self.started_ms.get_or_insert(now);
        }
    }

    /// Should the radio be scanning right now?
    ///
    /// Duty-cycled in EVERY state, not just the foreground. Leaving the background scan running
    /// flat out is the expensive mistake: it is the mode that runs all day. The exceptions are
    /// deliberate — scan fully while something is actually happening, and tighten the cycle once
    /// the graph is dense enough that the next peer will find us anyway.
    pub fn should_scan(&self, now: u64) -> bool {
        if now.saturating_sub(self.last_traffic_ms) < RECENT_TRAFFIC_MS && self.last_traffic_ms > 0
        {
            return true;
        }
        let (on, period) = if self.open.len() >= DENSE_LINKS {
            (SCAN_ON_DENSE_MS, SCAN_PERIOD_DENSE_MS)
        } else {
            (SCAN_ON_MS, SCAN_PERIOD_MS)
        };
        now % period < on
    }

    /// Forget bookkeeping that has outlived its usefulness, so a busy day does not grow the maps
    /// without bound. A peer past its cooldown is indistinguishable from one never seen.
    pub fn prune(&mut self, now: u64) {
        let longest = WEAK_LINK_BACKOFF_MS * 4;
        self.ended
            .retain(|_, &mut (at, _)| now.saturating_sub(at) < longest);
        self.failures.retain(|p, _| self.ended.contains_key(p));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STRONG: i16 = -60;

    fn policy() -> MeshPolicy {
        let mut p = MeshPolicy::new();
        p.attempting(0);
        p
    }

    #[test]
    fn a_fresh_strong_peer_is_admitted() {
        assert_eq!(policy().admit("a", STRONG, 10_000), Admit::Yes);
    }

    /// bitchat's distinction, and the reason the bus works: a peer who walked out of range comes
    /// straight back, while one who never answered is left alone for far longer.
    #[test]
    fn a_dropped_peer_returns_far_sooner_than_one_that_never_answered() {
        let mut p = policy();
        p.opened("a", 10_000);
        p.closed("a", Close::Dropped, 20_000);
        assert_eq!(p.admit("a", STRONG, 21_000), Admit::Cooldown);
        assert_eq!(p.admit("a", STRONG, 26_000), Admit::Yes, "back in seconds");

        let mut q = policy();
        q.closed("b", Close::TimedOut, 20_000);
        assert_eq!(
            q.admit("b", STRONG, 26_000),
            Admit::Cooldown,
            "not this one"
        );
        assert_eq!(q.admit("b", STRONG, 51_000), Admit::Yes);
    }

    #[test]
    fn a_completed_exchange_cools_down_but_not_for_long() {
        let mut p = policy();
        p.opened("a", 10_000);
        p.closed("a", Close::Completed, 10_000);
        assert_eq!(p.admit("a", STRONG, 39_000), Admit::Cooldown);
        assert_eq!(p.admit("a", STRONG, 41_000), Admit::Yes);
    }

    /// A peer that never answers must back off further each time, not be retried forever at a
    /// fixed interval.
    #[test]
    fn repeated_timeouts_back_off_progressively() {
        let mut p = policy();
        p.closed("a", Close::TimedOut, 0);
        assert_eq!(p.admit("a", STRONG, WEAK_LINK_BACKOFF_MS + 1), Admit::Yes);
        p.closed("a", Close::TimedOut, 0);
        assert_eq!(
            p.admit("a", STRONG, WEAK_LINK_BACKOFF_MS + 1),
            Admit::Cooldown
        );
        assert_eq!(
            p.admit("a", STRONG, 2 * WEAK_LINK_BACKOFF_MS + 1),
            Admit::Yes
        );
    }

    #[test]
    fn a_successful_link_clears_the_failure_history() {
        let mut p = policy();
        p.closed("a", Close::TimedOut, 0);
        p.closed("a", Close::TimedOut, 0);
        p.opened("a", 100_000);
        p.closed("a", Close::Dropped, 100_000);
        assert_eq!(p.admit("a", STRONG, 106_000), Admit::Yes, "forgiven");
    }

    /// A connect attempt costs more than the exchange when the peer is out of usable range.
    #[test]
    fn a_weak_peer_is_not_worth_a_connect() {
        let p = policy();
        assert_eq!(p.admit("a", -95, 1_000), Admit::TooWeak);
        assert_eq!(p.admit("a", -80, 1_000), Admit::Yes);
    }

    /// …unless it is the only peer there is. Alone, the gate opens progressively.
    #[test]
    fn the_signal_gate_relaxes_when_isolated() {
        let mut p = MeshPolicy::new();
        p.attempting(0);
        assert_eq!(p.rssi_gate(0), RSSI_DEFAULT);
        assert_eq!(p.rssi_gate(ISOLATION_RELAX_MS / 2), RSSI_ISOLATED_BASE);
        assert_eq!(p.rssi_gate(ISOLATION_RELAX_MS), RSSI_ISOLATED_RELAXED);
        assert_eq!(
            p.admit("a", -98, ISOLATION_RELAX_MS),
            Admit::Yes,
            "desperate"
        );
    }

    /// …and tightens again the moment we have company.
    #[test]
    fn the_signal_gate_tightens_once_we_have_a_link() {
        let mut p = policy();
        p.opened("a", 10_000);
        assert_eq!(p.rssi_gate(10_000), RSSI_CONNECTED);
        assert_eq!(p.admit("b", -88, 10_000), Admit::TooWeak);
    }

    /// A cap bounds concurrency; a room full of phones still bursts without a rate floor.
    #[test]
    fn connects_are_rate_limited_globally() {
        let mut p = policy();
        p.attempting(10_000);
        assert_eq!(p.admit("a", STRONG, 10_200), Admit::RateLimited);
        assert_eq!(p.admit("a", STRONG, 10_600), Admit::Yes);
    }

    #[test]
    fn links_are_capped() {
        let mut p = policy();
        for i in 0..MAX_LINKS {
            p.opened(&format!("p{i}"), 10_000);
        }
        assert_eq!(p.admit("extra", STRONG, 20_000), Admit::AtCapacity);
    }

    #[test]
    fn a_quiet_link_is_hung_up_after_the_linger() {
        let mut p = policy();
        p.opened("a", 10_000);
        p.quiet("a", 10_000);
        assert!(p.hangups(11_000).is_empty());
        assert_eq!(p.hangups(12_000), vec!["a".to_string()]);
    }

    #[test]
    fn activity_cancels_a_pending_hangup() {
        let mut p = policy();
        p.opened("a", 10_000);
        p.quiet("a", 10_000);
        p.activity("a", 11_000);
        assert!(p.hangups(15_000).is_empty());
    }

    /// The expensive mistake was leaving the background scan flat out — it is the mode that runs
    /// all day. Now every state is duty-cycled.
    #[test]
    fn the_scan_is_duty_cycled_in_every_state() {
        let p = MeshPolicy::new();
        let on = (0..SCAN_PERIOD_MS)
            .step_by(100)
            .filter(|&t| p.should_scan(t))
            .count();
        let total = (0..SCAN_PERIOD_MS).step_by(100).count();
        assert!(
            on * 3 <= total * 2 && on * 4 >= total,
            "duty ~1/3, got {on}/{total}"
        );
    }

    /// …but never sleeps through an encounter that is actually happening.
    #[test]
    fn recent_traffic_forces_the_scan_on() {
        let mut p = MeshPolicy::new();
        let off = SCAN_ON_MS + 1_000; // a moment the bare duty cycle would be off
        assert!(!p.should_scan(off));

        p.activity("a", off);
        assert!(
            p.should_scan(off + 1),
            "forced on while something is happening"
        );

        // Once the window lapses the force expires and the ordinary cycle resumes — it does not
        // latch on, and it does not latch off either.
        let later = off + RECENT_TRAFFIC_MS + 1;
        let by_cycle = later % SCAN_PERIOD_MS < SCAN_ON_MS;
        assert_eq!(p.should_scan(later), by_cycle, "back on the duty cycle");
    }

    #[test]
    fn a_dense_graph_scans_less() {
        let mut p = MeshPolicy::new();
        for i in 0..DENSE_LINKS {
            p.opened(&format!("p{i}"), 0);
        }
        let on = (0..SCAN_PERIOD_DENSE_MS)
            .step_by(100)
            .filter(|&t| p.should_scan(t))
            .count();
        let total = (0..SCAN_PERIOD_DENSE_MS).step_by(100).count();
        assert!(
            on * 5 <= total,
            "dense duty should be ~1/6, got {on}/{total}"
        );
    }

    /// A peer that connects and then says nothing must not hold a link slot forever. The linger
    /// cannot catch this: it only starts once WE have spoken and fallen silent.
    #[test]
    fn a_silent_link_is_hung_up_even_though_the_linger_never_started() {
        let mut p = policy();
        p.opened("a", 10_000);
        assert!(p.hangups(10_000 + STALL_MS - 1).is_empty());
        assert_eq!(p.hangups(10_000 + STALL_MS), vec!["a".to_string()]);
    }

    #[test]
    fn a_busy_link_never_stalls_out() {
        let mut p = policy();
        p.opened("a", 0);
        for t in (0..STALL_MS * 3).step_by(1_000) {
            p.activity("a", t);
            assert!(p.hangups(t).is_empty(), "still talking at {t}");
        }
    }

    #[test]
    fn bookkeeping_does_not_grow_without_bound() {
        let mut p = policy();
        p.closed("a", Close::Completed, 0);
        p.closed("b", Close::TimedOut, 0);
        p.prune(WEAK_LINK_BACKOFF_MS * 8);
        assert_eq!(p.admit("a", STRONG, WEAK_LINK_BACKOFF_MS * 8), Admit::Yes);
        assert!(p.ended.is_empty() && p.failures.is_empty());
    }
}
