//! O-69, the Door's write path in core: `Node::apply_local` stores a write and sends
//! nothing; `Node::publish_pending` sends what it left, on the held session. A second
//! member sees the write only once it is published, and a sequenced op is authored at
//! head, after a drain.

mod common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args, FORUM_POST};
use pacific_core::object::ObjectKind;

fn post(text: &str) -> Args {
    let mut a = Args::new();
    a.insert("text".into(), ArgVal::Text(text.into()));
    a
}

fn sees(h: &Harness, u: usize, room: &str, text: &str) -> bool {
    h.obj_view(u, room).iter().any(|m| m.text == text)
}

#[tokio::test]
async fn a_local_commit_is_read_at_once_and_sent_by_publish_pending() {
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let room = h.mint_object(ada, ObjectKind::Forum, "Talk", &[bo]).await;
    h.settle().await;

    let n = h.node(ada);
    n.apply_local(&room, FORUM_POST, post("local first")).await.expect("the local commit");
    drop(n);
    assert!(sees(&h, ada, &room, "local first"), "the writer reads its own write at once");
    h.sync(bo).await;
    assert!(!sees(&h, bo, &room, "local first"), "nothing was sent by the local commit");

    let n = h.node(ada);
    assert_eq!(n.publish_pending(&room).await.expect("published"), 1, "one Delta sent");
    assert!(n.holds_session(), "the held session given back after the publish");
    assert_eq!(n.publish_pending(&room).await.unwrap(), 0, "and nothing left to send");
    drop(n);
    h.sync(bo).await;
    assert!(sees(&h, bo, &room, "local first"), "bo sees it once it is published");
}

/// The outbox semantics stand: several local commits, one publish sends them all, in order.
#[tokio::test]
async fn one_publish_sends_every_local_commit_of_the_object() {
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let room = h.mint_object(ada, ObjectKind::Forum, "Talk", &[bo]).await;
    h.settle().await;
    let n = h.node(ada);
    for t in ["one", "two", "three"] {
        n.apply_local(&room, FORUM_POST, post(t)).await.unwrap();
    }
    assert_eq!(n.publish_pending(&room).await.unwrap(), 3);
    drop(n);
    h.sync(bo).await;
    let got: Vec<String> = h.obj_view(bo, &room).into_iter().map(|m| m.text).collect();
    assert_eq!(got, vec!["one", "two", "three"], "all three, in order");
}

/// A sequenced op is authored at head: a member added (a commit the writer's device has not
/// drained) before the owner's local commit, and the owner's op still lands at the live
/// epoch, so the new member folds it.
#[tokio::test]
async fn a_sequenced_local_commit_is_authored_at_head() {
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let (ada, bo, cy) = (0, 1, 2);
    let site = h.mint_object(ada, ObjectKind::Group, "Us", &[bo]).await;
    h.settle().await;
    // Another device of the room's governance moves the epoch: bo is made owner and adds cy,
    // so ada's device holds a superseded epoch until it drains.
    h.node(ada).group_hand_over(&site, &hex::encode(h.id(bo))).await.unwrap();
    h.settle().await;
    h.add_to_forum(bo, cy, &site).await;
    let epoch = h.epoch(cy, &site);
    let n = h.node(bo);
    let rename = pacific_core::group::OP_SET_PROFILE;
    let mut args = Args::new();
    args.insert("displayName".into(), ArgVal::Text("Us, renamed".into()));
    args.insert("shape".into(), ArgVal::Text("team".into()));
    n.apply_local(&site, rename, args).await.expect("the owner's sequenced write");
    assert!(n.publish_pending(&site).await.unwrap() >= 1);
    drop(n);
    h.sync(cy).await;
    assert_eq!(h.epoch(cy, &site), epoch, "no epoch was skipped");
    assert_eq!(h.group_view(cy, &site).display_name, "Us, renamed", "cy folds the owner's write");
}
