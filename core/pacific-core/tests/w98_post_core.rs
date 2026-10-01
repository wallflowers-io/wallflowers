//! W-98 Resources, core (EVENTS-CORE): the ICD 2.3.1 draft's post rows, folded through the one
//! fold path (`fold::view_of`) and read back as the view the ICD declares. Owner-only and
//! sequenced, so a member's write is refused at the spine.
//!
//! Op ids come from the ICD by name.

use pacific_core::coordinator::{sequenced_delta, ArgVal, Args, Coordinator, GENESIS_PREV};
use pacific_core::object::{DeltaRejection, MemberId, ObjectKind};
use pacific_core::post::PostType;
use serde_json::Value;

const OWNER: MemberId = [7u8; 32];
const MEMBER: MemberId = [9u8; 32];
const PNG: &str = "cHJvYmU=";
const PDF: &str = "JVBERi0xLjQ=";

fn icd() -> Value {
    let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json")).expect("the ICD");
    serde_json::from_str(&raw).expect("the ICD is JSON")
}

fn op(name: &str) -> u32 {
    let d = icd();
    let found = d["kinds"]["post"]["ops"].get(name).cloned().or_else(|| {
        d["facets"].as_object().unwrap().values().find_map(|f| {
            let on = f["on"].as_array().unwrap().iter().any(|k| k == "post");
            if on { f["ops"].get(name).cloned() } else { None }
        })
    });
    found.unwrap_or_else(|| panic!("the ICD declares {name} on post"))["op"].as_u64().unwrap() as u32
}

fn args(pairs: &[(&str, ArgVal)]) -> Args {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}
fn t(s: &str) -> ArgVal {
    ArgVal::Text(s.into())
}
fn i(n: i64) -> ArgVal {
    ArgVal::Int(n)
}

/// The owner's spine, in order, folded and viewed.
fn view(spine: &[(&str, Args)]) -> Value {
    let tid = ObjectKind::Post.type_id() as u32;
    let mut log = Vec::new();
    let mut prev = GENESIS_PREV;
    for (n, (name, a)) in spine.iter().enumerate() {
        let d = sequenced_delta(tid, op(name), a.clone(), 0, n as u64, prev);
        prev = d.id();
        log.push((OWNER, d.canonical_bytes()));
    }
    let v = pacific_core::fold::view_of("post", OWNER, vec![OWNER, MEMBER], vec![], log, Some(&OWNER)).expect("the post folds");
    serde_json::from_str(&v).expect("the view is JSON")
}

fn profile(extra: &[(&str, ArgVal)]) -> Args {
    let mut a = args(&[("title", t("Notes"))]);
    for (k, v) in extra {
        a.insert(k.to_string(), v.clone());
    }
    a
}

fn document() -> Args {
    args(&[("document", t(PDF)), ("documentMime", t("application/pdf")), ("name", t("zine.pdf"))])
}

fn asset(id: &str, at: i64) -> Args {
    args(&[("id", t(id)), ("asset", t(PNG)), ("assetMime", t("image/png")), ("alt", t("a still")), ("at", i(at))])
}

/// The post's spine refuses a member's write: the owner's alone is sequenced.
fn refused_from_a_member(name: &str, a: Args) {
    let mut c: Coordinator<PostType> = Coordinator::new(vec![OWNER, MEMBER], OWNER);
    let d = sequenced_delta(ObjectKind::Post.type_id() as u32, op(name), a, 0, 0, GENESIS_PREV);
    assert_eq!(c.deliver(d, MEMBER), Err(DeltaRejection::Unauthorized), "{name} from a member");
}

#[test]
fn set_document_owner_counts() {
    let v = view(&[("post.setProfile", profile(&[("form", t("pdf"))])), ("post.setDocument", document())]);
    assert_eq!(v["document"]["mime"], "application/pdf");
    assert_eq!(v["document"]["data"], PDF);
    assert_eq!(v["document"]["name"], "zine.pdf");
}

#[test]
fn set_document_member_refused() {
    refused_from_a_member("post.setDocument", document());
}

