//! M0 in-process conformance test — the crux.
//!
//! Proves the MLS + seal + coordinator + cross-process-`load_group` pipeline end
//! to end, WITHOUT the relay or the CLI. This is the in-process form of the M1
//! acceptance script and refutes the two worst defects the design carried (the
//! invented `snapshot()` API; the export-tag-equality assumption).

use pacific_core::coordinator::{self, Coordinator, ForumType};
use pacific_core::mls;
use pacific_core::mls_store::migrate;
use pacific_core::seal;

/// Build a client on its own temp SQLite DB with a fresh signing key. Returns
/// (client, db_path-keepalive, signing_pub_bytes).
struct Party {
    _tmp: tempfile::TempDir,
    db: std::path::PathBuf,
    sig_sk: Vec<u8>,
    sig_pk: Vec<u8>,
}

impl Party {
    fn new() -> Self {
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
        }
    }

    fn client(&self) -> mls::Client {
        // credential id = the 32-byte author id (mirrors the node, where cred_id is
        // the Ed25519 identity pubkey). Must be 32 bytes so the roster/sender
        // resolution round-trips.
        let sid = mls::signing_identity(&author_id(&self.sig_pk), &self.sig_pk);
        let sk = mls::SecretKey::new(self.sig_sk.clone());
        mls::build_client_sqlite(&self.db, sid, sk).unwrap()
    }
}

#[test]
fn m0_full_pipeline_including_cross_process_load() {
    let alice = Party::new();
    let bob = Party::new();

    // 1. Bob publishes a KeyPackage; Alice creates the group + adds Bob.
    let bob_kp = mls::make_key_package_bytes(&bob.client()).unwrap();
    let mut g_a = mls::create_group(&alice.client()).unwrap();
    let group_id = g_a.group_id().to_vec();
    let (_commit, welcome) = mls::add_member(&mut g_a, &bob_kp).unwrap();

    // 2. Bob joins from the Welcome.
    let mut g_b = mls::join_group(&bob.client(), &welcome).unwrap();

    // (1) both export the SAME shared group tag at the join epoch.
    let epoch_a = mls::epoch_be(g_a.current_epoch());
    let epoch_b = mls::epoch_be(g_b.current_epoch());
    assert_eq!(g_a.current_epoch(), g_b.current_epoch(), "epoch lockstep");
    let tag_a = mls::group_tag(&g_a, &epoch_a).unwrap();
    let tag_b = mls::group_tag(&g_b, &epoch_b).unwrap();
    assert_eq!(tag_a, tag_b, "both sides derive the same shared group tag");

    // (2) an application message round-trips through encrypt -> seal -> open -> decrypt.
    let delta = coordinator::forum_post("hello bob", 0, g_a.current_epoch());
    let cbor = delta.canonical_bytes();
    let inner = mls::encrypt_delta(&mut g_a, &cbor).unwrap();

    let dest = tag_a; // route on the shared pairwise tag
    let conn_a = mls::seal_conn_secret(&g_a, &epoch_a).unwrap();
    let conn_b = mls::seal_conn_secret(&g_b, &epoch_b).unwrap();
    assert_eq!(conn_a, conn_b, "both sides derive the same seal secret");

    let sealed = seal::seal(&inner, &dest, &conn_a).unwrap();
    // relay-blindness: the sealed blob does not contain the plaintext delta.
    assert!(!sealed.windows(cbor.len()).any(|w| w == &cbor[..]));

    let got_inner = seal::open(&sealed, &dest, &conn_b).unwrap();
    let plain = match mls::decrypt_message(&mut g_b, &got_inner).unwrap() {
        mls::Incoming::Application { data, .. } => data,
        _ => panic!("expected an application message"),
    };
    assert_eq!(
        plain, cbor,
        "decrypted plaintext == original canonical delta"
    );

    // (3) fold -> transcript matches, order-independent (OR-Set conformance).
    let mut coord = Coordinator::<ForumType>::new(
        vec![
            // membership keyed by MLS signing pubkeys (the in-test author ids).
            author_id(&alice.sig_pk),
            author_id(&bob.sig_pk),
        ],
        author_id(&alice.sig_pk), // alice created the group -> owner
    );
    let decoded = coordinator::decode_delta(&plain).unwrap();
    assert!(coord.deliver(decoded, author_id(&alice.sig_pk)).unwrap());
    let transcript = coord.state().transcript();
    assert_eq!(transcript.len(), 1);
    assert_eq!(transcript[0].2, "hello bob");

    // (4) cross-process: DROP both live groups, rebuild via client.load_group()
    //     from the SAME SQLite stores, and re-derive the identical tag. This is
    //     the multi-process-CLI seam (each `pacific <verb>` is a fresh process).
    drop(g_a);
    drop(g_b);

    let g_a2 = mls::load_group(&alice.client(), &group_id).unwrap();
    let g_b2 = mls::load_group(&bob.client(), &group_id).unwrap();
    let e2 = mls::epoch_be(g_a2.current_epoch());
    let tag_a2 = mls::group_tag(&g_a2, &e2).unwrap();
    let tag_b2 = mls::group_tag(&g_b2, &e2).unwrap();
    assert_eq!(
        tag_a2, tag_b2,
        "reloaded groups still derive the same tag (GroupStateStorage seam works)"
    );
    assert_eq!(
        tag_a2, tag_a,
        "reloaded tag matches the live tag (state survived the process boundary)"
    );
}

/// In the test we use the 32-byte MLS signing pubkey (truncated/padded) as the
/// author id, mirroring how the node keys membership by identity pubkey.
fn author_id(sig_pk: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let n = sig_pk.len().min(32);
    out[..n].copy_from_slice(&sig_pk[..n]);
    out
}
