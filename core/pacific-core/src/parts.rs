//! parts — the COMPOSITION facet: `base.setPart` and `base.clearPart`.
//!
//! ## What it is for
//!
//! An object is made of other objects. A Post's comments section is a Forum; so is a
//! Group's room, and a room inside a channel. A Group's Treasury, its Host and a
//! sub-group are parts of it too. Each part is its OWN GroupObject with its own MLS
//! roster — which is the whole of access control — and the parent writes only the
//! edge: which object, in what role.
//!
//! Ruled 26 September 2026: composition is general, and there is one way to write
//! it. `base.setForum` and `forum.setRoom` were two ops for one fact, and neither
//! could say a Treasury or a Host was part of a Group. They are this facet now, on
//! the same wire ids.
//!
//! ## The other half
//!
//! The part names its parent with `base.setParent` (`crate::parent`), so the edge
//! walks from either end. `mint` writes both halves in one act, while the parent's
//! owner is the part's sole member and owner.
//!
//! ## The facet pattern, as `geo` states it
//!
//! A type that has parts carries `parts: BTreeMap<String, PartRef>` in its `State`,
//! includes the two ops in its `ops()`, and delegates them to [`reduce_parts`] from
//! its `reduce`. Base ops take a reserved high op-id band so they never collide with
//! a type's own ops, which number from 0; this one is `0xF006`.

use std::collections::BTreeMap;

use crate::object::{Authority, Commutativity, DeltaRejection, Op, OpDecl, PartRef};
use crate::object_args::{req_hex_id, req_int, req_text};

/// Name an object as a part of this one, in a role. Re-setting an existing part
/// replaces its role in place.
pub const OP_SET_PART: u32 = 0xF006_0000;
/// Detach it. The part OBJECT is untouched: this removes the edge, never the part
/// or its log.
pub const OP_CLEAR_PART: u32 = 0xF006_0001;

/// The role a Host plays in its Site. A Group with a part in this role is a Site,
/// and only a Site has a Face (A-11).
pub const HOST_ROLE: &str = "host";

/// A room: the one role a claim's choice picks among (ICD 2.1.0 row 3).
pub const ROOM_ROLE: &str = "room";

