//! NC-144 (CYP3's finding, 1 Oct): a Group's view did not serve its `parent`. GroupState folds
//! base.setParent, and the ICD's `parent` facet lists group in `on`, so a sub-group read on its own
//! must say what it is part of (`reciprocity`, ruled 25 Sep): the edge walks from either end. Make
//! admin's second step (node.rs add_part_member) reads view["parent"]["parent"] for the same reason.

use crate::common;

use common::Harness;
use pacific_core::authoring::{self, Ctx};
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::parent::OP_SET_PARENT;
use serde_json::Value;

const OWNER: [u8; 32] = [7; 32];

fn group_view(log: &[(&[u8; 32], Vec<u8>)]) -> Value {
    let view = pacific_core::fold::view_of("group", OWNER, vec![OWNER], vec![(0, OWNER)], log.iter().map(|(a, d)| (**a, d.clone())), Some(&OWNER))
        .expect("the group folds");
    serde_json::from_str(&view).unwrap()
}

/// A sub-group that names its Site serves that half in its own view, as a Forum's and a Host's do.
#[test]
fn a_part_group_serves_its_parent() {
    let site = hex::encode([0xaau8; 32]);
    let mut a = Args::new();
    a.insert("parent".into(), ArgVal::Text(site.clone()));
    a.insert("role".into(), ArgVal::Text("grove".into()));
    a.insert("at".into(), ArgVal::Int(1));
    let ctx = Ctx { me: OWNER, epoch: 0, members: &[OWNER], owners: &[(0, OWNER)], log: &[], watermark: Some(0) };
    let d = authoring::build("group", OP_SET_PARENT, a, &ctx).expect("the part's half is written");
    let v = group_view(&[(&OWNER, d.canonical_bytes())]);
    assert!(v.as_object().unwrap().contains_key("parent"), "a Group's view carries `parent`: {v}");
    assert_eq!(v["parent"]["parent"], site.as_str(), "the Site it is part of");
    assert_eq!(v["parent"]["role"], "grove");
    assert_eq!(v["parent"]["at"], 1);
}

/// A Group that is part of nothing says so: `parent` is null, not absent.
#[test]
fn a_whole_group_serves_a_null_parent() {
    let v = group_view(&[]);
    assert!(v.as_object().unwrap().contains_key("parent"), "a Group's view carries `parent`: {v}");
    assert!(v["parent"].is_null());
}

fn args(pairs: &[(&str, ArgVal)]) -> Args {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

/// Make admin's second step (`Node::add_part_member`) reaches a sub-group part as it reaches a
/// Host: the part names its Site in its own view, so a Site member who is a contact is added.
/// Before NC-144 it refused every Group part with "<id> is a part of no Site".
#[tokio::test]
async fn make_admins_second_step_reaches_a_sub_group_part() {
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let site = h.mint_group(ada);
    h.group_set_profile(ada, &site, "Mill Road", "community").await;
    h.add_to_forum(ada, bo, &site).await;
    let sub = h.mint_group(ada);
    h.node(ada)
        .apply(&site, pacific_core::parts::OP_SET_PART, args(&[("part", ArgVal::Text(sub.clone())), ("role", ArgVal::Text("grove".into())), ("at", ArgVal::Int(1))]))
        .await
        .expect("the Site's half");
    h.node(ada)
        .apply(&sub, OP_SET_PARENT, args(&[("parent", ArgVal::Text(site.clone())), ("role", ArgVal::Text("grove".into())), ("at", ArgVal::Int(1))]))
        .await
        .expect("the part's half");
    h.pair(ada, bo).await;
    h.settle().await;
    h.node(ada).add_part_member(&sub, &h.id(bo)).await.expect("bo is on the Site and ada's contact");
    h.settle().await;
    assert!(h.roster(ada, &sub).contains(&h.id(bo)), "bo is on the sub-group's roster");
}
