//! mesh_link — the wire between two devices in range, and the exchange that runs over it.
//!
//! Deliberately a BYTE STREAM abstraction, not a Bluetooth one. The radio's only job is to move
//! opaque bytes and say when a peer appears or leaves; framing, reassembly and the exchange
//! state machine all live here, in the language that can test them without a radio in the room.
//! That is also what lets the same code carry the mesh over BLE today and a LoRa link between
//! Arcs later — neither has to re-implement any of this.
//!
//! FRAMING. Length-prefixed (`u32` big-endian) rather than per-chunk flags, because the transport
//! chunks at ITS convenience: BLE negotiates an ATT MTU somewhere around 20–180 bytes and hands
//! you whatever fits, so a frame boundary and a chunk boundary have nothing to do with each other.
//! A length prefix survives any chunking; a flag scheme has to agree on one.
//!
//! WHY THE INVENTORY IS BOUNDED. A full store is 4096 digests = 128 KiB, and BLE moves roughly
//! 1–10 KiB/s — minutes of radio time for an encounter that may last seconds. So an exchange
//! offers only `INVENTORY_LIMIT` digests: half the freshest, half ROTATING through the rest (see
//! `MeshStore::inventory_offer`). Convergence is therefore INCREMENTAL — two devices that keep
//! meeting keep converging, and a backlog larger than one encounter can carry drains across
//! several. The rotation is load-bearing, not a refinement: offering only the newest digests
//! leaves a large store forever re-offering the same top slice, and the backlog never moves.
//! Proper set reconciliation (a Bloom filter or IBLT, trading a little accuracy for a fixed small
//! size) is the upgrade, not a rewrite.

use crate::mesh::{Digest, Item, Manifest, MeshStore, CHUNK_BYTES};

/// Digests offered per exchange — see the note above on BLE's throughput budget.
pub const INVENTORY_LIMIT: usize = 64;
/// Items per Deliver, and bytes per Deliver. Both bounded so one reply cannot monopolise a link:
/// a peer waiting on a short text message must not be stuck behind somebody's photo.
pub const MAX_DELIVER_ITEMS: usize = 8;
pub const MAX_DELIVER_BYTES: usize = 4 * CHUNK_BYTES;
/// Hard ceiling on a decoded frame. A peer is untrusted: it can claim any length it likes, and
/// without this a single 4-byte header could ask us to allocate 4 GiB. Sized to hold the largest
/// legitimate Deliver with room for its headers — chunking is what keeps this small, and keeping
/// it small is what stops a megabyte message poisoning a link instead of crossing it.
pub const MAX_FRAME: usize = MAX_DELIVER_BYTES + 64 * 1024;

const KIND_INVENTORY: u8 = 0x01;
const KIND_WANT: u8 = 0x02;
const KIND_DELIVER: u8 = 0x03;
const ITEM_MANIFEST: u8 = 0x01;
const ITEM_CHUNK: u8 = 0x02;

/// What one device says to another. Three messages: here's what I have, send me these, here they
/// are. No handshake, no identity, no session — two strangers' phones have nothing to establish,
/// because everything they exchange is already sealed and content-addressed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MeshFrame {
    Inventory(Vec<[u8; 32]>),
    Want(Vec<[u8; 32]>),
    Deliver(Vec<Item>),
}

