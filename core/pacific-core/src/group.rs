//! group — the Group GroupObject: the canonical, reusable IDENTITY record.
//!
//! A Group is a "who": a person (a degenerate group of ONE), a team acting as
//! one, or an organisation. It is a GroupObject (kind = Group, type_id 18) whose
//! own append-only Delta log — folded here — is the SOURCE OF TRUTH for that
//! identity's profile, presence, credential-vault index, and the per-member role
//! overlay. Every other object (Project/Forum/Field/chat) references the SAME
//! Group by its stable object id, so identity data lives in exactly one place and
//! updates propagate by reference.
//!
//! WHERE THE NO-DUAL-SOURCE LINE RUNS (this is the load-bearing correction):
//!   - WHO is a member  → the MLS ratchet tree (via mls::roster_identities). The
//!     `group_members` SQLite table is only its rebuildable cache. A team Group's
//!     members ARE its MLS roster; we NEVER store a second member list here.
//!   - Everything ABOUT a member (name, shape, presence, ROLE, credentials,
//!     contact card) → authored as this Group's own Deltas and folded into
//!     `GroupState`, keyed by the member's identity pubkey. `setMemberRole` layers
//!     a role ONTO a real roster member; it does not confer membership.
//!
//! All seven ops are owner/sequenced (op-ids 0..6, pinned to pacific-ffi's Group
//! catalog so the FFI re-exports these constants — the Rust wins).
//!
//! FEDERATION (Group↔Group): a Group may be a member of another Group — a chapter
//! inside a national federation — WITHOUT the rosters ever merging. Each side
//! records its OWN half of the edge in its OWN log (`setAffiliation`: peer object
//! id + rel parent/child/peer), so every chapter member folds "we belong to the
//! federation" out of a log they already carry, and no global registry exists.
//! The MLS roster invariant is untouched: an affiliation never confers membership;
//! the bilateral channel between two federated groups is a 2-member `group-tether`
//! whose members are one DELEGATE from each side.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::identity::parse_identity_key;
use crate::membership;
use crate::publication;
use crate::wallet;
use crate::object::{
    Authority, Commutativity, DeltaRejection, MemberId, PartRef, ObjectKind, ObjectType, Op, OpDecl,
};
use crate::object_args::{opt_text, req_hex_id, req_int, req_text};

/// The object kinds whose Delta log is folded by [`GroupType`] — i.e. the kinds that may carry
/// the Group op vocabulary (profile/presence/vault/**roles**).
///
///   `group`         the reusable identity record (person/team/org).
///   `arc-tether`    a 2-member Arc↔Arc tether — the mesh link between two Arcs.
///   `member-tether` a 2-member {user, Arc} tether — membership, carrying the member's ROLE.
///   `group-tether`  a 2-member {delegate, delegate} tether — the bilateral channel between
///                   two federated Groups. Its members are one delegate FROM each group; the
///                   groups themselves record the edge in their own logs via setAffiliation.
///
/// Deliberately NOT `connection`: a connection's log is folded by `Coordinator<ForumType>`
/// (`forum.post`/`react`/`receipt` — see `Node::folded_forum`). Two op vocabularies over ONE
/// append-only log would read each other's op ids as their own and silently corrupt the fold,
/// so a role is never authored onto a chat connection. This is the list `group_author` gates on.
pub const GROUP_TYPED_KINDS: [&str; 1] = [
    "group",
    // `arc-tether`, `member-tether` and `group-tether` WERE here. They were
    // 2-member Group-typed twins of a `connection`, minted only because a role
    // had to ride a Group-folded log — and standing is a facet now, so a
    // connection carries it and the twins are gone (24 Sep 2026).

];

/// Is `kind` an object whose log carries the Group op vocabulary? (Standing is no
/// longer the test — that is `crate::roles`, and any kind may carry it.)
pub fn is_group_typed(kind: &str) -> bool {
    GROUP_TYPED_KINDS.contains(&kind)
}

// ---- op ids (MUST match pacific-ffi/src/delta.rs `op::GROUP_*`) -------------------
pub const OP_SET_PROFILE: u32 = 0; // owner / sequenced
pub const OP_SET_PRESENCE: u32 = 1; // owner / sequenced
pub const OP_STORE_CREDENTIAL: u32 = 2; // owner / sequenced
pub const OP_REVOKE_CREDENTIAL: u32 = 3; // owner / sequenced
// 4 WAS `group.setMemberRole`. Standing is a FACET now
// (`crate::roles::OP_SET_ROLE`), because a role had to ride a Group-folded log
// and that is the only reason `member-tether` and `arc-tether` existed as
// Group-typed twins of a connection. The id is left unused, never recycled.
pub const OP_SET_AFFILIATION: u32 = 5; // owner / sequenced
pub const OP_CLEAR_AFFILIATION: u32 = 6; // owner / sequenced
// 7 and 8 WERE `group.setForum` / `group.clearForum`. They are
// the parts facet now (`crate::parts`, base ops in the 0xF006 band, `base.setPart`
// since 26 Sep 2026) — because hosting a Forum is not a Group feature: a Post's comments, an
// Event's discussion and a Listing's thread are the same act, and each kind that
// hand-rolled its own was re-implementing what a Forum already is. The ids are
// left unused rather than recycled: a delta from an older build carrying 7 must
// not fold as something else.
pub const OP_SET_COVER: u32 = 9; // owner / sequenced
pub const OP_SET_OFFICE: u32 = 10; // owner / sequenced
pub const OP_CLEAR_OFFICE: u32 = 11; // owner / sequenced
// THE SPINE'S OWN MEMBERSHIP. These two record where *I* went, on my own identity
// record, and they are the reciprocal of `base.memberJoined` / `base.memberLeft`,
// which live on the JOINED object and record who arrived there. Owner/sequenced on
// my own spine is the load-bearing half: nobody may assert a membership on my
// behalf, and nobody may erase one.
pub const OP_JOINED_OBJECT: u32 = 12; // owner / sequenced
pub const OP_LEFT_OBJECT: u32 = 13; // owner / sequenced
// THE FACE: the site's public page as presentation (core/docs/the-face-on-the-wire.md).
pub const OP_SET_FACE: u32 = 14; // owner / sequenced
/// The face, edited by the owner or an admin (ICD 2.1.0 row 10). owner|role:admin / commutative.
pub const OP_EDIT_FACE: u32 = 17;
/// A member lists one of their Things on the Site's Trade board (W-98 Trade, T-7).
pub const OP_PUBLISH_LISTING: u32 = 21; // member / commutative
/// The owner or an admin takes a member's listing off the board, for good.
pub const OP_REMOVE_LISTING: u32 = 22; // owner|role:admin / commutative
/// The kiosk keys whose claims admit to this Site (A-3, D-53): the founders register one.
pub const OP_SET_CLAIM_ISSUER: u32 = 15; // owner / sequenced
pub const OP_CLEAR_CLAIM_ISSUER: u32 = 16; // owner / sequenced
/// An attendee's answer to an Event this group created (W-98 Events; `crate::rsvp`).
pub const OP_RSVP: u32 = 18; // member / commutative
/// How the Site takes answers for one Event it created. owner|role:admin / commutative.
pub const OP_SET_REGISTRATION: u32 = 19;
/// The host's decision on one member's answer. owner|role:admin / commutative.
pub const OP_RSVP_DECIDE: u32 = 20;

/// The biggest face document, in bytes of JSON. The system.hydrate ceiling, ON
/// PURPOSE: publishing copies this document onto the group's Host as a hydrated
/// item, so a face the group can hold is always a face its Host can carry.
pub const MAX_FACE: usize = crate::system::MAX_HYDRATED_PAYLOAD;

// ---- cover caps (BYTES OF BASE64, enforced at fold — the event.rs media rule) ------
//
// The cover is ONE slot, wholesale replaced: a still (jpeg/png), an animated gif, or a
// short clip (mp4). Same scale as the event poster for stills; motion gets the clip
// budget. Oversize is refused, never clamped — re-encoding would make two devices
// disagree about the same delta.
pub const MAX_COVER_STILL_B64: usize = 700_000;
pub const MAX_COVER_CLIP_B64: usize = 1_500_000;

// ---- typed vocab -------------------------------------------------------------

/// One person, a team acting as one, or an organisation. A PLACE is deliberately
/// NOT here — a place is its own entity, never a Group (PRIMITIVE-DECISIONS §1.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GroupShape {
    #[default]
    Individual,
    Team,
    Organisation,
    /// A body of people with a shared interest rather than a shared employer. Its own
    /// shape rather than a UI-side refinement of `Organisation`: the app was keeping
    /// "is this a community" in UserDefaults beside the object, which is a second
    /// source of truth for something the identity record is FOR.
    Community,
}
impl GroupShape {
    /// Parse the wire vocabulary. `pub` because `contact::publishProfile` speaks the
    /// same shape vocabulary as `group.setProfile` — one parser, not two.
    pub fn parse(s: &str) -> Result<Self, DeltaRejection> {
        Ok(match s {
            "individual" => Self::Individual,
            "team" => Self::Team,
            "organisation" => Self::Organisation,
            "community" => Self::Community,
            _ => return Err(DeltaRejection::MalformedArgs),
        })
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Individual => "individual",
            Self::Team => "team",
            Self::Organisation => "organisation",
            Self::Community => "community",
        }
    }
}

/// Where an appointment has got to. `Pending` is a real state, not a UI flag: the
/// roster add and the appointment are two different acts, and between them the office
/// is spoken for without being filled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OfficeStatus {
    Pending,
    Accepted,
}

impl OfficeStatus {
    pub fn parse(s: &str) -> Result<Self, DeltaRejection> {
        match s {
            "pending" => Ok(Self::Pending),
            "accepted" => Ok(Self::Accepted),
            // `vacant` is deliberately NOT a value: a vacant office is the ABSENCE of
            // an entry, so it can never disagree with one.
            _ => Err(DeltaRejection::MalformedArgs),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
        }
    }
}

/// One office and who holds it. The office NAME is free text (chair, treasurer,
/// secretary today; a co-op will want others) — a closed enum in core would mean a
/// core release every time a body invents a role for itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfficeHolder {
    /// The holder's identity pubkey (hex). Must be a roster member at fold time.
    pub holder: MemberId,
    pub status: OfficeStatus,
    /// Unix ms the appointment was authored.
    pub at: i64,
}

/// The capacity a member plays IN this Group (owner/admin/member/viewer/guest).
/// A role is a per-member property of the Group, layered on a real roster member.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupRole {
    Owner,
    Admin,
    Member,
    Viewer,
    Guest,
    /// ANOTHER ARC, federated with this one — not a standing within a body but a
    /// standing on the link itself. Added 24 Sep 2026 with the tether collapse:
    /// `arc-tether` and `member-tether` were separate KINDS whose only difference
    /// was what the other party is to us, and that is what a role says. One kind
    /// (`connection`), and the role tells a peer from a member.
    Peer,
    /// May add a member and record the Add, and nothing else (A-3, D-53): the Arc's
    /// always-on node, granted by a founder, revoked with `base.clearRole`.
    Admitter,
    /// A Transaction's three principals (W-98 Trade, T-3), on a Transaction only and
    /// fixed for its lifecycle (T-2): the settler owns it and moves only a dispute, the
    /// seller states the terms and attests the sale, the buyer accepts and attests receipt.
    Settler,
    Seller,
    Buyer,
}
impl GroupRole {
    /// May this role bring new objects into the Space's world — a Channel, an Event, a
    /// Listing minted AS the Space rather than as a person?
    ///
    /// ONE definition, here, so the `+` a member sees and the rule the fold applies can
    /// be read side by side. Scattered `is_owner` checks in the UI were the alternative,
    /// and they drift.
    ///
    /// # Why this is Owner-only, and what it would take to widen
    ///
    /// Belonging is authored by `base.setPart` and `group.setAffiliation`
    /// (op 5), and BOTH are `Authority::Owner`. `Authority` has exactly two levels —
    /// `Owner` and `AnyMember` — so there is no rung for Admin to stand on: the reducer
    /// cannot express "an admin may do this", and `group_author` refuses an owner-only op
    /// from a non-owner rather than persisting a delta that bricks the object at fold.
    ///
    /// So an Admin could mint the object and then fail to attach it — a half-done mint,
    /// which is worse than no button. Widening this means teaching the reducer to read
    /// `member_roles` at reduce time (a new authority level), which is a governance
    /// decision about who may speak for a Space, not a UI tweak.
    pub const fn may_mint(self) -> bool {
        matches!(self, Self::Owner)
    }

