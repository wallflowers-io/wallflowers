//! pacific-ffi — the single FFI boundary between the Swift UI and the Rust core.
//!
//! It exposes exactly the M1 surface the iOS app needs today: mint/reload the
//! on-device identity, read the pairing bundle, and drive local (group-of-1)
//! Forum objects. All of these are synchronous — the relay/async methods
//! (pair/sync) come with the two-device slice, not this one.
//!
//! State location: pacific-core resolves its on-device state dir from
//! `PACIFIC_STATE_DIR` (see pacific_core::paths). iOS has no `~/.pacific`, so the Swift
//! app hands us its sandbox path once at construction and we point pacific-core at
//! it. This is the seam paths.rs was built for — no pacific-core change required.

uniffi::setup_scaffolding!();

/// The full Delta surface: bindings for every op in the GroupObject catalog.
pub mod delta;

/// The device-to-device mesh seam — Core Bluetooth moves bytes, everything else stays in Rust.
pub mod mesh;

use std::sync::Arc;

use pacific_core::Node;

/// One loud FFI error. pacific-core's typed `CoreError` is flattened to a message
/// at the boundary and surfaces in Swift as a native `throws`. No silent path.
#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum FfiError {
    #[error("{0}")]
    Core(String),
}

impl From<pacific_core::CoreError> for FfiError {
    fn from(e: pacific_core::CoreError) -> Self {
        FfiError::Core(e.to_string())
    }
}

/// Parse a 64-char identity hex into the 32-byte peer id — loud on anything else.
fn parse_peer_hex(s: &str) -> Result<[u8; 32], FfiError> {
    hex::decode(s)
        .ok()
        .and_then(|v| <[u8; 32]>::try_from(v).ok())
        .ok_or_else(|| FfiError::Core(format!("'{s}' is not a 64-char identity hex")))
}

/// Inject the device master key (held in the iOS Keychain / Secure Enclave) into the
/// core's at-rest layer, so `id_ed25519` and every secret in `pacific.db` seal under
/// it (workstream S5). MUST be called at app startup BEFORE the identity or DB is
/// touched — otherwise the core reads/writes plaintext, or a sealed store fails to
/// open. Exactly 32 bytes; any other length is rejected loudly.
#[uniffi::export]
pub fn set_device_key(key: Vec<u8>) -> Result<(), FfiError> {
    let arr: [u8; 32] = key
        .as_slice()
        .try_into()
        .map_err(|_| FfiError::Core(format!("device key must be 32 bytes, got {}", key.len())))?;
    pacific_core::atrest::set_device_key(arr);
    Ok(())
}

/// Clear the injected device key (host sign-out / lock). Sealed secrets can no longer
/// be opened until the key is re-injected — by design (no key, no plaintext).
#[uniffi::export]
pub fn clear_device_key() {
    pacific_core::atrest::clear_device_key();
}

/// The two halves of the account channel derived from a passkey's PRF secret:
/// 64 bytes, `tag` (0..32) then `seal` (32..64).
///
/// A person signed into the same passkey on this phone and in a browser derives
/// the SAME bytes here, without the two devices ever meeting. The tag is where
/// they find each other on the relay; the seal is the AES-GCM key the traffic
/// travels under. Neither is transmitted — both sides compute them.
///
/// WHY THIS CROSSES THE FFI RATHER THAN BEING WRITTEN IN SWIFT. A second
/// implementation that drifted would not throw, would not fail a signature, and
/// would not log: the phone and the browser would simply derive different tags,
/// sit on different mailboxes, and hear nothing from each other forever. That
/// silence is the entire failure mode, so there is one implementation and both
/// platforms call it — the same argument `core-wasm` makes about SAS words, and
/// `identity::account_channel` carries the WebCrypto vector that pins it.
///
/// The PRF salt the authenticator evaluates over is
/// `identity::ACCOUNT_CHANNEL_DOMAIN`, exposed as [`account_channel_domain`] so
/// the Swift side does not retype the one string both halves depend on.
#[uniffi::export]
pub fn account_channel(prf_secret: Vec<u8>) -> Result<Vec<u8>, FfiError> {
    let prf: [u8; 32] = prf_secret.as_slice().try_into().map_err(|_| {
        FfiError::Core(format!(
            "PRF secret must be 32 bytes, got {} — a shorter one means the \
             authenticator returned no PRF result and the caller did not check",
            prf_secret.len()
        ))
    })?;
    let (tag, seal) = pacific_core::identity::account_channel(&prf);
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(&tag);
    out.extend_from_slice(&seal);
    Ok(out)
}

/// The PRF salt — what the authenticator evaluates its secret over, and the HKDF
/// salt [`account_channel`] uses. ONE string, used twice, on both platforms: a
/// client that retyped it and differed by a byte would derive a different
/// channel and never know.
#[uniffi::export]
pub fn account_channel_domain() -> Vec<u8> {
    pacific_core::identity::ACCOUNT_CHANNEL_DOMAIN.to_vec()
}

/// The WRAP's PRF salt — what the authenticator evaluates over to derive the key
/// the identity seed is wrapped under, and the HKDF salt `wrap::wrap_key` uses.
/// A different string from the account channel's on purpose: the same PRF
/// evaluated over two salts yields two unrelated secrets, so a page holding the
/// channel seal can never open a wrap. Served from the core for the same reason
/// [`account_channel_domain`] is — one string, never retyped.
///
/// THERE IS NO HISTORY EQUIVALENT, and its absence is the design (D1, 13 Sep
/// 2026): the history is sealed under a key derived from the SEED, so the
/// authenticator has nothing to evaluate for it and 24 words alone are enough to
/// open it.
#[uniffi::export]
pub fn wrap_domain() -> Vec<u8> {
    pacific_core::wrap::WRAP_DOMAIN.to_vec()
}

/// Every address one person's records live at, from one call.
///
/// ONE CALL, NOT FIVE LABELS. A caller that derived its own would be a second
/// definition of the label set, and the failure that causes is silent: two
/// derivations of one address put two of a person's devices on different rows,
/// and the second device finds an empty address and concludes the account is new.
/// Nothing logs it, and no test anywhere catches it. So Swift asks for the set.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct Locators {
    /// Where the chain-end anchor lives. Hex, 64 chars.
    pub head: String,
    /// Where the account backup lives.
    pub history: String,
    /// Where the list of live wrap locators lives — how a seed-holder enumerates
    /// and revokes credentials whose PRF it has never held.
    pub index: String,
    /// Where the FIRST entry of the named chain generation lives, present only when
    /// a generation was supplied. Every later entry is named from inside its
    /// predecessor, so only this one is derivable — and it needs the head's
    /// `chain_gen`, because a quiet default of 0 would put a device reading
    /// generation 3 on an empty row with nothing to say so.
    pub chain: Option<String>,
    /// The arc write key's public half — what the arc records on a first write
    /// and checks every later write against. No computable relation to
    /// `identity_key()`: the arc can hold both halves of a person and be unable
    /// to tell.
    pub acct_pk: String,
    /// Where the wrap lives, present only when a PRF was supplied. The wrap is
    /// the ONE record addressed from the passkey rather than the seed, because
    /// its only caller is a device holding a passkey and no seed.
    pub wrap: Option<String>,
}

/// Derive a person's addresses. `prf` is optional: a device restoring from 24
/// words has no passkey yet and needs the others regardless. `chain_gen` is the
/// head's: fetch and open the head first, then ask for the chain.
#[uniffi::export]
pub fn locators(
    seed: Vec<u8>,
    prf: Option<Vec<u8>>,
    chain_gen: Option<u32>,
) -> Result<Locators, FfiError> {
    use pacific_core::locator::{self, Record};
    // The STORAGE root, per five-things.html §06 — the two HKDF families are
    // disjoint and everything addressed here hangs off this one. A BYO account
    // mints the root instead of deriving it; nothing below this line differs.
    let root = *locator::storage_root(&as32(&seed, "seed")?);
    let at = |r: Record| -> Result<String, FfiError> {
        Ok(hex::encode(locator::locator(&root, r)?))
    };
    Ok(Locators {
        head: at(Record::Head)?,
        history: at(Record::History)?,
        index: at(Record::Index)?,
        // Index 0: where the generation starts. Arithmetic addressing means
        // every later entry has its own derivable address too, so a caller walks
        // by index rather than by following a pointer out of this one.
        chain: chain_gen.map(|g| hex::encode(locator::chain_locator(&root, g, 0))),
        acct_pk: hex::encode(locator::acct_pk(&root)),
        wrap: match prf {
            Some(p) => Some(hex::encode(locator::wrap_locator(&as32(&p, "PRF secret")?))),
            None => None,
        },
    })
}

/// Sign a write to a locator. 64 bytes, to travel beside the body.
///
/// A CHALLENGE-RESPONSE, like every other authenticated route: fetch a nonce from
/// `/auth/challenge`, sign, and let the arc spend it. The `audience` and the
/// `nonce` are inside the signature because without them a captured write
/// verifies at another arc, and verifies again tomorrow — and since the body is
/// bound, replaying it is not forgery but ROLLBACK.
#[uniffi::export]
pub fn arc_write_sign(
    seed: Vec<u8>,
    locator: Vec<u8>,
    audience: String,
    nonce: String,
    body: Vec<u8>,
) -> Result<Vec<u8>, FfiError> {
    let root = *pacific_core::locator::storage_root(&as32(&seed, "seed")?);
    let loc = as32(&locator, "locator")?;
    let w = pacific_core::locator::ArcWrite {
        audience: &audience,
        locator: &loc,
        nonce: &nonce,
        body: &body,
    };
    Ok(pacific_core::locator::sign_write(&root, &w)?.to_vec())
}

/// The arc's half of first-write-claims, so the two sides cannot drift.
///
/// `offered` is the key the writer presents on every write; it is compared to
/// `recorded` before the signature, and the signature is checked against
/// `recorded` and never against `offered`. The refusal names which of the three
/// cases it was — for the arc's LOG, never for its reply.
#[uniffi::export]
pub fn arc_write_verify(
    recorded: Vec<u8>,
    offered: Vec<u8>,
    locator: Vec<u8>,
    audience: String,
    nonce: String,
    body: Vec<u8>,
    sig: Vec<u8>,
) -> Result<(), FfiError> {
    let recorded = as32(&recorded, "recorded arc-write key")?;
    let offered = as32(&offered, "offered arc-write key")?;
    let loc = as32(&locator, "locator")?;
    let sig: [u8; 64] = sig.as_slice().try_into().map_err(|_| {
        FfiError::Core(format!("an arc-write signature is 64 bytes, got {}", sig.len()))
    })?;
    let w = pacific_core::locator::ArcWrite {
        audience: &audience,
        locator: &loc,
        nonce: &nonce,
        body: &body,
    };
    pacific_core::locator::verify_write(&recorded, &offered, &w, &sig)
        .map_err(|r| FfiError::Core(r.to_string()))
}

/// A fresh address for the next chain entry, to be sealed inside the current one.
/// RANDOM, not derived — a derived next-tag would be computable by anyone who
/// could compute the first, which collapses the chain back into one stable
/// per-user address and hands the relay an activity profile.
#[uniffi::export]
pub fn chain_next_tag() -> Result<Vec<u8>, FfiError> {
    Ok(pacific_core::locator::next_tag()?.to_vec())
}

/// 32 bytes, or a message naming which value was the wrong length. Every caller
/// here takes secrets or addresses that are all exactly 32 bytes, and a short one
/// always means the same thing upstream: something returned nothing and nobody
/// checked.
fn as32(b: &[u8], what: &str) -> Result<[u8; 32], FfiError> {
    b.try_into()
        .map_err(|_| FfiError::Core(format!("{what} must be 32 bytes, got {}", b.len())))
}

/// A passkey PRF output as the 32 bytes every derivation takes — loud on any
/// other length, which means the authenticator returned no PRF result and the
/// caller did not check.
fn prf32(prf_secret: &[u8]) -> Result<[u8; 32], FfiError> {
    prf_secret.try_into().map_err(|_| {
        FfiError::Core(format!(
            "PRF secret must be 32 bytes, got {} — a shorter one means the \
             authenticator returned no PRF result and the caller did not check",
            prf_secret.len()
        ))
    })
}

/// One row of the local object list: the object id (hex MLS group id), kind, and
/// the group's display name ("" when unnamed — the UI derives a label from the
/// roster in that case, never from the id hex).
/// How an object's membership RECORDS and its MLS ratchet tree disagree
/// (membership-through-mls.md §10.3) — identity hex per person. `pending_removal`
/// is in flight (a leave awaiting its commit), not drift; the other two are drift.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct MembershipDivergence {
    /// In the tree, never recorded joining.
    pub unrecorded: Vec<String>,
    /// Recorded leaving, still in the tree — the Remove has not landed yet.
    pub pending_removal: Vec<String>,
    /// Recorded present, not in the tree.
    pub phantom: Vec<String>,
}

#[derive(uniffi::Record)]
pub struct ObjectRow {
    pub id: String,
    pub kind: String,
    pub name: String,
}

/// One Post, folded. Counts rather than contents for the responses: a feed row needs to
/// know there ARE comments, not what they say, and shipping every comment through the
/// boundary to render a number is how a read model gets expensive.
#[derive(Clone, Debug, uniffi::Record)]
pub struct PostRow {
    pub id: String,
    pub title: String,
    pub body: String,
    /// base64, "" = none.
    pub icon: String,
    pub banner: String,
    /// The author took it down. The words are still in the log; the surface shows the
    /// tombstone instead of them.
    pub retracted: bool,
    pub reactions: u32,
    pub comments: u32,
}

/// The draft behind every `+` — what the UI has collected when the user commits.
///
/// One flat record rather than a variant per kind, because it crosses the FFI and
/// because the fields are overwhelmingly shared. `pacific_core::mint::profile_args`
/// is the single place that decides which of these a given kind reads, and which of
/// its OWN arg names each one lands on (`name` becomes `displayName` on a Group,
/// `title` on an Event and a Project). Unify at the binder, not at the schema — the
/// arg names are on the wire and the ICD pins them.
///
/// The empty value means ABSENT throughout, including for the three fields that have
/// a wire default (`shape`, `category`, `status`). Nothing here invents a default that
/// the reducer does not already declare.
///
/// Every field is required at the boundary — uniffi records have no field defaults, so
/// Swift builds this through `NewDraft.mint()` rather than by hand.
#[derive(Clone, Debug, Default, uniffi::Record)]
pub struct MintDraft {
    /// The one field every kind has.
    pub name: String,
    /// Group / Place / Thing / Event descriptor. "" = absent.
    pub descriptor: String,
    /// Group: `individual` | `team` | `organisation` | `community`. "" = `team`.
    pub shape: String,
    /// Group: the ContactCard JSON. "" = absent.
    pub card: String,
    /// Thing: `artifact` | `skill` | `job`. "" = absent (folds as `artifact`).
    pub category: String,
    /// Event: start, and end (0 = open-ended).
    pub start_ms: i64,
    pub end_ms: i64,
    /// Event: the venue NAME on the profile. The `happens_at` edge is a separate
    /// `event.setVenue` against a Place id — not this.
    pub venue: String,
    /// Event: RFC 5545 RRULE when this repeats; "" = a one-off.
    pub recurrence: String,
    /// Event: newline-separated artist names, billing order; "" = none.
    pub lineup: String,
    /// Post: `article` | `link` | `image`. "" = absent (folds as `article`).
    pub form: String,
    /// Post: a `link` post's destination. "" = absent, and the fold refuses one
    /// on any other form.
    pub link: String,
    /// Place: what the caller ASSERTS about lat/lng. False mints UNPLACED rather than
    /// refusing, so slow indoor GPS never costs the user their typing.
    pub placed: bool,
    pub lat: f64,
    pub lng: f64,
    /// Project: `active` | `archived` | `paused`. "" = `active`.
    pub status: String,
    /// The round face and the wide wall. Empty = none. Caps are enforced AT FOLD.
    pub icon: Vec<u8>,
    pub banner: Vec<u8>,
    /// Market price — only meaningful on an ACTIVE posture.
    pub price: String,
    /// Which way the listing faces: `offers` | `wants`. Rides `thing.setPosture` (op 1),
    /// so the caller applies it after the mint — a listing without one never reaches
    /// the market.
    pub stance: String,
    /// A Place REFERENCE (object id), not a coordinate.
    pub place: String,
}

/// Which slot of `MintDraft` a field binds to. Mirrors `pacific_core::mint::MintKey`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum MintKey {
    Name,
    Descriptor,
    Shape,
    Category,
    Start,
    Fix,
    Icon,
    Banner,
    Price,
    /// `offers` | `wants` — which way the listing faces.
    Stance,
    Recurrence,
    Place,
    /// Post: `article` | `link` | `image` — which of the three shapes it is.
    Form,
    /// Post: a `link` post's destination.
    Link,
}

/// What control the field wants. `choices` is non-empty exactly when this is `Choice`,
/// and it carries the reducer's OWN wire vocabulary — the UI offers those strings and no
/// others, so a picker cannot mint a value the reducer will reject.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum MintInput {
    Line,
    Multiline,
    Choice,
    Date,
    Fix,
    Icon,
    Banner,
    Money,
    Recurrence,
    Place,
}

/// One field a kind requires to come into existence.
///
/// The TYPE declares these; the UI is built from them. Before this the UI decided, in a
/// hand-written switch, and the reducers' `req_text`/`req_int` calls decided separately —
/// two statements of one fact, and they had drifted in four places at once.
#[derive(Clone, Debug, uniffi::Record)]
pub struct MintFieldRow {
    pub key: MintKey,
    /// The field's name as the person filling it reads it.
    pub label: String,
    pub input: MintInput,
    /// Empty unless `input == Choice`.
    pub choices: Vec<String>,
    /// The mint cannot commit without it.
    pub required: bool,
    /// The field is SHOWN but does not yet persist — see `Carriage::Deferred`. The UI must
    /// present it honestly rather than as a live control that saves.
    pub deferred: bool,
}

/// What a kind needs to be created, in the order it should be asked. Empty for a kind
/// that is not minted from a draft (`contact` is formed by pairing, `conversation` by
/// consuming prekeys).
///
/// This is a pure lookup over a static table — no node, no state, no I/O — so the UI can
/// call it while building a view.
#[uniffi::export]
pub fn mint_fields(kind: String) -> Vec<MintFieldRow> {
    let Some(k) = pacific_core::mint::kind_from_name(&kind) else {
        return Vec::new();
    };
    pacific_core::mint::fields(k)
        .iter()
        .map(|f| {
            use pacific_core::mint::{MintInput as CoreInput, MintKey as CoreKey};
            MintFieldRow {
                key: match f.key {
                    CoreKey::Name => MintKey::Name,
                    CoreKey::Descriptor => MintKey::Descriptor,
                    CoreKey::Shape => MintKey::Shape,
                    CoreKey::Category => MintKey::Category,
                    CoreKey::Start => MintKey::Start,
                    CoreKey::Fix => MintKey::Fix,
                    CoreKey::Icon => MintKey::Icon,
                    CoreKey::Banner => MintKey::Banner,
                    CoreKey::Price => MintKey::Price,
                    CoreKey::Stance => MintKey::Stance,
                    CoreKey::Recurrence => MintKey::Recurrence,
                    CoreKey::Place => MintKey::Place,
                    CoreKey::Form => MintKey::Form,
                    CoreKey::Link => MintKey::Link,
                },
                label: f.label.to_string(),
                input: match f.input {
                    CoreInput::Line => MintInput::Line,
                    CoreInput::Multiline => MintInput::Multiline,
                    CoreInput::Choice(_) => MintInput::Choice,
                    CoreInput::Date => MintInput::Date,
                    CoreInput::Fix => MintInput::Fix,
                    CoreInput::Icon => MintInput::Icon,
                    CoreInput::Banner => MintInput::Banner,
                    CoreInput::Money => MintInput::Money,
                    CoreInput::Recurrence => MintInput::Recurrence,
                    CoreInput::Place => MintInput::Place,
                },
                choices: match f.input {
                    CoreInput::Choice(v) => v.iter().map(|s| s.to_string()).collect(),
                    _ => Vec::new(),
                },
                required: f.required,
                deferred: f.carriage == pacific_core::mint::Carriage::Deferred,
            }
        })
        .collect()
}

