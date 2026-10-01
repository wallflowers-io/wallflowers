//! MEMBERSHIP THROUGH MLS — leave, remove and hand over, from real devices over a real
//! relay (membership-through-mls.md §15.3).
//!
//! The library tests in `mls.rs` (`rules_tests`) prove the commit rules with a rogue
//! client that skips them. This binary proves the doors: that `Node::group_leave`,
//! `Node::group_remove_member` and `Node::group_hand_over` drive the rules through the
//! relay's commit slot, that every other device learns the result by syncing, and that
//! a device taken out of a group knows it and holds nothing it could read the group
//! with afterwards.
//!
//! WHAT IS NOT HERE, and why. A leave is never committed by the leaver (RFC 9420
//! §12.2), so every leave scenario names who completes it. And a late joiner cannot
//! read the membership records sealed before its join (spec §17 item 9), so record
//! assertions are made on devices that were present when the records were written.

mod common;

use common::Harness;
use mls_rs::mls_rules::{DefaultMlsRules, EncryptionOptions};
use mls_rs::{Extension, MlsMessage};
use pacific_core::coordinator::{ArgVal, Args, Ballot, Rule};
use pacific_core::group::OP_SET_PROFILE;
use pacific_core::handshake;
use pacific_core::membership::RemovalReason;
use pacific_core::place::Access;
use pacific_core::router::Router;
use pacific_core::{mls, paths, seal};
use pacific_wire::address::Address;

/// A ROGUE client over device `u`'s own MLS stores, running mls-rs's DEFAULT rules —
/// what a modified build holding this member's keys could send. None of Pacific's
/// send-side rules apply to it, so what it produces meets only the receive side. It
/// sends PrivateMessage like a real build, so a refusal is the rules' doing, not the
/// wire format's. Loads `obj`'s group; nothing it stages is persisted unless the test
/// says so.
macro_rules! rogue_group {
    ($h:expr, $u:expr, $obj:expr) => {{
        let node = $h.node($u);
        let (sk, pk) = node.dir.mls_signing_keypair().unwrap();
        let db = paths::db_path();
        let mut enc = EncryptionOptions::default();
        enc.encrypt_control_messages = true;
        let client = mls_rs::Client::builder()
            .identity_provider(mls::PerLeafIdentity)
            .crypto_provider(mls::crypto())
            .group_state_storage(pacific_core::mls_store::SqliteGroupStateStorage::open(&db).unwrap())
            .key_package_repo(pacific_core::mls_store::SqliteKeyPackageStorage::open(&db).unwrap())
            .mls_rules(DefaultMlsRules::new().with_encryption_options(enc))
            .extension_types([mls::GROUP_NAME_EXT, mls::OWNER_EXT, mls::MIN_VERSION_EXT])
            .signing_identity(
                mls::signing_identity(&node.id.identity_pk(), &pk),
                mls::SecretKey::new(sk),
                mls::CIPHERSUITE,
            )
            .build();
        client.load_group(&hex::decode($obj).unwrap()).unwrap()
    }};
}

/// The current epoch's relay tag, commit-slot address and seal secret for `obj`, as
/// device `u` holds them — what a commit at this epoch is sealed and published with.
fn slot_keys(h: &Harness, u: usize, obj: &str) -> ([u8; 32], Address, [u8; 32]) {
    h.with_mls_group(u, obj, |g| {
        let eb = mls::epoch_be(g.current_epoch());
        (
            mls::group_tag(g, &eb).unwrap(),
            mls::group_address(g, &eb).unwrap(),
            mls::seal_conn_secret(g, &eb).unwrap(),
        )
    })
}

/// Seal `commit` to the epoch's tag and claim the epoch's commit slot, as device `u`.
/// Returns whether it won.
async fn claim_slot(h: &Harness, u: usize, obj: &str, commit: &[u8]) -> bool {
    let (tag, addr, secret) = slot_keys(h, u, obj);
    let sealed = seal::seal(commit, &tag, &secret).unwrap();
    let routes = h.node(u).routes().clone();
    let mut sess = Router::open(&routes).await.unwrap();
    let won = sess
        .publish_commit(&addr, &pacific_wire::blob_b64(&sealed))
        .await
        .unwrap();
    sess.close().await;
    won.is_some()
}