    pub fn parse(s: &str) -> Result<Self, DeltaRejection> {
        Ok(match s {
            "owner" => Self::Owner,
            "admin" => Self::Admin,
            "member" => Self::Member,
            "viewer" => Self::Viewer,
            "guest" => Self::Guest,
            "peer" => Self::Peer,
            "admitter" => Self::Admitter,
            "settler" => Self::Settler,
            "seller" => Self::Seller,
            "buyer" => Self::Buyer,
            _ => return Err(DeltaRejection::MalformedArgs),
        })
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Member => "member",
            Self::Viewer => "viewer",
            Self::Guest => "guest",
            Self::Peer => "peer",
            Self::Admitter => "admitter",
            Self::Settler => "settler",
            Self::Seller => "seller",
            Self::Buyer => "buyer",
        }
    }

    /// A deal's role: a Transaction's alone (the ICD's setRole: "on a Transaction only").
    pub const fn is_deal(self) -> bool {
        matches!(self, Self::Settler | Self::Seller | Self::Buyer)
    }
}

/// The kind of secret a stored credential HANDLE points at. The shape is public;
/// the secret material never enters the object graph (only a content-free handle).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredentialKind {
    ApiKey,
    OauthToken,
    BearerToken,
    BasicAuth,
    SshKey,
    Custom,
}
impl CredentialKind {
    pub fn parse(s: &str) -> Result<Self, DeltaRejection> {
        Ok(match s {
            "apiKey" => Self::ApiKey,
            "oauthToken" => Self::OauthToken,
            "bearerToken" => Self::BearerToken,
            "basicAuth" => Self::BasicAuth,
            "sshKey" => Self::SshKey,
            "custom" => Self::Custom,
            _ => return Err(DeltaRejection::MalformedArgs),
        })
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ApiKey => "apiKey",
            Self::OauthToken => "oauthToken",
            Self::BearerToken => "bearerToken",
            Self::BasicAuth => "basicAuth",
            Self::SshKey => "sshKey",
            Self::Custom => "custom",
        }
    }
}

/// Whether this Group is a live peer we can seal MLS envelopes to, or a known
/// contact with no syncable space yet. Never a fabricated space id.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Presence {
    /// Not yet declared — a fresh group-of-1 before setPresence.
    #[default]
    Unknown,
    /// A live peer, addressable by their IdentityKey (space1<hex of identity pubkey>).
    OnPlatform(String),
    /// A known contact; `inviteHint` (email/phone/link) reaches them out-of-band.
    OffPlatform(Option<String>),
}
impl Presence {
    pub fn identity_key(&self) -> Option<&str> {
        match self {
            Presence::OnPlatform(s) => Some(s),
            _ => None,
        }
    }
    pub fn invite_hint(&self) -> Option<&str> {
        match self {
            Presence::OffPlatform(h) => h.as_deref(),
            _ => None,
        }
    }
}

/// A labelled contact value (e.g. label "work", value an email/phone/URL).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Labeled {
    pub label: String,
    pub value: String,
}

/// The rich contact card — the vCard-shaped payload of a directory entry. Carried
/// as one optional JSON `card` arg on setProfile (ArgVal is Int|Text only), so the
/// identity essentials (displayName, shape) stay first-class scalars.
///
/// EVERY field is `#[serde(default)]`, which is what makes the card additively
/// extensible: a peer running an older build folds a card carrying fields it has
/// never heard of, keeps what it understands, and drops the rest — no version
/// negotiation, no rejected delta. That property is load-bearing for the avatar
/// slot below.
///
/// THE AVATAR IS TWO SLOTS, NOT ONE. `photo` is the STILL (and the only thing
/// vCard PHOTO interop can carry). `clip` is the parallel MOTION slot: the "last
/// seen" pinhole front-camera capture. They are different media, not two copies of
/// one — so holding both is not a dual source. A renderer resolves in this order:
///
///   clip (fresh motion) → photo (still) → block colour + initials
///
/// which is exactly "in lieu of OR in parallel to a profile photo".
///
/// THE CLIP DOES NOT CROSS THE CONNECTION FAN-OUT. A profile edit authors one delta
/// per Connection, so every byte on this card is paid for N times over with no
/// dedup possible — affordable for a ~100KB still, not for 1.5MB of motion. The
/// `contact.publishProfile` fold therefore REFUSES a card carrying inline clip
/// bytes, and `Node::set_my_profile` refuses to author one. The slot stays here for
/// renderers and for the detached route (`media::Delivery::Detached` carries a
/// digest, not bytes) — which is how "last seen" ships. A group's `setProfile` is
/// unaffected: that is one delta to one MLS group, not N.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContactCard {
    #[serde(default)]
    pub org: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub emails: Vec<Labeled>,
    #[serde(default)]
    pub phones: Vec<Labeled>,
    #[serde(default)]
    pub urls: Vec<Labeled>,
    /// base64 STILL image data (no external fetch); "" when none.
    #[serde(default)]
    pub photo: String,
    /// MIME of `photo` ("image/jpeg" when the app set it); "" when no photo.
    #[serde(default)]
    pub photo_mime: String,
    /// base64 MOTION data — the "last seen" pinhole clip; "" when none. Parallel to
    /// `photo`, never a replacement for it: a peer that can't play the clip still
    /// has the still to fall back to.
    ///
    /// MUST be empty on a `contact.publishProfile` card — inline clip bytes are
    /// refused by that fold (see the type doc). A group's `setProfile` still takes
    /// one, because a group card crosses once rather than once per Connection.
    #[serde(default)]
    pub clip: String,
    /// MIME of `clip` (e.g. "video/mp4"); "" when no clip.
    #[serde(default)]
    pub clip_mime: String,
    /// When the avatar media was CAPTURED (unix secs; 0 = unknown). This is what
    /// makes "last seen" mean something — freshness is a property of the capture,
    /// not of when the delta happened to arrive.
    #[serde(default)]
    pub captured_at: u64,
    /// The block-colour fallback as "RRGGBB" — what a peer renders behind the
    /// initials when there is no photo and no clip. Chosen by the card's owner, so
    /// their disc looks the same on every device that holds them.
    #[serde(default)]
    pub avatar_color: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Where you are, as a WORD — "London", "Seoul". A display string the owner
    /// typed, deliberately not a coordinate: the first-boot BIO step must be able
    /// to fill it without the app asking iOS for the location permission, and the
    /// map's permission belongs to the map. A Place/Event's `setLocation` GeoPoint
    /// is the geographic primitive; this is not one and is never resolved to one.
    #[serde(default)]
    pub city: String,
}

/// Content-free vault handle metadata. The secret lives in a device-local secure
/// store, NEVER here and never in an export.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialMeta {
    pub kind: CredentialKind,
    pub label: String,
}

/// How the PEER group relates to THIS group, from this group's point of view.
/// A chapter records its federation as `parent`; the federation records each
/// chapter as `child`; sister groups record each other as `peer`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AffiliationRel {
    Parent,
    Child,
    Peer,
    /// This group is ANCHORED AT a Place — someone printed a QR for a bench, a tree, an
    /// allotment, and this group put itself on it.
    ///
    /// Unlike the other three the peer is not a sovereign group but a Place anchor
    /// (`ObjectKind::Place`), and the edge carries an OBLIGATION: by anchoring here, this
    /// group's members undertake to admit anyone who scans that place and picks them.
    ///
    /// The duty has to sit on the GROUP rather than on the Place, because MLS admits
    /// nobody without a member of THAT group committing the Add — no other group can
    /// discharge it on your behalf. Which is exactly the point: it moves the burden off
    /// whichever phone printed the QR and onto the membership, which is many phones. A
    /// place is only as reachable as the groups that agreed to answer for it.
    ///
    /// It is a COMMITMENT, not an enforcement — nothing can make a device run. What the
    /// delta buys is that the promise is durable, legible to every member of the group,
    /// and revocable (`clearAffiliation`) rather than silently assumed.
    Anchored,
    /// This group CREATED the peer object — an Event it hosts, a Thing it lists (a job,
    /// a skill, inventory, a trade). Like `Anchored`, the peer is not a sovereign group;
    /// the edge is the group publishing "this is ours", recorded on the GROUP's log so
    /// every member folds the same catalogue.
    Created,
    /// This group (a performer: a person's self record, an organisation) PERFORMS AT the
    /// Event `peer` (W-98). The performer's half of `performs_at`, and the act's consent; the
    /// Event's half is its lineup.
    PerformsAt,
}
impl AffiliationRel {
    pub fn parse(s: &str) -> Result<Self, DeltaRejection> {
        Ok(match s {
            "parent" => Self::Parent,
            "child" => Self::Child,
            "peer" => Self::Peer,
            "anchored" => Self::Anchored,
            "created" => Self::Created,
            "performs_at" => Self::PerformsAt,
            _ => return Err(DeltaRejection::MalformedArgs),
        })
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Parent => "parent",
            Self::Child => "child",
            Self::Peer => "peer",
            Self::Anchored => "anchored",
            Self::Created => "created",
            Self::PerformsAt => "performs_at",
        }
    }

    /// Whether this edge obliges the group to service join requests arriving at the peer.
    /// Only `Anchored` does — a federation edge carries no such duty.
    pub const fn answers_for_peer(self) -> bool {
        matches!(self, Self::Anchored)
    }
}

/// One half of a Group↔Group edge, as recorded in THIS group's log. The other
/// group records the mirror half in its own log — two sovereign claims, no shared
/// row. `at` is EVENT time (unix ms, supplied by the author) so the projection
/// into the bi-temporal graph gets a real `valid_at`, not an arrival time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Affiliation {
    pub rel: AffiliationRel,
    /// The peer group's display name as known at authoring (a label, not truth —
    /// the peer's own log owns its name).
    pub name: String,
    /// Object id of the 2-member `group-tether` carrying the bilateral channel;
    /// "" when the edge was recorded without one.
    pub tether: String,
    /// Unix ms when the affiliation became effective (event time).
    pub at: i64,
}

/// The parts this Group is made of — its rooms, its Treasury, its Host, its
/// sub-groups. Shape: [`crate::object::PartRef`], one edge for every kind of part
/// (the ICD's `part_of`). Why this is not an Affiliation, and why membership stays
/// on the part's roster, is on `PartRef`.

