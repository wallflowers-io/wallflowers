//! #12 — connect from a member's card. Each member offers contact bundles on the Site they share
//! (`base.offerBundle`, ICD 2.4.0 `facets.bundles`); another takes one and pairs with it, so the
//! request is the private pairing, and the Site's log carries only offers and withdrawals
//! (ruled 30 Sep: pending invitations stay pairwise, off the Site).

mod common;

use common::Harness;
use pacific_core::contact::LinkState;
use serde_json::Value;

fn site_view(h: &Harness, u: usize, site: &str) -> Value {
    serde_json::from_str(&h.node(u).object_view(site).expect("the Site folds")).expect("JSON")
}

/// The kp_ids `member` has on offer, as `reader` folds the Site.
fn offers(h: &Harness, reader: usize, site: &str, member: usize) -> Vec<String> {
    site_view(h, reader, site)["bundles"][hex::encode(h.id(member))]
        .as_array()
        .map(|a| a.iter().map(|o| o["kp_id"].as_str().unwrap().to_string()).collect())
        .unwrap_or_default()
}

/// The Site, as its owner makes it in the WebApp: each member added by their contact code, and
/// sent the Site's history as the admitter (the Door's /v2/add; ICD `mls.historyPolicy`).
async fn admit_by_code(h: &Harness, members: &[usize]) -> String {
    let site = h.node(0).object_new("group", "").unwrap();
    for &m in members {
        h.settle().await;
        let code = h.node(m).build_contact_bundle().unwrap();
        let intro = pacific_core::handshake::parse_and_verify(&code).unwrap().intro_tag;
        h.node(0).group_add_member(&site, &code).await.unwrap();
        h.node(0).send_history(&site, &h.id(m), &intro).await.unwrap();
        h.node(m).sync_once().await.unwrap();
    }
    h.settle().await;
    site
}

#[tokio::test]
async fn a_member_connects_from_the_sites_offers_and_the_site_hears_nothing_of_it() {
    let h = Harness::new(&["ada", "ben", "cal"]).await;
    let site = admit_by_code(&h, &[1, 2]).await;

    // Each member's own offers, on every member's fold.
    let before = offers(&h, 0, &site, 1);
    assert_eq!(before.len(), 3, "ben's stock, as ada folds the Site");
    assert_eq!(offers(&h, 2, &site, 0).len(), 3, "ada's, as cal folds it");

    // From here, what the Site's log gains: ada and cal asking, ben accepting and declining.
    let quiet = h.node(0).object_log(&site).expect("the Site's log").len();

    // Ada asks: a pairing with one of ben's offers. Ben is invited; ada has invited.
    let peer = h.node(0).connect_member(&site, &h.id(1)).await.expect("ada connects to ben");
    assert_eq!(peer, h.id(1));
    h.settle().await;
    assert_eq!(h.node(1).link_with(&h.id(0)).unwrap(), Some(LinkState::InvitedMe));
    assert_eq!(h.node(0).link_with(&h.id(1)).unwrap(), Some(LinkState::Invited));

    // The offer ada used is withdrawn by ben's device, and his stock is made up again.
    for reader in [0, 2] {
        let after = offers(&h, reader, &site, 1);
        assert_eq!(after.len(), 3, "ben's stock made up, as {reader} folds it: {after:?}");
        assert_eq!(before.iter().filter(|k| !after.contains(k)).count(), 1, "exactly the used one is gone, as {reader} folds it");
    }

    // Ben accepts: linked on both sides, and ada's message reaches him.
    h.node(1).set_link(&h.id(0), true).await.expect("ben accepts");
    h.settle().await;
    assert_eq!(h.node(0).link_with(&h.id(1)).unwrap(), Some(LinkState::Linked));
    assert_eq!(h.node(1).link_with(&h.id(0)).unwrap(), Some(LinkState::Linked));
    h.node(0).author_post(&h.id(1), "hello from the Site").await.expect("ada messages ben");
    h.settle().await;
    let seen = h.node(1).dm_transcript(&h.id(0)).expect("ben's transcript");
    assert!(seen.iter().any(|l| l.contains("hello from the Site")), "{seen:?}");

    // Cal asks, and ben declines: ended on both sides.
    h.node(2).connect_member(&site, &h.id(1)).await.expect("cal connects to ben");
    h.settle().await;
    assert_eq!(h.node(1).link_with(&h.id(2)).unwrap(), Some(LinkState::InvitedMe));
    h.node(1).set_link(&h.id(2), false).await.expect("ben declines");
    h.settle().await;
    assert_eq!(h.node(2).link_with(&h.id(1)).unwrap(), Some(LinkState::Ended));

    // The Site's log gained offers and withdrawals, and nothing else: no one's asking is on it.
    use pacific_core::object::ObjectType;
    let log = h.node(0).object_log(&site).expect("the Site's log");
    // A delta already on it before (the owner restating its own for late joiners) is not new.
    let had: std::collections::HashSet<&String> = log[..quiet].iter().map(|(_, _, _, id, _)| id).collect();
    let gained: Vec<&str> = log[quiet..]
        .iter()
        .filter(|(_, _, _, id, _)| !had.contains(id))
        .map(|(op, ..)| pacific_core::group::GroupType::op(*op).map_or("?", |d| d.name))
        .collect();
    // Only members' standing rows, each about its own author: offers, withdrawals, cards, and the
    // membership records the MLS doors write (one may land late from the adds above). None says
    // who asked whom, and the link ops are on the connections, never here.
    let standing = ["base.offerBundle", "base.withdrawBundle", "base.publishProfile", "base.memberJoined"];
    assert!(gained.iter().all(|n| standing.contains(n)), "{gained:?}");
}

