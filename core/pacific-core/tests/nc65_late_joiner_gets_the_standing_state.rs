//! NC-65 (Ralph, 27 Sep: "(a) Owner re-emits"): a member someone else admitted, the Arc's
//! node at a kiosk (A-3), holds nothing of the Site from before it joined, until the
//! owner's device restates its standing state (`Node::restate_for_joiners`): the Face,
//! the parts, and each room's parent. Posts stay dark to late joiners, as the doctrine
//! pins (`reemit_own_spine`).
//!
//! Since O-75 an admission BY CLAIM also carries the Site's history from the Arc, so the
//! owner's restate is tested where the Arc adds the member itself (`group_add_member`,
//! which sends none), and O-75's promise beside it: through `admit_by_claim`, with the
//! owner offline, the member holds the Site's standing state at admission.

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

const FACE: &str = r##"{"v":1,"look":{"colours":{"paper":"#f6f1e7"}}}"##;

/// ada's Site, its Host and its room, as the Register and step 3 make them, with its Face
/// and the Arc (member 1) granted admitter on the Site and the room.
async fn a_site(h: &Harness) -> (String, String, String) {
    let (ada, arc) = (0, 1);
    let site = h.mint_object(ada, ObjectKind::Group, "Egregore", &[arc]).await;
    let host = h.mint_object(ada, ObjectKind::Host, "Egregore", &[]).await;
    let room = h.mint_object(ada, ObjectKind::Forum, "Talk", &[arc]).await;

    // The owner's setup, as the Register and step 3 make it: the Host and the room, both
    // ends of each edge; the Face (a Site has a Host part, A-11); the Arc's admitter role.
    {
        let f = h.node(ada);
        for (part, role) in [(&host, "host"), (&room, "room")] {
            f.apply(&site, pacific_core::parts::OP_SET_PART, pacific_core::parts::set_part_args(part, role, now_ms()))
                .await
                .expect("the part is the Site's");
            f.apply(part, pacific_core::parent::OP_SET_PARENT, pacific_core::parent::set_parent_args(&site, role, now_ms()))
                .await
                .expect("the Site is the part's parent");
        }
        f.apply(&site, pacific_core::group::OP_SET_FACE, args(&[("face", FACE)])).await.expect("the Site's face");
        let admitter = hex::encode(h.id(arc));
        for o in [&site, &room] {
            f.apply(o, pacific_core::roles::OP_SET_ROLE, args(&[("member", &admitter), ("role", "admitter")]))
                .await
                .expect("the founder grants the admitter role");
        }
    }
    h.settle().await;
    (site, host, room)
}

#[tokio::test]
async fn a_member_the_arc_admits_gets_the_sites_standing_state_from_its_owner() {
    let h = Harness::new(&["ada", "arc", "vis", "bea"]).await;
    let (ada, arc, vis, bea) = (0, 1, 2, 3);
    let (site, host, room) = a_site(&h).await;

    // The Arc adds a visitor to the Site and its room, itself, while the owner does nothing:
    // `group_add_member`, which carries no history (O-75's is `admit_by_claim`'s).
    let admit = |who: usize| {
        let packages: Vec<String> = (0..2).map(|_| h.node(who).build_contact_bundle().unwrap()).collect();
        let (h, site, room) = (&h, site.clone(), room.clone());
        async move {
            h.node(arc).group_add_member(&site, &packages[0]).await.expect("the Arc adds the visitor to the Site");
            h.node(arc).group_add_member(&room, &packages[1]).await.expect("and to the room");
        }
    };
    admit(vis).await;
    h.settle().await;
    assert_eq!(h.node(vis).object_kind(&room).expect("the visitor holds the room"), "forum");

    // THE DEFECT: the visitor holds the Site and the room, and none of their standing state.
    let before = h.node(vis).group_state(&site).unwrap();
    assert!(before.face.is_empty(), "no Face before the owner restates: {}", before.face);
    assert!(!before.parts.contains_key(&room), "no parts before the owner restates: {:?}", before.parts);

    // The owner's device restates, once; owed nothing, it sends nothing.
    let f = h.node(ada);
    assert!(f.restate_for_joiners().await.unwrap() > 0, "the owner restates what it owns");
    assert_eq!(f.restate_for_joiners().await.unwrap(), 0, "nothing is owed twice");
    h.settle().await;

    let after = h.node(vis).group_state(&site).unwrap();
    assert_eq!(after.face, FACE, "the visitor folds the Site's Face");
    assert_eq!(after.parts.get(&room).map(|p| p.role.as_str()), Some("room"), "and its room: {:?}", after.parts);
    assert_eq!(after.parts.get(&host).map(|p| p.role.as_str()), Some("host"), "and its Host");
    let room_view = h.node(vis).object_view(&room).unwrap();
    assert!(room_view.contains(&site), "the room names its Site for the visitor: {room_view}");
    assert!(h.node(vis).noncompliant_objects().unwrap().is_empty(), "everything the visitor holds folds");
    assert!(h.node(ada).noncompliant_objects().unwrap().is_empty(), "and the owner's too");

    // A second visitor, later: the same device sees the roster grow and restates again.
    admit(bea).await;
    h.settle().await;
    let _ = h.node(ada); // the process env back on the owner; `f` keeps what it restated
    assert!(f.restate_for_joiners().await.unwrap() > 0, "a new member is owed the standing state");
    h.settle().await;
    assert_eq!(h.node(bea).group_state(&site).unwrap().face, FACE, "the second visitor folds the Face");
    assert_eq!(h.node(vis).group_state(&site).unwrap().face, FACE, "and the first still does");
}

/// O-75, in NC-65's own file: admitted BY CLAIM, the member has the Site's standing state at
/// admission, from the Arc, with the owner offline and restating nothing: the Face, the
/// parts, and the room's parent.
#[tokio::test]
async fn a_member_admitted_by_claim_has_the_sites_standing_state_at_admission_with_the_owner_offline() {
    let h = Harness::new(&["ada", "arc", "vis"]).await;
    let (ada, arc, vis) = (0, 1, 2);
    let (site, host, room) = a_site(&h).await;
    h.partition(ada);
    let packages: Vec<String> = (0..2).map(|_| h.node(vis).build_contact_bundle().unwrap()).collect();
    let a = h.node(arc).admit_by_claim(&site, &claim(&site, 1), &packages, &kiosk_keys()).await.expect("admitted by claim");
    assert_eq!(a.rooms, vec![room.clone()]);
    // Only the visitor syncs: the owner is off the relay.
    h.sync(vis).await;

    let st = h.node(vis).group_state(&site).unwrap();
    assert_eq!(st.face, FACE, "the visitor folds the Site's Face, with the owner offline");
    assert_eq!(st.parts.get(&room).map(|p| p.role.as_str()), Some("room"), "and its room: {:?}", st.parts);
    assert_eq!(st.parts.get(&host).map(|p| p.role.as_str()), Some("host"), "and its Host");
    let room_view = h.node(vis).object_view(&room).unwrap();
    assert!(room_view.contains(&site), "the room names its Site for the visitor: {room_view}");
    assert!(h.node(vis).noncompliant_objects().unwrap().is_empty(), "everything the visitor holds folds");
}