impl From<MintDraft> for pacific_core::mint::MintDraft {
    fn from(d: MintDraft) -> Self {
        Self {
            name: d.name,
            descriptor: d.descriptor,
            shape: d.shape,
            card: d.card,
            category: d.category,
            start_ms: d.start_ms,
            end_ms: d.end_ms,
            venue: d.venue,
            recurrence: d.recurrence,
            lineup: d.lineup,
            form: d.form,
            link: d.link,
            placed: d.placed,
            lat: d.lat,
            lng: d.lng,
            status: d.status,
            icon: d.icon,
            banner: d.banner,
            price: d.price,
            stance: d.stance,
            place: d.place,
        }
    }
}

/// One listing DISCOVERED from someone else — the buy side of the market.
///
/// Deliberately not a `ThingRow`: I do not own this object and cannot edit it. What I hold
/// is the announcement, plus the two facts that make a stranger's offer trustworthy —
/// `origin` (whose it is) and `via`/`hops` (who vouched for the path it took to me).
/// Oriented pixels plus what the platform reported, handed over for attachment.
///
/// PIXELS, deliberately — not a HEIC, not a JPEG, not a file. The rule is that
/// nothing raw reaches a delta, and the cheapest way to make that true is to
/// give the caller no way to express it: this boundary accepts decoded pixels
/// only, so a picker's original bytes cannot cross it even by mistake. The
/// caller decodes and applies orientation (`CGImageSource` / a track's
/// `preferredTransform`); the core downsamples, encodes and records.
#[derive(Clone, Debug, uniffi::Record)]
pub struct MediaInput {
    /// Tightly packed RGBA, `width * height * 4` bytes, ALREADY UPRIGHT.
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// What it was before us (`image/heic`, `image/jpeg`…) — provenance only.
    pub source_mime: String,
    /// Size of the original the user picked, for the shrink report.
    pub source_bytes: u64,
    /// iOS 17+ stills carry an HDR gain map; we tone-map to SDR and record that.
    pub has_gain_map: bool,
    /// Portrait mode ships a subject matte. Noted even though nothing reads it yet.
    pub has_depth_matte: bool,
    /// True when the frames came from a Live Photo's motion half.
    pub live_photo: bool,
}

/// Preprocess an attachment for a chat message. One place, so every send path
/// (group, room, DM, reply) gets the same budget and the same refusals.
fn attach(input: &MediaInput) -> Result<pacific_media::MediaRef, FfiError> {
    let facts = pacific_media::SourceFacts {
        source_mime: input.source_mime.clone(),
        source_bytes: input.source_bytes as usize,
        width: input.width,
        height: input.height,
        duration_ms: 0,
        orientation: pacific_media::Orientation::Up,
        // The boundary asserts it, because the caller is the only party that can
        // know — and an unoriented attachment is refused rather than guessed at.
        orientation_applied: true,
        has_gain_map: input.has_gain_map,
        has_depth_matte: input.has_depth_matte,
        live_photo: input.live_photo,
        has_audio: false,
    };
    let out = pacific_media::preprocess(
        &input.rgba,
        input.width,
        input.height,
        pacific_media::Slot::Message,
        &facts,
    )
    .map_err(|e| FfiError::Core(format!("attachment refused: {e}")))?;
    Ok(out.media)
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct ListingRow {
    /// Identity pubkey (hex) of whoever created the listing.
    pub origin: String,
    /// The origin's Thing object id. `(origin, thing_id)` is its network-wide identity.
    pub thing_id: String,
    /// has | wants | offers | buying | selling
    pub posture: String,
    pub title: String,
    pub descriptor: String,
    /// "" when the posture is standing (only buying/selling carry a price).
    pub price: String,
    /// Epoch ms, 0 = standing.
    pub deadline: i64,
    /// Coarse geohash prefix, "" when geography sits out of routing.
    pub area: String,
    /// Forwards taken to reach me: 0 = straight from the origin, 1 = a friend passed it on.
    pub hops: u32,
    /// Identity pubkey (hex) of whoever told ME — the origin itself at 0 hops.
    pub via: String,
    /// The origin's photo for this listing, carried unchanged through every forward.
    /// "" when they listed without one.
    pub photo: String,
    pub photo_mime: String,
}

/// One THING the market list binds to: the object id, its name/descriptor, and the
/// posture it stands in (`""` = tracked but taking no market position, which is a real
/// state, not a missing value). `price` is present only on an ACTIVE posture.
#[derive(uniffi::Record)]
pub struct ThingRow {
    pub id: String,
    pub name: String,
    pub descriptor: String,
    /// artifact | skill | job — which market face this thing belongs to.
    pub category: String,
    /// "" | has | wants | offers | buying | selling
    pub posture: String,
    pub price: String,
    /// Epoch ms, 0 = standing (no deadline).
    pub deadline: i64,
    /// "private" | "network" — how far this listing may travel. Also the identity-disclosure
    /// control: network means the origin signature travels with it.
    pub reach: String,
    /// Coarse geohash prefix, "" when the thing is not a local good.
    pub area: String,
    /// The listing's FACE — one still, base64 ("" = none; the card renders stripes).
    pub photo: String,
    pub photo_mime: String,
}

/// One row of the hops-aware place listing: a Place I hold (`holder == ""`, hops 0)
/// or one a connection answered a discovery request with (`holder` = their identity
/// pk hex, hops 1|2). A remote row is a REFERENCE — enough to render and to adopt,
/// never the object itself.
#[derive(uniffi::Record)]
pub struct DiscoveredRow {
    /// Object id hex — local for a held row, the HOLDER's for a remote one.
    pub id: String,
    pub kind: String,
    pub name: String,
    pub descriptor: String,
    /// False when the row carries no point; `lat`/`lng` are meaningless then.
    pub placed: bool,
    pub lat: f64,
    pub lng: f64,
    /// "" = held locally; otherwise the holder's identity pk, hex.
    pub holder: String,
    /// Handshakes between me and whoever holds it. 0 = mine.
    pub hops: u32,
}

/// One note published at a Place.
#[derive(uniffi::Record)]
pub struct PlacePostRow {
    /// The author's identity pk, hex (MLS-authenticated, never on the wire).
    pub author: String,
    /// The author's note id — re-publishing the same note replaced this row.
    pub note: String,
    pub title: String,
    pub text: String,
    /// Author-clock ms.
    pub at: i64,
}

/// One PLACE the LIFE places list binds to: the object id, its name/descriptor, and
/// where it is.
///
/// Coordinates cross the FFI as DEGREES because that is what CoreLocation and MapKit
/// speak; the core stores e7 fixed-point integers so log replay folds deterministically
/// (see `geo.rs`), and the conversion happens at this boundary and nowhere else.
///
/// `placed` is the honest witness that a Place may have no coordinate. Without it, an
/// unplaced Place is indistinguishable from one at (0, 0) — a real point in the Gulf of
/// Guinea — so the caller must be told which it is rather than inferring it from zeros.
#[derive(uniffi::Record)]
pub struct PlaceRow {
    pub id: String,
    pub name: String,
    pub descriptor: String,
    /// False when the Place has no location facet yet, or carries a live STREAM source
    /// (whose positions are out-of-band and have no persisted point). `lat`/`lng` are
    /// meaningless when this is false.
    pub placed: bool,
    pub lat: f64,
    pub lng: f64,
    /// The location's own label/address when the point carried one; "" otherwise.
    pub label: String,
    pub address: String,
    /// Whose land this sits on, as far as anyone has established — a CLAIM with its
    /// provenance, never a verified fact. All "" / 0 / "unknown" when nobody has looked,
    /// which is a real and common state.
    ///
    /// `land_parcel` is the HMLR title number (or INSPIRE id). `land_proprietor` is "" when
    /// the source cannot name one — INSPIRE is geometry, and no free dataset covers a
    /// private individual. `land_as_of` is the DATASET vintage in unix ms, not the lookup
    /// time: these are monthly files and ownership moves. `land_permission` is
    /// "unknown" | "asked" | "granted" | "refused" — the part no register can answer.
    pub land_parcel: String,
    pub land_proprietor: String,
    pub land_company_no: String,
    /// "" | inspire | ccod | ocod | scotlis | manual
    pub land_source: String,
    pub land_as_of: i64,
    pub land_permission: String,
    /// The doorbell as hex, "" when none is fitted. This is what goes on the sign — a
    /// per-place capability, not anyone's identity. Public by construction; it buys
    /// REVOCATION (rotate to retire a printed sign, or to cut off an ex-member), not
    /// secrecy.
    pub doorbell: String,
    /// "private" | "public". PUBLIC means AUTO-ACCEPTED — anyone holding the Place's QR
    /// is admitted by the host automatically, with no per-person approval. It does NOT
    /// mean world-readable: joiners become members of the group, they do not read it
    /// from outside.
    pub access: String,
    /// "private" | "connections" | "network" — how far the fact of this Place's
    /// EXISTENCE may travel (what discovery answers carry, and how far they
    /// relay). NOT `access`: joinable and discoverable are different dials.
    pub visibility: String,
}

/// One SYSTEM this device holds — an external source as a GroupObject (kind 21).
/// `connector` is what the app dispatches on ("music.ra", "art.artrabbit").
#[derive(uniffi::Record)]
pub struct SystemRow {
    pub id: String,
    pub name: String,
    pub connector: String,
    pub scope: String,
    /// Items still on offer (withdrawn tombstones excluded).
    pub live_items: i64,
    pub total_items: i64,
}

/// One hydrated item — the source's own JSON, folded and provenanced.
/// `payload` is VERBATIM: the System that wrote it owns its shape, which is what lets
/// a new connector ship without a core change.
#[derive(uniffi::Record)]
pub struct HydratedItemRow {
    pub key: String,
    pub payload: String,
    pub fetched_at: i64,
    pub rev: i64,
    pub withdrawn: bool,
}

/// One item a connector is offering, on its way IN. `rev` is the source's own
/// revision — a re-offer at the same rev is skipped rather than written.
#[derive(uniffi::Record)]
pub struct HydrateItem {
    pub key: String,
    pub payload: String,
    pub fetched_at: i64,
    pub rev: i64,
    pub withdrawn: bool,
}

/// One EVENT the EVENTS page binds to: the object id, when/where, and its ticket
/// listing folded to display shape. `listing` is the stable public listing key
/// (sha256(group_id ‖ owner) — what the box office and buyers reference, never the
/// group id). `sold`/`over_capacity` mirror the ledger fold.
#[derive(uniffi::Record)]
pub struct EventRow {
    pub id: String,
    pub title: String,
    pub descriptor: String,
    pub start_ms: i64,
    /// 0 = no stated end.
    pub end_ms: i64,
    pub venue: String,
    /// The venue as a Place GroupObject reference ("" = none) — the `happens_at`
    /// edge the graph projector folds. `venue` above stays the flyer's free text.
    pub venue_place: String,
    pub venue_place_name: String,
    pub venue_at: i64,
    pub placed: bool,
    pub lat: f64,
    pub lng: f64,
    /// Whether a ticket listing exists at all.
    pub listed: bool,
    pub price_cents: i64,
    pub currency: String,
    pub capacity: i64,
    pub open: bool,
    pub sold: i64,
    pub over_capacity: bool,
    pub listing: String,
    /// The poster still, base64 ("" = none — the flyer-stripes placeholder state).
    /// Full media (photos, clip) rides `event_media`, not the list row.
    pub banner: String,
    pub banner_mime: String,
    pub photo_count: i64,
    pub has_clip: bool,
    /// The lineup — artist names, billing order; empty = no lineup.
    pub lineup: Vec<String>,
    /// The RFC 5545 RRULE when this night repeats, canonicalised by the core on
    /// write ("FREQ=WEEKLY;BYDAY=TH"); empty = it happens once.
    ///
    /// The rule crosses, NOT its occurrences. Expansion is relative to now and to
    /// a window, so a stored next-occurrence would be stale the moment it was
    /// written — the reader asks `recurrence_summary` / `recurrence_occurrences`
    /// for what it needs, which are already here and already tested.
    pub recurrence: String,
}

/// One event photo crossing the FFI.
#[derive(uniffi::Record)]
pub struct EventPhotoRow {
    pub data: String,
    pub mime: String,
}

/// The full media facet of one event — the detail page's read.
#[derive(uniffi::Record)]
pub struct EventMediaRow {
    pub banner: String,
    pub banner_mime: String,
    pub photos: Vec<EventPhotoRow>,
    pub clip: String,
    pub clip_mime: String,
}

/// One delivered ticket in the buyer's wallet, ready to present: the verbatim signed
/// core + sig, the QR payload to render, display fields parsed from the core, and
/// whether the signature verifies against the delivering peer's identity.
#[derive(uniffi::Record)]
pub struct WalletTicketRow {
    pub ticket: String,
    pub listing: String,
    pub core: String,
    pub sig: String,
    pub qr: String,
    pub title: String,
    pub start_ms: i64,
    pub verified: bool,
    /// Epoch ms the door told the holder they were let in; 0 = not admitted.
    /// The buyer is not a member of the Event group, so this receipt — carried on
    /// the pairwise channel — is the ONLY way their device can know.
    pub admitted_ms: i64,
}

/// One pending ticket request folded off a connection channel (the seller-side read).
#[derive(uniffi::Record)]
pub struct TicketRequestRow {
    pub pi: String,
    pub listing: String,
    pub qty: i64,
    /// The requesting author's identity pk, hex — MUST equal the paying buyer.
    pub author: String,
}

/// A human line for an RFC 5545 RRULE — "Every Thursday", "Every 2 weeks". Empty for
/// a rule that does not parse, so a caller renders nothing rather than a lie.
#[uniffi::export]
pub fn recurrence_summary(rrule: String) -> String {
    pacific_core::recurrence::Recurrence::parse(&rrule)
        .map(|r| r.summary())
        .unwrap_or_default()
}

/// The next `limit` start instants of a repeating event at or after `from` (unix ms).
/// A one-off yields just its own start, so callers need no special case.
#[uniffi::export]
pub fn recurrence_occurrences(
    rrule: String,
    start_ms: i64,
    from: i64,
    to: i64,
    limit: u32,
) -> Vec<i64> {
    match pacific_core::recurrence::Recurrence::parse(&rrule) {
        Ok(r) => r.occurrences(start_ms, from, to, limit as usize),
        Err(_) => {
            if start_ms >= from && start_ms <= to {
                vec![start_ms]
            } else {
                Vec::new()
            }
        }
    }
}

/// One invitation folded off a connection channel — the row the thread's invite card
/// renders. SELF-DESCRIBING: the guest does not hold the Event object, so everything
/// shown here comes from the leg itself.
#[derive(uniffi::Record)]
pub struct InviteRow {
    /// The Event object id — the join back to the live object, for whoever holds it.
    pub event: String,
    pub title: String,
    pub start_ms: i64,
    pub venue: String,
    pub note: String,
    pub at: i64,
    /// The inviter's identity pk, hex. Compare with your own to tell sent from received.
    pub author: String,
    /// The answer, when one has been given: "going" | "not_going" | "" (unanswered).
    pub reply: String,
    /// Unix ms of the answer, 0 when unanswered.
    pub replied_at: i64,
}

/// What the door shows for one scanned QR, joined in core: signature check against
/// the fold's signer + ledger membership + redemption state.
/// `verdict`: "bad_qr" | "bad_sig" | "unknown" (amber — not in this device's ledger)
/// | "valid" | "redeemed" | "double" (redeemed by two different doors).
#[derive(uniffi::Record)]
pub struct DoorCheckRow {
    pub verdict: String,
    pub ticket: String,
    pub title: String,
    pub buyer: String,
}

/// Project a folded `PlaceState` onto the wire row. The ONE place e7 becomes degrees.
/// `Option<GeoPoint>` → `(placed, lat°, lng°)` — THE e7→degrees crossing, and the
/// one statement of the 0.0-when-unplaced rule (see `PlaceRow`'s doc for why the
/// `placed` witness exists at all). Every row type that carries a point converts
/// through here.
fn degrees(p: Option<&pacific_core::geo::GeoPoint>) -> (bool, f64, f64) {
    match p {
        Some(p) => (true, p.lat(), p.lng()),
        None => (false, 0.0, 0.0),
    }
}

fn place_row(id: String, st: pacific_core::place::PlaceState) -> PlaceRow {
    // `point()` is None both when there is no location at all and when the source is a
    // live STREAM — neither has a persisted coordinate, and both are honestly "unplaced"
    // to a caller that wants somewhere to pin. Taken as owned values up front so the
    // borrow ends before the strings below move out of `st`.
    let land = st.land.clone();
    let (placed, lat, lng) = degrees(st.point());
    let (label, address) = st
        .point()
        .map(|p| (p.label.clone(), p.address.clone()))
        .unwrap_or_default();
    PlaceRow {
        id,
        name: st.name,
        descriptor: st.descriptor,
        placed,
        lat,
        lng,
        label,
        address,
        access: st.access.as_str().to_string(),
        visibility: st.visibility.as_str().to_string(),
        doorbell: st.doorbell.map(hex::encode).unwrap_or_default(),
        land_parcel: land.as_ref().map(|l| l.parcel.clone()).unwrap_or_default(),
        land_proprietor: land
            .as_ref()
            .map(|l| l.proprietor.clone())
            .unwrap_or_default(),
        land_company_no: land
            .as_ref()
            .map(|l| l.company_no.clone())
            .unwrap_or_default(),
        land_source: land
            .as_ref()
            .map(|l| l.source.as_str().to_string())
            .unwrap_or_default(),
        land_as_of: land.as_ref().map(|l| l.as_of).unwrap_or(0),
        land_permission: land
            .as_ref()
            .map(|l| l.permission.as_str().to_string())
            .unwrap_or_else(|| "unknown".to_string()),
    }
}

/// One 1:1 connection: the peer's stable space id, a human display name (from the
/// directory projection, else a short prefix of the space id), and whether the
/// double-opt-in is complete. This is the address the DM methods take.
#[derive(uniffi::Record)]
pub struct ConnectionRow {
    pub peer_id: String,
    pub display_name: String,
    pub connected: bool,
}

/// One emoji reaction group on a message: the emoji, how many people reacted with
/// it, and whether THIS device is one of them (drives the "selected" pill state,
/// exactly like Signal's CVReactionCountsView highlighting your own reaction).
#[derive(uniffi::Record)]
pub struct ChatReaction {
    pub emoji: String,
    pub count: u32,
    pub mine: bool,
}

/// One message in the STRUCTURED chat projection — the folded residue of real
/// forum.post/forum.react Deltas, not UI state. The UI binds bubbles, reaction
/// pills, reply quotes and the time-sent stamp to these fields directly (no
/// string parsing). A message is addressed by `(author, gen)`: `author` is the
/// author's 64-char identity hex and `gen` its Lamport generation — the same
/// `MsgRef` the core uses, so reacting/replying just echoes these two back.
#[derive(uniffi::Record)]
pub struct ChatMsg {
    pub author: String, // author identity, 64-char hex (the MsgRef key)
    pub gen: u64,       // Lamport generation (the MsgRef key)
    pub text: String,
    pub ts: u64,                 // display timestamp, unix millis (0 if absent)
    pub mine: bool,              // authored by THIS device
    pub reply_to_author: String, // "" when not a reply, else the parent's author hex
    pub reply_to_gen: u64,       // 0 when not a reply
    pub reply_preview: String,   // parent's text, resolved locally ("" if unknown)
    pub reactions: Vec<ChatReaction>,
    /// WhatsApp-style delivery/read code, set only on OUR OWN messages (`mine`):
    /// 0 = none (an incoming message), 1 = sent, 2 = delivered to every recipient,
    /// 3 = read by every recipient. The UI renders ✓ / ✓✓ / ✓✓-blue from this.
    pub receipt: u8,
    /// Live vote tallies (the Reddit half of a Forum post). The score the UI
    /// shows is `up - down`, derived here and everywhere — never stored.
    pub up: u32,
    pub down: u32,
    /// THIS device's vote: 1 up, -1 down, 0 none — drives the arrows' lit state.
    pub my_vote: i8,
}

/// One row of a Forum object's THREADED (Reddit-style) projection: the message
/// plus its nesting `depth` (0 = a top-level post) and `reply_count` (how many
/// posts are nested beneath it — its whole subtree, for the collapse and "N
/// replies" affordances). Rows arrive PRE-ORDER — a post immediately precedes its
/// subtree — so the UI indents by `depth` and collapses a subtree by skipping the
/// next `reply_count` rows. The tree twin of `ChatMsg`/`object_messages`.
#[derive(uniffi::Record)]
pub struct ForumThreadRow {
    pub post: ChatMsg,
    pub depth: u32,
    pub reply_count: u32,
}

/// One live timeline row of a Project's Gantt — the STRUCTURED projection the Work
/// tab binds to (replacing the pre-formatted `project_board` string dump, so the UI
/// never parses strings). Mirrors `pacific_core::project::ItemView`; honest-absent
/// status is the empty string (UI shows "—", never a fabricated "open").
#[derive(uniffi::Record)]
pub struct ProjectItemRow {
    pub id: String,
    pub kind: String, // "action" | "event" | "gate"
    pub title: String,
    pub status: String, // "" when uncontributed, else open|in_progress|done|cancelled
    pub blocked: bool,  // DERIVED from dependency edges, never stored
    pub assignees: Vec<String>, // hex member ids
    pub at: Option<i64>,
    pub end_date: Option<i64>,
    pub fields: std::collections::HashMap<String, String>, // custom LWW annotations (setItemField)
}

/// One dependency edge between two timeline items — the list the UI shows and can
/// remove. `from` must be done for `to` to unblock (the source of the derived
/// `ProjectItemRow.blocked`).
#[derive(uniffi::Record)]
pub struct DependencyRow {
    pub edge_id: String,
    pub from: String,
    pub to: String,
    pub kind: String,
}

/// One deliverable (a child-Project subscription) shown collapsed under the Gantt.
#[derive(uniffi::Record)]
pub struct DeliverableRow {
    pub sub_id: String,
    pub target: String,
    pub disclosure: String, // existence | summary | full
    pub origin_item: Option<String>,
    pub last_seen: Option<i64>,
}

/// The Project card header — title/status + the DERIVED piece count (count of
/// child-Project deliverables), replacing the mockup's stored `bodyCount`.
#[derive(uniffi::Record)]
pub struct ProjectHeaderRow {
    pub title: String,
    pub status: String, // active | archived | paused
    pub piece_count: u32,
}

/// A participant (member) + their Role in the Project (Objectives tab).
#[derive(uniffi::Record)]
pub struct ParticipantRow {
    pub member: String, // 64-char identity hex
    pub role: String,   // viewer | contributor | maintainer | owner
}

/// An external stakeholder — a non-member party the project touches (gets a swimlane).
#[derive(uniffi::Record)]
pub struct StakeholderRow {
    pub id: String,
    pub name: String,
    pub note: String,
}

/// A key location relevant to the project.
#[derive(uniffi::Record)]
pub struct LocationRow {
    pub id: String,
    pub name: String,
}

/// A KPI the objective is measured against.
#[derive(uniffi::Record)]
pub struct KpiRow {
    pub id: String,
    pub label: String,
    pub target: String,
}

/// The Objectives tab (Tab 1) — the human-authored spine of the project.
#[derive(uniffi::Record)]
pub struct ObjectivesRow {
    pub headline: String,
    pub goal: String,
    pub participants: Vec<ParticipantRow>,
    pub stakeholders: Vec<StakeholderRow>,
    pub locations: Vec<LocationRow>,
    pub kpis: Vec<KpiRow>,
}

/// A labelled contact value (email/phone/url) — the vCard-shaped card fields.
#[derive(uniffi::Record, Clone, Debug, Default)]
pub struct LabeledRow {
    pub label: String,
    pub value: String,
}

/// A member's Group role, projected onto the LIVE MLS roster (never the log).
#[derive(uniffi::Record)]
pub struct GroupRoleRow {
    pub member: String, // space1<hex> of the roster member
    pub role: String,   // owner | admin | member | viewer | guest
}

/// A credential HANDLE — id/kind/label only; the secret NEVER crosses the FFI.
#[derive(uniffi::Record)]
pub struct CredentialRow {
    pub id: String,
    pub kind: String, // apiKey | oauthToken | bearerToken | basicAuth | sshKey | custom
    pub label: String,
}

/// This group's half of a Group↔Group edge (federation): the peer group by object
/// id, how it relates to us, and the tether that carries the bilateral channel.
#[derive(uniffi::Record)]
pub struct AffiliationRow {
    pub peer: String,   // the peer group's object id (hex)
    pub rel: String,    // parent | child | peer
    pub name: String,   // the peer's display name as known at authoring
    pub tether: String, // group-tether object id ("" when none)
    pub at: i64,        // event unix ms
}

/// The folded Group read-model — the reusable identity record the directory UI
/// binds to. `presence`/`can_sync` say whether this contact is reachable over MLS.
#[derive(uniffi::Record)]
pub struct GroupRow {
    pub display_name: String,
    pub shape: String,    // individual | team | organisation | community
    pub presence: String, // "onPlatform:<space>" | "offPlatform:<hint>" | "unknown"
    pub can_sync: bool,
    pub org: String,
    pub title: String,
    pub note: String,
    pub photo: String, // base64 (may be empty)
    pub emails: Vec<LabeledRow>,
    pub phones: Vec<LabeledRow>,
    pub urls: Vec<LabeledRow>,
    pub tags: Vec<String>,
    pub roles: Vec<GroupRoleRow>,
    pub credentials: Vec<CredentialRow>,
    /// Group↔Group edges (this group's half) — a chapter lists its federation here.
    pub affiliations: Vec<AffiliationRow>,
    /// The appointments this body has made. An office with no row is VACANT.
    pub offices: Vec<OfficeRow>,
    /// The avatar slots, same shape as `ProfileCard` — an identity record renders
    /// its face the same way a connection's does (clip → photo → colour).
    pub photo_mime: String,
    pub clip: String,
    pub clip_mime: String,
    pub captured_at: u64,
    pub avatar_color: String,
    /// The COVER banner (base64 + mime, "" = none): a still, an animated gif, or a
    /// short mp4 — the group page's wall, beside the avatar's face.
    pub cover: String,
    pub cover_mime: String,
}

/// One row of the LIFE ▸ GROUPS listing: a Group identity record, folded — the
/// whole list in one FFI call, no per-row `group_view` round trips.
#[derive(uniffi::Record)]
pub struct GroupListRow {
    pub id: String,
    /// The folded displayName; "" when the profile was never set (the UI derives
    /// a label from the roster then, never from id hex).
    pub name: String,
    pub shape: String, // individual | team | organisation
    /// MLS roster size — membership's one source (the ratchet tree).
    pub members: u32,
    /// How many rooms this group hosts (the folded forum-edge count).
    pub forums: u32,
    /// The avatar's still + colour slots (clip → photo → colour, like a connection).
    pub photo: String,
    pub photo_mime: String,
    pub avatar_color: String,
    pub can_sync: bool,
    /// TRUE for a Space — a body you belong to; FALSE for an identity RECORD, the
    /// card this device keeps for one contact. Decided by the CORE (`group_rows`),
    /// never re-derived here or in the app: the two share the same object
    /// machinery, and every consumer that guessed got it wrong differently.
    pub is_space: bool,
}

/// One room a Group hosts, folded from the GROUP's log. `held` says whether THIS
/// device carries the room's MLS group — the difference between "tap to open"
/// and "a room you know of but are not in yet".
#[derive(uniffi::Record)]
pub struct GroupForumRow {
    pub id: String,
    pub name: String,
    /// When the room was attached (event unix ms).
    pub at: i64,
    pub held: bool,
}

/// One office and who holds it — the appointment, folded off the Group's own log
/// rather than kept beside it. `status`: "pending" | "accepted".
#[derive(uniffi::Record)]
pub struct OfficeRow {
    pub office: String,
    /// Holder's identity pubkey, hex.
    pub holder: String,
    pub status: String,
    pub at: i64,
}

/// One forum→host edge, the REVERSE of `group_forums`: which Group hosts this
/// room. The CHAT list joins on `forum` so a hosted room renders as
/// "Group / Room" instead of posing as a free-standing chat.
#[derive(uniffi::Record)]
pub struct ForumHostRow {
    /// The room's object id.
    pub forum: String,
    /// The hosting Group's object id.
    pub group: String,
    /// The hosting Group's folded displayName ("" when its profile was never set).
    pub group_name: String,
}

/// One constituent Room a Channel (forum) hosts, folded from the PARENT forum's
/// log — `GroupForumRow`'s contract one level down. `held` says whether THIS
/// device carries the room's MLS group: the difference between a tab that opens
/// and a tab you know of but are not in yet.
#[derive(uniffi::Record)]
pub struct ForumRoomRow {
    pub id: String,
    pub name: String,
    /// When the room was attached (event unix ms) — the tab order.
    pub at: i64,
    pub held: bool,
}

/// One room→host edge, the REVERSE of `forum_rooms`: which Channel hosts this
/// room. The CHAT list joins on `room` so a channel's room renders as
/// "Channel / Room" instead of posing as a free-standing chat.
#[derive(uniffi::Record)]
pub struct RoomHostRow {
    /// The room's object id.
    pub room: String,
    /// The hosting Channel's (forum's) object id.
    pub forum: String,
    /// The hosting Channel's live GroupContext name ("" when unnamed).
    pub forum_name: String,
}

fn labeled_rows(v: Vec<pacific_core::group::Labeled>) -> Vec<LabeledRow> {
    v.into_iter()
        .map(|l| LabeledRow {
            label: l.label,
            value: l.value,
        })
        .collect()
}

fn labeled_core(v: Vec<LabeledRow>) -> Vec<pacific_core::group::Labeled> {
    v.into_iter()
        .map(|l| pacific_core::group::Labeled {
            label: l.label,
            value: l.value,
        })
        .collect()
}

/// The editable contact card — everything a person can say about themselves, in the
/// structured shape the UI binds to (never a JSON blob the Swift side has to parse).
///
/// THE AVATAR IS TWO SLOTS. `photo` is the still; `clip` is the parallel "last seen"
/// pinhole capture. A renderer resolves clip → photo → `avatar_color` + initials, so
/// motion is used *in lieu of or in parallel to* the still rather than replacing it.
/// `captured_at` is when the media was TAKEN (not when it arrived) — the freshness
/// "last seen" is actually about.
#[derive(uniffi::Record, Clone, Debug, Default)]
pub struct ProfileCard {
    pub org: String,
    pub title: String,
    pub note: String,
    pub emails: Vec<LabeledRow>,
    pub phones: Vec<LabeledRow>,
    pub urls: Vec<LabeledRow>,
    pub tags: Vec<String>,
    /// base64 still image ("" when none).
    pub photo: String,
    /// MIME of `photo`, e.g. "image/jpeg" ("" when no photo).
    pub photo_mime: String,
    /// base64 "last seen" motion clip ("" when none).
    pub clip: String,
    /// MIME of `clip`, e.g. "video/mp4" ("" when no clip).
    pub clip_mime: String,
    /// When the avatar media was captured (unix secs; 0 = unknown).
    pub captured_at: u64,
    /// "RRGGBB" block colour behind the initials when there is no photo or clip.
    pub avatar_color: String,
    /// Where you are, as a WORD ("London") — a display string, never a coordinate.
    pub city: String,
}

/// A profile as published or held: the identity essentials plus the card.
/// `gen` is the publisher's monotone revision — 0 for your own local profile
/// (which has no per-connection gen until it is published) and for a peer who has
/// never published.
#[derive(uniffi::Record, Clone, Debug)]
pub struct ProfileRow {
    pub display_name: String,
    pub shape: String, // individual | team | organisation
    pub card: ProfileCard,
    pub gen: u64,
}

fn profile_card_row(c: pacific_core::group::ContactCard) -> ProfileCard {
    ProfileCard {
        org: c.org,
        title: c.title,
        note: c.note,
        emails: labeled_rows(c.emails),
        phones: labeled_rows(c.phones),
        urls: labeled_rows(c.urls),
        tags: c.tags,
        photo: c.photo,
        photo_mime: c.photo_mime,
        clip: c.clip,
        clip_mime: c.clip_mime,
        captured_at: c.captured_at,
        avatar_color: c.avatar_color,
        city: c.city,
    }
}

fn profile_card_core(c: ProfileCard) -> pacific_core::group::ContactCard {
    pacific_core::group::ContactCard {
        org: c.org,
        title: c.title,
        note: c.note,
        emails: labeled_core(c.emails),
        phones: labeled_core(c.phones),
        urls: labeled_core(c.urls),
        tags: c.tags,
        photo: c.photo,
        photo_mime: c.photo_mime,
        clip: c.clip,
        clip_mime: c.clip_mime,
        captured_at: c.captured_at,
        avatar_color: c.avatar_color,
        city: c.city,
    }
}

fn group_row(v: pacific_core::group::GroupView) -> GroupRow {
    use pacific_core::group::Presence;
    let presence = match &v.presence {
        Presence::OnPlatform(sid) => format!("onPlatform:{sid}"),
        Presence::OffPlatform(hint) => format!("offPlatform:{}", hint.clone().unwrap_or_default()),
        Presence::Unknown => "unknown".to_string(),
    };
    GroupRow {
        display_name: v.display_name,
        shape: v.shape.as_str().to_string(),
        presence,
        can_sync: v.can_sync,
        org: v.card.org,
        title: v.card.title,
        note: v.card.note,
        photo: v.card.photo,
        photo_mime: v.card.photo_mime,
        clip: v.card.clip,
        clip_mime: v.card.clip_mime,
        captured_at: v.card.captured_at,
        avatar_color: v.card.avatar_color,
        cover: v.cover,
        cover_mime: v.cover_mime,
        emails: labeled_rows(v.card.emails),
        phones: labeled_rows(v.card.phones),
        urls: labeled_rows(v.card.urls),
        tags: v.card.tags,
        roles: v
            .roles
            .into_iter()
            .map(|(m, r)| GroupRoleRow {
                member: space_id_of(&m),
                role: r.as_str().to_string(),
            })
            .collect(),
        credentials: v
            .credentials
            .into_iter()
            .map(|(id, meta)| CredentialRow {
                id,
                kind: meta.kind.as_str().to_string(),
                label: meta.label,
            })
            .collect(),
        offices: v
            .offices
            .iter()
            .map(|(office, h)| OfficeRow {
                office: office.clone(),
                holder: hex::encode(h.holder),
                status: h.status.as_str().to_string(),
                at: h.at,
            })
            .collect(),
        affiliations: v
            .affiliations
            .into_iter()
            .map(|(peer, a)| AffiliationRow {
                peer,
                rel: a.rel.as_str().to_string(),
                name: a.name,
                tether: a.tether,
                at: a.at,
            })
            .collect(),
    }
}

fn item_row(v: pacific_core::project::ItemView) -> ProjectItemRow {
    ProjectItemRow {
        id: v.id,
        kind: v.kind.as_str().to_string(),
        title: v.title,
        status: v.status.map(|s| s.as_str().to_string()).unwrap_or_default(),
        blocked: v.blocked,
        assignees: v.assignees.iter().map(hex::encode).collect(),
        at: v.at,
        end_date: v.end_date,
        fields: v.fields.into_iter().collect(),
    }
}

fn deliverable_row(d: pacific_core::project::DeliverableView) -> DeliverableRow {
    DeliverableRow {
        sub_id: d.sub_id,
        target: d.target,
        disclosure: d.disclosure.as_str().to_string(),
        origin_item: d.origin_item,
        last_seen: d.last_seen,
    }
}

/// The single handle the Swift app holds. Immutable (just the state dir), so it
/// is trivially `Send + Sync` as UniFFI requires. Every call rebuilds the Node
/// from persisted state — the exact per-call model the CLI uses.
#[derive(uniffi::Object)]
pub struct Core {
    state_dir: String,
}

impl Core {
    /// Point pacific-core at our sandbox before each op. Cheap and idempotent;
    /// guards against any process that touched the env in between.
    fn point(&self) {
        std::env::set_var("PACIFIC_STATE_DIR", &self.state_dir);
    }
}

/// Drive a pacific-core async op to completion on a local current-thread runtime.
/// pacific-core's relay futures hold !Send state across await (SQLite/MLS), so we
/// block here rather than expose an async FFI method (mirrors the CLI's rt). For
/// a group-of-1 the op is local and returns immediately; callers should invoke
/// the relay-backed path off the Swift main thread.
fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio current-thread runtime")
        .block_on(fut)
}