impl MeshFrame {
    /// Encode with its `u32` length prefix, ready to be chunked by whatever is carrying it.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Vec::new();
        match self {
            Self::Inventory(d) => {
                body.push(KIND_INVENTORY);
                put_digests(&mut body, d);
            }
            Self::Want(d) => {
                body.push(KIND_WANT);
                put_digests(&mut body, d);
            }
            Self::Deliver(items) => {
                body.push(KIND_DELIVER);
                body.extend_from_slice(&(items.len() as u16).to_be_bytes());
                for it in items {
                    match it {
                        Item::Manifest(m) => {
                            body.push(ITEM_MANIFEST);
                            body.extend_from_slice(&m.id);
                            body.extend_from_slice(&m.tag);
                            body.extend_from_slice(&m.len.to_be_bytes());
                            body.push(m.hops);
                            put_digests(&mut body, &m.chunks);
                        }
                        Item::Chunk { id, bytes } => {
                            body.push(ITEM_CHUNK);
                            body.extend_from_slice(id);
                            body.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
                            body.extend_from_slice(bytes);
                        }
                    }
                }
            }
        }
        let mut out = Vec::with_capacity(4 + body.len());
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(&body);
        out
    }

    fn decode(body: &[u8]) -> Option<Self> {
        let (&kind, rest) = body.split_first()?;
        match kind {
            KIND_INVENTORY => Some(Self::Inventory(take_digests(rest)?)),
            KIND_WANT => Some(Self::Want(take_digests(rest)?)),
            KIND_DELIVER => {
                let (n, mut rest) = take_u16(rest)?;
                let mut items = Vec::with_capacity((n as usize).min(MAX_DELIVER_ITEMS * 2));
                for _ in 0..n {
                    let (&kind, tail) = rest.split_first()?;
                    rest = tail;
                    match kind {
                        ITEM_MANIFEST => {
                            if rest.len() < 73 {
                                return None;
                            }
                            let id: Digest = rest[..32].try_into().ok()?;
                            let tag: Digest = rest[32..64].try_into().ok()?;
                            let len = u64::from_be_bytes(rest[64..72].try_into().ok()?);
                            let hops = rest[72];
                            rest = &rest[73..];
                            let (count, tail) = take_u16(rest)?;
                            let need = count as usize * 32;
                            if tail.len() < need {
                                return None;
                            }
                            let chunks = tail[..need]
                                .chunks_exact(32)
                                .map(|c| c.try_into().expect("chunks_exact(32)"))
                                .collect();
                            rest = &tail[need..];
                            items.push(Item::Manifest(Manifest {
                                id,
                                tag,
                                len,
                                chunks,
                                hops,
                                // stamped by the receiving store, never trusted from the wire
                                stored_at: 0,
                            }));
                        }
                        ITEM_CHUNK => {
                            if rest.len() < 36 {
                                return None;
                            }
                            let id: Digest = rest[..32].try_into().ok()?;
                            let len = u32::from_be_bytes(rest[32..36].try_into().ok()?) as usize;
                            rest = &rest[36..];
                            if rest.len() < len {
                                return None;
                            }
                            items.push(Item::Chunk {
                                id,
                                bytes: rest[..len].to_vec(),
                            });
                            rest = &rest[len..];
                        }
                        _ => return None,
                    }
                }
                Some(Self::Deliver(items))
            }
            _ => None,
        }
    }
}

fn put_digests(body: &mut Vec<u8>, d: &[[u8; 32]]) {
    body.extend_from_slice(&(d.len() as u16).to_be_bytes());
    for x in d {
        body.extend_from_slice(x);
    }
}

fn take_u16(b: &[u8]) -> Option<(u16, &[u8])> {
    if b.len() < 2 {
        return None;
    }
    Some((u16::from_be_bytes(b[..2].try_into().ok()?), &b[2..]))
}

fn take_digests(b: &[u8]) -> Option<Vec<[u8; 32]>> {
    let (n, rest) = take_u16(b)?;
    let n = n as usize;
    if rest.len() < n * 32 {
        return None;
    }
    Some(
        rest.chunks_exact(32)
            .take(n)
            .map(|c| c.try_into().expect("chunks_exact(32)"))
            .collect(),
    )
}

/// Reassembles frames from however the radio happened to chunk them.
#[derive(Default)]
pub struct FrameReader {
    buf: Vec<u8>,
    /// Set once a length prefix has been read and found impossible. The peer is desynchronised
    /// or hostile and the byte stream can never resynchronise, so the link is poisoned rather
    /// than left silently consuming garbage.
    poisoned: bool,
}

impl FrameReader {
    /// Feed whatever arrived; get back every frame that is now complete.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<MeshFrame> {
        if self.poisoned {
            return Vec::new();
        }
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            if self.buf.len() < 4 {
                break;
            }
            let len = u32::from_be_bytes(self.buf[..4].try_into().expect("4 bytes")) as usize;
            if len > MAX_FRAME {
                self.poisoned = true;
                self.buf.clear();
                break;
            }
            if self.buf.len() < 4 + len {
                break;
            }
            if let Some(f) = MeshFrame::decode(&self.buf[4..4 + len]) {
                out.push(f);
            }
            self.buf.drain(..4 + len);
        }
        out
    }

    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }
}

