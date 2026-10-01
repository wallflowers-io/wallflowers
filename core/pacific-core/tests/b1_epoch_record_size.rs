//! **B1 — MEASURE THE EPOCH RECORD.** `docs/the-build.html` item B1, gate G5.
//!
//! THROWAWAY. A measuring instrument, not a behaviour test: it asserts only the
//! handful of facts the economics argument rests on and prints everything else.
//! Delete it once the figures are written down.
//!
//! THE QUESTION. `~2,500 B` is OpenMLS's whole provider storage map under
//! `PastEpochDeletionPolicy::KeepAll`, measured in a different library. On
//! mls-rs the storage trait is `write(state, epoch_inserts, epoch_updates)` with
//! NO delete instruction (`vendor/mls-rs/src/group/state_repo.rs:194`), so the
//! retention unit is ours to pick, and the unit is one `EpochRecord` —
//! `EpochRecord { id, data }` where `data` is a `PriorEpoch`, mls-encoded
//! (`state_repo.rs:201`). `PriorEpoch` (`vendor/mls-rs/src/group/epoch.rs:28`):
//!
//!     context: GroupContext                      -- ~fixed
//!     self_index: LeafIndex                      -- 4 B
//!     secrets: EpochSecrets                      -- resumption + sender-data +
//!                                                   the SECRET TREE  (lazy; §1b)
//!     signature_public_keys: Vec<Option<..>>     -- ONE SLOT PER LEAF
//!     membership_key                             -- 32 B (feature-gated)
//!
//! Two of those five are sized by the ratchet tree, so the expectation is that
//! it DOES scale with membership. §1 measures the slope; §1b measures the part
//! that is NOT a function of membership alone.
//!
//! WHY NOT THE RELAY HARNESS. `tests/common/mod.rs` drives one `Node` at a time
//! through a real relay, which is the right instrument for convergence and the
//! wrong one for a size sweep at 50 leaves. This forms the group through the
//! SAME `mls::` calls `node.rs` uses (`create_group_named`, `add_member`,
//! `stage_rename` + `apply_staged`, `encrypt_delta`, `decrypt_message`) — the
//! `m1_nmember` style — so the records measured are the records the node
//! writes. §3 then re-measures on the REAL native store,
//! `SqliteGroupStateStorage`, pruning and all.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use pacific_core::mls::{self, Incoming};
use pacific_core::mls_mem::{
    build_client_mem, MemClient, MemGroup, MemGroupStateStorage, MemKeyPackageStorage, Snapshot,
};
use pacific_core::seal;
use sha2::{Digest, Sha256};

/// Rename commits pumped after the group is at full membership, so the last
/// records measured are steady-state records at N members rather than records
/// laid down while the tree was still growing.
const PUMP: usize = 6;

/// One device: its own signing key and its own pair of in-memory stores, which
/// is what a browser leaf holds and what `mls_mem` exists to be. The in-memory
/// store NEVER PRUNES (it holds zero references to `EPOCH_RETENTION` — that is
/// build item A2's complaint), which is exactly what a measurement wants: it
/// keeps every epoch record ever written.
struct Device {
    gss: MemGroupStateStorage,
    kps: MemKeyPackageStorage,
    client: MemClient,
    id: [u8; 32],
}

fn device(seed: u16) -> Device {
    let crypto = mls::crypto();
    let (sk, pk) = mls::generate_signing_key(&crypto).unwrap();
    let mut id = [0u8; 32];
    id[0] = (seed & 0xff) as u8;
    id[1] = (seed >> 8) as u8;
    let sid = mls::signing_identity(&id, pk.as_bytes());
    let gss = MemGroupStateStorage::new();
    let kps = MemKeyPackageStorage::new();
    let client = build_client_mem(
        gss.clone(),
        kps.clone(),
        sid,
        mls::SecretKey::new(sk.as_bytes().to_vec()),
    )
    .unwrap();
    Device {
        gss,
        kps,
        client,
        id,
    }
}

