//! `System` — an external source, and the door its items come through.
//!
//! A System is the connector itself as a GroupObject (kind 21): Resident Advisor,
//! ArtRabbit, an employer's directory. Its log carries what that source produced.
//!
//! # Why external items are DELTAS and not view rows
//!
//! Ralph's ruling (7 Aug): *"External hydration is necessary, each arrive as deltas
//! from that System. The items on the feed are a subset of deltas on GroupObjects."*
//!
//! Before this, fetched rows were mapped straight to a view model in memory — nothing
//! folded, nothing durable, no provenance, and the row vanished with the fetch. Every
//! consequence of that was paid downstream: two sources had to be reconciled at
//! display time by a title+day heuristic, a detail page needed a second "reference"
//! mode, and "new since you last looked" had nowhere to live.
//!
//! Arriving as deltas, an external item inherits what the substrate already does:
//! provenance (which System, which fetch), idempotence (re-hydrating an unchanged
//! item is a no-op), retraction (a withdrawn listing is a tombstone that travels),
//! sync, and a fold that needs no model. The feed then SELECTS over one substrate
//! instead of merging several.
//!
//! # One object per System, not one per item
//!
//! Ten thousand listings must not mint ten thousand MLS groups — the same reasoning
//! `contact.rs` gives for keeping a Listing separate from the Thing it announces. So
//! a System owns ONE object and each item is a delta on it. Minting a real object is
//! DEFERRED TO INTEREST: tapping a hydrated row mints the Event (the mint-from-listing
//! path in EVENT-SHARE, with its sourceKey→objectId twin guard). A hydrated item is an
//! announcement; a minted Event is the thing itself.
//!
//! # Op ids
//!
//! `define` is 0, as the catalog has always declared. Ids 1–9 stay RESERVED for the
//! connector op-group the catalog also declares (addField · setConnector ·
//! bindCredential · …), which remains unbuilt — hydration takes 10 so that building
//! those later needs no renumbering. Op ids are wire values: assigned once, never moved.

use std::collections::BTreeMap;

use crate::coordinator::{ArgVal, Args};
use crate::object::{
    Authority, Commutativity, DeltaRejection, MemberId, ObjectKind, ObjectType, Op, OpDecl,
};
use crate::object_args::{opt_text_nonempty as opt_text, req_int, req_text};

// ---- op ids (MUST match pacific-ffi `op::SYSTEM_*`) --------------------------------
pub const OP_DEFINE: u32 = 0; // owner / sequenced
pub const OP_HYDRATE: u32 = 10; // any-member / commutative (per-key LWW by rev)

/// The biggest payload one hydrated item may carry, in bytes of JSON.
///
/// An RA listing is ~1–2KB; 16K is generous for anything a feed row needs and still
/// bounds the damage a chatty or hostile connector can do to a log that syncs to every
/// member. Oversize is refused, never truncated: a silently clipped payload would fail
/// to parse on some devices and not others, which is worse than a loud reject.
pub const MAX_HYDRATED_PAYLOAD: usize = 16_384;

/// One item as its System produced it.
///
/// `payload` is the source's own JSON, kept VERBATIM — the System that wrote it owns
/// its shape, and every reader parses the same bytes. Keeping it opaque here is what
/// lets a new connector ship without a core change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HydratedItem {
    /// The source's stable id for this item, e.g. `ra:event:166016`. THE dedup key:
    /// re-fetching the same listing produces the same key, so hydration is idempotent
    /// and "have I already got this?" is an exact lookup rather than a heuristic.
    pub key: String,
    pub payload: String,
    /// When the connector observed it (epoch ms) — provenance, not validity.
    pub fetched_at: i64,
    /// The source's revision, monotone per key. The LWW key: a later fetch replaces an
    /// earlier one, an equal one is a no-op, an older one never wins a race.
    pub rev: u64,
    /// The source withdrew it (cancelled, delisted). A TOMBSTONE, not a deletion, so
    /// the news of its removal travels the same paths the item did — the discipline
    /// `contact.publishListing` already uses for withdrawn listings.
    pub withdrawn: bool,
    pub author: MemberId,
}

/// The compiled read-side state of a System.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemState {
    /// What a human calls it — "Resident Advisor".
    pub name: String,
    /// The connector id the app dispatches on — "music.ra", "art.artrabbit".
    pub connector: String,
    /// What this System is currently pulling, free text owned by the connector
    /// ("area=218;window=14d"). Opaque to the core on purpose.
    pub scope: String,
    /// key → item, withdrawn ones included (a tombstone is state, not absence).
    pub items: BTreeMap<String, HydratedItem>,
    /// What this object is a part of, and in what role (`base.setParent`). `None`
    /// until a parent names it as a part.
    pub parent: Option<crate::parent::ParentRef>,
}

impl SystemState {
    /// Items still on offer — what a feed shows.
    pub fn live(&self) -> Vec<&HydratedItem> {
        self.items.values().filter(|i| !i.withdrawn).collect()
    }

    pub fn item(&self, key: &str) -> Option<&HydratedItem> {
        self.items.get(key)
    }
}

