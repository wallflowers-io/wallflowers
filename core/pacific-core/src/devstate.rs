//! A device's state, sealed at rest under a key off the seed (D-34 (c); mdr/door.md §4).
//!
//! The Door runs one person's Node per process, its directory on tmpfs. Between
//! sessions that directory is this envelope and nothing else: sealed under a key only
//! the seed reaches, so the passkey opens it (PRF → wrap → seed) and so do the words.
//!
//! It is a CACHE of one device's MLS state, not account state: everything in it is on
//! the relay too, and losing it costs the device a new leaf (resumption.md §6.3).
//!
//! THE KEY is the head's shape, off the storage root, in the `pacific/storage/` family:
//! HKDF-SHA256(salt = domain, ikm = storage_root, info = "v1").
//!
//! THE HEADER IS IN THE CLEAR and bound as associated data with the person's key
//! (DQ-6): the node it was sealed on, the device it holds, and its counter. A seal
//! relabelled, or moved to another person or node, does not open. Whether a counter is
//! current is the caller's to judge, against the one the auth service keeps.
//!
//! Envelope: `PDS1 ‖ be64 counter ‖ device_id(32) ‖ be16 len ‖ node ‖ nonce(24) ‖ ct`.

use std::path::Path;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::CoreError;

/// The HKDF salt and the first bytes of the associated data.
pub const SEAL_DOMAIN: &[u8] = b"pacific/storage/devstate/seal/v1";
const MAGIC: &[u8; 4] = b"PDS1";
const NONCE_LEN: usize = 24;
/// A node is a host name.
const NODE_MAX: usize = 253;
const CONTENTS_VERSION: u32 = 1;

/// The files a state is, by name in the state directory. `pacific.db` holds the
/// directory and the MLS stores in one file.
pub const FILES: &[&str] = &["pacific.db", "id_ed25519"];
/// Not state: the process that opens a seal writes its own, from its configuration, so
/// the relay and the Arc it is configured with win (NC-85). A seal made before 28 Sep
/// carries `arc_url`; it is skipped on restore.
pub const OPENERS: &[&str] = &["routes", "arc_url"];
/// Where `Node::seal_state` copies the database, inside the state directory and only
/// for as long as the seal takes: never anywhere a seal is kept.
pub const SNAPSHOT: &str = ".devstate-snapshot.db";

/// The seal key, from the storage root.
pub fn seal_key(storage_root: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let hk = hkdf::Hkdf::<sha2::Sha256>::new(Some(SEAL_DOMAIN), storage_root);
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(b"v1", out.as_mut()).expect("32-byte OKM");
    out
}

/// What a seal says of itself before it is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// The node it was sealed on: the Door's host.
    pub node: String,
    /// The device whose leaves it holds (`Directory::device_id`).
    pub device_id: [u8; 32],
    pub counter: u64,
}

impl Header {
    fn encode(&self) -> Result<Vec<u8>, CoreError> {
        let node = self.node.as_bytes();
        if node.is_empty() || node.len() > NODE_MAX {
            return Err(CoreError::Seal(format!("a node is 1 to {NODE_MAX} bytes, got {}", node.len())));
        }
        let mut v = Vec::with_capacity(4 + 8 + 32 + 2 + node.len());
        v.extend_from_slice(MAGIC);
        v.extend_from_slice(&self.counter.to_be_bytes());
        v.extend_from_slice(&self.device_id);
        v.extend_from_slice(&(node.len() as u16).to_be_bytes());
        v.extend_from_slice(node);
        Ok(v)
    }
}

/// What `Node::drain_only` found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Drained {
    /// Messages from this device's own leaf that this state never published: another
    /// copy of it has spoken since this one was sealed. Above 0, the state is stale.
    pub own_unsent: u64,
    /// The objects drained.
    pub groups: usize,
}

