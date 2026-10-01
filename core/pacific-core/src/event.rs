//! `Event` — an occurrence with a when, and the object TICKETING hangs on.
//!
//! An Event is a GroupObject (kind 29) so it can be SHARED and STAFFED: the organizer
//! owns it, door staff join it as members, and its append-only log is the sale ledger.
//! Buyers deliberately NEVER join — membership == access, and a ticket holder has no
//! business reading the roster or the other buyers. The two cross-party legs
//! (request in, delivery out) ride the buyer's pairwise Contact channel instead,
//! exactly as `contact.publishProfile`/`publishListing` do — their reducers live HERE
//! and their `OpDecl`s are spliced into `CONTACT_OPS` (the `geo.rs`→`place.rs`
//! base-op-group pattern).
//!
//! # Who may sell: the delegate, enforced at fold
//!
//! Paid fulfilment lives at the Arc, not the organizer's phone: `setTickets` names a
//! `delegate` pk (the box-office node), and the fold accepts a `recordSale` only from
//! an AUTHORIZED SELLER — the owner, or any delegate a listing revision ever named.
//! The check accumulates (a delegate rotation must not retroactively poison sales that
//! were valid when made) and runs at fold, the one gate every device runs — the
//! `publishListing` relay-invariant discipline, applied to money.
//!
//! `recordSale` is AnyMember/Commutative KEYED BY `pi` (the payment reference) under a
//! single-writer discipline: only the delegate authors sales, so the OR-set behaves as
//! a totally-ordered ledger in practice, while late-joining door staff still converge
//! from any delivery order. Capacity is the author-side duty; the fold FLAGS an
//! oversold ledger loudly rather than rejecting it — a paid buyer must never be
//! punished at the door for the seller's bug.
//!
//! # No backfill; re-emission instead
//!
//! A member joining at epoch N structurally cannot decrypt earlier traffic (MLS
//! forward secrecy — the pinned no-backfill doctrine). Late-joining door staff get the
//! ledger because its LEGITIMATE HOLDERS re-author current state at the new epoch:
//! `Node::event_reemit` re-emits the owner's listing and the seller's sales, all
//! idempotent under the `pi`/LWW keys, so old members skip and new members converge.
//! State re-emission by its holder, never decryption of old ciphertext.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::coordinator::{ArgVal, Args};
use crate::geo;
use crate::object::{
    Authority, Commutativity, DeltaRejection, MemberId, ObjectKind, ObjectType, Op, OpDecl,
};
// event.rs's optional-text lens skips empty strings — that is `opt_text_nonempty`.
use crate::object_args::{arg_hex32, opt_text_nonempty as opt_text, req_hex_id, req_int, req_text};
use crate::CoreError;

// ---- op ids (MUST match pacific-ffi `op::EVENT_*`) ---------------------------------
pub const OP_SET_PROFILE: u32 = 0; // owner / sequenced
pub const OP_SET_TICKETS: u32 = 1; // owner / sequenced
pub const OP_RECORD_SALE: u32 = 2; // any-member / commutative (single-writer delegate)
pub const OP_REDEEM: u32 = 3; // any-member / commutative (OR-set by ticket)
pub const OP_SET_MEDIA: u32 = 4; // owner / sequenced
pub const OP_SET_VENUE: u32 = 7; // owner / sequenced (5/6 are the Contact-spliced ticket legs below)
// W-98 Events (ICD 2.3.1): the owner or a co-host (admin on THIS event), commutative, each an
// LWW register by (gen, author) but the photo set, which is keyed by id.
pub const OP_SET_LINEUP: u32 = 8;
pub const OP_SET_BANNER: u32 = 9;
pub const OP_ADD_PHOTO: u32 = 10;
pub const OP_REMOVE_PHOTO: u32 = 11;
pub const OP_SET_CLIP: u32 = 12;
pub const OP_EDIT_PROFILE: u32 = 13;

// ---- media caps (enforced at fold — the reducer is the one gate every device runs) --
//
// Media rides IN the delta as base64, the ContactCard photo/clip precedent (there is
// no external fetch in Pacific — a URL would reintroduce a server). The caps are
// BYTES OF BASE64 in the arg, chosen from the proven profile-card scale upward:
// a 1080px @0.8 JPEG poster lands ~200–400KB (≈530KB b64); the clip cap fits the
// "pinhole" grammar — a few seconds of small motion, not a film. Oversize is
// MalformedArgs, refused not clamped: silent re-encoding by the engine would make
// two devices disagree about the same delta.
pub const MAX_BANNER_B64: usize = 700_000;
pub const MAX_PHOTO_B64: usize = 500_000;
pub const MAX_EVENT_PHOTOS: usize = 6;
pub const MAX_CLIP_B64: usize = 1_500_000;

// ---- Contact-spliced op ids (CONTACT's op space; 0..4 are taken there) -------------
pub const OP_REQUEST_TICKET: u32 = 5; // any-member / commutative (per-pi LWW)
pub const OP_DELIVER_TICKET: u32 = 6; // any-member / commutative (wallet OR-set)
pub const OP_INVITE: u32 = 9; // any-member / commutative (per-(author,event) LWW)
pub const OP_INVITE_REPLY: u32 = 10; // any-member / commutative (per-(author,event) LWW)
pub const OP_ADMIT_TICKET: u32 = 11; // any-member / commutative (per-ticket, monotone)

/// The two ticket legs that ride the Contact channel. `contact.rs` writes these out
/// verbatim in `CONTACT_OPS` (a `static` cannot be concatenated in a const initialiser);
/// its `ticket_ops_are_spliced_verbatim` test pins the copies to THIS table so the
/// duplication cannot drift silently.
pub static CONTACT_TICKET_OPS: &[OpDecl] = &[
    OpDecl {
        op_id: OP_REQUEST_TICKET,
        name: "contact.requestTicket",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_DELIVER_TICKET,
        name: "contact.deliverTicket",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_ADMIT_TICKET,
        name: "contact.admitTicket",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
];

/// The stable, non-correlating listing key: `sha256(event_group_id ‖ owner_pk)`, hex.
/// Travels to buyers and the box office instead of the MLS group id, which would be a
/// correlation handle (and is meaningless to a non-member anyway).
pub fn listing_id(event_group_id: &[u8], owner_pk: &[u8; 32]) -> String {
    let mut h = Sha256::new();
    h.update(event_group_id);
    h.update(owner_pk);
    hex::encode(h.finalize())
}

/// The ticket listing — the owner's sequenced assertion of what is for sale.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TicketListing {
    /// Minor units of `currency`. 0 = a free event (no rail, phone-fulfilled).
    pub price_cents: i64,
    /// ISO-4217 lowercase ("usd", "krw"). The fee is the 10¢-EQUIVALENT in this
    /// currency, computed at the box office — a pure reducer cannot consult FX.
    pub currency: String,
    pub capacity: u32,
    pub open: bool,
    /// The rail's connected-account id (Stripe acct_…). Required iff priced.
    pub acct: Option<String>,
    /// The fulfilment delegate's identity pk — the box-office node that verifies
    /// payment, authors sales, and SIGNS CREDENTIALS. Required iff priced; absent on
    /// free events (the owner fulfils those directly).
    pub delegate: Option<MemberId>,
    pub terms: Option<String>,
    /// Registration revision for the box office's LWW (a re-registration replaces).
    pub rev: u64,
}

/// One recorded sale — the ledger row. Keyed by `pi` in [`EventState::ledger`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sale {
    pub buyer: MemberId,
    pub qty: u32,
    pub unit_cents: i64,
    /// The platform fee actually collected (10¢-equivalent × qty), mirrored from the
    /// box office for local bookkeeping; the rail's ledger is the money record.
    pub fee_cents: i64,
    pub currency: String,
    /// The ticket ids this sale minted. Globally unique across the ledger.
    pub tickets: BTreeSet<String>,
    pub author: MemberId,
}

/// The redemption record for one ticket id. `authors` rather than a count so that
/// re-emission (and a device re-scanning its own admit) stays idempotent: fraud is two
/// DIFFERENT doors admitting the same ticket, not one door checking twice.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Redemption {
    pub authors: BTreeSet<MemberId>,
    /// Deterministic first admitter (canonical fold order), for display.
    pub first: Option<MemberId>,
}

impl Redemption {
    /// Two different member devices admitted this ticket — the double-use flag.
    pub fn double_scanned(&self) -> bool {
        self.authors.len() > 1
    }
}

/// What the door shows for a scanned ticket, joined read-side (never at fold, so the
/// commutative arms stay order-independent).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DoorVerdict {
    /// Not in the ledger this device holds. With a valid signature this is AMBER
    /// ("verify with organizer"), not red — the device may simply not have synced
    /// since the sale.
    Unknown,
    Valid,
    AlreadyRedeemed {
        double_scanned: bool,
    },
}

/// One event photo at rest in the fold — base64 data + its MIME, nothing else.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventPhoto {
    pub data: String,
    pub mime: String,
}

/// The event's media facet: the BANNER (the poster — the page hero), a bounded
/// photo strip, and one short motion CLIP (the pinhole grammar: parallel to the
/// banner, never a replacement — a device that can't play it still has the poster).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventMedia {
    /// base64 still; "" when none.
    pub banner: String,
    pub banner_mime: String,
    pub photos: Vec<EventPhoto>,
    /// base64 motion; "" when none.
    pub clip: String,
    pub clip_mime: String,
}

impl EventMedia {
    pub fn is_empty(&self) -> bool {
        self.banner.is_empty() && self.photos.is_empty() && self.clip.is_empty()
    }
}

/// WHO an act is (W-98, Ralph 30 Sep): a person by their member key, or an organisation or
/// band by its GroupObject id. Never a name: the name is the performer's own card.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Performer {
    Member(MemberId),
    Object([u8; 32]),
}

impl Performer {
    /// The id as the act names it, and as a performer's `performs_at` half is looked up.
    pub fn id(&self) -> String {
        match self {
            Performer::Member(m) | Performer::Object(m) => hex::encode(m),
        }
    }
}

/// One act of `event.setLineup`: who, in what role, and when on, if said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Act {
    pub performer: Performer,
    pub role: String,
    pub start: Option<i64>,
    pub end: Option<i64>,
}

/// One photo of the W-98 set (`event.addPhoto`), keyed by its id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetPhoto {
    pub media: pacific_media::MediaRef,
    pub at: i64,
}

/// The venue as a REFERENCE to a Place GroupObject — what the graph projector folds
/// into the `happens_at` edge. Distinct from `EventState::venue` (free text on the
/// profile): the string says what the flyer says, the ref says WHICH Place object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VenueRef {
    /// The Place's object id (group id hex) — the graph entity on the far end.
    pub place: String,
    /// The Place's display name as known at authoring (a label courtesy).
    pub name: String,
    /// Event time (unix ms) the edge became true — the projected fact's valid_at.
    pub at: i64,
}

