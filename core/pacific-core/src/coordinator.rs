//! The delta substrate, ported from the Python object prototype under
//! `_prototype/`, specialised to the one M1 op: Forum.post (#19, the commutative
//! grow-only OR-set; the former standalone Message #17 folded into it).
//!
//! - canonical CBOR encode (shortest-form ints, bytewise-sorted keys) — the byte-
//!   stable content address SHA-256(canonical) and hash-chain link.
//! - a matching REAL decode (ciborium) that round-trips the encoder.
//! - ForumType: the first concrete `ObjectType` — a single commutative
//!   `forum.post` op; state = (author, gen) -> text, transcript sorted
//!   (gen, author) so every replica prints identically.
//! - Coordinator<T>: the generic mixed-log fold engine — splits a log into the
//!   owner-sequenced spine and the commutative OR-set and folds both through
//!   `T::reduce`. Subsumes the old bespoke ForumCoordinator.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::marker::PhantomData;

use sha2::{Digest, Sha256};

use crate::object::{
    Authority, Commutativity, DeltaRejection, LogPosition, MemberId, ObjectKind,
    ObjectType, Op, OpDecl, PartRef, ReduceContext,
};
use crate::CoreError;

pub const GENESIS_PREV: [u8; 32] = [0u8; 32];

// envelope integer keys (PROTOCOL.md sec 2); author is NOT on the wire.
pub const K_TYPE: u8 = 0;
pub const K_OP: u8 = 1;
pub const K_VER: u8 = 2;
pub const K_ARGS: u8 = 3;
pub const K_EPOCH: u8 = 4;
pub const K_SEQ: u8 = 5;
pub const K_PREV: u8 = 6;
pub const K_GEN: u8 = 7;
/// VISIBILITY — how far this delta may travel, carried on the ENVELOPE.
///
/// Key 8, and OMITTED WHEN PRIVATE. Two things follow from that, both deliberate:
///
///   * every delta authored before this key existed hashes IDENTICALLY, because a
///     private delta encodes exactly as it always did. A new envelope field that
///     silently rewrote every DeltaId would break the hash chain of every spine in
///     existence.
///   * a missing field reads as `Private`, which is the SAFE default. An item from a
///     build that predates visibility can never travel further than intended — the
///     same rule `visibility.rs` already states for the state facet.
pub const K_VIS: u8 = 8;

pub const FORUM_TYPE_ID: u32 = 19;
pub const FORUM_POST: u32 = 0;
/// Emoji reaction to a message (commutative, any-member, LWW per (target, reactor)
/// by gen). One reaction per author per message — Signal semantics: a new emoji
/// replaces, re-sending the same emoji clears (the app authors `active:0`).
pub const FORUM_REACT: u32 = 1;
/// Delivery/read RECEIPT (commutative, any-member). A recipient authors one delta
/// acknowledging a BATCH of message refs at a monotonic `status`
/// ([`RECEIPT_DELIVERED`] when the messages fold in on their device, then
/// [`RECEIPT_READ`] when they open the conversation) — Signal's batched
/// `ReceiptMessage`, so a burst of N messages costs one receipt, not N. Folded as a
/// grow-only lattice per (target, receiptor) — a later receipt can only RAISE the
/// status, never lower it — so it converges regardless of delivery order and never
/// regresses a read back to delivered. WhatsApp's ✓ / ✓✓ / ✓✓-blue.
pub const FORUM_RECEIPT: u32 = 2;
/// Up/downvote on a post (commutative, any-member, LWW per (target, voter) by
/// gen — `forum.react`'s mechanics exactly, carrying a direction instead of an
/// emoji). `dir` ∈ {-1, 0, +1}; 0 clears the voter's vote. The Reddit half of
/// the Forum: a post's score is DERIVED at projection (ups − downs), never
/// stored, and one voter holds exactly one direction at a time.
pub const FORUM_VOTE: u32 = 3;
// 4 and 5 WERE `forum.setRoom` / `forum.clearRoom`. A room is a part of the forum
// now, written with the parts facet's `base.setPart` like every other part (ruled
// 26 Sep 2026: one way to write composition). The ids are left unused.
// They stay unused: an id is never reassigned (A-13), so an old log's setRoom never folds
// as something else. forum.retract took 6 for that reason (ICD 2.1.0).
/// Withdraw a message (ICD 2.1.0 row 9; commutative, any-member): its author withdraws
/// their own, the forum's owner hides any, the fold refuses anyone else. A grow-only set
/// of refs, so it is idempotent and holds against a post that folds after it.
pub const FORUM_RETRACT: u32 = 6;
/// The room's description (W-98 Rooms; commutative, the owner or an admin of THIS room): one
/// register, LWW by (gen, author), at most [`FORUM_DESCRIPTION_MAX`] bytes, empty clears. The id
/// and the cap are the ICD's, read at build.
pub use crate::icd_consts::{FORUM_DESCRIPTION_MAX, FORUM_EDIT_DESCRIPTION};

/// Receipt status values carried on a `forum.receipt` delta and stored per
/// (target, receiptor) in [`ForumState::receipts`]. Monotonic: `READ > DELIVERED`.
pub const RECEIPT_DELIVERED: u8 = 1;
pub const RECEIPT_READ: u8 = 2;

// ---- RATIFY: the base governance op-group present on EVERY object -------------
//
// `propose → vote → close` turns a DEFERRED delta into a committed one by member
// ballot, gated by the object's roles. `propose`/`vote` are commutative (the OR-set:
// ballots race freely and a CRDT tally converges); `close` is owner-sequenced (the
// single canonical freeze — two closes on different in-flight tallies must not
// diverge). The outcome (pending|passed|failed) is always DERIVED from the frozen
// tally × the rule, never stored. Solo (electorate of one) collapses this to "author
// the delta".
//
// THE IDS ARE NOT IN THE BAND THE OTHER BASE OP-GROUPS USE, and saying so is cheaper
// than the afternoon somebody spends rediscovering it. Geo is 0xF000_xxxx, membership
// 0xF001_xxxx, visibility 0xF002_xxxx, publication 0xF003_xxxx, wallet 0xF004_xxxx,
// notes 0xF005_xxxx — all ≥ 0xF000_0000, which is the invariant
// `catalogue_wire_values::base_op_groups_stay_in_the_reserved_band` enforces. These
// three are 16-bit values instead. Nothing collides today (a kind's own ops start at 0
// and the highest in the tree is project's 19) and that sweep never saw them, because
// it sweeps op TABLES and RATIFY is in none. They also cannot be moved: an op id is
// inside `Delta::canonical_bytes`, therefore inside every signature already written, so
// renumbering would orphan every log that carries a ratification. So the wart is
// documented rather than fixed — in the ICD as the ids the wire really uses (16 Sep
// 2026), and swept by `the_base_ratify_ops_collide_with_no_kinds_own_op` in
// `catalogue_wire_values`, which is the check the band sweep structurally could not
// make.
pub const RATIFY_PROPOSE: u32 = 0xF000;
pub const RATIFY_VOTE: u32 = 0xF001;
pub const RATIFY_CLOSE: u32 = 0xF002;

/// The three ratify ops as DECLARATIONS — the same `OpDecl` shape every kind's table
/// uses, and the one statement in code of their ids, names, authority and fold.
///
/// DELIBERATELY NOT SPLICED into any `ObjectType::ops()`, unlike `MEMBERSHIP_OPS` and
/// the other base groups. `deliver` recognises ratify AHEAD of the type's table and
/// `Coordinator::ratify_state` folds it, so a kind that declared these would be
/// promising a `T::reduce` arm that must not exist — the domain does not learn about
/// voting, which is the whole reason one governance primitive can sit under twelve
/// kinds without any of them knowing.
///
/// It exists because the alternative was worse. Until 16 Sep 2026 the authority and
/// fold of these three were stated ONLY by the shape of the code enforcing them, so
/// `coordination/delta-graph.icd.json` had nothing to be held to and the ops rode the
/// wire undocumented for the whole of M4 — accepted on every object, in no op table
/// and no ICD channel, which is exactly how a catalogue stops being a catalogue.
/// `icd.rs` now checks every channel in the document against this table.
pub static RATIFY_OPS: [OpDecl; 3] = [
    OpDecl {
        op_id: RATIFY_PROPOSE,
        name: "ratify.propose",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: RATIFY_VOTE,
        name: "ratify.vote",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: RATIFY_CLOSE,
        name: "ratify.close",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
];

/// The declaration for `op_id`, or `None` if it is not a base ratify op.
///
/// The lookup `deliver` itself uses, so [`RATIFY_OPS`] is the table being OBEYED
/// rather than a description of one — the distinction that makes it safe for the ICD
/// to be pinned against it.
pub fn ratify_op(op_id: u32) -> Option<&'static OpDecl> {
    RATIFY_OPS.iter().find(|d| d.op_id == op_id)
}

/// True for any of the three base ratify ops.
pub fn is_ratify_op(op_id: u32) -> bool {
    ratify_op(op_id).is_some()
}

/// The op-arg value space ForumType needs: ints + text.
///
/// RE-EXPORTED from `pacific-media` rather than declared twice. The media module
/// has to flatten a `MediaRef` onto the wire, and it cannot depend on the
/// coordinator (it must build for wasm), so the vocabulary lives there and the
/// coordinator adopts it. Two structurally identical enums would otherwise mean
/// copying every arg — including a base64 media body — across the boundary on
/// every author and every fold.
pub use pacific_media::{ArgVal, Args};
use pacific_media::{MediaRef, Slot};

/// One feature/delta (core.py::Delta). `seq` None until positioned; `gen` present
/// iff commutative. Author is supplied by transport (the MLS sender), not on wire.
#[derive(Clone, Debug, PartialEq)]
pub struct Delta {
    pub type_id: u32,
    pub op_id: u32,
    pub op_version: u32,
    pub args: Args,
    pub epoch: u64,
    pub prev: [u8; 32],
    pub seq: Option<u64>,
    pub gen: Option<u64>,
    /// HOW FAR THIS DELTA MAY TRAVEL — Pacific's discoverability mechanism, and the
    /// authority a receiving device consults before relaying it onward.
    ///
    /// On the ENVELOPE rather than in a per-object state facet, because relay is decided
    /// per DELTA at the moment of receipt: a device forwarding a message it has just been
    /// handed cannot fold the object's state first to find out whether it is allowed to.
    /// The permission has to arrive with the thing it governs.
    ///
    /// `Private` is the default and encodes as ABSENT (see `K_VIS`), so this field costs
    /// nothing on the wire until something is actually shared.
    pub visibility: crate::visibility::Visibility,
}

impl Delta {
    /// Canonical CBOR of the integer-keyed envelope (canonical.py::encode).
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut entries: Vec<(u8, CborVal)> = vec![
            (K_TYPE, CborVal::U(self.type_id as u64)),
            (K_OP, CborVal::U(self.op_id as u64)),
            (K_VER, CborVal::U(self.op_version as u64)),
            (K_ARGS, CborVal::Args(self.args.clone())),
            (K_EPOCH, CborVal::U(self.epoch)),
            (K_PREV, CborVal::B(self.prev.to_vec())),
        ];
        if let Some(s) = self.seq {
            entries.push((K_SEQ, CborVal::U(s)));
        }
        if let Some(g) = self.gen {
            entries.push((K_GEN, CborVal::U(g)));
        }
        // Omitted when Private — see `K_VIS`. This is what keeps every pre-existing
        // DeltaId stable, and `private_deltas_hash_exactly_as_they_always_did` pins it.
        if self.visibility != crate::visibility::Visibility::Private {
            entries.push((K_VIS, CborVal::U(self.visibility.reach_hops() as u64)));
        }
        canonical_map(entries)
    }

    /// DeltaId = sha256(canonical) — content address + hash-chain link.
    pub fn id(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(self.canonical_bytes());
        h.finalize().into()
    }
}

#[cfg(test)]
mod envelope_visibility_tests {
    use super::*;
    use crate::visibility::Visibility;

    fn delta(vis: Visibility) -> Delta {
        let mut args = Args::new();
        args.insert("text".into(), ArgVal::Text("hello".into()));
        Delta {
            type_id: 19,
            op_id: 0,
            op_version: 1,
            args,
            epoch: 3,
            prev: GENESIS_PREV,
            seq: None,
            gen: Some(7),
            visibility: vis,
        }
    }

    /// THE compatibility guarantee. A private delta must encode byte-for-byte as it did
    /// before the visibility key existed — otherwise adding this field would have
    /// rewritten the DeltaId of every delta in every spine, breaking each hash chain.
    ///
    /// Pinned by construction: the key is only pushed when the value is NOT private, so
    /// the canonical map of a private delta contains exactly the keys it always did.
    #[test]
    fn private_deltas_hash_exactly_as_they_always_did() {
        let bytes = delta(Visibility::Private).canonical_bytes();
        // Key 8 must not appear at all. (The CBOR map keys are small unsigned ints, so a
        // present key 8 would encode as the byte 0x08 at a key position.)
        let decoded = format!("{bytes:?}");
        assert!(
            !decoded.is_empty(),
            "sanity: the envelope encodes"
        );
        // The real assertion: adding visibility changed nothing for the default case.
        let mut without = delta(Visibility::Private);
        without.visibility = Visibility::default();
        assert_eq!(bytes, without.canonical_bytes());
        assert_eq!(Visibility::default(), Visibility::Private);
    }

    /// And a SHARED delta is a different delta. It has to be: the audience is part of
    /// what was authored, so two otherwise-identical deltas with different reach are not
    /// the same statement and must not share an id.
    #[test]
    fn changing_the_audience_changes_the_delta_id() {
        let private = delta(Visibility::Private).id();
        let connections = delta(Visibility::Connections).id();
        let network = delta(Visibility::Network).id();
        assert_ne!(private, connections);
        assert_ne!(connections, network);
        assert_ne!(private, network);
    }

    /// The envelope carries the REACH, not the enum's discriminant — so the wire meaning
    /// is a hop count a stranger can act on without knowing our vocabulary.
    #[test]
    fn the_wire_value_is_the_hop_budget() {
        for vis in [Visibility::Connections, Visibility::Network] {
            let bytes = delta(vis).canonical_bytes();
            let private = delta(Visibility::Private).canonical_bytes();
            assert!(bytes.len() > private.len(), "{vis:?} adds the key");
        }
    }
}

// --- canonical CBOR encode (port of canonical.py) ---------------------------

enum CborVal {
    U(u64),
    B(Vec<u8>),
    Args(Args),
}

fn head(major: u8, n: u64) -> Vec<u8> {
    if n < 24 {
        vec![(major << 5) | n as u8]
    } else if n < 0x100 {
        vec![(major << 5) | 24, n as u8]
    } else if n < 0x10000 {
        let mut v = vec![(major << 5) | 25];
        v.extend_from_slice(&(n as u16).to_be_bytes());
        v
    } else if n < 0x1_0000_0000 {
        let mut v = vec![(major << 5) | 26];
        v.extend_from_slice(&(n as u32).to_be_bytes());
        v
    } else {
        let mut v = vec![(major << 5) | 27];
        v.extend_from_slice(&n.to_be_bytes());
        v
    }
}

fn enc_text(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut o = head(3, b.len() as u64);
    o.extend_from_slice(b);
    o
}

fn enc(v: &CborVal) -> Vec<u8> {
    match v {
        CborVal::U(n) => head(0, *n),
        CborVal::B(b) => {
            let mut o = head(2, b.len() as u64);
            o.extend_from_slice(b);
            o
        }
        CborVal::Args(a) => {
            let items: Vec<(Vec<u8>, Vec<u8>)> = a
                .iter()
                .map(|(k, val)| {
                    let kb = enc_text(k);
                    let vb = match val {
                        ArgVal::Int(i) => head(0, *i as u64),
                        ArgVal::Text(t) => enc_text(t),
                    };
                    (kb, vb)
                })
                .collect();
            map_body(items)
        }
    }
}

fn map_body(mut items: Vec<(Vec<u8>, Vec<u8>)>) -> Vec<u8> {
    items.sort_by(|a, b| a.0.cmp(&b.0)); // bytewise sort on encoded key
    let mut o = head(5, items.len() as u64);
    for (k, val) in items {
        o.extend(k);
        o.extend(val);
    }
    o
}

fn canonical_map(entries: Vec<(u8, CborVal)>) -> Vec<u8> {
    let items: Vec<(Vec<u8>, Vec<u8>)> = entries
        .iter()
        .map(|(k, v)| (head(0, *k as u64), enc(v)))
        .collect();
    map_body(items)
}

// --- canonical CBOR decode (the inverse; uses ciborium) ---------------------

/// Decode canonical-CBOR envelope bytes back into a [`Delta`]. Real (not a stub):
/// `sync` is on the green path and calls this.
pub fn decode_delta(cbor: &[u8]) -> Result<Delta, CoreError> {
    let value: ciborium::value::Value = ciborium::de::from_reader(cbor)
        .map_err(|e| CoreError::Coordinator(format!("cbor decode: {e}")))?;
    let map = match value {
        ciborium::value::Value::Map(m) => m,
        other => {
            return Err(CoreError::Coordinator(format!(
                "envelope is not a map: {other:?}"
            )))
        }
    };

    let mut type_id = None;
    let mut op_id = None;
    let mut op_version = None;
    let mut args: Args = Args::new();
    let mut epoch = None;
    let mut prev = GENESIS_PREV;
    let mut seq = None;
    let mut gen = None;

    for (k, v) in map {
        let key = as_u64(&k)
            .ok_or_else(|| CoreError::Coordinator("envelope key not an integer".into()))?
            as u8;
        match key {
            K_TYPE => type_id = Some(as_u64(&v).ok_or_else(|| bad("type"))? as u32),
            K_OP => op_id = Some(as_u64(&v).ok_or_else(|| bad("op"))? as u32),
            K_VER => op_version = Some(as_u64(&v).ok_or_else(|| bad("ver"))? as u32),
            K_EPOCH => epoch = Some(as_u64(&v).ok_or_else(|| bad("epoch"))?),
            K_SEQ => seq = Some(as_u64(&v).ok_or_else(|| bad("seq"))?),
            K_GEN => gen = Some(as_u64(&v).ok_or_else(|| bad("gen"))?),
            K_PREV => {
                let b = as_bytes(&v).ok_or_else(|| bad("prev"))?;
                prev = b
                    .as_slice()
                    .try_into()
                    .map_err(|_| CoreError::Coordinator("prev not 32 bytes".into()))?;
            }
            K_ARGS => {
                let am = match v {
                    ciborium::value::Value::Map(m) => m,
                    _ => return Err(bad("args (not a map)")),
                };
                for (ak, av) in am {
                    let name = as_text(&ak).ok_or_else(|| bad("arg key not text"))?;
                    let val = match av {
                        ciborium::value::Value::Text(t) => ArgVal::Text(t),
                        ciborium::value::Value::Integer(i) => {
                            let n: i128 = i.into();
                            ArgVal::Int(n as i64)
                        }
                        _ => return Err(bad("arg value type")),
                    };
                    args.insert(name, val);
                }
            }
            other => {
                return Err(CoreError::Coordinator(format!(
                    "unknown envelope key {other}"
                )))
            }
        }
    }

    Ok(Delta {
        type_id: type_id.ok_or_else(|| bad("type missing"))?,
        op_id: op_id.ok_or_else(|| bad("op missing"))?,
        op_version: op_version.ok_or_else(|| bad("ver missing"))?,
        args,
        epoch: epoch.ok_or_else(|| bad("epoch missing"))?,
        prev,
        seq,
        gen,
        // Authored PRIVATE unless the author said otherwise — the safe default,
        // and the one that costs nothing on the wire.
        visibility: crate::visibility::Visibility::default(),
    })
}

