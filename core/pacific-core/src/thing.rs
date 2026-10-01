//! `Thing` — a concrete thing in your world, and the object a MARKET posture hangs on.
//!
//! A Thing is minted from a resolved entity once you ACCEPT it at the swipe deck: the
//! resolver proposes ("is this a real thing worth tracking?"), you decide, and only then
//! does a durable, signed, syncable GroupObject exist. Extraction is probabilistic; a
//! GroupObject is a commitment — the swipe is the membrane between the two.
//!
//! # The posture is state, not a sixth object kind
//!
//! Per the market model: **Wants / Buying** are desideratives (a desire to acquire) and
//! **Offers / Selling** are commissives (committing yourself to provide), while **Has** is
//! simply holding the thing. Each is a posture *toward one thing*, so it lives here as the
//! thing's state rather than as its own type:
//!
//! | | latent (standing) | active (on-market now) |
//! |---|---|---|
//! | acquire | `Wants` | `Buying` |
//! | provide | `Has` → `Offers` | `Selling` |
//!
//! `price` and `deadline` are meaningful only once a posture goes ACTIVE — a standing Want
//! has no price. They are optional on every posture rather than modelled as a separate
//! active type, because the same thing moves between postures without becoming a new object.
//!
//! There is deliberately no demand-side counterpart to `Has`: not-wanting is not something
//! anyone lists. The asymmetry is real; it is not an oversight to be tidied away.
//!
//! # Ops
//!
//! All owner-sequenced. A Thing is a group of 1 until you share it, and even shared, its
//! profile and posture are the owner's assertions about their own thing — a member cannot
//! declare that *you* are selling something. Sharing is what makes a posture visible to a
//! peer (an owner-sequenced MLS Add), not what makes it editable by them.

use crate::coordinator::ArgVal;
use crate::object::{Authority, Commutativity, DeltaRejection, ObjectKind, ObjectType, Op, OpDecl};
// thing.rs's optional-text lens skips empty strings — that is `opt_text_nonempty`.
use crate::object_args::{opt_text_nonempty as opt_text, req_text};

// ---- op ids (MUST match pacific-ffi/src/delta.rs `op::THING_*`) --------------------
pub const OP_SET_PROFILE: u32 = 0; // owner / sequenced
pub const OP_SET_POSTURE: u32 = 1; // owner / sequenced
pub const OP_CLEAR_POSTURE: u32 = 2; // owner / sequenced
pub const OP_SET_PHOTO: u32 = 3; // owner / sequenced
/// The listing's photos beside its face (W-98 Trade, N3): one each, keyed by id.
pub const OP_ADD_PHOTO: u32 = 4; // owner / sequenced
pub const OP_REMOVE_PHOTO: u32 = 5; // owner / sequenced
/// Whether it is still for sale: available, reserved for one member, or sold (N5).
pub const OP_SET_DISPOSITION: u32 = 6; // owner / sequenced

/// thing.setProfile's `condition` (N1), as the ICD's vocabulary names them.
pub const CONDITIONS: [&str; 6] = ["new", "like_new", "very_good", "good", "fair", "for_parts"];
/// thing.setDisposition's `state` (N5).
pub const DISPOSITIONS: [&str; 3] = ["available", "reserved", "sold"];
/// thing.setProfile's `description`, in characters (the ICD's maxLength).
pub const MAX_DESCRIPTION: usize = 5000;
/// thing.addPhoto's inline photo, base64 characters (the ICD's maxLength): its own Delta
/// must fit the carriage ceiling.
pub const MAX_ADDED_PHOTO_B64: usize = 150_000;
/// Live photos a listing holds (the ICD's "at most 20 live"; Vinted 20, eBay 24).
pub const MAX_PHOTOS: usize = 20;

/// One of a listing's photos (`thing.addPhoto`).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ThingPhoto {
    pub id: String,
    pub mime: String,
    /// Inline: the bytes, base64. Empty when detached.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub data: String,
    /// Detached (O-79): the bytes' digest (hex), size, object key and sealed content key,
    /// which a reader on the roster fetches and opens; empty when inline.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub key: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub secret: String,
    pub width: u32,
    pub height: u32,
    pub at: i64,
}

/// Whether the thing is still for sale (`thing.setDisposition`).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Disposition {
    pub state: String,
    #[serde(rename = "for", skip_serializing_if = "Option::is_none")]
    pub for_member: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction: Option<String>,
    pub at: i64,
}

fn hex_of(t: &str, n: usize) -> bool {
    t.len() == n && t.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Where this thing stands in the market. Parsed from the wire string; an unknown value is
/// a malformed delta rather than a silent default, so a typo can never quietly become `has`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Posture {
    Has,
    Wants,
    Offers,
    Buying,
    Selling,
}

impl Posture {
    pub fn parse(s: &str) -> Result<Self, DeltaRejection> {
        match s {
            "has" => Ok(Posture::Has),
            "wants" => Ok(Posture::Wants),
            "offers" => Ok(Posture::Offers),
            "buying" => Ok(Posture::Buying),
            "selling" => Ok(Posture::Selling),
            _ => Err(DeltaRejection::MalformedArgs),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Posture::Has => "has",
            Posture::Wants => "wants",
            Posture::Offers => "offers",
            Posture::Buying => "buying",
            Posture::Selling => "selling",
        }
    }

