//! The ONE mint seam — how a new GroupObject of any kind is brought into existence.
//!
//! # Why this exists
//!
//! Every mint in the app was its own function with its own signature: `thing_mint`,
//! `event_mint`, `place_mint`, `object_new` + `group_author`, `group_forum_new`. They
//! all do the SAME TWO THINGS —
//!
//! ```text
//!     object_new(kind, name)  →  id
//!     <kind>_author(id, OP 0, <the kind's own profile args>)
//! ```
//!
//! — and they differ only in the third line: the arg VOCABULARY each kind's op 0 speaks.
//! A name is `displayName` on a Group, `title` on an Event and a Project, and `name` on
//! a Thing and a Place. Five words for one field.
//!
//! That divergence is real and it is not a mistake: each reducer's arg names belong to
//! its own schema, they are on the wire, and the ICD pins them
//! (`coordination/delta-graph.icd.json`). So this module does NOT unify the schemas.
//! It unifies the BINDER — one draft, one table that maps it onto each kind's own
//! vocabulary — which is the same call the Profile work made for the same reason.
//!
//! # The invariant this module is here to hold
//!
//! **Every kind's op 0 is its profile op.** That symmetry is real across group(18),
//! thing(27), place(28), event(29) and project(22), and `mint_op_is_always_op_zero`
//! pins it. It is what makes a unified mint a table rather than a switch of special
//! cases, and it is why the UI's `+` can be one component.
//!
//! Forum(19) is the deliberate exception: it has NO profile op (its op 0 is
//! `forum.post`), so a Forum is minted name-only — the name rides the MLS GroupContext
//! that `object_new` writes, and there is no op-0 delta at all. `profile_op` says so by
//! returning `None` rather than by a caller knowing to skip it.

use crate::coordinator::{ArgVal, Args};
use crate::object::ObjectKind;

/// The shared draft every mint is authored from — the union of what the kinds' op-0
/// args need, with the empty value meaning "absent" throughout.
///
/// One flat struct rather than an enum per kind, because it crosses the FFI and because
/// the fields are overwhelmingly shared: every kind has a name, and all but Forum have
/// a descriptor. The kind-specific tail is small, and a kind that does not read a field
/// simply does not read it — `profile_args` is the only thing that decides.
///
/// `Default` is the honest zero: an unnamed, undescribed, unplaced draft. Every field
/// that has a wire default (`shape`, `category`, `status`) reads it from the reducer's
/// own vocabulary when left empty, so this struct never invents one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MintDraft {
    /// The one field every kind has. Becomes `displayName` / `title` / `name`.
    pub name: String,
    /// Group/Place/Thing/Event `descriptor`. Empty = absent.
    pub descriptor: String,

    // ---- Post ---------------------------------------------------------------
    /// `article` | `link` | `image`. Empty = absent, which the reducer reads as
    /// `article` (its `#[default]`) — so we pass nothing rather than guessing.
    pub form: String,
    /// A `link` post's destination. Empty = absent, and the reducer refuses one
    /// on any other form.
    pub link: String,

    // ---- Group ------------------------------------------------------------
    /// `individual` | `team` | `organisation` | `community`. Empty = `team`.
    pub shape: String,
    /// The ContactCard JSON. Empty = absent.
    pub card: String,

    // ---- Thing ------------------------------------------------------------
    /// `artifact` | `skill` | `job`. Empty = absent, which the reducer reads as
    /// `artifact` (its `#[default]`) — so we pass nothing rather than guessing.
    pub category: String,

    // ---- Event ------------------------------------------------------------
    pub start_ms: i64,
    /// 0 = absent (open-ended).
    pub end_ms: i64,
    /// A venue NAME carried on the profile. The `happens_at` EDGE is a separate
    /// `event.setVenue` (op 7) against a Place id — not this.
    pub venue: String,
    /// RFC 5545 RRULE when this repeats; empty = a one-off.
    pub recurrence: String,
    /// Newline-separated artist names, billing order; empty = none.
    pub lineup: String,

    // ---- Place ------------------------------------------------------------
    /// What the caller ASSERTS about `lat`/`lng`. False mints the Place UNPLACED
    /// rather than refusing — the user's typing is not thrown away because GPS was
    /// slow indoors. The location rides a SECOND delta (`base.setLocation`), so it is
    /// not part of `profile_args`; see `Node::mint`.
    pub placed: bool,
    pub lat: f64,
    pub lng: f64,

    // ---- Project ----------------------------------------------------------
    /// `active` | `archived` | `paused`. Empty = `active`.
    pub status: String,

    // ---- Media (each rides its own op — see `Carriage::Separate`) ----------
    /// The round face. Empty = none. Caps are enforced AT FOLD, not here.
    pub icon: Vec<u8>,
    /// The wide wall. Empty = none.
    pub banner: Vec<u8>,

    // ---- Market -----------------------------------------------------------
    /// `offer` | `need` — which way round the listing faces. Rides `thing.setPosture`
    /// (op 1), not op 0, so it is `Carriage::Separate`: the mint collects it and the
    /// caller applies it as the object's opening posture. Empty = no posture, which
    /// leaves the listing invisible to the market.
    pub stance: String,
    /// Only meaningful on an ACTIVE posture — `thing.setPosture` refuses a price on a
    /// standing intent rather than dropping it. Empty = none.
    pub price: String,

    // ---- Where ------------------------------------------------------------
    /// A Place REFERENCE (object id), not a coordinate. Distinct from `placed/lat/lng`,
    /// which is a Place's OWN fix.
    pub place: String,
}

// ---------------------------------------------------------------------------
// The declaration: what each kind REQUIRES to come into existence.
// ---------------------------------------------------------------------------
//
// The UI used to decide this. A hand-written `switch kind` in Swift chose which fields
// to show, and the reducers' `req_text` / `req_int` calls decided which args had to be
// there — two independent statements of one fact, free to drift. They had, in three
// places at once: an Event could be minted with `startMs` 0 (an event in 1970) because
// the picker was never touched, and both the Place and Event surfaces collected a "Note"
// that `profile_args` never sent anywhere. Typed into, accepted, silently dropped.
//
// So the TYPE declares its own initialisation fields and the UI is built from them.
// `fields_carry_to_the_wire` then pins the half that matters: every field a mint collects
// must reach the delta.

