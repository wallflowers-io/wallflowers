//! The ws client to the blind relay — speaks the ON-DISK `pacific_wire::Frame`
//! (JSON text, `tag:String` hex, `blob:String` base64, `Sub.since:u64`,
//! `Ack{seq}`, ONE bare `Eose` per Sub). No hello/Auth frame.

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message as Ws;
use tokio_tungstenite::{connect_async_with_config, MaybeTlsStream, WebSocketStream};

use pacific_wire::address::Address;
use pacific_wire::Frame;

use crate::CoreError;

/// Every relay connection this process has dialled, and every drain `Sub` it has sent: a
/// round trip each (FC-11).
static DIALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static DRAINS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many relay connections this process has dialled.
pub fn dials() -> u64 {
    DIALS.load(std::sync::atomic::Ordering::Relaxed)
}

/// How many drain `Sub`s this process has sent.
pub fn drains() -> u64 {
    DRAINS.load(std::sync::atomic::Ordering::Relaxed)
}

/// A live ws session to the relay. One connection; the caller drives publish/drain.
pub struct RelaySession {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    /// How long an exchange waits for its next frame before the relay counts as silent.
    /// None: as long as TCP holds (a session for one operation). The link's sessions set
    /// it, since a session they keep must not hang every operation behind a dead peer.
    idle: Option<std::time::Duration>,
}

