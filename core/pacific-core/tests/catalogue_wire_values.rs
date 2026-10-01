//! THE WIRE VALUES, SWEPT — type ids and op ids, on every kind at once.
//!
//! A `type_id` and an `op_id` are the two numbers a delta actually carries. Everything
//! else about an op — its name, its authority, which arm folds it — is looked up by
//! those two numbers on the receiving device, against tables it holds locally. So a
//! wrong number is not a wrong label: it is a delta that folds as something else, or
//! folds as nothing, on every machine but the one that wrote it.
//!
//! Both failures had happened, and both were invisible to a per-kind test:
//!
//!   * `Node::post_author` stamped `ObjectKind::Thing.type_id()` (27) on every Post
//!     delta. `fold::fold_entries` keeps only deltas whose type id matches the lens,
//!     so every Post ever authored was dropped before the reducer, and `posts()`
//!     returned `Default` for an object whose log was full. Six other author doors did
//!     it right; nothing compared them.
//!   * `pacific-ffi`'s `delta_catalog()` carried the pre-cutover Forum op ids, so
//!     `forum.retractPost` encoded as op 1 and folded as `forum.react`. That one is now
//!     pinned in `pacific-ffi/src/delta.rs::catalogue_pin`, which is where the two
//!     tables are.
//!
//! This binary is the core-side half: it sweeps the kinds rather than naming one, so
//! the eighth kind is covered the day it is declared.

mod common;

use std::collections::BTreeSet;

use common::Harness;
use pacific_core::fold;
use pacific_core::coordinator::decode_delta;
use pacific_core::mint::{self, MintDraft};
use pacific_core::object::{Authority, Commutativity, ObjectKind, ObjectType, OpDecl};

/// Every op table in the catalogue, paired with the kind it backs. Field(20) and
/// Topic(23) are absent because neither has an `impl ObjectType` — see the ICD's
/// `anchor`, which excludes them on the same grounds and in the same words.
fn tables() -> Vec<(ObjectKind, &'static [OpDecl])> {
    vec![
        (ObjectKind::Group, pacific_core::group::GroupType::ops()),
        (ObjectKind::Forum, pacific_core::coordinator::ForumType::ops()),
        (ObjectKind::System, pacific_core::system::SystemType::ops()),
        (ObjectKind::Project, pacific_core::project::ProjectType::ops()),
        (ObjectKind::Contact, pacific_core::contact::ContactType::ops()),
        (
            ObjectKind::Conversation,
            pacific_core::coordinator::ConversationType::ops(),
        ),
        (ObjectKind::Thing, pacific_core::thing::ThingType::ops()),
        (ObjectKind::Place, pacific_core::place::PlaceType::ops()),
        (ObjectKind::Event, pacific_core::event::EventType::ops()),
        (ObjectKind::Post, pacific_core::post::PostType::ops()),
        (
            ObjectKind::Treasury,
            pacific_core::treasury::TreasuryType::ops(),
        ),
        (ObjectKind::Note, pacific_core::note::NoteType::ops()),
        (ObjectKind::Host, pacific_core::host::HostType::ops()),
        (ObjectKind::Transaction, pacific_core::transaction::TransactionType::ops()),
    ]
}

/// `tables()` really is every kind that has one. An exhaustive match, so a thirteenth
/// `ObjectKind` does not compile until somebody has said whether it has a reducer.
#[test]
fn the_sweep_covers_every_kind_that_has_an_op_table() {
    let swept: BTreeSet<u16> = tables().iter().map(|(k, _)| k.type_id()).collect();
    assert_eq!(swept.len(), tables().len(), "a kind is swept twice");
    for &kind in ObjectKind::ALL {
        let has_table = match kind {
            ObjectKind::Group
            | ObjectKind::Forum
            | ObjectKind::System
            | ObjectKind::Project
            | ObjectKind::Contact
            | ObjectKind::Conversation
            | ObjectKind::Thing
            | ObjectKind::Place
            | ObjectKind::Event
            | ObjectKind::Post
            // Promoted out of `group`'s facets into kinds of their own, 25 Sep 2026:
            // a Treasury's roster is narrower than the body's, and a Note's roster
            // is whoever may read it. Both carry their own op table now.
            | ObjectKind::Treasury
            | ObjectKind::Note
            // A Site's public copy, promoted from System+connector to a kind of its
            // own on 25 Sep 2026 so the Arc joins one object and nothing else.
            | ObjectKind::Host
            // One sale, W-98 Trade, 30 Sep 2026.
            | ObjectKind::Transaction => true,
            // Declared in the taxonomy and nowhere else: no `impl ObjectType`, no
            // reducer, no author door. If one gains a table, this arm is where the
            // compiler sends whoever wrote it.
            ObjectKind::Field | ObjectKind::Topic => false,
        };
        assert_eq!(
            swept.contains(&kind.type_id()),
            has_table,
            "`{}` (type {}): the sweep and this declaration disagree",
            kind.name(),
            kind.type_id()
        );
    }
}

