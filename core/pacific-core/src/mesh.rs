//! mesh — the device-to-device store-and-forward mailbox (FireChat's model, on our frames).
//!
//! The relay we already have is a blind mailbox: it addresses by an opaque rotating tag and
//! never learns content, sender, group_id or the graph. That property is what makes a MESH
//! tractable here — a nearby phone can serve the same mailbox without understanding anything,
//! because a carried message is just bytes under a tag.
//!
//! CHUNKED, because a GroupObject Delta can be a photo. At BLE's 1–10 KB/s even a capped 1 MiB
//! message is ~100 s of contact, and a corridor encounter is seconds. Carried whole, such a
//! message would never move: every encounter would restart it from zero and throw away the
//! progress. So a message is
//! split into `CHUNK_BYTES` pieces, each independently addressed, asked for and carried. Partial
//! progress is then real progress — it survives the peer walking away and resumes with whoever
//! is next. It also stops one photo monopolising a link while a text message waits behind it.
//!
//! TWO LEVELS OF CONTENT ADDRESSING, and the distinction matters:
//!   * a MESSAGE is `sha256(tag ‖ blob)` — unchanged from before, so a message's mesh identity
//!     is the same whether it travelled whole or in pieces;
//!   * a CHUNK is `sha256(bytes)` — content only, so identical bytes in two messages are stored
//!     and carried once.
//!
//! Reassembly re-derives the message id from the pieces and refuses anything that does not match,
//! so a manifest cannot be used to make a peer assemble bytes nobody sent.
//!
//! THE STORE IS CAPPED IN BYTES, not messages. A count-based cap was fine when a message was a
//! couple of kilobytes and catastrophic the moment one can be megabytes — 4096 × 3 MB is 12 GB on
//! a stranger's phone.
//!
//! WHERE THIS COMES FROM. The shape is Vahdat & Becker's epidemic routing — summary vector, pull
//! what you lack — which is also Briar's Bramble Synchronisation Protocol (OFFER/REQUEST/MESSAGE/
//! ACK) and Serval's Rhizome. Three borrowings in particular:
//!   * SMALLEST FIRST (`by_priority`) is Rhizome's rule verbatim — it "gives priority to smaller
//!     items" — and shortest-job-first from scheduling theory, which minimises mean completion
//!     time. Under a short, unpredictable contact window it is the difference between finishing
//!     ten messages and starting one.
//!   * RESUMPTION IS THE WHOLE GAME. Serval's LBARD found its tree-sync "having to start its
//!     synchronisation again from scratch every time a radio connection is made" to be the thing
//!     that stopped it converging on slow links. Chunks persist in the store precisely so a
//!     contact that ends early leaves progress behind rather than wasted radio.
//!   * BOUNDED EVERYTHING. bitchat caps a message at 1 MiB and downscales images to 512 KiB
//!     before offering them; Briar caps a message body at 32 KiB outright. We started at 16 MB and
//!     have taken bitchat's ceiling (`MAX_MESSAGE_BYTES`) — chunking makes a bigger one resumable,
//!     but resumable is not the same as reasonable to ask a stranger's phone to carry.
//!
//! Not yet borrowed: bitchat's PER-DEPOSITOR quotas. One peer can currently fill our whole store.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use sha2::{Digest as _, Sha256};

/// Pieces are sized against an encounter, not a packet: ~32 KB is a handful of seconds even at
/// background throughput, so a chunk usually completes within a brief meeting. Smaller would
/// multiply per-chunk overhead and flood the inventory (a 3 MB message at 4 KB is 750 digests —
/// twelve exchanges just to DESCRIBE it); larger risks never finishing one.
pub const CHUNK_BYTES: usize = 32 * 1024;
/// How much of this phone the mesh may spend carrying for other people.
pub const DEFAULT_CAP_BYTES: usize = 64 * 1024 * 1024;
/// The largest single message the mesh will carry at all.
///
/// bitchat's number, adopted after reading theirs: `FileTransferLimits.maxPayloadBytes` is 1 MiB
/// "to keep payload sizes sane on constrained radios", with voice notes and images downscaled to
/// 512 KiB before they are ever offered. That is the most-shipped iOS BLE mesh choosing an order
/// of magnitude below where we started, and the arithmetic agrees: 1 MiB is ~100 s of contact at
/// 10 KB/s, which a bus ride affords; 16 MB is 27 minutes, which nothing does. Chunking means a
/// larger message would still be RESUMABLE — but resumable is not the same as reasonable to ask a
/// stranger's phone to carry, and smallest-first would leave it at the back of every queue anyway.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
/// How long a carried message stays worth carrying.
pub const DEFAULT_TTL_SECS: u64 = 24 * 60 * 60;
/// A message this far from its origin has had its chances; dropping it bounds the flood.
pub const MAX_HOPS: u8 = 8;

