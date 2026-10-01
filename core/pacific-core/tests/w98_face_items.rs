//! W-98 on the Face (EVENTS-CORE): what a Site's Host carries of its events and posts.
//! G-6 (which event fields go public), NC-139 (only `network` reaches the Face), the post's
//! public page keys, and ASSURANCE's review: 3 (a post retracted or leaving `network` withdraws
//! every key it wrote) and (c) (no payload carries the Secret of an item that is not `network`).

use pacific_core::backlink::Backlink;
use pacific_core::event::EventState;
use pacific_core::face_items::{desired, desired_with, plan, Attached, Source, Write};
use pacific_core::post::{Asset, PostState};
use pacific_core::system::{HydratedItem, MAX_HYDRATED_PAYLOAD};
use pacific_core::visibility::Visibility;
use pacific_media::{Delivery, MediaKind, MediaRef};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const SITE: &str = "5117e5117e5117e5117e5117e5117e5117e5117e5117e5117e5117e5117e5117";
const NOW: i64 = 1_790_000_000_000;
const DAY: i64 = 86_400_000;
const SECRET: &str = "c2VjcmV0c2VjcmV0c2VjcmV0c2VjcmV0";

fn backlinked() -> BTreeMap<(String, String), Backlink> {
    [(("created".to_string(), SITE.to_string()), Backlink { rel: "created".into(), object: SITE.into(), at: 1 })].into()
}
fn event(title: &str) -> EventState {
    EventState { title: title.into(), start_ms: NOW + DAY, backlinks: backlinked(), ..Default::default() }
}
fn post(title: &str) -> PostState {
    PostState { title: title.into(), backlinks: backlinked(), ..Default::default() }
}
fn sealed(kind: MediaKind, mime: &str) -> MediaRef {
    MediaRef {
        kind,
        mime: mime.into(),
        delivery: Delivery::Sealed { digest: [3; 32], bytes: 900_000, key: "blobs/k".into(), secret: SECRET.into() },
        width: 0,
        height: 0,
        duration_ms: 0,
    }
}
/// A post with a sealed image and a sealed document.
fn with_media(mut p: PostState) -> PostState {
    p.assets.insert("00000000000000a1".into(), Asset { media: sealed(MediaKind::Still, "image/png"), alt: "a still".into(), at: 1 });
    p.document = Some((sealed(MediaKind::Document, "application/pdf"), "zine.pdf".into()));
    p
}
fn held(keys: &[&str]) -> BTreeMap<String, HydratedItem> {
    keys.iter()
        .map(|k| (k.to_string(), HydratedItem { key: k.to_string(), payload: "{}".into(), fetched_at: 0, rev: 1, withdrawn: false, author: [7; 32] }))
        .collect()
}

/// G-6: an event's public copy carries its zone and status, and never its join link.
#[test]
fn the_event_payload_carries_zone_and_status_and_never_the_join_link() {
    let e = EventState { tz: "Asia/Seoul".into(), status: "cancelled".into(), online: "https://meet.example.org/x".into(), ..event("Dig day") };
    let (d, left) = desired(SITE, &[Attached { id: "e1", at: 1, source: Source::Event(&e) }], NOW);
    assert!(left.is_empty(), "{left:?}");
    let p: Value = serde_json::from_str(&d["event:e1"]).unwrap();
    assert_eq!(p["tz"], "Asia/Seoul");
    assert_eq!(p["status"], "cancelled", "a cancelled event says so on the Face");
    assert!(p.get("online").is_none() && !d["event:e1"].contains("meet.example.org"), "the join link stays inside MLS");
}

/// NC-139: only `network` reaches the Face. Members only (private, connections) never does.
#[test]
fn a_members_only_event_or_post_never_reaches_the_face() {
    for v in [Visibility::Private, Visibility::Connections] {
        let e = EventState { visibility: Some(v), ..event("Members' night") };
        let p = PostState { visibility: Some(v), ..post("Members' notes") };
        let (d, _) = desired(SITE, &[Attached { id: "e1", at: 1, source: Source::Event(&e) }, Attached { id: "p1", at: 2, source: Source::Post(&p) }], NOW);
        assert!(d.is_empty(), "{v:?}: {d:?}");
    }
    let e = event("Open night");
    let (d, _) = desired(SITE, &[Attached { id: "e1", at: 1, source: Source::Event(&e) }], NOW);
    assert!(d.contains_key("event:e1"), "no visibility written is network (the ICD's default)");
}