/// The compiled read-side state of an Event.
#[derive(Clone, Debug, Default)]
pub struct EventState {
    /// THE PARTS THIS OBJECT IS MADE OF, keyed by the part's object id — its comments
    /// section is one, a real Forum GroupObject with its own roster and not a field
    /// on this one. See `crate::parts`.
    pub parts: std::collections::BTreeMap<String, crate::object::PartRef>,
    /// THE HALVES THIS OBJECT DECLARES of relations another object asserted.
    /// Keyed `(rel, object)`. Reciprocity is required (25 Sep 2026): a one-sided
    /// edge cannot be walked from the far end, and a kind that is not a Group had
    /// no way to write its half at all.
    pub backlinks: std::collections::BTreeMap<(String, String), crate::backlink::Backlink>,
    pub title: String,
    pub descriptor: String,
    /// Where tickets are sold when that is somewhere else (`ticketUrl` on
    /// `event.setProfile`). Empty is no link. Only http and https are ever stored.
    pub ticket_url: String,
    /// Epoch ms. 0 until the first `setProfile` — an event with no when is not yet real.
    pub start_ms: i64,
    pub end_ms: Option<i64>,
    pub venue: String,
    /// The venue as a Place reference (`event.setVenue`); None until one is set.
    pub venue_ref: Option<VenueRef>,
    /// The LINEUP — artist names, in billing order. Display-grade in v1 (no
    /// entity binding yet); the alpha's "list artists" pillar. Empty = no lineup.
    pub lineup: Vec<String>,
    /// An RFC 5545 RRULE when this event REPEATS, "" when it happens once.
    /// Stored as text and expanded on read (`crate::recurrence`) — occurrences are
    /// never minted as objects, so a weekly night stays ONE object with one log.
    pub recurrence: String,
    /// The common location facet (geo splice), for the map pin.
    pub location: Option<geo::LocationSource>,
    /// The poster/photos/clip facet. Defaults empty — an event with no media is a
    /// real state (the flyer-stripes placeholder), not a missing value.
    pub media: EventMedia,
    pub listing: Option<TicketListing>,
    /// Everyone the fold will accept a `recordSale` from: the owner, plus every
    /// delegate any listing revision ever named. Accumulates — rotation never
    /// retroactively poisons sales that were valid when made.
    pub sellers: BTreeSet<MemberId>,
    /// `pi` → the sale. The append-only money mirror.
    pub ledger: BTreeMap<String, Sale>,
    /// Every ticket id across the ledger (global-uniqueness index).
    pub tickets: BTreeSet<String>,
    /// Loud, never silent: the fold noticed sold > capacity. Recorded, not rejected —
    /// capacity is the seller's author-side duty and a paid buyer is never punished
    /// at the door for the seller's bug.
    pub over_capacity: bool,
    /// ticket id → its redemption record.
    pub redemptions: BTreeMap<String, Redemption>,
    /// What this object is a part of, and in what role (`base.setParent`). `None`
    /// until a parent names it as a part.
    pub parent: Option<crate::parent::ParentRef>,
    /// An IANA zone name; empty is the viewer's zone.
    pub tz: String,
    /// The join link of an online event; empty when none.
    pub online: String,
    /// `scheduled | postponed | cancelled`; empty is scheduled.
    pub status: String,
    /// A video's address; empty when none.
    pub video_url: String,
    /// How `descriptor` is written, `plain | markdown`; empty is plain.
    pub descriptor_format: String,
    /// Drawn without times in any zone.
    pub all_day: bool,
    /// Standing on THIS event (the roles facet): a co-host is an admin here.
    pub member_roles: BTreeMap<MemberId, crate::group::GroupRole>,
    /// The visibility facet; `None` is the kind's default (`visibility::default_for`).
    pub visibility: Option<crate::visibility::Visibility>,
    /// `event.setLineup`'s acts in billing order; `None` until one counted.
    pub acts: Option<Vec<Act>>,
    /// `event.setBanner`'s register; `None` until one counted, `Some(empty)` cleared.
    pub banner: Option<pacific_media::MediaRef>,
    /// `event.setClip`'s register, as `banner`.
    pub clip: Option<pacific_media::MediaRef>,
    /// The W-98 photo set by id; `None` until an add or a removal counted.
    pub photo_set: Option<BTreeMap<String, SetPhoto>>,
    /// Ids removed from the set: grow-only, so a removal is final whichever arrives first.
    pub photos_removed: BTreeSet<String>,
}

impl EventState {
    /// The visibility in force: the written one, else the kind's default from the ICD.
    pub fn visibility(&self) -> crate::visibility::Visibility {
        self.visibility.unwrap_or_else(|| crate::visibility::default_for(ObjectKind::Event.name()))
    }

    pub fn sold(&self) -> u32 {
        self.ledger.values().map(|s| s.qty).sum()
    }

    /// Seats left, or `None` when the event is UNCAPPED (capacity 0). An Option
    /// rather than a sentinel so no caller can render "0 left" for a show that
    /// never had a limit — the two states must not share a representation.
    pub fn remaining(&self) -> Option<u32> {
        let cap = self.listing.as_ref().map(|l| l.capacity).unwrap_or(0);
        if cap == 0 {
            return None;
        }
        Some(cap.saturating_sub(self.sold()))
    }

    /// The parsed repeat rule, or None for a one-off. The ONE place a caller turns
    /// stored text into occurrences — `recurrence` is canonical on write, so this
    /// cannot fail for state this node folded.
    pub fn repeat(&self) -> Option<crate::recurrence::Recurrence> {
        if self.recurrence.is_empty() {
            return None;
        }
        crate::recurrence::Recurrence::parse(&self.recurrence).ok()
    }