    /// On-market now, as opposed to a standing intent. Only an active posture can carry a
    /// price sensibly.
    pub const fn is_active(self) -> bool {
        matches!(self, Posture::Buying | Posture::Selling)
    }
}

/// What KIND of listable this thing is — the market face's category axis. `Artifact`
/// is a physical good (INVENTORY · TRADE), `Skill` a capability someone provides
/// (SKILLS), `Job` work moving between people (JOBS). Closed vocabulary, like
/// `Posture`: a typo'd category is a malformed delta, never a silent fifth face.
/// Absent on the wire = `Artifact`, so every pre-category log folds identically.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Category {
    #[default]
    Artifact,
    Skill,
    Job,
}

impl Category {
    pub fn parse(s: &str) -> Result<Self, DeltaRejection> {
        match s {
            "artifact" => Ok(Category::Artifact),
            "skill" => Ok(Category::Skill),
            "job" => Ok(Category::Job),
            _ => Err(DeltaRejection::MalformedArgs),
        }
    }
    pub const fn as_str(self) -> &'static str {
        match self {
            Category::Artifact => "artifact",
            Category::Skill => "skill",
            Category::Job => "job",
        }
    }
}

/// How far a posture may travel. This is the CONSENT control, and it is also the
/// identity-disclosure control: a listing that reaches the network carries the origin
/// signature, so `Network` means "my identity travels with this".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Reach {
    /// Direct connections only. Never forwarded. The "sell privately" case.
    #[default]
    Private,
    /// May be forwarded by a recipient's agent, attenuating as it goes.
    Network,
}

impl Reach {
    pub fn parse(s: &str) -> Result<Self, DeltaRejection> {
        match s {
            "private" => Ok(Reach::Private),
            "network" => Ok(Reach::Network),
            _ => Err(DeltaRejection::MalformedArgs),
        }
    }
    pub const fn as_str(self) -> &'static str {
        match self {
            Reach::Private => "private",
            Reach::Network => "network",
        }
    }
}

/// Geohash base-32. Deliberately excludes a, i, l, o — the characters that misread.
const GEOHASH_ALPHABET: &[u8] = b"0123456789bcdefghjkmnpqrstuvwxyz";

/// The coarsest and finest areas a listing may publish.
///
/// The floor is not arbitrary: 6 characters is roughly 1.2 x 0.6 km — a street — and
/// publishing that to a network that forwards is publishing where you live. The engine
/// refuses it rather than trusting every caller to remember.
pub const AREA_MIN_PRECISION: usize = 3;
pub const AREA_MAX_PRECISION: usize = 5;

/// Truncate a position to a geohash prefix.
///
/// TRUNCATION, NOT JITTER, and the difference matters: random noise added per listing
/// averages out across a seller's history, so a network that sees ten of your offers can
/// triangulate you. Truncation is idempotent — the same cell every time, nothing to average.
pub fn geohash(lat: f64, lon: f64, precision: usize) -> String {
    let p = precision.clamp(AREA_MIN_PRECISION, AREA_MAX_PRECISION);
    let (mut lat_lo, mut lat_hi) = (-90.0_f64, 90.0_f64);
    let (mut lon_lo, mut lon_hi) = (-180.0_f64, 180.0_f64);
    let mut out = String::with_capacity(p);
    let mut bit = 0;
    let mut idx = 0usize;
    let mut even = true; // longitude first, per the spec

    while out.len() < p {
        if even {
            let mid = (lon_lo + lon_hi) / 2.0;
            if lon > mid {
                idx = idx * 2 + 1;
                lon_lo = mid;
            } else {
                idx *= 2;
                lon_hi = mid;
            }
        } else {
            let mid = (lat_lo + lat_hi) / 2.0;
            if lat > mid {
                idx = idx * 2 + 1;
                lat_lo = mid;
            } else {
                idx *= 2;
                lat_hi = mid;
            }
        }
        even = !even;
        bit += 1;
        if bit == 5 {
            out.push(GEOHASH_ALPHABET[idx] as char);
            bit = 0;
            idx = 0;
        }
    }
    out
}

/// Reject anything that is not a publishable area: wrong charset, or fine enough to be a
/// street. Loud, because a bad area silently stored is a listing that cannot be routed.
fn parse_area(s: &str) -> Result<String, DeltaRejection> {
    if s.len() < AREA_MIN_PRECISION || s.len() > AREA_MAX_PRECISION {
        return Err(DeltaRejection::MalformedArgs);
    }
    if !s.bytes().all(|b| GEOHASH_ALPHABET.contains(&b)) {
        return Err(DeltaRejection::MalformedArgs);
    }
    Ok(s.to_string())
}

