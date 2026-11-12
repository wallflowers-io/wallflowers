//! tags — THE TAGS FACET (`facets.tags`): labels an object's members put on it, each optionally
//! linking another object.
//!
//! Ralph, 12 Nov 2026: "a tags facet on every kind. This will help for a number of linking ops
//! later on." A tag is a label plus an optional object; members tag, the owner or an admin hides.
//!
//! - `base.tag` / `base.untag`: a member's own tag, one slot per (author, label), LWW by gen.
//! - `base.hideTag`: the owner's or an admin's, per label, LWW by (gen, author). Members' tags
//!   are kept; a hidden label is drawn for no one but its curators and never leaves the object.
//! - `base.setTagged` / `base.clearTagged`: the LINKED object's half of `tagged`, written in its
//!   own log by its own member, keyed (object, label). A tag whose linked object holds no half
//!   is unconfirmed; only a node that holds both can say which.
//!
//! The facet pattern, as [`crate::exterior`]: a kind holds a [`TagsState`], splices the five
//! [`OpDecl`]s into its `ops()`, and delegates to [`reduce_tags`]. Band `0xF010`; every number
//! here is the ICD's (build.rs).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::coordinator::{ArgVal, Args};
use crate::object::{Authority, Commutativity, DeltaRejection, MemberId, Op, OpDecl};
use crate::object_args::{req_gen, req_hex_id, req_int, req_text};

pub(crate) mod icd {
    include!(concat!(env!("OUT_DIR"), "/icd_tags.rs"));
}
pub use icd::{LABEL_MAX, MAX_LIVE, MAX_PER_AUTHOR, OP_CLEAR_TAGGED, OP_HIDE_TAG, OP_SET_TAGGED, OP_TAG, OP_UNTAG};

pub const TAG: OpDecl = OpDecl { op_id: OP_TAG, name: "base.tag", authority: Authority::AnyMember, commutativity: Commutativity::Commutative };
pub const UNTAG: OpDecl = OpDecl { op_id: OP_UNTAG, name: "base.untag", authority: Authority::AnyMember, commutativity: Commutativity::Commutative };
pub const HIDE_TAG: OpDecl = OpDecl {
    op_id: OP_HIDE_TAG,
    name: "base.hideTag",
    authority: Authority::OwnerOrRole(crate::group::GroupRole::Admin),
    commutativity: Commutativity::Commutative,
};
pub const SET_TAGGED: OpDecl =
    OpDecl { op_id: OP_SET_TAGGED, name: "base.setTagged", authority: Authority::AnyMember, commutativity: Commutativity::Commutative };
pub const CLEAR_TAGGED: OpDecl =
    OpDecl { op_id: OP_CLEAR_TAGGED, name: "base.clearTagged", authority: Authority::AnyMember, commutativity: Commutativity::Commutative };

/// The five, for a drift pin; a kind's table names each one itself (const initialiser).
pub const TAGS_OPS: [OpDecl; 5] = [TAG, UNTAG, HIDE_TAG, SET_TAGGED, CLEAR_TAGGED];

pub fn is_tags_op(op_id: u32) -> bool {
    matches!(op_id, OP_TAG | OP_UNTAG | OP_HIDE_TAG | OP_SET_TAGGED | OP_CLEAR_TAGGED)
}

/// A label as the ICD's grammar has it: lowercase letters, digits and `_`, not empty, at most
/// [`LABEL_MAX`] bytes. What `Hashtags.extract` yields from `#Label`. Refused, never rewritten.
pub fn is_label(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= LABEL_MAX
        && s.chars().all(|c| c == '_' || c.is_numeric() || (c.is_alphabetic() && !c.is_uppercase()))
        && s.to_lowercase() == s
}

fn label(args: &Args) -> Result<String, DeltaRejection> {
    let l = req_text(args, "label")?;
    if !is_label(l) {
        return Err(DeltaRejection::MalformedArgs);
    }
    Ok(l.to_string())
}

/// One member's tag. `live` false is an untag, kept so the slot's gen orders a late tag.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub object: Option<String>,
    pub at: i64,
    pub gen: u64,
    pub live: bool,
}

