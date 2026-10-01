//! A WELCOME WHOSE ARC ROUTE IS PLAIN ws:// TO A PUBLIC HOST IS REFUSED (SECURITY, 27 Sep).
//!
//! The joiner stores the sender's `IntroPayload::arc`, and `routes_for` dials it. Plain
//! ws:// off the machine or the LAN carries routing tags and timing in the clear, and
//! joining without the route would leave the member silently off the group's relay. So
//! the Welcome is refused, nothing of it is kept, and the quarantine names why. The
//! control: the same Welcome over a ws:// route to a local host joins.
//!
//! Any other route that is not a relay URL is refused the same way, under its own
//! reason; an empty route is the default relay and joins.

mod common;

use common::Harness;
use pacific_core::{mls, paths};

const REASON: &str = "the group's Arc route is plain ws:// to a public host";
const NOT_RELAY: &str = "the group's Arc route is not a relay URL";

/// `owner` stamps `route` as its Arc, mints a forum under it and adds `member`, whose
/// Welcome carries the forum's Arc; `member` syncs once. The owner never dials `route`:
/// an object under the owner's own Arc routes through its device relay (`routes_for`).
async fn welcome_routed_over(h: &Harness, owner: usize, member: usize, route: &str) -> String {
    let obj = {
        let n = h.node(owner);
        std::fs::write(paths::arc_url_path(), route).unwrap();
        n.object_new("forum", "").unwrap()
    };
    let bundle = h.node(member).build_contact_bundle().unwrap();
    h.node(owner).group_add_member(&obj, &bundle).await.unwrap();
    h.node(member).sync_once().await.unwrap();
    obj
}

/// Does device `u` hold MLS state for `obj`?
fn holds_group(h: &Harness, u: usize, obj: &str) -> bool {
    let node = h.node(u);
    let (sk, pk) = node.dir.mls_signing_keypair().unwrap();
    let sid = mls::signing_identity(&node.id.identity_pk(), &pk);
    let client = mls::build_client_sqlite(&paths::db_path(), sid, mls::SecretKey::new(sk)).unwrap();
    mls::load_group(&client, &hex::decode(obj).unwrap()).is_ok()
}

#[tokio::test]
async fn a_welcome_routed_over_public_ws_is_refused_and_named() {
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let (ada, bo, cy) = (0, 1, 2);

    // The control first: ada's own Arc is the harness relay, ws:// on loopback.
    let local = welcome_routed_over(&h, ada, cy, &h.relay).await;
    assert!(holds_group(&h, cy, &local), "a ws:// route to a local host joins");
    assert_eq!(h.node(cy).object_arc(&local).unwrap().as_deref(), Some(h.relay.as_str()));
    assert!(h.node(cy).quarantined().unwrap().is_empty());

    let public = welcome_routed_over(&h, ada, bo, "ws://203.0.113.9:9092").await;
    assert!(!holds_group(&h, bo, &public), "bo refused the Welcome and persisted nothing of it");
    let gid = hex::decode(&public).unwrap();
    assert!(h.node(bo).dir.group_kind(&gid).unwrap().is_none(), "no directory row either");
    let q = h.node(bo).quarantined().unwrap();
    assert_eq!(q.len(), 1, "{q:?}");
    assert!(q[0].0.is_none(), "an intro blob, not a group's");
    assert!(q[0].2.contains(REASON), "the refusal is quarantined with its reason: {q:?}");
}

#[tokio::test]
async fn a_welcome_whose_route_is_not_a_relay_url_is_refused_and_named() {
    let h = Harness::new(&["ada", "bo", "cy", "di", "ed"]).await;
    let ada = 0;

    for (member, route) in [(1, "http://127.0.0.1:8080"), (2, "wss://"), (3, "wss://evil.com\\@127.0.0.1")] {
        let obj = welcome_routed_over(&h, ada, member, route).await;
        assert!(!holds_group(&h, member, &obj), "{route}: refused, and nothing of it persisted");
        let gid = hex::decode(&obj).unwrap();
        assert!(h.node(member).dir.group_kind(&gid).unwrap().is_none(), "{route}: no directory row");
        let q = h.node(member).quarantined().unwrap();
        assert_eq!(q.len(), 1, "{route}: {q:?}");
        assert!(q[0].2.contains(NOT_RELAY), "{route}: quarantined under its own reason: {q:?}");
    }

    // Empty is the default relay, not a route: the Welcome joins.
    let ed = 4;
    let obj = {
        let n = h.node(ada);
        std::fs::write(paths::arc_url_path(), &h.relay).unwrap();
        let obj = n.object_new("forum", "").unwrap();
        n.dir.set_group_arc(&hex::decode(&obj).unwrap(), "").unwrap();
        obj
    };
    let bundle = h.node(ed).build_contact_bundle().unwrap();
    h.node(ada).group_add_member(&obj, &bundle).await.unwrap();
    h.node(ed).sync_once().await.unwrap();
    assert!(holds_group(&h, ed, &obj), "an empty route joins");
    assert!(h.node(ed).quarantined().unwrap().is_empty());
}
