//! door — a request to join, sealed TO the door (ICD 2.6.0 `mls.door`; #76).
//!
//! The one channel a stranger has before admission is a blob on the relay. The Place knock seals
//! it WITH a key printed on the join card, so anyone holding the card reads every knock (the
//! problem the-face-on-the-wire.md §4 named). Here the exterior carries the door's PUBLIC key, a
//! request is sealed to it, and only a device holding `base.setDoor`'s private key — a member's —
//! can open it.
//!
//! - address: `Address::from_seed(SHA-256(addressDomain ‖ door))`, a public seed, so any holder
//!   of the exterior may publish there;
//! - seal: ephemeral X25519 → HKDF-SHA256 → XChaCha20-Poly1305, AAD the door;
//!   wire `E.public(32) ‖ nonce(24) ‖ ciphertext`;
//! - id: SHA-256 of the sealed blob.
//!
//! The plaintext is CBOR, signed by the asker. What the answering device then checks of the
//! bundles (each the asker's own) is `Node`'s.

use chacha20poly1305::aead::{Aead, KeyInit, OsRng, Payload};
use chacha20poly1305::{AeadCore, XChaCha20Poly1305, XNonce};
use ciborium::value::Value;
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use x25519_dalek::{EphemeralSecret, PublicKey, StaticSecret};

use crate::exterior::icd;
use crate::identity::Identity;
use crate::object::MemberId;
use crate::CoreError;

const NONCE_LEN: usize = 24;

/// A fresh door: its private key, for `base.setDoor`, and its public half, for the exterior.
pub fn mint() -> ([u8; 32], [u8; 32]) {
    let secret = StaticSecret::random_from_rng(OsRng);
    (secret.to_bytes(), PublicKey::from(&secret).to_bytes())
}

/// The public half of a door's private key.
pub fn public_of(key: &[u8; 32]) -> [u8; 32] {
    PublicKey::from(&StaticSecret::from(*key)).to_bytes()
}

/// The relay seed of a door's address. Public: any holder of the exterior derives it.
pub fn address_seed(door: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(icd::DOOR_ADDRESS_DOMAIN.as_bytes());
    h.update(door);
    h.finalize().into()
}

/// A request's id: the hash of its sealed blob.
pub fn id_of(blob: &[u8]) -> [u8; 32] {
    Sha256::digest(blob).into()
}

fn key(shared: &[u8; 32], eph: &[u8; 32], door: &[u8; 32]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(Some(icd::DOOR_SEAL_DOMAIN.as_bytes()), shared);
    let mut info = [0u8; 64];
    info[..32].copy_from_slice(eph);
    info[32..].copy_from_slice(door);
    let mut k = [0u8; 32];
    hk.expand(&info, &mut k).expect("32 bytes is a valid HKDF length");
    k
}

/// Seal `plain` to the door.
pub fn seal_to(door: &[u8; 32], plain: &[u8]) -> Vec<u8> {
    let eph = EphemeralSecret::random_from_rng(OsRng);
    let eph_pub = PublicKey::from(&eph).to_bytes();
    let shared = eph.diffie_hellman(&PublicKey::from(*door)).to_bytes();
    let cipher = XChaCha20Poly1305::new(&key(&shared, &eph_pub, door).into());
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ct = cipher.encrypt(&nonce, Payload { msg: plain, aad: door }).expect("XChaCha20 encryption");
    let mut out = Vec::with_capacity(32 + NONCE_LEN + ct.len());
    out.extend_from_slice(&eph_pub);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    out
}

