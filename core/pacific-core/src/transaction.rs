//! transaction — ONE SALE, recorded (W-98 Trade; ICD `kinds.transaction`).
//!
//! Record only (T-6): no op moves money and WallFlowers takes no fee; `amount` is what the
//! parties agreed and paid between them. Three principals, fixed for the lifecycle (T-2, T-3):
//! the SETTLER owns the object and holds no deal role, the SELLER states the terms and attests
//! the sale, the BUYER accepts them and attests receipt. A seller or buyer may be a Site,
//! through the WallFlowers system node (`transaction.setSite`).
//!
//! THE STATE MACHINE IS THE ICD'S (T-4): its states, initial state and transitions (`from`,
//! `to`, `records`, `when`) are read from `kinds.transaction.stateMachine`, never restated.
//! What the ICD states in prose, each transition's guard, is code here, one function a
//! transition. The Coordinator folds commutative ops in (gen, author, id) order, so every
//! replica steps the machine through the same sequence; an op its state does not allow is
//! inert and named in `nonconforming`, never an error. Completion by silence is read time
//! (`view`): the fold holds `accepted`.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::coordinator::{ArgVal, Args};
use crate::group::GroupRole;
use crate::object::{Authority, Commutativity, DeltaRejection, MemberId, ObjectKind, ObjectType, Op, OpDecl};

pub const OP_SET_TERMS: u32 = 0;
pub const OP_ACCEPT: u32 = 1;
pub const OP_CONFIRM_RECEIPT: u32 = 2;
pub const OP_DISPUTE: u32 = 3;
pub const OP_CONFIRM_SALE: u32 = 4;
pub const OP_CANCEL: u32 = 5;
pub const OP_RESOLVE: u32 = 6;
pub const OP_FRAUD_SIGNAL: u32 = 7;
pub const OP_RATE: u32 = 8;
pub const OP_SET_SITE: u32 = 9;

const SELLER: &[GroupRole] = &[GroupRole::Seller];
const BUYER: &[GroupRole] = &[GroupRole::Buyer];
const PARTY: &[GroupRole] = &[GroupRole::Buyer, GroupRole::Seller];
const SETTLER: &[GroupRole] = &[GroupRole::Settler];

const fn deal(op_id: u32, name: &'static str, who: &'static [GroupRole]) -> OpDecl {
    OpDecl { op_id, name, authority: Authority::Roles(who), commutativity: Commutativity::Commutative }
}

static OPS: &[OpDecl] = &[
    deal(OP_SET_TERMS, "transaction.setTerms", SELLER),
    deal(OP_ACCEPT, "transaction.accept", BUYER),
    deal(OP_CONFIRM_RECEIPT, "transaction.confirmReceipt", BUYER),
    deal(OP_DISPUTE, "transaction.dispute", PARTY),
    deal(OP_CONFIRM_SALE, "transaction.confirmSale", SELLER),
    deal(OP_CANCEL, "transaction.cancel", PARTY),
    deal(OP_RESOLVE, "transaction.resolve", SETTLER),
    deal(OP_FRAUD_SIGNAL, "transaction.fraudSignal", SETTLER),
    deal(OP_RATE, "transaction.rate", PARTY),
    OpDecl { op_id: OP_SET_SITE, name: "transaction.setSite", authority: Authority::Owner, commutativity: Commutativity::Sequenced },
    // The roles facet (base band 0xF007), spliced: a Transaction's roles are fixed
    // (`roles::reduce_fixed_roles`).
    OpDecl { op_id: crate::roles::OP_SET_ROLE, name: "base.setRole", authority: Authority::Owner, commutativity: Commutativity::Sequenced },
    OpDecl { op_id: crate::roles::OP_CLEAR_ROLE, name: "base.clearRole", authority: Authority::Owner, commutativity: Commutativity::Sequenced },
    // The parent facet (base band 0xF008), spliced: any kind can be a part (ICD `facets.parent`).
    OpDecl { op_id: crate::parent::OP_SET_PARENT, name: "base.setParent", authority: Authority::Owner, commutativity: Commutativity::Sequenced },
    OpDecl { op_id: crate::parent::OP_CLEAR_PARENT, name: "base.clearParent", authority: Authority::Owner, commutativity: Commutativity::Sequenced },
];

/// The standing terms, or the frozen ones once accepted.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Terms {
    pub gen: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub thing: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub descriptor: String,
    pub qty: i64,
    pub amount: i64,
    pub currency: String,
    pub delivery: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accept_by: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ship_by: Option<i64>,
    pub confirm_within: i64,
    pub at: i64,
}

/// One party's word at a moment: an attestation, an acceptance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Stamp {
    pub at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Dispute {
    pub by: String,
    pub reason: String,
    pub at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Resolution {
    pub outcome: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    pub at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Rating {
    pub stars: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    pub at: i64,
    #[serde(skip)]
    pub gen: i64,
}

/// An op the machine did not take, and why, in its words.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Nonconforming {
    pub op: String,
    pub author: String,
    pub gen: i64,
    pub why: String,
}

/// The Site a Site's sale sells through, and its Thing (`transaction.setSite`).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteSale {
    pub site: String,
    pub thing_id: String,
    pub at: i64,
}

