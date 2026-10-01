//! **P1 — RESUME A POOLED LEAF FROM TWENTY-FOUR WORDS.** The one claim the whole
//! recovery design rests on, run end to end against real cryptography.
//!
//! THE CLAIM. A device that has never existed before — holding nothing but the
//! account's 24 words — reaches a leaf that is ALREADY in an MLS group, resumes
//! its ratchet at the current epoch, reads what was said while it did not exist,
//! and speaks. **Without publishing a commit.** No Add, no epoch transition, no
//! other member online, no doorbell, no external commit.
//!
//! That last clause is the invariant, and §6 asserts it by counting handshake
//! messages on the wire across the whole recovery: `Add`ing the recovering device
//! would be visible to every member of every group on every sign-in, which is the
//! cost `leaves-and-the-archive.html` §04 refuses when it says "No external commit
//! anywhere".
//!
//! WHAT IS REAL HERE. Everything cryptographic:
//!   * MLS is the vendored `mls-rs` 0.55.4 through `pacific_core::mls` — the same
//!     `build_client` / `add_member` / `join_group` / `encrypt_delta` calls
//!     `node.rs` makes, with `PerLeafIdentity` so one person may hold two leaves.
//!   * Derivations are real HKDF-SHA256; `acct` is a real Ed25519 key and the arc
//!     really verifies its signature; chain entries and the head are really sealed
//!     with XChaCha20-Poly1305 and really refuse to open under the wrong key.
//!   * The 24 words are real BIP-39 over the seed, through `Identity`.
//!
//! WHAT IS A STAND-IN, AND SAYS SO. `Relay` and `Arc` below are in-process maps,
//! not the Semaphore relay and not the kenjin auth service. They implement the two
//! behaviours the design actually leans on — append-and-replay by opaque tag, and
//! first-write-claims with a compare-and-set on a declared position — and nothing
//! else. **They prove the protocol closes, not that the services exist.** Neither
//! service has these routes today.
//!
//! WHAT IS NOT DEMONSTRATED. Reading back past the join epoch (that needs the
//! retained `pacific/archive/v1` content key, which has no writer yet), leasing
//! (no lease exists anywhere in the tree), and rotation ordering. §5 pins the
//! forward-secrecy boundary that read-back would have to cross.

use std::collections::{BTreeMap, BTreeSet};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

use pacific_core::head::{self, Head, HEAD_VERSION};
use pacific_core::identity;
use pacific_core::mls::{self, Incoming};
use pacific_core::mls_mem::{
    build_client_mem, MemClient, MemGroup, MemGroupStateStorage, MemKeyPackageStorage, Snapshot,
};

// ══════════════════════════════════════════════════════ MODULE B · the roots ══
//
// FOLDED ONTO `pacific_core::locator`, 18 Sep 2026. This module used to carry its
// own `kdf`, its own label constants and its own `acct`, and that was the exact
// failure `locator.rs`'s own doc warns about: two derivations of one address put
// two of a person's devices on different rows with nothing anywhere reporting an
// error. The test written to prove resume-from-words was holding the second
// definition.
//
// The labels moved rather than the test: `locator.rs` was using
// `pacific/locator/v1` and `pacific/arc-write/v1` and deriving straight from the
// seed, which mixes the two HKDF families that `five-things.html` §06 ruled
// disjoint on 16 September. This file had it right and the module did not. Now
// there is one of them.

use pacific_core::locator::{self, ArcWrite, Record};

fn kdf(ikm: &[u8], label: &[u8]) -> [u8; 32] {
    let hk = hkdf::Hkdf::<Sha256>::new(Some(label), ikm);
    let mut out = [0u8; 32];
    hk.expand(b"v1", &mut out).expect("32-byte OKM");
    out
}

/// The one label this prototype still owns: the chain entry SEAL, which is not an
/// address and so is not in `Record`. Same family, same shape.
const F_CHAIN_SEAL: &[u8] = b"pacific/storage/chain/seal/v1";

fn identity_root(seed: &[u8; 32]) -> [u8; 32] {
    *locator::identity_root(seed)
}
fn storage_root(seed: &[u8; 32]) -> [u8; 32] {
    *locator::storage_root(seed)
}

// ═══════════════════════════════════════════ MODULE C · locators and authority ══
//
// `the-seam.html` §01. `pk` is the identity and NOT an address: every row is
// addressed by a locator derived from the storage root, and every write is
// authorised by `acct`, which has no computable relation to `pk`. The arc can hold
// both halves of a person and be unable to tell.

fn acct_key(sr: &[u8; 32]) -> SigningKey {
    locator::acct_key(sr)
}
fn head_locator(sr: &[u8; 32]) -> [u8; 32] {
    locator::locator(sr, Record::Head).expect("head is root-derived")
}
/// The first tag of chain generation `gen`. A reader takes `gen` from the head it
/// has just opened — there is no generation-free chain address to fall back on.
fn chain_tag0(sr: &[u8; 32], gen: u32) -> [u8; 32] {
    locator::chain_locator(sr, gen, 0)
}

