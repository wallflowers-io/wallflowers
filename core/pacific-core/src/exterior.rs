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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exterior {
    pub name: String,
    pub description: String,
    pub photo: Option<pacific_media::MediaRef>,
    /// The door's public key; `None`, no way to ask.
    pub door: Option<[u8; 32]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Door {
    pub key: [u8; 32],
    pub mode: DoorMode,
}

/// One request answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub asker: MemberId,
    pub admit: bool,
    pub by: MemberId,
    pub gen: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
    Ok(Exterior { name: name.to_string(), description, photo, door })
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
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// `base.setExterior`'s args.
pub fn set_exterior_args(name: &str, description: &str, photo: Option<&pacific_media::MediaRef>, door: Option<&[u8; 32]>) -> Args {
    let mut a = Args::new();
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
}