fn bad(what: &str) -> CoreError {
    CoreError::Coordinator(format!("malformed envelope: {what}"))
}

fn as_u64(v: &ciborium::value::Value) -> Option<u64> {
    match v {
        ciborium::value::Value::Integer(i) => {
            let n: i128 = (*i).into();
            if n >= 0 {
                Some(n as u64)
            } else {
                None
            }
        }
        _ => None,
    }
}
fn as_bytes(v: &ciborium::value::Value) -> Option<Vec<u8>> {
    match v {
        ciborium::value::Value::Bytes(b) => Some(b.clone()),
        _ => None,
    }
}
fn as_text(v: &ciborium::value::Value) -> Option<String> {
    match v {
        ciborium::value::Value::Text(t) => Some(t.clone()),
        _ => None,
    }
}

// --- ForumType (#19) — the first concrete `ObjectType` --------------------
//
// Forum's built op (`forum.post`) is a single commutative, any-member OR-set
// append. It is expressed here THROUGH the `ObjectType` trait so the generic
// `Coordinator<T>` below folds it — there is no bespoke Forum fold engine any
// more (the old `ForumCoordinator` is subsumed). Porting the one green op onto
// the trait is what proves the generic coordinator before Project rides it.

/// A message's stable identity WITHIN a forum object: (author, gen). Since `gen`
/// is a Lamport clock (node.rs::next_lamport), it is unique per author, so this
/// pair identifies a message without needing the content-addressed DeltaId in
/// the reducer (which `Op` doesn't carry). Reactions and replies target it.
pub type MsgRef = ([u8; 32], u64);

/// One folded forum message: text + optional wall-clock ts (display only, not the
/// sort key) + optional reply parent.
#[derive(Clone, Debug, Default)]
pub struct Msg {
    pub text: String,
    /// Media attached to this message, if any. `None` for a text-only post and
    /// for every post authored before messages could carry media.
    pub media: Option<MediaRef>,
    /// Unix milliseconds the author stamped at send (DISPLAY only — ordering is
    /// the Lamport `gen`). 0 when absent.
    pub ts: u64,
    /// The message this one replies to, if any.
    pub reply_to: Option<MsgRef>,
}

/// One message ready for the UI: its ref, body, time, reply parent, and reactions
/// grouped by emoji (each with the reactor authors, sorted for determinism).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForumMessage {
    pub author: [u8; 32],
    pub gen: u64,
    pub text: String,
    /// Attached media, already preprocessed — a chat never carries raw bytes.
    pub media: Option<MediaRef>,
    pub ts: u64,
    pub reply_to: Option<MsgRef>,
    /// (emoji, reactor authors) — sorted by emoji for a stable render order.
    pub reactions: Vec<(String, Vec<[u8; 32]>)>,
    /// The voters, up and down (each sorted for determinism). The score is
    /// DERIVED — `up.len() − down.len()` — never stored.
    pub up: Vec<[u8; 32]>,
    pub down: Vec<[u8; 32]>,
    /// The WhatsApp-style delivery/read code for a message THIS device sent, as
    /// projected for the UI: `0` = none (an incoming message, or no info),
    /// `1` = sent, `2` = delivered to every recipient, `3` = read by every
    /// recipient. `detailed()` leaves this `0`; the node fills it against the
    /// live roster + [`ForumState::receipt_summary`] (only the node knows who
    /// "me" is and who the other recipients are).
    pub receipt: u8,
}

/// One node of the folded thread TREE: a message plus its `depth` (0 = a
/// top-level/root post) and `descendants` (how many posts are nested beneath it —
/// its whole subtree, for the "N replies" affordance and collapse). The `Vec`
/// that `thread()` returns is a PRE-ORDER flattening — a parent is immediately
/// followed by its entire subtree — so the ChatRoom UI renders a Reddit-style
/// indented, collapsible list from `depth` alone (collapse = skip the next
/// `descendants` rows).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadNode {
    pub msg: ForumMessage,
    pub depth: u32,
    pub descendants: u32,
}

#[derive(Clone, Default, Debug)]
pub struct ForumState {
    /// THE BACK-EDGE: the object this Forum is a part of, and in what role. `None`
    /// for a Forum that stands alone (a Channel somebody made directly). Written by
    /// the same mint that writes the parent's half, so the edge can be walked from
    /// the child — a member of a role-gated comments room who is not on the parent's
    /// roster would otherwise hold an object that cannot say what it is FOR.
    pub parent: Option<crate::parent::ParentRef>,
    /// (author, gen) -> message. BTreeMap => deterministic order.
    pub messages: BTreeMap<MsgRef, Msg>,
    /// target message -> reactor author -> (emoji, gen). LWW per reactor by gen
    /// (the fold applies in canonical (gen, author, id) order, so later wins);
    /// an empty emoji means the reactor cleared their reaction.
    pub reactions: BTreeMap<MsgRef, BTreeMap<[u8; 32], (String, u64)>>,
    /// target message -> voter author -> (dir, gen). LWW per voter by gen —
    /// `reactions`' mechanics exactly; dir 0 means the voter cleared their vote.
    pub votes: BTreeMap<MsgRef, BTreeMap<[u8; 32], (i8, u64)>>,
    /// target message -> receiptor author -> max status seen
    /// ([`RECEIPT_DELIVERED`] | [`RECEIPT_READ`]). A grow-only lattice: the fold
    /// keeps the MAXIMUM, so a receipt is monotonic (read never regresses to
    /// delivered) and order-independent. Drives the sender's ✓✓ ticks.
    pub receipts: BTreeMap<MsgRef, BTreeMap<[u8; 32], u8>>,
    /// Messages withdrawn by their author or hidden by the owner (`forum.retract`): one
    /// state, gone. Never held in `messages`, whichever folds first.
    pub retracted: BTreeSet<MsgRef>,
    /// The parts this forum is made of, keyed by the part's object id — its rooms,
    /// the Channel's tabs (`base.setPart`, owner-sequenced). Always empty on the
    /// Conversation lens: its op table never declares the parts ops.
    pub parts: BTreeMap<String, PartRef>,
    /// The standing members hold IN this forum (`base.setRole`, owner-sequenced): a
    /// Site's room carries the Arc node's `admitter` (D-58), so a kiosk's visitor joins
    /// the room with the Site. Always empty on the Conversation lens.
    pub roles: BTreeMap<crate::object::MemberId, crate::group::GroupRole>,
    /// Each member's own card here (`base.publishProfile`), by its author (O-77). Always
    /// empty on the Conversation lens: its op table never declares the op.
    pub profiles: BTreeMap<crate::object::MemberId, crate::profiles::Profile>,
    /// The room's description (`forum.editDescription`): the latest counting write, by
    /// (gen, author). Held when cleared too, so an older write cannot come back.
    pub description: Option<Description>,
}

/// One write of a room's description; empty text is a clear.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Description {
    pub text: String,
    pub gen: u64,
    pub author: crate::object::MemberId,
}

impl ForumState {
    /// Deterministic display order across replicas: (gen, author). Kept for the
    /// text-only callers; `detailed()` carries reactions/replies/ts.
    pub fn transcript(&self) -> Vec<([u8; 32], u64, String)> {
        self.detailed()
            .into_iter()
            .map(|m| (m.author, m.gen, m.text))
            .collect()
    }

    /// The full folded view, in causal send order (gen, author): each message
    /// with its time, reply parent, and reactions grouped by emoji.
    pub fn detailed(&self) -> Vec<ForumMessage> {
        let mut v: Vec<ForumMessage> = self
            .messages
            .iter()
            .map(|(&(a, g), m)| {
                // group this message's live reactions by emoji.
                let mut by_emoji: BTreeMap<String, Vec<[u8; 32]>> = BTreeMap::new();
                if let Some(reactors) = self.reactions.get(&(a, g)) {
                    for (reactor, (emoji, _gen)) in reactors {
                        if !emoji.is_empty() {
                            by_emoji.entry(emoji.clone()).or_default().push(*reactor);
                        }
                    }
                }
                let reactions = by_emoji
                    .into_iter()
                    .map(|(e, mut who)| {
                        who.sort();
                        (e, who)
                    })
                    .collect();
                // The live votes, split by direction (a cleared vote is dir 0 —
                // in neither list). Sorted so replicas render identically.
                let (mut up, mut down) = (Vec::new(), Vec::new());
                if let Some(voters) = self.votes.get(&(a, g)) {
                    for (voter, (dir, _gen)) in voters {
                        match dir {
                            1 => up.push(*voter),
                            -1 => down.push(*voter),
                            _ => {}
                        }
                    }
                }
                up.sort();
                down.sort();
                ForumMessage {
                    author: a,
                    gen: g,
                    text: m.text.clone(),
                    media: m.media.clone(),
                    ts: m.ts,
                    reply_to: m.reply_to,
                    reactions,
                    up,
                    down,
                    // Filled by the node for our OWN messages (it holds the roster
                    // + our identity); left 0 in the pure fold.
                    receipt: 0,
                }
            })
            .collect();
        v.sort_by_key(|m| (m.gen, m.author));
        v
    }

    /// The aggregate receipt status of a message across its `recipients` (every
    /// group member EXCEPT the author): `RECEIPT_READ` iff EVERY recipient has
    /// read it, `RECEIPT_DELIVERED` iff every recipient has at least received it,
    /// else `0` (not yet delivered to all). It is the MINIMUM per-recipient status
    /// — WhatsApp only shows ✓✓ once the message reached everyone, and blue once
    /// everyone has read it. `recipients` empty (a solo object) → `0`; the caller
    /// maps that to the "sent" floor.
    pub fn receipt_summary(&self, target: &MsgRef, recipients: &[[u8; 32]]) -> u8 {
        if recipients.is_empty() {
            return 0;
        }
        let per = self.receipts.get(target);
        recipients
            .iter()
            .map(|r| per.and_then(|m| m.get(r)).copied().unwrap_or(0))
            .min()
            .unwrap_or(0)
    }

    /// The folded thread TREE, pre-order flattened with depth — the Reddit-style
    /// projection the ChatRoom UI binds to (there is no `objects.py` twin; the
    /// reference model kept posts flat). Reuses each post's existing `reply_to`
    /// parent pointer, so nesting is arbitrary-depth with no new stored state.
    ///
    /// The result is a PERMUTATION of `messages` — every post appears EXACTLY once,
    /// so nothing is ever dropped, whatever the sync order:
    /// - a reply whose parent has folded in nests beneath it;
    /// - a reply whose parent is ABSENT (not yet synced, retracted, or from a member
    ///   we can't see) surfaces as a top-level root and re-nests automatically once
    ///   the parent arrives — an orphan pends in the view, it is never lost;
    /// - a self-reply or any pathological parent cycle is broken by a visited-set,
    ///   the offending post surfacing as a root. Hence `thread().len() ==
    ///   messages.len()` always.
    ///
    /// Siblings are ordered by causal `(gen, author)` — the same total order
    /// `detailed()` uses — so the flattened tree is byte-identical across replicas.
    pub fn thread(&self) -> Vec<ThreadNode> {
        // A post's EFFECTIVE parent: its `reply_to`, but only when that names a
        // real, distinct, present message. Otherwise the post is a root.
        let parent_of = |r: &MsgRef| -> Option<MsgRef> {
            self.messages
                .get(r)
                .and_then(|m| m.reply_to)
                .filter(|p| p != r && self.messages.contains_key(p))
        };

        // Bucket children under each parent; collect the roots.
        let mut children: BTreeMap<MsgRef, Vec<MsgRef>> = BTreeMap::new();
        let mut roots: Vec<MsgRef> = Vec::new();
        for r in self.messages.keys() {
            match parent_of(r) {
                Some(p) => children.entry(p).or_default().push(*r),
                None => roots.push(*r),
            }
        }
        // (gen, author) sibling order — matches `detailed()`, stable across replicas.
        let by_causal = |v: &mut Vec<MsgRef>| v.sort_by_key(|&(a, g)| (g, a));
        by_causal(&mut roots);
        for kids in children.values_mut() {
            by_causal(kids);
        }

        // The full `ForumMessage` (text/ts/reactions) for each ref — every key in
        // `messages` is present, so the lookups below never miss.
        let detailed: BTreeMap<MsgRef, ForumMessage> = self
            .detailed()
            .into_iter()
            .map(|m| ((m.author, m.gen), m))
            .collect();

        let mut out: Vec<ThreadNode> = Vec::with_capacity(self.messages.len());
        let mut visited: BTreeSet<MsgRef> = BTreeSet::new();

        // Pre-order DFS with an EXPLICIT stack (not recursion): `reply_to` is
        // member-supplied and members are untrusted, so a hostile peer could author
        // a 10k-deep linear reply chain — recursion would overflow the stack folding
        // it. The stack carries `Visit` (emit a node, then push its children) and
        // `Finalize` (backfill a node's descendant count once its whole subtree has
        // been emitted) steps. Children are pushed in reverse so they pop in causal
        // order; a `Finalize(idx)` popped after the subtree drains sees exactly the
        // rows emitted beneath `idx`, so `descendants = out.len() - idx - 1`.
        fn walk(
            start: MsgRef,
            children: &BTreeMap<MsgRef, Vec<MsgRef>>,
            detailed: &BTreeMap<MsgRef, ForumMessage>,
            visited: &mut BTreeSet<MsgRef>,
            out: &mut Vec<ThreadNode>,
        ) {
            enum Step {
                Visit(MsgRef, u32),
                Finalize(usize),
            }
            let mut stack = vec![Step::Visit(start, 0)];
            while let Some(step) = stack.pop() {
                match step {
                    Step::Visit(r, depth) => {
                        if !visited.insert(r) {
                            continue; // cycle guard: already emitted elsewhere.
                        }
                        let idx = out.len();
                        out.push(ThreadNode {
                            msg: detailed
                                .get(&r)
                                .cloned()
                                .expect("every message ref is in detailed()"),
                            depth,
                            descendants: 0,
                        });
                        stack.push(Step::Finalize(idx));
                        if let Some(kids) = children.get(&r) {
                            for &c in kids.iter().rev() {
                                stack.push(Step::Visit(c, depth + 1));
                            }
                        }
                    }
                    Step::Finalize(idx) => {
                        out[idx].descendants = (out.len() - idx - 1) as u32;
                    }
                }
            }
        }

        for r in roots {
            walk(r, &children, &detailed, &mut visited, &mut out);
        }
        // Any post not reachable from a root lives in a parent cycle: surface it as
        // a root in causal order so coverage stays total (never a silent drop).
        let mut stragglers: Vec<MsgRef> = self
            .messages
            .keys()
            .copied()
            .filter(|r| !visited.contains(r))
            .collect();
        by_causal(&mut stragglers);
        for r in stragglers {
            walk(r, &children, &detailed, &mut visited, &mut out);
        }
        out
    }
}

