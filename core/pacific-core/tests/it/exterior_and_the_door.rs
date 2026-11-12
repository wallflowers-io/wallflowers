//! THE EXTERIOR, AND ASKING TO JOIN (#76, ICD 2.6.0) — real nodes, a real relay, real MLS.
//!
//! Joining is a follow request: a non-member holds an object's exterior, carried to them by
//! its members as far as its owner's visibility says, and asks through its door. The owner (or
//! an admitter) admits or declines; an `auto` door admits through whichever member is about.
//!
//! The people: an Owner, their connection Cleo, and Cleo's connection Sam, who knows nobody
//! inside. Sam is two hops from the roster; Cleo is one.

use crate::common;

use common::Harness;
use pacific_core::exterior::DoorMode;
use pacific_core::visibility::Visibility;

const OWNER: usize = 0;
const CLEO: usize = 1;
const SAM: usize = 2;

async fn three() -> Harness {
    let h = Harness::new(&["Owner", "Cleo", "Sam"]).await;
    h.pair(OWNER, CLEO).await;
    h.pair(CLEO, SAM).await;
    h.settle().await;
    h
}

fn holds(h: &Harness, u: usize, obj: &str) -> Option<u32> {
    h.node(u).exteriors().unwrap().into_iter().find(|e| e.carried.object == obj).map(|e| e.carried.hops)
}

fn draft(words: &str, door: Option<DoorMode>) -> pacific_core::node::ExteriorDraft {
    pacific_core::node::ExteriorDraft { name: "Allotment".into(), description: words.into(), photo: None, door, ..Default::default() }
}

async fn allotment(h: &Harness, door: Option<DoorMode>) -> String {
    let obj = h.node(OWNER).object_new("project", "Allotment").expect("mint");
    h.node(OWNER).set_exterior(&obj, &draft("Plots by the river", door)).await.expect("the exterior");
    obj
}

#[tokio::test]
async fn an_exterior_reaches_as_far_as_its_visibility_says() {
    let h = three().await;
    let obj = allotment(&h, Some(DoorMode::Review)).await;
    h.settle().await;
    assert_eq!((holds(&h, CLEO, &obj), holds(&h, SAM, &obj)), (None, None), "a project is private until its owner says");

    h.node(OWNER).set_visibility(&obj, Visibility::Connections).await.unwrap();
    h.settle().await;
    assert_eq!((holds(&h, CLEO, &obj), holds(&h, SAM, &obj)), (Some(1), None), "connections: one hop, and no further");

    h.node(OWNER).set_visibility(&obj, Visibility::Network).await.unwrap();
    h.settle().await;
    assert_eq!((holds(&h, CLEO, &obj), holds(&h, SAM, &obj)), (Some(1), Some(2)), "network: a connection's connection");
    let seen = h.node(SAM).exteriors().unwrap().into_iter().find(|e| e.carried.object == obj).unwrap().carried;
    assert_eq!((seen.exterior.name.as_str(), seen.exterior.description.as_str(), seen.owner), ("Allotment", "Plots by the river", h.id(OWNER)));
    assert!(seen.exterior.door.is_some(), "a door to knock at");

    h.node(OWNER).set_visibility(&obj, Visibility::Private).await.unwrap();
    h.settle().await;
    assert_eq!((holds(&h, CLEO, &obj), holds(&h, SAM, &obj)), (None, None), "narrowing the dial withdraws every copy");
}

#[tokio::test]
async fn a_request_waits_for_the_owner_who_admits_or_declines() {
    let mut h = three().await;
    let dee = h.add_user("Dee");
    h.pair(CLEO, dee).await;
    let obj = allotment(&h, Some(DoorMode::Review)).await;
    h.node(OWNER).set_visibility(&obj, Visibility::Network).await.unwrap();
    h.settle().await;

    let asked = h.node(SAM).request_join(&obj, "I grow beans").await.expect("Sam asks");
    h.node(dee).request_join(&obj, "").await.expect("Dee asks");
    h.settle().await;
    assert!(h.node(SAM).exteriors().unwrap().iter().any(|e| e.carried.object == obj && e.requested), "Sam's ask reads as asked");
    assert!(h.node(CLEO).door_requests(&obj).unwrap().is_empty(), "a non-member holding the face reads no request");

    let waiting = h.node(OWNER).pending_requests().unwrap();
    let sam = waiting.iter().find(|r| r.asker == h.id(SAM)).expect("the owner is told of Sam's ask");
    assert_eq!((sam.request.as_str(), sam.note.as_str(), sam.asker_name.as_str()), (asked.as_str(), "I grow beans", "Sam"));
    assert!(!h.roster(OWNER, &obj).contains(&h.id(SAM)), "nothing is admitted before the owner says");

    h.node(OWNER).decide_request(&obj, &asked, true).await.expect("admit Sam");
    let dees = waiting.iter().find(|r| r.asker == h.id(dee)).unwrap().request.clone();
    h.node(OWNER).decide_request(&obj, &dees, false).await.expect("decline Dee");
    h.settle().await;

    assert!(h.roster(SAM, &obj).contains(&h.id(SAM)), "Sam is inside, in the MLS group");
    assert!(!h.roster(OWNER, &obj).contains(&h.id(dee)), "Dee is not");
    assert!(h.node(OWNER).pending_requests().unwrap().is_empty(), "both answered, neither waits");
    assert!(h.node(dee).exteriors().unwrap().iter().any(|e| e.carried.object == obj && e.requested), "a decline is silent: Dee's ask still reads as asked");
}

