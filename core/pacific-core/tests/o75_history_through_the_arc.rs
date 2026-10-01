//! O-75: A LATE JOINER'S HISTORY, FROM THE ARC (core/docs/launch/mdr/arc-history.md; Software
//! Assurance's pdr/history-review.md; Ralph, 28 Sep: "This comes from the always on arc-host").
//! THE CONTRACT, as tests, through real nodes and the path a kiosk visitor takes: the Arc admits by
//! claim (`admit_by_claim`) and, with each Welcome, seals the joiner alone the rows it holds of
//! that object; the joiner stores each one that proves itself, and folds as a member who was there.
//!
//! Here, what the honest path shows. The forgeries, a sender that is not the admitter, a replay,
//! and provenance need a hand-made bundle: o75_history_bundles.rs.
//!
//! Every case keeps the owner OFFLINE (`settle_among` without it), as a kiosk visitor meets a Site
//! whose founders are asleep.
//!
//! POSTS, rule (iii) (Ralph, 29 Sep: "Message history is a prerequisite for success. Use the MLS
//! welcome history package, delivered by the arc"): the Arc vouches for every earlier post it
//! holds, to the cap, each kept as via the Arc.

mod common;

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use common::Harness;
use ed25519_dalek::Signer;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::object::ObjectKind;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

fn kiosk() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[9u8; 32])
}

fn kiosk_keys() -> HashMap<String, [u8; 32]> {
    let pk = kiosk().verifying_key().to_bytes();
    HashMap::from([(pacific_core::claim::kid_of(&pk), pk)])
}

/// A claim as the kiosk issues one, for `site`, with nonce `n`.
fn claim(site: &str, n: u8) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let pk = kiosk().verifying_key().to_bytes();
    let payload = format!(
        r#"{{"k":"{}","s":"{site}","n":"{}","iat":{now},"exp":{},"c":"skills"}}"#,
        pacific_core::claim::kid_of(&pk),
        B64.encode([n; 16]),
        now + 600
    );
    let p = B64.encode(payload);
    format!("v1.{p}.{}", B64.encode(kiosk().sign(format!("v1.{p}").as_bytes()).to_bytes()))
}

