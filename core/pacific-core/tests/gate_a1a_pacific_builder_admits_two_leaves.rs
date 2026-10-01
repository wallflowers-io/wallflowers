//! A1a, THROUGH PACIFIC'S OWN CLIENT BUILDER — the provider has to be WIRED IN,
//! not merely written.
//!
//! `docs/the-build.html` §02 A1a:
//!
//! > Lift `PerLeafIdentity` out of `m24_leaf_pool.rs:46-101` into `mls.rs` and hand
//! > it to the client builder in place of `BasicIdentityProvider`. `identity()` is
//! > credential ‖ signature_key; `valid_successor` compares credentials only,
//! > unchanged from Basic. `cred_id` stays the person's identity pubkey, so the
//! > fold, authorship, authority, `space_id` and the archive's identity edges are
//! > all untouched.
//!
//! `m24_leaf_pool` proves the type works at the library level, with a client it
//! builds itself. That is the experiment. THIS is the acceptance test, and the
//! difference is the whole point: every client here comes out of
//! `mls::build_client_sqlite`, the function `node.rs` calls. A `PerLeafIdentity`
//! that exists in `mls.rs` and is not what the builder hands out would pass m24
//! and fail here, and that is exactly the failure mode a lifted type has.
//!
//! No harness, no relay, no directory — the library path, so a failure here cannot
//! be blamed on anything above MLS.
//!
//! TODAY: fails — `mls::add_member` returns `MlsError::DuplicateLeafData`, because
//! `BasicIdentityProvider::identity()` returns the credential verbatim and both of
//! ada's leaves carry hers.
//! AFTER: passes.

use pacific_core::mls::{self, Incoming};
use pacific_core::mls_store::migrate;

/// One LEAF: its own SQLite stores, its own MLS signing key, and a credential id
/// that is the PERSON's identity pubkey — shared with every other leaf that person
/// holds. Two `Leaf`s built with the same `id` are one account on two devices.
struct Leaf {
    _tmp: tempfile::TempDir,
    db: std::path::PathBuf,
    sig_sk: Vec<u8>,
    sig_pk: Vec<u8>,
    id: [u8; 32],
}

impl Leaf {
    fn of(id: [u8; 32]) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("pacific.db");
        migrate(&db).unwrap();
        let crypto = mls::crypto();
        let (sk, pk) = mls::generate_signing_key(&crypto).unwrap();
        Leaf {
            _tmp: tmp,
            db,
            sig_sk: sk.as_bytes().to_vec(),
            sig_pk: pk.as_bytes().to_vec(),
            id,
        }
    }

    fn client(&self) -> mls::Client {
        let sid = mls::signing_identity(&self.id, &self.sig_pk);
        let sk = mls::SecretKey::new(self.sig_sk.clone());
        mls::build_client_sqlite(&self.db, sid, sk).unwrap()
    }
}

/// THE GATE. A client built by `mls::build_client_sqlite` admits a second leaf of
/// the SAME PERSON, and the credential does not change to buy it.
///
/// HOW THIS COULD BE FAKED. (a) Give the laptop its own `cred_id` (a device key,
/// or a discriminator appended to the identity pubkey). The add would succeed and
/// everything above MLS would break silently: the fold keys membership on that
/// value, authorship resolves through it, `space_id` is derived from it. Closed by
/// asserting `roster_identities` returns ada's identity key TWICE, byte-for-byte.
/// (b) Relax `valid_successor` to admit anyone — closed by admitting a THIRD
/// person, bo, and asserting the roster is exactly {ada, ada, bo}, so nothing was
/// dropped or merged. (c) Keep the provider for the experiment and leave
/// `build_client` on `BasicIdentityProvider` — closed because every client here
/// comes from the shipped builder.
#[test]
fn pacific_client_builder_admits_a_second_leaf_of_one_person() {
    let ada = [0xAD; 32];
    let bo = [0xB0; 32];

    let phone = Leaf::of(ada);
    let laptop = Leaf::of(ada);
    let sim = Leaf::of(bo);

    assert_ne!(
        phone.sig_pk, laptop.sig_pk,
        "the premise: two leaves of one person carry DIFFERENT signature keys. \
         RFC 9420 §7.3 requires exactly signature_key and encryption_key to be \
         unique among members — never the credential."
    );

    let mut group = mls::create_group(&phone.client()).unwrap();

    // ── the add that used to be refused ───────────────────────────────────────
    let laptop_kp = mls::make_key_package_bytes(&laptop.client()).unwrap();
    let (_commit, welcome) = mls::add_member(&mut group, &laptop_kp).expect(
        "A1a FAILED. `mls::build_client_sqlite` refused a second leaf for one \
         person. If this is `DuplicateLeafData`, the client builder is still on \
         `BasicIdentityProvider` — mls-rs's tree_index keeps a uniqueness map on \
         whatever `IdentityProvider::identity()` returns, and Basic returns the \
         credential verbatim. `PerLeafIdentity` returns credential ‖ signature_key.",
    );
    let laptop_group = mls::join_group(&laptop.client(), &welcome).unwrap();

    // ── and a real second PERSON still joins, unchanged ───────────────────────
    let sim_kp = mls::make_key_package_bytes(&sim.client()).unwrap();
    let (c2, w2) = mls::add_member(&mut group, &sim_kp).unwrap();
    let sim_group = mls::join_group(&sim.client(), &w2).unwrap();
    // COORDINATOR FIX (14 Sep): the laptop joined at the epoch BEFORE bo was
    // added, and an MLS member only advances by processing commits. Without
    // delivering bo's add, the laptop is legitimately one epoch behind and sees
    // two leaves. The original assertion read that as a product failure; it is
    // the test's own omission. The harness-based gates never hit this because
    // `settle()` delivers for them.
    let mut laptop_group = laptop_group;
    mls::decrypt_message(&mut laptop_group, &c2).unwrap();

    // ── cred_id is UNTOUCHED, which is what keeps everything above MLS working ─
    let mut roster = mls::roster_identities(&group).unwrap();
    roster.sort();
    let mut expect = vec![ada, ada, bo];
    expect.sort();
    assert_eq!(
        roster, expect,
        "the roster is {{ada, ada, bo}} — three leaves, and ada's identity key \
         appears TWICE, byte-identical. A per-device credential would make this \
         three DIFFERENT keys, and the fold, authorship, authority, space_id and \
         the archive's identity edges would all read ada as two people."
    );
    assert_eq!(
        mls::roster_identities(&laptop_group).unwrap().len(),
        3,
        "the laptop sees the same three leaves"
    );
    assert_eq!(mls::roster_identities(&sim_group).unwrap().len(), 3);
}

