//! The Delta surface for Swift — every op `pacific-core` folds on the four kinds
//! this facade carries, plus the reserved vocabularies, in ONE table that is held
//! to the reducers.
//!
//! # The table is PINNED, and that is the point of this file
//!
//! [`delta_catalog`] used to be a hand-written list whose provenance was a Python
//! prototype and whose only check was a reviewer's eye. It drifted, and the drift
//! was not cosmetic:
//!
//! ```text
//!   FFI op 1 = forum.retractPost        core op 1 = forum.react
//!   FFI op 2 = forum.configure          core op 2 = forum.receipt
//!   FFI op 3 = forum.addTopicArea       core op 3 = forum.vote
//!   FFI op 4 = forum.renameTopicArea    core op 4 = forum.setRoom
//!   FFI op 5 = forum.setVisibility      core op 5 = forum.clearRoom
//! ```
//!
//! An op id is a WIRE VALUE. Encoding a retraction therefore produced a delta that
//! every device folded as a reaction — the 2026-06-28 cutover folded Poll/Message/
//! Question into Forum and renumbered the band, and this table never followed.
//!
//! The pre-cutover names are gone, the ids are re-exported from core rather than
//! restated, and `catalogue_pin` (below) compares the two tables in BOTH
//! directions, exactly as `pacific-core/src/icd.rs` compares core against the ICD:
//!
//!   code -> core   a `built: true` spec must appear in that kind's
//!                  `ObjectType::ops()` with the same id, name, authority and fold.
//!   core -> code   every op a reducer declares must appear here as `built: true`.
//!                  Adding an op to a reducer and forgetting Swift now fails.
//!   the rest       a `built: false` spec may not squat on an id or a name core
//!                  uses — that is what made a retraction a reaction.
//!
//! Six of those ids are additionally `const`-asserted, so the Forum band cannot
//! part company from its reducer without failing the BUILD rather than a test.
//!
//! # What `built` means, and what it does not
//!
//! `built: true` means A REDUCER FOLDS THIS OP — machine-checked. It does NOT mean
//! the app can write it end to end: authoring needs a door on `Node` too, and those
//! are per-kind (`group_author`, `project_author`, `event_author`, …). Encoding is
//! real and total for every op in the table; `built` is the second of the three
//! gates, not the third.
//!
//! `built: false` means there is NO REDUCER. Such a delta encodes to correct
//! canonical bytes and folds nowhere, forever. Two groups are here on purpose:
//!
//! - **Field(20) and Topic(23)** — kinds with no `impl ObjectType` anywhere in
//!   pacific-core. A reserved vocabulary, kept in one place so it has one spelling
//!   when someone builds it.
//! - **System 1..=9** — the connector op-group core's `system.rs` reserves in
//!   writing, sitting between the two System ops that DO fold (0 and 10).
//!
//! Complex arg values (enums, refs, trees, structs, lists) can't ride the envelope's
//! `map<str, int|text>` as records — they lower to a single `text` arg holding JSON
//! (or a hex id / enum raw-value). Each such arg's `note` says how.

use std::collections::BTreeMap;

use base64::prelude::{Engine as _, BASE64_STANDARD};
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::object::{build_delta, ObjectKind};

use crate::FfiError;

// ============================================================================
// The canonical op-id numbering (the one decision this module fixes).
// ============================================================================

/// op-ids per kind.
///
/// EVERY id that core declares is RE-EXPORTED from core rather than restated here:
/// a constant that is a copy is a constant that drifts, and this module spent from
/// the 2026-06-28 catalogue cutover until 15 Sep 2026 doing exactly that on Forum
/// (`FORUM_RETRACT_POST = 1` against core's `FORUM_REACT = 1`, and four more below
/// it), so an iOS retraction encoded as a reaction.
///
/// What is NOT a re-export is the System connector band, 1..=9, which core's
/// `system.rs` reserves in writing for an op-group that has no reducer yet. Those
/// are declared here, marked `built: false`, and pinned by `catalogue_pin` to stay
/// out of every id core actually uses.
pub mod op {
    // ---- Group(18) — pinned to the reducer's own constants -------------------
    pub const GROUP_SET_PROFILE: u32 = pacific_core::group::OP_SET_PROFILE;
    pub const GROUP_SET_PRESENCE: u32 = pacific_core::group::OP_SET_PRESENCE;
    pub const GROUP_STORE_CREDENTIAL: u32 = pacific_core::group::OP_STORE_CREDENTIAL;
    pub const GROUP_REVOKE_CREDENTIAL: u32 = pacific_core::group::OP_REVOKE_CREDENTIAL;
    // Standing is a facet now (`crate::roles`), carried by every kind that wants
    // it rather than by Group alone.
    pub const BASE_SET_ROLE: u32 = pacific_core::roles::OP_SET_ROLE;
    pub const BASE_SET_PARENT: u32 = pacific_core::parent::OP_SET_PARENT;
    pub const BASE_CLEAR_PARENT: u32 = pacific_core::parent::OP_CLEAR_PARENT;
    pub const BASE_CLEAR_ROLE: u32 = pacific_core::roles::OP_CLEAR_ROLE;
    pub const GROUP_SET_AFFILIATION: u32 = pacific_core::group::OP_SET_AFFILIATION;
    pub const GROUP_CLEAR_AFFILIATION: u32 = pacific_core::group::OP_CLEAR_AFFILIATION;
    pub const BASE_SET_PART: u32 = pacific_core::parts::OP_SET_PART;
    // Each member's own card (O-77): the profiles facet, on a Group and a Forum.
    pub const BASE_PUBLISH_PROFILE: u32 = pacific_core::profiles::OP_PUBLISH_PROFILE;
    pub const BASE_CLEAR_PART: u32 = pacific_core::parts::OP_CLEAR_PART;
    pub const GROUP_JOINED_OBJECT: u32 = pacific_core::group::OP_JOINED_OBJECT;
    pub const GROUP_LEFT_OBJECT: u32 = pacific_core::group::OP_LEFT_OBJECT;
    pub const GROUP_SET_COVER: u32 = pacific_core::group::OP_SET_COVER;
    pub const GROUP_SET_OFFICE: u32 = pacific_core::group::OP_SET_OFFICE;
    pub const GROUP_CLEAR_OFFICE: u32 = pacific_core::group::OP_CLEAR_OFFICE;
    pub const GROUP_SET_FACE: u32 = pacific_core::group::OP_SET_FACE;
    pub const GROUP_EDIT_FACE: u32 = pacific_core::group::OP_EDIT_FACE;
    pub const GROUP_PUBLISH_LISTING: u32 = pacific_core::group::OP_PUBLISH_LISTING;
    pub const GROUP_REMOVE_LISTING: u32 = pacific_core::group::OP_REMOVE_LISTING;
    pub const GROUP_SET_CLAIM_ISSUER: u32 = pacific_core::group::OP_SET_CLAIM_ISSUER;
    pub const GROUP_CLEAR_CLAIM_ISSUER: u32 = pacific_core::group::OP_CLEAR_CLAIM_ISSUER;
    pub const GROUP_RSVP: u32 = pacific_core::group::OP_RSVP;
    pub const GROUP_SET_REGISTRATION: u32 = pacific_core::group::OP_SET_REGISTRATION;
    pub const GROUP_RSVP_DECIDE: u32 = pacific_core::group::OP_RSVP_DECIDE;
    pub const BASE_PUBLISH_ABOUT: u32 = pacific_core::about::OP_PUBLISH_ABOUT;
    pub const BASE_DEFINE_QUESTION: u32 = pacific_core::questions::OP_DEFINE_QUESTION;
    pub const BASE_RETIRE_QUESTION: u32 = pacific_core::questions::OP_RETIRE_QUESTION;
    pub const BASE_ANSWER_QUESTION: u32 = pacific_core::questions::OP_ANSWER_QUESTION;

    // ---- Forum(19) / Conversation(26) — the chat band ------------------------
    //
    // THE CUTOVER LANDED HERE, LATE. Poll(16), Message(17) and Question(24) folded
    // into Forum on 2026-06-28 and the band was renumbered; this table kept the
    // pre-cutover names until 15 Sep 2026. Re-exported now, so the ids cannot part
    // company again.
    pub const FORUM_POST: u32 = pacific_core::coordinator::FORUM_POST;
    pub const FORUM_REACT: u32 = pacific_core::coordinator::FORUM_REACT;
    pub const FORUM_RECEIPT: u32 = pacific_core::coordinator::FORUM_RECEIPT;
    pub const FORUM_VOTE: u32 = pacific_core::coordinator::FORUM_VOTE;
    pub const FORUM_RETRACT: u32 = pacific_core::coordinator::FORUM_RETRACT;
    pub const FORUM_EDIT_DESCRIPTION: u32 = pacific_core::coordinator::FORUM_EDIT_DESCRIPTION;
    // 4 and 5 were forum.setRoom / forum.clearRoom: a room is a part now (base.setPart).
    // Never reassigned (A-13).

    // ---- Field(20) — prototype order (setLiteral is 1, not 6) ----------------
    //
    // A RESERVED VOCABULARY, not a surface: kind 20 has no `impl ObjectType` in
    // pacific-core, so no op table, no reducer and no author door. Every Field spec
    // is `built: false` and `catalogue_pin` holds it there.
    pub const FIELD_SET_KEYINDEX: u32 = 0;
    pub const FIELD_SET_LITERAL: u32 = 1;
    pub const FIELD_DEFINE_DEEPLINK: u32 = 2;
    pub const FIELD_DEFINE_EXPRESSION: u32 = 3;
    pub const FIELD_RECORD_RESOLUTION: u32 = 4;
    pub const FIELD_SET_ROLE: u32 = 5;
    pub const FIELD_SUPPRESS: u32 = 6;
    pub const FIELD_UNSUPPRESS: u32 = 7;

    // ---- System(21) ----------------------------------------------------------
    // 0 and 10 are BUILT (`pacific_core::system`); 1..=9 are the connector
    // op-group core reserves in writing and nothing folds yet.
    pub const SYSTEM_DEFINE: u32 = pacific_core::system::OP_DEFINE;
    pub const SYSTEM_HYDRATE: u32 = pacific_core::system::OP_HYDRATE;
    pub const SYSTEM_ADD_FIELD: u32 = 1;
    pub const SYSTEM_REMOVE_FIELD: u32 = 2;
    pub const SYSTEM_SET_CONNECTOR: u32 = 3;
    pub const SYSTEM_ADD_OPERATION: u32 = 4;
    pub const SYSTEM_REMOVE_OPERATION: u32 = 5;
    pub const SYSTEM_BIND_CREDENTIAL: u32 = 6;
    pub const SYSTEM_REVOKE_CREDENTIAL: u32 = 7;
    pub const SYSTEM_SUPPRESS_OPERATION: u32 = 8;
    pub const SYSTEM_RESOLVE_VALUE: u32 = 9;

