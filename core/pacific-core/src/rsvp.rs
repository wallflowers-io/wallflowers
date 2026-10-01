//! rsvp — a Site's answers to the Events it created (W-98 Events; ICD 2.3.1 group ops 18–20).
//!
//! `group.rsvp` (member), `group.setRegistration` and `group.rsvpDecide` (owner|role:admin), all
//! commutative, on the Site's own log: an attendee is on no Event roster, so the Site folds the
//! answers. Each is refused unless its `event` is one this group created (its rel=created
//! affiliation edge).
//!
//! ## The register the answers are held to
//!
//! One per event (`group.setRegistration`, LWW by (gen, author)). `group.rsvp`'s fold enforces
//! it, since no fold reads another object: an answer after `closesMs` (by its own `at`), a maybe
//! where `maybe` is 0, or guests past `guestsMax`, is refused. A going answer takes its place and
//! its guests' (`capacity` counts going places); past capacity it waits where `waitlist` is 1 and
//! is refused where it is 0. Where `approval` is 1 it counts only once approved.
//!
//! The fold runs in (gen, author, id) order, so each answer is judged by the register and the
//! places as they stood at its gen: a register written later binds later answers and moves none
//! already given.
//!
//! ## A host's decision
//!
//! `group.rsvpDecide`, one per (event, member), LWW: approved counts as going (the host admits,
//! past capacity too), waitlisted waits for a place, declined is not admitted. It stands for the
//! member's later answers as well.
//!
//! ## Principals (ICD `principals`)
//!
//! `member` is a leaf on the roster, so an author off it answers nothing. `owner|role:admin` is
//! the owner at the delta's epoch, or a roster member holding admin in the Site's own folded
//! roles: the roles are the spine's, folded first, so a revoked admin's writes stop counting.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::coordinator::{ArgVal, Args};
use crate::group::{AffiliationRel, GroupRole, GroupState};
use crate::object::{DeltaRejection, MemberId, Op};
use crate::object_args::{req_int, req_text};

/// Guests an answer may bring, absent a register: `guests` is 0 to 10.
pub const MAX_GUESTS: i64 = 10;
/// A guest's name, in characters.
pub const MAX_GUEST_NAME: usize = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Going,
    Maybe,
    Declined,
}

impl Status {
    fn parse(s: &str) -> Result<Self, DeltaRejection> {
        Ok(match s {
            "going" => Self::Going,
            "maybe" => Self::Maybe,
            "declined" => Self::Declined,
            _ => return Err(DeltaRejection::MalformedArgs),
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Going => "going",
            Self::Maybe => "maybe",
            Self::Declined => "declined",
        }
    }
}

/// A host's decision on one member's answer (`group.rsvpDecide`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Approved,
    Waitlisted,
    Declined,
}

impl Decision {
    fn parse(s: &str) -> Result<Self, DeltaRejection> {
        Ok(match s {
            "approved" => Self::Approved,
            "waitlisted" => Self::Waitlisted,
            "declined" => Self::Declined,
            _ => return Err(DeltaRejection::MalformedArgs),
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Waitlisted => "waitlisted",
            Self::Declined => "declined",
        }
    }
}

/// Where a going answer stands: it counts as going, waits for a place, awaits the host's
/// approval, or was not admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Going,
    Waitlisted,
    Pending,
    Declined,
}

impl Place {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Going => "going",
            Self::Waitlisted => "waitlisted",
            Self::Pending => "pending",
            Self::Declined => "declined",
        }
    }
}

/// Whether the venue reaches the Site's public copy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Location {
    #[default]
    Public,
    Members,
}

impl Location {
    fn parse(s: &str) -> Result<Self, DeltaRejection> {
        Ok(match s {
            "public" => Self::Public,
            "members" => Self::Members,
            _ => return Err(DeltaRejection::MalformedArgs),
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Members => "members",
        }
    }
}

/// One member's answer to one event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub status: Status,
    pub guests: i64,
    pub at: i64,
    pub guest_names: Vec<String>,
    pub occurrence: Option<i64>,
    /// Only for a going answer.
    pub place: Option<Place>,
    pub gen: i64,
}

