//! Pluggable storage — the R2 backend, driven through the real relay.
//!
//! ## WHAT IS REAL HERE AND WHAT IS A STAND-IN
//!
//! Real: `relay::offload::OffloadMailbox`, `relay::blobs::R2Blobs`, the SigV4
//! presigner, the `reqwest` client, a TCP socket, the SQLite index, and the relay
//! itself speaking `Frame` over a WebSocket. Every byte of the backend under test
//! is the code that would run in production, and the HTTP it speaks is the HTTP it
//! would send to Cloudflare.
//!
//! A STAND-IN: [`StandIn`], the S3-compatible server at the other end. It is
//! **not R2** and it announces itself on stderr when it boots. It stores objects in
//! a `HashMap`, and it deliberately **does not verify signatures** — verifying
//! SigV4 would be a second implementation of `sigv4.rs`, which is the "test fixture
//! that restates the rule" error, and it would agree with itself rather than with
//! AWS. `sigv4.rs`'s own known-answer test against AWS's published vector is what
//! proves the signature; this server only checks that a request arrived PRESIGNED
//! at all.
//!
//! So what these tests prove is the backend's own behaviour — ordering, atomicity,
//! key derivation, error propagation, what is acked and what is not. What they do
//! NOT prove is anything about R2's own semantics. Nothing in this file should ever
//! be quoted as evidence that R2 behaves a particular way.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, Stream, StreamExt};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::{Error as WsErr, Message};

use relay::blobs::{body_key, Blobs, R2Blobs};
use relay::mailbox::{Appended, Mailbox, StoreError};
use relay::offload::OffloadMailbox;
use relay::sigv4::Credentials;
use relay::store::Store;
use relay::wire::Frame;
use relay::Retention;

/// Every tag in these tests is a REAL ADDRESS: the relay refuses a publish its tag
/// did not sign (pacific_wire::address). A short name stands for the address of a
/// seed spelled by that name, so the tests still read "aa", "bb".
fn addr(name: &str) -> relay::wire::address::Address {
    let mut seed = [0u8; 32];
    seed[..name.len()].copy_from_slice(name.as_bytes());
    relay::wire::address::Address::from_seed(&seed)
}

/// The tag `name` stands for, as the wire spells it.
fn t(name: &str) -> String {
    addr(name).tag_hex()
}

/// A `Pub` of `blob` to `name`'s address, signed by that address's key.
#[allow(dead_code)]
fn publish(name: &str, blob: &str, commit: bool) -> Frame {
    addr(name).pub_frame(blob.into(), commit)
}


// ---------------------------------------------------------------------------
// The stand-in object store. NOT R2. See the module doc.
// ---------------------------------------------------------------------------

/// An S3-compatible endpoint good enough to exercise a real S3 client: PUT, GET,
/// DELETE over HTTP/1.1, objects in a map. It says what it is at boot.
struct StandIn {
    objects: Mutex<HashMap<String, Vec<u8>>>,
    /// When set, every PUT answers 500. The failure a relay must never Ack.
    fail_puts: AtomicBool,
    /// When set, every DELETE answers 500 — the leak path in `evict`.
    fail_deletes: AtomicBool,
    /// Requests that arrived without a signature at all.
    unsigned: Mutex<u64>,
}