    // ---- Project(22) — op-ids sourced from `pacific_core::project` ------------
    pub const PROJECT_CONFIGURE: u32 = pacific_core::project::OP_CONFIGURE;
    pub const PROJECT_ADD_TIMELINE_ITEM: u32 = pacific_core::project::OP_ADD_TIMELINE_ITEM;
    pub const PROJECT_SUPPRESS_ITEM: u32 = pacific_core::project::OP_SUPPRESS_ITEM;
    pub const PROJECT_SET_ITEM_SCHEDULE: u32 = pacific_core::project::OP_SET_ITEM_SCHEDULE;
    pub const PROJECT_SET_ASSIGNEE: u32 = pacific_core::project::OP_SET_ASSIGNEE;
    pub const PROJECT_ADD_DEPENDENCY: u32 = pacific_core::project::OP_ADD_DEPENDENCY;
    pub const PROJECT_REMOVE_DEPENDENCY: u32 = pacific_core::project::OP_REMOVE_DEPENDENCY;
    pub const PROJECT_SUBSCRIBE: u32 = pacific_core::project::OP_SUBSCRIBE;
    pub const PROJECT_UNSUBSCRIBE: u32 = pacific_core::project::OP_UNSUBSCRIBE;
    pub const PROJECT_SET_ITEM_PROGRESS: u32 = pacific_core::project::OP_SET_ITEM_PROGRESS;
    pub const PROJECT_SET_ITEM_FIELD: u32 = pacific_core::project::OP_SET_ITEM_FIELD;
    pub const PROJECT_TOUCH_SUBSCRIPTION: u32 = pacific_core::project::OP_TOUCH_SUBSCRIPTION;
    // Objectives (Tab 1)
    pub const PROJECT_SET_OBJECTIVE: u32 = pacific_core::project::OP_SET_OBJECTIVE;
    pub const PROJECT_SET_ROLE: u32 = pacific_core::project::OP_SET_ROLE;
    pub const PROJECT_ADD_STAKEHOLDER: u32 = pacific_core::project::OP_ADD_STAKEHOLDER;
    pub const PROJECT_REMOVE_STAKEHOLDER: u32 = pacific_core::project::OP_REMOVE_STAKEHOLDER;
    pub const PROJECT_ADD_LOCATION: u32 = pacific_core::project::OP_ADD_LOCATION;
    pub const PROJECT_REMOVE_LOCATION: u32 = pacific_core::project::OP_REMOVE_LOCATION;
    pub const PROJECT_SET_KPI: u32 = pacific_core::project::OP_SET_KPI;
    pub const PROJECT_REMOVE_KPI: u32 = pacific_core::project::OP_REMOVE_KPI;

    // ---- Topic(23) -----------------------------------------------------------
    //
    // Reserved on the same terms as Field: no `impl ObjectType`, no reducer, no
    // author door. Declared so the vocabulary has one home, never `built`.
    pub const TOPIC_CHARTER: u32 = 0;
    pub const TOPIC_SET_CADENCE: u32 = 1;
    pub const TOPIC_CONNECT: u32 = 2;
    pub const TOPIC_DISCONNECT: u32 = 3;
    pub const TOPIC_ADD_FINDING: u32 = 4;
    pub const TOPIC_RETRACT_FINDING: u32 = 5;
    pub const TOPIC_POSE_QUESTION: u32 = 6;
    pub const TOPIC_RESOLVE_QUESTION: u32 = 7;

    // ---- the base op-groups, spliced onto every Group-typed object -----------
    // Re-exported from core rather than re-declared, so an id can never drift.
    // Each lives in its own 0xF00n_ band precisely so it cannot collide with a
    // kind's own low-numbered ops — the trap Forum fell into.
    pub const MEMBER_JOINED: u32 = pacific_core::membership::OP_MEMBER_JOINED;
    pub const MEMBER_LEFT: u32 = pacific_core::membership::OP_MEMBER_LEFT;
    pub const OWNER_HANDOVER: u32 = pacific_core::membership::OP_OWNER_HANDOVER;
    pub const CLAIM_SPENT: u32 = pacific_core::membership::OP_CLAIM_SPENT;

    pub const BASE_PUBLISH: u32 = pacific_core::publication::OP_PUBLISH;
    pub const BASE_UNPUBLISH: u32 = pacific_core::publication::OP_UNPUBLISH;

    pub const BASE_SET_WALLET_POLICY: u32 = pacific_core::wallet::OP_SET_POLICY;
    pub const BASE_RECORD_DEPOSIT: u32 = pacific_core::wallet::OP_RECORD_DEPOSIT;
    pub const BASE_ATTEST_SETTLEMENT: u32 = pacific_core::wallet::OP_ATTEST_SETTLEMENT;
    pub const BASE_ATTEST_BALANCE: u32 = pacific_core::wallet::OP_ATTEST_BALANCE;

    pub const BASE_NOTE_WRITE: u32 = pacific_core::note::OP_NOTE_WRITE;
    pub const BASE_NOTE_RETRACT: u32 = pacific_core::note::OP_NOTE_RETRACT;
    pub const BASE_NOTE_COMMENT: u32 = pacific_core::note::OP_NOTE_COMMENT;
    pub const BASE_NOTE_REACT: u32 = pacific_core::note::OP_NOTE_REACT;
    pub const BASE_NOTE_PROMOTE: u32 = pacific_core::note::OP_NOTE_PROMOTE;
}

// ---------------------------------------------------------------------------
// THE COLLISION, PINNED AT COMPILE TIME.
//
// `catalogue_pin` below is the full both-directions check and it is a test, so it
// runs when somebody runs tests. These five run when somebody TYPES, which is the
// right cost for the five ids that were actually wrong: a Forum op that drifts off
// its reducer again does not get as far as a test failure.
// ---------------------------------------------------------------------------
const _: () = assert!(op::FORUM_POST == 0, "forum.post is pinned to 0");
const _: () = assert!(op::FORUM_REACT == 1, "forum op 1 is react, not retractPost");
const _: () = assert!(op::FORUM_RECEIPT == 2, "forum op 2 is receipt, not configure");
const _: () = assert!(op::FORUM_VOTE == 3, "forum op 3 is vote, not addTopicArea");
// The System connector band is a PROMISE that it stays out of the built ids.
const _: () = assert!(op::SYSTEM_DEFINE == 0 && op::SYSTEM_HYDRATE == 10);

// ============================================================================
// UniFFI value types
// ============================================================================

/// The six GroupObject kinds (mirrors `pacific_core::object::ObjectKind`).
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Group,
    Forum,
    Field,
    System,
    Project,
    Topic,
}

impl Kind {
    fn core(self) -> ObjectKind {
        match self {
            Kind::Group => ObjectKind::Group,
            Kind::Forum => ObjectKind::Forum,
            Kind::Field => ObjectKind::Field,
            Kind::System => ObjectKind::System,
            Kind::Project => ObjectKind::Project,
            Kind::Topic => ObjectKind::Topic,
        }
    }
    fn type_id(self) -> u32 {
        self.core().type_id() as u32
    }
}

/// Who may author an op.
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Authority {
    AnyMember,
    Owner,
    /// The owner, or a member holding admin in the object's own roles (ICD 2.1.0 row 10).
    OwnerOrAdmin,
    /// The owner, or a member holding admitter in the object's own roles (NC-135).
    OwnerOrAdmitter,
}

/// How an op folds.
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fold {
    Sequenced,
    Commutative,
}

/// The two envelope arg value shapes (`map<str, int|text>`).
#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum ArgValue {
    Text { value: String },
    Int { value: i64 },
}

impl ArgValue {
    fn core(&self) -> ArgVal {
        match self {
            ArgValue::Text { value } => ArgVal::Text(value.clone()),
            ArgValue::Int { value } => ArgVal::Int(*value),
        }
    }
}

/// One key/value pair of a delta's args.
#[derive(uniffi::Record, Clone, Debug)]
pub struct ArgEntry {
    pub key: String,
    pub value: ArgValue,
}

fn text(key: &str, value: String) -> ArgEntry {
    ArgEntry {
        key: key.to_string(),
        value: ArgValue::Text { value },
    }
}
fn int(key: &str, value: i64) -> ArgEntry {
    ArgEntry {
        key: key.to_string(),
        value: ArgValue::Int { value },
    }
}
fn opt_text(key: &str, value: Option<String>) -> Option<ArgEntry> {
    value.map(|v| text(key, v))
}

/// A delta ready to encode: which op, and its args. Produced by the typed
/// constructors below (or hand-built for a generic path).
#[derive(uniffi::Record, Clone, Debug)]
pub struct DeltaDraft {
    pub kind: Kind,
    pub op_id: u32,
    pub args: Vec<ArgEntry>,
}

fn draft(kind: Kind, op_id: u32, args: Vec<Option<ArgEntry>>) -> DeltaDraft {
    DeltaDraft {
        kind,
        op_id,
        args: args.into_iter().flatten().collect(),
    }
}

/// Lower a draft's args to the core `Args` map — the authoring FFI path
/// (`Core::project_author`) needs the map, not just the canonical bytes.
pub(crate) fn draft_args(draft: &DeltaDraft) -> Args {
    draft
        .args
        .iter()
        .map(|e| (e.key.clone(), e.value.core()))
        .collect()
}

/// A canonically-encoded delta: content address + the bytes that get stored/sent.
#[derive(uniffi::Record, Clone, Debug)]
pub struct EncodedDelta {
    /// SHA-256 of the canonical bytes (hex) — the DeltaId / hash-chain target.
    pub delta_id: String,
    pub type_id: u32,
    pub op_id: u32,
    /// The canonical CBOR envelope, base64-encoded.
    pub cbor_base64: String,
}

/// One op's declaration — the metadata the app can enumerate.
#[derive(uniffi::Record, Clone, Debug)]
pub struct OpSpec {
    pub kind: Kind,
    pub type_id: u32,
    pub op_id: u32,
    /// Fully-qualified op name, e.g. "group.setProfile", "forum.poll.vote".
    pub name: String,
    pub authority: Authority,
    pub fold: Fold,
    /// True iff a reducer folds this op today (only forum.post).
    pub built: bool,
    /// The arg keys this op expects (see the constructors for types/encoding).
    pub arg_keys: Vec<String>,
    /// Encoding + status notes (JSON-encoded complex args, cutover caveats, …).
    pub note: String,
}

// ============================================================================
// The catalog — every op, enumerable from Swift.
// ============================================================================

fn spec(
    kind: Kind,
    op_id: u32,
    name: &str,
    authority: Authority,
    fold: Fold,
    built: bool,
    arg_keys: &[&str],
    note: &str,
) -> OpSpec {
    OpSpec {
        kind,
        type_id: kind.type_id(),
        op_id,
        name: name.to_string(),
        authority,
        fold,
        built,
        arg_keys: arg_keys.iter().map(|s| s.to_string()).collect(),
        note: note.to_string(),
    }
}