/// A curator's word on one label.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hide {
    pub hidden: bool,
    pub gen: u64,
    pub by: MemberId,
}

/// This object's half of another object's tag: that it is tagged with this one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Half {
    pub at: i64,
    pub gen: u64,
    pub live: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagsState {
    /// By (author, label).
    #[serde(with = "pairs")]
    pub by: BTreeMap<(MemberId, String), Tag>,
    /// By label.
    pub hidden: BTreeMap<String, Hide>,
    /// By (tagged object, label).
    #[serde(with = "pairs")]
    pub tagged: BTreeMap<(String, String), Half>,
}

impl TagsState {
    fn live(&self) -> impl Iterator<Item = (&(MemberId, String), &Tag)> {
        self.by.iter().filter(|(_, t)| t.live)
    }

    pub fn is_hidden(&self, label: &str) -> bool {
        self.hidden.get(label).is_some_and(|h| h.hidden)
    }
}

/// Fold one tags op. `admin` is whether the author holds admin in the kind's own roles (a kind
/// with no roles facet passes false: its owner is its only curator). Whether the author is a
/// member is not asked again: MLS answered it at ingest, for the epoch the op was written in.
pub fn reduce_tags(state: &mut TagsState, op: &Op<'_>, admin: bool) -> Result<(), DeltaRejection> {
    let args = op.args;
    match op.op_id {
        OP_TAG => {
            // Validate everything before the first mutation — reduce is atomic.
            let label = label(args)?;
            let object = match crate::arg_reads::get(args, "object") {
                None => None,
                Some(_) => Some(req_hex_id(args, "object")?),
            };
            let at = req_int(args, "at")?;
            let gen = req_gen(args)?;
            let key = (*op.author, label.clone());
            if state.by.get(&key).is_some_and(|t| t.gen > gen) {
                return Ok(());
            }
            let held = state.by.get(&key).is_some_and(|t| t.live);
            if !held {
                let labels: BTreeSet<&str> = state.live().map(|((_, l), _)| l.as_str()).collect();
                if !labels.contains(label.as_str()) && labels.len() >= MAX_LIVE {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                if state.live().filter(|((a, _), _)| a == op.author).count() >= MAX_PER_AUTHOR {
                    return Err(DeltaRejection::PreconditionFailed);
                }
            }
            state.by.insert(key, Tag { object, at, gen, live: true });
            Ok(())
        }
        OP_UNTAG => {
            let label = label(args)?;
            let gen = req_gen(args)?;
            let slot = state.by.get_mut(&(*op.author, label)).ok_or(DeltaRejection::PreconditionFailed)?;
            if slot.gen <= gen {
                slot.gen = gen;
                slot.live = false;
            }
            Ok(())
        }
        OP_HIDE_TAG => {
            // Checked at fold as at write: an admin's standing is the kind's folded roles.
            if *op.author != op.ctx.owner && !admin {
                return Err(DeltaRejection::Unauthorized);
            }
            let label = label(args)?;
            let hidden = match req_int(args, "hidden")? {
                0 => false,
                1 => true,
                _ => return Err(DeltaRejection::MalformedArgs),
            };
            let gen = req_gen(args)?;
            if state.hidden.get(&label).is_some_and(|h| (h.gen, h.by) > (gen, *op.author)) {
                return Ok(());
            }
            state.hidden.insert(label, Hide { hidden, gen, by: *op.author });
            Ok(())
        }
        OP_SET_TAGGED | OP_CLEAR_TAGGED => {
            let object = req_hex_id(args, "object")?;
            let label = label(args)?;
            let at = if op.op_id == OP_SET_TAGGED { req_int(args, "at")? } else { 0 };
            let gen = req_gen(args)?;
            let key = (object, label);
            match state.tagged.get_mut(&key) {
                Some(h) if h.gen > gen => {}
                Some(h) if op.op_id == OP_CLEAR_TAGGED => {
                    h.gen = gen;
                    h.live = false;
                }
                None if op.op_id == OP_CLEAR_TAGGED => return Err(DeltaRejection::PreconditionFailed),
                _ => {
                    state.tagged.insert(key, Half { at, gen, live: true });
                }
            }
            Ok(())
        }
        _ => Err(DeltaRejection::UnknownType),
    }
}

/// The tags as a view carries them, by (label, object): `[{label, object, count, by, hidden}]`.
/// A hidden label is listed only when `curator` (the owner or an admin, who may show it again).
pub fn listed(state: &TagsState, curator: bool) -> serde_json::Value {
    let mut rows: BTreeMap<(&str, Option<&str>), Vec<String>> = BTreeMap::new();
    for ((author, label), t) in state.live() {
        rows.entry((label.as_str(), t.object.as_deref())).or_default().push(hex::encode(author));
    }
    serde_json::Value::Array(
        rows.into_iter()
            .filter(|((label, _), _)| curator || !state.is_hidden(label))
            .map(|((label, object), by)| {
                serde_json::json!({ "label": label, "object": object, "count": by.len(), "by": by, "hidden": state.is_hidden(label) })
            })
            .collect(),
    )
}

/// This object's halves: `[{object, label}]`, the objects tagged with it.
pub fn tagged_by(state: &TagsState) -> serde_json::Value {
    serde_json::Value::Array(
        state.tagged.iter().filter(|(_, h)| h.live).map(|((object, label), _)| serde_json::json!({ "object": object, "label": label })).collect(),
    )
}

/// The labels the outside may hold: unhidden, with their linked objects.
pub fn outside(state: &TagsState) -> Vec<(String, Option<String>)> {
    let set: BTreeSet<(String, Option<String>)> =
        state.live().filter(|((_, l), _)| !state.is_hidden(l)).map(|((_, l), t)| (l.clone(), t.object.clone())).collect();
    set.into_iter().collect()
}

/// The args `base.tag` carries; `authoring::build` adds the gen.
pub fn tag_args(label: &str, object_id_hex: Option<&str>, at_ms: i64) -> Args {
    let mut a = Args::new();
    a.insert("label".into(), ArgVal::Text(label.to_string()));
    if let Some(o) = object_id_hex {
        a.insert("object".into(), ArgVal::Text(o.to_string()));
    }
    a.insert("at".into(), ArgVal::Int(at_ms));
    a
}

/// The args `base.untag` carries.
pub fn untag_args(label: &str) -> Args {
    let mut a = Args::new();
    a.insert("label".into(), ArgVal::Text(label.to_string()));
    a
}

/// The args `base.hideTag` carries.
pub fn hide_tag_args(label: &str, hidden: bool) -> Args {
    let mut a = untag_args(label);
    a.insert("hidden".into(), ArgVal::Int(i64::from(hidden)));
    a
}

/// The args `base.setTagged` carries: `object` is the tagged object.
pub fn set_tagged_args(object_id_hex: &str, label: &str, at_ms: i64) -> Args {
    let mut a = clear_tagged_args(object_id_hex, label);
    a.insert("at".into(), ArgVal::Int(at_ms));
    a
}

/// The args `base.clearTagged` carries.
pub fn clear_tagged_args(object_id_hex: &str, label: &str) -> Args {
    let mut a = untag_args(label);
    a.insert("object".into(), ArgVal::Text(object_id_hex.to_string()));
    a
}

/// Serde for a map keyed by a tuple: a list of pairs, since serde_json refuses a non-string key.
mod pairs {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<S: Serializer, K: Serialize, V: Serialize>(m: &BTreeMap<K, V>, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(m.iter())
    }

    pub fn deserialize<'de, D: Deserializer<'de>, K: Deserialize<'de> + Ord, V: Deserialize<'de>>(d: D) -> Result<BTreeMap<K, V>, D::Error> {
        Ok(Vec::<(K, V)>::deserialize(d)?.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_is_the_hashtag_grammar() {
        assert!(is_label("parsnips"));
        assert!(is_label("seed_swap_2026"));
        assert!(is_label("café"));
        assert!(!is_label("Parsnips"));
        assert!(!is_label("seed swap"));
        assert!(!is_label("#parsnips"));
        assert!(!is_label(""));
        assert!(!is_label(&"a".repeat(LABEL_MAX + 1)));
        assert!(is_label(&"a".repeat(LABEL_MAX)));
    }
}