impl StandIn {
    async fn start() -> (Arc<StandIn>, String) {
        let state = Arc::new(StandIn {
            objects: Mutex::new(HashMap::new()),
            fail_puts: AtomicBool::new(false),
            fail_deletes: AtomicBool::new(false),
            unsigned: Mutex::new(0),
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stand-in");
        let addr = listener.local_addr().unwrap();
        eprintln!(
            "STAND-IN: an in-process S3-compatible object store is listening on {addr}. \
             It is NOT Cloudflare R2 and it does not verify signatures."
        );
        let s = state.clone();
        tokio::spawn(async move {
            loop {
                let Ok((sock, _)) = listener.accept().await else {
                    continue;
                };
                let s = s.clone();
                tokio::spawn(async move {
                    let _ = handle(sock, s).await;
                });
            }
        });
        (state, format!("http://{addr}"))
    }

    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.objects.lock().unwrap().get(key).cloned()
    }

    fn len(&self) -> usize {
        self.objects.lock().unwrap().len()
    }

    fn keys(&self) -> Vec<String> {
        let mut k: Vec<String> = self.objects.lock().unwrap().keys().cloned().collect();
        k.sort();
        k
    }

    fn wipe(&self) {
        self.objects.lock().unwrap().clear();
    }
}

async fn handle(mut sock: TcpStream, state: Arc<StandIn>) -> std::io::Result<()> {
    let (r, mut w) = sock.split();
    let mut reader = BufReader::new(r);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await? == 0 {
        return Ok(());
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();

    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).await? == 0 {
            break;
        }
        if header.trim().is_empty() {
            break;
        }
        let lower = header.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).await?;
    }

    let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
    // A SHAPE check, not a verification: this server has no business holding a
    // second copy of the signing rule. It only refuses a request that was never
    // presigned at all, which would mean the backend forgot to sign.
    if !query.contains("X-Amz-Signature=") {
        *state.unsigned.lock().unwrap() += 1;
        return reply(&mut w, "403 Forbidden", b"").await;
    }
    // `/{bucket}/{key}` — the key is the last segment.
    let key = path.rsplit('/').next().unwrap_or("").to_string();

    match method.as_str() {
        "PUT" => {
            if state.fail_puts.load(Ordering::Relaxed) {
                return reply(&mut w, "500 Internal Server Error", b"").await;
            }
            state.objects.lock().unwrap().insert(key, body);
            reply(&mut w, "200 OK", b"").await
        }
        "GET" => match state.get(&key) {
            Some(b) => reply(&mut w, "200 OK", &b).await,
            None => reply(&mut w, "404 Not Found", b"").await,
        },
        "DELETE" => {
            if state.fail_deletes.load(Ordering::Relaxed) {
                return reply(&mut w, "500 Internal Server Error", b"").await;
            }
            state.objects.lock().unwrap().remove(&key);
            reply(&mut w, "204 No Content", b"").await
        }
        _ => reply(&mut w, "405 Method Not Allowed", b"").await,
    }
}

async fn reply(
    w: &mut (impl AsyncWriteExt + Unpin),
    status: &str,
    body: &[u8],
) -> std::io::Result<()> {
    // 204 carries no body and therefore no content-length.
    let head = if status.starts_with("204") {
        "HTTP/1.1 204 No Content\r\nconnection: close\r\n\r\n".to_string()
    } else {
        format!(
            "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        )
    };
    w.write_all(head.as_bytes()).await?;
    if !body.is_empty() {
        w.write_all(body).await?;
    }
    w.flush().await
}

// ---------------------------------------------------------------------------
// Rigging
// ---------------------------------------------------------------------------

fn creds() -> Credentials {
    Credentials {
        access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
        secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
    }
}

fn r2(endpoint: &str) -> Box<dyn Blobs> {
    Box::new(R2Blobs::from_endpoint(endpoint, "mailbox", "auto", creds()).expect("blob store"))
}

/// A unique scratch path for the index. No tempfile dev-dep for one string.
fn scratch(name: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir()
        .join(format!("relay-{name}-{}-{nanos}.db", std::process::id()))
        .to_string_lossy()
        .into_owned()
}

fn cleanup(path: &str) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{path}{suffix}"));
    }
}

fn forever() -> Retention {
    Retention {
        window_us: 0,
        max_per_tag: 0,
        sweep: Duration::from_secs(3600),
    }
}

/// Boot a relay whose bodies live in the stand-in and whose index lives at `path`.
async fn boot(path: &str, endpoint: &str, retention: Retention) -> (String, tokio::task::JoinHandle<()>) {
    let index = Store::open(path).expect("open index");
    let mailbox = OffloadMailbox::new(index, r2(endpoint));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let h = tokio::spawn(relay::serve_with_store(listener, mailbox, retention, None));
    (format!("ws://{addr}"), h)
}

async fn send(ws: &mut (impl SinkExt<Message, Error = WsErr> + Unpin), f: Frame) {
    ws.send(Message::Text(f.to_json())).await.expect("send frame");
}

async fn recv<S: Stream<Item = Result<Message, WsErr>> + Unpin>(ws: &mut S) -> Frame {
    loop {
        match ws.next().await.expect("stream open").expect("ws ok") {
            Message::Text(t) => return Frame::from_json(&t.to_string()).expect("parse frame"),
            Message::Close(_) => panic!("closed before a frame arrived"),
            _ => continue,
        }
    }
}