/// A slot in the shared `MintDraft` — what a field BINDS TO, independent of what the
/// kind's op-0 calls it on the wire. `Name` is `displayName` on a Group and `title` on
/// an Event; the binder (`profile_args`) is what knows that.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MintKey {
    Name,
    Descriptor,
    Shape,
    Category,
    Start,
    /// The device's live coordinate. Rides a SECOND delta (`base.setLocation`), never op 0.
    Fix,
    /// The round profile image. `group.setCover`-adjacent but not it: an icon is the
    /// object's face, a banner is its wall.
    Icon,
    /// The wide image behind the object's headline.
    Banner,
    /// A market price. `thing.setPosture` (op 1) — and only meaningful on an ACTIVE
    /// posture, which is why the reducer refuses it on a standing intent.
    Price,
    /// Which way the listing faces — `offer` | `need`. Also `thing.setPosture` (op 1).
    Stance,
    /// RFC 5545 RRULE. Rides op 0 on an Event, unlike every other switch here.
    Recurrence,
    /// Where the object happens / is. A REFERENCE to a Place, not a coordinate.
    Place,
    /// Which of a Post's three shapes this is — `article` | `link` | `image`.
    /// Rides op 0. Without it the three are told apart by guessing at the other
    /// fields, and a lane cannot render three shapes it cannot name.
    Form,
    /// A `link` Post's destination. Rides op 0, and the reducer refuses one on
    /// any other form — a destination on an article is a contradiction, not a
    /// spare field.
    Link,
}

/// What the field is, so the surface can render a control without knowing the kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MintInput {
    /// One line of text.
    Line,
    /// A few lines. Still one field — the ceiling is three fields per mint, not three words.
    Multiline,
    /// A CLOSED vocabulary, carrying the wire values themselves. The UI shows them; the
    /// reducer parses them; there is one list, here.
    Choice(&'static [&'static str]),
    /// A moment. Required means the mint cannot proceed without one.
    Date,
    /// A READOUT of the device's fix, never a picker — a Place is evidence of having been
    /// somewhere, so the coordinate is the one the device reports.
    Fix,
    /// A circular image slot — the object's face.
    Icon,
    /// A wide image slot — the object's wall.
    Banner,
    /// An amount.
    Money,
    /// A repeat rule.
    Recurrence,
    /// A Place reference.
    Place,
}

/// Whether a collected field reaches the op-0 delta, or rides its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Carriage {
    /// Lands in `profile_args` — the common case, and what `fields_carry_to_the_wire` checks.
    Op0,
    /// Authored as a separate delta after the mint (a Place's location, the media ops).
    Separate,
    /// DECLARED AND RENDERED, BUT NOT YET DELIVERED — a placeholder the user can see and
    /// cannot yet fill in a way that persists.
    ///
    /// A third state exists so a placeholder is a stated fact rather than a field that
    /// looks live and silently drops what you type. `deferred_fields_are_the_known_placeholders`
    /// enumerates them, so one cannot be added quietly and none can be forgotten.
    Deferred,
}

/// One field a mint collects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MintField {
    pub key: MintKey,
    /// The field's NAME, as the person filling it reads it. Here rather than in the UI
    /// because it varies per KIND for the same slot — "Name" on a Space, "Title" on an
    /// Event, "Place name" on a Place — and a per-kind table in the UI is exactly the
    /// thing that drifted. It is a label, never an explanation.
    pub label: &'static str,
    pub input: MintInput,
    /// The mint cannot commit without it. This is the union of two rules: an arg the
    /// reducer demands (`event.startMs`), and a product rule the reducer has no opinion
    /// on (a nameless object is a row you can never find again — `req_text` is perfectly
    /// happy with `""`).
    pub required: bool,
    pub carriage: Carriage,
}

const fn req(key: MintKey, label: &'static str, input: MintInput) -> MintField {
    MintField { key, label, input, required: true, carriage: Carriage::Op0 }
}
const fn opt(key: MintKey, label: &'static str, input: MintInput) -> MintField {
    MintField { key, label, input, required: false, carriage: Carriage::Op0 }
}

/// An optional field that rides its OWN delta after the mint.
const fn sep(key: MintKey, label: &'static str, input: MintInput) -> MintField {
    MintField { key, label, input, required: false, carriage: Carriage::Separate }
}
/// A field that is shown and not yet persisted. See `Carriage::Deferred`.
const fn later(key: MintKey, label: &'static str, input: MintInput) -> MintField {
    MintField { key, label, input, required: false, carriage: Carriage::Deferred }
}

/// The MEDIA row: a round face and a wide wall, side by side.
///
/// `Separate` — each rides its kind's OWN media op, authored by `Node::mint` after op 0:
/// `group.setCover` (9), `event.setMedia` (4), `thing.setPhoto` (3), `post.setMedia` (1).
///
/// It is NOT declared on Place, Project or Forum, because those three have no media op —
/// there is nowhere to put it. Showing the row there anyway would be the exact bug this
/// declaration exists to prevent: a slot you can fill that silently drops what you put in
/// it. A universal row is worth having, but not at the price of lying about three kinds;
/// when those gain a media op they gain the row by adding these two lines.
const ICON: MintField = sep(MintKey::Icon, "Icon", MintInput::Icon);
const BANNER: MintField = sep(MintKey::Banner, "Banner", MintInput::Banner);
/// The WHERE row — a REFERENCE to somewhere, not a coordinate.
///
/// Only an Event can keep one today: `event.setVenue` (op 7) is the `happens_at` edge the
/// graph projector folds. The other kinds have no op for "this is at that Place", so they
/// do not offer the row — same rule as MEDIA. A Space's location is its anchor
/// affiliation, which is a different act with a different meaning.
const PLACE_REF: MintField = sep(MintKey::Place, "Where", MintInput::Place);

/// `group.setProfile` requires `displayName` AND `shape` (group.rs:654-655). The shape
/// vocabulary is the reducer's own (`GroupShape::parse`) minus `individual`, which is a
/// person, not a Space you create.
static GROUP_FIELDS: &[MintField] = &[
    req(MintKey::Name, "Name", MintInput::Line),
    ICON,
    BANNER,
    opt(MintKey::Descriptor, "Describe", MintInput::Multiline),
    // A Space's own switch. `shape` is a REQUIRED op-0 arg (group.rs:655), so it is the
    // one switch that cannot be skipped.
    req(MintKey::Shape, "Kind", MintInput::Choice(&["community", "team", "organisation"])),
];

