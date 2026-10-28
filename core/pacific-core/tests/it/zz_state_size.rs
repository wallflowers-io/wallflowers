//! HOW BIG IS THE MLS STATE — measurement, not a test of behaviour.
//!
//! Answers the question the head-record design turns on: if a resuming device is
//! handed the CURRENT MLS state instead of walking forward from an anchor, how
//! many bytes is that, and does it scale with membership?
//!
//! Measured over the in-memory storage pair (`mls_mem`) because that is the
//! browser's, it needs no relay, and `Snapshot` is exactly the artefact a head
//! record would carry. The SQLite pair stores the same blobs.
//!
//! Scratch file — delete once the number is recorded.

use pacific_core::mls;
use pacific_core::mls_mem::{build_client_mem, MemGroupStateStorage, MemKeyPackageStorage, Snapshot};

struct Device {
    gss: MemGroupStateStorage,
    kps: MemKeyPackageStorage,
    client: pacific_core::mls_mem::MemClient,
}

fn device(seed: u8) -> Device {
    let crypto = mls::crypto();
    let (sk, pk) = mls::generate_signing_key(&crypto).unwrap();
    let cred = [seed; 32];
    let sid = mls::signing_identity(&cred, pk.as_bytes());
    let gss = MemGroupStateStorage::new();
    let kps = MemKeyPackageStorage::new();
    let client = build_client_mem(gss.clone(), kps.clone(), sid, sk).unwrap();
    Device { gss, kps, client }
}

/// Sizes for one owner holding a group of `n` members, after `extra` further
/// epoch transitions. Returns (group_state_bytes, epoch_count, epoch_bytes, kp_bytes).
fn measure(n: usize, extra: usize) -> (usize, usize, usize, usize) {
    let owner = device(1);
    let mut g = mls::create_group_named(&owner.client, "stoma").unwrap();

    // Add n-1 members. Each add is its own commit, so the group lands at epoch n-1.
    for i in 1..n {
        let m = device((i + 1) as u8);
        let kp = mls::make_key_package_bytes(&m.client).unwrap();
        let (_commit, _welcome) = mls::add_member(&mut g, &kp).unwrap();
    }

    // Further epochs with no membership change, to separate "cost of members"
    // from "cost of epochs".
    for i in 0..extra {
        let m = device((200 + i) as u8);
        let kp = mls::make_key_package_bytes(&m.client).unwrap();
        let (_c, _w) = mls::add_member(&mut g, &kp).unwrap();
    }

    drop(g);
    let snap = Snapshot::capture(&owner.gss, &owner.kps).unwrap();
    let state_bytes: usize = snap.states.iter().map(|(_, b)| b.len()).sum();
    let epoch_bytes: usize = snap.epochs.iter().map(|(_, _, b)| b.len()).sum();
    let kp_bytes: usize = snap.key_packages.iter().map(|(_, b)| b.len()).sum();
    (state_bytes, snap.epochs.len(), epoch_bytes, kp_bytes)
}

#[test]
fn how_big_is_the_current_mls_state() {
    println!("\n  members   GroupState    epochs   epoch bytes   per-epoch   keypkgs      TOTAL");
    println!("  ------------------------------------------------------------------------------");
    for n in [2usize, 5, 10, 20, 50] {
        let (state, ecount, ebytes, kp) = measure(n, 0);
        let per = if ecount > 0 { ebytes / ecount } else { 0 };
        println!(
            "  {:>7}   {:>9} B   {:>7}   {:>9} B   {:>7} B   {:>7} B   {:>8} B",
            n,
            state,
            ecount,
            ebytes,
            per,
            kp,
            state + ebytes + kp
        );
    }

    // Does a retained epoch grow with membership, or is it flat? This is the
    // figure `group-crypto.md` put at ~2,500 B against OpenMLS's storage map.
    println!("\n  epoch growth at fixed membership (5 members, extra epochs):");
    for extra in [0usize, 3, 10] {
        let (state, ecount, ebytes, _kp) = measure(5, extra);
        let per = if ecount > 0 { ebytes / ecount } else { 0 };
        println!(
            "    +{:>2} epochs -> GroupState {:>7} B · {:>2} epoch records · {:>8} B total · {:>6} B each",
            extra, state, ecount, ebytes, per
        );
    }
    println!();
}