#[tokio::test]
async fn an_auto_door_admits_through_any_member_while_the_owner_sleeps() {
    let h = Harness::new(&["Owner", "Mo", "Sam"]).await;
    let (mo, sam) = (1, 2);
    let obj = h.form_object(OWNER, "project", &[mo]).await;
    h.node(OWNER).set_exterior(&obj, &draft("", Some(DoorMode::Auto))).await.unwrap();
    h.node(OWNER).set_visibility(&obj, Visibility::Connections).await.unwrap();
    h.pair(mo, sam).await;
    h.settle().await;
    assert_eq!(holds(&h, sam, &obj), Some(1), "Mo, a member, carried it to Sam");

    h.node(sam).request_join(&obj, "").await.unwrap();
    h.settle_among(&[mo, sam]).await;
    assert!(h.roster(sam, &obj).contains(&h.id(sam)), "Mo's device let Sam in; the owner never woke");
}

#[tokio::test]
async fn a_withdrawn_ask_is_not_answered() {
    let h = three().await;
    let obj = allotment(&h, Some(DoorMode::Review)).await;
    h.node(OWNER).set_visibility(&obj, Visibility::Network).await.unwrap();
    h.settle().await;
    h.node(SAM).request_join(&obj, "").await.unwrap();
    h.node(SAM).withdraw_request(&obj).await.unwrap();
    h.settle().await;
    assert!(h.node(OWNER).pending_requests().unwrap().is_empty(), "a withdrawn ask is not shown");
    assert!(!h.node(SAM).exteriors().unwrap().iter().any(|e| e.carried.object == obj && e.requested));
}

#[tokio::test]
async fn the_ask_and_the_answer_are_not_kept_on_one_device() {
    let mut h = three().await;
    let obj = allotment(&h, Some(DoorMode::Review)).await;
    h.node(OWNER).set_visibility(&obj, Visibility::Network).await.unwrap();
    h.settle().await;
    let asked = h.node(SAM).request_join(&obj, "").await.unwrap();
    h.settle().await;

    // Sam opens WallFlowers on a device that has never seen it: the ask is on Sam's record.
    let words = h.with_sync(SAM, |n| n.recovery_key()).unwrap();
    let head = h.with_sync(SAM, |n| n.head_update().unwrap());
    let (fresh, _) = h
        .sign_in_from_words("Sam", "fresh", &words, Some(pacific_core::resumption::HeadInput { blob: head.sealed, declared_position: head.position }))
        .await
        .expect("Sam signs in from the words");
    h.settle().await;
    let ask = h.node(fresh).asked().unwrap().remove(&obj).expect("the ask is on the record the fresh device folds");
    assert_eq!((hex::encode(ask.request), ask.owner, ask.withdrawn), (asked.clone(), h.id(OWNER), false));

    // The owner declines on one device; their other device holds the decision, from the log.
    let laptop = h.add_device("Owner", "laptop");
    let bundle = h.node(laptop).build_contact_bundle().unwrap();
    h.node(OWNER).group_add_member(&obj, &bundle).await.unwrap();
    h.settle().await;
    h.node(OWNER).decide_request(&obj, &asked, false).await.unwrap();
    h.settle().await;
    let decided = h.node(laptop).exterior_state(&obj).unwrap().unwrap().1.decided;
    assert!(decided.values().any(|d| d.asker == h.id(SAM) && !d.admit), "the laptop folds the decline");
    assert!(h.node(laptop).pending_requests().unwrap().is_empty());
}

