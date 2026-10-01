//! visibility — WHO MAY KNOW THIS OBJECT EXISTS. A common base facet, spliced
//! into a kind's `ops()` exactly as [`crate::geo`] splices location.
//!
//! NOT MEMBERSHIP, NOT ACCESS. The MLS roster says who is IN an object; a Place's
//! `access` says who may JOIN it (the doorbell's auto-admit); visibility says how
//! far the fact of its existence may TRAVEL — what your responder advertises in a
//! discovery answer, and how far that answer may be relayed. The three used to be
//! entangled ("public" meant both joinable and advertised); the n-2 cache is what
//! forced them apart, because an advertisement now OUTLIVES the moment it was
//! made: it sits in a friend-of-friend's fold until you say something newer.
//!
//! The dial is graded by DISTANCE, the same unit the mesh thinks in:
//!
//!   private       members only. Never advertised, never in an answer.
//!   connections   my direct peers may learn it exists (n-1). An answer carrying
//!                 it must not be relayed onward — enforced at fold by every
//!                 honest device, like a `private` listing's relay ban.
//!   network       the full discovery reach (n-2 today).
//!
//! Tightening heals outward on the existing machinery: the next reconcile
//! re-answers any live question with the reduced set (an answer replaces its
//! predecessor WHOLESALE), and every asker's app-load warm refreshes what their
//! cache holds. No retraction op is needed — the newest statement wins.

use serde::{Deserialize, Serialize};

use crate::coordinator::{ArgVal, Args};
use crate::object::{Authority, Commutativity, DeltaRejection, Op, OpDecl};
use crate::object_args::req_text;

/// How far the fact of an object's existence may travel. Ordered: each level
/// contains the ones below it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    /// Members only — the safe default: nothing is advertised because nobody said
    /// it could be. (Also what a missing field deserializes to, so an item from a
    /// build that predates visibility can never travel FURTHER than intended.)
    #[default]
    Private,
    /// My direct connections (n-1). Advertised in direct answers; never relayed.
    Connections,
    /// The whole discovery reach (n-2). Advertised and relayable.
    Network,
}

impl Visibility {
    pub fn parse(s: &str) -> Result<Self, DeltaRejection> {
        Ok(match s {
            "private" => Self::Private,
            "connections" => Self::Connections,
            "network" => Self::Network,
            _ => return Err(DeltaRejection::MalformedArgs),
        })
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Connections => "connections",
            Self::Network => "network",
        }
    }

    /// The furthest holder-distance an asker may sit at and still be shown this
    /// object — the ONE place the levels become hops. An item is included in (or
    /// may remain in) an answer iff `reach_hops() >= `the answer's distance.
    pub const fn reach_hops(self) -> u32 {
        match self {
            Self::Private => 0,
            Self::Connections => 1,
            Self::Network => crate::contact::MAX_DISCOVER_HOPS,
        }
    }
}

/// MAY A DEVICE RELAY THIS DELTA ONWARD, having just received it at `hops` distance?
///
/// THE one rule. Relay is decided per DELTA at the moment of receipt — a device holding a
/// message it has just been handed cannot fold the object's state first to find out
/// whether it is allowed to forward it, so the permission rides the envelope
/// (`Delta::visibility`) and this function is what reads it.
///
/// `hops` is how far the delta has ALREADY travelled: 0 is a direct copy from the author,
/// 1 is one relay, and so on. A delta may be passed on iff the distance it would then be
/// at is still within its own reach:
///
/// ```text
///   private      0    never leaves the members. Not advertised, not relayed.
///   connections  1    my direct peers may learn it. They may NOT pass it on.
///   network      2    the full discovery reach (MAX_DISCOVER_HOPS).
/// ```
///
/// Note what this deliberately does NOT do: it never asks who the recipient is. Relay is
/// a question about DISTANCE, not identity — an honest device cannot know whether the
/// next hop is a friend, and a rule that depended on that would be unenforceable.
///
/// It is the same ceiling `contact.rs` already applies to discovery answers
/// (`reach_hops`), so an object's advertisement and the deltas that carry it cannot
/// disagree about how far either may go.
pub const fn may_relay(vis: Visibility, hops: u32) -> bool {
    // Forwarding puts it at hops + 1; that has to remain within reach.
    hops + 1 <= vis.reach_hops()
}

// ---- the base visibility op-group (common to every GroupObject) --------------

/// Reserved base-op band, beside geo's `0xF000_xxxx` and membership's `0xF001_xxxx`.
pub const OP_SET_VISIBILITY: u32 = 0xF002_0000;

/// The base visibility op, to be included in a type's `ops()`. Owner/sequenced:
/// how far an object's existence travels is the owner's assertion about their own
/// object, exactly as its location is.
pub static VISIBILITY_OPS: &[OpDecl] = &[OpDecl {
    op_id: OP_SET_VISIBILITY,
    name: "base.setVisibility",
    authority: Authority::Owner,
    commutativity: Commutativity::Sequenced,
}];

/// True if `op_id` is the base visibility op — so an object can route it to
/// [`reduce_visibility`] before its own op match.
pub fn is_visibility_op(op_id: u32) -> bool {
    op_id == OP_SET_VISIBILITY
}

