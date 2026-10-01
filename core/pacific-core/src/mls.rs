//! Thin, sync wrappers over mls-rs 0.55.2 for the M1 pipeline.
//!
//! Persistence is the corrected, REAL path: the `Client` is rebuilt each process
//! with a SQLite `GroupStateStorage` + `KeyPackageStorage` ([`crate::mls_store`]),
//! and cross-process continuity is `Group::write_to_storage()` +
//! `Client::load_group(group_id)`. There is no `group.snapshot()` (it does not
//! exist in 0.55.2). One cipher suite: CURVE25519_AES128.

use std::collections::{BTreeMap, BTreeSet};

use mls_rs::client_builder::{
    BaseConfig, WithCryptoProvider, WithGroupStateStorage, WithIdentityProvider, WithKeyPackageRepo,
    WithMlsRules,
};
use mls_rs::{GroupStateStorage, KeyPackageStorage};
use mls_rs::crypto::{SignaturePublicKey, SignatureSecretKey};
use mls_rs::error::MlsError;
use mls_rs::extension::ExtensionType;
use mls_rs::group::proposal::{ExternalInit, PreSharedKeyProposal, ReInitProposal, RemoveProposal};
use mls_rs::group::{
    CommitEffect, Group as MlsGroup, GroupContext, ReceivedMessage, Roster, Sender,
};
use mls_rs::identity::basic::BasicCredential;
pub use mls_rs::identity::SigningIdentity;
use mls_rs::mls_rules::{
    CommitDirection, CommitOptions, CommitSource, EncryptionOptions, ProposalBundle, ProposalInfo,
};
use mls_rs_core::error::IntoAnyError;
use mls_rs_core::identity::{CredentialType, IdentityProvider, MemberValidationContext};
use mls_rs_core::time::MlsTime;
use mls_rs::{
    CipherSuite, CipherSuiteProvider, Client as MlsClient, CryptoProvider, Extension,
    ExtensionList, MlsMessage, MlsRules,
};
use mls_rs_crypto_rustcrypto::RustCryptoProvider;

use crate::CoreError;

pub const CIPHERSUITE: CipherSuite = CipherSuite::CURVE25519_AES128;

/// Re-export so node.rs can name the secret-key type without a deep import path.
pub use mls_rs::crypto::SignatureSecretKey as SecretKey;

// ── THE IDENTITY PROVIDER ────────────────────────────────────────────────────
// Lifted verbatim from `tests/m24_leaf_pool.rs`, where it was written as an
// experiment and passed: `basic_identity_refuses_one_persons_second_leaf` is the
// control, `a_per_leaf_identity_provider_admits_two_leaves_of_one_person` is the
// result. This is that result, promoted from a test file to the protocol.

/// Pacific's `IdentityProvider`: a leaf's MLS identity is its credential AND its
/// own signature key, so two leaves of ONE PERSON are distinct to mls-rs's tree
/// index while still carrying the same `cred_id` to everything above MLS.
///
/// WHY THIS IS NOT `BasicIdentityProvider`. RFC 9420 §7.3 requires exactly two
/// fields to be unique among members — `signature_key` and `encryption_key` —
/// and NOT the credential. Two leaves carrying one person's credential are legal
/// MLS so long as each brings its own keys. mls-rs refuses them anyway, because
/// `tree_index.rs` keeps a THIRD uniqueness map keyed on whatever the configured
/// `IdentityProvider::identity()` returns, and `BasicIdentityProvider` returns
/// the credential identifier verbatim. That is library policy layered on the
/// protocol, and the provider is ours to choose. Choosing this one is what makes
/// "one person, two devices, both authoring" possible without a wire change:
/// `cred_id` stays the person's Ed25519 identity pubkey, so the membership fold,
/// authorship, authority, `space_id` and the archive's identity edges are all
/// untouched — see [`roster_identities`], which still reads the credential.
///
/// THE CONSEQUENCE, stated here because it is easy to miss: a group's ratchet
/// tree may now hold MORE LEAVES THAN IT HOLDS PEOPLE. Anything that asks "how
/// many people" must deduplicate on `cred_id` (the `group_members` primary key
/// does), and anything that asks "do I need the relay" must count leaves
/// (`Directory::group_leaf_count`).
#[derive(Debug, Clone, Copy)]
pub struct PerLeafIdentity;

/// Why [`PerLeafIdentity`] refuses a leaf.
#[derive(Debug, PartialEq, Eq)]
pub enum LeafRefused {
    /// Its credential is not a `BasicCredential`. Pacific mints no other kind.
    NotBasic,
    /// Its signature key is not a usable Ed25519 point: a small-order key, for which
    /// R = the identity and s = 0 verify over every message, and mls-rs's provider
    /// verifies non-strictly (NC-46). Only its holder is exposed, since a Delta still
    /// needs its identity's strict signature: this is depth (Software Security).
    WeakKey,
}

impl std::fmt::Display for LeafRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LeafRefused::NotBasic => write!(f, "credential is not a BasicCredential"),
            LeafRefused::WeakKey => write!(f, "the leaf's signature key is a weak Ed25519 key"),
        }
    }
}
impl std::error::Error for LeafRefused {}
impl IntoAnyError for LeafRefused {}

/// The credential identifier — the person's 32-byte Ed25519 identity pubkey.
/// Shared by `identity` and `valid_successor`, which is what keeps the two rules
/// in agreement about what "the same person" means.
fn leaf_cred_id(sid: &SigningIdentity) -> Result<Vec<u8>, LeafRefused> {
    sid.credential
        .as_basic()
        .map(|b| b.identifier.to_vec())
        .ok_or(LeafRefused::NotBasic)
}

/// Every leaf's signature key is a strong Ed25519 point ([`CIPHERSUITE`] signs with
/// Ed25519 alone). Checked wherever a leaf is validated: key packages, updates, commits.
fn strong_key(sid: &SigningIdentity) -> Result<(), LeafRefused> {
    let bytes: [u8; 32] = sid.signature_key.as_bytes().try_into().map_err(|_| LeafRefused::WeakKey)?;
    match ed25519_dalek::VerifyingKey::from_bytes(&bytes) {
        Ok(vk) if !vk.is_weak() => Ok(()),
        _ => Err(LeafRefused::WeakKey),
    }
}

impl IdentityProvider for PerLeafIdentity {
    type Error = LeafRefused;

    fn validate_member(
        &self,
        sid: &SigningIdentity,
        _t: Option<MlsTime>,
        _c: MemberValidationContext<'_>,
    ) -> Result<(), Self::Error> {
        leaf_cred_id(sid)?;
        strong_key(sid)
    }

    fn validate_external_sender(
        &self,
        sid: &SigningIdentity,
        _t: Option<MlsTime>,
        _e: Option<&ExtensionList>,
    ) -> Result<(), Self::Error> {
        leaf_cred_id(sid)?;
        strong_key(sid)
    }

    /// credential ‖ signature key — unique per LEAF, not per person. This one
    /// line is the whole change.
    fn identity(&self, sid: &SigningIdentity, _e: &ExtensionList) -> Result<Vec<u8>, Self::Error> {
        let mut out = leaf_cred_id(sid)?;
        out.extend_from_slice(sid.signature_key.as_bytes());
        Ok(out)
    }

    /// UNCHANGED FROM BASIC: one identity may MOVE between leaves — that is what
    /// an Update is — and may not be replaced by a stranger. Compares credentials
    /// only, deliberately: if this compared `identity()` no leaf could ever
    /// rotate its signature key.
    fn valid_successor(
        &self,
        pred: &SigningIdentity,
        succ: &SigningIdentity,
        _e: &ExtensionList,
    ) -> Result<bool, Self::Error> {
        Ok(leaf_cred_id(pred)? == leaf_cred_id(succ)?)
    }

    fn supported_types(&self) -> Vec<CredentialType> {
        vec![BasicCredential::credential_type()]
    }
}

/// The MLS configuration this crate builds, composed from the public `WithX`
/// builder aliases in the SAME order [`build_client`] applies them: identity →
/// crypto → group-state → key-package.
///
/// PARAMETERISED OVER THE TWO STORAGE PROVIDERS, and that is the whole point.
/// Everything above them — the cipher suite, the [`PerLeafIdentity`] identity
/// provider, the group-name extension — is Pacific's protocol and must be
/// identical on every platform or two clients cannot form a group. Where the
/// bytes are KEPT is not protocol: SQLite on a phone, IndexedDB in a browser,
/// whatever the next platform brings. Welding the two together is what made
/// `mls` require `storage`, and what made a browser leaf impossible without
/// dragging SQLite into wasm.
///
/// A platform supplies two `mls-rs` trait impls and gets the rest for free.
pub type PacificConfigWith<G, K> = WithMlsRules<
    PacificRules,
    WithKeyPackageRepo<
        K,
        WithGroupStateStorage<
            G,
            WithCryptoProvider<
                RustCryptoProvider,
                WithIdentityProvider<PerLeafIdentity, BaseConfig>,
            >,
        >,
    >,
>;

/// The two providers a platform brings, named once so seventeen signatures do
/// not each restate them. Blanket impls, so a platform implements the `mls-rs`
/// traits and these follow — there is nothing extra to write.
pub trait GroupStore: GroupStateStorage + Clone {}
impl<T: GroupStateStorage + Clone> GroupStore for T {}
pub trait KeyStore: KeyPackageStorage + Clone {}
impl<T: KeyPackageStorage + Clone> KeyStore for T {}

/// A configured MLS client over a platform's own storage.
pub type ClientWith<G, K> = MlsClient<PacificConfigWith<G, K>>;
/// A loaded/created MLS group over a platform's own storage.
pub type GroupWith<G, K> = MlsGroup<PacificConfigWith<G, K>>;

/// THE NATIVE INSTANTIATION — SQLite, which is what a phone and the Arc run.
/// Named separately so `node.rs` keeps saying `mls::Group` and nothing about the
/// native build changes; `storage` is what these two names cost, and a platform
/// without it names its own pair instead.
#[cfg(feature = "storage")]
pub type PacificConfig =
    PacificConfigWith<crate::mls_store::SqliteGroupStateStorage, crate::mls_store::SqliteKeyPackageStorage>;
/// The fully-configured MLS client type on native storage.
#[cfg(feature = "storage")]
pub type Client = ClientWith<crate::mls_store::SqliteGroupStateStorage, crate::mls_store::SqliteKeyPackageStorage>;
/// A loaded/created MLS group on native storage.
#[cfg(feature = "storage")]
pub type Group = GroupWith<crate::mls_store::SqliteGroupStateStorage, crate::mls_store::SqliteKeyPackageStorage>;

fn err<E: std::fmt::Display>(e: E) -> CoreError {
    CoreError::Mls(e.to_string())
}

/// An Add's refusal, by its kind: a key package past its lifetime is
/// [`CoreError::KeyPackageExpired`] (W-96), so a caller answers it without reading MLS's words.
fn add_err(e: MlsError) -> CoreError {
    match e {
        MlsError::InvalidLifetime { .. } => CoreError::KeyPackageExpired(e.to_string()),
        e => err(e),
    }
}

pub fn crypto() -> RustCryptoProvider {
    RustCryptoProvider::new()
}

/// Generate a fresh MLS leaf signature keypair. Persist it (directory `me`) so
/// every later process rebuilds the SAME client identity.
pub fn generate_signing_key(
    crypto: &RustCryptoProvider,
) -> Result<(SignatureSecretKey, SignaturePublicKey), CoreError> {
    let csp = crypto
        .cipher_suite_provider(CIPHERSUITE)
        .ok_or_else(|| err("ciphersuite unsupported by provider"))?;
    csp.signature_key_generate().map_err(err)
}

/// Reconstruct the `SigningIdentity` from persisted public-key bytes.
///
/// The BasicCredential carries `cred_id` — the Ed25519 STABLE IDENTITY pubkey, so
/// each party has a UNIQUE leaf credential. (Using the display name here would
/// make two parties named the same collide as "duplicate identity" in mls-rs.)
pub fn signing_identity(cred_id: &[u8], sig_pk: &[u8]) -> SigningIdentity {
    let cred = BasicCredential::new(cred_id.to_vec()).into_credential();
    SigningIdentity::new(cred, SignaturePublicKey::new(sig_pk.to_vec()))
}

/// Build the MLS client over storage the caller supplies.
///
/// THE ONE PLACE PACIFIC'S MLS PROTOCOL IS STATED, and it takes no path: the
/// cipher suite, the identity provider and the group-name extension are fixed
/// here for every platform, and the two things that vary arrive as arguments.
/// A platform that reimplemented this builder would be free to drift on the
/// cipher suite or forget the extension, and the failure mode is an add/join
/// mls-rs rejects at the far end — which is why there is one builder and not one
/// per platform.
pub fn build_client<G, K>(gss: G, kps: K, sid: SigningIdentity, sk: SignatureSecretKey)
    -> Result<ClientWith<G, K>, CoreError>
where
    G: GroupStateStorage + Clone,
    K: KeyPackageStorage + Clone,
{
    build_client_living(gss, kps, sid, sk, None)
}

