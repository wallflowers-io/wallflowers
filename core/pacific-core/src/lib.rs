//! pacific-core — the on-device client engine for Pacific.
//!
//! Modules land in dependency order (see the M1 build spec): paths → identity →
//! directory → mls_store → mls → seal → coordinator → transport → handshake → node.
//!
//! Two features carve off what an operating system provides — `storage` (SQLite)
//! and `transport` (tokio sockets) — so the FOLD can be built for a target that
//! has neither. Both are on by default; a native build is byte-identical to
//! before the split. See `[features]` in Cargo.toml for why.
//!
//! No silent fallbacks: every failure is a typed [`CoreError`]; nothing here
//! stubs, defaults, or swallows. Honest-by-default lives one layer up (the CLI
//! returns milestone-tagged NotImplemented for verbs pacific-core does not yet back).

#![forbid(unsafe_code)]

pub mod atrest;
pub mod contact;
/// The ICD's numbers, read at build (build.rs): one include, for every module that needs one.
mod icd_consts {
    include!(concat!(env!("OUT_DIR"), "/icd_consts.rs"));
}
pub mod coordinator;
/// The demo expressed as real operations between users (the fitness function):
/// what the UI shows is reconstructable through the real pipe, never hardcoded.
#[cfg(all(feature = "mls", feature = "storage", feature = "transport"))]
pub mod demo;
#[cfg(all(feature = "mls", feature = "storage", feature = "transport"))]
pub mod demo_project;
#[cfg(feature = "storage")]
pub mod directory;
/// Event ticketing: the Event kind (29), the ticket Delta family, the delegate fold
/// invariants, and the signed door credential. The two pairwise ticket legs are
/// spliced into Contact's op table from here.
pub mod event;
/// A Site's own events and posts, as its Host's live items for the public Face (O-48).
pub mod face_items;
pub mod geo;
/// The constituent-object facet: an object that hosts a Forum (its comments,
/// its discussion, its room) points at a real GroupObject rather than growing a
/// comment op of its own.
pub mod host;
pub mod parts;
pub mod profiles;
pub mod about;
pub mod questions;
pub mod rsvp;
/// The standing facet: what one member of an object is to it. Any kind may
/// carry it, which is why a connection no longer needs a Group-typed twin.
pub mod roles;
pub mod group;
pub mod handshake;
pub mod history;
/// THE GroupObject interface over `delta_log`: one generic constructor + set operations.
pub mod feed_origin_icd;
/// The newitem modal's contract, checked against what the types declare.
pub mod newitem_icd;
pub mod icd;
pub mod identity;
/// The rust-direct (rust->rust) GroupObject route into the union LodeDB store:
/// `lodedb-core`'s `CoreAppender` appends reduced content at the delta_log.
#[cfg(feature = "lode")]
pub mod lode_route;
/// The seam a projector plugs into; `lode_route` is one (O-36).
#[cfg(feature = "storage")]
pub mod projection;
/// The held connection (O-69): one socket per Node, subscribed to its whole tag set.
#[cfg(feature = "transport")]
pub mod live;
#[cfg(feature = "transport")]
pub mod mailbox;
pub mod membership;
pub mod media;
pub mod mesh;
pub mod mesh_link;
pub mod mesh_policy;
/// The one mint seam — every `+` in the app authors through here.
pub mod mint;
/// Authoring: one pure constructor for every kind, with the log handed in,
/// so the browser and the phone write the same deltas and both probe before they do.
pub mod authoring;
/// What a reducer reads: every arg read goes through here, and a test may record it (NC-9).
pub mod arg_reads;
pub mod fold;
/// The fold cache (O-69): folds kept per directory, keyed so a hit equals a refold.
pub mod fold_cache;
/// Per-delta authorship signatures — who wrote this, provable by anyone, forever.
pub mod delta_sig;
/// The kiosk's claim token: one visitor's single-use admission to a Site (A-3, SEC-A1).
pub mod claim;
/// The wrap: the identity seed sealed under the passkey, and the one account
/// artefact served unauthenticated. No MLS, no storage — a seed and an AEAD.
pub mod wrap;
/// Locators: the seed-derived addresses a person's records live at, and the one
/// opaque key that writes them. `identity_pk` is the account's NAME; using the
/// name as the address makes the arc a census, and this is what replaces it.
pub mod locator;
/// The head: where the archive chain currently ends, sealed under the SEED (not
/// the PRF, so the words door reaches it) and anchored beside the wrap at the arc.
/// The one thing a chain cannot prove about itself — that its tail was not cut.
pub mod head;
/// A device's state sealed at rest under a key off the seed (D-34 (c)).
pub mod devstate;
#[cfg(feature = "mls")]
pub mod mls;
/// In-memory MLS storage — the platform pair for targets without SQLite (the browser).
#[cfg(feature = "mls")]
pub mod mls_mem;
/// The spine: the per-account chain of entries that lets a seed NAME its objects.
/// Pure format — every platform imports THIS rather than writing its own encoder.
pub mod spine;
pub mod lease;
pub mod resumption;
/// SQLite impls of the two `mls-rs` storage traits. `storage`, not `mls`: they
/// ARE the native platform's answer, and a platform without SQLite brings its own.
#[cfg(all(feature = "mls", feature = "storage"))]
pub mod mls_store;
#[cfg(all(feature = "mls", feature = "storage", feature = "transport"))]
pub mod node;
/// The object model contract (object = group of 1; the 10 typed Delta objects).
pub mod note;
pub mod backlink;
pub mod parent;
pub mod object;
pub(crate) mod object_args;
#[cfg(feature = "storage")]
pub mod object_store;
pub mod paths;
/// Somewhere in the world, as a shareable GroupObject — the first kind to carry the
/// common location facet (`geo`) as its defining state.
pub mod place;
/// A published thing with its own spine — two roles, Owner and Viewer.
pub mod post;
pub mod project;
pub mod recurrence;
#[cfg(feature = "transport")]
pub mod router;
pub mod seal;
/// An external source as a GroupObject (kind 21) — Resident Advisor, ArtRabbit — and
/// the door its items come through. Hydrated items arrive as DELTAS on the System's own
/// log, so the feed selects over one substrate instead of merging several.
pub mod system;
/// A concrete thing in your world + its market posture (Has/Wants/Offers/Buying/Selling) —
/// minted from a resolved entity once you accept it at the swipe deck.
pub mod treasury;
pub mod thing;
pub mod transaction;
#[cfg(feature = "transport")]
pub mod transport;
/// vCard (RFC 6350) import/export for the Group identity record — ONE parser
/// shared by the CLI and the app; importing a card mints/updates a Group.
pub mod vcard;
pub mod publication;
pub mod visibility;
pub mod wallet;

