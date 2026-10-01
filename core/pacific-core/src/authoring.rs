//! authoring — one constructor for every kind, pure, so the browser and
//! the phone write the same deltas.
//!
//! WHY IT EXISTS. Each kind had its own `*_author` door in `node.rs`, and every one
//! of them reads the directory, opens SQLite and loads an MLS group before it can
//! build a delta. None of that exists in a browser, so the browser either had no
//! door or wrote its own — and a delta a browser builds by hand is the one that
//! turns out inert, or poisons the log for every device that folds it. This is the
//! door without the platform: the object's log is HANDED IN, and what comes back is
//! a delta that has already been folded against it.
//!
//! THREE CALLS:
//!   - [`mint_ops`] — what a mint authors after the group exists, as data;
//!   - [`probe`]    — would this delta do anything on this object;
//!   - [`build`]    — the delta for an op, positioned and probed, or a refusal.
//!
//! `build` returns a delta ONLY if the probe passes, so a caller cannot skip it.
//! That is the whole point of putting the probe inside rather than beside.
//!
//! THE REFUSALS ARE THE NATIVE DOORS'. A kind that does not declare the op; a
//! membership op, which only MLS may record (§10.3); a note op onto a
//! Group-typed object that is not a note (`group_author_inner`); an owner-only op
//! from someone who is not the owner, which the sequenced spine would reject at
//! fold and so poison the log; a commutative op with no watermark (d50d9d6); and
//! whatever the reducer itself says. Each in words, with the op.
//!
//! THE OP TABLES ARE NOT RESTATED HERE. A kind's ops are `T::ops()`, which
//! `icd.rs` holds to `coordination/delta-graph.icd.json` — so an op added to the
//! ICD and its reducer is authorable here the moment it builds.

use crate::coordinator::{self, Args, ArgVal, Coordinator, Delta};
use crate::object::{
    Authority, Commutativity, DeltaRejection, LogPosition, MemberId, NextGen, ObjectKind,
    ObjectType, Op, OpDecl, ReduceContext,
};

/// Everything about the object the door needs, handed in.
#[derive(Clone, Copy, Debug)]
pub struct Ctx<'a> {
    /// The author — this device's person.
    pub me: MemberId,
    /// The epoch the delta will be sealed in.
    pub epoch: u64,
    /// The current roster, by person.
    pub members: &'a [MemberId],
    /// The owner by epoch, as `Coordinator::with_owners` takes it: `(from_epoch, owner)`.
    pub owners: &'a [(u64, MemberId)],
    /// The object's log so far, with each delta's author. The state is folded from
    /// it; nothing here is trusted from the caller that the log can answer.
    pub log: &'a [(Delta, MemberId)],
    /// The generation floor this device was handed for the object. `None` refuses
    /// every commutative op: a device that joined late holds an EMPTY log by
    /// forward secrecy, so without a floor its next gen collides with one its own
    /// person already used on another device (d50d9d6, `object::NextGen`). A device
    /// that minted the object has seen all of it, and passes `Some(0)`.
    pub watermark: Option<u64>,
}