/// The one builder, with the lifetime of the key packages it makes: `None` is mls-rs's own,
/// a year. A contact code's packages live a short while (W-96): the length runs from the
/// start [`key_package_made_at`] backdates by [`KEY_PACKAGE_SKEW_SECS`], so the skew is added.
pub fn build_client_living<G, K>(gss: G, kps: K, sid: SigningIdentity, sk: SignatureSecretKey, life: Option<std::time::Duration>)
    -> Result<ClientWith<G, K>, CoreError>
where
    G: GroupStateStorage + Clone,
    K: KeyPackageStorage + Clone,
{
    let builder = MlsClient::builder()
        .identity_provider(PerLeafIdentity)
        .crypto_provider(crypto())
        .group_state_storage(gss)
        .key_package_repo(kps)
        // THE COMMIT RULES (membership-through-mls.md §4). Installed here, in the one
        // builder, so the phone, the Arc and the browser judge every commit they SEND
        // and every commit they RECEIVE by the same table — a device with different
        // rules would accept a commit its peers refuse, and the group would split.
        .mls_rules(PacificRules)
        // Advertise support for every GroupContext extension Pacific writes, so every
        // leaf's capabilities allow them — otherwise mls-rs rejects the add/join. A
        // build that does not list OWNER_EXT cannot be added to a group made by one
        // that does (§3.2): that is the rollout constraint, stated where it is caused.
        .extension_types([GROUP_NAME_EXT, OWNER_EXT, MIN_VERSION_EXT])
        .signing_identity(sid, sk, CIPHERSUITE);
    Ok(match life {
        Some(life) => builder.key_package_lifetime(life + std::time::Duration::from_secs(KEY_PACKAGE_SKEW_SECS)).build(),
        None => builder.build(),
    })
}

/// The native convenience: open the SQLite providers at `db_path` and build.
/// Every caller in `node.rs` goes through here, so the native path is unchanged.
#[cfg(feature = "storage")]
pub fn build_client_sqlite(
    db_path: &std::path::Path,
    sid: SigningIdentity,
    sk: SignatureSecretKey,
) -> Result<Client, CoreError> {
    let gss = crate::mls_store::SqliteGroupStateStorage::open(db_path).map_err(err)?;
    let kps = crate::mls_store::SqliteKeyPackageStorage::open(db_path).map_err(err)?;
    build_client(gss, kps, sid, sk)
}

/// [`build_client_sqlite`], its key packages living `life` (W-96's contact code).
#[cfg(feature = "storage")]
pub fn build_client_sqlite_living(
    db_path: &std::path::Path,
    sid: SigningIdentity,
    sk: SignatureSecretKey,
    life: std::time::Duration,
) -> Result<Client, CoreError> {
    let gss = crate::mls_store::SqliteGroupStateStorage::open(db_path).map_err(err)?;
    let kps = crate::mls_store::SqliteKeyPackageStorage::open(db_path).map_err(err)?;
    build_client_living(gss, kps, sid, sk, Some(life))
}

/// A fresh one-time KeyPackage — the thing a peer scans and consumes to add
/// this leaf. Generic over storage for the same reason the builder is: minting
/// one is the platform-portable half of pairing.
pub fn make_key_package_bytes<G, K>(client: &ClientWith<G, K>) -> Result<Vec<u8>, CoreError>
where
    G: GroupStateStorage + Clone,
    K: KeyPackageStorage + Clone,
{
    key_package_made_at(client, MlsTime::now().seconds_since_epoch())
}

/// How far ahead of its adder a joiner's clock may run. The adder checks a key
/// package's lifetime against ITS OWN clock, and mls-rs dates the lifetime from the
/// maker's current second, so a phone one second ahead of the device adding it was
/// refused (NC-50). The lifetime starts this far before the maker's clock instead.
pub const KEY_PACKAGE_SKEW_SECS: u64 = 3600;

/// A key package whose maker's clock reads `now`.
fn key_package_made_at<G, K>(client: &ClientWith<G, K>, now: u64) -> Result<Vec<u8>, CoreError>
where
    G: GroupStateStorage + Clone,
    K: KeyPackageStorage + Clone,
{
    let not_before = MlsTime::from(now.saturating_sub(KEY_PACKAGE_SKEW_SECS));
    let kp = client
        .generate_key_package_message(ExtensionList::default(), ExtensionList::default(), Some(not_before))
        .map_err(err)?;
    kp.to_bytes().map_err(err)
}

/// An unnamed group (Contacts / DMs), owned by its creator.
pub fn create_group<G, K>(client: &ClientWith<G, K>) -> Result<GroupWith<G, K>, CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    create_group_named(client, "")
}

/// Private-use MLS extension (0xF001) carrying a group's DISPLAY NAME inside its
/// GroupContext. Because the Welcome delivers the current GroupInfo (group context +
/// ratchet tree), a late joiner reads the CURRENT name straight from their Welcome —
/// with NO message backfill and NO forward-secrecy break (this is the correct home
/// for durable group STATE, unlike per-message deltas which forward secrecy withholds
/// from joiners). Registered in the client capabilities below so mls-rs does not
/// reject it as an `UnsupportedGroupExtension` on add/join.
pub const GROUP_NAME_EXT: ExtensionType = ExtensionType::new(0xF001);

/// The GroupContext a new group is born with: its display name (if any), its OWNER,
/// and the protocol floor it was made at. The owner and the floor are the two things
/// the commit rules read (§3, §4), and a group that did not carry them from its first
/// epoch would have to be migrated into them later (§3.3), so every group made from
/// here on carries them from the start.
fn genesis_ctx_exts(name: &str, owner: &[u8; 32]) -> ExtensionList {
    let mut exts = ExtensionList::new();
    if !name.trim().is_empty() {
        exts.set(Extension::new(GROUP_NAME_EXT, name.as_bytes().to_vec()));
    }
    exts.set(Extension::new(OWNER_EXT, owner.to_vec()));
    exts.set(Extension::new(MIN_VERSION_EXT, PROTOCOL_VERSION.to_be_bytes().to_vec()));
    exts
}

/// Create a group whose GroupContext carries `name` (Conversations/Forums) and names
/// its CREATOR as owner. An empty name creates an unnamed group (Contacts / DMs).
///
/// The owner is derived from the client's own credential rather than passed in: at
/// creation the owner is always the creator, and an argument would only be a second
/// place to get that wrong.
pub fn create_group_named<G, K>(client: &ClientWith<G, K>, name: &str) -> Result<GroupWith<G, K>, CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    let (sid, _) = client.signing_identity().map_err(err)?;
    let owner = basic_id(sid)?;
    let owner = &owner;
    let mut g = client
        .create_group(genesis_ctx_exts(name, owner), ExtensionList::default(), None)
        .map_err(err)?;
    g.write_to_storage().map_err(err)?;
    tracing::info!(
        target: "pacific::mls",
        group = %hex::encode(g.group_id()),
        owner = %hex::encode(owner),
        named = !name.trim().is_empty(),
        floor = PROTOCOL_VERSION,
        "group created with its owner in the context"
    );
    Ok(g)
}

/// Create a two-leaf group — us and the peer whose key package this is — for pairing
/// and tethers, and add the peer. Applied and persisted; returns
/// `(group, commit, welcome, legacy)`.
///
/// THE ROLLOUT BRIDGE (§3.2). A build from before the owner extension does not
/// advertise it, and mls-rs refuses to add such a leaf to a group whose context
/// carries it. Pairing is the one place the creator knows, at creation, who the next
/// leaf is, so for that peer the group starts LEGACY instead: no owner and no floor in
/// the context, the owner read from leaf 0 — us (§3.3). Once the peer's updated build
/// refreshes its leaf (§11.3), our next sync writes the owner in, and every door that
/// needs it works from then on. Nothing else about the group differs.
pub fn create_group_with_peer<G, K>(
    client: &ClientWith<G, K>,
    peer_kp: &[u8],
) -> Result<(GroupWith<G, K>, Vec<u8>, Vec<u8>, bool), CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    let (sid, _) = client.signing_identity().map_err(err)?;
    let owner = basic_id(sid)?;
    let kp = MlsMessage::from_bytes(peer_kp).map_err(err)?;
    // Nothing is persisted until the Add is built, so the group that could not take
    // this peer is simply dropped.
    let mut g = client
        .create_group(genesis_ctx_exts("", &owner), ExtensionList::default(), None)
        .map_err(err)?;
    let (mut g, out, legacy) = match g.commit_builder().add_member(kp.clone()).and_then(|b| b.build()) {
        Ok(out) => (g, out, false),
        Err(MlsError::UnsupportedGroupExtension(t)) if t == OWNER_EXT || t == MIN_VERSION_EXT => {
            let mut lg = client
                .create_group(ExtensionList::new(), ExtensionList::default(), None)
                .map_err(err)?;
            let out = lg
                .commit_builder()
                .add_member(kp)
                .map_err(add_err)?
                .build()
                .map_err(add_err)?;
            tracing::warn!(
                target: "pacific::mls",
                group = %hex::encode(lg.group_id()),
                owner = %hex::encode(owner),
                missing = ?t,
                "the peer runs a build without the owner extension — this group starts \
                 legacy, and its owner is written into the context once they update"
            );
            (lg, out, true)
        }
        Err(e) => return Err(add_err(e)),
    };
    let commit = out.commit_message.to_bytes().map_err(err)?;
    let welcome = out
        .welcome_messages
        .first()
        .ok_or_else(|| err("welcome missing after add"))?
        .to_bytes()
        .map_err(err)?;
    g.apply_pending_commit().map_err(err)?;
    g.write_to_storage().map_err(err)?;
    if !legacy {
        tracing::info!(
            target: "pacific::mls",
            group = %hex::encode(g.group_id()),
            owner = %hex::encode(owner),
            floor = PROTOCOL_VERSION,
            "two-leaf group created with its owner in the context"
        );
    }
    Ok((g, commit, welcome, legacy))
}

/// The group's display name from its GroupContext extension ("" if unnamed). Rides
/// the Welcome, so every member — including late joiners — reads the same value.
pub fn group_name<G, K>(group: &GroupWith<G, K>) -> String
where
    G: GroupStore,
    K: KeyStore,
{
    group
        .context()
        .extensions()
        .get(GROUP_NAME_EXT)
        .map(|e| String::from_utf8_lossy(&e.extension_data).into_owned())
        .unwrap_or_default()
}

// ── THE OWNER, THE FLOOR, AND THE COMMIT RULES ──────────────────────────────────────
// docs/membership-through-mls.md §2–§4. Every member must apply the same rules to the
// same commit — mls-rs's own trait documentation: "Each member of a group MUST apply
// the same proposal rules in order to maintain a working group." So the rules below
// read ONLY what every member provably holds at the commit's epoch: the ratchet tree
// (the roster mls-rs passes in) and the GroupContext (covered by the confirmation
// tag). Nothing here may read the fold, the directory, a clock, or this device's
// opinion of anything: two devices holding different prefixes of the log would then
// judge the same commit differently, and the group would split.

/// Private-use GroupContext extension (0xF002): the object's OWNER — exactly 32 bytes,
/// the identity pubkey every leaf of that person carries as its `BasicCredential` id.
/// Written at creation (`genesis_ctx_exts`) and changed ONLY by a handover commit the
/// current owner makes (§9). A person, not a leaf: every leaf whose credential is these
/// 32 bytes is an owner leaf (`PerLeafIdentity`).
pub const OWNER_EXT: ExtensionType = ExtensionType::new(0xF002);

/// Private-use GroupContext extension (0xF003): the lowest Pacific protocol version
/// allowed to process this group's commits, as a big-endian u16. XMTP learned in
/// July–August 2026 that a client which REJECTS a commit its newer peers accept forks
/// the group; a client below the floor therefore PAUSES the group instead of judging
/// it (`CoreError::UpgradeRequired`, which the drain treats as retryable). It must exist
/// before the first rule change that needs it, which is why it arrives with the first
/// rules. Only ever raised, and only by the owner.
pub const MIN_VERSION_EXT: ExtensionType = ExtensionType::new(0xF003);

/// This build's Pacific protocol version — what the floor is compared against. Raise
/// it together with any change to the commit rules, never alone.
pub const PROTOCOL_VERSION: u16 = 1;

/// The commit-rules table as data, one rule id per proposal type. It restates nothing:
/// `icd.rs` pins it against `info.x-mls.commitRules` in the ICD in both directions, so
/// the document and the code cannot drift. `PacificRules` is what enforces it; the
/// tests in this module are what prove the enforcement matches the name.
pub const COMMIT_RULES: [(&str, &str); 9] = [
    ("add", "anyMember"),
    ("update", "self"),
    ("remove", "ownerOrOwnLeaves"),
    ("groupContextExtensions", "owner"),
    ("psk", "refused"),
    ("reinit", "refused"),
    ("externalInit", "refused"),
    ("custom", "refused"),
    ("externalCommit", "refused"),
];