/// A document is a PDF, within the ICD's ceiling; empty clears it.
#[test]
fn a_document_is_a_pdf_within_its_ceiling_and_empty_clears() {
    let cap = icd()["kinds"]["post"]["ops"]["post.setDocument"]["args"]["document"]["maxLength"].as_u64().unwrap() as usize;
    for bad in [
        args(&[("document", t(PNG)), ("documentMime", t("image/png"))]),
        args(&[("document", t(&"A".repeat(cap + 4))), ("documentMime", t("application/pdf"))]),
        args(&[("document", t(PDF)), ("documentMime", t("application/pdf")), ("name", t(&"n".repeat(256)))]),
    ] {
        let v = view(&[("post.setProfile", profile(&[])), ("post.setDocument", document()), ("post.setDocument", bad.clone())]);
        assert_eq!(v["document"]["data"], PDF, "refused: {bad:?}");
    }
    let v = view(&[
        ("post.setProfile", profile(&[])),
        ("post.setDocument", document()),
        ("post.setDocument", args(&[("document", t("")), ("documentMime", t(""))])),
    ]);
    assert_eq!(v["document"], Value::Null);
}

#[test]
fn add_asset_owner_counts() {
    let id = format!("{:016x}", 1);
    let v = view(&[("post.setProfile", profile(&[])), ("post.addAsset", asset(&id, 5))]);
    assert_eq!(v["assets"][0]["id"], id.as_str());
    assert_eq!(v["assets"][0]["alt"], "a still");
    assert_eq!(v["assets"][0]["data"], PNG);
}

#[test]
fn add_asset_member_refused() {
    refused_from_a_member("post.addAsset", asset(&format!("{:016x}", 1), 5));
}

#[test]
fn remove_asset_owner_counts() {
    let id = format!("{:016x}", 1);
    let v = view(&[("post.setProfile", profile(&[])), ("post.addAsset", asset(&id, 5)), ("post.removeAsset", args(&[("id", t(&id))]))]);
    assert_eq!(v["assets"].as_array().unwrap().len(), 0);
}

#[test]
fn remove_asset_member_refused() {
    refused_from_a_member("post.removeAsset", args(&[("id", t(&format!("{:016x}", 1)))]));
}

/// At most twenty live assets, ordered by `at`; a removed id stays removed.
#[test]
fn the_assets_cap_at_twenty_and_a_removal_is_final() {
    let mut spine = vec![("post.setProfile", profile(&[]))];
    for n in 0..21 {
        spine.push(("post.addAsset", asset(&format!("{:016x}", n + 1), 100 - n)));
    }
    let v = view(&spine);
    assert_eq!(v["assets"].as_array().unwrap().len(), 20, "a 21st live add is refused");
    assert_eq!(v["assets"][0]["id"], format!("{:016x}", 20), "ordered by at");

    let id = format!("{:016x}", 9);
    let v = view(&[
        ("post.setProfile", profile(&[])),
        ("post.addAsset", asset(&id, 5)),
        ("post.removeAsset", args(&[("id", t(&id))])),
        ("post.addAsset", asset(&id, 6)),
    ]);
    assert_eq!(v["assets"].as_array().unwrap().len(), 0, "an add after its removal is refused");
}

/// setProfile's bodyFormat and excerpt, and setMedia's bannerAlt, fold and read back.
#[test]
fn the_profile_carries_its_body_format_and_excerpt() {
    let v = view(&[
        ("post.setProfile", profile(&[("body", t("# Hi")), ("bodyFormat", t("markdown")), ("excerpt", t("A short one"))])),
        ("post.setMedia", args(&[("banner", t(PNG)), ("bannerMime", t("image/png")), ("bannerAlt", t("the cover"))])),
    ]);
    assert_eq!(v["body_format"], "markdown");
    assert_eq!(v["excerpt"], "A short one");
    assert_eq!(v["banner_alt"], "the cover");

    let v = view(&[("post.setProfile", profile(&[]))]);
    assert_eq!(v["body_format"], "plain", "absent is plain");

    for bad in [("bodyFormat", t("html")), ("excerpt", t(&"e".repeat(301)))] {
        let v = view(&[("post.setProfile", profile(&[])), ("post.setProfile", profile(&[("body", t("after")), bad.clone()]))]);
        assert_eq!(v["body"], "", "refused: {bad:?}");
    }
    let v = view(&[("post.setMedia", args(&[("banner", t(PNG)), ("bannerMime", t("image/png")), ("bannerAlt", t(&"a".repeat(192)))]))]);
    assert_eq!(v["banner"], "", "a bannerAlt past 191 characters refuses the media");
}

/// NC-139: a post with no visibility is `network`; the owner can make it members only.
#[test]
fn a_post_is_network_until_its_owner_says_otherwise() {
    assert_eq!(view(&[("post.setProfile", profile(&[]))])["visibility"], "network");
    let v = view(&[("post.setProfile", profile(&[])), ("base.setVisibility", args(&[("visibility", t("connections"))]))]);
    assert_eq!(v["visibility"], "connections");
}