/// Split an encoded frame for a radio with a fixed maximum write size.
pub fn chunks(bytes: &[u8], mtu: usize) -> Vec<Vec<u8>> {
    let mtu = mtu.max(1);
    bytes.chunks(mtu).map(<[u8]>::to_vec).collect()
}

/// The opening move when a peer comes into range: offer what we are carrying.
pub fn greet(store: &mut MeshStore) -> MeshFrame {
    MeshFrame::Inventory(store.inventory_offer(INVENTORY_LIMIT))
}

/// Handle one frame from a peer and return what to send back, carrying anything delivered.
///
/// Three cases, and the exchange terminates because each reply is strictly further along than
/// what prompted it: Inventory begets Want (or silence, when we already hold it all), Want begets
/// Deliver, and Deliver begets nothing. Two devices with identical stores exchange exactly one
/// frame each and stop — which is the common case for phones that keep meeting, and the reason
/// this is cheap enough to run on every encounter.
pub fn respond(store: &mut MeshStore, frame: MeshFrame, now: u64) -> Option<MeshFrame> {
    match frame {
        MeshFrame::Inventory(theirs) => {
            let want = store.missing(&theirs);
            (!want.is_empty()).then_some(MeshFrame::Want(want))
        }
        MeshFrame::Want(digests) => {
            let items = store.serve(&digests, MAX_DELIVER_ITEMS, MAX_DELIVER_BYTES);
            (!items.is_empty()).then_some(MeshFrame::Deliver(items))
        }
        MeshFrame::Deliver(items) => {
            // PROGRESS drives the exchange, not what kind of item arrived. A Deliver is bounded
            // (`MAX_DELIVER_ITEMS`/`MAX_DELIVER_BYTES`), so a large message crosses over many
            // rounds and most replies carry chunks rather than manifests — keying the next Want
            // on "did we learn a manifest" would stall after the first handful of pieces.
            // Keying it on "did anything land" instead runs the exchange until the peer stops
            // having anything, which is the correct end of it, and cannot loop: a reply that
            // delivers nothing new ends it.
            let mut progress = false;
            for it in items {
                progress |= store.accept(it, now);
            }
            if !progress {
                return None;
            }
            let want = store.wanted(INVENTORY_LIMIT);
            (!want.is_empty()).then_some(MeshFrame::Want(want))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{chunk_id, message_id, CHUNK_BYTES};

    fn tag(n: u8) -> Digest {
        [n; 32]
    }

    /// Drive two stores to convergence over a byte pipe chunked at a deliberately BLE-hostile
    /// MTU, so the framing is exercised by fragmentation rather than assumed to survive it.
    /// Returns the number of round trips — the encounter length this would need in the wild.
    fn converge(a: &mut MeshStore, b: &mut MeshStore, mtu: usize, now: u64) -> usize {
        let (mut ra, mut rb) = (FrameReader::default(), FrameReader::default());
        let mut to_b: Vec<u8> = greet(a).encode();
        let mut to_a: Vec<u8> = greet(b).encode();
        let mut rounds = 0;
        while !to_a.is_empty() || !to_b.is_empty() {
            rounds += 1;
            assert!(rounds < 4096, "exchange must terminate");
            let (mut next_a, mut next_b) = (Vec::new(), Vec::new());
            for chunk in chunks(&to_b, mtu) {
                for f in rb.push(&chunk) {
                    if let Some(r) = respond(b, f, now) {
                        next_a.extend_from_slice(&r.encode());
                    }
                }
            }
            for chunk in chunks(&to_a, mtu) {
                for f in ra.push(&chunk) {
                    if let Some(r) = respond(a, f, now) {
                        next_b.extend_from_slice(&r.encode());
                    }
                }
            }
            to_a = next_a;
            to_b = next_b;
        }
        rounds
    }

    fn sample_manifest() -> Manifest {
        let body = b"sealed".to_vec();
        Manifest {
            id: message_id(&tag(4), &body),
            tag: tag(4),
            len: body.len() as u64,
            chunks: vec![chunk_id(&body)],
            hops: 3,
            stored_at: 0,
        }
    }

    #[test]
    fn every_frame_survives_a_round_trip() {
        for f in [
            MeshFrame::Inventory(vec![tag(1), tag(2)]),
            MeshFrame::Want(vec![tag(9)]),
            MeshFrame::Deliver(vec![Item::Manifest(sample_manifest())]),
            MeshFrame::Deliver(vec![Item::Chunk {
                id: chunk_id(b"x"),
                bytes: b"x".to_vec(),
            }]),
            MeshFrame::Deliver(vec![
                Item::Manifest(sample_manifest()),
                Item::Chunk {
                    id: chunk_id(b"sealed"),
                    bytes: b"sealed".to_vec(),
                },
            ]),
        ] {
            let bytes = f.encode();
            let mut r = FrameReader::default();
            assert_eq!(r.push(&bytes), vec![f]);
        }
    }

    /// The property BLE actually needs: a frame split at 20-byte boundaries — the pessimistic
    /// ATT MTU — reassembles byte-identically.
    #[test]
    fn a_frame_split_at_ble_mtu_reassembles() {
        let bytes = vec![7u8; CHUNK_BYTES];
        let f = MeshFrame::Deliver(vec![Item::Chunk {
            id: chunk_id(&bytes),
            bytes,
        }]);
        let mut r = FrameReader::default();
        let mut got = Vec::new();
        for c in chunks(&f.encode(), 20) {
            got.extend(r.push(&c));
        }
        assert_eq!(got, vec![f]);
    }

    /// A full-sized Deliver must fit under the ceiling — chunking is what keeps a megabyte
    /// message from poisoning a link instead of crossing it.
    #[test]
    fn the_largest_legitimate_deliver_fits_under_the_frame_ceiling() {
        let items: Vec<Item> = (0..4u8)
            .map(|i| {
                let bytes = vec![i; CHUNK_BYTES];
                Item::Chunk {
                    id: chunk_id(&bytes),
                    bytes,
                }
            })
            .collect();
        let encoded = MeshFrame::Deliver(items).encode();
        assert!(
            encoded.len() <= MAX_FRAME,
            "{} > {}",
            encoded.len(),
            MAX_FRAME
        );
        let mut r = FrameReader::default();
        let mut got = Vec::new();
        for c in chunks(&encoded, 180) {
            got.extend(r.push(&c));
        }
        assert_eq!(got.len(), 1);
        assert!(!r.is_poisoned());
    }

    #[test]
    fn several_frames_in_one_chunk_all_arrive() {
        let mut bytes = MeshFrame::Want(vec![tag(1)]).encode();
        bytes.extend(MeshFrame::Want(vec![tag(2)]).encode());
        let mut r = FrameReader::default();
        assert_eq!(r.push(&bytes).len(), 2);
    }

    /// An untrusted peer must not be able to make us allocate on its say-so.
    #[test]
    fn an_absurd_length_prefix_poisons_the_link_instead_of_allocating() {
        let mut r = FrameReader::default();
        assert!(r.push(&[0xFF, 0xFF, 0xFF, 0xFF]).is_empty());
        assert!(r.is_poisoned());
        assert!(
            r.push(&MeshFrame::Want(vec![tag(1)]).encode()).is_empty(),
            "a poisoned link stays shut rather than pretending to resynchronise"
        );
    }

    #[test]
    fn two_peers_converge_over_a_chunked_pipe() {
        let now = 1_000;
        let (mut a, mut b) = (MeshStore::default(), MeshStore::default());
        a.carry(tag(1), b"from-a".to_vec(), 0, now);
        b.carry(tag(2), b"from-b".to_vec(), 0, now);

        converge(&mut a, &mut b, 20, now);

        assert_eq!(a.undelivered(&[tag(2)]), vec![(tag(2), b"from-b".to_vec())]);
        assert_eq!(b.undelivered(&[tag(1)]), vec![(tag(1), b"from-a".to_vec())]);
    }

    /// A photo-sized Delta crosses a link whose frames are capped far below it. This is the case
    /// that used to poison the connection outright.
    #[test]
    fn a_photo_sized_delta_crosses_a_ble_sized_link() {
        let now = 1_000;
        let blob: Vec<u8> = (0..900_000u32).map(|i| (i % 251) as u8).collect();
        let (mut a, mut b) = (MeshStore::default(), MeshStore::default());
        a.carry(tag(9), blob.clone(), 0, now);

        converge(&mut a, &mut b, 180, now);

        assert_eq!(b.undelivered(&[tag(9)]), vec![(tag(9), blob)]);
    }

    /// Count every byte a transfer actually puts on the wire, so "what can we get away with" is
    /// measured rather than guessed. Returns (bytes, frames) — a frame is one turn of the link,
    /// and a delivery costs two of them (the Want that asks and the Deliver that answers).
    fn wire_cost(blob_len: usize) -> (usize, usize) {
        let now = 1_000;
        let blob: Vec<u8> = (0..blob_len as u32).map(|i| (i % 251) as u8).collect();
        let (mut a, mut b) = (MeshStore::default(), MeshStore::default());
        a.carry(tag(9), blob.clone(), 0, now);

        let mut bytes = 0usize;
        let mut frames = 0usize;
        let mut pending = Some(greet(&mut a));
        while let Some(frame) = pending.take() {
            frames += 1;
            assert!(frames < 4096, "must terminate");
            let encoded = frame.encode();
            bytes += encoded.len();
            let mut reader = FrameReader::default();
            let decoded = reader.push(&encoded);
            assert_eq!(decoded.len(), 1, "frame must survive the wire");
            // Alternate sides: whoever received answers next.
            let (recv, send) = if frames % 2 == 1 {
                (&mut b, &mut a)
            } else {
                (&mut a, &mut b)
            };
            let _ = send;
            pending = respond(recv, decoded.into_iter().next().unwrap(), now);
        }
        assert_eq!(
            b.undelivered(&[tag(9)]),
            vec![(tag(9), blob)],
            "must actually arrive"
        );
        (bytes, frames)
    }

    /// The honest overhead figure for the largest message we will carry. Chunking is not free —
    /// every piece costs a 32-byte digest in a Want and a 37-byte header in a Deliver — and this
    /// pins how much. If it ever creeps far above a few percent, the chunk size is wrong.
    #[test]
    fn a_photo_sized_delta_costs_only_a_few_percent_in_overhead() {
        let len = 900_000;
        let (bytes, frames) = wire_cost(len);
        let overhead = (bytes as f64 - len as f64) / len as f64;
        println!(
            "900 KB Delta: {bytes} bytes on the wire ({:.2}% overhead), {frames} frames, \
             {} exchanges; at 10 KB/s that is {:.0}s of contact",
            overhead * 100.0,
            frames / 2,
            bytes as f64 / 10_240.0
        );
        assert!(overhead < 0.05, "{:.2}% overhead", overhead * 100.0);
        // Frames matter as much as bytes: each is a link turnaround an encounter must last for.
        // Bounded by MAX_DELIVER_BYTES rather than by chunk count, which is the point of the cap.
        assert!(
            frames < 2 * (len / MAX_DELIVER_BYTES) + 16,
            "{frames} frames"
        );
    }

    /// A short text Delta must stay cheap — chunking large messages must not tax small ones.
    #[test]
    fn a_small_delta_still_crosses_in_a_handful_of_frames() {
        let (bytes, frames) = wire_cost(400);
        println!("400-byte Delta: {bytes} bytes on the wire, {frames} frames");
        assert!(frames <= 6, "{frames} frames for a 400-byte message");
    }

    /// Run an exchange for a BOUNDED number of frames — a contact window that ends when the bus
    /// does, rather than when the protocol is finished. Returns the frames actually spent.
    fn contact(a: &mut MeshStore, b: &mut MeshStore, budget: usize, now: u64) -> usize {
        let mut pending = Some(greet(a));
        let mut spent = 0;
        while let Some(frame) = pending.take() {
            if spent >= budget {
                break;
            }
            spent += 1;
            let bytes = frame.encode();
            let mut reader = FrameReader::default();
            let f = reader.push(&bytes).into_iter().next().expect("one frame");
            let recv: &mut MeshStore = if spent % 2 == 1 { b } else { a };
            pending = respond(recv, f, now);
        }
        spent
    }

    /// THE SHORT-CONTACT TEST. Someone passes you at the door: seconds, not the whole bus ride.
    /// Enough for a dozen short Deltas, nowhere near enough for a photo. The small ones must all
    /// arrive; the photo must make progress and KEEP it. Serval's Rhizome takes the same line —
    /// it "gives priority to smaller items" — and shortest-job-first minimises mean completion
    /// time. (Given the whole five minutes a photo now finishes too, since `MAX_MESSAGE_BYTES`
    /// came down to bitchat's 1 MiB; this pins the behaviour when the contact is far shorter.)
    #[test]
    fn a_short_contact_finishes_the_small_messages_before_starting_the_big_one() {
        let now = 1_000;
        let (mut a, mut b) = (MeshStore::default(), MeshStore::default());

        // Twelve short Deltas — chat messages, receipts, a profile edit.
        let notes: Vec<Vec<u8>> = (0..12)
            .map(|i| format!("a short delta number {i}").into_bytes())
            .collect();
        for n in &notes {
            a.carry(tag(2), n.clone(), 0, now);
        }
        // …and one photo-sized Delta that is BOTH the biggest and the freshest, so freshness
        // cannot rescue this test. Size is the only thing that can put the small ones first.
        let photo: Vec<u8> = (0..900_000u32).map(|i| (i % 251) as u8).collect();
        a.carry(tag(1), photo.clone(), 0, now + 1);

        // A brief window — a few exchanges, not a journey.
        contact(&mut a, &mut b, 8, now);

        let arrived = b.undelivered(&[tag(2)]);
        assert_eq!(
            arrived.len(),
            notes.len(),
            "every short Delta should have made it"
        );
        assert!(
            b.undelivered(&[tag(1)]).is_empty(),
            "the photo cannot have finished in this window"
        );
        assert!(
            b.bytes() > 0,
            "but the photo's pieces are kept, not discarded"
        );

        // …and the next encounter picks the photo up where it stopped, rather than restarting.
        // (LBARD's hard-won lesson: sync that begins again from scratch on every connection is
        // the thing that never converges.)
        let before = b.bytes();
        contact(&mut a, &mut b, 8, now);
        assert!(b.bytes() > before, "progress resumed rather than restarted");
    }

    /// THE BUS RIDE. Ralph's case: you sit next to someone for five minutes. With the payload
    /// ceiling at bitchat's 1 MiB rather than our original 16 MB, that window is now comfortably
    /// enough for the LARGEST message the mesh will carry — which is the argument for the ceiling.
    /// At ~10 KB/s a full-size message is ~100 s of contact; five minutes is three times over.
    #[test]
    fn a_five_minute_contact_completes_the_largest_message_we_carry() {
        let now = 1_000;
        let blob: Vec<u8> = (0..crate::mesh::MAX_MESSAGE_BYTES as u32)
            .map(|i| (i % 251) as u8)
            .collect();
        let (mut a, mut b) = (MeshStore::default(), MeshStore::default());
        a.carry(tag(1), blob.clone(), 0, now);

        let (bytes, frames) = (blob.len(), contact(&mut a, &mut b, 4096, now));
        let seconds = bytes as f64 / 10_240.0;
        println!("1 MiB (max) Delta: {frames} frames, ~{seconds:.0}s of contact at 10 KB/s");

        assert_eq!(
            b.undelivered(&[tag(1)]),
            vec![(tag(1), blob)],
            "arrives whole"
        );
        assert!(
            seconds < 300.0,
            "must fit a five-minute ride, needs {seconds:.0}s"
        );
    }

    /// Devices that keep meeting must not keep paying.
    #[test]
    fn a_second_encounter_costs_one_frame_each() {
        let now = 1_000;
        let (mut a, mut b) = (MeshStore::default(), MeshStore::default());
        a.carry(tag(1), b"x".to_vec(), 0, now);
        converge(&mut a, &mut b, 180, now);
        assert_eq!(converge(&mut a, &mut b, 180, now), 1, "nothing left to say");
    }

    /// A backlog larger than one exchange can offer drains ACROSS encounters.
    #[test]
    fn a_backlog_larger_than_the_inventory_limit_drains_over_repeated_encounters() {
        let now = 1_000;
        let (mut a, mut b) = (MeshStore::default(), MeshStore::default());
        let total = INVENTORY_LIMIT * 3;
        for i in 0..total {
            a.carry(tag(1), format!("msg-{i}").into_bytes(), 0, now);
        }
        for _ in 0..8 {
            converge(&mut a, &mut b, 180, now);
        }
        assert_eq!(b.len(), total, "repeated encounters should finish the job");
    }
}
