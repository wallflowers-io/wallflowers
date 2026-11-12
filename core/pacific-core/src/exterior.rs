//! exterior — WHAT A NON-MEMBER HOLDS OF AN OBJECT (#76, ICD 2.6.0 `facets.exterior`).
//!
//! Joining is a follow request, not a dip in (Ralph, 9 Nov): you ask, the owner is notified,
//! admission may wait or be declined, and once admitted you are in the MLS group. So the
//! exterior is what stands outside: a name, a picture, a description, and a door.
//!
//! - `base.setExterior` — the owner's own words for the outside, replaced wholesale, with the
//!   door's public key. Owner-sequenced so a carried copy is checked against the owner key alone
//!   (`contact.publishExterior`).
//! - `base.setDoor` — the door's X25519 private key and its mode. In the log so every member's
//!   device can open a request (`mls.door`); never projected, never carried.
//! - `base.decideRequest` — the answer to one request at a `review` door, so every admitter's
//!   device stops showing it. First decision per request holds.
//!
//! The facet pattern, as [`crate::profiles`]: a kind holds an [`ExteriorState`], splices
//! [`SET_EXTERIOR`], [`SET_DOOR`] and [`DECIDE_REQUEST`] into its `ops()`, and delegates to
//! [`reduce_exterior`]. Band `0xF00E`; every number and word here is the ICD's (build.rs).

use std::collections::BTreeMap;

use crate::coordinator::{ArgVal, Args};
use crate::object::{Authority, Commutativity, DeltaRejection, MemberId, Op, OpDecl};
use crate::object_args::{arg_hex32, opt_text, req_gen, req_text};

pub(crate) mod icd {
    include!(concat!(env!("OUT_DIR"), "/icd_exterior.rs"));
}
pub use icd::{OP_DECIDE_REQUEST, OP_SET_DOOR, OP_SET_EXTERIOR};
/// `mls.door.relay`, read from the ICD once, by pacific-wire, which the relay reads too.
pub use pacific_wire::{DOOR_CAP, DOOR_MAX_BYTES, DOOR_PREFIX, DOOR_TTL_DAYS};


pub const SET_EXTERIOR: OpDecl = OpDecl {
    op_id: OP_SET_EXTERIOR,
    name: "base.setExterior",
    authority: Authority::Owner,
    commutativity: Commutativity::Sequenced,
};
pub const SET_DOOR: OpDecl = OpDecl {
    op_id: OP_SET_DOOR,
    name: "base.setDoor",
    authority: Authority::Owner,
    commutativity: Commutativity::Sequenced,
};
pub const DECIDE_REQUEST: OpDecl = OpDecl {
    op_id: OP_DECIDE_REQUEST,
    name: "base.decideRequest",
    authority: Authority::OwnerOrRole(crate::group::GroupRole::Admitter),
    commutativity: Commutativity::Commutative,
};

/// The three, for a drift pin; a kind's table names each one itself (const initialiser).
pub const EXTERIOR_OPS: [OpDecl; 3] = [SET_EXTERIOR, SET_DOOR, DECIDE_REQUEST];

pub fn is_exterior_op(op_id: u32) -> bool {
    matches!(op_id, OP_SET_EXTERIOR | OP_SET_DOOR | OP_DECIDE_REQUEST)
}

/// The outside, as its owner wrote it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Exterior {
    pub name: String,
    pub description: String,
    pub photo: Option<pacific_media::MediaRef>,
    /// The door's public key; `None`, no way to ask.
    pub door: Option<[u8; 32]>,
    /// The owner's settings for the outside, and where the hosted document is.
    #[serde(default)]
    pub outside: Outside,
}

/// What `base.setExterior` says beyond the words: how much an Event's Place shows, whether a
/// Post reads outside, and where the hosted exterior document is (`media.hosted`).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Outside {
    /// A geohash length, `PRECISION_MIN..=PRECISION_MAX`; 0, the venue's name alone.
    pub precision: u8,
    pub readable_outside: bool,
    pub hosted: Option<Hosted>,
}

/// The hosted exterior document's key and content key, as the pointer carries them.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Hosted {
    /// 64 lowercase hex: an address (`pacific_wire::address`).
    pub key: String,
    pub secret: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DoorMode {
    Review,
    Auto,
}

impl DoorMode {
    pub fn parse(s: &str) -> Option<Self> {
        icd::DOOR_MODES.contains(&s).then(|| if s == "auto" { Self::Auto } else { Self::Review })
    }
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Review => "review",
            Self::Auto => "auto",
        }
    }
}

/// The door, as the roster holds it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Door {
    pub key: [u8; 32],
    pub mode: DoorMode,
}

/// One request answered.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Decision {
    pub asker: MemberId,
    pub admit: bool,
    pub by: MemberId,
    pub gen: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExteriorState {
    pub exterior: Option<Exterior>,
    pub door: Option<Door>,
    /// By request id.
    pub decided: BTreeMap<[u8; 32], Decision>,
}

