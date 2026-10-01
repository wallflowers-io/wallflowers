//! A-10, SEC-35: A HOSTILE MEMBER'S DELTA TAKES NO EFFECT, AND IS NAMED, ON EVERY NODE.
//!
//! A member holds the group's MLS keys, so it can put any bytes on the relay the group
//! reads. MLS proves which LEAF sent them; it does not prove who wrote the Delta
//! (CS-32). A-10, ruled (a), 26 Sep: every Delta travels and is stored with its
//! author's signature, and a Node verifies it before the Delta takes effect (SEC-35;
//! srr/vv.md: "the others refused before they take effect, and named").
//!
//! RED UNTIL A-10 LANDS (Ralph, D-11: a test for an unbuilt requirement is red, and
//! committed). Today the MLS payload is the envelope alone, a peer's Delta is stored
//! with no signature (`Directory::append_delta`), and no fold reads one (CS-31).
//!
//! THE HOSTILE DELTA COMES THROUGH THE ONE DOOR, `authoring::build`, from the log the
//! member holds, so it is well formed in every way but its signature: the test does
//! not write a Delta by hand. It is a `forum.post`, an op any member may write, so
//! the only thing that can refuse it is the signature.
//!
//! The cases, and where each is put:
//!   · UNSIGNED, at the relay: the envelope alone, sealed and published under the
//!     member's own MLS state as `flush_one` does. Every other Node drains it.
//!   · SIGNED OVER OTHER CONTENT and SIGNED BY ANOTHER KEY, at the relay: A-10 part (1)'s
//!     payload, `{1: envelope, 2: author, 3: sig}`, made by core's own
//!     `delta_sig::seal_payload`, claiming Bo as its author.
//!   · The same two, at rest: stored in a Node's log with those signatures.
//! And the control for the relay cases: the member's own post, signed by it, through
//! the same path, takes effect. Without it a refusal could be the test's own payload.
//!
//! NAMED, in words that mention the signature, where the design puts the refusal
//! (Software Engineering, 26 Sep): a payload refused at ingest never enters the log,
//! so it is named in the Node's quarantine (`Directory::quarantine_list`); a Delta
//! already in the log is the fold's, named by `Node::noncompliant_objects()`.

mod common;

use common::Harness;
use pacific_core::authoring::{self, Ctx};
use pacific_core::coordinator::{decode_delta, ArgVal, Args, Delta};
use pacific_core::object::MemberId;
use pacific_core::transport::RelaySession;
use pacific_core::{delta_sig, mls, paths, seal};
use serde_json::Value;

const FORGED: &str = "posted in someone else's name";

/// `forum.post`'s op id, read from the ICD rather than written here.
fn post_op() -> u32 {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json");
    let icd: Value = serde_json::from_str(&std::fs::read_to_string(path).expect("the ICD is readable")).expect("the ICD is JSON");
    icd["kinds"]["forum"]["ops"]["forum.post"]["op"].as_u64().expect("the ICD declares forum.post") as u32
}

/// A post by `author`, built through `authoring::build` from the log device `at` holds.
fn post_as(h: &Harness, at: usize, author: MemberId, obj: &str, text: &str) -> Delta {
    let epoch = h.epoch(at, obj);
    let node = h.node(at);
    let gid = hex::decode(obj).unwrap();
    let kind = node.dir.group_kind(&gid).unwrap().expect("the forum is held here");
    let members = node.dir.group_members(&gid).unwrap();
    let owners = node.dir.owner_history(&gid).unwrap();
    let log: Vec<(Delta, MemberId)> = node
        .dir
        .load_log(&gid)
        .unwrap()
        .into_iter()
        .filter_map(|(a, env)| decode_delta(&env).ok().map(|d| (d, a)))
        .collect();
    let ctx = Ctx {
        me: author,
        epoch,
        members: &members,
        owners: &owners,
        log: &log,
        watermark: Some(node.dir.gen_floor(&gid).unwrap()),
    };
    let mut args = Args::new();
    args.insert("text".into(), ArgVal::Text(text.into()));
    authoring::build(&kind, post_op(), args, &ctx).expect("the one door builds a member's post")
}

