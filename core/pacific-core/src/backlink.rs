//! backlink — this object declaring its own half of a relation another asserted.
//!
//! ## What it is for
//!
//! Reciprocity is required (ruled 25 September 2026): every relation is declared at
//! BOTH ends, because a one-sided edge cannot be walked from the far end. `group`
//! already had a way to write its halves — `group.setAffiliation {peer, rel, …}`,
//! whose `rel` vocabulary names which half it is. **A kind that is not a Group had
//! no way at all.**
//!
//! That is why two relations stayed one-sided when the rest closed:
//!
//! * `happens_at` — `event.setVenue` writes the venue into the EVENT. A Place has
//!   `setProfile`, `setAccess`, `setDoorbell`, `setLand`, `clearLand` and `post`,
//!   and not one of them can record an event held there. So a venue could not list
//!   what happens in it, anyone could claim their event was there, and the Place
//!   could not decline.
//! * `created` — `group.setAffiliation {rel: created}` writes it in the CREATOR.
//!   The Post, Event or Thing carried nothing.
//!
//! ## Why a set, where `parent` is a slot
//!
//! A part belongs to exactly one object, so [`crate::parent`] holds one `ParentRef`.
//! A backlink is the general case and holds many: a Place has many events held
//! there, a Thing has one creator, and both are the same op. The key is
//! `(rel, object)` so two relations to the same object coexist and re-declaring one
//! updates in place.
//!
//! ## Not on `group`
//!
//! Deliberately. A Group writes its halves with `setAffiliation`, and two ways to
//! write one fact is two sources of truth — the error this whole document forbids.
//! The reserved band is `0xF009`.

use std::collections::BTreeMap;

use crate::object::{Authority, Commutativity, DeltaRejection, Op, OpDecl};
use crate::object_args::{req_hex_id, req_int, req_text};

/// Declare this object's half: the object at the other end, and which relation.
pub const OP_SET_BACKLINK: u32 = 0xF009_0000;
/// Withdraw this side's half. The far end keeps its own.
pub const OP_CLEAR_BACKLINK: u32 = 0xF009_0001;

pub static BACKLINK_OPS: &[OpDecl] = &[
    OpDecl {
        op_id: OP_SET_BACKLINK,
        name: "base.setBacklink",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_CLEAR_BACKLINK,
        name: "base.clearBacklink",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
];

/// Is this one of the backlink ops?
pub fn is_backlink_op(op_id: u32) -> bool {
    op_id == OP_SET_BACKLINK || op_id == OP_CLEAR_BACKLINK
}

/// The relations a backlink may declare. A CLOSED vocabulary: an unknown `rel` is
/// refused rather than stored, because a half nothing reads is a half that only
/// looks like reciprocity.
pub const RELATIONS: &[&str] = &["created", "venue"];

/// One declared half.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Backlink {
    pub rel: String,
    pub object: String,
    pub at: i64,
}

/// The one reducer, over whatever State holds the halves. Keyed `(rel, object)`.
pub fn reduce_backlink(
    links: &mut BTreeMap<(String, String), Backlink>,
    op: &Op<'_>,
) -> Result<(), DeltaRejection> {
    match op.op_id {
        OP_SET_BACKLINK => {
            // Validate everything before the first mutation — reduce is atomic.
            let object = req_hex_id(op.args, "object")?;
            let rel = req_text(op.args, "rel")?.to_string();
            if !RELATIONS.contains(&rel.as_str()) {
                return Err(DeltaRejection::MalformedArgs);
            }
            let at = req_int(op.args, "at")?;
            links.insert((rel.clone(), object.clone()), Backlink { rel, object, at });
            Ok(())
        }
        OP_CLEAR_BACKLINK => {
            let object = req_hex_id(op.args, "object")?;
            let rel = req_text(op.args, "rel")?.to_string();
            // Withdrawing a half that was never declared fails loudly, as every
            // other `clear*` in the catalogue does.
            links
                .remove(&(rel, object))
                .ok_or(DeltaRejection::PreconditionFailed)?;
            Ok(())
        }
        _ => Err(DeltaRejection::UnknownType),
    }
}

/// The halves as a view carries them: a list, since a map keyed by `(rel, object)` has
/// no string key, and serde_json refuses one. So every object that declared its half
/// stopped rendering (NC-81).
pub fn listed(links: &BTreeMap<(String, String), Backlink>) -> serde_json::Value {
    serde_json::Value::Array(
        links.values().map(|b| serde_json::json!({ "rel": b.rel, "object": b.object, "at": b.at })).collect(),
    )
}

