//! membership — the base op-group that makes a roster change an EVENT.
//!
//! # Why this exists
//!
//! Membership lives in the MLS ratchet tree, and the `group_members` table is a
//! cache of it: [`crate::directory::Directory::set_group_members`] DELETEs the rows
//! and re-INSERTs the current roster. That is a correct cache and a terrible
//! history. A member who leaves is *overwritten* — there is no delta, so there is
//! no author, no timestamp, no reason, and nothing for any consumer to observe.
//!
//! The consequences are not confined to the graph:
//!
//! - **The graph can only store a level, never an interval.** lodedb-graph is
//!   bi-temporal; `member_of` wants `valid_at`/`invalid_at`. A projector that reads
//!   only the roster sees the present and has no idea when it started, so a
//!   departure it never witnessed leaves the edge standing forever.
//! - **A departure does not sync.** Deltas replicate; a cache overwrite does not.
//!   Every device rediscovers the change independently by diffing, or does not.
//! - **Nobody can be told why.** Removed by the owner and walked out are the same
//!   silence, and the UI has nothing to render.
//!
//! So: **it is not legal to leave without emitting a delta.** These two ops are how
//! you say it.
//!
//! # This is not a second source of truth
//!
//! The ratchet tree stays authoritative — it decides who can decrypt, and no delta
//! can overrule cryptography. This log is the TRANSITION RECORD, and the two are
//! tied by [`MembershipLog::divergence`]: the folded log's current set must equal
//! the MLS roster. They cannot drift silently, because a drift is a loud error
//! naming the members involved. Read it as: the tree says who, the log says when,
//! who by, and why — and a client that commits without recording is detectable.
//!
//! # Wiring (the base op-group)
//!
//! Like [`crate::geo`], this is a BASE op-group available to every object kind, in a
//! reserved high op-id band so it can never collide with a type's own ops (which
//! number from 0). A kind that carries membership history embeds a
//! `membership: MembershipLog` in its `State`, splices [`MEMBERSHIP_OPS`] into its
//! `ops()`, and routes [`is_membership_op`] to [`reduce_membership`] from `reduce`.

use std::collections::{BTreeMap, BTreeSet};

use crate::object::{Authority, Commutativity, DeltaRejection, MemberId, Op, OpDecl};
use crate::object_args::{req_hex_id, req_int};

/// Someone joined the object's roster. Authored by whoever committed the MLS Add.
pub const OP_MEMBER_JOINED: u32 = 0xF001_0000;
/// Someone left it — walked out, or was removed. Authored by the LEAVER when they
/// walk, by the owner when they remove.
pub const OP_MEMBER_LEFT: u32 = 0xF001_0001;
/// The owner hands the object to another member. Owner/sequenced — the one
/// membership op that IS the owner's alone, because it is the only one that
/// changes who may write the spine.
pub const OP_OWNER_HANDOVER: u32 = 0xF001_0002;

/// The base table every kind splices in verbatim.
///
/// **Any member, commutative** — because leaving must not need the owner's help.
/// Only the owner may write the hash-chained sequenced spine ("only the owner
/// sequences the spine", `SequencedLog::deliver`), so a sequenced departure would
/// mean you can only leave a group whose owner is online and willing. That is not
/// how leaving a group works anywhere else, and it is not how it works here.
///
/// Commutative therefore forces the fold to be order-independent, so these deltas
/// are stored as an OR-set of EVENTS and the tenures are derived on read by
/// sorting on `at`. Delivery order cannot be relied on for a commutative op — a
/// departure may well arrive before the join it ends.
pub static MEMBERSHIP_OPS: [OpDecl; 4] = [
    OpDecl {
        op_id: OP_MEMBER_JOINED,
        name: "base.memberJoined",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_MEMBER_LEFT,
        name: "base.memberLeft",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_OWNER_HANDOVER,
        name: "base.ownerHandover",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_CLAIM_SPENT,
        name: "base.claimSpent",
        authority: Authority::OwnerOrRole(crate::group::GroupRole::Admitter),
        commutativity: Commutativity::Commutative,
    },
];

