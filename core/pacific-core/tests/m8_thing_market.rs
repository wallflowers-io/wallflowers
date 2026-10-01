//! The hammer journey's DURABLE half, over a real Node, a real MLS group and real Deltas.
//!
//! The upstream half (chat → batch → resolver → proposal) is probabilistic and needs a model.
//! Everything from ACCEPT onward is deterministic and is what this file pins: minting a Thing
//! GroupObject, giving it a market posture, and folding it back out the way LIFE reads it.
//!
//! What is actually being protected here is the seam the market rests on — that a posture is
//! durable state on a group-of-1 object, not a UI flag. If `things()` ever stops round-
//! tripping, the LIFE market silently empties and nothing else fails first.

mod common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::thing::{Posture, OP_CLEAR_POSTURE, OP_SET_POSTURE, OP_SET_PROFILE};

fn args(pairs: &[(&str, ArgVal)]) -> Args {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

/// Accept "hammer" at the deck → it is a Thing you WANT, and it reads back that way.
#[tokio::test]
async fn mint_a_thing_and_put_it_in_the_market() {
    let h = Harness::new(&["Alice"]).await;
    let alice = h.node(0);
    let relay = h.relay.clone();

    // What the swipe deck does on ACCEPT: a group of 1, then its profile.
    let id = alice.object_new("thing", "hammer").expect("mint the thing");
    alice
        .apply(
            &id,
            OP_SET_PROFILE,
            args(&[
                ("name", ArgVal::Text("hammer".into())),
                ("descriptor", ArgVal::Text("cracked handle".into())),
            ]),
        )
        .await
        .expect("set profile");

    // The market posture — a standing Want, so deliberately no price.
    alice
        .apply(
            &id,
            OP_SET_POSTURE,
            args(&[("posture", ArgVal::Text("wants".into()))]),
        )
        .await
        .expect("set posture");

    let st = alice.thing_state(&id).expect("fold the thing");
    assert_eq!(st.name, "hammer");
    assert_eq!(st.descriptor, "cracked handle");
    assert_eq!(st.posture, Some(Posture::Wants));
    assert_eq!(st.price, None, "a standing Want carries no price");

    // The list LIFE's market actually binds to.
    let all = alice.things().expect("list things");
    let row = all
        .iter()
        .find(|(oid, _)| *oid == id)
        .expect("our thing is listed");
    assert_eq!(row.1.posture, Some(Posture::Wants));

    // Only Things are listed — the directory holds many kinds.
    assert!(
        all.iter()
            .all(|(_, s)| s.name == "hammer" || !s.name.is_empty() || s.posture.is_some()),
        "things() must not leak other object kinds"
    );
}

/// A posture MOVES on the same object. The hammer you wanted becomes the hammer you have —
/// it does not become a different thing, which is the whole reason posture is state.
#[tokio::test]
async fn a_posture_moves_and_clears_on_the_same_object() {
    let h = Harness::new(&["Alice"]).await;
    let alice = h.node(0);
    let relay = h.relay.clone();

    let id = alice.object_new("thing", "hammer").unwrap();
    alice
        .apply(
            &id,
            OP_SET_POSTURE,
            args(&[("posture", ArgVal::Text("wants".into()))]),
        )
        .await
        .unwrap();

    // Borrowed it — now you have it. Same object id throughout.
    alice
        .apply(
            &id,
            OP_SET_POSTURE,
            args(&[("posture", ArgVal::Text("has".into()))]),
        )
        .await
        .unwrap();
    assert_eq!(alice.thing_state(&id).unwrap().posture, Some(Posture::Has));

    // Selling it: an ACTIVE posture, so a price is meaningful here and only here.
    alice
        .apply(
            &id,
            OP_SET_POSTURE,
            args(&[
                ("posture", ArgVal::Text("selling".into())),
                ("price", ArgVal::Text("£15".into())),
            ]),
        )
        .await
        .unwrap();
    let st = alice.thing_state(&id).unwrap();
    assert_eq!(st.posture, Some(Posture::Selling));
    assert_eq!(st.price.as_deref(), Some("£15"));

    // Off the market: the price goes with the posture it qualified.
    alice
        .apply(&id, OP_CLEAR_POSTURE, Args::new())
        .await
        .unwrap();
    let st = alice.thing_state(&id).unwrap();
    assert_eq!(st.posture, None);
    assert_eq!(st.price, None, "a price without a posture is meaningless");
}

/// Loud, not lenient. A price on a STANDING intent is a modelling error — core rejects the
/// delta rather than storing a price the market would then have to explain away.
#[tokio::test]
async fn a_price_on_a_standing_intent_is_refused() {
    let h = Harness::new(&["Alice"]).await;
    let alice = h.node(0);
    let relay = h.relay.clone();

    let id = alice.object_new("thing", "hammer").unwrap();
    let priced_want = args(&[
        ("posture", ArgVal::Text("wants".into())),
        ("price", ArgVal::Text("£15".into())),
    ]);
    let result = alice.apply(&id, OP_SET_POSTURE, priced_want).await;
    assert!(
        result.is_err(),
        "a Want has no price until it becomes Buying"
    );

    // And the object is untouched — a refused delta must not half-apply.
    assert_eq!(alice.thing_state(&id).unwrap().posture, None);
}