/// TWO LEAVES MEANS TWO RATCHETS — ada's laptop encrypts, ada's phone decrypts,
/// and the sender the MLS layer reports is ADA, not a device.
///
/// This is what makes the leaf a real second device rather than a second entry in
/// a tree. It is also the assertion that would catch a "fix" that admitted the
/// leaf but left it unable to speak.
///
/// TODAY: fails — the add above never succeeds, so there is no second ratchet.
/// AFTER: passes.
///
/// HOW THIS COULD BE FAKED. Share one ratchet between the devices (restore MLS
/// group state onto the second device instead of giving it a leaf) — closed
/// because each `Leaf` here has its own SQLite store and its own signing key, and
/// the laptop's group arrives through a Welcome, not a snapshot.
#[test]
fn both_leaves_of_one_person_encrypt_and_the_sender_is_the_person() {
    let ada = [0xAD; 32];
    let bo = [0xB0; 32];
    let phone = Leaf::of(ada);
    let laptop = Leaf::of(ada);
    let sim = Leaf::of(bo);

    let mut g_phone = mls::create_group(&phone.client()).unwrap();
    let kp = mls::make_key_package_bytes(&laptop.client()).unwrap();
    let (_c, w) = mls::add_member(&mut g_phone, &kp).expect("A1a: the second leaf");
    let mut g_laptop = mls::join_group(&laptop.client(), &w).unwrap();

    let kp2 = mls::make_key_package_bytes(&sim.client()).unwrap();
    let (commit2, w2) = mls::add_member(&mut g_phone, &kp2).unwrap();
    let mut g_sim = mls::join_group(&sim.client(), &w2).unwrap();
    // the laptop processes the add-bo commit off the epoch it is still on
    match mls::decrypt_message(&mut g_laptop, &commit2).unwrap() {
        Incoming::Commit { .. } => {}
        _ => panic!(
            "the laptop should have processed the add-bo commit as a HANDSHAKE — \
             if it came back SkippedOwn, mls-rs is treating ada's two leaves as \
             one sender"
        ),
    }
    assert_eq!(
        g_phone.current_epoch(),
        g_laptop.current_epoch(),
        "one group, one epoch — two leaves of one person must not fork it"
    );

    // ada's LAPTOP speaks.
    let msg = mls::encrypt_delta(&mut g_laptop, b"then 200/100, reprint English").unwrap();

    for (name, g) in [("ada's phone", &mut g_phone), ("bo's sim", &mut g_sim)] {
        match mls::decrypt_message(g, &msg).unwrap() {
            Incoming::Application { sender, data } => {
                assert_eq!(
                    sender, ada,
                    "{name}: the MLS-authenticated sender is ADA — the leaf signed \
                     it, but the credential it carries is the person's, and that \
                     is the value the commutative fold keys membership on"
                );
                assert_eq!(data, b"then 200/100, reprint English");
            }
            _ => panic!(
                "{name} did not read ada's laptop as an APPLICATION message. \
                 SkippedOwn here would mean mls-rs matched the leaf to itself — \
                 two leaves of one person must be two senders with two ratchets."
            ),
        }
    }
}