/// The compiled read-side state of a Thing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThingState {
    /// THE PARTS THIS OBJECT IS MADE OF, keyed by the part's object id — its comments
    /// section is one, a real Forum GroupObject with its own roster and not a field
    /// on this one. See `crate::parts`.
    pub parts: std::collections::BTreeMap<String, crate::object::PartRef>,
    /// THE HALVES THIS OBJECT DECLARES of relations another object asserted.
    /// Keyed `(rel, object)`. Reciprocity is required (25 Sep 2026): a one-sided
    /// edge cannot be walked from the far end, and a kind that is not a Group had
    /// no way to write its half at all.
    pub backlinks: std::collections::BTreeMap<(String, String), crate::backlink::Backlink>,
    pub name: String,
    /// One line — "cracked handle", "22in, blue". May be empty.
    pub descriptor: String,
    /// Which market face this thing belongs to (artifact | skill | job).
    pub category: Category,
    /// `None` means the thing is tracked but takes no market position. That is a real
    /// state, not a missing value: you can know about a thing without wanting or having it.
    pub posture: Option<Posture>,
    /// Free text ("£15", "swap for a drill"). Kept as text on purpose — a price is a
    /// human offer, not a currency-typed amount, and forcing a numeric type here would
    /// lose "make me an offer".
    pub price: Option<String>,
    /// Epoch ms the posture lapses. `None` = standing.
    pub deadline: Option<i64>,
    /// How far this posture may travel. Defaults to `Private` — safe by default, because a
    /// posture that leaks is not a mistake anyone can take back.
    pub reach: Reach,
    /// Coarse area as a geohash prefix, `None` when the thing is not a local good
    /// (shippable, digital) and geography should sit out of routing entirely.
    pub area: Option<String>,
    /// The listing's FACE: one still, base64-in-delta (the ContactCard/EventMedia
    /// precedent — no external fetch, a URL would reintroduce a server). "" = no
    /// photo, a real state the card renders as stripes, never a placeholder image.
    pub photo: String,
    /// MIME of `photo` ("image/jpeg" when the app set it); "" when no photo.
    pub photo_mime: String,
    /// What this object is a part of, and in what role (`base.setParent`). `None`
    /// until a parent names it as a part.
    pub parent: Option<crate::parent::ParentRef>,
    /// The item's condition (N1); `None` is unstated.
    pub condition: Option<String>,
    /// The listing's long description, several lines (N2); `descriptor` stays the one line.
    pub description: String,
    /// The listing's photos by id (N3), and the ids removed for good.
    pub photos: std::collections::BTreeMap<String, ThingPhoto>,
    pub photos_removed: std::collections::BTreeSet<String>,
    /// Available, reserved or sold (N5); `None` is never set.
    pub disposition: Option<Disposition>,
}

pub struct ThingType;

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
        name: "thing.setProfile",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SET_POSTURE,
        name: "thing.setPosture",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_CLEAR_POSTURE,
        name: "thing.clearPosture",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SET_PHOTO,
        name: "thing.setPhoto",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_ADD_PHOTO,
        name: "thing.addPhoto",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_REMOVE_PHOTO,
        name: "thing.removePhoto",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SET_DISPOSITION,
        name: "thing.setDisposition",
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

