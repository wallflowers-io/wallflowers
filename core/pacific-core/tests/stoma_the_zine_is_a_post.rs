//! The cookbook zine, as a **Post (30)** — authored, answered and retracted on real
//! devices, and agreed on by all three.
//!
//! WHAT THE BRIEF ASKED FOR: `setProfile`, `setMedia`, `retract`, `react` and `comment`
//! over real devices, proving that `retract` is a STATE and not a delete — the object
//! stays and says so.
//!
//! IT COULD NOT BE WRITTEN UNTIL 15 SEP 2026, and the two tests that now drive it were,
//! until that date, CHARACTERISATIONS pinning why. `Node::post_author` — the only author
//! path for the kind, and the one `Node::mint(ObjectKind::Post, …)` dispatches to — had
//! two defects, and the first hid the second:
//!
//!   1. IT STAMPED THE WRONG WIRE TYPE. The envelope was built with
//!      `ObjectKind::Thing.type_id()` (27) where every other author uses its own kind.
//!      `PostType::KIND.type_id()` is 30, and `fold::fold_entries` — the ONE fold
//!      path, shared by the directory and the archive — keeps only deltas whose
//!      `type_id` matches the lens. So every Post delta ever authored was discarded
//!      before it reached the reducer, and `Node::posts()` returned the `Default`
//!      `PostState` for an object whose log was full.
//!
//!   2. IT NEVER TOOK THE COMMUTATIVE ARM. `post_author` had no `match
//!      decl.commutativity`, so `post.react` (3) and `post.comment` (4) — both declared
//!      `anyMember`/`commutative` — were written as owner-sequenced deltas carrying a
//!      `seq` and no `gen`. Fixing (1) alone would NOT have delivered a working Post: it
//!      would have made szonja's reaction a non-owner write on the spine, and
//!      `Coordinator::deliver` routes by the OP'S DECLARATION, so the delta would land
//!      in the commutative arm, be refused for carrying no `gen`, and `fold_entries`
//!      would propagate that with `?` — the object would stop being readable for
//!      everyone instead of merely being empty. The two had to be fixed together, and
//!      the last test here is the one that would have caught a half-fix.
//!
//! Both fixes are in `pacific-core/src/node.rs::post_author`.

mod common;

use common::Harness;
use pacific_core::fold;
use pacific_core::coordinator::{self, ArgVal, Args};
use pacific_core::object::{Commutativity, MemberId, ObjectKind, ObjectType};
use pacific_core::post::{self, Form, PostType, OP_REACT, OP_RETRACT, OP_SET_PROFILE};