/// Device `u` puts `payload` on the relay as its own application message: encrypted
/// under its MLS state, sealed, published to the group's address for the epoch.
/// `Node::flush_one`'s steps, minus the Node and whatever it would have added.
async fn publish_raw(h: &Harness, u: usize, obj: &str, envelope: &[u8]) {
    let node = h.node(u);
    let (sk, pk) = node.dir.mls_signing_keypair().expect("an MLS signing key");
    let sid = mls::signing_identity(&node.id.identity_pk(), &pk);
    let client = mls::build_client_sqlite(&paths::db_path(), sid, mls::SecretKey::new(sk)).unwrap();
    let mut group = mls::load_group(&client, &hex::decode(obj).unwrap()).unwrap();
    let inner = mls::encrypt_delta(&mut group, envelope).unwrap();
    let epoch = mls::epoch_be(group.current_epoch());
    let secret = mls::seal_conn_secret(&group, &epoch).unwrap();
    let tag = mls::group_tag(&group, &epoch).unwrap();
    let addr = mls::group_address(&group, &epoch).unwrap();
    let sealed = seal::seal(&inner, &tag, &secret).unwrap();
    let mut relay = RelaySession::connect(&h.relay).await.unwrap();
    relay.publish(&addr, pacific_wire::blob_b64(&sealed)).await.expect("the relay takes a member's publish");
}

/// On device `u`: the forged text did not take effect.
fn no_effect(h: &Harness, u: usize, obj: &str, case: &str) {
    let who = h.device_name(u);
    if let Ok(view) = h.node(u).object_detailed(obj) {
        assert!(
            !view.iter().any(|m| m.text == FORGED),
            "{case}: the post took effect on {who}; a Delta whose signature does not prove \
             its author must be refused before it takes effect (SEC-35, A-10)"
        );
    }
}

/// On device `u`: refused at ingest, and the quarantine names the object's group with a
/// reason about the signature.
fn refused_at_ingest_and_named(h: &Harness, u: usize, obj: &str, case: &str) {
    no_effect(h, u, obj, case);
    let who = h.device_name(u);
    let gid = hex::decode(obj).unwrap();
    let held = h.node(u).dir.quarantine_list().unwrap();
    let named = held.iter().find(|(g, ..)| g.as_deref() == Some(&gid[..]));
    assert!(named.is_some(), "{case}: {who}'s quarantine does not name the forum: {held:?}");
    let why = named.unwrap().2.to_lowercase();
    assert!(why.contains("sign"), "{case}: {who} quarantined it, but not for the signature: {why}");
}

/// On device `u`: refused at fold, and `noncompliant_objects` names the object with a
/// reason about the signature.
fn refused_at_fold_and_named(h: &Harness, u: usize, obj: &str, case: &str) {
    no_effect(h, u, obj, case);
    let who = h.device_name(u);
    let bad = h.node(u).noncompliant_objects().unwrap();
    let named = bad.iter().find(|b| b.object_id == obj);
    assert!(named.is_some(), "{case}: {who} does not name the forum in noncompliant_objects: {bad:?}");
    let why = named.unwrap().reason.to_lowercase();
    assert!(why.contains("sign"), "{case}: {who} names the forum, but not the signature: {why}");
}

/// Ada owns a forum; Bo and Mal are members; Bo's honest post reaches everyone, and
/// every Node can reduce all it holds. Returns the forum.
async fn forum_with_an_honest_post(h: &Harness) -> String {
    let (ada, bo, mal) = (0, 1, 2);
    let forum = h.form_forum(ada, &[bo, mal]).await;
    h.settle().await;
    h.obj_post(bo, &forum, "an honest post").await;
    h.settle().await;
    for u in [ada, bo, mal] {
        assert!(
            h.obj_view(u, &forum).iter().any(|m| m.text == "an honest post"),
            "the control: Bo's own post reaches {} — if not, the fixture is wrong, not A-10",
            h.device_name(u)
        );
        assert!(h.node(u).noncompliant_objects().unwrap().is_empty(), "the control: everything folds");
    }
    forum
}

#[tokio::test]
async fn an_unsigned_delta_a_member_puts_on_the_relay_is_refused_and_named_on_every_node() {
    let h = Harness::new(&["ada", "bo", "mal"]).await;
    let (ada, bo, mal) = (0, 1, 2);
    let forum = forum_with_an_honest_post(&h).await;

    let forged = post_as(&h, mal, h.id(mal), &forum, FORGED);
    publish_raw(&h, mal, &forum, &forged.canonical_bytes()).await;
    h.sync(ada).await;
    h.sync(bo).await;

    for u in [ada, bo] {
        refused_at_ingest_and_named(&h, u, &forum, "unsigned, at the relay");
    }
}

#[tokio::test]
async fn a_delta_stored_with_its_authors_signature_over_other_content_is_refused_and_named() {
    let h = Harness::new(&["ada", "bo", "mal"]).await;
    let (ada, bo) = (0, 1);
    let forum = forum_with_an_honest_post(&h).await;
    let gid = hex::decode(&forum).unwrap();

    let forged = post_as(&h, ada, h.id(bo), &forum, FORGED);
    let sig = delta_sig::sign_delta(&h.node(bo).id, &gid, &[7u8; 32]);
    let stored = h
        .node(ada)
        .dir
        .append_delta_signed(&gid, &forged.id(), &h.id(bo), &forged.canonical_bytes(), Some(&sig), 1_790_000_000_000)
        .unwrap();
    assert!(stored, "the fixture: the Delta is in Ada's log");

    refused_at_fold_and_named(&h, ada, &forum, "signed over other content, at rest");
}