/// One object this record's owner belongs to — or belonged to. The SPINE's own
/// membership, folded.
///
/// This is the only enumeration of a person's graph that survives them opening
/// Pacific on a device that has never seen it: fold the spine and every other
/// object is reachable from here. `tag` addresses that object's sealed way-in
/// material, which is key material and therefore lives on the relay rather than
/// in this log — `group.storeCredential` already rules that secrets never enter
/// the object graph.
///
/// The record is KEPT when you leave, dated rather than removed, so the fold
/// answers both "am I in X" and "was I ever". Re-joining is a later
/// `joinedObject` and the sequenced order resolves it with no tombstone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Joined {
    /// The directory kind — "forum", "place", "project", … — which decides the lens.
    /// It rides here because it only ever travels in a sealed Welcome otherwise, and
    /// a returning device never gets one.
    pub kind: String,
    /// The arc that governs the object, so a restored device routes to the same one
    /// its creator chose rather than falling back to its own default.
    pub arc: String,
    /// The relay tag of this object's sealed way-in entry, hex.
    pub tag: String,
    /// Unix ms at the join (event time, author-supplied).
    pub at: i64,
    /// Unix ms at departure; `None` while still a member. This is the RECORD of a
    /// departure on my own identity record; the departure itself is the MLS Remove
    /// (`Node::group_leave`, `Node::group_remove_member` — membership-through-mls.md
    /// §5–§6), which is what takes the leaf out of the tree.
    pub left_at: Option<i64>,
    /// Why, captured at departure. Free text, and the symmetric partner of the
    /// `why` a connection carries at the moment it forms. Empty when unstated.
    pub reason: String,
}

// ---- the folded state --------------------------------------------------------

/// What a spent claim carried besides its member (ICD 2.1.0 row 2): the claim's `c` and `a`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimPick {
    pub choice: Option<String>,
    pub share: Option<String>,
}

/// The compiled projection of a Group's Delta log — the source of truth for the
/// identity. `Default` is the genesis of a fresh group-of-1 (an individual).
/// NOTE: member SET is NOT here — that is the MLS roster; this holds the overlay.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GroupState {
    pub display_name: String,
    pub shape: GroupShape,
    pub presence: Presence,
    pub card: ContactCard,
    /// role OVERLAY on the MLS roster, keyed by member identity pubkey.
    pub member_roles: BTreeMap<MemberId, GroupRole>,
    /// content-free credential handles, keyed by handle id.
    pub credentials: BTreeMap<String, CredentialMeta>,
    /// Group↔Group edges (this group's half), keyed by the PEER group's object id.
    /// Never a roster: an affiliation relates two sovereign groups by reference.
    pub affiliations: BTreeMap<String, Affiliation>,
    /// Every tenure ever held on this group. NOT the roster — the MLS ratchet tree
    /// stays authoritative for who is in. This is the HISTORY the tree cannot keep:
    /// when each spell started, when it ended, and whether they left or were
    /// removed. `membership::MembershipLog::divergence` ties the two together.
    pub membership: membership::MembershipLog,
    /// office name -> who holds it. An office with no entry is VACANT — absence is
    /// the state, so "vacant" needs no representation and cannot contradict a holder.
    pub offices: BTreeMap<String, OfficeHolder>,
    /// The parts this group is made of, keyed by the part's object id, each in its
    /// role. The set every member folds identically; who is IN each part stays the
    /// part's MLS roster.
    pub parts: BTreeMap<String, PartRef>,
    /// THE SPINE'S MEMBERSHIP: the objects this record's owner joined, keyed by the
    /// object's id (hex). Not to be confused with `membership` above, which is every
    /// tenure others held on THIS group — this is the opposite direction, and it is
    /// what makes the spine the entry point to the rest of the graph.
    pub joined: BTreeMap<String, Joined>,
    /// THE FACE — this group's public page as a JSON document, and the WORKING COPY:
    /// members see it, it is never served. Publishing copies it across the seam onto
    /// the group's Host, where the Arc can read it and the group cannot be read.
    pub face: String,
    /// The kiosk keys whose claims admit to this Site, by key id: base64url of the
    /// 32-byte Ed25519 key (A-3, D-53).
    pub claim_issuers: BTreeMap<String, String>,
    /// Every claim spent on an Add here: sha256 of its nonce, hex, to who it admitted.
    /// The first per claim holds.
    pub claims_spent: BTreeMap<String, MemberId>,
    /// Each spent claim that carried a choice or a share, by the same key.
    pub claims_picked: BTreeMap<String, ClaimPick>,
    /// The COVER — the banner across the top of the group's page, base64. "" = none.
    /// One slot beside the card's avatar (photo/clip): the avatar is the group's FACE,
    /// the cover is its WALL.
    pub cover: String,
    /// MIME of `cover`: image/jpeg | image/png | image/gif | video/mp4; "" when none.
    pub cover_mime: String,
    /// The answers to the Events this group created, their registers and the hosts'
    /// decisions (group.rsvp, setRegistration, rsvpDecide; `crate::rsvp`).
    pub rsvps: crate::rsvp::Rsvps,
    /// Each member's own bio and links here (`base.publishAbout`), by its author.
    pub about: BTreeMap<MemberId, crate::about::About>,
    /// The owner's and admins' questions and the members' answers (`crate::questions`).
    pub questions: crate::questions::Questions,
    /// The group's public address and the member that serves it — the SITE face.
    /// Default is unpublished, so a group that predates the facet stays private.
    pub publication: publication::Publication,
    /// Notes as folded state. Default is EMPTY, so every Group-typed object that
    /// predates the facet folds to a notebook with no entries rather than failing.
    /// This is the field that puts a person's notes inside the archive, and
    /// therefore inside the backup, by construction — see `crate::note`.
    pub notebook: crate::note::NoteBook,
    /// What this object is a part of, and in what role (`base.setParent`). `None`
    /// until a parent names it as a part.
    pub parent: Option<crate::parent::ParentRef>,
    /// Each member's own card here (`base.publishProfile`), by its author. Empty for every
    /// log that predates the facet.
    pub profiles: BTreeMap<MemberId, crate::profiles::Profile>,
    /// THE TRADE BOARD (W-98 Trade, T-7): each live listing by (lister, their Thing).
    pub listings: BTreeMap<(MemberId, String), SiteListing>,
    /// Listings the owner or an admin took off, for good (`group.removeListing`).
    pub listings_removed: std::collections::BTreeSet<(MemberId, String)>,
}

/// One listing on a Site's Trade board: a snapshot of the lister's Thing, as
/// `contact.publishListing` carries one pairwise, less the relay's fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteListing {
    #[serde(serialize_with = "hex_member")]
    pub author: MemberId,
    pub thing_id: String,
    pub posture: String,
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub descriptor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area: Option<String>,
    pub reach: String,
    pub rev: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub photo: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub photo_mime: String,
    /// The Site's own listing, the Site sells it (`site` 1; relation sold_through).
    pub site: bool,
    pub gen: i64,
}

fn hex_member<S: serde::Serializer>(m: &MemberId, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&hex::encode(m))
}