/// The Post kind is real: its own type id, its own five ops, its own reducer.
/// Establishing that first, so nothing below can be read as "Post is not built yet".
#[tokio::test]
async fn the_post_kind_is_declared_and_its_reducer_works() {
    assert_eq!(ObjectKind::Post.type_id(), 30);
    assert_eq!(PostType::KIND, ObjectKind::Post);
    // The kind's OWN ops. Facet ops are excluded deliberately — they live in
    // reserved high bands and belong to the facet, not to Post. Pinning the whole
    // list meant this re-broke every time a facet reached Post (hosting, then
    // backlink), and a guard that rots on unrelated work is drift with a delay on
    // it. The facets are asserted below, by band, so they are still pinned.
    let own: Vec<(u32, &str)> = PostType::ops()
        .iter()
        .filter(|d| d.op_id < 0x1000)
        .map(|d| (d.op_id, d.name))
        .collect();
    assert_eq!(
        own,
        vec![
            // `post.comment` (4) is absent on purpose: a Post HOSTS its comments
            // room rather than carrying comments (m34).
            (0, "post.setProfile"),
            (1, "post.setMedia"),
            (2, "post.retract"),
            (3, "post.react"),
            // W-98 Resources (ICD 2.3.1): the document and the body's images.
            (5, "post.setDocument"),
            (6, "post.addAsset"),
            (7, "post.removeAsset"),
        ]
    );
    // And the facets it carries, named rather than enumerated by position.
    for (op, why) in [
        (pacific_core::parts::OP_SET_PART, "a Post is made of its comments room"),
        (pacific_core::parts::OP_CLEAR_PART, "and can detach it"),
        (pacific_core::backlink::OP_SET_BACKLINK, "a Post declares its own half of `created`"),
        (pacific_core::backlink::OP_CLEAR_BACKLINK, "and can withdraw it"),
    ] {
        assert!(PostType::op(op).is_some(), "{why}");
    }

    // And the reducer does exactly what the brief describes — retract is a STATE.
    // Driven here through the Coordinator, which is the same fold `Node` uses.
    let owner: MemberId = [7u8; 32];
    let mut coord = coordinator::Coordinator::<PostType>::new(vec![owner], owner);
    let mut seq = 0u64;
    let mut prev = coordinator::GENESIS_PREV;
    for (op, args) in [
        (
            OP_SET_PROFILE,
            post::set_profile_args("Cookbook Zine 2", Some("300 at A4, 150 at US paper"), None, None),
        ),
        (OP_RETRACT, Args::new()),
    ] {
        let d = coordinator::sequenced_delta(
            ObjectKind::Post.type_id() as u32,
            op,
            args,
            0,
            seq,
            prev,
        );
        prev = d.id();
        seq += 1;
        coord.deliver(d, owner).unwrap();
    }
    let st = coord.state();
    assert_eq!(st.title, "Cookbook Zine 2");
    assert!(
        st.retracted,
        "post.retract must set a flag, not remove anything"
    );
    assert_eq!(
        st.body, "300 at A4, 150 at US paper",
        "AND THE WORDS ARE STILL THERE. A retraction is a statement about the object, \
         not an erasure of it — the log is append-only and the fold says so"
    );
}

