//! bundles — the BUNDLES facet: `base.offerBundle` / `base.withdrawBundle`, each member's own
//! offer of contact bundles in a Site.
//!
//! #12, connect from a member's card (ICD 2.4.0 `facets.bundles`, band `0xF00D`). Ruled 30 Sep:
//! a pending invitation stays pairwise and off the Site, so the Site carries only each member's
//! OFFER. A member who wants to connect takes one of the other's live bundles and pairs with it
//! (`Node::connect_member` → `pair_scan`); the invitation is that pairing's `contact.setLink`.
//!
//! An offer's bundle must be the AUTHOR'S (its identity) and its key package must hash to
//! `kp_id`, or it is refused: no member offers a way to reach someone else. A withdrawal is final
//! for its (author, kp_id) — a key package is used once — so offer and withdrawal commute.
//!
//! The facet pattern, as [`crate::about`]: a type holds `bundles: Bundles`, splices
//! [`BUNDLE_OPS`] into its `ops()` and delegates to [`reduce_bundles`].

use std::collections::{BTreeMap, BTreeSet};

use crate::object::{Authority, Commutativity, DeltaRejection, MemberId, Op, OpDecl};
use crate::object_args::{req_int, req_text};

pub const OP_OFFER_BUNDLE: u32 = 0xF00D_0000;
pub const OP_WITHDRAW_BUNDLE: u32 = 0xF00D_0001;

pub static BUNDLE_OPS: &[OpDecl] = &[
    OpDecl {
        op_id: OP_OFFER_BUNDLE,
        name: "base.offerBundle",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_WITHDRAW_BUNDLE,
        name: "base.withdrawBundle",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
];

pub fn is_bundle_op(op_id: u32) -> bool {
    op_id == OP_OFFER_BUNDLE || op_id == OP_WITHDRAW_BUNDLE
}

/// One offer: the bundle and the key package's expiry.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Offer {
    pub bundle: String,
    pub not_after: u64,
}

/// Every member's offers and withdrawals, by (author, kp_id).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Bundles {
    offers: BTreeMap<(MemberId, [u8; 32]), Offer>,
    withdrawn: BTreeSet<(MemberId, [u8; 32])>,
}

impl Bundles {
    /// `member`'s live offers: offered and not withdrawn, expired or not.
    pub fn live(&self, member: &MemberId) -> impl Iterator<Item = ([u8; 32], &Offer)> + '_ {
        let m = *member;
        self.offers
            .range((m, [0u8; 32])..=(m, [0xFF; 32]))
            .filter(move |(k, _)| !self.withdrawn.contains(k))
            .map(|((_, kp), o)| (*kp, o))
    }
}

fn kp_arg(op: &Op<'_>) -> Result<[u8; 32], DeltaRejection> {
    let s = req_text(op.args, "kp_id")?;
    let v = hex::decode(s).map_err(|_| DeltaRejection::MalformedArgs)?;
    v.try_into().map_err(|_| DeltaRejection::MalformedArgs)
}

/// `base.offerBundle`'s args; the write path stamps `gen`.
pub fn offer_args(kp_id: &[u8; 32], bundle: &str, not_after: u64) -> crate::coordinator::Args {
    use crate::coordinator::ArgVal;
    crate::coordinator::Args::from([
        ("kp_id".into(), ArgVal::Text(hex::encode(kp_id))),
        ("bundle".into(), ArgVal::Text(bundle.into())),
        ("not_after".into(), ArgVal::Int(not_after as i64)),
    ])
}

/// `base.withdrawBundle`'s args; the write path stamps `gen`.
pub fn withdraw_args(kp_id: &[u8; 32]) -> crate::coordinator::Args {
    crate::coordinator::Args::from([("kp_id".into(), crate::coordinator::ArgVal::Text(hex::encode(kp_id)))])
}

/// The one reducer, over whatever State holds the facet. The author is the subject.
pub fn reduce_bundles(b: &mut Bundles, op: &Op<'_>) -> Result<(), DeltaRejection> {
    // Validate every arg before the first mutation — reduce is atomic.
    let kp = kp_arg(op)?;
    req_int(op.args, "gen")?;
    let key = (*op.author, kp);
    match op.op_id {
        OP_OFFER_BUNDLE => {
            let bundle = req_text(op.args, "bundle")?;
            let not_after = u64::try_from(req_int(op.args, "not_after")?).map_err(|_| DeltaRejection::MalformedArgs)?;
            let parsed = crate::handshake::parse_and_verify(bundle).map_err(|_| DeltaRejection::MalformedArgs)?;
            if parsed.identity_pk != *op.author || crate::contact::kp_id(&parsed.key_package) != kp {
                return Err(DeltaRejection::MalformedArgs);
            }
            if !op.ctx.is_member(op.author) {
                return Err(DeltaRejection::Unauthorized);
            }
            b.offers.insert(key, Offer { bundle: bundle.to_string(), not_after });
        }
        OP_WITHDRAW_BUNDLE => {
            if !op.ctx.is_member(op.author) {
                return Err(DeltaRejection::Unauthorized);
            }
            b.withdrawn.insert(key);
        }
        _ => return Err(DeltaRejection::UnknownType),
    }
    Ok(())
}