/// A kiosk claim was spent on this Add (A-3, D-53): the sha256 of its nonce, never the
/// token, and who it admitted. Written by the admitter beside the Add. Commutative, and
/// the FIRST per claim holds, so a claim admits once however many copies race; the
/// author must be the owner or an admitter (`owner|role:admitter`), checked by the
/// reducer (NC-135).
pub const OP_CLAIM_SPENT: u32 = 0xF001_0003;

/// True if `op_id` is a base membership op, so a kind can route it before its own
/// op match.
pub fn is_membership_op(op_id: u32) -> bool {
    matches!(op_id, OP_MEMBER_JOINED | OP_MEMBER_LEFT | OP_OWNER_HANDOVER | OP_CLAIM_SPENT)
}

/// How a tenure ended.
///
/// NOT a wire argument — DERIVED from who authored the departure. You left if you
/// recorded it yourself; you were removed if someone else did. That makes the
/// distinction unforgeable: nobody can dress an ejection up as a resignation, and
/// nobody can claim to have removed someone who walked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Departure {
    /// Still in the roster.
    #[default]
    Present,
    /// Walked out under their own authority.
    Left,
    /// Taken out by the owner.
    Removed,
}

impl Departure {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Left => "left",
            Self::Removed => "removed",
        }
    }
}

/// WHY someone was removed (membership-through-mls.md, amendment 8) — a closed set, so
/// a removal can say why without carrying prose into the log. Optional on the wire:
/// a leave needs none (authorship already says "left"), and an owner may decline to
/// give one. An unknown value is malformed, never guessed at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RemovalReason {
    /// The person asked to be removed, and the owner did it for them.
    Requested,
    /// A device was lost or retired; the person may still be here on another.
    LostDevice,
    /// The person stopped taking part.
    Inactive,
    /// The owner removed them for how they behaved here.
    Conduct,
    /// None of the above.
    Other,
}

impl RemovalReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::LostDevice => "lost_device",
            Self::Inactive => "inactive",
            Self::Conduct => "conduct",
            Self::Other => "other",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "requested" => Self::Requested,
            "lost_device" => Self::LostDevice,
            "inactive" => Self::Inactive,
            "conduct" => Self::Conduct,
            "other" => Self::Other,
            _ => return None,
        })
    }
}

/// One authored transition. The stored unit, because the ops are commutative and
/// an OR-set of events is the only shape that folds the same in any delivery order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Transition {
    /// Unix ms the transition happened (event time, author-supplied).
    pub at: i64,
    /// `None` = joined; `Some(_)` = left, and how.
    pub departure: Option<Departure>,
    /// Who authored it — the leaver themselves, or the owner who removed them.
    pub by: MemberId,
    /// Why, when the author said (amendment 8).
    pub reason: Option<RemovalReason>,
}

/// One continuous interval of belonging — exactly the shape a bi-temporal
/// `member_of` edge needs. DERIVED from the transitions, never stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tenure {
    /// Unix ms the member joined.
    pub joined_at: i64,
    /// Unix ms they left, or 0 while the tenure is open.
    pub left_at: i64,
    /// How it ended; `Present` while open.
    pub departure: Departure,
    /// Why it ended, when the author said.
    pub reason: Option<RemovalReason>,
}

impl Tenure {
    pub fn is_open(&self) -> bool {
        self.left_at == 0
    }
}

/// One authored transfer of ownership. Rides the sequenced spine, so the Vec is in
/// authored order and the LAST one wins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handover {
    pub at: i64,
    /// The new owner's identity pubkey (hex).
    pub to: String,
    /// The outgoing owner who authored it.
    pub by: MemberId,
}

/// Every membership transition ever authored on one object, keyed by member
/// identity pubkey (hex).
///
/// A `BTreeSet` per member, so re-delivering the same delta is idempotent and the
/// fold is identical on every replica regardless of arrival order — the two things
/// a commutative op must guarantee.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MembershipLog {
    pub transitions: BTreeMap<String, BTreeSet<Transition>>,
    /// Ownership transfers, in spine order.
    pub handovers: Vec<Handover>,
}

