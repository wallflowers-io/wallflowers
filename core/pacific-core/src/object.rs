//! The object model — the contract every shared object implements.
//!
//! # Every object is a group of 1
//!
//! An **object** in Pacific *is* an MLS group plus an append-only log of typed
//! **Deltas** ("features"). Its compiled state is `reduce(genesis, ordered log)` —
//! the log is the source of truth, the state a rebuildable cache. Its membership is
//! the MLS group's members, and **membership == access** (only members hold the
//! keys to decrypt and fold the log).
//!
//! The only thing that varies across the lifecycle is the **member count**:
//!
//! - **1 member (you)** — a private, on-device object you author alone. There is no
//!   one to sync with, so it never touches the relay. `create a Project`, `start a
//!   Poll`, and `import a YC company` all mint a fresh MLS group of one and append a
//!   `create` Delta. Free and local.
//! - **2 members** — the pairwise case (today's "handshake" is exactly this).
//! - **N members** — a shared room / working group.
//!
//! "Collaboration" is therefore just an owner-sequenced MLS **Add**: the existing
//! log backfills to the newcomer (via the Welcome + a log replay), and from then on
//! every member folds the same Deltas to the same state. Pairing is not special —
//! it is "create a group of 2".
//!
//! ## The YC example (group-of-1 entities)
//!
//! When the YC System returns a company, the app mints a **group-of-1 object** you
//! own — an `org` entity, members = {you}, seeded with Deltas carrying YC's facts
//! under a stable entity id (the YC slug) plus your own private annotations. It is
//! never synced. Reconciling that private stub with the real entity when you connect
//! is an **entity-resolution** concern (a private stub always stays private; you
//! *join the entity's canonical group* rather than promoting your stub), not a group
//! merge — two MLS groups never merge.
//!
//! # The type catalog (what the UI team works with)
//!
//! Six object types. Each declares a fixed set of **ops** (the Deltas you can author)
//! with an [`Authority`] (who may issue it) and a [`Commutativity`] (how it folds),
//! and exposes **projections** (the read-side state). Ported from the Python
//! object prototype under `_prototype/`.
//!
//! | id | kind | ops (Δ) | projections |
//! |----|------|---------|-------------|
//! | 18 | **Group**    | setProfile · setPresence · storeCredential · revokeCredential · setMemberRole *(owner/seq)* | vault_index, member_space, can_sync, can_act |
//! | 19 | **Forum**    | configure · addTopicArea · renameTopicArea · setVisibility *(owner/seq)* · post · retractPost *(any/comm)* · **poll**{configure·close *(owner/seq)* · vote *(any/comm, LWW)*} · **question**{ask *(any/seq)* · respond *(any/comm, single-slot LWW)*} | thread, visible_thread, areas, tally, question_status |
//! | 20 | **Field**    | setKeyIndex · defineDeepLink · defineExpression · setRole · suppress · unsuppress *(owner/seq)* · setLiteral · recordResolution *(any/comm)* | value, liveness, dependencies, flavour |
//! | 21 | **System**   | define · addField · removeField · setConnector · addOperation · removeOperation · bindCredential · revokeCredential · suppressOperation *(owner/seq)* · resolveValue *(any/comm)* | definition, live_values, writable_fields |
//! | 22 | **Project**  | configure · addTimelineItem · suppressItem · setItemSchedule · setAssignee · addDependency · removeDependency · subscribe · unsubscribe *(owner/seq)* · setItemProgress · setItemField · touchSubscription *(any/comm)* | board, item_view, deliverables, piece_count, subscription_cursor |
//! | 23 | **Topic**    | charter · setCadence · connect · disconnect *(owner/seq)* · addFinding · retractFinding · poseQuestion · resolveQuestion *(any/comm)* | digest, revision, findings, open_questions |
//!
//! **Catalog cutover (2026-06-28):** former standalone kinds **Poll(16)**, **Message(17)**
//! and **Question(24)** are FOLDED into Forum as delta op-groups (Message.post ≡ Forum.post;
//! Poll/Question become `poll.*` / `question.*` op-groups on a Forum). **Role(0)** is no
//! longer an object — roles are a per-member PROPERTY of every GroupObject, assigned via
//! Delta: the Role ops become a BASE governance op-group on all six (assignOwner/setRole/
//! addMember/removeMember/repinSuccession) and roster/owner/roles live in the base
//! `.members` facet. type_ids 0/16/17/24 are RESERVED (an old delta on them fails loud as
//! `UnknownType`). The absorbed-op REDUCERS are the next, separate PR (today still
//! milestone-NotImplemented — folding moves where ops live, it does not claim they are built).
//!
//! Two op classes, two fold rules ([`Commutativity`]):
//! - **SEQUENCED** — owner-stamped total order on `(epoch, seq)`, `prev` hash-chained.
//!   A second Delta at the same position with a different DeltaId is a [`DeltaRejection::ForkDetected`].
//! - **COMMUTATIVE** — a CRDT OR-set, per-author last-writer-wins by `(gen, author)`,
//!   folded in canonical `(gen, author, DeltaId)` order so state is a function of the
//!   Delta *set*, never arrival order.
//!
//! Spec invariant (enforced at registration): a COMMUTATIVE op MUST be `ANY_MEMBER`
//! authority — a broadcast op can't be owner-gated before it propagates.