    /// The next `limit` start instants at or after `from`. A one-off event yields its
    /// own start when the window admits it, so callers need no special case.
    pub fn occurrences(&self, from: i64, to: i64, limit: usize) -> Vec<i64> {
        match self.repeat() {
            Some(r) => r.occurrences(self.start_ms, from, to, limit),
            None => {
                if self.start_ms >= from && self.start_ms <= to {
                    vec![self.start_ms]
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// True when there is no door limit — no listing, or a listing with capacity 0.
    pub fn is_uncapped(&self) -> bool {
        self.listing.as_ref().map(|l| l.capacity).unwrap_or(0) == 0
    }

    /// The signer a door must verify a credential against: the delegate when one is
    /// named, else the owner (free events). From the FOLD, never from the QR.
    pub fn credential_signer(&self, owner: &MemberId) -> MemberId {
        self.listing
            .as_ref()
            .and_then(|l| l.delegate)
            .unwrap_or(*owner)
    }

    /// The fixed point this Event resolves to, if placed (same contract as
    /// `PlaceState::point` — a Stream source yields nothing by design).
    pub fn point(&self) -> Option<&geo::GeoPoint> {
        match &self.location {
            Some(geo::LocationSource::Fixed { point }) => Some(point),
            _ => None,
        }
    }

    /// The door check, joined read-side over ledger ∩ redemptions.
    pub fn door_check(&self, ticket: &str) -> DoorVerdict {
        if !self.tickets.contains(ticket) {
            return DoorVerdict::Unknown;
        }
        match self.redemptions.get(ticket) {
            Some(r) if !r.authors.is_empty() => DoorVerdict::AlreadyRedeemed {
                double_scanned: r.double_scanned(),
            },
            _ => DoorVerdict::Valid,
        }
    }
}

pub struct EventType;

/// The base geo entries are written out rather than concatenated because
/// [`crate::geo::LOCATION_OPS`] is a `static` and Rust cannot read a static in a const
/// initialiser; `base_location_ops_are_spliced_verbatim` below pins them to geo's.
const OPS: &[OpDecl] = &[
    // The BASE parts op-group, spliced — see `crate::parts`. Written out
    // because `PART_OPS` is a `static` and Rust cannot read one in a const
    // initialiser.
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
    // The base BACKLINK ops, spliced. This is how this kind writes ITS half of a
    // relation something else asserted — `group.setAffiliation`'s job, for kinds
    // that are not a Group.
    OpDecl {
        op_id: crate::backlink::OP_SET_BACKLINK,
        name: "base.setBacklink",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: crate::backlink::OP_CLEAR_BACKLINK,
        name: "base.clearBacklink",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SET_PROFILE,
        name: "event.setProfile",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SET_TICKETS,
        name: "event.setTickets",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_RECORD_SALE,
        name: "event.recordSale",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_REDEEM,
        name: "event.redeem",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_SET_MEDIA,
        name: "event.setMedia",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SET_VENUE,
        name: "event.setVenue",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: geo::OP_SET_LOCATION,
        name: "base.setLocation",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: geo::OP_CLEAR_LOCATION,
        name: "base.clearLocation",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
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
    // The base STANDING ops, spliced (`crate::roles`): a co-host is an admin on this event.
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
    // The base VISIBILITY op, spliced (`crate::visibility`; NC-139).
    OpDecl {
        op_id: crate::visibility::OP_SET_VISIBILITY,
        name: "base.setVisibility",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SET_LINEUP,
        name: "event.setLineup",
        authority: Authority::OwnerOrRole(crate::group::GroupRole::Admin),
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_SET_BANNER,
        name: "event.setBanner",
        authority: Authority::OwnerOrRole(crate::group::GroupRole::Admin),
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_ADD_PHOTO,
        name: "event.addPhoto",
        authority: Authority::OwnerOrRole(crate::group::GroupRole::Admin),
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_REMOVE_PHOTO,
        name: "event.removePhoto",
        authority: Authority::OwnerOrRole(crate::group::GroupRole::Admin),
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_SET_CLIP,
        name: "event.setClip",
        authority: Authority::OwnerOrRole(crate::group::GroupRole::Admin),
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_EDIT_PROFILE,
        name: "event.editProfile",
        authority: Authority::OwnerOrRole(crate::group::GroupRole::Admin),
        commutativity: Commutativity::Commutative,
    },
];

impl ObjectType for EventType {
    const KIND: ObjectKind = ObjectKind::Event;
    type State = EventState;

    fn ops() -> &'static [OpDecl] {
        OPS
    }

    fn role_of(state: &EventState, member: &MemberId) -> Option<crate::group::GroupRole> {
        state.member_roles.get(member).copied()
    }

    fn reduce(state: &mut Self::State, op: &Op<'_>) -> Result<(), DeltaRejection> {
        if crate::parent::is_parent_op(op.op_id) {
            return crate::parent::reduce_parent(&mut state.parent, op);
        }
        if crate::roles::is_role_op(op.op_id) {
            return crate::roles::reduce_roles(&mut state.member_roles, op);
        }
        if crate::visibility::is_visibility_op(op.op_id) {
            let mut v = state.visibility();
            crate::visibility::reduce_visibility(&mut v, op)?;
            state.visibility = Some(v);
            return Ok(());
        }
        if crate::parts::is_part_op(op.op_id) {
            return crate::parts::reduce_parts(&mut state.parts, op);
        }
        if crate::backlink::is_backlink_op(op.op_id) {
            return crate::backlink::reduce_backlink(&mut state.backlinks, op);
        }
        if geo::is_location_op(op.op_id) {
            return geo::reduce_location(&mut state.location, op);
        }
        let args = op.args;
        match op.op_id {
            OP_SET_PROFILE => {
                // VALIDATE EVERYTHING FIRST, THEN COMMIT — a reduce-time rejection is
                // skipped at fold, so a half-applied profile would be the state forever.
                let p = Profile::of(args)?;
                p.apply(state);
                Ok(())
            }
            // THE CO-HOSTS' OPS (W-98; Ralph 30 Sep). Each is folded after the spine in (gen,
            // author, id) order, so the last write that counts is the register's value, and with
            // none counting the spine's stands. A write counts from the owner, or from a member
            // on this event's roster holding admin in THIS event's roles; a revoked co-host's
            // writes stop counting, since the spine (and its clearRole) folds first.
            OP_EDIT_PROFILE => {
                counts(state, op)?;
                let p = Profile::of(args)?;
                p.apply(state);
                Ok(())
            }
            OP_SET_LINEUP => {
                counts(state, op)?;
                let acts = acts_of(req_text(args, "acts")?)?;
                state.acts = Some(acts);
                Ok(())
            }
            OP_SET_BANNER => {
                counts(state, op)?;
                state.banner = Some(media_of(args, "banner", pacific_media::Slot::Banner, icd::EVENT_BANNER_MAX)?);
                Ok(())
            }
            OP_SET_CLIP => {
                counts(state, op)?;
                state.clip = Some(media_of(args, "clip", pacific_media::Slot::Clip, icd::EVENT_CLIP_MAX)?);
                Ok(())
            }
            OP_ADD_PHOTO => {
                counts(state, op)?;
                let id = key16(args)?;
                let media = media_of(args, "photo", pacific_media::Slot::Photo, icd::EVENT_PHOTO_MAX)?;
                let at = req_int(args, "at")?;
                if media.is_empty() || at <= 0 {
                    return Err(DeltaRejection::MalformedArgs);
                }
                if state.photos_removed.contains(&id) {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                let set = state.photo_set.get_or_insert_with(BTreeMap::new);
                if !set.contains_key(&id) && set.len() >= icd::EVENT_PHOTOS_LIVE {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                set.insert(id, SetPhoto { media, at });
                Ok(())
            }
            OP_REMOVE_PHOTO => {
                counts(state, op)?;
                let id = key16(args)?;
                state.photo_set.get_or_insert_with(BTreeMap::new).remove(&id);
                state.photos_removed.insert(id);
                Ok(())
            }
            OP_SET_VENUE => {
                // Empty/absent `place` clears the reference; re-setting replaces it
                // (LWW on the sequenced spine). Validate everything before the first
                // mutation — a reduce-time rejection is skipped at fold.
                match opt_text(args, "place") {
                    None => {
                        state.venue_ref = None;
                        Ok(())
                    }
                    Some(_) => {
                        let place = req_hex_id(args, "place")?;
                        let name = req_text(args, "name")?.to_string();
                        let at = req_int(args, "at")?;
                        if name.is_empty() || at <= 0 {
                            return Err(DeltaRejection::MalformedArgs);
                        }
                        state.venue_ref = Some(VenueRef { place, name, at });
                        Ok(())
                    }
                }
            }
            OP_SET_TICKETS => {
                let price_cents = req_int(args, "priceCents")?;
                if price_cents < 0 {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let currency = req_text(args, "currency")?.to_ascii_lowercase();
                if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_alphabetic()) {
                    return Err(DeltaRejection::MalformedArgs);
                }
                // 0 — or the arg omitted entirely — means UNCAPPED. A room with no
                // door count is a real listing (a free show, a public talk), and the
                // old rule forced organisers to invent a ceiling and then police it.
                // Negative or oversized is still malformed: that is a typo, not a
                // statement about the door.
                let capacity = match crate::arg_reads::get(args, "capacity") {
                    None => 0,
                    Some(_) => match req_int(args, "capacity")? {
                        c if (0..=u32::MAX as i64).contains(&c) => c as u32,
                        _ => return Err(DeltaRejection::MalformedArgs),
                    },
                };
                let open = req_int(args, "open")? != 0;
                let acct = opt_text(args, "acct");
                let delegate = match crate::arg_reads::get(args, "delegate") {
                    Some(ArgVal::Text(_)) => Some(arg_hex32(args, "delegate")?),
                    _ => None,
                };
                let terms = opt_text(args, "terms");
                let rev = match req_int(args, "rev")? {
                    r if r >= 0 => r as u64,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                // A priced event REQUIRES the rail account and the fulfilment
                // delegate: paid fulfilment lives at the Arc, and a listing that
                // could take money with nowhere to put it and nobody to deliver is
                // a misconfiguration to refuse loudly, not store.
                if price_cents > 0 && (acct.is_none() || delegate.is_none()) {
                    return Err(DeltaRejection::PreconditionFailed);
                }

                state.sellers.insert(op.ctx.owner);
                if let Some(d) = delegate {
                    state.sellers.insert(d);
                }
                state.listing = Some(TicketListing {
                    price_cents,
                    currency,
                    capacity,
                    open,
                    acct,
                    delegate,
                    terms,
                    rev,
                });
                Ok(())
            }
            OP_RECORD_SALE => {
                // The fold is the one gate every device runs: only an authorized
                // seller's ledger rows exist, no matter whose build authored them.
                let Some(_listing) = state.listing.as_ref() else {
                    return Err(DeltaRejection::PreconditionFailed);
                };
                if !state.sellers.contains(op.author) {
                    return Err(DeltaRejection::Unauthorized);
                }
                let pi = req_text(args, "pi")?.to_string();
                if pi.is_empty() {
                    return Err(DeltaRejection::MalformedArgs);
                }
                // Idempotent by payment reference: re-emission for late joiners and
                // crash-retries re-author the same row; holders skip, joiners insert.
                if state.ledger.contains_key(&pi) {
                    return Ok(());
                }
                let buyer = arg_hex32(args, "buyer")?;
                let qty = match req_int(args, "qty")? {
                    q if q > 0 && q <= u32::MAX as i64 => q as u32,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                let unit_cents = req_int(args, "unitCents")?;
                let fee_cents = req_int(args, "feeCents")?;
                let currency = req_text(args, "currency")?.to_string();
                // The fee mirrors what the rail actually collected: present iff the
                // sale moved money. Exact amount is the box office's FX computation —
                // a pure reducer checks shape, not rates.
                match (unit_cents, fee_cents) {
                    (u, _) if u < 0 => return Err(DeltaRejection::MalformedArgs),
                    (0, f) if f != 0 => return Err(DeltaRejection::PreconditionFailed),
                    (u, f) if u > 0 && f < 1 => return Err(DeltaRejection::PreconditionFailed),
                    _ => {}
                }
                let tickets_json = req_text(args, "tickets")?;
                let ids: Vec<String> = serde_json::from_str(tickets_json)
                    .map_err(|_| DeltaRejection::MalformedArgs)?;
                if ids.len() != qty as usize || ids.iter().any(|t| t.is_empty()) {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let set: BTreeSet<String> = ids.iter().cloned().collect();
                if set.len() != ids.len() || ids.iter().any(|t| state.tickets.contains(t)) {
                    // A ticket id can be minted by exactly one sale, ever.
                    return Err(DeltaRejection::PreconditionFailed);
                }

                state.tickets.extend(set.iter().cloned());
                state.ledger.insert(
                    pi,
                    Sale {
                        buyer,
                        qty,
                        unit_cents,
                        fee_cents,
                        currency,
                        tickets: set,
                        author: *op.author,
                    },
                );
                if let Some(l) = state.listing.as_ref() {
                    // An uncapped listing cannot be oversold — there is no ceiling to
                    // cross, so the flag must stay down rather than trip at the first sale.
                    if l.capacity > 0 && state.sold() > l.capacity {
                        state.over_capacity = true;
                    }
                }
                Ok(())
            }
            OP_SET_MEDIA => {
                // VALIDATE EVERYTHING FIRST, THEN COMMIT — and refuse oversize rather
                // than clamp: an engine that silently re-encodes makes two devices
                // disagree about the same delta.
                let banner = opt_text(args, "banner").unwrap_or_default();
                let banner_mime = opt_text(args, "bannerMime").unwrap_or_default();
                if banner.len() > MAX_BANNER_B64 || (banner.is_empty() != banner_mime.is_empty()) {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let photos: Vec<EventPhoto> = match crate::arg_reads::get(args, "photos") {
                    Some(ArgVal::Text(json)) => {
                        serde_json::from_str(json).map_err(|_| DeltaRejection::MalformedArgs)?
                    }
                    None => Vec::new(),
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                if photos.len() > MAX_EVENT_PHOTOS
                    || photos.iter().any(|p| {
                        p.data.is_empty() || p.data.len() > MAX_PHOTO_B64 || p.mime.is_empty()
                    })
                {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let clip = opt_text(args, "clip").unwrap_or_default();
                let clip_mime = opt_text(args, "clipMime").unwrap_or_default();
                if clip.len() > MAX_CLIP_B64 || (clip.is_empty() != clip_mime.is_empty()) {
                    return Err(DeltaRejection::MalformedArgs);
                }

                state.media = EventMedia {
                    banner,
                    banner_mime,
                    photos,
                    clip,
                    clip_mime,
                };
                Ok(())
            }
            OP_REDEEM => {
                let ticket = req_text(args, "ticket")?.to_string();
                if ticket.is_empty() {
                    return Err(DeltaRejection::MalformedArgs);
                }
                // Deliberately NOT validated against the ledger here: both arms are
                // commutative and interleave in canonical order, so a redeem folding
                // before its sale must not be dropped. Validity is a READ-SIDE join
                // (`door_check`), keeping the fold order-independent.
                let r = state.redemptions.entry(ticket).or_default();
                if r.first.is_none() {
                    r.first = Some(*op.author);
                }
                r.authors.insert(*op.author);
                Ok(())
            }
            _ => Err(DeltaRejection::UnknownType),
        }
    }
}

use crate::visibility::icd;

/// Whether this write counts: `gen` carried (the LWW key), and the author the owner or an
/// admin on this event's roster (ICD `owner|role:admin`). Refused otherwise, as host.editMedia.
fn counts(state: &EventState, op: &Op<'_>) -> Result<(), DeltaRejection> {
    if !matches!(crate::arg_reads::get(op.args, "gen"), Some(ArgVal::Int(_))) {
        return Err(DeltaRejection::MalformedArgs);
    }
    let admin = op.ctx.is_member(op.author)
        && state.member_roles.get(op.author) == Some(&crate::group::GroupRole::Admin);
    if *op.author != op.ctx.owner && !admin {
        return Err(DeltaRejection::PreconditionFailed);
    }
    Ok(())
}

/// An http or https address of at most `max` characters (the arg's ICD `maxLength`), or empty:
/// a consumer draws it into an href, so any other scheme is refused, never dropped.
pub(crate) fn web_address(args: &Args, key: &str, max: usize) -> Result<String, DeltaRejection> {
    let v = opt_text(args, key).unwrap_or_default();
    if !v.is_empty() {
        let lower = v.to_ascii_lowercase();
        let web = lower.starts_with("https://") || lower.starts_with("http://");
        if !web || v.chars().count() > max {
            return Err(DeltaRejection::MalformedArgs);
        }
    }
    Ok(v)
}

/// One word of an ICD vocabulary, or empty when absent; anything else is refused.
pub(crate) fn word_of(args: &Args, key: &str, vocabulary: &[&str]) -> Result<String, DeltaRejection> {
    match opt_text(args, key) {
        None => Ok(String::new()),
        Some(w) if vocabulary.contains(&w.as_str()) => Ok(w),
        Some(_) => Err(DeltaRejection::MalformedArgs),
    }
}

/// event.setProfile's fields, as setProfile and editProfile both carry them. Validated whole
/// before anything is applied.
struct Profile {
    title: String,
    descriptor: String,
    ticket_url: String,
    start_ms: i64,
    end_ms: Option<i64>,
    venue: String,
    recurrence: String,
    lineup: Vec<String>,
    tz: String,
    online: String,
    status: String,
    video_url: String,
    descriptor_format: String,
    all_day: bool,
}

impl Profile {
    fn of(args: &Args) -> Result<Profile, DeltaRejection> {
        let title = req_text(args, "title")?.to_string();
        let start_ms = req_int(args, "startMs")?;
        if start_ms <= 0 {
            return Err(DeltaRejection::MalformedArgs);
        }
        let end_ms = match crate::arg_reads::get(args, "endMs") {
            Some(ArgVal::Int(e)) if *e >= start_ms => Some(*e),
            Some(_) => return Err(DeltaRejection::MalformedArgs),
            None => None,
        };
        let descriptor = opt_text(args, "descriptor").unwrap_or_default();
        let venue = opt_text(args, "venue").unwrap_or_default();
        // Parsed, not merely stored: an RRULE we cannot expand would leave the
        // organiser believing the event repeats when nothing will ever show it
        // twice. Canonicalised on the way in so the stored text round-trips.
        let recurrence = match opt_text(args, "recurrence") {
            Some(r) if !r.trim().is_empty() => crate::recurrence::Recurrence::parse(&r)?.to_rrule(),
            _ => String::new(),
        };
        // Lineup: newline-separated artist names. Tolerant parse (trim, drop
        // blanks, cap) — a name list has no wrong values, only noise; capping
        // keeps a hostile delta from growing the state unboundedly.
        let lineup: Vec<String> = opt_text(args, "lineup")
            .map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty()).take(60).map(str::to_string).collect())
            .unwrap_or_default();
        // An EXTERNAL ticket link, the join link and the video: web addresses or refused.
        let ticket_url = web_address(args, "ticketUrl", icd::EVENT_TICKET_URL_MAX)?;
        let online = web_address(args, "online", icd::EVENT_ONLINE_MAX)?;
        let video_url = web_address(args, "videoUrl", icd::EVENT_VIDEO_URL_MAX)?;
        let tz = opt_text(args, "tz").unwrap_or_default();
        if tz.chars().count() > icd::EVENT_TZ_MAX
            || !tz.bytes().all(|b| b.is_ascii_alphanumeric() || b"/_+-".contains(&b))
        {
            return Err(DeltaRejection::MalformedArgs);
        }
        let status = word_of(args, "status", icd::EVENT_STATUS)?;
        let descriptor_format = word_of(args, "descriptorFormat", icd::EVENT_DESCRIPTOR_FORMATS)?;
        let all_day = match crate::arg_reads::get(args, "allDay") {
            None | Some(ArgVal::Int(0)) => false,
            Some(ArgVal::Int(1)) => true,
            Some(_) => return Err(DeltaRejection::MalformedArgs),
        };
        Ok(Profile { title, descriptor, ticket_url, start_ms, end_ms, venue, recurrence, lineup, tz, online, status, video_url, descriptor_format, all_day })
    }

    fn apply(self, state: &mut EventState) {
        state.title = self.title;
        state.descriptor = self.descriptor;
        state.ticket_url = self.ticket_url;
        state.start_ms = self.start_ms;
        state.end_ms = self.end_ms;
        state.venue = self.venue;
        state.recurrence = self.recurrence;
        state.lineup = self.lineup;
        state.tz = self.tz;
        state.online = self.online;
        state.status = self.status;
        state.video_url = self.video_url;
        state.descriptor_format = self.descriptor_format;
        state.all_day = self.all_day;
    }
}

/// `event.setLineup`'s acts: a JSON list of at most the ICD's `maxItems`, each exactly one of
/// `member` and `object` (64 hex), a role from the ICD's vocabulary, and optional `start` and
/// `end` (unix ms, end not before start). Any other key, a performer twice, or anything
/// malformed refuses the whole list: a name beside the key would be a copy of the card.
fn acts_of(json: &str) -> Result<Vec<Act>, DeltaRejection> {
    let bad = || DeltaRejection::MalformedArgs;
    let list: Vec<serde_json::Value> = serde_json::from_str(json).map_err(|_| bad())?;
    if list.len() > icd::EVENT_ACTS_MAX {
        return Err(bad());
    }
    let key32 = |v: &serde_json::Value| -> Result<[u8; 32], DeltaRejection> {
        let s = v.as_str().ok_or_else(bad)?;
        hex::decode(s).ok().and_then(|b| <[u8; 32]>::try_from(b).ok()).ok_or_else(bad)
    };
    let mut seen = BTreeSet::new();
    let mut acts = Vec::with_capacity(list.len());
    for item in &list {
        let o = item.as_object().ok_or_else(bad)?;
        if o.keys().any(|k| !["member", "object", "role", "start", "end"].contains(&k.as_str())) {
            return Err(bad());
        }
        let performer = match (o.get("member"), o.get("object")) {
            (Some(m), None) => Performer::Member(key32(m)?),
            (None, Some(g)) => Performer::Object(key32(g)?),
            _ => return Err(bad()),
        };
        let role = o.get("role").and_then(|r| r.as_str()).ok_or_else(bad)?;
        if !icd::EVENT_ACT_ROLES.contains(&role) {
            return Err(bad());
        }
        let when = |k: &str| match o.get(k) {
            None => Ok(None),
            Some(v) => v.as_i64().map(Some).ok_or_else(bad),
        };
        let (start, end) = (when("start")?, when("end")?);
        if let (Some(a), Some(b)) = (start, end) {
            if b < a {
                return Err(bad());
            }
        }
        if !seen.insert(performer.clone()) {
            return Err(bad());
        }
        acts.push(Act { performer, role: role.to_string(), start, end });
    }
    Ok(acts)
}

/// A media slot of the W-98 ops, read as the media crate writes one under `prefix`: present
/// (absent is refused), structurally whole, a kind `slot` takes, and inline within the ICD's
/// `maxLength` (the carriage ceiling). An empty ref is how a register is cleared.
pub(crate) fn media_of(args: &Args, prefix: &str, slot: pacific_media::Slot, max: usize) -> Result<pacific_media::MediaRef, DeltaRejection> {
    let bad = || DeltaRejection::MalformedArgs;
    let m = pacific_media::MediaRef::from_args_by(prefix, |k| crate::arg_reads::get(args, k)).ok_or_else(bad)?.map_err(|_| bad())?;
    if m.is_empty() {
        return Ok(m);
    }
    m.validate_bounds(slot).map_err(|_| bad())?;
    if !slot.accepts().contains(&m.kind) {
        return Err(bad());
    }
    if matches!(&m.delivery, pacific_media::Delivery::Inline { data } if data.len() > max) {
        return Err(bad());
    }
    Ok(m)
}

/// A set member's key: the ICD's `pattern`, `^[0-9a-f]{16}$`, chosen by the client.
pub(crate) fn key16(args: &Args) -> Result<String, DeltaRejection> {
    let id = req_text(args, "id")?;
    if id.len() != 16 || !id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err(DeltaRejection::MalformedArgs);
    }
    Ok(id.to_string())
}

/// THE PERFORMER'S CONSENT (performs_at, Ralph 30 Sep): each act is confirmed exactly where the
/// reader holds the performer's own half, `group.setAffiliation {rel: performs_at, peer: this
/// event}` on the performer's record (a person's self record, an organisation's group).
/// `halves` is the performers, by [`Performer::id`], whose half the reader holds; the caller
/// that holds both objects builds it (node.rs). Unconfirmed acts are kept, never dropped.
pub fn confirmed(acts: &[Act], halves: &BTreeSet<String>) -> Vec<bool> {
    acts.iter().map(|a| halves.contains(&a.performer.id())).collect()
}

/// The acts as the event view carries them: `[{member | object, role, start, end, confirmed}]`.
pub fn acts_view(acts: &[Act], halves: &BTreeSet<String>) -> serde_json::Value {
    let ok = confirmed(acts, halves);
    serde_json::Value::Array(
        acts.iter()
            .zip(ok)
            .map(|(a, confirmed)| {
                let mut o = serde_json::Map::new();
                let (k, id) = match &a.performer {
                    Performer::Member(_) => ("member", a.performer.id()),
                    Performer::Object(_) => ("object", a.performer.id()),
                };
                o.insert(k.into(), id.into());
                o.insert("role".into(), a.role.clone().into());
                o.insert("start".into(), a.start.map(Into::into).unwrap_or(serde_json::Value::Null));
                o.insert("end".into(), a.end.map(Into::into).unwrap_or(serde_json::Value::Null));
                o.insert("confirmed".into(), confirmed.into());
                serde_json::Value::Object(o)
            })
            .collect(),
    )
}

/// One media ref as a view draws it: `{mime, data, width, height, duration_ms}`, and for a
/// detached ref its delivery (digest, bytes, and O-79's key and secret: the roster's to read).
pub fn media_view(m: &pacific_media::MediaRef) -> serde_json::Value {
    use pacific_media::Delivery;
    if m.is_empty() {
        return serde_json::Value::Null;
    }
    let mut o = serde_json::json!({
        "mime": m.mime, "width": m.width, "height": m.height, "duration_ms": m.duration_ms,
    });
    match &m.delivery {
        Delivery::Inline { data } => {
            o["data"] = data.clone().into();
        }
        Delivery::Detached { digest, bytes } => {
            o["data"] = "".into();
            o["via"] = "detached".into();
            o["digest"] = hex::encode(digest).into();
            o["bytes"] = (*bytes).into();
        }
        Delivery::Sealed { digest, bytes, key, secret } => {
            o["data"] = "".into();
            o["via"] = "detached".into();
            o["digest"] = hex::encode(digest).into();
            o["bytes"] = (*bytes).into();
            o["key"] = key.clone().into();
            o["secret"] = secret.clone().into();
        }
        Delivery::Live { session } => {
            o["data"] = "".into();
            o["via"] = "live".into();
            o["session"] = session.clone().into();
        }
    }
    o
}

// ==== the two Contact-spliced reducers (called from `ContactType::reduce`) ==========

/// A buyer's claim on a checkout session, authored BEFORE the payment page opens so a
/// paid-then-killed-app session always has a request for the sweep to find.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TicketRequest {
    pub listing: String,
    pub qty: u32,
    pub author: MemberId,
    pub gen: u64,
}

/// One delivered ticket at rest in the buyer's wallet. `core` is the credential's
/// signed JSON **verbatim** — stored and re-presented byte-for-byte, never
/// re-serialized, or the signature dies (the SELLING-DELTA rule).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalletTicket {
    pub listing: String,
    pub core: String,
    pub sig: String,
    pub author: MemberId,
    /// When a door told the HOLDER their ticket was admitted (epoch ms; 0 = not yet).
    ///
    /// Admission itself is recorded in the EVENT fold, which the buyer is deliberately
    /// not a member of — so without this the holder could never learn the outcome of
    /// their own scan. The door sends a receipt back down the pairwise channel; this
    /// is where it lands. MONOTONE: the first receipt wins and later ones cannot move
    /// it, so a re-scan or a re-delivery never rewrites history.
    pub admitted_ms: i64,
}

impl WalletTicket {
    pub fn admitted(&self) -> bool {
        self.admitted_ms > 0
    }
}

/// An invitation to an event, carried on the pairwise Contact channel.
///
/// SELF-DESCRIBING on purpose. The invitee does not have the Event GroupObject —
/// buyers and guests never join it (see the module header on the delegate model) —
/// so an invite carrying only an object id would render as an opaque handle in a
/// chat and be unanswerable without a round-trip to a node nobody has. Carrying the
/// title, the when and the where means the card in the thread says something the
/// moment it lands, offline, forever.
///
/// It is a SNAPSHOT, not a mirror: if the organiser later moves the event, this
/// invite still says what was sent. The event id is the join back to the live object
/// for anyone who does hold it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invite {
    /// The Event object id (hex) — the join back to the live object.
    pub event: String,
    pub title: String,
    /// Unix ms the event starts, as known at invite time.
    pub start_ms: i64,
    /// Venue name as known at invite time; "" when unplaced.
    pub venue: String,
    /// The organiser's line to this person; "" when none.
    pub note: String,
    /// Unix ms the invite was sent.
    pub at: i64,
    pub author: MemberId,
    pub gen: u64,
}

/// The invitee's answer. Its own leg rather than a field on the invite, because the
/// two are authored by DIFFERENT people — one author may never write the other's
/// slot, and a reply must not be able to rewrite the invitation it answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InviteReply {
    pub event: String,
    pub going: bool,
    pub at: i64,
    pub author: MemberId,
    pub gen: u64,
}

/// `contact.invite` — per-(author, event) LWW, same stale-replay guard as the rest.
pub fn reduce_invite(
    invites: &mut BTreeMap<(MemberId, String), Invite>,
    op: &Op<'_>,
) -> Result<(), DeltaRejection> {
    let event = req_text(op.args, "event")?.to_string();
    let title = req_text(op.args, "title")?.to_string();
    if event.is_empty() || title.is_empty() {
        // An invite with no event, or no name for it, is not answerable.
        return Err(DeltaRejection::MalformedArgs);
    }
    let start_ms = req_int(op.args, "startMs")?;
    if start_ms <= 0 {
        return Err(DeltaRejection::MalformedArgs);
    }
    let at = req_int(op.args, "at")?;
    if at <= 0 {
        return Err(DeltaRejection::MalformedArgs);
    }
    let gen = match req_int(op.args, "gen")? {
        g if g >= 0 => g as u64,
        _ => return Err(DeltaRejection::MalformedArgs),
    };
    let key = (*op.author, event.clone());
    if let Some(held) = invites.get(&key) {
        if gen < held.gen {
            return Ok(()); // a stale replay never un-does a newer invite
        }
    }
    invites.insert(
        key,
        Invite {
            event,
            title,
            start_ms,
            venue: opt_text(op.args, "venue").unwrap_or_default(),
            note: opt_text(op.args, "note").unwrap_or_default(),
            at,
            author: *op.author,
            gen,
        },
    );
    Ok(())
}

/// `contact.inviteReply` — per-(author, event) LWW. Changing your mind is just a
/// later gen, so "yes then no" needs no retraction op.
pub fn reduce_invite_reply(
    replies: &mut BTreeMap<(MemberId, String), InviteReply>,
    op: &Op<'_>,
) -> Result<(), DeltaRejection> {
    let event = req_text(op.args, "event")?.to_string();
    if event.is_empty() {
        return Err(DeltaRejection::MalformedArgs);
    }
    let at = req_int(op.args, "at")?;
    if at <= 0 {
        return Err(DeltaRejection::MalformedArgs);
    }
    let gen = match req_int(op.args, "gen")? {
        g if g >= 0 => g as u64,
        _ => return Err(DeltaRejection::MalformedArgs),
    };
    let key = (*op.author, event.clone());
    if let Some(held) = replies.get(&key) {
        if gen < held.gen {
            return Ok(());
        }
    }
    replies.insert(
        key,
        InviteReply {
            event,
            going: req_int(op.args, "going")? != 0,
            at,
            author: *op.author,
            gen,
        },
    );
    Ok(())
}

/// `contact.requestTicket` — per-`pi` LWW with the `publishProfile` stale-replay guard.
pub fn reduce_request_ticket(
    requests: &mut BTreeMap<String, TicketRequest>,
    op: &Op<'_>,
) -> Result<(), DeltaRejection> {
    let listing = req_text(op.args, "listing")?.to_string();
    let pi = req_text(op.args, "pi")?.to_string();
    if listing.is_empty() || pi.is_empty() {
        return Err(DeltaRejection::MalformedArgs);
    }
    let qty = match req_int(op.args, "qty")? {
        q if q > 0 && q <= u32::MAX as i64 => q as u32,
        _ => return Err(DeltaRejection::MalformedArgs),
    };
    let gen = match req_int(op.args, "gen")? {
        g if g >= 0 => g as u64,
        _ => return Err(DeltaRejection::MalformedArgs),
    };
    if let Some(held) = requests.get(&pi) {
        if gen < held.gen {
            return Ok(()); // a stale replay never un-does a newer claim
        }
    }
    requests.insert(
        pi,
        TicketRequest {
            listing,
            qty,
            author: *op.author,
            gen,
        },
    );
    Ok(())
}

/// `contact.deliverTicket` — fold `[{core, sig}]` into the wallet, keyed by the ticket
/// id READ from each core. The core string is kept verbatim; parsing is only a lens.
pub fn reduce_deliver_ticket(
    wallet: &mut BTreeMap<String, WalletTicket>,
    op: &Op<'_>,
) -> Result<(), DeltaRejection> {
    let listing = req_text(op.args, "listing")?.to_string();
    if listing.is_empty() {
        return Err(DeltaRejection::MalformedArgs);
    }
    let payload = req_text(op.args, "tickets")?;
    #[derive(Deserialize)]
    struct Entry {
        core: String,
        sig: String,
    }
    let entries: Vec<Entry> =
        serde_json::from_str(payload).map_err(|_| DeltaRejection::MalformedArgs)?;
    if entries.is_empty() {
        return Err(DeltaRejection::MalformedArgs);
    }
    // Validate ALL entries before inserting ANY — the atomicity contract.
    let mut parsed: Vec<(String, Entry)> = Vec::with_capacity(entries.len());
    for e in entries {
        if e.core.is_empty() || e.sig.is_empty() {
            return Err(DeltaRejection::MalformedArgs);
        }
        let core: TicketCore =
            serde_json::from_str(&e.core).map_err(|_| DeltaRejection::MalformedArgs)?;
        if core.ticket.is_empty() {
            return Err(DeltaRejection::MalformedArgs);
        }
        parsed.push((core.ticket, e));
    }
    for (ticket, e) in parsed {
        // A RE-DELIVERY MUST NOT UN-ADMIT. Redelivery is a normal, idempotent part of
        // the fulfilment reconcile (a crash between recordSale and deliver replays it),
        // so carrying the existing receipt forward is what keeps "already admitted"
        // true across it.
        let admitted_ms = wallet.get(&ticket).map(|w| w.admitted_ms).unwrap_or(0);
        wallet.insert(
            ticket,
            WalletTicket {
                listing: listing.clone(),
                core: e.core,
                sig: e.sig,
                author: *op.author,
                admitted_ms,
            },
        );
    }
    Ok(())
}

/// `contact.admitTicket` — the door's receipt to the HOLDER: "you were let in".
///
/// The buyer is not a member of the Event group, so the redemption recorded there is
/// invisible to them; this carries the outcome back down the pairwise channel the
/// ticket was delivered on. Monotone and idempotent: the FIRST admission time wins, so
/// two doors, a replay, or a later re-delivery all converge on the same value.
///
/// A receipt for a ticket this wallet does not hold is IGNORED, not rejected — the
/// arms are commutative and a receipt may fold before the delivery that introduced its
/// ticket; the delivery will simply carry admitted_ms = 0 and the next receipt sets it.
pub fn reduce_admit_ticket(
    wallet: &mut BTreeMap<String, WalletTicket>,
    op: &Op<'_>,
) -> Result<(), DeltaRejection> {
    let ticket = req_text(op.args, "ticket")?.to_string();
    if ticket.is_empty() {
        return Err(DeltaRejection::MalformedArgs);
    }
    let at = match req_int(op.args, "admittedMs")? {
        ms if ms > 0 => ms,
        _ => return Err(DeltaRejection::MalformedArgs),
    };
    if let Some(w) = wallet.get_mut(&ticket) {
        if w.admitted_ms == 0 || at < w.admitted_ms {
            w.admitted_ms = at;
        }
    }
    Ok(())
}

/// Build the args for `admitTicket`.
pub fn admit_ticket_args(ticket: &str, admitted_ms: i64) -> Args {
    let mut a = Args::new();
    a.insert("ticket".into(), ArgVal::Text(ticket.into()));
    a.insert("admittedMs".into(), ArgVal::Int(admitted_ms));
    a
}

// ==== the credential ================================================================

/// The signed core of one ticket. Built ONCE with [`TicketCore::to_signed_json`]; from
/// then on the JSON string is the object — hashed, signed, delivered, displayed, and
/// verified as the same bytes. Unknown fields are tolerated on parse so a newer
/// issuer's tickets still verify on an older door.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TicketCore {
    pub v: u32,
    pub listing: String,
    pub ticket: String,
    pub title: String,
    pub start_ms: i64,
    /// The paying buyer's identity pk (hex) — binds the credential to its purchaser;
    /// the v2 door-challenge hook. Not gated on at the v1 door.
    pub buyer: String,
    pub issued_at: i64,
}

impl TicketCore {
    /// Serialize the core ONCE. The returned string is canonical for this ticket's
    /// lifetime — callers must never re-serialize a parsed core.
    pub fn to_signed_json(&self, identity: &crate::identity::Identity) -> (String, String) {
        let core = serde_json::to_string(self).expect("TicketCore serializes");
        let sig = hex::encode(identity.sign(core.as_bytes()));
        (core, sig)
    }
}

/// Parse a core for DISPLAY without verifying — wallet list rendering only; never a
/// substitute for [`verify_ticket`] on any trust decision.
pub fn parse_ticket_core(core_json: &str) -> Option<TicketCore> {
    serde_json::from_str(core_json).ok()
}

/// Verify a credential against the signer pk read from the EVENT FOLD (never the QR),
/// and only then parse the core. Loud on any failure.
pub fn verify_ticket(
    core_json: &str,
    sig_hex: &str,
    signer_pk: &[u8; 32],
) -> Result<TicketCore, CoreError> {
    let sig_bytes = hex::decode(sig_hex)
        .ok()
        .and_then(|v| <[u8; 64]>::try_from(v).ok())
        .ok_or_else(|| CoreError::Identity("ticket sig is not 64 hex bytes".into()))?;
    crate::identity::verify_sig(signer_pk, core_json.as_bytes(), &sig_bytes)?;
    serde_json::from_str(core_json)
        .map_err(|e| CoreError::Identity(format!("verified ticket core failed to parse: {e}")))
}

/// The QR payload: `pacific://ticket?c=<b64url(core)>&s=<hex sig>`.
pub fn qr_payload(core_json: &str, sig_hex: &str) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64URL, Engine as _};
    format!(
        "pacific://ticket?c={}&s={}",
        B64URL.encode(core_json.as_bytes()),
        sig_hex
    )
}

/// Parse a scanned QR payload back into `(core_json, sig_hex)`. Accepts exactly the
/// shape `qr_payload` emits; anything else is a loud error, never a guess.
pub fn parse_qr_payload(payload: &str) -> Result<(String, String), CoreError> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64URL, Engine as _};
    let rest = payload
        .strip_prefix("pacific://ticket?c=")
        .ok_or_else(|| CoreError::Identity("not a pacific ticket QR".into()))?;
    let (c, s) = rest
        .split_once("&s=")
        .ok_or_else(|| CoreError::Identity("ticket QR missing signature".into()))?;
    let core = B64URL
        .decode(c)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .ok_or_else(|| CoreError::Identity("ticket QR core is not base64url utf-8".into()))?;
    Ok((core, s.to_string()))
}

