//! training_wheels' SPOOL: the outbox that keeps shadow mode from ever failing the MLS path
//! (core/docs/launch/mdr/training-wheels.md). While a row cannot go straight to the database,
//! the capture appends it here and answers; a drainer hands the rows to the database, in order,
//! when it returns. Only a spool that cannot be written fails the action.
//!
//! SEALED (Software Security, B3). The spool sits outside the encrypted volume and fills while
//! that volume is shut, so every row is sealed to a PUBLIC key before it touches the disk:
//! X25519 from a fresh ephemeral key to the spool's key (`DOOR_TW_SPOOL_PUB`), HKDF-SHA256 (salt
//! `wallflowers/door/tw-spool/v1`, info ephemeral ‖ recipient), AES-256-GCM, as `seal.rs` opens a
//! sealed PRF. Writing needs no secret. The private half is a file inside the volume
//! (`DOOR_TW_SPOOL_KEY`), read only by a drain: while it is absent, the drain waits and deletes
//! nothing.
//!
//! Files in the spool's directory, each one base64url record (ephemeral ‖ nonce ‖ ciphertext) per
//! line:
//! - `live.ndjson` takes every append, each line written and fsync'd before `append` returns;
//! - `draining.ndjson` is a live file a drain has rotated out. It is drained entirely before the
//!   live file, so order holds, and appends never wait on a drain: they go on to a fresh live file;
//! - `aside.ndjson` keeps, sealed and as written, every line a drain could not deliver (one that
//!   does not open with the key, or is not a row), each named. Nothing is dropped.
//!
//! ACKNOWLEDGED means the sink returned `Ok` for a batch. Its end offset is then written to
//! `draining.ack` and fsync'd, and no later drain, in this process or after a restart, hands
//! those rows to the sink again. The draining file is removed only once the sink has taken all of
//! it. A crash between the sink's `Ok` and that fsync hands that one batch over again, which is
//! safe because the sink is idempotent: it inserts each row ON CONFLICT DO NOTHING on its key (an
//! `action`'s UUIDv7 `id`; a `delta`'s `object` and `delta_id`).

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

const LIVE: &str = "live.ndjson";
const DRAINING: &str = "draining.ndjson";
const ASIDE: &str = "aside.ndjson";
const ACK: &str = "draining.ack";
const DEFAULT_DIR: &str = "/var/lib/door/tw-spool";
const BATCH: usize = 256;
const SALT: &[u8] = b"wallflowers/door/tw-spool/v1";
const TABLES: [&str; 2] = ["action", "delta"];

/// One row: a JSON object whose `table` is "action" or "delta", beside the row's own fields.
pub type Row = serde_json::Value;

/// Where drained rows go: the database, in order. `Ok` acknowledges the whole batch. Called from
/// a blocking thread; it must be idempotent on each row's key (see the module's doc).
pub trait Sink {
    fn put(&self, rows: &[Row]) -> Result<(), String>;
}

/// A line not delivered, named by file and byte offset only, never its contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    pub file: &'static str,
    pub at: u64,
    pub bytes: u64,
    pub why: &'static str,
}

impl std::fmt::Display for Skipped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at byte {}, {} bytes: {}", self.file, self.at, self.bytes, self.why)
    }
}

/// What a drain did: the rows the sink acknowledged, and every line set aside.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Drained {
    pub taken: usize,
    pub skipped: Vec<Skipped>,
}

#[derive(Debug)]
pub enum DrainError {
    /// The key file is absent: the volume is shut. Nothing was read or removed.
    Locked(PathBuf),
    /// The key file cannot be used. Nothing was read or removed.
    Key(String),
    /// The sink refused a batch. Everything it had not acknowledged is kept.
    Sink(String),
    Io(io::Error),
}

impl std::fmt::Display for DrainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Locked(p) => write!(f, "the spool's key {} is absent; rows kept", p.display()),
            Self::Key(e) => write!(f, "the spool's key: {e}; rows kept"),
            Self::Sink(e) => write!(f, "the sink refused a batch, kept: {e}"),
            Self::Io(e) => write!(f, "the spool could not be read: {e}"),
        }
    }
}

impl From<io::Error> for DrainError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

pub struct Spool {
    dir: PathBuf,
    recipient: PublicKey,
    key: PathBuf,
    live: Mutex<File>,
    drain: Mutex<()>,
    batch: usize,
    torn: Vec<Skipped>,
}