/// Open a blob sealed to the door whose private key is `key`; a wrong key, or a blob that is not
/// one, fails loudly.
pub fn open_with(door_key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>, CoreError> {
    if blob.len() < 32 + NONCE_LEN {
        return Err(CoreError::Seal("a door blob is shorter than its header".into()));
    }
    let secret = StaticSecret::from(*door_key);
    let door = PublicKey::from(&secret).to_bytes();
    let eph: [u8; 32] = blob[..32].try_into().expect("32 bytes");
    let shared = secret.diffie_hellman(&PublicKey::from(eph)).to_bytes();
    let cipher = XChaCha20Poly1305::new(&key(&shared, &eph, &door).into());
    cipher
        .decrypt(XNonce::from_slice(&blob[32..32 + NONCE_LEN]), Payload { msg: &blob[32 + NONCE_LEN..], aad: &door })
        .map_err(|_| CoreError::Seal("a door blob that does not open with this door".into()))
}

/// What a knock says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Knock {
    Request(Request),
    Withdraw(Withdraw),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub object: Vec<u8>,
    pub asker: MemberId,
    /// The asker's own contact bundles, as `handshake` encodes them.
    pub bundles: Vec<String>,
    pub note: String,
    pub at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Withdraw {
    pub object: Vec<u8>,
    pub asker: MemberId,
    pub request: [u8; 32],
    pub at: u64,
}

fn put(h: &mut Sha256, b: &[u8]) {
    h.update((b.len() as u32).to_be_bytes());
    h.update(b);
}

fn head(kind: &str, object: &[u8], asker: &MemberId) -> Sha256 {
    let mut h = Sha256::new();
    h.update(icd::DOOR_SIG_DOMAIN.as_bytes());
    h.update([0]);
    h.update(kind.as_bytes());
    h.update([0]);
    put(&mut h, object);
    h.update(asker);
    h
}

impl Request {
    fn digest(&self) -> [u8; 32] {
        let mut h = head("request", &self.object, &self.asker);
        h.update((self.bundles.len() as u32).to_be_bytes());
        for b in &self.bundles {
            put(&mut h, b.as_bytes());
        }
        put(&mut h, self.note.as_bytes());
        h.update(self.at.to_be_bytes());
        h.finalize().into()
    }

    /// Why this request would be refused before any bundle is read, or None.
    pub fn refusal(&self) -> Option<String> {
        if self.bundles.is_empty() || self.bundles.len() > icd::DOOR_BUNDLES {
            return Some(format!("a request carries 1 to {} bundles, not {}", icd::DOOR_BUNDLES, self.bundles.len()));
        }
        if self.note.len() > icd::DOOR_NOTE_MAX {
            return Some(format!("a request's note is at most {} bytes", icd::DOOR_NOTE_MAX));
        }
        None
    }
}

impl Withdraw {
    fn digest(&self) -> [u8; 32] {
        let mut h = head("withdraw", &self.object, &self.asker);
        h.update(self.request);
        h.update(self.at.to_be_bytes());
        h.finalize().into()
    }
}

fn text(k: &str) -> Value {
    Value::Text(k.into())
}

/// The knock as CBOR, signed by `id`, whose key must be the asker's.
pub fn encode(id: &Identity, knock: &Knock) -> Vec<u8> {
    let (kind, mut fields, digest) = match knock {
        Knock::Request(r) => {
            assert_eq!(r.asker, id.identity_pk(), "a request is signed by its asker");
            let fields = vec![
                (text("bundles"), Value::Array(r.bundles.iter().map(|b| Value::Text(b.clone())).collect())),
                (text("note"), Value::Text(r.note.clone())),
                (text("at"), Value::Integer(r.at.into())),
            ];
            ("request", (r.object.clone(), r.asker, fields), r.digest())
        }
        Knock::Withdraw(w) => {
            assert_eq!(w.asker, id.identity_pk(), "a withdrawal is signed by its asker");
            let fields = vec![(text("request"), Value::Bytes(w.request.to_vec())), (text("at"), Value::Integer(w.at.into()))];
            ("withdraw", (w.object.clone(), w.asker, fields), w.digest())
        }
    };
    let mut map = vec![
        (text("kind"), text(kind)),
        (text("object"), Value::Bytes(fields.0)),
        (text("asker"), Value::Bytes(fields.1.to_vec())),
    ];
    map.append(&mut fields.2);
    map.push((text("sig"), Value::Bytes(id.sign(&digest).to_vec())));
    let mut out = Vec::new();
    ciborium::ser::into_writer(&Value::Map(map), &mut out).expect("CBOR into a Vec");
    out
}

/// A knock out of its plaintext, its signature checked.
pub fn decode(plain: &[u8]) -> Result<Knock, CoreError> {
    let bad = |why: &str| CoreError::Seal(format!("a door knock: {why}"));
    let v: Value = ciborium::de::from_reader(plain).map_err(|_| bad("not CBOR"))?;
    let map = v.into_map().map_err(|_| bad("not a map"))?;
    let get = |k: &str| map.iter().find(|(key, _)| key.as_text() == Some(k)).map(|(_, v)| v.clone());
    let bytes = |k: &str| get(k).and_then(|v| v.into_bytes().ok()).ok_or_else(|| bad(&format!("no `{k}`")));
    let int = |k: &str| {
        get(k).and_then(|v| v.into_integer().ok()).and_then(|i| u64::try_from(i).ok()).ok_or_else(|| bad(&format!("no `{k}`")))
    };
    let kind = get("kind").and_then(|v| v.into_text().ok()).ok_or_else(|| bad("no `kind`"))?;
    let object = bytes("object")?;
    let asker: MemberId = bytes("asker")?.try_into().map_err(|_| bad("an asker is 32 bytes"))?;
    let sig: [u8; 64] = bytes("sig")?.try_into().map_err(|_| bad("a signature is 64 bytes"))?;
    let (knock, digest) = match kind.as_str() {
        "request" => {
            let bundles = get("bundles")
                .and_then(|v| v.into_array().ok())
                .ok_or_else(|| bad("no `bundles`"))?
                .into_iter()
                .map(|b| b.into_text().map_err(|_| bad("a bundle is text")))
                .collect::<Result<Vec<_>, _>>()?;
            let note = get("note").and_then(|v| v.into_text().ok()).ok_or_else(|| bad("no `note`"))?;
            let r = Request { object, asker, bundles, note, at: int("at")? };
            if let Some(why) = r.refusal() {
                return Err(bad(&why));
            }
            let d = r.digest();
            (Knock::Request(r), d)
        }
        "withdraw" => {
            let request: [u8; 32] = bytes("request")?.try_into().map_err(|_| bad("a request id is 32 bytes"))?;
            let w = Withdraw { object, asker, request, at: int("at")? };
            let d = w.digest();
            (Knock::Withdraw(w), d)
        }
        other => return Err(bad(&format!("`{other}` is no kind of knock"))),
    };
    crate::identity::verify_sig(&asker, &digest, &sig).map_err(|_| bad("its signature does not verify"))?;
    Ok(knock)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asker() -> Identity {
        Identity::in_memory([5; 32])
    }

    fn request(id: &Identity) -> Knock {
        Knock::Request(Request { object: vec![1, 2, 3], asker: id.identity_pk(), bundles: vec!["b".into()], note: "hello".into(), at: 7 })
    }

    #[test]
    fn only_the_door_opens_a_knock() {
        let (key, door) = mint();
        assert_eq!(public_of(&key), door);
        let id = asker();
        let blob = seal_to(&door, &encode(&id, &request(&id)));
        assert_eq!(decode(&open_with(&key, &blob).unwrap()).unwrap(), request(&id));
        let (other, _) = mint();
        assert!(open_with(&other, &blob).is_err(), "another door's key cannot open it");
    }

    #[test]
    fn a_forged_or_overfull_knock_is_refused() {
        let id = asker();
        let mut plain = encode(&id, &request(&id));
        let at = plain.len() - 70;
        plain[at] ^= 1;
        assert!(decode(&plain).is_err(), "a byte changed under the signature");
        let full = Knock::Request(Request {
            object: vec![1],
            asker: id.identity_pk(),
            bundles: vec!["b".into(); icd::DOOR_BUNDLES + 1],
            note: String::new(),
            at: 1,
        });
        assert!(decode(&encode(&id, &full)).is_err());
    }

    #[test]
    fn a_withdrawal_names_its_request() {
        let id = asker();
        let w = Knock::Withdraw(Withdraw { object: vec![1], asker: id.identity_pk(), request: [8; 32], at: 9 });
        assert_eq!(decode(&encode(&id, &w)).unwrap(), w);
    }
}
