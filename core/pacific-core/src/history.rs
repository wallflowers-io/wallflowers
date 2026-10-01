//! history — a late joiner's history, from the object's admitter (O-75; mdr/arc-history.md;
//! pdr/history-review.md). The wire both halves share: the Arc's send and the joiner's ingest.
//!
//! When the Arc admits someone to an object, it seals to that person's intro mailbox, beside the
//! Welcome, the rows of the object it holds: a BUNDLE, `{object, joiner, rows, sender, sig}`.
//!
//! - `rows`: each an `mls.applicationPayload` exactly as a member received it, `{envelope,
//!   author, sig}` (`delta_sig::seal_payload`), unaltered. The owner's sequenced spine first, in
//!   chain order and complete (a partial spine breaks the chain); posts after it, newest first.
//! - `sender`, `sig`: the admitter's identity key, and its Ed25519 signature over [`digest`]. The
//!   intro seal is keyed by the joiner's intro tag alone, so the seal proves no sender; this does.
//! - `joiner`: the person it is for. A bundle names one, so it cannot be replayed to another.
//!
//! What the joiner checks before storing a row is `Node`'s (`ingest_history`), not this module's.

use sha2::{Digest, Sha256};

use crate::identity::Identity;
use crate::object::MemberId;
use crate::CoreError;

/// What an intro-mailbox blob carrying a bundle says it is, where a Welcome's payload names an
/// object kind.
pub const KIND: &str = "history";

/// The signature's domain: a bundle's signature can be read as no other signed message.
pub const DOMAIN: &[u8] = b"wallflowers/history/v1";

/// A bundle's encoded size at most: the ICD's `mls.intro.kinds.history.cap`, read from the ICD at
/// build (build.rs), what one relay blob carries once sealed. `Node::publish_history` also keeps
/// the sealed blob within the relay's limit, the backstop.
pub const CAP: usize = icd::HISTORY_CAP;

/// What one object's card bundles carry at one admission, together, at most (O-75 cards): the
/// ICD's `mls.intro.kinds.history.cardsTotal`, read at build. The spine's bundle is not counted.
pub const CARDS_TOTAL: usize = icd::HISTORY_CARDS_TOTAL;

use crate::icd_consts as icd;

/// One row: a Delta as its member received it, with its author and the author's A-10 signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub envelope: Vec<u8>,
    pub author: MemberId,
    pub sig: [u8; 64],
}

impl Row {
    /// The row as `mls.applicationPayload` carries it.
    pub fn payload(&self) -> Vec<u8> {
        crate::delta_sig::seal_payload(&self.envelope, &self.author, &self.sig)
    }
}

/// One object's history, for one joiner, signed by the admitter that sends it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bundle {
    pub object: Vec<u8>,
    pub joiner: MemberId,
    pub rows: Vec<Row>,
    pub sender: MemberId,
    pub sig: [u8; 64],
}

/// What the sender signs: SHA-256 over the domain, then each field length-prefixed where its
/// length varies, so no two bundles share the bytes.
pub fn digest(object: &[u8], joiner: &MemberId, sender: &MemberId, rows: &[Row]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(DOMAIN);
    h.update([0u8]);
    h.update((object.len() as u32).to_be_bytes());
    h.update(object);
    h.update(joiner);
    h.update(sender);
    h.update((rows.len() as u32).to_be_bytes());
    for r in rows {
        let p = r.payload();
        h.update((p.len() as u32).to_be_bytes());
        h.update(&p);
    }
    h.finalize().into()
}

/// A bundle of `rows` of `object` for `joiner`, signed by `id`, its sender.
pub fn sign(id: &Identity, object: &[u8], joiner: &MemberId, rows: Vec<Row>) -> Bundle {
    let sender = id.identity_pk();
    let sig = id.sign(&digest(object, joiner, &sender, &rows));
    Bundle { object: object.to_vec(), joiner: *joiner, rows, sender, sig }
}

/// The sender's signature over the bundle, checked.
pub fn verify(b: &Bundle) -> Result<(), CoreError> {
    crate::identity::verify_sig(&b.sender, &digest(&b.object, &b.joiner, &b.sender, &b.rows), &b.sig)
        .map_err(|_| CoreError::Handshake("history bundle: its sender's signature does not verify".into()))
}

/// The bundle as the intro blob's plaintext: a CBOR map, `kind` "history", each row its payload.
pub fn encode(b: &Bundle) -> Vec<u8> {
    use ciborium::value::Value;
    let map = Value::Map(vec![
        (Value::Text("kind".into()), Value::Text(KIND.into())),
        (Value::Text("object".into()), Value::Bytes(b.object.clone())),
        (Value::Text("joiner".into()), Value::Bytes(b.joiner.to_vec())),
        (Value::Text("rows".into()), Value::Array(b.rows.iter().map(|r| Value::Bytes(r.payload())).collect())),
        (Value::Text("sender".into()), Value::Bytes(b.sender.to_vec())),
        (Value::Text("sig".into()), Value::Bytes(b.sig.to_vec())),
    ]);
    let mut out = Vec::new();
    ciborium::ser::into_writer(&map, &mut out).expect("CBOR into a Vec");
    out
}

