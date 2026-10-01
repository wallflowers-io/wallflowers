//! O-77 (Ralph, 28 Sep: "All members of a community are required to publish a profile card on
//! entry, which includes Name, profile icon and public key"; ICD 2.1.0 row 4). A member's card,
//! `base.publishProfile` on the `profiles` facet, carries a name and a picture; its subject is
//! its author, whose key the signature already is. A visitor admitted by a kiosk's claim, once
//! named on their self record, shows by that name in the Site and in its room for every other
//! member, and a rename is published again. A card names no one else, and keeps to its cap.

mod common;

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use common::Harness;
use ed25519_dalek::Signer;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::object::ObjectKind;
use serde_json::Value;
use std::collections::HashMap;

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
        r#"{{"k":"{}","s":"{site}","n":"{}","iat":{now},"exp":{}}}"#,
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

/// `who`'s name on their own self record, as the webapp writes it: `group.setProfile` through
/// the one write path.
async fn name_myself(h: &Harness, who: usize, name: &str) {
    let n = h.node(who);
    let me = n.self_object().unwrap().expect("a self record");
    n.apply(&me, pacific_core::group::OP_SET_PROFILE, text(&[("displayName", name), ("shape", "individual")]))
        .await
        .expect("the self record takes its name");
}

/// The name `of`'s card carries in `object`, as `reader` folds the object's view: `profiles`
/// is `{<member hex>: {name, icon: {mime, data} | null, gen}}` (webapp REQUIREMENTS § 3).
fn card_in(h: &Harness, reader: usize, object: &str, of: usize) -> Option<Value> {
    let view: Value = serde_json::from_str(&h.node(reader).object_view(object).unwrap()).unwrap();
    let card = view["profiles"].get(hex::encode(h.id(of)))?;
    assert!(card["gen"].as_u64().is_some(), "a card names its gen: {card}");
    Some(serde_json::json!({ "name": card["name"], "icon": card["icon"] }))
}

/// The Site, its room, the Arc's node as their admitter, and `eve` a member of both.
async fn egregore(h: &Harness, ada: usize, arc: usize, eve: usize) -> (String, String) {
    let site = h.mint_object(ada, ObjectKind::Group, "Egregore", &[arc, eve]).await;
    let room = h.mint_object(ada, ObjectKind::Forum, "Talk", &[arc, eve]).await;
    let founder = h.node(ada);
    founder
        .apply(&site, pacific_core::parts::OP_SET_PART, pacific_core::parts::set_part_args(&room, "room", now_ms()))
        .await
        .unwrap();
    founder
        .apply(&room, pacific_core::parent::OP_SET_PARENT, pacific_core::parent::set_parent_args(&site, "room", now_ms()))
        .await
        .unwrap();
    let admitter = hex::encode(h.id(arc));
    for o in [&site, &room] {
        founder.apply(o, pacific_core::roles::OP_SET_ROLE, text(&[("member", &admitter), ("role", "admitter")])).await.unwrap();
    }
    h.settle().await;
    (site, room)
}

/// The visitor, admitted by a claim with no name yet (a kiosk's sign-up asks none), then named:
/// every other member reads them by that name in the Site and in the room. A rename is
/// published again.
#[tokio::test]
async fn a_visitor_admitted_by_claim_then_named_shows_by_name_in_the_site_and_the_room() {
    let h = Harness::new(&["ada", "arc", "", "eve"]).await;
    let (ada, arc, vis, eve) = (0, 1, 2, 3);
    let (site, room) = egregore(&h, ada, arc, eve).await;
    let bundles: Vec<String> = (0..2).map(|_| h.node(vis).build_contact_bundle().unwrap()).collect();
    let admitted = h.node(arc).admit_by_claim(&site, &claim(&site, 1), &bundles, &kiosk_keys()).await.expect("admitted");
    assert_eq!(admitted.rooms, vec![room.clone()], "{:?}", admitted.unjoined);
    h.settle().await;
    // Admitted and not yet named: no card, and the webapp draws a hue and an initial.
    assert_eq!(card_in(&h, eve, &site, vis), None, "no name, no card");

    name_myself(&h, vis, "Vis").await;
    h.settle().await;
    for object in [&site, &room] {
        assert_eq!(card_in(&h, eve, object, vis), Some(serde_json::json!({ "name": "Vis", "icon": null })), "{object}, as eve reads it");
        assert_eq!(card_in(&h, ada, object, vis), Some(serde_json::json!({ "name": "Vis", "icon": null })), "{object}, as ada reads it");
    }

    name_myself(&h, vis, "Vis Aurelia").await;
    h.settle().await;
    for object in [&site, &room] {
        assert_eq!(card_in(&h, eve, object, vis), Some(serde_json::json!({ "name": "Vis Aurelia", "icon": null })), "renamed, in {object}");
    }
    assert_eq!(card_in(&h, eve, &site, ada), Some(serde_json::json!({ "name": "ada", "icon": null })), "the founder's own card, beside it");
}