/// `project.configure` requires `title` and `status` — but `status` is not a question to
/// ask at mint (there is one sane answer), so the binder supplies `active` and it is not
/// a FIELD. Required arg ≠ required field, and this is the distinction.
///
/// The OBJECTIVE is deliberately absent. It is `project.setObjective` (op 12), which takes
/// `headline` AND `goal` — two args, its own op, its own authority. The old wizard asked
/// for one "Objective" box and threw it away; asking for it here and sending half of it
/// would be the same bug with a nicer face. It belongs on the project's own page.
static PROJECT_FIELDS: &[MintField] = &[
    req(MintKey::Name, "Name", MintInput::Line),
];

/// `thing.setProfile` requires `name`; category folds as `artifact` when absent, so it is
/// offered but never forced.
static THING_FIELDS: &[MintField] = &[
    req(MintKey::Name, "Name", MintInput::Line),
    ICON,
    BANNER,
    opt(MintKey::Descriptor, "Describe", MintInput::Multiline),
    opt(MintKey::Category, "Kind", MintInput::Choice(&["artifact", "skill", "job"])),
    // STANCE comes AFTER kind, because KIND is what decides its vocabulary: a THING is
    // OFFERED or WANTED, a SKILL is HAD or NEEDED. The whole closed set is declared here
    // — the surface narrows it per kind, and can only ever narrow.
    //
    // `Separate` — it rides `thing.setPosture` (op 1), and without it a listing mints
    // with no posture at all and never reaches the market.
    sep(MintKey::Stance, "Stance", MintInput::Choice(&["offers", "wants", "has"])),
    // PRICE is DEFERRED, and not for want of plumbing.
    //
    // `thing.setPosture` (op 1) refuses a price unless the posture `is_active()`, which is
    // Buying or Selling — and the offered stance vocabulary is deliberately offers|wants|
    // has (the closed HOLDS/WANTS/OFFERS ruling; selling is V2). So there is no stance a
    // person can currently choose that would let a price be kept.
    //
    // Showing the row is the design and it stays. Claiming it saves would be the exact lie
    // this declaration exists to prevent, so it says `Deferred` until an active posture is
    // offerable — at which point this becomes `sep` and nothing else changes.
    later(MintKey::Price, "Price", MintInput::Money),
];

/// `place.setProfile` requires `name`. The fix is OPTIONAL on purpose: a Place minted
/// without one is UNPLACED — listed, not pinned — which is honest, and refusing to save
/// because GPS is slow indoors would lose the user's typing for a reason they cannot act on.
static PLACE_FIELDS: &[MintField] = &[
    req(MintKey::Name, "Name", MintInput::Line),
    opt(MintKey::Descriptor, "Describe", MintInput::Multiline),
    // A Place's own fix is not the PLACE row — it IS the place. No `PLACE_REF` here.
    sep(MintKey::Fix, "Here", MintInput::Fix),
];

/// `event.setProfile` requires `title` AND `startMs` (event.rs:413-414, `req_int`). The
/// date is therefore a REQUIRED field — without it the mint sent 0 and the event landed
/// in 1970.
static EVENT_FIELDS: &[MintField] = &[
    req(MintKey::Name, "Name", MintInput::Line),
    ICON,
    BANNER,
    opt(MintKey::Descriptor, "Describe", MintInput::Multiline),
    // The Event switches. `startMs` is taken with `req_int` (event.rs:414), so When is
    // REQUIRED — 0 satisfied the reducer and put the event in 1970.
    req(MintKey::Start, "When", MintInput::Date),
    // Recurrence is the one switch that rides OP 0 rather than its own delta: it is an
    // arg of `event.setProfile`, validated at reduce time so a rule we cannot expand is
    // refused rather than stored.
    opt(MintKey::Recurrence, "Recurrence", MintInput::Recurrence),
    PLACE_REF,
];

/// A Post wears the universal rows plus the ONE switch it turns out to need: which
/// of the three shapes it is. It has no schedule and no price, and it was the
/// simplest kind in the catalog until `form` — a lane that renders an article, a
/// link and an image differently cannot be handed three posts it cannot tell apart.
///
/// `Link` is offered unconditionally rather than only under `form = link`, because
/// a MintField list is static and the surface has no conditional rows. The reducer
/// is the gate: a destination on an article is refused at fold.
static POST_FIELDS: &[MintField] = &[
    req(MintKey::Name, "Name", MintInput::Line),
    ICON,
    BANNER,
    opt(MintKey::Descriptor, "Describe", MintInput::Multiline),
    // NAME, MEDIA, DESCRIBE, then the kind's own switches — the row order the
    // contract states, and the same place Group and Thing put their `Kind`.
    // The label is "Kind" because that is already the word those two use for a
    // closed vocabulary narrowing their type; a third word for one idea is how
    // a surface stops being total.
    opt(MintKey::Form, "Kind", MintInput::Choice(&["article", "link", "image"])),
    opt(MintKey::Link, "Links to", MintInput::Line),
];

/// A Forum is minted NAME-ONLY — it has no profile op at all, so its one field carries
/// nothing to a delta: the name rides the MLS GroupContext that `object_new` writes.
/// A Treasury is minted NAME-ONLY. Its op 0 is `setPolicy`, not a profile, and a
/// treasury with no policy is one nobody may spend from — the right genesis state,
/// so the mint authors nothing and the owner sets the bands deliberately.
/// A Note is minted NAME-ONLY, like a Treasury and a Forum: its op 0 is `write`,
/// which is an ENTRY. A note with no entries is an empty notebook, which is the
/// right genesis state and needs no delta to reach.
static NOTE_FIELDS: &[MintField] = &[
    MintField {
        key: MintKey::Name,
        label: "Name",
        input: MintInput::Line,
        required: true,
        carriage: Carriage::Separate,
    },
];

/// A Host is minted with a NAME, which its op 0 (`host.define`) carries. It is a
/// lazy part of a Group: minted on publication rather than with the body, because a
/// Group with no public face needs no Host and minting one eagerly would put an
/// Arc-readable object on the spine of every private group.
static HOST_FIELDS: &[MintField] = &[req(MintKey::Name, "Name", MintInput::Line)];

static TREASURY_FIELDS: &[MintField] = &[
    MintField {
        key: MintKey::Name,
        label: "Name",
        input: MintInput::Line,
        required: true,
        carriage: Carriage::Separate,
    },
];

static FORUM_FIELDS: &[MintField] = &[
    MintField {
        key: MintKey::Name,
        label: "Name",
        input: MintInput::Line,
        required: true,
        carriage: Carriage::Separate,
    },
];

