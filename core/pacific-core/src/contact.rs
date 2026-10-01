//! contact — the Contact GroupObject: a 1:1 relationship channel (kind = Contact,
//! type_id 25) created by pairing. It is NOT a conversation: it carries no messages.
//! Its append-only Delta log — folded here — is the SOURCE OF TRUTH for the two
//! halves of a relationship:
//!
//! - the PREKEY POOL: the fresh, one-time MLS key packages each side stocks for the
//!   other so the peer can be added to Conversations/Forums instantly, with no live
//!   code exchange; and
//! - each side's SELF-PUBLISHED PROFILE: the name/card/avatar they choose to show
//!   THIS counterparty (`pair_scan` calls these the "who" and the "prekey" halves).
//!
//! Model (see the design thread): a Conversation (DM or group chat) is formed by
//! CONSUMING a prekey drawn from the relevant Contact. Prekeys therefore live ONLY
//! here — never on a Forum (the Project-linked thread) — which is the whole reason
//! Contact is its own object.
//!
//! The op-group is all AnyMember/Commutative — the prekey arm a tombstoned OR-set,
//! the profile arm a per-author LWW register:
//!
//! - `prekeySupply {kp_id, kp, not_after}` — offer one fresh key package.
//! - `prekeyConsume {kp_id}` — claim one to add its owner somewhere.
//! - `prekeyRevoke {kp_id}` — withdraw an unconsumed one (expiry / rotation).
//! - `publishProfile {displayName, shape, card, gen}` — announce MY profile to you.
//! - `setLink {active, gen}` — my own wish for our link (W-98), per-author LWW.
//!
//! Consume and revoke are terminal tombstones: a supply that folds AFTER a tombstone
//! for its `kp_id` is ignored, so the fold is order-independent (a consume that races
//! ahead of the supply that introduced it still wins). "You supply only your own key
//! packages" is a NODE-layer invariant (the kp's credential must equal the author);
//! the pure fold just records `from = author`.
//!
//! WHY PROFILE IS HERE AND NOT `group.setProfile`. They carry the same payload but
//! they are different speech acts, and the difference is authority:
//! - `group.setProfile` is Owner/Sequenced — "set the profile OF this shared
//!   identity record". Exactly one party may speak, because the record is one who.
//! - `contact.publishProfile` is AnyMember/Commutative — "here is MY OWN profile".
//!   A connection has TWO selves and no shared identity record, so an owner-only op
//!   is structurally wrong for it: the scanned side (never the owner of the
//!   connection group) could never describe itself.
//!
//! Folding per-author keeps both selves without either clobbering the other, and
//! makes the profile a per-pairwise-connection facet — the shape backlog item 1.4
//! ("contextual profiles": one key, many faces) needs, at no extra cost today.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::coordinator::{ArgVal, Args};
use crate::group::{ContactCard, GroupShape};
use crate::object::{
    Authority, Commutativity, DeltaRejection, MemberId, ObjectKind, ObjectType, Op, OpDecl,
};
use crate::object_args::{arg_b64, arg_hex32, opt_text_nonempty, req_int};

// ---- op ids (Contact's own op space, numbered from 0 like every other kind) ----
pub const OP_PREKEY_SUPPLY: u32 = 0; // any-member / commutative
pub const OP_PREKEY_CONSUME: u32 = 1; // any-member / commutative
pub const OP_PREKEY_REVOKE: u32 = 2; // any-member / commutative
pub const OP_PUBLISH_PROFILE: u32 = 3; // any-member / commutative (per-author LWW)
pub const OP_PUBLISH_LISTING: u32 = 4; // any-member / commutative (per-(origin,thing) LWW)
                                       // 5 and 6 are the TICKET legs — declared and reduced in `event.rs`
                                       // (`event::OP_REQUEST_TICKET` / `event::OP_DELIVER_TICKET`), spliced into
                                       // `CONTACT_OPS` below the way `place.rs` splices geo's LOCATION_OPS.
pub const OP_DISCOVER_REQUEST: u32 = 7; // any-member / commutative (per-(origin,rid), shortest path)
pub const OP_DISCOVER_RESPONSE: u32 = 8; // any-member / commutative (per-(origin,rid,holder) LWW)
/// `contact.setLink` (W-98): this side's own wish for the link. Member / commutative, per-author
/// LWW by gen. Read from the ICD at build.
pub const OP_SET_LINK: u32 = icd::OP_SET_LINK;

mod icd {
    include!(concat!(env!("OUT_DIR"), "/icd_links.rs"));
}

/// How far a `network` listing may travel from its origin. Every device enforces this at
/// FOLD time, not just when forwarding, so one modified client cannot flood the mesh: a
/// delta claiming more hops than this is rejected by every honest recipient.
///
/// Three is the reach of "a friend of a friend of a friend" — far enough that a village
/// finds a ladder, short enough that the graph stays legible and a listing does not
/// outrun the trust that carried it.
pub const MAX_LISTING_HOPS: u32 = 3;

/// How far a discovery request (and its answers) may travel from the asker. Two is
/// "my peers, and their peers": far enough to find the café your friend's friend
/// holds, short enough that the asker still has a human path to every answer.
/// Enforced at FOLD on every device, exactly like [`MAX_LISTING_HOPS`].
pub const MAX_DISCOVER_HOPS: u32 = 2;

/// How long a discovery request stays answerable/forwardable (origin-clock ms).
/// Discovery is a live question from an open sheet, not a standing subscription:
/// after this window every device simply stops reconciling it, and the rows age
/// out of the read without needing a retraction to chase them.
pub const DISCOVER_TTL_MS: i64 = 15 * 60 * 1000;

/// Inside the discovery TTL window? `now_ms == 0` skips the filter (tests,
/// forensic reads) — the ONE statement of the expiry rule.
fn within_ttl(at: i64, now_ms: i64) -> bool {
    now_ms == 0 || at + DISCOVER_TTL_MS > now_ms
}

/// The kinds a discovery QUESTION is asked for — what `Node::warm_gossip_caches`
/// re-asks on every app load. Trade's categories are absent on purpose: a listing
/// is PUSH gossip (`publishListing` reconciles on every sync), so the trade cache
/// warms itself without anyone asking.
pub const DISCOVER_KINDS: &[&str] = &["place"];

/// The path gate every relayed announcement shares, enforced at FOLD:
/// the path cannot be forged shorter (a direct copy is hops=0 by definition, a
/// relay is at least 1 — so only the origin may say 0, and the origin may say
/// nothing else), and past `ceiling` the copy has outrun the trust that carried
/// it. Used by the listing arm and the discovery-request arm; each states its own
/// ceiling (the network constant for listings, the origin's remaining budget for
/// requests).
fn check_relay_path(relayed: bool, hops: u32, ceiling: u32) -> Result<(), DeltaRejection> {
    if relayed == (hops == 0) || hops > ceiling {
        return Err(DeltaRejection::MalformedArgs);
    }
    Ok(())
}

/// The stable id of a key package = SHA-256 of its bytes. Deterministic, so both
/// sides name the same offer without coordination.
pub fn kp_id(kp: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(kp);
    h.finalize().into()
}

/// One offered key package in the pool. SELF-CONTAINED: carries everything needed to
/// add the offerer to a new group — the key package AND their intro-mailbox tag (so
/// the consumer can seal the Welcome to them without a fresh code exchange).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPackageOffer {
    /// The KP owner = the `prekeySupply` author. You consume a PEER's offers to add
    /// them; you never consume your own.
    pub from: [u8; 32],
    /// The MLS KeyPackage bytes (base64 on the wire); fed straight to `add_member`.
    pub kp: Vec<u8>,
    /// The offerer's intro-mailbox tag — where the Welcome for this add gets sealed.
    pub intro_tag: [u8; 32],
    /// MLS `not_after` lifetime bound (unix secs); 0 = unknown/none.
    pub not_after: u64,
}

/// The folded prekey pool for a Contact: unspent offers + a tombstone set.
#[derive(Clone, Default, Debug)]
pub struct PrekeyPool {
    /// kp_id -> offer, for offers not yet consumed or revoked.
    pub available: BTreeMap<[u8; 32], KeyPackageOffer>,
    /// consumed ∪ revoked — terminal; suppresses any later supply of the same id.
    pub spent: BTreeSet<[u8; 32]>,
}

/// One member's SELF-published profile, as they last announced it on this channel.
/// Authorship is not a field: it is the MLS-authenticated sender of the delta that
/// carried it (`mls::Incoming::Application { sender, .. }` — "the author is never on
/// the wire"), so a profile cannot be published in someone else's name.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SelfProfile {
    pub display_name: String,
    pub shape: GroupShape,
    pub card: ContactCard,
    /// The publisher's Lamport `gen` for this announcement — the LWW key, and the
    /// monotone revision the app shows/compares. Retained so a later fold can tell a
    /// re-announcement from a stale replay.
    pub gen: u64,
}

/// One market listing as it arrived on this channel — the market's unit of DISCOVERY.
///
/// A listing is not a Thing. A Thing is the object you own and mint locally; a listing is
/// the announcement of its posture, travelling to someone else. Keeping them apart is what
/// lets a listing be forwarded without handing over the object, and lets a recipient hold
/// ten thousand listings without minting ten thousand groups.
///
/// KEYED BY `(origin, thing_id)`, NOT BY AUTHOR. The same listing can reach you down two
/// paths — directly from Ada, and relayed by Ben who also knows her. Keying on the origin
/// collapses those to one row; keying on the author would show it twice and let the second
/// path overwrite the first with a longer one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    /// The identity that CREATED this listing. Preserved unchanged across every forward,
    /// so a listing is always attributed to its author, never to whoever passed it on.
    pub origin: MemberId,
    /// The origin's Thing object id. Unique per origin, so `(origin, thing_id)` is a
    /// network-wide identity for the listing without any coordinator.
    pub thing_id: String,
    pub posture: crate::thing::Posture,
    pub title: String,
    pub descriptor: String,
    /// Only ever set on an ACTIVE posture (buying/selling) — the same rule `thing` enforces.
    pub price: Option<String>,
    /// Epoch ms the posture lapses; `None` = standing.
    pub deadline: Option<i64>,
    /// Coarse geohash cell, `None` when geography sits out of routing.
    pub area: Option<String>,
    /// Whether this may be forwarded again. `Private` listings arrive only from their
    /// origin and stop with you.
    pub reach: crate::thing::Reach,
    /// Forwards taken to reach me. 0 = straight from the origin, 1 = a friend passed it on.
    pub hops: u32,
    /// Who told ME — the MLS-authenticated author of the delta that carried it. On a direct
    /// publish this equals `origin`; on a relay it is the neighbour who forwarded, which is
    /// exactly the provenance that makes a stranger's listing trustworthy.
    pub via: MemberId,
    /// The ORIGIN's revision of this listing (not the delta envelope's per-author gen).
    /// It has to be the origin's, or two copies arriving down different paths could not be
    /// ordered against each other.
    pub rev: u64,
    /// A withdrawn listing is kept as a tombstone rather than deleted: the withdrawal has
    /// to be able to chase the listing down every path it travelled, and a row that is
    /// simply absent cannot beat a stale copy arriving later.
    pub withdrawn: bool,
    /// The listing's FACE — the Thing's photo, base64-in-delta, carried UNCHANGED by
    /// every forward (it is listing content, exactly like the title). "" = the origin
    /// listed without a picture; the buyer's card renders stripes, never a stand-in.
    pub photo: String,
    /// MIME of `photo`; "" when no photo.
    pub photo_mime: String,
}