/// The generation this account writes. A fresh account's first chain is 0; the
/// head carries it so a restarted chain is findable from the root alone.
const GEN: u32 = 0;

fn chain_seal_key(sr: &[u8; 32]) -> [u8; 32] {
    kdf(sr, F_CHAIN_SEAL)
}

// ═══════════════════════════════════════════════════════ the sealed envelope ══
//
// Same shape as the wrap, the history and the head — MAGIC(4) ‖ nonce(24) ‖ ct —
// so one length check and one magic serves all of them at the arc.

const CHAIN_MAGIC: &[u8; 4] = b"PCH1";

fn seal(plain: &[u8], key: &[u8; 32], magic: &[u8; 4]) -> Vec<u8> {
    let mut nonce = [0u8; 24];
    getrandom::getrandom(&mut nonce).expect("entropy");
    let ct = XChaCha20Poly1305::new(key.into())
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad: magic })
        .expect("seal");
    let mut out = Vec::with_capacity(4 + 24 + ct.len());
    out.extend_from_slice(magic);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    out
}

fn unseal(blob: &[u8], key: &[u8; 32], magic: &[u8; 4]) -> Result<Vec<u8>, ()> {
    if blob.len() < 28 || &blob[..4] != magic {
        return Err(());
    }
    XChaCha20Poly1305::new(key.into())
        .decrypt(
            XNonce::from_slice(&blob[4..28]),
            Payload { msg: &blob[28..], aad: magic },
        )
        .map_err(|_| ())
}

// ══════════════════════════════════════════════════════════ THE STAND-INS ══
//
// NOT the Semaphore relay and NOT the kenjin auth service. See the module header.

/// Append-and-replay by opaque tag. The relay never learns what a tag means and
/// never opens a blob; that is the whole of what it does here.
#[derive(Default)]
struct Relay {
    /// tag -> [(kind, blob)] in publication order. The relay never learns what a
    /// tag means and never opens a blob; `kind` exists only so the trace can name
    /// the objects, and is not visible to a real relay.
    blobs: BTreeMap<[u8; 32], Vec<(&'static str, Vec<u8>)>>,
    order: Vec<[u8; 32]>,
    /// Every handshake message ever published, so §6 can count them.
    handshakes: usize,
}

impl Relay {
    fn publish(&mut self, tag: [u8; 32], kind: &'static str, blob: Vec<u8>) {
        if !self.blobs.contains_key(&tag) {
            self.order.push(tag);
        }
        self.blobs.entry(tag).or_default().push((kind, blob));
    }
    fn publish_handshake(&mut self, tag: [u8; 32], kind: &'static str, blob: Vec<u8>) {
        self.handshakes += 1;
        self.publish(tag, kind, blob);
    }
    fn drain(&self, tag: &[u8; 32]) -> Vec<Vec<u8>> {
        self.blobs
            .get(tag)
            .map(|v| v.iter().map(|(_, b)| b.clone()).collect())
            .unwrap_or_default()
    }
    fn objects(&self) -> Vec<Obj> {
        let mut out = Vec::new();
        for tag in &self.order {
            for (kind, blob) in &self.blobs[tag] {
                out.push(Obj { at: h8(tag), kind: (*kind).into(), bytes: blob.len(), note: String::new() });
            }
        }
        out
    }
}

struct Row {
    blob: Vec<u8>,
    position: u64,
    writer: VerifyingKey,
}

/// The host this stand-in answers as. Inside every signature, so one captured
/// here does not verify at another arc.
const ARC_HOST: &str = "arc.example";

/// First-write-claims with a compare-and-set on a position it cannot decrypt.
/// The arc holds no seed, so it cannot verify that a locator belongs to anybody —
/// recording the first claimant is the most it can do (`the-seam.html` §01).
#[derive(Default)]
struct Arc_ {
    rows: BTreeMap<[u8; 32], Row>,
    /// Issued and not yet spent. A nonce is single-use, which is what stops a
    /// captured write being replayed — and only the arc can enforce that, which
    /// is why it is modelled here rather than left to the signature.
    live_nonces: BTreeSet<String>,
}

impl Arc_ {
    fn nonce(&mut self) -> String {
        let n = hex::encode(rand32());
        self.live_nonces.insert(n.clone());
        n
    }

    fn put(
        &mut self,
        loc: [u8; 32],
        blob: Vec<u8>,
        position: u64,
        nonce: &str,
        writer: VerifyingKey,
        sig: Signature,
    ) -> Result<(), String> {
        if !self.live_nonces.remove(nonce) {
            return Err("challenge is unknown, expired or already used".into());
        }
        let msg = Self::to_sign(&loc, nonce, &blob);
        writer.verify(&msg, &sig).map_err(|_| "bad signature".to_string())?;
        if let Some(existing) = self.rows.get(&loc) {
            if existing.writer != writer {
                return Err("locator is claimed by another key".into());
            }
            if position <= existing.position {
                return Err(format!(
                    "stale write: declared {position}, stored {}",
                    existing.position
                ));
            }
        }
        self.rows.insert(loc, Row { blob, position, writer });
        Ok(())
    }