#[derive(Clone, Debug, Default)]
pub struct TransactionState {
    pub roles: BTreeMap<MemberId, GroupRole>,
    /// An op other than a role has folded: the roles are fixed from here (T-2).
    pub began: bool,
    pub site: Option<SiteSale>,
    /// One of the machine's states; empty until the first op reads the initial one.
    pub state: String,
    pub terms: Option<Terms>,
    pub accepted: Option<Stamp>,
    pub receipt: Option<Stamp>,
    pub sale: Option<Stamp>,
    pub dispute: Option<Dispute>,
    pub resolution: Option<Resolution>,
    pub ratings: BTreeMap<&'static str, Rating>,
    pub nonconforming: Vec<Nonconforming>,
    /// What this Transaction is a part of, and in what role (`base.setParent`).
    pub parent: Option<crate::parent::ParentRef>,
}

pub struct TransactionType;

impl ObjectType for TransactionType {
    const KIND: ObjectKind = ObjectKind::Transaction;
    type State = TransactionState;

    fn ops() -> &'static [OpDecl] {
        OPS
    }

    fn reduce(st: &mut TransactionState, op: &Op<'_>) -> Result<(), DeltaRejection> {
        if st.state.is_empty() {
            st.state = machine().initial.clone();
        }
        if crate::parent::is_parent_op(op.op_id) {
            return crate::parent::reduce_parent(&mut st.parent, op);
        }
        match op.op_id {
            // A deal's roles are never cleared: the op is inert and named, as the machine
            // names every op it does not take.
            crate::roles::OP_CLEAR_ROLE => {
                let member = text(op.args, "member").unwrap_or_default();
                st.nonconforming.push(Nonconforming { op: "base.clearRole".into(), author: hex::encode(op.author), gen: 0, why: format!("a deal's roles are never cleared ({member})") });
                Ok(())
            }
            crate::roles::OP_SET_ROLE => crate::roles::reduce_fixed_roles(&mut st.roles, op, st.began),
            OP_SET_SITE => {
                if st.began || st.site.is_some() {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                shape("transaction.setSite", op.args).map_err(|_| DeltaRejection::MalformedArgs)?;
                st.site = Some(SiteSale { site: text(op.args, "site").unwrap_or_default(), thing_id: text(op.args, "thingId").unwrap_or_default(), at: int(op.args, "at").unwrap_or(0) });
                st.began = true;
                Ok(())
            }
            id => {
                let decl = Self::op(id).ok_or(DeltaRejection::UnknownType)?;
                let role = party(st, decl, op.author).ok_or(DeltaRejection::Unauthorized)?;
                shape(decl.name, op.args).map_err(|_| DeltaRejection::MalformedArgs)?;
                st.began = true;
                if let Err(why) = step(st, decl.name, op.args, role) {
                    st.nonconforming.push(Nonconforming { op: decl.name.to_string(), author: hex::encode(op.author), gen: int(op.args, "gen").unwrap_or(0), why });
                }
                Ok(())
            }
        }
    }

    fn role_of(state: &TransactionState, member: &MemberId) -> Option<GroupRole> {
        state.roles.get(member).copied()
    }

    const RULES: bool = true;

    /// The fold's rules, asked of the state the author holds before it is written: the same
    /// shape, the same machine, stepped on a copy. `gen` is not yet stamped, so a placeholder
    /// stands in for it.
    fn refuses(st: &TransactionState, op_id: u32, args: &Args, author: &MemberId) -> Option<String> {
        match op_id {
            crate::roles::OP_SET_ROLE if st.began => Some("the roles are fixed once the deal has begun".into()),
            crate::roles::OP_CLEAR_ROLE => Some("a deal's roles are never cleared".into()),
            crate::roles::OP_SET_ROLE => None,
            OP_SET_SITE if st.began || st.site.is_some() => Some("the Site is named once, before the deal begins".into()),
            OP_SET_SITE => shape("transaction.setSite", args).err(),
            OP_FRAUD_SIGNAL => Some("declared, not implemented".into()),
            id => {
                let decl = Self::op(id)?;
                let role = party(st, decl, author)?;
                let mut a = args.clone();
                a.entry("gen".into()).or_insert(ArgVal::Int(i64::MAX));
                if let Err(why) = shape(decl.name, &a) {
                    return Some(why);
                }
                step(&mut st.clone(), decl.name, &a, role).err()
            }
        }
    }
}

/// The machine, as the ICD states it, and each op's args.
#[derive(Debug)]
pub struct Machine {
    pub states: Vec<String>,
    pub initial: String,
    pub transitions: Vec<Transition>,
    /// `transaction.setTerms`' confirmWithin default, ms.
    pub confirm_within: i64,
    /// Each op's declared args, by op name.
    pub args: BTreeMap<String, Vec<ArgRule>>,
}

/// One declared arg: its type, whether it is required, and what the ICD bounds it by.
#[derive(Debug)]
pub struct ArgRule {
    pub name: String,
    pub int: bool,
    pub required: bool,
    pub vocabulary: Option<Vec<String>>,
    pub max_length: Option<usize>,
    pub pattern: Option<String>,
}

#[derive(Debug)]
pub struct Transition {
    pub op: Option<String>,
    pub when: Option<String>,
    pub from: Vec<String>,
    pub to: Option<String>,
    pub records: Option<String>,
}

