//! THE LEAF-IMPERSONATION HOLE A1a OPENED, and the check that closes it.
//!
//! ## What was actually attested
//!
//! `handshake::parse_and_verify` runs exactly one check:
//!
//! ```ignore
//! identity::verify_sig(&b.identity_pk, &b.signing_bytes()?, &b.sig)?;
//! ```
//!
//! That attests "the holder of `identity_pk` signed these bundle bytes". It does
//! not look inside `key_package`, and the two fields are independent: anybody
//! holding their OWN identity key can sign a perfectly well-formed bundle wrapped
//! around SOMEBODY ELSE'S credential. `node.rs::add_member_core` then handed
//! `key_package` straight to `mls::stage_add_member`, and at a public door
//! `admits_by_any_member` lets any member call it.
//!
//! The leaf that lands carries the VICTIM'S identity pubkey as its BasicCredential
//! id — which is what `mls::member_identity` reads, which is what the commutative
//! fold keys authorship and AUTHORITY on. The attacker does not merely get in;
//! they get in AS SOMEONE ELSE.
//!
//! ## Why this is A1a's blocker and not an old bug
//!
//! Before A1a it was covered BY ACCIDENT. `BasicIdentityProvider::identity()`
//! returns the credential verbatim, so mls-rs's tree-index uniqueness map refused
//! any leaf whose credential was already in the tree (`DuplicateLeafData`).
//! `mls::PerLeafIdentity` returns credential ‖ signature_key PRECISELY so that two
//! leaves may share one credential — that is A1a's whole purpose — and the
//! accidental refusal went with it. `mls_no_longer_refuses_the_forgery` below is
//! the proof of that, at the library layer, so nobody has to take it on trust.
//!
//! ## The shape of the fix these tests pin
//!
//! Credential EQUALITY, and nothing else, in `add_member_core`. It must not look at
//! the leaf's signature key: an honest second device brings a fresh signature key
//! and the SAME credential. So the last test here adds ada's laptop and bo's phone
//! through the very same door and requires both to succeed — a check that refuses
//! the forgery by refusing every duplicate credential would pass the first two
//! tests and destroy the feature it was written to protect.

mod common;

use common::Harness;
use pacific_core::mls_store::migrate;
use pacific_core::{handshake, mls, CoreError};

/// THE FORGING PRIMITIVE — `m24_leaf_pool.rs::leaf_of`, but built through Pacific's
/// OWN client builder so what it emits is byte-for-byte what an attacker's patched
/// client would paste: a real one-time KeyPackage, correctly self-signed by a fresh
/// leaf signature key, whose BasicCredential identifier is `claimed_id` — a value
/// the minter need not hold any key for.
///
/// The `TempDir` is returned, not dropped: it owns the SQLite store the key
/// package's private half lives in, and a dropped tempdir would make this a test of
/// a dangling file rather than of a forgery.
fn key_package_claiming(claimed_id: &[u8; 32]) -> (tempfile::TempDir, Vec<u8>) {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("pacific.db");
    migrate(&db).unwrap();
    let (sk, pk) = mls::generate_signing_key(&mls::crypto()).unwrap();
    let sid = mls::signing_identity(claimed_id, pk.as_bytes());
    let client =
        mls::build_client_sqlite(&db, sid, mls::SecretKey::new(sk.as_bytes().to_vec())).unwrap();
    let kp = mls::make_key_package_bytes(&client).unwrap();
    (tmp, kp)
}

/// THE HOLE, AT THE LIBRARY LAYER — the control for the two gates below.
///
/// A group founded by ada admits a leaf that ada never minted and holds no key for,
/// and afterwards the roster reads {ada, ada}: the attacker is indistinguishable
/// from ada's second device. Nothing in mls-rs objects, because `PerLeafIdentity`
/// keys the tree index on credential ‖ signature_key and the two signature keys
/// differ.
///
/// This test asserts the LIBRARY behaviour deliberately — it calls `mls::add_member`
/// directly, never `Node` — so it stays green after the fix and keeps saying why the
/// fix has to exist above MLS. If it ever starts failing, mls-rs has taken the
/// refusal back and A1a is at risk, which is worth being told about.
#[test]
fn mls_no_longer_refuses_the_forgery_so_pacific_must() {
    let ada = [0xAD; 32];

    // ada's own founding leaf, minted honestly.
    let founder_tmp = tempfile::tempdir().unwrap();
    let founder_db = founder_tmp.path().join("pacific.db");
    migrate(&founder_db).unwrap();
    let (sk, pk) = mls::generate_signing_key(&mls::crypto()).unwrap();
    let founder = mls::build_client_sqlite(
        &founder_db,
        mls::signing_identity(&ada, pk.as_bytes()),
        mls::SecretKey::new(sk.as_bytes().to_vec()),
    )
    .unwrap();
    let mut group = mls::create_group(&founder).unwrap();

    // mallory mints a key package that SAYS ada, holding none of ada's keys.
    let (_tmp, forged) = key_package_claiming(&ada);
    assert_eq!(
        handshake::key_package_cred_id(&forged).unwrap(),
        ada,
        "the premise: the forged key package's leaf credential is ADA's identity key"
    );

    mls::add_member(&mut group, &forged).expect(
        "PREMISE BROKEN: mls-rs refused the forged leaf. If this is \
         `DuplicateLeafData` the client builder is back on `BasicIdentityProvider` \
         and A1a has been reverted — in which case the gates below are testing a \
         hole that no longer exists, and `gate_a1a_*` should be red too.",
    );

    let roster = mls::roster_identities(&group).unwrap();
    assert_eq!(
        roster,
        vec![ada, ada],
        "and this is the damage: two leaves, both claiming ada, one of them \
         mallory's. Every delta the second leaf authors resolves to ada through \
         `mls::member_identity`, and the fold keys AUTHORITY on that value."
    );
}