    /// Ungated, by design: an anchor nobody can watch is an anchor an arc can
    /// move. Returns the ciphertext and the position it DECLARES — which the
    /// client then holds it to, via `head::open_declared`.
    fn get(&self, loc: &[u8; 32]) -> Option<(Vec<u8>, u64)> {
        self.rows.get(loc).map(|r| (r.blob.clone(), r.position))
    }

    fn objects(&self) -> Vec<Obj> {
        self.rows
            .iter()
            .map(|(loc, r)| Obj {
                at: h8(loc),
                kind: "head".into(),
                bytes: r.blob.len(),
                note: format!("position={} writer={}", r.position, h8(r.writer.as_bytes())),
            })
            .collect()
    }

    /// `locator::write_payload`, not a local shape.
    ///
    /// This used to sign `pacific/storage/put/v1 ‖ loc ‖ position ‖ sha256(blob)`,
    /// which binds the address and the bytes and NEITHER an audience nor a nonce
    /// — so a captured signature verified at any arc, and verified again
    /// tomorrow. Because the body is bound the replay is idempotent, which makes
    /// it rollback rather than forgery, and that is not better.
    ///
    /// The POSITION leaves the signature and that is correct rather than a loss:
    /// it is sealed inside the head, the arc declares it on GET, and
    /// `head::open_declared` fails loudly when the two disagree. The seal binds
    /// it, so the signature does not have to.
    fn to_sign(loc: &[u8; 32], nonce: &str, blob: &[u8]) -> Vec<u8> {
        locator::write_payload(&ArcWrite {
            audience: ARC_HOST,
            locator: loc,
            nonce,
            body: blob,
        })
        .expect("no newline in a fixed audience or a hex nonce")
    }
}

// ═══════════════════════════════════════════════════════════ chain entries ══
//
// `leaves-and-the-archive.html` §04's kinds, minus the two this prototype does not
// exercise. Each entry names the NEXT tag, so only the first is seed-derived and
// the relay sees unrelated single-entry tags — the unlinkability argument in
// `keys.md` §6, which is why this is not an indexed scheme.

#[derive(serde::Serialize, serde::Deserialize, Debug)]
enum Kind {
    /// The group id and its home arc. Written on join.
    Group { group_id: Vec<u8>, arc: String },
    /// The Welcome that put this pooled leaf in the tree. Without it a group
    /// joined while offline is unreachable.
    Welcome { welcome: Vec<u8> },
    /// A pooled leaf's private material: the signature key it speaks with, and
    /// the key-package privates the Welcome is sealed to. Written BEFORE the
    /// commit that would rotate it, never after.
    Leaf {
        sig_sk: Vec<u8>,
        sig_pk: Vec<u8>,
        kp_id: Vec<u8>,
        kp_data: Vec<u8>,
    },
    /// A commit's ciphertext — it is what rebuilds the tree.
    Commit { commit: Vec<u8> },
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Entry {
    payload: Kind,
    /// Unguessable without this entry. The head is the only derivable tag.
    next: [u8; 32],
}

// ═══════════════════════════════════════════════════════════════ the trace ══
//
// Emitted so the protocol document renders the ACTUAL state of every object at
// every step rather than a description of it. `P1_TRACE=<path> cargo test …`

fn h8(b: &[u8]) -> String {
    hex::encode(&b[..4.min(b.len())])
}

#[derive(serde::Serialize)]
struct Field { k: String, v: String }
#[derive(serde::Serialize)]
struct Actor { id: String, user: String, fields: Vec<Field> }
#[derive(serde::Serialize)]
struct Obj { at: String, kind: String, bytes: usize, note: String }
#[derive(serde::Serialize)]
struct Step {
    n: usize,
    phase: String,
    title: String,
    call: String,
    actors: Vec<Actor>,
    relay: Vec<Obj>,
    arc: Vec<Obj>,
}

fn field(k: &str, v: impl Into<String>) -> Field {
    Field { k: k.into(), v: v.into() }
}

/// A device's MLS view, as fields. `None` = this device holds no group.
fn mls_fields(g: Option<&MemGroup>) -> Vec<Field> {
    match g {
        None => vec![field("mls", "—")],
        Some(g) => {
            let leaves = mls::roster_identities(g).unwrap();
            let mut people: Vec<[u8; 32]> = leaves.clone();
            people.sort();
            people.dedup();
            vec![
                field("group_id", h8(g.group_id())),
                field("epoch", g.current_epoch().to_string()),
                field("leaf_index", g.current_member_index().to_string()),
                field("leaves", format!("{} ({} people)", leaves.len(), people.len())),
            ]
        }
    }
}

fn actor(id: &str, user: &str, mut base: Vec<Field>, g: Option<&MemGroup>) -> Actor {
    base.extend(mls_fields(g));
    Actor { id: id.into(), user: user.into(), fields: base }
}

fn rand32() -> [u8; 32] {
    let mut b = [0u8; 32];
    getrandom::getrandom(&mut b).expect("entropy");
    b
}

/// Append entries to the relay, hash-chained. Returns (position, tail-hash).
fn write_chain(relay: &mut Relay, sr: &[u8; 32], gen: u32, payloads: Vec<Kind>) -> (u64, [u8; 32]) {
    let key = chain_seal_key(sr);
    let mut tag = chain_tag0(sr, gen);
    let mut tail = [0u8; 32];
    for (i, payload) in payloads.iter().enumerate() {
        let next = if i + 1 == payloads.len() { [0u8; 32] } else { rand32() };
        let plain = serde_json::to_vec(&Entry { payload: clone_payload(payload), next }).unwrap();
        let blob = seal(&plain, &key, CHAIN_MAGIC);
        tail = Sha256::digest(&blob).into();
        let kind = match payload {
            Kind::Group { .. } => "chain:group",
            Kind::Welcome { .. } => "chain:welcome",
            Kind::Leaf { .. } => "chain:leaf",
            Kind::Commit { .. } => "chain:commit",
        };
        relay.publish(tag, kind, blob);
        tag = next;
    }
    (payloads.len() as u64, tail)
}

fn clone_payload(p: &Kind) -> Kind {
    serde_json::from_slice(&serde_json::to_vec(p).unwrap()).unwrap()
}

/// Walk from the seed-derived first tag, following each entry's `next`, and
/// return the entries plus the hash of the LAST ciphertext seen.
fn walk_chain(relay: &Relay, sr: &[u8; 32], gen: u32) -> (Vec<Kind>, u64, [u8; 32]) {
    let key = chain_seal_key(sr);
    let mut tag = chain_tag0(sr, gen);
    let (mut out, mut n, mut tail) = (Vec::new(), 0u64, [0u8; 32]);
    loop {
        let blobs = relay.drain(&tag);
        let Some(blob) = blobs.first() else { break };
        let plain = unseal(blob, &key, CHAIN_MAGIC).expect("a chain entry must open under the seed");
        let entry: Entry = serde_json::from_slice(&plain).unwrap();
        tail = Sha256::digest(blob).into();
        n += 1;
        out.push(entry.payload);
        if entry.next == [0u8; 32] {
            break;
        }
        tag = entry.next;
    }
    (out, n, tail)
}

// ═════════════════════════════════════════════════════════════════ devices ══

struct Device {
    gss: MemGroupStateStorage,
    kps: MemKeyPackageStorage,
    client: MemClient,
}

fn device(cred_id: &[u8; 32]) -> (Device, Vec<u8>, Vec<u8>) {
    let crypto = mls::crypto();
    let (sk, pk) = mls::generate_signing_key(&crypto).unwrap();
    let sid = mls::signing_identity(cred_id, pk.as_bytes());
    let gss = MemGroupStateStorage::new();
    let kps = MemKeyPackageStorage::new();
    let client = build_client_mem(gss.clone(), kps.clone(), sid, sk.clone()).unwrap();
    (Device { gss, kps, client }, sk.as_bytes().to_vec(), pk.as_bytes().to_vec())
}

/// The group's relay tag at one epoch — a real MLS exporter, the same label
/// `node.rs` uses.
fn tag_at(group: &MemGroup) -> [u8; 32] {
    let e = mls::epoch_be(group.current_epoch());
    mls::group_tag(group, &e).unwrap()
}

#[test]
fn a_device_holding_only_the_words_resumes_a_pooled_leaf_and_speaks() {
    let mut relay = Relay::default();
    let mut arc = Arc_::default();
    let arc_url = "wss://arc.example";

    // ── 1 · the account ────────────────────────────────────────────────────────
    // Real BIP-39 over a real 32-byte seed. `seed = f(24 words)` — no name in the
    // derivation, ruled 17 Sep.
    let seed = rand32();
    let id = identity::Identity::in_memory(seed);
    let words = id.recovery_key().expect("a seeded identity has words");
    assert_eq!(words.split_whitespace().count(), 24, "BIP-39, 24 words");

    let ada_cred = id.identity_pk();
    let sr = storage_root(&seed);
    let ir = identity_root(&seed);
    assert_ne!(sr, ir, "the two roots must be independent");

    let mut trace: Vec<Step> = Vec::new();
    let acct0 = acct_key(&sr);
    let phone_acct = || vec![
        field("seed", h8(&seed)),
        field("words", format!("{}…", words.split_whitespace().take(2).collect::<Vec<_>>().join(" "))),
        field("identity_root", h8(&ir)),
        field("storage_root", h8(&sr)),
    ];
    let phone_loc = || {
        let mut v = phone_acct();
        v.push(field("acct", h8(acct0.verifying_key().as_bytes())));
        v.push(field("L_head", h8(&head_locator(&sr))));
        v.push(field("chain_tag0", h8(&chain_tag0(&sr, GEN))));
        v
    };
    macro_rules! snap {
        ($phase:expr, $title:expr, $call:expr, $actors:expr) => {
            trace.push(Step { n: trace.len() + 1, phase: $phase.into(), title: $title.into(),
                              call: $call.into(), actors: $actors,
                              relay: relay.objects(), arc: arc.objects() });
        };
    }
    snap!("provision", "THE ACCOUNT",
        "seed = 32 B · words = recovery_key_from_seed(seed) · identity_root, storage_root = HKDF(seed, …)",
        vec![actor("ada.phone", "ada", phone_acct(), None)]);
    snap!("provision", "LOCATORS AND AUTHORITY",
        "acct = Ed25519(HKDF(sr,\"…/acct/v1\")) · L_head = HKDF(sr,\"…/head/v1\") · chain_tag0 = HKDF(sr,\"…/chain/v1\" ‖ gen)",
        vec![actor("ada.phone", "ada", phone_loc(), None)]);

    // ── 2 · a real group, and a pooled leaf provisioned into it ────────────────
    let (phone, _, _) = device(&ada_cred);
    let (bob, _, _) = device(&[0xBB; 32]);

    let mut g_phone = mls::create_group_named(&phone.client, "Kitchen").unwrap();

    let bob_kp = mls::make_key_package_bytes(&bob.client).unwrap();
    // A commit is published to the tag of the epoch it LEAVES — that is the tag
    // every current member is draining when it arrives.
    let t_before_bob = tag_at(&g_phone);
    let (c1, w_bob) = mls::add_member(&mut g_phone, &bob_kp).unwrap();
    relay.publish_handshake(t_before_bob, "commit:add-bob", c1);
    let mut g_bob = mls::join_group(&bob.client, &w_bob).unwrap();
    snap!("provision", "A REAL GROUP",
        "create_group_named(&phone.client,\"Kitchen\") · add_member(&mut g_phone,&bob_kp) · join_group(&bob.client,&w_bob)",
        vec![actor("ada.phone", "ada", phone_loc(), Some(&g_phone)),
             actor("bob", "bob", vec![], Some(&g_bob))]);

    // Said while the group was phone + Bob only. The pooled leaf does not exist
    // yet, so no device that recovers through it may ever read this.
    let t_early = tag_at(&g_phone);
    let said_early = mls::encrypt_delta(&mut g_phone, b"before the pooled leaf existed").unwrap();
    relay.publish(t_early, "app", said_early.clone());
    assert!(matches!(mls::decrypt_message(&mut g_bob, &said_early).unwrap(), Incoming::Application { .. }));
    snap!("provision", "SAID BEFORE THE POOLED LEAF",
        "encrypt_delta(&mut g_phone, b\"before the pooled leaf existed\")",
        vec![actor("ada.phone", "ada", phone_loc(), Some(&g_phone)),
             actor("bob", "bob", vec![], Some(&g_bob))]);

    // The pool leaf: Ada's credential, its OWN signature key. Legal MLS — RFC 9420
    // §7.3 requires signature_key and encryption_key to be unique among members,
    // not the credential — and reachable because `PerLeafIdentity` is the provider.
    let (pool, pool_sk, pool_pk) = device(&ada_cred);
    let pool_kp = mls::make_key_package_bytes(&pool.client).unwrap();
    let pool_snapshot = Snapshot::capture(&pool.gss, &pool.kps).unwrap();
    let (kp_id, kp_data) = pool_snapshot.key_packages[0].clone();

    let tag_before_pool = tag_at(&g_phone);
    let (c2, w_pool) = mls::add_member(&mut g_phone, &pool_kp).unwrap();
    relay.publish_handshake(tag_before_pool, "commit:add-pool", c2.clone());
    assert!(matches!(mls::decrypt_message(&mut g_bob, &c2).unwrap(), Incoming::Commit { .. }));
    snap!("provision", "PROVISION A POOLED LEAF",
        "pool kp: cred_id = ada, own signature key · add_member(&mut g_phone,&pool_kp) → (commit, welcome)",
        vec![actor("ada.phone", "ada", phone_loc(), Some(&g_phone)),
             actor("ada.pool", "ada", vec![field("kp_id", h8(&kp_id)), field("kp_data", format!("{} B", kp_data.len())),
                   field("sig_pk", h8(&pool_pk)), field("status", "in the tree, unheld")], None),
             actor("bob", "bob", vec![], Some(&g_bob))]);

    // ── 3 · traffic, and one epoch the laptop will have to climb ───────────────
    let t_join = tag_at(&g_phone);
    let said_at_join = mls::encrypt_delta(&mut g_phone, b"said at the epoch the leaf joined").unwrap();
    relay.publish(t_join, "app", said_at_join.clone());
    assert!(matches!(mls::decrypt_message(&mut g_bob, &said_at_join).unwrap(), Incoming::Application { .. }));
    snap!("provision", "TRAFFIC AT THE JOIN EPOCH",
        "encrypt_delta(&mut g_phone, b\"said at the epoch the leaf joined\")",
        vec![actor("ada.phone", "ada", phone_loc(), Some(&g_phone)),
             actor("bob", "bob", vec![], Some(&g_bob))]);

    // A rekey commit AFTER the pool leaf was added: the laptop must replay this to
    // reach the current epoch. This is the rung `sync_group` climbs for free.
    let t_rekey = tag_at(&g_phone);
    let c3 = mls::stage_rekey(&mut g_phone).unwrap();
    mls::apply_staged(&mut g_phone).unwrap();
    relay.publish_handshake(t_rekey, "commit:rekey", c3.clone());
    assert!(matches!(mls::decrypt_message(&mut g_bob, &c3).unwrap(), Incoming::Commit { .. }));

    let t_now = tag_at(&g_bob);
    let bob_said = mls::encrypt_delta(&mut g_bob, b"bob, after the rekey").unwrap();
    relay.publish(t_now, "app", bob_said.clone());
    assert!(matches!(mls::decrypt_message(&mut g_phone, &bob_said).unwrap(), Incoming::Application { .. }));
    snap!("provision", "AN EPOCH TO CLIMB",
        "stage_rekey + apply_staged → commit · then Bob speaks at the new epoch",
        vec![actor("ada.phone", "ada", phone_loc(), Some(&g_phone)),
             actor("bob", "bob", vec![], Some(&g_bob))]);

    // ── 4 · the chain, and the head that anchors it ────────────────────────────
    let (position, tail) = write_chain(
        &mut relay,
        &sr,
        GEN,
        vec![
            Kind::Group { group_id: g_phone.group_id().to_vec(), arc: arc_url.into() },
            Kind::Welcome { welcome: w_pool },
            Kind::Leaf { sig_sk: pool_sk, sig_pk: pool_pk, kp_id, kp_data },
            Kind::Commit { commit: c3 },
        ],
    );

    snap!("provision", "WRITE THE CHAIN",
        "4 entries, each sealed under chain_key, each naming the next tag · first tag = chain_tag0",
        vec![actor("ada.phone", "ada", phone_loc(), Some(&g_phone)),
             actor("bob", "bob", vec![], Some(&g_bob))]);
    let head = Head { v: HEAD_VERSION, arc: arc_url.into(), position, tail: tail.to_vec(), chain_gen: GEN };
    let sealed_head = head::seal(&head, &sr).unwrap();
    let acct = acct_key(&sr);
    let loc = head_locator(&sr);
    let n1 = arc.nonce();
    let sig = acct.sign(&Arc_::to_sign(&loc, &n1, &sealed_head));
    arc.put(loc, sealed_head, position, &n1, acct.verifying_key(), sig).unwrap();

    // First-write-claims really refuses a stranger and really refuses a stale write.
    let stranger = SigningKey::from_bytes(&rand32());
    let n2 = arc.nonce();
    let s2 = stranger.sign(&Arc_::to_sign(&loc, &n2, b"x"));
    assert!(arc.put(loc, b"x".to_vec(), position + 1, &n2, stranger.verifying_key(), s2).is_err());
    let n3 = arc.nonce();
    let s3 = acct.sign(&Arc_::to_sign(&loc, &n3, b"y"));
    assert!(arc.put(loc, b"y".to_vec(), position, &n3, acct.verifying_key(), s3).is_err());

    // AND THE REPLAY, which is what the nonce is for: the same bytes and the same
    // signature under a challenge the arc has already spent. Without it a capture
    // verifies forever, and because the body is bound the replay is idempotent —
    // which makes it rollback rather than forgery, and that is not better.
    //
    // On its own locator, deliberately. Doing this against L_head would leave the
    // row declaring a position the sealed blob does not carry, and the walk below
    // would then fail in `head::open_declared` — correctly, but for a reason this
    // control is not about. (It did, the first time.)
    let spare = locator::locator(&sr, Record::Index).unwrap();
    let n4 = arc.nonce();
    let s4 = acct.sign(&Arc_::to_sign(&spare, &n4, b"once"));
    arc.put(spare, b"once".to_vec(), 1, &n4, acct.verifying_key(), s4)
        .expect("a fresh challenge is accepted");
    assert!(
        arc.put(spare, b"once".to_vec(), 2, &n4, acct.verifying_key(), s4).is_err(),
        "a spent challenge must not be replayable"
    );

    // ── NEGATIVE CONTROLS ──────────────────────────────────────────────────────
    // The seal is real: a chain entry does not open under a seed that is not this
    // account's. If this ever passes, the entries are not actually sealed.
    {
        let wrong = chain_seal_key(&storage_root(&rand32()));
        let first = relay.drain(&chain_tag0(&sr, GEN)).remove(0);
        assert!(unseal(&first, &wrong, CHAIN_MAGIC).is_err(), "a chain entry must not open under a stranger's seed");
        assert!(head::open(&arc.get(&loc).unwrap().0, &rand32()).is_err(), "nor must the head");
    }


    snap!("provision", "WRITE THE HEAD",
        "head::seal(&Head{position,tail}, &sr) · arc.put(L_head, blob, position, acct.vk(), sig)",
        vec![actor("ada.phone", "ada", phone_loc(), Some(&g_phone)),
             actor("bob", "bob", vec![], Some(&g_bob))]);    let handshakes_before_recovery = relay.handshakes;

    // ══ 5 · A DEVICE THAT HAS NEVER EXISTED, HOLDING ONLY THE WORDS ════════════
    let recovered = identity::seed_from_recovery_key(&words).expect("24 words are the seed");
    assert_eq!(recovered, seed, "the words ARE the key, not a backup of it");
    let sr2 = storage_root(&recovered);
    let lap0 = || vec![field("words", "the same 24"), field("seed", h8(&recovered)),
                       field("storage_root", h8(&sr2)), field("L_head", h8(&head_locator(&sr2)))];
    snap!("recover", "A DEVICE THAT NEVER EXISTED",
        "seed_from_recovery_key(words) → storage_root → L_head, chain_key, acct — chain_tag0 waits for the head's chain_gen",
        vec![actor("ada.laptop", "ada", lap0(), None),
             actor("ada.phone", "ada", phone_loc(), Some(&g_phone)),
             actor("bob", "bob", vec![], Some(&g_bob))]);

    // The head read is ungated, and the arc is held to the position it published.
    let (blob, declared) = arc.get(&head_locator(&sr2)).expect("head row");
    let head2 = head::open_declared(&blob, &sr2, declared).expect("head opens under the seed");
    snap!("recover", "OPEN THE ANCHOR",
        "arc.get(L_head) → (blob, declared) · head::open_declared(&blob,&sr2,declared)",
        vec![actor("ada.laptop", "ada", { let mut v = lap0();
                 v.push(field("head.position", head2.position.to_string()));
                 v.push(field("head.tail", h8(&head2.tail))); v.push(field("head.arc", head2.arc.clone()));
                 v.push(field("head.chain_gen", head2.chain_gen.to_string()));
                 v.push(field("chain_tag0", h8(&chain_tag0(&sr2, head2.chain_gen)))); v }, None),
             actor("bob", "bob", vec![], Some(&g_bob))]);

    // The generation comes from the head, never assumed: a restarted chain lives
    // somewhere new, and the next generation's address holds nothing yet.
    let (entries, walked, walked_tail) = walk_chain(&relay, &sr2, head2.chain_gen);
    assert_eq!(walk_chain(&relay, &sr2, head2.chain_gen + 1).1, 0,
               "the next generation is a different address, and nothing is there yet");
    assert_eq!(walked, head2.position, "chain length must match the anchor");
    assert_eq!(walked_tail.as_slice(), head2.tail.as_slice(), "ChainVerdict::Whole");
    assert_eq!(head2.check(walked, &walked_tail), head::ChainVerdict::Whole);
    snap!("recover", "WALK THE CHAIN",
        "tag = chain_tag0 → unseal → entry.next → … · head.check(walked, H(last ct))",
        vec![actor("ada.laptop", "ada", { let mut v = lap0();
                 v.push(field("entries", walked.to_string()));
                 v.push(field("walked_tail", h8(&walked_tail)));
                 v.push(field("verdict", format!("{:?}", head2.check(walked, &walked_tail)))); v }, None),
             actor("bob", "bob", vec![], Some(&g_bob))]);

    let mut welcome = None;
    let mut leaf = None;
    let mut commits = Vec::new();
    for p in entries {
        match p {
            Kind::Welcome { welcome: w } => welcome = Some(w),
            Kind::Leaf { sig_sk, sig_pk, kp_id, kp_data } => leaf = Some((sig_sk, sig_pk, kp_id, kp_data)),
            Kind::Commit { commit } => commits.push(commit),
            Kind::Group { .. } => {}
        }
    }
    let (sig_sk, sig_pk, kp_id, kp_data) = leaf.expect("a leaf entry");

    // Restore ONLY the pooled leaf's key package. No group state, no epoch
    // secrets, no ratchet tree — the laptop rebuilds the tree itself, below.
    let restored = Snapshot { states: vec![], epochs: vec![], key_packages: vec![(kp_id, kp_data)] };
    let (lap_gss, lap_kps) = restored.restore();
    let lap_sid = mls::signing_identity(&ada_cred, &sig_pk);
    let lap_sk = mls::SecretKey::from(sig_sk.clone());
    let laptop = build_client_mem(lap_gss, lap_kps, lap_sid, lap_sk).unwrap();

    // The `leaf` entry is load-bearing: without the key-package privates the
    // Welcome is sealed to, there is no way in at all. This is the assertion that
    // says the chain carries something that matters.
    {
        let bare = Snapshot::default().restore();
        let stranded = build_client_mem(
            bare.0,
            bare.1,
            mls::signing_identity(&ada_cred, &sig_pk),
            mls::SecretKey::from(sig_sk.clone()),
        )
        .unwrap();
        assert!(
            mls::join_group(&stranded, welcome.as_ref().unwrap()).is_err(),
            "the Welcome must be unusable without the pooled leaf's key package"
        );
    }

    // Join at the epoch the pool leaf was added, then CLIMB.
    let mut g_lap = mls::join_group(&laptop, &welcome.expect("a welcome entry")).unwrap();
    for c in &commits {
        assert!(matches!(mls::decrypt_message(&mut g_lap, c).unwrap(), Incoming::Commit { .. }));
    }
    assert_eq!(g_lap.current_epoch(), g_bob.current_epoch(), "the laptop is at the current epoch");
    snap!("recover", "RESTORE, JOIN, CLIMB",
        "restore ONLY the key package · join_group(&laptop,&welcome) · replay archived commit(s)",
        vec![actor("ada.laptop", "ada", vec![field("restored", "1 key package · 0 group states"),
                 field("commits_replayed", commits.len().to_string())], Some(&g_lap)),
             actor("bob", "bob", vec![], Some(&g_bob))]);

    // It reads what was said while it did not exist.
    match mls::decrypt_message(&mut g_lap, &bob_said).unwrap() {
        Incoming::Application { data, .. } => assert_eq!(data, b"bob, after the rekey"),
        _ => panic!("expected the application message Bob sent after the rekey"),
    }

    // Everything from the join epoch forward is readable...
    match mls::decrypt_message(&mut g_lap, &said_at_join).unwrap() {
        Incoming::Application { data, .. } => assert_eq!(data, b"said at the epoch the leaf joined"),
        _ => panic!("the join epoch is readable"),
    }

    // ...and the forward-secrecy boundary holds exactly there. A message from
    // before the pooled leaf was in the tree does NOT open, and must not: reading
    // back past this point is what the retained `pacific/archive/v1` content key
    // is for, and nothing writes one yet.
    assert!(
        mls::decrypt_message(&mut g_lap, &said_early).is_err(),
        "forward secrecy: nothing from before the pooled leaf joined may be readable"
    );
    snap!("recover", "READ, AND THE BOUNDARY",
        "decrypt(bob_said) ✓ · decrypt(said_at_join) ✓ · decrypt(said_early) → Err",
        vec![actor("ada.laptop", "ada", vec![field("reads epoch 3", "yes"), field("reads join epoch", "yes"),
                 field("reads earlier", "NO — forward secrecy")], Some(&g_lap)),
             actor("bob", "bob", vec![], Some(&g_bob))]);

    // ── 6 · IT SPEAKS, AND IT PUBLISHED NO COMMIT ──────────────────────────────
    let out = mls::encrypt_delta(&mut g_lap, b"from a device that had only the words").unwrap();
    relay.publish(tag_at(&g_lap), "app", out.clone());

    match mls::decrypt_message(&mut g_bob, &out).unwrap() {
        Incoming::Application { sender, data } => {
            assert_eq!(data, b"from a device that had only the words");
            assert_eq!(sender, ada_cred, "a different leaf, the same person");
        }
        _ => panic!("Bob must read the recovered device's message"),
    }

    assert_eq!(
        relay.handshakes, handshakes_before_recovery,
        "THE INVARIANT: recovery published no handshake. No Add, no commit, no epoch \
         transition — the recovering device was invisible to every member until it spoke."
    );
    snap!("recover", "SPEAK — AND NOTHING WAS PUBLISHED",
        "encrypt_delta(&mut g_lap, …) · assert_eq!(relay.handshakes, handshakes_before_recovery)",
        vec![actor("ada.laptop", "ada", vec![field("spoke as", h8(&ada_cred)),
                 field("handshakes published", "0")], Some(&g_lap)),
             actor("ada.phone", "ada", phone_loc(), Some(&g_phone)),
             actor("bob", "bob", vec![], Some(&g_bob))]);

    if let Ok(path) = std::env::var("P1_TRACE") {
        std::fs::write(&path, serde_json::to_vec_pretty(&trace).unwrap()).unwrap();
        println!("     trace: {} steps → {path}", trace.len());
    }

    println!("\n  P1 · a device holding only 24 words resumed a pooled leaf");
    println!("     chain: {} entries, verdict Whole, tail {}", walked, hex::encode(&walked_tail[..8]));
    println!("     epoch: climbed {} commit(s) to reach {}", commits.len(), g_lap.current_epoch());
    println!("     handshakes published during recovery: {}\n", relay.handshakes - handshakes_before_recovery);
}
