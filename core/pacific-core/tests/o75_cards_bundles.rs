//! O-75, CARDS, what the joiner refuses (row 5's cards variant, `applicationPayload.history`): a
//! card is stored when its signer is on the object's roster NOW, and its subject is its signer.
//! Each bundle here is the test's own, sealed by the Arc, the joiner's admitter, to a visitor it
//! let in by its plain Add with the owner offline; each carries the Site's spine and the genuine
//! cards beside the one under test, so a refusal is of that row alone, named by its index.

mod common;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use common::Harness;
use pacific_core::coordinator::{decode_delta, ArgVal, Args};
use pacific_core::history::Row;
use pacific_core::group::{ContactCard, GroupShape};
use pacific_core::object::ObjectKind;
use pacific_core::profiles::{publish_args, Card, OP_PUBLISH_PROFILE};
use serde_json::Value;

fn text(pairs: &[(&str, &str)]) -> Args {
    pairs.iter().map(|(k, v)| (k.to_string(), ArgVal::Text(v.to_string()))).collect()
}

fn icon(seed: u8) -> String {
    let mut png = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==").unwrap();
    png.extend((0..2_000).map(|i| seed.wrapping_add(i as u8)));
    STANDARD.encode(png)
}

fn card(name: &str, seed: u8) -> Card {
    Card { display_name: name.into(), photo: Some(icon(seed)), photo_mime: Some("image/png".into()) }
}

const ADA: usize = 0;
const ARC: usize = 1;
const BO: usize = 2;
const CY: usize = 3;
const VIS: usize = 4;
const GIL: usize = 5;

/// The Site, the Arc its member and admitter, bo and cy members; ada, bo and cy name themselves
/// with an icon on their self records, and the card walk publishes each card.
/// With `cy_departs`, the owner removes cy after hers. Then the owner goes offline, and the Arc
/// adds the visitor by its plain Add, which sends no bundle.
async fn site(cy_departs: bool) -> (Harness, String) {
    let h = Harness::new(&["ada", "arc", "bo", "cy", "vis", "gil"]).await;
    let site = h.mint_object(ADA, ObjectKind::Group, "Egregore", &[ARC, BO, CY]).await;
    h.node(ADA).apply(&site, pacific_core::roles::OP_SET_ROLE, text(&[("member", &hex::encode(h.id(ARC))), ("role", "admitter")])).await.unwrap();
    h.settle().await;
    for (who, name, seed) in [(ADA, "Ada", 1u8), (BO, "Bo", 2), (CY, "Cy", 3)] {
        let photo = ContactCard { photo: icon(seed), photo_mime: "image/png".into(), ..Default::default() };
        h.node(who).set_my_profile(name, GroupShape::Individual, &photo).await.expect("a self record's card");
        h.node(who).reconcile_profiles().await.expect("the card walk");
    }
    h.settle().await;
    if cy_departs {
        h.node(ADA).group_remove_member(&site, &hex::encode(h.id(CY)), Some("requested")).await.expect("the owner removes cy");
        h.settle().await;
    }
    let bundle = h.node(VIS).build_contact_bundle().unwrap();
    h.node(ARC).group_add_member(&site, &bundle).await.expect("the admitter adds the visitor");
    h.settle_among(&[ARC, BO, VIS]).await;
    (h, site)
}

/// The Arc's rows of `object`, the spine first in chain order, then its members' cards.
fn spine_and_cards(h: &Harness, object: &str) -> (Vec<Row>, Vec<Row>) {
    let gid = hex::decode(object).unwrap();
    let mut spine = Vec::new();
    let mut cards = Vec::new();
    for (author, envelope, sig) in h.node(ARC).dir.load_log_signed(&gid).unwrap() {
        let d = decode_delta(&envelope).unwrap();
        let Some(sig) = sig else { continue };
        let row = Row { envelope, author, sig };
        if d.seq.is_some() {
            spine.push(((d.epoch, d.seq.unwrap()), row));
        } else if d.op_id == OP_PUBLISH_PROFILE {
            cards.push(row);
        }
    }
    spine.sort_by_key(|(k, _)| *k);
    (spine.into_iter().map(|(_, r)| r).collect(), cards)
}

