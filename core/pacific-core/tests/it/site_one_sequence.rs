//! A SITE, AS THE SIGN-UP MAKES IT, FROM ANY CLIENT: `mint` the Group, `site_host`,
//! `site_edit_face`, `site_publish` (Node, the one sequence RED and the webapp call).
//!
//!   the owner makes a Site with a face, and publishes it   →  the Arc serves face_spec's bundle
//!   a step repeated                                         →  nothing made twice
//!   the face edited after publication                       →  the Arc serves the edit
//!   an admin edits the face, not on the Host                →  the face counts, pictures refused
//!   a Save or a Publish                                      →  one send per op it changed; none if none
//!   a device that has never seen it, from the words         →  the same face, address, Host, pictures

use crate::common;

use common::Harness;
use pacific_core::face_spec;
use pacific_core::mint::MintDraft;
use pacific_core::object::ObjectKind;
use pacific_core::resumption::HeadInput;
use pacific_core::roles::OP_SET_ROLE;

const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

fn face(name: &str) -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/faces").join(name);
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    v.to_string()
}

fn draft() -> MintDraft {
    MintDraft {
        name: "Riverside".into(),
        shape: "community".into(),
        card: r#"{"note":"A garden by the river."}"#.into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_site_made_and_published_by_one_sequence_is_served_and_its_edits_too() {
    let mut h = Harness::people(&[("ada", &["phone"]), ("arc", &["node"])]).await;
    let (phone, arc) = (h.device("ada", "phone"), h.device("arc", "node"));
    let arc_id = h.id(arc);

    let site = h.with(phone, |n| async move { n.mint(ObjectKind::Group, &draft()).await }).await.unwrap();
    let host = h.node(phone).site_host(&site).await.unwrap();
    assert_eq!(h.node(phone).site_host(&site).await.unwrap(), host, "a retry makes no second Host");

    let every = face("every-part.json");
    let media = vec![("mark".to_string(), "image/png".to_string(), PNG_B64.to_string())];
    assert!(!h.node(phone).site_edit_face(&site, &every, &media).await.unwrap(), "not published yet");
    h.node(phone)
        .site_edit_face(&site, r#"{"look":1}"#, &[])
        .await
        .expect_err("a face the specification refuses is refused before a delta");
    assert_eq!(h.node(phone).site_face(&site).unwrap().face, every, "and the face stands");

    let bundle = h.node(arc).build_contact_bundle().unwrap();
    h.node(phone).site_publish(&site, "riverside", &bundle).await.unwrap();
    h.node(phone).site_publish(&site, "riverside", &bundle).await.unwrap();
    h.settle().await;
    assert_eq!(h.roster(phone, &host).iter().filter(|k| **k == arc_id).count(), 1, "the Arc, added once");

    let scan = h.node(arc).host_sites(&arc_id).unwrap();
    assert_eq!(scan.sites.len(), 1, "{:?}", scan.refused);
    let served = &scan.sites[0];
    assert_eq!(served.slug, "riverside");
    assert_eq!(served.media, vec![("mark".to_string(), "image/png".to_string())]);
    let working: serde_json::Value = serde_json::from_str(&every).unwrap();
    let want = face_spec::bundle("Riverside", "community", "A garden by the river.", &working);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&served.bundle).unwrap(), want, "the page face_spec publishes");

    // EDITED AFTER PUBLICATION: the Arc serves the edit.
    let paper = face("preset-paper.json");
    assert!(h.node(phone).site_edit_face(&site, &paper, &[]).await.unwrap(), "published, so hydrated again");
    h.settle().await;
    let served = h.node(arc).host_sites(&arc_id).unwrap().sites.remove(0).bundle;
    let want = face_spec::bundle("Riverside", "community", "A garden by the river.", &serde_json::from_str(&paper).unwrap());
    assert_eq!(serde_json::from_str::<serde_json::Value>(&served).unwrap(), want, "the edit, served");
}

/// The phone made and published a Site; a device signed in from the words while the phone
/// is still about reads all of it, pictures included, once the phone's own sync has
/// restated what it owns (NC-65: sync_once step 5c).
#[tokio::test]
async fn a_device_restored_from_the_words_reads_the_whole_site() {
    let mut h = Harness::people(&[("ada", &["phone"]), ("arc", &["node"])]).await;
    let (phone, arc) = (h.device("ada", "phone"), h.device("arc", "node"));
    let site = h.with(phone, |n| async move { n.mint(ObjectKind::Group, &draft()).await }).await.unwrap();
    let host = h.node(phone).site_host(&site).await.unwrap();
    let paper = face("preset-paper.json");
    let media = vec![("mark".to_string(), "image/png".to_string(), PNG_B64.to_string())];
    h.node(phone).site_edit_face(&site, &paper, &media).await.unwrap();
    let bundle = h.node(arc).build_contact_bundle().unwrap();
    h.node(phone).site_publish(&site, "riverside", &bundle).await.unwrap();
    h.settle().await;

    let words = h.with_sync(phone, |n| n.recovery_key()).expect("words");
    let head = h.with_sync(phone, |n| n.head_update().unwrap());
    let (sim, _) = h
        .sign_in_from_words("ada", "sim", &words, Some(HeadInput { blob: head.sealed, declared_position: head.position }))
        .await
        .unwrap();
    h.settle().await;
    let seen = h.node(sim).site_face(&site).unwrap();
    assert_eq!(seen.face, paper);
    assert_eq!(seen.host.as_deref(), Some(host.as_str()));
    assert_eq!((seen.slug.as_str(), seen.published), ("riverside", true));
    assert_eq!(seen.media.iter().map(|m| m.0.as_str()).collect::<Vec<_>>(), vec!["mark"]);
}

