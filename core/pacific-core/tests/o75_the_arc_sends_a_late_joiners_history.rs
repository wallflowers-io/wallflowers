//! O-75, THE ARC'S SEND: when the Arc admits someone by claim, it seals each object the
//! admission added them to (the Site, then each room) its history, after the Welcome, to the
//! joiner's intro mailbox: a `history::Bundle` it signs, of the rows it holds, unaltered.
//! DURABLE STATE FIRST: the owner's sequenced spine, whole and in chain order, and the
//! membership records beside it; then every other row, newest first, to one relay blob. Never
//! a Host's items. (The joiner's ingest, and AH-1 to AH-8, are BW-D's and BW-E's.)

mod common;

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use common::Harness;
use ed25519_dalek::Signer;
use pacific_core::coordinator::{decode_delta, ArgVal, Args, FORUM_POST};
use pacific_core::object::ObjectKind;
use std::collections::HashMap;

fn kiosk() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[9u8; 32])
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

fn kiosk_keys() -> HashMap<String, [u8; 32]> {
    let pk = kiosk().verifying_key().to_bytes();
    HashMap::from([(pacific_core::claim::kid_of(&pk), pk)])
}

fn args(pairs: &[(&str, &str)]) -> Args {
    pairs.iter().map(|(k, v)| (k.to_string(), ArgVal::Text(v.to_string()))).collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

fn post(text: &str) -> Args {
    let mut a = Args::new();
    a.insert("text".into(), ArgVal::Text(text.into()));
    a
}

/// ada's Site, its room and its Host; the Arc admits to the Site and the room.
async fn a_site(h: &Harness) -> (String, String, String) {
    let (ada, arc) = (0, 1);
    let site = h.mint_object(ada, ObjectKind::Group, "Egregore", &[arc]).await;
    let room = h.mint_object(ada, ObjectKind::Forum, "Talk", &[arc]).await;
    let host = h.node(ada).host_new(&site, "Egregore").await.unwrap();
    let founder = h.node(ada);
    founder
        .apply(&site, pacific_core::parts::OP_SET_PART, pacific_core::parts::set_part_args(&room, "room", now_ms()))
        .await
        .unwrap();
    founder
        .apply(&room, pacific_core::parent::OP_SET_PARENT, pacific_core::parent::set_parent_args(&site, "room", now_ms()))
        .await
        .unwrap();
    let admitter = [("member", hex::encode(h.id(arc))), ("role", "admitter".to_string())];
    let admitter: Vec<(&str, &str)> = admitter.iter().map(|(k, v)| (*k, v.as_str())).collect();
    for o in [&site, &room] {
        founder.apply(o, pacific_core::roles::OP_SET_ROLE, args(&admitter)).await.unwrap();
    }
    h.settle().await;
    (site, room, host)
}

/// The bundle's rows, in order: `spine` sequenced rows in chain order, then `records`
/// membership records, then the rest (posts, reactions, receipts) newest first; each an unaltered row, its author's
/// signature verifying over the object's group; and the bundle signed by the Arc, for the
/// joiner.
fn check_order(b: &pacific_core::history::Bundle, r: &pacific_core::node::HistorySent, arc: [u8; 32], joiner: [u8; 32]) {
    pacific_core::history::verify(b).expect("the Arc's signature over the bundle");
    assert_eq!((b.sender, b.joiner), (arc, joiner));
    assert_eq!(b.rows.len(), r.spine + r.records + r.posts, "{r:?}");
    let deltas: Vec<_> = b.rows.iter().map(|row| decode_delta(&row.envelope).unwrap()).collect();
    for (row, d) in b.rows.iter().zip(&deltas) {
        pacific_core::delta_sig::verify_delta(&b.object, &row.author, &d.id(), &row.sig).expect("the row's author signed it, unaltered");
    }
    let (spine, rest) = deltas.split_at(r.spine);
    assert!(spine.iter().all(|d| d.seq.is_some()), "the spine first");
    let pos: Vec<_> = spine.iter().map(|d| (d.epoch, d.seq.unwrap())).collect();
    assert!(pos.windows(2).all(|w| w[0] < w[1]), "in chain order: {pos:?}");
    let (records, posts) = rest.split_at(r.records);
    assert!(records.iter().all(|d| d.seq.is_none() && pacific_core::membership::is_membership_op(d.op_id)), "then the membership records");
    assert!(posts.iter().all(|d| d.seq.is_none() && !pacific_core::membership::is_membership_op(d.op_id)), "then the rest");
    let gens: Vec<u64> = posts.iter().map(|d| d.gen.unwrap_or(0)).collect();
    assert!(gens.windows(2).all(|w| w[0] >= w[1]), "newest first: {gens:?}");
}

#[tokio::test]
async fn an_admission_sends_the_site_and_its_room_their_history_durable_state_first() {
    let h = Harness::new(&["ada", "arc", "vis"]).await;
    let (ada, arc, vis) = (0, 1, 2);
    let (site, room, host) = a_site(&h).await;
    for t in ["one", "two", "three"] {
        h.node(ada).apply(&room, FORUM_POST, post(t)).await.unwrap();
    }
    h.settle().await;

    let bundles: Vec<String> = (0..2).map(|_| h.node(vis).build_contact_bundle().unwrap()).collect();
    let a = h.node(arc).admit_by_claim(&site, &claim(&site, 1), &bundles, &kiosk_keys()).await.expect("admitted");
    assert_eq!(a.rooms, vec![room.clone()]);
    let objects: Vec<&str> = a.history.iter().map(|x| x.object.as_str()).collect();
    assert_eq!(objects, vec![site.as_str(), room.as_str()], "the Site's, then the room's; never the Host's: {:?}", a.history);
    for x in &a.history {
        assert_eq!(x.unsent, None, "sent: {x:?}");
        assert!(x.spine > 0, "the spine went: {x:?}");
    }
    let room_sent = &a.history[1];
    assert!(room_sent.posts >= 3, "the room's three posts, and its receipts: {room_sent:?}");

    // What was sealed is what history_of builds: in order, unaltered, signed.
    let tag = pacific_core::handshake::parse_and_verify(&bundles[0]).unwrap().intro_tag;
    for object in [&site, &room] {
        let (b, r) = h.node(arc).history_of(object, &h.id(vis), &tag).unwrap();
        check_order(&b.expect("a bundle"), &r, h.id(arc), h.id(vis));
    }
    let (b, _) = h.node(arc).history_of(&room, &h.id(vis), &tag).unwrap();
    let texts: Vec<String> = b
        .unwrap()
        .rows
        .iter()
        .filter_map(|row| decode_delta(&row.envelope).ok())
        .filter(|d| d.op_id == FORUM_POST && d.type_id == ObjectKind::Forum.type_id() as u32)
        .filter_map(|d| match d.args.get("text") {
            Some(ArgVal::Text(t)) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["three", "two", "one"], "the room's posts, newest first");

    // A Host's items are no history, whatever asks.
    assert!(h.node(ada).history_of(&host, &h.id(vis), &tag).is_err(), "a Host carries no history");
}

/// At the cap: the newest rows go, the bundle stays one relay blob, and the rest is counted.
#[tokio::test]
async fn at_the_cap_the_newest_go_and_the_bundle_is_one_relay_blob() {
    let h = Harness::new(&["ada", "arc", "vis"]).await;
    let (ada, arc, vis) = (0, 1, 2);
    let (_, room, _) = a_site(&h).await;
    let n = 160;
    for i in 0..n {
        h.node(ada).apply(&room, FORUM_POST, post(&format!("{i:04} {}", "x".repeat(1500)))).await.unwrap();
    }
    h.settle().await;
    let bundle = h.node(vis).build_contact_bundle().unwrap();
    let tag = pacific_core::handshake::parse_and_verify(&bundle).unwrap().intro_tag;
    let (b, r) = h.node(arc).history_of(&room, &h.id(vis), &tag).unwrap();
    let b = b.expect("a bundle");
    check_order(&b, &r, h.id(arc), h.id(vis));
    assert!(r.left > 0 && r.posts < n, "some held back: {r:?}");
    let sealed = pacific_wire::blob_b64(&pacific_core::seal::seal(&pacific_core::history::encode(&b), &tag, &tag).unwrap());
    assert!(sealed.len() <= pacific_core::node::HISTORY_MAX_B64, "{} over the relay's blob", sealed.len());
    assert!(pacific_core::history::encode(&b).len() <= pacific_core::history::CAP, "inside history::CAP");
    let first = b.rows[r.spine + r.records..]
        .iter()
        .map(|row| decode_delta(&row.envelope).unwrap())
        .find(|d| d.op_id == FORUM_POST)
        .expect("a post went");
    assert!(matches!(first.args.get("text"), Some(ArgVal::Text(t)) if t.starts_with(&format!("{:04}", n - 1))), "the newest post first");
}

/// A spine that one blob cannot carry whole is not sent broken: nothing of that object goes,
/// and the report says why. The room's history still goes.
#[tokio::test]
async fn a_spine_over_the_blob_sends_nothing_and_says_so() {
    let h = Harness::new(&["ada", "arc", "vis"]).await;
    let (ada, arc, vis) = (0, 1, 2);
    let (site, room, _) = a_site(&h).await;
    for i in 0..16 {
        let mut face = Args::new();
        face.insert("face".into(), ArgVal::Text(format!(r#"{{"v":1,"n":{i},"pad":"{}"}}"#, "p".repeat(15_000))));
        h.node(ada).apply(&site, pacific_core::group::OP_SET_FACE, face).await.unwrap();
    }
    h.node(ada).apply(&room, FORUM_POST, post("still here")).await.unwrap();
    h.settle().await;
    let bundles: Vec<String> = (0..2).map(|_| h.node(vis).build_contact_bundle().unwrap()).collect();
    let a = h.node(arc).admit_by_claim(&site, &claim(&site, 2), &bundles, &kiosk_keys()).await.expect("admitted");
    let site_sent = a.history.iter().find(|x| x.object == site).expect("the Site's report");
    assert!(site_sent.unsent.as_deref().is_some_and(|w| w.contains("its spine alone")), "named: {site_sent:?}");
    assert_eq!((site_sent.spine, site_sent.posts), (0, 0), "and nothing of it went: {site_sent:?}");
    let room_sent = a.history.iter().find(|x| x.object == room).expect("the room's report");
    assert_eq!(room_sent.unsent, None, "the room's history goes: {room_sent:?}");
}

// ---- O-75 cards: each current member's latest card, after the spine ------------------

fn card(name: &str) -> Args {
    let mut a = Args::new();
    a.insert("card".into(), ArgVal::Text(format!(r#"{{"displayName":"{name}"}}"#)));
    a
}

/// The card a bundle row carries, by its display name.
fn name_of(row: &pacific_core::history::Row) -> String {
    let d = decode_delta(&row.envelope).unwrap();
    let Some(ArgVal::Text(c)) = d.args.get("card") else { panic!("a card row carries a card") };
    serde_json::from_str::<serde_json::Value>(c).unwrap()["displayName"].as_str().unwrap().to_string()
}

/// After the spine's bundle, each CURRENT member's latest card, unaltered and signed, newest
/// first: never a departed member's, never an earlier card of a member's, never the joiner's.
#[tokio::test]
async fn the_members_latest_cards_go_after_the_spine_and_never_a_departed_members() {
    let h = Harness::new(&["ada", "arc", "vis", "bo", "cy"]).await;
    let (ada, arc, vis, bo, cy) = (0, 1, 2, 3, 4);
    let (site, _, _) = a_site(&h).await;
    h.add_to_forum(ada, bo, &site).await;
    h.add_to_forum(ada, cy, &site).await;
    h.settle().await;
    for (u, name) in [(bo, "bo, first"), (cy, "cy"), (ada, "ada"), (bo, "bo, latest")] {
        h.node(u).apply(&site, pacific_core::profiles::OP_PUBLISH_PROFILE, card(name)).await.expect("a member's card");
        h.settle().await;
    }
    h.node(ada).group_remove_member(&site, &hex::encode(h.id(cy)), None).await.expect("cy is removed");
    h.settle().await;

    let bundles: Vec<String> = (0..2).map(|_| h.node(vis).build_contact_bundle().unwrap()).collect();
    let tag = pacific_core::handshake::parse_and_verify(&bundles[0]).unwrap().intro_tag;
    let (cards, sent, unsent) = h.node(arc).cards_of(&site, &h.id(vis), &tag, pacific_core::history::CAP, pacific_core::node::CARDS_MAX).unwrap();
    assert_eq!(unsent, 0);
    let rows: Vec<&pacific_core::history::Row> = cards.iter().flat_map(|b| &b.rows).collect();
    assert_eq!(rows.len(), sent);
    for b in &cards {
        pacific_core::history::verify(b).expect("each card bundle signed by the Arc");
        assert_eq!((b.sender, b.joiner), (h.id(arc), h.id(vis)));
    }
    // A member's latest card is the one the Site folds for them (the highest gen), whichever
    // wrote it: core publishes a card from the self record on entry, too.
    let view: serde_json::Value = serde_json::from_str(&h.node(arc).object_view(&site).unwrap()).unwrap();
    let names: Vec<String> = rows.iter().map(|r| name_of(r)).collect();
    for r in &rows {
        let folded = &view["profiles"][hex::encode(r.author)];
        assert_eq!(name_of(r), folded["name"].as_str().unwrap_or_default(), "the member's latest card: {folded}");
    }
    let authors: std::collections::HashSet<[u8; 32]> = rows.iter().map(|r| r.author).collect();
    assert_eq!(authors.len(), rows.len(), "one card per member: {names:?}");
    assert!(authors.contains(&h.id(bo)) && authors.contains(&h.id(ada)), "{names:?}");
    assert!(!names.contains(&"bo, first".to_string()), "never an earlier card: {names:?}");
    assert!(!authors.contains(&h.id(cy)), "never a departed member's: {names:?}");
    assert!(rows.iter().all(|r| r.author != h.id(vis)), "never the joiner's own");
    let gens: Vec<u64> = rows.iter().map(|r| decode_delta(&r.envelope).unwrap().gen.unwrap_or(0)).collect();
    assert!(gens.windows(2).all(|w| w[0] >= w[1]), "newest first: {gens:?}");
    for r in &rows {
        let d = decode_delta(&r.envelope).unwrap();
        pacific_core::delta_sig::verify_delta(&cards[0].object, &r.author, &d.id(), &r.sig).expect("unaltered, its author's signature");
    }
    // The spine's bundle carries no card: cards travel as each member's latest, here.
    let (spine, _) = h.node(arc).history_of(&site, &h.id(vis), &tag).unwrap();
    assert!(spine.unwrap().rows.iter().all(|r| !pacific_core::profiles::is_profile_op(decode_delta(&r.envelope).unwrap().op_id)));

    // Through the admission: the Site's report names the cards that went.
    let a = h.node(arc).admit_by_claim(&site, &claim(&site, 3), &bundles, &kiosk_keys()).await.expect("admitted");
    let site_sent = a.history.iter().find(|x| x.object == site).unwrap();
    assert_eq!((site_sent.cards, site_sent.cards_unsent), (sent, 0), "{site_sent:?}");
    assert!(site_sent.card_bundles >= 1, "{site_sent:?}");
}

/// Cards pack into as many bundles as a blob takes, each within it and self-contained, to the
/// admission's total; past it, the rest are counted, not sent.
#[tokio::test]
async fn cards_pack_into_bundles_within_a_blob_to_the_admissions_total() {
    let h = Harness::new(&["ada", "arc", "vis", "bo", "cy", "dee"]).await;
    let (ada, arc, vis) = (0, 1, 2);
    let (site, _, _) = a_site(&h).await;
    for u in [3, 4, 5] {
        h.add_to_forum(ada, u, &site).await;
    }
    h.settle().await;
    for u in [0, 3, 4, 5] {
        h.node(u).apply(&site, pacific_core::profiles::OP_PUBLISH_PROFILE, card(&format!("member {u} {}", "n".repeat(300)))).await.unwrap();
        h.settle().await;
    }
    let bundle = h.node(vis).build_contact_bundle().unwrap();
    let tag = pacific_core::handshake::parse_and_verify(&bundle).unwrap().intro_tag;
    let (all, sent, unsent) = h.node(arc).cards_of(&site, &h.id(vis), &tag, pacific_core::history::CAP, pacific_core::node::CARDS_MAX).unwrap();
    assert_eq!((all.len(), unsent), (1, 0), "a few small cards, one bundle");
    assert!(sent >= 2, "{sent}");
    // A blob one byte short of that bundle cannot hold them all.
    let blob = pacific_core::history::encode(&all[0]).len() - 1;
    let (split, sent2, unsent2) = h.node(arc).cards_of(&site, &h.id(vis), &tag, blob, pacific_core::node::CARDS_MAX).unwrap();
    assert!(split.len() > 1, "a smaller blob, several bundles: {}", split.len());
    assert_eq!((sent2, unsent2), (sent, 0));
    for b in &split {
        pacific_core::history::verify(b).expect("each self-contained and signed");
        assert!(pacific_core::history::encode(b).len() <= blob, "each within the blob");
    }
    let one = pacific_core::history::encode(&split[0]).len();
    let (capped, sent3, unsent3) = h.node(arc).cards_of(&site, &h.id(vis), &tag, blob, one).unwrap();
    assert_eq!(capped.len(), 1, "the admission's total holds one bundle");
    assert_eq!(sent3 + unsent3, sent, "and the rest are counted: {sent3} sent, {unsent3} not");
    assert!(unsent3 > 0);
}

/// The cards' total is the ICD's (`cardsTotal`, read at build), one number; a test's
/// PACIFIC_HISTORY_TOTAL_CAP may only lower it, and anything else is refused.
#[test]
fn the_cards_total_is_the_icds_and_a_tests_override_only_lowers_it() {
    use pacific_core::node::{cards_max_from, CARDS_MAX};
    assert_eq!(CARDS_MAX, pacific_core::history::CARDS_TOTAL, "one number, the ICD's");
    assert_eq!(cards_max_from(None).unwrap(), CARDS_MAX);
    assert_eq!(cards_max_from(Some("200000")).unwrap(), 200_000, "a test lowers it");
    for over in [format!("{}", CARDS_MAX + 1), "lots".to_string(), "-1".to_string()] {
        let e = cards_max_from(Some(&over)).unwrap_err().to_string();
        assert!(e.contains("may only lower"), "{over}: {e}");
    }
}
