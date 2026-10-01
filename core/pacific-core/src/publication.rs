//! publication — WHETHER THIS OBJECT HAS A PUBLIC ADDRESS, AND WHO SERVES IT.
//!
//! A common base facet, spliced into a kind's `ops()` exactly as [`crate::geo`]
//! splices location and [`crate::visibility`] splices reach.
//!
//! # Why this is not a fourth `Visibility` level
//!
//! [`crate::visibility::Visibility`] grades how far the fact of an object's
//! existence may travel THROUGH THE MESH, and it grades it in HOPS: private 0,
//! connections 1, network `MAX_DISCOVER_HOPS`. Publication is not a further hop.
//! It is a different MEDIUM — an HTTP reader is not sitting at some distance in
//! the mesh, they are not in the mesh at all.
//!
//! That distinction is forced by the encoding as much as by the meaning:
//! `Visibility::reach_hops()` IS the wire form (`coordinator.rs` writes it as
//! `K_VIS`, unsigned, inside `canonical_bytes` and therefore inside the DeltaId),
//! and there is no value on an unsigned hop scale that means "not a hop count" —
//! 0 is already `private`. A fourth level would have to either claim infinite
//! mesh reach or collide with `private`, and adding a signed sentinel would
//! change every DeltaId on a hash-chained log.
//!
//! Keeping them orthogonal also buys the thing a site actually wants: it may be
//! `network` in the mesh AND published on the web, set independently, rather than
//! being forced to choose one point on a single dial.
//!
//! # Publishing is a one-way door, and this facet cannot close it
//!
//! `visibility` can promise that tightening heals outward: an answer replaces its
//! predecessor wholesale and the newest statement wins, so narrowing reach
//! genuinely narrows it. **That promise does not survive contact with the web.**
//! Once a projection has been served over HTTP it is crawled and cached by
//! parties this protocol has no relationship with. [`OP_UNPUBLISH`] stops the
//! publisher serving; it does not un-publish what was already read. Surfaces MUST
//! say so at the moment the dial is turned — an "unpublish" that implied
//! retraction would be the dishonest half of this design.
//!
//! # The publisher is a member, deliberately
//!
//! Serving a projection means holding the fold, and holding the fold means
//! receiving the deltas — so the publisher is an ordinary MLS member of this
//! object and nothing more exotic. Three properties fall out of that, and all
//! three are the reason to do it this way rather than by handing an Arc a key:
//!
//! - **Visible.** The publisher is IN the roster. There is no hidden observer of
//!   a public-facing object, which is exactly the object you least want one on.
//! - **Revocable.** Remove the member and the serving stops, and the removal is
//!   itself a delta (`base.memberLeft`) with an author and a timestamp.
//! - **Read-only by construction.** Nothing here grants authority. The publisher
//!   folds and serves; it authors no deltas, so `author_pk` stays clean and
//!   per-author LWW never sees an Arc racing a person.
//!
//! It is the shape `event.rs` already uses for the box-office delegate: a named
//! member trusted for one role, named ON the object so the fold can check it.

use serde::{Deserialize, Serialize};

use crate::coordinator::{ArgVal, Args};
use crate::object::{Authority, Commutativity, DeltaRejection, MemberId, Op, OpDecl};
use crate::object_args::{arg_hex32, req_text};

/// Bounds on a slug, chosen to be simultaneously a safe URL path segment and a
/// legal DNS label — so `sites.example.com/<slug>` and `<slug>.example.com` are
/// both open to us later without a migration.
pub const SLUG_MIN: usize = 2;
pub const SLUG_MAX: usize = 63;

/// The public address of an object, and the member that serves it.
///
/// Default is UNPUBLISHED, and an object from a build that predates this facet
/// deserializes to exactly that — absence of an address is the safe reading, the
/// same discipline `Visibility` uses in defaulting to `private`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Publication {
    /// The public path segment this object answers on; `""` when unpublished.
    /// Validated at fold by [`valid_slug`] — never trusted from the wire.
    pub slug: String,
    /// The member serving the public projection. `None` when unpublished.
    ///
    /// Held as the identity pubkey rather than a URL: WHO serves is the object's
    /// assertion and belongs in the log; WHERE they serve is deployment detail
    /// that changes without the object's consent (`groups.arc_url` already
    /// carries that, and is swappable by supermajority).
    pub publisher: Option<MemberId>,
}