impl Answer {
    /// The going places this answer holds: its own and its guests', where it counts as going.
    fn places(&self) -> i64 {
        if self.place == Some(Place::Going) { 1 + self.guests } else { 0 }
    }
}

/// How the Site takes answers for one event. `Default` is the register absent one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    /// Going places, 0 uncapped.
    pub capacity: i64,
    pub waitlist: bool,
    pub approval: bool,
    pub guests_max: i64,
    pub maybe: bool,
    pub closes_ms: Option<i64>,
    pub location: Location,
    pub gen: i64,
}

impl Default for Registration {
    fn default() -> Self {
        Self { capacity: 0, waitlist: false, approval: false, guests_max: MAX_GUESTS, maybe: true, closes_ms: None, location: Location::Public, gen: 0 }
    }
}

/// Everything the three ops fold to, by event id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rsvps {
    pub answers: BTreeMap<String, BTreeMap<MemberId, Answer>>,
    pub registration: BTreeMap<String, Registration>,
    pub decisions: BTreeMap<String, BTreeMap<MemberId, (Decision, i64)>>,
}

fn gen_of(args: &Args) -> Result<i64, DeltaRejection> {
    let g = req_int(args, "gen")?;
    if g < 0 {
        return Err(DeltaRejection::MalformedArgs);
    }
    Ok(g)
}

/// `event`, 64 hex, lowercase as ids are written.
fn event_of(args: &Args) -> Result<String, DeltaRejection> {
    let e = req_text(args, "event")?;
    if e.len() != 64 || !e.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(DeltaRejection::MalformedArgs);
    }
    Ok(e.to_string())
}

/// An optional integer: absent is `default`; any other type is malformed.
fn int_or(args: &Args, key: &str, default: i64) -> Result<i64, DeltaRejection> {
    match crate::arg_reads::get(args, key) {
        None => Ok(default),
        Some(ArgVal::Int(n)) => Ok(*n),
        Some(_) => Err(DeltaRejection::MalformedArgs),
    }
}

fn flag(args: &Args, key: &str, default: bool) -> Result<bool, DeltaRejection> {
    match int_or(args, key, default as i64)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(DeltaRejection::MalformedArgs),
    }
}

/// The owner at the delta's epoch, or a roster member holding admin in the Site's own roles.
fn owner_or_admin(state: &GroupState, op: &Op<'_>) -> bool {
    *op.author == op.ctx.owner || (op.ctx.is_member(op.author) && state.member_roles.get(op.author) == Some(&GroupRole::Admin))
}

fn created(state: &GroupState, event: &str) -> bool {
    state.affiliations.get(event).is_some_and(|a| a.rel == AffiliationRel::Created)
}

/// `group.rsvp`, `group.setRegistration` and `group.rsvpDecide`. Every arg is validated before
/// the first mutation: a refused delta leaves the state as it was.
pub fn reduce(state: &mut GroupState, op: &Op<'_>) -> Result<(), DeltaRejection> {
    match op.op_id {
        crate::group::OP_RSVP => rsvp(state, op),
        crate::group::OP_SET_REGISTRATION => set_registration(state, op),
        crate::group::OP_RSVP_DECIDE => decide(state, op),
        _ => Err(DeltaRejection::UnknownType),
    }
}

