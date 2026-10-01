//! router — the ONE place that decides where a message goes.
//!
//! Before this, transport was not a decision anybody made: `relay_url` was a parameter on 24 FFI
//! functions and 80 references in `node`, so the app re-chose a destination on every call and the
//! core simply obeyed the string it was handed. That is why the mesh could not be reached — there
//! was no point in the system where anything could say "and also to the devices in the room",
//! because there was no point in the system that decided anything at all.
//!
//! The router owns the transport set and three policies that only make sense in one place:
//!
//!   * **Fan-out.** A sealed blob goes to every configured transport. The relay reaches people who
//!     are elsewhere; the mesh reaches people who are here and possibly offline. They are not
//!     alternatives, and picking one per call was always the wrong shape.
//!   * **Routing by capability.** An MLS commit needs an arbiter, and only the relay has one (its
//!     single commit slot per group-epoch tag). A commit therefore goes ONLY to transports that
//!     can sequence — a routing decision, which until now surfaced as a transport error from
//!     whichever mailbox happened to be asked.
//!   * **Per-source drain positions.** `seq` belongs to the transport that issued it, so each
//!     transport carries its own cursor (`inbox_cursor_v2`) and the router merges the results.
//!     Sharing one cursor across two transports silently skips or replays mail.
//!
//! FAILURE IS PER-TRANSPORT AND NEVER SILENT. A dead relay must not stop the mesh delivering, and
//! an absent mesh must not stop the relay — but a publish that reached NOTHING is an error, not a
//! shrug, because a message the user believes was sent is the worst outcome available.

use crate::mailbox::Mailbox;
use crate::CoreError;
use pacific_wire::address::Address;

/// One transport the router will use, named by URL (`wss://…`, `mesh:`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub url: String,
}

/// What this device's messages ride on. Ordered: the first sequencing-capable transport arbitrates
/// commits, and drains are merged in this order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Routes(Vec<Route>);

impl Routes {
    /// Parse a transport set from a comma-separated list, skipping blanks. Unknown schemes are NOT
    /// filtered here — `Mailbox::connect` rejects them loudly at open time, because a silently
    /// dropped transport is how "my messages went nowhere" happens.
    pub fn parse(spec: &str) -> Self {
        Self(
            spec.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| Route { url: s.to_string() })
                .collect(),
        )
    }

    pub fn to_spec(&self) -> String {
        self.0
            .iter()
            .map(|r| r.url.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn urls(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|r| r.url.as_str())
    }

    /// Add a transport if it is not already present. Order is preserved — earlier is preferred.
    pub fn add(&mut self, url: &str) {
        if !self.0.iter().any(|r| r.url == url) {
            self.0.push(Route {
                url: url.to_string(),
            });
        }
    }

    pub fn remove(&mut self, url: &str) {
        self.0.retain(|r| r.url != url);
    }

    /// Replace the relay(s) in this set, keeping every non-relay transport. Used when an object is
    /// re-homed to a different Arc: the Arc changes where the RELAY leg goes, and has nothing to
    /// say about the devices in the room, which stay in the set regardless.
    pub fn with_relay(&self, relay_url: &str) -> Self {
        let mut out: Vec<Route> = self
            .0
            .iter()
            .filter(|r| !is_relay(&r.url))
            .cloned()
            .collect();
        out.insert(
            0,
            Route {
                url: relay_url.to_string(),
            },
        );
        Self(out)
    }

    /// This device's transport set, read from `paths::routes_path`.
    ///
    /// Falls back through the configuration this replaced — the legacy `relay_url` file, then the
    /// canonical production relay — so a device set up before the router keeps working without a
    /// migration step. The MESH is always appended: it costs nothing when no one is in range, and
    /// leaving it out by default would mean the feature only ever worked for people who knew to
    /// ask for it.
    pub fn load() -> Self {
        if let Ok(spec) = std::fs::read_to_string(crate::paths::routes_path()) {
            let parsed = Self::parse(spec.trim());
            if !parsed.is_empty() {
                return parsed;
            }
        }
        let relay = std::fs::read_to_string(crate::paths::relay_url_path())
            .map(|s| s.trim().to_string())
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| crate::paths::DEFAULT_RELAY_URL.to_string());
        let mut routes = Self::parse(&relay);
        routes.add(MESH_URL);
        routes
    }

    /// Persist this transport set as the device's.
    pub fn save(&self) -> Result<(), CoreError> {
        let path = crate::paths::routes_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_spec())?;
        Ok(())
    }
}

