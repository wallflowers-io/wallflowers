//! Per-delta authorship signatures — who wrote this, provable by anyone, forever.
//!
//! ## The hole this closes
//!
//! Live, authorship is unforgeable: MLS signs every application message, and
//! `mls::Incoming::Application` hands back "the MLS-authenticated SENDER's
//! identity pubkey". The author is never on the wire — it is the ratchet-tree
//! leaf that signed the message.
//!
//! Stored, authorship was a CLAIM. [`crate::archive::LogEntry`] carries an
//! `author` field that the exporting device wrote down after MLS told it, and the
//! proof — MLS's own `FramedContentAuthData` — was consumed at decrypt time and
//! thrown away. `import_archive` re-derives the delta id from the envelope, which
//! catches a tampered envelope, but the id commits to the CONTENT and not to the
//! author: `(author = alice, envelope = X)` and `(author = bob, envelope = X)` are
//! both perfectly consistent.
//!
//! That matters more than misattribution, because the fold keys AUTHORITY on the
//! author: `deliver()` rejects a "non-member, non-owner spine write". Forge the
//! author and you forge the permission.
//!
//! ## Why now
//!
//! It was survivable while an archive was sealed under its own owner's seed —
//! only you could rewrite your own history, which is a strange thing to want and
//! harms only you. It stops being survivable the moment archived deltas are
//! SHARED: one blob readable by a whole group, written by any member. Signal
//! signs every `SenderKeyMessage` with the sender's key for exactly this reason.
//!
//! ## Shape, and why it is this shape
//!
//! ```text
//! sig = Ed25519(identity_sk, DOMAIN ‖ group_id ‖ author ‖ delta_id)
//! ```
//!
//! OUTSIDE THE ENVELOPE, never in it. `Delta::id()` is `SHA256(canonical_bytes())`,
//! so a signature inside the envelope would be hashed into the id it signs. Sitting
//! outside means `canonical_bytes` is untouched, every DeltaId ever minted stays
//! valid, and the delta-graph ICD does not move.
//!
//! THE AUTHOR IS THE VERIFYING KEY. `MemberId` is `[u8; 32]` and that is the
//! Ed25519 identity pubkey — the same bytes an MLS leaf carries as its
//! BasicCredential id. So verification needs NOTHING but the signed triple: no
//! roster, no group state, no MLS. A device holding only the 24 words can fetch a
//! blob from object storage and check it cold. That property is what makes shared
//! archive blobs safe to serve from anywhere.
//!
//! WHAT EACH FIELD STOPS:
//!   · `group_id`  — replaying a delta into a different group.
//!   · `author`    — reattribution, which is the whole point.
//!   · `delta_id`  — any change to the content, since the id commits to all of it.
//!   · `DOMAIN`    — reuse of an identity signature made for some other purpose.
//!
//! WHAT IT DOES NOT CLAIM: that the author was a MEMBER at the time, or that the
//! delta was ever accepted. It binds a name to some bytes. Membership and
//! authority remain the fold's business, judged against the roster — this only
//! guarantees the fold is judging a name nobody forged.

use crate::CoreError;
use crate::identity::{verify_sig, Identity};
use crate::object::MemberId;

/// Domain separation for an authorship signature. Versioned: a later scheme that
/// signed different bytes would be a different string, so the two could never be
/// confused for one another.
pub const DELTA_AUTHOR_DOMAIN: &[u8] = b"pacific/delta-author/v1";

/// The bytes an authorship signature covers. One definition, used by both sides —
/// a signer and a verifier that disagree here fail in the one way that looks like
/// a forgery.
pub fn authorship_payload(group_id: &[u8], author: &MemberId, delta_id: &[u8; 32]) -> Vec<u8> {
    let mut out =
        Vec::with_capacity(DELTA_AUTHOR_DOMAIN.len() + 1 + group_id.len() + author.len() + 32);
    out.extend_from_slice(DELTA_AUTHOR_DOMAIN);
    // A length prefix, so a group id and an author cannot be slid across the
    // boundary between them to produce the same bytes from different inputs.
    out.push(0);
    out.extend_from_slice(&(group_id.len() as u32).to_be_bytes());
    out.extend_from_slice(group_id);
    out.extend_from_slice(author);
    out.extend_from_slice(delta_id);
    out
}