/// Within one kind, an op id appears once and a name appears once.
///
/// This is the shape of the Forum collision, checked where it can be checked cheaply:
/// two rows sharing an id means `T::op(id)` answers one of them and the other can never
/// be authored, and two rows sharing a name means a caller who knows the name cannot
/// know the id.
#[test]
fn no_kind_declares_one_id_or_one_name_twice() {
    for (kind, ops) in tables() {
        let mut ids: BTreeSet<u32> = BTreeSet::new();
        let mut names: BTreeSet<&str> = BTreeSet::new();
        for d in ops {
            assert!(
                ids.insert(d.op_id),
                "`{}` declares op id {} twice (`{}`) — an op id is a wire value and the \
                 receiver looks the op UP by it",
                kind.name(),
                d.op_id,
                d.name
            );
            assert!(
                names.insert(d.name),
                "`{}` declares `{}` twice",
                kind.name(),
                d.name
            );
        }
    }
}

/// The spec invariant, swept: a COMMUTATIVE op must be ANY-MEMBER.
///
/// `OpDecl::is_well_formed` states it and each module tests its own table; this asks
/// every table at once, which is the version a new kind inherits for free.
#[test]
fn every_declared_op_is_well_formed() {
    for (kind, ops) in tables() {
        for d in ops {
            assert!(
                d.is_well_formed(),
                "`{}` on `{}` is commutative and owner-gated — a broadcast op cannot be \
                 owner-gated before it propagates",
                d.name,
                kind.name()
            );
        }
    }
}

/// An op's name says which vocabulary it belongs to, and the base op-groups live in a
/// HIGH RESERVED BAND so they can never collide with a kind's own low-numbered ops.
///
/// That band is the whole reason `base.memberJoined` (0xF0010000) can be spliced into
/// GroupType beside `group.setProfile` (0). Forum's collision was the same problem
/// without the band: two vocabularies, one numbering, from 1 up.
#[test]
fn base_op_groups_stay_in_the_reserved_band() {
    const RESERVED: u32 = 0xF000_0000;
    for (kind, ops) in tables() {
        for d in ops {
            let is_base = d.name.starts_with("base.");
            // A Conversation's table is Forum's, so `forum.` is its own vocabulary too.
            let own = kind.name();
            assert!(
                is_base || d.name.starts_with(&format!("{own}.")) || kind == ObjectKind::Conversation,
                "`{}` on `{}` belongs to neither that kind's vocabulary nor `base.`",
                d.name,
                own
            );
            assert_eq!(
                d.op_id >= RESERVED,
                is_base,
                "`{}` (op {}) on `{}`: the reserved band 0xF0000000+ is for the base \
                 op-groups spliced onto many kinds, and ONLY for them. A kind's own op \
                 in that band, or a base op outside it, is how two vocabularies start \
                 sharing a number.",
                d.name,
                d.op_id,
                own
            );
        }
    }
}

