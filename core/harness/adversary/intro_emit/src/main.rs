//! `intro_emit` — the E10 tripwire's emitter. It drives the REAL pairing path.
//!
//! WHY THIS FILE EXISTS, AND WHY IT LOOKS LIKE THIS.
//!
//! E10 claims that a relay can open a pairing intro blob using only the routing tag
//! the blob was published under. The tripwire that guards that claim is the acceptance
//! test for The Build's D-E and A5, which means it has exactly one job: to STOP PASSING
//! on the day the fix lands. It can only do that if the ciphertext it attacks comes out
//! of the CALL SITES THE FIX WILL CHANGE.
//!
//! The previous emitter built an `IntroPayload` by hand and called `seal::seal(&p, &t, &t)`
//! itself. That blob is openable from its tag for as long as somebody writes that line —
//! forever, whatever `node.rs` does. A tripwire attacking it would report "still
//! vulnerable" after the vulnerability was closed. So this emitter never touches
//! `seal::seal` on the intro path. It calls:
//!
//!   --scenario pair      -> `Node::pair_scan_why`     -> node.rs:319 `seal(&payload, &dest, &dest)`
//!   --scenario groupjoin -> `Node::group_add_member`  -> node.rs:632 `seal(&payload, intro_tag, intro_tag)`
//!
//! and lets the node publish to the real relay over the real transport. When D-E seals
//! the intro to something the relay does not hold, or A5 makes `seal()` refuse
//! `tag == secret`, THIS output changes and the tripwire goes red. That is the signal.
//!
//! WHAT IT MUST NEVER PRINT on the intro scenarios: the sealed blob, the seal key, or
//! the encoded payload. Eve fetches the ciphertext off the relay herself, exactly as the
//! operator would. Printed instead: (a) the routing tag — which the relay must hold to
//! route, and which is the whole point — and (b) the GROUND TRUTH of what was sealed, so
//! the suite can grade what Eve recovered against what was really there. Because the
//! plaintext never crosses this boundary, a Python adversary cannot "recover" a field it
//! was handed: the caller passes a fresh per-run nonce in `--why` / `--arc`, and finding
//! that nonce inside the ciphertext is the proof.
//!
//! This crate is NOT a `cargo example` under core/. See Cargo.toml for why.
//!
//! Build & run:
//!   cargo build --manifest-path harness/adversary/intro_emit/Cargo.toml
//!   intro_emit --relay ws://127.0.0.1:8787 --scenario pair \
//!              --why "yard-cadiz-<nonce>" --arc "ws://127.0.0.1:8787/arc-<nonce>"
//!
//! stdout: exactly one line of JSON. Everything else goes to stderr.

use std::path::Path;

use pacific_core::{handshake, paths, router::Routes, seal, Node};

/// `--name value`, or `default` when absent.
fn arg(argv: &[String], name: &str, default: &str) -> String {
    let mut it = argv.iter();
    while let Some(a) = it.next() {
        if a == name {
            return it.next().cloned().unwrap_or_else(|| default.to_string());
        }
    }
    default.to_string()
}

/// `PACIFIC_STATE_DIR` is process-global — the same env-switching discipline
/// `core/pacific-core/tests/common/mod.rs` documents. One device is "active" at a time.
fn activate(dir: &Path) {
    std::env::set_var("PACIFIC_STATE_DIR", dir);
}

fn unhex32(s: &str, what: &str) -> [u8; 32] {
    let v = hex::decode(s).unwrap_or_else(|e| panic!("{what} is not hex: {e}"));
    v.try_into()
        .unwrap_or_else(|_| panic!("{what} must be 32 bytes of hex"))
}

#[tokio::main]
async fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let relay = arg(&argv, "--relay", "");
    let scenario = arg(&argv, "--scenario", "pair");

    match scenario.as_str() {
        "pair" => {
            assert!(!relay.is_empty(), "--relay ws://host:port is required");
            pair(&argv, &relay).await
        }
        "groupjoin" => {
            assert!(!relay.is_empty(), "--relay ws://host:port is required");
            groupjoin(&argv, &relay).await
        }
        "seal_twice" => seal_twice(&argv),
        other => panic!("unknown --scenario {other}"),
    }
}

