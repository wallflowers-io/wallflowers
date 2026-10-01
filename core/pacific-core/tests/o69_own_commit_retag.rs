//! O-69, the held connection after the reader's OWN commit (Software Engineering's BW-A,
//! 28 Sep, at 1e5fc26): a member who commits (here the owner adding the Arc's node) moves
//! the group to a new epoch. The held connection must listen on that epoch's tag, or what
//! others say there (the Arc admitting a visitor, a member's post) is heard only at the
//! reconciler or the reader's next write. The reader here never writes after its commit and
//! must hold both within one tick (3 s).

mod common;

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use common::Harness;
use ed25519_dalek::Signer;
use pacific_core::coordinator::{ArgVal, Args, FORUM_POST};
use pacific_core::object::ObjectKind;
use pacific_core::Node;
use std::collections::HashMap;
use std::time::{Duration, Instant};

fn kiosk() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[9u8; 32])
}

/// A claim as the kiosk issues one, for `site`, with nonce `n`.
fn claim(site: &str, n: u8) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let pk = kiosk().verifying_key().to_bytes();
    let payload = format!(
        r#"{{"k":"{}","s":"{site}","n":"{}","iat":{now},"exp":{},"c":"skills"}}"#,
        pacific_core::claim::kid_of(&pk),
        B64.encode([n; 16]),
        now + 600
    );
    let p = B64.encode(payload);
    let sig = kiosk().sign(format!("v1.{p}").as_bytes());
    format!("v1.{p}.{}", B64.encode(sig.to_bytes()))
}

fn args(pairs: &[(&str, &str)]) -> Args {
    pairs.iter().map(|(k, v)| (k.to_string(), ArgVal::Text(v.to_string()))).collect()
}

/// The reader's own node, held: its connection lives in it; pointed at before each use.
struct Reader<'a> {
    h: &'a Harness,
    n: Node,
}

impl Reader<'_> {
    fn at(&self) -> &Node {
        let _ = self.h.node(0);
        &self.n
    }

    /// As the Door's bell does, for one tick: whatever is delivered, ingested; whatever the
    /// connection cannot vouch for, drained on the bell.
    async fn one_tick(&self, holds: impl Fn(&Node) -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(3);
        while Instant::now() < end {
            self.at().ingest_delivered().unwrap();
            if self.at().needs_catch_up().unwrap() {
                self.at().catch_up().await.unwrap();
            }
            if holds(self.at()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

#[tokio::test(flavor = "multi_thread")]
async fn after_its_own_commit_a_reader_that_never_writes_hears_the_arcs_add_and_a_post() {
    std::env::set_var("PACIFIC_FOLD_CACHE_MODEL", "o69-retag");
    std::env::set_var("PACIFIC_FOLD_CACHE_VERIFY", "1");
    let h = Harness::new(&["ada", "arc", "bo", "vis"]).await;
    let (arc, bo, vis) = (1, 2, 3);
    // A Site and its room, both halves of the edge, bo in both.
    let site = h.mint_object(0, ObjectKind::Group, "Egregore", &[bo]).await;
    let room = h.mint_object(0, ObjectKind::Forum, "Talk", &[bo]).await;
    h.node(0).apply(&site, pacific_core::parts::OP_SET_PART, pacific_core::parts::set_part_args(&room, "room", now_ms())).await.unwrap();
    h.node(0).apply(&room, pacific_core::parent::OP_SET_PARENT, pacific_core::parent::set_parent_args(&site, "room", now_ms())).await.unwrap();
    h.settle().await;

    // The reader, live and caught up, before anything below.
    let ada = Reader { h: &h, n: h.node(0) };
    assert!(ada.at().go_live(Box::new(|| {}), Duration::from_secs(10)).unwrap());
    ada.at().sync_once().await.unwrap();

    // THE READER'S OWN COMMITS: the owner adds the Arc's node to the Site and the room, then
    // grants it the admitter role in both and registers the kiosk. Her last writes.
    for o in [&site, &room] {
        let bundle = h.node(arc).build_contact_bundle().unwrap();
        ada.at().group_add_member(o, &bundle).await.expect("the owner adds the Arc");
        h.node(arc).sync_once().await.unwrap();
    }
    for o in [&site, &room] {
        ada.at()
            .apply(o, pacific_core::roles::OP_SET_ROLE, args(&[("member", &hex::encode(h.id(arc))), ("role", "admitter")]))
            .await
            .expect("the admitter role");
    }
    let pk = kiosk().verifying_key().to_bytes();
    ada.at()
        .apply(&site, pacific_core::group::OP_SET_CLAIM_ISSUER, args(&[("kid", &pacific_core::claim::kid_of(&pk)), ("key", &B64.encode(pk))]))
        .await
        .expect("the kiosk's key");
    let members = |n: &Node| n.object_members(&room).map(|m| m.len()).unwrap_or(0);
    assert!(ada.one_tick(|n| members(n) == 3).await, "the control: the reader holds its own commit");

    // Nothing more from the reader. The Arc admits a visitor to the Site and the room (its
    // commits, each at a next epoch), and bo posts in the room.
    h.node(arc).sync_once().await.unwrap();
    let packages = vec![h.node(vis).build_contact_bundle().unwrap(), h.node(vis).build_contact_bundle().unwrap()];
    h.node(arc).admit_by_claim(&site, &claim(&site, 1), &packages, &HashMap::new()).await.expect("the Arc admits the visitor");
    h.node(bo).sync_once().await.unwrap();
    let mut post = Args::new();
    post.insert("text".into(), ArgVal::Text("said after the Arc's add".into()));
    h.node(bo).apply(&room, FORUM_POST, post).await.expect("bo posts");

    assert!(ada.one_tick(|n| members(n) == 4).await, "within one tick, the reader holds the Arc's Add ({} members)", members(ada.at()));
    assert!(
        ada.one_tick(|n| n.object_view(&room).is_ok_and(|v| v.contains("said after the Arc's add"))).await,
        "and bo's post at the new epoch"
    );
}
