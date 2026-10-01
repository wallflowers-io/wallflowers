//! A-10, SEC-35: A DELTA'S AUTHOR IS SOMEONE THE GROUP ADMITTED.
//!
//! A-10 part 1 (45e1e53) takes a Delta's author from its signature, not from the MLS
//! leaf that sent it, and folds a commutative Delta without asking whether its author
//! was a member: "MLS answered 'was a member when they wrote it' at ingest"
//! (`coordinator.rs`, `deliver`). With the author taken from the signature, MLS no
//! longer answers it. A member can sign a Delta with any key it makes, and becomes as
//! many authors as it likes: one vote, one reaction, one RSVP each.
//!
//! The cases, both put on the relay by Mal, a member, through `flush_one`'s steps:
//!   · a post signed by a key the group never admitted;
//!   · a post "signed" by the identity point, a small-order key for which one fixed
//!     64 bytes verify over every message unless verification is strict.
//! Each must take no effect, and be named, on every other Node. The control is
//! `a10_hostile_peer`'s: Mal's own signed post through the same path takes effect.
//!
//! Red at 45e1e53: both posts took effect on Ada (NC-45, NC-46). Green from f034c75
//! (the author is the sending leaf) and bb07a71 (`verify_strict`).

mod common;

use common::Harness;
use pacific_core::authoring::{self, Ctx};
use pacific_core::coordinator::{decode_delta, ArgVal, Args, Delta};
use pacific_core::identity::Identity;
use pacific_core::object::MemberId;
use pacific_core::transport::RelaySession;
use pacific_core::{delta_sig, mls, paths, seal};
use serde_json::Value;

const FORGED: &str = "posted by nobody the forum admitted";

/// `forum.post`'s op id, read from the ICD.
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
    authoring::build(&kind, post_op(), args, &ctx).expect("the one door builds a post")
}

/// Device `u` puts `payload` on the relay as its own application message.
async fn publish_raw(h: &Harness, u: usize, obj: &str, payload: &[u8]) {
    let node = h.node(u);
    let (sk, pk) = node.dir.mls_signing_keypair().expect("an MLS signing key");
    let sid = mls::signing_identity(&node.id.identity_pk(), &pk);
    let client = mls::build_client_sqlite(&paths::db_path(), sid, mls::SecretKey::new(sk)).unwrap();
    let mut group = mls::load_group(&client, &hex::decode(obj).unwrap()).unwrap();
    let inner = mls::encrypt_delta(&mut group, payload).unwrap();
    let epoch = mls::epoch_be(group.current_epoch());
    let secret = mls::seal_conn_secret(&group, &epoch).unwrap();
    let tag = mls::group_tag(&group, &epoch).unwrap();
    let addr = mls::group_address(&group, &epoch).unwrap();
    let sealed = seal::seal(&inner, &tag, &secret).unwrap();
    let mut relay = RelaySession::connect(&h.relay).await.unwrap();
    relay.publish(&addr, pacific_wire::blob_b64(&sealed)).await.expect("the relay takes a member's publish");
}

/// Ada owns a forum with Bo and Mal; Bo's honest post reaches everyone.
async fn forum(h: &Harness) -> String {
    let (ada, bo, mal) = (0, 1, 2);
    let forum = h.form_forum(ada, &[bo, mal]).await;
    h.settle().await;
    h.obj_post(bo, &forum, "an honest post").await;
    h.settle().await;
    for u in [ada, bo, mal] {
        assert!(h.obj_view(u, &forum).iter().any(|m| m.text == "an honest post"), "the control: Bo's post reaches {}", h.device_name(u));
    }
    forum
}

/// On Ada and Bo: the post took no effect, and something names the forum: the
/// quarantine, if it was refused at ingest, or `noncompliant_objects`, at fold.
fn refused_and_named(h: &Harness, obj: &str, case: &str) {
    let gid = hex::decode(obj).unwrap();
    for u in [0, 1] {
        let who = h.device_name(u);
        if let Ok(view) = h.node(u).object_detailed(obj) {
            assert!(!view.iter().any(|m| m.text == FORGED), "{case}: the post took effect on {who} (SEC-35, A-10)");
        }
        let held = h.node(u).dir.quarantine_list().unwrap();
        let quarantined = held.iter().any(|(g, ..)| g.as_deref() == Some(&gid[..]));
        let noncompliant = h.node(u).noncompliant_objects().unwrap().iter().any(|b| b.object_id == obj);
        assert!(quarantined || noncompliant, "{case}: {who} names the forum nowhere: quarantine {held:?}");
    }
}

#[tokio::test]
async fn a_post_signed_by_a_key_the_group_never_admitted_is_refused_and_named() {
    let h = Harness::new(&["ada", "bo", "mal"]).await;
    let forum = forum(&h).await;
    let gid = hex::decode(&forum).unwrap();
    let stranger = Identity::in_memory([0x5a; 32]);
    let forged = post_as(&h, 2, stranger.identity_pk(), &forum, FORGED);
    let sig = delta_sig::sign_delta(&stranger, &gid, &forged.id());
    publish_raw(&h, 2, &forum, &delta_sig::seal_payload(&forged.canonical_bytes(), &stranger.identity_pk(), &sig)).await;
    h.sync(0).await;
    h.sync(1).await;
    refused_and_named(&h, &forum, "a key the group never admitted");
}

#[tokio::test]
async fn a_post_signed_by_the_identity_point_is_refused_and_named() {
    let h = Harness::new(&["ada", "bo", "mal"]).await;
    let forum = forum(&h).await;
    // The identity point, encoded; and R = the identity, s = 0, which satisfies
    // [s]B = R + [k]A for every k, so every message.
    let mut weak: MemberId = [0u8; 32];
    weak[0] = 1;
    let mut sig = [0u8; 64];
    sig[0] = 1;
    let forged = post_as(&h, 2, weak, &forum, FORGED);
    publish_raw(&h, 2, &forum, &delta_sig::seal_payload(&forged.canonical_bytes(), &weak, &sig)).await;
    h.sync(0).await;
    h.sync(1).await;
    refused_and_named(&h, &forum, "the identity point");
}
