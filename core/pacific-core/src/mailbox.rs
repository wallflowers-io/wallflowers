//! mailbox — the transport seam. What the node publishes into and drains from, whatever is
//! actually carrying the bytes.
//!
//! Until now `node` constructed a `RelaySession` directly in a dozen places, which quietly made
//! "a relay over a WebSocket" the only thing messaging could ever be. It never had to be: the
//! relay's whole design is a blind mailbox addressed by opaque rotating tag, and NOTHING about
//! that requires a server. A phone in range can serve it (`mesh`), and an Arc-to-Arc LoRa link
//! can serve it later. This enum is where those become peers of each other rather than a rewrite.
//!
//! An enum, not a trait object: the operations are async, `dyn` would need `async_trait`, and the
//! set of transports is small and closed enough that dispatch costs nothing and the compiler can
//! still see through it. Callers hold `&mut Mailbox` exactly where they held `&mut RelaySession`.

use crate::mesh::MeshStore;
use crate::transport::RelaySession;
use crate::CoreError;
use pacific_wire::address::Address;

/// Where this device's sealed blobs go, and where it looks for other people's.
pub enum Mailbox {
    /// A blind relay over ws/wss — infrastructure, reachable from anywhere, needs the internet.
    /// Boxed: a live session is ~1.4 KB of socket state, and the router keeps a `Vec<Mailbox>` in
    /// which every element would otherwise be that size — including the mesh ones, which are
    /// nothing at all.
    Relay(Box<RelaySession>),
    /// The devices around this one — no internet, no server, no operator. A HANDLE to the
    /// device's one shared store (`mesh::shared`), not a store of its own: what the mesh is
    /// carrying has to outlive any single sync, or a phone would arrive at every encounter
    /// with nothing to offer and nothing to hand on.
    Mesh,
}

impl Mailbox {
    /// Open a mailbox for `url`. The scheme picks the transport: `ws://` / `wss://` dial a relay,
    /// `mesh:` opens the local mesh. Anything else is a configuration error rather than a silent
    /// fallback — messaging that quietly goes somewhere other than where you pointed it is the
    /// failure mode this codebase has been bitten by most.
    pub async fn connect(url: &str) -> Result<Self, CoreError> {
        match url.split_once(':').map(|(s, _)| s) {
            Some("ws") | Some("wss") => {
                Ok(Self::Relay(Box::new(RelaySession::connect(url).await?)))
            }
            Some("mesh") => Ok(Self::Mesh),
            _ => Err(CoreError::Transport(format!(
                "unsupported relay scheme in {url:?} — expected ws://, wss:// or mesh:"
            ))),
        }
    }

    /// PUB one sealed blob. On the mesh this means "hand it to the devices around you and keep
    /// carrying it" — there is no acknowledging server, so the returned seq is 0 and carries no
    /// meaning beyond "accepted locally".
    pub async fn publish(&mut self, addr: &Address, blob_b64: String) -> Result<u64, CoreError> {
        match self {
            Self::Relay(s) => s.publish(addr, blob_b64).await,
            Self::Mesh => {
                let (tag, blob) = decode(&addr.tag_hex(), &blob_b64)?;
                store()?.carry(tag, blob, 0, now_secs());
                Ok(0)
            }
        }
    }

    /// PUB a sealed COMMIT into the tag's single slot. `Ok(None)` means the slot was already
    /// taken and the caller must rebase onto the winner.
    ///
    /// THE MESH HAS NO SEQUENCER. The relay's commit slot is a blind first-writer-wins arbiter
    /// (`commit_taken` in the relay), and that is precisely the thing a partitioned mesh cannot
    /// provide: two devices out of range of each other can each believe they won the same epoch.
    /// Answering `Ok(Some(0))` here would claim an arbitration that did not happen, so until the
    /// merge rule is designed and tested, a commit over the mesh alone is refused OUT LOUD
    /// rather than silently forking two members' MLS state.
    pub async fn publish_commit(
        &mut self,
        addr: &Address,
        blob_b64: String,
    ) -> Result<Option<u64>, CoreError> {
        match self {
            Self::Relay(s) => s.publish_commit(addr, blob_b64).await,
            Self::Mesh => Err(CoreError::Transport(
                "the mesh cannot sequence an MLS commit — membership changes still need a relay"
                    .into(),
            )),
        }
    }