#[uniffi::export]
impl Core {
    /// Bind the core to an on-device state directory (the iOS app sandbox).
    #[uniffi::constructor]
    pub fn new(state_dir: String) -> Arc<Self> {
        std::env::set_var("PACIFIC_STATE_DIR", &state_dir);
        Arc::new(Self { state_dir })
    }

    /// True once an identity has been minted on this device.
    pub fn has_identity(&self) -> bool {
        self.point();
        Node::open().is_ok()
    }

    /// Mint the on-device identity on first run. Loud if one already exists.
    /// Returns the stable IdentityKey (`ed25519:…`).
    pub fn init_identity(&self, display_name: String) -> Result<String, FfiError> {
        self.point();
        Ok(Node::init_identity(&display_name)?.identity_key())
    }

    /// The RESTORE door: bring an identity back from its recovery key (24 BIP-39
    /// words) on a device that has none. Returns the same IdentityKey the words
    /// were minted under — a wrong or mistyped phrase fails loudly here rather
    /// than quietly producing a different identity.
    ///
    /// Restores the IDENTITY only. Group state and history are not derivable from
    /// a key, so a restored device comes back as itself with no groups until it is
    /// re-admitted.
    pub fn restore_identity(
        &self,
        display_name: String,
        recovery_key: String,
    ) -> Result<String, FfiError> {
        self.point();
        Ok(Node::restore_identity(&display_name, &recovery_key)?.identity_key())
    }

    /// This identity's recovery key — the 24 words that bring it back. Empty when
    /// there is none to show (an identity minted before seeds existed); the UI
    /// omits the step rather than showing a placeholder. Device-local: nothing
    /// transmits this, and no server has ever held it.
    pub fn recovery_key(&self) -> Result<String, FfiError> {
        self.point();
        Ok(Node::open()?.recovery_key().unwrap_or_default())
    }

    /// The stable, user-facing identifier for this device's identity.
    pub fn identity_key(&self) -> Result<String, FfiError> {
        self.point();
        Ok(Node::open()?.identity_key())
    }

    /// This device's own identity display name (from `id init`). The REAL self
    /// name — the app's own avatar/profile reads this, so two devices show their
    /// own identities rather than a shared hardcoded mock.
    pub fn display_name(&self) -> Result<String, FfiError> {
        self.point();
        Ok(Node::open()?.display_name()?)
    }

    /// The human-comparable short authentication string (identity fingerprint).
    pub fn sas(&self) -> Result<String, FfiError> {
        self.point();
        Ok(Node::open()?.sas())
    }

    /// The read-aloud three-word rendering of the SAS — same identity as `sas()`,
    /// for a "verify you match" check people can say out loud.
    pub fn sas_words(&self) -> Result<String, FfiError> {
        self.point();
        Ok(Node::open()?.sas_words())
    }

