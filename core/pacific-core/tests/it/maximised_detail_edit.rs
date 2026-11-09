//! THE OWNER'S EDIT (`Node::edit`, the maximised detail's write path; ARC_NETWORK's merge
//! condition). `event.setProfile` is sequenced, so a write is a slot claim and a publish,
//! kept for good (#72). One changed value costs exactly that; restating a value the fold
//! already holds costs nothing, because nothing is sent.

use crate::common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::event::{self, OP_SET_PROFILE};
use pacific_core::router::published;

/// 12 Dec 2026, 19:00.
const START_MS: i64 = 1_797_100_000_000;

fn status(word: &str) -> Args {
    [("status".to_string(), ArgVal::Text(word.into()))].into_iter().collect()
}

#[tokio::test]
async fn an_owners_edit_costs_one_claim_and_one_publish_and_restating_it_costs_nothing() {
    let h = Harness::people(&[("axel", &["phone", "laptop"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");
    let feast = h.form_object(phone, "event", &[]).await;
    h.add_to_forum(phone, laptop, &feast).await;
    h.node(phone)
        .apply(&feast, OP_SET_PROFILE, event::set_profile_args("Feast", None, START_MS, None, None, None, None))
        .await
        .unwrap();
    h.settle().await;

    let before = published();
    let sent = h.node(phone).edit(&feast, "event.setProfile", status("postponed")).await.unwrap();
    assert!(sent.is_some(), "a changed value is written");
    assert_eq!(published() - before, 2, "a claim and a publish, exactly");
    h.settle().await;
    let st = h.node(laptop).event_state(&feast).unwrap();
    assert_eq!((st.status.as_str(), st.title.as_str(), st.start_ms), ("postponed", "Feast", START_MS),
               "the other device folds the one change, and the rest stands");

    let before = published();
    let sent = h.node(phone).edit(&feast, "event.setProfile", status("postponed")).await.unwrap();
    assert!(sent.is_none(), "the same value again is no write");
    assert_eq!(published() - before, 0, "and nothing is published");
}
