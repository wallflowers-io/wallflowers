//! m34 — macro-nodes: an object is made of objects.
//!
//! Ruled 24 September 2026. A Post is not only a Post — it is a Post and its
//! comments section, and a comments section IS a Forum. Before this, four kinds
//! answered "where do comments go" four different ways (`post.comment` on the
//! Post's own log, `place.post` on the Place's, Forum deltas riding an Event's
//! log under a second type id) and two kinds had no answer at all.
//!
//! WHY IT IS WORTH AN EXTRA OBJECT. A hosted Forum has its own MLS roster, and a
//! roster is the whole of access control — so comments can be open under a
//! members-only Post, or members-only under a public one, with no new concept:
//! joining the room is the ordinary role-gated MLS operation it already was. A
//! comment op on the parent can never express that; it shares the parent's roster
//! by construction.
//!
//! EAGER, not lazy. Lazily would spare a quiet Post its second object, but then
//! the part does not exist until somebody acts — and WHO mints it is a race, the
//! one `conversation_open` had to settle by naming a side. At mint there is one
//! owner and no race.

mod common;

use common::Harness;
use pacific_core::mint::MintDraft;
use pacific_core::object::ObjectKind;

fn draft(name: &str) -> MintDraft {
    MintDraft { name: name.to_string(), ..Default::default() }
}

/// The shape is declared, not assumed.
#[test]
fn a_post_declares_its_comments_section() {
    let parts = ObjectKind::Post.parts();
    assert_eq!(parts.len(), 1, "a Post is made of itself and one part");
    assert_eq!(parts[0].role, "comments");
    assert_eq!(parts[0].kind, ObjectKind::Forum, "and the part is a Forum");
    assert!(
        ObjectKind::Forum.parts().is_empty(),
        "a Forum is not made of anything — nothing recurses without end"
    );
}

/// Minting the parent brings the whole shape into being, and the parent points at it.
#[tokio::test]
async fn minting_a_post_mints_its_forum_and_attaches_it() {
    let h = Harness::new(&["Ana"]).await;
    let post = h
        .node(0)
        .mint(ObjectKind::Post, &draft("a piece"))
        .await
        .expect("mint");

    let v: serde_json::Value =
        serde_json::from_str(&h.node(0).object_view(&post).unwrap()).unwrap();
    let parts = v["parts"].as_array().expect("a Post's view carries its parts");
    assert_eq!(parts.len(), 1, "one part, attached: {v}");
    assert_eq!(parts[0]["role"], "comments");

    // The part is a REAL object: its own kind, its own roster, its own log.
    let comments = parts[0]["part"].as_str().expect("the edge names it");
    assert_eq!(h.node(0).object_kind(comments).unwrap(), "forum");
    // And it names its parent: both halves of part_of, written by the one mint.
    let cv: serde_json::Value =
        serde_json::from_str(&h.node(0).object_view(comments).unwrap()).unwrap();
    assert_eq!(cv["parent"]["parent"], post.as_str(), "the comments name the Post: {cv}");
    assert_eq!(cv["parent"]["role"], "comments");
    h.node(0)
        .object_post_reply(comments, "first comment", None, None)
        .await
        .expect("and it takes messages, because it is a Forum");
    let seen = h.node(0).object_transcript(comments).unwrap();
    assert!(seen.iter().any(|l| l.contains("first comment")), "{seen:?}");
}

/// And it is on the spine like anything else — a macro-node costs its parts.
#[tokio::test]
async fn every_part_takes_its_own_vertebra() {
    let h = Harness::new(&["Ana"]).await;
    let before = h.node(0).spine_entries().unwrap().len();
    let post = h
        .node(0)
        .mint(ObjectKind::Post, &draft("a piece"))
        .await
        .expect("mint");
    let after = h.node(0).spine_entries().unwrap();
    assert_eq!(
        after.len(),
        before + 2,
        "the Post and its comments Forum, both nameable from the seed"
    );
    assert!(after.iter().any(|(_, e)| hex::encode(e.body.group_id()) == post));
}

/// EVERY KIND WITH PARTS NAMES THEM IN ITS VIEW. A part the view leaves out is a
/// comments section no surface can find: the Event's view dropped its parts
/// and a Thing had no view at all, so both drew their comments as loose objects.
#[tokio::test]
async fn every_view_names_the_parts_its_mint_made() {
    let h = Harness::new(&["Ana"]).await;
    for kind in ObjectKind::ALL.iter().copied() {
        if kind.parts().is_empty() || !pacific_core::mint::is_mintable(kind) {
            continue;
        }
        let mut d = draft("with parts");
        d.start_ms = 1_790_000_000_000;
        let id = h.node(0).mint(kind, &d).await.unwrap_or_else(|e| panic!("mint {kind:?}: {e}"));
        let v: serde_json::Value =
            serde_json::from_str(&h.node(0).object_view(&id).unwrap()).unwrap();
        let parts = v["parts"].as_array().unwrap_or_else(|| panic!("{kind:?}'s view names no parts: {v}"));
        assert_eq!(parts.len(), kind.parts().len(), "{kind:?}: {v}");
    }
}