/// Deliver a Welcome to a newcomer's intro mailbox the way the Add door does, with
/// whatever owner the sender chooses to CLAIM.
async fn send_welcome(h: &Harness, from: usize, to: &handshake::ContactBundle, welcome: Vec<u8>, kind: &str, claimed_owner: [u8; 32]) {
    let payload = handshake::IntroPayload {
        scanner_pk: h.id(from),
        scanner_name: h.name(from).to_string(),
        welcome,
        why: None,
        kind: Some(kind.to_string()),
        arc: None,
        owner: Some(claimed_owner),
        gen_watermark: None,
    }
    .encode()
    .unwrap();
    let sealed = seal::seal(&payload, &to.intro_tag, &to.intro_tag).unwrap();
    let routes = h.node(from).routes().clone();
    let mut sess = Router::open(&routes).await.unwrap();
    sess.publish(&Address::from_seed(&to.intro_tag), &pacific_wire::blob_b64(&sealed))
        .await
        .unwrap();
    sess.close().await;
}

/// `member`'s ROGUE commit that adds `newcomer` AND rewrites the owner to `member` —
/// Marmot's Welcome-bootstrap attack (§8.5). Returns (commit, welcome), unapplied.
fn add_and_seize(h: &Harness, member: usize, obj: &str, newcomer: &handshake::ContactBundle) -> (Vec<u8>, Vec<u8>) {
    let mut g = rogue_group!(h, member, obj);
    let mut exts = g.context().extensions().clone();
    exts.set(Extension::new(mls::OWNER_EXT, h.id(member).to_vec()));
    let kp = MlsMessage::from_bytes(&newcomer.key_package).unwrap();
    let out = g
        .commit_builder()
        .add_member(kp)
        .unwrap()
        .set_group_context_ext(exts)
        .unwrap()
        .build()
        .unwrap();
    (
        out.commit_message.to_bytes().unwrap(),
        out.welcome_messages[0].to_bytes().unwrap(),
    )
}

fn has_owner_ext(h: &Harness, u: usize, obj: &str) -> bool {
    h.with_mls_group(u, obj, |g| mls::has_owner_ext(g))
}

fn texts(h: &Harness, u: usize, obj: &str) -> Vec<String> {
    h.obj_view(u, obj).into_iter().map(|m| m.text).collect()
}

/// Does device `u` still hold MLS state for `obj`? `Harness::with_mls_group` panics
/// when it does not, which is the right default everywhere but here.
fn holds_group(h: &Harness, u: usize, obj: &str) -> bool {
    let node = h.node(u);
    let (sk, pk) = node.dir.mls_signing_keypair().unwrap();
    let sid = mls::signing_identity(&node.id.identity_pk(), &pk);
    let client = mls::build_client_sqlite(&paths::db_path(), sid, mls::SecretKey::new(sk)).unwrap();
    mls::load_group(&client, &hex::decode(obj).unwrap()).is_ok()
}

/// The people in `obj` as `u` sees them, sorted — roster order is MLS leaf order, an
/// artefact of where the tree put each member (m21 says the same).
fn people(h: &Harness, u: usize, obj: &str) -> Vec<String> {
    let mut p = h.roster_people(u, obj);
    p.sort();
    p
}

fn owner_seen_by(h: &Harness, u: usize, obj: &str) -> [u8; 32] {
    h.with_mls_group(u, obj, |g| mls::group_owner(g).unwrap())
}

fn assert_compliant(h: &Harness, us: &[usize]) {
    for &u in us {
        let bad = h.node(u).noncompliant_objects().unwrap();
        assert!(
            bad.is_empty(),
            "{} holds objects it cannot fold: {:?}",
            h.device_name(u),
            bad.iter().map(|n| (&n.kind, &n.reason)).collect::<Vec<_>>()
        );
    }
}

async fn remove(h: &Harness, owner: usize, obj: &str, member: usize, reason: Option<&str>) {
    let (o, m, r) = (obj.to_string(), hex::encode(h.id(member)), reason.map(str::to_string));
    h.with(owner, |n| async move { n.group_remove_member(&o, &m, r.as_deref()).await })
        .await
        .expect("the owner's remove");
}

async fn leave(h: &Harness, u: usize, obj: &str) {
    let o = obj.to_string();
    h.with(u, |n| async move { n.group_leave(&o).await })
        .await
        .expect("the leave");
}