/// rustls 0.23 panics on the first `wss://` handshake unless a process-level
/// CryptoProvider is installed, and it won't auto-pick one when both aws-lc-rs and
/// ring are compiled in (our graph has both). Install ring exactly once, before any
/// dial. Idempotent, and a no-op cost on plaintext `ws://`. Surfaced by running on a
/// real device against a `wss://` relay — the local harness used plaintext `ws://`.
fn ensure_crypto_provider() {
    use std::sync::Once;
    static CRYPTO: Once = Once::new();
    CRYPTO.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

impl RelaySession {
    /// Connect. No hello frame — the on-disk relay has none.
    pub async fn connect(url: &str) -> Result<Self, CoreError> {
        ensure_crypto_provider();
        DIALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (ws, _) = connect_async_with_config(url, None, true)  // true: TCP_NODELAY
            .await
            .map_err(|e| CoreError::Transport(format!("connect {url}: {e}")))?;
        Ok(Self { ws, idle: None })
    }

    /// The longest an exchange waits for its next frame (`RelaySession::idle`).
    pub(crate) fn with_idle(mut self, idle: std::time::Duration) -> Self {
        self.idle = Some(idle);
        self
    }

    /// Declare vocabulary `v` (`pacific_wire::V_*`) once, with an empty `Sub`, so no later
    /// request on this session is gated. The declared level only rises (the relay keeps the
    /// highest a connection has declared).
    pub(crate) async fn declare(&mut self, v: u8) -> Result<(), CoreError> {
        self.send(&Frame::Sub { tags: vec![], since: 0, v }).await?;
        while self.next_frame().await? != Frame::Eose {}
        Ok(())
    }

    /// A websocket ping: the keepalive of a session held between operations.
    pub(crate) async fn ping(&mut self) -> Result<(), CoreError> {
        self.ws
            .send(Ws::Ping(Vec::new().into()))
            .await
            .map_err(|e| CoreError::Transport(e.to_string()))
    }

    /// Read whatever arrives on a session between operations (a pong, a frame nobody asked
    /// for): Ok when anything was heard, Err when the socket has ended.
    pub(crate) async fn idle_read(&mut self) -> Result<(), CoreError> {
        match self.ws.next().await {
            Some(Ok(Ws::Close(_))) | None => Err(CoreError::Transport("relay closed".into())),
            Some(Ok(_)) => Ok(()),
            Some(Err(e)) => Err(CoreError::Transport(e.to_string())),
        }
    }

    async fn send(&mut self, f: &Frame) -> Result<(), CoreError> {
        self.ws
            .send(Ws::Text(f.to_json()))
            .await
            .map_err(|e| CoreError::Transport(e.to_string()))
    }

    async fn next_frame(&mut self) -> Result<Frame, CoreError> {
        loop {
            let next = match self.idle {
                None => self.ws.next().await,
                Some(d) => tokio::time::timeout(d, self.ws.next())
                    .await
                    .map_err(|_| CoreError::Transport(format!("the relay was silent for {} ms", d.as_millis())))?,
            };
            match next {
                Some(Ok(Ws::Text(t))) => {
                    return Frame::from_json(&t)
                        .map_err(|e| CoreError::Transport(format!("bad frame: {e}")))
                }
                Some(Ok(Ws::Binary(b))) => {
                    let s = String::from_utf8_lossy(&b);
                    return Frame::from_json(&s)
                        .map_err(|e| CoreError::Transport(format!("bad frame: {e}")));
                }
                Some(Ok(Ws::Close(_))) | None => {
                    return Err(CoreError::Transport("relay closed".into()))
                }
                Some(Ok(_)) => continue, // ping/pong
                Some(Err(e)) => return Err(CoreError::Transport(e.to_string())),
            }
        }
    }

    /// PUB one sealed blob (hex tag, base64 blob); wait for `Ack`. A plain
    /// publish is never slot-gated, so a rejecting ack here is a protocol
    /// violation — surfaced loudly, never swallowed.
    pub async fn publish(&mut self, addr: &Address, blob_b64: String) -> Result<u64, CoreError> {
        self.publish_frame(&addr.pub_frame(blob_b64, false)).await
    }

    /// [`RelaySession::publish`] for a `Pub` already built and signed by its caller.
    pub(crate) async fn publish_frame(&mut self, frame: &Frame) -> Result<u64, CoreError> {
        self.send(frame).await?;
        loop {
            match self.next_frame().await? {
                Frame::Ack { seq, ok: true, .. } => return Ok(seq),
                Frame::Ack { ok: false, reason, .. } => {
                    // A plain publish has no slot to lose, so every refusal carries a
                    // reason: a bad signature, a size or rate limit, a full store.
                    return Err(CoreError::Transport(format!(
                        "the relay refused a publish: {}",
                        reason.as_deref().unwrap_or("no reason given (protocol violation)")
                    )));
                }
                // ignore any interleaved live Msg while waiting for our ack
                _ => continue,
            }
        }
    }

    /// PUB one sealed COMMIT blob into the tag's single commit slot (the blind
    /// sequencer: one commit per group-epoch tag, first-writer-wins). Returns
    /// `Ok(Some(seq))` if this commit WON the slot, `Ok(None)` if the slot was
    /// already taken — the caller must discard its pending commit and rebase
    /// onto the winner it will find by draining the tag.
    pub async fn publish_commit(
        &mut self,
        addr: &Address,
        blob_b64: String,
    ) -> Result<Option<u64>, CoreError> {
        self.publish_commit_frame(&addr.pub_frame(blob_b64, true)).await
    }

    /// [`RelaySession::publish_commit`] for a commit `Pub` already built and signed.
    pub(crate) async fn publish_commit_frame(&mut self, frame: &Frame) -> Result<Option<u64>, CoreError> {
        self.send(frame).await?;
        loop {
            match self.next_frame().await? {
                Frame::Ack { seq, ok: true, .. } => return Ok(Some(seq)),
                // No reason is the one refusal that is not an error: the slot was
                // taken, and the caller rebases onto the winner.
                Frame::Ack { ok: false, reason: None, .. } => return Ok(None),
                Frame::Ack { ok: false, reason: Some(r), .. } => {
                    return Err(CoreError::Transport(format!("the relay refused a commit: {r}")))
                }
                _ => continue,
            }
        }
    }

    /// SUB to `tags_hex` from `since`, drain stored messages up to the single
    /// bare `Eose`. Returns (tag_hex, seq, blob_b64) per delivered message.
    pub async fn drain(
        &mut self,
        tags_hex: Vec<String>,
        since: u64,
    ) -> Result<Vec<(String, u64, String)>, CoreError> {
        if tags_hex.is_empty() {
            return Ok(vec![]);
        }
        DRAINS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.send(&Frame::Sub {
            tags: tags_hex,
            since,
            // DELIBERATELY 0. The vocabulary for `Gap` exists (so the frame parses
            // rather than killing the drain), but opting in means deciding what a
            // hole in someone's history LOOKS like — a banner, a break in the
            // conversation, a silent re-sync — and that ruling has not been made
            // (SCALE-UNBLOCKS O-1). Until it is, the relay stays quiet and the
            // operator sees the same fact in its `gaps_announced` counter.
            // Flipping this to 1 is safe against any relay: older ones ignore
            // unknown fields, so the only effect is that Gap frames start arriving.
            v: 0,
        })
        .await?;
        self.send_clear().await?;
        let mut out = Vec::new();
        loop {
            match self.next_frame().await? {
                Frame::Msg { tag, seq, blob } => out.push((tag, seq, blob)),
                Frame::Eose => break, // ONE bare Eose ends the backlog
                _ => continue,
            }
        }
        self.read_clear().await?;
        Ok(out)
    }

    /// AFTER A DRAIN, NO TAG STAYS LIVE. A `Sub` replaces the connection's subscriptions
    /// (relay lib.rs, `subscribe`), so the drained tags would go on being pushed to a session
    /// kept for later operations, and a later drain of the same tag would meet its live
    /// copy beside its replay. An empty `Sub`, pipelined behind the drain's, ends them; its
    /// `Eose` is the last frame of the exchange.
    async fn send_clear(&mut self) -> Result<(), CoreError> {
        self.send(&Frame::Sub { tags: vec![], since: 0, v: 0 }).await
    }

    /// The clearing `Sub`'s `Eose`, and nothing before it: what arrived between the drain's
    /// `Eose` and it was pushed live, not asked for.
    async fn read_clear(&mut self) -> Result<(), CoreError> {
        while self.next_frame().await? != Frame::Eose {}
        Ok(())
    }

    /// SUB to each `(tag, since)` in turn, all sent before any answer is read, and drain each
    /// to its `Eose`: one round trip for every tag, each from its own cursor. The relay
    /// answers a connection's frames in order, so the messages before the n-th `Eose` are the
    /// n-th Sub's; a live message for an earlier Sub's tag, pushed after its `Eose`, is not
    /// asked for here and is left to its tag's next drain.
    pub async fn drain_many(&mut self, subs: &[(String, u64)]) -> Result<Vec<Vec<(String, u64, String)>>, CoreError> {
        if subs.is_empty() {
            return Ok(vec![]);
        }
        for (tag, since) in subs {
            DRAINS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.send(&Frame::Sub { tags: vec![tag.clone()], since: *since, v: 0 }).await?;
        }
        self.send_clear().await?;
        let mut out = Vec::with_capacity(subs.len());
        for (tag, _) in subs {
            let mut got = Vec::new();
            loop {
                match self.next_frame().await? {
                    Frame::Msg { tag: t, seq, blob } if t == *tag => got.push((t, seq, blob)),
                    Frame::Eose => break,
                    _ => continue,
                }
            }
            out.push(got);
        }
        self.read_clear().await?;
        Ok(out)
    }

    /// A presigned URL for one object (O-79): an upload of exactly `put_len` bytes, or with
    /// `None` a download. Declares [`pacific_wire::V_MEDIA`] first with an EMPTY `Sub`, which
    /// ends this session's subscriptions: use a session of its own (`media_store` does).
    pub async fn media_url(&mut self, key: &str, put_len: Option<u64>) -> Result<String, CoreError> {
        self.send(&Frame::Sub { tags: vec![], since: 0, v: pacific_wire::V_MEDIA }).await?;
        while self.next_frame().await? != Frame::Eose {}
        self.send(&match put_len {
            Some(len) => Frame::MediaPut { key: key.to_string(), len },
            None => Frame::MediaGet { key: key.to_string() },
        })
        .await?;
        loop {
            match self.next_frame().await? {
                Frame::MediaUrl { key: k, url, .. } if k == key => return Ok(url),
                Frame::MediaErr { key: k, reason } if k == key => {
                    return Err(CoreError::Transport(format!("the relay refused to presign: {reason}")))
                }
                _ => continue,
            }
        }
    }

    /// PUB one knock to `door` as a door's tag ([`Address::door_tag`]), held by the relay to
    /// `mls.door.relay`'s bound; wait for `Ack`. A refusal carries its reason: `too_large`
    /// past the knock cap, `rate_limited` past this connection's knocks an hour.
    pub async fn publish_door(&mut self, door: &Address, blob_b64: String) -> Result<u64, CoreError> {
        self.publish_door_frame(&door.door_pub_frame(blob_b64)).await
    }

    /// [`RelaySession::publish_door`] for a knock already built and signed.
    pub(crate) async fn publish_door_frame(&mut self, frame: &Frame) -> Result<u64, CoreError> {
        self.send(frame).await?;
        loop {
            match self.next_frame().await? {
                Frame::Ack { seq, ok: true, .. } => return Ok(seq),
                Frame::Ack { ok: false, reason, .. } => {
                    return Err(CoreError::Transport(format!(
                        "the relay refused a knock: {}",
                        reason.as_deref().unwrap_or("no reason given (protocol violation)")
                    )));
                }
                _ => continue,
            }
        }
    }

    /// [`RelaySession::drain_many`] for door tags ([`Address::door_tag`]), declaring
    /// [`pacific_wire::V_DOOR`] so the relay also says how many knocks each door has lost
    /// to its cap, ever (`Evicted`): the second value, per tag, absent when none.
    pub async fn drain_doors(
        &mut self,
        subs: &[(String, u64)],
    ) -> Result<(Vec<Vec<(String, u64, String)>>, std::collections::BTreeMap<String, u64>), CoreError> {
        if subs.is_empty() {
            return Ok((vec![], Default::default()));
        }
        for (tag, since) in subs {
            DRAINS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.send(&Frame::Sub { tags: vec![tag.clone()], since: *since, v: pacific_wire::V_DOOR }).await?;
        }
        self.send_clear().await?;
        let mut out = Vec::with_capacity(subs.len());
        let mut evicted = std::collections::BTreeMap::new();
        for (tag, _) in subs {
            let mut got = Vec::new();
            loop {
                match self.next_frame().await? {
                    Frame::Msg { tag: t, seq, blob } if t == *tag => got.push((t, seq, blob)),
                    Frame::Evicted { tag: t, total } if t == *tag => {
                        evicted.insert(t, total);
                    }
                    Frame::Eose => break,
                    _ => continue,
                }
            }
            out.push(got);
        }
        self.read_clear().await?;
        Ok((out, evicted))
    }

    /// The socket, for the one reader that holds it (`live::Live`): dialled by `connect`,
    /// so every connection to a relay still starts in one place.
    pub(crate) fn into_socket(self) -> WebSocketStream<MaybeTlsStream<TcpStream>> {
        self.ws
    }

    /// Gracefully close the ws connection (clean handshake; avoids the relay
    /// logging a "Connection reset without closing handshake").
    pub async fn close(mut self) {
        let _ = self.ws.close(None).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pacific_wire::blob_b64;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn publish_then_drain_against_in_process_relay() {
        // spawn the real on-disk relay on an ephemeral port
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { relay::serve(listener).await });

        let url = format!("ws://{addr}");
        let at = Address::from_seed(&[0x42u8; 32]);
        let tag = at.tag_hex();
        let payload = blob_b64(b"sealed-bytes");

        let mut pubber = RelaySession::connect(&url).await.unwrap();
        let seq = pubber.publish(&at, payload.clone()).await.unwrap();
        assert!(seq >= 1);

        // a fresh subscriber drains the backlog and sees exactly one message + Eose
        let mut sub = RelaySession::connect(&url).await.unwrap();
        let msgs = sub.drain(vec![tag.clone()], 0).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].0, tag);
        assert_eq!(msgs[0].2, payload);
    }

    /// A drain is answered with several small frames back to back (each message, then Eose).
    /// With Nagle's algorithm on, Linux holds the second until the first is ACKed, and the
    /// ACK is delayed ~40 ms: core's MLS suites ran ~6x slower on Linux than on macOS.
    #[tokio::test]
    async fn thirty_drains_are_answered_within_a_second() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { relay::serve(listener).await });

        let at = Address::from_seed(&[0x43u8; 32]);
        let url = format!("ws://{addr}");
        let mut s = RelaySession::connect(&url).await.unwrap();
        for i in 0..3u8 {
            s.publish(&at, blob_b64(&[i])).await.unwrap();
        }
        let started = std::time::Instant::now();
        for _ in 0..30 {
            assert_eq!(s.drain(vec![at.tag_hex()], 0).await.unwrap().len(), 3);
        }
        let took = started.elapsed();
        assert!(took < std::time::Duration::from_secs(1), "30 drains took {took:?}");
    }

    /// A session kept after a drain is not left subscribed: a publish to the drained tag
    /// from someone else pushes nothing onto it (#144).
    #[tokio::test]
    async fn after_a_drain_no_tag_stays_live_on_the_session() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { relay::serve(listener).await });
        let url = format!("ws://{addr}");
        let at = Address::from_seed(&[0x44u8; 32]);

        let mut kept = RelaySession::connect(&url).await.unwrap();
        assert!(kept.drain(vec![at.tag_hex()], 0).await.unwrap().is_empty());
        assert!(kept.drain_many(&[(at.tag_hex(), 0)]).await.unwrap()[0].is_empty());
        let mut other = RelaySession::connect(&url).await.unwrap();
        other.publish(&at, blob_b64(b"later")).await.unwrap();
        let pushed = tokio::time::timeout(std::time::Duration::from_millis(300), kept.next_frame()).await;
        assert!(pushed.is_err(), "the drained tag was still live: {pushed:?}");
    }

    #[tokio::test]
    async fn the_relay_socket_does_not_wait_to_coalesce() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { relay::serve(listener).await });

        let ws = RelaySession::connect(&format!("ws://{addr}")).await.unwrap().into_socket();
        let MaybeTlsStream::Plain(tcp) = ws.get_ref() else { panic!("a ws:// relay is plain TCP") };
        assert!(tcp.nodelay().unwrap(), "TCP_NODELAY is off");
    }
}
