//! `Place` — somewhere in the world, as a durable, signed, syncable GroupObject.
//!
//! A Place is somewhere you have BEEN: minted against a live location fix, not dropped
//! on a map somewhere you have never stood. That is what makes it evidence rather than
//! an assertion, and it is why the coordinate is authored at mint time rather than
//! edited in afterwards.
//!
//! # Why a GroupObject and not a local row
//!
//! The point of knowing where somewhere is, is telling someone else. A device-local
//! entity row cannot be sent, cannot be co-owned, has no log and no signature — so it
//! can never be more than a private note to yourself. A Place is a group of 1 until you
//! share it; sharing is an MLS Add and nothing else about the object changes.
//!
//! # The location facet — the first base op-group consumer
//!
//! A Place's defining state is the COMMON location facet ([`crate::geo`]), not a
//! Place-specific field: `geo.rs` deliberately makes location available to every kind
//! (things, orgs, places, optionally people). Place is simply the kind whose *whole
//! purpose* is to carry one, so it is the first type to splice [`crate::geo::LOCATION_OPS`]
//! into its `ops()` and delegate to [`crate::geo::reduce_location`].
//!
//! Base ops live in a reserved high op-id band (`0xF000_xxxx`) precisely so this
//! splice can never collide with a type's own ops, which number from 0.
//!
//! # A Place is not a Thing
//!
//! [`crate::thing`] also touches geography, but for the opposite reason: a Thing's
//! `area` is a geohash prefix that `parse_area` REFUSES to let get street-level,
//! because a market posture broadcasts position and a doorstep is not publishable. A
//! Place is the precise point you stood on, held privately until you choose to share
//! the object. Coarsening a Place would destroy the only thing it is for.
//!
//! # Ops
//!
//! Owner-sequenced, like Thing. A Place is the owner's assertion about somewhere they
//! went; a member you share it with does not get to move it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::coordinator::{ArgVal, Args};
use crate::geo::{self, LocationSource};
use crate::object::{Authority, Commutativity, DeltaRejection, ObjectKind, ObjectType, Op, OpDecl};
use crate::object_args::{opt_text, req_text};

// ---- op ids (MUST match pacific-ffi/src/delta.rs `op::PLACE_*`) --------------------
pub const OP_SET_PROFILE: u32 = 0; // owner / sequenced
pub const OP_SET_ACCESS: u32 = 1; // owner / sequenced
pub const OP_SET_DOORBELL: u32 = 2; // owner / sequenced
pub const OP_SET_LAND: u32 = 3; // owner / sequenced
pub const OP_CLEAR_LAND: u32 = 4; // owner / sequenced
pub const OP_POST: u32 = 5; // any-member / commutative (per-(author,note) LWW)

/// Which register a land claim came out of. Mandatory on every claim, because an
/// unsourced assertion about who owns somewhere is worse than no assertion: it looks
/// like a fact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LandSource {
    /// HMLR INSPIRE Index Polygons — free, OGL, England & Wales, MONTHLY bulk GML.
    /// Resolves the parcel and whether it is registered at all. Carries NO proprietor.
    Inspire,
    /// HMLR "UK companies that own property in England and Wales" — corporate
    /// proprietors only. Excludes private individuals, charities, and UK companies with
    /// overseas addresses.
    Ccod,
    /// HMLR overseas-company equivalent of CCOD.
    Ocod,
    /// Registers of Scotland (ScotLIS). A separate register — Scotland is not covered by
    /// any HMLR dataset.
    Scotlis,
    /// A human typed it in, or read it off a paper title. No dataset backs it.
    Manual,
}

impl LandSource {
    pub fn parse(s: &str) -> Result<Self, DeltaRejection> {
        Ok(match s {
            "inspire" => Self::Inspire,
            "ccod" => Self::Ccod,
            "ocod" => Self::Ocod,
            "scotlis" => Self::Scotlis,
            "manual" => Self::Manual,
            _ => return Err(DeltaRejection::MalformedArgs),
        })
    }
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inspire => "inspire",
            Self::Ccod => "ccod",
            Self::Ocod => "ocod",
            Self::Scotlis => "scotlis",
            Self::Manual => "manual",
        }
    }
    /// Whether this source can name a proprietor at all. INSPIRE cannot — it is geometry.
    /// A claim that names an owner and cites INSPIRE is incoherent and gets rejected.
    pub const fn names_proprietors(self) -> bool {
        !matches!(self, Self::Inspire)
    }
}

/// Whether whoever owns this land has been ASKED, and what they said.
///
/// SEPARATE FROM THE REGISTRY ON PURPOSE. A register tells you who to ask; it cannot tell
/// you whether they said yes. Geocaching learned this the hard way and made written
/// landowner permission a precondition of listing — and its whole enforcement model is
/// volunteer reviewers demanding evidence, which a p2p network does not have.
///
/// So this is a self-reported claim by whoever put the sign up, and it defaults to
/// `Unknown`. It is recorded rather than enforced, so that "nobody asked" is a visible
/// state instead of an unexamined assumption.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LandPermission {
    /// Nobody has said anything. The honest default.
    #[default]
    Unknown,
    /// Asked, no answer yet.
    Asked,
    /// The proprietor agreed. A human claim, not a verifiable one.
    Granted,
    /// The proprietor said no. Kept rather than deleted — a refusal is exactly the thing
    /// that must not quietly disappear when someone re-anchors here later.
    Refused,
}