/// The complete delta catalogue: every op `pacific-core` folds on the four kinds
/// this facade surfaces, plus the two reserved vocabularies (Field, Topic) and the
/// System connector band that core reserves and nothing folds yet.
///
/// `built` is not a comment. `catalogue_pin` asserts that a `built: true` spec
/// appears in that kind's `ObjectType::ops()` with the same id, name, authority and
/// fold, that every core op appears here, and that no `built: false` spec squats on
/// an id or a name core is using. Adding an op to a reducer without adding it here
/// now fails; so does adding one here that no reducer folds.
#[uniffi::export]
pub fn delta_catalog() -> Vec<OpSpec> {
    use Authority::{AnyMember, Owner, OwnerOrAdmin, OwnerOrAdmitter};
    use Fold::{Commutative, Sequenced};
    use Kind::*;
    vec![
        // ---- Group(18) ----
        spec(Group, op::GROUP_SET_PROFILE, "group.setProfile", Owner, Sequenced, true,
             &["displayName", "shape", "card"], "shape ∈ {individual,team,organisation,community}; optional card=JSON"),
        spec(Group, op::GROUP_SET_PRESENCE, "group.setPresence", Owner, Sequenced, true,
             &["kind", "identityKey", "inviteHint", "spaceId"], "kind ∈ {onPlatform,offPlatform}; identityKey iff onPlatform"),
        spec(Group, op::GROUP_STORE_CREDENTIAL, "group.storeCredential", Owner, Sequenced, true,
             &["id", "kind", "label"], "NEVER the secret — only a handle id/kind/label crosses"),
        spec(Group, op::GROUP_REVOKE_CREDENTIAL, "group.revokeCredential", Owner, Sequenced, true,
             &["id"], "idempotent"),
        spec(Group, op::BASE_SET_ROLE, "base.setRole", Owner, Sequenced, true,
             &["member", "role"], "role ∈ {owner,admin,member,viewer,guest}; member must be in the roster"),
        spec(Group, op::GROUP_SET_AFFILIATION, "group.setAffiliation", Owner, Sequenced, true,
             &["peer", "rel", "name", "tether", "at"], "Group↔peer edge (this side's half). rel ∈ {parent,child,peer,anchored,created}; peer = the other object's id (hex — a group, or for anchored/created a Place/Event/Thing); at = event unix ms; optional tether = group-tether object id"),
        spec(Group, op::GROUP_CLEAR_AFFILIATION, "group.clearAffiliation", Owner, Sequenced, true,
             &["peer"], "removes this side's half of the edge; the peer group clears its own"),
        spec(Group, op::BASE_PUBLISH_PROFILE, "base.publishProfile", AnyMember, Commutative, true,
             &["card", "gen"], "the author's own card in this group: a name and optionally a picture; core publishes it from the self record (O-77)"),
        spec(Group, op::BASE_SET_PART, "base.setPart", Owner, Sequenced, true,
             &["part", "role", "at", "choice"], "name an object this Group is made of, in a role: room, treasury, host, a sub-group. part = its object id (hex); who is IN it stays its own MLS roster; its name is its own GroupContext's; at = event unix ms"),
        spec(Group, op::BASE_CLEAR_PART, "base.clearPart", Owner, Sequenced, true,
             &["part"], "detach the part; the object, its log and its roster are untouched"),
        spec(Group, op::GROUP_JOINED_OBJECT, "group.joinedObject", Owner, Sequenced, true,
             &["object", "kind", "arc", "tag", "at"], "on MY OWN identity record: I joined an object, and where the way back into it lives. The reciprocal of base.memberJoined, which sits on the joined object and names who arrived. Owner-sequenced on my own spine, so nobody may assert a membership on my behalf. tag addresses that object's sealed way-in material, which is key material and never enters the object graph"),
        spec(Group, op::GROUP_LEFT_OBJECT, "group.leftObject", Owner, Sequenced, true,
             &["object", "reason", "at"], "on MY OWN identity record: I left an object. The record is KEPT and dated, so \"was I ever in X\" stays answerable and re-joining needs no tombstone. Leaving ITSELF is the MLS Remove (Node::group_leave), which does revoke: the removed leaf derives no later epoch. This op only records it on my own identity record"),
        spec(Group, op::GROUP_SET_COVER, "group.setCover", Owner, Sequenced, true,
             &["data", "mime"], "the cover banner, ONE slot wholesale-replaced. mime ∈ {image/jpeg,image/png,image/gif,video/mp4}; stills/gifs ≤ 700K b64, mp4 ≤ 1.5M; empty data clears"),
        spec(Group, op::GROUP_SET_OFFICE, "group.setOffice", Owner, Sequenced, true,
             &["office", "holder", "status", "at"], "an office is an OVERLAY on a real member — holder must already be in the roster. status ∈ {pending,filled}; at = event unix ms"),
        spec(Group, op::GROUP_CLEAR_OFFICE, "group.clearOffice", Owner, Sequenced, true,
             &["office"], "vacate; vacating an office nobody holds fails loudly"),
        spec(Group, op::BASE_CLEAR_ROLE, "base.clearRole", Owner, Sequenced, true,
             &["member"], "withdraw a standing. The member stays on the roster — a role is an OVERLAY on it, and removing the overlay is not removing the person"),
        spec(Group, op::GROUP_SET_FACE, "group.setFace", Owner, Sequenced, true,
             &["face"], "this group's public page as a JSON document — the WORKING COPY. Members see it, it is never served; publishing copies it across the seam onto the group's Host. Empty clears; over 16 KiB or not a JSON object is REFUSED rather than trimmed, because a clipped document would parse on some devices and not others"),
        spec(Group, op::GROUP_EDIT_FACE, "group.editFace", OwnerOrAdmin, Commutative, true,
             &["face", "gen"], "the shared face: one LWW register by (gen, author) over the sequenced one; an admin's write counts while they hold admin"),
        spec(Group, op::GROUP_PUBLISH_LISTING, "group.publishListing", AnyMember, Commutative, true,
             &["gen", "thingId", "posture", "title", "reach", "rev", "descriptor", "price", "deadline", "area", "withdrawn", "photo", "photoMime", "site"], "a member lists one of their Things on the Site's Trade board: the latest by gen per (lister, Thing); withdrawn takes it down; site 1 is the Site's own listing, the owner's or an admin's (W-98 Trade, T-7)"),
        spec(Group, op::GROUP_REMOVE_LISTING, "group.removeListing", OwnerOrAdmin, Commutative, true,
             &["author", "thingId", "gen"], "the owner or an admin takes a member's listing off the board, for good (W-98 Trade, T-7)"),
        spec(Group, op::GROUP_SET_CLAIM_ISSUER, "group.setClaimIssuer", Owner, Sequenced, true,
             &["kid", "key"], "register a kiosk's claim-signing key on the Site (A-3): key = base64url of the 32-byte Ed25519 public key, kid = the first 8 bytes of its sha256, hex; a kid that is not its key's is refused"),
        spec(Group, op::GROUP_CLEAR_CLAIM_ISSUER, "group.clearClaimIssuer", Owner, Sequenced, true,
             &["kid"], "retire a kiosk's key: claims it signed admit no one from here on"),
        // ---- W-98: the Site's answers to its Events; the about and questions facets ----
        spec(Group, op::GROUP_RSVP, "group.rsvp", AnyMember, Commutative, true,
             &["event", "status", "guests", "at", "gen", "guestNames", "occurrence"], "a member's answer to an Event this group created (its rel=created edge): going, maybe or declined, with guests; one register per (author, event), the later gen wins. The fold holds it to the event's register: closesMs by `at`, maybe, guestsMax, and capacity (an answer and its guests take places), past which it waits (waitlist 1) or is refused"),
        spec(Group, op::GROUP_SET_REGISTRATION, "group.setRegistration", OwnerOrAdmin, Commutative, true,
             &["event", "capacity", "waitlist", "approval", "guestsMax", "maybe", "closesMs", "location", "gen"], "how the Site takes answers for one Event it created: one register per event, LWW; it binds the answers folded after it"),
        spec(Group, op::GROUP_RSVP_DECIDE, "group.rsvpDecide", OwnerOrAdmin, Commutative, true,
             &["event", "member", "decision", "gen"], "the host's decision on one member's answer: approved counts as going, waitlisted waits for a place, declined is not admitted; LWW per (event, member)"),
        spec(Group, op::BASE_PUBLISH_ABOUT, "base.publishAbout", AnyMember, Commutative, true,
             &["bio", "links", "gen"], "the author's own bio and https links in this Site, within the ICD's caps; per-author LWW; empty bio and no links clears"),
        spec(Group, op::BASE_DEFINE_QUESTION, "base.defineQuestion", OwnerOrAdmin, Commutative, true,
             &["text", "options", "multi", "free", "gen", "max", "hint", "textMax"], "a question to the Site's members, free text, options or both; it is its (author, gen), and counts while its author is the owner or an admin"),
        spec(Group, op::BASE_RETIRE_QUESTION, "base.retireQuestion", OwnerOrAdmin, Commutative, true,
             &["target_author", "target_gen", "gen"], "close a question to new answers; it and its answers stay, marked retired"),
        spec(Group, op::BASE_ANSWER_QUESTION, "base.answerQuestion", AnyMember, Commutative, true,
             &["target_author", "target_gen", "text", "choices", "gen"], "the author's answer to one question: free text where it takes it, option indexes where it has options; per (author, question) LWW; empty clears; refused on a retired question"),
        // ---- base parent op-group (BUILT — pacific_core::parent) ----
        //
        // The other half of `part_of`, written in the PART. Every kind can be a part
        // (ICD `facets.parent`): a room, a comments section, a Treasury, a Host, a
        // sub-group. The kinds this facade surfaces each carry it.
        spec(Forum, op::BASE_SET_PARENT, "base.setParent", Owner, Sequenced, true,
             &["parent", "role", "at"], "name the object this one is a part of, and in what role. The parent's half is its parts op; both are written by the same mint, by the one principal that owns both at that moment"),
        spec(Forum, op::BASE_CLEAR_PARENT, "base.clearParent", Owner, Sequenced, true,
             &["parent"], "detach. The parent is untouched — it clears its own half with its parts op"),
        spec(Group, op::BASE_SET_PARENT, "base.setParent", Owner, Sequenced, true,
             &["parent", "role", "at"], "name the object this group is a part of — a sub-group of a group — and in what role"),
        spec(Group, op::BASE_CLEAR_PARENT, "base.clearParent", Owner, Sequenced, true,
             &["parent"], "detach. The parent is untouched"),
        spec(System, op::BASE_SET_PARENT, "base.setParent", Owner, Sequenced, true,
             &["parent", "role", "at"], "name the object this one is a part of, and in what role"),
        spec(System, op::BASE_CLEAR_PARENT, "base.clearParent", Owner, Sequenced, true,
             &["parent"], "detach. The parent is untouched"),
        spec(Project, op::BASE_SET_PARENT, "base.setParent", Owner, Sequenced, true,
             &["parent", "role", "at"], "name the object this one is a part of, and in what role"),
        spec(Project, op::BASE_CLEAR_PARENT, "base.clearParent", Owner, Sequenced, true,
             &["parent"], "detach. The parent is untouched"),
        // ---- Forum(19) — the chat band, and the whole of it ----
        //
        // These six ARE the Forum catalogue. `retractPost`/`configure`/`addTopicArea`/
        // `renameTopicArea`/`setVisibility` and the `poll.*`/`question.*` groups stood
        // here until 15 Sep 2026 on ids 1..10 with no reducer behind any of them, so
        // `forum.retractPost` encoded as `forum.react` and iOS retracting a post
        // authored a reaction. They are gone rather than renumbered: an op-id is a
        // wire value and the band belongs to the ops that fold.
        spec(Forum, op::FORUM_POST, "forum.post", AnyMember, Commutative, true,
             &["text", "gen", "reply_author", "reply_gen", "ts", "media", "mediaMime", "mediaVia", "mediaKind", "mediaDigest", "mediaBytes", "mediaSession", "mediaW", "mediaH", "mediaMs", "mediaKey", "mediaSecret"], "gen is core-assigned. Optional: ts (unix ms), reply_author+reply_gen (the MsgRef this answers), media* (a MediaRef flattened under `media`, bounded at fold; mediaKey and mediaSecret with a detached ref, O-79)"),
        spec(Forum, op::FORUM_REACT, "forum.react", AnyMember, Commutative, true,
             &["target_author", "target_gen", "emoji", "active", "gen", "target"],
             "target = the (author,gen) MsgRef of the message reacted to; active int 0|1 and active:0 (or an empty emoji) CLEARS. LWW per reactor by this react's own gen"),
        spec(Forum, op::FORUM_RECEIPT, "forum.receipt", AnyMember, Commutative, true,
             &["status", "refs", "upto", "gen"],
             "one receipt acknowledges a BATCH. status int: 1=delivered 2=read. refs = \"<authorHex>:<gen>\" joined by commas. A grow-only lattice per (target,receiptor): the MAX status wins, so no gen tiebreak is needed"),
        spec(Forum, op::FORUM_VOTE, "forum.vote", AnyMember, Commutative, true,
             &["target_author", "target_gen", "dir", "gen"],
             "dir int ∈ {-1,0,+1}; 0 clears, anything else is malformed and drops whole. LWW per voter by this vote's own gen"),
        spec(Forum, op::FORUM_RETRACT, "forum.retract", AnyMember, Commutative, true,
             &["target_author", "target_gen", "gen"],
             "the message's author withdraws it, the owner hides it; anyone else is refused. Grow-only"),
        spec(Forum, op::FORUM_EDIT_DESCRIPTION, "forum.editDescription", OwnerOrAdmin, Commutative, true,
             &["description", "gen"], "the room's description: one LWW register by (gen, author); the owner's always counts, an admin of this room's while they hold admin. Over the ICD's maxBytes refused, never truncated; empty clears"),
        spec(Forum, op::BASE_SET_PART, "base.setPart", Owner, Sequenced, true,
             &["part", "role", "at", "choice"], "name a room this Channel is made of (role room). part = the room forum's object id (hex); who is IN it stays its own MLS roster"),
        spec(Forum, op::BASE_CLEAR_PART, "base.clearPart", Owner, Sequenced, true,
             &["part"], "detach; the room object, its log and its roster are untouched. Detaching one never attached fails loudly"),
        spec(Forum, op::BASE_SET_ROLE, "base.setRole", Owner, Sequenced, true,
             &["member", "role"], "a standing IN this forum (D-58): a Site's room carries the Arc node's admitter, so a kiosk's visitor joins the room with the Site"),
        spec(Forum, op::BASE_CLEAR_ROLE, "base.clearRole", Owner, Sequenced, true,
             &["member"], "withdraw a standing; the member stays on the roster"),
        spec(Forum, op::BASE_PUBLISH_PROFILE, "base.publishProfile", AnyMember, Commutative, true,
             &["card", "gen"], "the author's own card in this room: a name and optionally a picture; core publishes it from the self record (O-77)"),
        // ---- Field(20) — RESERVED VOCABULARY, NOT A SURFACE ----
        //
        // Kind 20 has no `impl ObjectType` anywhere in pacific-core: no op table, no
        // reducer, no author door on `Node`. Every spec below encodes to canonical
        // bytes and folds NOWHERE — `built: false` is the whole claim, and
        // `catalogue_pin` refuses to let any of them turn true without a reducer.
        spec(Field, op::FIELD_SET_KEYINDEX, "field.setKeyIndex", Owner, Sequenced, false,
             &["key", "index"], "index = JSON FieldIndex {kind:ordinal|named|path|cell|whole,...}"),
        spec(Field, op::FIELD_SET_LITERAL, "field.setLiteral", AnyMember, Commutative, false,
             &["gen", "qty"], "qty = JSON FieldValue {dec} | {int,unit} | {text}"),
        spec(Field, op::FIELD_DEFINE_DEEPLINK, "field.defineDeepLink", Owner, Sequenced, false,
             &["system", "endpoint", "address", "credentialOwner", "credentialEntry", "direction"],
             "address = JSON KeyIndex; direction ∈ {pull,push,bidirectional}; credentialEntry is a vault ref, never the secret"),
        spec(Field, op::FIELD_DEFINE_EXPRESSION, "field.defineExpression", Owner, Sequenced, false,
             &["expr", "selfId", "knownEdges"],
             "expr = JSON Parametric tree; knownEdges = JSON dict for cycle detection"),
        spec(Field, op::FIELD_RECORD_RESOLUTION, "field.recordResolution", AnyMember, Commutative, false,
             &["gen", "at", "status", "qty", "reason"],
             "status ∈ {resolved,failed,unresolved}; qty iff resolved; reason iff failed"),
        spec(Field, op::FIELD_SET_ROLE, "field.setRole", Owner, Sequenced, false,
             &["target", "role"], "role int: 0=none 1=existence 2=viewer 3=editor 4=owner"),
        spec(Field, op::FIELD_SUPPRESS, "field.suppress", Owner, Sequenced, false, &[], "hide, non-destructive"),
        spec(Field, op::FIELD_UNSUPPRESS, "field.unsuppress", Owner, Sequenced, false, &[], "restore"),
        // ---- System(21) ----
        spec(System, op::SYSTEM_DEFINE, "system.define", Owner, Sequenced, true,
             &["name", "connector", "scope"],
             "BUILT (pacific_core::system). connector is the dispatch key — \"music.ra\", \"art.artrabbit\" — and is how system_for_connector finds where to hydrate INTO. scope is free text OWNED BY THE CONNECTOR and opaque to the core"),
        spec(System, op::SYSTEM_HYDRATE, "system.hydrate", AnyMember, Commutative, true,
             &["key", "payload", "fetchedAt", "rev", "withdrawn"],
             "BUILT. One fetched item, per-key LWW by rev. payload is the source's own JSON kept verbatim, ≤16K, REFUSED not truncated. A stale replay is a silent no-op, never a rejection. withdrawn int 0|1 is a TOMBSTONE — the news that a listing was cancelled travels the paths the listing did"),
        // core's system.rs reserves 1..=9 IN WRITING for the connector op-group, so
        // these cannot collide and are kept rather than deleted. None folds.
        spec(System, op::SYSTEM_ADD_FIELD, "system.addField", Owner, Sequenced, false,
             &["field"], "RESERVED, no reducer. field = JSON SystemField {key,hint,locator,writable}"),
        spec(System, op::SYSTEM_REMOVE_FIELD, "system.removeField", Owner, Sequenced, false, &["key"], "RESERVED, no reducer"),
        spec(System, op::SYSTEM_SET_CONNECTOR, "system.setConnector", Owner, Sequenced, false,
             &["connector"], "RESERVED, no reducer. connector = JSON Connector {kind,endpoint,auth}"),
        spec(System, op::SYSTEM_ADD_OPERATION, "system.addOperation", Owner, Sequenced, false,
             &["operation"], "RESERVED, no reducer. operation = JSON Operation {id,mode,touches,requires}"),
        spec(System, op::SYSTEM_REMOVE_OPERATION, "system.removeOperation", Owner, Sequenced, false, &["id"], "RESERVED, no reducer"),
        spec(System, op::SYSTEM_BIND_CREDENTIAL, "system.bindCredential", Owner, Sequenced, false,
             &["handle"], "RESERVED, no reducer. handle = JSON CredentialHandle (opaque; never the secret)"),
        spec(System, op::SYSTEM_REVOKE_CREDENTIAL, "system.revokeCredential", Owner, Sequenced, false, &["id"], "RESERVED, no reducer"),
        spec(System, op::SYSTEM_SUPPRESS_OPERATION, "system.suppressOperation", Owner, Sequenced, false,
             &["id"], "RESERVED, no reducer. A reversible off-switch"),
        spec(System, op::SYSTEM_RESOLVE_VALUE, "system.resolveValue", AnyMember, Commutative, false,
             &["action"], "RESERVED, no reducer. action = JSON SystemAction"),
        // ---- Project(22) — the Work-tab model; every op is BUILT (pacific_core::project) ----
        spec(Project, op::PROJECT_CONFIGURE, "project.configure", Owner, Sequenced, true,
             &["title", "status"], "status ∈ {active,archived,paused}"),
        spec(Project, op::PROJECT_ADD_TIMELINE_ITEM, "project.addTimelineItem", Owner, Sequenced, true,
             &["itemId", "kind", "title"], "kind ∈ {action,event,gate}"),
        spec(Project, op::PROJECT_SUPPRESS_ITEM, "project.suppressItem", Owner, Sequenced, true,
             &["itemId", "suppressed"], "suppressed int 0|1 (non-destructive)"),
        spec(Project, op::PROJECT_SET_ITEM_SCHEDULE, "project.setItemSchedule", Owner, Sequenced, true,
             &["itemId", "at", "endDate"], "at/endDate = epoch-seconds; endDate optional"),
        spec(Project, op::PROJECT_SET_ASSIGNEE, "project.setAssignee", Owner, Sequenced, true,
             &["itemId", "member", "assigned"], "member = hex id (must be a group member); assigned int 0|1"),
        spec(Project, op::PROJECT_ADD_DEPENDENCY, "project.addDependency", Owner, Sequenced, true,
             &["edgeId", "from", "to", "kind"], "kind ∈ {blocks,gates}; cycle-guarded; 'blocked' is DERIVED"),
        spec(Project, op::PROJECT_REMOVE_DEPENDENCY, "project.removeDependency", Owner, Sequenced, true, &["edgeId"], ""),
        spec(Project, op::PROJECT_SUBSCRIBE, "project.subscribe", Owner, Sequenced, true,
             &["subId", "target", "kind", "disclosure", "originItem"],
             "kind ∈ {topic,system,forum,project}; disclosure ∈ {existence,summary,full}; originItem optional (pins to a timeline item). A deliverable is a kind=project edge."),
        spec(Project, op::PROJECT_UNSUBSCRIBE, "project.unsubscribe", Owner, Sequenced, true, &["subId"], ""),
        spec(Project, op::PROJECT_SET_ITEM_PROGRESS, "project.setItemProgress", AnyMember, Commutative, true,
             &["itemId", "status", "gen"], "status ∈ {open,in_progress,done,cancelled}; blocked is DERIVED not stored"),
        spec(Project, op::PROJECT_SET_ITEM_FIELD, "project.setItemField", AnyMember, Commutative, true,
             &["itemId", "field", "value", "gen"], "per-item free-text attribute, LWW by (gen,author)"),
        spec(Project, op::PROJECT_TOUCH_SUBSCRIPTION, "project.touchSubscription", AnyMember, Commutative, true,
             &["subId", "at", "gen"], "the deliverable read cursor (collapsed-view freshness)"),
        // ---- Project objectives (Tab 1): the human-authored spine ----
        spec(Project, op::PROJECT_SET_OBJECTIVE, "project.setObjective", Owner, Sequenced, true,
             &["headline", "goal"], "the project's headline and goal — its semantic heart"),
        spec(Project, op::PROJECT_SET_ROLE, "project.setRole", Owner, Sequenced, true,
             &["member", "role"], "member = hex id (a group member); role ∈ {viewer,contributor,maintainer,owner}"),
        spec(Project, op::PROJECT_ADD_STAKEHOLDER, "project.addStakeholder", Owner, Sequenced, true,
             &["stakeholderId", "name", "note"], "an external (non-member) party — gets a swimlane"),
        spec(Project, op::PROJECT_REMOVE_STAKEHOLDER, "project.removeStakeholder", Owner, Sequenced, true,
             &["stakeholderId"], ""),
        spec(Project, op::PROJECT_ADD_LOCATION, "project.addLocation", Owner, Sequenced, true,
             &["locationId", "name"], "a key location the project touches"),
        spec(Project, op::PROJECT_REMOVE_LOCATION, "project.removeLocation", Owner, Sequenced, true,
             &["locationId"], ""),
        spec(Project, op::PROJECT_SET_KPI, "project.setKpi", Owner, Sequenced, true,
             &["kpiId", "label", "target"], "a KPI the objective is measured against"),
        spec(Project, op::PROJECT_REMOVE_KPI, "project.removeKpi", Owner, Sequenced, true,
             &["kpiId"], ""),
        // The parts facet. A Project's comments section IS a Forum, and a part.
        spec(Project, op::BASE_SET_PART, "base.setPart", Owner, Sequenced, true,
             &["part", "role", "at", "choice"], "name an object this Project is made of, in a role (comments). part = its object id (hex); at = event unix ms"),
        spec(Project, op::BASE_CLEAR_PART, "base.clearPart", Owner, Sequenced, true,
             &["part"], "detach the part; the object, its log and its roster are untouched"),
        // ---- Topic(23) — RESERVED VOCABULARY, NOT A SURFACE (see Field) ----
        spec(Topic, op::TOPIC_CHARTER, "topic.charter", Owner, Sequenced, false, &["brief"], ""),
        spec(Topic, op::TOPIC_SET_CADENCE, "topic.setCadence", Owner, Sequenced, false,
             &["cadence"], "cadence = JSON {manual} | {every:{seconds}} | {adaptive:{floor,ceiling}}"),
        spec(Topic, op::TOPIC_CONNECT, "topic.connect", Owner, Sequenced, false,
             &["connection"], "connection = JSON Connection"),
        spec(Topic, op::TOPIC_DISCONNECT, "topic.disconnect", Owner, Sequenced, false, &["connectionId"], ""),
        spec(Topic, op::TOPIC_ADD_FINDING, "topic.addFinding", AnyMember, Commutative, false,
             &["finding"], "finding = JSON Finding (source REQUIRED — no uncited claim)"),
        spec(Topic, op::TOPIC_RETRACT_FINDING, "topic.retractFinding", AnyMember, Commutative, false,
             &["findingId"], ""),
        spec(Topic, op::TOPIC_POSE_QUESTION, "topic.poseQuestion", AnyMember, Commutative, false,
             &["question"], "unknown stays unknown — recorded, not invented"),
        spec(Topic, op::TOPIC_RESOLVE_QUESTION, "topic.resolveQuestion", AnyMember, Commutative, false,
             &["question"], ""),
        // ---- base membership op-group (BUILT — pacific_core::membership) ----
        //
        // From here down, every op is spliced into GROUP_OPS and folds on ANY
        // Group-typed object — a group, a notebook, a note, one of the tethers. The
        // 0xF00n_ bands exist so they can never collide with a kind's own low ids.
        spec(Group, op::MEMBER_JOINED, "base.memberJoined", AnyMember, Commutative, true,
             &["member", "at"], "owner-authored (they committed the MLS Add); at = unix ms"),
        spec(Group, op::MEMBER_LEFT, "base.memberLeft", AnyMember, Commutative, true,
             &["member", "at", "reason"],
             "author it about YOURSELF to leave — no owner needed. left vs removed is DERIVED from the author, never sent. An owner must hand over first"),
        spec(Group, op::OWNER_HANDOVER, "base.ownerHandover", Owner, Sequenced, true,
             &["member", "at"], "REQUIRED before an owner may leave — an ownerless object can never accept another sequenced delta"),
        spec(Group, op::CLAIM_SPENT, "base.claimSpent", OwnerOrAdmitter, Commutative, true,
             &["claim", "member", "at", "choice", "share"], "written by the MLS door beside the Add a kiosk claim bought (A-3): claim = sha256 of its nonce, never the token; the first per claim holds, and only the owner or an admitter may write one"),
        // ---- base publication op-group (BUILT — pacific_core::publication) ----
        spec(Group, op::BASE_PUBLISH, "base.publish", Owner, Sequenced, true,
             &["slug", "publisher"],
             "give the object a PUBLIC ADDRESS and name the member that serves it; the publisher must already be in the roster. A ONE-WAY DOOR: once served over HTTP a projection is cached outside the protocol and unpublish cannot retract it"),
        spec(Group, op::BASE_UNPUBLISH, "base.unpublish", Owner, Sequenced, true,
             &[], "stop serving. Does not un-cache what was already read"),
        // THE WALLET AND NOTE OP-GROUPS WERE HERE, on Group. They moved to
        // `treasury`(31) and `note`(32) on 25 Sep 2026 — a Treasury's roster is
        // narrower than the body's on purpose, and a Note's is whoever may read it.
        //
        // Their ops are NOT re-listed under the new kinds, because neither kind is
        // in `Kind`: adding one is a UniFFI surface change and neither has an iOS
        // caller yet. `every_core_kind_is_surfaced_or_declined` records that
        // decision, and this catalogue surfaces what Swift can reach.
    ]
}