/// Why the door would not write it — in words, with the op.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No lens folds this kind, so nothing could tell whether the delta does anything.
    NoLens { kind: String },
    /// The kind does not declare this op.
    NotDeclared { kind: String, op_id: u32 },
    /// Membership is recorded beside the MLS commit that makes it true.
    ByTheMlsDoors { op_id: u32 },
    /// An op onto a pairing channel that is not one of the legs that ride it.
    NotALeg { kind: String, op_id: u32 },
    /// Owner-only, and the author is not the owner at this epoch.
    OwnerOnly { op: &'static str },
    /// The owner's or a role holder's (`Authority::OwnerOrRole`), and the author is neither.
    OwnerOrRoleOnly { op: &'static str, role: &'static str },
    /// A role holder's alone (`Authority::Roles`), and the author holds none of them.
    RoleOnly { op: &'static str, roles: String },
    /// The kind's own rule refuses it in the state the author folds (`ObjectType::refuses`):
    /// a Transaction's state machine.
    NotNow { op: &'static str, why: String },
    /// Commutative, and this device has no generation floor for the object.
    NoWatermark { op: &'static str },
    /// The coordinator would not take it — routing, ownership of the sequence, chain.
    Structural { op_id: u32, why: DeltaRejection },
    /// The reducer refused it against the state the log folds to.
    Reducer { op_id: u32, why: DeltaRejection },
    /// This exact delta is already in the log.
    AlreadyFolded { op_id: u32 },
    /// Sequenced, and this device does not hold the object's chain: it joined after
    /// the object began and holds no head to follow (NC-65).
    ChainNotHeld { op: &'static str },
}

impl Refusal {
    /// The op this refusal is about, where there is one.
    pub fn op_id(&self) -> Option<u32> {
        match self {
            Refusal::NotDeclared { op_id, .. }
            | Refusal::ByTheMlsDoors { op_id }
            | Refusal::NotALeg { op_id, .. }
            | Refusal::Structural { op_id, .. }
            | Refusal::Reducer { op_id, .. }
            | Refusal::AlreadyFolded { op_id } => Some(*op_id),
            Refusal::NoLens { .. }
            | Refusal::OwnerOnly { .. }
            | Refusal::OwnerOrRoleOnly { .. }
            | Refusal::RoleOnly { .. }
            | Refusal::NotNow { .. }
            | Refusal::NoWatermark { .. }
            | Refusal::ChainNotHeld { .. } => None,
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::NoLens { kind } => write!(
                f,
                "'{kind}' has no lens in this build, so nothing can tell whether a delta on it does anything"
            ),
            Refusal::NotDeclared { kind, op_id } => {
                write!(f, "a '{kind}' object declares no op {op_id}")
            }
            Refusal::ByTheMlsDoors { op_id } => write!(
                f,
                "op {op_id} is a membership record, written beside the MLS commit that makes it \
                 true — leave, remove, hand over or add"
            ),
            Refusal::NotALeg { kind, op_id } => write!(
                f,
                "op {op_id} is not a contact leg — a '{kind}' carries the ticket, \
                 invite and link legs, and its prekeys and cards are written by the pairing \
                 and sync paths, not authored"
            ),
            Refusal::OwnerOnly { op } => write!(f, "'{op}' is owner-only, and the author is not the owner"),
            Refusal::OwnerOrRoleOnly { op, role } => {
                write!(f, "'{op}' is the owner's or an {role}'s, and the author is neither")
            }
            Refusal::RoleOnly { op, roles } => write!(f, "'{op}' is the {roles}'s, and the author is not"),
            Refusal::NotNow { op, why } => write!(f, "'{op}': {why}"),
            Refusal::NoWatermark { op } => write!(
                f,
                "'{op}' is commutative and this device has no generation floor for the object — its next \
                 gen could collide with one its person already used"
            ),
            Refusal::Structural { op_id, why } => write!(f, "op {op_id} would not fold: {why:?}"),
            Refusal::Reducer { op_id, why } => write!(f, "op {op_id} was refused by the reducer: {why:?}"),
            Refusal::AlreadyFolded { op_id } => write!(f, "this op {op_id} is already in the log"),
            Refusal::ChainNotHeld { .. } => write!(
                f,
                "this device joined after this object began, and can't see its current state; \
                 sign in on a device that holds it"
            ),
        }
    }
}

impl std::error::Error for Refusal {}

/// Which fold a kind string reads with. The kind STRING, not `ObjectKind`, because
/// six strings share the Group vocabulary and differ in what may be written to them
/// (`group::GROUP_TYPED_KINDS`), and a `connection` is a chat whose log is Forum's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lens {
    Group,
    Forum,
    Conversation,
    Post,
    Event,
    Place,
    Thing,
    Contact,
    Project,
    System,
    Treasury,
    Note,
    Host,
    Transaction,
}

/// The kind strings outside the Group vocabulary, and the fold each reads with.
/// With `group::GROUP_TYPED_KINDS` these are every kind this door will write to:
/// [`lens`] and [`kinds`] both read this one table.
const LENSES: &[(&str, Lens)] = &[
    ("forum", Lens::Forum),
    ("conversation", Lens::Conversation),
    ("post", Lens::Post),
    ("event", Lens::Event),
    ("place", Lens::Place),
    ("thing", Lens::Thing),
    // The pairing channel's own vocabulary, on the kind objects are actually
    // stored under. `"contact"` was the kind string here and nothing was ever
    // stored with it, so every op in the ICD's 25 channel was declared on a kind
    // that did not exist while its deltas were written onto connections.
    ("connection", Lens::Contact),
    ("project", Lens::Project),
    ("system", Lens::System),
    ("treasury", Lens::Treasury),
    // BOTH strings fold as a Note. `notebook` was a person's group of one and
    // `note` a group of N; roster size is not a kind, so they collapsed (25 Sep
    // 2026). The old string stays readable here because objects were stored
    // under it — dropping it would make every existing notebook unfoldable.
    ("note", Lens::Note),
    ("notebook", Lens::Note),
    ("host", Lens::Host),
    ("transaction", Lens::Transaction),
];

fn lens(kind: &str) -> Option<Lens> {
    if crate::group::is_group_typed(kind) {
        return Some(Lens::Group);
    }
    LENSES.iter().find(|(k, _)| *k == kind).map(|(_, l)| *l)
}

/// Every kind string this door will write to.
pub fn kinds() -> Vec<&'static str> {
    crate::group::GROUP_TYPED_KINDS
        .iter()
        .copied()
        .chain(LENSES.iter().map(|(k, _)| *k))
        .collect()
}

/// Run `$f::<T>(args…)` for the type the kind string folds with.
macro_rules! on_lens {
    ($kind:expr, $f:ident ( $($a:expr),* )) => {
        match lens($kind) {
            None => Err(Refusal::NoLens { kind: $kind.to_string() }),
            Some(Lens::Group) => $f::<crate::group::GroupType>($($a),*),
            Some(Lens::Forum) => $f::<coordinator::ForumType>($($a),*),
            Some(Lens::Conversation) => $f::<coordinator::ConversationType>($($a),*),
            Some(Lens::Post) => $f::<crate::post::PostType>($($a),*),
            Some(Lens::Event) => $f::<crate::event::EventType>($($a),*),
            Some(Lens::Place) => $f::<crate::place::PlaceType>($($a),*),
            Some(Lens::Thing) => $f::<crate::thing::ThingType>($($a),*),
            Some(Lens::Contact) => $f::<crate::contact::ContactType>($($a),*),
            Some(Lens::Project) => $f::<crate::project::ProjectType>($($a),*),
            Some(Lens::System) => $f::<crate::system::SystemType>($($a),*),
            Some(Lens::Treasury) => $f::<crate::treasury::TreasuryType>($($a),*),
            Some(Lens::Note) => $f::<crate::note::NoteType>($($a),*),
            Some(Lens::Host) => $f::<crate::host::HostType>($($a),*),
            Some(Lens::Transaction) => $f::<crate::transaction::TransactionType>($($a),*),
        }
    };
}

/// What a mint authors once the group exists, in order: the kind's op-0 profile,
/// and for a Place minted PLACED, its location. `Node::mint`, as data.
///
/// Empty for a Forum, which is minted name-only — its name rides the GroupContext,
/// and authoring an op 0 would post a message. Refused for a kind that is not
/// minted from a draft, with the reason (`mint::classify`).
pub fn mint_ops(
    kind: ObjectKind,
    draft: &crate::mint::MintDraft,
) -> Result<Vec<(u32, Args)>, crate::mint::NoMint> {
    crate::mint::classify(kind)?;
    let mut ops = Vec::new();
    if let Some(op0) = crate::mint::profile_args(kind, draft) {
        ops.push(op0);
    }
    if kind == ObjectKind::Place && draft.placed {
        let source = crate::geo::LocationSource::Fixed {
            point: crate::geo::GeoPoint::from_degrees(draft.lat, draft.lng),
        };
        ops.push((crate::geo::OP_SET_LOCATION, crate::geo::set_location_args(&source)));
    }
    Ok(ops)
}

/// Would `candidate`, authored by `ctx.me`, do anything on this object?
///
/// Two questions, both of which must say yes: would the coordinator take it — the
/// op is routed, the sequence is the owner's, the chain holds — and would the
/// reducer accept it against the state the log folds to. The first alone passes a
/// well-formed delta that the reducer then ignores; that is the inert delta.
pub fn probe(kind: &str, ctx: &Ctx, candidate: &Delta) -> Result<(), Refusal> {
    on_lens!(kind, probe_as(ctx, candidate))
}

/// The delta for `op_id` on a `kind` object, positioned and probed.
///
/// SEQUENCED ops take the next position after the folded spine head
/// (`coordinator::next_sequenced_pos`). COMMUTATIVE ops take the next Lamport
/// generation over EVERY author in the log, raised to the watermark
/// (`object::NextGen`), and carry it in `args["gen"]` as the native doors do.
pub fn build(kind: &str, op_id: u32, args: Args, ctx: &Ctx) -> Result<Delta, Refusal> {
    on_lens!(kind, build_as(kind, op_id, args, ctx))
}

/// The next Lamport generation over `log`, raised to `watermark` — the gen `build`
/// gives a commutative op, and the floor an adder hands a newcomer in its intro
/// (`IntroPayload::gen_watermark`). One function for both, so what a joiner is told
/// to start above is exactly what the adder would write next itself. The native
/// `Node::next_lamport` is the same rule over the local log.
pub fn next_gen(log: &[(Delta, MemberId)], watermark: u64) -> u64 {
    NextGen::compute(log.iter().filter_map(|(d, _)| d.gen).max(), watermark).get()
}

fn owner_at(owners: &[(u64, MemberId)], epoch: u64) -> Option<MemberId> {
    owners
        .iter()
        .filter(|(from, _)| *from <= epoch)
        .max_by_key(|(from, _)| *from)
        .or_else(|| owners.iter().min_by_key(|(from, _)| *from))
        .map(|(_, o)| *o)
}

/// The log, folded. A peer's bad delta already in it is TOLERATED: a replica
/// cannot refuse history. Only the delta being written is refused.
fn fold<T: ObjectType>(ctx: &Ctx) -> Coordinator<T> {
    let mut c = Coordinator::<T>::with_owners(ctx.members.to_vec(), ctx.owners.to_vec());
    // ORDER FIRST — the sequenced spine by (epoch, seq), commutative last — exactly
    // as every `Node::folded_*` does before it delivers. Without it a log handed
    // over in storage order delivers a sequenced delta before the one it follows,
    // the Coordinator rejects it, the rejection is swallowed below, and the head
    // this fold reports is SHORT: the next sequenced op then claims a position
    // already taken and the object forks. Caught by m11/m12 the day `apply` began
    // building every sequenced delta through here.
    let mut rows: Vec<&(Delta, MemberId)> = ctx.log.iter().collect();
    rows.sort_by_key(|(d, _)| (d.seq.is_none(), d.epoch, d.seq.unwrap_or(0)));
    for (d, author) in rows {
        let _ = c.deliver(d.clone(), *author);
    }
    c
}

fn declared<T: ObjectType>(op_id: u32) -> Option<&'static OpDecl> {
    coordinator::ratify_op(op_id).or_else(|| T::op(op_id))
}

fn probe_as<T: ObjectType>(ctx: &Ctx, candidate: &Delta) -> Result<(), Refusal> {
    let op_id = candidate.op_id;
    let mut c = fold::<T>(ctx);
    // The state BEFORE the candidate, for the reducer's question.
    let mut state = c.state();
    match c.deliver(candidate.clone(), ctx.me) {
        Ok(true) => {}
        Ok(false) => return Err(Refusal::AlreadyFolded { op_id }),
        Err(why) => return Err(Refusal::Structural { op_id, why }),
    }
    // RATIFY ops are a base group every object carries and no domain reducer
    // knows; `ratify_state` folds them. The coordinator's answer is the probe.
    if coordinator::ratify_op(op_id).is_some() {
        return Ok(());
    }
    let owner = owner_at(ctx.owners, candidate.epoch).unwrap_or(ctx.me);
    let rctx = ReduceContext {
        members: ctx.members,
        owner,
        epoch: candidate.epoch,
    };
    let op = Op {
        op_id,
        args: &candidate.args,
        author: &ctx.me,
        pos: candidate.seq.map(|seq| LogPosition {
            epoch: candidate.epoch,
            seq,
        }),
        ctx: &rctx,
    };
    T::reduce(&mut state, &op).map_err(|why| Refusal::Reducer { op_id, why })
}

/// May `op_id` be written onto a `kind` object at all — before any state is
/// consulted? The kind declares it, and none of the gates on kind refuses it.
/// `build` asks exactly this first, and [`catalogue`] asks it of every kind, so
/// what a consumer is told an op may go on and what the door accepts cannot
/// disagree.
fn authorable_as<T: ObjectType>(kind: &str, op_id: u32) -> Result<&'static OpDecl, Refusal> {
    let decl = declared::<T>(op_id).ok_or_else(|| Refusal::NotDeclared {
        kind: kind.to_string(),
        op_id,
    })?;
    if T::KIND == ObjectKind::Group {
        if crate::membership::is_membership_op(op_id) {
            return Err(Refusal::ByTheMlsDoors { op_id });
        }
    }
    // The pairing channel takes the ticket and invite legs and nothing else. The
    // rule was an allowlist inside one `Node` method, so it held for the native
    // caller and not for the browser authoring through this door.
    if T::KIND == ObjectKind::Contact
        && !crate::contact::is_pairwise_leg(op_id)
        && coordinator::ratify_op(op_id).is_none()
    {
        return Err(Refusal::NotALeg {
            kind: kind.to_string(),
            op_id,
        });
    }
    Ok(decl)
}