fn hex64(t: &str) -> bool {
    t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `group.publishListing` and `group.removeListing` (W-98 Trade, T-7). Folded in (gen,
/// author, id) order, so the latest by gen stands for each (lister, Thing). Validated whole
/// before the first mutation.
fn reduce_listing(state: &mut GroupState, op: &Op<'_>) -> Result<(), DeltaRejection> {
    use crate::coordinator::ArgVal;
    let args = op.args;
    let gen = match crate::arg_reads::get(args, "gen") {
        Some(ArgVal::Int(g)) => *g,
        _ => return Err(DeltaRejection::MalformedArgs),
    };
    let owner_or_admin = *op.author == op.ctx.owner || (op.ctx.is_member(op.author) && state.member_roles.get(op.author) == Some(&GroupRole::Admin));
    if op.op_id == OP_REMOVE_LISTING {
        let author = req_text(args, "author")?;
        let thing_id = req_text(args, "thingId")?;
        if !hex64(author) || !hex64(thing_id) {
            return Err(DeltaRejection::MalformedArgs);
        }
        if !owner_or_admin {
            return Err(DeltaRejection::PreconditionFailed);
        }
        let mut who = [0u8; 32];
        hex::decode_to_slice(author, &mut who).map_err(|_| DeltaRejection::MalformedArgs)?;
        let key = (who, thing_id.to_string());
        state.listings.remove(&key);
        state.listings_removed.insert(key);
        return Ok(());
    }
    let thing_id = req_text(args, "thingId")?.to_string();
    let posture = crate::thing::Posture::parse(req_text(args, "posture")?)?;
    let title = req_text(args, "title")?.to_string();
    let reach = crate::thing::Reach::parse(req_text(args, "reach")?)?;
    let rev = req_int(args, "rev")?;
    if !hex64(&thing_id) || title.trim().is_empty() || rev < 0 {
        return Err(DeltaRejection::MalformedArgs);
    }
    let text = |k: &str| match crate::arg_reads::get(args, k) {
        Some(ArgVal::Text(t)) => Ok(t.clone()),
        None => Ok(String::new()),
        _ => Err(DeltaRejection::MalformedArgs),
    };
    let int = |k: &str| match crate::arg_reads::get(args, k) {
        Some(ArgVal::Int(n)) => Ok(Some(*n)),
        None => Ok(None),
        _ => Err(DeltaRejection::MalformedArgs),
    };
    let descriptor = text("descriptor")?;
    let price = Some(text("price")?).filter(|p| !p.is_empty());
    // A price on a standing intent is meaningless: `thing`'s own rule.
    if price.is_some() && !posture.is_active() {
        return Err(DeltaRejection::MalformedArgs);
    }
    let deadline = int("deadline")?.filter(|d| *d > 0);
    let area = Some(text("area")?).filter(|a| !a.is_empty());
    let withdrawn = int("withdrawn")?.is_some_and(|w| w != 0);
    let site = int("site")?.is_some_and(|w| w != 0);
    let photo = text("photo")?;
    let photo_mime = text("photoMime")?;
    let face = crate::media::MediaRef {
        kind: crate::media::MediaKind::Still,
        mime: photo_mime.clone(),
        delivery: crate::media::Delivery::Inline { data: photo.clone() },
        width: 0,
        height: 0,
        duration_ms: 0,
    };
    if face.validate_bounds(crate::media::Slot::Avatar).is_err() {
        return Err(DeltaRejection::MalformedArgs);
    }
    // The Site's own listing is the owner's or an admin's word (relation sold_through).
    if site && !owner_or_admin {
        return Err(DeltaRejection::PreconditionFailed);
    }
    let key = (*op.author, thing_id.clone());
    if state.listings_removed.contains(&key) {
        return Err(DeltaRejection::PreconditionFailed);
    }
    if withdrawn {
        state.listings.remove(&key);
        return Ok(());
    }
    state.listings.insert(
        key,
        SiteListing { author: *op.author, thing_id, posture: posture.as_str().into(), title, descriptor, price, deadline, area, reach: reach.as_str().into(), rev, photo, photo_mime, site, gen },
    );
    Ok(())
}

/// The read view the CLI/FFI bind to (roster is threaded in by the caller, since
/// membership is the MLS ratchet tree, not this log).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupView {
    pub display_name: String,
    pub shape: GroupShape,
    pub presence: Presence,
    pub card: ContactCard,
    /// (member pubkey, role) for the roster members that carry a role overlay.
    pub roles: Vec<(MemberId, GroupRole)>,
    pub credentials: Vec<(String, CredentialMeta)>,
    /// (peer group object id, affiliation) — this group's half of each Group↔Group edge.
    pub affiliations: Vec<(String, Affiliation)>,
    /// (part object id, part ref) — what this group is made of, each in its role.
    pub parts: Vec<(String, PartRef)>,
    /// (office name, holder) — the appointments this body has made.
    pub offices: Vec<(String, OfficeHolder)>,
    /// The cover banner (base64 + mime); "" when none.
    pub cover: String,
    pub cover_mime: String,
    /// Can we currently reach this identity over MLS? (on-platform.)
    pub can_sync: bool,
}

impl GroupState {
    /// Project to the read view. `roster` is the MLS membership (source of truth);
    /// we surface the role overlay only for pubkeys that ARE roster members.
    pub fn view(&self, roster: &[MemberId]) -> GroupView {
        let roles = roster
            .iter()
            .filter_map(|m| self.member_roles.get(m).map(|r| (*m, *r)))
            .collect();
        let credentials = self
            .credentials
            .iter()
            .map(|(id, m)| (id.clone(), m.clone()))
            .collect();
        let affiliations = self
            .affiliations
            .iter()
            .map(|(peer, a)| (peer.clone(), a.clone()))
            .collect();
        let parts = self
            .parts
            .iter()
            .map(|(id, f)| (id.clone(), f.clone()))
            .collect();
        let offices = self
            .offices
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        GroupView {
            display_name: self.display_name.clone(),
            shape: self.shape,
            presence: self.presence.clone(),
            card: self.card.clone(),
            roles,
            credentials,
            affiliations,
            parts,
            offices,
            cover: self.cover.clone(),
            cover_mime: self.cover_mime.clone(),
            can_sync: matches!(self.presence, Presence::OnPlatform(_)),
        }
    }
}

// ---- op catalog + reducer ----------------------------------------------------

static GROUP_OPS: &[OpDecl] = &[
    OpDecl {
        op_id: OP_SET_PROFILE,
        name: "group.setProfile",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SET_PRESENCE,
        name: "group.setPresence",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_STORE_CREDENTIAL,
        name: "group.storeCredential",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_REVOKE_CREDENTIAL,
        name: "group.revokeCredential",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    // The BASE standing op-group, spliced — see `crate::roles`.
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
    OpDecl {
        op_id: OP_SET_AFFILIATION,
        name: "group.setAffiliation",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_CLEAR_AFFILIATION,
        name: "group.clearAffiliation",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    // The BASE parts op-group, spliced. Written out rather than concatenated
    // because `parts::PART_OPS` is a `static` and Rust cannot read one in a
    // const initialiser; `base_part_ops_are_spliced_verbatim` pins these to it.
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
    OpDecl {
        op_id: OP_JOINED_OBJECT,
        name: "group.joinedObject",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_LEFT_OBJECT,
        name: "group.leftObject",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SET_COVER,
        name: "group.setCover",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    // The base PROFILES op, spliced: each member's own card (O-77). Written out, as the
    // parts ops are; `base_profile_op_is_spliced_verbatim` pins it to `PROFILE_OPS`.
    OpDecl {
        op_id: crate::profiles::OP_PUBLISH_PROFILE,
        name: "base.publishProfile",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    // The base ABOUT and QUESTIONS facets (W-98 Members), spliced: written out, as the
    // profiles op is; `base_about_op_is_spliced_verbatim` and
    // `base_question_ops_are_spliced_verbatim` pin them to `ABOUT_OPS` and `QUESTION_OPS`.
    // An answer before a retirement: the ICD's args probe folds each op on the state the ones
    // above it left, and a retired question takes no answer.
    OpDecl {
        op_id: crate::about::OP_PUBLISH_ABOUT,
        name: "base.publishAbout",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: crate::questions::OP_DEFINE_QUESTION,
        name: "base.defineQuestion",
        authority: Authority::OwnerOrRole(GroupRole::Admin),
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: crate::questions::OP_ANSWER_QUESTION,
        name: "base.answerQuestion",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: crate::questions::OP_RETIRE_QUESTION,
        name: "base.retireQuestion",
        authority: Authority::OwnerOrRole(GroupRole::Admin),
        commutativity: Commutativity::Commutative,
    },
    // ---- base membership op-group, spliced verbatim -------------------------
    // Duplicated rather than referenced because `membership::MEMBERSHIP_OPS` is a
    // `static` and Rust cannot read a static in a const initialiser (the same
    // reason place.rs duplicates geo's). `base_membership_ops_are_spliced_verbatim`
    // asserts they stay identical, so the duplication cannot drift silently.
    OpDecl {
        op_id: OP_SET_OFFICE,
        name: "group.setOffice",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_CLEAR_OFFICE,
        name: "group.clearOffice",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SET_FACE,
        name: "group.setFace",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_EDIT_FACE,
        name: "group.editFace",
        authority: Authority::OwnerOrRole(GroupRole::Admin),
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_PUBLISH_LISTING,
        name: "group.publishListing",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_REMOVE_LISTING,
        name: "group.removeListing",
        authority: Authority::OwnerOrRole(GroupRole::Admin),
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_SET_CLAIM_ISSUER,
        name: "group.setClaimIssuer",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_CLEAR_CLAIM_ISSUER,
        name: "group.clearClaimIssuer",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    // W-98 Events: the answers to the Events this group created (`crate::rsvp`).
    OpDecl {
        op_id: OP_RSVP,
        name: "group.rsvp",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_SET_REGISTRATION,
        name: "group.setRegistration",
        authority: Authority::OwnerOrRole(GroupRole::Admin),
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_RSVP_DECIDE,
        name: "group.rsvpDecide",
        authority: Authority::OwnerOrRole(GroupRole::Admin),
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: membership::OP_MEMBER_JOINED,
        name: "base.memberJoined",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: membership::OP_MEMBER_LEFT,
        name: "base.memberLeft",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: membership::OP_OWNER_HANDOVER,
        name: "base.ownerHandover",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: membership::OP_CLAIM_SPENT,
        name: "base.claimSpent",
        authority: Authority::OwnerOrRole(GroupRole::Admitter),
        commutativity: Commutativity::Commutative,
    },
    // The base PUBLICATION ops, written out rather than concatenated for the same
    // const-initialiser reason geo and visibility are spliced verbatim elsewhere;
    // `base_publication_ops_are_spliced_verbatim` pins them against drift.
    OpDecl {
        op_id: publication::OP_PUBLISH,
        name: "base.publish",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: publication::OP_UNPUBLISH,
        name: "base.unpublish",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    // THE WALLET OPS WERE HERE. They moved to `treasury` (kind 31) on 25 September
    // 2026. The comment they carried stated the ruling that was reversed: "A Group is
    // where a shared treasury hangs, because the group's roster IS the electorate and a
    // second roster could diverge from it." shared-wallets.html §104 named the condition
    // for reversing that — a member of the site who cannot see the money — and multiple
    // membership per node made it the rule rather than the exception.
    //
    // The `0xF004` band is VACATED, not reused: an old delta carrying one of those ids
    // must fail loud rather than fold as something else.
    // THE NOTE OPS WERE HERE. They moved to `note` (kind 32) on 25 September 2026,
    // and `notebook` collapsed into it: a private notebook is a Note whose roster is
    // one, which is a fact about the roster and never was a kind. The `0xF005` band
    // is VACATED, not reused.
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

/// A face document as group.setFace and group.editFace both take it. ONE document,
/// wholesale replaced; empty clears. Refused, never trimmed, over MAX_FACE — a clipped
/// document would parse on some devices and not others. It must be a JSON OBJECT: every
/// reader (the editor, the Host copy, the renderer) parses the same bytes, and a face that
/// is not one is a face nobody can draw.
fn face_of(state: &GroupState, args: &crate::coordinator::Args) -> Result<String, DeltaRejection> {
    let face = req_text(args, "face")?;
    if face.is_empty() {
        return Ok(String::new());
    }
    // A SITE IS A GROUP THAT HAS A HOST PART, and only a Site has a Face (A-11, Ralph
    // 27 Sep): refused here, so at write and at fold alike.
    if !state.parts.values().any(|p| p.role == crate::parts::HOST_ROLE) {
        return Err(DeltaRejection::PreconditionFailed);
    }
    if face.len() > MAX_FACE {
        return Err(DeltaRejection::MalformedArgs);
    }
    match serde_json::from_str::<serde_json::Value>(face) {
        Ok(serde_json::Value::Object(_)) => Ok(face.to_string()),
        _ => Err(DeltaRejection::MalformedArgs),
    }
}

pub struct GroupType;

impl ObjectType for GroupType {
    const KIND: ObjectKind = ObjectKind::Group;
    type State = GroupState;

    fn ops() -> &'static [OpDecl] {
        GROUP_OPS
    }

    fn role_of(state: &GroupState, member: &MemberId) -> Option<GroupRole> {
        state.member_roles.get(member).copied()
    }

    fn reduce(state: &mut GroupState, op: &Op<'_>) -> Result<(), DeltaRejection> {
        if crate::parent::is_parent_op(op.op_id) {
            return crate::parent::reduce_parent(&mut state.parent, op);
        }
        // Base ops first: they own a reserved id band, so this can never shadow a
        // Group op, and routing them here keeps the facet's validation in ONE place
        // rather than reimplemented per kind.
        // A SPENT CLAIM (A-3): only the owner or an admitter writes one, and the first per
        // claim holds, so a claim admits once however many copies race.
        if op.op_id == membership::OP_CLAIM_SPENT {
            let author = *op.author;
            if author != op.ctx.owner && state.member_roles.get(&author) != Some(&GroupRole::Admitter) {
                return Err(DeltaRejection::PreconditionFailed);
            }
            let claim = req_text(op.args, "claim")?.to_ascii_lowercase();
            if claim.len() != 64 || !claim.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(DeltaRejection::MalformedArgs);
            }
            let member: MemberId = hex::decode(req_hex_id(op.args, "member")?)
                .ok()
                .and_then(|b| b.try_into().ok())
                .ok_or(DeltaRejection::MalformedArgs)?;
            if state.claims_spent.contains_key(&claim) {
                return Err(DeltaRejection::PreconditionFailed);
            }
            // The claim's `c` and `a` (ICD 2.1.0 row 2), each held to the claim's own rule.
            let choice = match crate::arg_reads::get(op.args, "choice") {
                None => None,
                Some(crate::coordinator::ArgVal::Text(c)) if crate::claim::CHOICES.contains(&c.as_str()) => Some(c.clone()),
                Some(_) => return Err(DeltaRejection::MalformedArgs),
            };
            let share = match crate::arg_reads::get(op.args, "share") {
                None => None,
                Some(crate::coordinator::ArgVal::Text(a)) if crate::claim::is_share(a) => Some(a.clone()),
                Some(_) => return Err(DeltaRejection::MalformedArgs),
            };
            if choice.is_some() || share.is_some() {
                state.claims_picked.insert(claim.clone(), ClaimPick { choice, share });
            }
            state.claims_spent.insert(claim, member);
            return Ok(());
        }
        if membership::is_membership_op(op.op_id) {
            return membership::reduce_membership(&mut state.membership, op);
        }
        if crate::parts::is_part_op(op.op_id) {
            return crate::parts::reduce_parts(&mut state.parts, op);
        }
        if crate::profiles::is_profile_op(op.op_id) {
            return crate::profiles::reduce_profiles(&mut state.profiles, op);
        }
        if crate::about::is_about_op(op.op_id) {
            return crate::about::reduce_about(&mut state.about, op);
        }
        if crate::questions::is_question_op(op.op_id) {
            return crate::questions::reduce_questions(&mut state.questions, &state.member_roles, op);
        }
        if matches!(op.op_id, OP_RSVP | OP_SET_REGISTRATION | OP_RSVP_DECIDE) {
            return crate::rsvp::reduce(state, op);
        }
        if crate::roles::is_role_op(op.op_id) {
            return crate::roles::reduce_roles(&mut state.member_roles, op);
        }
        if publication::is_publication_op(op.op_id) {
            return publication::reduce_publication(&mut state.publication, op);
        }
        // Every op here is owner/sequenced: the spine already rejected non-owner
        // writes at deliver time, so the reducer just parses + applies.
        let args = op.args;
        match op.op_id {
            OP_SET_PROFILE => {
                // Validate EVERY arg before the first mutation — a rejected op must
                // leave the state untouched (the reducer contract: reduce is atomic).
                let display_name = req_text(args, "displayName")?.to_string();
                let shape = GroupShape::parse(req_text(args, "shape")?)?;
                let card = match opt_text(args, "card") {
                    Some(json) => Some(
                        serde_json::from_str::<ContactCard>(&json)
                            .map_err(|_| DeltaRejection::MalformedArgs)?,
                    ),
                    None => None,
                };
                // Tolerant of FIELDS is not tolerant of SIZE: the card's avatar slots
                // are media and take the event.rs caps (one cap, one place) at THIS
                // fold too — without the gate a patched peer could publish a photo or
                // clip of any size that every member folds and stores. Oversize is
                // MalformedArgs, refused not clamped: silent re-encoding by the
                // engine would make two devices disagree about the same delta.
                if let Some(c) = &card {
                    if c.photo.len() > crate::event::MAX_PHOTO_B64
                        || c.clip.len() > crate::event::MAX_CLIP_B64
                    {
                        return Err(DeltaRejection::MalformedArgs);
                    }
                }
                state.display_name = display_name;
                state.shape = shape;
                if let Some(c) = card {
                    state.card = c;
                }
                Ok(())
            }
            OP_SET_PRESENCE => {
                state.presence = match req_text(args, "kind")? {
                    "onPlatform" => {
                        // Honest presence: a real 32-byte IdentityKey, NEVER a fabricated
                        // token. This is the single reducer gate — every path (import,
                        // CLI, FFI) folds through it, so an unparseable space can never
                        // land an on-platform (can_sync=true) record.
                        let space = req_text(args, "identityKey")?;
                        parse_identity_key(space).map_err(|_| DeltaRejection::MalformedArgs)?;
                        Presence::OnPlatform(space.to_string())
                    }
                    "offPlatform" => Presence::OffPlatform(opt_text(args, "inviteHint")),
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                Ok(())
            }
            OP_STORE_CREDENTIAL => {
                let id = req_text(args, "id")?.to_string();
                let kind = CredentialKind::parse(req_text(args, "kind")?)?;
                let label = req_text(args, "label")?.to_string();
                state.credentials.insert(id, CredentialMeta { kind, label });
                Ok(())
            }
            OP_REVOKE_CREDENTIAL => {
                let id = req_text(args, "id")?;
                state
                    .credentials
                    .remove(id)
                    .ok_or(DeltaRejection::PreconditionFailed)?;
                Ok(())
            }
            OP_SET_AFFILIATION => {
                // `peer` is the OTHER group's object id (hex). Validate every arg
                // before the first mutation — reduce is atomic. Re-setting an
                // existing peer updates the edge (rename, re-tether, rel change).
                let peer = req_hex_id(args, "peer")?;
                let rel = AffiliationRel::parse(req_text(args, "rel")?)?;
                let name = req_text(args, "name")?.to_string();
                let tether = opt_text(args, "tether").unwrap_or_default();
                let at = req_int(args, "at")?;
                state.affiliations.insert(
                    peer,
                    Affiliation {
                        rel,
                        name,
                        tether,
                        at,
                    },
                );
                Ok(())
            }
            OP_CLEAR_AFFILIATION => {
                let peer = req_hex_id(args, "peer")?;
                // Clearing an edge that was never recorded is a real precondition
                // failure, not a no-op (mirrors revokeCredential).
                state
                    .affiliations
                    .remove(&peer)
                    .ok_or(DeltaRejection::PreconditionFailed)?;
                Ok(())
            }
            OP_JOINED_OBJECT => {
                // Validate every arg before the first mutation — reduce is atomic.
                let object = req_hex_id(args, "object")?;
                let kind = req_text(args, "kind")?.to_string();
                let arc = req_text(args, "arc")?.to_string();
                let tag = req_hex_id(args, "tag")?;
                let at = req_int(args, "at")?;
                // Re-joining CLEARS the departure rather than stacking a second
                // record: the spine is sequenced, so the latest statement wins and
                // a stale `left_at` would make a live membership look ended.
                state.joined.insert(
                    object,
                    Joined { kind, arc, tag, at, left_at: None, reason: String::new() },
                );
                Ok(())
            }
            OP_LEFT_OBJECT => {
                let object = req_hex_id(args, "object")?;
                let at = req_int(args, "at")?;
                let reason = opt_text(args, "reason").unwrap_or_default().to_string();
                // Leaving something you were never in fails loudly, mirroring
                // clearPart and clearAffiliation. The record is KEPT and dated —
                // "was I ever in X" stays answerable, and a removal here would
                // erase the only evidence the tenure happened.
                let rec = state
                    .joined
                    .get_mut(&object)
                    .ok_or(DeltaRejection::PreconditionFailed)?;
                rec.left_at = Some(at);
                rec.reason = reason;
                Ok(())
            }
            OP_SET_OFFICE => {
                // Validate every arg before the first mutation — reduce is atomic.
                let office = req_text(args, "office")?.trim().to_ascii_lowercase();
                if office.is_empty() {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let holder_hex = req_hex_id(args, "holder")?;
                let holder: MemberId = hex::decode(&holder_hex)
                    .ok()
                    .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
                    .ok_or(DeltaRejection::MalformedArgs)?;
                // An office is an overlay on a real member, exactly like a role: it
                // cannot confer membership, and appointing someone who is not in the
                // room would be an assertion the roster does not back.
                if !op.ctx.is_member(&holder) {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                let status = OfficeStatus::parse(req_text(args, "status")?)?;
                let at = req_int(args, "at")?;
                if at <= 0 {
                    return Err(DeltaRejection::MalformedArgs);
                }
                state
                    .offices
                    .insert(office, OfficeHolder { holder, status, at });
                Ok(())
            }
            OP_CLEAR_OFFICE => {
                let office = req_text(args, "office")?.trim().to_ascii_lowercase();
                // Vacating an office nobody holds fails loudly, mirroring clearPart
                // and clearAffiliation.
                state
                    .offices
                    .remove(&office)
                    .ok_or(DeltaRejection::PreconditionFailed)?;
                Ok(())
            }
            OP_SET_CLAIM_ISSUER => {
                use base64::Engine as _;
                let kid = req_text(args, "kid")?;
                let key = req_text(args, "key")?;
                let bytes: [u8; 32] = base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(key)
                    .ok()
                    .and_then(|b| b.try_into().ok())
                    .ok_or(DeltaRejection::MalformedArgs)?;
                if crate::claim::kid_of(&bytes) != kid {
                    return Err(DeltaRejection::MalformedArgs);
                }
                state.claim_issuers.insert(kid.to_string(), key.to_string());
                Ok(())
            }
            OP_CLEAR_CLAIM_ISSUER => {
                let kid = req_text(args, "kid")?;
                state.claim_issuers.remove(kid).ok_or(DeltaRejection::PreconditionFailed)?;
                Ok(())
            }
            OP_SET_FACE => {
                state.face = face_of(state, args)?;
                Ok(())
            }
            OP_PUBLISH_LISTING | OP_REMOVE_LISTING => reduce_listing(state, op),
            OP_EDIT_FACE => {
                // THE SHARED FACE (ICD 2.1.0 row 10): one LWW register by (gen, author),
                // folded after the spine in (gen, author, id) order, so the last write that
                // counts is the face, and with none counting the sequenced one stands. It
                // counts from the owner, or from a member on the roster holding admin in
                // THIS group's roles. The roles are the folded ones, so a revoked admin's
                // writes stop counting: the answer to a hostile max-gen write (R1). `gen` is
                // declared required, and refused absent, as forum.retract's.
                if !matches!(crate::arg_reads::get(args, "gen"), Some(crate::coordinator::ArgVal::Int(_))) {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let author = op.author;
                let admin = op.ctx.is_member(author) && state.member_roles.get(author) == Some(&GroupRole::Admin);
                if *author != op.ctx.owner && !admin {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                state.face = face_of(state, args)?;
                Ok(())
            }
            OP_SET_COVER => {
                // ONE slot, wholesale replaced. Empty data clears; otherwise the mime
                // is a closed vocabulary and the cap follows it (motion gets the clip
                // budget). Validate before the first mutation — reduce is atomic.
                let data = opt_text(args, "data").unwrap_or_default();
                if data.is_empty() {
                    state.cover = String::new();
                    state.cover_mime = String::new();
                    return Ok(());
                }
                let mime = req_text(args, "mime")?;
                let cap = match mime {
                    "image/jpeg" | "image/png" | "image/gif" => MAX_COVER_STILL_B64,
                    "video/mp4" => MAX_COVER_CLIP_B64,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                if data.len() > cap {
                    return Err(DeltaRejection::MalformedArgs);
                }
                state.cover_mime = mime.to_string();
                state.cover = data;
                Ok(())
            }
            _ => Err(DeltaRejection::UnknownType),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator::{sequenced_delta, ArgVal, Args, Coordinator, GENESIS_PREV};
    use crate::object::ReduceContext;

    const OWNER: MemberId = [7u8; 32];
    const BOB: MemberId = [0xB0u8; 32];
    const STRANGER: MemberId = [9u8; 32];

    fn tid() -> u32 {
        ObjectKind::Group.type_id() as u32
    }

    /// A removal the owner recorded survives that owner handing over (§10.2 of
    /// membership-through-mls.md): the fold judges each record by the owner of ITS
    /// epoch. Judged by today's owner, the removal would stop folding, and the log would
    /// report someone the tree no longer holds as present — drift, after every handover.
    #[test]
    fn a_removal_the_former_owner_recorded_survives_the_handover() {
        let rec = |op_id: u32, member: MemberId, at: i64, epoch: u64, gen: u64| {
            let mut a = Args::new();
            a.insert("member".into(), ArgVal::Text(hex::encode(member)));
            a.insert("at".into(), ArgVal::Int(at));
            crate::object::build_delta(ObjectKind::Group, op_id, a, epoch, Some(gen))
        };
        let mut c = Coordinator::<GroupType>::with_owners(vec![OWNER, BOB], vec![(0, OWNER), (2, BOB)]);
        c.deliver(rec(membership::OP_MEMBER_JOINED, STRANGER, 1_000, 0, 0), OWNER)
            .unwrap();
        c.deliver(rec(membership::OP_MEMBER_LEFT, STRANGER, 2_000, 1, 1), OWNER)
            .unwrap();
        let t = c
            .state()
            .membership
            .departed(&hex::encode(STRANGER))
            .expect("the removal OWNER recorded at epoch 1 still folds once BOB owns it");
        assert_eq!(t.departure, membership::Departure::Removed);
    }

    /// ICD 2.1.0 row 2: a spend's `choice` and `share` are held to the claim's own rule,
    /// so a patched admitter cannot record what no kiosk issues.
    #[test]
    fn a_spent_claims_choice_and_share_are_the_claims_own() {
        let spend = |n: u8, extra: &[(&str, &str)]| {
            let mut a = Args::new();
            a.insert("claim".into(), ArgVal::Text(hex::encode([n; 32])));
            a.insert("member".into(), ArgVal::Text(hex::encode(STRANGER)));
            a.insert("at".into(), ArgVal::Int(1_000));
            for (k, v) in extra {
                a.insert(k.to_string(), ArgVal::Text(v.to_string()));
            }
            crate::object::build_delta(ObjectKind::Group, membership::OP_CLAIM_SPENT, a, 0, Some(n as u64))
        };
        let mut c = Coordinator::<GroupType>::new(vec![OWNER], OWNER);
        let bad = [(1, ("choice", "time")), (2, ("share", "abcdefghjkmnpqrstuv1")), (3, ("share", "abc"))];
        for (n, arg) in bad {
            c.deliver(spend(n, &[arg]), OWNER).unwrap();
        }
        c.deliver(spend(4, &[("choice", "resources"), ("share", "abcdefghjkmnpqrstuvw")]), OWNER).unwrap();
        let st = c.state();
        for (n, arg) in bad {
            assert!(!st.claims_spent.contains_key(&hex::encode([n; 32])), "{arg:?} does not fold");
        }
        assert_eq!(st.claims_picked.len(), 1);
        let s = &st.claims_picked[&hex::encode([4u8; 32])];
        assert_eq!((s.choice.as_deref(), s.share.as_deref()), (Some("resources"), Some("abcdefghjkmnpqrstuvw")));
    }

    /// Drift guard for the publication splice — the same pin `place.rs` puts on
    /// its geo and visibility splices. GROUP_OPS writes the base ops out verbatim
    /// (const-initialiser), so this is what stops the two copies diverging.
    #[test]
    fn base_publication_ops_are_spliced_verbatim() {
        for want in publication::PUBLICATION_OPS {
            let got = GroupType::op(want.op_id)
                .unwrap_or_else(|| panic!("Group is missing base op {:#x}", want.op_id));
            assert_eq!(got.name, want.name);
            assert_eq!(got.authority, want.authority);
            assert_eq!(got.commutativity, want.commutativity);
        }
    }

    /// The wallet band, likewise — and it is the splice most worth pinning,
    /// because the ICD conformance test reads Group's table rather than
    /// `WALLET_OPS`, so a copy that drifted here would be documented wrong in
    /// both places at once and still pass.
    /// The note band, likewise. Same reason as the wallet pin: the ICD
    /// conformance test reads Group's table rather than `NOTE_OPS`, so a copy that
    /// drifted here would be documented wrong in both places at once and pass.
    /// NEITHER note kind is Group-typed any more. They were, because the note facet
    /// was spliced into `GROUP_OPS` and a note had to fold with the Group vocabulary
    /// to be authored at all. A Note is its own kind now (32), so borrowing stops —
    /// and `GROUP_TYPED_KINDS` is down to `group` alone, which is what it should
    /// always have said. `fold::lens_for` still reads BOTH strings, so objects
    /// stored under either still fold and stay in the archive.
    #[test]
    fn neither_note_kind_borrows_the_group_vocabulary() {
        assert!(!is_group_typed("notebook"));
        assert!(!is_group_typed("note"));
        assert_eq!(GROUP_TYPED_KINDS.len(), 1, "only `group` is Group-typed");
        for k in ["note", "notebook"] {
            assert_eq!(crate::fold::lens_for(k), Some(crate::object::ObjectKind::Note),
                "`{k}` must still fold, or every object stored under it leaves the archive");
        }
    }

    /// The facet routes through the Group reducer and survives a real fold, so a
    /// site's address is ordinary folded state — not a side channel.
    #[test]
    fn a_group_publishes_and_unpublishes_through_the_fold() {
        let mut c: Coordinator<GroupType> = Coordinator::new(vec![OWNER, BOB], OWNER);
        let mut sp = Spine::new();
        assert!(!c.state().publication.is_published());

        sp.deliver(
            &mut c,
            publication::OP_PUBLISH,
            publication::publish_args("cambridge-dd", &BOB),
        );
        let st = c.state();
        assert!(st.publication.is_published());
        assert_eq!(st.publication.slug, "cambridge-dd");
        assert_eq!(st.publication.publisher, Some(BOB));

        sp.deliver(&mut c, publication::OP_UNPUBLISH, publication::unpublish_args());
        assert!(!c.state().publication.is_published());
    }
    fn t(s: &str) -> ArgVal {
        ArgVal::Text(s.to_string())
    }
    fn args(pairs: Vec<(&str, ArgVal)>) -> Args {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }
    fn space(m: MemberId) -> String {
        format!("{}{}", crate::identity::IDENTITY_KEY_PREFIX, hex::encode(m))
    }

    /// Chains owner/sequenced group deltas with proper (seq, prev) links.
    struct Spine {
        seq: u64,
        prev: [u8; 32],
    }
    impl Spine {
        fn new() -> Self {
            Self {
                seq: 0,
                prev: GENESIS_PREV,
            }
        }
        fn deliver(&mut self, c: &mut Coordinator<GroupType>, op_id: u32, a: Args) {
            let d = sequenced_delta(tid(), op_id, a, 0, self.seq, self.prev);
            let id = d.id();
            c.deliver(d, OWNER).unwrap();
            self.seq += 1;
            self.prev = id;
        }
    }

    /// THE SPINE IS THE ENTRY POINT, and this is the fold that makes it one.
    ///
    /// A person's identity record enumerates the objects they belong to, so a
    /// device holding nothing but the words folds this and reaches the rest of the
    /// graph. Nothing else in the tree survives that trip: the `groups` table is
    /// device-local, and `kind`, `arc` and the way-in tag otherwise ride only in a
    /// sealed Welcome a returning device never gets.
    #[test]
    fn the_spine_folds_its_own_memberships() {
        let mut c: Coordinator<GroupType> = Coordinator::new(vec![OWNER, BOB], OWNER);
        let mut sp = Spine::new();
        let forum = "a".repeat(64);
        let place = "b".repeat(64);
        let tag = "c".repeat(64);

        assert!(c.state().joined.is_empty(), "a fresh record belongs to nothing");

        sp.deliver(
            &mut c,
            OP_JOINED_OBJECT,
            args(vec![
                ("object", t(&forum)),
                ("kind", t("forum")),
                ("arc", t("wss://uk.arc.example")),
                ("tag", t(&tag)),
                ("at", ArgVal::Int(1_000)),
            ]),
        );
        sp.deliver(
            &mut c,
            OP_JOINED_OBJECT,
            args(vec![
                ("object", t(&place)),
                ("kind", t("place")),
                ("arc", t("wss://uk.arc.example")),
                ("tag", t(&tag)),
                ("at", ArgVal::Int(1_100)),
            ]),
        );

        let st = c.state();
        assert_eq!(st.joined.len(), 2, "both memberships fold");
        let f = &st.joined[&forum];
        assert_eq!((f.kind.as_str(), f.at, f.left_at), ("forum", 1_000, None));
        assert_eq!(f.arc, "wss://uk.arc.example", "the governing arc survives the trip");
        assert_eq!(f.tag, tag, "and so does the way back in");

        // Leaving KEEPS the record and dates it — "was I ever in X" stays answerable,
        // and removing it would erase the only evidence the tenure happened.
        sp.deliver(
            &mut c,
            OP_LEFT_OBJECT,
            args(vec![
                ("object", t(&place)),
                ("reason", t("it went quiet")),
                ("at", ArgVal::Int(2_000)),
            ]),
        );
        let st = c.state();
        assert_eq!(st.joined.len(), 2, "the record is kept, not removed");
        assert_eq!(st.joined[&place].left_at, Some(2_000));
        assert_eq!(st.joined[&place].reason, "it went quiet");
        assert_eq!(st.joined[&forum].left_at, None, "leaving one leaves the other alone");
        assert_eq!(
            st.joined.values().filter(|j| j.left_at.is_none()).count(),
            1,
            "reconstruction takes only the live ones"
        );

        // Re-joining CLEARS the departure rather than stacking a second record. The
        // spine is sequenced, so the latest statement wins; a stale `left_at` here
        // would make a live membership look ended and hide the object from a restore.
        sp.deliver(
            &mut c,
            OP_JOINED_OBJECT,
            args(vec![
                ("object", t(&place)),
                ("kind", t("place")),
                ("arc", t("wss://nl.arc.example")),
                ("tag", t(&tag)),
                ("at", ArgVal::Int(3_000)),
            ]),
        );
        let st = c.state();
        assert_eq!(st.joined[&place].left_at, None, "re-joining clears the departure");
        assert_eq!(st.joined[&place].at, 3_000, "and re-dates the tenure");
        assert_eq!(st.joined[&place].arc, "wss://nl.arc.example", "it may come back on another arc");
        assert!(st.joined[&place].reason.is_empty(), "the old reason does not linger");
    }

    /// Leaving something you were never in fails loudly, mirroring `clearPart` and
    /// `clearAffiliation`. Silence here would let a restore believe a membership was
    /// ended that was never recorded.
    #[test]
    fn leaving_an_object_you_never_joined_is_refused() {
        let members = vec![OWNER, BOB];
        let mut probe = GroupState::default();
        let ctx = crate::object::ReduceContext { members: &members, owner: OWNER, epoch: 0 };
        let a = args(vec![("object", t(&"f".repeat(64))), ("at", ArgVal::Int(1))]);
        assert_eq!(
            GroupType::reduce(
                &mut probe,
                &Op { op_id: OP_LEFT_OBJECT, args: &a, author: &OWNER, pos: None, ctx: &ctx },
            ),
            Err(DeltaRejection::PreconditionFailed)
        );
        assert!(probe.joined.is_empty(), "a refused leave mutates nothing");
    }

    #[test]
    fn all_ops_well_formed() {
        for d in GROUP_OPS {
            assert!(
                d.is_well_formed(),
                "op {} violates the spec invariant",
                d.name
            );
        }
    }

    #[test]
    fn vocab_round_trips() {
        for s in ["individual", "team", "organisation"] {
            assert_eq!(GroupShape::parse(s).unwrap().as_str(), s);
        }
        for r in ["owner", "admin", "member", "viewer", "guest"] {
            assert_eq!(GroupRole::parse(r).unwrap().as_str(), r);
        }
        for a in ["parent", "child", "peer"] {
            assert_eq!(AffiliationRel::parse(a).unwrap().as_str(), a);
        }
        assert!(GroupShape::parse("place").is_err()); // PLACE is not a Group
        assert!(AffiliationRel::parse("federation").is_err()); // rel vocabulary is closed
    }

    /// The mint capability, pinned against the AUTHORITY of the ops it depends on.
    ///
    /// This is the test that stops the gate and the fold drifting apart: if someone
    /// widens `may_mint` to Admin without also giving the reducer a role-aware authority,
    /// an admin gets a `+` that mints an object and then cannot attach it.
    #[test]
    fn may_mint_matches_the_authority_of_the_ops_it_needs() {
        assert!(GroupRole::Owner.may_mint());
        for r in [GroupRole::Admin, GroupRole::Member, GroupRole::Viewer, GroupRole::Guest] {
            assert!(!r.may_mint(), "{r:?} cannot author the belonging ops");
        }
        // The two ops that make a thing belong to a Space. Both Owner-only — which is
        // exactly why `may_mint` is Owner-only.
        for op in [crate::parts::OP_SET_PART, OP_SET_AFFILIATION] {
            let decl = GroupType::op(op).expect("declared");
            assert_eq!(decl.authority, Authority::Owner, "{} authority", decl.name);
        }
    }

    /// A team Group (owner + Bob) folds its profile/presence/card, a role overlaid
    /// on a REAL roster member, and drops a role aimed at a non-member — while the
    /// view surfaces roles only for roster members.
    #[test]
    fn folds_profile_presence_and_role_overlay() {
        let roster = vec![OWNER, BOB];
        let mut c = Coordinator::<GroupType>::new(roster.clone(), OWNER);
        let mut sp = Spine::new();

        sp.deliver(&mut c, OP_SET_PROFILE, args(vec![
            ("displayName", t("Aries")),
            ("shape", t("organisation")),
            ("card", t(r#"{"org":"Example Org","emails":[{"label":"work","value":"hi@example.com"}]}"#)),
        ]));
        sp.deliver(
            &mut c,
            OP_SET_PRESENCE,
            args(vec![
                ("kind", t("offPlatform")),
                ("inviteHint", t("hi@example.com")),
            ]),
        );
        sp.deliver(
            &mut c,
            crate::roles::OP_SET_ROLE,
            args(vec![("member", t(&space(BOB))), ("role", t("admin"))]),
        );
        // A role aimed at a NON-member: chains fine, but the reducer rejects it at
        // fold (is_member=false), so it never lands.
        sp.deliver(
            &mut c,
            crate::roles::OP_SET_ROLE,
            args(vec![("member", t(&space(STRANGER))), ("role", t("member"))]),
        );

        let st = c.state();
        assert_eq!(st.display_name, "Aries");
        assert_eq!(st.shape, GroupShape::Organisation);
        assert_eq!(st.card.org, "Example Org");
        assert_eq!(st.card.emails[0].value, "hi@example.com");
        assert_eq!(
            st.presence,
            Presence::OffPlatform(Some("hi@example.com".into()))
        );
        assert_eq!(st.member_roles.get(&BOB), Some(&GroupRole::Admin));
        assert!(
            !st.member_roles.contains_key(&STRANGER),
            "role on a non-member dropped"
        );

        let v = st.view(&roster);
        assert_eq!(v.roles, vec![(BOB, GroupRole::Admin)]);
        assert!(!v.can_sync); // off-platform → not reachable
    }

    /// Honest presence: a real 32-byte IdentityKey lands on-platform (can_sync=true);
    /// a garbage anchor is rejected at fold and never fabricates reachability.
    #[test]
    fn presence_on_platform_requires_a_real_space_id() {
        let real = format!("{}{}", crate::identity::IDENTITY_KEY_PREFIX, hex::encode(BOB));
        let mut c = Coordinator::<GroupType>::new(vec![OWNER], OWNER);
        let mut sp = Spine::new();
        sp.deliver(
            &mut c,
            OP_SET_PROFILE,
            args(vec![("displayName", t("Aries")), ("shape", t("team"))]),
        );
        sp.deliver(
            &mut c,
            OP_SET_PRESENCE,
            args(vec![("kind", t("onPlatform")), ("identityKey", t(&real))]),
        );
        assert_eq!(c.state().presence, Presence::OnPlatform(real.clone()));
        assert!(c.state().view(&[OWNER]).can_sync);

        // a fabricated anchor is dropped at fold — presence stays the last good value.
        sp.deliver(
            &mut c,
            OP_SET_PRESENCE,
            args(vec![("kind", t("onPlatform")), ("identityKey", t("hello"))]),
        );
        assert_eq!(
            c.state().presence,
            Presence::OnPlatform(real),
            "garbage space rejected, no fabrication"
        );
    }

    /// The Berkeley shape: a chapter records its federation as `parent`, updates the
    /// edge in place, and clears it — while a garbage peer id or an unknown rel is
    /// The cover: one slot, wholesale replaced — gif and clip take their own caps,
    /// oversize and alien mimes are refused whole, and empty data clears.
    #[test]
    fn a_cover_folds_within_caps_and_clears() {
        let mut c = Coordinator::<GroupType>::new(vec![OWNER], OWNER);
        let mut sp = Spine::new();

        sp.deliver(
            &mut c,
            OP_SET_COVER,
            args(vec![("data", t("aGVsbG8=")), ("mime", t("image/gif"))]),
        );
        let st = c.state();
        assert_eq!(st.cover, "aGVsbG8=");
        assert_eq!(st.cover_mime, "image/gif");

        // A rejected replacement leaves the folded cover untouched.
        let ctx = ReduceContext {
            members: &[OWNER],
            owner: OWNER,
            epoch: 0,
        };
        let mut probe = c.state();
        for bad in [
            args(vec![("data", t("eA==")), ("mime", t("image/tiff"))]), // alien mime
            args(vec![
                ("data", ArgVal::Text("x".repeat(MAX_COVER_STILL_B64 + 1))),
                ("mime", t("image/jpeg")),
            ]),
        ] {
            let op = Op {
                op_id: OP_SET_COVER,
                args: &bad,
                author: &OWNER,
                pos: None,
                ctx: &ctx,
            };
            assert_eq!(
                GroupType::reduce(&mut probe, &op),
                Err(DeltaRejection::MalformedArgs)
            );
            assert_eq!(
                probe.cover_mime, "image/gif",
                "a refused cover must not half-apply"
            );
        }

        // Video rides the clip budget, and empty data clears the slot.
        sp.deliver(
            &mut c,
            OP_SET_COVER,
            args(vec![("data", t("bXA0")), ("mime", t("video/mp4"))]),
        );
        assert_eq!(c.state().cover_mime, "video/mp4");
        sp.deliver(
            &mut c,
            OP_SET_COVER,
            args(vec![("data", t("")), ("mime", t(""))]),
        );
        let st = c.state();
        assert!(st.cover.is_empty() && st.cover_mime.is_empty());
    }

    /// The profile card's avatar slots are fold-gated like every other media slot:
    /// an oversized photo or clip is refused whole (leaving the previously folded
    /// profile intact), while an at-cap card is a legal card, not a casualty.
    #[test]
    fn a_profile_card_avatar_is_capped_at_fold() {
        let mut c = Coordinator::<GroupType>::new(vec![OWNER], OWNER);
        let mut sp = Spine::new();

        // A good card lands first — the state the refusals below must not corrupt.
        sp.deliver(
            &mut c,
            OP_SET_PROFILE,
            args(vec![
                ("displayName", t("Aries")),
                ("shape", t("team")),
                ("card", t(r#"{"photo":"c3RpbGw=","photo_mime":"image/jpeg"}"#)),
            ]),
        );
        assert_eq!(c.state().card.photo, "c3RpbGw=");

        let ctx = ReduceContext {
            members: &[OWNER],
            owner: OWNER,
            epoch: 0,
        };
        let mut probe = c.state();
        for bad in [
            format!(
                r#"{{"photo":"{}","photo_mime":"image/jpeg"}}"#,
                "x".repeat(crate::event::MAX_PHOTO_B64 + 1)
            ),
            format!(
                r#"{{"clip":"{}","clip_mime":"video/mp4"}}"#,
                "x".repeat(crate::event::MAX_CLIP_B64 + 1)
            ),
        ] {
            let a = args(vec![
                ("displayName", t("Clobber")),
                ("shape", t("team")),
                ("card", t(&bad)),
            ]);
            let op = Op {
                op_id: OP_SET_PROFILE,
                args: &a,
                author: &OWNER,
                pos: None,
                ctx: &ctx,
            };
            assert_eq!(
                GroupType::reduce(&mut probe, &op),
                Err(DeltaRejection::MalformedArgs)
            );
            assert_eq!(
                probe.card.photo, "c3RpbGw=",
                "a refused card must not half-apply"
            );
            assert_eq!(
                probe.display_name, "Aries",
                "a refused profile must not half-apply"
            );
        }

        // Exactly at the caps: folds fine.
        let at_cap = format!(
            r#"{{"photo":"{}","photo_mime":"image/jpeg","clip":"{}","clip_mime":"video/mp4"}}"#,
            "x".repeat(crate::event::MAX_PHOTO_B64),
            "y".repeat(crate::event::MAX_CLIP_B64),
        );
        sp.deliver(
            &mut c,
            OP_SET_PROFILE,
            args(vec![
                ("displayName", t("Aries")),
                ("shape", t("team")),
                ("card", t(&at_cap)),
            ]),
        );
        let st = c.state();
        assert_eq!(st.card.photo.len(), crate::event::MAX_PHOTO_B64);
        assert_eq!(st.card.clip.len(), crate::event::MAX_CLIP_B64);
    }

    /// The `created` rel folds like any affiliation — a group publishing "this Event /
    /// this listing is ours" — and carries no join-service duty (that is Anchored's).
    #[test]
    fn a_created_edge_folds_and_answers_for_nobody() {
        let listing = hex::encode([0xC0u8; 32]);
        let mut c = Coordinator::<GroupType>::new(vec![OWNER], OWNER);
        let mut sp = Spine::new();
        sp.deliver(
            &mut c,
            OP_SET_AFFILIATION,
            args(vec![
                ("peer", t(&listing)),
                ("rel", t("created")),
                ("name", t("Launch party")),
                ("at", ArgVal::Int(1_750_000_000_000)),
            ]),
        );
        let st = c.state();
        let a = st.affiliations.get(&listing).expect("edge folded");
        assert_eq!(a.rel, AffiliationRel::Created);
        assert_eq!(a.name, "Launch party");
        assert!(
            !a.rel.answers_for_peer(),
            "created is authorship, not a door duty"
        );
    }

    /// rejected atomically and a clear of a never-recorded edge fails loudly.
    #[test]
    fn folds_affiliation_set_update_and_clear() {
        let fed = hex::encode([0xFEu8; 32]); // the federation's object id
        let mut c = Coordinator::<GroupType>::new(vec![OWNER], OWNER);
        let mut sp = Spine::new();

        sp.deliver(
            &mut c,
            OP_SET_AFFILIATION,
            args(vec![
                ("peer", t(&fed)),
                ("rel", t("parent")),
                ("name", t("National Student Federation")),
                ("tether", t("aa11")),
                ("at", ArgVal::Int(1_700_000_000_000)),
            ]),
        );
        let st = c.state();
        let a = st.affiliations.get(&fed).expect("edge folded");
        assert_eq!(a.rel, AffiliationRel::Parent);
        assert_eq!(a.name, "National Student Federation");
        assert_eq!(a.tether, "aa11");
        assert_eq!(a.at, 1_700_000_000_000);
        assert_eq!(st.view(&[OWNER]).affiliations.len(), 1);

        // Re-setting the SAME peer updates the edge in place (rename + re-rel).
        sp.deliver(
            &mut c,
            OP_SET_AFFILIATION,
            args(vec![
                ("peer", t(&fed)),
                ("rel", t("peer")),
                ("name", t("NSF")),
                ("at", ArgVal::Int(1_700_000_000_001)),
            ]),
        );
        let st = c.state();
        let a = st.affiliations.get(&fed).unwrap();
        assert_eq!(a.rel, AffiliationRel::Peer);
        assert_eq!(a.name, "NSF");
        assert_eq!(
            a.tether, "",
            "omitted tether folds as none, not the stale value"
        );
        assert_eq!(
            st.affiliations.len(),
            1,
            "same peer = update, never a second edge"
        );

        // A fabricated (non-hex) peer id never lands; an unknown rel is atomic-rejected.
        sp.deliver(
            &mut c,
            OP_SET_AFFILIATION,
            args(vec![
                ("peer", t("not-hex!")),
                ("rel", t("parent")),
                ("name", t("Ghost")),
                ("at", ArgVal::Int(1)),
            ]),
        );
        sp.deliver(
            &mut c,
            OP_SET_AFFILIATION,
            args(vec![
                ("peer", t(&fed)),
                ("rel", t("overlord")),
                ("name", t("Clobber")),
                ("at", ArgVal::Int(2)),
            ]),
        );
        let st = c.state();
        assert_eq!(st.affiliations.len(), 1);
        assert_eq!(
            st.affiliations.get(&fed).unwrap().name,
            "NSF",
            "bad rel dropped whole"
        );

        // Clear removes the edge; clearing again is a real precondition failure (inert).
        sp.deliver(&mut c, OP_CLEAR_AFFILIATION, args(vec![("peer", t(&fed))]));
        assert!(c.state().affiliations.is_empty());
        sp.deliver(&mut c, OP_CLEAR_AFFILIATION, args(vec![("peer", t(&fed))]));
        assert!(
            c.state().affiliations.is_empty(),
            "double-clear stays inert"
        );
    }

    /// The part set folds like a keyed map: attach, re-role in place, detach — with a
    /// garbage part id rejected whole and a detach of a never-attached part loud.
    #[test]
    fn folds_part_set_update_and_clear() {
        let room = hex::encode([0xF0u8; 32]); // the part's object id
        let mut c = Coordinator::<GroupType>::new(vec![OWNER], OWNER);
        let mut sp = Spine::new();

        sp.deliver(
            &mut c,
            crate::parts::OP_SET_PART,
            args(vec![
                ("part", t(&room)),
                ("role", t("room")),
                ("at", ArgVal::Int(1_700_000_000_000)),
            ]),
        );
        let st = c.state();
        let f = st.parts.get(&room).expect("edge folded");
        assert_eq!(f.role, "room");
        assert_eq!(f.at, 1_700_000_000_000);
        assert_eq!(st.view(&[OWNER]).parts.len(), 1);

        // Re-setting the SAME part changes its role in place, never mints a second edge.
        sp.deliver(
            &mut c,
            crate::parts::OP_SET_PART,
            args(vec![
                ("part", t(&room)),
                ("role", t("treasury")),
                ("at", ArgVal::Int(1_700_000_000_001)),
            ]),
        );
        let st = c.state();
        assert_eq!(
            st.parts.len(),
            1,
            "same part = update, never a second edge"
        );
        assert_eq!(st.parts.get(&room).unwrap().role, "treasury");

        // A fabricated (non-hex) part id never lands; a missing role is atomic-rejected.
        sp.deliver(
            &mut c,
            crate::parts::OP_SET_PART,
            args(vec![
                ("part", t("not-hex!")),
                ("role", t("room")),
                ("at", ArgVal::Int(1)),
            ]),
        );
        sp.deliver(
            &mut c,
            crate::parts::OP_SET_PART,
            args(vec![("part", t(&room)), ("at", ArgVal::Int(2))]),
        );
        let st = c.state();
        assert_eq!(st.parts.len(), 1);
        assert_eq!(
            st.parts.get(&room).unwrap().role,
            "treasury",
            "bad args dropped whole"
        );

        // Clear removes the edge; clearing again is a real precondition failure (inert).
        sp.deliver(&mut c, crate::parts::OP_CLEAR_PART, args(vec![("part", t(&room))]));
        assert!(c.state().parts.is_empty());
        sp.deliver(&mut c, crate::parts::OP_CLEAR_PART, args(vec![("part", t(&room))]));
        assert!(c.state().parts.is_empty(), "double-clear stays inert");
    }

    /// A rejected setProfile (bad shape) must not partially mutate — reduce is atomic.
    #[test]
    fn set_profile_is_atomic_on_a_bad_shape() {
        let mut c = Coordinator::<GroupType>::new(vec![OWNER], OWNER);
        let mut sp = Spine::new();
        sp.deliver(
            &mut c,
            OP_SET_PROFILE,
            args(vec![("displayName", t("Good")), ("shape", t("team"))]),
        );
        // a second setProfile with a valid name but INVALID shape must be dropped whole.
        sp.deliver(
            &mut c,
            OP_SET_PROFILE,
            args(vec![("displayName", t("Clobber")), ("shape", t("planet"))]),
        );
        let st = c.state();
        assert_eq!(
            st.display_name, "Good",
            "the name from the rejected op never applied"
        );
        assert_eq!(st.shape, GroupShape::Team);
    }

    /// The splice must stay byte-identical to membership's table, or a Group would
    /// declare a base op with different authority than every other kind that adopts
    /// it later — and two kinds would disagree about who may record a departure.
    #[test]
    fn base_membership_ops_are_spliced_verbatim() {
        for want in membership::MEMBERSHIP_OPS.iter() {
            let got = GroupType::op(want.op_id)
                .unwrap_or_else(|| panic!("Group is missing base op {:#x}", want.op_id));
            assert_eq!(got.name, want.name);
            assert_eq!(got.authority, want.authority);
            assert_eq!(got.commutativity, want.commutativity);
        }
    }

    /// The ruling this op-group exists to enforce, through GroupType::reduce so it
    /// exercises the routing the splice added: a member walks out unaided, and the
    /// record reconciles with the ratchet tree instead of replacing it.
    #[test]
    fn a_member_can_leave_a_group_without_the_owner() {
        let alice = [0xAAu8; 32];
        let hex_alice = hex::encode(alice);
        let mut st = GroupState::default();

        fn fold(
            st: &mut GroupState,
            author: MemberId,
            member: MemberId,
            op_id: u32,
            at: i64,
        ) -> Result<(), DeltaRejection> {
            let members = [OWNER, member];
            let ctx = crate::object::ReduceContext {
                members: &members,
                owner: OWNER,
                epoch: 0,
            };
            let a = args(vec![
                ("member", t(&hex::encode(member))),
                ("at", ArgVal::Int(at)),
            ]);
            GroupType::reduce(
                st,
                &Op {
                    op_id,
                    args: &a,
                    author: &author,
                    pos: None,
                    ctx: &ctx,
                },
            )
        }

        fold(&mut st, OWNER, alice, membership::OP_MEMBER_JOINED, 1_000).unwrap();
        assert_eq!(
            st.membership.divergence(&[alice]),
            None,
            "log matches the tree"
        );

        // Alice authors her OWN departure — no owner involvement anywhere.
        fold(&mut st, alice, alice, membership::OP_MEMBER_LEFT, 2_000)
            .expect("leaving must not require the owner");

        let tenures = st.membership.tenures(&hex_alice);
        assert_eq!(tenures.len(), 1);
        assert_eq!(
            tenures[0].left_at, 2_000,
            "the interval closed, it did not vanish"
        );
        assert_eq!(tenures[0].departure, membership::Departure::Left);

        // Still in the ratchet tree until someone commits the Remove — expected,
        // because MLS forbids committing your own removal.
        let d = st.membership.divergence(&[alice]).expect("in flight");
        assert!(!d.is_drift());
        assert_eq!(
            st.membership.divergence(&[]),
            None,
            "settled once committed"
        );
    }
    /// An office is an overlay on a real member — exactly like a role. It cannot
    /// confer membership, which is the invariant the whole Group model rests on.
    #[test]
    fn an_office_cannot_be_given_to_a_non_member() {
        let mut c = Coordinator::<GroupType>::new(vec![OWNER, BOB], OWNER);
        let mut spine = Spine::new();
        spine.deliver(
            &mut c,
            OP_SET_OFFICE,
            args(vec![
                ("office", t("treasurer")),
                ("holder", t(&hex::encode(BOB))),
                ("status", t("accepted")),
                ("at", ArgVal::Int(1_000)),
            ]),
        );
        assert!(
            c.state().offices.contains_key("treasurer"),
            "a member can hold one"
        );

        spine.deliver(
            &mut c,
            OP_SET_OFFICE,
            args(vec![
                ("office", t("chair")),
                ("holder", t(&hex::encode(STRANGER))),
                ("status", t("accepted")),
                ("at", ArgVal::Int(1_000)),
            ]),
        );
        assert!(
            !c.state().offices.contains_key("chair"),
            "appointing an outsider is an assertion the roster does not back"
        );
    }

    /// A vacant office is the ABSENCE of an entry, so it can never disagree with a
    /// holder — there is no "vacant" value to leave behind.
    #[test]
    fn clearing_an_office_removes_it_rather_than_marking_it_vacant() {
        let mut c = Coordinator::<GroupType>::new(vec![OWNER, BOB], OWNER);
        let mut spine = Spine::new();
        spine.deliver(
            &mut c,
            OP_SET_OFFICE,
            args(vec![
                ("office", t("chair")),
                ("holder", t(&hex::encode(BOB))),
                ("status", t("pending")),
                ("at", ArgVal::Int(1_000)),
            ]),
        );
        assert_eq!(c.state().offices["chair"].status, OfficeStatus::Pending);

        spine.deliver(&mut c, OP_CLEAR_OFFICE, args(vec![("office", t("chair"))]));
        assert!(
            c.state().offices.is_empty(),
            "vacant is absence, not a value"
        );
    }

    /// Re-appointing replaces in place — a body swaps its treasurer without first
    /// vacating the office.
    #[test]
    fn re_appointing_replaces_the_holder() {
        let mut c = Coordinator::<GroupType>::new(vec![OWNER, BOB], OWNER);
        let mut spine = Spine::new();
        for (who, at) in [(OWNER, 1_000), (BOB, 2_000)] {
            spine.deliver(
                &mut c,
                OP_SET_OFFICE,
                args(vec![
                    ("office", t("secretary")),
                    ("holder", t(&hex::encode(who))),
                    ("status", t("accepted")),
                    ("at", ArgVal::Int(at)),
                ]),
            );
        }
        assert_eq!(c.state().offices.len(), 1);
        assert_eq!(c.state().offices["secretary"].holder, BOB);
        assert_eq!(c.state().offices["secretary"].at, 2_000);
    }

    #[test]
    fn office_names_are_free_text_but_normalised() {
        let mut c = Coordinator::<GroupType>::new(vec![OWNER, BOB], OWNER);
        let mut spine = Spine::new();
        // A co-op invents its own office; core must not need a release for it.
        spine.deliver(
            &mut c,
            OP_SET_OFFICE,
            args(vec![
                ("office", t("  Membership Steward  ")),
                ("holder", t(&hex::encode(BOB))),
                ("status", t("accepted")),
                ("at", ArgVal::Int(1_000)),
            ]),
        );
        assert!(
            c.state().offices.contains_key("membership steward"),
            "trimmed + lowercased, so the same office is one key"
        );
    }

    #[test]
    fn a_vacant_status_on_the_wire_is_malformed() {
        let mut c = Coordinator::<GroupType>::new(vec![OWNER, BOB], OWNER);
        let mut spine = Spine::new();
        spine.deliver(
            &mut c,
            OP_SET_OFFICE,
            args(vec![
                ("office", t("chair")),
                ("holder", t(&hex::encode(BOB))),
                ("status", t("vacant")),
                ("at", ArgVal::Int(1_000)),
            ]),
        );
        assert!(
            c.state().offices.is_empty(),
            "vacant has no representation — absence is the state"
        );
    }

    /// Community is a real shape now, not a UserDefaults refinement of Organisation.
    #[test]
    fn community_is_a_shape_that_round_trips() {
        assert_eq!(
            GroupShape::parse("community").unwrap(),
            GroupShape::Community
        );
        assert_eq!(GroupShape::Community.as_str(), "community");
        let mut c = Coordinator::<GroupType>::new(vec![OWNER], OWNER);
        let mut spine = Spine::new();
        spine.deliver(
            &mut c,
            OP_SET_PROFILE,
            args(vec![
                ("displayName", t("Allotment Society")),
                ("shape", t("community")),
            ]),
        );
        assert_eq!(c.state().shape, GroupShape::Community);
    }
}