// ============================================================================
// Typed constructors — one per op, producing a DeltaDraft. These are the
// idiomatic Swift bindings (e.g. `groupSetProfile(displayName:shape:)`).
// gen (the commutative LWW key) is a caller arg where the op is commutative,
// because it must be monotone per author; encoders/reducers do not invent it.
// ============================================================================

// ---- Group ----
#[uniffi::export]
pub fn group_set_profile(display_name: String, shape: String, card: Option<String>) -> DeltaDraft {
    // `card` is the ContactCard as JSON (org/title/emails/phones/urls/photo/note/tags).
    draft(
        Kind::Group,
        op::GROUP_SET_PROFILE,
        vec![
            Some(text("displayName", display_name)),
            Some(text("shape", shape)),
            opt_text("card", card),
        ],
    )
}
#[uniffi::export]
pub fn group_set_presence(
    kind: String,
    identity_key: Option<String>,
    invite_hint: Option<String>,
) -> DeltaDraft {
    draft(
        Kind::Group,
        op::GROUP_SET_PRESENCE,
        vec![
            Some(text("kind", kind)),
            opt_text("identityKey", identity_key),
            opt_text("inviteHint", invite_hint),
        ],
    )
}
#[uniffi::export]
pub fn group_store_credential(id: String, kind: String, label: String) -> DeltaDraft {
    draft(
        Kind::Group,
        op::GROUP_STORE_CREDENTIAL,
        vec![
            Some(text("id", id)),
            Some(text("kind", kind)),
            Some(text("label", label)),
        ],
    )
}
#[uniffi::export]
pub fn group_revoke_credential(id: String) -> DeltaDraft {
    draft(
        Kind::Group,
        op::GROUP_REVOKE_CREDENTIAL,
        vec![Some(text("id", id))],
    )
}
/// GIVE A GROUP A PUBLIC ADDRESS — the op behind "an account owns a site".
///
/// `publisher` is the member that ANSWERS for the object, hex-encoded, and it must
/// already be on the roster: `reduce_publication` refuses otherwise, because an
/// address recorded against someone who cannot fold the object is an address that
/// can never answer. Both args are validated before either is assigned, so a
/// rejected publish leaves no half-published object behind.
///
/// A ONE-WAY DOOR, and the catalogue says so in the same words: once a projection
/// has been served over HTTP it is cached outside the protocol, and `group_unpublish`
/// stops future reads without retracting past ones.
///
/// ADDED 15 Sep 2026. The op has been in the ICD and in this catalogue as
/// `built: true` since the publication facet landed — both of which are accurate
/// about the REDUCER and said nothing about a caller, and there was none on either
/// platform. `base.publish` could be folded and could not be authored.
#[uniffi::export]
pub fn group_publish(slug: String, publisher: String) -> DeltaDraft {
    draft(
        Kind::Group,
        op::BASE_PUBLISH,
        vec![
            Some(text("slug", slug)),
            Some(text("publisher", publisher)),
        ],
    )
}
/// Stop serving the public address. Takes no args — the reducer clears BOTH the
/// slug and the publisher, so a later publisher cannot silently inherit a name the
/// owner did not re-choose.
#[uniffi::export]
pub fn group_unpublish() -> DeltaDraft {
    draft(Kind::Group, op::BASE_UNPUBLISH, vec![])
}
/// Edit the shared face (owner or admin): a JSON document, as group.setFace takes it.
#[uniffi::export]
pub fn group_edit_face(face: String, gen: i64) -> DeltaDraft {
    draft(Kind::Group, op::GROUP_EDIT_FACE, vec![Some(text("face", face)), Some(int("gen", gen))])
}
/// `space` is the member's key (hex), carried as `member` (NC-38); the name stays, as
/// the Swift signature's.
#[uniffi::export]
pub fn group_set_member_role(space: String, role: String) -> DeltaDraft {
    draft(
        Kind::Group,
        op::BASE_SET_ROLE,
        vec![Some(text("member", space)), Some(text("role", role))],
    )
}
#[uniffi::export]
pub fn group_set_affiliation(
    peer: String,
    rel: String,
    name: String,
    tether: Option<String>,
    at: i64,
) -> DeltaDraft {
    draft(
        Kind::Group,
        op::GROUP_SET_AFFILIATION,
        vec![
            Some(text("peer", peer)),
            Some(text("rel", rel)),
            Some(text("name", name)),
            opt_text("tether", tether),
            Some(int("at", at)),
        ],
    )
}
#[uniffi::export]
pub fn group_clear_affiliation(peer: String) -> DeltaDraft {
    draft(
        Kind::Group,
        op::GROUP_CLEAR_AFFILIATION,
        vec![Some(text("peer", peer))],
    )
}
#[uniffi::export]
pub fn group_set_part(part: String, role: String, at: i64) -> DeltaDraft {
    draft(
        Kind::Group,
        op::BASE_SET_PART,
        vec![Some(text("part", part)), Some(text("role", role)), Some(int("at", at))],
    )
}
#[uniffi::export]
pub fn group_clear_part(part: String) -> DeltaDraft {
    draft(Kind::Group, op::BASE_CLEAR_PART, vec![Some(text("part", part))])
}

