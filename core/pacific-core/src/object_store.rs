//! object_store — THE GroupObject interface over `delta_log`.
//!
//! One constructor, one place that knows how a log becomes an object, and the set
//! operations every consumer kept re-deriving. Before this there were EIGHT
//! near-identical `folded_*` functions in `node.rs`, each repeating the same five
//! steps and each an independent chance to omit the type-id filter — the omission
//! that "poisoned every membership tether" (prekeys on a member-tether's log made
//! the Group fold throw, which broke both `setMemberRole` authoring and
//! `verify_member`'s role read). `ObjectType::KIND` already knows the type id, so
//! deriving the filter here makes that class of bug unrepresentable.
//!
//! THE STORAGE CONTRACT this exists to serve:
//!
//!   delta_log   FULL HISTORY. The canonical CBOR envelopes; the only truth. An
//!               object IS its rows here. Append-only, signed, syncs between
//!               devices.
//!   fold        a pure function of that history (plus the MLS roster + owner).
//!               Deterministic: two devices holding the same log fold the same
//!               state.
//!   topology    ONE node per GroupObject, keyed by its object id, carrying the
//!               CURRENT folded state. A cache of the fold, never a second truth
//!               — deletable and rebuildable from the log at any time.
//!
//! `fold_digest` is what makes that last line checkable: a hash of the canonical
//! folded state. The graph re-embeds exactly when the digest moves, so "is this
//! node stale?" is one comparison rather than a field-by-field diff that silently
//! misses whatever the projection forgot to carry.

use std::collections::BTreeMap;

use crate::coordinator::{Coordinator};
use crate::directory::Directory;
use crate::object::{MemberId, ObjectKind, ObjectType};
use crate::CoreError;

/// A GroupObject's identity + cheap metadata, without folding it. The row type of
/// the listing operations — deliberately fold-free so listing a hundred objects
/// costs a hundred index lookups, not a hundred folds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectRef {
    /// The object id (the MLS group id).
    pub id: Vec<u8>,
    /// The directory kind string ("group" | "place" | "event" | "member-tether" | …).
    pub kind: String,
    /// How many deltas the log carries — activity volume, not recency.
    pub deltas: usize,
    /// When the most recent delta ARRIVED here (unix ms, 0 when empty). Local
    /// receipt time, not authored time: it answers "what moved lately on this
    /// device", which is what a recency sort is actually asking.
    pub last_at: i64,
}

/// The folded state of one object plus the digest that says whether a cache of it
/// is stale. `state` is `T::State`; the digest covers the CANONICAL log, so it is
/// stable across devices that hold the same deltas.
pub struct Folded<T: ObjectType> {
    pub state: T::State,
    pub members: Vec<MemberId>,
    pub digest: [u8; 32],
}

/// One row of the hops-aware listing: an object I HOLD (`holder: None, hops: 0`) or
/// one a peer ANSWERED a discovery request with (`holder: Some(who), hops: 1|2`).
/// A remote row is a REFERENCE — its `id` is meaningful in the holder's store and
/// serves as provenance in ours; publishing to one first mints a local counterpart
/// carrying that id in `GeoPoint::external_id`.
#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveredRow {
    /// Object id, hex — local for a held row, the holder's for a remote one.
    pub id: String,
    pub kind: String,
    pub name: String,
    pub descriptor: String,
    pub point: Option<crate::geo::GeoPoint>,
    /// `None` = held locally.
    pub holder: Option<MemberId>,
    /// Handshakes between me and whoever holds it. 0 = mine.
    pub hops: u32,
}

/// The GroupObject interface over the delta log.
///
/// Borrows the `Directory` rather than owning it: the directory is the one SQLite
/// handle, and a store that opened its own would be a second connection to the
/// same file — the dual-source mistake in miniature.
pub struct GroupObjectStore<'a> {
    dir: &'a Directory,
}

impl<'a> GroupObjectStore<'a> {
    pub fn new(dir: &'a Directory) -> Self {
        Self { dir }
    }

    // ---- THE constructor -----------------------------------------------------