impl LandPermission {
    pub fn parse(s: &str) -> Result<Self, DeltaRejection> {
        Ok(match s {
            "unknown" => Self::Unknown,
            "asked" => Self::Asked,
            "granted" => Self::Granted,
            "refused" => Self::Refused,
            _ => return Err(DeltaRejection::MalformedArgs),
        })
    }
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Asked => "asked",
            Self::Granted => "granted",
            Self::Refused => "refused",
        }
    }
}

/// What register says about the land a Place sits on — A CLAIM, NOT A FACT.
///
/// The core cannot reach HM Land Registry, and the free datasets are monthly bulk files
/// rather than a point-query API. So resolution happens outside (ingest INSPIRE for the
/// parcel, join CCOD/OCOD for a corporate proprietor) and the ANSWER is authored in here
/// as a signed delta by whoever did the lookup. Everything needed to judge the claim
/// travels with it: which register, and how old the data was.
///
/// What this is for: a bench or an allotment is almost always council-owned, and a council
/// is a corporate proprietor — so it IS in CCOD, and a Place can say whose land it is on.
/// What it cannot do is cover a private individual: no free or bulk route exists, by
/// design, which is precisely the case where you would most want to know.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LandClaim {
    /// HMLR title number, or the INSPIRE id when only the polygon resolved. Never empty —
    /// a claim that does not say WHICH parcel is not a claim.
    pub parcel: String,
    /// Registered proprietor, "" when the source cannot name one (INSPIRE) or the parcel
    /// is held by someone the free datasets exclude (a private individual).
    #[serde(default)]
    pub proprietor: String,
    /// Companies House number when the proprietor is a UK company — the join key back to
    /// the corporate record. "" otherwise.
    #[serde(default)]
    pub company_no: String,
    pub source: LandSource,
    /// The DATASET VINTAGE in unix ms, not the time of lookup. These are monthly files, so
    /// a reader has to be able to see how stale the claim is; ownership changes and this
    /// does not follow it.
    pub as_of: i64,
    /// Whether the proprietor was asked, and what they said. See [`LandPermission`] —
    /// this is the part no register can answer.
    #[serde(default)]
    pub permission: LandPermission,
}

impl LandClaim {
    /// Whether this claim actually identifies an owner you could go and ask.
    pub fn names_an_owner(&self) -> bool {
        !self.proprietor.is_empty()
    }
    /// Whether the land's owner is on record as having agreed.
    pub fn is_permitted(&self) -> bool {
        self.permission == LandPermission::Granted
    }
}

/// Who gets in. A PUBLIC PLACE is not a separate object kind — it is a Place in a
/// different access posture, exactly as a Thing's market posture is state on the Thing
/// rather than a kind of its own.
///
/// `Public` means AUTO-ACCEPTED: anyone holding the Place's QR is admitted by the host,
/// with no per-person approval. Opening the Place IS the consent — asking the owner to
/// confirm each arrival would be asking them to re-make a decision they already made,
/// and a poster on a wall nobody is watching would admit nobody.
///
/// The host still performs the MLS Add, because MLS has no way to let an unknown party
/// into a group without a member committing them. That is a mechanism, not a gate: it is
/// automatic (`Node::reconcile_place_joins`) and needs no human. The alternative — an
/// external-commit join off a published GroupInfo — was rejected because GroupInfo is
/// per-epoch (a static QR would work for exactly one scanner) and publishing it would
/// make the Place readable by anyone who ever photographed the poster.
///
/// So: the host's device must be running for someone to get in, and closing the Place
/// stops future admissions without evicting anyone already inside.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Access {
    /// Invite-only. The QR is meaningless; membership comes from the owner reaching out.
    #[default]
    Private,
    /// Anyone with the QR is let in automatically. Still not world-readable — they join
    /// the group, they do not read it from outside.
    Public,
}

impl Access {
    pub fn parse(s: &str) -> Result<Self, DeltaRejection> {
        match s {
            "private" => Ok(Access::Private),
            "public" => Ok(Access::Public),
            _ => Err(DeltaRejection::MalformedArgs),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Access::Private => "private",
            Access::Public => "public",
        }
    }

    /// Whether a stranger holding the QR is let in.
    pub const fn admits_strangers(self) -> bool {
        matches!(self, Access::Public)
    }
}