    /// A fresh, signed contact bundle (base64) to render as a pairing QR.
    pub fn contact_bundle(&self) -> Result<String, FfiError> {
        self.point();
        Ok(Node::open()?.build_contact_bundle()?)
    }

    /// THE WRAP, sealed: the identity seed under a key only the passkey can
    /// derive (`pacific_core::wrap`). `prf_secret` is the passkey's PRF output
    /// over [`wrap_domain`]; `host` is the Arc it will be stored at, bound into
    /// the seal so the record cannot be replayed onto an Arc an attacker owns.
    ///
    /// This is the artefact the Arc serves to anyone who asks by public key, so
    /// it carries the seed and nothing else. Loud on an identity that has none.
    pub fn export_wrap(&self, prf_secret: Vec<u8>, host: String) -> Result<Vec<u8>, FfiError> {
        self.point();
        let prf = prf32(&prf_secret)?;
        Ok(Node::open()?.export_wrap(&prf, &host)?)
    }

    /// THE RESTORE DOOR, first half: on an install that holds no identity, open
    /// `wrap` under the passkey's PRF output and come back as that identity —
    /// same key, so every peer's pin still matches. Returns the IdentityKey.
    ///
    /// A wrong passkey fails before anything is written, and so does the right
    /// passkey against a wrap lifted from another Arc — `host` is bound into the
    /// seal. An install that already holds an identity refuses, loudly.
    ///
    /// THE GROUPS DO NOT ARRIVE WITH IT. This door restores the identity and
    /// nothing else; the history blob that used to carry the groups was an escrow
    /// and has been taken off this surface (see the note below `arc_credential`).
    /// A restored device is a member of no group until the archive chain is wired,
    /// and `m15_identity_restore` asserts exactly that rather than letting a test
    /// pin a promise the design does not make.
    pub fn restore_from_wrap(
        &self,
        display_name: String,
        prf_secret: Vec<u8>,
        wrap: Vec<u8>,
        host: String,
    ) -> Result<String, FfiError> {
        self.point();
        let prf = prf32(&prf_secret)?;
        Ok(Node::restore_from_wrap(&prf, &wrap, &host, &display_name)?.identity_key())
    }

    // THE HISTORY BLOB IS NOT ON THIS SURFACE, and its absence is the point.
    //
    // `export_history` and `import_history` used to sit here. What they moved was
    // `backup::History`: the whole plaintext archive, this device's intro tag, and
    // `mls_mem::Snapshot` — every group's ratchet tree, epoch secrets and
    // key-package private halves. Re-admission §1 called that "a complete key
    // escrow" and the register retired it on 14 September 2026 (Leaves §02, §04):
    // "the whole snapshot is gone; the narrow material stays."
    //
    // It was retired in the documents and left reachable in the code. No app ever
    // called it — iOS wires the identity half (`export_wrap` / `restore_from_wrap`)
    // and has never called this one — so removing it from the FFI takes the escrow
    // out of reach of every client without changing a line of Swift. The core
    // internals go next, with their tests, once the replacement lands.
    //
    // The replacement is the archive chain: one retained exporter output per epoch
    // at an address computed from the storage root, and the MLS state through
    // mls-rs's own `GroupStateStorage`. See `core/docs/archive-by-arithmetic.md`.
    // Until it is wired, a restored device comes back with its identity and no
    // groups — which is what `m15_identity_restore` already asserts, and what the
    // boot flow and `two-devices.sh` already tell the person.

    /// PROVE THIS ACCOUNT TO A SERVER: sign a challenge the Arc just issued.
    ///
    /// `audience` is the Arc the nonce came from, and it is inside the signed
    /// bytes, so a signature captured by one Arc cannot be replayed at another.
    /// Returns the signature as hex; the Arc verifies it against the identity
    /// public key it already knows this account by. The passkey is not involved:
    /// it unwraps the seed on this device and never leaves the authenticator.
    ///
    /// Not a signing oracle — the caller supplies an audience and a nonce, never
    /// the bytes. See `pacific_core::identity::auth_payload` for the one
    /// definition this and the verifier both build from.
    pub fn sign_challenge(&self, audience: String, nonce: String) -> Result<String, FfiError> {
        self.point();
        Ok(Node::open()?.sign_challenge(&audience, &nonce)?)
    }

    /// CLOSE THE FORK after a restore: a fresh-path commit in every group,
    /// through the relay's commit slot, so the install the backup came from can
    /// no longer read forward. Returns the hex ids rekeyed. Fails the whole call
    /// if any group cannot be rekeyed — a partly-rekeyed restore is a partly-
    /// forked one, and the app must say so rather than proceed. Safe to retry.
    /// Relay-backed: call off the main thread.
    pub fn rekey_all_groups(&self) -> Result<Vec<String>, FfiError> {
        self.point();
        let node = Node::open()?;
        Ok(block_on(async {
            let mut sess = pacific_core::router::Router::open(node.routes()).await?;
            let out = node.rekey_all_groups(&mut sess).await;
            sess.close().await;
            out
        })?)
    }

    /// A fresh membership credential for `arc_identity_key` — the canonical Arc credential
    /// (ICD §10). Membership IS the {user, Arc} MLS tether, so there is no key to issue,
    /// store, or leak: a member proves standing by signing with the SAME identity key that
    /// established the tether, and the Arc reads the ROLE that rode that tether to decide
    /// what the caller may do.
    ///
    /// Format, mirroring `verify_member` on the Arc:
    ///   credential = `<identity_key>:<ts_ms>:<sig_hex>`
    ///   sig        = Ed25519 over `pacific-arc-member:v1\n<arc_identity_key>\n<identity_key>\n<ts_ms>`
    /// Sent as `Authorization`. The Arc accepts a ±5 minute window, so this is minted PER
    /// REQUEST and never cached — a stale credential is simply refused.
    ///
    /// The whole string is built here, in one place, rather than exposing a general signing
    /// primitive across the FFI: the payload format stays pinned to the verifier, and the app
    /// never gets a signing oracle it could be talked into pointing at other bytes.
    pub fn arc_member_credential(&self, arc_identity_key: String) -> Result<String, FfiError> {
        self.point();
        let node = Node::open()?;
        // BARE HEX inside the token: the credential is colon-delimited
        // (`<id>:<ts>:<sig>`), and the `ed25519:` rendering carries a colon of its
        // own — putting it here would shatter the split. Hex is the wire form and
        // parses on both sides; the rendering is for display, not for tokens.
        let identity_key = hex::encode(node.id.identity_pk());
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let arc_hex = pacific_core::identity::parse_identity_key(&arc_identity_key)
            .map(hex::encode)?;
        let payload = format!("pacific-arc-member:v1\n{arc_hex}\n{identity_key}\n{ts}");
        let sig = node.id.sign(payload.as_bytes());
        Ok(format!("{identity_key}:{ts}:{}", hex::encode(sig)))
    }

    /// List our standalone objects: (id, kind, name). `name` is the MLS GroupContext
    /// display name — read live, and resolved for the whole list on one MLS client,
    /// so the chat list needs no per-row `object_title` round trip.
    pub fn objects(&self) -> Result<Vec<ObjectRow>, FfiError> {
        self.point();
        Ok(Node::open()?
            .objects_named()?
            .into_iter()
            .map(|(id, kind, name)| ObjectRow { id, kind, name })
            .collect())
    }

    /// Mint a new object (e.g. kind `"forum"`) as a group of one. `name` (may be
    /// empty) rides the MLS GroupContext extension, so late joiners read it from
    /// their Welcome. Local only.
    pub fn object_new(&self, kind: String, name: String) -> Result<String, FfiError> {
        self.point();
        Ok(Node::open()?.object_new(&kind, &name)?)
    }

    /// THE mint — the one call behind every `+` in the app.
    ///
    /// `kind` is the core's own wire name (`ObjectKind::name()`): `group` · `forum` ·
    /// `project` · `thing` · `place` · `event` · `post`. Anything else is refused loudly, which
    /// includes the kinds that exist but are not minted from a draft — `contact` is
    /// formed by PAIRING and `conversation` by consuming a Contact's prekeys.
    ///
    /// Mints the object and authors its op-0 profile delta, nothing more. Posture,
    /// tickets, media and venue are separate ops with their own authority; the caller
    /// authors them after, so a failure is reportable against the step that failed
    /// rather than leaving a half-configured object behind one opaque error.
    pub fn object_mint(&self, kind: String, draft: MintDraft) -> Result<String, FfiError> {
        self.point();
        let k = pacific_core::mint::kind_from_name(&kind)
            .ok_or_else(|| FfiError::Core(format!("unknown object kind '{kind}'")))?;
        Ok(block_on(Node::open()?.mint(k, &draft.into()))?)
    }

    /// Mint a THING and give it a name in one call — what the swipe deck does on ACCEPT.
    /// Local: a group of 1, no relay. Returns the new object id.
    /// `category` is the market-face axis: "artifact" | "skill" | "job" ("" = artifact).
    ///
    /// Kept as a named call for the swipe deck, but it is now `object_mint` with the
    /// Thing draft filled in — there is ONE mint path, not two that can drift.
    pub fn thing_mint(
        &self,
        name: String,
        descriptor: String,
        category: String,
    ) -> Result<String, FfiError> {
        self.object_mint(
            "thing".into(),
            MintDraft {
                name,
                descriptor,
                category,
                ..Default::default()
            },
        )
    }

    /// Set a thing's market posture. `price` is rejected on a standing intent (a Want has
    /// no price until it becomes Buying) — pass "" there. `deadline` 0 = standing.
    #[allow(clippy::too_many_arguments)]
    pub fn thing_set_posture(
        &self,
        object_id: String,
        posture: String,
        price: String,
        deadline: i64,
        reach: String,
        area: String,
    ) -> Result<String, FfiError> {
        self.point();
        let mut args = pacific_core::coordinator::Args::new();
        args.insert(
            "posture".into(),
            pacific_core::coordinator::ArgVal::Text(posture),
        );
        if !price.is_empty() {
            args.insert(
                "price".into(),
                pacific_core::coordinator::ArgVal::Text(price),
            );
        }
        if deadline > 0 {
            args.insert(
                "deadline".into(),
                pacific_core::coordinator::ArgVal::Int(deadline),
            );
        }
        if !reach.is_empty() {
            args.insert(
                "reach".into(),
                pacific_core::coordinator::ArgVal::Text(reach),
            );
        }
        if !area.is_empty() {
            args.insert("area".into(), pacific_core::coordinator::ArgVal::Text(area));
        }
        let node = Node::open()?;
        Ok(block_on(node.apply(
            &object_id,
            pacific_core::thing::OP_SET_POSTURE,
            args,
        ))?)
    }

    /// Drop a thing's posture (and with it any price/deadline, which only qualify one).
    pub fn thing_clear_posture(&self, object_id: String) -> Result<String, FfiError> {
        self.point();
        let node = Node::open()?;
        Ok(block_on(node.apply(
            &object_id,
            pacific_core::thing::OP_CLEAR_POSTURE,
            pacific_core::coordinator::Args::new(),
        ))?)
    }

    /// Set (or clear, with two empty strings) a Thing's photo — the listing's
    /// face. One still, base64-in-delta (ContactCard/EventMedia precedent), capped
    /// at the shared photo cap and REFUSED oversize, never re-encoded. Rides the
    /// next `publish_market` to every connection. Call OFF the Swift main thread.
    pub fn thing_set_photo(
        &self,
        object_id: String,
        photo: String,
        mime: String,
    ) -> Result<String, FfiError> {
        self.point();
        let mut args = pacific_core::coordinator::Args::new();
        if !photo.is_empty() {
            args.insert(
                "photo".into(),
                pacific_core::coordinator::ArgVal::Text(photo),
            );
            args.insert("mime".into(), pacific_core::coordinator::ArgVal::Text(mime));
        }
        let node = Node::open()?;
        Ok(block_on(node.apply(
            &object_id,
            pacific_core::thing::OP_SET_PHOTO,
            args,
        ))?)
    }

    /// Re-title an existing Thing — the profile half of a listing edit. The profile op
    /// is a full replace, so `category` must be restated ("" = artifact).
    pub fn thing_set_profile(
        &self,
        object_id: String,
        name: String,
        descriptor: String,
        category: String,
    ) -> Result<String, FfiError> {
        self.point();
        let mut args = pacific_core::coordinator::Args::new();
        args.insert("name".into(), pacific_core::coordinator::ArgVal::Text(name));
        args.insert(
            "descriptor".into(),
            pacific_core::coordinator::ArgVal::Text(descriptor),
        );
        if !category.is_empty() {
            args.insert(
                "category".into(),
                pacific_core::coordinator::ArgVal::Text(category),
            );
        }
        let node = Node::open()?;
        Ok(block_on(node.apply(
            &object_id,
            pacific_core::thing::OP_SET_PROFILE,
            args,
        ))?)
    }

    /// Truncate a position to a publishable coarse area (a geohash prefix).
    ///
    /// Exposed rather than reimplemented in Swift so there is exactly ONE truncation, and so
    /// the precision floor is enforced in the same place the reducer enforces it.
    pub fn coarse_area(&self, lat: f64, lon: f64, precision: u8) -> String {
        pacific_core::thing::geohash(lat, lon, precision as usize)
    }

    /// Every Thing we own, folded. The LIFE market list reads THIS — minted GroupObjects
    /// are the durable truth; the resolver graph is the derived read-index.
    pub fn things(&self) -> Result<Vec<ThingRow>, FfiError> {
        self.point();
        Ok(Node::open()?
            .things()?
            .into_iter()
            .map(|(id, st)| ThingRow {
                id,
                name: st.name,
                descriptor: st.descriptor,
                category: st.category.as_str().to_string(),
                posture: st
                    .posture
                    .map(|p| p.as_str().to_string())
                    .unwrap_or_default(),
                price: st.price.unwrap_or_default(),
                deadline: st.deadline.unwrap_or(0),
                reach: st.reach.as_str().to_string(),
                area: st.area.unwrap_or_default(),
                photo: st.photo,
                photo_mime: st.photo_mime,
            })
            .collect())
    }

    // ---- EVENT ticketing -------------------------------------------------

    /// Mint an EVENT and set its profile in one call. Local: a group of 1, no relay.
    pub fn event_mint(
        &self,
        title: String,
        descriptor: String,
        start_ms: i64,
        end_ms: i64,
        venue: String,
        // `recurrence`: an RFC 5545 RRULE ("FREQ=WEEKLY;BYDAY=TH") when this
        // repeats; "" for a one-off. Refused at fold if we cannot expand it.
        recurrence: String,
        // `lineup`: newline-separated artist names, billing order; "" = none.
        lineup: String,
    ) -> Result<String, FfiError> {
        self.object_mint(
            "event".into(),
            MintDraft {
                name: title,
                descriptor,
                start_ms,
                end_ms,
                venue,
                recurrence,
                lineup,
                ..Default::default()
            },
        )
    }

    /// Set (or clear) the event's venue — a REFERENCE to a Place GroupObject, the
    /// `happens_at` edge the graph projector folds. Empty `place` clears it. `at` is
    /// the event time (unix ms) the edge became true; 0 = now.
    pub fn event_set_venue(
        &self,
        object_id: String,
        place: String,
        name: String,
        at: i64,
    ) -> Result<String, FfiError> {
        self.point();
        let at = if at > 0 {
            at
        } else {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(1)
        };
        let args = pacific_core::event::set_venue_args(
            (!place.is_empty()).then_some(place.as_str()),
            &name,
            at,
        );
        let node = Node::open()?;
        Ok(block_on(node.apply(
            &object_id,
            pacific_core::event::OP_SET_VENUE,
            args,
        ))?)
    }

    /// Set (or update) the ticket listing. `price_cents` 0 = a free event; a priced
    /// event REQUIRES `acct` (the rail's connected account) and `delegate` (the
    /// fulfilment Arc's identity pk, hex) — refused loudly otherwise. Registration
    /// with the box office is the CALLER's next step (owner-signed HTTP).
    #[allow(clippy::too_many_arguments)]
    pub fn event_set_tickets(
        &self,
        object_id: String,
        price_cents: i64,
        currency: String,
        capacity: i64,
        open: bool,
        acct: String,
        delegate: String,
        terms: String,
        rev: i64,
    ) -> Result<String, FfiError> {
        self.point();
        let delegate_pk: Option<[u8; 32]> = if delegate.is_empty() {
            None
        } else {
            Some(
                hex::decode(&delegate)
                    .ok()
                    .and_then(|v| <[u8; 32]>::try_from(v).ok())
                    .ok_or_else(|| {
                        FfiError::Core("delegate must be a 64-char identity hex".into())
                    })?,
            )
        };
        let args = pacific_core::event::set_tickets_args(
            price_cents,
            &currency,
            capacity.max(0) as u32,
            open,
            (!acct.is_empty()).then_some(acct.as_str()),
            delegate_pk.as_ref(),
            (!terms.is_empty()).then_some(terms.as_str()),
            rev.max(0) as u64,
        );
        let node = Node::open()?;
        Ok(block_on(node.apply(
            &object_id,
            pacific_core::event::OP_SET_TICKETS,
            args,
        ))?)
    }

    /// One Post, folded to display shape.
    ///
    /// `retracted` rides the row rather than filtering the list here: a retraction is a
    /// statement and the surface should be able to show the tombstone. Filtering it out
    /// at the read would make "taken down" indistinguishable from "never existed".
    pub fn posts(&self) -> Result<Vec<PostRow>, FfiError> {
        self.point();
        Ok(Node::open()?
            .posts()?
            .into_iter()
            .map(|(id, st)| PostRow {
                id,
                title: st.title,
                body: st.body,
                icon: st.icon,
                banner: st.banner,
                retracted: st.retracted,
                reactions: st.responses.reactions.len() as u32,
                // A Post's comments live in the Forum it hosts (m34), so the
                // count is that room's, not a field here. `forums` names it.
                comments: 0,
            })
            .collect())
    }

    /// Every Event we hold, folded to display shape. The EVENTS page's GroupObject
    /// source reads THIS — minted objects are the durable truth.
    pub fn events(&self) -> Result<Vec<EventRow>, FfiError> {
        self.point();
        let node = Node::open()?;
        let mut out = Vec::new();
        for (id, st) in node.events()? {
            let listing_key = node.event_listing_id(&id).unwrap_or_default();
            let (placed, lat, lng) = match st.point() {
                Some(p) => (true, p.lat(), p.lng()),
                None => (false, 0.0, 0.0),
            };
            let l = st.listing.as_ref();
            out.push(EventRow {
                id,
                title: st.title.clone(),
                descriptor: st.descriptor.clone(),
                start_ms: st.start_ms,
                end_ms: st.end_ms.unwrap_or(0),
                venue: st.venue.clone(),
                venue_place: st
                    .venue_ref
                    .as_ref()
                    .map(|v| v.place.clone())
                    .unwrap_or_default(),
                venue_place_name: st
                    .venue_ref
                    .as_ref()
                    .map(|v| v.name.clone())
                    .unwrap_or_default(),
                venue_at: st.venue_ref.as_ref().map(|v| v.at).unwrap_or(0),
                placed,
                lat,
                lng,
                listed: l.is_some(),
                price_cents: l.map(|l| l.price_cents).unwrap_or(0),
                currency: l.map(|l| l.currency.clone()).unwrap_or_default(),
                capacity: l.map(|l| l.capacity as i64).unwrap_or(0),
                open: l.map(|l| l.open).unwrap_or(false),
                sold: st.sold() as i64,
                over_capacity: st.over_capacity,
                listing: listing_key,
                banner: st.media.banner.clone(),
                banner_mime: st.media.banner_mime.clone(),
                photo_count: st.media.photos.len() as i64,
                has_clip: !st.media.clip.is_empty(),
                lineup: st.lineup.clone(),
                recurrence: st.recurrence.clone(),
            });
        }
        Ok(out)
    }

