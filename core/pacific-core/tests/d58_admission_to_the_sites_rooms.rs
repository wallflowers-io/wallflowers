//! D-58 (Ralph, 27 Sep: "(a) Admitter on forums"): a Site's rooms carry the admitter
//! role, and one claim, spent once on the Site, admits its visitor to the Site and to
//! every room the Arc's node admits to. A room that cannot be joined is named; the same
//! member presenting the claim again completes it, with no second spend (Software
//! Security, D-58's points 1–3).

mod common;

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use common::Harness;
use ed25519_dalek::Signer;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::object::ObjectKind;
use std::collections::HashMap;

fn kiosk() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[9u8; 32])
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

fn args(pairs: &[(&str, &str)]) -> Args {
    pairs.iter().map(|(k, v)| (k.to_string(), ArgVal::Text(v.to_string()))).collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

#[tokio::test]
async fn one_claim_admits_to_the_site_and_the_rooms_its_admitter_holds() {
    let h = Harness::new(&["ada", "arc", "vis", "eve"]).await;
    let (ada, arc, vis, eve) = (0, 1, 2, 3);
    let site = h.mint_object(ada, ObjectKind::Group, "Egregore", &[arc]).await;
    let room = h.mint_object(ada, ObjectKind::Forum, "Talk", &[arc]).await;
    let founders = h.mint_object(ada, ObjectKind::Forum, "Founders", &[arc]).await;

    // Both rooms are the Site's, both halves of the edge; the node admits to the Site and
    // to `room` alone. A room is granted one by one, never by default (point 3).
    let founder = h.node(ada);
    for r in [&room, &founders] {
        founder
            .apply(&site, pacific_core::parts::OP_SET_PART, pacific_core::parts::set_part_args(r, "room", now_ms()))
            .await
            .expect("the room is the Site's part");
        founder
            .apply(r, pacific_core::parent::OP_SET_PARENT, pacific_core::parent::set_parent_args(&site, "room", now_ms()))
            .await
            .expect("the Site is the room's parent");
    }
    let admitter = [("member", hex::encode(h.id(arc))), ("role", "admitter".to_string())];
    let admitter: Vec<(&str, &str)> = admitter.iter().map(|(k, v)| (*k, v.as_str())).collect();
    for o in [&site, &room] {
        founder.apply(o, pacific_core::roles::OP_SET_ROLE, args(&admitter)).await.expect("the founder grants the admitter role");
    }
    h.settle().await;

    // One key package: the Site takes it, and the room is named, not skipped.
    let one = vec![h.node(vis).build_contact_bundle().unwrap()];
    let first = h.node(arc).admit_by_claim(&site, &claim(&site, 1), &one, &kiosk_keys()).await.expect("admitted to the Site");
    assert_eq!(first.member, h.id(vis));
    assert!(first.rooms.is_empty());
    assert_eq!(first.unjoined.iter().map(|(r, _)| r.as_str()).collect::<Vec<_>>(), vec![room.as_str()], "{:?}", first.unjoined);
    h.settle().await;

    // The same person, the same claim, fresh key packages: the room is completed, and
    // nothing is spent twice (point 2).
    let two: Vec<String> = (0..2).map(|_| h.node(vis).build_contact_bundle().unwrap()).collect();
    let again = h.node(arc).admit_by_claim(&site, &claim(&site, 1), &two, &kiosk_keys()).await.expect("a retry completes the rooms");
    assert_eq!(again.rooms, vec![room.clone()]);
    assert!(again.unjoined.is_empty(), "{:?}", again.unjoined);
    h.settle().await;
    assert_eq!(h.node(ada).group_state(&site).unwrap().claims_spent.len(), 1, "spent once");
    assert_eq!(h.node(vis).object_kind(&room).expect("the visitor holds the room"), "forum");
    assert!(h.node(vis).object_kind(&founders).is_err(), "a room without the grant admits no one");

    // The visitor posts in the room, and it folds for the founder.
    let mut post = Args::new();
    post.insert("text".into(), ArgVal::Text("hello from the kiosk".into()));
    h.node(vis).apply(&room, pacific_core::coordinator::FORUM_POST, post).await.expect("a member posts");
    h.settle().await;
    let seen = h.node(ada).object_transcript(&room).unwrap();
    assert!(seen.iter().any(|l| l.ends_with("hello from the kiosk")), "{seen:?}");

    // The claim is this person's: another identity bringing it is refused, and so are
    // key packages naming two people (point 1).
    let other = vec![h.node(eve).build_contact_bundle().unwrap()];
    let stolen = h.node(arc).admit_by_claim(&site, &claim(&site, 1), &other, &kiosk_keys()).await;
    assert!(stolen.is_err_and(|e| e.to_string().contains("used")));
    let mixed = vec![h.node(vis).build_contact_bundle().unwrap(), h.node(eve).build_contact_bundle().unwrap()];
    let two_people = h.node(arc).admit_by_claim(&site, &claim(&site, 2), &mixed, &kiosk_keys()).await;
    assert!(two_people.is_err_and(|e| e.to_string().contains("more than one identity")));
    assert!(h.node(eve).object_kind(&site).is_err(), "no one else was admitted");
}

/// The kiosk's key, as the stand-in file gives it.
fn kiosk_keys() -> HashMap<String, [u8; 32]> {
    let pk = kiosk().verifying_key().to_bytes();
    HashMap::from([(pacific_core::claim::kid_of(&pk), pk)])
}

/// A claim as the kiosk issues one, its payload ending with `tail`: its `c`, or nothing.
fn token(site: &str, n: u8, tail: &str) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let pk = kiosk().verifying_key().to_bytes();
    let payload = format!(
        r#"{{"k":"{}","s":"{site}","n":"{}","iat":{now},"exp":{}{tail}}}"#,
        pacific_core::claim::kid_of(&pk),
        B64.encode([n; 16]),
        now + 600
    );
    let p = B64.encode(payload);
    format!("v1.{p}.{}", B64.encode(kiosk().sign(format!("v1.{p}").as_bytes()).to_bytes()))
}