/// szonja scans axel's code in the yard. The intro blob lands on AXEL's intro tag,
/// published by `pair_scan_why` itself — node.rs's own `sess.publish`, not ours.
async fn pair(argv: &[String], relay: &str) {
    let why = arg(argv, "--why", "yard-cadiz");
    let arc = arg(argv, "--arc", relay);
    let scanner_name = arg(argv, "--scanner-name", "szonja");
    let scannee_name = arg(argv, "--scannee-name", "axel");

    let axel_home = tempfile::tempdir().expect("tempdir");
    let szonja_home = tempfile::tempdir().expect("tempdir");

    // --- axel: mint, pin the relay, pin the Arc, publish his bundle -----------
    activate(axel_home.path());
    let mut axel = Node::init_identity(&scannee_name).expect("axel init");
    axel.set_routes(Routes::parse(relay)).expect("axel routes");
    let axel_pk = axel.id.identity_pk();
    drop(axel);
    std::fs::write(paths::arc_url_path(), &arc).expect("axel arc");
    let axel_bundle = Node::open()
        .expect("axel open")
        .build_contact_bundle()
        .expect("axel bundle");

    // --- szonja: mint, pin the same relay + Arc, then SCAN --------------------
    activate(szonja_home.path());
    let mut szonja = Node::init_identity(&scanner_name).expect("szonja init");
    szonja.set_routes(Routes::parse(relay)).expect("szonja routes");
    let szonja_pk = szonja.id.identity_pk();
    drop(szonja);
    std::fs::write(paths::arc_url_path(), &arc).expect("szonja arc");

    // THE CALL SITE. Everything after this line is observation.
    let peer = Node::open()
        .expect("szonja open")
        .pair_scan_why(&axel_bundle, &why)
        .await
        .expect("pair_scan_why");
    assert_eq!(peer, axel_pk, "pair_scan_why yielded axel's identity");

    // The destination: axel's intro mailbox tag, read back out of his own bundle.
    let dest = handshake::parse_and_verify(&axel_bundle)
        .expect("bundle verifies")
        .intro_tag;

    emit(serde_json::json!({
        "scenario": "pair",
        "call_site": "node.rs:319 seal::seal(&payload, &dest, &dest)",
        "via": "Node::pair_scan_why",
        "relay": relay,
        "intro_tag": hex::encode(dest),
        "expect": {
            "scanner_pk": hex::encode(szonja_pk),
            "scanner_name": scanner_name,
            "why": why,
            "kind": "connection",
            "arc": arc,
            "owner": hex::encode(szonja_pk),
        },
        // Nothing in the payload should carry this; it is here so the suite can
        // prove the marker-absence check is capable of firing at all.
        "absent_marker": null,
    }));
}

/// axel owns a named forum and admits szonja. The intro blob lands on SZONJA's intro
/// tag and carries a Welcome the relay cannot read.
async fn groupjoin(argv: &[String], relay: &str) {
    let arc = arg(argv, "--arc", relay);
    let group_name = arg(argv, "--group-name", "STOMA-zine");
    let owner_name = arg(argv, "--owner-name", "axel");
    let joiner_name = arg(argv, "--joiner-name", "szonja");

    let axel_home = tempfile::tempdir().expect("tempdir");
    let szonja_home = tempfile::tempdir().expect("tempdir");

    activate(szonja_home.path());
    let mut szonja = Node::init_identity(&joiner_name).expect("szonja init");
    szonja.set_routes(Routes::parse(relay)).expect("szonja routes");
    let szonja_pk = szonja.id.identity_pk();
    drop(szonja);
    std::fs::write(paths::arc_url_path(), &arc).expect("szonja arc");
    let szonja_bundle = Node::open()
        .expect("szonja open")
        .build_contact_bundle()
        .expect("szonja bundle");
    let dest = handshake::parse_and_verify(&szonja_bundle)
        .expect("bundle verifies")
        .intro_tag;

    activate(axel_home.path());
    let mut axel = Node::init_identity(&owner_name).expect("axel init");
    axel.set_routes(Routes::parse(relay)).expect("axel routes");
    let axel_pk = axel.id.identity_pk();
    drop(axel);
    std::fs::write(paths::arc_url_path(), &arc).expect("axel arc");

    // The NAME rides the GroupContext extension, so it travels inside the Welcome
    // — which is HPKE-sealed to szonja's KeyPackage init_key. It is the marker the
    // narrowing test looks for: Eve must NOT find it anywhere on the wire.
    let object = Node::open()
        .expect("axel open")
        .object_new("forum", &group_name)
        .expect("object_new");
    // THE SECOND CALL SITE.
    Node::open()
        .expect("axel open")
        .group_add_member(&object, &szonja_bundle)
        .await
        .expect("group_add_member");

    emit(serde_json::json!({
        "scenario": "groupjoin",
        "call_site": "node.rs:632 seal::seal(&payload, intro_tag, intro_tag)",
        "via": "Node::group_add_member",
        "relay": relay,
        "intro_tag": hex::encode(dest),
        "object_id": object,
        "expect": {
            "scanner_pk": hex::encode(axel_pk),
            "scanner_name": owner_name,
            "why": serde_json::Value::Null,
            "kind": "forum",
            "arc": arc,
            "owner": hex::encode(axel_pk),
        },
        "joiner_pk": hex::encode(szonja_pk),
        // The group's display name. Sealed inside the Welcome and nowhere else.
        "absent_marker": group_name,
    }));
}

/// E12 support: seal the SAME plaintext twice, with `tag != secret` so this scenario
/// survives A5, and hand both blobs to the suite. Content addressing keys on bytes,
/// and a random 24-byte nonce per seal means one plaintext has two keys.
///
/// This is the ONLY scenario that calls `seal::seal` directly, and it is deliberately
/// not on the intro path: it measures the seal's nonce behaviour, not node.rs's.
fn seal_twice(argv: &[String]) {
    let tag = unhex32(&arg(argv, "--seal-tag", ""), "--seal-tag");
    let secret = unhex32(&arg(argv, "--seal-secret", ""), "--seal-secret");
    let plaintext = arg(argv, "--plaintext", "the Xalapa kitchen list").into_bytes();
    assert_ne!(tag, secret, "seal_twice must use tag != secret so A5 cannot break it");

    let a = seal::seal(&plaintext, &tag, &secret).expect("seal a");
    let b = seal::seal(&plaintext, &tag, &secret).expect("seal b");

    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    emit(serde_json::json!({
        "scenario": "seal_twice",
        "plaintext_len": plaintext.len(),
        "blob_a_b64": B64.encode(&a),
        "blob_b_b64": B64.encode(&b),
    }));
}

fn emit(v: serde_json::Value) {
    println!("{}", serde_json::to_string(&v).expect("emit"));
}
