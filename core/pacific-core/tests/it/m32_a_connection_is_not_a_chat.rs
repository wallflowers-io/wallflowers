//! m32 — the migration: a connection is the pairing channel, and the chat is a
//! Conversation derived from it.
//!
//! Ruled 24 September 2026, with the messages on disk declared disposable —
//! WallFlowers is pre-users, and this is the last moment the change is free.
//!
//! What it ends: a connection's log carried two vocabularies at once, Forum
//! messages beside Contact prekeys, and `lens_for("connection")` said Forum —
//! the code stating that a connection is a chat room, which the ICD denies in so
//! many words ("the 1:1 pairing channel: prekey pool + pairwise fan-out. Not an
//! entity itself").
//!
//! What it starts: `conversation_open(peer)` derives the chat by CONSUMING a
//! prekey the connection carries, which is what the pool has always been for.

use crate::common;

use common::Harness;
use pacific_core::object::ObjectKind;

/// The connection folds as the pairing channel, and it will not take a message.
#[tokio::test]
async fn a_connection_carries_no_messages() {
    let h = Harness::new(&["ana", "ben"]).await;
    h.pair(0, 1).await;

    assert_eq!(
        pacific_core::fold::lens_for("connection"),
        Some(ObjectKind::Contact),
        "a connection is the pairing channel"
    );

    let n = h.node(0);
    let connection = n
        .connection_object(&h.id(1))
        .expect("read")
        .expect("paired")
        .0;
    let refused = n.object_post_reply(&connection, "hello?", None, None).await;
    assert!(
        refused.is_err(),
        "a pairing channel must refuse a message rather than store one no fold reads"
    );
}

/// PAIRING OPENS EXACTLY ONE CONVERSATION, and both sides fold it.
///
/// Who opens it is decided by the pairing rather than by whoever speaks first:
/// two people messaging before either has synced would otherwise derive one
/// conversation each and split the thread, with nothing afterwards to say which
/// was meant. The SCANNED side opens it, because the scanner published its
/// prekeys into the channel during `pair_scan` and the scanned side drained them
/// to get here.
#[tokio::test]
async fn pairing_opens_one_conversation_on_both_sides() {
    let h = Harness::new(&["ana", "ben"]).await;
    h.pair(0, 1).await;

    h.node(0)
        .author_post(&h.id(1), "hello over the conversation")
        .await
        .expect("the first post derives it");
    h.settle().await;

    let convo = h
        .node(0)
        .conversation_with(&h.id(1))
        .unwrap()
        .expect("opened at pairing");

    for u in [0, 1] {
        let seen = h.node(u).dm_transcript(&h.id(1 - u)).expect("transcript");
        assert!(
            seen.iter().any(|l| l.contains("hello over the conversation")),
            "device {u} folds the message: {seen:?}"
        );
    }
    // The peer joined the SAME object, through the Welcome the prekey bought.
    assert_eq!(
        h.node(1).conversation_with(&h.id(0)).unwrap().as_deref(),
        Some(convo.as_str()),
        "one conversation, both sides"
    );
}

/// Connected and silent is an empty transcript, not a refusal. Never connected
/// is a refusal, not an empty transcript.
#[tokio::test]
async fn silence_and_absence_do_not_read_alike() {
    let h = Harness::new(&["ana", "ben"]).await;
    h.pair(0, 1).await;
    assert!(
        h.node(0).dm_transcript(&h.id(1)).expect("connected").is_empty(),
        "nothing said yet"
    );
    assert!(
        h.node(0).dm_transcript(&[9u8; 32]).is_err(),
        "no connection at all is loud"
    );
}

/// And the conversation is on both spines, like any other object.
#[tokio::test]
async fn the_conversation_takes_a_vertebra_on_both_spines() {
    let h = Harness::new(&["ana", "ben"]).await;
    h.pair(0, 1).await;
    h.node(0).author_post(&h.id(1), "one").await.expect("post");
    h.settle().await;

    let convo = h.node(0).conversation_with(&h.id(1)).unwrap().expect("derived");
    for u in [0, 1] {
        let named: Vec<String> = h
            .node(u)
            .spine_entries()
            .unwrap()
            .into_iter()
            .map(|(_, e)| hex::encode(e.body.group_id()))
            .collect();
        assert!(named.contains(&convo), "device {u} names it: {named:?}");
    }
}

/// A SYNC DOES NOT FOLD A CONNECTION AS A CHAT. `sync_group` built every object's transcript,
/// and a pairing channel has none (`chat_type_id` refuses it), so every tick named every
/// connection "object sync failed". Its rows are drained and its outbox flushed; that is all.
#[tokio::test]
async fn a_sync_names_no_connection_a_failure() {
    use tracing_subscriber::layer::SubscriberExt;
    let h = common::Harness::new(&["ada", "bo"]).await;
    h.pair(0, 1).await;
    let failed = Failed::default();
    let _on = tracing::subscriber::set_default(tracing_subscriber::registry().with(failed.clone()));
    h.sync(0).await;
    h.sync(1).await;
    assert_eq!(failed.0.lock().unwrap().clone(), Vec::<String>::new(), "no object's sync failed");
}

/// Every "object sync failed" a subscriber on this thread sees, with its error.
#[derive(Clone, Default)]
struct Failed(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Failed {
    fn on_event(&self, e: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        struct Fields(String, String);
        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
                match f.name() {
                    "message" => self.0 = format!("{v:?}"),
                    "error" => self.1 = format!("{v:?}"),
                    _ => {}
                }
            }
        }
        let mut m = Fields(String::new(), String::new());
        e.record(&mut m);
        if m.0.contains("object sync failed") {
            self.0.lock().unwrap().push(m.1);
        }
    }
}
