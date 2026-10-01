//! A device's state sealed at rest and opened again (D-34 (c); mdr/door.md §4): the
//! Door ends a person's process with its directory sealed, and the next sign-in opens
//! it as the same device, holding every leaf it held, taking none.

mod common;

use common::Harness;
use pacific_core::resumption::Outcome;
use pacific_core::router::Routes;
use pacific_core::{devstate, paths, Node};

const NODE: &str = "door.example";

/// The Door sets an at-rest key before any identity exists (`account.rs`); so do these.
fn at_rest() {
    pacific_core::atrest::set_device_key([0x5a; 32]);
}

/// A state directory of its own, on this harness's relay, and nothing else in it.
fn fresh_dir(h: &Harness) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::env::set_var("PACIFIC_STATE_DIR", d.path());
    Routes::parse(&h.relay).save().unwrap();
    pacific_core::node::set_default_arc(&h.relay).unwrap();
    d
}

fn state_held(dir: &std::path::Path) -> Vec<&'static str> {
    devstate::FILES.iter().copied().filter(|f| dir.join(f).exists()).collect()
}

#[tokio::test]
async fn a_sealed_state_opens_as_the_same_device_and_takes_no_leaf() {
    at_rest();
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let obj = h.form_forum(ada, &[bo]).await;
    h.settle().await;
    h.obj_post(bo, &obj, "before the seal").await;
    h.settle().await;

    let (blob, seed, device, pk, log, named) = h.with_sync(ada, |n| {
        let blob = n.seal_state(NODE, 7).unwrap();
        assert!(
            !paths::state_dir().join(devstate::SNAPSHOT).exists(),
            "the database's copy is gone once the seal is made"
        );
        (
            blob,
            n.id.seed_bytes().unwrap(),
            n.dir.device_id().unwrap(),
            n.id.identity_pk(),
            n.object_log(&obj).unwrap(),
            n.objects_named().unwrap(),
        )
    });
    assert!(!blob.windows(15).any(|w| w == b"before the seal"), "the seal hides the posts");
    assert_eq!(devstate::peek(&blob).unwrap().counter, 7);

    let dir = fresh_dir(&h);
    // The opener's own Arc, as the Door's session writes it before it opens (NC-85).
    pacific_core::node::set_default_arc(&h.relay).unwrap();
    let (door, header) = Node::open_restored(&blob, &seed, NODE).unwrap();
    assert_eq!(std::fs::read_to_string(paths::arc_url_path()).unwrap(), h.relay, "the opener's Arc stands");
    assert_eq!(header.device_id, device);
    assert_eq!(door.id.identity_pk(), pk);
    assert_eq!(door.dir.device_id().unwrap(), device, "the same device, not a new one");
    assert_eq!(door.object_log(&obj).unwrap(), log, "the forum as the device last folded it");
    assert_eq!(door.objects_named().unwrap(), named);

    let r = door.resume(None).await.unwrap();
    assert!(!r.objects.is_empty(), "{r:?}");
    for o in &r.objects {
        assert!(matches!(o.outcome, Outcome::AlreadyHeld), "{} took a leaf: {r:?}", o.object);
    }
    drop(door);
    drop(dir);
}

#[tokio::test]
async fn a_state_opens_nowhere_else_and_never_over_another() {
    at_rest();
    let h = Harness::new(&["ada"]).await;
    let ada = 0;
    let (blob, seed) = h.with_sync(ada, |n| (n.seal_state(NODE, 1).unwrap(), n.id.seed_bytes().unwrap()));

    let dir = fresh_dir(&h);
    assert!(Node::open_restored(&blob, &seed, "door.elsewhere").is_err(), "another node");
    assert!(state_held(dir.path()).is_empty(), "a refused open leaves nothing: {:?}", state_held(dir.path()));
    assert!(Node::open_restored(&blob, &[0x33; 32], NODE).is_err(), "another person's seed");
    assert!(state_held(dir.path()).is_empty());
    let (n, _) = Node::open_restored(&blob, &seed, NODE).unwrap();
    n.discard().unwrap();
    assert!(state_held(dir.path()).is_empty(), "discard takes the state out");

    // Over a directory that holds a state: refused, and that state untouched. (A refused
    // open and a discard each leave a new at-rest key; the harness's devices share one.)
    at_rest();
    let before = h.with_sync(ada, |n| n.dir.device_id().unwrap());
    let held = h.with_sync(ada, |n| {
        let home = paths::state_dir();
        drop(n);
        assert!(Node::open_restored(&blob, &seed, NODE).is_err());
        state_held(&home)
    });
    assert!(held.contains(&"pacific.db") && held.contains(&"id_ed25519"), "{held:?}");
    assert_eq!(h.with_sync(ada, |n| n.dir.device_id().unwrap()), before, "the state it would have overwritten");
    drop(dir);
}
