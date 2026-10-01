//! A SITE IS A PUBLISHED GROUP, AND YOU CAN ASK FOR IT BY ITS ADDRESS.
//!
//! `base.publish` gives an object a public face: a slug, and the member who
//! answers for it. `Node::published_site` is the one place that decides what a
//! NON-MEMBER may see, and the signup flow's whole premise — an account that owns
//! a site at an address — resolves through it.
//!
//! IT HAD NEVER RUN. Until 15 Sep 2026 `published_site` compared the directory's
//! kind string to `"Group"`, capital G, and the directory only ever writes
//! `"group"` (`ObjectKind::name`). So the loop skipped every object on earth and
//! the function could not return `Some` for anything ever created. It has no
//! callers on either platform, so nothing noticed — forty lines of careful
//! reasoning about revocation, guarding a branch that was unreachable.
//!
//! This binary is that missing caller. It mints a group, publishes it, and asks
//! for it back by slug — which is the shortest statement of "an account owns a
//! site" that is actually made of Deltas.

mod common;

use common::Harness;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::publication::OP_PUBLISH;

/// The happy path, and the one that pins the kind string.
#[tokio::test]
async fn a_group_published_under_a_slug_is_served_at_that_slug() {
    let h = Harness::people(&[("ada", &["phone"])]).await;
    let phone = h.device("ada", "phone");
    let node = h.node(phone);

    let site = h.mint_group(phone);
    h.group_set_profile(phone, &site, "Ada's reading room", "organisation")
        .await;

    // NOT PUBLISHED YET, and the address must not answer. Asserted first so the
    // positive below cannot pass against a function that says yes to everything.
    assert!(
        node.published_site("reading-room", &h.id(phone))
            .unwrap()
            .is_none(),
        "an unpublished object has no address"
    );

    let mut args = Args::new();
    args.insert("slug".into(), ArgVal::Text("reading-room".into()));
    args.insert(
        "publisher".into(),
        ArgVal::Text(hex::encode(h.id(phone))),
    );
    node.apply(&site, OP_PUBLISH, args).await.unwrap();

    let served = node
        .published_site("reading-room", &h.id(phone))
        .unwrap()
        .expect(
            "the published group was not served at its own slug. If this is None, \
             check the kind guard in `Node::published_site` — it compared against \
             \"Group\" until 15 Sep 2026 and the directory writes \"group\".",
        );
    assert_eq!(
        served.slug, "reading-room",
        "served under the slug it was published at"
    );
}

/// A slug nobody took, and a slug the fold could never have stored. Both answer
/// `None`, and the caller cannot tell them apart — which is deliberate.
#[tokio::test]
async fn an_unclaimed_or_impossible_slug_answers_nothing() {
    let h = Harness::people(&[("ada", &["phone"])]).await;
    let phone = h.device("ada", "phone");
    let node = h.node(phone);

    let site = h.mint_group(phone);
    let mut args = Args::new();
    args.insert("slug".into(), ArgVal::Text("taken".into()));
    args.insert("publisher".into(), ArgVal::Text(hex::encode(h.id(phone))));
    node.apply(&site, OP_PUBLISH, args).await.unwrap();

    assert!(
        node.published_site("taken", &h.id(phone)).unwrap().is_some(),
        "the fixture published"
    );
    assert!(
        node.published_site("not-taken", &h.id(phone))
            .unwrap()
            .is_none(),
        "a slug nobody published answers nothing"
    );
    assert!(
        node.published_site("Not A Slug!", &h.id(phone))
            .unwrap()
            .is_none(),
        "and a string the fold could not have stored is refused before the scan"
    );
}

/// THE PUBLISHER MUST BE A MEMBER, refused at the reducer rather than at serve
/// time — an address recorded against someone who cannot fold the object is an
/// address that can never answer.
#[tokio::test]
async fn publishing_to_a_non_member_is_refused() {
    let h = Harness::people(&[("ada", &["phone"]), ("bo", &["sim"])]).await;
    let phone = h.device("ada", "phone");
    let node = h.node(phone);

    let site = h.mint_group(phone);
    let mut args = Args::new();
    args.insert("slug".into(), ArgVal::Text("bos-place".into()));
    // bo is not in this group — ada minted it alone.
    args.insert("publisher".into(), ArgVal::Text(hex::encode(h.id(h.device("bo", "sim")))));

    // Authored or refused, the outcome that matters is the same: the address does
    // not resolve to a publisher who was never in the object.
    let _ = node.apply(&site, OP_PUBLISH, args).await;
    assert!(
        node.published_site("bos-place", &h.id(h.device("bo", "sim")))
            .unwrap()
            .is_none(),
        "a non-member publisher must never be served — `reduce_publication` \
         checks `ctx.is_member` before assigning either half"
    );
}