/// The ops a `kind` declares, and the ratify ops every object carries.
fn declared_ops<T: ObjectType>() -> Result<Vec<&'static OpDecl>, Refusal> {
    Ok(T::ops().iter().chain(coordinator::RATIFY_OPS.iter()).collect())
}
/// WHICH ICD CHANNEL A KIND STRING WRITES. The ICD is keyed by channel — `group`,
/// `forum`, `conversation` — and the directory speaks KIND strings — `connection`,
/// `notebook`, `member-tether`. This is the one map between them, and it is the
/// same dispatch `build` makes, so a consumer reading the ICD for what an op is
/// cannot disagree with the door about which ops a kind takes.
pub fn channel_of(kind: &str) -> Option<ObjectKind> {
    fn kind_of<T: ObjectType>() -> Result<ObjectKind, Refusal> {
        Ok(T::KIND)
    }
    on_lens!(kind, kind_of()).ok()
}


/// Whether op `op_id` on `kind` is commutative: ordered by (gen, author, id), so the epoch
/// it is authored at is its author's stamp alone, whatever epoch seals it. Unknown is not.
pub fn is_commutative(kind: &str, op_id: u32) -> bool {
    on_lens!(kind, authorable_as(kind, op_id)).is_ok_and(|d| d.commutativity == Commutativity::Commutative)
}

