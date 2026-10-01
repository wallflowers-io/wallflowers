//! The base NOTE op-group — a person's notes as folded state on a GroupObject.
//!
//! # Why this exists
//!
//! Notes were the one thing a person writes that lived outside the object model:
//! one `union.lode` document per note, referenced by nothing in the archive or
//! the backup, both since removed (5636ac4, a58798c). Anything that is not an
//! object on a kind [`crate::fold::lens_for`] names does not fold and is not in
//! the fold digest. **A note that is a GroupObject is in both by construction**,
//! with nothing extra to remember. That coupling is the whole reason this module
//! exists rather than a second storage path.
//!
//! # Shape — a note is an ENTRY, not an object, until it is shared
//!
//! ```text
//!   notebook   a group of ONE, lazily minted, holding every private note
//!   note       a group of N, minted at share time, holding exactly one note
//! ```
//!
//! Both kinds are Group-typed ([`crate::group::GROUP_TYPED_KINDS`]), so this
//! facet splices into `GROUP_OPS` exactly as `publication` and `wallet` do, and
//! both fold through `ObjectKind::Group`. A group of 1 never publishes
//! (`append_and_flush` early-returns at `members <= 1`), so a private note has no
//! transport copy *in principle* — the Arc snapshot is its only durability, which
//! is precisely why this had to move inside the archive.
//!
//! # Every op is AnyMember + Commutative, and that is FORCED, not chosen
//!
//! [`crate::object::OpDecl::is_well_formed`] requires a commutative op to be
//! any-member, and co-editing a shared note requires any-member authoring. So the
//! reducer re-checks membership itself, as every commutative reducer must.
//!
//! # The keyspace IS the permission model
//!
//! Every slot is keyed `(author, note)`. Only the author can write or retract
//! their own entry, because nobody else's delta can land in their slot — that is
//! STRUCTURAL, not a permission check. Removing someone *else's* words is a
//! MEMBERSHIP act, which is membership == access doing the work.
//!
//! # Ordering
//!
//! Last-write-wins per slot on a total key, lifted from `place.post`. `max` over a
//! totally-ordered key is associative and commutative, so the fold is
//! order-independent under any delivery order — which is the requirement
//! (`object_store`: "State is therefore a function of the delta SET, never of
//! arrival order"). `gen` plays no part: the outcome is decided by the author's
//! own `at`, so it is `gen`-independent.

use std::collections::BTreeMap;

use crate::coordinator::{ArgVal, Args};
use crate::object::{
    Authority, Commutativity, DeltaRejection, MemberId, ObjectKind, ObjectType, Op, OpDecl,
};
use crate::object_args::{opt_int, opt_text, req_int, req_text};

// ---- op ids ------------------------------------------------------------------

/// Reserved base-op band, beside geo's `0xF000_xxxx`, membership's `0xF001_xxxx`,
/// visibility's `0xF002_xxxx`, publication's `0xF003_xxxx` and wallet's
/// `0xF004_xxxx`. `note_owns_its_band` pins it.
pub const OP_NOTE_WRITE: u32 = 0xF005_0000;
pub const OP_NOTE_RETRACT: u32 = 0xF005_0001;
pub const OP_NOTE_COMMENT: u32 = 0xF005_0002;
pub const OP_NOTE_REACT: u32 = 0xF005_0003;
pub const OP_NOTE_PROMOTE: u32 = 0xF005_0004;

// ---- caps, enforced at the fold ----------------------------------------------
//
// The log is NEVER pruned (`append_delta` is INSERT OR IGNORE and no code path
// deletes from `delta_log`), so an uncapped body is an uncapped backup: every
// revision is a permanent full copy in the log AND in every segment shipped to
// the Arc. The caps follow the house precedent (`post::MAX_COMMENT_CHARS`).