pub struct SystemType;

const OPS: &[OpDecl] = &[
    OpDecl {
        op_id: OP_DEFINE,
        name: "system.define",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_HYDRATE,
        name: "system.hydrate",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
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

impl ObjectType for SystemType {
    const KIND: ObjectKind = ObjectKind::System;
    type State = SystemState;

    fn ops() -> &'static [OpDecl] {
        OPS
    }

    fn reduce(state: &mut Self::State, op: &Op<'_>) -> Result<(), DeltaRejection> {
        if crate::parent::is_parent_op(op.op_id) {
            return crate::parent::reduce_parent(&mut state.parent, op);
        }
        let args = op.args;
        match op.op_id {
            OP_DEFINE => {
                // VALIDATE EVERYTHING FIRST, THEN COMMIT — a reduce-time rejection is
                // skipped at fold, so a half-applied definition would be the state forever.
                let name = req_text(args, "name")?.to_string();
                let connector = req_text(args, "connector")?.to_string();
                if name.is_empty() || connector.is_empty() {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let scope = opt_text(args, "scope").unwrap_or_default();
                state.name = name;
                state.connector = connector;
                state.scope = scope;
                Ok(())
            }
            OP_HYDRATE => {
                let key = req_text(args, "key")?.to_string();
                let payload = req_text(args, "payload")?.to_string();
                if key.is_empty() || payload.is_empty() {
                    return Err(DeltaRejection::MalformedArgs);
                }
                if payload.len() > MAX_HYDRATED_PAYLOAD {
                    return Err(DeltaRejection::MalformedArgs);
                }
                let fetched_at = match req_int(args, "fetchedAt")? {
                    ms if ms > 0 => ms,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                let rev = match req_int(args, "rev")? {
                    r if r >= 0 => r as u64,
                    _ => return Err(DeltaRejection::MalformedArgs),
                };
                let withdrawn = matches!(crate::arg_reads::get(args, "withdrawn"), Some(ArgVal::Int(w)) if *w != 0);

                // LWW BY REV, and a stale replay is a NO-OP rather than a rejection:
                // two devices hydrating the same source will race constantly, and a
                // loud reject on the loser would poison a fold for a difference that
                // does not matter. Equal revs keep what is held, so the fold is a
                // function of the delta SET and not of arrival order.
                if let Some(held) = state.items.get(&key) {
                    if rev <= held.rev {
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
            _ => Err(DeltaRejection::UnknownType),
        }
    }
}

// ---- args builders (node/FFI author deltas through these) --------------------------

pub fn define_args(name: &str, connector: &str, scope: Option<&str>) -> Args {
    let mut a = Args::new();
    a.insert("name".into(), ArgVal::Text(name.into()));
    a.insert("connector".into(), ArgVal::Text(connector.into()));
    if let Some(s) = scope {
        a.insert("scope".into(), ArgVal::Text(s.into()));
    }
    a
}

/// Build the args for one hydrated item. `payload` is the source's own JSON, passed
/// through untouched.
pub fn hydrate_args(key: &str, payload: &str, fetched_at: i64, rev: u64, withdrawn: bool) -> Args {
    let mut a = Args::new();
    a.insert("key".into(), ArgVal::Text(key.into()));
    a.insert("payload".into(), ArgVal::Text(payload.into()));
    a.insert("fetchedAt".into(), ArgVal::Int(fetched_at));
    a.insert("rev".into(), ArgVal::Int(rev as i64));
    a.insert("withdrawn".into(), ArgVal::Int(withdrawn as i64));
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator::{sequenced_delta, Coordinator, GENESIS_PREV};
    use crate::object::{build_delta, ReduceContext};

    fn owner() -> MemberId {
        [3u8; 32]
    }
    fn other() -> MemberId {
        [4u8; 32]
    }

    fn folded(seq: Vec<(u32, Args)>, comm: Vec<(u32, Args, MemberId, u64)>) -> SystemState {
        let mut c: Coordinator<SystemType> = Coordinator::new(vec![owner(), other()], owner());
        let mut prev = GENESIS_PREV;
        for (i, (op_id, a)) in seq.into_iter().enumerate() {
            let d = sequenced_delta(
                ObjectKind::System.type_id() as u32,
                op_id,
                a,
                0,
                i as u64,
                prev,
            );
            prev = d.id();
            c.deliver(d, owner()).expect("owner-sequenced accepted");
        }
        for (op_id, a, author, gen) in comm {
            let d = build_delta(ObjectKind::System, op_id, a, 0, Some(gen));
            c.deliver(d, author).expect("commutative accepted");
        }
        c.state()
    }

    fn item(key: &str, rev: u64) -> Args {
        hydrate_args(
            key,
            &format!("{{\"id\":\"{key}\",\"rev\":{rev}}}"),
            1_700_000_000_000,
            rev,
            false,
        )
    }

    #[test]
    fn a_system_defines_itself_and_holds_its_items() {
        let st = folded(
            vec![(
                OP_DEFINE,
                define_args("Resident Advisor", "music.ra", Some("area=218;window=14d")),
            )],
            vec![
                (OP_HYDRATE, item("ra:event:1", 1), owner(), 0),
                (OP_HYDRATE, item("ra:event:2", 1), owner(), 1),
            ],
        );
        assert_eq!(st.name, "Resident Advisor");
        assert_eq!(st.connector, "music.ra");
        assert_eq!(st.scope, "area=218;window=14d");
        assert_eq!(st.live().len(), 2);
    }

    /// Re-hydration is the STEADY STATE — a connector refetches the same listings every
    /// pass. An unchanged item must not grow the state or churn the fold.
    #[test]
    fn re_hydrating_an_unchanged_item_is_a_no_op() {
        let st = folded(
            vec![],
            vec![
                (OP_HYDRATE, item("ra:event:1", 3), owner(), 0),
                (OP_HYDRATE, item("ra:event:1", 3), owner(), 1),
                (OP_HYDRATE, item("ra:event:1", 2), other(), 2), // an older fetch, racing
            ],
        );
        assert_eq!(st.items.len(), 1);
        assert_eq!(st.items["ra:event:1"].rev, 3, "an older rev never wins");
    }

    /// A newer rev replaces; a withdrawal is a tombstone that keeps the row addressable
    /// so the news of its removal can travel.
    #[test]
    fn a_newer_rev_replaces_and_a_withdrawal_tombstones() {
        let mut updated = item("ra:event:1", 2);
        updated.insert(
            "payload".into(),
            ArgVal::Text("{\"id\":\"ra:event:1\",\"t\":\"moved\"}".into()),
        );
        let st = folded(
            vec![],
            vec![
                (OP_HYDRATE, item("ra:event:1", 1), owner(), 0),
                (OP_HYDRATE, updated, owner(), 1),
                (
                    OP_HYDRATE,
                    hydrate_args("ra:event:9", "{\"id\":\"9\"}", 1_700_000_000_000, 4, true),
                    owner(),
                    2,
                ),
            ],
        );
        assert!(st.items["ra:event:1"].payload.contains("moved"));
        assert_eq!(st.items.len(), 2, "the withdrawn item is still addressable");
        assert_eq!(st.live().len(), 1, "but it is not live");
        assert!(st.item("ra:event:9").unwrap().withdrawn);
    }

    /// Loud, not lenient: an oversize payload is refused, never clipped — a truncated
    /// JSON would parse on no device and look like a connector bug on every one.
    #[test]
    fn an_oversize_payload_is_refused_not_truncated() {
        let mut st = SystemState::default();
        let ctx = ReduceContext {
            members: &[owner()],
            owner: owner(),
            epoch: 0,
        };
        let big = "x".repeat(MAX_HYDRATED_PAYLOAD + 1);
        let a = hydrate_args("ra:event:1", &big, 1_700_000_000_000, 1, false);
        let op = Op {
            op_id: OP_HYDRATE,
            args: &a,
            author: &owner(),
            pos: None,
            ctx: &ctx,
        };
        assert_eq!(
            SystemType::reduce(&mut st, &op),
            Err(DeltaRejection::MalformedArgs)
        );
        assert!(st.items.is_empty(), "a rejected delta must not half-apply");

        // …and the empties, which a lazy connector will send.
        for bad in [
            hydrate_args("", "{}", 1_700_000_000_000, 1, false),
            hydrate_args("k", "", 1_700_000_000_000, 1, false),
            hydrate_args("k", "{}", 0, 1, false),
        ] {
            let op = Op {
                op_id: OP_HYDRATE,
                args: &bad,
                author: &owner(),
                pos: None,
                ctx: &ctx,
            };
            assert_eq!(
                SystemType::reduce(&mut st, &op),
                Err(DeltaRejection::MalformedArgs)
            );
        }
    }

    /// The fold is a function of the delta SET, not arrival order — two devices
    /// hydrating the same source in opposite orders must agree.
    #[test]
    fn hydration_is_order_independent() {
        let a = folded(
            vec![],
            vec![
                (OP_HYDRATE, item("k1", 1), owner(), 0),
                (OP_HYDRATE, item("k1", 5), other(), 1),
                (OP_HYDRATE, item("k2", 2), owner(), 2),
            ],
        );
        let b = folded(
            vec![],
            vec![
                (OP_HYDRATE, item("k2", 2), owner(), 2),
                (OP_HYDRATE, item("k1", 5), other(), 1),
                (OP_HYDRATE, item("k1", 1), owner(), 0),
            ],
        );
        assert_eq!(a, b);
        assert_eq!(a.items["k1"].rev, 5);
    }

    #[test]
    fn every_op_satisfies_the_spec_invariant() {
        for d in SystemType::ops() {
            assert!(
                d.is_well_formed(),
                "op {} violates the spec invariant",
                d.name
            );
        }
        assert_eq!(ObjectKind::System.type_id(), 21);
        assert_eq!(ObjectKind::from_type_id(21), Some(ObjectKind::System));
    }
}