/// `base.setExterior`'s args, checked: what a reducer folds and a receiver of a carried copy
/// re-checks, so both refuse the same rows.
pub fn parse(args: &Args) -> Result<Exterior, DeltaRejection> {
    let bad = DeltaRejection::MalformedArgs;
    let name = req_text(args, "name")?;
    if name.trim().is_empty() || name.len() > icd::NAME_MAX {
        return Err(bad);
    }
    let description = opt_text(args, "description").unwrap_or_default();
    if description.len() > icd::DESCRIPTION_MAX {
        return Err(bad);
    }
    let photo = match crate::arg_reads::get(args, "photo") {
        None => None,
        Some(_) => {
            let m = crate::event::media_of(args, "photo", pacific_media::Slot::Banner, icd::PHOTO_MAX)?;
            (!m.is_empty()).then_some(m)
        }
    };
    let door = match crate::arg_reads::get(args, "door") {
        None => None,
        Some(_) => Some(arg_hex32(args, "door")?),
    };
    let precision = match crate::object_args::opt_int(args, "precision") {
        None => 0,
        Some(p) if (icd::PRECISION_MIN as i64..=icd::PRECISION_MAX as i64).contains(&p) => p as u8,
        Some(_) => return Err(bad),
    };
    let readable_outside = match crate::object_args::opt_int(args, "readableOutside") {
        None | Some(0) => false,
        Some(1) => true,
        Some(_) => return Err(bad),
    };
    let hosted = match (crate::arg_reads::get(args, "hostedKey"), opt_text(args, "hostedSecret")) {
        (None, None) => None,
        (Some(_), Some(secret)) => {
            use base64::Engine as _;
            let key = hex::encode(arg_hex32(args, "hostedKey")?);
            let secret: [u8; 32] = base64::engine::general_purpose::STANDARD
                .decode(secret)
                .ok()
                .and_then(|v| v.try_into().ok())
                .ok_or(bad)?;
            Some(Hosted { key, secret })
        }
        _ => return Err(bad),
    };
    Ok(Exterior { name: name.to_string(), description, photo, door, outside: Outside { precision, readable_outside, hosted } })
}

/// Fold one exterior op. `admitter` is whether the author holds admitter in the kind's own
/// roles (a kind with no roles facet passes false: its owner is its only admitter).
pub fn reduce_exterior(state: &mut ExteriorState, op: &Op<'_>, admitter: bool) -> Result<(), DeltaRejection> {
    match op.op_id {
        OP_SET_EXTERIOR => {
            state.exterior = Some(parse(op.args)?);
            Ok(())
        }
        OP_SET_DOOR => {
            let key = req_text(op.args, "key")?;
            let mode = DoorMode::parse(req_text(op.args, "mode")?).ok_or(DeltaRejection::MalformedArgs)?;
            state.door = if key.is_empty() { None } else { Some(Door { key: arg_hex32(op.args, "key")?, mode }) };
            Ok(())
        }
        OP_DECIDE_REQUEST => {
            // Checked at fold as at write: an admitter's standing is the kind's folded roles.
            if *op.author != op.ctx.owner && !admitter {
                return Err(DeltaRejection::Unauthorized);
            }
            let request = arg_hex32(op.args, "request")?;
            let asker = arg_hex32(op.args, "asker")?;
            let decision = req_text(op.args, "decision")?;
            if !icd::DECISIONS.contains(&decision) {
                return Err(DeltaRejection::MalformedArgs);
            }
            let gen = req_gen(op.args)?;
            state.decided.entry(request).or_insert(Decision { asker, admit: decision == "admit", by: *op.author, gen });
            Ok(())
        }
        _ => Err(DeltaRejection::UnknownType),
    }
}

/// The kind named `name`, if it has an exterior (`facets.exterior.on`).
pub fn exterior_kind(name: &str) -> Option<crate::object::ObjectKind> {
    icd::EXTERIOR_KINDS.contains(&name).then(|| crate::object::ObjectKind::ALL.iter().copied().find(|k| k.name() == name)).flatten()
}

/// One owner-signed row of `object`, as `contact.publishExterior` carries it: checked against
/// the owner key and the object's group id alone, which is all a non-member holds. The row's
/// typeId must be `kind`'s and its op `op_id`; a sequenced row's place in the log comes back
/// with its args.
pub fn signed_row(group: &[u8], kind: crate::object::ObjectKind, owner: &MemberId, op_id: u32, payload: &[u8]) -> Result<(crate::object::LogPosition, Args), DeltaRejection> {
    let bad = DeltaRejection::MalformedArgs;
    let (envelope, author, sig) = crate::delta_sig::open_payload(payload).map_err(|_| bad.clone())?;
    if author != *owner {
        return Err(DeltaRejection::Unauthorized);
    }
    let delta = crate::coordinator::decode_delta(&envelope).map_err(|_| bad.clone())?;
    if delta.type_id != kind.type_id() as u32 || delta.op_id != op_id {
        return Err(bad);
    }
    let seq = delta.seq.ok_or(bad.clone())?;
    crate::delta_sig::verify_delta(group, &author, &delta.id(), &sig).map_err(|_| DeltaRejection::Unauthorized)?;
    Ok((crate::object::LogPosition { epoch: delta.epoch, seq }, delta.args))
}