// ==== args builders (node/FFI author deltas through these) ==========================

pub fn set_profile_args(
    title: &str,
    descriptor: Option<&str>,
    start_ms: i64,
    end_ms: Option<i64>,
    venue: Option<&str>,
    // `recurrence`: an RFC 5545 RRULE when the event repeats. Validated at reduce
    // time, so a rule we cannot expand is refused rather than silently stored.
    recurrence: Option<&str>,
    // `lineup`: newline-separated artist names, billing order. None = unchanged
    // vocabulary for older callers; the fold treats absent as empty.
    lineup: Option<&str>,
) -> Args {
    let mut a = Args::new();
    a.insert("title".into(), ArgVal::Text(title.into()));
    if let Some(d) = descriptor {
        a.insert("descriptor".into(), ArgVal::Text(d.into()));
    }
    a.insert("startMs".into(), ArgVal::Int(start_ms));
    if let Some(e) = end_ms {
        a.insert("endMs".into(), ArgVal::Int(e));
    }
    if let Some(v) = venue {
        a.insert("venue".into(), ArgVal::Text(v.into()));
    }
    if let Some(r) = recurrence {
        a.insert("recurrence".into(), ArgVal::Text(r.into()));
    }
    if let Some(l) = lineup {
        a.insert("lineup".into(), ArgVal::Text(l.into()));
    }
    a
}