    /// Fold one object's log through `T`'s reducer — the ONE way a GroupObject is
    /// constructed from storage.
    ///
    /// The type-id filter is derived from `T::KIND`, never passed in. ONE log can
    /// carry MANY op-groups (a member-tether holds Group role deltas *and* Contact
    /// prekey deltas), and the Coordinator hard-rejects a foreign `type_id`, so a
    /// fold that does not filter throws on the first foreign delta.
    ///
    /// Ordering is `(is_commutative, epoch, seq)`: the sequenced spine in log order
    /// first, then commutative ops, which the Coordinator re-sorts canonically by
    /// `(gen, author, delta_id)`. State is therefore a function of the delta SET,
    /// never of arrival order.
    pub fn folded<T: ObjectType>(&self, id: &[u8]) -> Result<Coordinator<T>, CoreError> {
        // THE FOLD CACHE (O-69): a hit equals a refold of the same rows under the same
        // model, in state and in failure.
        self.dir.cached(id, T::KIND.name(), "fold", |a: &Coordinator<T>, b| a.accepted_digest() == b.accepted_digest(), || {
            self.fold_fresh::<T>(id)
        })
    }

    /// The fold itself, uncached.
    fn fold_fresh<T: ObjectType>(&self, id: &[u8]) -> Result<Coordinator<T>, CoreError> {
        // ONE fold path: `fold::fold_entries` is the
        // implementation, so a browser folding an exported archive and a phone
        // folding its own directory run byte-for-byte the same code.
        // The owner AT EACH EPOCH (§10.2): the directory's history, which answers with
        // `owner_pk` from epoch 0 for an object that has never changed hands.
        let mut owners = self.dir.owner_history(id)?;
        if owners.is_empty() {
            owners.push((0, self.owner(id)?));
        }
        let members = self.dir.group_members(id)?;
        let log = self.dir.load_log(id)?;
        crate::fold::fold_entries_owned::<T>(members, owners, log)
    }

    /// [`folded`] plus the roster and the fold digest — what a projection needs in
    /// one call. The digest is over the object's CANONICAL delta ids for this lens,
    /// so it moves if and only if the folded state could have moved, and two devices
    /// with the same log compute the same value.
    pub fn fold<T: ObjectType>(&self, id: &[u8]) -> Result<Folded<T>, CoreError> {
        let members = self.dir.group_members(id)?;
        let coord = self.folded::<T>(id)?;
        Ok(Folded {
            state: coord.state(),
            members,
            digest: self.fold_digest(id, T::KIND)?,
        })
    }

    /// A hash of every delta id on this object FOR THIS LENS, in canonical order.
    ///
    /// Cache key for the embedding: re-embed when it changes, skip when it does not.
    /// Delta ids are content addresses (sha256 of the canonical envelope), so this
    /// is a hash over content, and the roster is folded in because membership is
    /// part of the object's meaning even though it lives in the ratchet tree.
    pub fn fold_digest(&self, id: &[u8], kind: ObjectKind) -> Result<[u8; 32], CoreError> {
        let log = self.dir.load_log(id)?;
        crate::fold::digest_of(
            kind,
            log.iter().map(|(_, e)| e.as_slice()),
            &self.dir.group_members(id)?,
        )
    }

    // ---- set operations ------------------------------------------------------

    /// Every object of one directory kind. The listing the LIFE tab and the graph
    /// projection both want, and neither should be re-deriving from `all_groups`.
    pub fn by_kind(&self, kind: &str) -> Result<Vec<ObjectRef>, CoreError> {
        self.refs(|k| k == kind)
    }

    /// Every object whose kind is any of `kinds` — one pass for the projection,
    /// which wants places+events+groups+projects+things together.
    pub fn by_kinds(&self, kinds: &[&str]) -> Result<Vec<ObjectRef>, CoreError> {
        self.refs(|k| kinds.contains(&k))
    }

