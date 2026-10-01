//! host — a Site's public copy, and the only object an Arc joins (kind 33).
//!
//! ## Why a second object exists at all
//!
//! The obvious design — let the Arc read the group and render it — is the one thing
//! the architecture forbids. An Arc that could fold a group's log would hold that
//! group's history in plaintext, which is the breach The Seam exists to prevent. So
//! the Arc is never added to a group. But something has to hand bytes to a browser,
//! and it has to be a full member of SOMETHING: a non-member cannot decrypt, cannot
//! fold, and cannot be removed.
//!
//! A Host resolves that. It is a second GroupObject, minted by the group's owner,
//! whose roster is the owner's devices and the Arc. The public face is copied onto
//! it deliberately, by a Delta the group can see and revoke. What the Arc can see is
//! exactly what somebody typed into a page they knew was public — and seeing it is
//! MEMBERSHIP, which means the group can take it back with an MLS Remove.
//!
//! The group names its Host; the Host carries the copy; the Arc serves the copy.
//! Three objects, one of which the Arc is a member of.
//!
//! ## Why it is a kind and not a System wearing a connector
//!
//! It shipped on branch `host-faces` (20 September 2026, core 924 green) as
//! `System`(21) with `connector == "wallflowers.host"`, because the taxonomy was held
//! closed and a connector string was the existing extension point. Every host-only op
//! then had to ask `state.is_host()` at reduce time and answer `PreconditionFailed`
//! for a System that was not one.
//!
//! Ruled 25 September 2026: a Host is a GroupObject of its own, as Treasury and Note
//! are. The gate disappears rather than being reimplemented — a Host cannot fail to
//! be a Host, because being one is the kind. That is the same dividend the note
//! collapse paid when `Refusal::NoteOffNote` stopped being necessary.
//!
//! ## What is carried over unchanged
//!
//! The RULES are the branch's, verbatim: the media slots, the mime vocabulary, the
//! 160,000-byte base64 ceiling and the reason for it, and hydration's per-key LWW by
//! `rev` with a tombstone rather than a deletion. What changed is their home and
//! their op ids — this kind's own table, numbered from 0, as the ICD declares it.

use std::collections::BTreeMap;

use crate::coordinator::Args;
use crate::object::{
    Authority, Commutativity, DeltaRejection, MemberId, ObjectKind, ObjectType, Op, OpDecl,
};
use crate::object_args::{opt_text, req_int, req_text};
use crate::publication;
use crate::system::{HydratedItem, MAX_HYDRATED_PAYLOAD};

/// Name it. On the branch this also carried the connector string that made the
/// host-only ops legal; the kind is that statement now.
pub const OP_DEFINE: u32 = 0;
/// The public copy: key `face` is the bundle verbatim, `event:<id>` / `post:<id>`
/// are live items. Per-key LWW by `rev`.
pub const OP_HYDRATE: u32 = 1;
/// One picture per slot.
pub const OP_SET_MEDIA: u32 = 2;
/// A slot's picture, edited by the owner or an admin of this Host (ICD 2.1.0 row 10).
pub const OP_EDIT_MEDIA: u32 = 3;

/// The picture slots a Host serves. A closed vocabulary: a slot nobody renders is a
/// slot that silently does nothing.
pub const MEDIA_SLOTS: &[&str] = &["mark", "cover", "logo", "wallpaper"];

/// NO SVG, deliberately: an SVG is a document and can carry script, and these bytes
/// are served to anyone with the address, with no account and no protocol.
pub const MEDIA_MIMES: &[&str] = &["image/jpeg", "image/png", "image/webp"];

/// The biggest picture a Host carries, in bytes of base64.
///
/// Not a nicety. A delta bigger than the relay's 256 KiB publish is written locally
/// and never delivered, which leaves a hole in the object's chain and stops every
/// OTHER member folding it at all. Refusing an oversize picture is what keeps one
/// from breaking the face for everybody.
///
/// So it sits under `node::MAX_DELTA_ENVELOPE_BYTES` (157,286), the largest envelope a
/// device will author, with a kilobyte for the rest of the `host.setMedia` envelope.
/// `node` is not in every build of this crate, so the number is stated here and pinned
/// to the ceiling by `the_ceiling_is_derived_from_the_relays_own_default`. It was
/// 160,000 until 26 Sep 2026, above the ceiling: a picture at the cap could not be sent.
pub const MAX_MEDIA_B64: usize = 156_000;

/// A picture in one slot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Media {
    pub mime: String,
    pub data: String,
}