impl Spool {
    /// The spool at `DOOR_TW_SPOOL` (else /var/lib/door/tw-spool), sealing to `DOOR_TW_SPOOL_PUB`
    /// (32 bytes, base64url), draining with the key file at `DOOR_TW_SPOOL_KEY`.
    pub fn from_env() -> io::Result<Self> {
        let var = |k: &str| std::env::var(k).map_err(|_| io::Error::other(format!("{k} is unset")));
        let recipient = public_key(&var("DOOR_TW_SPOOL_PUB")?)
            .map_err(|e| io::Error::other(format!("DOOR_TW_SPOOL_PUB: {e}")))?;
        let dir = std::env::var("DOOR_TW_SPOOL").unwrap_or_else(|_| DEFAULT_DIR.into());
        Self::open(dir, recipient, var("DOOR_TW_SPOOL_KEY")?)
    }

    /// Open the spool in `dir` (0700, its files 0600). A torn last line, from a crash mid-append,
    /// is cut off here, so later appends start on a line of their own, and is named in
    /// [`Self::torn`]: its append never returned, so its action was never acknowledged.
    pub fn open(dir: impl AsRef<Path>, recipient: PublicKey, key: impl AsRef<Path>) -> io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
        let mut torn = Vec::new();
        for name in [DRAINING, LIVE] {
            if let Some(t) = cut_torn_tail(&dir.join(name), name)? {
                eprintln!("tw spool: a torn last line skipped: {t}");
                torn.push(t);
            }
        }
        // An ack with no draining file is stale: its file was taken whole.
        if !dir.join(DRAINING).exists() {
            remove_if_present(&dir.join(ACK))?;
        }
        let live = open_append(&dir.join(LIVE))?;
        sync_dir(&dir)?;
        Ok(Self {
            dir,
            recipient,
            key: key.as_ref().to_path_buf(),
            live: Mutex::new(live),
            drain: Mutex::new(()),
            batch: BATCH,
            torn,
        })
    }

    /// Rows handed to the sink per `put`.
    pub fn with_batch(mut self, batch: usize) -> Self {
        self.batch = batch.max(1);
        self
    }

    /// The torn tails [`Self::open`] cut off.
    pub fn torn(&self) -> &[Skipped] {
        &self.torn
    }

    /// Seal `row` and append it as one line, written and fsync'd before this returns. An error
    /// means the row is not spooled, and the caller fails the action. A row with no known
    /// `table` is refused: no drain could deliver it.
    pub fn append(&self, row: &Row) -> io::Result<()> {
        if !row["table"].as_str().is_some_and(|t| TABLES.contains(&t)) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "a row's table is action or delta"));
        }
        let plain = Zeroizing::new(serde_json::to_vec(row).map_err(io::Error::other)?);
        let mut line = seal(&self.recipient, &plain)?.into_bytes();
        line.push(b'\n');
        let mut live = self.live.lock().unwrap_or_else(|p| p.into_inner());
        live.write_all(&line)?;
        live.sync_data()
    }

    /// Whether any row, live or draining, is not yet acknowledged by the sink. The capture
    /// writes straight to the database only when this is false, so rows arrive in order. A
    /// spool that cannot tell says true.
    pub fn pending(&self) -> bool {
        !self.is_empty().unwrap_or(false)
    }

    fn is_empty(&self) -> io::Result<bool> {
        let live = self.live.lock().unwrap_or_else(|p| p.into_inner());
        Ok(!self.dir.join(DRAINING).exists() && live.metadata()?.len() == 0)
    }

    /// One pass: a draining file left from before is drained first; then the live file is
    /// rotated out and drained. The key file is read first; absent, nothing is touched. On a
    /// sink failure the pass stops, keeping every row the sink has not acknowledged; the next
    /// pass resumes after the last acknowledged batch.
    pub fn drain(&self, sink: &dyn Sink) -> Result<Drained, DrainError> {
        let _one = self.drain.lock().unwrap_or_else(|p| p.into_inner());
        let mut out = Drained::default();
        if !self.pending() {
            return Ok(out);
        }
        let secret = self.secret()?;
        if self.dir.join(DRAINING).exists() {
            self.drain_file(&secret, sink, &mut out)?;
        }
        if self.rotate()? {
            self.drain_file(&secret, sink, &mut out)?;
        }
        Ok(out)
    }

    /// Drain until nothing is pending or `stop` is set, retrying a failed pass (the volume shut,
    /// the database down) after a wait that doubles from 100 ms to 30 s, reset by a pass that
    /// succeeds.
    pub fn drain_until_empty(&self, sink: &dyn Sink, stop: &AtomicBool) -> Drained {
        let mut all = Drained::default();
        let mut wait = Duration::from_millis(100);
        while !stop.load(Ordering::Relaxed) {
            match self.drain(sink) {
                Ok(d) => {
                    all.taken += d.taken;
                    all.skipped.extend(d.skipped);
                    wait = Duration::from_millis(100);
                    if !self.pending() {
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("tw spool: {e}; retrying in {} ms", wait.as_millis());
                    let until = std::time::Instant::now() + wait;
                    while std::time::Instant::now() < until && !stop.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    wait = (wait * 2).min(Duration::from_secs(30));
                }
            }
        }
        all
    }

    /// The private key, from its file: 32 bytes, base64url. Its public half must be the one the
    /// spool seals to, or every row would be set aside.
    fn secret(&self) -> Result<StaticSecret, DrainError> {
        let text = match fs::read_to_string(&self.key) {
            Ok(t) => Zeroizing::new(t),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(DrainError::Locked(self.key.clone())),
            Err(e) => return Err(DrainError::Io(e)),
        };
        let bytes: Zeroizing<[u8; 32]> = B64
            .decode(text.trim())
            .ok()
            .and_then(|v| <[u8; 32]>::try_from(v.as_slice()).ok())
            .map(Zeroizing::new)
            .ok_or_else(|| DrainError::Key("not 32 bytes of base64url".into()))?;
        let secret = StaticSecret::from(*bytes);
        if PublicKey::from(&secret) != self.recipient {
            return Err(DrainError::Key("its public half is not DOOR_TW_SPOOL_PUB".into()));
        }
        Ok(secret)
    }

    /// Move a non-empty live file aside as the draining file, and start a fresh live one. Under
    /// the append lock, so no append lands between the two.
    fn rotate(&self) -> io::Result<bool> {
        let mut live = self.live.lock().unwrap_or_else(|p| p.into_inner());
        if live.metadata()?.len() == 0 {
            return Ok(false);
        }
        remove_if_present(&self.dir.join(ACK))?;
        fs::rename(self.dir.join(LIVE), self.dir.join(DRAINING))?;
        *live = open_append(&self.dir.join(LIVE))?;
        sync_dir(&self.dir)?;
        Ok(true)
    }

    /// Hand the draining file to the sink from its last acknowledged offset, a batch at a time,
    /// setting aside each line that is not a deliverable row; remove the file once the sink has
    /// taken everything.
    fn drain_file(&self, secret: &StaticSecret, sink: &dyn Sink, out: &mut Drained) -> Result<(), DrainError> {
        let path = self.dir.join(DRAINING);
        let mut at = read_ack(&self.dir.join(ACK))?;
        let mut file = File::open(&path)?;
        file.seek(SeekFrom::Start(at))?;
        let mut lines = BufReader::new(file);
        let mut batch: Vec<Row> = Vec::with_capacity(self.batch);
        let mut line = Vec::new();
        loop {
            line.clear();
            let n = lines.read_until(b'\n', &mut line)? as u64;
            if n > 0 {
                match deliverable(secret, &self.recipient, &line) {
                    Ok(row) => batch.push(row),
                    Err(why) => {
                        let mut aside = open_append(&self.dir.join(ASIDE))?;
                        aside.write_all(&line)?;
                        if line.last() != Some(&b'\n') {
                            aside.write_all(b"\n")?;
                        }
                        aside.sync_data()?;
                        let s = Skipped { file: DRAINING, at, bytes: n, why };
                        eprintln!("tw spool: set aside in {ASIDE}: {s}");
                        out.skipped.push(s);
                    }
                }
                at += n;
            }
            if batch.len() == self.batch || (n == 0 && !batch.is_empty()) {
                sink.put(&batch).map_err(DrainError::Sink)?;
                write_ack(&self.dir, at)?;
                out.taken += batch.len();
                batch.clear();
            }
            if n == 0 {
                break;
            }
        }
        fs::remove_file(&path)?;
        remove_if_present(&self.dir.join(ACK))?;
        sync_dir(&self.dir)?;
        Ok(())
    }
}