/// A question travelling the graph: "list your GroupObjects of `kind`, matching `tags`".
///
/// The PULL twin of [`Listing`]'s push gossip, carried by the same rules: the ORIGIN
/// rides unchanged in the args, the forwarder is the MLS-authenticated delta author,
/// `(origin, rid)` is the network-wide identity, and the hop invariants are enforced at
/// fold so a patched client cannot flood. A request is immutable — two copies can differ
/// only in the path that carried them, so the fold keeps the shortest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoverRequest {
    /// The identity that ASKED. Preserved unchanged across every forward.
    pub origin: MemberId,
    /// The origin's request id (hex). `(origin, rid)` names this question network-wide.
    pub rid: String,
    /// The directory kind being asked for ("place", …).
    pub kind: String,
    /// Free-text tags the responder matches against its objects. Empty = everything
    /// of `kind` the responder is willing to share.
    pub tags: Vec<String>,
    /// Forwards taken to reach me. 0 = straight from the asker.
    pub hops: u32,
    /// The TOTAL hop allowance the origin granted: 1 = my peers only, 2 = their peers
    /// too. `hops == budget` means answer but do not forward.
    pub budget: u32,
    /// Origin-clock ms — the TTL anchor. After [`DISCOVER_TTL_MS`] the question expires
    /// everywhere at once, which is what lets it need no retraction.
    pub at: i64,
    /// Who handed it to ME (the delta author). Also the channel the answers flow back
    /// down — "the forward is the introduction".
    pub via: MemberId,
}

/// One object in a discovery answer — a REFERENCE row, not the object. Holding one
/// grants nothing: it is a name, a kind and a point, enough to render a row and to
/// mint a local counterpart if the user publishes there (the RA reference-mode shape).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoverItem {
    /// The HOLDER's object id (hex) — meaningful in their store, provenance in ours.
    pub id: String,
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub descriptor: String,
    #[serde(default)]
    pub lat_e7: Option<i32>,
    #[serde(default)]
    pub lng_e7: Option<i32>,
    /// The holder's visibility dial for this object AT ANSWER TIME — consent that
    /// travels with the row, so every device between holder and asker can enforce
    /// it: a relay strips items whose reach the next leg would exceed, and the
    /// fold rejects an answer that arrives past an item's reach. Missing (an old
    /// build) deserializes to `Private`, which can never travel further than the
    /// holder meant.
    #[serde(default)]
    pub vis: crate::visibility::Visibility,
}

/// One holder's complete answer to one request, as it reached me on this channel.
/// Keyed `(origin, rid, holder)`: the same holder's answer down two paths collapses
/// to one row, and a re-answer (their public set changed) replaces it wholesale.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoverResponse {
    /// The ASKER these rows are for — responses only ever travel toward them.
    pub origin: MemberId,
    pub rid: String,
    /// Whose objects these are. On a direct answer this equals the delta author; on a
    /// relay it is the friend-of-friend the intermediary is vouching for.
    pub holder: MemberId,
    pub items: Vec<DiscoverItem>,
    /// The holder's distance from the origin: 1 = a direct peer answered, 2 = an
    /// intermediary relayed a friend-of-friend's answer.
    pub hops: u32,
    /// Holder-clock ms of the answer — the LWW key for re-answers.
    pub at: i64,
    /// The MLS-authenticated author of the delta that carried it.
    pub via: MemberId,
}

/// The folded state of a Contact channel: the prekey pool, every member's self-published
/// profile, AND the listings that reached me here. Three arms of ONE op-group over ONE log
/// — the same "one log, many lenses" discipline `folded_forum`/`folded_group` apply across
/// op-groups, applied here within one.
#[derive(Clone, Default, Debug)]
pub struct ContactState {
    /// WHAT THE OTHER PARTY IS TO US. The standing facet (`crate::roles`), which
    /// is what collapsed `member-tether` and `arc-tether` back into `connection`:
    /// those kinds existed only because a role could not ride a Contact-folded
    /// log, and now it can.
    pub roles: std::collections::BTreeMap<crate::object::MemberId, crate::group::GroupRole>,
    pub prekeys: PrekeyPool,
    /// member identity pubkey -> the profile THEY published. Per-author, so the two
    /// sides of a connection never clobber each other.
    pub profiles: BTreeMap<MemberId, SelfProfile>,
    /// `(origin, thing_id)` -> the listing. See `Listing` for why the key is the origin
    /// rather than the author.
    pub listings: BTreeMap<(MemberId, String), Listing>,
    /// `pi` -> the ticket request this channel carries (buyer→seller leg). Authored
    /// BEFORE the payment page opens, so every paid session has a request to find.
    pub ticket_requests: BTreeMap<String, crate::event::TicketRequest>,
    /// ticket id -> the delivered credential (seller→buyer leg). Cores are VERBATIM.
    pub wallet: BTreeMap<String, crate::event::WalletTicket>,
    /// `(author, event_id)` -> an invitation this channel carries. Keyed by AUTHOR so
    /// each side owns its own slot: an invite and its reply cannot overwrite one
    /// another, and both sides may invite each other to different things.
    pub invites: BTreeMap<(MemberId, String), crate::event::Invite>,
    /// `(author, event_id)` -> the answer. Its own map for the same reason.
    pub invite_replies: BTreeMap<(MemberId, String), crate::event::InviteReply>,
    /// `(origin, rid)` -> the live question this channel carries. The fold is also the
    /// dedup ledger: "have I already forwarded this into that channel?" is answered by
    /// the target channel's own copy of this map, so discovery needs no side table.
    pub discover_requests: BTreeMap<(MemberId, String), DiscoverRequest>,
    /// `(origin, rid, holder)` -> that holder's answer as carried here.
    pub discover_responses: BTreeMap<(MemberId, String, MemberId), DiscoverResponse>,
    /// What this object is a part of, and in what role (`base.setParent`). `None`
    /// until a parent names it as a part.
    pub parent: Option<crate::parent::ParentRef>,
    /// member -> their own wish for the link (`contact.setLink`). Per-author, so each side
    /// states only its own.
    pub links: BTreeMap<MemberId, LinkWish>,
}

/// One side's wish for the link (`contact.setLink`), the highest gen that side wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkWish {
    pub active: bool,
    pub gen: u64,
}

/// The link between the two sides, from one side (the ICD's summary of `contact.setLink`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkState {
    /// Neither side has stated a wish.
    None,
    /// My active is 1, theirs unstated.
    Invited,
    /// Theirs is 1, mine unstated.
    InvitedMe,
    /// Both 1.
    Linked,
    /// Either side's 0: withdrawn, declined or ended.
    Ended,
}

impl LinkState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Invited => "invited",
            Self::InvitedMe => "invited_me",
            Self::Linked => "linked",
            Self::Ended => "ended",
        }
    }
}

impl ContactState {
    /// `reader`'s wish, the other side's, and the link they make, from `reader`'s side.
    pub fn link_from(&self, reader: &MemberId) -> (Option<bool>, Option<bool>, LinkState) {
        let mine = self.links.get(reader).map(|w| w.active);
        let theirs = self
            .links
            .iter()
            .filter(|(who, _)| *who != reader)
            .max_by_key(|(who, w)| (w.gen, **who))
            .map(|(_, w)| w.active);
        let state = match (mine, theirs) {
            (Some(false), _) | (_, Some(false)) => LinkState::Ended,
            (Some(true), Some(true)) => LinkState::Linked,
            (Some(true), None) => LinkState::Invited,
            (None, Some(true)) => LinkState::InvitedMe,
            (None, None) => LinkState::None,
        };
        (mine, theirs, state)
    }

    /// Whether `member`'s own active is 1.
    pub fn wants_link(&self, member: &MemberId) -> bool {
        self.links.get(member).is_some_and(|w| w.active)
    }

    /// The profile `peer` published on this channel, if they have published one.
    pub fn profile_of(&self, peer: &MemberId) -> Option<&SelfProfile> {
        self.profiles.get(peer)
    }

    /// Live listings on this channel — withdrawn tombstones and lapsed deadlines excluded.
    /// `now_ms` is the caller's clock; pass 0 to skip the deadline filter entirely.
    pub fn live_listings(&self, now_ms: i64) -> Vec<&Listing> {
        self.listings
            .values()
            .filter(|l| !l.withdrawn)
            .filter(|l| now_ms == 0 || l.deadline.is_none_or(|d| d > now_ms))
            .collect()
    }

    /// Discovery questions still inside their TTL window on this channel. `now_ms` is
    /// the caller's clock; pass 0 to skip the expiry filter (tests, forensic reads).
    pub fn live_discover_requests(&self, now_ms: i64) -> Vec<&DiscoverRequest> {
        self.discover_requests
            .values()
            .filter(|r| within_ttl(r.at, now_ms))
            .collect()
    }

    /// Answers still inside their question's TTL window on this channel.
    pub fn live_discover_responses(&self, now_ms: i64) -> Vec<&DiscoverResponse> {
        self.discover_responses
            .values()
            .filter(|r| within_ttl(r.at, now_ms))
            .collect()
    }

    /// The CACHEABLE answers on this channel: every FULL answer to `origin`'s
    /// untagged questions, out to the network ceiling (n-2: friends and their
    /// friends) — no TTL. An answer is durable knowledge about the graph (like a
    /// listing, which also outlives the moment it arrived), so it keeps rendering
    /// between sessions until the holder answers again; relays carry the HOLDER's
    /// own clock, so a newer answer beats an older one down any path.
    ///
    /// Tagged answers never cache: a tagged answer is a SUBSET (the responder
    /// filtered), so keeping it as "what this holder has" would shrink the shelf
    /// after every filtered search. They still surface through
    /// [`Self::live_discover_responses`] while their question lives.
    ///
    /// This is the FILTER only. Which of a holder's answers wins — newest across
    /// every rid and every channel — is the store's call
    /// (`GroupObjectStore::by_kind_hops`), because one channel cannot see another.
    pub fn cacheable_discover_answers(&self, origin: &MemberId) -> Vec<&DiscoverResponse> {
        self.discover_responses
            .values()
            .filter(|r| r.origin == *origin && r.holder != *origin)
            .filter(|r| {
                // The question this answered rides the same fold (we authored it
                // into this channel) — only an untagged question's answer caches.
                self.discover_requests
                    .get(&(r.origin, r.rid.clone()))
                    .is_some_and(|req| req.tags.is_empty())
            })
            .collect()
    }

    /// Listings I may pass on: `network` reach, below the hop ceiling, and either still live
    /// or a WITHDRAWAL.
    ///
    /// Withdrawals are deliberately included even though `live_listings` excludes them. A
    /// tombstone has to travel the same paths the listing did, or a sold bike stays on sale
    /// forever two hops out — the relay must forward the news that it is gone, not merely
    /// stop forwarding the news that it exists. A listing that simply LAPSED needs no such
    /// chase: every recipient's own clock retires it.
    pub fn forwardable(&self, now_ms: i64) -> Vec<&Listing> {
        self.listings
            .values()
            .filter(|l| l.reach == crate::thing::Reach::Network && l.hops < MAX_LISTING_HOPS)
            .filter(|l| l.withdrawn || now_ms == 0 || l.deadline.is_none_or(|d| d > now_ms))
            .collect()
    }
}

impl PrekeyPool {
    /// Offers FROM `peer` that are neither spent nor expired at `now` (secs).
    pub fn usable_from(&self, peer: &[u8; 32], now: u64) -> Vec<([u8; 32], KeyPackageOffer)> {
        self.available
            .iter()
            .filter(|(_, o)| &o.from == peer && (o.not_after == 0 || o.not_after > now))
            .map(|(id, o)| (*id, o.clone()))
            .collect()
    }

    /// How many usable offers we hold from `peer` — the stock-level for replenishment.
    pub fn count_from(&self, peer: &[u8; 32], now: u64) -> usize {
        self.usable_from(peer, now).len()
    }