/// All three keys ride every message — the ICD payload schema
/// (`coordination/delta-graph.icd.json`, `event.setVenue`) requires them, with
/// `additionalProperties: false`. `place` "" clears the reference.
pub fn set_venue_args(place: Option<&str>, name: &str, at: i64) -> Args {
    let mut a = Args::new();
    a.insert(
        "place".into(),
        ArgVal::Text(place.unwrap_or_default().into()),
    );
    a.insert("name".into(), ArgVal::Text(name.into()));
    a.insert("at".into(), ArgVal::Int(at));
    a
}

#[allow(clippy::too_many_arguments)]
pub fn set_tickets_args(
    price_cents: i64,
    currency: &str,
    capacity: u32,
    open: bool,
    acct: Option<&str>,
    delegate: Option<&[u8; 32]>,
    terms: Option<&str>,
    rev: u64,
) -> Args {
    let mut a = Args::new();
    a.insert("priceCents".into(), ArgVal::Int(price_cents));
    a.insert("currency".into(), ArgVal::Text(currency.into()));
    a.insert("capacity".into(), ArgVal::Int(capacity as i64));
    a.insert("open".into(), ArgVal::Int(open as i64));
    if let Some(acct) = acct {
        a.insert("acct".into(), ArgVal::Text(acct.into()));
    }
    if let Some(d) = delegate {
        a.insert("delegate".into(), ArgVal::Text(hex::encode(d)));
    }
    if let Some(t) = terms {
        a.insert("terms".into(), ArgVal::Text(t.into()));
    }
    a.insert("rev".into(), ArgVal::Int(rev as i64));
    a
}

