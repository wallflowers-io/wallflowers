//! Project (#22) — the Work-tab object, the second concrete `ObjectType`.
//!
//! This is the model locked for the Work tab, expressed purely on the two fold
//! arms the generic `Coordinator<T>` proves:
//!
//! - **Timeline facet** — a vertical Gantt as a projection over ONE node table.
//!   Item identity + structure are OWNER-SEQUENCED (`addTimelineItem`,
//!   `setItemSchedule`, `setAssignee`, `add/removeDependency`, `suppressItem`);
//!   live per-member progress + free-text fields are ANY-MEMBER COMMUTATIVE, LWW
//!   by `(gen, author)` (`setItemProgress`, `setItemField`). A `blocked` status is
//!   DERIVED from an unresolved incoming dependency edge — never stored — so there
//!   is no free-floating flag to fall out of sync.
//! - **Deliverables** — a deliverable IS a child Project, referenced by an
//!   owner-sequenced `subscribe(kind = project)` edge pinned to a timeline item via
//!   `originItem`. The edge is EDGE + DISCLOSURE ONLY, never a copy of the child.
//!   `touchSubscription` advances a per-member last-seen cursor (the collapsed-view
//!   freshness). The season's "N-piece body" is `piece_count()` — a DERIVED count
//!   of child-Project edges, never a stored scalar.
//!
//! The reducer is PURE: it reads only `op.ctx` (members/owner/epoch) and returns a
//! typed [`DeltaRejection`] instead of mutating on any error. A reduce-time reject
//! is skipped by the fold (the op is inert), never swallowed silently — the loud
//! integrity checks (owner/chain/fork/member/dedup) already ran at deliver time.
//!
//! NOTE: the FFI catalog (`pacific-ffi/src/delta.rs`) still enumerates the OLD Project
//! op set (createTask/…). Aligning it to THIS op table (and regenerating the Swift
//! bindings) is the sibling follow-up — the Rust reducer is the source of truth
//! ("the Rust wins", per that module's own header). No Swift call-site consumes the
//! Project constructors today, so that alignment breaks nothing.

use std::collections::{BTreeMap, BTreeSet};

use crate::coordinator::Args;
use crate::object::{
    Authority, Commutativity, DeltaRejection, MemberId, ObjectKind, ObjectType, Op, OpDecl,
};
use crate::object_args::{opt_int, opt_text, req_gen, req_int, req_text};

// ---- op-ids (the single source of truth the FFI catalog will align to) -------

pub const OP_CONFIGURE: u32 = 0; // owner / sequenced
pub const OP_ADD_TIMELINE_ITEM: u32 = 1; // owner / sequenced
pub const OP_SUPPRESS_ITEM: u32 = 2; // owner / sequenced
pub const OP_SET_ITEM_SCHEDULE: u32 = 3; // owner / sequenced
pub const OP_SET_ASSIGNEE: u32 = 4; // owner / sequenced
pub const OP_ADD_DEPENDENCY: u32 = 5; // owner / sequenced
pub const OP_REMOVE_DEPENDENCY: u32 = 6; // owner / sequenced
pub const OP_SUBSCRIBE: u32 = 7; // owner / sequenced
pub const OP_UNSUBSCRIBE: u32 = 8; // owner / sequenced
pub const OP_SET_ITEM_PROGRESS: u32 = 9; // any-member / commutative
pub const OP_SET_ITEM_FIELD: u32 = 10; // any-member / commutative
pub const OP_TOUCH_SUBSCRIPTION: u32 = 11; // any-member / commutative
                                           // ---- objectives — the human-authored spine (Tab 1). owner-sequenced. ----------
pub const OP_SET_OBJECTIVE: u32 = 12; // owner / sequenced — headline + goal
pub const OP_SET_ROLE: u32 = 13; // owner / sequenced — a participant's role
pub const OP_ADD_STAKEHOLDER: u32 = 14; // owner / sequenced — external stakeholder (upsert)
pub const OP_REMOVE_STAKEHOLDER: u32 = 15; // owner / sequenced
pub const OP_ADD_LOCATION: u32 = 16; // owner / sequenced — key location (upsert)
pub const OP_REMOVE_LOCATION: u32 = 17; // owner / sequenced
pub const OP_SET_KPI: u32 = 18; // owner / sequenced — a KPI (upsert)
pub const OP_REMOVE_KPI: u32 = 19; // owner / sequenced

// ---- typed vocab (unknown values self-reject as MalformedArgs) ---------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ProjectStatus {
    #[default]
    Active,
    Archived,
    Paused,
}
impl ProjectStatus {
    fn parse(s: &str) -> Result<Self, DeltaRejection> {
        Ok(match s {
            "active" => Self::Active,
            "archived" => Self::Archived,
            "paused" => Self::Paused,
            _ => return Err(DeltaRejection::MalformedArgs),
        })
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
            Self::Paused => "paused",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Action, // a unit of work
    Event,  // an externally-driven marker/window (a show, a launch)
    Gate,   // a promoted precondition marker
}
impl ItemKind {
    fn parse(s: &str) -> Result<Self, DeltaRejection> {
        Ok(match s {
            "action" => Self::Action,
            "event" => Self::Event,
            "gate" => Self::Gate,
            _ => return Err(DeltaRejection::MalformedArgs),
        })
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Action => "action",
            Self::Event => "event",
            Self::Gate => "gate",
        }
    }
}