fn text(pairs: &[(&str, &str)]) -> Args {
    pairs.iter().map(|(k, v)| (k.to_string(), ArgVal::Text(v.to_string()))).collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

const FACE: &str = r##"{"v":1,"look":{"colours":{"paper":"#f6f1e7"}}}"##;

/// The people: the owner, the Arc, a member who was there, the visitors, and one never a member.
const ADA: usize = 0;
const ARC: usize = 1;
const BO: usize = 2;
const VIS: usize = 3;
const GIL: usize = 4;

/// A Site as the Register and the owner's one sign-in leave it: its Host and a room, both ends of
/// each edge, a Face, the Arc a member of all three and admitter of the Site and the room; bo a
/// member who was there; `posts` posts in the room, ada's and bo's in turn, each `size` bytes.
/// Then the owner goes offline.
struct World {
    h: Harness,
    site: String,
    host: String,
    room: String,
}

async fn world(posts: usize, size: usize) -> World {
    let h = Harness::new(&["ada", "arc", "bo", "vis", "gil"]).await;
    let site = h.mint_object(ADA, ObjectKind::Group, "Egregore", &[ARC, BO]).await;
    let host = h.mint_object(ADA, ObjectKind::Host, "Egregore", &[ARC]).await;
    let room = h.mint_object(ADA, ObjectKind::Forum, "Talk", &[ARC, BO]).await;
    {
        let f = h.node(ADA);
        for (part, role) in [(&host, "host"), (&room, "room")] {
            f.apply(&site, pacific_core::parts::OP_SET_PART, pacific_core::parts::set_part_args(part, role, now_ms())).await.unwrap();
            f.apply(part, pacific_core::parent::OP_SET_PARENT, pacific_core::parent::set_parent_args(&site, role, now_ms())).await.unwrap();
        }
        f.apply(&site, pacific_core::group::OP_SET_FACE, text(&[("face", FACE)])).await.unwrap();
        let admitter = hex::encode(h.id(ARC));
        for o in [&site, &room] {
            f.apply(o, pacific_core::roles::OP_SET_ROLE, text(&[("member", &admitter), ("role", "admitter")])).await.unwrap();
        }
    }
    h.settle().await;
    for i in 0..posts {
        let who = if i % 2 == 0 { ADA } else { BO };
        let body = format!("post {i:04} {}", "x".repeat(size.saturating_sub(10)));
        h.node(who).apply(&room, pacific_core::coordinator::FORUM_POST, text(&[("text", &body)])).await.unwrap();
    }
    h.settle().await;
    World { h, site, host, room }
}

/// Everyone syncs but the owner.
async fn settle_without_the_owner(h: &Harness) {
    h.settle_among(&[ARC, BO, VIS, GIL]).await;
}

/// The Arc admits `who` by claim `n`, to the Site and the room, with the owner offline.
async fn admit(w: &World, who: usize, n: u8) {
    let packages: Vec<String> = (0..2).map(|_| w.h.node(who).build_contact_bundle().unwrap()).collect();
    let a = w.h.node(ARC).admit_by_claim(&w.site, &claim(&w.site, n), &packages, &kiosk_keys()).await.expect("admitted by claim");
    assert_eq!(a.rooms, vec![w.room.clone()], "admitted to the room too: {:?}", a.unjoined);
    settle_without_the_owner(&w.h).await;
}

fn view(h: &Harness, who: usize, id: &str) -> Value {
    serde_json::from_str(&h.node(who).object_view(id).expect("a view")).unwrap()
}

fn log_ids(h: &Harness, who: usize, id: &str) -> Vec<Vec<u8>> {
    let n = h.node(who);
    let gid = hex::decode(id).unwrap();
    n.dir
        .load_log_signed(&gid)
        .unwrap()
        .iter()
        .map(|(_, e, _)| pacific_core::coordinator::decode_delta(e).unwrap().id().to_vec())
        .collect()
}

// ─── the spine ────────────────────────────────────────────────────────────────

/// J-A'S CASE: the owner offline, a visitor the Arc admits folds the Site's name, Face and parts
/// (its Host and its room) and the room's parent, as bo, who was there, folds them.
#[tokio::test]
async fn the_spine_a_visitor_admitted_while_the_owner_is_offline_folds_the_site_as_a_member_who_was_there() {
    let w = world(0, 0).await;
    admit(&w, VIS, 1).await;
    let (theirs, ours) = (view(&w.h, VIS, &w.site), view(&w.h, BO, &w.site));
    for field in ["display_name", "face", "parts"] {
        assert_eq!(theirs[field], ours[field], "the Site's {field}, the visitor's against bo's");
    }
    assert_eq!(theirs["face"]["look"]["colours"]["paper"], "#f6f1e7", "the Face itself: {}", theirs["face"]);
    let parts: BTreeSet<(String, String)> = theirs["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["part"].as_str().unwrap().to_string(), p["role"].as_str().unwrap().to_string()))
        .collect();
    assert!(parts.contains(&(w.host.clone(), "host".into())) && parts.contains(&(w.room.clone(), "room".into())), "{parts:?}");
    assert_eq!(view(&w.h, VIS, &w.room)["parent"], view(&w.h, BO, &w.room)["parent"], "the room's parent");
    assert!(w.h.node(VIS).noncompliant_objects().unwrap().is_empty(), "everything the visitor holds folds");
}

// ─── AH-1, and what a member who was there no longer sees ─────────────────────

/// AH-1: the room's posts, after the spine, fold for the visitor as for bo.
#[tokio::test]
async fn ah1_the_rooms_posts_fold_for_the_visitor_as_for_a_member_who_was_there() {
    let w = world(12, 40).await;
    admit(&w, VIS, 1).await;
    let (theirs, ours) = (w.h.node(VIS).object_transcript(&w.room).unwrap(), w.h.node(BO).object_transcript(&w.room).unwrap());
    assert_eq!(ours.len(), 12, "bo sees the twelve");
    assert_eq!(theirs, ours, "the visitor sees what bo sees");
}

/// Retractions travel: a post its author withdrew before the visitor came stays gone for them.
#[tokio::test]
async fn a_post_withdrawn_before_the_visitor_came_stays_gone_for_them() {
    let w = world(4, 40).await;
    let mine = view(&w.h, BO, &w.room)["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["author"] == hex::encode(w.h.id(BO)))
        .cloned()
        .expect("a post of bo's");
    let withdraw: Args = [
        ("target_author".to_string(), ArgVal::Text(hex::encode(w.h.id(BO)))),
        ("target_gen".to_string(), ArgVal::Int(mine["gen"].as_i64().unwrap())),
    ]
    .into_iter()
    .collect();
    w.h.node(BO).apply(&w.room, pacific_core::coordinator::FORUM_RETRACT, withdraw).await.expect("bo withdraws it");
    settle_without_the_owner(&w.h).await;
    let ours = w.h.node(BO).object_transcript(&w.room).unwrap();
    assert_eq!(ours.len(), 3, "bo no longer sees it: {ours:?}");
    admit(&w, VIS, 1).await;
    assert_eq!(w.h.node(VIS).object_transcript(&w.room).unwrap(), ours, "nor does the visitor, and the rest is there");
}

// ─── AH-5: the same row twice ─────────────────────────────────────────────────

/// AH-5: the owner, back online, restates its standing state (NC-65); what the visitor already
/// had from the Arc is stored once, and the fold does not move.
#[tokio::test]
async fn ah5_a_row_received_again_directly_is_stored_once_and_the_fold_is_unchanged() {
    let w = world(6, 40).await;
    admit(&w, VIS, 1).await;
    let (site_before, room_before) = (view(&w.h, VIS, &w.site), view(&w.h, VIS, &w.room));
    let ids_before = log_ids(&w.h, VIS, &w.site);
    assert_eq!(site_before["face"], view(&w.h, BO, &w.site)["face"], "the Site's Face came from the Arc");

    // The owner comes back, catches up (it learns of the visitor), and restates what it owns.
    w.h.settle().await;
    assert!(w.h.node(ADA).restate_for_joiners().await.unwrap() > 0, "the owner restates, owing the visitor");
    w.h.settle().await;
    let ids_after = log_ids(&w.h, VIS, &w.site);
    let unique: BTreeSet<&Vec<u8>> = ids_after.iter().collect();
    assert_eq!(unique.len(), ids_after.len(), "no Delta stored twice");
    assert!(ids_before.iter().all(|id| ids_after.contains(id)), "nothing the Arc sent was lost");
    assert_eq!(view(&w.h, VIS, &w.site)["face"], site_before["face"], "the Face did not move");
    assert_eq!(view(&w.h, VIS, &w.site)["parts"], site_before["parts"], "nor the parts");
    assert_eq!(view(&w.h, VIS, &w.room)["messages"], room_before["messages"], "nor the room");
}

// ─── AH-6: the cap ────────────────────────────────────────────────────────────

/// The cap on one object's bundle, as the ICD states it (mls.intro.kinds.history.cap).
fn cap() -> usize {
    let icd: Value = serde_json::from_str(include_str!("../../coordination/delta-graph.icd.json")).unwrap();
    icd.pointer("/mls/intro/kinds/history/cap").and_then(Value::as_u64).expect("the ICD states the history cap") as usize
}

/// A room whose posts are over the cap, `cap / size + 40` posts of `size` bytes.
async fn over_the_cap() -> (World, usize, usize) {
    let (cap, size) = (cap(), 2048);
    let posts = cap / size + 40;
    let w = world(posts, size).await;
    admit(&w, VIS, 1).await;
    (w, posts, size)
}

/// AH-6, the spine: the room's posts over the cap take nothing from the Site's spine, which
/// arrives whole.
#[tokio::test]
async fn ah6_over_the_cap_the_spine_is_complete() {
    let (w, _, _) = over_the_cap().await;
    let (theirs, ours) = (view(&w.h, VIS, &w.site), view(&w.h, BO, &w.site));
    for field in ["display_name", "face", "parts"] {
        assert_eq!(theirs[field], ours[field], "the Site's {field}, whole");
    }
}

/// AH-6, the posts: the room's newest posts arrive, unbroken, within the cap; its oldest do not.
#[tokio::test]
async fn ah6_over_the_cap_the_newest_posts_arrive() {
    let (w, posts, size) = over_the_cap().await;
    let cap = cap();
    let (theirs, ours) = (w.h.node(VIS).object_transcript(&w.room).unwrap(), w.h.node(BO).object_transcript(&w.room).unwrap());
    assert_eq!(ours.len(), posts);
    assert!(!theirs.is_empty(), "posts arrived");
    assert!(theirs.len() < posts, "not all: the cap cut them ({} of {posts})", theirs.len());
    assert_eq!(theirs.last(), ours.last(), "the newest arrived");
    assert!(ours.ends_with(&theirs), "and what arrived is the newest, unbroken");
    assert!(theirs.len() <= cap / size, "within the cap: {} posts of {size} bytes or more", theirs.len());
}

/// A SPINE ALONE OVER THE CAP (UX-B's handback, 29 Sep; the ICD: "a spine over the cap is not
/// sent, and the admitter names it"): the Site's sequenced rows alone are over the cap, so the Arc
/// sends nothing of them. `Admitted.history` names it, "its spine alone", and counts none sent;
/// the visitor holds no row of the Site's spine, no bundle with a cut spine, and nothing refused.
/// The room's history still goes. Its cards are no chain and go regardless (`send_history`).
#[tokio::test]
async fn a_sites_spine_alone_over_the_cap_sends_none_of_it_and_says_so() {
    let (cap, size) = (cap(), 15_000);
    let w = world(3, 40).await;
    for i in 0..cap / size + 2 {
        let face = format!(r#"{{"v":1,"n":{i},"pad":"{}"}}"#, "p".repeat(size));
        w.h.node(ADA).apply(&w.site, pacific_core::group::OP_SET_FACE, text(&[("face", &face)])).await.expect("a Face under its 16 KiB");
    }
    w.h.settle().await;
    let site_gid = hex::decode(&w.site).unwrap();
    let spine_rows = |who: usize| -> Vec<Vec<u8>> {
        let n = w.h.node(who);
        let log = n.dir.load_log_signed(&site_gid).unwrap();
        log.into_iter().filter(|(_, e, _)| pacific_core::coordinator::decode_delta(e).unwrap().seq.is_some()).map(|(_, e, _)| e).collect()
    };
    let spine_bytes: usize = spine_rows(ARC).iter().map(Vec::len).sum();
    assert!(spine_bytes > cap, "the Site's spine alone, {spine_bytes} bytes of envelopes, is over the cap, {cap}");

    let packages: Vec<String> = (0..2).map(|_| w.h.node(VIS).build_contact_bundle().unwrap()).collect();
    let a = w.h.node(ARC).admit_by_claim(&w.site, &claim(&w.site, 1), &packages, &kiosk_keys()).await.expect("admitted by claim");
    assert_eq!(a.rooms, vec![w.room.clone()], "admitted to the room too: {:?}", a.unjoined);
    let site_sent = a.history.iter().find(|x| x.object == w.site).expect("the Site's report");
    assert!(site_sent.unsent.as_deref().is_some_and(|why| why.contains("its spine alone")), "named: {site_sent:?}");
    assert_eq!((site_sent.spine, site_sent.records, site_sent.posts), (0, 0, 0), "and none of it counted sent: {site_sent:?}");
    let room_sent = a.history.iter().find(|x| x.object == w.room).expect("the room's report");
    assert!(room_sent.unsent.is_none() && room_sent.spine > 0, "the room's history goes: {room_sent:?}");
    settle_without_the_owner(&w.h).await;

    assert!(spine_rows(VIS).is_empty(), "the visitor holds no row of the Site's spine: {} arrived", spine_rows(VIS).len());
    let refused: Vec<_> = w.h.node(VIS).quarantined().unwrap().into_iter().filter(|(g, ..)| g.as_deref() == Some(&site_gid[..])).collect();
    assert!(refused.is_empty(), "nor anything of the Site refused, for nothing of it came: {refused:?}");
    assert_eq!(w.h.node(VIS).object_transcript(&w.room).unwrap(), w.h.node(BO).object_transcript(&w.room).unwrap(), "the room's posts, as bo sees them");
}

// ─── AH-7, AH-8: to whom, and of what ─────────────────────────────────────────

/// AH-7: the history is the joiner's alone: bo, a member who was there, receives nothing new from
/// the visitor's admission but the membership record of it.
#[tokio::test]
async fn ah7_no_other_member_receives_the_history() {
    let w = world(8, 40).await;
    let before: Vec<BTreeSet<Vec<u8>>> = [&w.site, &w.room].iter().map(|o| log_ids(&w.h, BO, o).into_iter().collect()).collect();
    let quarantined = w.h.node(BO).quarantined().unwrap().len();
    admit(&w, VIS, 1).await;
    assert_eq!(view(&w.h, VIS, &w.site)["face"], view(&w.h, BO, &w.site)["face"], "the visitor has the Site's history");
    // What reached bo since: the records of the admission (the Add, the spent claim), nothing of
    // the history, whose rows bo held already and whose bundle was sealed to the visitor alone.
    for (o, before) in [&w.site, &w.room].into_iter().zip(before) {
        let n = w.h.node(BO);
        let gid = hex::decode(o).unwrap();
        for (_, e, _) in n.dir.load_log_signed(&gid).unwrap() {
            let d = pacific_core::coordinator::decode_delta(&e).unwrap();
            if !before.contains(&d.id().to_vec()) {
                assert!(d.seq.is_none() && d.op_id != pacific_core::coordinator::FORUM_POST, "a spine row or a post reached bo again: op {:#x}", d.op_id);
            }
        }
    }
    assert_eq!(w.h.node(BO).quarantined().unwrap().len(), quarantined, "and nothing refused either: nothing came");
}

/// AH-8: the admission is to the Site and the room: the Host's items and a DM of the Arc's never go.
#[tokio::test]
async fn ah8_the_hosts_items_and_a_dm_never_go_in_a_bundle() {
    let w = world(2, 40).await;
    {
        let f = w.h.node(ADA);
        let item: Args = [
            ("key".to_string(), ArgVal::Text("face".into())),
            ("payload".to_string(), ArgVal::Text(r#"{"v":1,"profile":{"displayName":"Egregore"}}"#.into())),
            ("fetchedAt".to_string(), ArgVal::Int(now_ms())),
            ("rev".to_string(), ArgVal::Int(now_ms())),
        ]
        .into_iter()
        .collect();
        f.apply(&w.host, pacific_core::host::OP_HYDRATE, item).await.expect("the Host's item");
    }
    w.h.settle().await;
    w.h.pair(ARC, GIL).await;
    let dm = w.h.node(ARC).conversation_with(&w.h.id(GIL)).unwrap().expect("the Arc's DM with gil");
    admit(&w, VIS, 1).await;
    assert_eq!(view(&w.h, VIS, &w.site)["face"], view(&w.h, BO, &w.site)["face"], "the visitor has the Site's history");
    assert!(log_ids(&w.h, VIS, &w.host).is_empty(), "no row of the Host");
    assert!(log_ids(&w.h, VIS, &dm).is_empty(), "no row of the Arc's DM");
}