pub fn record_sale_args(
    pi: &str,
    buyer: &[u8; 32],
    qty: u32,
    unit_cents: i64,
    fee_cents: i64,
    currency: &str,
    ticket_ids: &[String],
) -> Args {
    let mut a = Args::new();
    a.insert("pi".into(), ArgVal::Text(pi.into()));
    a.insert("buyer".into(), ArgVal::Text(hex::encode(buyer)));
    a.insert("qty".into(), ArgVal::Int(qty as i64));
    a.insert("unitCents".into(), ArgVal::Int(unit_cents));
    a.insert("feeCents".into(), ArgVal::Int(fee_cents));
    a.insert("currency".into(), ArgVal::Text(currency.into()));
    a.insert(
        "tickets".into(),
        ArgVal::Text(serde_json::to_string(ticket_ids).expect("ids serialize")),
    );
    a
}

/// Build the args for `setMedia`. `photos` serializes to ONE json text arg (the
/// `publishProfile` envelope rule); empty slots are simply absent.
pub fn set_media_args(
    banner: Option<(&str, &str)>,
    photos: &[EventPhoto],
    clip: Option<(&str, &str)>,
) -> Args {
    let mut a = Args::new();
    if let Some((data, mime)) = banner {
        a.insert("banner".into(), ArgVal::Text(data.into()));
        a.insert("bannerMime".into(), ArgVal::Text(mime.into()));
    }
    if !photos.is_empty() {
        a.insert(
            "photos".into(),
            ArgVal::Text(serde_json::to_string(photos).expect("photos serialize")),
        );
    }
    if let Some((data, mime)) = clip {
        a.insert("clip".into(), ArgVal::Text(data.into()));
        a.insert("clipMime".into(), ArgVal::Text(mime.into()));
    }
    a
}

pub fn redeem_args(ticket: &str) -> Args {
    let mut a = Args::new();
    a.insert("ticket".into(), ArgVal::Text(ticket.into()));
    a
}

/// Author helper for `contact.invite`. `gen` is supplied by the node (the author's
/// delta count), like every other commutative leg.
pub fn invite_args(
    event: &str,
    title: &str,
    start_ms: i64,
    venue: &str,
    note: &str,
    at: i64,
) -> Args {
    let mut a = Args::new();
    a.insert("event".into(), ArgVal::Text(event.into()));
    a.insert("title".into(), ArgVal::Text(title.into()));
    a.insert("startMs".into(), ArgVal::Int(start_ms));
    a.insert("venue".into(), ArgVal::Text(venue.into()));
    a.insert("note".into(), ArgVal::Text(note.into()));
    a.insert("at".into(), ArgVal::Int(at));
    a
}

/// Author helper for `contact.inviteReply`.
pub fn invite_reply_args(event: &str, going: bool, at: i64) -> Args {
    let mut a = Args::new();
    a.insert("event".into(), ArgVal::Text(event.into()));
    a.insert("going".into(), ArgVal::Int(i64::from(going)));
    a.insert("at".into(), ArgVal::Int(at));
    a
}

pub fn request_ticket_args(listing: &str, qty: u32, pi: &str) -> Args {
    let mut a = Args::new();
    a.insert("listing".into(), ArgVal::Text(listing.into()));
    a.insert("qty".into(), ArgVal::Int(qty as i64));
    a.insert("pi".into(), ArgVal::Text(pi.into()));
    a
}

