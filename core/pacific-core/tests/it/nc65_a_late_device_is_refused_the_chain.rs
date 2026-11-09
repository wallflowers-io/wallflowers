//! NC-65, under #68 (Ralph, 9 Nov 2026: "they get everything"). A device that joined an
//! object after it began, through a pool leaf, once folded what it could not see as empty, and
//! core refused its sequenced ops rather than let them break the chain (ChainBroken). Now the
//! leaf's segment (ICD `mls.poolHistory`) brings what came before it, so the session holds the
//! chain and writes. A device the owner adds still holds it from the Add's re-emission.

use crate::common;

use common::Harness;
use pacific_core::group::OP_SET_FACE;
use pacific_core::resumption::{HeadInput, Outcome};

const HOST: &str = "arc.example";
const PRF: [u8; 32] = [0x11; 32];

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
async fn a_session_that_joined_through_the_pool_holds_the_sites_chain_and_writes() {
    let mut h = Harness::people(&[("ada", &["phone"]), ("bo", &["sim"])]).await;
    let (phone, bo) = (h.device("ada", "phone"), h.device("bo", "sim"));
    let site = a_site(&h, phone, bo).await;
    let key = h.with_sync(phone, |n| n.identity_key());
    let wrap = h.with_sync(phone, |n| n.export_wrap(&PRF, HOST).unwrap());
    let head = |h: &Harness| {
        let u = h.with_sync(phone, |n| n.head_update().unwrap());
        HeadInput { blob: u.sealed, declared_position: u.position }
    };

    // The first session takes the pool leaf the Site began with, and goes (a Door session
    // that ended). Upkeep adds the next leaf after the Site's state was published, and gives
    // it the segment of what came before it: the session that takes that leaf holds the chain.
    let (first, _) = h.sign_in_from_wrap("ada", "first", &PRF, &wrap, HOST, &key, Some(head(&h))).await.unwrap();
    h.partition(first);
    h.settle_among(&[phone, bo]).await;
    let (door, r) = h.sign_in_from_wrap("ada", "door", &PRF, &wrap, HOST, &key, Some(head(&h))).await.unwrap();
    let site_outcome = r.objects.iter().find(|o| o.object == site).map(|o| o.outcome.clone());
    assert!(matches!(site_outcome, Some(Outcome::Joined { .. })), "the session holds the Site: {r:?}");
    assert_eq!(h.node(door).group_state(&site).unwrap().display_name, "Mill Road Allotments", "and folds its state");

    h.node(door).apply(&site, OP_SET_FACE, face(r#"{"v":1,"listed":true}"#)).await.unwrap();
    let room = h.node(phone).object_new("forum", "Workshop").unwrap();
    let part = pacific_core::parts::set_part_args(&room, "room", 1);
    h.node(door).apply(&site, pacific_core::parts::OP_SET_PART, part).await.unwrap();
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