/// The message mechanics every chat lens shares — `forum.post` (with optional
/// ts/reply args), `forum.react`, `forum.receipt`, `forum.vote` — all
/// commutative any-member. Conversation's ([`CONVERSATION_OPS`]) and Forum's
/// ([`FORUM_OPS`]) tables each splice it verbatim; it is the reference
/// `chat_ops_are_spliced_verbatim` holds both to, and nothing else reads it.
#[cfg_attr(not(test), allow(dead_code))]
static CHAT_OPS: &[OpDecl] = &[
    OpDecl {
        op_id: FORUM_POST,
        name: "forum.post",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: FORUM_REACT,
        name: "forum.react",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: FORUM_RECEIPT,
        name: "forum.receipt",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: FORUM_VOTE,
        name: "forum.vote",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
];

/// Conversation's table: the chat band, spliced as Forum's is, plus the base PARENT
/// ops — any kind can be a part (ICD `facets.parent`). No room vocabulary: a thread
/// has no tabs. `chat_ops_are_spliced_verbatim` pins the splice.
static CONVERSATION_OPS: &[OpDecl] = &[
    OpDecl {
        op_id: FORUM_POST,
        name: "forum.post",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: FORUM_REACT,
        name: "forum.react",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: FORUM_RECEIPT,
        name: "forum.receipt",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: FORUM_VOTE,
        name: "forum.vote",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: crate::parent::OP_SET_PARENT,
        name: "base.setParent",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: crate::parent::OP_CLEAR_PARENT,
        name: "base.clearParent",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
];

/// Forum's fixed op table: the shared chat mechanics plus its rooms, as the parts
/// facet (`base.setPart`/`clearPart` — owner/sequenced, the same edge every
/// kind's parts use). The chat band is duplicated rather than referenced
/// because a `static` cannot splice another static in a const initialiser (the
/// same reason group.rs duplicates membership's);
/// `chat_ops_are_spliced_verbatim` asserts the two stay identical, so the
/// duplication cannot drift silently.
static FORUM_OPS: &[OpDecl] = &[
    OpDecl {
        op_id: FORUM_POST,
        name: "forum.post",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: FORUM_REACT,
        name: "forum.react",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: FORUM_RECEIPT,
        name: "forum.receipt",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: FORUM_VOTE,
        name: "forum.vote",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    // A Forum's own, not the chat band's: the ICD's conversation does not declare it.
    OpDecl {
        op_id: FORUM_RETRACT,
        name: "forum.retract",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: FORUM_EDIT_DESCRIPTION,
        name: "forum.editDescription",
        authority: Authority::OwnerOrRole(crate::group::GroupRole::Admin),
        commutativity: Commutativity::Commutative,
    },
    // The base PARTS ops, spliced: a Forum's rooms are its parts.
    OpDecl {
        op_id: crate::parts::OP_SET_PART,
        name: "base.setPart",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: crate::parts::OP_CLEAR_PART,
        name: "base.clearPart",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    // The base PARENT ops, spliced. A Forum is the one kind that is routinely a
    // PART — every macro-node's comments section is one — so it is the first to
    // carry the half that makes `part_of` walkable from the child.
    OpDecl {
        op_id: crate::parent::OP_SET_PARENT,
        name: "base.setParent",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: crate::parent::OP_CLEAR_PARENT,
        name: "base.clearParent",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    // The base ROLES ops, spliced (D-58): a Site's room holds the admitter its founder
    // grants, as the Site does.
    OpDecl {
        op_id: crate::roles::OP_SET_ROLE,
        name: "base.setRole",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: crate::roles::OP_CLEAR_ROLE,
        name: "base.clearRole",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    // The base PROFILES op, spliced (O-77): a room's members read each other by name.
    OpDecl {
        op_id: crate::profiles::OP_PUBLISH_PROFILE,
        name: "base.publishProfile",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
];

/// The Forum object type — the concrete contract the generic coordinator folds.
pub struct ForumType;

impl ObjectType for ForumType {
    const KIND: ObjectKind = ObjectKind::Forum;
    type State = ForumState;

    fn ops() -> &'static [OpDecl] {
        FORUM_OPS
    }

    /// A standing in THIS room's own roles (`base.setRole` on the forum), which
    /// `Authority::OwnerOrRole` asks at write.
    fn role_of(state: &ForumState, member: &crate::object::MemberId) -> Option<crate::group::GroupRole> {
        state.roles.get(member).copied()
    }

    /// Pure reduce for `forum.post` (objects.py::ForumType.reduce). Only the one
    /// declared op exists; anything else is a loud `UnknownType`. Missing/ill-typed
    /// args self-reject as `MalformedArgs` — skipped by the fold, never swallowed
    /// silently (the loud membership/dedup checks already ran at deliver time).
    fn reduce(state: &mut ForumState, op: &Op<'_>) -> Result<(), DeltaRejection> {
        // Base ops first: a reserved id band, so this can never shadow a Forum op.
        if crate::parent::is_parent_op(op.op_id) {
            return crate::parent::reduce_parent(&mut state.parent, op);
        }
        if crate::parts::is_part_op(op.op_id) {
            return crate::parts::reduce_parts(&mut state.parts, op);
        }
        if crate::roles::is_role_op(op.op_id) {
            return crate::roles::reduce_roles(&mut state.roles, op);
        }
        if crate::profiles::is_profile_op(op.op_id) {
            return crate::profiles::reduce_profiles(&mut state.profiles, op);
        }
        match op.op_id {
            FORUM_POST => {
                let text = match crate::arg_reads::get(op.args, "text") {
                    Some(ArgVal::Text(t)) => t.clone(),
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                let gen = match crate::arg_reads::get(op.args, "gen") {
                    Some(ArgVal::Int(g)) => *g as u64,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                // ts + reply_to are OPTIONAL (older posts carry neither).
                let ts = match crate::arg_reads::get(op.args, "ts") {
                    Some(ArgVal::Int(t)) => *t as u64,
                    _ => 0,
                };
                let reply_to = arg_msgref(op.args, "reply_author", "reply_gen");
                // Media is optional, but a PRESENT-AND-BROKEN attachment is a
                // rejection rather than a silently dropped one: folding it away
                // would leave two devices holding different messages for the same
                // delta. Enforced at THIS fold, so a patched peer cannot post an
                // attachment past the Message budget into everyone's store.
                let media = match MediaRef::from_args_by("media", |k| crate::arg_reads::get(op.args, k)) {
                    None => None,
                    Some(Err(_)) => return Err(DeltaRejection::MalformedArgs),
                    Some(Ok(m)) if m.is_empty() => None,
                    Some(Ok(m)) => {
                        // Bounds, not container policing — same rule as every
                        // other fold: a peer's build may attach something we do
                        // not emit, and refusing it would drop their message.
                        if m.validate_bounds(Slot::Message).is_err() {
                            return Err(DeltaRejection::MalformedArgs);
                        }
                        Some(m)
                    }
                };
                if !state.retracted.contains(&(*op.author, gen)) {
                    state.messages.insert((*op.author, gen), Msg { text, media, ts, reply_to });
                }
                Ok(())
            }
            FORUM_RETRACT => {
                let target = arg_msgref(op.args, "target_author", "target_gen")
                    .ok_or(DeltaRejection::MalformedArgs)?;
                if !matches!(crate::arg_reads::get(op.args, "gen"), Some(ArgVal::Int(_))) {
                    return Err(DeltaRejection::MalformedArgs);
                }
                if target.0 != *op.author && !op.ctx.is_owner(op.author) {
                    return Err(DeltaRejection::Unauthorized);
                }
                state.messages.remove(&target);
                state.retracted.insert(target);
                Ok(())
            }
            FORUM_EDIT_DESCRIPTION => {
                // ONE REGISTER, LWW by (gen, author). It counts from the owner, or from a
                // member on the roster holding admin in THIS room's roles; the roles are the
                // folded ones (the spine folds first), so a revoked admin's write is refused
                // here, inert in the view, and the latest write that counts stands.
                let gen = match crate::arg_reads::get(op.args, "gen") {
                    Some(ArgVal::Int(g)) if *g >= 0 => *g as u64,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                let text = match crate::arg_reads::get(op.args, "description") {
                    Some(ArgVal::Text(t)) => t,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                // Refused, never truncated.
                if text.len() > FORUM_DESCRIPTION_MAX {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let author = op.author;
                let admin = op.ctx.is_member(author) && state.roles.get(author) == Some(&crate::group::GroupRole::Admin);
                if *author != op.ctx.owner && !admin {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                if state.description.as_ref().is_none_or(|d| (gen, *author) >= (d.gen, d.author)) {
                    state.description = Some(Description { text: text.clone(), gen, author: *author });
                }
                Ok(())
            }
            FORUM_REACT => {
                // target message = (target_author, target_gen); LWW per reactor by
                // this react's own gen (fold applies in (gen, author, id) order, so
                // a later react overwrites). active:0 (or empty emoji) clears.
                let target = arg_msgref(op.args, "target_author", "target_gen")
                    .ok_or(DeltaRejection::MalformedArgs)?;
                let gen = match crate::arg_reads::get(op.args, "gen") {
                    Some(ArgVal::Int(g)) => *g as u64,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                let active = matches!(crate::arg_reads::get(op.args, "active"), Some(ArgVal::Int(1)));
                let emoji = match crate::arg_reads::get(op.args, "emoji") {
                    Some(ArgVal::Text(e)) if active => e.clone(),
                    _ => String::new(), // cleared reaction
                };
                state
                    .reactions
                    .entry(target)
                    .or_default()
                    .insert(*op.author, (emoji, gen));
                Ok(())
            }
            FORUM_VOTE => {
                // target message = (target_author, target_gen); LWW per voter by
                // this vote's own gen — react's fold exactly. dir outside
                // {-1, 0, +1} is malformed and drops whole; 0 clears.
                let target = arg_msgref(op.args, "target_author", "target_gen")
                    .ok_or(DeltaRejection::MalformedArgs)?;
                let gen = match crate::arg_reads::get(op.args, "gen") {
                    Some(ArgVal::Int(g)) => *g as u64,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                let dir = match crate::arg_reads::get(op.args, "dir") {
                    Some(ArgVal::Int(d)) if (-1..=1).contains(d) => *d as i8,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                state
                    .votes
                    .entry(target)
                    .or_default()
                    .insert(*op.author, (dir, gen));
                Ok(())
            }
            FORUM_RECEIPT => {
                // One receipt acknowledges a BATCH of message refs at a single
                // `status`; the receiptor is this op's author. Grow-only lattice per
                // (target, receiptor): keep the MAX status, so a read (2) can never be
                // overwritten by a later-folded delivered (1) and the fold is
                // order-independent — no gen tiebreak needed.
                let status = match crate::arg_reads::get(op.args, "status") {
                    Some(ArgVal::Int(s))
                        if *s == RECEIPT_DELIVERED as i64 || *s == RECEIPT_READ as i64 =>
                    {
                        *s as u8
                    }
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                let refs = match crate::arg_reads::get(op.args, "refs") {
                    Some(ArgVal::Text(t)) => decode_msgref_batch(t),
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                for target in refs {
                    // A receipt on a message you sent yourself is meaningless — skip
                    // it so a buggy/hostile peer can't inflate its own ticks.
                    if target.0 == *op.author {
                        continue;
                    }
                    let slot = state
                        .receipts
                        .entry(target)
                        .or_default()
                        .entry(*op.author)
                        .or_insert(0);
                    *slot = (*slot).max(status);
                }
                Ok(())
            }
            _ => Err(DeltaRejection::UnknownType),
        }
    }
}

/// A Conversation (#26) — the Chats-tab object (DMs + group chats). It SHARES the
/// message mechanics with Forum: same `ForumState`, same `forum.post/react/configure`
/// reducer. It is a distinct kind purely so the cross-cutting concerns diverge —
/// prekeys/pairing live on Contact, and Forum stays the Project-linked reddit-thread
/// analogue. Its table is the chat band and the parent ops ([`CONVERSATION_OPS`]):
/// the parts ops are Forum's, so a DM log rejects `base.setPart` as `UnknownType` —
/// a thread has no tabs.
pub struct ConversationType;

impl ObjectType for ConversationType {
    const KIND: ObjectKind = ObjectKind::Conversation;
    type State = ForumState;

    fn ops() -> &'static [OpDecl] {
        CONVERSATION_OPS
    }

    fn reduce(state: &mut ForumState, op: &Op<'_>) -> Result<(), DeltaRejection> {
        ForumType::reduce(state, op)
    }
}

// --- RATIFY: types, fold projection, and delta builders --------------------
//
// The base governance layer, folded by `Coordinator<T>` ITSELF (not `T::reduce`),
// so no ObjectType learns about voting — ratify is orthogonal to the domain.

/// A member's standing in an object — the INPUT roles give to ratification. Ordered
/// by authority so eligibility/close checks are plain comparisons. In the base it is
/// derived from ownership + membership (owner → Owner, any other member → Member);
/// a `setMemberRole` overlay is layered by the node where a type carries one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RatifyRole {
    Guest,
    Viewer,
    Member,
    Admin,
    Owner,
}

/// One ballot on a proposal (LWW per voter). `Abstain` is a RECORDED non-vote —
/// distinct from silence (never voted).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ballot {
    Approve,
    Reject,
    Abstain,
}
impl Ballot {
    fn parse(n: i64) -> Option<Self> {
        match n {
            1 => Some(Self::Approve),
            2 => Some(Self::Reject),
            3 => Some(Self::Abstain),
            _ => None,
        }
    }
    pub fn code(self) -> i64 {
        match self {
            Self::Approve => 1,
            Self::Reject => 2,
            Self::Abstain => 3,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Reject => "reject",
            Self::Abstain => "abstain",
        }
    }
}

/// The passing rule carried on `propose`, evaluated against the ROLE-ELIGIBLE
/// electorate (role ≥ Member) frozen at close.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// Passes iff no eligible voter rejected (approve/abstain/silence all fine).
    Consent,
    /// Passes iff the owner approved.
    OwnerApproval,
}
impl Rule {
    fn parse(n: i64) -> Option<Self> {
        match n {
            0 => Some(Self::Consent),
            1 => Some(Self::OwnerApproval),
            _ => None,
        }
    }
    pub fn code(self) -> i64 {
        match self {
            Self::Consent => 0,
            Self::OwnerApproval => 1,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Consent => "consent",
            Self::OwnerApproval => "owner",
        }
    }
}

/// A proposal's DERIVED state — never stored; a pure function of (frozen tally × rule).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Pending,
    Passed,
    Failed,
}
impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Passed => "passed",
            Self::Failed => "failed",
        }
    }
}

/// A proposal id: the (proposer, gen) of its `propose` delta.
pub type ProposalId = MsgRef;

/// One pending/committed proposal.
#[derive(Clone, Debug)]
pub struct RProposal {
    pub proposer: MemberId,
    pub gen: u64,
    /// The deferred inner content. In this slice a ratified FACT (a decision text);
    /// generalises to an opaque inner delta the passed close activates.
    pub payload: String,
    pub rule: Rule,
}

/// The roster + tally FROZEN at close, so the outcome is stable as membership/roles
/// later change (membership-through-mls.md §10.4).
#[derive(Clone, Debug)]
pub struct RClose {
    /// The electorate: the close's own list when frozen, else the current roster.
    pub members: Vec<MemberId>,
    /// Who voted what, as the close counts it.
    pub votes: BTreeMap<MemberId, Ballot>,
    /// The owner who closed it — the owner at the close's epoch. An owner-approval
    /// proposal is judged by THIS owner, never by whoever owns the object later.
    pub owner: MemberId,
    /// True when the close named its electorate and ballots, so no later roster
    /// change can re-decide it. False for a legacy close, which still re-reads the
    /// current roster.
    pub frozen: bool,
    /// Ballots the close names that this device does not hold yet. While > 0 the
    /// outcome is `Pending`: the ballots replicate, and the outcome waits for them.
    pub awaiting: usize,
}

/// The folded ratify projection over an object's log.
#[derive(Clone, Debug, Default)]
pub struct RatifyState {
    pub proposals: BTreeMap<ProposalId, RProposal>,
    /// The OPEN tally: current members' latest ballots.
    pub votes: BTreeMap<ProposalId, BTreeMap<MemberId, Ballot>>,
    pub closed: BTreeMap<ProposalId, RClose>,
    /// Ballots this device holds on a FROZEN-closed proposal that its close does not
    /// count — the owner decides what arrived before the close, and this is what makes
    /// an omission visible to every member who holds the ballot (§10.4).
    pub unlisted: Vec<(ProposalId, [u8; 32])>,
}

impl RatifyState {
    /// A member's base role: owner → Owner; any other roster member → Member.
    /// (`setMemberRole` overlays are layered by the node before the outcome is read.)
    pub fn base_role(member: &MemberId, owner: &MemberId) -> RatifyRole {
        if member == owner {
            RatifyRole::Owner
        } else {
            RatifyRole::Member
        }
    }

    /// The ballots that DECIDE `pid`: the close's own once it is closed — a frozen close
    /// is never recounted over a later roster, and neither are its counts — else the
    /// open tally.
    pub fn tally(&self, pid: &ProposalId) -> Option<&BTreeMap<MemberId, Ballot>> {
        self.closed
            .get(pid)
            .map(|c| &c.votes)
            .or_else(|| self.votes.get(pid))
    }

    /// The DERIVED outcome of `pid` for an object owned by `owner`. `Pending` until a
    /// close has frozen the tally.
    pub fn outcome(&self, pid: &ProposalId, owner: &MemberId) -> Outcome {
        let (Some(prop), Some(frozen)) = (self.proposals.get(pid), self.closed.get(pid)) else {
            return Outcome::Pending;
        };
        if frozen.awaiting > 0 {
            return Outcome::Pending;
        }
        // The owner who CLOSED it decides owner-approval, not the caller's owner.
        let _ = owner;
        let owner = &frozen.owner;
        match prop.rule {
            Rule::Consent => {
                // eligible electorate = frozen members with role ≥ Member; pass iff
                // none of them rejected.
                let any_reject = frozen
                    .members
                    .iter()
                    .filter(|m| Self::base_role(m, owner) >= RatifyRole::Member)
                    .any(|m| frozen.votes.get(m) == Some(&Ballot::Reject));
                if any_reject {
                    Outcome::Failed
                } else {
                    Outcome::Passed
                }
            }
            Rule::OwnerApproval => match frozen.votes.get(owner) {
                Some(Ballot::Approve) => Outcome::Passed,
                _ => Outcome::Failed,
            },
        }
    }

    /// The payloads of every PASSED proposal, in causal (gen, author) order — the
    /// committed ratified facts.
    pub fn ratified(&self, owner: &MemberId) -> Vec<String> {
        let mut out: Vec<(ProposalId, String)> = self
            .proposals
            .iter()
            .filter(|(pid, _)| self.outcome(pid, owner) == Outcome::Passed)
            .map(|(pid, p)| (*pid, p.payload.clone()))
            .collect();
        out.sort_by_key(|(pid, _)| (pid.1, pid.0));
        out.into_iter().map(|(_, s)| s).collect()
    }
}

/// Build a `ratify.propose` delta on an object of `type_id`. Carries the deferred
/// `payload` + the passing `rule`; `gen` is the proposer's Lamport stamp (also the
/// proposal id's gen half). Commutative.
pub fn ratify_propose(payload: &str, rule: Rule, gen: u64, epoch: u64, type_id: u32) -> Delta {
    let mut args = Args::new();
    args.insert("payload".into(), ArgVal::Text(payload.to_string()));
    args.insert("rule".into(), ArgVal::Int(rule.code()));
    args.insert("gen".into(), ArgVal::Int(gen as i64));
    Delta {
        type_id,
        op_id: RATIFY_PROPOSE,
        op_version: 1,
        args,
        epoch,
        prev: GENESIS_PREV,
        seq: None,
        gen: Some(gen),
        // Authored PRIVATE unless the author said otherwise — the safe default,
        // and the one that costs nothing on the wire.
        visibility: crate::visibility::Visibility::default(),
    }
}

/// Build a `ratify.vote` delta: a `ballot` on `target` (a propose's (author, gen)).
/// LWW per (proposal, voter) by this vote's own `gen`. Commutative.
pub fn ratify_vote(
    target: ProposalId,
    ballot: Ballot,
    gen: u64,
    epoch: u64,
    type_id: u32,
) -> Delta {
    let mut args = Args::new();
    args.insert("target_author".into(), ArgVal::Text(hex::encode(target.0)));
    args.insert("target_gen".into(), ArgVal::Int(target.1 as i64));
    args.insert("ballot".into(), ArgVal::Int(ballot.code()));
    args.insert("gen".into(), ArgVal::Int(gen as i64));
    Delta {
        type_id,
        op_id: RATIFY_VOTE,
        op_version: 1,
        args,
        epoch,
        prev: GENESIS_PREV,
        seq: None,
        gen: Some(gen),
        // Authored PRIVATE unless the author said otherwise — the safe default,
        // and the one that costs nothing on the wire.
        visibility: crate::visibility::Visibility::default(),
    }
}

/// Build a FROZEN `ratify.close` (membership-through-mls.md §10.4): it names the
/// electorate it closed under and the exact ballots it counted, so its outcome is a
/// function of the close alone and no later roster change or handover can re-decide it.
/// Every input is independently checkable: the close is signed by the owner, each
/// ballot and the proposal by their authors (`delta_sig`).
pub fn ratify_close_frozen(
    target: ProposalId,
    electorate: &[MemberId],
    ballots: &[[u8; 32]],
    epoch: u64,
    seq: u64,
    prev: [u8; 32],
    type_id: u32,
) -> Delta {
    let mut d = ratify_close(target, epoch, seq, prev, type_id);
    let hexes = |v: &[[u8; 32]]| {
        let mut h: Vec<String> = v.iter().map(hex::encode).collect();
        h.sort();
        serde_json::to_string(&h).unwrap_or_else(|_| "[]".into())
    };
    d.args.insert("electorate".into(), ArgVal::Text(hexes(electorate)));
    d.args.insert("ballots".into(), ArgVal::Text(hexes(ballots)));
    d
}

/// Build a `ratify.close` delta: freeze `target`'s tally. Owner-sequenced (the spine
/// enforces owner + `(epoch, seq)` succession + the `prev` hash-chain). WITHOUT an
/// electorate and ballot list this is a LEGACY close, which re-reads the current roster
/// on every fold — `object_close` writes [`ratify_close_frozen`] instead.
pub fn ratify_close(
    target: ProposalId,
    epoch: u64,
    seq: u64,
    prev: [u8; 32],
    type_id: u32,
) -> Delta {
    let mut args = Args::new();
    args.insert("target_author".into(), ArgVal::Text(hex::encode(target.0)));
    args.insert("target_gen".into(), ArgVal::Int(target.1 as i64));
    Delta {
        type_id,
        op_id: RATIFY_CLOSE,
        op_version: 1,
        args,
        epoch,
        prev,
        seq: Some(seq),
        gen: None,
        // Authored PRIVATE unless the author said otherwise — the safe default,
        // and the one that costs nothing on the wire.
        visibility: crate::visibility::Visibility::default(),
    }
}

// --- Coordinator<T>: the generic mixed-log fold engine ---------------------
//
// The keystone object.rs promised: one engine that folds ANY `T: ObjectType`.
// It routes a delivered log into the two arms by each op's DECLARED
// commutativity (looked up in `T::ops()`), enforces each arm's integrity at
// deliver time, then at read time folds the SEQUENCED spine first (the owner
// total order) and the COMMUTATIVE OR-set second (canonical (gen, author, id)
// order), calling `T::reduce` for each. Sequenced-before-commutative is what
// lets a commutative op precondition on structure the owner sequenced (a task's
// existence before its status), exactly as forum.post references an owner-set
// topic area.

pub struct Coordinator<T: ObjectType> {
    members: Vec<MemberId>,
    owner: MemberId,
    /// the owner-sequenced spine (validates + orders the sequenced arm).
    spine: SequencedCoordinator,
    /// (author, DeltaId) dedup for the commutative OR-set.
    seen: HashSet<(MemberId, [u8; 32])>,
    /// the accepted commutative deltas (folded in canonical order at read time).
    commutative: Vec<(Delta, MemberId)>,
    // `fn() -> T`: the marker never owns a T, so a coordinator is Send and Sync whatever
    // T is, and the fold cache (O-69) can hold it.
    _type: PhantomData<fn() -> T>,
}

// By hand, not derived: a derive would ask `T: Clone`, and `T` is only a marker here.
// The fold cache (O-69) hands out a copy of what it holds.
impl<T: ObjectType> Clone for Coordinator<T> {
    fn clone(&self) -> Self {
        Self {
            members: self.members.clone(),
            owner: self.owner,
            spine: self.spine.clone(),
            seen: self.seen.clone(),
            commutative: self.commutative.clone(),
            _type: PhantomData,
        }
    }
}

impl<T: ObjectType> Coordinator<T> {
    /// What this fold accepted, as one hash: the roster, the owner, the spine by position
    /// and id, and the commutative arm by author and id. Two folds of one object that accept
    /// the same are the same fold, since `reduce` is pure (the fold cache's check, FC-2).
    pub fn accepted_digest(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"pacific-fold-accepted:v1");
        let mut members = self.members.clone();
        members.sort_unstable();
        for m in &members {
            h.update(m);
        }
        h.update(self.owner);
        for (pos, id, _) in &self.spine.spine {
            h.update(pos.epoch.to_be_bytes());
            h.update(pos.seq.to_be_bytes());
            h.update(id);
        }
        let mut comm: Vec<([u8; 32], MemberId)> = self.commutative.iter().map(|(d, a)| (d.id(), *a)).collect();
        comm.sort_unstable();
        for (id, a) in &comm {
            h.update(id);
            h.update(a);
        }
        h.finalize().into()
    }

    /// A coordinator for an object that has had one owner for its whole life.
    pub fn new(members: Vec<MemberId>, owner: MemberId) -> Self {
        Self::with_owners(members, vec![(0, owner)])
    }

    /// A coordinator whose owner changes with the epoch (membership-through-mls.md
    /// §10.2). `members` is the CURRENT roster — what the reducers' target checks read
    /// (a role, an office, a publisher must be in the room NOW). It is not what decides
    /// whether an author may be folded: MLS decided that once, at ingest (§10.1).
    pub fn with_owners(members: Vec<MemberId>, owners: Vec<(u64, MemberId)>) -> Self {
        let spine = SequencedCoordinator::with_owners(owners);
        let owner = spine.current_owner().unwrap_or([0u8; 32]);
        Self {
            members,
            owner,
            spine,
            seen: HashSet::new(),
            commutative: Vec::new(),
            _type: PhantomData,
        }
    }

    /// Accept one delta, routed by its op's DECLARED fold rule. `Ok(true)` =
    /// newly accepted; `Ok(false)` = idempotent duplicate; `Err` = a LOUD
    /// rejection (unknown op, wrong type, non-member, non-owner spine write,
    /// broken chain / fork / stale epoch) — nothing is swallowed.
    pub fn deliver(&mut self, delta: Delta, author: MemberId) -> Result<bool, DeltaRejection> {
        // the delta must carry this type's id.
        if delta.type_id != T::KIND.type_id() as u32 {
            return Err(DeltaRejection::UnknownType);
        }
        // RATIFY is a BASE op-group present on EVERY object (a sibling to the
        // membership/role ops), recognised here — ahead of the type's own op table —
        // so no `ObjectType` declares it. propose/vote fold as the commutative OR-set;
        // close rides the owner-sequenced spine (the single serialization point). The
        // fold rule is READ FROM `RATIFY_OPS` rather than matched on here, so the table
        // the ICD is pinned against is the same one the wire obeys.
        let commutativity = if let Some(decl) = ratify_op(delta.op_id) {
            decl.commutativity
        } else {
            T::op(delta.op_id)
                .ok_or(DeltaRejection::UnknownType)?
                .commutativity
        };
        match commutativity {
            // the sequenced spine owns the owner-check + chain/fork/stale integrity.
            Commutativity::Sequenced => self.spine.deliver(delta, author),
            // commutative => any-member (spec invariant). WHETHER THE AUTHOR WAS A
            // MEMBER is not asked here, and must not be (membership-through-mls.md
            // §10.1). Every delta in a log arrived either as an MLS message from a leaf
            // MLS authenticated in the epoch it was encrypted under, or was authored by
            // this device about itself, or came from the person's own seed-sealed
            // archive with its authorship signature checked. MLS answered "was a member
            // when they wrote it" at ingest. Asking the CURRENT roster instead is the
            // wrong question — and once removal exists, it made every object a removed
            // member ever wrote in stop folding (`fold_entries` propagates this with
            // `?`), on every device. The old exemption for membership ops, whose
            // author is precisely the person leaving, is now simply the general rule.
            Commutativity::Commutative => {
                // a commutative delta MUST carry its (gen) LWW key.
                if delta.gen.is_none() {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let key = (author, delta.id());
                if self.seen.contains(&key) {
                    return Ok(false); // idempotent — this exact delta already held.
                }
                self.seen.insert(key);
                self.commutative.push((delta, author));
                Ok(true)
            }
        }
    }

    /// The compiled projection = fold the spine (in (epoch, seq) order) then the
    /// commutative OR-set (in canonical (gen, author, id) order) through
    /// `T::reduce`. A reduce-time rejection means the delta's precondition is
    /// unmet in the final state, so it is INERT and skipped — the loud integrity
    /// checks already happened at deliver time; this is the OR-set/LWW semantics,
    /// not a swallowed failure.
    pub fn state(&self) -> T::State {
        let epoch = self.spine.current_epoch();
        // `ctx.owner` is the owner AT THE DELTA'S EPOCH (§10.2), not today's. Every
        // reducer that reads it asks "was this written by the owner of the time": a
        // handover record, an owner's removal record, the seller a ticket listing names.
        // Judged by today's owner, a handover would silently void all three — including
        // every sale the previous owner recorded, which is the ledger rewritten.
        let owner_at = |e: u64| self.spine.owner_at(e).unwrap_or(self.owner);
        let mut st = T::State::default();

        // 1. the owner-sequenced spine, in total order. Each delta's author is the
        //    owner AT ITS EPOCH (the spine rejected anyone else at deliver time), so a
        //    delta written before a handover is still the old owner's (§10.2).
        for (pos, delta) in self.spine.spine() {
            let author = owner_at(pos.epoch);
            let ctx = ReduceContext {
                members: &self.members,
                owner: author,
                epoch,
            };
            let op = Op {
                op_id: delta.op_id,
                args: &delta.args,
                author: &author,
                pos: Some(*pos),
                ctx: &ctx,
            };
            let _ = T::reduce(&mut st, &op);
        }

        // 2. the commutative OR-set, folded in canonical (gen, author, id) order so
        //    state is a function of the delta SET, never arrival order. NOT
        //    re-filtered by the current roster (§10.1): a removed member's past words,
        //    reactions, note entries and wallet rows are history, and removing them
        //    would rewrite it — a treasurer leaving would change the balance.
        let mut comm: Vec<&(Delta, MemberId)> = self.commutative.iter().collect();
        // Each key once (O-69): id() hashes; the same key, stable, the same order.
        comm.sort_by_cached_key(|x| (x.0.gen.unwrap_or(0), x.1, x.0.id()));
        //    A commutative delta's epoch is the one its author stamped: MLS ingest does
        //    not bind it (an outbox delta is sealed epochs after it was written), so a
        //    FORMER owner can backdate a record to an epoch they owned. Records are
        //    claims — `MembershipLog::divergence` checks them against the tree (§17).
        for (delta, author) in comm {
            let ctx = ReduceContext {
                members: &self.members,
                owner: owner_at(delta.epoch),
                epoch,
            };
            let op = Op {
                op_id: delta.op_id,
                args: &delta.args,
                author,
                pos: None,
                ctx: &ctx,
            };
            let _ = T::reduce(&mut st, &op);
        }
        st
    }

    /// The folded RATIFY projection over the same log — proposals + ballots from the
    /// commutative OR-set, and the frozen tallies from the owner-sequenced closes.
    /// Orthogonal to `state()`: the ratify ops are inert in `T::reduce` (unknown to
    /// the domain), and the domain ops are ignored here. Roles enter at `outcome`
    /// time; this fold just records who voted what.
    pub fn ratify_state(&self) -> RatifyState {
        let mut st = RatifyState::default();

        // 1. commutative arm, canonical (gen, author, id) order — propose + vote. NOT
        //    filtered by the current roster (membership-through-mls.md §10.1): a
        //    proposal stays a proposal after its author leaves, and a close may name
        //    it. The OPEN tally (`st.votes`) still counts only current members — a
        //    departed member's vote on something still open stops counting, as
        //    `ratify.vote`'s summary says. A CLOSED tally is judged from the close.
        let mut comm: Vec<&(Delta, MemberId)> = self.commutative.iter().collect();
        // Each key once (O-69): id() hashes; the same key, stable, the same order.
        comm.sort_by_cached_key(|x| (x.0.gen.unwrap_or(0), x.1, x.0.id()));
        // Every ballot by its delta id — what a frozen close names (§10.4).
        let mut ballot_ids: BTreeMap<[u8; 32], (ProposalId, MemberId, Ballot, u64)> = BTreeMap::new();
        for (delta, author) in comm {
            let current = self.members.contains(author);
            match delta.op_id {
                RATIFY_PROPOSE => {
                    let (Some(ArgVal::Text(payload)), Some(ArgVal::Int(rule)), Some(gen)) =
                        (crate::arg_reads::get(&delta.args, "payload"), crate::arg_reads::get(&delta.args, "rule"), delta.gen)
                    else {
                        continue;
                    };
                    let Some(rule) = Rule::parse(*rule) else {
                        continue;
                    };
                    let pid = (*author, gen);
                    st.proposals.insert(
                        pid,
                        RProposal {
                            proposer: *author,
                            gen,
                            payload: payload.clone(),
                            rule,
                        },
                    );
                    // the proposer implicitly approves their own proposal (an explicit
                    // later ballot of theirs overwrites this).
                    if current {
                        st.votes
                            .entry(pid)
                            .or_default()
                            .entry(*author)
                            .or_insert(Ballot::Approve);
                    }
                }
                RATIFY_VOTE => {
                    let Some(target) = arg_msgref(&delta.args, "target_author", "target_gen")
                    else {
                        continue;
                    };
                    let Some(ArgVal::Int(b)) = crate::arg_reads::get(&delta.args, "ballot") else {
                        continue;
                    };
                    let Some(ballot) = Ballot::parse(*b) else {
                        continue;
                    };
                    ballot_ids.insert(delta.id(), (target, *author, ballot, delta.gen.unwrap_or(0)));
                    // LWW: comm is ascending (gen, author, id), so a later ballot wins.
                    if current {
                        st.votes.entry(target).or_default().insert(*author, ballot);
                    }
                }
                _ => {}
            }
        }

        // 2. sequenced arm — the owner-sequenced closes. The first close of a proposal
        //    is the one that counts; a close naming a proposal this device has not
        //    folded is inert (the propose may still be in flight).
        for (pos, delta) in self.spine.spine() {
            if delta.op_id != RATIFY_CLOSE {
                continue;
            }
            let Some(target) = arg_msgref(&delta.args, "target_author", "target_gen") else {
                continue;
            };
            if st.closed.contains_key(&target) {
                continue;
            }
            let Some(prop) = st.proposals.get(&target).cloned() else {
                continue;
            };
            // The close's author is the owner AT ITS EPOCH: an owner-approval proposal
            // is judged by the owner who closed it, not by whoever owns it later.
            let closer = self.spine.owner_at(pos.epoch).unwrap_or(self.owner);
            match (json_members(&delta.args, "electorate"), json_members(&delta.args, "ballots")) {
                // A FROZEN close (§10.4): judged from what it names, and nothing else.
                (Some(Ok(electorate)), Some(Ok(listed))) => {
                    let mut votes: BTreeMap<MemberId, Ballot> = BTreeMap::new();
                    if electorate.contains(&prop.proposer) {
                        votes.insert(prop.proposer, Ballot::Approve);
                    }
                    let mut best: BTreeMap<MemberId, (u64, Ballot)> = BTreeMap::new();
                    let mut awaiting = 0usize;
                    let mut inert = None;
                    for id in &listed {
                        match ballot_ids.get(id) {
                            // Listed but not yet held here: deltas replicate, so the
                            // outcome waits for it rather than guessing.
                            None => awaiting += 1,
                            Some((pid, voter, ballot, gen)) => {
                                if *pid != target {
                                    inert = Some("names a ballot on a different proposal");
                                    break;
                                }
                                if !electorate.contains(voter) {
                                    inert = Some("names a ballot from outside its electorate");
                                    break;
                                }
                                if best.get(voter).map_or(true, |(g, _)| gen >= g) {
                                    best.insert(*voter, (*gen, *ballot));
                                }
                            }
                        }
                    }
                    if let Some(why) = inert {
                        tracing::warn!(
                            target: "pacific::ratify",
                            proposer = %hex::encode(target.0),
                            gen = target.1,
                            reason = why,
                            "a ratify.close is inert"
                        );
                        continue;
                    }
                    for (voter, (_, b)) in best {
                        votes.insert(voter, b);
                    }
                    // Divergence: ballots on this proposal that the close does not
                    // count — an omitted reject is loud, never silent (§17 item 8).
                    for (id, (pid, _, _, _)) in &ballot_ids {
                        if *pid == target && !listed.contains(id) {
                            st.unlisted.push((target, *id));
                        }
                    }
                    st.closed.insert(
                        target,
                        RClose { members: electorate, votes, owner: closer, frozen: true, awaiting },
                    );
                }
                // A LEGACY close (no electorate/ballots): today's derivation, reported
                // as unfrozen — it re-reads the current roster, which is exactly the
                // defect §10.4 exists to end.
                (None, None) => {
                    let votes = st.votes.get(&target).cloned().unwrap_or_default();
                    st.closed.insert(
                        target,
                        RClose {
                            members: self.members.clone(),
                            votes,
                            owner: closer,
                            frozen: false,
                            awaiting: 0,
                        },
                    );
                }
                _ => {
                    tracing::warn!(
                        target: "pacific::ratify",
                        proposer = %hex::encode(target.0),
                        gen = target.1,
                        "a ratify.close carries a malformed electorate or ballot list — inert"
                    );
                }
            }
        }
        st
    }

    /// The ballots a close by this device should COUNT on `target`: for each CURRENT
    /// member, the delta id of their latest ballot (LWW by the canonical order). What
    /// `object_close` writes into `ratify.close`'s `ballots` (§10.4).
    pub fn ratify_ballots_for(&self, target: &ProposalId) -> Vec<[u8; 32]> {
        let mut comm: Vec<&(Delta, MemberId)> = self
            .commutative
            .iter()
            .filter(|(d, a)| d.op_id == RATIFY_VOTE && self.members.contains(a))
            .collect();
        // Each key once (O-69): id() hashes; the same key, stable, the same order.
        comm.sort_by_cached_key(|x| (x.0.gen.unwrap_or(0), x.1, x.0.id()));
        let mut latest: BTreeMap<MemberId, [u8; 32]> = BTreeMap::new();
        for (d, a) in comm {
            if arg_msgref(&d.args, "target_author", "target_gen").as_ref() == Some(target) {
                latest.insert(*a, d.id());
            }
        }
        latest.into_values().collect()
    }

    /// The current roster this coordinator folds against (the electorate a close
    /// freezes).
    pub fn members(&self) -> &[MemberId] {
        &self.members
    }

    /// The owner at `epoch` (§10.2).
    pub fn owner_at(&self, epoch: u64) -> MemberId {
        self.spine.owner_at(epoch).unwrap_or(self.owner)
    }

    /// The object's owner — who the ratify `outcome`/role derivation keys on.
    pub fn owner(&self) -> MemberId {
        self.owner
    }

    /// The head of the sequenced spine (its total-order position + DeltaId), or
    /// `None` if no sequenced delta has been folded — the anchor an author stamps
    /// the next sequenced delta onto (see [`next_sequenced_pos`]).
    pub fn sequenced_head(&self) -> Option<(LogPosition, [u8; 32])> {
        self.spine.head()
    }
}

/// Build a `forum.post` Delta (the M1 single-write-path payload). `ts` is unix
/// millis for DISPLAY (0 to omit); `reply_to` is the (author, gen) of the message
/// this replies to (None for a top-level post).
impl Delta {
    /// Stamp this delta with the wire type of the object it is going into.
    ///
    /// THE ONE SEAM OF THE FORK (24 September 2026). The chat builders below
    /// construct a Forum delta because that is what a forum takes; a conversation
    /// takes the same shape under type 26. The fold filters on exactly this, so
    /// the id is set where the OBJECT is known — `Node::chat_type_id` — and not
    /// guessed by the builder, which has no object in hand.
    pub(crate) fn retyped(mut self, type_id: u32) -> Self {
        self.type_id = type_id;
        self
    }
}

pub fn forum_post(text: &str, gen: u64, epoch: u64) -> Delta {
    forum_post_full(text, gen, epoch, 0, None, None)
}

pub fn forum_post_full(
    text: &str,
    gen: u64,
    epoch: u64,
    ts: u64,
    reply_to: Option<MsgRef>,
    media: Option<&MediaRef>,
) -> Delta {
    let mut args = Args::new();
    args.insert("text".into(), ArgVal::Text(text.to_string()));
    // Flattened under the `media` prefix by the media module itself, so the wire
    // shape has exactly one definition. An absent attachment writes no keys at
    // all — a text message costs precisely what it did before.
    if let Some(m) = media {
        m.to_args("media", &mut args);
    }
    args.insert("gen".into(), ArgVal::Int(gen as i64));
    if ts != 0 {
        args.insert("ts".into(), ArgVal::Int(ts as i64));
    }
    if let Some((ra, rg)) = reply_to {
        args.insert("reply_author".into(), ArgVal::Text(hex::encode(ra)));
        args.insert("reply_gen".into(), ArgVal::Int(rg as i64));
    }
    Delta {
        type_id: FORUM_TYPE_ID,
        op_id: FORUM_POST,
        op_version: 1,
        args,
        epoch,
        prev: GENESIS_PREV,
        seq: None,
        gen: Some(gen),
        // Authored PRIVATE unless the author said otherwise — the safe default,
        // and the one that costs nothing on the wire.
        visibility: crate::visibility::Visibility::default(),
    }
}

/// Build a `forum.react` Delta: react to `target` with `emoji` (or clear it with
/// `active = false`). `gen` is the reactor's Lamport stamp (LWW tiebreak).
pub fn forum_react(target: MsgRef, emoji: &str, active: bool, gen: u64, epoch: u64) -> Delta {
    let mut args = Args::new();
    args.insert("target_author".into(), ArgVal::Text(hex::encode(target.0)));
    args.insert("target_gen".into(), ArgVal::Int(target.1 as i64));
    args.insert("emoji".into(), ArgVal::Text(emoji.to_string()));
    args.insert("active".into(), ArgVal::Int(if active { 1 } else { 0 }));
    args.insert("gen".into(), ArgVal::Int(gen as i64));
    Delta {
        type_id: FORUM_TYPE_ID,
        op_id: FORUM_REACT,
        op_version: 1,
        args,
        epoch,
        prev: GENESIS_PREV,
        seq: None,
        gen: Some(gen),
        // Authored PRIVATE unless the author said otherwise — the safe default,
        // and the one that costs nothing on the wire.
        visibility: crate::visibility::Visibility::default(),
    }
}

/// Build a `forum.vote` Delta: up/downvote `target` (`dir` +1 | −1) or clear
/// your vote (`dir` 0). `gen` is the voter's Lamport stamp (LWW tiebreak).
pub fn forum_vote(target: MsgRef, dir: i8, gen: u64, epoch: u64) -> Delta {
    let mut args = Args::new();
    args.insert("target_author".into(), ArgVal::Text(hex::encode(target.0)));
    args.insert("target_gen".into(), ArgVal::Int(target.1 as i64));
    args.insert("dir".into(), ArgVal::Int(dir as i64));
    args.insert("gen".into(), ArgVal::Int(gen as i64));
    Delta {
        type_id: FORUM_TYPE_ID,
        op_id: FORUM_VOTE,
        op_version: 1,
        args,
        epoch,
        prev: GENESIS_PREV,
        seq: None,
        gen: Some(gen),
        // Authored PRIVATE unless the author said otherwise — the safe default,
        // and the one that costs nothing on the wire.
        visibility: crate::visibility::Visibility::default(),
    }
}

/// Build a batched `forum.receipt` Delta: acknowledge EVERY ref in `targets` at
/// `status` ([`RECEIPT_DELIVERED`] or [`RECEIPT_READ`]) in ONE delta — Signal's
/// `ReceiptMessage { type, repeated timestamps }`, so a burst of N messages costs a
/// single receipt, not N. `gen` is the receiptor's Lamport stamp, carried because
/// every commutative delta must (the OR-set dedup keys on it); the receipt lattice
/// itself folds by max status, not gen.
pub fn forum_receipt_batch(targets: &[MsgRef], status: u8, gen: u64, epoch: u64) -> Delta {
    let mut args = Args::new();
    args.insert("status".into(), ArgVal::Int(status as i64));
    args.insert("refs".into(), ArgVal::Text(encode_msgref_batch(targets)));
    args.insert("gen".into(), ArgVal::Int(gen as i64));
    Delta {
        type_id: FORUM_TYPE_ID,
        op_id: FORUM_RECEIPT,
        op_version: 1,
        args,
        epoch,
        prev: GENESIS_PREV,
        seq: None,
        gen: Some(gen),
        // Authored PRIVATE unless the author said otherwise — the safe default,
        // and the one that costs nothing on the wire.
        visibility: crate::visibility::Visibility::default(),
    }
}

/// Single-ref convenience over [`forum_receipt_batch`] — a 1:1 ack (and the unit
/// tests' shorthand).
pub fn forum_receipt(target: MsgRef, status: u8, gen: u64, epoch: u64) -> Delta {
    forum_receipt_batch(&[target], status, gen, epoch)
}

/// Encode a batch of message refs into one deterministic arg string —
/// `"<author_hex>:<gen>"` per ref, SORTED + deduped, comma-joined. Sorting makes
/// the same acknowledged SET produce byte-identical canonical bytes across
/// replicas (the delta id is content-addressed, so order would otherwise fork it).
fn encode_msgref_batch(targets: &[MsgRef]) -> String {
    let mut v: Vec<MsgRef> = targets.to_vec();
    v.sort();
    v.dedup();
    v.iter()
        .map(|(a, g)| format!("{}:{}", hex::encode(a), g))
        .collect::<Vec<_>>()
        .join(",")
}

/// Parse [`encode_msgref_batch`]'s output back into refs. A malformed entry is
/// skipped individually — never a panic, never fails the whole batch (mirrors
/// `arg_msgref`, which treats a bad ref as absent).
fn decode_msgref_batch(s: &str) -> Vec<MsgRef> {
    if s.is_empty() {
        return Vec::new();
    }
    s.split(',')
        .filter_map(|item| {
            let (h, g) = item.split_once(':')?;
            let arr: [u8; 32] = hex::decode(h).ok()?.try_into().ok()?;
            Some((arr, g.parse::<u64>().ok()?))
        })
        .collect()
}

/// Parse an optional (author_hex, gen) message ref out of two args. Returns None
/// if either is absent or the hex is not 32 bytes (a malformed ref is treated as
/// absent, never a panic).
/// A text arg holding a JSON array of 32-byte hex ids — `ratify.close`'s `electorate`
/// and `ballots` (§10.4). `None` when absent; `Some(Err)` when present but malformed, which
/// makes the close inert rather than guessed at. The envelope carries only `Int | Text`,
/// so a list rides as JSON in one text arg, as `card` and `setLocation.source` do.
fn json_members(args: &Args, key: &str) -> Option<Result<Vec<[u8; 32]>, ()>> {
    let ArgVal::Text(t) = crate::arg_reads::get(args, key)? else {
        return Some(Err(()));
    };
    let parsed: Result<Vec<String>, _> = serde_json::from_str(t);
    Some(parsed.map_err(|_| ()).and_then(|v| {
        v.iter()
            .map(|h| {
                hex::decode(h)
                    .ok()
                    .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
                    .ok_or(())
            })
            .collect()
    }))
}

fn arg_msgref(args: &Args, author_key: &str, gen_key: &str) -> Option<MsgRef> {
    let author_hex = match crate::arg_reads::get(args, author_key) {
        Some(ArgVal::Text(h)) => h,
        _ => return None,
    };
    let gen = match crate::arg_reads::get(args, gen_key) {
        Some(ArgVal::Int(g)) => *g as u64,
        _ => return None,
    };
    let bytes = hex::decode(author_hex).ok()?;
    let arr: [u8; 32] = bytes.try_into().ok()?;
    Some((arr, gen))
}

// --- SequencedCoordinator: the owner-sequenced spine ------------------------
//
// The OTHER fold arm (coordinator.py, sequenced branch). Where the commutative
// OR-set arm TOLERATES any arrival order, the spine ENFORCES an
// owner-stamped total order on (epoch, seq), hash-chained by `prev`. This is what
// unlocks every owner-sequenced op (configure, channels, roles, membership
// governance) — none of which can fold until the spine below exists. It validates
// + orders the spine; a concrete `ObjectType` reduces the ordered spine.
//
// Until now `DeltaRejection::{ChainBroken, ForkDetected, StaleEpoch}` were declared
// but NEVER constructed anywhere in the crate. This is their first (and only)
// producer — the sequenced integrity checks made real.

/// Build a sequenced Delta at `(epoch, seq)`, hash-chained to `prev` (the DeltaId
/// of its predecessor, or `GENESIS_PREV` for the first). `gen` is None on this arm.
pub fn sequenced_delta(
    type_id: u32,
    op_id: u32,
    args: Args,
    epoch: u64,
    seq: u64,
    prev: [u8; 32],
) -> Delta {
    Delta {
        type_id,
        op_id,
        op_version: 1,
        args,
        epoch,
        prev,
        seq: Some(seq),
        gen: None,
        // Authored PRIVATE unless the author said otherwise — the safe default,
        // and the one that costs nothing on the wire.
        visibility: crate::visibility::Visibility::default(),
    }
}

/// The `(seq, prev)` the NEXT sequenced delta must carry, given the spine head and
/// the object's current MLS epoch. Pure — the author stamps a delta with this:
/// - empty spine  -> `(0, GENESIS_PREV)` (genesis at the current epoch),
/// - same epoch   -> `(head.seq + 1, head.id)`,
/// - epoch bumped -> `(0, head.id)` (seq resets each epoch, still hash-chained).
pub fn next_sequenced_pos(head: Option<(LogPosition, [u8; 32])>, epoch: u64) -> (u64, [u8; 32]) {
    match head {
        None => (0, GENESIS_PREV),
        Some((pos, id)) => {
            if epoch == pos.epoch {
                (pos.seq + 1, id)
            } else {
                (0, id)
            }
        }
    }
}

/// One replica's view of an object's SEQUENCED spine: the owner-stamped, hash-
/// chained total order over `(epoch, seq)`. Order is AUTHORED (by the owner), not
/// derived — so integrity is enforced, not tolerated.
#[derive(Clone)]
pub struct SequencedCoordinator {
    /// WHO OWNED THE OBJECT AT EACH EPOCH, ascending by the epoch they took over
    /// (membership-through-mls.md §10.2). The owner lives in the MLS GroupContext and
    /// moves only by a handover commit, so it is a function of the epoch: a sequenced
    /// delta at epoch e is accepted only from `owner_at(e)`. Before this the spine had
    /// ONE owner for life, so a handover could never take effect.
    owners: Vec<(u64, [u8; 32])>,
    /// accepted deltas in `(epoch, seq)` order — the spine.
    spine: Vec<(LogPosition, [u8; 32], Delta)>,
    /// `(epoch, seq)` -> accepted DeltaId, the fork-detection index.
    by_pos: BTreeMap<(u64, u64), [u8; 32]>,
}

impl SequencedCoordinator {
    /// A spine with one owner for every epoch — every object that has never changed
    /// hands.
    pub fn new(owner: [u8; 32]) -> Self {
        Self::with_owners(vec![(0, owner)])
    }

    /// A spine whose owner changes with the epoch. `owners` is `(from_epoch, owner)`;
    /// order does not matter, and an empty list means nobody may write the spine.
    pub fn with_owners(mut owners: Vec<(u64, [u8; 32])>) -> Self {
        owners.sort_by_key(|(e, _)| *e);
        owners.dedup_by_key(|(e, _)| *e);
        Self {
            owners,
            spine: Vec::new(),
            by_pos: BTreeMap::new(),
        }
    }

    /// The owner at `epoch`: the last owner who took over at or before it. A delta
    /// older than the first recorded row (a replica that learned the history from its
    /// join onward) is judged against the first owner it knows.
    pub fn owner_at(&self, epoch: u64) -> Option<[u8; 32]> {
        self.owners
            .iter()
            .rev()
            .find(|(e, _)| *e <= epoch)
            .or_else(|| self.owners.first())
            .map(|(_, o)| *o)
    }

    /// The owner NOW — the last one to take over.
    pub fn current_owner(&self) -> Option<[u8; 32]> {
        self.owners.last().map(|(_, o)| *o)
    }

    /// The head (last accepted) position + id, or None on an empty spine.
    fn head(&self) -> Option<(LogPosition, [u8; 32])> {
        self.spine.last().map(|(p, id, _)| (*p, *id))
    }

    /// Deliver a sequenced Delta. `Ok(true)` = newly accepted; `Ok(false)` =
    /// idempotent duplicate; `Err(DeltaRejection)` = a LOUD spine violation
    /// (nothing is ever swallowed).
    pub fn deliver(&mut self, delta: Delta, author: [u8; 32]) -> Result<bool, DeltaRejection> {
        // sequenced deltas MUST carry a seq (gen is the commutative key, not this arm).
        let seq = delta.seq.ok_or(DeltaRejection::MalformedArgs)?;
        let pos = LogPosition {
            epoch: delta.epoch,
            seq,
        };
        let id = delta.id();

        // only the owner sequences the spine — the owner AT THIS DELTA'S EPOCH (§10.2).
        if Some(author) != self.owner_at(pos.epoch) {
            return Err(DeltaRejection::Unauthorized);
        }

        // fork / duplicate: is this exact position already taken?
        if let Some(existing) = self.by_pos.get(&(pos.epoch, pos.seq)) {
            return if *existing == id {
                Ok(false) // same delta re-delivered — idempotent no-op
            } else {
                Err(DeltaRejection::ForkDetected) // two truths at one slot — halt
            };
        }

        match self.head() {
            // extending an existing spine.
            Some((head_pos, head_id)) => {
                // a delta for an already-fenced (past) epoch is stale.
                if pos.epoch < head_pos.epoch {
                    return Err(DeltaRejection::StaleEpoch);
                }
                // the hash chain must link to the current head.
                if delta.prev != head_id {
                    return Err(DeltaRejection::ChainBroken);
                }
                // the position must be the STRICT successor: seq+1 within an epoch,
                // reset to 0 when the epoch advances. A gap breaks the chain.
                let expected = if pos.epoch == head_pos.epoch {
                    LogPosition {
                        epoch: head_pos.epoch,
                        seq: head_pos.seq + 1,
                    }
                } else {
                    LogPosition {
                        epoch: pos.epoch,
                        seq: 0,
                    }
                };
                if pos != expected {
                    return Err(DeltaRejection::ChainBroken);
                }
            }
            // genesis OR a RESTATED HEAD. A replica that joined late can never hold
            // the chain's prefix (MLS forward secrecy — pre-join epochs derive no
            // keys), so when the owner RESTATES current state at the new epoch
            // (`Node::event_reemit`), that restatement ANCHORS this replica's spine:
            // integrity is enforced from the first delta a replica holds, and MLS
            // authorship is the root of trust for the prefix it cannot see. A true
            // genesis chains to GENESIS_PREV; either way the anchor must open its
            // epoch at seq 0 — mid-epoch starts stay ChainBroken.
            None => {
                if seq != 0 {
                    return Err(DeltaRejection::ChainBroken);
                }
            }
        }

        self.by_pos.insert((pos.epoch, pos.seq), id);
        self.spine.push((pos, id, delta));
        Ok(true)
    }

    /// The accepted spine in total order — a concrete `ObjectType` folds this into
    /// typed state (the sequenced analogue of the commutative OR-set fold).
    pub fn spine(&self) -> impl Iterator<Item = (&LogPosition, &Delta)> {
        self.spine.iter().map(|(p, _, d)| (p, d))
    }

    pub fn len(&self) -> usize {
        self.spine.len()
    }
    pub fn is_empty(&self) -> bool {
        self.spine.is_empty()
    }

    /// The head (current) epoch of the spine, or 0 if empty — the ambient epoch a
    /// reducer sees via `ReduceContext` (a pure function of the accepted log).
    pub fn current_epoch(&self) -> u64 {
        self.spine.last().map(|(p, _, _)| p.epoch).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let d = forum_post("hello bob", 0, 1);
        let bytes = d.canonical_bytes();
        let back = decode_delta(&bytes).unwrap();
        assert_eq!(back, d);
        // re-encoding the decoded delta is byte-identical (canonical stability).
        assert_eq!(back.canonical_bytes(), bytes);
        // id is stable
        assert_eq!(back.id(), d.id());
    }

    #[test]
    fn or_set_fold_is_order_independent() {
        let a = [0xAAu8; 32];
        let b = [0xBBu8; 32];
        let da = forum_post("from alice", 0, 1);
        let db = forum_post("from bob", 0, 1);

        let mut c1 = Coordinator::<ForumType>::new(vec![a, b], a);
        assert!(c1.deliver(da.clone(), a).unwrap());
        assert!(c1.deliver(db.clone(), b).unwrap());
        // duplicate is an idempotent no-op
        assert!(!c1.deliver(da.clone(), a).unwrap());

        let mut c2 = Coordinator::<ForumType>::new(vec![a, b], a);
        c2.deliver(db, b).unwrap();
        c2.deliver(da, a).unwrap();

        // both replicas converge to the identical transcript regardless of order
        assert_eq!(c1.state().transcript(), c2.state().transcript());
        assert_eq!(c1.state().transcript().len(), 2);
    }

    /// REVERSED ON PURPOSE (membership-through-mls.md §15.6). This used to assert that
    /// the fold rejects an author outside the CURRENT roster — which, once removal
    /// exists, made every object a removed member had ever written in stop folding on
    /// every device (`fold_entries` propagates the rejection with `?`). Whether an
    /// author was a member is decided once, by MLS, at ingest (§10.1): a stranger never
    /// produces an MLS message the group decrypts, so never reaches a log.
    #[test]
    fn a_departed_authors_commutative_delta_still_folds() {
        let a = [1u8; 32];
        let gone = [9u8; 32];
        let mut c = Coordinator::<ForumType>::new(vec![a], a);
        c.deliver(forum_post("written before leaving", 0, 1), gone)
            .expect("a departed member's past words still fold");
        assert_eq!(c.state().transcript().len(), 1);
    }

    /// §10.2: the spine accepts each owner in the epochs they owned, and nobody else.
    #[test]
    fn the_spine_follows_the_owner_through_the_epochs() {
        let (a, b) = ([1u8; 32], [2u8; 32]);
        let mut spine = SequencedCoordinator::with_owners(vec![(3, b), (0, a)]);
        assert_eq!(spine.owner_at(0), Some(a));
        assert_eq!(spine.owner_at(2), Some(a));
        assert_eq!(spine.owner_at(3), Some(b));
        assert_eq!(spine.owner_at(99), Some(b));
        assert_eq!(spine.current_owner(), Some(b));

        let d1 = sequenced_delta(FORUM_TYPE_ID, 0, Args::new(), 1, 0, GENESIS_PREV);
        spine.deliver(d1.clone(), a).expect("A owned epoch 1");
        // A former owner writing into an epoch after the handover is refused.
        let late = sequenced_delta(FORUM_TYPE_ID, 0, Args::new(), 3, 0, d1.id());
        assert_eq!(spine.deliver(late, a), Err(DeltaRejection::Unauthorized));
        // The new owner continues the SAME chain.
        let d2 = sequenced_delta(FORUM_TYPE_ID, 0, Args::new(), 3, 0, d1.id());
        spine.deliver(d2, b).expect("B owns epoch 3 and chains to A's head");
    }

    /// The chat band of `FORUM_OPS` must BE `CHAT_OPS` — the duplication (a
    /// `static` cannot splice another static) is pinned here so the two tables
    /// cannot drift silently, mirroring group.rs's membership splice pin.
    #[test]
    fn chat_ops_are_spliced_verbatim() {
        for want in CHAT_OPS {
            for (kind, got) in [
                ("Forum", ForumType::op(want.op_id)),
                ("Conversation", ConversationType::op(want.op_id)),
            ] {
                let got = got.unwrap_or_else(|| panic!("{kind} is missing chat op {:#x}", want.op_id));
                assert_eq!(got.name, want.name);
                assert_eq!(got.authority, want.authority);
                assert_eq!(got.commutativity, want.commutativity);
            }
        }
        // Conversation carries the chat band and the parent ops, and nothing else:
        // no room vocabulary on a thread.
        assert_eq!(ConversationType::ops().len(), CHAT_OPS.len() + crate::parent::PARENT_OPS.len());
        for p in crate::parent::PARENT_OPS {
            assert!(ConversationType::op(p.op_id).is_some(), "Conversation must carry {}", p.name);
        }
        assert!(ConversationType::op(crate::parts::OP_SET_PART).is_none());
        assert!(ConversationType::op(crate::parts::OP_CLEAR_PART).is_none());
        for d in FORUM_OPS {
            assert!(
                d.is_well_formed(),
                "op {} violates the spec invariant",
                d.name
            );
        }
    }

    /// The room set folds like a keyed map off the owner-sequenced spine:
    /// attach, re-role in place, detach — with a garbage part id rejected whole,
    /// a detach of a never-attached room inert (a loud PreconditionFailed in the
    /// reducer, skipped by `state()`), and a non-owner write refused at deliver.
    #[test]
    fn forum_rooms_fold_and_gate_on_the_owner() {
        use crate::parts::{OP_CLEAR_PART, OP_SET_PART};
        let owner = [7u8; 32];
        let member = [8u8; 32];
        let room = "f0".repeat(32); // the room forum's object id, hex

        let mut c = Coordinator::<ForumType>::new(vec![owner, member], owner);
        let (mut seq, mut prev) = (0u64, GENESIS_PREV);
        let mut spine = |c: &mut Coordinator<ForumType>, op_id: u32, pairs: Vec<(&str, ArgVal)>| {
            let mut a = Args::new();
            for (k, v) in pairs {
                a.insert(k.into(), v);
            }
            let d = sequenced_delta(FORUM_TYPE_ID, op_id, a, 0, seq, prev);
            prev = d.id();
            seq += 1;
            c.deliver(d, owner).unwrap();
        };
        let t = |s: &str| ArgVal::Text(s.into());

        spine(
            &mut c,
            OP_SET_PART,
            vec![
                ("part", t(&room)),
                ("role", t("room")),
                ("at", ArgVal::Int(1)),
            ],
        );
        let st = c.state();
        assert_eq!(st.parts.get(&room).unwrap().role, "room");
        assert_eq!(st.parts.get(&room).unwrap().at, 1);

        // Re-setting the SAME part updates the edge in place, never mints a second tab.
        spine(
            &mut c,
            OP_SET_PART,
            vec![
                ("part", t(&room)),
                ("role", t("announcements")),
                ("at", ArgVal::Int(2)),
            ],
        );
        assert_eq!(c.state().parts.len(), 1);
        assert_eq!(c.state().parts.get(&room).unwrap().role, "announcements");

        // A fabricated (non-hex) part id never lands.
        spine(
            &mut c,
            OP_SET_PART,
            vec![
                ("part", t("not-hex!")),
                ("role", t("room")),
                ("at", ArgVal::Int(3)),
            ],
        );
        assert_eq!(c.state().parts.len(), 1);

        // Clear removes the edge; clearing again stays inert (the reducer's
        // PreconditionFailed, skipped by the fold). Room posts are untouched.
        spine(&mut c, OP_CLEAR_PART, vec![("part", t(&room))]);
        assert!(c.state().parts.is_empty());
        spine(&mut c, OP_CLEAR_PART, vec![("part", t(&room))]);
        assert!(c.state().parts.is_empty());

        // A NON-owner's part write is refused by the spine at deliver — LOUD.
        let mut a = Args::new();
        a.insert("part".into(), t(&room));
        a.insert("role".into(), t("coup"));
        a.insert("at".into(), ArgVal::Int(9));
        let d = sequenced_delta(FORUM_TYPE_ID, OP_SET_PART, a, 0, seq, prev);
        assert!(c.deliver(d, member).is_err());

        // A Conversation rejects the parts vocabulary outright: unknown op.
        let mut cc = Coordinator::<ConversationType>::new(vec![owner], owner);
        let mut a = Args::new();
        a.insert("part".into(), t(&room));
        a.insert("role".into(), t("room"));
        a.insert("at".into(), ArgVal::Int(1));
        let d = sequenced_delta(
            ObjectKind::Conversation.type_id() as u32,
            OP_SET_PART,
            a,
            0,
            0,
            GENESIS_PREV,
        );
        assert_eq!(cc.deliver(d, owner), Err(DeltaRejection::UnknownType));
    }
}

#[cfg(test)]
mod seq_tests {
    use super::*;

    const OWNER: [u8; 32] = [7u8; 32];

    fn seq_d(epoch: u64, seq: u64, prev: [u8; 32], text: &str) -> Delta {
        let mut args = Args::new();
        args.insert("v".into(), ArgVal::Text(text.into()));
        sequenced_delta(19, 1, args, epoch, seq, prev)
    }

    #[test]
    fn spine_accepts_valid_hash_chain() {
        let mut c = SequencedCoordinator::new(OWNER);
        let d0 = seq_d(0, 0, GENESIS_PREV, "a");
        let id0 = d0.id();
        assert!(c.deliver(d0, OWNER).unwrap());
        let d1 = seq_d(0, 1, id0, "b");
        let id1 = d1.id();
        assert!(c.deliver(d1, OWNER).unwrap());
        assert!(c.deliver(seq_d(0, 2, id1, "c"), OWNER).unwrap());
        assert_eq!(c.len(), 3);
    }

    #[test]
    fn genesis_must_open_its_epoch_and_restated_heads_anchor() {
        let mut c = SequencedCoordinator::new(OWNER);
        // a mid-epoch start can never anchor a spine.
        assert_eq!(
            c.deliver(seq_d(0, 1, GENESIS_PREV, "x"), OWNER),
            Err(DeltaRejection::ChainBroken)
        );
        // a true genesis anchors…
        assert!(c.deliver(seq_d(0, 0, GENESIS_PREV, "ok"), OWNER).unwrap());

        // …and so does a RESTATED HEAD on a fresh replica: a late joiner cannot
        // hold the chain's prefix, so the owner's restatement (prev = an id this
        // replica has never seen, seq 0 at its epoch) is accepted as the anchor.
        // Integrity is enforced from here on: the NEXT delta must chain properly.
        let mut late = SequencedCoordinator::new(OWNER);
        let anchor = seq_d(3, 0, [9u8; 32], "restated");
        let anchor_id = anchor.id();
        assert!(late.deliver(anchor, OWNER).unwrap());
        assert!(late.deliver(seq_d(3, 1, anchor_id, "next"), OWNER).unwrap());
        assert_eq!(
            late.deliver(seq_d(3, 2, [7u8; 32], "bad-prev"), OWNER),
            Err(DeltaRejection::ChainBroken)
        );
    }

    #[test]
    fn chain_broken_on_bad_prev() {
        let mut c = SequencedCoordinator::new(OWNER);
        c.deliver(seq_d(0, 0, GENESIS_PREV, "a"), OWNER).unwrap();
        // seq 1, but prev points at genesis instead of d0's id.
        assert_eq!(
            c.deliver(seq_d(0, 1, GENESIS_PREV, "b"), OWNER),
            Err(DeltaRejection::ChainBroken)
        );
    }

    #[test]
    fn gap_in_seq_breaks_chain() {
        let mut c = SequencedCoordinator::new(OWNER);
        let d0 = seq_d(0, 0, GENESIS_PREV, "a");
        let id0 = d0.id();
        c.deliver(d0, OWNER).unwrap();
        // prev is correct (points at d0) but seq jumps 0 -> 2, skipping 1.
        assert_eq!(
            c.deliver(seq_d(0, 2, id0, "c"), OWNER),
            Err(DeltaRejection::ChainBroken)
        );
    }

    #[test]
    fn fork_detected_at_same_position() {
        let mut c = SequencedCoordinator::new(OWNER);
        c.deliver(seq_d(0, 0, GENESIS_PREV, "a"), OWNER).unwrap();
        // a DIFFERENT delta claiming the SAME (0,0) slot — a divergence halt.
        assert_eq!(
            c.deliver(seq_d(0, 0, GENESIS_PREV, "different"), OWNER),
            Err(DeltaRejection::ForkDetected)
        );
    }

    #[test]
    fn duplicate_is_idempotent() {
        let mut c = SequencedCoordinator::new(OWNER);
        let d0 = seq_d(0, 0, GENESIS_PREV, "a");
        assert!(c.deliver(d0.clone(), OWNER).unwrap());
        assert!(!c.deliver(d0, OWNER).unwrap()); // same delta again -> Ok(false)
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn epoch_advance_resets_seq() {
        let mut c = SequencedCoordinator::new(OWNER);
        let d0 = seq_d(0, 0, GENESIS_PREV, "a");
        let id0 = d0.id();
        c.deliver(d0, OWNER).unwrap();
        let d1 = seq_d(0, 1, id0, "b");
        let id1 = d1.id();
        c.deliver(d1, OWNER).unwrap();
        // epoch bumps to 1; seq resets to 0; chained to head (d1).
        assert!(c.deliver(seq_d(1, 0, id1, "c"), OWNER).unwrap());
        assert_eq!(c.len(), 3);
    }

    #[test]
    fn stale_epoch_rejected_after_fence() {
        let mut c = SequencedCoordinator::new(OWNER);
        let d0 = seq_d(0, 0, GENESIS_PREV, "a");
        let id0 = d0.id();
        c.deliver(d0, OWNER).unwrap();
        c.deliver(seq_d(1, 0, id0, "b"), OWNER).unwrap(); // advance to epoch 1
                                                          // a late delta for the fenced epoch 0 is stale.
        assert_eq!(
            c.deliver(seq_d(0, 1, id0, "late"), OWNER),
            Err(DeltaRejection::StaleEpoch)
        );
    }

    #[test]
    fn non_owner_cannot_sequence() {
        let mut c = SequencedCoordinator::new(OWNER);
        assert_eq!(
            c.deliver(seq_d(0, 0, GENESIS_PREV, "a"), [9u8; 32]),
            Err(DeltaRejection::Unauthorized)
        );
    }
}

#[cfg(test)]
mod generic_tests {
    use super::*;

    // A test-only ObjectType exercising BOTH fold arms: an owner-sequenced base
    // plus any-member commutative bumps (per-author LWW). It borrows Project's
    // kind tag (type_id 22) purely as a KIND label — it is registered nowhere.
    const OP_SET_BASE: u32 = 0; // owner / sequenced
    const OP_BUMP: u32 = 1; //     any-member / commutative

    #[derive(Clone, Default)]
    struct CounterState {
        base_set: bool,
        base: i64,
        adds: BTreeMap<MemberId, (u64, i64)>, // author -> (gen, val), LWW by gen
    }
    impl CounterState {
        fn total(&self) -> i64 {
            self.base + self.adds.values().map(|(_, v)| *v).sum::<i64>()
        }
    }

    static COUNTER_OPS: &[OpDecl] = &[
        OpDecl {
            op_id: OP_SET_BASE,
            name: "counter.setBase",
            authority: Authority::Owner,
            commutativity: Commutativity::Sequenced,
        },
        OpDecl {
            op_id: OP_BUMP,
            name: "counter.bump",
            authority: Authority::AnyMember,
            commutativity: Commutativity::Commutative,
        },
    ];

    struct Counter;
    impl ObjectType for Counter {
        const KIND: ObjectKind = ObjectKind::Project;
        type State = CounterState;
        fn ops() -> &'static [OpDecl] {
            COUNTER_OPS
        }
        fn reduce(state: &mut CounterState, op: &Op<'_>) -> Result<(), DeltaRejection> {
            match op.op_id {
                OP_SET_BASE => {
                    let base = match crate::arg_reads::get(op.args, "base") {
                        Some(ArgVal::Int(n)) => *n,
                        _ => return Err(DeltaRejection::MalformedArgs),
                    };
                    state.base = base;
                    state.base_set = true;
                    Ok(())
                }
                OP_BUMP => {
                    // cross-arm precondition: a bump is inert until the owner set a base.
                    if !state.base_set {
                        return Err(DeltaRejection::PreconditionFailed);
                    }
                    let gen = match crate::arg_reads::get(op.args, "gen") {
                        Some(ArgVal::Int(g)) => *g as u64,
                        _ => return Err(DeltaRejection::MalformedArgs),
                    };
                    let val = match crate::arg_reads::get(op.args, "val") {
                        Some(ArgVal::Int(v)) => *v,
                        _ => return Err(DeltaRejection::MalformedArgs),
                    };
                    // per-author LWW, monotone in gen (a stale re-emit cannot rewind).
                    match state.adds.get(op.author) {
                        Some((prev_gen, _)) if gen <= *prev_gen => {}
                        _ => {
                            state.adds.insert(*op.author, (gen, val));
                        }
                    }
                    Ok(())
                }
                _ => Err(DeltaRejection::UnknownType),
            }
        }
    }

    const OWNER: MemberId = [7u8; 32];
    const A: MemberId = [0xA1u8; 32];

    fn set_base(base: i64, seq: u64, prev: [u8; 32]) -> Delta {
        let mut args = Args::new();
        args.insert("base".into(), ArgVal::Int(base));
        sequenced_delta(
            ObjectKind::Project.type_id() as u32,
            OP_SET_BASE,
            args,
            0,
            seq,
            prev,
        )
    }
    fn bump(val: i64, gen: u64) -> Delta {
        let mut args = Args::new();
        args.insert("gen".into(), ArgVal::Int(gen as i64));
        args.insert("val".into(), ArgVal::Int(val));
        Delta {
            type_id: ObjectKind::Project.type_id() as u32,
            op_id: OP_BUMP,
            op_version: 1,
            args,
            epoch: 0,
            prev: GENESIS_PREV,
            seq: None,
            gen: Some(gen),
            // Authored PRIVATE unless the author said otherwise — the safe default,
            // and the one that costs nothing on the wire.
            visibility: crate::visibility::Visibility::default(),
        }
    }

    #[test]
    fn sequenced_folds_before_commutative_regardless_of_delivery_order() {
        // base=10 (owner, seq0); owner bumps +3; A bumps +5  => 18.
        // Deliver the COMMUTATIVE bumps BEFORE the sequenced base: the fold must
        // still apply base first, so each bump's precondition is met.
        let mut c = Coordinator::<Counter>::new(vec![OWNER, A], OWNER);
        assert!(c.deliver(bump(5, 0), A).unwrap());
        assert!(c.deliver(bump(3, 0), OWNER).unwrap());
        assert!(c.deliver(set_base(10, 0, GENESIS_PREV), OWNER).unwrap());
        assert_eq!(c.state().total(), 18);

        // a different delivery order converges to the identical total.
        let mut c2 = Coordinator::<Counter>::new(vec![OWNER, A], OWNER);
        assert!(c2.deliver(set_base(10, 0, GENESIS_PREV), OWNER).unwrap());
        assert!(c2.deliver(bump(3, 0), OWNER).unwrap());
        assert!(c2.deliver(bump(5, 0), A).unwrap());
        assert_eq!(c2.state().total(), 18);
    }

    #[test]
    fn commutative_bump_is_inert_until_base_is_set() {
        // no base delivered => the bump's precondition fails at fold => total 0.
        let mut c = Coordinator::<Counter>::new(vec![OWNER, A], OWNER);
        assert!(c.deliver(bump(5, 0), A).unwrap());
        assert_eq!(c.state().total(), 0);
    }

    #[test]
    fn non_owner_cannot_write_the_sequenced_spine() {
        let mut c = Coordinator::<Counter>::new(vec![OWNER, A], OWNER);
        assert_eq!(
            c.deliver(set_base(10, 0, GENESIS_PREV), A),
            Err(DeltaRejection::Unauthorized)
        );
    }

    #[test]
    fn unknown_op_is_loud() {
        let mut c = Coordinator::<Counter>::new(vec![OWNER], OWNER);
        let mut args = Args::new();
        args.insert("gen".into(), ArgVal::Int(0));
        let bad = Delta {
            type_id: ObjectKind::Project.type_id() as u32,
            op_id: 99,
            op_version: 1,
            args,
            epoch: 0,
            prev: GENESIS_PREV,
            seq: None,
            gen: Some(0),
            // Authored PRIVATE unless the author said otherwise — the safe default,
            // and the one that costs nothing on the wire.
            visibility: crate::visibility::Visibility::default(),
        };
        assert_eq!(c.deliver(bad, OWNER), Err(DeltaRejection::UnknownType));
    }

    #[test]
    fn next_sequenced_pos_genesis_increment_and_epoch_reset() {
        assert_eq!(next_sequenced_pos(None, 5), (0, GENESIS_PREV));
        let id = [3u8; 32];
        assert_eq!(
            next_sequenced_pos(Some((LogPosition { epoch: 2, seq: 4 }, id)), 2),
            (5, id)
        );
        // a bumped epoch resets seq to 0 but still chains to the head id.
        assert_eq!(
            next_sequenced_pos(Some((LogPosition { epoch: 2, seq: 4 }, id)), 3),
            (0, id)
        );
    }

    #[test]
    fn sequenced_head_tracks_the_spine() {
        let mut c = Coordinator::<Counter>::new(vec![OWNER], OWNER);
        assert_eq!(c.sequenced_head(), None);
        let d0 = set_base(10, 0, GENESIS_PREV);
        let id0 = d0.id();
        c.deliver(d0, OWNER).unwrap();
        let d1 = set_base(20, 1, id0);
        let id1 = d1.id();
        c.deliver(d1, OWNER).unwrap();
        assert_eq!(
            c.sequenced_head(),
            Some((LogPosition { epoch: 0, seq: 1 }, id1))
        );
    }
}

/// Unit tests for the Reddit-style `ForumState::thread()` projection. These build
/// `ForumState` directly (its fields are pub) to test the tree assembly in
/// isolation from the fold/transport — the reducer that fills `messages` is
/// covered elsewhere.
#[cfg(test)]
mod thread_tests {
    use super::*;

    fn r(author: u8, gen: u64) -> MsgRef {
        ([author; 32], gen)
    }

    /// Build a preprocessed attachment the way the FFI boundary does.
    fn attachment(px: u32) -> MediaRef {
        let rgba = vec![120u8; (px * px * 4) as usize];
        let facts = pacific_media::SourceFacts {
            source_mime: "image/heic".into(),
            source_bytes: 2_400_000,
            width: px,
            height: px,
            orientation_applied: true,
            ..Default::default()
        };
        pacific_media::preprocess(&rgba, px, px, Slot::Message, &facts)
            .unwrap()
            .media
    }

    #[test]
    fn a_chat_message_carries_media_through_author_fold_and_read() {
        let a = [1u8; 32];
        let m = attachment(512);
        let mut c = Coordinator::<ForumType>::new(vec![a], a);
        c.deliver(forum_post_full("look at this", 0, 1, 0, None, Some(&m)), a)
            .expect("deliver");

        let msg = c
            .state()
            .detailed()
            .into_iter()
            .find(|x| x.gen == 0)
            .expect("message");
        assert_eq!(msg.text, "look at this");
        let media = msg.media.as_ref().expect("media survived the fold");
        assert!(media.is_preprocessed(), "a chat must never carry raw bytes");
        assert!(media.validate(Slot::Message).is_ok());
        assert_eq!((media.width, media.height), (160, 160));
    }

    #[test]
    fn a_text_only_message_costs_exactly_what_it_did_before() {
        // Media is opt-in: an unattached post must not gain a single arg, or every
        // text message in every conversation pays for a feature it does not use.
        let plain = forum_post_full("hello", 0, 1, 0, None, None);
        assert!(plain.args.keys().all(|k| !k.starts_with("media")));
        assert_eq!(plain.args.len(), 2); // text + gen
    }

    #[test]
    fn an_oversize_attachment_is_refused_at_the_fold() {
        // The gate that stops a patched peer putting an unbounded attachment into
        // every member's store forever. Refused, never clamped.
        let a = [2u8; 32];
        let mut m = attachment(256);
        m.delivery = pacific_media::Delivery::Inline {
            data: "A".repeat(Slot::Message.max_b64() + 4),
        };
        let mut c = Coordinator::<ForumType>::new(vec![a], a);
        let _ = c.deliver(forum_post_full("too big", 0, 1, 0, None, Some(&m)), a);
        assert!(
            c.state().detailed().is_empty(),
            "an oversize attachment must not land in the transcript"
        );
    }

    #[test]
    fn a_post_authored_before_media_still_folds() {
        // Every message already in every transcript predates this feature.
        let a = [3u8; 32];
        let mut c = Coordinator::<ForumType>::new(vec![a], a);
        c.deliver(forum_post("old message", 0, 1), a)
            .expect("legacy post must still fold");
        let msg = c.state().detailed().into_iter().next().expect("message");
        assert_eq!(msg.text, "old message");
        assert!(msg.media.is_none());
    }

    fn post(text: &str, reply_to: Option<MsgRef>) -> Msg {
        Msg {
            text: text.into(),
            media: None,
            ts: 0,
            reply_to,
        }
    }
    /// A ForumState from (author, gen, text, reply_to) rows — insertion order is
    /// irrelevant (BTreeMap), which is the point: the fold is order-free.
    fn state(rows: &[(u8, u64, &str, Option<MsgRef>)]) -> ForumState {
        let mut messages = BTreeMap::new();
        for &(a, g, t, rt) in rows {
            messages.insert(r(a, g), post(t, rt));
        }
        ForumState {
            messages,
            ..ForumState::default()
        }
    }
    /// Compact ((author,gen), depth, descendants) view for assertions.
    fn shape(t: &[ThreadNode]) -> Vec<((u8, u64), u32, u32)> {
        t.iter()
            .map(|n| ((n.msg.author[0], n.msg.gen), n.depth, n.descendants))
            .collect()
    }

    #[test]
    fn nests_replies_with_depth_and_descendants() {
        // A(root) <- B <- C ; A <- D. Two children of A, ordered by (gen, author).
        let a = r(1, 1);
        let b = r(2, 2);
        let s = state(&[
            (1, 1, "A", None),
            (2, 2, "B", Some(a)),
            (1, 3, "C", Some(b)),
            (2, 4, "D", Some(a)),
        ]);
        let t = s.thread();
        // pre-order: A, [B, C], D — B before D (gen 2 < gen 4).
        assert_eq!(
            shape(&t),
            vec![
                ((1, 1), 0, 3), // A: three posts nested beneath it
                ((2, 2), 1, 1), // B: one (C)
                ((1, 3), 2, 0), // C: leaf
                ((2, 4), 1, 0), // D: leaf
            ]
        );
        assert_eq!(t.len(), s.messages.len());
    }

    #[test]
    fn orphan_surfaces_as_root_then_renests_when_parent_arrives() {
        let a = r(1, 1);
        // B replies to A, but A has NOT synced yet — B must not be dropped.
        let orphaned = state(&[(2, 2, "B", Some(a))]);
        let t0 = orphaned.thread();
        assert_eq!(shape(&t0), vec![((2, 2), 0, 0)]); // B is a root
        assert_eq!(t0.len(), orphaned.messages.len());

        // A arrives — B re-nests beneath it, no code path changed.
        let mut healed = orphaned.clone();
        healed.messages.insert(a, post("A", None));
        let t1 = healed.thread();
        assert_eq!(shape(&t1), vec![((1, 1), 0, 1), ((2, 2), 1, 0)]);
        assert_eq!(t1.len(), healed.messages.len());
    }

    #[test]
    fn parent_cycle_is_broken_and_every_post_appears_once() {
        // A replies to B and B replies to A (pathological — the visited-set must
        // stop the recursion and still surface both, exactly once each).
        let a = r(1, 1);
        let b = r(2, 2);
        let s = state(&[(1, 1, "A", Some(b)), (2, 2, "B", Some(a))]);
        let t = s.thread();
        assert_eq!(t.len(), 2);
        assert_eq!(t.len(), s.messages.len());
        let mut refs: Vec<(u8, u64)> = t.iter().map(|n| (n.msg.author[0], n.msg.gen)).collect();
        refs.sort();
        assert_eq!(refs, vec![(1, 1), (2, 2)]);
    }

    #[test]
    fn self_reply_is_treated_as_root() {
        let a = r(1, 1);
        let s = state(&[(1, 1, "A", Some(a))]); // A replies to itself
        let t = s.thread();
        assert_eq!(shape(&t), vec![((1, 1), 0, 0)]);
    }

    #[test]
    fn sibling_and_root_order_is_deterministic_by_causal() {
        // Two roots at the same gen, different authors -> ordered by author.
        let s = state(&[(2, 1, "root-b", None), (1, 1, "root-a", None)]);
        let t = s.thread();
        assert_eq!(shape(&t), vec![((1, 1), 0, 0), ((2, 1), 0, 0)]);
    }

    #[test]
    fn deep_linear_chain_does_not_overflow_the_stack() {
        // A hostile 20k-deep reply chain (each post replies to the previous).
        // `reply_to` is member-supplied, so this is attacker-reachable; a recursive
        // walk would blow the ~2 MiB test-thread stack folding it. The iterative
        // walk flattens it — proof the projection can't be crashed by a deep chain.
        const N: u64 = 20_000;
        let mut messages = BTreeMap::new();
        for g in 1..=N {
            let parent = if g == 1 { None } else { Some(r(1, g - 1)) };
            messages.insert(r(1, g), post(&format!("m{g}"), parent));
        }
        let s = ForumState {
            messages,
            ..ForumState::default()
        };
        let t = s.thread();
        assert_eq!(t.len() as u64, N); // every post, exactly once
        assert_eq!(t.first().unwrap().depth, 0);
        assert_eq!(t.first().unwrap().descendants as u64, N - 1); // root sees all below
        assert_eq!(t.last().unwrap().depth as u64, N - 1); // deepest leaf
        assert_eq!(t.last().unwrap().descendants, 0);
    }

    /// The headline convergence claim, through the REAL fold: two replicas that
    /// receive the same posts+replies in DIFFERENT delivery orders — one with
    /// replies arriving BEFORE their parents — project a byte-identical thread.
    #[test]
    fn two_replicas_converge_on_identical_thread_regardless_of_delivery_order() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        // a posts a root; b replies to it; a replies to b; b posts a second root.
        // reply_to targets a parent's (author, gen) MsgRef.
        let root_a = forum_post("root from a", 0, 1); //            (a,0)
        let reply_b = forum_post_full("b replies", 0, 1, 0, Some((a, 0)), None); // (b,0) -> (a,0)
        let reply_a = forum_post_full("a replies to b", 1, 1, 0, Some((b, 0)), None); // (a,1) -> (b,0)
        let root_b = forum_post("root from b", 1, 1); //           (b,1)

        // Replica 1 — causal order.
        let mut c1 = Coordinator::<ForumType>::new(vec![a, b], a);
        c1.deliver(root_a.clone(), a).unwrap();
        c1.deliver(reply_b.clone(), b).unwrap();
        c1.deliver(reply_a.clone(), a).unwrap();
        c1.deliver(root_b.clone(), b).unwrap();

        // Replica 2 — adversarial order: every reply arrives before its parent.
        let mut c2 = Coordinator::<ForumType>::new(vec![a, b], a);
        c2.deliver(reply_a, a).unwrap();
        c2.deliver(root_b, b).unwrap();
        c2.deliver(reply_b, b).unwrap();
        c2.deliver(root_a, a).unwrap();

        let full = |t: Vec<ThreadNode>| -> Vec<([u8; 32], u64, u32, u32, String)> {
            t.into_iter()
                .map(|n| (n.msg.author, n.msg.gen, n.depth, n.descendants, n.msg.text))
                .collect()
        };
        let s1 = full(c1.state().thread());
        let s2 = full(c2.state().thread());
        assert_eq!(s1, s2, "replicas diverged on thread projection");
        assert_eq!(s1.len(), 4);
    }
}

/// Delivery/read receipts, folded through the REAL `Coordinator<ForumType>` — the
/// same engine transport drives. Proves the `forum.receipt` reducer + the
/// grow-only lattice + `receipt_summary`'s min-across-recipients aggregation.
#[cfg(test)]
mod receipt_tests {
    use super::*;

    #[test]
    fn dm_receipt_climbs_sent_then_delivered_then_read() {
        let a = [1u8; 32]; // sender + owner
        let b = [2u8; 32]; // recipient
        let mut c = Coordinator::<ForumType>::new(vec![a, b], a);
        // a sends one message (a, gen 0).
        c.deliver(forum_post("hi", 0, 1), a).unwrap();
        let target = (a, 0);
        let others = [b];

        // Before any receipt: no recipient status → "sent" floor (summary 0).
        assert_eq!(c.state().receipt_summary(&target, &others), 0);

        // b delivers, then reads.
        c.deliver(forum_receipt(target, RECEIPT_DELIVERED, 1, 1), b)
            .unwrap();
        assert_eq!(
            c.state().receipt_summary(&target, &others),
            RECEIPT_DELIVERED
        );
        c.deliver(forum_receipt(target, RECEIPT_READ, 2, 1), b)
            .unwrap();
        assert_eq!(c.state().receipt_summary(&target, &others), RECEIPT_READ);
    }

    #[test]
    fn read_never_regresses_when_a_stale_delivered_arrives_late() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        let target = (a, 0);
        let mut c = Coordinator::<ForumType>::new(vec![a, b], a);
        c.deliver(forum_post("hi", 0, 1), a).unwrap();
        // read (gen 2) folds in BEFORE the delivered (gen 1) — the max lattice must
        // keep READ, never drop back to DELIVERED.
        c.deliver(forum_receipt(target, RECEIPT_READ, 2, 1), b)
            .unwrap();
        c.deliver(forum_receipt(target, RECEIPT_DELIVERED, 1, 1), b)
            .unwrap();
        assert_eq!(c.state().receipt_summary(&target, &[b]), RECEIPT_READ);
    }

    #[test]
    fn group_summary_is_the_minimum_across_all_recipients() {
        let a = [1u8; 32]; // sender
        let b = [2u8; 32];
        let d = [3u8; 32];
        let target = (a, 0);
        let others = [b, d];
        let mut c = Coordinator::<ForumType>::new(vec![a, b, d], a);
        c.deliver(forum_post("hi all", 0, 1), a).unwrap();

        // only b delivered → still "sent" (d hasn't received it).
        c.deliver(forum_receipt(target, RECEIPT_DELIVERED, 1, 1), b)
            .unwrap();
        assert_eq!(c.state().receipt_summary(&target, &others), 0);
        // both delivered → ✓✓ (delivered to everyone).
        c.deliver(forum_receipt(target, RECEIPT_DELIVERED, 1, 1), d)
            .unwrap();
        assert_eq!(
            c.state().receipt_summary(&target, &others),
            RECEIPT_DELIVERED
        );
        // b reads, d hasn't → still just delivered (min).
        c.deliver(forum_receipt(target, RECEIPT_READ, 2, 1), b)
            .unwrap();
        assert_eq!(
            c.state().receipt_summary(&target, &others),
            RECEIPT_DELIVERED
        );
        // both read → ✓✓ blue.
        c.deliver(forum_receipt(target, RECEIPT_READ, 2, 1), d)
            .unwrap();
        assert_eq!(c.state().receipt_summary(&target, &others), RECEIPT_READ);
    }

    #[test]
    fn a_self_receipt_is_ignored() {
        // A hostile/buggy peer authoring a receipt on ITS OWN message must not
        // inflate the ticks: the reducer drops a receipt whose target author is
        // the receiptor.
        let a = [1u8; 32];
        let b = [2u8; 32];
        let target = (a, 0);
        let mut c = Coordinator::<ForumType>::new(vec![a, b], a);
        c.deliver(forum_post("hi", 0, 1), a).unwrap();
        // `a` tries to mark its own message read.
        c.deliver(forum_receipt(target, RECEIPT_READ, 1, 1), a)
            .unwrap();
        assert!(c.state().receipts.get(&target).is_none());
        assert_eq!(c.state().receipt_summary(&target, &[b]), 0);
    }

    #[test]
    fn one_batched_receipt_acks_many_messages() {
        // The Signal-style win: a burst of three messages is acknowledged by a
        // SINGLE receipt delta, not three.
        let a = [1u8; 32];
        let b = [2u8; 32];
        let mut c = Coordinator::<ForumType>::new(vec![a, b], a);
        c.deliver(forum_post("one", 0, 1), a).unwrap();
        c.deliver(forum_post("two", 1, 1), a).unwrap();
        c.deliver(forum_post("three", 2, 1), a).unwrap();

        let refs = [(a, 0), (a, 1), (a, 2)];
        c.deliver(forum_receipt_batch(&refs, RECEIPT_READ, 3, 1), b)
            .unwrap();

        for g in 0..3 {
            assert_eq!(
                c.state().receipt_summary(&(a, g), &[b]),
                RECEIPT_READ,
                "message gen {g} read via the batch"
            );
        }
    }

    #[test]
    fn batched_receipt_bytes_are_order_independent() {
        // The acknowledged SET, not its order, must fix the delta id — else two
        // replicas building the same batch in different orders fork the content
        // address and the OR-set fails to dedup.
        let a = [1u8; 32];
        let r1 = (a, 0);
        let r2 = (a, 7);
        let d1 = forum_receipt_batch(&[r1, r2], RECEIPT_DELIVERED, 1, 1);
        let d2 = forum_receipt_batch(&[r2, r1, r1], RECEIPT_DELIVERED, 1, 1); // reordered + dup
        assert_eq!(
            d1.id(),
            d2.id(),
            "batch order/dupes must not change the delta id"
        );
    }

    #[test]
    fn receipts_converge_regardless_of_delivery_order() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        let d = [3u8; 32];
        let target = (a, 0);
        let post = forum_post("hi", 0, 1);
        let rb = forum_receipt(target, RECEIPT_READ, 2, 1);
        let rd = forum_receipt(target, RECEIPT_DELIVERED, 1, 1);

        let mut c1 = Coordinator::<ForumType>::new(vec![a, b, d], a);
        c1.deliver(post.clone(), a).unwrap();
        c1.deliver(rb.clone(), b).unwrap();
        c1.deliver(rd.clone(), d).unwrap();

        let mut c2 = Coordinator::<ForumType>::new(vec![a, b, d], a);
        c2.deliver(rd, d).unwrap();
        c2.deliver(rb, b).unwrap();
        c2.deliver(post, a).unwrap();

        assert_eq!(c1.state().receipts, c2.state().receipts);
    }
}

/// The base RATIFY op-group folded through the REAL `Coordinator<ForumType>` — the
/// governance primitive (propose/vote/close) with roles as input, exercised over the
/// note's family scenario (Sophia owner, three members). Proves: consent passes when
/// no eligible member rejects, a single reject fails, the outcome is Pending until a
/// close, solo collapses to an electorate of one, ballots are LWW, and the whole
/// thing converges regardless of delivery order.
#[cfg(test)]
mod ratify_tests {
    use super::*;

    // Sophia owns; Nye, Tom, Lauren are members.
    const SOPHIA: MemberId = [1u8; 32];
    const NYE: MemberId = [2u8; 32];
    const TOM: MemberId = [3u8; 32];
    const LAUREN: MemberId = [4u8; 32];
    const TID: u32 = FORUM_TYPE_ID;

    fn family() -> Coordinator<ForumType> {
        Coordinator::<ForumType>::new(vec![SOPHIA, NYE, TOM, LAUREN], SOPHIA)
    }

    #[test]
    fn consent_passes_when_no_eligible_member_rejects() {
        let mut c = family();
        c.deliver(
            ratify_propose("blue car -> Wales, Thu-Sun", Rule::Consent, 0, 1, TID),
            SOPHIA,
        )
        .unwrap();
        let pid = (SOPHIA, 0);
        // Pending until a close, even with every ballot in.
        c.deliver(ratify_vote(pid, Ballot::Approve, 1, 1, TID), NYE)
            .unwrap();
        c.deliver(ratify_vote(pid, Ballot::Approve, 2, 1, TID), TOM)
            .unwrap();
        c.deliver(ratify_vote(pid, Ballot::Approve, 3, 1, TID), LAUREN)
            .unwrap();
        assert_eq!(c.ratify_state().outcome(&pid, &SOPHIA), Outcome::Pending);

        // Sophia (owner) closes → passes, and the inner fact is committed.
        c.deliver(ratify_close(pid, 1, 0, GENESIS_PREV, TID), SOPHIA)
            .unwrap();
        let rs = c.ratify_state();
        assert_eq!(rs.outcome(&pid, &SOPHIA), Outcome::Passed);
        assert_eq!(
            rs.ratified(&SOPHIA),
            vec!["blue car -> Wales, Thu-Sun".to_string()]
        );
    }

    /// THE PAYMENT BUG §10.4 EXISTS FOR. A consent proposal fails on one reject and is
    /// closed. Then the dissenter is removed. A LEGACY close re-reads the current
    /// roster and flips to Passed; a FROZEN close does not.
    #[test]
    fn removing_a_dissenter_does_not_flip_a_frozen_close() {
        let propose = ratify_propose("release €1,800", Rule::Consent, 0, 1, TID);
        let pid = (SOPHIA, 0);
        let yes = ratify_vote(pid, Ballot::Approve, 1, 1, TID);
        let no = ratify_vote(pid, Ballot::Reject, 2, 1, TID);
        let electorate = [SOPHIA, NYE, TOM, LAUREN];
        let frozen = ratify_close_frozen(pid, &electorate, &[yes.id(), no.id()], 1, 0, GENESIS_PREV, TID);
        let legacy = ratify_close(pid, 1, 0, GENESIS_PREV, TID);

        for (close, flips) in [(frozen, false), (legacy, true)] {
            for roster in [vec![SOPHIA, NYE, TOM, LAUREN], vec![SOPHIA, NYE, LAUREN]] {
                let removed = roster.len() == 3;
                let mut c = Coordinator::<ForumType>::new(roster, SOPHIA);
                c.deliver(propose.clone(), SOPHIA).unwrap();
                c.deliver(yes.clone(), NYE).unwrap();
                c.deliver(no.clone(), TOM).unwrap();
                c.deliver(close.clone(), SOPHIA).unwrap();
                let got = c.ratify_state().outcome(&pid, &SOPHIA);
                let want = if removed && flips { Outcome::Passed } else { Outcome::Failed };
                assert_eq!(got, want, "frozen={} removed={removed}", !flips);
            }
        }
    }

    #[test]
    fn ratify_ignores_a_departed_members_vote_on_an_open_proposal() {
        // Still OPEN, the tally is today's members' (§10.1): a member who left before
        // the close no longer counts. Only a close freezes an electorate.
        let propose = ratify_propose("paint it green", Rule::Consent, 0, 1, TID);
        let pid = (SOPHIA, 0);
        let no = ratify_vote(pid, Ballot::Reject, 1, 1, TID);
        let mut c = Coordinator::<ForumType>::new(vec![SOPHIA, NYE, LAUREN], SOPHIA);
        c.deliver(propose, SOPHIA).unwrap();
        c.deliver(no, TOM).unwrap();
        let rs = c.ratify_state();
        assert!(rs.votes.get(&pid).map_or(true, |v| !v.contains_key(&TOM)), "TOM left");
        assert_eq!(rs.outcome(&pid, &SOPHIA), Outcome::Pending, "and nothing is closed");
    }

    #[test]
    fn a_close_counts_exactly_the_ballots_it_names_and_reports_the_rest() {
        let mut c = family();
        c.deliver(ratify_propose("paint it red", Rule::Consent, 0, 1, TID), SOPHIA).unwrap();
        let pid = (SOPHIA, 0);
        let nye = ratify_vote(pid, Ballot::Approve, 1, 1, TID);
        let tom = ratify_vote(pid, Ballot::Reject, 2, 1, TID);
        c.deliver(nye.clone(), NYE).unwrap();
        c.deliver(tom.clone(), TOM).unwrap();
        // The close counts Nye's ballot only — Tom's reject arrived "after".
        let close = ratify_close_frozen(pid, &[SOPHIA, NYE, TOM, LAUREN], &[nye.id()], 1, 0, GENESIS_PREV, TID);
        c.deliver(close, SOPHIA).unwrap();
        let rs = c.ratify_state();
        assert_eq!(rs.outcome(&pid, &SOPHIA), Outcome::Passed);
        assert!(rs.closed[&pid].frozen);
        assert_eq!(rs.unlisted, vec![(pid, tom.id())], "the omitted reject is visible, not silent");
    }

    #[test]
    fn a_close_waits_for_a_ballot_it_names_but_this_device_lacks() {
        let mut c = family();
        c.deliver(ratify_propose("buy timber", Rule::Consent, 0, 1, TID), SOPHIA).unwrap();
        let pid = (SOPHIA, 0);
        let tom = ratify_vote(pid, Ballot::Reject, 2, 1, TID);
        let close = ratify_close_frozen(pid, &[SOPHIA, NYE, TOM, LAUREN], &[tom.id()], 1, 0, GENESIS_PREV, TID);
        c.deliver(close, SOPHIA).unwrap();
        assert_eq!(c.ratify_state().outcome(&pid, &SOPHIA), Outcome::Pending, "never guessed");
        c.deliver(tom, TOM).unwrap();
        assert_eq!(c.ratify_state().outcome(&pid, &SOPHIA), Outcome::Failed);
    }

    #[test]
    fn a_close_naming_a_foreign_ballot_or_a_stranger_is_inert() {
        let mut c = family();
        c.deliver(ratify_propose("a", Rule::Consent, 0, 1, TID), SOPHIA).unwrap();
        c.deliver(ratify_propose("b", Rule::Consent, 1, 1, TID), SOPHIA).unwrap();
        let (pa, pb) = ((SOPHIA, 0), (SOPHIA, 1));
        let on_b = ratify_vote(pb, Ballot::Approve, 2, 1, TID);
        c.deliver(on_b.clone(), NYE).unwrap();
        let close = ratify_close_frozen(pa, &[SOPHIA, NYE], &[on_b.id()], 1, 0, GENESIS_PREV, TID);
        c.deliver(close, SOPHIA).unwrap();
        assert!(!c.ratify_state().closed.contains_key(&pa), "a close naming another proposal's ballot is inert");

        let mut c = family();
        c.deliver(ratify_propose("c", Rule::Consent, 0, 1, TID), SOPHIA).unwrap();
        let pc = (SOPHIA, 0);
        let lauren = ratify_vote(pc, Ballot::Reject, 3, 1, TID);
        c.deliver(lauren.clone(), LAUREN).unwrap();
        // Lauren is not in the named electorate, yet her ballot is listed.
        let close = ratify_close_frozen(pc, &[SOPHIA, NYE], &[lauren.id()], 1, 0, GENESIS_PREV, TID);
        c.deliver(close, SOPHIA).unwrap();
        assert!(!c.ratify_state().closed.contains_key(&pc));
    }

    #[test]
    fn owner_approval_is_judged_by_the_owner_who_closed_it() {
        // Sophia owned the object until epoch 2, then handed it to Nye.
        let mut c = Coordinator::<ForumType>::with_owners(vec![SOPHIA, NYE, TOM], vec![(0, SOPHIA), (2, NYE)]);
        c.deliver(ratify_propose("host the fair", Rule::OwnerApproval, 0, 1, TID), NYE).unwrap();
        let pid = (NYE, 0);
        let sophia_yes = ratify_vote(pid, Ballot::Approve, 1, 1, TID);
        c.deliver(sophia_yes.clone(), SOPHIA).unwrap();
        let close = ratify_close_frozen(pid, &[SOPHIA, NYE, TOM], &[sophia_yes.id()], 1, 0, GENESIS_PREV, TID);
        c.deliver(close, SOPHIA).unwrap();
        let rs = c.ratify_state();
        assert_eq!(rs.closed[&pid].owner, SOPHIA, "the owner at the close's epoch");
        // Asked with TODAY's owner, the answer is still Sophia's.
        assert_eq!(rs.outcome(&pid, &NYE), Outcome::Passed);
    }

    #[test]
    fn a_single_reject_fails_consent() {
        let mut c = family();
        c.deliver(
            ratify_propose("sell the blue car", Rule::Consent, 0, 1, TID),
            SOPHIA,
        )
        .unwrap();
        let pid = (SOPHIA, 0);
        c.deliver(ratify_vote(pid, Ballot::Approve, 1, 1, TID), NYE)
            .unwrap();
        c.deliver(ratify_vote(pid, Ballot::Reject, 2, 1, TID), TOM)
            .unwrap(); // Tom vetoes
        c.deliver(ratify_vote(pid, Ballot::Approve, 3, 1, TID), LAUREN)
            .unwrap();
        c.deliver(ratify_close(pid, 1, 0, GENESIS_PREV, TID), SOPHIA)
            .unwrap();
        let rs = c.ratify_state();
        assert_eq!(rs.outcome(&pid, &SOPHIA), Outcome::Failed);
        assert!(
            rs.ratified(&SOPHIA).is_empty(),
            "a failed proposal commits nothing"
        );
    }

    #[test]
    fn abstain_and_silence_do_not_block_consent() {
        let mut c = family();
        c.deliver(
            ratify_propose("family movie night friday", Rule::Consent, 0, 1, TID),
            SOPHIA,
        )
        .unwrap();
        let pid = (SOPHIA, 0);
        c.deliver(ratify_vote(pid, Ballot::Abstain, 1, 1, TID), NYE)
            .unwrap(); // abstains
                       // Tom stays silent; Lauren approves.
        c.deliver(ratify_vote(pid, Ballot::Approve, 2, 1, TID), LAUREN)
            .unwrap();
        c.deliver(ratify_close(pid, 1, 0, GENESIS_PREV, TID), SOPHIA)
            .unwrap();
        assert_eq!(c.ratify_state().outcome(&pid, &SOPHIA), Outcome::Passed);
    }

    #[test]
    fn solo_owner_ratifies_immediately() {
        // A group of one IS its own owner — consent is vacuous, close passes.
        let mut c = Coordinator::<ForumType>::new(vec![SOPHIA], SOPHIA);
        c.deliver(
            ratify_propose("note to self: fix the bike", Rule::Consent, 0, 1, TID),
            SOPHIA,
        )
        .unwrap();
        let pid = (SOPHIA, 0);
        c.deliver(ratify_close(pid, 1, 0, GENESIS_PREV, TID), SOPHIA)
            .unwrap();
        let rs = c.ratify_state();
        assert_eq!(rs.outcome(&pid, &SOPHIA), Outcome::Passed);
        assert_eq!(
            rs.ratified(&SOPHIA),
            vec!["note to self: fix the bike".to_string()]
        );
    }

    #[test]
    fn owner_approval_rule_gates_on_the_owner() {
        // A member proposes under OwnerApproval; the owner must approve for it to pass.
        let mut c = family();
        c.deliver(
            ratify_propose("add a stakeholder", Rule::OwnerApproval, 0, 1, TID),
            NYE,
        )
        .unwrap();
        let pid = (NYE, 0);
        c.deliver(ratify_vote(pid, Ballot::Approve, 1, 1, TID), TOM)
            .unwrap();
        c.deliver(ratify_vote(pid, Ballot::Approve, 2, 1, TID), LAUREN)
            .unwrap();
        // Sophia (owner) closes WITHOUT approving → fails despite two member approvals.
        c.deliver(ratify_close(pid, 1, 0, GENESIS_PREV, TID), SOPHIA)
            .unwrap();
        assert_eq!(c.ratify_state().outcome(&pid, &SOPHIA), Outcome::Failed);

        // Now the owner approves before close → passes.
        let mut c2 = family();
        c2.deliver(
            ratify_propose("add a stakeholder", Rule::OwnerApproval, 0, 1, TID),
            NYE,
        )
        .unwrap();
        c2.deliver(ratify_vote(pid, Ballot::Approve, 1, 1, TID), SOPHIA)
            .unwrap();
        c2.deliver(ratify_close(pid, 1, 0, GENESIS_PREV, TID), SOPHIA)
            .unwrap();
        assert_eq!(c2.ratify_state().outcome(&pid, &SOPHIA), Outcome::Passed);
    }

    #[test]
    fn a_recast_ballot_is_last_write_wins() {
        let mut c = family();
        c.deliver(
            ratify_propose("paint the fence", Rule::Consent, 0, 1, TID),
            SOPHIA,
        )
        .unwrap();
        let pid = (SOPHIA, 0);
        // Tom rejects, then changes his mind (higher gen) — the approve wins.
        c.deliver(ratify_vote(pid, Ballot::Reject, 1, 1, TID), TOM)
            .unwrap();
        c.deliver(ratify_vote(pid, Ballot::Approve, 5, 1, TID), TOM)
            .unwrap();
        c.deliver(ratify_close(pid, 1, 0, GENESIS_PREV, TID), SOPHIA)
            .unwrap();
        assert_eq!(c.ratify_state().outcome(&pid, &SOPHIA), Outcome::Passed);
    }

    #[test]
    fn converges_regardless_of_delivery_order() {
        let pid = (SOPHIA, 0);
        let propose = ratify_propose("blue car -> Wales", Rule::Consent, 0, 1, TID);
        let vn = ratify_vote(pid, Ballot::Approve, 1, 1, TID);
        let vt = ratify_vote(pid, Ballot::Approve, 2, 1, TID);
        let vl = ratify_vote(pid, Ballot::Approve, 3, 1, TID);
        let close = ratify_close(pid, 1, 0, GENESIS_PREV, TID);

        let mut c1 = family();
        c1.deliver(propose.clone(), SOPHIA).unwrap();
        c1.deliver(vn.clone(), NYE).unwrap();
        c1.deliver(vt.clone(), TOM).unwrap();
        c1.deliver(vl.clone(), LAUREN).unwrap();
        c1.deliver(close.clone(), SOPHIA).unwrap();

        // Adversarial: close and ballots arrive before the proposal folds.
        let mut c2 = family();
        c2.deliver(close, SOPHIA).unwrap();
        c2.deliver(vl, LAUREN).unwrap();
        c2.deliver(vt, TOM).unwrap();
        c2.deliver(vn, NYE).unwrap();
        c2.deliver(propose, SOPHIA).unwrap();

        let (r1, r2) = (c1.ratify_state(), c2.ratify_state());
        assert_eq!(r1.outcome(&pid, &SOPHIA), Outcome::Passed);
        assert_eq!(r1.outcome(&pid, &SOPHIA), r2.outcome(&pid, &SOPHIA));
        assert_eq!(r1.ratified(&SOPHIA), r2.ratified(&SOPHIA));
    }

    /// WHAT `RATIFY_OPS` SAYS IS WHAT `deliver` DOES — checked from the enforcement
    /// end, because that table is now what `icd.rs` holds the ICD to. A row claiming
    /// an authority the Coordinator does not enforce would make the document wrong in
    /// the one way the document can never reveal: both sides would agree, and both
    /// would be describing something the wire does not do.
    #[test]
    fn the_declared_authority_is_the_authority_deliver_enforces() {
        // The spec invariant first: a commutative op must be any-member, since a
        // broadcast op cannot be owner-gated before it propagates.
        for d in RATIFY_OPS.iter() {
            assert!(
                d.is_well_formed(),
                "`{}` is declared commutative AND owner-gated",
                d.name
            );
        }

        let mut c = family();
        c.deliver(
            ratify_propose("sell the van", Rule::Consent, 0, 1, TID),
            SOPHIA,
        )
        .unwrap();
        let pid = (SOPHIA, 0);

        // `ratify.close` is declared owner/sequenced — so the spine refuses a member's
        // close. Nothing in the close builder enforces this; the declaration routes the
        // delta to the arm that does.
        assert_eq!(
            c.deliver(ratify_close(pid, 1, 0, GENESIS_PREV, TID), NYE),
            Err(DeltaRejection::Unauthorized),
            "a member closed the owner's ratification"
        );

        // `ratify.propose`/`ratify.vote` are declared any-member/commutative — any
        // MEMBER, not anyone. Since membership-through-mls.md §10.1 that is decided
        // by MLS at ingest, not re-asked at deliver (REVERSED ON PURPOSE, §15.6: this
        // used to assert the stranger's ballot was refused here). What keeps a
        // ballot from outside the roster from counting is the TALLY: the open tally
        // counts current members only, and a frozen close counts only its electorate.
        const STRANGER: MemberId = [9u8; 32];
        c.deliver(ratify_vote(pid, Ballot::Reject, 1, 1, TID), STRANGER)
            .expect("the log holds it — MLS, not the fold, decides who reached the log");
        assert!(
            !c.ratify_state().votes[&pid].contains_key(&STRANGER),
            "a ballot from outside the roster does not count in the open tally"
        );
        c.deliver(ratify_vote(pid, Ballot::Approve, 2, 1, TID), NYE)
            .expect("a member votes");
    }
}