/// GATE 1 — a bundle whose signature verifies is still refused when the key package
/// inside it speaks for somebody else.
///
/// mallory signs the bundle with mallory's own identity key, so every check that
/// existed before this change passes: the base64 decodes, the version matches, the
/// Ed25519 signature verifies against `identity_pk`. The only thing wrong with it is
/// the one thing nothing was looking at.
///
/// HOW THIS COULD BE FAKED. (a) Refuse the add for some unrelated reason — closed by
/// asserting the refusal names `ERR_KEY_PACKAGE_IDENTITY`, and by asserting first
/// that `parse_and_verify` ACCEPTS this bundle, so the refusal cannot be coming from
/// the signature check. (b) Refuse it only because mallory is a stranger — closed
/// because the caller is the group's OWNER, who may add anyone. (c) Let the add
/// half-happen and clean up afterwards — closed by asserting the epoch has not
/// moved and the roster is unchanged, so no commit was ever staged.
#[tokio::test]
async fn an_owner_refuses_a_bundle_whose_key_package_names_somebody_else() {
    let h = Harness::people(&[("ada", &["phone"]), ("mallory", &["laptop"])]).await;
    let phone = h.device("ada", "phone");
    let laptop = h.device("mallory", "laptop");
    let ada_id = h.id(phone);
    let mallory_id = h.id(laptop);
    assert_ne!(ada_id, mallory_id);

    let studio = h.form_named_forum(phone, "the studio");
    let epoch_before = h.epoch(phone, &studio);
    let roster_before = h.roster(phone, &studio);

    // ── mallory's forgery: mallory's signature, ada's credential ──────────────
    let (_tmp, forged_kp) = key_package_claiming(&ada_id);
    let forged_bundle = {
        let n = h.node(laptop);
        let intro = n.dir.my_intro_tag().unwrap();
        handshake::build_bundle(&n.id, forged_kp.clone(), intro, "mallory")
            .unwrap()
            .encode()
            .unwrap()
    };

    // The bundle is GENUINE by every pre-existing measure. This assertion is the
    // point of the test: the signature check cannot see the attack.
    let parsed = handshake::parse_and_verify(&forged_bundle)
        .expect("the forged bundle's self-signature is valid — that is the whole hole");
    assert_eq!(
        parsed.identity_pk, mallory_id,
        "`parse_and_verify` attests only that MALLORY signed it"
    );
    assert_eq!(
        handshake::key_package_cred_id(&parsed.key_package).unwrap(),
        ada_id,
        "…while the key package inside speaks for ADA"
    );

    // ── the door ──────────────────────────────────────────────────────────────
    let err = h
        .node(phone)
        .group_add_member(&studio, &forged_bundle)
        .await
        .expect_err(
            "THE HOLE IS OPEN. `add_member_core` grafted a leaf carrying ada's \
             credential on the strength of mallory's signature. Everything that \
             leaf ever authors is attributed to ada by `mls::member_identity`, and \
             the fold grants it ada's AUTHORITY.",
        );
    assert!(
        matches!(err, CoreError::Handshake(_)),
        "the refusal belongs to the handshake layer (the bundle is malformed in \
         meaning, not the group's state): got {err:?}"
    );
    assert!(
        err.to_string().contains(handshake::ERR_KEY_PACKAGE_IDENTITY),
        "the refusal must NAME itself so an operator can tell an impersonation \
         attempt from a relay hiccup — expected {:?}, got {err}",
        handshake::ERR_KEY_PACKAGE_IDENTITY
    );

    // ── and nothing happened ──────────────────────────────────────────────────
    assert_eq!(
        h.epoch(phone, &studio),
        epoch_before,
        "a refused add must not advance the epoch — if it did, the commit was \
         staged and published before the check ran"
    );
    assert_eq!(
        h.roster(phone, &studio),
        roster_before,
        "and the roster is untouched"
    );
}