    /// Pick one usable offer from `peer` to consume (deterministic: lowest kp_id).
    /// Returns (kp_id, key_package, offerer's intro_tag) — everything needed to add them.
    pub fn pick_from(&self, peer: &[u8; 32], now: u64) -> Option<([u8; 32], Vec<u8>, [u8; 32])> {
        self.usable_from(peer, now)
            .into_iter()
            .next()
            .map(|(id, o)| (id, o.kp, o.intro_tag))
    }
}

// ---- arg builders (node authors deltas through these) ------------------------

/// Build the args for `prekeySupply`. `kp_id` is derived from the bytes so callers
/// can't desync the id from the payload.
/// THE LEGS THAT MAY RIDE A PAIRWISE CHANNEL, and nothing else.
///
/// A connection's log must never become a place to author arbitrary ops: it is
/// the pairing channel, and its other traffic — prekeys, profile and listing
/// fan-out — is written by the pairing and sync paths, which are not authoring
/// doors and take no caller's op id. These five are the exception, because both
/// halves of a ticket and both halves of an invitation have nowhere else to go
/// (a guest never joins the Event GroupObject). And `setLink`: each side's wish for
/// the link is the connection's own, written through the one write path.
pub fn is_pairwise_leg(op_id: u32) -> bool {
    matches!(
        op_id,
        crate::event::OP_REQUEST_TICKET
            | crate::event::OP_DELIVER_TICKET
            | crate::event::OP_ADMIT_TICKET
            | crate::event::OP_INVITE
            | crate::event::OP_INVITE_REPLY
            | OP_SET_LINK
    )
}

/// The args for `setLink`: this side's wish. `gen` is the write path's (`authoring::build`).
pub fn set_link_args(active: bool) -> Args {
    let mut a = Args::new();
    a.insert("active".into(), ArgVal::Int(active as i64));
    a
}

pub fn supply_args(kp: &[u8], intro_tag: &[u8; 32], not_after: u64) -> Args {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    let mut a = Args::new();
    a.insert("kp_id".into(), ArgVal::Text(hex::encode(kp_id(kp))));
    a.insert("kp".into(), ArgVal::Text(B64.encode(kp)));
    a.insert("intro_tag".into(), ArgVal::Text(hex::encode(intro_tag)));
    a.insert("not_after".into(), ArgVal::Int(not_after as i64));
    a
}

/// Build the args for `prekeyConsume`/`prekeyRevoke` (same shape: just the id).
pub fn id_args(kp_id: &[u8; 32]) -> Args {
    let mut a = Args::new();
    a.insert("kp_id".into(), ArgVal::Text(hex::encode(kp_id)));
    a
}

/// Build the args for `publishProfile`. The card rides as ONE json `card` arg
/// because the envelope's arg map is `str -> int|text` — the same lowering
/// `group.setProfile` uses, so both ops speak the identical card wire shape and a
/// card can move between them without re-encoding.
///
/// `gen` is the author's Lamport stamp; the node assigns it from
/// `author_delta_count` so it is monotone per author, which is exactly what makes
/// the LWW deterministic.
pub fn publish_profile_args(
    display_name: &str,
    shape: GroupShape,
    card: &ContactCard,
    gen: u64,
) -> Result<Args, serde_json::Error> {
    let mut a = Args::new();
    a.insert("displayName".into(), ArgVal::Text(display_name.to_string()));
    a.insert("shape".into(), ArgVal::Text(shape.as_str().to_string()));
    a.insert("card".into(), ArgVal::Text(serde_json::to_string(card)?));
    a.insert("gen".into(), ArgVal::Int(gen as i64));
    Ok(a)
}

/// Build the args for `publishListing`.
///
/// `rev` is the ORIGIN's revision of this listing, and it must be carried unchanged by
/// every forwarder — it is the key the cross-path LWW resolves on. `hops` is 0 for a direct
/// publish and incremented by each relay.
#[allow(clippy::too_many_arguments)]
pub fn publish_listing_args(
    origin: &MemberId,
    thing_id: &str,
    posture: crate::thing::Posture,
    title: &str,
    descriptor: &str,
    price: Option<&str>,
    deadline: Option<i64>,
    area: Option<&str>,
    reach: crate::thing::Reach,
    hops: u32,
    rev: u64,
    withdrawn: bool,
    // photo: the Thing's (photo b64, mime) — `None` when the origin listed without one.
    photo: Option<(&str, &str)>,
) -> Args {
    let mut a = Args::new();
    a.insert("origin".into(), ArgVal::Text(hex::encode(origin)));
    a.insert("thingId".into(), ArgVal::Text(thing_id.to_string()));
    a.insert("posture".into(), ArgVal::Text(posture.as_str().to_string()));
    a.insert("title".into(), ArgVal::Text(title.to_string()));
    a.insert("descriptor".into(), ArgVal::Text(descriptor.to_string()));
    if let Some(p) = price {
        a.insert("price".into(), ArgVal::Text(p.to_string()));
    }
    a.insert("deadline".into(), ArgVal::Int(deadline.unwrap_or(0)));
    if let Some(area) = area {
        a.insert("area".into(), ArgVal::Text(area.to_string()));
    }
    a.insert("reach".into(), ArgVal::Text(reach.as_str().to_string()));
    a.insert("hops".into(), ArgVal::Int(hops as i64));
    a.insert("rev".into(), ArgVal::Int(rev as i64));
    a.insert("withdrawn".into(), ArgVal::Int(withdrawn as i64));
    if let Some((photo, mime)) = photo {
        a.insert("photo".into(), ArgVal::Text(photo.to_string()));
        a.insert("photoMime".into(), ArgVal::Text(mime.to_string()));
    }
    a
}

/// Build the args for `discoverRequest`. `tags` rides as ONE json text arg — the arg
/// map is `str -> int|text`, the same lowering `card` and `claim` use.
pub fn discover_request_args(
    origin: &MemberId,
    rid: &str,
    kind: &str,
    tags: &[String],
    hops: u32,
    budget: u32,
    at: i64,
) -> Args {
    let mut a = Args::new();
    a.insert("origin".into(), ArgVal::Text(hex::encode(origin)));
    a.insert("rid".into(), ArgVal::Text(rid.to_string()));
    a.insert("kind".into(), ArgVal::Text(kind.to_string()));
    a.insert(
        "tags".into(),
        ArgVal::Text(serde_json::to_string(tags).expect("Vec<String> always serializes")),
    );
    a.insert("hops".into(), ArgVal::Int(hops as i64));
    a.insert("budget".into(), ArgVal::Int(budget as i64));
    a.insert("at".into(), ArgVal::Int(at));
    a
}

/// Build the args for `discoverResponse`. `items` is the holder's WHOLE answer as one
/// json text arg — atomic, so a re-answer replaces the set rather than merging rows.
pub fn discover_response_args(
    origin: &MemberId,
    rid: &str,
    holder: &MemberId,
    items: &[DiscoverItem],
    hops: u32,
    at: i64,
) -> Args {
    let mut a = Args::new();
    a.insert("origin".into(), ArgVal::Text(hex::encode(origin)));
    a.insert("rid".into(), ArgVal::Text(rid.to_string()));
    a.insert("holder".into(), ArgVal::Text(hex::encode(holder)));
    a.insert(
        "items".into(),
        ArgVal::Text(serde_json::to_string(items).expect("DiscoverItem always serializes")),
    );
    a.insert("hops".into(), ArgVal::Int(hops as i64));
    a.insert("at".into(), ArgVal::Int(at));
    a
}

// ---- the op table + ObjectType impl -----------------------------------------

