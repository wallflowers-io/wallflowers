//! W-98 Trade, TB4 (TEST, run 112): what a view states against the reader's clock is
//! derived when it is read, never frozen in the view the fold cache holds (O-69). The ICD:
//! thing.setDisposition's view, "reserved past `until` reads available"; a Transaction's
//! completion by silence (stateMachine.silence). The cache is on here, as in production.

mod common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::mint::MintDraft;
use pacific_core::object::ObjectKind;

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

#[tokio::test]
async fn a_reservation_past_its_until_reads_available_through_the_cached_view() {
    std::env::set_var("PACIFIC_FOLD_CACHE_MODEL", "w98-read-time");
    let h = Harness::new(&["Ana"]).await;
    let n = h.node(0);
    let thing = n.mint(ObjectKind::Thing, &MintDraft { name: "bike".into(), ..Default::default() }).await.expect("mint");
    let until = now_ms() + 1500;
    let mut a = Args::new();
    a.insert("state".into(), ArgVal::Text("reserved".into()));
    a.insert("for".into(), ArgVal::Text("ab".repeat(32)));
    a.insert("until".into(), ArgVal::Int(until));
    a.insert("at".into(), ArgVal::Int(now_ms()));
    n.apply(&thing, pacific_core::thing::OP_SET_DISPOSITION, a).await.expect("setDisposition");

    let read = || -> serde_json::Value { serde_json::from_str(&n.object_view(&thing).unwrap()).unwrap() };
    assert_eq!(read()["disposition"]["state"], "reserved", "held until {until}");
    tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
    let v = read();
    assert_eq!(v["disposition"]["state"], "available", "past until, the same view read again: {v}");
    assert_eq!(v["disposition"]["until"], until, "the hold is still stated");
}

/// The read-time step by kind (`fold::at_read`): a deal accepted with one attestation reads
/// completed once its confirmWithin has passed, naming who attested; before it, accepted;
/// a view of another kind is returned as it came.
#[test]
fn a_deals_silence_is_read_against_the_readers_clock() {
    let deal = serde_json::json!({ "state": "accepted", "completedBy": null, "terms": { "confirmWithin": 1000 }, "receipt": null, "sale": { "at": 5000 } }).to_string();
    let at = |now: i64| -> serde_json::Value { serde_json::from_str(&pacific_core::fold::at_read("transaction", deal.clone(), now)).unwrap() };
    assert_eq!(at(5999)["state"], "accepted");
    assert_eq!(at(6000)["state"], "completed");
    assert_eq!(at(6000)["completedBy"], "seller");
    let post = r#"{"title":"t","until":1}"#.to_string();
    assert_eq!(pacific_core::fold::at_read("post", post.clone(), i64::MAX), post);
}