/// The owner removes a member. The member's device learns it, deletes the group's MLS
/// state and can neither read what is said afterwards nor say anything. What it wrote
/// while a member still folds on every device that stays.
#[tokio::test]
async fn the_owner_removes_a_member_who_can_then_neither_read_nor_write() {
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let (ada, bo, cy) = (0, 1, 2);
    let obj = h.form_forum(ada, &[bo, cy]).await;
    h.obj_post(bo, &obj, "bo, while a member").await;
    h.settle().await;

    remove(&h, ada, &obj, bo, Some("conduct")).await;
    h.settle().await;

    assert!(h.node(bo).is_departed(&obj).unwrap(), "bo's device knows it was removed");
    assert!(!holds_group(&h, bo, &obj), "and holds no MLS state for the group");
    for u in [ada, cy] {
        assert_eq!(people(&h, u, &obj), vec!["ada", "cy"], "{}", h.device_name(u));
        assert_eq!(h.device_leaves(u, &obj), 2);
        assert!(!h.node(u).is_departed(&obj).unwrap());
    }
    assert_eq!(h.epoch(ada, &obj), h.epoch(cy, &obj), "one group, one epoch");

    h.obj_post(ada, &obj, "after bo left").await;
    h.settle().await;
    assert!(texts(&h, cy, &obj).contains(&"after bo left".to_string()));
    let bos = texts(&h, bo, &obj);
    assert!(!bos.contains(&"after bo left".to_string()), "a removed member reads nothing new");
    assert!(bos.contains(&"bo, while a member".to_string()), "but keeps what it received");

    let o = obj.clone();
    let e = h
        .with(bo, |n| async move { n.object_post(&o, "let me back in", None).await })
        .await
        .expect_err("a removed member cannot write");
    assert!(e.to_string().contains("removed"), "the refusal says why: {e}");

    for u in [ada, cy] {
        assert!(
            texts(&h, u, &obj).contains(&"bo, while a member".to_string()),
            "{} still folds what bo wrote while a member",
            h.device_name(u)
        );
    }
    assert_compliant(&h, &[ada, bo, cy]);
}

/// A removal on a Group-typed object is recorded with its reason (amendment 8), by the
/// door that made it, and the records agree with the tree afterwards.
#[tokio::test]
async fn a_removal_is_recorded_with_its_reason() {
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let (ada, bo, cy) = (0, 1, 2);
    let obj = h.mint_group(ada);
    h.add_to_forum(ada, bo, &obj).await;
    h.add_to_forum(ada, cy, &obj).await;

    remove(&h, ada, &obj, bo, Some("conduct")).await;
    h.settle().await;

    let bo_hex = hex::encode(h.id(bo));
    for u in [ada, bo] {
        let st = h.node(u).group_state(&obj).unwrap();
        let t = st.membership.departed(&bo_hex).expect("bo's departure is recorded");
        assert_eq!(t.reason, Some(RemovalReason::Conduct), "{}", h.device_name(u));
        assert!(!st.membership.is_present(&bo_hex));
    }
    let roster = h.roster(ada, &obj);
    assert_eq!(
        h.node(ada).group_state(&obj).unwrap().membership.divergence(&roster),
        None,
        "ada holds every record, and they agree with the tree"
    );
    // cy joined after ada's founder record was sealed, so reads ada as unrecorded —
    // the late-joiner limit (§17 item 9), and nothing more: no phantom, no bo.
    let d = h
        .node(cy)
        .group_state(&obj)
        .unwrap()
        .membership
        .divergence(&roster)
        .expect("the late joiner lacks the founder's record");
    assert_eq!(d.unrecorded, vec![hex::encode(h.id(ada))]);
    assert!(d.phantom.is_empty() && d.pending_removal.is_empty(), "{d:?}");

    let (o, m) = (obj.clone(), hex::encode(h.id(cy)));
    let e = h
        .with(ada, |n| async move { n.group_remove_member(&o, &m, Some("spite")).await })
        .await
        .expect_err("a reason outside the vocabulary is refused");
    assert!(e.to_string().contains("unknown removal reason"), "{e}");
}

/// bo leaves while ada, the owner, is offline. cy completes it by committing bo's
/// proposal (§7): no owner is needed for a leave. ada catches up later.
#[tokio::test]
async fn a_member_leaves_while_the_owner_is_offline_and_another_member_completes_it() {
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let (ada, bo, cy) = (0, 1, 2);
    let obj = h.form_forum(ada, &[bo, cy]).await;
    let before = h.epoch(cy, &obj);

    leave(&h, bo, &obj).await;
    assert!(h.node(bo).is_leaving(&obj).unwrap(), "the leave is in flight");
    leave(&h, bo, &obj).await; // idempotent while in flight

    h.settle_among(&[bo, cy]).await;
    assert!(h.node(bo).is_departed(&obj).unwrap(), "cy's commit completed the leave");
    assert!(!h.node(bo).is_leaving(&obj).unwrap(), "and the leave is no longer pending");
    assert!(!holds_group(&h, bo, &obj));
    assert_eq!(people(&h, cy, &obj), vec!["ada", "cy"]);
    assert_eq!(h.epoch(cy, &obj), before + 1, "one commit completed it");

    h.settle().await;
    assert_eq!(people(&h, ada, &obj), vec!["ada", "cy"], "ada catches up");
    assert_eq!(h.epoch(ada, &obj), h.epoch(cy, &obj));

    h.obj_post(cy, &obj, "just us now").await;
    h.settle().await;
    assert!(texts(&h, ada, &obj).contains(&"just us now".to_string()));
    assert!(!texts(&h, bo, &obj).contains(&"just us now".to_string()));
}