async fn ack(ws: &mut (impl SinkExt<Message, Error = WsErr> + Stream<Item = Result<Message, WsErr>> + Unpin), tag: &str, blob: &str, commit: bool) -> (u64, bool) {
    send(ws, publish(tag, blob, commit)).await;
    match recv(ws).await {
        Frame::Ack { seq, ok, .. } => (seq, ok),
        other => panic!("expected an Ack, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

/// THE RULING, demonstrated: the arc authors the blob to object storage. The index
/// keeps a 64-hex key where the body used to be, and the bytes are in the bucket.
#[tokio::test]
async fn the_body_goes_to_object_storage_and_the_index_keeps_only_a_key() {
    let (objects, endpoint) = StandIn::start().await;
    let path = scratch("offload-body");
    let (url, _relay) = boot(&path, &endpoint, forever()).await;

    let (mut publisher, _) = connect_async(&url).await.unwrap();
    let (seq, ok) = ack(&mut publisher, "aa", "aGk", false).await;
    assert!(ok, "a publish whose body reached object storage is acked");

    // The index holds the KEY, not the blob. Read it with a second `Store` on the
    // same file — the public API, and the same rows the relay wrote.
    let rows = Store::open(&path).unwrap().replay(&t("aa"), 0).unwrap();
    assert_eq!(rows.len(), 1);
    let (row_seq, stored) = &rows[0];
    assert_eq!(*row_seq, seq);
    assert_eq!(
        stored,
        &body_key(&t("aa"), seq),
        "the index row must carry the derived object key"
    );
    assert_ne!(stored, "aGk", "the blob body must NOT be in the index");

    // And the bytes are in the bucket, under exactly that key.
    assert_eq!(objects.len(), 1);
    assert_eq!(objects.keys(), vec![body_key(&t("aa"), seq)]);
    assert_eq!(objects.get(&body_key(&t("aa"), seq)).unwrap(), b"aGk".to_vec());
    assert_eq!(*objects.unsigned.lock().unwrap(), 0, "every request was presigned");

    cleanup(&path);
}

/// The whole point of durability, on the new backend: a subscriber that was offline
/// across a redeploy drains the blob, and the body comes back from the object store
/// through the relay's ordinary replay.
#[tokio::test]
async fn an_offloaded_blob_replays_after_a_restart() {
    let (_objects, endpoint) = StandIn::start().await;
    let path = scratch("offload-restart");

    let (url, relay1) = boot(&path, &endpoint, forever()).await;
    let (mut publisher, _) = connect_async(&url).await.unwrap();
    let (seq, ok) = ack(&mut publisher, "aa", "aGk", false).await;
    assert!(ok);

    drop(publisher);
    relay1.abort();
    let (url2, _relay2) = boot(&path, &endpoint, forever()).await;

    let (mut late, _) = connect_async(&url2).await.unwrap();
    send(&mut late, Frame::Sub { tags: vec![t("aa")], since: 0, v: 0 }).await;
    match recv(&mut late).await {
        Frame::Msg { tag, seq: got, blob } => {
            assert_eq!(tag, t("aa"));
            assert_eq!(got, seq, "the seq the publisher was acked");
            assert_eq!(blob, "aGk", "the body came back from object storage");
        }
        other => panic!("expected the pre-restart blob, got {other:?}"),
    }
    assert_eq!(recv(&mut late).await, Frame::Eose);

    cleanup(&path);
}

/// THE PROPERTY R2 CANNOT HOLD ON ITS OWN, holding — because the slot never left
/// SQLite. First-writer-wins per tag, the loser stores nothing, and a restart does
/// not re-open the slot. If this ever goes red on this backend, the arbitration has
/// been moved into object storage and the group can fork.
#[tokio::test]
async fn the_commit_slot_still_arbitrates_on_the_offload_backend() {
    let (objects, endpoint) = StandIn::start().await;
    let path = scratch("offload-slot");

    let (url, relay1) = boot(&path, &endpoint, forever()).await;
    let (mut a, _) = connect_async(&url).await.unwrap();
    let (_, won) = ack(&mut a, "bb", "Y29tbWl0", true).await;
    assert!(won, "the first commit wins the slot");

    let (mut b, _) = connect_async(&url).await.unwrap();
    let (_, lost) = ack(&mut b, "bb", "cml2YWw", true).await;
    assert!(!lost, "the second commit for the same tag must lose");
    assert_eq!(objects.len(), 1, "and the loser's body must not be left behind");

    // Across a restart, which is where the pre-durability relay forked groups.
    drop(a);
    drop(b);
    relay1.abort();
    let (url2, _relay2) = boot(&path, &endpoint, forever()).await;
    let (mut c, _) = connect_async(&url2).await.unwrap();
    let (_, after_restart) = ack(&mut c, "bb", "cml2YWwy", true).await;
    assert!(
        !after_restart,
        "a restart must NOT re-open a claimed commit slot — that is a group fork"
    );

    // Only the winner is in the mailbox.
    send(&mut c, Frame::Sub { tags: vec![t("bb")], since: 0, v: 0 }).await;
    let mut bodies = Vec::new();
    loop {
        match recv(&mut c).await {
            Frame::Msg { blob, .. } => bodies.push(blob),
            Frame::Eose => break,
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(bodies, vec!["Y29tbWl0".to_string()]);
    assert_eq!(objects.len(), 1, "no orphan bodies accumulated across the races");

    cleanup(&path);
}

/// BODY FIRST, INDEX SECOND. If object storage refuses the body, the publish is
/// NOT acked and the index never learns the blob existed — the alternative is an
/// index row that replays as nothing and a publisher that was told it was fine.
#[tokio::test]
async fn a_body_that_did_not_reach_object_storage_is_never_acked() {
    let (objects, endpoint) = StandIn::start().await;
    let path = scratch("offload-refuse");
    let (url, _relay) = boot(&path, &endpoint, forever()).await;

    objects.fail_puts.store(true, Ordering::Relaxed);
    let (mut publisher, _) = connect_async(&url).await.unwrap();
    let (_, ok) = ack(&mut publisher, "aa", "aGk", false).await;
    assert!(!ok, "a publish whose body was refused must not be acked ok");

    assert_eq!(objects.len(), 0);
    assert!(
        Store::open(&path).unwrap().replay(&t("aa"), 0).unwrap().is_empty(),
        "no index row may exist for a body that was never stored"
    );

    // And the relay is still usable once the store comes back.
    objects.fail_puts.store(false, Ordering::Relaxed);
    let (_, ok) = ack(&mut publisher, "aa", "aGk", false).await;
    assert!(ok, "the refusal was transient, not fatal to the connection");

    cleanup(&path);
}

/// A commit that lost the race wrote its body before it found out. Nothing
/// references that object, so it is garbage by the module's own rule — and the
/// backend cleans it up while it still knows the name.
#[tokio::test]
async fn a_lost_commit_race_does_not_leave_an_orphan_object() {
    let (objects, endpoint) = StandIn::start().await;
    let path = scratch("offload-orphan");
    let index = Store::open(&path).unwrap();
    let mailbox = OffloadMailbox::new(index, r2(&endpoint));

    assert_eq!(mailbox.append("bb", 10, "winner", true).await.unwrap(), Appended::Stored);
    assert_eq!(mailbox.append("bb", 20, "loser", true).await.unwrap(), Appended::SlotTaken);

    assert_eq!(objects.keys(), vec![body_key("bb", 10)]);
    assert!(
        objects.get(&body_key("bb", 20)).is_none(),
        "the loser's body must not survive as unreferenced garbage"
    );

    cleanup(&path);
}

/// INDEX FIRST on the way out. Eviction drops the row and then the object, so the
/// bucket does not keep paying for blobs the mailbox has forgotten — and the floor
/// the index raised is still there to announce the hole.
#[tokio::test]
async fn eviction_removes_the_object_as_well_as_the_row() {
    let (objects, endpoint) = StandIn::start().await;
    let path = scratch("offload-evict");
    let index = Store::open(&path).unwrap();
    let mailbox = OffloadMailbox::new(index, r2(&endpoint));

    mailbox.append("cc", 10, "old", false).await.unwrap();
    mailbox.append("cc", 20, "new", false).await.unwrap();
    assert_eq!(objects.len(), 2);

    // Cap of one: the older blob goes.
    assert_eq!(mailbox.evict(0, 0, 1).await.unwrap(), 1);
    assert_eq!(objects.keys(), vec![body_key("cc", 20)], "the evicted object is gone");
    assert_eq!(
        mailbox.floors(&["cc".to_string()]).await.unwrap()["cc"],
        10,
        "and the floor still names the hole, so a Gap can be announced"
    );

    cleanup(&path);
}

/// The leak, named rather than hidden. If the object store refuses the DELETE, the
/// index row is ALREADY gone — that ordering is deliberate — so the object becomes
/// unreferenced garbage. Eviction still reports the rows it removed, because it did
/// remove them; what it must not do is pretend nothing was left behind, and
/// `offload.rs` logs a count for exactly that.
#[tokio::test]
async fn a_delete_that_fails_leaks_the_object_and_still_drops_the_row() {
    let (objects, endpoint) = StandIn::start().await;
    let path = scratch("offload-leak");
    let index = Store::open(&path).unwrap();
    let mailbox = OffloadMailbox::new(index, r2(&endpoint));

    mailbox.append("cc", 10, "old", false).await.unwrap();
    mailbox.append("cc", 20, "new", false).await.unwrap();
    objects.fail_deletes.store(true, Ordering::Relaxed);

    assert_eq!(
        mailbox.evict(0, 0, 1).await.unwrap(),
        1,
        "the row count is what was removed from the index, and it was removed"
    );
    assert_eq!(objects.len(), 2, "the object leaked — this is the documented cost");
    assert_eq!(
        mailbox.replay("cc", 0).await.unwrap().len(),
        1,
        "and the mailbox is consistent: the leaked object is unreferenced, not a hole"
    );

    cleanup(&path);
}

/// An index row whose object has vanished is CORRUPTION and is reported as such.
///
/// The temptation is to skip the row and return the rest, which is exactly the
/// silent short replay the mailbox exists to prevent: the subscriber cannot tell a
/// complete backlog from one with a hole in it. So `replay` fails instead.
#[tokio::test]
async fn a_missing_body_is_an_error_not_a_short_replay() {
    let (objects, endpoint) = StandIn::start().await;
    let path = scratch("offload-missing");
    let index = Store::open(&path).unwrap();
    let mailbox = OffloadMailbox::new(index, r2(&endpoint));

    mailbox.append("aa", 10, "one", false).await.unwrap();
    mailbox.append("aa", 20, "two", false).await.unwrap();
    assert_eq!(mailbox.replay("aa", 0).await.unwrap().len(), 2);

    // Something removed the objects out of band — a lifecycle rule, a console, a
    // bucket restored from an older copy.
    objects.wipe();

    match mailbox.replay("aa", 0).await {
        Err(StoreError::BodyMissing) => {}
        Err(other) => panic!("expected BodyMissing, got {other}"),
        Ok(rows) => panic!("a missing body was rounded down to a {}-row backlog", rows.len()),
    }

    cleanup(&path);
}

/// A transport failure and a missing object are different events and must not be
/// collapsed: one is retryable, the other is a hole. The error the relay logs has
/// to be able to tell an operator which of the two happened.
#[tokio::test]
async fn an_unreachable_object_store_reads_differently_from_a_missing_object() {
    let path = scratch("offload-unreachable");
    let index = Store::open(&path).unwrap();
    // A port nothing is listening on — the connect fails for real.
    let mailbox = OffloadMailbox::new(index, r2("http://127.0.0.1:1"));

    match mailbox.append("aa", 10, "one", false).await {
        Err(StoreError::Body(msg)) => assert!(
            msg.contains("unreachable") || msg.contains("failed") || msg.contains("timed out"),
            "unhelpful transport error: {msg}"
        ),
        other => panic!("expected a Body error, got {other:?}"),
    }
    assert!(
        Store::open(&path).unwrap().replay("aa", 0).unwrap().is_empty(),
        "an unreachable store must not leave an index row behind"
    );

    cleanup(&path);
}

/// The boot-time refusal, on the REAL binary: object-storage bodies with an
/// in-memory index would make the blobs durable and the COMMIT SLOTS not, which is
/// a fork wearing the costume of a more durable deployment. It must not start.
#[tokio::test]
async fn the_binary_refuses_object_storage_bodies_with_no_durable_index() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_semaphore"))
        .env("RELAY_BIND", "127.0.0.1:0")
        .env("RELAY_MAILBOX_BLOBS", "r2")
        .env_remove("RELAY_STORE")
        .env("RELAY_R2_ENDPOINT", "https://example.invalid")
        .env("RELAY_R2_BUCKET", "mailbox")
        .env("RELAY_R2_ACCESS_KEY_ID", "x")
        .env("RELAY_R2_SECRET_ACCESS_KEY", "y")
        .output()
        .expect("run semaphore");
    assert!(!out.status.success(), "it started, and it must not have");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("RELAY_STORE"),
        "the refusal must name what is missing: {stderr}"
    );
}