/// Why a commit, or a proposal within it, is refused. Every variant names its reason
/// in words an operator can act on: a refused commit is quarantined and logged, and
/// "rejected" alone would say nothing about whether this is an attack, an old build or
/// a bug.
#[derive(Clone, PartialEq, Eq)]
pub enum RuleViolation {
    /// The group's floor is above this build (§ MIN_VERSION_EXT). Pause; never judge.
    UpgradeRequired { floor: u16 },
    /// External commits are refused: the doorbell replaced external joins.
    ExternalCommit,
    /// A proposal type Pacific never uses (PSK, ReInit, ExternalInit, custom), or one
    /// from a sender that is not a member.
    RefusedProposal(&'static str),
    /// A leaf index that names no member of the current tree.
    NotAMember { leaf: u32 },
    /// A leaf whose credential is not a 32-byte `BasicCredential`.
    NotBasic,
    /// No owner could be read — neither the extension nor a leaf 0.
    NoOwner(String),
    /// A Remove in a group made before the owner lived in the context (§3.3): the
    /// leaf-0 owner is only true while no leaf has ever been removed.
    LegacyGroupCannotRemove,
    /// Nobody removes the owner's leaves; the owner hands over first (§9).
    OwnerCannotBeRemoved,
    /// A Remove whose proposer is neither the owner nor the person whose leaf it is.
    RemoveNotAuthorized { leaf: u32 },
    /// Something only the owner may propose, proposed by someone else.
    NotOwner(&'static str),
    /// A GroupContext extension Pacific does not write.
    UnknownContextExtension(u16),
    /// A context change that drops the owner extension.
    OwnerExtensionDropped,
    /// An owner extension that is not exactly 32 bytes.
    BadOwnerExtension,
    /// A handover to someone who holds no leaf.
    HandoverTargetNotInTree,
    /// A handover riding in the same commit as an Add or a Remove.
    HandoverWithMembershipChange,
    /// The §3.3 migration naming someone other than the leaf-0 owner.
    MigrationNotSelf,
    /// The floor may only rise.
    FloorLowered { from: u16, to: u16 },
    /// A device may not raise the floor above its own version.
    FloorAboveBuild { floor: u16 },
    /// A floor extension that is not exactly 2 bytes.
    BadFloor,
}

impl std::fmt::Display for RuleViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use RuleViolation::*;
        match self {
            UpgradeRequired { floor } => write!(
                f,
                "the group requires protocol {floor}; this build speaks {PROTOCOL_VERSION} — update to continue"
            ),
            ExternalCommit => write!(f, "external commits are refused"),
            RefusedProposal(what) => write!(f, "{what} proposals are refused"),
            NotAMember { leaf } => write!(f, "leaf {leaf} is not a member of the current tree"),
            NotBasic => write!(f, "a leaf's credential is not a 32-byte BasicCredential"),
            NoOwner(why) => write!(f, "no owner can be read from the group: {why}"),
            LegacyGroupCannotRemove => write!(
                f,
                "this group predates the owner extension; removal is refused until the owner migrates it"
            ),
            OwnerCannotBeRemoved => write!(f, "the owner cannot be removed — hand the object over first"),
            RemoveNotAuthorized { leaf } => write!(
                f,
                "the removal of leaf {leaf} was proposed by neither the owner nor the person leaving"
            ),
            NotOwner(what) => write!(f, "only the owner may {what}"),
            UnknownContextExtension(t) => write!(f, "context extension {t:#06x} is not one Pacific writes"),
            OwnerExtensionDropped => write!(f, "a context change may not drop the owner"),
            BadOwnerExtension => write!(f, "the owner extension is not exactly 32 bytes"),
            HandoverTargetNotInTree => write!(f, "a handover must name someone who holds a leaf"),
            HandoverWithMembershipChange => {
                write!(f, "a handover may not ride in the same commit as an Add or a Remove")
            }
            MigrationNotSelf => write!(f, "the owner migration must name the leaf-0 owner"),
            FloorLowered { from, to } => write!(f, "the protocol floor may only rise ({from} → {to})"),
            FloorAboveBuild { floor } => {
                write!(f, "cannot raise the floor to {floor} above this build's {PROTOCOL_VERSION}")
            }
            BadFloor => write!(f, "the floor extension is not exactly 2 bytes"),
        }
    }
}
/// `Debug` IS the sentence. mls-rs wraps a rules error and reports it with `{:?}`, and
/// that report is what reaches the logs and the quarantine table — an operator should
/// read the reason, not a variant name.
impl std::fmt::Debug for RuleViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}
impl std::error::Error for RuleViolation {}
impl IntoAnyError for RuleViolation {}

/// Where an owner was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerSource {
    /// The owner extension — every group made from this build on.
    Context,
    /// The person at leaf 0, for a group made before the extension existed (§3.3).
    LegacyLeafZero,
}

/// The person a leaf belongs to, as a rules error rather than a `CoreError`.
fn person_rv(sid: &SigningIdentity) -> Result<[u8; 32], RuleViolation> {
    basic_id(sid).map_err(|_| RuleViolation::NotBasic)
}

/// The owner a context names, or — for a group made before the owner lived in the
/// context — the person at leaf 0. Every device computes the same answer from the
/// same state, which is the property the leaf-0 fallback exists to keep.
pub fn owner_in(exts: &ExtensionList, roster: &Roster) -> Result<([u8; 32], OwnerSource), RuleViolation> {
    if let Some(e) = exts.get(OWNER_EXT) {
        let o: [u8; 32] = e
            .extension_data
            .as_slice()
            .try_into()
            .map_err(|_| RuleViolation::BadOwnerExtension)?;
        return Ok((o, OwnerSource::Context));
    }
    let leaf0 = roster
        .member_with_index(0)
        .map_err(|e| RuleViolation::NoOwner(e.to_string()))?;
    Ok((person_rv(&leaf0.signing_identity)?, OwnerSource::LegacyLeafZero))
}

/// The floor a context names: 0 when absent (every group made before the floor existed).
pub fn floor_in(exts: &ExtensionList) -> Result<u16, RuleViolation> {
    match exts.get(MIN_VERSION_EXT) {
        None => Ok(0),
        Some(e) => {
            let b: [u8; 2] = e.extension_data.as_slice().try_into().map_err(|_| RuleViolation::BadFloor)?;
            Ok(u16::from_be_bytes(b))
        }
    }
}

/// The group's owner as MLS states it — the source every door and the fold's owner
/// history read (§3, §10.2). `groups.owner_pk` in the directory is a cache of this.
pub fn group_owner<G, K>(group: &GroupWith<G, K>) -> Result<[u8; 32], CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    owner_in(group.context().extensions(), &group.roster())
        .map(|(o, _)| o)
        .map_err(|e| CoreError::Mls(e.to_string()))
}

/// Does this group carry the owner extension, or is it a legacy group still reading
/// its owner from leaf 0 (§3.3)?
pub fn has_owner_ext<G, K>(group: &GroupWith<G, K>) -> bool
where
    G: GroupStore,
    K: KeyStore,
{
    group.context().extensions().has_extension(OWNER_EXT)
}

/// The group's protocol floor (0 when absent).
pub fn group_floor<G, K>(group: &GroupWith<G, K>) -> Result<u16, CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    floor_in(group.context().extensions()).map_err(|e| CoreError::Mls(e.to_string()))
}

/// Refuse a group whose floor is above this build: the caller must PAUSE (retryable,
/// no cursor advance), never judge or skip what it cannot understand.
pub fn check_floor<G, K>(group: &GroupWith<G, K>) -> Result<(), CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    let floor = group_floor(group)?;
    if floor > PROTOCOL_VERSION {
        tracing::warn!(
            target: "pacific::mls",
            group = %hex::encode(group.group_id()),
            floor,
            build = PROTOCOL_VERSION,
            "group paused: its protocol floor is above this build"
        );
        return Err(CoreError::UpgradeRequired(
            RuleViolation::UpgradeRequired { floor }.to_string(),
        ));
    }
    Ok(())
}

/// Every leaf index whose credential is `person` — a person may hold several leaves,
/// one per device (`PerLeafIdentity`), and leaving or being removed takes all of them.
pub fn leaves_of<G, K>(group: &GroupWith<G, K>, person: &[u8; 32]) -> Vec<u32>
where
    G: GroupStore,
    K: KeyStore,
{
    group
        .roster()
        .members_iter()
        .filter(|m| basic_id(&m.signing_identity).ok().as_ref() == Some(person))
        .map(|m| m.index)
        .collect()
}

/// This device's own leaf index.
pub fn my_leaf<G, K>(group: &GroupWith<G, K>) -> u32
where
    G: GroupStore,
    K: KeyStore,
{
    group.current_member_index()
}

/// The cached Removes of `person`'s leaves whose PROPOSER is one of `by` — resolved
/// the way `PacificRules` resolves it (§4.3), so a proposal the rules would drop on
/// send never counts here either.
fn cached_removes_of_by<G, K>(group: &GroupWith<G, K>, person: &[u8; 32], by: &[[u8; 32]]) -> bool
where
    G: GroupStore,
    K: KeyStore,
{
    let mine: BTreeSet<u32> = leaves_of(group, person).into_iter().collect();
    let roster = group.roster();
    group.get_cached_proposals().iter().any(|c| {
        let removes_mine = matches!(c.proposal(),
            mls_rs::group::proposal::Proposal::Remove(r) if mine.contains(&r.to_remove()));
        let proposer = match c.sender() {
            Sender::Member(i) => leaf_person(&roster, *i).ok(),
            _ => None,
        };
        removes_mine && proposer.is_some_and(|p| by.contains(&p))
    })
}

/// Is a removal of one of `person`'s leaves waiting in this group's cache that will
/// STAND — proposed by that person (a leave) or by the owner? A device for which this
/// is true is being removed, and must neither commit (its commit could only drop those
/// proposals and restart the leave, §7) nor author.
///
/// A Remove proposed by anyone else does not count. The rules drop it on every commit,
/// and a device that counted it would stop committing and self-updating on another
/// member's say-so.
pub fn pending_removal_of<G, K>(group: &GroupWith<G, K>, person: &[u8; 32]) -> bool
where
    G: GroupStore,
    K: KeyStore,
{
    let mut by = vec![*person];
    if let Ok(owner) = group_owner(group) {
        by.push(owner);
    }
    cached_removes_of_by(group, person, &by)
}

/// Is a LEAVE of `person` waiting — a removal of their leaves proposed by one of
/// their own devices (§6.4)? Only this makes a sibling device adopt the leave. A
/// Remove proposed by anyone else, even the owner, is not this person leaving, and
/// counting it would let any member start someone else's departure.
pub fn pending_leave_of<G, K>(group: &GroupWith<G, K>, person: &[u8; 32]) -> bool
where
    G: GroupStore,
    K: KeyStore,
{
    cached_removes_of_by(group, person, &[*person])
}

/// Are proposals waiting that this device must commit before it may send (RFC 9420
/// §12.4 — mls-rs refuses to encrypt with `CommitRequired` while this is true)?
pub fn commit_required<G, K>(group: &GroupWith<G, K>) -> bool
where
    G: GroupStore,
    K: KeyStore,
{
    group.commit_required()
}

/// Does every leaf in the tree advertise the owner extension? The §3.3 migration may
/// only be committed once it does — mls-rs refuses a context extension a member does
/// not support.
pub fn every_leaf_supports_owner_ext<G, K>(group: &GroupWith<G, K>) -> bool
where
    G: GroupStore,
    K: KeyStore,
{
    group
        .roster()
        .members_iter()
        .all(|m| m.capabilities.extensions.contains(&OWNER_EXT))
}

/// Does THIS device's own leaf advertise the owner extension? A leaf made by an older
/// build does not until it commits with a path (§11.3).
pub fn my_leaf_supports_owner_ext<G, K>(group: &GroupWith<G, K>) -> bool
where
    G: GroupStore,
    K: KeyStore,
{
    group
        .roster()
        .member_with_index(group.current_member_index())
        .map(|m| m.capabilities.extensions.contains(&OWNER_EXT))
        .unwrap_or(false)
}

/// Pacific's `MlsRules` — the commit rules of §4.3, applied identically on send and
/// on receive by every device.
#[derive(Debug, Clone, Copy, Default)]
pub struct PacificRules;

/// The person holding `leaf` in the current tree.
fn leaf_person(roster: &Roster, leaf: u32) -> Result<[u8; 32], RuleViolation> {
    let m = roster
        .member_with_index(leaf)
        .map_err(|_| RuleViolation::NotAMember { leaf })?;
    person_rv(&m.signing_identity)
}

/// The person who PROPOSED a change — the authority that counts (XMTP's rule 5, and
/// §4.3). For a proposal carried by value the sender is the committer; for one carried
/// by reference it is whoever sent it. A commit that sweeps the cache must never lend
/// the committer's authority to a proposal someone else made.
fn proposer<T>(roster: &Roster, info: &ProposalInfo<T>) -> Result<[u8; 32], RuleViolation> {
    match info.sender() {
        Sender::Member(i) => leaf_person(roster, *i),
        _ => Err(RuleViolation::RefusedProposal("non-member")),
    }
}