/// THE iOS CREATOR'S ORDER: a project made by the unified mint (with its comments part),
/// then the dial, then the exterior — as `CoreService.setOutside` writes them.
#[tokio::test]
async fn a_project_from_the_creator_reaches_a_connection() {
    let h = three().await;
    let mint = pacific_core::mint::MintDraft { name: "Allotment".into(), ..Default::default() };
    let obj = h.node(OWNER).mint(pacific_core::object::ObjectKind::Project, &mint).await.expect("mint");
    h.node(OWNER).set_visibility(&obj, Visibility::Network).await.unwrap();
    h.node(OWNER).set_exterior(&obj, &draft("Plots by the river", Some(DoorMode::Review))).await.unwrap();
    h.settle().await;
    assert_eq!((holds(&h, CLEO, &obj), holds(&h, SAM, &obj)), (Some(1), Some(2)));
}

// ---- What the carry and the door cost (ARC_NETWORK's B1, B2) ----

/// A settled mesh with exteriors at Network publishes nothing more, however often it syncs.
#[tokio::test]
async fn a_settled_mesh_carries_nothing_more() {
    let h = three().await;
    for words in ["Plots by the river", "Seeds and swaps"] {
        let obj = allotment(&h, Some(DoorMode::Review)).await;
        h.node(OWNER).set_exterior(&obj, &draft(words, Some(DoorMode::Review))).await.unwrap();
        h.node(OWNER).set_visibility(&obj, Visibility::Network).await.unwrap();
    }
    h.settle().await;
    let sent = pacific_core::router::published();
    for _ in 0..3 {
        h.sync_all().await;
    }
    assert_eq!(pacific_core::router::published(), sent, "three settled ticks on every device publish nothing");
}

/// A Save that changes nothing writes nothing, and so moves no copy.
#[tokio::test]
async fn an_unchanged_save_writes_nothing() {
    let h = three().await;
    let obj = allotment(&h, Some(DoorMode::Review)).await;
    h.node(OWNER).set_visibility(&obj, Visibility::Network).await.unwrap();
    h.settle().await;
    let sent = pacific_core::router::published();
    let again = h.node(OWNER).set_outside(&obj, Some(Visibility::Network), Some(&draft("Plots by the river", Some(DoorMode::Review)))).await.unwrap();
    assert_eq!(again, 0, "nothing differs from the fold");
    h.sync_all().await;
    assert_eq!(pacific_core::router::published(), sent, "and nothing is sent");
    let moved = h.node(OWNER).set_exterior(&obj, &draft("Plots by the weir", Some(DoorMode::Review))).await.unwrap();
    assert_eq!(moved, 1, "new words are one write: the door is unchanged");
}

/// THE FIRST PASS: two objects with exteriors and three connections outside them is six copies,
/// once; the next pass is none.
#[tokio::test]
async fn the_first_pass_carries_each_exterior_once_to_each_connection() {
    let h = Harness::new(&["Owner", "Ana", "Bo", "Cy"]).await;
    for peer in 1..4 {
        h.pair(OWNER, peer).await;
    }
    h.settle().await;
    for words in ["Plots by the river", "Seeds and swaps"] {
        let obj = h.node(OWNER).object_new("project", "Allotment").unwrap();
        h.node(OWNER).set_outside(&obj, Some(Visibility::Connections), Some(&draft(words, Some(DoorMode::Review)))).await.unwrap();
    }
    assert_eq!(h.node(OWNER).carry_exteriors().await.unwrap(), 6, "2 objects × 3 connections");
    assert_eq!(h.node(OWNER).carry_exteriors().await.unwrap(), 0, "and once");
}

/// THE RACE AT AN OPEN DOOR: three members, one knock. One Add wins; the others stop rather
/// than graft a second leaf, and the asker is in with one.
#[tokio::test]
async fn one_knock_at_an_auto_door_is_one_leaf() {
    let h = Harness::new(&["Owner", "Mo", "Max", "Sam"]).await;
    let (mo, max, sam) = (1, 2, 3);
    let obj = h.form_object(OWNER, "project", &[mo, max]).await;
    h.node(OWNER).set_outside(&obj, Some(Visibility::Connections), Some(&draft("", Some(DoorMode::Auto)))).await.unwrap();
    h.pair(mo, sam).await;
    h.settle().await;
    h.node(sam).request_join(&obj, "").await.unwrap();
    // Every member drains the door and answers before any has seen another's Add.
    for m in [OWNER, mo, max] {
        h.node(m).drain_doors().await.unwrap();
    }
    for m in [OWNER, mo, max] {
        let _ = h.node(m).answer_exterior_doors().await;
    }
    h.settle().await;
    let leaves = h.device_identities(OWNER, &obj).into_iter().filter(|k| *k == h.id(sam)).count();
    assert_eq!(leaves, 1, "one knock, one leaf");
    assert!(h.roster(sam, &obj).contains(&h.id(sam)), "and Sam holds it");
}