/// THE NEXT `gen` FOR A COMMUTATIVE DELTA, computed in one place.
///
/// 37 of the ICD's 95 ops fold commutatively, and the fold keys those by
/// `(author_pk, gen)`. Mint the same pair twice and the fold DROPS ONE OF THE
/// TWO — the delta persists into a durable, signed log, does nothing for ever,
/// and the author is told it succeeded. That is the inert-delta failure, and it
/// is silent at exactly the layer least able to notice.
///
/// It takes TWO inputs and a platform supplies both, because either alone is
/// wrong:
///
///   - **the highest `gen` already in this device's log.** A device that has been
///     running for months is well past whatever floor it was handed on the day it
///     joined, and must keep using its log.
///   - **the watermark the Welcome carried.** A device that joined late has an
///     EMPTY log — forward secrecy guarantees it — so its log says 0 and every ref
///     it mints collides with one its own PERSON already used from another
///     device. The adder's high-water mark is the only thing that arrives before
///     this device can author anything.
///
/// ## Why it is a type and not a `u64`
///
/// Because `u64` accepts the wrong answer. "Count what I can see" is the natural
/// reading, it type-checks, and it produces the collision above while appearing to
/// work. There is deliberately no `From<u64>` and no public field: the only way to
/// get one is to hand over both halves, and a platform that has only one of them
/// finds out here rather than in somebody's dropped message.
///
/// A platform therefore supplies INPUTS, never a conclusion. The maximum is taken
/// here so there is one implementation of the rule rather than one per caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct NextGen(u64);

impl NextGen {
    /// The only constructor.
    ///
    /// `highest_in_log` is `None` when the log holds no commutative delta yet —
    /// which is a fresh group AND a device that joined late, and the watermark is
    /// what separates them.
    pub fn compute(highest_in_log: Option<u64>, watermark: u64) -> Self {
        NextGen(highest_in_log.map_or(0, |m| m + 1).max(watermark))
    }

    /// The value to stamp on the delta.
    pub fn get(self) -> u64 {
        self.0
    }
}

use crate::coordinator::{Args, Delta};

/// A member's stable identity public key — the principal that authored a Delta
/// (supplied by the MLS-authenticated transport, never carried on the wire).
pub type MemberId = [u8; 32];

/// Who may issue an op (`core.py::Authority`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Authority {
    /// Any current member may author it.
    AnyMember,
    /// Only the object's owner may author it.
    Owner,
    /// The owner, or a member on the current roster holding this role in the object's OWN
    /// folded `roles` state (ICD `principals.either`, `owner|role:<name>`; 2.1.0 row 10).
    /// Commutative only: the sequenced spine takes the owner alone.
    OwnerOrRole(crate::group::GroupRole),
    /// A member holding one of these roles in the object's own folded `roles` state, and
    /// no one else: not the owner by being the owner (ICD `principals.role`, `role:<a>` or
    /// `role:<a>|role:<b>`). The Transaction's: the settler owns it and holds no deal role,
    /// so an owner check could not say "the settler may not author the deal". Commutative
    /// only, as OwnerOrRole.
    Roles(&'static [crate::group::GroupRole]),
}