/// Keep, drop, or refuse. On SEND a by-reference proposal that breaks a rule is
/// DROPPED — someone else sent it, and refusing would make the commit impossible to
/// build (the mls-rs guidance). Anything else that breaks a rule is an error: a
/// by-value proposal on send means a door asked for something illegal, and on receive
/// every violation refuses the whole commit.
fn verdict(send: bool, by_reference: bool, v: Result<(), RuleViolation>) -> Result<bool, RuleViolation> {
    match v {
        Ok(()) => Ok(true),
        Err(e) if send && by_reference => {
            tracing::warn!(
                target: "pacific::mls::rules",
                reason = %e,
                "dropping a by-reference proposal this device may not commit"
            );
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

/// Refuse every proposal of one type Pacific never uses.
macro_rules! refuse_all {
    ($bundle:expr, $ty:ty, $send:expr, $what:expr) => {
        $bundle.retain_by_type::<$ty, _, RuleViolation>(|p| {
            verdict($send, p.is_by_reference(), Err(RuleViolation::RefusedProposal($what)))
        })
    };
}

impl PacificRules {
    fn judge(
        direction: CommitDirection,
        source: &CommitSource,
        roster: &Roster,
        context: &GroupContext,
        mut proposals: ProposalBundle,
    ) -> Result<ProposalBundle, RuleViolation> {
        let send = direction == CommitDirection::Send;
        let exts = context.extensions();

        // 0. THE FLOOR — before anything is judged. A device below it pauses.
        let floor = floor_in(exts)?;
        if floor > PROTOCOL_VERSION {
            return Err(RuleViolation::UpgradeRequired { floor });
        }

        // 1. WHO COMMITS. External commits are refused outright.
        let committer = match source {
            CommitSource::ExistingMember(m) => person_rv(&m.signing_identity)?,
            CommitSource::NewMember(_) => return Err(RuleViolation::ExternalCommit),
        };
        let _ = committer;
        let (owner, owner_source) = owner_in(exts, roster)?;
        let has_owner_ext = owner_source == OwnerSource::Context;

        // Who holds which leaves NOW (pre-commit), keyed by person.
        let mut leaves_by_person: BTreeMap<[u8; 32], BTreeSet<u32>> = BTreeMap::new();
        for m in roster.members_iter() {
            if let Ok(p) = person_rv(&m.signing_identity) {
                leaves_by_person.entry(p).or_default().insert(m.index);
            }
        }

        // 2. THE TYPES PACIFIC NEVER USES.
        refuse_all!(proposals, PreSharedKeyProposal, send, "pre-shared-key")?;
        refuse_all!(proposals, ReInitProposal, send, "reinit")?;
        refuse_all!(proposals, ExternalInit, send, "external-init")?;
        proposals.retain_custom::<_, RuleViolation>(|p| {
            verdict(send, p.is_by_reference(), Err(RuleViolation::RefusedProposal("custom")))
        })?;

        // 3. REMOVES. On send, at most one per leaf (two devices of a leaving person
        //    may both have proposed it, §6.4; RFC 9420 §12.2 makes a list with two
        //    invalid). Then each is authorized by its PROPOSER.
        if send {
            let mut seen = BTreeSet::new();
            proposals.retain_by_type::<RemoveProposal, _, RuleViolation>(|p| Ok(seen.insert(p.proposal.to_remove())))?;
        }
        // A REMOVE IS ALLOWED FROM THE OWNER, OR OF THE PROPOSER'S OWN LEAVES
        // (resumption.md §8, ruled 19 Sep). Rule (b) — a person's leaves go together or
        // not at all — is retired: under retained keys every device holds the seed, so
        // "a stolen device cannot remove its siblings and stay" protected nothing, and
        // evicting one expired pool leaf needs exactly the single-leaf removal it barred.
        // The owner may remove their own leaves too, but never the last: that is the
        // owner leaving, and the owner hands over first (§9).
        let judge_remove = |p: &ProposalInfo<RemoveProposal>, owner_left: bool| {
            if !has_owner_ext {
                return Err(RuleViolation::LegacyGroupCannotRemove);
            }
            let leaf = p.proposal.to_remove();
            let target = leaf_person(roster, leaf)?;
            let by = proposer(roster, p)?;
            if target == owner {
                return if by == owner && !owner_left { Ok(()) } else { Err(RuleViolation::OwnerCannotBeRemoved) };
            }
            if by == owner || by == target {
                return Ok(());
            }
            Err(RuleViolation::RemoveNotAuthorized { leaf })
        };
        // Would the removes that ride take every leaf the owner holds?
        let owner_emptied = |b: &ProposalBundle| -> bool {
            let removed: BTreeSet<u32> = b.remove_proposals().iter().map(|p| p.proposal.to_remove()).collect();
            leaves_by_person.get(&owner).is_some_and(|ls| !ls.is_empty() && ls.is_subset(&removed))
        };
        proposals.retain_by_type::<RemoveProposal, _, RuleViolation>(|p| {
            verdict(send, p.is_by_reference(), judge_remove(p, false))
        })?;
        let emptied = owner_emptied(&proposals);
        proposals.retain_by_type::<RemoveProposal, _, RuleViolation>(|p| {
            verdict(send, p.is_by_reference(), judge_remove(p, emptied))
        })?;

        // 4. GROUP CONTEXT CHANGES — rename, handover, migration, floor. Owner only.
        let membership_changes =
            !proposals.add_proposals().is_empty() || !proposals.remove_proposals().is_empty();
        let old_floor = floor;
        proposals.retain_by_type::<ExtensionList, _, RuleViolation>(|p| {
            let v = (|| {
                if proposer(roster, p)? != owner {
                    return Err(RuleViolation::NotOwner("change the group context"));
                }
                let new = &p.proposal;
                for e in new.iter() {
                    let t = e.extension_type;
                    if t != GROUP_NAME_EXT && t != OWNER_EXT && t != MIN_VERSION_EXT {
                        return Err(RuleViolation::UnknownContextExtension(t.raw_value()));
                    }
                }
                match new.get(OWNER_EXT) {
                    None if has_owner_ext => return Err(RuleViolation::OwnerExtensionDropped),
                    None => {}
                    Some(e) => {
                        let next: [u8; 32] = e
                            .extension_data
                            .as_slice()
                            .try_into()
                            .map_err(|_| RuleViolation::BadOwnerExtension)?;
                        if !has_owner_ext {
                            // §3.3: the legacy owner writes ITSELF into the context.
                            if next != owner {
                                return Err(RuleViolation::MigrationNotSelf);
                            }
                        } else if next != owner {
                            // §9: a handover.
                            if !leaves_by_person.contains_key(&next) {
                                return Err(RuleViolation::HandoverTargetNotInTree);
                            }
                            if membership_changes {
                                return Err(RuleViolation::HandoverWithMembershipChange);
                            }
                        }
                    }
                }
                let new_floor = floor_in(new)?;
                if new_floor < old_floor {
                    return Err(RuleViolation::FloorLowered { from: old_floor, to: new_floor });
                }
                if send && new_floor > PROTOCOL_VERSION {
                    return Err(RuleViolation::FloorAboveBuild { floor: new_floor });
                }
                Ok(())
            })();
            verdict(send, p.is_by_reference(), v)
        })?;

        // 5. ADDS are any member's (who may ADMIT is still decided at the doors, §16);
        //    UPDATES are their sender's own leaf, which MLS itself enforces.
        Ok(proposals)
    }
}

impl MlsRules for PacificRules {
    type Error = RuleViolation;

    fn filter_proposals(
        &self,
        direction: CommitDirection,
        source: CommitSource,
        current_roster: &Roster,
        current_context: &GroupContext,
        proposals: ProposalBundle,
    ) -> Result<ProposalBundle, Self::Error> {
        let before = proposals.length();
        let result = Self::judge(direction, &source, current_roster, current_context, proposals);
        match &result {
            Ok(kept) => tracing::debug!(
                target: "pacific::mls::rules",
                direction = ?direction,
                epoch = current_context.epoch,
                proposals = before,
                kept = kept.length(),
                "commit judged"
            ),
            Err(e) => tracing::warn!(
                target: "pacific::mls::rules",
                direction = ?direction,
                epoch = current_context.epoch,
                proposals = before,
                reason = %e,
                "commit refused by the commit rules"
            ),
        }
        result
    }

    fn commit_options(
        &self,
        _new_roster: &Roster,
        _new_context: &GroupContext,
        _proposals: &ProposalBundle,
    ) -> Result<CommitOptions, Self::Error> {
        // Every commit carries a path: RFC 9420 §12.4's "full" commit, so every commit
        // refreshes its committer's leaf — an Add included (§4.4, §11.1).
        Ok(CommitOptions::new().with_path_required(true))
    }

    fn encryption_options(
        &self,
        _current_roster: &Roster,
        _current_context: &GroupContext,
    ) -> Result<EncryptionOptions, Self::Error> {
        // Proposals and commits travel as PrivateMessage (RFC 9420 §6's SHOULD), which
        // closes keys.md §7 — they were PublicMessage by default, not by decision.
        let mut e = EncryptionOptions::default();
        e.encrypt_control_messages = true;
        Ok(e)
    }
}

/// Stage the removal of `leaves` WITHOUT applying it — the §5 door's commit. Same
/// pending-commit discipline as [`stage_add_member`]: nothing is applied or persisted
/// until [`apply_staged`] after the commit wins its slot.
pub fn stage_remove<G, K>(group: &mut GroupWith<G, K>, leaves: &[u32]) -> Result<Vec<u8>, CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    let mut b = group.commit_builder();
    for l in leaves {
        b = b.remove_member(*l).map_err(err)?;
    }
    let commit = b.build().map_err(err)?;
    commit.commit_message.to_bytes().map_err(err)
}

/// Propose the removal of `leaf` BY REFERENCE — how a person leaves (§6.1): MLS forbids
/// committing your own removal (RFC 9420 §12.2), so the leaver proposes and another
/// member commits. The proposal is cached here too, which is what stops this device
/// encrypting another application message (`CommitRequired`) — so the leaver's last
/// delta must be flushed BEFORE this is called.
pub fn propose_remove<G, K>(group: &mut GroupWith<G, K>, leaf: u32) -> Result<Vec<u8>, CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    let msg = group.propose_remove(leaf, Vec::new()).map_err(err)?;
    group.write_to_storage().map_err(err)?;
    msg.to_bytes().map_err(err)
}

/// Stage a commit that writes `owner` into the owner extension WITHOUT applying it —
/// a handover (§9) or, for a legacy group, the migration (§3.3). Edits a clone of the
/// current extension list, as [`stage_rename`] does, so no other context extension is
/// dropped.
pub fn stage_set_owner<G, K>(group: &mut GroupWith<G, K>, owner: &[u8; 32]) -> Result<Vec<u8>, CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    let mut exts = group.context().extensions().clone();
    exts.set(Extension::new(OWNER_EXT, owner.to_vec()));
    if !exts.has_extension(MIN_VERSION_EXT) {
        // A legacy group gains its floor in the same commit that gains its owner.
        exts.set(Extension::new(MIN_VERSION_EXT, PROTOCOL_VERSION.to_be_bytes().to_vec()));
    }
    let commit = group
        .commit_builder()
        .set_group_context_ext(exts)
        .map_err(err)?
        .build()
        .map_err(err)?;
    commit.commit_message.to_bytes().map_err(err)
}

/// Cross-process group load — the corrected continuity path.
pub fn load_group<G, K>(client: &ClientWith<G, K>, group_id: &[u8]) -> Result<GroupWith<G, K>, CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    client.load_group(group_id).map_err(err)
}

/// Add peer by KeyPackage -> (commit_bytes, welcome_bytes). Applies + persists
/// IMMEDIATELY — only correct where no other member can commit concurrently
/// (group creation: pair_scan's fresh 2-party group, where the commit is never
/// even published to a shared tag). The shared-group add path uses
/// [`stage_add_member`] + [`apply_staged`] instead: RFC 9420 §14 — "The
/// generation of Commit messages MUST NOT modify a client's state".
pub fn add_member<G, K>(group: &mut GroupWith<G, K>, peer_kp: &[u8]) -> Result<(Vec<u8>, Vec<u8>), CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    let (commit_b, welcome_b) = stage_add_member(group, peer_kp)?;
    apply_staged(group)?;
    Ok((commit_b, welcome_b))
}

/// Stage an add WITHOUT applying it: build the commit + welcome, leave the
/// pending commit in memory, and DO NOT persist. The caller publishes the
/// commit into the epoch tag's slot first; only a winning ack applies it
/// ([`apply_staged`]). Losing means simply dropping/reloading the group — the
/// staged state was never written, so nothing needs undoing.
pub fn stage_add_member<G, K>(
    group: &mut GroupWith<G, K>,
    peer_kp: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    let kp = MlsMessage::from_bytes(peer_kp).map_err(err)?;
    let commit = group
        .commit_builder()
        .add_member(kp)
        .map_err(add_err)?
        .build()
        .map_err(add_err)?;
    let commit_b = commit.commit_message.to_bytes().map_err(err)?;
    let welcome_b = commit
        .welcome_messages
        .first()
        .ok_or_else(|| err("welcome missing after add"))?
        .to_bytes()
        .map_err(err)?;
    Ok((commit_b, welcome_b))
}