/// `tickets` entries are `(core_json, sig_hex)` pairs, kept verbatim.
pub fn deliver_ticket_args(listing: &str, tickets: &[(String, String)]) -> Args {
    #[derive(Serialize)]
    struct Entry<'a> {
        core: &'a str,
        sig: &'a str,
    }
    let entries: Vec<Entry> = tickets
        .iter()
        .map(|(c, s)| Entry { core: c, sig: s })
        .collect();
    let mut a = Args::new();
    a.insert("listing".into(), ArgVal::Text(listing.into()));
    a.insert(
        "tickets".into(),
        ArgVal::Text(serde_json::to_string(&entries).expect("entries serialize")),
    );
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator::{sequenced_delta, Coordinator, GENESIS_PREV};
    use crate::object::{build_delta, MemberId, ReduceContext};

    fn owner() -> MemberId {
        [7u8; 32]
    }
    fn delegate() -> MemberId {
        [9u8; 32]
    }
    fn stranger() -> MemberId {
        [13u8; 32]
    }

    fn args(pairs: &[(&str, ArgVal)]) -> Args {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    /// A deterministic test identity (the real one is minted on disk by `id init`).
    fn test_identity(seed: u8) -> crate::identity::Identity {
        crate::identity::Identity::in_memory([seed; 32])
    }

    /// Fold owner-sequenced deltas, then commutative deltas from arbitrary authors,
    /// through the REAL coordinator — spine first, exactly as `state()` folds.
    fn folded(seq_ops: Vec<(u32, Args)>, comm_ops: Vec<(u32, Args, MemberId, u64)>) -> EventState {
        let mut c: Coordinator<EventType> =
            Coordinator::new(vec![owner(), delegate(), stranger()], owner());
        let mut prev = GENESIS_PREV;
        for (i, (op_id, a)) in seq_ops.into_iter().enumerate() {
            let d = sequenced_delta(
                ObjectKind::Event.type_id() as u32,
                op_id,
                a,
                0,
                i as u64,
                prev,
            );
            prev = d.id();
            c.deliver(d, owner())
                .expect("owner-sequenced delta accepted");
        }
        for (op_id, a, author, gen) in comm_ops {
            let d = build_delta(ObjectKind::Event, op_id, a, 0, Some(gen));
            c.deliver(d, author).expect("commutative delta accepted");
        }
        c.state()
    }

    /// The external ticket link: stored when it is a web address, refused when it
    /// is anything else, because a consumer draws it straight into an href.
    #[test]
    fn a_ticket_url_is_a_web_address_or_it_is_refused() {
        let profile = |url: Option<&str>| {
            let mut a = set_profile_args("Prodigal at the Hall", None, 1_760_000_000_000, None, None, None, None);
            if let Some(u) = url {
                a.insert("ticketUrl".into(), ArgVal::Text(u.into()));
            }
            a
        };
        let reduce = |a: &Args| {
            let members = [owner()];
            let ctx = ReduceContext { members: &members, owner: owner(), epoch: 0 };
            let mut st = EventState::default();
            let op = Op { op_id: OP_SET_PROFILE, args: a, author: &owner(), pos: None, ctx: &ctx };
            EventType::reduce(&mut st, &op).map(|()| st)
        };

        let st = reduce(&profile(Some("https://tickets.example/prodigal"))).expect("an https link is stored");
        assert_eq!(st.ticket_url, "https://tickets.example/prodigal");
        assert_eq!(reduce(&profile(Some("HTTP://tickets.example/x"))).unwrap().ticket_url, "HTTP://tickets.example/x",
            "the scheme is matched without regard to case, and stored as given");
        assert_eq!(reduce(&profile(None)).unwrap().ticket_url, "", "absent is no link");

        for bad in ["javascript:alert(1)", "data:text/html,hi", "tickets.example/x", "ftp://tickets.example/x"] {
            assert_eq!(reduce(&profile(Some(bad))).err(), Some(DeltaRejection::MalformedArgs), "{bad} is refused");
        }
        let long = format!("https://t.example/{}", "x".repeat(crate::post::MAX_LINK_CHARS));
        assert_eq!(reduce(&profile(Some(&long))).err(), Some(DeltaRejection::MalformedArgs), "capped as a post's link is");

        // And through the fold, the link is part of the profile it came with.
        let st = folded(vec![(OP_SET_PROFILE, profile(Some("https://tickets.example/prodigal")))], vec![]);
        assert_eq!(st.ticket_url, "https://tickets.example/prodigal");
    }

    fn priced_listing() -> (u32, Args) {
        (
            OP_SET_TICKETS,
            set_tickets_args(
                1200,
                "usd",
                3,
                true,
                Some("acct_1"),
                Some(&delegate()),
                None,
                1,
            ),
        )
    }

    fn sale(pi: &str, qty: u32, tickets: &[&str]) -> Args {
        let ids: Vec<String> = tickets.iter().map(|s| s.to_string()).collect();
        record_sale_args(pi, &stranger(), qty, 1200, 10 * qty as i64, "usd", &ids)
    }

    #[test]
    fn an_event_with_a_priced_listing_folds() {
        let st = folded(
            vec![
                (
                    OP_SET_PROFILE,
                    set_profile_args(
                        "Launch party",
                        Some("closing night"),
                        1_800_000_000_000,
                        None,
                        Some("ARC Gangnam"),
                        None,
                        None,
                    ),
                ),
                priced_listing(),
            ],
            vec![],
        );
        assert_eq!(st.title, "Launch party");
        assert_eq!(st.venue, "ARC Gangnam");
        let l = st.listing.expect("listing");
        assert_eq!(l.price_cents, 1200);
        assert_eq!(l.capacity, 3);
        assert!(l.open);
        assert_eq!(l.delegate, Some(delegate()));
        assert!(st.sellers.contains(&owner()) && st.sellers.contains(&delegate()));
    }

    /// setVenue folds a Place reference, replaces it LWW on the spine, and clears on
    /// an empty `place` — all without touching the free-text profile venue.
    #[test]
    fn set_venue_folds_replaces_and_clears() {
        let profile = (
            OP_SET_PROFILE,
            set_profile_args(
                "Launch party",
                None,
                1_800_000_000_000,
                None,
                Some("TBD"),
                None,
                None,
            ),
        );
        let place_a = "ab".repeat(32);
        let place_b = "cd".repeat(32);

        let st = folded(
            vec![
                profile.clone(),
                (
                    OP_SET_VENUE,
                    set_venue_args(Some(&place_a), "ARC Gangnam", 500),
                ),
            ],
            vec![],
        );
        assert_eq!(
            st.venue_ref,
            Some(VenueRef {
                place: place_a.clone(),
                name: "ARC Gangnam".into(),
                at: 500
            })
        );
        assert_eq!(
            st.venue, "TBD",
            "the profile's free-text venue is a separate facet"
        );

        let st = folded(
            vec![
                profile.clone(),
                (
                    OP_SET_VENUE,
                    set_venue_args(Some(&place_a), "ARC Gangnam", 500),
                ),
                (OP_SET_VENUE, set_venue_args(Some(&place_b), "Monroe", 600)),
            ],
            vec![],
        );
        assert_eq!(
            st.venue_ref.as_ref().map(|v| v.place.as_str()),
            Some(place_b.as_str())
        );

        let st = folded(
            vec![
                profile,
                (
                    OP_SET_VENUE,
                    set_venue_args(Some(&place_a), "ARC Gangnam", 500),
                ),
                (OP_SET_VENUE, set_venue_args(None, "", 0)),
            ],
            vec![],
        );
        assert!(st.venue_ref.is_none(), "empty place clears the reference");
    }

    /// A malformed venue reference is refused whole — never half-applied.
    #[test]
    fn set_venue_rejects_malformed() {
        let ctx = ReduceContext {
            members: &[owner()],
            owner: owner(),
            epoch: 0,
        };
        for bad in [
            set_venue_args(Some("not-hex"), "X", 500), // not a 64-char object id
            set_venue_args(Some(&"ab".repeat(32)), "", 500), // no name
            set_venue_args(Some(&"ab".repeat(32)), "X", 0), // no event time
        ] {
            let mut st = EventState::default();
            let op = Op {
                op_id: OP_SET_VENUE,
                args: &bad,
                author: &owner(),
                pos: None,
                ctx: &ctx,
            };
            assert_eq!(
                EventType::reduce(&mut st, &op),
                Err(DeltaRejection::MalformedArgs)
            );
            assert!(
                st.venue_ref.is_none(),
                "a rejected venue must not half-apply"
            );
        }
    }

    /// Paid fulfilment lives at the Arc: a priced listing with no rail account or no
    /// delegate is a misconfiguration, refused loudly.
    #[test]
    fn a_priced_listing_requires_acct_and_delegate() {
        let mut st = EventState::default();
        let ctx = ReduceContext {
            members: &[owner()],
            owner: owner(),
            epoch: 0,
        };
        for bad in [
            set_tickets_args(1200, "usd", 3, true, None, Some(&delegate()), None, 1),
            set_tickets_args(1200, "usd", 3, true, Some("acct_1"), None, None, 1),
        ] {
            let op = Op {
                op_id: OP_SET_TICKETS,
                args: &bad,
                author: &owner(),
                pos: None,
                ctx: &ctx,
            };
            assert_eq!(
                EventType::reduce(&mut st, &op),
                Err(DeltaRejection::PreconditionFailed)
            );
            assert!(
                st.listing.is_none(),
                "a rejected listing must not half-apply"
            );
        }
        // Free events need neither.
        let free = set_tickets_args(0, "usd", 10, true, None, None, None, 1);
        let op = Op {
            op_id: OP_SET_TICKETS,
            args: &free,
            author: &owner(),
            pos: None,
            ctx: &ctx,
        };
        assert_eq!(EventType::reduce(&mut st, &op), Ok(()));
    }

    /// The fold accepts a sale only from an authorized seller — the money invariant.
    #[test]
    fn only_the_delegate_or_owner_may_record_a_sale() {
        let st = folded(
            vec![priced_listing()],
            vec![(OP_RECORD_SALE, sale("pi_1", 1, &["t1"]), delegate(), 0)],
        );
        assert_eq!(st.ledger.len(), 1);
        assert_eq!(st.sold(), 1);

        // A stranger's "sale" must not exist in state, no matter what client authored it.
        let mut s2 = st.clone();
        let ctx = ReduceContext {
            members: &[owner(), delegate(), stranger()],
            owner: owner(),
            epoch: 0,
        };
        let forged = sale("pi_2", 1, &["t2"]);
        let op = Op {
            op_id: OP_RECORD_SALE,
            args: &forged,
            author: &stranger(),
            pos: None,
            ctx: &ctx,
        };
        assert_eq!(
            EventType::reduce(&mut s2, &op),
            Err(DeltaRejection::Unauthorized)
        );
        assert_eq!(s2.ledger.len(), 1);
    }

    /// A handover must not void the sales the previous owner recorded (§10.2 of
    /// membership-through-mls.md). The seller a listing names is the owner who LISTED
    /// it, so the ledger — the money mirror — is the same before and after ownership
    /// moves. Judged by today's owner instead, every earlier sale would stop folding.
    #[test]
    fn a_handover_does_not_void_the_previous_owners_sales() {
        let mut c: Coordinator<EventType> = Coordinator::with_owners(
            vec![owner(), delegate(), stranger()],
            vec![(0, owner()), (2, stranger())],
        );
        let (op_id, a) = priced_listing();
        let listing = sequenced_delta(ObjectKind::Event.type_id() as u32, op_id, a, 0, 0, GENESIS_PREV);
        c.deliver(listing, owner()).expect("the owner of epoch 0 lists");
        let s = build_delta(ObjectKind::Event, OP_RECORD_SALE, sale("pi_1", 1, &["t1"]), 1, Some(0));
        c.deliver(s, owner()).expect("and sells at epoch 1");
        let st = c.state();
        assert_eq!(st.ledger.len(), 1, "the sale made while they owned it still stands");
        assert!(st.sellers.contains(&owner()), "the lister stays a seller");
    }

    /// Re-emission and crash-retry both re-author the same row: `pi` makes it a no-op
    /// for holders and a converging insert for late joiners.
    #[test]
    fn a_sale_is_idempotent_by_payment_reference() {
        let st = folded(
            vec![priced_listing()],
            vec![
                (
                    OP_RECORD_SALE,
                    sale("pi_1", 2, &["t1", "t2"]),
                    delegate(),
                    0,
                ),
                (
                    OP_RECORD_SALE,
                    sale("pi_1", 2, &["t1", "t2"]),
                    delegate(),
                    1,
                ),
            ],
        );
        assert_eq!(st.ledger.len(), 1);
        assert_eq!(st.sold(), 2);
        assert_eq!(st.ledger["pi_1"].tickets.len(), 2);
    }

    /// The fee mirrors money movement: free sales carry none, priced sales must.
    /// A ticket id can be minted once, ever.
    #[test]
    fn fee_shape_and_ticket_uniqueness_are_enforced() {
        let mut st = folded(vec![priced_listing()], vec![]);
        let ctx = ReduceContext {
            members: &[owner(), delegate(), stranger()],
            owner: owner(),
            epoch: 0,
        };
        // Priced sale with fee 0 — refused.
        let no_fee = record_sale_args("pi_x", &stranger(), 1, 1200, 0, "usd", &["tx".into()]);
        let op = Op {
            op_id: OP_RECORD_SALE,
            args: &no_fee,
            author: &delegate(),
            pos: None,
            ctx: &ctx,
        };
        assert_eq!(
            EventType::reduce(&mut st, &op),
            Err(DeltaRejection::PreconditionFailed)
        );

        // Good sale, then a different pi re-minting the same ticket id — refused.
        let ok = sale("pi_1", 1, &["t1"]);
        let op = Op {
            op_id: OP_RECORD_SALE,
            args: &ok,
            author: &delegate(),
            pos: None,
            ctx: &ctx,
        };
        assert_eq!(EventType::reduce(&mut st, &op), Ok(()));
        let dup_ticket = sale("pi_2", 1, &["t1"]);
        let op = Op {
            op_id: OP_RECORD_SALE,
            args: &dup_ticket,
            author: &delegate(),
            pos: None,
            ctx: &ctx,
        };
        assert_eq!(
            EventType::reduce(&mut st, &op),
            Err(DeltaRejection::PreconditionFailed)
        );
        assert_eq!(st.ledger.len(), 1);
    }

    /// Capacity is flagged, never rejected: a paid buyer is not punished at the door
    /// for the seller's bug — but the fold says it saw the overrun, loudly.
    #[test]
    fn an_oversold_ledger_is_flagged_not_hidden() {
        let st = folded(
            vec![priced_listing()], // capacity 3
            vec![
                (
                    OP_RECORD_SALE,
                    sale("pi_1", 2, &["t1", "t2"]),
                    delegate(),
                    0,
                ),
                (
                    OP_RECORD_SALE,
                    sale("pi_2", 2, &["t3", "t4"]),
                    delegate(),
                    1,
                ),
            ],
        );
        assert_eq!(st.sold(), 4);
        assert!(st.over_capacity, "the overrun must be visible in state");
        // Every paid ticket still admits.
        assert_eq!(st.door_check("t4"), DoorVerdict::Valid);
    }

    /// Redemption is an OR-set: deterministic first admitter, idempotent per device,
    /// and the double-use flag means two DIFFERENT doors — not one door re-checking.
    #[test]
    fn redemption_converges_and_flags_double_use() {
        let base = vec![priced_listing()];
        let sales = vec![(OP_RECORD_SALE, sale("pi_1", 1, &["t1"]), delegate(), 0)];

        // Same device scanning twice: no flag.
        let mut once = folded(
            base.clone(),
            sales
                .iter()
                .cloned()
                .chain([
                    (OP_REDEEM, redeem_args("t1"), owner(), 1),
                    (OP_REDEEM, redeem_args("t1"), owner(), 2),
                ])
                .collect(),
        );
        match once.door_check("t1") {
            DoorVerdict::AlreadyRedeemed { double_scanned } => assert!(!double_scanned),
            v => panic!("expected AlreadyRedeemed, got {v:?}"),
        }

        // Two different doors: flagged, and the flag is order-independent.
        once = folded(
            base,
            sales
                .into_iter()
                .chain([
                    (OP_REDEEM, redeem_args("t1"), delegate(), 5),
                    (OP_REDEEM, redeem_args("t1"), owner(), 1),
                ])
                .collect(),
        );
        match once.door_check("t1") {
            DoorVerdict::AlreadyRedeemed { double_scanned } => assert!(double_scanned),
            v => panic!("expected AlreadyRedeemed, got {v:?}"),
        }
        assert_eq!(once.door_check("t_unknown"), DoorVerdict::Unknown);
    }

    /// The pairwise legs: per-pi LWW requests and a verbatim-core wallet.
    #[test]
    fn request_and_delivery_fold_on_the_contact_channel() {
        let buyer = stranger();
        let ctx = ReduceContext {
            members: &[buyer, delegate()],
            owner: buyer,
            epoch: 0,
        };

        let mut requests = BTreeMap::new();
        let mut a = request_ticket_args("lst1", 2, "pi_1");
        a.insert("gen".into(), ArgVal::Int(1));
        let op = Op {
            op_id: OP_REQUEST_TICKET,
            args: &a,
            author: &buyer,
            pos: None,
            ctx: &ctx,
        };
        assert_eq!(reduce_request_ticket(&mut requests, &op), Ok(()));
        // A stale replay never un-does the newer claim.
        let mut stale = request_ticket_args("lst1", 1, "pi_1");
        stale.insert("gen".into(), ArgVal::Int(0));
        let op = Op {
            op_id: OP_REQUEST_TICKET,
            args: &stale,
            author: &buyer,
            pos: None,
            ctx: &ctx,
        };
        assert_eq!(reduce_request_ticket(&mut requests, &op), Ok(()));
        assert_eq!(requests["pi_1"].qty, 2);

        // Delivery: the wallet holds the core VERBATIM.
        let id = test_identity(42);
        let pk = id.identity_pk();
        let core = TicketCore {
            v: 1,
            listing: "lst1".into(),
            ticket: "t1".into(),
            title: "Launch party".into(),
            start_ms: 1_800_000_000_000,
            buyer: hex::encode(buyer),
            issued_at: 1_754_000_000_000,
        };
        let (core_json, sig) = core.to_signed_json(&id);
        let mut wallet = BTreeMap::new();
        let mut d = deliver_ticket_args("lst1", &[(core_json.clone(), sig.clone())]);
        d.insert("gen".into(), ArgVal::Int(0));
        let op = Op {
            op_id: OP_DELIVER_TICKET,
            args: &d,
            author: &delegate(),
            pos: None,
            ctx: &ctx,
        };
        assert_eq!(reduce_deliver_ticket(&mut wallet, &op), Ok(()));
        assert_eq!(
            wallet["t1"].core, core_json,
            "the core must be stored byte-for-byte"
        );
        assert!(verify_ticket(&wallet["t1"].core, &wallet["t1"].sig, &pk).is_ok());
    }

    /// Sign → QR → scan → verify round-trips; a tampered core or wrong signer dies.
    #[test]
    fn the_credential_round_trips_and_tampering_dies() {
        let id = test_identity(42);
        let core = TicketCore {
            v: 1,
            listing: "lst1".into(),
            ticket: "t1".into(),
            title: "Launch party".into(),
            start_ms: 1_800_000_000_000,
            buyer: hex::encode(stranger()),
            issued_at: 1_754_000_000_000,
        };
        let (core_json, sig) = core.to_signed_json(&id);
        let qr = qr_payload(&core_json, &sig);
        let (scanned_core, scanned_sig) = parse_qr_payload(&qr).expect("QR parses");
        assert_eq!(
            scanned_core, core_json,
            "the QR must carry the core verbatim"
        );
        let verified =
            verify_ticket(&scanned_core, &scanned_sig, &id.identity_pk()).expect("verifies");
        assert_eq!(verified.ticket, "t1");

        let tampered = scanned_core.replace("t1", "t2");
        assert!(verify_ticket(&tampered, &scanned_sig, &id.identity_pk()).is_err());
        let wrong_signer = test_identity(99);
        assert!(verify_ticket(&scanned_core, &scanned_sig, &wrong_signer.identity_pk()).is_err());
    }

    /// Media folds, restates, and refuses oversize/mismatched args atomically.
    #[test]
    fn media_folds_within_caps_and_rejects_oversize() {
        let photo = EventPhoto {
            data: "aGVsbG8=".into(),
            mime: "image/jpeg".into(),
        };
        let st = folded(
            vec![
                (
                    OP_SET_MEDIA,
                    set_media_args(
                        Some(("cG9zdGVy", "image/jpeg")),
                        &[photo.clone()],
                        Some(("bW90aW9u", "video/mp4")),
                    ),
                ),
                // A later setMedia REPLACES the facet wholesale (LWW by sequence).
                (
                    OP_SET_MEDIA,
                    set_media_args(Some(("bmV3", "image/jpeg")), &[], None),
                ),
            ],
            vec![],
        );
        assert_eq!(st.media.banner, "bmV3");
        assert!(
            st.media.photos.is_empty(),
            "restatement replaces, never merges"
        );
        assert!(st.media.clip.is_empty());

        // Oversize banner: refused, nothing half-applied.
        let mut s = EventState::default();
        let ctx = ReduceContext {
            members: &[owner()],
            owner: owner(),
            epoch: 0,
        };
        let big = "x".repeat(MAX_BANNER_B64 + 1);
        let bad = set_media_args(Some((big.as_str(), "image/jpeg")), &[], None);
        let op = Op {
            op_id: OP_SET_MEDIA,
            args: &bad,
            author: &owner(),
            pos: None,
            ctx: &ctx,
        };
        assert_eq!(
            EventType::reduce(&mut s, &op),
            Err(DeltaRejection::MalformedArgs)
        );
        assert!(
            s.media.is_empty(),
            "a rejected media delta must not half-apply"
        );

        // A photo with data but no mime: refused.
        let bad_photo = vec![EventPhoto {
            data: "eA==".into(),
            mime: String::new(),
        }];
        let bad = set_media_args(None, &bad_photo, None);
        let op = Op {
            op_id: OP_SET_MEDIA,
            args: &bad,
            author: &owner(),
            pos: None,
            ctx: &ctx,
        };
        assert_eq!(
            EventType::reduce(&mut s, &op),
            Err(DeltaRejection::MalformedArgs)
        );

        // Too many photos: refused.
        let many: Vec<EventPhoto> = (0..=MAX_EVENT_PHOTOS).map(|_| photo.clone()).collect();
        let bad = set_media_args(None, &many, None);
        let op = Op {
            op_id: OP_SET_MEDIA,
            args: &bad,
            author: &owner(),
            pos: None,
            ctx: &ctx,
        };
        assert_eq!(
            EventType::reduce(&mut s, &op),
            Err(DeltaRejection::MalformedArgs)
        );
    }

    #[test]
    fn every_op_satisfies_the_spec_invariant() {
        for d in EventType::ops().iter().chain(CONTACT_TICKET_OPS) {
            assert!(
                d.is_well_formed(),
                "op {} violates the spec invariant",
                d.name
            );
        }
        assert_eq!(ObjectKind::Event.type_id(), 29);
        assert_eq!(ObjectKind::from_type_id(29), Some(ObjectKind::Event));
    }

    /// The base geo entries in OPS must stay byte-identical to geo's own table.
    #[test]
    fn base_location_ops_are_spliced_verbatim() {
        for base in geo::LOCATION_OPS {
            let spliced = EventType::ops()
                .iter()
                .find(|o| o.op_id == base.op_id)
                .expect("base op spliced");
            assert_eq!(spliced.name, base.name);
            assert_eq!(spliced.authority, base.authority);
            assert_eq!(spliced.commutativity, base.commutativity);
        }
    }
    /// The lineup folds in billing order, trims noise, and a rebroadcast without
    /// the arg clears it (profile is LWW-whole, not merge-per-field).
    #[test]
    fn a_lineup_folds_in_order_and_tolerates_noise() {
        let st = folded(
            vec![(
                OP_SET_PROFILE,
                set_profile_args(
                    "Late Night Special",
                    None,
                    1_785_412_800_000,
                    None,
                    Some("561 Wilson"),
                    None,
                    Some("DJ Slice\n  Margherita Quartet  \n\nOven Mitts"),
                ),
            )],
            vec![],
        );
        assert_eq!(st.lineup, vec!["DJ Slice", "Margherita Quartet", "Oven Mitts"]);
    }

    /// A repeat rule survives the fold and expands from the event's own start.
    #[test]
    fn a_recurring_event_folds_its_rule_and_expands() {
        let st = folded(
            vec![(
                OP_SET_PROFILE,
                set_profile_args(
                    "Thursday night",
                    None,
                    1_785_412_800_000,
                    None,
                    None,
                    Some("FREQ=WEEKLY;COUNT=3"),
                    None,
                ),
            )],
            vec![],
        );
        assert_eq!(
            st.recurrence, "FREQ=WEEKLY;COUNT=3",
            "canonicalised on the way in"
        );
        let occ = st.occurrences(0, i64::MAX / 2, 10);
        assert_eq!(occ.len(), 3, "three Thursdays, expanded on read");
        assert_eq!(
            occ[0], 1_785_412_800_000,
            "the first is the event's own start"
        );
        assert_eq!(st.repeat().unwrap().summary(), "Every week");
    }

    /// A one-off needs no special case at the call site.
    #[test]
    fn a_one_off_yields_only_itself() {
        let st = folded(
            vec![(
                OP_SET_PROFILE,
                set_profile_args("Once", None, 1_785_412_800_000, None, None, None, None),
            )],
            vec![],
        );
        assert!(st.recurrence.is_empty());
        assert_eq!(st.occurrences(0, i64::MAX / 2, 10), vec![1_785_412_800_000]);
    }

    /// A rule we cannot expand is REFUSED — the profile does not half-apply.
    #[test]
    fn an_unexpandable_rule_rejects_the_whole_profile() {
        let st = folded(
            vec![(
                OP_SET_PROFILE,
                set_profile_args(
                    "Bad",
                    None,
                    1_785_412_800_000,
                    None,
                    None,
                    Some("FREQ=HOURLY"),
                    None,
                ),
            )],
            vec![],
        );
        assert!(
            st.title.is_empty(),
            "the rejected op applied nothing at all"
        );
    }

    /// Capacity 0 means UNCAPPED, and must never read as sold out or overrun.
    #[test]
    fn an_uncapped_listing_never_sells_out() {
        let st = folded(
            vec![
                (
                    OP_SET_PROFILE,
                    set_profile_args("Free show", None, 1_800_000_000_000, None, None, None, None),
                ),
                (
                    OP_SET_TICKETS,
                    set_tickets_args(0, "gbp", 0, true, None, None, None, 0),
                ),
            ],
            vec![],
        );
        assert!(st.is_uncapped());
        assert_eq!(st.remaining(), None, "no limit is not 'zero left'");
        assert!(!st.over_capacity, "an uncapped listing cannot be oversold");
    }

    /// performs_at: an act is confirmed exactly where the reader holds the performer's own half,
    /// a person by member key or an organisation by object id; the rest stay, unconfirmed.
    #[test]
    fn an_act_is_confirmed_only_by_the_performers_own_half() {
        let acts = acts_of(&serde_json::json!([
            { "member": hex::encode(owner()), "role": "host" },
            { "object": "ab".repeat(32), "role": "headliner" },
            { "member": hex::encode(stranger()), "role": "support" },
        ]).to_string())
        .unwrap();
        let halves: BTreeSet<String> = [hex::encode(owner()), "ab".repeat(32)].into_iter().collect();
        assert_eq!(confirmed(&acts, &halves), vec![true, true, false]);
        assert_eq!(confirmed(&acts, &BTreeSet::new()), vec![false, false, false], "a reader holding no half confirms none");
        let v = acts_view(&acts, &halves);
        assert_eq!(v.as_array().unwrap().len(), 3, "unconfirmed acts are kept");
        assert_eq!(v[1]["object"], "ab".repeat(32));
        assert_eq!(v[2]["confirmed"], false);
    }
}