impl Publication {
    /// True when this object has a public address being served.
    ///
    /// Both halves must hold: a slug with no publisher is an address nobody
    /// answers, and a publisher with no slug has nothing to answer on. The fold
    /// never produces either, and this is the guard that says so out loud.
    pub fn is_published(&self) -> bool {
        !self.slug.is_empty() && self.publisher.is_some()
    }
}

/// Is `s` a legal slug?
///
/// Lowercase alphanumerics and single interior hyphens, [`SLUG_MIN`]..=[`SLUG_MAX`].
/// Deliberately narrow: this string lands in a URL, and a permissive rule here
/// would push escaping onto every surface that ever renders it. Uppercase is
/// rejected rather than folded, because two slugs differing only in case would
/// resolve to one address and the log would show two owners with a claim to it.
pub fn valid_slug(s: &str) -> bool {
    let n = s.len();
    if n < SLUG_MIN || n > SLUG_MAX {
        return false;
    }
    if !s.is_ascii() {
        return false;
    }
    let b = s.as_bytes();
    if b[0] == b'-' || b[n - 1] == b'-' {
        return false;
    }
    let mut prev_hyphen = false;
    for &c in b {
        let ok = c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-';
        if !ok {
            return false;
        }
        if c == b'-' && prev_hyphen {
            return false; // no `--`, which reads as a typo and confuses punycode
        }
        prev_hyphen = c == b'-';
    }
    true
}

// ---- the base publication op-group (common to every GroupObject) -------------

/// Reserved base-op band, beside geo's `0xF000_xxxx`, membership's `0xF001_xxxx`
/// and visibility's `0xF002_xxxx`.
pub const OP_PUBLISH: u32 = 0xF003_0000;
pub const OP_UNPUBLISH: u32 = 0xF003_0001;

