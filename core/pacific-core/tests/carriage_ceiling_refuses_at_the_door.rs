//! THE CARRIAGE CEILING — a Delta too big to be carried is refused at the door, and
//! nothing is written.
//!
//! WHAT THIS BINARY IS ABOUT. `group.setCover` accepts 700,000 base64 characters AT THE
//! FOLD (`group::MAX_COVER_STILL_B64`). A relay accepts 262,144 bytes in one publish
//! (`relay::limits::Limits::default().max_blob_bytes`). Between those two numbers is a
//! band in which this device writes a Delta locally, the relay refuses it, and the
//! author is told `Ok`: measured on two devices over a real relay, a 150,000-character
//! cover arrived and a 180,000-character one did not (envelope 180,091 → MLS 196,704 →
//! sealed 196,744 → base64 262,328, which is 184 over). The peer kept the stale cover
//! and was told nothing — and then the next SEQUENCED op that did fit named a
//! predecessor that peer never received, so the object stopped folding there at all.
//! `ChainBroken`: one oversize cover plus one profile edit, and the group is dead on
//! every other device.
//!
//! So the door refuses first. `node::MAX_DELTA_ENVELOPE_BYTES` is the ceiling and
//! `Node::append_and_flush` is the one place it is enforced — before the append, which
//! is the property these tests are really about. The fold's caps are UNCHANGED: they are
//! wire values, and lowering one would make this build refuse deltas older builds
//! legitimately produced.

mod common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::group::{OP_SET_COVER, OP_SET_PROFILE};
use pacific_core::node::{MAX_DELTA_ENVELOPE_BYTES, RELAY_DEFAULT_MAX_BLOB_B64};

/// A `group.setCover` carrying `n` base64 characters of (nonsense) JPEG.
fn cover_of(n: usize) -> Args {
    let mut a = Args::new();
    a.insert("data".into(), ArgVal::Text("A".repeat(n)));
    a.insert("mime".into(), ArgVal::Text("image/jpeg".into()));
    a
}

/// The largest cover this door will take. The envelope is the payload plus the CBOR
/// framing around it (the keys, the mime, `prev`, the epoch and the seq) — a hundred-odd
/// bytes — so the ceiling MINUS a kilobyte is "at the ceiling" with the framing paid for.
/// If the framing ever grew past 1 KiB this test would fail loudly rather than quietly
/// testing something smaller, which is the point of naming the allowance.
const AT_THE_CEILING: usize = MAX_DELTA_ENVELOPE_BYTES - 1_024;

/// THE PIN. Core cannot read the relay's configuration — `max_blob_bytes` is read from
/// `RELAY_MAX_BLOB_BYTES` at the relay's own boot and an operator may raise it — so
/// `RELAY_DEFAULT_MAX_BLOB_B64` states the conservative DEFAULT and this holds the two
/// together. It is the relay's own crate answering, not a number copied out of its
/// source: if that default ever moves, the ceiling derived from it is wrong and this
/// fails rather than drifting.
#[test]
fn the_ceiling_is_derived_from_the_relays_own_default() {
    assert_eq!(
        relay::limits::Limits::default().max_blob_bytes,
        RELAY_DEFAULT_MAX_BLOB_B64,
        "core refuses at the relay's conservative default; the relay has moved it"
    );
    // The arithmetic in the doc comment, asserted rather than described: the sealed
    // budget is what base64s to exactly the cap, and the ceiling leaves MLS a quarter.
    assert_eq!(MAX_DELTA_ENVELOPE_BYTES, 157_286);
    assert!(
        MAX_DELTA_ENVELOPE_BYTES < pacific_core::group::MAX_COVER_STILL_B64,
        "the door is meant to be STRICTER than the fold — that is the whole change"
    );
    // A Host's picture cap is a chain-safety rule, not a fold cap: a picture at the cap
    // must fit one envelope, with a kilobyte for the rest of `host.setMedia`.
    assert!(
        pacific_core::host::MAX_MEDIA_B64 + 1_024 <= MAX_DELTA_ENVELOPE_BYTES,
        "a Host picture at its own cap would be refused at the door"
    );
}