/// A card is its author's own: it has no field to name anyone else, so one that tries is
/// refused, and whatever name it carries is shown against its author's key, never moving
/// another's. Over the cap is refused, never cut.
#[tokio::test]
async fn a_card_names_no_one_else_and_keeps_to_its_cap() {
    let h = Harness::new(&["ada", "eve"]).await;
    let (ada, eve) = (0, 1);
    let site = h.mint_object(ada, ObjectKind::Group, "Egregore", &[eve]).await;
    let op = pacific_core::authoring::op_on("group", "base.publishProfile").expect("a group carries base.publishProfile");
    let publish = |card: String| {
        let n = h.node(eve);
        let site = site.clone();
        async move { n.apply(&site, op.op_id, text(&[("card", &card)])).await }
    };
    let ada_hex = hex::encode(h.id(ada));
    for claims_another in [
        serde_json::json!({ "displayName": "Ada", "member": ada_hex }),
        serde_json::json!({ "displayName": "Ada", "publicKey": ada_hex }),
    ] {
        assert!(publish(claims_another.to_string()).await.is_err(), "a card naming another member is refused: {claims_another}");
    }
    let over = "A".repeat(16_384);
    assert!(publish(serde_json::json!({ "displayName": "Eve", "photo": over, "photo_mime": "image/webp" }).to_string()).await.is_err(), "over the cap: refused");
    assert!(publish(serde_json::json!({ "displayName": "" }).to_string()).await.is_err(), "a card with no name");

    // Eve names herself "ada" on her own record: her card says so, against her key, and
    // Ada's own card is Ada's still.
    publish(serde_json::json!({ "displayName": "ada" }).to_string()).await.expect("her own card, any name");
    name_myself(&h, eve, "ada").await;
    h.settle().await;
    assert_eq!(card_in(&h, ada, &site, eve), Some(serde_json::json!({ "name": "ada", "icon": null })), "under eve's key");
    let view: Value = serde_json::from_str(&h.node(ada).object_view(&site).unwrap()).unwrap();
    assert_eq!(view["profiles"].as_object().map(|p| p.len()), Some(2), "two cards, each its author's: {}", view["profiles"]);
    assert_eq!(card_in(&h, eve, &site, ada), Some(serde_json::json!({ "name": "ada", "icon": null })), "ada's own, unmoved");
}

/// The self record is the card's one source: a card written straight into an object (a site's
/// pass through /v2/apply, say) gives way to the record's own at the author's next reconcile,
/// even when nothing about the record changed. One Node throughout, as the Door holds one: the
/// walk is memoised on it, and a straight write must not hide behind the memo.
#[tokio::test]
async fn a_card_written_straight_to_an_object_gives_way_to_the_self_records() {
    let h = Harness::new(&["ada", "eve"]).await;
    let (ada, eve) = (0, 1);
    let site = h.mint_object(ada, ObjectKind::Group, "Egregore", &[eve]).await;
    let n = h.node(eve);
    n.reconcile_profiles().await.expect("a reconcile");
    assert_eq!(n.reconcile_profiles().await.unwrap().published, 0, "settled: nothing to write");
    let op = pacific_core::authoring::op_on("group", "base.publishProfile").unwrap();
    let straight = serde_json::json!({ "displayName": "Someone Else" }).to_string();
    n.apply(&site, op.op_id, text(&[("card", &straight)])).await.expect("a member's own card, written straight");
    assert_eq!(n.reconcile_profiles().await.unwrap().published, 1, "the record's card, published over it");
    drop(n);
    h.settle().await;
    assert_eq!(card_in(&h, ada, &site, eve), Some(serde_json::json!({ "name": "eve", "icon": null })), "the record's card again");
}