/// What the folded log and the MLS roster disagree about.
///
/// `pending_removal` is EXPECTED, not an error: RFC 9420 does not let a member
/// commit their own removal, so between authoring a departure and another member
/// committing the Remove proposal, the leaver is legitimately still in the tree.
/// The other two are real drift.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Divergence {
    /// In the ratchet tree, but the log never recorded them joining — someone
    /// committed an Add without authoring the delta.
    pub unrecorded: Vec<String>,
    /// Announced their departure, still in the tree: the Remove has not been
    /// committed yet. Transient and legitimate.
    pub pending_removal: Vec<String>,
    /// The log says present, but they are not in the tree — someone committed a
    /// Remove without authoring the delta.
    pub phantom: Vec<String>,
}

impl Divergence {
    /// True when something is actually WRONG, as opposed to merely in flight.
    pub fn is_drift(&self) -> bool {
        !self.unrecorded.is_empty() || !self.phantom.is_empty()
    }
}

impl MembershipLog {
    /// The intervals one member has held, oldest first.
    ///
    /// Derived by walking the transitions in `at` order: a join opens an interval,
    /// a departure closes the open one. Coherence is resolved HERE rather than at
    /// reduce time, because a commutative reducer sees deltas in canonical
    /// `(gen, author, delta_id)` order, which has nothing to do with when things
    /// happened. Incoherent input degrades rather than panicking: a departure with
    /// no open interval is ignored, a second join while open is ignored.
    pub fn tenures(&self, member_hex: &str) -> Vec<Tenure> {
        let Some(events) = self.transitions.get(member_hex) else {
            return Vec::new();
        };
        let mut out: Vec<Tenure> = Vec::new();
        for e in events {
            match e.departure {
                None => {
                    if !out.last().is_some_and(Tenure::is_open) {
                        out.push(Tenure {
                            joined_at: e.at,
                            left_at: 0,
                            departure: Departure::Present,
                            reason: None,
                        });
                    }
                }
                Some(d) => {
                    if let Some(open) = out.last_mut().filter(|t| t.is_open()) {
                        open.left_at = e.at;
                        open.departure = d;
                        open.reason = e.reason;
                    }
                }
            }
        }
        out
    }

    /// Who owns this object according to the LOG, given the genesis owner the
    /// object was created with.
    ///
    /// This is what makes ownership foldable rather than only a directory row: the
    /// directory's `owner_pk` is a cache of this, the same way `group_members` is a
    /// cache of the ratchet tree. Handovers ride the sequenced spine, so the last
    /// one authored wins and every replica computes the same answer.
    pub fn owner(&self, genesis: MemberId) -> String {
        self.handovers
            .last()
            .map_or_else(|| hex::encode(genesis), |h| h.to.clone())
    }

    /// The successor the current owner has designated, if the directory has not
    /// caught up yet. The app uses this to know a handover is in flight.
    pub fn pending_handover(&self, directory_owner: MemberId) -> Option<&Handover> {
        self.handovers
            .last()
            .filter(|h| h.to != hex::encode(directory_owner))
    }

    /// Everyone holding an open tenure.
    pub fn current(&self) -> BTreeSet<String> {
        self.transitions
            .keys()
            .filter(|m| self.is_present(m))
            .cloned()
            .collect()
    }

    pub fn is_present(&self, member_hex: &str) -> bool {
        self.tenures(member_hex).last().is_some_and(Tenure::is_open)
    }

    /// Everyone who has announced a departure — gone as far as the app is
    /// concerned, whether or not the ratchet tree has caught up.
    pub fn departed(&self, member_hex: &str) -> Option<Tenure> {
        self.tenures(member_hex)
            .into_iter()
            .filter(|t| !t.is_open())
            .next_back()
    }

