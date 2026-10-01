//! seal_intro_vector — emit a REAL Rust-produced intro blob for the adversary harness.
//!
//! WHY THIS FILE EXISTS (owned by harness Track 3 — adversaries):
//! The Python adversary in `harness/adversary/` re-implements the seal derivation
//! (HKDF-SHA256 + XChaCha20-Poly1305) to open intro blobs it scrapes off the relay.
//! A re-implementation is only trustworthy if it opens bytes the ACTUAL Rust core
//! produced. This example builds a genuine `handshake::IntroPayload`, encodes it with
//! the real `IntroPayload::encode()` (canonical CBOR), and seals it with the real
//! `seal::seal()` exactly the way `node.rs:319` / `node.rs:632` do at pairing:
//!
//!     let dest = bundle.intro_tag;                 // the routing address
//!     let sealed = seal::seal(&payload, &dest, &dest)?;   // conn_secret == dest_tag == the tag
//!
//! It prints the tag (hex, as `pacific_wire::tag_hex` = `hex::encode` would publish it),
//! the sealed blob (base64), and the GROUND-TRUTH payload fields as JSON, so the Python
//! side can (a) open the blob keyed only by the tag and (b) assert what it recovered
//! equals what Rust put in. If this file is ever deleted, the adversary's Rust-match
//! proof goes with it — leave it.
//!
//! Run:  cargo run -q -p pacific-core --example seal_intro_vector
//! Optional single arg: a UTF-8 seed string for the intro tag (else a fixed default),
//! so the harness can produce a fresh distinct tag per run without changing this file.

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use pacific_core::handshake::IntroPayload;
use pacific_core::seal;

/// Pack a UTF-8 string into a fixed 32-byte value (truncate/zero-pad). Only used to
/// build reproducible, human-legible test material — NOT how real keys are minted.
fn b32(s: &str) -> [u8; 32] {
    let mut t = [0u8; 32];
    let b = s.as_bytes();
    let n = b.len().min(32);
    t[..n].copy_from_slice(&b[..n]);
    t
}

fn main() {
    // The intro tag is the routing address the relay must hold to deliver the blob.
    // In the protocol it is `bundle.intro_tag`, a stored random 32-byte tag. Here it is
    // seeded from an optional CLI arg (else a fixed default) so the vector is reproducible.
    let seed = std::env::args().nth(1).unwrap_or_else(|| "STOMA/cadiz-yard/intro-tag".into());
    let intro_tag = b32(&seed);

    // A realistic pairing payload: szonja scans axel's code to join STOMA in the Cádiz yard.
    let payload = IntroPayload {
        scanner_pk: b32("szonja-ed25519-identity-pubkey"),
        scanner_name: "szonja".into(),
        // The adversary never needs a VALID welcome — it only reads the plaintext CBOR
        // fields. Opaque bytes stand in for the real MLS Welcome here.
        welcome: b"<mls-welcome bytes: opaque to eve, but she does not need them>".to_vec(),
        why: Some("joining STOMA".into()),
        kind: Some("forum".into()),
        arc: Some("arc.kenjin.cc".into()),
        owner: Some(b32("axel-ed25519-identity-pubkey")),
    };

    let inner = payload.encode().expect("intro encode");
    // EXACTLY node.rs:319 / node.rs:632 — the seal key is a pure function of the tag.
    let sealed = seal::seal(&inner, &intro_tag, &intro_tag).expect("seal");

    // Ground truth + wire material, as JSON on one line.
    let out = serde_json::json!({
        "note": "real Rust-core intro blob; key derivable from the tag alone",
        "tag_hex": hex::encode(intro_tag),
        "sealed_blob_b64": B64.encode(&sealed),
        "payload_cbor_hex": hex::encode(&inner),
        "ground_truth": {
            "scanner_pk": hex::encode(payload.scanner_pk),
            "scanner_name": payload.scanner_name,
            "why": payload.why,
            "kind": payload.kind,
            "arc": payload.arc,
            "owner": payload.owner.map(hex::encode),
            "welcome_len": payload.welcome.len(),
        }
    });
    println!("{}", serde_json::to_string(&out).expect("json"));
}
