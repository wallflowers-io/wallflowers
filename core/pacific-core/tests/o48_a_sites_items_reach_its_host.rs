//! A SITE'S OWN EVENTS AND POSTS REACH ITS HOST, AND ITS ARC (O-48).
//!
//! Ralph, 27 Sep: "The Face needs to be hydrated. This happens via the Group <-> Host
//! route." The owner's device folds the Site, takes what the Site names as `created` and
//! what names the Site back, and writes them onto the Host as `event:<id>` / `post:<id>`
//! items (`Node::host_sync_items`); the Arc, on the Host's roster, serves what it folds.
//! Over a real relay:
//!
//!   an event and a post, declared at both ends   →  on the Host, and at the Arc
//!   one the Site names and that does not name it →  left off, and named
//!   nothing moved                                 →  nothing written
//!   the post retracted, the event let go          →  withdrawn, and gone from the Arc
//!
//! Every op opens its own `Node` at the point of use (`h.node(u)`), as m29 explains.

mod common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::mint::MintDraft;
use pacific_core::object::ObjectKind;

const BUNDLE: &str = r#"{"v":1,"profile":{"displayName":"Mill Road Allotments","shape":"community"}}"#;
const DAY: i64 = 86_400_000;

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

fn peer(id: &str) -> Args {
    let mut a = Args::new();
    a.insert("peer".into(), ArgVal::Text(id.into()));
    a
}

/// Both halves of `created`, at one `at`, as the webapp's create() writes them.
async fn created(h: &Harness, u: usize, site: &str, object: &str, name: &str, at: i64) {
    h.node(u).group_attach_created(site, object, name, at).await.unwrap();
    h.node(u)
        .apply(object, pacific_core::backlink::OP_SET_BACKLINK, pacific_core::backlink::set_backlink_args(site, "created", at))
        .await
        .unwrap();
}

fn arc_items(h: &Harness, arc: usize, arc_id: &[u8; 32]) -> Vec<(String, serde_json::Value)> {
    let scan = h.node(arc).host_sites(arc_id).unwrap();
    assert_eq!(scan.sites.len(), 1, "the Arc serves the one Site: {:?}", scan.refused);
    scan.sites[0]
        .items
        .iter()
        .map(|i| (i.key.clone(), serde_json::from_str(&i.payload).unwrap()))
        .collect()
}

#[tokio::test]
async fn a_sites_own_events_and_posts_reach_its_host_and_leave_it() {
    let h = Harness::people(&[("ada", &["phone"]), ("arc", &["node"])]).await;
    let (phone, arc) = (h.device("ada", "phone"), h.device("arc", "node"));
    let arc_id = h.id(arc);

    let site = h.mint_group(phone);
    h.group_set_profile(phone, &site, "Mill Road Allotments", "community").await;
    let host = h.node(phone).host_new(&site, "Mill Road Allotments").await.unwrap();
    let bundle = h.node(arc).build_contact_bundle().unwrap();
    h.node(phone).group_add_member(&host, &bundle).await.unwrap();
    h.settle().await;
    h.node(phone).host_publish(&host, "allotments", &hex::encode(arc_id), BUNDLE, &[]).await.unwrap();

    let at = now_ms();
    let dig = MintDraft { name: "Dig day".into(), start_ms: at + DAY, venue: "The shed".into(), ..Default::default() };
    let event = h.node(phone).mint(ObjectKind::Event, &dig).await.unwrap();
    created(&h, phone, &site, &event, "Dig day", at).await;
    let notes = MintDraft { name: "Seed swap notes".into(), descriptor: "Bring spares.".into(), ..Default::default() };
    let post = h.node(phone).mint(ObjectKind::Post, &notes).await.unwrap();
    created(&h, phone, &site, &post, "Seed swap notes", at + 1).await;
    // THE SITE'S HALF ONLY, as the webapp wrote before it wrote both.
    let lone = h.node(phone).mint(ObjectKind::Post, &MintDraft { name: "Half declared".into(), ..Default::default() }).await.unwrap();
    h.node(phone).group_attach_created(&site, &lone, "Half declared", at + 2).await.unwrap();

    // NOTHING ON THE HOST YET, asserted before the positive.
    h.settle().await;
    assert!(arc_items(&h, arc, &arc_id).is_empty(), "no item reaches the Arc that no one put");

    let done = h.node(phone).host_sync_items(&site).await.unwrap();
    assert_eq!(done.host, host);
    let mut put = done.put.clone();
    put.sort();
    // The post's public page (W-98 Resources, ICD 2.3.1): its body, in one chunk here.
    let mut want = vec![format!("event:{event}"), format!("post:{post}"), format!("post:{post}:body:0")];
    want.sort();
    assert_eq!(put, want, "the event and the post declared at both ends, and the post's page");
    assert!(done.withdrawn.is_empty());
    assert_eq!(
        done.left,
        vec![(lone.clone(), "the Site names it, and it does not name the Site".to_string())],
        "the half-declared post is left off, and named"
    );

    h.settle().await;
    let items = arc_items(&h, arc, &arc_id);
    let ev = &items.iter().find(|(k, _)| k == &format!("event:{event}")).expect("the event reaches the Arc").1;
    assert_eq!((ev["title"].as_str(), ev["venue"].as_str(), ev["startMs"].as_i64()), (Some("Dig day"), Some("The shed"), Some(at + DAY)));
    let po = &items.iter().find(|(k, _)| k == &format!("post:{post}")).expect("the post reaches the Arc").1;
    assert_eq!((po["title"].as_str(), po["body"].as_str(), po["at"].as_i64()), (Some("Seed swap notes"), Some("Bring spares."), Some(at + 1)));
    assert!(!items.iter().any(|(k, _)| k.ends_with(&lone)), "and the half-declared one does not");

    // NOTHING MOVED, NOTHING WRITTEN: not a delta on the Host.
    let before = h.node(phone).object_log(&host).unwrap().len();
    let again = h.node(phone).host_sync_items(&site).await.unwrap();
    assert!(again.put.is_empty() && again.withdrawn.is_empty(), "{again:?}");
    assert_eq!(h.node(phone).object_log(&host).unwrap().len(), before, "an unchanged Site writes nothing");

    // THE SWEEP the Door runs finds this Site by its Host, and says the same.
    let all = h.node(phone).host_sync_all().await.unwrap();
    assert_eq!(all.len(), 1, "one Site with a Host: {all:?}");
    assert_eq!(all[0].0, site);
    assert!(all[0].1.as_ref().is_ok_and(|s| s.put.is_empty() && s.withdrawn.is_empty()));

    // WHAT LEFT IS WITHDRAWN: the post retracted by its author, the event let go by the Site.
    h.node(phone).apply(&post, pacific_core::post::OP_RETRACT, Args::new()).await.unwrap();
    h.node(phone).apply(&site, pacific_core::group::OP_CLEAR_AFFILIATION, peer(&event)).await.unwrap();
    let gone = h.node(phone).host_sync_items(&site).await.unwrap();
    let mut withdrawn = gone.withdrawn.clone();
    withdrawn.sort();
    assert_eq!(withdrawn, want, "the retracted post, its page, and the released event are withdrawn");
    assert!(gone.put.is_empty());

    h.settle().await;
    assert!(arc_items(&h, arc, &arc_id).is_empty(), "and the Arc serves neither");
}