#[cfg(all(feature = "mls", feature = "storage", feature = "transport"))]
pub use node::Node;

use thiserror::Error;

/// One loud error type for the whole engine — never a silent fallback.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("identity: {0}")]
    Identity(String),
    #[error("no local identity — run `pacific id init` first")]
    NoIdentity,
    #[error("identity already exists at {0} — refusing to overwrite")]
    IdentityExists(String),
    #[error("directory: {0}")]
    Directory(String),
    #[error("mls: {0}")]
    Mls(String),
    /// A key package past its lifetime, by MLS's own check (W-96: a contact code gone stale).
    /// Said as MLS says it, and told apart by its kind, never by its text.
    #[error("mls: {0}")]
    KeyPackageExpired(String),
    #[error("seal: {0}")]
    Seal(String),
    #[error("at-rest: {0}")]
    AtRest(String),
    #[error("at-rest key unavailable — a sealed secret cannot be opened without the device key")]
    AtRestKeyUnavailable,
    #[error("transport: {0}")]
    Transport(String),
    #[error("no relay configured — run `pacific relay set <ws-url>`")]
    NoRelay,
    #[error("arc governance: {0}")]
    Governance(String),
    #[error("coordinator: {0}")]
    Coordinator(String),
    #[error("handshake: {0}")]
    Handshake(String),
    #[error("not connected to peer {0} (double-opt-in not complete)")]
    NotConnected(String),
    #[error("unknown peer {0}")]
    UnknownPeer(String),
    #[error("commit race lost: {0}")]
    CommitRace(String),
    /// A later lease cell took the pool leaf this device spoke through (resumption.md
    /// §6.2): it stops speaking in that object rather than burn generations it no
    /// longer owns.
    #[error("fenced: {0}")]
    Fenced(String),
    /// The group's protocol floor (`mls::MIN_VERSION_EXT`) is above this build. The
    /// group is PAUSED — never judged, never skipped — until the app is updated.
    #[error("update required: {0}")]
    UpgradeRequired(String),
    /// A membership door refused, with the reason in words (membership-through-mls.md
    /// §5, §6, §9): not the owner, the owner must hand over first, the only member
    /// left, already leaving, no longer a member, or a legacy group not yet migrated.
    #[error("membership: {0}")]
    Membership(String),
    /// A door refused to author a Delta that is too big to be carried to the other
    /// devices — see `node::MAX_DELTA_ENVELOPE_BYTES`. Its own variant because it is
    /// PERMANENT and LOCAL: nothing was written, retrying the same bytes will refuse
    /// again, and the only fix is a smaller payload. That is the opposite of a
    /// `Transport` failure, which means "not yet" and is answered by the outbox.
    #[error("too large: {0}")]
    TooLarge(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl CoreError {
    /// Retryable = the failure concerns THIS DEVICE (storage/io), not the blob —
    /// abort the drain WITHOUT advancing the cursor so nothing is skipped by a
    /// transient local fault. Everything else is permanent for the blob:
    /// quarantine + advance. (XMTP's cursor rule: "If the error is retryable we
    /// cannot move on to the next message otherwise you can get into a forked
    /// group state.") The Door's sealed state is kept on the same rule (NC-75).
    pub fn is_retryable(&self) -> bool {
        // UpgradeRequired too: a paused group must not advance its cursor past a commit
        // it cannot judge — it is re-read once the app is updated (mls::MIN_VERSION_EXT).
        matches!(self, CoreError::Directory(_) | CoreError::Io(_) | CoreError::UpgradeRequired(_))
    }
}

/// Bounded past-epoch retention — the one window shared by BOTH stores so they
/// can never disagree: mls_store prunes each group's epoch secrets, and the
/// directory prunes the matching epoch mailbox tags `sync` re-drains. Field
/// consensus is 3 (Wire MAX_PAST_EPOCHS, XMTP max_past_epochs, mls-rs's SQLite
/// provider default): late application messages from the last 3 epochs still
/// decrypt; anything older is dropped — an explicit forward-secrecy dial, per
/// RFC 9750 §7's mandatory out-of-order tolerances, not an accident of
/// implementation.
pub const EPOCH_RETENTION: u64 = 3;

#[cfg(feature = "storage")]
impl From<rusqlite::Error> for CoreError {
    fn from(e: rusqlite::Error) -> Self {
        CoreError::Directory(e.to_string())
    }
}

impl From<crate::object::DeltaRejection> for CoreError {
    fn from(r: crate::object::DeltaRejection) -> Self {
        CoreError::Coordinator(format!("delta rejected: {r:?}"))
    }
}

/// A LodeDB (`lodedb-core`) error surfaced by the rust-direct union-store route
/// (WS-G, coordination §4). Its stable code prefixes the loud message so a failed
/// projection never degrades silently.
#[cfg(feature = "lode")]
impl From<lodedb_core::CoreError> for CoreError {
    fn from(e: lodedb_core::CoreError) -> Self {
        CoreError::Coordinator(format!("lodedb {}: {}", e.code().as_str(), e.message()))
    }
}

/// A process-global lock for tests that mutate `PACIFIC_STATE_DIR` (env is global).
#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