/// An object's exterior as a non-member holds it, off `contact.publishExterior`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Carried {
    pub object: String,
    pub kind: crate::object::ObjectKind,
    pub owner: MemberId,
    pub exterior: Exterior,
    /// Where the exterior row sits in the object's log: the newest wins.
    pub pos: crate::object::LogPosition,
    pub visibility: crate::visibility::Visibility,
    /// Where the visibility row sits; `None`, the kind's default.
    pub visibility_pos: Option<crate::object::LogPosition>,
    /// The holder's distance from the roster: 1 from a member.
    pub hops: u32,
    /// Who carried it here.
    pub via: MemberId,
    /// The rows, verbatim, so a holder can carry them on.
    pub exterior_row: Vec<u8>,
    pub visibility_row: Option<Vec<u8>>,
    /// Withdrawn by a narrower visibility: held as a tombstone, so the withdrawal is carried
    /// on down every path the copy took, and an older copy cannot come back.
    pub withdrawn: bool,
}

/// `contact.publishExterior`'s args, checked: the rows signed by the owner, the copy within its
/// reach. What the Contact fold folds.
pub fn carried(args: &Args, via: &MemberId) -> Result<Carried, DeltaRejection> {
    use crate::object_args::{arg_b64, req_hex_id};
    let bad = DeltaRejection::MalformedArgs;
    let object = req_hex_id(args, "object")?;
    let group = hex::decode(&object).map_err(|_| bad.clone())?;
    let kind = exterior_kind(req_text(args, "kind")?).ok_or(bad.clone())?;
    let owner = arg_hex32(args, "owner")?;
    let exterior_row = arg_b64(args, "exterior")?;
    let visibility_row = match crate::arg_reads::get(args, "visibility") {
        None => None,
        Some(_) => Some(arg_b64(args, "visibility")?),
    };
    let hops = u32::try_from(crate::object_args::req_int(args, "hops")?).map_err(|_| bad.clone())?;
    req_gen(args)?;
    let (pos, ext_args) = signed_row(&group, kind, &owner, OP_SET_EXTERIOR, &exterior_row)?;
    let exterior = parse(&ext_args)?;
    let (visibility, visibility_pos) = match &visibility_row {
        None => (crate::visibility::default_for(kind.name()), None),
        Some(row) => {
            let (at, a) = signed_row(&group, kind, &owner, crate::visibility::OP_SET_VISIBILITY, row)?;
            (crate::visibility::Visibility::parse(req_text(&a, "visibility")?)?, Some(at))
        }
    };
    if hops == 0 {
        return Err(bad);
    }
    Ok(Carried { object, kind, owner, exterior, pos, visibility, visibility_pos, hops, via: *via, exterior_row, visibility_row, withdrawn: false })
}

impl Carried {
    /// THE REACH IS THE OWNER'S: one hop from a member at `connections`, two at `network`, none
    /// at `private` (`Visibility::reach_hops`).
    pub fn within_reach(&self) -> bool {
        self.hops <= self.visibility.reach_hops()
    }

    /// Newer rows win; the same rows, the shorter path.
    pub fn supersedes(&self, held: &Carried) -> bool {
        let (new, old) = ((self.pos, self.visibility_pos), (held.pos, held.visibility_pos));
        new > old || (new == old && self.hops < held.hops)
    }
}

/// Fold one carried copy into the copies held, by object. A copy past its reach is refused, and
/// when its visibility row is newer than the one held it WITHDRAWS the held copy, kept as a
/// tombstone: narrowing the dial chases the exterior down every path it took, as a withdrawn
/// listing does.
pub fn fold_carried(held: &mut BTreeMap<String, Carried>, mut c: Carried) -> Result<(), DeltaRejection> {
    let prior = held.get(&c.object);
    if !c.within_reach() {
        if prior.is_some_and(|h| c.visibility_pos > h.visibility_pos) {
            c.withdrawn = true;
            held.insert(c.object.clone(), c);
            return Ok(());
        }
        return Err(DeltaRejection::PreconditionFailed);
    }
    if prior.is_none_or(|h| c.supersedes(h)) {
        held.insert(c.object.clone(), c);
    }
    Ok(())
}

/// `contact.publishExterior`'s args for a held copy, carried on at `hops`; `authoring::build`
/// adds the gen.
pub fn publish_exterior_args(object: &str, kind: &str, owner: &MemberId, exterior_row: &[u8], visibility_row: Option<&[u8]>, hops: u32) -> Args {
    use base64::Engine as _;
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    let mut a = Args::new();
    a.insert("object".into(), ArgVal::Text(object.into()));
    a.insert("kind".into(), ArgVal::Text(kind.into()));
    a.insert("owner".into(), ArgVal::Text(hex::encode(owner)));
    a.insert("exterior".into(), ArgVal::Text(b64(exterior_row)));
    if let Some(v) = visibility_row {
        a.insert("visibility".into(), ArgVal::Text(b64(v)));
    }
    a.insert("hops".into(), ArgVal::Int(hops as i64));
    a
}