    /// Drain `tags_hex` from `since`.
    ///
    /// `since` is a RELAY sequence number, assigned by whichever relay accepted the publish, and
    /// it is meaningless on a device that never saw that relay — so the mesh ignores it and
    /// tracks what it has handed over by content digest instead. It reports seq 0 for everything,
    /// which is safe because `Directory::advance_cursor` never rewinds: a mesh delivery therefore
    /// cannot disturb the relay cursor for the same tag, and the two transports can drain the
    /// same mailbox without corrupting each other's position.
    pub async fn drain(
        &mut self,
        tags_hex: Vec<String>,
        since: u64,
    ) -> Result<Vec<(String, u64, String)>, CoreError> {
        match self {
            Self::Relay(s) => s.drain(tags_hex, since).await,
            Self::Mesh => {
                let mut tags = Vec::with_capacity(tags_hex.len());
                for t in &tags_hex {
                    tags.push(decode_tag(t)?);
                }
                // Only COMPLETE messages come back — a half-carried Delta is progress the mesh
                // keeps, not something the core should ever be handed.
                Ok(store()?
                    .undelivered(&tags)
                    .into_iter()
                    .map(|(tag, blob)| {
                        (
                            pacific_wire::tag_hex(&tag),
                            0,
                            pacific_wire::blob_b64(&blob),
                        )
                    })
                    .collect())
            }
        }
    }

    /// Each tag drained from its own `since`, in one round trip where the transport can
    /// (`RelaySession::drain_many`); the mesh answers each from its store.
    pub async fn drain_many(&mut self, subs: &[(String, u64)]) -> Result<Vec<Vec<(String, u64, String)>>, CoreError> {
        match self {
            Self::Relay(s) => s.drain_many(subs).await,
            Self::Mesh => {
                let mut out = Vec::with_capacity(subs.len());
                for (tag, since) in subs {
                    out.push(Box::pin(self.drain(vec![tag.clone()], *since)).await?);
                }
                Ok(out)
            }
        }
    }

    /// Which cursor space this mailbox's `seq` values live in. A drain position is meaningless
    /// outside it, so the Directory keys `inbox_cursor_v2` on exactly this.
    pub fn source(&self) -> &'static str {
        match self {
            Self::Relay(_) => "relay",
            Self::Mesh => "mesh",
        }
    }

    /// Can this transport ARBITRATE an MLS commit? The relay can: it holds a single commit slot
    /// per group-epoch tag and awards it first-writer-wins. The mesh cannot — two devices out of
    /// range of each other would each believe they had won. The router uses this to route a
    /// commit rather than discovering the problem as a transport error.
    pub fn can_sequence(&self) -> bool {
        matches!(self, Self::Relay(_))
    }

    /// Close cleanly. The mesh has nothing to hang up — its store outlives any one encounter,
    /// which is the point of carrying.
    pub async fn close(self) {
        match self {
            Self::Relay(s) => s.close().await,
            Self::Mesh => {}
        }
    }
}

/// Lock the device's shared mesh store. A poisoned lock means another thread panicked mid-carry;
/// surfaced as a transport error rather than propagating the panic into the UI.
fn store() -> Result<std::sync::MutexGuard<'static, MeshStore>, CoreError> {
    crate::mesh::shared()
        .lock()
        .map_err(|_| CoreError::Transport("mesh store lock poisoned".into()))
}

/// Seconds since the epoch — the mesh's TTL clock. Carrying is time-bounded, not ordered, so
/// this is only ever compared against itself and never used to sequence anything.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn decode(tag_hex: &str, blob_b64: &str) -> Result<([u8; 32], Vec<u8>), CoreError> {
    let tag = decode_tag(tag_hex)?;
    let blob = pacific_wire::blob_unb64(blob_b64)
        .map_err(|e| CoreError::Transport(format!("mesh: bad blob: {e}")))?;
    Ok((tag, blob))
}

fn decode_tag(tag_hex: &str) -> Result<[u8; 32], CoreError> {
    pacific_wire::tag_unhex(tag_hex)
        .map_err(|e| CoreError::Transport(format!("mesh: bad tag: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pacific_wire::blob_b64;

    #[tokio::test]
    async fn an_unknown_scheme_is_refused_rather_than_guessed() {
        assert!(Mailbox::connect("http://example.test").await.is_err());
        assert!(Mailbox::connect("example.test:8787").await.is_err());
    }

    #[tokio::test]
    async fn the_mesh_mailbox_round_trips_a_blob() {
        let mut m = Mailbox::connect("mesh:").await.unwrap();
        let addr = Address::from_seed(&[9u8; 32]);
        let tag = addr.tag_hex();
        let blob = blob_b64(b"sealed-bytes");
        m.publish(&addr, blob.clone()).await.unwrap();

        let got = m.drain(vec![tag.clone()], 0).await.unwrap();
        assert_eq!(got, vec![(tag.clone(), 0, blob)]);
        assert!(
            m.drain(vec![tag], 0).await.unwrap().is_empty(),
            "a drained blob is not replayed on the next sync"
        );
    }

    /// A commit MUST NOT appear to succeed on a transport with no arbiter — a silent fork is
    /// far worse than a refused membership change.
    #[tokio::test]
    async fn the_mesh_refuses_to_sequence_a_commit() {
        let mut m = Mailbox::connect("mesh:").await.unwrap();
        let err = m
            .publish_commit(&Address::from_seed(&[1u8; 32]), blob_b64(b"commit"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("cannot sequence"));
    }
}