impl super::Outbox for Spool {
    fn pending(&self) -> bool {
        Spool::pending(self)
    }

    fn append(&self, row: &Row) -> Result<(), String> {
        Spool::append(self, row).map_err(|e| format!("the spool could not take a row: {e}"))
    }
}

/// A new key pair: the secret written to `path` as base64url text, one line, 0400, refusing a
/// file already there; the public half returned, in the same encoding, for DOOR_TW_SPOOL_PUB.
pub fn keygen(path: &Path) -> io::Result<String> {
    let secret = StaticSecret::random_from_rng(rand_core::OsRng);
    let text = Zeroizing::new(format!("{}\n", B64.encode(secret.to_bytes())));
    let mut f = OpenOptions::new().write(true).create_new(true).mode(0o400).open(path).map_err(|e| {
        if e.kind() == io::ErrorKind::AlreadyExists {
            io::Error::new(e.kind(), format!("{} exists; refusing to replace a key", path.display()))
        } else {
            e
        }
    })?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    Ok(B64.encode(PublicKey::from(&secret).as_bytes()))
}

/// A public key from 32 bytes of base64url.
pub fn public_key(b64: &str) -> Result<PublicKey, &'static str> {
    B64.decode(b64.trim())
        .ok()
        .and_then(|v| <[u8; 32]>::try_from(v.as_slice()).ok())
        .map(PublicKey::from)
        .ok_or("not 32 bytes of base64url")
}