impl Authority {
    /// The ICD's `ego` for it: `owner`, `member`, or `owner|role:<name>`.
    pub fn ego(&self) -> String {
        match self {
            Authority::Owner => "owner".into(),
            Authority::AnyMember => "member".into(),
            Authority::OwnerOrRole(r) => format!("owner|role:{}", r.as_str()),
            Authority::Roles(rs) => rs.iter().map(|r| format!("role:{}", r.as_str())).collect::<Vec<_>>().join("|"),
        }
    }

    /// As the web catalogue names it: `owner`, `anyMember`, or the ICD's `owner|role:<name>`.
    pub fn web(&self) -> String {
        match self {
            Authority::AnyMember => "anyMember".into(),
            a => a.ego(),
        }
    }
}

/// How an op folds (`core.py::Commutativity`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Commutativity {
    /// Owner-stamped total order on `(epoch, seq)`, `prev` hash-chained.
    Sequenced,
    /// CRDT OR-set, per-author LWW by `(gen, author)`.
    Commutative,
}

/// One constituent of a macro-node: what it is FOR, and what kind it is. `mint`
/// brings each into being with the parent and writes both halves of the edge:
/// `base.setPart` on the parent, `base.setParent` on the part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Part {
    /// What the part is for, as the ICD names it: `comments`.
    pub role: &'static str,
    pub kind: ObjectKind,
}

/// One COMPOSITION edge, the parent's half (`base.setPart`): a part that is its own
/// GroupObject, in a role. A room, a comments section, a Treasury, a Host and a
/// sub-group are the same edge; the ICD names it `part_of`.
///
/// Deliberately NOT an [`crate::group::Affiliation`]: an affiliation relates two
/// SOVEREIGN parties (a chapter and its federation, a group and a place it
/// answers for) and therefore carries a `rel` and a tether. A part has no
/// counterparty to negotiate with — it is attached and detached at the parent's
/// sole discretion.
///
/// Membership is NOT here and never will be: the edge says the part belongs to
/// the parent; the part's own MLS roster says who is in it (no-dual-source). Nor is
/// its name: the part's own MLS GroupContext holds that.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PartRef {
    /// What the part is for: `room`, `comments`, `treasury`, `host`, …
    pub role: String,
    /// Unix ms when the part was attached (event time, author-supplied).
    pub at: i64,
    /// A room's claim choice (ICD 2.1.0 row 3): the visitors whose claim carries it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice: Option<String>,
}

/// One operation a type understands (`core.py::OpDecl`). The op table is fixed at
/// compile time per type — there is no runtime op discovery.
#[derive(Clone, Copy, Debug)]
pub struct OpDecl {
    pub op_id: u32,
    pub name: &'static str,
    pub authority: Authority,
    pub commutativity: Commutativity,
}

impl OpDecl {
    /// The spec invariant (`core.py::validate_spec`): a commutative op must be
    /// any-member. Call this for every op at registration to fail loudly.
    pub const fn is_well_formed(&self) -> bool {
        match self.commutativity {
            Commutativity::Commutative => matches!(self.authority, Authority::AnyMember | Authority::OwnerOrRole(_) | Authority::Roles(_)),
            Commutativity::Sequenced => true,
        }
    }
}

/// Total-order key for sequenced Deltas (`core.py::LogPosition`). `epoch` is the MLS
/// epoch the Delta was sequenced in; `seq` is the gap-free, per-epoch, owner-assigned
/// sequence number that resets to 0 each epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LogPosition {
    pub epoch: u64,
    pub seq: u64,
}

/// The ONLY ambient inputs a reducer may read (`core.py::ReduceContext`). Crucially
/// it carries **no liveness** — folds must be a pure function of the log, so they
/// replay identically on every replica.
#[derive(Clone, Debug)]
pub struct ReduceContext<'a> {
    pub members: &'a [MemberId],
    pub owner: MemberId,
    pub epoch: u64,
}