/// The state itself: the at-rest device key its secrets are sealed under, and its files.
pub struct Contents {
    pub device_key: Zeroizing<[u8; 32]>,
    pub files: Vec<(String, Zeroizing<Vec<u8>>)>,
}

#[derive(Serialize, Deserialize)]
struct Wire {
    v: u32,
    #[serde(with = "serde_bytes")]
    device_key: Vec<u8>,
    files: Vec<(String, serde_bytes::ByteBuf)>,
}

fn aad(pk: &[u8; 32], header: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(SEAL_DOMAIN.len() + 32 + header.len());
    v.extend_from_slice(SEAL_DOMAIN);
    v.extend_from_slice(pk);
    v.extend_from_slice(header);
    v
}

/// Seal `contents` for the person `pk`, under `key` ([`seal_key`]).
pub fn seal(pk: &[u8; 32], header: &Header, contents: &Contents, key: &[u8; 32]) -> Result<Vec<u8>, CoreError> {
    for (name, _) in &contents.files {
        if !FILES.contains(&name.as_str()) {
            return Err(CoreError::Seal(format!("{name:?} is not a state file")));
        }
    }
    encrypt(pk, header, contents, key)
}

fn encrypt(pk: &[u8; 32], header: &Header, contents: &Contents, key: &[u8; 32]) -> Result<Vec<u8>, CoreError> {
    let mut wire = Wire {
        v: CONTENTS_VERSION,
        device_key: contents.device_key.to_vec(),
        files: contents.files.iter().map(|(n, b)| (n.clone(), serde_bytes::ByteBuf::from(b.to_vec()))).collect(),
    };
    let mut plain = Zeroizing::new(Vec::new());
    let encoded = ciborium::into_writer(&wire, &mut *plain);
    {
        use zeroize::Zeroize;
        wire.device_key.zeroize();
        for (_, b) in wire.files.iter_mut() {
            let b: &mut Vec<u8> = b;
            b.zeroize();
        }
    }
    encoded.map_err(|e| CoreError::Seal(format!("state encode: {e}")))?;

    let head = header.encode()?;
    let mut nonce = [0u8; NONCE_LEN];
    crate::head::getrandom_fill(&mut nonce)?;
    let ct = XChaCha20Poly1305::new(key.into())
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: &plain[..], aad: &aad(pk, &head) })
        .map_err(|_| CoreError::Seal("state seal failed".into()))?;
    let mut out = head;
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// The header, and where the nonce begins.
fn parse(blob: &[u8]) -> Result<(Header, usize), CoreError> {
    let short = || CoreError::Seal(format!("not a sealed state ({} bytes)", blob.len()));
    if blob.len() < 4 + 8 + 32 + 2 || &blob[..4] != MAGIC {
        return Err(short());
    }
    let counter = u64::from_be_bytes(blob[4..12].try_into().expect("8 bytes"));
    let device_id: [u8; 32] = blob[12..44].try_into().expect("32 bytes");
    let len = u16::from_be_bytes(blob[44..46].try_into().expect("2 bytes")) as usize;
    let end = 46 + len;
    if len == 0 || len > NODE_MAX || blob.len() < end + NONCE_LEN + 16 {
        return Err(short());
    }
    let node = String::from_utf8(blob[46..end].to_vec()).map_err(|_| CoreError::Seal("a seal's node is not UTF-8".into()))?;
    Ok((Header { node, device_id, counter }, end))
}

/// What a seal says of itself, unverified: the caller's counter check needs it before
/// it has a key, and the AEAD holds every byte of it once the seal is opened.
pub fn peek(blob: &[u8]) -> Result<Header, CoreError> {
    parse(blob).map(|(h, _)| h)
}

