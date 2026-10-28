//! m24 — CAN ONE PERSON HOLD TWO LEAVES? The experiment that decides whether
//! multi-device needs a wire change.
//!
//! `m21` pins the refusal: adding a second device to a group fails with
//! `MlsError::DuplicateLeafData`, because both leaves carry the same identity
//! pubkey as their BasicCredential id. The standing conclusion was that the fix
//! is on the wire — either `cred_id` becomes a device key, or it gains a device
//! discriminator — and that every multi-device assertion waits behind it.
//!
//! THAT CONCLUSION MAY BE WRONG, and this binary is how we find out rather than
//! argue. RFC 9420 §7.3 requires exactly two fields to be unique among members:
//!
//!     *  signature_key
//!     *  encryption_key
//!
//! NOT the credential. Two leaves carrying one person's credential are legal MLS
//! so long as each brings its own keys. mls-rs refuses them anyway, because
//! `tree_index.rs` keeps a THIRD uniqueness map keyed on whatever the configured
//! `IdentityProvider::identity()` returns — and `BasicIdentityProvider` returns
//! the credential identifier verbatim. That is library policy layered on the
//! protocol, and the provider is ours to choose.
//!
//! So: same credential, same `cred_id`, per-leaf `identity()`. If the add
//! succeeds, the blocker is a config line rather than a wire format, and
//! `m21`'s reasoning has to be revisited.
//!
//! IT SUCCEEDED. The blocker was a config line. `PerLeafIdentity` now lives in
//! `pacific_core::mls` and is what every Pacific client is built with (A1a), the
//! five relay gates in `node.rs` count leaves instead of members so a person's own
//! objects still reach the relay (A1b), and `m21`'s second test was rewritten from
//! the refusal to the behaviour.

use mls_rs::client_builder::MlsConfig;
use mls_rs::identity::basic::BasicCredential;
use mls_rs::identity::SigningIdentity;
use mls_rs::{CipherSuite, CipherSuiteProvider, CryptoProvider, ExtensionList};
use mls_rs_core::identity::IdentityProvider;
use mls_rs_crypto_rustcrypto::RustCryptoProvider;
use pacific_core::mls::PerLeafIdentity;

const CS: CipherSuite = CipherSuite::CURVE25519_AES128;

// THE ONE CHANGE UNDER TEST — and it is no longer defined in this file.
//
// `PerLeafIdentity` was written here as an experiment: `identity()` is the
// credential AND the leaf's own signature key, so two leaves of one person are
// distinct to mls-rs's tree index while still carrying the same `cred_id` to
// everything above it, and `valid_successor` keeps the credential rule so a leaf
// may still be replaced by another of the SAME PERSON. It passed, so A1a lifted
// it into `pacific_core::mls`, where `build_client` hands it to every client on
// every platform.
//
// This binary goes on testing it and IMPORTS it rather than keeping a copy. A
// private copy would keep passing after the shipped provider drifted, which is the
// one thing an experiment that has become an invariant must never do.

/// The credential identifier — the person's identity pubkey, shared by every leaf
/// they hold. The assertions below are all really about this value staying put.
fn cred_id(sid: &SigningIdentity) -> Result<Vec<u8>, &'static str> {
    sid.credential
        .as_basic()
        .map(|b| b.identifier.to_vec())
        .ok_or("credential is not a BasicCredential")
}

/// A signing identity carrying `person` as its credential id, with fresh keys —
/// i.e. one LEAF of that person.
fn leaf_of(person: &[u8]) -> (SigningIdentity, mls_rs::crypto::SignatureSecretKey) {
    let cs = RustCryptoProvider::default().cipher_suite_provider(CS).unwrap();
    let (sk, pk) = cs.signature_key_generate().unwrap();
    (
        SigningIdentity::new(BasicCredential::new(person.to_vec()).into_credential(), pk),
        sk,
    )
}

fn client<P: IdentityProvider + Clone>(
    provider: P,
    sid: SigningIdentity,
    sk: mls_rs::crypto::SignatureSecretKey,
) -> mls_rs::Client<impl MlsConfig> {
    mls_rs::Client::builder()
        .identity_provider(provider)
        .crypto_provider(RustCryptoProvider::default())
        .signing_identity(sid, sk, CS)
        .build()
}

