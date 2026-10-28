//! NC-140 (Software Testing, run 106; Software Management's rule): a claim presented
//! seconds after a Site or a room is made reaches an Arc's node that has not synced yet.
//!
//! 1. THE RACE: the Site's Welcome still waits in the node's intro mailbox. The node takes
//!    it and admits; a Site still not there after a bounded wait is a named, retryable
//!    refusal, never the directory's "group … has no owner".
//! 2. THE SILENT ANSWER: a claim that cannot see the Site's rooms for it SAYS SO. A room's
//!    Welcome waiting is taken; a room the node is not in, or not the admitter of, is named
//!    in `unjoined` with why. `rooms: [], unjoined: []` only for a Site with no room for
//!    the claim. The same claim presented again completes the rooms, spent once.

use crate::common;

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use common::Harness;
use ed25519_dalek::Signer;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::object::ObjectKind;
use pacific_core::CoreError;
use std::collections::HashMap;

fn kiosk() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[9u8; 32])
}

fn kiosk_keys() -> HashMap<String, [u8; 32]> {
    let pk = kiosk().verifying_key().to_bytes();
    HashMap::from([(pacific_core::claim::kid_of(&pk), pk)])
}

/// A claim as the kiosk issues one, for `site`, with nonce `n` and no choice.
fn claim(site: &str, n: u8) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let pk = kiosk().verifying_key().to_bytes();
    let payload = format!(
        r#"{{"k":"{}","s":"{site}","n":"{}","iat":{now},"exp":{}}}"#,
        pacific_core::claim::kid_of(&pk),
        B64.encode([n; 16]),
        now + 600
    );
    let p = B64.encode(payload);
    format!("v1.{p}.{}", B64.encode(kiosk().sign(format!("v1.{p}").as_bytes()).to_bytes()))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

fn admitter(h: &Harness, arc: usize) -> Args {
    let mut a = Args::new();
    a.insert("member".into(), ArgVal::Text(hex::encode(h.id(arc))));
    a.insert("role".into(), ArgVal::Text("admitter".into()));
    a
}

/// The founder's mint, with no settle after it: no one else has synced.
async fn mint(h: &Harness, owner: usize, kind: ObjectKind, name: &str) -> String {
    let draft = pacific_core::mint::MintDraft { name: name.to_string(), ..Default::default() };
    h.node(owner).mint(kind, &draft).await.expect("minted")
}

/// The founder adds `who` to `obj`, and `who` does not sync: its Welcome waits.
async fn add_unsynced(h: &Harness, owner: usize, who: usize, obj: &str) {
    let bundle = h.node(who).build_contact_bundle().unwrap();
    h.node(owner).group_add_member(obj, &bundle).await.expect("added");
}

/// The room is the Site's part, and the Site the room's parent (runbook § 3, room step 2).
async fn mark_room(h: &Harness, owner: usize, site: &str, room: &str) {
    let founder = h.node(owner);
    founder
        .apply(site, pacific_core::parts::OP_SET_PART, pacific_core::parts::set_part_args(room, "room", now_ms()))
        .await
        .expect("the room is the Site's part");
    founder
        .apply(room, pacific_core::parent::OP_SET_PARENT, pacific_core::parent::set_parent_args(site, "room", now_ms()))
        .await
        .expect("the Site is the room's parent");
}

/// A Site made and the Arc's node added and granted, the claim presented before the node
/// syncs: admitted. A Site with no room answers the clean empty.
#[tokio::test]
async fn a_claim_before_the_arc_takes_the_sites_welcome_is_admitted() {
    let h = Harness::new(&["ada", "arc", "vis"]).await;
    let (ada, arc, vis) = (0, 1, 2);
    let site = mint(&h, ada, ObjectKind::Group, "Egregore").await;
    add_unsynced(&h, ada, arc, &site).await;
    h.node(ada).apply(&site, pacific_core::roles::OP_SET_ROLE, admitter(&h, arc)).await.expect("the admitter grant");

    let bundle = vec![h.node(vis).build_contact_bundle().unwrap()];
    let a = h.node(arc).admit_by_claim(&site, &claim(&site, 1), &bundle, &kiosk_keys()).await.expect("admitted once the node takes the Welcome");
    assert_eq!(a.member, h.id(vis));
    assert!(a.rooms.is_empty() && a.unjoined.is_empty(), "a Site with no room: {:?} {:?}", a.rooms, a.unjoined);
    h.settle().await;
    assert_eq!(h.node(vis).object_kind(&site).expect("the visitor holds the Site"), "group");
    assert_eq!(h.node(ada).group_state(&site).unwrap().claims_spent.len(), 1);
}

