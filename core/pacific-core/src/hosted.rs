//! hosted — THE HOSTED EXTERIOR DOCUMENT (`media.hosted`, Ralph 12 Nov 2026).
//!
//! An object's outside lives at one key in the relay's object storage, overwritten in place by
//! its owner's devices, so a change reaches every reader on their next fetch and nothing is
//! carried across the network again. The carried exterior (`base.setExterior`) is the pointer:
//! the key and its content key.
//!
//! - The key is an address (`pacific_wire::address`) whose seed comes from the owner identity's
//!   seed, so every owner device and a restore write to the same place, and only they can.
//! - The document is the fields (`exterior::fields_of`) or a tombstone, with a version, signed
//!   by the owner identity and sealed under the content key with the key as AAD.
//! - A reader checks the object, the kind, the owner's signature and the fields against the
//!   ICD's list; the version against the last it opened is the caller's (it holds the memory).

use hkdf::Hkdf;
use pacific_wire::address::Address;
use sha2::Sha256;

use crate::exterior::{self, icd::HOSTED_MAX_BYTES, icd::HOSTED_SALT, Hosted};
use crate::object::ObjectKind;
use crate::CoreError;

/// What the owner identity signs a document under (`media.hosted.document.plaintext`).
pub const DOC_SIG_DOMAIN: &[u8] = b"wallflowers-hosted-exterior:v1\n";

/// A hosted object's address and content key, as an owner device derives them.
pub struct Keys {
    pub address: Address,
    pub hosted: Hosted,
}

/// `media.hosted.seed` and `contentKey`: from the owner identity's seed and the object's id.
pub fn keys(seed: &[u8; 32], object: &str) -> Keys {
    let hk = Hkdf::<Sha256>::new(Some(HOSTED_SALT.as_bytes()), seed);
    let expand = |label: &[u8]| {
        let mut out = zeroize::Zeroizing::new([0u8; 32]);
        hk.expand(&[label, object.as_bytes()].concat(), out.as_mut()).expect("32-byte OKM");
        out
    };
    let address = Address::from_seed(&expand(b"key"));
    let secret = *expand(b"content");
    Keys { hosted: Hosted { key: address.tag_hex(), secret }, address }
}

/// What a document says: the outside, or that there is none.
#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Fields(serde_json::Map<String, serde_json::Value>),
    Tombstone,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Doc {
    pub object: String,
    pub kind: ObjectKind,
    /// The writer's clock, unix ms, never below the last it wrote or read.
    pub version: i64,
    pub body: Body,
}

fn bad(why: &str) -> CoreError {
    CoreError::Seal(format!("hosted exterior: {why}"))
}

/// The document, signed by `owner` and sealed under the pointer: the bytes an owner device
/// uploads. Refused past `media.hosted.maxBytes`.
pub fn seal(owner: &crate::identity::Identity, hosted: &Hosted, doc: &Doc) -> Result<Vec<u8>, CoreError> {
    let mut d = serde_json::json!({ "object": doc.object, "version": doc.version, "kind": doc.kind.name() });
    match &doc.body {
        Body::Fields(f) => d["fields"] = serde_json::Value::Object(f.clone()),
        Body::Tombstone => d["tombstone"] = serde_json::json!(1),
    }
    let d = d.to_string();
    let sig = owner.sign(&[DOC_SIG_DOMAIN, d.as_bytes()].concat());
    let plain = serde_json::json!({ "doc": d, "sig": hex::encode(sig) }).to_string();
    let sealed = crate::seal::seal_with_key(plain.as_bytes(), &hosted.secret, hosted.key.as_bytes())?;
    if sealed.len() > HOSTED_MAX_BYTES {
        return Err(bad(&format!("{} bytes, past media.hosted.maxBytes", sealed.len())));
    }
    Ok(sealed)
}