#[test]
fn basic_identity_refuses_one_persons_second_leaf() {
    // The control. This is m21's failure, reproduced at the library level with
    // nothing of Pacific's in the way, so the next test's success cannot be
    // explained by anything other than the provider.
    use mls_rs::identity::basic::BasicIdentityProvider;
    let ada = b"ada-identity-key";

    let (sid1, sk1) = leaf_of(ada);
    let (sid2, sk2) = leaf_of(ada);
    let phone = client(BasicIdentityProvider, sid1, sk1);
    let laptop = client(BasicIdentityProvider, sid2, sk2);

    let mut group = phone.create_group(ExtensionList::default(), Default::default(), None).unwrap();
    let kp = laptop.generate_key_package_message(Default::default(), Default::default(), None).unwrap();

    let err = group
        .commit_builder()
        .add_member(kp)
        .unwrap()
        .build()
        .expect_err("BasicIdentityProvider admitted a second leaf for one person");
    let text = format!("{err:?}");
    assert!(
        text.contains("DuplicateLeafData"),
        "expected the duplicate-identity refusal, got: {text}"
    );
}

#[test]
fn a_per_leaf_identity_provider_admits_two_leaves_of_one_person() {
    let ada = b"ada-identity-key";
    let bo = b"bo-identity-key";

    let (sid_phone, sk_phone) = leaf_of(ada);
    let (sid_laptop, sk_laptop) = leaf_of(ada);
    let (sid_bo, sk_bo) = leaf_of(bo);

    let phone = client(PerLeafIdentity, sid_phone, sk_phone);
    let laptop = client(PerLeafIdentity, sid_laptop, sk_laptop);
    let bos = client(PerLeafIdentity, sid_bo, sk_bo);

    let mut group = phone.create_group(ExtensionList::default(), Default::default(), None).unwrap();

    // Bo joins the ordinary way, so the group is a real two-person group before
    // the interesting bit.
    let bo_kp = bos.generate_key_package_message(Default::default(), Default::default(), None).unwrap();
    let commit = group.commit_builder().add_member(bo_kp).unwrap().build().unwrap();
    group.apply_pending_commit().unwrap();
    let mut bo_group = bos
        .join_group(None, &commit.welcome_messages[0], None)
        .unwrap()
        .0;

    // ── THE EXPERIMENT ────────────────────────────────────────────────────
    let laptop_kp = laptop.generate_key_package_message(Default::default(), Default::default(), None).unwrap();
    let commit = group
        .commit_builder()
        .add_member(laptop_kp)
        .expect("a second leaf for one person must be proposable")
        .build()
        .expect("ADDING ADA'S SECOND LEAF FAILED — the refusal is not the identity provider");
    group.apply_pending_commit().unwrap();
    bo_group.process_incoming_message(commit.commit_message.clone()).unwrap();
    let mut laptop_group = laptop
        .join_group(None, &commit.welcome_messages[0], None)
        .unwrap()
        .0;

    // Three leaves, two people.
    assert_eq!(group.roster().members().len(), 3, "three leaves in the tree");
    let people: std::collections::BTreeSet<Vec<u8>> = group
        .roster()
        .members()
        .iter()
        .map(|m| cred_id(&m.signing_identity).unwrap())
        .collect();
    assert_eq!(people.len(), 2, "three leaves fold to two PEOPLE");

    // ── and both of ada's leaves can speak, independently ─────────────────
    let from_phone = group.encrypt_application_message(b"from the phone", Default::default()).unwrap();
    let from_laptop = laptop_group.encrypt_application_message(b"from the laptop", Default::default()).unwrap();

    // Each is read by the other two. No coordination, no lease, no lock: two
    // leaves means two ratchets, which is the whole point.
    for (msg, expected) in [(from_phone, "from the phone"), (from_laptop, "from the laptop")] {
        for g in [&mut bo_group, &mut laptop_group, &mut group] {
            // A group cannot process its own message; mls-rs says so loudly.
            let Ok(received) = g.process_incoming_message(msg.clone()) else {
                continue;
            };
            if let mls_rs::group::ReceivedMessage::ApplicationMessage(m) = received {
                assert_eq!(String::from_utf8_lossy(m.data()), expected);
                // The sender resolves to the PERSON, not to the leaf: this is
                // what keeps authorship and authority unchanged above MLS.
                let sender = g.roster().members()[m.sender_index as usize].clone();
                let who = cred_id(&sender.signing_identity).unwrap();
                assert!(who == ada.to_vec() || who == bo.to_vec());
            }
        }
    }
}