    /// THE n-2 GOSSIP CACHE — [`by_kind`], widened to the social graph. ONE read
    /// for every category the Discover and Trade faces show: everything of `kind`
    /// within `hops` handshakes of `me`, from the folds alone. 0 = what I hold;
    /// 1–2 add what the graph has told me — no TTL, so every shelf renders between
    /// sessions with nothing extra persisted (the log IS the cache), and a
    /// holder's newer statement replaces their older one wholesale.
    ///
    ///   "place"  DISCOVER: held Places + the discovery answers
    ///            (`cacheable_discover_answers` filtered per channel, newest per
    ///            holder selected HERE — one channel cannot see another), topped
    ///            with the live TTL window (where tag-filtered answers surface).
    ///   "thing"  TRADE: held Things + the market's merged listings — push gossip
    ///            that needed no question to arrive and already outlives its
    ///            moment; withdrawn/lapsed rows drop at read.
    ///
    /// A pure read that asks nobody anything — refreshing is `Node::discover` /
    /// the market's own sync reconcile, warmed together by
    /// `Node::warm_gossip_caches` on app load.
    ///
    /// MINTED WINS (places): a remote row whose id is already some local object's
    /// `external_id` provenance (the user adopted it) is dropped in favour of the
    /// local one, so publishing to a discovered place never shows it twice.
    pub fn by_kind_hops(
        &self,
        kind: &str,
        hops: u32,
        me: &MemberId,
        now_ms: i64,
    ) -> Result<Vec<DiscoveredRow>, CoreError> {
        match kind {
            "place" => self.place_rows(hops, me, now_ms),
            "thing" => self.thing_rows(hops, me, now_ms),
            // A loud error, where the responder side returns empty for the same
            // input (`Node::discover_items_for`) — DELIBERATE asymmetry: an
            // unknown kind here is a local caller's typo and must fail its test;
            // on the wire it is a question from a newer build, and "nothing to
            // say" is the forward-compatible answer.
            _ => Err(CoreError::Directory(format!(
                "by_kind_hops supports 'place' | 'thing' (asked for '{kind}')"
            ))),
        }
    }

    /// The DISCOVER face: held Places, the no-TTL answer cache, the live window.
    fn place_rows(
        &self,
        hops: u32,
        me: &MemberId,
        now_ms: i64,
    ) -> Result<Vec<DiscoveredRow>, CoreError> {
        // ONE directory sweep for both id sets — `refs()`'s deltas/recency would be
        // computed only to be thrown away here.
        let mut place_ids: Vec<Vec<u8>> = Vec::new();
        let mut connection_ids: Vec<Vec<u8>> = Vec::new();
        for (id, k) in self.dir.all_groups()? {
            match k.as_str() {
                "place" => place_ids.push(id),
                "connection" => connection_ids.push(id),
                _ => {}
            }
        }

        let mut out = Vec::new();
        let mut adopted: std::collections::BTreeSet<String> = Default::default();
        for id in &place_ids {
            let st = self.folded::<crate::place::PlaceType>(id)?.state();
            if let Some(from) = st.adopted_from() {
                adopted.insert(from.to_string());
            }
            let point = st.point().cloned();
            out.push(DiscoveredRow {
                id: hex::encode(id),
                kind: "place".to_string(),
                name: st.name,
                descriptor: st.descriptor,
                point,
                holder: None,
                hops: 0,
            });
        }
        if hops == 0 {
            return Ok(out);
        }

        // Remote rows, two legs into one map:
        //   1. THE CACHE — every holder's newest full answer, out to n-2, NO TTL.
        //      Selection is GLOBAL (across channels and rids): relays carry the
        //      holder's own clock, so "newest" means the same thing down any path.
        //   2. every LIVE answer (the TTL window) — where tag-filtered answers
        //      surface while their question lives.
        // Deduped by (holder, object) keeping the shortest path, cache first so a
        // fresh copy of the same row never loses to a relayed one.
        let mut cacheable: Vec<crate::contact::DiscoverResponse> = Vec::new();
        let mut live: Vec<crate::contact::DiscoverResponse> = Vec::new();
        for c in &connection_ids {
            let st = self.folded::<crate::contact::ContactType>(c)?.state();
            cacheable.extend(st.cacheable_discover_answers(me).into_iter().cloned());
            live.extend(
                st.live_discover_responses(now_ms)
                    .into_iter()
                    .filter(|r| r.origin == *me && r.holder != *me)
                    .cloned(),
            );
        }

        let mut best: BTreeMap<(MemberId, String), DiscoveredRow> = BTreeMap::new();
        for resp in newest_per_holder(cacheable).into_iter().chain(live) {
            if resp.hops > hops {
                continue;
            }
            for item in &resp.items {
                if item.kind != "place" || adopted.contains(&item.id) {
                    continue;
                }
                let key = (resp.holder, item.id.clone());
                if best.get(&key).is_some_and(|held| held.hops <= resp.hops) {
                    continue;
                }
                let point = match (item.lat_e7, item.lng_e7) {
                    (Some(lat_e7), Some(lng_e7)) => Some(crate::geo::GeoPoint {
                        lat_e7,
                        lng_e7,
                        ..Default::default()
                    }),
                    _ => None,
                };
                best.insert(
                    key,
                    DiscoveredRow {
                        id: item.id.clone(),
                        kind: item.kind.clone(),
                        name: item.name.clone(),
                        descriptor: item.descriptor.clone(),
                        point,
                        holder: Some(resp.holder),
                        hops: resp.hops,
                    },
                );
            }
        }
        out.extend(best.into_values());
        Ok(out)
    }