/// EVERY OP THIS DOOR WILL WRITE, and the kinds it will write each onto: the
/// `on` a consumer checks before it asks. Derived by asking [`authorable_as`] of
/// every kind string, so it says what `build` will accept and nothing else: a
/// note op is on `notebook` and `note`, not on a Site's `group`; the ratify ops
/// are on every kind; a membership record is on none. Keyed by the ICD NAME:
/// op ids are per kind (`forum.post` and `group.setProfile` are both op 0), so an
/// id alone would merge different ops. Ordered by name.
/// The op `name` names ON `kind`, or `None`. Op ids are per kind, so a name must be
/// resolved against the object's own kind: resolved on its own, another kind's
/// `forum.post` gives an id that names a different op here (NC-54).
pub fn op_on(kind: &str, name: &str) -> Option<&'static OpDecl> {
    catalogue()
        .into_iter()
        .find(|(d, on)| d.name == name && on.contains(&kind))
        .map(|(d, _)| d)
}

pub fn catalogue() -> Vec<(&'static OpDecl, Vec<&'static str>)> {
    let mut on: std::collections::BTreeMap<&'static str, (&'static OpDecl, Vec<&'static str>)> = Default::default();
    for kind in kinds() {
        let ops: Vec<&'static OpDecl> = on_lens!(kind, declared_ops()).unwrap_or_default();
        for d in ops {
            // The decl build finds for this id on this kind must be THIS op: ids are
            // per kind, so an id that passes may name a different op.
            let ok = lens(kind).is_some()
                && on_lens!(kind, authorable_as(kind, d.op_id)).is_ok_and(|found| found.name == d.name);
            if ok {
                on.entry(d.name).or_insert((d, Vec::new())).1.push(kind);
            }
        }
    }
    on.into_values().collect()
}