pub fn machine() -> &'static Machine {
    static M: OnceLock<Machine> = OnceLock::new();
    M.get_or_init(|| {
        let icd: serde_json::Value = serde_json::from_str(include_str!("../../coordination/delta-graph.icd.json")).expect("the ICD");
        let k = &icd["kinds"]["transaction"];
        let sm = &k["stateMachine"];
        let strs = |v: &serde_json::Value| v.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect::<Vec<_>>();
        let opt = |v: &serde_json::Value| v.as_str().map(String::from);
        Machine {
            states: sm["states"].as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default(),
            initial: sm["initial"].as_str().expect("stateMachine.initial").to_string(),
            transitions: sm["transitions"]
                .as_array()
                .expect("stateMachine.transitions")
                .iter()
                .map(|t| Transition { op: opt(&t["op"]), when: opt(&t["when"]), from: strs(&t["from"]), to: opt(&t["to"]), records: opt(&t["records"]) })
                .collect(),
            confirm_within: k["ops"]["transaction.setTerms"]["args"]["confirmWithin"]["default"].as_i64().expect("confirmWithin's default"),
            args: k["ops"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(name, o)| {
                    let rules = o["args"]
                        .as_object()
                        .into_iter()
                        .flatten()
                        .map(|(a, d)| ArgRule {
                            name: a.clone(),
                            int: d["type"] == "integer",
                            required: d["required"].as_bool().unwrap_or(false),
                            vocabulary: d["vocabulary"].as_object().map(|v| v.keys().cloned().collect()),
                            max_length: d["maxLength"].as_u64().map(|n| n as usize),
                            pattern: d["pattern"].as_str().map(String::from),
                        })
                        .collect();
                    (name.clone(), rules)
                })
                .collect(),
        }
    })
}

/// The Transaction as the fold leaves it (ICD `kinds.transaction.view`), with no clock: the
/// fold holds `accepted` through a silence. [`at_read`] applies what the reader's clock says.
pub fn view(st: &TransactionState) -> serde_json::Value {
    let state = current(st).to_string();
    let by = (state == "completed").then_some("both");
    let roles: serde_json::Map<String, serde_json::Value> = st.roles.iter().map(|(m, r)| (r.as_str().to_string(), hex::encode(m).into())).collect();
    serde_json::json!({
        "state": state,
        "completedBy": by,
        "roles": roles,
        "site": st.site,
        "terms": st.terms,
        "accepted": st.accepted,
        "receipt": st.receipt,
        "sale": st.sale,
        "dispute": st.dispute,
        "resolution": st.resolution,
        "ratings": st.ratings,
        "nonconforming": st.nonconforming,
    })
}

/// What the view states against the reader's clock, applied as it is read, after the fold
/// cache (`Node::object_view`; TB4): an accepted deal with one attestation and no dispute
/// reads `completed` once the terms' confirmWithin has passed since it, naming who attested
/// (`stateMachine.silence`).
pub fn at_read(view: &mut serde_json::Value, now_ms: i64) {
    if view["state"] != "accepted" {
        return;
    }
    let (receipt, sale) = (view["receipt"]["at"].as_i64(), view["sale"]["at"].as_i64());
    let (who, at) = match (receipt, sale) {
        (Some(at), None) => ("buyer", at),
        (None, Some(at)) => ("seller", at),
        _ => return,
    };
    let window = view["terms"]["confirmWithin"].as_i64().unwrap_or(machine().confirm_within);
    if now_ms >= at.saturating_add(window) {
        view["state"] = "completed".into();
        view["completedBy"] = who.into();
    }
}

/// The machine's current state: the initial one until an op has stepped it.
fn current(st: &TransactionState) -> &str {
    if st.state.is_empty() { &machine().initial } else { &st.state }
}

/// The frozen terms' window after the first attestation, ms.
fn window(st: &TransactionState) -> i64 {
    st.terms.as_ref().map_or(machine().confirm_within, |t| t.confirm_within)
}

/// The one attestation that stands, and whose, when exactly one does.
fn one_attestation(st: &TransactionState) -> Option<(&'static str, Stamp)> {
    match (st.receipt, st.sale) {
        (Some(r), None) => Some(("buyer", r)),
        (None, Some(s)) => Some(("seller", s)),
        _ => None,
    }
}

/// The role the author writes this op as, if they hold one it names.
fn party(st: &TransactionState, decl: &OpDecl, author: &MemberId) -> Option<GroupRole> {
    let r = st.roles.get(author).copied()?;
    match decl.authority {
        Authority::Roles(rs) if rs.contains(&r) => Some(r),
        _ => None,
    }
}