/// Leaving from one device takes every device of that person (§6.4): the proposals
/// name both of bo's leaves, bo's laptop adopts the leave rather than committing it
/// away, and ada's commit removes both.
#[tokio::test]
async fn leaving_from_one_device_takes_every_device_of_that_person() {
    let h = Harness::people(&[("ada", &["phone"]), ("bo", &["phone", "laptop"])]).await;
    let ada = h.device("ada", "phone");
    let (bo_phone, bo_laptop) = (h.device("bo", "phone"), h.device("bo", "laptop"));
    let obj = h.form_forum(ada, &[bo_phone]).await;
    h.add_to_forum(ada, bo_laptop, &obj).await;
    assert_eq!(h.device_leaves(ada, &obj), 3);

    leave(&h, bo_phone, &obj).await;
    h.settle().await;

    for u in [bo_phone, bo_laptop] {
        assert!(h.node(u).is_departed(&obj).unwrap(), "{} is out", h.device_name(u));
        assert!(!holds_group(&h, u, &obj));
    }
    assert_eq!(h.device_leaves(ada, &obj), 1, "both of bo's leaves are gone");
    assert_eq!(people(&h, ada, &obj), vec!["ada"]);
}

/// Only the owner removes. The doors refuse everyone else, and a Remove proposal
/// forged below the doors and published over the relay changes nothing: the rules drop
/// it from every commit, and the member it names does not take it as their own leave.
#[tokio::test]
async fn nobody_but_the_owner_can_remove_and_a_forged_proposal_changes_nothing() {
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let (ada, bo, cy) = (0, 1, 2);
    let obj = h.form_forum(ada, &[bo, cy]).await;

    for target in [cy, ada] {
        let (o, m) = (obj.clone(), hex::encode(h.id(target)));
        let e = h
            .with(bo, |n| async move { n.group_remove_member(&o, &m, None).await })
            .await
            .expect_err("a member's remove door refuses");
        assert!(e.to_string().contains("only the owner"), "{e}");
    }
    let (o, m) = (obj.clone(), hex::encode(h.id(ada)));
    let e = h
        .with(ada, |n| async move { n.group_remove_member(&o, &m, None).await })
        .await
        .expect_err("the owner cannot remove themselves");
    assert!(e.to_string().contains("hand the object over"), "{e}");

    // Below the doors: bo proposes cy's removal and publishes it at the live epoch.
    let gid = hex::decode(&obj).unwrap();
    let cy_id = h.id(cy);
    h.with(bo, |n| async move {
        let (sk, pk) = n.dir.mls_signing_keypair().unwrap();
        let sid = mls::signing_identity(&n.id.identity_pk(), &pk);
        let client = mls::build_client_sqlite(&paths::db_path(), sid, mls::SecretKey::new(sk)).unwrap();
        let mut g = mls::load_group(&client, &gid).unwrap();
        let eb = mls::epoch_be(g.current_epoch());
        let tag = mls::group_tag(&g, &eb).unwrap();
        let addr = mls::group_address(&g, &eb).unwrap();
        let secret = mls::seal_conn_secret(&g, &eb).unwrap();
        let leaf = mls::leaves_of(&g, &cy_id)[0];
        let p = mls::propose_remove(&mut g, leaf).unwrap();
        let sealed = seal::seal(&p, &tag, &secret).unwrap();
        let mut sess = Router::open(n.routes()).await.unwrap();
        sess.publish(&addr, &pacific_wire::blob_b64(&sealed)).await.unwrap();
        sess.close().await;
    })
    .await;

    // cy meets the proposal FIRST, before anyone has committed it away — the order in
    // which adopting it as cy's own leave (§6.4) would bite. cy commits the cache
    // itself instead, and the rules drop the proposal from that commit.
    let e0 = h.epoch(cy, &obj);
    h.sync(cy).await;
    assert!(!h.node(cy).is_leaving(&obj).unwrap(), "cy did not adopt a leave bo proposed");
    assert_eq!(h.epoch(cy, &obj), e0 + 1, "cy committed the waiting proposal away");
    h.settle().await;

    for u in [ada, bo, cy] {
        assert_eq!(people(&h, u, &obj), vec!["ada", "bo", "cy"], "{}", h.device_name(u));
    }
    assert!(!h.node(cy).is_departed(&obj).unwrap());
    assert_eq!(h.epoch(ada, &obj), h.epoch(cy, &obj));
    assert_eq!(h.epoch(bo, &obj), h.epoch(cy, &obj));

    h.obj_post(cy, &obj, "still here").await;
    h.settle().await;
    for u in [ada, bo] {
        assert!(texts(&h, u, &obj).contains(&"still here".to_string()));
    }
}