/// Open a seal made for `pk` on `node`. Another person's, another node's, a relabelled
/// or a tampered seal fails here, and nothing of it is returned.
pub fn open(blob: &[u8], pk: &[u8; 32], node: &str, key: &[u8; 32]) -> Result<(Header, Contents), CoreError> {
    let (header, end) = parse(blob)?;
    if header.node != node {
        return Err(CoreError::Seal(format!("this state was sealed on {}, not {node}", header.node)));
    }
    let nonce = &blob[end..end + NONCE_LEN];
    let plain = Zeroizing::new(
        XChaCha20Poly1305::new(key.into())
            .decrypt(XNonce::from_slice(nonce), Payload { msg: &blob[end + NONCE_LEN..], aad: &aad(pk, &blob[..end]) })
            .map_err(|_| CoreError::Seal("the state will not open (another person's, relabelled, or tampered)".into()))?,
    );
    let wire: Wire = ciborium::from_reader(&plain[..]).map_err(|e| CoreError::Seal(format!("state decode: {e}")))?;
    if wire.v != CONTENTS_VERSION {
        return Err(CoreError::Seal(format!("state version {} (this build reads {CONTENTS_VERSION})", wire.v)));
    }
    let device_key: [u8; 32] = wire
        .device_key
        .as_slice()
        .try_into()
        .map_err(|_| CoreError::Seal("a state's device key is 32 bytes".into()))?;
    drop(Zeroizing::new(wire.device_key));
    let files = wire.files.into_iter().map(|(n, b)| (n, Zeroizing::new(b.into_vec()))).collect();
    Ok((header, Contents { device_key: Zeroizing::new(device_key), files }))
}

/// Put an opened state into `dir`, which must hold none, and take up its device key.
pub fn restore(contents: &Contents, dir: &Path) -> Result<(), CoreError> {
    std::fs::create_dir_all(dir)?;
    for name in FILES {
        if dir.join(name).exists() {
            return Err(CoreError::Seal(format!("{} already holds {name}: a state restores into an empty directory", dir.display())));
        }
    }
    for (name, bytes) in &contents.files {
        if OPENERS.contains(&name.as_str()) {
            continue;
        }
        if !FILES.contains(&name.as_str()) {
            return Err(CoreError::Seal(format!("{name:?} is not a state file")));
        }
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        std::io::Write::write_all(&mut o.open(dir.join(name))?, bytes)?;
    }
    crate::atrest::set_device_key(*contents.device_key);
    Ok(())
}

