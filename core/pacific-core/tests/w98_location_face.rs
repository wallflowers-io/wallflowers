//! W-98: an event's venue on the Face follows its registration's `location` (group.setRegistration;
//! the ICD: "members: every member of the Site; kept off the public copy, which is outside MLS").
//! End to end on one device: the Site's owner syncs its Host, and a members-location event's
//! public copy carries no venue while a public one's does.

mod common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::mint::MintDraft;
use pacific_core::object::ObjectKind;
use serde_json::Value;

const DAY: i64 = 86_400_000;

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

fn icd_op(kind: &str, name: &str) -> u32 {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json");
    let icd: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    icd["kinds"][kind]["ops"][name]["op"].as_u64().unwrap() as u32
}

/// A Site's event with a venue, declared at both ends.
async fn event(h: &Harness, u: usize, site: &str, name: &str, venue: &str, at: i64) -> String {
    let id = h.node(u).mint(ObjectKind::Event, &MintDraft { name: name.into(), start_ms: at + DAY, venue: venue.into(), ..Default::default() }).await.unwrap();
    h.node(u).group_attach_created(site, &id, name, at).await.unwrap();
    h.node(u)
        .apply(&id, pacific_core::backlink::OP_SET_BACKLINK, pacific_core::backlink::set_backlink_args(site, "created", at))
        .await
        .unwrap();
    id
}

fn payload(view: &Value, key: &str) -> Value {
    let item = view["items"].as_array().unwrap().iter().find(|i| i["key"] == key).unwrap_or_else(|| panic!("{key} on the Host"));
    serde_json::from_str(item["payload"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn a_members_location_keeps_the_venue_off_the_face() {
    let h = Harness::people(&[("ada", &["phone"])]).await;
    let phone = h.device("ada", "phone");
    let site = h.mint_group(phone);
    h.group_set_profile(phone, &site, "Mill Road", "community").await;
    let host = h.node(phone).host_new(&site, "Mill Road").await.unwrap();
    let at = now_ms();
    let hidden = event(&h, phone, &site, "Supper club", "14 Mill Road, flat 2", at).await;
    let open = event(&h, phone, &site, "Open day", "The shed", at).await;

    let mut reg = Args::new();
    reg.insert("event".into(), ArgVal::Text(hidden.clone()));
    reg.insert("location".into(), ArgVal::Text("members".into()));
    h.node(phone).apply(&site, icd_op("group", "group.setRegistration"), reg).await.unwrap();

    h.node(phone).host_sync_items(&site).await.unwrap();
    let view: Value = serde_json::from_str(&h.node(phone).object_view(&host).unwrap()).unwrap();
    let kept = payload(&view, &format!("event:{hidden}"));
    assert_eq!(kept["title"], "Supper club", "the event itself is on the Face");
    assert!(kept.get("venue").is_none(), "its venue is the members' alone: {kept}");
    assert!(!view.to_string().contains("flat 2"), "nowhere on the Host");
    assert_eq!(payload(&view, &format!("event:{open}"))["venue"], "The shed", "a public location's venue is");
}
