//! Reactions, replies and timestamps — through the real delta pipe.
//!
//! Two connected users. A posts; B reacts, changes the reaction, then clears it;
//! B also replies to A. Every step is a real Delta over a real relay, and both
//! replicas must converge on the same structured view (reactions grouped by
//! emoji, reply linkage, display timestamp). This proves the modern-chat
//! features are the residue of real ops, not UI state.

mod common;

use common::Harness;

#[tokio::test]
async fn reactions_replies_timestamps_converge() {
    let h = Harness::new(&["Nye Thompson", "Harvey Deane"]).await;
    let (a, b) = (0, 1);
    h.pair(a, b).await;

    // A posts a message; it carries a display timestamp.
    h.dm_post(a, b, "the tags don't match the rail").await;
    let target = {
        let view = h.dm_view(a, b);
        assert_eq!(view.len(), 1);
        assert!(view[0].ts > 0, "a post carries a display timestamp");
        (view[0].author, view[0].gen)
    };

    // B syncs, reacts ❤️, and REPLIES to A's message.
    h.sync(b).await;
    h.dm_react(b, a, target, "❤️", true).await;
    h.dm_reply(b, a, "on it — Bea's untangling", target).await;
    h.settle().await;

    // A converges: the post carries B's ❤️ and there's a reply pointing at it.
    let view = h.assert_dm_converges(a, b);
    let msg = view.iter().find(|m| (m.author, m.gen) == target).unwrap();
    assert_eq!(msg.reactions.len(), 1, "one emoji group");
    assert_eq!(msg.reactions[0].0, "❤️");
    assert_eq!(msg.reactions[0].1, vec![h.id(b)], "one reactor (B)");
    let reply = view.iter().find(|m| m.reply_to == Some(target)).unwrap();
    assert_eq!(
        reply.text, "on it — Bea's untangling",
        "reply linked to the target"
    );

    // B changes ❤️ → 👍 (replace, still one reaction).
    h.dm_react(b, a, target, "👍", true).await;
    {
        let m = h
            .dm_view(b, a)
            .into_iter()
            .find(|m| (m.author, m.gen) == target)
            .unwrap();
        assert_eq!(m.reactions.len(), 1, "replacing keeps a single reaction");
        assert_eq!(m.reactions[0].0, "👍", "emoji replaced");
    }

    // B clears it.
    h.dm_react(b, a, target, "👍", false).await;
    {
        let m = h
            .dm_view(b, a)
            .into_iter()
            .find(|m| (m.author, m.gen) == target)
            .unwrap();
        assert!(m.reactions.is_empty(), "clearing removes the reaction");
    }

    // Both replicas agree the reaction was cleared.
    h.settle().await;
    let view = h.assert_dm_converges(a, b);
    let m = view.iter().find(|m| (m.author, m.gen) == target).unwrap();
    assert!(
        m.reactions.is_empty(),
        "both replicas agree the reaction was cleared"
    );
}