/// The mesh's route URL — a scheme with no host, because there is no host.
pub const MESH_URL: &str = "mesh:";

/// Is this route a relay (as opposed to the mesh, or anything added later)?
fn is_relay(url: &str) -> bool {
    url.starts_with("ws://") || url.starts_with("wss://")
}

/// A live set of open mailboxes. Built for one operation (or one sync pass) and dropped after —
/// the relay session is a socket, and holding one open across an idle app is what a mesh policy
/// would call a held connection.
/// Every blob this process has handed to [`Router::publish`] or [`Router::publish_commit`],
/// taken or not: a state that has tried to speak has moved, and a sealed copy of it is behind
/// (D-34 (c); `Node::published`).
static PUBLISHED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many publishes this process has attempted.
pub fn published() -> u64 {
    PUBLISHED.load(std::sync::atomic::Ordering::Relaxed)
}

pub struct Router {
    open: Vec<Mailbox>,
    /// URLs that failed to open, kept so a total failure can say WHY rather than "no transports".
    failed: Vec<(String, String)>,
}

/// A live session holds sockets, which are not printable — report what a reader actually wants
/// to know about a router instead: which transports are up, and which are not.
impl std::fmt::Debug for Router {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Router")
            .field(
                "live",
                &self.open.iter().map(Mailbox::source).collect::<Vec<_>>(),
            )
            .field("failed", &self.failed)
            .finish()
    }
}

impl Router {
    /// Open every route, tolerating individual failures. An unreachable relay is normal (no
    /// signal); an unreachable mesh is normal (nothing in range). Only opening NOTHING is fatal,
    /// and then the error names every attempt.
    pub async fn open(routes: &Routes) -> Result<Self, CoreError> {
        if routes.is_empty() {
            return Err(CoreError::NoRelay);
        }
        let mut open = Vec::new();
        let mut failed = Vec::new();
        for url in routes.urls() {
            match Mailbox::connect(url).await {
                Ok(m) => open.push(m),
                Err(e) => failed.push((url.to_string(), e.to_string())),
            }
        }
        if open.is_empty() {
            let why = failed
                .iter()
                .map(|(u, e)| format!("{u}: {e}"))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(CoreError::Transport(format!(
                "no transport available — {why}"
            )));
        }
        Ok(Self { open, failed })
    }

    /// How many transports are actually live.
    pub fn live(&self) -> usize {
        self.open.len()
    }

    /// Transports that opened but could not be reached, for diagnostics.
    pub fn failures(&self) -> &[(String, String)] {
        &self.failed
    }

    /// PUB to every transport. Succeeds if ANY accepted it, and returns the seq from the first
    /// that did — a relay's seq where there is a relay, which keeps the outbox semantics the
    /// delta log already relies on.
    pub async fn publish(&mut self, addr: &Address, blob_b64: &str) -> Result<u64, CoreError> {
        PUBLISHED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        #[cfg(feature = "storage")]
        crate::directory::record_sent(blob_b64)?;
        let mut first = None;
        let mut errors = Vec::new();
        for m in &mut self.open {
            match m.publish(addr, blob_b64.to_string()).await {
                Ok(seq) => {
                    if first.is_none() {
                        first = Some(seq);
                    }
                }
                Err(e) => errors.push(format!("{}: {e}", m.source())),
            }
        }
        first.ok_or_else(|| {
            CoreError::Transport(format!(
                "publish reached no transport — {}",
                errors.join("; ")
            ))
        })
    }