/// `base.setExterior`'s args, with no settings and no hosted document.
pub fn set_exterior_args(name: &str, description: &str, photo: Option<&pacific_media::MediaRef>, door: Option<&[u8; 32]>) -> Args {
    set_exterior_args_with(name, description, photo, door, &Outside::default())
}

/// `base.setExterior`'s args, whole.
pub fn set_exterior_args_with(name: &str, description: &str, photo: Option<&pacific_media::MediaRef>, door: Option<&[u8; 32]>, outside: &Outside) -> Args {
    use base64::Engine as _;
    let mut a = Args::new();
    if outside.precision > 0 {
        a.insert("precision".into(), ArgVal::Int(outside.precision as i64));
    }
    if outside.readable_outside {
        a.insert("readableOutside".into(), ArgVal::Int(1));
    }
    if let Some(h) = &outside.hosted {
        a.insert("hostedKey".into(), ArgVal::Text(h.key.clone()));
        a.insert("hostedSecret".into(), ArgVal::Text(base64::engine::general_purpose::STANDARD.encode(h.secret)));
    }
    a.insert("name".into(), ArgVal::Text(name.into()));
    if !description.is_empty() {
        a.insert("description".into(), ArgVal::Text(description.into()));
    }
    if let Some(p) = photo {
        p.to_args("photo", &mut a);
    }
    if let Some(d) = door {
        a.insert("door".into(), ArgVal::Text(hex::encode(d)));
    }
    a
}

// ── THE FIELDS: what the hosted document holds (`facets.exterior.fields`) ─────────────────

/// What `fields_of` reads beside the fold: who is on the roster, the names the owner's device
/// knows, and the venue Place's point when the Event has none of its own.
pub struct FieldsCtx<'a> {
    pub owner: MemberId,
    /// The roster's identities, deduplicated or not.
    pub roster: &'a [MemberId],
    pub name_of: &'a dyn Fn(&MemberId) -> String,
    pub venue_point: Option<crate::geo::GeoPoint>,
}

