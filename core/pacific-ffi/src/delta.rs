//! The op constructors Swift calls, each producing a [`DeltaDraft`] that `Core`
//! authors (`group_author`, `project_author`). Only constructors with a caller live
//! here; op ids are the reducers' own constants, and `icd_pin` holds each
//! constructor's op and args to the ICD.

use pacific_core::coordinator::{ArgVal, Args};

/// op ids, re-exported from the reducers rather than restated.
pub mod op {
    pub const GROUP_SET_AFFILIATION: u32 = pacific_core::group::OP_SET_AFFILIATION;
    pub const PROJECT_CONFIGURE: u32 = pacific_core::project::OP_CONFIGURE;
    pub const PROJECT_ADD_TIMELINE_ITEM: u32 = pacific_core::project::OP_ADD_TIMELINE_ITEM;
    pub const PROJECT_SUPPRESS_ITEM: u32 = pacific_core::project::OP_SUPPRESS_ITEM;
    pub const PROJECT_SET_ITEM_SCHEDULE: u32 = pacific_core::project::OP_SET_ITEM_SCHEDULE;
    pub const PROJECT_SET_ASSIGNEE: u32 = pacific_core::project::OP_SET_ASSIGNEE;
    pub const PROJECT_ADD_DEPENDENCY: u32 = pacific_core::project::OP_ADD_DEPENDENCY;
    pub const PROJECT_REMOVE_DEPENDENCY: u32 = pacific_core::project::OP_REMOVE_DEPENDENCY;
    pub const PROJECT_SUBSCRIBE: u32 = pacific_core::project::OP_SUBSCRIBE;
    pub const PROJECT_UNSUBSCRIBE: u32 = pacific_core::project::OP_UNSUBSCRIBE;
    pub const PROJECT_SET_ITEM_PROGRESS: u32 = pacific_core::project::OP_SET_ITEM_PROGRESS;
    pub const PROJECT_SET_ITEM_FIELD: u32 = pacific_core::project::OP_SET_ITEM_FIELD;
    pub const PROJECT_SET_OBJECTIVE: u32 = pacific_core::project::OP_SET_OBJECTIVE;
    pub const PROJECT_ADD_STAKEHOLDER: u32 = pacific_core::project::OP_ADD_STAKEHOLDER;
    pub const PROJECT_REMOVE_STAKEHOLDER: u32 = pacific_core::project::OP_REMOVE_STAKEHOLDER;
    pub const PROJECT_ADD_LOCATION: u32 = pacific_core::project::OP_ADD_LOCATION;
    pub const PROJECT_REMOVE_LOCATION: u32 = pacific_core::project::OP_REMOVE_LOCATION;
    pub const PROJECT_SET_KPI: u32 = pacific_core::project::OP_SET_KPI;
    pub const PROJECT_REMOVE_KPI: u32 = pacific_core::project::OP_REMOVE_KPI;
}

/// The kinds a draft can name (mirrors `pacific_core::object::ObjectKind`).
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Group,
    Project,
}

/// The two envelope arg value shapes (`map<str, int|text>`).
#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum ArgValue {
    Text { value: String },
    Int { value: i64 },
}

impl ArgValue {
    fn core(&self) -> ArgVal {
        match self {
            ArgValue::Text { value } => ArgVal::Text(value.clone()),
            ArgValue::Int { value } => ArgVal::Int(*value),
        }
    }
}

/// One key/value pair of a delta's args.
#[derive(uniffi::Record, Clone, Debug)]
pub struct ArgEntry {
    pub key: String,
    pub value: ArgValue,
}

fn text(key: &str, value: String) -> ArgEntry {
    ArgEntry {
        key: key.to_string(),
        value: ArgValue::Text { value },
    }
}
fn int(key: &str, value: i64) -> ArgEntry {
    ArgEntry {
        key: key.to_string(),
        value: ArgValue::Int { value },
    }
}
fn opt_text(key: &str, value: Option<String>) -> Option<ArgEntry> {
    value.map(|v| text(key, v))
}

/// A delta ready to author: which op, and its args. Produced by the constructors
/// below, or built by hand for a generic path (`CoreService.groupAuthor`).
#[derive(uniffi::Record, Clone, Debug)]
pub struct DeltaDraft {
    pub kind: Kind,
    pub op_id: u32,
    pub args: Vec<ArgEntry>,
}

