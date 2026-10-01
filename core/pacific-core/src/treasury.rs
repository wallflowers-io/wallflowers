//! treasury — a shared treasury as a GroupObject of its own (kind 31).
//!
//! ## Why it is a kind and not a facet on `group`
//!
//! It was four `base.wallet*` ops on `group`, and `shared-wallets.html` §30 refused a
//! `Wallet(31)` kind on the grounds that it "would buy a finance sub-group narrower
//! than the site, at the price of a second roster that can silently diverge from it".
//!
//! §104 of that same brief named the condition that would reverse it: *"if you
//! genuinely need a member of the site who cannot see the money… that is the one
//! scenario that would justify promoting this to a `Wallet(31)` kind with its own
//! narrower roster."* Ruled 25 September 2026: multiple membership per node is that
//! scenario stated as a rule rather than met case by case, so a Treasury is a
//! GroupObject and its roster is its own. A member of the body need not be a member
//! of the money.
//!
//! ## What is reused, and what is not
//!
//! The STATE and the REDUCER are [`crate::wallet`]'s, unchanged — the policy bands,
//! the deposit ledger, the settlement and balance attestations and their divergence
//! check are a working implementation and this is not a rewrite of them. What changes
//! is where they live: the ops are this kind's own table, numbered from 0, instead of
//! a reserved band spliced onto somebody else's log.
//!
//! ## The treasurer check, and the ruling that is coming for it
//!
//! `reduce_wallet` takes `is_treasurer` as a parameter because, in `group.rs`'s own
//! words, it is "the one base facet that needs to know a ROLE, not just a member —
//! `Authority` has no rung for Admin, so the treasurer check is a precondition inside
//! the reducer". That workaround is exactly what role-valued `ego` was ruled to
//! replace (25 September). Until `authoring::build` resolves `ego: "role:treasurer"`
//! the precondition stays where it is, and this module passes the flag through rather
//! than inventing a second answer.

use crate::object::{
    Authority, Commutativity, DeltaRejection, ObjectKind, ObjectType, Op, OpDecl,
};
use crate::wallet;

/// Open or amend the policy: the bands, the rule, the disclosure.
pub const OP_SET_POLICY: u32 = 0;
/// Money in, recorded by a member.
pub const OP_RECORD_DEPOSIT: u32 = 1;
/// A member's attestation that a settlement happened.
pub const OP_ATTEST_SETTLEMENT: u32 = 2;
/// A member's attestation of the balance they observe.
pub const OP_ATTEST_BALANCE: u32 = 3;

/// This kind's own table, numbered from 0. The `0xF004` band it used to occupy is
/// VACATED and must not be reused: an old delta carrying one of those ids has to
/// fail loud rather than fold as something else.
pub static OPS: &[OpDecl] = &[
    OpDecl {
        op_id: OP_SET_POLICY,
        name: "treasury.setPolicy",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_RECORD_DEPOSIT,
        name: "treasury.recordDeposit",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_ATTEST_SETTLEMENT,
        name: "treasury.attestSettlement",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_ATTEST_BALANCE,
        name: "treasury.attestBalance",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
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

/// This kind's op id → the facet id `wallet`'s reducer still matches on.
///
/// The two tables are related by arithmetic, not by a second list: `wallet`'s band is
/// contiguous from `OP_SET_POLICY`, and so is this one, so the mapping is the offset.
/// A transcribed pair table would be four numbers to keep in step with two others.
const fn to_facet_op(op_id: u32) -> Option<u32> {
    if op_id <= OP_ATTEST_BALANCE {
        Some(wallet::OP_SET_POLICY + op_id)
    } else {
        None
    }
}

pub struct TreasuryType;

impl ObjectType for TreasuryType {
    const KIND: ObjectKind = ObjectKind::Treasury;
    type State = wallet::Wallet;

    fn ops() -> &'static [OpDecl] {
        OPS
    }

    fn reduce(state: &mut Self::State, op: &Op<'_>) -> Result<(), DeltaRejection> {
        if crate::parent::is_parent_op(op.op_id) {
            return crate::parent::reduce_parent(&mut state.parent, op);
        }
        let facet_op = to_facet_op(op.op_id).ok_or(DeltaRejection::UnknownType)?;
        // A Treasury's roster IS the money's roster — everyone on it is a treasurer,
        // which is the whole point of giving it one narrower than the body's. On a
        // Group the flag distinguished Admin from Member; here there is nobody on
        // the roster who should not be trusted with the money, because the way you
        // are not trusted with it is not being on this object at all.
        let restated = Op {
            op_id: facet_op,
            args: op.args,
            author: op.author,
            pos: op.pos,
            ctx: op.ctx,
        };
        wallet::reduce_wallet(state, &restated, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_contiguous_from_zero_and_maps_onto_the_facet() {
        // The kind's OWN ops, below the base band; the rest are the parent facet's.
        let own: Vec<_> = OPS.iter().filter(|d| !crate::parent::is_parent_op(d.op_id)).collect();
        for (i, decl) in own.iter().enumerate() {
            assert_eq!(decl.op_id, i as u32, "{} is out of place", decl.name);
            assert_eq!(
                to_facet_op(decl.op_id),
                Some(wallet::OP_SET_POLICY + i as u32),
                "{} does not map onto the facet id its reducer matches",
                decl.name
            );
        }
        assert_eq!(OPS.len(), own.len() + crate::parent::PARENT_OPS.len(), "an op is neither the kind's nor the parent facet's");
        assert_eq!(to_facet_op(own.len() as u32), None, "the map must not run past the table");
    }

    #[test]
    fn every_op_this_kind_declares_is_one_the_facet_reducer_knows() {
        for decl in OPS {
            if crate::parent::is_parent_op(decl.op_id) {
                continue; // reduced by the parent facet, ahead of the wallet map
            }
            let facet = to_facet_op(decl.op_id).expect("mapped");
            assert!(
                wallet::is_wallet_op(facet),
                "{} maps to {facet:#x}, which the wallet reducer does not accept",
                decl.name
            );
        }
    }

    /// The vacated band must stay vacated. If `wallet`'s ids are ever reused for
    /// something else, an old Treasury delta would fold as that something.
    #[test]
    fn the_facet_band_is_still_the_one_this_maps_onto() {
        assert_eq!(wallet::OP_SET_POLICY, 0xF004_0000);
        assert_eq!(wallet::OP_ATTEST_BALANCE, 0xF004_0003);
    }
}