/// The owner hands over, and the powers go with it: the new owner renames and removes,
/// the old owner can do neither and may now leave. What the old owner wrote while
/// owning still folds, because the fold reads the owner at each delta's epoch.
#[tokio::test]
async fn the_owner_hands_over_and_then_leaves() {
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let (ada, bo, cy) = (0, 1, 2);
    let obj = h.form_forum(ada, &[bo, cy]).await;
    h.obj_post(ada, &obj, "from the founder").await;
    h.settle().await;

    let o = obj.clone();
    let e = h
        .with(ada, |n| async move { n.group_leave(&o).await })
        .await
        .expect_err("the owner may not leave without handing over");
    assert!(e.to_string().contains("hand the object over"), "{e}");

    let (o, m) = (obj.clone(), hex::encode(h.id(bo)));
    h.with(ada, |n| async move { n.group_hand_over(&o, &m).await })
        .await
        .expect("the handover");
    h.settle().await;
    for u in [ada, bo, cy] {
        assert_eq!(owner_seen_by(&h, u, &obj), h.id(bo), "{} reads bo as owner", h.device_name(u));
    }

    assert!(h.try_rename(ada, &obj, "ada's").await.is_err(), "the old owner cannot rename");
    h.rename_object(bo, &obj, "bo's now").await;
    assert_eq!(h.object_name(cy, &obj), "bo's now");
    let (o, m) = (obj.clone(), hex::encode(h.id(cy)));
    let e = h
        .with(ada, |n| async move { n.group_remove_member(&o, &m, None).await })
        .await
        .expect_err("the old owner cannot remove");
    assert!(e.to_string().contains("only the owner"), "{e}");

    leave(&h, ada, &obj).await;
    h.settle().await;
    assert!(h.node(ada).is_departed(&obj).unwrap());
    for u in [bo, cy] {
        assert_eq!(people(&h, u, &obj), vec!["bo", "cy"], "{}", h.device_name(u));
        assert_eq!(owner_seen_by(&h, u, &obj), h.id(bo));
        assert!(texts(&h, u, &obj).contains(&"from the founder".to_string()));
    }
    assert_compliant(&h, &[ada, bo, cy]);
}

/// A proposal waiting in the cache does not stop anyone posting (§7): cy posts without
/// syncing first, the post path drains bo's leave proposal, commits it, and sends at
/// the new epoch — which bo can no longer read.
#[tokio::test]
async fn a_pending_leave_does_not_stop_anyone_posting() {
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let (ada, bo, cy) = (0, 1, 2);
    let obj = h.form_forum(ada, &[bo, cy]).await;

    leave(&h, bo, &obj).await;
    h.obj_post(cy, &obj, "posted over a pending leave").await;
    h.settle().await;

    assert!(h.node(bo).is_departed(&obj).unwrap(), "cy's post completed bo's leave");
    assert!(texts(&h, ada, &obj).contains(&"posted over a pending leave".to_string()));
    assert!(
        !texts(&h, bo, &obj).contains(&"posted over a pending leave".to_string()),
        "the post was sealed after the commit that removed bo"
    );
}

/// No routine self-update (19 Sep ruling; resumption.md §10): a leaf last refreshed
/// sixty days ago is not refreshed by a sync. PCS is forgone under retained epochs, so
/// a scheduled update would be an epoch per group per week buying nothing.
#[tokio::test]
async fn no_routine_self_update() {
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let obj = h.form_forum(ada, &[bo]).await;
    let gid = hex::decode(&obj).unwrap();
    let e0 = h.epoch(ada, &obj);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    h.with_sync(ada, |n| n.dir.record_self_update(&gid, now - 60 * 24 * 3600, e0).unwrap());
    h.sync(ada).await;
    h.settle().await;
    assert_eq!(h.epoch(ada, &obj), e0, "sixty idle days move no epoch");
    assert_eq!(h.epoch(bo, &obj), e0);
}