/// Every epoch record one device holds for one group: `(epoch_id, bytes)`,
/// ordered, plus the size of the single current-epoch `GroupState` blob — which
/// is NOT per-epoch: one row per group, overwritten on every commit.
struct Held {
    state_len: usize,
    rows: Vec<(u64, usize)>,
}

fn held(d: &Device, gid: &[u8]) -> Held {
    let snap = Snapshot::capture(&d.gss, &d.kps).unwrap();
    let mut rows: Vec<(u64, usize)> = snap
        .epochs
        .iter()
        .filter(|(g, _, _)| g == gid)
        .map(|(_, e, blob)| (*e, blob.len()))
        .collect();
    rows.sort_by_key(|(e, _)| *e);
    let state_len = snap
        .states
        .iter()
        .find(|(g, _)| g == gid)
        .map(|(_, b)| b.len())
        .unwrap_or(0);
    Held { state_len, rows }
}

fn size_at(h: &Held, epoch: u64) -> usize {
    h.rows
        .iter()
        .find(|(e, _)| *e == epoch)
        .unwrap_or_else(|| panic!("no epoch record {epoch}"))
        .1
}

/// Form a real N-member group: the owner mints it and adds each member in turn,
/// and every member already in folds every commit — a real group, not N forks
/// of one.
fn build(n: usize) -> (Vec<Device>, Vec<MemGroup>) {
    assert!(n >= 2);
    let devs: Vec<Device> = (0..n).map(|i| device(i as u16)).collect();
    let mut groups: Vec<MemGroup> = Vec::with_capacity(n);
    groups.push(mls::create_group_named(&devs[0].client, "b1").unwrap());

    for j in 1..n {
        let kp = mls::make_key_package_bytes(&devs[j].client).unwrap();
        let welcome = {
            let (head, tail) = groups.split_at_mut(1);
            let (commit, welcome) = mls::add_member(&mut head[0], &kp).unwrap();
            for g in tail.iter_mut() {
                assert!(
                    matches!(
                        mls::decrypt_message(g, &commit).unwrap(),
                        Incoming::Commit { .. }
                    ),
                    "an existing member folds the add commit"
                );
            }
            welcome
        };
        groups.push(mls::join_group(&devs[j].client, &welcome).unwrap());
    }

    let mut roster = mls::roster_identities(&groups[0]).unwrap();
    roster.sort();
    let mut expect: Vec<[u8; 32]> = devs.iter().map(|d| d.id).collect();
    expect.sort();
    assert_eq!(roster, expect, "the roster really is exactly these N leaves");
    (devs, groups)
}

/// One steady-state epoch: a GroupContextExtensions (rename) commit, the
/// cheapest epoch the product actually produces, folded by every member.
fn pump_one(groups: &mut [MemGroup], label: &str) {
    let (head, tail) = groups.split_at_mut(1);
    let commit = mls::stage_rename(&mut head[0], label).unwrap();
    mls::apply_staged(&mut head[0]).unwrap();
    for g in tail.iter_mut() {
        assert!(matches!(
            mls::decrypt_message(g, &commit).unwrap(),
            Incoming::Commit { .. }
        ));
    }
}

fn mean(rows: &[(u64, usize)]) -> f64 {
    rows.iter().map(|(_, b)| *b as f64).sum::<f64>() / rows.len() as f64
}

/// A convergent (deterministic-nonce) seal: the nonce is derived from the
/// PLAINTEXT, so identical plaintext under an identical key gives identical
/// bytes. Nothing in the tree does this — every seal here (`seal::seal`,
/// `atrest::seal_at_rest`, `backup::seal`, `wrap::seal`) draws a RANDOM nonce.
/// It exists in this test only to separate "dedup fails because the nonce is
/// random" from "dedup fails because the key changed at the epoch boundary".
fn convergent(plain: &[u8], key: &[u8; 32]) -> Vec<u8> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let h = Sha256::digest(plain);
    let nonce = XNonce::from_slice(&h[..24]);
    cipher
        .encrypt(
            nonce,
            Payload {
                msg: plain,
                aad: b"pacific/archive/v1",
            },
        )
        .unwrap()
}