/// A note is prose. 64 KiB is ~10k words.
pub const MAX_NOTE_TEXT: usize = 64 * 1024;
pub const MAX_NOTE_TITLE: usize = 512;
/// Exactly `post::MAX_COMMENT_CHARS` — one rule in two places.
pub const MAX_COMMENT_CHARS: usize = 2_000;
pub const MAX_EMOJI_BYTES: usize = 32;
/// A note id is a UUID from the app; anything longer is a caller bug.
pub const MAX_NOTE_ID: usize = 128;

/// `at` is unix MILLISECONDS and is BOUNDED. Unbounded, one op dated 2099 would
/// win its slot forever: on a shared note a skewed `noteWrite` would beat every
/// other member's entry permanently, and a skewed `noteRetract` would hide the
/// note for everyone. The bound is a fixed constant rather than a wall-clock
/// comparison so that every replica folds the same delta set to the same state —
/// a clock read inside a reducer would break replica equality.
///
/// 2100-01-01T00:00:00Z.
pub const MAX_AT_MS: i64 = 4_102_444_800_000;

// ---- state -------------------------------------------------------------------

/// One author's current entry for one note, keyed `(author, note)`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NoteEntry {
    pub note: String,
    pub title: String,
    pub text: String,
    /// The AUTHOR's timestamp, unix ms. Rides every op because `import_archive`
    /// overwrites `received_at` with `now` for every restored row and the
    /// envelope carries no timestamp — without it, every restored note's clock
    /// would collapse to restore time.
    pub at: i64,
    pub created: i64,
    /// The note's stable id in its previous life, so an old `place.post` row and
    /// the journal stay resolvable through one identity.
    pub source: String,
    /// A tombstone. The entry keeps its slot so that a later write by the same
    /// author legitimately resurrects it.
    pub retracted: bool,
}

/// One comment, keyed `(author, commentId)` — so two authors may use the same
/// comment id without colliding.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NoteComment {
    pub note: String,
    pub comment: String,
    pub text: String,
    pub at: i64,
}

/// One author's reaction to one note, keyed `(author, note)`. An empty `emoji` is
/// the CLEARED state rather than a removal: a removal would need a tombstone to
/// stay commutative, and an empty string already is one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NoteReaction {
    pub note: String,
    pub emoji: String,
    pub at: i64,
}

/// Where a private note's SHARED twin lives, keyed `(author, note)`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotePromotion {
    pub note: String,
    pub object: String,
    pub at: i64,
}

/// The folded notebook — the facet a Group-typed object carries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NoteBook {
    pub entries: BTreeMap<(MemberId, String), NoteEntry>,
    pub comments: BTreeMap<(MemberId, String), NoteComment>,
    pub reactions: BTreeMap<(MemberId, String), NoteReaction>,
    pub promoted: BTreeMap<(MemberId, String), NotePromotion>,
    /// What this object is a part of, and in what role (`base.setParent`). `None`
    /// until a parent names it as a part.
    pub parent: Option<crate::parent::ParentRef>,
}

impl NoteBook {
    /// The winning entry for `note` ACROSS authors: the greatest `(at, text)`
    /// among entries that are not retracted. A co-edited note therefore reads as
    /// ONE document with a deterministic winner, and the losers are prior
    /// versions rather than deletions — [`NoteBook::versions`] still returns them.
    pub fn current(&self, note: &str) -> Option<&NoteEntry> {
        self.entries
            .values()
            .filter(|e| e.note == note && !e.retracted)
            .max_by(|a, b| (a.at, &a.text).cmp(&(b.at, &b.text)))
    }

    /// Every author's entry for `note`, attributed, oldest first. Nobody's words
    /// are destroyed by someone else winning the slot.
    pub fn versions(&self, note: &str) -> Vec<(&MemberId, &NoteEntry)> {
        let mut v: Vec<_> = self
            .entries
            .iter()
            .filter(|((_, _), e)| e.note == note)
            .map(|((who, _), e)| (who, e))
            .collect();
        v.sort_by(|a, b| (a.1.at, &a.1.text).cmp(&(b.1.at, &b.1.text)));
        v
    }