pub type Digest = [u8; 32];

/// A message's identity: `sha256(tag ‖ blob)`. Both halves are already opaque, so this leaks
/// nothing a carrier did not hold, and it is identical on every device — which is what makes
/// dedupe work across routes that never met.
pub fn message_id(tag: &Digest, blob: &[u8]) -> Digest {
    let mut h = Sha256::new();
    h.update(tag);
    h.update(blob);
    h.finalize().into()
}

/// A chunk's identity: the hash of its bytes alone, so the same bytes appearing in two messages
/// are carried once.
pub fn chunk_id(bytes: &[u8]) -> Digest {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().into()
}

/// What a message IS, once split: where it is addressed, how long it is, and the ordered pieces
/// it is made of. Small — a 3 MB message's manifest is about 3 KB — so a peer can learn that a
/// message exists, and decide whether to chase it, long before carrying any of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub id: Digest,
    pub tag: Digest,
    pub len: u64,
    pub chunks: Vec<Digest>,
    pub hops: u8,
    pub stored_at: u64,
}

/// One thing that can cross a link. A digest resolves to whichever of these the holder has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Manifest(Manifest),
    Chunk { id: Digest, bytes: Vec<u8> },
}

/// This device's bounded slice of the mesh.
pub struct MeshStore {
    manifests: HashMap<Digest, Manifest>,
    chunks: HashMap<Digest, Vec<u8>>,
    /// How many manifests reference each chunk, so evicting one message cannot pull the pieces
    /// out from under another that shares them.
    refs: HashMap<Digest, usize>,
    /// Message ids already handed to the local core. Kept apart from the store so that evicting
    /// something we carry for others can never cause us to re-deliver it to ourselves.
    delivered: HashSet<Digest>,
    bytes: usize,
    cap_bytes: usize,
    ttl_secs: u64,
    offer_cursor: usize,
}

impl Default for MeshStore {
    fn default() -> Self {
        Self::new(DEFAULT_CAP_BYTES, DEFAULT_TTL_SECS)
    }
}

impl MeshStore {
    pub fn new(cap_bytes: usize, ttl_secs: u64) -> Self {
        Self {
            manifests: HashMap::new(),
            chunks: HashMap::new(),
            refs: HashMap::new(),
            delivered: HashSet::new(),
            bytes: 0,
            cap_bytes,
            ttl_secs,
            offer_cursor: 0,
        }
    }

    /// Messages known about — complete or not.
    pub fn len(&self) -> usize {
        self.manifests.len()
    }

    pub fn is_empty(&self) -> bool {
        self.manifests.is_empty()
    }

    /// Chunk bytes currently held. What the cap is measured against.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Split one of our own sealed blobs into the mesh. Returns false if it is already held or
    /// too large to carry.
    pub fn carry(&mut self, tag: Digest, blob: Vec<u8>, hops: u8, now: u64) -> bool {
        if hops > MAX_HOPS || blob.len() > MAX_MESSAGE_BYTES {
            return false;
        }
        let id = message_id(&tag, &blob);
        if self.manifests.contains_key(&id) {
            return false;
        }
        let pieces: Vec<Vec<u8>> = if blob.is_empty() {
            vec![Vec::new()]
        } else {
            blob.chunks(CHUNK_BYTES).map(<[u8]>::to_vec).collect()
        };
        let manifest = Manifest {
            id,
            tag,
            len: blob.len() as u64,
            chunks: pieces.iter().map(|p| chunk_id(p)).collect(),
            hops,
            stored_at: now,
        };
        if !self.admit_manifest(manifest, now) {
            return false;
        }
        for p in pieces {
            self.admit_chunk(chunk_id(&p), p, now);
        }
        true
    }