// ---- Forum ----
#[uniffi::export]
pub fn forum_post(text_body: String) -> DeltaDraft {
    // gen is core-assigned at author time (author_delta_count); not a caller arg.
    draft(
        Kind::Forum,
        op::FORUM_POST,
        vec![Some(text("text", text_body))],
    )
}
/// React to a message. `target_*` is the `(author, gen)` MsgRef of the message
/// reacted to; `active: false` (or an empty emoji) takes the reaction back.
#[uniffi::export]
pub fn forum_react(
    target_author: String,
    target_gen: i64,
    emoji: String,
    active: bool,
    gen: i64,
) -> DeltaDraft {
    draft(
        Kind::Forum,
        op::FORUM_REACT,
        vec![
            Some(text("target_author", target_author)),
            Some(int("target_gen", target_gen)),
            Some(text("emoji", emoji)),
            Some(int("active", i64::from(active))),
            Some(int("gen", gen)),
        ],
    )
}

/// Acknowledge a BATCH of messages at one status (1 = delivered, 2 = read).
/// `refs` is `"<authorHex>:<gen>"` joined by commas — the wire form the reducer
/// parses, and one delta for a screenful rather than one per message.
#[uniffi::export]
pub fn forum_receipt(status: i64, refs: String, gen: i64) -> DeltaDraft {
    draft(
        Kind::Forum,
        op::FORUM_RECEIPT,
        vec![
            Some(int("status", status)),
            Some(text("refs", refs)),
            Some(int("gen", gen)),
        ],
    )
}