/// The compiled read-side state of a Place.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlaceState {
    /// THE PARTS THIS OBJECT IS MADE OF, keyed by the part's object id — its comments
    /// section is one, a real Forum GroupObject with its own roster and not a field
    /// on this one. See `crate::parts`.
    pub parts: std::collections::BTreeMap<String, crate::object::PartRef>,
    /// THE HALVES THIS OBJECT DECLARES of relations another object asserted.
    /// Keyed `(rel, object)`. Reciprocity is required (25 Sep 2026): a one-sided
    /// edge cannot be walked from the far end, and a kind that is not a Group had
    /// no way to write its half at all.
    pub backlinks: std::collections::BTreeMap<(String, String), crate::backlink::Backlink>,
    /// What the user calls it — "the kitchen", "Sheep Lane". May be empty before the
    /// first `setProfile`.
    pub name: String,
    /// One line of context. May be empty.
    pub descriptor: String,
    /// The common location facet. `None` means the Place has no coordinate yet — a real
    /// state, not a missing value: an object can exist between `object_new` and the
    /// `setLocation` that places it, and a Place whose fix never arrived is honestly
    /// unplaced rather than sitting at (0, 0) off the coast of Africa.
    pub location: Option<LocationSource>,
    /// Whether strangers holding the QR are let in automatically. Defaults to `Private` —
    /// safe by default, because a Place that opens by accident admits people you never
    /// chose, and closing it afterwards does not evict them.
    pub access: Access,
    /// THE DOORBELL — a 32-byte capability that is both the mailbox address strangers
    /// knock at and the key that seals what they leave. `None` until the place is opened.
    ///
    /// It is deliberately printed on the poster, so it is public by construction. It is
    /// NOT buying secrecy — anyone with the sign can read who is arriving, which for a
    /// public allotment gate is the roster of a public allotment. What it buys is
    /// REVOCATION, which nothing else provides:
    ///
    ///   * Author a new one and every printed poster goes inert. That is the only way to
    ///     retire a sign that ended up somewhere it should not have.
    ///   * It is the only way to cut off an EX-MEMBER. Removing someone from the MLS group
    ///     does not retract what they already folded out of this log, so they keep the old
    ///     doorbell and can keep watching arrivals until it is rotated.
    ///
    /// Living in the Place's log (rather than on one person's device) is what lets ANY
    /// member drain the doorbell — which is what takes the founder's phone off the
    /// critical path.
    pub doorbell: Option<[u8; 32]>,
    /// Whose land this sits on, as far as anyone has established. `None` means nobody has
    /// looked — which is a real and common state, not a gap to be filled with a guess.
    pub land: Option<LandClaim>,
    /// How far the fact of this Place's existence may TRAVEL — the common
    /// visibility facet ([`crate::visibility`]), NOT membership and NOT `access`:
    /// the roster says who is in, `access` says who may join, this says who may
    /// learn it exists (what discovery answers carry, and how far they relay).
    /// Defaults to `Private`: nothing is advertised because nobody said it could be.
    pub visibility: crate::visibility::Visibility,
    /// Notes members have published AT this Place, keyed `(author, note_id)`. The
    /// kind's first any-member arm: posting a note is not moving the Place, so it
    /// breaks the owner-sequenced rule deliberately rather than by accident. The
    /// note id is the AUTHOR'S dedup key — re-publishing the same note replaces
    /// their copy instead of stacking a duplicate.
    pub posts: BTreeMap<(crate::object::MemberId, String), PlacePost>,
    /// What this object is a part of, and in what role (`base.setParent`). `None`
    /// until a parent names it as a part.
    pub parent: Option<crate::parent::ParentRef>,
}

/// One note published at a Place by one member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlacePost {
    /// The MLS-authenticated delta author — never on the wire.
    pub author: crate::object::MemberId,
    /// The author's note id (their device-local note UUID) — the dedup/LWW key.
    pub note: String,
    /// May be empty; the note body is the substance.
    pub title: String,
    pub text: String,
    /// Author-clock ms — display order, and the LWW key for re-publishes.
    pub at: i64,
}

impl PlaceState {
    /// The fixed point this Place resolves to, if it has one. A `Stream` source yields
    /// nothing here by design — its positions are out-of-band, so there is no persisted
    /// coordinate to return (see `geo.rs`).
    pub fn point(&self) -> Option<&geo::GeoPoint> {
        match &self.location {
            Some(LocationSource::Fixed { point }) => Some(point),
            _ => None,
        }
    }

    /// The remote object id this Place was ADOPTED from, if any — the discovered
    /// reference's id, carried in the point's provenance slot. THE minted-wins
    /// dedup key: `place_adopt` returns the existing Place when it matches, and
    /// `by_kind_hops` stops listing the remote row. One accessor so the two can
    /// never disagree about what "adopted" means.
    pub fn adopted_from(&self) -> Option<&str> {
        self.point()
            .map(|p| p.external_id.as_str())
            .filter(|id| !id.is_empty())
    }

    /// The posts in display order — newest first, deterministic across devices.
    pub fn posts_by_time(&self) -> Vec<&PlacePost> {
        let mut v: Vec<&PlacePost> = self.posts.values().collect();
        v.sort_by(|a, b| b.at.cmp(&a.at).then(a.author.cmp(&b.author)));
        v
    }
}

pub struct PlaceType;