/// GATE 2 — THE BINDING CHECK RUNS BEFORE THE AUTHORITY GATE, so the public-door
/// path is covered too.
///
/// `admits_by_any_member` lets any member of an OPEN PLACE admit a stranger — that
/// is what stops a public place depending on one person's phone, and it is the
/// widest version of this hole: the party doing the adding has no relationship with
/// the person they are admitting, so "I know this bundle is really them" is
/// available to nobody in the loop. A check hidden inside the owner branch of that
/// gate would never run there.
///
/// bo here is a MEMBER AND NOT THE OWNER of an ordinary forum, so `add_member_core`
/// has two reasons to refuse: the binding, and the owner-only rule. Requiring that
/// the BINDING one is what comes back pins the ordering — the check is ahead of the
/// authority gate, and therefore ahead of every branch of it, including the one
/// that says yes.
///
/// HOW THIS COULD BE FAKED. Assert only that the call errors — closed by naming the
/// expected refusal, since the owner-only refusal would otherwise satisfy it and the
/// test would go quietly green over an unprotected public door.
#[tokio::test]
async fn the_binding_refusal_comes_before_the_owner_gate() {
    let h = Harness::people(&[
        ("ada", &["phone"]),
        ("bo", &["sim"]),
        ("mallory", &["laptop"]),
    ])
    .await;
    let phone = h.device("ada", "phone");
    let sim = h.device("bo", "sim");
    let laptop = h.device("mallory", "laptop");
    let ada_id = h.id(phone);

    // bo is a member of ada's forum, and bo — not ada — is handed the bundle.
    let studio = h.form_named_forum(phone, "the studio");
    h.add_to_forum(phone, sim, &studio).await;

    let (_tmp, forged_kp) = key_package_claiming(&ada_id);
    let forged_bundle = {
        let n = h.node(laptop);
        let intro = n.dir.my_intro_tag().unwrap();
        handshake::build_bundle(&n.id, forged_kp, intro, "mallory")
            .unwrap()
            .encode()
            .unwrap()
    };

    let err = h
        .node(sim)
        .group_add_member(&studio, &forged_bundle)
        .await
        .expect_err("a non-owner must refuse the forgery too");
    assert!(
        err.to_string().contains(handshake::ERR_KEY_PACKAGE_IDENTITY),
        "the refusal must be the BINDING refusal, not the owner-only refusal — a \
         green assertion here that only ever sees \"only the group owner can add \
         members\" would go red the day a public door is opened in this object: \
         got {err}"
    );
}

/// GATE 3 — THE FEATURE THE CHECK IS PROTECTING STILL WORKS.
///
/// Two honest adds through the same door that just refused mallory: a second PERSON
/// (the ordinary case), and ada's own SECOND DEVICE (A1a's entire purpose — same
/// credential, fresh signature key, which is the shape the forgery also has).
///
/// This is the test that fails if the check is drawn too tight. "Refuse a key
/// package whose credential is already in the tree" would close the hole and take
/// multi-device with it; "refuse a key package whose signature key is not the
/// bundle signer's identity key" would refuse EVERY leaf, since an MLS leaf key and
/// an Ed25519 identity key are different keys by construction.
///
/// HOW THIS COULD BE FAKED. Assert only that the calls return `Ok` — closed by
/// asserting the two numbers that must move in opposite directions: THREE leaves in
/// the ratchet tree, TWO people in the roster, and ada's identity key appearing
/// once in the deduplicated projection while holding two of the three leaves.
#[tokio::test]
async fn the_honest_adds_a_second_person_and_a_second_device_still_pass() {
    let mut h = Harness::people(&[("ada", &["phone"]), ("bo", &["sim"])]).await;
    let phone = h.device("ada", "phone");
    let sim = h.device("bo", "sim");
    let ada_id = h.id(phone);
    let bo_id = h.id(sim);

    let studio = h.form_named_forum(phone, "the studio");

    // (1) an ordinary add of a second PERSON
    h.add_to_forum(phone, sim, &studio).await;
    assert_eq!(h.device_leaves(phone, &studio), 2, "ada + bo");

    // (2) A1a's whole purpose: ada's laptop, admitted exactly as a person is
    let laptop = h.add_device("ada", "laptop");
    h.add_to_forum(phone, laptop, &studio).await;

    for u in [phone, sim, laptop] {
        assert_eq!(
            h.device_leaves(u, &studio),
            3,
            "{}: THREE LEAVES — ada's phone, ada's laptop, bo's sim. If this is 2, \
             the binding check refused ada's second device: it is comparing more \
             than the credential, and it has broken the feature it exists to \
             protect.",
            h.device_name(u)
        );
        let mut people = h.roster(u, &studio);
        people.sort();
        let mut expect = vec![ada_id, bo_id];
        expect.sort();
        assert_eq!(
            people, expect,
            "{}: and TWO PEOPLE — the credential is untouched, so ada's two leaves \
             still deduplicate to one member",
            h.device_name(u)
        );
    }

    // the laptop is a real leaf, not a spectator: it authors and the others read it.
    h.obj_post(laptop, &studio, "then 200/100, reprint English").await;
    h.settle().await;
    for u in [phone, sim] {
        let seen: Vec<String> = h.obj_view(u, &studio).into_iter().map(|m| m.text).collect();
        assert!(
            seen.iter().any(|t| t == "then 200/100, reprint English"),
            "{}: ada's laptop wrote and it did not arrive — the leaf was admitted \
             but cannot speak",
            h.device_name(u)
        );
    }
}