impl ReduceContext<'_> {
    pub fn is_member(&self, who: &MemberId) -> bool {
        self.members.contains(who)
    }
    pub fn is_owner(&self, who: &MemberId) -> bool {
        &self.owner == who
    }
}

/// Everything `reduce` needs about one Delta being applied. `pos` is `Some` for a
/// sequenced op (its total-order position) and `None` for a commutative one.
pub struct Op<'a> {
    pub op_id: u32,
    pub args: &'a Args,
    pub author: &'a MemberId,
    pub pos: Option<LogPosition>,
    pub ctx: &'a ReduceContext<'a>,
}

/// Every reject is typed and LOUD — nothing is ever swallowed (`core.py` rejection
/// taxonomy, PROTOCOL sec 3.6). A reducer returns one of these instead of mutating.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeltaRejection {
    /// Author lacks the authority the op requires.
    Unauthorized,
    /// A precondition on current state failed.
    PreconditionFailed,
    /// Args missing/ill-typed for the op.
    MalformedArgs,
    /// `prev` did not chain to the expected predecessor (sequenced).
    ChainBroken,
    /// Same position, different DeltaId — a divergence halt (sequenced, sec 3.5).
    ForkDetected,
    /// A late `(N, *)` after the epoch fence (sequenced, sec 3.3).
    StaleEpoch,
    /// Unknown op id for this type.
    UnknownType,
    /// The op-arg principal does not match the MLS sender (sec 2).
    SenderMismatch,
    /// Idempotent no-op — this exact DeltaId was already folded.
    Duplicate,
}

/// The object kinds and their wire `type_id`s. A closed taxonomy: adding a kind
/// is a compile error everywhere a match must handle it (the reason this is an enum
/// rather than open trait-object dispatch — see the crate's "inheritance in Rust"
/// notes: closed set ⇒ enum + exhaustiveness).
///
/// `type_id`s are WIRE values on an append-only log: they are assigned once and never
/// reused. A new kind takes the next free id (see the RESERVED set below) — never a
/// reclaimed one, or an old delta folds into the wrong type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    Group,
    Forum,
    Field,
    System,
    Project,
    Topic,
    /// A 1:1 relationship channel formed by pairing — holds the prekey pool and the
    /// pairing lifecycle. NOT a conversation (no messages); Conversations are formed
    /// by consuming its prekeys.
    Contact,
    /// A 1:1-or-1:n message thread (DMs + group chats — the Chats tab). Shares the
    /// message mechanics with Forum (`ForumState`), but is its own object so prekeys
    /// stay on Contact and Forum stays the Project-linked reddit-thread analogue.
    Conversation,
    /// A concrete thing in your world — the hammer, the bike, the drill. Minted from a
    /// resolved entity once you accept it at the swipe deck, and the object a MARKET
    /// posture hangs on: Has / Wants / Offers / Buying / Selling. The posture is state
    /// ON the thing rather than a sixth object kind, because Wants/Buying/Offers/Selling
    /// are commitments about a thing and Has is simply holding it.
    Thing,
    /// Somewhere in the world — your kitchen, a bar, a site. A Place is a GroupObject
    /// so it can be SHARED: the point of knowing where somewhere is, is telling someone
    /// else. A device-local row cannot be sent, cannot be co-owned, and has no log.
    ///
    /// Its defining state is the common location facet (`geo.rs`) carrying a FIXED
    /// point — which is why Place is the first kind to include a base op-group in its
    /// `ops()`. A Place is not a Thing: a Thing's `area` is deliberately coarsened to a
    /// publishable geohash prefix because a market posture broadcasts position, whereas
    /// a Place's whole value is the precise coordinate you stood on.
    Place,
    /// An occurrence with a when — and the object TICKETING hangs on. The organizer
    /// owns it, door staff join it, and its log is the sale ledger; buyers NEVER join
    /// (the ticket legs ride their pairwise Contact channel instead). See `event.rs`
    /// for the delegate model: paid fulfilment is authored by an Arc box-office node
    /// the owner names in the listing, enforced at fold.
    Event,
    /// A published thing with its own spine — see `post.rs`. NOT `forum.post` (a message
    /// in a Channel) or `place.post` (a note on a wall): both are ops on a spine that
    /// already exists, whereas this is the object you can hand someone.
    ///
    /// Two roles, Owner and Viewer, which land exactly on the two authority levels.
    Post,
    /// A shared treasury: its own roster, so a member of the body
    /// need not be a member of the money (ruled 25 Sep 2026).
    Treasury,
    /// Prose with its own roster. A private notebook is a Note whose
    /// roster is one: roster size is not a kind (ruled 25 Sep 2026).
    Note,
    /// A Site's public copy, and the only object an Arc joins. Its roster is
    /// the owner's devices and the Arc; the group's members are not on it.
    Host,
    /// ONE SALE, record only (W-98 Trade; see `transaction.rs`): the buyer, the seller and
    /// the settler, who owns it and holds no deal role. Terminal: one object a sale.
    Transaction,
}