pub static PART_OPS: &[OpDecl] = &[
    OpDecl {
        op_id: OP_SET_PART,
        name: "base.setPart",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_CLEAR_PART,
        name: "base.clearPart",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
];

/// Is this one of the parts ops?
pub fn is_part_op(op_id: u32) -> bool {
    op_id == OP_SET_PART || op_id == OP_CLEAR_PART
}

/// The one reducer for both, over whatever State holds the edges.
pub fn reduce_parts(parts: &mut BTreeMap<String, PartRef>, op: &Op<'_>) -> Result<(), DeltaRejection> {
    match op.op_id {
        OP_SET_PART => {
            // Validate every arg before the first mutation — reduce is atomic.
            let part = req_hex_id(op.args, "part")?;
            let role = req_text(op.args, "role")?.to_string();
            if role.is_empty() {
                // A part with no role says nothing about what it is FOR.
                return Err(DeltaRejection::MalformedArgs);
            }
            let at = req_int(op.args, "at")?;
            // A room's claim choice: on a room only, and one a kiosk issues.
            let choice = match crate::arg_reads::get(op.args, "choice") {
                None => None,
                Some(crate::coordinator::ArgVal::Text(c)) if role == ROOM_ROLE && crate::claim::CHOICES.contains(&c.as_str()) => Some(c.clone()),
                Some(_) => return Err(DeltaRejection::MalformedArgs),
            };
            parts.insert(part, PartRef { role, at, choice });
            Ok(())
        }
        OP_CLEAR_PART => {
            let part = req_hex_id(op.args, "part")?;
            // Detaching a part that was never attached fails loudly, as
            // `clearAffiliation` does.
            parts.remove(&part).ok_or(DeltaRejection::PreconditionFailed)?;
            Ok(())
        }
        _ => Err(DeltaRejection::UnknownType),
    }
}

/// The args `base.setPart` carries.
pub fn set_part_args(part_id_hex: &str, role: &str, at_ms: i64) -> crate::coordinator::Args {
    let mut a = crate::coordinator::Args::new();
    a.insert("part".into(), crate::coordinator::ArgVal::Text(part_id_hex.to_string()));
    a.insert("role".into(), crate::coordinator::ArgVal::Text(role.to_string()));
    a.insert("at".into(), crate::coordinator::ArgVal::Int(at_ms));
    a
}

/// The args `base.clearPart` carries.
pub fn clear_part_args(part_id_hex: &str) -> crate::coordinator::Args {
    let mut a = crate::coordinator::Args::new();
    a.insert("part".into(), crate::coordinator::ArgVal::Text(part_id_hex.to_string()));
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{MemberId, ReduceContext};

    const ME: MemberId = [7u8; 32];
    static ROSTER: &[MemberId] = &[ME];
    const PART: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";

    fn run(parts: &mut BTreeMap<String, PartRef>, op_id: u32, args: &crate::coordinator::Args) -> Result<(), DeltaRejection> {
        let ctx = ReduceContext { members: ROSTER, owner: ME, epoch: 0 };
        reduce_parts(parts, &Op { op_id, args, author: &ME, pos: None, ctx: &ctx })
    }

    #[test]
    fn a_part_is_attached_in_a_role_and_re_set_in_place() {
        let mut parts = BTreeMap::new();
        run(&mut parts, OP_SET_PART, &set_part_args(PART, "treasury", 1)).unwrap();
        assert_eq!(parts[PART], PartRef { role: "treasury".into(), at: 1, choice: None });
        run(&mut parts, OP_SET_PART, &set_part_args(PART, "host", 2)).unwrap();
        assert_eq!(parts.len(), 1, "one part, one edge");
        assert_eq!(parts[PART].role, "host");
    }

    #[test]
    fn a_part_with_no_role_is_refused() {
        let mut parts = BTreeMap::new();
        assert_eq!(run(&mut parts, OP_SET_PART, &set_part_args(PART, "", 1)), Err(DeltaRejection::MalformedArgs));
        assert!(parts.is_empty(), "reduce must be atomic");
    }

    #[test]
    fn clearing_a_part_never_attached_fails_loudly() {
        let mut parts = BTreeMap::new();
        run(&mut parts, OP_SET_PART, &set_part_args(PART, "room", 1)).unwrap();
        run(&mut parts, OP_CLEAR_PART, &clear_part_args(PART)).unwrap();
        assert!(parts.is_empty());
        assert_eq!(run(&mut parts, OP_CLEAR_PART, &clear_part_args(PART)), Err(DeltaRejection::PreconditionFailed));
    }

    /// ICD 2.1.0 row 3: a room names the claim choice it takes; no other role may, and a
    /// choice no kiosk issues is refused. Re-setting the room without one clears it.
    #[test]
    fn a_room_carries_a_claim_choice_and_nothing_else_does() {
        let with = |role: &str, c: &str| {
            let mut a = set_part_args(PART, role, 1);
            a.insert("choice".into(), crate::coordinator::ArgVal::Text(c.into()));
            a
        };
        let mut parts = BTreeMap::new();
        run(&mut parts, OP_SET_PART, &with("room", "skills")).unwrap();
        assert_eq!(parts[PART].choice.as_deref(), Some("skills"));
        assert_eq!(run(&mut parts, OP_SET_PART, &with("treasury", "skills")), Err(DeltaRejection::MalformedArgs));
        assert_eq!(run(&mut parts, OP_SET_PART, &with("room", "time")), Err(DeltaRejection::MalformedArgs));
        assert_eq!(parts[PART].choice.as_deref(), Some("skills"), "a refusal changes nothing");
        run(&mut parts, OP_SET_PART, &set_part_args(PART, "room", 2)).unwrap();
        assert_eq!(parts[PART].choice, None);
    }

    /// The wire ids `base.setForum` held: taken over, not re-numbered (ruled 26 Sep
    /// 2026, before any user data existed).
    #[test]
    fn the_band_is_the_one_setforum_held() {
        assert_eq!((OP_SET_PART, OP_CLEAR_PART), (0xF006_0000, 0xF006_0001));
    }
}