/// The args as the ICD declares them: each required one present, each of its type, each
/// within its vocabulary, maxLength and pattern, and nothing it does not declare. Then the
/// bounds its prose states: a quantity of at least 1, an amount not negative, stars 1 to 5,
/// a reason not empty, ids 64 hex.
fn shape(op: &str, a: &Args) -> Result<(), String> {
    let rules = machine().args.get(op).ok_or_else(|| format!("{op} is not in the model"))?;
    for (k, _) in a.iter() {
        if !rules.iter().any(|r| &r.name == k) {
            return Err(format!("{op} has no arg {k}"));
        }
    }
    for r in rules {
        match (crate::arg_reads::get(a, &r.name), r.int) {
            (None, _) if r.required => return Err(format!("{op} needs {}", r.name)),
            (None, _) => {}
            (Some(ArgVal::Int(_)), true) => {}
            (Some(ArgVal::Text(t)), false) => {
                if r.vocabulary.as_ref().is_some_and(|v| !v.contains(t)) {
                    return Err(format!("{} is not one of {}", r.name, r.vocabulary.as_ref().map(|v| v.join(", ")).unwrap_or_default()));
                }
                if r.max_length.is_some_and(|n| t.chars().count() > n) {
                    return Err(format!("{} is over {} characters", r.name, r.max_length.unwrap_or(0)));
                }
                if let Some(p) = &r.pattern {
                    if !pattern(p, t).expect("a pattern this build reads") {
                        return Err(format!("{} is not {p}", r.name));
                    }
                }
            }
            (Some(_), _) => return Err(format!("{} is not {}", r.name, if r.int { "an integer" } else { "text" })),
        }
    }
    let has = |k: &str| rules.iter().any(|r| r.name == k);
    let at_least = |k: &str, min: i64| !has(k) || int(a, k).map_or(true, |n| n >= min);
    let hex64 = |k: &str| !has(k) || text(a, k).map_or(true, |t| t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit()));
    if !at_least("qty", 1) || !at_least("amount", 0) || !at_least("confirmWithin", 0) {
        return Err("qty is at least 1; amount and confirmWithin are not negative".into());
    }
    if has("stars") && int(a, "stars").is_some_and(|n| !(1..=5).contains(&n)) {
        return Err("stars are 1 to 5".into());
    }
    if op == "transaction.dispute" && text(a, "reason").is_some_and(|r| r.trim().is_empty()) {
        return Err("a dispute has a reason".into());
    }
    if !hex64("thing") || !hex64("site") || !hex64("thingId") {
        return Err("an id is 64 hex".into());
    }
    Ok(())
}

/// The patterns the ICD writes, as this build reads them: `^[A-Z]{n}$`. Any other is
/// `None`, and `the_machine_is_the_icds` names it.
pub fn pattern(p: &str, s: &str) -> Option<bool> {
    let n: usize = p.strip_prefix("^[A-Z]{")?.strip_suffix("}$")?.parse().ok()?;
    Some(s.len() == n && s.bytes().all(|b| b.is_ascii_uppercase()))
}

/// One op through the machine: its transition from the current state, its guard, what it
/// records, where it leads; then any transition a condition takes (`when`). `Err` is why
/// it did not step, and nothing of it stands.
fn step(st: &mut TransactionState, op: &str, a: &Args, role: GroupRole) -> Result<(), String> {
    let m = machine();
    let cur = current(st).to_string();
    let t = m.transitions.iter().find(|t| t.op.as_deref() == Some(op)).ok_or_else(|| format!("{op} has no transition"))?;
    if !t.from.contains(&cur) {
        return Err(format!("not in {cur}"));
    }
    guard(st, op, a, role)?;
    record(st, op, a, role);
    if let Some(to) = &t.to {
        st.state = to.clone();
    }
    for w in m.transitions.iter().filter(|t| t.op.is_none()) {
        if w.from.iter().any(|f| f == current(st)) && when(st, w.when.as_deref()) == Some(true) {
            if let Some(to) = &w.to {
                st.state = to.clone();
            }
        }
    }
    Ok(())
}

/// The conditions the machine's `when` transitions name, as this build reads them; `None`
/// for one it does not, which `the_machine_is_the_icds` names.
pub fn when(st: &TransactionState, w: Option<&str>) -> Option<bool> {
    match w? {
        "receipt and sale both recorded" => Some(st.receipt.is_some() && st.sale.is_some()),
        _ => None,
    }
}

/// Each transition's guard, as the ICD states it in prose.
fn guard(st: &TransactionState, op: &str, a: &Args, role: GroupRole) -> Result<(), String> {
    let at = int(a, "at").unwrap_or(0);
    let not_before = |what: &str, t: Option<i64>| match t {
        Some(t) if at < t => Err(format!("at is before {what}")),
        _ => Ok(()),
    };
    match op {
        "transaction.setTerms" => match &st.site {
            Some(s) if text(a, "thing").as_deref() != Some(s.thing_id.as_str()) => Err("a Site's sale sells the Thing it names".into()),
            _ => Ok(()),
        },
        "transaction.cancel" => not_before("the terms", st.terms.as_ref().map(|t| t.at)),
        "transaction.accept" => {
            let terms = st.terms.as_ref().ok_or("no terms stand")?;
            if int(a, "terms") != Some(terms.gen) {
                return Err(format!("the standing terms are gen {}", terms.gen));
            }
            if terms.accept_by.is_some_and(|by| at > by) {
                return Err("after the terms' acceptBy".into());
            }
            not_before("the terms", Some(terms.at))
        }
        "transaction.confirmReceipt" if st.receipt.is_some() => Err("receipt is confirmed once".into()),
        "transaction.confirmSale" if st.sale.is_some() => Err("the sale is confirmed once".into()),
        "transaction.confirmReceipt" | "transaction.confirmSale" => not_before("the accept", st.accepted.map(|s| s.at)),
        "transaction.dispute" => match one_attestation(st) {
            Some((_, s)) if at > s.at.saturating_add(window(st)) => Err("after the terms' confirmWithin".into()),
            Some((_, s)) => not_before("the attestation it contests", Some(s.at)),
            None => not_before("the accept", st.accepted.map(|s| s.at)),
        },
        "transaction.resolve" => not_before("the dispute", st.dispute.as_ref().map(|d| d.at)),
        "transaction.rate" => match current(st) {
            "completed" => Ok(()),
            "resolved" if st.resolution.as_ref().is_some_and(|r| r.outcome == "completed") => Ok(()),
            "resolved" => Err("the deal was cancelled".into()),
            _ => match one_attestation(st) {
                Some((_, s)) if at >= s.at.saturating_add(window(st)) => Ok(()),
                _ => Err("not yet completed".into()),
            },
        },
        _ => {
            let _ = role;
            Ok(())
        }
    }
}

