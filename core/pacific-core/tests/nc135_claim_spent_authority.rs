//! NC-135: base.claimSpent's authority. The ICD (facets.membership) says `owner|role:admitter`;
//! the op table said `member`. The fold already held the author to the owner or an admitter,
//! so the table was the one that disagreed.

use pacific_core::authoring::{self, Ctx};
use pacific_core::coordinator::{ArgVal, Args, Coordinator};
use pacific_core::group::{GroupRole, GroupType};
use pacific_core::membership::{MEMBERSHIP_OPS, OP_CLAIM_SPENT};
use pacific_core::object::{build_delta, MemberId, ObjectKind, ObjectType};
use pacific_core::roles::{set_role_args, OP_SET_ROLE};
use serde_json::Value;

const OWNER: MemberId = [1; 32];
const ADMITTER: MemberId = [2; 32];
const MEMBER: MemberId = [3; 32];
const VISITOR: MemberId = [4; 32];

fn icd() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("the ICD")).expect("the ICD is JSON")
}

/// The ego the ICD gives base.claimSpent.
fn ego() -> String {
    icd()["facets"]["membership"]["ops"]["base.claimSpent"]["ego"].as_str().expect("the ICD declares base.claimSpent").to_string()
}

#[test]
fn the_declared_authority_is_the_icds() {
    let group = GroupType::op(OP_CLAIM_SPENT).expect("a group declares base.claimSpent");
    assert_eq!(group.authority.ego(), ego(), "GROUP_OPS");
    let base = MEMBERSHIP_OPS.iter().find(|d| d.op_id == OP_CLAIM_SPENT).expect("the membership facet declares it");
    assert_eq!(base.authority.ego(), ego(), "MEMBERSHIP_OPS");
}

/// What the authority says, the fold does: the owner's spend and an admitter's fold, a plain
/// member's does not.
#[test]
fn only_the_owner_or_an_admitter_spends_a_claim() {
    let members = [OWNER, ADMITTER, MEMBER, VISITOR];
    let owners = [(0, OWNER)];
    let ctx = Ctx { me: OWNER, epoch: 0, members: &members, owners: &owners, log: &[], watermark: Some(0) };
    let grant = authoring::build("group", OP_SET_ROLE, set_role_args(&ADMITTER, GroupRole::Admitter), &ctx).expect("the owner grants admitter");

    let spend = |n: u8| {
        let mut a = Args::new();
        a.insert("claim".into(), ArgVal::Text(hex::encode([n; 32])));
        a.insert("member".into(), ArgVal::Text(hex::encode(VISITOR)));
        a.insert("at".into(), ArgVal::Int(1_000));
        a.insert("gen".into(), ArgVal::Int(n as i64));
        build_delta(ObjectKind::Group, OP_CLAIM_SPENT, a, 0, Some(n as u64))
    };
    let mut c = Coordinator::<GroupType>::with_owners(members.to_vec(), owners.to_vec());
    c.deliver(grant, OWNER).unwrap();
    for (n, by) in [(1u8, OWNER), (2, ADMITTER), (3, MEMBER)] {
        c.deliver(spend(n), by).unwrap();
    }
    let spent = c.state().claims_spent;
    assert!(spent.contains_key(&hex::encode([1u8; 32])), "the owner's");
    assert!(spent.contains_key(&hex::encode([2u8; 32])), "an admitter's");
    assert!(!spent.contains_key(&hex::encode([3u8; 32])), "a plain member's is inert");
}
