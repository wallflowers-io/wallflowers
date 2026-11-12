//! THE HOSTED EXTERIOR (`media.hosted`, Ralph 12 Nov) — real nodes, a real relay, object storage
//! on this machine.
//!
//! The outside is Luma-complete, and a change to it travels once: the owner's device overwrites
//! one document at one key, and every reader fetches it. The carried exterior is the pointer and
//! does not move. Owner and Cleo are connections; Sam is Cleo's, two hops from the roster.

use crate::common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::exterior::DoorMode;
use pacific_core::node::ExteriorDraft;
use pacific_core::visibility::Visibility;

const OWNER: usize = 0;
const CLEO: usize = 1;
const SAM: usize = 2;

fn text(pairs: &[(&str, &str)]) -> Args {
    pairs.iter().map(|(k, v)| (k.to_string(), ArgVal::Text(v.to_string()))).collect()
}

async fn gig(h: &Harness) -> String {
    let obj = h.node(OWNER).object_new("event", "Gig").expect("mint");
    let mut p = text(&[("title", "Gig"), ("venue", "The Lantern")]);
    p.insert("startMs".into(), ArgVal::Int(1_800_000_000_000));
    h.node(OWNER).edit(&obj, "event.setProfile", p).await.expect("the profile");
    let at = pacific_core::geo::LocationSource::Fixed { point: pacific_core::geo::GeoPoint::from_degrees(51.4545, -2.5879) };
    h.node(OWNER).apply_local(&obj, pacific_core::geo::OP_SET_LOCATION, pacific_core::geo::set_location_args(&at)).await.expect("the place");
    let draft = ExteriorDraft { name: "Gig".into(), door: Some(DoorMode::Auto), precision: 5, ..Default::default() };
    h.node(OWNER).set_outside(&obj, Some(Visibility::Network), Some(&draft)).await.expect("the outside");
    obj
}

fn pointer(h: &Harness, u: usize, obj: &str) -> pacific_core::exterior::Carried {
    h.node(u).exteriors().unwrap().into_iter().find(|e| e.carried.object == obj).expect("the exterior reached them").carried
}

#[tokio::test]
async fn the_outside_is_whole_and_a_change_to_it_is_not_carried_again() {
    let (h, objects) = Harness::people_with_media(&[("Owner", &["a"]), ("Cleo", &["a"]), ("Sam", &["a"])]).await;
    h.pair(OWNER, CLEO).await;
    h.pair(CLEO, SAM).await;
    let obj = gig(&h).await;
    h.settle().await;

    // Sam, two hops out, reads the Event whole.
    let f = h.node(SAM).refresh_exterior(&obj).await.unwrap().expect("a hosted outside");
    assert_eq!((f["title"].as_str(), f["venue"].as_str(), f["startMs"].as_i64()), (Some("Gig"), Some("The Lantern"), Some(1_800_000_000_000)));
    assert_eq!(f["point"].as_str(), Some("gcnht"), "the Place at the owner's five characters");
    assert_eq!(f["going"].as_i64(), Some(0));
    assert_eq!(f["hosts"][0]["key"].as_str(), Some(hex::encode(h.id(OWNER)).as_str()));
    let hosted = pointer(&h, SAM, &obj).exterior.outside.hosted.expect("the pointer names the document");
    let path = format!("/pacific-media/{}", hosted.key);
    let first = objects.lock().unwrap().get(&path).cloned().expect("one document, at its key");

    // Cleo attends. The count changes outside; the pointer does not move.
    let (sam_pos, owner_row) = (pointer(&h, SAM, &obj).pos, h.node(OWNER).exterior_state(&obj).unwrap().unwrap().1.exterior);
    h.node(CLEO).request_join(&obj, "").await.unwrap();
    h.settle().await;
    assert!(h.roster(OWNER, &obj).contains(&h.id(CLEO)), "the auto entry let Cleo in");
    h.settle().await;
    let f = h.node(SAM).refresh_exterior(&obj).await.unwrap().unwrap();
    assert_eq!(f["going"].as_i64(), Some(1), "one attendee, seen two hops out");
    assert_eq!(pointer(&h, SAM, &obj).pos, sam_pos, "no new exterior was carried to Sam");
    assert_eq!(h.node(OWNER).exterior_state(&obj).unwrap().unwrap().1.exterior, owner_row, "no new base.setExterior was written");
    assert_eq!(objects.lock().unwrap().keys().filter(|k| **k == path).count(), 1, "overwritten in place");

    // An older document put back is refused: Sam keeps the newest he has opened.
    objects.lock().unwrap().insert(path.clone(), first);
    let f = h.node(SAM).refresh_exterior(&obj).await.unwrap().unwrap();
    assert_eq!(f["going"].as_i64(), Some(1), "a replayed document does not roll the outside back");

    // A settled node writes nothing more.
    let held = objects.lock().unwrap().get(&path).cloned();
    h.settle().await;
    assert_eq!(objects.lock().unwrap().get(&path).cloned(), held, "nothing changed, nothing written");

    // Private: the document is a tombstone, signed by its owner.
    h.node(OWNER).set_visibility(&obj, Visibility::Private).await.unwrap();
    h.settle().await;
    let sealed = objects.lock().unwrap().get(&path).cloned().unwrap();
    let doc = pacific_core::hosted::open(&sealed, &hosted, &h.id(OWNER), &obj, pacific_core::object::ObjectKind::Event).unwrap();
    assert_eq!(doc.body, pacific_core::hosted::Body::Tombstone);
    assert!(h.node(SAM).exterior_fields(&obj).await.unwrap().is_none(), "withdrawn where it is read");
}

#[tokio::test]
async fn a_posts_body_goes_outside_only_when_its_poster_says() {
    let (h, _) = Harness::people_with_media(&[("Owner", &["a"]), ("Cleo", &["a"])]).await;
    h.pair(OWNER, CLEO).await;
    let obj = h.node(OWNER).object_new("post", "Notes").expect("mint");
    h.node(OWNER).edit(&obj, "post.setProfile", text(&[("title", "Notes"), ("body", "the words"), ("excerpt", "a teaser")])).await.expect("the post");
    let mut draft = ExteriorDraft { name: "Notes".into(), door: Some(DoorMode::Review), ..Default::default() };
    h.node(OWNER).set_outside(&obj, Some(Visibility::Connections), Some(&draft)).await.unwrap();
    h.settle().await;
    let f = h.node(CLEO).refresh_exterior(&obj).await.unwrap().unwrap();
    assert_eq!((f["excerpt"].as_str(), f.get("body")), (Some("a teaser"), None), "the teaser alone");

    draft.readable_outside = true;
    h.node(OWNER).set_outside(&obj, None, Some(&draft)).await.unwrap();
    h.settle().await;
    let f = h.node(CLEO).refresh_exterior(&obj).await.unwrap().unwrap();
    assert_eq!(f["body"].as_str(), Some("the words"), "readable outside, as its poster said");
}