/// What a stepped op records.
fn record(st: &mut TransactionState, op: &str, a: &Args, role: GroupRole) {
    let at = int(a, "at").unwrap_or(0);
    match op {
        "transaction.setTerms" => {
            st.terms = Some(Terms {
                gen: int(a, "gen").unwrap_or(0),
                thing: text(a, "thing").unwrap_or_default(),
                descriptor: text(a, "descriptor").unwrap_or_default(),
                qty: int(a, "qty").unwrap_or(1),
                amount: int(a, "amount").unwrap_or(0),
                currency: text(a, "currency").unwrap_or_default(),
                delivery: text(a, "delivery").unwrap_or_default(),
                accept_by: int(a, "acceptBy"),
                ship_by: int(a, "shipBy"),
                confirm_within: int(a, "confirmWithin").unwrap_or(machine().confirm_within),
                at,
            })
        }
        "transaction.accept" => st.accepted = Some(Stamp { at }),
        "transaction.confirmReceipt" => st.receipt = Some(Stamp { at }),
        "transaction.confirmSale" => st.sale = Some(Stamp { at }),
        "transaction.dispute" => st.dispute = Some(Dispute { by: role.as_str().into(), reason: text(a, "reason").unwrap_or_default(), at }),
        "transaction.resolve" => st.resolution = Some(Resolution { outcome: text(a, "outcome").unwrap_or_default(), note: text(a, "note").unwrap_or_default(), at }),
        "transaction.rate" => {
            st.ratings.insert(if role == GroupRole::Buyer { "buyer" } else { "seller" }, Rating { stars: int(a, "stars").unwrap_or(0), text: text(a, "text").unwrap_or_default(), at, gen: int(a, "gen").unwrap_or(0) });
        }
        _ => {}
    }
}

fn int(a: &Args, k: &str) -> Option<i64> {
    match crate::arg_reads::get(a, k) {
        Some(ArgVal::Int(n)) => Some(*n),
        _ => None,
    }
}