impl ObjectType for ThingType {
    const KIND: ObjectKind = ObjectKind::Thing;
    type State = ThingState;

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
        let args = op.args;
        match op.op_id {
            OP_SET_PROFILE => {
                // Parse everything before the first mutation — the posture arm's rule.
                let name = req_text(args, "name")?.to_string();
                let category = match opt_text(args, "category") {
                    Some(c) => Category::parse(&c)?,
                    None => Category::Artifact,
                };
                let condition = opt_text(args, "condition");
                if condition.as_deref().is_some_and(|c| !CONDITIONS.contains(&c)) {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let description = opt_text(args, "description").unwrap_or_default();
                if description.chars().count() > MAX_DESCRIPTION {
                    return Err(DeltaRejection::MalformedArgs);
                }
                state.name = name;
                state.descriptor = opt_text(args, "descriptor").unwrap_or_default();
                state.category = category;
                state.condition = condition;
                state.description = description;
                Ok(())
            }
            OP_SET_POSTURE => {
                // VALIDATE EVERYTHING FIRST, THEN COMMIT. A reducer that mutates as it parses
                // leaves half a posture behind when a later field is bad — and since a
                // reduce-time rejection is skipped at fold, that half would be the state
                // forever. Every `?` below must fire before a single field is written.
                let posture = Posture::parse(req_text(args, "posture")?)?;
                let price = opt_text(args, "price");
                // A price on a standing intent is meaningless — a Want has no price until it
                // becomes Buying. Reject rather than silently drop it, so the caller learns
                // its model is wrong instead of losing data quietly.
                if price.is_some() && !posture.is_active() {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                let deadline = match crate::arg_reads::get(args, "deadline") {
                    Some(ArgVal::Int(ms)) => Some(*ms),
                    None => None,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                // Absent = Private. The safe reading of silence, so a caller that forgets the
                // field under-shares rather than over-shares.
                let reach = match opt_text(args, "reach") {
                    Some(r) => Reach::parse(&r)?,
                    None => Reach::Private,
                };
                let area = match opt_text(args, "area") {
                    Some(a) => Some(parse_area(&a)?),
                    None => None,
                };

                state.posture = Some(posture);
                state.price = price;
                state.deadline = deadline;
                state.reach = reach;
                state.area = area;
                Ok(())
            }
            OP_SET_PHOTO => {
                // ONE still per thing — the marketplace thumbnail, not a gallery.
                // Same cap as an event photo (one cap, one place: event.rs), and
                // oversize is REFUSED, never re-encoded — a silently re-encoding
                // engine makes two devices disagree about the same delta. Empty
                // photo + empty mime CLEARS the slot (taking the picture down is
                // a real state); the two must agree on emptiness.
                let photo = opt_text(args, "photo").unwrap_or_default();
                let mime = opt_text(args, "mime").unwrap_or_default();
                if photo.len() > crate::event::MAX_PHOTO_B64
                    || (photo.is_empty() != mime.is_empty())
                {
                    return Err(DeltaRejection::MalformedArgs);
                }
                state.photo = photo;
                state.photo_mime = mime;
                Ok(())
            }
            OP_CLEAR_POSTURE => {
                // Clearing takes the price and deadline with it: they qualify a posture and
                // are meaningless once it is gone.
                state.posture = None;
                state.price = None;
                state.deadline = None;
                // Reach and area qualify a posture; with none, publishing scope is meaningless
                // and leaving them set would let a later posture inherit an unstated audience.
                state.reach = Reach::Private;
                state.area = None;
                Ok(())
            }
            OP_ADD_PHOTO => {
                // Each arg judged as it is read: the id, the photo, then when.
                let id = req_text(args, "id")?.to_string();
                if !hex_of(&id, 16) {
                    return Err(DeltaRejection::MalformedArgs);
                }
                // The photo's own args, each judged as it is read: a picture and its still's
                // mime, or neither; delivered inline (detached waits for O-79's client half);
                // a still.
                let text = |k: &str| match crate::arg_reads::get(args, k) {
                    Some(ArgVal::Text(t)) => Ok(t.clone()),
                    None => Ok(String::new()),
                    _ => Err(DeltaRejection::MalformedArgs),
                };
                let body = text("photo")?;
                {
                    use base64::Engine;
                    if body.len() > MAX_ADDED_PHOTO_B64 || (!body.is_empty() && base64::engine::general_purpose::STANDARD.decode(&body).is_err()) {
                        return Err(DeltaRejection::MalformedArgs);
                    }
                }
                // `photoVia` is absent, inline or detached: the media crate reads an empty one
                // as neither.
                let via = match crate::arg_reads::get(args, "photoVia") {
                    None => "inline".to_string(),
                    Some(ArgVal::Text(v)) if v == "inline" || v == "detached" => v.clone(),
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                // A still's mime, beside a picture inline or a reference detached; neither, for
                // an empty photo.
                let mime = text("photoMime")?;
                let still = crate::media::MediaKind::of_mime(&mime) == Some(crate::media::MediaKind::Still);
                let shaped = if via == "detached" { still && body.is_empty() } else { body.is_empty() == mime.is_empty() && (mime.is_empty() || still) };
                if !shaped {
                    return Err(DeltaRejection::MalformedArgs);
                }
                if !matches!(text("photoKind")?.as_str(), "" | "still") {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let m = match crate::media::MediaRef::from_args_by("photo", |k| crate::arg_reads::get(args, k)) {
                    Some(Ok(m)) => m,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                // A live session is not a photo, and the object key and content key ride only a
                // detached photo (the ICD: "absent inline"); on a detached one, the media crate
                // holds both or neither (O-79).
                if !text("photoSession")?.is_empty() {
                    return Err(DeltaRejection::MalformedArgs);
                }
                if via != "detached" && (!text("photoKey")?.is_empty() || !text("photoSecret")?.is_empty()) {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let at = crate::object_args::req_int(args, "at")?;
                if state.photos_removed.contains(&id) {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                // An empty photo clears this id (the ICD's `photo`: "empty clears"); unlike
                // removePhoto, the id may be added again.
                if m.is_empty() {
                    state.photos.remove(&id);
                    return Ok(());
                }
                if m.kind != crate::media::MediaKind::Still || m.validate_bounds(crate::media::Slot::Photo).is_err() {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let (data, digest, bytes, key, secret) = match &m.delivery {
                    crate::media::Delivery::Inline { data } if data.len() <= MAX_ADDED_PHOTO_B64 => (data.clone(), String::new(), None, String::new(), String::new()),
                    crate::media::Delivery::Sealed { digest, bytes, key, secret } => (String::new(), hex::encode(digest), Some(*bytes), key.clone(), secret.clone()),
                    crate::media::Delivery::Detached { digest, bytes } => (String::new(), hex::encode(digest), Some(*bytes), String::new(), String::new()),
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                if !state.photos.contains_key(&id) && state.photos.len() >= MAX_PHOTOS {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                state.photos.insert(id.clone(), ThingPhoto { id, mime: m.mime.clone(), data, digest, bytes, key, secret, width: m.width, height: m.height, at });
                Ok(())
            }
            OP_REMOVE_PHOTO => {
                let id = req_text(args, "id")?.to_string();
                if !hex_of(&id, 16) {
                    return Err(DeltaRejection::MalformedArgs);
                }
                state.photos.remove(&id);
                state.photos_removed.insert(id);
                Ok(())
            }
            OP_SET_DISPOSITION => {
                // Each arg judged as it is read. `for` qualifies a hold, and a hold names its
                // member; `transaction` qualifies a sale; each is refused on another state.
                // `until` qualifies a hold and is kept only with one.
                let kind = req_text(args, "state")?.to_string();
                if !DISPOSITIONS.contains(&kind.as_str()) {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let reserved = kind == "reserved";
                let at = crate::object_args::req_int(args, "at")?;
                let for_member = opt_text(args, "for");
                if reserved != for_member.is_some() || for_member.as_deref().is_some_and(|m| !hex_of(m, 64)) {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let transaction = opt_text(args, "transaction");
                if (kind != "sold" && transaction.is_some()) || transaction.as_deref().is_some_and(|t| !hex_of(t, 64)) {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let until = match crate::arg_reads::get(args, "until") {
                    Some(ArgVal::Int(n)) => Some(*n).filter(|_| reserved),
                    None => None,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                state.disposition = Some(Disposition { state: kind, for_member, until, transaction, at });
                Ok(())
            }
            _ => Err(DeltaRejection::UnknownType),
        }
    }
}

/// What the Thing's view states against the reader's clock, applied as it is read, after
/// the fold cache (`Node::object_view`; TB4): a hold past its `until` reads available (ICD
/// `thing.setDisposition`'s view). The fold's own view keeps the hold as written.
pub fn at_read(view: &mut serde_json::Value, now_ms: i64) {
    let d = &mut view["disposition"];
    if d["state"] == "reserved" && d["until"].as_i64().is_some_and(|u| now_ms > u) {
        d["state"] = "available".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator::{sequenced_delta, Args, Coordinator, GENESIS_PREV};
    use crate::object::MemberId;

    /// W-98 Trade: the caps and vocabularies thing.rs states are the ICD's, read from it.
    #[test]
    fn w98_caps_and_vocabularies_are_the_icds() {
        let icd: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json")).unwrap()).unwrap();
        let ops = &icd["kinds"]["thing"]["ops"];
        let keys = |v: &serde_json::Value| v.as_object().map(|o| o.keys().cloned().collect::<std::collections::BTreeSet<_>>()).unwrap_or_default();
        let set = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<std::collections::BTreeSet<_>>();
        assert_eq!(keys(&ops["thing.setProfile"]["args"]["condition"]["vocabulary"]), set(&CONDITIONS));
        assert_eq!(keys(&ops["thing.setDisposition"]["args"]["state"]["vocabulary"]), set(&DISPOSITIONS));
        assert_eq!(ops["thing.setProfile"]["args"]["description"]["maxLength"].as_u64(), Some(MAX_DESCRIPTION as u64));
        assert_eq!(ops["thing.addPhoto"]["args"]["photo"]["maxLength"].as_u64(), Some(MAX_ADDED_PHOTO_B64 as u64));
        assert_eq!(ops["thing.addPhoto"]["maxLive"].as_u64(), Some(MAX_PHOTOS as u64));
    }

    fn w98_folded(ops: Vec<(u32, Args)>) -> ThingState {
        let mut c: Coordinator<ThingType> = Coordinator::new(vec![owner()], owner());
        let mut prev = GENESIS_PREV;
        for (i, (op_id, a)) in ops.into_iter().enumerate() {
            let d = sequenced_delta(ObjectKind::Thing.type_id() as u32, op_id, a, 0, i as u64, prev);
            prev = d.id();
            c.deliver(d, owner()).expect("owner-sequenced delta delivered");
        }
        c.state()
    }
    fn photo(id: &str, at: i64) -> (u32, Args) {
        (OP_ADD_PHOTO, args(&[("id", ArgVal::Text(id.into())), ("photo", ArgVal::Text("/9j/4AAQ".into())), ("photoMime", ArgVal::Text("image/jpeg".into())), ("photoVia", ArgVal::Text("inline".into())), ("photoKind", ArgVal::Text("still".into())), ("at", ArgVal::Int(at))]))
    }

    /// N1, N2: a listing states its condition, from the vocabulary, and a long description.
    #[test]
    fn w98_a_listing_states_its_condition_and_description() {
        let st = w98_folded(vec![(OP_SET_PROFILE, args(&[("name", ArgVal::Text("drill".into())), ("condition", ArgVal::Text("very_good".into())), ("description", ArgVal::Text("Two batteries.\nCharger included.".into()))]))]);
        assert_eq!(st.condition.as_deref(), Some("very_good"));
        assert!(st.description.contains("Charger"));
        let st = w98_folded(vec![(OP_SET_PROFILE, args(&[("name", ArgVal::Text("drill".into())), ("condition", ArgVal::Text("mint".into()))]))]);
        assert_eq!(st.name, "", "a condition outside the vocabulary is refused whole");
    }

    /// N3: photos are a set by id, at most MAX_PHOTOS live; a removed id stays removed.
    #[test]
    fn w98_photos_are_a_set_and_a_removed_one_stays_removed() {
        let mut ops: Vec<(u32, Args)> = (0..MAX_PHOTOS + 1).map(|n| photo(&format!("{n:016x}"), n as i64)).collect();
        ops.push((OP_REMOVE_PHOTO, args(&[("id", ArgVal::Text(format!("{:016x}", 0)))])));
        ops.push(photo(&format!("{:016x}", 0), 99));
        let st = w98_folded(ops);
        assert_eq!(st.photos.len(), MAX_PHOTOS - 1, "the {}th refused, then one removed, and not added again", MAX_PHOTOS + 1);
        assert!(!st.photos.contains_key(&format!("{:016x}", 0)));
    }

    /// N3: a photo is inline within its cap, or detached with its digest, size and both keys
    /// (O-79); an empty photo clears its id, which may be added again.
    #[test]
    fn w98_a_photo_is_inline_or_detached_and_an_empty_one_clears() {
        let id = "00000000000000aa".to_string();
        let detached = (OP_ADD_PHOTO, args(&[("id", ArgVal::Text(id.clone())), ("photo", ArgVal::Text(String::new())), ("photoMime", ArgVal::Text("image/jpeg".into())), ("photoVia", ArgVal::Text("detached".into())), ("photoDigest", ArgVal::Text("ab".repeat(32))), ("photoBytes", ArgVal::Int(90_000)), ("photoKey", ArgVal::Text("cd".repeat(32))), ("photoSecret", ArgVal::Text("c2VjcmV0".into())), ("at", ArgVal::Int(1))]));
        let st = w98_folded(vec![detached.clone()]);
        assert_eq!(st.photos[&id].digest, "ab".repeat(32));
        assert_eq!(st.photos[&id].bytes, Some(90_000));
        let mut unkeyed = detached.clone();
        unkeyed.1.remove("photoSecret");
        assert!(w98_folded(vec![unkeyed]).photos.is_empty(), "a detached photo carries both keys");
        let clear = (OP_ADD_PHOTO, args(&[("id", ArgVal::Text(id.clone())), ("photo", ArgVal::Text(String::new())), ("photoMime", ArgVal::Text(String::new())), ("at", ArgVal::Int(2))]));
        let st = w98_folded(vec![photo(&id, 1), clear, photo(&id, 3)]);
        assert_eq!(st.photos[&id].at, 3, "cleared, then added again");
        let mut big = photo(&id, 1);
        big.1.insert("photo".into(), ArgVal::Text("A".repeat(MAX_ADDED_PHOTO_B64 + 4)));
        assert!(w98_folded(vec![big]).photos.is_empty(), "over the ICD's maxLength");
    }

    /// N5: reserved for one member until a time, which then reads available; sold names its
    /// Transaction; a hold with no member is refused.
    #[test]
    fn w98_a_disposition_holds_for_one_member_until_its_time() {
        let who = "ab".repeat(32);
        let st = w98_folded(vec![(OP_SET_DISPOSITION, args(&[("state", ArgVal::Text("reserved".into())), ("for", ArgVal::Text(who.clone())), ("until", ArgVal::Int(1000)), ("at", ArgVal::Int(1))]))]);
        let read = |now: i64| {
            let mut v = serde_json::json!({ "disposition": st.disposition });
            at_read(&mut v, now);
            v["disposition"].clone()
        };
        assert_eq!(read(999)["state"], "reserved");
        assert_eq!(read(1001)["state"], "available");
        assert_eq!(read(999)["for"], who);
        let st = w98_folded(vec![(OP_SET_DISPOSITION, args(&[("state", ArgVal::Text("reserved".into())), ("at", ArgVal::Int(1))]))]);
        assert!(st.disposition.is_none());
        let st = w98_folded(vec![(OP_SET_DISPOSITION, args(&[("state", ArgVal::Text("sold".into())), ("transaction", ArgVal::Text("cd".repeat(32))), ("at", ArgVal::Int(2))]))]);
        assert_eq!(st.disposition.as_ref().and_then(|d| d.transaction.clone()), Some("cd".repeat(32)));
    }

    fn owner() -> MemberId {
        [7u8; 32]
    }

    fn args(pairs: &[(&str, ArgVal)]) -> Args {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    /// Fold a run of owner-sequenced deltas and return the compiled state.
    fn folded(ops: Vec<(u32, Args)>) -> ThingState {
        let mut c: Coordinator<ThingType> = Coordinator::new(vec![owner()], owner());
        let mut prev = GENESIS_PREV;
        for (i, (op_id, a)) in ops.into_iter().enumerate() {
            let d = sequenced_delta(
                ObjectKind::Thing.type_id() as u32,
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
        c.state()
    }

    /// The whole hammer journey in one fold: mint the thing, declare wanting it.
    #[test]
    fn a_thing_carries_a_market_posture() {
        let st = folded(vec![
            (
                OP_SET_PROFILE,
                args(&[
                    ("name", ArgVal::Text("hammer".into())),
                    ("descriptor", ArgVal::Text("cracked handle".into())),
                ]),
            ),
            (
                OP_SET_POSTURE,
                args(&[("posture", ArgVal::Text("wants".into()))]),
            ),
        ]);
        assert_eq!(st.name, "hammer");
        assert_eq!(st.descriptor, "cracked handle");
        assert_eq!(st.posture, Some(Posture::Wants));
        assert_eq!(st.price, None, "a standing Want has no price");
    }

    /// The category axis: absent = artifact (every pre-category log folds identically),
    /// skill/job fold, and garbage is a malformed delta — refused whole, name untouched.
    #[test]
    fn a_category_folds_defaults_and_rejects_garbage() {
        let st = folded(vec![(
            OP_SET_PROFILE,
            args(&[("name", ArgVal::Text("hammer".into()))]),
        )]);
        assert_eq!(st.category, Category::Artifact, "absent = artifact");

        let st = folded(vec![(
            OP_SET_PROFILE,
            args(&[
                ("name", ArgVal::Text("bike repair".into())),
                ("category", ArgVal::Text("skill".into())),
            ]),
        )]);
        assert_eq!(st.category, Category::Skill);

        let st = folded(vec![(
            OP_SET_PROFILE,
            args(&[
                ("name", ArgVal::Text("bar shift, Sat".into())),
                ("category", ArgVal::Text("job".into())),
            ]),
        )]);
        assert_eq!(st.category, Category::Job);

        let mut st = ThingState::default();
        let ctx = crate::object::ReduceContext {
            members: &[owner()],
            owner: owner(),
            epoch: 0,
        };
        let bad = args(&[
            ("name", ArgVal::Text("x".into())),
            ("category", ArgVal::Text("vibe".into())),
        ]);
        let op = Op {
            op_id: OP_SET_PROFILE,
            args: &bad,
            author: &owner(),
            pos: None,
            ctx: &ctx,
        };
        assert_eq!(
            ThingType::reduce(&mut st, &op),
            Err(DeltaRejection::MalformedArgs)
        );
        assert!(st.name.is_empty(), "a rejected profile must not half-apply");
    }

    /// A posture moves; the thing does not become a new object.
    #[test]
    fn a_posture_moves_and_clears() {
        let st = folded(vec![
            (
                OP_SET_POSTURE,
                args(&[("posture", ArgVal::Text("wants".into()))]),
            ),
            (
                OP_SET_POSTURE,
                args(&[
                    ("posture", ArgVal::Text("buying".into())),
                    ("price", ArgVal::Text("£15".into())),
                ]),
            ),
        ]);
        assert_eq!(st.posture, Some(Posture::Buying));
        assert_eq!(st.price.as_deref(), Some("£15"));

        let cleared = folded(vec![
            (
                OP_SET_POSTURE,
                args(&[
                    ("posture", ArgVal::Text("selling".into())),
                    ("price", ArgVal::Text("£15".into())),
                ]),
            ),
            (OP_CLEAR_POSTURE, args(&[])),
        ]);
        assert_eq!(cleared.posture, None);
        assert_eq!(
            cleared.price, None,
            "a price without a posture is meaningless"
        );
    }

    /// Loud, not lenient: an unknown posture and a price on a standing intent are both
    /// rejected rather than coerced into something plausible.
    #[test]
    fn malformed_postures_are_rejected_not_coerced() {
        let mut st = ThingState::default();
        let ctx = crate::object::ReduceContext {
            members: &[owner()],
            owner: owner(),
            epoch: 0,
        };
        let bad_name = args(&[("posture", ArgVal::Text("hoarding".into()))]);
        let op = Op {
            op_id: OP_SET_POSTURE,
            args: &bad_name,
            author: &owner(),
            pos: None,
            ctx: &ctx,
        };
        assert_eq!(
            ThingType::reduce(&mut st, &op),
            Err(DeltaRejection::MalformedArgs)
        );

        let priced_want = args(&[
            ("posture", ArgVal::Text("wants".into())),
            ("price", ArgVal::Text("£15".into())),
        ]);
        let op = Op {
            op_id: OP_SET_POSTURE,
            args: &priced_want,
            author: &owner(),
            pos: None,
            ctx: &ctx,
        };
        assert_eq!(
            ThingType::reduce(&mut st, &op),
            Err(DeltaRejection::PreconditionFailed)
        );
        assert_eq!(st.posture, None, "a rejected delta must not mutate state");
    }

    /// Truncation must be IDEMPOTENT — the same cell every time for the same position.
    /// This is the whole reason the design truncates rather than jitters: noise added per
    /// listing averages out across a seller's history and the network triangulates them.
    #[test]
    fn area_truncation_is_stable_and_nests() {
        let (lat, lon) = (51.4545, -2.5879); // Bristol
        let a = geohash(lat, lon, 5);
        for _ in 0..10 {
            assert_eq!(
                geohash(lat, lon, 5),
                a,
                "the same position must give the same cell"
            );
        }
        // Coarser is a PREFIX of finer — what makes "shared prefix = proximity" work, and
        // what lets a CLA compare two areas with a string compare instead of trigonometry.
        assert!(a.starts_with(&geohash(lat, lon, 4)));
        assert!(a.starts_with(&geohash(lat, lon, 3)));

        // Somewhere far away shares no prefix.
        let sydney = geohash(-33.8688, 151.2093, 5);
        assert_ne!(sydney.chars().next(), a.chars().next());
    }

    /// The floor exists so a listing cannot publish a street. Refuse, do not clamp silently.
    #[test]
    fn a_street_level_area_is_refused() {
        let mut st = ThingState::default();
        let ctx = crate::object::ReduceContext {
            members: &[owner()],
            owner: owner(),
            epoch: 0,
        };

        let too_fine = args(&[
            ("posture", ArgVal::Text("selling".into())),
            ("area", ArgVal::Text("gcnb1x9".into())), // 7 chars — a doorstep
        ]);
        let op = Op {
            op_id: OP_SET_POSTURE,
            args: &too_fine,
            author: &owner(),
            pos: None,
            ctx: &ctx,
        };
        assert_eq!(
            ThingType::reduce(&mut st, &op),
            Err(DeltaRejection::MalformedArgs)
        );

        // And a charset a geohash cannot contain (a, i, l, o all misread).
        let bad_charset = args(&[
            ("posture", ArgVal::Text("selling".into())),
            ("area", ArgVal::Text("gail".into())),
        ]);
        let op = Op {
            op_id: OP_SET_POSTURE,
            args: &bad_charset,
            author: &owner(),
            pos: None,
            ctx: &ctx,
        };
        assert_eq!(
            ThingType::reduce(&mut st, &op),
            Err(DeltaRejection::MalformedArgs)
        );
        assert_eq!(st.posture, None, "a rejected delta must not half-apply");
    }

    /// Silence means PRIVATE. A caller that forgets the field under-shares.
    #[test]
    fn reach_defaults_closed_and_clears_with_the_posture() {
        let listed = folded(vec![(
            OP_SET_POSTURE,
            args(&[
                ("posture", ArgVal::Text("selling".into())),
                ("price", ArgVal::Text("£15".into())),
                ("reach", ArgVal::Text("network".into())),
                ("area", ArgVal::Text("gcnb1".into())),
            ]),
        )]);
        assert_eq!(listed.reach, Reach::Network);
        assert_eq!(listed.area.as_deref(), Some("gcnb1"));

        let silent = folded(vec![(
            OP_SET_POSTURE,
            args(&[("posture", ArgVal::Text("wants".into()))]),
        )]);
        assert_eq!(
            silent.reach,
            Reach::Private,
            "absent reach must not mean network"
        );
        assert_eq!(silent.area, None);

        // Taking it off the market takes its audience with it — otherwise a later posture
        // would silently inherit an audience the owner never restated.
        let cleared = folded(vec![
            (
                OP_SET_POSTURE,
                args(&[
                    ("posture", ArgVal::Text("selling".into())),
                    ("reach", ArgVal::Text("network".into())),
                    ("area", ArgVal::Text("gcnb1".into())),
                ]),
            ),
            (OP_CLEAR_POSTURE, args(&[])),
        ]);
        assert_eq!(cleared.reach, Reach::Private);
        assert_eq!(cleared.area, None);
    }

    /// The listing's face: set, replace, clear — with an oversize photo refused
    /// whole and a picture-without-a-type (or type-without-a-picture) rejected.
    #[test]
    fn photo_sets_replaces_clears_and_refuses_oversize() {
        let base = vec![(
            OP_SET_PROFILE,
            args(&[("name", ArgVal::Text("Hammer".into()))]),
        )];

        let mut set = base.clone();
        set.push((
            OP_SET_PHOTO,
            args(&[
                ("photo", ArgVal::Text("QkFTRTY0".into())),
                ("mime", ArgVal::Text("image/jpeg".into())),
            ]),
        ));
        let st = folded(set.clone());
        assert_eq!(
            (st.photo.as_str(), st.photo_mime.as_str()),
            ("QkFTRTY0", "image/jpeg")
        );

        // Clear: empty photo + empty mime takes the picture down.
        let mut cleared = set.clone();
        cleared.push((OP_SET_PHOTO, args(&[])));
        let st = folded(cleared);
        assert_eq!((st.photo.as_str(), st.photo_mime.as_str()), ("", ""));

        // Oversize is REFUSED whole — the held photo survives, nothing half-lands.
        let mut oversize = set.clone();
        oversize.push((
            OP_SET_PHOTO,
            args(&[
                (
                    "photo",
                    ArgVal::Text("x".repeat(crate::event::MAX_PHOTO_B64 + 1)),
                ),
                ("mime", ArgVal::Text("image/jpeg".into())),
            ]),
        ));
        let st = folded(oversize);
        assert_eq!(st.photo, "QkFTRTY0", "oversize dropped whole");

        // A picture and its type must agree on emptiness.
        let mut mismatch = set;
        mismatch.push((
            OP_SET_PHOTO,
            args(&[("photo", ArgVal::Text("QkFTRTY0".into()))]),
        ));
        let st = folded(mismatch);
        assert_eq!(st.photo_mime, "image/jpeg", "typeless photo dropped whole");
    }

    #[test]
    fn every_op_satisfies_the_spec_invariant() {
        for d in ThingType::ops() {
            assert!(
                d.is_well_formed(),
                "op {} violates the spec invariant",
                d.name
            );
        }
        assert_eq!(ObjectKind::Thing.type_id(), 27);
        assert_eq!(ObjectKind::from_type_id(27), Some(ObjectKind::Thing));
    }
}