/// The AES key for one row, from the ephemeral-static X25519 secret.
fn row_key(shared: &[u8; 32], ephemeral: &PublicKey, recipient: &PublicKey) -> io::Result<Zeroizing<[u8; 32]>> {
    let mut info = [0u8; 64];
    info[..32].copy_from_slice(ephemeral.as_bytes());
    info[32..].copy_from_slice(recipient.as_bytes());
    let mut key = Zeroizing::new([0u8; 32]);
    hkdf::Hkdf::<sha2::Sha256>::new(Some(SALT), shared)
        .expand(&info, key.as_mut())
        .map_err(|_| io::Error::other("hkdf"))?;
    Ok(key)
}

/// ephemeral ‖ nonce ‖ AES-256-GCM(plain), base64url.
fn seal(recipient: &PublicKey, plain: &[u8]) -> io::Result<String> {
    let ephemeral = StaticSecret::random_from_rng(rand_core::OsRng);
    let epk = PublicKey::from(&ephemeral);
    let shared = Zeroizing::new(ephemeral.diffie_hellman(recipient).to_bytes());
    let key = row_key(&shared, &epk, recipient)?;
    let mut nonce = [0u8; 12];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut nonce);
    let ct = Aes256Gcm::new_from_slice(key.as_ref())
        .map_err(|_| io::Error::other("aes key"))?
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: plain, aad: SALT })
        .map_err(|_| io::Error::other("aes-gcm"))?;
    let mut record = Vec::with_capacity(32 + 12 + ct.len());
    record.extend_from_slice(epk.as_bytes());
    record.extend_from_slice(&nonce);
    record.extend_from_slice(&ct);
    Ok(B64.encode(record))
}

/// A whole line, opened and parsed into a row with a known table, or why not.
fn deliverable(secret: &StaticSecret, recipient: &PublicKey, line: &[u8]) -> Result<Row, &'static str> {
    let Some(body) = line.strip_suffix(b"\n") else {
        return Err("a torn last line");
    };
    let record = B64.decode(body).map_err(|_| "not a sealed record")?;
    if record.len() < 32 + 12 + 16 {
        return Err("not a sealed record");
    }
    let epk = PublicKey::from(<[u8; 32]>::try_from(&record[..32]).expect("32 bytes"));
    let shared = Zeroizing::new(secret.diffie_hellman(&epk).to_bytes());
    let key = row_key(&shared, &epk, recipient).map_err(|_| "hkdf")?;
    let plain = Zeroizing::new(
        Aes256Gcm::new_from_slice(key.as_ref())
            .map_err(|_| "aes key")?
            .decrypt(Nonce::from_slice(&record[32..44]), Payload { msg: &record[44..], aad: SALT })
            .map_err(|_| "does not open with the key")?,
    );
    let row: Row = serde_json::from_slice(&plain).map_err(|_| "opens, but is not a row")?;
    if !row["table"].as_str().is_some_and(|t| TABLES.contains(&t)) {
        return Err("opens, but names no table");
    }
    Ok(row)
}