impl ObjectKind {
    /// Every declared kind, in type-id order. MUST list every variant — it is what
    /// the catalog-wide sweeps iterate (`mint::classify`, the mint conformance tests).
    /// `all_lists_every_kind` pins the count so a new variant cannot be added here
    /// without being added there.
    pub const ALL: &'static [ObjectKind] = &[
        ObjectKind::Group,
        ObjectKind::Forum,
        ObjectKind::Field,
        ObjectKind::System,
        ObjectKind::Project,
        ObjectKind::Topic,
        ObjectKind::Contact,
        ObjectKind::Conversation,
        ObjectKind::Thing,
        ObjectKind::Place,
        ObjectKind::Event,
        ObjectKind::Post,
        ObjectKind::Treasury,
        ObjectKind::Note,
        ObjectKind::Host,
        ObjectKind::Transaction,
    ];

    pub const fn type_id(self) -> u16 {
        match self {
            ObjectKind::Group => 18,
            ObjectKind::Forum => 19,
            ObjectKind::Field => 20,
            ObjectKind::System => 21,
            ObjectKind::Project => 22,
            ObjectKind::Topic => 23,
            ObjectKind::Contact => 25,
            ObjectKind::Conversation => 26,
            ObjectKind::Thing => 27,
            // 24 is RESERVED (folded Question) — 28 is the next free id, NOT 24.
            ObjectKind::Place => 28,
            ObjectKind::Event => 29,
            // 30 is the next free id — 0/16/17/24 are RESERVED, see below.
            ObjectKind::Post => 30,
            ObjectKind::Treasury => 31,
            ObjectKind::Note => 32,
            ObjectKind::Host => 33,
            ObjectKind::Transaction => 34,
        }
    }

    /// THE CONSTITUENT OBJECTS THIS KIND IS MADE OF — its macro-node shape.
    ///
    /// A Post is not only a Post: it is a Post and its comments section, and the
    /// comments section is a Forum GroupObject. `mint` brings the whole shape into
    /// being and points the parent at each part with the part's edge.
    ///
    /// The ICD declares this in `x-object.parts` and `icd.rs` pins this table to
    /// it, so a shape stated in the document and not here is a failing test rather
    /// than an object that quietly comes up half-built.
    pub const fn parts(self) -> &'static [Part] {
        const COMMENTS: &[Part] = &[Part { role: "comments", kind: ObjectKind::Forum }];
        match self {
            // A Treasury is made of nothing. Comments on the money belong on the
            // thing being paid for, and a room hung here would have the TREASURY's
            // roster — which is narrower than the body's on purpose, so it would be
            // the one room most members of the body could not read.
            // Neither is made of anything: a Treasury's roster is narrower than the
            // body's, and a Note's comments are `note.comment` on the note itself.
            // Nor is a Host: what it carries is a COPY of the group's face, put
            // there by a Delta, not a constituent object of its own.
            ObjectKind::Treasury | ObjectKind::Note | ObjectKind::Host | ObjectKind::Transaction => &[],
            // Everything a person publishes carries a room to answer it in.
            ObjectKind::Post
            | ObjectKind::Event
            | ObjectKind::Thing
            | ObjectKind::Place
            | ObjectKind::Project => COMMENTS,
            // A GROUP is deliberately not here. A Site's rooms are CHOSEN — it
            // hosts the forums its owner attaches, and one implied "comments"
            // room would be a room nobody asked for on every Space ever made.
            // It has the facet; it has no automatic part.
            ObjectKind::Group => &[],
            // A Forum is a part, and parts have no parts: this is where the
            // recursion stops, and it stops by declaration rather than by a depth
            // counter somewhere.
            ObjectKind::Forum
            | ObjectKind::Conversation
            | ObjectKind::Contact
            | ObjectKind::System
            | ObjectKind::Field
            | ObjectKind::Topic => &[],
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            ObjectKind::Group => "group",
            ObjectKind::Forum => "forum",
            ObjectKind::Field => "field",
            ObjectKind::System => "system",
            ObjectKind::Project => "project",
            ObjectKind::Topic => "topic",
            ObjectKind::Contact => "contact",
            ObjectKind::Conversation => "conversation",
            ObjectKind::Thing => "thing",
            ObjectKind::Place => "place",
            ObjectKind::Event => "event",
            ObjectKind::Post => "post",
            ObjectKind::Treasury => "treasury",
            ObjectKind::Note => "note",
            ObjectKind::Host => "host",
            ObjectKind::Transaction => "transaction",
        }
    }

    pub fn from_type_id(id: u16) -> Option<Self> {
        Some(match id {
            18 => ObjectKind::Group,
            19 => ObjectKind::Forum,
            20 => ObjectKind::Field,
            21 => ObjectKind::System,
            22 => ObjectKind::Project,
            23 => ObjectKind::Topic,
            25 => ObjectKind::Contact,
            26 => ObjectKind::Conversation,
            27 => ObjectKind::Thing,
            28 => ObjectKind::Place,
            29 => ObjectKind::Event,
            30 => ObjectKind::Post,
            31 => ObjectKind::Treasury,
            32 => ObjectKind::Note,
            33 => ObjectKind::Host,
            34 => ObjectKind::Transaction,
            // 0/16/17/24 (Role/Poll/Message/Question) are RESERVED — folded into
            // Forum / the base governance op-group; an old delta on them fails loud.
            _ => return None,
        })
    }

    /// This kind's op table, or `None` for a kind that has no reducer at all.
    ///
    /// A DISPATCH, not a table: every name, id, authority and fold it yields comes
    /// from the type's own `ops()`, which is the thing the Coordinator already
    /// enforces and `icd.rs` already holds to the ICD. Nothing is restated here, and
    /// the match is exhaustive, so a thirteenth kind does not compile until somebody
    /// has said which op table is its own.
    ///
    /// `Field` and `Topic` answer `None` for the reason `icd::anchor` gives: they are
    /// a reserved wire id and nothing else — no `impl ObjectType`, no reducer, no
    /// author door.
    pub fn op_table(self) -> Option<&'static [OpDecl]> {
        Some(match self {
            ObjectKind::Group => crate::group::GroupType::ops(),
            ObjectKind::Forum => crate::coordinator::ForumType::ops(),
            ObjectKind::System => crate::system::SystemType::ops(),
            ObjectKind::Project => crate::project::ProjectType::ops(),
            ObjectKind::Contact => crate::contact::ContactType::ops(),
            ObjectKind::Conversation => crate::coordinator::ConversationType::ops(),
            ObjectKind::Thing => crate::thing::ThingType::ops(),
            ObjectKind::Place => crate::place::PlaceType::ops(),
            ObjectKind::Event => crate::event::EventType::ops(),
            ObjectKind::Post => crate::post::PostType::ops(),
            // Promoted out of `group`'s facets into kinds of their own, 25 Sep 2026.
            // This branch was cut when the enum ended at Post; the exhaustive match
            // did its job and stopped the merge from compiling.
            ObjectKind::Treasury => crate::treasury::TreasuryType::ops(),
            ObjectKind::Note => crate::note::NoteType::ops(),
            ObjectKind::Host => crate::host::HostType::ops(),
            ObjectKind::Transaction => crate::transaction::TransactionType::ops(),
            ObjectKind::Field | ObjectKind::Topic => return None,
        })
    }
}