fn rsvp(state: &mut GroupState, op: &Op<'_>) -> Result<(), DeltaRejection> {
    let args = op.args;
    let event = event_of(args)?;
    let status = Status::parse(req_text(args, "status")?)?;
    let guests = int_or(args, "guests", 0)?;
    if !(0..=MAX_GUESTS).contains(&guests) || (guests != 0 && status != Status::Going) {
        return Err(DeltaRejection::MalformedArgs);
    }
    let at = req_int(args, "at")?;
    if at <= 0 {
        return Err(DeltaRejection::MalformedArgs);
    }
    let gen = gen_of(args)?;
    let guest_names = match crate::arg_reads::get(args, "guestNames") {
        None => Vec::new(),
        Some(ArgVal::Text(j)) => {
            let names: Vec<String> = serde_json::from_str(j).map_err(|_| DeltaRejection::MalformedArgs)?;
            let long = |n: &String| n.is_empty() || n.chars().count() > MAX_GUEST_NAME;
            if names.len() as i64 > guests || names.iter().any(long) {
                return Err(DeltaRejection::MalformedArgs);
            }
            names
        }
        Some(_) => return Err(DeltaRejection::MalformedArgs),
    };
    let occurrence = match crate::arg_reads::get(args, "occurrence") {
        None => None,
        Some(ArgVal::Int(n)) => Some(*n),
        Some(_) => return Err(DeltaRejection::MalformedArgs),
    };
    if !op.ctx.is_member(op.author) {
        return Err(DeltaRejection::Unauthorized);
    }
    if !created(state, &event) {
        return Err(DeltaRejection::PreconditionFailed);
    }
    let reg = state.rsvps.registration.get(&event).cloned().unwrap_or_default();
    if reg.closes_ms.is_some_and(|c| at > c)
        || (status == Status::Maybe && !reg.maybe)
        || guests > reg.guests_max
    {
        return Err(DeltaRejection::PreconditionFailed);
    }
    let answers = state.rsvps.answers.get(&event);
    if answers.and_then(|a| a.get(op.author)).is_some_and(|held| held.gen > gen) {
        return Ok(());
    }
    let place = match status {
        Status::Going => Some(match state.rsvps.decisions.get(&event).and_then(|d| d.get(op.author)).map(|(d, _)| *d) {
            Some(Decision::Approved) => Place::Going,
            Some(Decision::Waitlisted) => Place::Waitlisted,
            Some(Decision::Declined) => Place::Declined,
            None if reg.approval => Place::Pending,
            None => {
                let held: i64 = answers.map_or(0, |a| a.iter().filter(|(m, _)| *m != op.author).map(|(_, a)| a.places()).sum());
                if reg.capacity == 0 || held + 1 + guests <= reg.capacity {
                    Place::Going
                } else if reg.waitlist {
                    Place::Waitlisted
                } else {
                    return Err(DeltaRejection::PreconditionFailed);
                }
            }
        }),
        _ => None,
    };
    state
        .rsvps
        .answers
        .entry(event)
        .or_default()
        .insert(*op.author, Answer { status, guests, at, guest_names, occurrence, place, gen });
    Ok(())
}

fn set_registration(state: &mut GroupState, op: &Op<'_>) -> Result<(), DeltaRejection> {
    let args = op.args;
    let event = event_of(args)?;
    let capacity = int_or(args, "capacity", 0)?;
    let waitlist = flag(args, "waitlist", false)?;
    let approval = flag(args, "approval", false)?;
    let guests_max = int_or(args, "guestsMax", MAX_GUESTS)?;
    let maybe = flag(args, "maybe", true)?;
    let closes_ms = match crate::arg_reads::get(args, "closesMs") {
        None => None,
        Some(ArgVal::Int(n)) if *n > 0 => Some(*n),
        Some(_) => return Err(DeltaRejection::MalformedArgs),
    };
    let location = match crate::arg_reads::get(args, "location") {
        None => Location::Public,
        Some(ArgVal::Text(l)) => Location::parse(l)?,
        Some(_) => return Err(DeltaRejection::MalformedArgs),
    };
    let gen = gen_of(args)?;
    if capacity < 0 || !(0..=MAX_GUESTS).contains(&guests_max) {
        return Err(DeltaRejection::MalformedArgs);
    }
    if !owner_or_admin(state, op) {
        return Err(DeltaRejection::Unauthorized);
    }
    if !created(state, &event) {
        return Err(DeltaRejection::PreconditionFailed);
    }
    if state.rsvps.registration.get(&event).is_some_and(|r| r.gen > gen) {
        return Ok(());
    }
    state
        .rsvps
        .registration
        .insert(event, Registration { capacity, waitlist, approval, guests_max, maybe, closes_ms, location, gen });
    Ok(())
}