fn text(a: &Args, k: &str) -> Option<String> {
    match crate::arg_reads::get(a, k) {
        Some(ArgVal::Text(t)) => Some(t.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator::{sequenced_delta, Coordinator, GENESIS_PREV};
    use crate::object::build_delta;

    const SETTLER_ID: MemberId = [1u8; 32];
    const SELLER_ID: MemberId = [2u8; 32];
    const BUYER_ID: MemberId = [3u8; 32];
    const DAY: i64 = 86_400_000;
    const T0: i64 = 1_790_000_000_000;

    /// A Transaction as its settler mints it: the three roles, then (for a Site's sale) the
    /// Site, on the sequenced spine; then the parties' commutative ops, each (op, author,
    /// gen, args).
    struct Deal {
        c: Coordinator<TransactionType>,
        prev: [u8; 32],
        seq: u64,
    }

    fn a(pairs: &[(&str, ArgVal)]) -> Args {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }
    fn t(s: &str) -> ArgVal {
        ArgVal::Text(s.into())
    }
    fn i(n: i64) -> ArgVal {
        ArgVal::Int(n)
    }

    impl Deal {
        fn new() -> Self {
            let mut d = Deal { c: Coordinator::new(vec![SETTLER_ID, SELLER_ID, BUYER_ID], SETTLER_ID), prev: GENESIS_PREV, seq: 0 };
            for (who, role) in [(SETTLER_ID, "settler"), (SELLER_ID, "seller"), (BUYER_ID, "buyer")] {
                d.seq_op(crate::roles::OP_SET_ROLE, a(&[("member", t(&hex::encode(who))), ("role", t(role))])).expect("a role at mint");
            }
            d
        }
        fn seq_op(&mut self, op: u32, args: Args) -> Result<bool, DeltaRejection> {
            let d = sequenced_delta(ObjectKind::Transaction.type_id() as u32, op, args, 0, self.seq, self.prev);
            let id = d.id();
            let r = self.c.deliver(d, SETTLER_ID);
            if r.is_ok() {
                self.prev = id;
                self.seq += 1;
            }
            r
        }
        fn op(&mut self, op: u32, who: MemberId, gen: u64, mut args: Args) -> &mut Self {
            args.insert("gen".into(), i(gen as i64));
            self.c.deliver(build_delta(ObjectKind::Transaction, op, args, 0, Some(gen)), who).expect("delivered");
            self
        }
        fn terms(&mut self, gen: u64, at: i64) -> &mut Self {
            self.op(OP_SET_TERMS, SELLER_ID, gen, a(&[("qty", i(1)), ("amount", i(1500)), ("currency", t("GBP")), ("delivery", t("meet")), ("descriptor", t("a drill")), ("at", i(at))]))
        }
        fn accept(&mut self, gen: u64, terms: u64, at: i64) -> &mut Self {
            self.op(OP_ACCEPT, BUYER_ID, gen, a(&[("terms", i(terms as i64)), ("at", i(at))]))
        }
        fn state(&self) -> TransactionState {
            self.c.state()
        }
        fn view(&self, now: i64) -> serde_json::Value {
            let mut v = view(&self.c.state());
            at_read(&mut v, now);
            v
        }
    }
    fn whys(st: &TransactionState) -> Vec<String> {
        st.nonconforming.iter().map(|n| format!("{}: {}", n.op, n.why)).collect()
    }

    /// The machine is the ICD's: its states, and a transition for every op but the roles.
    #[test]
    fn the_machine_is_the_icds() {
        let m = machine();
        assert_eq!(m.initial, "open");
        let mut states = m.states.clone();
        states.sort();
        assert_eq!(states, ["accepted", "cancelled", "completed", "disputed", "offered", "open", "resolved"]);
        for d in OPS.iter().filter(|d| !crate::roles::is_role_op(d.op_id) && !crate::parent::is_parent_op(d.op_id)) {
            assert!(m.transitions.iter().any(|t| t.op.as_deref() == Some(d.name)), "{} has a transition", d.name);
        }
        for t in &m.transitions {
            if let Some(op) = &t.op {
                assert!(OPS.iter().any(|d| d.name == op), "{op} is an op of this kind");
            }
            for s in t.from.iter().chain(t.to.iter()) {
                assert!(m.states.contains(s), "{s} is a state");
            }
        }
        assert_eq!(m.confirm_within, 2 * DAY);
        for t in m.transitions.iter().filter(|t| t.op.is_none()) {
            assert!(when(&TransactionState::default(), t.when.as_deref()).is_some(), "this build reads the condition {:?}", t.when);
        }
        for r in m.args.values().flatten() {
            if let Some(p) = &r.pattern {
                assert!(pattern(p, "").is_some(), "this build reads the pattern {p}");
            }
        }
        for d in OPS {
            assert!(m.args.contains_key(d.name) || crate::roles::is_role_op(d.op_id) || crate::parent::is_parent_op(d.op_id), "{} has its args", d.name);
        }
    }

    /// Offered, accepted, both attest: completed.
    #[test]
    fn a_sale_completes_when_both_attest() {
        let mut d = Deal::new();
        d.terms(1, T0).accept(2, 1, T0 + 1000);
        assert_eq!(d.state().state, "accepted");
        d.op(OP_CONFIRM_RECEIPT, BUYER_ID, 3, a(&[("at", i(T0 + 2000))]));
        assert_eq!(d.state().state, "accepted", "one attestation is not both");
        d.op(OP_CONFIRM_SALE, SELLER_ID, 4, a(&[("at", i(T0 + 3000))]));
        let v = d.view(T0 + 4000);
        assert_eq!(v["state"], "completed");
        assert_eq!(v["completedBy"], "both");
        assert_eq!(v["terms"]["amount"], 1500);
        assert!(d.state().nonconforming.is_empty(), "{:?}", whys(&d.state()));
    }

    /// The settler may not author the deal, nor a party the other's part: refused at fold,
    /// so nothing of it stands.
    #[test]
    fn a_party_writes_only_its_own_part() {
        let mut d = Deal::new();
        d.op(OP_SET_TERMS, SETTLER_ID, 1, a(&[("qty", i(1)), ("amount", i(1)), ("currency", t("GBP")), ("delivery", t("meet")), ("at", i(T0))]));
        d.op(OP_SET_TERMS, BUYER_ID, 2, a(&[("qty", i(1)), ("amount", i(1)), ("currency", t("GBP")), ("delivery", t("meet")), ("at", i(T0))]));
        assert_eq!(d.state().state, "open");
        assert!(d.state().terms.is_none());
        d.terms(3, T0);
        d.op(OP_ACCEPT, SELLER_ID, 4, a(&[("terms", i(3)), ("at", i(T0 + 1))]));
        assert_eq!(d.state().state, "offered", "the seller cannot accept its own terms");
        assert_eq!(TransactionType::role_of(&d.state(), &SETTLER_ID), Some(GroupRole::Settler));
    }

    /// Roles are fixed (T-2): one a member, the settler the owner, none cleared, none once
    /// an op other than a role has folded.
    #[test]
    fn roles_are_fixed_for_the_lifecycle() {
        let mut d = Deal::new();
        let set = |who: MemberId, role: &str| a(&[("member", t(&hex::encode(who))), ("role", t(role))]);
        let _ = d.seq_op(crate::roles::OP_SET_ROLE, set(BUYER_ID, "seller"));
        assert_eq!(d.state().roles[&BUYER_ID], GroupRole::Buyer, "one role a member");
        let _ = d.seq_op(crate::roles::OP_CLEAR_ROLE, a(&[("member", t(&hex::encode(BUYER_ID)))]));
        assert_eq!(d.state().roles.len(), 3, "a deal's role is never cleared");
        assert!(crate::roles::reduce_fixed_roles(&mut BTreeMap::new(), &Op { op_id: crate::roles::OP_SET_ROLE, args: &set(SELLER_ID, "settler"), author: &SETTLER_ID, pos: None, ctx: &crate::object::ReduceContext { members: &[SETTLER_ID, SELLER_ID], owner: SETTLER_ID, epoch: 0 } }, false).is_err(), "the settler is the owner");
        assert!(crate::roles::reduce_fixed_roles(&mut BTreeMap::new(), &Op { op_id: crate::roles::OP_SET_ROLE, args: &set(SELLER_ID, "seller"), author: &SETTLER_ID, pos: None, ctx: &crate::object::ReduceContext { members: &[SETTLER_ID, SELLER_ID], owner: SETTLER_ID, epoch: 0 } }, true).is_err(), "not once the deal has begun");
        let begun = TransactionState { began: true, ..d.state() };
        assert!(TransactionType::refuses(&begun, crate::roles::OP_SET_ROLE, &set(SELLER_ID, "seller"), &SETTLER_ID).is_some(), "refused at write once begun");
    }

    /// An op its state does not allow is inert and named.
    #[test]
    fn an_op_out_of_its_state_is_inert_and_named() {
        let mut d = Deal::new();
        d.op(OP_CONFIRM_RECEIPT, BUYER_ID, 1, a(&[("at", i(T0))]));
        let st = d.state();
        assert_eq!(st.state, "open");
        assert!(st.receipt.is_none());
        assert_eq!(whys(&st).len(), 1, "{:?}", whys(&st));
        assert!(whys(&st)[0].starts_with("transaction.confirmReceipt:"));
        assert!(TransactionType::refuses(&st, OP_CONFIRM_RECEIPT, &a(&[("at", i(T0))]), &BUYER_ID).is_some(), "refused at write too");
    }

    /// accept names the standing terms, in time, and not before them.
    #[test]
    fn an_accept_names_the_standing_terms_in_time() {
        let mut d = Deal::new();
        d.op(OP_SET_TERMS, SELLER_ID, 1, a(&[("qty", i(1)), ("amount", i(900)), ("currency", t("EUR")), ("delivery", t("pickup")), ("acceptBy", i(T0 + DAY)), ("at", i(T0))]));
        d.terms(2, T0 + 10);
        d.accept(3, 1, T0 + 20);
        assert_eq!(d.state().state, "offered", "gen 1 no longer stands");
        d.accept(4, 2, T0 + 5);
        assert_eq!(d.state().state, "offered", "before the terms it answers");
        d.accept(5, 2, T0 + 30);
        assert_eq!(d.state().state, "accepted");
        assert_eq!(d.state().terms.as_ref().map(|t| t.gen), Some(2));
        d.terms(6, T0 + 40);
        assert_eq!(d.state().terms.as_ref().map(|t| t.gen), Some(2), "frozen once accepted");

        let mut late = Deal::new();
        late.op(OP_SET_TERMS, SELLER_ID, 1, a(&[("qty", i(1)), ("amount", i(900)), ("currency", t("EUR")), ("delivery", t("pickup")), ("acceptBy", i(T0 + DAY)), ("at", i(T0))]));
        late.accept(2, 1, T0 + 2 * DAY);
        assert_eq!(late.state().state, "offered", "after acceptBy");
    }

    /// One attestation and silence: completed at read time once confirmWithin has passed.
    #[test]
    fn silence_completes_once_confirm_within_has_passed() {
        let mut d = Deal::new();
        d.terms(1, T0).accept(2, 1, T0 + 1);
        d.op(OP_CONFIRM_SALE, SELLER_ID, 3, a(&[("at", i(T0 + 100))]));
        assert_eq!(d.view(T0 + 100 + 2 * DAY - 1)["state"], "accepted");
        let v = d.view(T0 + 100 + 2 * DAY);
        assert_eq!(v["state"], "completed");
        assert_eq!(v["completedBy"], "seller");
        assert_eq!(d.state().state, "accepted", "the fold holds accepted");
    }

    /// A dispute is either party's, within the window; only the settler resolves it.
    #[test]
    fn a_dispute_is_the_settlers_to_resolve() {
        let mut d = Deal::new();
        d.terms(1, T0).accept(2, 1, T0 + 1);
        d.op(OP_CONFIRM_SALE, SELLER_ID, 3, a(&[("at", i(T0 + 100))]));
        d.op(OP_DISPUTE, BUYER_ID, 4, a(&[("reason", t("never came")), ("at", i(T0 + 100 + 3 * DAY))]));
        assert_eq!(d.state().state, "accepted", "outside confirmWithin");
        d.op(OP_DISPUTE, BUYER_ID, 5, a(&[("reason", t("never came")), ("at", i(T0 + 200))]));
        assert_eq!(d.state().state, "disputed");
        d.op(OP_RESOLVE, SELLER_ID, 6, a(&[("outcome", t("completed")), ("at", i(T0 + 300))]));
        assert_eq!(d.state().state, "disputed", "the seller cannot resolve");
        d.op(OP_RESOLVE, SETTLER_ID, 7, a(&[("outcome", t("cancelled")), ("note", t("no receipt")), ("at", i(T0 + 300))]));
        let v = d.view(T0 + 400);
        assert_eq!(v["state"], "resolved");
        assert_eq!(v["resolution"]["outcome"], "cancelled");
        assert_eq!(v["dispute"]["by"], "buyer");
    }

    /// Ratings: once each, after completion, the latest by gen counting.
    #[test]
    fn ratings_count_after_completion_once_each() {
        let mut d = Deal::new();
        d.terms(1, T0).accept(2, 1, T0 + 1);
        d.op(OP_RATE, BUYER_ID, 3, a(&[("stars", i(5)), ("at", i(T0 + 2))]));
        assert!(d.state().ratings.is_empty(), "not before completion");
        d.op(OP_CONFIRM_RECEIPT, BUYER_ID, 4, a(&[("at", i(T0 + 10))])).op(OP_CONFIRM_SALE, SELLER_ID, 5, a(&[("at", i(T0 + 11))]));
        d.op(OP_RATE, BUYER_ID, 6, a(&[("stars", i(4)), ("text", t("fine")), ("at", i(T0 + 20))]));
        d.op(OP_RATE, BUYER_ID, 7, a(&[("stars", i(5)), ("at", i(T0 + 21))]));
        d.op(OP_RATE, SELLER_ID, 8, a(&[("stars", i(9)), ("at", i(T0 + 22))]));
        let v = d.view(T0 + 30);
        assert_eq!(v["ratings"]["buyer"]["stars"], 5);
        assert!(v["ratings"]["seller"].is_null(), "9 stars is not 1 to 5");
    }

    /// fraudSignal is declared, not implemented: refused at write, inert at fold.
    #[test]
    fn fraud_signal_is_a_stub() {
        let mut d = Deal::new();
        let args = a(&[("signal", t("velocity")), ("at", i(T0))]);
        assert!(TransactionType::refuses(&d.state(), OP_FRAUD_SIGNAL, &args, &SETTLER_ID).is_some());
        d.op(OP_FRAUD_SIGNAL, SETTLER_ID, 1, args);
        assert_eq!(d.state().state, "open");
        assert_eq!(whys(&d.state()).len(), 1);
    }

    /// Cancel is the one exit before acceptance; after it, only a dispute ends the deal.
    #[test]
    fn cancel_is_before_acceptance_only() {
        let mut d = Deal::new();
        d.terms(1, T0);
        d.op(OP_CANCEL, BUYER_ID, 2, a(&[("at", i(T0 + 1))]));
        assert_eq!(d.state().state, "cancelled");
        let mut e = Deal::new();
        e.terms(1, T0).accept(2, 1, T0 + 1);
        e.op(OP_CANCEL, SELLER_ID, 3, a(&[("at", i(T0 + 2))]));
        assert_eq!(e.state().state, "accepted");
    }

    /// A Site's sale: setSite once, before any op but the roles, and the terms sell its Thing.
    #[test]
    fn a_sites_sale_sells_its_listed_thing() {
        let site = "5".repeat(64);
        let thing = "6".repeat(64);
        let mut d = Deal::new();
        d.seq_op(OP_SET_SITE, a(&[("site", t(&site)), ("thingId", t(&thing)), ("at", i(T0))])).expect("setSite");
        let _ = d.seq_op(OP_SET_SITE, a(&[("site", t(&site)), ("thingId", t(&"8".repeat(64))), ("at", i(T0))]));
        assert_eq!(d.state().site.as_ref().map(|s| s.thing_id.clone()), Some(thing.clone()), "once");
        d.op(OP_SET_TERMS, SELLER_ID, 1, a(&[("thing", t(&"7".repeat(64))), ("qty", i(1)), ("amount", i(1)), ("currency", t("KRW")), ("delivery", t("meet")), ("at", i(T0))]));
        assert_eq!(d.state().state, "open", "another Thing");
        d.op(OP_SET_TERMS, SELLER_ID, 2, a(&[("thing", t(&thing)), ("qty", i(1)), ("amount", i(1)), ("currency", t("KRW")), ("delivery", t("meet")), ("at", i(T0))]));
        assert_eq!(d.state().state, "offered");
        assert_eq!(d.state().roles.len(), 3);
    }

    /// Args are the ICD's: a currency of three capitals, a delivery from its vocabulary, a
    /// descriptor within its maxLength; refused at fold.
    #[test]
    fn terms_args_are_the_icds() {
        for bad in [
            a(&[("qty", i(1)), ("amount", i(1)), ("currency", t("gbp")), ("delivery", t("meet")), ("at", i(T0))]),
            a(&[("qty", i(1)), ("amount", i(1)), ("currency", t("GBP")), ("delivery", t("drone")), ("at", i(T0))]),
            a(&[("qty", i(1)), ("amount", i(1)), ("currency", t("GBP")), ("delivery", t("meet")), ("descriptor", t(&"x".repeat(301))), ("at", i(T0))]),
            a(&[("qty", i(0)), ("amount", i(1)), ("currency", t("GBP")), ("delivery", t("meet")), ("at", i(T0))]),
            a(&[("qty", i(1)), ("amount", i(-1)), ("currency", t("GBP")), ("delivery", t("meet")), ("at", i(T0))]),
        ] {
            let mut d = Deal::new();
            d.op(OP_SET_TERMS, SELLER_ID, 1, bad.clone());
            assert!(d.state().terms.is_none(), "{bad:?}");
        }
    }
}
