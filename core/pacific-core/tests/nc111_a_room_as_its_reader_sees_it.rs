//! A room's view, as each member's own Node serves it (`Node::object_view`).
//!
//! NC-111: a reaction is served `[emoji, count, mine]`, `mine` true when the reader's own key
//! is among that emoji's reactors. No reader is served another's key.
//! WEBAPP REQUIREMENTS § 7: a message's picture is served, by the keys `forum.post` carries it
//! under (`MediaRef::to_args("media")`, the ICD's args).

mod common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args, FORUM_POST, FORUM_REACT};
use pacific_core::object::ObjectKind;
use serde_json::{json, Value};

fn view(h: &Harness, u: usize, room: &str) -> Value {
    serde_json::from_str(&h.node(u).object_view(room).expect("the room folds")).expect("the view is JSON")
}

fn text(s: &str) -> ArgVal {
    ArgVal::Text(s.into())
}

fn as_json(v: &ArgVal) -> Value {
    match v {
        ArgVal::Int(i) => json!(i),
        ArgVal::Text(t) => json!(t),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reader_is_told_its_own_reaction_and_no_ones_key() {
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let (ada, bo, cy) = (0, 1, 2);
    let room = h.mint_object(ada, ObjectKind::Forum, "Talk", &[bo, cy]).await;
    h.settle().await;
    let first: Args = [("text".to_string(), text("hello"))].into();
    h.node(ada).apply(&room, FORUM_POST, first).await.expect("ada posts");
    h.settle().await;
    let gen = view(&h, ada, &room)["messages"][0]["gen"].as_i64().expect("the post's gen");
    for (who, emoji) in [(bo, "👍"), (cy, "🎉")] {
        let react: Args = [
            ("target_author".to_string(), text(&hex::encode(h.id(ada)))),
            ("target_gen".to_string(), ArgVal::Int(gen)),
            ("emoji".to_string(), text(emoji)),
            ("active".to_string(), ArgVal::Int(1)),
        ]
        .into();
        h.node(who).apply(&room, FORUM_REACT, react).await.expect("a member reacts");
    }
    h.settle().await;

    for (reader, own) in [(ada, None), (bo, Some("👍")), (cy, Some("🎉"))] {
        let v = view(&h, reader, &room);
        let reactions = &v["messages"][0]["reactions"];
        let want: Vec<Value> = ["🎉", "👍"].iter().map(|e| json!([e, 1, Some(*e) == own])).collect();
        assert_eq!(reactions, &json!(want), "as {} reads it", h.name(reader));
        let served = reactions.to_string();
        for u in [ada, bo, cy] {
            assert!(!served.contains(&hex::encode(h.id(u))), "{}'s key is served to {}", h.name(u), h.name(reader));
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_messages_picture_is_served_by_its_args_names() {
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let room = h.mint_object(ada, ObjectKind::Forum, "Talk", &[bo]).await;
    h.settle().await;
    let mut picture = Args::new();
    pacific_media::MediaRef::inline("image/png", "cHJvYmU=").expect("a picture").with_intrinsics(1, 1, 0).to_args("media", &mut picture);
    let mut post: Args = [("text".to_string(), text("look"))].into();
    post.extend(picture.iter().map(|(k, v)| (k.clone(), match v {
        pacific_media::ArgVal::Int(i) => ArgVal::Int(*i),
        pacific_media::ArgVal::Text(t) => ArgVal::Text(t.clone()),
    })));
    h.node(ada).apply(&room, FORUM_POST, post.clone()).await.expect("ada posts a picture");
    let plain: Args = [("text".to_string(), text("no picture"))].into();
    h.node(ada).apply(&room, FORUM_POST, plain).await.expect("and a line");
    h.settle().await;

    let v = view(&h, bo, &room);
    let msgs = v["messages"].as_array().expect("messages");
    let look = msgs.iter().find(|m| m["text"] == "look").expect("the picture's message");
    for k in picture.keys() {
        assert_eq!(look[k], as_json(&post[k]), "`{k}`, as the post carried it");
    }
    let line = msgs.iter().find(|m| m["text"] == "no picture").expect("the line");
    for k in picture.keys() {
        assert!(line.get(k).is_none(), "a message without a picture serves no `{k}`");
    }
}