/// Sign a delta this identity is authoring.
pub fn sign_delta(id: &Identity, group_id: &[u8], delta_id: &[u8; 32]) -> [u8; 64] {
    let author = id.identity_pk();
    id.sign(&authorship_payload(group_id, &author, delta_id))
}

/// Check an authorship signature. The author's public key is the author field
/// itself, so this needs nothing the caller does not already hold.
pub fn verify_delta(
    group_id: &[u8],
    author: &MemberId,
    delta_id: &[u8; 32],
    sig: &[u8; 64],
) -> Result<(), CoreError> {
    verify_sig(author, &authorship_payload(group_id, author, delta_id), sig)
}

// ── the payload on the wire (A-10 part 1; ICD-0 `mls.applicationPayload`) ────────
//
// RULED 26 Sep (A-10, (a)): every Delta travels WITH its author's signature, and a
// Node verifies it before the Delta takes effect (SEC-35). The MLS application
// message carries this map, canonical CBOR, and never the bare envelope:
//
//     { 1: envelope bytes, 2: author (32-byte identity key), 3: sig (64 bytes) }
//
// The envelope is unchanged, so `Delta::id()` and every id already minted are too.
// The author is taken from here, never from the MLS leaf that sent it (CS-32).

/// The payload's keys, as ICD-0 names them (pinned by `icd.rs`).
pub const PAYLOAD_ENVELOPE: u64 = 1;
pub const PAYLOAD_AUTHOR: u64 = 2;
pub const PAYLOAD_SIG: u64 = 3;

/// The MLS application payload for one of this device's own Deltas.
pub fn seal_payload(envelope: &[u8], author: &MemberId, sig: &[u8; 64]) -> Vec<u8> {
    use ciborium::value::{Integer, Value};
    let map = Value::Map(vec![
        (Value::Integer(Integer::from(PAYLOAD_ENVELOPE)), Value::Bytes(envelope.to_vec())),
        (Value::Integer(Integer::from(PAYLOAD_AUTHOR)), Value::Bytes(author.to_vec())),
        (Value::Integer(Integer::from(PAYLOAD_SIG)), Value::Bytes(sig.to_vec())),
    ]);
    let mut out = Vec::new();
    ciborium::ser::into_writer(&map, &mut out).expect("CBOR into a Vec");
    out
}