    /// Take one item off the wire. Manifests are cheap and always welcome; a chunk nobody has a
    /// manifest for is refused, because an unreferenced chunk is bytes we can never use or serve
    /// usefully, and accepting them is how a peer fills our store with rubbish.
    pub fn accept(&mut self, item: Item, now: u64) -> bool {
        match item {
            Item::Manifest(mut m) => {
                if m.hops > MAX_HOPS || m.len as usize > MAX_MESSAGE_BYTES {
                    return false;
                }
                if self.manifests.contains_key(&m.id) {
                    return false;
                }
                m.stored_at = now;
                self.admit_manifest(m, now)
            }
            Item::Chunk { id, bytes } => {
                if chunk_id(&bytes) != id {
                    return false; // the bytes are not what was asked for
                }
                if self.chunks.contains_key(&id) {
                    return false;
                }
                if !self.refs.contains_key(&id) {
                    return false; // no manifest wants it
                }
                self.admit_chunk(id, bytes, now)
            }
        }
    }

    fn admit_manifest(&mut self, m: Manifest, now: u64) -> bool {
        self.evict_expired(now);
        for c in &m.chunks {
            *self.refs.entry(*c).or_insert(0) += 1;
        }
        self.manifests.insert(m.id, m);
        true
    }

    fn admit_chunk(&mut self, id: Digest, bytes: Vec<u8>, now: u64) -> bool {
        if self.chunks.contains_key(&id) {
            return true; // already held — shared with another message, and already paid for
        }
        self.evict_expired(now);
        let need = bytes.len();
        if self.bytes + need > self.cap_bytes && !self.make_room(need, now) {
            return false;
        }
        self.bytes += bytes.len();
        self.chunks.insert(id, bytes);
        true
    }

    /// Messages in SERVICE ORDER: smallest first, freshest as the tiebreak, id last so the order
    /// is total and two devices reason about it identically.
    ///
    /// Smallest-first is the Serval Rhizome rule ("Rhizome gives priority to smaller items"), and
    /// it is shortest-job-first from scheduling theory, which minimises mean completion time. In
    /// a mesh the argument is sharper than that: a contact window is short and its length is not
    /// known in advance, so the question at every encounter is "how many messages can I finish
    /// before this person walks away". Finishing ten small ones beats getting 3% into a photo,
    /// because a partial message delivers nothing to anybody — and, thanks to chunking, the photo
    /// keeps whatever it did get and resumes at the next encounter regardless.
    fn by_priority(&self) -> Vec<&Manifest> {
        let mut order: Vec<&Manifest> = self.manifests.values().collect();
        order.sort_by(|a, b| {
            a.len
                .cmp(&b.len)
                .then(b.stored_at.cmp(&a.stored_at))
                .then(a.id.cmp(&b.id))
        });
        order
    }

    /// The digests to offer in ONE encounter — manifest ids, bounded.
    ///
    /// Half the budget is the freshest traffic, so a new message spreads immediately. The other
    /// half ROTATES through the rest, advancing a cursor each time. That rotation is load-bearing:
    /// offering only the newest means a store larger than the limit re-offers the same top slice
    /// forever and the backlog behind it never moves.
    pub fn inventory_offer(&mut self, limit: usize) -> Vec<Digest> {
        if limit == 0 || self.manifests.is_empty() {
            return Vec::new();
        }
        let order = self.by_priority();

        let fresh_n = (limit / 2).max(1).min(order.len());
        let mut out: Vec<Digest> = order[..fresh_n].iter().map(|m| m.id).collect();
        let rest = &order[fresh_n..];
        if !rest.is_empty() {
            let take = (limit - fresh_n).min(rest.len());
            for k in 0..take {
                out.push(rest[(self.offer_cursor + k) % rest.len()].id);
            }
            self.offer_cursor = (self.offer_cursor + take) % rest.len();
        }
        out
    }

