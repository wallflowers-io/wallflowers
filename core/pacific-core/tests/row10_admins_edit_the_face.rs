//! ICD 2.1.0 ROW 10: THE OWNER AND AN ADMIN EDIT THE FACE (Ralph, 28 Sep: "For owners and
//! admins, this includes an option to edit the site face").
//!
//! `group.setFace` and `host.setMedia` are sequenced, and the sequenced spine takes the
//! owner alone (`SequencedCoordinator::deliver`), so an admin cannot write them however
//! their authority reads. The shared face is commutative: `group.editFace` on the Site
//! and `host.editMedia` on its Host, ego `owner|role:admin`, one LWW register by
//! (gen, author) (per slot for media), checked against the object's OWN roles. The face
//! is the latest counting shared write (the owner's always count; an admin's while they
//! hold admin); with none counting, the sequenced op. A revoked admin's writes stop
//! counting (R1, BUILD 29 Sep), which is also the answer to a hostile max-gen write.
//!
//! Ops are resolved by NAME from the catalogue the ICD holds the code to, so before the
//! code declares them each test here fails naming the op, rather than failing to build.

mod common;

use common::Harness;
use pacific_core::coordinator::{self, ArgVal, Args, Coordinator};
use pacific_core::group::{GroupType, OP_SET_FACE};
use pacific_core::host::{HostType, OP_SET_MEDIA};
use pacific_core::object::{build_delta, DeltaRejection, ObjectKind, ObjectType};
use pacific_core::roles::{OP_CLEAR_ROLE, OP_SET_ROLE};

const OWNER: [u8; 32] = [1; 32];
const ADMIN: [u8; 32] = [2; 32];
const MEMBER: [u8; 32] = [3; 32];
const HOST: &str = "4444444444444444444444444444444444444444444444444444444444444444";
/// A 1×1 png.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
const WEBP: &str = "UklGRhoAAABXRUJQVlA4TA0AAAAvAAAAEAcQERGIiP4HAA==";

/// The op `name` on `kind`, as the catalogue declares it.
fn op(kind: &str, name: &str) -> u32 {
    pacific_core::authoring::op_on(kind, name)
        .unwrap_or_else(|| panic!("{name} is not declared on {kind} (ICD 2.1.0 row 10)"))
        .op_id
}

fn args(kv: &[(&str, ArgVal)]) -> Args {
    kv.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}
fn t(s: &str) -> ArgVal {
    ArgVal::Text(s.into())
}

/// One object's log, folded as every replica folds it: the owner's sequenced spine,
/// hash-chained, and commutative deltas by whoever wrote them, with the gen they carry.
struct Log<T: ObjectType> {
    c: Coordinator<T>,
    kind: ObjectKind,
    epoch: u64,
}

impl<T: ObjectType> Log<T> {
    fn new(kind: ObjectKind, roster: &[[u8; 32]]) -> Self {
        Self { c: Coordinator::<T>::new(roster.to_vec(), OWNER), kind, epoch: 1 }
    }
    /// A sequenced delta by `author`; the spine's answer, so a refusal can be pinned.
    fn seq_by(&mut self, author: [u8; 32], op_id: u32, a: Args) -> Result<bool, DeltaRejection> {
        let (seq, prev) = coordinator::next_sequenced_pos(self.c.sequenced_head(), self.epoch);
        let d = coordinator::sequenced_delta(self.kind.type_id() as u32, op_id, a, self.epoch, seq, prev);
        self.c.deliver(d, author)
    }
    fn seq(&mut self, op_id: u32, a: Args) {
        self.seq_by(OWNER, op_id, a).expect("the owner sequences the spine");
    }
    fn comm(&mut self, author: [u8; 32], op_id: u32, mut a: Args, gen: u64) {
        a.insert("gen".into(), ArgVal::Int(gen as i64));
        let _ = self.c.deliver(build_delta(self.kind, op_id, a, self.epoch, Some(gen)), author);
    }
    fn state(&self) -> T::State {
        self.c.state()
    }
}

/// A Site at fold: a group with a Host part (A-11: only a Site has a Face), ADMIN holding
/// admin, MEMBER holding nothing.
fn site() -> Log<GroupType> {
    let mut l = Log::<GroupType>::new(ObjectKind::Group, &[OWNER, ADMIN, MEMBER]);
    l.seq(op("group", "base.setPart"), args(&[("part", t(HOST)), ("role", t("host")), ("at", ArgVal::Int(1))]));
    l.seq(OP_SET_ROLE, args(&[("member", t(&hex::encode(ADMIN))), ("role", t("admin"))]));
    l
}
fn face(doc: &str) -> Args {
    args(&[("face", t(doc))])
}
fn edit_face() -> u32 {
    op("group", "group.editFace")
}