    /// Set (or wholesale replace) an event's media facet. Caps are enforced at the
    /// fold (banner ≤ ~700KB b64, ≤ 6 photos ≤ ~500KB each, clip ≤ ~1.5MB b64) —
    /// oversize is refused loudly here via the dry-run probe, never clamped.
    pub fn event_set_media(
        &self,
        object_id: String,
        banner: String,
        banner_mime: String,
        photos: Vec<EventPhotoRow>,
        clip: String,
        clip_mime: String,
    ) -> Result<String, FfiError> {
        self.point();
        let core_photos: Vec<pacific_core::event::EventPhoto> = photos
            .into_iter()
            .map(|p| pacific_core::event::EventPhoto {
                data: p.data,
                mime: p.mime,
            })
            .collect();
        let args = pacific_core::event::set_media_args(
            (!banner.is_empty()).then_some((banner.as_str(), banner_mime.as_str())),
            &core_photos,
            (!clip.is_empty()).then_some((clip.as_str(), clip_mime.as_str())),
        );
        let node = Node::open()?;
        Ok(block_on(node.apply(
            &object_id,
            pacific_core::event::OP_SET_MEDIA,
            args,
        ))?)
    }

    /// The full media facet of one event — the detail page's read.
    pub fn event_media(&self, object_id: String) -> Result<EventMediaRow, FfiError> {
        self.point();
        let st = Node::open()?.event_state(&object_id)?;
        Ok(EventMediaRow {
            banner: st.media.banner,
            banner_mime: st.media.banner_mime,
            photos: st
                .media
                .photos
                .into_iter()
                .map(|p| EventPhotoRow {
                    data: p.data,
                    mime: p.mime,
                })
                .collect(),
            clip: st.media.clip,
            clip_mime: st.media.clip_mime,
        })
    }

    // ---- SYSTEM hydration (external items enter as deltas) ---------------

    /// HYDRATE a batch of external items into the System for `connector`, minting the
    /// System object on first use. Returns how many deltas were actually AUTHORED.
    ///
    /// That count is the one that moved, not the one offered: re-offering an item the
    /// fold already holds at the same revision is skipped, because the steady state of
    /// every connector is re-fetching what it already sent, and writing those would
    /// grow a log that syncs to every member for no change in state.
    ///
    /// Reaches the relay once the object has other members → call OFF the main thread.
    pub fn system_hydrate(
        &self,
        connector: String,
        display_name: String,
        scope: String,
        items: Vec<HydrateItem>,
    ) -> Result<u32, FfiError> {
        self.point();
        let batch: Vec<(String, String, i64, u64, bool)> = items
            .into_iter()
            .map(|i| {
                (
                    i.key,
                    i.payload,
                    i.fetched_at,
                    i.rev.max(0) as u64,
                    i.withdrawn,
                )
            })
            .collect();
        let node = Node::open()?;
        Ok(block_on(node.system_hydrate(
            &connector,
            &display_name,
            (!scope.is_empty()).then_some(scope.as_str()),
            &batch,
        ))? as u32)
    }

    /// Every System this device holds.
    pub fn systems(&self) -> Result<Vec<SystemRow>, FfiError> {
        self.point();
        Ok(Node::open()?
            .systems()?
            .into_iter()
            .map(|(id, st)| SystemRow {
                id,
                name: st.name.clone(),
                connector: st.connector.clone(),
                scope: st.scope.clone(),
                live_items: st.live().len() as i64,
                total_items: st.items.len() as i64,
            })
            .collect())
    }

    /// The items a System holds. `live_only` excludes withdrawn tombstones — what a
    /// feed shows; pass false to see the tombstones too.
    pub fn system_items(
        &self,
        object_id: String,
        live_only: bool,
    ) -> Result<Vec<HydratedItemRow>, FfiError> {
        self.point();
        let st = Node::open()?.system_state(&object_id)?;
        Ok(st
            .items
            .values()
            .filter(|i| !live_only || !i.withdrawn)
            .map(|i| HydratedItemRow {
                key: i.key.clone(),
                payload: i.payload.clone(),
                fetched_at: i.fetched_at,
                rev: i.rev as i64,
                withdrawn: i.withdrawn,
            })
            .collect())
    }

    /// The stable listing key of one event (what buyers and the box office use).
    pub fn event_listing_id(&self, object_id: String) -> Result<String, FfiError> {
        self.point();
        Ok(Node::open()?.event_listing_id(&object_id)?)
    }

    /// BUYER: author the request Delta into the connection with `peer` (the organizer
    /// on free events, the fulfilment Arc on priced ones) — called BEFORE opening the
    /// checkout page, so a paid-then-killed-app session always has a request.
    pub fn ticket_request(
        &self,
        peer: String,
        listing: String,
        qty: i64,
        pi: String,
    ) -> Result<String, FfiError> {
        self.point();
        let peer_pk = parse_peer_hex(&peer)?;
        let node = Node::open()?;
        Ok(block_on(node.apply(
            &node.connection_object(&peer_pk)?
                .ok_or_else(|| FfiError::Core("no connection with that peer".into()))?
                .0,
            pacific_core::event::OP_REQUEST_TICKET,
            pacific_core::event::request_ticket_args(&listing, qty.max(0) as u32, &pi),
        ))?)
    }

    /// Invite `peer` to an event. The leg carries title/when/where because the guest
    /// never holds the Event GroupObject — see `InviteRow`.
    #[allow(clippy::too_many_arguments)]
    pub fn send_invite(
        &self,
        peer: String,
        event: String,
        title: String,
        start_ms: i64,
        venue: String,
        note: String,
        at: i64,
    ) -> Result<String, FfiError> {
        self.point();
        let peer_pk = parse_peer_hex(&peer)?;
        let node = Node::open()?;
        Ok(block_on(node.apply(
            &node.connection_object(&peer_pk)?
                .ok_or_else(|| FfiError::Core("no connection with that peer".into()))?
                .0,
            pacific_core::event::OP_INVITE,
            pacific_core::event::invite_args(&event, &title, start_ms, &venue, &note, at),
        ))?)
    }

    /// Answer an invitation. Changing your mind is just calling this again.
    pub fn reply_invite(
        &self,
        peer: String,
        event: String,
        going: bool,
        at: i64,
    ) -> Result<String, FfiError> {
        self.point();
        let peer_pk = parse_peer_hex(&peer)?;
        let node = Node::open()?;
        Ok(block_on(node.apply(
            &node.connection_object(&peer_pk)?
                .ok_or_else(|| FfiError::Core("no connection with that peer".into()))?
                .0,
            pacific_core::event::OP_INVITE_REPLY,
            pacific_core::event::invite_reply_args(&event, going, at),
        ))?)
    }

    /// Every invitation on the channel with `peer` — sent AND received, newest first,
    /// each joined to its answer. One read for the thread card and the event page.
    pub fn invites(&self, peer: String) -> Result<Vec<InviteRow>, FfiError> {
        self.point();
        let peer_pk = parse_peer_hex(&peer)?;
        let node = Node::open()?;
        let st = node.contact_state_with(&peer_pk)?;
        let mut out: Vec<InviteRow> = st
            .invites
            .values()
            .map(|i| {
                // The answer is keyed by (replier, event) and the replier is whoever is
                // NOT the inviter — on a 2-member channel that is unambiguous.
                let answer = st
                    .invite_replies
                    .values()
                    .find(|r| r.event == i.event && r.author != i.author);
                InviteRow {
                    event: i.event.clone(),
                    title: i.title.clone(),
                    start_ms: i.start_ms,
                    venue: i.venue.clone(),
                    note: i.note.clone(),
                    at: i.at,
                    author: hex::encode(i.author),
                    reply: answer
                        .map(|r| if r.going { "going" } else { "not_going" })
                        .unwrap_or("")
                        .into(),
                    replied_at: answer.map(|r| r.at).unwrap_or(0),
                }
            })
            .collect();
        out.sort_by(|a, b| b.at.cmp(&a.at));
        Ok(out)
    }

    /// SELLER: pending ticket requests folded off our connection with `peer`.
    pub fn ticket_requests(&self, peer: String) -> Result<Vec<TicketRequestRow>, FfiError> {
        self.point();
        let peer_pk = parse_peer_hex(&peer)?;
        let node = Node::open()?;
        Ok(node
            .contact_state_with(&peer_pk)?
            .ticket_requests
            .into_iter()
            .map(|(pi, r)| TicketRequestRow {
                pi,
                listing: r.listing,
                qty: r.qty as i64,
                author: hex::encode(r.author),
            })
            .collect())
    }

    /// SELLER: record a sale on the event ledger. `pi` is the payment reference
    /// ("free-…" client claims on free events); `tickets` are the minted ids.
    #[allow(clippy::too_many_arguments)]
    pub fn event_record_sale(
        &self,
        object_id: String,
        pi: String,
        buyer: String,
        qty: i64,
        unit_cents: i64,
        fee_cents: i64,
        currency: String,
        tickets: Vec<String>,
    ) -> Result<String, FfiError> {
        self.point();
        let buyer_pk = parse_peer_hex(&buyer)?;
        let node = Node::open()?;
        Ok(block_on(node.apply(
            &object_id,
            pacific_core::event::OP_RECORD_SALE,
            pacific_core::event::record_sale_args(
                &pi,
                &buyer_pk,
                qty.max(0) as u32,
                unit_cents,
                fee_cents,
                &currency,
                &tickets,
            ),
        ))?)
    }

    /// SELLER: issue the signed credentials for a recorded sale and DELIVER them into
    /// the connection with the sale's buyer, in one call. The fulfilment tail:
    /// recordSale → this → (box-office fulfil, last).
    pub fn event_issue_and_deliver(&self, object_id: String, pi: String) -> Result<u32, FfiError> {
        self.point();
        let node = Node::open()?;
        let creds = node.event_issue_credentials(&object_id, &pi)?;
        let st = node.event_state(&object_id)?;
        let sale = st
            .ledger
            .get(&pi)
            .ok_or_else(|| FfiError::Core(format!("no sale {pi}")))?;
        let listing = node.event_listing_id(&object_id)?;
        let n = creds.len() as u32;
        block_on(node.apply(
            &node
                .connection_object(&sale.buyer)?
                .ok_or_else(|| FfiError::Core("no connection with that peer".into()))?
                .0,
            pacific_core::event::OP_DELIVER_TICKET,
            pacific_core::event::deliver_ticket_args(&listing, &creds),
        ))?;
        Ok(n)
    }

    /// BUYER: the ticket wallet folded off the connection with `peer` (who delivered).
    /// `verified` checks each credential's signature against that peer's identity —
    /// the signer a buyer can actually know.
    pub fn wallet(&self, peer: String) -> Result<Vec<WalletTicketRow>, FfiError> {
        self.point();
        let peer_pk = parse_peer_hex(&peer)?;
        let node = Node::open()?;
        Ok(node
            .contact_state_with(&peer_pk)?
            .wallet
            .into_iter()
            .map(|(ticket, w)| {
                let (title, start_ms) = match pacific_core::event::parse_ticket_core(&w.core) {
                    Some(c) => (c.title, c.start_ms),
                    None => (String::new(), 0),
                };
                let verified =
                    pacific_core::event::verify_ticket(&w.core, &w.sig, &peer_pk).is_ok();
                WalletTicketRow {
                    ticket,
                    listing: w.listing,
                    qr: pacific_core::event::qr_payload(&w.core, &w.sig),
                    core: w.core,
                    sig: w.sig,
                    title,
                    start_ms,
                    verified,
                    admitted_ms: w.admitted_ms,
                }
            })
            .collect())
    }

    /// DOOR: one scanned QR → one verdict, joined in core. Verifies the signature
    /// against the signer THIS device's fold names (the listing's delegate, else the
    /// owner), then joins ledger ∩ redemptions. "unknown" is AMBER, not red — this
    /// device may simply not have synced since the sale.
    pub fn event_door_check(
        &self,
        object_id: String,
        qr: String,
    ) -> Result<DoorCheckRow, FfiError> {
        self.point();
        let node = Node::open()?;
        let empty = |verdict: &str| DoorCheckRow {
            verdict: verdict.into(),
            ticket: String::new(),
            title: String::new(),
            buyer: String::new(),
        };
        let Ok((core_json, sig)) = pacific_core::event::parse_qr_payload(&qr) else {
            return Ok(empty("bad_qr"));
        };
        let st = node.event_state(&object_id)?;
        let owner_pk: [u8; 32] = node
            .object_owner(&object_id)?
            .and_then(|v| <[u8; 32]>::try_from(v).ok())
            .ok_or_else(|| FfiError::Core(format!("no owner for {object_id}")))?;
        let signer = st.credential_signer(&owner_pk);
        let Ok(core) = pacific_core::event::verify_ticket(&core_json, &sig, &signer) else {
            return Ok(empty("bad_sig"));
        };
        let verdict = match st.door_check(&core.ticket) {
            pacific_core::event::DoorVerdict::Unknown => "unknown",
            pacific_core::event::DoorVerdict::Valid => "valid",
            pacific_core::event::DoorVerdict::AlreadyRedeemed {
                double_scanned: true,
            } => "double",
            pacific_core::event::DoorVerdict::AlreadyRedeemed { .. } => "redeemed",
        };
        Ok(DoorCheckRow {
            verdict: verdict.into(),
            ticket: core.ticket,
            title: core.title,
            buyer: core.buyer,
        })
    }

    /// DOOR → HOLDER: tell the buyer their ticket was admitted.
    ///
    /// Redemption lands in the EVENT fold, which the buyer is deliberately not a member
    /// of, so without this receipt the holder's screen can never truthfully say they
    /// were let in. Requires a connection with the buyer (the door usually has one —
    /// it is whoever delivered the ticket); with no connection this is a no-op the
    /// caller can ignore, and the seller sends it on its next pass instead.
    pub fn ticket_admit_receipt(
        &self,
        buyer: String,
        ticket: String,
        admitted_ms: i64,
    ) -> Result<String, FfiError> {
        self.point();
        let peer = parse_peer_hex(&buyer)?;
        let node = Node::open()?;
        Ok(block_on(node.apply(
            &node
                .connection_object(&peer)?
                .ok_or_else(|| FfiError::Core("no connection with that peer".into()))?
                .0,
            pacific_core::event::OP_ADMIT_TICKET,
            pacific_core::event::admit_ticket_args(&ticket, admitted_ms),
        ))?)
    }

    /// DOOR: admit — author the redemption Delta (OR-set; converges across doors).
    pub fn event_redeem(&self, object_id: String, ticket: String) -> Result<String, FfiError> {
        self.point();
        let node = Node::open()?;
        Ok(block_on(node.apply(
            &object_id,
            pacific_core::event::OP_REDEEM,
            pacific_core::event::redeem_args(&ticket),
        ))?)
    }

    /// OWNER/SELLER: restate own current event state at the current epoch so a
    /// late-joining door-staff device converges (state re-emission — the no-backfill
    /// doctrine's answer). Call after adding a member. Returns deltas re-emitted.
    pub fn event_reemit(&self, object_id: String) -> Result<u32, FfiError> {
        self.point();
        let node = Node::open()?;
        Ok(block_on(node.event_reemit(&object_id))? as u32)
    }

    /// Every relay tag this device currently listens on, lowercase hex — the set the
    /// WAKE TIER must watch to know this phone has mail (RINGFENCE §6).
    ///
    /// A pure directory read: no MLS group loaded, no epoch advanced, no relay
    /// touched — so the app may call it on every sync tick. Tags rotate per epoch,
    /// so the result is a snapshot; the caller re-registers when the set changes.
    /// Sorted and deduped, so an unchanged set hashes identically.
    pub fn wake_tags(&self) -> Result<Vec<String>, FfiError> {
        self.point();
        Ok(Node::open()?.wake_tags()?)
    }

    /// THE MARKET READ: every live listing others have published or relayed to me.
    ///
    /// This is the buy side — what `things()` is to my own inventory, this is to everyone
    /// else's. Deduped across channels by `(origin, thing_id)`, newest revision winning and
    /// the shortest path breaking ties, so a listing that reached me three ways appears once.
    pub fn listings(&self) -> Result<Vec<ListingRow>, FfiError> {
        self.point();
        Ok(Node::open()?
            .inbound_listings()?
            .into_iter()
            .map(|l| ListingRow {
                origin: hex::encode(l.origin),
                thing_id: l.thing_id,
                posture: l.posture.as_str().to_string(),
                title: l.title,
                descriptor: l.descriptor,
                price: l.price.unwrap_or_default(),
                deadline: l.deadline.unwrap_or(0),
                area: l.area.unwrap_or_default(),
                // DISTANCE FROM ME, not forwards from the origin. The wire counts
                // forwards — 0 is a direct publish — because that is what
                // MAX_LISTING_HOPS and check_relay_path meter, and the wire keeps
                // it. The READ MODEL answers a different question: how far away is
                // this? A connection is one hop away, not zero. 0 is reserved for
                // my own fold, which never reaches this list (`merged_listings`
                // excludes me), so a listing here is never 0.
                //
                // `DiscoveredRow` already answers it this way (`by_kind_hops` does
                // `l.hops + 1`), and the two read models disagreeing by one about
                // the same listing is exactly the drift this closes.
                hops: l.hops + 1,
                via: hex::encode(l.via),
                photo: l.photo,
                photo_mime: l.photo_mime,
            })
            .collect())
    }

    /// Run the market pass: fan my postures out to every connection, then relay onward every
    /// `network` listing I hold. Returns the number of deltas authored — 0 when nothing has
    /// changed, so it is safe (and intended) to call on every sync.
    pub fn publish_market(&self) -> Result<u32, FfiError> {
        self.point();
        Ok(block_on(Node::open()?.publish_market(false))? as u32)
    }

    // -- Places -------------------------------------------------------------

    /// Mint a PLACE: a durable, shareable GroupObject for somewhere you have been.
    ///
    /// `placed` is what the caller asserts about `lat`/`lng`. Pass `false` when no fix
    /// was acquired and the Place is minted UNPLACED — the object still exists (the
    /// user's name and note are not thrown away because GPS was slow indoors) and a
    /// later `place_set_location` places it. Passing `true` with junk coordinates is
    /// rejected by the reducer rather than stored.
    ///
    /// Local: a group of 1, no relay traffic, until it is shared.
    pub fn place_mint(
        &self,
        name: String,
        descriptor: String,
        placed: bool,
        lat: f64,
        lng: f64,
    ) -> Result<String, FfiError> {
        self.object_mint(
            "place".into(),
            MintDraft {
                name,
                descriptor,
                placed,
                lat,
                lng,
                ..Default::default()
            },
        )
    }

    /// Place (or move) an existing Place by authoring a base `setLocation` delta.
    /// Owner-sequenced: where somewhere is, is the owner's assertion about their own
    /// Place.
    pub fn place_set_location(
        &self,
        object_id: String,
        lat: f64,
        lng: f64,
    ) -> Result<String, FfiError> {
        self.point();
        let source = pacific_core::geo::LocationSource::Fixed {
            point: pacific_core::geo::GeoPoint::from_degrees(lat, lng),
        };
        Ok(block_on(Node::open()?.apply(
            &object_id,
            pacific_core::geo::OP_SET_LOCATION,
            pacific_core::geo::set_location_args(&source),
        ))?)
    }

    /// Every Place we own, folded. This is what LIFE's PLACES list reads — the minted
    /// GroupObjects are the durable truth.
    pub fn places(&self) -> Result<Vec<PlaceRow>, FfiError> {
        self.point();
        Ok(Node::open()?
            .places()?
            .into_iter()
            .map(|(id, st)| place_row(id, st))
            .collect())
    }

