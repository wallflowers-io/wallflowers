//! M1 N-member conformance — the group scales past two.
//!
//! In-process (no relay/CLI), the same style as m0: three parties in ONE MLS
//! group. Proves the load-bearing N-member mechanics:
//!   (a) an owner ADD distributes a commit that an EXISTING member processes to
//!       advance its epoch + roster (the commit-distribution mechanic);
//!   (b) all three members derive the SAME shared group tag at the new epoch;
//!   (c) an application message carries its MLS-authenticated SENDER;
//!   (d) skip-own: a member drains its own echoed post off the shared tag and
//!       mls-rs reports it as `CantProcessMessageFromSelf` -> Incoming::SkippedOwn;
//!   (e) the commutative fold over the roster converges byte-for-byte regardless
//!       of arrival order;
//!   (f) a non-member author is rejected loudly.

use pacific_core::coordinator::{self, Coordinator, ForumType};
use pacific_core::mls::{self, Incoming};
use pacific_core::mls_store::migrate;
use pacific_core::seal;

/// A party on its own temp SQLite DB with a fresh MLS signing key. `id` is the
/// stable identity pubkey — used as the BasicCredential id AND (thus) the author
/// id the commutative fold keys membership on, exactly as the node does.
struct Party {
    _tmp: tempfile::TempDir,
    db: std::path::PathBuf,
    sig_sk: Vec<u8>,
    sig_pk: Vec<u8>,
    id: [u8; 32],
}

impl Party {
    fn new(id: [u8; 32]) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("pacific.db");
        migrate(&db).unwrap();
        let crypto = mls::crypto();
        let (sk, pk) = mls::generate_signing_key(&crypto).unwrap();
        Party {
            _tmp: tmp,
            db,
            sig_sk: sk.as_bytes().to_vec(),
            sig_pk: pk.as_bytes().to_vec(),
            id,
        }
    }

    fn client(&self) -> mls::Client {
        let sid = mls::signing_identity(&self.id, &self.sig_pk);
        let sk = mls::SecretKey::new(self.sig_sk.clone());
        mls::build_client_sqlite(&self.db, sid, sk).unwrap()
    }
}