/// A bundle, if the plaintext is one: `Ok(None)` for anything whose `kind` is not "history" (a
/// Welcome's payload). Over [`CAP`], or malformed, is refused by name.
pub fn decode(plain: &[u8]) -> Result<Option<Bundle>, CoreError> {
    use ciborium::value::Value;
    let bad = |why: &str| CoreError::Handshake(format!("history bundle: {why}"));
    let Ok(Value::Map(entries)) = ciborium::de::from_reader::<Value, _>(plain) else { return Ok(None) };
    let field = |k: &str| entries.iter().find(|(key, _)| key.as_text() == Some(k)).map(|(_, v)| v);
    if field("kind").and_then(Value::as_text) != Some(KIND) {
        return Ok(None);
    }
    if plain.len() > CAP {
        return Err(bad(&format!("{} bytes, over the {CAP} a bundle holds", plain.len())));
    }
    let bytes = |k: &str| field(k).and_then(Value::as_bytes).cloned().ok_or_else(|| bad(&format!("no {k}")));
    let key = |k: &str| -> Result<MemberId, CoreError> { bytes(k)?.try_into().map_err(|_| bad(&format!("{k} is not a key"))) };
    let rows = field("rows")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("no rows"))?
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let p = v.as_bytes().ok_or_else(|| bad(&format!("row {i} is not a payload")))?;
            let (envelope, author, sig) = crate::delta_sig::open_payload(p).map_err(|e| bad(&format!("row {i}: {e}")))?;
            Ok(Row { envelope, author, sig })
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    Ok(Some(Bundle {
        object: bytes("object")?,
        joiner: key("joiner")?,
        rows,
        sender: key("sender")?,
        sig: bytes("sig")?.try_into().map_err(|_| bad("sig is not 64 bytes"))?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(seed: u8) -> Identity {
        Identity::in_memory([seed; 32])
    }

    fn row(author: &Identity, object: &[u8], envelope: &[u8]) -> Row {
        let delta_id: [u8; 32] = Sha256::digest(envelope).into();
        Row { envelope: envelope.to_vec(), author: author.identity_pk(), sig: crate::delta_sig::sign_delta(author, object, &delta_id) }
    }

    /// The cap is the ICD's: row 5's `mls.intro.kinds.history.cap`, as build.rs read it, and the
    /// guard that the read works.
    #[test]
    fn the_cap_is_the_icds() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json");
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(doc["mls"]["intro"]["kinds"]["history"]["cap"].as_u64(), Some(CAP as u64));
        assert_eq!(doc["mls"]["intro"]["kinds"]["history"]["cbor"]["kind"], serde_json::json!("the text \"history\""), "KIND, as the ICD names it");
    }

    /// The cards' total is the ICD's: row 5's cards variant, `mls.intro.kinds.history.cardsTotal`,
    /// as build.rs read it.
    #[test]
    fn the_cards_total_is_the_icds() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json");
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(doc["mls"]["intro"]["kinds"]["history"]["cardsTotal"].as_u64(), Some(CARDS_TOTAL as u64));
    }

    #[test]
    fn a_bundle_round_trips_and_its_signature_is_its_senders() {
        let (arc, ada, vis) = (id(1), id(2), id(3));
        let object = b"the site".to_vec();
        let b = sign(&arc, &object, &vis.identity_pk(), vec![row(&ada, &object, b"one"), row(&ada, &object, b"two")]);
        let back = decode(&encode(&b)).unwrap().expect("a bundle");
        assert_eq!(back, b);
        verify(&back).expect("the sender's signature");
    }

    #[test]
    fn a_bundle_altered_after_signing_does_not_verify() {
        let (arc, ada, vis, eve) = (id(1), id(2), id(3), id(4));
        let object = b"the site".to_vec();
        let b = sign(&arc, &object, &vis.identity_pk(), vec![row(&ada, &object, b"one")]);
        for altered in [
            Bundle { joiner: eve.identity_pk(), ..b.clone() },
            Bundle { object: b"another".to_vec(), ..b.clone() },
            Bundle { rows: vec![], ..b.clone() },
            Bundle { sender: eve.identity_pk(), ..b.clone() },
        ] {
            assert!(verify(&altered).is_err(), "{altered:?}");
        }
    }

    #[test]
    fn a_welcome_is_not_a_bundle_and_an_oversized_bundle_is_refused() {
        let welcome = crate::handshake::IntroPayload {
            scanner_pk: [1; 32],
            scanner_name: "arc".into(),
            welcome: vec![1, 2, 3],
            why: None,
            kind: Some("group".into()),
            arc: None,
            owner: None,
            gen_watermark: None,
        }
        .encode()
        .unwrap();
        assert_eq!(decode(&welcome).unwrap(), None);
        let (arc, ada, vis) = (id(1), id(2), id(3));
        let object = b"the site".to_vec();
        let big = vec![row(&ada, &object, &vec![7u8; CAP])];
        let why = decode(&encode(&sign(&arc, &object, &vis.identity_pk(), big))).unwrap_err().to_string();
        assert!(why.contains(&format!("over the {CAP}")), "{why}");
    }
}
