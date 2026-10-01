//! A ROOM THE ARC JOINS LATE (Ralph, 30 Sep: "Everyone joining (paid or unpaid) should be put
//! into all groups, including healing resistance"). The Arc holds only the rows it has seen, so a
//! room a founder added it to after the room had a log gives it none of that log, and a visitor it
//! admits there gets the room without it. The founder, adding the Arc, seals it the room's history
//! (`send_history`, the founder as sender); the Arc takes it, and passes the whole log on to each
//! joiner it admits, with the founder offline.

mod common;

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use common::Harness;
use ed25519_dalek::Signer;
use pacific_core::coordinator::{ArgVal, Args, FORUM_POST};
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

const ADA: usize = 0;
const ARC: usize = 1;
const BO: usize = 2;
const VIS: usize = 3;

fn view(h: &Harness, who: usize, id: &str) -> Value {
    serde_json::from_str(&h.node(who).object_view(id).expect("a view")).unwrap()
}

#[tokio::test]
async fn a_room_the_arc_joined_late_folds_whole_for_a_visitor_it_admits() {
    let h = Harness::new(&["ada", "arc", "bo", "vis"]).await;
    let site = h.mint_object(ADA, ObjectKind::Group, "Egregore", &[ARC, BO]).await;
    // THE ROOM, WITHOUT THE ARC: ada and bo, and its log before the Arc is in it.
    let room = h.mint_object(ADA, ObjectKind::Forum, "Healing Resistance", &[BO]).await;
    let arc = hex::encode(h.id(ARC));
    {
        let f = h.node(ADA);
        f.apply(&site, pacific_core::parts::OP_SET_PART, pacific_core::parts::set_part_args(&room, "room", now_ms())).await.unwrap();
        f.apply(&room, pacific_core::parent::OP_SET_PARENT, pacific_core::parent::set_parent_args(&site, "room", now_ms())).await.unwrap();
        f.apply(&site, pacific_core::roles::OP_SET_ROLE, text(&[("member", &arc), ("role", "admitter")])).await.unwrap();
        f.apply(&room, FORUM_POST, text(&[("text", "ada, before the Arc")])).await.unwrap();
    }
    h.settle().await;
    h.node(BO).apply(&room, FORUM_POST, text(&[("text", "bo, before the Arc")])).await.unwrap();
    h.settle().await;

    // LATER: ada adds the Arc to the room, makes it the room's admitter, and seals it the room's
    // history.
    h.add_to_forum(ADA, ARC, &room).await;
    h.node(ADA).apply(&room, pacific_core::roles::OP_SET_ROLE, text(&[("member", &arc), ("role", "admitter")])).await.unwrap();
    let tag = pacific_core::handshake::parse_and_verify(&h.node(ARC).build_contact_bundle().unwrap()).unwrap().intro_tag;
    let sent = h.node(ADA).send_history(&room, &h.id(ARC), &tag).await.expect("ada sends the Arc the room's history");
    assert_eq!(sent.unsent, None, "{sent:?}");
    assert!(sent.spine > 0 && sent.posts >= 2, "the spine and both posts: {sent:?}");

    // ADA OFFLINE from here: what the visitor gets of the room is what the Arc holds.
    let online = [ARC, BO, VIS];
    h.settle_among(&online).await;
    let packages: Vec<String> = (0..2).map(|_| h.node(VIS).build_contact_bundle().unwrap()).collect();
    let a = h.node(ARC).admit_by_claim(&site, &claim(&site, 1), &packages, &kiosk_keys()).await.expect("admitted by claim");
    assert_eq!(a.rooms, vec![room.clone()], "admitted to the room: {:?}", a.unjoined);
    h.settle_among(&online).await;

    // WHOLE, for the visitor as for bo, who was there.
    assert_eq!(view(&h, VIS, &room)["display_name"], view(&h, BO, &room)["display_name"], "the room's name");
    let (theirs, ours) = (h.node(VIS).object_transcript(&room).unwrap(), h.node(BO).object_transcript(&room).unwrap());
    assert!(ours.iter().any(|l| l.contains("ada, before the Arc")), "bo sees ada's post: {ours:?}");
    assert_eq!(theirs, ours, "the visitor sees what bo sees");
    assert!(h.node(VIS).noncompliant_objects().unwrap().is_empty(), "everything the visitor holds folds");

    // The Arc holds ada's earlier post, via ada, and refused none of what she sent.
    assert!(h.node(ARC).quarantined().unwrap().is_empty(), "{:?}", h.node(ARC).quarantined().unwrap());
    let gid = hex::decode(&room).unwrap();
    let ada_post = h
        .node(ARC)
        .dir
        .load_log_signed(&gid)
        .unwrap()
        .into_iter()
        .map(|(_, e, _)| pacific_core::coordinator::decode_delta(&e).unwrap())
        .find(|d| d.op_id == FORUM_POST && matches!(d.args.get("text"), Some(ArgVal::Text(t)) if t == "ada, before the Arc"))
        .expect("the Arc holds ada's earlier post");
    assert_eq!(h.node(ARC).history_provenance(&room, &ada_post.id()).unwrap(), Some(h.id(ADA)), "via ada");
}

/// THE OWNER'S SEND (`send_history_as_owner`, the Door's /v2/history): the owner's alone, of a
/// Site or a room alone, to a member on its roster alone. A dry run sends nothing and answers what
/// would go, sealed against the one relay blob.
#[tokio::test]
async fn the_owners_send_goes_only_to_a_member_and_a_dry_run_sends_nothing() {
    let h = Harness::new(&["ada", "arc", "bo"]).await;
    let room = h.mint_object(ADA, ObjectKind::Forum, "Healing Resistance", &[BO]).await;
    let host = h.mint_object(ADA, ObjectKind::Host, "Egregore", &[]).await;
    h.node(ADA).apply(&room, FORUM_POST, text(&[("text", "ada, before the Arc")])).await.unwrap();
    h.settle().await;
    let arc = || h.node(ARC).build_contact_bundle().unwrap();

    let dry = h.node(ADA).send_history_as_owner(&room, &arc(), true).await.expect("a dry run");
    assert!(dry.unsent.is_none() && dry.posts >= 1, "ada's post would go: {dry:?}");
    assert!(dry.sealed > 0 && dry.sealed <= pacific_core::node::HISTORY_MAX_B64, "{dry:?}");

    let e = h.node(ADA).send_history_as_owner(&room, &arc(), false).await.expect_err("the Arc is not on the room");
    assert!(e.to_string().contains("is not a member of"), "{e}");
    let e = h.node(BO).send_history_as_owner(&room, &arc(), true).await.expect_err("bo is not the owner");
    assert!(e.to_string().contains("owner sends its history"), "{e}");
    let e = h.node(ADA).send_history_as_owner(&host, &arc(), true).await.expect_err("a Host");
    assert!(e.to_string().contains("carries no history"), "{e}");

    h.settle_among(&[ARC]).await;
    assert!(h.node(ARC).held_history().unwrap().is_empty() && h.node(ARC).quarantined().unwrap().is_empty(), "nothing went");
}