    /// The tie to cryptographic truth: `None` when the folded log agrees with the
    /// ratchet tree, `Some` naming every disagreement when it does not.
    ///
    /// This is what stops the log becoming a second source of truth. It is a
    /// CHECK, never a repair — silently reconciling would recreate the overwrite
    /// this module exists to abolish, and would forge a transition nobody authored.
    pub fn divergence(&self, roster: &[MemberId]) -> Option<Divergence> {
        let tree: BTreeSet<String> = roster.iter().map(hex::encode).collect();
        let logged = self.current();
        let mut d = Divergence::default();
        for who in tree.difference(&logged) {
            // In the tree without an open tenure: either they announced a
            // departure the Remove has not caught up with, or nobody ever
            // recorded them joining.
            if self.departed(who).is_some() {
                d.pending_removal.push(who.clone());
            } else {
                d.unrecorded.push(who.clone());
            }
        }
        d.phantom = logged.difference(&tree).cloned().collect();
        if d.unrecorded.is_empty() && d.pending_removal.is_empty() && d.phantom.is_empty() {
            None
        } else {
            Some(d)
        }
    }
}

/// Fold a base membership op.
///
/// Validation is strictly PER-DELTA: authorship and well-formedness, never
/// cross-delta coherence. A commutative reducer cannot check "were they already in
/// the group" without depending on delivery order, and an order-dependent
/// commutative op is a replica divergence waiting to happen. Coherence is resolved
/// in [`MembershipLog::tenures`] instead.
pub fn reduce_membership(log: &mut MembershipLog, op: &Op<'_>) -> Result<(), DeltaRejection> {
    let member = req_hex_id(op.args, "member")?;
    let at = req_int(op.args, "at")?;
    if at <= 0 {
        // An undated transition cannot be ordered against the others, and an
        // interval with no start is not an interval.
        return Err(DeltaRejection::MalformedArgs);
    }
    let author_hex = hex::encode(op.author);
    let is_owner = *op.author == op.ctx.owner;

    if op.op_id == OP_OWNER_HANDOVER {
        // The spine already rejected any non-owner author, but reduce is also
        // reachable directly, so state the requirement rather than assume it.
        if !is_owner {
            return Err(DeltaRejection::Unauthorized);
        }
        if member == author_hex {
            // Handing the object to yourself is a no-op dressed as governance.
            return Err(DeltaRejection::PreconditionFailed);
        }
        // You cannot hand the object to someone who is not in the room.
        if !op.ctx.members.iter().any(|m| hex::encode(m) == member) {
            return Err(DeltaRejection::PreconditionFailed);
        }
        log.handovers.push(Handover {
            at,
            to: member,
            by: *op.author,
        });
        return Ok(());
    }

    let departure = match op.op_id {
        OP_MEMBER_JOINED => {
            // Recorded by whoever COMMITTED the MLS Add, alongside it
            // (membership-through-mls.md §10.3) — at a public door that is any
            // member, which is why the ICD has always said anyMember and this arm,
            // until now, did not. A record is a claim about an MLS operation, never
            // the operation: `divergence` is what compares it with the tree, and
            // `group_author` refuses to write one except from the Add door itself.
            None
        }
        OP_MEMBER_LEFT => {
            // THE RULE: you may always record your OWN departure — leaving never
            // needs the owner's permission — and the owner may record anyone's.
            // Nobody else may speak about a third party's membership.
            if member != author_hex && !is_owner {
                return Err(DeltaRejection::Unauthorized);
            }
            // SUCCESSION: the owner may not walk out of an object they still own.
            //
            // Not a nicety — `SequencedLog::deliver` rejects every author that is
            // not the owner, so an ownerless object can never accept another
            // sequenced delta: no profile change, no role change, no affiliation,
            // no forum attach, permanently. Signal refuses to let the last admin
            // leave for the same reason. Hand over first (`base.ownerHandover`),
            // then leave as an ordinary member.
            //
            // A group of ONE is exempt: there is nobody to hand it to, and an
            // object with no members left is not frozen, it is finished. Blocking
            // here would strand every solo object its owner no longer wants.
            if member == hex::encode(op.ctx.owner) && op.ctx.members.len() > 1 {
                return Err(DeltaRejection::PreconditionFailed);
            }
            // Derived, never supplied: authorship IS the distinction, so it cannot
            // be forged by an author who simply sets the field they prefer.
            Some(if member == author_hex {
                Departure::Left
            } else {
                Departure::Removed
            })
        }
        _ => return Err(DeltaRejection::UnknownType),
    };
    // Why (amendment 8): optional, from a closed set, and only on a departure.
    let reason = match crate::arg_reads::get(op.args, "reason") {
        None => None,
        Some(crate::coordinator::ArgVal::Text(t)) if departure.is_some() => {
            Some(RemovalReason::parse(t).ok_or(DeltaRejection::MalformedArgs)?)
        }
        Some(_) => return Err(DeltaRejection::MalformedArgs),
    };

    log.transitions
        .entry(member)
        .or_default()
        .insert(Transition {
            at,
            departure,
            by: *op.author,
            reason,
        });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator::{ArgVal, Args};
    use crate::object::ReduceContext;

    const OWNER: MemberId = [7u8; 32];
    const ALICE: MemberId = [0xAAu8; 32];
    const BOB: MemberId = [0xBBu8; 32];

    fn apply(
        log: &mut MembershipLog,
        op_id: u32,
        author: MemberId,
        member: MemberId,
        at: i64,
    ) -> Result<(), DeltaRejection> {
        let members = [OWNER, ALICE, BOB];
        let ctx = ReduceContext {
            members: &members,
            owner: OWNER,
            epoch: 0,
        };
        let mut a = Args::new();
        a.insert("member".into(), ArgVal::Text(hex::encode(member)));
        a.insert("at".into(), ArgVal::Int(at));
        reduce_membership(
            log,
            &Op {
                op_id,
                args: &a,
                author: &author,
                pos: None,
                ctx: &ctx,
            },
        )
    }

    /// THE RULE: leaving never needs the owner's help.
    #[test]
    fn a_member_can_record_their_own_departure() {
        let mut log = MembershipLog::default();
        apply(&mut log, OP_MEMBER_JOINED, OWNER, ALICE, 1_000).unwrap();
        apply(&mut log, OP_MEMBER_LEFT, ALICE, ALICE, 2_000)
            .expect("a member must be able to walk out unaided");

        let t = log.tenures(&hex::encode(ALICE));
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].left_at, 2_000);
        assert_eq!(
            t[0].departure,
            Departure::Left,
            "self-authored means left, not removed"
        );
        assert!(!log.is_present(&hex::encode(ALICE)));
    }

    /// Authorship IS the distinction, so neither side can be forged.
    #[test]
    fn the_owner_removing_someone_reads_as_removed() {
        let mut log = MembershipLog::default();
        apply(&mut log, OP_MEMBER_JOINED, OWNER, ALICE, 1_000).unwrap();
        apply(&mut log, OP_MEMBER_LEFT, OWNER, ALICE, 2_000).unwrap();
        assert_eq!(
            log.tenures(&hex::encode(ALICE))[0].departure,
            Departure::Removed
        );
    }

    #[test]
    fn nobody_may_speak_about_a_third_partys_membership() {
        let mut log = MembershipLog::default();
        apply(&mut log, OP_MEMBER_JOINED, OWNER, ALICE, 1_000).unwrap();
        assert_eq!(
            apply(&mut log, OP_MEMBER_LEFT, BOB, ALICE, 2_000),
            Err(DeltaRejection::Unauthorized),
            "Bob cannot evict Alice, nor claim she left"
        );
        // REVERSED ON PURPOSE (membership-through-mls.md §15.6): this used to assert
        // that only the owner may record an arrival. At a public door the committer
        // of the Add is any member, and the ICD has always said anyMember; the record
        // is a claim that `divergence` checks against the tree.
        apply(&mut log, OP_MEMBER_JOINED, BOB, BOB, 1_500)
            .expect("whoever committed the Add records the arrival");
        assert!(
            log.is_present(&hex::encode(ALICE)),
            "rejects changed nothing"
        );
    }

    /// The property a commutative op MUST have: same set in, same fold out.
    #[test]
    fn the_fold_is_independent_of_delivery_order() {
        let a = hex::encode(ALICE);
        let mut forwards = MembershipLog::default();
        apply(&mut forwards, OP_MEMBER_JOINED, OWNER, ALICE, 1_000).unwrap();
        apply(&mut forwards, OP_MEMBER_LEFT, ALICE, ALICE, 2_000).unwrap();
        apply(&mut forwards, OP_MEMBER_JOINED, OWNER, ALICE, 3_000).unwrap();

        // Same deltas, delivered backwards — as a commutative lane may well do.
        let mut backwards = MembershipLog::default();
        apply(&mut backwards, OP_MEMBER_JOINED, OWNER, ALICE, 3_000).unwrap();
        apply(&mut backwards, OP_MEMBER_LEFT, ALICE, ALICE, 2_000).unwrap();
        apply(&mut backwards, OP_MEMBER_JOINED, OWNER, ALICE, 1_000).unwrap();

        assert_eq!(forwards, backwards, "replicas must not disagree");
        assert_eq!(forwards.tenures(&a).len(), 2, "two spells, derived on read");
        assert!(forwards.is_present(&a));
        assert_eq!(forwards.tenures(&a)[0].left_at, 2_000);
    }

    #[test]
    fn redelivering_the_same_transition_is_idempotent() {
        let mut log = MembershipLog::default();
        apply(&mut log, OP_MEMBER_JOINED, OWNER, ALICE, 1_000).unwrap();
        apply(&mut log, OP_MEMBER_JOINED, OWNER, ALICE, 1_000).unwrap();
        assert_eq!(log.transitions[&hex::encode(ALICE)].len(), 1);
    }

    /// The reason this module exists: a departure is an interval, not a deletion.
    #[test]
    fn a_departure_keeps_the_interval_it_ended() {
        let a = hex::encode(ALICE);
        let mut log = MembershipLog::default();
        apply(&mut log, OP_MEMBER_JOINED, OWNER, ALICE, 10).unwrap();
        apply(&mut log, OP_MEMBER_LEFT, ALICE, ALICE, 20).unwrap();
        assert!(
            log.transitions.contains_key(&a),
            "the member must survive their own departure — the graph needs the \
             closed interval to bound member_of with invalid_at"
        );
        assert!(log.current().is_empty());
    }

    #[test]
    fn an_undated_transition_is_malformed() {
        let mut log = MembershipLog::default();
        assert_eq!(
            apply(&mut log, OP_MEMBER_JOINED, OWNER, ALICE, 0),
            Err(DeltaRejection::MalformedArgs)
        );
    }

    /// MLS will not let you commit your own removal, so "announced but still in the
    /// tree" is a legitimate in-flight state, not drift.
    #[test]
    fn an_announced_departure_awaiting_its_commit_is_not_drift() {
        let mut log = MembershipLog::default();
        apply(&mut log, OP_MEMBER_JOINED, OWNER, ALICE, 10).unwrap();
        apply(&mut log, OP_MEMBER_LEFT, ALICE, ALICE, 20).unwrap();

        let d = log.divergence(&[ALICE]).expect("tree still holds Alice");
        assert_eq!(d.pending_removal, vec![hex::encode(ALICE)]);
        assert!(
            !d.is_drift(),
            "waiting for the Remove commit is not an error"
        );

        // Once another member commits the Remove, the two agree.
        assert_eq!(log.divergence(&[]), None);
    }

    #[test]
    fn divergence_names_real_drift_in_both_directions() {
        let mut log = MembershipLog::default();
        apply(&mut log, OP_MEMBER_JOINED, OWNER, ALICE, 10).unwrap();
        assert_eq!(log.divergence(&[ALICE]), None, "log agrees with the tree");

        // In the tree, never recorded: an Add committed without a delta.
        let d = log.divergence(&[ALICE, BOB]).expect("must diverge");
        assert_eq!(d.unrecorded, vec![hex::encode(BOB)]);
        assert!(d.is_drift());

        // In the log, gone from the tree: a Remove committed without a delta.
        let d = log.divergence(&[BOB]).expect("must diverge");
        assert_eq!(d.phantom, vec![hex::encode(ALICE)]);
        assert!(d.is_drift());
    }

    /// The gap Signal closes and we had open: an ownerless object can never accept
    /// another sequenced delta, so it is frozen forever.
    #[test]
    fn the_owner_cannot_walk_out_of_an_object_they_still_own() {
        let mut log = MembershipLog::default();
        apply(&mut log, OP_MEMBER_JOINED, OWNER, ALICE, 1_000).unwrap();
        assert_eq!(
            apply(&mut log, OP_MEMBER_LEFT, OWNER, OWNER, 2_000),
            Err(DeltaRejection::PreconditionFailed),
            "hand over first — otherwise nobody can ever author a sequenced delta again"
        );
    }

    /// ...but a solo object has nobody to hand to, and an object with no members
    /// left is finished rather than frozen. Blocking here would strand every group
    /// of one its owner no longer wants.
    #[test]
    fn the_owner_of_a_group_of_one_may_simply_leave() {
        let mut log = MembershipLog::default();
        let members = [OWNER];
        let ctx = ReduceContext {
            members: &members,
            owner: OWNER,
            epoch: 0,
        };
        let mut a = Args::new();
        a.insert("member".into(), ArgVal::Text(hex::encode(OWNER)));
        a.insert("at".into(), ArgVal::Int(1_000));
        reduce_membership(
            &mut log,
            &Op {
                op_id: OP_MEMBER_LEFT,
                args: &a,
                author: &OWNER,
                pos: None,
                ctx: &ctx,
            },
        )
        .expect("nobody to hand a solo object to");
    }

    #[test]
    fn handover_moves_the_folded_owner_and_then_the_old_owner_may_leave() {
        let mut log = MembershipLog::default();
        apply(&mut log, OP_MEMBER_JOINED, OWNER, ALICE, 1_000).unwrap();
        assert_eq!(log.owner(OWNER), hex::encode(OWNER), "genesis owner");

        apply(&mut log, OP_OWNER_HANDOVER, OWNER, ALICE, 2_000).unwrap();
        assert_eq!(
            log.owner(OWNER),
            hex::encode(ALICE),
            "the fold knows the owner moved; the directory row is its cache"
        );
        assert!(
            log.pending_handover(OWNER).is_some(),
            "in flight until the directory catches up"
        );
        assert!(log.pending_handover(ALICE).is_none(), "settled");

        // With ALICE now owner, the old owner is an ordinary member and may leave.
        let members = [OWNER, ALICE, BOB];
        let ctx = ReduceContext {
            members: &members,
            owner: ALICE,
            epoch: 0,
        };
        let mut a = Args::new();
        a.insert("member".into(), ArgVal::Text(hex::encode(OWNER)));
        a.insert("at".into(), ArgVal::Int(3_000));
        reduce_membership(
            &mut log,
            &Op {
                op_id: OP_MEMBER_LEFT,
                args: &a,
                author: &OWNER,
                pos: None,
                ctx: &ctx,
            },
        )
        .expect("once handed over, the former owner leaves like anyone else");
        assert!(!log.is_present(&hex::encode(OWNER)));
    }

    #[test]
    fn handover_must_name_a_real_member_who_is_not_yourself() {
        let mut log = MembershipLog::default();
        assert_eq!(
            apply(&mut log, OP_OWNER_HANDOVER, OWNER, OWNER, 1_000),
            Err(DeltaRejection::PreconditionFailed),
            "handing to yourself is governance theatre"
        );
        let outsider = [0xCCu8; 32];
        assert_eq!(
            apply(&mut log, OP_OWNER_HANDOVER, OWNER, outsider, 1_000),
            Err(DeltaRejection::PreconditionFailed),
            "cannot hand the object to someone who is not in the room"
        );
        assert_eq!(
            apply(&mut log, OP_OWNER_HANDOVER, ALICE, BOB, 1_000),
            Err(DeltaRejection::Unauthorized),
            "only the owner hands over"
        );
    }

    #[test]
    fn base_ops_sit_in_a_reserved_band_clear_of_geo() {
        assert!(is_membership_op(OP_MEMBER_JOINED) && is_membership_op(OP_MEMBER_LEFT));
        assert!(!is_membership_op(crate::geo::OP_SET_LOCATION));
        assert!(!crate::geo::is_location_op(OP_MEMBER_JOINED));
        assert!(OP_MEMBER_JOINED > 0xF000_0001);
        for op in MEMBERSHIP_OPS.iter() {
            assert!(op.is_well_formed());
        }
    }
}