    /// The TRADE face: held Things, then the market's merged listings as rows. A
    /// listing needed no question to arrive (push gossip) and already persists in
    /// the folds — this read just gives Trade the same shape Discover gets.
    fn thing_rows(
        &self,
        hops: u32,
        me: &MemberId,
        now_ms: i64,
    ) -> Result<Vec<DiscoveredRow>, CoreError> {
        let mut out = Vec::new();
        for r in self.by_kind("thing")? {
            let st = self.folded::<crate::thing::ThingType>(&r.id)?.state();
            out.push(DiscoveredRow {
                id: hex::encode(&r.id),
                kind: "thing".to_string(),
                name: st.name,
                descriptor: st.descriptor,
                point: None, // a Thing's geography is a coarse area, never a point
                holder: None,
                hops: 0,
            });
        }
        if hops == 0 {
            return Ok(out);
        }
        for l in self.merged_listings(me)? {
            // Withdrawn/lapsed rows are merged first (the tombstone must win),
            // then dropped at read — `Node::inbound_listings`' discipline.
            if l.withdrawn || l.deadline.is_some_and(|d| d <= now_ms) {
                continue;
            }
            // A listing's `hops` counts forwards; its holder sits one further out.
            if l.hops + 1 > hops {
                continue;
            }
            out.push(DiscoveredRow {
                id: l.thing_id.clone(),
                kind: "thing".to_string(),
                name: l.title.clone(),
                descriptor: l.descriptor.clone(),
                point: None,
                holder: Some(l.origin),
                hops: l.hops + 1,
            });
        }
        Ok(out)
    }

    /// Every listing across every connection — tombstones INCLUDED — deduped by
    /// `(origin, thing_id)` keeping the newest revision and, among equals, the
    /// shortest path. MERGE FIRST, FILTER SECOND: a tombstone dropped per-channel
    /// before the merge leaves a stale live copy from another channel to win, and
    /// a sold bike never comes off the market. `me`'s own listings are excluded —
    /// my side is read from the Thing folds, where it is authoritative.
    pub fn merged_listings(
        &self,
        me: &MemberId,
    ) -> Result<Vec<crate::contact::Listing>, CoreError> {
        let mut best: BTreeMap<(MemberId, String), crate::contact::Listing> = BTreeMap::new();
        for r in self.by_kind("connection")? {
            let st = self.folded::<crate::contact::ContactType>(&r.id)?.state();
            for l in st.listings.values() {
                if l.origin == *me {
                    continue;
                }
                let key = (l.origin, l.thing_id.clone());
                let better = match best.get(&key) {
                    None => true,
                    Some(held) => l.rev > held.rev || (l.rev == held.rev && l.hops < held.hops),
                };
                if better {
                    best.insert(key, l.clone());
                }
            }
        }
        Ok(best.into_values().collect())
    }