    /// One Place, folded.
    pub fn place(&self, object_id: String) -> Result<PlaceRow, FfiError> {
        self.point();
        let st = Node::open()?.place_state(&object_id)?;
        Ok(place_row(object_id, st))
    }

    // -- Discovery (ask the graph) -------------------------------------------

    /// The app-load pass: warm the n-2 gossip caches by re-asking the graph for
    /// every discoverable kind at full budget. Idempotent — a still-live question
    /// is reused, never re-flooded. Trade warms itself (listings are push gossip,
    /// reconciled by every sync). Returns the request ids in play.
    pub fn warm_caches(&self) -> Result<Vec<String>, FfiError> {
        self.point();
        Ok(block_on(Node::open()?.warm_gossip_caches())?)
    }

    /// Ask my connections (and, within `budget` hops, theirs) to list their PUBLIC
    /// places matching `tags`. Returns the request id; answers stream in over the
    /// next sync ticks — read them with [`Self::discover_results`]. Re-asking the
    /// same live question reuses its id rather than re-flooding.
    pub fn discover_start(
        &self,
        kind: String,
        tags: Vec<String>,
        budget: u32,
    ) -> Result<String, FfiError> {
        self.point();
        Ok(block_on(Node::open()?.discover(&kind, &tags, budget))?)
    }

    /// Everything of `kind` within `hops` handshakes: what I hold (hops 0) merged
    /// with every live answer to my questions — shortest-path deduped, and a row I
    /// already ADOPTED locally wins over its remote reference. Pure read.
    pub fn discover_results(
        &self,
        kind: String,
        hops: u32,
    ) -> Result<Vec<DiscoveredRow>, FfiError> {
        self.point();
        Ok(Node::open()?
            .discover_results(&kind, hops)?
            .into_iter()
            .map(|r| {
                let (placed, lat, lng) = degrees(r.point.as_ref());
                DiscoveredRow {
                    id: r.id,
                    kind: r.kind,
                    name: r.name,
                    descriptor: r.descriptor,
                    placed,
                    lat,
                    lng,
                    holder: r.holder.map(hex::encode).unwrap_or_default(),
                    hops: r.hops,
                }
            })
            .collect())
    }

    /// Adopt a DISCOVERED place as a local Place I can publish into. The remote id
    /// lands in the location's provenance slot, so adopting the same discovery
    /// twice returns the existing Place and the remote row stops being listed.
    pub fn place_adopt(
        &self,
        name: String,
        descriptor: String,
        placed: bool,
        lat: f64,
        lng: f64,
        remote_id: String,
    ) -> Result<String, FfiError> {
        self.point();
        let (lat, lng) = if placed {
            (Some(lat), Some(lng))
        } else {
            (None, None)
        };
        Ok(block_on(Node::open()?.place_adopt(
            &name,
            &descriptor,
            lat,
            lng,
            &remote_id,
        ))?)
    }

    /// Publish a note AT a Place — any member may; `note` is the note's id, so
    /// publishing the same note again replaces the earlier copy rather than
    /// stacking a duplicate. Returns the delta id.
    pub fn place_post(
        &self,
        object_id: String,
        note: String,
        title: String,
        text: String,
        at: i64,
    ) -> Result<String, FfiError> {
        self.point();
        Ok(block_on(
            Node::open()?.place_post(&object_id, &note, &title, &text, at),
        )?)
    }

    /// The notes published at one Place, newest first.
    pub fn place_posts(&self, object_id: String) -> Result<Vec<PlacePostRow>, FfiError> {
        self.point();
        Ok(Node::open()?
            .place_posts(&object_id)?
            .into_iter()
            .map(|p| PlacePostRow {
                author: hex::encode(p.author),
                note: p.note,
                title: p.title,
                text: p.text,
                at: p.at,
            })
            .collect())
    }

    // -- Public places (request to join) ------------------------------------

    /// Open a Place to join requests, or close it again. `access` is "public" | "private".
    ///
    /// OPENING IS THE CONSENT: while public, whoever scans the QR is admitted
    /// automatically on the host's next sync, with no per-person approval. Opening does
    /// not publish the Place's contents — joiners become members, they do not read it
    /// from outside. Closing stops future admissions and evicts nobody.
    pub fn place_set_access(&self, object_id: String, access: String) -> Result<String, FfiError> {
        self.point();
        let access = pacific_core::place::Access::parse(&access)
            .map_err(|_| FfiError::Core("access must be \"public\" or \"private\"".into()))?;
        Ok(block_on(
            Node::open()?.place_set_access(&object_id, access),
        )?)
    }

    /// Set how far the fact of this Place's existence may TRAVEL — the visibility
    /// dial, not the door. "private" (members only, never advertised) |
    /// "connections" (direct peers, never relayed) | "network" (the full n-2
    /// discovery reach). Tightening heals outward: live questions are re-answered
    /// with the reduced set, and every peer's next app-load warm refreshes their
    /// cache.
    pub fn place_set_visibility(
        &self,
        object_id: String,
        visibility: String,
    ) -> Result<String, FfiError> {
        self.point();
        let v = pacific_core::visibility::Visibility::parse(&visibility).map_err(|_| {
            FfiError::Core("visibility must be \"private\" | \"connections\" | \"network\"".into())
        })?;
        Ok(block_on(Node::open()?.place_set_visibility(&object_id, v))?)
    }

    /// The string behind a PUBLIC place's QR — what goes on the poster.
    ///
    /// Carries the place id AND a fresh signed contact bundle for us, because a stranger
    /// has no other route to the host in this architecture: no directory, no server-side
    /// identity lookup. Scanning PAIRS with the host, and the host admits automatically.
    ///
    /// Errors on a private Place rather than vending a bundle — handing out a pairing
    /// route to somewhere the owner has not opened is not a thing to do quietly.
    pub fn place_join_card(&self, object_id: String) -> Result<String, FfiError> {
        self.point();
        Ok(Node::open()?.place_join_card(&object_id)?)
    }

    /// Record whose land a Place sits on — an authored CLAIM with its provenance.
    ///
    /// The core does not do the lookup: HMLR's free products are MONTHLY BULK FILES
    /// (INSPIRE for the parcel, CCOD/OCOD for a corporate proprietor), not a point-query
    /// API, and Scotland is a separate register. Resolve outside, author the answer here.
    ///
    /// `source` is "inspire" | "ccod" | "ocod" | "scotlis" | "manual". Naming a proprietor
    /// while citing "inspire" is REJECTED — INSPIRE is geometry and cannot know owners.
    /// `as_of` is the dataset vintage in unix ms, and is required.
    ///
    /// `permission` is "unknown" | "asked" | "granted" | "refused" — a self-report, because
    /// no register can say whether the owner agreed. Knowing the owner is never treated as
    /// having asked them.
    #[allow(clippy::too_many_arguments)]
    pub fn place_set_land(
        &self,
        object_id: String,
        parcel: String,
        proprietor: String,
        company_no: String,
        source: String,
        as_of: i64,
        permission: String,
    ) -> Result<String, FfiError> {
        self.point();
        let source = pacific_core::place::LandSource::parse(&source).map_err(|_| {
            FfiError::Core("source must be inspire|ccod|ocod|scotlis|manual".into())
        })?;
        let permission = pacific_core::place::LandPermission::parse(&permission).map_err(|_| {
            FfiError::Core("permission must be unknown|asked|granted|refused".into())
        })?;
        let claim = pacific_core::place::LandClaim {
            parcel,
            proprietor,
            company_no,
            source,
            as_of,
            permission,
        };
        Ok(block_on(Node::open()?.place_set_land(&object_id, &claim))?)
    }

    /// Retract a land claim — the lookup was wrong, or the land changed hands.
    pub fn place_clear_land(&self, object_id: String) -> Result<String, FfiError> {
        self.point();
        Ok(block_on(Node::open()?.place_clear_land(&object_id))?)
    }

    /// Fit (or ROTATE) a place's doorbell. Returns the new capability as hex.
    ///
    /// Rotating retires every printed sign carrying the old one, and is the only way to
    /// stop an ex-member reading arrivals — removing them from the group does not retract
    /// the doorbell they already folded out of the log.
    pub fn place_fit_doorbell(&self, object_id: String) -> Result<String, FfiError> {
        self.point();
        Ok(hex::encode(block_on(
            Node::open()?.place_fit_doorbell(&object_id),
        )?))
    }

    /// KNOCK — what a stranger's phone does on scanning the sign.
    ///
    /// Deposits OUR signed contact bundle in the place's doorbell mailbox. No pairing, no
    /// 1:1 connection with whoever printed the poster, no waiting on prekeys. Returns as
    /// soon as the knock is published; MEMBERSHIP LANDS when any member next answers.
    pub fn place_knock(&self, place_id: String, doorbell_hex: String) -> Result<(), FfiError> {
        self.point();
        let raw = hex::decode(&doorbell_hex)
            .ok()
            .and_then(|v| <[u8; 32]>::try_from(v).ok())
            .ok_or_else(|| FfiError::Core("doorbell must be 64 hex chars".into()))?;
        Ok(block_on(Node::open()?.place_knock(&place_id, &raw))?)
    }

    /// Answer the door at every open place we are in, now rather than on the next sync.
    /// Runs on EVERY member, not just the owner. Returns what anyone was admitted to.
    pub fn place_answer_doors(&self) -> Result<Vec<String>, FfiError> {
        self.point();
        Ok(block_on(Node::open()?.answer_all_doors())?)
    }

    // -- Anchoring a group to a place (the agreement) -------------------------

    /// Put your group on the bench: record the undertaking to admit whoever scans that
    /// place and picks you. Refuses at a place that is not open.
    pub fn group_anchor_at_place(
        &self,
        group_id: String,
        place_id: String,
        at_ms: i64,
    ) -> Result<String, FfiError> {
        self.point();
        Ok(block_on(
            Node::open()?.group_anchor_at_place(&group_id, &place_id, at_ms),
        )?)
    }

    /// Stop answering for a place. Members already admitted stay.
    /// Set (or clear) the group's COVER banner — one slot, wholesale replaced.
    /// `mime` ∈ {image/jpeg, image/png, image/gif, video/mp4}; empty `data` clears.
    /// Caps are enforced at the fold (stills/gifs ≤ 700K b64, mp4 ≤ 1.5M) — refused,
    /// never clamped, so fit media BEFORE authoring.
    pub fn group_set_cover(
        &self,
        object_id: String,
        data: String,
        mime: String,
    ) -> Result<String, FfiError> {
        self.point();
        let mut args = pacific_core::coordinator::Args::new();
        args.insert("data".into(), pacific_core::coordinator::ArgVal::Text(data));
        args.insert("mime".into(), pacific_core::coordinator::ArgVal::Text(mime));
        let node = Node::open()?;
        Ok(block_on(node.apply(
            &object_id,
            pacific_core::group::OP_SET_COVER,
            args,
        ))?)
    }

    /// Record that the group CREATED `object_id` — an Event it hosts or a Thing it
    /// lists. Owner-only (the affiliation op is owner/sequenced). `at` 0 = now.
    pub fn group_attach_created(
        &self,
        group_id: String,
        object_id: String,
        name: String,
        at: i64,
    ) -> Result<String, FfiError> {
        self.point();
        let at = if at > 0 {
            at
        } else {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(1)
        };
        Ok(block_on(
            Node::open()?.group_attach_created(&group_id, &object_id, &name, at),
        )?)
    }

    pub fn group_unanchor(&self, group_id: String, place_id: String) -> Result<String, FfiError> {
        self.point();
        Ok(block_on(
            Node::open()?.group_unanchor(&group_id, &place_id),
        )?)
    }

    /// The places this group has agreed to answer for: `(place_id, place_name)` pairs
    /// flattened as alternating entries.
    pub fn group_anchored_places(&self, group_id: String) -> Result<Vec<String>, FfiError> {
        self.point();
        Ok(Node::open()?
            .group_anchored_places(&group_id)?
            .into_iter()
            .flat_map(|(id, name)| [id, name])
            .collect())
    }

    /// Our groups that answer for `place_id` — what the scan picker lists.
    pub fn groups_anchored_at(&self, place_id: String) -> Result<Vec<String>, FfiError> {
        self.point();
        Ok(Node::open()?.our_groups_anchored_at(&place_id)?)
    }

    /// Admit everyone pending on our public places NOW, rather than waiting for the next
    /// sync tick (the sync loop calls this itself). Returns the ids of places somebody
    /// was admitted to.
    pub fn place_reconcile_joins(&self) -> Result<Vec<String>, FfiError> {
        self.point();
        Ok(block_on(Node::open()?.reconcile_place_joins())?)
    }

    /// Admit a joiner explicitly. Public places do this themselves on every sync
    /// (`reconcile_place_joins`); this is the manual escape hatch — adding someone who
    /// never scanned, or forcing it without waiting for a tick. Refused if the Place is
    /// private.
    pub fn place_admit(&self, object_id: String, peer_id: String) -> Result<String, FfiError> {
        self.point();
        let peer = pacific_core::identity::parse_identity_key(&peer_id)?;
        Ok(space_id_of(&block_on(
            Node::open()?.place_admit(&object_id, &peer),
        )?))
    }

    /// Author one `forum.post` into a Forum object. For a group-of-1 this is a
    /// local append (survives restarts) and never touches the relay; once the
    /// object has other members it posts over the shared group tag. Async because
    /// the N-member path publishes to the relay — pass the relay URL (unused for a
    /// solo object, but required by the one write path).
    pub fn object_post(
        &self,
        object_id: String,
        text: String,
        media: Option<MediaInput>,
    ) -> Result<String, FfiError> {
        self.point();
        let node = Node::open()?;
        let m = media.as_ref().map(attach).transpose()?;
        Ok(block_on(node.object_post(&object_id, &text, m.as_ref()))?)
    }

    /// The group's DISPLAY NAME, read from its MLS GroupContext extension ("" until
    /// named). Rides the Welcome, so every member — including late joiners — reads the
    /// same current value; the UI derives a fallback label only while this is blank.
    pub fn object_title(&self, object_id: String) -> Result<String, FfiError> {
        self.point();
        Ok(Node::open()?.object_name(&object_id)?)
    }

    /// The object's OWNER as 64-char identity hex (same form as `object_members`),
    /// or "" if we hold no such group. Only the owner may add members or rename, so
    /// the UI compares this to its own identity before offering either.
    pub fn object_owner(&self, object_id: String) -> Result<String, FfiError> {
        self.point();
        Ok(Node::open()?
            .object_owner(&object_id)?
            .map(hex::encode)
            .unwrap_or_default())
    }

    /// RENAME an object — an owner-only MLS GroupContextExtensions commit. Existing
    /// members fold it off the epoch tag on their next sync; anyone added afterwards
    /// reads the new name from their Welcome. Reaches the relay for a multi-member
    /// object → call OFF the Swift main thread. A non-owner throws, as does losing
    /// the epoch's commit slot (retry against the new epoch).
    pub fn object_rename(&self, object_id: String, name: String) -> Result<(), FfiError> {
        self.point();
        let node = Node::open()?;
        block_on(node.object_rename(&object_id, &name))?;
        Ok(())
    }

    /// The folded transcript of a local Forum object (deterministic order).
    pub fn object_transcript(&self, object_id: String) -> Result<Vec<String>, FfiError> {
        self.point();
        Ok(Node::open()?.object_transcript(&object_id)?)
    }

    /// The STRUCTURED transcript of a Forum object — the group-object twin of
    /// `dm_messages` (timestamps, reply linkage, reaction groups). One UI path.
    pub fn object_messages(&self, object_id: String) -> Result<Vec<ChatMsg>, FfiError> {
        self.point();
        let node = Node::open()?;
        let me = node.id.identity_pk();
        Ok(to_chat_msgs(node.object_detailed(&object_id)?, &me))
    }

    /// The THREADED projection of a Forum (ChatRoom) object — Reddit-style nesting:
    /// each post with its `depth` and `reply_count`, pre-order flattened. The tree
    /// twin of `object_messages`; the ChatRoom UI binds to this. Reply/react target
    /// the same `post.author`/`post.gen` as the flat path.
    pub fn object_thread(&self, object_id: String) -> Result<Vec<ForumThreadRow>, FfiError> {
        self.point();
        let node = Node::open()?;
        let me = node.id.identity_pk();
        Ok(to_thread_rows(node.object_thread(&object_id)?, &me))
    }

    /// The MLS member roster of an object — identity pubkeys as 64-char hex, the
    /// same ids that appear in `ProjectItemRow.assignees` and `ChatMsg.author`. Who
    /// a Project's assignee picker offers and the "Add people" screen already lists.
    pub fn object_members(&self, object_id: String) -> Result<Vec<String>, FfiError> {
        self.point();
        Ok(Node::open()?
            .object_members(&object_id)?
            .into_iter()
            .map(hex::encode)
            .collect())
    }

    /// Toggle THIS device's emoji reaction on a message in a Forum object — the
    /// group-object twin of `react_dm`. Reaches the relay once the object has
    /// other members — call OFF the Swift main thread.
    pub fn object_react(
        &self,
        object_id: String,
        target_author: String,
        target_gen: u64,
        emoji: String,
        active: bool,
    ) -> Result<(), FfiError> {
        self.point();
        let target = parse_msgref(&target_author, target_gen)?;
        block_on(Node::open()?.object_react(&object_id, target, &emoji, active))?;
        Ok(())
    }

    /// Up/downvote a message in a Forum object: `dir` 1 (up), -1 (down), 0
    /// (clear your vote) — the karma twin of `object_react`, one vote per member
    /// per post (LWW). Reaches the relay once the object has other members —
    /// call OFF the Swift main thread.
    pub fn object_vote_post(
        &self,
        object_id: String,
        target_author: String,
        target_gen: u64,
        dir: i8,
    ) -> Result<(), FfiError> {
        self.point();
        if !(-1..=1).contains(&dir) {
            return Err(FfiError::Core(format!(
                "vote dir must be -1|0|1, got {dir}"
            )));
        }
        let target = parse_msgref(&target_author, target_gen)?;
        block_on(Node::open()?.object_vote_post(&object_id, target, dir))?;
        Ok(())
    }

    /// Reply to a message in a Forum object — the group-object twin of `reply_dm`.
    /// Returns the new message's DeltaId hex. Reaches the relay once the object
    /// has other members — call OFF the Swift main thread.
    pub fn object_reply(
        &self,
        object_id: String,
        text: String,
        reply_to_author: String,
        reply_to_gen: u64,
        media: Option<MediaInput>,
    ) -> Result<String, FfiError> {
        self.point();
        let parent = parse_msgref(&reply_to_author, reply_to_gen)?;
        let m = media.as_ref().map(attach).transpose()?;
        Ok(hex::encode(block_on(Node::open()?.object_post_reply(
            &object_id,
            &text,
            Some(parent),
            m.as_ref(),
        ))?))
    }

    /// Mark every message WE hold from another member of this (forum) object READ —
    /// authors the ✓✓-blue receipts the senders see. Emitted when the user opens the
    /// conversation; idempotent (only un-acknowledged messages produce a receipt).
    /// Reaches the relay once the object has other members — call OFF the Swift main
    /// thread. Returns how many receipts were authored.
    pub fn object_mark_read(&self, object_id: String) -> Result<u32, FfiError> {
        self.point();
        Ok(block_on(Node::open()?.object_mark_read(&object_id))? as u32)
    }

    /// Author one Project delta (built via the `project_*` constructors) into a
    /// Project object. Sequenced vs commutative is decided by the op; same one-
    /// write-path semantics as `object_post` (local append + best-effort relay
    /// flush), so call OFF the Swift main thread. Returns the delta id (hex).
    pub fn project_author(
        &self,
        object_id: String,
        draft: delta::DeltaDraft,
    ) -> Result<String, FfiError> {
        self.point();
        // Group and Project both number ops from 0; a mis-kinded draft would
        // otherwise author an inert no-op and return a "success" delta id. Reject
        // it loudly (no silent no-op) before touching the log.
        if draft.kind != delta::Kind::Project {
            return Err(FfiError::Core(format!(
                "draft is a {:?} op, not a Project op",
                draft.kind
            )));
        }
        let op_id = draft.op_id;
        let args = delta::draft_args(&draft);
        let node = Node::open()?;
        Ok(block_on(node.apply(&object_id, op_id, args))?)
    }