/// What to call the `(type_id, op_id)` pair on an envelope, in words — `"group.setCover"`.
///
/// For messages a person reads. It resolves through [`ObjectKind::op_table`], so the
/// name is the op's OWN declared name and there is no second list to rot; the base
/// membership ops, which no kind declares, come from `membership::MEMBERSHIP_OPS`.
/// A pair this build does not know falls back to the numbers rather than guessing —
/// "type 31 op 7" is the honest answer when an older build meets a newer delta, and
/// it is the answer an operator needs to tell that apart from corruption.
pub fn op_label(type_id: u32, op_id: u32) -> String {
    let named = u16::try_from(type_id)
        .ok()
        .and_then(ObjectKind::from_type_id)
        .and_then(|k| k.op_table())
        .into_iter()
        .flatten()
        .chain(crate::membership::MEMBERSHIP_OPS.iter())
        .find(|d| d.op_id == op_id)
        .map(|d| d.name);
    match named {
        Some(n) => n.to_string(),
        None => format!("type {type_id} op {op_id}"),
    }
}

/// The contract every object type implements. Replaces `core.py::ObjectType` (a
/// Python base class) with a Rust trait: `State` is the compiled projection, and
/// `reduce` MUST be **pure and deterministic** — it may read only `op.ctx`, and
/// returns a typed [`DeltaRejection`] instead of mutating on any error.
///
/// A concrete `T: ObjectType` is folded by the generic coordinator (a separate
/// module): it splits the log into the sequenced spine (`(epoch, seq)` order,
/// hash-chained) and the commutative OR-set (`(gen, author, DeltaId)` order), and
/// calls `reduce` for each in turn.
pub trait ObjectType: 'static {
    /// The kind tag (and thus the `type_id`) this type backs.
    const KIND: ObjectKind;

    /// The compiled, read-side state. `Default` is the genesis state for a fresh,
    /// group-of-1 object.
    type State: Default + Clone;

    /// The fixed op table. Every op authored against this type must appear here.
    fn ops() -> &'static [OpDecl];

    /// Apply one op to the state, or reject it loudly. Pure: no IO, no clock, no
    /// ambient reads beyond `op.ctx`.
    fn reduce(state: &mut Self::State, op: &Op<'_>) -> Result<(), DeltaRejection>;

    /// The standing `member` holds in this object's own folded `roles` state, for a kind
    /// that carries the facet (`Authority::OwnerOrRole`); `None` for any other.
    fn role_of(_state: &Self::State, _member: &MemberId) -> Option<crate::group::GroupRole> {
        None
    }

    /// Whether this kind has rules beyond authority ([`ObjectType::refuses`]): only then does
    /// `authoring::build` fold the log to ask them.
    const RULES: bool = false;

    /// What this kind refuses at write beyond authority, judged on the state the author
    /// folds: a state machine's rule (the Transaction's, ICD `stateMachine`), in its words.
    /// `None` admits. `authoring::build` asks it; the reducer holds the same rule at fold.
    fn refuses(_state: &Self::State, _op_id: u32, _args: &Args, _author: &MemberId) -> Option<String> {
        None
    }

    /// Look up an op's declaration by id (default impl over [`ObjectType::ops`]).
    fn op(op_id: u32) -> Option<&'static OpDecl> {
        Self::ops().iter().find(|o| o.op_id == op_id)
    }
}