/// Serde for a State that derives it: the halves written as [`listed`] writes them, and
/// read back into the map by their own `(rel, object)`.
pub mod as_list {
    use super::Backlink;
    use serde::{Deserialize, Deserializer, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<S: Serializer>(links: &BTreeMap<(String, String), Backlink>, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(links.values())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<(String, String), Backlink>, D::Error> {
        Ok(Vec::<Backlink>::deserialize(d)?.into_iter().map(|b| ((b.rel.clone(), b.object.clone()), b)).collect())
    }
}

/// The args `base.setBacklink` carries.
pub fn set_backlink_args(object_id_hex: &str, rel: &str, at_ms: i64) -> crate::coordinator::Args {
    let mut a = crate::coordinator::Args::new();
    a.insert("object".into(), crate::coordinator::ArgVal::Text(object_id_hex.to_string()));
    a.insert("rel".into(), crate::coordinator::ArgVal::Text(rel.to_string()));
    a.insert("at".into(), crate::coordinator::ArgVal::Int(at_ms));
    a
}

/// The args `base.clearBacklink` carries.
pub fn clear_backlink_args(object_id_hex: &str, rel: &str) -> crate::coordinator::Args {
    let mut a = crate::coordinator::Args::new();
    a.insert("object".into(), crate::coordinator::ArgVal::Text(object_id_hex.to_string()));
    a.insert("rel".into(), crate::coordinator::ArgVal::Text(rel.to_string()));
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator::ArgVal;
    use crate::object::{MemberId, ReduceContext};

    const ME: MemberId = [1u8; 32];
    static ROSTER: &[MemberId] = &[ME];

    fn ctx() -> ReduceContext<'static> {
        ReduceContext { members: ROSTER, owner: ME, epoch: 0 }
    }
    fn op<'a>(
        op_id: u32,
        args: &'a crate::coordinator::Args,
        c: &'a ReduceContext<'a>,
    ) -> Op<'a> {
        Op { op_id, args, author: &ME, pos: None, ctx: c }
    }

    /// The finding this facet exists for: a Place can record an event held there.
    #[test]
    fn a_place_can_hold_many_events() {
        let c = ctx();
        let mut links = BTreeMap::new();
        for (i, id) in ["aa", "bb", "cc"].iter().enumerate() {
            let a = set_backlink_args(&id.repeat(32), "venue", i as i64);
            reduce_backlink(&mut links, &op(OP_SET_BACKLINK, &a, &c)).unwrap();
        }
        assert_eq!(links.len(), 3, "a venue hosts more than one thing");
    }

    /// Two relations to the SAME object coexist — the key is the pair.
    #[test]
    fn two_relations_to_one_object_are_separate_halves() {
        let c = ctx();
        let mut links = BTreeMap::new();
        let id = "aa".repeat(32);
        for rel in ["created", "venue"] {
            let a = set_backlink_args(&id, rel, 1);
            reduce_backlink(&mut links, &op(OP_SET_BACKLINK, &a, &c)).unwrap();
        }
        assert_eq!(links.len(), 2);
    }

    /// Re-declaring updates in place rather than accumulating duplicates.
    #[test]
    fn redeclaring_the_same_half_updates_it() {
        let c = ctx();
        let mut links = BTreeMap::new();
        let id = "aa".repeat(32);
        for at in [1, 9] {
            let a = set_backlink_args(&id, "venue", at);
            reduce_backlink(&mut links, &op(OP_SET_BACKLINK, &a, &c)).unwrap();
        }
        assert_eq!(links.len(), 1);
        assert_eq!(links.values().next().unwrap().at, 9);
    }

    #[test]
    fn an_unknown_relation_is_refused_rather_than_stored() {
        let c = ctx();
        let mut links = BTreeMap::new();
        let a = set_backlink_args(&"aa".repeat(32), "invented", 1);
        assert_eq!(
            reduce_backlink(&mut links, &op(OP_SET_BACKLINK, &a, &c)),
            Err(DeltaRejection::MalformedArgs)
        );
        assert!(links.is_empty(), "reduce must be atomic");
    }

    #[test]
    fn clearing_a_half_that_was_never_declared_fails_loudly() {
        let c = ctx();
        let mut links = BTreeMap::new();
        let a = clear_backlink_args(&"aa".repeat(32), "venue");
        assert_eq!(
            reduce_backlink(&mut links, &op(OP_CLEAR_BACKLINK, &a, &c)),
            Err(DeltaRejection::PreconditionFailed)
        );
    }

    #[test]
    fn a_malformed_object_id_moves_nothing() {
        let c = ctx();
        let mut links = BTreeMap::new();
        let mut a = crate::coordinator::Args::new();
        a.insert("object".into(), ArgVal::Text("not-hex".into()));
        a.insert("rel".into(), ArgVal::Text("venue".into()));
        a.insert("at".into(), ArgVal::Int(1));
        assert!(reduce_backlink(&mut links, &op(OP_SET_BACKLINK, &a, &c)).is_err());
        assert!(links.is_empty());
    }

    #[test]
    fn the_band_is_its_own() {
        assert_eq!(OP_SET_BACKLINK >> 16, 0xF009);
        assert!(!is_backlink_op(crate::parent::OP_SET_PARENT));
        assert!(!is_backlink_op(crate::parts::OP_SET_PART));
    }
    /// NC-81: a state holding its halves renders. The map is keyed by a pair, which
    /// serde_json refuses as an object key ("key must be a string"), so a post that
    /// declared its Site stopped rendering; the halves go as a list and come back.
    #[test]
    fn a_state_with_its_halves_renders_and_reads_back() {
        let c = ctx();
        let mut post = crate::post::PostState::default();
        for (rel, id) in [("created", "aa"), ("venue", "bb")] {
            let a = set_backlink_args(&id.repeat(32), rel, 7);
            reduce_backlink(&mut post.backlinks, &op(OP_SET_BACKLINK, &a, &c)).unwrap();
        }
        let json = serde_json::to_string(&post).expect("a post with its halves serializes");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["backlinks"], listed(&post.backlinks), "one list, as every view writes it");
        assert_eq!(v["backlinks"].as_array().map(Vec::len), Some(2));
        assert!(json.contains(&"aa".repeat(32)) && json.contains("\"rel\":\"created\""), "{json}");
        let back: crate::post::PostState = serde_json::from_str(&json).expect("and reads back");
        assert_eq!(back.backlinks, post.backlinks);
    }
}