/// THE ONE MEDIA RULE, read by the door and by the fold — `Node::host_publish`
/// refuses on it before anything is written, and [`HostType::reduce`] refuses on it
/// again for a client that skipped the door. Two callers, one rule, so they cannot
/// drift.
pub fn check_media(slot: &str, data: &str, mime: &str) -> Result<(), String> {
    if !MEDIA_SLOTS.contains(&slot) {
        return Err(format!("'{slot}' is not a picture slot ({})", MEDIA_SLOTS.join(", ")));
    }
    if data.is_empty() {
        return Ok(()); // clearing a slot needs no mime and no bytes
    }
    if !MEDIA_MIMES.contains(&mime) {
        return Err(format!(
            "'{mime}' is not a picture a Host serves ({})",
            MEDIA_MIMES.join(", ")
        ));
    }
    if data.len() > MAX_MEDIA_B64 {
        return Err(format!(
            "that picture is {} of base64; the most a Host carries is {MAX_MEDIA_B64}",
            data.len()
        ));
    }
    use base64::Engine as _;
    if base64::engine::general_purpose::STANDARD.decode(data).is_err() {
        return Err("that picture is not base64".into());
    }
    Ok(())
}

/// The compiled read-side state of a Host: what the Arc folds and serves.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostState {
    /// What a human calls it.
    pub name: String,
    /// key → item, withdrawn ones included (a tombstone is state, not absence).
    /// Key `face` is the bundle; `event:<id>` and `post:<id>` are live items.
    pub items: BTreeMap<String, HydratedItem>,
    /// The public address and the Arc that serves it. Unpublished by default.
    pub publication: publication::Publication,
    /// slot → picture.
    pub media: BTreeMap<String, Media>,
    /// What this object is a part of, and in what role (`base.setParent`). `None`
    /// until a parent names it as a part.
    pub parent: Option<crate::parent::ParentRef>,
    /// Each member's standing on THIS Host (the roles facet, ICD 2.1.0 row 10): an admin
    /// of the Site is added to the Host's roster and made admin here too, since no fold
    /// reads another object.
    pub member_roles: BTreeMap<MemberId, crate::group::GroupRole>,
}

impl HostState {
    /// Items still on offer — what the Arc renders.
    pub fn live(&self) -> Vec<&HydratedItem> {
        self.items.values().filter(|i| !i.withdrawn).collect()
    }

    /// The face bundle, if one has been copied across the seam yet.
    pub fn face(&self) -> Option<&HydratedItem> {
        self.items.get("face").filter(|i| !i.withdrawn)
    }
}