fn draft(kind: Kind, op_id: u32, args: Vec<Option<ArgEntry>>) -> DeltaDraft {
    DeltaDraft {
        kind,
        op_id,
        args: args.into_iter().flatten().collect(),
    }
}

/// Lower a draft's args to the core `Args` map — the authoring FFI path
/// (`Core::project_author`) needs the map, not just the canonical bytes.
pub(crate) fn draft_args(draft: &DeltaDraft) -> Args {
    draft
        .args
        .iter()
        .map(|e| (e.key.clone(), e.value.core()))
        .collect()
}

#[uniffi::export]
pub fn group_set_affiliation(
    peer: String,
    rel: String,
    name: String,
    tether: Option<String>,
    at: i64,
) -> DeltaDraft {
    draft(
        Kind::Group,
        op::GROUP_SET_AFFILIATION,
        vec![
            Some(text("peer", peer)),
            Some(text("rel", rel)),
            Some(text("name", name)),
            opt_text("tether", tether),
            Some(int("at", at)),
        ],
    )
}

#[uniffi::export]
pub fn project_configure(title: String, status: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_CONFIGURE,
        vec![Some(text("title", title)), Some(text("status", status))],
    )
}

#[uniffi::export]
pub fn project_add_timeline_item(item_id: String, kind: String, title: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_ADD_TIMELINE_ITEM,
        vec![
            Some(text("itemId", item_id)),
            Some(text("kind", kind)),
            Some(text("title", title)),
        ],
    )
}

#[uniffi::export]
pub fn project_suppress_item(item_id: String, suppressed: bool) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_SUPPRESS_ITEM,
        vec![
            Some(text("itemId", item_id)),
            Some(int("suppressed", suppressed as i64)),
        ],
    )
}

#[uniffi::export]
pub fn project_set_item_schedule(item_id: String, at: i64, end_date: Option<i64>) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_SET_ITEM_SCHEDULE,
        vec![
            Some(text("itemId", item_id)),
            Some(int("at", at)),
            end_date.map(|e| int("endDate", e)),
        ],
    )
}

#[uniffi::export]
pub fn project_set_assignee(item_id: String, member: String, assigned: bool) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_SET_ASSIGNEE,
        vec![
            Some(text("itemId", item_id)),
            Some(text("member", member)),
            Some(int("assigned", assigned as i64)),
        ],
    )
}

#[uniffi::export]
pub fn project_add_dependency(
    edge_id: String,
    from: String,
    to: String,
    kind: String,
) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_ADD_DEPENDENCY,
        vec![
            Some(text("edgeId", edge_id)),
            Some(text("from", from)),
            Some(text("to", to)),
            Some(text("kind", kind)),
        ],
    )
}

#[uniffi::export]
pub fn project_remove_dependency(edge_id: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_REMOVE_DEPENDENCY,
        vec![Some(text("edgeId", edge_id))],
    )
}

#[uniffi::export]
pub fn project_subscribe(
    sub_id: String,
    target: String,
    kind: String,
    disclosure: String,
    origin_item: Option<String>,
) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_SUBSCRIBE,
        vec![
            Some(text("subId", sub_id)),
            Some(text("target", target)),
            Some(text("kind", kind)),
            Some(text("disclosure", disclosure)),
            opt_text("originItem", origin_item),
        ],
    )
}

#[uniffi::export]
pub fn project_unsubscribe(sub_id: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_UNSUBSCRIBE,
        vec![Some(text("subId", sub_id))],
    )
}

#[uniffi::export]
pub fn project_set_item_progress(item_id: String, status: String, gen: i64) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_SET_ITEM_PROGRESS,
        vec![
            Some(text("itemId", item_id)),
            Some(text("status", status)),
            Some(int("gen", gen)),
        ],
    )
}

#[uniffi::export]
pub fn project_set_item_field(
    item_id: String,
    field: String,
    value: String,
    gen: i64,
) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_SET_ITEM_FIELD,
        vec![
            Some(text("itemId", item_id)),
            Some(text("field", field)),
            Some(text("value", value)),
            Some(int("gen", gen)),
        ],
    )
}

#[uniffi::export]
pub fn project_set_objective(headline: String, goal: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_SET_OBJECTIVE,
        vec![Some(text("headline", headline)), Some(text("goal", goal))],
    )
}

#[uniffi::export]
pub fn project_add_stakeholder(stakeholder_id: String, name: String, note: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_ADD_STAKEHOLDER,
        vec![
            Some(text("stakeholderId", stakeholder_id)),
            Some(text("name", name)),
            Some(text("note", note)),
        ],
    )
}