fn open_append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).mode(0o600).open(path)
}

/// Cut `path` back to its last newline, naming what was cut.
fn cut_torn_tail(path: &Path, name: &'static str) -> io::Result<Option<Skipped>> {
    let mut file = match OpenOptions::new().read(true).write(true).open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let len = file.metadata()?.len();
    if len == 0 {
        return Ok(None);
    }
    let mut last = [0u8; 1];
    file.seek(SeekFrom::Start(len - 1))?;
    file.read_exact(&mut last)?;
    if last[0] == b'\n' {
        return Ok(None);
    }
    let mut keep = 0u64;
    let mut pos = len;
    let mut chunk = vec![0u8; 64 * 1024];
    while pos > 0 {
        let take = chunk.len().min(pos as usize);
        pos -= take as u64;
        file.seek(SeekFrom::Start(pos))?;
        file.read_exact(&mut chunk[..take])?;
        if let Some(i) = chunk[..take].iter().rposition(|&b| b == b'\n') {
            keep = pos + i as u64 + 1;
            break;
        }
    }
    file.set_len(keep)?;
    file.sync_all()?;
    Ok(Some(Skipped { file: name, at: keep, bytes: len - keep, why: "a torn last line" }))
}

fn read_ack(path: &Path) -> io::Result<u64> {
    match fs::read_to_string(path) {
        Ok(s) => s.trim().parse().map_err(io::Error::other),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e),
    }
}

