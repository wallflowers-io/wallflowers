//! parent — the other half of `part_of`, written in the PART.
//!
//! ## Why a part must name its parent
//!
//! `parts` writes the edge in the parent: a Post says which Forum is its comments
//! section. That is one-sided, and one-sided breaks traversal. A member of a
//! role-gated comments Forum who is NOT on the parent's roster holds an object that
//! cannot say what it is the comments section OF — the spine enumerates it, the fold
//! is silent, and the object is contextless. A person's WallFlowers is the subgraph
//! they can walk, so an edge that cannot be walked from one end is not in it.
//!
//! Ruled 25 September 2026: every relation is declared at BOTH ends. This is the
//! first half of that ruling in code.
//!
//! ## Who writes it, and why there is no race
//!
//! `mint` does, in the same act that mints the part. At mint time the parent's owner
//! is the sole member and owner of the part, so ONE principal authors both halves and
//! nothing has to agree with anything. That is what eager minting bought: the lazy
//! alternative leaves the part unminted until somebody acts, and then WHO mints it is
//! a race — the one `conversation_open` had to settle by naming a side.
//!
//! ## The facet pattern
//!
//! A type that can BE a part carries `parent: Option<ParentRef>` in its `State`,
//! includes [`PARENT_OPS`] in its `ops()`, and delegates to [`reduce_parent`]. The
//! reserved band is `0xF008`; `0xF004` (wallet) and `0xF005` (note) are VACATED by
//! the move to kinds 31 and 32 and must not be reused.

use crate::object::{Authority, Commutativity, DeltaRejection, Op, OpDecl};
use crate::object_args::{req_hex_id, req_int, req_text};

/// Name the object this one is a part of, and in what role. The parent's half is
/// `base.setPart`; both are written by the same mint.
pub const OP_SET_PARENT: u32 = 0xF008_0000;
/// Detach. The parent is untouched — it clears its own half with `base.clearPart`.
pub const OP_CLEAR_PARENT: u32 = 0xF008_0001;

pub static PARENT_OPS: &[OpDecl] = &[
    OpDecl {
        op_id: OP_SET_PARENT,
        name: "base.setParent",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_CLEAR_PARENT,
        name: "base.clearParent",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
];

/// Is this one of the parent ops?
pub fn is_parent_op(op_id: u32) -> bool {
    op_id == OP_SET_PARENT || op_id == OP_CLEAR_PARENT
}

/// What a part knows about the object it constitutes.
///
/// `role` is the parent's word for this part — "comments", "treasury", "notes" —
/// and matches the `role` in the parent's own `parts` declaration. Holding it here
/// means a reader who has only the part can say what it is FOR, not merely what it
/// is attached to.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ParentRef {
    pub parent: String,
    pub role: String,
    pub at: i64,
}

/// The one reducer, over whatever State holds the back-edge.
///
/// ONE parent, wholesale replaced. A part belongs to exactly one object: two parents
/// would make `part_of` ambiguous from this end, which is the ambiguity the ruling
/// exists to remove.
pub fn reduce_parent(
    slot: &mut Option<ParentRef>,
    op: &Op<'_>,
) -> Result<(), DeltaRejection> {
    match op.op_id {
        OP_SET_PARENT => {
            // Validate every arg before the first mutation — reduce is atomic.
            let parent = req_hex_id(op.args, "parent")?;
            let role = req_text(op.args, "role")?.to_string();
            let at = req_int(op.args, "at")?;
            *slot = Some(ParentRef { parent, role, at });
            Ok(())
        }
        OP_CLEAR_PARENT => {
            // Detaching from a parent that was never set fails loudly, as
            // `clearPart` and `clearAffiliation` do.
            slot.take().ok_or(DeltaRejection::PreconditionFailed)?;
            Ok(())
        }
        _ => Err(DeltaRejection::UnknownType),
    }
}

/// The args `base.setParent` carries.
pub fn set_parent_args(parent_id_hex: &str, role: &str, at_ms: i64) -> crate::coordinator::Args {
    let mut a = crate::coordinator::Args::new();
    a.insert("parent".into(), crate::coordinator::ArgVal::Text(parent_id_hex.to_string()));
    a.insert("role".into(), crate::coordinator::ArgVal::Text(role.to_string()));
    a.insert("at".into(), crate::coordinator::ArgVal::Int(at_ms));
    a
}

/// The args `base.clearParent` carries.
pub fn clear_parent_args(parent_id_hex: &str) -> crate::coordinator::Args {
    let mut a = crate::coordinator::Args::new();
    a.insert("parent".into(), crate::coordinator::ArgVal::Text(parent_id_hex.to_string()));
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

    fn op<'a>(op_id: u32, args: &'a crate::coordinator::Args, who: &'a MemberId,
              c: &'a ReduceContext<'a>) -> Op<'a> {
        Op { op_id, args, author: who, pos: None, ctx: c }
    }

    #[test]
    fn a_part_names_its_parent_and_its_role() {
        let (me, c) = (ME, ctx());
        let mut slot = None;
        let a = set_parent_args(&"aa".repeat(16), "comments", 7);
        reduce_parent(&mut slot, &op(OP_SET_PARENT, &a, &me, &c)).unwrap();
        let got = slot.expect("set");
        assert_eq!(got.role, "comments");
        assert_eq!(got.at, 7);
    }

    /// One parent. A second set REPLACES rather than accumulating, because a part
    /// belonging to two objects makes `part_of` ambiguous from this end.
    #[test]
    fn setting_a_second_parent_replaces_the_first() {
        let (me, c) = (ME, ctx());
        let mut slot = None;
        let a = set_parent_args(&"aa".repeat(16), "comments", 1);
        reduce_parent(&mut slot, &op(OP_SET_PARENT, &a, &me, &c)).unwrap();
        let b = set_parent_args(&"bb".repeat(16), "notes", 2);
        reduce_parent(&mut slot, &op(OP_SET_PARENT, &b, &me, &c)).unwrap();
        let got = slot.expect("set");
        assert_eq!(got.role, "notes");
        assert_eq!(got.parent, "bb".repeat(16));
    }

    #[test]
    fn clearing_a_parent_that_was_never_set_fails_loudly() {
        let (me, c) = (ME, ctx());
        let mut slot = None;
        let a = clear_parent_args(&"aa".repeat(16));
        assert_eq!(
            reduce_parent(&mut slot, &op(OP_CLEAR_PARENT, &a, &me, &c)),
            Err(DeltaRejection::PreconditionFailed)
        );
    }

    #[test]
    fn a_malformed_parent_id_is_refused_before_anything_moves() {
        let (me, c) = (ME, ctx());
        let mut slot = None;
        let mut a = crate::coordinator::Args::new();
        a.insert("parent".into(), ArgVal::Text("not-hex".into()));
        a.insert("role".into(), ArgVal::Text("comments".into()));
        a.insert("at".into(), ArgVal::Int(1));
        assert!(reduce_parent(&mut slot, &op(OP_SET_PARENT, &a, &me, &c)).is_err());
        assert!(slot.is_none(), "reduce must be atomic");
    }

    /// The band is reserved and the vacated ones stay vacated.
    #[test]
    fn the_band_is_its_own() {
        assert_eq!(OP_SET_PARENT >> 16, 0xF008);
        assert_eq!(OP_CLEAR_PARENT >> 16, 0xF008);
        assert!(!is_parent_op(crate::parts::OP_SET_PART));
    }
}