#[tokio::test]
async fn a_delta_stored_signed_by_a_key_other_than_its_authors_is_refused_and_named() {
    let h = Harness::new(&["ada", "bo", "mal"]).await;
    let (ada, bo, mal) = (0, 1, 2);
    let forum = forum_with_an_honest_post(&h).await;
    let gid = hex::decode(&forum).unwrap();

    let forged = post_as(&h, ada, h.id(bo), &forum, FORGED);
    let sig = delta_sig::sign_delta(&h.node(mal).id, &gid, &forged.id());
    let stored = h
        .node(ada)
        .dir
        .append_delta_signed(&gid, &forged.id(), &h.id(bo), &forged.canonical_bytes(), Some(&sig), 1_790_000_000_000)
        .unwrap();
    assert!(stored, "the fixture: the Delta is in Ada's log");

    refused_at_fold_and_named(&h, ada, &forum, "signed by another key, at rest");
}

/// Mal sends `payload` through the relay; Ada and Bo drain it.
async fn through_the_relay(h: &Harness, obj: &str, payload: &[u8]) {
    let (ada, bo, mal) = (0, 1, 2);
    publish_raw(h, mal, obj, payload).await;
    h.sync(ada).await;
    h.sync(bo).await;
}

#[tokio::test]
async fn a_members_own_signed_post_through_the_relay_takes_effect() {
    // THE CONTROL for the two cases below: the same path, the same encoder, a true
    // signature. If this fails, a refusal below proves nothing about signatures.
    let h = Harness::new(&["ada", "bo", "mal"]).await;
    let (ada, bo, mal) = (0, 1, 2);
    let forum = forum_with_an_honest_post(&h).await;
    let gid = hex::decode(&forum).unwrap();
    let own = post_as(&h, mal, h.id(mal), &forum, "Mal's own, signed");
    let sig = delta_sig::sign_delta(&h.node(mal).id, &gid, &own.id());
    through_the_relay(&h, &forum, &delta_sig::seal_payload(&own.canonical_bytes(), &h.id(mal), &sig)).await;
    for u in [ada, bo] {
        let view = h.obj_view(u, &forum);
        let got = view.iter().find(|m| m.text == "Mal's own, signed");
        assert!(
            got.is_some(),
            "the control: a member's own signed post, sent as the test sends the others, did not reach {}",
            h.device_name(u)
        );
        assert_eq!(
            got.unwrap().author,
            h.id(mal),
            "{} credits the post to someone other than its signer (NC-21: the author is the signer, not the leaf)",
            h.device_name(u)
        );
    }
}

#[tokio::test]
async fn a_delta_through_the_relay_with_its_authors_signature_over_other_content_is_refused_and_named() {
    let h = Harness::new(&["ada", "bo", "mal"]).await;
    let (ada, bo, mal) = (0, 1, 2);
    let forum = forum_with_an_honest_post(&h).await;
    let gid = hex::decode(&forum).unwrap();
    let forged = post_as(&h, mal, h.id(bo), &forum, FORGED);
    let sig = delta_sig::sign_delta(&h.node(bo).id, &gid, &[7u8; 32]);
    through_the_relay(&h, &forum, &delta_sig::seal_payload(&forged.canonical_bytes(), &h.id(bo), &sig)).await;
    for u in [ada, bo] {
        refused_at_ingest_and_named(&h, u, &forum, "signed over other content, at the relay");
    }
}

#[tokio::test]
async fn a_delta_through_the_relay_signed_by_a_key_other_than_its_authors_is_refused_and_named() {
    let h = Harness::new(&["ada", "bo", "mal"]).await;
    let (ada, bo, mal) = (0, 1, 2);
    let forum = forum_with_an_honest_post(&h).await;
    let gid = hex::decode(&forum).unwrap();
    let forged = post_as(&h, mal, h.id(bo), &forum, FORGED);
    let sig = delta_sig::sign_delta(&h.node(mal).id, &gid, &forged.id());
    through_the_relay(&h, &forum, &delta_sig::seal_payload(&forged.canonical_bytes(), &h.id(bo), &sig)).await;
    for u in [ada, bo] {
        refused_at_ingest_and_named(&h, u, &forum, "signed by another key, at the relay");
    }
}

/// The leaf `who`'s device sends from in `obj`, as device `u` sees the tree: the leaf
/// whose signature key is that device's own MLS signing key.
fn device_leaf(h: &Harness, u: usize, obj: &str, who: usize) -> u32 {
    let (_, key) = h.node(who).dir.mls_signing_keypair().unwrap();
    h.with_mls_group(u, obj, |g| {
        g.roster().members().iter().find(|m| m.signing_identity.signature_key.as_bytes() == key.as_slice()).map(|m| m.index)
    })
    .expect("the device is a leaf of the group")
}