/// The post's public page: its whole body in chunks, each within the Host item's cap, read
/// from 0 until a key is absent.
#[test]
fn a_post_body_is_carried_whole_in_chunks_within_the_cap() {
    let body: String = "é∑ \"quoted\" long read\n".repeat(4_000);
    let p = PostState { body: body.clone(), ..post("A long read") };
    let (d, left) = desired(SITE, &[Attached { id: "p1", at: 1, source: Source::Post(&p) }], NOW);
    assert!(left.is_empty(), "{left:?}");
    let n = (0..).take_while(|i| d.contains_key(&format!("post:p1:body:{i}"))).count();
    assert!(n > 1, "{n} chunks");
    let mut whole = String::new();
    for i in 0..n {
        let c = &d[&format!("post:p1:body:{i}")];
        assert!(c.len() <= MAX_HYDRATED_PAYLOAD, "chunk {i}: {} bytes", c.len());
        whole.push_str(serde_json::from_str::<Value>(c).expect("a JSON object, as every payload")["text"].as_str().unwrap());
    }
    assert_eq!(whole, body, "the chunks are the body, whole");
    assert!(!d.contains_key(&format!("post:p1:body:{n}")));
}

/// ASSURANCE (c): no payload carries the Secret of an item that is not `network`. At `network`
/// the Host is the way out of MLS, and the assets and the document go with their keys.
#[test]
fn no_payload_carries_the_secret_of_an_item_that_is_not_network() {
    for v in [Visibility::Private, Visibility::Connections] {
        let p = with_media(PostState { visibility: Some(v), ..post("Members' zine") });
        let (d, _) = desired(SITE, &[Attached { id: "p1", at: 1, source: Source::Post(&p) }], NOW);
        assert!(d.values().all(|payload| !payload.contains(SECRET)), "{v:?}");
    }
    let p = with_media(post("Public zine"));
    let (d, _) = desired(SITE, &[Attached { id: "p1", at: 1, source: Source::Post(&p) }], NOW);
    let asset: Value = serde_json::from_str(&d["post:p1:asset:00000000000000a1"]).expect("a network post's asset");
    assert_eq!(asset["assetSecret"], SECRET);
    assert_eq!(asset["alt"], "a still");
    let doc: Value = serde_json::from_str(&d["post:p1:document"]).expect("a network post's document");
    assert_eq!(doc["documentKey"], "blobs/k");
    assert_eq!(doc["name"], "zine.pdf");
}

/// ASSURANCE 3: a post retracted, or leaving `network`, withdraws every key it wrote: the post,
/// its body chunks, its assets and its document.
#[test]
fn a_post_retracted_or_leaving_network_withdraws_every_key_it_wrote() {
    let keys = ["post:p1", "post:p1:body:0", "post:p1:body:1", "post:p1:asset:00000000000000a1", "post:p1:document"];
    let gone = with_media(PostState { retracted: true, ..post("Zine") });
    let private = with_media(PostState { visibility: Some(Visibility::Private), ..post("Zine") });
    for p in [&gone, &private] {
        let (d, _) = desired(SITE, &[Attached { id: "p1", at: 1, source: Source::Post(p) }], NOW);
        let w = plan(&d, &held(&keys));
        let mut withdrawn: Vec<&str> = w.iter().filter_map(|w| match w {
            Write::Withdraw { key } => Some(key.as_str()),
            Write::Put { .. } => None,
        }).collect();
        let mut want = keys.to_vec();
        withdrawn.sort();
        want.sort();
        assert_eq!(withdrawn, want, "retracted {}: every key it wrote", p.retracted);
        assert!(w.iter().all(|w| matches!(w, Write::Withdraw { .. })), "and nothing put");
    }
}

/// ASSURANCE 2: the Host copy carries an act only where the writer holds the performer's own
/// performs_at half; an unconfirmed act stays inside MLS, and with no half given none goes out.
#[test]
fn the_host_copy_carries_only_confirmed_acts() {
    use pacific_core::event::{Act, Performer};
    let (yes, no) = ([1u8; 32], [2u8; 32]);
    let e = EventState {
        acts: Some(vec![
            Act { performer: Performer::Object(yes), role: "headliner".into(), start: None, end: None },
            Act { performer: Performer::Member(no), role: "support".into(), start: None, end: None },
        ]),
        ..event("Night")
    };
    let a = [Attached { id: "e1", at: 1, source: Source::Event(&e) }];
    let halves: BTreeMap<String, BTreeSet<String>> = [("e1".to_string(), [hex::encode(yes)].into())].into();
    let (d, _) = desired_with(SITE, &a, NOW, &halves, &BTreeSet::new());
    let p: Value = serde_json::from_str(&d["event:e1"]).unwrap();
    let acts = p["acts"].as_array().expect("the confirmed act");
    assert_eq!(acts.len(), 1);
    assert_eq!(acts[0]["object"], hex::encode(yes));
    assert!(!d["event:e1"].contains(&hex::encode(no)), "the unconfirmed act stays inside MLS");

    let (d, _) = desired(SITE, &a, NOW);
    assert!(serde_json::from_str::<Value>(&d["event:e1"]).unwrap().get("acts").is_none(), "no half given, no act out");
}
