//! The in-band stale check (D-34 (c); srr/security.md step 7): a sealed state opened
//! again drains, publishing nothing, and a message from its own leaf that it never sent
//! means a later copy of it has spoken. That copy's seal is the one to open; this one
//! would send again from ratchet positions already used.

mod common;

use common::Harness;
use pacific_core::router::Routes;
use pacific_core::{devstate, Node};

const NODE: &str = "door.example";

fn at_rest() {
    pacific_core::atrest::set_device_key([0x5a; 32]);
}

fn fresh_dir(h: &Harness) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::env::set_var("PACIFIC_STATE_DIR", d.path());
    Routes::parse(&h.relay).save().unwrap();
    pacific_core::node::set_default_arc(&h.relay).unwrap();
    d
}

/// Open `blob` in `dir`, drain it, and put it down: what it found, and what it published.
async fn drained(dir: &tempfile::TempDir, blob: &[u8], seed: &[u8; 32]) -> (devstate::Drained, u64) {
    std::env::set_var("PACIFIC_STATE_DIR", dir.path());
    let (n, _) = Node::open_restored(blob, seed, NODE).unwrap();
    let sent = n.published();
    let d = n.drain_only().await.unwrap();
    let sent = n.published() - sent;
    n.discard().unwrap();
    (d, sent)
}

/// The published blob and the drained one are the same bytes: the relay stores the
/// string and hands it back. So the device's own echo is known, and nothing is sent.
#[tokio::test]
async fn a_device_knows_its_own_echo_and_drain_only_says_nothing() {
    at_rest();
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let obj = h.form_forum(ada, &[bo]).await;
    h.settle().await;
    h.obj_post(ada, &obj, "mine").await;

    let (d, echoes, sent) = h
        .with(ada, |n| async move {
            let (skipped, sent) = (n.skipped_own(), n.published());
            let d = n.drain_only().await.unwrap();
            (d, n.skipped_own() - skipped, n.published() - sent)
        })
        .await;
    assert!(echoes >= 1, "ada's own post came back to her");
    assert_eq!(d.own_unsent, 0, "and she knows it for her own: {d:?}");
    assert!(d.groups >= 1, "{d:?}");
    assert_eq!(sent, 0, "drain_only publishes nothing");
}

#[tokio::test]
async fn a_state_sealed_before_its_device_spoke_is_stale() {
    at_rest();
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let obj = h.form_forum(ada, &[bo]).await;
    h.settle().await;
    let (before, seed) = h.with_sync(ada, |n| (n.seal_state(NODE, 1).unwrap(), n.id.seed_bytes().unwrap()));

    h.obj_post(ada, &obj, "after the seal").await;
    h.settle().await;
    let after = h.with_sync(ada, |n| n.seal_state(NODE, 2).unwrap());

    let dir = fresh_dir(&h);
    let (stale, sent) = drained(&dir, &before, &seed).await;
    assert!(stale.own_unsent >= 1, "the later post is seen from the earlier state: {stale:?}");
    assert_eq!(sent, 0, "and the earlier state sent nothing");

    let (current, sent) = drained(&dir, &after, &seed).await;
    assert_eq!(current.own_unsent, 0, "the later state sent it: {current:?}");
    assert_eq!(sent, 0);
    at_rest();
}

#[tokio::test]
async fn a_state_sealed_before_its_device_committed_is_stale() {
    at_rest();
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let (ada, bo, cy) = (0, 1, 2);
    let obj = h.form_forum(ada, &[bo]).await;
    h.settle().await;
    let (before, seed) = h.with_sync(ada, |n| (n.seal_state(NODE, 1).unwrap(), n.id.seed_bytes().unwrap()));

    h.add_to_forum(ada, cy, &obj).await;

    let dir = fresh_dir(&h);
    let (stale, sent) = drained(&dir, &before, &seed).await;
    assert!(stale.own_unsent >= 1, "the later commit, from this leaf, is seen: {stale:?}");
    assert_eq!(sent, 0);
    at_rest();
}