    /// Author one Group delta (built via the `group_*` constructors) into a Group
    /// object — the reusable identity record. Same one-write-path semantics as
    /// `project_author` (local append + best-effort relay flush); call OFF the
    /// Swift main thread. Returns the delta id (hex).
    pub fn group_author(
        &self,
        object_id: String,
        draft: delta::DeltaDraft,
    ) -> Result<String, FfiError> {
        self.point();
        if draft.kind != delta::Kind::Group {
            return Err(FfiError::Core(format!(
                "draft is a {:?} op, not a Group op",
                draft.kind
            )));
        }
        let op_id = draft.op_id;
        let args = delta::draft_args(&draft);
        let node = Node::open()?;
        Ok(block_on(node.apply(&object_id, op_id, args))?)
    }

    /// MAY I MINT INTO THIS SPACE — a Channel, an Event, a Listing authored AS the
    /// Space rather than as me?
    ///
    /// The rule lives in `GroupRole::may_mint`, next to the role enum and pinned against
    /// the AUTHORITY of the ops that make a thing belong (`base.setPart` and
    /// `group.setAffiliation` 5, both Owner-only). Asking the core rather than deciding
    /// in the UI is what stops the `+` a member sees from disagreeing with what the fold
    /// will accept.
    ///
    /// False for an object that is not a Group, and for one we are not in.
    pub fn group_may_mint(&self, object_id: String) -> Result<bool, FfiError> {
        self.point();
        let node = Node::open()?;
        let me = node.id.identity_pk();
        let st = node.group_state(&object_id)?;
        // The owner is the owner whether or not a `setMemberRole` was ever authored —
        // the roster's creator, not a row in the role map.
        if node.object_owner(&object_id)?.map(hex::encode) == Some(hex::encode(me)) {
            return Ok(pacific_core::group::GroupRole::Owner.may_mint());
        }
        Ok(st
            .member_roles
            .get(&me)
            .copied()
            .is_some_and(|r| r.may_mint()))
    }

    /// The folded Group read-model — profile + presence + contact card + the roles
    /// projected onto the live MLS roster + credential handles. Synchronous read.
    pub fn group_view(&self, object_id: String) -> Result<GroupRow, FfiError> {
        self.point();
        Ok(group_row(Node::open()?.group_view(&object_id)?))
    }

    /// The LIFE ▸ GROUPS listing: every Group identity record we hold, folded.
    /// A group whose log will not fold is skipped by the core rather than
    /// blanking the list. Synchronous read (folds, no relay).
    pub fn groups(&self) -> Result<Vec<GroupListRow>, FfiError> {
        self.point();
        Ok(Node::open()?
            .group_rows()?
            .into_iter()
            .map(|(id, v, members, is_space)| GroupListRow {
                id,
                name: v.display_name,
                shape: v.shape.as_str().to_string(),
                members: members as u32,
                forums: v.parts.iter().filter(|(_, p)| p.role == pacific_core::node::ROOM).count() as u32,
                photo: v.card.photo,
                photo_mime: v.card.photo_mime,
                avatar_color: v.card.avatar_color,
                can_sync: v.can_sync,
                is_space,
            })
            .collect())
    }

    /// Appoint a member to an office. Owner-only and owner-sequenced; the holder must
    /// already be on the roster (an office is an overlay on membership, never a grant
    /// of it). Re-appointing the same office replaces the holder in place.
    pub fn group_set_office(
        &self,
        group_id: String,
        office: String,
        holder: String,
        status: String,
        at: i64,
    ) -> Result<String, FfiError> {
        self.point();
        let mut args = pacific_core::coordinator::Args::new();
        args.insert(
            "office".into(),
            pacific_core::coordinator::ArgVal::Text(office),
        );
        args.insert(
            "holder".into(),
            pacific_core::coordinator::ArgVal::Text(holder),
        );
        args.insert(
            "status".into(),
            pacific_core::coordinator::ArgVal::Text(status),
        );
        args.insert("at".into(), pacific_core::coordinator::ArgVal::Int(at));
        let node = Node::open()?;
        Ok(block_on(node.apply(
            &group_id,
            pacific_core::group::OP_SET_OFFICE,
            args,
        ))?)
    }

    /// Vacate an office — removes the appointment rather than marking it vacant.
    pub fn group_clear_office(&self, group_id: String, office: String) -> Result<String, FfiError> {
        self.point();
        let mut args = pacific_core::coordinator::Args::new();
        args.insert(
            "office".into(),
            pacific_core::coordinator::ArgVal::Text(office),
        );
        let node = Node::open()?;
        Ok(block_on(node.apply(
            &group_id,
            pacific_core::group::OP_CLEAR_OFFICE,
            args,
        ))?)
    }

    /// Mint a Forum FOR a Group and attach it in one call: a real `forum`-kind
    /// GroupObject plus both halves of `part_of` (role `room`), which every member folds. Current
    /// roster members with stocked prekeys are added to the room immediately
    /// (best-effort); the rest can join when theirs replenish. Owner-only.
    /// Reaches the relay — call OFF the Swift main thread.
    pub fn group_forum_new(
        &self,
        group_id: String,
        name: String,
        at_ms: i64,
    ) -> Result<String, FfiError> {
        self.point();
        Ok(block_on(
            Node::open()?.group_forum_new(&group_id, &name, at_ms),
        )?)
    }

    /// Attach an EXISTING Forum object to a Group (the set half of the forum
    /// edge). Owner-only. Reaches the relay — call OFF the Swift main thread.
    pub fn group_forum_attach(
        &self,
        group_id: String,
        forum_id: String,
        at_ms: i64,
    ) -> Result<String, FfiError> {
        self.point();
        Ok(block_on(
            Node::open()?.group_forum_attach(&group_id, &forum_id, at_ms),
        )?)
    }

    /// Detach a room from a Group (the clear half). The Forum object, its log
    /// and its roster are untouched. Owner-only. Reaches the relay — call OFF
    /// the Swift main thread.
    pub fn group_forum_detach(
        &self,
        group_id: String,
        forum_id: String,
    ) -> Result<String, FfiError> {
        self.point();
        Ok(block_on(
            Node::open()?.group_forum_detach(&group_id, &forum_id),
        )?)
    }

    /// The Group's rooms, folded from ITS log, attach order. Synchronous read.
    pub fn group_forums(&self, group_id: String) -> Result<Vec<GroupForumRow>, FfiError> {
        self.point();
        Ok(Node::open()?
            .group_forums(&group_id)?
            .into_iter()
            .map(|(id, name, at, held)| GroupForumRow { id, name, at, held })
            .collect())
    }

    /// Every forum→host edge this device can fold — the whole map in one call,
    /// so the CHAT list never does a per-row fold. Synchronous read.
    pub fn forum_hosts(&self) -> Result<Vec<ForumHostRow>, FfiError> {
        self.point();
        Ok(Node::open()?
            .forum_hosts()?
            .into_iter()
            .map(|(forum, group, group_name)| ForumHostRow {
                forum,
                group,
                group_name,
            })
            .collect())
    }

    // ---- FORUM ROOMS — a Channel's constituent Rooms (its tabs) ----

    /// Mint a Room FOR a Channel and attach it in one call: a real `forum`-kind
    /// GroupObject plus the `forum.setRoom` edge every member folds — the
    /// `group_forum_new` shape one level down. Current roster members with
    /// stocked prekeys are added to the room immediately (best-effort).
    /// Owner-only. Reaches the relay — call OFF the Swift main thread.
    pub fn forum_room_new(
        &self,
        forum_id: String,
        name: String,
        at_ms: i64,
    ) -> Result<String, FfiError> {
        self.point();
        Ok(block_on(
            Node::open()?.forum_room_new(&forum_id, &name, at_ms),
        )?)
    }

    /// Attach an EXISTING Forum object to a Channel as a Room (the set half of
    /// the room edge). Owner-only. Reaches the relay — call OFF the Swift main
    /// thread.
    pub fn forum_room_attach(
        &self,
        forum_id: String,
        room_id: String,
        at_ms: i64,
    ) -> Result<String, FfiError> {
        self.point();
        Ok(block_on(
            Node::open()?.forum_room_attach(&forum_id, &room_id, at_ms),
        )?)
    }

    /// Detach a Room from its Channel (the clear half). The Room object, its
    /// log and its roster are untouched. Owner-only. Reaches the relay — call
    /// OFF the Swift main thread.
    pub fn forum_room_detach(&self, forum_id: String, room_id: String) -> Result<String, FfiError> {
        self.point();
        Ok(block_on(
            Node::open()?.forum_room_detach(&forum_id, &room_id),
        )?)
    }

    /// The Channel's rooms, folded from ITS log, attach order. Synchronous read.
    pub fn forum_rooms(&self, forum_id: String) -> Result<Vec<ForumRoomRow>, FfiError> {
        self.point();
        Ok(Node::open()?
            .forum_rooms(&forum_id)?
            .into_iter()
            .map(|(id, name, at, held)| ForumRoomRow { id, name, at, held })
            .collect())
    }

    /// Every room→host edge this device can fold — the whole map in one call,
    /// so the CHAT list never does a per-row fold. Synchronous read.
    pub fn room_hosts(&self) -> Result<Vec<RoomHostRow>, FfiError> {
        self.point();
        Ok(Node::open()?
            .room_hosts()?
            .into_iter()
            .map(|(room, forum, forum_name)| RoomHostRow {
                room,
                forum,
                forum_name,
            })
            .collect())
    }

    // ---- MY PROFILE and its fan-out to every connection ----
    //
    // The profile used to live half in the core (`display_name`, fixed at first
    // mint and never editable) and half in iOS UserDefaults (the avatar), which is
    // why it could never be published: half of it had no way to cross the FFI. It
    // lives in the core now, and every edit fans out as a signed Delta.

    /// My own profile — name, shape and the full card. Reads local state only.
    pub fn my_profile(&self) -> Result<ProfileRow, FfiError> {
        self.point();
        let (display_name, shape, card) = Node::open()?.my_profile()?;
        Ok(ProfileRow {
            display_name,
            shape: shape.as_str().to_string(),
            card: profile_card_row(card),
            gen: 0,
        })
    }

    /// Replace my profile and TRANSMIT it to every connection as a signed
    /// `contact.publishProfile` Delta. Returns how many connections received one
    /// (0 when nothing changed — the call is idempotent).
    ///
    /// The local write always lands; publishing is what can partially fail, and it
    /// degrades to "queued in that connection's outbox and retried on the next
    /// sync", never to "lost". Reaches the relay — call OFF the Swift main thread.
    pub fn set_my_profile(
        &self,
        display_name: String,
        shape: String,
        card: ProfileCard,
    ) -> Result<u32, FfiError> {
        self.point();
        let shape = pacific_core::group::GroupShape::parse(&shape)
            .map_err(|_| FfiError::Core(format!("bad shape '{shape}'")))?;
        let node = Node::open()?;
        let n = block_on(node.set_my_profile(&display_name, shape, &profile_card_core(card)))?;
        Ok(n as u32)
    }

    /// Re-publish my current profile. `force` resends even to connections already
    /// holding it; otherwise only stale ones are told. The manual twin of the
    /// reconciliation every sync runs. Reaches the relay — call OFF the main thread.
    pub fn publish_profile(&self, force: bool) -> Result<u32, FfiError> {
        self.point();
        let node = Node::open()?;
        Ok(block_on(node.publish_profile(force))? as u32)
    }

    /// The profile `peer_id` published to us, or None if they have not published
    /// one (an older build, or a pairing whose first sync has not landed). None is
    /// NOT an empty profile: the caller falls back to the pairing-bundle name
    /// rather than rendering a blank card. Synchronous read (a fold, no relay).
    pub fn peer_profile(&self, peer_id: String) -> Result<Option<ProfileRow>, FfiError> {
        self.point();
        let peer = pacific_core::identity::parse_identity_key(&peer_id)?;
        Ok(Node::open()?.peer_profile(&peer)?.map(|p| ProfileRow {
            display_name: p.display_name,
            shape: p.shape.as_str().to_string(),
            card: profile_card_row(p.card),
            gen: p.gen,
        }))
    }

    /// Import a vCard (.vcf) string, which may hold MANY cards (an address-book
    /// export). Each card mints or re-homes its own Group; returns one object id
    /// per card, in file order. Loud on a non-Group KIND or missing FN in any
    /// card. Call OFF the main thread (best-effort relay flush).
    pub fn group_import_vcard(&self, vcf_text: String) -> Result<Vec<String>, FfiError> {
        self.point();
        let node = Node::open()?;
        Ok(block_on(node.group_import_vcard(&vcf_text))?)
    }

    /// Export a Group as a vCard 4.0 string (MEMBER derived from the roster;
    /// credentials never serialised). Synchronous.
    pub fn group_export_vcard(&self, object_id: String) -> Result<String, FfiError> {
        self.point();
        Ok(Node::open()?.group_export_vcard(&object_id)?)
    }

    /// The folded board of a Project object — one line per live timeline item.
    /// Deterministic text dump, kept for the CLI and the two-sim board diff; the
    /// UI uses the STRUCTURED `project_items` below.
    pub fn project_board(&self, object_id: String) -> Result<Vec<String>, FfiError> {
        self.point();
        Ok(Node::open()?.project_board(&object_id)?)
    }

    /// Structured timeline rows for the Work-tab Gantt (the typed projection the UI
    /// binds to — no string parsing). Synchronous read (no relay).
    pub fn project_items(&self, object_id: String) -> Result<Vec<ProjectItemRow>, FfiError> {
        self.point();
        let st = Node::open()?.project_state(&object_id)?;
        Ok(st.board().into_iter().map(item_row).collect())
    }

    /// The Project's deliverables (child-Project subscriptions), collapsed-view rows.
    pub fn project_deliverables(&self, object_id: String) -> Result<Vec<DeliverableRow>, FfiError> {
        self.point();
        let st = Node::open()?.project_state(&object_id)?;
        Ok(st.deliverables().into_iter().map(deliverable_row).collect())
    }

    /// The Project's dependency edges — what the UI lists and removes; the derived
    /// `blocked` state comes from these unresolved edges.
    pub fn project_dependencies(&self, object_id: String) -> Result<Vec<DependencyRow>, FfiError> {
        self.point();
        Ok(Node::open()?
            .project_dependencies(&object_id)?
            .into_iter()
            .map(|(edge_id, from, to, kind)| DependencyRow {
                edge_id,
                from,
                to,
                kind,
            })
            .collect())
    }

    /// The Objectives spine of a Project (Tab 1): headline, goal, participants+roles,
    /// external stakeholders, key locations, KPIs — what the timeline is derived from.
    pub fn project_objectives(&self, object_id: String) -> Result<ObjectivesRow, FfiError> {
        self.point();
        let st = Node::open()?.project_state(&object_id)?;
        Ok(ObjectivesRow {
            headline: st.headline.clone(),
            goal: st.goal.clone(),
            participants: st
                .participants
                .iter()
                .map(|(m, r)| ParticipantRow {
                    member: hex::encode(m),
                    role: r.as_str().to_string(),
                })
                .collect(),
            stakeholders: st
                .external_stakeholders
                .iter()
                .map(|(id, s)| StakeholderRow {
                    id: id.clone(),
                    name: s.name.clone(),
                    note: s.note.clone(),
                })
                .collect(),
            locations: st
                .key_locations
                .iter()
                .map(|(id, n)| LocationRow {
                    id: id.clone(),
                    name: n.clone(),
                })
                .collect(),
            kpis: st
                .kpis
                .iter()
                .map(|(id, k)| KpiRow {
                    id: id.clone(),
                    label: k.label.clone(),
                    target: k.target.clone(),
                })
                .collect(),
        })
    }

    /// The Project's linked ChatRoom (Tab 3) — the target of its first `forum`-kind
    /// subscription edge, or nil if none is linked yet.
    pub fn project_chatroom(&self, object_id: String) -> Result<Option<String>, FfiError> {
        self.point();
        let st = Node::open()?.project_state(&object_id)?;
        Ok(st
            .subscriptions
            .values()
            .find(|s| s.kind == pacific_core::project::SubKind::Forum)
            .map(|s| s.target.clone()))
    }

    /// The Project card header (title/status + DERIVED piece count).
    pub fn project_header(&self, object_id: String) -> Result<ProjectHeaderRow, FfiError> {
        self.point();
        let st = Node::open()?.project_state(&object_id)?;
        Ok(ProjectHeaderRow {
            title: st.title.clone(),
            status: st.status.as_str().to_string(),
            piece_count: st.piece_count() as u32,
        })
    }

    /// This device authors its half of the AW26 Project demo through real ops
    /// (owner-sequenced structure if we are the owner, plus our own progress).
    /// Reaches the relay — call OFF the Swift main thread. Mirrors `seed_dm_demo`.
    pub fn seed_project_demo(&self, object_id: String) -> Result<bool, FfiError> {
        self.point();
        let node = Node::open()?;
        Ok(block_on(node.seed_project_demo(&object_id))?)
    }

    // ---- the 1:1 direct-message surface ----
    // A connection is a `kind='connection'` MLS group of two, formed by
    // pair_scan / pair_accept. `objects()` deliberately excludes these so the
    // Chats/Work lists stay clean; the DM surface below is how the app reaches
    // them. `peer_id` is the peer's `space1…` space id (as `connections()`
    // returns and `pair_accept` takes).

    /// The user's 1:1 connections. `connected` is true once the double-opt-in is
    /// done; a still-pending peer surfaces with `connected=false` (not hidden).
    /// `display_name` falls back to a short prefix of the space id when the
    /// directory has no name yet.
    pub fn connections(&self) -> Result<Vec<ConnectionRow>, FfiError> {
        self.point();
        Ok(Node::open()?
            .connections()?
            .into_iter()
            .map(|(pk, name, connected)| {
                let peer_id = space_id_of(&pk);
                let display_name = if name.is_empty() {
                    // no directory name yet — a short, stable handle from the id.
                    format!("{}…", &peer_id[..14.min(peer_id.len())])
                } else {
                    name
                };
                ConnectionRow {
                    peer_id,
                    display_name,
                    connected,
                }
            })
            .collect())
    }

    /// Send a 1:1 message to `peer_id`. Resolves the peer's connection group and
    /// authors one post through the single write path (local append + best-effort
    /// relay flush — identical semantics to `object_post`). Returns the delta id.
    /// Loud if the peer is unknown or the double-opt-in isn't complete (the peer
    /// must be Connected). Reaches the relay, so call OFF the Swift main thread.
    pub fn dm_post(&self, peer_id: String, text: String) -> Result<String, FfiError> {
        self.point();
        let peer = pacific_core::identity::parse_identity_key(&peer_id)?;
        let node = Node::open()?;
        Ok(hex::encode(block_on(node.author_post(&peer, &text))?))
    }

    /// The folded transcript of the 1:1 conversation with `peer_id` — same
    /// deterministic, pre-formatted style as `object_transcript`. Loud if there
    /// is no connection group for this peer.
    pub fn dm_transcript(&self, peer_id: String) -> Result<Vec<String>, FfiError> {
        self.point();
        let peer = pacific_core::identity::parse_identity_key(&peer_id)?;
        Ok(Node::open()?.dm_transcript(&peer)?)
    }

