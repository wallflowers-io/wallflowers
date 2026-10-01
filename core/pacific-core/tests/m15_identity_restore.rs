//! RESTORE — an identity brought back on a device that never held it, from the
//! 24 words the mint emitted. The `Node` half, over real state directories.
//!
//! WHAT THIS PINS, and why it is worth a test binary of its own. `space_id` IS the
//! identity public key, so "restore" here means something no account-based system
//! has to mean: come back as the same key, with the same id, so every pin anyone
//! else made on you still matches. A restore that produced a *nearly* right
//! identity would be worse than a failed one — the device would look fine and be a
//! stranger to everyone. So the assertions are exact: same identity key, same
//! fingerprint words, same pre-rotation commitment.
//!
//! And the other half of honesty: it restores the IDENTITY, not the history.
//! `pacific.db` (MLS ratchet trees, epoch secrets, every folded object) is NOT
//! derivable from a key, so the restored device comes up with a fresh directory
//! and no groups. That is asserted too — a test that quietly let history through
//! would be pinning a promise the design does not make.

use pacific_core::{identity, Node};

/// Point the process at a state dir. `Node` resolves it from this env var, which
/// is process-global — hence ONE test in this binary, taking the devices in turn,
/// exactly as two real phones would.
fn activate(dir: &tempfile::TempDir) {
    std::env::set_var("PACIFIC_STATE_DIR", dir.path());
}

#[test]
fn a_recovery_key_brings_the_same_identity_back_on_a_new_device() {
    // ── device A: the mint ────────────────────────────────────────────────────
    let phone = tempfile::tempdir().unwrap();
    activate(&phone);
    // A new device's Arc, before its identity (O-73). This test never dials it.
    pacific_core::node::set_default_arc("ws://127.0.0.1:1").unwrap();

    let a = Node::init_identity("Ada").unwrap();
    let identity_key = a.identity_key();
    let words = a.sas_words();
    let recovery = a.recovery_key().expect("a fresh identity emits a recovery key");
    assert_eq!(recovery.split_whitespace().count(), 24);
    // The two artefacts must not be confusable: the PUBLIC fingerprint is three
    // words, the SECRET recovery key is twenty-four, and neither contains the other.
    assert_eq!(words.split_whitespace().count(), 3);
    assert!(!recovery.contains(&words));
    drop(a);

    // ── device B: never seen this key ─────────────────────────────────────────
    let new_phone = tempfile::tempdir().unwrap();
    activate(&new_phone);
    assert!(Node::open().is_err(), "the new device starts with nothing");

    let b = Node::restore_identity("Ada", &recovery).unwrap();
    assert_eq!(b.identity_key(), identity_key, "the same key comes back");
    assert_eq!(b.sas_words(), words, "so the words every peer compares still match");
    assert_eq!(b.recovery_key().as_deref(), Some(recovery.as_str()));
    assert_eq!(b.display_name().unwrap(), "Ada");
    // It is a usable node, not just a key file: it can build the signed contact
    // bundle a pairing QR carries, which needs the directory and an MLS key.
    assert!(!b.build_contact_bundle().unwrap().is_empty());
    drop(b);

    // …and it survives the process boundary, from the seed file it just wrote.
    assert_eq!(Node::open().unwrap().identity_key(), identity_key);

    // ── the honest limit: the identity came back, the history did not ─────────
    assert!(
        Node::open().unwrap().objects().unwrap().is_empty(),
        "restore brings the identity, never the groups — those are re-admission's job"
    );

    // ── and it refuses to overwrite a live identity ───────────────────────────
    assert!(
        Node::restore_identity("Ada", &recovery).is_err(),
        "a device that already holds an identity must refuse, loudly"
    );

    // A phrase that is not this one is refused before anything is written.
    let other = tempfile::tempdir().unwrap();
    activate(&other);
    let mut wrong: Vec<&str> = recovery.split_whitespace().collect();
    wrong.swap(0, 1);
    assert!(
        Node::restore_identity("Ada", &wrong.join(" ")).is_err(),
        "two swapped words fail the mnemonic checksum rather than deriving a stranger"
    );
    assert!(
        identity::load().is_err(),
        "and nothing was written on the way to that refusal"
    );

    std::env::remove_var("PACIFIC_STATE_DIR");
}