/// A typed builder helper used by `node`/CLI to author a Delta against a type+op
/// without hand-assembling the envelope (the single-write-path payload). `gen` is
/// `Some` for commutative ops (the LWW key), `None` for sequenced.
pub fn build_delta(
    kind: ObjectKind,
    op_id: u32,
    args: Args,
    epoch: u64,
    gen: Option<u64>,
) -> Delta {
    Delta {
        type_id: kind.type_id() as u32,
        op_id,
        op_version: 1,
        args,
        epoch,
        prev: crate::coordinator::GENESIS_PREV,
        seq: None,
        gen,
        // Authored PRIVATE unless the author said otherwise — the safe default,
        // and the one that costs nothing on the wire.
        visibility: crate::visibility::Visibility::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_id_roundtrip_and_names() {
        for k in [
            ObjectKind::Group,
            ObjectKind::Forum,
            ObjectKind::Field,
            ObjectKind::System,
            ObjectKind::Project,
            ObjectKind::Topic,
        ] {
            assert_eq!(ObjectKind::from_type_id(k.type_id()), Some(k));
            assert!(!k.name().is_empty());
        }
        assert_eq!(ObjectKind::from_type_id(99), None);
    }

    /// `ALL` is hand-maintained, so pin it: every entry must round-trip, the ids must be
    /// distinct, and the COUNT must match the enum. Adding a variant without adding it
    /// here fails here — which is what keeps the catalog-wide sweeps honest.
    #[test]
    fn all_lists_every_kind() {
        // 15 since 25 Sep 2026: `treasury`(31) and `note`(32) were promoted out of
        // `group`'s facets into kinds of their own, and `host`(33) out of System;
        // 16 since 30 Sep: `transaction`(34), W-98 Trade.
        // This number is hand-maintained
        // and so is `ALL` — see the doc comment. It is one of sixteen places a kind
        // must be named.
        assert_eq!(ObjectKind::ALL.len(), 16);
        let mut ids: Vec<u16> = ObjectKind::ALL.iter().map(|k| k.type_id()).collect();
        for &k in ObjectKind::ALL {
            assert_eq!(ObjectKind::from_type_id(k.type_id()), Some(k));
        }
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), ObjectKind::ALL.len(), "duplicate type_id in ALL");
    }

    #[test]
    fn spec_invariant_commutative_is_any_member() {
        let ok = OpDecl {
            op_id: 0,
            name: "vote",
            authority: Authority::AnyMember,
            commutativity: Commutativity::Commutative,
        };
        let bad = OpDecl {
            op_id: 1,
            name: "x",
            authority: Authority::Owner,
            commutativity: Commutativity::Commutative,
        };
        assert!(ok.is_well_formed());
        assert!(
            !bad.is_well_formed(),
            "a commutative owner-gated op must be rejected"
        );
    }
}