    /// Every object, newest ACTIVITY first (most recently received delta).
    pub fn by_recency(&self) -> Result<Vec<ObjectRef>, CoreError> {
        let mut all = self.refs(|_| true)?;
        all.sort_by(|a, b| b.last_at.cmp(&a.last_at).then(a.id.cmp(&b.id)));
        Ok(all)
    }

    /// Objects that received a delta at or after `since` (unix ms) — the activity
    /// feed, and the incremental input a projection wants instead of a full sweep.
    pub fn active_since(&self, since: i64) -> Result<Vec<ObjectRef>, CoreError> {
        let mut hits: Vec<ObjectRef> = self
            .refs(|_| true)?
            .into_iter()
            .filter(|r| r.last_at >= since)
            .collect();
        hits.sort_by(|a, b| b.last_at.cmp(&a.last_at).then(a.id.cmp(&b.id)));
        Ok(hits)
    }

    /// The MLS roster of one object — membership's ONE source (the ratchet tree,
    /// via its rebuildable cache). Never a member list stored in a delta.
    pub fn members(&self, id: &[u8]) -> Result<Vec<MemberId>, CoreError> {
        self.dir.group_members(id)
    }

    /// Every object `who` is a member of, by kind. The inverse of [`members`] —
    /// "which groups is this person in", which membership edges in the graph need
    /// and which previously meant walking every group by hand.
    pub fn memberships_of(&self, who: &MemberId) -> Result<Vec<ObjectRef>, CoreError> {
        let mut out = Vec::new();
        for r in self.refs(|_| true)? {
            if self.dir.group_members(&r.id)?.iter().any(|m| m == who) {
                out.push(r);
            }
        }
        Ok(out)
    }

    // ---- internals -----------------------------------------------------------

    /// The object's owner, or a loud error.
    ///
    /// Never defaults: an object with no owner row is an object this device does
    /// not have, and folding it must FAIL rather than return a plausible empty
    /// state. A default owner would let "I was never given this group" and "this
    /// group is empty" render identically — and since owner authority is what
    /// `Sequenced` ops are checked against, a zeroed owner is also an authority
    /// check against a key nobody holds.
    fn owner(&self, id: &[u8]) -> Result<MemberId, CoreError> {
        let owner = self.dir.group_owner(id)?.ok_or_else(|| {
            CoreError::Directory(format!("group {} has no owner", hex::encode(id)))
        })?;
        owner
            .as_slice()
            .try_into()
            .map_err(|_| CoreError::Directory("owner_pk width".into()))
    }

    /// Build the fold-free refs for every group whose kind passes `keep`.
    fn refs(&self, keep: impl Fn(&str) -> bool) -> Result<Vec<ObjectRef>, CoreError> {
        let recency: BTreeMap<Vec<u8>, i64> = self.dir.log_recency()?.into_iter().collect();
        let mut out = Vec::new();
        for (id, kind) in self.dir.all_groups()? {
            if !keep(&kind) {
                continue;
            }
            let deltas = self.dir.log_len(&id)?;
            let last_at = recency.get(&id).copied().unwrap_or(0);
            out.push(ObjectRef {
                id,
                kind,
                deltas,
                last_at,
            });
        }
        Ok(out)
    }
}