fn decide(state: &mut GroupState, op: &Op<'_>) -> Result<(), DeltaRejection> {
    let args = op.args;
    let event = event_of(args)?;
    let member = crate::object_args::arg_hex32(args, "member")?;
    let decision = Decision::parse(req_text(args, "decision")?)?;
    let gen = gen_of(args)?;
    if !owner_or_admin(state, op) {
        return Err(DeltaRejection::Unauthorized);
    }
    if !created(state, &event) {
        return Err(DeltaRejection::PreconditionFailed);
    }
    let decisions = state.rsvps.decisions.entry(event.clone()).or_default();
    if decisions.get(&member).is_some_and(|(_, g)| *g > gen) {
        return Ok(());
    }
    decisions.insert(member, (decision, gen));
    if let Some(a) = state.rsvps.answers.get_mut(&event).and_then(|a| a.get_mut(&member)) {
        if a.status == Status::Going {
            a.place = Some(match decision {
                Decision::Approved => Place::Going,
                Decision::Waitlisted => Place::Waitlisted,
                Decision::Declined => Place::Declined,
            });
        }
    }
    Ok(())
}

/// The view's `rsvps`, `rsvp_counts` and `registration` (the ICD's `view` lines): each answer
/// `{status, guests, at, guestNames, occurrence}`, with `place` where it is going and
/// `decision` where one was made; per event `{going, maybe, declined, guests}`, guests counted
/// only for going; each register as written.
pub fn view(r: &Rsvps) -> (Value, Value, Value) {
    let mut rsvps = serde_json::Map::new();
    let mut counts = serde_json::Map::new();
    for (event, answers) in &r.answers {
        let decided = r.decisions.get(event);
        let (mut going, mut maybe, mut declined, mut guests) = (0, 0, 0, 0);
        let mut each = serde_json::Map::new();
        for (m, a) in answers {
            let mut o = json!({
                "status": a.status.as_str(), "guests": a.guests, "at": a.at,
                "guestNames": a.guest_names, "occurrence": a.occurrence,
            });
            if let Some(p) = a.place {
                o["place"] = p.as_str().into();
            }
            if let Some((d, _)) = decided.and_then(|d| d.get(m)) {
                o["decision"] = d.as_str().into();
            }
            match (a.status, a.place) {
                (Status::Going, Some(Place::Going)) => {
                    going += 1;
                    guests += a.guests;
                }
                (Status::Maybe, _) => maybe += 1,
                (Status::Declined, _) => declined += 1,
                _ => {}
            }
            each.insert(hex::encode(m), o);
        }
        rsvps.insert(event.clone(), Value::Object(each));
        counts.insert(event.clone(), json!({ "going": going, "maybe": maybe, "declined": declined, "guests": guests }));
    }
    let registration = r
        .registration
        .iter()
        .map(|(e, g)| {
            (e.clone(), json!({
                "capacity": g.capacity, "waitlist": g.waitlist as i64, "approval": g.approval as i64,
                "guestsMax": g.guests_max, "maybe": g.maybe as i64, "closesMs": g.closes_ms,
                "location": g.location.as_str(),
            }))
        })
        .collect();
    (Value::Object(rsvps), Value::Object(counts), Value::Object(registration))
}

#[cfg(test)]
mod tests {
    /// The caps are the ICD's: the args' summaries state the numbers this fold holds.
    #[test]
    fn the_caps_are_the_icds() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json");
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let args = &doc["kinds"]["group"]["ops"]["group.rsvp"]["args"];
        assert!(args["guests"]["summary"].as_str().unwrap().contains(&format!("0 to {}", super::MAX_GUESTS)));
        assert!(args["guestNames"]["summary"].as_str().unwrap().contains(&format!("1 to {} characters", super::MAX_GUEST_NAME)));
        let reg = &doc["kinds"]["group"]["ops"]["group.setRegistration"]["args"];
        assert!(reg["guestsMax"]["summary"].as_str().unwrap().contains(&format!("default {}", super::MAX_GUESTS)));
    }
}