/// `who`'s name and picture on their own self record, as the webapp writes them: group.setProfile
/// with a ContactCard carrying `photo` (standard base64) and `photo_mime`.
async fn picture_myself(h: &Harness, who: usize, name: &str, photo: &str) {
    let n = h.node(who);
    let me = n.self_object().unwrap().expect("a self record");
    let card = serde_json::json!({ "photo": photo, "photo_mime": "image/webp" }).to_string();
    n.apply(&me, pacific_core::group::OP_SET_PROFILE, text(&[("displayName", name), ("shape", "individual"), ("card", &card)]))
        .await
        .expect("the self record takes its name and picture");
}

/// The picture at the webapp's real size (a 128 px webp, under 11,000 base64 characters until
/// 2.2.0; BUILD, 29 Sep) rides the card, and another member reads it in the Site's view.
#[tokio::test]
async fn an_eleven_thousand_character_picture_rides_the_card() {
    let h = Harness::new(&["ada", "eve"]).await;
    let (ada, eve) = (0, 1);
    let site = h.mint_object(ada, ObjectKind::Group, "Egregore", &[eve]).await;
    let photo = "A".repeat(11_000);
    picture_myself(&h, eve, "Eve", &photo).await;
    let (card, left_out) = h.node(eve).my_card().unwrap();
    assert_eq!(left_out, None, "nothing left out");
    assert!(card.unwrap().to_arg().len() < pacific_core::profiles::MAX_CARD_BYTES);
    h.settle().await;
    let view: Value = serde_json::from_str(&h.node(ada).object_view(&site).unwrap()).unwrap();
    let theirs = &view["profiles"][hex::encode(h.id(eve))];
    assert_eq!(theirs["name"], "Eve");
    assert_eq!(theirs["icon"]["mime"], "image/webp");
    assert_eq!(theirs["icon"]["data"].as_str().map(str::len), Some(11_000), "the whole picture, as ada reads it");
}

/// The cap at its edge: a card of exactly 16,384 bytes is taken; one byte more is refused,
/// never cut.
#[tokio::test]
async fn a_card_of_16384_bytes_is_taken_and_one_byte_more_refused() {
    let h = Harness::new(&["ada", "eve"]).await;
    let (ada, eve) = (0, 1);
    let site = h.mint_object(ada, ObjectKind::Group, "Egregore", &[eve]).await;
    let cap = pacific_core::profiles::MAX_CARD_BYTES;
    assert_eq!(cap, 16_384);
    // A picture in whole base64 quanta, and the name padded to land the card on `bytes`.
    let card_of = |bytes: usize| -> String {
        let shell = serde_json::json!({ "displayName": "E", "photo": "", "photo_mime": "image/webp" }).to_string().len();
        let photo = (bytes - shell) / 4 * 4;
        let name = format!("E{}", "e".repeat(bytes - shell - photo));
        let card = serde_json::json!({ "displayName": name, "photo": "A".repeat(photo), "photo_mime": "image/webp" }).to_string();
        assert_eq!(card.len(), bytes);
        card
    };
    let op = pacific_core::authoring::op_on("group", "base.publishProfile").unwrap();
    let n = h.node(eve);
    n.apply(&site, op.op_id, text(&[("card", &card_of(cap))])).await.expect("16,384 bytes: taken");
    let over = n.apply(&site, op.op_id, text(&[("card", &card_of(cap + 1))])).await;
    assert!(over.is_err(), "16,385 bytes: refused, not truncated");
}

/// A self record's picture too big for the card: the card goes with the name alone, and core
/// says why, for the webapp to ask for a smaller one.
#[tokio::test]
async fn a_picture_too_big_for_the_card_is_left_out_and_said() {
    let h = Harness::new(&["ada", "eve"]).await;
    let (ada, eve) = (0, 1);
    let site = h.mint_object(ada, ObjectKind::Group, "Egregore", &[eve]).await;
    picture_myself(&h, eve, "Eve", &"A".repeat(20_000)).await;
    let (card, left_out) = h.node(eve).my_card().unwrap();
    assert_eq!(card.map(|c| (c.display_name, c.photo)), Some(("Eve".to_string(), None)), "the name alone");
    let said = left_out.expect("said, not dropped");
    assert!(said.starts_with("icon left out: the card is ") && said.contains("over the 16384 a card holds"), "{said}");
    h.settle().await;
    assert_eq!(card_in(&h, ada, &site, eve), Some(serde_json::json!({ "name": "Eve", "icon": null })), "name-only, as ada reads it");
}
