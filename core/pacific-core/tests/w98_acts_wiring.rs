//! W-98 performs_at on the Face (ASSURANCE 2), end to end on one device: `Node::host_sync_items`
//! passes face_items the performers whose own half this device holds, and the Host copy of a
//! network event carries those acts and no other. An organisation's half is on its group; a
//! person's on their self record, which only they hold, so another person's act stays in MLS.

mod common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::mint::MintDraft;
use pacific_core::object::ObjectKind;
use serde_json::Value;

const DAY: i64 = 86_400_000;
const STRANGER: [u8; 32] = [9; 32];

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// A performer's half: group.setAffiliation {rel: performs_at, peer: the event}.
fn performs_at(event: &str, at: i64) -> Args {
    let mut a = Args::new();
    a.insert("peer".into(), ArgVal::Text(event.into()));
    a.insert("rel".into(), ArgVal::Text("performs_at".into()));
    a.insert("name".into(), ArgVal::Text("Night market".into()));
    a.insert("at".into(), ArgVal::Int(at));
    a
}

#[tokio::test]
async fn a_confirmed_act_reaches_the_host_and_an_unconfirmed_one_does_not() {
    let h = Harness::people(&[("ada", &["phone"])]).await;
    let phone = h.device("ada", "phone");
    let me = hex::encode(h.id(phone));
    let site = h.mint_group(phone);
    h.group_set_profile(phone, &site, "Mill Road", "community").await;
    let host = h.node(phone).host_new(&site, "Mill Road").await.unwrap();
    let band = h.mint_group(phone);
    h.group_set_profile(phone, &band, "The Band", "team").await;

    let at = now_ms();
    let event = h.node(phone).mint(ObjectKind::Event, &MintDraft { name: "Night market".into(), start_ms: at + DAY, ..Default::default() }).await.unwrap();
    h.node(phone).group_attach_created(&site, &event, "Night market", at).await.unwrap();
    h.node(phone)
        .apply(&event, pacific_core::backlink::OP_SET_BACKLINK, pacific_core::backlink::set_backlink_args(&site, "created", at))
        .await
        .unwrap();

    let acts = serde_json::json!([
        { "object": band, "role": "headliner" },
        { "member": me, "role": "host" },
        { "member": hex::encode(STRANGER), "role": "support" },
    ]);
    let mut lineup = Args::new();
    lineup.insert("acts".into(), ArgVal::Text(acts.to_string()));
    h.node(phone).apply(&event, pacific_core::event::OP_SET_LINEUP, lineup).await.unwrap();

    // The band's half on its group, and Ada's on her self record. The stranger declares none.
    h.node(phone).apply(&band, pacific_core::group::OP_SET_AFFILIATION, performs_at(&event, at)).await.unwrap();
    let record = h.node(phone).spine().unwrap().expect("Ada's self record");
    h.node(phone).apply(&record, pacific_core::group::OP_SET_AFFILIATION, performs_at(&event, at)).await.unwrap();

    h.node(phone).host_sync_items(&site).await.unwrap();
    let view: Value = serde_json::from_str(&h.node(phone).object_view(&host).unwrap()).unwrap();
    let item = view["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["key"] == format!("event:{event}"))
        .expect("the event on the Host");
    let payload: Value = serde_json::from_str(item["payload"].as_str().unwrap()).unwrap();
    let shown: Vec<String> = payload["acts"]
        .as_array()
        .expect("the confirmed acts")
        .iter()
        .map(|a| a["object"].as_str().or(a["member"].as_str()).unwrap().to_string())
        .collect();
    assert_eq!(shown, vec![band.clone(), me.clone()], "the band and Ada, in billing order");
    assert!(!item["payload"].as_str().unwrap().contains(&hex::encode(STRANGER)), "the unconfirmed act stays inside MLS");
}

/// The ICD: an act is confirmed "to a reader who holds the performer's half". On Ada's device
/// her own act and her band's read confirmed once each has written its half; on Bob's, who
/// holds the event and neither half, every act reads unconfirmed; and the stranger's act is
/// unconfirmed to both.
#[tokio::test]
async fn a_performer_sees_their_act_confirmed_and_a_reader_without_the_half_does_not() {
    let h = Harness::people(&[("ada", &["phone"]), ("bob", &["phone"])]).await;
    let (ada, bob) = (h.device("ada", "phone"), h.device("bob", "phone"));
    let me = hex::encode(h.id(ada));
    let band = h.mint_group(ada);
    h.group_set_profile(ada, &band, "The Band", "team").await;
    let at = now_ms();
    let event = h.node(ada).mint(ObjectKind::Event, &MintDraft { name: "Night market".into(), start_ms: at + DAY, ..Default::default() }).await.unwrap();
    let acts = serde_json::json!([
        { "object": band, "role": "headliner" },
        { "member": me, "role": "host" },
        { "member": hex::encode(STRANGER), "role": "support" },
    ]);
    let mut lineup = Args::new();
    lineup.insert("acts".into(), ArgVal::Text(acts.to_string()));
    h.node(ada).apply(&event, pacific_core::event::OP_SET_LINEUP, lineup).await.unwrap();
    let bundle = h.node(bob).build_contact_bundle().unwrap();
    h.node(ada).group_add_member(&event, &bundle).await.unwrap();

    let confirmed = |u: usize| -> Vec<bool> {
        let v: Value = serde_json::from_str(&h.node(u).object_view(&event).unwrap()).unwrap();
        v["acts"].as_array().expect("acts").iter().map(|a| a["confirmed"].as_bool().unwrap()).collect()
    };
    assert_eq!(confirmed(ada), vec![false, false, false], "no half written yet");

    h.node(ada).apply(&band, pacific_core::group::OP_SET_AFFILIATION, performs_at(&event, at)).await.unwrap();
    let record = h.node(ada).spine().unwrap().expect("Ada's self record");
    h.node(ada).apply(&record, pacific_core::group::OP_SET_AFFILIATION, performs_at(&event, at)).await.unwrap();
    h.settle().await;

    assert_eq!(confirmed(ada), vec![true, true, false], "Ada holds her band's half and her own");
    assert_eq!(confirmed(bob), vec![false, false, false], "Bob holds the event and neither half");
}
