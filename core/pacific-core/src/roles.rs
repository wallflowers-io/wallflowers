//! roles — the STANDING facet: what one member of an object is to it.
//!
//! ## What it is for
//!
//! `group.setMemberRole` was a Group op, so a role could only ride a log folded
//! as GroupType. That is the entire reason four kind strings existed for one
//! thing: `connection` (a person, folded as Contact) could not carry a role, so
//! `member-tether` and `arc-tether` were minted as Group-typed twins of it —
//! identical 2-member objects differing only in which vocabulary their log spoke.
//! The membership plane says so itself: *"A `member-tether`, NOT a `connection`:
//! membership carries the member's ROLE, and a role must ride a log folded as
//! GroupType."*
//!
//! Ruled 24 September 2026: standing is a facet, so any kind may carry it, and a
//! connection is just a connection. What differed between the tethers was never
//! the object — it was what the other party is to us, which is the role.
//!
//! ## The facet pattern, as `geo` states it
//!
//! A type that carries standing holds `roles: BTreeMap<MemberId, Role>` in its
//! `State`, includes [`ROLE_OPS`] in its `ops()`, and delegates to
//! [`reduce_roles`]. Base ops take a reserved high op-id band; this one is
//! `0xF007`.

use std::collections::BTreeMap;

use crate::group::GroupRole;
use crate::object::{Authority, Commutativity, DeltaRejection, MemberId, Op, OpDecl};
use crate::object_args::req_text;

/// Set one member's standing on this object. Owner-sequenced: standing is the
/// owner's to grant, and a concurrent pair would otherwise race.
pub const OP_SET_ROLE: u32 = 0xF007_0000;
/// Take it away. The member stays on the roster — the MLS tree is the roster, and
/// a role is an overlay on it.
pub const OP_CLEAR_ROLE: u32 = 0xF007_0001;

pub static ROLE_OPS: &[OpDecl] = &[
    OpDecl {
        op_id: OP_SET_ROLE,
        name: "base.setRole",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_CLEAR_ROLE,
        name: "base.clearRole",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
];

pub fn is_role_op(op_id: u32) -> bool {
    op_id == OP_SET_ROLE || op_id == OP_CLEAR_ROLE
}

/// The one reducer, over whatever State holds the standings.
///
/// `member` is the identity key the standing is about, as `space1<hex>` or bare
/// hex — the wire has carried both, and the older `group.setMemberRole` took the
/// prefixed form in an arg it called `space`.
pub fn reduce_roles(
    roles: &mut BTreeMap<MemberId, GroupRole>,
    op: &Op<'_>,
) -> Result<(), DeltaRejection> {
    let who = member_id(req_text(op.args, "member")?)?;
    match op.op_id {
        OP_SET_ROLE => {
            let role = GroupRole::parse(req_text(op.args, "role")?)
                .map_err(|_| DeltaRejection::MalformedArgs)?;
            // A deal's roles are a Transaction's alone (`reduce_fixed_roles`).
            if role.is_deal() {
                return Err(DeltaRejection::MalformedArgs);
            }
            // A ROLE OVERLAYS A REAL ROSTER MEMBER — never confers membership.
            // The MLS tree is the roster and this is an overlay on it; a standing
            // granted to someone who is not in the tree would be an access claim
            // with nothing behind it.
            if !op.ctx.is_member(&who) {
                return Err(DeltaRejection::PreconditionFailed);
            }
            roles.insert(who, role);
            Ok(())
        }
        OP_CLEAR_ROLE => {
            roles.remove(&who).ok_or(DeltaRejection::PreconditionFailed)?;
            Ok(())
        }
        _ => Err(DeltaRejection::UnknownType),
    }
}

/// A TRANSACTION'S STANDING (W-98 Trade; ICD `kinds.transaction.roles`): its three deal
/// roles, fixed for its lifecycle (T-2). Each is set once, one role a member, the settler
/// being the owner; none is ever cleared; and none is set once the object has folded an op
/// that is not a role (`began`, the caller's).
pub fn reduce_fixed_roles(
    roles: &mut BTreeMap<MemberId, GroupRole>,
    op: &Op<'_>,
    began: bool,
) -> Result<(), DeltaRejection> {
    match op.op_id {
        OP_SET_ROLE => {
            let who = member_id(req_text(op.args, "member")?)?;
            let role = GroupRole::parse(req_text(op.args, "role")?)?;
            if !role.is_deal() {
                return Err(DeltaRejection::MalformedArgs);
            }
            if began || !op.ctx.is_member(&who) || roles.contains_key(&who) || roles.values().any(|r| *r == role) {
                return Err(DeltaRejection::PreconditionFailed);
            }
            if (role == GroupRole::Settler) != (who == op.ctx.owner) {
                return Err(DeltaRejection::PreconditionFailed);
            }
            roles.insert(who, role);
            Ok(())
        }
        OP_CLEAR_ROLE => Err(DeltaRejection::PreconditionFailed),
        _ => Err(DeltaRejection::UnknownType),
    }
}

fn member_id(s: &str) -> Result<MemberId, DeltaRejection> {
    let hex_part = s.rsplit(|c: char| !c.is_ascii_hexdigit()).next().unwrap_or(s);
    let raw = hex::decode(hex_part).map_err(|_| DeltaRejection::MalformedArgs)?;
    <MemberId>::try_from(raw.as_slice()).map_err(|_| DeltaRejection::MalformedArgs)
}

/// The args `base.setRole` carries.
pub fn set_role_args(member: &MemberId, role: GroupRole) -> crate::coordinator::Args {
    let mut a = crate::coordinator::Args::new();
    a.insert("member".into(), crate::coordinator::ArgVal::Text(hex::encode(member)));
    a.insert("role".into(), crate::coordinator::ArgVal::Text(role.as_str().to_string()));
    a
}