    /// Every note id this book holds, sorted and deduplicated.
    pub fn note_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.entries.values().map(|e| e.note.clone()).collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// Every note that has a live (non-retracted) winner, sorted by id.
    pub fn live(&self) -> Vec<&NoteEntry> {
        self.note_ids()
            .into_iter()
            .filter_map(|id| self.current(&id))
            .collect()
    }
}

// ---- op table ----------------------------------------------------------------

/// The base note ops, to be included in a type's `ops()`.
///
/// All five are any-member/commutative. That is forced by
/// [`crate::object::OpDecl::is_well_formed`] — a commutative op MUST be
/// any-member — and by co-editing, which requires any member to be able to
/// author. The per-author keyspace is what keeps "only you may retract your own
/// words" true without an authority rung.
pub static NOTE_OPS: &[OpDecl] = &[
    OpDecl {
        op_id: OP_NOTE_WRITE,
        name: "base.noteWrite",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_NOTE_RETRACT,
        name: "base.noteRetract",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_NOTE_COMMENT,
        name: "base.noteComment",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_NOTE_REACT,
        name: "base.noteReact",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_NOTE_PROMOTE,
        name: "base.notePromote",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
];

/// True if `op_id` is a base note op — so an object can route it to
/// [`reduce_note`] before its own op match.
pub fn is_note_op(op_id: u32) -> bool {
    matches!(
        op_id,
        OP_NOTE_WRITE | OP_NOTE_RETRACT | OP_NOTE_COMMENT | OP_NOTE_REACT | OP_NOTE_PROMOTE
    )
}

// ---- arg validation ----------------------------------------------------------

fn note_id(args: &Args) -> Result<String, DeltaRejection> {
    let n = req_text(args, "note")?;
    if n.is_empty() || n.len() > MAX_NOTE_ID {
        return Err(DeltaRejection::MalformedArgs);
    }
    Ok(n.to_string())
}

fn at_ms(args: &Args) -> Result<i64, DeltaRejection> {
    let at = req_int(args, "at")?;
    if at <= 0 || at > MAX_AT_MS {
        return Err(DeltaRejection::MalformedArgs);
    }
    Ok(at)
}

fn capped(s: &str, max: usize) -> Result<String, DeltaRejection> {
    if s.len() > max {
        return Err(DeltaRejection::MalformedArgs);
    }
    Ok(s.to_string())
}

/// The total order on a slot. A retract at the same `at` as a write beats it, so
/// the winner never depends on delivery order.
fn entry_key(e: &NoteEntry) -> (i64, u8, &str) {
    (e.at, e.retracted as u8, e.text.as_str())
}

// ---- the reducer -------------------------------------------------------------

/// Fold one base note op into an object's notebook facet.
///
/// EVERY arg is validated BEFORE the first mutation: a reduce-time rejection is
/// skipped at fold, so a half-applied write would leave a note holding one
/// author's title over another's body as its permanent state (the reducer's
/// atomicity contract, as `place.rs` states it).
pub fn reduce_note(book: &mut NoteBook, op: &Op<'_>) -> Result<(), DeltaRejection> {
    // WHETHER THE AUTHOR WAS A MEMBER is not re-asked here. MLS answered it at
    // ingest, for the epoch the entry was written in (membership-through-mls.md
    // §10.1); asking today's roster would erase a departed co-author's words from a
    // shared note. The per-author keyspace is still what stops anyone writing into
    // another author's slot.
    let args = op.args;
    match op.op_id {
        OP_NOTE_WRITE => {
            let note = note_id(args)?;
            let at = at_ms(args)?;
            let text = capped(req_text(args, "text")?, MAX_NOTE_TEXT)?;
            let title = capped(&opt_text(args, "title").unwrap_or_default(), MAX_NOTE_TITLE)?;
            let source = capped(&opt_text(args, "source").unwrap_or_default(), MAX_NOTE_ID)?;
            let created = match opt_int(args, "created") {
                Some(c) if c <= 0 || c > MAX_AT_MS => return Err(DeltaRejection::MalformedArgs),
                Some(c) => c,
                None => at,
            };
            let cand = NoteEntry {
                note: note.clone(),
                title,
                text,
                at,
                created,
                source,
                retracted: false,
            };
            put_entry(book, (*op.author, note), cand);
        }
        OP_NOTE_RETRACT => {
            let note = note_id(args)?;
            let at = at_ms(args)?;
            // A TOMBSTONE, carrying nothing from the entry it supersedes. It must
            // not copy the current text: the candidate would then depend on what
            // had already been delivered, and two replicas folding the same set in
            // different orders would disagree. Empty text is the honest tombstone,
            // and a later write by the same author resurrects the slot.
            let cand = NoteEntry {
                note: note.clone(),
                at,
                retracted: true,
                ..NoteEntry::default()
            };
            put_entry(book, (*op.author, note), cand);
        }
        OP_NOTE_COMMENT => {
            let note = note_id(args)?;
            let at = at_ms(args)?;
            let comment = capped(req_text(args, "comment")?, MAX_NOTE_ID)?;
            if comment.is_empty() {
                return Err(DeltaRejection::MalformedArgs);
            }
            let text = req_text(args, "text")?;
            if text.chars().count() > MAX_COMMENT_CHARS {
                return Err(DeltaRejection::MalformedArgs);
            }
            let cand = NoteComment {
                note,
                comment: comment.clone(),
                text: text.to_string(),
                at,
            };
            let key = (*op.author, comment);
            let replace = match book.comments.get(&key) {
                Some(old) => (cand.at, &cand.text) > (old.at, &old.text),
                None => true,
            };
            if replace {
                book.comments.insert(key, cand);
            }
        }
        OP_NOTE_REACT => {
            let note = note_id(args)?;
            let at = at_ms(args)?;
            let emoji = capped(req_text(args, "emoji")?, MAX_EMOJI_BYTES)?;
            let cand = NoteReaction {
                note: note.clone(),
                emoji,
                at,
            };
            let key = (*op.author, note);
            let replace = match book.reactions.get(&key) {
                Some(old) => (cand.at, &cand.emoji) > (old.at, &old.emoji),
                None => true,
            };
            if replace {
                book.reactions.insert(key, cand);
            }
        }
        OP_NOTE_PROMOTE => {
            let note = note_id(args)?;
            let at = at_ms(args)?;
            let object = crate::object_args::req_hex_id(args, "object")?;
            let cand = NotePromotion {
                note: note.clone(),
                object,
                at,
            };
            let key = (*op.author, note);
            let replace = match book.promoted.get(&key) {
                Some(old) => (cand.at, &cand.object) > (old.at, &old.object),
                None => true,
            };
            if replace {
                book.promoted.insert(key, cand);
            }
        }
        _ => return Err(DeltaRejection::UnknownType),
    }
    Ok(())
}

fn put_entry(book: &mut NoteBook, key: (MemberId, String), cand: NoteEntry) {
    let replace = match book.entries.get(&key) {
        Some(old) => entry_key(&cand) > entry_key(old),
        None => true,
    };
    if replace {
        book.entries.insert(key, cand);
    }
}

// ---- arg builders ------------------------------------------------------------
//
// The wire vocabulary in ONE place, so the FFI, the tests and the ICD cannot
// drift into three spellings of the same field.

fn text(m: &mut Args, k: &str, v: &str) {
    m.insert(k.to_string(), ArgVal::Text(v.to_string()));
}
fn int(m: &mut Args, k: &str, v: i64) {
    m.insert(k.to_string(), ArgVal::Int(v));
}

pub fn note_write_args(
    note: &str,
    title: &str,
    body: &str,
    at: i64,
    created: Option<i64>,
    source: &str,
) -> Args {
    let mut m = Args::new();
    text(&mut m, "note", note);
    text(&mut m, "title", title);
    text(&mut m, "text", body);
    int(&mut m, "at", at);
    if let Some(c) = created {
        int(&mut m, "created", c);
    }
    if !source.is_empty() {
        text(&mut m, "source", source);
    }
    m
}

pub fn note_retract_args(note: &str, at: i64) -> Args {
    let mut m = Args::new();
    text(&mut m, "note", note);
    int(&mut m, "at", at);
    m
}

pub fn note_comment_args(note: &str, comment: &str, body: &str, at: i64) -> Args {
    let mut m = Args::new();
    text(&mut m, "note", note);
    text(&mut m, "comment", comment);
    text(&mut m, "text", body);
    int(&mut m, "at", at);
    m
}

pub fn note_react_args(note: &str, emoji: &str, at: i64) -> Args {
    let mut m = Args::new();
    text(&mut m, "note", note);
    text(&mut m, "emoji", emoji);
    int(&mut m, "at", at);
    m
}

pub fn note_promote_args(note: &str, object: &str, at: i64) -> Args {
    let mut m = Args::new();
    text(&mut m, "note", note);
    text(&mut m, "object", object);
    int(&mut m, "at", at);
    m
}

// ---- tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::ReduceContext;

    const ADA: MemberId = [1u8; 32];
    const BOB: MemberId = [2u8; 32];
    const EVE: MemberId = [9u8; 32];

    fn ctx() -> (Vec<MemberId>, MemberId) {
        (vec![ADA, BOB], ADA)
    }

    /// Apply one op directly to the reducer, as `wallet.rs` unit-tests its facet.
    fn apply(book: &mut NoteBook, op_id: u32, args: &Args, who: &MemberId) -> Result<(), DeltaRejection> {
        let (members, owner) = ctx();
        let rc = ReduceContext {
            members: &members,
            owner,
            epoch: 1,
        };
        let op = Op {
            op_id,
            args,
            author: who,
            pos: None,
            ctx: &rc,
        };
        reduce_note(book, &op)
    }

    fn write(book: &mut NoteBook, who: &MemberId, note: &str, body: &str, at: i64) -> Result<(), DeltaRejection> {
        apply(book, OP_NOTE_WRITE, &note_write_args(note, "T", body, at, None, ""), who)
    }

    #[test]
    fn note_owns_its_band() {
        for d in NOTE_OPS {
            assert_eq!(d.op_id & 0xFFFF_0000, 0xF005_0000, "note owns 0xF005");
        }
    }

    /// The spec invariant, checked at registration rather than discovered at fold.
    #[test]
    fn every_note_op_is_well_formed() {
        for d in NOTE_OPS {
            assert!(d.is_well_formed(), "{} is not well formed", d.name);
            assert_eq!(d.authority, Authority::AnyMember);
            assert_eq!(d.commutativity, Commutativity::Commutative);
        }
    }

    #[test]
    fn a_note_writes_and_reads_back() {
        let mut b = NoteBook::default();
        write(&mut b, &ADA, "n1", "hello", 1_000).unwrap();
        assert_eq!(b.current("n1").unwrap().text, "hello");
        assert_eq!(b.note_ids(), vec!["n1".to_string()]);
        assert_eq!(b.live().len(), 1);
    }

    #[test]
    fn later_at_wins_the_slot() {
        let mut b = NoteBook::default();
        write(&mut b, &ADA, "n1", "first", 1_000).unwrap();
        write(&mut b, &ADA, "n1", "second", 2_000).unwrap();
        assert_eq!(b.current("n1").unwrap().text, "second");
        // An older delta arriving late does NOT clobber the newer one.
        write(&mut b, &ADA, "n1", "stale", 500).unwrap();
        assert_eq!(b.current("n1").unwrap().text, "second");
    }

    /// The whole requirement: state is a function of the delta SET, not of
    /// arrival order.
    #[test]
    fn the_fold_is_order_independent() {
        let ops: Vec<(MemberId, &str, i64)> = vec![
            (ADA, "a1", 3_000),
            (BOB, "b1", 1_000),
            (ADA, "a2", 5_000),
            (BOB, "b2", 4_000),
        ];
        let mut forward = NoteBook::default();
        for (who, body, at) in ops.iter() {
            write(&mut forward, who, "n1", body, *at).unwrap();
        }
        let mut backward = NoteBook::default();
        for (who, body, at) in ops.iter().rev() {
            write(&mut backward, who, "n1", body, *at).unwrap();
        }
        assert_eq!(forward, backward);
        assert_eq!(forward.current("n1").unwrap().text, "a2");
    }

    #[test]
    fn retract_hides_then_a_later_write_resurrects() {
        let mut b = NoteBook::default();
        write(&mut b, &ADA, "n1", "hello", 1_000).unwrap();
        apply(&mut b, OP_NOTE_RETRACT, &note_retract_args("n1", 2_000), &ADA).unwrap();
        assert!(b.current("n1").is_none(), "a retracted note has no live winner");
        assert_eq!(b.note_ids(), vec!["n1".to_string()], "the slot is kept");
        write(&mut b, &ADA, "n1", "back", 3_000).unwrap();
        assert_eq!(b.current("n1").unwrap().text, "back");
    }

    /// Retract-vs-write at the same instant must not depend on arrival order.
    #[test]
    fn retract_beats_a_write_at_the_same_instant_either_way_round() {
        let mut a = NoteBook::default();
        write(&mut a, &ADA, "n1", "hello", 1_000).unwrap();
        apply(&mut a, OP_NOTE_RETRACT, &note_retract_args("n1", 1_000), &ADA).unwrap();
        let mut b = NoteBook::default();
        apply(&mut b, OP_NOTE_RETRACT, &note_retract_args("n1", 1_000), &ADA).unwrap();
        write(&mut b, &ADA, "n1", "hello", 1_000).unwrap();
        assert_eq!(a, b);
        assert!(a.current("n1").is_none());
    }

    /// The per-author keyspace IS the permission model: Bob cannot touch Ada's
    /// entry, however late or however high his clock.
    #[test]
    fn one_member_cannot_overwrite_anothers_entry() {
        let mut b = NoteBook::default();
        write(&mut b, &ADA, "n1", "ada's words", 1_000).unwrap();
        write(&mut b, &BOB, "n1", "bob's words", 9_000).unwrap();
        // Both survive, attributed. Bob wins the cross-author display only.
        assert_eq!(b.versions("n1").len(), 2);
        assert_eq!(b.current("n1").unwrap().text, "bob's words");
        // Ada's own entry is untouched in her own slot.
        let ada = b.entries.get(&(ADA, "n1".to_string())).unwrap();
        assert_eq!(ada.text, "ada's words");
        // And Bob retracting retracts only HIS entry — Ada's becomes the winner.
        apply(&mut b, OP_NOTE_RETRACT, &note_retract_args("n1", 10_000), &BOB).unwrap();
        assert_eq!(b.current("n1").unwrap().text, "ada's words");
    }

    /// REVERSED ON PURPOSE (membership-through-mls.md §15.6). This used to assert
    /// that an author off the CURRENT roster is rejected — which, once removal exists,
    /// would erase a departed co-author's words from every shared note they wrote in.
    /// Whether an author was a member is decided once, by MLS, at ingest (§10.1); a
    /// stranger never reaches the fold, because a stranger cannot produce an MLS
    /// message the group will decrypt. What the reducer still guarantees is the
    /// per-author keyspace: EVE's entry lands in EVE's slot and nobody else's.
    #[test]
    fn a_departed_co_authors_words_still_fold() {
        let mut b = NoteBook::default();
        write(&mut b, &EVE, "n1", "written before leaving", 1_000).unwrap();
        assert_eq!(b.current("n1").unwrap().text, "written before leaving");
        assert!(b.entries.keys().all(|(author, _)| *author == EVE), "her slot, and only hers");
    }

    /// An unbounded `at` would freeze a slot forever — on a shared note that
    /// silently deletes every co-author's words.
    #[test]
    fn an_out_of_range_at_is_rejected() {
        let mut b = NoteBook::default();
        assert_eq!(
            write(&mut b, &ADA, "n1", "from 2099", MAX_AT_MS + 1),
            Err(DeltaRejection::MalformedArgs)
        );
        assert_eq!(write(&mut b, &ADA, "n1", "epoch zero", 0), Err(DeltaRejection::MalformedArgs));
        assert_eq!(write(&mut b, &ADA, "n1", "negative", -1), Err(DeltaRejection::MalformedArgs));
        assert!(b.entries.is_empty());
    }

    #[test]
    fn oversized_args_are_rejected_and_leave_the_slot_intact() {
        let mut b = NoteBook::default();
        write(&mut b, &ADA, "n1", "good", 1_000).unwrap();
        let huge = "x".repeat(MAX_NOTE_TEXT + 1);
        assert_eq!(
            write(&mut b, &ADA, "n1", &huge, 2_000),
            Err(DeltaRejection::MalformedArgs)
        );
        // Atomicity: the rejected op left the previous entry exactly as it was.
        assert_eq!(b.current("n1").unwrap().text, "good");
    }

    #[test]
    fn a_missing_note_id_or_at_is_malformed() {
        let mut b = NoteBook::default();
        let mut m = Args::new();
        m.insert("text".into(), ArgVal::Text("orphan".into()));
        assert_eq!(apply(&mut b, OP_NOTE_WRITE, &m, &ADA), Err(DeltaRejection::MalformedArgs));
    }

    #[test]
    fn comments_reactions_and_promotions_fold() {
        let mut b = NoteBook::default();
        write(&mut b, &ADA, "n1", "hello", 1_000).unwrap();
        apply(&mut b, OP_NOTE_COMMENT, &note_comment_args("n1", "c1", "nice", 2_000), &BOB).unwrap();
        apply(&mut b, OP_NOTE_REACT, &note_react_args("n1", "🔥", 2_000), &BOB).unwrap();
        let obj = "ab".repeat(16);
        apply(&mut b, OP_NOTE_PROMOTE, &note_promote_args("n1", &obj, 3_000), &ADA).unwrap();
        assert_eq!(b.comments.len(), 1);
        assert_eq!(b.reactions.get(&(BOB, "n1".into())).unwrap().emoji, "🔥");
        assert_eq!(b.promoted.get(&(ADA, "n1".into())).unwrap().object, obj);
        // An empty emoji CLEARS rather than removing the row.
        apply(&mut b, OP_NOTE_REACT, &note_react_args("n1", "", 4_000), &BOB).unwrap();
        assert_eq!(b.reactions.get(&(BOB, "n1".into())).unwrap().emoji, "");
    }

    #[test]
    fn a_promotion_must_name_a_real_object_id() {
        let mut b = NoteBook::default();
        assert_eq!(
            apply(&mut b, OP_NOTE_PROMOTE, &note_promote_args("n1", "not-hex", 1_000), &ADA),
            Err(DeltaRejection::MalformedArgs)
        );
    }

    #[test]
    fn an_unknown_op_in_the_band_is_unknown_not_silently_ignored() {
        let mut b = NoteBook::default();
        let m = note_retract_args("n1", 1_000);
        assert_eq!(apply(&mut b, 0xF005_00FF, &m, &ADA), Err(DeltaRejection::UnknownType));
        assert!(!is_note_op(0xF005_00FF));
    }
}