/// A card row of `object`, its Delta as `author` would write it, signed by `signer`.
fn card_row(h: &Harness, object: &str, author: usize, signer: usize, c: &Card) -> Row {
    let gid = hex::decode(object).unwrap();
    let mut args = publish_args(c);
    args.insert("gen".into(), ArgVal::Int(1_000));
    let d = pacific_core::object::build_delta(ObjectKind::Group, OP_PUBLISH_PROFILE, args, h.epoch(ARC, object), Some(1_000));
    let sig = pacific_core::delta_sig::sign_delta(&h.node(signer).id, &gid, &d.id());
    Row { envelope: d.canonical_bytes(), author: h.id(author), sig }
}

/// Seal `rows` from the Arc to the visitor, and let it take them.
async fn send(h: &Harness, object: &str, rows: Vec<Row>) {
    let b = pacific_core::history::sign(&h.node(ARC).id, &hex::decode(object).unwrap(), &h.id(VIS), rows);
    h.node(ARC).send_history_raw(&h.node(VIS).build_contact_bundle().unwrap(), &b).await.expect("sent");
    h.settle_among(&[ARC, BO, VIS]).await;
}

fn name_of(h: &Harness, reader: usize, object: &str, member: usize) -> Option<String> {
    let v: Value = serde_json::from_str(&h.node(reader).object_view(object).ok()?).ok()?;
    v["profiles"][hex::encode(h.id(member))]["name"].as_str().map(str::to_string)
}

fn reasons(h: &Harness, who: usize) -> Vec<String> {
    h.node(who).quarantined().unwrap().into_iter().map(|(_, _, why, _)| why).collect()
}

/// A card signed by a non-member (gil, never on the roster) is refused and named, row by row;
/// the members' cards beside it are taken.
#[tokio::test]
async fn a_card_signed_by_a_non_member_is_refused_and_named() {
    let (h, site) = site(false).await;
    let (spine, cards) = spine_and_cards(&h, &site);
    let mut rows = spine;
    rows.extend(cards);
    let gils = rows.len();
    rows.push(card_row(&h, &site, GIL, GIL, &card("Gil", 7)));
    send(&h, &site, rows).await;
    assert_eq!(name_of(&h, VIS, &site, BO).as_deref(), Some("Bo"), "a member's card beside it is taken");
    assert_eq!(name_of(&h, VIS, &site, GIL), None, "gil's is not");
    let why = reasons(&h, VIS);
    assert!(why.iter().any(|r| r.starts_with(&format!("history row {gils}:"))), "gil's row, refused by name: {why:?}");
}

/// A card for another person is refused (self-authority): a row naming bo as its author that bo
/// did not sign, here the admitter's own forgery of bo's card. Bo's card stays his own, and cy's
/// genuine card lands on cy, never on bo.
#[tokio::test]
async fn a_card_for_another_person_is_refused() {
    let (h, site) = site(false).await;
    let (spine, cards) = spine_and_cards(&h, &site);
    let mut rows = spine;
    rows.extend(cards);
    let forged = rows.len();
    rows.push(card_row(&h, &site, BO, ARC, &card("Not Bo", 8)));
    send(&h, &site, rows).await;
    assert_eq!(name_of(&h, VIS, &site, BO).as_deref(), Some("Bo"), "bo's card is the one bo signed");
    assert_eq!(name_of(&h, VIS, &site, CY).as_deref(), Some("Cy"), "cy's lands on cy");
    let why = reasons(&h, VIS);
    assert!(why.iter().any(|r| r.starts_with(&format!("history row {forged}:"))), "the forged row, refused by name: {why:?}");
}

/// A departed member's card, sent anyway, is not accepted: cy signed it while a member, and is
/// not one now. Refused and named; ada's and bo's are taken.
#[tokio::test]
async fn a_departed_members_card_sent_anyway_is_not_accepted() {
    let (h, site) = site(true).await;
    let (spine, cards) = spine_and_cards(&h, &site);
    let cys = cards.iter().position(|r| r.author == h.id(CY)).expect("the Arc holds cy's card");
    let at = spine.len() + cys;
    let mut rows = spine;
    rows.extend(cards);
    send(&h, &site, rows).await;
    assert_eq!(name_of(&h, VIS, &site, ADA).as_deref(), Some("Ada"));
    assert_eq!(name_of(&h, VIS, &site, BO).as_deref(), Some("Bo"));
    assert_eq!(name_of(&h, VIS, &site, CY), None, "not cy's: she has left");
    let why = reasons(&h, VIS);
    assert!(why.iter().any(|r| r.starts_with(&format!("history row {at}:"))), "cy's row, refused by name: {why:?}");
}
