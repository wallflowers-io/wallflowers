//! NC-65's guard. A device that joined an object after it began, and was handed no
//! restatement of it, folds what it cannot see as empty. A sequenced op from it chains to
//! nothing and breaks the chain for every replica that holds it (ChainBroken); after
//! that the Arc's node failed every admission. So core refuses, and says why.
//!
//! The Door's fresh session is that device: it joins through a pool leaf, and nobody
//! restates the object for it. A device the owner adds is not: the Add re-emits the
//! owner's spine at the newcomer's epoch (`reemit_own_spine`), so it holds the chain.

mod common;

use common::Harness;
use pacific_core::group::OP_SET_FACE;
use pacific_core::resumption::{HeadInput, Outcome};

const HOST: &str = "arc.example";
const PRF: [u8; 32] = [0x11; 32];
const WHY: &str = "this device joined after this object began, and can't see its current state; \
                   sign in on a device that holds it";

fn face(doc: &str) -> pacific_core::coordinator::Args {
    let mut a = pacific_core::coordinator::Args::new();
    a.insert("face".into(), pacific_core::coordinator::ArgVal::Text(doc.into()));
    a
}

/// Ada's Site, with Bo in it and its Host, its state published as it is written: a Site
/// with members sends each op at the epoch it is written in, and a later leaf holds none
/// of it.
async fn a_site(h: &Harness, phone: usize, bo: usize) -> String {
    let site = h.mint_group(phone);
    h.add_to_forum(phone, bo, &site).await;
    h.group_set_profile(phone, &site, "Mill Road Allotments", "community").await;
    h.node(phone).host_new(&site, "Mill Road Allotments").await.unwrap();
    h.settle().await;
    site
}

#[tokio::test]
async fn a_session_that_joined_through_the_pool_is_refused_the_sites_sequenced_ops() {
    let mut h = Harness::people(&[("ada", &["phone"]), ("bo", &["sim"])]).await;
    let (phone, bo) = (h.device("ada", "phone"), h.device("bo", "sim"));
    let site = a_site(&h, phone, bo).await;
    let key = h.with_sync(phone, |n| n.identity_key());
    let wrap = h.with_sync(phone, |n| n.export_wrap(&PRF, HOST).unwrap());
    let head = |h: &Harness| {
        let u = h.with_sync(phone, |n| n.head_update().unwrap());
        HeadInput { blob: u.sealed, declared_position: u.position }
    };

    // The first session takes the pool leaf the Site began with, and goes (a Door
    // session that ended). Upkeep adds the next leaf, after the Site's state was
    // published, and `pool_provision` restates nothing: the session that takes that
    // leaf folds the Site as empty.
    let (first, _) = h.sign_in_from_wrap("ada", "first", &PRF, &wrap, HOST, &key, Some(head(&h))).await.unwrap();
    h.partition(first);
    h.settle_among(&[phone, bo]).await;
    let (door, r) = h.sign_in_from_wrap("ada", "door", &PRF, &wrap, HOST, &key, Some(head(&h))).await.unwrap();
    let site_outcome = r.objects.iter().find(|o| o.object == site).map(|o| o.outcome.clone());
    assert!(matches!(site_outcome, Some(Outcome::Joined { .. })), "the session holds the Site: {r:?}");
    assert!(h.node(door).object_log(&site).unwrap().is_empty(), "and folds it as empty");

    let refused = h.node(door).apply(&site, OP_SET_FACE, face(r#"{"v":1,"listed":true}"#)).await.unwrap_err();
    assert!(refused.to_string().contains(WHY), "setFace: {refused}");
    let room = h.node(phone).object_new("forum", "Workshop").unwrap();
    let part = pacific_core::parts::set_part_args(&room, "room", 1);
    let refused = h.node(door).apply(&site, pacific_core::parts::OP_SET_PART, part.clone()).await.unwrap_err();
    assert!(refused.to_string().contains(WHY), "setPart: {refused}");

    // The device that minted it writes as before, and its chain is whole.
    h.node(phone).apply(&site, OP_SET_FACE, face(r#"{"v":1,"listed":true}"#)).await.unwrap();
    h.node(phone).apply(&site, pacific_core::parts::OP_SET_PART, part).await.unwrap();
    h.settle_among(&[phone, bo, door]).await;
    h.node(phone).object_compliance(&site).unwrap();
    h.node(bo).object_compliance(&site).unwrap();
    assert_eq!(h.node(bo).group_state(&site).unwrap().face, r#"{"v":1,"listed":true}"#, "Bo's chain is whole");
}

#[tokio::test]
async fn a_device_the_owner_added_holds_the_chain_and_writes() {
    let mut h = Harness::people(&[("ada", &["phone"]), ("bo", &["sim"])]).await;
    let (phone, bo) = (h.device("ada", "phone"), h.device("bo", "sim"));
    let site = a_site(&h, phone, bo).await;

    let laptop = h.add_device("ada", "laptop");
    let bundle = h.node(laptop).build_contact_bundle().unwrap();
    h.node(phone).group_add_member(&site, &bundle).await.unwrap();
    h.node(laptop).sync_once().await.unwrap();

    h.node(laptop).apply(&site, OP_SET_FACE, face(r#"{"v":1,"listed":false}"#)).await.unwrap();
    h.settle().await;
    h.node(phone).object_compliance(&site).unwrap();
    assert_eq!(h.node(phone).group_state(&site).unwrap().face, r#"{"v":1,"listed":false}"#, "the phone folds it");
}
