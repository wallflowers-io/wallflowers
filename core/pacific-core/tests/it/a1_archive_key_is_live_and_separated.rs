//! a1 — the archive content key exists, answers, and is nobody else's key.
//!
//! `keys.md` §4 lists three exporter outputs and marks the third *planned*:
//! `pacific/archive/v1`, "the content key for that epoch's R2 blobs", retained
//! where the other two are pruned at `EPOCH_RETENTION`. This is the test that it
//! is no longer planned.
//!
//! WHAT IT PINS, and each line is a property the archive rests on:
//!
//!   - the exporter ANSWERS, so a device can take a key for the epoch it is at;
//!   - it is SEPARATED from the two live exporters at the same epoch, so holding
//!     a relay tag or a transport seal does not hand anyone the content;
//!   - it MOVES with the epoch, which is both the reason a removed member cannot
//!     read forward and the reason a key not written down while the group is at
//!     that epoch is gone for good (RFC 9420 §8.5).
//!
//! WHAT IT DOES NOT PIN. Nothing writes this key down yet, so nothing recovers
//! anything with it — `m27` reports exactly that, per object and per epoch. This
//! file says the derivation is real; it does not say the archive is built.

use pacific_core::mls::{self};
use pacific_core::mls_mem::{
    build_client_mem, MemGroupStateStorage, MemKeyPackageStorage,
};

/// One member, one group, at epoch 0 — which RFC 9420 §11 makes a group with a
/// fully initialised key schedule and NO COMMIT behind it.
fn a_group_of_one() -> pacific_core::mls_mem::MemGroup {
    // The provider's own generator, as `b1` and `node.rs` use it. Not raw dalek
    // bytes: for Ed25519 the provider's secret is the 64-byte keypair encoding,
    // and handing it a 32-byte seed builds a client that then cannot sign.
    let crypto = mls::crypto();
    let (sk, pk) = mls::generate_signing_key(&crypto).unwrap();
    let sid = mls::signing_identity(&[0x31u8; 32], pk.as_bytes());
    let client = build_client_mem(
        MemGroupStateStorage::new(),
        MemKeyPackageStorage::new(),
        sid,
        sk,
    )
    .unwrap();
    mls::create_group_named(&client, "thursday").unwrap()
}

#[test]
fn the_archive_key_answers_at_epoch_zero() {
    let group = a_group_of_one();
    assert_eq!(group.current_epoch(), 0, "a created group starts at epoch 0");

    let eb = mls::epoch_be(group.current_epoch());
    let k = mls::archive_key(&group, &eb).expect("epoch 0 has a working exporter");
    assert_eq!(k.len(), 32);
    assert_ne!(k, [0u8; 32], "an all-zero key is not an exporter output");

    // Deterministic: two calls at one epoch are the same key, or a member and the
    // member beside them would archive under different keys.
    let again = mls::archive_key(&group, &eb).unwrap();
    assert_eq!(k, again, "the exporter is a function of the epoch, not of the call");
}

#[test]
fn it_is_not_the_relay_tag_and_not_the_transport_seal() {
    let group = a_group_of_one();
    let eb = mls::epoch_be(group.current_epoch());

    let archive = mls::archive_key(&group, &eb).unwrap();
    let seal = mls::seal_conn_secret(&group, &eb).unwrap();
    let tag = mls::group_tag(&group, &eb).unwrap();

    // THE SPLIT THAT MAKES RETENTION SAFE. The relay tag and the transport seal
    // are pruned at EPOCH_RETENTION; the archive key is kept for ever. If any two
    // of them converged, keeping the durable one would quietly keep the expiring
    // ones too, and the departure from §9.2 would be wider than it is declared to
    // be.
    assert_ne!(archive, seal, "the archive key is not the transport seal");
    assert_ne!(archive, tag, "the archive key is not the relay tag");
    assert_ne!(seal, tag, "and those two were already distinct");
}

#[test]
fn it_moves_with_the_epoch_and_the_old_one_cannot_be_asked_for_again() {
    let mut group = a_group_of_one();
    let at_zero = mls::archive_key(&group, &mls::epoch_be(0)).unwrap();

    // An empty commit: the cheapest way to leave epoch 0.
    let out = group.commit_builder().build().expect("stage a commit");
    group.apply_pending_commit().expect("apply it");
    assert_eq!(group.current_epoch(), 1);
    let _ = out;

    let at_one = mls::archive_key(&group, &mls::epoch_be(1)).unwrap();
    assert_ne!(
        at_zero, at_one,
        "a new epoch must derive a new content key — this is why a removed member \
         cannot read forward"
    );

    // AND THE POINT OF RECORDING ON ENTRY. The exporter belongs to the epoch the
    // group is AT. Asking it for epoch 0's context once the group has moved does
    // not return epoch 0's key: the answer comes from the current epoch's
    // exporter_secret, so a key not written down while the group was there is not
    // recoverable by asking again.
    let asked_late = mls::archive_key(&group, &mls::epoch_be(0)).unwrap();
    assert_ne!(
        asked_late, at_zero,
        "asking the new epoch's exporter for the old epoch's context must NOT \
         reproduce the old key — if it did, record-on-entry would be optional"
    );
}
