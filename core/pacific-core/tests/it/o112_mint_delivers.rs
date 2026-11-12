//! #112: the mint DELIVERS every field it collects. `every_separate_field_is_actually_delivered`
//! (`mint.rs`) pins which fields ride their own op; this is where each is read back, from the
//! folded state, on the device that minted and on another that never saw the draft.
//!
//! The draft is filled from the kind's own declaration (`mint::fields`), so a field added
//! there with no read-back here fails `read_back`'s match, and one that is collected but not
//! authored fails its assertion: the defect #112 found, every media field folding bare.

use crate::common;

use common::Harness;
use pacific_core::media::{Delivery, MediaRef};
use pacific_core::mint::{fields, Carriage, MintDraft, MintInput, MintKey};
use pacific_core::object::ObjectKind;
use serde_json::Value;

/// A jpeg's and a png's first bytes, then enough of a body to be a picture to anyone storing it.
fn jpeg() -> Vec<u8> {
    [&[0xFF, 0xD8, 0xFF, 0xE0][..], &b" an icon".repeat(400)].concat()
}
fn png() -> Vec<u8> {
    [&b"\x89PNG\r\n\x1a\n"[..], &b" a banner".repeat(4000)].concat()
}

const START: i64 = 1_790_000_000_000;

/// The draft a person would leave: every field `kind` declares, answered. `place` is the Place
/// an Event's WHERE names.
fn draft(kind: ObjectKind, place: &str) -> MintDraft {
    let mut d = MintDraft::default();
    for f in fields(kind) {
        let choice = match f.input {
            MintInput::Choice(v) => v[0].to_string(),
            _ => String::new(),
        };
        match f.key {
            MintKey::Name => d.name = format!("a {}", kind.name()),
            MintKey::Descriptor => d.descriptor = "what it is".into(),
            MintKey::Shape => d.shape = choice,
            MintKey::Category => d.category = choice,
            MintKey::Start => d.start_ms = START,
            MintKey::Fix => (d.placed, d.lat, d.lng) = (true, 51.5, -0.12),
            MintKey::Icon => d.icon = jpeg(),
            MintKey::Banner => d.banner = png(),
            MintKey::Price => d.price = "5".into(),
            MintKey::Stance => d.stance = choice,
            MintKey::Recurrence => d.recurrence = "FREQ=WEEKLY".into(),
            MintKey::Place => d.place = place.to_string(),
            // A link post, so its destination is one the fold takes.
            MintKey::Form => d.form = "link".into(),
            MintKey::Link => d.link = "https://wallflowers.io".into(),
        }
    }
    d
}

/// A view slot's detached ref, as `Node::media_get` takes it.
fn reference(slot: &Value) -> MediaRef {
    assert_eq!(slot["via"], "detached", "a detached ref, not {slot}");
    let digest: [u8; 32] = hex::decode(slot["digest"].as_str().unwrap()).unwrap().try_into().unwrap();
    MediaRef::sealed(
        slot["mime"].as_str().unwrap(),
        digest,
        slot["bytes"].as_u64().unwrap(),
        slot["key"].as_str().unwrap(),
        slot["secret"].as_str().unwrap(),
    )
    .unwrap()
}

/// Every Separate field of `kind`'s draft, read back on device `u`.
async fn read_back(h: &Harness, u: usize, kind: ObjectKind, id: &str, d: &MintDraft) {
    let k = kind.name();
    let view: Value = serde_json::from_str(&h.with_sync(u, |n| n.object_view(id)).unwrap()).unwrap();
    for f in fields(kind).iter().filter(|f| f.carriage == Carriage::Separate) {
        match f.key {
            MintKey::Icon | MintKey::Banner => {
                let (slot, bytes, mime) = match f.key {
                    MintKey::Icon => ("icon", d.icon.clone(), "image/jpeg"),
                    _ => ("banner", d.banner.clone(), "image/png"),
                };
                let r = reference(&view[slot]);
                assert_eq!(r.mime, mime, "{k}: the {slot}'s mime, read from its bytes");
                assert!(matches!(r.delivery, Delivery::Sealed { .. }), "{k}: the {slot} is sealed");
                let got = h.with(u, |n| async move { n.media_get(&r).await }).await.unwrap();
                assert_eq!(got, bytes, "{k}: the {slot}'s bytes come back");
            }
            MintKey::Place => {
                let venue = h.with_sync(u, |n| n.event_state(id)).unwrap().venue_ref.expect("the Event's WHERE");
                assert_eq!(venue.place, d.place, "{k}: happens_at names the Place");
                assert_eq!(venue.name, "a place", "{k}: named as the Place names itself");
            }
            MintKey::Fix => {
                assert!(h.with_sync(u, |n| n.place_state(id)).unwrap().location.is_some(), "{k}: the fix");
            }
            MintKey::Name => {
                assert_eq!(h.with_sync(u, |n| n.object_name(id)).unwrap(), d.name, "{k}: the name");
            }
            // The caller's: `thing.setPosture` takes a reach the draft does not carry.
            MintKey::Stance => {}
            other => panic!("{k}: {other:?} is Separate and nothing here reads it back"),
        }
    }
}