/// A kind's folded state, for `fields_of`.
pub enum KindState<'a> {
    Event(&'a crate::event::EventState),
    Post(&'a crate::post::PostState),
    /// Every other kind: the exterior's own words alone.
    Other,
}

/// Who is going (D9, BLUE_INTERIOR 12 Nov): distinct roster identities, less the owner and every
/// admin and admitter in the Event's own roles. The one count; nothing else computes it.
pub fn going_of(st: &crate::event::EventState, owner: &MemberId, roster: &[MemberId]) -> u32 {
    use crate::group::GroupRole;
    let hosting = |m: &MemberId| m == owner || matches!(st.member_roles.get(m), Some(GroupRole::Admin | GroupRole::Admitter));
    roster.iter().filter(|m| !hosting(m)).collect::<std::collections::BTreeSet<_>>().len() as u32
}

/// A MediaRef as its op flattens it, under `prefix`: the ICD's form, read back by
/// `MediaRef::from_args_by`.
fn media_json(prefix: &str, m: &pacific_media::MediaRef) -> serde_json::Value {
    let mut a = Args::new();
    m.to_args(prefix, &mut a);
    serde_json::Value::Object(
        a.into_iter()
            .map(|(k, v)| (k, match v { ArgVal::Int(i) => serde_json::json!(i), ArgVal::Text(t) => serde_json::json!(t) }))
            .collect(),
    )
}

/// An inline still or clip from an older media facet, as a reference.
fn inline_ref(mime: &str, data: &str) -> Option<pacific_media::MediaRef> {
    (!data.is_empty()).then(|| pacific_media::MediaRef::inline(mime, data).ok()).flatten()
}

/// THE ONE BUILDER of the outside: everything a non-member reads of an object, from its fold.
/// Its keys are the kind's `facets.exterior.fields` and nothing else (held by a test), so the
/// interior's other state cannot leak by a field added here and not to the ICD.
pub fn fields_of(ext: &Exterior, state: KindState<'_>, ctx: &FieldsCtx<'_>) -> serde_json::Map<String, serde_json::Value> {
    use serde_json::json;
    let mut f = serde_json::Map::new();
    f.insert("name".into(), json!(ext.name));
    if !ext.description.is_empty() {
        f.insert("description".into(), json!(ext.description));
    }
    if let Some(p) = &ext.photo {
        f.insert("photo".into(), media_json("photo", p));
    }
    let text = |f: &mut serde_json::Map<String, serde_json::Value>, k: &str, v: &str| {
        if !v.is_empty() {
            f.insert(k.into(), json!(v));
        }
    };
    match state {
        KindState::Event(st) => {
            text(&mut f, "title", &st.title);
            if st.start_ms > 0 {
                f.insert("startMs".into(), json!(st.start_ms));
            }
            if let Some(e) = st.end_ms {
                f.insert("endMs".into(), json!(e));
            }
            for (k, v) in [("tz", &st.tz), ("recurrence", &st.recurrence), ("status", &st.status), ("online", &st.online),
                ("videoUrl", &st.video_url), ("descriptor", &st.descriptor), ("descriptorFormat", &st.descriptor_format),
                ("ticketUrl", &st.ticket_url)]
            {
                text(&mut f, k, v);
            }
            if st.all_day {
                f.insert("allDay".into(), json!(1));
            }
            let venue = st.venue_ref.as_ref().map(|v| v.name.as_str()).filter(|n| !n.is_empty()).unwrap_or(&st.venue);
            text(&mut f, "venue", venue);
            let point = st.point().cloned().or_else(|| ctx.venue_point.clone());
            if let (Some(p), n) = (point, ext.outside.precision) {
                if n > 0 {
                    f.insert("point".into(), json!(crate::geo::geohash(p.lat(), p.lng(), n as usize)));
                }
            }
            let hosts: Vec<_> = std::iter::once(ctx.owner)
                .chain(st.member_roles.iter().filter(|(_, r)| matches!(r, crate::group::GroupRole::Admin)).map(|(m, _)| *m).filter(|m| *m != ctx.owner))
                .map(|m| json!({ "key": hex::encode(m), "name": (ctx.name_of)(&m) }))
                .collect();
            f.insert("hosts".into(), json!(hosts));
            if !st.lineup.is_empty() {
                f.insert("lineup".into(), json!(st.lineup));
            }
            if let Some(acts) = &st.acts {
                let acts: Vec<_> = acts.iter().map(|a| json!({ "performer": a.performer.id(), "role": a.role, "start": a.start, "end": a.end })).collect();
                f.insert("acts".into(), json!(acts));
            }
            let banner = st.banner.clone().filter(|m| !m.is_empty()).or_else(|| inline_ref(&st.media.banner_mime, &st.media.banner));
            if let Some(b) = banner {
                f.insert("banner".into(), media_json("banner", &b));
            }
            let clip = st.clip.clone().filter(|m| !m.is_empty()).or_else(|| inline_ref(&st.media.clip_mime, &st.media.clip));
            if let Some(c) = clip {
                f.insert("clip".into(), media_json("clip", &c));
            }
            let photos: Vec<_> = match &st.photo_set {
                Some(set) => set.values().map(|p| media_json("photo", &p.media)).collect(),
                None => st.media.photos.iter().filter_map(|p| inline_ref(&p.mime, &p.data)).map(|m| media_json("photo", &m)).collect(),
            };
            if !photos.is_empty() {
                f.insert("photos".into(), json!(photos));
            }
            if let Some(l) = &st.listing {
                f.insert("priceCents".into(), json!(l.price_cents));
                text(&mut f, "currency", &l.currency);
                f.insert("capacity".into(), json!(l.capacity));
                f.insert("open".into(), json!(l.open as i64));
            }
            f.insert("going".into(), json!(going_of(st, &ctx.owner, ctx.roster)));
        }
        KindState::Post(st) => {
            text(&mut f, "title", &st.title);
            text(&mut f, "excerpt", &st.excerpt);
            f.insert("form".into(), json!(st.form.as_str()));
            for (k, v) in [("icon", &st.icon), ("iconMime", &st.icon_mime), ("banner", &st.banner), ("bannerMime", &st.banner_mime), ("bannerAlt", &st.banner_alt)] {
                text(&mut f, k, v);
            }
            if st.retracted {
                f.insert("retracted".into(), json!(1));
            }
            if ext.outside.readable_outside && !st.retracted {
                f.insert("readableOutside".into(), json!(1));
                text(&mut f, "body", &st.body);
                text(&mut f, "bodyFormat", &st.body_format);
                text(&mut f, "link", &st.link);
                if let Some((doc, name)) = &st.document {
                    f.insert("document".into(), media_json("document", doc));
                    text(&mut f, "documentName", name);
                }
                let assets: Vec<_> = st
                    .assets
                    .iter()
                    .filter(|(id, _)| !st.assets_removed.contains(*id))
                    .map(|(id, a)| json!({ "id": id, "asset": media_json("asset", &a.media), "alt": a.alt, "at": a.at }))
                    .collect();
                if !assets.is_empty() {
                    f.insert("assets".into(), json!(assets));
                }
            }
        }
        KindState::Other => {}
    }
    f
}

/// The keys a kind's hosted document may hold (`facets.exterior.fields`).
pub fn declared_fields(kind: crate::object::ObjectKind) -> Vec<&'static str> {
    use crate::object::ObjectKind;
    let own: &[&[&str]] = match kind {
        ObjectKind::Event => &[icd::FIELDS_EVENT],
        ObjectKind::Post => &[icd::FIELDS_POST, icd::FIELDS_POST_READABLE],
        _ => &[],
    };
    icd::FIELDS_EVERY.iter().chain(own.iter().flat_map(|l| l.iter())).copied().collect()
}

/// `base.setDoor`'s args; `None` closes the door.
pub fn set_door_args(key: Option<&[u8; 32]>, mode: DoorMode) -> Args {
    let mut a = Args::new();
    a.insert("key".into(), ArgVal::Text(key.map(hex::encode).unwrap_or_default()));
    a.insert("mode".into(), ArgVal::Text(mode.as_str().into()));
    a
}