// ── THE KIND ───────────────────────────────────────────────────────────────
//
// A Note is a GroupObject of its own (kind 32), ruled 25 September 2026. It was
// five `base.note*` ops in a reserved band spliced onto `group`, and the ICD
// declared them `on: ["group"]` — which core had never honoured: `authoring::build`
// refused every note op unless the kind string was `notebook` or `note`, with its
// own test asserting "a note is not written onto a Site's group". The document was
// wrong, not the code, and this is the code saying so in the type system.
//
// `notebook` COLLAPSES INTO `note`. A private notebook was a group of one and a
// shared note a group of N, and they were two kind strings over one vocabulary —
// the same twinning the tethers had, for the same reason. Roster size is not a
// kind. What made them different is who is on the roster, which is where that
// fact belongs.
//
// The STATE and the REDUCER below are unchanged. What moves is the op table: this
// kind's own, numbered from 0, instead of the `0xF005` band, which is now VACATED
// and must not be reused.

/// Write or revise an entry. Per-author keyspace: nobody writes into another's slot.
pub const OP_WRITE: u32 = 0;
/// Retract your own entry. A tombstone, not a deletion.
pub const OP_RETRACT: u32 = 1;
/// Comment on an entry.
pub const OP_COMMENT: u32 = 2;
/// React to an entry.
pub const OP_REACT: u32 = 3;
/// Promote an entry into a Post — the one note op that makes another object.
pub const OP_PROMOTE: u32 = 4;