/// Stage a RENAME without applying it: a GroupContextExtensions commit that rewrites
/// [`GROUP_NAME_EXT`] in the GroupContext. Same pending-commit discipline as
/// [`stage_add_member`] — the caller publishes into the epoch's commit slot and only
/// [`apply_staged`] on a win. Existing members converge by folding the commit off the
/// old epoch tag; members who join later read the new name from their Welcome's
/// GroupInfo, so there is exactly ONE home for the name at every epoch.
///
/// `set_group_context_ext` REPLACES the whole extension list, so we edit a clone of
/// the current one — sending a list containing only the name would silently drop
/// every other context extension the group carries.
pub fn stage_rename<G, K>(group: &mut GroupWith<G, K>, name: &str) -> Result<Vec<u8>, CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    let mut exts = group.context().extensions().clone();
    let trimmed = name.trim();
    if trimmed.is_empty() {
        exts.remove(GROUP_NAME_EXT);
    } else {
        exts.set(Extension::new(GROUP_NAME_EXT, trimmed.as_bytes().to_vec()));
    }
    let commit = group
        .commit_builder()
        .set_group_context_ext(exts)
        .map_err(err)?
        .build()
        .map_err(err)?;
    commit.commit_message.to_bytes().map_err(err)
}

/// Stage a REKEY without applying it: a commit carrying NO proposals, which RFC
/// 9420 §12.4 obliges to carry a fresh UpdatePath — a new HPKE key for our leaf
/// and new epoch secrets for every member (mls-rs forces the path when the
/// proposal list is empty; see `path_update_required`). Same pending-commit
/// discipline as [`stage_add_member`]: the caller publishes into the epoch's
/// commit slot and only [`apply_staged`] on a win; losing means dropping the
/// group, since nothing was written.
///
/// THE FORK-CLOSER after a backup restore (retired, a58798c): once this commit is
/// accepted, a stale copy of the same leaf — the install the backup was taken
/// from — cannot decrypt forward, and its own next commit loses the slot. A
/// stale device becomes deaf, never a second voice.
///
/// WHY THE STALE COPY CANNOT FOLLOW. The commit is signed by the leaf both
/// copies share, so every OTHER member accepts it and moves on. The stale copy
/// sees a commit from its own leaf index with no pending commit behind it, which
/// mls-rs refuses as `CantProcessMessageFromSelf` (`decrypt_message` reports it
/// as `SkippedOwn`) — so it stays at the old epoch, cannot derive the new tag,
/// and cannot open anything sealed there. Nothing about this is a special case
/// in the protocol: it is what a self-update means. `Node::rekey_all_groups`
/// runs one of these per group, through the relay's commit slot.
//
// ONE implementation, shared by the phone (`node.rs`) and the browser
// (core-wasm's `mls_rekey`) — the wire shape (commit bytes out, pending left in
// memory) is what both sides depend on, so there is exactly one of these.
pub fn stage_rekey<G, K>(group: &mut GroupWith<G, K>) -> Result<Vec<u8>, CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    let commit = group.commit_builder().build().map_err(err)?;
    commit.commit_message.to_bytes().map_err(err)
}

/// Apply + persist the staged (pending) commit — call ONLY after the commit won
/// its epoch slot on the relay (or, at group creation, where no race exists).
pub fn apply_staged<G, K>(group: &mut GroupWith<G, K>) -> Result<(), CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    group.apply_pending_commit().map_err(err)?;
    group.write_to_storage().map_err(err)?;
    Ok(())
}

pub fn join_group<G, K>(client: &ClientWith<G, K>, welcome: &[u8]) -> Result<GroupWith<G, K>, CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    let mut group = join_group_unsaved(client, welcome)?;
    save_group(&mut group)?;
    Ok(group)
}

/// Join from a Welcome WITHOUT persisting it — for a joiner that still has checks to
/// make before it accepts the group (§8.5: the sender and owner in the roster, the
/// owner against the card). Persist with [`save_group`] once they pass, so a refused
/// Welcome leaves no group state behind.
pub fn join_group_unsaved<G, K>(client: &ClientWith<G, K>, welcome: &[u8]) -> Result<GroupWith<G, K>, CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    let w = MlsMessage::from_bytes(welcome).map_err(err)?;
    let (group, _info) = client.join_group(None, &w, None).map_err(err)?;
    Ok(group)
}

/// As [`join_group_unsaved`], with the identity of the member who committed the Add that let
/// this device in: MLS's `NewMemberInfo.sender`, authenticated by the GroupInfo's signature, so a
/// history bundle can be held to its admitter (O-75). Not necessarily who proposed the Add.
pub fn join_group_unsaved_from<G, K>(client: &ClientWith<G, K>, welcome: &[u8]) -> Result<(GroupWith<G, K>, [u8; 32]), CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    let w = MlsMessage::from_bytes(welcome).map_err(err)?;
    let (group, info) = client.join_group(None, &w, None).map_err(err)?;
    let committer = member_identity(&group, info.sender)?;
    Ok((group, committer))
}

/// Persist a group's current state (see [`join_group_unsaved`]).
pub fn save_group<G, K>(group: &mut GroupWith<G, K>) -> Result<(), CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    group.write_to_storage().map_err(err)
}

/// Pairwise routing tag = export_secret(label, epoch_be, 32). Both sides derive
/// identical bytes from the shared exporter at the same epoch.
pub fn relay_tag<G, K>(group: &GroupWith<G, K>, label: &[u8], epoch_be: &[u8]) -> Result<[u8; 32], CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    let secret = group.export_secret(label, epoch_be, 32).map_err(err)?;
    secret
        .as_bytes()
        .try_into()
        .map_err(|_| err("export_secret length != 32"))
}

/// The seal's HKDF salt: a distinct exporter so the tag and the seal key never
/// collide. label = "pacific/seal/v1".
pub fn seal_conn_secret<G, K>(group: &GroupWith<G, K>, epoch_be: &[u8]) -> Result<[u8; 32], CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    let secret = group
        .export_secret(crate::seal::SEAL_LABEL, epoch_be, 32)
        .map_err(err)?;
    secret
        .as_bytes()
        .try_into()
        .map_err(|_| err("seal secret length != 32"))
}

/// Encrypt a canonical-CBOR delta as an MLS application message; persist the
/// ratchet advance. Returns the wire bytes to seal + PUB.
pub fn encrypt_delta<G, K>(group: &mut GroupWith<G, K>, delta_cbor: &[u8]) -> Result<Vec<u8>, CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    let msg = group
        .encrypt_application_message(delta_cbor, Vec::new())
        .map_err(err)?;
    group.write_to_storage().map_err(err)?;
    msg.to_bytes().map_err(err)
}

/// The outcome of processing one inbound MLS message off the shared group tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    /// An application message (a Delta envelope) decrypted for us, tagged with the
    /// MLS-authenticated SENDER's identity pubkey. The author is never on the wire;
    /// it is the ratchet-tree leaf that signed the message — which is exactly what
    /// the commutative fold keys membership on.
    Application { sender: [u8; 32], data: Vec<u8> },
    /// A commit was applied: the epoch advanced, and the roster and context may have
    /// changed. Not a Delta to fold.
    Commit {
        /// The person who committed it.
        committer: [u8; 32],
        /// The epoch the group is at now.
        epoch: u64,
    },
    /// A proposal was received and cached. The epoch did NOT move; a commit is now
    /// required before this device may send (RFC 9420 §12.4), which is §7's job.
    Proposal {
        /// The person who proposed it.
        sender: [u8; 32],
        /// The leaf it removes, if it is a Remove.
        removes: Option<u32>,
    },
    /// A commit REMOVED this device (`CommitEffect::Removed`). mls-rs leaves the group
    /// at its old epoch; the caller must stop using it and delete its state (§8.3).
    Removed {
        /// The person who PROPOSED the removal — mls-rs's `remover` is the proposal's
        /// sender, not the committer. So `by` is this device's own person after a leave
        /// (§6), and the owner after a removal (§5): exactly the left-vs-removed
        /// distinction the membership record draws from authorship.
        by: [u8; 32],
    },
    /// Our OWN message, echoed back because every member drains the one shared
    /// group tag. mls-rs refuses to process a self-authored message; we skip it
    /// (the Delta is already in our local log). This is NOT a silent fallback —
    /// it matches ONLY the exact `CantProcessMessageFromSelf` case; every other
    /// error is still returned loudly. It is the leaf's, not necessarily this
    /// state's: a copy of this device sealed earlier sees the later copy's messages
    /// here too, which the sent ledger tells apart (D-34 (c)).
    SkippedOwn,
    /// A handshake from an epoch this group has already left, inside the retained
    /// window: the commit that moved it on, already applied, whoever wrote it.
    StaleHandshake,
}

/// Process an inbound MLS message off the shared group tag. Persists state only
/// when it actually changed (application ratchet advance / handshake), never on a
/// self-skip.
pub fn decrypt_message<G, K>(group: &mut GroupWith<G, K>, inner: &[u8]) -> Result<Incoming, CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    // A group whose floor is above this build is PAUSED, not judged (MIN_VERSION_EXT).
    check_floor(group)?;
    let msg = MlsMessage::from_bytes(inner).map_err(err)?;
    let msg_epoch = msg.epoch();
    let current = group.current_epoch();
    let received = match group.process_incoming_message(msg) {
        Ok(r) => r,
        Err(MlsError::CantProcessMessageFromSelf) => return Ok(Incoming::SkippedOwn),
        // A HANDSHAKE FROM AN EPOCH THIS GROUP HAS ALREADY LEFT. An old epoch's tag is
        // re-drained for late application messages, and the one commit on it is the
        // one that moved this group on — already applied, whoever wrote it. mls-rs
        // refuses any handshake off the current epoch, and refuses an application
        // message only below its retained window, so inside that window an
        // `InvalidEpoch` is that stale handshake and nothing else. Below it, it may be
        // content genuinely lost, and is reported as an error as before.
        Err(MlsError::InvalidEpoch)
            if msg_epoch.is_some_and(|e| e < current && e + crate::EPOCH_RETENTION >= current) =>
        {
            return Ok(Incoming::StaleHandshake)
        }
        Err(e) => return Err(err(e)),
    };
    let gid = hex::encode(group.group_id());
    let out = match received {
        ReceivedMessage::ApplicationMessage(app) => {
            let sender = member_identity(group, app.sender_index)?;
            Incoming::Application {
                sender,
                data: app.data().to_vec(),
            }
        }
        ReceivedMessage::Commit(c) => match c.effect {
            CommitEffect::Removed { remover, .. } => {
                // The group is still at the old epoch (mls-rs skips the key schedule
                // for a removed member), so the proposer's leaf is still in its roster.
                let by = match remover {
                    Sender::Member(i) => member_identity(group, i)?,
                    _ => [0u8; 32],
                };
                tracing::warn!(
                    target: "pacific::mls",
                    group = %gid,
                    by = %hex::encode(by),
                    epoch = group.current_epoch(),
                    "this device was removed from the group"
                );
                // Nothing is persisted: the caller deletes this group's state (§8.3).
                return Ok(Incoming::Removed { by });
            }
            CommitEffect::ReInit(_) => {
                // PacificRules refuses ReInit on send and receive, so a commit that
                // carries one cannot have been accepted; if it ever is, say so loudly.
                return Err(err("a ReInit commit was applied — Pacific never sends one"));
            }
            CommitEffect::NewEpoch(ne) => {
                let committer = member_identity(group, c.committer).unwrap_or([0u8; 32]);
                tracing::info!(
                    target: "pacific::mls",
                    group = %gid,
                    epoch = ne.epoch,
                    committer = %hex::encode(committer),
                    applied = ne.applied_proposals.len(),
                    unused = ne.unused_proposals.len(),
                    "commit applied"
                );
                Incoming::Commit { committer, epoch: ne.epoch }
            }
        },
        ReceivedMessage::Proposal(p) => {
            let sender = match p.sender {
                mls_rs::group::ProposalSender::Member(i) => member_identity(group, i)?,
                _ => return Err(err("a proposal from outside the group — Pacific never accepts one")),
            };
            let removes = match &p.proposal {
                mls_rs::group::proposal::Proposal::Remove(r) => Some(r.to_remove()),
                _ => None,
            };
            tracing::info!(
                target: "pacific::mls",
                group = %gid,
                epoch = group.current_epoch(),
                sender = %hex::encode(sender),
                removes = ?removes,
                "proposal cached — a commit is now required before sending"
            );
            Incoming::Proposal { sender, removes }
        }
        other => {
            return Err(err(format!(
                "unexpected message on a group tag: {}",
                match other {
                    ReceivedMessage::GroupInfo(_) => "GroupInfo",
                    ReceivedMessage::Welcome => "Welcome",
                    ReceivedMessage::KeyPackage(_) => "KeyPackage",
                    _ => "other",
                }
            )))
        }
    };
    group.write_to_storage().map_err(err)?;
    Ok(out)
}