/// Stored progress vocab — NOTE `blocked` is deliberately absent: it is DERIVED
/// from dependency edges, never a stored status (no second source of truth).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemStatus {
    Open,
    InProgress,
    Done,
    Cancelled,
}
impl ItemStatus {
    fn parse(s: &str) -> Result<Self, DeltaRejection> {
        Ok(match s {
            "open" => Self::Open,
            "in_progress" => Self::InProgress,
            "done" => Self::Done,
            "cancelled" => Self::Cancelled,
            _ => return Err(DeltaRejection::MalformedArgs),
        })
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepKind {
    Blocks,
    Gates,
}
impl DepKind {
    fn parse(s: &str) -> Result<Self, DeltaRejection> {
        Ok(match s {
            "blocks" => Self::Blocks,
            "gates" => Self::Gates,
            _ => return Err(DeltaRejection::MalformedArgs),
        })
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Blocks => "blocks",
            Self::Gates => "gates",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubKind {
    Topic,
    System,
    Forum,
    Project,
}
impl SubKind {
    fn parse(s: &str) -> Result<Self, DeltaRejection> {
        Ok(match s {
            "topic" => Self::Topic,
            "system" => Self::System,
            "forum" => Self::Forum,
            "project" => Self::Project,
            _ => return Err(DeltaRejection::MalformedArgs),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disclosure {
    Existence,
    Summary,
    Full,
}
impl Disclosure {
    fn parse(s: &str) -> Result<Self, DeltaRejection> {
        Ok(match s {
            "existence" => Self::Existence,
            "summary" => Self::Summary,
            "full" => Self::Full,
            _ => return Err(DeltaRejection::MalformedArgs),
        })
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Existence => "existence",
            Self::Summary => "summary",
            Self::Full => "full",
        }
    }
}

// ---- state -------------------------------------------------------------------

/// A participant's role in the Project — bounds how far their CLA may act on their
/// behalf (Brick 3); advisory to the UI today.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Role {
    Viewer,
    #[default]
    Contributor,
    Maintainer,
    Owner,
}
impl Role {
    fn parse(s: &str) -> Result<Self, DeltaRejection> {
        Ok(match s {
            "viewer" => Self::Viewer,
            "contributor" => Self::Contributor,
            "maintainer" => Self::Maintainer,
            "owner" => Self::Owner,
            _ => return Err(DeltaRejection::MalformedArgs),
        })
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Contributor => "contributor",
            Self::Maintainer => "maintainer",
            Self::Owner => "owner",
        }
    }
}

/// An external stakeholder — a party the project touches who is NOT a member (no
/// device, no CLA). Gets its own timeline swimlane (a supplier's lead time).
#[derive(Clone, Debug, PartialEq)]
pub struct ExternalStakeholder {
    pub name: String,
    pub note: String,
}

/// A KPI the objective is measured against.
#[derive(Clone, Debug, PartialEq)]
pub struct Kpi {
    pub label: String,
    pub target: String,
}

/// One timeline node (owner-sequenced structure). Its live status/fields live in
/// the commutative cells, not here.
#[derive(Clone, Debug, PartialEq)]
pub struct TimelineItem {
    pub kind: ItemKind,
    pub title: String,
    pub suppressed: bool,
    pub at: Option<i64>, // epoch-seconds, author-captured (no clock in reduce)
    pub end_date: Option<i64>, // window end, if any
    pub assignees: BTreeSet<MemberId>,
}

/// A typed dependency edge (owner-sequenced structure). `from` must be done for
/// `to` to be unblocked.
#[derive(Clone, Debug, PartialEq)]
pub struct Dependency {
    pub from: String,
    pub to: String,
    pub kind: DepKind,
}

/// A cross-object edge: EDGE + disclosure only, never a copy of the target.
#[derive(Clone, Debug, PartialEq)]
pub struct Subscription {
    pub target: String, // the referenced object id (a child Project id for deliverables)
    pub kind: SubKind,
    pub disclosure: Disclosure,
    pub origin_item: Option<String>, // the timeline item this edge hangs off, if pinned
}

#[derive(Clone, Debug, Default)]
pub struct ProjectState {
    /// THE PARTS THIS OBJECT IS MADE OF, keyed by the part's object id — its comments
    /// section is one, a real Forum GroupObject with its own roster and not a field
    /// on this one. See `crate::parts`.
    pub parts: std::collections::BTreeMap<String, crate::object::PartRef>,
    pub title: String,
    pub status: ProjectStatus,

    // objectives — the human-authored spine (Tab 1), owner-sequenced
    pub headline: String,
    pub goal: String,
    pub participants: BTreeMap<MemberId, Role>,
    pub external_stakeholders: BTreeMap<String, ExternalStakeholder>,
    pub key_locations: BTreeMap<String, String>, // id -> name
    pub kpis: BTreeMap<String, Kpi>,

    // timeline — owner-sequenced structure
    pub items: BTreeMap<String, TimelineItem>,
    pub deps: BTreeMap<String, Dependency>,

    // timeline — any-member commutative contributions, LWW by (gen, author)
    pub item_status: BTreeMap<String, BTreeMap<MemberId, (u64, ItemStatus)>>,
    pub item_fields: BTreeMap<String, BTreeMap<String, BTreeMap<MemberId, (u64, String)>>>,

    // deliverables / cross-object edges — owner-sequenced, with a commutative cursor
    pub subscriptions: BTreeMap<String, Subscription>,
    pub subscription_seen: BTreeMap<String, BTreeMap<MemberId, (u64, i64)>>,
    /// What this object is a part of, and in what role (`base.setParent`). `None`
    /// until a parent names it as a part.
    pub parent: Option<crate::parent::ParentRef>,
}

// ---- derived-never-stored projections ----------------------------------------

/// LWW winner of a `{author: (gen, value)}` cell, ranked by `(gen, author)` only
/// (value excluded from the key) so it matches the coordinator's canonical fold.
/// Author keys are unique within a cell, so `(gen, author)` never ties.
fn lww_winner<V: Clone>(cell: &BTreeMap<MemberId, (u64, V)>) -> Option<V> {
    let mut best: Option<(u64, MemberId)> = None;
    let mut winner: Option<&V> = None;
    for (author, (gen, val)) in cell {
        let key = (*gen, *author);
        if best.map_or(true, |b| key > b) {
            best = Some(key);
            winner = Some(val);
        }
    }
    winner.cloned()
}

/// The read-side view of one timeline item (honest-absent: status/fields absent
/// until contributed, never zero-filled).
#[derive(Clone, Debug, PartialEq)]
pub struct ItemView {
    pub id: String,
    pub kind: ItemKind,
    pub title: String,
    pub at: Option<i64>,
    pub end_date: Option<i64>,
    pub assignees: Vec<MemberId>,
    pub status: Option<ItemStatus>,
    pub blocked: bool,
    pub fields: BTreeMap<String, String>,
}

/// The collapsed-view row for one deliverable (a child-Project edge).
#[derive(Clone, Debug, PartialEq)]
pub struct DeliverableView {
    pub sub_id: String,
    pub target: String,
    pub disclosure: Disclosure,
    pub origin_item: Option<String>,
    pub last_seen: Option<i64>,
}

impl ProjectState {
    /// The LWW-resolved progress of an item, or `None` until first contributed.
    pub fn resolved_status(&self, item_id: &str) -> Option<ItemStatus> {
        self.item_status.get(item_id).and_then(lww_winner)
    }

    /// DERIVED: an item is blocked iff a live incoming dependency edge's source is
    /// not `Done`. A suppressed/absent source does not block.
    pub fn blocked(&self, item_id: &str) -> bool {
        self.deps.values().any(|d| {
            if d.to != item_id {
                return false;
            }
            match self.items.get(&d.from) {
                Some(src) if !src.suppressed => {
                    self.resolved_status(&d.from) != Some(ItemStatus::Done)
                }
                _ => false,
            }
        })
    }

    /// One item's compiled view, or `None` if the item is absent or suppressed.
    pub fn item_view(&self, item_id: &str) -> Option<ItemView> {
        let item = self.items.get(item_id)?;
        if item.suppressed {
            return None;
        }
        let fields = self
            .item_fields
            .get(item_id)
            .map(|cells| {
                cells
                    .iter()
                    .filter_map(|(f, cell)| lww_winner(cell).map(|v| (f.clone(), v)))
                    .collect()
            })
            .unwrap_or_default();
        Some(ItemView {
            id: item_id.to_string(),
            kind: item.kind,
            title: item.title.clone(),
            at: item.at,
            end_date: item.end_date,
            assignees: item.assignees.iter().copied().collect(),
            status: self.resolved_status(item_id),
            blocked: self.blocked(item_id),
            fields,
        })
    }

    /// The live Gantt: every non-suppressed item, ordered by schedule then id
    /// (unscheduled items sort last). The regenerated geometry, nothing stored.
    pub fn board(&self) -> Vec<ItemView> {
        let mut views: Vec<ItemView> = self
            .items
            .keys()
            .filter_map(|id| self.item_view(id))
            .collect();
        views.sort_by(|a, b| {
            (a.at.unwrap_or(i64::MAX), &a.id).cmp(&(b.at.unwrap_or(i64::MAX), &b.id))
        });
        views
    }

    /// LWW-resolved last-seen cursor for a subscription edge, or `None` until
    /// first touched.
    pub fn subscription_cursor(&self, sub_id: &str) -> Option<i64> {
        self.subscription_seen.get(sub_id).and_then(lww_winner)
    }

    /// The deliverables (child-Project edges) shown collapsed under the timeline.
    pub fn deliverables(&self) -> Vec<DeliverableView> {
        self.subscriptions
            .iter()
            .filter(|(_, s)| s.kind == SubKind::Project)
            .map(|(sub_id, s)| DeliverableView {
                sub_id: sub_id.clone(),
                target: s.target.clone(),
                disclosure: s.disclosure,
                origin_item: s.origin_item.clone(),
                last_seen: self.subscription_cursor(sub_id),
            })
            .collect()
    }

    /// The "N-piece body" — a DERIVED count of child-Project deliverables, never a
    /// stored scalar. Replaces the Aries mockup's `bodyCount: Int`.
    pub fn piece_count(&self) -> usize {
        self.subscriptions
            .values()
            .filter(|s| s.kind == SubKind::Project)
            .count()
    }
}

// ---- typed arg extraction (ill-typed/missing => MalformedArgs) ----------------

fn req_member(args: &Args, key: &str) -> Result<MemberId, DeltaRejection> {
    let s = req_text(args, key)?;
    let bytes = hex::decode(s).map_err(|_| DeltaRejection::MalformedArgs)?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| DeltaRejection::MalformedArgs)
}

/// Does `start` reach `target` following `from -> to` edges? (cycle guard).
fn reaches(deps: &BTreeMap<String, Dependency>, start: &str, target: &str) -> bool {
    let mut stack = vec![start.to_string()];
    let mut seen = BTreeSet::new();
    while let Some(node) = stack.pop() {
        if node == target {
            return true;
        }
        if !seen.insert(node.clone()) {
            continue;
        }
        for d in deps.values() {
            if d.from == node {
                stack.push(d.to.clone());
            }
        }
    }
    false
}

// ---- the op table + the ObjectType impl --------------------------------------

static PROJECT_OPS: &[OpDecl] = &[
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
    OpDecl {
        op_id: OP_CONFIGURE,
        name: "project.configure",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_ADD_TIMELINE_ITEM,
        name: "project.addTimelineItem",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SUPPRESS_ITEM,
        name: "project.suppressItem",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SET_ITEM_SCHEDULE,
        name: "project.setItemSchedule",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SET_ASSIGNEE,
        name: "project.setAssignee",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_ADD_DEPENDENCY,
        name: "project.addDependency",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_REMOVE_DEPENDENCY,
        name: "project.removeDependency",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SUBSCRIBE,
        name: "project.subscribe",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_UNSUBSCRIBE,
        name: "project.unsubscribe",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SET_ITEM_PROGRESS,
        name: "project.setItemProgress",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_SET_ITEM_FIELD,
        name: "project.setItemField",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_TOUCH_SUBSCRIPTION,
        name: "project.touchSubscription",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_SET_OBJECTIVE,
        name: "project.setObjective",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SET_ROLE,
        name: "project.setRole",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_ADD_STAKEHOLDER,
        name: "project.addStakeholder",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_REMOVE_STAKEHOLDER,
        name: "project.removeStakeholder",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_ADD_LOCATION,
        name: "project.addLocation",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_REMOVE_LOCATION,
        name: "project.removeLocation",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_SET_KPI,
        name: "project.setKpi",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_REMOVE_KPI,
        name: "project.removeKpi",
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

/// The Project object type (#22) — the Work-tab model folded through the trait.
pub struct ProjectType;

impl ObjectType for ProjectType {
    const KIND: ObjectKind = ObjectKind::Project;
    type State = ProjectState;

    fn ops() -> &'static [OpDecl] {
        PROJECT_OPS
    }

    fn reduce(state: &mut ProjectState, op: &Op<'_>) -> Result<(), DeltaRejection> {
        if crate::parent::is_parent_op(op.op_id) {
            return crate::parent::reduce_parent(&mut state.parent, op);
        }
        if crate::parts::is_part_op(op.op_id) {
            return crate::parts::reduce_parts(&mut state.parts, op);
        }
        let args = op.args;
        match op.op_id {
            // ---- owner-sequenced structure ----
            OP_CONFIGURE => {
                state.title = req_text(args, "title")?.to_string();
                state.status = ProjectStatus::parse(req_text(args, "status")?)?;
                Ok(())
            }
            OP_ADD_TIMELINE_ITEM => {
                let item_id = req_text(args, "itemId")?.to_string();
                if state.items.contains_key(&item_id) {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                let kind = ItemKind::parse(req_text(args, "kind")?)?;
                let title = req_text(args, "title")?.to_string();
                state.items.insert(
                    item_id,
                    TimelineItem {
                        kind,
                        title,
                        suppressed: false,
                        at: None,
                        end_date: None,
                        assignees: BTreeSet::new(),
                    },
                );
                Ok(())
            }
            OP_SUPPRESS_ITEM => {
                let item_id = req_text(args, "itemId")?;
                let suppressed = req_int(args, "suppressed")? != 0;
                let item = state
                    .items
                    .get_mut(item_id)
                    .ok_or(DeltaRejection::PreconditionFailed)?;
                item.suppressed = suppressed;
                Ok(())
            }
            OP_SET_ITEM_SCHEDULE => {
                let item_id = req_text(args, "itemId")?;
                let at = req_int(args, "at")?;
                let end_date = opt_int(args, "endDate");
                let item = state
                    .items
                    .get_mut(item_id)
                    .ok_or(DeltaRejection::PreconditionFailed)?;
                item.at = Some(at);
                item.end_date = end_date;
                Ok(())
            }
            OP_SET_ASSIGNEE => {
                let item_id = req_text(args, "itemId")?;
                let member = req_member(args, "member")?;
                let assigned = req_int(args, "assigned")? != 0;
                // a role/assignment needs a real member (parent-child, like setRole).
                if !op.ctx.is_member(&member) {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                let item = state
                    .items
                    .get_mut(item_id)
                    .ok_or(DeltaRejection::PreconditionFailed)?;
                if assigned {
                    item.assignees.insert(member);
                } else {
                    item.assignees.remove(&member);
                }
                Ok(())
            }
            OP_ADD_DEPENDENCY => {
                let edge_id = req_text(args, "edgeId")?.to_string();
                if state.deps.contains_key(&edge_id) {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                let from = req_text(args, "from")?.to_string();
                let to = req_text(args, "to")?.to_string();
                let kind = DepKind::parse(req_text(args, "kind")?)?;
                if from == to {
                    return Err(DeltaRejection::PreconditionFailed); // no self-dependency
                }
                if !state.items.contains_key(&from) || !state.items.contains_key(&to) {
                    return Err(DeltaRejection::PreconditionFailed); // both endpoints must exist
                }
                // cycle guard: if `to` already reaches `from`, adding from->to loops.
                if reaches(&state.deps, &to, &from) {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                state.deps.insert(edge_id, Dependency { from, to, kind });
                Ok(())
            }
            OP_REMOVE_DEPENDENCY => {
                let edge_id = req_text(args, "edgeId")?;
                if state.deps.remove(edge_id).is_none() {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                Ok(())
            }
            OP_SUBSCRIBE => {
                let sub_id = req_text(args, "subId")?.to_string();
                if state.subscriptions.contains_key(&sub_id) {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                let target = req_text(args, "target")?.to_string();
                let kind = SubKind::parse(req_text(args, "kind")?)?;
                let disclosure = Disclosure::parse(req_text(args, "disclosure")?)?;
                let origin_item = opt_text(args, "originItem");
                if let Some(ref oi) = origin_item {
                    if !state.items.contains_key(oi) {
                        return Err(DeltaRejection::PreconditionFailed); // pin needs a real item
                    }
                }
                state.subscriptions.insert(
                    sub_id,
                    Subscription {
                        target,
                        kind,
                        disclosure,
                        origin_item,
                    },
                );
                Ok(())
            }
            OP_UNSUBSCRIBE => {
                let sub_id = req_text(args, "subId")?;
                if state.subscriptions.remove(sub_id).is_none() {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                state.subscription_seen.remove(sub_id); // a cursor cannot outlive its edge
                Ok(())
            }

            // ---- objectives — the human-authored spine (owner-sequenced) ----
            OP_SET_OBJECTIVE => {
                state.headline = req_text(args, "headline")?.to_string();
                state.goal = req_text(args, "goal")?.to_string();
                Ok(())
            }
            OP_SET_ROLE => {
                let member = req_member(args, "member")?;
                if !op.ctx.is_member(&member) {
                    return Err(DeltaRejection::PreconditionFailed); // a role needs a real member
                }
                let role = Role::parse(req_text(args, "role")?)?;
                state.participants.insert(member, role);
                Ok(())
            }
            OP_ADD_STAKEHOLDER => {
                // upsert (add OR edit) — keyed by a stable id the UI mints.
                let id = req_text(args, "stakeholderId")?.to_string();
                let name = req_text(args, "name")?.to_string();
                let note = opt_text(args, "note").unwrap_or_default();
                state
                    .external_stakeholders
                    .insert(id, ExternalStakeholder { name, note });
                Ok(())
            }
            OP_REMOVE_STAKEHOLDER => {
                let id = req_text(args, "stakeholderId")?;
                if state.external_stakeholders.remove(id).is_none() {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                Ok(())
            }
            OP_ADD_LOCATION => {
                let id = req_text(args, "locationId")?.to_string();
                let name = req_text(args, "name")?.to_string();
                state.key_locations.insert(id, name); // upsert
                Ok(())
            }
            OP_REMOVE_LOCATION => {
                let id = req_text(args, "locationId")?;
                if state.key_locations.remove(id).is_none() {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                Ok(())
            }
            OP_SET_KPI => {
                let id = req_text(args, "kpiId")?.to_string();
                let label = req_text(args, "label")?.to_string();
                let target = req_text(args, "target")?.to_string();
                state.kpis.insert(id, Kpi { label, target }); // upsert
                Ok(())
            }
            OP_REMOVE_KPI => {
                let id = req_text(args, "kpiId")?;
                if state.kpis.remove(id).is_none() {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                Ok(())
            }

            // ---- any-member commutative contributions (LWW by gen, author) ----
            OP_SET_ITEM_PROGRESS => {
                let item_id = req_text(args, "itemId")?;
                let status = ItemStatus::parse(req_text(args, "status")?)?;
                let gen = req_gen(args)?;
                if !state.items.contains_key(item_id) {
                    return Err(DeltaRejection::PreconditionFailed); // parent item must exist
                }
                let cell = state.item_status.entry(item_id.to_string()).or_default();
                if let Some((prev_gen, _)) = cell.get(op.author) {
                    if gen <= *prev_gen {
                        return Err(DeltaRejection::PreconditionFailed); // non-monotonic
                    }
                }
                cell.insert(*op.author, (gen, status));
                Ok(())
            }
            OP_SET_ITEM_FIELD => {
                let item_id = req_text(args, "itemId")?;
                let field = req_text(args, "field")?.to_string();
                let value = req_text(args, "value")?.to_string();
                let gen = req_gen(args)?;
                if !state.items.contains_key(item_id) {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                let cell = state
                    .item_fields
                    .entry(item_id.to_string())
                    .or_default()
                    .entry(field)
                    .or_default();
                if let Some((prev_gen, _)) = cell.get(op.author) {
                    if gen <= *prev_gen {
                        return Err(DeltaRejection::PreconditionFailed);
                    }
                }
                cell.insert(*op.author, (gen, value));
                Ok(())
            }
            OP_TOUCH_SUBSCRIPTION => {
                let sub_id = req_text(args, "subId")?;
                let at = req_int(args, "at")?;
                let gen = req_gen(args)?;
                if !state.subscriptions.contains_key(sub_id) {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                let cell = state
                    .subscription_seen
                    .entry(sub_id.to_string())
                    .or_default();
                if let Some((prev_gen, _)) = cell.get(op.author) {
                    if gen <= *prev_gen {
                        return Err(DeltaRejection::PreconditionFailed);
                    }
                }
                cell.insert(*op.author, (gen, at));
                Ok(())
            }

            _ => Err(DeltaRejection::UnknownType),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator::{sequenced_delta, ArgVal, Coordinator, Delta, GENESIS_PREV};

    const OWNER: MemberId = [7u8; 32];
    const A: MemberId = [0xA1u8; 32];

    fn tid() -> u32 {
        ObjectKind::Project.type_id() as u32
    }
    fn args(pairs: Vec<(&str, ArgVal)>) -> Args {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }
    fn t(s: &str) -> ArgVal {
        ArgVal::Text(s.to_string())
    }
    fn i(n: i64) -> ArgVal {
        ArgVal::Int(n)
    }

    /// Chains owner-sequenced deltas at (epoch 0, seq n) with proper `prev` links.
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
        fn deliver(&mut self, c: &mut Coordinator<ProjectType>, op_id: u32, a: Args) {
            let d = sequenced_delta(tid(), op_id, a, 0, self.seq, self.prev);
            let id = d.id();
            c.deliver(d, OWNER).unwrap();
            self.seq += 1;
            self.prev = id;
        }
    }

    fn comm(c: &mut Coordinator<ProjectType>, author: MemberId, op_id: u32, mut a: Args, gen: u64) {
        a.insert("gen".to_string(), ArgVal::Int(gen as i64));
        let d = Delta {
            type_id: tid(),
            op_id,
            op_version: 1,
            args: a,
            epoch: 0,
            prev: GENESIS_PREV,
            seq: None,
            gen: Some(gen),
            visibility: crate::visibility::Visibility::default(),
        };
        c.deliver(d, author).unwrap();
    }

    fn coord() -> Coordinator<ProjectType> {
        Coordinator::<ProjectType>::new(vec![OWNER, A], OWNER)
    }

    #[test]
    fn all_ops_well_formed() {
        for o in PROJECT_OPS {
            assert!(o.is_well_formed(), "{} violates the spec invariant", o.name);
        }
    }

    #[test]
    fn objectives_spine_folds() {
        let mut c = coord();
        let mut sp = Spine::new();
        sp.deliver(
            &mut c,
            OP_SET_OBJECTIVE,
            args(vec![
                ("headline", t("Ship AW26 on time")),
                ("goal", t("Land the drop by 27 Aug with zero crossed codes")),
            ]),
        );
        sp.deliver(
            &mut c,
            OP_SET_ROLE,
            args(vec![
                ("member", t(&hex::encode(A))),
                ("role", t("maintainer")),
            ]),
        );
        sp.deliver(
            &mut c,
            OP_ADD_STAKEHOLDER,
            args(vec![
                ("stakeholderId", t("s1")),
                ("name", t("Fabric supplier")),
                ("note", t("6-week lead time")),
            ]),
        );
        sp.deliver(
            &mut c,
            OP_ADD_LOCATION,
            args(vec![("locationId", t("l1")), ("name", t("Paris atelier"))]),
        );
        sp.deliver(
            &mut c,
            OP_SET_KPI,
            args(vec![
                ("kpiId", t("k1")),
                ("label", t("Sell-through")),
                ("target", t("70% in 4 weeks")),
            ]),
        );

        let s = c.state();
        assert_eq!(s.headline, "Ship AW26 on time");
        assert!(s.goal.contains("27 Aug"));
        assert_eq!(s.participants.get(&A), Some(&Role::Maintainer));
        assert_eq!(
            s.external_stakeholders.get("s1").unwrap().name,
            "Fabric supplier"
        );
        assert_eq!(
            s.key_locations.get("l1").map(String::as_str),
            Some("Paris atelier")
        );
        assert_eq!(s.kpis.get("k1").unwrap().target, "70% in 4 weeks");

        // upsert (edit) + remove
        sp.deliver(
            &mut c,
            OP_SET_KPI,
            args(vec![
                ("kpiId", t("k1")),
                ("label", t("Sell-through")),
                ("target", t("80% in 4 weeks")),
            ]),
        );
        sp.deliver(
            &mut c,
            OP_REMOVE_STAKEHOLDER,
            args(vec![("stakeholderId", t("s1"))]),
        );
        let s = c.state();
        assert_eq!(s.kpis.get("k1").unwrap().target, "80% in 4 weeks");
        assert!(s.external_stakeholders.is_empty());
    }

    #[test]
    fn timeline_items_and_honest_absent_status() {
        let mut c = coord();
        let mut sp = Spine::new();
        sp.deliver(
            &mut c,
            OP_CONFIGURE,
            args(vec![("title", t("SS27")), ("status", t("active"))]),
        );
        sp.deliver(
            &mut c,
            OP_ADD_TIMELINE_ITEM,
            args(vec![
                ("itemId", t("i1")),
                ("kind", t("action")),
                ("title", t("Cut samples")),
            ]),
        );
        sp.deliver(
            &mut c,
            OP_ADD_TIMELINE_ITEM,
            args(vec![
                ("itemId", t("i2")),
                ("kind", t("event")),
                ("title", t("Paris show")),
            ]),
        );
        comm(
            &mut c,
            OWNER,
            OP_SET_ITEM_PROGRESS,
            args(vec![("itemId", t("i1")), ("status", t("in_progress"))]),
            0,
        );

        let s = c.state();
        assert_eq!(s.title, "SS27");
        assert_eq!(s.board().len(), 2);
        assert_eq!(
            s.item_view("i1").unwrap().status,
            Some(ItemStatus::InProgress)
        );
        // honest-absent: an item with no contributed progress has status None.
        assert_eq!(s.item_view("i2").unwrap().status, None);
    }

    #[test]
    fn suppress_omits_item_from_board() {
        let mut c = coord();
        let mut sp = Spine::new();
        sp.deliver(
            &mut c,
            OP_ADD_TIMELINE_ITEM,
            args(vec![
                ("itemId", t("i1")),
                ("kind", t("action")),
                ("title", t("x")),
            ]),
        );
        sp.deliver(
            &mut c,
            OP_SUPPRESS_ITEM,
            args(vec![("itemId", t("i1")), ("suppressed", i(1))]),
        );
        let s = c.state();
        assert!(s.board().is_empty());
        assert_eq!(s.item_view("i1"), None);
    }

    #[test]
    fn blocked_is_derived_from_dependencies() {
        let mut c = coord();
        let mut sp = Spine::new();
        sp.deliver(
            &mut c,
            OP_ADD_TIMELINE_ITEM,
            args(vec![
                ("itemId", t("i1")),
                ("kind", t("action")),
                ("title", t("upstream")),
            ]),
        );
        sp.deliver(
            &mut c,
            OP_ADD_TIMELINE_ITEM,
            args(vec![
                ("itemId", t("i2")),
                ("kind", t("action")),
                ("title", t("downstream")),
            ]),
        );
        sp.deliver(
            &mut c,
            OP_ADD_DEPENDENCY,
            args(vec![
                ("edgeId", t("e1")),
                ("from", t("i1")),
                ("to", t("i2")),
                ("kind", t("blocks")),
            ]),
        );

        // i2 blocked until i1 is done.
        assert!(c.state().item_view("i2").unwrap().blocked);
        comm(
            &mut c,
            OWNER,
            OP_SET_ITEM_PROGRESS,
            args(vec![("itemId", t("i1")), ("status", t("done"))]),
            0,
        );
        assert!(!c.state().item_view("i2").unwrap().blocked);
    }

    #[test]
    fn dependency_cycle_is_rejected_at_fold() {
        let mut c = coord();
        let mut sp = Spine::new();
        sp.deliver(
            &mut c,
            OP_ADD_TIMELINE_ITEM,
            args(vec![
                ("itemId", t("i1")),
                ("kind", t("action")),
                ("title", t("a")),
            ]),
        );
        sp.deliver(
            &mut c,
            OP_ADD_TIMELINE_ITEM,
            args(vec![
                ("itemId", t("i2")),
                ("kind", t("action")),
                ("title", t("b")),
            ]),
        );
        sp.deliver(
            &mut c,
            OP_ADD_DEPENDENCY,
            args(vec![
                ("edgeId", t("e1")),
                ("from", t("i1")),
                ("to", t("i2")),
                ("kind", t("blocks")),
            ]),
        );
        // e2 would close a cycle i1->i2->i1; the spine ACCEPTS it but the fold skips it.
        sp.deliver(
            &mut c,
            OP_ADD_DEPENDENCY,
            args(vec![
                ("edgeId", t("e2")),
                ("from", t("i2")),
                ("to", t("i1")),
                ("kind", t("blocks")),
            ]),
        );
        let s = c.state();
        assert_eq!(s.deps.len(), 1, "the cycle-closing edge is inert");
        assert!(
            !s.item_view("i1").unwrap().blocked,
            "i1 stays unblocked (no i2->i1 edge)"
        );
    }

    #[test]
    fn deliverables_are_project_subscriptions_and_piece_count_is_derived() {
        let mut c = coord();
        let mut sp = Spine::new();
        sp.deliver(
            &mut c,
            OP_ADD_TIMELINE_ITEM,
            args(vec![
                ("itemId", t("m1")),
                ("kind", t("event")),
                ("title", t("deadline")),
            ]),
        );
        sp.deliver(
            &mut c,
            OP_SUBSCRIBE,
            args(vec![
                ("subId", t("d1")),
                ("target", t("child-a")),
                ("kind", t("project")),
                ("disclosure", t("summary")),
                ("originItem", t("m1")),
            ]),
        );
        sp.deliver(
            &mut c,
            OP_SUBSCRIBE,
            args(vec![
                ("subId", t("d2")),
                ("target", t("child-b")),
                ("kind", t("project")),
                ("disclosure", t("existence")),
            ]),
        );
        // a non-deliverable edge (a topic subscription) must not count as a piece.
        sp.deliver(
            &mut c,
            OP_SUBSCRIBE,
            args(vec![
                ("subId", t("s1")),
                ("target", t("topic-x")),
                ("kind", t("topic")),
                ("disclosure", t("full")),
            ]),
        );
        comm(
            &mut c,
            OWNER,
            OP_TOUCH_SUBSCRIPTION,
            args(vec![("subId", t("d1")), ("at", i(1000))]),
            0,
        );

        let s = c.state();
        assert_eq!(
            s.piece_count(),
            2,
            "the 'N-piece body' is a derived count of child projects"
        );
        assert_eq!(s.deliverables().len(), 2);
        assert_eq!(s.subscription_cursor("d1"), Some(1000));
    }

    #[test]
    fn subscribe_pin_to_missing_item_is_inert() {
        let mut c = coord();
        let mut sp = Spine::new();
        // originItem "nope" does not exist -> the subscribe self-rejects at fold.
        sp.deliver(
            &mut c,
            OP_SUBSCRIBE,
            args(vec![
                ("subId", t("d1")),
                ("target", t("child-a")),
                ("kind", t("project")),
                ("disclosure", t("summary")),
                ("originItem", t("nope")),
            ]),
        );
        assert_eq!(c.state().piece_count(), 0);
    }

    #[test]
    fn progress_before_item_exists_is_inert_but_applies_once_it_does() {
        // Deliver the COMMUTATIVE progress BEFORE the sequenced addTimelineItem.
        // The fold applies the spine first, so the progress lands on the item.
        let mut c = coord();
        comm(
            &mut c,
            OWNER,
            OP_SET_ITEM_PROGRESS,
            args(vec![("itemId", t("i1")), ("status", t("in_progress"))]),
            0,
        );
        // progress for an item that is NEVER added stays inert.
        comm(
            &mut c,
            A,
            OP_SET_ITEM_PROGRESS,
            args(vec![("itemId", t("ghost")), ("status", t("done"))]),
            0,
        );
        let mut sp = Spine::new();
        sp.deliver(
            &mut c,
            OP_ADD_TIMELINE_ITEM,
            args(vec![
                ("itemId", t("i1")),
                ("kind", t("action")),
                ("title", t("x")),
            ]),
        );

        let s = c.state();
        assert_eq!(s.board().len(), 1, "the ghost item never materialises");
        assert_eq!(
            s.item_view("i1").unwrap().status,
            Some(ItemStatus::InProgress)
        );
    }

    #[test]
    fn assignee_must_be_a_member() {
        let mut c = coord();
        let mut sp = Spine::new();
        sp.deliver(
            &mut c,
            OP_ADD_TIMELINE_ITEM,
            args(vec![
                ("itemId", t("i1")),
                ("kind", t("action")),
                ("title", t("x")),
            ]),
        );
        // A is a member -> assignment sticks.
        sp.deliver(
            &mut c,
            OP_SET_ASSIGNEE,
            args(vec![
                ("itemId", t("i1")),
                ("member", t(&hex::encode(A))),
                ("assigned", i(1)),
            ]),
        );
        // a non-member hex -> the assignment self-rejects at fold.
        sp.deliver(
            &mut c,
            OP_SET_ASSIGNEE,
            args(vec![
                ("itemId", t("i1")),
                ("member", t(&hex::encode([0x99u8; 32]))),
                ("assigned", i(1)),
            ]),
        );
        let s = c.state();
        let assignees = &s.item_view("i1").unwrap().assignees;
        assert_eq!(assignees, &vec![A]);
    }

    #[test]
    fn enum_as_str_roundtrips_parse() {
        assert_eq!(
            ProjectStatus::parse(ProjectStatus::Paused.as_str()),
            Ok(ProjectStatus::Paused)
        );
        assert_eq!(ItemKind::parse(ItemKind::Gate.as_str()), Ok(ItemKind::Gate));
        assert_eq!(
            ItemStatus::parse(ItemStatus::InProgress.as_str()),
            Ok(ItemStatus::InProgress)
        );
        assert_eq!(
            Disclosure::parse(Disclosure::Summary.as_str()),
            Ok(Disclosure::Summary)
        );
    }
}