// ---- the sequenced ops stand, and why they cannot carry an admin ---------------------

/// Pins WHY row 10 is two new ops and not a relabel: the sequenced spine refuses an
/// admin's group.setFace whatever its declared authority.
#[test]
fn an_admins_sequenced_set_face_is_unauthorized_at_fold() {
    let mut l = site();
    assert_eq!(l.seq_by(ADMIN, OP_SET_FACE, face(r#"{"v":1}"#)), Err(DeltaRejection::Unauthorized));
    assert_eq!(l.state().face, "", "and it sets nothing");
}

/// A Site that has only ever had sequenced faces folds as it did.
#[test]
fn a_site_with_only_sequenced_faces_folds_unchanged() {
    let mut l = site();
    l.seq(OP_SET_FACE, face(r#"{"v":1,"a":1}"#));
    l.seq(OP_SET_FACE, face(r#"{"v":1,"a":2}"#));
    assert_eq!(l.state().face, r#"{"v":1,"a":2}"#);
}

// ---- the shared face ----------------------------------------------------------------

#[test]
fn an_admins_shared_edit_is_the_face_over_the_sequenced_one() {
    let mut l = site();
    l.seq(OP_SET_FACE, face(r#"{"v":1,"by":"owner"}"#));
    l.comm(ADMIN, edit_face(), face(r#"{"v":1,"by":"admin"}"#), 1);
    assert_eq!(l.state().face, r#"{"v":1,"by":"admin"}"#);
}

/// Ordered by gen, the Lamport generation, whoever writes: never by a clock.
#[test]
fn a_later_gen_wins_whoever_writes() {
    let mut l = site();
    l.comm(OWNER, edit_face(), face(r#"{"v":1,"n":5}"#), 5);
    l.comm(ADMIN, edit_face(), face(r#"{"v":1,"n":7}"#), 7);
    assert_eq!(l.state().face, r#"{"v":1,"n":7}"#, "the admin's later write");
    l.comm(OWNER, edit_face(), face(r#"{"v":1,"n":9}"#), 9);
    assert_eq!(l.state().face, r#"{"v":1,"n":9}"#, "then the owner's");
    l.comm(ADMIN, edit_face(), face(r#"{"v":1,"n":6}"#), 6);
    assert_eq!(l.state().face, r#"{"v":1,"n":9}"#, "an earlier gen, delivered last, does not win");
}

#[test]
fn a_member_who_is_not_an_admin_does_not_count() {
    let mut l = site();
    l.seq(OP_SET_FACE, face(r#"{"v":1,"by":"owner"}"#));
    l.comm(MEMBER, edit_face(), face(r#"{"v":1,"by":"member"}"#), 3);
    assert_eq!(l.state().face, r#"{"v":1,"by":"owner"}"#);
}

#[test]
fn revoking_an_admin_makes_their_writes_stop_counting_and_the_face_falls_back() {
    let mut l = site();
    l.comm(OWNER, edit_face(), face(r#"{"v":1,"by":"owner"}"#), 5);
    l.comm(ADMIN, edit_face(), face(r#"{"v":1,"by":"admin"}"#), 7);
    assert_eq!(l.state().face, r#"{"v":1,"by":"admin"}"#);
    l.seq(OP_CLEAR_ROLE, args(&[("member", t(&hex::encode(ADMIN)))]));
    assert_eq!(l.state().face, r#"{"v":1,"by":"owner"}"#, "the owner's, the latest that counts");
}

/// gen is the author's own number: an admin could write the highest there is, and no
/// write after could pass it. Revoking them is the answer.
#[test]
fn a_max_gen_write_by_an_admin_then_revoked_no_longer_holds_the_face() {
    let mut l = site();
    l.comm(ADMIN, edit_face(), face(r#"{"v":1,"by":"hostile"}"#), i64::MAX as u64);
    l.comm(OWNER, edit_face(), face(r#"{"v":1,"by":"owner"}"#), 12);
    assert_eq!(l.state().face, r#"{"v":1,"by":"hostile"}"#, "it holds while they hold admin");
    l.seq(OP_CLEAR_ROLE, args(&[("member", t(&hex::encode(ADMIN)))]));
    assert_eq!(l.state().face, r#"{"v":1,"by":"owner"}"#);
}

#[test]
fn a_site_whose_admins_are_all_revoked_shows_its_sequenced_face() {
    let mut l = site();
    l.seq(OP_SET_FACE, face(r#"{"v":1,"by":"sequenced"}"#));
    l.comm(ADMIN, edit_face(), face(r#"{"v":1,"by":"admin"}"#), 4);
    l.seq(OP_CLEAR_ROLE, args(&[("member", t(&hex::encode(ADMIN)))]));
    assert_eq!(l.state().face, r#"{"v":1,"by":"sequenced"}"#);
}

/// The document's rules are group.setFace's: a JSON object within the cap, on a Site.
#[test]
fn a_shared_edit_is_held_to_the_faces_rules() {
    let mut l = site();
    l.comm(OWNER, edit_face(), face(r#"{"v":1}"#), 1);
    l.comm(OWNER, edit_face(), face("[1,2]"), 2);
    l.comm(OWNER, edit_face(), face(&format!(r#"{{"x":"{}"}}"#, "a".repeat(16_400))), 3);
    assert_eq!(l.state().face, r#"{"v":1}"#, "not an object, and over the cap, are refused");
    let mut group = Log::<GroupType>::new(ObjectKind::Group, &[OWNER]);
    group.comm(OWNER, edit_face(), face(r#"{"v":1}"#), 1);
    assert_eq!(group.state().face, "", "a group that is not a Site has no Face");
}

// ---- the Host's media ---------------------------------------------------------------

fn host() -> Log<HostType> {
    let mut l = Log::<HostType>::new(ObjectKind::Host, &[OWNER, ADMIN, MEMBER]);
    l.seq(OP_SET_ROLE, args(&[("member", t(&hex::encode(ADMIN))), ("role", t("admin"))]));
    l
}
fn media(slot: &str, data: &str, mime: &str) -> Args {
    args(&[("slot", t(slot)), ("media", t(data)), ("mediaMime", t(mime))])
}

/// The roles facet reaches the Host (additive): standing is held on the Host itself.
#[test]
fn the_host_holds_its_own_roles() {
    let l = host();
    let decl = HostType::op(OP_SET_ROLE).unwrap_or_else(|| panic!("base.setRole is not declared on host (row 10)"));
    assert_eq!(decl.name, "base.setRole");
    drop(l);
}

#[test]
fn an_admin_of_the_host_edits_a_slot_and_revoking_them_falls_back() {
    let edit = op("host", "host.editMedia");
    let mut l = host();
    l.seq(OP_SET_MEDIA, media("mark", PNG, "image/png"));
    l.comm(ADMIN, edit, media("mark", WEBP, "image/webp"), 3);
    l.comm(MEMBER, edit, media("logo", PNG, "image/png"), 4);
    let st = l.state();
    assert_eq!(st.media["mark"].mime, "image/webp", "the admin's edit is the slot's picture");
    assert!(!st.media.contains_key("logo"), "a member who is not an admin does not count");
    l.seq(OP_CLEAR_ROLE, args(&[("member", t(&hex::encode(ADMIN)))]));
    assert_eq!(l.state().media["mark"].mime, "image/png", "revoked: the sequenced picture");
}

// ---- through the one write path, over a relay ---------------------------------------

fn face_args(doc: &str) -> Args {
    face(doc)
}

/// A Site of ada's with bo (made admin) and cy (a member), and its Host.
async fn a_site(h: &Harness) -> (String, String) {
    let (ada, bo, cy) = (0, 1, 2);
    let site = h.mint_group(ada);
    h.group_set_profile(ada, &site, "Mill Road", "community").await;
    let host = h.node(ada).host_new(&site, "Mill Road").await.unwrap();
    h.add_to_forum(ada, bo, &site).await;
    h.add_to_forum(ada, cy, &site).await;
    h.node(ada)
        .apply(&site, OP_SET_ROLE, args(&[("member", t(&hex::encode(h.id(bo)))), ("role", t("admin"))]))
        .await
        .expect("the owner makes bo admin");
    h.settle().await;
    (site, host)
}

#[tokio::test]
async fn an_admin_edits_the_face_and_every_member_reads_it() {
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let (site, _) = a_site(&h).await;
    let edit = edit_face();
    h.node(1).apply(&site, OP_SET_FACE, face_args(r#"{"v":1}"#)).await.expect_err("the sequenced op stays the owner's");
    h.node(1).apply(&site, edit, face_args(r#"{"v":1,"by":"bo"}"#)).await.expect("an admin's shared edit");
    h.settle().await;
    for u in [0, 2] {
        assert_eq!(h.node(u).group_state(&site).unwrap().face, r#"{"v":1,"by":"bo"}"#, "{} reads it", h.name(u));
    }
    h.node(2).apply(&site, edit, face_args(r#"{"v":1,"by":"cy"}"#)).await.expect_err("a member who is not an admin is refused at write");
}

#[tokio::test]
async fn a_revoked_admin_is_refused_and_their_edit_stops_counting() {
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let (site, _) = a_site(&h).await;
    let edit = edit_face();
    h.node(0).apply(&site, edit, face_args(r#"{"v":1,"by":"ada"}"#)).await.unwrap();
    h.settle().await;
    h.node(1).apply(&site, edit, face_args(r#"{"v":1,"by":"bo"}"#)).await.unwrap();
    h.settle().await;
    assert_eq!(h.node(2).group_state(&site).unwrap().face, r#"{"v":1,"by":"bo"}"#);
    h.node(0).apply(&site, OP_CLEAR_ROLE, args(&[("member", t(&hex::encode(h.id(1))))])).await.unwrap();
    h.settle().await;
    h.node(1).apply(&site, edit, face_args(r#"{"v":1,"by":"bo again"}"#)).await.expect_err("revoked: refused at write");
    assert_eq!(h.node(2).group_state(&site).unwrap().face, r#"{"v":1,"by":"ada"}"#, "and the face falls back");
}

/// Make admin on the Host: bo is added to the Host's roster and made admin there, and
/// edits a slot every Host member reads.
#[tokio::test]
async fn an_admin_on_the_host_edits_its_media() {
    let h = Harness::new(&["ada", "bo", "cy"]).await;
    let (_, host) = a_site(&h).await;
    h.add_to_forum(0, 1, &host).await;
    h.node(0)
        .apply(&host, OP_SET_ROLE, args(&[("member", t(&hex::encode(h.id(1)))), ("role", t("admin"))]))
        .await
        .expect("the owner makes bo admin on the Host too");
    h.settle().await;
    h.node(1).apply(&host, op("host", "host.editMedia"), media("mark", PNG, "image/png")).await.expect("an admin's picture");
    h.settle().await;
    let view: serde_json::Value = serde_json::from_str(&h.node(0).object_view(&host).unwrap()).unwrap();
    assert_eq!(view["media"]["mark"]["mime"], "image/png", "{view}");
}

// ---- Make admin's second step: a Site member onto its Host, by member id ------------

/// The owner adds a member of the Site to its Host by their identity alone
/// (`Node::add_part_member`): only someone on the Site's roster, and only a contact, whose
/// prekey is the key package. Each refusal is said in the words the webapp shows.
#[tokio::test]
async fn a_site_member_who_is_a_contact_is_added_to_the_host_by_member_id() {
    let h = Harness::new(&["ada", "bo", "cy", "dee"]).await;
    let (ada, bo, cy, dee) = (0, 1, 2, 3);
    let (site, host) = a_site(&h).await;
    h.pair(ada, bo).await;
    h.pair(ada, dee).await;
    h.settle().await;
    let hex_of = |u: usize| hex::encode(h.id(u));

    let e = h.node(ada).add_part_member(&host, &h.id(dee)).await.expect_err("dee is not on the Site");
    assert_eq!(e.to_string(), format!("membership: {} is not a member of {site}", hex_of(dee)));
    let e = h.node(ada).add_part_member(&host, &h.id(cy)).await.expect_err("cy is not ada's contact");
    assert_eq!(e.to_string(), format!("membership: {} is not your contact: ask them to connect with you first", hex_of(cy)));
    let e = h.node(ada).add_part_member(&site, &h.id(bo)).await.expect_err("the Site is a part of nothing");
    assert_eq!(e.to_string(), format!("membership: {site} is a part of no Site"));

    h.node(ada).add_part_member(&host, &h.id(bo)).await.expect("bo is on the Site and ada's contact");
    h.settle().await;
    assert!(h.roster(ada, &host).contains(&h.id(bo)), "bo is on the Host's roster");
    h.node(ada)
        .apply(&host, OP_SET_ROLE, args(&[("member", t(&hex_of(bo))), ("role", t("admin"))]))
        .await
        .expect("and so may hold admin there");
}