/// Record the offset the sink has taken through: written aside, fsync'd, renamed over.
fn write_ack(dir: &Path, at: u64) -> io::Result<()> {
    let tmp = dir.join("draining.ack.tmp");
    let mut f = OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&tmp)?;
    f.write_all(format!("{at}\n").as_bytes())?;
    f.sync_all()?;
    fs::rename(&tmp, dir.join(ACK))?;
    sync_dir(dir)
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;

    /// A spool directory, with its key file beside it as the encrypted volume would hold it;
    /// removed when dropped.
    struct Dir {
        root: PathBuf,
        spool: PathBuf,
        key: PathBuf,
        public: PublicKey,
        secret: String,
    }
    impl Dir {
        fn new(name: &str) -> Self {
            static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = N.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!("tw-spool-{name}-{}-{n}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            let secret = StaticSecret::random_from_rng(rand_core::OsRng);
            let public = PublicKey::from(&secret);
            let d = Self {
                spool: root.join("spool"),
                key: root.join("volume-key"),
                root,
                public,
                secret: B64.encode(secret.to_bytes()),
            };
            d.unlock();
            d
        }
        fn open(&self) -> Spool {
            Spool::open(&self.spool, self.public, &self.key).unwrap()
        }
        fn unlock(&self) {
            fs::write(&self.key, &self.secret).unwrap();
        }
        fn lock(&self) {
            fs::remove_file(&self.key).unwrap();
        }
        fn file(&self, name: &str) -> PathBuf {
            self.spool.join(name)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn row(i: u64) -> Row {
        serde_json::json!({ "table": "action", "id": format!("row-{i}"), "n": i })
    }

    fn ns(rows: &[Row]) -> Vec<u64> {
        rows.iter().map(|r| r["n"].as_u64().unwrap()).collect()
    }

    /// The database, as the PG sink is: idempotent on each row's key (ON CONFLICT DO NOTHING),
    /// and refusing the next `fail` batches when told.
    #[derive(Default)]
    struct Db {
        table: Mutex<Vec<Row>>,
        keys: Mutex<HashSet<String>>,
        puts: Mutex<Vec<Vec<u64>>>,
        fail: std::sync::atomic::AtomicUsize,
    }
    impl Sink for Db {
        fn put(&self, rows: &[Row]) -> Result<(), String> {
            if self.fail.load(Ordering::Relaxed) > 0 {
                self.fail.fetch_sub(1, Ordering::Relaxed);
                return Err("the database is down".into());
            }
            self.puts.lock().unwrap().push(ns(rows));
            let mut keys = self.keys.lock().unwrap();
            for r in rows {
                if keys.insert(r["id"].as_str().unwrap().to_string()) {
                    self.table.lock().unwrap().push(r.clone());
                }
            }
            Ok(())
        }
    }
    impl Db {
        fn ns(&self) -> Vec<u64> {
            ns(&self.table.lock().unwrap())
        }
    }

    #[test]
    fn a_spooled_line_holds_none_of_the_row_and_opens_only_in_the_drain() {
        let d = Dir::new("sealed");
        let spool = d.open();
        let body = "a known body string, which only the database may read";
        spool.append(&serde_json::json!({ "table": "action", "id": "a-1", "n": 0, "args": { "text": body } })).unwrap();
        let disk = fs::read(d.file(LIVE)).unwrap();
        for needle in [body.as_bytes(), b"action", b"a-1", b"args"] {
            assert!(!disk.windows(needle.len()).any(|w| w == needle), "{:?} on disk", String::from_utf8_lossy(needle));
        }
        assert_eq!(disk.iter().filter(|&&b| b == b'\n').count(), 1, "one sealed record per line");
        let db = Db::default();
        assert_eq!(spool.drain(&db).unwrap().taken, 1);
        assert_eq!(db.table.lock().unwrap()[0]["args"]["text"], body, "the round trip");
        assert!(!spool.pending());
    }

    #[test]
    fn keygen_writes_a_private_key_the_spool_opens_with_and_never_replaces_one() {
        let d = Dir::new("keygen");
        let path = d.root.join("tw-spool.key");
        let public = public_key(&keygen(&path).unwrap()).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o400);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.ends_with('\n') && !text.trim_end().contains('='), "base64url, no padding, one line");
        let before = text.clone();
        assert_eq!(keygen(&path).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&path).unwrap(), before, "the key there is kept");
        let spool = Spool::open(&d.spool, public, &path).unwrap();
        spool.append(&row(0)).unwrap();
        let db = Db::default();
        assert_eq!(spool.drain(&db).unwrap().taken, 1, "a pair that works");
    }

    #[test]
    fn a_row_with_no_known_table_is_refused_at_append() {
        let d = Dir::new("table");
        let spool = d.open();
        for bad in [
            serde_json::json!({ "id": "x" }),
            serde_json::json!({ "table": "click", "id": "x" }),
            serde_json::json!({ "table": "users", "id": "x" }),
        ] {
            assert_eq!(spool.append(&bad).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        }
        assert!(!spool.pending(), "nothing written");
    }

    #[test]
    fn rows_come_out_in_order_across_a_rotation_and_the_files_are_private() {
        let d = Dir::new("order");
        let spool = Arc::new(d.open());
        for i in 0..5 {
            spool.append(&row(i)).unwrap();
        }
        assert!(spool.pending());
        // Rows appended while a drain holds the draining file go to a fresh live file, and come
        // out after it.
        struct AppendsDuring(Arc<Spool>, Db);
        impl Sink for AppendsDuring {
            fn put(&self, rows: &[Row]) -> Result<(), String> {
                if self.1.table.lock().unwrap().is_empty() {
                    for i in 5..10 {
                        self.0.append(&row(i)).unwrap();
                    }
                }
                self.1.put(rows)
            }
        }
        let sink = AppendsDuring(spool.clone(), Db::default());
        assert_eq!(spool.drain(&sink).unwrap().taken, 5, "one pass: the file rotated out");
        assert!(spool.pending(), "the rows appended during the drain are waiting");
        assert_eq!(spool.drain(&sink).unwrap().taken, 5);
        assert_eq!(sink.1.ns(), (0..10).collect::<Vec<_>>());
        assert!(!spool.pending());
        assert_eq!(fs::metadata(&d.spool).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(d.file(LIVE)).unwrap().permissions().mode() & 0o777, 0o600);
    }

    /// Acknowledged: the sink returned Ok for the batch. Batches of 3 over 10 rows; the second
    /// batch is refused. The first stays taken and is not handed over again, by a retry in this
    /// process or after a restart; nothing unacknowledged is lost.
    #[test]
    fn a_sink_failing_mid_drain_loses_nothing_and_repeats_no_acknowledged_batch() {
        let d = Dir::new("fail");
        let db = Db::default();
        {
            let spool = d.open().with_batch(3);
            for i in 0..10 {
                spool.append(&row(i)).unwrap();
            }
            struct FailSecond<'a>(&'a Db);
            impl Sink for FailSecond<'_> {
                fn put(&self, rows: &[Row]) -> Result<(), String> {
                    if self.0.puts.lock().unwrap().len() == 1 {
                        return Err("the database is down".into());
                    }
                    self.0.put(rows)
                }
            }
            match spool.drain(&FailSecond(&db)) {
                Err(DrainError::Sink(e)) => assert_eq!(e, "the database is down"),
                other => panic!("the refusal is the sink's: {other:?}"),
            }
            assert_eq!(db.ns(), vec![0, 1, 2]);
            db.fail.store(1, Ordering::Relaxed);
            assert!(spool.drain(&db).is_err());
            assert_eq!(db.ns(), vec![0, 1, 2], "a refused retry takes nothing");
        }
        let spool = d.open().with_batch(3);
        assert_eq!(spool.drain(&db).unwrap().taken, 7, "a restart resumes after the acknowledged batch");
        assert_eq!(db.ns(), (0..10).collect::<Vec<_>>(), "each row once, in order");
        assert_eq!(db.puts.lock().unwrap()[0], vec![0, 1, 2]);
        assert!(db.puts.lock().unwrap()[1..].iter().all(|b| !b.contains(&0)), "the first batch, never again");
        assert!(!spool.pending());
    }

    /// A crash after the sink took a batch but before its offset was fsync'd: the batch is
    /// handed over again, once, and the idempotent sink keeps each row once.
    #[test]
    fn a_batch_redelivered_after_a_crash_mid_acknowledgement_is_absorbed_by_the_idempotent_sink() {
        let d = Dir::new("redeliver");
        let db = Db::default();
        {
            let spool = d.open().with_batch(3);
            for i in 0..6 {
                spool.append(&row(i)).unwrap();
            }
            struct FailSecond<'a>(&'a Db);
            impl Sink for FailSecond<'_> {
                fn put(&self, rows: &[Row]) -> Result<(), String> {
                    if self.0.puts.lock().unwrap().len() == 1 {
                        return Err("the database is down".into());
                    }
                    self.0.put(rows)
                }
            }
            assert!(spool.drain(&FailSecond(&db)).is_err());
        }
        // The first batch's acknowledgement never reached the disk.
        fs::remove_file(d.file(ACK)).unwrap();
        let spool = d.open().with_batch(3);
        assert_eq!(spool.drain(&db).unwrap().taken, 6);
        assert_eq!(*db.puts.lock().unwrap(), vec![vec![0, 1, 2], vec![0, 1, 2], vec![3, 4, 5]]);
        assert_eq!(db.ns(), (0..6).collect::<Vec<_>>(), "each row once");
    }

    #[test]
    fn a_torn_last_line_is_skipped_and_named_and_what_follows_it_is_kept() {
        let d = Dir::new("torn");
        {
            let spool = d.open();
            for i in 0..3 {
                spool.append(&row(i)).unwrap();
            }
        }
        // A crash mid-append: part of a record, no newline.
        let whole = fs::metadata(d.file(LIVE)).unwrap().len();
        OpenOptions::new().append(true).open(d.file(LIVE)).unwrap().write_all(b"AAAAAAAAAAAAAAAAAAAAAA").unwrap();
        let spool = d.open();
        assert_eq!(
            spool.torn(),
            &[Skipped { file: LIVE, at: whole, bytes: 22, why: "a torn last line" }],
            "named by file and offset"
        );
        spool.append(&row(4)).unwrap();
        let db = Db::default();
        let drained = spool.drain(&db).unwrap();
        assert_eq!(db.ns(), vec![0, 1, 2, 4], "the torn row is not taken; the next one is");
        assert!(drained.skipped.is_empty(), "cut at open, so the drain sees whole lines");
    }

    /// While the volume is shut the key file is absent: the drain touches nothing, and appends
    /// go on. Unlocked, everything drains.
    #[test]
    fn with_the_key_file_absent_rows_are_kept_not_lost() {
        let d = Dir::new("locked");
        let spool = d.open();
        spool.append(&row(0)).unwrap();
        d.lock();
        let db = Db::default();
        match spool.drain(&db) {
            Err(DrainError::Locked(p)) => assert_eq!(p, d.key),
            other => panic!("locked: {other:?}"),
        }
        spool.append(&row(1)).unwrap();
        assert!(spool.pending() && !d.file(DRAINING).exists(), "nothing rotated or removed");
        assert!(db.table.lock().unwrap().is_empty());
        d.unlock();
        assert_eq!(spool.drain(&db).unwrap().taken, 2);
        assert_eq!(db.ns(), vec![0, 1]);
    }

    #[test]
    fn a_key_file_that_is_not_the_spools_key_is_refused_and_rows_are_kept() {
        let d = Dir::new("wrongkey");
        let spool = d.open();
        spool.append(&row(0)).unwrap();
        fs::write(&d.key, B64.encode(StaticSecret::random_from_rng(rand_core::OsRng).to_bytes())).unwrap();
        match spool.drain(&Db::default()) {
            Err(DrainError::Key(e)) => assert_eq!(e, "its public half is not DOOR_TW_SPOOL_PUB"),
            other => panic!("refused: {other:?}"),
        }
        assert!(spool.pending() && !d.file(ASIDE).exists(), "nothing set aside for a wrong key file");
    }

    /// A row sealed to another key (a key since rotated) does not open: it is set aside, sealed
    /// and as written, and named; the rows around it are delivered.
    #[test]
    fn a_row_that_will_not_open_is_set_aside_and_named_never_dropped() {
        let d = Dir::new("aside");
        let spool = d.open();
        spool.append(&row(0)).unwrap();
        let at = fs::metadata(d.file(LIVE)).unwrap().len();
        let other = PublicKey::from(&StaticSecret::random_from_rng(rand_core::OsRng));
        let stranger = format!("{}\n", seal(&other, &serde_json::to_vec(&row(1)).unwrap()).unwrap());
        OpenOptions::new().append(true).open(d.file(LIVE)).unwrap().write_all(stranger.as_bytes()).unwrap();
        spool.append(&row(2)).unwrap();
        let db = Db::default();
        let drained = spool.drain(&db).unwrap();
        assert_eq!(db.ns(), vec![0, 2]);
        assert_eq!(
            drained.skipped,
            vec![Skipped { file: DRAINING, at, bytes: stranger.len() as u64, why: "does not open with the key" }]
        );
        assert_eq!(fs::read_to_string(d.file(ASIDE)).unwrap(), stranger, "kept, as written");
    }

    /// A restart with a draining file left (its drain refused) and a live file behind it: the
    /// draining file goes first, whole, then the live one.
    #[test]
    fn a_restart_drains_a_leftover_draining_file_first() {
        let d = Dir::new("restart");
        {
            let spool = d.open();
            for i in 0..3 {
                spool.append(&row(i)).unwrap();
            }
            let down = Db::default();
            down.fail.store(1, Ordering::Relaxed);
            assert!(spool.drain(&down).is_err());
            for i in 3..6 {
                spool.append(&row(i)).unwrap();
            }
        }
        assert!(d.file(DRAINING).exists() && d.file(LIVE).exists(), "both files on disk");
        let spool = d.open();
        let db = Db::default();
        assert_eq!(spool.drain(&db).unwrap().taken, 6);
        assert_eq!(db.ns(), (0..6).collect::<Vec<_>>());
        assert!(!spool.pending());
    }

    /// 10k rows appended from one thread while another drains, the database refusing every
    /// fifth batch: every row arrives once, in order.
    #[test]
    fn ten_thousand_rows_appended_while_draining_arrive_once_in_order() {
        let d = Dir::new("10k");
        let spool = Arc::new(d.open().with_batch(64));
        struct Flaky(Db, std::sync::atomic::AtomicUsize);
        impl Sink for Flaky {
            fn put(&self, rows: &[Row]) -> Result<(), String> {
                if self.1.fetch_add(1, Ordering::Relaxed) % 5 == 4 {
                    return Err("the database is down".into());
                }
                self.0.put(rows)
            }
        }
        let sink = Arc::new(Flaky(Db::default(), Default::default()));
        let done = Arc::new(AtomicBool::new(false));
        let drainer = {
            let (spool, sink, done) = (spool.clone(), sink.clone(), done.clone());
            std::thread::spawn(move || {
                while !done.load(Ordering::Relaxed) {
                    let _ = spool.drain(sink.as_ref());
                }
            })
        };
        for i in 0..10_000 {
            spool.append(&row(i)).unwrap();
        }
        done.store(true, Ordering::Relaxed);
        drainer.join().unwrap();
        spool.drain_until_empty(sink.as_ref(), &AtomicBool::new(false));
        assert_eq!(sink.0.ns(), (0..10_000).collect::<Vec<_>>());
        let delivered: usize = sink.0.puts.lock().unwrap().iter().map(Vec::len).sum();
        assert_eq!(delivered, 10_000, "no batch handed over twice");
        assert!(!spool.pending());
    }
}