/// An R2 object key is the sha256 of the bytes stored (`the-harness.html` §02,
/// "key = sha256 hex, 64 chars"), so dedup is byte-identity and nothing else.
fn r2_key(blob: &[u8]) -> String {
    hex::encode(Sha256::digest(blob))
}

#[test]
fn b1_epoch_record_size_against_membership() {
    println!("\n============ B1 · THE RETAINED MLS EPOCH RECORD (mls-rs 0.55.4) ================");
    println!(
        "unit measured: one EpochRecord.data = a PriorEpoch, mls-encoded — exactly the bytes\n\
         GroupStateStorage::write() is handed, and exactly the bytes mls_epoch.data holds.\n"
    );

    // ---------------------------------------------------------------- §1 ----
    // The sweep. 2, 5 and 20 are the figures B1 asks for; 3, 10 and 50 are
    // there so the shape of the curve is not inferred from three points.
    let sizes = [2usize, 3, 5, 10, 20, 50];
    let mut summary: Vec<(usize, f64, usize, usize)> = Vec::new();
    let mut twenty: Option<Held> = None;

    for &n in &sizes {
        let (devs, mut groups) = build(n);
        for p in 0..PUMP {
            pump_one(&mut groups, &format!("b1-{p}"));
        }
        let gid = groups[0].group_id().to_vec();
        let epoch = groups[0].current_epoch();
        for g in &groups {
            assert_eq!(g.current_epoch(), epoch, "every member is at one epoch");
        }
        let owner = held(&devs[0], &gid);
        let late = held(&devs[n - 1], &gid);

        let steady = &owner.rows[owner.rows.len() - PUMP..];
        println!(
            "---- N = {n} members, {} epochs held by the owner, current epoch {epoch}",
            owner.rows.len()
        );
        println!(
            "     epoch records (epoch:bytes): {}",
            owner
                .rows
                .iter()
                .map(|(e, b)| format!("{e}:{b}"))
                .collect::<Vec<_>>()
                .join("  ")
        );
        println!(
            "     steady state (the {PUMP} rename epochs at full membership): mean {:.1} B, last {} B",
            mean(steady),
            owner.rows.last().unwrap().1
        );
        println!(
            "     current GroupState blob (ONE row per group, not per epoch): {} B",
            owner.state_len
        );
        println!(
            "     the last joiner holds {} epoch record(s) for the same group: joining at epoch\n\
             \x20    {} it was never given the earlier ones — forward secrecy, not retention",
            late.rows.len(),
            epoch as usize - PUMP
        );
        summary.push((n, mean(steady), owner.state_len, owner.rows.last().unwrap().1));
        if n == 20 {
            twenty = Some(owner);
        }
    }

    println!("\n---- SUMMARY: per-epoch bytes against membership -------------------------------");
    println!("   N   steady-state epoch record   GroupState blob   B per extra member");
    let base = summary[0].1;
    for (n, m, state, _last) in &summary {
        let slope = if *n == 2 {
            String::from("     —")
        } else {
            format!("{:6.1}", (m - base) / ((*n - 2) as f64))
        };
        println!("{:>4}  {:>12.1} B              {:>8} B       {slope}", n, m, state);
    }
    let at2 = summary[0].1;
    let at20 = summary.iter().find(|s| s.0 == 20).unwrap().1;
    let at50 = summary.iter().find(|s| s.0 == 50).unwrap().1;
    let at5_last = summary.iter().find(|s| s.0 == 5).unwrap().3;
    let slope = (at50 - at2) / 48.0;
    println!(
        "     fit over the whole sweep: epoch record ≈ {:.0} + {:.0}·N bytes  (N = leaves)",
        at2 - 2.0 * slope,
        slope
    );
    assert!(
        at20 > at2,
        "the headline finding: the record DOES scale with membership"
    );

    // Growth INSIDE one group's own history: every add is an epoch, so the N=20
    // record list is itself a membership sweep with everything else held fixed
    // (same group id, same owner, same extensions, same device).
    if let Some(t) = &twenty {
        println!("\n---- the same thing inside ONE group (N=20): every add is an epoch ----------");
        let mut prev = 0usize;
        for (e, b) in &t.rows {
            let members = std::cmp::min(*e as usize + 1, 20);
            let delta = if prev == 0 { 0i64 } else { *b as i64 - prev as i64 };
            println!("     epoch {e:>2} ({members:>2} members): {b:>5} B   (+{delta})");
            prev = *b;
        }
    }

    // --------------------------------------------------------------- §1b ----
    // The part that is NOT a function of membership. `EpochSecrets.secret_tree`
    // is a SPARSE map (`TreeSecretsVec`, secret_tree.rs:88) filled in lazily as
    // sender ratchets are walked, so the cost of an epoch depends on HOW MANY
    // DISTINCT SENDERS this device read at it — not on the roster alone. Two
    // devices in the same 20-member group at the same epoch can hold different
    // numbers of bytes.
    println!("\n---- §1b the secret tree is LAZY: the same epoch costs more if you read more --");
    let n = 20usize;
    let (devs, mut groups) = build(n);
    pump_one(&mut groups, "quiet");
    pump_one(&mut groups, "before-traffic");
    let busy_epoch = groups[0].current_epoch();
    // The epoch immediately before the busy one: same membership, same kind of
    // commit, no application traffic read at it. The control.
    let quiet_epoch = busy_epoch - 1;

    // Every other member posts one application message at `busy_epoch`; the
    // owner decrypts all of them, walking 19 distinct sender ratchets.
    let mut deltas: Vec<(usize, Vec<u8>)> = Vec::new();
    for (j, g) in groups.iter_mut().enumerate().skip(1) {
        deltas.push((j, mls::encrypt_delta(g, b"a delta at the busy epoch").unwrap()));
    }
    for (j, d) in &deltas {
        match mls::decrypt_message(&mut groups[0], d).unwrap() {
            Incoming::Application { sender, .. } => {
                assert_eq!(sender, devs[*j].id, "the MLS-authenticated sender")
            }
            _ => panic!("expected an application message"),
        }
    }
    pump_one(&mut groups, "after-traffic");
    let gid = groups[0].group_id().to_vec();
    let owner = held(&devs[0], &gid);
    let quiet = size_at(&owner, quiet_epoch);
    let busy = size_at(&owner, busy_epoch);
    println!("     N = 20. epoch {quiet_epoch}, owner read 0 senders  : {quiet} B");
    println!(
        "     N = 20. epoch {busy_epoch}, owner read {} senders : {busy} B   (+{} B, {:.0} B per sender read)",
        deltas.len(),
        busy - quiet,
        (busy - quiet) as f64 / deltas.len() as f64
    );
    let reader = held(&devs[1], &gid);
    println!(
        "     …and a member that read NOTHING at that epoch holds {} B for it — same group,\n\
         \x20    same epoch, different bytes. Membership is the FLOOR, not the figure.",
        size_at(&reader, busy_epoch)
    );
    assert!(busy > quiet, "reading from senders inflates the epoch record");

    // ---------------------------------------------------------------- §2 ----
    println!("\n---- §2 what a retention policy costs ------------------------------------------");
    println!(
        "     EPOCH_RETENTION = {} (mls_store.rs prunes epoch_id < max - {}, so it keeps {} rows)",
        pacific_core::EPOCH_RETENTION,
        pacific_core::EPOCH_RETENTION,
        pacific_core::EPOCH_RETENTION + 1
    );
    let keep = (pacific_core::EPOCH_RETENTION + 1) as f64;
    println!(
        "     bounded window, per group:  {:.0} B at 2 members, {:.0} B at 20, {:.0} B at 50",
        at2 * keep,
        at20 * keep,
        at50 * keep
    );
    println!(
        "     KeepAll (what a chain implies), 1000 epochs: {:.2} MB at 2, {:.2} MB at 20, {:.2} MB at 50",
        at2 * 1000.0 / 1e6,
        at20 * 1000.0 / 1e6,
        at50 * 1000.0 / 1e6
    );
    println!("     at-rest sealing adds a flat 44 B/row (PxS1 magic 4 + nonce 24 + Poly1305 tag 16)");

    // ---------------------------------------------------------------- §3 ----
    // The same measurement on the REAL native store, with its pruning.
    std::env::remove_var("PACIFIC_ATREST_KEY");
    pacific_core::atrest::clear_device_key();
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("pacific.db");
    pacific_core::mls_store::migrate(&db).unwrap();
    let crypto = mls::crypto();
    let (sk, pk) = mls::generate_signing_key(&crypto).unwrap();
    let owner_id = [0xAAu8; 32];
    let sqlite_client = mls::build_client_sqlite(
        &db,
        mls::signing_identity(&owner_id, pk.as_bytes()),
        mls::SecretKey::new(sk.as_bytes().to_vec()),
    )
    .unwrap();
    let mut og = mls::create_group_named(&sqlite_client, "b1-sqlite").unwrap();
    let peers: Vec<Device> = (0..4).map(|i| device(1000 + i)).collect();
    let mut peer_groups: Vec<MemGroup> = Vec::new();
    for p in &peers {
        let kp = mls::make_key_package_bytes(&p.client).unwrap();
        let (commit, welcome) = mls::add_member(&mut og, &kp).unwrap();
        for g in peer_groups.iter_mut() {
            assert!(matches!(
                mls::decrypt_message(g, &commit).unwrap(),
                Incoming::Commit { .. }
            ));
        }
        peer_groups.push(mls::join_group(&p.client, &welcome).unwrap());
    }
    for p in 0..10 {
        let commit = mls::stage_rename(&mut og, &format!("sq-{p}")).unwrap();
        mls::apply_staged(&mut og).unwrap();
        for g in peer_groups.iter_mut() {
            assert!(matches!(
                mls::decrypt_message(g, &commit).unwrap(),
                Incoming::Commit { .. }
            ));
        }
    }
    let gid = og.group_id().to_vec();
    let epochs_written = og.current_epoch();
    drop(og);

    let conn = rusqlite::Connection::open(&db).unwrap();
    let mut st = conn
        .prepare("SELECT epoch_id, length(data) FROM mls_epoch WHERE group_id=?1 ORDER BY epoch_id")
        .unwrap();
    let rows: Vec<(i64, i64)> = st
        .query_map(rusqlite::params![&gid], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let state_len: i64 = conn
        .query_row(
            "SELECT length(data) FROM mls_group_state WHERE group_id=?1",
            rusqlite::params![&gid],
            |r| r.get(0),
        )
        .unwrap();
    println!(
        "\n---- §3 the NATIVE store: SqliteGroupStateStorage, 5 members, {} epochs reached ----",
        epochs_written
    );
    println!(
        "     mls_epoch rows on disk: {} of {} epochs — {:?}",
        rows.len(),
        epochs_written + 1,
        rows
    );
    println!("     mls_group_state row: {state_len} B");
    println!(
        "     whole group on disk: {} B of epoch rows + {state_len} B of state = {} B",
        rows.iter().map(|(_, b)| b).sum::<i64>(),
        rows.iter().map(|(_, b)| b).sum::<i64>() + state_len
    );
    assert_eq!(
        rows.len() as u64,
        pacific_core::EPOCH_RETENTION + 1,
        "the native store keeps EPOCH_RETENTION+1 rows and nothing older"
    );
    assert_eq!(
        rows.last().unwrap().1 as usize,
        at5_last,
        "the on-disk figure IS the in-memory figure at the same membership (5 leaves)"
    );

    // ---------------------------------------------------------------- §4 ----
    // G5's second half: does "stored once for the whole group" survive an epoch
    // boundary? R2 keys are sha256 CONTENT hashes, so the question is entirely
    // whether two seals of one plaintext produce identical BYTES.
    println!("\n============ G5(b) · R2 CONTENT-ADDRESSING ACROSS AN EPOCH BOUNDARY ===========");
    let a = device(2001);
    let b = device(2002);
    let mut ga = mls::create_group_named(&a.client, "r2").unwrap();
    let kp = mls::make_key_package_bytes(&b.client).unwrap();
    let (_c, w) = mls::add_member(&mut ga, &kp).unwrap();
    let mut gb = mls::join_group(&b.client, &w).unwrap();

    let e0 = ga.current_epoch();
    let tag0 = mls::group_tag(&ga, &mls::epoch_be(e0)).unwrap();
    let sec0 = mls::seal_conn_secret(&ga, &mls::epoch_be(e0)).unwrap();
    assert_eq!(
        tag0,
        mls::group_tag(&gb, &mls::epoch_be(e0)).unwrap(),
        "both members derive the same epoch material — that shared derivation is the\n\
         'whole group' in 'stored once for the whole group'"
    );

    let commit = mls::stage_rename(&mut ga, "r2-next").unwrap();
    mls::apply_staged(&mut ga).unwrap();
    assert!(matches!(
        mls::decrypt_message(&mut gb, &commit).unwrap(),
        Incoming::Commit { .. }
    ));
    let e1 = ga.current_epoch();
    let tag1 = mls::group_tag(&ga, &mls::epoch_be(e1)).unwrap();
    let sec1 = mls::seal_conn_secret(&ga, &mls::epoch_be(e1)).unwrap();
    assert_ne!(sec0, sec1, "one commit ⇒ a different epoch key");
    assert_ne!(tag0, tag1, "…and a different epoch tag");

    // One archive blob, 64 KiB, byte-identical plaintext every time.
    let plain: Vec<u8> = (0..65_536u32).map(|i| (i % 251) as u8).collect();

    let s1 = seal::seal(&plain, &tag0, &sec0).unwrap();
    let s2 = seal::seal(&plain, &tag0, &sec0).unwrap();
    let s3 = seal::seal(&plain, &tag1, &sec1).unwrap();
    println!(
        "     plaintext {} B -> sealed {} B (24 B nonce + 16 B Poly1305 tag)",
        plain.len(),
        s1.len()
    );
    println!("     AS THE TREE SEALS TODAY (seal::seal, RANDOM 24-byte nonce):");
    println!("       identical plaintext, epoch {e0}, seal #1 -> R2 key {}…", &r2_key(&s1)[..16]);
    println!("       identical plaintext, epoch {e0}, seal #2 -> R2 key {}…", &r2_key(&s2)[..16]);
    println!("       identical plaintext, epoch {e1}          -> R2 key {}…", &r2_key(&s3)[..16]);
    assert_ne!(
        r2_key(&s1),
        r2_key(&s2),
        "two seals of ONE plaintext under ONE epoch key already give two R2 keys"
    );
    assert_ne!(r2_key(&s1), r2_key(&s3));

    let c1 = convergent(&plain, &sec0);
    let c2 = convergent(&plain, &sec0);
    let c3 = convergent(&plain, &sec1);
    println!("     WITH A CONVERGENT (plaintext-derived) NONCE — nothing in the tree does this:");
    println!("       identical plaintext, epoch {e0}, seal #1 -> R2 key {}…", &r2_key(&c1)[..16]);
    println!("       identical plaintext, epoch {e0}, seal #2 -> R2 key {}…", &r2_key(&c2)[..16]);
    println!("       identical plaintext, epoch {e1}          -> R2 key {}…", &r2_key(&c3)[..16]);
    assert_eq!(
        r2_key(&c1),
        r2_key(&c2),
        "a deterministic nonce is what would make 'stored once' true WITHIN an epoch"
    );
    assert_ne!(
        r2_key(&c1),
        r2_key(&c3),
        "…and the epoch key change is what makes it false ACROSS one"
    );
    println!(
        "\n     VERDICT: \"stored once for the whole group\" is FALSE across an epoch boundary,\n\
         \x20    and — with the sealing primitives that exist in the tree today — false WITHIN\n\
         \x20    one too, because every seal draws a fresh random nonce."
    );
    println!("===============================================================================\n");
}