/// Big-endian 8-byte epoch context for the exporter labels.
pub fn epoch_be(epoch: u64) -> [u8; 8] {
    epoch.to_be_bytes()
}

/// The N-member routing label: ONE shared per-epoch mailbox tag every member of
/// the group PUBs to and drains. Replaces the old 2-party directional lo2hi/hi2lo
/// scheme (which could not address 3+ parties). The relay sees one opaque tag per
/// group per epoch and cannot tell members apart — blindness is preserved.
pub const GROUP_LABEL: &[u8] = b"pacific/relay/v1/group";

/// THE ARCHIVE CONTENT KEY for one epoch — `keys.md` §4's third exporter, the one
/// it lists as *planned*.
///
/// The other two exporter outputs expire: the relay tag and the transport seal are
/// pruned at [`crate::EPOCH_RETENTION`], because a tag we can no longer decrypt is
/// a tag we must no longer drain. This one is RETAINED, and that is the whole of
/// the departure from RFC 9420 §9.2 — one 32-byte value per epoch per group, under
/// our own label, rather than the key schedule's own secrets.
///
/// Two properties fall out of the exporter being per-epoch, and both are free:
/// a REMOVED member cannot derive later epochs' content keys, and a NEW member
/// cannot derive earlier ones. No segments, no rotation, no bookkeeping.
///
/// CURRENT EPOCH ONLY. `export_secret` answers for the epoch the group is at and
/// no other, so this has to be taken while the group is there and written down
/// before the epoch advances — which is why `Node` records it on epoch ENTRY and
/// not on the way out. RFC 9420 §11 makes that unavoidable rather than merely
/// tidy: a group is created at epoch 0 with a fresh random epoch secret and NO
/// COMMIT, so there is no "on the way out of epoch 0" to hang a write on.
pub const ARCHIVE_LABEL: &[u8] = b"pacific/archive/v1";

/// The epoch's archive content key. See [`ARCHIVE_LABEL`].
pub fn archive_key<G, K>(group: &GroupWith<G, K>, epoch_be: &[u8]) -> Result<[u8; 32], CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    let secret = group
        .export_secret(ARCHIVE_LABEL, epoch_be, 32)
        .map_err(err)?;
    secret
        .as_bytes()
        .try_into()
        .map_err(|_| err("archive key length != 32"))
}

/// The shared group mailbox tag at `epoch_be`: the PUBLIC KEY of the epoch's address
/// ([`group_address`]). Every member derives it identically at the same epoch; a
/// subscriber, a cursor and the seal's destination binding all use this.
pub fn group_tag<G, K>(group: &GroupWith<G, K>, epoch_be: &[u8]) -> Result<[u8; 32], CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    Ok(group_address(group, epoch_be)?.tag())
}

/// The epoch's relay ADDRESS, with the key that writes it — what a publish needs.
///
/// The exporter output under `pacific/relay/v1/group` is the address's SEED: every
/// member at this epoch holds it and nobody else can, so exactly the members can
/// publish here (`pacific_wire::address`). It never leaves the device; the relay sees
/// only the public key. Before 18 Sep 2026 that exporter output WAS the tag, so
/// anyone who saw a tag on the wire could append to it and take its commit slot.
pub fn group_address<G, K>(
    group: &GroupWith<G, K>,
    epoch_be: &[u8],
) -> Result<pacific_wire::address::Address, CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    Ok(pacific_wire::address::Address::from_seed(&relay_tag(group, GROUP_LABEL, epoch_be)?))
}

/// The 32-byte Ed25519 identity pubkey a leaf carries as its BasicCredential id.
fn basic_id(sid: &SigningIdentity) -> Result<[u8; 32], CoreError> {
    let basic = sid
        .credential
        .as_basic()
        .ok_or_else(|| err("member credential is not a BasicCredential"))?;
    basic
        .identifier
        .as_slice()
        .try_into()
        .map_err(|_| err("member cred_id is not 32 bytes"))
}

/// The identity pubkey of the member at ratchet-tree leaf `index` (the MLS sender
/// of an application message).
fn member_identity<G, K>(group: &GroupWith<G, K>, index: u32) -> Result<[u8; 32], CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    let member = group.roster().member_with_index(index).map_err(err)?;
    basic_id(&member.signing_identity)
}

/// The group's current membership as identity pubkeys, read straight from the MLS
/// ratchet tree — the single source of truth for "who is a member". Each leaf's
/// BasicCredential carries the 32-byte Ed25519 identity pubkey as its cred_id.
pub fn roster_identities<G, K>(group: &GroupWith<G, K>) -> Result<Vec<[u8; 32]>, CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    group
        .roster()
        .members_iter()
        .map(|m| basic_id(&m.signing_identity))
        .collect()
}

/// The per-epoch authentication secret shared by all members — an MITM cannot
/// forge it. The basis for a real multi-party safety number (SAS); a later slice
/// renders it human-comparable. Exposed now because the primitive is free once we
/// hold the group.
pub fn epoch_authenticator<G, K>(group: &GroupWith<G, K>) -> Result<Vec<u8>, CoreError>
where
    G: GroupStore,
    K: KeyStore,
{
    let secret = group.epoch_authenticator().map_err(err)?;
    Ok(secret.as_bytes().to_vec())
}

// ── the commit rules, proven on real groups (membership-through-mls.md §15.1) ────────
//
// Every test here builds real MLS groups over in-memory storage — the same `mls.rs`
// code path the phone and the browser run. The SEND side is exercised through the
// honest doors (`stage_remove`, `stage_set_owner`, `stage_rename`); the RECEIVE side
// through a ROGUE client: the same member's stores, but mls-rs's DEFAULT rules, which
// is exactly what a modified client would run. A rule is only real if an honest device
// refuses what the rogue sends.
#[cfg(test)]
mod leaf_tests {
    use super::*;

    /// A leaf whose MLS signature key is the identity point is refused wherever a leaf
    /// is validated; an honest leaf of the same person is admitted (NC-46, depth).
    #[test]
    fn a_leaf_with_a_weak_signature_key_is_refused() {
        let person = crate::identity::Identity::in_memory([7u8; 32]).identity_pk();
        let honest = crate::identity::Identity::in_memory([8u8; 32]).identity_pk();
        let mut weak = [0u8; 32];
        weak[0] = 1;
        let v = |key: &[u8]| PerLeafIdentity.validate_member(&signing_identity(&person, key), None, MemberValidationContext::None);
        assert_eq!(v(&honest), Ok(()));
        assert_eq!(v(&weak), Err(LeafRefused::WeakKey));
        assert_eq!(
            PerLeafIdentity.validate_external_sender(&signing_identity(&person, &weak), None, None),
            Err(LeafRefused::WeakKey)
        );
    }
}

#[cfg(test)]
mod rules_tests {
    use super::*;
    use crate::mls_mem::{MemClient, MemGroup, MemGroupStateStorage, MemKeyPackageStorage};
    use mls_rs::group::proposal::{CustomProposal, Proposal, ProposalType};
    use mls_rs::mls_rules::ProposalSource;
    use mls_rs::WireFormat;

    struct Dev {
        gss: MemGroupStateStorage,
        kps: MemKeyPackageStorage,
        client: MemClient,
        cred: [u8; 32],
        sk: SecretKey,
        pk: Vec<u8>,
    }

    /// A device of person `cred`, on a build that advertises `exts`.
    fn dev_with(cred: [u8; 32], exts: &[ExtensionType]) -> Dev {
        let c = crypto();
        let (sk, pk) = generate_signing_key(&c).unwrap();
        let gss = MemGroupStateStorage::new();
        let kps = MemKeyPackageStorage::new();
        let client = client_over(&gss, &kps, cred, &sk, pk.as_bytes(), exts);
        Dev { gss, kps, client, cred, sk, pk: pk.as_bytes().to_vec() }
    }

    fn client_over(
        gss: &MemGroupStateStorage,
        kps: &MemKeyPackageStorage,
        cred: [u8; 32],
        sk: &SecretKey,
        pk: &[u8],
        exts: &[ExtensionType],
    ) -> MemClient {
        MlsClient::builder()
            .identity_provider(PerLeafIdentity)
            .crypto_provider(crypto())
            .group_state_storage(gss.clone())
            .key_package_repo(kps.clone())
            .mls_rules(PacificRules)
            .extension_types(exts.to_vec())
            .signing_identity(signing_identity(&cred, pk), sk.clone(), CIPHERSUITE)
            .build()
    }

    /// A device on THIS build.
    fn dev(seed: u8) -> Dev {
        dev_with([seed; 32], &[GROUP_NAME_EXT, OWNER_EXT, MIN_VERSION_EXT])
    }

    /// A second device of the same person: same credential, its own signing key.
    fn sibling(of: &Dev) -> Dev {
        dev_with(of.cred, &[GROUP_NAME_EXT, OWNER_EXT, MIN_VERSION_EXT])
    }

    /// A rogue client over `d`'s own stores with mls-rs's DEFAULT rules.
    macro_rules! rogue {
        ($d:expr) => {
            MlsClient::builder()
                .identity_provider(PerLeafIdentity)
                .crypto_provider(crypto())
                .group_state_storage($d.gss.clone())
                .key_package_repo($d.kps.clone())
                .extension_types([GROUP_NAME_EXT, OWNER_EXT, MIN_VERSION_EXT])
                .signing_identity(signing_identity(&$d.cred, &$d.pk), $d.sk.clone(), CIPHERSUITE)
                .build()
        };
    }

    fn gid(g: &MemGroup) -> Vec<u8> {
        g.group_id().to_vec()
    }

    /// `owner` creates a group and adds `others` one by one; every existing member
    /// processes each Add commit, and each newcomer joins from its Welcome. Returns
    /// every device's group, in the order given (owner first).
    fn group_of(owner: &Dev, others: &[&Dev]) -> Vec<MemGroup> {
        let mut groups = vec![create_group_named(&owner.client, "t").unwrap()];
        for d in others {
            let kp = make_key_package_bytes(&d.client).unwrap();
            let (commit, welcome) = add_member(&mut groups[0], &kp).unwrap();
            for g in groups.iter_mut().skip(1) {
                assert!(matches!(decrypt_message(g, &commit).unwrap(), Incoming::Commit { .. }));
            }
            groups.push(join_group(&d.client, &welcome).unwrap());
        }
        groups
    }

    /// NC-50: a joiner whose clock runs ahead of its adder's is admitted. Dated from the
    /// maker's own second, as mls-rs dates it, the same key package is refused.
    #[test]
    fn a_joiner_whose_clock_is_ahead_of_its_adder_is_admitted() {
        let (owner, ahead, dated_now) = (dev(1), dev(2), dev(3));
        let mut g = create_group_named(&owner.client, "t").unwrap();
        let theirs = MlsTime::now().seconds_since_epoch() + 5;
        let refused = dated_now
            .client
            .generate_key_package_message(ExtensionList::default(), ExtensionList::default(), Some(MlsTime::from(theirs)))
            .unwrap()
            .to_bytes()
            .unwrap();
        assert!(add_member(&mut g, &refused).is_err(), "dated from a clock 5 s ahead: refused, as NC-50 found");
        let kp = key_package_made_at(&ahead.client, theirs).unwrap();
        let (_commit, welcome) = add_member(&mut g, &kp).expect("a joiner 5 s ahead is admitted");
        join_group(&ahead.client, &welcome).unwrap();
    }

    fn apply_mine(g: &mut MemGroup, commit: &[u8], others: &mut [&mut MemGroup]) {
        apply_staged(g).unwrap();
        for o in others.iter_mut() {
            decrypt_message(o, commit).unwrap();
        }
    }

    fn people(g: &MemGroup) -> BTreeSet<[u8; 32]> {
        roster_identities(g).unwrap().into_iter().collect()
    }

    // ---- the owner in the context (§3) ------------------------------------------

    #[test]
    fn a_group_is_created_with_its_owner_and_floor_in_the_context() {
        let a = dev(0xA1);
        let g = create_group_named(&a.client, "t").unwrap();
        assert!(has_owner_ext(&g));
        assert_eq!(group_owner(&g).unwrap(), a.cred);
        assert_eq!(group_floor(&g).unwrap(), PROTOCOL_VERSION);
        assert_eq!(group_name(&g), "t", "the name still rides beside them");
    }