    /// The STRUCTURED 1:1 transcript: real messages with timestamps, reply
    /// linkage and emoji-reaction groups — everything the modern-chat UI binds
    /// to. Same fold as `dm_transcript`, richer projection. Loud if there is no
    /// connection group for this peer.
    pub fn dm_messages(&self, peer_id: String) -> Result<Vec<ChatMsg>, FfiError> {
        self.point();
        let peer = pacific_core::identity::parse_identity_key(&peer_id)?;
        let node = Node::open()?;
        let me = node.id.identity_pk();
        Ok(to_chat_msgs(node.dm_detailed(&peer)?, &me))
    }

    /// Toggle THIS device's emoji reaction on a message in the 1:1 with `peer_id`.
    /// `target_author`/`target_gen` are the `ChatMsg.author`/`ChatMsg.gen` of the
    /// message being reacted to. `active=true` sets/replaces our reaction with
    /// `emoji` (one per reactor, LWW — like Signal), `false` clears it. A real
    /// forum.react Delta over the relay — call OFF the Swift main thread.
    pub fn react_dm(
        &self,
        peer_id: String,
        target_author: String,
        target_gen: u64,
        emoji: String,
        active: bool,
    ) -> Result<(), FfiError> {
        self.point();
        let peer = pacific_core::identity::parse_identity_key(&peer_id)?;
        let target = parse_msgref(&target_author, target_gen)?;
        block_on(Node::open()?.react_dm(&peer, target, &emoji, active))?;
        Ok(())
    }

    /// Post a reply to a message in the 1:1 with `peer_id`. `reply_to_author`/
    /// `reply_to_gen` are the `ChatMsg.author`/`ChatMsg.gen` of the message being
    /// quoted. A real forum.post Delta carrying the parent ref — call OFF the
    /// Swift main thread. Returns the new message's DeltaId hex.
    pub fn reply_dm(
        &self,
        peer_id: String,
        text: String,
        reply_to_author: String,
        reply_to_gen: u64,
        media: Option<MediaInput>,
    ) -> Result<String, FfiError> {
        self.point();
        let peer = pacific_core::identity::parse_identity_key(&peer_id)?;
        let parent = parse_msgref(&reply_to_author, reply_to_gen)?;
        let m = media.as_ref().map(attach).transpose()?;
        Ok(hex::encode(block_on(Node::open()?.author_post_reply(
            &peer,
            &text,
            Some(parent),
            m.as_ref(),
        ))?))
    }

    /// Mark every message WE hold from `peer_id` in this 1:1 READ — authors the
    /// ✓✓-blue receipts the peer sees. Emitted when the user opens the conversation;
    /// idempotent. Reaches the relay — call OFF the Swift main thread. Returns how
    /// many receipts were authored. Loud if the peer is unknown / not connected.
    pub fn dm_mark_read(&self, peer_id: String) -> Result<u32, FfiError> {
        self.point();
        let peer = pacific_core::identity::parse_identity_key(&peer_id)?;
        Ok(block_on(Node::open()?.dm_mark_read(&peer))? as u32)
    }

    /// Seed the DEMO conversation through REAL ops: this device authors only the
    /// scripted lines belonging to its own identity (resolved from its display
    /// name) and drains the peer's over the relay. What the UI then shows is the
    /// folded residue of real Deltas — no hardcoded transcript, no faked author.
    /// Returns true if it ran (this identity is a scripted actor), false if not.
    /// RUN ONCE per install (posts carry a per-author generation counter); the
    /// caller guards it. Reaches the relay — call OFF the Swift main thread.
    pub fn seed_dm_demo(&self, peer_id: String) -> Result<bool, FfiError> {
        self.point();
        let peer = pacific_core::identity::parse_identity_key(&peer_id)?;
        let node = Node::open()?;
        let my_name = node.display_name()?;
        let Some(actor) = pacific_core::demo::actor_for_display_name(&my_name) else {
            return Ok(false);
        };
        block_on(node.seed_dm_script(&peer, &pacific_core::demo::NYE_HARVEY, actor))?;
        Ok(true)
    }

    // ---- the MLS transport surface: pairing + N-member add + relay sync ----
    // These reach the relay, so they block on pacific-core's async ops via a local
    // current-thread runtime (its futures are !Send, same as object_post). Call
    // them OFF the Swift main thread. Relays: pass default_relay_url() unless
    // overriding.

    /// Scan a peer's pairing bundle: form the 2-party connection and seal the
    /// Welcome to the peer over the relay. Returns the peer's space id.
    pub fn pair_scan(&self, bundle: String) -> Result<String, FfiError> {
        self.point();
        let node = Node::open()?;
        Ok(space_id_of(&block_on(node.pair_scan(&bundle))?))
    }

    /// Like `pair_scan`, but the 2-member tether carries `kind` instead of
    /// "connection" — e.g. "group-tether", the delegate↔delegate channel between
    /// two federated Groups. Returns the peer's space id.
    pub fn pair_scan_kind(&self, bundle: String, kind: String) -> Result<String, FfiError> {
        self.point();
        let node = Node::open()?;
        Ok(space_id_of(&block_on(node.pair_scan_kind(&bundle, &kind))?))
    }

    /// Accept a pending connection (double-opt-in): mark the peer Connected.
    /// `peer` is the peer's space id (as returned by pair_scan / a pending intro).
    pub fn pair_accept(&self, peer: String) -> Result<(), FfiError> {
        self.point();
        let peer_id = pacific_core::identity::parse_identity_key(&peer)?;
        // Async in core since accepting also stocks our prekeys into the channel
        // (the half `pair_scan` never did for this side). The exported signature is
        // unchanged, so the uniffi checksum and the generated bindings still match.
        block_on(Node::open()?.pair_accept(&peer_id))?;
        Ok(())
    }

    /// Owner-add a member (by their pairing bundle) into an EXISTING GroupObject:
    /// distributes the MLS Commit to existing members and the Welcome to the
    /// newcomer over the relay. Returns the new member's space id.
    pub fn group_add_member(&self, object_id: String, bundle: String) -> Result<String, FfiError> {
        self.point();
        let node = Node::open()?;
        Ok(space_id_of(&block_on(
            node.group_add_member(&object_id, &bundle),
        )?))
    }

    // ---- membership through MLS (docs/membership-through-mls.md) ----------------
    // The roster IS the access control, so leaving, removing and handing over are MLS
    // commits — and core writes the membership RECORD alongside each, never the app.
    // All three reach the relay → call OFF the Swift main thread. A refusal is a normal
    // throw whose message says why, in words meant for a person:
    //   "the owner must hand the object over before leaving"
    //   "you are the only one here — nobody could complete your leave; delete the object instead"
    //   "only the owner may remove someone", "you are no longer a member of this group", …

    /// LEAVE (§6): record the departure and propose this person's removal — every
    /// device of theirs. Another member completes it on their next sync; this device
    /// learns it has left when that commit arrives. Idempotent while a leave is in flight.
    pub fn group_leave(&self, object_id: String) -> Result<(), FfiError> {
        self.point();
        let node = Node::open()?;
        block_on(node.group_leave(&object_id))?;
        Ok(())
    }

    /// REMOVE (§5): the owner removes every device of `member_id`. `reason`, when
    /// given, is one of `requested | lost_device | inactive | conduct | other`.
    pub fn group_remove_member(
        &self,
        object_id: String,
        member_id: String,
        reason: Option<String>,
    ) -> Result<(), FfiError> {
        self.point();
        let node = Node::open()?;
        block_on(node.group_remove_member(&object_id, &member_id, reason.as_deref()))?;
        Ok(())
    }

    /// HAND OVER (§9): the owner makes `member_id` the owner. Required before an owner
    /// may leave.
    pub fn group_hand_over(&self, object_id: String, member_id: String) -> Result<(), FfiError> {
        self.point();
        let node = Node::open()?;
        block_on(node.group_hand_over(&object_id, &member_id))?;
        Ok(())
    }

    /// Is a leave in flight for this object — proposed, not yet completed (§6)?
    pub fn is_leaving(&self, object_id: String) -> Result<bool, FfiError> {
        self.point();
        Ok(Node::open()?.is_leaving(&object_id)?)
    }

    /// Has a commit removed this device from the object (§8.3)? Its history stays
    /// readable; nothing more can be sent or received in it.
    pub fn is_departed(&self, object_id: String) -> Result<bool, FfiError> {
        self.point();
        Ok(Node::open()?.is_departed(&object_id)?)
    }

    /// How the membership RECORDS and the MLS tree disagree (§10.3), or `None` when
    /// they agree. Group-typed objects only.
    pub fn membership_divergence(&self, object_id: String) -> Result<Option<MembershipDivergence>, FfiError> {
        self.point();
        Ok(Node::open()?
            .membership_divergence(&object_id)?
            .map(|d| MembershipDivergence {
                unrecorded: d.unrecorded,
                pending_removal: d.pending_removal,
                phantom: d.phantom,
            }))
    }

    /// KNOCK ON A JOIN CARD (§8.5): parse it, remember the owner it names, knock. A
    /// Welcome whose group names a different owner is then refused. Reaches the relay.
    pub fn place_knock_card(&self, card: String) -> Result<(), FfiError> {
        self.point();
        let node = Node::open()?;
        block_on(node.place_knock_card(&card))?;
        Ok(())
    }

    /// Add an EXISTING contact (by their space id) to an object by CONSUMING one of
    /// their stocked prekeys — no fresh code exchange. This is the tap-to-add path.
    /// Reaches the relay → call OFF the Swift main thread. Errors (e.g. "no usable
    /// prekey — ask them to replenish") surface as a normal throw.
    pub fn add_contact_to_object(
        &self,
        object_id: String,
        peer_id: String,
    ) -> Result<String, FfiError> {
        self.point();
        let peer = pacific_core::identity::parse_identity_key(&peer_id)?;
        let node = Node::open()?;
        Ok(space_id_of(&block_on(
            node.add_contact_to_object(&object_id, &peer),
        )?))
    }

    /// Point this device's messaging at a transport set — a comma-separated spec such as
    /// `"wss://arc.example/v1/relay,mesh:"`. Persisted by the core.
    ///
    /// This replaces the `relayUrl` that used to ride on every call. That shape meant the app
    /// re-decided transport per operation and nothing could ever add a second one; the mesh was
    /// unreachable for exactly that reason. Now there is one decision, made here, and the router
    /// fans out across whatever is in the set.
    pub fn set_routes(&self, spec: String) -> Result<(), FfiError> {
        let routes = pacific_core::router::Routes::parse(&spec);
        if routes.is_empty() {
            return Err(FfiError::Core(
                "a transport set needs at least one route (e.g. \"wss://…,mesh:\")".into(),
            ));
        }
        self.point();
        let mut node = Node::open()?;
        node.set_routes(routes)?;
        Ok(())
    }

    /// The transport set this device is using, as a spec string.
    pub fn routes(&self) -> Result<String, FfiError> {
        self.point();
        Ok(Node::open()?.routes().to_spec())
    }

    /// Sync with the relay: drain pending Welcomes (join), process membership
    /// commits (epoch advance), decrypt + fold inbound deltas. Returns the folded
    /// transcript lines across all synced objects.
    pub fn sync(&self) -> Result<Vec<String>, FfiError> {
        self.point();
        let node = Node::open()?;
        Ok(block_on(node.sync_once())?)
    }
}

/// A `space1<hex>` space id from a raw identity pubkey.
fn space_id_of(pk: &[u8; 32]) -> String {
    format!("{}{}", pacific_core::identity::IDENTITY_KEY_PREFIX, hex::encode(pk))
}

/// Parse a `ChatMsg.author` (64-char identity hex) + gen back into the core's
/// `MsgRef`. Loud on malformed hex — a reaction/reply target must be a real ref.
fn parse_msgref(author_hex: &str, gen: u64) -> Result<pacific_core::coordinator::MsgRef, FfiError> {
    let bytes = hex::decode(author_hex)
        .map_err(|e| FfiError::Core(format!("bad target author hex: {e}")))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| FfiError::Core("target author must be 32 bytes".into()))?;
    Ok((arr, gen))
}

/// Project the core's folded `ForumMessage`s into the FFI `ChatMsg` records the
/// UI binds to: stamp `mine` against this device's key, resolve each reply's
/// parent text from the same transcript, and mark our own emoji reaction.
fn to_chat_msgs(msgs: Vec<pacific_core::coordinator::ForumMessage>, me: &[u8; 32]) -> Vec<ChatMsg> {
    // Index text by (author, gen) so a reply can show a preview of its parent.
    let by_ref: std::collections::HashMap<pacific_core::coordinator::MsgRef, String> = msgs
        .iter()
        .map(|m| ((m.author, m.gen), m.text.clone()))
        .collect();
    msgs.into_iter()
        .map(|m| {
            let (reply_to_author, reply_to_gen, reply_preview) = match m.reply_to {
                Some(r) => (
                    hex::encode(r.0),
                    r.1,
                    by_ref.get(&r).cloned().unwrap_or_default(),
                ),
                None => (String::new(), 0, String::new()),
            };
            let reactions = m
                .reactions
                .into_iter()
                .map(|(emoji, reactors)| ChatReaction {
                    count: reactors.len() as u32,
                    mine: reactors.iter().any(|r| r == me),
                    emoji,
                })
                .collect();
            ChatMsg {
                author: hex::encode(m.author),
                gen: m.gen,
                text: m.text,
                ts: m.ts,
                mine: &m.author == me,
                reply_to_author,
                reply_to_gen,
                reply_preview,
                reactions,
                receipt: m.receipt,
                up: m.up.len() as u32,
                down: m.down.len() as u32,
                my_vote: if m.up.iter().any(|v| v == me) {
                    1
                } else if m.down.iter().any(|v| v == me) {
                    -1
                } else {
                    0
                },
            }
        })
        .collect()
}

/// Project the core's folded thread tree into the FFI `ForumThreadRow`s the
/// ChatRoom UI binds to. The post projection reuses `to_chat_msgs` (order- and
/// length-preserving, and the whole tree is present so every reply preview
/// resolves), then re-attaches each node's `depth`/`descendants`.
fn to_thread_rows(
    nodes: Vec<pacific_core::coordinator::ThreadNode>,
    me: &[u8; 32],
) -> Vec<ForumThreadRow> {
    let msgs: Vec<pacific_core::coordinator::ForumMessage> =
        nodes.iter().map(|n| n.msg.clone()).collect();
    to_chat_msgs(msgs, me)
        .into_iter()
        .zip(nodes)
        .map(|(post, n)| ForumThreadRow {
            post,
            depth: n.depth,
            reply_count: n.descendants,
        })
        .collect()
}

/// The canonical production relay URL — **Semaphore**, the blind public tier at
/// `arc.wallflowers.io/v1/relay` — the Arc's single public origin. The app uses this as
/// its default relay unless the user overrides it.
#[uniffi::export]
pub fn default_relay_url() -> String {
    pacific_core::paths::DEFAULT_RELAY_URL.to_string()
}

/// Whether `url` is a relay this build has retired — the app calls this before
/// honouring a persisted relay, so a device that stored a now-dead host migrates
/// to [`default_relay_url`] instead of quietly talking to nothing.
#[uniffi::export]
pub fn is_retired_relay_url(url: String) -> bool {
    pacific_core::paths::is_retired_relay_url(&url)
}

/// The RAIL — the money tier, now behind the same origin (`arc.wallflowers.io`). Separate
/// from the relay by design: the rail holds secrets and settles payments, the relay
/// holds nothing. Routes live under `origin/v1/box/…`.
//
// NB: keep the two-character sequence "/" + "*" out of this doc comment. UniFFI
// copies doc comments verbatim into Swift `/** … */` blocks, and Swift block
// comments NEST — so a bare `/*` in the text opens a nested comment that the
// closing `*/` only half-closes, and the generated bindings fail to compile with
// "unterminated '/*' comment" at end of file.
#[uniffi::export]
pub fn default_rail_url() -> String {
    "https://arc.wallflowers.io".to_string()
}

/// The default Arc URL — the server a community runs for its members (the blind
/// relay, membership, push and the box office behind one gateway), used when no Arc
/// is configured. Mirror of `pacific_core::paths::DEFAULT_ARC_URL`; override with `arc set`.
#[uniffi::export]
pub fn default_arc_url() -> String {
    pacific_core::paths::DEFAULT_ARC_URL.to_string()
}

#[cfg(test)]
mod arc_credential_tests {
    use super::*;

    /// The membership credential is verified by code that lives in ANOTHER repository
    /// (arc `planes/membership/src/console.rs::verify_member`), so nothing in this workspace
    /// would catch a drift in its wire format — a swapped field, a missing newline, the wrong
    /// hex — until a device silently 401s against a live Arc. This test transcribes that
    /// verifier's checks verbatim and runs a real minted credential through them, so the two
    /// halves are pinned together here rather than in production.
    ///
    /// If this fails after an edit to `arc_member_credential`, the credential is wrong.
    /// If it fails after an edit to `verify_member`, this transcription is stale — update both.
    #[test]
    fn minted_credential_satisfies_verify_member() {
        let dir = std::env::temp_dir().join(format!("pacific-cred-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp state dir");

        let core = Core::new(dir.to_string_lossy().to_string());
        // The app's order: its Arc before its identity (O-73). Never dialled here.
        std::env::set_var("PACIFIC_STATE_DIR", &dir);
        pacific_core::node::set_default_arc("ws://127.0.0.1:1").expect("the device's Arc");
        core.init_identity("Credential Test".into())
            .expect("mint identity");

        // The Arc we are signing FOR — any valid space id; the audience is what binds the
        // credential to one Arc, so the test uses a different identity than the signer's.
        let arc_identity_key =
            "ed25519:1ef175203b1b0c160f4f0daf31fe42a3ffb14e737e80089adb542f26e07677dc".to_string();
        let cred = core
            .arc_member_credential(arc_identity_key.clone())
            .expect("mint credential");

        // ---- verify_member, transcribed ----------------------------------------------
        // Accepts a bare token or a `Bearer <token>` — it takes the last space-separated part.
        let token = cred.trim().rsplit(' ').next().unwrap_or("").trim();
        let mut parts = token.splitn(3, ':');
        let (Some(identity_key), Some(ts_s), Some(sig_hex)) =
            (parts.next(), parts.next(), parts.next())
        else {
            panic!("credential is not <identity_key>:<ts_ms>:<sig_hex> — got {cred:?}");
        };
        let ts: i64 = ts_s.parse().expect("timestamp is not an integer");
        let sig_vec = hex::decode(sig_hex).expect("signature is not hex");
        let sig = <[u8; 64]>::try_from(sig_vec.as_slice()).expect("signature is not 64 bytes");
        let member_pk =
            pacific_core::identity::parse_identity_key(identity_key).expect("space id does not parse");

        // Fresh? The Arc allows ±5min; a credential minted just now must be well inside it.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        assert!(
            (now - ts).abs() < 60_000,
            "timestamp is not fresh: minted {ts}, now {now} — wrong unit?"
        );

        // Signed by that identity, bound to THIS Arc? Field order and the newlines matter.
        // BOTH ids are BARE HEX inside the payload: the token is colon-delimited and the
        // `ed25519:` rendering carries a colon, so machine formats take the hex (12 Aug).
        let arc_hex = hex::encode(
            pacific_core::identity::parse_identity_key(&arc_identity_key).unwrap(),
        );
        let payload = format!("pacific-arc-member:v1\n{arc_hex}\n{identity_key}\n{ts}");
        pacific_core::identity::verify_sig(&member_pk, payload.as_bytes(), &sig)
            .expect("signature does not verify — payload construction disagrees with the Arc");

        // The audience must be the ARC, not us: a credential that verified under our own
        // space id would be replayable at any Arc.
        assert_ne!(identity_key, arc_hex, "signer and audience must differ");

        // And it must NOT verify for a different Arc.
        let other = format!(
            "pacific-arc-member:v1\n{}\n{identity_key}\n{ts}",
            "00".repeat(32)
        );
        assert!(
            pacific_core::identity::verify_sig(&member_pk, other.as_bytes(), &sig).is_err(),
            "credential verified for the WRONG Arc — the audience is not bound into the signature"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