/// Every mintable kind, minted with every field it declares, reads every Separate field back:
/// on the phone that minted it, and on the laptop, which never saw the draft and holds only
/// what the log and the relay give it.
#[tokio::test]
async fn every_separate_field_folds_on_both_devices() {
    let (h, _objects) = Harness::people_with_media(&[("ada", &["phone", "laptop"])]).await;
    let (phone, laptop) = (h.device("ada", "phone"), h.device("ada", "laptop"));
    h.with_sync(phone, |n| n.ensure_self_object()).expect("the phone's self record");
    h.sync(phone).await;
    h.sync(phone).await;
    h.with(laptop, |n| async move { n.resume(None).await }).await.expect("the laptop resumes");

    let place_draft = draft(ObjectKind::Place, "");
    let place_draft = MintDraft { name: "a place".into(), ..place_draft };
    let place = h.with(phone, |n| async move { n.mint(ObjectKind::Place, &place_draft).await }).await.expect("a Place");

    let mut minted = Vec::new();
    for &kind in ObjectKind::ALL {
        if !pacific_core::mint::is_mintable(kind) || !fields(kind).iter().any(|f| f.carriage == Carriage::Separate) {
            continue;
        }
        let d = draft(kind, &place);
        let dd = d.clone();
        let id = h
            .with(phone, |n| async move { n.mint(kind, &dd).await })
            .await
            .unwrap_or_else(|e| panic!("{} mints: {e:?}", kind.name()));
        read_back(&h, phone, kind, &id, &d).await;
        minted.push((kind, id, d));
    }
    assert!(minted.iter().any(|(k, ..)| *k == ObjectKind::Post), "the sweep reached a Post");

    h.sync(phone).await;
    h.sync(phone).await;
    h.sync(laptop).await;
    for (kind, id, d) in &minted {
        read_back(&h, laptop, *kind, id, d).await;
    }
}

/// A mint whose upload fails creates nothing: no object is left bare.
#[tokio::test]
async fn a_failed_upload_creates_nothing() {
    // This relay presigns nothing.
    let h = Harness::people(&[("ada", &["phone"])]).await;
    let phone = h.device("ada", "phone");
    let before = h.with_sync(phone, |n| n.all_groups()).unwrap().len();
    let d = draft(ObjectKind::Post, "");
    let err = h.with(phone, |n| async move { n.mint(ObjectKind::Post, &d).await }).await;
    assert!(err.is_err(), "the mint is refused");
    assert_eq!(h.with_sync(phone, |n| n.all_groups()).unwrap().len(), before, "and nothing was created");
}

/// A picture the slots do not take is refused before anything is uploaded or created.
#[tokio::test]
async fn a_slot_takes_only_a_picture() {
    let (h, objects) = Harness::people_with_media(&[("ada", &["phone"])]).await;
    let phone = h.device("ada", "phone");
    let before = h.with_sync(phone, |n| n.all_groups()).unwrap().len();
    let d = MintDraft { name: "a post".into(), banner: b"%PDF-1.7 not a picture".to_vec(), ..Default::default() };
    assert!(h.with(phone, |n| async move { n.mint(ObjectKind::Post, &d).await }).await.is_err());
    assert!(objects.lock().unwrap().is_empty(), "nothing was uploaded");
    assert_eq!(h.with_sync(phone, |n| n.all_groups()).unwrap().len(), before, "nothing was created");
}
