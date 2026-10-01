//! A GROUP'S PUBLIC FACE REACHES ITS ARC, AND NOTHING ELSE DOES.
//!
//! The seam's rule is that an Arc never joins a group (the-seam.html §03), so a face
//! cannot be read off the group's log — it has to be put where the Arc can read it. That
//! place is the HOST (kind 33): a part of the Site, on whose roster the Arc sits and the
//! group's members do not. The owner copies the face across (`host.hydrate` key `face`,
//! pictures by `host.setMedia`) and names the Arc that serves it (`base.publish`). This
//! binary is that round trip, over a real relay:
//!
//!   the owner drafts the face on the GROUP     →  the Arc cannot see it
//!   the owner mints a Host and adds the Arc    →  the Arc folds the Host
//!   the owner publishes                        →  the Arc serves it at the slug
//!   the owner unpublishes / is removed         →  the Arc serves nothing
//!
//! The Arc here is an ordinary `Node` — which is what arc-node is.
//!
//! Ported from signup-host@244ea2e, where a Host was a System with connector
//! `wallflowers.host`, onto the Host kind (35a0ba0).
//!
//! Every op opens its own `Node` at the point of use (`h.node(u)`), because
//! `PACIFIC_STATE_DIR` is process-global and `settle` re-points it: a handle kept across
//! a sync would write into whichever device synced last. Where a call needs the other
//! device's bundle, that bundle is taken FIRST, into a variable — Rust evaluates the
//! receiver before the arguments, so `a.op(b.thing())` would leave the env on b.

mod common;

use common::Harness;
use pacific_core::group::OP_SET_FACE;
use pacific_core::host::MAX_MEDIA_B64;

/// A 1×1 png, so a picture is real bytes rather than a string that happens to decode.
const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

const BUNDLE: &str = r#"{"v":1,"profile":{"displayName":"Mill Road Allotments","shape":"community","card":{"note":"Thirty plots behind the station.","urls":[]}},"face":{"v":1,"listed":true,"door":{"doorbell":"door","label":"","questions":[]}},"location":{"shape":"area","precision":5,"label":"Romsey","cells":[]}}"#;

fn face_args(doc: &str) -> pacific_core::coordinator::Args {
    let mut a = pacific_core::coordinator::Args::new();
    a.insert("face".into(), pacific_core::coordinator::ArgVal::Text(doc.into()));
    a
}