/// Vote a message up or down. `dir` is -1, 0 or +1; 0 clears, and anything else
/// is malformed and drops the whole delta at fold.
#[uniffi::export]
pub fn forum_vote(target_author: String, target_gen: i64, dir: i64, gen: i64) -> DeltaDraft {
    draft(
        Kind::Forum,
        op::FORUM_VOTE,
        vec![
            Some(text("target_author", target_author)),
            Some(int("target_gen", target_gen)),
            Some(int("dir", dir)),
            Some(int("gen", gen)),
        ],
    )
}

/// Withdraw a message (its author) or hide it (the owner): `target_*` is its MsgRef.
#[uniffi::export]
pub fn forum_retract(target_author: String, target_gen: i64, gen: i64) -> DeltaDraft {
    draft(
        Kind::Forum,
        op::FORUM_RETRACT,
        vec![
            Some(text("target_author", target_author)),
            Some(int("target_gen", target_gen)),
            Some(int("gen", gen)),
        ],
    )
}

/// Name a room this Channel is made of — the parts facet, as a Group's. Who is IN
/// the room stays the room's own MLS roster.
#[uniffi::export]
pub fn forum_set_part(part: String, role: String, at: i64) -> DeltaDraft {
    draft(
        Kind::Forum,
        op::BASE_SET_PART,
        vec![Some(text("part", part)), Some(text("role", role)), Some(int("at", at))],
    )
}

/// Detach a room. The room object, its log and its roster are untouched.
#[uniffi::export]
pub fn forum_clear_part(part: String) -> DeltaDraft {
    draft(Kind::Forum, op::BASE_CLEAR_PART, vec![Some(text("part", part))])
}

// ---- Field ----
#[uniffi::export]
pub fn field_set_key_index(key: String, index_json: String) -> DeltaDraft {
    draft(
        Kind::Field,
        op::FIELD_SET_KEYINDEX,
        vec![Some(text("key", key)), Some(text("index", index_json))],
    )
}
#[uniffi::export]
pub fn field_set_literal(gen: i64, qty_json: String) -> DeltaDraft {
    draft(
        Kind::Field,
        op::FIELD_SET_LITERAL,
        vec![Some(int("gen", gen)), Some(text("qty", qty_json))],
    )
}
#[uniffi::export]
pub fn field_define_deep_link(
    system: String,
    endpoint: String,
    address_json: String,
    credential_owner: String,
    credential_entry: String,
    direction: String,
) -> DeltaDraft {
    draft(
        Kind::Field,
        op::FIELD_DEFINE_DEEPLINK,
        vec![
            Some(text("system", system)),
            Some(text("endpoint", endpoint)),
            Some(text("address", address_json)),
            Some(text("credentialOwner", credential_owner)),
            Some(text("credentialEntry", credential_entry)),
            Some(text("direction", direction)),
        ],
    )
}
#[uniffi::export]
pub fn field_define_expression(
    expr_json: String,
    self_id: String,
    known_edges_json: String,
) -> DeltaDraft {
    draft(
        Kind::Field,
        op::FIELD_DEFINE_EXPRESSION,
        vec![
            Some(text("expr", expr_json)),
            Some(text("selfId", self_id)),
            Some(text("knownEdges", known_edges_json)),
        ],
    )
}
#[uniffi::export]
pub fn field_record_resolution(
    gen: i64,
    at: i64,
    status: String,
    qty_json: Option<String>,
    reason: Option<String>,
) -> DeltaDraft {
    draft(
        Kind::Field,
        op::FIELD_RECORD_RESOLUTION,
        vec![
            Some(int("gen", gen)),
            Some(int("at", at)),
            Some(text("status", status)),
            opt_text("qty", qty_json),
            opt_text("reason", reason),
        ],
    )
}
#[uniffi::export]
pub fn field_set_role(target: String, role: i64) -> DeltaDraft {
    draft(
        Kind::Field,
        op::FIELD_SET_ROLE,
        vec![Some(text("target", target)), Some(int("role", role))],
    )
}
#[uniffi::export]
pub fn field_suppress() -> DeltaDraft {
    draft(Kind::Field, op::FIELD_SUPPRESS, vec![])
}
#[uniffi::export]
pub fn field_unsuppress() -> DeltaDraft {
    draft(Kind::Field, op::FIELD_UNSUPPRESS, vec![])
}

// ---- System ----
//
// Two constructors, because two ops fold. The reserved connector band (1..=9) is
// enumerable through `delta_catalog()` — where every one of them says `built:
// false` — and deliberately has no idiomatic binding: a Swift call site that
// looks like the others would imply a write that lands nowhere.

/// What this System is. `connector` is the dispatch key ("music.ra") and is how
/// `system_for_connector` finds where to hydrate into; `scope` is free text the
/// connector owns and the core never parses.
#[uniffi::export]
pub fn system_define(name: String, connector: String, scope: Option<String>) -> DeltaDraft {
    draft(
        Kind::System,
        op::SYSTEM_DEFINE,
        vec![
            Some(text("name", name)),
            Some(text("connector", connector)),
            opt_text("scope", scope),
        ],
    )
}

/// One fetched item, as its source produced it. `key` is the source's stable id
/// (the dedup key), `rev` the source's monotone revision (the LWW key), and
/// `withdrawn` a tombstone rather than a deletion — the news that a listing was
/// cancelled has to travel the paths the listing did.
#[uniffi::export]
pub fn system_hydrate(
    key: String,
    payload: String,
    fetched_at: i64,
    rev: i64,
    withdrawn: bool,
) -> DeltaDraft {
    draft(
        Kind::System,
        op::SYSTEM_HYDRATE,
        vec![
            Some(text("key", key)),
            Some(text("payload", payload)),
            Some(int("fetchedAt", fetched_at)),
            Some(int("rev", rev)),
            Some(int("withdrawn", i64::from(withdrawn))),
        ],
    )
}

// ---- Project (the Work-tab model; every op is folded by pacific_core::project) ----
#[uniffi::export]
pub fn project_configure(title: String, status: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_CONFIGURE,
        vec![Some(text("title", title)), Some(text("status", status))],
    )
}
#[uniffi::export]
pub fn project_add_timeline_item(item_id: String, kind: String, title: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_ADD_TIMELINE_ITEM,
        vec![
            Some(text("itemId", item_id)),
            Some(text("kind", kind)),
            Some(text("title", title)),
        ],
    )
}
#[uniffi::export]
pub fn project_suppress_item(item_id: String, suppressed: bool) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_SUPPRESS_ITEM,
        vec![
            Some(text("itemId", item_id)),
            Some(int("suppressed", suppressed as i64)),
        ],
    )
}
#[uniffi::export]
pub fn project_set_item_schedule(item_id: String, at: i64, end_date: Option<i64>) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_SET_ITEM_SCHEDULE,
        vec![
            Some(text("itemId", item_id)),
            Some(int("at", at)),
            end_date.map(|e| int("endDate", e)),
        ],
    )
}
#[uniffi::export]
pub fn project_set_assignee(item_id: String, member: String, assigned: bool) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_SET_ASSIGNEE,
        vec![
            Some(text("itemId", item_id)),
            Some(text("member", member)),
            Some(int("assigned", assigned as i64)),
        ],
    )
}
#[uniffi::export]
pub fn project_add_dependency(
    edge_id: String,
    from: String,
    to: String,
    kind: String,
) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_ADD_DEPENDENCY,
        vec![
            Some(text("edgeId", edge_id)),
            Some(text("from", from)),
            Some(text("to", to)),
            Some(text("kind", kind)),
        ],
    )
}
#[uniffi::export]
pub fn project_remove_dependency(edge_id: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_REMOVE_DEPENDENCY,
        vec![Some(text("edgeId", edge_id))],
    )
}
#[uniffi::export]
pub fn project_subscribe(
    sub_id: String,
    target: String,
    kind: String,
    disclosure: String,
    origin_item: Option<String>,
) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_SUBSCRIBE,
        vec![
            Some(text("subId", sub_id)),
            Some(text("target", target)),
            Some(text("kind", kind)),
            Some(text("disclosure", disclosure)),
            opt_text("originItem", origin_item),
        ],
    )
}
#[uniffi::export]
pub fn project_unsubscribe(sub_id: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_UNSUBSCRIBE,
        vec![Some(text("subId", sub_id))],
    )
}
#[uniffi::export]
pub fn project_set_item_progress(item_id: String, status: String, gen: i64) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_SET_ITEM_PROGRESS,
        vec![
            Some(text("itemId", item_id)),
            Some(text("status", status)),
            Some(int("gen", gen)),
        ],
    )
}
#[uniffi::export]
pub fn project_set_item_field(
    item_id: String,
    field: String,
    value: String,
    gen: i64,
) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_SET_ITEM_FIELD,
        vec![
            Some(text("itemId", item_id)),
            Some(text("field", field)),
            Some(text("value", value)),
            Some(int("gen", gen)),
        ],
    )
}
#[uniffi::export]
pub fn project_touch_subscription(sub_id: String, at: i64, gen: i64) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_TOUCH_SUBSCRIPTION,
        vec![
            Some(text("subId", sub_id)),
            Some(int("at", at)),
            Some(int("gen", gen)),
        ],
    )
}
// ---- Project objectives (Tab 1) ----
#[uniffi::export]
pub fn project_set_objective(headline: String, goal: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_SET_OBJECTIVE,
        vec![Some(text("headline", headline)), Some(text("goal", goal))],
    )
}
#[uniffi::export]
pub fn project_set_role(member: String, role: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_SET_ROLE,
        vec![Some(text("member", member)), Some(text("role", role))],
    )
}
#[uniffi::export]
pub fn project_add_stakeholder(stakeholder_id: String, name: String, note: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_ADD_STAKEHOLDER,
        vec![
            Some(text("stakeholderId", stakeholder_id)),
            Some(text("name", name)),
            Some(text("note", note)),
        ],
    )
}
#[uniffi::export]
pub fn project_remove_stakeholder(stakeholder_id: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_REMOVE_STAKEHOLDER,
        vec![Some(text("stakeholderId", stakeholder_id))],
    )
}
#[uniffi::export]
pub fn project_add_location(location_id: String, name: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_ADD_LOCATION,
        vec![
            Some(text("locationId", location_id)),
            Some(text("name", name)),
        ],
    )
}
#[uniffi::export]
pub fn project_remove_location(location_id: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_REMOVE_LOCATION,
        vec![Some(text("locationId", location_id))],
    )
}
#[uniffi::export]
pub fn project_set_kpi(kpi_id: String, label: String, target: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_SET_KPI,
        vec![
            Some(text("kpiId", kpi_id)),
            Some(text("label", label)),
            Some(text("target", target)),
        ],
    )
}
#[uniffi::export]
pub fn project_remove_kpi(kpi_id: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_REMOVE_KPI,
        vec![Some(text("kpiId", kpi_id))],
    )
}