static CONTACT_OPS: &[OpDecl] = &[
    // The BASE standing op-group, spliced — see `crate::roles`. A connection can
    // carry what the other party is to us, which is what made the tether kinds
    // unnecessary.
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
        op_id: OP_PREKEY_SUPPLY,
        name: "contact.prekeySupply",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_PREKEY_CONSUME,
        name: "contact.prekeyConsume",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_PREKEY_REVOKE,
        name: "contact.prekeyRevoke",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_PUBLISH_PROFILE,
        name: "contact.publishProfile",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_PUBLISH_LISTING,
        name: "contact.publishListing",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    // The two ticket legs, spliced from `event::CONTACT_TICKET_OPS`. Written out
    // rather than concatenated (a `static` cannot feed a const initialiser);
    // `ticket_ops_are_spliced_verbatim` below pins these copies to event's table.
    OpDecl {
        op_id: crate::event::OP_REQUEST_TICKET,
        name: "contact.requestTicket",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: crate::event::OP_ADMIT_TICKET,
        name: "contact.admitTicket",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: crate::event::OP_DELIVER_TICKET,
        name: "contact.deliverTicket",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    // The invite legs, spliced from event.rs — same pattern as the ticket legs: a
    // guest never joins the Event GroupObject, so the invitation rides the pairwise
    // channel the two of you already share.
    OpDecl {
        op_id: crate::event::OP_INVITE,
        name: "contact.invite",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: crate::event::OP_INVITE_REPLY,
        name: "contact.inviteReply",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_DISCOVER_REQUEST,
        name: "contact.discoverRequest",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_DISCOVER_RESPONSE,
        name: "contact.discoverResponse",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_SET_LINK,
        name: "contact.setLink",
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

/// The Contact object type (#25) — the prekey-pool + self-profile op-group folded
/// through the trait.
pub struct ContactType;


/// Bound one media slot that arrived from another peer.
///
/// Size and structure only — see [`crate::media::MediaRef::validate_bounds`] for
/// why a fold must not police the container format of a delta it did not author.
fn foreign_media_exceeds(data: &str, mime: &str, slot: crate::media::Slot) -> bool {
    let m = crate::media::MediaRef {
        // Kind is not consulted by `validate_bounds`; a placeholder keeps us from
        // inventing a judgement about a container we may not recognise.
        kind: crate::media::MediaKind::Still,
        mime: mime.to_string(),
        delivery: crate::media::Delivery::Inline {
            data: data.to_string(),
        },
        width: 0,
        height: 0,
        duration_ms: 0,
    };
    m.validate_bounds(slot).is_err()
}

impl ObjectType for ContactType {
    const KIND: ObjectKind = ObjectKind::Contact;
    type State = ContactState;

    fn ops() -> &'static [OpDecl] {
        CONTACT_OPS
    }

    /// Tombstoned OR-set (prekeys) + per-author LWW register (profiles).
    ///
    /// The prekey arm is order-independent because `supply` checks `spent` and
    /// `consume`/`revoke` both tombstone AND remove from `available` — so whichever
    /// order the fold delivers them, `available` ends without the id and `spent` ends
    /// with it.
    ///
    /// The profile arm is order-independent because the coordinator folds the
    /// commutative set in canonical `(gen, author, id)` order, so one author's
    /// announcements always arrive oldest-first and the last write genuinely is the
    /// newest. The explicit `gen >=` guard makes that independent of the caller's
    /// ordering too, so a direct `reduce` (a test, a future re-fold) can't regress
    /// state by replaying an old announcement.
    ///
    /// Missing/ill-typed args self-reject (skipped by the fold).
    fn reduce(state: &mut ContactState, op: &Op<'_>) -> Result<(), DeltaRejection> {
        if crate::parent::is_parent_op(op.op_id) {
            return crate::parent::reduce_parent(&mut state.parent, op);
        }
        // Base ops first: a reserved id band, so this can never shadow a Contact op.
        if crate::roles::is_role_op(op.op_id) {
            return crate::roles::reduce_roles(&mut state.roles, op);
        }
        match op.op_id {
            OP_PREKEY_SUPPLY => {
                let id = arg_hex32(op.args, "kp_id")?;
                let kp = arg_b64(op.args, "kp")?;
                let intro_tag = arg_hex32(op.args, "intro_tag")?;
                let not_after = match crate::arg_reads::get(op.args, "not_after") {
                    Some(ArgVal::Int(n)) => *n as u64,
                    _ => 0,
                };
                if !state.prekeys.spent.contains(&id) {
                    state.prekeys.available.insert(
                        id,
                        KeyPackageOffer {
                            from: *op.author,
                            kp,
                            intro_tag,
                            not_after,
                        },
                    );
                }
                Ok(())
            }
            OP_PREKEY_CONSUME | OP_PREKEY_REVOKE => {
                let id = arg_hex32(op.args, "kp_id")?;
                state.prekeys.spent.insert(id);
                state.prekeys.available.remove(&id);
                Ok(())
            }
            OP_PUBLISH_PROFILE => {
                // Validate EVERY arg before the first mutation — same atomicity
                // contract as `group.setProfile`: a rejected announcement must leave
                // the previous profile intact rather than half-replace it.
                let display_name = match crate::arg_reads::get(op.args, "displayName") {
                    Some(ArgVal::Text(s)) => s.clone(),
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                let shape = match crate::arg_reads::get(op.args, "shape") {
                    Some(ArgVal::Text(s)) => GroupShape::parse(s)?,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                // An unknown-field-tolerant parse: `ContactCard` is all-`default`, so
                // a card from a NEWER peer carrying fields this build has never heard
                // of still folds — it keeps what it understands. That is what lets the
                // "last seen" clip slot ship to half a network at a time.
                let card = match crate::arg_reads::get(op.args, "card") {
                    Some(ArgVal::Text(json)) => serde_json::from_str::<ContactCard>(json)
                        .map_err(|_| DeltaRejection::MalformedArgs)?,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                // …but tolerant of FIELDS is not tolerant of SIZE: the card's STILL
                // slot is media and clears `media::Slot` at THIS fold too — the same
                // gate the listing's face clears in `publishListing` below. Without it
                // a patched peer could announce a photo of any size that every
                // connection folds and stores. Oversize is MalformedArgs, refused not
                // clamped: silent re-encoding by the engine would make two devices
                // disagree about the same delta.
                if foreign_media_exceeds(&card.photo, &card.photo_mime, crate::media::Slot::Avatar) {
                    return Err(DeltaRejection::MalformedArgs);
                }
                // THE CLIP DOES NOT RIDE THIS ROUTE — refused outright, at any size.
                //
                // A profile edit authors one sealed delta PER CONNECTION
                // (`Node::publish_profile`): N separate MLS groups, N distinct
                // ciphertexts, nothing a relay can dedupe because no two copies are
                // the same bytes. A downscaled still is ~100KB of that. An inline clip
                // is up to `Slot::Clip`'s 1.5MB — the difference between one avatar
                // edit at 150 connections costing ~15MB and costing ~300MB, and the
                // same volume again landing in every recipient's append-only log.
                //
                // The SLOT stays on the card. Renderers still resolve clip → photo →
                // block colour, and a detached clip carries a 32-byte digest instead
                // of bytes (`media::Delivery::Detached`) — that is the route "last
                // seen" takes when it ships. What is cut is bytes-in-the-delta.
                //
                // Refused rather than stripped, and the whole card with it: a fold
                // that quietly dropped the clip would leave the sender believing it
                // crossed. `Node::set_my_profile` enforces the same rule at authoring,
                // so an honest build never reaches this line — whatever does has
                // patched the rule out, and it does not get to spend every
                // connection's bandwidth on it.
                if !card.clip.is_empty() || !card.clip_mime.is_empty() {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let gen = match crate::arg_reads::get(op.args, "gen") {
                    Some(ArgVal::Int(g)) if *g >= 0 => *g as u64,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                // Look up BEFORE inserting: an `entry().or_default()` here would
                // leave an empty profile behind on the stale-replay path, turning
                // "this peer has never published" into "this peer is nameless".
                if let Some(held) = state.profiles.get(op.author) {
                    if gen < held.gen {
                        return Ok(()); // a stale replay never un-does a newer announcement
                    }
                }
                state.profiles.insert(
                    *op.author,
                    SelfProfile {
                        display_name,
                        shape,
                        card,
                        gen,
                    },
                );
                Ok(())
            }
            OP_PUBLISH_LISTING => {
                // VALIDATE EVERYTHING, THEN COMMIT — same atomicity contract as the two ops
                // above: a malformed relay must leave the listing I already hold intact.
                let origin = arg_hex32(op.args, "origin")?;
                let thing_id = match crate::arg_reads::get(op.args, "thingId") {
                    Some(ArgVal::Text(s)) if !s.is_empty() => s.clone(),
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                let posture = match crate::arg_reads::get(op.args, "posture") {
                    Some(ArgVal::Text(s)) => crate::thing::Posture::parse(s)?,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                let title = match crate::arg_reads::get(op.args, "title") {
                    Some(ArgVal::Text(s)) => s.clone(),
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                let reach = match crate::arg_reads::get(op.args, "reach") {
                    Some(ArgVal::Text(s)) => crate::thing::Reach::parse(s)?,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                let hops = match crate::arg_reads::get(op.args, "hops") {
                    Some(ArgVal::Int(h)) if *h >= 0 => *h as u32,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                let rev = match crate::arg_reads::get(op.args, "rev") {
                    Some(ArgVal::Int(r)) if *r >= 0 => *r as u64,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };

                // ---- THE THREE RELAY INVARIANTS, ENFORCED AT FOLD ----
                //
                // Enforced HERE rather than only in the forwarding code because the fold is
                // the one gate every device runs on every delta. A patched client that
                // forwards a private listing, forges a shorter path, or floods past the hop
                // ceiling is rejected by every honest recipient, so the mesh's safety does
                // not depend on every peer running our build.
                let relayed = origin != *op.author;
                // 1. CONSENT TRAVELS. Only a `network` listing may be relayed at all;
                //    `private` means "my connections, and no further", so a delta whose
                //    author is not the origin can never carry one.
                if relayed && reach != crate::thing::Reach::Network {
                    return Err(DeltaRejection::MalformedArgs);
                }
                // 2 + 3. THE PATH CANNOT BE FORGED SHORTER, and THE CEILING IS
                //    ABSOLUTE — the shared gate (see `check_relay_path`). Without the
                //    first, a forwarder could claim hops=0 and beat the origin's own
                //    copy on the shortest-path tie-break below; beyond the second a
                //    listing has outrun the trust that carried it.
                check_relay_path(relayed, hops, MAX_LISTING_HOPS)?;

                let descriptor = match crate::arg_reads::get(op.args, "descriptor") {
                    Some(ArgVal::Text(s)) => s.clone(),
                    _ => String::new(),
                };
                // A price on a standing intent is meaningless — the same rule `thing`
                // enforces on the object itself, applied to its announcement so the two
                // can never disagree.
                let price = match crate::arg_reads::get(op.args, "price") {
                    Some(ArgVal::Text(s)) if !s.is_empty() => {
                        if !posture.is_active() {
                            return Err(DeltaRejection::MalformedArgs);
                        }
                        Some(s.clone())
                    }
                    _ => None,
                };
                let deadline = match crate::arg_reads::get(op.args, "deadline") {
                    Some(ArgVal::Int(d)) if *d > 0 => Some(*d),
                    _ => None,
                };
                let area = match crate::arg_reads::get(op.args, "area") {
                    Some(ArgVal::Text(s)) if !s.is_empty() => Some(s.clone()),
                    _ => None,
                };
                let withdrawn = matches!(crate::arg_reads::get(op.args, "withdrawn"), Some(ArgVal::Int(w)) if *w != 0);
                // The listing's face travels WITH it, and clears the same
                // `media::Slot::Avatar` gate at THIS fold, so a patched forwarder
                // cannot balloon the gossip.
                let photo = match crate::arg_reads::get(op.args, "photo") {
                    Some(ArgVal::Text(s)) => s.clone(),
                    None => String::new(),
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                let photo_mime = match crate::arg_reads::get(op.args, "photoMime") {
                    Some(ArgVal::Text(s)) => s.clone(),
                    None => String::new(),
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                // Size, mime/kind agreement and the both-or-neither rule are all
                // one call now; `HalfSet` is exactly the "picture with no type"
                // case this fold used to spell out by hand.
                if foreign_media_exceeds(&photo, &photo_mime, crate::media::Slot::Avatar) {
                    return Err(DeltaRejection::MalformedArgs);
                }

                // ---- LWW: newest revision wins; among equals, the SHORTEST path wins ----
                //
                // Both halves are needed. Revision ordering makes an edit or a withdrawal
                // chase the listing down every path it took. Shortest-path tie-breaking
                // makes the fold order-independent when the same revision arrives twice:
                // without it, whichever copy folded last would win and two devices could
                // disagree about `via`/`hops` forever.
                let key = (origin, thing_id.clone());
                if let Some(held) = state.listings.get(&key) {
                    if rev < held.rev || (rev == held.rev && hops >= held.hops) {
                        return Ok(());
                    }
                }
                state.listings.insert(
                    key,
                    Listing {
                        origin,
                        thing_id,
                        posture,
                        title,
                        descriptor,
                        price,
                        deadline,
                        area,
                        reach,
                        hops,
                        via: *op.author,
                        rev,
                        withdrawn,
                        photo,
                        photo_mime,
                    },
                );
                Ok(())
            }
            OP_DISCOVER_REQUEST => {
                // VALIDATE EVERYTHING, THEN COMMIT — the listing arm's atomicity contract.
                let origin = arg_hex32(op.args, "origin")?;
                let rid = opt_text_nonempty(op.args, "rid").ok_or(DeltaRejection::MalformedArgs)?;
                let kind =
                    opt_text_nonempty(op.args, "kind").ok_or(DeltaRejection::MalformedArgs)?;
                let tags: Vec<String> = match crate::arg_reads::get(op.args, "tags") {
                    Some(ArgVal::Text(json)) => {
                        serde_json::from_str(json).map_err(|_| DeltaRejection::MalformedArgs)?
                    }
                    None => Vec::new(),
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                let hops = match req_int(op.args, "hops")? {
                    h if h >= 0 => h as u32,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                let budget = match req_int(op.args, "budget")? {
                    b if b > 0 => b as u32,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                let at = match req_int(op.args, "at")? {
                    t if t > 0 => t,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };

                // The listing arm's shared path gate, with the origin's REMAINING
                // budget as the ceiling. `hops >= budget` is rejected (not just `>`):
                // a request's recipient sits at distance hops+1, so a copy at
                // hops == budget names someone the allowance can no longer reach —
                // authoring it anywhere would be a signed no-op.
                if budget > MAX_DISCOVER_HOPS {
                    return Err(DeltaRejection::MalformedArgs);
                }
                check_relay_path(origin != *op.author, hops, budget - 1)?;

                // A request is immutable — copies differ only in path, so the shortest
                // wins and ties keep the incumbent (order-independent either way).
                let key = (origin, rid.clone());
                if let Some(held) = state.discover_requests.get(&key) {
                    if hops >= held.hops {
                        return Ok(());
                    }
                }
                state.discover_requests.insert(
                    key,
                    DiscoverRequest {
                        origin,
                        rid,
                        kind,
                        tags,
                        hops,
                        budget,
                        at,
                        via: *op.author,
                    },
                );
                Ok(())
            }
            OP_DISCOVER_RESPONSE => {
                let origin = arg_hex32(op.args, "origin")?;
                let holder = arg_hex32(op.args, "holder")?;
                let rid = opt_text_nonempty(op.args, "rid").ok_or(DeltaRejection::MalformedArgs)?;
                let items: Vec<DiscoverItem> = match crate::arg_reads::get(op.args, "items") {
                    Some(ArgVal::Text(json)) => {
                        serde_json::from_str(json).map_err(|_| DeltaRejection::MalformedArgs)?
                    }
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                let hops = match req_int(op.args, "hops")? {
                    h if h > 0 => h as u32,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                let at = match req_int(op.args, "at")? {
                    t if t > 0 => t,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };

                // `hops` is the HOLDER's distance from the asker, so it is the same
                // on the holder's own authoring and on every relay of it — unlike the
                // request leg there is no per-forward increment to forge. What CAN be
                // checked: an answer is at least one hop out (the `> 0` lens above),
                // never past the network ceiling, and neither authored by the asker
                // nor about their own objects (both would be answering your own
                // question).
                if hops > MAX_DISCOVER_HOPS {
                    return Err(DeltaRejection::MalformedArgs);
                }
                if origin == *op.author || origin == holder {
                    return Err(DeltaRejection::MalformedArgs);
                }
                // VISIBILITY TRAVELS WITH THE ROW, and every honest device enforces
                // it at fold — a patched relay that forwards a `connections` item
                // to a friend-of-friend is rejected by the recipient, exactly as a
                // relayed `private` listing is. `hops` is the answer's distance, so
                // the check is one comparison per item.
                if items.iter().any(|i| i.vis.reach_hops() < hops) {
                    return Err(DeltaRejection::MalformedArgs);
                }

                // LWW by the holder's clock — a re-answer replaces the set wholesale.
                // Ties break on the serialized items so two devices folding the same
                // deltas in either order settle on the same row.
                let key = (origin, rid.clone(), holder);
                if let Some(held) = state.discover_responses.get(&key) {
                    let tie = at == held.at
                        && serde_json::to_string(&items).unwrap_or_default()
                            <= serde_json::to_string(&held.items).unwrap_or_default();
                    if at < held.at || tie {
                        return Ok(());
                    }
                }
                state.discover_responses.insert(
                    key,
                    DiscoverResponse {
                        origin,
                        rid,
                        holder,
                        items,
                        hops,
                        at,
                        via: *op.author,
                    },
                );
                Ok(())
            }
            // The ticket legs route to their domain module — the validation lives in
            // ONE place (`event.rs`) exactly as geo owns location for every kind.
            crate::event::OP_REQUEST_TICKET => {
                crate::event::reduce_request_ticket(&mut state.ticket_requests, op)
            }
            crate::event::OP_DELIVER_TICKET => {
                crate::event::reduce_deliver_ticket(&mut state.wallet, op)
            }
            crate::event::OP_ADMIT_TICKET => {
                crate::event::reduce_admit_ticket(&mut state.wallet, op)
            }
            crate::event::OP_INVITE => crate::event::reduce_invite(&mut state.invites, op),
            crate::event::OP_INVITE_REPLY => {
                crate::event::reduce_invite_reply(&mut state.invite_replies, op)
            }
            OP_SET_LINK => {
                // One of the two people on this connection states their own wish; a leaf
                // not on its roster states nothing here.
                if !op.ctx.is_member(op.author) {
                    return Err(DeltaRejection::Unauthorized);
                }
                let active = match req_int(op.args, "active")? {
                    0 => false,
                    1 => true,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                let gen = match req_int(op.args, "gen")? {
                    g if g >= 0 => g as u64,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                // Per-author LWW by gen: a stale replay never undoes a newer wish.
                if state.links.get(op.author).is_some_and(|held| gen < held.gen) {
                    return Ok(());
                }
                state.links.insert(*op.author, LinkWish { active, gen });
                Ok(())
            }
            _ => Err(DeltaRejection::UnknownType),
        }
    }
}

// The MemberId re-export keeps signatures readable for node when it picks peers.
pub type PeerId = MemberId;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator::{Coordinator, Delta};
    use crate::object::{build_delta, ObjectKind};

    /// The two ticket OpDecls written out in CONTACT_OPS must stay byte-identical to
    /// `event::CONTACT_TICKET_OPS` — the drift guard the splice pattern requires.
    #[test]
    fn ticket_ops_are_spliced_verbatim() {
        for base in crate::event::CONTACT_TICKET_OPS {
            let spliced = ContactType::op(base.op_id).expect("ticket op spliced");
            assert_eq!(spliced.name, base.name);
            assert_eq!(spliced.authority, base.authority);
            assert_eq!(spliced.commutativity, base.commutativity);
        }
    }

    /// The invite legs get the same drift guard, and the same "spliced from event.rs"
    /// contract as the ticket legs above.
    #[test]
    fn invite_ops_are_declared_where_the_reducer_expects_them() {
        let inv = ContactType::op(crate::event::OP_INVITE).expect("invite op declared");
        assert_eq!(inv.name, "contact.invite");
        assert_eq!(inv.authority, Authority::AnyMember);
        assert_eq!(inv.commutativity, Commutativity::Commutative);
        let rep = ContactType::op(crate::event::OP_INVITE_REPLY).expect("reply op declared");
        assert_eq!(rep.name, "contact.inviteReply");
        // The legs must not collide with Contact's own ops, which number from 0.
        assert!(crate::event::OP_INVITE > OP_DISCOVER_RESPONSE);
    }

    fn me() -> [u8; 32] {
        [1u8; 32]
    }
    fn peer() -> [u8; 32] {
        [2u8; 32]
    }

    // A supply delta authored BY `author` offering `kp` bytes.
    fn supply(kp: &[u8], not_after: u64, gen: u64) -> Delta {
        build_delta(
            ObjectKind::Contact,
            OP_PREKEY_SUPPLY,
            supply_args(kp, &[7u8; 32], not_after),
            0,
            Some(gen),
        )
    }
    fn consume(id: &[u8; 32], gen: u64) -> Delta {
        build_delta(
            ObjectKind::Contact,
            OP_PREKEY_CONSUME,
            id_args(id),
            0,
            Some(gen),
        )
    }
    fn revoke(id: &[u8; 32], gen: u64) -> Delta {
        build_delta(
            ObjectKind::Contact,
            OP_PREKEY_REVOKE,
            id_args(id),
            0,
            Some(gen),
        )
    }

    fn folded(entries: Vec<(Delta, [u8; 32])>) -> ContactState {
        let mut c = Coordinator::<ContactType>::new(vec![me(), peer()], me());
        for (d, author) in entries {
            c.deliver(d, author).unwrap();
        }
        c.state().clone()
    }

    fn pool(entries: Vec<(Delta, [u8; 32])>) -> PrekeyPool {
        folded(entries).prekeys
    }

    #[test]
    fn supply_then_consume_leaves_it_spent() {
        let kp = b"kp-alpha".to_vec();
        let id = kp_id(&kp);
        let p = pool(vec![(supply(&kp, 0, 0), peer()), (consume(&id, 1), me())]);
        assert!(
            p.available.is_empty(),
            "consumed offer must not remain available"
        );
        assert!(p.spent.contains(&id));
        assert_eq!(p.count_from(&peer(), 0), 0);
    }

    #[test]
    fn consume_before_supply_still_wins() {
        // Out-of-order sync: the tombstone folds first; the later supply is ignored.
        let kp = b"kp-beta".to_vec();
        let id = kp_id(&kp);
        let p = pool(vec![(consume(&id, 0), me()), (supply(&kp, 0, 1), peer())]);
        assert!(p.available.is_empty());
        assert!(p.spent.contains(&id));
    }

    #[test]
    fn revoke_withdraws_an_unconsumed_offer() {
        let kp = b"kp-gamma".to_vec();
        let id = kp_id(&kp);
        let p = pool(vec![(supply(&kp, 0, 0), peer()), (revoke(&id, 1), peer())]);
        assert!(p.available.is_empty());
        assert_eq!(p.count_from(&peer(), 0), 0);
    }

    #[test]
    fn stock_of_twenty_is_pickable_and_attributed_to_peer() {
        let mut entries = vec![];
        for i in 0..20u8 {
            let kp = vec![0xAB, i]; // distinct bytes -> distinct kp_ids
            entries.push((supply(&kp, 0, i as u64), peer()));
        }
        let p = pool(entries);
        assert_eq!(p.count_from(&peer(), 0), 20, "all twenty offers usable");
        assert_eq!(p.count_from(&me(), 0), 0, "none are attributed to me");
        let (_id, kp, _tag) = p.pick_from(&peer(), 0).expect("one to consume");
        assert!(!kp.is_empty());
    }

    #[test]
    fn expired_offers_are_not_usable() {
        let kp = b"kp-old".to_vec();
        // not_after = 100; at now=200 it is expired.
        let p = pool(vec![(supply(&kp, 100, 0), peer())]);
        assert_eq!(p.count_from(&peer(), 200), 0, "expired offer excluded");
        assert_eq!(p.count_from(&peer(), 50), 1, "still valid before expiry");
    }

    // ---- publishProfile: the self-announced profile arm --------------------------

    fn card_with(note: &str) -> ContactCard {
        ContactCard {
            note: note.into(),
            ..Default::default()
        }
    }

    fn publish(name: &str, card: &ContactCard, gen: u64) -> Delta {
        build_delta(
            ObjectKind::Contact,
            OP_PUBLISH_PROFILE,
            publish_profile_args(name, GroupShape::Individual, card, gen).unwrap(),
            0,
            Some(gen),
        )
    }

    #[test]
    fn each_side_publishes_its_own_profile_without_clobbering_the_other() {
        // The whole reason this op is AnyMember/per-author rather than owner-only:
        // a connection has TWO selves and both must be able to describe themselves.
        let st = folded(vec![
            (publish("Ada", &card_with("mine"), 0), me()),
            (publish("Grace", &card_with("theirs"), 0), peer()),
        ]);
        assert_eq!(st.profile_of(&me()).unwrap().display_name, "Ada");
        assert_eq!(st.profile_of(&peer()).unwrap().display_name, "Grace");
        assert_eq!(st.profile_of(&peer()).unwrap().card.note, "theirs");
    }

    #[test]
    fn a_later_announcement_replaces_the_earlier_one() {
        let st = folded(vec![
            (publish("Ada", &card_with("v1"), 0), peer()),
            (publish("Ada Lovelace", &card_with("v2"), 1), peer()),
        ]);
        let p = st.profile_of(&peer()).unwrap();
        assert_eq!(p.display_name, "Ada Lovelace");
        assert_eq!(p.card.note, "v2");
        assert_eq!(p.gen, 1);
    }

    #[test]
    fn a_stale_announcement_arriving_late_does_not_win() {
        // Out-of-order delivery: gen 1 folds first, then the older gen 0 shows up.
        // The register must still hold v2 — LWW is by gen, never by arrival.
        let st = folded(vec![
            (publish("Ada Lovelace", &card_with("v2"), 1), peer()),
            (publish("Ada", &card_with("v1"), 0), peer()),
        ]);
        let p = st.profile_of(&peer()).unwrap();
        assert_eq!(p.display_name, "Ada Lovelace");
        assert_eq!(p.card.note, "v2");
    }

    #[test]
    fn a_peer_who_never_published_has_no_profile() {
        // Not an empty one — the difference matters: the UI falls back to the
        // pairing-bundle name rather than rendering a blank card.
        let st = folded(vec![(publish("Ada", &card_with(""), 0), me())]);
        assert!(st.profile_of(&peer()).is_none());
    }

    #[test]
    fn a_malformed_announcement_leaves_the_previous_profile_intact() {
        // Atomicity: a bad shape must not half-apply (the group.setProfile contract).
        let mut bad =
            publish_profile_args("Clobber", GroupShape::Individual, &card_with("bad"), 1).unwrap();
        bad.insert("shape".into(), ArgVal::Text("planet".into()));
        let st = folded(vec![
            (publish("Ada", &card_with("good"), 0), peer()),
            (
                build_delta(ObjectKind::Contact, OP_PUBLISH_PROFILE, bad, 0, Some(1)),
                peer(),
            ),
        ]);
        let p = st.profile_of(&peer()).unwrap();
        assert_eq!(p.display_name, "Ada");
        assert_eq!(p.card.note, "good");
    }

    #[test]
    fn a_card_from_a_newer_peer_still_folds() {
        // Forward compatibility is the property that lets the "last seen" clip slot
        // (or anything after it) ship to half a network at a time: an unknown field
        // is ignored, not a rejected delta.
        let mut args =
            publish_profile_args("Ada", GroupShape::Individual, &card_with("hi"), 0).unwrap();
        args.insert(
            "card".into(),
            ArgVal::Text(r#"{"note":"hi","hologram":"not-invented-yet","tags":["yc"]}"#.into()),
        );
        let st = folded(vec![(
            build_delta(ObjectKind::Contact, OP_PUBLISH_PROFILE, args, 0, Some(0)),
            peer(),
        )]);
        let p = st.profile_of(&peer()).unwrap();
        assert_eq!(p.card.note, "hi");
        assert_eq!(p.card.tags, vec!["yc".to_string()]);
    }

    #[test]
    fn the_still_crosses_the_fan_out_and_an_inline_clip_does_not() {
        // The STILL rides the connection fan-out with the capture time that makes
        // "last seen" meaningful. The MOTION slot does not ride it at all: one delta
        // per Connection makes 1.5MB of inline clip unaffordable, so the fold refuses
        // a card carrying clip bytes and the slot ships detached instead.
        let still_only = ContactCard {
            photo: "c3RpbGw=".into(),
            photo_mime: "image/jpeg".into(),
            captured_at: 1_800_000_000,
            avatar_color: "7A6FF0".into(),
            ..Default::default()
        };
        let st = folded(vec![(publish("Ada", &still_only, 0), peer())]);
        let got = &st.profile_of(&peer()).unwrap().card;
        assert_eq!(got.photo, "c3RpbGw=");
        assert_eq!(got.captured_at, 1_800_000_000);
        assert_eq!(got.avatar_color, "7A6FF0");

        // A TINY clip — four bytes, orders of magnitude under any cap — is refused
        // just the same. The rule is the ROUTE, not the size.
        let with_clip = ContactCard {
            clip: "Y2xpcA==".into(),
            clip_mime: "video/mp4".into(),
            ..Default::default()
        };
        let st = folded(vec![
            (publish("Ada", &card_with("good"), 0), peer()),
            (publish("Clobber", &with_clip, 1), peer()),
        ]);
        let p = st.profile_of(&peer()).unwrap();
        assert_eq!(p.card.note, "good", "a card carrying a clip is refused whole");
        assert_eq!(p.gen, 0, "the refused announcement must not bump the gen");
    }


    #[test]
    fn a_profile_clip_is_refused_whatever_its_container() {
        // This slot used to be size-gated only, and deliberately container-blind —
        // the fold's job being to bound the gossip rather than to enforce OUR mime
        // set on somebody else's data. Cutting the inline route makes the container
        // irrelevant in the other direction: mp4, gif, quicktime, or a mime this
        // build has never heard of, none of them cross.
        for (clip, mime) in [
            ("Y2xpcA==", "video/mp4"),
            ("Z2lm", "image/gif"),
            ("bW92", "video/quicktime"),
            ("aGVpYw==", "image/heic"),
        ] {
            let card = ContactCard {
                clip: clip.into(),
                clip_mime: mime.into(),
                ..Default::default()
            };
            let st = folded(vec![(publish("Ada", &card, 0), peer())]);
            assert!(
                st.profile_of(&peer()).is_none(),
                "a {mime} clip must not cross the connection fan-out"
            );
        }
    }

    #[test]
    fn a_clip_mime_with_no_bytes_is_refused_too() {
        // The pair is refused as a pair. A card carrying `clip_mime` with an empty
        // `clip` would fold to a dangling container with nothing in it — harmless to
        // render, but it is the shape a sender ends up with after trying to strip
        // the bytes locally, and letting it through would suggest the route exists.
        let card = ContactCard {
            clip_mime: "video/mp4".into(),
            ..Default::default()
        };
        let st = folded(vec![(publish("Ada", &card, 0), peer())]);
        assert!(st.profile_of(&peer()).is_none());
    }

    #[test]
    fn a_profile_photo_in_a_container_we_do_not_author_still_folds() {
        // Same rule on the still slot. HEIC matters specifically: it is what iOS
        // produces natively, so refusing it at the fold would drop real cards.
        let card = ContactCard {
            photo: "aGVpYw==".into(),
            photo_mime: "image/heic".into(),
            ..Default::default()
        };
        let st = folded(vec![(publish("Ada", &card, 0), peer())]);
        assert_eq!(
            st.profile_of(&peer()).expect("heic card refused").card.photo_mime,
            "image/heic"
        );
    }

    #[test]
    fn an_oversized_avatar_announcement_is_refused_whole() {
        // The avatar cap hole, closed: the listing's face was already fold-gated
        // (see publishListing's photo gate) but the card's photo slot would
        // otherwise fold unchecked — a patched peer could announce media of any
        // size. A refusal never disturbs the profile already held. The clip arm is
        // refused for a different reason now (the route, not the cap) and is kept
        // here to prove BOTH refusals leave the held profile intact.
        let big_photo = ContactCard {
            photo: "x".repeat(crate::event::MAX_PHOTO_B64 + 1),
            photo_mime: "image/jpeg".into(),
            ..Default::default()
        };
        let big_clip = ContactCard {
            clip: "x".repeat(crate::event::MAX_CLIP_B64 + 1),
            clip_mime: "video/mp4".into(),
            ..Default::default()
        };
        for bad in [big_photo, big_clip] {
            let st = folded(vec![
                (publish("Ada", &card_with("good"), 0), peer()),
                (publish("Clobber", &bad, 1), peer()),
            ]);
            let p = st.profile_of(&peer()).unwrap();
            assert_eq!(p.display_name, "Ada", "an oversized card is refused whole");
            assert_eq!(p.card.note, "good");
            assert_eq!(p.gen, 0, "the refused announcement must not bump the gen");
        }
    }

    #[test]
    fn an_at_cap_avatar_announcement_folds() {
        // Exactly at the caps is a legal card — the gate refuses OVERsize only.
        let card = ContactCard {
            photo: "x".repeat(crate::event::MAX_PHOTO_B64),
            photo_mime: "image/jpeg".into(),
            ..Default::default()
        };
        let st = folded(vec![(publish("Ada", &card, 0), peer())]);
        let got = &st.profile_of(&peer()).unwrap().card;
        assert_eq!(got.photo.len(), crate::event::MAX_PHOTO_B64);
    }

    #[test]
    fn profiles_and_prekeys_share_one_log_without_interfering() {
        let kp = b"kp-shared".to_vec();
        let st = folded(vec![
            (supply(&kp, 0, 0), peer()),
            (publish("Ada", &card_with("hi"), 1), peer()),
        ]);
        assert_eq!(st.prekeys.count_from(&peer(), 0), 1, "the offer survives");
        assert_eq!(st.profile_of(&peer()).unwrap().display_name, "Ada");
    }

    #[test]
    fn resupply_of_a_consumed_id_is_not_resurrected() {
        let kp = b"kp-zombie".to_vec();
        let id = kp_id(&kp);
        let p = pool(vec![
            (supply(&kp, 0, 0), peer()),
            (consume(&id, 1), me()),
            (supply(&kp, 0, 2), peer()), // re-offer the SAME id after consume
        ]);
        assert!(p.available.is_empty(), "a consumed id stays spent");
        assert!(p.spent.contains(&id));
    }

    // ---- publishListing: discovery, and the multihop relay rules -----------------

    use crate::thing::{Posture, Reach};

    /// A third party — the origin of a listing that reaches me by relay.
    fn ada() -> [u8; 32] {
        [3u8; 32]
    }

    #[allow(clippy::too_many_arguments)]
    fn listing_delta(
        origin: [u8; 32],
        thing: &str,
        posture: Posture,
        title: &str,
        price: Option<&str>,
        reach: Reach,
        hops: u32,
        rev: u64,
        withdrawn: bool,
        gen: u64,
    ) -> Delta {
        build_delta(
            ObjectKind::Contact,
            OP_PUBLISH_LISTING,
            publish_listing_args(
                &origin,
                thing,
                posture,
                title,
                "",
                price,
                None,
                Some("gcpv"),
                reach,
                hops,
                rev,
                withdrawn,
                None,
            ),
            0,
            Some(gen),
        )
    }

    /// The ordinary case: a peer announces their own listing, hops = 0.
    fn direct(thing: &str, title: &str, rev: u64, gen: u64) -> Delta {
        listing_delta(
            peer(),
            thing,
            Posture::Offers,
            title,
            None,
            Reach::Network,
            0,
            rev,
            false,
            gen,
        )
    }

    #[test]
    fn a_peers_listing_arrives_and_is_attributed_to_them() {
        let st = folded(vec![(direct("thing-1", "Extension ladder", 0, 0), peer())]);
        let l = st
            .listings
            .get(&(peer(), "thing-1".into()))
            .expect("listing folded");
        assert_eq!(l.title, "Extension ladder");
        assert_eq!(l.origin, peer(), "attributed to its origin");
        assert_eq!(l.via, peer(), "and told to me directly");
        assert_eq!(l.hops, 0);
        assert_eq!(st.live_listings(0).len(), 1);
    }

    #[test]
    fn a_relayed_listing_keeps_its_origin_but_records_who_passed_it_on() {
        // Ben (`peer`) forwards Ada's listing to me. The listing is Ada's; the trust path
        // is Ben's — and both have to survive, or a stranger's offer is unattributable.
        let st = folded(vec![(
            listing_delta(
                ada(),
                "t-9",
                Posture::Selling,
                "Cargo bike",
                Some("£300"),
                Reach::Network,
                1,
                0,
                false,
                0,
            ),
            peer(),
        )]);
        let l = st
            .listings
            .get(&(ada(), "t-9".into()))
            .expect("relayed listing folded");
        assert_eq!(l.origin, ada(), "still Ada's listing");
        assert_eq!(l.via, peer(), "Ben is the one who told me");
        assert_eq!(l.hops, 1);
        assert_eq!(l.price.as_deref(), Some("£300"));
    }

    #[test]
    fn a_private_listing_cannot_be_relayed() {
        // The consent invariant: `private` means my connections and no further. A relay
        // carrying one is rejected by the fold, not merely discouraged in the sender.
        let st = folded(vec![(
            listing_delta(
                ada(),
                "t-secret",
                Posture::Selling,
                "Ring",
                Some("£20"),
                Reach::Private,
                1,
                0,
                false,
                0,
            ),
            peer(),
        )]);
        assert!(
            st.listings.is_empty(),
            "a relayed private listing must not fold"
        );
    }

    #[test]
    fn a_forwarder_cannot_forge_a_shorter_path() {
        // hops=0 claims "straight from the origin", so only the origin may say it.
        let forged = folded(vec![(
            listing_delta(
                ada(),
                "t-9",
                Posture::Offers,
                "Cargo bike",
                None,
                Reach::Network,
                0,
                0,
                false,
                0,
            ),
            peer(),
        )]);
        assert!(
            forged.listings.is_empty(),
            "relay claiming hops=0 must not fold"
        );

        // ...and symmetrically, an origin cannot claim to be its own relay.
        let odd = folded(vec![(
            listing_delta(
                peer(),
                "t-1",
                Posture::Offers,
                "Ladder",
                None,
                Reach::Network,
                2,
                0,
                false,
                0,
            ),
            peer(),
        )]);
        assert!(
            odd.listings.is_empty(),
            "origin publishing at hops>0 must not fold"
        );
    }

    #[test]
    fn the_hop_ceiling_is_enforced_at_fold() {
        let ok = folded(vec![(
            listing_delta(
                ada(),
                "t-far",
                Posture::Offers,
                "Trailer",
                None,
                Reach::Network,
                MAX_LISTING_HOPS,
                0,
                false,
                0,
            ),
            peer(),
        )]);
        assert_eq!(ok.listings.len(), 1, "exactly at the ceiling still folds");

        let too_far = folded(vec![(
            listing_delta(
                ada(),
                "t-far",
                Posture::Offers,
                "Trailer",
                None,
                Reach::Network,
                MAX_LISTING_HOPS + 1,
                0,
                false,
                0,
            ),
            peer(),
        )]);
        assert!(
            too_far.listings.is_empty(),
            "past the ceiling is rejected by every device"
        );
    }

    #[test]
    fn the_shortest_path_wins_regardless_of_arrival_order() {
        // The same revision reaching me twice: once relayed at 2 hops, once at 1. Whichever
        // folds last, the state must settle on the shorter path — otherwise two devices
        // holding the same log disagree.
        let long = listing_delta(
            ada(),
            "t-9",
            Posture::Offers,
            "Bike",
            None,
            Reach::Network,
            2,
            0,
            false,
            0,
        );
        let short = listing_delta(
            ada(),
            "t-9",
            Posture::Offers,
            "Bike",
            None,
            Reach::Network,
            1,
            0,
            false,
            1,
        );

        let a = folded(vec![(long.clone(), peer()), (short.clone(), me())]);
        let b = folded(vec![(short, me()), (long, peer())]);
        assert_eq!(a.listings[&(ada(), "t-9".into())].hops, 1);
        assert_eq!(
            b.listings[&(ada(), "t-9".into())].hops,
            1,
            "order-independent"
        );
    }

    #[test]
    fn a_newer_revision_beats_a_shorter_stale_path() {
        // Revision dominates distance: an edit that arrives the long way still replaces a
        // stale copy that came direct, or a price change could never catch up with itself.
        let st = folded(vec![
            (
                listing_delta(
                    ada(),
                    "t-9",
                    Posture::Selling,
                    "Bike",
                    Some("£300"),
                    Reach::Network,
                    1,
                    0,
                    false,
                    0,
                ),
                peer(),
            ),
            (
                listing_delta(
                    ada(),
                    "t-9",
                    Posture::Selling,
                    "Bike",
                    Some("£250"),
                    Reach::Network,
                    3,
                    1,
                    false,
                    1,
                ),
                me(),
            ),
        ]);
        let l = &st.listings[&(ada(), "t-9".into())];
        assert_eq!(l.price.as_deref(), Some("£250"), "the newer revision wins");
        assert_eq!(l.hops, 3);
    }

    #[test]
    fn a_stale_revision_never_resurrects_a_withdrawn_listing() {
        // The withdrawal has to chase the listing down every path it took, including paths
        // that are still delivering the older copy.
        let st = folded(vec![
            (direct("thing-1", "Ladder", 0, 0), peer()),
            (
                listing_delta(
                    peer(),
                    "thing-1",
                    Posture::Offers,
                    "Ladder",
                    None,
                    Reach::Network,
                    0,
                    1,
                    true,
                    1,
                ),
                peer(),
            ),
            (direct("thing-1", "Ladder", 0, 2), peer()), // a slow copy of rev 0
        ]);
        let l = &st.listings[&(peer(), "thing-1".into())];
        assert!(l.withdrawn, "the tombstone holds");
        assert!(
            st.live_listings(0).is_empty(),
            "and it is not offered to the market"
        );
    }

    #[test]
    fn a_lapsed_deadline_drops_out_of_the_live_set() {
        let args = publish_listing_args(
            &peer(),
            "t-dated",
            Posture::Selling,
            "Concert ticket",
            "",
            Some("£40"),
            Some(1_000),
            None,
            Reach::Network,
            0,
            0,
            false,
            None,
        );
        let st = folded(vec![(
            build_delta(ObjectKind::Contact, OP_PUBLISH_LISTING, args, 0, Some(0)),
            peer(),
        )]);
        assert_eq!(st.live_listings(500).len(), 1, "live before the deadline");
        assert!(st.live_listings(2_000).is_empty(), "gone after it");
    }

    #[test]
    fn forwardable_excludes_private_and_the_last_hop() {
        let st = folded(vec![
            (direct("mine", "Ladder", 0, 0), peer()),
            (
                listing_delta(
                    ada(),
                    "far",
                    Posture::Offers,
                    "Trailer",
                    None,
                    Reach::Network,
                    MAX_LISTING_HOPS,
                    0,
                    false,
                    1,
                ),
                peer(),
            ),
            (
                listing_delta(
                    peer(),
                    "priv",
                    Posture::Selling,
                    "Ring",
                    Some("£20"),
                    Reach::Private,
                    0,
                    0,
                    false,
                    2,
                ),
                peer(),
            ),
        ]);
        let f = st.forwardable(0);
        assert_eq!(f.len(), 1, "only the network listing below the ceiling");
        assert_eq!(f[0].thing_id, "mine");
    }

    #[test]
    fn a_withdrawal_is_still_forwardable_though_it_is_not_live() {
        // The tombstone has to chase the listing down the paths it took. If `forwardable`
        // filtered it out with the rest of the not-live rows, a sold bike would stay on
        // sale forever two hops out.
        let st = folded(vec![
            (direct("thing-1", "Ladder", 0, 0), peer()),
            (
                listing_delta(
                    peer(),
                    "thing-1",
                    Posture::Offers,
                    "Ladder",
                    None,
                    Reach::Network,
                    0,
                    1,
                    true,
                    1,
                ),
                peer(),
            ),
        ]);
        assert!(
            st.live_listings(0).is_empty(),
            "withdrawn: not on the market"
        );
        let f = st.forwardable(0);
        assert_eq!(
            f.len(),
            1,
            "but still relayed, so the news of its removal travels"
        );
        assert!(f[0].withdrawn);
    }

    #[test]
    fn two_origins_may_use_the_same_thing_id_without_collision() {
        // Thing ids are minted locally, so two people can hold the same one. The key is
        // (origin, thing_id) precisely so that is not a collision.
        let st = folded(vec![
            (
                listing_delta(
                    peer(),
                    "t-1",
                    Posture::Offers,
                    "Ben's ladder",
                    None,
                    Reach::Network,
                    0,
                    0,
                    false,
                    0,
                ),
                peer(),
            ),
            (
                listing_delta(
                    ada(),
                    "t-1",
                    Posture::Offers,
                    "Ada's kayak",
                    None,
                    Reach::Network,
                    1,
                    0,
                    false,
                    1,
                ),
                peer(),
            ),
        ]);
        assert_eq!(
            st.listings.len(),
            2,
            "same thing id, different origins, two rows"
        );
    }

    #[test]
    fn a_price_on_a_standing_intent_is_rejected() {
        // The same rule the Thing reducer enforces, applied to the announcement so the
        // object and its listing can never disagree.
        let st = folded(vec![(
            listing_delta(
                peer(),
                "t-1",
                Posture::Wants,
                "A drill",
                Some("£10"),
                Reach::Network,
                0,
                0,
                false,
                0,
            ),
            peer(),
        )]);
        assert!(
            st.listings.is_empty(),
            "a Want has no price until it becomes Buying"
        );
    }

    // ---- discovery: the pull twin of the listing gossip ---------------------------

    fn request_delta(origin: [u8; 32], rid: &str, hops: u32, budget: u32, gen: u64) -> Delta {
        build_delta(
            ObjectKind::Contact,
            OP_DISCOVER_REQUEST,
            discover_request_args(
                &origin,
                rid,
                "place",
                &["coffee".into()],
                hops,
                budget,
                1_000,
            ),
            0,
            Some(gen),
        )
    }

    fn item(id: &str, name: &str) -> DiscoverItem {
        item_vis(id, name, crate::visibility::Visibility::Network)
    }

    fn item_vis(id: &str, name: &str, vis: crate::visibility::Visibility) -> DiscoverItem {
        DiscoverItem {
            id: id.into(),
            kind: "place".into(),
            name: name.into(),
            descriptor: String::new(),
            lat_e7: Some(514_545_000),
            lng_e7: Some(-25_879_000),
            vis,
        }
    }

    fn response_delta(
        origin: [u8; 32],
        rid: &str,
        holder: [u8; 32],
        items: &[DiscoverItem],
        hops: u32,
        at: i64,
        gen: u64,
    ) -> Delta {
        build_delta(
            ObjectKind::Contact,
            OP_DISCOVER_RESPONSE,
            discover_response_args(&origin, rid, &holder, items, hops, at),
            0,
            Some(gen),
        )
    }

    #[test]
    fn a_direct_request_folds_and_a_forged_relay_does_not() {
        // me asks peer directly: hops=0, author=origin.
        let st = folded(vec![(request_delta(me(), "r1", 0, 2, 0), me())]);
        let r = st.discover_requests.get(&(me(), "r1".into())).unwrap();
        assert_eq!(r.kind, "place");
        assert_eq!(r.tags, vec!["coffee".to_string()]);
        assert_eq!(r.budget, 2);
        assert_eq!(r.via, me());

        // A relay claiming hops=0 is a forged path — rejected like a listing's.
        let forged = folded(vec![(request_delta(ada(), "r2", 0, 2, 0), peer())]);
        assert!(forged.discover_requests.is_empty());

        // ...and an origin cannot claim to be its own relay.
        let odd = folded(vec![(request_delta(me(), "r3", 1, 2, 0), me())]);
        assert!(odd.discover_requests.is_empty());
    }

    #[test]
    fn the_discover_budget_ceiling_is_absolute() {
        // budget beyond the network ceiling: rejected outright.
        let st = folded(vec![(
            request_delta(me(), "r1", 0, MAX_DISCOVER_HOPS + 1, 0),
            me(),
        )]);
        assert!(st.discover_requests.is_empty());

        // hops past the granted budget: rejected even below the network ceiling.
        let st = folded(vec![(request_delta(ada(), "r2", 2, 1, 0), peer())]);
        assert!(st.discover_requests.is_empty());

        // hops AT the budget names a recipient beyond it — equally dead on arrival.
        let st = folded(vec![(request_delta(ada(), "r3", 2, 2, 0), peer())]);
        assert!(st.discover_requests.is_empty());
    }

    #[test]
    fn the_shortest_request_path_wins_in_either_order() {
        // My own question comes back to me relayed (a cycle in the graph) after my
        // direct copy: the fold must settle on the direct one in either order.
        let direct = request_delta(me(), "r1", 0, 2, 0);
        let echoed = request_delta(me(), "r1", 1, 2, 1);
        let a = folded(vec![(echoed.clone(), peer()), (direct.clone(), me())]);
        let b = folded(vec![(direct, me()), (echoed, peer())]);
        assert_eq!(a.discover_requests[&(me(), "r1".into())].hops, 0);
        assert_eq!(b.discover_requests[&(me(), "r1".into())].hops, 0);
    }

    #[test]
    fn a_direct_answer_folds_and_carries_its_holder() {
        // peer answers my question with their own places: author == holder, hops == 1.
        let st = folded(vec![(
            response_delta(
                me(),
                "r1",
                peer(),
                &[item("aa", "Blue Bottle")],
                1,
                5_000,
                0,
            ),
            peer(),
        )]);
        let r = &st.discover_responses[&(me(), "r1".into(), peer())];
        assert_eq!(r.items.len(), 1);
        assert_eq!(r.items[0].name, "Blue Bottle");
        assert_eq!(r.hops, 1);
        assert_eq!(r.via, peer());
    }

    #[test]
    fn an_answer_is_bounded_and_never_the_askers_own() {
        // Zero hops claims the asker answered themselves; past the ceiling has
        // outrun the trust that carried it. Both rejected by every device.
        for bad_hops in [0, MAX_DISCOVER_HOPS + 1] {
            let st = folded(vec![(
                response_delta(me(), "r1", peer(), &[item("aa", "X")], bad_hops, 5_000, 0),
                peer(),
            )]);
            assert!(
                st.discover_responses.is_empty(),
                "hops={bad_hops} must not fold"
            );
        }

        // The asker never authors answers to their own question...
        let st = folded(vec![(
            response_delta(me(), "r1", me(), &[item("aa", "X")], 1, 5_000, 0),
            me(),
        )]);
        assert!(st.discover_responses.is_empty());

        // ...and nobody answers a question WITH the asker's own objects.
        let st = folded(vec![(
            response_delta(me(), "r1", me(), &[item("aa", "X")], 2, 5_000, 0),
            peer(),
        )]);
        assert!(st.discover_responses.is_empty());
    }

    #[test]
    fn a_relayed_answer_keeps_its_holder_and_a_re_answer_replaces_wholesale() {
        // Ben relays Ada's answer to me: holder=ada, author=peer, hops=2.
        let st = folded(vec![
            (
                response_delta(
                    me(),
                    "r1",
                    ada(),
                    &[item("aa", "Old Café"), item("bb", "Kiosk")],
                    2,
                    5_000,
                    0,
                ),
                peer(),
            ),
            // Ada's public set changed; the newer answer replaces the SET, not a row.
            (
                response_delta(me(), "r1", ada(), &[item("cc", "New Café")], 2, 6_000, 1),
                peer(),
            ),
        ]);
        let r = &st.discover_responses[&(me(), "r1".into(), ada())];
        assert_eq!(r.holder, ada());
        assert_eq!(r.via, peer());
        assert_eq!(r.items.len(), 1, "replaced wholesale, never merged");
        assert_eq!(r.items[0].name, "New Café");

        // A stale replay arriving late does not resurrect the old set.
        let st2 = folded(vec![
            (
                response_delta(me(), "r1", ada(), &[item("cc", "New Café")], 2, 6_000, 0),
                peer(),
            ),
            (
                response_delta(me(), "r1", ada(), &[item("aa", "Old Café")], 2, 5_000, 1),
                peer(),
            ),
        ]);
        assert_eq!(
            st2.discover_responses[&(me(), "r1".into(), ada())].items[0].name,
            "New Café"
        );
    }

    /// An untagged question — the shape whose answers are cacheable.
    fn plain_request(origin: [u8; 32], rid: &str, hops: u32, gen: u64) -> Delta {
        build_delta(
            ObjectKind::Contact,
            OP_DISCOVER_REQUEST,
            discover_request_args(&origin, rid, "place", &[], hops, 2, 1_000),
            0,
            Some(gen),
        )
    }

    #[test]
    fn full_answers_cache_to_two_hops_without_ttl() {
        // Untagged questions; a direct answer AND a relayed friend-of-friend's —
        // the whole n-2 reach is cacheable, and none of it expires: the shelf
        // renders between sessions from the fold alone.
        let st = folded(vec![
            (plain_request(me(), "r1", 0, 0), me()),
            (plain_request(me(), "r2", 0, 1), me()),
            (
                response_delta(me(), "r1", peer(), &[item("aa", "Café")], 1, 1_000, 2),
                peer(),
            ),
            (
                response_delta(me(), "r2", ada(), &[item("bb", "Studio")], 2, 1_500, 3),
                peer(),
            ),
        ]);
        let cacheable = st.cacheable_discover_answers(&me());
        assert_eq!(cacheable.len(), 2, "direct and relayed both cache");

        // Long after both questions' TTLs the LIVE read is empty…
        let expired = 1_500 + DISCOVER_TTL_MS;
        assert!(st.live_discover_responses(expired).is_empty());
        // …but the cacheable set still holds — no clock in its signature at all.
        assert_eq!(st.cacheable_discover_answers(&me()).len(), 2);
    }

    #[test]
    fn tagged_answers_and_other_askers_never_cache() {
        let st = folded(vec![
            // A TAGGED question: its answer is a subset the responder filtered —
            // caching it as "what this peer has" would shrink the shelf.
            (request_delta(me(), "r1", 0, 2, 0), me()),
            (
                response_delta(me(), "r1", peer(), &[item("aa", "Filtered")], 1, 1_000, 1),
                peer(),
            ),
            // Someone ELSE's question passing through me: their answers are not my
            // cache (and `origin` scoping is what keeps it that way).
            (plain_request(ada(), "r2", 1, 2), peer()),
            (
                response_delta(ada(), "r2", peer(), &[item("bb", "Theirs")], 1, 1_000, 3),
                peer(),
            ),
        ]);
        assert!(
            st.cacheable_discover_answers(&me()).is_empty(),
            "neither the filtered answer nor another asker's may cache for me"
        );
        // The tagged answer still surfaces while its question lives.
        assert_eq!(st.live_discover_responses(1_500).len(), 2);
    }

    #[test]
    fn an_answer_never_arrives_past_an_items_reach() {
        use crate::visibility::Visibility;

        // A `connections` item in a DIRECT answer: exactly what the dial permits.
        let st = folded(vec![(
            response_delta(
                me(),
                "r1",
                peer(),
                &[item_vis("aa", "For friends", Visibility::Connections)],
                1,
                5_000,
                0,
            ),
            peer(),
        )]);
        assert_eq!(
            st.discover_responses.len(),
            1,
            "reach 1 covers a direct answer"
        );

        // The same item RELAYED (hops 2): past its reach — every device refuses.
        let st = folded(vec![(
            response_delta(
                me(),
                "r1",
                ada(),
                &[item_vis("aa", "For friends", Visibility::Connections)],
                2,
                5_000,
                0,
            ),
            peer(),
        )]);
        assert!(
            st.discover_responses.is_empty(),
            "a relay cannot widen consent"
        );

        // A `private` item (or one from a build that predates the dial — same
        // wire shape) has no reach at all: it belongs in NO answer.
        let st = folded(vec![(
            response_delta(
                me(),
                "r1",
                peer(),
                &[item_vis("aa", "Mine", Visibility::Private)],
                1,
                5_000,
                0,
            ),
            peer(),
        )]);
        assert!(st.discover_responses.is_empty());
    }

    #[test]
    fn discovery_ttl_retires_the_question_and_its_answers() {
        let st = folded(vec![
            (request_delta(peer(), "r1", 0, 2, 0), peer()),
            (
                response_delta(me(), "r2", peer(), &[item("aa", "X")], 1, 1_000, 1),
                peer(),
            ),
        ]);
        assert_eq!(st.live_discover_requests(0).len(), 1, "0 skips the filter");
        assert_eq!(st.live_discover_requests(1_000 + DISCOVER_TTL_MS).len(), 0);
        assert_eq!(st.live_discover_responses(500 + DISCOVER_TTL_MS).len(), 1);
        assert_eq!(st.live_discover_responses(1_000 + DISCOVER_TTL_MS).len(), 0);
    }
    fn invite_args(event: &str, title: &str, start: i64, at: i64, gen: i64) -> Args {
        let mut a = Args::new();
        a.insert("event".into(), ArgVal::Text(event.into()));
        a.insert("title".into(), ArgVal::Text(title.into()));
        a.insert("startMs".into(), ArgVal::Int(start));
        a.insert("venue".into(), ArgVal::Text("The Lexington".into()));
        a.insert("at".into(), ArgVal::Int(at));
        a.insert("gen".into(), ArgVal::Int(gen));
        a
    }

    fn reply_args(event: &str, going: bool, at: i64, gen: i64) -> Args {
        let mut a = Args::new();
        a.insert("event".into(), ArgVal::Text(event.into()));
        a.insert("going".into(), ArgVal::Int(i64::from(going)));
        a.insert("at".into(), ArgVal::Int(at));
        a.insert("gen".into(), ArgVal::Int(gen));
        a
    }

    fn invite(event: &str, gen: u64) -> Delta {
        build_delta(
            ObjectKind::Contact,
            crate::event::OP_INVITE,
            invite_args(
                event,
                "Coral Night",
                1_800_000_000_000,
                1_700_000_000_000,
                gen as i64,
            ),
            0,
            Some(gen),
        )
    }
    fn reply(event: &str, going: bool, gen: u64) -> Delta {
        build_delta(
            ObjectKind::Contact,
            crate::event::OP_INVITE_REPLY,
            reply_args(event, going, 1_700_000_100_000, gen as i64),
            0,
            Some(gen),
        )
    }

    /// The invitation is SELF-DESCRIBING: the guest never holds the Event object, so
    /// everything the card renders must be in the leg itself.
    #[test]
    fn an_invite_carries_enough_to_render_without_the_event_object() {
        let st = folded(vec![(invite("ev1", 0), me())]);
        let got = st
            .invites
            .get(&(me(), "ev1".into()))
            .expect("invite folded");
        assert_eq!(got.title, "Coral Night");
        assert_eq!(got.venue, "The Lexington");
        assert_eq!(got.start_ms, 1_800_000_000_000);
        assert_eq!(got.event, "ev1", "and the id joins back to the live object");
    }

    /// An invite and its answer are authored by DIFFERENT people, so neither may
    /// occupy or overwrite the other's slot.
    #[test]
    fn a_reply_cannot_overwrite_the_invitation_it_answers() {
        let st = folded(vec![
            (invite("ev1", 0), me()),
            (reply("ev1", true, 0), peer()),
        ]);
        assert_eq!(st.invites.len(), 1, "the invitation still stands");
        assert_eq!(st.invites[&(me(), "ev1".into())].title, "Coral Night");
        assert!(st.invite_replies[&(peer(), "ev1".into())].going);
    }

    /// Changing your mind is a later gen, not a retraction op.
    #[test]
    fn changing_your_mind_is_just_a_later_gen() {
        let st = folded(vec![
            (invite("ev1", 0), me()),
            (reply("ev1", true, 0), peer()),
            (reply("ev1", false, 1), peer()),
        ]);
        assert!(
            !st.invite_replies[&(peer(), "ev1".into())].going,
            "no longer going"
        );
        assert_eq!(
            st.invite_replies.len(),
            1,
            "one slot, not a history of flip-flops"
        );
    }

    #[test]
    fn a_stale_replay_never_undoes_a_newer_answer() {
        let st = folded(vec![
            (invite("ev1", 0), me()),
            (reply("ev1", false, 5), peer()),
            (reply("ev1", true, 1), peer()), // an old delta arriving late
        ]);
        assert!(
            !st.invite_replies[&(peer(), "ev1".into())].going,
            "the newer answer stands"
        );
    }

    /// Both sides may invite each other to different things on the same channel.
    #[test]
    fn each_side_owns_its_own_invite_slot() {
        let st = folded(vec![(invite("ev1", 0), me()), (invite("ev2", 0), peer())]);
        assert_eq!(st.invites.len(), 2);
        assert!(st.invites.contains_key(&(me(), "ev1".into())));
        assert!(st.invites.contains_key(&(peer(), "ev2".into())));
    }

    #[test]
    fn an_unanswerable_invite_is_refused() {
        for bad in [
            invite_args("", "Coral Night", 1, 1, 0),    // no event
            invite_args("ev1", "", 1, 1, 0),            // nothing to call it
            invite_args("ev1", "Coral Night", 0, 1, 0), // no when
            invite_args("ev1", "Coral Night", 1, 0, 0), // undated invite
        ] {
            let d = build_delta(
                ObjectKind::Contact,
                crate::event::OP_INVITE,
                bad,
                0,
                Some(0),
            );
            let mut c = Coordinator::<ContactType>::new(vec![me(), peer()], me());
            // deliver accepts it (integrity is fine); the REDUCER refuses it, so the
            // malformed leg is inert rather than half-applied.
            c.deliver(d, me()).unwrap();
            assert!(
                c.state().invites.is_empty(),
                "a malformed invite folds to nothing"
            );
        }
    }
}