/// THE RELAY-WRITE GATE (8 Nov): a Save or a Publish sends each op it changed once, and a
/// Save that changes nothing sends nothing. Counted where every publish is counted
/// (`Node::published`), across the act and its own send.
#[tokio::test]
async fn a_save_or_a_publish_sends_only_what_it_changed() {
    let h = Harness::people(&[("ada", &["phone"]), ("bo", &["phone"]), ("arc", &["node"])]).await;
    let (ada, bo, arc) = (h.device("ada", "phone"), h.device("bo", "phone"), h.device("arc", "node"));
    let site = h.with(ada, |n| async move { n.mint(ObjectKind::Group, &draft()).await }).await.unwrap();
    h.add_to_forum(ada, bo, &site).await;
    h.node(ada).site_host(&site).await.unwrap();
    let bundle = h.node(arc).build_contact_bundle().unwrap();
    h.node(ada).site_publish(&site, "riverside", &bundle).await.unwrap();
    h.settle().await;

    let sent = |h: &Harness| h.node(ada).published();
    let mark = vec![("mark".to_string(), "image/png".to_string(), PNG_B64.to_string())];
    let every = face("every-part.json");

    // The face, a picture and the page: three ops, three sends.
    let before = sent(&h);
    h.node(ada).site_edit_face(&site, &every, &mark).await.unwrap();
    assert_eq!(sent(&h) - before, 3, "group.editFace, host.editMedia, host.hydrate");
    h.settle().await;

    // The same Save again: nothing changed, nothing sent.
    let before = sent(&h);
    assert!(!h.node(ada).site_edit_face(&site, &every, &mark).await.unwrap(), "no hydrate");
    assert_eq!(sent(&h) - before, 0, "an unchanged Save sends nothing");

    // Only the picture: one op.
    let other = vec![("mark".to_string(), "image/png".to_string(), PNG_B64.replace("A60e6kg", "A60e6kh"))];
    let before = sent(&h);
    h.node(ada).site_edit_face(&site, &every, &other).await.unwrap();
    assert_eq!(sent(&h) - before, 1, "host.editMedia alone: the page carries no picture");
    h.settle().await;

    // Only the face: it and the page.
    let paper = face("preset-paper.json");
    let before = sent(&h);
    assert!(h.node(ada).site_edit_face(&site, &paper, &[]).await.unwrap());
    assert_eq!(sent(&h) - before, 2, "group.editFace, host.hydrate");
    h.settle().await;

    // Publish again, the same address: nothing.
    let before = sent(&h);
    h.node(ada).site_publish(&site, "riverside", &bundle).await.unwrap();
    assert_eq!(sent(&h) - before, 0, "a Publish that moves nothing sends nothing");
    let before = sent(&h);
    h.node(ada).site_host(&site).await.unwrap();
    assert_eq!(sent(&h) - before, 0, "nor does asking for the Host again");
}

#[tokio::test]
async fn an_admin_edits_the_face_but_not_the_pictures_of_a_host_they_are_not_on() {
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let site = h.with(ada, |n| async move { n.mint(ObjectKind::Group, &draft()).await }).await.unwrap();
    h.node(ada).site_host(&site).await.unwrap();
    h.add_to_forum(ada, bo, &site).await;
    let mut role = pacific_core::coordinator::Args::new();
    role.insert("member".into(), pacific_core::coordinator::ArgVal::Text(hex::encode(h.id(bo))));
    role.insert("role".into(), pacific_core::coordinator::ArgVal::Text("admin".into()));
    h.node(ada).apply(&site, OP_SET_ROLE, role).await.unwrap();
    h.settle().await;

    let mark = vec![("mark".to_string(), "image/png".to_string(), PNG_B64.to_string())];
    let why = h.node(bo).site_edit_face(&site, &face("default.json"), &mark).await.unwrap_err().to_string();
    assert!(why.contains("not on its Host"), "{why}");
    h.node(bo).site_edit_face(&site, &face("default.json"), &[]).await.expect("the face itself");
    h.settle().await;
    assert_eq!(h.node(ada).site_face(&site).unwrap().face, face("default.json"), "the admin's face counts");
}

#[test]
fn a_page_is_offered_where_the_icd_makes_a_kind_of_a_host() {
    assert_eq!(face_spec::kinds_with_a_page(), ["group".to_string()]);
}