// ---- Topic ----
#[uniffi::export]
pub fn topic_charter(brief: String) -> DeltaDraft {
    draft(
        Kind::Topic,
        op::TOPIC_CHARTER,
        vec![Some(text("brief", brief))],
    )
}
#[uniffi::export]
pub fn topic_set_cadence(cadence_json: String) -> DeltaDraft {
    draft(
        Kind::Topic,
        op::TOPIC_SET_CADENCE,
        vec![Some(text("cadence", cadence_json))],
    )
}
#[uniffi::export]
pub fn topic_connect(connection_json: String) -> DeltaDraft {
    draft(
        Kind::Topic,
        op::TOPIC_CONNECT,
        vec![Some(text("connection", connection_json))],
    )
}
#[uniffi::export]
pub fn topic_disconnect(connection_id: String) -> DeltaDraft {
    draft(
        Kind::Topic,
        op::TOPIC_DISCONNECT,
        vec![Some(text("connectionId", connection_id))],
    )
}
#[uniffi::export]
pub fn topic_add_finding(finding_json: String) -> DeltaDraft {
    draft(
        Kind::Topic,
        op::TOPIC_ADD_FINDING,
        vec![Some(text("finding", finding_json))],
    )
}
#[uniffi::export]
pub fn topic_retract_finding(finding_id: String) -> DeltaDraft {
    draft(
        Kind::Topic,
        op::TOPIC_RETRACT_FINDING,
        vec![Some(text("findingId", finding_id))],
    )
}
#[uniffi::export]
pub fn topic_pose_question(question: String) -> DeltaDraft {
    draft(
        Kind::Topic,
        op::TOPIC_POSE_QUESTION,
        vec![Some(text("question", question))],
    )
}
#[uniffi::export]
pub fn topic_resolve_question(question: String) -> DeltaDraft {
    draft(
        Kind::Topic,
        op::TOPIC_RESOLVE_QUESTION,
        vec![Some(text("question", question))],
    )
}

// ---- Base governance: NOT HERE, AND THAT IS THE ANSWER ----------------------
//
// `governance.assignOwner/setRole/addMember/removeMember/repinSuccession` stood
// here on ids 100..104 until 15 Sep 2026. No kind declares them, nothing folds
// them, and `governance.setRole` shadowed `group.setMemberRole`(4) — which IS
// built — so the tidier-looking call was the one that wrote a delta into a
// signed log to do nothing forever.
//
// What is real, and what the app should call instead:
//   owner        -> `base.ownerHandover` (owner/sequenced, reduced by
//                   pacific_core::membership)
//   role         -> `group.setMemberRole` (owner/sequenced, reduced by
//                   pacific_core::group)
//   add / remove -> the MLS commit itself, recorded by `base.memberJoined` /
//                   `base.memberLeft`. The roster IS the access control; a delta
//                   that "adds a member" without an MLS Add adds nobody.
//   succession   -> nothing. There is no reducer and no state for it.

// ---- base membership op-group ------------------------------------------------
// NO DRAFT BUILDERS, on purpose (membership-through-mls.md §10.3). A membership
// record is written by the core door that performs the MLS operation, alongside it
// — `Core::group_leave`, `Core::group_remove_member`, `Core::group_hand_over`, and
// the Add door — and `group_author` refuses one written any other way. The three
// builders that stood here let the app write `base.memberLeft` with no Remove behind
// it, which is exactly how "Leave" came to revoke nothing. The op constants above
// stay: they are the catalogue, pinned against core by `catalogue_pin`.

// ============================================================================
// Encode — lower any DeltaDraft to canonical bytes + DeltaId. Total for every
// op (encoding is real even where the reducer is not built). Authoring/folding
// stays in `Core.object_post` for forum.post only.
// ============================================================================

/// Build the canonical envelope for a draft. `gen` is read from the draft's own
/// `gen` arg when present (commutative ops carry it), else the delta is sequenced.
#[uniffi::export]
pub fn encode_delta(draft: DeltaDraft) -> Result<EncodedDelta, FfiError> {
    let mut args: Args = BTreeMap::new();
    let mut gen: Option<u64> = None;
    for entry in &draft.args {
        if entry.key == "gen" {
            if let ArgValue::Int { value } = entry.value {
                if value < 0 {
                    return Err(FfiError::Core("gen must be non-negative".into()));
                }
                gen = Some(value as u64);
            }
        }
        args.insert(entry.key.clone(), entry.value.core());
    }

    // epoch is stamped by the group at author time; for a standalone encode we
    // use 0 (the DeltaId is recomputed with the real epoch when actually authored).
    let delta = build_delta(draft.kind.core(), draft.op_id, args, 0, gen);
    Ok(EncodedDelta {
        delta_id: hex::encode(delta.id()),
        type_id: delta.type_id,
        op_id: delta.op_id,
        cbor_base64: BASE64_STANDARD.encode(delta.canonical_bytes()),
    })
}

// ============================================================================
// THE PIN — the two tables, compared in both directions.
//
// `pacific-core/src/icd.rs` does exactly this between the reducers and the
// AsyncAPI document, and it is the reason that document has not rotted. This is
// the same mechanism one layer out, and it is cheaper: no JSON, no parsing, just
// two Rust tables that must agree.
// ============================================================================

#[cfg(test)]
mod catalogue_pin {
    use super::*;
    use pacific_core::object::{
        Authority as CoreAuthority, Commutativity, ObjectKind, ObjectType, OpDecl,
    };
    use std::collections::BTreeSet;

    /// The core op table behind one FFI `Kind`, or the reason there is none.
    ///
    /// A `match`, so a new `Kind` variant does not compile until somebody has
    /// said which reducer stands behind it.
    fn core_ops(kind: Kind) -> Result<&'static [OpDecl], &'static str> {
        match kind {
            Kind::Group => Ok(pacific_core::group::GroupType::ops()),
            Kind::Forum => Ok(pacific_core::coordinator::ForumType::ops()),
            Kind::System => Ok(pacific_core::system::SystemType::ops()),
            Kind::Project => Ok(pacific_core::project::ProjectType::ops()),
            Kind::Field => Err("Field (20) has no `impl ObjectType` in pacific-core"),
            Kind::Topic => Err("Topic (23) has no `impl ObjectType` in pacific-core"),
        }
    }

    /// Every FFI `Kind`, so the sweeps below cannot miss one.
    const ALL_KINDS: &[Kind] = &[
        Kind::Group,
        Kind::Forum,
        Kind::Field,
        Kind::System,
        Kind::Project,
        Kind::Topic,
    ];

    fn authority_matches(ffi: Authority, core: CoreAuthority) -> bool {
        matches!(
            (ffi, core),
            (Authority::Owner, CoreAuthority::Owner)
                | (Authority::AnyMember, CoreAuthority::AnyMember)
                | (Authority::OwnerOrAdmin, CoreAuthority::OwnerOrRole(pacific_core::group::GroupRole::Admin))
                | (Authority::OwnerOrAdmitter, CoreAuthority::OwnerOrRole(pacific_core::group::GroupRole::Admitter))
        )
    }

    fn fold_matches(ffi: Fold, core: Commutativity) -> bool {
        matches!(
            (ffi, core),
            (Fold::Sequenced, Commutativity::Sequenced)
                | (Fold::Commutative, Commutativity::Commutative)
        )
    }

    /// `ALL_KINDS` really is all of them — pinned against the `Kind::core()` map,
    /// which is itself an exhaustive match, so a seventh variant fails there.
    #[test]
    fn all_kinds_lists_every_variant() {
        let seen: BTreeSet<u32> = ALL_KINDS.iter().map(|k| k.type_id()).collect();
        assert_eq!(seen.len(), ALL_KINDS.len(), "a kind is listed twice");
        assert_eq!(
            seen,
            BTreeSet::from([18, 19, 20, 21, 22, 23]),
            "ALL_KINDS drifted from the Kind enum"
        );
    }

    /// code -> core. Every `built: true` spec is a real op on its kind's reducer,
    /// with the SAME id, name, authority and fold.
    ///
    /// The id is the one that mattered: `forum.retractPost` claimed op 1 while the
    /// reducer's op 1 was `forum.react`, so a retraction folded as a reaction on
    /// every device that received it.
    #[test]
    fn every_built_spec_is_a_real_op_on_its_reducer() {
        for s in delta_catalog() {
            if !s.built {
                continue;
            }
            let ops = core_ops(s.kind).unwrap_or_else(|why| {
                panic!(
                    "`{}` is marked built on {:?}, but {why}. A spec is built only when a \
                     reducer folds it.",
                    s.name, s.kind
                )
            });
            let decl = ops.iter().find(|d| d.op_id == s.op_id).unwrap_or_else(|| {
                panic!(
                    "`{}` claims op {} on {:?}, and that kind's reducer declares no such op. \
                     Either it is not built, or the id moved.",
                    s.name, s.op_id, s.kind
                )
            });
            assert_eq!(
                decl.name, s.name,
                "OP-ID COLLISION: {:?} op {} is `{}` in pacific-core and `{}` here. An op id \
                 is a wire value — a delta authored through this table would fold as the \
                 other op on every device that received it.",
                s.kind, s.op_id, decl.name, s.name
            );
            assert!(
                authority_matches(s.authority, decl.authority),
                "`{}` authority disagrees with its reducer ({:?} here, {:?} in core)",
                s.name,
                s.authority,
                decl.authority
            );
            assert!(
                fold_matches(s.fold, decl.commutativity),
                "`{}` fold disagrees with its reducer ({:?} here, {:?} in core). The arm is \
                 chosen by the OP'S DECLARATION, so a disagreement here is a delta that \
                 lands in the wrong half of the coordinator.",
                s.name,
                s.fold,
                decl.commutativity
            );
            assert_eq!(
                s.type_id,
                s.kind.core().type_id() as u32,
                "`{}` carries the wrong type id",
                s.name
            );
        }
    }

    /// core -> code. Every op a reducer declares appears here, as `built: true`.
    ///
    /// This is the direction that was missing entirely. `group.setOffice`,
    /// `forum.setRoom`, `system.hydrate`, the whole wallet and note facets —
    /// twenty ops with live reducers that Swift could not name, because nothing
    /// failed when core grew and this table did not.
    #[test]
    fn every_core_op_appears_in_the_catalogue() {
        let catalogue = delta_catalog();
        let mut missing: Vec<String> = Vec::new();

        for &kind in ALL_KINDS {
            let Ok(ops) = core_ops(kind) else { continue };
            for decl in ops {
                let found = catalogue
                    .iter()
                    .find(|s| s.kind == kind && s.op_id == decl.op_id);
                match found {
                    None => missing.push(format!(
                        "{:?} op {} `{}` — declared by the reducer, absent from delta_catalog()",
                        kind, decl.op_id, decl.name
                    )),
                    Some(s) if !s.built => missing.push(format!(
                        "{:?} op {} `{}` — in the catalogue but marked built:false, and its \
                         reducer folds it",
                        kind, decl.op_id, decl.name
                    )),
                    Some(_) => {}
                }
            }
        }

        assert!(
            missing.is_empty(),
            "the FFI catalogue has fallen behind the reducers:\n  {}",
            missing.join("\n  ")
        );
    }

