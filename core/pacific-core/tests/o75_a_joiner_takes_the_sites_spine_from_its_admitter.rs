//! O-75, the joiner's ingest (mdr/arc-history.md; J-A): a visitor the Arc admits while the Site's
//! owner is offline holds none of the owner's spine, so it folds the Site with no name or parts.
//! A history bundle, the Site's owner-signed spine sealed to the visitor alone, gives it both,
//! each row kept with the admitter it came via. A bundle from anyone else is refused, named, and
//! so is one sent for another member.
//!
//! A later joiner never sees an earlier member's card (`base.publishProfile`, O-77), which
//! predates the joiner's epoch (TEST's run 77, J8b): a bundle of cards gives it them, each by a
//! member of the roster the joiner holds, taken alone after the spine, held when it comes first.
//!
//! THE INGEST ALONE: the visitor is admitted by the Arc's plain Add (`group_add_member`), which
//! sends no bundle, so each bundle here is the test's own. The Arc's send at a claim admission
//! (`admit_by_claim`) is BW-B's, and J-A end to end through it is in o75_history_through_the_arc.

mod common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::object::ObjectKind;
use serde_json::Value;

fn text(pairs: &[(&str, &str)]) -> Args {
    pairs.iter().map(|(k, v)| (k.to_string(), ArgVal::Text(v.to_string()))).collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// The rows `holder` keeps of `object`, as a bundle carries them: the owner's sequenced spine
/// first, in chain order; then the rest.
fn rows_of(h: &Harness, holder: usize, object: &str) -> Vec<pacific_core::history::Row> {
    let gid = hex::decode(object).unwrap();
    let mut rows: Vec<(pacific_core::coordinator::Delta, pacific_core::history::Row)> = h
        .node(holder)
        .dir
        .load_log_signed(&gid)
        .unwrap()
        .into_iter()
        .filter_map(|(author, envelope, sig)| {
            let d = pacific_core::coordinator::decode_delta(&envelope).ok()?;
            Some((d, pacific_core::history::Row { envelope, author, sig: sig? }))
        })
        .collect();
    rows.sort_by_key(|(d, _)| (d.seq.is_none(), d.epoch, d.seq.unwrap_or(0)));
    rows.into_iter().map(|(_, r)| r).collect()
}

/// Of `rows`, the spine's (sequenced) and the cards (`base.publishProfile`), apart.
fn spine_and_cards(rows: Vec<pacific_core::history::Row>) -> (Vec<pacific_core::history::Row>, Vec<pacific_core::history::Row>) {
    let d = |r: &pacific_core::history::Row| pacific_core::coordinator::decode_delta(&r.envelope).unwrap();
    let cards = rows.iter().filter(|r| pacific_core::profiles::is_profile_op(d(r).op_id)).cloned().collect();
    (rows.into_iter().filter(|r| d(r).seq.is_some()).collect(), cards)
}

/// `who`'s name on their own self record, as the webapp writes it; their card follows (O-77).
async fn name_myself(h: &Harness, who: usize, name: &str) {
    let n = h.node(who);
    let me = n.self_object().unwrap().expect("a self record");
    n.apply(&me, pacific_core::group::OP_SET_PROFILE, text(&[("displayName", name), ("shape", "individual")])).await.expect("the self record takes its name");
}

/// The name on `of`'s card in `object`, as `reader` folds it.
fn card_name(h: &Harness, reader: usize, object: &str, of: usize) -> Option<String> {
    view(h, reader, object)?["profiles"].get(hex::encode(h.id(of)))?["name"].as_str().map(str::to_string)
}

fn view(h: &Harness, reader: usize, object: &str) -> Option<Value> {
    h.node(reader).object_view(object).ok().and_then(|v| serde_json::from_str(&v).ok())
}

/// The Site and its room, the Arc their admitter, the owner a founder who then goes offline.
async fn a_site_whose_owner_goes_offline(h: &Harness, ada: usize, arc: usize) -> (String, String) {
    let draft = pacific_core::mint::MintDraft { name: "Egregore".into(), shape: "community".into(), ..Default::default() };
    let site = h.node(ada).mint(ObjectKind::Group, &draft).await.unwrap();
    let room = h.mint_object(ada, ObjectKind::Forum, "Talk", &[]).await;
    let founder = h.node(ada);
    founder.apply(&site, pacific_core::parts::OP_SET_PART, pacific_core::parts::set_part_args(&room, "room", now_ms())).await.unwrap();
    founder.apply(&room, pacific_core::parent::OP_SET_PARENT, pacific_core::parent::set_parent_args(&site, "room", now_ms())).await.unwrap();
    drop(founder);
    h.add_to_forum(ada, arc, &site).await;
    h.add_to_forum(ada, arc, &room).await;
    let admitter = hex::encode(h.id(arc));
    for o in [&site, &room] {
        h.node(ada).apply(o, pacific_core::roles::OP_SET_ROLE, text(&[("member", &admitter), ("role", "admitter")])).await.unwrap();
    }
    h.settle().await;
    (site, room)
}

#[tokio::test]
async fn a_visitor_admitted_while_the_owner_is_offline_takes_the_sites_spine_from_the_arc() {
    let h = Harness::new(&["ada", "arc", "vis"]).await;
    let (ada, arc, vis) = (0, 1, 2);
    let (site, _room) = a_site_whose_owner_goes_offline(&h, ada, arc).await;
    // The owner is away from here on: only the Arc and the visitor sync. The Arc admits by its
    // plain Add, which sends no history.
    let bundle = h.node(vis).build_contact_bundle().unwrap();
    h.node(arc).group_add_member(&site, &bundle).await.expect("the admitter adds the visitor");
    h.settle_among(&[arc, vis]).await;
    let before = view(&h, vis, &site);
    assert_ne!(before.as_ref().and_then(|v| v["display_name"].as_str()), Some("Egregore"), "J-A: the visitor folds no name yet: {before:?}");

    let rows = rows_of(&h, arc, &site);
    assert!(!rows.is_empty());
    let b = pacific_core::history::sign(&h.node(arc).id, &hex::decode(&site).unwrap(), &h.id(vis), rows);
    let contact = h.node(vis).build_contact_bundle().unwrap();
    h.node(arc).send_history_raw(&contact, &b).await.expect("sent");
    h.settle_among(&[arc, vis]).await;

    let after = view(&h, vis, &site).expect("the Site folds for the visitor");
    assert_eq!(after["display_name"], "Egregore", "the name, from the owner's spine: {after}");
    assert!(after["parts"].to_string().contains("room"), "the room, a part of the Site: {}", after["parts"]);
    h.node(vis).object_compliance(&site).expect("the Site folds whole");
    // Each spine row keeps the admitter it came via.
    let spine_row = b.rows.iter().find(|r| pacific_core::coordinator::decode_delta(&r.envelope).unwrap().seq.is_some()).unwrap();
    let id = pacific_core::coordinator::decode_delta(&spine_row.envelope).unwrap().id();
    assert_eq!(h.node(vis).history_provenance(&site, &id).unwrap(), Some(h.id(arc)), "via the Arc");
}

#[tokio::test]
async fn a_bundle_from_anyone_but_the_admitter_or_for_another_member_is_refused_named() {
    let h = Harness::new(&["ada", "arc", "vis", "eve"]).await;
    let (ada, arc, vis, eve) = (0, 1, 2, 3);
    let (site, _room) = a_site_whose_owner_goes_offline(&h, ada, arc).await;
    h.add_to_forum(ada, eve, &site).await;
    let bundle = h.node(vis).build_contact_bundle().unwrap();
    h.node(arc).group_add_member(&site, &bundle).await.expect("the admitter adds the visitor");
    h.settle_among(&[arc, vis, eve]).await;
    let gid = hex::decode(&site).unwrap();
    let rows = rows_of(&h, arc, &site);
    let contact = h.node(vis).build_contact_bundle().unwrap();

    // Eve holds the rows too, but she did not let the visitor in.
    let from_eve = pacific_core::history::sign(&h.node(eve).id, &gid, &h.id(vis), rows.clone());
    h.node(eve).send_history_raw(&contact, &from_eve).await.unwrap();
    // The Arc's bundle for eve, sent to the visitor's mailbox: a replay to another member.
    let for_eve = pacific_core::history::sign(&h.node(arc).id, &gid, &h.id(eve), rows);
    h.node(arc).send_history_raw(&contact, &for_eve).await.unwrap();
    h.settle_among(&[arc, vis, eve]).await;

    let reasons: Vec<String> = h.node(vis).quarantined().unwrap().into_iter().map(|(_, _, why, _)| why).collect();
    assert!(reasons.iter().any(|r| r.contains("history bundle: sent by") && r.contains("committed this device's Add")), "{reasons:?}");
    assert!(reasons.iter().any(|r| r.contains("history bundle: for ") && r.contains("not this device's person")), "{reasons:?}");
    assert_ne!(view(&h, vis, &site).and_then(|v| v["display_name"].as_str().map(str::to_string)).as_deref(), Some("Egregore"), "nothing of either was taken");
}

#[tokio::test]
async fn a_later_joiner_takes_an_earlier_members_card_from_the_admitter_held_until_its_spine() {
    let h = Harness::new(&["ada", "arc", "vis", "bo"]).await;
    let (ada, arc, vis, bo) = (0, 1, 2, 3);
    let (site, _room) = a_site_whose_owner_goes_offline(&h, ada, arc).await;
    h.add_to_forum(ada, bo, &site).await;
    name_myself(&h, bo, "Bo").await;
    h.settle().await;
    assert_eq!(card_name(&h, arc, &site, bo).as_deref(), Some("Bo"), "the Arc holds bo's card");
    let bundle = h.node(vis).build_contact_bundle().unwrap();
    h.node(arc).group_add_member(&site, &bundle).await.expect("the admitter adds the visitor");
    h.settle_among(&[arc, vis]).await;
    assert_eq!(card_name(&h, vis, &site, bo), None, "J8b: bo's card predates the visitor's epoch");

    let gid = hex::decode(&site).unwrap();
    let (spine, cards) = spine_and_cards(rows_of(&h, arc, &site));
    assert!(!spine.is_empty() && !cards.is_empty());
    let contact = h.node(vis).build_contact_bundle().unwrap();
    // The cards before their spine: held, as a bundle before its Welcome is.
    let of_cards = pacific_core::history::sign(&h.node(arc).id, &gid, &h.id(vis), cards.clone());
    h.node(arc).send_history_raw(&contact, &of_cards).await.expect("sent");
    h.settle_among(&[arc, vis]).await;
    let held = h.node(vis).held_history().unwrap();
    assert!(held.iter().any(|(o, from, n, _)| o == &site && from == &h.id(arc) && *n == cards.len()), "held for its spine: {held:?}");
    assert_eq!(card_name(&h, vis, &site, bo), None, "nothing of it taken yet");

    let of_spine = pacific_core::history::sign(&h.node(arc).id, &gid, &h.id(vis), spine);
    h.node(arc).send_history_raw(&contact, &of_spine).await.expect("sent");
    h.settle_among(&[arc, vis]).await;
    assert_eq!(card_name(&h, vis, &site, bo).as_deref(), Some("Bo"), "bo's card, from the admitter");
    assert!(!h.node(vis).held_history().unwrap().iter().any(|(o, ..)| o == &site), "no longer held");
    h.node(vis).object_compliance(&site).expect("the Site folds whole");
    let card = cards.iter().map(|r| pacific_core::coordinator::decode_delta(&r.envelope).unwrap()).find(|_| true).unwrap();
    assert_eq!(h.node(vis).history_provenance(&site, &card.id()).unwrap(), Some(h.id(arc)), "via the Arc");

    // A card that comes directly later supersedes it: last one wins by gen.
    name_myself(&h, bo, "Bo Two").await;
    h.settle_among(&[arc, vis, bo]).await;
    assert_eq!(card_name(&h, vis, &site, bo).as_deref(), Some("Bo Two"));
}

#[tokio::test]
async fn a_card_by_someone_outside_the_roster_the_joiner_holds_is_refused_named() {
    let h = Harness::new(&["ada", "arc", "vis", "bo"]).await;
    let (ada, arc, vis, bo) = (0, 1, 2, 3);
    let (site, _room) = a_site_whose_owner_goes_offline(&h, ada, arc).await;
    h.add_to_forum(ada, bo, &site).await;
    name_myself(&h, bo, "Bo").await;
    h.settle().await;
    h.node(ada).group_remove_member(&site, &hex::encode(h.id(bo)), None).await.expect("bo removed");
    h.settle().await;
    assert_eq!(card_name(&h, arc, &site, bo).as_deref(), Some("Bo"), "the Arc holds a departed member's card");
    let bundle = h.node(vis).build_contact_bundle().unwrap();
    h.node(arc).group_add_member(&site, &bundle).await.expect("the admitter adds the visitor");
    h.settle_among(&[arc, vis]).await;

    // One bundle, the spine and the cards together.
    let (spine, cards) = spine_and_cards(rows_of(&h, arc, &site));
    let rows: Vec<_> = spine.into_iter().chain(cards).collect();
    let b = pacific_core::history::sign(&h.node(arc).id, &hex::decode(&site).unwrap(), &h.id(vis), rows);
    h.node(arc).send_history_raw(&h.node(vis).build_contact_bundle().unwrap(), &b).await.expect("sent");
    h.settle_among(&[arc, vis]).await;

    let reasons: Vec<String> = h.node(vis).quarantined().unwrap().into_iter().map(|(_, _, why, _)| why).collect();
    let short = hex::encode(&h.id(bo)[..6]);
    assert!(reasons.iter().any(|r| r.contains("history row") && r.contains(&format!("a card by {short}, not in the object's roster"))), "{reasons:?}");
    assert_eq!(card_name(&h, vis, &site, bo), None, "not taken");
    assert_eq!(view(&h, vis, &site).unwrap()["display_name"], "Egregore", "the spine beside it taken");
}
