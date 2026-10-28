//! M2 transport hardening — the poison-inbox class is dead.
//!
//! Regression suite for the 2026-07-10 field failure (an orphaned Welcome —
//! its key package already consumed — wedged sync forever) and the transport
//! contract adopted from the case study (_research/mls-transport-case-study.txt):
//!   1. orphan_welcome_is_quarantined_never_wedges — THE live-bug repro;
//!   2. garbage_on_group_tag_is_quarantined_not_fatal — ingest taxonomy on a
//!      group mailbox: junk blobs are recorded + skipped, the fold survives;
//!   3. lost_commit_slot_surfaces_commit_race — pending-commit discipline: a
//!      taken epoch slot means the staged add is discarded (state unchanged)
//!      and the caller is told to rebase, loudly;
//!   4. epoch_retention_is_bounded — mls_store prunes to EPOCH_RETENTION past
//!      epochs (previously unbounded — a forward-secrecy leak).
//!
//! `PACIFIC_STATE_DIR` is process-global, so every test that constructs a `Node`
//! serializes on ENV_LOCK and runs on a current-thread runtime.

use std::sync::Mutex;

use pacific_core::mls_store::migrate;
use pacific_core::router::Routes;
use pacific_core::transport::RelaySession;
use pacific_wire::address::Address;
use pacific_core::{handshake, mls, paths, seal, CoreError, Node, EPOCH_RETENTION};

static ENV_LOCK: Mutex<()> = Mutex::new(());

async fn spawn_relay() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { relay::serve(listener).await });
    format!("ws://{addr}")
}

/// An m1-style raw MLS party on its own EXPLICIT db path (no global state).
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

/// Rebuild the node's own MLS client the way `Node::signer` does (its fields
/// are pub for exactly this kind of white-box test).
fn node_client(node: &Node) -> mls::Client {
    let (sk, pk) = node.dir.mls_signing_keypair().unwrap();
    let sid = mls::signing_identity(&node.id.identity_pk(), &pk);
    mls::build_client_sqlite(&paths::db_path(), sid, mls::SecretKey::new(sk)).unwrap()
}

/// THE live-bug repro. Two Welcomes built on ONE key package land in the intro
/// mailbox (exactly what repeated pair-scans of the same bundle produce). The
/// first joins; the second is a permanent orphan (its KP was consumed). Before
/// M2 the orphan error propagated out of sync, the cursor never advanced, and
/// EVERY subsequent sync refetched and re-failed it — the wedged node observed
/// in the two-simulator demo. Now: quarantined once, cursor advanced, sync
/// stays green forever.
#[tokio::test]
async fn orphan_welcome_is_quarantined_never_wedges() {
    let _g = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PACIFIC_STATE_DIR", tmp.path());
    let url = spawn_relay().await;

    pacific_core::node::set_default_arc(&url).unwrap();
    let mut node = Node::init_identity("A").unwrap();

    node.set_routes(Routes::parse(&url)).unwrap();
    let bundle_str = node.build_contact_bundle().unwrap();
    let bundle = handshake::parse_and_verify(&bundle_str).unwrap();

    // A scanner builds TWO groups on A's ONE key package → welcome + orphan.
    let scanner = Party::new([0x5C; 32]);
    let mut g1 = mls::create_group(&scanner.client()).unwrap();
    let (_c1, w1) = mls::add_member(&mut g1, &bundle.key_package).unwrap();
    let mut g2 = mls::create_group(&scanner.client()).unwrap();
    let (_c2, w2) = mls::add_member(&mut g2, &bundle.key_package).unwrap();

    let mut sess = RelaySession::connect(&url).await.unwrap();
    for w in [w1, w2] {
        let payload = handshake::IntroPayload {
            scanner_pk: scanner.id,
            scanner_name: "Scanner".into(),
            welcome: w,
            why: None,
            kind: Some("connection".to_string()),
            arc: None,
            owner: None,
            // This binary is about the TRANSPORT — two Welcomes landing on one
            // intro tag and both being drainable. It authors nothing, so there is
            // no high-water mark to carry and None is the honest value.
            gen_watermark: None,
        }
        .encode()
        .unwrap();
        let sealed = seal::seal(&payload, &bundle.intro_tag, &bundle.intro_tag).unwrap();
        sess.publish(
            &Address::from_seed(&bundle.intro_tag),
            pacific_wire::blob_b64(&sealed),
        )
        .await
        .unwrap();
    }
    sess.close().await;

    // Sync must SUCCEED: one join, one quarantined orphan.
    node.sync_once()
        .await
        .expect("sync survives the orphan welcome");
    assert_eq!(node.connections().unwrap().len(), 1, "joined exactly once");
    let q = node.quarantined().unwrap();
    assert_eq!(
        q.len(),
        1,
        "the orphan is quarantined evidence, not a wedge"
    );
    assert!(!q[0].2.is_empty(), "quarantine carries the failure reason");

    // THE regression assert: the next sync neither refetches nor re-fails the
    // orphan (cursor advanced past it) and stays green.
    node.sync_once().await.expect("subsequent sync stays green");
    assert_eq!(
        node.quarantined().unwrap().len(),
        1,
        "no re-quarantine — cursor advanced"
    );
}