/// ICD 2.1.0 row 3 (P1: "The room they are placed into should correlate to the selection"):
/// a room names the claim choice it takes. A claim goes to the rooms carrying its `c`; when
/// none does, to the rooms without a choice, and the answer names that fallback; a claim
/// with no `c` goes to the rooms without a choice. A part in any other role is joined on
/// every claim.
#[tokio::test]
async fn a_claims_choice_picks_its_rooms() {
    let h = Harness::new(&["ada", "arc", "sk", "fin", "bare"]).await;
    let (ada, arc, sk, fin, bare) = (0, 1, 2, 3, 4);
    let site = h.mint_object(ada, ObjectKind::Group, "Egregore", &[arc]).await;
    let resources = h.mint_object(ada, ObjectKind::Forum, "Resources", &[arc]).await;
    let skills = h.mint_object(ada, ObjectKind::Forum, "Skills, Time & Services", &[arc]).await;
    let open = h.mint_object(ada, ObjectKind::Forum, "Talk", &[arc]).await;
    let comments = h.mint_object(ada, ObjectKind::Forum, "Comments", &[arc]).await;

    let founder = h.node(ada);
    for (part, role, choice) in [(&resources, "room", Some("resources")), (&skills, "room", Some("skills")), (&open, "room", None), (&comments, "comments", None)] {
        let mut a = pacific_core::parts::set_part_args(part, role, now_ms());
        if let Some(c) = choice {
            a.insert("choice".into(), ArgVal::Text(c.into()));
        }
        founder.apply(&site, pacific_core::parts::OP_SET_PART, a).await.expect("the part is the Site's");
        founder
            .apply(part, pacific_core::parent::OP_SET_PARENT, pacific_core::parent::set_parent_args(&site, role, now_ms()))
            .await
            .expect("the Site is the part's parent");
    }
    let admitter = [("member", hex::encode(h.id(arc))), ("role", "admitter".to_string())];
    let admitter: Vec<(&str, &str)> = admitter.iter().map(|(k, v)| (*k, v.as_str())).collect();
    for o in [&site, &resources, &skills, &open, &comments] {
        founder.apply(o, pacific_core::roles::OP_SET_ROLE, args(&admitter)).await.expect("the founder grants the admitter role");
    }
    h.settle().await;

    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };
    for (who, n, tail, rooms, fallback) in [
        (sk, 1, r#","c":"skills""#, vec![skills.clone(), comments.clone()], None),
        (fin, 2, r#","c":"financial""#, vec![open.clone(), comments.clone()], Some("financial")),
        (bare, 3, "", vec![open.clone(), comments.clone()], None),
    ] {
        let bundles: Vec<String> = (0..5).map(|_| h.node(who).build_contact_bundle().unwrap()).collect();
        let a = h.node(arc).admit_by_claim(&site, &token(&site, n, tail), &bundles, &kiosk_keys()).await.expect("admitted");
        assert_eq!(sorted(a.rooms), sorted(rooms), "claim {tail:?}");
        assert!(a.unjoined.is_empty(), "{:?}", a.unjoined);
        assert_eq!(a.fallback.as_deref(), fallback, "claim {tail:?}");
    }
    h.settle().await;
    assert!(h.node(sk).object_kind(&resources).is_err() && h.node(sk).object_kind(&open).is_err(), "a chosen room only");
    assert_eq!(h.node(sk).object_kind(&skills).expect("the chosen room"), "forum");
}

/// A claim spent straight after the rooms are marked (TEST run 77, J-A): the Arc's node must
/// read the Site as it is at the relay, not as its last poll left it. Before, a resources
/// claim seconds after the marks found no room marked and fell back to every open room.
#[tokio::test]
async fn a_claim_just_after_the_rooms_are_marked_goes_to_the_marked_room() {
    let h = Harness::new(&["ada", "arc", "vis"]).await;
    let (ada, arc, vis) = (0, 1, 2);
    let site = h.mint_object(ada, ObjectKind::Group, "Egregore", &[arc]).await;
    let resources = h.mint_object(ada, ObjectKind::Forum, "Resources", &[arc]).await;
    let skills = h.mint_object(ada, ObjectKind::Forum, "Skills, Time & Services", &[arc]).await;
    let open = h.mint_object(ada, ObjectKind::Forum, "Talk", &[arc]).await;

    // The rooms, unmarked, and the Arc their admitter: all of it settled, as a Site stands
    // before the snippet marks it.
    let founder = h.node(ada);
    for room in [&resources, &skills, &open] {
        founder
            .apply(&site, pacific_core::parts::OP_SET_PART, pacific_core::parts::set_part_args(room, "room", now_ms()))
            .await
            .expect("the room is the Site's part");
        founder
            .apply(room, pacific_core::parent::OP_SET_PARENT, pacific_core::parent::set_parent_args(&site, "room", now_ms()))
            .await
            .expect("the Site is the room's parent");
    }
    let admitter = [("member", hex::encode(h.id(arc))), ("role", "admitter".to_string())];
    let admitter: Vec<(&str, &str)> = admitter.iter().map(|(k, v)| (*k, v.as_str())).collect();
    for o in [&site, &resources, &skills, &open] {
        founder.apply(o, pacific_core::roles::OP_SET_ROLE, args(&admitter)).await.expect("the founder grants the admitter role");
    }
    h.settle().await;

    // The marks, and no sync of the Arc's before the claim.
    for (room, c) in [(&resources, "resources"), (&skills, "skills")] {
        let mut a = pacific_core::parts::set_part_args(room, "room", now_ms());
        a.insert("choice".into(), ArgVal::Text(c.into()));
        h.node(ada).apply(&site, pacific_core::parts::OP_SET_PART, a).await.expect("the room is marked");
    }
    let bundles: Vec<String> = (0..4).map(|_| h.node(vis).build_contact_bundle().unwrap()).collect();
    let a = h.node(arc).admit_by_claim(&site, &token(&site, 1, r#","c":"resources""#), &bundles, &kiosk_keys()).await.expect("admitted");
    assert_eq!(a.rooms, vec![resources.clone()], "the marked room only");
    assert_eq!(a.fallback, None);
    assert!(a.unjoined.is_empty(), "{:?}", a.unjoined);
}

/// A MEMBER'S SECOND CLAIM (TEST, 1 Oct, door-test: "a joined member's second claim answers 503
/// forever"): a person already on the Site presenting a different claim was added to the Site
/// again, MLS refused the duplicate identity, and the Arc answered 503 on every try. Now the Site
/// is not added twice; the claim is spent once, by them; and the rooms of ITS choice they are not
/// in are joined. The second claim again is a retry: nothing new is spent.
#[tokio::test]
async fn a_members_second_claim_joins_its_rooms_and_never_adds_them_twice() {
    let h = Harness::new(&["ada", "arc", "vis"]).await;
    let (ada, arc, vis) = (0, 1, 2);
    let site = h.mint_object(ada, ObjectKind::Group, "Egregore", &[arc]).await;
    let resources = h.mint_object(ada, ObjectKind::Forum, "Resources", &[arc]).await;
    let skills = h.mint_object(ada, ObjectKind::Forum, "Skills, Time & Services", &[arc]).await;
    let founder = h.node(ada);
    for (room, c) in [(&resources, "resources"), (&skills, "skills")] {
        let mut a = pacific_core::parts::set_part_args(room, "room", now_ms());
        a.insert("choice".into(), ArgVal::Text(c.into()));
        founder.apply(&site, pacific_core::parts::OP_SET_PART, a).await.expect("the room is the Site's");
        founder
            .apply(room, pacific_core::parent::OP_SET_PARENT, pacific_core::parent::set_parent_args(&site, "room", now_ms()))
            .await
            .expect("the Site is the room's parent");
    }
    let admitter = [("member", hex::encode(h.id(arc))), ("role", "admitter".to_string())];
    let admitter: Vec<(&str, &str)> = admitter.iter().map(|(k, v)| (*k, v.as_str())).collect();
    for o in [&site, &resources, &skills] {
        founder.apply(o, pacific_core::roles::OP_SET_ROLE, args(&admitter)).await.expect("the founder grants the admitter role");
    }
    h.settle().await;

    // Each person's key packages made before the Arc's node is opened: a node is its own store.
    let bundles = || (0..3).map(|_| h.node(vis).build_contact_bundle().unwrap()).collect::<Vec<_>>();
    let b = bundles();
    let first = h.node(arc).admit_by_claim(&site, &token(&site, 1, r#","c":"resources""#), &b, &kiosk_keys()).await.expect("the first claim admits");
    assert_eq!(first.rooms, vec![resources.clone()]);
    h.settle().await;

    let b = bundles();
    let second = h.node(arc).admit_by_claim(&site, &token(&site, 2, r#","c":"skills""#), &b, &kiosk_keys()).await;
    let second = second.expect("a member's second claim is admitted, not refused");
    assert_eq!(second.member, h.id(vis));
    assert_eq!(second.rooms, vec![skills.clone()], "the rooms of the second claim's choice");
    assert!(second.unjoined.is_empty(), "{:?}", second.unjoined);
    h.settle().await;
    assert_eq!(h.node(vis).object_kind(&skills).expect("the visitor holds the second claim's room"), "forum");
    let spent = h.node(ada).group_state(&site).unwrap().claims_spent;
    assert_eq!(spent.len(), 2, "each claim spent once: {spent:?}");
    assert!(spent.values().all(|m| *m == h.id(vis)), "both by the member: {spent:?}");

    let b = bundles();
    let again = h.node(arc).admit_by_claim(&site, &token(&site, 2, r#","c":"skills""#), &b, &kiosk_keys()).await.expect("the second claim again is a retry");
    assert_eq!(again.rooms, vec![skills.clone()]);
    h.settle().await;
    assert_eq!(h.node(ada).group_state(&site).unwrap().claims_spent.len(), 2, "nothing spent again");
}