/// THE SCENARIO THE BRIEF ASKED FOR, on real devices.
///
/// axel publishes the zine from his phone, szonja reacts and comments from hers, axel
/// retracts from the laptop — and all three devices fold the same log to the same state:
/// the object is still there, it still carries its words, its readers' responses are on
/// it, and it says it was retracted.
///
/// Every delta on the wire carries type id 30. That is the assertion the old
/// characterisation made in reverse, and it is the one that makes the rest possible:
/// `fold::fold_entries` keeps only deltas whose type id matches the lens, so a Post
/// delta wearing Thing's 27 is discarded before any reducer sees it.
#[tokio::test]
async fn the_zine_is_published_answered_and_retracted_on_every_device() {
    let h = Harness::people(&[("axel", &["phone", "laptop"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let laptop = h.device("axel", "laptop");
    let szonja = h.device("szonja", "phone");
    h.pair(phone, szonja).await;

    // MINTED, not formed: the mint is what brings a Post's comments room into
    // being, and this scenario is about answering the zine.
    let zine = h.mint_object(phone, ObjectKind::Post, "Cookbook Zine 2", &[szonja]).await;
    h.add_to_forum(phone, laptop, &zine).await;
    h.settle().await;
    assert_eq!(h.device_leaves(phone, &zine), 3, "three leaves, two people");

    // ── the owner publishes ───────────────────────────────────────────────────
    h.node(phone)
        .apply(
            &zine,
            OP_SET_PROFILE,
            post::set_profile_args(
                "Cookbook Zine 2 — print run and the Spanish edition",
                Some("200 Spanish, 100 English. Plates held."),
                Some(Form::Article),
                None,
            ),
        )
        .await
        .expect("the owner may author a sequenced Post op");
    h.settle().await;

    // ── a Viewer answers ──────────────────────────────────────────────────────
    // szonja is not the owner. Everyone on the roster who is not the owner is a
    // Viewer, and the two Viewer ops are anyMember/commutative precisely so she can
    // use them without the owner being online to sequence anything.
    let mut react = Args::new();
    react.insert("emoji".into(), ArgVal::Text("🔥".into()));
    h.node(szonja)
        .apply(&zine, OP_REACT, react)
        .await
        .expect("a Viewer may react");

    // ── and comments in the ROOM, which is a different object ────────────────
    // `post.comment` is gone: a Post declares a comments Forum as a constituent
    // part and the mint brings it into being (m34). The room has its own roster,
    // so being on the ZINE is not being in its comments — the owner admits her,
    // which is the ordinary role-gated MLS act and the whole point of the shape.
    let v: serde_json::Value =
        serde_json::from_str(&h.node(phone).object_view(&zine).unwrap()).unwrap();
    let room = v["parts"][0]["part"].as_str().expect("the zine is made of a room").to_string();
    assert_eq!(v["parts"][0]["role"], "comments", "in the role its kind declares");
    assert!(
        h.node(szonja).object_transcript(&room).is_err(),
        "a Viewer of the Post is not yet in its comments room"
    );
    h.add_contact_to(phone, szonja, &room).await;
    h.node(szonja)
        .apply(
            &room,
            pacific_core::coordinator::FORUM_POST,
            {
                let mut a = Args::new();
                a.insert("text".into(), ArgVal::Text("Hold six for the Xalapa table.".into()));
                a
            },
        )
        .await
        .expect("and once admitted she may comment");
    h.settle().await;

    // ── the owner retracts, from his OTHER device ─────────────────────────────
    h.node(laptop)
        .apply(&zine, OP_RETRACT, Args::new())
        .await
        .expect("the owner's second device is still the owner");
    h.settle().await;

    // ── every delta on the wire is a POST delta ───────────────────────────────
    for u in [phone, laptop, szonja] {
        let log = h.node(u).dir.load_log(&hex::decode(&zine).unwrap()).unwrap();
        // THE MINTER HOLDS FIVE, A LATER JOINER HOLDS WHAT IT JOINED ABOVE.
        // The mint authors two — the Post's op 0 and the `base.setForum` that
        // attaches its comments room — before anyone else is on the roster, and
        // forward secrecy means a device added afterwards cannot have them. The
        // scenario's own three (setProfile, react, retract) must reach everyone,
        // and that is the transport claim this loop is really making.
        let want = if u == phone { 5 } else { 3 };
        assert!(
            log.len() >= want,
            "{} holds {} deltas, wanted at least {want} — the transport half has to \
             be sound for the fold assertions below to mean anything",
            h.device_name(u),
            log.len()
        );
        for (_author, envelope) in log {
            let d = coordinator::decode_delta(&envelope).unwrap();
            assert_eq!(
                d.type_id,
                ObjectKind::Post.type_id() as u32,
                "{}: a Post delta must carry 30 — `fold::fold_entries` filters on \
                 exactly this, and it carried Thing's 27 until 15 Sep 2026",
                h.device_name(u)
            );
        }
    }

    // ── and all three devices fold it to the same state ───────────────────────
    for u in [phone, laptop, szonja] {
        let posts = h.node(u).posts().unwrap();
        assert_eq!(posts.len(), 1, "{} holds the zine", h.device_name(u));
        let st = &posts[0].1;
        let who = h.device_name(u);

        assert_eq!(
            st.title, "Cookbook Zine 2 — print run and the Spanish edition",
            "{who} reads the title the owner wrote"
        );
        assert_eq!(
            st.body, "200 Spanish, 100 English. Plates held.",
            "{who}: THE WORDS SURVIVE THE RETRACTION. A retraction is a statement about \
             the object, not an erasure of it"
        );
        assert!(
            st.retracted,
            "{who}: `post.retract` is a STATE, and it is readable — this is the claim the \
             brief asked to be proved and the one the type-id defect made unprovable"
        );
        assert_eq!(
            st.responses.reactions.get(&h.id(szonja)).map(String::as_str),
            Some("🔥"),
            "{who} sees szonja's reaction"
        );
        // Her comment is in the ROOM's log, not the Post's — checked once below.
        assert!(
            st.responses.reactions.get(&h.id(phone)).is_none(),
            "{who}: nobody else reacted"
        );
    }
}

/// THE SECOND HALF OF THE FIX, which the first would otherwise hide: a Viewer's
/// response rides the COMMUTATIVE arm.
///
/// `Coordinator::deliver` routes by the op's DECLARATION, never by the delta's shape.
/// So a `post.react` written onto the spine — a `seq`, no `gen` — is handed to the
/// commutative arm anyway, refused there for carrying no LWW key, and `fold_entries`
/// turns that into a hard error for every reader: not "the reaction is lost" but "the
/// object is unreadable". This test pins the arm at the wire, because a fold that
/// merely *works* cannot distinguish a correct commutative delta from one that got
/// lucky.
#[tokio::test]
async fn a_viewers_reaction_rides_the_commutative_arm() {
    let h = Harness::people(&[("axel", &["phone"]), ("szonja", &["phone"])]).await;
    let phone = h.device("axel", "phone");
    let szonja = h.device("szonja", "phone");
    h.pair(phone, szonja).await;

    // MINTED, not formed: the mint is what brings a Post's comments room into
    // being, and this scenario is about answering the zine.
    // The PRIMITIVE here on purpose: this test counts what a Viewer's one op puts
    // on the wire, so the object starts with an empty log.
    let zine = h.form_object(phone, "post", &[szonja]).await;
    h.settle().await;

    // The declarations this test is about.
    assert_eq!(
        PostType::op(OP_REACT).unwrap().commutativity,
        Commutativity::Commutative
    );
    // A Post hosts its comments rather than carrying them, so the AnyMember op
    // this used to check is `forum.post` — in the room, on the room's roster.
    assert_eq!(
        pacific_core::object::ObjectKind::Post.parts()[0].kind,
        pacific_core::object::ObjectKind::Forum
    );
    assert_eq!(
        PostType::op(OP_RETRACT).unwrap().commutativity,
        Commutativity::Sequenced,
        "the owner's ops still take the spine — the arm is chosen per op, not per kind"
    );

    let mut a = Args::new();
    a.insert("emoji".into(), ArgVal::Text("🔥".into()));
    h.node(szonja)
        .apply(&zine, OP_REACT, a)
        .await
        .expect("a Viewer may react");
    h.settle().await;

    let log = h.node(phone).dir.load_log(&hex::decode(&zine).unwrap()).unwrap();
    assert_eq!(log.len(), 1);
    let (author, envelope) = &log[0];
    let d = coordinator::decode_delta(envelope).unwrap();
    assert_eq!(*author, h.id(szonja), "szonja authored it");
    assert_ne!(*author, h.id(phone), "and she is not the owner");

    assert!(
        d.seq.is_none(),
        "a commutative delta must NOT carry a spine position — a non-owner write on the \
         spine is `Unauthorized` at deliver, which `fold_entries` propagates to every reader"
    );
    assert!(
        d.gen.is_some(),
        "a commutative delta MUST carry its (gen) LWW key, or the commutative arm refuses \
         it as MalformedArgs and the object stops folding"
    );
    assert_eq!(
        d.args.get("gen"),
        Some(&ArgVal::Int(d.gen.unwrap() as i64)),
        "the envelope's gen and the arg the reducer reads are the same number"
    );
    assert_eq!(d.type_id, ObjectKind::Post.type_id() as u32);

    // And it folds — on the OWNER's device, which never saw the author's local state.
    let st = &h.node(phone).posts().unwrap()[0].1;
    assert_eq!(
        st.responses.reactions.get(&h.id(szonja)).map(String::as_str),
        Some("🔥")
    );
}