/// A Site whose Welcome has not reached the node: a refusal named for what it is, which is
/// not a definite refusal (409 at the Arc), after one look at the intro mailbox; the Arc's
/// bounded wait is its /admit's, outside its node lock. Once the node is added, the same
/// claim admits.
#[tokio::test]
async fn a_claim_for_a_site_not_on_the_arc_yet_is_a_named_retryable_refusal() {
    let h = Harness::new(&["ada", "arc", "vis"]).await;
    let (ada, arc, vis) = (0, 1, 2);
    let site = mint(&h, ada, ObjectKind::Group, "Egregore").await;

    let bundle = vec![h.node(vis).build_contact_bundle().unwrap()];
    let t = std::time::Instant::now();
    let e = h.node(arc).admit_by_claim(&site, &claim(&site, 1), &bundle, &kiosk_keys()).await.expect_err("no Site on this Arc");
    let said = e.to_string();
    assert!(said.contains("not on this Arc yet"), "named for what it is: {said}");
    assert!(!said.contains("has no owner"), "never the directory's error: {said}");
    assert!(!matches!(e, CoreError::Membership(_)), "retryable, not a definite refusal: {said}");
    assert!(t.elapsed() < std::time::Duration::from_secs(8), "bounded: {:?}", t.elapsed());

    add_unsynced(&h, ada, arc, &site).await;
    h.node(ada).apply(&site, pacific_core::roles::OP_SET_ROLE, admitter(&h, arc)).await.expect("the admitter grant");
    let a = h.node(arc).admit_by_claim(&site, &claim(&site, 1), &bundle, &kiosk_keys()).await.expect("the retry admits");
    assert_eq!(a.member, h.id(vis));
}

/// A room made, the node added and granted there, the claim presented before the node takes
/// the room's Welcome: the visitor is in the room, not answered `rooms: []`.
#[tokio::test]
async fn a_claim_before_the_arc_takes_a_rooms_welcome_joins_the_room() {
    let h = Harness::new(&["ada", "arc", "vis"]).await;
    let (ada, arc, vis) = (0, 1, 2);
    let site = h.mint_object(ada, ObjectKind::Group, "Egregore", &[arc]).await;
    h.node(ada).apply(&site, pacific_core::roles::OP_SET_ROLE, admitter(&h, arc)).await.expect("the admitter grant");
    h.settle().await;

    let room = mint(&h, ada, ObjectKind::Forum, "Talk").await;
    mark_room(&h, ada, &site, &room).await;
    add_unsynced(&h, ada, arc, &room).await;
    h.node(ada).apply(&room, pacific_core::roles::OP_SET_ROLE, admitter(&h, arc)).await.expect("the room's grant");

    let bundles: Vec<String> = (0..2).map(|_| h.node(vis).build_contact_bundle().unwrap()).collect();
    let a = h.node(arc).admit_by_claim(&site, &claim(&site, 1), &bundles, &kiosk_keys()).await.expect("admitted");
    assert_eq!(a.rooms, vec![room.clone()], "unjoined: {:?}", a.unjoined);
    assert!(a.unjoined.is_empty(), "{:?}", a.unjoined);
}

/// The room steps one at a time (runbook § 3): marked, the node not in it; the node in it,
/// not its admitter; granted. Each claim before the last names the room and why; the last
/// completes it, and the claim is spent once.
#[tokio::test]
async fn a_room_the_arc_cannot_admit_to_yet_is_named_never_a_clean_success() {
    let h = Harness::new(&["ada", "arc", "vis"]).await;
    let (ada, arc, vis) = (0, 1, 2);
    let site = h.mint_object(ada, ObjectKind::Group, "Egregore", &[arc]).await;
    h.node(ada).apply(&site, pacific_core::roles::OP_SET_ROLE, admitter(&h, arc)).await.expect("the admitter grant");
    h.settle().await;
    let room = mint(&h, ada, ObjectKind::Forum, "Talk").await;
    mark_room(&h, ada, &site, &room).await;
    h.settle().await;

    let token = claim(&site, 1);
    // Each presentation's key packages are made BEFORE the node is opened: `h.node` points the
    // process at one device's state.
    let bundles = || -> Vec<String> { (0..2).map(|_| h.node(vis).build_contact_bundle().unwrap()).collect() };
    let named = |a: &pacific_core::node::Admitted| a.unjoined.iter().map(|(r, w)| (r.clone(), w.clone())).collect::<Vec<_>>();

    // Marked; the node is not in the room.
    let b = bundles();
    let a = h.node(arc).admit_by_claim(&site, &token, &b, &kiosk_keys()).await.expect("admitted to the Site");
    assert!(a.rooms.is_empty(), "{:?}", a.rooms);
    let n = named(&a);
    assert_eq!(n.iter().map(|(r, _)| r.as_str()).collect::<Vec<_>>(), vec![room.as_str()], "the room is named: {n:?}");
    assert!(n[0].1.contains("not in the room"), "{n:?}");

    // The node in the room; no grant yet.
    h.add_to_forum(ada, arc, &room).await;
    let b = bundles();
    let a = h.node(arc).admit_by_claim(&site, &token, &b, &kiosk_keys()).await.expect("the retry");
    assert!(a.rooms.is_empty(), "{:?}", a.rooms);
    let n = named(&a);
    assert_eq!(n.iter().map(|(r, _)| r.as_str()).collect::<Vec<_>>(), vec![room.as_str()], "the room is named: {n:?}");
    assert!(n[0].1.contains("not the room's admitter"), "{n:?}");

    // Granted: the retry completes the room, and nothing is spent twice.
    h.node(ada).apply(&room, pacific_core::roles::OP_SET_ROLE, admitter(&h, arc)).await.expect("the room's grant");
    h.settle().await;
    let b = bundles();
    let a = h.node(arc).admit_by_claim(&site, &token, &b, &kiosk_keys()).await.expect("the retry completes");
    assert_eq!(a.rooms, vec![room.clone()]);
    assert!(a.unjoined.is_empty(), "{:?}", a.unjoined);
    h.settle().await;
    assert_eq!(h.node(ada).group_state(&site).unwrap().claims_spent.len(), 1, "spent once");
    assert_eq!(h.node(vis).object_kind(&room).expect("the visitor holds the room"), "forum");
}
