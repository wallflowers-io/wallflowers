//! The ws client to the blind relay — speaks the ON-DISK `pacific_wire::Frame`
//! (JSON text, `tag:String` hex, `blob:String` base64, `Sub.since:u64`,
//! `Ack{seq}`, ONE bare `Eose` per Sub). No hello/Auth frame.

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message as Ws;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

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
        let (ws, _) = connect_async(url)
            .await
            .map_err(|e| CoreError::Transport(format!("connect {url}: {e}")))?;
        Ok(Self { ws })
    }

    async fn send(&mut self, f: &Frame) -> Result<(), CoreError> {
        self.ws
            .send(Ws::Text(f.to_json()))
            .await
            .map_err(|e| CoreError::Transport(e.to_string()))
    }

    async fn next_frame(&mut self) -> Result<Frame, CoreError> {
        loop {
            match self.ws.next().await {
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
        self.send(&addr.pub_frame(blob_b64, false)).await?;
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
        self.send(&addr.pub_frame(blob_b64, true)).await?;
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
        let mut out = Vec::new();
        loop {
            match self.next_frame().await? {
                Frame::Msg { tag, seq, blob } => out.push((tag, seq, blob)),
                Frame::Eose => break, // ONE bare Eose ends the backlog
                _ => continue,
            }
        }
        Ok(out)
    }

    /// SUB to each `(tag, since)` in turn, all sent before any answer is read, and drain each
    /// to its `Eose`: one round trip for every tag, each from its own cursor. The relay
    /// answers a connection's frames in order, so the messages before the n-th `Eose` are the
    /// n-th Sub's; a live message for an earlier Sub's tag, pushed after its `Eose`, is not
    /// asked for here and is left to its tag's next drain.
    pub async fn drain_many(&mut self, subs: &[(String, u64)]) -> Result<Vec<Vec<(String, u64, String)>>, CoreError> {
        for (tag, since) in subs {
            DRAINS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.send(&Frame::Sub { tags: vec![tag.clone()], since: *since, v: 0 }).await?;
        }
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
        Ok(out)
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
}