/// Garbage on a GROUP mailbox: both un-unsealable junk and sealed-but-not-MLS
/// junk are quarantined individually; deltas before and after them still fold.
#[tokio::test]
async fn garbage_on_group_tag_is_quarantined_not_fatal() {
    let _g = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PACIFIC_STATE_DIR", tmp.path());
    let url = spawn_relay().await;

    pacific_core::node::set_default_arc(&url).unwrap();
    let mut node = Node::init_identity("A").unwrap();

    node.set_routes(Routes::parse(&url)).unwrap();
    let oid = node.object_new("forum", "").unwrap();
    let group_id = hex::decode(&oid).unwrap();
    node.object_post(&oid, "before the junk", None).await.unwrap();

    // Derive the group's live mailbox tag exactly as the node does.
    let client = node_client(&node);
    let group = mls::load_group(&client, &group_id).unwrap();
    let eb = mls::epoch_be(group.current_epoch());
    let tag = mls::group_tag(&group, &eb).unwrap();
    let secret = mls::seal_conn_secret(&group, &eb).unwrap();

    let mut sess = RelaySession::connect(&url).await.unwrap();
    // (i) raw junk: fails seal::open.
    sess.publish(
        &mls::group_address(&group, &eb).unwrap(),
        pacific_wire::blob_b64(b"not sealed at all"),
    )
    .await
    .unwrap();
    // (ii) properly sealed junk: passes seal::open, fails MLS decode.
    let sealed_junk = seal::seal(b"junk that is not an MLS message", &tag, &secret).unwrap();
    sess.publish(
        &mls::group_address(&group, &eb).unwrap(),
        pacific_wire::blob_b64(&sealed_junk),
    )
    .await
    .unwrap();
    sess.close().await;

    node.object_post(&oid, "after the junk", None).await.unwrap();

    node.sync_once()
        .await
        .expect("sync survives group-tag junk");
    assert_eq!(
        node.quarantined().unwrap().len(),
        2,
        "both junk blobs quarantined"
    );
    let transcript = node.object_transcript(&oid).unwrap();
    assert_eq!(transcript.len(), 2, "the fold kept every real delta");
    assert!(transcript[0].contains("before the junk"));
    assert!(transcript[1].contains("after the junk"));

    node.sync_once().await.expect("stays green");
    assert_eq!(node.quarantined().unwrap().len(), 2, "no re-quarantine");
}