    /// PUB a COMMIT, to sequencing-capable transports only.
    ///
    /// The mesh is deliberately skipped rather than asked and refused: two devices out of range of
    /// each other would each believe they had won the epoch, and a fork is far worse than a
    /// membership change that waits for a relay. With no arbiter present at all, this is an error —
    /// never an optimistic success, which would claim an arbitration that did not happen.
    pub async fn publish_commit(
        &mut self,
        addr: &Address,
        blob_b64: &str,
    ) -> Result<Option<u64>, CoreError> {
        PUBLISHED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        #[cfg(feature = "storage")]
        crate::directory::record_sent(blob_b64)?;
        let Some(m) = self.open.iter_mut().find(|m| m.can_sequence()) else {
            return Err(CoreError::Transport(
                "no transport can sequence an MLS commit — a membership change needs a relay"
                    .into(),
            ));
        };
        m.publish_commit(addr, blob_b64.to_string())
            .await
    }

    /// Drain `tags` from every transport, each from ITS OWN cursor, and hand back the results
    /// tagged with the source that produced them so the caller advances the right cursor.
    ///
    /// `since` is resolved per transport by the caller-supplied closure rather than taken as one
    /// number, because one number is exactly the bug: a relay seq written into the mesh's position
    /// (or vice versa) skips or replays mail on both.
    pub async fn drain<F>(
        &mut self,
        tags_hex: &[String],
        mut since_for: F,
    ) -> Result<Vec<Drained>, CoreError>
    where
        F: FnMut(&str) -> Result<u64, CoreError>,
    {
        let mut out = Vec::new();
        let mut down = Vec::new();
        for m in &mut self.open {
            let source = m.source();
            let since = since_for(source)?;
            match m.drain(tags_hex.to_vec(), since).await {
                // ONLY THE TAGS ASKED FOR (NC-35). A subscription streams live for as long
                // as its socket lives, so a session that drained other tags earlier is
                // handed their messages here too. Those stay on the relay, and their own
                // drain fetches them from their own cursor.
                Ok(msgs) => out.extend(
                    msgs.into_iter()
                        .filter(|(tag, _, _)| tags_hex.iter().any(|t| t == tag))
                        .map(|(tag, seq, blob)| Drained { source, tag, seq, blob }),
                ),
                // One transport being down must not stop the others delivering.
                Err(e) => down.push((source.to_string(), e.to_string())),
            }
        }
        // EVERY TRANSPORT DOWN IS NOT AN EMPTY MAILBOX (NC-34): a pass that reached no one
        // must not report that nothing arrived.
        if !down.is_empty() && down.len() == self.open.len() {
            let why = down.iter().map(|(u, e)| format!("{u}: {e}")).collect::<Vec<_>>().join("; ");
            self.failed.extend(down);
            return Err(CoreError::Transport(format!("drain reached no transport — {why}")));
        }
        self.failed.extend(down);
        Ok(out)
    }

    /// Every tag drained, each from its own cursor per transport (`since_for(tag, source)`), in
    /// one round trip per transport: per tag, what every transport delivered. As `drain`: one
    /// transport down does not stop the others, and every transport down is an error.
    pub async fn drain_many<F>(&mut self, tags_hex: &[String], mut since_for: F) -> Result<Vec<Vec<Drained>>, CoreError>
    where
        F: FnMut(&str, &str) -> Result<u64, CoreError>,
    {
        let mut out: Vec<Vec<Drained>> = tags_hex.iter().map(|_| Vec::new()).collect();
        if tags_hex.is_empty() {
            return Ok(out);
        }
        let mut down = Vec::new();
        for m in &mut self.open {
            let source = m.source();
            let mut subs = Vec::with_capacity(tags_hex.len());
            for t in tags_hex {
                subs.push((t.clone(), since_for(t, source)?));
            }
            match m.drain_many(&subs).await {
                Ok(per_tag) => {
                    for (i, msgs) in per_tag.into_iter().enumerate() {
                        out[i].extend(msgs.into_iter().map(|(tag, seq, blob)| Drained { source, tag, seq, blob }));
                    }
                }
                Err(e) => down.push((source.to_string(), e.to_string())),
            }
        }
        if !down.is_empty() && down.len() == self.open.len() {
            let why = down.iter().map(|(u, e)| format!("{u}: {e}")).collect::<Vec<_>>().join("; ");
            self.failed.extend(down);
            return Err(CoreError::Transport(format!("drain reached no transport — {why}")));
        }
        self.failed.extend(down);
        Ok(out)
    }