/// Place's own ops, then the BASE location op-group spliced in.
///
/// The two base entries are written out rather than concatenated because
/// [`crate::geo::LOCATION_OPS`] is a `static` and Rust cannot read a static in a const
/// initialiser. `base_location_ops_are_spliced_verbatim` below asserts they stay
/// identical to geo's, so the duplication cannot drift silently.
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
        name: "place.setProfile",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SET_ACCESS,
        name: "place.setAccess",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SET_DOORBELL,
        name: "place.setDoorbell",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SET_LAND,
        name: "place.setLand",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_CLEAR_LAND,
        name: "place.clearLand",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_POST,
        name: "place.post",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
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
    // The base VISIBILITY op, spliced like geo's — written out for the same
    // const-initialiser reason; `base_visibility_op_is_spliced_verbatim` pins it.
    OpDecl {
        op_id: crate::visibility::OP_SET_VISIBILITY,
        name: "base.setVisibility",
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
];

impl ObjectType for PlaceType {
    const KIND: ObjectKind = ObjectKind::Place;
    type State = PlaceState;

    fn ops() -> &'static [OpDecl] {
        OPS
    }

    fn reduce(state: &mut Self::State, op: &Op<'_>) -> Result<(), DeltaRejection> {
        if crate::parent::is_parent_op(op.op_id) {
            return crate::parent::reduce_parent(&mut state.parent, op);
        }
        if crate::parts::is_part_op(op.op_id) {
            return crate::parts::reduce_parts(&mut state.parts, op);
        }
        if crate::backlink::is_backlink_op(op.op_id) {
            return crate::backlink::reduce_backlink(&mut state.backlinks, op);
        }
        // Base ops first: they own reserved id bands, so this can never shadow a
        // Place op, and routing them here keeps each facet's validation in ONE
        // place (geo::reduce_location, visibility::reduce_visibility) rather than
        // reimplemented per kind.
        if geo::is_location_op(op.op_id) {
            return geo::reduce_location(&mut state.location, op);
        }
        if crate::visibility::is_visibility_op(op.op_id) {
            return crate::visibility::reduce_visibility(&mut state.visibility, op);
        }
        match op.op_id {
            OP_SET_PROFILE => {
                // Validate before mutating: a reduce-time rejection is skipped at fold,
                // so a half-applied profile would be the state forever.
                let name = req_text(op.args, "name")?.to_string();
                let descriptor = opt_text(op.args, "descriptor").unwrap_or_default();
                state.name = name;
                state.descriptor = descriptor;
                Ok(())
            }
            OP_SET_ACCESS => {
                // Parsed before assignment: an unknown value is a malformed delta, never
                // a silent fallback to Private (which would look like the owner had
                // closed the Place) or to Public (which would open it by typo).
                state.access = Access::parse(req_text(op.args, "access")?)?;
                Ok(())
            }
            OP_SET_LAND => {
                // Validate the WHOLE claim before storing any of it. A half-applied land
                // claim would be a permanent assertion about somebody's property.
                let json = req_text(op.args, "claim")?;
                let claim: LandClaim =
                    serde_json::from_str(json).map_err(|_| DeltaRejection::MalformedArgs)?;
                if claim.parcel.is_empty() {
                    return Err(DeltaRejection::MalformedArgs); // which parcel?
                }
                if claim.as_of <= 0 {
                    // An undated claim cannot be judged for staleness, and ownership moves.
                    return Err(DeltaRejection::MalformedArgs);
                }
                if claim.names_an_owner() && !claim.source.names_proprietors() {
                    // INSPIRE is geometry; it cannot tell you who owns anything. A claim
                    // citing it while naming an owner is either confused or dressing up a
                    // guess in a free dataset's authority. Refuse it.
                    return Err(DeltaRejection::PreconditionFailed);
                }
                state.land = Some(claim);
                Ok(())
            }
            OP_CLEAR_LAND => {
                // Retracting is a real act — the lookup was wrong, or the land changed
                // hands. Better than leaving a stale claim standing.
                state.land = None;
                Ok(())
            }
            OP_SET_DOORBELL => {
                // Rotation is just re-authoring: the newest delta on the spine wins, and
                // every poster carrying the previous value stops resolving. A malformed
                // key must NOT clear the slot — that would silently retire a working sign.
                let hex = req_text(op.args, "doorbell")?;
                let raw = hex_to_32(hex)?;
                state.doorbell = Some(raw);
                Ok(())
            }
            OP_POST => {
                // Validate everything, then commit — a rejected re-publish must leave
                // the author's previous copy of this note intact.
                let note = req_text(op.args, "note")?.to_string();
                let text = req_text(op.args, "text")?.to_string();
                if note.is_empty() || text.is_empty() {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let title = opt_text(op.args, "title").unwrap_or_default();
                let at = match crate::object_args::req_int(op.args, "at")? {
                    t if t > 0 => t,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                // Per-(author, note) LWW by the author's clock, text as the
                // deterministic tie-break — same order-independence contract as the
                // contact channel's discovery answers.
                let key = (*op.author, note.clone());
                if let Some(held) = state.posts.get(&key) {
                    if at < held.at || (at == held.at && text <= held.text) {
                        return Ok(());
                    }
                }
                state.posts.insert(
                    key,
                    PlacePost {
                        author: *op.author,
                        note,
                        title,
                        text,
                        at,
                    },
                );
                Ok(())
            }
            _ => Err(DeltaRejection::UnknownType),
        }
    }
}

/// 64 hex chars → 32 bytes. Anything else is a malformed delta, never a truncation.
fn hex_to_32(s: &str) -> Result<[u8; 32], DeltaRejection> {
    let raw = hex::decode(s).map_err(|_| DeltaRejection::MalformedArgs)?;
    raw.try_into().map_err(|_| DeltaRejection::MalformedArgs)
}

/// The one text arg a `place.setLand` delta carries: the JSON-encoded claim.
pub fn set_land_args(claim: &LandClaim) -> Args {
    let mut a = Args::new();
    a.insert(
        "claim".into(),
        ArgVal::Text(serde_json::to_string(claim).expect("LandClaim always serializes")),
    );
    a
}

/// The one text arg a `place.setDoorbell` delta carries.
pub fn set_doorbell_args(doorbell: &[u8; 32]) -> Args {
    let mut a = Args::new();
    a.insert("doorbell".into(), ArgVal::Text(hex::encode(doorbell)));
    a
}

/// The one text arg a `place.setProfile` delta carries.
pub fn set_profile_args(name: &str, descriptor: &str) -> Args {
    let mut a = Args::new();
    a.insert("name".into(), ArgVal::Text(name.to_string()));
    if !descriptor.is_empty() {
        a.insert("descriptor".into(), ArgVal::Text(descriptor.to_string()));
    }
    a
}

/// The one text arg a `place.setAccess` delta carries.
pub fn set_access_args(access: Access) -> Args {
    let mut a = Args::new();
    a.insert("access".into(), ArgVal::Text(access.as_str().to_string()));
    a
}

/// The args a `place.post` delta carries. `note` is the author's note id — their
/// dedup key, so publishing the same note twice replaces rather than stacks.
pub fn post_args(note: &str, title: &str, text: &str, at: i64) -> Args {
    let mut a = Args::new();
    a.insert("note".into(), ArgVal::Text(note.to_string()));
    if !title.is_empty() {
        a.insert("title".into(), ArgVal::Text(title.to_string()));
    }
    a.insert("text".into(), ArgVal::Text(text.to_string()));
    a.insert("at".into(), ArgVal::Int(at));
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::GeoPoint;
    use crate::object::ReduceContext;

    const OWNER: [u8; 32] = [7u8; 32];

    /// Apply one op to a state, building a minimal owner-sequenced Op (the spine has
    /// already gated authority by the time `reduce` runs — mirrors `geo`'s own helper).
    fn apply(state: &mut PlaceState, op_id: u32, args: &Args) -> Result<(), DeltaRejection> {
        let members = [OWNER];
        let ctx = ReduceContext {
            members: &members,
            owner: OWNER,
            epoch: 0,
        };
        let op = Op {
            op_id,
            args,
            author: &OWNER,
            pos: None,
            ctx: &ctx,
        };
        PlaceType::reduce(state, &op)
    }

    /// The splice must stay byte-identical to geo's table, or a Place would declare a
    /// base op with different authority than every other kind that adopts it later.
    #[test]
    fn base_location_ops_are_spliced_verbatim() {
        for want in geo::LOCATION_OPS {
            let got = PlaceType::op(want.op_id)
                .unwrap_or_else(|| panic!("Place is missing base op {:#x}", want.op_id));
            assert_eq!(got.name, want.name);
            assert_eq!(got.authority, want.authority);
            assert_eq!(got.commutativity, want.commutativity);
        }
    }

    /// Same drift guard for the visibility splice.
    #[test]
    fn base_visibility_op_is_spliced_verbatim() {
        for want in crate::visibility::VISIBILITY_OPS {
            let got = PlaceType::op(want.op_id)
                .unwrap_or_else(|| panic!("Place is missing base op {:#x}", want.op_id));
            assert_eq!(got.name, want.name);
            assert_eq!(got.authority, want.authority);
            assert_eq!(got.commutativity, want.commutativity);
        }
    }

    /// Visibility is its own slot — orthogonal to access (joinability) and to
    /// everything else on the object. The whole point of the facet.
    #[test]
    fn visibility_is_not_access() {
        use crate::visibility::{set_visibility_args, Visibility, OP_SET_VISIBILITY};
        let mut st = PlaceState::default();
        assert_eq!(st.visibility, Visibility::Private, "safe by default");

        apply(
            &mut st,
            OP_SET_VISIBILITY,
            &set_visibility_args(Visibility::Network),
        )
        .unwrap();
        assert_eq!(st.visibility, Visibility::Network);
        assert_eq!(
            st.access,
            Access::Private,
            "advertising a place does not open its door"
        );

        apply(&mut st, OP_SET_ACCESS, &set_access_args(Access::Public)).unwrap();
        apply(
            &mut st,
            OP_SET_VISIBILITY,
            &set_visibility_args(Visibility::Private),
        )
        .unwrap();
        assert_eq!(
            st.access,
            Access::Public,
            "going dark does not slam the door on the QR"
        );
    }

    /// Base ops live in a reserved high band so they can never collide with a type's
    /// own ops (which number from 0).
    #[test]
    fn base_band_never_collides_with_place_ops() {
        assert!(OP_SET_PROFILE < 0xF000_0000);
        assert!(geo::is_location_op(geo::OP_SET_LOCATION));
        assert!(!geo::is_location_op(OP_SET_PROFILE));
    }

    #[test]
    fn place_type_id_is_28_and_round_trips() {
        assert_eq!(ObjectKind::Place.type_id(), 28);
        assert_eq!(ObjectKind::from_type_id(28), Some(ObjectKind::Place));
        assert_eq!(ObjectKind::Place.name(), "place");
    }

    #[test]
    fn set_profile_then_location_folds() {
        let mut st = PlaceState::default();
        apply(
            &mut st,
            OP_SET_PROFILE,
            &set_profile_args("the kitchen", "top floor"),
        )
        .unwrap();
        assert_eq!(st.name, "the kitchen");
        assert_eq!(st.descriptor, "top floor");
        assert!(st.location.is_none());

        let src = LocationSource::Fixed {
            point: GeoPoint::from_degrees(51.4545, -2.5879),
        };
        apply(&mut st, geo::OP_SET_LOCATION, &geo::set_location_args(&src)).unwrap();
        let p = st.point().expect("fixed point");
        assert_eq!(p.lat_e7, 514_545_000);
        assert_eq!(p.lng_e7, -25_879_000);
        // The profile survives a location delta — separate slots, not a whole-state put.
        assert_eq!(st.name, "the kitchen");
    }

    #[test]
    fn clear_location_unplaces_but_keeps_the_place() {
        let mut st = PlaceState::default();
        apply(&mut st, OP_SET_PROFILE, &set_profile_args("the bar", "")).unwrap();
        let src = LocationSource::Fixed {
            point: GeoPoint::from_degrees(1.0, 2.0),
        };
        apply(&mut st, geo::OP_SET_LOCATION, &geo::set_location_args(&src)).unwrap();
        apply(&mut st, geo::OP_CLEAR_LOCATION, &Args::new()).unwrap();
        assert!(st.location.is_none());
        // Clearing the coordinate must not erase the Place.
        assert_eq!(st.name, "the bar");
    }

    /// A `Stream` source yields no persisted point — positions are out-of-band.
    #[test]
    fn a_stream_source_has_no_fixed_point() {
        let mut st = PlaceState::default();
        let src = LocationSource::Stream {
            access: geo::LocationStream {
                kind: "device".into(),
                ..Default::default()
            },
        };
        apply(&mut st, geo::OP_SET_LOCATION, &geo::set_location_args(&src)).unwrap();
        assert!(st.location.is_some());
        assert!(st.point().is_none());
    }

    /// An out-of-range point must leave the slot untouched rather than store nonsense.
    #[test]
    fn a_bad_coordinate_is_rejected_and_does_not_clobber() {
        let mut st = PlaceState::default();
        let good = LocationSource::Fixed {
            point: GeoPoint::from_degrees(51.0, -2.0),
        };
        apply(
            &mut st,
            geo::OP_SET_LOCATION,
            &geo::set_location_args(&good),
        )
        .unwrap();

        let bad = LocationSource::Fixed {
            point: GeoPoint {
                lat_e7: GeoPoint::LAT_MAX_E7 + 1,
                lng_e7: 0,
                ..Default::default()
            },
        };
        assert!(apply(&mut st, geo::OP_SET_LOCATION, &geo::set_location_args(&bad)).is_err());
        // Still the good point.
        assert_eq!(st.point().unwrap().lat_e7, 510_000_000);
    }

    #[test]
    fn an_unknown_op_is_rejected() {
        let mut st = PlaceState::default();
        assert_eq!(
            apply(&mut st, 999, &Args::new()),
            Err(DeltaRejection::UnknownType)
        );
    }

    /// Private by default: a Place must never become askable because nobody said otherwise.
    #[test]
    fn a_place_is_private_until_it_is_opened() {
        let mut st = PlaceState::default();
        assert_eq!(st.access, Access::Private);
        assert!(!st.access.admits_strangers());

        apply(&mut st, OP_SET_ACCESS, &set_access_args(Access::Public)).unwrap();
        assert_eq!(st.access, Access::Public);
        assert!(st.access.admits_strangers());

        // And it closes again — a public place is not a one-way door.
        apply(&mut st, OP_SET_ACCESS, &set_access_args(Access::Private)).unwrap();
        assert_eq!(st.access, Access::Private);
    }

    /// A typo must not quietly open (or close) a Place.
    #[test]
    fn an_unknown_access_value_is_malformed_not_a_default() {
        let mut st = PlaceState::default();
        apply(&mut st, OP_SET_ACCESS, &set_access_args(Access::Public)).unwrap();

        let mut bad = Args::new();
        bad.insert("access".into(), ArgVal::Text("open".into()));
        assert_eq!(
            apply(&mut st, OP_SET_ACCESS, &bad),
            Err(DeltaRejection::MalformedArgs)
        );
        assert_eq!(
            st.access,
            Access::Public,
            "the rejected op left the posture alone"
        );
    }

    /// Access, profile and location are separate slots on one object.
    #[test]
    fn access_survives_a_profile_and_a_location_delta() {
        let mut st = PlaceState::default();
        apply(&mut st, OP_SET_ACCESS, &set_access_args(Access::Public)).unwrap();
        apply(
            &mut st,
            OP_SET_PROFILE,
            &set_profile_args("the bar", "back room"),
        )
        .unwrap();
        let src = LocationSource::Fixed {
            point: GeoPoint::from_degrees(51.4545, -2.5879),
        };
        apply(&mut st, geo::OP_SET_LOCATION, &geo::set_location_args(&src)).unwrap();

        assert_eq!(st.access, Access::Public);
        assert_eq!(st.name, "the bar");
        assert!(st.point().is_some());
    }

    fn claim(parcel: &str, proprietor: &str, source: LandSource) -> LandClaim {
        LandClaim {
            parcel: parcel.into(),
            proprietor: proprietor.into(),
            company_no: String::new(),
            source,
            as_of: 1_750_000_000_000,
            permission: LandPermission::Unknown,
        }
    }

    /// The allotment case: council-owned, so CCOD can actually name it.
    #[test]
    fn a_land_claim_records_its_source_and_vintage() {
        let mut st = PlaceState::default();
        assert!(st.land.is_none(), "nobody has looked yet — a real state");

        let mut c = claim("BL123456", "BRISTOL CITY COUNCIL", LandSource::Ccod);
        c.company_no = "".into();
        c.permission = LandPermission::Granted;
        apply(&mut st, OP_SET_LAND, &set_land_args(&c)).unwrap();

        let land = st.land.clone().expect("claim stored");
        assert_eq!(land.parcel, "BL123456");
        assert!(land.names_an_owner());
        assert!(land.is_permitted());
        assert_eq!(land.source, LandSource::Ccod);
        assert_eq!(
            land.as_of, 1_750_000_000_000,
            "the DATASET vintage travels with it"
        );
    }

    /// INSPIRE is geometry. A claim citing it while naming an owner is dressing a guess up
    /// in a free dataset's authority, and must be refused rather than stored.
    #[test]
    fn inspire_cannot_name_a_proprietor() {
        let mut st = PlaceState::default();
        let bad = claim("INSPIRE-1234", "SOME PERSON", LandSource::Inspire);
        assert_eq!(
            apply(&mut st, OP_SET_LAND, &set_land_args(&bad)),
            Err(DeltaRejection::PreconditionFailed)
        );
        assert!(st.land.is_none(), "the rejected claim stored nothing");

        // INSPIRE with NO proprietor is the legitimate use — the parcel resolved, the
        // owner did not (a private individual is in no free dataset).
        let ok = claim("INSPIRE-1234", "", LandSource::Inspire);
        apply(&mut st, OP_SET_LAND, &set_land_args(&ok)).unwrap();
        let land = st.land.clone().unwrap();
        assert!(
            !land.names_an_owner(),
            "registered, owner unknown — stated honestly"
        );
        assert!(!land.is_permitted());
    }

    #[test]
    fn a_claim_must_name_a_parcel_and_be_dated() {
        let mut st = PlaceState::default();
        let mut no_parcel = claim("", "", LandSource::Inspire);
        no_parcel.parcel = String::new();
        assert_eq!(
            apply(&mut st, OP_SET_LAND, &set_land_args(&no_parcel)),
            Err(DeltaRejection::MalformedArgs)
        );

        let mut undated = claim("BL1", "", LandSource::Inspire);
        undated.as_of = 0;
        assert_eq!(
            apply(&mut st, OP_SET_LAND, &set_land_args(&undated)),
            Err(DeltaRejection::MalformedArgs),
            "ownership moves; an undated claim cannot be judged stale"
        );
    }

    /// Permission defaults to Unknown and is never inferred from ownership being known.
    #[test]
    fn knowing_the_owner_is_not_permission() {
        let mut st = PlaceState::default();
        let c = claim("BL123456", "BRISTOL CITY COUNCIL", LandSource::Ccod);
        apply(&mut st, OP_SET_LAND, &set_land_args(&c)).unwrap();
        let land = st.land.clone().unwrap();
        assert!(land.names_an_owner());
        assert_eq!(land.permission, LandPermission::Unknown);
        assert!(
            !land.is_permitted(),
            "a register says who to ask, not what they said"
        );
    }

    /// A refusal must survive — it is exactly what must not quietly vanish.
    #[test]
    fn land_can_be_retracted_but_a_refusal_is_recorded() {
        let mut st = PlaceState::default();
        let mut c = claim("BL9", "A PARISH COUNCIL", LandSource::Ccod);
        c.permission = LandPermission::Refused;
        apply(&mut st, OP_SET_LAND, &set_land_args(&c)).unwrap();
        assert_eq!(st.land.clone().unwrap().permission, LandPermission::Refused);

        apply(&mut st, OP_CLEAR_LAND, &Args::new()).unwrap();
        assert!(st.land.is_none(), "retraction is a real act");
    }

    /// Land, location, access and doorbell are independent slots on one object.
    #[test]
    fn land_survives_every_other_delta() {
        let mut st = PlaceState::default();
        apply(
            &mut st,
            OP_SET_LAND,
            &set_land_args(&claim("BL1", "COUNCIL", LandSource::Ccod)),
        )
        .unwrap();
        apply(&mut st, OP_SET_ACCESS, &set_access_args(Access::Public)).unwrap();
        apply(&mut st, OP_SET_DOORBELL, &set_doorbell_args(&[9u8; 32])).unwrap();
        apply(
            &mut st,
            OP_SET_PROFILE,
            &set_profile_args("the allotment", ""),
        )
        .unwrap();
        let src = LocationSource::Fixed {
            point: GeoPoint::from_degrees(51.4545, -2.5879),
        };
        apply(&mut st, geo::OP_SET_LOCATION, &geo::set_location_args(&src)).unwrap();

        assert_eq!(st.land.as_ref().unwrap().proprietor, "COUNCIL");
        assert_eq!(st.access, Access::Public);
        assert_eq!(st.doorbell, Some([9u8; 32]));
        assert_eq!(st.name, "the allotment");
        assert!(st.point().is_some());
    }

    #[test]
    fn set_profile_without_a_name_is_malformed() {
        let mut st = PlaceState::default();
        assert_eq!(
            apply(&mut st, OP_SET_PROFILE, &Args::new()),
            Err(DeltaRejection::MalformedArgs)
        );
    }

    // ---- place.post: the first any-member arm ------------------------------------

    const MEMBER: [u8; 32] = [8u8; 32];

    /// Like [`apply`] but authored by an arbitrary member — `place.post` is
    /// AnyMember, and the reduce must accept a non-owner author.
    fn apply_as(
        state: &mut PlaceState,
        author: &[u8; 32],
        op_id: u32,
        args: &Args,
    ) -> Result<(), DeltaRejection> {
        let members = [OWNER, MEMBER];
        let ctx = ReduceContext {
            members: &members,
            owner: OWNER,
            epoch: 0,
        };
        let op = Op {
            op_id,
            args,
            author,
            pos: None,
            ctx: &ctx,
        };
        PlaceType::reduce(state, &op)
    }

    #[test]
    fn a_member_posts_a_note_and_the_owner_keeps_the_place() {
        let mut st = PlaceState::default();
        apply(&mut st, OP_SET_PROFILE, &set_profile_args("the café", "")).unwrap();
        apply_as(
            &mut st,
            &MEMBER,
            OP_POST,
            &post_args("note-1", "Wifi", "Password is on the till", 1_000),
        )
        .unwrap();

        let posts = st.posts_by_time();
        assert_eq!(posts.len(), 1);
        assert_eq!(
            posts[0].author, MEMBER,
            "attributed to who posted, not the owner"
        );
        assert_eq!(posts[0].title, "Wifi");
        assert_eq!(posts[0].text, "Password is on the till");
        assert_eq!(st.name, "the café", "posting is not moving the Place");
    }

    #[test]
    fn republishing_the_same_note_replaces_rather_than_stacks() {
        let mut st = PlaceState::default();
        apply_as(&mut st, &MEMBER, OP_POST, &post_args("n1", "", "v1", 1_000)).unwrap();
        apply_as(&mut st, &MEMBER, OP_POST, &post_args("n1", "", "v2", 2_000)).unwrap();
        assert_eq!(st.posts.len(), 1, "same (author, note) is one row");
        assert_eq!(st.posts_by_time()[0].text, "v2");

        // A stale replay arriving late never regresses the newer copy.
        apply_as(&mut st, &MEMBER, OP_POST, &post_args("n1", "", "v1", 1_000)).unwrap();
        assert_eq!(st.posts_by_time()[0].text, "v2");
    }

    #[test]
    fn two_authors_may_use_the_same_note_id_without_collision() {
        let mut st = PlaceState::default();
        apply_as(
            &mut st,
            &OWNER,
            OP_POST,
            &post_args("n1", "", "mine", 1_000),
        )
        .unwrap();
        apply_as(
            &mut st,
            &MEMBER,
            OP_POST,
            &post_args("n1", "", "theirs", 2_000),
        )
        .unwrap();
        assert_eq!(st.posts.len(), 2, "keyed per author");
    }

    #[test]
    fn a_post_needs_a_note_id_a_body_and_a_clock() {
        let mut st = PlaceState::default();
        for bad in [
            post_args("", "", "text", 1_000), // no note id
            post_args("n1", "", "", 1_000),   // no body
            post_args("n1", "", "text", 0),   // no clock
        ] {
            assert_eq!(
                apply_as(&mut st, &MEMBER, OP_POST, &bad),
                Err(DeltaRejection::MalformedArgs)
            );
        }
        assert!(st.posts.is_empty());
    }

    #[test]
    fn posts_sort_newest_first_deterministically() {
        let mut st = PlaceState::default();
        apply_as(&mut st, &OWNER, OP_POST, &post_args("a", "", "old", 1_000)).unwrap();
        apply_as(&mut st, &MEMBER, OP_POST, &post_args("b", "", "new", 2_000)).unwrap();
        let v = st.posts_by_time();
        assert_eq!(v[0].text, "new");
        assert_eq!(v[1].text, "old");
    }
}