    /// Given the message ids a peer is offering, what to ask them for: manifests we have never
    /// seen, PLUS the missing pieces of anything we already know about but have not finished.
    ///
    /// That second half is the resumption mechanism. A peer offering a message we are halfway
    /// through is exactly who can finish it, and asking only for whole unknown messages would
    /// mean a large one restarting at every encounter and never completing.
    pub fn missing(&self, theirs: &[Digest]) -> Vec<Digest> {
        let mut out = Vec::new();
        for id in theirs {
            match self.manifests.get(id) {
                None => out.push(*id),
                Some(m) => out.extend(
                    m.chunks
                        .iter()
                        .filter(|c| !self.chunks.contains_key(*c))
                        .copied(),
                ),
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The pieces this device still lacks, across every message it knows about but has not
    /// finished — freshest message first, bounded.
    ///
    /// This is what keeps a long transfer moving. Asking only for the pieces of manifests learned
    /// in the last breath stalls the moment a reply carries chunks rather than manifests, which
    /// is most of a large message; asking for everything outstanding lets an exchange run until
    /// the peer stops having anything, which is the correct end of it.
    pub fn wanted(&self, limit: usize) -> Vec<Digest> {
        let mut out = Vec::new();
        for m in self.by_priority() {
            for c in &m.chunks {
                if !self.chunks.contains_key(c) {
                    out.push(*c);
                    if out.len() >= limit {
                        return out;
                    }
                }
            }
        }
        out
    }

    /// Resolve requested digests into items to send, bounded by count and bytes so one reply
    /// cannot monopolise a link. Manifests go first: they are tiny, and until a peer has one it
    /// cannot ask for anything else.
    pub fn serve(&self, want: &[Digest], max_items: usize, max_bytes: usize) -> Vec<Item> {
        let mut out = Vec::new();
        let mut budget = max_bytes;
        for d in want {
            let Some(m) = self.manifests.get(d) else {
                continue;
            };
            let cost = 80 + m.chunks.len() * 32;
            if cost > budget {
                break;
            }
            budget -= cost;
            let mut m = m.clone();
            m.hops = m.hops.saturating_add(1);
            out.push(Item::Manifest(m));
        }
        // Chunks in the order of the SMALLEST message that needs them, so a short contact
        // finishes small messages instead of nibbling at a big one. A peer's Want is a set, not a
        // queue — it carries no preference — so the order is ours to choose, and this is where the
        // smallest-first policy actually bites.
        let asked: HashSet<Digest> = want.iter().copied().collect();
        let mut sent: HashSet<Digest> = HashSet::new();
        let mut sent_chunks = 0;
        'messages: for m in self.by_priority() {
            for c in &m.chunks {
                if sent_chunks >= max_items {
                    break 'messages;
                }
                if !asked.contains(c) || sent.contains(c) {
                    continue;
                }
                let Some(bytes) = self.chunks.get(c) else {
                    continue;
                };
                if bytes.len() > budget {
                    continue;
                }
                budget -= bytes.len();
                sent_chunks += 1;
                sent.insert(*c);
                out.push(Item::Chunk {
                    id: *c,
                    bytes: bytes.clone(),
                });
            }
        }
        out
    }

    /// Messages on `tags` that are COMPLETE and not yet handed to the local core, reassembled and
    /// verified. A message whose pieces do not re-derive its id is dropped rather than delivered —
    /// the manifest came from a stranger, and this is the one place that matters.
    pub fn undelivered(&mut self, tags: &[Digest]) -> Vec<(Digest, Vec<u8>)> {
        let want: HashSet<Digest> = tags.iter().copied().collect();
        let ready: Vec<Digest> = self
            .manifests
            .values()
            .filter(|m| want.contains(&m.tag) && !self.delivered.contains(&m.id))
            .filter(|m| m.chunks.iter().all(|c| self.chunks.contains_key(c)))
            .map(|m| m.id)
            .collect();

        let mut out = Vec::new();
        let mut forged = Vec::new();
        for id in ready {
            let m = &self.manifests[&id];
            let mut blob = Vec::with_capacity(m.len as usize);
            for c in &m.chunks {
                blob.extend_from_slice(&self.chunks[c]);
            }
            if message_id(&m.tag, &blob) == m.id {
                self.delivered.insert(m.id);
                out.push((m.tag, blob));
            } else {
                forged.push(m.id);
            }
        }
        for id in forged {
            self.forget(&id);
        }
        out
    }

    /// True once every piece of this message is held.
    pub fn is_complete(&self, id: &Digest) -> bool {
        self.manifests
            .get(id)
            .is_some_and(|m| m.chunks.iter().all(|c| self.chunks.contains_key(c)))
    }

    /// Drop everything past its TTL.
    pub fn evict_expired(&mut self, now: u64) {
        let ttl = self.ttl_secs;
        let stale: Vec<Digest> = self
            .manifests
            .values()
            .filter(|m| now.saturating_sub(m.stored_at) >= ttl)
            .map(|m| m.id)
            .collect();
        for id in stale {
            self.forget(&id);
        }
    }

    /// Shed whole messages — furthest travelled first, oldest as the tiebreak — until `need` bytes
    /// fit. Whole messages, because half a message is worth nothing to anyone and keeping the
    /// remnant would just hold bytes hostage. Refuses if only messages nearer home would have to
    /// go, so a flood of far-travelled traffic cannot displace what this device most wants to keep.
    fn make_room(&mut self, need: usize, now: u64) -> bool {
        let mut victims: Vec<(u8, u64, Digest)> = self
            .manifests
            .values()
            .map(|m| (m.hops, m.stored_at, m.id))
            .collect();
        // Furthest travelled first; among equals, the oldest.
        victims.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let _ = now;
        for (_, _, id) in victims {
            if self.bytes + need <= self.cap_bytes {
                return true;
            }
            self.forget(&id);
        }
        self.bytes + need <= self.cap_bytes
    }

    /// Forget one message and any chunks it was the last claimant of.
    fn forget(&mut self, id: &Digest) {
        let Some(m) = self.manifests.remove(id) else {
            return;
        };
        for c in &m.chunks {
            let gone = match self.refs.get_mut(c) {
                Some(n) if *n > 1 => {
                    *n -= 1;
                    false
                }
                Some(_) => {
                    self.refs.remove(c);
                    true
                }
                None => true,
            };
            if gone {
                if let Some(bytes) = self.chunks.remove(c) {
                    self.bytes = self.bytes.saturating_sub(bytes.len());
                }
            }
        }
    }
}

/// THE device's store. One per process, not one per connection: a mesh only works because a phone
/// keeps carrying between encounters, so a store scoped to a sync would be empty every time it
/// mattered. The radio reaches it to reconcile with a peer; `Mailbox::Mesh` to publish and drain.
///
/// In memory for now, so a relaunch forgets what it was carrying for other people. A real
/// limitation for the carry-through-a-blackout story; not a correctness problem, because
/// everything here is content-addressed and re-offered at the next encounter.
pub fn shared() -> &'static Mutex<MeshStore> {
    static STORE: OnceLock<Mutex<MeshStore>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(MeshStore::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(n: u8) -> Digest {
        [n; 32]
    }

    /// One direction of an exchange: `to` asks `from` for what it lacks.
    fn pull(from: &mut MeshStore, to: &mut MeshStore, now: u64) -> bool {
        let offer = from.inventory_offer(64);
        let want = to.missing(&offer);
        if want.is_empty() {
            return false;
        }
        let mut moved = false;
        for item in from.serve(&want, 64, 1 << 20) {
            moved |= to.accept(item, now);
        }
        moved
    }

    /// Run two stores to convergence the way the link driver does.
    fn reconcile(a: &mut MeshStore, b: &mut MeshStore, now: u64) {
        for _ in 0..256 {
            let moved = pull(a, b, now) | pull(b, a, now);
            if !moved {
                return;
            }
        }
        panic!("exchange did not converge");
    }

    #[test]
    fn a_small_message_round_trips() {
        let mut s = MeshStore::default();
        assert!(s.carry(tag(1), b"sealed".to_vec(), 0, 100));
        assert!(
            !s.carry(tag(1), b"sealed".to_vec(), 3, 100),
            "dedupes by content"
        );
        let got = s.undelivered(&[tag(1)]);
        assert_eq!(got, vec![(tag(1), b"sealed".to_vec())]);
        assert!(s.undelivered(&[tag(1)]).is_empty(), "delivered once");
    }

    #[test]
    fn a_message_is_split_into_chunks() {
        let mut s = MeshStore::default();
        let blob: Vec<u8> = (0..CHUNK_BYTES * 3 + 11).map(|i| (i % 251) as u8).collect();
        s.carry(tag(1), blob.clone(), 0, 100);
        let m = s.manifests.values().next().unwrap();
        assert_eq!(m.chunks.len(), 4);
        assert_eq!(m.len as usize, blob.len());
        assert_eq!(s.bytes(), blob.len());
    }

    /// The whole point: a photo-sized message crosses a mesh in pieces.
    #[test]
    fn a_large_message_crosses_in_pieces() {
        let now = 1_000;
        let blob: Vec<u8> = (0..900_000u32).map(|i| (i % 251) as u8).collect();
        let (mut a, mut b) = (MeshStore::default(), MeshStore::default());
        a.carry(tag(9), blob.clone(), 0, now);
        reconcile(&mut a, &mut b, now);
        assert_eq!(b.undelivered(&[tag(9)]), vec![(tag(9), blob)]);
    }

    /// The property that makes chunking worth the complexity: an encounter that ends early leaves
    /// PROGRESS, and the next peer finishes the job.
    #[test]
    fn a_transfer_interrupted_midway_resumes_with_someone_else() {
        let now = 1_000;
        let blob: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let mut a = MeshStore::default();
        a.carry(tag(4), blob.clone(), 0, now);

        // B meets A briefly: long enough for the manifest and a couple of pieces, then A walks
        // away. Two rounds, because a manifest is what tells you the pieces exist at all.
        let mut b = MeshStore::default();
        let offer = a.inventory_offer(64);
        for item in a.serve(&b.missing(&offer), 2, 1 << 20) {
            b.accept(item, now);
        }
        for item in a.serve(&b.wanted(2), 2, 1 << 20) {
            b.accept(item, now);
        }
        let carried_after_brief_meeting = b.bytes();
        assert!(carried_after_brief_meeting > 0, "some pieces landed");
        assert!(
            !b.is_complete(&message_id(&tag(4), &blob)),
            "but not all of them"
        );
        assert!(
            b.undelivered(&[tag(4)]).is_empty(),
            "an incomplete message is never delivered"
        );

        // C has the whole thing. B asks only for the pieces it still lacks.
        let mut c = MeshStore::default();
        c.carry(tag(4), blob.clone(), 0, now);
        let offer = c.inventory_offer(64);
        let want = b.missing(&offer);
        assert!(
            want.len() < blob.len() / CHUNK_BYTES + 1,
            "resumption asks for less than the whole message"
        );
        reconcile(&mut b, &mut c, now);
        assert_eq!(b.undelivered(&[tag(4)]), vec![(tag(4), blob)]);
    }

    #[test]
    fn identical_chunks_in_two_messages_are_stored_once() {
        let mut s = MeshStore::default();
        let body = vec![3u8; CHUNK_BYTES];
        s.carry(tag(1), body.clone(), 0, 100);
        let before = s.bytes();
        s.carry(tag(2), body.clone(), 0, 100); // same bytes, different tag ⇒ different message
        assert_eq!(s.bytes(), before, "the shared chunk is not stored twice");
        assert_eq!(s.len(), 2, "but both messages are known");
    }

    #[test]
    fn evicting_one_message_keeps_chunks_another_still_needs() {
        let mut s = MeshStore::default();
        let body = vec![3u8; 1024];
        s.carry(tag(1), body.clone(), 0, 100);
        s.carry(tag(2), body.clone(), 0, 100);
        let id1 = message_id(&tag(1), &body);
        s.forget(&id1);
        assert_eq!(s.len(), 1);
        assert_eq!(
            s.undelivered(&[tag(2)]),
            vec![(tag(2), body)],
            "survivor is intact"
        );
    }

    /// A forged manifest must not make us assemble bytes nobody sent.
    #[test]
    fn a_manifest_whose_pieces_do_not_match_is_refused() {
        let now = 100;
        let mut donor = MeshStore::default();
        donor.carry(tag(5), b"honest bytes".to_vec(), 0, now);
        let real = donor.manifests.values().next().unwrap().clone();

        let mut s = MeshStore::default();
        let forged = Manifest {
            id: [0xAB; 32],
            ..real.clone()
        };
        assert!(s.accept(Item::Manifest(forged), now));
        for c in &real.chunks {
            s.accept(
                Item::Chunk {
                    id: *c,
                    bytes: donor.chunks[c].clone(),
                },
                now,
            );
        }
        assert!(
            s.undelivered(&[tag(5)]).is_empty(),
            "must not deliver a mismatched message"
        );
        assert_eq!(s.len(), 0, "and must drop it rather than keep retrying");
    }

    #[test]
    fn a_chunk_nobody_has_a_manifest_for_is_refused() {
        let mut s = MeshStore::default();
        let bytes = vec![1u8; 64];
        assert!(!s.accept(
            Item::Chunk {
                id: chunk_id(&bytes),
                bytes
            },
            100
        ));
        assert_eq!(s.bytes(), 0);
    }

    #[test]
    fn a_chunk_whose_bytes_do_not_match_its_digest_is_refused() {
        let mut s = MeshStore::default();
        s.carry(tag(1), vec![9u8; 100], 0, 100);
        assert!(!s.accept(
            Item::Chunk {
                id: [0xCD; 32],
                bytes: vec![0u8; 10]
            },
            100
        ));
    }

    /// The cap is BYTES. A count-based cap with megabyte messages is gigabytes on a phone.
    #[test]
    fn the_store_is_capped_in_bytes() {
        let mut s = MeshStore::new(CHUNK_BYTES * 4, DEFAULT_TTL_SECS);
        for i in 0..8u8 {
            s.carry(tag(i), vec![i; CHUNK_BYTES], 0, 100 + i as u64);
        }
        assert!(s.bytes() <= CHUNK_BYTES * 4, "held {} bytes", s.bytes());
    }

    #[test]
    fn eviction_sheds_the_furthest_travelled_first() {
        let mut s = MeshStore::new(CHUNK_BYTES * 2, DEFAULT_TTL_SECS);
        s.carry(tag(1), vec![1u8; CHUNK_BYTES], 0, 100);
        s.accept(
            Item::Manifest(Manifest {
                id: [2; 32],
                tag: tag(2),
                len: CHUNK_BYTES as u64,
                chunks: vec![chunk_id(&vec![2u8; CHUNK_BYTES])],
                hops: 6,
                stored_at: 100,
            }),
            100,
        );
        s.accept(
            Item::Chunk {
                id: chunk_id(&vec![2u8; CHUNK_BYTES]),
                bytes: vec![2u8; CHUNK_BYTES],
            },
            100,
        );

        s.carry(tag(3), vec![3u8; CHUNK_BYTES], 0, 101);
        assert!(
            s.manifests
                .contains_key(&message_id(&tag(1), &vec![1u8; CHUNK_BYTES])),
            "kept ours"
        );
        assert!(
            !s.manifests.contains_key(&[2; 32]),
            "shed the far-travelled one"
        );
    }

    #[test]
    fn expired_messages_stop_being_carried() {
        let mut s = MeshStore::new(1 << 20, 60);
        s.carry(tag(1), b"old".to_vec(), 0, 100);
        s.evict_expired(200);
        assert_eq!(s.len(), 0);
        assert_eq!(s.bytes(), 0);
    }

    #[test]
    fn an_oversized_message_is_refused_outright() {
        let mut s = MeshStore::new(usize::MAX, DEFAULT_TTL_SECS);
        assert!(!s.carry(tag(1), vec![0u8; MAX_MESSAGE_BYTES + 1], 0, 100));
    }

    #[test]
    fn a_message_past_the_hop_limit_is_refused() {
        let mut s = MeshStore::default();
        assert!(!s.carry(tag(1), b"far".to_vec(), MAX_HOPS + 1, 100));
    }

    #[test]
    fn only_the_asked_for_tags_are_delivered_locally() {
        let mut s = MeshStore::default();
        s.carry(tag(1), b"mine".to_vec(), 0, 100);
        s.carry(tag(2), b"a-strangers".to_vec(), 0, 100);
        assert_eq!(s.undelivered(&[tag(1)]), vec![(tag(1), b"mine".to_vec())]);
        assert_eq!(
            s.len(),
            2,
            "the stranger's message is still CARRIED, just not delivered"
        );
    }
}