/// THE ONE BASE OP-GROUP THAT SWEEP CANNOT SEE, checked here instead.
///
/// RATIFY (`ratify.propose`/`ratify.vote`/`ratify.close`) is in no op table at all —
/// `Coordinator::deliver` recognises it ahead of the type's table and
/// `Coordinator::ratify_state` folds it — so the band sweep above never visits it, and its
/// ids are not in the band anyway: 0xF000/0xF001/0xF002, three 16-bit values. They cannot be
/// renumbered into the band, because an op id is inside `Delta::canonical_bytes` and
/// therefore inside every signature already written (`coordinator.rs` carries the full
/// note). So the thing the band exists to BUY is bought here directly: no kind's own op may
/// share an id with one of them.
///
/// It holds with room today — the highest op id any kind declares is project's 19 — and the
/// day a table reaches 61440 this fails, rather than that kind's op silently folding as a
/// ballot on every device in the group.
#[test]
fn the_base_ratify_ops_collide_with_no_kinds_own_op() {
    use pacific_core::coordinator::RATIFY_OPS;

    for (kind, ops) in tables() {
        for d in ops {
            for r in RATIFY_OPS.iter() {
                assert_ne!(
                    d.op_id,
                    r.op_id,
                    "`{}` on `{}` shares op id {} with the base op `{}`, and \
                     `Coordinator::deliver` checks the ratify table FIRST — so every delta \
                     authored as `{}` would fold as a ratification instead",
                    d.name,
                    kind.name(),
                    d.op_id,
                    r.name,
                    d.name
                );
            }
        }
    }
}

/// Every declared op has a REDUCER ARM, not merely a row in a table.
///
/// `catalogue_reach` asks this of the eight channels it sweeps; this asks it of every
/// table, which is how `system` and `conversation` — neither of which that sweep
/// visits — are covered. Run the real reducer with empty args: anything at all except
/// `UnknownType` means control reached an arm and the arm parsed, which is the
/// question. `MalformedArgs` is the usual answer and is a PASS — it is the arm saying
/// "these args are empty", which an absent arm could never say.
#[test]
fn every_declared_op_has_a_reducer_arm() {
    use pacific_core::coordinator::Args;
    use pacific_core::object::{DeltaRejection, MemberId, Op, ReduceContext};

    const OWNER: MemberId = [1u8; 32];
    const OTHER: MemberId = [2u8; 32];

    fn probe<T: ObjectType>(op_id: u32) -> bool {
        let members = [OWNER, OTHER];
        let ctx = ReduceContext { members: &members, owner: OWNER, epoch: 1 };
        let args = Args::new();
        let mut st = T::State::default();
        !matches!(
            T::reduce(
                &mut st,
                &Op { op_id, args: &args, author: &OWNER, pos: None, ctx: &ctx },
            ),
            Err(DeltaRejection::UnknownType)
        )
    }

    type Probe = dyn Fn(u32) -> bool;
    let probes: Vec<(ObjectKind, &'static [OpDecl], &Probe)> = vec![
        (ObjectKind::Group, pacific_core::group::GroupType::ops(),
         &probe::<pacific_core::group::GroupType>),
        (ObjectKind::Forum, pacific_core::coordinator::ForumType::ops(),
         &probe::<pacific_core::coordinator::ForumType>),
        (ObjectKind::System, pacific_core::system::SystemType::ops(),
         &probe::<pacific_core::system::SystemType>),
        (ObjectKind::Project, pacific_core::project::ProjectType::ops(),
         &probe::<pacific_core::project::ProjectType>),
        (ObjectKind::Contact, pacific_core::contact::ContactType::ops(),
         &probe::<pacific_core::contact::ContactType>),
        (ObjectKind::Conversation, pacific_core::coordinator::ConversationType::ops(),
         &probe::<pacific_core::coordinator::ConversationType>),
        (ObjectKind::Thing, pacific_core::thing::ThingType::ops(),
         &probe::<pacific_core::thing::ThingType>),
        (ObjectKind::Place, pacific_core::place::PlaceType::ops(),
         &probe::<pacific_core::place::PlaceType>),
        (ObjectKind::Event, pacific_core::event::EventType::ops(),
         &probe::<pacific_core::event::EventType>),
        (ObjectKind::Post, pacific_core::post::PostType::ops(),
         &probe::<pacific_core::post::PostType>),
        (ObjectKind::Treasury, pacific_core::treasury::TreasuryType::ops(),
         &probe::<pacific_core::treasury::TreasuryType>),
        (ObjectKind::Note, pacific_core::note::NoteType::ops(),
         &probe::<pacific_core::note::NoteType>),
        (ObjectKind::Host, pacific_core::host::HostType::ops(),
         &probe::<pacific_core::host::HostType>),
        (ObjectKind::Transaction, pacific_core::transaction::TransactionType::ops(),
         &probe::<pacific_core::transaction::TransactionType>),
    ];
    assert_eq!(
        probes.len(),
        tables().len(),
        "a kind gained an op table and this sweep did not learn about it"
    );

    let mut armless: Vec<String> = Vec::new();
    for (kind, ops, p) in &probes {
        for d in *ops {
            if !p(d.op_id) {
                armless.push(format!("{}.{} (op {})", kind.name(), d.name, d.op_id));
            }
        }
    }
    assert!(
        armless.is_empty(),
        "declared with no reducer arm — an op with no reducer is not an op:\n  {}",
        armless.join("\n  ")
    );
}