/// Take a state out of `dir`: its files, the database's journal and any snapshot, and
/// the at-rest key with them, replaced by a new one. What is left is a directory a
/// device that has never seen the account can start in.
pub fn clear(dir: &Path) -> Result<(), CoreError> {
    let journal = ["pacific.db-wal", "pacific.db-shm", SNAPSHOT];
    for name in FILES.iter().chain(journal.iter()) {
        match std::fs::remove_file(dir.join(name)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    let mut key = Zeroizing::new([0u8; 32]);
    crate::head::getrandom_fill(key.as_mut())?;
    crate::atrest::set_device_key(*key);
    Ok(())
}

/// Removes its file when dropped, whatever happened in between.
pub(crate) struct Scrub(pub std::path::PathBuf);

impl Drop for Scrub {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NODE: &str = "door.wallflowers.io";

    fn contents() -> Contents {
        Contents {
            device_key: Zeroizing::new([7u8; 32]),
            files: vec![
                ("pacific.db".into(), Zeroizing::new(b"a marker the seal must hide".to_vec())),
                ("id_ed25519".into(), Zeroizing::new(b"sealed seed".to_vec())),
            ],
        }
    }

    fn header(counter: u64) -> Header {
        Header { node: NODE.into(), device_id: [3u8; 32], counter }
    }

    #[test]
    fn a_seal_opens_for_its_person_and_node_and_hides_what_it_holds() {
        let key = seal_key(&[1u8; 32]);
        let blob = seal(&[9u8; 32], &header(5), &contents(), &key).unwrap();
        assert!(!blob.windows(11).any(|w| w == b"a marker th"));
        assert_eq!(peek(&blob).unwrap(), header(5));
        let (h, c) = open(&blob, &[9u8; 32], NODE, &key).unwrap();
        assert_eq!(h, header(5));
        assert_eq!(*c.device_key, [7u8; 32]);
        assert_eq!(c.files.len(), 2);
        assert_eq!(&c.files[0].1[..], b"a marker the seal must hide");
    }

    #[test]
    fn another_person_another_node_or_another_key_does_not_open_it() {
        let key = seal_key(&[1u8; 32]);
        let blob = seal(&[9u8; 32], &header(5), &contents(), &key).unwrap();
        assert!(open(&blob, &[8u8; 32], NODE, &key).is_err(), "another person");
        assert!(open(&blob, &[9u8; 32], "door.example", &key).is_err(), "another node");
        assert!(open(&blob, &[9u8; 32], NODE, &seal_key(&[2u8; 32])).is_err(), "another root");
    }

    #[test]
    fn a_relabelled_or_tampered_seal_does_not_open() {
        let key = seal_key(&[1u8; 32]);
        let blob = seal(&[9u8; 32], &header(5), &contents(), &key).unwrap();

        let mut later = blob.clone();
        later[4..12].copy_from_slice(&6u64.to_be_bytes());
        assert_eq!(peek(&later).unwrap().counter, 6, "the header reads as relabelled");
        assert!(open(&later, &[9u8; 32], NODE, &key).is_err(), "a relabelled counter");

        let mut moved = blob.clone();
        moved[12] ^= 1;
        assert!(open(&moved, &[9u8; 32], NODE, &key).is_err(), "a relabelled device");

        let mut tampered = blob.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(open(&tampered, &[9u8; 32], NODE, &key).is_err(), "a tampered byte");

        assert!(peek(&blob[..40]).is_err(), "a short blob");
    }

    #[test]
    fn the_words_reach_the_key_the_passkey_reaches() {
        let seed = [42u8; 32];
        let words = crate::identity::recovery_key_from_seed(&seed);
        let from_words = crate::identity::seed_from_recovery_key(&words).unwrap();
        let key = seal_key(&crate::locator::storage_root(&seed));
        let blob = seal(&[9u8; 32], &header(1), &contents(), &key).unwrap();
        let again = seal_key(&crate::locator::storage_root(&from_words));
        assert!(open(&blob, &[9u8; 32], NODE, &again).is_ok());
    }

    #[test]
    fn only_state_files_are_sealed_or_restored() {
        let key = seal_key(&[1u8; 32]);
        let mut c = contents();
        c.files.push(("../escape".into(), Zeroizing::new(vec![1])));
        assert!(seal(&[9u8; 32], &header(1), &c, &key).is_err());
        for opener in OPENERS {
            let mut c = contents();
            c.files.push((opener.to_string(), Zeroizing::new(vec![1])));
            assert!(seal(&[9u8; 32], &header(1), &c, &key).is_err(), "{opener} is the opener's");
        }
    }

    /// NC-85: a seal made before 28 Sep carries `arc_url`. It opens, and the opener's own
    /// Arc stands: the sealed one is not written.
    #[test]
    fn a_seal_carrying_its_arc_restores_without_it() {
        let key = seal_key(&[1u8; 32]);
        let mut c = contents();
        c.files.push(("arc_url".into(), Zeroizing::new(b"wss://arc.example/v1/relay".to_vec())));
        let blob = encrypt(&[9u8; 32], &header(1), &c, &key).unwrap();
        let (_, opened) = open(&blob, &[9u8; 32], NODE, &key).unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("arc_url"), b"ws://127.0.0.1:8787/v1/relay").unwrap();
        // restore takes up the seal's at-rest key, which is process-wide.
        let _g = crate::TEST_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let restored = restore(&opened, dir.path());
        crate::atrest::clear_device_key();
        restored.unwrap();
        assert!(dir.path().join("pacific.db").exists());
        assert_eq!(std::fs::read(dir.path().join("arc_url")).unwrap(), b"ws://127.0.0.1:8787/v1/relay");
    }
}