#[tokio::test]
async fn only_the_sites_owner_hydrates_its_host() {
    let h = Harness::people(&[("ada", &["phone"]), ("bea", &["phone"])]).await;
    let (ada, bea) = (h.device("ada", "phone"), h.device("bea", "phone"));
    let site = h.mint_group(ada);
    h.group_set_profile(ada, &site, "Mill Road Allotments", "community").await;
    h.node(ada).host_new(&site, "Mill Road Allotments").await.unwrap();
    let bundle = h.node(bea).build_contact_bundle().unwrap();
    h.node(ada).group_add_member(&site, &bundle).await.unwrap();
    h.settle().await;

    let err = h.node(bea).host_sync_items(&site).await.unwrap_err().to_string();
    assert!(err.contains("only the Site's owner"), "{err}");
    assert!(h.node(bea).host_sync_all().await.unwrap().is_empty(), "a member's sweep owns no Site");

    // A GROUP WITHOUT A HOST is not a Site: said, and passed over by the sweep.
    let plain = h.mint_group(ada);
    let err = h.node(ada).host_sync_items(&plain).await.unwrap_err().to_string();
    assert!(err.contains("no Host"), "{err}");
    let all = h.node(ada).host_sync_all().await.unwrap();
    assert_eq!(all.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), vec![site.as_str()]);
}

/// A resource: a Post of form `pdf` with its document's `link` (ICD 2.1.0 rows 6–7), declared
/// at both ends, reaches the Arc's items with those args by their ICD names — what the Arc's
/// /v1/face/:slug/items serves a site of its own.
#[tokio::test]
async fn a_pdf_resource_reaches_the_arc_with_its_form_and_link() {
    let h = Harness::people(&[("ada", &["phone"]), ("arc", &["node"])]).await;
    let (phone, arc) = (h.device("ada", "phone"), h.device("arc", "node"));
    let arc_id = h.id(arc);
    let site = h.mint_group(phone);
    h.group_set_profile(phone, &site, "Egregore", "community").await;
    let host = h.node(phone).host_new(&site, "Egregore").await.unwrap();
    let bundle = h.node(arc).build_contact_bundle().unwrap();
    h.node(phone).group_add_member(&host, &bundle).await.unwrap();
    h.settle().await;
    h.node(phone).host_publish(&host, "egregore", &hex::encode(arc_id), BUNDLE, &[]).await.unwrap();

    let at = now_ms();
    let zine = h.node(phone).mint(ObjectKind::Post, &MintDraft { name: "Zine #1".into(), ..Default::default() }).await.unwrap();
    let link = "https://egregores-echoes.com/zine-1.pdf";
    let args: Args = [("title", "Zine #1"), ("form", "pdf"), ("link", link)].into_iter().map(|(k, v)| (k.to_string(), ArgVal::Text(v.into()))).collect();
    h.node(phone).apply(&zine, pacific_core::post::OP_SET_PROFILE, args).await.expect("a pdf resource");
    created(&h, phone, &site, &zine, "Zine #1", at).await;

    let done = h.node(phone).host_sync_items(&site).await.unwrap();
    assert_eq!(done.put, vec![format!("post:{zine}")]);
    h.settle().await;
    let items = arc_items(&h, arc, &arc_id);
    let po = &items.iter().find(|(k, _)| k == &format!("post:{zine}")).expect("the resource reaches the Arc").1;
    assert_eq!((po["title"].as_str(), po["form"].as_str(), po["link"].as_str(), po["at"].as_i64()), (Some("Zine #1"), Some("pdf"), Some(link), Some(at)));
}
