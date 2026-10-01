//! O-75: A LATE JOINER'S HISTORY, THE BUNDLE ITSELF (core/docs/launch/mdr/arc-history.md; row 5's
//! mls.intro.kinds.history and mls.applicationPayload.history). What the joiner refuses, whole or
//! row by row, and what it keeps of where a row came from, with bundles made by hand on the wire
//! (`pacific_core::history`) and posted to the joiner's intro mailbox (`Node::send_history_raw`),
//! so that a sender, a row or a signature can be what an honest Arc never sends.
//!
//! The spine (the owner's durable state), and posts under rule (iii) (Ralph, 29 Sep: "Use the MLS
//! welcome history package, delivered by the arc"): the Arc vouches for the posts it holds, and
//! the joiner refuses forged authorship, the wrong room, tampering and replay. The honest path,
//! the owner offline, is o75_history_through_the_arc.rs.

mod common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::history::{self, Bundle, Row};
use pacific_core::object::ObjectKind;
use serde_json::Value;

fn text(pairs: &[(&str, &str)]) -> Args {
    pairs.iter().map(|(k, v)| (k.to_string(), ArgVal::Text(v.to_string()))).collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// The cap, as the ICD states it (row 5: mls.intro.kinds.history.cap); the wire's must agree.
fn cap() -> usize {
    let icd: Value = serde_json::from_str(include_str!("../../coordination/delta-graph.icd.json")).unwrap();
    let cap = icd.pointer("/mls/intro/kinds/history/cap").and_then(Value::as_u64).expect("the ICD states the history cap") as usize;
    assert_eq!(cap, history::CAP, "the wire's cap is the ICD's");
    cap
}

const ADA: usize = 0; // the owner
const ARC: usize = 1; // the admitter
const EVE: usize = 2; // a member of the Site, not its admitter
const VIS: usize = 3; // the visitor the Arc admits
const GIL: usize = 4; // a second visitor
const ZED: usize = 5; // never a member of anything here

/// The Site (a name, its Host and room, a Face), the Arc its admitter, eve a member; a second Site,
/// "Elsewhere", the Arc its admitter too, that the visitor is never admitted to. Then the owner
/// goes offline, and the Arc adds vis and gil to the Site.
struct World {
    h: Harness,
    site: String,
    elsewhere: String,
    room: String,
    elsewhere_room: String,
}

async fn world() -> World {
    let h = Harness::new(&["ada", "arc", "eve", "vis", "gil", "zed"]).await;
    let (mut sites, mut rooms) = (vec![], vec![]);
    for name in ["Egregore", "Elsewhere"] {
        let draft = pacific_core::mint::MintDraft { name: name.into(), shape: "community".into(), ..Default::default() };
        let site = h.node(ADA).mint(ObjectKind::Group, &draft).await.unwrap();
        let room = h.mint_object(ADA, ObjectKind::Forum, "Talk", &[]).await;
        let host = h.mint_object(ADA, ObjectKind::Host, name, &[]).await;
        let f = h.node(ADA);
        // A Site's Face needs its Host part (A-11).
        for (part, role) in [(&host, "host"), (&room, "room")] {
            f.apply(&site, pacific_core::parts::OP_SET_PART, pacific_core::parts::set_part_args(part, role, now_ms())).await.unwrap();
            f.apply(part, pacific_core::parent::OP_SET_PARENT, pacific_core::parent::set_parent_args(&site, role, now_ms())).await.unwrap();
        }
        f.apply(&site, pacific_core::group::OP_SET_FACE, text(&[("face", &format!(r#"{{"v":1,"name":"{name}"}}"#))])).await.unwrap();
        drop(f);
        h.add_to_forum(ADA, ARC, &site).await;
        h.add_to_forum(ADA, ARC, &room).await;
        let admitter = hex::encode(h.id(ARC));
        for o in [&site, &room] {
            h.node(ADA).apply(o, pacific_core::roles::OP_SET_ROLE, text(&[("member", &admitter), ("role", "admitter")])).await.unwrap();
        }
        // The spine's last row, after the admitter's grant: the one the forgeries stand in for.
        // (The joiner learns the sender's standing from the spine, so the grant must arrive.)
        h.node(ADA).apply(&site, pacific_core::group::OP_SET_FACE, text(&[("face", &format!(r#"{{"v":1,"name":"{name}","final":true}}"#))])).await.unwrap();
        sites.push(site);
        rooms.push(room);
    }
    h.add_to_forum(ADA, EVE, &sites[0]).await;
    h.settle().await;
    let w = World { h, site: sites.remove(0), elsewhere: sites.remove(0), room: rooms.remove(0), elsewhere_room: rooms.remove(0) };
    // Admitted by the Arc's own Add, not admit_by_claim, which now sends the honest bundle: the
    // Arc is recorded as the Add's committer (at the Welcome), and the bundles these tests make
    // are the only history the visitors receive.
    for who in [VIS, GIL] {
        let contact = w.h.node(who).build_contact_bundle().unwrap();
        w.h.node(ARC).group_add_member(&w.site, &contact).await.expect("the Arc adds the visitor");
        w.h.node(who).sync_once().await.expect("the visitor takes its Welcome");
    }
    offline(&w.h).await;
    w
}

/// Everyone syncs but the owner.
async fn offline(h: &Harness) {
    h.settle_among(&[ARC, EVE, VIS, GIL, ZED]).await;
}

/// The owner's sequenced spine of `object` as `holder` keeps it, in chain order.
fn spine(h: &Harness, holder: usize, object: &str) -> Vec<Row> {
    let gid = hex::decode(object).unwrap();
    let mut rows: Vec<(pacific_core::coordinator::Delta, Row)> = h
        .node(holder)
        .dir
        .load_log_signed(&gid)
        .unwrap()
        .into_iter()
        .filter_map(|(author, envelope, sig)| {
            let d = pacific_core::coordinator::decode_delta(&envelope).ok()?;
            d.seq?;
            Some((d, Row { envelope, author, sig: sig? }))
        })
        .collect();
    rows.sort_by_key(|(d, _)| (d.epoch, d.seq.unwrap_or(0)));
    rows.into_iter().map(|(_, r)| r).collect()
}

/// `object`'s posts as `holder` keeps them.
fn posts(h: &Harness, holder: usize, object: &str) -> Vec<Row> {
    let gid = hex::decode(object).unwrap();
    h.node(holder)
        .dir
        .load_log_signed(&gid)
        .unwrap()
        .into_iter()
        .filter_map(|(author, envelope, sig)| {
            let d = pacific_core::coordinator::decode_delta(&envelope).ok()?;
            (d.op_id == pacific_core::coordinator::FORUM_POST).then_some(Row { envelope, author, sig: sig? })
        })
        .collect()
}

/// `n` posts in each room, the owner's and the Arc's in turn, which the Arc holds; then the Arc
/// adds vis to the Site's room, whose spine and posts vis has from a bundle alone.
async fn with_posts(w: &World, n: usize) {
    for room in [&w.room, &w.elsewhere_room] {
        for i in 0..n {
            let who = if i % 2 == 0 { ADA } else { ARC };
            // Each room's own text: a Delta's id is its envelope's hash, which names no group, so
            // the same post in both rooms would be one id, and a row of the other room would be
            // held here as this room's own.
            let post = format!("post {i} of {}", &room[..8]);
            w.h.node(who).apply(room, pacific_core::coordinator::FORUM_POST, text(&[("text", &post)])).await.unwrap();
        }
    }
    w.h.settle().await;
    let contact = w.h.node(VIS).build_contact_bundle().unwrap();
    w.h.node(ARC).group_add_member(&w.room, &contact).await.expect("the Arc adds the visitor to the room");
    w.h.node(VIS).sync_once().await.expect("the visitor takes its Welcome");
    offline(&w.h).await;
    assert!(posts(&w.h, VIS, &w.room).is_empty(), "the visitor holds no post but from a bundle");
}

fn id_of(r: &Row) -> [u8; 32] {
    pacific_core::coordinator::decode_delta(&r.envelope).map(|d| d.id()).unwrap_or([0u8; 32])
}

/// Whether `who` stores a row with this id in `object`'s log.
fn holds(h: &Harness, who: usize, object: &str, id: &[u8; 32]) -> bool {
    let gid = hex::decode(object).unwrap();
    h.node(who)
        .dir
        .load_log_signed(&gid)
        .unwrap()
        .iter()
        .any(|(_, e, _)| pacific_core::coordinator::decode_delta(e).map(|d| &d.id() == id).unwrap_or(false))
}

/// What `who` has quarantined since `from` entries: (the group named, the reason).
fn refused_since(h: &Harness, who: usize, from: usize) -> Vec<(Option<Vec<u8>>, String)> {
    let all = h.node(who).quarantined().unwrap();
    let mut new: Vec<_> = all.into_iter().map(|(g, _, why, _)| (g, why)).collect();
    new.truncate(new.len().saturating_sub(from));
    new
}

async fn send(w: &World, from: usize, to: usize, b: &Bundle) {
    let contact = w.h.node(to).build_contact_bundle().unwrap();
    w.h.node(from).send_history_raw(&contact, b).await.expect("posted");
    offline(&w.h).await;
}

fn sign(w: &World, by: usize, object: &str, joiner: usize, rows: Vec<Row>) -> Bundle {
    history::sign(&w.h.node(by).id, &hex::decode(object).unwrap(), &w.h.id(joiner), rows)
}

// ─── AH-2, AH-3: a row that does not prove itself ─────────────────────────────

/// AH-2: in a bundle from the admitter, each forgery of a spine row is refused, named with the
/// object, and not stored, while the rest of the bundle is: a signature that does not verify; an
/// envelope altered, whose id is no longer the Delta its signature names; a row of another group.
/// The row the forgeries stand in for then arrives whole, and is stored: nothing was poisoned.
#[tokio::test]
async fn ah2_a_forged_spine_row_is_refused_named_and_not_stored() {
    let w = world().await;
    let rows = spine(&w.h, ARC, &w.site);
    let (head, last) = (rows[..rows.len() - 1].to_vec(), rows.last().unwrap().clone());
    let gid = hex::decode(&w.site).unwrap();

    let mut bad_sig = last.clone();
    bad_sig.sig[0] ^= 1;
    let mut altered = last.clone();
    let n = altered.envelope.len();
    altered.envelope[n - 1] ^= 1;
    let other = spine(&w.h, ARC, &w.elsewhere).last().unwrap().clone();

    for (forged, what) in [(bad_sig, "a signature that does not verify"), (altered, "an altered envelope"), (other, "another group's row")] {
        let before = w.h.node(VIS).quarantined().unwrap().len();
        let mut bundle_rows = head.clone();
        bundle_rows.push(forged.clone());
        send(&w, ARC, VIS, &sign(&w, ARC, &w.site, VIS, bundle_rows)).await;
        let named = refused_since(&w.h, VIS, before);
        assert!(
            named.iter().any(|(g, why)| g.as_deref() == Some(&gid[..]) && why.contains("history row")),
            "{what}: refused and named with the Site: {named:?}"
        );
        assert!(!holds(&w.h, VIS, &w.site, &id_of(&forged)) || id_of(&forged) == id_of(&last), "{what}: not stored");
        assert!(head.iter().all(|r| holds(&w.h, VIS, &w.site, &id_of(r))), "{what}: the rest of the bundle is stored; named: {named:?}");
        assert!(!holds(&w.h, VIS, &w.site, &id_of(&last)), "{what}: the true row is not stored in the forgery's place");
    }
    send(&w, ARC, VIS, &sign(&w, ARC, &w.site, VIS, rows.clone())).await;
    assert!(holds(&w.h, VIS, &w.site, &id_of(&last)), "the true row, sent whole, is stored");
    w.h.node(VIS).object_compliance(&w.site).expect("and the Site folds whole");
}

/// AH-3: a spine row whose author is not the object's owner is refused and named: a member's
/// (eve's) and a stranger's (zed's), each signing the owner's own envelope as theirs.
#[tokio::test]
async fn ah3_a_spine_row_by_anyone_but_the_owner_is_refused() {
    let w = world().await;
    let rows = spine(&w.h, ARC, &w.site);
    let (head, last) = (rows[..rows.len() - 1].to_vec(), rows.last().unwrap().clone());
    let gid = hex::decode(&w.site).unwrap();
    let did = id_of(&last);
    for (who, what) in [(EVE, "a member, not the owner"), (ZED, "never a member")] {
        let n = w.h.node(who);
        let theirs = Row { envelope: last.envelope.clone(), author: n.id.identity_pk(), sig: pacific_core::delta_sig::sign_delta(&n.id, &gid, &did) };
        drop(n);
        let before = w.h.node(VIS).quarantined().unwrap().len();
        let mut bundle_rows = head.clone();
        bundle_rows.push(theirs);
        send(&w, ARC, VIS, &sign(&w, ARC, &w.site, VIS, bundle_rows)).await;
        let named = refused_since(&w.h, VIS, before);
        assert!(named.iter().any(|(g, why)| g.as_deref() == Some(&gid[..]) && why.contains("history row") && why.contains("owner")), "{what}: {named:?}");
        assert!(!holds(&w.h, VIS, &w.site, &did), "{what}: not stored");
    }
}

// ─── AH-4, and the bundle's own proof ─────────────────────────────────────────

/// AH-4: a member who holds the visitor's contact bundle, but did not let them in and is no
/// admitter, sends the Site's spine: refused whole, named, nothing of it stored.
#[tokio::test]
async fn ah4_a_bundle_from_a_non_admitter_holding_the_contact_bundle_is_refused_whole() {
    let w = world().await;
    let rows = spine(&w.h, ARC, &w.site);
    let last = id_of(rows.last().unwrap());
    let before = w.h.node(VIS).quarantined().unwrap().len();
    send(&w, EVE, VIS, &sign(&w, EVE, &w.site, VIS, rows)).await;
    let named = refused_since(&w.h, VIS, before);
    assert!(named.iter().any(|(_, why)| why.contains("history bundle:")), "refused whole, named: {named:?}");
    assert!(!holds(&w.h, VIS, &w.site, &last), "nothing of it stored");
}

/// AH-4: the admitter sends the history of an object the visitor was never admitted to: it is
/// held (a bundle may come before its Welcome), nothing of it is stored, and once the hold has
/// passed with no Welcome, it is refused whole and named. The hold is shortened for the test
/// (PACIFIC_HISTORY_HOLD_SECS, read at each review of the held bundles), here alone.
#[tokio::test]
async fn ah4_a_bundle_for_an_object_the_joiner_was_not_admitted_to_is_held_then_refused() {
    let w = world().await;
    let rows = spine(&w.h, ARC, &w.elsewhere);
    send(&w, ARC, VIS, &sign(&w, ARC, &w.elsewhere, VIS, rows.clone())).await;
    let held = w.h.node(VIS).held_history().unwrap();
    assert!(held.iter().any(|(o, from, n, _)| o == &w.elsewhere && from == &w.h.id(ARC) && *n == rows.len()), "held, awaiting a Welcome: {held:?}");
    assert!(rows.iter().all(|r| !holds(&w.h, VIS, &w.elsewhere, &id_of(r))), "nothing of it stored");
    assert!(w.h.node(VIS).object_kind(&w.elsewhere).is_err(), "and the visitor holds no such object");

    let before = w.h.node(VIS).quarantined().unwrap().len();
    std::env::set_var("PACIFIC_HISTORY_HOLD_SECS", "0");
    w.h.node(VIS).sync_once().await.expect("the held bundles reviewed");
    std::env::remove_var("PACIFIC_HISTORY_HOLD_SECS");
    let named = refused_since(&w.h, VIS, before);
    assert!(named.iter().any(|(_, why)| why.contains("history bundle:") && why.contains("never admitted")), "{named:?}");
    assert!(!w.h.node(VIS).held_history().unwrap().iter().any(|(o, ..)| o == &w.elsewhere), "no longer held");
    assert!(rows.iter().all(|r| !holds(&w.h, VIS, &w.elsewhere, &id_of(r))), "and still nothing stored");
}

/// A bundle the admitter made for one visitor, posted to another's mailbox: refused whole.
#[tokio::test]
async fn a_bundle_replayed_to_another_member_is_refused_whole() {
    let w = world().await;
    let rows = spine(&w.h, ARC, &w.site);
    let last = id_of(rows.last().unwrap());
    let before = w.h.node(GIL).quarantined().unwrap().len();
    send(&w, ARC, GIL, &sign(&w, ARC, &w.site, VIS, rows)).await;
    let named = refused_since(&w.h, GIL, before);
    assert!(named.iter().any(|(_, why)| why.contains("history bundle:") && why.contains("not this device")), "{named:?}");
    assert!(!holds(&w.h, GIL, &w.site, &last), "nothing of it stored");
}

/// A row altered after the admitter signed the bundle: the bundle's signature no longer holds,
/// and it is refused whole.
#[tokio::test]
async fn a_bundle_with_a_row_altered_after_signing_is_refused_whole() {
    let w = world().await;
    let rows = spine(&w.h, ARC, &w.site);
    let last = id_of(rows.last().unwrap());
    let mut b = sign(&w, ARC, &w.site, VIS, rows);
    let k = b.rows.len() - 1;
    let n = b.rows[k].envelope.len();
    b.rows[k].envelope[n - 1] ^= 1;
    let before = w.h.node(VIS).quarantined().unwrap().len();
    send(&w, ARC, VIS, &b).await;
    let named = refused_since(&w.h, VIS, before);
    assert!(named.iter().any(|(_, why)| why.contains("history bundle:") && why.contains("signature")), "{named:?}");
    assert!(!holds(&w.h, VIS, &w.site, &last), "nothing of it stored");
}

/// A bundle over the cap the ICD states, however true its rows: its sender cannot publish it (one
/// relay blob, sealed, carries no more), and a reader refuses it whole, naming the cap.
#[tokio::test]
async fn a_bundle_over_the_cap_is_neither_sent_nor_read() {
    let w = world().await;
    let cap = cap();
    let rows = spine(&w.h, ARC, &w.site);
    let mut many = vec![];
    while history::encode(&sign(&w, ARC, &w.site, VIS, many.clone())).len() <= cap {
        many.extend(rows.iter().cloned());
    }
    let b = sign(&w, ARC, &w.site, VIS, many);
    let bytes = history::encode(&b);
    assert!(bytes.len() > cap, "the bundle is over the cap");
    let contact = w.h.node(VIS).build_contact_bundle().unwrap();
    assert!(w.h.node(ARC).send_history_raw(&contact, &b).await.is_err(), "the admitter cannot publish it");
    let refused = history::decode(&bytes).expect_err("a reader refuses it");
    assert!(refused.to_string().contains("history bundle:") && refused.to_string().contains(&cap.to_string()), "{refused}");
}

// ─── posts, rule (iii) ────────────────────────────────────────────────────────

/// A post whose signature does not verify, in the admitter's bundle for the room: refused, named
/// with the room, and not stored; the room's spine and its other posts are taken, via the Arc. The
/// true post then arrives whole, and is stored: nothing was poisoned.
#[tokio::test]
async fn a_forged_post_is_refused_named_and_the_rest_taken() {
    let w = world().await;
    with_posts(&w, 6).await;
    let (head, all) = (spine(&w.h, ARC, &w.room), posts(&w.h, ARC, &w.room));
    assert_eq!(all.len(), 6, "the Arc holds the six");
    let gid = hex::decode(&w.room).unwrap();
    let mut forged = all[0].clone();
    forged.sig[0] ^= 1;
    let before = w.h.node(VIS).quarantined().unwrap().len();
    let rows = head.iter().cloned().chain([forged.clone()]).chain(all[1..].iter().cloned()).collect();
    send(&w, ARC, VIS, &sign(&w, ARC, &w.room, VIS, rows)).await;
    let named = refused_since(&w.h, VIS, before);
    assert!(
        named.iter().any(|(g, why)| g.as_deref() == Some(&gid[..]) && why.contains("history row")),
        "refused and named with the room: {named:?}"
    );
    assert!(!holds(&w.h, VIS, &w.room, &id_of(&forged)), "not stored");
    for r in &all[1..] {
        assert!(holds(&w.h, VIS, &w.room, &id_of(r)), "the other posts are taken; named: {named:?}");
        assert_eq!(w.h.node(VIS).history_provenance(&w.room, &id_of(r)).unwrap(), Some(w.h.id(ARC)), "via the Arc");
    }
    let ours = w.h.node(ARC).object_transcript(&w.room).unwrap();
    let theirs = w.h.node(VIS).object_transcript(&w.room).unwrap();
    assert_eq!(theirs.len(), ours.len() - 1, "the visitor folds all but the forged one: {theirs:?}");
    assert!(theirs.iter().all(|l| ours.contains(l)), "as the Arc folds them");

    send(&w, ARC, VIS, &sign(&w, ARC, &w.room, VIS, head.into_iter().chain(all).collect())).await;
    assert!(holds(&w.h, VIS, &w.room, &id_of(&forged)), "the true post, sent whole, is stored");
    assert_eq!(w.h.node(VIS).object_transcript(&w.room).unwrap(), ours, "and the room folds whole");
}

/// A post of another room, signed there, in the admitter's bundle for this one: refused, named
/// with this room, and not stored; the rest is taken.
#[tokio::test]
async fn a_post_of_another_room_is_refused() {
    let w = world().await;
    with_posts(&w, 4).await;
    let (head, all) = (spine(&w.h, ARC, &w.room), posts(&w.h, ARC, &w.room));
    let other = posts(&w.h, ARC, &w.elsewhere_room).remove(0);
    let gid = hex::decode(&w.room).unwrap();
    let before = w.h.node(VIS).quarantined().unwrap().len();
    let rows = head.iter().cloned().chain(all.iter().cloned()).chain([other.clone()]).collect();
    send(&w, ARC, VIS, &sign(&w, ARC, &w.room, VIS, rows)).await;
    let named = refused_since(&w.h, VIS, before);
    assert!(
        named.iter().any(|(g, why)| g.as_deref() == Some(&gid[..]) && why.contains("history row")),
        "refused and named with the room: {named:?}"
    );
    assert!(!holds(&w.h, VIS, &w.room, &id_of(&other)), "not stored");
    assert!(all.iter().all(|r| holds(&w.h, VIS, &w.room, &id_of(r))), "the room's own posts are taken; named: {named:?}");
    assert_eq!(w.h.node(VIS).object_transcript(&w.room).unwrap(), w.h.node(ARC).object_transcript(&w.room).unwrap(), "and fold as the Arc's");
}

/// AH-4's path: eve, a member of the Site and no admitter, sends the room's spine and posts:
/// refused whole, named, none of it stored. The same rows from the Arc are taken.
#[tokio::test]
async fn posts_in_a_non_admitters_bundle_are_refused_whole() {
    let w = world().await;
    with_posts(&w, 4).await;
    let (head, all) = (spine(&w.h, ARC, &w.room), posts(&w.h, ARC, &w.room));
    let rows: Vec<Row> = head.into_iter().chain(all.iter().cloned()).collect();
    let before = w.h.node(VIS).quarantined().unwrap().len();
    send(&w, EVE, VIS, &sign(&w, EVE, &w.room, VIS, rows.clone())).await;
    let named = refused_since(&w.h, VIS, before);
    assert!(named.iter().any(|(_, why)| why.contains("history bundle:")), "refused whole, named: {named:?}");
    assert!(all.iter().all(|r| !holds(&w.h, VIS, &w.room, &id_of(r))), "no post of it stored");
    send(&w, ARC, VIS, &sign(&w, ARC, &w.room, VIS, rows)).await;
    assert!(all.iter().all(|r| holds(&w.h, VIS, &w.room, &id_of(r))), "the same posts from the Arc are taken");
}

// ─── provenance ───────────────────────────────────────────────────────────────

/// A row stored from the admitter's bundle keeps the admitter it came via; one the visitor
/// received directly, the owner's Face written after they joined, is its own.
#[tokio::test]
async fn a_row_from_a_bundle_is_kept_as_via_its_admitter_and_one_received_directly_is_not() {
    let w = world().await;
    let rows = spine(&w.h, ARC, &w.site);
    send(&w, ARC, VIS, &sign(&w, ARC, &w.site, VIS, rows.clone())).await;
    for r in &rows {
        assert_eq!(w.h.node(VIS).history_provenance(&w.site, &id_of(r)).unwrap(), Some(w.h.id(ARC)), "via the Arc");
    }
    // The owner comes back and writes; the visitor receives it as any member does.
    w.h.node(ADA).apply(&w.site, pacific_core::group::OP_SET_FACE, text(&[("face", r#"{"v":1,"name":"after"}"#)])).await.unwrap();
    w.h.settle().await;
    let direct = spine(&w.h, VIS, &w.site).into_iter().find(|r| !rows.iter().any(|s| id_of(s) == id_of(r))).expect("a row the visitor received directly");
    assert_eq!(w.h.node(VIS).history_provenance(&w.site, &id_of(&direct)).unwrap(), None, "received directly: its own");
}