/// A closed decision is frozen (§10.4): removing someone who voted against it does not
/// recount it over the roster that remains.
#[tokio::test]
async fn removing_a_dissenter_does_not_flip_a_closed_decision() {
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let (ada, bo, cy) = (0, 1, 2);
    let obj = h.form_forum(ada, &[bo, cy]).await;

    let p = h.obj_propose(ada, &obj, "paint it green", Rule::Consent).await;
    h.settle().await;
    h.obj_vote(bo, &obj, p.clone(), Ballot::Reject).await;
    h.obj_vote(cy, &obj, p.clone(), Ballot::Approve).await;
    h.settle().await;
    h.obj_close(ada, &obj, p).await;
    h.settle().await;
    for u in [ada, bo, cy] {
        assert_eq!(h.obj_ratify(u, &obj)[0].outcome, "failed", "{}", h.device_name(u));
    }

    remove(&h, ada, &obj, bo, None).await;
    h.settle().await;
    for u in [ada, cy] {
        let r = &h.obj_ratify(u, &obj)[0];
        assert_eq!(r.outcome, "failed", "{} — bo's reject still counts", h.device_name(u));
        assert_eq!(r.reject, 1);
    }
}

/// A removed member can be added back (§8.3 step 6): the fresh Add makes the device a
/// member again, and it reads and writes from the epoch it rejoined at.
#[tokio::test]
async fn a_removed_member_can_be_added_back() {
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let obj = h.form_forum(ada, &[bo]).await;
    remove(&h, ada, &obj, bo, Some("requested")).await;
    h.settle().await;
    assert!(h.node(bo).is_departed(&obj).unwrap());

    h.add_to_forum(ada, bo, &obj).await;
    assert!(!h.node(bo).is_departed(&obj).unwrap(), "the Add cleared the departure");
    assert!(holds_group(&h, bo, &obj));
    assert_eq!(h.epoch(bo, &obj), h.epoch(ada, &obj));

    h.obj_post(bo, &obj, "back again").await;
    h.settle().await;
    assert!(texts(&h, ada, &obj).contains(&"back again".to_string()));
}

/// A leave survives the device that began it going quiet (§6.2, §6.4). bo's phone
/// proposes, and before anyone commits the proposals, ada commits something else and
/// they lapse with their epoch. bo's laptop took the leave as its own on DRAINING the
/// proposals — the same drain that then carried ada's commit and cleared the cache — so
/// it proposes again at the next epoch, and cy completes it without the phone.
#[tokio::test]
async fn a_leave_survives_the_proposing_device_going_quiet() {
    let h = Harness::people(&[("ada", &["phone"]), ("bo", &["phone", "laptop"]), ("cy", &["phone"])]).await;
    let ada = h.device("ada", "phone");
    let (bo_phone, bo_laptop) = (h.device("bo", "phone"), h.device("bo", "laptop"));
    let cy = h.device("cy", "phone");
    let obj = h.form_forum(ada, &[bo_phone, cy]).await;
    h.add_to_forum(ada, bo_laptop, &obj).await;

    leave(&h, bo_phone, &obj).await;
    // ada has not synced, so has not seen the proposals: this rename's commit leaves
    // them out, and they lapse with the epoch.
    h.try_rename(ada, &obj, "renamed over a leave").await.expect("the rename");

    // The phone never syncs again.
    h.settle_among(&[ada, bo_laptop, cy]).await;
    assert!(h.node(bo_laptop).is_departed(&obj).unwrap(), "the laptop carried the leave through");
    for u in [ada, cy] {
        assert_eq!(people(&h, u, &obj), vec!["ada", "cy"], "{}", h.device_name(u));
    }

    // When the phone comes back it learns the leave it began is done.
    h.sync(bo_phone).await;
    h.sync(bo_phone).await;
    assert!(h.node(bo_phone).is_departed(&obj).unwrap(), "the phone learns it too");
}

/// A member's forged commit removing the owner — built by a rogue client below every
/// door — is refused by every honest member (§4.3). It holds its epoch's commit slot,
/// so the group stalls there (§4.6): application messages flow, commits do not. The
/// rules turn a takeover into a stall; recovering from the stall is §17's residual.
#[tokio::test]
async fn a_forged_remove_of_the_owner_is_refused_by_every_member() {
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let (ada, bo, cy) = (0, 1, 2);
    let obj = h.form_forum(ada, &[bo, cy]).await;
    let e0 = h.epoch(ada, &obj);

    let ada_leaf = h.with_mls_group(bo, &obj, |g| mls::leaves_of(g, &h.id(ada))[0]);
    let forged = {
        let mut g = rogue_group!(h, bo, &obj);
        let out = g.commit_builder().remove_member(ada_leaf).unwrap().build().unwrap();
        out.commit_message.to_bytes().unwrap()
    };
    assert!(claim_slot(&h, bo, &obj, &forged).await, "the relay cannot judge it; it takes the slot");
    h.settle_among(&[ada, cy]).await;

    for u in [ada, cy] {
        assert_eq!(h.epoch(u, &obj), e0, "{} refused the commit", h.device_name(u));
        assert_eq!(people(&h, u, &obj), vec!["ada", "bo", "cy"]);
        assert_eq!(owner_seen_by(&h, u, &obj), h.id(ada), "ada still owns it");
        assert!(!h.node(u).is_departed(&obj).unwrap());
    }

    // The stall: no honest commit can land in a slot the forgery holds.
    let (o, m) = (obj.clone(), hex::encode(h.id(bo)));
    let e = h
        .with(ada, |n| async move { n.group_remove_member(&o, &m, None).await })
        .await
        .expect_err("the epoch's slot is held by a commit nobody honest can apply");
    assert!(e.to_string().contains("cannot process"), "{e}");
    // Application messages still flow at that epoch.
    h.obj_post(cy, &obj, "the group still talks").await;
    h.settle_among(&[ada, cy]).await;
    assert!(texts(&h, ada, &obj).contains(&"the group still talks".to_string()));
}

