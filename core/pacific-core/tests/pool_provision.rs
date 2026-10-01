//! Provisioning a pool leaf (resumption.md §5): one Add of the person's own leaf,
//! and a way-in on the spine that a device with nothing can join through.

mod common;

use common::Harness;
use pacific_core::mls;
use pacific_core::mls_mem::{build_client_mem, Snapshot};
use pacific_core::spine::Body;

#[tokio::test]
async fn a_pool_leaf_is_one_add_of_ones_own_and_its_way_in_joins() {
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let obj = h.form_forum(ada, &[bo]).await;
    h.settle().await; // upkeep has already provisioned one; this is a second, on demand
    let (leaves0, people0, e0) = (h.leaves(ada, &obj), h.roster_people(ada, &obj), h.epoch(ada, &obj));

    let o = obj.clone();
    let leaf = h.with(ada, |n| async move { n.pool_provision(&o).await.unwrap() }).await;
    h.settle().await;

    assert_eq!(h.leaves(ada, &obj), leaves0 + 1, "one more leaf");
    assert_eq!(h.roster_people(ada, &obj), people0, "and no more people");
    assert_eq!(h.epoch(ada, &obj), e0 + 1, "one epoch");
    assert_eq!(h.epoch(bo, &obj), e0 + 1, "bo followed it");
    assert_eq!(leaf.epoch, e0 + 1, "the leaf joined at the Add's epoch");

    // The way-in is on the account's chain.
    let gid = hex::decode(&obj).unwrap();
    let walk = h.with(ada, |n| async move { n.chain_walk(0).await.unwrap() }).await;
    let pools = walk
        .entries
        .iter()
        .rev()
        .find_map(|(_, e)| match &e.body {
            Body::Pool(p) if p.group_id == gid => Some(p.pools.clone()),
            _ => None,
        })
        .expect("a Pool entry for the object");
    assert!(pools.iter().any(|p| p.pool == leaf.pool), "the way-in names the new leaf");

    // And the material in it is enough to get in from nothing.
    let (gss, kps) = Snapshot { states: vec![], epochs: vec![], key_packages: vec![(leaf.kp_id.clone(), leaf.kp_data.clone())] }.restore();
    let client = build_client_mem(
        gss,
        kps,
        mls::signing_identity(&h.id(ada), &leaf.pool),
        mls::SecretKey::from(leaf.sig_sk.clone()),
    )
    .unwrap();
    let g = mls::join_group(&client, &leaf.welcome).expect("the way-in joins");
    assert_eq!(g.current_epoch(), leaf.epoch);
}