/// The defect itself: a cover inside the fold's cap and outside the relay's is refused
/// at the door, the refusal says what was too big and by how much, and the device is
/// byte-for-byte where it was — no delta in the log, no outbox entry, nothing to strand.
#[tokio::test]
async fn an_oversize_cover_is_refused_and_nothing_is_written() {
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let obj = h.mint_group(ada);
    h.add_to_forum(ada, bo, &obj).await;
    h.settle().await;

    let before = h.node(ada).object_log(&obj).unwrap();
    let cover_before = h.node(ada).group_view(&obj).unwrap().cover;

    // 180,000 characters: comfortably inside `MAX_COVER_STILL_B64` (700,000), and the
    // exact size measured NOT to reach a second device.
    let o = obj.clone();
    let e = h
        .with(ada, |n| async move { n.apply(&o, OP_SET_COVER, cover_of(180_000)).await })
        .await
        .expect_err("a cover the relay cannot carry must be refused at the door");

    let said = e.to_string();
    for want in [
        "too large",
        "group.setCover",
        &MAX_DELTA_ENVELOPE_BYTES.to_string(),
        &RELAY_DEFAULT_MAX_BLOB_B64.to_string(),
        "NOTHING WAS WRITTEN",
    ] {
        assert!(said.contains(want), "the refusal must name {want:?}: {said}");
    }
    // AND THE SIZE OF THE THING THAT WAS TOO BIG. "MalformedArgs" is what this used to
    // be worth; a person cannot act on that, so the actual envelope length has to be in
    // there — the payload was 180,000 characters and the envelope is a little over it.
    let numbers: Vec<usize> = said
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|s| s.parse().ok())
        .collect();
    assert!(
        numbers.iter().any(|&n| (180_000..181_000).contains(&n)),
        "the refusal must say how big the delta actually was: {said}"
    );

    assert_eq!(
        h.node(ada).object_log(&obj).unwrap(),
        before,
        "a refused delta leaves no row in the log — not even an undelivered one"
    );
    assert_eq!(
        h.node(ada).group_view(&obj).unwrap().cover,
        cover_before,
        "and no local state change"
    );

    // And the object is still whole on both devices after a sync.
    h.settle().await;
    for u in [ada, bo] {
        let bad = h.node(u).noncompliant_objects().unwrap();
        assert!(bad.is_empty(), "{} holds {bad:?} it cannot fold", h.device_name(u));
        h.node(u).object_compliance(&obj).unwrap();
    }
}

/// The other side of the door: a delta AT the ceiling is not merely accepted locally, it
/// actually arrives. Without this the fix could be a ceiling of zero.
#[tokio::test]
async fn a_cover_at_the_ceiling_still_reaches_the_second_device() {
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let obj = h.mint_group(ada);
    h.add_to_forum(ada, bo, &obj).await;
    h.settle().await;

    let o = obj.clone();
    h.with(ada, |n| async move { n.apply(&o, OP_SET_COVER, cover_of(AT_THE_CEILING)).await })
        .await
        .expect("a delta at the ceiling is exactly the case that must still work");
    h.settle().await;

    for u in [ada, bo] {
        let v = h.node(u).group_view(&obj).unwrap();
        assert_eq!(
            v.cover.len(),
            AT_THE_CEILING,
            "{} holds the whole cover, over a real relay",
            h.device_name(u)
        );
        assert_eq!(v.cover_mime, "image/jpeg", "{}", h.device_name(u));
    }
}

/// THE SCENARIO THAT COST THE GROUP. Oversize cover, then an ordinary profile edit that
/// does fit. Before the door, the cover was written and stranded, the profile edit named
/// it as `prev`, and every other device answered `ChainBroken` for ever after. Now the
/// cover never enters the log, the profile edit chains onto the delta before it, and the
/// peer folds.
#[tokio::test]
async fn the_chain_cannot_be_broken_through_the_doors() {
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let obj = h.mint_group(ada);
    h.add_to_forum(ada, bo, &obj).await;
    h.settle().await;

    let o = obj.clone();
    h.with(ada, |n| async move { n.apply(&o, OP_SET_COVER, cover_of(180_000)).await })
        .await
        .expect_err("refused at the door");

    let o = obj.clone();
    let mut p = Args::new();
    p.insert("displayName".into(), ArgVal::Text("Stoma".into()));
    p.insert("shape".into(), ArgVal::Text("organisation".into()));
    h.with(ada, |n| async move { n.apply(&o, OP_SET_PROFILE, p).await })
        .await
        .expect("the edit after a refusal is an ordinary write");
    h.settle().await;

    for u in [ada, bo] {
        let who = h.device_name(u);
        h.node(u)
            .object_compliance(&obj)
            .unwrap_or_else(|e| panic!("{who} can no longer fold the object: {e}"));
        assert_eq!(
            h.node(u).group_view(&obj).unwrap().display_name,
            "Stoma",
            "{who} folded the edit that followed the refusal"
        );
        assert_eq!(
            h.node(u).group_view(&obj).unwrap().cover,
            "",
            "{who} never saw a cover, because none was ever written"
        );
    }
}