fn build_as<T: ObjectType>(kind: &str, op_id: u32, mut args: Args, ctx: &Ctx) -> Result<Delta, Refusal> {
    let decl = authorable_as::<T>(kind, op_id)?;

    if decl.authority == Authority::Owner && owner_at(ctx.owners, ctx.epoch) != Some(ctx.me) {
        return Err(Refusal::OwnerOnly { op: decl.name });
    }
    // THE OWNER, OR A ROLE HOLDER IN THIS OBJECT'S OWN FOLDED STANDING (ICD 2.1.0 row 10):
    // the check `principals.enforcement` names, beside the owner's. The reducer holds the
    // same rule at fold, so a device that skips this gains nothing.
    if let Authority::OwnerOrRole(role) = decl.authority {
        if owner_at(ctx.owners, ctx.epoch) != Some(ctx.me) && T::role_of(&fold::<T>(ctx).state(), &ctx.me) != Some(role) {
            return Err(Refusal::OwnerOrRoleOnly { op: decl.name, role: role.as_str() });
        }
    }
    // A ROLE HOLDER'S ALONE (`Authority::Roles`): the owner by being the owner is not one.
    if let Authority::Roles(roles) = decl.authority {
        if !T::role_of(&fold::<T>(ctx).state(), &ctx.me).is_some_and(|r| roles.contains(&r)) {
            return Err(Refusal::RoleOnly { op: decl.name, roles: roles.iter().map(|r| r.as_str()).collect::<Vec<_>>().join(" or ") });
        }
    }
    if T::RULES {
        if let Some(why) = T::refuses(&fold::<T>(ctx).state(), op_id, &args, &ctx.me) {
            return Err(Refusal::NotNow { op: decl.name, why });
        }
    }

    let delta = match decl.commutativity {
        Commutativity::Commutative => {
            let watermark = ctx.watermark.ok_or(Refusal::NoWatermark { op: decl.name })?;
            let gen = next_gen(ctx.log, watermark);
            args.insert("gen".to_string(), ArgVal::Int(gen as i64));
            crate::object::build_delta(T::KIND, op_id, args, ctx.epoch, Some(gen))
        }
        Commutativity::Sequenced => {
            let head = fold::<T>(ctx).sequenced_head();
            // NC-65. An empty spine is a genesis only on a device that has held the object
            // since it began: owner history from epoch 0 is written at creation, and a
            // joiner's starts at its join epoch. A device that joined later folds what it
            // cannot see as empty, and a delta chained to nothing breaks the chain for
            // every replica that holds it.
            if head.is_none() && !ctx.owners.iter().any(|(from, _)| *from == 0) {
                return Err(Refusal::ChainNotHeld { op: decl.name });
            }
            let (seq, prev) = coordinator::next_sequenced_pos(head, ctx.epoch);
            coordinator::sequenced_delta(T::KIND.type_id() as u32, op_id, args, ctx.epoch, seq, prev)
        }
    };

    probe_as::<T>(ctx, &delta)?;
    Ok(delta)
}