/// All five are any-member/commutative, which is FORCED by `OpDecl::is_well_formed`
/// and by co-editing: the per-author keyspace is what keeps "only you may retract
/// your own words" true with no authority rung for it.
pub static OPS: &[OpDecl] = &[
    OpDecl {
        op_id: OP_WRITE,
        name: "note.write",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_RETRACT,
        name: "note.retract",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_COMMENT,
        name: "note.comment",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_REACT,
        name: "note.react",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_PROMOTE,
        name: "note.promote",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    // The base PARENT ops: any kind can be a part (ICD `facets.parent`), and the
    // part names what it is part of so `part_of` walks from this end too.
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

/// This kind's op id → the facet id `reduce_note` still matches on. Arithmetic, not
/// a second table: both bands are contiguous from their first op.
const fn to_facet_op(op_id: u32) -> Option<u32> {
    if op_id <= OP_PROMOTE {
        Some(OP_NOTE_WRITE + op_id)
    } else {
        None
    }
}

pub struct NoteType;

impl ObjectType for NoteType {
    const KIND: ObjectKind = ObjectKind::Note;
    type State = NoteBook;

    fn ops() -> &'static [OpDecl] {
        OPS
    }

    fn reduce(state: &mut Self::State, op: &Op<'_>) -> Result<(), DeltaRejection> {
        if crate::parent::is_parent_op(op.op_id) {
            return crate::parent::reduce_parent(&mut state.parent, op);
        }
        let facet_op = to_facet_op(op.op_id).ok_or(DeltaRejection::UnknownType)?;
        let restated = Op {
            op_id: facet_op,
            args: op.args,
            author: op.author,
            pos: op.pos,
            ctx: op.ctx,
        };
        reduce_note(state, &restated)
    }
}

#[cfg(test)]
mod kind_tests {
    use super::*;

    #[test]
    fn the_table_is_contiguous_from_zero_and_maps_onto_the_facet() {
        // The kind's OWN ops, below the base band; the rest are the parent facet's.
        let own: Vec<_> = OPS.iter().filter(|d| !crate::parent::is_parent_op(d.op_id)).collect();
        for (i, decl) in own.iter().enumerate() {
            assert_eq!(decl.op_id, i as u32, "{} is out of place", decl.name);
            let facet = to_facet_op(decl.op_id).expect("mapped");
            assert_eq!(facet, OP_NOTE_WRITE + i as u32, "{} maps wrong", decl.name);
            assert!(is_note_op(facet), "{} maps to an id reduce_note refuses", decl.name);
        }
        assert_eq!(OPS.len(), own.len() + crate::parent::PARENT_OPS.len(), "an op is neither the kind's nor the parent facet's");
        assert_eq!(to_facet_op(own.len() as u32), None, "the map must not run past the table");
    }

    /// The vacated band stays vacated: reusing `0xF005` would make an old note
    /// delta fold as whatever took its id.
    #[test]
    fn the_facet_band_is_still_the_one_this_maps_onto() {
        assert_eq!(OP_NOTE_WRITE, 0xF005_0000);
        assert_eq!(OP_NOTE_PROMOTE, 0xF005_0004);
    }
}