/// The joiner takes the owner from the Welcome's group context, never from the payload
/// (§8.5 step 1). bo — a member, not the owner — adds cy in a rogue commit that also
/// makes bo the owner, and tells cy in the payload that ada owns it. cy refuses the
/// Welcome and keeps none of it; ada refuses the commit.
#[tokio::test]
async fn a_joiner_takes_the_owner_from_the_context_not_the_payload() {
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let (ada, bo, cy) = (0, 1, 2);
    let obj = h.form_forum(ada, &[bo]).await;
    let bundle = handshake::parse_and_verify(&h.node(cy).build_contact_bundle().unwrap()).unwrap();

    let (commit, welcome) = add_and_seize(&h, bo, &obj, &bundle);
    assert!(claim_slot(&h, bo, &obj, &commit).await);
    send_welcome(&h, bo, &bundle, welcome, "forum", h.id(ada)).await;

    h.sync(cy).await;
    assert!(!holds_group(&h, cy, &obj), "cy refused the Welcome and persisted nothing of it");
    let q = h.node(cy).quarantined().unwrap();
    assert!(
        q.iter().any(|(_, _, why, _)| why.contains("context names")),
        "the refusal is quarantined with its reason: {q:?}"
    );

    h.settle_among(&[ada]).await;
    assert_eq!(owner_seen_by(&h, ada, &obj), h.id(ada), "ada refused the commit");
    assert_eq!(people(&h, ada, &obj), vec!["ada", "bo"]);
}

/// A knocker holds the one expectation the attack cannot forge: the owner printed on
/// the card it knocked on (§8.5 step 2). bo answers cy's knock with the same rogue
/// Add-and-seize, and this time the payload tells the consistent lie — bo owns it. cy
/// refuses, because the card said ada.
#[tokio::test]
async fn a_knocker_refuses_a_welcome_whose_owner_differs_from_the_card() {
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let (ada, bo, cy) = (0, 1, 2);
    let place = h.node(ada).place_mint("the venue", "", None).await.expect("mint");
    h.node(ada).place_set_access(&place, Access::Public).await.expect("open");
    h.node(ada).place_fit_doorbell(&place).await.expect("fit the doorbell");
    h.add_to_forum(ada, bo, &place).await;
    let card = h.node(ada).place_join_card(&place).expect("print the card");
    assert!(card.contains(&format!("owner={}", hex::encode(h.id(ada)))), "the card names ada");

    let c = card.clone();
    h.with(cy, |n| async move { n.place_knock_card(&c).await })
        .await
        .expect("cy knocks");
    // ada stays offline: an honest answer is not what is being tested.
    let bundle = handshake::parse_and_verify(&h.node(cy).build_contact_bundle().unwrap()).unwrap();
    let (commit, welcome) = add_and_seize(&h, bo, &place, &bundle);
    assert!(claim_slot(&h, bo, &place, &commit).await);
    send_welcome(&h, bo, &bundle, welcome, "place", h.id(bo)).await;

    h.sync(cy).await;
    assert!(!holds_group(&h, cy, &place), "cy refused the Welcome");
    let q = h.node(cy).quarantined().unwrap();
    assert!(
        q.iter().any(|(_, _, why, _)| why.contains("card")),
        "refused because of the card: {q:?}"
    );
}