    #[test]
    fn pairing_with_an_old_build_starts_a_legacy_group_it_can_join() {
        // §3.2's bridge. A new peer gets the owner extension; a peer whose build
        // predates it gets a legacy group instead of a refusal, joins it, and reads
        // the creator as owner through leaf 0 — ready for §3.3 to migrate.
        let a = dev(0xA0);
        let new_peer = dev(0xB0);
        let kp = make_key_package_bytes(&new_peer.client).unwrap();
        let (g, _c, _w, legacy) = create_group_with_peer(&a.client, &kp).unwrap();
        assert!(!legacy && has_owner_ext(&g));

        let old_peer = dev_with([0xC0; 32], &[GROUP_NAME_EXT]);
        let kp = make_key_package_bytes(&old_peer.client).unwrap();
        let (mut g, _c, w, legacy) = create_group_with_peer(&a.client, &kp).unwrap();
        assert!(legacy, "an old build is paired, not refused");
        assert!(!has_owner_ext(&g));
        assert_eq!(group_owner(&g).unwrap(), a.cred, "leaf 0 is the creator");
        let mut go = join_group(&old_peer.client, &w).expect("the old build joins");
        let msg = encrypt_delta(&mut g, b"hello, old friend").unwrap();
        match decrypt_message(&mut go, &msg).unwrap() {
            Incoming::Application { data, .. } => assert_eq!(data, b"hello, old friend"),
            _ => panic!("the old build reads the group"),
        }
    }

    #[test]
    fn a_legacy_group_reads_its_owner_from_leaf_zero() {
        let a = dev(0xA2);
        let b = dev(0xB2);
        let mut g = a.client.create_group(ExtensionList::new(), ExtensionList::default(), None).unwrap();
        g.write_to_storage().unwrap();
        let kp = make_key_package_bytes(&b.client).unwrap();
        let (_c, w) = add_member(&mut g, &kp).unwrap();
        let gb = join_group(&b.client, &w).unwrap();
        assert!(!has_owner_ext(&gb));
        assert_eq!(group_owner(&gb).unwrap(), a.cred, "leaf 0 is the creator");
        assert_eq!(
            owner_in(gb.context().extensions(), &gb.roster()).unwrap().1,
            OwnerSource::LegacyLeafZero
        );
        assert_eq!(group_floor(&gb).unwrap(), 0, "no floor before the floor existed");
    }

    #[test]
    fn an_old_build_cannot_be_added_to_a_new_group() {
        // §3.2 — the rollout constraint, pinned where it bites.
        let a = dev(0xA3);
        let old = dev_with([0xB3; 32], &[GROUP_NAME_EXT]);
        let mut g = create_group_named(&a.client, "t").unwrap();
        let kp = make_key_package_bytes(&old.client).unwrap();
        assert!(
            add_member(&mut g, &kp).is_err(),
            "a leaf that does not advertise the owner extension cannot join a group that carries it"
        );
    }

    // ---- removal (§4.3, §5) -----------------------------------------------------

    #[test]
    fn the_owner_may_remove_a_member_who_learns_it() {
        let (a, b, c) = (dev(0xA4), dev(0xB4), dev(0xC4));
        let mut gs = group_of(&a, &[&b, &c]);
        let (ga, rest) = gs.split_at_mut(1);
        let (gb, gc) = rest.split_at_mut(1);
        let (ga, gb, gc) = (&mut ga[0], &mut gb[0], &mut gc[0]);

        let leaves = leaves_of(ga, &b.cred);
        let commit = stage_remove(ga, &leaves).unwrap();
        apply_staged(ga).unwrap();
        assert!(matches!(decrypt_message(gc, &commit).unwrap(), Incoming::Commit { .. }));
        match decrypt_message(gb, &commit).unwrap() {
            Incoming::Removed { by } => assert_eq!(by, a.cred, "B is told who removed it"),
            _ => panic!("B must learn it was removed"),
        }
        assert!(!people(ga).contains(&b.cred) && !people(gc).contains(&b.cred));
    }

    #[test]
    fn the_owner_may_remove_one_device_of_a_member() {
        let (a, b1) = (dev(0xA5), dev(0xB5));
        let b2 = sibling(&b1);
        let mut gs = group_of(&a, &[&b1, &b2]);
        let lost = my_leaf(&gs[2]);
        let commit = stage_remove(&mut gs[0], &[lost]).unwrap();
        let (ga, rest) = gs.split_at_mut(1);
        apply_mine(&mut ga[0], &commit, &mut [&mut rest[0]]);
        assert_eq!(leaves_of(&ga[0], &b1.cred).len(), 1, "B keeps its other device");
    }

    #[test]
    fn a_member_cannot_remove_another_member_on_send() {
        let (a, b, c) = (dev(0xA6), dev(0xB6), dev(0xC6));
        let mut gs = group_of(&a, &[&b, &c]);
        let leaves = leaves_of(&gs[1], &c.cred);
        let e = stage_remove(&mut gs[1], &leaves).unwrap_err().to_string();
        assert!(e.contains("neither the owner"), "{e}");
    }

    #[test]
    fn a_rogue_member_removing_another_is_refused_by_every_honest_member() {
        let (a, b, c) = (dev(0xA7), dev(0xB7), dev(0xC7));
        let mut gs = group_of(&a, &[&b, &c]);
        let rb = rogue!(b);
        let mut rg = rb.load_group(&gid(&gs[1])).unwrap();
        let leaf_c = leaves_of(&gs[2], &c.cred)[0];
        let forged = rg.commit_builder().remove_member(leaf_c).unwrap().build().unwrap();
        let bytes = forged.commit_message.to_bytes().unwrap();
        for i in [0usize, 2] {
            let e = decrypt_message(&mut gs[i], &bytes).unwrap_err().to_string();
            assert!(e.contains("neither the owner"), "{e}");
        }
        assert!(people(&gs[0]).contains(&c.cred), "C is still in the room");
    }

    #[test]
    fn a_rogue_member_removing_the_owner_is_refused() {
        let (a, b, c) = (dev(0xA8), dev(0xB8), dev(0xC8));
        let mut gs = group_of(&a, &[&b, &c]);
        let rb = rogue!(b);
        let mut rg = rb.load_group(&gid(&gs[1])).unwrap();
        let forged = rg.commit_builder().remove_member(0).unwrap().build().unwrap();
        let bytes = forged.commit_message.to_bytes().unwrap();
        for i in [0usize, 2] {
            let e = decrypt_message(&mut gs[i], &bytes).unwrap_err().to_string();
            assert!(e.contains("owner cannot be removed"), "{e}");
        }
    }

    #[test]
    fn a_remove_in_a_legacy_group_is_refused() {
        let (a, b) = (dev(0xA9), dev(0xB9));
        let mut g = a.client.create_group(ExtensionList::new(), ExtensionList::default(), None).unwrap();
        g.write_to_storage().unwrap();
        let kp = make_key_package_bytes(&b.client).unwrap();
        let (_c, w) = add_member(&mut g, &kp).unwrap();
        let _gb = join_group(&b.client, &w).unwrap();
        let leaves = leaves_of(&g, &b.cred);
        let e = stage_remove(&mut g, &leaves).unwrap_err().to_string();
        assert!(e.contains("predates the owner extension"), "{e}");
    }

    // ---- leaving (§4.3 b, §6, §7) ------------------------------------------------

    #[test]
    fn a_leave_proposed_by_the_leaver_is_committed_by_another_member() {
        let (a, b, c) = (dev(0xAA), dev(0xBA), dev(0xCA));
        let mut gs = group_of(&a, &[&b, &c]);
        let epoch = gs[0].current_epoch();
        let mine = my_leaf(&gs[1]);
        let proposal = propose_remove(&mut gs[1], mine).unwrap();
        assert_eq!(MlsMessage::from_bytes(&proposal).unwrap().wire_format(), WireFormat::PrivateMessage);
        assert!(commit_required(&gs[1]), "the leaver can no longer send");

        for i in [0usize, 2] {
            match decrypt_message(&mut gs[i], &proposal).unwrap() {
                Incoming::Proposal { sender, removes } => {
                    assert_eq!(sender, b.cred);
                    assert_eq!(removes, Some(mine));
                }
                _ => panic!("a proposal must arrive as one"),
            }
            assert_eq!(gs[i].current_epoch(), epoch, "a proposal does not move the epoch");
            assert!(commit_required(&gs[i]));
        }
        assert!(pending_removal_of(&gs[1], &b.cred));

        // C — not the owner — completes it: the rules authorize the PROPOSER.
        let commit = stage_rekey(&mut gs[2]).unwrap();
        apply_staged(&mut gs[2]).unwrap();
        assert!(matches!(decrypt_message(&mut gs[0], &commit).unwrap(), Incoming::Commit { .. }));
        match decrypt_message(&mut gs[1], &commit).unwrap() {
            // `by` is who PROPOSED it — the leaver — though C committed it.
            Incoming::Removed { by } => assert_eq!(by, b.cred, "a leave reads as a leave"),
            _ => panic!("the leaver must learn the leave completed"),
        }
        assert!(!people(&gs[0]).contains(&b.cred) && !people(&gs[2]).contains(&b.cred));
        assert!(!commit_required(&gs[0]) && !commit_required(&gs[2]));
    }

    #[test]
    fn leaving_from_one_device_takes_every_device_of_that_person() {
        let (a, p1, c) = (dev(0xAB), dev(0xBB), dev(0xCB));
        let p2 = sibling(&p1);
        let mut gs = group_of(&a, &[&p1, &p2, &c]);
        let leaves = leaves_of(&gs[1], &p1.cred);
        assert_eq!(leaves.len(), 2);
        let proposals: Vec<Vec<u8>> = leaves.iter().map(|l| propose_remove(&mut gs[1], *l).unwrap()).collect();
        for i in [0usize, 2, 3] {
            for p in &proposals {
                decrypt_message(&mut gs[i], p).unwrap();
            }
        }
        assert!(pending_removal_of(&gs[2], &p1.cred), "the sibling sees its own removal coming");
        let commit = stage_rekey(&mut gs[3]).unwrap();
        apply_staged(&mut gs[3]).unwrap();
        decrypt_message(&mut gs[0], &commit).unwrap();
        for i in [1usize, 2] {
            assert!(matches!(decrypt_message(&mut gs[i], &commit).unwrap(), Incoming::Removed { .. }));
        }
        assert!(!people(&gs[0]).contains(&p1.cred), "the person is gone, both devices");
    }

    #[test]
    fn a_remove_another_member_proposed_does_not_start_anyones_departure() {
        // B, not the owner, proposes C's removal. Nothing checks a proposal until a
        // commit carries it, so it is cached everywhere — and C must not read it as
        // its own leave (§6.4) or stop committing because of it, or any member could
        // make any other member leave.
        let (a, b, c) = (dev(0xAD), dev(0xBD), dev(0xCD));
        let mut gs = group_of(&a, &[&b, &c]);
        let leaf_c = my_leaf(&gs[2]);
        let forged = propose_remove(&mut gs[1], leaf_c).unwrap();
        for i in [0usize, 2] {
            decrypt_message(&mut gs[i], &forged).unwrap();
            assert!(commit_required(&gs[i]), "it is cached like any other proposal");
        }
        assert!(!pending_leave_of(&gs[2], &c.cred), "C is not leaving");
        assert!(!pending_removal_of(&gs[2], &c.cred), "and nothing that will stand removes it");

        // C commits the cache itself: the rules drop B's proposal, and C stays.
        let commit = stage_rekey(&mut gs[2]).unwrap();
        apply_staged(&mut gs[2]).unwrap();
        assert!(matches!(decrypt_message(&mut gs[0], &commit).unwrap(), Incoming::Commit { .. }));
        assert!(people(&gs[0]).contains(&c.cred) && people(&gs[2]).contains(&c.cred));
        assert!(!commit_required(&gs[0]) && !commit_required(&gs[2]), "the commit cleared the cache");
    }

    #[test]
    fn a_remove_the_owner_proposed_stands_but_is_not_a_leave() {
        let (a, b) = (dev(0xAE), dev(0xBE));
        let mut gs = group_of(&a, &[&b]);
        let leaf_b = my_leaf(&gs[1]);
        let p = propose_remove(&mut gs[0], leaf_b).unwrap();
        decrypt_message(&mut gs[1], &p).unwrap();
        assert!(pending_removal_of(&gs[1], &b.cred), "B must not commit its own removal");
        assert!(!pending_leave_of(&gs[1], &b.cred), "but B is being removed, not leaving");
    }

    /// resumption.md §8: a person may remove ONE of their own leaves — what evicting an
    /// expired pool leaf needs. Rule (b), which demanded every leaf at once, is retired.
    #[test]
    fn a_person_may_remove_one_of_their_own_leaves() {
        let (a, p1, c) = (dev(0xAC), dev(0xBC), dev(0xCC));
        let p2 = sibling(&p1);
        let mut gs = group_of(&a, &[&p1, &p2, &c]);
        // P2 proposes removing P1 — the same person's other leaf.
        let leaf_p1 = my_leaf(&gs[1]);
        let proposal = propose_remove(&mut gs[2], leaf_p1).unwrap();
        for i in [0usize, 1, 3] {
            decrypt_message(&mut gs[i], &proposal).unwrap();
        }
        // C commits it: kept on send, accepted on receive.
        let commit = stage_rekey(&mut gs[3]).unwrap();
        apply_staged(&mut gs[3]).unwrap();
        assert_eq!(leaves_of(&gs[3], &p1.cred).len(), 1, "one of the person's leaves went");
        decrypt_message(&mut gs[0], &commit).unwrap();
        decrypt_message(&mut gs[2], &commit).unwrap();
        assert_eq!(leaves_of(&gs[0], &p1.cred).len(), 1);
        assert!(people(&gs[0]).contains(&p1.cred), "and the person is still in the room");
    }