/// Who holds leaf `leaf` in `obj`, as device `u` sees the tree.
fn holder(h: &Harness, u: usize, obj: &str, leaf: u32) -> Option<[u8; 32]> {
    h.with_mls_group(u, obj, |g| {
        g.roster().members().iter().find(|m| m.index == leaf).and_then(|m| {
            m.signing_identity.credential.as_basic().and_then(|b| <[u8; 32]>::try_from(b.identifier.as_slice()).ok())
        })
    })
}

/// NC-21 (CS-32): a message from an earlier epoch, arriving after its sender's leaf was
/// refilled, is never credited to the leaf's new holder. mls-rs 0.55.4 refuses it before
/// it is ever decrypted into a Delta: `validate_sender_signature_key_from_prior_epoch`
/// (group/util.rs:225–249) compares the leaf's key in the message's epoch with the key it
/// holds now and returns MemberNotFound, "instead of mis-attributing it"; its own test
/// (group/mod.rs:3595–3620) is this case. So the message takes no effect anywhere, and
/// the refusal is named (RX.7, RX.8). Losing it is O-42, parked.
///
/// Bo writes in epoch e and his message is held back: it goes out late, under his own
/// epoch-e state, signed as any Node signs. By then Ada has removed Bo and added Dee, who
/// takes Bo's leaf, within EPOCH_RETENTION. Cy, online throughout, drains the old epoch.
#[tokio::test]
async fn a_late_message_from_a_refilled_leaf_is_never_credited_to_its_new_holder_and_is_named() {
    let h = Harness::new(&["ada", "bo", "cy", "dee"]).await;
    let (ada, bo, cy, dee) = (0, 1, 2, 3);
    let forum = h.form_forum(ada, &[bo, cy]).await;
    h.settle().await;
    let gid = hex::decode(&forum).unwrap();
    let e = h.epoch(bo, &forum);
    let leaf = device_leaf(&h, ada, &forum, bo);

    // Bo's message, written in epoch e and not yet sent.
    let late = post_as(&h, bo, h.id(bo), &forum, "Bo, before he was removed");
    let sig = delta_sig::sign_delta(&h.node(bo).id, &gid, &late.id());
    let payload = delta_sig::seal_payload(&late.canonical_bytes(), &h.id(bo), &sig);

    // Bo is removed and Dee added, with Bo's device never syncing: it keeps epoch e.
    let (o, m) = (forum.clone(), hex::encode(h.id(bo)));
    h.with(ada, |n| async move { n.group_remove_member(&o, &m, None).await }).await.expect("the owner's remove");
    let bundle = h.node(dee).build_contact_bundle().unwrap();
    h.node(ada).group_add_member(&forum, &bundle).await.unwrap();
    h.node(dee).sync_once().await.unwrap();
    h.settle_among(&[ada, cy, dee]).await;
    assert_eq!(
        holder(&h, cy, &forum, leaf),
        Some(h.id(dee)),
        "the fixture: Dee must hold the leaf Bo's device sent from, or the case is not the one NC-21 names"
    );
    let now = h.epoch(cy, &forum);
    assert!(now > e && now - e <= pacific_core::EPOCH_RETENTION, "the fixture: epoch {e} to {now} is inside the retention");

    // Bo's message goes out late, and Cy drains.
    publish_raw(&h, bo, &forum, &payload).await;
    h.sync(cy).await;

    let node = h.node(cy);
    let view = h.obj_view(cy, &forum);
    assert!(
        !view.iter().any(|m| m.text == "Bo, before he was removed" && m.author == h.id(dee)),
        "Bo's message from epoch {e}, arriving after Dee took his leaf, is credited to Dee (NC-21)"
    );
    assert!(
        !view.iter().any(|m| m.text == "Bo, before he was removed"),
        "Bo's late message took effect on Cy: a message from a refilled leaf must be refused, not folded"
    );
    let logged = node.dir.load_log(&gid).unwrap().into_iter()
        .filter_map(|(_, env)| decode_delta(&env).ok()).any(|d| d.id() == late.id());
    assert!(!logged, "Bo's late message is in Cy's log");
    let held: Vec<String> = node.dir.quarantine_list().unwrap().into_iter()
        .filter(|(g, ..)| g.as_deref() == Some(&gid[..])).map(|(_, _, why, _)| why).collect();
    assert!(
        held.iter().any(|why| why.contains("No member found")),
        "Cy refused Bo's late message without naming why (MemberNotFound), or did not refuse it: {held:?}"
    );
}