/// Open a fetched document and check it: the pointer's object and kind, the carried exterior's
/// owner's signature, and fields its kind lists. Anything that does not check is an error.
pub fn open(sealed: &[u8], hosted: &Hosted, owner: &[u8; 32], object: &str, kind: ObjectKind) -> Result<Doc, CoreError> {
    if sealed.len() > HOSTED_MAX_BYTES {
        return Err(bad("past media.hosted.maxBytes"));
    }
    let plain = crate::seal::open_with_key(sealed, &hosted.secret, hosted.key.as_bytes())?;
    let outer: serde_json::Value = serde_json::from_slice(&plain).map_err(|_| bad("not JSON"))?;
    let (Some(d), Some(sig)) = (outer["doc"].as_str(), outer["sig"].as_str()) else {
        return Err(bad("no doc or sig"));
    };
    let sig: [u8; 64] = hex::decode(sig).ok().and_then(|s| s.try_into().ok()).ok_or_else(|| bad("a signature that is not 64 bytes"))?;
    crate::identity::verify_sig(owner, &[DOC_SIG_DOMAIN, d.as_bytes()].concat(), &sig).map_err(|_| bad("not signed by the owner"))?;
    let d: serde_json::Value = serde_json::from_str(d).map_err(|_| bad("doc is not JSON"))?;
    if d["object"].as_str() != Some(object) {
        return Err(bad("another object's"));
    }
    if d["kind"].as_str() != Some(kind.name()) {
        return Err(bad("another kind's"));
    }
    let version = d["version"].as_i64().ok_or_else(|| bad("no version"))?;
    let body = if d["tombstone"].as_i64() == Some(1) {
        Body::Tombstone
    } else {
        let f = d["fields"].as_object().ok_or_else(|| bad("no fields"))?;
        let declared = exterior::declared_fields(kind);
        if let Some(k) = f.keys().find(|k| !declared.contains(&k.as_str())) {
            return Err(bad(&format!("`{k}` is not in facets.exterior.fields")));
        }
        Body::Fields(f.clone())
    };
    Ok(Doc { object: object.into(), kind, version, body })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;

    fn doc(body: Body) -> Doc {
        Doc { object: "ab".repeat(16), kind: ObjectKind::Event, version: 7, body }
    }

    fn fields() -> Body {
        Body::Fields(serde_json::json!({ "name": "Gig", "going": 3 }).as_object().unwrap().clone())
    }

    /// Two devices of one identity, and a restore, land on one key and one content key; another
    /// object, or another person, lands elsewhere.
    #[test]
    fn every_owner_device_derives_one_place() {
        let (a, b) = (keys(&[1; 32], "obj"), keys(&[1; 32], "obj"));
        assert_eq!(a.hosted, b.hosted);
        assert_ne!(keys(&[1; 32], "other").hosted.key, a.hosted.key);
        assert_ne!(keys(&[2; 32], "obj").hosted, a.hosted);
        assert_ne!(a.hosted.key.as_bytes(), &a.hosted.secret[..], "the key is public, the content key is not");
    }

    #[test]
    fn a_document_opens_for_its_pointer_and_owner_alone() {
        let owner = Identity::in_memory([4; 32]);
        let k = keys(&[4; 32], &"ab".repeat(16)).hosted;
        let sealed = seal(&owner, &k, &doc(fields())).unwrap();
        let pk = owner.identity_pk();
        assert_eq!(open(&sealed, &k, &pk, &"ab".repeat(16), ObjectKind::Event).unwrap(), doc(fields()));
        let other = Identity::in_memory([5; 32]);
        assert!(open(&seal(&other, &k, &doc(fields())).unwrap(), &k, &pk, &"ab".repeat(16), ObjectKind::Event).is_err(), "signed by someone else");
        assert!(open(&sealed, &k, &pk, &"cd".repeat(16), ObjectKind::Event).is_err(), "passed off as another object");
        assert!(open(&sealed, &k, &pk, &"ab".repeat(16), ObjectKind::Post).is_err(), "as another kind");
        let wrong = keys(&[6; 32], "x").hosted;
        assert!(open(&sealed, &wrong, &pk, &"ab".repeat(16), ObjectKind::Event).is_err(), "under another key");
        let t = seal(&owner, &k, &doc(Body::Tombstone)).unwrap();
        assert_eq!(open(&t, &k, &pk, &"ab".repeat(16), ObjectKind::Event).unwrap().body, Body::Tombstone);
    }

    /// A field the kind does not list is refused at the reader, as the builder never writes one.
    #[test]
    fn a_field_the_icd_does_not_list_is_refused() {
        let owner = Identity::in_memory([4; 32]);
        let k = keys(&[4; 32], "o").hosted;
        let leak = Body::Fields(serde_json::json!({ "name": "Gig", "acct": "acct_123" }).as_object().unwrap().clone());
        let sealed = seal(&owner, &k, &Doc { object: "o".into(), kind: ObjectKind::Event, version: 1, body: leak }).unwrap();
        assert!(open(&sealed, &k, &owner.identity_pk(), "o", ObjectKind::Event).is_err());
    }

    /// The domain and salt this module uses are the ones the ICD states.
    #[test]
    fn the_construction_is_the_icds() {
        let icd: serde_json::Value = serde_json::from_str(include_str!("../../coordination/delta-graph.icd.json")).unwrap();
        let h = &icd["media"]["hosted"];
        let plain = h["document"]["plaintext"].as_str().unwrap();
        assert!(plain.contains(&String::from_utf8_lossy(DOC_SIG_DOMAIN).replace('\n', "\\n")), "{plain}");
        assert!(h["seed"].as_str().unwrap().contains(HOSTED_SALT));
    }
}