/// Fold the base visibility op into an object's single-slot facet. Parsed before
/// assignment: an unknown value is a malformed delta, never a silent fallback to
/// `private` (which would look like the owner went dark) or to `network` (which
/// would advertise by typo).
pub fn reduce_visibility(slot: &mut Visibility, op: &Op<'_>) -> Result<(), DeltaRejection> {
    match op.op_id {
        OP_SET_VISIBILITY => {
            *slot = Visibility::parse(req_text(op.args, "visibility")?)?;
            Ok(())
        }
        _ => Err(DeltaRejection::UnknownType),
    }
}

/// W-98's numbers and words, read from the ICD at build (build.rs).
pub(crate) mod icd {
    include!(concat!(env!("OUT_DIR"), "/icd_w98.rs"));
}

/// A kind's visibility when none was written: the ICD's `facets.visibility.defaults` (NC-139,
/// Ralph 30 Sep: an event or post with none is `network`, as every one was under O-48; a
/// place's stays private). A kind the ICD names no default for is private.
pub fn default_for(kind: &str) -> Visibility {
    icd::VISIBILITY_DEFAULTS
        .iter()
        .find(|(k, _)| *k == kind)
        .and_then(|(_, v)| Visibility::parse(v).ok())
        .unwrap_or_default()
}

/// The one text arg a `base.setVisibility` delta carries.
pub fn set_visibility_args(v: Visibility) -> Args {
    let mut a = Args::new();
    a.insert("visibility".into(), ArgVal::Text(v.as_str().to_string()));
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::ReduceContext;

    const OWNER: [u8; 32] = [7u8; 32];

    fn apply(slot: &mut Visibility, args: &Args) -> Result<(), DeltaRejection> {
        let members = [OWNER];
        let ctx = ReduceContext {
            members: &members,
            owner: OWNER,
            epoch: 0,
        };
        let op = Op {
            op_id: OP_SET_VISIBILITY,
            args,
            author: &OWNER,
            pos: None,
            ctx: &ctx,
        };
        reduce_visibility(slot, &op)
    }

    #[test]
    fn private_by_default_and_levels_are_ordered_by_reach() {
        assert_eq!(Visibility::default(), Visibility::Private);
        assert!(Visibility::Private < Visibility::Connections);
        assert!(Visibility::Connections < Visibility::Network);
        assert_eq!(Visibility::Private.reach_hops(), 0);
        assert_eq!(Visibility::Connections.reach_hops(), 1);
        assert_eq!(
            Visibility::Network.reach_hops(),
            crate::contact::MAX_DISCOVER_HOPS
        );
    }

    #[test]
    fn set_widens_and_tightens_and_a_typo_changes_nothing() {
        let mut v = Visibility::default();
        apply(&mut v, &set_visibility_args(Visibility::Network)).unwrap();
        assert_eq!(v, Visibility::Network);
        apply(&mut v, &set_visibility_args(Visibility::Connections)).unwrap();
        assert_eq!(
            v,
            Visibility::Connections,
            "tightening is just re-authoring"
        );

        let mut bad = Args::new();
        bad.insert("visibility".into(), ArgVal::Text("everyone".into()));
        assert_eq!(apply(&mut v, &bad), Err(DeltaRejection::MalformedArgs));
        assert_eq!(v, Visibility::Connections, "the typo left the dial alone");
    }

    #[test]
    fn a_missing_wire_field_deserializes_to_private() {
        // Forward compatibility with builds that predate visibility: an item
        // without the field must never travel further than PRIVATE allows.
        #[derive(serde::Deserialize)]
        struct Probe {
            #[serde(default)]
            vis: Visibility,
        }
        let p: Probe = serde_json::from_str("{}").unwrap();
        assert_eq!(p.vis, Visibility::Private);
    }
}

#[cfg(test)]
mod relay_tests {
    use super::*;

    /// A private delta never moves. This is the one that matters: everything authored
    /// without an explicit audience is private, so the default behaviour of the whole
    /// system is "does not travel".
    #[test]
    fn private_never_relays_at_any_distance() {
        for hops in 0..4 {
            assert!(!may_relay(Visibility::Private, hops));
        }
    }

    /// `connections` reaches your direct peers and stops there. The peer who received it
    /// may READ it; they may not pass it on — which is the whole difference between
    /// "my connections" and "the network".
    #[test]
    fn connections_reaches_direct_peers_and_stops() {
        assert!(may_relay(Visibility::Connections, 0), "author → peer");
        assert!(!may_relay(Visibility::Connections, 1), "peer → stranger is the ban");
    }

    /// `network` spends exactly the discovery budget and no more.
    #[test]
    fn network_spends_the_discovery_budget_and_stops() {
        assert!(may_relay(Visibility::Network, 0));
        assert!(may_relay(Visibility::Network, 1));
        assert!(!may_relay(Visibility::Network, crate::contact::MAX_DISCOVER_HOPS));
    }

    /// The envelope rule and the discovery-answer ceiling are the SAME number. If these
    /// ever diverge, an object could advertise further than its deltas may travel — or
    /// the reverse, which is worse: content moving beyond what was advertised.
    #[test]
    fn relay_agrees_with_the_discovery_ceiling() {
        for vis in [Visibility::Private, Visibility::Connections, Visibility::Network] {
            let furthest = (0..8).filter(|h| may_relay(vis, *h)).count() as u32;
            assert_eq!(furthest, vis.reach_hops(), "{vis:?}");
        }
    }
}