#[test]
fn nmember_add_distributes_commit_and_all_converge() {
    let a = Party::new([0xA1; 32]); // owner
    let b = Party::new([0xB2; 32]);
    let c = Party::new([0xC3; 32]);

    // A creates the group and adds B (the 2-party bootstrap). Epoch 0 -> 1.
    let b_kp = mls::make_key_package_bytes(&b.client()).unwrap();
    let mut g_a = mls::create_group(&a.client()).unwrap();
    let (_c1, w_b) = mls::add_member(&mut g_a, &b_kp).unwrap();
    let mut g_b = mls::join_group(&b.client(), &w_b).unwrap();
    assert_eq!(
        g_a.current_epoch(),
        g_b.current_epoch(),
        "A and B in epoch lockstep at join"
    );

    // A adds C. Capture the CURRENT (pre-add) epoch's tag + secret so B — still at
    // that epoch — can drain the commit off it. add_member advances A one epoch.
    let c_kp = mls::make_key_package_bytes(&c.client()).unwrap();
    let e_pre = mls::epoch_be(g_a.current_epoch());
    let old_tag = mls::group_tag(&g_a, &e_pre).unwrap();
    let old_secret = mls::seal_conn_secret(&g_a, &e_pre).unwrap();
    let (commit, w_c) = mls::add_member(&mut g_a, &c_kp).unwrap();
    let mut g_c = mls::join_group(&c.client(), &w_c).unwrap();

    // (a) COMMIT DISTRIBUTION: B processes the sealed commit off the old tag and
    // advances — the mechanic that did not exist in the pairwise pipeline.
    let sealed_commit = seal::seal(&commit, &old_tag, &old_secret).unwrap();
    let inner_commit = seal::open(&sealed_commit, &old_tag, &old_secret).unwrap();
    match mls::decrypt_message(&mut g_b, &inner_commit).unwrap() {
        Incoming::Commit { .. } => {}
        _ => panic!("B should process the add-C commit as a handshake"),
    }

    // (b) all three now agree on epoch, the shared tag, and the roster {A,B,C}.
    assert_eq!(
        g_a.current_epoch(),
        g_b.current_epoch(),
        "B advanced to A's epoch"
    );
    assert_eq!(
        g_a.current_epoch(),
        g_c.current_epoch(),
        "C joined at A's epoch"
    );
    let e = mls::epoch_be(g_a.current_epoch());
    let tag = mls::group_tag(&g_a, &e).unwrap();
    assert_eq!(
        tag,
        mls::group_tag(&g_b, &e).unwrap(),
        "B derives the same group tag"
    );
    assert_eq!(
        tag,
        mls::group_tag(&g_c, &e).unwrap(),
        "C derives the same group tag"
    );
    let secret = mls::seal_conn_secret(&g_a, &e).unwrap();

    let mut roster = mls::roster_identities(&g_a).unwrap();
    roster.sort();
    let mut expect = vec![a.id, b.id, c.id];
    expect.sort();
    assert_eq!(roster, expect, "the MLS roster is exactly {{A, B, C}}");
    assert_eq!(mls::roster_identities(&g_b).unwrap().len(), 3);
    assert_eq!(mls::roster_identities(&g_c).unwrap().len(), 3);

    // (c)+(d) A posts one Delta on the shared tag: it names A as the MLS sender to
    // B and C, and A skips its OWN echoed copy.
    let da = coordinator::forum_post("hello from A", 0, g_a.current_epoch());
    let da_cbor = da.canonical_bytes();
    let inner_a = mls::encrypt_delta(&mut g_a, &da_cbor).unwrap();
    let sealed_a = seal::seal(&inner_a, &tag, &secret).unwrap();

    let self_inner = seal::open(&sealed_a, &tag, &secret).unwrap();
    assert!(
        matches!(
            mls::decrypt_message(&mut g_a, &self_inner).unwrap(),
            Incoming::SkippedOwn
        ),
        "A skips its own echoed post (CantProcessMessageFromSelf)"
    );

    let inner_b = seal::open(&sealed_a, &tag, &secret).unwrap();
    match mls::decrypt_message(&mut g_b, &inner_b).unwrap() {
        Incoming::Application { sender, data } => {
            assert_eq!(sender, a.id, "B sees A as the sender");
            assert_eq!(data, da_cbor, "B round-trips A's plaintext");
        }
        _ => panic!("B expected A's application message"),
    }
    let inner_c = seal::open(&sealed_a, &tag, &secret).unwrap();
    match mls::decrypt_message(&mut g_c, &inner_c).unwrap() {
        Incoming::Application { sender, data } => {
            assert_eq!(sender, a.id, "C sees A as the sender");
            assert_eq!(data, da_cbor, "C round-trips A's plaintext");
        }
        _ => panic!("C expected A's application message"),
    }

    // (e) the commutative fold over the roster converges regardless of order.
    let epoch = g_a.current_epoch();
    let da = coordinator::forum_post("hello from A", 0, epoch);
    let db = coordinator::forum_post("hello from B", 0, epoch);
    let dc = coordinator::forum_post("hello from C", 0, epoch);
    let members = mls::roster_identities(&g_a).unwrap();

    let mut c1 = Coordinator::<ForumType>::new(members.clone(), a.id);
    c1.deliver(da.clone(), a.id).unwrap();
    c1.deliver(db.clone(), b.id).unwrap();
    c1.deliver(dc.clone(), c.id).unwrap();

    let mut c2 = Coordinator::<ForumType>::new(members.clone(), a.id);
    c2.deliver(dc, c.id).unwrap();
    c2.deliver(da, a.id).unwrap();
    c2.deliver(db, b.id).unwrap();

    assert_eq!(
        c1.state().transcript(),
        c2.state().transcript(),
        "the 3-party fold is order-independent"
    );
    assert_eq!(
        c1.state().transcript().len(),
        3,
        "all three posts are present"
    );

    // (f) REVERSED ON PURPOSE (membership-through-mls.md §15.6). This asserted that the
    //     fold rejects an author outside the CURRENT roster — which, once removal
    //     exists, stops every object a removed member ever wrote in from folding. The
    //     question "was the author a member when they wrote it" is answered once, by
    //     MLS, at ingest (§10.1). So the property is proved where it now lives: an
    //     outsider cannot produce a message the group will decrypt at all.
    let _ = members;
    let outsider = Party::new([0x99; 32]);
    let mut og = mls::create_group(&outsider.client()).unwrap();
    let forged = mls::encrypt_delta(
        &mut og,
        &coordinator::forum_post("intruder", 0, epoch).canonical_bytes(),
    )
    .unwrap();
    assert!(
        mls::decrypt_message(&mut g_a, &forged).is_err(),
        "an outsider's message does not decrypt in the group, so it never reaches a log"
    );
}