    /// The owner may remove their own leaves too, but never the last one: that is the
    /// owner leaving, and the owner hands over first.
    #[test]
    fn the_owner_cannot_remove_their_own_last_leaf() {
        let (a, b) = (dev(0xAD), dev(0xBD));
        let mut gs = group_of(&a, &[&b]);
        let leaf_a = my_leaf(&gs[0]);
        let proposal = propose_remove(&mut gs[0], leaf_a).unwrap();
        decrypt_message(&mut gs[1], &proposal).unwrap();
        // An honest committer drops it; the commit builds and removes nobody.
        let commit = stage_rekey(&mut gs[1]).unwrap();
        apply_staged(&mut gs[1]).unwrap();
        assert!(people(&gs[1]).contains(&a.cred), "the owner is still in");
        decrypt_message(&mut gs[0], &commit).unwrap();
    }

    #[test]
    fn a_bad_by_reference_proposal_is_dropped_when_committing_not_fatal() {
        let (a, b, c) = (dev(0xAD), dev(0xBD), dev(0xCD));
        let mut gs = group_of(&a, &[&b, &c]);
        // B (not the owner) proposes removing C, by reference, through a rogue client.
        let rb = rogue!(b);
        let mut rg = rb.load_group(&gid(&gs[1])).unwrap();
        let leaf_c = leaves_of(&gs[2], &c.cred)[0];
        let bad = rg.propose_remove(leaf_c, Vec::new()).unwrap().to_bytes().unwrap();
        decrypt_message(&mut gs[0], &bad).unwrap();
        assert!(commit_required(&gs[0]));
        // The OWNER sweeps its cache — and must not lend its authority to B's proposal.
        let commit = stage_rekey(&mut gs[0]).unwrap();
        apply_staged(&mut gs[0]).unwrap();
        assert!(people(&gs[0]).contains(&c.cred), "C was not removed by B's proposal");
        assert!(!commit_required(&gs[0]), "and the group can write again");
        decrypt_message(&mut gs[2], &bad).unwrap();
        assert!(matches!(decrypt_message(&mut gs[2], &commit).unwrap(), Incoming::Commit { .. }));
    }

    #[test]
    fn two_devices_proposing_the_same_removes_commit_them_once() {
        let (a, p1) = (dev(0xAE), dev(0xBE));
        let p2 = sibling(&p1);
        let mut gs = group_of(&a, &[&p1, &p2]);
        let leaves = leaves_of(&gs[1], &p1.cred);
        let mut all = Vec::new();
        for i in [1usize, 2] {
            for l in &leaves {
                all.push((i, propose_remove(&mut gs[i], *l).unwrap()));
            }
        }
        for (from, p) in &all {
            for i in [0usize, 1, 2] {
                if i != *from {
                    decrypt_message(&mut gs[i], p).unwrap();
                }
            }
        }
        let commit = stage_rekey(&mut gs[0]).unwrap();
        apply_staged(&mut gs[0]).unwrap();
        assert!(!people(&gs[0]).contains(&p1.cred));
        for i in [1usize, 2] {
            assert!(matches!(decrypt_message(&mut gs[i], &commit).unwrap(), Incoming::Removed { .. }));
        }
    }

    // ---- the context: rename, handover, migration, floor (§4.3, §9, §3.3) --------

    #[test]
    fn only_the_owner_may_rename() {
        let (a, b) = (dev(0xAF), dev(0xBF));
        let mut gs = group_of(&a, &[&b]);
        let e = stage_rename(&mut gs[1], "mine now").unwrap_err().to_string();
        assert!(e.contains("only the owner"), "{e}");
        stage_rename(&mut gs[0], "still ours").unwrap();
    }

    #[test]
    fn the_owner_hands_over_and_the_new_owner_takes_the_powers() {
        let (a, b, c) = (dev(0x11), dev(0x21), dev(0x31));
        let mut gs = group_of(&a, &[&b, &c]);
        let commit = stage_set_owner(&mut gs[0], &b.cred).unwrap();
        {
            let (ga, rest) = gs.split_at_mut(1);
            let (gb, gc) = rest.split_at_mut(1);
            apply_mine(&mut ga[0], &commit, &mut [&mut gb[0], &mut gc[0]]);
        }
        for g in &gs {
            assert_eq!(group_owner(g).unwrap(), b.cred);
        }
        // The old owner lost the power to remove; the new owner has it.
        let leaves = leaves_of(&gs[0], &c.cred);
        assert!(stage_remove(&mut gs[0], &leaves).is_err());
        let a_leaves = leaves_of(&gs[1], &a.cred);
        stage_remove(&mut gs[1], &a_leaves).expect("the new owner may remove the old one");
    }

    #[test]
    fn a_handover_cannot_ride_with_a_remove() {
        // One commit may not both move the owner and remove someone: the remove would be
        // judged against an owner the same commit is replacing (§4.3).
        let (a, b, c) = (dev(0x13), dev(0x23), dev(0x33));
        let mut gs = group_of(&a, &[&b, &c]);
        let leaf_c = my_leaf(&gs[2]);
        let mut exts = gs[0].context().extensions().clone();
        exts.set(Extension::new(OWNER_EXT, b.cred.to_vec()));
        let e = gs[0]
            .commit_builder()
            .remove_member(leaf_c)
            .unwrap()
            .set_group_context_ext(exts.clone())
            .unwrap()
            .build()
            .map(|_| ())
            .unwrap_err()
            .to_string();
        assert!(e.contains("may not ride"), "refused on send: {e}");

        let ra = rogue!(a);
        let mut rg = ra.load_group(&gid(&gs[0])).unwrap();
        let forged = rg
            .commit_builder()
            .remove_member(leaf_c)
            .unwrap()
            .set_group_context_ext(exts)
            .unwrap()
            .build()
            .unwrap()
            .commit_message
            .to_bytes()
            .unwrap();
        let e = decrypt_message(&mut gs[1], &forged).unwrap_err().to_string();
        assert!(e.contains("may not ride"), "and on receive: {e}");
    }

    #[test]
    fn a_handover_must_name_someone_in_the_tree() {
        let (a, b) = (dev(0x12), dev(0x22));
        let mut gs = group_of(&a, &[&b]);
        let e = stage_set_owner(&mut gs[0], &[0x99; 32]).unwrap_err().to_string();
        assert!(e.contains("holds a leaf"), "{e}");
    }

    #[test]
    fn a_rogue_handover_is_refused_on_receive() {
        let (a, b) = (dev(0x13), dev(0x23));
        let mut gs = group_of(&a, &[&b]);
        let rb = rogue!(b);
        let mut rg = rb.load_group(&gid(&gs[1])).unwrap();
        let mut exts = rg.context().extensions().clone();
        exts.set(Extension::new(OWNER_EXT, b.cred.to_vec()));
        let forged = rg
            .commit_builder()
            .set_group_context_ext(exts)
            .unwrap()
            .build()
            .unwrap()
            .commit_message
            .to_bytes()
            .unwrap();
        let e = decrypt_message(&mut gs[0], &forged).unwrap_err().to_string();
        assert!(e.contains("only the owner"), "{e}");
        assert_eq!(group_owner(&gs[0]).unwrap(), a.cred);
    }

    #[test]
    fn a_self_update_refreshes_a_leafs_capabilities_and_the_legacy_owner_migrates() {
        // §3.3's premise, verified before anything relies on it.
        let a = dev(0x14);
        let old = dev_with([0x24; 32], &[GROUP_NAME_EXT]);
        let mut ga = a.client.create_group(ExtensionList::new(), ExtensionList::default(), None).unwrap();
        ga.write_to_storage().unwrap();
        let kp = make_key_package_bytes(&old.client).unwrap();
        let (_c, w) = add_member(&mut ga, &kp).unwrap();
        let _gb_old = join_group(&old.client, &w).unwrap();
        assert!(!every_leaf_supports_owner_ext(&ga));
        assert!(stage_set_owner(&mut ga, &a.cred).is_err(), "not while a leaf cannot carry it");
        let _ = load_group(&a.client, &gid(&ga)).map(|g| ga = g);

        // The old device updates its app: same stores, same key, the new build.
        let new_client = client_over(
            &old.gss,
            &old.kps,
            old.cred,
            &old.sk,
            &old.pk,
            &[GROUP_NAME_EXT, OWNER_EXT, MIN_VERSION_EXT],
        );
        let mut gb = load_group(&new_client, &gid(&ga)).unwrap();
        assert!(!my_leaf_supports_owner_ext(&gb));
        let refresh = stage_rekey(&mut gb).unwrap();
        apply_staged(&mut gb).unwrap();
        assert!(my_leaf_supports_owner_ext(&gb), "a path commit rebuilds the leaf from the new build");
        decrypt_message(&mut ga, &refresh).unwrap();
        assert!(every_leaf_supports_owner_ext(&ga));

        // The legacy member may not migrate the group to itself…
        assert!(stage_set_owner(&mut gb, &old.cred).is_err());
        let _ = load_group(&new_client, &gid(&ga)).map(|g| gb = g);
        // …the leaf-0 owner may.
        let m = stage_set_owner(&mut ga, &a.cred).unwrap();
        apply_staged(&mut ga).unwrap();
        decrypt_message(&mut gb, &m).unwrap();
        assert!(has_owner_ext(&gb));
        assert_eq!(group_owner(&gb).unwrap(), a.cred);
        assert_eq!(group_floor(&gb).unwrap(), PROTOCOL_VERSION, "the floor arrives with the owner");
    }

    #[test]
    fn the_floor_only_rises_and_a_group_above_this_build_pauses() {
        let a = dev(0x15);
        let mut g = create_group_named(&a.client, "t").unwrap();
        let mut lower = g.context().extensions().clone();
        lower.set(Extension::new(MIN_VERSION_EXT, 0u16.to_be_bytes().to_vec()));
        let r = g.commit_builder().set_group_context_ext(lower).unwrap().build();
        assert!(r.is_err(), "the floor may only rise");

        let mut exts = genesis_ctx_exts("t", &a.cred);
        exts.set(Extension::new(MIN_VERSION_EXT, 99u16.to_be_bytes().to_vec()));
        let mut paused = a.client.create_group(exts, ExtensionList::default(), None).unwrap();
        paused.write_to_storage().unwrap();
        match check_floor(&paused) {
            Err(CoreError::UpgradeRequired(m)) => assert!(m.contains("update to continue"), "{m}"),
            other => panic!("expected a pause, got {other:?}"),
        }
    }

    // ---- the rules as a function, and the options (§4.2, §4.4) --------------------

    #[test]
    fn external_commits_and_custom_proposals_are_refused() {
        let a = dev(0x16);
        let g = create_group_named(&a.client, "t").unwrap();
        let roster = g.roster();
        let me = roster.member_with_index(0).unwrap();
        let r = PacificRules.filter_proposals(
            CommitDirection::Receive,
            CommitSource::NewMember(me.signing_identity.clone()),
            &roster,
            g.context(),
            ProposalBundle::default(),
        );
        assert_eq!(r.unwrap_err(), RuleViolation::ExternalCommit);

        let mut b = ProposalBundle::default();
        b.add(
            Proposal::Custom(CustomProposal::new(ProposalType::new(0xF0AA), vec![])),
            Sender::Member(0),
            ProposalSource::ByValue,
        );
        let r = PacificRules.filter_proposals(
            CommitDirection::Receive,
            CommitSource::ExistingMember(me),
            &roster,
            g.context(),
            b,
        );
        assert_eq!(r.unwrap_err(), RuleViolation::RefusedProposal("custom"));
    }

    #[test]
    fn every_commit_carries_a_path_and_travels_encrypted() {
        let (a, b) = (dev(0x17), dev(0x27));
        let g = create_group_named(&a.client, "t").unwrap();
        let opts = PacificRules.commit_options(&g.roster(), g.context(), &ProposalBundle::default()).unwrap();
        assert!(opts.path_required, "an Add-only commit refreshes its committer too");
        let mut gs = group_of(&a, &[&b]);
        let commit = stage_rekey(&mut gs[0]).unwrap();
        assert_eq!(MlsMessage::from_bytes(&commit).unwrap().wire_format(), WireFormat::PrivateMessage);
    }

    #[test]
    fn the_rules_table_names_every_proposal_type_once() {
        let names: BTreeSet<&str> = COMMIT_RULES.iter().map(|(k, _)| *k).collect();
        assert_eq!(names.len(), COMMIT_RULES.len());
        for (_, rule) in COMMIT_RULES {
            assert!(["anyMember", "self", "ownerOrOwnLeaves", "owner", "refused"].contains(&rule));
        }
    }
}