/// The base publication ops, to be included in a type's `ops()`.
///
/// Owner/sequenced, for the same reason visibility is: giving an object a public
/// face is the owner's assertion about their own object. Sequenced because two
/// concurrent `publish` deltas naming different slugs must resolve to one answer
/// — a commutative merge would leave the object claiming two addresses.
pub static PUBLICATION_OPS: &[OpDecl] = &[
    OpDecl {
        op_id: OP_PUBLISH,
        name: "base.publish",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_UNPUBLISH,
        name: "base.unpublish",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
];

/// True if `op_id` is a base publication op — so an object can route it to
/// [`reduce_publication`] before its own op match.
pub fn is_publication_op(op_id: u32) -> bool {
    matches!(op_id, OP_PUBLISH | OP_UNPUBLISH)
}

/// Fold a base publication op into an object's single-slot facet.
///
/// Both args are validated BEFORE either is assigned: a reduce-time rejection is
/// skipped at fold, so a half-applied publish would leave the object addressable
/// with no publisher — live, and unserveable — as its permanent state.
pub fn reduce_publication(
    slot: &mut Publication,
    op: &Op<'_>,
) -> Result<(), DeltaRejection> {
    match op.op_id {
        OP_PUBLISH => {
            let slug = req_text(op.args, "slug")?;
            if !valid_slug(slug) {
                return Err(DeltaRejection::MalformedArgs);
            }
            let publisher = arg_hex32(op.args, "publisher")?;
            // The publisher must be IN the object. A non-member could never
            // receive the deltas, so it could never fold the state it claims to
            // serve — accepting one would record an address that cannot answer.
            if !op.ctx.is_member(&publisher) {
                return Err(DeltaRejection::MalformedArgs);
            }
            slot.slug = slug.to_string();
            slot.publisher = Some(publisher);
            Ok(())
        }
        OP_UNPUBLISH => {
            // Clear BOTH halves: leaving the slug behind would keep the object
            // claiming an address it no longer serves, and a later publisher
            // would silently inherit a name the owner may not have re-chosen.
            slot.slug.clear();
            slot.publisher = None;
            Ok(())
        }
        _ => Err(DeltaRejection::UnknownType),
    }
}

/// The two args a `base.publish` delta carries.
pub fn publish_args(slug: &str, publisher: &MemberId) -> Args {
    let mut a = Args::new();
    a.insert("slug".into(), ArgVal::Text(slug.to_string()));
    a.insert("publisher".into(), ArgVal::Text(hex::encode(publisher)));
    a
}

/// `base.unpublish` carries no args — the absence IS the instruction.
pub fn unpublish_args() -> Args {
    Args::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::ReduceContext;

    const OWNER: [u8; 32] = [7u8; 32];
    const ARC: [u8; 32] = [9u8; 32];
    const STRANGER: [u8; 32] = [11u8; 32];

    fn apply(slot: &mut Publication, op_id: u32, args: &Args) -> Result<(), DeltaRejection> {
        let members = [OWNER, ARC];
        let ctx = ReduceContext {
            members: &members,
            owner: OWNER,
            epoch: 0,
        };
        let op = Op {
            op_id,
            args,
            author: &OWNER,
            pos: None,
            ctx: &ctx,
        };
        reduce_publication(slot, &op)
    }

    #[test]
    fn default_is_unpublished() {
        let p = Publication::default();
        assert!(p.slug.is_empty());
        assert_eq!(p.publisher, None);
        assert!(!p.is_published());
    }

    #[test]
    fn publish_then_unpublish_round_trips() {
        let mut p = Publication::default();
        apply(&mut p, OP_PUBLISH, &publish_args("cambridge-dd", &ARC)).unwrap();
        assert!(p.is_published());
        assert_eq!(p.slug, "cambridge-dd");
        assert_eq!(p.publisher, Some(ARC));

        apply(&mut p, OP_UNPUBLISH, &unpublish_args()).unwrap();
        assert!(!p.is_published());
        assert!(p.slug.is_empty(), "unpublish clears the address too");
        assert_eq!(p.publisher, None);
    }

    #[test]
    fn a_publisher_must_be_a_member() {
        let mut p = Publication::default();
        assert_eq!(
            apply(&mut p, OP_PUBLISH, &publish_args("ok-slug", &STRANGER)),
            Err(DeltaRejection::MalformedArgs)
        );
        assert!(!p.is_published(), "a rejected publish leaves no trace");
    }

    #[test]
    fn a_rejected_publish_never_half_applies() {
        let mut p = Publication::default();
        // Valid slug, non-member publisher: the slug must NOT survive the reject.
        let _ = apply(&mut p, OP_PUBLISH, &publish_args("good-name", &STRANGER));
        assert!(p.slug.is_empty());
        assert_eq!(p.publisher, None);
    }

    #[test]
    fn slug_rules() {
        assert!(valid_slug("ab"));
        assert!(valid_slug("cambridge-dd"));
        assert!(valid_slug("a1-b2-c3"));
        assert!(valid_slug(&"a".repeat(SLUG_MAX)));

        assert!(!valid_slug(""), "empty");
        assert!(!valid_slug("a"), "under SLUG_MIN");
        assert!(!valid_slug(&"a".repeat(SLUG_MAX + 1)), "over SLUG_MAX");
        assert!(!valid_slug("-lead"), "leading hyphen");
        assert!(!valid_slug("trail-"), "trailing hyphen");
        assert!(!valid_slug("a--b"), "double hyphen");
        assert!(!valid_slug("Cambridge"), "uppercase would alias a second owner");
        assert!(!valid_slug("with space"));
        assert!(!valid_slug("with/slash"), "would escape the path segment");
        assert!(!valid_slug("with.dot"), "would split a DNS label");
        assert!(!valid_slug("café"), "non-ascii");
    }

    #[test]
    fn a_bad_slug_is_rejected_at_fold_not_sanitised() {
        let mut p = Publication::default();
        assert_eq!(
            apply(&mut p, OP_PUBLISH, &publish_args("Bad Slug", &ARC)),
            Err(DeltaRejection::MalformedArgs)
        );
        assert!(!p.is_published());
    }

    #[test]
    fn publishing_again_replaces_the_address_wholesale() {
        let mut p = Publication::default();
        apply(&mut p, OP_PUBLISH, &publish_args("first-name", &ARC)).unwrap();
        apply(&mut p, OP_PUBLISH, &publish_args("second-name", &OWNER)).unwrap();
        assert_eq!(p.slug, "second-name");
        assert_eq!(p.publisher, Some(OWNER), "the newest statement wins");
    }

    #[test]
    fn is_published_needs_both_halves() {
        let half = Publication {
            slug: "orphan".into(),
            publisher: None,
        };
        assert!(!half.is_published(), "an address nobody answers is not published");
        let other = Publication {
            slug: String::new(),
            publisher: Some(ARC),
        };
        assert!(!other.is_published(), "a publisher with no address is not published");
    }

    #[test]
    fn unknown_op_in_band_is_refused() {
        let mut p = Publication::default();
        assert_eq!(
            apply(&mut p, 0xF003_00FF, &unpublish_args()),
            Err(DeltaRejection::UnknownType)
        );
    }
}