/// The view's `bundles`: `{<member key>: [{kp_id, bundle, not_after}]}`, live offers of members
/// on `roster` only.
pub fn view(b: &Bundles, roster: &[MemberId]) -> serde_json::Value {
    serde_json::Value::Object(
        roster
            .iter()
            .filter_map(|m| {
                let live: Vec<_> = b
                    .live(m)
                    .map(|(kp, o)| serde_json::json!({ "kp_id": hex::encode(kp), "bundle": o.bundle, "not_after": o.not_after }))
                    .collect();
                (!live.is_empty()).then(|| (hex::encode(m), serde_json::Value::Array(live)))
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::coordinator::{ArgVal, Args};
    use crate::object::ReduceContext;

    fn bundle_of(seed: u8, kp: &[u8]) -> String {
        let id = crate::identity::Identity::in_memory([seed; 32]);
        crate::handshake::build_bundle(&id, kp.to_vec(), [7; 32], "someone").unwrap().encode().unwrap()
    }

    fn pk(seed: u8) -> MemberId {
        crate::identity::Identity::in_memory([seed; 32]).identity_pk()
    }

    fn offer(bundle: &str, kp: &[u8]) -> Args {
        Args::from([
            ("kp_id".into(), ArgVal::Text(hex::encode(crate::contact::kp_id(kp)))),
            ("bundle".into(), ArgVal::Text(bundle.into())),
            ("not_after".into(), ArgVal::Int(2_000_000_000)),
            ("gen".into(), ArgVal::Int(1)),
        ])
    }

    fn withdraw(kp: &[u8]) -> Args {
        Args::from([("kp_id".into(), ArgVal::Text(hex::encode(crate::contact::kp_id(kp)))), ("gen".into(), ArgVal::Int(2))])
    }

    fn apply(b: &mut Bundles, op_id: u32, args: &Args, author: &MemberId, members: &[MemberId]) -> Result<(), DeltaRejection> {
        let ctx = ReduceContext { members, owner: members[0], epoch: 0 };
        reduce_bundles(b, &Op { op_id, args, author, pos: None, ctx: &ctx })
    }

    /// A member's own bundle is offered and drawn; another's, or one whose key package is not
    /// `kp_id`, is refused; an author off the roster offers nothing.
    #[test]
    fn only_the_authors_own_bundle_is_offered() {
        let (a, b, c) = (pk(1), pk(2), pk(3));
        let members = [a, b];
        let mut st = Bundles::default();
        apply(&mut st, OP_OFFER_BUNDLE, &offer(&bundle_of(1, b"kp-a"), b"kp-a"), &a, &members).unwrap();
        assert_eq!(apply(&mut st, OP_OFFER_BUNDLE, &offer(&bundle_of(1, b"kp-x"), b"kp-x"), &b, &members), Err(DeltaRejection::MalformedArgs));
        assert_eq!(apply(&mut st, OP_OFFER_BUNDLE, &offer(&bundle_of(2, b"kp-b"), b"other"), &b, &members), Err(DeltaRejection::MalformedArgs));
        assert_eq!(apply(&mut st, OP_OFFER_BUNDLE, &offer(&bundle_of(3, b"kp-c"), b"kp-c"), &c, &members), Err(DeltaRejection::Unauthorized));
        let v = view(&st, &members);
        assert_eq!(v.as_object().unwrap().len(), 1);
        assert_eq!(v[hex::encode(a)][0]["kp_id"], hex::encode(crate::contact::kp_id(b"kp-a")));
    }

    /// A withdrawal is final for its (author, kp_id), in either order; another member's
    /// withdrawal of the same kp_id withdraws nothing.
    #[test]
    fn offer_and_withdrawal_commute() {
        let (a, b) = (pk(1), pk(2));
        let members = [a, b];
        let (o, x) = (offer(&bundle_of(1, b"kp-a"), b"kp-a"), withdraw(b"kp-a"));
        let mut first = Bundles::default();
        apply(&mut first, OP_OFFER_BUNDLE, &o, &a, &members).unwrap();
        apply(&mut first, OP_WITHDRAW_BUNDLE, &x, &b, &members).unwrap();
        assert_eq!(first.live(&a).count(), 1, "b's withdrawal is not a's");
        apply(&mut first, OP_WITHDRAW_BUNDLE, &x, &a, &members).unwrap();
        let mut later = Bundles::default();
        apply(&mut later, OP_WITHDRAW_BUNDLE, &x, &a, &members).unwrap();
        apply(&mut later, OP_OFFER_BUNDLE, &o, &a, &members).unwrap();
        apply(&mut later, OP_WITHDRAW_BUNDLE, &x, &b, &members).unwrap();
        assert_eq!(first, later);
        assert_eq!(first.live(&a).count(), 0);
        assert_eq!(view(&first, &members), serde_json::json!({}));
    }

    /// The ops the Group splices into its table are `BUNDLE_OPS`' own.
    #[test]
    fn base_bundle_ops_are_spliced_verbatim() {
        use crate::object::ObjectType;
        let key = |d: &OpDecl| (d.op_id, d.name, d.authority, d.commutativity);
        for d in BUNDLE_OPS {
            assert_eq!(crate::group::GroupType::op(d.op_id).map(key), Some(key(d)), "{}", d.name);
        }
    }
}