#[tokio::test]
async fn a_face_published_onto_a_host_is_served_by_the_arc_named_on_it() {
    // "arc" is a person here because that is what an Arc is on the wire: an identity
    // with a device, holding no seat in anyone's group until it is given one.
    let h = Harness::people(&[("ada", &["phone"]), ("arc", &["node"])]).await;
    let (phone, arc) = (h.device("ada", "phone"), h.device("arc", "node"));
    let arc_id = h.id(arc);

    // THE GROUP, made a Site by its Host part (A-11: only a Site has a Face), and its
    // face — the working copy, on the group's own log.
    let site = h.mint_group(phone);
    h.group_set_profile(phone, &site, "Mill Road Allotments", "community").await;
    let host = h.node(phone).host_new(&site, "Mill Road Allotments").await.unwrap();
    h.node(phone).apply(&site, OP_SET_FACE, face_args(r#"{"v":1,"listed":true}"#)).await.unwrap();
    assert_eq!(
        h.node(phone).group_state(&site).unwrap().face,
        r#"{"v":1,"listed":true}"#,
        "the group holds its own working copy"
    );

    // NOTHING YET. Asserted before the positive, so a resolver that says yes to
    // everything cannot pass this binary.
    assert!(h.node(arc).host_sites(&arc_id).unwrap().sites.is_empty(), "an Arc serves nothing it was not given");

    // THE HOST, a part of the Site, and the Arc on its roster — never on the group's.
    assert_eq!(
        h.node(phone).group_state(&site).unwrap().parts[&host].role,
        "host",
        "the Site names its Host as a part"
    );
    let v: serde_json::Value = serde_json::from_str(&h.node(phone).object_view(&host).unwrap()).unwrap();
    assert_eq!(v["name"], "Mill Road Allotments");
    assert_eq!(v["parent"]["parent"], site.as_str(), "and the Host names its Site: both halves of part_of");
    assert_eq!(v["parent"]["role"], "host");

    let bundle = h.node(arc).build_contact_bundle().unwrap();
    h.node(phone).group_add_member(&host, &bundle).await.unwrap();
    h.settle().await;
    assert!(
        h.roster(phone, &host).contains(&arc_id),
        "the Arc is on the Host"
    );
    assert!(
        !h.roster(phone, &site).contains(&arc_id),
        "and on nothing of the group's — the seam's own rule"
    );

    // PUBLISHED: the face, a picture, and the address naming the Arc.
    h.node(phone)
        .host_publish(
            &host,
            "allotments",
            &hex::encode(arc_id),
            BUNDLE,
            &[("mark".into(), "image/png".into(), PNG_B64.into())],
        )
        .await
        .unwrap();
    h.node(phone).host_put_item(&host, "event:1", r#"{"title":"Dig day","startMs":1790000000000,"venue":"The shed"}"#, false).await.unwrap();
    h.settle().await;

    let scan = h.node(arc).host_sites(&arc_id).unwrap();
    assert_eq!(scan.sites.len(), 1, "the Arc serves the one face it was given: {:?}", scan.refused);
    let site_view = &scan.sites[0];
    assert_eq!(site_view.slug, "allotments");
    assert_eq!(site_view.name, "Mill Road Allotments");
    assert_eq!(site_view.bundle, BUNDLE, "byte for byte what the owner published");
    assert_eq!(site_view.media, vec![("mark".to_string(), "image/png".to_string())]);
    assert_eq!(site_view.items.len(), 1);
    assert_eq!(site_view.items[0].key, "event:1");
    let (mime, bytes) = h.node(arc).host_media(&host, "mark", &arc_id).unwrap().unwrap();
    assert_eq!(mime, "image/png");
    assert_eq!(&bytes[1..4], b"PNG", "the Arc hands out the picture's own bytes");

    // THE GROUP'S OWN LOG IS STILL THE GROUP'S. The Arc holds the Host, not the site.
    assert!(h.node(arc).group_state(&site).is_err(), "the Arc cannot fold a group it is not in");

    // TAKEN DOWN by its owner.
    h.node(phone).host_unpublish(&host).await.unwrap();
    h.settle().await;
    let scan = h.node(arc).host_sites(&arc_id).unwrap();
    assert!(scan.sites.is_empty(), "unpublish stops the serving");
    assert!(h.node(arc).host_media(&host, "mark", &arc_id).unwrap().is_none(), "and its pictures with it");
    assert!(
        scan.refused.iter().any(|(id, why)| id == &host && why == "not published"),
        "and the Arc says why, rather than dropping it silently: {:?}",
        scan.refused
    );
}

/// The other revocation: the Arc is REMOVED from the Host. Serving stops because the
/// publisher is no longer on the roster — `published_site`'s own rule, now performable
/// (membership-through-mls.md §5).
/// A-11 (Ralph, 27 Sep): a Site is a Group that has a Host part, and group.setFace is
/// refused on a Group without one, at write. Clearing is not a Face, so it stands.
#[tokio::test]
async fn a_face_is_refused_on_a_group_that_is_not_a_site() {
    let h = Harness::people(&[("ada", &["phone"])]).await;
    let phone = h.device("ada", "phone");
    let group = h.mint_group(phone);
    let refused = h.node(phone).apply(&group, OP_SET_FACE, face_args(r#"{"v":1}"#)).await;
    assert!(refused.is_err(), "a Group with no Host part is refused a Face: {refused:?}");
    assert_eq!(h.node(phone).group_state(&group).unwrap().face, "", "and holds none");
    h.node(phone).apply(&group, OP_SET_FACE, face_args("")).await.expect("clearing stands");
    let host = h.node(phone).host_new(&group, "Allotments").await.unwrap();
    assert_eq!(h.node(phone).group_state(&group).unwrap().parts[&host].role, "host");
    h.node(phone).apply(&group, OP_SET_FACE, face_args(r#"{"v":1}"#)).await.expect("with its Host, it is a Site");
}

#[tokio::test]
async fn removing_the_arc_from_the_host_stops_the_serving() {
    let h = Harness::people(&[("ada", &["phone"]), ("arc", &["node"])]).await;
    let (phone, arc) = (h.device("ada", "phone"), h.device("arc", "node"));
    let arc_id = h.id(arc);

    let site = h.mint_group(phone);
    let host = h.node(phone).host_new(&site, "Mill Road Allotments").await.unwrap();
    let bundle = h.node(arc).build_contact_bundle().unwrap();
    h.node(phone).group_add_member(&host, &bundle).await.unwrap();
    h.settle().await;
    h.node(phone).host_publish(&host, "allotments", &hex::encode(arc_id), BUNDLE, &[]).await.unwrap();
    h.settle().await;
    assert_eq!(h.node(arc).host_sites(&arc_id).unwrap().sites.len(), 1);

    h.node(phone).group_remove_member(&host, &hex::encode(arc_id), Some("other")).await.unwrap();
    h.settle().await;

    let scan = h.node(arc).host_sites(&arc_id).unwrap();
    assert!(scan.sites.is_empty(), "a removed Arc serves nothing");
    let seen = scan.refused.iter().any(|(id, _)| id == &host);
    assert!(
        seen || scan.refused.is_empty(),
        "either it says why, or the Host is gone from this device altogether: {:?}",
        scan.refused
    );
}

/// What a Host will not carry: an address the fold refuses, a publisher that is not on
/// the roster, a picture too big for a delta — and nothing but a Host takes an address.
#[tokio::test]
async fn a_host_refuses_what_it_should_before_anything_is_written() {
    let h = Harness::people(&[("ada", &["phone"]), ("arc", &["node"])]).await;
    let (phone, arc) = (h.device("ada", "phone"), h.device("arc", "node"));
    let arc_id = h.id(arc);
    let site = h.mint_group(phone);
    let host = h.node(phone).host_new(&site, "Allotments").await.unwrap();

    // The Arc is not on the Host yet: publishing is refused at the door, not at the fold.
    let e = h.node(phone).host_publish(&host, "allotments", &hex::encode(arc_id), BUNDLE, &[]).await.unwrap_err();
    assert!(format!("{e}").contains("not on this Host"), "{e}");

    let bundle = h.node(arc).build_contact_bundle().unwrap();
    h.node(phone).group_add_member(&host, &bundle).await.unwrap();
    h.settle().await;

    let e = h.node(phone).host_publish(&host, "Allotments!", &hex::encode(arc_id), BUNDLE, &[]).await.unwrap_err();
    assert!(format!("{e}").contains("not a valid address"), "{e}");
    let e = h.node(phone).host_publish(&host, "allotments", &hex::encode(arc_id), "not json", &[]).await.unwrap_err();
    assert!(format!("{e}").contains("JSON object"), "{e}");

    // AN OVERSIZE PICTURE IS REFUSED AT THE DOOR, and nothing at all is written. It
    // matters that this one is caught here rather than at the fold: a delta bigger than
    // the relay's 256 KiB publish is written locally and silently never delivered, and
    // the hole it leaves stops every OTHER member folding the Host at all.
    let e = h
        .node(phone)
        .host_publish(
            &host,
            "allotments",
            &hex::encode(arc_id),
            BUNDLE,
            &[("cover".into(), "image/jpeg".into(), "A".repeat(MAX_MEDIA_B64 + 4))],
        )
        .await
        .unwrap_err();
    assert!(format!("{e}").contains("the most a Host carries"), "{e}");
    let e = h
        .node(phone)
        .host_publish(&host, "allotments", &hex::encode(arc_id), BUNDLE, &[("cover".into(), "image/svg+xml".into(), PNG_B64.into())])
        .await
        .unwrap_err();
    assert!(format!("{e}").contains("not a picture a Host serves"), "{e}");
    h.settle().await;
    assert!(
        h.node(arc).host_sites(&arc_id).unwrap().sites.is_empty(),
        "a refused publish published nothing"
    );

    // And now one that is allowed, so the negatives below have something to be unlike.
    h.node(phone).host_publish(&host, "allotments", &hex::encode(arc_id), BUNDLE, &[]).await.unwrap();
    h.settle().await;
    let scan = h.node(arc).host_sites(&arc_id).unwrap();
    assert_eq!(scan.sites.len(), 1, "the publish that was allowed stands: {:?}", scan.refused);
    assert!(scan.sites[0].media.is_empty(), "and it carries no picture");

    // A System is not a Host and takes no address: its kind does not declare the op,
    // so the door refuses it before anything is written.
    let feed = h.node(phone).object_new("system", "Resident Advisor").unwrap();
    h.node(phone)
        .apply(&feed, pacific_core::system::OP_DEFINE, pacific_core::system::define_args("Resident Advisor", "music.ra", None))
        .await
        .unwrap();
    let e = h
        .node(phone)
        .apply(&feed, pacific_core::publication::OP_PUBLISH, pacific_core::publication::publish_args("ra", &arc_id))
        .await
        .unwrap_err();
    assert!(!format!("{e}").is_empty());
    h.settle().await;
    let scan = h.node(arc).host_sites(&arc_id).unwrap();
    assert_eq!(scan.sites.len(), 1, "still only the one Host");
    assert!(scan.sites.iter().all(|s| s.slug != "ra"), "a System cannot be published");
}

/// The Arc is on the Host's roster, and `host.hydrate` is any-member. A server that
/// could author the page it serves would be serving its own words under the group's
/// name, so the fold refuses the publisher's hydrate and the door refuses it first.
#[tokio::test]
async fn the_arc_cannot_author_the_face_it_serves() {
    let h = Harness::people(&[("ada", &["phone"]), ("arc", &["node"])]).await;
    let (phone, arc) = (h.device("ada", "phone"), h.device("arc", "node"));
    let arc_id = h.id(arc);

    let site = h.mint_group(phone);
    let host = h.node(phone).host_new(&site, "Allotments").await.unwrap();
    let bundle = h.node(arc).build_contact_bundle().unwrap();
    h.node(phone).group_add_member(&host, &bundle).await.unwrap();
    h.settle().await;
    h.node(phone).host_publish(&host, "allotments", &hex::encode(arc_id), BUNDLE, &[]).await.unwrap();
    h.settle().await;

    // The Arc tries to write a face of its own, and an item: refused at its own door.
    assert!(
        h.node(arc).host_put_item(&host, "event:evil", r#"{"title":"Not ours","startMs":1790000000000}"#, false).await.is_err(),
        "the publisher may not put an item on the face it serves"
    );
    assert!(
        h.node(arc)
            .apply(
                &host,
                pacific_core::host::OP_HYDRATE,
                pacific_core::host::hydrate_args("face", r#"{"v":1,"profile":{"displayName":"Not ours"}}"#, 9_999_999_999_999, 9_999_999_999_999, false),
            )
            .await
            .is_err(),
        "nor a face"
    );
    h.settle().await;

    let scan = h.node(arc).host_sites(&arc_id).unwrap();
    let served = &scan.sites[0];
    assert_eq!(served.bundle, BUNDLE, "the Arc serves the group's face, not its own");
    assert!(served.items.is_empty(), "nor items it wrote itself");
}

/// The cap is a claim about the wire: `MAX_MEDIA_B64` says one `host.setMedia` delta at
/// that size still fits the relay's 256 KiB publish. A picture at exactly the cap is
/// therefore carried end to end here — if the envelope ever grows past the relay's
/// limit, this goes red before a member finds out by having their face break.
#[tokio::test]
async fn a_picture_at_the_cap_still_reaches_the_arc() {
    let h = Harness::people(&[("ada", &["phone"]), ("arc", &["node"])]).await;
    let (phone, arc) = (h.device("ada", "phone"), h.device("arc", "node"));
    let arc_id = h.id(arc);

    let site = h.mint_group(phone);
    let host = h.node(phone).host_new(&site, "Allotments").await.unwrap();
    let bundle = h.node(arc).build_contact_bundle().unwrap();
    h.node(phone).group_add_member(&host, &bundle).await.unwrap();
    h.settle().await;

    // "AAAA…" is base64 of the right length, which is all the cap is about.
    let big = "A".repeat(MAX_MEDIA_B64);
    h.node(phone)
        .host_publish(&host, "allotments", &hex::encode(arc_id), BUNDLE, &[("cover".into(), "image/jpeg".into(), big)])
        .await
        .unwrap();
    h.settle().await;

    let scan = h.node(arc).host_sites(&arc_id).unwrap();
    assert_eq!(scan.sites.len(), 1, "the Host still folds with a picture at the cap: {:?}", scan.refused);
    let (mime, bytes) = h.node(arc).host_media(&host, "cover", &arc_id).unwrap().unwrap();
    assert_eq!(mime, "image/jpeg");
    assert_eq!(bytes.len(), MAX_MEDIA_B64 / 4 * 3, "every byte of it arrived");
}

/// A HOST IS NAMED FROM THE SEED. The doctrine's question — will this survive the
/// person opening WallFlowers on a device that has never seen it? — applied to the one
/// object their public address lives on. The mint queued a spine entry for it and for
/// the Site, the sync published both, and a device holding the seed can name them.
/// Neither is recoverable yet, which `m27_minted_objects_are_not_recoverable` holds for
/// every object; when the archive writers land, the second half flips with that one.
#[tokio::test]
async fn a_host_is_named_from_the_seed_and_not_yet_recoverable() {
    let h = Harness::people(&[("ada", &["phone"]), ("arc", &["node"])]).await;
    let (phone, arc) = (h.device("ada", "phone"), h.device("arc", "node"));
    let arc_id = h.id(arc);

    let site = h.mint_group(phone);
    let host = h.node(phone).host_new(&site, "Mill Road Allotments").await.unwrap();
    let bundle = h.node(arc).build_contact_bundle().unwrap();
    h.node(phone).group_add_member(&host, &bundle).await.unwrap();
    h.settle().await;
    h.node(phone)
        .host_publish(&host, "allotments", &hex::encode(arc_id), BUNDLE, &[("mark".into(), "image/png".into(), PNG_B64.into())])
        .await
        .unwrap();
    h.settle().await;

    for (what, obj) in [("the Host", &host), ("the Site's group", &site)] {
        let r = h.node(phone).verify_recoverability(obj).await.unwrap();
        assert!(
            r.named,
            "{what} is named from the seed — its spine entry is at the address its index derives"
        );
        assert!(
            !r.recoverable(),
            "{what} is named and NOT recoverable: the archive key records and the MLS state \
             are still unwritten (m27_minted_objects_are_not_recoverable) — {r:?}"
        );
    }
}