#[uniffi::export]
pub fn project_remove_stakeholder(stakeholder_id: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_REMOVE_STAKEHOLDER,
        vec![Some(text("stakeholderId", stakeholder_id))],
    )
}

#[uniffi::export]
pub fn project_add_location(location_id: String, name: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_ADD_LOCATION,
        vec![
            Some(text("locationId", location_id)),
            Some(text("name", name)),
        ],
    )
}

#[uniffi::export]
pub fn project_remove_location(location_id: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_REMOVE_LOCATION,
        vec![Some(text("locationId", location_id))],
    )
}

#[uniffi::export]
pub fn project_set_kpi(kpi_id: String, label: String, target: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_SET_KPI,
        vec![
            Some(text("kpiId", kpi_id)),
            Some(text("label", label)),
            Some(text("target", target)),
        ],
    )
}

#[uniffi::export]
pub fn project_remove_kpi(kpi_id: String) -> DeltaDraft {
    draft(
        Kind::Project,
        op::PROJECT_REMOVE_KPI,
        vec![Some(text("kpiId", kpi_id))],
    )
}

/// Each constructor's op and args, held to the ICD: the op is one the ICD declares for
/// the kind, every arg is one it declares, and every required one is carried.
#[cfg(test)]
mod icd_pin {
    use super::*;
    use std::collections::BTreeSet;

    fn icd() -> serde_json::Value {
        serde_json::from_str(include_str!("../../coordination/delta-graph.icd.json")).expect("the ICD")
    }

    /// The ICD's op with this id on this kind, from the kind's ops or a facet's.
    fn icd_op<'a>(icd: &'a serde_json::Value, kind: &str, op_id: u32) -> Option<(&'a String, &'a serde_json::Value)> {
        let from_kind = icd["kinds"][kind]["ops"].as_object().into_iter().flatten();
        let from_facets = icd["facets"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, f)| f["on"].as_array().is_some_and(|on| on.iter().any(|k| k == kind)))
            .flat_map(|(_, f)| f["ops"].as_object().into_iter().flatten());
        from_kind.chain(from_facets).find(|(_, o)| o["op"].as_u64() == Some(op_id as u64))
    }

    #[test]
    fn every_constructor_writes_an_icd_op_with_its_args() {
        let icd = icd();
        let (s, n) = (|| "x".to_string(), 1i64);
        let drafts = [
            group_set_affiliation(s(), s(), s(), Some(s()), n),
            project_configure(s(), s()),
            project_add_timeline_item(s(), s(), s()),
            project_suppress_item(s(), true),
            project_set_item_schedule(s(), n, Some(n)),
            project_set_assignee(s(), s(), true),
            project_add_dependency(s(), s(), s(), s()),
            project_remove_dependency(s()),
            project_subscribe(s(), s(), s(), s(), Some(s())),
            project_unsubscribe(s()),
            project_set_item_progress(s(), s(), n),
            project_set_item_field(s(), s(), s(), n),
            project_set_objective(s(), s()),
            project_add_stakeholder(s(), s(), s()),
            project_remove_stakeholder(s()),
            project_add_location(s(), s()),
            project_remove_location(s()),
            project_set_kpi(s(), s(), s()),
            project_remove_kpi(s()),
        ];
        let mut wrong = Vec::new();
        for d in &drafts {
            let kind = match d.kind {
                Kind::Group => "group",
                Kind::Project => "project",
            };
            let Some((name, op)) = icd_op(&icd, kind, d.op_id) else {
                wrong.push(format!("{kind} op {}: the ICD declares no such op", d.op_id));
                continue;
            };
            let ours: BTreeSet<&str> = d.args.iter().map(|a| a.key.as_str()).collect();
            let args = op["args"].as_object().cloned().unwrap_or_default();
            let undeclared: Vec<_> = ours.iter().filter(|k| !args.contains_key(**k)).collect();
            let missing: Vec<_> = args
                .iter()
                .filter(|(k, a)| a["required"].as_bool() == Some(true) && !ours.contains(k.as_str()))
                .map(|(k, _)| k)
                .collect();
            if !undeclared.is_empty() || !missing.is_empty() {
                wrong.push(format!("{name}: undeclared {undeclared:?}, missing {missing:?}"));
            }
        }
        assert!(wrong.is_empty(), "constructors the ICD does not agree with:\n{}", wrong.join("\n"));
    }
}