/// A group made before this PR carries no owner in its context; its owner is its
/// leaf-0 creator (§3.3). Once every leaf can read the extension, the owner's next sync
/// writes itself into the context, and from then on the membership doors work.
#[tokio::test]
async fn a_legacy_group_migrates_its_owner_extension() {
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let obj = h.node(ada).object_new("forum", "old").unwrap();
    // Make it a legacy group: strip the owner and the floor from its context. Only a
    // rogue client can; the rules refuse it to every honest one.
    {
        let mut g = rogue_group!(h, ada, &obj);
        let mut exts = g.context().extensions().clone();
        exts.remove(mls::OWNER_EXT);
        exts.remove(mls::MIN_VERSION_EXT);
        g.commit_builder().set_group_context_ext(exts).unwrap().build().unwrap();
        g.apply_pending_commit().unwrap();
        g.write_to_storage().unwrap();
    }
    assert!(!has_owner_ext(&h, ada, &obj));
    assert_eq!(owner_seen_by(&h, ada, &obj), h.id(ada), "leaf 0 is the owner");

    // bo joins; ada has not synced since, so has not migrated yet.
    let bundle = h.node(bo).build_contact_bundle().unwrap();
    h.node(ada).group_add_member(&obj, &bundle).await.expect("the add");
    h.sync(bo).await;
    assert!(!has_owner_ext(&h, bo, &obj));
    assert_eq!(owner_seen_by(&h, bo, &obj), h.id(ada), "bo reads the same legacy owner");
    let o = obj.clone();
    let e = h
        .with(bo, |n| async move { n.group_leave(&o).await })
        .await
        .expect_err("no leave before the migration");
    assert!(e.to_string().contains("predates the owner extension"), "{e}");

    h.settle().await;
    for u in [ada, bo] {
        assert!(has_owner_ext(&h, u, &obj), "{} — migrated", h.device_name(u));
        assert_eq!(owner_seen_by(&h, u, &obj), h.id(ada));
    }
    leave(&h, bo, &obj).await;
    h.settle().await;
    assert!(h.node(bo).is_departed(&obj).unwrap(), "and the doors work");
}

/// After a handover the new owner writes the owner-sequenced spine, and every device
/// folds both owners' deltas, each in its own epochs (§10.2). The old owner can no
/// longer write it, and its leave is recorded and completed.
#[tokio::test]
async fn the_new_owner_writes_the_spine_after_a_handover() {
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let (ada, bo, cy) = (0, 1, 2);
    let obj = h.mint_group(ada);
    h.add_to_forum(ada, bo, &obj).await;
    h.add_to_forum(ada, cy, &obj).await;
    h.group_set_profile(ada, &obj, "ADA'S", "team").await;
    h.settle().await;

    let (o, m) = (obj.clone(), hex::encode(h.id(bo)));
    h.with(ada, |n| async move { n.group_hand_over(&o, &m).await })
        .await
        .expect("the handover");
    h.settle().await;
    h.group_set_profile(bo, &obj, "BO'S", "team").await;
    h.settle().await;
    for u in [ada, bo, cy] {
        assert_eq!(h.group_view(u, &obj).display_name, "BO'S", "{}", h.device_name(u));
    }

    let o = obj.clone();
    let mut a = Args::new();
    a.insert("displayName".into(), ArgVal::Text("ADA'S AGAIN".into()));
    a.insert("shape".into(), ArgVal::Text("team".into()));
    h.with(ada, |n| async move { n.apply(&o, OP_SET_PROFILE, a).await })
        .await
        .expect_err("the old owner no longer writes the spine");

    leave(&h, ada, &obj).await;
    h.settle().await;
    assert!(h.node(ada).is_departed(&obj).unwrap());
    let st = h.node(bo).group_state(&obj).unwrap();
    assert!(st.membership.departed(&hex::encode(h.id(ada))).is_some(), "ada's leave is recorded");
    assert_eq!(st.membership.handovers.last().map(|x| x.to.clone()), Some(hex::encode(h.id(bo))));
    assert_compliant(&h, &[ada, bo, cy]);
}

/// An owner-approval proposal is judged by the owner who CLOSED it (§10.4). Handing
/// the object over afterwards does not re-judge it by the new owner, who never voted.
#[tokio::test]
async fn a_handover_does_not_rejudge_a_closed_owner_approval() {
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let (ada, bo, cy) = (0, 1, 2);
    let obj = h.form_forum(ada, &[bo, cy]).await;
    let p = h.obj_propose(ada, &obj, "open on sundays", Rule::OwnerApproval).await;
    h.obj_vote(ada, &obj, p.clone(), Ballot::Approve).await;
    h.settle().await;
    h.obj_close(ada, &obj, p).await;
    h.settle().await;
    for u in [ada, bo, cy] {
        assert_eq!(h.obj_ratify(u, &obj)[0].outcome, "passed", "{}", h.device_name(u));
    }

    let (o, m) = (obj.clone(), hex::encode(h.id(bo)));
    h.with(ada, |n| async move { n.group_hand_over(&o, &m).await })
        .await
        .expect("the handover");
    h.settle().await;
    for u in [ada, bo, cy] {
        assert_eq!(h.obj_ratify(u, &obj)[0].outcome, "passed", "{} — still ada's decision", h.device_name(u));
    }
}