#[cfg(test)]
mod tests {

    /// NC-54: a name resolves only on the kinds that declare it.
    #[test]
    fn an_op_name_resolves_only_on_its_own_kind() {
        let post = super::op_on("forum", "forum.post").expect("forum.post is a forum's");
        assert_eq!(post.name, "forum.post");
        assert!(super::op_on("group", "forum.post").is_none(), "a group has no forum.post, whatever id it carries");
        assert!(super::op_on("forum", "no.such").is_none());
    }

    use super::*;
    use crate::group::GroupType;
    use crate::mint::MintDraft;

    const OWNER: MemberId = [1u8; 32];
    const OTHER: MemberId = [2u8; 32];

    fn ctx<'a>(me: MemberId, members: &'a [MemberId], owners: &'a [(u64, MemberId)], log: &'a [(Delta, MemberId)]) -> Ctx<'a> {
        Ctx { me, epoch: 0, members, owners, log, watermark: Some(0) }
    }
    fn draft(name: &str) -> MintDraft {
        MintDraft { name: name.to_string(), ..Default::default() }
    }
    /// The op a kind's mint authors first — the tests' one source of valid args.
    fn op0(kind: ObjectKind) -> (u32, Args) {
        mint_ops(kind, &draft("thursday")).unwrap().remove(0)
    }

    /// NC-65: an owner's device that joined after the object began, holding no head,
    /// is refused a sequenced op; one that has held the object since it began is not.
    #[test]
    fn a_device_that_joined_late_holding_no_head_is_refused_a_sequenced_op() {
        let (kind, (op, args)) = ("group", op0(ObjectKind::Group));
        let members = [OWNER];
        let joined = [(3, OWNER)];
        let late = Ctx { epoch: 3, ..ctx(OWNER, &members, &joined, &[]) };
        let refused = build(kind, op, args.clone(), &late).unwrap_err();
        assert!(matches!(refused, Refusal::ChainNotHeld { .. }), "{refused:?}");
        assert_eq!(
            refused.to_string(),
            "this device joined after this object began, and can't see its current state; \
             sign in on a device that holds it"
        );

        let began = [(0, OWNER)];
        assert!(build(kind, op, args, &ctx(OWNER, &members, &began, &[])).is_ok(), "the minting device");
    }

    #[test]
    fn a_mint_is_its_profile_and_a_placed_place_is_its_location_too() {
        assert!(mint_ops(ObjectKind::Forum, &draft("x")).unwrap().is_empty(), "a Forum is name-only");
        assert_eq!(mint_ops(ObjectKind::Event, &draft("x")).unwrap().len(), 1);
        let placed = MintDraft { placed: true, lat: 51.5, lng: -0.12, ..draft("here") };
        let ops = mint_ops(ObjectKind::Place, &placed).unwrap();
        assert_eq!(ops.len(), 2, "profile, then the fix");
        assert_eq!(ops[1].0, crate::geo::OP_SET_LOCATION);
        assert_eq!(mint_ops(ObjectKind::Contact, &draft("x")), Err(crate::mint::NoMint::Paired));
    }

    /// The browser mint's path for every kind with an op 0: the owner, an empty log,
    /// and a draft a person could actually submit.
    #[test]
    fn every_mintable_kind_builds_its_op_zero_on_an_empty_log() {
        let (members, owners) = ([OWNER], [(0, OWNER)]);
        // An Event needs a start: its reducer refuses `startMs <= 0`, so a draft
        // without one mints an event whose profile the fold then skips.
        let real = MintDraft { start_ms: 1_758_000_000_000, ..draft("thursday") };
        for (kind, name) in [
            (ObjectKind::Group, "group"),
            (ObjectKind::Project, "project"),
            (ObjectKind::Thing, "thing"),
            (ObjectKind::Place, "place"),
            (ObjectKind::Event, "event"),
            (ObjectKind::Post, "post"),
        ] {
            for (op_id, args) in mint_ops(kind, &real).unwrap() {
                let d = build(name, op_id, args, &ctx(OWNER, &members, &owners, &[]))
                    .unwrap_or_else(|r| panic!("{name} op {op_id}: {r}"));
                assert_eq!(d.type_id, kind.type_id() as u32, "{name}: its own type id");
            }
        }
    }

    /// WHAT THE PROBE IS FOR, on the case that finds it. An Event drafted with no
    /// start is refused here. The native mint does not probe, so the same draft
    /// through `Node::mint` authors an op 0 the fold skips — an event that exists
    /// with no profile, and nothing said.
    #[test]
    fn an_event_with_no_start_is_refused_rather_than_written_inert() {
        let (members, owners) = ([OWNER], [(0, OWNER)]);
        let (op, args) = mint_ops(ObjectKind::Event, &draft("no date yet")).unwrap().remove(0);
        assert!(matches!(
            build("event", op, args, &ctx(OWNER, &members, &owners, &[])),
            Err(Refusal::Reducer { why: DeltaRejection::MalformedArgs, .. })
        ));
    }

    /// What a consumer is told an op may go on is what the door accepts: the
    /// catalogue is asked of the same check `build` makes first.
    #[test]
    fn the_catalogue_says_where_each_op_may_go_and_build_agrees() {
        let cat = catalogue();
        let on = |name: &str| -> Vec<&str> {
            cat.iter().find(|(d, _)| d.name == name).map(|(_, k)| k.clone()).unwrap_or_default()
        };
        assert_eq!(on("note.write"), vec!["note", "notebook"],
            "a note is written onto a Note, never onto a Site's group. It used to be \
             `base.noteWrite` on the Group vocabulary, gated by a bespoke refusal; the \
             kind carries its own op now and the gate is the op table.");
        assert_eq!(on("forum.post"), vec!["forum", "conversation"],
            "a conversation shares the forum's chat ops — and a CONNECTION does not: \
             it is the pairing channel, and the chat is the conversation derived \
             from its prekeys (migrated 24 Sep 2026)");
        assert!(on("contact.prekeySupply").is_empty(),
            "a prekey is STOCKED by the pairing and sync paths, never authored — the \
             pairing channel takes the ticket and invite legs and nothing else");
        assert_eq!(on("contact.invite"), vec!["connection"],
            "and a leg that has nowhere else to go does go on it; `contact` was a \
             kind string nothing was ever stored under");
        assert!(on("base.memberJoined").is_empty(), "a membership record rides its MLS commit");
        assert_eq!(on("ratify.vote").len(), kinds().len(), "the ratify ops are on every kind");
        assert!(!on("system.hydrate").is_empty() || cat.iter().any(|(d, _)| d.name.starts_with("system.")),
            "System's ops are in it");
        // And every (op, kind) it names passes build's own first check; every one it
        // leaves out fails it.
        for (d, ks) in &cat {
            for k in kinds() {
                let passes = on_lens!(k, authorable_as(k, d.op_id)).is_ok_and(|found| found.name == d.name);
                assert_eq!(passes, ks.contains(&k), "{} on {k}", d.name);
            }
        }
    }

    #[test]
    fn a_sequenced_op_takes_the_next_position_after_the_log() {
        let (members, owners) = ([OWNER], [(0, OWNER)]);
        let (op, args) = op0(ObjectKind::Group);
        let first = build("group", op, args.clone(), &ctx(OWNER, &members, &owners, &[])).unwrap();
        assert_eq!((first.seq, first.prev), (Some(0), coordinator::GENESIS_PREV));

        let log = [(first.clone(), OWNER)];
        let mut renamed = args;
        renamed.insert("displayName".into(), ArgVal::Text("friday".into()));
        let second = build("group", op, renamed, &ctx(OWNER, &members, &owners, &log)).unwrap();
        assert_eq!(second.seq, Some(1));
        assert_eq!(second.prev, first.id(), "chained to the head it followed");
    }

    #[test]
    fn an_owner_only_op_from_someone_else_is_refused_before_it_can_poison_the_log() {
        let (members, owners) = ([OWNER, OTHER], [(0, OWNER)]);
        let (op, args) = op0(ObjectKind::Group);
        assert!(matches!(
            build("group", op, args, &ctx(OTHER, &members, &owners, &[])),
            Err(Refusal::OwnerOnly { .. })
        ));
    }

    #[test]
    fn an_op_the_kind_does_not_declare_is_refused() {
        let (members, owners) = ([OWNER], [(0, OWNER)]);
        assert!(matches!(
            build("group", 0xDEAD_BEEF, Args::new(), &ctx(OWNER, &members, &owners, &[])),
            Err(Refusal::NotDeclared { op_id: 0xDEAD_BEEF, .. })
        ));
        assert!(matches!(
            build("nonesuch", 0, Args::new(), &ctx(OWNER, &members, &owners, &[])),
            Err(Refusal::NoLens { .. })
        ));
    }

    #[test]
    fn membership_is_the_mls_doors_and_notes_stay_on_notes() {
        let (members, owners) = ([OWNER], [(0, OWNER)]);
        let c = ctx(OWNER, &members, &owners, &[]);
        assert!(matches!(
            build("group", crate::membership::OP_MEMBER_JOINED, Args::new(), &c),
            Err(Refusal::ByTheMlsDoors { .. })
        ));
        assert!(
            !GroupType::ops().iter().any(|d| crate::note::is_note_op(d.op_id)),
            "the Group vocabulary must NOT carry the note facet any more — a Note is \
             kind 32 and declares its own ops (25 Sep 2026)"
        );
        // And onto a Note the door lets it through: the kind declares the op, so
        // the gate is the op table rather than a bespoke refusal. Whatever happens
        // next is the reducer's, not the gate's — empty args will fail there.
        for kind in ["note", "notebook"] {
            assert!(
                !matches!(
                    build(kind, crate::note::OP_WRITE, Args::new(), &c),
                    Err(Refusal::NoLens { .. }) | Err(Refusal::NotDeclared { .. })
                ),
                "`{kind}` must reach the Note door"
            );
        }
    }

    #[test]
    fn a_commutative_op_needs_a_watermark_and_takes_the_lamport_gen_above_it() {
        let (members, owners) = ([OWNER, OTHER], [(0, OWNER)]);
        let post = coordinator::forum_post("hello", 0, 0);
        let mut args = post.args.clone();
        args.remove("gen");

        let blind = Ctx { watermark: None, ..ctx(OWNER, &members, &owners, &[]) };
        assert!(matches!(
            build("forum", post.op_id, args.clone(), &blind),
            Err(Refusal::NoWatermark { .. })
        ));

        // OTHER already wrote at gen 6: the next gen is over EVERY author, and the
        // watermark lifts it further.
        let theirs = coordinator::forum_post("earlier", 6, 0);
        let log = [(theirs, OTHER)];
        let d = build("forum", post.op_id, args.clone(), &ctx(OWNER, &members, &owners, &log)).unwrap();
        assert_eq!(d.gen, Some(7), "one past the highest gen in the log, whoever wrote it");
        assert_eq!(d.args.get("gen"), Some(&ArgVal::Int(7)), "carried in args, as the native doors do");

        let floored = Ctx { watermark: Some(40), ..ctx(OWNER, &members, &owners, &log) };
        assert_eq!(build("forum", post.op_id, args, &floored).unwrap().gen, Some(40));
    }

    #[test]
    fn the_probe_refuses_what_the_reducer_would_ignore() {
        let (members, owners) = ([OWNER], [(0, OWNER)]);
        let (op, _) = op0(ObjectKind::Group);
        match build("group", op, Args::new(), &ctx(OWNER, &members, &owners, &[])) {
            Err(Refusal::Reducer { op_id, .. }) => assert_eq!(op_id, op),
            other => panic!("a profile with no fields is the reducer's to refuse, got {other:?}"),
        }
    }
}