    /// An unbuilt spec may not squat on an id, or a name, that core is using.
    ///
    /// Deleting the five colliding Forum ops fixed today's bug; this is what stops
    /// tomorrow's. A reserved band is only reserved while it stays out of the way,
    /// and the two that remain — Field/Topic (no reducer at all) and System 1..=9
    /// (reserved in writing by `pacific_core::system`) — are held to that here.
    #[test]
    fn no_unbuilt_spec_squats_on_a_real_op() {
        for s in delta_catalog() {
            if s.built {
                continue;
            }
            let Ok(ops) = core_ops(s.kind) else { continue };
            if let Some(d) = ops.iter().find(|d| d.op_id == s.op_id) {
                panic!(
                    "`{}` is unbuilt but holds {:?} op {}, which pacific-core folds as `{}`. \
                     Move it to a band core does not use, or delete it — encoding it authors \
                     `{}`.",
                    s.name, s.kind, s.op_id, d.name, d.name
                );
            }
            if let Some(d) = ops.iter().find(|d| d.name == s.name) {
                panic!(
                    "`{}` is unbuilt here but pacific-core declares it at op {} — one name, \
                     two ids, and the wire only carries the id.",
                    s.name, d.op_id
                );
            }
        }
    }

    /// One op id appears once per kind, and one name once in the whole table.
    #[test]
    fn the_catalogue_has_no_internal_duplicates() {
        let mut ids: BTreeSet<(u32, u32)> = BTreeSet::new();
        let mut names: BTreeSet<(u32, String)> = BTreeSet::new();
        for s in delta_catalog() {
            assert!(
                ids.insert((s.type_id, s.op_id)),
                "type {} op {} is declared twice (`{}`)",
                s.type_id,
                s.op_id,
                s.name
            );
            // Keyed by (kind, name), NOT by name alone. A FACET op appears on every
            // kind that carries it — `base.setPart` is on seven — so a global name
            // check forbids cataloguing facets at all, which is probably why
            // Project's parts ops were once never added here. What must not repeat is
            // one name twice on ONE kind, which is what this now says.
            assert!(
                names.insert((s.type_id, s.name.clone())),
                "`{}` is declared twice on type {}",
                s.name,
                s.type_id
            );
        }
    }

    /// The kinds core has, against the kinds this facade surfaces.
    ///
    /// An exhaustive match on `ObjectKind`, so a THIRTEENTH core kind does not
    /// compile until somebody has said here whether Swift gets it. The six that
    /// are absent are absent on the record: their ops reach the app through typed
    /// `Node` calls (`thing_state`, `posts`, `event_author`, …) rather than through
    /// the generic draft path, and adding them to `Kind` is a UniFFI surface change
    /// that has to be taken deliberately.
    #[test]
    fn every_core_kind_is_surfaced_or_declined() {
        let surfaced: BTreeSet<u16> = ALL_KINDS.iter().map(|k| k.core().type_id()).collect();
        for &kind in ObjectKind::ALL {
            let want = match kind {
                ObjectKind::Group
                | ObjectKind::Forum
                | ObjectKind::Field
                | ObjectKind::System
                | ObjectKind::Project
                | ObjectKind::Topic => true,
                // Not in `Kind`, and not by accident: each has its own typed door
                // on `Node`, so nothing here is the only way to reach it.
                ObjectKind::Contact
                | ObjectKind::Conversation
                | ObjectKind::Thing
                | ObjectKind::Place
                | ObjectKind::Event
                | ObjectKind::Post
                // Promoted out of `group`'s facets into kinds of their own on
                // 25 Sep 2026. Declined here DELIBERATELY: adding either to `Kind`
                // is a UniFFI surface change, and neither has an iOS caller yet.
                // The decision is on the record rather than implied by absence.
                | ObjectKind::Treasury
                | ObjectKind::Note
                // 25 Sep 2026, out of System. The Arc's object: read through
                // `host_sites`/`host_media`, no iOS caller. Same UniFFI rule.
                | ObjectKind::Host
                // 30 Sep 2026, W-98 Trade: one sale, opened by the settler. No iOS
                // caller yet; the same UniFFI rule.
                | ObjectKind::Transaction => false,
            };
            assert_eq!(
                surfaced.contains(&kind.type_id()),
                want,
                "`{}` (type {}) — the Kind enum and this decision disagree",
                kind.name(),
                kind.type_id()
            );
        }
    }

    /// THE REGRESSION, named. Kept as its own test because the five ids below are
    /// the whole reason this module exists, and a failure here should not need the
    /// sweep above to be legible.
    #[test]
    fn the_forum_band_is_the_reducers_band() {
        let want = [
            (0u32, "forum.post"),
            (1, "forum.react"),
            (2, "forum.receipt"),
            (3, "forum.vote"),
            (6, "forum.retract"),
            (7, "forum.editDescription"),
        ];
        // The kind's OWN band. Facet ops live in reserved high bands and are
        // excluded deliberately — pinning them together made this re-break every
        // time a facet reached Forum (parent, 25 Sep), and what the message below
        // is about is the ids 0..3 NOT MOVING. A facet at 0xF008 cannot move them.
        // 4 and 5 were forum.setRoom/clearRoom until 26 Sep 2026: a room is a part
        // now (base.setPart, pinned below with the other facets). Never reassigned:
        // forum.retract is 6.
        let got: Vec<(u32, String)> = delta_catalog()
            .into_iter()
            .filter(|s| s.kind == Kind::Forum && s.op_id < 0x1000)
            .map(|s| (s.op_id, s.name))
            .collect();
        assert_eq!(
            got,
            want.iter()
                .map(|(i, n)| (*i, n.to_string()))
                .collect::<Vec<_>>(),
            "the Forum band moved. It is the band the reducer folds — `forum.retractPost` \
             on op 1 is how an iOS retraction became a reaction."
        );
        // And the facets it carries, named rather than enumerated by position.
        for (op, why) in [
            (op::BASE_SET_PARENT, "a Forum names the object it is a part of"),
            (op::BASE_CLEAR_PARENT, "and can detach"),
            (op::BASE_SET_PART, "a Forum is made of its rooms"),
            (op::BASE_CLEAR_PART, "and can detach one"),
        ] {
            assert!(
                delta_catalog().iter().any(|s| s.kind == Kind::Forum && s.op_id == op),
                "{why}"
            );
        }
    }

    /// Encoding still lowers to the envelope the core builds, with the right wire
    /// type id — the drafts are not merely well-named.
    #[test]
    fn a_drafted_op_encodes_with_its_kinds_type_id() {
        let d = forum_react("aa".repeat(32), 4, "🔥".into(), true, 7);
        let e = encode_delta(d).expect("encodes");
        assert_eq!(e.type_id, 19, "a Forum delta carries 19");
        assert_eq!(e.op_id, pacific_core::coordinator::FORUM_REACT);
        assert!(!e.cbor_base64.is_empty());

        let h = system_hydrate("ra:event:1".into(), "{}".into(), 1, 0, false);
        let e = encode_delta(h).expect("encodes");
        assert_eq!(e.type_id, 21, "a System delta carries 21");
        assert_eq!(e.op_id, pacific_core::system::OP_HYDRATE);
    }

    /// THE PUBLISH CONSTRUCTORS ENCODE WHAT THE REDUCER READS.
    ///
    /// `reduce_publication` looks up `slug` and `publisher` by name, so a
    /// constructor that spelled either differently would produce a delta that
    /// encodes cleanly, travels, and is refused as `MalformedArgs` on every device
    /// including the author's. The catalogue's `args` row is the contract; this
    /// asserts the constructor honours it rather than that the row exists.
    #[test]
    fn the_publish_constructors_carry_the_args_the_reducer_looks_up() {
        let pk = "bb".repeat(32);
        let d = group_publish("reading-room".into(), pk.clone());
        let keys: Vec<&str> = d.args.iter().map(|a| a.key.as_str()).collect();
        let want = delta_catalog()
            .into_iter()
            .find(|s| s.op_id == op::BASE_PUBLISH)
            .expect("base.publish is in the catalogue");
        for k in want.arg_keys.iter() {
            assert!(
                keys.contains(&k.as_str()),
                "group_publish does not carry {k:?}, which the catalogue declares \
                 and `reduce_publication` looks up by name. It carries: {keys:?}"
            );
        }
        let e = encode_delta(d).expect("encodes");
        assert_eq!(e.type_id, 18, "a Group delta carries 18");
        assert_eq!(e.op_id, pacific_core::publication::OP_PUBLISH);

        // Unpublish takes NOTHING, and that is the reducer's shape too: it clears
        // both halves rather than being told which to clear.
        let u = group_unpublish();
        assert!(u.args.is_empty(), "base.unpublish takes no args");
        let e = encode_delta(u).expect("encodes");
        assert_eq!(e.op_id, pacific_core::publication::OP_UNPUBLISH);
    }

    /// The ICD's args for `name`, from its kind's ops or a facet's; `None` for an op the
    /// ICD does not declare (the reserved vocabularies).
    fn icd_args(kind: Kind, name: &str) -> Option<BTreeSet<String>> {
        let d: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json")).expect("the ICD"),
        )
        .expect("the ICD is JSON");
        let channel = format!("{kind:?}").to_lowercase();
        let op = d["kinds"][&channel]["ops"].get(name).or_else(|| {
            d["facets"].as_object()?.values().find_map(|f| f["ops"].get(name))
        })?;
        Some(op["args"].as_object().map(|a| a.keys().cloned().collect()).unwrap_or_default())
    }

    /// NC-9 / ICD 2.1.0 row 1: a spec the ICD declares carries the ICD's args, every one,
    /// and none it does not declare.
    #[test]
    fn every_spec_the_icd_declares_carries_the_icds_args() {
        let mut wrong = Vec::new();
        for s in delta_catalog() {
            let Some(theirs) = icd_args(s.kind, &s.name) else { continue };
            let ours: BTreeSet<String> = s.arg_keys.iter().cloned().collect();
            if ours != theirs {
                wrong.push(format!(
                    "{:?} {}: not declared {:?}, missing {:?}",
                    s.kind,
                    s.name,
                    ours.difference(&theirs).collect::<Vec<_>>(),
                    theirs.difference(&ours).collect::<Vec<_>>()
                ));
            }
        }
        assert!(wrong.is_empty(), "the catalogue states args the ICD does not:\n{}", wrong.join("\n"));
    }

    /// NC-38: a setRole draft carries only base.setRole's args, so authoring can take it.
    #[test]
    fn group_set_member_role_carries_base_set_roles_args() {
        let d = group_set_member_role("aa".repeat(32), "admin".into());
        let theirs = icd_args(Kind::Group, "base.setRole").expect("the ICD declares base.setRole");
        let ours: BTreeSet<String> = d.args.iter().map(|a| a.key.clone()).collect();
        assert_eq!(ours, theirs);
    }
}