/// THE ONE THAT WOULD HAVE CAUGHT `post_author`: mint one object of every mintable
/// kind, through the real door, and read the type id off the wire.
///
/// `Node::mint` dispatches op 0 to the kind's own author (`author_profile`), which is
/// exactly where the defect lived — six doors stamped their own kind and the seventh
/// stamped Thing's. Nothing compared them, because each kind's tests only ever minted
/// its own kind and a Post folding to `Default` looks like an empty Post.
#[tokio::test]
async fn every_mint_stamps_its_own_kinds_type_id_on_the_wire() {
    let h = Harness::people(&[("axel", &["phone"])]).await;
    let phone = h.device("axel", "phone");

    let mut minted = 0usize;
    for &kind in ObjectKind::ALL {
        if !mint::is_mintable(kind) {
            continue;
        }
        let draft = MintDraft {
            name: format!("a {}", kind.name()),
            descriptor: "swept by catalogue_wire_values".into(),
            // Required by the Event reducer; harmless everywhere else, since a kind
            // that does not read a field simply does not read it.
            start_ms: 1_770_000_000_000,
            ..MintDraft::default()
        };

        let object_id = h
            .node(phone)
            .mint(kind, &draft)
            .await
            .unwrap_or_else(|e| panic!("minting a `{}` failed: {e:?}", kind.name()));

        let log = h
            .node(phone)
            .dir
            .load_log(&hex::decode(&object_id).unwrap())
            .unwrap();

        // Forum is minted NAME-ONLY — its name rides the MLS GroupContext and there is
        // no op 0 to author — so an empty log is the correct answer for it and only it.
        if mint::profile_args(kind, &draft).is_none() {
            assert!(
                log.is_empty(),
                "`{}` authors no profile delta, so its log should be empty at mint",
                kind.name()
            );
            continue;
        }

        assert!(
            !log.is_empty(),
            "`{}` declares a profile op but minted an empty log",
            kind.name()
        );
        for (_author, envelope) in log {
            let d = decode_delta(&envelope).unwrap();
            assert_eq!(
                d.type_id,
                kind.type_id() as u32,
                "A `{}` delta went onto the wire carrying type id {} — `{}` is {}. \
                 `fold::fold_entries` keeps only deltas whose type id matches the \
                 lens, so this object's whole log is invisible to its own reducer and \
                 it folds to Default forever.",
                kind.name(),
                d.type_id,
                kind.name(),
                kind.type_id()
            );
        }
        minted += 1;
    }

    assert!(
        minted >= 6,
        "only {minted} kinds were swept — `mint::classify` used to admit seven, and a \
         sweep that quietly stops covering kinds proves less each time it runs"
    );
}

/// The op the mint authors is the one the kind's table declares at that id, with the
/// authority the door enforced. Cheap, and it closes the other half of the same seam:
/// a correct type id carrying an op id from a different vocabulary.
#[test]
fn every_mintable_kinds_profile_op_is_declared_by_its_own_table() {
    let draft = MintDraft {
        name: "x".into(),
        start_ms: 1,
        ..MintDraft::default()
    };
    for (kind, ops) in tables() {
        let Some((op_id, _args)) = mint::profile_args(kind, &draft) else {
            continue;
        };
        let decl = ops.iter().find(|d| d.op_id == op_id).unwrap_or_else(|| {
            panic!(
                "`{}`'s mint authors op {op_id}, which its own op table does not declare",
                kind.name()
            )
        });
        assert_eq!(
            decl.op_id, 0,
            "the invariant every kind keeps: op 0 is the profile (`{}` uses {})",
            kind.name(), decl.op_id
        );
        assert_eq!(
            decl.authority,
            Authority::Owner,
            "`{}` is authored at mint by the owner alone",
            decl.name
        );
        assert_eq!(
            decl.commutativity,
            Commutativity::Sequenced,
            "a profile is the head of the spine, not an OR-set entry"
        );
    }
}
