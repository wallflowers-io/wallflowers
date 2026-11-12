//! #112: the MEDIA facet (ICD `facets.media`): an object's icon and banner as detached refs,
//! on every kind the facet names, folded through the one fold path (`fold::view_of`).
//!
//! Op ids and the kinds come from the ICD by name, so a renumbering fails here.

use pacific_core::coordinator::{sequenced_delta, ArgVal, Args, GENESIS_PREV};
use pacific_core::object::{build_delta, MemberId, ObjectKind};
use pacific_media::MediaRef;
use serde_json::Value;

const OWNER: MemberId = [7u8; 32];
const ADMIN: MemberId = [8u8; 32];
const MEMBER: MemberId = [9u8; 32];

fn icd() -> Value {
    let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json")).expect("the ICD");
    serde_json::from_str(&raw).expect("the ICD is JSON")
}

fn facet_op(name: &str) -> u32 {
    icd()["facets"]["media"]["ops"][name]["op"].as_u64().unwrap_or_else(|| panic!("the ICD declares {name}")) as u32
}

/// The kinds the facet is on, as the ICD names them.
fn kinds() -> Vec<ObjectKind> {
    icd()["facets"]["media"]["on"]
        .as_array()
        .expect("facets.media.on")
        .iter()
        .map(|k| *ObjectKind::ALL.iter().find(|o| o.name() == k.as_str().unwrap()).expect("a kind"))
        .collect()
}

/// A sealed ref, as `Node::media_put` returns one.
fn sealed(mime: &str, fill: u8) -> MediaRef {
    MediaRef::sealed(mime, [fill; 32], 4096, hex::encode([fill; 32]), "c2VjcmV0").expect("a sealed ref")
}

fn slot(prefix: &str, m: &MediaRef, gen: i64) -> Args {
    let mut a = Args::new();
    m.to_args(prefix, &mut a);
    a.insert("gen".into(), ArgVal::Int(gen));
    a
}

/// One object's log: the owner's sequenced spine, then commutative writes `(op, args, author, gen)`.
fn view(kind: ObjectKind, spine: &[(u32, Args)], writes: &[(u32, Args, MemberId, u64)]) -> Value {
    let tid = kind.type_id() as u32;
    let mut log = Vec::new();
    let mut prev = GENESIS_PREV;
    for (n, (op_id, a)) in spine.iter().enumerate() {
        let d = sequenced_delta(tid, *op_id, a.clone(), 0, n as u64, prev);
        prev = d.id();
        log.push((OWNER, d.canonical_bytes()));
    }
    for (op_id, a, author, gen) in writes {
        log.push((*author, build_delta(kind, *op_id, a.clone(), 0, Some(*gen)).canonical_bytes()));
    }
    let v = pacific_core::fold::view_of(kind.name(), OWNER, vec![OWNER, ADMIN, MEMBER], vec![], log, Some(&OWNER))
        .unwrap_or_else(|e| panic!("{} folds: {e:?}", kind.name()));
    serde_json::from_str(&v).expect("the view is JSON")
}

/// Whether a view slot holds a detached ref. A kind's own slot reads beside the facet's (a
/// Post's empty `icon` string), so "the facet wrote nothing" is this, not null.
fn detached(slot: &Value) -> bool {
    slot["via"] == "detached"
}

fn admin() -> (u32, Args) {
    let mut a = Args::new();
    a.insert("member".into(), ArgVal::Text(hex::encode(ADMIN)));
    a.insert("role".into(), ArgVal::Text("admin".into()));
    (pacific_core::roles::OP_SET_ROLE, a)
}

/// The owner's icon and banner fold on every kind, as the detached refs they were written as.
#[test]
fn the_owners_icon_and_banner_fold_on_every_kind() {
    let (icon, banner) = (sealed("image/jpeg", 1), sealed("image/png", 2));
    for kind in kinds() {
        let v = view(kind, &[], &[
            (facet_op("base.setIcon"), slot("icon", &icon, 1), OWNER, 1),
            (facet_op("base.setBanner"), slot("banner", &banner, 2), OWNER, 2),
        ]);
        let k = kind.name();
        assert_eq!(v["icon"]["via"], "detached", "{k}: the icon");
        assert_eq!(v["icon"]["digest"], hex::encode([1u8; 32]), "{k}: the icon's digest");
        assert_eq!(v["icon"]["mime"], "image/jpeg", "{k}");
        assert_eq!(v["banner"]["key"], hex::encode([2u8; 32]), "{k}: the banner's key");
        assert_eq!(v["banner"]["secret"], "c2VjcmV0", "{k}: the banner's secret");
    }
}

/// A plain member's write is refused; the last write that counts wins; an empty ref clears.
#[test]
fn a_member_is_refused_and_the_last_write_wins() {
    for kind in kinds() {
        let k = kind.name();
        let v = view(kind, &[], &[(facet_op("base.setIcon"), slot("icon", &sealed("image/png", 3), 1), MEMBER, 1)]);
        assert!(!detached(&v["icon"]), "{k}: a member's icon is refused");

        let v = view(kind, &[], &[
            (facet_op("base.setIcon"), slot("icon", &sealed("image/png", 3), 1), OWNER, 1),
            (facet_op("base.setIcon"), slot("icon", &sealed("image/png", 4), 2), OWNER, 2),
        ]);
        assert_eq!(v["icon"]["digest"], hex::encode([4u8; 32]), "{k}: the later write wins");

        let mut clear = Args::new();
        MediaRef::empty().to_args("banner", &mut clear);
        clear.insert("gen".into(), ArgVal::Int(2));
        let v = view(kind, &[], &[
            (facet_op("base.setBanner"), slot("banner", &sealed("image/png", 5), 1), OWNER, 1),
            (facet_op("base.setBanner"), clear, OWNER, 2),
        ]);
        assert!(!detached(&v["banner"]), "{k}: an empty ref clears the banner");
    }
}

/// Where the kind carries roles, an admin's write counts; without the role it is refused.
#[test]
fn an_admin_counts_where_the_kind_has_roles() {
    for kind in [ObjectKind::Group, ObjectKind::Event] {
        let write = [(facet_op("base.setBanner"), slot("banner", &sealed("image/png", 6), 1), ADMIN, 1)];
        assert_eq!(view(kind, &[admin()], &write)["banner"]["via"], "detached", "{}: an admin's banner", kind.name());
        assert!(!detached(&view(kind, &[], &write)["banner"]), "{}: refused without the role", kind.name());
    }
}

/// The facet's slot wins over the kind's own where written: a Post's inline `post.setMedia`
/// banner reads until `base.setBanner` writes.
#[test]
fn the_facet_wins_over_the_kinds_own_slot() {
    let mut legacy = Args::new();
    legacy.insert("banner".into(), ArgVal::Text("QUJDRA==".into()));
    legacy.insert("bannerMime".into(), ArgVal::Text("image/png".into()));
    let set_media = pacific_core::post::OP_SET_MEDIA;
    let v = view(ObjectKind::Post, &[(set_media, legacy.clone())], &[]);
    assert_eq!(v["banner"], "QUJDRA==", "the legacy banner reads alone");
    let v = view(ObjectKind::Post, &[(set_media, legacy)], &[(facet_op("base.setBanner"), slot("banner", &sealed("image/png", 7), 1), OWNER, 1)]);
    assert_eq!(v["banner"]["digest"], hex::encode([7u8; 32]), "the facet's banner wins");
    assert!(v.get("base_media").is_none(), "the view carries the facet as its keys, not its field");
}