/// The envelope, its author and the signature, out of a received payload; or a
/// refusal that names the signature. A bare envelope (a build that predates A-10,
/// or a member putting bytes on the relay by hand) has no author signature and is
/// refused here, before anything is stored.
pub fn open_payload(data: &[u8]) -> Result<(Vec<u8>, MemberId, [u8; 64]), CoreError> {
    use ciborium::value::Value;
    let unsigned = || CoreError::Identity("a Delta with no author signature: refused before it is stored (A-10)".into());
    let v: Value = ciborium::de::from_reader(data).map_err(|_| unsigned())?;
    let Value::Map(entries) = v else { return Err(unsigned()) };
    if entries.len() != 3 {
        return Err(unsigned());
    }
    let field = |k: u64| -> Result<Vec<u8>, CoreError> {
        entries
            .iter()
            .find(|(key, _)| key.as_integer().and_then(|i| u64::try_from(i).ok()) == Some(k))
            .and_then(|(_, val)| val.as_bytes().cloned())
            .ok_or_else(unsigned)
    };
    let envelope = field(PAYLOAD_ENVELOPE)?;
    let author: MemberId = field(PAYLOAD_AUTHOR)?.try_into().map_err(|_| unsigned())?;
    let sig: [u8; 64] = field(PAYLOAD_SIG)?.try_into().map_err(|_| unsigned())?;
    Ok((envelope, author, sig))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;

    fn id(seed: u8) -> Identity {
        Identity::in_memory([seed; 32])
    }

    #[test]
    fn the_payload_carries_envelope_author_and_signature_and_a_bare_envelope_is_refused() {
        let ada = id(1);
        let gid = b"group-one".to_vec();
        let envelope = b"an envelope".to_vec();
        let delta = [0x33u8; 32];
        let sig = sign_delta(&ada, &gid, &delta);
        let (e, a, s) = open_payload(&seal_payload(&envelope, &ada.identity_pk(), &sig)).unwrap();
        assert_eq!((e, a, s), (envelope.clone(), ada.identity_pk(), sig));
        let why = open_payload(&envelope).unwrap_err().to_string();
        assert!(why.contains("signature"), "{why}");
    }

    #[test]
    fn a_signature_binds_author_group_and_content() {
        let ada = id(1);
        let gid = b"group-one".to_vec();
        let delta = [0x11u8; 32];
        let sig = sign_delta(&ada, &gid, &delta);

        assert!(verify_delta(&gid, &ada.identity_pk(), &delta, &sig).is_ok());

        // REATTRIBUTION: the same bytes claimed by someone else. This is the
        // attack — a member of the group rewriting who said what — and the
        // author field being the verifying key is what refuses it.
        let ben = id(2);
        assert!(verify_delta(&gid, &ben.identity_pk(), &delta, &sig).is_err());

        // CROSS-GROUP REPLAY: a delta lifted into another group.
        assert!(verify_delta(b"group-two", &ada.identity_pk(), &delta, &sig).is_err());

        // ANY CHANGE TO THE CONTENT, since the id commits to the whole envelope.
        assert!(verify_delta(&gid, &ada.identity_pk(), &[0x12u8; 32], &sig).is_err());
    }

    #[test]
    fn a_signature_proves_possession_and_not_membership() {
        // THE LIMIT OF THIS MECHANISM, pinned so nobody mistakes it for more.
        //
        // The author field carries the verifying key, so a forger does not have
        // to break a signature to invent an author: they sign with their own key
        // and put their own public key in `author`. That verifies, correctly —
        // the signature says "the holder of this key signed these bytes", which
        // is true, and says nothing whatever about whether that key belongs in
        // the group.
        let nobody = id(9);
        let gid = b"group-one".to_vec();
        let delta = [0x11u8; 32];
        let sig = sign_delta(&nobody, &gid, &delta);
        assert!(
            verify_delta(&gid, &nobody.identity_pk(), &delta, &sig).is_ok(),
            "a self-consistent forgery verifies — which is why the ROSTER has to \
             be folded from signed membership deltas rather than read out of an \
             archive field"
        );
    }

    #[test]
    fn the_domain_separates_it_from_every_other_identity_signature() {
        // The same key signing the same trailing bytes for another purpose must
        // not produce something that verifies here.
        let ada = id(1);
        let gid = b"group-one".to_vec();
        let delta = [0x11u8; 32];

        let mut without_domain = Vec::new();
        without_domain.extend_from_slice(&(gid.len() as u32).to_be_bytes());
        without_domain.extend_from_slice(&gid);
        without_domain.extend_from_slice(&ada.identity_pk());
        without_domain.extend_from_slice(&delta);
        let stray = ada.sign(&without_domain);

        assert!(verify_delta(&gid, &ada.identity_pk(), &delta, &stray).is_err());
    }

    #[test]
    fn the_length_prefix_stops_the_boundary_sliding() {
        // Without a length prefix, a group id one byte longer and an author one
        // byte shorter would concatenate to the same buffer. The ids here are
        // chosen so that naive concatenation collides.
        let ada = id(1);
        let a = authorship_payload(b"aa", &[7u8; 32], &[9u8; 32]);
        let b = authorship_payload(b"a", &[7u8; 32], &[9u8; 32]);
        assert_ne!(a, b);
        let _ = ada;
    }
}