/// What the UI must collect to bring this kind into existence, in the order it should be
/// asked. Empty for a kind that cannot be minted from a draft.
pub fn fields(kind: ObjectKind) -> &'static [MintField] {
    match kind {
        ObjectKind::Group => GROUP_FIELDS,
        ObjectKind::Project => PROJECT_FIELDS,
        ObjectKind::Thing => THING_FIELDS,
        ObjectKind::Place => PLACE_FIELDS,
        ObjectKind::Event => EVENT_FIELDS,
        ObjectKind::Forum => FORUM_FIELDS,
        ObjectKind::Treasury => TREASURY_FIELDS,
        ObjectKind::Note => NOTE_FIELDS,
        ObjectKind::Host => HOST_FIELDS,
        ObjectKind::Post => POST_FIELDS,
        ObjectKind::Field
        | ObjectKind::System
        | ObjectKind::Topic
        | ObjectKind::Contact
        | ObjectKind::Conversation
        | ObjectKind::Transaction => &[],
    }
}

impl MintDraft {
    /// Read the slot a field binds to. `Start` and `Fix` are not text and answer "".
    pub fn text(&self, key: MintKey) -> &str {
        match key {
            MintKey::Name => &self.name,
            MintKey::Descriptor => &self.descriptor,
            MintKey::Shape => &self.shape,
            MintKey::Category => &self.category,
            MintKey::Price => &self.price,
            MintKey::Stance => &self.stance,
            MintKey::Recurrence => &self.recurrence,
            MintKey::Place => &self.place,
            MintKey::Form => &self.form,
            MintKey::Link => &self.link,
            // Not text: an instant, a fix, and two byte blobs.
            MintKey::Start | MintKey::Fix | MintKey::Icon | MintKey::Banner => "",
        }
    }

    /// Write it. Same total match, so a new `MintKey` cannot be half-wired.
    pub fn set_text(&mut self, key: MintKey, value: &str) {
        let slot = match key {
            MintKey::Name => &mut self.name,
            MintKey::Descriptor => &mut self.descriptor,
            MintKey::Shape => &mut self.shape,
            MintKey::Category => &mut self.category,
            MintKey::Price => &mut self.price,
            MintKey::Stance => &mut self.stance,
            MintKey::Recurrence => &mut self.recurrence,
            MintKey::Place => &mut self.place,
            MintKey::Form => &mut self.form,
            MintKey::Link => &mut self.link,
            MintKey::Start | MintKey::Fix | MintKey::Icon | MintKey::Banner => return,
        };
        *slot = value.to_string();
    }

    /// Is this field answered? The commit gate is the AND of this over every required
    /// field — one rule, applied to a declaration, instead of a per-kind condition.
    ///
    /// Text is trimmed, because a spacebar is not a name. `Start` is answered by any
    /// non-zero instant: 0 is the sentinel `event.setProfile` used to receive when the
    /// picker was never touched, and it means 1970, not "now".
    pub fn is_answered(&self, key: MintKey) -> bool {
        match key {
            MintKey::Start => self.start_ms != 0,
            MintKey::Fix => self.placed,
            MintKey::Icon => !self.icon.is_empty(),
            MintKey::Banner => !self.banner.is_empty(),
            k => !self.text(k).trim().is_empty(),
        }
    }
}

/// Why a kind has no mint. Stated as a value so a new kind cannot quietly acquire
/// "no mint" by omission — `every_built_kind_is_classified` forces an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoMint {
    /// Formed by PAIRING (a double opt-in), not authored by one side. Contact(25).
    Paired,
    /// Formed by CONSUMING a Contact's prekeys, not minted directly. Conversation(26).
    Derived,
    /// Has an `ObjectType` impl but no ICD channel and no product surface. System(21).
    Internal,
    /// Declared in `ObjectKind` with no `ObjectType` impl at all. Field(20), Topic(23).
    Unbuilt,
}

/// Can this kind be minted from a draft, and if not, why not.
///
/// A TOTAL match on purpose: adding a variant to `ObjectKind` without deciding whether
/// it has a `+` is a **compile error**, not a test failure and not a silent nothing.
/// That is the strongest form this rule can take, and it is worth the exhaustive arm.
pub const fn classify(kind: ObjectKind) -> Result<(), NoMint> {
    match kind {
        ObjectKind::Group      // 18
        | ObjectKind::Forum    // 19 — name-only, no profile op (see module header)
        | ObjectKind::Project  // 22
        | ObjectKind::Thing    // 27
        | ObjectKind::Place    // 28
        | ObjectKind::Event    // 29
        | ObjectKind::Post      // 30
        | ObjectKind::Treasury  // 31 — name-only, like a Forum
        | ObjectKind::Note   // 32 — likewise
        | ObjectKind::Host => Ok(()), // 33 — minted on publication, name via host.define
        ObjectKind::Field | ObjectKind::Topic => Err(NoMint::Unbuilt),
        // 34 — the settler opens one, adding the parties and fixing their roles.
        ObjectKind::System | ObjectKind::Transaction => Err(NoMint::Internal),
        ObjectKind::Contact => Err(NoMint::Paired),
        ObjectKind::Conversation => Err(NoMint::Derived),
    }
}

/// Whether a `+` can produce this kind at all.
pub const fn is_mintable(kind: ObjectKind) -> bool {
    classify(kind).is_ok()
}