    pub async fn close(self) {
        for m in self.open {
            m.close().await;
        }
    }
}

/// One drained blob, with the transport that produced it — the source is what makes its `seq`
/// meaningful, so the two always travel together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drained {
    pub source: &'static str,
    pub tag: String,
    pub seq: u64,
    pub blob: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spec_round_trips() {
        let r = Routes::parse("wss://arc.example/v1/relay, mesh:");
        assert_eq!(r.len(), 2);
        assert_eq!(r.to_spec(), "wss://arc.example/v1/relay,mesh:");
    }

    #[test]
    fn blanks_are_skipped_and_order_is_kept() {
        let r = Routes::parse(" , wss://a , , mesh: ,");
        assert_eq!(r.urls().collect::<Vec<_>>(), vec!["wss://a", "mesh:"]);
    }

    #[test]
    fn swapping_the_relay_keeps_the_mesh() {
        let r = Routes::parse("wss://a,mesh:").with_relay("wss://b");
        assert_eq!(r.to_spec(), "wss://b,mesh:");
    }

    #[test]
    fn adding_the_same_transport_twice_is_idempotent() {
        let mut r = Routes::parse("wss://a");
        r.add("mesh:");
        r.add("mesh:");
        assert_eq!(r.to_spec(), "wss://a,mesh:");
        r.remove("mesh:");
        assert_eq!(r.to_spec(), "wss://a");
    }

    #[tokio::test]
    async fn an_empty_route_set_is_no_relay_not_a_silent_noop() {
        assert!(matches!(
            Router::open(&Routes::default()).await,
            Err(CoreError::NoRelay)
        ));
    }

    /// A dead relay must not take the mesh down with it.
    #[tokio::test]
    async fn one_dead_transport_does_not_stop_the_others() {
        let routes = Routes::parse("ws://127.0.0.1:1,mesh:");
        let r = Router::open(&routes).await.expect("mesh should still open");
        assert_eq!(r.live(), 1);
        assert_eq!(
            r.failures().len(),
            1,
            "and the dead one is reported, not hidden"
        );
    }

    #[tokio::test]
    async fn every_transport_down_is_an_error_naming_each_one() {
        let routes = Routes::parse("ws://127.0.0.1:1,ws://127.0.0.1:2");
        let err = Router::open(&routes).await.unwrap_err().to_string();
        assert!(
            err.contains("127.0.0.1:1") && err.contains("127.0.0.1:2"),
            "{err}"
        );
    }

    /// The mesh alone cannot arbitrate a commit — and must say so rather than inventing a winner.
    #[tokio::test]
    async fn a_commit_with_no_sequencer_is_refused() {
        let mut r = Router::open(&Routes::parse("mesh:")).await.unwrap();
        let err = r
            .publish_commit(&Address::from_seed(&[0xaa; 32]), "bb")
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("sequence"), "{err}");
    }

    #[tokio::test]
    async fn a_publish_that_reaches_the_mesh_succeeds_without_a_relay() {
        let mut r = Router::open(&Routes::parse("mesh:")).await.unwrap();
        let addr = Address::from_seed(&[3u8; 32]);
        let tag = addr.tag_hex();
        let blob = pacific_wire::blob_b64(b"sealed");
        assert!(r.publish(&addr, &blob).await.is_ok());

        let got = r.drain(&[tag.clone()], |_| Ok(0)).await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].source, "mesh");
        assert_eq!(got[0].blob, blob);
    }
}