/// `base.decideRequest`'s args; `authoring::build` adds the gen.
pub fn decide_request_args(request: &[u8; 32], asker: &MemberId, admit: bool) -> Args {
    let mut a = Args::new();
    a.insert("request".into(), ArgVal::Text(hex::encode(request)));
    a.insert("asker".into(), ArgVal::Text(hex::encode(asker)));
    a.insert("decision".into(), ArgVal::Text(if admit { "admit" } else { "decline" }.into()));
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{LogPosition, ReduceContext};

    const OWNER: MemberId = [1; 32];
    const OTHER: MemberId = [2; 32];

    fn run(state: &mut ExteriorState, op_id: u32, args: &Args, author: &MemberId, admitter: bool) -> Result<(), DeltaRejection> {
        let members = [OWNER, OTHER];
        let ctx = ReduceContext { members: &members, owner: OWNER, epoch: 0 };
        reduce_exterior(state, &Op { op_id, args, author, pos: Some(LogPosition { epoch: 0, seq: 1 }), ctx: &ctx }, admitter)
    }

    /// A row as the owner's device writes it: canonical envelope, A-10 signature, payload.
    fn row(id: &crate::identity::Identity, group: &[u8], kind: crate::object::ObjectKind, op_id: u32, args: Args, seq: u64) -> Vec<u8> {
        let delta = crate::coordinator::Delta {
            type_id: kind.type_id() as u32,
            op_id,
            op_version: 1,
            args,
            epoch: 0,
            prev: [0; 32],
            seq: Some(seq),
            gen: None,
            visibility: Default::default(),
        };
        let sig = crate::delta_sig::sign_delta(id, group, &delta.id());
        crate::delta_sig::seal_payload(&delta.canonical_bytes(), &id.identity_pk(), &sig)
    }

    fn vis_args(v: &str) -> Args {
        [("visibility".to_string(), ArgVal::Text(v.into()))].into_iter().collect()
    }

    fn carry(owner: &MemberId, ext: &[u8], vis: Option<&[u8]>, hops: u32) -> Args {
        let mut a = publish_exterior_args(&hex::encode(GROUP), "event", owner, ext, vis, hops);
        a.insert("gen".into(), ArgVal::Int(1));
        a
    }

    const GROUP: [u8; 4] = [0xaa, 0xbb, 0xcc, 0xdd];

    #[test]
    fn a_carried_exterior_reaches_as_far_as_its_owner_says() {
        use crate::object::ObjectKind::Event;
        let owner = crate::identity::Identity::in_memory([1; 32]);
        let pk = owner.identity_pk();
        let ext = row(&owner, &GROUP, Event, OP_SET_EXTERIOR, set_exterior_args("Gig", "Friday", None, Some(&[7; 32])), 1);
        let conn = row(&owner, &GROUP, Event, crate::visibility::OP_SET_VISIBILITY, vis_args("connections"), 2);
        let net = row(&owner, &GROUP, Event, crate::visibility::OP_SET_VISIBILITY, vis_args("network"), 3);
        let private = row(&owner, &GROUP, Event, crate::visibility::OP_SET_VISIBILITY, vis_args("private"), 4);
        let mut held = BTreeMap::new();
        let fold = |held: &mut BTreeMap<String, Carried>, vis: &[u8], hops| fold_carried(held, carried(&carry(&pk, &ext, Some(vis), hops), &OTHER)?);

        assert_eq!(fold(&mut held, &conn, 2), Err(DeltaRejection::PreconditionFailed), "connections reach one hop");
        fold(&mut held, &conn, 1).unwrap();
        assert_eq!(held[&hex::encode(GROUP)].exterior.name, "Gig");
        fold(&mut held, &net, 2).unwrap();
        assert_eq!((held[&hex::encode(GROUP)].hops, held[&hex::encode(GROUP)].visibility), (2, crate::visibility::Visibility::Network), "the newer row wins");
        fold(&mut held, &private, 2).unwrap();
        assert!(held[&hex::encode(GROUP)].withdrawn, "a newer, narrower visibility withdraws the copy");
        assert_eq!(fold(&mut held, &net, 1), Ok(()));
        assert!(held[&hex::encode(GROUP)].withdrawn, "an older copy does not come back");
    }

    #[test]
    fn a_row_the_owner_did_not_sign_for_this_object_is_refused() {
        use crate::object::ObjectKind::Event;
        let owner = crate::identity::Identity::in_memory([1; 32]);
        let other = crate::identity::Identity::in_memory([2; 32]);
        let net = |id: &crate::identity::Identity, g: &[u8]| row(id, g, Event, crate::visibility::OP_SET_VISIBILITY, vis_args("network"), 2);
        let forged = row(&other, &GROUP, Event, OP_SET_EXTERIOR, set_exterior_args("Gig", "", None, None), 1);
        let pk = owner.identity_pk();
        assert_eq!(carried(&carry(&pk, &forged, Some(&net(&owner, &GROUP)), 1), &OTHER).err(), Some(DeltaRejection::Unauthorized), "another key's row");
        let elsewhere = row(&owner, &[9, 9], Event, OP_SET_EXTERIOR, set_exterior_args("Gig", "", None, None), 1);
        assert_eq!(carried(&carry(&pk, &elsewhere, Some(&net(&owner, &GROUP)), 1), &OTHER).err(), Some(DeltaRejection::Unauthorized), "another object's row");
        let thing = row(&owner, &GROUP, crate::object::ObjectKind::Thing, OP_SET_EXTERIOR, set_exterior_args("Gig", "", None, None), 1);
        assert_eq!(carried(&carry(&pk, &thing, None, 1), &OTHER).err(), Some(DeltaRejection::MalformedArgs), "a row of another kind");
    }

    /// `contact.publishExterior` cannot be probed by icd.rs (its rows need the owner's secret
    /// key), so its declared args are held to what `carried` reads here.
    #[test]
    fn the_leg_reads_exactly_the_args_the_icd_declares() {
        use crate::object::ObjectKind::Event;
        let owner = crate::identity::Identity::in_memory([1; 32]);
        let ext = row(&owner, &GROUP, Event, OP_SET_EXTERIOR, set_exterior_args("Gig", "", None, None), 1);
        let net = row(&owner, &GROUP, Event, crate::visibility::OP_SET_VISIBILITY, vis_args("network"), 2);
        let args = carry(&owner.identity_pk(), &ext, Some(&net), 1);
        let (got, read) = crate::arg_reads::recording(|| carried(&args, &OTHER).map(|_| ()));
        got.unwrap();
        let icd: serde_json::Value = serde_json::from_str(include_str!("../../coordination/delta-graph.icd.json")).unwrap();
        let declared: std::collections::BTreeSet<String> =
            icd["kinds"]["contact"]["ops"]["contact.publishExterior"]["args"].as_object().unwrap().keys().cloned().collect();
        // The rows' own args are their ops' (base.setExterior's, base.setVisibility's), read as those.
        let leg: std::collections::BTreeSet<String> = read.into_iter().filter(|k| declared.contains(k)).collect();
        assert_eq!(leg, declared);
    }

    #[test]
    fn an_exterior_is_named_and_capped() {
        let mut s = ExteriorState::default();
        run(&mut s, OP_SET_EXTERIOR, &set_exterior_args("Allotment", "Plots by the river", None, Some(&[7; 32])), &OWNER, false).unwrap();
        let e = s.exterior.clone().unwrap();
        assert_eq!((e.name.as_str(), e.description.as_str(), e.door), ("Allotment", "Plots by the river", Some([7; 32])));
        assert_eq!(run(&mut s, OP_SET_EXTERIOR, &set_exterior_args(" ", "", None, None), &OWNER, false), Err(DeltaRejection::MalformedArgs));
        let long = "x".repeat(icd::DESCRIPTION_MAX + 1);
        assert_eq!(run(&mut s, OP_SET_EXTERIOR, &set_exterior_args("A", &long, None, None), &OWNER, false), Err(DeltaRejection::MalformedArgs));
        assert_eq!(s.exterior, Some(e), "a refused row changes nothing");
    }

    #[test]
    fn a_door_opens_and_closes() {
        let mut s = ExteriorState::default();
        run(&mut s, OP_SET_DOOR, &set_door_args(Some(&[9; 32]), DoorMode::Review), &OWNER, false).unwrap();
        assert_eq!(s.door, Some(Door { key: [9; 32], mode: DoorMode::Review }));
        run(&mut s, OP_SET_DOOR, &set_door_args(None, DoorMode::Review), &OWNER, false).unwrap();
        assert_eq!(s.door, None);
    }

    #[test]
    fn the_first_decision_holds_and_only_an_admitter_decides() {
        let mut s = ExteriorState::default();
        let mut decline = decide_request_args(&[3; 32], &[4; 32], false);
        decline.insert("gen".into(), ArgVal::Int(1));
        assert_eq!(run(&mut s, OP_DECIDE_REQUEST, &decline, &OTHER, false), Err(DeltaRejection::Unauthorized));
        run(&mut s, OP_DECIDE_REQUEST, &decline, &OTHER, true).unwrap();
        let mut admit = decide_request_args(&[3; 32], &[4; 32], true);
        admit.insert("gen".into(), ArgVal::Int(2));
        run(&mut s, OP_DECIDE_REQUEST, &admit, &OWNER, false).unwrap();
        assert!(!s.decided[&[3; 32]].admit, "the first decision holds");
    }

    fn outside(precision: u8, readable: bool) -> Exterior {
        let args = set_exterior_args_with("Gig", "Friday", None, None, &Outside { precision, readable_outside: readable, hosted: None });
        parse(&args).unwrap()
    }

    fn ctx<'a>(roster: &'a [MemberId], name_of: &'a dyn Fn(&MemberId) -> String) -> FieldsCtx<'a> {
        FieldsCtx { owner: OWNER, roster, name_of, venue_point: None }
    }

    /// Every key the builder writes is one the ICD lists for the kind: nothing of the interior
    /// leaves by a field added to the code and not to the ICD.
    #[test]
    fn the_fields_are_the_icds_and_nothing_else() {
        use crate::event::{EventState, TicketListing};
        let mut ev = EventState { title: "Gig".into(), start_ms: 1, end_ms: Some(2), tz: "Europe/London".into(), venue: "The Lantern".into(), lineup: vec!["Band".into()], all_day: true, ..Default::default() };
        ev.location = Some(crate::geo::LocationSource::Fixed { point: crate::geo::GeoPoint::from_degrees(51.45, -2.58) });
        ev.listing = Some(TicketListing { price_cents: 500, currency: "gbp".into(), capacity: 80, open: true, acct: Some("acct_secret".into()), delegate: None, terms: Some("terms".into()), rev: 1 });
        ev.media.banner = "aGk=".into();
        ev.media.banner_mime = "image/jpeg".into();
        let name = |_: &MemberId| "Olive".to_string();
        let f = fields_of(&outside(5, false), KindState::Event(&ev), &ctx(&[OWNER, OTHER], &name));
        let declared = declared_fields(crate::object::ObjectKind::Event);
        for k in f.keys() {
            assert!(declared.contains(&k.as_str()), "{k} is not in facets.exterior.fields.event");
        }
        let text = serde_json::to_string(&f).unwrap();
        assert!(!text.contains("acct_secret") && !text.contains("terms"), "the ticket account and terms stay inside");
        let mut post = crate::post::PostState { title: "T".into(), body: "words".into(), excerpt: "e".into(), ..Default::default() };
        post.assets.insert("a1".into(), crate::post::Asset { media: pacific_media::MediaRef::inline("image/png", "aGk=").unwrap(), alt: "x".into(), at: 1 });
        let f = fields_of(&outside(0, true), KindState::Post(&post), &ctx(&[], &name));
        let declared = declared_fields(crate::object::ObjectKind::Post);
        for k in f.keys() {
            assert!(declared.contains(&k.as_str()), "{k} is not in facets.exterior.fields.post");
        }
    }

    /// The Place shows at the owner's precision: a geohash of that length, or nothing at 0.
    #[test]
    fn the_place_shows_at_the_owners_precision() {
        let mut ev = crate::event::EventState { venue: "The Lantern".into(), ..Default::default() };
        let name = |_: &MemberId| String::new();
        let mut c = ctx(&[], &name);
        c.venue_point = Some(crate::geo::GeoPoint::from_degrees(51.4545, -2.5879));
        let at = |p| fields_of(&outside(p, false), KindState::Event(&ev), &c).get("point").cloned();
        assert_eq!(at(0), None, "precision 0: the venue's name alone");
        assert_eq!(at(4), Some(serde_json::json!("gcnh")));
        assert_eq!(at(7).unwrap().as_str().unwrap().len(), 7);
        ev.location = Some(crate::geo::LocationSource::Fixed { point: crate::geo::GeoPoint::from_degrees(-33.8688, 151.2093) });
        let own = fields_of(&outside(3, false), KindState::Event(&ev), &c);
        assert_eq!(own["point"], serde_json::json!("r3g"), "the Event's own point before its venue's");
        assert_eq!(own["venue"], serde_json::json!("The Lantern"));
        assert_eq!(parse(&set_exterior_args_with("G", "", None, None, &Outside { precision: icd::PRECISION_MAX + 1, ..Default::default() })), Err(DeltaRejection::MalformedArgs));
    }

    /// Going counts who attends: distinct identities, never the owner, a co-host or an admitter.
    #[test]
    fn going_counts_attendees_alone() {
        use crate::group::GroupRole;
        let mut ev = crate::event::EventState::default();
        let (cohost, arc, a, b) = ([5; 32], [6; 32], [7; 32], [8; 32]);
        ev.member_roles.insert(cohost, GroupRole::Admin);
        ev.member_roles.insert(arc, GroupRole::Admitter);
        assert_eq!(going_of(&ev, &OWNER, &[OWNER, cohost, arc, a, b, a]), 2);
    }

    /// A Post's body goes outside only when its poster says so.
    #[test]
    fn a_posts_body_is_outside_only_when_its_poster_says() {
        let post = crate::post::PostState { title: "T".into(), body: "the words".into(), excerpt: "teaser".into(), ..Default::default() };
        let name = |_: &MemberId| String::new();
        let shut = fields_of(&outside(0, false), KindState::Post(&post), &ctx(&[], &name));
        assert!(!shut.contains_key("body") && shut["excerpt"] == "teaser");
        let open = fields_of(&outside(0, true), KindState::Post(&post), &ctx(&[], &name));
        assert_eq!(open["body"], "the words");
    }

    /// The pointer round-trips: the settings and the hosted key are what a reader parses back.
    #[test]
    fn the_pointer_round_trips() {
        let o = Outside { precision: 6, readable_outside: true, hosted: Some(Hosted { key: "ab".repeat(32), secret: [3; 32] }) };
        let e = parse(&set_exterior_args_with("G", "", None, None, &o)).unwrap();
        assert_eq!(e.outside, o);
        let mut half = set_exterior_args("G", "", None, None);
        half.insert("hostedKey".into(), ArgVal::Text("ab".repeat(32)));
        assert_eq!(parse(&half), Err(DeltaRejection::MalformedArgs), "a key with no content key");
    }
}