#[cfg(test)]
mod next_gen_tests {
    use super::NextGen;

    #[test]
    fn a_device_with_history_keeps_using_its_log() {
        // Months of history, a floor from the day it joined. The log wins, or the
        // device would start re-minting refs it has already used.
        assert_eq!(NextGen::compute(Some(400), 12).get(), 401);
    }

    #[test]
    fn a_device_that_joined_late_takes_the_watermark() {
        // THE CASE THAT MATTERS. Forward secrecy guarantees the log is empty, so
        // the log alone says 0 — and 0 is a ref this person has already used from
        // another device. The fold keys commutative deltas by (author_pk, gen) and
        // would drop one of the pair: a signed delta that does nothing, for ever,
        // reported as success.
        assert_eq!(NextGen::compute(None, 87).get(), 87);
        assert_ne!(NextGen::compute(None, 87).get(), 0, "0 is the collision");
    }

    #[test]
    fn a_fresh_group_starts_at_zero() {
        // No log and no floor is a group this device made itself, where 0 is
        // correct — which is why the watermark cannot simply be "always present".
        assert_eq!(NextGen::compute(None, 0).get(), 0);
    }

    #[test]
    fn the_floor_never_drags_a_running_device_backwards() {
        // A floor that could pull the answer DOWN would re-open the collision it
        // exists to close, so the rule is max and not "prefer the floor".
        assert_eq!(NextGen::compute(Some(900), 5).get(), 901);
    }

    #[test]
    fn neither_half_can_be_supplied_alone() {
        // Not an assertion about values — about the API. There is no From<u64>,
        // no public field and no Default, so a platform holding only one half
        // cannot produce a NextGen at all. "Count what I can see" type-checks as a
        // u64 and is the wrong answer; here it does not compile.
        let both = NextGen::compute(Some(3), 99);
        assert_eq!(both.get(), 99, "the larger half wins, whichever it is");
    }
}
