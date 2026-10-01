//! m27 — a minted GroupObject cannot be got back from the seed, and says so.
//!
//! THE PROMISE THIS DOES NOT LET US MAKE. `backup.rs` put it best before it was
//! deleted: "the words are the backup" is true of an account that has a history
//! stored, "and a surface that says it to someone who has none is lying to them".
//! The boot flow shows a person 24 words and calls them a recovery key. This test
//! is what stops that becoming a lie without anyone noticing.
//!
//! WHAT A RECOVERABLE OBJECT NEEDS. The first is written now; the other two
//! are not:
//!
//!   - a SPINE entry, or a recovering device cannot NAME the object. Every
//!     address below derives from the group id, so without this there is nothing
//!     to derive from. WRITTEN: at mint, join, connection and departure (m29), and
//!     published by the drain in `sync_once` (m28). Its `first_epoch` is also the
//!     account's floor, and `unreadable_epochs` starts there.
//!   - an ARCHIVE KEY RECORD per epoch, at `locator::archive_locator`, or that
//!     epoch's content is unreadable for ever. The exporter answers only while
//!     the group is AT that epoch (RFC 9420 §8.5), so a key not taken then is not
//!     takeable later.
//!   - MLS STATE the seed can reach, or the object returns READ-ONLY. mls-rs
//!     rebuilds a live group from `GroupStateStorage`; this build keeps it in
//!     `pacific.db` under the device key, which a device that lost the db lost.
//!
//! WHY THIS TEST PASSES RATHER THAN FAILS. It asserts the truth, which is that
//! the migration is not done. A red test would be deleted or ignored; a green one
//! that pins a known gap is a gate — when the writers land, the assertions below
//! stop holding and this file has to be rewritten to say the opposite. That is
//! the point. `docs/archive-by-arithmetic.md` is the specification it closes on.
//!
//! The shape is `noncompliant_objects`': a sweep whose EMPTY result is the claim,
//! and whose non-empty result names what is missing rather than counting it.

mod common;

use common::Harness;

/// A freshly minted object: named locally, recoverable not at all.
#[tokio::test]
async fn a_minted_object_is_unrecoverable_and_names_all_three_gaps() {
    let h = Harness::new(&["Ana"]).await;
    let n = h.node(0);

    let object = n.object_new("forum", "thursday").expect("mint a forum");
    let r = n.recoverability(&object).expect("recoverability is answerable");

    assert_eq!(r.object_id, object);
    assert!(
        !r.recoverable(),
        "a minted object must not claim to be recoverable while nothing writes \
         the three artefacts: {r:?}"
    );

    // Each gap named separately, because each is a different loss.
    assert!(
        !r.named,
        "queued is not published: until the drain runs, the seed cannot name this object"
    );
    assert!(
        !r.speakable,
        "MLS state is in pacific.db under the device key — a seed on another \
         device cannot reach it, so this object would come back read-only"
    );
    assert!(
        !r.unreadable_epochs.is_empty(),
        "epoch 0 has no archive key record, so what is said in it is unreadable \
         from a standing start"
    );
    assert!(
        r.unreadable_epochs.contains(&0),
        "epoch 0 is the one RFC 9420 §11 makes unavoidable — a group is created \
         there with a fresh random epoch secret and NO COMMIT, so there is no way \
         to record it on the way out: {:?}",
        r.unreadable_epochs
    );
    // AND THE FLOOR, said explicitly. The unreadable list starts at the account's
    // floor, not at 0. A mint's floor IS 0 — the creator was there from the first
    // epoch — so epoch 0 is listed here and is a real loss. A member who joined
    // later is not charged for the epochs before it arrived; m29 pins that.
    assert_eq!(r.first_epoch, Some(0), "the mint's entry puts the floor at epoch 0");
}

/// The sweep. Empty is the claim; it is not empty.
#[tokio::test]
async fn the_sweep_names_every_object_the_words_would_not_bring_back() {
    let h = Harness::new(&["Ben"]).await;
    let n = h.node(0);

    let forum = n.object_new("forum", "one").expect("mint a forum");
    let thing = n.object_new("thing", "two").expect("mint a thing");

    let lost = n.unrecoverable_objects().expect("the sweep answers");

    let ids: Vec<&str> = lost.iter().map(|r| r.object_id.as_str()).collect();
    assert!(ids.contains(&forum.as_str()), "the forum is not recoverable and must be listed: {ids:?}");
    assert!(ids.contains(&thing.as_str()), "the thing is not recoverable and must be listed: {ids:?}");

    // THE ASSERTION THE MIGRATION INVERTS. When the spine, the archive records
    // and the MLS state are written at mint, this list empties and the two
    // assertions above stop holding. Rewrite this file then; do not weaken it.
    assert!(
        !lost.is_empty(),
        "the sweep came back empty, which would mean every object on this device \
         can be got back from its 24 words. If that is now TRUE the migration has \
         landed and this test should be inverted rather than deleted — see \
         docs/archive-by-arithmetic.md"
    );
}

/// The writer path: minting queues the entry, and it is the shared format.
///
/// `object_new` produces the spine entry through `spine::place` — the same core
/// import iOS and the browser both call — and queues it against the address the
/// storage root derives. Publishing it is the drain's job, in `sync_once` (m28).
/// This test never syncs, so the entry stays in this device's own database, which
/// is precisely the thing a recovering device does not have — and
/// `recoverability` says so by CHECKING the delivered rows rather than by
/// declaring it.
#[tokio::test]
async fn minting_queues_a_spine_entry_in_the_shared_format() {
    let h = Harness::new(&["Cass"]).await;
    let n = h.node(0);

    // The account's own record already holds a vertebra (m30) — it is minted with
    // the identity and takes index 0 — so this counts the DELTA, not the total.
    let before = n.pending_spine_entries().expect("the queue answers");

    let object = n.object_new("forum", "friday").expect("mint a forum");
    let gid = hex::decode(&object).unwrap();

    let queued = n.pending_spine_entries().expect("the queue answers");
    assert_eq!(queued.len(), before.len() + 1, "one mint, one entry");

    let (index, entry) = queued
        .iter()
        .find(|(_, e)| e.body.group_id() == gid.as_slice())
        .expect("an entry names the object that was just minted");
    assert_eq!(
        *index as usize,
        before.len(),
        "it takes the next index after the ones already owed"
    );
    assert_eq!(
        entry.body.group_id(),
        gid.as_slice(),
        "the entry names the object that was just minted"
    );
    match &entry.body {
        pacific_core::spine::Body::Group(j) => assert_eq!(
            j.first_epoch, 0,
            "a created group starts at epoch 0 — RFC 9420 §11, no commit behind it"
        ),
        other => panic!("a mint writes a Group entry, got {other:?}"),
    }

    // A second object takes the next index; the first keeps its own.
    let second = n.object_new("thing", "kettle").expect("mint a thing");
    let queued = n.pending_spine_entries().unwrap();
    assert_eq!(queued.len(), before.len() + 2);
    let last = queued.last().expect("the queue is not empty");
    assert_eq!(
        last.0 as usize,
        before.len() + 1,
        "indices are claimed in order"
    );
    assert_eq!(
        last.1.body.group_id(),
        hex::decode(&second).unwrap().as_slice()
    );

    // AND IT IS STILL NOT RECOVERABLE, because queued is not published. The
    // check reads delivered rows, of which there are none.
    let r = n.recoverability(&object).unwrap();
    assert!(!r.named, "a queued entry is not a published one");
    assert!(!r.recoverable());
}
