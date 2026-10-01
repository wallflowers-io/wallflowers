//! O-75, CARDS (TEST's run 77; BUILD, 29 Sep): a member who joins after others never saw their
//! `base.publishProfile` cards, which predate the joiner's epoch. The Arc, admitting by claim with
//! the owner OFFLINE, sends each CURRENT member's latest card (row 5's cards variant,
//! `mls.intro.kinds.history.cards`); the joiner stores a card whose signer is on the roster now.
//!
//! Each member's card is their self record's (`set_my_profile`, then the card walk,
//! `reconcile_profiles`), as the webapp writes it: a name and an icon.
//!
//! Through real nodes and the path a kiosk visitor takes (`admit_by_claim`). The forgeries are in
//! o75_cards_bundles.rs; twenty members over several blobs in o75_cards_twenty.rs; the 2 MiB total
//! in o75_cards_total_cap.rs.

mod common;

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD as B64};
use base64::Engine;
use common::Harness;
use ed25519_dalek::Signer;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::object::ObjectKind;
use pacific_core::group::{ContactCard, GroupShape};
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

/// A 1×1 png, then `pad` bytes after its end, as a larger still carries more.
fn icon(pad: usize, seed: u8) -> String {
    let mut png = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==").unwrap();
    png.extend((0..pad).map(|i| seed.wrapping_add(i as u8)));
    STANDARD.encode(png)
}

/// `who` names themselves and takes an icon on their self record; the card walk publishes it
/// into every object they are a member of.
async fn my_card(h: &Harness, who: usize, name: &str, seed: u8) {
    let photo = ContactCard { photo: icon(2_000, seed), photo_mime: "image/png".into(), ..Default::default() };
    h.node(who).set_my_profile(name, GroupShape::Individual, &photo).await.expect("a self record's card");
    h.node(who).reconcile_profiles().await.expect("the card walk");
}

const ADA: usize = 0;
const ARC: usize = 1;
const BO: usize = 2;
const CY: usize = 3;
const VIS: usize = 4;

/// A Site and its room, the Arc a member and admitter of both, bo and cy members of both. Each
/// of ada, bo and cy names themselves with an icon, and the card walk publishes it in both.
struct World {
    h: Harness,
    site: String,
    room: String,
}

async fn world() -> World {
    let h = Harness::new(&["ada", "arc", "bo", "cy", "vis"]).await;
    let site = h.mint_object(ADA, ObjectKind::Group, "Egregore", &[ARC, BO, CY]).await;
    let room = h.mint_object(ADA, ObjectKind::Forum, "Talk", &[ARC, BO, CY]).await;
    {
        let f = h.node(ADA);
        f.apply(&site, pacific_core::parts::OP_SET_PART, pacific_core::parts::set_part_args(&room, "room", now_ms())).await.unwrap();
        f.apply(&room, pacific_core::parent::OP_SET_PARENT, pacific_core::parent::set_parent_args(&site, "room", now_ms())).await.unwrap();
        let admitter = hex::encode(h.id(ARC));
        for o in [&site, &room] {
            f.apply(o, pacific_core::roles::OP_SET_ROLE, text(&[("member", &admitter), ("role", "admitter")])).await.unwrap();
        }
    }
    h.settle().await;
    for (who, name, seed) in [(ADA, "Ada", 1u8), (BO, "Bo", 2), (CY, "Cy", 3)] {
        my_card(&h, who, name, seed).await;
    }
    h.settle().await;
    World { h, site, room }
}

/// The Arc admits `who` by claim `n`, to the Site and the room, with the owner offline.
async fn admit(w: &World, who: usize, n: u8, present: &[usize]) -> pacific_core::node::Admitted {
    let packages: Vec<String> = (0..2).map(|_| w.h.node(who).build_contact_bundle().unwrap()).collect();
    let a = w.h.node(ARC).admit_by_claim(&w.site, &claim(&w.site, n), &packages, &kiosk_keys()).await.expect("admitted by claim");
    assert_eq!(a.rooms, vec![w.room.clone()], "admitted to the room too: {:?}", a.unjoined);
    w.h.settle_among(present).await;
    a
}