/// The cache's selection rule, category-agnostic and PURE: one answer per holder —
/// the newest by the HOLDER's clock (relays carry it unchanged, so "newest" means
/// the same thing down any path), rid as the deterministic tie-break, and among
/// byte-identical answers the shortest path. Two devices holding the same folds
/// pick the same winners.
fn newest_per_holder(
    answers: Vec<crate::contact::DiscoverResponse>,
) -> Vec<crate::contact::DiscoverResponse> {
    let mut best: BTreeMap<MemberId, crate::contact::DiscoverResponse> = BTreeMap::new();
    for r in answers {
        let better = match best.get(&r.holder) {
            None => true,
            Some(held) => {
                (r.at, &r.rid) > (held.at, &held.rid)
                    || (r.at == held.at && r.rid == held.rid && r.hops < held.hops)
            }
        };
        if better {
            best.insert(r.holder, r);
        }
    }
    best.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contact::{DiscoverItem, DiscoverResponse};

    fn answer(holder: u8, rid: &str, name: &str, hops: u32, at: i64) -> DiscoverResponse {
        DiscoverResponse {
            origin: [1u8; 32],
            rid: rid.into(),
            holder: [holder; 32],
            items: vec![DiscoverItem {
                id: format!("obj-{name}"),
                kind: "place".into(),
                name: name.into(),
                descriptor: String::new(),
                lat_e7: None,
                lng_e7: None,
                vis: crate::visibility::Visibility::Network,
            }],
            hops,
            at,
            via: [9u8; 32],
        }
    }

    #[test]
    fn newest_per_holder_takes_the_holders_latest_statement() {
        // Two questions over time, answered by the same holder: the newer answer
        // wins WHOLESALE — the older set must not leak rows into the cache.
        let picked = newest_per_holder(vec![
            answer(2, "r1", "Old Café", 1, 1_000),
            answer(2, "r2", "New Café", 1, 2_000),
            answer(3, "r1", "Studio", 2, 500),
        ]);
        assert_eq!(picked.len(), 2, "one statement per holder");
        let names: Vec<&str> = picked.iter().map(|r| r.items[0].name.as_str()).collect();
        assert!(names.contains(&"New Café"), "holder 2's newest");
        assert!(!names.contains(&"Old Café"), "…and only the newest");
        assert!(names.contains(&"Studio"), "a 2-hop holder caches too (n-2)");
    }

    #[test]
    fn newest_per_holder_is_order_independent() {
        // The same answer reaching me down two paths (differing hops), folded in
        // either order, settles identically — the shortest path carries it.
        let short = answer(2, "r1", "Café", 1, 1_000);
        let long = DiscoverResponse {
            hops: 2,
            ..answer(2, "r1", "Café", 1, 1_000)
        };
        let a = newest_per_holder(vec![short.clone(), long.clone()]);
        let b = newest_per_holder(vec![long, short]);
        assert_eq!(a[0].hops, 1);
        assert_eq!(b[0].hops, 1, "order-independent");
    }
}

#[cfg(test)]
mod icd_dump {
    use crate::object::{Commutativity, ObjectType};
    fn dump<T: ObjectType>() {
        for o in T::ops() {
            println!(
                "ICD\t{}\t{}\t{}\t{}\t{}",
                T::KIND.name(),
                o.op_id,
                o.name,
                o.authority.web(),
                match o.commutativity {
                    Commutativity::Sequenced => "sequenced",
                    Commutativity::Commutative => "commutative",
                }
            );
        }
    }
    #[test]
    fn dump_all() {
        dump::<crate::group::GroupType>();
        dump::<crate::coordinator::ForumType>();
        dump::<crate::coordinator::ConversationType>();
        dump::<crate::project::ProjectType>();
        dump::<crate::contact::ContactType>();
        dump::<crate::thing::ThingType>();
        dump::<crate::place::PlaceType>();
        dump::<crate::event::EventType>();
    }
}

/// A fold's failure as the cache keeps it: the rows' own refusal (an authorship
/// signature that does not prove its author, a Delta the reducer rejects), never this
/// device's (storage, IO), which a later fold may not meet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    identity: bool,
    words: String,
}

impl Refusal {
    pub fn of(e: &CoreError) -> Option<Self> {
        match e {
            CoreError::Identity(w) => Some(Self { identity: true, words: w.clone() }),
            CoreError::Coordinator(w) => Some(Self { identity: false, words: w.clone() }),
            _ => None,
        }
    }

    pub fn into_error(self) -> CoreError {
        if self.identity {
            CoreError::Identity(self.words)
        } else {
            CoreError::Coordinator(self.words)
        }
    }
}
