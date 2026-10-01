//! The node facade — the only surface the CLI touches. It owns identity +
//! directory, rebuilds the MLS client per call from the persisted signing key +
//! SQLite group state, and exposes the four M1 operations.

use rand_core::{OsRng, RngCore};
use zeroize::Zeroizing;

use crate::directory::{Directory, PeerStatus};
use pacific_wire::address::Address;
use crate::group::{self, GroupShape, GroupState, GroupType, GroupView, Presence};
use crate::identity::{self, Identity};
use crate::object::{Authority, Commutativity, ObjectKind, ObjectType};
use crate::project::{ProjectState, ProjectType};
use crate::vcard;
use crate::{coordinator, handshake, mls, paths, router::Router, seal, CoreError};

/// One row of a (forum) object's folded RATIFY projection — a proposal with its
/// deferred payload, passing rule, DERIVED outcome, ballot tallies, and this device's
/// own ballot. The read model the ratify UI binds to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RatifyView {
    pub proposer: [u8; 32],
    pub gen: u64,
    pub payload: String,
    pub rule: String,    // "consent" | "owner"
    pub outcome: String, // "pending" | "passed" | "failed"
    pub approve: u32,
    pub reject: u32,
    pub abstain: u32,
    pub mine_ballot: String, // "" | "approve" | "reject" | "abstain"
}

/// How many fresh key packages each side stocks into a Contact channel at pairing,
/// so the peer can add them to future Conversations without a live code exchange.
/// A small buffer for now (real target ~20; see the prekey design). Each retained
/// offer keeps a private half in `mls_key_package` until consumed/expired.
const PREKEY_STOCK: usize = 8;

// ---- the carriage ceiling -----------------------------------------------------------
//
// THE DEFECT, MEASURED ON TWO DEVICES OVER A REAL RELAY (19 Sep 2026). A `group.setCover`
// carrying 180,000 base64 characters built a 180,091-byte envelope; mls-rs padded and
// framed that to a 196,704-byte ciphertext (+9%); `seal` added its 24-byte nonce and
// 16-byte tag (196,744); `pacific_wire::blob_b64` turned those into 262,328 characters
// (+33%) — 184 over the relay's 262,144-byte publish cap. The same cover at 150,000
// characters arrived. So the cliff between "syncs" and "never syncs" sat INSIDE one op's
// own fold cap (`group::MAX_COVER_STILL_B64` is 700,000), and the author was told
// nothing: the delta was written locally, refused by the relay, and left in the outbox
// for ever. The peer kept its stale cover; then the next SEQUENCED op that did fit named
// a predecessor that peer had never received, and the object stopped folding there
// altogether — `ChainBroken`. One oversize cover plus one profile edit killed the group
// on every other device.
//
// WHY THE FOLD'S CAPS ARE NOT THE FIX. A fold cap is a wire value: lowering one would
// make this build refuse deltas older builds legitimately produced, which is a worse
// failure than the one being fixed. The fold stays exactly as it is. Only the DOOR gets
// stricter, and only on what THIS device authors.
//
// CORE CANNOT READ THE RELAY'S CAP. `max_blob_bytes` is the relay's own configuration
// (`RELAY_MAX_BLOB_BYTES`, `arc/planes/relay/src/limits.rs`), it is read at the relay's
// boot, and an operator may raise it. Core therefore refuses at the CONSERVATIVE
// DEFAULT — and an operator who raises the relay's cap does not thereby raise this one.
// Core just stays stricter than it has to be, which is the direction that strands
// nobody. `the_ceiling_is_derived_from_the_relays_own_default`, in
// `tests/carriage_ceiling_refuses_at_the_door.rs`, asks the relay's own crate for its
// default and fails if it ever moves away from the number below.

/// The relay's DEFAULT `max_blob_bytes`: the largest single publish it will accept,
/// counted in bytes of base64 as sent. Mirrors `relay::limits::Limits::default()`, and
/// pinned to it by `the_ceiling_is_derived_from_the_relays_own_default`.
pub const RELAY_DEFAULT_MAX_BLOB_B64: usize = 262_144;

/// The largest Delta envelope (canonical CBOR — before MLS, before the seal, before
/// base64) this device will AUTHOR. Derived, never transcribed:
///
/// ```text
///   262,144 b64 chars    the relay's conservative default  (RELAY_DEFAULT_MAX_BLOB_B64)
///   ÷ 4 × 3 = 196,608    the sealed bytes that base64s to exactly that
///   × 4 ÷ 5 = 157,286    less the worst case MLS can add to the plaintext
/// ```
///
/// WHY 4/5 AND NOT 9/10. The measured +9% is not a rate, it is a STEP: mls-rs defaults
/// to `PaddingMode::StepFunction`, which rounds the plaintext up to a multiple of an
/// eighth of the next power of two — everything in `[131072, 163840)` pads to 163,840 and
/// everything in `[163840, 196608)` pads to 196,608, which is precisely why 150,000 arrived
/// and 180,000 did not. The worst case of that function is +25% (a plaintext landing on
/// 2^k/2), so the ceiling allows for +25% rather than for the one figure a single
/// measurement happened to show. At the ceiling the real chain is 157,286 → 163,840
/// padded → +96 framing → +40 seal → 218,636 base64: 43,508 characters inside the cap.
///
/// It is BELOW several ops' fold caps on purpose (cover 700,000; group icon via
/// `group.setProfile` 500,000; event banner 700,000, photo 500,000, clip 1,500,000).
/// Those remain what the fold accepts from a peer. This is what this device will send.
/// Media that large needs a detached path — it does not need a delta that cannot move.
pub const MAX_DELTA_ENVELOPE_BYTES: usize = RELAY_DEFAULT_MAX_BLOB_B64 / 4 * 3 * 4 / 5;

/// A callee's future, built in a frame of its own and boxed. In a debug build a future is built in
/// its caller's poll frame before it moves, so an async fn holds every future it awaits at once:
/// `sync_once`'s frame was 378 KiB of a test thread's 2 MiB, above every path that syncs (29 Sep).
/// Boxing at the await leaves that frame as it was; building here does not (O-77's
/// `reconcile_profiles_boxed`, stated once). `Pin<Box<F>>`, not `dyn`: `Send` stays the callee's.
#[inline(never)]
fn boxed<F: std::future::Future>(make: impl FnOnce() -> F) -> std::pin::Pin<Box<F>> {
    Box::pin(make())
}

/// The door. `Ok(())` means this envelope can be carried to the other devices; the
/// `Err` is PERMANENT — the same bytes will be refused again — and says so in words a
/// person can act on, naming the op, the size, the ceiling and the overshoot.
///
/// Called from [`Node::append_and_flush`] BEFORE anything is written, which is the
/// property the whole change turns on: a refused delta leaves no local state, no outbox
/// entry, and therefore no gap in the sequenced spine for a peer to break its chain on.
fn refuse_if_uncarriable(delta: &coordinator::Delta, envelope_len: usize) -> Result<(), CoreError> {
    if envelope_len <= MAX_DELTA_ENVELOPE_BYTES {
        return Ok(());
    }
    let over = envelope_len - MAX_DELTA_ENVELOPE_BYTES;
    // How much smaller the payload has to get, rounded UP so the advice is never
    // optimistic — the envelope is the payload plus a hundred-odd bytes of framing.
    let shrink_pct = (over * 100).div_ceil(envelope_len);
    Err(CoreError::TooLarge(format!(
        "this {} delta is {envelope_len} bytes — {over} over the {MAX_DELTA_ENVELOPE_BYTES}-byte \
         ceiling one publish can carry (a relay refuses anything over \
         {RELAY_DEFAULT_MAX_BLOB_B64} bytes once MLS and the seal have grown it). NOTHING WAS \
         WRITTEN: make the payload about {shrink_pct}% smaller and author it again. Storing it \
         would not have helped: it could never have reached anyone else, and the next op on this \
         object would then have broken the chain on every other device.",
        crate::object::op_label(delta.type_id, delta.op_id)
    )))
}

/// What [`Node::reconcile_profiles`] did: the objects it published my card into, and why the
/// card went without its picture, if it did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileSync {
    pub published: usize,
    pub icon_left_out: Option<String>,
}

pub struct Node {
    pub id: Identity,
    pub dir: Directory,
    /// OPT-IN rust-direct projection sink (WS-G, coordination §4): when installed,
    /// a locally-authored GroupObject content delta's reduced text is projected
    /// into the union LodeDB store at the append path. `None` by default — nothing
    /// is projected and the existing write path is byte-for-byte unchanged (every
    /// existing test constructs a node with no projector). Install via
    /// [`Node::install_projection`].
    projection: Option<Box<dyn crate::projection::ContentProjector>>,
    /// THIS DEVICE'S TRANSPORT SET — the one place that decides where a message goes. It used to
    /// be a `relay_url` argument on every public method, which meant the caller re-decided per
    /// call and nothing could ever add a second transport. Now it is state, resolved per object
    /// by `routes_for`.
    routes: crate::router::Routes,
    /// THE HELD SESSION (K-33): the relay session `sync_once` keeps between passes
    /// instead of dialling afresh each time. See [`Node::home_session`].
    held: std::sync::Mutex<Option<Router>>,
    /// How many of this device's own messages a drain has handed back and MLS has
    /// skipped (NC-33). A device's own message comes back whenever its tag is
    /// drained; one it does not also hold in its log is one it has lost.
    skipped_own: std::sync::atomic::AtomicU64,
    /// Messages from this device's own leaf that this state never published (the sent
    /// ledger, D-34 (c)): another copy of this device has spoken. See [`Node::drain_only`].
    own_unsent: std::sync::atomic::AtomicU64,
    /// My card and the objects I held when [`Node::reconcile_profiles`] last found it in every
    /// one of them (O-77): while neither moves, there is nothing to walk, so a write's tail and
    /// the sync tick pay one fold of the self record and one query, not a fold per group.
    card_everywhere: std::sync::Mutex<Option<u64>>,
    /// What `join_what_the_record_names` last tried, and when: an object named but not
    /// yet joinable is tried again after [`JOIN_RETRY`], not on every sync.
    joined_tried: std::sync::Mutex<Option<(std::collections::BTreeSet<String>, std::time::Instant)>>,
    /// Each object this device owns, and its roster when its standing state was last
    /// restated in full (NC-65): a member not in it is a joiner owed the restatement.
    restated: std::sync::Mutex<std::collections::HashMap<String, std::collections::BTreeSet<[u8; 32]>>>,
    /// THE HELD CONNECTION (O-69): read always, subscribed to every tag this Node listens on.
    /// See [`Node::go_live`].
    live: std::sync::Mutex<Option<crate::live::Live>>,
    /// What the held connection cannot vouch for yet, and what waits on it. See
    /// [`Node::ingest_delivered`].
    live_state: std::sync::Mutex<LiveState>,
}

/// How long a drain waits for a held connection dialled beside it to be confirmed.
const LIVE_CONFIRM: std::time::Duration = std::time::Duration::from_secs(3);

/// What frames waiting for a drain may hold before they are dropped and everything drained.
const WAITING_BYTES: usize = 8 << 20;

/// The held connection's bookkeeping on this Node's side (O-69).
#[derive(Default)]
struct LiveState {
    /// Each tag drained after its subscription was confirmed, and that subscription's `since`:
    /// complete from then on.
    drained: std::collections::HashMap<String, u64>,
    /// The queue overflowed: no tag is complete until drained again.
    dropped: bool,
    /// Frames on a tag not yet complete, held until a drain has brought what precedes them.
    waiting: Vec<crate::live::Delivered>,
    /// Objects a commit moved: drained, epoch by epoch, before more is ingested for them.
    follow: std::collections::BTreeSet<Vec<u8>>,
    /// A Welcome was joined from a delivered frame: the full pass follows, as after any join.
    joined: bool,
    /// Tags drained from the relay, each a round trip (FC-15).
    tag_drains: u64,
}

/// What [`Node::ingest_delivered`] and [`Node::catch_up`] did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Ingested {
    /// Frames turned into rows (or quarantined, as a drain would).
    pub frames: usize,
    /// Commits among them.
    pub commits: usize,
    /// Welcomes joined.
    pub joined: usize,
    /// Frames held for a drain.
    pub waiting: usize,
    /// Objects drained over the held session.
    pub drained: usize,
}

/// How long a set of named-but-unheld objects waits before it is tried again.
const JOIN_RETRY: std::time::Duration = std::time::Duration::from_secs(30);

/// What a NON-MEMBER may see of a published object.
///
/// Everything absent here is absent on purpose. In particular the ROSTER is not
/// carried: a member list is the single most likely thing to leak by accident
/// from a public-facing object, and no surface can leak a field it was never
/// handed. Credentials, affiliations, offices, the membership log and the
/// contact card are withheld for the same reason — a site's public face is its
/// name and its look, not its people.
///
/// Widening this struct is a deliberate act. Anything added here becomes readable
/// by the open internet the moment it ships, and `base.unpublish` cannot take it
/// back once crawled.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PublishedSite {
    /// The object id, so an embed can deep-link back into the app.
    pub object_id: String,
    pub slug: String,
    pub name: String,
    /// The group's shape, rendered as its wire string.
    pub shape: String,
    /// Base64 cover image; "" when none.
    pub cover: String,
    pub cover_mime: String,
}

/// An object this node holds whose log will not fold — the value that replaces a
/// silent `continue`.
///
/// Carried out of [`Node::noncompliant_objects`] rather than logged, because a
/// line in a log on one device is not a fact the system can act on: the harness
/// asserts on this, the app can surface it, and the ICD can name it. A `NSLog`
/// cannot be any of those.
/// IS MLS STATE REACHABLE FROM THE SEED? No. `build_client_sqlite` persists into
/// `pacific.db` under the device key, which is right for this device and
/// unreachable from a seed on another one. mls-rs rebuilds a live group from
/// `GroupStateStorage`; what is missing is an implementation of that trait over
/// storage a seed can reach. Same rule as above: flip it when it is true.
const MLS_STATE_IS_SEED_REACHABLE: bool = false;

/// THE GENERATION THIS DEVICE WRITES ITS SPINE AT.
///
/// One constant rather than a `0` at four sites. The queue, the drain, the
/// pending view and `recoverability` must all name the same generation or the
/// drain publishes what the check does not read, and four literals that must
/// agree are four chances to disagree silently. A restart that needs a fresh
/// chain bumps this — the head names the live one — and bumping it has to move
/// every site at once, which is what a constant buys and a literal does not.
const SPINE_GEN: u32 = 0;

/// Whether one object could be got back from the seed alone, and what is missing.
///
/// Three independent facts, not a score. A `false` in any one of them is a
/// different loss: unnamed means recovery never finds it, unreadable means its
/// content is gone for those epochs, unspeakable means it comes back read-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recoverability {
    /// The object, named the way every other door in this file names it.
    pub object_id: String,
    /// Is there a spine entry, so a recovering device can NAME this object?
    pub named: bool,
    /// Is the MLS state somewhere the seed reaches, so it comes back LIVE?
    pub speakable: bool,
    /// Epochs with no archive key record — their content cannot be read again.
    /// Listed rather than counted: a hole in the middle and a missing tail are
    /// different failures and a number hides which one this is.
    pub unreadable_epochs: Vec<u64>,
    /// The epoch this device believes the object is at.
    pub current_epoch: u64,
    /// The account's floor here, from the spine entries this device holds: the
    /// epoch it joined at. Epochs below it were never this account's, so they are
    /// not in `unreadable_epochs` — "never yours" and "lost" are different facts.
    /// `None` when this device holds no entry for the object (see `recoverability`).
    pub first_epoch: Option<u64>,
    /// The last epoch the account was a member, if the spine records it leaving.
    pub left_epoch: Option<u64>,
}

impl Recoverability {
    /// All three, or it is not recoverable. Deliberately not a partial credit:
    /// an object that can be named and not read is a name pointing at a hole.
    pub fn recoverable(&self) -> bool {
        self.named && self.speakable && self.unreadable_epochs.is_empty()
    }

    /// Could not be checked — reported rather than assumed either way.
    pub fn unknown(object_id: &str, _why: String) -> Self {
        Recoverability {
            object_id: object_id.to_string(),
            named: false,
            speakable: false,
            unreadable_epochs: Vec::new(),
            current_epoch: 0,
            first_epoch: None,
            left_epoch: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Noncompliance {
    /// The object, named the way every other door in this file names it.
    pub object_id: String,
    /// Its declared kind — which is what chose the lens that refused.
    pub kind: String,
    /// The reducer's own words, not a summary. Whether this is an old build, a
    /// wrong op id, or a genuinely corrupt log is only legible in the original.
    pub reason: String,
}

///
/// Everything here was put on the Host by the group's owner on purpose: the Arc is a
/// member of the Host and never of the group (the-seam.html §03), so nothing in this
/// struct can come from the group's own log. Pictures are listed by slot; their bytes
/// come from [`Node::host_media`], so a directory of sites does not carry every image.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HostSite {
    /// The Host's object id (kind 33).
    pub object_id: String,
    pub slug: String,
    /// The Host's `host.define` name — the site's name as its owner published it.
    pub name: String,
    /// The Host's owner, hex. The Arc arbitrates a slug between Hosts by it.
    pub owner: String,
    /// The published bundle — `host.hydrate` key `face` — as JSON text; "" when the
    /// owner has published an address and no face yet.
    pub bundle: String,
    /// Live items (`event:` / `post:` keys), withdrawn ones left out.
    pub items: Vec<HostItem>,
    /// `(slot, mime)` for each picture the Host carries.
    pub media: Vec<(String, String)>,
}

/// The `host.hydrate` key the face bundle lives under. One well-known key, because
/// the Arc has to find it without being told: it folds the Host and reads this.
pub const HOST_FACE_KEY: &str = "face";

/// The role a room plays in the Group or Channel it is a part of.
pub const ROOM: &str = "room";

/// The role a Host plays in its Site: the parts facet's, where a build without
/// storage finds it too (NC-79).
pub use crate::parts::HOST_ROLE;

/// Who a kiosk claim admitted, and what the claim carries for the webapp at landing.
#[derive(Debug, Clone)]
pub struct Admitted {
    pub member: [u8; 32],
    pub choice: Option<String>,
    pub artifact: Option<String>,
    /// The Site's rooms the member is in now (D-58).
    pub rooms: Vec<String>,
    /// A room this node admits to that the member is not in yet, and why: named, never
    /// skipped, and completed by presenting the same claim again.
    pub unjoined: Vec<(String, String)>,
    /// The claim's choice when no room of this node's carries it, and the member went to
    /// its rooms without a choice instead (ICD 2.1.0 row 3): a Site set up short.
    pub fallback: Option<String>,
    /// What history went to the member (O-75): one per object this admission added them to,
    /// the Site and its rooms, named where it could not go.
    pub history: Vec<HistorySent>,
}

/// What an admitter sent a joiner of one object (O-75): the rows of each tier, the rows held
/// and not sent (past the cap, or unsigned), and, when nothing could go, why.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistorySent {
    pub object: String,
    pub spine: usize,
    pub records: usize,
    pub posts: usize,
    pub left: usize,
    pub unsent: Option<String>,
    /// The members' cards (O-75 cards, O-77): how many went, in how many bundles after the
    /// spine's, and how many did not (past `CARDS_MAX`, or one alone over a blob).
    pub cards: usize,
    pub card_bundles: usize,
    pub cards_unsent: usize,
    /// The spine's bundle sealed, in base64 characters, as the relay measures a blob against
    /// `HISTORY_MAX_B64`; 0 when none goes.
    pub sealed: usize,
}

/// What a history send publishes (`Node::history_planned`): the spine's bundle, if any rows go;
/// the card bundles; the report of both.
type HistoryPlan = (Option<crate::history::Bundle>, Vec<crate::history::Bundle>, HistorySent);

/// What one admission carries of an object's members' cards, at most, over all its card
/// bundles (O-75 cards): the ICD's `cardsTotal` (`history::CARDS_TOTAL`), one number, read at
/// build.
pub const CARDS_MAX: usize = crate::history::CARDS_TOTAL;

/// The cards' total a send uses: `CARDS_MAX`, or lower where PACIFIC_HISTORY_TOTAL_CAP lowers
/// it for a test. It may only lower it: a value above the ICD's, or not a number, is refused,
/// by the send and by arc-node at start.
pub fn cards_max() -> Result<usize, CoreError> {
    cards_max_from(std::env::var("PACIFIC_HISTORY_TOTAL_CAP").ok().as_deref())
}

/// `cards_max`'s rule, on the override as given.
pub fn cards_max_from(over: Option<&str>) -> Result<usize, CoreError> {
    match over {
        None => Ok(CARDS_MAX),
        Some(v) => match v.trim().parse::<usize>() {
            Ok(n) if n <= CARDS_MAX => Ok(n),
            _ => Err(CoreError::Coordinator(format!(
                "PACIFIC_HISTORY_TOTAL_CAP={v} is refused: it may only lower the ICD's cardsTotal, {CARDS_MAX}"
            ))),
        },
    }
}

/// The one relay blob a history travels in (O-75): the relay's own cap on a blob, measured
/// on the sealed blob as the relay sees it, since a larger one is refused `too_large`. It
/// holds a bundle of about 196 KB encoded, inside `history::CAP`.
pub const HISTORY_MAX_B64: usize = RELAY_DEFAULT_MAX_BLOB_B64;

/// One live item on a Host's face.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HostItem {
    pub key: String,
    pub payload: String,
    pub fetched_at: i64,
}

/// Every Host this node serves, and every Host it holds that it will NOT serve, each
/// with why — the value that replaces a silent `continue` (the doctrine, "Compliant or
/// non-compliant").
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]

pub struct HostScan {
    pub sites: Vec<HostSite>,
    /// `(object_id, reason)`.
    pub refused: Vec<(String, String)>,
}

/// One pass of a Site's items onto its Host (O-48): what was put, what was withdrawn,
/// and what the Site names that stayed off its Face, each with why.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HostSync {
    pub host: String,
    pub put: Vec<String>,
    pub withdrawn: Vec<String>,
    /// `(object_id, reason)`.
    pub left: Vec<(String, String)>,
}


impl Node {
    /// Open existing state (requires `id init` already ran).
    pub fn open() -> Result<Self, CoreError> {
        Ok(Self {
            id: identity::load()?,
            dir: Directory::open()?,
            projection: None,
            routes: crate::router::Routes::load(),
            held: Default::default(),
            skipped_own: Default::default(),
            own_unsent: Default::default(),
            card_everywhere: Default::default(),
            joined_tried: Default::default(),
            restated: Default::default(),
            live: Default::default(),
            live_state: Default::default(),
        })
    }

    /// Installs the OPT-IN union-store projection sink (WS-G). Once installed, a
    /// locally-authored `forum.post` into a GroupObject also projects its reduced
    /// text into LodeDB via the rust-direct route (fail-loud; a projection error
    /// propagates). Left uninstalled, the write path is unchanged.
    pub fn install_projection(&mut self, projector: Box<dyn crate::projection::ContentProjector>) {
        self.projection = Some(projector);
    }

    // ---- `pacific id init` ----
    pub fn init_identity(display_name: &str) -> Result<Self, CoreError> {
        let node = Self::open_new_identity(identity::init()?, display_name)?;
        // THE SELF RECORD, minted with the identity — the first object this
        // account creates, so it takes the first vertebra on the spine.
        //
        // A RESTORE DOES NOT DO THIS. The account already has a record, named at
        // index 0 of the spine it is recovering; minting a second one here would
        // give one account two records and lose the first quietly. Until the
        // archive key records and seed-reachable MLS state land, a restored device
        // cannot open the original — so it has none, and says so, rather than
        // fabricating a replacement. It acquires one on its first profile write.
        node.ensure_self_object()?;
        Ok(node)
    }

    /// ---- `pacific id restore` ---- The RESTORE door: bring an identity back
    /// from its recovery key on a device that has none.
    ///
    /// The identity comes back whole — same key, same id, so every peer's pin
    /// still matches. Nothing else does: MLS group state and history live in
    /// `pacific.db` and cannot be re-derived from a key, so this device rebuilds
    /// its directory from scratch (a fresh MLS leaf key, a fresh intro tag) and
    /// re-enters its groups by being re-admitted. Same loud refusal as the mint
    /// when an identity already exists here.
    pub fn restore_identity(display_name: &str, recovery_key: &str) -> Result<Self, CoreError> {
        Self::open_new_identity(identity::init_from_recovery_key(recovery_key)?, display_name)
    }

    /// Everything a just-created identity needs before it is a usable node: the
    /// directory, the MLS store, this install's own leaf signing key, and the
    /// `me` row. Shared by the mint and the restore doors — the device-local half
    /// is built the same way either way, because a restored device IS a new
    /// device as far as MLS is concerned.
    fn open_new_identity(id: identity::Identity, display_name: &str) -> Result<Self, CoreError> {
        let dir = Directory::open()?;
        crate::mls_store::migrate(&paths::db_path()).map_err(|e| CoreError::Mls(e.to_string()))?;

        // generate + persist the MLS leaf signing keypair (cross-process identity).
        let crypto = mls::crypto();
        let (sk, pk) = mls::generate_signing_key(&crypto)?;

        let intro = rand_tag();
        dir.put_me(
            &id.identity_pk(),
            &id.next_key_commit(),
            &intro,
            display_name,
            sk.as_bytes(),
            pk.as_bytes(),
        )?;
        let node = Self {
            id,
            dir,
            projection: None,
            routes: crate::router::Routes::load(),
            held: Default::default(),
            skipped_own: Default::default(),
            own_unsent: Default::default(),
            card_everywhere: Default::default(),
            joined_tried: Default::default(),
            restated: Default::default(),
            live: Default::default(),
            live_state: Default::default(),
        };
        Ok(node)
    }

    /// This account's own GroupObject: a group of one, owned by this identity,
    /// carrying the profile and — through `group.joinedObject` — the objects it
    /// belongs to. Created with the identity; created on demand for an account
    /// that predates it, so an older device acquires one on its next write.
    pub fn ensure_self_object(&self) -> Result<String, CoreError> {
        if let Some(gid) = self.dir.my_object()? {
            // It must still be here. A directory rebuilt under the same words has
            // the column and not the group, and a dangling id is worse than none.
            if self.dir.group_kind(&gid)?.is_some() {
                return Ok(hex::encode(&gid));
            }
        }
        let name = self.dir.my_display_name().unwrap_or_default();
        let id_hex = self.object_new("group", &name)?;
        let gid = hex::decode(&id_hex)
            .map_err(|e| CoreError::Directory(format!("self object id hex: {e}")))?;
        self.dir.set_my_object(&gid)?;
        Ok(id_hex)
    }

    /// The directory kind of an object this device holds.
    pub fn object_kind(&self, object_id_hex: &str) -> Result<String, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        self.dir
            .group_kind(&group_id)?
            .ok_or_else(|| CoreError::Directory(format!("no object {object_id_hex}")))
    }

    /// This account's own GroupObject, if it has one. `None` only for a directory
    /// that predates the record and has not written since.
    pub fn self_object(&self) -> Result<Option<String>, CoreError> {
        Ok(self.dir.my_object()?.map(hex::encode))
    }

    // ---- `pacific id show` ----
    pub fn identity_key(&self) -> String {
        self.id.identity_key()
    }
    pub fn fingerprint(&self) -> String {
        self.id.fingerprint()
    }
    pub fn sas(&self) -> String {
        self.fingerprint()
    }

    /// This identity's RECOVERY KEY — 24 BIP-39 words carrying the seed the whole
    /// identity derives from. `None` when there is nothing honest to show (an
    /// identity minted before seeds existed). Never leaves the device; the first
    /// boot sequence shows it once and it can be re-read from the profile page.
    pub fn recovery_key(&self) -> Option<String> {
        self.id.recovery_key()
    }
    /// The read-aloud three-word SAS — the same identity as `sas()`/`fingerprint()`,
    /// rendered for humans to confirm "verify you match".
    pub fn sas_words(&self) -> String {
        self.id.fingerprint_words()
    }

    /// This device's own identity display name (chosen at `id init`). The real
    /// self-name — what the UI's own avatar/profile should read, and the name a
    /// peer sees for us in a pairing bundle.
    pub fn display_name(&self) -> Result<String, CoreError> {
        self.dir.my_display_name()
    }

    /// SIGN A SERVER-ISSUED CHALLENGE — how an account proves itself to an Arc.
    ///
    /// The Arc issues a single-use nonce, this signs it, and the Arc verifies
    /// against the identity public key it already knows the account by. The
    /// passkey is not involved at any point: it unwraps the seed on this device
    /// and never leaves the authenticator.
    ///
    /// The payload is [`identity::auth_payload`] — one definition shared with the
    /// verifier, for the reason stated there.
    ///
    /// DELIBERATELY NOT A SIGNING ORACLE. The caller supplies an audience and a
    /// nonce, never the bytes themselves, so what gets signed always says what it
    /// is for and who it is for.
    ///
    /// THIS REPLACES `arc_member_credential`, which built
    /// `<identity_key>:<ts_ms>:<sig_hex>` — a token the Arc's verifier could
    /// never parse, because an identity key renders as `ed25519:<hex>` and so
    /// already contains the colon the verifier splits on. It had no callers (the
    /// working copy lives in `pacific-ffi`), and a server nonce closes the replay
    /// window its timestamp left open.
    pub fn sign_challenge(&self, audience: &str, nonce: &str) -> Result<String, CoreError> {
        if nonce.trim().is_empty() {
            return Err(CoreError::Identity(
                "a challenge with no nonce is not a challenge".into(),
            ));
        }
        let me = self.identity_key();
        let sig = self.id.sign(&identity::auth_payload(audience, &me, nonce));
        Ok(hex::encode(sig))
    }

    /// Rebuild (signing_identity, secret_key) from persisted bytes — passed into
    /// `mls::build_client` inline by each op (the client type is `impl MlsConfig`,
    /// so it can't be returned through a named helper).
    fn signer(&self) -> Result<(mls::SigningIdentity, mls::SecretKey, String), CoreError> {
        let (sk_bytes, pk_bytes) = self.dir.mls_signing_keypair()?;
        let name = self.dir.my_display_name()?;
        // credential id = the stable Ed25519 identity pubkey (unique per party).
        let sid = mls::signing_identity(&self.id.identity_pk(), &pk_bytes);
        Ok((sid, mls::SecretKey::new(sk_bytes), name))
    }

    // ---- `pacific pair --show` ----
    pub fn build_contact_bundle(&self) -> Result<String, CoreError> {
        self.contact_bundle_living(None)
    }

    /// A contact bundle whose key package lives `life` (W-96, a contact code): past it, an
    /// adder's mls-rs refuses the package, and no one is added by it.
    pub fn build_contact_bundle_living(&self, life: std::time::Duration) -> Result<String, CoreError> {
        self.contact_bundle_living(Some(life))
    }

    fn contact_bundle_living(&self, life: Option<std::time::Duration>) -> Result<String, CoreError> {
        let (sid, sk, name) = self.signer()?;
        let client = match life {
            Some(life) => mls::build_client_sqlite_living(&paths::db_path(), sid, sk, life)?,
            None => mls::build_client_sqlite(&paths::db_path(), sid, sk)?,
        };
        let kp = mls::make_key_package_bytes(&client)?;
        let intro = self.dir.my_intro_tag()?;
        let bundle = handshake::build_bundle(&self.id, kp, intro, &name)?;
        bundle.encode()
    }

    // ---- `pacific pair --scan <bundle>` (scanner: create group + send Welcome) ----
    /// Scan a peer's bundle into a 2-member CONNECTION — the default user↔user tether.
    pub async fn pair_scan(&self, bundle_str: &str) -> Result<[u8; 32], CoreError> {
        self.pair_scan_kind(bundle_str, "connection").await
    }

    /// Like [`pair_scan`], but the created 2-member group carries `kind` instead of
    /// `"connection"`. Used for Arc↔Arc tethers (`kind = "arc-tether"`): a 2-member
    /// GroupObject that carries the role/op machinery rather than the connection projection.
    pub async fn pair_scan_kind(
        &self,
        bundle_str: &str,
        kind: &str,
    ) -> Result<[u8; 32], CoreError> {
        self.pair_scan_full(bundle_str, kind, None).await
    }

    /// Pair, stating WHY — the reason rides the sealed intro payload and is persisted on
    /// the host's peer row.
    ///
    /// This is what makes a PUBLIC PLACE self-serve: the scanner pairs with
    /// `why = "place-join:<place id>"`, and the host replays that intent on every sync
    /// until it can admit them ([`Self::reconcile_place_joins`]). The reason has to travel
    /// in-band rather than as a follow-up message, because at scan time the two parties
    /// have no channel yet — the connection being formed IS the first one.
    pub async fn pair_scan_why(&self, bundle_str: &str, why: &str) -> Result<[u8; 32], CoreError> {
        self.pair_scan_full(bundle_str, "connection", Some(why))
            .await
    }

    async fn pair_scan_full(
        &self,
        bundle_str: &str,
        kind: &str,
        why: Option<&str>,
    ) -> Result<[u8; 32], CoreError> {
        let bundle = handshake::parse_and_verify(bundle_str)?;
        let peer_id = bundle.identity_pk;
        // THE SAME BINDING CHECK `add_member_core` makes, and for the same reason:
        // this door also grafts a scanned key package into a ratchet tree. A bundle
        // signed by mallory around a key package carrying ada's credential would make
        // the connection we are about to create read every one of mallory's messages
        // as ADA's. See `handshake::verify_key_package_identity`.
        handshake::verify_key_package_identity(&peer_id, &bundle.key_package)?;

        // create the 2-party group (a connection is just a group of 2), add the
        // peer -> Welcome. We are the owner.
        let (sid, sk, name) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        // Legacy when the peer's build predates the owner extension (§3.2's bridge):
        // otherwise a phone on an older build could never pair or join an Arc again.
        let (group, _commit, welcome, legacy) =
            mls::create_group_with_peer(&client, &bundle.key_package)?;
        let group_id = group.group_id().to_vec();
        tracing::info!(target: "pacific::membership", group = %hex::encode(&group_id),
                       kind, peer = %hex::encode(peer_id), legacy, "paired");
        // The scanner CREATED this group, so it is a member from epoch 0 (RFC 9420
        // §11). The scanned side joins through a Welcome and writes its own.
        self.note_on_spine(crate::spine::Body::Group(crate::spine::Joined {
            group_id: group_id.clone(),
            first_epoch: 0,
        }))?;

        // SCANNING IS THE SCANNER'S OPT-IN, so the scanner is CONNECTED here, not
        // pending. The double-opt-in gate belongs on the scanned side alone
        // (`PendingIn`, set in `process_intro_blob`): reading someone's code in
        // person IS the act of consent, and there is nobody left for this side to
        // wait on. `pair_accept` sends nothing to the peer — it is purely local —
        // so a `PendingOut` here could never be cleared by anything the peer does.
        //
        // It was not merely cosmetic: `connected_group` (the send path) refuses to
        // author a DM unless MY row is Connected, so a `PendingOut` scanner could
        // not message the person they had just scanned until they tapped "Accept"
        // on their own device — an accept bar that appeared to be about the peer.
        let now = unix_now();
        self.dir.upsert_peer(
            &peer_id,
            &bundle.display_name,
            PeerStatus::Connected,
            Some(&bundle.next_key_commit),
            Some(&bundle.intro_tag),
            now,
        )?;
        // Whoever creates the object determines its governing Arc: stamp OUR default.
        let my_arc = self.my_default_arc();
        self.dir
            .put_group(&group_id, kind, &self.id.identity_pk(), Some(&my_arc))?;
        self.dir
            .set_group_members(&group_id, &mls::roster_identities(&group)?)?;
        // The owner from epoch 0, and this leaf's freshness (§10.2, §11).
        self.dir.record_owner(&group_id, 0, &self.id.identity_pk())?;
        self.dir.record_self_update(&group_id, now, group.current_epoch())?;

        // bootstrap hop: seal {scanner_pk, kind, arc, welcome} to the peer's intro_tag,
        // keyed by the intro_tag itself (a shared capability from the in-person bundle).
        // Carrying `arc` lets the peer route to the SAME Arc we chose for this object.
        let payload = handshake::IntroPayload {
            scanner_pk: self.id.identity_pk(),
            scanner_name: name,
            welcome,
            why: why.map(str::to_string),
            kind: Some(kind.to_string()),
            arc: Some(my_arc),
            // We just minted this group with ourselves as owner (put_group above).
            owner: Some(self.id.identity_pk()),
            // NONE, and deliberately: this group was created two statements ago and
            // nothing has been authored into it, so its high-water mark is 0 and a
            // floor of 0 is what None already means. Sending Some(0) would be the
            // same value dressed as a decision.
            gen_watermark: None,
        }
        .encode()?;
        let dest = bundle.intro_tag;
        let sealed = seal::seal(&payload, &dest, &dest)?;

        let mut sess = Router::open(&self.routes).await?;
        sess.publish(
            &Address::from_seed(&dest),
            &pacific_wire::blob_b64(&sealed),
        )
        .await?;
        sess.close().await;
        // Establish the Contact: seed the peer's identity Group + edge, and stock our
        // prekeys into this channel (so they can add us later, no code exchange). Prekeys are
        // Contact-typed and ride the SAME log as this tether's Group ops — ONE log, MANY
        // op-groups. Each fold takes only its own lens's type (`folded_group` filters to Group,
        // `folded_prekeys` to Contact), so the two vocabularies coexist without interfering.
        if let Err(e) = self.top_up_prekeys(&group_id).await {
            tracing::warn!(error = %e, "prekey stocking failed at pairing; sync will top up");
        }
        // Announce our CURRENT profile on the new channel straight away. The pairing
        // bundle carried only a display name, and it carried it once — everything
        // since (photo, clip, org, links) has to arrive as a delta or not at all.
        // `sync_once`'s reconciliation would catch this within a tick anyway; doing
        // it here means the peer sees a real card the moment the pairing lands.
        if kind == "connection" {
            // THE SCAN IS THE SCANNER'S WISH for the link (`contact.setLink` active 1), and
            // its card waits on it.
            self.want_link(&group_id).await?;
            let (display_name, shape, card) = self.my_profile()?;
            match self
                .publish_profile_into(&group_id, &display_name, shape, &card)
                .await
            {
                Err(e) => tracing::warn!(error = %e, "initial profile publish failed; sync will retry"),
                Ok(false) => {}
                Ok(true) => {
                    let card_json = serde_json::to_string(&card).unwrap_or_default();
                    let digest = Self::profile_digest(&display_name, shape, &card_json);
                    let _ = self
                        .dir
                        .set_published_profile_digest(&group_id, &digest, unix_now());
                }
            }
        }
        Ok(peer_id)
    }

    // ---- `pacific pair accept <space>` (records OUR Connected side) ----
    /// Accept a pending connection: this side's `contact.setLink` active 1, then our prekeys.
    ///
    /// The stocking half was missing entirely. `pair_scan_kind` stocks only the
    /// SCANNER's offers, so until this ran the scanner held ZERO offers from us and
    /// `add_contact_to_object` could never add us to any object — permanently, and
    /// with an error pointing at a "replenish" verb nothing implemented. Both
    /// `process_intro_blob` and `reconcile_place_joins` already documented the
    /// stocking as happening here; now it does.
    ///
    /// Stocking is best-effort: a relay hiccup must not block the acceptance, and
    /// `replenish_prekeys` on the next sync tops up whatever did not land.
    pub async fn pair_accept(&self, peer_id: &[u8; 32]) -> Result<(), CoreError> {
        self.dir.set_peer_status(peer_id, PeerStatus::Connected)?;
        if let Some(group_id) = self.dir.connection_group_for(peer_id)? {
            // THIS SIDE'S WISH for the link, on the connection, so it survives a new device.
            self.want_link(&group_id).await?;
            if let Err(e) = self.top_up_prekeys(&group_id).await {
                tracing::warn!(error = %e, "prekey stocking failed on accept; sync will top up");
            }
            // WHO OPENS THE CONVERSATION, decided once and by the pairing itself.
            //
            // Two people who message each other before either has synced would
            // otherwise derive one conversation EACH and split the thread between
            // them, with no way to tell afterwards which was meant. So the SCANNED
            // side opens it — the side that did not create the connection — and it
            // can do so immediately, because the scanner published its prekeys into
            // the channel during `pair_scan` and this side drained them to get here.
            // The scanner learns of it from the Welcome on its next sync.
            let mine = self
                .dir
                .group_owner(&group_id)?
                .map(|o| o == self.id.identity_pk())
                .unwrap_or(false);
            if !mine {
                if let Err(e) = self.conversation_open(peer_id).await {
                    tracing::warn!(error = %e, "the conversation could not be opened at accept");
                }
            }
        }
        Ok(())
    }

    /// This side's wish for the link on connection `group_id`: `contact.setLink` active 1,
    /// through the one write path. Nothing when it is 1 already.
    async fn want_link(&self, group_id: &[u8]) -> Result<(), CoreError> {
        if self.folded_contact(group_id)?.wants_link(&self.id.identity_pk()) {
            return Ok(());
        }
        self.apply(&hex::encode(group_id), crate::contact::OP_SET_LINK, crate::contact::set_link_args(true))
            .await
            .map(|_| ())
    }

    // ---- `pacific group add <group> <bundle>` (owner adds a member to a group) ----
    /// Add a member (by their pairing bundle) to an EXISTING group we own. Owner-
    /// only: MLS lets any member commit, but Pacific sequences membership through
    /// the owner so there are no concurrent-commit races. Distributes the commit to
    /// existing members over the OLD-epoch group tag, and the Welcome to the new
    /// member's intro mailbox.
    pub async fn group_add_member(
        &self,
        group_id_hex: &str,
        bundle_str: &str,
    ) -> Result<[u8; 32], CoreError> {
        let bundle = handshake::parse_and_verify(bundle_str)?;
        self.add_member_core(
            group_id_hex,
            &bundle.identity_pk,
            &bundle.key_package,
            &bundle.display_name,
            &bundle.next_key_commit,
            &bundle.intro_tag,
        )
        .await
    }

    /// Add an EXISTING contact to `object_id_hex` by CONSUMING one of their stocked
    /// prekeys — NO live code exchange (the whole point of the prekey pool). Picks a
    /// usable, self-contained offer from our Contact channel with them (key package +
    /// their intro tag), adds them with it, then authors a `prekeyConsume` tombstone so
    /// the offer is never reused and the peer knows to replenish. Owner-only, like any add.
    pub async fn add_contact_to_object(
        &self,
        object_id_hex: &str,
        peer_id: &[u8; 32],
    ) -> Result<[u8; 32], CoreError> {
        let contact_group = self
            .dir
            .connection_group_for(peer_id)?
            .ok_or_else(|| CoreError::NotConnected(hex::encode(peer_id)))?;
        let (kp_id, kp, intro_tag) = self
            .folded_prekeys(&contact_group)?
            .pick_from(peer_id, unix_now() as u64)
            .ok_or_else(|| {
                CoreError::Coordinator(format!(
                    "no usable prekey for contact {} — ask them to replenish",
                    hex::encode(peer_id)
                ))
            })?;
        let display_name = self.dir.peer_display_name(peer_id)?;
        let added = self
            .add_member_core(
                object_id_hex,
                peer_id,
                &kp,
                &display_name,
                &[0u8; 32], // next_key_commit: rotation bookkeeping only; the known peer's row already holds it
                &intro_tag,
            )
            .await?;
        let _ = self.author_prekey_consume(&contact_group, &kp_id).await;
        Ok(added)
    }

    /// Add a member of a Site to one of its parts (its Host, a room) by their identity
    /// alone: Make admin's second step (ICD 2.1.0 row 10). The part's parent is its own
    /// `base.setParent`, and only someone on THAT Site's current roster may be added this
    /// way, so the call cannot bring a stranger onto a part. The Add is the part owner's
    /// (`add_member_core`'s gate), and its key package is the member's prekey in the owner's
    /// contact channel with them (`add_contact_to_object`): nowhere else holds one for a
    /// person who is not handing it over. Refused, in words, when the part names no Site,
    /// when the member is not on it, and when they are not the owner's contact.
    pub async fn add_part_member(&self, part_hex: &str, member: &[u8; 32]) -> Result<[u8; 32], CoreError> {
        let view: serde_json::Value = serde_json::from_str(&self.object_view(part_hex)?)
            .map_err(|e| CoreError::Coordinator(format!("{part_hex}'s view is not JSON: {e}")))?;
        let site = view["parent"]["parent"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| CoreError::Membership(format!("{part_hex} is a part of no Site")))?
            .to_string();
        if !self.group_roster(&site)?.contains(member) {
            return Err(CoreError::Membership(format!("{} is not a member of {site}", hex::encode(member))));
        }
        self.add_contact_to_object(part_hex, member).await.map_err(|e| match e {
            CoreError::NotConnected(_) => CoreError::Membership(format!(
                "{} is not your contact: ask them to connect with you first",
                hex::encode(member)
            )),
            other => other,
        })
    }

    /// Author a `prekeyConsume` tombstone for `kp_id` into a Contact channel.
    async fn author_prekey_consume(
        &self,
        contact_group_id: &[u8],
        kp_id: &[u8; 32],
    ) -> Result<[u8; 32], CoreError> {
        let (sid, sk, _n) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut group = mls::load_group(&client, contact_group_id)?;
        let epoch = group.current_epoch();
        let gen = self
            .dir
            .author_delta_count(contact_group_id, &self.id.identity_pk())?;
        let delta = crate::object::build_delta(
            crate::object::ObjectKind::Contact,
            crate::contact::OP_PREKEY_CONSUME,
            crate::contact::id_args(kp_id),
            epoch,
            Some(gen),
        );
        self.append_and_flush(&mut group, contact_group_id, delta)
            .await
    }

    /// Whether a NON-OWNER member may add to this object — true only at a public door.
    ///
    /// Two cases, and no others: the object IS a Place that is currently open, or it is a
    /// Group that anchored itself at a Place that is currently open. Both re-read the
    /// posture live, so closing a Place immediately withdraws the permission from every
    /// member, everywhere, without a further delta.
    ///
    /// Fails CLOSED on any read error: an object whose state will not fold is not one to
    /// start relaxing authority around.
    fn admits_by_any_member(&self, object_id_hex: &str) -> bool {
        // Dispatch on the RECORDED KIND, never by trying folds until one answers:
        // folding a group's log through the Place lens filters every delta out and
        // returns an honest EMPTY state — whose access is Private — so the old
        // try-place-first shape short-circuited to "no" for every group and quietly
        // made anchored-group admission owner-only (m14, the hosting contract's H2).
        let Ok(gid) = self.object_group_id(object_id_hex) else {
            return false;
        };
        match self.dir.group_kind(&gid) {
            Ok(Some(k)) if k == "place" => self
                .place_state(object_id_hex)
                .map(|p| p.access.admits_strangers())
                .unwrap_or(false),
            Ok(Some(k)) if k == "group" => {
                let Ok(anchored) = self.group_anchored_places(object_id_hex) else {
                    return false;
                };
                anchored.iter().any(|(place_id, _)| {
                    self.place_state(place_id)
                        .map(|p| p.access.admits_strangers())
                        .unwrap_or(false)
                })
            }
            _ => false,
        }
    }

    /// The shared MLS owner-add: distribute the commit to existing members over the
    /// OLD-epoch tag, seal the Welcome to the newcomer's `intro_tag`. Reached by both a
    /// fresh bundle (`group_add_member`) and a stocked prekey (`add_contact_to_object`).
    async fn add_member_core(
        &self,
        group_id_hex: &str,
        newcomer: &[u8; 32],
        key_package: &[u8],
        display_name: &str,
        next_key_commit: &[u8; 32],
        intro_tag: &[u8; 32],
    ) -> Result<[u8; 32], CoreError> {
        // THE BINDING CHECK, AND IT GOES FIRST — before the owner gate, before any
        // group state is read, staged, sealed or published.
        //
        // `newcomer` is an identity somebody VOUCHED FOR: `group_add_member` took it
        // from a bundle whose self-signature verified, `add_contact_to_object` took it
        // from our own directory. `key_package` is the thing that actually becomes a
        // LEAF, and nothing so far has related the two. A bundle signed by mallory
        // around a key package carrying ada's BasicCredential passes every check above
        // this line, and the leaf it grafts authors as ADA: `mls::member_identity`
        // reads the credential, and the fold keys authorship and AUTHORITY on it.
        //
        // mls-rs used to refuse that by accident — `BasicIdentityProvider::identity()`
        // returns the credential verbatim, so a credential already in the tree was
        // `DuplicateLeafData`. A1a's `PerLeafIdentity` returns credential ‖
        // signature_key so that one person may hold two leaves, which is the feature;
        // the side effect is that the forgery is now legal MLS. It is refused HERE
        // instead, on credential equality alone, so a person's second device — same
        // credential, fresh signature key — still passes untouched.
        handshake::verify_key_package_identity(newcomer, key_package)?;
        let group_id = self.object_group_id(group_id_hex)?;
        let me = self.id.identity_pk();
        // Read the owner ONCE: it both gates the add and rides the Welcome, so the
        // newcomer records the object's true owner rather than whoever admitted them.
        let object_owner: [u8; 32] = match self.dir.group_owner(&group_id)? {
            Some(o) => o
                .as_slice()
                .try_into()
                .map_err(|_| CoreError::Directory("owner_pk width".into()))?,
            None => return Err(CoreError::Directory(format!("no group {group_id_hex}"))),
        };
        // ANY MEMBER MAY ADMIT AT A PUBLIC DOOR. Narrowly scoped: only when this
        // object is a public Place, or a Group that agreed to answer for one. Every
        // other object stays owner-only — a Forum or a Project must not start
        // accepting adds from whoever happens to be in it.
        //
        // This is what stops a public place depending on one person's phone. It is
        // safe because the door is the thing that was made public: opening a Place is
        // a deliberate, owner-authored, revocable act, and answering it is exactly the
        // duty an anchored group signed up for.
        // AN ADMITTER MAY ADD (A-3, D-53): the role a founder grants the Arc's always-on
        // node, so a visitor is admitted with no owner present. Admission only.
        if object_owner != me && !self.admits_by_any_member(group_id_hex) && !self.is_admitter(group_id_hex, &me) {
            return Err(CoreError::Coordinator(
                "only the group owner can add members".into(),
            ));
        }
        let kind = self
            .dir
            .group_kind(&group_id)?
            .unwrap_or_else(|| "connection".to_string());
        // Group-typed objects record the arrival (§10.3), after the Add wins.
        let records_membership = group::is_group_typed(&kind);
        let newcomer = *newcomer;

        let (sid, sk, name) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut sess = Router::open(&self.routes).await?;

        // PENDING-COMMIT DISCIPLINE (RFC 9420 §14: commit generation MUST NOT
        // modify state): stage the add, publish the commit into the epoch tag's
        // single COMMIT SLOT on the relay, and only apply it locally after
        // winning the slot. The relay accepts exactly one commit per epoch tag,
        // first-writer-wins, over opaque bytes.
        //
        // A LOST SLOT IS RETRIED, as `commit_via_slot` retries. Every sync now
        // commits pool leaves (resumption.md §6.5), so an add that gave up on its
        // first loss would fail whenever it met one; the loser's staged commit was
        // never applied, so folding the winner and re-staging is always safe.
        let mut round = 0;
        let (mut group, welcome, leaves_before) = loop {
            let mut group = mls::load_group(&client, &group_id)?;
            // The newcomer's leaves before this Add, so the one it makes can be named
            // if its Welcome cannot go (NC-36).
            let leaves_before = mls::leaves_of(&group, &newcomer);
            // Capture the CURRENT (pre-add) epoch's tag + secret BEFORE the add
            // advances us, so existing members — still at that epoch — can drain the
            // commit off it.
            let old_epoch_n = group.current_epoch();
            let old_epoch = mls::epoch_be(old_epoch_n);
            let old_tag = mls::group_tag(&group, &old_epoch)?;
            let old_addr = mls::group_address(&group, &old_epoch)?;
            let old_secret = mls::seal_conn_secret(&group, &old_epoch)?;
            // Record the pre-add epoch tag so we can re-drain it for any application
            // message a member posts to it before processing this commit.
            self.dir
                .record_epoch_tag(&group_id, old_epoch_n, &old_tag, &old_secret)?;
            let (commit, welcome) = mls::stage_add_member(&mut group, key_package)?;
            let sealed_commit = seal::seal(&commit, &old_tag, &old_secret)?;
            let won = sess
                .publish_commit(&old_addr, &pacific_wire::blob_b64(&sealed_commit))
                .await?;
            if won.is_some() {
                break (group, welcome, leaves_before);
            }
            round += 1;
            let mut fresh = mls::load_group(&client, &group_id)?;
            let _ = self
                .drain_tag(&mut sess, &mut fresh, &group_id, &old_tag, &old_secret)
                .await;
            if round >= 8 || fresh.current_epoch() == old_epoch_n {
                sess.close().await;
                return Err(CoreError::CommitRace(format!(
                    "epoch {old_epoch_n} commit slot already taken — synced past the winner; retry the add"
                )));
            }
        };
        // WON: apply + persist, then update projections from the APPLIED roster.
        mls::apply_staged(&mut group)?;
        self.after_own_commit(&group_id, &group)?;
        tracing::info!(
            target: "pacific::membership",
            group = %group_id_hex,
            newcomer = %hex::encode(newcomer),
            epoch = group.current_epoch(),
            "add committed"
        );
        self.dir.upsert_peer(
            &newcomer,
            display_name,
            PeerStatus::Connected,
            Some(next_key_commit),
            Some(intro_tag),
            unix_now(),
        )?;

        // 2. seal the WELCOME to the newcomer's intro mailbox (carries the kind).
        //    Ordered AFTER the commit won its slot — the Welcome for a commit
        //    "MUST NOT be delivered to a new joiner until it's clear that the
        //    Commit has been accepted" (RFC 9420 §14).
        // Propagate the OBJECT's governing Arc (the creator's choice), not the
        // sharer's default — the newcomer must land under the same jurisdiction and
        // route to the same relay as the existing members.
        let object_arc = self
            .dir
            .group_arc(&group_id)?
            .or_else(|| Some(self.my_default_arc()));
        let payload = handshake::IntroPayload {
            scanner_pk: me,
            scanner_name: name,
            welcome,
            why: None,
            kind: Some(kind),
            arc: object_arc,
            // The OBJECT's owner, which at a public door is not necessarily us — the
            // newcomer's fold validates owner-sequenced deltas against this.
            owner: Some(object_owner),
            // THE GEN FLOOR. We hold this group's log and the joiner cannot — forward
            // secrecy, and the comment below about NO message backfill is the same
            // rule seen from the other side. So the one number it needs in order not
            // to re-mint a ref its own person already used has to travel in the one
            // message that reaches it before it can author anything. See
            // `IntroPayload::gen_watermark`.
            gen_watermark: Some(self.next_lamport(&group_id)?),
        }
        .encode()?;
        let sealed_welcome = seal::seal(&payload, intro_tag, intro_tag)?;
        // A ROSTER MEMBER WITH NO WELCOME HAS NO WAY IN (NC-36). The Welcome goes after
        // the commit won, as RFC 9420 §14 requires; if it still cannot go after three
        // tries (or at once, when the relay says it is too large), the Add is undone:
        // the leaf it made is removed through the slot, and the caller is told.
        let blob = pacific_wire::blob_b64(&sealed_welcome);
        let dest = Address::from_seed(intro_tag);
        let mut refused = None;
        for attempt in 0..3u64 {
            match sess.publish(&dest, &blob).await {
                Ok(_) => {
                    refused = None;
                    break;
                }
                Err(e) => {
                    let permanent = e.to_string().contains("too_large");
                    refused = Some(e);
                    if permanent {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(250 * (attempt + 1))).await;
                    if let Ok(s) = Router::open(&self.routes).await {
                        sess = s;
                    }
                }
            }
        }
        if let Some(e) = refused {
            let new_leaf = mls::leaves_of(&group, &newcomer).into_iter().find(|l| !leaves_before.contains(l));
            let undone = match new_leaf {
                Some(leaf) => self
                    .commit_via_slot(&mut sess, &group_id, "undo an Add whose Welcome did not go", 8, move |g| {
                        mls::stage_remove(g, &[leaf]).map(Some)
                    })
                    .await
                    .map(|_| ()),
                None => Err(CoreError::Membership("the Add's leaf could not be found".into())),
            };
            sess.close().await;
            tracing::warn!(target: "pacific::membership", group = %group_id_hex,
                           newcomer = %hex::encode(newcomer), error = %e, undone = undone.is_ok(),
                           "the Welcome could not be delivered");
            return Err(match undone {
                Ok(()) => CoreError::Transport(format!(
                    "the Add was undone: its Welcome could not be delivered — {e}"
                )),
                Err(u) => CoreError::Transport(format!(
                    "the Add stands without a Welcome (leaf {new_leaf:?}), and only the owner can remove it — the Welcome: {e}; the undo: {u}"
                )),
            });
        }
        // NO message backfill: MLS forward secrecy means a member joining at this
        // epoch cannot decrypt anything sealed at earlier epochs, and we do NOT
        // re-encrypt-and-resend history (that would deliberately defeat forward
        // secrecy — the Signal model this app pins). Durable group STATE (name,
        // roster) rides the Welcome's GroupInfo instead: the name is a GroupContext
        // extension (see `mls::group_name`), the roster is the ratchet tree.
        sess.close().await;
        // THE RECORD OF THE ADD (§10.3), written by the device that committed it, at the
        // epoch the newcomer joined — so the newcomer can read its own arrival.
        if records_membership {
            if let Err(e) = self
                .write_membership_record(group_id_hex, crate::membership::OP_MEMBER_JOINED, &newcomer, None)
                .await
            {
                tracing::warn!(target: "pacific::membership", group = %group_id_hex,
                               newcomer = %hex::encode(newcomer), error = %e,
                               "the Add stands, but its record could not be written");
            }
        }
        // STATE RE-EMISSION, the door's own discipline applied to every Add: restate
        // our durable deltas at the epoch the newcomer holds. The solo-era outbox used
        // to do this for a FIRST add only; a pool leaf (resumption.md §5) means no
        // object stays solo, so without this every newcomer would fold it empty.
        if let Err(e) = self.reemit_own_spine(group_id_hex).await {
            tracing::warn!(target: "pacific::membership", group = %group_id_hex, error = %e,
                           "the Add stands, but our durable state was not restated for the newcomer");
        }
        Ok(newcomer)
    }

    // ---- the single write path: author ONE post to a CONNECTION peer ----
    pub async fn author_post(&self, peer_id: &[u8; 32], text: &str) -> Result<[u8; 32], CoreError> {
        self.author_post_reply(peer_id, text, None, None).await
    }

    /// Author a post to a connection, optionally as a REPLY to `reply_to` (the
    /// (author, gen) of the message being answered). Same single write path.
    pub async fn author_post_reply(
        &self,
        peer_id: &[u8; 32],
        text: &str,
        reply_to: Option<coordinator::MsgRef>,
        media: Option<&pacific_media::MediaRef>,
    ) -> Result<[u8; 32], CoreError> {
        let group_id = hex::decode(self.conversation_open(peer_id).await?)
            .map_err(|e| CoreError::Coordinator(format!("conversation id: {e}")))?;
        self.post_to_group(&group_id, text, reply_to, media).await
    }

    /// React to (or clear a reaction on) a message in a connection DM. `target`
    /// is the (author, gen) of the message; `active=false` clears our reaction.
    pub async fn react_dm(
        &self,
        peer_id: &[u8; 32],
        target: coordinator::MsgRef,
        emoji: &str,
        active: bool,
    ) -> Result<[u8; 32], CoreError> {
        let group_id = hex::decode(self.conversation_open(peer_id).await?)
            .map_err(|e| CoreError::Coordinator(format!("conversation id: {e}")))?;
        self.react_in_group(&group_id, target, emoji, active).await
    }

    /// THE CONVERSATION WITH `peer`, if this device holds one.
    ///
    /// A CONNECTION IS NOT A CHAT. The ICD is explicit — channel 25 is "the 1:1
    /// pairing channel: prekey pool + pairwise fan-out. Not an entity itself" —
    /// and the messages belong to a Conversation (26). Until 24 September 2026 the
    /// DM was posted into the connection as a Forum delta, so a connection's log
    /// carried two vocabularies and its lens claimed it was a chat room.
    pub fn conversation_with(&self, peer_id: &[u8; 32]) -> Result<Option<String>, CoreError> {
        Ok(self.dir.conversation_group_for(peer_id)?.map(hex::encode))
    }

    /// The conversation with `peer`, DERIVED if there is none.
    ///
    /// `mint::classify(Conversation)` answers `Derived`, and this is the
    /// derivation: a group of one, then the peer added by CONSUMING a prekey the
    /// connection carries — which is what the prekey pool is for, and why the
    /// pairing channel stays a pairing channel. Both sides get a vertebra, theirs
    /// written when the Welcome lands.
    pub async fn conversation_open(&self, peer_id: &[u8; 32]) -> Result<String, CoreError> {
        if let Some(id) = self.conversation_with(peer_id)? {
            return Ok(id);
        }
        // Connected, or there is no prekey to consume and nobody to Welcome.
        if self.dir.peer_status(peer_id)? != PeerStatus::Connected {
            return Err(CoreError::NotConnected(hex::encode(peer_id)));
        }
        // LOOK BEFORE MAKING A SECOND ONE. Pairing opens the conversation on the
        // scanned side, so the other side's copy arrives as a Welcome — and a
        // device that has not synced since would otherwise create its own and
        // split the thread between two objects. Best-effort: offline, we open one
        // and the two converge when they meet, which is the rare case this turns
        // the common one into.
        let _ = self.sync_once().await;
        if let Some(id) = self.conversation_with(peer_id)? {
            return Ok(id);
        }
        let name = self.dir.peer_display_name(peer_id)?;
        let id = self.object_new("conversation", &name)?;
        // THE PREKEY MAY NOT HAVE CROSSED YET. Both sides stock at pairing, but
        // the offers travel as deltas and a device that has not synced since does
        // not hold the peer's. That is a sync away, not a refusal — the same
        // self-healing shape as the profile fan-out — so try, sync, and try once
        // more before saying there is nothing to consume.
        //
        // LOUD if it still fails, and the object stays: it is already on the
        // spine, and calling this again adds the peer to the one that exists.
        if let Err(first) = self.add_contact_to_object(&id, peer_id).await {
            tracing::info!(error = %first, "no prekey in hand for the conversation; syncing");
            self.sync_once().await?;
            self.add_contact_to_object(&id, peer_id).await?;
        }
        Ok(id)
    }

    /// Every 1:1 connection we hold: (peer_pk, display_name, connected). The peer
    /// pk is the member of the `connection` group that isn't us; `connected` is
    /// true once the double-opt-in has moved the peer to `Connected`. Reads the
    /// directory projection only — no MLS load, no relay.
    #[allow(clippy::type_complexity)]
    pub fn connections(&self) -> Result<Vec<([u8; 32], String, bool)>, CoreError> {
        let me = self.id.identity_pk();
        Ok(self
            .dir
            .connections(&me)?
            .into_iter()
            .map(|(pk, name, status)| {
                let connected = status.as_deref() == Some(PeerStatus::Connected.as_str());
                (pk, name, connected)
            })
            .collect())
    }

    /// The `connection` GroupObject we hold for `peer_id`: (object id hex, kind).
    /// `Directory::list_objects` excludes connections on purpose — a connection is
    /// not an object in the LIFE sense — so `objects_named` cannot answer this, and
    /// a reader of the graph needs it: pairing writes a vertebra on the spine like
    /// any other join (`pair_scan_full`, `spine_on_join`), and an enumeration that
    /// omits it is shorter than the spine it claims to draw.
    pub fn connection_object(
        &self,
        peer_id: &[u8; 32],
    ) -> Result<Option<(String, String)>, CoreError> {
        let Some(group_id) = self.dir.connection_group_for(peer_id)? else {
            return Ok(None);
        };
        let kind = self.dir.group_kind(&group_id)?.unwrap_or_default();
        Ok(Some((hex::encode(&group_id), kind)))
    }

    /// The folded transcript of our 1:1 conversation with `peer_id` — the same
    /// deterministic fold `object_transcript` runs, over the connection group.
    /// Loud if there is no connection group for this peer.
    pub fn dm_transcript(&self, peer_id: &[u8; 32]) -> Result<Vec<String>, CoreError> {
        // No CONNECTION is a different fact from nothing said yet, and the two
        // must not read alike: the first is loud, the second is an empty list.
        if self.dir.connection_group_for(peer_id)?.is_none() {
            return Err(CoreError::UnknownPeer(hex::encode(peer_id)));
        }
        match self.dir.conversation_group_for(peer_id)? {
            Some(gid) => self.forum_transcript(&gid),
            None => Ok(Vec::new()),
        }
    }

    /// Author one `forum.post` Delta into a group's log (the single write path):
    /// append locally first (source of truth, queued), then BEST-EFFORT flush to
    /// the shared group tag. A solo group (just us) never touches the relay; a
    /// post made on a dead link stays in the outbox and flushes on the next `sync`.
    /// Drain a group's mailbox to the HEAD epoch (process any pending membership
    /// commits) so a subsequent author seals at the live epoch — never a superseded
    /// one a later joiner can't decrypt. This is `sync_group` step (a) in isolation.
    /// Best-effort by contract: a solo object or an unreachable relay is a no-op, and
    /// the caller authors at the local epoch (the outbox re-flush on next sync then
    /// converges it). Persists advanced MLS state, so the caller's reload sees head.
    async fn drain_to_head(&self, group_id: &[u8]) -> Result<(), CoreError> {
        // LEAVES, not members. One person signed in on two devices is one member
        // and two leaves; counting members reads "solo" and their own objects
        // never reach the relay. See `Directory::group_leaf_count`.
        if self.dir.group_leaf_count(group_id)? <= 1 {
            return Ok(()); // solo: no peers, no commits, never touches the relay
        }
        let (mut sess, home) = match self.write_session(group_id).await {
            Ok(s) => s,
            Err(_) => return Ok(()), // offline: author at the local epoch
        };
        let (sid, sk, _name) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut group = match mls::load_group(&client, group_id) {
            Ok(g) => g,
            Err(_) => {
                self.done_with(sess, home).await;
                return Ok(());
            }
        };
        // Advance by draining the CURRENT tag until no commit advances the epoch,
        // recording each epoch's tag+secret we pass through (mirrors sync_group).
        loop {
            let e = group.current_epoch();
            let eb = mls::epoch_be(e);
            let tag = mls::group_tag(&group, &eb)?;
            let secret = mls::seal_conn_secret(&group, &eb)?;
            self.dir.record_epoch_tag(group_id, e, &tag, &secret)?;
            if !self
                .drain_tag(&mut sess, &mut group, group_id, &tag, &secret)
                .await?
            {
                break;
            }
        }
        self.done_with(sess, home).await;
        Ok(())
    }

    /// BEFORE A WRITE (O-69): what the held connection delivered, ingested with no round
    /// trip; then, if `to_head` and the group is still short of head as far as the relay
    /// knows, a drain on the held session. Best-effort: a failure authors at the local epoch.
    async fn converge_before_writing(&self, group_id: &[u8], to_head: bool) {
        if let Err(e) = self.ingest_delivered() {
            tracing::warn!(target: "pacific::sync", error = %e, "what the held connection delivered was not ingested before a write");
        }
        if to_head && !self.caught_up(group_id).unwrap_or(false) {
            let _ = self.drain_to_head(group_id).await;
        }
    }

    /// THE SESSION A WRITE RIDES (O-69): the held one when the object routes home, so a
    /// drain or a publish pays no dial; a dial to the governing Arc otherwise. `true` when
    /// it is the held one, which `done_with` gives back.
    async fn write_session(&self, group_id: &[u8]) -> Result<(Router, bool), CoreError> {
        let routes = self.routes_for(group_id);
        if routes == self.routes {
            Ok((self.home_session().await?, true))
        } else {
            Ok((Router::open(&routes).await?, false))
        }
    }

    /// A write's session after a clean pass: the held one kept, a dialled one closed. A
    /// session that failed is dropped instead, and the next write dials.
    async fn done_with(&self, sess: Router, home: bool) {
        if home {
            self.keep_session(sess);
        } else {
            sess.close().await;
        }
    }

    /// THE RESUMPTION UPKEEP alone, on the held session (O-69): a write's tail after a mint,
    /// so each new object has its pool leaf, and what it holds can be published and taken
    /// up by another device, before the next sync.
    pub async fn upkeep(&self) -> Result<(), CoreError> {
        let mut sess = self.home_session().await?;
        self.resumption_upkeep_at(&mut sess, unix_now()).await?;
        self.keep_session(sess);
        Ok(())
    }

    /// EVERY OBJECT'S OUTBOX sent (O-69): `publish_pending` for each held object with a
    /// Delta of this device's waiting. The count sent; an object whose link fails keeps
    /// its Deltas for the next sync, and the rest are still sent.
    pub async fn publish_all_pending(&self) -> Result<usize, CoreError> {
        let me = self.id.identity_pk();
        let mut sent = 0;
        for gid in self.held_groups()? {
            if self.dir.pending_outbox(&gid, &me)?.is_empty() {
                continue;
            }
            match self.publish_pending(&hex::encode(&gid)).await {
                Ok(k) => sent += k,
                Err(e) => tracing::warn!(target: "pacific::sync", group = %hex::encode(&gid), error = %e,
                                         "a local commit stays in the outbox for the next sync"),
            }
        }
        Ok(sent)
    }

    /// SEND WHAT A LOCAL COMMIT LEFT IN THE OUTBOX (O-69, the Door's write path after its
    /// answer): the group drained to head, a waiting proposal committed, and each of this
    /// device's undelivered Deltas sealed at the live epoch, published on the held
    /// session and marked delivered on its Ack. A link that fails leaves them in the
    /// outbox for the next sync, as a write made offline does. The count sent.
    pub async fn publish_pending(&self, object_id_hex: &str) -> Result<usize, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        if self.dir.group_leaf_count(&group_id)? <= 1 {
            return Ok(0);
        }
        let me = self.id.identity_pk();
        if self.dir.pending_outbox(&group_id, &me)?.is_empty() {
            return Ok(0);
        }
        self.converge_before_writing(&group_id, true).await;
        let (mut sess, home) = self.write_session(&group_id).await?;
        let (sid, sk, _name) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut group = self.load_member_group(&client, &group_id)?;
        if mls::commit_required(&group) {
            self.commit_pending(&mut sess, &group_id).await?;
            let (sid, sk, _name) = self.signer()?;
            let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
            group = self.load_member_group(&client, &group_id)?;
        }
        let mut sent = 0;
        for (did, env) in self.dir.pending_outbox(&group_id, &me)? {
            self.flush_one(&mut sess, &mut group, &group_id, &did, &env).await?;
            sent += 1;
        }
        self.done_with(sess, home).await;
        Ok(sent)
    }

    async fn post_to_group(
        &self,
        group_id: &[u8],
        text: &str,
        reply_to: Option<coordinator::MsgRef>,
        media: Option<&pacific_media::MediaRef>,
    ) -> Result<[u8; 32], CoreError> {
        // Converge to the HEAD epoch before sealing: a member who has not yet
        // processed the latest membership commit would otherwise author into a
        // superseded epoch that later joiners cannot decrypt (the step-110 bug).
        // Best-effort — offline authors at the local epoch and the outbox re-flushes
        // at the live epoch on the next sync.
        let _ = self.drain_to_head(group_id).await;
        let (sid, sk, _name) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut group = self.load_member_group(&client, group_id)?;
        // Stamp the post with a LAMPORT timestamp (1 + the max gen the sender has
        // already seen), NOT a per-author counter. The transcript sorts by
        // (gen, author), so a Lamport gen makes that a causal SEND ORDER: a
        // message sorts after everything its author had seen when they wrote it.
        // (The old per-author count made my first post — gen 0 — sort above a
        // peer's earlier post that was also their gen 0: the reported "my reply
        // jumped to the top" bug.) Concurrent posts that never saw each other tie
        // on gen and fall back to the deterministic author-bytes order.
        // `ts` is a wall-clock stamp for DISPLAY only (never the sort key).
        let gen = self.next_lamport(group_id)?;
        let delta = coordinator::forum_post_full(
            text,
            gen,
            group.current_epoch(),
            unix_millis(),
            reply_to,
            media,
        )
        .retyped(self.chat_type_id(group_id)?);
        self.append_and_flush(&mut group, group_id, delta).await
    }

    /// Author one `forum.react` Delta (an emoji reaction, or its removal) into a
    /// group's log — same single write path as a post.
    async fn react_in_group(
        &self,
        group_id: &[u8],
        target: coordinator::MsgRef,
        emoji: &str,
        active: bool,
    ) -> Result<[u8; 32], CoreError> {
        // Converge to the head epoch first (see `post_to_group`): react at the live
        // epoch so members on the newer epoch can decrypt it.
        let _ = self.drain_to_head(group_id).await;
        let (sid, sk, _name) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut group = self.load_member_group(&client, group_id)?;
        let gen = self.next_lamport(group_id)?;
        let delta = coordinator::forum_react(target, emoji, active, gen, group.current_epoch())
            .retyped(self.chat_type_id(group_id)?);
        self.append_and_flush(&mut group, group_id, delta).await
    }

    // ---- delivery/read receipts (WhatsApp's ✓ / ✓✓ / ✓✓-blue) --------------
    //
    // A receipt is a commutative `forum.receipt` delta a RECIPIENT authors, carrying
    // a monotonic status and a BATCH of the message refs it acknowledges — Signal's
    // `ReceiptMessage { type, repeated timestamps }` model, not one delta per message.
    // Delivery receipts ride the sync itself (see `sync_group` step (d)): one batched
    // delta per sync that received new messages, flushed on the already-open session —
    // no extra fold, no second relay connect, no per-message write. Read receipts are
    // one batched delta emitted when the user opens the conversation. Both are gated
    // on what is actually OWED (`owed_receipts`), so a quiet conversation authors
    // nothing and a burst of N messages costs ONE receipt, not N.

    /// The refs of foreign messages in `state` we have not yet acknowledged at
    /// `want_status` — the receipts still owed. Skips our own messages (you never
    /// receipt yourself). Pure over an already-folded state so the sync path can
    /// reuse the fold it already did for the transcript.
    fn owed_receipts(
        state: &coordinator::ForumState,
        me: &[u8; 32],
        want_status: u8,
    ) -> Vec<coordinator::MsgRef> {
        state
            .messages
            .keys()
            .copied()
            .filter(|(author, _)| author != me)
            .filter(|r| {
                let have = state
                    .receipts
                    .get(r)
                    .and_then(|m| m.get(me))
                    .copied()
                    .unwrap_or(0);
                have < want_status
            })
            .collect()
    }

    /// Author ONE batched `forum.receipt` acknowledging every ref in `targets` at
    /// `status`, and best-effort flush it — the Signal-style batch: a burst of N
    /// messages is a single delta, not N. Loads the group once; unsent receipts sit
    /// in the outbox and re-flush at the live epoch on the next sync. Returns the
    /// number of refs acknowledged (0 short-circuits before touching MLS/relay).
    async fn author_batch_receipt(
        &self,
        group_id: &[u8],
        targets: &[coordinator::MsgRef],
        status: u8,
    ) -> Result<usize, CoreError> {
        if targets.is_empty() {
            return Ok(0);
        }
        // Converge to head first (see `post_to_group`) so the receipt seals at the
        // live epoch every current member can decrypt.
        let _ = self.drain_to_head(group_id).await;
        let (sid, sk, _name) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut group = mls::load_group(&client, group_id)?;
        let me = self.id.identity_pk();
        let gen = self.next_lamport(group_id)?;
        let delta = coordinator::forum_receipt_batch(targets, status, gen, group.current_epoch())
            .retyped(self.chat_type_id(group_id)?);
        let envelope = delta.canonical_bytes();
        let delta_id = delta.id();
        self.dir
            .append_delta_signed(
            group_id,
            &delta_id,
            &me,
            &envelope,
            Some(&crate::delta_sig::sign_delta(&self.id, group_id, &delta_id)),
            unix_now(),
        )?;
        // Flush for shared groups only (a solo object has no recipients and never
        // owes a receipt anyway); drain the whole outbox so any stranded receipt goes.
        // LEAVES, not members — a second device of ours is a recipient.
        if self.dir.group_leaf_count(group_id)? > 1 {
            let routes = self.routes_for(group_id);
            if let Ok(mut sess) = Router::open(&routes).await {
                for (did, env) in self.dir.pending_outbox(group_id, &me)? {
                    let _ = self
                        .flush_one(&mut sess, &mut group, group_id, &did, &env)
                        .await;
                }
                sess.close().await;
            }
        }
        Ok(targets.len())
    }

    /// Mark every foreign message in `group_id` READ — the ✓✓ (blue), one batched
    /// delta emitted when the user opens the conversation. Read implies delivered, so
    /// this also covers any messages that never got a separate delivery receipt out.
    async fn mark_read_in_group(&self, group_id: &[u8]) -> Result<usize, CoreError> {
        let state = self.forum_state(group_id)?;
        let owed = Self::owed_receipts(&state, &self.id.identity_pk(), coordinator::RECEIPT_READ);
        self.author_batch_receipt(group_id, &owed, coordinator::RECEIPT_READ)
            .await
    }

    /// Read-receipt a (forum) object conversation — the group-object twin of
    /// `dm_mark_read`. Reaches the relay once the object has other members.
    pub async fn object_mark_read(&self, object_id_hex: &str) -> Result<usize, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        self.mark_read_in_group(&group_id).await
    }

    /// Read-receipt a 1:1 conversation with `peer_id`. Loud if not connected;
    /// nothing to acknowledge when nothing has been said, so no conversation is
    /// zero rather than a refusal.
    pub async fn dm_mark_read(&self, peer_id: &[u8; 32]) -> Result<usize, CoreError> {
        if self.dir.peer_status(peer_id)? != PeerStatus::Connected {
            return Err(CoreError::NotConnected(hex::encode(peer_id)));
        }
        match self.dir.conversation_group_for(peer_id)? {
            Some(gid) => self.mark_read_in_group(&gid).await,
            None => Ok(0),
        }
    }

    /// The next Lamport timestamp for a post in `group_id`: one past the highest
    /// `gen` over every Delta already in the folded log (the sender's own posts
    /// plus everything received). Received deltas are appended before this is
    /// read, so `max(gen in log)` is exactly the sender's logical clock.
    fn next_lamport(&self, group_id: &[u8]) -> Result<u64, CoreError> {
        let mut max_gen: Option<u64> = None;
        for (_author, envelope) in self.dir.load_log(group_id)? {
            if let Ok(d) = coordinator::decode_delta(&envelope) {
                if let Some(g) = d.gen {
                    max_gen = Some(max_gen.map_or(g, |m| m.max(g)));
                }
            }
        }
        // THE RULE LIVES IN `object::NextGen`, and this reads it rather than
        // restating it. Both halves matter and either alone is wrong — the log,
        // because a device running for months is past any floor it was handed; the
        // watermark, because a device that joined late has an EMPTY log by forward
        // secrecy, so its log says 0 and every ref it mints collides with one its
        // own PERSON already used elsewhere. The fold keys commutative deltas by
        // (author_pk, gen) and drops one of a colliding pair.
        //
        // This used to compute the max here. It moved because the browser needs
        // the same answer and cannot see this function: a second implementation
        // over there would be a second rule, and the one that drifts is the one
        // nobody is watching.
        Ok(crate::object::NextGen::compute(max_gen, self.dir.gen_floor(group_id)?).get())
    }

    /// The ONE write-path tail shared by every author (forum.post, project.*):
    /// append a built delta locally (source of truth, queued), then BEST-EFFORT
    /// flush to the shared group tag. A solo group (just us) never touches the
    /// relay; a delta made on a dead link stays in the outbox and flushes on the
    /// next `sync`, so joiners converge on prior history.
    ///
    /// It is also THE DOOR the carriage ceiling is enforced at ([`refuse_if_uncarriable`]):
    /// a delta too big for one publish is refused here, before a byte is written.
    async fn append_and_flush(
        &self,
        group: &mut mls::Group,
        group_id: &[u8],
        delta: coordinator::Delta,
    ) -> Result<[u8; 32], CoreError> {
        let (delta_id, envelope) = self.append_local(group_id, delta)?;
        // LEAVES, not members: with one member and two leaves there IS somewhere
        // to send, and this is the gate that decided otherwise.
        if self.dir.group_leaf_count(group_id)? <= 1 {
            return Ok(delta_id);
        }
        // Route to the object's governing Arc's relay (its static IP), not a global
        // relay — an object flushes only through the Arc that governs it.
        if let Ok((mut sess, home)) = self.write_session(group_id).await {
            // A proposal is waiting: RFC 9420 §12.4 says commit before sending, and
            // mls-rs refuses to encrypt until someone has (§7). Commit it, then reload —
            // the caller's copy of the group is now an epoch behind, and writing it back
            // would overwrite the newer state.
            if mls::commit_required(group) {
                match self.commit_pending(&mut sess, group_id).await {
                    Ok(_) => {
                        let (sid, sk, _n) = self.signer()?;
                        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
                        *group = mls::load_group(&client, group_id)?;
                    }
                    Err(e) => tracing::warn!(
                        target: "pacific::membership",
                        group = %hex::encode(group_id),
                        error = %e,
                        "could not commit the pending proposals; the delta waits in the outbox"
                    ),
                }
            }
            if let Err(e) = self
                .flush_one(&mut sess, group, group_id, &delta_id, &envelope)
                .await
            {
                // STILL A WARN, AND STILL NOT THE CALLER'S BUSINESS: the outbox's
                // "never lost" property is exactly that an author offline, or on a
                // dead link, gets `Ok` and the delta goes on the next sync. What it
                // CANNOT yet say is whether this failure is temporary. Every publish
                // failure — a relay refusing the blob and a socket that never opened
                // alike — arrives here as one `CoreError::Transport` carrying a joined
                // message (`Router::publish`), so permanent and transient are
                // indistinguishable at this point without threading the relay's
                // `Ack.reason` up through every transport. The SIZE is logged so an
                // operator reading this line can at least tell the two apart by eye;
                // making core tell them apart is its own change.
                tracing::warn!(
                    target: "pacific::sync",
                    group = %hex::encode(group_id),
                    delta = %hex::encode(&delta_id[..8]),
                    bytes = envelope.len(),
                    error = %e,
                    "delta stored but not yet sent; it stays in the outbox for the next sync"
                );
                sess.close().await;
            } else {
                self.done_with(sess, home).await;
            }
        }
        Ok(delta_id)
    }

    /// THE LOCAL COMMIT of an authored Delta: stored, signed, in the outbox, and nothing
    /// sent. What a following read sees, and what `publish_pending` or the next sync sends.
    fn append_local(&self, group_id: &[u8], delta: coordinator::Delta) -> Result<([u8; 32], Vec<u8>), CoreError> {
        // Nothing more is written into a group this device has left, or is leaving
        // (membership-through-mls.md §6.1 step 5, §8.3).
        self.refuse_if_departed(group_id)?;
        if self.dir.pending_departure(group_id)?.is_some() {
            return Err(CoreError::Membership(
                "you are leaving this group — nothing more can be written to it".into(),
            ));
        }
        let envelope = delta.canonical_bytes();
        // BEFORE THE APPEND, and that is the whole point: a delta the relay will
        // refuse must leave no local state and no outbox entry behind it.
        refuse_if_uncarriable(&delta, envelope.len())?;
        let delta_id = delta.id();
        self.dir.append_delta_signed(
            group_id,
            &delta_id,
            &self.id.identity_pk(),
            &envelope,
            Some(&crate::delta_sig::sign_delta(&self.id, group_id, &delta_id)),
            unix_now(),
        )?;
        Ok((delta_id, envelope))
    }

    /// Encrypt + seal + PUB one queued Delta to the shared group tag, marking it
    /// delivered on the relay Ack. Shared by the post path and the outbox resend.
    async fn flush_one(
        &self,
        sess: &mut Router,
        group: &mut mls::Group,
        group_id: &[u8],
        delta_id: &[u8; 32],
        envelope: &[u8],
    ) -> Result<(), CoreError> {
        // THE ENVELOPE TRAVELS SIGNED (A-10 part 1, SEC-35): every Delta flushed here is
        // this device's own, so it goes with this identity's signature over it.
        let payload = crate::delta_sig::seal_payload(
            envelope,
            &self.id.identity_pk(),
            &crate::delta_sig::sign_delta(&self.id, group_id, delta_id),
        );
        let inner = mls::encrypt_delta(group, &payload)?;
        let epoch = mls::epoch_be(group.current_epoch());
        let conn_secret = mls::seal_conn_secret(group, &epoch)?;
        let dest = mls::group_tag(group, &epoch)?;
        let dest_addr = mls::group_address(group, &epoch)?;
        // Record this epoch's tag+secret so it can be re-drained later (the exporter
        // only yields the CURRENT epoch, so we must persist each one we hold).
        self.dir
            .record_epoch_tag(group_id, group.current_epoch(), &dest, &conn_secret)?;
        let sealed = seal::seal(&inner, &dest, &conn_secret)?;
        sess.publish(
            &dest_addr,
            &pacific_wire::blob_b64(&sealed),
        )
        .await?;
        self.dir.mark_delivered(group_id, delta_id)?;
        Ok(())
    }

    /// Drain one epoch's shared mailbox tag from its cursor: unseal, decrypt, and
    /// fold each message. Returns whether processing a commit advanced our epoch.
    /// Used both for the current tag (to follow epoch advances) and for every
    /// superseded epoch tag (to catch application messages that raced the fence).
    ///
    /// INGEST TAXONOMY (the field-consensus contract; case study §4.1): each
    /// blob is classified, and the cursor ALWAYS advances except on a retryable
    /// (local/db) failure —
    ///   applied     → folded/advanced as before;
    ///   quarantined → PERMANENTLY unprocessable (garbage bytes, undecryptable
    ///                 or replayed MLS message, undecodable delta): recorded in
    ///                 the bounded quarantine table, cursor advances, the drain
    ///                 CONTINUES. One poison blob can no longer wedge a mailbox.
    ///   retryable   → Directory/Io (this device's storage, not the blob):
    ///                 abort WITHOUT advancing, so nothing is skipped by a
    ///                 transient local fault.
    async fn drain_tag(
        &self,
        sess: &mut Router,
        group: &mut mls::Group,
        group_id: &[u8],
        tag: &[u8; 32],
        secret: &[u8; 32],
    ) -> Result<bool, CoreError> {
        // Each transport is drained from ITS OWN cursor and its results advance ITS OWN cursor:
        // a `seq` belongs to whichever transport issued it, so one shared position would have a
        // mesh delivery drag the relay's cursor forward and skip mail (or the reverse).
        // A TAG THE HELD CONNECTION VOUCHES FOR is not drained (O-69): everything the relay
        // holds past its cursor has been delivered and taken, and nothing on it waits. Only
        // with nothing queued at all: ingesting here would move the group under the caller.
        if self.live_vouches(group_id, tag)? {
            return Ok(false);
        }
        let dir = &self.dir;
        self.live_state().tag_drains += 1;
        // What the held connection had confirmed before this drain began: complete after it.
        let confirmed = self.live_since(&pacific_wire::tag_hex(tag));
        let msgs = sess
            .drain(&[pacific_wire::tag_hex(tag)], |source| {
                dir.cursor(tag, source)
            })
            .await?;
        let advanced = self.ingest_drained(group, group_id, tag, secret, msgs)?;
        self.live_drained(pacific_wire::tag_hex(tag), confirmed);
        Ok(advanced)
    }

    /// One tag's drained messages, in the order the relay gave them: each processed, or
    /// quarantined, and its cursor advanced, as `drain_tag`'s taxonomy says. Whether a commit
    /// advanced the epoch.
    fn ingest_drained(
        &self,
        group: &mut mls::Group,
        group_id: &[u8],
        tag: &[u8; 32],
        secret: &[u8; 32],
        msgs: Vec<crate::router::Drained>,
    ) -> Result<bool, CoreError> {
        let mut advanced = false;
        for m in msgs {
            match self.process_group_blob(group, group_id, tag, secret, &m.blob) {
                Ok(did_advance) => advanced |= did_advance,
                Err(e) if e.is_retryable() => return Err(e),
                Err(e) => {
                    tracing::warn!(seq = m.seq, source = m.source, error = %e,
                                   "quarantining unprocessable group blob");
                    self.dir.quarantine_put(
                        Some(group_id),
                        tag,
                        m.seq,
                        &e.to_string(),
                        m.blob.as_bytes(),
                        unix_now(),
                    )?;
                }
            }
            self.dir.advance_cursor(tag, m.source, m.seq)?;
            // A commit removed this device (§8.3): its group state is gone, and nothing
            // after the removing commit is readable by it. Stop here.
            if self.dir.is_departed(group_id)? {
                break;
            }
        }
        Ok(advanced)
    }

    /// Process ONE sealed blob off a group mailbox. Returns whether it advanced
    /// the epoch. Every failure here concerns the BLOB (permanent) except
    /// Directory/Io, which concern this device (retryable) — the split
    /// `drain_tag` keys its cursor discipline on.
    fn process_group_blob(
        &self,
        group: &mut mls::Group,
        group_id: &[u8],
        tag: &[u8; 32],
        secret: &[u8; 32],
        blob_b64: &str,
    ) -> Result<bool, CoreError> {
        let blob = pacific_wire::blob_unb64(blob_b64)
            .map_err(|e| CoreError::Seal(format!("base64: {e}")))?;
        let inner = seal::open(&blob, tag, secret)?;
        match mls::decrypt_message(group, &inner)? {
            mls::Incoming::Application { sender, data } => {
                // THE AUTHOR SIGNED IT, AND SENT IT (A-10; SEC-35, NC-45). A payload with
                // no signature, or one that does not prove its author, is refused here,
                // before anything is stored, and quarantined in those words. So is one
                // signed by anyone but the leaf MLS authenticated: that leaf is who the
                // group admitted, in the epoch the message was sent in (mls-rs refuses a
                // late message whose leaf has since been refilled, `group/util.rs`), and
                // it is what the commutative fold's "was a member" rests on. Otherwise
                // one member signs as many authors as it makes keys.
                let (envelope, author, sig) = crate::delta_sig::open_payload(&data)?;
                let delta = coordinator::decode_delta(&envelope)?;
                let did = delta.id();
                crate::delta_sig::verify_delta(group_id, &author, &did, &sig).map_err(|_| {
                    CoreError::Identity(format!(
                        "delta {} claims author {} and its signature does not prove it (A-10)",
                        hex::encode(&did[..6]),
                        hex::encode(&author[..6])
                    ))
                })?;
                if author != sender {
                    return Err(CoreError::Identity(format!(
                        "delta {} is signed by {}, a key the group did not admit to the leaf that sent it ({}) (A-10, NC-45)",
                        hex::encode(&did[..6]),
                        hex::encode(&author[..6]),
                        hex::encode(&sender[..6])
                    )));
                }
                self.dir
                    .append_delta_signed(group_id, &did, &author, &envelope, Some(&sig), unix_now())?;
                Ok(false)
            }
            mls::Incoming::Commit { .. } => {
                self.after_commit(group_id, group)?;
                Ok(true)
            }
            // A proposal does not move the epoch. §7 commits it after the drain.
            //
            // §6.4 — another device of THIS person proposed removing one of our leaves:
            // the person is leaving, and this device goes too. Adopted here, as the
            // proposal is drained, and not after the drain: a commit later in the same
            // drain can lapse the proposal and clear the cache, and a device that looked
            // afterwards would never learn its person had left. A Remove anyone else
            // proposed is not a leave, and is never adopted (`mls::pending_leave_of`).
            mls::Incoming::Proposal { sender, removes } => {
                let me = self.id.identity_pk();
                if let Some(leaf) = removes {
                    if sender == me
                        && mls::leaves_of(group, &me).contains(&leaf)
                        && self.dir.pending_departure(group_id)?.is_none()
                    {
                        self.dir
                            .set_pending_departure(group_id, group.current_epoch(), unix_now())?;
                        tracing::info!(target: "pacific::membership", group = %hex::encode(group_id),
                                       epoch = group.current_epoch(), leaf,
                                       "another device of this person proposed leaving; this one goes with it");
                    }
                }
                Ok(false)
            }
            mls::Incoming::Removed { by } => {
                // THE SPINE RECORDS THE DEPARTURE FIRST. The group is still at the epoch
                // this device was removed from (mls-rs skips a removed member's key
                // schedule), so that is the last epoch the account could read. And it
                // must be queued BEFORE `on_removed`: a Directory failure here is
                // retryable and re-processes this commit, while one after `on_removed`
                // would find the group departed and never come back to write it.
                //
                // EVERY removed device writes it, not the lowest leaf as a join does.
                // Leaving takes all of a person's leaves, and the lowest may be a phone
                // in a drawer that never processes this commit; a join's writer is by
                // definition online. The duplicates share one `last_epoch`.
                if !self.dir.is_departed(group_id)? {
                    self.note_on_spine(crate::spine::Body::Left(crate::spine::Departed {
                        group_id: group_id.to_vec(),
                        last_epoch: group.current_epoch(),
                    }))?;
                }
                self.on_removed(group_id, &by)?;
                Ok(false)
            }
            mls::Incoming::SkippedOwn => {
                self.skipped_own.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                // THE IN-BAND STALE CHECK (srr/security.md step 7): the leaf's own
                // message, which this state never sent, was sent by a later copy of it.
                if !self.dir.was_sent(blob_b64)? {
                    self.own_unsent.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    tracing::warn!(target: "pacific::sync", group = %hex::encode(group_id),
                                   "a message from this device's own leaf that this state never sent: \
                                    another copy of this device has spoken");
                    return Ok(false);
                }
                tracing::debug!(target: "pacific::sync", group = %hex::encode(group_id),
                                "own message skipped: MLS does not return a sender its own");
                Ok(false)
            }
            mls::Incoming::StaleHandshake => {
                self.skipped_own.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(false)
            }
        }
    }

    /// How many of this device's own messages drains have handed back (NC-33).
    pub fn skipped_own(&self) -> u64 {
        self.skipped_own.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// RECEIVE, AND SAY NOTHING (D-34 (c); srr/security.md step 7). Every held object's
    /// current tag drained to its head, and every retained epoch's tag, with no upkeep,
    /// no flush and no receipt: this state publishes nothing. What it counts is the
    /// in-band stale check. A message from this device's own leaf that this state never
    /// published means another copy of it has spoken since this one was sealed, and a
    /// state that goes on from here would send again from ratchet positions already
    /// used. Run on a state just opened, before anything else it does.
    ///
    /// A relay that cannot be reached is `Err`: the check did not run.
    pub async fn drain_only(&self) -> Result<crate::devstate::Drained, CoreError> {
        let before = self.own_unsent.load(std::sync::atomic::Ordering::Relaxed);
        let (sid, sk, _name) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        // Every group this leaf speaks in, by the relay that carries it.
        let mut by_arc: std::collections::BTreeMap<String, Vec<(Vec<u8>, mls::Group)>> = Default::default();
        for (group_id, _kind) in self.dir.all_groups()? {
            // Departed, or alone in it: nothing of this leaf's is on the relay.
            if self.dir.is_departed(&group_id)? || self.dir.group_leaf_count(&group_id)? <= 1 {
                continue;
            }
            // As `sync_group`: a group whose state will not load is one this leaf
            // cannot speak in either.
            let Ok(group) = mls::load_group(&client, &group_id) else { continue };
            by_arc.entry(self.routes_for(&group_id).to_spec()).or_default().push((group_id, group));
        }
        let mut groups = 0;
        let home = self.routes.to_spec();
        for (spec, mut held) in by_arc {
            // The home relay's on the held session, kept for what the open does next.
            let mut sess = match spec == home {
                true => self.home_session().await?,
                false => Router::open(&crate::router::Routes::parse(&spec)).await?,
            };
            // A held connection dialled beside this one is confirmed before the drains
            // begin, so each leaves its tag complete.
            if let Some(l) = self.live() {
                l.wait_confirmed(LIVE_CONFIRM);
            }
            let drained = self.drain_in_rounds(&mut sess, &mut held).await;
            match (&drained, spec == home) {
                (Ok(()), true) => self.keep_session(sess),
                _ => sess.close().await,
            }
            drained?;
            groups += held.len();
        }
        let own_unsent = self.own_unsent.load(std::sync::atomic::Ordering::Relaxed) - before;
        Ok(crate::devstate::Drained { own_unsent, groups })
    }

    /// SYNC STEPS (a) AND (b) FOR MANY GROUPS AT ONCE, in rounds of one pipelined drain each
    /// (`Router::drain_many`): the first round asks every group's current epoch tag (recorded
    /// as it is reached) and every retained epoch's; a group a commit moved asks its next
    /// epoch's tag in the next. Each tag from its own cursor, each group's current tag
    /// processed before its older ones, as `drain_group_tags` does one at a time.
    async fn drain_in_rounds(&self, sess: &mut Router, held: &mut [(Vec<u8>, mls::Group)]) -> Result<(), CoreError> {
        let mut owed: Vec<usize> = (0..held.len()).collect();
        let mut first = true;
        // A bound, not a guess at how many commits can land at once: each round follows at
        // least one epoch, and a group that is still moving is followed by the next pass.
        for _round in 0..16 {
            if owed.is_empty() {
                break;
            }
            let mut asks: Vec<(usize, [u8; 32], zeroize::Zeroizing<[u8; 32]>)> = Vec::new();
            for &i in &owed {
                let (group_id, group) = &held[i];
                let e = group.current_epoch();
                let eb = mls::epoch_be(e);
                let tag = mls::group_tag(group, &eb)?;
                let secret = mls::seal_conn_secret(group, &eb)?;
                self.dir.record_epoch_tag(group_id, e, &tag, &secret)?;
                asks.push((i, tag, zeroize::Zeroizing::new(secret)));
                if first {
                    for (_ep, t, sec) in self.dir.epoch_tags(group_id)? {
                        if t != tag {
                            asks.push((i, t, sec));
                        }
                    }
                }
            }
            first = false;
            let hexes: Vec<String> = asks.iter().map(|(_, t, _)| pacific_wire::tag_hex(t)).collect();
            let confirmed: Vec<Option<u64>> = hexes.iter().map(|h| self.live_since(h)).collect();
            let tags: std::collections::HashMap<String, [u8; 32]> = asks.iter().map(|(_, t, _)| (pacific_wire::tag_hex(t), *t)).collect();
            let dir = &self.dir;
            self.live_state().tag_drains += hexes.len() as u64;
            let per_tag = sess.drain_many(&hexes, |hex, source| dir.cursor(&tags[hex], source)).await?;
            let mut moved = Vec::new();
            for (((i, tag, secret), msgs), (hex, since)) in asks.iter().zip(per_tag).zip(hexes.into_iter().zip(confirmed)) {
                let (group_id, group) = &mut held[*i];
                if self.dir.is_departed(group_id)? {
                    continue;
                }
                if self.ingest_drained(group, group_id, tag, secret, msgs)? && !moved.contains(i) {
                    moved.push(*i);
                }
                self.live_drained(hex, since);
            }
            owed = moved;
        }
        Ok(())
    }

    /// Messages from this device's own leaf that this state never sent, seen by any drain
    /// since this Node opened, `sync_once`'s included. Above 0 in a live process, a second
    /// copy of this device is speaking beside it.
    pub fn own_unsent(&self) -> u64 {
        self.own_unsent.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// How many publishes this process has attempted, commits and upkeep included: a
    /// count that has moved since a seal says the seal is behind (D-34 (c)). One Node
    /// per process is the Door's shape, and the count is the process's.
    pub fn published(&self) -> u64 {
        crate::router::published()
    }

    /// Everything a device records after ANY commit it applies — received or its own
    /// winning one (membership-through-mls.md §8.2): the roster projections, and the
    /// owner, when the commit moved it (a handover, §9, or the migration, §3.3).
    fn after_commit(&self, group_id: &[u8], group: &mls::Group) -> Result<(), CoreError> {
        let roster = mls::roster_identities(group)?;
        self.dir.set_group_members(group_id, &roster)?;
        let owner = mls::group_owner(group)?;
        let cached = self.dir.group_owner(group_id)?;
        if cached.as_deref() != Some(&owner[..]) {
            tracing::info!(
                target: "pacific::membership",
                group = %hex::encode(group_id),
                epoch = group.current_epoch(),
                from = %cached.as_deref().map(hex::encode).unwrap_or_default(),
                to = %hex::encode(owner),
                "the owner moved"
            );
        }
        // Idempotent; also backfills a history row for the current owner of a group
        // made before the history existed, at the epoch this device first saw it.
        let history = self.dir.owner_history(group_id)?;
        if history.last().map(|(_, o)| *o) != Some(owner) {
            self.dir.record_owner(group_id, group.current_epoch(), &owner)?;
        }
        tracing::debug!(
            target: "pacific::membership",
            group = %hex::encode(group_id),
            epoch = group.current_epoch(),
            leaves = roster.len(),
            "roster refreshed after commit"
        );
        // THE HELD CONNECTION FOLLOWS THE EPOCH, WHOEVER COMMITTED (O-69): the new epoch's
        // tag recorded and listened on now. This device's own commit (an owner adding the
        // Arc's node) otherwise left it unheard, and what others said there (the Arc's
        // admissions, members' posts) waited for the next pass or this device's next write.
        let e = group.current_epoch();
        let eb = mls::epoch_be(e);
        self.dir.record_epoch_tag(group_id, e, &mls::group_tag(group, &eb)?, &mls::seal_conn_secret(group, &eb)?)?;
        self.live_retag();
        Ok(())
    }

    /// A commit removed THIS device (§8.3). Record who did it, delete every secret this
    /// device held for the group — mls-rs's snapshot and epoch records, the epoch tags
    /// and their seal secrets — and stop: sync, the doors and rekey all skip a departed
    /// group from now on. The delta log stays: it is history this person received.
    fn on_removed(&self, group_id: &[u8], by: &[u8; 32]) -> Result<(), CoreError> {
        let deleted = crate::mls_store::SqliteGroupStateStorage::open(&paths::db_path())
            .and_then(|s| s.delete_group(group_id))
            .map_err(|e| CoreError::Directory(format!("delete MLS state: {e}")))?;
        self.dir.mark_departed(group_id, by, unix_now())?;
        tracing::warn!(
            target: "pacific::membership",
            group = %hex::encode(group_id),
            by = %hex::encode(by),
            left_ourselves = (*by == self.id.identity_pk()),
            mls_rows_deleted = deleted,
            "removed from the group — its MLS state is deleted and it will not be synced again"
        );
        Ok(())
    }

    // ======================================================================
    // OBJECTS — every object is a GROUP OF 1 until others collaborate.
    //
    // Creating an object mints a fresh MLS group containing only us; we author
    // Deltas into its log and fold them to state, entirely on-device (no relay —
    // there is no one to sync with). Adding a member later (the N-member path)
    // turns the SAME object into a shared group; nothing else changes.
    // ======================================================================

    /// Create a new object of `kind` as a group of 1 (owner = us). Returns the
    /// object id (hex of the MLS group_id).
    pub fn object_new(&self, kind: &str, name: &str) -> Result<String, CoreError> {
        let (sid, sk, _name) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        // The DISPLAY NAME (if any) rides the GroupContext extension, so every member
        // — including late joiners, via their Welcome — reads the same current name.
        // A solo MLS group — just us — with US as its owner in the GroupContext (§3).
        let group = mls::create_group_named(&client, name)?;
        let group_id = group.group_id().to_vec();
        // The creator determines the object's governing Arc: stamp OUR default.
        self.dir.put_group(
            &group_id,
            kind,
            &self.id.identity_pk(),
            Some(&self.my_default_arc()),
        )?;
        self.dir
            .set_group_members(&group_id, &mls::roster_identities(&group)?)?;
        // The owner from epoch 0, and this leaf's freshness (§10.2, §11).
        self.dir.record_owner(&group_id, 0, &self.id.identity_pk())?;
        self.dir.record_self_update(&group_id, unix_now(), group.current_epoch())?;
        // The founder's tenure (§10.3): a Group-typed object records who is in it from
        // its first delta, written here by core rather than by each app.
        // Not for a NOTEBOOK: it is a person's private group of one ("plumbing", as the
        // directory calls it), with nobody else ever to be in it.
        if group::is_group_typed(kind) && kind != "notebook" {
            self.record_founder(&group_id, group.current_epoch())?;
        }
        // THE SPINE ENTRY, minted in the same act as the object.
        //
        // Without it a device holding only the 24 words cannot NAME this object,
        // and every archive address is derived from the group id, so there would
        // be nothing to derive from. The entry is produced by `spine::place` —
        // the same core import every platform uses, so a spine written here and
        // read in a browser is the ordinary case rather than a format to agree on.
        //
        // QUEUED, NOT PUBLISHED. This function is synchronous and the relay is
        // not; the row is the durable record that the entry exists and is owed to
        // the account's address, and `sync` carries it. A mint that cannot queue
        // is a mint that would come back unnameable, so it fails rather than
        // returning an object it cannot promise.
        self.queue_spine_entry_for(&group_id, group.current_epoch())?;

        tracing::info!(
            target: "pacific::membership",
            object = %hex::encode(&group_id),
            kind,
            "object created, owned by its creator"
        );
        Ok(hex::encode(group_id))
    }

    /// Mint this account's spine entry for `group_id`, joined at `first_epoch`.
    ///
    /// The index is claimed by the table's PRIMARY KEY, so two sessions of one
    /// account cannot both take it. A seedless identity — minted before seeds
    /// existed — has no storage root and therefore no spine; that is reported, not
    /// papered over, because such an account cannot be recovered from words at all
    /// and pretending otherwise is the lie this whole path exists to stop telling.
    fn queue_spine_entry_for(&self, group_id: &[u8], first_epoch: u64) -> Result<(), CoreError> {
        self.queue_spine_body(crate::spine::Body::Group(crate::spine::Joined {
            group_id: group_id.to_vec(),
            first_epoch,
        }))
    }

    /// Seal one spine entry and queue it at the next index. The one writer every
    /// door goes through, so the format and the address are the shared import's.
    fn queue_spine_body(&self, body: crate::spine::Body) -> Result<(), CoreError> {
        let seed = self.seed_or_refuse()?;
        let root = crate::locator::storage_root(&seed);
        // Generation 0 until a restart needs another; the head names the live one.
        let gen = SPINE_GEN;
        let index = self.dir.next_spine_index(gen)?;
        let (address, blob) = crate::spine::place(&root, gen, index, body)?;
        self.dir.queue_spine_entry(gen, index, &address, &blob)
    }

    /// Record a membership change on the spine, for the doors that are not the mint:
    /// joining through a Welcome, making a connection, leaving. `Ok(false)` when the
    /// identity has no seed.
    ///
    /// WHY A SEEDLESS IDENTITY IS SKIPPED HERE AND REFUSED AT THE MINT. A legacy
    /// identity has no storage root and so no spine, and `object_new` refuses to
    /// mint for it rather than pretend. Refusing HERE would cost more than the lie
    /// it prevents: a join that fails is a Welcome quarantined for ever, and a
    /// departure that fails is a removal re-processed without end. The object is
    /// simply unnameable from words — which `recoverability` reports — and the
    /// membership change itself stands.
    fn note_on_spine(&self, body: crate::spine::Body) -> Result<bool, CoreError> {
        if self.id.seed_bytes().is_none() {
            tracing::warn!(
                target: "pacific::spine",
                group = %hex::encode(body.group_id()),
                "a seedless identity has no spine; this membership change is not recorded on it"
            );
            return Ok(false);
        }
        self.queue_spine_body(body)?;
        Ok(true)
    }

    /// The spine entry for a group this device has just joined through a Welcome.
    ///
    /// ONE ENTRY PER ACCOUNT, NOT PER DEVICE. A person's devices share one seed and
    /// so one spine, and a second device being added to a group the account already
    /// belongs to is not the account joining. So the entry is written by the
    /// person's LOWEST leaf in the roster: alone, that is this device; beside an
    /// existing device of the same person, it is that device, which wrote the
    /// entry when it joined. (Joining into a vacated lower slot writes a second
    /// entry with a later floor; readers keep the lowest, so it is harmless.)
    ///
    /// NOT FATAL. The group is already saved: a failure here that made the Welcome
    /// fail would have it re-processed against a consumed key package and
    /// quarantined, losing a join that happened. So it is reported, and
    /// `recoverability` says the object cannot be named.
    fn spine_on_join(&self, group: &mls::Group, group_id: &[u8]) {
        let me = self.id.identity_pk();
        let lowest_of_mine = mls::leaves_of(group, &me).into_iter().min();
        if lowest_of_mine != Some(mls::my_leaf(group)) {
            return;
        }
        let body = crate::spine::Body::Group(crate::spine::Joined {
            group_id: group_id.to_vec(),
            first_epoch: group.current_epoch(),
        });
        if let Err(e) = self.note_on_spine(body) {
            tracing::warn!(
                target: "pacific::spine",
                group = %hex::encode(group_id),
                error = %e,
                "joined, but the spine entry could not be queued — this object cannot be \
                 named from the seed"
            );
        }
    }

    /// ONE OBJECT, AS IT FOLDS — kind, roster, owner and the lens's own view, from
    /// this device's directory. The read every surface wanted the archive for, and
    /// the reason the archive was being shipped whole to a browser that only needed
    /// a list of names.
    ///
    /// The view is [`crate::fold::view_of`], which is the same renderer the wasm
    /// build serves, so a phone and a browser cannot disagree about what an object
    /// says. A kind with no lens is reported in those words rather than folded to
    /// nothing.
    pub fn object_view(&self, object_id_hex: &str) -> Result<String, CoreError> {
        let id = hex::decode(object_id_hex)
            .map_err(|e| CoreError::Coordinator(format!("object id: {e}")))?;
        let kind = self
            .dir
            .group_kind(&id)?
            .ok_or_else(|| CoreError::Coordinator("no such object".into()))?;
        // THE VIEW IS A FOLD'S PRODUCT, cached with it (O-69): unscoped, the same for every
        // session of this process, and filtered by each caller after, as a fresh one is.
        // Its inputs are read inside, after the key.
        let view = self.dir.cached(&id, &kind, "view", |a: &String, b| a == b, || {
            let members = self.dir.group_members(&id)?;
            let owners = self.dir.owner_history(&id)?;
            let owner = self
                .object_owner(object_id_hex)?
                .and_then(|v| <[u8; 32]>::try_from(v.as_slice()).ok())
                .unwrap_or([0u8; 32]);
            // The rows' own refusal (a signature that does not prove its author) is a
            // fold's failure too, and is kept as one.
            crate::fold::view_of(&kind, owner, members, owners, self.dir.load_log(&id)?, Some(&self.id.identity_pk()))
        })?;
        // WHAT IT STATES AGAINST THE READER'S CLOCK, applied after the cache, never frozen in it
        // (TB4): a Thing's hold past its until, a deal's silence past its confirmWithin.
        let view = crate::fold::at_read(&kind, view, unix_millis() as i64);
        if kind != "event" {
            return Ok(view);
        }
        // AN ACT IS CONFIRMED TO THE READER WHO HOLDS THE PERFORMER'S HALF (the ICD, performs_at).
        // The half is another object's, so it is this reader's join, made after the cached fold
        // of the event alone: a half written later is read at once, with no stale cache.
        let mut v: serde_json::Value = serde_json::from_str(&view).map_err(|e| CoreError::Coordinator(e.to_string()))?;
        for act in v["acts"].as_array_mut().into_iter().flatten() {
            let key = |k: &str| act[k].as_str().and_then(|h| hex::decode(h).ok()).and_then(|b| <[u8; 32]>::try_from(b).ok());
            let performer = match (key("member"), key("object")) {
                (Some(m), _) => crate::event::Performer::Member(m),
                (None, Some(g)) => crate::event::Performer::Object(g),
                (None, None) => continue,
            };
            act["confirmed"] = self.holds_performs_at(object_id_hex, &performer).into();
        }
        Ok(v.to_string())
    }

    /// PERFORMS_AT, from this device: does it hold `performer`'s own half toward `event_hex`,
    /// group.setAffiliation {rel: performs_at, peer: the event}? An organisation's is on its
    /// group, where this device holds it; a person's on their self record, which only they
    /// hold, so another person's act is never confirmed here. The one reading of the half, for
    /// the event's view and for the Face (host_sync_items).
    fn holds_performs_at(&self, event_hex: &str, performer: &crate::event::Performer) -> bool {
        let record = match performer {
            crate::event::Performer::Object(g) => Some(hex::encode(g)),
            crate::event::Performer::Member(m) if *m == self.id.identity_pk() => self.self_object().ok().flatten(),
            crate::event::Performer::Member(_) => None,
        };
        record.filter(|r| self.object_kind(r).is_ok_and(|k| k == "group")).is_some_and(|r| {
            self.object_group_id(&r).and_then(|g| self.folded_group(&g)).is_ok_and(|c| {
                c.state().affiliations.get(event_hex).is_some_and(|a| a.rel == crate::group::AffiliationRel::PerformsAt)
            })
        })
    }

    /// Every spine entry this device holds for its account, published or not,
    /// opened, in index order.
    pub fn spine_entries(&self) -> Result<Vec<(u64, crate::spine::Entry)>, CoreError> {
        let seed = self.seed_or_refuse()?;
        let key = crate::spine::entry_key(&crate::locator::storage_root(&seed));
        let mut out = Vec::new();
        for (index, blob) in self.dir.delivered_spine(SPINE_GEN)? {
            out.push((index, crate::spine::open_entry(&blob, &key, index)?));
        }
        for (index, _addr, blob) in self.dir.pending_spine(SPINE_GEN)? {
            out.push((index, crate::spine::open_entry(&blob, &key, index)?));
        }
        out.sort_by_key(|(i, _)| *i);
        Ok(out)
    }

    /// Spine entries minted and not yet published, opened.
    ///
    /// For a surface that wants to show what this device still owes its own
    /// account, and for the tests that pin the writer. Opening them here rather
    /// than handing out sealed bytes keeps the key in one place.
    pub fn pending_spine_entries(&self) -> Result<Vec<(u64, crate::spine::Entry)>, CoreError> {
        let seed = self.seed_or_refuse()?;
        let key = crate::spine::entry_key(&crate::locator::storage_root(&seed));
        let mut out = Vec::new();
        for (index, _addr, blob) in self.dir.pending_spine(SPINE_GEN)? {
            out.push((index, crate::spine::open_entry(&blob, &key, index)?));
        }
        Ok(out)
    }

    /// Publish this account's queued spine entries. Returns how many went out.
    ///
    /// THE STEP THAT MAKES A MINTED OBJECT NAMEABLE. `object_new` seals an entry
    /// and queues it; until it is published it lives in `pacific.db`, which is
    /// precisely what a recovering device does not have. This is the move from
    /// "this device knows" to "the seed can find it", and it is why
    /// [`Node::recoverability`] reads the DELIVERED rows rather than the queued
    /// ones.
    ///
    /// WHERE THE BYTES GO. `spine::place` already computed the address —
    /// `chain_locator(storage_root, gen, index)`, arithmetic, so entry `i` is
    /// reachable without walking `i-1`. The relay takes an opaque 32-byte tag and
    /// keeps it: `Retention::default()` is no age window and no per-tag cap,
    /// ruled 13 September 2026 ("a phone left off for a week comes back to all of
    /// its mail"). THE SPINE DEPENDS ON THAT RULING. A deploy that sets
    /// `window_us` would expire the entries an account recovers from, and the
    /// failure would be silent and total — the account would simply come back
    /// holding nothing. If a retention window is ever wanted, the spine needs its
    /// own durable store first.
    ///
    /// ORDER AND GAPS. Entries go in index order, and a failure stops the run
    /// rather than skipping ahead: the next `sync_once` retries from the same
    /// place, and what was already marked delivered stays delivered. A gap is
    /// survivable by construction — that is what the arithmetic addressing bought
    /// — but it is not something to CAUSE on purpose while the relay is refusing.
    pub async fn drain_spine(&self) -> Result<usize, CoreError> {
        let pending = self.dir.pending_spine(SPINE_GEN)?;
        if pending.is_empty() {
            return Ok(0);
        }
        let mut sess = Router::open(&self.routes).await?;
        let sent = self.drain_spine_over(&mut sess).await;
        sess.close().await;
        sent
    }

    /// The drain over a session the caller already holds — `sync_once` publishes
    /// through the device relay it has open rather than dialling a second time.
    ///
    /// EACH ENTRY TAKES ITS INDEX'S SLOT (resumption.md A1): first writer wins, so two
    /// devices of one person can never both hold index `i`. A lost slot means another
    /// device appended there: walk to the end, and re-place this entry and everything
    /// queued after it from there.
    async fn drain_spine_over(&self, sess: &mut Router) -> Result<usize, CoreError> {
        let mut sent = 0usize;
        let mut rounds = 0;
        'outer: loop {
            for (index, address, blob) in self.dir.pending_spine(SPINE_GEN)? {
                let won = sess
                    .publish_commit(&Address::from_seed(&address), &pacific_wire::blob_b64(&blob))
                    .await?;
                if won.is_none() {
                    rounds += 1;
                    if rounds > 8 {
                        return Err(CoreError::CommitRace(format!(
                            "spine index {index}: lost the slot eight times"
                        )));
                    }
                    self.replace_pending_spine_from(sess, index).await?;
                    continue 'outer;
                }
                // Marked only after the relay acked it. The reverse order would let a
                // failed publish count as recoverable, which is the one lie this
                // whole path exists to stop telling.
                self.dir.mark_spine_delivered(SPINE_GEN, index)?;
                sent += 1;
                tracing::info!(target: "pacific::spine", gen = SPINE_GEN, index, "spine entry published");
            }
            return Ok(sent);
        }
    }

    /// Another device won index `from`: learn the chain's end from the relay, then
    /// re-seal every queued entry from `from` onward at the indices after it.
    async fn replace_pending_spine_from(&self, sess: &mut Router, from: u64) -> Result<(), CoreError> {
        let root = crate::locator::storage_root(&*self.seed_or_refuse()?);
        let key = crate::spine::entry_key(&root);
        let queued = self.dir.take_pending_spine_from(SPINE_GEN, from)?;
        let walk = self.chain_walk_over(sess, SPINE_GEN).await?;
        let mut next = walk.position.max(self.dir.next_spine_index(SPINE_GEN)?);
        for (index, blob) in &queued {
            let body = crate::spine::open_entry(blob, &key, *index)?.body;
            let (address, blob) = crate::spine::place(&root, SPINE_GEN, next, body)?;
            self.dir.queue_spine_entry(SPINE_GEN, next, &address, &blob)?;
            next += 1;
        }
        Ok(())
    }

    /// Read generation `gen` of this account's chain from the relay: index 0, 1, 2 …
    /// until the first empty one (resumption.md A1). Every entry found is recorded
    /// locally as delivered — it is this account's, whichever device wrote it.
    pub async fn chain_walk(&self, gen: u32) -> Result<crate::spine::Walk, CoreError> {
        let mut sess = Router::open(&self.routes).await?;
        let out = self.chain_walk_over(&mut sess, gen).await;
        sess.close().await;
        out
    }

    async fn chain_walk_over(&self, sess: &mut Router, gen: u32) -> Result<crate::spine::Walk, CoreError> {
        const BATCH: u64 = 16;
        let root = crate::locator::storage_root(&*self.seed_or_refuse()?);
        let key = crate::spine::entry_key(&root);
        let mut entries = Vec::new();
        let mut tail = crate::head::NO_TAIL;
        let mut index = 0u64;
        loop {
            let addrs: Vec<[u8; 32]> =
                (index..index + BATCH).map(|i| crate::locator::chain_locator(&root, gen, i)).collect();
            let tags: Vec<String> = addrs.iter().map(|a| Address::from_seed(a).tag_hex()).collect();
            let mut got = sess.drain(&tags, |_| Ok(0)).await?;
            got.sort_by_key(|m| m.seq);
            let mut ended = false;
            for (k, tag) in tags.iter().enumerate() {
                let i = index + k as u64;
                // The earliest blob at this index that opens AS this index: the slot's
                // winner. Anything else at the tag is noise, not the account's.
                let found = got.iter().filter(|m| &m.tag == tag).find_map(|m| {
                    let blob = pacific_wire::blob_unb64(&m.blob).ok()?;
                    let e = crate::spine::open_entry(&blob, &key, i).ok()?;
                    Some((e, blob))
                });
                match found {
                    Some((e, blob)) => {
                        tail = crate::spine::tail_hash(&blob);
                        if gen == SPINE_GEN {
                            self.dir.record_walked_spine_entry(gen, i, &addrs[k], &blob)?;
                        }
                        entries.push((i, e));
                    }
                    None => {
                        ended = true;
                        break;
                    }
                }
            }
            if ended {
                break;
            }
            index += BATCH;
        }
        let position = entries.len() as u64;
        Ok(crate::spine::Walk { entries, position, tail })
    }

    /// Append `bodies` to generation 0, each at the next free index through its slot,
    /// and return the head for the platform to store (resumption.md §3.4).
    pub async fn chain_append(
        &self,
        bodies: Vec<crate::spine::Body>,
    ) -> Result<crate::spine::HeadUpdate, CoreError> {
        let root = crate::locator::storage_root(&*self.seed_or_refuse()?);
        let mut sess = Router::open(&self.routes).await?;
        let out = async {
            // What is already queued goes first, so the order on the relay is the
            // order things happened.
            self.drain_spine_over(&mut sess).await?;
            for body in bodies {
                let mut rounds = 0;
                loop {
                    let walk = self.chain_walk_over(&mut sess, SPINE_GEN).await?;
                    let next = walk.position.max(self.dir.next_spine_index(SPINE_GEN)?);
                    let (address, blob) = crate::spine::place(&root, SPINE_GEN, next, body.clone())?;
                    let won = sess
                        .publish_commit(&Address::from_seed(&address), &pacific_wire::blob_b64(&blob))
                        .await?;
                    if won.is_some() {
                        self.dir.record_walked_spine_entry(SPINE_GEN, next, &address, &blob)?;
                        break;
                    }
                    rounds += 1;
                    if rounds >= 8 {
                        return Err(CoreError::CommitRace("chain append: lost the slot eight times".into()));
                    }
                }
            }
            Ok::<(), CoreError>(())
        }
        .await;
        sess.close().await;
        out?;
        self.head_update()
    }

    /// The head for the chain as this device last walked or wrote it, sealed for the
    /// account record. Position is the count of entries from index 0 with no hole;
    /// the tail is the hash of the last of them.
    pub fn head_update(&self) -> Result<crate::spine::HeadUpdate, CoreError> {
        let root = crate::locator::storage_root(&*self.seed_or_refuse()?);
        let mut position = 0u64;
        let mut tail = crate::head::NO_TAIL;
        for (index, blob) in self.dir.delivered_spine(SPINE_GEN)? {
            if index != position {
                break;
            }
            tail = crate::spine::tail_hash(&blob);
            position += 1;
        }
        let head = crate::head::Head::new(self.routes.to_spec(), position, tail, SPINE_GEN);
        let sealed = crate::head::seal(&head, &root)?;
        Ok(crate::spine::HeadUpdate { position, tail, sealed })
    }

    /// Whether a sealed head, as the auth service holds it, names the chain this
    /// device holds: the same position and the same tail. Bytes cannot say: every
    /// seal takes a fresh nonce, so two devices sealing one head make different
    /// blobs, and the service calls the second a fork (NC-28).
    pub fn head_is_mine(&self, blob: &[u8], declared_position: u64) -> Result<bool, CoreError> {
        let root = crate::locator::storage_root(&*self.seed_or_refuse()?);
        let theirs = crate::head::open_declared(blob, &root, declared_position)?;
        let mine = self.head_update()?;
        Ok(theirs.position == mine.position && theirs.tail.as_slice() == mine.tail.as_slice())
    }

    // ── pool leaves (resumption.md §5) ─────────────────────────────────────────

    /// Provision one pool leaf into `object` (§5.1): a fresh signing key under the
    /// PERSON's credential, one KeyPackage whose private halves are captured, and an
    /// Add of it through the epoch's slot — one epoch. Any member may add a leaf of
    /// their own credential, and no `memberJoined` is written: the person is already
    /// present. The way-in goes on the spine as a `Pool` entry listing every idle leaf
    /// the spine already named for the object, plus this one.
    pub async fn pool_provision(&self, object: &str) -> Result<crate::spine::PoolLeaf, CoreError> {
        let gid = hex::decode(object).map_err(|e| CoreError::Directory(format!("object id: {e}")))?;
        let mut sess = Router::open(&self.routes).await?;
        let out = self.pool_provision_over(&mut sess, &gid).await;
        sess.close().await;
        out
    }

    async fn pool_provision_over(
        &self,
        sess: &mut Router,
        gid: &[u8],
    ) -> Result<crate::spine::PoolLeaf, CoreError> {
        let gid = gid.to_vec();
        let object = hex::encode(&gid);
        let me = self.id.identity_pk();
        let crypto = mls::crypto();
        let (sk, pk) = mls::generate_signing_key(&crypto)?;
        let pool: [u8; 32] = pk
            .as_bytes()
            .try_into()
            .map_err(|_| CoreError::Mls("a pool leaf's key is 32 bytes".into()))?;
        let gss = crate::mls_mem::MemGroupStateStorage::new();
        let kps = crate::mls_mem::MemKeyPackageStorage::new();
        let client = crate::mls_mem::build_client_mem(
            gss.clone(),
            kps.clone(),
            mls::signing_identity(&me, &pool),
            sk.clone(),
        )?;
        let kp = mls::make_key_package_bytes(&client)?;
        let snap = crate::mls_mem::Snapshot::capture(&gss, &kps)
            .map_err(|e| CoreError::Mls(format!("capture the pool leaf's key package: {e}")))?;
        let (kp_id, kp_data) = snap
            .key_packages
            .first()
            .cloned()
            .ok_or_else(|| CoreError::Mls("no key package was made".into()))?;

        let welcome = std::cell::RefCell::new(None::<Vec<u8>>);
        let won = self
            .commit_via_slot(sess, &gid, "pool leaf", 8, |g| {
                // Already in the tree: an earlier round's commit carried it.
                if g.roster().members_iter().any(|m| m.signing_identity.signature_key.as_bytes() == pool) {
                    return Ok(None);
                }
                let (commit, w) = mls::stage_add_member(g, &kp)?;
                *welcome.borrow_mut() = Some(w);
                Ok(Some(commit))
            })
            .await;
        if !won? {
            return Err(CoreError::Mls("the pool leaf's Add was not committed".into()));
        }
        let welcome = welcome
            .into_inner()
            .ok_or_else(|| CoreError::Mls("the Add produced no Welcome".into()))?;
        let (sid, msk, _n) = self.signer()?;
        let epoch = mls::load_group(&mls::build_client_sqlite(&paths::db_path(), sid, msk)?, &gid)?.current_epoch();

        let leaf = crate::spine::PoolLeaf {
            pool,
            welcome,
            sig_sk: sk.as_bytes().to_vec(),
            kp_id,
            kp_data,
            epoch,
        };
        // Only leaves still in the tree: an evicted one has been removed and is
        // never listed again (§6.3).
        let group = mls::load_group(&mls::build_client_sqlite(&paths::db_path(), self.signer()?.0, self.signer()?.1)?, &gid)?;
        let in_tree = pool_keys_in(&group);
        let mut pools: Vec<_> = self.spine_pools(&gid)?.into_iter().filter(|p| in_tree.contains(&p.pool)).collect();
        pools.push(leaf.clone());
        self.queue_spine_body(crate::spine::Body::Pool(crate::spine::Pool {
            group_id: gid.clone(),
            kind: self.dir.group_kind(&gid)?.unwrap_or_default(),
            arc: self.dir.group_arc(&gid)?.unwrap_or_default(),
            pools,
        }))?;
        tracing::info!(target: "pacific::resumption", group = %object, epoch, "pool leaf provisioned");
        let _ = &object;
        Ok(leaf)
    }

    /// The pool leaves the latest `Pool` entry this device knows names for `group`.
    fn spine_pools(&self, group_id: &[u8]) -> Result<Vec<crate::spine::PoolLeaf>, CoreError> {
        Ok(self
            .spine_entries()?
            .into_iter()
            .rev()
            .find_map(|(_, e)| match e.body {
                crate::spine::Body::Pool(p) if p.group_id == group_id => Some(p.pools),
                _ => None,
            })
            .unwrap_or_default())
    }

    // ── resumption (resumption.md §2, §4, §6.5, §9) ────────────────────────────

    /// The spine's group: the self record (A2), hex.
    pub fn spine(&self) -> Result<Option<String>, CoreError> {
        self.self_object()
    }

    /// Objects this device holds that the seed cannot yet get back into: no
    /// delivered `Pool` entry names a leaf for them. One sync that reaches a relay
    /// empties it (§12).
    pub fn spine_pending(&self) -> Vec<String> {
        let (Ok(delivered), Ok(groups)) = (self.delivered_pools(), self.held_groups()) else {
            return Vec::new();
        };
        groups
            .into_iter()
            .filter(|g| !delivered.contains_key(g))
            .map(hex::encode)
            .collect()
    }

    /// Every group this device holds and has not left.
    fn held_groups(&self) -> Result<Vec<Vec<u8>>, CoreError> {
        let mut out = Vec::new();
        for (gid, _kind) in self.dir.all_groups()? {
            if !self.dir.is_departed(&gid)? {
                out.push(gid);
            }
        }
        Ok(out)
    }

    /// The latest DELIVERED `Pool` entry per group.
    fn delivered_pools(&self) -> Result<std::collections::HashMap<Vec<u8>, crate::spine::Pool>, CoreError> {
        let key = crate::spine::entry_key(&crate::locator::storage_root(&*self.seed_or_refuse()?));
        let mut out = std::collections::HashMap::new();
        for (index, blob) in self.dir.delivered_spine(SPINE_GEN)? {
            if let Ok(e) = crate::spine::open_entry(&blob, &key, index) {
                if let crate::spine::Body::Pool(p) = e.body {
                    if !p.pools.is_empty() {
                        out.insert(p.group_id.clone(), p);
                    }
                }
            }
        }
        Ok(out)
    }

    /// Cells 1.. of several pool leaves in ONE drain per round — an upkeep over a
    /// person's objects reads every leaf's standing, and a drain per leaf would be a
    /// round trip per leaf.
    async fn lease_cells_many(
        &self,
        sess: &mut Router,
        pools: &[[u8; 32]],
    ) -> Result<std::collections::HashMap<[u8; 32], Vec<(u64, crate::lease::Lease)>>, CoreError> {
        const BATCH: u64 = 4;
        let root = crate::locator::storage_root(&*self.seed_or_refuse()?);
        let mut out: std::collections::HashMap<[u8; 32], Vec<(u64, crate::lease::Lease)>> =
            pools.iter().map(|p| (*p, Vec::new())).collect();
        let mut open: Vec<[u8; 32]> = pools.to_vec();
        let mut n = 1u64;
        while !open.is_empty() {
            let mut tags = Vec::new();
            for p in &open {
                for i in n..n + BATCH {
                    tags.push((*p, i, crate::lease::cell_address(&root, p, i).tag_hex()));
                }
            }
            let names: Vec<String> = tags.iter().map(|t| t.2.clone()).collect();
            let mut got = sess.drain(&names, |_| Ok(0)).await?;
            got.sort_by_key(|m| m.seq);
            let mut still = Vec::new();
            for p in &open {
                let mut full = true;
                for i in n..n + BATCH {
                    let tag = &tags.iter().find(|t| t.0 == *p && t.1 == i).expect("tag").2;
                    let found = got.iter().filter(|m| &m.tag == tag).find_map(|m| {
                        let blob = pacific_wire::blob_unb64(&m.blob).ok()?;
                        crate::lease::open_cell(&blob, &root).ok()
                    });
                    match found {
                        Some(l) => out.get_mut(p).expect("pool").push((i, l)),
                        None => {
                            full = false;
                            break;
                        }
                    }
                }
                if full {
                    still.push(*p);
                }
            }
            open = still;
            n += BATCH;
        }
        Ok(out)
    }

    /// Upkeep (§6.5), after the groups drain, on every sync. In order: renew what is
    /// due; evict every expired leaf of this person and remove it; provision a pool
    /// leaf wherever none is idle — which is both draining the joins the doors made
    /// and replenishing what a resume took; publish what the spine now owes.
    /// `Some` when the chain moved, for the platform to store (A6.1).
    pub async fn resumption_upkeep_at(
        &self,
        sess: &mut Router,
        now: i64,
    ) -> Result<Option<crate::spine::HeadUpdate>, CoreError> {
        if self.id.seed_bytes().is_none() {
            return Ok(None);
        }
        let before = self.head_update()?.position;
        let me = self.dir.device_id()?;

        if let Err(e) = self.lease_renew_due_over(sess, now).await {
            tracing::warn!(target: "pacific::resumption", error = %e, "lease renewal failed; the next sync tries again");
        }

        // What the spine says NOW, including what the person's other devices
        // appended: decided from this device's rows alone, two devices would each
        // find "no idle leaf" and both provision one.
        if let Err(e) = self.chain_walk_over(sess, SPINE_GEN).await {
            tracing::warn!(target: "pacific::resumption", error = %e, "could not walk the spine; deciding from what this device knows");
        }

        // The standing of every pool leaf the spine names, in one read.
        let groups = self.held_groups()?;
        let mut per_group: Vec<(Vec<u8>, Vec<crate::spine::PoolLeaf>)> = Vec::new();
        let mut all: Vec<[u8; 32]> = Vec::new();
        for gid in &groups {
            let pools = self.spine_pools(gid)?;
            all.extend(pools.iter().map(|p| p.pool));
            per_group.push((gid.clone(), pools));
        }
        let cells = self.lease_cells_many(sess, &all).await?;
        let (sid, sk, _) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;

        for (gid, pools) in per_group {
            let step = async {
                let group = mls::load_group(&client, &gid)?;
                let in_tree = pool_keys_in(&group);
                drop(group);
                let mut idle = false;
                for p in pools.iter().filter(|p| in_tree.contains(&p.pool)) {
                    let c = cells.get(&p.pool).cloned().unwrap_or_default();
                    match crate::lease::standing(&c) {
                        crate::lease::Standing::Idle => idle = true,
                        crate::lease::Standing::Held { n, lease } if lease.until < now && lease.holder != me => {
                            let ev = crate::lease::Lease { holder: me, until: 0, act: crate::lease::Act::Evict };
                            if self.lease_claim(sess, &p.pool, n + 1, &ev).await? {
                                self.remove_pool_leaf_over(sess, &gid, &p.pool).await?;
                            }
                        }
                        // An evictor that died before its Remove: finish it.
                        crate::lease::Standing::Evicted { .. } => {
                            self.remove_pool_leaf_over(sess, &gid, &p.pool).await?;
                        }
                        _ => {}
                    }
                }
                if !idle {
                    self.pool_provision_over(sess, &gid).await?;
                }
                Ok::<(), CoreError>(())
            };
            if let Err(e) = step.await {
                tracing::warn!(target: "pacific::resumption", group = %hex::encode(&gid), error = %e,
                               "pool upkeep failed for this object; the next sync tries again");
            }
        }

        self.drain_spine_over(sess).await?;
        let after = self.head_update()?;
        Ok((after.position > before).then_some(after))
    }

    /// Remove the pool leaf keyed `pool` from `group`, through the slot. A person
    /// may remove their own leaves (§8).
    async fn remove_pool_leaf_over(&self, sess: &mut Router, gid: &[u8], pool: &[u8; 32]) -> Result<(), CoreError> {
        let pool = *pool;
        self.commit_via_slot(sess, gid, "evict a pool leaf", 8, move |g| {
            let leaf = g
                .roster()
                .members_iter()
                .find(|m| m.signing_identity.signature_key.as_bytes() == pool)
                .map(|m| m.index);
            match leaf {
                Some(l) => mls::stage_remove(g, &[l]).map(Some),
                None => Ok(None),
            }
        })
        .await?;
        Ok(())
    }

    /// Commit a single-leaf Remove through the slot (A6.7). PacificRules judges it;
    /// this door adds no check of its own.
    pub async fn remove_leaf(&self, object: &str, leaf: u32) -> Result<(), CoreError> {
        let gid = hex::decode(object).map_err(|e| CoreError::Directory(format!("object id: {e}")))?;
        let mut sess = Router::open(&self.routes).await?;
        let out = self
            .commit_via_slot(&mut sess, &gid, "remove a leaf", 8, move |g| mls::stage_remove(g, &[leaf]).map(Some))
            .await;
        sess.close().await;
        out.map(|_| ())
    }

    /// §2.2 at the real clock.
    pub async fn resume(&self, head: Option<crate::resumption::HeadInput>) -> Result<crate::resumption::Resumed, CoreError> {
        self.resume_at(head, unix_now()).await
    }

    /// §2.2. A device holding only the seed reaches every object the spine names,
    /// publishing nothing but its lease claims (§9.3). Idempotent: an object already
    /// held is `AlreadyHeld`.
    pub async fn resume_at(
        &self,
        head: Option<crate::resumption::HeadInput>,
        now: i64,
    ) -> Result<crate::resumption::Resumed, CoreError> {
        use crate::resumption::{ObjectOutcome, Outcome, Resumed};
        let root = crate::locator::storage_root(&*self.seed_or_refuse()?);
        // A head that fails its own check is unusable, and nothing is taken (A7.1).
        let head = match head {
            Some(h) => Some(crate::head::open_declared(&h.blob, &root, h.declared_position)?),
            None => None,
        };
        let gen = head.as_ref().map_or(SPINE_GEN, |h| h.chain_gen);

        // The held session, and kept after a walk that ends well: an open pays one dial.
        let mut sess = self.home_session().await?;
        let out = async {
            let walk = self.chain_walk_over(&mut sess, gen).await?;
            let verdict = head.as_ref().map(|h| h.check(walk.position, &walk.tail));

            // The spine is the self record at index 0 (A2); then the live objects in
            // first-joined order, and the latest way-in of each.
            let Some(SpinePlan { spine, targets, ways }) = spine_plan(&walk.entries) else {
                return Ok(Resumed { verdict, spine: None, objects: Vec::new() });
            };

            let mut objects = Vec::new();
            for gid in targets {
                let way = ways.get(&gid);
                let kind = way
                    .map(|w| w.kind.clone())
                    .filter(|k| !k.is_empty())
                    .unwrap_or_else(|| if gid == spine { "group".into() } else { String::new() });
                let outcome = if self.dir.group_kind(&gid)?.is_some() && !self.dir.is_departed(&gid)? {
                    Outcome::AlreadyHeld
                } else {
                    match way {
                        None => Outcome::WayInMissing,
                        Some(w) => self.take_and_join(&mut sess, &gid, w, now).await?,
                    }
                };
                if gid == spine && matches!(outcome, Outcome::Joined { .. } | Outcome::AlreadyHeld) {
                    self.dir.set_my_object(&gid)?;
                }
                objects.push(ObjectOutcome { object: hex::encode(&gid), kind, outcome });
            }
            Ok(Resumed { verdict, spine: Some(hex::encode(&spine)), objects })
        }
        .await;
        match out {
            Ok(_) => self.keep_session(sess),
            Err(_) => sess.close().await,
        }
        out
    }

    /// Take the first idle leaf the way-in lists and join through it. The only thing
    /// written is the lease claim.
    ///
    /// A way-in whose route fails the check (SECURITY, 27 Sep) takes no leaf and
    /// writes nothing; `noncompliant_objects` names it.
    async fn take_and_join(
        &self,
        sess: &mut Router,
        gid: &[u8],
        way: &crate::spine::Pool,
        now: i64,
    ) -> Result<crate::resumption::Outcome, CoreError> {
        use crate::resumption::Outcome;
        if let Some(why) = way_in_refusal(way) {
            return Ok(Outcome::JoinFailed(why));
        }
        let me = self.dir.device_id()?;
        for leaf in &way.pools {
            let cells = self.lease_cells_over(sess, &leaf.pool).await?;
            if crate::lease::standing(&cells) != crate::lease::Standing::Idle {
                continue;
            }
            let until = now + crate::lease::LEASE_SECS;
            let lease = crate::lease::Lease { holder: me, until, act: crate::lease::Act::Take };
            if !self.lease_claim(sess, &leaf.pool, 1, &lease).await? {
                continue; // another device took it first
            }
            self.dir.put_lease(gid, &leaf.pool, 1, until)?;
            return Ok(match self.join_through(gid, way, leaf) {
                Ok(()) => {
                    // Climb to the current epoch — reading only (§9.3).
                    if let Err(e) = self.drain_to_head(gid).await {
                        tracing::warn!(target: "pacific::resumption", group = %hex::encode(gid), error = %e,
                                       "joined, but could not climb yet; the next sync walks forward");
                    }
                    Outcome::Joined { from_epoch: leaf.epoch }
                }
                Err(e) => Outcome::JoinFailed(e.to_string()),
            });
        }
        Ok(Outcome::PoolExhausted)
    }

    /// §5.4: the key package into an in-memory store, this device's own group-state
    /// store under it, the pool leaf's signing identity; join from the Welcome and
    /// persist. mls-rs keeps the signer in the group's snapshot, so from here the
    /// ordinary client loads the group and signs in it as the pool leaf.
    fn join_through(&self, gid: &[u8], way: &crate::spine::Pool, leaf: &crate::spine::PoolLeaf) -> Result<(), CoreError> {
        let me = self.id.identity_pk();
        let (_, kps) = crate::mls_mem::Snapshot {
            states: vec![],
            epochs: vec![],
            key_packages: vec![(leaf.kp_id.clone(), leaf.kp_data.clone())],
        }
        .restore();
        let gss = crate::mls_store::SqliteGroupStateStorage::open(&paths::db_path())
            .map_err(|e| CoreError::Mls(e.to_string()))?;
        let client = mls::build_client(
            gss,
            kps,
            mls::signing_identity(&me, &leaf.pool),
            mls::SecretKey::from(leaf.sig_sk.clone()),
        )?;
        let mut group = mls::join_group_unsaved(&client, &leaf.welcome)?;
        if group.group_id() != gid {
            return Err(CoreError::Handshake("the way-in's Welcome is for another group".into()));
        }
        mls::save_group(&mut group)?;
        let owner = mls::group_owner(&group)?;
        let roster = mls::roster_identities(&group)?;
        let epoch = group.current_epoch();
        drop(group);

        self.dir.record_owner(gid, epoch, &owner)?;
        if self.dir.group_kind(gid)?.is_none() {
            let arc = (!way.arc.is_empty()).then_some(way.arc.as_str());
            self.dir.put_group(gid, &way.kind, &owner, arc)?;
        }
        self.dir.set_group_members(gid, &roster)?;
        // THE GEN FLOOR. The pool leaf is the same author as the person's other
        // leaves and cannot see their messages before its epoch, so its Lamport
        // generations start past anything they could have minted (A8).
        self.dir.raise_gen_floor(gid, unix_now_ms())?;
        let (sid, sk, _) = self.signer()?;
        let g = mls::load_group(&mls::build_client_sqlite(&paths::db_path(), sid, sk)?, gid)?;
        self.record_current_epoch_tag(gid, &g)?;
        Ok(())
    }

    // ── leases (resumption.md §6) ──────────────────────────────────────────────

    /// A pool leaf's cells, from 1 until the first empty one, opened.
    pub async fn lease_cells(&self, pool: &[u8; 32]) -> Result<Vec<(u64, crate::lease::Lease)>, CoreError> {
        let mut sess = Router::open(&self.routes).await?;
        let out = self.lease_cells_over(&mut sess, pool).await;
        sess.close().await;
        out
    }

    async fn lease_cells_over(
        &self,
        sess: &mut Router,
        pool: &[u8; 32],
    ) -> Result<Vec<(u64, crate::lease::Lease)>, CoreError> {
        const BATCH: u64 = 8;
        let root = crate::locator::storage_root(&*self.seed_or_refuse()?);
        let mut out = Vec::new();
        let mut n = 1u64;
        loop {
            let tags: Vec<String> =
                (n..n + BATCH).map(|i| crate::lease::cell_address(&root, pool, i).tag_hex()).collect();
            let mut got = sess.drain(&tags, |_| Ok(0)).await?;
            got.sort_by_key(|m| m.seq);
            for (k, tag) in tags.iter().enumerate() {
                let found = got.iter().filter(|m| &m.tag == tag).find_map(|m| {
                    let blob = pacific_wire::blob_unb64(&m.blob).ok()?;
                    crate::lease::open_cell(&blob, &root).ok()
                });
                match found {
                    Some(l) => out.push((n + k as u64, l)),
                    None => return Ok(out),
                }
            }
            n += BATCH;
        }
    }

    /// Claim cell `n` of `pool`. `true` is the lease; `false` is someone else's.
    async fn lease_claim(
        &self,
        sess: &mut Router,
        pool: &[u8; 32],
        n: u64,
        lease: &crate::lease::Lease,
    ) -> Result<bool, CoreError> {
        let root = crate::locator::storage_root(&*self.seed_or_refuse()?);
        let blob = crate::lease::seal_cell(lease, &root)?;
        Ok(sess
            .publish_commit(&crate::lease::cell_address(&root, pool, n), &pacific_wire::blob_b64(&blob))
            .await?
            .is_some())
    }

    /// TAKE: only from idle (§6.2). `true` means this device now speaks through the
    /// leaf until `now + LEASE_SECS`; `false` means it was held or evicted.
    pub async fn lease_take(&self, object: &str, pool: &[u8; 32], now: i64) -> Result<bool, CoreError> {
        let gid = hex::decode(object).map_err(|e| CoreError::Directory(format!("object id: {e}")))?;
        let mut sess = Router::open(&self.routes).await?;
        let out = async {
            if self.lease_cells_over(&mut sess, pool).await?.first().is_some() {
                return Ok(false);
            }
            let until = now + crate::lease::LEASE_SECS;
            let lease = crate::lease::Lease { holder: self.dir.device_id()?, until, act: crate::lease::Act::Take };
            if !self.lease_claim(&mut sess, pool, 1, &lease).await? {
                return Ok(false);
            }
            self.dir.put_lease(&gid, pool, 1, until)?;
            Ok(true)
        }
        .await;
        sess.close().await;
        out
    }

    /// RENEW every held lease with fewer than `LEASE_RENEW_BELOW` seconds left. A lost
    /// cell is a FENCE: the leaf is someone else's now and this device stops speaking
    /// through it.
    pub async fn lease_renew_due(&self, now: i64) -> Result<(), CoreError> {
        let mut sess = Router::open(&self.routes).await?;
        let out = self.lease_renew_due_over(&mut sess, now).await;
        sess.close().await;
        out
    }

    async fn lease_renew_due_over(&self, sess: &mut Router, now: i64) -> Result<(), CoreError> {
        let due: Vec<_> = self
            .dir
            .leases()?
            .into_iter()
            .filter(|(_, _, _, until, fenced)| !fenced && until - now < crate::lease::LEASE_RENEW_BELOW)
            .collect();
        let me = self.dir.device_id()?;
        for (gid, pool, cell, _, _) in due {
            let until = now + crate::lease::LEASE_SECS;
            let lease = crate::lease::Lease { holder: me, until, act: crate::lease::Act::Renew };
            if self.lease_claim(sess, &pool, cell + 1, &lease).await? {
                self.dir.put_lease(&gid, &pool, cell + 1, until)?;
            } else {
                self.dir.mark_fenced(&gid, &pool)?;
                tracing::warn!(target: "pacific::resumption", group = %hex::encode(&gid),
                               "lease lost at renewal — fenced; this device stops speaking there");
            }
        }
        Ok(())
    }

    /// EVICT: a pool leaf whose current lease expired and is not this device's.
    /// Claims the next cell with `Evict`; `true` means this device must now remove the
    /// leaf (§6.2). A leaf is never taken over (§6.3).
    pub async fn lease_evict(&self, pool: &[u8; 32], now: i64) -> Result<bool, CoreError> {
        let me = self.dir.device_id()?;
        let mut sess = Router::open(&self.routes).await?;
        let out = async {
            let cells = self.lease_cells_over(&mut sess, pool).await?;
            match crate::lease::standing(&cells) {
                crate::lease::Standing::Held { n, lease } if lease.until < now && lease.holder != me => {
                    let ev = crate::lease::Lease { holder: me, until: 0, act: crate::lease::Act::Evict };
                    self.lease_claim(&mut sess, pool, n + 1, &ev).await
                }
                _ => Ok(false),
            }
        }
        .await;
        sess.close().await;
        out
    }

    /// Objects in which this device was fenced, hex (A7.4).
    pub fn fenced(&self) -> Result<Vec<String>, CoreError> {
        Ok(self.dir.fenced_groups()?.into_iter().map(hex::encode).collect())
    }

    /// List our standalone objects: (object_id, kind).
    pub fn objects(&self) -> Result<Vec<(String, String)>, CoreError> {
        Ok(self
            .dir
            .list_objects()?
            .into_iter()
            .map(|(g, k)| (hex::encode(g), k))
            .collect())
    }

    /// [`objects`] plus each object's DISPLAY NAME: `(id, kind, name)`.
    ///
    /// Same source of truth as [`object_name`] — the MLS GroupContext, never a
    /// directory cache that could go stale against a rename commit — but it builds
    /// the MLS client ONCE for the whole list instead of once per row. The chat list
    /// rebuilds on every sync tick, so the per-row `object_name` round trip (open the
    /// DB, rebuild the client, load the group) was the dominant cost there.
    ///
    /// A group that fails to load is reported with an empty name rather than
    /// failing the whole list — one unreadable object must not blank the UI.
    pub fn objects_named(&self) -> Result<Vec<(String, String, String)>, CoreError> {
        let (sid, sk, _n) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        Ok(self
            .dir
            .list_objects()?
            .into_iter()
            .map(|(g, k)| {
                let name = mls::load_group(&client, &g)
                    .as_ref()
                    .map(mls::group_name)
                    .unwrap_or_default();
                (hex::encode(g), k, name)
            })
            .collect())
    }

    /// All groups this device holds: `(group_id_hex, kind)` — including connections, unlike
    /// [`objects`]. The Arc-served console splits these by kind (`arc-tether` vs `connection`).
    pub fn all_groups(&self) -> Result<Vec<(String, String)>, CoreError> {
        Ok(self
            .dir
            .all_groups()?
            .into_iter()
            .map(|(g, k)| (hex::encode(g), k))
            .collect())
    }

    /// The roster (member identity pubkeys) of a group, by hex group id.
    pub fn group_roster(&self, group_id_hex: &str) -> Result<Vec<[u8; 32]>, CoreError> {
        let gid =
            hex::decode(group_id_hex).map_err(|_| CoreError::Directory("bad group id".into()))?;
        self.dir.group_members(&gid)
    }

    /// The owner identity pubkey of a group as hex, by hex group id (`None` if unrecorded).
    pub fn group_owner_hex(&self, group_id_hex: &str) -> Result<Option<String>, CoreError> {
        let gid =
            hex::decode(group_id_hex).map_err(|_| CoreError::Directory("bad group id".into()))?;
        Ok(self.dir.group_owner(&gid)?.map(hex::encode))
    }

    /// The stored display name for a peer/member (`""` if unknown).
    pub fn peer_name(&self, pk: &[u8; 32]) -> Result<String, CoreError> {
        self.dir.peer_display_name(pk)
    }

    // ======================================================================
    // THE ACCOUNT, IN TWO ARTEFACTS (ruled 13 Sep 2026, D1):
    //   THE WRAP     the seed under the passkey (`crate::wrap`). Served to
    //                anyone who asks by public key, because that read is how a
    //                new device obtains the key it would authenticate with.
    //   THE HISTORY  archive + MLS state + intro tag, under a SEED-derived key.
    //                Retired 14 Sep 2026 as a key escrow; its module is gone
    //                (a58798c).
    //
    // Restore on a fresh install = open the wrap (or type the 24 words) ->
    // `resume`: take a pool leaf in every object the spine names (resumption.md
    // §2.2). Splitting the two is what let the Arc's out-of-band second factor
    // go: the public artefact carries no history.
    // ======================================================================

    /// EXPORT THE WRAP: the identity seed, sealed under the passkey and bound to
    /// the host it will be stored on.
    ///
    /// This is the artefact the Arc serves to ANYONE who asks for it by public
    /// key. That read cannot be gated on authentication, because it is how a new
    /// device obtains the key it would authenticate with — so the wrap carries
    /// the seed and nothing else: no history, no ratchet state, nothing that
    /// would make a public read a disclosure (D1, [`crate::wrap`]).
    ///
    /// An identity with no seed — a legacy key file holding its two secrets
    /// directly — is REFUSED rather than wrapped without one.
    pub fn export_wrap(&self, prf: &[u8; 32], host: &str) -> Result<Vec<u8>, CoreError> {
        let seed = self.seed_or_refuse()?;
        crate::wrap::seal(prf, &seed, host)
    }

    /// The seed, or the one refusal both exports share.
    fn seed_or_refuse(&self) -> Result<Zeroizing<[u8; 32]>, CoreError> {
        self.id.seed_bytes().ok_or_else(|| {
            CoreError::Identity(
                "this identity has no seed (a legacy key file) and cannot be backed up — \
                 a backup without it would restore as a different identity"
                    .into(),
            )
        })
    }

    /// RESTORE FROM THE WRAP. A fresh install, the wrap pulled from the Arc, and
    /// the passkey's PRF output: out comes the same identity, in no group yet.
    ///
    /// The ORDER is the safety argument:
    ///   1. open the wrap. A wrong passkey, a wrap lifted from another host, or a
    ///      tampered byte fails HERE, before one byte is written, so a refused
    ///      restore leaves the install exactly as empty as it found it;
    ///   2. the identity, from the seed (`identity::init_from_seed`) — the same
    ///      never-overwrite refusal as the mint and the 24-word door;
    ///   3. the device-local half exactly as `open_new_identity` builds it: a
    ///      fresh directory, a fresh MLS signing key, a fresh intro tag.
    ///
    /// AND THEN THE CALLER MUST RESUME. This touches no network and joins no
    /// group. `resume`, given the head, takes a pool leaf in every object the
    /// spine names (resumption.md §2.2); run it before `ensure_self_object`, or
    /// that mints a second self record. The history import and rekey this door
    /// once did went with the escrow (a58798c).
    ///
    /// A failure after step 2 leaves a partial install behind (a key file with a
    /// half-built directory), which a second attempt refuses as an existing
    /// identity. That is the same exposure every door through `open_new_identity`
    /// has, and the remedy is the same: wipe the state dir and restore again.
    pub fn restore_from_wrap(
        prf: &[u8; 32],
        wrap: &[u8],
        host: &str,
        display_name: &str,
    ) -> Result<Self, CoreError> {
        let seed = crate::wrap::open(prf, wrap, host)?;
        let id = identity::init_from_seed(&seed)?;
        Self::open_new_identity(id, display_name)
    }

    /// SIGN IN ON A DEVICE THAT HOLDS NOTHING: the wrap, then every object the
    /// spine names, in the one order that is safe. `restore_from_wrap` leaves the
    /// rest to its caller; this is the rest, stated once (mdr/door.md §4 step 3):
    ///   1. the seed out of the wrap must derive `expect_key`, the key the
    ///      passkey's handle names. Another key is refused before a byte is
    ///      written: restoring it would sign in as a stranger;
    ///   2. `restore_from_wrap`: the identity and a fresh device half;
    ///   3. `resume` with the head the auth service holds (resumption.md §2.2,
    ///      A3): a pool leaf in every object the spine names;
    ///   4. the self record, only when the spine was reached or the chain is
    ///      empty (an account made where no Node ran, whose first device this
    ///      is). A spine the chain names and this device could not join is
    ///      reported, never replaced: minting here would give the person two.
    ///
    /// The routes must already be in the state dir, because `resume` walks the
    /// chain on the relay. The `Resumed` names every object's outcome. An error
    /// after step 2 leaves the identity behind, as `restore_from_wrap`'s does.
    pub async fn sign_in_from_wrap(
        prf: &[u8; 32],
        wrap: &[u8],
        host: &str,
        display_name: &str,
        expect_key: &str,
        head: Option<crate::resumption::HeadInput>,
    ) -> Result<(Self, crate::resumption::Resumed), CoreError> {
        use crate::resumption::Outcome;
        let found = identity::Identity::in_memory(*crate::wrap::open(prf, wrap, host)?).identity_key();
        if found != expect_key {
            return Err(CoreError::Identity(format!(
                "the wrap for {expect_key} opens to {found}; nothing was written"
            )));
        }
        let node = Self::restore_from_wrap(prf, wrap, host, display_name)?;
        let resumed = node.resume(head).await?;
        // The spine is first (A6.5); no objects at all means no chain.
        let reached = match resumed.objects.first() {
            None => resumed.spine.is_none(),
            Some(o) => matches!(o.outcome, Outcome::Joined { .. } | Outcome::AlreadyHeld),
        };
        if reached {
            node.ensure_self_object()?;
        }
        Ok((node, resumed))
    }

    /// SEAL THIS DEVICE'S STATE for `node` at `counter` (`devstate`; D-34 (c)): the
    /// database, the identity file, the arc, and the at-rest key their secrets are
    /// sealed under, under a key off this identity's seed.
    ///
    /// The database is copied by `VACUUM INTO`, a consistent copy through this Node's
    /// own connection, into the state directory itself ([`crate::devstate::SNAPSHOT`]),
    /// and the copy is gone before this returns. The device id is read first: it is
    /// minted on first read, and a snapshot taken before it would restore as another
    /// device.
    pub fn seal_state(&self, node: &str, counter: u64) -> Result<Vec<u8>, CoreError> {
        let device_key = Zeroizing::new(crate::atrest::device_key()?.ok_or(CoreError::AtRestKeyUnavailable)?);
        let header = crate::devstate::Header { node: node.to_string(), device_id: self.dir.device_id()?, counter };
        let dir = paths::state_dir();
        let snap = dir.join(crate::devstate::SNAPSHOT);
        let db = {
            let _gone = crate::devstate::Scrub(snap.clone());
            let _ = std::fs::remove_file(&snap);
            self.dir.conn.execute("VACUUM INTO ?1", [snap.to_string_lossy().as_ref()])?;
            Zeroizing::new(std::fs::read(&snap)?)
        };
        let mut files = vec![("pacific.db".to_string(), db)];
        for name in crate::devstate::FILES.iter().filter(|n| **n != "pacific.db") {
            match std::fs::read(dir.join(name)) {
                Ok(b) => files.push((name.to_string(), Zeroizing::new(b))),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        let key = crate::devstate::seal_key(&crate::locator::storage_root(&*self.seed_or_refuse()?));
        let contents = crate::devstate::Contents { device_key, files };
        crate::devstate::seal(&self.id.identity_pk(), &header, &contents, &key)
    }

    /// OPEN A SEALED STATE into the state directory, which must hold none: sealed for
    /// the person `seed` is, on `node`, holding the device its header names, or
    /// refused. The at-rest key becomes the seal's.
    ///
    /// On `Err` the directory holds none of it and the at-rest key is new, so the
    /// caller can go on as a device that has never seen the account. Whether the
    /// header's counter is current is the caller's to judge, against the auth
    /// service's; whether the state is stale in-band is [`Node::drain_only`]'s.
    pub fn open_restored(
        blob: &[u8],
        seed: &[u8; 32],
        node: &str,
    ) -> Result<(Self, crate::devstate::Header), CoreError> {
        let dir = paths::state_dir();
        if let Some(held) = crate::devstate::FILES.iter().find(|n| dir.join(n).exists()) {
            return Err(CoreError::Seal(format!(
                "{} already holds {held}: a state opens into an empty directory",
                dir.display()
            )));
        }
        let opened = (|| {
            let pk = identity::Identity::in_memory(*seed).identity_pk();
            let key = crate::devstate::seal_key(&crate::locator::storage_root(seed));
            let (header, contents) = crate::devstate::open(blob, &pk, node, &key)?;
            crate::devstate::restore(&contents, &dir)?;
            let n = Self::open()?;
            if n.id.identity_pk() != pk {
                return Err(CoreError::Seal("the state holds another identity".into()));
            }
            if n.dir.device_id()? != header.device_id {
                return Err(CoreError::Seal("the state holds another device than its header names".into()));
            }
            Ok((n, header))
        })();
        if opened.is_err() {
            crate::devstate::clear(&dir)?;
        }
        opened
    }

    /// PUT THIS STATE DOWN: the Node closed, its files gone from the state directory,
    /// and a new at-rest key, so what follows starts as a device that has never seen
    /// the account. What a stale [`Node::drain_only`] ends in (D-34 (c)).
    pub fn discard(self) -> Result<(), CoreError> {
        drop(self);
        crate::devstate::clear(&paths::state_dir())
    }

    /// CLOSE THE FORK: a fresh-path commit in EVERY group this device holds,
    /// through the relay's commit slot. It followed the backup restore, retired
    /// with the escrow (a58798c); no path in core calls it now, and the FFI
    /// still exposes it.
    ///
    /// Per group (`rekey_group`): stage an empty commit (`mls::stage_rekey`), seal
    /// it to the CURRENT epoch's tag, claim that epoch's single commit slot, and
    /// only on a win apply it and record the new epoch's tag. Losing the slot
    /// means another member committed first: fold their commit off the old tag,
    /// re-stage against the new epoch, try again. The same discipline as
    /// `add_member_core` and `object_rename`, minus the Welcome.
    ///
    /// A SOLO group (no one else has joined) skips the relay — there is no slot to
    /// win and no one to hear the commit — but still commits, so the two copies
    /// part ways there too and the epoch tells them apart.
    ///
    /// Returns the hex id of every group rekeyed. A group that cannot be rekeyed
    /// fails the WHOLE call, not that row: a restore with one group left un-rekeyed
    /// is a restore with one group still forked, and the caller has to know. Safe
    /// to run again — each run is a fresh commit, and the groups already rekeyed
    /// simply advance one more epoch.
    ///
    /// Iterates the DIRECTORY's groups, which after a restore are the archive's:
    /// a kind the archive does not carry (an `arc-tether`) has no directory row
    /// here, so its MLS state — restored with the rest — is never operated on,
    /// and the device re-tethers as a fresh leaf. That is the archive's line, not
    /// a gap in this one.
    pub async fn rekey_all_groups(&self, sess: &mut Router) -> Result<Vec<String>, CoreError> {
        let mut done = Vec::new();
        for (group_id, _kind) in self.dir.all_groups()? {
            // A departed group has no MLS state left to rekey (§8.3).
            if self.dir.is_departed(&group_id)? {
                continue;
            }
            self.rekey_group(sess, &group_id).await?;
            done.push(hex::encode(&group_id));
        }
        Ok(done)
    }

    /// One group's rekey through the relay's commit slot — see `rekey_all_groups`.
    async fn rekey_group(&self, sess: &mut Router, group_id: &[u8]) -> Result<(), CoreError> {
        let (sid, sk, _n) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut group = mls::load_group(&client, group_id)?;

        // A group of one has no peers to sequence against: no relay round trip,
        // no slot to win. Apply directly — the shortcut group creation takes.
        if mls::roster_identities(&group)?.len() <= 1 {
            mls::stage_rekey(&mut group)?;
            mls::apply_staged(&mut group)?;
            self.record_current_epoch_tag(group_id, &group)?;
            return Ok(());
        }

        // Bounded. Every lost round means another member's commit went in first
        // and we folded it; a slot we cannot win in this many rounds is a group
        // committing faster than we can follow, and that is worth surfacing
        // rather than spinning on.
        const MAX_LOST_SLOTS: usize = 8;
        for _ in 0..MAX_LOST_SLOTS {
            // Capture the CURRENT epoch's tag + secret before we advance, so the
            // members still at that epoch can drain the commit off it. Staging
            // does not modify state (RFC 9420 §14), so this stays valid across
            // `stage_rekey`.
            let old_epoch_n = group.current_epoch();
            let old_epoch = mls::epoch_be(old_epoch_n);
            let old_tag = mls::group_tag(&group, &old_epoch)?;
            let old_addr = mls::group_address(&group, &old_epoch)?;
            let old_secret = mls::seal_conn_secret(&group, &old_epoch)?;
            self.dir
                .record_epoch_tag(group_id, old_epoch_n, &old_tag, &old_secret)?;

            let commit = mls::stage_rekey(&mut group)?;
            let sealed_commit = seal::seal(&commit, &old_tag, &old_secret)?;
            let won = sess
                .publish_commit(
                    &old_addr,
                    &pacific_wire::blob_b64(&sealed_commit),
                )
                .await?;
            if won.is_some() {
                mls::apply_staged(&mut group)?;
                self.after_own_commit(group_id, &group)?;
                return Ok(());
            }
            // LOST the slot: the staged commit was never applied or persisted, so
            // reloading discards it. Fold the winner in off the old tag — that is
            // what moves us to the epoch the next round stages against.
            group = mls::load_group(&client, group_id)?;
            self.drain_tag(sess, &mut group, group_id, &old_tag, &old_secret)
                .await?;
            if group.current_epoch() == old_epoch_n {
                // The slot is held by something we cannot process: junk (now in
                // quarantine), or a commit from OUR OWN leaf — the stale install
                // committed first, and its copy of this group has won. Either way
                // there is no epoch to follow it to, and no point re-staging.
                return Err(CoreError::CommitRace(format!(
                    "epoch {old_epoch_n} commit slot of {} is held by a commit this device \
                     cannot process — the group cannot be rekeyed from here",
                    hex::encode(group_id)
                )));
            }
        }
        Err(CoreError::CommitRace(format!(
            "gave up rekeying {} after {MAX_LOST_SLOTS} lost commit slots",
            hex::encode(group_id)
        )))
    }

    // ======================================================================
    // MEMBERSHIP THROUGH MLS — docs/membership-through-mls.md §5–§11.
    //
    // The roster IS the access control, so every change to it is an MLS commit, and
    // the membership deltas are RECORDS written alongside the commit that makes them
    // true (§10.3). Every door here is logged under `pacific::membership`: who asked,
    // what was committed, which epoch, and — when a slot is lost or a rule refuses —
    // why, in words.
    // ======================================================================

    /// Refuse to act on a group a commit removed this device from (§8.3).
    fn refuse_if_departed(&self, group_id: &[u8]) -> Result<(), CoreError> {
        if self.dir.is_departed(group_id)? {
            return Err(CoreError::Membership(
                "you were removed from this group — you are no longer a member".into(),
            ));
        }
        Ok(())
    }

    /// Load a group this device is about to WRITE to. A device a commit removed holds
    /// no MLS state for the group any more (§8.3), so a bare load would fail with
    /// "group not found"; this says what actually happened.
    fn load_member_group(&self, client: &mls::Client, group_id: &[u8]) -> Result<mls::Group, CoreError> {
        self.refuse_if_departed(group_id)?;
        mls::load_group(client, group_id)
    }

    /// What a device records after ITS OWN commit wins its slot: everything any commit
    /// records (§8.2), the new epoch's tag, and that this leaf was just refreshed —
    /// every commit carries a path (§4.4), so every win is a self-update (§11).
    fn after_own_commit(&self, group_id: &[u8], group: &mls::Group) -> Result<(), CoreError> {
        self.after_commit(group_id, group)?;
        self.record_current_epoch_tag(group_id, group)?;
        self.dir
            .record_self_update(group_id, unix_now(), group.current_epoch())?;
        Ok(())
    }

    /// THE ONE WAY A MEMBERSHIP DOOR COMMITS — `rekey_group`'s discipline, shared.
    /// `stage` builds the commit against the group as it is NOW (it is called afresh each
    /// round), or returns `None` when there is nothing left to do — someone else's
    /// commit already did it. Seal to the current epoch's tag, claim that epoch's single
    /// slot; on a win apply and record, on a loss fold the winner off the old tag and
    /// try again, at most `max_rounds` times. A group of one leaf has no slot to win and
    /// applies directly. Returns whether a commit of ours was applied.
    async fn commit_via_slot<F>(
        &self,
        sess: &mut Router,
        group_id: &[u8],
        what: &'static str,
        max_rounds: usize,
        mut stage: F,
    ) -> Result<bool, CoreError>
    where
        F: FnMut(&mut mls::Group) -> Result<Option<Vec<u8>>, CoreError>,
    {
        let gid = hex::encode(group_id);
        let (sid, sk, _n) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut group = mls::load_group(&client, group_id)?;

        if group.roster().members_iter().count() <= 1 {
            let Some(_commit) = stage(&mut group)? else {
                return Ok(false);
            };
            mls::apply_staged(&mut group)?;
            self.after_own_commit(group_id, &group)?;
            tracing::info!(target: "pacific::membership", group = %gid, what,
                           epoch = group.current_epoch(), "committed (solo — no slot)");
            return Ok(true);
        }

        for round in 0..max_rounds {
            let old_epoch_n = group.current_epoch();
            let old_epoch = mls::epoch_be(old_epoch_n);
            let old_tag = mls::group_tag(&group, &old_epoch)?;
            let old_addr = mls::group_address(&group, &old_epoch)?;
            let old_secret = mls::seal_conn_secret(&group, &old_epoch)?;
            self.dir
                .record_epoch_tag(group_id, old_epoch_n, &old_tag, &old_secret)?;
            let Some(commit) = stage(&mut group)? else {
                tracing::debug!(target: "pacific::membership", group = %gid, what, round,
                                "nothing left to commit");
                return Ok(false);
            };
            let sealed = seal::seal(&commit, &old_tag, &old_secret)?;
            let won = sess
                .publish_commit(&old_addr, &pacific_wire::blob_b64(&sealed))
                .await?;
            if won.is_some() {
                mls::apply_staged(&mut group)?;
                self.after_own_commit(group_id, &group)?;
                tracing::info!(target: "pacific::membership", group = %gid, what, round,
                               from_epoch = old_epoch_n, epoch = group.current_epoch(),
                               "commit won its slot");
                return Ok(true);
            }
            tracing::info!(target: "pacific::membership", group = %gid, what, round,
                           epoch = old_epoch_n,
                           "commit slot lost — folding the winner and re-staging");
            group = mls::load_group(&client, group_id)?;
            self.drain_tag(sess, &mut group, group_id, &old_tag, &old_secret)
                .await?;
            if self.dir.is_departed(group_id)? {
                return Err(CoreError::Membership(
                    "removed from the group while committing".into(),
                ));
            }
            if group.current_epoch() == old_epoch_n {
                return Err(CoreError::CommitRace(format!(
                    "epoch {old_epoch_n} commit slot of {gid} is held by a commit this device \
                     cannot process — the group cannot advance from here ({what})"
                )));
            }
        }
        Err(CoreError::CommitRace(format!(
            "gave up ({what}) on {gid} after {max_rounds} lost commit slots"
        )))
    }

    /// §7 — commit the proposals waiting in this group's cache. RFC 9420 §12.4: a member
    /// that has seen proposals MUST commit before it sends; mls-rs enforces the second
    /// half. This is what completes somebody else's leave. A device whose own removal is
    /// among the proposals never commits: its commit could only drop them (§4.2) and
    /// restart the leave. Returns whether a commit of ours landed.
    async fn commit_pending(&self, sess: &mut Router, group_id: &[u8]) -> Result<bool, CoreError> {
        let me = self.id.identity_pk();
        {
            let (sid, sk, _n) = self.signer()?;
            let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
            let group = mls::load_group(&client, group_id)?;
            if !mls::commit_required(&group) {
                return Ok(false);
            }
            if self.dir.pending_departure(group_id)?.is_some() || mls::pending_removal_of(&group, &me) {
                tracing::debug!(target: "pacific::membership", group = %hex::encode(group_id),
                                "proposals wait, but they remove this device — another member commits them");
                return Ok(false);
            }
        }
        tracing::info!(target: "pacific::membership", group = %hex::encode(group_id),
                       "committing the proposals waiting in the cache");
        self.commit_via_slot(sess, group_id, "commit pending proposals", 8, |g| {
            if !mls::commit_required(g) {
                return Ok(None);
            }
            mls::stage_rekey(g).map(Some)
        })
        .await
    }

    /// §6.1 step 3 — propose the removal of EVERY leaf of this person, by reference, at
    /// the current epoch, and mark the leave as in flight. Proposals ride the ordinary
    /// publish path, not the commit slot. Returns the epoch proposed at.
    async fn propose_my_departure(&self, sess: &mut Router, group_id: &[u8]) -> Result<u64, CoreError> {
        let me = self.id.identity_pk();
        let (sid, sk, _n) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut group = mls::load_group(&client, group_id)?;
        let epoch_n = group.current_epoch();
        let eb = mls::epoch_be(epoch_n);
        let tag = mls::group_tag(&group, &eb)?;
        let addr = mls::group_address(&group, &eb)?;
        let secret = mls::seal_conn_secret(&group, &eb)?;
        self.dir.record_epoch_tag(group_id, epoch_n, &tag, &secret)?;
        let leaves = mls::leaves_of(&group, &me);
        for leaf in &leaves {
            let p = mls::propose_remove(&mut group, *leaf)?;
            let sealed = seal::seal(&p, &tag, &secret)?;
            sess.publish(&addr, &pacific_wire::blob_b64(&sealed)).await?;
        }
        self.dir.set_pending_departure(group_id, epoch_n, unix_now())?;
        tracing::info!(target: "pacific::membership", group = %hex::encode(group_id),
                       epoch = epoch_n, leaves = leaves.len(),
                       "proposed this person's removal — another member completes it");
        Ok(epoch_n)
    }

    /// The person a member hex names — bare hex or `ed25519:<hex>`.
    fn parse_member(member_hex: &str) -> Result<[u8; 32], CoreError> {
        let h = member_hex.trim();
        let h = h.strip_prefix("ed25519:").unwrap_or(h);
        hex::decode(h)
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
            .ok_or_else(|| CoreError::Membership(format!("not a member id: {member_hex}")))
    }

    /// Write a membership RECORD (§10.3) through the Group write path — only ever
    /// called by the doors below, alongside the MLS operation it records.
    async fn write_membership_record(
        &self,
        object_id_hex: &str,
        op_id: u32,
        member: &[u8; 32],
        reason: Option<&str>,
    ) -> Result<String, CoreError> {
        let mut args = coordinator::Args::new();
        args.insert("member".into(), coordinator::ArgVal::Text(hex::encode(member)));
        args.insert("at".into(), coordinator::ArgVal::Int(unix_millis() as i64));
        if let Some(r) = reason {
            args.insert("reason".into(), coordinator::ArgVal::Text(r.to_string()));
        }
        self.write_record(object_id_hex, op_id, args).await
    }

    /// Whether `who` holds the admitter role on this object (A-3).
    /// Does `who` hold `admitter` here: on a Site (a Group), or on one of its rooms (a
    /// Forum, D-58). The recorded kind picks the fold, never a try of each.
    fn is_admitter(&self, object_id_hex: &str, who: &[u8; 32]) -> bool {
        let Ok(gid) = self.object_group_id(object_id_hex) else { return false };
        let role = match self.dir.group_kind(&gid) {
            Ok(Some(k)) if k == "forum" => self.forum_state(&gid).ok().and_then(|st| st.roles.get(who).copied()),
            Ok(Some(k)) if group::is_group_typed(&k) => {
                self.group_state(object_id_hex).ok().and_then(|st| st.member_roles.get(who).copied())
            }
            _ => None,
        };
        role == Some(crate::group::GroupRole::Admitter)
    }

    /// ADMISSION BY A KIOSK CLAIM (A-3, D-53, D-58, SEC-A1): the claim checked against
    /// the Site's registered kiosk keys (and `stand_in`, labelled, until they are),
    /// unspent in the Site's fold; exactly the bundles' one identity added to the Site, by
    /// this Node as owner or admitter, and the spend recorded beside the Add; then that
    /// member added to every part of the Site this Node admits to (its rooms), a fresh key
    /// package each. A room that cannot be joined is named. The same member presenting the
    /// same claim again completes the rooms, with no second spend. The CALLER serialises
    /// admissions (the Arc node holds one lock over every Node call), so two presentations
    /// of one claim cannot both pass the unspent check; the fold's first-per-claim is the
    /// backstop.
    pub async fn admit_by_claim(
        &self,
        site_hex: &str,
        token: &str,
        bundles: &[String],
        stand_in: &std::collections::HashMap<String, [u8; 32]>,
    ) -> Result<Admitted, CoreError> {
        use base64::Engine as _;
        // ONE PERSON: every key package is the same identity's (Software Security, D-58).
        let ids = bundles
            .iter()
            .map(|b| handshake::parse_and_verify(b).map(|c| c.identity_pk))
            .collect::<Result<Vec<_>, _>>()?;
        let member = *ids.first().ok_or_else(|| CoreError::Membership("no contact bundle".into()))?;
        if ids.iter().any(|i| *i != member) {
            return Err(CoreError::Membership("the bundles name more than one identity".into()));
        }
        // The Site and its parts as the relay holds them now, not as the last poll left them
        // (TEST run 77): the rooms' marks, the admitter grants and the spent claims.
        self.drain_site(site_hex).await?;
        // THE JOINER'S INTRO MAILBOX, where each Welcome is sealed, and each history after it
        // (O-75).
        let intro_tag = handshake::parse_and_verify(&bundles[0])?.intro_tag;
        let mut history = Vec::new();
        let mut send = |r: Result<HistorySent, CoreError>, object: &str| {
            history.push(r.unwrap_or_else(|e| HistorySent { object: object.to_string(), unsent: Some(e.to_string()), ..Default::default() }))
        };
        let state = self.group_state(site_hex)?;
        let mut keys = stand_in.clone();
        for (kid, key) in &state.claim_issuers {
            if let Some(k) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(key).ok().and_then(|b| b.try_into().ok()) {
                keys.insert(kid.clone(), k);
            }
        }
        let claim = crate::claim::verify(token, &keys, site_hex, unix_now() as u64).map_err(CoreError::Membership)?;
        let spent = claim.spent_key();
        let mut packages = bundles.iter();
        match state.claims_spent.get(&spent) {
            // A RETRY: the member this claim admitted, still on the Site, completing
            // their rooms. Nothing is spent again.
            Some(m) if *m == member && self.object_members(site_hex).is_ok_and(|on| on.contains(&member)) => {}
            Some(_) => return Err(CoreError::Membership("this claim has been used".into())),
            None => {
                // A MEMBER'S SECOND CLAIM (TEST, 1 Oct): already on the Site, so not added again
                // (MLS refuses a second leaf for one identity, and the Arc answered 503 forever);
                // the claim is still spent, by them, and the rooms of its choice follow below.
                let added = if self.object_members(site_hex).is_ok_and(|on| on.contains(&member)) {
                    member
                } else {
                    let added = self.group_add_member(site_hex, packages.next().expect("one bundle at least")).await?;
                    send(self.send_history(site_hex, &member, &intro_tag).await, site_hex);
                    added
                };
                let mut args = coordinator::Args::new();
                args.insert("claim".into(), coordinator::ArgVal::Text(spent));
                args.insert("member".into(), coordinator::ArgVal::Text(hex::encode(added)));
                args.insert("at".into(), coordinator::ArgVal::Int(unix_millis() as i64));
                if let Some(c) = &claim.choice {
                    args.insert("choice".into(), coordinator::ArgVal::Text(c.clone()));
                }
                if let Some(a) = &claim.artifact {
                    args.insert("share".into(), coordinator::ArgVal::Text(a.clone()));
                }
                self.write_record(site_hex, crate::membership::OP_CLAIM_SPENT, args).await?;
            }
        }
        // THE SITE'S ROOMS (D-58): each part a founder made this Node an admitter of. Rooms
        // by the claim's choice (ICD 2.1.0 row 3): those carrying its `c`; when none does, or
        // it has none, those without a choice. A part in any other role, on every claim.
        let me = self.id.identity_pk();
        let mine: Vec<(&String, &crate::object::PartRef)> = state.parts.iter().filter(|(p, _)| self.is_admitter(p, &me)).collect();
        let room = |r: &crate::object::PartRef| r.role == crate::parts::ROOM_ROLE;
        let chosen = claim.choice.as_ref().filter(|c| mine.iter().any(|(_, r)| room(r) && r.choice.as_ref() == Some(*c)));
        let fallback = if chosen.is_none() { claim.choice.clone() } else { None };
        let (mut rooms, mut unjoined) = (Vec::new(), Vec::new());
        for part in mine.iter().filter(|(_, r)| !room(r) || r.choice.as_ref() == chosen).map(|(p, _)| *p) {
            if self.object_members(part).is_ok_and(|on| on.contains(&member)) {
                rooms.push(part.clone());
                continue;
            }
            match packages.next() {
                None => unjoined.push((part.clone(), "no key package left for it; present the claim again".into())),
                Some(b) => match self.group_add_member(part, b).await {
                    Ok(_) => {
                        // A room's history; never another part's (a Host's items are not a room's).
                        if state.parts.get(part).is_some_and(|r| r.role == crate::parts::ROOM_ROLE) {
                            send(self.send_history(part, &member, &intro_tag).await, part);
                        }
                        rooms.push(part.clone())
                    }
                    Err(e) => unjoined.push((part.clone(), e.to_string())),
                },
            }
        }
        Ok(Admitted { member, choice: claim.choice, artifact: claim.artifact, rooms, unjoined, fallback, history })
    }

    /// O-75: THE HISTORY OF ONE OBJECT, as this Node holds it, for `joiner`, whom it has just
    /// admitted: its stored rows (`history::Row`), each unaltered with its author and the
    /// author's own signature (A-10). DURABLE STATE FIRST: the owner's sequenced spine, whole
    /// and in chain order (epoch, seq), and the membership records beside it, since the
    /// joiner's check that an author was a member when writing reads them; then every other
    /// row, newest first by gen, to what one relay blob carries (`HISTORY_MAX_B64`, on the
    /// sealed blob). A spine that is not whole cannot go as a chain: an unsigned row of it, or
    /// a spine alone over the blob, sends nothing, named. An unsigned row of the rest is left
    /// out and counted. Only a Site (group) or a room (forum) has one: never a Host's items,
    /// a DM or a tether. The bundle is signed by this Node, the admitter (`history::sign`).
    pub fn history_of(&self, object_hex: &str, joiner: &[u8; 32], intro_tag: &[u8; 32]) -> Result<(Option<crate::history::Bundle>, HistorySent), CoreError> {
        use crate::history::Row;
        let kind = self.object_kind(object_hex)?;
        if kind != "group" && kind != "forum" {
            return Err(CoreError::Membership(format!("a {kind} carries no history (O-75: the Site and its rooms)")));
        }
        let group_id = self.object_group_id(object_hex)?;
        let mut report = HistorySent { object: object_hex.to_string(), ..Default::default() };
        let (mut spine, mut records, mut rest) = (Vec::new(), Vec::new(), Vec::new());
        let mut spine_unsigned = 0usize;
        for (author, envelope, sig) in self.dir.load_log_signed(&group_id)? {
            let delta = coordinator::decode_delta(&envelope)?;
            let (id, gen) = (delta.id(), delta.gen.unwrap_or(0));
            match (delta.seq, sig) {
                (Some(seq), Some(sig)) => spine.push(((delta.epoch, seq), Row { envelope, author, sig })),
                (Some(_), None) => spine_unsigned += 1,
                (None, None) => report.left += 1,
                (None, Some(sig)) if crate::membership::is_membership_op(delta.op_id) => records.push(((gen, author, id), Row { envelope, author, sig })),
                // A card travels as its author's latest, in the card bundles (`cards_of`).
                (None, Some(_)) if crate::profiles::is_profile_op(delta.op_id) => {}
                (None, Some(sig)) => rest.push(((gen, author, id), Row { envelope, author, sig })),
            }
        }
        let sealed_len = |rows: &[Row]| -> Result<usize, CoreError> {
            let b = crate::history::sign(&self.id, &group_id, joiner, rows.to_vec());
            Ok(pacific_wire::blob_b64(&seal::seal(&crate::history::encode(&b), intro_tag, intro_tag)?).len())
        };
        if spine_unsigned > 0 {
            report.unsent = Some(format!("{spine_unsigned} row(s) of its spine carry no signature, so the chain cannot be proved"));
            return Ok((None, report));
        }
        spine.sort_by(|a, b| a.0.cmp(&b.0));
        records.sort_by(|a, b| a.0.cmp(&b.0));
        rest.sort_by(|a, b| b.0.cmp(&a.0));
        let mut rows: Vec<Row> = spine.into_iter().map(|(_, r)| r).collect();
        report.spine = rows.len();
        let whole = sealed_len(&rows)?;
        if whole > HISTORY_MAX_B64 {
            report.unsent = Some(format!("its spine alone is {whole} characters sealed, over the {HISTORY_MAX_B64} one blob holds"));
            report.spine = 0;
            return Ok((None, report));
        }
        // What fits, estimated from each row's own size, then made exact by the sealed blob.
        let mut room = HISTORY_MAX_B64 - whole;
        let mut stopped = false;
        for (is_record, tier) in [(true, records), (false, rest)] {
            for (_, r) in tier {
                let cost = (r.payload().len() + 8) * 4 / 3 + 8;
                if stopped || cost > room {
                    stopped = true;
                    report.left += 1;
                    continue;
                }
                room -= cost;
                rows.push(r);
                if is_record {
                    report.records += 1
                } else {
                    report.posts += 1
                }
            }
        }
        while rows.len() > report.spine && sealed_len(&rows)? > HISTORY_MAX_B64 {
            rows.pop();
            if report.posts > 0 {
                report.posts -= 1
            } else {
                report.records -= 1
            }
            report.left += 1;
        }
        report.sealed = sealed_len(&rows)?;
        Ok((Some(crate::history::sign(&self.id, &group_id, joiner, rows)), report))
    }

    /// O-75 CARDS (O-77: "All members of a community are required to publish a profile card
    /// on entry"): each CURRENT member's latest `base.publishProfile` row on the object (its
    /// roster now, the highest gen per author, never a departed member, never the joiner's
    /// own), unaltered, newest first, packed into `history` bundles each within `blob` bytes
    /// encoded (never more than `history::CAP`, nor the relay's sealed measure), each signed
    /// by this Node, to at most `total` bytes of encoded bundles. Answers the bundles, the
    /// cards in them, and the cards that did not go. `send_history` passes `history::CAP`
    /// and `CARDS_MAX`.
    pub fn cards_of(&self, object_hex: &str, joiner: &[u8; 32], intro_tag: &[u8; 32], blob: usize, total: usize) -> Result<(Vec<crate::history::Bundle>, usize, usize), CoreError> {
        use crate::history::{encode, sign, Row, CAP};
        let group_id = self.object_group_id(object_hex)?;
        let roster: std::collections::HashSet<[u8; 32]> = self.object_members(object_hex)?.into_iter().collect();
        let mut latest: std::collections::HashMap<[u8; 32], ((u64, [u8; 32]), Row)> = std::collections::HashMap::new();
        for (author, envelope, sig) in self.dir.load_log_signed(&group_id)? {
            let Some(sig) = sig else { continue };
            if author == *joiner || !roster.contains(&author) {
                continue;
            }
            let delta = coordinator::decode_delta(&envelope)?;
            if !crate::profiles::is_profile_op(delta.op_id) {
                continue;
            }
            let key = (delta.gen.unwrap_or(0), delta.id());
            if latest.get(&author).is_none_or(|(k, _)| key > *k) {
                latest.insert(author, (key, Row { envelope, author, sig }));
            }
        }
        let mut cards: Vec<((u64, [u8; 32]), Row)> = latest.into_values().collect();
        cards.sort_by(|a, b| b.0.cmp(&a.0));
        let fits = |rows: &[Row]| -> Result<Option<usize>, CoreError> {
            let plain = encode(&sign(&self.id, &group_id, joiner, rows.to_vec()));
            let sealed = pacific_wire::blob_b64(&seal::seal(&plain, intro_tag, intro_tag)?).len();
            Ok((plain.len() <= blob.min(CAP) && sealed <= HISTORY_MAX_B64).then_some(plain.len()))
        };
        let (mut bundles, mut current, mut used, mut sent, mut unsent) = (Vec::new(), Vec::<Row>::new(), 0usize, 0usize, 0usize);
        let mut full = false;
        for (_, card) in cards {
            if full {
                unsent += 1;
                continue;
            }
            let mut with = current.clone();
            with.push(card.clone());
            if fits(&with)?.is_some() {
                current = with;
                continue;
            }
            if current.is_empty() {
                unsent += 1; // one card alone over a blob
                continue;
            }
            // Close the bundle that is full, if the admission's total still holds it.
            let bytes = fits(&current)?.unwrap_or(usize::MAX);
            if used + bytes > total {
                unsent += current.len() + 1;
                current.clear();
                full = true;
                continue;
            }
            used += bytes;
            sent += current.len();
            bundles.push(sign(&self.id, &group_id, joiner, std::mem::take(&mut current)));
            if fits(std::slice::from_ref(&card))?.is_some() {
                current.push(card);
            } else {
                unsent += 1;
            }
        }
        if !current.is_empty() {
            let bytes = fits(&current)?.unwrap_or(usize::MAX);
            if used + bytes > total {
                unsent += current.len();
            } else {
                sent += current.len();
                bundles.push(sign(&self.id, &group_id, joiner, current));
            }
        }
        Ok((bundles, sent, unsent))
    }

    /// O-75: send `object_hex`'s history (`history_of`) to a joiner's intro mailbox, after its
    /// Welcome, through `publish_history`, which seals it as the Welcome is sealed. Nothing is
    /// sent when nothing can go, and the reason is named in this Node's journal and in the
    /// report.
    pub async fn send_history(&self, object_hex: &str, joiner: &[u8; 32], intro_tag: &[u8; 32]) -> Result<HistorySent, CoreError> {
        let plan = self.history_planned(object_hex, joiner, intro_tag)?;
        self.send_planned(object_hex, intro_tag, plan).await
    }

    /// [`Self::send_history`]'s publish, of what [`Self::history_planned`] made.
    async fn send_planned(&self, object_hex: &str, intro_tag: &[u8; 32], plan: HistoryPlan) -> Result<HistorySent, CoreError> {
        let (bundle, cards, report) = plan;
        if let Some(why) = &report.unsent {
            tracing::warn!(target: "pacific::history", object = %object_hex, "no history sent: {why}");
        }
        if let Some(bundle) = &bundle {
            // THE ONE PUBLISH PATH (BW-D's): sealed as the Welcome is, held to the cap and the relay.
            self.publish_history(intro_tag, bundle).await?;
        }
        // THE MEMBERS' CARDS, after the spine's bundle, each bundle on its own; they need no
        // chain, so they go whether the spine could or not.
        for b in &cards {
            self.publish_history(intro_tag, b).await?;
        }
        if report.cards_unsent > 0 {
            tracing::warn!(target: "pacific::history", object = %object_hex, "{} card(s) not sent", report.cards_unsent);
        }
        Ok(report)
    }

    /// What [`Self::send_history`] sends: the spine's bundle (`history_of`), the card bundles
    /// (`cards_of`), and the report of both.
    fn history_planned(
        &self,
        object_hex: &str,
        joiner: &[u8; 32],
        intro_tag: &[u8; 32],
    ) -> Result<HistoryPlan, CoreError> {
        let (bundle, mut report) = self.history_of(object_hex, joiner, intro_tag)?;
        let (cards, sent, unsent) = self.cards_of(object_hex, joiner, intro_tag, crate::history::CAP, cards_max()?)?;
        (report.cards, report.card_bundles, report.cards_unsent) = (sent, cards.len(), unsent);
        let bundle = bundle.filter(|b| !b.rows.is_empty());
        if bundle.is_none() {
            report.sealed = 0;
        }
        Ok((bundle, cards, report))
    }

    /// THE OWNER SEALS AN OBJECT'S HISTORY TO A MEMBER (Ralph, 30 Sep: every joiner into every
    /// room). The Arc holds only rows it has seen, so a room its founder adds it to late gives it
    /// none of the room's log, and none reaches a joiner it admits: the founder sends the Arc the
    /// log (`send_history`, this Node the sender), the Arc takes it (`take_history`: the owner has
    /// standing), and passes it on. `bundle` is the member's contact bundle: whom, and their
    /// intro mailbox. Only the object's owner, only a Site or a room, and only to a member on
    /// its roster; `dry` sends nothing and answers what would go, to anyone's bundle.
    pub async fn send_history_as_owner(&self, object_hex: &str, bundle: &str, dry: bool) -> Result<HistorySent, CoreError> {
        let contact = handshake::parse_and_verify(bundle)?;
        if self.object_owner(object_hex)?.as_deref() != Some(&self.id.identity_pk()[..]) {
            return Err(CoreError::Membership(format!("only {object_hex}'s owner sends its history")));
        }
        let plan = self.history_planned(object_hex, &contact.identity_pk, &contact.intro_tag)?;
        if dry {
            return Ok(plan.2);
        }
        if !self.object_members(object_hex)?.contains(&contact.identity_pk) {
            return Err(CoreError::Membership(format!("{} is not a member of {object_hex}: add them first", hex::encode(contact.identity_pk))));
        }
        self.send_planned(object_hex, &contact.intro_tag, plan).await
    }

    /// Drain a Site and the parts it names over the home session: one pipelined round, and a
    /// second only for parts the first revealed. Groups on another relay are left to it.
    async fn drain_site(&self, site_hex: &str) -> Result<(), CoreError> {
        let home = self.routes.to_spec();
        let ours = |id: &String| self.object_group_id(id).ok().filter(|g| self.routes_for(g).to_spec() == home);
        let parts = |n: &Node| -> Result<std::collections::BTreeSet<String>, CoreError> {
            Ok(n.group_state(site_hex)?.parts.into_keys().collect())
        };
        let before = parts(self)?;
        let mut sess = self.home_session().await?;
        let drained = async {
            let first = std::iter::once(site_hex.to_string()).chain(before.iter().cloned()).filter_map(|id| ours(&id)).collect();
            self.drain_groups_at_once(&mut sess, &first).await?;
            let revealed = parts(self)?.difference(&before).filter_map(ours).collect();
            self.drain_groups_at_once(&mut sess, &revealed).await
        }
        .await;
        match drained {
            Ok(()) => {
                self.keep_session(sess);
                Ok(())
            }
            Err(e) => {
                sess.close().await;
                Err(e)
            }
        }
    }

    /// A record that rides an MLS operation this Node is making: built here, not
    /// through `apply`, which refuses these for every caller.
    async fn write_record(&self, object_id_hex: &str, op_id: u32, mut args: coordinator::Args) -> Result<String, CoreError> {
        // NOT THROUGH `apply`, and that is the point: `authoring::build` refuses a
        // membership op for every caller (`Refusal::ByTheMlsDoors`), because a
        // record with no MLS operation behind it is the defect that rule exists to
        // stop. This is one of the paths that HAS the operation — the commit is
        // being made around this call — so it builds the delta itself.
        let group_id = self.object_group_id(object_id_hex)?;
        let decl = GroupType::op(op_id)
            .ok_or_else(|| CoreError::Coordinator(format!("group has no op {op_id}")))?;
        let (sid, sk, _name) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut group = self.load_member_group(&client, &group_id)?;
        let epoch = group.current_epoch();
        let delta = match decl.commutativity {
            Commutativity::Commutative => {
                // The Lamport rule, as `apply` and `post_to_group` use it — never
                // the per-author count the typed paths used to stamp.
                let gen = self.next_lamport(&group_id)?;
                args.insert("gen".to_string(), coordinator::ArgVal::Int(gen as i64));
                crate::object::build_delta(ObjectKind::Group, op_id, args, epoch, Some(gen))
            }
            Commutativity::Sequenced => {
                let (seq, prev) = self.group_next_seq(&group_id, epoch)?;
                coordinator::sequenced_delta(
                    ObjectKind::Group.type_id() as u32,
                    op_id,
                    args,
                    epoch,
                    seq,
                    prev,
                )
            }
        };
        let delta_id = self.append_and_flush(&mut group, &group_id, delta).await?;
        Ok(hex::encode(delta_id))
    }

    /// The founder's tenure, written at creation into a solo log (§10.3) — so a
    /// Group-typed object records who is in it from its first delta, in core, rather
    /// than by each app.
    fn record_founder(&self, group_id: &[u8], epoch: u64) -> Result<(), CoreError> {
        let me = self.id.identity_pk();
        let gen = self.dir.author_delta_count(group_id, &me)?;
        let mut args = coordinator::Args::new();
        args.insert("member".into(), coordinator::ArgVal::Text(hex::encode(me)));
        args.insert("at".into(), coordinator::ArgVal::Int(unix_millis() as i64));
        args.insert("gen".into(), coordinator::ArgVal::Int(gen as i64));
        let delta = crate::object::build_delta(
            ObjectKind::Group,
            crate::membership::OP_MEMBER_JOINED,
            args,
            epoch,
            Some(gen),
        );
        let envelope = delta.canonical_bytes();
        let id = delta.id();
        self.dir.append_delta_signed(
            group_id,
            &id,
            &me,
            &envelope,
            Some(&crate::delta_sig::sign_delta(&self.id, group_id, &id)),
            unix_now(),
        )?;
        Ok(())
    }

    /// LEAVE (§6.1). Record the departure (on Group kinds), flush it, THEN propose the
    /// removal of every leaf of this person — once this device holds its own proposal,
    /// mls-rs refuses to encrypt anything else. Another member commits it (§7); this
    /// device learns it left from the removing commit (§8.3). Idempotent while a leave
    /// is in flight.
    pub async fn group_leave(&self, object_id_hex: &str) -> Result<(), CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        self.refuse_if_departed(&group_id)?;
        if self.dir.pending_departure(&group_id)?.is_some() {
            tracing::info!(target: "pacific::membership", object = %object_id_hex,
                           "already leaving — nothing to do");
            return Ok(());
        }
        let me = self.id.identity_pk();
        {
            let (sid, sk, _n) = self.signer()?;
            let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
            let group = mls::load_group(&client, &group_id)?;
            let people: std::collections::BTreeSet<[u8; 32]> =
                mls::roster_identities(&group)?.into_iter().collect();
            if !people.contains(&me) {
                return Err(CoreError::Membership("you are not a member of this group".into()));
            }
            if people.len() < 2 {
                return Err(CoreError::Membership(
                    "you are the only one here — nobody could complete your leave; delete the object instead".into(),
                ));
            }
            if mls::group_owner(&group)? == me {
                return Err(CoreError::Membership(
                    "the owner must hand the object over before leaving".into(),
                ));
            }
            if !mls::has_owner_ext(&group) {
                return Err(CoreError::Membership(
                    "this group predates the owner extension and migrates on its owner's next sync — try again after that".into(),
                ));
            }
        }
        let kind = self.dir.group_kind(&group_id)?.unwrap_or_default();
        if group::is_group_typed(&kind) {
            self.write_membership_record(object_id_hex, crate::membership::OP_MEMBER_LEFT, &me, None)
                .await?;
        }
        let mut sess = Router::open(&self.routes_for(&group_id)).await?;
        let r = self.propose_my_departure(&mut sess, &group_id).await;
        sess.close().await;
        let epoch = r?;
        tracing::info!(target: "pacific::membership", object = %object_id_hex, epoch,
                       "leaving: the removal is proposed and waits for another member to commit it");
        Ok(())
    }

    /// REMOVE (§5) — the owner removes every device of `member_hex`. `reason` is one of
    /// `requested | lost_device | inactive | conduct | other` (amendment 8). The record
    /// goes first, at the epoch the removed person can still read.
    pub async fn group_remove_member(
        &self,
        object_id_hex: &str,
        member_hex: &str,
        reason: Option<&str>,
    ) -> Result<(), CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        self.refuse_if_departed(&group_id)?;
        let member = Self::parse_member(member_hex)?;
        if let Some(r) = reason {
            crate::membership::RemovalReason::parse(r).ok_or_else(|| {
                CoreError::Membership(format!(
                    "unknown removal reason '{r}' — one of requested, lost_device, inactive, conduct, other"
                ))
            })?;
        }
        let me = self.id.identity_pk();
        {
            let (sid, sk, _n) = self.signer()?;
            let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
            let group = mls::load_group(&client, &group_id)?;
            if mls::group_owner(&group)? != me {
                return Err(CoreError::Membership("only the owner may remove someone".into()));
            }
            if !mls::has_owner_ext(&group) {
                return Err(CoreError::Membership(
                    "this group predates the owner extension; it migrates on your next sync — try again after that".into(),
                ));
            }
            if member == me {
                return Err(CoreError::Membership(
                    "the owner cannot remove themselves — hand the object over first".into(),
                ));
            }
            if mls::leaves_of(&group, &member).is_empty() {
                return Err(CoreError::Membership(format!(
                    "{} is not a member of this group",
                    hex::encode(&member[..6])
                )));
            }
        }
        let kind = self.dir.group_kind(&group_id)?.unwrap_or_default();
        if group::is_group_typed(&kind) {
            let m = hex::encode(member);
            let already = self
                .folded_group(&group_id)
                .map(|c| {
                    let log = c.state().membership;
                    !log.is_present(&m) && log.departed(&m).is_some()
                })
                .unwrap_or(false);
            if !already {
                self.write_membership_record(object_id_hex, crate::membership::OP_MEMBER_LEFT, &member, reason)
                    .await?;
            }
        }
        let mut sess = Router::open(&self.routes_for(&group_id)).await?;
        let r = self
            .commit_via_slot(&mut sess, &group_id, "remove", 8, |g| {
                let leaves = mls::leaves_of(g, &member);
                if leaves.is_empty() {
                    return Ok(None);
                }
                mls::stage_remove(g, &leaves).map(Some)
            })
            .await;
        sess.close().await;
        r?;
        tracing::info!(target: "pacific::membership", object = %object_id_hex,
                       member = %hex::encode(member), reason = ?reason, "removed");
        Ok(())
    }

    /// HAND OVER (§9) — the owner makes `member_hex` the owner. The record rides the spine
    /// at the epoch this device still owns; the GroupContext commit is what makes it true,
    /// and every member records the new owner from the epoch that commit starts (§10.2).
    pub async fn group_hand_over(&self, object_id_hex: &str, member_hex: &str) -> Result<(), CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        self.refuse_if_departed(&group_id)?;
        let member = Self::parse_member(member_hex)?;
        let me = self.id.identity_pk();
        {
            let (sid, sk, _n) = self.signer()?;
            let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
            let group = mls::load_group(&client, &group_id)?;
            if mls::group_owner(&group)? != me {
                return Err(CoreError::Membership("only the owner may hand the object over".into()));
            }
            if member == me {
                return Err(CoreError::Membership("you already own it".into()));
            }
            if mls::leaves_of(&group, &member).is_empty() {
                return Err(CoreError::Membership("a handover must name someone in the group".into()));
            }
            if !mls::has_owner_ext(&group) {
                return Err(CoreError::Membership(
                    "this group predates the owner extension; it migrates on your next sync — try again after that".into(),
                ));
            }
        }
        let kind = self.dir.group_kind(&group_id)?.unwrap_or_default();
        if group::is_group_typed(&kind) {
            self.write_membership_record(object_id_hex, crate::membership::OP_OWNER_HANDOVER, &member, None)
                .await?;
        }
        let mut sess = Router::open(&self.routes_for(&group_id)).await?;
        let r = self
            .commit_via_slot(&mut sess, &group_id, "hand over", 8, |g| {
                if mls::group_owner(g)? == member {
                    return Ok(None);
                }
                mls::stage_set_owner(g, &member).map(Some)
            })
            .await;
        sess.close().await;
        r?;
        tracing::info!(target: "pacific::membership", object = %object_id_hex,
                       to = %hex::encode(member), "handed over");
        Ok(())
    }

    /// How the membership RECORDS and the MLS tree disagree (§10.3) — `None` when they
    /// agree. Only Group-typed objects carry the records.
    pub fn membership_divergence(
        &self,
        object_id_hex: &str,
    ) -> Result<Option<crate::membership::Divergence>, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        let kind = self.dir.group_kind(&group_id)?.unwrap_or_default();
        if !group::is_group_typed(&kind) {
            return Ok(None);
        }
        let roster = self.dir.group_members(&group_id)?;
        Ok(self.folded_group(&group_id)?.state().membership.divergence(&roster))
    }

    /// Is a leave in flight for this object (§6)?
    pub fn is_leaving(&self, object_id_hex: &str) -> Result<bool, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        Ok(self.dir.pending_departure(&group_id)?.is_some())
    }

    /// Has a commit removed this device from the object (§8.3)?
    pub fn is_departed(&self, object_id_hex: &str) -> Result<bool, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        self.dir.is_departed(&group_id)
    }

    /// Membership upkeep, run by every sync of every group (§6.2, §6.4, §7, §11, §3.3).
    /// Never fails the sync: each problem is logged and the next sync tries again.
    async fn membership_upkeep(&self, sess: &mut Router, group_id: &[u8]) {
        if let Err(e) = self.membership_upkeep_at(sess, group_id, unix_now()).await {
            tracing::warn!(target: "pacific::membership", group = %hex::encode(group_id),
                           error = %e, "membership upkeep failed; the next sync tries again");
        }
    }

    /// [`Self::membership_upkeep`] at a given `now` (unix seconds) — the clock is a
    /// parameter so a test can move it.
    pub async fn membership_upkeep_at(
        &self,
        sess: &mut Router,
        group_id: &[u8],
        now: i64,
    ) -> Result<(), CoreError> {
        if self.dir.is_departed(group_id)? {
            return Ok(());
        }
        let me = self.id.identity_pk();
        let (sid, sk, _n) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let group = mls::load_group(&client, group_id)?;
        let leaves = group.roster().members_iter().count();

        // §6.4 — another device of this person proposed its removal: this one goes too.
        // Only a proposal from this person's own device counts; a Remove of us that
        // anyone else proposed is not us leaving.
        if mls::pending_leave_of(&group, &me) && self.dir.pending_departure(group_id)?.is_none() {
            self.dir
                .set_pending_departure(group_id, group.current_epoch(), now)?;
            tracing::info!(target: "pacific::membership", group = %hex::encode(group_id),
                           "another device of this person is leaving; this one goes with it");
        }
        // §6.2 — a leave in flight whose proposals lapsed with their epoch.
        if let Some(at) = self.dir.pending_departure(group_id)? {
            if group.current_epoch() > at && !mls::pending_leave_of(&group, &me) {
                drop(group);
                let e = self.propose_my_departure(sess, group_id).await?;
                tracing::info!(target: "pacific::membership", group = %hex::encode(group_id),
                               epoch = e, "leave proposed again — the last proposal lapsed with its epoch");
            }
            // A leaving device neither commits nor self-updates.
            return Ok(());
        }
        // The owner proposed our removal by reference: any commit of ours would carry
        // it and remove its own committer, which RFC 9420 §12.2 forbids. Wait for
        // another member to commit it.
        if mls::pending_removal_of(&group, &me) {
            tracing::info!(target: "pacific::membership", group = %hex::encode(group_id),
                           "the owner proposed this person's removal — waiting for it to be committed");
            return Ok(());
        }
        drop(group);

        // §7 — complete what others proposed.
        self.commit_pending(sess, group_id).await?;
        if leaves <= 1 {
            return Ok(());
        }

        let group = mls::load_group(&client, group_id)?;
        // §11.3 only: an older build's leaf commits once to advertise the owner
        // extension. The weekly self-update of §11.2 is gone (19 Sep ruling): PCS is
        // already forgone under retained epochs, so it was an epoch per group per week
        // buying nothing.
        let refresh = !mls::my_leaf_supports_owner_ext(&group);
        let migrate = !mls::has_owner_ext(&group)
            && mls::group_owner(&group)? == me
            && mls::every_leaf_supports_owner_ext(&group);
        drop(group);

        if migrate {
            // §3.3 — the leaf-0 owner writes itself into the context.
            let done = self
                .commit_via_slot(sess, group_id, "owner migration", 8, |g| {
                    if mls::has_owner_ext(g) {
                        return Ok(None);
                    }
                    mls::stage_set_owner(g, &me).map(Some)
                })
                .await?;
            if done {
                tracing::info!(target: "pacific::membership", group = %hex::encode(group_id),
                               "legacy group migrated: its owner now lives in the context");
            }
        } else if refresh {
            // One round: a lost slot means someone else committed, and the next sync
            // tries again.
            match self
                .commit_via_slot(sess, group_id, "capability refresh", 1, |g| mls::stage_rekey(g).map(Some))
                .await
            {
                Ok(_) | Err(CoreError::CommitRace(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Persist the tag + seal secret of the epoch `group` is at now, so a later
    /// sync can re-drain it — the exporter yields only the current epoch, and we
    /// have just moved to it.
    fn record_current_epoch_tag(&self, group_id: &[u8], group: &mls::Group) -> Result<(), CoreError> {
        let e = group.current_epoch();
        let eb = mls::epoch_be(e);
        let tag = mls::group_tag(group, &eb)?;
        let secret = mls::seal_conn_secret(group, &eb)?;
        self.dir.record_epoch_tag(group_id, e, &tag, &secret)
    }

    fn object_group_id(&self, object_id_hex: &str) -> Result<Vec<u8>, CoreError> {
        hex::decode(object_id_hex).map_err(|_| CoreError::Directory("bad object id".into()))
    }

    /// The group's owner identity pubkey (recorded at `put_group`), as a fixed
    /// array. Loud if the row has no owner — every object/connection group sets one.
    fn group_owner_id(&self, group_id: &[u8]) -> Result<[u8; 32], CoreError> {
        let owner = self.dir.group_owner(group_id)?.ok_or_else(|| {
            CoreError::Directory(format!("group {} has no owner", hex::encode(group_id)))
        })?;
        owner
            .as_slice()
            .try_into()
            .map_err(|_| CoreError::Directory("owner_pk width".into()))
    }

    // ======================================================================
    // ARC — every object is governed by ONE Arc (moderation/reports/safety),
    // and its MLS traffic routes through THAT Arc's Semaphore relay. Whoever
    // creates the object stamps their default Arc as its governing Arc; the
    // roster can move it elsewhere by supermajority (`swap_group_arc`). This is
    // cultural federation: the object lives under the jurisdiction of a chosen Arc.
    // ======================================================================

    /// This device's default Arc — the ws endpoint we stamp onto objects we create,
    /// read from the `arc_url` state file (else the built-in default). An Arc co-hosts
    /// its Semaphore relay at the same static IP, so this doubles as the relay we route
    /// our own objects' traffic through. A persisted Arc this build has retired is
    /// passed over for the default, as the relay setting is.
    fn my_default_arc(&self) -> String {
        std::fs::read_to_string(paths::arc_url_path())
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && !paths::is_retired_relay_url(s))
            .unwrap_or_else(|| paths::DEFAULT_ARC_URL.to_string())
    }

    /// Set this device's default Arc: the route `object_new` stamps and the Welcomes
    /// for those objects carry. What fails the route check is refused by name, and
    /// nothing is written (SECURITY, 27 Sep).
    pub fn set_default_arc(&self, url: &str) -> Result<(), CoreError> {
        set_default_arc(url)
    }

    /// The relay an object's traffic routes through. An Arc co-hosts its Semaphore
    /// relay at its static IP, so an object RE-HOMED to a different Arc routes to that
    /// Arc's endpoint. An object governed by our OWN default Arc (the common case) —
    /// or one whose Arc we have not learned yet — routes through `fallback`, the
    /// device relay that hosts our own Arc's Semaphore. Never silently drops an
    /// object, never fabricates an Arc.
    ///
    /// An object stamped with a RETIRED relay routes through `fallback` too: the
    /// stamp was written once, at creation, and outlives every setting a migration
    /// can reach, so without this a group made before a move would keep dialling the
    /// host it moved off (paths.rs, RETIRED_RELAY_URLS).
    fn routes_for(&self, group_id: &[u8]) -> crate::router::Routes {
        match self.dir.group_arc(group_id) {
            Ok(Some(arc))
                if !arc.is_empty()
                    && arc != self.my_default_arc()
                    && !paths::is_retired_relay_url(&arc) =>
            {
                self.routes.with_relay(&arc)
            }
            _ => self.routes.clone(),
        }
    }

    /// This device's transport set.
    pub fn routes(&self) -> &crate::router::Routes {
        &self.routes
    }

    /// Point this device's messaging at a new transport set, and persist it. The ONE way transport
    /// changes — there is no per-call override, deliberately: a message that quietly went somewhere
    /// other than where you pointed it is the failure this whole refactor exists to make impossible.
    pub fn set_routes(&mut self, routes: crate::router::Routes) -> Result<(), CoreError> {
        routes.save()?;
        self.routes = routes;
        // A session held for the old routes goes to the old place.
        *self.held.get_mut().unwrap_or_else(|p| p.into_inner()) = None;
        Ok(())
    }

    /// The Arc that governs an object (its moderation/reports/safety authority), or
    /// `None` if we have not learned it yet.
    pub fn object_arc(&self, object_id_hex: &str) -> Result<Option<String>, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        self.dir.group_arc(&group_id)
    }

    /// Swap the Arc that governs `object_id_hex` — its moderation/safety jurisdiction
    /// and the relay its traffic routes through. Requires a SUPERMAJORITY (⌈2n/3⌉) of
    /// the object's current MLS roster to approve, so no single member — not even the
    /// owner — can move an object into a different cultural jurisdiction unilaterally.
    /// `approvals` are the identity pubkeys that approved; they are deduped and
    /// intersected with the live roster before the count, so stale or non-member
    /// approvals cannot pad the tally. Loud on an unmet threshold or a malformed Arc
    /// URL — never a silent partial swap.
    pub fn swap_group_arc(
        &self,
        object_id_hex: &str,
        new_arc: &str,
        approvals: &[[u8; 32]],
    ) -> Result<(), CoreError> {
        if !is_arc_url(new_arc) {
            return Err(CoreError::Governance(format!(
                "not an Arc url (wss://, or ws:// to a local address): {new_arc}"
            )));
        }
        let group_id = self.object_group_id(object_id_hex)?;
        let roster = self.dir.group_members(&group_id)?;
        if roster.is_empty() {
            return Err(CoreError::Governance("object has no roster".into()));
        }
        let roster_set: std::collections::HashSet<[u8; 32]> = roster.iter().copied().collect();
        let approved = approvals
            .iter()
            .copied()
            .filter(|a| roster_set.contains(a))
            .collect::<std::collections::HashSet<[u8; 32]>>()
            .len();
        let needed = supermajority_threshold(roster.len());
        if approved < needed {
            return Err(CoreError::Governance(format!(
                "arc swap needs {needed}/{} (supermajority); have {approved}",
                roster.len()
            )));
        }
        self.dir.set_group_arc(&group_id, new_arc)?;
        Ok(())
    }

    /// THE WIRE TYPE THIS OBJECT'S CHAT DELTAS TAKE — which is NOT the object's
    /// own type.
    ///
    /// An Event folds as an Event and carries a FORUM discussion beside it; a
    /// connection folds as a Forum and carries CONTACT prekeys beside that. One
    /// log, many op-groups, and the chat op-group is Forum on every kind but one:
    /// a `conversation` is its own object with its own type (the fork, 24 Sep
    /// 2026). The fold filters on exactly this, so a delta written under the
    /// wrong one is stored and then invisible.
    fn chat_type_id(&self, group_id: &[u8]) -> Result<u32, CoreError> {
        let kind = self
            .dir
            .group_kind(group_id)?
            .ok_or_else(|| CoreError::Directory(format!("no object {}", hex::encode(group_id))))?;
        match crate::fold::lens_for(&kind) {
            Some(ObjectKind::Conversation) => Ok(ObjectKind::Conversation.type_id() as u32),
            // A pairing channel carries prekeys and profiles, not messages. Writing
            // a Forum delta here is what the migration ended, and it must refuse
            // rather than store something no fold will ever read.
            Some(ObjectKind::Contact) => Err(CoreError::Coordinator(format!(
                "'{kind}' carries no messages — its conversation does"
            ))),
            _ => Ok(coordinator::FORUM_TYPE_ID),
        }
    }

    /// Fold a forum/connection object's log into a `Coordinator<ForumType>`. The
    /// single shared projection behind every Forum read (transcript/detailed/thread/
    /// title) and the sequenced-seq allocator. Deltas are ordered (sequenced spine by
    /// (epoch,seq), commutative last) before delivery so the owner-sequenced title
    /// folds deterministically — mirrors `folded_project`.
    fn folded_forum(&self, group_id: &[u8]) -> Result<ChatFold, CoreError> {
        // Through the fold cache (O-69), as every fold: a hit equals a refold.
        self.dir.cached(group_id, "chat", "fold", |a: &ChatFold, b| a.accepted_digest() == b.accepted_digest(), || {
            self.folded_forum_fresh(group_id)
        })
    }

    /// The chat lens's state, folded and reduced, through the fold cache (O-69): what a
    /// sync reads on every pass and a view on every read, so a quiet room costs neither.
    fn forum_state(&self, group_id: &[u8]) -> Result<coordinator::ForumState, CoreError> {
        self.dir.cached(group_id, "chat", "state", |a: &coordinator::ForumState, b| format!("{a:?}") == format!("{b:?}"), || {
            self.folded_forum_fresh(group_id).map(|f| f.state())
        })
    }

    fn folded_forum_fresh(&self, group_id: &[u8]) -> Result<ChatFold, CoreError> {
        let want = self.chat_type_id(group_id)?;
        let members = self.dir.group_members(group_id)?;
        // The owner at each delta's epoch (§10.2), not only today's: a spine delta the
        // previous owner wrote before a handover is still theirs to have written.
        let owners = self.dir.owner_history(group_id)?;
        let mut entries: Vec<(coordinator::Delta, [u8; 32])> = Vec::new();
        for (author, envelope) in self.dir.load_log(group_id)? {
            let d = coordinator::decode_delta(&envelope)?;
            // ONE log, MANY op-groups: a Contact channel carries both message deltas
            // (Forum) and prekey deltas (Contact). Fold only THIS lens's type — the
            // Coordinator hard-rejects foreign type_ids, so filtering is mandatory.
            if d.type_id == want {
                entries.push((d, author));
            }
        }
        entries.sort_by_key(|(d, _)| (d.seq.is_none(), d.epoch, d.seq.unwrap_or(0)));
        // THE FORK: one state, two wire types, and the Coordinator hard-rejects a
        // foreign one — so the object's own lens picks which to fold with.
        if want == coordinator::ConversationType::KIND.type_id() as u32 {
            let mut coord =
                coordinator::Coordinator::<coordinator::ConversationType>::with_owners(
                    members, owners,
                );
            for (delta, author) in entries {
                coord.deliver(delta, author)?;
            }
            return Ok(ChatFold::Conversation(coord));
        }
        let mut coord = coordinator::Coordinator::<coordinator::ForumType>::with_owners(members, owners);
        for (delta, author) in entries {
            coord.deliver(delta, author)?;
        }
        Ok(ChatFold::Forum(coord))
    }

    /// Fold `group_id`'s CONTACT op-group (the other lens over the same log): the
    /// prekey pool AND each side's self-published profile. A malformed delta is
    /// skipped, never fatal to the fold.
    fn folded_contact(&self, group_id: &[u8]) -> Result<crate::contact::ContactState, CoreError> {
        let members = self.dir.group_members(group_id)?;
        // The owner at each delta's epoch (§10.2), not only today's: a spine delta the
        // previous owner wrote before a handover is still theirs to have written.
        let owners = self.dir.owner_history(group_id)?;
        let mut coord =
            coordinator::Coordinator::<crate::contact::ContactType>::with_owners(members, owners);
        let contact_tid = crate::object::ObjectKind::Contact.type_id() as u32;
        for (author, envelope) in self.dir.load_log(group_id)? {
            let d = coordinator::decode_delta(&envelope)?;
            if d.type_id == contact_tid {
                let _ = coord.deliver(d, author); // best-effort: skip a bad offer
            }
        }
        Ok(coord.state().clone())
    }

    /// Just the prekey arm of [`folded_contact`] — the add-a-contact path's view.
    fn folded_prekeys(&self, group_id: &[u8]) -> Result<crate::contact::PrekeyPool, CoreError> {
        Ok(self.folded_contact(group_id)?.prekeys)
    }

    /// The prekey pool WE hold for `peer_id` — the offers they stocked into our shared
    /// Contact channel, ready to consume to add them to a new Conversation.
    pub fn contact_prekeys(
        &self,
        peer_id: &[u8; 32],
    ) -> Result<crate::contact::PrekeyPool, CoreError> {
        let group_id = self
            .dir
            .connection_group_for(peer_id)?
            .ok_or_else(|| CoreError::NotConnected(hex::encode(peer_id)))?;
        self.folded_prekeys(&group_id)
    }

    // ======================================================================
    // MY PROFILE — and its fan-out to every connection as a signed Delta.
    //
    // Before this, a profile reached a peer EXACTLY ONCE: inside the pairing
    // bundle, at QR-scan time. Change your photo afterwards and every contact you
    // already had kept the old one forever, because there was no channel to tell
    // them on. This is that channel.
    //
    // Shape of the guarantee ("transmitted to ALL Connections"):
    //   1. WRITE-LOCAL-FIRST. Each connection gets its own `contact.publishProfile`
    //      delta appended to its log durably, before any network is touched — so a
    //      change made in a tunnel is not lost, it is merely not yet flushed.
    //   2. BEST-EFFORT FLUSH, DURABLE RETRY. `append_and_flush` tries the relay now;
    //      whatever fails stays in that group's outbox and `sync_group` step (c)
    //      re-flushes it at the live epoch on every subsequent sync.
    //   3. SELF-HEALING RECONCILIATION. Every publish records a digest of what that
    //      connection was told. `publish_profile` re-publishes into any connection
    //      whose digest ≠ the live profile — which is what closes the gaps a pure
    //      event-driven fan-out leaves: connections formed AFTER the edit, a peer
    //      added while the app was closed, a restored backup.
    //
    // SIGNED: the delta is sealed as an MLS application message, so authorship is
    // the MLS-authenticated sender ("the author is never on the wire" —
    // `mls::Incoming::Application`). Nobody can publish a card in your name, and the
    // canonical CBOR is content-addressed by `delta.id()`.
    // ======================================================================

    /// My own profile: `(display_name, shape, card)`. A DB written before the card
    /// column existed reads as an empty card rather than an error — the display name
    /// alone is still a valid profile.
    pub fn my_profile(&self) -> Result<(String, GroupShape, group::ContactCard), CoreError> {
        // THE RECORD FIRST. Its log is the source; the `me` columns are the cache
        // that answers before the first `group.setProfile` has been authored, and
        // for a directory older than the record. A folded profile with no display
        // name is an unwritten one, not an empty one — fall through rather than
        // report a blank card over a stored one.
        if let Some(gid) = self.dir.my_object()? {
            if let Ok(st) = self.folded_group(&gid).map(|c| c.state().clone()) {
                if !st.display_name.is_empty() {
                    return Ok((st.display_name, st.shape, st.card));
                }
            }
        }
        self.cached_profile()
    }

    /// The `me` columns, verbatim. The cache half of the profile: what this device
    /// wrote, before the record has been told.
    fn cached_profile(&self) -> Result<(String, GroupShape, group::ContactCard), CoreError> {
        let display_name = self.dir.my_display_name()?;
        let (card_json, shape_str) = self.dir.my_profile()?;
        let shape = GroupShape::parse(&shape_str).unwrap_or(GroupShape::Individual);
        let card = if card_json.is_empty() {
            group::ContactCard::default()
        } else {
            serde_json::from_str(&card_json).unwrap_or_default()
        };
        Ok((display_name, shape, card))
    }

    /// Author `group.setProfile` onto the self record.
    async fn author_self_profile(
        &self,
        self_id: &str,
        display_name: &str,
        shape: GroupShape,
        card: &group::ContactCard,
    ) -> Result<(), CoreError> {
        let card_json = serde_json::to_string(card)
            .map_err(|e| CoreError::Coordinator(format!("card encode: {e}")))?;
        let mut args = coordinator::Args::new();
        args.insert(
            "displayName".into(),
            coordinator::ArgVal::Text(display_name.to_string()),
        );
        args.insert(
            "shape".into(),
            coordinator::ArgVal::Text(shape.as_str().to_string()),
        );
        args.insert("card".into(), coordinator::ArgVal::Text(card_json));
        self.apply(self_id, group::OP_SET_PROFILE, args).await?;
        Ok(())
    }

    /// RECONCILE THE RECORD AND ITS CACHE, in the one direction that cannot lose
    /// an edit.
    ///
    /// A record that has a profile is the truth — its log is sequenced, so the
    /// latest `setProfile` wins, including one authored on another device — and
    /// the `me` columns follow it. A record with NO profile is seeded from the
    /// cache: that is a backfilled account, or one whose first edit has not been
    /// authored yet. Reading the record and writing it from the same value in one
    /// pass is what made a second edit publish the first one's card.
    ///
    /// Best-effort by contract: a refusal leaves the edit in the cache, where the
    /// next pass finds it. A profile must never fail a sync.
    async fn reconcile_self_profile(&self) {
        let out = async {
            let self_id = self.ensure_self_object()?;
            let gid = hex::decode(&self_id)
                .map_err(|e| CoreError::Directory(format!("self object id hex: {e}")))?;
            let folded = self.folded_group(&gid).map(|c| c.state().clone()).ok();
            let (name, shape, card) = self.cached_profile()?;
            match folded {
                Some(st) if !st.display_name.is_empty() => {
                    if st.display_name != name || st.shape != shape || st.card != card {
                        let card_json = serde_json::to_string(&st.card)
                            .map_err(|e| CoreError::Coordinator(format!("card encode: {e}")))?;
                        self.dir.set_my_display_name(&st.display_name)?;
                        self.dir.set_my_profile(&card_json, st.shape.as_str())?;
                    }
                }
                _ if !name.is_empty() => {
                    self.author_self_profile(&self_id, &name, shape, &card).await?;
                }
                _ => {}
            }
            Ok::<(), CoreError>(())
        }
        .await;
        if let Err(e) = out {
            tracing::warn!(error = %e, "the self record and its cache could not be reconciled");
        }
    }

    /// Replace my profile, then fan it out to every connection. Returns how many
    /// connections received a fresh delta.
    ///
    /// The local write lands FIRST and unconditionally: your profile is yours
    /// whether or not the network is reachable. Publishing is what may partially
    /// fail, and it degrades to "queued in the outbox", never to "lost".
    pub async fn set_my_profile(
        &self,
        display_name: &str,
        shape: GroupShape,
        card: &group::ContactCard,
    ) -> Result<usize, CoreError> {
        // The clip does not ride the connection fan-out (see the refusal in
        // `contact`'s OP_PUBLISH_PROFILE fold). Caught HERE as well, before the local
        // write, so an inline clip fails loudly at the edit that created it rather
        // than being authored into every connection and refused by every one of them.
        // Failing before `dir.set_my_profile` also keeps the STORED card publishable:
        // a card that no peer will fold must never become the one we hold.
        if !card.clip.is_empty() || !card.clip_mime.is_empty() {
            return Err(CoreError::Coordinator(
                "profile clip cannot ride the connection fan-out: inline clip bytes \
                 are refused at every peer's fold — publish it detached"
                    .into(),
            ));
        }
        let card_json = serde_json::to_string(card)
            .map_err(|e| CoreError::Coordinator(format!("card encode: {e}")))?;
        self.dir.set_my_display_name(display_name)?;
        self.dir.set_my_profile(&card_json, shape.as_str())?;
        // THE RECORD, which is where this now lives. Best-effort for the same
        // reason the fan-out is: your profile is yours whether or not anything
        // else is reachable, and the cache above already holds it. A refusal here
        // is picked up by `reconcile_self_profile` on the next pass.
        if let Ok(self_id) = self.ensure_self_object() {
            if let Err(e) = self
                .author_self_profile(&self_id, display_name, shape, card)
                .await
            {
                tracing::warn!(error = %e, "the self record refused the profile; it stays cached");
            }
        }
        self.publish_profile(false).await
    }

    /// MAKE THE SELF RECORD NAME EVERY OBJECT THE SPINE DOES.
    ///
    /// The two are halves of one fact and neither is the other. A spine entry is
    /// sealed bytes at a derived address: recoverable from the words, and opaque
    /// until the object behind it is back. `group.joinedObject` is the same list
    /// as a folded Delta: readable by any member's client, and lost with the
    /// device unless the spine names it. This keeps them the same list.
    ///
    /// Idempotent, so it is safe on every sync: an object the record already
    /// names authors nothing.
    pub async fn reconcile_joined(&self) -> Result<usize, CoreError> {
        let self_id = self.ensure_self_object()?;
        let self_gid = hex::decode(&self_id)
            .map_err(|e| CoreError::Directory(format!("self object id hex: {e}")))?;
        let seed = self.seed_or_refuse()?;
        let root = crate::locator::storage_root(&seed);
        let named: std::collections::BTreeSet<String> = self
            .folded_group(&self_gid)
            .map(|c| c.state().joined.keys().cloned().collect())
            .unwrap_or_default();

        let mut wrote = 0usize;
        for (index, entry) in self.spine_entries()? {
            let gid = entry.body.group_id().to_vec();
            let object = hex::encode(&gid);
            // The record does not name ITSELF: it is the thing doing the naming,
            // and a device that cannot reach it has nothing to read the list from.
            if object == self_id || named.contains(&object) {
                continue;
            }
            // Only what this device actually holds. A spine entry for an object
            // whose directory row is gone is a naming this device cannot qualify.
            let Some(kind) = self.dir.group_kind(&gid)? else {
                continue;
            };
            let arc = self
                .dir
                .group_arc(&gid)?
                .unwrap_or_else(|| self.my_default_arc());
            // The way-in entry's own address, computed rather than stored — the
            // same arithmetic a recovering device runs.
            let tag = hex::encode(crate::locator::chain_locator(&root, SPINE_GEN, index));
            let mut args = coordinator::Args::new();
            args.insert("object".into(), coordinator::ArgVal::Text(object.clone()));
            args.insert("kind".into(), coordinator::ArgVal::Text(kind));
            args.insert("arc".into(), coordinator::ArgVal::Text(arc));
            args.insert("tag".into(), coordinator::ArgVal::Text(tag));
            args.insert("at".into(), coordinator::ArgVal::Int(unix_now() as i64));
            match self
                .apply(&self_id, group::OP_JOINED_OBJECT, args)
                .await
            {
                Ok(_) => wrote += 1,
                Err(e) => tracing::warn!(
                    object = %object, error = %e,
                    "the self record refused a vertebra; it stays named on the spine only"
                ),
            }
        }
        Ok(wrote)
    }

    /// MY CARD (O-77): what `base.publishProfile` carries of me, from the self record: its
    /// name, and its ContactCard's picture where the card can hold it. No name, no card:
    /// the webapp draws a hue and an initial. A picture the card cannot hold is left out and
    /// SAID, in the second value, so the webapp can ask for a smaller one; never dropped
    /// silently.
    pub fn my_card(&self) -> Result<(Option<crate::profiles::Card>, Option<String>), CoreError> {
        let (name, _, card) = self.my_profile()?;
        if name.trim().is_empty() {
            return Ok((None, None));
        }
        let named = crate::profiles::Card { display_name: name, photo: None, photo_mime: None };
        if card.photo.is_empty() {
            return Ok((Some(named), None));
        }
        let pictured = crate::profiles::Card { photo: Some(card.photo.clone()), photo_mime: Some(card.photo_mime.clone()), ..named.clone() };
        Ok(match pictured.refusal() {
            None => (Some(pictured), None),
            Some(why) => (Some(named), Some(format!("icon left out: {why}"))),
        })
    }

    /// PUBLISH MY CARD where it is missing or stale (O-77): into every object this device
    /// holds and is a member of whose kind carries `base.publishProfile` (the ICD's `profiles`
    /// facet, asked of the catalogue, not listed here), through the one write path. Beside
    /// [`Self::reconcile_joined`], so a join publishes on the sync after it, and a new name or
    /// picture on the self record reaches every such object. Idempotent: where the folded card
    /// is mine already it writes nothing.
    /// [`Self::reconcile_profiles`], its future built here and boxed, in a frame of its own: an
    /// async caller (`sync_once`, the Door's write tail) then holds a pointer, not a temporary of
    /// the future's size in its own poll frame. In a debug build a future is built on the caller's
    /// stack before it moves, so boxing it where it is awaited does not shrink that frame, and
    /// fc1's nested writes overflowed the test thread's 2 MiB (BW-A, 29 Sep).
    #[inline(never)]
    pub fn reconcile_profiles_boxed(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ProfileSync, CoreError>> + '_>> {
        Box::pin(self.reconcile_profiles())
    }

    pub async fn reconcile_profiles(&self) -> Result<ProfileSync, CoreError> {
        let (card, icon_left_out) = self.my_card()?;
        let mut out = ProfileSync { published: 0, icon_left_out };
        let Some(card) = card else { return Ok(out) };
        let me = self.id.identity_pk();
        let own = self.dir.my_object()?;
        let objects = self.dir.list_objects()?;
        // THE MEMO: this card, in this set of objects, found everywhere already. Nothing to
        // walk until the card or the set moves: a name, a picture, a join, a mint.
        let memo = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            card.to_arg().hash(&mut h);
            let mut ids: Vec<&Vec<u8>> = objects.iter().map(|(g, _)| g).collect();
            ids.sort();
            ids.hash(&mut h);
            h.finish()
        };
        if *self.card_everywhere.lock().unwrap_or_else(|p| p.into_inner()) == Some(memo) {
            return Ok(out);
        }
        let mut whole = true;
        for (gid, kind) in objects {
            if own.as_deref() == Some(gid.as_slice()) || crate::authoring::op_on(&kind, "base.publishProfile").is_none() {
                continue;
            }
            let object = hex::encode(&gid);
            if !self.dir.group_members(&gid)?.contains(&me) || self.is_departed(&object).unwrap_or(true) {
                continue;
            }
            let held = match crate::authoring::channel_of(&kind) {
                Some(crate::object::ObjectKind::Forum) => self.forum_state(&gid).map(|s| s.profiles.get(&me).cloned()),
                _ => self.folded_group(&gid).map(|c| c.state().profiles.get(&me).cloned()),
            };
            if matches!(&held, Ok(Some(p)) if p.card == card) {
                continue;
            }
            match self.apply(&object, crate::profiles::OP_PUBLISH_PROFILE, crate::profiles::publish_args(&card)).await {
                Ok(_) => out.published += 1,
                Err(e) => {
                    whole = false;
                    tracing::warn!(object = %object, error = %e, "my card was not published here; the next sync tries again");
                }
            }
        }
        if whole {
            *self.card_everywhere.lock().unwrap_or_else(|p| p.into_inner()) = Some(memo);
        }
        Ok(out)
    }

    /// The objects the self record names that this device does not hold: what another
    /// device of this person made or joined since this one last walked the spine.
    pub fn unheld_joined(&self) -> Result<Vec<String>, CoreError> {
        let Some(self_gid) = self.dir.my_object()? else {
            return Ok(Vec::new());
        };
        let named: Vec<String> = self
            .folded_group(&self_gid)
            .map(|c| c.state().joined.keys().cloned().collect())
            .unwrap_or_default();
        let mut out = Vec::new();
        for object in named {
            let Ok(gid) = hex::decode(&object) else { continue };
            if self.dir.group_kind(&gid)?.is_none() {
                out.push(object);
            }
        }
        Ok(out)
    }

    /// LIVE SYNC BETWEEN A PERSON'S DEVICES (Ralph, 27 Sep). Another device's object is
    /// named on the self record, which every device of the person holds and syncs; only
    /// [`Node::resume`] joins it, through its pool leaf. Without this a device held
    /// nothing new until it signed in again (Software Testing, E12). A sync with nothing
    /// unheld walks no spine; a set already tried waits [`JOIN_RETRY`]. Reported, never
    /// failing the sync.
    async fn join_what_the_record_names(&self) {
        let unheld: std::collections::BTreeSet<String> = match self.unheld_joined() {
            Ok(v) => v.into_iter().collect(),
            Err(e) => return tracing::warn!(target: "pacific::resumption", error = %e, "the self record could not be read for joining"),
        };
        if unheld.is_empty() {
            return;
        }
        {
            let mut tried = self.joined_tried.lock().unwrap();
            if tried.as_ref().is_some_and(|(set, at)| *set == unheld && at.elapsed() < JOIN_RETRY) {
                return;
            }
            *tried = Some((unheld.clone(), std::time::Instant::now()));
        }
        match self.resume(None).await {
            Ok(_) => tracing::info!(target: "pacific::resumption", named = unheld.len(), "joined what another device of this person named"),
            Err(e) => tracing::warn!(target: "pacific::resumption", error = %e, "objects named on the self record could not be joined this sync"),
        }
    }

    /// The digest that answers "has this connection been told THIS profile?".
    /// Domain-separated so it can never collide with another hash in the log.
    fn profile_digest(display_name: &str, shape: GroupShape, card_json: &str) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"pacific/profile/v1\0");
        h.update(display_name.as_bytes());
        h.update([0]);
        h.update(shape.as_str().as_bytes());
        h.update([0]);
        h.update(card_json.as_bytes());
        h.finalize().into()
    }

    /// Publish my current profile into every CONNECTION whose copy is stale, and whose link
    /// this side has wished for (`contact.setLink`; see [`Self::publish_profile_into`]).
    ///
    /// `force` re-publishes even where the digest already matches (the "resend it
    /// anyway" escape hatch). Otherwise this is idempotent and cheap: an unchanged
    /// profile authors nothing, so calling it on every sync costs one digest compare
    /// per connection and never touches the relay.
    ///
    /// Per-connection failures are isolated — one unreachable peer must not stop the
    /// other N-1 from being told. Returns the number actually published.
    pub async fn publish_profile(&self, force: bool) -> Result<usize, CoreError> {
        self.reconcile_self_profile().await;
        let (display_name, shape, card) = self.my_profile()?;
        let card_json = serde_json::to_string(&card)
            .map_err(|e| CoreError::Coordinator(format!("card encode: {e}")))?;
        let digest = Self::profile_digest(&display_name, shape, &card_json);

        let mut published = 0usize;
        for (group_id, kind) in self.dir.all_groups()? {
            // Connections only. A Forum/Project roster is not "your connections",
            // and an Arc tether is a machine link — neither wants your contact card.
            if kind != "connection" {
                continue;
            }
            if !force
                && self.dir.published_profile_digest(&group_id)?.as_deref() == Some(&digest[..])
            {
                continue; // this peer already holds exactly this card
            }
            match self
                .publish_profile_into(&group_id, &display_name, shape, &card)
                .await
            {
                // This side has not wished for the link: no card, and nothing recorded.
                Ok(false) => {}
                Ok(true) => {
                    // Recorded only on a successful local APPEND — the delta is now
                    // durable and the outbox owns delivery. Recording before the
                    // append would let a failed write masquerade as delivered and
                    // never be retried.
                    self.dir
                        .set_published_profile_digest(&group_id, &digest, unix_now())?;
                    published += 1;
                }
                Err(e) => {
                    tracing::warn!(
                        group = %hex::encode(&group_id[..group_id.len().min(6)]),
                        error = %e,
                        "profile publish failed for one connection; continuing with the rest"
                    );
                }
            }
        }
        Ok(published)
    }

    /// Author ONE `contact.publishProfile` delta into `group_id`, once this side's own
    /// `contact.setLink` active is 1 (the ICD: a side's card waits for its own side); `false`,
    /// and nothing written, until then.
    ///
    /// Commutative, so it carries the author's Lamport `gen` from
    /// `author_delta_count` — monotone per author, which is precisely the key the
    /// reducer's LWW resolves on. Note the count is over the WHOLE log (messages,
    /// prekeys, profiles), so gens are unique-and-increasing per author even though
    /// three op-groups share the log.
    async fn publish_profile_into(
        &self,
        group_id: &[u8],
        display_name: &str,
        shape: GroupShape,
        card: &group::ContactCard,
    ) -> Result<bool, CoreError> {
        if !self.folded_contact(group_id)?.wants_link(&self.id.identity_pk()) {
            return Ok(false);
        }
        let (sid, sk, _n) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut group = mls::load_group(&client, group_id)?;
        let epoch = group.current_epoch();
        let gen = self
            .dir
            .author_delta_count(group_id, &self.id.identity_pk())?;
        let args = crate::contact::publish_profile_args(display_name, shape, card, gen)
            .map_err(|e| CoreError::Coordinator(format!("profile args: {e}")))?;
        let delta = crate::object::build_delta(
            ObjectKind::Contact,
            crate::contact::OP_PUBLISH_PROFILE,
            args,
            epoch,
            Some(gen),
        );
        self.append_and_flush(&mut group, group_id, delta).await?;
        Ok(true)
    }

    // ======================================================================
    // THE MARKET — listing fan-out, and multihop discovery.
    //
    // A Thing is minted as a group of one: it never leaves the device. What travels
    // is a LISTING — the announcement of that thing's posture — carried on the same
    // Contact channels the profile fan-out uses, and governed by the same three
    // guarantees (write-local-first, best-effort flush with durable retry,
    // self-healing digest reconciliation).
    //
    // MULTIHOP. A listing marked `network` may be RELAYED by a recipient into their
    // own connections, up to `MAX_LISTING_HOPS`. That is what turns a set of private
    // 1:1 channels into a market: a village finds a ladder three handshakes away,
    // with no directory, no server, and nobody's contact list ever being shared.
    //
    // The relay carries the ORIGIN unchanged and the RELAYER as the delta author, so
    // every listing arrives with both "whose it is" and "who vouched for the path".
    // `private` never relays — the reducer rejects it on every device, so consent is
    // enforced by the network rather than by the sender's good manners.
    // ======================================================================

    /// The digest that answers "has this connection been told THIS listing, as it now
    /// stands?". Domain-separated, and it deliberately covers `hops` as well as the
    /// payload: a relay that reaches us by a shorter path is worth re-announcing.
    #[allow(clippy::too_many_arguments)]
    fn listing_digest(
        origin: &[u8; 32],
        thing_id: &str,
        posture: &str,
        title: &str,
        descriptor: &str,
        price: &str,
        deadline: i64,
        area: &str,
        reach: &str,
        hops: u32,
        rev: u64,
        withdrawn: bool,
        // The photo b64 — part of the announcement, so setting or clearing ONLY
        // the picture still moves the digest and republishes (a digest that skips
        // it would silently strand photo-only edits).
        photo: &str,
    ) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"pacific/listing/v1\0");
        for part in [
            hex::encode(origin).as_str(),
            thing_id,
            posture,
            title,
            descriptor,
            price,
            area,
            reach,
            photo,
        ] {
            h.update(part.as_bytes());
            h.update([0]);
        }
        h.update(deadline.to_be_bytes());
        h.update(hops.to_be_bytes());
        h.update(rev.to_be_bytes());
        h.update([withdrawn as u8]);
        h.finalize().into()
    }

    /// Publish MY things' postures into every connection whose copy is stale, then relay
    /// every forwardable listing I hold onward. Returns how many deltas were authored.
    ///
    /// Idempotent and cheap: an unchanged market authors nothing, so this is safe to call
    /// on every sync. Per-connection failures are isolated — one unreachable peer must not
    /// stop the rest of the market from moving.
    pub async fn publish_market(&self, force: bool) -> Result<usize, CoreError> {
        let mut n = self.publish_my_listings(force).await?;
        n += self.relay_listings().await?;
        Ok(n)
    }

    /// Fan out my OWN things: one `publishListing` per (connection, posture-bearing thing).
    ///
    /// A thing with no posture is not a listing and is never announced — "tracked but taking
    /// no market position" is a real state, and broadcasting it would leak an inventory
    /// nobody offered to share. A thing whose posture was CLEARED is announced once as a
    /// withdrawal, so the tombstone chases it down every path it travelled.
    async fn publish_my_listings(&self, force: bool) -> Result<usize, CoreError> {
        let me = self.id.identity_pk();
        let connections: Vec<Vec<u8>> = self
            .dir
            .all_groups()?
            .into_iter()
            .filter(|(_, kind)| kind == "connection")
            .map(|(gid, _)| gid)
            .collect();
        if connections.is_empty() {
            return Ok(0);
        }

        let mut published = 0usize;
        for (thing_id, state) in self.things()? {
            // `rev` is the origin's revision of this listing. The thing's own delta count is
            // exactly that: monotone, incremented by every edit, and identical on every
            // device that folds the log — which is what lets copies arriving down different
            // paths be ordered against each other.
            let group_id = self.object_group_id(&thing_id)?;
            let rev = self.dir.log_len(&group_id)? as u64;
            // A WITHDRAWAL cannot read its reach off the Thing: clearing a posture resets
            // the audience to private (so a later posture cannot inherit an unstated one),
            // and a private tombstone could never chase a listing that travelled the
            // network. The publish ledger is what still remembers how far it was allowed
            // to go, so the retraction follows the announcement.
            let (posture, withdrawn, reach) = match state.posture {
                Some(p) => (p, false, state.reach),
                None => match self.dir.last_published_reach(&me, &thing_id)? {
                    // Never announced at all → nothing to retract; stay silent.
                    None => continue,
                    Some(r) => (
                        crate::thing::Posture::Has,
                        true,
                        crate::thing::Reach::parse(&r).unwrap_or_default(),
                    ),
                },
            };
            let price = state.price.clone().unwrap_or_default();
            let deadline = state.deadline.unwrap_or(0);
            let area = state.area.clone().unwrap_or_default();
            let digest = Self::listing_digest(
                &me,
                &thing_id,
                posture.as_str(),
                &state.name,
                &state.descriptor,
                &price,
                deadline,
                &area,
                reach.as_str(),
                0,
                rev,
                withdrawn,
                &state.photo,
            );

            for group_id in &connections {
                if !force
                    && self
                        .dir
                        .published_listing(group_id, &me, &thing_id)?
                        .is_some_and(|(d, _, _)| d == digest)
                {
                    continue; // this peer already holds exactly this listing
                }
                let args = crate::contact::publish_listing_args(
                    &me,
                    &thing_id,
                    posture,
                    &state.name,
                    &state.descriptor,
                    (!price.is_empty()).then_some(price.as_str()),
                    (deadline > 0).then_some(deadline),
                    (!area.is_empty()).then_some(area.as_str()),
                    reach,
                    0, // mine: straight from the origin
                    rev,
                    withdrawn,
                    (!state.photo.is_empty())
                        .then_some((state.photo.as_str(), state.photo_mime.as_str())),
                );
                match self
                    .contact_author_into(group_id, crate::contact::OP_PUBLISH_LISTING, args)
                    .await
                {
                    Ok(()) => {
                        self.dir.set_published_listing(
                            group_id,
                            &me,
                            &thing_id,
                            &digest,
                            rev,
                            0,
                            reach.as_str(),
                            unix_now(),
                        )?;
                        published += 1;
                    }
                    Err(e) => tracing::warn!(
                        group = %hex::encode(&group_id[..group_id.len().min(6)]),
                        error = %e,
                        "listing publish failed for one connection; continuing with the rest"
                    ),
                }
            }
        }
        Ok(published)
    }

    /// Relay every forwardable listing I hold into my OTHER connections — the multihop half.
    ///
    /// THE LOOP RULES, and why each is needed:
    ///   - never relay back into the channel it arrived on (the trivial two-cycle);
    ///   - never relay to the ORIGIN (they know; it would also tell them who is passing
    ///     their listing around);
    ///   - never relay what I already told that peer at this revision by an equal-or-shorter
    ///     path (the digest ledger) — this is what terminates the flood in a cyclic graph,
    ///     because a second copy arriving the long way authors nothing;
    ///   - the hop ceiling itself, enforced again at fold by every recipient.
    async fn relay_listings(&self) -> Result<usize, CoreError> {
        // (`merged_listings` already excludes my own listings — those go out through
        // `publish_my_listings`, where the Thing state is authoritative.)
        let now_ms = unix_now() * 1_000;
        let connections: Vec<Vec<u8>> = self
            .dir
            .all_groups()?
            .into_iter()
            .filter(|(_, kind)| kind == "connection")
            .map(|(gid, _)| gid)
            .collect();

        // The best copy of each listing across every channel — merged over ALL rows, so a
        // withdrawal is never shadowed by a stale live copy sitting in another channel (see
        // `merged_listings`). Then keep only what may actually travel on.
        let best: Vec<crate::contact::Listing> = self
            .merged_listings()?
            .into_iter()
            .filter(|l| {
                l.reach == crate::thing::Reach::Network && l.hops < crate::contact::MAX_LISTING_HOPS
            })
            .filter(|l| l.withdrawn || l.deadline.is_none_or(|d| d > now_ms))
            .collect();

        let mut relayed = 0usize;
        for l in best {
            let (origin, thing_id) = (l.origin, l.thing_id.clone());
            // The channel this copy came in on — never relay back down it.
            let arrived_on = self.channel_holding(&origin, &thing_id, l.rev, l.hops)?;
            let hops = l.hops + 1;
            let price = l.price.clone().unwrap_or_default();
            let deadline = l.deadline.unwrap_or(0);
            let area = l.area.clone().unwrap_or_default();
            let digest = Self::listing_digest(
                &origin,
                &thing_id,
                l.posture.as_str(),
                &l.title,
                &l.descriptor,
                &price,
                deadline,
                &area,
                l.reach.as_str(),
                hops,
                l.rev,
                l.withdrawn,
                &l.photo,
            );

            for group_id in &connections {
                if Some(group_id) == arrived_on.as_ref() {
                    continue; // never back the way it came
                }
                // Never to the origin themselves.
                if self
                    .dir
                    .group_members(group_id)?
                    .iter()
                    .any(|m| m == &origin)
                {
                    continue;
                }
                // Already told them this revision by a path at least this short → silence.
                // This is the flood terminator: relaying is monotone, so the mesh quiesces.
                if let Some((d, rev, prev_hops)) =
                    self.dir.published_listing(group_id, &origin, &thing_id)?
                {
                    if d == digest || rev > l.rev || (rev == l.rev && prev_hops <= hops) {
                        continue;
                    }
                }
                let args = crate::contact::publish_listing_args(
                    &origin,
                    &thing_id,
                    l.posture,
                    &l.title,
                    &l.descriptor,
                    (!price.is_empty()).then_some(price.as_str()),
                    (deadline > 0).then_some(deadline),
                    (!area.is_empty()).then_some(area.as_str()),
                    l.reach,
                    hops,
                    l.rev,
                    l.withdrawn,
                    // The face travels UNCHANGED with every forward — listing content,
                    // exactly like the title.
                    (!l.photo.is_empty()).then_some((l.photo.as_str(), l.photo_mime.as_str())),
                );
                match self
                    .contact_author_into(group_id, crate::contact::OP_PUBLISH_LISTING, args)
                    .await
                {
                    Ok(()) => {
                        self.dir.set_published_listing(
                            group_id,
                            &origin,
                            &thing_id,
                            &digest,
                            l.rev,
                            hops,
                            l.reach.as_str(),
                            unix_now(),
                        )?;
                        relayed += 1;
                    }
                    Err(e) => tracing::warn!(
                        group = %hex::encode(&group_id[..group_id.len().min(6)]),
                        error = %e,
                        "listing relay failed for one connection; continuing with the rest"
                    ),
                }
            }
        }
        Ok(relayed)
    }

    /// Which connection channel carries this exact copy of a listing — the one it arrived on,
    /// which the relay must not send it back down. `None` when no channel holds it at this
    /// revision (it was superseded between the merge and here), in which case relaying to all
    /// is still safe: the ledger check below suppresses anything already told.
    fn channel_holding(
        &self,
        origin: &[u8; 32],
        thing_id: &str,
        rev: u64,
        hops: u32,
    ) -> Result<Option<Vec<u8>>, CoreError> {
        for (group_id, kind) in self.dir.all_groups()? {
            if kind != "connection" {
                continue;
            }
            if self
                .folded_contact(&group_id)?
                .listings
                .get(&(*origin, thing_id.to_string()))
                .is_some_and(|l| l.rev == rev && l.hops == hops)
            {
                return Ok(Some(group_id));
            }
        }
        Ok(None)
    }

    /// Author ONE commutative Contact delta into `group_id`. Commutative, so it carries
    /// the author's Lamport `gen` for coordinator ordering — any op-specific revision
    /// (a listing's `rev`, an answer's `at`) rides in the args instead. The write path
    /// the listing fan-out and the discovery legs share.
    ///
    /// CAVEAT for the next arm: this puts `gen` on the ENVELOPE only. An op whose
    /// REDUCER reads `gen` from args (`publishProfile`) must build it into its args
    /// itself — which is why `publish_profile_into` remains its own path rather
    /// than delegating here. The ticket legs no longer do: they go through
    /// `apply`, which takes the Lamport gen from `object::NextGen` like every
    /// other write.
    async fn contact_author_into(
        &self,
        group_id: &[u8],
        op_id: u32,
        args: coordinator::Args,
    ) -> Result<(), CoreError> {
        let (sid, sk, _n) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut group = mls::load_group(&client, group_id)?;
        let epoch = group.current_epoch();
        let gen = self
            .dir
            .author_delta_count(group_id, &self.id.identity_pk())?;
        let delta = crate::object::build_delta(ObjectKind::Contact, op_id, args, epoch, Some(gen));
        self.append_and_flush(&mut group, group_id, delta).await?;
        Ok(())
    }

    /// THE MARKET READ: every live listing that reached me, from every connection, deduped
    /// across channels by `(origin, thing_id)` keeping the newest revision and — among
    /// equals — the shortest path.
    ///
    /// My own listings are excluded: the market lists what OTHERS are offering, and the
    /// LIFE surface reads my side from `things()` where it is authoritative.
    pub fn inbound_listings(&self) -> Result<Vec<crate::contact::Listing>, CoreError> {
        let now_ms = unix_now() * 1_000;
        Ok(self
            .merged_listings()?
            .into_iter()
            .filter(|l| !l.withdrawn)
            .filter(|l| l.deadline.is_none_or(|d| d > now_ms))
            .collect())
    }

    /// Every listing I hold from every channel — tombstones INCLUDED. The merge
    /// itself (and the reason it merges FIRST and filters second) lives on the
    /// store with the rest of the n-2 cache: `GroupObjectStore::merged_listings`.
    fn merged_listings(&self) -> Result<Vec<crate::contact::Listing>, CoreError> {
        let me = self.id.identity_pk();
        crate::object_store::GroupObjectStore::new(&self.dir).merged_listings(&me)
    }

    // ======================================================================
    // DISCOVERY — the pull twin of the market's push gossip.
    //
    // The market broadcasts standing postures; discovery asks a LIVE QUESTION:
    // "list your GroupObjects of kind X matching these tags" — sent to my
    // connections and, within the hop budget, forwarded to theirs. Answers are
    // REFERENCE ROWS (name, kind, point), never the objects, and they flow back
    // down the path the question travelled ("the forward is the introduction").
    //
    // No side tables: a request is immutable and TTL-bounded, so the Contact
    // folds themselves answer every dedup question the listing machinery needed
    // ledgers for — "did I forward this there?" is "does that channel's fold
    // hold it?".
    // ======================================================================

    /// Every connection channel, folded — the read the whole gossip layer starts
    /// from (the market's merge, the discovery legs). One place owns the kind
    /// filter so no caller can forget it.
    fn connection_folds(&self) -> Result<Vec<(Vec<u8>, crate::contact::ContactState)>, CoreError> {
        let mut folds = Vec::new();
        for (gid, kind) in self.dir.all_groups()? {
            if kind == "connection" {
                let st = self.folded_contact(&gid)?;
                folds.push((gid, st));
            }
        }
        Ok(folds)
    }

    /// Ask the graph. Authors one `discoverRequest` into every connection (hops 0,
    /// `budget` clamped to [1, MAX_DISCOVER_HOPS]) and returns the request id the
    /// answers will carry. Re-asking the same live question reuses its rid — the
    /// sheet can re-open without re-flooding — and tops up any connection formed
    /// since. Answers arrive over the next sync ticks; read them with
    /// [`Self::discover_results`].
    pub async fn discover(
        &self,
        kind: &str,
        tags: &[String],
        budget: u32,
    ) -> Result<String, CoreError> {
        let me = self.id.identity_pk();
        let budget = budget.clamp(1, crate::contact::MAX_DISCOVER_HOPS);
        let now_ms = unix_now() * 1_000;
        let folds = self.connection_folds()?;

        // Reuse the newest live identical question rather than minting a fresh rid.
        let (at, rid) = folds
            .iter()
            .flat_map(|(_, st)| st.live_discover_requests(now_ms))
            .filter(|r| r.origin == me && r.kind == kind && r.tags == tags && r.budget == budget)
            .max_by_key(|r| r.at)
            .map(|r| (r.at, r.rid.clone()))
            .unwrap_or_else(|| {
                let mut raw = [0u8; 16];
                OsRng.fill_bytes(&mut raw);
                (now_ms, hex::encode(raw))
            });

        for (gid, st) in &folds {
            if st.discover_requests.contains_key(&(me, rid.clone())) {
                continue; // this channel already carries the question
            }
            let args = crate::contact::discover_request_args(&me, &rid, kind, tags, 0, budget, at);
            if let Err(e) = self
                .contact_author_into(gid, crate::contact::OP_DISCOVER_REQUEST, args)
                .await
            {
                tracing::warn!(
                    group = %hex::encode(&gid[..gid.len().min(6)]),
                    error = %e,
                    "discover request failed for one connection; continuing with the rest"
                );
            }
        }
        Ok(rid)
    }

    /// Warm the n-2 gossip caches — the app-load pass. Re-asks the graph for every
    /// discoverable kind ([`crate::contact::DISCOVER_KINDS`]) at the full budget;
    /// `discover`'s rid-reuse makes this idempotent (a still-live question is
    /// topped up, never re-flooded — only an expired one fires fresh). Trade needs
    /// no asking: listings are push gossip, reconciled by every sync's
    /// `publish_market`. Returns the request ids in play.
    pub async fn warm_gossip_caches(&self) -> Result<Vec<String>, CoreError> {
        let mut rids = Vec::new();
        for kind in crate::contact::DISCOVER_KINDS {
            rids.push(
                self.discover(kind, &[], crate::contact::MAX_DISCOVER_HOPS)
                    .await?,
            );
        }
        Ok(rids)
    }

    /// Everything of `kind` within `hops` handshakes: what I hold, plus the graph's
    /// cached statements (no TTL — every shelf renders between sessions from the
    /// folds alone), plus the live TTL window — merged, shortest-path deduped,
    /// minted-wins. Kinds: "place" (Discover) and "thing" (Trade). A pure read;
    /// the asking happens in [`Self::discover`] / [`Self::warm_gossip_caches`].
    pub fn discover_results(
        &self,
        kind: &str,
        hops: u32,
    ) -> Result<Vec<crate::object_store::DiscoveredRow>, CoreError> {
        let me = self.id.identity_pk();
        crate::object_store::GroupObjectStore::new(&self.dir).by_kind_hops(
            kind,
            hops,
            &me,
            unix_now() * 1_000,
        )
    }

    /// The objects I would show `r`'s asker, filtered from an already-folded place
    /// sweep (the caller folds ONCE per reconcile pass, not once per question).
    /// CONSENT IS THE VISIBILITY DIAL, graded by the asker's distance: a request
    /// that reached me at `hops` forwards has its asker at `hops + 1`, and an
    /// object is shown iff its visibility reach covers that distance — so a
    /// `connections` Place answers a friend and never a friend-of-friend, and a
    /// `private` one answers nobody. (`access` is NOT consulted: joinable and
    /// discoverable are different dials.) Unknown kinds return empty rather than
    /// erroring: on the network side "I have nothing to say" is the forward-
    /// compatible answer to a question from a newer build (contrast
    /// `by_kind_hops`, whose loud error is right for a local caller's typo).
    /// Sorted by id so equality against a previously-folded answer is well-defined
    /// (that comparison is the re-answer gate).
    fn discover_items_for(
        r: &crate::contact::DiscoverRequest,
        places: &[(String, crate::place::PlaceState)],
    ) -> Vec<crate::contact::DiscoverItem> {
        if r.kind != "place" {
            return Vec::new();
        }
        let asker_distance = r.hops + 1;
        let tags: Vec<String> = r.tags.iter().map(|t| t.to_lowercase()).collect();
        let mut items = Vec::new();
        for (id, st) in places {
            if st.visibility.reach_hops() < asker_distance {
                continue;
            }
            if !tags.is_empty() {
                let hay = format!("{} {}", st.name, st.descriptor).to_lowercase();
                if !tags.iter().any(|t| hay.contains(t)) {
                    continue;
                }
            }
            let point = st.point();
            items.push(crate::contact::DiscoverItem {
                id: id.clone(),
                kind: r.kind.clone(),
                name: st.name.clone(),
                descriptor: st.descriptor.clone(),
                lat_e7: point.map(|p| p.lat_e7),
                lng_e7: point.map(|p| p.lng_e7),
                vis: st.visibility,
            });
        }
        items.sort_by(|a, b| a.id.cmp(&b.id));
        items
    }

    /// The discovery reconcile pass — answer, forward, relay. Runs on every sync,
    /// authors nothing once the mesh has quiesced (every gate below reads a fold
    /// that already contains what a previous pass authored).
    ///
    /// THE LOOP RULES are `relay_listings`' four, applied to a question:
    ///   - never forward back down the channel it arrived on;
    ///   - never forward to a channel containing the ASKER (they asked; it would
    ///     also tell them who is passing their question around);
    ///   - never author what the target channel's fold already holds — the flood
    ///     terminator;
    ///   - the budget, re-enforced at fold by every recipient (`hops >= budget`
    ///     never folds, so a request never travels past whom it may still reach).
    pub async fn reconcile_discovery(&self) -> Result<usize, CoreError> {
        let me = self.id.identity_pk();
        let now_ms = unix_now() * 1_000;

        let mut folds: Vec<(
            Vec<u8>,
            crate::contact::ContactState,
            Vec<crate::object::MemberId>,
        )> = Vec::new();
        for (gid, st) in self.connection_folds()? {
            let members = self.dir.group_members(&gid)?;
            folds.push((gid, st, members));
        }
        if folds.is_empty() {
            return Ok(0);
        }

        // My shareable places, folded AT MOST ONCE per pass (lazily — a tick with
        // no live foreign question folds nothing), and each distinct question
        // SHAPE filtered once — never per (channel × question). The key carries
        // the asker's distance (r.hops) as well as kind+tags, because visibility
        // grades the answer by distance: the same question one hop further out
        // gets a smaller set. A live question sits in the folds for its whole
        // 15-minute TTL and this pass rides the 3-second tick, so the
        // per-question cost is what the multiplier bites.
        let mut my_places: Option<Vec<(String, crate::place::PlaceState)>> = None;
        let mut answers: std::collections::BTreeMap<
            (String, Vec<String>, u32),
            Vec<crate::contact::DiscoverItem>,
        > = Default::default();

        // The folds were captured before this pass authors anything, so a question
        // sitting in TWO source channels (a diamond in the graph) needs an
        // in-memory guard against double-authoring within the pass itself.
        let mut fwd_sent: std::collections::BTreeSet<(Vec<u8>, crate::object::MemberId, String)> =
            Default::default();
        let mut resp_sent: std::collections::BTreeSet<(
            Vec<u8>,
            crate::object::MemberId,
            String,
            crate::object::MemberId,
        )> = Default::default();
        let mut authored = 0usize;

        for (gid, st, _members) in &folds {
            // ---- ANSWER every live question that reached me here ----
            for r in st.live_discover_requests(now_ms) {
                if r.origin == me || r.hops + 1 > r.budget {
                    continue;
                }
                let key = (r.kind.clone(), r.tags.clone(), r.hops);
                let items = match answers.get(&key) {
                    Some(i) => i.clone(),
                    None => {
                        if my_places.is_none() {
                            my_places = Some(self.places()?);
                        }
                        let i = Self::discover_items_for(r, my_places.as_deref().unwrap_or(&[]));
                        answers.insert(key, i.clone());
                        i
                    }
                };
                let held = st.discover_responses.get(&(r.origin, r.rid.clone(), me));
                // Re-answer when my shareable set changed; stay silent when I have
                // nothing and never said anything (an empty first answer is noise).
                let stale = match held {
                    Some(h) => h.items != items,
                    None => !items.is_empty(),
                };
                if stale {
                    let args = crate::contact::discover_response_args(
                        &r.origin,
                        &r.rid,
                        &me,
                        &items,
                        r.hops + 1,
                        now_ms,
                    );
                    // Per-channel isolation, like the market: one wedged channel
                    // must not block discovery on every other.
                    match self
                        .contact_author_into(gid, crate::contact::OP_DISCOVER_RESPONSE, args)
                        .await
                    {
                        Ok(()) => authored += 1,
                        Err(e) => tracing::warn!(
                            group = %hex::encode(&gid[..gid.len().min(6)]),
                            error = %e,
                            "discovery answer failed for one connection; continuing"
                        ),
                    }
                }

                // ---- FORWARD it while the budget still reaches someone ----
                // The next copy travels at hops+1, so its recipient sits at
                // hops+2 — forward only while THEY can still answer within budget.
                if r.hops + 2 > r.budget {
                    continue;
                }
                for (gid2, st2, members2) in &folds {
                    if gid2 == gid {
                        continue; // never back the way it came
                    }
                    if members2.iter().any(|m| m == &r.origin) {
                        continue; // never to the asker
                    }
                    if st2
                        .discover_requests
                        .contains_key(&(r.origin, r.rid.clone()))
                    {
                        continue; // that channel already carries it — the flood terminator
                    }
                    if !fwd_sent.insert((gid2.clone(), r.origin, r.rid.clone())) {
                        continue;
                    }
                    let args = crate::contact::discover_request_args(
                        &r.origin,
                        &r.rid,
                        &r.kind,
                        &r.tags,
                        r.hops + 1,
                        r.budget,
                        r.at,
                    );
                    match self
                        .contact_author_into(gid2, crate::contact::OP_DISCOVER_REQUEST, args)
                        .await
                    {
                        Ok(()) => authored += 1,
                        Err(e) => tracing::warn!(
                            group = %hex::encode(&gid2[..gid2.len().min(6)]),
                            error = %e,
                            "discovery forward failed for one connection; continuing"
                        ),
                    }
                }
            }

            // ---- RELAY answers back toward their asker ----
            // An answer I carry for someone else's question moves to every channel
            // where that question reached ME from someone else (`via != me` = the
            // upstream leg). The holder's `at` rides UNCHANGED: re-stamping it would
            // let a relay outrank the holder's own later re-answer at the asker.
            for resp in st.live_discover_responses(now_ms) {
                if resp.origin == me || resp.holder == me {
                    continue;
                }
                for (gid2, st2, _m2) in &folds {
                    if gid2 == gid {
                        continue;
                    }
                    let Some(req) = st2.discover_requests.get(&(resp.origin, resp.rid.clone()))
                    else {
                        continue;
                    };
                    if req.via == me {
                        continue; // I forwarded the question there — downstream, not upstream
                    }
                    if st2
                        .discover_responses
                        .get(&(resp.origin, resp.rid.clone(), resp.holder))
                        .is_some_and(|h| h.items == resp.items)
                    {
                        continue; // already relayed, and the answer has not changed
                    }
                    if !resp_sent.insert((gid2.clone(), resp.origin, resp.rid.clone(), resp.holder))
                    {
                        continue;
                    }
                    let args = crate::contact::discover_response_args(
                        &resp.origin,
                        &resp.rid,
                        &resp.holder,
                        &resp.items,
                        resp.hops,
                        resp.at,
                    );
                    match self
                        .contact_author_into(gid2, crate::contact::OP_DISCOVER_RESPONSE, args)
                        .await
                    {
                        Ok(()) => authored += 1,
                        Err(e) => tracing::warn!(
                            group = %hex::encode(&gid2[..gid2.len().min(6)]),
                            error = %e,
                            "discovery relay failed for one connection; continuing"
                        ),
                    }
                }
            }
        }
        Ok(authored)
    }

    /// The profile `peer_id` published to US on our shared Contact channel, or None
    /// if they have not published one (an older build, or a brand-new pairing).
    /// None is NOT an empty profile — the caller falls back to the pairing-bundle
    /// name rather than rendering a blank card.
    pub fn peer_profile(
        &self,
        peer_id: &[u8; 32],
    ) -> Result<Option<crate::contact::SelfProfile>, CoreError> {
        let Some(group_id) = self.dir.connection_group_for(peer_id)? else {
            return Ok(None);
        };
        Ok(self.folded_contact(&group_id)?.profile_of(peer_id).cloned())
    }

    /// Refresh the cached `peers.display_name` from what the peer PUBLISHED on
    /// `group_id`. Called on sync so a peer's rename lands everywhere the cached
    /// projection is read (the connections list, chat headers, derived group names)
    /// without every one of those surfaces having to learn about the fold.
    /// Returns true when a name actually changed.
    fn refresh_peer_profile_cache(&self, group_id: &[u8]) -> Result<bool, CoreError> {
        let me = self.id.identity_pk();
        let state = self.folded_contact(group_id)?;
        let mut changed = false;
        for (author, profile) in &state.profiles {
            if author == &me || profile.display_name.is_empty() {
                continue;
            }
            changed |= self
                .dir
                .set_peer_display_name(author, &profile.display_name)?;
        }
        Ok(changed)
    }

    /// A peer's display name, `""` if unknown.
    ///
    /// AN INDIVIDUAL'S GROUP IS ONLY FOR THEM (ruled 24 September 2026). Pairing
    /// used to mint a group of one ABOUT the peer and read the name out of its MLS
    /// GroupContext — a second record of a person who already has their own, on a
    /// spine that already names the connection. The name comes from the peer's own
    /// `contact.publishProfile`, folded into the peer row on sync.
    pub fn peer_identity_name(&self, peer_id: &[u8; 32]) -> Result<String, CoreError> {
        self.dir.peer_display_name(peer_id)
    }

    /// Mint `n` fresh key packages and OFFER them into our Contact channel with
    /// `group_id` (author `prekeySupply` deltas), so the peer can add US to future
    /// Conversations with no live code exchange. Each mint retains its private half
    /// (mls_key_package) until consumed. Best-effort per offer; reaches the relay
    /// once the channel has the peer in it.
    async fn stock_prekeys(&self, group_id: &[u8], n: usize) -> Result<(), CoreError> {
        let (sid, sk, _name) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut group = mls::load_group(&client, group_id)?;
        let epoch = group.current_epoch();
        let me = self.id.identity_pk();
        // Every offer carries OUR intro tag, so whoever consumes it can seal our Welcome.
        let my_intro = self.dir.my_intro_tag()?;
        let base = self.dir.author_delta_count(group_id, &me)?;
        for i in 0..n {
            let kp = mls::make_key_package_bytes(&client)?;
            let args = crate::contact::supply_args(&kp, &my_intro, 0);
            let delta = crate::object::build_delta(
                crate::object::ObjectKind::Contact,
                crate::contact::OP_PREKEY_SUPPLY,
                args,
                epoch,
                Some(base + i as u64),
            );
            if let Err(e) = self.append_and_flush(&mut group, group_id, delta).await {
                tracing::warn!(error = %e, offer = i,
                               "prekey offer failed; sync will top the channel back up");
            }
        }
        Ok(())
    }

    /// Top every connection channel back up to [`PREKEY_STOCK`] offers of OUR OWN.
    ///
    /// Two holes this closes, both of which made adds fail permanently rather than
    /// transiently:
    ///
    /// 1. Only the scanner stocked at pairing, so the scanned side supplied nothing
    ///    and the scanner could never add them anywhere. `pair_accept` now stocks
    ///    directly, but this catches every connection formed before it did, and any
    ///    accept whose stocking did not reach the relay.
    /// 2. An offer is consumed per add and was never replaced, so even a healthy
    ///    pair ran dry after eight and landed on `add_contact_to_object`'s "ask them
    ///    to replenish" — a verb no code path implemented.
    ///
    /// Cheap in the steady state: a full channel costs one fold and writes nothing.
    async fn replenish_prekeys(&self) -> Result<usize, CoreError> {
        let mut topped = 0usize;
        for (group_id, kind) in self.dir.all_groups()? {
            if kind != "connection" {
                continue;
            }
            match self.top_up_prekeys(&group_id).await {
                Ok(n) => topped += n,
                Err(e) => tracing::warn!(error = %e, "prekey top-up failed; retry next sync"),
            }
        }
        Ok(topped)
    }

    /// Bring ONE connection channel up to [`PREKEY_STOCK`] offers of our own,
    /// authoring only the shortfall. Returns how many were authored (0 when the
    /// channel is already full), so callers can run it unconditionally — pairing,
    /// accepting and every sync all funnel through here rather than each stocking a
    /// fresh batch and leaving the pool to grow without bound.
    async fn top_up_prekeys(&self, group_id: &[u8]) -> Result<usize, CoreError> {
        let me = self.id.identity_pk();
        let held = self
            .folded_prekeys(group_id)?
            .count_from(&me, unix_now() as u64);
        if held >= PREKEY_STOCK {
            return Ok(0);
        }
        let want = PREKEY_STOCK - held;
        self.stock_prekeys(group_id, want).await?;
        Ok(want)
    }

    /// The group's DISPLAY NAME, read from its MLS GroupContext extension ("" until
    /// named). It rides the Welcome's GroupInfo, so a late joiner reads the same
    /// current name with no message backfill (see `mls::group_name`).
    pub fn object_name(&self, object_id_hex: &str) -> Result<String, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        let (sid, sk, _n) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let group = mls::load_group(&client, &group_id)?;
        Ok(mls::group_name(&group))
    }

    /// RENAME an object: an owner GroupContextExtensions commit that rewrites the
    /// name in the GroupContext. The twin of `add_member_core` — same owner check,
    /// same epoch commit-slot discipline — minus the Welcome, since the roster is
    /// untouched. Existing members fold the commit off the OLD epoch tag on their
    /// next sync; anyone added afterwards reads the new name from their Welcome.
    ///
    /// There is deliberately NO title delta: the name is durable group STATE, and a
    /// per-message delta would be withheld from late joiners by forward secrecy.
    pub async fn object_rename(&self, object_id_hex: &str, name: &str) -> Result<(), CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        let me = self.id.identity_pk();
        match self.dir.group_owner(&group_id)? {
            Some(o) if o.as_slice() == me.as_slice() => {}
            Some(_) => {
                return Err(CoreError::Coordinator(
                    "only the group owner can rename the group".into(),
                ))
            }
            None => return Err(CoreError::Directory(format!("no group {object_id_hex}"))),
        }

        let (sid, sk, _n) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut group = mls::load_group(&client, &group_id)?;

        // A group of one has no peers to sequence against: no relay round trip, no
        // slot to win. Apply directly — the same shortcut group creation takes.
        let solo = mls::roster_identities(&group)?.len() <= 1;

        // Capture the CURRENT epoch's tag + secret before we advance, so members
        // still at that epoch can drain the commit off it. Staging does not modify
        // state (RFC 9420 §14), so this stays valid across `stage_rename`.
        let old_epoch_n = group.current_epoch();
        let old_epoch = mls::epoch_be(old_epoch_n);
        let old_tag = mls::group_tag(&group, &old_epoch)?;
        let old_addr = mls::group_address(&group, &old_epoch)?;
        let old_secret = mls::seal_conn_secret(&group, &old_epoch)?;

        let commit = mls::stage_rename(&mut group, name)?;
        if solo {
            mls::apply_staged(&mut group)?;
            return Ok(());
        }
        self.dir
            .record_epoch_tag(&group_id, old_epoch_n, &old_tag, &old_secret)?;
        let sealed_commit = seal::seal(&commit, &old_tag, &old_secret)?;

        let mut sess = Router::open(&self.routes).await?;
        let won = sess
            .publish_commit(
                &old_addr,
                &pacific_wire::blob_b64(&sealed_commit),
            )
            .await?;
        if won.is_none() {
            // LOST the race: the staged commit was never applied or persisted, so
            // reloading discards it. Fold the winner in, then surface the race — the
            // caller re-derives the rename against the new epoch and retries.
            let mut fresh = mls::load_group(&client, &group_id)?;
            let _ = self
                .drain_tag(&mut sess, &mut fresh, &group_id, &old_tag, &old_secret)
                .await;
            sess.close().await;
            return Err(CoreError::CommitRace(format!(
                "epoch {old_epoch_n} commit slot already taken — synced past the winner; retry the rename"
            )));
        }
        mls::apply_staged(&mut group)?;
        sess.close().await;
        Ok(())
    }

    /// Fold a group's persisted log into its Forum transcript over the current
    /// roster — the single deterministic projection shared by `object_transcript`,
    /// `dm_transcript` and `sync`'s rebuild (connection DMs and forum objects alike
    /// carry `forum.post` deltas). Runs through the generic `Coordinator<ForumType>`;
    /// there is no bespoke Forum fold engine left.
    fn forum_transcript(&self, group_id: &[u8]) -> Result<Vec<String>, CoreError> {
        Ok(self
            .folded_forum(group_id)?
            .state()
            .transcript()
            .into_iter()
            .map(|(a, g, t)| format!("[{} g{}] {}", short(&a), g, t))
            .collect())
    }

    /// The STRUCTURED folded view of a forum/connection group: every message with
    /// its (author, gen) id, time, reply parent, reactions — and, for OUR OWN
    /// messages, the WhatsApp-style receipt code (`ForumMessage::receipt`). This is
    /// what the chat UI reads (timestamps, reaction pills, reply quotes, ✓✓ ticks);
    /// `forum_transcript` stays the text-only projection.
    ///
    /// The receipt code is filled HERE (not in the pure `detailed()` fold) because
    /// only the node knows our identity and the current recipient roster: `1` = sent
    /// (left this device), `2` = delivered to every recipient, `3` = read by every
    /// recipient. Left `0` on incoming messages.
    pub fn forum_detailed(
        &self,
        group_id: &[u8],
    ) -> Result<Vec<coordinator::ForumMessage>, CoreError> {
        let state = self.forum_state(group_id)?;
        let mut msgs = state.detailed();
        let me = self.id.identity_pk();
        let recipients: Vec<[u8; 32]> = self
            .dir
            .group_members(group_id)?
            .into_iter()
            .filter(|m| m != &me)
            .collect();
        for m in &mut msgs {
            if m.author != me {
                continue; // receipts are shown only on the sender's own bubbles
            }
            m.receipt = if recipients.is_empty() {
                // Self-chat / note-to-self (no other members): "delivered to every
                // recipient" and "read by every recipient" are both vacuously true
                // over an empty recipient set, and you authored (so have seen) it —
                // so show it as read (✓✓-blue), a "saved & seen" affordance rather
                // than a stuck single ✓. Adding a member later recomputes against the
                // real roster.
                3
            } else {
                // summary: 0 none / 1 delivered-to-all / 2 read-by-all → UI 1/2/3,
                // with 0 the "sent" floor (we hold it; nobody has ack'd yet).
                match state.receipt_summary(&(m.author, m.gen), &recipients) {
                    coordinator::RECEIPT_READ => 3,
                    coordinator::RECEIPT_DELIVERED => 2,
                    _ => 1,
                }
            };
        }
        Ok(msgs)
    }

    /// The folded thread TREE of a forum/connection group — every post nested under
    /// its reply parent (Reddit-style), pre-order flattened with depth. The tree
    /// twin of `forum_detailed`: same fold, a different read-time projection.
    pub fn forum_thread(&self, group_id: &[u8]) -> Result<Vec<coordinator::ThreadNode>, CoreError> {
        Ok(self.forum_state(group_id)?.thread())
    }

    /// The structured view of a 1:1 conversation with `peer_id`.
    pub fn dm_detailed(
        &self,
        peer_id: &[u8; 32],
    ) -> Result<Vec<coordinator::ForumMessage>, CoreError> {
        if self.dir.connection_group_for(peer_id)?.is_none() {
            return Err(CoreError::UnknownPeer(hex::encode(peer_id)));
        }
        match self.dir.conversation_group_for(peer_id)? {
            Some(gid) => self.forum_detailed(&gid),
            None => Ok(Vec::new()),
        }
    }

    /// Author one `forum.post` Delta into a (forum or event) object's log. Local for
    /// a group of 1; over the shared group tag once others have joined.
    ///
    /// EVENTS carry the same log deliberately: one MLS log holds many op-groups and
    /// each lens folds only its own type_id, so a shared Event GroupObject IS the
    /// room its members discuss it in — type-19 message deltas beside the type-29
    /// event deltas, no second object and no link to lose. Other kinds still refuse
    /// until they earn a thread.
    pub async fn object_post(
        &self,
        object_id_hex: &str,
        text: &str,
        media: Option<&pacific_media::MediaRef>,
    ) -> Result<String, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        let kind = self
            .dir
            .group_kind(&group_id)?
            .ok_or_else(|| CoreError::Directory(format!("no object {object_id_hex}")))?;
        if !matches!(kind.as_str(), "forum" | "event") {
            return Err(CoreError::Coordinator(format!(
                "object kind '{kind}' has no text-post op yet"
            )));
        }
        let delta_id = self.post_to_group(&group_id, text, None, media).await?;
        let delta_id_hex = hex::encode(delta_id);
        // OPT-IN rust-direct projection (WS-G): the delta append path is the seam
        // where a synced GroupObject's reduced content lands in the union LodeDB
        // store (coordination §4 THE ONE FORK — the delta still goes to the wire;
        // its reduced text ALSO lands in LodeDB). Skipped entirely when no
        // projector is installed; fail-loud when one is.
        if let Some(projector) = &self.projection {
            projector.project(crate::projection::ProjectedContent {
                source: crate::projection::DeltaSource::Forum,
                group_object_id: object_id_hex,
                object_id: &delta_id_hex,
                text,
            })?;
        }
        Ok(delta_id_hex)
    }

    /// The compiled projection of a (forum) object — the folded transcript, over
    /// the full group roster (not just us).
    pub fn object_transcript(&self, object_id_hex: &str) -> Result<Vec<String>, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        self.forum_transcript(&group_id)
    }

    /// The STRUCTURED projection of a (forum) object — messages with timestamps,
    /// reply linkage and reaction groups. The group-object twin of `dm_detailed`,
    /// so the UI drives one code path for both DMs and group objects.
    pub fn object_detailed(
        &self,
        object_id_hex: &str,
    ) -> Result<Vec<coordinator::ForumMessage>, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        self.forum_detailed(&group_id)
    }

    /// The thread-TREE projection of a (forum) object — the ChatRoom read path.
    /// Reddit-style nesting over the same fold `object_detailed` returns flat.
    pub fn object_thread(
        &self,
        object_id_hex: &str,
    ) -> Result<Vec<coordinator::ThreadNode>, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        self.forum_thread(&group_id)
    }

    /// The current MLS member roster of an object (their identity pubkeys) — who a
    /// Project can assign work to, and who a group's "Add people" screen already
    /// lists. The same membership the commutative fold keys access on.
    pub fn object_members(&self, object_id_hex: &str) -> Result<Vec<[u8; 32]>, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        self.dir.group_members(&group_id)
    }

    /// The object's OWNER identity pubkey, or `None` if we hold no such group.
    /// Only the owner may add members or rename, so the UI reads this to decide
    /// whether to offer those affordances at all.
    pub fn object_owner(&self, object_id_hex: &str) -> Result<Option<Vec<u8>>, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        self.dir.group_owner(&group_id)
    }

    /// React to a message in a (forum) object — the group-object twin of `react_dm`.
    pub async fn object_react(
        &self,
        object_id_hex: &str,
        target: coordinator::MsgRef,
        emoji: &str,
        active: bool,
    ) -> Result<[u8; 32], CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        self.react_in_group(&group_id, target, emoji, active).await
    }

    /// Up/downvote a message in a (forum) object — the karma twin of
    /// [`object_react`]: same one-write-path, same LWW-per-author fold. `dir` is
    /// +1 (up), −1 (down) or 0 (clear your vote). Named `_post` because
    /// [`object_vote`] is RATIFY's ballot — a different thing entirely.
    pub async fn object_vote_post(
        &self,
        object_id_hex: &str,
        target: coordinator::MsgRef,
        dir: i8,
    ) -> Result<[u8; 32], CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        // Converge to the head epoch first (see `post_to_group`): vote at the live
        // epoch so members on the newer epoch can decrypt it.
        let _ = self.drain_to_head(&group_id).await;
        let (sid, sk, _name) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut group = self.load_member_group(&client, &group_id)?;
        let gen = self.next_lamport(&group_id)?;
        let delta = coordinator::forum_vote(target, dir, gen, group.current_epoch())
            .retyped(self.chat_type_id(&group_id)?);
        self.append_and_flush(&mut group, &group_id, delta).await
    }

    /// Reply to a message in a (forum) object — the group-object twin of
    /// `author_post_reply`. Returns the new message's DeltaId.
    pub async fn object_post_reply(
        &self,
        object_id_hex: &str,
        text: &str,
        reply_to: Option<coordinator::MsgRef>,
        media: Option<&pacific_media::MediaRef>,
    ) -> Result<[u8; 32], CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        self.post_to_group(&group_id, text, reply_to, media).await
    }

    // ---- RATIFY on a (forum) object: propose / vote / close / read ----------
    //
    // The base governance layer over the wire: `propose`/`vote` are commutative
    // (same single write path as a post/react), `close` is owner-sequenced (it rides
    // the spine, so `next_sequenced_pos` positions it and the spine rejects a
    // non-owner). Roles/outcome are derived on read (`object_ratify`).

    /// Propose a deferred ratification. `payload` is the deferred inner content (a
    /// decision/fact in this slice); `rule` the passing rule. Returns the proposal id
    /// = this device's (identity, gen). Commutative — any member.
    pub async fn object_propose(
        &self,
        object_id_hex: &str,
        payload: &str,
        rule: coordinator::Rule,
    ) -> Result<coordinator::MsgRef, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        let _ = self.drain_to_head(&group_id).await;
        let (sid, sk, _name) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut group = self.load_member_group(&client, &group_id)?;
        let gen = self.next_lamport(&group_id)?;
        let delta = coordinator::ratify_propose(
            payload,
            rule,
            gen,
            group.current_epoch(),
            self.chat_type_id(&group_id)?,
        );
        self.append_and_flush(&mut group, &group_id, delta).await?;
        Ok((self.id.identity_pk(), gen))
    }

    /// Cast a ballot on `target` — commutative, LWW per (proposal, voter).
    pub async fn object_vote(
        &self,
        object_id_hex: &str,
        target: coordinator::MsgRef,
        ballot: coordinator::Ballot,
    ) -> Result<(), CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        let _ = self.drain_to_head(&group_id).await;
        let (sid, sk, _name) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut group = self.load_member_group(&client, &group_id)?;
        let gen = self.next_lamport(&group_id)?;
        let delta = coordinator::ratify_vote(
            target,
            ballot,
            gen,
            group.current_epoch(),
            self.chat_type_id(&group_id)?,
        );
        self.append_and_flush(&mut group, &group_id, delta).await?;
        Ok(())
    }

    /// Close `target`, freezing its tally — the OWNER-sequenced serialization point.
    /// Loud if this device is not the object's owner (the spine would reject it too,
    /// but a non-owner close would poison the log, so refuse early).
    pub async fn object_close(
        &self,
        object_id_hex: &str,
        target: coordinator::MsgRef,
    ) -> Result<(), CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        if self.group_owner_id(&group_id)? != self.id.identity_pk() {
            return Err(CoreError::Coordinator(
                "only the object owner can close a ratification".into(),
            ));
        }
        let _ = self.drain_to_head(&group_id).await;
        let (sid, sk, _name) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut group = self.load_member_group(&client, &group_id)?;
        let epoch = group.current_epoch();
        let coord = self.folded_forum(&group_id)?;
        let (seq, prev) = coordinator::next_sequenced_pos(coord.sequenced_head(), epoch);
        // A FROZEN close (membership-through-mls.md §10.4): name the electorate it closes
        // under and the exact ballots it counts, so no later removal or handover can
        // re-decide it — the defect that would let removing a dissenter pass a payment.
        let electorate = coord.members().to_vec();
        let ballots = coord.ratify_ballots_for(&target);
        tracing::info!(
            target: "pacific::ratify",
            object = %object_id_hex,
            proposer = %hex::encode(target.0),
            gen = target.1,
            electorate = electorate.len(),
            ballots = ballots.len(),
            epoch,
            "closing a ratification, frozen"
        );
        let delta = coordinator::ratify_close_frozen(
            target,
            &electorate,
            &ballots,
            epoch,
            seq,
            prev,
            self.chat_type_id(&group_id)?,
        );
        self.append_and_flush(&mut group, &group_id, delta).await?;
        Ok(())
    }

    /// The folded ratify projection of a (forum) object — each proposal with its
    /// rule, DERIVED outcome, ballot tallies, and this device's own ballot.
    pub fn object_ratify(&self, object_id_hex: &str) -> Result<Vec<RatifyView>, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        let coord = self.folded_forum(&group_id)?;
        let rs = coord.ratify_state();
        let owner = coord.owner();
        let me = self.id.identity_pk();
        let mut out: Vec<RatifyView> = rs
            .proposals
            .iter()
            .map(|(pid, prop)| {
                // The counts shown are the counts that decided it: once closed, the
                // close's own ballots, so the numbers never contradict the outcome.
                let votes = rs.tally(pid);
                let count = |b: coordinator::Ballot| {
                    votes
                        .map(|m| m.values().filter(|v| **v == b).count())
                        .unwrap_or(0) as u32
                };
                RatifyView {
                    proposer: prop.proposer,
                    gen: prop.gen,
                    payload: prop.payload.clone(),
                    rule: prop.rule.as_str().to_string(),
                    outcome: rs.outcome(pid, &owner).as_str().to_string(),
                    approve: count(coordinator::Ballot::Approve),
                    reject: count(coordinator::Ballot::Reject),
                    abstain: count(coordinator::Ballot::Abstain),
                    mine_ballot: votes
                        .and_then(|m| m.get(&me))
                        .map(|b| b.as_str().to_string())
                        .unwrap_or_default(),
                }
            })
            .collect();
        out.sort_by_key(|v| (v.gen, v.proposer));
        Ok(out)
    }

    // ---- Project (#22) — author + project the Work-tab object ---------------

    /// Fold a project's persisted log into a `Coordinator<ProjectType>`. The log is
    /// stored in content-address order, but the SEQUENCED spine requires strict
    /// `(epoch, seq)` succession — so deliver sequenced deltas in `(epoch, seq)`
    /// order first (commutative deltas are order-independent, the OR-set).
    fn folded_project(
        &self,
        group_id: &[u8],
    ) -> Result<coordinator::Coordinator<ProjectType>, CoreError> {
        let members = self.dir.group_members(group_id)?;
        // The owner at each delta's epoch (§10.2), not only today's: a spine delta the
        // previous owner wrote before a handover is still theirs to have written.
        let owners = self.dir.owner_history(group_id)?;
        let mut entries: Vec<(coordinator::Delta, [u8; 32])> = Vec::new();
        for (author, envelope) in self.dir.load_log(group_id)? {
            entries.push((coordinator::decode_delta(&envelope)?, author));
        }
        entries.sort_by_key(|(d, _)| (d.seq.is_none(), d.epoch, d.seq.unwrap_or(0)));
        let mut coord = coordinator::Coordinator::<ProjectType>::with_owners(members, owners);
        for (delta, author) in entries {
            coord.deliver(delta, author)?;
        }
        Ok(coord)
    }

    /// The compiled [`ProjectState`], folded over the current roster.
    pub fn project_state(&self, object_id_hex: &str) -> Result<ProjectState, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        Ok(self.folded_project(&group_id)?.state())
    }

    /// The project's dependency edges as `(edge_id, from_item, to_item, kind)` — the
    /// list the UI needs to SHOW and REMOVE a dependency. `blocked` is DERIVED from
    /// these unresolved edges, but the edges themselves are otherwise unexposed.
    pub fn project_dependencies(
        &self,
        object_id_hex: &str,
    ) -> Result<Vec<(String, String, String, String)>, CoreError> {
        Ok(self
            .project_state(object_id_hex)?
            .deps
            .into_iter()
            .map(|(edge, d)| (edge, d.from, d.to, d.kind.as_str().to_string()))
            .collect())
    }

    /// The live Gantt as one human-readable line per non-suppressed item — the
    /// CLI/FFI board view (`title · Kind · Status[ blocked]`).
    pub fn project_board(&self, object_id_hex: &str) -> Result<Vec<String>, CoreError> {
        Ok(self
            .project_state(object_id_hex)?
            .board()
            .into_iter()
            .map(|v| {
                let status = v
                    .status
                    .map(|s| format!("{s:?}"))
                    .unwrap_or_else(|| "—".to_string());
                let blocked = if v.blocked { " [blocked]" } else { "" };
                format!("{} · {:?} · {}{}", v.title, v.kind, status, blocked)
            })
            .collect())
    }

    /// The `(seq, prev)` the next sequenced project delta must carry — folds the
    /// persisted spine and stamps against the group's current MLS epoch.
    fn project_next_seq(&self, group_id: &[u8], epoch: u64) -> Result<(u64, [u8; 32]), CoreError> {
        Ok(coordinator::next_sequenced_pos(
            self.folded_project(group_id)?.sequenced_head(),
            epoch,
        ))
    }


    // ======================================================================
    // GROUP — the reusable identity record (person/team/org). Same object
    // machinery as Project: a group-of-1 log folded by `Coordinator<GroupType>`.
    // The Group's OWN Delta log is authoritative for profile/presence/vault/
    // roles; MEMBERSHIP stays the MLS roster (never stored in the log). vCard is
    // an interchange projection over this state, never a second store.
    // ======================================================================

    /// Fold a group object's persisted log into a `Coordinator<GroupType>` —
    /// sequenced spine in `(epoch, seq)` order first, mirroring `folded_project`.
    fn folded_group(
        &self,
        group_id: &[u8],
    ) -> Result<coordinator::Coordinator<GroupType>, CoreError> {
        crate::object_store::GroupObjectStore::new(&self.dir)
            .folded::<crate::group::GroupType>(group_id)
    }

    // -- Thing (the market object) ------------------------------------------

    /// Fold a Thing object's log. Filters to the Thing type id for the same reason
    /// `folded_group` does: one log can carry several op-groups, and the Coordinator
    /// hard-rejects a foreign type_id.
    fn folded_thing(
        &self,
        group_id: &[u8],
    ) -> Result<coordinator::Coordinator<crate::thing::ThingType>, CoreError> {
        crate::object_store::GroupObjectStore::new(&self.dir)
            .folded::<crate::thing::ThingType>(group_id)
    }

    /// A POST's spine, folded. One store, one fold path — the same seam every kind uses.
    fn folded_post(
        &self,
        group_id: &[u8],
    ) -> Result<coordinator::Coordinator<crate::post::PostType>, CoreError> {
        crate::object_store::GroupObjectStore::new(&self.dir)
            .folded::<crate::post::PostType>(group_id)
    }

    /// The next `(seq, prev)` on a POST's spine — folds its OWN lens (per-type
    /// `sequenced_head`; the `thing_next_seq` trap applies verbatim).
    fn post_next_seq(&self, group_id: &[u8], epoch: u64) -> Result<(u64, [u8; 32]), CoreError> {
        Ok(coordinator::next_sequenced_pos(
            self.folded_post(group_id)?.sequenced_head(),
            epoch,
        ))
    }

    /// The next `(seq, prev)` on a THING's spine.
    ///
    /// It must fold the THING lens, not the Group one: `sequenced_head` is per-type, and
    /// asking the Group fold about a Thing object returns genesis every time — which stamps
    /// every delta at seq 0 and makes the second one a `ForkDetected` at read.
    fn thing_next_seq(&self, group_id: &[u8], epoch: u64) -> Result<(u64, [u8; 32]), CoreError> {
        Ok(coordinator::next_sequenced_pos(
            self.folded_thing(group_id)?.sequenced_head(),
            epoch,
        ))
    }

    /// The compiled [`crate::thing::ThingState`] for one Thing object.
    pub fn thing_state(&self, object_id_hex: &str) -> Result<crate::thing::ThingState, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        Ok(self.folded_thing(&group_id)?.state())
    }

    /// Every Thing we own, folded: `(object_id, ThingState)`. The LIFE market list reads
    /// this — minted GroupObjects are the durable truth, the resolver graph is the derived
    /// read-index. An object whose log fails to fold is reported loudly rather than skipped:
    /// a thing that silently vanishes from your market is worse than an error.
    pub fn things(&self) -> Result<Vec<(String, crate::thing::ThingState)>, CoreError> {
        let mut out = Vec::new();
        for (gid, kind) in self.dir.list_objects()? {
            if kind != "thing" {
                continue;
            }
            let state = self.folded_thing(&gid)?.state();
            out.push((hex::encode(gid), state));
        }
        Ok(out)
    }


    // -- System (an external source, and the door its items come through) -----

    /// Fold a System object's log. Filters to the System type id for the same reason
    /// `folded_thing` does: one log can carry several op-groups, and the Coordinator
    /// hard-rejects a foreign type_id.


    pub fn host_sites(&self, as_publisher: &[u8; 32]) -> Result<HostScan, CoreError> {
        let mut scan = HostScan::default();
        for (gid, kind) in self.dir.list_objects()? {
            if kind != "host" {
                continue;
            }
            let object_id = hex::encode(&gid);
            let st = match self.folded_host(&gid) {
                Ok(c) => c.state(),
                Err(e) => {
                    scan.refused.push((object_id, format!("will not fold: {e}")));
                    continue;
                }
            };
            // NO is_host() CHECK. On branch host-faces a Host was a System whose
            // connector happened to be "wallflowers.host", so this scan had to walk
            // every System and skip the ones that were not Hosts. A Host is kind 33
            // now (25 Sep 2026): everything this loop reaches is one by construction,
            // and "not a refusal, just not a Host" is a state that no longer exists.
            if !st.publication.is_published() {
                scan.refused.push((object_id, "not published".into()));
                continue;
            }
            if st.publication.publisher.as_ref() != Some(as_publisher) {
                scan.refused.push((object_id, "published by another Arc".into()));
                continue;
            }
            // REMOVED IS REMOVED, whatever this device still holds. A removed member
            // keeps the log it folded before the commit, so the roster it last saw
            // still names it; the departure is the fact that matters.
            if self.dir.is_departed(&gid).unwrap_or(false) {
                scan.refused.push((object_id, "this Arc was removed from the Host".into()));
                continue;
            }
            match self.group_roster(&object_id) {
                Ok(r) if r.contains(as_publisher) => {}
                Ok(_) => {
                    scan.refused.push((object_id, "this Arc was removed from the Host".into()));
                    continue;
                }
                Err(e) => {
                    scan.refused.push((object_id, format!("roster unreadable: {e}")));
                    continue;
                }
            }
            let owner = match self.group_owner_id(&gid) {
                Ok(o) => hex::encode(o),
                Err(e) => {
                    scan.refused.push((object_id, format!("owner unreadable: {e}")));
                    continue;
                }
            };
            let by_group = |i: &&crate::system::HydratedItem| !i.withdrawn && &i.author != as_publisher;
            let bundle = st
                .items
                .get(HOST_FACE_KEY)
                .filter(|i| by_group(i))
                .map(|i| i.payload.clone())
                .unwrap_or_default();
            let items = st
                .items
                .values()
                .filter(|i| i.key.starts_with("event:") || i.key.starts_with("post:"))
                .filter(|i| by_group(i))
                .map(|i| HostItem { key: i.key.clone(), payload: i.payload.clone(), fetched_at: i.fetched_at })
                .collect();
            let media = st.media.iter().map(|(k, m)| (k.clone(), m.mime.clone())).collect();
            scan.sites.push(HostSite {
                object_id,
                slug: st.publication.slug.clone(),
                name: st.name.clone(),
                owner,
                bundle,
                items,
                media,
            });
        }
        Ok(scan)
    }

    pub fn host_media(
        &self,
        host_id_hex: &str,
        slot: &str,
        as_publisher: &[u8; 32],
    ) -> Result<Option<(String, Vec<u8>)>, CoreError> {
        let gid = match self.object_group_id(host_id_hex) {
            Ok(g) => g,
            Err(_) => return Ok(None),
        };
        let st = self.folded_host(&gid)?.state();
        if !st.publication.is_published()
            || st.publication.publisher.as_ref() != Some(as_publisher)
            || !self.group_roster(host_id_hex)?.contains(as_publisher)
        {
            return Ok(None);
        }
        use base64::Engine as _;
        Ok(match st.media.get(slot) {
            Some(m) => Some((
                m.mime.clone(),
                base64::engine::general_purpose::STANDARD
                    .decode(&m.data)
                    .map_err(|e| CoreError::Coordinator(format!("host picture {slot}: {e}")))?,
            )),
            None => None,
        })
    }

    // -- Host, the owner's side: over `mint` and `apply`, the one write path ------

    /// A Host for this Site: its public copy, and the only object an Arc joins.
    /// `mint` names it (`host.define`), and both halves of `part_of` say whose it is:
    /// the Site's `base.setPart` and the Host's `base.setParent`, role `host`. Owner
    /// of the Site only. Returns the Host's id.
    pub async fn host_new(&self, group_id_hex: &str, name: &str) -> Result<String, CoreError> {
        self.require_kind(group_id_hex, "group")?;
        if self.group_owner_id(&self.object_group_id(group_id_hex)?)? != self.id.identity_pk() {
            return Err(CoreError::Coordinator("only the Site's owner makes its Host".into()));
        }
        let draft = crate::mint::MintDraft { name: name.to_string(), ..Default::default() };
        let host = self.mint(ObjectKind::Host, &draft).await?;
        self.compose(group_id_hex, &host, HOST_ROLE, unix_millis() as i64).await?;
        Ok(host)
    }

    /// Publish a face on a Host: its pictures, the bundle, then the address naming the
    /// Arc that serves it. The Arc must already be on the Host's roster (add it with
    /// [`Self::group_add_member`] and its `/v1/bundle`): `base.publish` refuses a
    /// publisher that is not. `media` is `(slot, mime, base64)`; empty base64 clears.
    ///
    /// Everything is checked BEFORE the first delta: half a face published and then a
    /// refusal is worse than a refusal, and an oversize picture would be written here
    /// and never delivered (`host::MAX_MEDIA_B64`).
    pub async fn host_publish(
        &self,
        host_id_hex: &str,
        slug: &str,
        arc_hex: &str,
        bundle_json: &str,
        media: &[(String, String, String)],
    ) -> Result<(), CoreError> {
        self.require_kind(host_id_hex, "host")?;
        if !crate::publication::valid_slug(slug) {
            return Err(CoreError::Coordinator(format!("'{slug}' is not a valid address")));
        }
        if bundle_json.len() > crate::system::MAX_HYDRATED_PAYLOAD
            || !matches!(serde_json::from_str::<serde_json::Value>(bundle_json), Ok(serde_json::Value::Object(_)))
        {
            return Err(CoreError::Coordinator(format!(
                "the face bundle must be a JSON object of at most {} bytes",
                crate::system::MAX_HYDRATED_PAYLOAD
            )));
        }
        let arc: [u8; 32] = hex::decode(arc_hex)
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or_else(|| CoreError::Coordinator(format!("'{arc_hex}' is not an identity key")))?;
        if !self.group_roster(host_id_hex)?.contains(&arc) {
            return Err(CoreError::Coordinator(
                "the Arc is not on this Host — add it with its bundle first".into(),
            ));
        }
        for (slot, mime, data) in media {
            crate::host::check_media(slot, data, mime).map_err(CoreError::Coordinator)?;
        }
        let held = self.folded_host(&self.object_group_id(host_id_hex)?)?.state().publication;

        for (slot, mime, data) in media {
            self.apply(host_id_hex, crate::host::OP_SET_MEDIA, crate::host::set_media_args(slot, data, mime))
                .await?;
        }
        let now = unix_millis();
        self.apply(
            host_id_hex,
            crate::host::OP_HYDRATE,
            crate::host::hydrate_args(HOST_FACE_KEY, bundle_json, now as i64, now, false),
        )
        .await?;
        if held.slug != slug || held.publisher != Some(arc) {
            self.apply(host_id_hex, crate::publication::OP_PUBLISH, crate::publication::publish_args(slug, &arc))
                .await?;
        }
        Ok(())
    }

    /// Put one live item on a Host's face (`event:<id>` / `post:<id>`), or withdraw it.
    pub async fn host_put_item(
        &self,
        host_id_hex: &str,
        key: &str,
        payload_json: &str,
        withdrawn: bool,
    ) -> Result<(), CoreError> {
        self.require_kind(host_id_hex, "host")?;
        if !(key.starts_with("event:") || key.starts_with("post:")) {
            return Err(CoreError::Coordinator(format!("'{key}' is not a live item key (event: / post:)")));
        }
        let now = unix_millis();
        self.apply(
            host_id_hex,
            crate::host::OP_HYDRATE,
            crate::host::hydrate_args(key, payload_json, now as i64, now, withdrawn),
        )
        .await?;
        Ok(())
    }

    /// Take a Host's face down. The Arc stops at its next scan; what visitors already
    /// saw, the web keeps — the ICD's one-way-door warning on `base.publish`.
    pub async fn host_unpublish(&self, host_id_hex: &str) -> Result<(), CoreError> {
        self.require_kind(host_id_hex, "host")?;
        self.apply(host_id_hex, crate::publication::OP_UNPUBLISH, crate::publication::unpublish_args())
            .await?;
        Ok(())
    }

    /// The Site's own events and posts onto its Host's `event:<id>` / `post:<id>` items,
    /// and what left withdrawn (O-48; Ralph, 27 Sep: "The Face needs to be hydrated. This
    /// happens via the Group <-> Host route"). The owner's device writes it: the Host's
    /// roster is the owner's devices and the Arc, and the Arc may not hydrate the page it
    /// serves. What is wanted is `face_items`'; this folds, diffs and writes. An unchanged
    /// Site writes nothing. Each write's rev is above the one held, so it wins on every
    /// device whatever the clocks say.
    pub async fn host_sync_items(&self, site_hex: &str) -> Result<HostSync, CoreError> {
        self.host_sync_items_with(site_hex, true).await
    }

    async fn host_sync_items_with(&self, site_hex: &str, send: bool) -> Result<HostSync, CoreError> {
        use crate::face_items::{Attached, Source, Write};
        self.require_kind(site_hex, "group")?;
        let site_gid = self.object_group_id(site_hex)?;
        if self.group_owner_id(&site_gid)? != self.id.identity_pk() {
            return Err(CoreError::Coordinator("only the Site's owner hydrates its Host".into()));
        }
        let site = self.folded_group(&site_gid)?.state();
        let host = site
            .parts
            .iter()
            .find(|(_, p)| p.role == HOST_ROLE)
            .map(|(id, _)| id.clone())
            .ok_or_else(|| CoreError::Coordinator("this Site has no Host".into()))?;
        let held = self.folded_host(&self.object_group_id(&host)?)?.state();
        if !held.parent.as_ref().is_some_and(|p| p.parent == site_hex && p.role == HOST_ROLE) {
            return Err(CoreError::Coordinator(format!(
                "the Site names {host} as its Host, and the Host does not name the Site"
            )));
        }

        // What the Site names as created, folded. One that is not held or will not fold
        // is named, and the rest go on.
        let mut left: Vec<(String, String)> = Vec::new();
        let (mut events, mut posts) = (Vec::new(), Vec::new());
        for (id, a) in site.affiliations.iter().filter(|(_, a)| a.rel == crate::group::AffiliationRel::Created) {
            let folded = match self.object_kind(id).as_deref() {
                Ok("event") => self.object_group_id(id).and_then(|g| self.folded_event(&g)).map(|c| events.push((id, a.at, c.state()))),
                Ok("post") => self.object_group_id(id).and_then(|g| self.folded_post(&g)).map(|c| posts.push((id, a.at, c.state()))),
                Ok(_) => Ok(()),
                Err(_) => {
                    left.push((id.clone(), "not held on this device".into()));
                    continue;
                }
            };
            if let Err(e) = folded {
                left.push((id.clone(), format!("does not fold: {e}")));
            }
        }
        let attached: Vec<Attached> = events
            .iter()
            .map(|(id, at, s)| Attached { id, at: *at, source: Source::Event(s) })
            .chain(posts.iter().map(|(id, at, s)| Attached { id, at: *at, source: Source::Post(s) }))
            .collect();
        // PERFORMS_AT (ASSURANCE 2): an act goes on the Face only where this device holds the
        // performer's own half, group.setAffiliation {rel: performs_at, peer: the event}: an
        // organisation's group it holds, or, for a person, their self record, which only they
        // hold. Another person's act stays inside MLS.
        let mut halves: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> = Default::default();
        for (id, _, e) in &events {
            for act in e.acts.iter().flatten() {
                if self.holds_performs_at(id, &act.performer) {
                    halves.entry(id.to_string()).or_default().insert(act.performer.id());
                }
            }
        }
        let now = unix_millis();
        // The venue stays to members where the Site's registration says so (setRegistration's
        // `location`), read from the Site's state this sync already holds.
        let members_venue: std::collections::BTreeSet<String> = site
            .rsvps
            .registration
            .iter()
            .filter(|(_, r)| r.location == crate::rsvp::Location::Members)
            .map(|(e, _)| e.clone())
            .collect();
        let (desired, off) = crate::face_items::desired_with(site_hex, &attached, now as i64, &halves, &members_venue);
        left.extend(off.into_iter().map(|(id, why)| (id, why.to_string())));

        let mut out = HostSync { host: host.clone(), left, ..Default::default() };
        for w in crate::face_items::plan(&desired, &held.items) {
            let (key, payload, withdrawn) = match &w {
                Write::Put { key, payload } => (key, payload.as_str(), false),
                Write::Withdraw { key } => (key, "", true),
            };
            let rev = held.items.get(key).map_or(now, |h| now.max(h.rev + 1));
            let args = crate::host::hydrate_args(key, payload, now as i64, rev, withdrawn);
            if send {
                self.apply(&host, crate::host::OP_HYDRATE, args).await?;
            } else {
                self.apply_local(&host, crate::host::OP_HYDRATE, args).await?;
            }
            if withdrawn { &mut out.withdrawn } else { &mut out.put }.push(key.clone());
        }
        Ok(out)
    }

    /// `host_sync_items` for every Site this person owns that has a Host: what the Door
    /// runs after each write and on its tick. One Site's refusal is named with it and the
    /// rest go on; a Site whose log will not fold is `noncompliant_objects`' to name.
    pub async fn host_sync_all(&self) -> Result<Vec<(String, Result<HostSync, String>)>, CoreError> {
        self.host_sync_all_with(true).await
    }

    /// `host_sync_all`, its item writes local commits (O-69): what the Door runs before a
    /// write's answer, so the Face has the write when it is answered; the write's tail sends
    /// them with it.
    pub async fn host_sync_all_local(&self) -> Result<Vec<(String, Result<HostSync, String>)>, CoreError> {
        self.host_sync_all_with(false).await
    }

    async fn host_sync_all_with(&self, send: bool) -> Result<Vec<(String, Result<HostSync, String>)>, CoreError> {
        let me = self.id.identity_pk();
        let mut out = Vec::new();
        for (gid, kind) in self.dir.list_objects()? {
            if kind != "group" || self.group_owner_id(&gid).ok() != Some(me) {
                continue;
            }
            let Ok(site) = self.folded_group(&gid) else { continue };
            if !site.state().parts.values().any(|p| p.role == HOST_ROLE) {
                continue;
            }
            let id = hex::encode(&gid);
            let done = self.host_sync_items_with(&id, send).await.map_err(|e| e.to_string());
            out.push((id, done));
        }
        Ok(out)
    }

    /// Fold a Host's log. Same shape as `folded_system`, and separate because a Host
    /// is its own kind (33) since 25 Sep 2026 — it was a System wearing the connector
    /// `wallflowers.host`, and every host-only op had to ask `is_host()` at reduce
    /// time. The kind is that statement now.
    fn folded_host(
        &self,
        group_id: &[u8],
    ) -> Result<coordinator::Coordinator<crate::host::HostType>, CoreError> {
        let members = self.dir.group_members(group_id)?;
        let owners = self.dir.owner_history(group_id)?;
        let tid = ObjectKind::Host.type_id() as u32;
        let mut entries: Vec<(coordinator::Delta, [u8; 32])> = Vec::new();
        for (author, envelope) in self.dir.load_log(group_id)? {
            let d = coordinator::decode_delta(&envelope)?;
            if d.type_id == tid {
                entries.push((d, author));
            }
        }
        entries.sort_by_key(|(d, _)| (d.seq.is_none(), d.epoch, d.seq.unwrap_or(0)));
        let mut coord = coordinator::Coordinator::<crate::host::HostType>::with_owners(members, owners);
        for (delta, author) in entries {
            coord.deliver(delta, author)?;
        }
        Ok(coord)
    }

    fn folded_system(
        &self,
        group_id: &[u8],
    ) -> Result<coordinator::Coordinator<crate::system::SystemType>, CoreError> {
        let members = self.dir.group_members(group_id)?;
        // The owner at each delta's epoch (§10.2), not only today's: a spine delta the
        // previous owner wrote before a handover is still theirs to have written.
        let owners = self.dir.owner_history(group_id)?;
        let tid = ObjectKind::System.type_id() as u32;
        let mut entries: Vec<(coordinator::Delta, [u8; 32])> = Vec::new();
        for (author, envelope) in self.dir.load_log(group_id)? {
            let d = coordinator::decode_delta(&envelope)?;
            if d.type_id == tid {
                entries.push((d, author));
            }
        }
        entries.sort_by_key(|(d, _)| (d.seq.is_none(), d.epoch, d.seq.unwrap_or(0)));
        let mut coord = coordinator::Coordinator::<crate::system::SystemType>::with_owners(members, owners);
        for (delta, author) in entries {
            coord.deliver(delta, author)?;
        }
        Ok(coord)
    }

    /// The next `(seq, prev)` on a SYSTEM's spine — folds its OWN lens (per-type
    /// `sequenced_head`; the `thing_next_seq` trap applies verbatim).
    fn system_next_seq(&self, group_id: &[u8], epoch: u64) -> Result<(u64, [u8; 32]), CoreError> {
        Ok(coordinator::next_sequenced_pos(
            self.folded_system(group_id)?.sequenced_head(),
            epoch,
        ))
    }

    /// The compiled [`crate::system::SystemState`] for one System object.
    pub fn system_state(
        &self,
        object_id_hex: &str,
    ) -> Result<crate::system::SystemState, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        Ok(self.folded_system(&group_id)?.state())
    }

    /// Every System this device holds: `(object_id, SystemState)`. The app dispatches a
    /// connector on `state.connector`, so this is how it finds where to hydrate INTO.
    pub fn systems(&self) -> Result<Vec<(String, crate::system::SystemState)>, CoreError> {
        let mut out = Vec::new();
        for (gid, kind) in self.dir.list_objects()? {
            if kind != "system" {
                continue;
            }
            let state = self.folded_system(&gid)?.state();
            out.push((hex::encode(gid), state));
        }
        Ok(out)
    }

    /// The System object for a connector id ("music.ra"), if this device has one.
    /// Hydration is keyed by connector rather than object id so a connector never has
    /// to remember which group it wrote into last time.
    pub fn system_for_connector(&self, connector: &str) -> Result<Option<String>, CoreError> {
        Ok(self
            .systems()?
            .into_iter()
            .find(|(_, st)| st.connector == connector)
            .map(|(id, _)| id))
    }


    // -- System (an external source, and the door its items come through) -----


    /// Hydrate a BATCH of items into the System for `connector`, minting the System
    /// object on first use.
    ///
    /// Returns how many deltas were actually authored — which is the count that MOVED,
    /// not the count offered: an item whose `rev` the fold already holds is skipped
    /// here rather than written and then ignored, because the steady state of every
    /// connector is re-offering what it already sent. Writing those would grow a log
    /// that syncs to every member, for no change in state.
    pub async fn system_hydrate(
        &self,
        connector: &str,
        display_name: &str,
        scope: Option<&str>,
        items: &[(String, String, i64, u64, bool)], // (key, payload, fetched_at, rev, withdrawn)
    ) -> Result<usize, CoreError> {
        let object_id = match self.system_for_connector(connector)? {
            Some(id) => id,
            None => {
                let id = self.object_new("system", display_name)?;
                self.apply(
                    &id,
                    crate::system::OP_DEFINE,
                    crate::system::define_args(display_name, connector, scope),
                )
                .await?;
                id
            }
        };
        let held = self.system_state(&object_id)?;
        let mut written = 0usize;
        for (key, payload, fetched_at, rev, withdrawn) in items {
            if let Some(h) = held.item(key) {
                if *rev <= h.rev && *withdrawn == h.withdrawn {
                    continue; // the fold already holds this exact revision
                }
            }
            self.apply(
                &object_id,
                crate::system::OP_HYDRATE,
                crate::system::hydrate_args(key, payload, *fetched_at, *rev, *withdrawn),
            )
            .await?;
            written += 1;
        }
        Ok(written)
    }

    // -- Event (an occurrence with a when — and the ticketing object) ---------

    /// Fold an Event object's log — through the store, like every other kind.
    fn folded_event(
        &self,
        group_id: &[u8],
    ) -> Result<coordinator::Coordinator<crate::event::EventType>, CoreError> {
        crate::object_store::GroupObjectStore::new(&self.dir)
            .folded::<crate::event::EventType>(group_id)
    }

    /// The next `(seq, prev)` on an EVENT's spine — folds its OWN lens (per-type
    /// `sequenced_head`; the `thing_next_seq` trap applies verbatim).
    fn event_next_seq(&self, group_id: &[u8], epoch: u64) -> Result<(u64, [u8; 32]), CoreError> {
        Ok(coordinator::next_sequenced_pos(
            self.folded_event(group_id)?.sequenced_head(),
            epoch,
        ))
    }

    /// The compiled [`crate::event::EventState`] for one Event object.
    pub fn event_state(&self, object_id_hex: &str) -> Result<crate::event::EventState, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        Ok(self.folded_event(&group_id)?.state())
    }

    /// The stable listing key for one Event — what travels to buyers and the box
    /// office instead of the group id.
    pub fn event_listing_id(&self, object_id_hex: &str) -> Result<String, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        let owner = self.group_owner_id(&group_id)?;
        Ok(crate::event::listing_id(&group_id, &owner))
    }

    /// Every Event we hold, folded: `(object_id, EventState)`. Loud on a fold failure —
    /// an event that silently vanishes from EVENTS is worse than an error.
    /// Every Post we hold, folded. The same shape `events()` has — one store, one fold
    /// path — so a new kind costs a function, not a mechanism.
    pub fn posts(&self) -> Result<Vec<(String, crate::post::PostState)>, CoreError> {
        let store = crate::object_store::GroupObjectStore::new(&self.dir);
        let mut out = Vec::new();
        for r in store.by_kind("post")? {
            let state = self.folded_post(&r.id)?.state();
            out.push((hex::encode(r.id), state));
        }
        Ok(out)
    }

    pub fn events(&self) -> Result<Vec<(String, crate::event::EventState)>, CoreError> {
        let store = crate::object_store::GroupObjectStore::new(&self.dir);
        let mut out = Vec::new();
        for r in store.by_kind("event")? {
            let state = self.folded_event(&r.id)?.state();
            out.push((hex::encode(r.id), state));
        }
        Ok(out)
    }


    /// EVERY RELAY TAG THIS DEVICE LISTENS ON, lowercase hex — what the wake tier
    /// must watch to know this phone has mail (see coordination/RINGFENCE.txt §6).
    ///
    /// A PURE DIRECTORY READ, deliberately: no MLS group is loaded, no epoch is
    /// advanced, no relay is touched. `record_epoch_tag` already persists each
    /// epoch's tag before the drain that uses it, so everything needed is on disk —
    /// and a read that cannot mutate is a read the app may call on every sync tick
    /// without wondering what it costs.
    ///
    /// The set is exactly what `sync_once` drains:
    ///   * the intro mailbox (where a stranger's pairing Welcome lands),
    ///   * every retained epoch tag of every group — the CURRENT one plus the
    ///     `EPOCH_RETENTION` window `sync` still re-drains, so a message that raced
    ///     a rotation is not missed,
    ///   * each open Place's doorbell (where a knock arrives).
    ///
    /// Tags ROTATE per epoch, so this is a snapshot with a short life: the caller
    /// re-registers whenever the set changes. Deduped and sorted so an unchanged
    /// set hashes identically and the app can skip a pointless re-registration.
    pub fn wake_tags(&self) -> Result<Vec<String>, CoreError> {
        let mut tags: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        // PUBLIC addresses only. The intro value and a doorbell are SEEDS — whoever
        // holds one can write there — so the waker is handed the key each names,
        // never the secret behind it.
        tags.insert(Address::from_seed(&self.dir.my_intro_tag()?).tag_hex());
        for (group_id, _kind) in self.dir.all_groups()? {
            // Addresses only — this loop never wanted the secret, and reading one
            // per row would put the device's entire wake set behind the at-rest key.
            for tag in self.dir.epoch_tag_addresses(&group_id)? {
                tags.insert(hex::encode(tag));
            }
        }
        // Place doorbells: a knock is addressed to the doorbell, not to a group tag,
        // so a place whose owner is asleep would otherwise never be woken.
        for (gid, kind) in self.dir.list_objects()? {
            if kind != "place" {
                continue;
            }
            if let Ok(st) = self.folded_place(&gid).map(|c| c.state()) {
                if let Some(bell) = st.doorbell {
                    tags.insert(Address::from_seed(&bell).tag_hex());
                }
            }
        }
        Ok(tags.into_iter().collect())
    }

    /// The connection channel we share with `peer`, if one exists — the road the two
    /// ticket legs ride.
    fn connection_group_with(&self, peer: &[u8; 32]) -> Result<Vec<u8>, CoreError> {
        for (group_id, kind) in self.dir.all_groups()? {
            if kind != "connection" {
                continue;
            }
            if self.dir.group_members(&group_id)?.contains(peer) {
                return Ok(group_id);
            }
        }
        Err(CoreError::Directory(format!(
            "no connection with peer {}",
            hex::encode(&peer[..6])
        )))
    }


    /// The folded Contact-channel state we share with `peer` — where the app reads
    /// the ticket wallet and the fulfilment agent reads pending requests.
    pub fn contact_state_with(
        &self,
        peer: &[u8; 32],
    ) -> Result<crate::contact::ContactState, CoreError> {
        let group_id = self.connection_group_with(peer)?;
        self.folded_contact(&group_id)
    }

    /// Mint the signed credentials for one recorded sale, with THIS node's identity
    /// key as the signer. The fulfilment path: the seller (owner on free events, the
    /// delegate on paid ones) records the sale, then issues, then delivers. Each core
    /// is serialized ONCE here — from now on the string is the object, verbatim.
    pub fn event_issue_credentials(
        &self,
        object_id_hex: &str,
        pi: &str,
    ) -> Result<Vec<(String, String)>, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        let state = self.folded_event(&group_id)?.state();
        let sale = state.ledger.get(pi).ok_or_else(|| {
            CoreError::Coordinator(format!("no sale {pi} in the ledger of {object_id_hex}"))
        })?;
        if sale.author != self.id.identity_pk() {
            return Err(CoreError::Coordinator(format!(
                "sale {pi} was recorded by another seller — only its author issues its tickets"
            )));
        }
        let owner = self.group_owner_id(&group_id)?;
        let listing = crate::event::listing_id(&group_id, &owner);
        let issued_at = unix_now();
        let mut out = Vec::with_capacity(sale.tickets.len());
        for ticket in &sale.tickets {
            let core = crate::event::TicketCore {
                v: 1,
                listing: listing.clone(),
                ticket: ticket.clone(),
                title: state.title.clone(),
                start_ms: state.start_ms,
                buyer: hex::encode(sale.buyer),
                issued_at,
            };
            out.push(core.to_signed_json(&self.id));
        }
        Ok(out)
    }

    /// SLICE 0 — late-joiner ledger convergence by STATE RE-EMISSION.
    ///
    /// A member joining at epoch N structurally cannot decrypt earlier traffic (MLS
    /// forward secrecy; the pinned no-backfill doctrine — `replay_history` was
    /// deliberately reverted because re-encrypting old ciphertext defeats it). Late
    /// door staff converge because the ledger's LEGITIMATE HOLDERS re-author their
    /// own current state at the NEW epoch:
    ///
    ///   * the OWNER re-authors the current listing (a fresh spine delta — LWW by
    ///     sequence, so holders simply take the restated latest);
    ///   * the SELLER (owner or delegate) re-authors every ledger row it authored
    ///     (idempotent by `pi` — holders skip, joiners insert, states converge);
    ///   * every device re-authors its OWN redemptions (idempotent by
    ///     `(ticket, author)` — the `Redemption.authors` set).
    ///
    /// Call after adding a member to an event group. Returns how many deltas were
    /// re-emitted.
    pub async fn event_reemit(&self, object_id_hex: &str) -> Result<usize, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        let me = self.id.identity_pk();
        let state = self.folded_event(&group_id)?.state();
        let mut emitted = 0usize;

        if self.group_owner_id(&group_id)? == me {
            if !state.title.is_empty() && state.start_ms > 0 {
                let args = crate::event::set_profile_args(
                    &state.title,
                    (!state.descriptor.is_empty()).then_some(state.descriptor.as_str()),
                    state.start_ms,
                    state.end_ms,
                    (!state.venue.is_empty()).then_some(state.venue.as_str()),
                    // Re-emitting the profile must carry the repeat rule too, or a
                    // recurring event quietly becomes a one-off on every rebroadcast.
                    (!state.recurrence.is_empty()).then_some(state.recurrence.as_str()),
                    // Same for the lineup — a rebroadcast must not silently unbill.
                    (!state.lineup.is_empty())
                        .then(|| state.lineup.join("\n"))
                        .as_deref(),
                );
                self.apply(object_id_hex, crate::event::OP_SET_PROFILE, args)
                    .await?;
                emitted += 1;
            }
            if !state.media.is_empty() {
                let m = &state.media;
                let args = crate::event::set_media_args(
                    (!m.banner.is_empty()).then_some((m.banner.as_str(), m.banner_mime.as_str())),
                    &m.photos,
                    (!m.clip.is_empty()).then_some((m.clip.as_str(), m.clip_mime.as_str())),
                );
                self.apply(object_id_hex, crate::event::OP_SET_MEDIA, args)
                    .await?;
                emitted += 1;
            }
            if let Some(l) = state.listing.as_ref() {
                let delegate = l.delegate;
                let args = crate::event::set_tickets_args(
                    l.price_cents,
                    &l.currency,
                    l.capacity,
                    l.open,
                    l.acct.as_deref(),
                    delegate.as_ref(),
                    l.terms.as_deref(),
                    l.rev,
                );
                self.apply(object_id_hex, crate::event::OP_SET_TICKETS, args)
                    .await?;
                emitted += 1;
            }
        }
        for (pi, sale) in state.ledger.iter().filter(|(_, s)| s.author == me) {
            let ids: Vec<String> = sale.tickets.iter().cloned().collect();
            let args = crate::event::record_sale_args(
                pi,
                &sale.buyer,
                sale.qty,
                sale.unit_cents,
                sale.fee_cents,
                &sale.currency,
                &ids,
            );
            self.apply(object_id_hex, crate::event::OP_RECORD_SALE, args)
                .await?;
            emitted += 1;
        }
        for (ticket, r) in state
            .redemptions
            .iter()
            .filter(|(_, r)| r.authors.contains(&me))
        {
            let _ = r;
            self.apply(
                object_id_hex,
                crate::event::OP_REDEEM,
                crate::event::redeem_args(ticket),
            )
            .await?;
            emitted += 1;
        }
        Ok(emitted)
    }

    // -- Place (somewhere in the world) --------------------------------------

    /// Fold a Place object's log. Filters to the Place type id for the same reason
    /// `folded_thing` does: one log can carry several op-groups, and the Coordinator
    /// hard-rejects a foreign type_id.
    fn folded_place(
        &self,
        group_id: &[u8],
    ) -> Result<coordinator::Coordinator<crate::place::PlaceType>, CoreError> {
        crate::object_store::GroupObjectStore::new(&self.dir)
            .folded::<crate::place::PlaceType>(group_id)
    }

    /// The next `(seq, prev)` on a PLACE's spine. Per-type, like `thing_next_seq` —
    /// asking the Group fold about a Place returns genesis every time and forks the log.
    fn place_next_seq(&self, group_id: &[u8], epoch: u64) -> Result<(u64, [u8; 32]), CoreError> {
        Ok(coordinator::next_sequenced_pos(
            self.folded_place(group_id)?.sequenced_head(),
            epoch,
        ))
    }

    /// The compiled [`crate::place::PlaceState`] for one Place object.
    pub fn place_state(&self, object_id_hex: &str) -> Result<crate::place::PlaceState, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        Ok(self.folded_place(&group_id)?.state())
    }

    /// Every Place we own, folded: `(object_id, PlaceState)`. LIFE's PLACES list reads
    /// this — minted GroupObjects are the durable truth. An object whose log fails to
    /// fold is reported loudly rather than skipped.
    pub fn places(&self) -> Result<Vec<(String, crate::place::PlaceState)>, CoreError> {
        let mut out = Vec::new();
        for (gid, kind) in self.dir.list_objects()? {
            if kind != "place" {
                continue;
            }
            let state = self.folded_place(&gid)?.state();
            out.push((hex::encode(gid), state));
        }
        Ok(out)
    }


    /// Publish a note AT a Place — the typed face of `place.post`, routed through
    /// [`Self::place_author`] like every other Place write (the commutativity
    /// branch there puts it on the gen path, never the spine). `note` is the
    /// author's note id: publishing the same note again REPLACES their copy.
    pub async fn place_post(
        &self,
        object_id_hex: &str,
        note: &str,
        title: &str,
        text: &str,
        at: i64,
    ) -> Result<String, CoreError> {
        self.apply(
            object_id_hex,
            crate::place::OP_POST,
            crate::place::post_args(note, title, text, at),
        )
        .await
    }

    /// The notes published at one Place, newest first.
    pub fn place_posts(
        &self,
        object_id_hex: &str,
    ) -> Result<Vec<crate::place::PlacePost>, CoreError> {
        Ok(self
            .place_state(object_id_hex)?
            .posts_by_time()
            .into_iter()
            .cloned()
            .collect())
    }

    /// Take a DISCOVERED place as my own local Place so I can publish there — the
    /// reference-row → minted-object step (the RA "reference mode" shape). The
    /// remote object id lands in `GeoPoint::external_id`, which is both provenance
    /// and the MINTED-WINS dedup key: adopting the same discovery twice returns the
    /// existing Place, and `by_kind_hops` stops listing the remote row once a local
    /// counterpart carries its id.
    pub async fn place_adopt(
        &self,
        name: &str,
        descriptor: &str,
        lat: Option<f64>,
        lng: Option<f64>,
        remote_id_hex: &str,
    ) -> Result<String, CoreError> {
        if remote_id_hex.is_empty() {
            return Err(CoreError::Directory("adopt needs the discovered id".into()));
        }
        for (id, st) in self.places()? {
            if st.adopted_from() == Some(remote_id_hex) {
                return Ok(id);
            }
        }
        let location = match (lat, lng) {
            (Some(lat), Some(lng)) => {
                let mut point = crate::geo::GeoPoint::from_degrees(lat, lng);
                point.external_id = remote_id_hex.to_string();
                point.external_source = "pacific-discover".to_string();
                Some(crate::geo::LocationSource::Fixed { point })
            }
            // An unplaced discovery adopts as an unplaced Place — honest, but with
            // no point there is nowhere to carry provenance, so it cannot dedup.
            _ => None,
        };
        self.place_mint(name, descriptor, location).await
    }

    /// Set how far a Place's EXISTENCE may travel — the visibility dial, not the
    /// door ([`Self::place_set_access`] is who may join; this is who may discover).
    /// Owner-sequenced like every other assertion about one's own Place.
    ///
    /// Tightening heals outward without a retraction op: the next reconcile
    /// re-answers any live question with the reduced set (answers replace
    /// wholesale), and every asker's app-load warm refreshes their cache.
    pub async fn place_set_visibility(
        &self,
        object_id_hex: &str,
        v: crate::visibility::Visibility,
    ) -> Result<String, CoreError> {
        self.apply(
            object_id_hex,
            crate::visibility::OP_SET_VISIBILITY,
            crate::visibility::set_visibility_args(v),
        )
        .await
    }

    /// Open a Place to the public, or close it again. Owner-sequenced.
    ///
    /// OPENING IS THE CONSENT. While a Place is public, whoever scans its QR is admitted
    /// AUTOMATICALLY by this device ([`Self::reconcile_place_joins`]) — there is no
    /// per-person approval, because the owner already made that decision here.
    ///
    /// Opening does not publish the Place's contents: joiners become MEMBERS of the
    /// group, they do not read it from outside. Closing stops future admissions and
    /// evicts nobody.
    pub async fn place_set_access(
        &self,
        object_id_hex: &str,
        access: crate::place::Access,
    ) -> Result<String, CoreError> {
        self.apply(
            object_id_hex,
            crate::place::OP_SET_ACCESS,
            crate::place::set_access_args(access),
        )
        .await
    }

    /// The payload behind a PUBLIC PLACE's QR — a poster on a wall, scanned by strangers.
    ///
    /// It carries the place id and the DOORBELL — and nobody's personal identity.
    ///
    /// It used to carry the host's contact bundle, because a stranger had no other route
    /// to them. That was wrong for a sign on a gate in two ways: it published the host's
    /// `intro_tag`, which is ONE PER DEVICE and the key to their whole inbound pairing
    /// mailbox, to everyone who walked past; and it forced every joiner into a 1:1
    /// connection with whoever printed the poster. The doorbell replaces both — a
    /// per-place capability, rotatable, belonging to the place rather than a person.
    ///
    /// Wholly stable: the same printed sign keeps working until the doorbell is rotated,
    /// which is precisely what rotation is for.
    pub fn place_join_card(&self, object_id_hex: &str) -> Result<String, CoreError> {
        let st = self.place_state(object_id_hex)?;
        if !st.access.admits_strangers() {
            return Err(CoreError::Coordinator(format!(
                "place {object_id_hex} is private — open it before publishing a QR"
            )));
        }
        let doorbell = st.doorbell.ok_or_else(|| {
            CoreError::Coordinator(format!("place {object_id_hex} has no doorbell fitted"))
        })?;
        // The card names the OWNER (membership-through-mls.md §8.5): the one expectation
        // a knocker holds that an attacker inside the group cannot rewrite. Older parsers
        // ignore the extra parameter.
        let owner = self.group_owner_id(&self.object_group_id(object_id_hex)?)?;
        Ok(format!(
            "pacific://place-join?id={object_id_hex}&db={}&owner={}",
            hex::encode(doorbell),
            hex::encode(owner)
        ))
    }

    /// The owner a join card names, if it names one (§8.5). A card printed before the
    /// field existed names none, and its knock falls back to trust on first use.
    pub fn parse_place_join_card_owner(card: &str) -> Option<[u8; 32]> {
        let rest = card.trim().strip_prefix("pacific://place-join?")?;
        rest.split('&').find_map(|pair| match pair.split_once('=') {
            Some(("owner", v)) => hex::decode(v).ok().and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok()),
            _ => None,
        })
    }

    /// KNOCK ON A CARD (§8.5): parse it, remember the owner it names, then knock. The
    /// Welcome that answers is refused if its group's context names anyone else — the
    /// defence against a member who adds a newcomer and rewrites the owner in one commit.
    pub async fn place_knock_card(&self, card: &str) -> Result<(), CoreError> {
        let (place_id_hex, doorbell) = Self::parse_place_join_card(card)?;
        if let Some(owner) = Self::parse_place_join_card_owner(card) {
            let gid = hex::decode(&place_id_hex)
                .map_err(|_| CoreError::Coordinator("bad place id on the card".into()))?;
            self.dir.set_expected_owner(&gid, &owner)?;
            tracing::info!(target: "pacific::membership", place = %place_id_hex,
                           owner = %hex::encode(owner), "knocking on a card that names its owner");
        } else {
            tracing::info!(target: "pacific::membership", place = %place_id_hex,
                           "knocking on a card that names no owner — the owner is trusted on first use");
        }
        self.place_knock(&place_id_hex, &doorbell).await
    }

    /// The `why` prefix a scanner records when it pairs off a public place's QR.
    /// `"place-join:<place object id>"`.
    pub const PLACE_JOIN_WHY: &'static str = "place-join:";

    /// Parse a join card back into `(place id, doorbell)` — the inverse of
    /// [`Self::place_join_card`], for whoever was HANDED the poster rather than printing
    /// it. The app's scanner keeps its own Swift mirror (`PublicPlace.joinLink`); this is
    /// the grammar's Rust owner, and what an Arc asked to HOST a place calls — a host is
    /// given exactly the card a stranger scans, because holding the card is the whole
    /// authorization there is (a host is a knocker like any other until a member admits it).
    ///
    /// Unknown query parameters are ignored: a later card may carry more, and today's
    /// cards must keep working against tomorrow's parsers.
    pub fn parse_place_join_card(card: &str) -> Result<(String, [u8; 32]), CoreError> {
        let rest = card
            .trim()
            .strip_prefix("pacific://place-join?")
            .ok_or_else(|| CoreError::Coordinator("not a place-join card".into()))?;
        let (mut id, mut db) = (None, None);
        for pair in rest.split('&') {
            match pair.split_once('=') {
                Some(("id", v)) => id = Some(v.to_string()),
                Some(("db", v)) => db = Some(v.to_string()),
                _ => {}
            }
        }
        let id = id
            .filter(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_hexdigit()))
            .ok_or_else(|| CoreError::Coordinator("place-join card has no place id".into()))?;
        let db =
            db.ok_or_else(|| CoreError::Coordinator("place-join card has no doorbell".into()))?;
        let raw =
            hex::decode(&db).map_err(|e| CoreError::Coordinator(format!("doorbell hex: {e}")))?;
        let bell: [u8; 32] = raw
            .as_slice()
            .try_into()
            .map_err(|_| CoreError::Coordinator("doorbell is not 32 bytes".into()))?;
        Ok((id, bell))
    }

    /// Record whose land this Place sits on — an authored CLAIM, with its provenance.
    ///
    /// The core deliberately does not do the lookup: HM Land Registry's free products are
    /// monthly bulk files (INSPIRE for the parcel, CCOD/OCOD for a corporate proprietor),
    /// not a point-query API, and Scotland is a separate register again. So resolution is
    /// somebody else's job and this stores the answer, signed, with the dataset vintage
    /// attached so a reader can see how stale it is.
    ///
    /// `permission` is the part no register can answer and is not validated here — it is a
    /// self-report by whoever put the sign up. Recorded rather than enforced, so that
    /// "nobody asked" stays visible instead of becoming an assumption.
    pub async fn place_set_land(
        &self,
        place_id_hex: &str,
        claim: &crate::place::LandClaim,
    ) -> Result<String, CoreError> {
        self.apply(
            place_id_hex,
            crate::place::OP_SET_LAND,
            crate::place::set_land_args(claim),
        )
        .await
    }

    /// Retract a land claim — the lookup was wrong, or the land changed hands.
    pub async fn place_clear_land(&self, place_id_hex: &str) -> Result<String, CoreError> {
        self.apply(
            place_id_hex,
            crate::place::OP_CLEAR_LAND,
            coordinator::Args::new(),
        )
        .await
    }

    /// Open a Place AND fit it with a doorbell — the one call the UI needs.
    ///
    /// Rotating is the same call again: a fresh doorbell supersedes the old, and every
    /// poster carrying the previous one stops resolving. That is the only revocation there
    /// is, and the only way to stop an ex-member reading arrivals (see `PlaceState`).
    pub async fn place_fit_doorbell(&self, place_id_hex: &str) -> Result<[u8; 32], CoreError> {
        use rand_core::RngCore;
        let mut key = [0u8; 32];
        rand_core::OsRng.fill_bytes(&mut key);
        self.apply(
            place_id_hex,
            crate::place::OP_SET_DOORBELL,
            crate::place::set_doorbell_args(&key),
        )
        .await?;
        Ok(key)
    }

    /// KNOCK — what a stranger's phone does when they scan the sign.
    ///
    /// It deposits OUR OWN signed contact bundle in the place's doorbell mailbox. That is
    /// the whole protocol: no pairing, no 1:1 connection with whoever printed the poster,
    /// no waiting for prekeys to be stocked. A bundle is self-contained (`key_package`,
    /// `identity_pk`, signature), so any member who picks it up can add us with
    /// `group_add_member` on their very next pass.
    ///
    /// Sealed to the doorbell, which is on the poster — so this is confidential against
    /// the relay operator, and deliberately not against anyone holding the sign.
    pub async fn place_knock(
        &self,
        place_id_hex: &str,
        doorbell: &[u8; 32],
    ) -> Result<(), CoreError> {
        // A STACK of bundles, not one: each MLS add spends a key package, and the
        // member who answers admits us to the place AND to every group anchored there
        // — one admission per bundle. Four covers the place plus three anchored
        // groups; beyond that the answerer logs the shortfall and the next knock
        // (idempotent) tops the rest up. Each bundle mints and stores its own key
        // package; they share our identity and intro tag.
        const KNOCK_BUNDLES: usize = 4;
        let bundles: Vec<String> = (0..KNOCK_BUNDLES)
            .map(|_| self.build_contact_bundle())
            .collect::<Result<_, _>>()?;
        // The place id rides alongside so a member draining several doorbells knows which
        // door this knock was at without having to guess from the tag.
        let payload = format!("{place_id_hex}\n{}", bundles.join("\n"));
        let blob = seal::seal(payload.as_bytes(), doorbell, doorbell)?;
        let mut sess = Router::open(&self.routes).await?;
        let out = sess
            .publish(
                &Address::from_seed(doorbell),
                &pacific_wire::blob_b64(&blob),
            )
            .await;
        sess.close().await;
        out.map(|_| ())
    }

    /// ANSWER THE DOOR — drain one Place's doorbell and admit whoever knocked.
    ///
    /// Runs on EVERY member, not just the owner: the doorbell lives in the Place's log, so
    /// everyone who is in folds the same key. That is what takes the founder's phone off
    /// the critical path — whoever is about answers.
    ///
    /// A knocker is admitted to every Group of ours ANCHORED at this place (the agreement
    /// those groups made), and to the Place itself. Per-knock and per-group fault isolation:
    /// a bundle that will not verify, or a group we cannot commit to, must not stop the rest.
    ///
    /// Returns the object ids somebody was admitted to.
    pub async fn place_answer_door(&self, place_id_hex: &str) -> Result<Vec<String>, CoreError> {
        let state = self.place_state(place_id_hex)?;
        let Some(doorbell) = state.doorbell else {
            return Ok(Vec::new());
        };
        if !state.access.admits_strangers() {
            return Ok(Vec::new()); // closed: the sign is inert, knocks are ignored
        }

        let dir = &self.dir;
        let mut sess = Router::open(&self.routes).await?;
        let msgs = sess
            .drain(&[Address::from_seed(&doorbell).tag_hex()], |source| {
                dir.cursor(&doorbell, source)
            })
            .await?;
        sess.close().await;

        let mut admitted = Vec::new();
        for m in msgs {
            // Advance the cursor whatever happens: a knock we cannot use is not a reason to
            // re-read it forever (the same quarantine discipline as the intro mailbox).
            let handled = self.admit_one_knock(place_id_hex, &doorbell, &m.blob).await;
            match handled {
                Ok(mut ids) => admitted.append(&mut ids),
                Err(e) => tracing::debug!(place = %place_id_hex, error = %e, "unusable knock"),
            }
            self.dir.advance_cursor(&doorbell, m.source, m.seq)?;
        }
        Ok(admitted)
    }

    /// Answer every open door we are behind. One pass per Place we are a member of.
    ///
    /// Per-place fault isolation: one place whose relay is unreachable, or whose log will
    /// not fold, must not stop us answering the others.
    pub async fn answer_all_doors(&self) -> Result<Vec<String>, CoreError> {
        let mut admitted = Vec::new();
        for (gid, kind) in self.dir.list_objects()? {
            if kind != "place" {
                continue;
            }
            let place_id = hex::encode(&gid);
            match self.place_answer_door(&place_id).await {
                Ok(mut ids) => admitted.append(&mut ids),
                Err(e) => tracing::debug!(place = %place_id, error = %e, "door not answered"),
            }
        }
        Ok(admitted)
    }

    async fn admit_one_knock(
        &self,
        place_id_hex: &str,
        doorbell: &[u8; 32],
        blob_b64: &str,
    ) -> Result<Vec<String>, CoreError> {
        let blob = pacific_wire::blob_unb64(blob_b64)
            .map_err(|e| CoreError::Seal(format!("base64: {e}")))?;
        let opened = seal::open(&blob, doorbell, doorbell)?;
        let text =
            String::from_utf8(opened).map_err(|_| CoreError::Seal("knock is not utf-8".into()))?;
        let (knocked_at, rest) = text
            .split_once('\n')
            .ok_or_else(|| CoreError::Seal("malformed knock".into()))?;
        if knocked_at != place_id_hex {
            // Someone resealed a knock from another door under this tag. Refuse rather
            // than admit them somewhere they never asked to be.
            return Err(CoreError::Seal("knock names a different place".into()));
        }
        // The knock carries a STACK of bundles (one key package each — an MLS add
        // spends one, and we admit to several groups). All must verify, and all must
        // be the same person: a stack that disagrees about who is knocking is not a
        // knock, it is two people hiding behind one seal.
        let mut bundles = Vec::new();
        for line in rest.lines().filter(|l| !l.trim().is_empty()) {
            bundles.push((line, handshake::parse_and_verify(line)?));
        }
        let peer = match bundles.first() {
            Some((_, b)) => b.identity_pk,
            None => return Err(CoreError::Seal("knock carries no bundle".into())),
        };
        if bundles.iter().any(|(_, b)| b.identity_pk != peer) {
            return Err(CoreError::Seal("knock bundles disagree on identity".into()));
        }

        let mut admitted = Vec::new();
        // The PLACE FIRST — it is what the card names, and if bundles run short the
        // card's own promise must never be the casualty — then every group of ours
        // that AGREED to answer for this place.
        let mut targets = vec![place_id_hex.to_string()];
        targets.extend(self.our_groups_anchored_at(place_id_hex).unwrap_or_default());

        let mut fresh = bundles.iter();
        for target in targets {
            let Ok(gid) = self.object_group_id(&target) else {
                continue;
            };
            if self
                .dir
                .group_members(&gid)
                .map(|m| m.contains(&peer))
                .unwrap_or(false)
            {
                continue; // already in — a sign gets scanned twice; spends no bundle
            }
            let Some((bundle_str, _)) = fresh.next() else {
                // More doors than bundles. Loud, not silent: the next (idempotent)
                // knock carries a fresh stack and tops up whatever is left.
                tracing::warn!(
                    place = %place_id_hex,
                    target = %target,
                    "knock ran out of key packages before every anchored group was joined"
                );
                break;
            };
            match self.group_add_member(&target, bundle_str).await {
                Ok(_) => admitted.push(target),
                Err(e) => tracing::debug!(target = %target, error = %e, "cannot admit here"),
            }
        }
        // Each admission above went through `add_member_core`, which restates our
        // durable state at the newcomer's epoch.
        Ok(admitted)
    }

    /// STATE RE-EMISSION at the door — the no-backfill doctrine's other half, in
    /// `event_reemit`'s exact shape, narrowed to what only its author may say.
    ///
    /// A member admitted at epoch N structurally cannot decrypt anything sealed
    /// earlier (MLS forward secrecy), so its fold of this object is honestly EMPTY
    /// until the durable state is restated at an epoch it holds. Restating is the
    /// AUTHOR's act alone: a delta's author is the MLS sender that delivered it, so
    /// nobody can re-emit anyone else's words — which is the security model, not a
    /// limitation. Here WE re-flush, at the current epoch, every delta of OUR OWN
    /// authoring on this object's owner-sequenced spine (`seq` is `Some`: profile,
    /// access, doorbell, location, visibility — durable state). The commutative
    /// arms (posts, chat) are deliberately NOT re-emitted: history stays dark to
    /// late joiners, exactly as the Signal model pins.
    ///
    /// Idempotent everywhere: a re-flushed delta keeps its id, holders' logs drop
    /// the duplicate, joiners insert. Called after each admission we perform — an
    /// owner answering its own door is what turns a fresh member into a capable
    /// HOST (docs/doorbell-hosting-icd.html H8).
    async fn reemit_own_spine(&self, object_id_hex: &str) -> Result<usize, CoreError> {
        Ok(self.reemit_own_spine_counted(object_id_hex).await?.0)
    }

    /// STANDING STATE FOR LATE JOINERS (NC-65; Ralph, 27 Sep: "(a) Owner re-emits").
    ///
    /// A member someone else admitted (the Arc's node at a kiosk, A-3) holds nothing of an
    /// object from before its epoch, and only the owner can say that state again: every
    /// owner-sequenced op is the owner's to author (ICD-0: `ego: owner`, and `claimSpent`,
    /// the admitter's one write, is commutative). So for every object this device owns
    /// whose roster holds someone it has not restated to, it re-flushes its own spine
    /// ([`Node::reemit_own_spine`]): verbatim, so a joiner rebuilds the chain from genesis
    /// and a holder drops the duplicate. The first call after an open restates everything
    /// it owns, since nothing records what was restated before; an object is marked done
    /// only when every one of its deltas went out, so a relay that fails mid-way is tried
    /// again on the next call. Posts stay dark to late joiners, as `reemit_own_spine`
    /// says. Returns how many deltas went out.
    pub async fn restate_for_joiners(&self) -> Result<usize, CoreError> {
        let me = self.id.identity_pk();
        let mut out = 0usize;
        for (id, _kind, _name) in self.objects_named()? {
            let Ok(group_id) = self.object_group_id(&id) else { continue };
            if self.group_owner_id(&group_id).ok() != Some(me) {
                continue;
            }
            let roster: std::collections::BTreeSet<[u8; 32]> = self.dir.group_members(&group_id)?.into_iter().collect();
            let owed = self
                .restated
                .lock()
                .map(|r| r.get(&id).is_none_or(|was| !roster.is_subset(was)))
                .unwrap_or(true);
            if !owed {
                continue;
            }
            let (sent, total) = self.reemit_own_spine_counted(&id).await?;
            out += sent;
            if sent == total {
                if let Ok(mut r) = self.restated.lock() {
                    r.insert(id, roster);
                }
            }
        }
        Ok(out)
    }

    /// [`Node::reemit_own_spine`], saying how many of how many went out.
    async fn reemit_own_spine_counted(&self, object_id_hex: &str) -> Result<(usize, usize), CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        let me = self.id.identity_pk();
        let mut mine = Vec::new();
        for (author, envelope) in self.dir.load_log(&group_id)? {
            if author != me {
                continue;
            }
            let Ok(d) = coordinator::decode_delta(&envelope) else {
                continue;
            };
            if d.seq.is_some() {
                mine.push((d.id(), envelope));
            }
        }
        if mine.is_empty() {
            return Ok((0, 0));
        }
        let total = mine.len();
        let (sid, sk, _n) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut group = mls::load_group(&client, &group_id)?;
        let routes = self.routes_for(&group_id);
        let mut sess = Router::open(&routes).await?;
        let mut n = 0usize;
        for (did, env) in mine {
            if self
                .flush_one(&mut sess, &mut group, &group_id, &did, &env)
                .await
                .is_ok()
            {
                n += 1;
            }
        }
        sess.close().await;
        Ok((n, total))
    }

    /// Anchor a Group to a Place — put your group on the bench.
    ///
    /// THIS IS AN AGREEMENT, not a listing. It records
    /// [`crate::group::AffiliationRel::Anchored`] on the GROUP's own log, and that edge
    /// means: this group's members undertake to admit whoever scans that place and picks
    /// them. Authored on the group rather than on the place precisely because the duty is
    /// the group's — MLS lets nobody else discharge it — and because every member then
    /// folds the commitment out of their own copy of the log, rather than learning about
    /// it from whoever printed the QR.
    ///
    /// Refuses to anchor at something that is not a readable Place, or at one that is not
    /// open: agreeing to answer for a private place would be agreeing to nothing.
    pub async fn group_anchor_at_place(
        &self,
        group_id_hex: &str,
        place_id_hex: &str,
        at_ms: i64,
    ) -> Result<String, CoreError> {
        let place = self.place_state(place_id_hex).map_err(|_| {
            CoreError::Coordinator(format!("{place_id_hex} is not a place we can read"))
        })?;
        if !place.access.admits_strangers() {
            return Err(CoreError::Coordinator(format!(
                "place {place_id_hex} is private — nothing to answer for"
            )));
        }
        let mut args = coordinator::Args::new();
        args.insert(
            "peer".into(),
            coordinator::ArgVal::Text(place_id_hex.to_string()),
        );
        args.insert("rel".into(), coordinator::ArgVal::Text("anchored".into()));
        args.insert("name".into(), coordinator::ArgVal::Text(place.name.clone()));
        args.insert("at".into(), coordinator::ArgVal::Int(at_ms));
        self.apply(group_id_hex, crate::group::OP_SET_AFFILIATION, args)
            .await
    }

    /// Record that this group CREATED `object_id_hex` — an Event it hosts, or a Thing it
    /// lists (a job, a skill, inventory, a trade). Same authoring shape as
    /// [`Self::group_anchor_at_place`]: the owner-sequenced `setAffiliation`, rel
    /// `created`, on the GROUP's own log — so every member folds the same catalogue and
    /// the graph projector minted the `created` edge from the fold, never from a raw arg.
    ///
    /// Refuses an object this device does not hold: a group cannot claim authorship of
    /// something it has never seen.
    pub async fn group_attach_created(
        &self,
        group_id_hex: &str,
        object_id_hex: &str,
        name: &str,
        at_ms: i64,
    ) -> Result<String, CoreError> {
        let _ = self.object_group_id(object_id_hex)?;
        let mut args = coordinator::Args::new();
        args.insert(
            "peer".into(),
            coordinator::ArgVal::Text(object_id_hex.to_string()),
        );
        args.insert("rel".into(), coordinator::ArgVal::Text("created".into()));
        args.insert("name".into(), coordinator::ArgVal::Text(name.to_string()));
        args.insert("at".into(), coordinator::ArgVal::Int(at_ms));
        self.apply(group_id_hex, crate::group::OP_SET_AFFILIATION, args)
            .await
    }

    /// Stop answering for a Place. Members already admitted stay; this only ends the
    /// undertaking to admit anyone NEW arriving via that anchor.
    pub async fn group_unanchor(
        &self,
        group_id_hex: &str,
        place_id_hex: &str,
    ) -> Result<String, CoreError> {
        let mut args = coordinator::Args::new();
        args.insert(
            "peer".into(),
            coordinator::ArgVal::Text(place_id_hex.to_string()),
        );
        self.apply(group_id_hex, crate::group::OP_CLEAR_AFFILIATION, args)
            .await
    }

    /// Every Place this group has agreed to answer for: `(place_object_id, place_name)`.
    ///
    /// Folded from the group's own log, so it reads the same on every member's device —
    /// which is what makes the duty shared rather than one person's recollection.
    pub fn group_anchored_places(
        &self,
        group_id_hex: &str,
    ) -> Result<Vec<(String, String)>, CoreError> {
        Ok(self
            .group_state(group_id_hex)?
            .affiliations
            .into_iter()
            .filter(|(_, a)| a.rel.answers_for_peer())
            .map(|(peer, a)| (peer, a.name))
            .collect())
    }

    /// Every group of OURS that has agreed to answer for `place_id_hex`.
    ///
    /// What the reconcile iterates: a request arriving at a place is dischargeable by this
    /// device only for the groups we are in that anchored there.
    pub fn our_groups_anchored_at(&self, place_id_hex: &str) -> Result<Vec<String>, CoreError> {
        let mut out = Vec::new();
        for (gid, kind) in self.dir.list_objects()? {
            if kind != "group" {
                continue;
            }
            let gid_hex = hex::encode(&gid);
            // A group whose log will not fold must not take the whole sweep down.
            let Ok(anchored) = self.group_anchored_places(&gid_hex) else {
                continue;
            };
            if anchored.iter().any(|(p, _)| p == place_id_hex) {
                out.push(gid_hex);
            }
        }
        Ok(out)
    }

    // -- The group's Forums (its chatrooms) -----------------------------------

    /// Every Group object we hold, folded to its read view:
    /// `(object id, view, roster size, is_space)` — the LIFE listings in one call.
    ///
    /// TWO SPECIES SHARE THIS MACHINERY, and telling them apart is this function's
    /// job because it is the only place that can:
    ///
    ///   • a SPACE — a body you belong to, constituted by someone (`mintOrg`
    ///     authors `setProfile` with a name and a shape at the mint);
    ///   • an identity RECORD — a person's card: THIS account's own self record,
    ///     or one minted for an OFF-PLATFORM contact (`import_one_card` from a
    ///     vCard, a resolver proposal accepted as a Person).
    ///
    /// Nothing about the object itself separates them — same kind, same log, same
    /// fold — so callers were left to re-derive it, and a caller that forgot listed
    /// every contact as a Space.
    ///
    /// `is_space` is the answer, decided once: a record folds to the INDIVIDUAL
    /// shape, and a Space is what is left. A member of this network is NOT among
    /// these — an individual's Group is only for them (ruled 24 September 2026),
    /// so pairing no longer mints one about the peer.
    ///
    /// A group whose log will not fold is SKIPPED rather than failing the sweep —
    /// one poisoned object must not blank the whole screen (the same rule as
    /// [`our_groups_anchored_at`]).
    pub fn group_rows(&self) -> Result<Vec<(String, group::GroupView, usize, bool)>, CoreError> {
        let mut out = Vec::new();
        for (gid, kind) in self.dir.list_objects()? {
            if kind != "group" {
                continue;
            }
            let gid_hex = hex::encode(&gid);
            let Ok(view) = self.group_view(&gid_hex) else {
                continue;
            };
            let members = self.dir.group_members(&gid).map(|m| m.len()).unwrap_or(0);
            let is_record = view.shape == group::GroupShape::Individual;
            out.push((gid_hex, view, members, !is_record));
        }
        Ok(out)
    }

    /// Attach an EXISTING Forum object to a Group as a room — the Group's half of
    /// `part_of` (`base.setPart`, role `room`), which every member folds. The room's
    /// name is its own MLS GroupContext's. The room's half is written only when we
    /// mint it ([`Self::group_forum_new`]): an existing forum may be somebody else's.
    ///
    /// Refuses a target that is not a `forum`-kind object we hold, and a host that
    /// is not a `group` identity record: a tether has no chatrooms, and attaching
    /// a project or a place as a "room" would fold nonsense into every member.
    pub async fn group_forum_attach(
        &self,
        group_id_hex: &str,
        forum_id_hex: &str,
        at_ms: i64,
    ) -> Result<String, CoreError> {
        self.require_kind(group_id_hex, "group")?;
        self.require_kind(forum_id_hex, "forum")?;
        self.apply(group_id_hex, crate::parts::OP_SET_PART, crate::parts::set_part_args(forum_id_hex, ROOM, at_ms))
            .await
    }

    /// Detach a room from the Group. The Forum object, its log and its roster are
    /// untouched — this clears the group's edge to it, nothing else.
    pub async fn group_forum_detach(
        &self,
        group_id_hex: &str,
        forum_id_hex: &str,
    ) -> Result<String, CoreError> {
        self.apply(group_id_hex, crate::parts::OP_CLEAR_PART, crate::parts::clear_part_args(forum_id_hex))
            .await
    }

    /// Mint a Forum FOR a Group and attach it, in one call: a real `forum`-kind
    /// GroupObject (its own MLS group of 1) plus both halves of `part_of`.
    ///
    /// The room is for the group's MEMBERS, so every current roster member we can
    /// reach is added immediately — best-effort, by consuming a stocked prekey
    /// per member (`add_contact_to_object`). A member with no usable prekey is
    /// skipped without failing the mint: the edge already tells every member the
    /// room exists, and they can be added when their prekeys replenish.
    pub async fn group_forum_new(
        &self,
        group_id_hex: &str,
        name: &str,
        at_ms: i64,
    ) -> Result<String, CoreError> {
        self.require_kind(group_id_hex, "group")?;
        let roster = self
            .dir
            .group_members(&self.object_group_id(group_id_hex)?)?;
        let forum_id = self.object_new("forum", name)?;
        self.compose(group_id_hex, &forum_id, ROOM, at_ms).await?;
        let me = self.id.identity_pk();
        for member in roster {
            if member == me {
                continue;
            }
            if let Err(e) = self.add_contact_to_object(&forum_id, &member).await {
                tracing::debug!(member = %hex::encode(member), error = %e,
                    "forum mint: member not added yet (no usable prekey?)");
            }
        }
        Ok(forum_id)
    }

    /// The Group's rooms, folded from ITS log: `(forum id, name, attached at,
    /// held)` in attach order — its parts in role `room`. `held` says whether THIS
    /// device carries the room's MLS group — the difference between "tap to open"
    /// and "a room you know of but are not in yet". The name is the room's own
    /// GroupContext's, so only a held room has one; an unheld room shows its role
    /// until Ralph rules where a room's name lives for those not in it (W-25).
    pub fn group_forums(
        &self,
        group_id_hex: &str,
    ) -> Result<Vec<(String, String, i64, bool)>, CoreError> {
        Ok(self.rooms_of(self.group_state(group_id_hex)?.parts))
    }

    /// The REVERSE of [`group_forums`]: which Group hosts each room this device
    /// holds — `(forum id, group id, group display name)`, one row per edge.
    ///
    /// The CHAT list wants this for every row at once (a hosted room renders as
    /// "Group / Room"), so it is one sweep here rather than a per-row fold in the
    /// UI. Same fault isolation as [`group_rows`]: a group whose log will not
    /// fold contributes nothing rather than failing the sweep.
    pub fn forum_hosts(&self) -> Result<Vec<(String, String, String)>, CoreError> {
        let mut out = Vec::new();
        for (gid, kind) in self.dir.list_objects()? {
            if kind != "group" {
                continue;
            }
            let gid_hex = hex::encode(&gid);
            let Ok(state) = self.group_state(&gid_hex) else {
                continue;
            };
            for forum_id in state.parts.iter().filter(|(_, p)| p.role == ROOM).map(|(id, _)| id) {
                out.push((
                    forum_id.clone(),
                    gid_hex.clone(),
                    state.display_name.clone(),
                ));
            }
        }
        Ok(out)
    }

    // ======================================================================
    // FORUM ROOMS — the constituent Rooms a Forum (a Channel in the UI) hosts:
    // the Group→Forum edge one level down. The edge lives in the PARENT forum's
    // log (`forum.setRoom`/`clearRoom`, owner-sequenced); each Room is a full
    // `forum`-kind GroupObject with its own MLS roster; the UI surfaces the
    // folded set as tabs on the channel.

    /// The next `(seq, prev)` on a FORUM's spine — folds its OWN lens (per-type
    /// `sequenced_head`; the `thing_next_seq` trap applies verbatim). The forum
    /// spine was empty until the room ops arrived: every chat op is commutative.
    fn forum_next_seq(&self, group_id: &[u8], epoch: u64) -> Result<(u64, [u8; 32]), CoreError> {
        Ok(coordinator::next_sequenced_pos(
            self.folded_forum(group_id)?.sequenced_head(),
            epoch,
        ))
    }


    /// APPLY ONE DELTA TO ONE OBJECT — the one write path for every kind.
    ///
    /// `mint` brings a GroupObject into existence; `apply` is everything that
    /// happens to one afterwards. Between them they are the whole of writing.
    ///
    /// THERE USED TO BE NINE OF THESE, one per op table, each a copy of the same
    /// sequence wrapped around a different `T` — and the copies drifted. Eight of
    /// them stamped a commutative delta's `gen` with `author_delta_count`, a
    /// per-author counter that `post_to_group`'s own comment names as the cause of
    /// the "my reply jumped to the top" bug; the fix had landed in one path and
    /// nowhere else. This calls `authoring::build`, which is the rule, once: the
    /// op lookup, the MLS-doors and note gates, the owner check, the Lamport gen
    /// over `object::NextGen`, the sequenced position and the dry-run probe.
    ///
    /// What is left here is the I/O the pure door cannot do: converge to the head
    /// epoch, gather the context from the directory, and append.
    pub async fn apply(
        &self,
        object_id_hex: &str,
        op_id: u32,
        args: coordinator::Args,
    ) -> Result<String, CoreError> {
        self.apply_with(object_id_hex, op_id, args, true).await
    }

    /// THE LOCAL COMMIT of one op (O-69, the Door's write path): authored as `apply`
    /// authors it and stored, and nothing sent; `publish_pending` sends it after the
    /// answer, or the next sync does, as a write made offline. A sequenced op is authored
    /// at head, after a drain on the held session; a commutative op at the epoch this
    /// device holds, which is its author's stamp whatever epoch seals it
    /// (`Coordinator::state`), so it waits on no round trip.
    pub async fn apply_local(
        &self,
        object_id_hex: &str,
        op_id: u32,
        args: coordinator::Args,
    ) -> Result<String, CoreError> {
        self.apply_with(object_id_hex, op_id, args, false).await
    }

    async fn apply_with(
        &self,
        object_id_hex: &str,
        op_id: u32,
        args: coordinator::Args,
        send: bool,
    ) -> Result<String, CoreError> {
        // A card written here, by any caller, may not be the self record's: the memoised card
        // walk looks again at its next pass (O-77). Its own writes clear it too, and it sets it
        // again when it has found the card everywhere.
        if op_id == crate::profiles::OP_PUBLISH_PROFILE {
            *self.card_everywhere.lock().unwrap_or_else(|p| p.into_inner()) = None;
        }
        let group_id = self.object_group_id(object_id_hex)?;
        let kind = self
            .dir
            .group_kind(&group_id)?
            .ok_or_else(|| CoreError::Directory(format!("no object {object_id_hex}")))?;
        // Converge before sealing: a member who has not processed the latest
        // membership commit would otherwise author into a superseded epoch that
        // later joiners cannot decrypt. Best-effort, as everywhere else. What the held
        // connection delivered is ingested first, with no round trip; a drain is paid only
        // where that leaves the group short of head.
        self.converge_before_writing(&group_id, send || !crate::authoring::is_commutative(&kind, op_id)).await;

        let (sid, sk, _name) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut group = self.load_member_group(&client, &group_id)?;
        let epoch = group.current_epoch();

        let members = self.dir.group_members(&group_id)?;
        let owners = self.dir.owner_history(&group_id)?;
        // The whole log, undecoded rows skipped: `authoring::fold` delivers each
        // and the Coordinator drops what is not its type, so one log carrying
        // several op-groups needs no filtering here.
        let log: Vec<(coordinator::Delta, crate::object::MemberId)> = self
            .dir
            .load_log(&group_id)?
            .into_iter()
            .filter_map(|(author, envelope)| {
                coordinator::decode_delta(&envelope).ok().map(|d| (d, author))
            })
            .collect();
        let ctx = crate::authoring::Ctx {
            me: self.id.identity_pk(),
            epoch,
            members: &members,
            owners: &owners,
            log: &log,
            // The floor this device was handed, which `next_lamport` reads too —
            // one rule, `object::NextGen`, and the same answer either way.
            watermark: Some(self.dir.gen_floor(&group_id)?),
        };
        let delta = crate::authoring::build(&kind, op_id, args, &ctx)
            .map_err(|r| CoreError::Coordinator(r.to_string()))?;
        let delta_id = if send {
            self.append_and_flush(&mut group, &group_id, delta).await?
        } else {
            self.append_local(&group_id, delta)?.0
        };
        Ok(hex::encode(delta_id))
    }


    /// Attach an EXISTING Forum object to a Forum as a room — the Channel's half of
    /// `part_of` (`base.setPart`, role `room`), which every member folds. Refuses a
    /// parent or target that is not a `forum` we hold, and a self-attach — a channel
    /// cannot be its own tab.
    pub async fn forum_room_attach(
        &self,
        forum_id_hex: &str,
        room_id_hex: &str,
        at_ms: i64,
    ) -> Result<String, CoreError> {
        self.require_kind(forum_id_hex, "forum")?;
        self.require_kind(room_id_hex, "forum")?;
        if forum_id_hex.eq_ignore_ascii_case(room_id_hex) {
            return Err(CoreError::Coordinator(
                "a channel cannot host itself as a room".into(),
            ));
        }
        self.apply(forum_id_hex, crate::parts::OP_SET_PART, crate::parts::set_part_args(room_id_hex, ROOM, at_ms))
            .await
    }

    /// Detach a Room from its Channel. The Room object, its log and its roster
    /// are untouched — this clears the parent's edge to it, nothing else.
    pub async fn forum_room_detach(
        &self,
        forum_id_hex: &str,
        room_id_hex: &str,
    ) -> Result<String, CoreError> {
        self.apply(forum_id_hex, crate::parts::OP_CLEAR_PART, crate::parts::clear_part_args(room_id_hex))
            .await
    }

    /// Mint a Room FOR a Channel and attach it, in one call: a real `forum`-kind
    /// GroupObject (its own MLS group of 1) plus both halves of `part_of` — the
    /// `group_forum_new` shape one level down. The room is for the channel's
    /// MEMBERS, so every current roster member we can reach is added immediately
    /// (best-effort, one stocked prekey each); the edge already tells the rest
    /// the room exists, and they can be added when their prekeys replenish.
    pub async fn forum_room_new(
        &self,
        forum_id_hex: &str,
        name: &str,
        at_ms: i64,
    ) -> Result<String, CoreError> {
        self.require_kind(forum_id_hex, "forum")?;
        let roster = self
            .dir
            .group_members(&self.object_group_id(forum_id_hex)?)?;
        let room_id = self.object_new("forum", name)?;
        self.compose(forum_id_hex, &room_id, ROOM, at_ms).await?;
        let me = self.id.identity_pk();
        for member in roster {
            if member == me {
                continue;
            }
            if let Err(e) = self.add_contact_to_object(&room_id, &member).await {
                tracing::debug!(member = %hex::encode(member), error = %e,
                    "room mint: member not added yet (no usable prekey?)");
            }
        }
        Ok(room_id)
    }

    /// The Channel's rooms, folded from ITS log: `(room id, name, attached at,
    /// held)` in attach order — `group_forums`' contract verbatim.
    pub fn forum_rooms(
        &self,
        forum_id_hex: &str,
    ) -> Result<Vec<(String, String, i64, bool)>, CoreError> {
        let group_id = self.object_group_id(forum_id_hex)?;
        Ok(self.rooms_of(self.forum_state(&group_id)?.parts))
    }

    /// `(id, name, at, held)` for the parts in role `room`, in attach order. The
    /// name is the room's own GroupContext's when we hold it; otherwise the role.
    fn rooms_of(&self, parts: std::collections::BTreeMap<String, crate::object::PartRef>) -> Vec<(String, String, i64, bool)> {
        let mut out = Vec::new();
        for (id, p) in parts.into_iter().filter(|(_, p)| p.role == ROOM) {
            let held = hex::decode(&id)
                .ok()
                .and_then(|gid| self.dir.group_kind(&gid).ok().flatten())
                .is_some();
            let name = match held.then(|| self.object_name(&id)) {
                Some(Ok(live)) if !live.is_empty() => live,
                _ => p.role.clone(),
            };
            out.push((id, name, p.at, held));
        }
        out.sort_by_key(|(_, _, at, _)| *at);
        out
    }

    /// BOTH halves of `part_of`, in one act: `base.setPart` on the parent and
    /// `base.setParent` on the part. Only for a part this device just minted, while
    /// it is the part's sole member and owner — so one principal authors both and
    /// nothing has to agree with anything (`crate::parent`).
    async fn compose(&self, parent_id_hex: &str, part_id_hex: &str, role: &str, at_ms: i64) -> Result<(), CoreError> {
        self.apply(parent_id_hex, crate::parts::OP_SET_PART, crate::parts::set_part_args(part_id_hex, role, at_ms))
            .await?;
        self.apply(part_id_hex, crate::parent::OP_SET_PARENT, crate::parent::set_parent_args(parent_id_hex, role, at_ms))
            .await?;
        Ok(())
    }

    /// The REVERSE of [`forum_rooms`]: which Channel hosts each room this device
    /// holds — `(room id, forum id, forum display name)`, one row per edge. The
    /// CHAT list wants this for every row at once (a hosted room renders as
    /// "Channel / Room"), exactly like [`Node::forum_hosts`]; same fault
    /// isolation — a forum whose log will not fold contributes nothing.
    pub fn room_hosts(&self) -> Result<Vec<(String, String, String)>, CoreError> {
        let mut out = Vec::new();
        for (gid, kind) in self.dir.list_objects()? {
            if kind != "forum" {
                continue;
            }
            let Ok(coord) = self.folded_forum(&gid) else {
                continue;
            };
            let state = coord.state();
            if !state.parts.values().any(|p| p.role == ROOM) {
                continue;
            }
            let gid_hex = hex::encode(&gid);
            let host_name = self.object_name(&gid_hex).unwrap_or_default();
            for room_id in state.parts.iter().filter(|(_, p)| p.role == ROOM).map(|(id, _)| id) {
                out.push((room_id.clone(), gid_hex.clone(), host_name.clone()));
            }
        }
        Ok(out)
    }

    // ======================================================================
    // COMPLIANCE — an object either folds, or it does not
    // ======================================================================
    //
    // WHY THIS EXISTS. Every sweep in this file that lists objects has, at its
    // centre, a line of this shape:
    //
    //     let Ok(view) = self.group_view(&gid_hex) else { continue };
    //
    // It was written for fault isolation, and the isolation is right: one
    // poisoned object must not blank the whole screen. What it ALSO does is make
    // a non-folding object indistinguishable from an object that was never
    // created. The member is shown a shorter list. Nothing is said. Nobody is
    // told which object went, or why, or that anything went at all.
    //
    // That is the failure this system cannot afford, because the head/recovery/
    // ratchet path is the only thing that carries state between devices and
    // sessions. An object that quietly drops out of the list on ONE device looks,
    // to its owner, exactly like an object that was never durable — and they will
    // only find out which it was on a device that no longer has it. A shortcut
    // that "works for now" and then vanishes is worse than a refusal, because it
    // implies a maturity that will not survive the next session.
    //
    // So the skip STAYS and stops being SILENT. An object is either compliant or
    // non-compliant, and which one it is, is a fact this node can be asked for.

    /// Fold-probe one object through the lens its kind declares.
    ///
    /// `Ok(())` is the whole of "compliant": the log this device holds reduces
    /// from its first delta to its last without the reducer refusing one. There is
    /// no partial credit and no "compliant enough" — a log carrying one op this
    /// reducer does not know folds to a DIFFERENT state on a device that does know
    /// it, which is the one thing a replicated object may never do.
    pub fn object_compliance(&self, object_id_hex: &str) -> Result<(), CoreError> {
        let gid = self.object_group_id(object_id_hex)?;
        let kind = self
            .dir
            .group_kind(&gid)?
            .ok_or_else(|| CoreError::Directory(format!("no object {object_id_hex}")))?;
        self.fold_probe(&gid, &kind)
    }

    /// The kind → lens dispatch, and the only place that knows all nine.
    ///
    /// A kind with NO lens is reported as non-compliant rather than skipped, which
    /// is the same ruling in the other direction: this device holds an object it
    /// cannot reduce, and saying so is the honest answer. Silently returning `Ok`
    /// for it would let `noncompliant_objects` come back empty on a device that
    /// cannot read half of what it is storing.
    fn fold_probe(&self, group_id: &[u8], kind: &str) -> Result<(), CoreError> {
        match kind {
            // Every Group-typed kind, not just the bare one: a tether and a
            // notebook carry the Group op vocabulary and fold with it, and a kind
            // this arm forgot was reported NON-COMPLIANT rather than folded.
            k if crate::group::is_group_typed(k) => self.folded_group(group_id).map(|_| ()),
            // Both chat kinds fold here: `folded_forum` picks the coordinator the
            // object's own lens names (the fork, 24 Sep 2026).
            "forum" | "conversation" => self.folded_forum(group_id).map(|_| ()),
            "project" => self.folded_project(group_id).map(|_| ()),
            "thing" => self.folded_thing(group_id).map(|_| ()),
            "place" => self.folded_place(group_id).map(|_| ()),
            "event" => self.folded_event(group_id).map(|_| ()),
            "post" => self.folded_post(group_id).map(|_| ()),
            "system" => self.folded_system(group_id).map(|_| ()),
            // Its own kind since 25 Sep 2026; every Site has one.
            "host" => self.folded_host(group_id).map(|_| ()),
            // Through the one fold, as their views fold (fold::view_of); NC-66.
            "treasury" => crate::object_store::GroupObjectStore::new(&self.dir).folded::<crate::treasury::TreasuryType>(group_id).map(|_| ()),
            "note" => crate::object_store::GroupObjectStore::new(&self.dir).folded::<crate::note::NoteType>(group_id).map(|_| ()),
            "transaction" => crate::object_store::GroupObjectStore::new(&self.dir).folded::<crate::transaction::TransactionType>(group_id).map(|_| ()),
            "connection" => self.folded_contact(group_id).map(|_| ()),
            other => Err(CoreError::Coordinator(format!(
                "'{other}' has no lens in this build — an object of a kind this \
                 device cannot fold is NON-COMPLIANT here, not absent"
            ))),
        }
    }

    /// CAN THIS OBJECT BE GOT BACK FROM THE SEED ALONE? The three things a minted
    /// GroupObject needs, each checked, each named when it is missing.
    ///
    /// A device that has lost everything holds 24 words. From them it derives the
    /// storage root, and from that it must be able to (1) NAME this object, (2)
    /// READ what was said in it, and (3) SPEAK in it again. Three artefacts:
    ///
    ///   - a SPINE entry, or the object cannot be named — recovery does not know
    ///     the group id, and every address below is derived from it;
    ///   - an ARCHIVE KEY RECORD per epoch at [`crate::locator::archive_locator`],
    ///     or that epoch's content is unreadable for ever, because the exporter
    ///     answers only while the group is AT that epoch;
    ///   - MLS STATE somewhere the seed reaches, or the object comes back
    ///     read-only: `mls-rs` can rebuild a live group from
    ///     `GroupStateStorage`, but this build keeps it in `pacific.db` and a
    ///     device that lost the db lost that too.
    ///
    /// NONE OF THE THREE IS WRITTEN TODAY, and this reports that rather than
    /// implying otherwise. `object_new` writes to the local directory and nowhere
    /// a seed can reach, so every object on this device is unrecoverable and this
    /// says so per object and per epoch. The migration is done when
    /// [`Node::unrecoverable_objects`] comes back empty; see
    /// `docs/archive-by-arithmetic.md`.
    pub fn recoverability(&self, object_id_hex: &str) -> Result<Recoverability, CoreError> {
        let gid = self.object_group_id(object_id_hex)?;
        let seed = self.seed_or_refuse()?;
        let root = crate::locator::storage_root(&seed);

        // (1) Named — CHECKED, not assumed. A spine entry counts once it has been
        // PUBLISHED: queued-and-unsent lives in this device's own database, which
        // is the thing a recovering device does not have. So this reads the
        // delivered rows, opens each under the root, and looks for this group.
        //
        // It flips on its own when the drain lands. That is the point of checking
        // rather than declaring: nobody has to remember to come back here.
        let key = crate::spine::entry_key(&root);
        let mut named = false;
        for (index, blob) in self.dir.delivered_spine(SPINE_GEN)? {
            if let Ok(entry) = crate::spine::open_entry(&blob, &key, index) {
                if entry.body.group_id() == gid.as_slice() {
                    named = true;
                    break;
                }
            }
        }

        // The FLOOR and the DEPARTURE, from every entry this device holds for the
        // object, published or not: an epoch below the join was never the account's
        // whether or not the entry has gone out yet. Lowest join, latest departure —
        // and a join after the latest departure is a rejoin, so it is not departed.
        //
        // THIS DEVICE'S ENTRIES ONLY. A device added beside another of the same
        // person writes no entry (`spine_on_join`), so here it has no floor and
        // reports from 0 — more epochs as unreadable, never fewer. The account's own
        // answer is the whole spine, read by `spine_read`.
        let mut joins: Vec<u64> = Vec::new();
        let mut lefts: Vec<u64> = Vec::new();
        for (_, entry) in self.spine_entries()? {
            match entry.body {
                crate::spine::Body::Group(j) if j.group_id == gid => joins.push(j.first_epoch),
                crate::spine::Body::Left(d) if d.group_id == gid => lefts.push(d.last_epoch),
                _ => {}
            }
        }
        let first_epoch = joins.iter().copied().min();
        let left_epoch = match (joins.iter().copied().max(), lefts.iter().copied().max()) {
            (Some(j), Some(l)) if l >= j => Some(l),
            (None, Some(l)) => Some(l),
            _ => None,
        };

        // (2) Readable. One record per epoch, at an address computed from the
        // root. The epochs this device was present for are the ones it could have
        // recorded; it recorded none of them.
        // The epochs this device has passed through, as the directory recorded
        // them. Bounded by EPOCH_RETENTION, so this UNDERSTATES the loss for an
        // older group — the epochs pruned from the tag table are unreadable too,
        // and are not listed because this device no longer knows they existed.
        let epoch = self
            .dir
            .epoch_tags(&gid)?
            .iter()
            .map(|t| t.0)
            .max()
            .unwrap_or(0);
        let mut unreadable_epochs = Vec::new();
        // From the floor, not from 0: below it is "never yours", not "lost".
        let upper = left_epoch.map_or(epoch, |l| l.min(epoch));
        for e in first_epoch.unwrap_or(0)..=upper {
            let _addr = crate::locator::archive_locator(&root, &gid, e);
            // The address is derivable; the record is not there. When the writer
            // exists this becomes a fetch, and the missing ones stay named.
            unreadable_epochs.push(e);
        }

        // (3) Speakable. See MLS_STATE_IS_SEED_REACHABLE.
        let speakable = MLS_STATE_IS_SEED_REACHABLE;

        Ok(Recoverability {
            object_id: object_id_hex.to_string(),
            named,
            speakable,
            unreadable_epochs,
            current_epoch: epoch,
            first_epoch,
            left_epoch,
        })
    }

    /// [`Node::recoverability`], with `named` VERIFIED against the relay.
    ///
    /// WHAT THE LOCAL ANSWER CANNOT SEE. `recoverability` proves `named` by
    /// opening the DELIVERED rows out of `pacific.db`. That is a real proof of
    /// two things — the entry sealed correctly under the storage root, and it
    /// names this group — but it is not proof that the bytes are at the relay
    /// NOW. The evidence is a past `publish` ack recorded locally; the claim is
    /// present tense. Any relay that acks and then does not keep — a retention
    /// window, an eviction, data loss, an ack before a durable write — leaves
    /// `named` true for ever while the account comes back holding nothing.
    ///
    /// So this fetches the entry back from the address its index derives and
    /// opens what the relay returned. `named` then means: the bytes are there,
    /// now, and the seed found them.
    ///
    /// THIS IS ALSO HOW CORE ENFORCES A DEPLOY SETTING IT CANNOT READ. The spine
    /// depends on the relay keeping tags for ever (`Retention::default()`, no age
    /// window, ruled 13 September 2026). Core cannot read that config — but it can
    /// observe its consequence. A deploy that sets `window_us` stops being a
    /// silent, total failure and becomes a `named` that goes false at a named
    /// index. Not by reading the setting; by failing when it bites.
    ///
    /// ONE ROUND TRIP, and that is what the arithmetic addressing bought:
    /// `chain_locator(root, gen, index)` means one entry can be checked without
    /// walking the chain to it.
    ///
    /// Kept SEPARATE from `recoverability` rather than replacing it: the local
    /// answer is cheap and right for a hot path that is redrawing a list, and
    /// this one costs a dial. Use this at sign-in, in the tests, and anywhere the
    /// answer is about to be told to a person.
    pub async fn verify_recoverability(
        &self,
        object_id_hex: &str,
    ) -> Result<Recoverability, CoreError> {
        let mut r = self.recoverability(object_id_hex)?;
        // Nothing claimed locally — there is no claim to check, and dialling to
        // confirm an absence would be a round trip that cannot change the answer.
        if !r.named {
            return Ok(r);
        }

        let gid = self.object_group_id(object_id_hex)?;
        let seed = self.seed_or_refuse()?;
        let root = crate::locator::storage_root(&seed);
        let key = crate::spine::entry_key(&root);

        // Which index carries this group, according to what we think we sent.
        let mut at: Option<u64> = None;
        for (index, blob) in self.dir.delivered_spine(SPINE_GEN)? {
            if let Ok(e) = crate::spine::open_entry(&blob, &key, index) {
                if e.body.group_id() == gid.as_slice() {
                    at = Some(index);
                    break;
                }
            }
        }
        let Some(index) = at else {
            // `recoverability` said named and the rows do not agree. Report the
            // disagreement as not-named rather than papering it over.
            r.named = false;
            return Ok(r);
        };

        // The address is recomputed, never stored: arithmetic is the point.
        let addr = crate::locator::chain_locator(&root, SPINE_GEN, index);
        let tag = Address::from_seed(&addr).tag_hex();
        let mut sess = Router::open(&self.routes).await?;
        // `since = 0`: everything at that tag. A cursor would ask "what is new to
        // me", and the question here is "is it there at all".
        let got = sess.drain(&[tag], |_| Ok(0)).await;
        sess.close().await;

        // The fetched bytes must open under THIS account's key at THIS index and
        // name THIS group. Anything less is not the entry we are looking for.
        r.named = got?.iter().any(|m| {
            pacific_wire::blob_unb64(&m.blob)
                .ok()
                .and_then(|b| crate::spine::open_entry(&b, &key, index).ok())
                .is_some_and(|e| e.body.group_id() == gid.as_slice())
        });
        Ok(r)
    }

    /// Every object this node holds that could NOT be got back from the seed.
    ///
    /// An EMPTY result is the claim "everything I hold, I could get back" — the
    /// assertion the migration closes, and the number a device should be able to
    /// show its owner before it tells them their words are a backup. A non-empty
    /// one NAMES what would be lost, which is the whole of it: `backup.rs`
    /// (removed in a58798c) said
    /// "the words are the backup" is true of an account that has a history stored,
    /// "and a surface that says it to someone who has none is lying to them".
    ///
    /// Today this returns every object. That is the honest number.
    pub fn unrecoverable_objects(&self) -> Result<Vec<Recoverability>, CoreError> {
        let mut out = Vec::new();
        for (gid, _kind) in self.dir.list_objects()? {
            let id = hex::encode(&gid);
            match self.recoverability(&id) {
                Ok(r) if r.recoverable() => {}
                Ok(r) => out.push(r),
                // A seedless identity (minted before seeds existed) cannot be
                // recovered from words at all, and saying which is not this
                // sweep's job — it reports what it could not check.
                Err(e) => out.push(Recoverability::unknown(&id, e.to_string())),
            }
        }
        Ok(out)
    }

    /// Every object this node holds that will NOT fold, in the reducer's own words.
    /// The list the sweeps above cannot give you, because each of them answers a
    /// skip with nothing.
    ///
    /// An EMPTY result is the claim "everything I hold, I can reduce" — the
    /// assertion a conformance run should make, and the number a device should be
    /// able to show its owner. A non-empty one NAMES the objects that are on this
    /// device and are not on its screen — and the objects the spine names that the
    /// restore refused for their route, which are on neither.
    pub fn noncompliant_objects(&self) -> Result<Vec<Noncompliance>, CoreError> {
        let mut out = Vec::new();
        for (gid, kind) in self.dir.list_objects()? {
            if let Err(e) = self.fold_probe(&gid, &kind) {
                out.push(Noncompliance {
                    object_id: hex::encode(&gid),
                    kind,
                    reason: e.to_string(),
                });
            }
        }
        out.extend(self.refused_way_ins()?);
        Ok(out)
    }

    /// Every object the spine names and this device does not hold because its way-in's
    /// route failed the check (SECURITY, 27 Sep). The restore stored nothing of it, so
    /// the directory cannot name it; the walked spine can. A seedless identity has no
    /// spine.
    fn refused_way_ins(&self) -> Result<Vec<Noncompliance>, CoreError> {
        if self.id.seed_bytes().is_none() {
            return Ok(Vec::new());
        }
        let Some(plan) = spine_plan(&self.spine_entries()?) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for gid in &plan.targets {
            if self.dir.group_kind(gid)?.is_some() && !self.dir.is_departed(gid)? {
                continue;
            }
            let Some(way) = plan.ways.get(gid) else {
                continue;
            };
            if let Some(reason) = way_in_refusal(way) {
                out.push(Noncompliance { object_id: hex::encode(gid), kind: way.kind.clone(), reason });
            }
        }
        Ok(out)
    }

    /// Loud kind gate: the object must exist here AND be of `want` kind.
    fn require_kind(&self, object_id_hex: &str, want: &str) -> Result<(), CoreError> {
        let gid = self.object_group_id(object_id_hex)?;
        let kind = self
            .dir
            .group_kind(&gid)?
            .ok_or_else(|| CoreError::Directory(format!("no object {object_id_hex}")))?;
        if kind != want {
            return Err(CoreError::Coordinator(format!(
                "{object_id_hex} is a '{kind}' object, not a '{want}'"
            )));
        }
        Ok(())
    }

    /// Auto-admit everyone who paired off a public Place's QR. Run on every sync.
    ///
    /// A PUBLIC PLACE IS AUTO-ACCEPTED: opening the Place IS the consent, so there is no
    /// per-person approval — the host admits whoever asks, for as long as it stays open.
    /// Closing it stops future admissions (the posture is re-read every pass) without
    /// evicting anyone already in.
    ///
    /// Why this is a RECONCILE and not a one-shot at intro time: when the host first sees
    /// the pairing, it usually cannot admit yet — `add_contact_to_object` consumes one of
    /// the contact's stocked prekeys, and those are not stocked until `pair_accept` has
    /// run. So the intent is persisted (on the peer's `why`) and replayed every sync until
    /// it succeeds. That also makes it survive a restart and a period offline.
    ///
    /// Idempotent by roster check, and per-joiner fault-isolated: one joiner who cannot be
    /// admitted (no prekeys yet, place since closed, place deleted) must not stop the
    /// others. Returns the ids of the places somebody was actually admitted to.
    pub async fn reconcile_place_joins(&self) -> Result<Vec<String>, CoreError> {
        let pending = self.dir.peers_why_prefixed(Self::PLACE_JOIN_WHY)?;
        let mut admitted = Vec::new();
        for (peer_pk, place_id) in pending {
            // Ours? A `why` is the SCANNER's claim; it names a place id we may not own,
            // may never have heard of, or may have deleted. Fold it ourselves and let a
            // failure drop this joiner rather than the pass.
            let Ok(state) = self.place_state(&place_id) else {
                continue;
            };
            if !state.access.admits_strangers() {
                continue; // closed since they scanned — no admission, no error
            }
            // Already in? Then this intent is spent. Not an error: a poster gets scanned
            // twice, and re-adding a member would churn the group for nothing.
            let Ok(group_id) = self.object_group_id(&place_id) else {
                continue;
            };
            if self
                .dir
                .group_members(&group_id)
                .map(|m| m.contains(&peer_pk))
                .unwrap_or(false)
            {
                continue;
            }
            match self.place_admit(&place_id, &peer_pk).await {
                Ok(_) => admitted.push(place_id.clone()),
                Err(e) => {
                    // The common case here is "no usable prekey" — they have not accepted
                    // the pairing yet. Genuinely transient: leave the intent recorded and
                    // try again next sync.
                    tracing::debug!(place = %place_id, error = %e,
                                    "place join not admittable yet; will retry");
                }
            }

            // DISCHARGE THE ANCHORED GROUPS' DUTY. A place is a sparse anchor; the thing a
            // scanner actually wants to be in is a GROUP that put itself on that place. Any
            // member of such a group can admit — which is the whole reason the obligation
            // lives on the group rather than on whichever phone printed the QR.
            //
            // Only our own anchored groups, and only ones we can commit an Add to: MLS
            // gives no way to admit someone to a group we are not in.
            let Ok(groups) = self.our_groups_anchored_at(&place_id) else {
                continue;
            };
            for gid in groups {
                let Ok(gg) = self.object_group_id(&gid) else {
                    continue;
                };
                if self
                    .dir
                    .group_members(&gg)
                    .map(|m| m.contains(&peer_pk))
                    .unwrap_or(false)
                {
                    continue; // already in this group
                }
                match self.add_contact_to_object(&gid, &peer_pk).await {
                    Ok(_) => admitted.push(gid),
                    Err(e) => tracing::debug!(group = %gid, error = %e,
                                              "anchored group cannot admit yet; will retry"),
                }
            }
        }
        Ok(admitted)
    }

    /// Admit a joiner to a Place.
    ///
    /// Guarded on the Place being PUBLIC, so a stale intent cannot let someone into a
    /// Place the owner has since closed. Requires an existing contact with stocked
    /// prekeys — which the QR's pairing half is what establishes.
    pub async fn place_admit(
        &self,
        object_id_hex: &str,
        peer_id: &[u8; 32],
    ) -> Result<[u8; 32], CoreError> {
        let st = self.place_state(object_id_hex)?;
        if !st.access.admits_strangers() {
            return Err(CoreError::Coordinator(format!(
                "place {object_id_hex} is private — reopen it to admit anyone"
            )));
        }
        self.add_contact_to_object(object_id_hex, peer_id).await
    }

    /// Mint a Place: a group of 1, its profile, and the fix it was minted at — in that
    /// order, as three deltas on one spine.
    ///
    /// The coordinate is authored AT MINT, not left for a later edit, because a Place is
    /// evidence of having been somewhere. `location` is optional only so a caller whose
    /// fix never arrived still gets a durable object rather than losing the user's input;
    /// such a Place is honestly unplaced until a `setLocation` lands.
    /// It is `mint` plus a location, and it exists alongside it because a
    /// `LocationSource` is richer than a draft's lat/lng: a Place can be pinned to a
    /// live `Stream` as well as a `Fixed` point, and the draft has no way to say so.
    /// Object creation and the profile delta are NOT restated here — they are `mint`.
    pub async fn place_mint(
        &self,
        name: &str,
        descriptor: &str,
        location: Option<crate::geo::LocationSource>,
    ) -> Result<String, CoreError> {
        let object_id = self
            .mint(
                crate::object::ObjectKind::Place,
                &crate::mint::MintDraft {
                    name: name.to_string(),
                    descriptor: descriptor.to_string(),
                    // The location rides `place_set_source` below, which takes the full
                    // source — not the draft's Fixed-only lat/lng.
                    ..Default::default()
                },
            )
            .await?;
        if let Some(source) = location {
            self.place_set_source(&object_id, &source).await?;
        }
        Ok(object_id)
    }

    /// The ONE place a Place's location facet is authored. Both mint paths land here,
    /// so the base `setLocation` op and its arg builder are named once.
    async fn place_set_source(
        &self,
        object_id: &str,
        source: &crate::geo::LocationSource,
    ) -> Result<String, CoreError> {
        self.apply(
            object_id,
            crate::geo::OP_SET_LOCATION,
            crate::geo::set_location_args(source),
        )
        .await
    }

    /// THE mint — bring a new GroupObject of any mintable kind into existence.
    ///
    /// This is `place_mint` above, generalised: `object_new` then the kind's own op-0
    /// profile delta, built from one shared draft by `mint::profile_args`. Every `+` in
    /// the app comes through here, so there is ONE place where "a new thing exists" is
    /// implemented and one place where it can go wrong.
    ///
    /// Three things it deliberately does NOT do:
    ///
    /// * **Invent a name.** An empty draft name is passed through as an empty name. The
    ///   reducers accept it and the UI gates on it; refusing here would put the rule in
    ///   two places and let them drift.
    /// * **Author anything past op 0.** Posture, tickets, media, venue and location are
    ///   SEPARATE ops with their own authority and their own fold gates. A mint that
    ///   quietly authored six deltas would make failure partial and unreportable. The
    ///   one exception is a Place's location, below, because an unplaced Place is a
    ///   different object rather than the same object missing a field.
    /// * **Roll back.** If op 0 fails the object still exists, unnamed — the MLS group
    ///   was created and cannot be un-created. We return the error AND the id would be
    ///   lost, so we surface the error and leave the empty object for the directory to
    ///   show; a silent orphan is worse than a visible one.
    pub async fn mint(
        &self,
        kind: crate::object::ObjectKind,
        draft: &crate::mint::MintDraft,
    ) -> Result<String, CoreError> {
        if let Err(why) = crate::mint::classify(kind) {
            return Err(CoreError::Coordinator(format!(
                "{} objects are not minted from a draft ({why:?})",
                kind.name()
            )));
        }
        let object_id = self.object_new(kind.name(), &draft.name)?;

        // Forum returns None here — it is minted NAME-ONLY, and its name already rode
        // the GroupContext above. Authoring its op 0 would post a message.
        if let Some((op_id, args)) = crate::mint::profile_args(kind, draft) {
            self.author_profile(kind, &object_id, op_id, args).await?;
        }

        // A Place's fix is the one second delta a mint authors, because a Place minted
        // UNPLACED is a different thing (listed, never pinned) rather than the same
        // thing missing a field. `placed` is what the CALLER asserts about lat/lng —
        // false mints unplaced rather than refusing, so slow indoor GPS never costs the
        // user their typing. Junk coordinates asserted as real are rejected by the
        // reducer, not stored.
        // THE MACRO-NODE. A kind declares what it is MADE OF (`ObjectKind::parts`,
        // pinned to the ICD's `x-object.parts`), and the mint brings the whole
        // shape into being: each part is a real GroupObject of its own, minted
        // here and attached by both halves of `part_of`.
        //
        // EAGERLY, ruled 24 September 2026. Lazily would spare a quiet Post its
        // second object, but it would also mean the part does not exist until
        // somebody acts — and then WHO mints it is a race, the same one
        // `conversation_open` had to settle by naming a side. The parent has one
        // owner at mint time and no race at all.
        for part in kind.parts() {
            let part_id = self.object_new(part.kind.name(), part.role)?;
            self.compose(&object_id, &part_id, part.role, unix_millis() as i64).await?;
        }

        if kind == crate::object::ObjectKind::Place && draft.placed {
            self.place_set_source(
                &object_id,
                &crate::geo::LocationSource::Fixed {
                    point: crate::geo::GeoPoint::from_degrees(draft.lat, draft.lng),
                },
            )
            .await?;
        }

        Ok(object_id)
    }

    /// Dispatch op 0 to the kind's own author. The five authors are separate functions
    /// (each pins its own kind string and its own `ObjectType`), so the unified mint
    /// needs one place that knows which is which — this is it, and it is exhaustive so
    /// a new mintable kind cannot be added without landing here.
    async fn author_profile(
        &self,
        kind: crate::object::ObjectKind,
        object_id: &str,
        op_id: u32,
        args: coordinator::Args,
    ) -> Result<String, CoreError> {
        use crate::object::ObjectKind as K;
        match kind {
            K::Group => self.apply(object_id, op_id, args).await,
            K::Project => self.apply(object_id, op_id, args).await,
            K::Thing => self.apply(object_id, op_id, args).await,
            K::Place => self.apply(object_id, op_id, args).await,
            K::Event => self.apply(object_id, op_id, args).await,
            K::Post => self.apply(object_id, op_id, args).await,
            // Forum has no profile op and `mint` never calls us for it; the rest are
            // refused by `classify` before the object is created.
            // A Treasury's op 0 is `setPolicy`, which is NOT a profile: a treasury
            // with no policy is a treasury nobody may spend from, which is the right
            // genesis state. `mint::classify` decides whether it is minted from a
            // draft at all; this arm exists so adding the kind could not slip past
            // the exhaustiveness check that brought me here.
            // A Note's op 0 is `write`, which is an entry, not a profile.
            // A Host's op 0 IS its name (`host.define`), so it mints with a
            // profile like a Group does — unlike Forum/Treasury/Note, whose op 0 is
            // a message, a policy and an entry.
            K::Host => self.apply(object_id, op_id, args).await,
            K::Treasury
            | K::Note
            | K::Forum
            | K::Field
            | K::System
            | K::Topic
            | K::Contact
            | K::Conversation
            | K::Transaction => Err(CoreError::Coordinator(format!(
                "{} has no profile op to author",
                kind.name()
            ))),
        }
    }

    /// The compiled [`GroupState`], folded over the current roster.
    /// THE PUBLIC PROJECTION of a published object, resolved by its slug.
    ///
    /// This is the ONE place that decides what a non-member may see. It lives in
    /// core rather than in whatever process serves HTTP, because the moment two
    /// consumers each derive their own public/private cut they drift, and the
    /// drift is only ever discovered as a leak.
    ///
    /// Returns `None` when no object this node folds claims `slug`, which is also
    /// what an unpublished or newly-unpublished object gives — the caller cannot
    /// tell those apart, and should not be able to.
    ///
    /// The publisher check is deliberate: an object is only served by the member
    /// it NAMED as its publisher. A node that happens to hold the deltas but was
    /// not named does not get to answer for the object.
    ///
    /// TWO checks, and both are load-bearing. The folded `publication.publisher`
    /// says who was NAMED; the live MLS roster says who is STILL HERE. Removing
    /// the publisher from the group is how publication is revoked, and removal
    /// only stops NEW deltas arriving — this node keeps the log it already has
    /// and would go on folding and serving it forever. So the roster is
    /// consulted at serve time, every time. Without that second check a site
    /// stays up, frozen at the moment of eviction, after the members voted the
    /// publisher out.
    pub fn published_site(
        &self,
        slug: &str,
        as_publisher: &[u8; 32],
    ) -> Result<Option<PublishedSite>, CoreError> {
        if !crate::publication::valid_slug(slug) {
            return Ok(None); // never scan on a string the fold could not have stored
        }
        for (object_id, kind) in self.all_groups()? {
            // LOWERCASE. The directory stores kind strings as written by
            // `object_new`/`put_group` — "group", never "Group" — so the capital
            // this read until 15 Sep 2026 matched nothing and `published_site`
            // could not return `Some` for any object ever created. It had no
            // callers, so nothing said so; the test below is the one that would
            // have. Compare against `ObjectKind::Group.as_str()` rather than a
            // literal, so a rename of the kind string is a compile error here
            // instead of a silent empty result.
            if kind != crate::object::ObjectKind::Group.name() {
                continue;
            }
            let st = match self.group_state(&object_id) {
                Ok(st) => st,
                Err(_) => continue, // an object we cannot fold cannot be published
            };
            if !st.publication.is_published() || st.publication.slug != slug {
                continue;
            }
            if st.publication.publisher.as_ref() != Some(as_publisher) {
                continue;
            }
            // Still a member? Removal is the revocation path, and it must bite
            // here rather than only at the next (never-arriving) delta.
            match self.group_roster(&object_id) {
                Ok(roster) if roster.contains(as_publisher) => {}
                // Not on the tree any more, or the tree cannot be read: either
                // way this node no longer has standing to answer for the object.
                _ => continue,
            }
            return Ok(Some(PublishedSite {
                object_id,
                slug: st.publication.slug.clone(),
                name: st.display_name.clone(),
                shape: st.shape.as_str().to_string(),
                cover: st.cover.clone(),
                cover_mime: st.cover_mime.clone(),
            }));
        }
        Ok(None)
    }

    pub fn group_state(&self, object_id_hex: &str) -> Result<GroupState, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        Ok(self.folded_group(&group_id)?.state())
    }

    /// The read-model the directory UI/CLI shows: profile + presence + card + the
    /// roles projected onto the LIVE MLS roster + `can_sync`. Membership comes
    /// from the roster, not the log.
    pub fn group_view(&self, object_id_hex: &str) -> Result<GroupView, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        let roster = self.dir.group_members(&group_id)?;
        Ok(self.folded_group(&group_id)?.state().view(&roster))
    }

    /// The role `member` holds in a Group-typed object — the identity record, or a tether.
    ///
    /// This is the READ side of the membership tether: an Arc calls it to learn what a caller
    /// may do, having verified they signed with the key that established the tether. It reads
    /// [`group_view`], so the role is projected onto the LIVE MLS roster — a role recorded for
    /// someone who has since left the roster never surfaces, and a role can never by itself
    /// confer membership (`setMemberRole` layers onto a real member; see the module docs).
    ///
    /// `None` means "no explicit role overlay", NOT "no access" — the caller decides the
    /// default (an Arc treats an un-roled member as its signup default).
    pub fn member_role(
        &self,
        object_id_hex: &str,
        member: &[u8; 32],
    ) -> Result<Option<group::GroupRole>, CoreError> {
        Ok(self
            .group_view(object_id_hex)?
            .roles
            .into_iter()
            .find(|(m, _)| m == member)
            .map(|(_, r)| r))
    }

    /// The `(seq, prev)` the next sequenced group delta must carry.
    fn group_next_seq(&self, group_id: &[u8], epoch: u64) -> Result<(u64, [u8; 32]), CoreError> {
        Ok(coordinator::next_sequenced_pos(
            self.folded_group(group_id)?.sequenced_head(),
            epoch,
        ))
    }



    /// Import a vCard file — which may hold MANY cards (an address-book export is
    /// `BEGIN..END` per contact). Each card mints (or re-homes) its own Group;
    /// returns one object id per card, in file order. Loud on a non-Group KIND
    /// (location/app/device) or a missing FN in ANY card — never a silent drop of
    /// N-1 contacts. MEMBER lines are advisory; credentials never import.
    pub async fn group_import_vcard(&self, vcf_text: &str) -> Result<Vec<String>, CoreError> {
        let cards = vcard::vcard_to_cards(vcf_text)
            .map_err(|e| CoreError::Coordinator(format!("vcard import: {e}")))?;
        let mut ids = Vec::with_capacity(cards.len());
        for pc in cards {
            ids.push(self.import_one_card(&pc).await?);
        }
        Ok(ids)
    }

    /// Author one parsed card's profile + presence onto a new or re-homed Group.
    async fn import_one_card(&self, pc: &vcard::ParsedCard) -> Result<String, CoreError> {
        // Re-home to an existing object ONLY if X-PACIFIC-GROUP names a group WE
        // OWN — writing an owner-only setProfile into a group we merely belong to
        // would brick it. Otherwise mint a fresh group-of-1. No silent
        // create-on-mismatch, no write to someone else's record.
        let object_id = match &pc.group_ref {
            Some(gref) if self.owned_group_exists(gref) => gref.clone(),
            _ => self.object_new("group", "")?,
        };

        let card_json = serde_json::to_string(&pc.card)
            .map_err(|e| CoreError::Coordinator(format!("card encode: {e}")))?;
        let mut profile = coordinator::Args::new();
        profile.insert(
            "displayName".into(),
            coordinator::ArgVal::Text(pc.display_name.clone()),
        );
        profile.insert(
            "shape".into(),
            coordinator::ArgVal::Text(pc.shape.as_str().to_string()),
        );
        profile.insert("card".into(), coordinator::ArgVal::Text(card_json));
        self.apply(&object_id, group::OP_SET_PROFILE, profile)
            .await?;

        let presence = match &pc.presence {
            Presence::OnPlatform(sid) => {
                let mut a = coordinator::Args::new();
                a.insert(
                    "kind".into(),
                    coordinator::ArgVal::Text("onPlatform".into()),
                );
                a.insert("identityKey".into(), coordinator::ArgVal::Text(sid.clone()));
                Some(a)
            }
            Presence::OffPlatform(hint) => {
                let mut a = coordinator::Args::new();
                a.insert(
                    "kind".into(),
                    coordinator::ArgVal::Text("offPlatform".into()),
                );
                if let Some(h) = hint {
                    a.insert("inviteHint".into(), coordinator::ArgVal::Text(h.clone()));
                }
                Some(a)
            }
            Presence::Unknown => None,
        };
        if let Some(a) = presence {
            self.apply(&object_id, group::OP_SET_PRESENCE, a)
                .await?;
        }
        Ok(object_id)
    }

    /// Export a Group as a vCard 4.0 string. MEMBER lines are derived from the
    /// live MLS roster; the credential vault is never serialised.
    pub fn group_export_vcard(&self, object_id_hex: &str) -> Result<String, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        let kind = self
            .dir
            .group_kind(&group_id)?
            .ok_or_else(|| CoreError::Directory(format!("no object {object_id_hex}")))?;
        if kind != "group" {
            return Err(CoreError::Coordinator(format!(
                "object kind '{kind}' is not a group"
            )));
        }
        let state = self.folded_group(&group_id)?.state();
        let roster = self.dir.group_members(&group_id)?;
        Ok(vcard::group_to_vcard(&state, object_id_hex, &roster))
    }

    /// Is this id a `group`-kind object that WE OWN? The re-home gate for vCard
    /// import: we only ever write setProfile/setPresence (owner-only ops) into our
    /// own records. A ref to a non-group object, a group we don't own, or a
    /// dangling id all fall through to minting a fresh stub instead.
    fn owned_group_exists(&self, object_id_hex: &str) -> bool {
        let Ok(gid) = self.object_group_id(object_id_hex) else {
            return false;
        };
        if self.dir.group_kind(&gid).ok().flatten().as_deref() != Some("group") {
            return false;
        }
        matches!(self.dir.group_owner(&gid), Ok(Some(o)) if o.as_slice() == self.id.identity_pk().as_slice())
    }

    /// The raw feature tree: one row per Delta in the log, content-addressed.
    /// (op_id, gen, author_short, delta_id_short, byte_len).
    #[allow(clippy::type_complexity)]
    pub fn object_log(
        &self,
        object_id_hex: &str,
    ) -> Result<Vec<(u32, u64, String, String, usize)>, CoreError> {
        let group_id = self.object_group_id(object_id_hex)?;
        let mut out = Vec::new();
        for (author, envelope) in self.dir.load_log(&group_id)? {
            let delta = coordinator::decode_delta(&envelope)?;
            out.push((
                delta.op_id,
                delta.gen.unwrap_or(0),
                short(&author),
                hex::encode(&delta.id()[..6]),
                envelope.len(),
            ));
        }
        Ok(out)
    }

    // ---- `pacific sync` — join pending Welcomes, then drain every group's shared
    //      tag: apply commits (advancing epoch), decrypt + fold application deltas.
    /// THE HELD SESSION (K-33; mdr/door.md §4 step 5). A pass takes it out, and puts
    /// it back only if nothing failed on it, so a broken socket is dropped and the
    /// next pass dials again. A Node that syncs every few seconds holds one relay
    /// connection instead of opening two or three a pass.
    async fn home_session(&self) -> Result<Router, CoreError> {
        let held = self.held.lock().map(|mut h| h.take()).unwrap_or(None);
        match held {
            Some(s) => Ok(s),
            None => Router::open(&self.routes).await,
        }
    }

    /// Whether this Node holds a relay session between passes.
    pub fn holds_session(&self) -> bool {
        self.held.lock().map(|h| h.is_some()).unwrap_or(false)
    }

    fn keep_session(&self, sess: Router) {
        if let Ok(mut h) = self.held.lock() {
            *h = Some(sess);
        }
    }

    // ---- THE HELD CONNECTION (O-69; mdr/fold-cache.md § How a read converges) ----

    /// Open the held connection to this device's relay, subscribed to every tag this Node
    /// listens on, and wait up to `within` for the relay to confirm the subscription. `ring` is
    /// called from the connection's thread whenever something was delivered; the caller
    /// coalesces, and calls [`Node::ingest_delivered`] on this Node's own thread. Whether the
    /// subscription was confirmed in time.
    pub fn go_live(&self, ring: Box<dyn Fn() + Send + Sync>, within: std::time::Duration) -> Result<bool, CoreError> {
        let url = self
            .routes
            .urls()
            .find(|u| u.starts_with("ws://") || u.starts_with("wss://"))
            .ok_or(CoreError::NoRelay)?
            .to_string();
        let live = crate::live::Live::start(url, self.dir.highest_cursor("relay")?, ring);
        live.set_tags(self.live_tags()?);
        let confirmed = live.wait_confirmed(within);
        *self.live.lock().unwrap_or_else(|p| p.into_inner()) = Some(live);
        Ok(confirmed)
    }

    fn live(&self) -> Option<crate::live::Live> {
        self.live.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn live_state(&self) -> std::sync::MutexGuard<'_, LiveState> {
        self.live_state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The tags the held connection listens on, sorted (FC-13).
    pub fn live_tags_held(&self) -> Vec<String> {
        self.live().map(|l| l.tags()).unwrap_or_default()
    }

    /// Tags this Node has drained from the relay since it opened, each a round trip (FC-15).
    pub fn tag_drains(&self) -> u64 {
        self.live_state().tag_drains
    }

    /// Whether this Node has a held connection, open or reconnecting.
    pub fn live_held(&self) -> bool {
        self.live().is_some()
    }

    /// Whether the held connection is open.
    pub fn live_up(&self) -> bool {
        self.live().is_some_and(|l| l.up())
    }

    /// Frames the held connection has delivered and this Node has not yet taken.
    pub fn live_pending(&self) -> bool {
        self.live().is_some_and(|l| l.pending())
    }

    /// Every tag this Node listens on over its home relay: its intro mailbox, and each held
    /// object's retained epoch tags. An object another Arc governs is synced as before.
    fn live_tags(&self) -> Result<Vec<String>, CoreError> {
        let mut out = vec![self.intro_wire()?.1];
        let home = self.routes.to_spec();
        for (g, _) in self.dir.all_groups()? {
            if self.dir.is_departed(&g)? || self.routes_for(&g).to_spec() != home {
                continue;
            }
            for (_, tag, _) in self.dir.epoch_tags(&g)? {
                out.push(pacific_wire::tag_hex(&tag));
            }
        }
        Ok(out)
    }

    /// THE SET FOLLOWS THE NODE: after a join, a departure or an epoch change.
    fn live_retag(&self) {
        if let Some(l) = self.live() {
            match self.live_tags() {
                Ok(t) => l.set_tags(t),
                Err(e) => tracing::warn!(target: "pacific::live", error = %e, "the held connection's tags were not updated"),
            }
        }
    }

    /// The intro mailbox's tag, and the tag it carries on the wire: unlike an object's, the
    /// relay knows it by its address (`Address::from_seed`), as `drain_intro` asks for it.
    fn intro_wire(&self) -> Result<([u8; 32], String), CoreError> {
        let tag = self.dir.my_intro_tag()?;
        Ok((tag, Address::from_seed(&tag).tag_hex()))
    }

    fn live_since(&self, hex: &str) -> Option<u64> {
        self.live().and_then(|l| l.since(hex))
    }

    /// A drain of `tag` began after the held connection confirmed it from `since`: the tag is
    /// complete, whatever its cursor.
    fn live_drained(&self, hex: String, confirmed: Option<u64>) {
        if let Some(since) = confirmed {
            self.live_state().drained.insert(hex, since);
        }
    }

    /// A TAG IS COMPLETE when everything the relay holds on it past this device's cursor has
    /// been delivered on the held connection or drained: its subscription is confirmed from
    /// `since`, and either the cursor has reached `since` (all before it drained, all after it
    /// delivered) or a drain began after the confirmation. A queue that overflowed dropped
    /// deliveries, and then only a drain counts.
    fn tag_complete(&self, live: &crate::live::Live, hex: &str, tag: &[u8; 32]) -> Result<bool, CoreError> {
        let Some(since) = live.since(hex) else { return Ok(false) };
        let (dropped, drained) = {
            let st = self.live_state();
            (st.dropped, st.drained.get(hex) == Some(&since))
        };
        Ok(drained || (!dropped && self.dir.cursor(tag, "relay")? >= since))
    }

    /// The held connection is open, holds nothing untaken on `tag`, and `tag` is complete with
    /// nothing on it waiting, in an object no commit has moved since its last drain: a drain
    /// of it would bring nothing.
    fn live_vouches(&self, group_id: &[u8], tag: &[u8; 32]) -> Result<bool, CoreError> {
        self.live_vouches_hex(&pacific_wire::tag_hex(tag), tag, Some(group_id))
    }

    fn live_vouches_hex(&self, hex: &str, tag: &[u8; 32], group_id: Option<&[u8]>) -> Result<bool, CoreError> {
        // Per tag: this device's own publish echoes back on its tag, and must not stop the
        // connection vouching for the others.
        let Some(live) = self.live().filter(|l| l.up() && !l.pending_on(hex)) else { return Ok(false) };
        {
            let st = self.live_state();
            if group_id.is_some_and(|g| st.follow.contains(g)) || st.joined || st.waiting.iter().any(|f| f.tag == hex) {
                return Ok(false);
            }
        }
        self.tag_complete(&live, hex, tag)
    }

    /// A tag a drain would make complete: confirmed, and not complete. One not yet confirmed
    /// waits for its `Eose`, which rings.
    fn tag_owes_drain(&self, live: &crate::live::Live, hex: &str, tag: &[u8; 32]) -> Result<bool, CoreError> {
        Ok(live.since(hex).is_some() && !self.tag_complete(live, hex, tag)?)
    }

    /// INGEST WHAT THE HELD CONNECTION DELIVERED, on this thread and with no round trip: each
    /// frame through the one ingest a drain uses (`process_intro_blob`, `process_group_blob`),
    /// the same quarantine and the same cursors, in the relay's order. A frame is never
    /// ingested ahead of what may precede it: one on a tag not yet complete waits, and so does
    /// everything after a commit, until [`Node::catch_up`] has drained them.
    pub fn ingest_delivered(&self) -> Result<Ingested, CoreError> {
        let Some(live) = self.live() else { return Ok(Ingested::default()) };
        let taken = live.take();
        let mut frames = {
            let mut st = self.live_state();
            if taken.overflowed {
                st.dropped = true;
                st.drained.clear();
            }
            std::mem::take(&mut st.waiting)
        };
        frames.extend(taken.frames);
        if frames.is_empty() {
            return Ok(Ingested::default());
        }
        frames.sort_by_key(|f| f.seq);
        let mut out = Ingested::default();
        let mut waiting = Vec::new();
        let (intro_tag, intro_hex) = self.intro_wire()?;
        let (intro, rest): (Vec<_>, Vec<_>) = frames.into_iter().partition(|f| f.tag == intro_hex);
        // Welcomes: joined here, and the new object is drained by `catch_up`'s full pass.
        if !intro.is_empty() {
            if self.tag_complete(&live, &intro_hex, &intro_tag)? {
                for f in intro {
                    if f.seq <= self.dir.cursor(&intro_tag, "relay")? {
                        continue;
                    }
                    match self.process_intro_blob(&intro_tag, &f.blob) {
                        Ok(()) => out.joined += 1,
                        Err(e) if e.is_retryable() => {
                            self.live_state().waiting.push(f);
                            return Err(e);
                        }
                        Err(e) => {
                            tracing::warn!(seq = f.seq, error = %e, "quarantining unprocessable intro blob");
                            self.dir.quarantine_put(None, &intro_tag, f.seq, &e.to_string(), f.blob.as_bytes(), unix_now())?;
                        }
                    }
                    self.dir.advance_cursor(&intro_tag, "relay", f.seq)?;
                }
            } else {
                waiting.extend(intro);
            }
        }
        // Group frames, by object, each object's in seq order.
        let home = self.routes.to_spec();
        let mut tags: std::collections::HashMap<String, (Vec<u8>, [u8; 32], zeroize::Zeroizing<[u8; 32]>)> = Default::default();
        for (g, _) in self.dir.all_groups()? {
            if self.dir.is_departed(&g)? || self.routes_for(&g).to_spec() != home {
                continue;
            }
            for (_, tag, secret) in self.dir.epoch_tags(&g)? {
                tags.insert(pacific_wire::tag_hex(&tag), (g.clone(), tag, secret));
            }
        }
        let mut by_group: Vec<(Vec<u8>, Vec<crate::live::Delivered>)> = Vec::new();
        for f in rest {
            // A tag not held (dropped since, or an epoch not yet recorded) is a drain's to bring.
            let Some((g, _, _)) = tags.get(&f.tag) else { continue };
            match by_group.iter_mut().find(|(id, _)| id == g) {
                Some((_, fs)) => fs.push(f),
                None => by_group.push((g.clone(), vec![f])),
            }
        }
        let follow = self.live_state().follow.clone();
        let mut departed = false;
        let (sid, sk, _name) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        for (g, fs) in by_group {
            let mut ready = !follow.contains(&g);
            for f in &fs {
                ready = ready && self.tag_complete(&live, &f.tag, &tags[&f.tag].1)?;
            }
            if !ready {
                waiting.extend(fs);
                continue;
            }
            let mut group = match mls::load_group(&client, &g) {
                Ok(group) => group,
                Err(_) => continue,
            };
            let mut fs = fs.into_iter();
            while let Some(f) = fs.next() {
                let (_, tag, secret) = &tags[&f.tag];
                if f.seq <= self.dir.cursor(tag, "relay")? {
                    continue;
                }
                match self.process_group_blob(&mut group, &g, tag, secret, &f.blob) {
                    Ok(advanced) => {
                        out.frames += 1;
                        if advanced {
                            // AN EPOCH MOVED: what follows on this object waits for a drain that
                            // records each epoch's tag as it passes it (`drain_group_tags`).
                            out.commits += 1;
                            self.dir.advance_cursor(tag, "relay", f.seq)?;
                            self.live_state().follow.insert(g.clone());
                            waiting.extend(fs);
                            break;
                        }
                    }
                    Err(e) if e.is_retryable() => {
                        waiting.push(f);
                        waiting.extend(fs);
                        self.live_state().waiting.extend(waiting);
                        return Err(e);
                    }
                    Err(e) => {
                        out.frames += 1;
                        tracing::warn!(seq = f.seq, error = %e, "quarantining unprocessable group blob");
                        self.dir.quarantine_put(Some(&g), tag, f.seq, &e.to_string(), f.blob.as_bytes(), unix_now())?;
                    }
                }
                self.dir.advance_cursor(tag, "relay", f.seq)?;
                if self.dir.is_departed(&g)? {
                    // Removed: nothing more is readable, and nothing more is listened for.
                    departed = true;
                    break;
                }
            }
        }
        if departed {
            self.live_retag();
        }
        out.waiting = waiting.len();
        let mut st = self.live_state();
        st.joined |= out.joined > 0;
        if waiting.iter().map(|f| f.blob.len()).sum::<usize>() > WAITING_BYTES {
            // Held past what a drain would cost: dropped, and every tag drained instead.
            waiting.clear();
            st.dropped = true;
            st.drained.clear();
        }
        st.waiting = waiting;
        drop(st);
        Ok(out)
    }

    /// Whether [`Node::catch_up`] has anything to do: frames wait, a commit moved an object, a
    /// queue overflowed, a Welcome was joined, or a tag is not complete.
    pub fn needs_catch_up(&self) -> Result<bool, CoreError> {
        let Some(live) = self.live() else { return Ok(false) };
        {
            let st = self.live_state();
            if st.joined || !st.follow.is_empty() || st.dropped || st.waiting.iter().any(|f| live.since(&f.tag).is_some()) {
                return Ok(true);
            }
        }
        let (intro_tag, intro_hex) = self.intro_wire()?;
        if self.tag_owes_drain(&live, &intro_hex, &intro_tag)? {
            return Ok(true);
        }
        let home = self.routes.to_spec();
        for (g, _) in self.dir.all_groups()? {
            if self.dir.is_departed(&g)? || self.routes_for(&g).to_spec() != home {
                continue;
            }
            for (_, tag, _) in self.dir.epoch_tags(&g)? {
                if self.tag_owes_drain(&live, &pacific_wire::tag_hex(&tag), &tag)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// DRAIN WHAT THE HELD CONNECTION CANNOT VOUCH FOR, on the held session: the intro mailbox
    /// and each object with a tag not yet complete or an epoch moved, one round trip per tag.
    /// That happens after a (re)subscription, an epoch change or an overflow, never on a read.
    /// A Welcome joined runs the full pass, as a join always has. Then what waited is ingested.
    pub async fn catch_up(&self) -> Result<Ingested, CoreError> {
        let Some(live) = self.live() else { return Ok(Ingested::default()) };
        let (intro_tag, intro_hex) = self.intro_wire()?;
        let intro = self.tag_owes_drain(&live, &intro_hex, &intro_tag)?;
        let (mut groups, dropped, joined) = {
            let mut st = self.live_state();
            (std::mem::take(&mut st.follow), st.dropped, std::mem::take(&mut st.joined))
        };
        groups.extend(self.groups_owing_drains(&live)?);
        let mut out = Ingested::default();
        if intro || !groups.is_empty() {
            let mut sess = self.home_session().await?;
            let drained = async {
                let joined = if intro { self.drain_intro(&mut sess).await? } else { 0 };
                self.drain_groups_at_once(&mut sess, &groups).await?;
                Ok::<usize, CoreError>(joined)
            }
            .await;
            match drained {
                Ok(joined) => {
                    self.keep_session(sess);
                    out.joined = joined;
                    out.drained = groups.len();
                }
                Err(e) => {
                    sess.close().await;
                    self.live_state().follow.extend(groups);
                    return Err(e);
                }
            }
        }
        if joined || out.joined > 0 {
            // A JOIN IS FOLLOWED BY THE FULL PASS, as it always was: it records the new
            // object's epochs, and what joining owes (the self record, the spine).
            if let Err(e) = self.sync_once().await {
                self.live_state().joined = true;
                return Err(e);
            }
        }
        if dropped {
            // An overflow left no tag complete but by a drain, so every object was drained.
            let mut st = self.live_state();
            if st.follow.is_empty() {
                st.dropped = false;
            }
        }
        self.live_retag();
        let more = self.ingest_delivered()?;
        out.frames += more.frames;
        out.commits += more.commits;
        out.joined += more.joined;
        out.waiting = more.waiting;
        Ok(out)
    }

    /// The home objects with a tag the held connection cannot vouch for yet: confirmed, and
    /// neither reached by its cursor nor drained since. Each new epoch's tag is one, once.
    fn groups_owing_drains(&self, live: &crate::live::Live) -> Result<std::collections::BTreeSet<Vec<u8>>, CoreError> {
        let home = self.routes.to_spec();
        let mut out = std::collections::BTreeSet::new();
        for (g, _) in self.dir.all_groups()? {
            if self.dir.is_departed(&g)? || self.routes_for(&g).to_spec() != home {
                continue;
            }
            for (_, tag, _) in self.dir.epoch_tags(&g)? {
                if self.tag_owes_drain(live, &pacific_wire::tag_hex(&tag), &tag)? {
                    out.insert(g.clone());
                    break;
                }
            }
        }
        Ok(out)
    }

    /// Every tag of these objects drained in ONE pipelined request (`drain_in_rounds`), not
    /// one round trip a tag: an object that gained epochs at every join owed a drain for each.
    async fn drain_groups_at_once(&self, sess: &mut Router, groups: &std::collections::BTreeSet<Vec<u8>>) -> Result<(), CoreError> {
        if groups.is_empty() {
            return Ok(());
        }
        let (sid, sk, _name) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut held: Vec<(Vec<u8>, mls::Group)> = Vec::new();
        for g in groups {
            if self.dir.is_departed(g)? {
                continue;
            }
            if let Ok(group) = mls::load_group(&client, g) {
                held.push((g.clone(), group));
            }
        }
        self.drain_in_rounds(sess, &mut held).await
    }

    /// EVERYTHING THE RELAY HOLDS FOR THIS OBJECT IS INGESTED, as far as the held connection
    /// can vouch: it is open, nothing delivered on its tags waits untaken, nothing for it waits for
    /// a drain, and each of its tags is complete. A writer that sees this authors at the head it
    /// holds, with no drain.
    pub fn caught_up(&self, group_id: &[u8]) -> Result<bool, CoreError> {
        let Some(live) = self.live().filter(|l| l.up()) else { return Ok(false) };
        {
            let st = self.live_state();
            if st.follow.contains(group_id) || st.joined {
                return Ok(false);
            }
        }
        if self.dir.is_departed(group_id)? || self.routes_for(group_id).to_spec() != self.routes.to_spec() {
            return Ok(false);
        }
        let tags = self.dir.epoch_tags(group_id)?;
        if tags.is_empty() {
            return Ok(false);
        }
        for (_, tag, _) in tags {
            let hex = pacific_wire::tag_hex(&tag);
            if live.pending_on(&hex) || self.live_state().waiting.iter().any(|f| f.tag == hex) || !self.tag_complete(&live, &hex, &tag)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub async fn sync_once(&self) -> Result<Vec<String>, CoreError> {
        let mut sess = boxed(|| self.home_session()).await?;

        // 1. bootstrap: drain OUR intro mailbox, unseal {scanner_pk, kind, welcome},
        //    join. Same ingest taxonomy as the group mailboxes: a Welcome that can
        //    PERMANENTLY never join (orphaned — its key package was already
        //    consumed by an earlier join; garbage bytes; a forged owner) is
        //    quarantined and the cursor advances. This is precisely the poison
        //    observed on 2026-07-10: repeated pair-scans re-sealed Welcomes built
        //    on one already-consumed key package, and the old `?` here refetched
        //    and re-failed the orphan on every sync, wedging the whole node.
        self.drain_intro(&mut sess).await?;

        // 1a. WHAT THE HELD CONNECTION CANNOT VOUCH FOR YET, drained at once (O-69): each
        //     object's steps (a) and (b) below then find its tags vouched, and skip them.
        if let Some(live) = self.live() {
            let owed = self.groups_owing_drains(&live)?;
            self.drain_groups_at_once(&mut sess, &owed).await?;
        }

        // 1b. the spine. Account-level, not per-Arc — no Arc governs which objects
        //     a person belongs to — so it rides the DEVICE relay session already
        //     open here rather than dialling again. ISOLATION, as with the object
        //     loop below: a relay that will not take the spine must not stop the
        //     mail being delivered, so this reports and continues. What it owes is
        //     never forgotten — the rows stay undelivered and
        //     `pending_spine_entries` still names them, so the next sync retries.
        match self.drain_spine_over(&mut sess).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(target: "pacific::spine", published = n, "spine drained"),
            Err(e) => tracing::warn!(
                target: "pacific::spine",
                error = %e,
                "spine drain failed — objects minted on this device stay unnameable \
                 from the seed until it succeeds"
            ),
        }

        // The intro mailbox lived on the DEVICE relay (a Welcome is sealed to us
        // before any object — and thus any per-object Arc — exists). The objects this
        // device's own routes govern ride the same session; any other Arc gets its own.
        let home_spec = self.routes.to_spec();
        let mut home = Some(sess);

        // 2. every object: advance the epoch, re-drain every epoch tag for late
        //    messages, flush our outbox at the live epoch, then fold to a transcript.
        //    PER-ARC ROUTING: each object syncs through the Arc that governs it
        //    (moderation authority + Semaphore relay), so we bucket objects by their
        //    TRANSPORT SET and open ONE router per distinct set. An object whose Arc
        //    we have not learned yet uses this device's default routes. The mesh is in
        //    every set — an Arc decides where the relay leg goes and has nothing to say
        //    about the devices in the room. ISOLATION holds at two levels: a dead Arc
        //    relay fails only its own cohort (we move to the next), and within a cohort
        //    a single object's failure never blocks its siblings.
        let mut by_arc: std::collections::BTreeMap<String, Vec<Vec<u8>>> =
            std::collections::BTreeMap::new();
        for (group_id, _kind) in self.dir.all_groups()? {
            // A group this device was removed from is never synced again (§8.3).
            if self.dir.is_departed(&group_id)? {
                continue;
            }
            by_arc
                .entry(self.routes_for(&group_id).to_spec())
                .or_default()
                .push(group_id);
        }
        let mut printed = Vec::new();
        for (spec, groups) in by_arc {
            let own = spec == home_spec;
            let opened = match home.take().filter(|_| own) {
                Some(s) => Ok(s),
                None => {
                    let routes = crate::router::Routes::parse(&spec);
                    boxed(|| Router::open(&routes)).await
                }
            };
            let mut sess = match opened {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(routes = %spec, error = %e,
                                   "no transport for these objects; skipping them this sync");
                    continue;
                }
            };
            let mut dead = false;
            for group_id in groups {
                match boxed(|| self.sync_group(&mut sess, &group_id, &mut printed)).await {
                    Ok(()) => {}
                    // this Arc's relay is dead for the whole cohort — stop it, try the next Arc.
                    Err(CoreError::Transport(_)) => {
                        dead = true;
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(group = %hex::encode(&group_id[..group_id.len().min(6)]),
                                       error = %e, "object sync failed; continuing with siblings");
                    }
                }
            }
            if own && !dead {
                home = Some(sess);
            } else {
                sess.close().await;
            }
        }
        // 3. RECONCILE the profile fan-out. Anything that gave a connection a stale
        //    copy of our card — a pairing formed after the last edit, an intro
        //    Welcome joined in step 1 above, an append that failed mid-fan-out — is
        //    caught here, because this compares each connection's recorded digest
        //    against the live profile rather than trusting that the edit-time
        //    fan-out reached everyone. When nothing is stale it authors nothing and
        //    touches no relay, so it is free on the steady-state 3-second tick.
        match boxed(|| self.publish_profile(false)).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(connections = n, "published profile to stale connections"),
            // Never fail a whole sync over the profile: messages matter more.
            Err(e) => tracing::warn!(error = %e, "profile reconciliation failed this sync"),
        }

        // 3a. RECONCILE the self record's vertebrae. Same shape, same reason: an
        //     object minted or joined between syncs is on the spine immediately
        //     (`object_new` queues the entry or fails) and named in the fold only
        //     once this has run. Idempotent, so the steady tick authors nothing.
        match boxed(|| self.reconcile_joined()).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(objects = n, "named objects on the self record"),
            Err(e) => tracing::warn!(error = %e, "the self record could not be reconciled"),
        }

        // History bundles held for an object never joined (O-75): refused, named, after the hold.
        match self.expire_held_history() {
            Ok(0) => {}
            Ok(n) => tracing::warn!(bundles = n, "history bundles refused: their objects were never joined"),
            Err(e) => tracing::warn!(error = %e, "held history could not be reviewed"),
        }

        // 3a'. MY CARD (O-77), beside the vertebrae: into every held object whose kind carries
        //      the profiles facet, where it is missing or stale. A join publishes here on the
        //      sync after it; a rename, on the sync after the self record takes it.
        match self.reconcile_profiles_boxed().await {
            Ok(s) if s.published > 0 => tracing::info!(objects = s.published, "my card published"),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "my card could not be reconciled"),
        }

        // 3b. RECONCILE the market. The same self-healing shape as the profile pass, doing
        //     two jobs: fan my own postures out to any connection holding a stale copy, and
        //     RELAY onward every `network` listing I hold that has not yet reached the hop
        //     ceiling. The relay is what makes discovery multihop — a listing walks the mesh
        //     one sync at a time, so a neighbour three handshakes away finds the ladder
        //     without anyone's contact list ever being shared. Both halves are digest-gated,
        //     so a settled market authors nothing and touches no relay on the steady tick.
        match boxed(|| self.publish_market(false)).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(deltas = n, "published and relayed market listings"),
            // Never fail a whole sync over the market: messages matter more.
            Err(e) => tracing::warn!(error = %e, "market reconciliation failed this sync"),
        }

        // 3c. RECONCILE discovery. Answer every live question that reached us, forward
        //     the ones whose budget still reaches someone, and relay answers back
        //     toward their asker. Every gate reads a Contact fold that already
        //     contains what a previous pass authored, so a quiesced mesh authors
        //     nothing and touches no relay on the steady tick.
        match boxed(|| self.reconcile_discovery()).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(deltas = n, "answered/forwarded discovery"),
            // Never fail a whole sync over discovery: messages matter more.
            Err(e) => tracing::warn!(error = %e, "discovery reconciliation failed this sync"),
        }

        // 4. RECONCILE public-place joins. Same shape as the profile pass above, and for
        //    the same reason: the intent was recorded when someone scanned a poster, but
        //    it usually could not be honoured at that moment (their prekeys are not
        //    stocked until they accept the pairing). Replaying it here means a public
        //    place admits people while its host is simply running, with no approval step
        //    and nothing for the host to remember to do. Cheap when there is nothing
        //    pending: one indexed query that returns no rows.
        match boxed(|| self.reconcile_place_joins()).await {
            Ok(v) if v.is_empty() => {}
            Ok(v) => tracing::info!(count = v.len(), "auto-admitted joiners to public places"),
            // Never fail a whole sync over a join: messages matter more, and the intent
            // is durable, so the next tick tries again.
            Err(e) => tracing::warn!(error = %e, "public-place join reconciliation failed"),
        }

        // 5. ANSWER THE DOOR at every open place we are in. This runs on EVERY member, not
        //    just whoever printed the sign — the doorbell is in the Place's log, so anyone
        //    who is in can let the next person in. That is the whole point: an allotment
        //    stays joinable while ANY plot-holder's app is running, not one specific one.
        match boxed(|| self.answer_all_doors()).await {
            Ok(v) if v.is_empty() => {}
            Ok(v) => tracing::info!(count = v.len(), "admitted knockers at public doors"),
            Err(e) => tracing::warn!(error = %e, "answering doors failed this sync"),
        }

        // 5. REPLENISH our prekey offers. Same shape and same reason as the two passes
        //    above: the offers a peer needs in order to add us are authored at pairing
        //    and at accept, both of which can fail or predate the code that writes
        //    them, and each add spends one. Without this a connection silently becomes
        //    un-addable — the failure surfaces much later, as "no usable prekey", to
        //    the OTHER party. Free when every channel is full.
        match boxed(|| self.replenish_prekeys()).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(offers = n, "replenished prekey offers"),
            Err(e) => tracing::warn!(error = %e, "prekey replenishment failed this sync"),
        }

        // 6. THE POOL (resumption.md §6.5): renew, evict, provision, and publish what
        //    the spine owes. Last, so it sees every group at its current epoch. Its
        //    failure is reported and never fails the sync.
        //    It rides the held session, and a pass that ends well keeps it.
        let upkeep = match home.take() {
            Some(s) => Ok(s),
            None => boxed(|| Router::open(&self.routes)).await,
        };
        match upkeep {
            Ok(mut sess) => match boxed(|| self.resumption_upkeep_at(&mut sess, unix_now())).await {
                Ok(_) => self.keep_session(sess),
                Err(e) => {
                    tracing::warn!(target: "pacific::resumption", error = %e, "pool upkeep failed this sync");
                    sess.close().await;
                }
            },
            Err(e) => tracing::warn!(target: "pacific::resumption", error = %e, "pool upkeep: no relay this sync"),
        }

        // 7. WHAT ANOTHER DEVICE OF THIS PERSON MADE OR JOINED, joined now rather than
        //    at the next sign-in. Last, so the self record it reads is folded (step 2).
        boxed(|| self.join_what_the_record_names()).await;

        // Delivery receipts are NOT a separate pass here: `sync_group` step (d) emits
        // them inline on the still-open per-Arc session, reusing the fold it already
        // did — so there is no second connect and no all-groups re-fold.

        // The held connection follows whatever this pass joined, left or advanced (O-69).
        self.live_retag();
        Ok(printed)
    }

    /// SYNC STEP 1: drain OUR intro mailbox, unseal {scanner_pk, kind, welcome}, join. Same
    /// ingest taxonomy as the group mailboxes: a Welcome that can PERMANENTLY never join
    /// (orphaned — its key package was already consumed by an earlier join; garbage bytes; a
    /// forged owner) is quarantined and the cursor advances. Returns the Welcomes joined.
    async fn drain_intro(&self, sess: &mut Router) -> Result<usize, CoreError> {
        let (intro_tag, intro_hex) = self.intro_wire()?;
        // Vouched for by the held connection, as `drain_tag`'s: nothing to bring.
        if self.live_vouches_hex(&intro_hex, &intro_tag, None)? {
            return Ok(0);
        }
        let confirmed = self.live_since(&intro_hex);
        self.live_state().tag_drains += 1;
        let dir = &self.dir;
        let intro_msgs = sess
            .drain(&[intro_hex.clone()], |source| {
                dir.cursor(&intro_tag, source)
            })
            .await?;
        let mut joined = 0;
        for m in intro_msgs {
            let (seq, blob_b64) = (m.seq, m.blob);
            match self.process_intro_blob(&intro_tag, &blob_b64) {
                Ok(()) => joined += 1,
                Err(e) if e.is_retryable() => return Err(e),
                Err(e) => {
                    tracing::warn!(seq, source = m.source, error = %e,
                                   "quarantining unprocessable intro blob");
                    self.dir.quarantine_put(
                        None,
                        &intro_tag,
                        seq,
                        &e.to_string(),
                        blob_b64.as_bytes(),
                        unix_now(),
                    )?;
                }
            }
            self.dir.advance_cursor(&intro_tag, m.source, seq)?;
        }
        self.live_drained(intro_hex, confirmed);
        Ok(joined)
    }

    /// SYNC STEPS (a) AND (b) for one group.
    /// (a) advance the epoch by draining the CURRENT tag until no commit advances it,
    ///     recording each epoch's tag+secret as we pass through.
    /// (b) re-drain EVERY epoch tag we still hold, from its cursor. This catches application
    ///     messages that landed on a now-superseded epoch tag AFTER we advanced past it
    ///     (mls-rs retains the last EPOCH_RETENTION epochs' secrets to decrypt them; the
    ///     directory prunes tags in lockstep). Cursors make caught-up tags a cheap no-op.
    async fn drain_group_tags(
        &self,
        sess: &mut Router,
        group: &mut mls::Group,
        group_id: &[u8],
    ) -> Result<(), CoreError> {
        loop {
            let e = group.current_epoch();
            let eb = mls::epoch_be(e);
            let tag = mls::group_tag(group, &eb)?;
            let secret = mls::seal_conn_secret(group, &eb)?;
            self.dir.record_epoch_tag(group_id, e, &tag, &secret)?;
            if !self.drain_tag(sess, group, group_id, &tag, &secret).await? {
                break;
            }
        }
        for (_ep, tag, secret) in self.dir.epoch_tags(group_id)? {
            self.drain_tag(sess, group, group_id, &tag, &secret).await?;
            if self.dir.is_departed(group_id)? {
                break;
            }
        }
        Ok(())
    }

    /// Sync ONE group end-to-end (steps a–d). Split out so `sync_once` can
    /// isolate per-group failures.
    async fn sync_group(
        &self,
        sess: &mut Router,
        group_id: &[u8],
        printed: &mut Vec<String>,
    ) -> Result<(), CoreError> {
        // A group this device was removed from has no MLS state left, and nothing to
        // sync (membership-through-mls.md §8.3).
        if self.dir.is_departed(group_id)? {
            return Ok(());
        }
        let (sid, sk, _name) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        let mut group = match mls::load_group(&client, group_id) {
            Ok(g) => g,
            Err(_) => return Ok(()),
        };

        // Self-heal / migration backfill: a pre-slice group (or one whose
        // projection was never populated) rebuilds its member set from the MLS
        // roster — the source of truth.
        if self.dir.group_members(group_id)?.is_empty() {
            self.dir
                .set_group_members(group_id, &mls::roster_identities(&group)?)?;
        }

        // (a) and (b): the current tag to head, then every retained epoch's tag.
        self.drain_group_tags(sess, &mut group, group_id).await?;

        // A commit in the drain removed this device: stop here (§8.3).
        if self.dir.is_departed(group_id)? {
            return Ok(());
        }

        // (b3) MEMBERSHIP UPKEEP (§6.2, §6.4, §7, §11, §3.3): complete what others
        //      proposed, propose a lapsed leave again, refresh this leaf when due,
        //      migrate a legacy group's owner. It commits through the slot with its own
        //      copy of the group, so ours is reloaded afterwards — writing a stale copy
        //      back would overwrite the newer state.
        drop(group);
        self.membership_upkeep(sess, group_id).await;
        if self.dir.is_departed(group_id)? {
            return Ok(());
        }
        let mut group = match mls::load_group(&client, group_id) {
            Ok(g) => g,
            Err(_) => return Ok(()),
        };

        // (b2) a CONNECTION may have just carried a peer's new profile. Fold the
        //      Contact lens and refresh the cached `peers.display_name` from it, so a
        //      peer's rename reaches the connections list / chat headers / derived
        //      group names without each of those surfaces knowing about the fold.
        //      Connections only: it is a second decode pass over the log, and a
        //      connection's log is a 1:1 DM — the small one. Forums never carry a
        //      Contact lens, so there would be nothing to find there anyway.
        if self.dir.group_kind(group_id)?.as_deref() == Some("connection") {
            let _ = self.refresh_peer_profile_cache(group_id);
        }

        // (c) flush our outbox at the NOW-current epoch — only for shared groups
        //     (a solo group never touches the relay). Draining FIRST means
        //     flush_one always targets the live epoch, so a post queued at an
        //     older epoch is re-sent to the current tag, never stranded.
        //     LEAVES, not members — see `Directory::group_leaf_count`.
        if self.dir.group_leaf_count(group_id)? > 1 {
            for (did, env) in self.dir.pending_outbox(group_id, &self.id.identity_pk())? {
                let _ = self.flush_one(sess, &mut group, group_id, &did, &env).await;
            }
        }

        // (d) fold ONCE, over the roster: rebuild the transcript AND emit a single
        //     batched DELIVERY receipt for any foreign messages we now hold but have
        //     not yet acknowledged. Reusing this fold (not a second pass) and the
        //     already-open `sess` is what keeps the periodic sync O(1 delta) instead
        //     of a write-per-message-per-tick.
        let state = self.forum_state(group_id)?;
        printed.extend(
            state
                .transcript()
                .into_iter()
                .map(|(a, g, t)| format!("[{} g{}] {}", short(&a), g, t)),
        );

        // Delivery ack: shared groups only, and only when something is actually owed
        // (a quiet group authors nothing). One delta covers the whole burst; it
        // seals at the head epoch we just advanced to and flushes on this session.
        // LEAVES, not members — see `Directory::group_leaf_count`.
        if self.dir.group_leaf_count(group_id)? > 1 {
            let me = self.id.identity_pk();
            let owed = Self::owed_receipts(&state, &me, coordinator::RECEIPT_DELIVERED);
            if !owed.is_empty() {
                let gen = self.next_lamport(group_id)?;
                let delta = coordinator::forum_receipt_batch(
                    &owed,
                    coordinator::RECEIPT_DELIVERED,
                    gen,
                    group.current_epoch(),
                )
                .retyped(self.chat_type_id(group_id)?);
                let envelope = delta.canonical_bytes();
                let delta_id = delta.id();
                self.dir.append_delta_signed(
                    group_id,
                    &delta_id,
                    &me,
                    &envelope,
                    Some(&crate::delta_sig::sign_delta(&self.id, group_id, &delta_id)),
                    unix_now(),
                )?;
                let _ = self
                    .flush_one(sess, &mut group, group_id, &delta_id, &envelope)
                    .await;
            }
        }
        Ok(())
    }

    /// Process ONE sealed intro-mailbox blob (a Welcome). Failures are permanent
    /// for the blob unless Directory/Io (retryable — this device's storage).
    fn process_intro_blob(&self, intro_tag: &[u8; 32], blob_b64: &str) -> Result<(), CoreError> {
        let blob = pacific_wire::blob_unb64(blob_b64)
            .map_err(|e| CoreError::Seal(format!("base64: {e}")))?;
        let payload_bytes = seal::open(&blob, intro_tag, intro_tag)?;
        // A HISTORY BUNDLE (O-75), not a Welcome: the object's admitter's rows, for this device
        // alone. Its own checks; a whole bundle refused is named, and quarantined by the caller.
        if let Some(bundle) = crate::history::decode(&payload_bytes)? {
            return self.ingest_history(bundle, intro_tag);
        }
        let payload = handshake::IntroPayload::decode(&payload_bytes)?;
        // The owner the ADDER CLAIMS. It is no longer taken on that word
        // (membership-through-mls.md §8.5): below, the owner comes from the Welcome's
        // GroupContext, and this claim must equal it.
        let claimed_owner = payload.owner.unwrap_or(payload.scanner_pk);
        let kind = payload
            .kind
            .clone()
            .unwrap_or_else(|| "connection".to_string());

        let (sid, sk, _name) = self.signer()?;
        let client = mls::build_client_sqlite(&paths::db_path(), sid, sk)?;
        // Joined in memory only: nothing is persisted until every check below passes,
        // so a refused Welcome leaves no group state on this device.
        // Who committed the Add that let this device in (O-75): a history bundle for this object
        // is taken only from them.
        let (mut group, committer) = mls::join_group_unsaved_from(&client, &payload.welcome)?;
        let group_id = group.group_id().to_vec();

        // Validated rejection (NOT a silent fallback): the sealed Welcome names its
        // sender, and the owner it claims for the group; require BOTH to actually be
        // members of the joined MLS roster, so a forged Welcome to our intro tag can
        // neither smuggle in an absent sender nor install an attacker as owner.
        // Quarantined by the caller.
        let roster = mls::roster_identities(&group)?;
        if !roster.contains(&payload.scanner_pk) {
            return Err(CoreError::Handshake(
                "welcome sender is not in the joined roster (possible forgery)".into(),
            ));
        }
        // THE OWNER COMES FROM THE CONTEXT (§8.5 step 1) — the one place every member
        // agrees on, covered by the confirmation tag. A legacy group answers with its
        // leaf-0 creator (§3.3).
        let owner_pk = mls::group_owner(&group)?;
        if claimed_owner != owner_pk {
            return Err(CoreError::Handshake(format!(
                "welcome claims owner {} but the group's context names {} (possible forgery)",
                hex::encode(&claimed_owner[..6]),
                hex::encode(&owner_pk[..6])
            )));
        }
        if !roster.contains(&owner_pk) {
            return Err(CoreError::Handshake(
                "welcome names an owner outside the joined roster (possible forgery)".into(),
            ));
        }
        // THE CARD CHECK (§8.5 step 2). If this device knocked on a card that named the
        // owner, the group it is let into must be owned by that person. A member who
        // commits an Add AND an owner rewrite can fool a newcomer who trusts the Welcome
        // alone — the newcomer would join a fork only the attacker shares. The card is
        // the expectation the attack cannot forge.
        if let Some(expected) = self.dir.expected_owner(&group_id)? {
            if expected != owner_pk {
                tracing::warn!(
                    target: "pacific::membership",
                    group = %hex::encode(&group_id),
                    expected = %hex::encode(expected),
                    context_owner = %hex::encode(owner_pk),
                    "refusing a Welcome whose owner differs from the card it answers"
                );
                return Err(CoreError::Handshake(
                    "the group's owner is not the owner named on the card that was knocked on".into(),
                ));
            }
        }
        // THE ROUTE (SECURITY, 27 Sep). `payload.arc` is the sender's word, stored below
        // and dialled by `routes_for`. Plain ws:// to a public host carries tags and
        // timing in the clear, anything else that is not a relay URL cannot be dialled,
        // and joining without the route would leave this member silently off the
        // group's relay: refused, by name. Absent or empty is the default relay.
        if let Some(why) = payload.arc.as_deref().filter(|a| !a.is_empty()).and_then(route_refusal) {
            return Err(CoreError::Handshake(format!("the group's Arc route is {why}")));
        }
        // Every check passed: the group is accepted, and only now persisted.
        mls::save_group(&mut group)?;
        self.dir.set_admitted_by(&group_id, &committer, unix_now())?;
        // Idempotent: only record on the FIRST join of this group. Inherit the
        // object's governing Arc from the Welcome (the creator's choice); `None`
        // (older sharer / web client) leaves routing to fall back to our default.
        // OUTSIDE the first-join guard below, and monotonic, because a re-add is
        // exactly the case that needs it: a device removed and re-added has a local
        // log that stopped at the old epoch while the group moved on, so the floor
        // it is handed the second time is the higher one. `raise_gen_floor` takes
        // the max, so an out-of-order or replayed Welcome can only move it forward.
        if let Some(mark) = payload.gen_watermark {
            self.dir.raise_gen_floor(&group_id, mark)?;
        }
        // A fresh Add of a group this device was once removed from (§8.3 step 6).
        let readded = self.dir.is_departed(&group_id)?;
        if readded {
            self.dir.clear_departed(&group_id)?;
            tracing::info!(
                target: "pacific::membership",
                group = %hex::encode(&group_id),
                "re-added to a group this device had left"
            );
        }
        // The owner from the join epoch on (§10.2), and this leaf's freshness (§11).
        self.dir.record_owner(&group_id, group.current_epoch(), &owner_pk)?;
        self.dir
            .record_self_update(&group_id, unix_now(), group.current_epoch())?;
        tracing::info!(
            target: "pacific::membership",
            group = %hex::encode(&group_id),
            kind = %kind,
            owner = %hex::encode(owner_pk),
            epoch = group.current_epoch(),
            "joined from a Welcome"
        );
        // The account joining is a spine entry — on the first join of this group, or
        // a re-add after leaving it, whose floor is the new join epoch.
        if readded || self.dir.group_kind(&group_id)?.is_none() {
            self.spine_on_join(&group, &group_id);
        }
        if self.dir.group_kind(&group_id)?.is_none() {
            self.dir
                .put_group(&group_id, &kind, &owner_pk, payload.arc.as_deref())?;
            self.dir
                .set_group_members(&group_id, &mls::roster_identities(&group)?)?;
            if kind == "connection" {
                self.dir.upsert_peer(
                    &owner_pk,
                    &payload.scanner_name,
                    PeerStatus::PendingIn,
                    None,
                    None,
                    unix_now(),
                )?;
                // Capture the "why" the n+1 chose at scan time — what Pacific remembers.
                if let Some(w) = &payload.why {
                    self.dir.set_peer_why(&owner_pk, w)?;
                }
            } else if kind == "arc-tether" {
                // Arc↔Arc tether: remember the peer Arc's name so the console can render
                // it. No contact seeding — the peer is an Arc, not a user connection.
                self.dir.upsert_peer(
                    &owner_pk,
                    &payload.scanner_name,
                    PeerStatus::Connected,
                    None,
                    None,
                    unix_now(),
                )?;
            }
        }
        // Its history, if the bundle came first and was held (O-75).
        self.take_held_history(&group_id, intro_tag);
        Ok(())
    }

    /// HISTORY FROM THE ADMITTER (O-75; mdr/arc-history.md): a bundle of `object`'s rows, which
    /// the member that let this device in sealed to it alone beside the Welcome, taken
    /// ([`Self::take_history`]), or held until what it needs has come: its Welcome, or, for one
    /// that carries no spine, the spine that gives its sender standing. A spine taken takes up
    /// what was held for it.
    fn ingest_history(&self, b: crate::history::Bundle, intro_tag: &[u8; 32]) -> Result<(), CoreError> {
        match self.take_history(&b)? {
            HistoryTaken::Early(why) => {
                self.dir.hold_history(&b.object, &crate::history::encode(&b), unix_now())?;
                tracing::info!(object = %hex::encode(&b.object), "a history bundle held: {why}");
            }
            HistoryTaken::Taken { spine } if spine > 0 => self.take_held_history(&b.object, intro_tag),
            HistoryTaken::Taken { .. } => {}
        }
        Ok(())
    }

    /// One history bundle's checks, and what of it is stored. Several may come for one object:
    /// the spine's, then cards' and posts', each signed and taken alone. Stored only when:
    /// 1. it is for this device's person, its sender's signature verifies, the object is a Site
    ///    or a room (H-3), and the sender committed this device's Add and is its owner or holds
    ///    admitter there;
    /// 2. each row's Delta id is its envelope's hash and its A-10 signature verifies for this
    ///    object;
    /// 3. a spine row's author is the owner; a card's (`base.publishProfile`, its subject its
    ///    author) is in the object's roster as this device holds it, and it folds as a card, last
    ///    one wins by gen, so a card that comes directly later supersedes it; any other row
    ///    (posts, reacts, receipts, retracts, records) is taken on the admitter's word, with no
    ///    check that its author was a member when writing (Ralph, 29 Sep, W-87: rule (iii)).
    /// `Early` when the object is not yet joined,
    /// or when a bundle with no spine finds its sender with no standing in what this device holds
    /// yet: its spine may come after it. A whole bundle refused is an `Err` naming why; a row
    /// refused is quarantined by name and the rest are taken. A row already held is ignored, and
    /// a stored one keeps its admitter as `via`.
    fn take_history(&self, b: &crate::history::Bundle) -> Result<HistoryTaken, CoreError> {
        let bad = |why: String| CoreError::Handshake(format!("history bundle: {why}"));
        let short = |k: &[u8; 32]| hex::encode(&k[..6]);
        if b.joiner != self.id.identity_pk() {
            return Err(bad(format!("for {}, not this device's person", short(&b.joiner))));
        }
        crate::history::verify(b)?;
        let object = b.object.clone();
        let Some(kind) = self.dir.group_kind(&object)? else {
            // Before its Welcome: kept until the Welcome joins the object (`take_held_history`),
            // and refused if it never does (`expire_held_history`).
            return Ok(HistoryTaken::Early("for an object this device was never admitted to".into()));
        };
        if kind != "group" && kind != "forum" {
            return Err(bad(format!("for a {kind}, whose history is never shared (H-3)")));
        }
        match self.dir.admitted_by(&object)? {
            Some(c) if c == b.sender => {}
            Some(c) => return Err(bad(format!("sent by {}, not {}, which committed this device's Add", short(&b.sender), short(&c)))),
            None => return Err(bad("this device holds no record of who let it into the object".into())),
        }
        let owner = self.group_owner_id(&object)?;
        let members = self.dir.group_members(&object)?;
        // Where a refused row is named: this bundle's own tag, one entry per row. Written once the
        // bundle is not early, so a bundle held and taken later names each row once.
        let tag: [u8; 32] = {
            use sha2::Digest;
            sha2::Sha256::new().chain_update(b"wallflowers/history/quarantine").chain_update(&object).chain_update(b.sig).finalize().into()
        };
        let now = unix_now();
        let mut refused: Vec<(usize, String)> = Vec::new();
        // CHECKS 2 AND 3, row by row.
        let mut spine: Vec<([u8; 32], &crate::history::Row)> = Vec::new();
        let mut cards: Vec<([u8; 32], &crate::history::Row)> = Vec::new();
        let mut rest: Vec<([u8; 32], &crate::history::Row)> = Vec::new();
        for (i, r) in b.rows.iter().enumerate() {
            match history_row(&object, &owner, r) {
                Ok((id, HistoryTier::Spine)) => spine.push((id, r)),
                Ok((_, HistoryTier::Card)) if !members.contains(&r.author) => {
                    refused.push((i, format!("a card by {}, not in the object's roster as this device holds it", short(&r.author))))
                }
                Ok((id, HistoryTier::Card)) => cards.push((id, r)),
                Ok((id, HistoryTier::Rest)) => rest.push((id, r)),
                Err(why) => refused.push((i, why)),
            }
        }
        let name_refused = |refused: &[(usize, String)]| {
            for (i, why) in refused {
                let _ = self.dir.quarantine_put(Some(&object), &tag, *i as u64, &format!("history row {i}: {why}"), &b.rows[*i].payload(), now);
            }
        };
        // CHECK 1's standing, AFTER the rows are verified: the admitter grant folded from what
        // this device holds with the bundle's owner-signed spine, which may be all it has of it
        // (the owner offline, J-A). A spine that does not chain onto the held log is refused.
        let owners = self.dir.owner_history(&object)?;
        let held = self.dir.load_log(&object)?;
        let standing = |log: Vec<(crate::object::MemberId, Vec<u8>)>| -> Result<bool, CoreError> {
            Ok(if kind == "group" {
                crate::fold::fold_entries_owned::<GroupType>(members.clone(), owners.clone(), log)?.state().member_roles.get(&b.sender)
                    == Some(&group::GroupRole::Admitter)
            } else {
                crate::fold::fold_entries_owned::<coordinator::ForumType>(members.clone(), owners.clone(), log)?.state().roles.get(&b.sender)
                    == Some(&group::GroupRole::Admitter)
            })
        };
        let with = held.iter().cloned().chain(spine.iter().map(|(_, r)| (r.author, r.envelope.clone()))).collect();
        let (chains, admitter) = match standing(with) {
            Ok(a) => (true, a),
            Err(e) => {
                for (i, r) in b.rows.iter().enumerate() {
                    if spine.iter().any(|(_, s)| std::ptr::eq(*s, r)) {
                        refused.push((i, format!("the spine does not chain onto what this device holds: {e}")));
                    }
                }
                (false, standing(held).unwrap_or(false))
            }
        };
        // THE OWNER HAS STANDING of its own: a founder adding the Arc to a room that already has a
        // log seals it that log (Ralph, 30 Sep: every joiner into every room). Admitter is a grant
        // the owner makes, not one it holds.
        let admitter = admitter || b.sender == owner;
        let no_standing = format!("sent by {}, which holds no admitter standing in the object", short(&b.sender));
        if !admitter && spine.is_empty() {
            return Ok(HistoryTaken::Early(no_standing));
        }
        name_refused(&refused);
        if !admitter {
            return Err(bad(no_standing));
        }
        let mut stored = 0usize;
        if chains {
            for (id, r) in &spine {
                if self.dir.append_delta_via(&object, id, &r.author, &r.envelope, &r.sig, &b.sender, now)? {
                    stored += 1;
                }
            }
        }
        let spine_stored = stored;
        for (id, r) in cards.iter().chain(&rest) {
            if self.dir.append_delta_via(&object, id, &r.author, &r.envelope, &r.sig, &b.sender, now)? {
                stored += 1;
            }
        }
        tracing::info!(object = %hex::encode(&object), stored, spine = spine.len(), cards = cards.len(), rest = rest.len(), via = %short(&b.sender), "history from the admitter");
        Ok(HistoryTaken::Taken { spine: spine_stored })
    }

    /// The bundles held for `object`, taken up now it is joined, or its spine taken; round again
    /// while a round takes one, since a spine held beside cards can come in either order. One
    /// still early stays held.
    fn take_held_history(&self, object: &[u8], intro_tag: &[u8; 32]) {
        loop {
            let Ok(held) = self.dir.held_history() else { return };
            let mut moved = false;
            for (id, o, bytes, _) in held {
                if o != object {
                    continue;
                }
                let out = crate::history::decode(&bytes)
                    .and_then(|b| b.ok_or_else(|| CoreError::Handshake("history bundle: what was held is not one".into())))
                    .and_then(|b| self.take_history(&b));
                match out {
                    Ok(HistoryTaken::Early(_)) => continue,
                    Ok(HistoryTaken::Taken { .. }) => {}
                    Err(e) => {
                        let _ = self.dir.quarantine_put(Some(object), intro_tag, id as u64, &e.to_string(), &bytes, unix_now());
                    }
                }
                moved |= self.dir.drop_held_history(id).is_ok();
            }
            if !moved {
                return;
            }
        }
    }

    /// A bundle held past [`history_hold`] is taken if it now can be, and otherwise refused,
    /// named: one for an object never joined, or one whose sender's standing never came.
    fn expire_held_history(&self) -> Result<usize, CoreError> {
        let (intro_tag, _) = self.intro_wire()?;
        let now = unix_now();
        let hold = history_hold();
        let mut refused = 0;
        for (id, object, bytes, at) in self.dir.held_history()? {
            if now - at < hold {
                continue;
            }
            let out = crate::history::decode(&bytes)
                .and_then(|b| b.ok_or_else(|| CoreError::Handshake("history bundle: what was held is not one".into())))
                .and_then(|b| self.take_history(&b));
            let why = match out {
                Ok(HistoryTaken::Taken { .. }) => None,
                Ok(HistoryTaken::Early(why)) => Some(format!("history bundle: {why}")),
                Err(e) => Some(e.to_string()),
            };
            if let Some(why) = why {
                self.dir.quarantine_put(Some(&object), &intro_tag, id as u64, &why, &bytes, now)?;
                refused += 1;
            }
            self.dir.drop_held_history(id)?;
        }
        Ok(refused)
    }

    /// The bundles held for an object not yet joined: (object hex, sender, rows, held since in
    /// unix s).
    pub fn held_history(&self) -> Result<Vec<(String, [u8; 32], usize, i64)>, CoreError> {
        Ok(self
            .dir
            .held_history()?
            .into_iter()
            .filter_map(|(_, o, bytes, at)| {
                let b = crate::history::decode(&bytes).ok().flatten()?;
                Some((hex::encode(o), b.sender, b.rows.len(), at))
            })
            .collect())
    }

    /// The admitter a held row came via in a history bundle (O-75), or None for a row its
    /// author's own message brought.
    pub fn history_provenance(&self, object_hex: &str, delta_id: &[u8; 32]) -> Result<Option<[u8; 32]>, CoreError> {
        self.dir.delta_via(&self.object_group_id(object_hex)?, delta_id)
    }

    /// THE ONE PUBLISH OF A HISTORY BUNDLE (O-75): sealed to `intro_tag`, the joiner's, and posted
    /// to their intro mailbox, as a Welcome is. Refused, named, over the ICD's cap, or when the
    /// sealed blob is over the relay's limit (base64 characters of the sealed blob, the tighter).
    /// Which rows, and for whom, is the caller's: the Arc's send, or a test's forgery.
    pub async fn publish_history(&self, intro_tag: &[u8; 32], bundle: &crate::history::Bundle) -> Result<(), CoreError> {
        let plain = crate::history::encode(bundle);
        if plain.len() > crate::history::CAP {
            return Err(CoreError::Handshake(format!("history bundle: {} bytes, over the {} a bundle holds", plain.len(), crate::history::CAP)));
        }
        let sealed = seal::seal(&plain, intro_tag, intro_tag)?;
        let blob = pacific_wire::blob_b64(&sealed);
        if blob.len() > RELAY_DEFAULT_MAX_BLOB_B64 {
            return Err(CoreError::Handshake(format!(
                "history bundle: {} bytes seal to {} base64 characters, over the relay's {RELAY_DEFAULT_MAX_BLOB_B64}",
                plain.len(),
                blob.len()
            )));
        }
        let mut sess = Router::open(&self.routes).await?;
        let sent = sess.publish(&Address::from_seed(intro_tag), &blob).await;
        sess.close().await;
        sent.map(|_| ())
    }

    /// [`Self::publish_history`] to the joiner a contact bundle names. No sender-side checks.
    pub async fn send_history_raw(&self, joiner_contact_bundle: &str, bundle: &crate::history::Bundle) -> Result<(), CoreError> {
        let contact = handshake::parse_and_verify(joiner_contact_bundle)?;
        self.publish_history(&contact.intro_tag, bundle).await
    }

    /// The quarantined-blob evidence: (group_id?, seq, reason, at) newest first.
    #[allow(clippy::type_complexity)]
    pub fn quarantined(&self) -> Result<Vec<(Option<Vec<u8>>, u64, String, i64)>, CoreError> {
        self.dir.quarantine_list()
    }
}

/// How long a history bundle is held for an object this device has not joined (O-75), in seconds:
/// its Welcome normally comes first, and a bundle no Welcome follows is refused after this. An
/// hour, as the ICD's row 5 states; PACIFIC_HISTORY_HOLD_SECS shortens it for a test.
fn history_hold() -> i64 {
    std::env::var("PACIFIC_HISTORY_HOLD_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(3600)
}

/// What became of a history bundle (O-75; `Node::take_history`): taken, with how many spine rows
/// it stored; or early, held until what it needs comes, with the refusal it becomes if that never
/// does.
enum HistoryTaken {
    Taken { spine: usize },
    Early(String),
}

/// Which of a bundle's tiers a row is: the owner's sequenced spine, a member's card, or the rest
/// (posts, reacts, receipts, retracts, records), taken on the admitter's word.
enum HistoryTier {
    Spine,
    Card,
    Rest,
}

/// One history row's checks (O-75; `Node::take_history`): its Delta id is its envelope's hash
/// (the envelope is the Delta's canonical bytes), its A-10 signature verifies for `object`, a
/// spine row is the owner's, and a card folds as one by the facet's own reducer. Its id and its
/// tier; or why not. A card's author in the roster is the caller's.
fn history_row(object: &[u8], owner: &[u8; 32], r: &crate::history::Row) -> Result<([u8; 32], HistoryTier), String> {
    let d = coordinator::decode_delta(&r.envelope).map_err(|e| format!("not a Delta: {e}"))?;
    if d.canonical_bytes() != r.envelope {
        return Err("its envelope is not the Delta's canonical bytes, so its id is not their hash".into());
    }
    let id = d.id();
    crate::delta_sig::verify_delta(object, &r.author, &id, &r.sig)
        .map_err(|_| "its author's A-10 signature does not verify for this object".to_string())?;
    if d.seq.is_some() {
        if r.author != *owner {
            return Err(format!("a spine row by {}, not the object's owner", hex::encode(&r.author[..6])));
        }
        return Ok((id, HistoryTier::Spine));
    }
    if !crate::profiles::is_profile_op(d.op_id) {
        return Ok((id, HistoryTier::Rest));
    }
    let ctx = crate::object::ReduceContext { members: std::slice::from_ref(&r.author), owner: *owner, epoch: d.epoch };
    let op = crate::object::Op { op_id: d.op_id, args: &d.args, author: &r.author, pos: None, ctx: &ctx };
    crate::profiles::reduce_profiles(&mut Default::default(), &op).map_err(|e| format!("a card that does not fold: {e:?}"))?;
    Ok((id, HistoryTier::Card))
}

/// A well-formed Arc endpoint: `wss://` to a host, or `ws://` to a loopback or private
/// address — plain ws anywhere else carries routing tags and timing in the clear. Both
/// read by the dialler's parser ([`dialled_host`]). Arcs have static IPs, so the
/// endpoint is stable; we validate shape, never reachability.
fn is_arc_url(s: &str) -> bool {
    if s.starts_with("wss://") {
        return dialled_host(s).is_some();
    }
    s.starts_with("ws://") && ws_host_is_local(s)
}

/// Why `arc` may not be a group's route, or `None` when it is a relay URL: the one
/// check ([`is_arc_url`]) every door a route comes through asks, each refusal named
/// (SECURITY, 27 Sep). An absent or empty route is the default relay; the doors that
/// allow it say so before asking.
/// Set the default Arc of the device in the state directory, before any Node is open:
/// the Door's session takes its relay as its Arc, so what it mints is stamped with the
/// relay it is written on (NC-85). [`Node::set_default_arc`]'s rule.
pub fn set_default_arc(url: &str) -> Result<(), CoreError> {
    let url = url.trim();
    if let Some(why) = route_refusal(url) {
        return Err(CoreError::Governance(format!("this device's Arc route is {why}: {url:?}")));
    }
    std::fs::write(paths::arc_url_path(), url)?;
    Ok(())
}

fn route_refusal(arc: &str) -> Option<&'static str> {
    if is_arc_url(arc) {
        None
    } else if arc.starts_with("ws://") && dialled_host(arc).is_some() {
        Some("plain ws:// to a public host")
    } else {
        Some("not a relay URL")
    }
}

/// Why the restore refuses `way`, naming its route; `None` when the route is a relay
/// URL, or empty (the default relay).
fn way_in_refusal(way: &crate::spine::Pool) -> Option<String> {
    if way.arc.is_empty() {
        return None;
    }
    let why = route_refusal(&way.arc)?;
    Some(format!("the group's Arc route is {why}: {:?}", way.arc))
}

/// What a spine names (resumption.md §2.2): the self record at index 0, every object
/// joined and not left behind it in first-joined order, and the latest way-in of each.
struct SpinePlan {
    spine: Vec<u8>,
    targets: Vec<Vec<u8>>,
    ways: std::collections::HashMap<Vec<u8>, crate::spine::Pool>,
}

/// The [`SpinePlan`] of `entries`, read the one way `resume_at` and
/// `noncompliant_objects` both read it; `None` when index 0 is not a self record.
fn spine_plan(entries: &[(u64, crate::spine::Entry)]) -> Option<SpinePlan> {
    let spine = entries.first().and_then(|(i, e)| match &e.body {
        crate::spine::Body::Group(j) if *i == 0 => Some(j.group_id.clone()),
        _ => None,
    })?;
    let mut order: Vec<Vec<u8>> = Vec::new();
    let mut live: std::collections::HashMap<Vec<u8>, bool> = Default::default();
    let mut ways: std::collections::HashMap<Vec<u8>, crate::spine::Pool> = Default::default();
    for (_, e) in entries {
        match &e.body {
            crate::spine::Body::Group(j) => {
                if !live.contains_key(&j.group_id) {
                    order.push(j.group_id.clone());
                }
                live.insert(j.group_id.clone(), true);
            }
            crate::spine::Body::Left(d) => {
                live.insert(d.group_id.clone(), false);
            }
            crate::spine::Body::Pool(p) => {
                ways.insert(p.group_id.clone(), p.clone());
            }
        }
    }
    let targets: Vec<Vec<u8>> = std::iter::once(spine.clone())
        .chain(order.into_iter().filter(|g| *g != spine && live.get(g) == Some(&true)))
        .collect();
    Some(SpinePlan { spine, targets, ways })
}

/// The host `url` dials, read by the dialler's own parser — tungstenite's `http::Uri`,
/// whose `host()` tokio-tungstenite connects to — so the host checked is the host
/// dialled. `None` when it will not parse or names no host: what it will not parse, it
/// will not dial.
fn dialled_host(url: &str) -> Option<String> {
    let uri = url.parse::<tokio_tungstenite::tungstenite::http::Uri>().ok()?;
    uri.host().filter(|h| !h.is_empty()).map(str::to_string)
}

/// The host `url` dials ([`dialled_host`]) is localhost, 127/8, ::1, 10/8, 172.16/12,
/// 192.168/16 or fc00::/7.
fn ws_host_is_local(url: &str) -> bool {
    let Some(host) = dialled_host(url) else {
        return false;
    };
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(&host);
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private(),
        Ok(std::net::IpAddr::V6(ip)) => ip.is_loopback() || ip.segments()[0] & 0xfe00 == 0xfc00,
        Err(_) => false,
    }
}

/// The supermajority count for an `n`-member roster: ⌈2n/3⌉. Note this collapses to
/// unanimity for n ≤ 2 (⌈2/3⌉=1, ⌈4/3⌉=2) — you cannot out-vote a 1- or 2-person
/// object, so both must agree to move it to another Arc.
fn supermajority_threshold(n: usize) -> usize {
    (2 * n + 2) / 3
}

fn rand_tag() -> [u8; 32] {
    let mut t = [0u8; 32];
    OsRng.fill_bytes(&mut t);
    t
}

/// The signature keys of every leaf in `group` — which pool leaves are still in it.
fn pool_keys_in(group: &mls::Group) -> std::collections::HashSet<[u8; 32]> {
    group
        .roster()
        .members_iter()
        .filter_map(|m| <[u8; 32]>::try_from(m.signing_identity.signature_key.as_bytes()).ok())
        .collect()
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}
/// Wall-clock milliseconds since the epoch — the DISPLAY timestamp stamped onto a
/// post. Never used for ordering (that's the Lamport `gen`); purely the "10:44"
/// label the UI shows.
fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
fn short(pk: &[u8; 32]) -> String {
    hex::encode(&pk[..4])
}

#[cfg(test)]
mod arc_governance_tests {
    use super::{is_arc_url, route_refusal, supermajority_threshold};

    #[test]
    fn supermajority_is_two_thirds_ceiling_and_unanimous_for_tiny_rosters() {
        // ⌈2n/3⌉
        assert_eq!(supermajority_threshold(1), 1); // a solo object: only you
        assert_eq!(supermajority_threshold(2), 2); // both must agree — no out-voting
        assert_eq!(supermajority_threshold(3), 2);
        assert_eq!(supermajority_threshold(4), 3);
        assert_eq!(supermajority_threshold(5), 4);
        assert_eq!(supermajority_threshold(6), 4);
        assert_eq!(supermajority_threshold(9), 6);
        assert_eq!(supermajority_threshold(10), 7);
        // a bare majority is NOT enough to move jurisdiction: 5/9 < 6 needed.
        assert!(5 < supermajority_threshold(9));
    }

    #[test]
    fn arc_url_shape_is_validated_never_reachability() {
        assert!(is_arc_url("wss://uk.arc.example"));
        assert!(is_arc_url("wss://example.com"));
        assert!(is_arc_url("wss://203.0.113.9:9092")); // Arcs have static IPs
        // Plain ws only where the tags and timing never leave the machine or the LAN.
        assert!(is_arc_url("ws://127.0.0.1"));
        assert!(is_arc_url("ws://127.0.0.1:9092/relay"));
        assert!(is_arc_url("ws://localhost:9092"));
        assert!(is_arc_url("ws://[::1]:9092"));
        assert!(is_arc_url("ws://10.0.0.5"));
        assert!(is_arc_url("ws://172.16.0.1:9092"));
        assert!(is_arc_url("ws://172.31.255.255"));
        assert!(is_arc_url("ws://192.168.1.20:9092"));
        assert!(is_arc_url("ws://[fd12:3456::1]:9092"));
        assert!(!is_arc_url("ws://example.com"));
        assert!(!is_arc_url("ws://203.0.113.9:9092"));
        assert!(!is_arc_url("ws://172.32.0.1"));
        assert!(!is_arc_url("ws://[2001:db8::1]:9092"));
        assert!(!is_arc_url("ws://localhost.example.com"));
        assert!(!is_arc_url("ws://127.0.0.1.example.com"));
        assert!(!is_arc_url("ws://10.0.0.5@example.com")); // the host is after the @
        // A WHATWG parser dials evil.com; the dialler's parser refuses the backslash.
        assert!(!is_arc_url("ws://evil.com\\@127.0.0.1"));
        assert!(!is_arc_url("ws://evil.com\\@127.0.0.1:9092/relay"));
        assert!(is_arc_url("ws://[::1]"));
        assert!(is_arc_url("ws://[fd12:3456::1]/relay"));
        assert!(!is_arc_url("ws://[::ffff:8.8.8.8]:9092")); // v4-mapped public
        assert!(!is_arc_url("ws://[::1")); // unclosed bracket
        assert!(is_arc_url("ws://localhost"));
        assert!(is_arc_url("ws://127.0.0.1:1/relay?x=1"));
        assert!(!is_arc_url("ws://example.com:9092"));
        assert!(!is_arc_url("ws://")); // no host
        assert!(!is_arc_url("https://arc.example")); // must be a ws endpoint
        assert!(!is_arc_url("wss://")); // no host
        assert!(!is_arc_url("")); // empty
        assert!(!is_arc_url("arc.example")); // no scheme
        // wss:// by the same parser: a host, and the host dialled.
        assert!(is_arc_url("wss://host"));
        assert!(is_arc_url("wss://arc.wallflowers.io/v1/relay"));
        assert!(!is_arc_url("wss://evil.com\\@x"));
        assert!(!is_arc_url("wss://:443")); // a port is not a host
        assert!(!is_arc_url("wss://[::1")); // unclosed bracket
    }

    #[test]
    fn a_refused_route_is_named_for_why() {
        assert_eq!(route_refusal("wss://uk.arc.example"), None);
        assert_eq!(route_refusal("ws://127.0.0.1:9092"), None);
        assert_eq!(route_refusal("ws://203.0.113.9:9092"), Some("plain ws:// to a public host"));
        assert_eq!(route_refusal("ws://evil.com\\@127.0.0.1"), Some("not a relay URL"));
        assert_eq!(route_refusal("ws://"), Some("not a relay URL"));
        assert_eq!(route_refusal("http://127.0.0.1:8080"), Some("not a relay URL"));
        assert_eq!(route_refusal("wss://"), Some("not a relay URL"));
        assert_eq!(route_refusal(""), Some("not a relay URL"));
    }
}

#[cfg(test)]
mod join_card_tests {
    use super::Node;

    /// The doorbell every grammar test rings.
    const BELL: [u8; 32] = [0xAB; 32];

    fn bell_hex() -> String {
        hex::encode(BELL)
    }

    #[test]
    fn parse_round_trips_the_generated_grammar() {
        // The exact string place_join_card formats — the two sides must never drift.
        let card = format!("pacific://place-join?id=deadbeef01&db={}", bell_hex());
        let (id, bell) = Node::parse_place_join_card(&card).unwrap();
        assert_eq!(id, "deadbeef01");
        assert_eq!(bell, BELL);
    }

    #[test]
    fn unknown_params_ride_through_and_order_does_not_matter() {
        // C2: a later card may carry more; today's parser must not refuse it. And a
        // hand-assembled card that swaps the params is the same card.
        let card = format!(
            "pacific://place-join?v=2&db={}&note=west-gate&id=deadbeef01",
            bell_hex()
        );
        let (id, bell) = Node::parse_place_join_card(&card).unwrap();
        assert_eq!(id, "deadbeef01");
        assert_eq!(bell, BELL);
    }

    #[test]
    fn surrounding_whitespace_is_tolerated() {
        // A card pasted out of a message arrives with a newline more often than not.
        let card = format!("  pacific://place-join?id=aa&db={}\n", bell_hex());
        assert!(Node::parse_place_join_card(&card).is_ok());
    }

    #[test]
    fn malformed_cards_refuse_loudly() {
        // Wrong scheme entirely — a pairing QR must never parse as a place card.
        assert!(Node::parse_place_join_card("pacific://pair-scan?bundle=xx").is_err());
        // Missing doorbell.
        assert!(Node::parse_place_join_card("pacific://place-join?id=aa").is_err());
        // Missing id.
        let no_id = format!("pacific://place-join?db={}", bell_hex());
        assert!(Node::parse_place_join_card(&no_id).is_err());
        // Non-hex id — an id that could not name an object.
        let bad_id = format!("pacific://place-join?id=not-hex!&db={}", bell_hex());
        assert!(Node::parse_place_join_card(&bad_id).is_err());
        // Doorbell of the wrong width: 31 and 33 bytes, and odd-length hex.
        for db in [
            hex::encode([0u8; 31]),
            hex::encode([0u8; 33]),
            "abc".to_string(),
        ] {
            let card = format!("pacific://place-join?id=aa&db={db}");
            assert!(Node::parse_place_join_card(&card).is_err(), "db={db}");
        }
        // Empty string and empty id value.
        assert!(Node::parse_place_join_card("").is_err());
        let empty_id = format!("pacific://place-join?id=&db={}", bell_hex());
        assert!(Node::parse_place_join_card(&empty_id).is_err());
    }
}

/// A chat object's fold, under whichever wire type its kind names.
///
/// One `ForumState` either way — `ConversationType::reduce` delegates to
/// `ForumType::reduce`, which is what the ICD means by "shares Forum's
/// mechanics" — but `Coordinator<T>` hard-rejects a foreign `type_id`, so the two
/// cannot be one Rust type. Every reader below takes what it needs through here.
#[derive(Clone)]
enum ChatFold {
    Forum(coordinator::Coordinator<coordinator::ForumType>),
    Conversation(coordinator::Coordinator<coordinator::ConversationType>),
}

impl ChatFold {
    /// What the fold accepted (the fold cache's check, FC-2).
    fn accepted_digest(&self) -> [u8; 32] {
        match self {
            ChatFold::Forum(c) => c.accepted_digest(),
            ChatFold::Conversation(c) => c.accepted_digest(),
        }
    }

    fn state(&self) -> coordinator::ForumState {
        match self {
            ChatFold::Forum(c) => c.state(),
            ChatFold::Conversation(c) => c.state(),
        }
    }
    fn sequenced_head(&self) -> Option<(crate::object::LogPosition, [u8; 32])> {
        match self {
            ChatFold::Forum(c) => c.sequenced_head(),
            ChatFold::Conversation(c) => c.sequenced_head(),
        }
    }
    fn members(&self) -> &[crate::object::MemberId] {
        match self {
            ChatFold::Forum(c) => c.members(),
            ChatFold::Conversation(c) => c.members(),
        }
    }
    fn owner(&self) -> crate::object::MemberId {
        match self {
            ChatFold::Forum(c) => c.owner(),
            ChatFold::Conversation(c) => c.owner(),
        }
    }
    fn ratify_state(&self) -> coordinator::RatifyState {
        match self {
            ChatFold::Forum(c) => c.ratify_state(),
            ChatFold::Conversation(c) => c.ratify_state(),
        }
    }
    fn ratify_ballots_for(&self, target: &coordinator::ProposalId) -> Vec<[u8; 32]> {
        match self {
            ChatFold::Forum(c) => c.ratify_ballots_for(target),
            ChatFold::Conversation(c) => c.ratify_ballots_for(target),
        }
    }
}