fn view(h: &Harness, who: usize, id: &str) -> Value {
    serde_json::from_str(&h.node(who).object_view(id).expect("a view")).unwrap()
}

/// `reader`'s card of `member` in `object`: (name, icon mime, gen), if it holds one.
fn card_of(h: &Harness, reader: usize, object: &str, member: usize) -> Option<(String, Option<String>, u64)> {
    let v = view(h, reader, object);
    let c = &v["profiles"][hex::encode(h.id(member))];
    c.is_object().then(|| (c["name"].as_str().unwrap_or_default().to_string(), c["icon"]["mime"].as_str().map(str::to_string), c["gen"].as_u64().unwrap_or(0)))
}

/// A visitor admitted after three members with icons sees every current member's card, in the
/// Site and in the room, as bo, who was there, sees them: N = 3.
#[tokio::test]
async fn a_member_joining_after_three_members_with_icons_sees_every_current_members_card() {
    let w = world().await;
    admit(&w, VIS, 1, &[ARC, BO, CY, VIS]).await;
    for o in [&w.site, &w.room] {
        for (m, name) in [(ADA, "Ada"), (BO, "Bo"), (CY, "Cy")] {
            let theirs = card_of(&w.h, VIS, o, m);
            assert_eq!(theirs.as_ref().map(|c| (c.0.as_str(), c.1.as_deref())), Some((name, Some("image/png"))), "{name}'s card, with its icon, in {o}");
            assert_eq!(theirs, card_of(&w.h, BO, o, m), "{name}'s card as bo, who was there, holds it");
        }
        w.h.node(VIS).object_compliance(o).expect("everything the visitor holds folds");
    }
}

/// A departed member's card is not sent, or not accepted: cy leaves (the owner removes her,
/// online), then the owner goes offline and the Arc admits the visitor, who sees ada's and bo's
/// cards and not cy's.
#[tokio::test]
async fn a_departed_members_card_is_not_sent_or_not_accepted() {
    let w = world().await;
    for o in [&w.site, &w.room] {
        w.h.node(ADA).group_remove_member(o, &hex::encode(w.h.id(CY)), Some("requested")).await.expect("the owner removes cy");
    }
    w.h.settle().await;
    admit(&w, VIS, 2, &[ARC, BO, VIS]).await;
    for o in [&w.site, &w.room] {
        assert!(card_of(&w.h, VIS, o, ADA).is_some() && card_of(&w.h, VIS, o, BO).is_some(), "the current members' cards, in {o}");
        assert_eq!(card_of(&w.h, VIS, o, CY), None, "not cy's, who has left {o}");
    }
}

/// A card received directly later supersedes the bundled one, last by gen: the visitor takes
/// bo's card from the Arc, then bo, online, publishes another, which the visitor holds instead.
#[tokio::test]
async fn a_card_received_directly_later_supersedes_the_bundled_one() {
    let w = world().await;
    admit(&w, VIS, 3, &[ARC, BO, CY, VIS]).await;
    let bundled = card_of(&w.h, VIS, &w.site, BO).expect("bo's card, from the Arc");
    assert_eq!(bundled.0, "Bo");
    my_card(&w.h, BO, "Bo, later", 9).await;
    w.h.settle_among(&[ARC, BO, CY, VIS]).await;
    let direct = card_of(&w.h, VIS, &w.site, BO).expect("bo's card");
    assert_eq!(direct.0, "Bo, later", "the later card, received directly");
    assert!(direct.2 > bundled.2, "by gen: {direct:?} after {bundled:?}");
    assert_eq!(direct, card_of(&w.h, BO, &w.site, BO).unwrap(), "as bo holds his own");
}