/// Pending-commit discipline: when the epoch's commit slot is already taken,
/// group_add_member must (a) NOT change local group state (the staged commit
/// was never applied or persisted), (b) surface CommitRace loudly, (c) never
/// send the Welcome (RFC 9420 §14 coupling). Also documents the known M1
/// limitation: a garbage slot-claim poisons that epoch's slot — the specified
/// escape hatch is the deterministic client-side tie-break (case study R3b),
/// not yet built.
#[tokio::test]
async fn lost_commit_slot_surfaces_commit_race() {
    let _g = ENV_LOCK.lock().unwrap();

    // B mints an identity + bundle in its OWN state dir first…
    let tmp_b = tempfile::tempdir().unwrap();
    std::env::set_var("PACIFIC_STATE_DIR", tmp_b.path());
    // B only mints a bundle here and never syncs, so it needs no transport of its own; its
    // Arc, set before its identity (O-73), is never dialled.
    pacific_core::node::set_default_arc("ws://127.0.0.1:1").unwrap();
    let node_b = Node::init_identity("B").unwrap();
    let bundle_b = node_b.build_contact_bundle().unwrap();
    drop(node_b);

    // …then the env moves to A for the rest of the test (A does all MLS work).
    let tmp_a = tempfile::tempdir().unwrap();
    std::env::set_var("PACIFIC_STATE_DIR", tmp_a.path());
    let url = spawn_relay().await;
    pacific_core::node::set_default_arc(&url).unwrap();
    let mut node_a = Node::init_identity("A").unwrap();
    node_a.set_routes(Routes::parse(&url)).unwrap();
    let oid = node_a.object_new("forum", "").unwrap();
    let group_id = hex::decode(&oid).unwrap();

    // Pre-claim the current epoch's commit slot.
    let client = node_client(&node_a);
    let group = mls::load_group(&client, &group_id).unwrap();
    let epoch_before = group.current_epoch();
    let eb = mls::epoch_be(epoch_before);
    let tag = mls::group_tag(&group, &eb).unwrap();
    let secret = mls::seal_conn_secret(&group, &eb).unwrap();
    let squatter = seal::seal(b"not a real commit", &tag, &secret).unwrap();
    let mut sess = RelaySession::connect(&url).await.unwrap();
    let won = sess
        .publish_commit(
            &mls::group_address(&group, &eb).unwrap(),
            pacific_wire::blob_b64(&squatter),
        )
        .await
        .unwrap();
    assert!(won.is_some(), "squatter claims the slot");
    sess.close().await;

    // The add must lose the slot, change nothing, and say so loudly.
    let err = node_a
        .group_add_member(&oid, &bundle_b)
        .await
        .expect_err("slot taken ⇒ the add must fail loudly");
    assert!(
        matches!(err, CoreError::CommitRace(_)),
        "typed CommitRace, got: {err}"
    );

    let group_after = mls::load_group(&node_client(&node_a), &group_id).unwrap();
    assert_eq!(
        group_after.current_epoch(),
        epoch_before,
        "staged commit was discarded — local state unchanged"
    );
    assert_eq!(
        mls::roster_identities(&group_after).unwrap().len(),
        1,
        "no member was half-added"
    );

    // The squatter blob itself is quarantined by the loss-path drain (it can
    // never process), and sync stays green.
    node_a
        .sync_once()
        .await
        .expect("sync green after lost race");
    assert!(
        node_a
            .quarantined()
            .unwrap()
            .iter()
            .any(|(g, _, _, _)| g.as_deref() == Some(&group_id[..])),
        "the squatting junk commit is quarantined evidence"
    );
}

/// Epoch retention is bounded to EPOCH_RETENTION past epochs + current (was
/// unbounded). Five sequential adds advance five epochs; the store must hold
/// at most EPOCH_RETENTION + 1 epoch secrets.
#[test]
fn epoch_retention_is_bounded() {
    let owner = Party::new([0xA0; 32]);
    let mut group = mls::create_group(&owner.client()).unwrap();
    for i in 1..=5u8 {
        let member = Party::new([i; 32]);
        let kp = mls::make_key_package_bytes(&member.client()).unwrap();
        let (_c, _w) = mls::add_member(&mut group, &kp).unwrap();
    }
    assert_eq!(group.current_epoch(), 5, "five adds ⇒ epoch 5");

    let conn = rusqlite::Connection::open(&owner.db).unwrap();
    let rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM mls_epoch WHERE group_id=?1",
            rusqlite::params![group.group_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        rows as u64 <= EPOCH_RETENTION + 1,
        "epoch secrets bounded to retention window, got {rows} rows"
    );
    // The window is anchored to the newest STORED epoch (the current epoch's
    // secrets live in the group-state blob, so stored records lag one behind):
    // max_stored - min_stored must never exceed EPOCH_RETENTION.
    let (min_epoch, max_epoch): (i64, i64) = conn
        .query_row(
            "SELECT MIN(epoch_id), MAX(epoch_id) FROM mls_epoch WHERE group_id=?1",
            rusqlite::params![group.group_id()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(
        (max_epoch - min_epoch) as u64 <= EPOCH_RETENTION,
        "stored epoch span bounded by the retention window (min {min_epoch}, max {max_epoch})"
    );
}
