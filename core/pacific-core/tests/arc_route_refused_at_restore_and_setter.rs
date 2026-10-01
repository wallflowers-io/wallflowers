//! THE ROUTE CHECK AT THE RESTORE AND AT THE LOCAL SETTER (SECURITY, 27 Sep).
//!
//! (ii) A way-in on the spine whose route fails the check is not joined: no lease is
//! claimed, no MLS state or directory row is kept, the outcome says why, and
//! `noncompliant_objects` names the object with its route. The control: the self
//! record's way-in, on a relay URL, joins.
//!
//! (iii) `set_default_arc` refuses what fails the check, by name, and writes nothing.

mod common;

use common::Harness;
use pacific_core::resumption::{HeadInput, Outcome};
use pacific_core::spine::Body;
use pacific_core::{mls, paths};

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
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
async fn a_way_in_whose_route_fails_is_not_restored_and_is_named() {
    let mut h = Harness::new(&["ada"]).await;
    let phone = 0;
    let not_relay = h.form_named_forum(phone, "not relay");
    let public_ws = h.form_named_forum(phone, "public ws");
    h.settle().await; // upkeep provisions a way-in for each
    let spine = h.with_sync(phone, |n| n.spine().unwrap()).expect("A2: every identity has a spine");

    // The latest way-in of each forum names a route that fails: what an older build,
    // or another device of the account, could have written.
    let bad = [
        (not_relay.as_str(), "http://127.0.0.1:8080", "not a relay URL"),
        (public_ws.as_str(), "ws://203.0.113.9:9092", "plain ws:// to a public host"),
    ];
    let head = h
        .with(phone, |n| async move {
            let entries = n.spine_entries().unwrap();
            let bodies = bad
                .iter()
                .map(|(obj, route, _)| {
                    let gid = hex::decode(obj).unwrap();
                    let mut way = entries
                        .iter()
                        .rev()
                        .find_map(|(_, e)| match &e.body {
                            Body::Pool(p) if p.group_id == gid => Some(p.clone()),
                            _ => None,
                        })
                        .expect("a way-in for each forum");
                    way.arc = route.to_string();
                    Body::Pool(way)
                })
                .collect();
            n.chain_append(bodies).await.unwrap()
        })
        .await;

    h.partition(phone);
    let laptop = h.add_device("ada", "laptop");
    let input = HeadInput { blob: head.sealed.clone(), declared_position: head.position };
    let r = h.with(laptop, |n| async move { n.resume_at(Some(input), now()).await.unwrap() }).await;

    assert_eq!(r.objects[0].object, spine, "the spine is first (A6.5)");
    assert!(matches!(r.objects[0].outcome, Outcome::Joined { .. }), "the control: {:?}", r.objects[0]);

    let named = h.with_sync(laptop, |n| n.noncompliant_objects().unwrap());
    assert_eq!(named.len(), bad.len(), "{named:?}");
    for (obj, route, why) in bad {
        let reason = format!("the group's Arc route is {why}: {route:?}");
        let o = r.objects.iter().find(|o| o.object == obj).expect("the spine names it");
        assert_eq!(o.outcome, Outcome::JoinFailed(reason.clone()), "{route}");

        let gid = hex::decode(obj).unwrap();
        assert!(!holds_group(&h, laptop, obj), "{route}: no MLS state");
        assert!(h.node(laptop).dir.group_kind(&gid).unwrap().is_none(), "{route}: no directory row");
        assert!(
            h.node(laptop).dir.leases().unwrap().iter().all(|l| l.0 != gid),
            "{route}: no leaf was taken"
        );

        let n = named.iter().find(|c| c.object_id == obj).unwrap_or_else(|| panic!("{route}: not named: {named:?}"));
        assert_eq!(n.kind, "forum");
        assert_eq!(n.reason, reason);
    }

    // The device that holds them names nothing: the sweep is of what the restore refused.
    assert!(h.with_sync(phone, |n| n.noncompliant_objects().unwrap()).is_empty());
}

#[tokio::test]
async fn the_local_arc_setter_refuses_what_fails_the_check() {
    let h = Harness::new(&["ada"]).await;
    let n = h.node(0);
    n.set_default_arc(&h.relay).unwrap();

    for (route, why) in [
        ("http://127.0.0.1:8080", "not a relay URL"),
        ("wss://", "not a relay URL"),
        ("wss://evil.com\\@127.0.0.1", "not a relay URL"),
        ("ws://203.0.113.9:9092", "plain ws:// to a public host"),
        ("", "not a relay URL"),
    ] {
        let e = n.set_default_arc(route).unwrap_err().to_string();
        assert!(e.contains(&format!("this device's Arc route is {why}: {route:?}")), "{route}: {e}");
        assert_eq!(std::fs::read_to_string(paths::arc_url_path()).unwrap(), h.relay, "{route}: nothing written");
    }

    let obj = n.object_new("forum", "").unwrap();
    assert_eq!(n.object_arc(&obj).unwrap().as_deref(), Some(h.relay.as_str()), "what passed is what is stamped");
}