/// THE TEST OF PERSISTENCE: Ben, on a device that has never seen his account, signed in from
/// the passkey's wrap as the Door does, holds the link and the conversation, and reads Ada.
#[tokio::test]
async fn a_connection_made_from_a_card_survives_a_device_that_never_saw_it() {
    const PRF: [u8; 32] = [0x22; 32];
    const HOST: &str = "arc.example";
    let mut h = Harness::new(&["ada", "ben"]).await;
    let site = admit_by_code(&h, &[1]).await;
    h.node(0).connect_member(&site, &h.id(1)).await.expect("ada connects to ben");
    h.settle().await;
    h.node(1).set_link(&h.id(0), true).await.expect("ben accepts");
    h.settle().await;
    h.node(0).author_post(&h.id(1), "before the new device").await.expect("ada messages ben");
    h.settle().await;

    let key = h.with_sync(1, |n| n.identity_key());
    let wrap = h.with_sync(1, |n| n.export_wrap(&PRF, HOST).unwrap());
    let head = h.with_sync(1, |n| n.head_update().unwrap());
    let head = pacific_core::resumption::HeadInput { blob: head.sealed, declared_position: head.position };
    h.partition(1);
    let (fresh, _) = h.sign_in_from_wrap("ben", "fresh", &PRF, &wrap, HOST, &key, Some(head)).await.expect("ben signs in");
    h.settle_among(&[0, fresh]).await;

    assert_eq!(h.node(fresh).link_with(&h.id(0)).unwrap(), Some(LinkState::Linked), "the link, on the new device");
    assert!(h.node(fresh).conversation_with(&h.id(0)).unwrap().is_some(), "the conversation, on the new device");
    let seen = h.node(fresh).dm_transcript(&h.id(0)).expect("the transcript");
    assert!(seen.iter().any(|l| l.contains("before the new device")), "{seen:?}");
    h.node(0).author_post(&h.id(1), "after it").await.expect("ada messages again");
    h.settle_among(&[0, fresh]).await;
    let seen = h.node(fresh).dm_transcript(&h.id(0)).expect("the transcript");
    assert!(seen.iter().any(|l| l.contains("after it")), "{seen:?}");
    // And it speaks: ben's answer from the new device reaches ada.
    h.node(fresh).author_post(&h.id(0), "from the new device").await.expect("ben answers from the new device");
    h.settle_among(&[0, fresh]).await;
    let seen = h.node(0).dm_transcript(&h.id(1)).expect("ada's transcript");
    assert!(seen.iter().any(|l| l.contains("from the new device")), "{seen:?}");
}