/// The op a mint authors immediately after `object_new`, and the args it carries — the
/// kind's own vocabulary, built from the shared draft.
///
/// `None` means the kind is minted NAME-ONLY: there is no op-0 delta to author, because
/// the kind's op 0 is not a profile. Forum's is a message, Treasury's a policy, Note's
/// an entry — and in each case the empty state is the right genesis state. A Host is
/// NOT among them: `host.define` carries its name, so it authors one like a Group.
///
/// Returns `None` for a non-mintable kind too. Callers reach this through `Node::mint`,
/// which rejects those loudly first — this function never needs to distinguish, because
/// "nothing to author" is the correct answer in both cases.
pub fn profile_args(kind: ObjectKind, d: &MintDraft) -> Option<(u32, Args)> {
    let mut a = Args::new();
    match kind {
        // displayName + shape are REQUIRED by the reducer (group.rs:654-655); shape
        // takes the wire default here rather than at the reducer, because `req_text`
        // has no default to fall back on.
        ObjectKind::Group => {
            a.insert("displayName".into(), text(&d.name));
            a.insert(
                "shape".into(),
                text(if d.shape.is_empty() { "team" } else { &d.shape }),
            );
            // A Space has no `descriptor` arg — its purpose rides the ContactCard's
            // `note`, which is where `mintOrg` always put it. Built HERE rather than by
            // the caller, so the declared `Descriptor` field actually reaches the wire
            // and the UI never has to know this op carries JSON.
            //
            // An explicit `card` wins: a caller with a full ContactCard (avatar, emails)
            // is not overridden by a one-line purpose.
            let card = if d.card.is_empty() {
                card_with_note(&d.descriptor)
            } else {
                d.card.clone()
            };
            put_if_set(&mut a, "card", &card);
            Some((crate::group::OP_SET_PROFILE, a))
        }

        // title + status are both required (project.rs:621-622).
        ObjectKind::Project => {
            a.insert("title".into(), text(&d.name));
            a.insert(
                "status".into(),
                text(if d.status.is_empty() { "active" } else { &d.status }),
            );
            Some((crate::project::OP_CONFIGURE, a))
        }

        ObjectKind::Thing => {
            a.insert("name".into(), text(&d.name));
            a.insert("descriptor".into(), text(&d.descriptor));
            // Absent, NOT "artifact" — `Category` derives `#[default] Artifact`, so an
            // omitted category folds identically on every pre-category log. Writing the
            // default explicitly would be a lie about what the user chose.
            put_if_set(&mut a, "category", &d.category);
            Some((crate::thing::OP_SET_PROFILE, a))
        }

        // The one builder that is already shared with its reducer — reuse it rather
        // than restating the two keys here.
        ObjectKind::Place => Some((
            crate::place::OP_SET_PROFILE,
            crate::place::set_profile_args(&d.name, &d.descriptor),
        )),

        // Likewise — `event::set_profile_args` is the canonical seven-arg builder and
        // the ICD pins its keys.
        ObjectKind::Event => Some((
            crate::event::OP_SET_PROFILE,
            crate::event::set_profile_args(
                &d.name,
                non_empty(&d.descriptor),
                d.start_ms,
                (d.end_ms > 0).then_some(d.end_ms),
                non_empty(&d.venue),
                non_empty(&d.recurrence),
                non_empty(&d.lineup),
            ),
        )),

        // A Post's op 0 carries the words. `title` is required by the reducer; `body` is
        // the prose under it and may be absent (a post can be a headline).
        ObjectKind::Post => Some((
            crate::post::OP_SET_PROFILE,
            crate::post::set_profile_args(
                &d.name,
                non_empty(&d.descriptor),
                // Empty means the caller had no opinion, so author no `form` at
                // all and let the reducer's default speak.
                if d.form.is_empty() { None } else { crate::post::Form::parse(&d.form).ok() },
                non_empty(&d.link),
            ),
        )),

        // Name-only: no profile op exists. See the module header.
        ObjectKind::Host => {
            a.insert("name".into(), text(&d.name));
            Some((crate::host::OP_DEFINE, a))
        }
        ObjectKind::Forum | ObjectKind::Treasury | ObjectKind::Note => None,

        // Not mintable — `Node::mint` refuses before reaching here.
        ObjectKind::Field
        | ObjectKind::System
        | ObjectKind::Topic
        | ObjectKind::Contact
        | ObjectKind::Conversation
        | ObjectKind::Transaction => None,
    }
}

fn text(s: &str) -> ArgVal {
    ArgVal::Text(s.to_string())
}

fn non_empty(s: &str) -> Option<&str> {
    (!s.is_empty()).then_some(s)
}

fn put_if_set(a: &mut Args, key: &str, v: &str) {
    if !v.is_empty() {
        a.insert(key.into(), text(v));
    }
}

/// A minimal ContactCard carrying one note. Empty in → empty out, because the reducer
/// parses `card` as JSON when present and `""` is not JSON.
///
/// Hand-built rather than `serde_json::to_string(&ContactCard{..})`: serialising the full
/// card would emit every default field, and a mint should write what the user said and
/// nothing else. The one value is escaped properly.
fn card_with_note(note: &str) -> String {
    let note = note.trim();
    if note.is_empty() {
        return String::new();
    }
    let escaped: String = note
        .chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            '\n' => vec!['\\', 'n'],
            '\r' => vec!['\\', 'r'],
            '\t' => vec!['\\', 't'],
            c if (c as u32) < 0x20 => vec![' '],
            c => vec![c],
        })
        .collect();
    format!("{{\"note\":\"{escaped}\"}}")
}