pub static OPS: &[OpDecl] = &[
    OpDecl {
        op_id: OP_DEFINE,
        name: "host.define",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_HYDRATE,
        name: "host.hydrate",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_SET_MEDIA,
        name: "host.setMedia",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_EDIT_MEDIA,
        name: "host.editMedia",
        authority: Authority::OwnerOrRole(crate::group::GroupRole::Admin),
        commutativity: Commutativity::Commutative,
    },
    // The BASE standing op-group, spliced — see `crate::roles` (ICD 2.1.0 row 10).
    OpDecl {
        op_id: crate::roles::OP_SET_ROLE,
        name: "base.setRole",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: crate::roles::OP_CLEAR_ROLE,
        name: "base.clearRole",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    // The publication facet, spliced. A Host is the object that HAS a public
    // address — `base.publish {slug, publisher}` names the Arc that serves it, and
    // the publisher must already be on this roster, which is what makes serving
    // visible and revocable: remove the member and the serving stops.
    OpDecl {
        op_id: publication::OP_PUBLISH,
        name: "base.publish",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: publication::OP_UNPUBLISH,
        name: "base.unpublish",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    // The base PARENT ops: any kind can be a part (ICD `facets.parent`), and the
    // part names what it is part of so `part_of` walks from this end too.
    OpDecl {
        op_id: crate::parent::OP_SET_PARENT,
        name: "base.setParent",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: crate::parent::OP_CLEAR_PARENT,
        name: "base.clearParent",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
];

pub struct HostType;

impl ObjectType for HostType {
    const KIND: ObjectKind = ObjectKind::Host;
    type State = HostState;

    fn ops() -> &'static [OpDecl] {
        OPS
    }

    fn role_of(state: &HostState, member: &MemberId) -> Option<crate::group::GroupRole> {
        state.member_roles.get(member).copied()
    }

    fn reduce(state: &mut Self::State, op: &Op<'_>) -> Result<(), DeltaRejection> {
        if crate::parent::is_parent_op(op.op_id) {
            return crate::parent::reduce_parent(&mut state.parent, op);
        }
        if crate::roles::is_role_op(op.op_id) {
            return crate::roles::reduce_roles(&mut state.member_roles, op);
        }
        // The publication facet, in its reserved band, before this kind's own ops.
        if publication::is_publication_op(op.op_id) {
            publication::reduce_publication(&mut state.publication, op)?;
            // NAMING A PUBLISHER DISOWNS WHATEVER IT ALREADY WROTE. The gate on
            // hydrate refuses the server that serves this face, but a server could
            // have written one BEFORE it was named, with a rev nothing can beat —
            // rev is the author's own number. Publication rides the spine, which
            // folds first, so whichever order the deltas arrived in the state is
            // the same.
            if let Some(publisher) = state.publication.publisher {
                state.items.retain(|_, held| held.author != publisher);
            }
            return Ok(());
        }
        let args = op.args;
        match op.op_id {
            OP_DEFINE => {
                let name = req_text(args, "name")?.to_string();
                if name.is_empty() {
                    return Err(DeltaRejection::MalformedArgs);
                }
                state.name = name;
                Ok(())
            }
            OP_HYDRATE => {
                // A SERVER MAY NOT AUTHOR THE FACE IT SERVES. The Arc sits on the
                // roster to READ it, and hydrate is any-member, so without this the
                // machine publishing the page could write it under the group's name.
                // Refused at fold (ICD `principals.arc`), so a patched Arc gains
                // nothing by skipping the door.
                if state.publication.publisher.as_ref() == Some(op.author) {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                // Validate every arg before the first mutation — reduce is atomic.
                let key = req_text(args, "key")?.to_string();
                let payload = req_text(args, "payload")?.to_string();
                if payload.len() > MAX_HYDRATED_PAYLOAD {
                    // REFUSED, never truncated: a silently clipped payload would parse
                    // on some devices and not others.
                    return Err(DeltaRejection::MalformedArgs);
                }
                let fetched_at = req_int(args, "fetchedAt")?;
                let rev = req_int(args, "rev")?;
                if rev < 0 {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let rev = rev as u64;
                let withdrawn = opt_text(args, "withdrawn").map(|w| w == "1").unwrap_or(false);

                // A STALE REPLAY IS A NO-OP, not a rejection: two devices hydrating the
                // same source race constantly, and a loud reject on the loser would
                // poison a fold over a difference that does not matter. Equal revs keep
                // what is held, so the fold is a function of the delta SET and not of
                // arrival order.
                if let Some(held) = state.items.get(&key) {
                    if held.rev >= rev {
                        return Ok(());
                    }
                }
                state.items.insert(
                    key.clone(),
                    HydratedItem {
                        key,
                        payload,
                        fetched_at,
                        rev,
                        withdrawn,
                        author: *op.author,
                    },
                );
                Ok(())
            }
            OP_SET_MEDIA => set_media(state, args),
            OP_EDIT_MEDIA => {
                // THE SHARED PICTURES (ICD 2.1.0 row 10): per slot, one LWW register by
                // (gen, author), folded after the spine in (gen, author, id) order, so the
                // last write that counts is the slot's picture, and with none counting the
                // sequenced one stands. It counts from the owner, or from a member on this
                // Host's roster holding admin in THIS Host's roles; a revoked admin's writes
                // stop counting (R1). `gen` is declared required, and refused absent, as
                // forum.retract's.
                if !matches!(crate::arg_reads::get(args, "gen"), Some(crate::coordinator::ArgVal::Int(_))) {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let author = op.author;
                let admin = op.ctx.is_member(author) && state.member_roles.get(author) == Some(&crate::group::GroupRole::Admin);
                if *author != op.ctx.owner && !admin {
                    return Err(DeltaRejection::PreconditionFailed);
                }
                set_media(state, args)
            }
            _ => Err(DeltaRejection::UnknownType),
        }
    }
}

/// One slot's picture, as host.setMedia and host.editMedia both take it. Validate
/// everything, then commit: a reduce-time rejection is skipped at fold, so a half-applied
/// picture would be the state forever.
fn set_media(state: &mut HostState, args: &Args) -> Result<(), DeltaRejection> {
    let slot = req_text(args, "slot")?;
    let data = req_text(args, "media")?;
    let mime = if data.is_empty() { "" } else { req_text(args, "mediaMime")? };
    // The same rule the door read — bytes every reader can decode, or nothing: the Arc
    // serves these as they are, and a picture only some devices can read is worse than none.
    if check_media(slot, data, mime).is_err() {
        return Err(DeltaRejection::MalformedArgs);
    }
    if data.is_empty() {
        state.media.remove(slot);
        return Ok(());
    }
    state.media.insert(slot.to_string(), Media { mime: mime.to_string(), data: data.to_string() });
    Ok(())
}

// ---- args builders (node/FFI author deltas through these) -------------------------

pub fn define_args(name: &str) -> Args {
    let mut a = Args::new();
    a.insert("name".into(), crate::coordinator::ArgVal::Text(name.into()));
    a
}

pub fn hydrate_args(key: &str, payload: &str, fetched_at: i64, rev: u64, withdrawn: bool) -> Args {
    let mut a = Args::new();
    a.insert("key".into(), crate::coordinator::ArgVal::Text(key.into()));
    a.insert("payload".into(), crate::coordinator::ArgVal::Text(payload.into()));
    a.insert("fetchedAt".into(), crate::coordinator::ArgVal::Int(fetched_at));
    a.insert("rev".into(), crate::coordinator::ArgVal::Int(rev as i64));
    if withdrawn {
        a.insert("withdrawn".into(), crate::coordinator::ArgVal::Text("1".into()));
    }
    a
}

pub fn set_media_args(slot: &str, data: &str, mime: &str) -> Args {
    let mut a = Args::new();
    a.insert("slot".into(), crate::coordinator::ArgVal::Text(slot.into()));
    a.insert("media".into(), crate::coordinator::ArgVal::Text(data.into()));
    a.insert("mediaMime".into(), crate::coordinator::ArgVal::Text(mime.into()));
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::ReduceContext;

    const ME: MemberId = [7u8; 32];
    static ROSTER: &[MemberId] = &[ME];
    const PNG_B64: &str = "iVBORw0KGgo=";

    fn ctx() -> ReduceContext<'static> {
        ReduceContext { members: ROSTER, owner: ME, epoch: 0 }
    }
    fn op<'a>(op_id: u32, args: &'a Args, c: &'a ReduceContext<'a>) -> Op<'a> {
        Op { op_id, args, author: &ME, pos: None, ctx: c }
    }

    /// The gate that is GONE. On the branch every host-only op asked `is_host()` and
    /// answered PreconditionFailed for a System that was not one. A Host cannot fail
    /// to be a Host, so setMedia needs no such check and this test records that the
    /// op works on a bare, undefined Host.
    #[test]
    fn a_host_needs_no_connector_to_be_one() {
        let (c, mut st) = (ctx(), HostState::default());
        let a = set_media_args("mark", PNG_B64, "image/png");
        HostType::reduce(&mut st, &op(OP_SET_MEDIA, &a, &c)).unwrap();
        assert_eq!(st.media["mark"].mime, "image/png");
    }

    #[test]
    fn the_slot_and_mime_vocabularies_are_closed() {
        let (c, mut st) = (ctx(), HostState::default());
        for (slot, mime) in [("banner", "image/png"), ("mark", "image/svg+xml")] {
            let a = set_media_args(slot, PNG_B64, mime);
            assert_eq!(
                HostType::reduce(&mut st, &op(OP_SET_MEDIA, &a, &c)),
                Err(DeltaRejection::MalformedArgs),
                "{slot}/{mime} must be refused"
            );
        }
        assert!(st.media.is_empty(), "reduce must be atomic");
    }

    /// The size cap is a CHAIN-SAFETY rule, not a nicety: a delta over the relay's
    /// publish ceiling is written locally and never delivered, and the hole stops
    /// every other member folding the object at all.
    #[test]
    fn an_oversize_picture_is_refused_rather_than_clipped() {
        let (c, mut st) = (ctx(), HostState::default());
        let big = "A".repeat(MAX_MEDIA_B64 + 4);
        let a = set_media_args("cover", &big, "image/jpeg");
        assert_eq!(
            HostType::reduce(&mut st, &op(OP_SET_MEDIA, &a, &c)),
            Err(DeltaRejection::MalformedArgs)
        );
        assert!(st.media.is_empty());
    }

    #[test]
    fn empty_data_clears_the_slot() {
        let (c, mut st) = (ctx(), HostState::default());
        let set = set_media_args("logo", PNG_B64, "image/png");
        HostType::reduce(&mut st, &op(OP_SET_MEDIA, &set, &c)).unwrap();
        let clear = set_media_args("logo", "", "");
        HostType::reduce(&mut st, &op(OP_SET_MEDIA, &clear, &c)).unwrap();
        assert!(st.media.is_empty());
    }

    /// Per-key LWW by rev, and a stale replay is a NO-OP rather than a rejection —
    /// two devices hydrating the same source race constantly.
    #[test]
    fn hydration_is_lww_by_rev_and_a_stale_replay_does_not_reject() {
        let (c, mut st) = (ctx(), HostState::default());
        for (rev, body) in [(1u64, "{\"v\":1}"), (3, "{\"v\":3}"), (2, "{\"v\":2}")] {
            let a = hydrate_args("face", body, 0, rev, false);
            HostType::reduce(&mut st, &op(OP_HYDRATE, &a, &c)).expect("never rejects");
        }
        assert_eq!(st.face().expect("a face").payload, "{\"v\":3}", "the highest rev wins");
        assert_eq!(st.items.len(), 1);
    }

    /// A withdrawal is a TOMBSTONE that travels, not an absence.
    #[test]
    fn a_withdrawn_item_is_held_rather_than_removed() {
        let (c, mut st) = (ctx(), HostState::default());
        let a = hydrate_args("event:1", "{}", 0, 1, false);
        HostType::reduce(&mut st, &op(OP_HYDRATE, &a, &c)).unwrap();
        let w = hydrate_args("event:1", "{}", 0, 2, true);
        HostType::reduce(&mut st, &op(OP_HYDRATE, &w, &c)).unwrap();
        assert_eq!(st.items.len(), 1, "the tombstone is state");
        assert!(st.live().is_empty(), "and it is not live");
    }

    #[test]
    fn an_oversize_payload_is_refused_rather_than_truncated() {
        let (c, mut st) = (ctx(), HostState::default());
        let big = "x".repeat(MAX_HYDRATED_PAYLOAD + 1);
        let a = hydrate_args("face", &big, 0, 1, false);
        assert_eq!(
            HostType::reduce(&mut st, &op(OP_HYDRATE, &a, &c)),
            Err(DeltaRejection::MalformedArgs)
        );
        assert!(st.items.is_empty());
    }

    /// The Arc named as publisher cannot hydrate the Host it serves, and naming it
    /// disowns what it wrote before — the same state whichever came first.
    #[test]
    fn the_publisher_cannot_author_the_face_it_serves() {
        const ARC: MemberId = [9u8; 32];
        static BOTH: &[MemberId] = &[ME, ARC];
        let c = ReduceContext { members: BOTH, owner: ME, epoch: 0 };
        let as_arc = |op_id, a: &Args| HostType::reduce(&mut HostState::default(), &Op { op_id, args: a, author: &ARC, pos: None, ctx: &c });
        let mut st = HostState::default();
        let early = hydrate_args("face", "{\"by\":\"arc\"}", 0, 99, false);
        HostType::reduce(&mut st, &Op { op_id: OP_HYDRATE, args: &early, author: &ARC, pos: None, ctx: &c }).unwrap();
        let named = publication::publish_args("allotments", &ARC);
        HostType::reduce(&mut st, &Op { op_id: publication::OP_PUBLISH, args: &named, author: &ME, pos: None, ctx: &c }).unwrap();
        assert!(st.face().is_none(), "naming the Arc disowns the face it wrote before");
        let late = hydrate_args("face", "{}", 0, 100, false);
        assert_eq!(
            HostType::reduce(&mut st, &Op { op_id: OP_HYDRATE, args: &late, author: &ARC, pos: None, ctx: &c }),
            Err(DeltaRejection::PreconditionFailed),
            "and refuses what it writes after"
        );
        assert!(as_arc(OP_HYDRATE, &late).is_ok(), "an Arc not named publisher is any other member");
    }

    /// The kind's OWN ops are contiguous from 0. Facet ops are excluded: they live
    /// in reserved high bands and belong to the facet, not to Host. I wrote this as a
    /// whole-list pin and it broke the moment the publication facet was spliced in —
    /// the same defect I had already fixed three times today in older tests.
    #[test]
    fn the_table_is_this_kinds_own_numbered_from_zero() {
        let own: Vec<u32> = OPS.iter().map(|d| d.op_id).filter(|id| *id < 0x1000).collect();
        assert_eq!(own, vec![OP_DEFINE, OP_HYDRATE, OP_SET_MEDIA, OP_EDIT_MEDIA]);
        for (i, id) in own.iter().enumerate() {
            assert_eq!(*id, i as u32, "op {id} is out of place");
        }
        assert_eq!(HostType::KIND.type_id(), 33);
    }

    /// And the facet it carries, named rather than enumerated by position.
    #[test]
    fn a_host_carries_the_publication_facet() {
        for op in [publication::OP_PUBLISH, publication::OP_UNPUBLISH] {
            assert!(
                OPS.iter().any(|d| d.op_id == op),
                "a Host is the object that HAS a public address"
            );
        }
    }
}