/// Resolve a wire kind name to its `ObjectKind`, and refuse anything else loudly.
/// The inverse of `ObjectKind::name()`.
pub fn kind_from_name(name: &str) -> Option<ObjectKind> {
    ObjectKind::ALL.iter().copied().find(|k| k.name() == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{DeltaRejection, ObjectType};

    /// The invariant the whole unified mint rests on: for every kind that HAS a profile
    /// op, that op is op **0**. If a new kind ever puts its profile somewhere else, the
    /// mint stops being a table and this test says so before the UI does.
    #[test]
    fn mint_op_is_always_op_zero() {
        for &kind in ObjectKind::ALL.iter().filter(|k| is_mintable(**k)) {
            let Some((op_id, _)) = profile_args(kind, &MintDraft::default()) else {
                // The NAME-ONLY kinds. Each has an op 0 that is not a profile — a
                // Forum's is `post`, a Treasury's `setPolicy`, a Note's `write` — and
                // in each case the empty state IS the right genesis state, so the
                // mint authors nothing and the name rides the GroupContext.
                assert!(
                    matches!(kind, ObjectKind::Forum | ObjectKind::Treasury | ObjectKind::Note),
                    "{kind:?} has no profile op"
                );
                continue;
            };
            assert_eq!(op_id, 0, "{kind:?} authors op {op_id} on mint, not 0");
        }
    }

    /// A non-mintable kind builds NO args — so a caller that ignores `classify` still
    /// cannot author a profile onto a kind that has none.
    #[test]
    fn non_mintable_kinds_build_no_args() {
        for &kind in ObjectKind::ALL.iter().filter(|k| !is_mintable(**k)) {
            assert!(
                profile_args(kind, &MintDraft::default()).is_none(),
                "{kind:?} is not mintable but built profile args"
            );
        }
    }

    /// Forum is the ONE mintable kind with no profile op, and its op 0 is `forum.post`
    /// — which is exactly why the mint must not blindly author op 0. Pin both halves.
    #[test]
    fn forum_is_name_only_and_its_op_zero_is_a_post() {
        assert!(is_mintable(ObjectKind::Forum));
        assert!(profile_args(ObjectKind::Forum, &MintDraft::default()).is_none());
        assert_eq!(crate::coordinator::ForumType::op(0).unwrap().name, "forum.post");
    }

    /// The op a mint authors must be a REAL op on that kind's own table — not a number
    /// that happens to be 0. This is what catches a mint pointed at a kind whose op 0
    /// is something other than a profile (Forum, whose op 0 is `forum.post`).
    #[test]
    fn mint_op_is_declared_by_the_kind() {
        fn decl_name(kind: ObjectKind, op: u32) -> Option<&'static str> {
            Some(match kind {
                ObjectKind::Group => crate::group::GroupType::op(op)?.name,
                ObjectKind::Project => crate::project::ProjectType::op(op)?.name,
                ObjectKind::Thing => crate::thing::ThingType::op(op)?.name,
                ObjectKind::Place => crate::place::PlaceType::op(op)?.name,
                ObjectKind::Event => crate::event::EventType::op(op)?.name,
                ObjectKind::Post => crate::post::PostType::op(op)?.name,
                ObjectKind::Host => crate::host::HostType::op(op)?.name,
                _ => return None,
            })
        }
        for &kind in ObjectKind::ALL.iter().filter(|k| is_mintable(**k)) {
            let Some((op_id, _)) = profile_args(kind, &MintDraft::default()) else {
                continue; // Forum — name-only by design
            };
            let name = decl_name(kind, op_id)
                .unwrap_or_else(|| panic!("{kind:?} op {op_id} is not on its op table"));
            // The op-0 vocabulary is not uniform, and the exceptions are each on the
            // record: a Project CONFIGURES rather than profiles, and a Host DEFINES —
            // its op 0 carries the name its owner published, which is the whole of what
            // a Host is before a face is copied onto it.
            assert!(
                name.ends_with(".setProfile") || name == "project.configure" || name == "host.define",
                "{kind:?} mints via '{name}', which is not a profile op"
            );
        }
    }

    /// A Group mint carries the two args its reducer requires, and omits the card when
    /// there is none — an empty `card` must be ABSENT, not `""`, because the reducer
    /// parses it as JSON when present.
    #[test]
    fn group_mint_args_are_reducer_shaped() {
        let d = MintDraft {
            name: "Aries".into(),
            ..Default::default()
        };
        let (op, a) = profile_args(ObjectKind::Group, &d).unwrap();
        assert_eq!(op, crate::group::OP_SET_PROFILE);
        assert_eq!(a.get("displayName"), Some(&ArgVal::Text("Aries".into())));
        assert_eq!(a.get("shape"), Some(&ArgVal::Text("team".into())));
        assert!(a.get("card").is_none(), "empty card must be absent, not \"\"");
    }

    /// An omitted category is ABSENT, so a pre-category log folds identically.
    #[test]
    fn thing_mint_omits_an_unset_category() {
        let d = MintDraft {
            name: "hammer".into(),
            ..Default::default()
        };
        let (_, a) = profile_args(ObjectKind::Thing, &d).unwrap();
        assert!(a.get("category").is_none());

        let d = MintDraft {
            category: "skill".into(),
            ..d
        };
        let (_, a) = profile_args(ObjectKind::Thing, &d).unwrap();
        assert_eq!(a.get("category"), Some(&ArgVal::Text("skill".into())));
    }

    /// The shared draft's ONE name lands on each kind's OWN key. This is the binder
    /// doing its job, and it is the single most likely thing to be got wrong.
    #[test]
    fn one_draft_name_lands_on_each_kinds_own_key() {
        let d = MintDraft {
            name: "N".into(),
            ..Default::default()
        };
        let key = |k: ObjectKind| -> Option<String> {
            let (_, a) = profile_args(k, &d)?;
            ["displayName", "title", "name"]
                .iter()
                .find(|k| a.get(**k) == Some(&ArgVal::Text("N".into())))
                .map(|s| s.to_string())
        };
        assert_eq!(key(ObjectKind::Group).as_deref(), Some("displayName"));
        assert_eq!(key(ObjectKind::Project).as_deref(), Some("title"));
        assert_eq!(key(ObjectKind::Event).as_deref(), Some("title"));
        assert_eq!(key(ObjectKind::Thing).as_deref(), Some("name"));
        assert_eq!(key(ObjectKind::Place).as_deref(), Some("name"));
    }

    // ---- the declaration ---------------------------------------------------

    /// Only mintable kinds declare fields, and every mintable kind declares at least one.
    /// A kind you can create but are asked nothing about is a bug in one direction; a
    /// kind you are asked about but cannot create is a bug in the other.
    #[test]
    fn exactly_the_mintable_kinds_declare_fields() {
        for &kind in ObjectKind::ALL {
            assert_eq!(
                !fields(kind).is_empty(),
                is_mintable(kind),
                "{kind:?}: declared fields and mintability disagree"
            );
        }
    }

    /// **The test this whole declaration exists for.** Every field a mint COLLECTS must
    /// reach the delta. Three fields did not, at once: the Place and Event surfaces each
    /// collected a "Note" that `profile_args` never sent, and a Space's purpose was
    /// hand-built into `card` by the UI. Typed into, accepted, silently dropped.
    ///
    /// Fills every `Op0` field with a value unique to that field, then asserts each one
    /// appears somewhere in the args the binder produces.
    #[test]
    fn fields_carry_to_the_wire() {
        for &kind in ObjectKind::ALL.iter().filter(|k| is_mintable(**k)) {
            let decl = fields(kind);
            let mut d = MintDraft::default();
            let mut expected: Vec<(MintKey, String)> = Vec::new();

            for (i, f) in decl.iter().enumerate() {
                if f.carriage != Carriage::Op0 {
                    continue;
                }
                match &f.input {
                    MintInput::Choice(vocab) => {
                        // A closed vocabulary: use a REAL value, since the reducer parses it.
                        let v = vocab[vocab.len() - 1];
                        d.set_text(f.key, v);
                        expected.push((f.key, v.to_string()));
                    }
                    MintInput::Date => {
                        d.start_ms = 1_700_000_000_000 + i as i64;
                    }
                    _ => {
                        let v = format!("value-{kind:?}-{i}");
                        d.set_text(f.key, &v);
                        expected.push((f.key, v));
                    }
                }
            }

            // A kind with no Op0 fields authors no op-0 delta — Forum, whose name rides
            // the MLS GroupContext. Nothing to carry, so nothing to check.
            if !decl.iter().any(|f| f.carriage == Carriage::Op0) {
                assert!(
                    profile_args(kind, &d).is_none(),
                    "{kind:?} declares no Op0 fields yet builds op-0 args"
                );
                continue;
            }
            let Some((_, args)) = profile_args(kind, &d) else {
                panic!("{kind:?} declares Op0 fields but builds no args");
            };
            let haystack: String = args
                .values()
                .map(|v| match v {
                    ArgVal::Text(t) => t.clone(),
                    ArgVal::Int(n) => n.to_string(),
                })
                .collect::<Vec<_>>()
                .join("\u{1}");

            for (key, value) in expected {
                assert!(
                    haystack.contains(&value),
                    "{kind:?}: field {key:?} is collected but never reaches the delta \
                     (args = {args:?})"
                );
            }
            // The date rides as an Int, so check it by key rather than by substring.
            if decl.iter().any(|f| f.input == MintInput::Date) {
                assert_eq!(
                    args.get("startMs"),
                    Some(&ArgVal::Int(d.start_ms)),
                    "{kind:?}: the declared Date field must reach the delta"
                );
            }
        }
    }

    /// `Separate` is the escape hatch from `fields_carry_to_the_wire`, so it is closed by
    /// hand: every field that claims to ride its own delta must be one something actually
    /// authors. Adding a `Separate` field without delivering it fails here, which is the
    /// only thing stopping the carriage flag from becoming a way to silence the test above.
    ///
    /// The two that exist, and who delivers them:
    ///   Place · Fix   → `Node::mint` authors `base.setLocation` when `placed`
    ///   Forum · Name  → `object_new` writes it into the MLS GroupContext (no delta at all)
    ///   Thing · Stance → the caller authors `thing.setPosture` (op 1) with it as the
    ///                    opening posture; a listing minted without one never reaches
    ///                    the market, which is why it is collected at mint at all.
    #[test]
    fn every_separate_field_is_actually_delivered() {
        let mut separate: Vec<(ObjectKind, MintKey)> = Vec::new();
        for &kind in ObjectKind::ALL {
            for f in fields(kind).iter().filter(|f| f.carriage == Carriage::Separate) {
                separate.push((kind, f.key));
            }
        }
        separate.sort_by_key(|(k, _)| k.type_id());
        // Delivered by, in order: the kind's own media op after op 0; `object_new`'s
        // GroupContext; `thing.setPosture` (1); `base.setLocation`; `event.setVenue` (7).
        let mut expected = vec![
            (ObjectKind::Group, MintKey::Icon), (ObjectKind::Group, MintKey::Banner),
            (ObjectKind::Forum, MintKey::Name),
            (ObjectKind::Thing, MintKey::Icon), (ObjectKind::Thing, MintKey::Banner),
            (ObjectKind::Thing, MintKey::Stance),
            (ObjectKind::Place, MintKey::Fix),
            (ObjectKind::Event, MintKey::Icon), (ObjectKind::Event, MintKey::Banner),
            (ObjectKind::Event, MintKey::Place),
            (ObjectKind::Post,  MintKey::Icon), (ObjectKind::Post,  MintKey::Banner),
            // Name-only, like a Forum: one Separate field, delivered by name.
            (ObjectKind::Treasury, MintKey::Name),
            (ObjectKind::Note, MintKey::Name),
        ];
        expected.sort_by_key(|(k, _)| k.type_id());
        assert_eq!(
            separate, expected,
            "a Separate field must be delivered by name — see this test's doc comment"
        );

    }

    /// The placeholders, named. `Deferred` is the other escape hatch from
    /// `fields_carry_to_the_wire`, so it is closed the same way: a field that is shown but
    /// does not yet persist must be listed HERE, which means adding one is a deliberate
    /// act and removing one (by landing its op) is a test failure that reminds you to.
    ///
    /// Why each is deferred:
    ///   Icon · Banner — the media ops exist on only three kinds (`group.setCover` 9,
    ///                   `event.setMedia` 4, `thing.setPhoto` 3); Place, Project and Forum
    ///                   have none. The ROW is universal by design; persistence is not.
    ///   Price         — `thing.setPosture` (1) refuses a price on a standing intent, and
    ///                   the mint collects no posture, so a price has nothing to ride.
    ///   Place         — the PLACE row is a placeholder by instruction (18 Aug).
    ///
    /// A required field may NEVER be deferred: that would gate the commit on something
    /// that cannot be saved.
    #[test]
    fn deferred_fields_are_the_known_placeholders() {
        let mut deferred: Vec<(ObjectKind, MintKey)> = Vec::new();
        for &kind in ObjectKind::ALL {
            for f in fields(kind).iter().filter(|f| f.carriage == Carriage::Deferred) {
                assert!(
                    !f.required,
                    "{kind:?} {:?} is REQUIRED and DEFERRED — the commit would gate on \
                     something that cannot be saved",
                    f.key
                );
                deferred.push((kind, f.key));
            }
        }
        let allowed = [MintKey::Icon, MintKey::Banner, MintKey::Price, MintKey::Place];
        for (kind, key) in &deferred {
            assert!(
                allowed.contains(key),
                "{kind:?} {key:?} is deferred but not a known placeholder — deliver it, or \
                 document it in this test"
            );
        }
        // MEDIA is offered exactly where a kind can STORE it. Place, Project and Forum
        // have no media op, so they do not show the row — a slot you can fill that drops
        // what you put in it is the bug this whole declaration exists to prevent.
        for &kind in ObjectKind::ALL.iter().filter(|k| is_mintable(**k)) {
            let has_media = fields(kind).iter().any(|f| f.key == MintKey::Icon);
            let can_store = matches!(
                kind,
                ObjectKind::Group | ObjectKind::Thing | ObjectKind::Event | ObjectKind::Post
            );
            assert_eq!(
                has_media, can_store,
                "{kind:?}: the MEDIA row must be offered exactly where a media op exists"
            );
        }
    }

    /// A draft answering exactly the REQUIRED fields must produce a delta the kind's own
    /// reducer accepts. This is what stops the declaration from being decorative: it is
    /// checked against `reduce` itself, not against a second copy of the rules.
    ///
    /// Only `MalformedArgs` is a failure here — authority and precondition are the
    /// Coordinator's business, and `reduce` is being called directly.
    #[test]
    fn required_fields_satisfy_the_reducer() {
        for &kind in ObjectKind::ALL.iter().filter(|k| is_mintable(**k)) {
            let mut d = MintDraft::default();
            for f in fields(kind).iter().filter(|f| f.required) {
                match &f.input {
                    MintInput::Choice(vocab) => d.set_text(f.key, vocab[0]),
                    MintInput::Date => d.start_ms = 1_700_000_000_000,
                    MintInput::Fix => d.placed = true,
                    _ => d.set_text(f.key, "answered"),
                }
            }
            let Some((op_id, args)) = profile_args(kind, &d) else {
                continue; // Forum — name-only, nothing to reduce
            };
            assert!(
                !matches!(reduce_probe(kind, op_id, &args), Err(DeltaRejection::MalformedArgs)),
                "{kind:?}: answering every required field still built a MALFORMED delta \
                 (args = {args:?})"
            );
        }
    }

    /// And the converse, for the args the reducer genuinely demands: leaving a required
    /// field blank must NOT quietly produce a valid-looking delta with a sentinel in it.
    ///
    /// `event.startMs` is the case that bit. `req_int` is satisfied by 0, so an untouched
    /// date picker minted an event in **1970** — accepted by the reducer, wrong in the
    /// world. The gate has to be `is_answered`, and this pins that 0 does not pass it.
    #[test]
    fn an_unanswered_date_is_not_a_valid_start() {
        let d = MintDraft::default();
        assert!(!d.is_answered(MintKey::Start), "0 must not read as an answered date");

        let start = fields(ObjectKind::Event)
            .iter()
            .find(|f| f.key == MintKey::Start)
            .expect("Event declares a Start field");
        assert!(start.required, "the reducer takes startMs with req_int — so it is required");

        // …and once answered it crosses intact.
        let answered = MintDraft { start_ms: 1_700_000_000_000, ..Default::default() };
        assert!(answered.is_answered(MintKey::Start));
    }

    /// Whitespace is not an answer — the gate trims, so a spacebar cannot mint an object
    /// you will never find again.
    #[test]
    fn whitespace_does_not_answer_a_field() {
        let mut d = MintDraft::default();
        d.set_text(MintKey::Name, "   \n\t ");
        assert!(!d.is_answered(MintKey::Name));
        d.set_text(MintKey::Name, " Aries ");
        assert!(d.is_answered(MintKey::Name));
    }

    /// A Space's purpose reaches the wire as the ContactCard's `note`, and an EXPLICIT
    /// card is not overridden by it.
    #[test]
    fn a_spaces_purpose_becomes_its_card_note() {
        let d = MintDraft {
            name: "Aries".into(),
            descriptor: "we build \"things\"".into(),
            ..Default::default()
        };
        let (_, a) = profile_args(ObjectKind::Group, &d).unwrap();
        let ArgVal::Text(card) = a.get("card").expect("purpose ⇒ card") else {
            panic!("card must be text")
        };
        assert_eq!(card, r#"{"note":"we build \"things\""}"#, "the note must be escaped");

        let explicit = MintDraft { card: r#"{"org":"Example Org"}"#.into(), ..d };
        let (_, a) = profile_args(ObjectKind::Group, &explicit).unwrap();
        assert_eq!(a.get("card"), Some(&ArgVal::Text(r#"{"org":"Example Org"}"#.into())));
    }

    /// A closed vocabulary is declared ONCE. Every choice the UI can offer must be one
    /// the reducer will actually parse — otherwise the picker mints malformed deltas.
    #[test]
    fn declared_choices_are_values_the_reducer_accepts() {
        for f in fields(ObjectKind::Group) {
            if let MintInput::Choice(vocab) = f.input {
                for v in vocab {
                    assert!(crate::group::GroupShape::parse(v).is_ok(), "shape '{v}'");
                }
            }
        }
        // A Thing declares TWO closed vocabularies now, and they answer to DIFFERENT
        // parsers — Kind to `Category`, Stance to `Posture`. Asserting every Thing choice
        // is a Category was true only while Kind was the only one.
        for f in fields(ObjectKind::Thing) {
            if let MintInput::Choice(vocab) = f.input {
                for v in vocab {
                    match f.key {
                        MintKey::Stance => {
                            assert!(crate::thing::Posture::parse(v).is_ok(), "posture '{v}'")
                        }
                        _ => assert!(crate::thing::Category::parse(v).is_ok(), "category '{v}'"),
                    }
                }
            }
        }
    }

    /// Run a kind's real reducer over a fresh state. Used only to ask whether the ARGS
    /// were well-formed, so authority/precondition outcomes are irrelevant.
    fn reduce_probe(
        kind: ObjectKind,
        op_id: u32,
        args: &Args,
    ) -> Result<(), DeltaRejection> {
        use crate::object::{LogPosition, MemberId, Op, ReduceContext};
        const ME: MemberId = [7u8; 32];
        let members = [ME];
        let ctx = ReduceContext { members: &members, owner: ME, epoch: 0 };
        let op = Op {
            op_id,
            args,
            author: &ME,
            pos: Some(LogPosition { epoch: 0, seq: 0 }),
            ctx: &ctx,
        };
        match kind {
            ObjectKind::Group => {
                crate::group::GroupType::reduce(&mut Default::default(), &op)
            }
            ObjectKind::Project => {
                crate::project::ProjectType::reduce(&mut Default::default(), &op)
            }
            ObjectKind::Thing => {
                crate::thing::ThingType::reduce(&mut Default::default(), &op)
            }
            ObjectKind::Place => {
                crate::place::PlaceType::reduce(&mut Default::default(), &op)
            }
            ObjectKind::Event => {
                crate::event::EventType::reduce(&mut Default::default(), &op)
            }
            ObjectKind::Post => {
                crate::post::PostType::reduce(&mut Default::default(), &op)
            }
            ObjectKind::Host => {
                crate::host::HostType::reduce(&mut Default::default(), &op)
            }
            other => panic!("no reducer probe for {other:?}"),
        }
    }

    #[test]
    fn kind_names_round_trip() {
        for &k in ObjectKind::ALL {
            assert_eq!(kind_from_name(k.name()), Some(k));
        }
        assert_eq!(kind_from_name("nonsense"), None);
        // A reserved/folded kind is not a name, so it can never be minted by string.
        assert_eq!(kind_from_name("message"), None);
    }
}
