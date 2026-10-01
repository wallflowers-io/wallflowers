//! The Door's security requirements, tested from outside: the image binary is started on
//! loopback with a throwaway root, and spoken to over HTTP as a browser or an attacker would.
//! Each test names the srs.md row it verifies (srr/vv.md § The Door).
//!
//! Through a TLS edge: `DOOR_EDGE=caddy` (and `CADDY`, the binary, if it is not on PATH)
//! fronts every Door a test starts with its own Caddy, `tls internal` on a free port, its
//! own data directory, and `skip_install_trust`, so nothing touches the system's trust
//! store. The Door's public origin is then the edge's https URL, and the test client
//! trusts that Caddy's root alone. Headers an edge can rewrite (SEC-7, SEC-8, SEC-27) are
//! then checked as a browser behind the edge would see them (K-43).
//!
//! A test for a requirement not yet built fails by name until its piece lands (D-11, resolved
//! 26 Sep: red tests may be committed). Names carry the DT row they verify.

use std::net::TcpListener;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The origin the image is started with, as the webapp's would be.
const LISTED: &str = "http://127.0.0.1:18231";
/// An origin nobody listed.
const STRANGER: &str = "https://attacker.example";

/// One image process on a free loopback port, killed and its root removed on drop.
struct Door {
    child: Child,
    root: PathBuf,
    /// Its own seals (D-34 (c)), beside the root, so no test opens another's.
    seals: PathBuf,
    base: String,
    edge: Option<Edge>,
    log: PathBuf,
}

/// A Caddy in front of one Door, terminating TLS (DOOR_EDGE=caddy).
struct Edge {
    child: Child,
    ca: PathBuf,
    log: PathBuf,
}

impl Drop for Edge {
    fn drop(&mut self) {
        report("the Caddy edge", &mut self.child, &self.log);
        let _ = Command::new("kill").args(["-KILL", "--", &format!("-{}", self.child.id())]).status();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn edge_mode() -> bool {
    std::env::var("DOOR_EDGE").map(|v| v == "caddy").unwrap_or(false)
}

/// A name no other test in this process holds: the clock alone repeats within a
/// microsecond on macOS, and parallel tests then shared a root, the first to finish
/// deleting the other's (NC-74).
fn unique() -> String {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    format!("{}-{nanos}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// A fixture's stdout and stderr, kept beside its root, not in it: nothing a test
/// inspects holds them.
fn log_beside(root: &std::path::Path, suffix: &str) -> (PathBuf, Stdio, Stdio) {
    let path = root.with_extension(suffix);
    let f = std::fs::File::create(&path).expect("a fixture's log");
    let g = f.try_clone().expect("a fixture's log");
    (path, Stdio::from(f), Stdio::from(g))
}

/// When a test fails, why a fixture went away: its exit status and the end of its
/// log, which is kept. Otherwise the log is removed.
fn report(what: &str, child: &mut Child, log: &std::path::Path) {
    if !std::thread::panicking() {
        let _ = std::fs::remove_file(log);
        return;
    }
    let status = match child.try_wait() {
        Ok(Some(s)) => s.to_string(),
        _ => "still running".into(),
    };
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let tail = lines[lines.len().saturating_sub(20)..].join("\n");
    eprintln!("{what}: {status}; the end of {}:\n{tail}", log.display());
}

/// Wait for `port` on loopback, as long as a first launch may take.
fn wait_listening(port: u16, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "{what} did not listen on {port} within 120 s (on macOS, a first launch waits for the system's check of the binary)");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Start a Caddy terminating TLS on `edge_port` and proxying to the Door on `door_port`.
fn start_edge(root: &std::path::Path, edge_port: u16, door_port: u16) -> Edge {
    let dir = root.join("caddy");
    std::fs::create_dir_all(&dir).unwrap();
    let caddyfile = dir.join("Caddyfile");
    std::fs::write(&caddyfile, format!(
        "{{\n  admin off\n  local_certs\n  skip_install_trust\n  auto_https disable_redirects\n  storage file_system {data}\n}}\n\
         https://localhost:{edge_port} {{\n  tls internal\n  reverse_proxy 127.0.0.1:{door_port}\n}}\n",
        data = dir.join("data").display()
    )).unwrap();
    let caddy = std::env::var("CADDY").unwrap_or_else(|_| "caddy".into());
    let (log, out, err) = log_beside(root, "caddy.log");
    let child = Command::new(&caddy)
        .args(["run", "--config", &caddyfile.to_string_lossy(), "--adapter", "caddyfile"])
        .env("XDG_DATA_HOME", &dir)
        .env("XDG_CONFIG_HOME", &dir)
        .env("HOME", &dir)
        .stdout(out)
        .stderr(err)
        .process_group(0)
        .spawn()
        .unwrap_or_else(|e| panic!("DOOR_EDGE=caddy, but {caddy} would not start: {e}"));
    let ca = dir.join("data/pki/authorities/local/root.crt");
    let edge = Edge { child, ca, log };
    wait_listening(edge_port, "the Caddy edge");
    let deadline = Instant::now() + Duration::from_secs(120);
    while !edge.ca.exists() {
        assert!(Instant::now() < deadline, "the Caddy edge wrote no root at {} within 120 s", edge.ca.display());
        std::thread::sleep(Duration::from_millis(50));
    }
    wait_serving(&format!("https://localhost:{edge_port}"), &edge.ca);
    edge
}

/// A client that trusts one root alone: an edge's.
fn edge_client(ca: &std::path::Path) -> reqwest::Client {
    let pem = std::fs::read(ca).expect("the edge's root");
    reqwest::Client::builder()
        .tls_built_in_root_certs(false)
        .add_root_certificate(reqwest::Certificate::from_pem(&pem).expect("a PEM root"))
        .build()
        .expect("a client for the edge")
}

/// Wait for a handshake verified against the edge's root. Caddy listens before it
/// has issued its certificate, and until then answers `internal_error`; with many
/// starting at once, issuing takes seconds. Its own thread and runtime: the Door
/// starts inside a test's.
fn wait_serving(base: &str, ca: &std::path::Path) {
    let (url, ca) = (format!("{base}/v2/icd"), ca.to_path_buf());
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a runtime");
        rt.block_on(async {
            let http = edge_client(&ca);
            let deadline = Instant::now() + Duration::from_secs(120);
            loop {
                match http.get(&url).send().await {
                    Ok(_) => return,
                    Err(e) => assert!(Instant::now() < deadline, "the Caddy edge completed no verified handshake within 120 s: {url}: {e:?}"),
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
    })
    .join()
    .unwrap_or_else(|e| std::panic::resume_unwind(e));
}

/// Every Door of this suite: the fold cache on, under a test's model (the build is unstamped),
/// and every hit refolded and compared (FC-2).
const FOLD_CACHE_VERIFIED: [(&str, &str); 2] = [("PACIFIC_FOLD_CACHE_MODEL", "door-suite"), ("PACIFIC_FOLD_CACHE_VERIFY", "1")];

impl Door {
    fn start() -> Door {
        Door::start_with(&[])
    }

    fn start_with(extra: &[(&str, &str)]) -> Door {
        Door::start_closed(extra, &[])
    }

    /// As `start_with`, the image started with each of `closed` (of 0, 1 and 2) closed
    /// (NC-80).
    fn start_closed(extra: &[(&str, &str)], closed: &'static [i32]) -> Door {
        Door::launch(std::path::Path::new(env!("CARGO_BIN_EXE_door")), extra, closed)
    }

    /// The image at `bin`, a copy a test may replace while it runs (NC-80).
    fn launch(bin: &std::path::Path, extra: &[(&str, &str)], closed: &'static [i32]) -> Door {
        // THE PORT, TRIED UP TO THREE TIMES: free_port() lets its port go before the Door
        // binds it, and another test's process can take it in between. The Door then exits
        // on "bind the door", or the port answers for someone else; a Door is this one only
        // once its own line names the port.
        for attempt in 1..=3 {
            let port = free_port();
            let root = std::env::temp_dir().join(format!("door-security-{}", unique()));
            // Behind an edge the Door's public origin is the edge's, whatever a test asked for.
            let edge_port = edge_mode().then(free_port);
            let public = edge_port.map(|p| format!("https://localhost:{p}"));
            let seals = PathBuf::from(format!("{}-seals", root.display()));
            let mut cmd = Command::new(bin);
            cmd
                .env("DOOR_PORT", port.to_string())
                .env("DOOR_ROOT", &root)
                .env("DOOR_SEALS", &seals)
                .env("DOOR_ORIGIN", LISTED)
                // No relay: these tests never open an account process.
                .env("DOOR_RELAY", "ws://127.0.0.1:9/v1/relay")
                .env_remove("DOOR_RELAY_BIN")
                // A test's in-process Node sets this for itself; the Door sets its sessions' own.
                .env_remove("PACIFIC_STATE_DIR")
                // The fold cache on, every hit refolded and compared (O-69, FC-2); a test may
                // override either.
                .envs(FOLD_CACHE_VERIFIED)
                .envs(extra.iter().copied());
            if let Some(p) = &public {
                cmd.env("DOOR_PUBLIC", p);
            }
            let (log, out, err) = log_beside(&root, "log");
            if !closed.is_empty() {
                // In the child, after its stdio is set and before it execs the image.
                unsafe {
                    cmd.pre_exec(move || {
                        for &fd in closed {
                            libc::close(fd);
                        }
                        Ok(())
                    });
                }
            }
            let child = cmd
                .stdout(out)
                .stderr(err)
                // Its own process group, so that the account processes it spawns die with it:
                // the image does not take them down itself when it is killed.
                .process_group(0)
                .spawn()
                .expect("start the door image");
            let mut door = Door { child, root, seals, base: format!("http://127.0.0.1:{port}"), edge: None, log };
            // macOS checks a newly linked, unsigned binary on its first launch, and every
            // rebuild makes a new one: the process waits in dyld before `main` for that
            // check, not for the Door. Once checked it listens in a fraction of a second.
            let ours = format!("on 127.0.0.1:{port} ");
            // A Door started without its stdout names nothing: its listening port stands in.
            let silent = closed.contains(&1);
            let deadline = Instant::now() + Duration::from_secs(120);
            let exited = loop {
                let said = std::fs::read_to_string(&door.log).unwrap_or_default();
                if said.contains(&ours) || (silent && std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()) {
                    break None;
                }
                if let Ok(Some(st)) = door.child.try_wait() {
                    break Some((st, said));
                }
                assert!(Instant::now() < deadline, "the door image named no port within 120 s (on macOS, a first launch waits for the system's check of the binary)");
                std::thread::sleep(Duration::from_millis(50));
            };
            match exited {
                None => {}
                Some((_, said)) if said.contains("bind the door") && attempt < 3 => {
                    eprintln!("door-security: port {port} was taken before the Door bound it; attempt {} of 3", attempt + 1);
                    continue;
                }
                Some((st, said)) => panic!("the door image exited ({st}) before it named its port: {}", said.lines().last().unwrap_or_default()),
            }
            wait_listening(port, "the door image");
            if let (Some(edge_port), Some(public)) = (edge_port, public) {
                door.edge = Some(start_edge(&door.root, edge_port, port));
                door.base = public;
            }
            return door;
        }
        unreachable!("each of the three attempts returns, retries or panics")
    }

    /// An HTTP client for this Door: behind an edge, it trusts that edge's root alone.
    fn http(&self) -> reqwest::Client {
        match &self.edge {
            None => reqwest::Client::new(),
            Some(e) => edge_client(&e.ca),
        }
    }

    /// A route every version serves without credentials: the model (`icd`).
    async fn icd_route(&self, http: &reqwest::Client) -> String {
        for v in ["/v2/icd", "/v1/icd"] {
            let r = http.get(format!("{}{v}", self.base)).send().await.expect("reach the door");
            if r.status().is_success() {
                return v.to_string();
            }
        }
        panic!("the door serves neither /v2/icd nor /v1/icd");
    }
}

impl Drop for Door {
    fn drop(&mut self) {
        self.edge.take();
        report("the door image", &mut self.child, &self.log);
        let _ = Command::new("kill").args(["-KILL", "--", &format!("-{}", self.child.id())]).status();
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
        let _ = std::fs::remove_dir_all(&self.seals);
    }
}

/// A credentialed preflight from `origin` for `path`: what a browser sends before a
/// cross-origin request that carries the session.
async fn preflight(http: &reqwest::Client, door: &Door, path: &str, origin: &str) -> reqwest::Response {
    http.request(reqwest::Method::OPTIONS, format!("{}{path}", door.base))
        .header("Origin", origin)
        .header("Access-Control-Request-Method", "POST")
        .header("Access-Control-Request-Headers", "content-type")
        .send()
        .await
        .expect("preflight")
}

/// SEC-8: "The Door shall answer credentialed cross-origin requests only from the origins
/// it lists." An origin nobody listed is granted nothing, on the preflight or on the request.
#[tokio::test(flavor = "multi_thread")]
async fn dt26_sec8_an_unlisted_origin_is_granted_nothing() {
    let door = Door::start();
    let http = door.http();
    let path = door.icd_route(&http).await;

    let pre = preflight(&http, &door, &path, STRANGER).await;
    assert!(
        pre.headers().get("access-control-allow-origin").is_none(),
        "preflight from {STRANGER} was granted {:?}",
        pre.headers().get("access-control-allow-origin")
    );
    // `Access-Control-Allow-Credentials` alone grants nothing: a browser releases a
    // credentialed response only when `Access-Control-Allow-Origin` names its origin
    // (tower-http sends the credentials header to every preflight).

    let get = http
        .get(format!("{}{path}", door.base))
        .header("Origin", STRANGER)
        .send()
        .await
        .expect("request");
    assert!(
        get.headers().get("access-control-allow-origin").is_none(),
        "a request from {STRANGER} was granted {:?}",
        get.headers().get("access-control-allow-origin")
    );
}

/// SEC-8's control: the listed origin is granted, by name and with credentials, so the test
/// above cannot pass merely because cross-origin answers are broken altogether.
#[tokio::test(flavor = "multi_thread")]
async fn dt26_sec8_the_listed_origin_is_granted_by_name() {
    let door = Door::start();
    let http = door.http();
    let path = door.icd_route(&http).await;

    let pre = preflight(&http, &door, &path, LISTED).await;
    assert_eq!(
        pre.headers().get("access-control-allow-origin").and_then(|v| v.to_str().ok()),
        Some(LISTED),
        "the listed origin must be named, never `*`"
    );
    assert_eq!(
        pre.headers().get("access-control-allow-credentials").and_then(|v| v.to_str().ok()),
        Some("true")
    );
}

/// O-69, FC-15: a quiet session makes no relay round trip. Its held connection subscribed
/// once, at the open; its tick (every 3 s) neither syncs nor dials, and its reconciler runs
/// at 60 s, outside the window. The relay's own log is the witness: no connection opened and
/// no Sub drained over six ticks.
#[tokio::test(flavor = "multi_thread")]
async fn fc15_a_quiet_session_makes_no_relay_round_trip() {
    let auth = Auth::start();
    let relay = Relay::start();
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let _who = sign_up(&http, &door).await;
    // The open's own work: the first pass, the first catch-up, the first ticks.
    tokio::time::sleep(Duration::from_secs(8)).await;
    let seen = || {
        let log = std::fs::read_to_string(&relay.log).unwrap_or_default();
        (log.matches("connection opened").count(), log.matches("subscribe drain").count())
    };
    let before = seen();
    assert!(before.1 > 0, "the control: the relay logs each Sub ({before:?})");
    tokio::time::sleep(Duration::from_secs(20)).await;
    assert_eq!(seen(), before, "(connections, Subs) moved over a quiet 20 s");
}

/// SEC-6: "The Door shall not store a person's recovery words or seed in the clear."
/// After sign-up, with the session open, no file under `DOOR_ROOT` holds the phrase, the
/// seed it encodes, or that seed in hex.
#[tokio::test(flavor = "multi_thread")]
async fn sec6_the_store_holds_no_words_or_seed_in_the_clear() {
    let auth = Auth::start();
    let relay = Relay::start();
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let seed = pacific_core::identity::seed_from_recovery_key(&who.words).expect("the words encode a seed");
    let needles: Vec<Vec<u8>> = vec![who.words.clone().into_bytes(), seed.to_vec(), hex::encode(seed).into_bytes()];
    let mut files = 0;
    let mut stack = vec![door.root.clone()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(bytes) = std::fs::read(&path) {
                files += 1;
                for n in &needles {
                    assert!(!bytes.windows(n.len()).any(|w| w == &n[..]), "{} holds the words or the seed in the clear", path.display());
                }
            }
        }
    }
    assert!(files > 0, "the control: the Door's root holds no file at all");
}

/// DV-9: "Cookie routes require a matching `Origin` on writes." A browser sends one on
/// every write, `null` from a page that sends no referrer, so an unlisted Origin, `null`
/// or none is refused 403 and the write takes no effect. Sign-out is the write: refused,
/// the session still answers; from the listed origin, it ends.
#[tokio::test(flavor = "multi_thread")]
async fn dv9_a_cookie_write_needs_a_listed_origin() {
    let auth = Auth::start();
    let relay = Relay::start();
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let me = || http.get(format!("{}/v2/me", door.base)).header("cookie", &who.cookie).send();
    assert_eq!(me().await.unwrap().status().as_u16(), 200, "the control: the session answers");
    for origin in [Some(STRANGER), Some("null"), None] {
        let mut rq = http.post(format!("{}/v2/signout", door.base)).header("cookie", &who.cookie);
        if let Some(o) = origin {
            rq = rq.header("origin", o);
        }
        let r = rq.send().await.unwrap();
        assert_eq!(r.status().as_u16(), 403, "a cookie write with Origin {origin:?}");
        assert_eq!(me().await.unwrap().status().as_u16(), 200, "the refused write with Origin {origin:?} ended the session");
    }
    let r = http.post(format!("{}/v2/signout", door.base)).header("cookie", &who.cookie).header("origin", LISTED).send().await.unwrap();
    assert!(r.status().is_success(), "sign-out from the listed origin: {}", r.status());
    assert_eq!(me().await.unwrap().status().as_u16(), 401, "signed out, the session still answers");
}

/// SEC-5, CS-16: "The Door shall expose no unauthenticated route that lists sessions or
/// changes stored state." The development routes are gone, under every version.
#[tokio::test(flavor = "multi_thread")]
async fn dt24_sec5_no_route_lists_sessions_or_changes_state_unauthenticated() {
    let door = Door::start();
    let http = door.http();
    let gone: &[(&str, &str)] = &[
        ("POST", "/v1/persona"),
        ("GET", "/v1/personas"),
        ("GET", "/v1/sessions"),
        ("DELETE", "/v1/state"),
        ("POST", "/v2/persona"),
        ("GET", "/v2/personas"),
        ("GET", "/v2/sessions"),
        ("DELETE", "/v2/state"),
    ];
    for (method, path) in gone {
        let r = http
            .request(method.parse().unwrap(), format!("{}{path}", door.base))
            .body("{}")
            .header("content-type", "application/json")
            .send()
            .await
            .expect("reach the door");
        assert!(
            !r.status().is_success(),
            "{method} {path} answered {} without credentials",
            r.status()
        );
    }
}

/// SEC-5: every /v2 route but `icd` refuses a request that carries no credential.
#[tokio::test(flavor = "multi_thread")]
async fn dt24_sec5_every_v2_route_but_icd_needs_a_credential() {
    let door = Door::start();
    let http = door.http();
    let guarded: &[(&str, &str)] = &[
        ("GET", "/v2/me"),
        ("GET", "/v2/kinds"),
        ("GET", "/v2/graph"),
        ("GET", "/v2/draft/thing"),
        ("POST", "/v2/mint"),
        ("POST", "/v2/apply"),
        ("GET", "/v2/events"),
    ];
    for (method, path) in guarded {
        let r = http
            .request(method.parse().unwrap(), format!("{}{path}", door.base))
            .body("{}")
            .header("content-type", "application/json")
            .send()
            .await
            .expect("reach the door");
        assert_eq!(r.status().as_u16(), 401, "{method} {path} without a credential");
    }
}

/// CS-16: a session id in `X-Door-Session` is not a credential.
#[tokio::test(flavor = "multi_thread")]
async fn dt24_cs16_x_door_session_is_not_honoured() {
    let door = Door::start();
    let http = door.http();
    let r = http
        .get(format!("{}/v2/me", door.base))
        .header("X-Door-Session", "00000000000000000000000000000000")
        .send()
        .await
        .expect("reach the door");
    assert_eq!(r.status().as_u16(), 401);
}

/// SEC-27: "The Door shall allow itself to be framed only by the origins it lists." Every
/// Door page refuses framing.
#[tokio::test(flavor = "multi_thread")]
async fn dt14_sec27_the_door_refuses_to_be_framed() {
    let door = Door::start();
    let http = door.http();
    for path in ["/v2/icd", "/signin"] {
        let r = http.get(format!("{}{path}", door.base)).send().await.expect("reach the door");
        let csp = r
            .headers()
            .get("content-security-policy")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let xfo = r.headers().get("x-frame-options").and_then(|v| v.to_str().ok()).unwrap_or("");
        assert!(
            csp.contains("frame-ancestors 'none'") || xfo.eq_ignore_ascii_case("DENY"),
            "{path}: no frame-ancestors 'none' (CSP {csp:?}, X-Frame-Options {xfo:?})"
        );
    }
}

/// UX's perf audit: the webapp's page names the model's hash; by it the model comes gzip where
/// accepted, immutable for a year through every layer; `/v2/icd` stays PIN-5's, the pinned bytes
/// to a client asking for no encoding; any other hash is 404.
#[tokio::test(flavor = "multi_thread")]
async fn perf_the_model_is_named_by_its_hash_served_gzip_and_cached_for_good() {
    use sha2::Digest;
    use std::io::Read;
    let door = Door::start();
    let http = door.http();
    let pin = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../core/coordination/delta-graph.icd.sha256")).unwrap();
    let pin = pin.split_whitespace().next().unwrap().to_string();
    let varies = |r: &reqwest::Response| r.headers().get_all("vary").iter().any(|v| v.to_str().unwrap_or("").to_ascii_lowercase().contains("accept-encoding"));

    for path in ["/", "/index.html"] {
        let r = http.get(format!("{}{path}", door.base)).send().await.unwrap();
        assert_eq!((r.status().as_u16(), r.headers()["cache-control"].to_str().unwrap()), (200, "no-cache"), "{path}");
        assert!(r.text().await.unwrap().contains(&format!("id=\"wallflowers-icd\" content=\"{pin}\"")), "{path} names the pin");
    }

    let r = http.get(format!("{}/v2/icd/{pin}", door.base)).header("accept-encoding", "gzip, br").send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(r.headers()["cache-control"], "public, max-age=31536000, immutable");
    assert_eq!(r.headers()["content-encoding"], "gzip");
    assert!(varies(&r));
    let gz = r.bytes().await.unwrap();
    let mut model = Vec::new();
    flate2::read::GzDecoder::new(&gz[..]).read_to_end(&mut model).unwrap();
    assert_eq!(hex::encode(sha2::Sha256::digest(&model)), pin, "gzip {} bytes of {}", gz.len(), model.len());

    let r = http.get(format!("{}/v2/icd", door.base)).send().await.unwrap();
    assert_eq!(r.headers()["cache-control"], "public, max-age=60");
    assert!(r.headers().get("content-encoding").is_none() && varies(&r));
    assert_eq!(hex::encode(sha2::Sha256::digest(r.bytes().await.unwrap())), pin, "PIN-5 reads the pinned bytes");

    let r = http.get(format!("{}/v2/icd/{}", door.base, "0".repeat(64))).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 404);
    assert_ne!(r.headers().get("cache-control").and_then(|v| v.to_str().ok()), Some("public, max-age=31536000, immutable"));
}

// ─── with an account: the auth service, sign-up, and the sealed PRF ────────────

/// The workspace root, derived from this crate's place in it (app/door → product → ..).
fn workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// The website commit whose auth service these tests run against: the one that deploys,
/// not whatever the workspace's checkout holds (NC-73: its main had no seal counter, and
/// the D-34 tests failed for their environment). `DOOR_TEST_AUTH_REF` names another.
const AUTH_REF: &str = "be38a21";

/// The auth service (business/website/auth, at `AUTH_REF`) on a free port with a
/// throwaway database, run as the end-to-end stack runs it. Its process group dies on drop.
struct Auth {
    child: Child,
    root: PathBuf,
    url: String,
    log: PathBuf,
}

impl Auth {
    fn start() -> Auth {
        Auth::start_as(None)
    }

    /// The auth service, calling itself `public` where one is given: its address as a
    /// caller reaches it through something in front of it.
    fn start_as(public: Option<&str>) -> Auth {
        let ws = workspace();
        let website = ws.join("business/website");
        let python = ws.join(".venv/bin/python");
        assert!(website.exists() && python.exists(), "the auth service needs {} and {}", website.display(), python.display());
        // The auth service as committed at AUTH_REF, extracted into each attempt's root.
        let reference = std::env::var("DOOR_TEST_AUTH_REF").unwrap_or_else(|_| AUTH_REF.into());
        let archive = Command::new("git")
            .args(["-C", &website.to_string_lossy(), "archive", "--format=tar", &reference, "auth"])
            .output()
            .expect("git archive the auth service");
        assert!(archive.status.success(), "the website has no {reference}: {}", String::from_utf8_lossy(&archive.stderr));
        // THE PORT, TRIED UP TO THREE TIMES, as Door::launch's (795122b): another process can
        // take a free port before uvicorn binds it. The service is this one only once its own
        // line names the port.
        for attempt in 1..=3 {
            let root = std::env::temp_dir().join(format!("door-security-auth-{}", unique()));
            std::fs::create_dir_all(&root).unwrap();
            extract(&archive.stdout, &root);
            let dir = root.join("auth");
            let port = free_port();
            let url = format!("http://127.0.0.1:{port}");
            let (log, out, err) = log_beside(&root, "log");
            let child = Command::new(&python)
                .args(["-m", "uvicorn", "app.main:app", "--host", "127.0.0.1", "--port", &port.to_string()])
                .current_dir(&dir)
                .env("DB_PATH", root.join("auth.db"))
                .env("PUBLIC_ORIGIN", public.unwrap_or(&url))
                .stdout(out)
                .stderr(err)
                .process_group(0)
                .spawn()
                .expect("start the auth service");
            let mut auth = Auth { child, root, url, log };
            let ours = format!("running on http://127.0.0.1:{port} ");
            // As the Door's: a first launch after a build waits for the system's check.
            let deadline = Instant::now() + Duration::from_secs(120);
            let exited = loop {
                let said = std::fs::read_to_string(&auth.log).unwrap_or_default();
                if said.contains(&ours) {
                    break None;
                }
                if let Ok(Some(st)) = auth.child.try_wait() {
                    break Some((st, said));
                }
                assert!(Instant::now() < deadline, "the auth service named no port within 120 s");
                std::thread::sleep(Duration::from_millis(50));
            };
            match exited {
                None => return auth,
                Some((_, said)) if said.contains("address already in use") && attempt < 3 => {
                    eprintln!("door-security: port {port} was taken before the auth service bound it; attempt {} of 3", attempt + 1);
                }
                Some((st, said)) => {
                    panic!("the auth service exited ({st}) before it named its port: {}", said.lines().last().unwrap_or_default())
                }
            }
        }
        unreachable!("the third attempt returns or panics")
    }
}

/// Unpack `archive` into `into` through tar's stdin. tar stops reading at the end-of-archive
/// marker, and `git archive` pads past it to a whole record, so tar can close the pipe before
/// the padding is written: a broken pipe then is fine when tar exits 0. Any other write error,
/// or tar failing, is not.
fn extract(archive: &[u8], into: &std::path::Path) {
    let mut tar = Command::new("tar").args(["-x", "-C"]).arg(into).stdin(Stdio::piped()).spawn().expect("tar");
    let wrote = std::io::Write::write_all(tar.stdin.as_mut().unwrap(), archive);
    drop(tar.stdin.take());
    let status = tar.wait().expect("tar");
    match wrote {
        Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => panic!("extract into {}: {e}", into.display()),
        Err(_) if !status.success() => panic!("extract into {}: tar closed its input and failed ({status})", into.display()),
        _ => assert!(status.success(), "extract into {}: tar failed ({status})", into.display()),
    }
}

/// The broken pipe, on purpose: an archive padded well past its end-of-archive marker, more than
/// a pipe holds, so tar exits before the padding is written. It extracts, and is not an error.
#[test]
fn an_archive_tar_stops_reading_early_still_extracts() {
    let root = std::env::temp_dir().join(format!("door-security-tar-{}", unique()));
    let (src, into) = (root.join("src"), root.join("into"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&into).unwrap();
    std::fs::write(src.join("f"), "the archive's one file").unwrap();
    let made = Command::new("tar").args(["-c", "-f", "-", "-C"]).arg(&src).arg("f").output().expect("tar -c");
    assert!(made.status.success());
    let mut archive = made.stdout;
    archive.extend(std::iter::repeat_n(0u8, 4 << 20));
    extract(&archive, &into);
    assert_eq!(std::fs::read_to_string(into.join("f")).unwrap(), "the archive's one file");
    let _ = std::fs::remove_dir_all(&root);
}

impl Drop for Auth {
    fn drop(&mut self) {
        report("the auth service", &mut self.child, &self.log);
        let _ = Command::new("kill").args(["-KILL", "--", &format!("-{}", self.child.id())]).status();
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The Arc's relay, the binary the end-to-end stack builds (`cargo build -p relay` in arc),
/// on free loopback ports, its store in a throwaway directory. Nothing is reported
/// anywhere: no Sentry. Its process group dies on drop.
struct Relay {
    child: Child,
    root: PathBuf,
    url: String,
    log: PathBuf,
}

impl Relay {
    fn start() -> Relay {
        let bin = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../arc/target/debug/semaphore");
        assert!(bin.exists(), "this test needs the relay: run `cargo build -p relay` in arc ({} is absent)", bin.display());
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let tunnel = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let root = std::env::temp_dir().join(format!("door-security-relay-{}", unique()));
        std::fs::create_dir_all(&root).unwrap();
        let (log, out, err) = log_beside(&root, "log");
        let child = Command::new(bin)
            .current_dir(&root)
            .env("RELAY_BIND", format!("127.0.0.1:{port}"))
            .env("RELAY_TUNNEL_BIND", format!("127.0.0.1:{tunnel}"))
            .env_remove("SENTRY_DSN")
            .env_remove("RELAY_STORE")
            .stdout(out)
            .stderr(err)
            .process_group(0)
            .spawn()
            .expect("start the relay");
        // As the Door's: a first launch after a build waits for the system's check.
        let deadline = Instant::now() + Duration::from_secs(120);
        while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(Instant::now() < deadline, "the relay did not listen on {port} within 120 s");
            std::thread::sleep(Duration::from_millis(50));
        }
        Relay { child, root, url: format!("ws://127.0.0.1:{port}/v1/relay"), log }
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        report("the relay", &mut self.child, &self.log);
        let _ = Command::new("kill").args(["-KILL", "--", &format!("-{}", self.child.id())]).status();
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The PRF output sealed to a session key exactly as the sign-in window seals it:
/// app/web/door/seal.js, under node.
fn seal(key: &str, prf: &[u8; 32], attempt: &str) -> serde_json::Value {
    let js = "const {seal}=require(process.argv[1]);\
              Promise.resolve(seal(process.argv[2],Buffer.from(process.argv[3],'hex'),process.argv[4]))\
              .then(x=>process.stdout.write(JSON.stringify(x)))";
    let sealer = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../web/door/seal.js");
    let out = Command::new("node")
        .args(["-e", js, &sealer.to_string_lossy(), key, &hex::encode(prf), attempt])
        .output()
        .expect("node, for seal.js");
    assert!(out.status.success(), "seal.js: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("seal.js's output")
}

fn random32() -> [u8; 32] {
    let mut b = [0u8; 32];
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos().to_le_bytes();
    std::fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut b)).unwrap_or_else(|_| {
        b[..16].copy_from_slice(&t);
    });
    b
}

struct Account {
    words: String,
    pk: String,
    handle: String,
    prf: [u8; 32],
    cookie: String,
}

/// The cookie a response set, as `name=value`.
fn cookie_of(r: &reqwest::Response) -> Option<String> {
    r.headers().get("set-cookie").and_then(|v| v.to_str().ok()).and_then(|c| c.split(';').next()).map(str::to_string)
}

/// D-55: the work a start owes, paid as the window pays it: the Door's challenge, solved
/// by app/web/door/pow-worker.js under node. Null where none is owed.
async fn work(http: &reqwest::Client, door: &Door, start: &str) -> serde_json::Value {
    let r = http.get(format!("{}/v2/work?for={start}", door.base)).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200, "/v2/work?for={start}");
    let o: serde_json::Value = r.json().await.unwrap();
    match o["challenge"].as_str() {
        None => serde_json::Value::Null,
        Some(c) => serde_json::json!({ "challenge": c, "nonce": solve(c) }),
    }
}

/// The first count that does a challenge's work, as the window counts: pow-worker.js.
fn solve(challenge: &str) -> String {
    let solver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../web/door/pow-worker.js");
    let out = Command::new("node")
        .args(["-e", "process.stdout.write(require(process.argv[1]).solve(process.argv[2]))", &solver.to_string_lossy(), challenge])
        .output()
        .expect("node, for pow-worker.js");
    assert!(out.status.success(), "pow-worker.js: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).expect("pow-worker.js's nonce")
}

/// One person through sign-up, as DR-4 does it.
async fn sign_up(http: &reqwest::Client, door: &Door) -> Account {
    sign_up_as(http, door, "security").await
}

/// A sign-up under a display name of the test's choosing.
async fn sign_up_as(http: &reqwest::Client, door: &Door, name: &str) -> Account {
    let body = serde_json::json!({ "name": name, "work": work(http, door, "signup").await });
    let r = http.post(format!("{}/v2/signup", door.base)).json(&body).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200, "/v2/signup: {}", r.text().await.unwrap_or_default());
    let o: serde_json::Value = r.json().await.unwrap();
    assert!(o.get("words").is_none(), "/v2/signup gave the words before the account exists (cca2e2b)");
    let attempt = o["attempt"].as_str().unwrap().to_string();
    let prf = random32();
    let r = http
        .post(format!("{}/v2/signup/finish", door.base))
        .json(&serde_json::json!({ "attempt": attempt, "sealed": seal(o["key"].as_str().unwrap(), &prf, &attempt) }))
        .send()
        .await
        .unwrap();
    let cookie = cookie_of(&r).unwrap_or_default();
    let no_store = r.headers().get("cache-control").and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("no-store"));
    assert_eq!(r.status().as_u16(), 200, "/v2/signup/finish: {}", r.text().await.unwrap_or_default());
    assert!(no_store, "/v2/signup/finish carries the words and may be cached");
    let done: serde_json::Value = r.json().await.unwrap();
    // The words the tests look for on the record: 24, or those tests check nothing.
    let words = done["words"].as_str().unwrap_or_default().to_string();
    assert_eq!(words.split_whitespace().count(), 24, "/v2/signup/finish gave no 24 words");
    Account {
        words,
        pk: o["pk"].as_str().unwrap().to_string(),
        handle: o["handle"].as_str().unwrap().to_string(),
        prf,
        cookie,
    }
}

/// A sign-in attempt: its id and the session key to seal to.
async fn attempt(http: &reqwest::Client, door: &Door) -> (String, String) {
    let r = http.post(format!("{}/v2/signin", door.base)).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200, "/v2/signin");
    let o: serde_json::Value = r.json().await.unwrap();
    (o["attempt"].as_str().unwrap().to_string(), o["key"].as_str().unwrap().to_string())
}

async fn finish(http: &reqwest::Client, door: &Door, body: serde_json::Value) -> reqwest::Response {
    http.post(format!("{}/v2/signin/finish", door.base)).json(&body).send().await.unwrap()
}

/// SEC-4: "The Door shall open a session only on proof of possession of the person's key,
/// never on a public key alone." Proof is the PRF output that opens the account's wrap,
/// sealed to this attempt's session. A bare key, a missing or wrong PRF, an attempt used
/// twice, and a seal made for another attempt each open nothing; the valid pair opens.
#[tokio::test(flavor = "multi_thread")]
async fn dt17_sec4_only_the_prf_that_opens_the_wrap_opens_a_session() {
    let auth = Auth::start();
    let relay = Relay::start();
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;

    // a bare public key: there is no route that takes one
    for path in ["/v2/session", "/v1/session"] {
        let r = http.post(format!("{}{path}", door.base)).json(&serde_json::json!({ "pk": who.pk })).send().await.unwrap();
        assert!(!r.status().is_success(), "POST {path} {{pk}}: {}", r.status());
        assert!(cookie_of(&r).is_none(), "POST {path} {{pk}} set a cookie");
    }
    // the handle alone, with nothing sealed
    let (a, _) = attempt(&http, &door).await;
    let r = finish(&http, &door, serde_json::json!({ "attempt": a, "handle": who.handle })).await;
    assert!(!r.status().is_success() && cookie_of(&r).is_none(), "the handle alone: {}", r.status());
    // a wrong PRF
    let (a, k) = attempt(&http, &door).await;
    let r = finish(&http, &door, serde_json::json!({ "attempt": a, "handle": who.handle, "sealed": seal(&k, &random32(), &a) })).await;
    assert_eq!(r.status().as_u16(), 401, "a wrong PRF");
    assert!(cookie_of(&r).is_none());
    // a seal made for one attempt, presented with another
    let (a1, k1) = attempt(&http, &door).await;
    let (a2, _) = attempt(&http, &door).await;
    let r = finish(&http, &door, serde_json::json!({ "attempt": a2, "handle": who.handle, "sealed": seal(&k1, &who.prf, &a1) })).await;
    assert!(!r.status().is_success() && cookie_of(&r).is_none(), "another attempt's seal: {}", r.status());
    // the valid pair opens, and its attempt cannot be used again
    let (a, k) = attempt(&http, &door).await;
    let body = serde_json::json!({ "attempt": a, "handle": who.handle, "sealed": seal(&k, &who.prf, &a) });
    let r = finish(&http, &door, body.clone()).await;
    let opened = cookie_of(&r);
    assert_eq!(r.status().as_u16(), 200, "the valid pair: {}", r.text().await.unwrap_or_default());
    let me = http.get(format!("{}/v2/me", door.base)).header("cookie", opened.expect("a session cookie")).send().await.unwrap();
    assert_eq!(me.status().as_u16(), 200, "/v2/me with the session");
    let r = finish(&http, &door, body).await;
    assert_eq!(r.status().as_u16(), 401, "the same attempt, twice");
    assert!(cookie_of(&r).is_none());
}

/// SEC-7: "The Door's session cookie shall be sent only over HTTPS." Served at an https
/// origin, the session cookie is `__Host-`, Secure, HttpOnly and SameSite=Strict, with no
/// Domain, so no other host and no page script can have it.
#[tokio::test(flavor = "multi_thread")]
async fn dt25_sec7_the_session_cookie_is_secure_httponly_strict() {
    let auth = Auth::start();
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_PUBLIC", "https://door.example.test")]);
    let http = door.http();
    let body = serde_json::json!({ "name": "dt25", "work": work(&http, &door, "signup").await });
    let r = http.post(format!("{}/v2/signup", door.base)).json(&body).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200, "/v2/signup");
    let o: serde_json::Value = r.json().await.unwrap();
    let a = o["attempt"].as_str().unwrap().to_string();
    let r = http
        .post(format!("{}/v2/signup/finish", door.base))
        .json(&serde_json::json!({ "attempt": a, "sealed": seal(o["key"].as_str().unwrap(), &random32(), &a) }))
        .send()
        .await
        .unwrap();
    let set = r.headers().get("set-cookie").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    assert_eq!(r.status().as_u16(), 200, "/v2/signup/finish: {}", r.text().await.unwrap_or_default());
    let parts: Vec<String> = set.split(';').map(|s| s.trim().to_ascii_lowercase()).collect();
    assert!(set.starts_with("__Host-"), "the cookie is not __Host-: {set}");
    for flag in ["secure", "httponly", "samesite=strict", "path=/"] {
        assert!(parts.iter().any(|p| p == flag), "the cookie lacks {flag}: {set}");
    }
    assert!(!parts.iter().any(|p| p.starts_with("domain=")), "the cookie names a Domain: {set}");
}

/// DV-4 (DT-18, its header half): the sign-in window runs only its own script, keeps
/// nothing in a cache, and cannot be framed.
#[tokio::test(flavor = "multi_thread")]
async fn dt18_dv4_the_signin_window_runs_only_its_own_script() {
    let door = Door::start();
    let http = door.http();
    let r = http.get(format!("{}/signin", door.base)).send().await.expect("reach the door");
    assert_eq!(r.status().as_u16(), 200, "/signin");
    let csp = r.headers().get("content-security-policy").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let cache = r.headers().get("cache-control").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let directive = |name: &str| csp.split(';').map(str::trim).find(|d| d.starts_with(name)).unwrap_or("").to_string();
    assert_eq!(directive("default-src"), "default-src 'none'", "CSP: {csp}");
    assert_eq!(directive("script-src"), "script-src 'self'", "CSP: {csp}");
    assert_eq!(directive("connect-src"), "connect-src 'self'", "CSP: {csp}");
    assert_eq!(directive("frame-ancestors"), "frame-ancestors 'none'", "CSP: {csp}");
    assert!(!csp.contains("unsafe-inline") && !csp.contains("unsafe-eval"), "CSP: {csp}");
    assert!(cache.contains("no-store"), "Cache-Control: {cache}");
}

// ─── step 5: a public site's sign-in, its code and its DPoP token ─────────────

const CLIENT: &str = "dt-client";
const CALLBACK: &str = "https://client.example.test/auth/callback";

/// A registered client for the Door's stand-in registration (DOOR_CLIENTS, D-32).
fn clients_file(root: &std::path::Path) -> PathBuf {
    clients_file_named(root, "")
}

fn clients_file_named(root: &std::path::Path, name: &str) -> PathBuf {
    let path = root.join("clients.json");
    let reg = serde_json::json!({ CLIENT: {
        "name": name, "callbacks": [CALLBACK], "origins": ["https://client.example.test"], "site": ""
    }});
    std::fs::write(&path, reg.to_string()).unwrap();
    path
}

fn b64u(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// A client's DPoP key (RFC 9449), made as a page's non-extractable key would be.
struct Dpop {
    key: p256::ecdsa::SigningKey,
}

impl Dpop {
    fn new() -> Dpop {
        loop {
            if let Ok(key) = p256::ecdsa::SigningKey::from_slice(&random32()) {
                return Dpop { key };
            }
        }
    }

    /// A proof for one request: `htm`, `htu`, now, a fresh `jti`, and `ath` for a token.
    fn proof(&self, htm: &str, htu: &str, token: Option<&str>) -> String {
        self.proof_at(htm, htu, token, now_secs(), &hex::encode(random32()))
    }

    /// The same, dated `iat` and named `jti`.
    fn proof_at(&self, htm: &str, htu: &str, token: Option<&str>, iat: u64, jti: &str) -> String {
        use p256::ecdsa::signature::Signer;
        use sha2::Digest;
        let point = self.key.verifying_key().to_encoded_point(false);
        let header = serde_json::json!({ "typ": "dpop+jwt", "alg": "ES256", "jwk": {
            "kty": "EC", "crv": "P-256", "x": b64u(point.x().unwrap()), "y": b64u(point.y().unwrap()) }});
        let mut claims = serde_json::json!({ "htm": htm, "htu": htu, "jti": jti, "iat": iat });
        if let Some(t) = token {
            claims["ath"] = serde_json::Value::String(b64u(&sha2::Sha256::digest(t.as_bytes())));
        }
        let signing_input = format!("{}.{}", b64u(header.to_string().as_bytes()), b64u(claims.to_string().as_bytes()));
        let sig: p256::ecdsa::Signature = self.key.sign(signing_input.as_bytes());
        format!("{signing_input}.{}", b64u(&sig.to_bytes()))
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

/// PKCE (RFC 7636 S256): a verifier and its challenge.
fn pkce() -> (String, String) {
    use sha2::Digest;
    let verifier = b64u(&random32());
    let challenge = b64u(&sha2::Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// A public site's sign-in, finished in the window: the redirect it answers with.
async fn client_sign_in(http: &reqwest::Client, door: &Door, who: &Account, redirect_uri: &str, challenge: &str) -> reqwest::Response {
    let (a, k) = attempt(http, door).await;
    finish(http, door, serde_json::json!({
        "attempt": a, "handle": who.handle, "sealed": seal(&k, &who.prf, &a),
        "client": { "client": CLIENT, "redirect_uri": redirect_uri, "code_challenge": challenge, "state": "st-1" },
    }))
    .await
}

fn code_of(redirect: &str) -> String {
    let u = reqwest::Url::parse(redirect).expect("a redirect URL");
    u.query_pairs().find(|(k, _)| k == "code").map(|(_, v)| v.into_owned()).expect("a code")
}

async fn exchange(http: &reqwest::Client, door: &Door, proof: Option<String>, code: &str, verifier: &str, client: &str) -> reqwest::Response {
    let mut rq = http.post(format!("{}/v2/token", door.base)).json(&serde_json::json!({
        "code": code, "code_verifier": verifier, "client": client, "redirect_uri": CALLBACK,
    }));
    if let Some(p) = proof {
        rq = rq.header("DPoP", p);
    }
    rq.send().await.unwrap()
}

/// SEC-41 and DT-9: a code goes only to the callback the client registered, matched as
/// a whole string and answered as registered; a near miss gets no code.
#[tokio::test(flavor = "multi_thread")]
async fn dt9_sec41_a_code_goes_only_to_the_registered_callback() {
    let auth = Auth::start();
    let relay = Relay::start();
    let clients = clients_file(&auth.root);
    // Seven sign-ins from one address against its cap of four: a refused callback
    // ends its attempt (5d913fe).
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap()),
    ]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let (_, challenge) = pkce();
    for near in [
        "https://client.example.test/auth/callback/",
        "https://client.example.test/auth/other",
        "http://client.example.test/auth/callback",
        "https://client.example.test/auth/callback?x=1",
        "https://client.example.test.evil.example/auth/callback",
        "https://CLIENT.example.test/auth/callback",
    ] {
        let r = client_sign_in(&http, &door, &who, near, &challenge).await;
        let status = r.status().as_u16();
        let body = r.text().await.unwrap_or_default();
        assert!(!body.contains("code="), "{near} got a code: {body}");
        assert_eq!(status, 400, "{near}: {body}");
    }
    let r = client_sign_in(&http, &door, &who, CALLBACK, &challenge).await;
    assert!(cookie_of(&r).is_none(), "a public site's sign-in set the Door's cookie");
    let body: serde_json::Value = r.json().await.unwrap();
    let redirect = body["redirect"].as_str().expect("a redirect");
    assert!(redirect.starts_with(&format!("{CALLBACK}?code=")), "the redirect is not the registered callback: {redirect}");
    assert!(redirect.ends_with("&state=st-1"), "state was not returned unchanged: {redirect}");
    assert!(!redirect.contains("access_token") && !redirect.contains("token="), "a token rides the redirect: {redirect}");
}

/// DT-11, DT-20 and SEC-39: a code is spent once, by its client, with its verifier and a
/// DPoP proof, and the token it buys works only with proofs by that key, once each, for
/// the method and URL they name.
#[tokio::test(flavor = "multi_thread")]
async fn dt11_dt20_sec39_the_code_and_the_token_need_their_keys() {
    let auth = Auth::start();
    let relay = Relay::start();
    let clients = clients_file(&auth.root);
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap()),
    ]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let key = Dpop::new();
    let token_url = format!("{}/v2/token", door.base);

    // no proof, the wrong verifier, another client: each refused. A request that reaches
    // the code (the wrong verifier, another client) spends it, so a verifier gets one
    // guess per code; one refused for want of a proof never reaches the code, and a thief
    // without the verifier learns nothing from it.
    for case in ["no proof", "wrong verifier", "another client"] {
        let (verifier, challenge) = pkce();
        let r = client_sign_in(&http, &door, &who, CALLBACK, &challenge).await;
        let code = code_of(r.json::<serde_json::Value>().await.unwrap()["redirect"].as_str().unwrap());
        let r = match case {
            "no proof" => exchange(&http, &door, None, &code, &verifier, CLIENT).await,
            "wrong verifier" => exchange(&http, &door, Some(key.proof("POST", &token_url, None)), &code, &pkce().0, CLIENT).await,
            _ => exchange(&http, &door, Some(key.proof("POST", &token_url, None)), &code, &verifier, "another-client").await,
        };
        assert!(!r.status().is_success(), "{case}: {}", r.status());
        if case != "no proof" {
            let again = exchange(&http, &door, Some(key.proof("POST", &token_url, None)), &code, &verifier, CLIENT).await;
            assert!(!again.status().is_success(), "{case}: a refused code was redeemable after");
        }
    }

    // the valid exchange, then the same code again
    let (verifier, challenge) = pkce();
    let r = client_sign_in(&http, &door, &who, CALLBACK, &challenge).await;
    let code = code_of(r.json::<serde_json::Value>().await.unwrap()["redirect"].as_str().unwrap());
    let r = exchange(&http, &door, Some(key.proof("POST", &token_url, None)), &code, &verifier, CLIENT).await;
    assert_eq!(r.status().as_u16(), 200, "the valid exchange");
    let token = r.json::<serde_json::Value>().await.unwrap()["access_token"].as_str().unwrap().to_string();
    let r = exchange(&http, &door, Some(key.proof("POST", &token_url, None)), &code, &verifier, CLIENT).await;
    assert!(!r.status().is_success(), "the same code, twice");

    // the token at /v2/me
    let me = format!("{}/v2/me", door.base);
    let call = |auth: Option<String>, proof: Option<String>| {
        let mut rq = http.get(&me);
        if let Some(a) = auth { rq = rq.header("Authorization", a); }
        if let Some(p) = proof { rq = rq.header("DPoP", p); }
        rq.send()
    };
    let good = key.proof("GET", &me, Some(&token));
    assert_eq!(call(Some(format!("DPoP {token}")), Some(good.clone())).await.unwrap().status().as_u16(), 200, "token and proof");
    assert_eq!(call(Some(format!("DPoP {token}")), Some(good)).await.unwrap().status().as_u16(), 401, "the same proof twice");
    assert_eq!(call(Some(format!("DPoP {token}")), None).await.unwrap().status().as_u16(), 401, "no proof");
    assert_eq!(call(Some(format!("Bearer {token}")), None).await.unwrap().status().as_u16(), 401, "as a bearer token");
    let other = Dpop::new().proof("GET", &me, Some(&token));
    assert_eq!(call(Some(format!("DPoP {token}")), Some(other)).await.unwrap().status().as_u16(), 401, "another key's proof");
    let elsewhere = key.proof("GET", &format!("{}/v2/graph", door.base), Some(&token));
    assert_eq!(call(Some(format!("DPoP {token}")), Some(elsewhere)).await.unwrap().status().as_u16(), 401, "a proof for another URL");
    let unbound = key.proof("GET", &me, None);
    assert_eq!(call(Some(format!("DPoP {token}")), Some(unbound)).await.unwrap().status().as_u16(), 401, "a proof without ath");
}

// ─── W-90: the Site's setup as a build step, on its setup client ─────────────

/// The setup client for a Site, as the Door is deployed with it (clients.stand-in.json): its
/// id, its one callback, its Site.
fn setup_client(slug: &str) -> (PathBuf, String, String, String) {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("clients.stand-in.json");
    let reg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    let id = format!("wallflowers-setup-{slug}");
    let c = &reg[&id];
    let callbacks = c["callbacks"].as_array().expect("the setup client's callbacks");
    assert_eq!(callbacks.len(), 1, "{id}: one callback, the CLI's");
    let cb = callbacks[0].as_str().unwrap().to_string();
    (file, id, cb, c["site"].as_str().unwrap().to_string())
}

/// The window's passkey leg for a registered client, as the window does it: the redirect.
async fn window_sign_in(http: &reqwest::Client, door: &Door, who: &Account, client: &str, redirect_uri: &str, challenge: &str, state: &str) -> String {
    let (a, k) = attempt(http, door).await;
    let r = finish(http, door, serde_json::json!({
        "attempt": a, "handle": who.handle, "sealed": seal(&k, &who.prf, &a),
        "client": { "client": client, "redirect_uri": redirect_uri, "code_challenge": challenge, "state": state },
    }))
    .await;
    assert_eq!(r.status().as_u16(), 200, "the window's sign-in");
    r.json::<serde_json::Value>().await.unwrap()["redirect"].as_str().expect("a redirect").to_string()
}

/// W-90: A SITE'S TOKEN ADDS NO ONE (account.rs, Ask::Add). site-setup.mjs signs in as the
/// Site's setup client, so it re-runs the Site's setup; the first run's adds of the Arc are
/// the console's. Its token, traded as the CLI trades it, is refused at /v2/add by name.
#[tokio::test(flavor = "multi_thread")]
async fn w90_a_site_token_adds_no_one() {
    let auth = Auth::start();
    let relay = Relay::start();
    let (clients, client, callback, site) = setup_client("egregore");
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap()),
    ]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let (verifier, challenge) = pkce();
    let redirect = window_sign_in(&http, &door, &who, &client, &callback, &challenge, "w90").await;
    assert!(redirect.starts_with(&format!("{callback}?code=")), "the setup client's one callback: {redirect}");
    let key = Dpop::new();
    let token_url = format!("{}/v2/token", door.base);
    let r = http
        .post(&token_url)
        .header("DPoP", key.proof("POST", &token_url, None))
        .json(&serde_json::json!({ "code": code_of(&redirect), "code_verifier": verifier, "client": client, "redirect_uri": callback }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200, "the exchange");
    let t: serde_json::Value = r.json().await.unwrap();
    assert_eq!(t["scope"], site.as_str(), "the token is the Site's");
    let token = t["access_token"].as_str().unwrap();
    let add = format!("{}/v2/add", door.base);
    let r = http
        .post(&add)
        .header("Authorization", format!("DPoP {token}"))
        .header("DPoP", key.proof("POST", &add, Some(token)))
        .json(&serde_json::json!({ "object": site, "bundle": "b" }))
        .send()
        .await
        .unwrap();
    let status = r.status().as_u16();
    let body = r.text().await.unwrap_or_default();
    assert_eq!((status, body.contains("a site's token adds no one")), (400, true), "{status} {body}");
}

/// W-90, THE TOKEN AND DPoP HALF ON THE REAL DOOR: `make site-setup` (site-setup.mjs) stops,
/// naming it, when its one callback's port is taken; otherwise it opens the window for the
/// setup client with an S256 challenge and a state, takes the window's redirect on that
/// callback, trades the code at /v2/token with its proof, reads the graph with its token and
/// a fresh proof, and signs out at exit, ending the session. This account holds no Site of the
/// snippet's, so the snippet stops after that read; its writes are site-setup.test.mjs's.
#[tokio::test(flavor = "multi_thread")]
async fn w90_site_setup_signs_in_on_its_one_callback_and_signs_out() {
    use std::io::{BufRead, Read, Write};
    let auth = Auth::start();
    let relay = Relay::start();
    let (clients, client, callback, _) = setup_client("egregore");
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap()),
    ]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let before = open_session_dirs(&door);

    // The Arc's bundles, a stand-in on loopback: a different one each time.
    let arc = TcpListener::bind("127.0.0.1:0").unwrap();
    let arc_url = format!("http://127.0.0.1:{}/v1/bundle", arc.local_addr().unwrap().port());
    std::thread::spawn(move || {
        for (i, c) in arc.incoming().enumerate() {
            let Ok(mut c) = c else { continue };
            let mut buf = [0u8; 2048];
            let _ = c.read(&mut buf);
            let body = format!("bundle-{i}");
            let _ = write!(c, "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
        }
    });
    let mark = door.root.join("mark.png");
    use base64::Engine;
    let png = base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==")
        .unwrap();
    std::fs::write(&mark, png).unwrap();
    let cli = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("hosting/one-signin/site-setup.mjs");
    let setup = || {
        let mut cmd = Command::new("node");
        cmd.arg(&cli)
            .args(["egregore", mark.to_str().unwrap()])
            .env("DOOR", &door.base)
            .env("ARC_BUNDLE_URL", &arc_url)
            .env("SITE_SETUP_OPEN", "true")
            .env_remove("SITE_SETUP_CLIENTS")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(e) = &door.edge {
            cmd.env("NODE_EXTRA_CA_CERTS", &e.ca);
        }
        cmd.spawn().expect("node, for site-setup.mjs")
    };
    let port = reqwest::Url::parse(&callback).unwrap().port().expect("the callback's port");

    // Its port taken: it stops, naming it, and opens no window.
    {
        let _held = TcpListener::bind(("127.0.0.1", port)).expect("the setup client's callback port, free for this test");
        let out = setup().wait_with_output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "it ran with its port taken: {err}");
        assert!(err.contains(&format!("127.0.0.1:{port} is taken")) && !err.contains("the window: "), "{err}");
    }

    let mut child = setup();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let err = child.stderr.take().unwrap();
    let said = std::thread::spawn(move || {
        let mut all = String::new();
        for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
            let _ = tx.send(line.clone());
            all.push_str(&line);
            all.push('\n');
        }
        all
    });
    let window = loop {
        let line = rx.recv_timeout(Duration::from_secs(60)).expect("site-setup named its window within 60 s");
        if let Some(u) = line.strip_prefix("the window: ") {
            break reqwest::Url::parse(u).expect("the window's URL");
        }
    };
    let q: std::collections::HashMap<String, String> = window.query_pairs().into_owned().collect();
    assert_eq!(format!("{}{}", window.origin().ascii_serialization(), window.path()), format!("{}/signin", door.base));
    assert_eq!((q["client"].as_str(), q["redirect_uri"].as_str(), q["code_challenge_method"].as_str()), (client.as_str(), callback.as_str(), "S256"));
    let r = http.get(window.as_str()).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200, "the Door opens the window for the setup client");
    let redirect = window_sign_in(&http, &door, &who, &client, &callback, &q["code_challenge"], &q["state"]).await;
    let r = http.get(&redirect).send().await.expect("the CLI's callback answers");
    assert_eq!(r.status().as_u16(), 200, "the callback took the code");

    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        assert!(Instant::now() < deadline, "site-setup did not finish within 60 s");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let mut out = String::new();
    child.stdout.take().unwrap().read_to_string(&mut out).unwrap();
    let err = said.join().unwrap();
    assert_eq!(status.code(), Some(1), "the snippet stops: this account holds no Site of its own\n{out}{err}");
    for l in ["✓ signed in\n", "✓ the graph\n", "✓ signed out\n"] {
        assert!(out.contains(l), "{l:?} in:\n{out}{err}");
    }
    assert!(err.contains("this account holds no Site "), "{err}");
    let deadline = Instant::now() + Duration::from_secs(20);
    while open_session_dirs(&door) > before {
        assert!(Instant::now() < deadline, "the setup's session did not end at its sign-out");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// ─── consent, the bounds, and the refusal record (caffe1a) ───────────────────

/// door.md §5 step 3, and DT-8's consent: the window names the site before any passkey
/// is asked for, escaped, and an unregistered client gets no window at all. Since Ralph's
/// one step (29 Sep) the name heads the one screen the passkey is asked from, and titles it.
#[tokio::test(flavor = "multi_thread")]
async fn dt8_consent_names_the_site_escaped_before_any_passkey() {
    let root = std::env::temp_dir().join(format!("door-security-consent-{}", unique()));
    std::fs::create_dir_all(&root).unwrap();
    let hostile = "Egregore\"><script>alert(1)</script>";
    let clients = clients_file_named(&root, hostile);
    let door = Door::start_with(&[("DOOR_CLIENTS", clients.to_str().unwrap())]);
    let http = door.http();
    let (_, challenge) = pkce();
    let url = |client: &str, method: Option<&str>| {
        let mut u = reqwest::Url::parse(&format!("{}/signin", door.base)).unwrap();
        u.query_pairs_mut().append_pair("client", client).append_pair("redirect_uri", CALLBACK)
            .append_pair("code_challenge", &challenge).append_pair("state", "st-1");
        if let Some(m) = method {
            u.query_pairs_mut().append_pair("code_challenge_method", m);
        }
        u
    };
    let got = http.get(url(CLIENT, Some("S256"))).send().await.unwrap();
    assert_eq!(got.status().as_u16(), 200, "the site's window");
    let page = got.text().await.unwrap();
    let named = "Egregore&quot;&gt;&lt;script&gt;alert(1)&lt;/script&gt;";
    assert!(page.contains(&format!("<h1 class=\"site-name\">{named}</h1>")), "the registered name does not head the window, escaped");
    assert!(page.contains(&format!("<title>{named}</title>")), "the registered name does not title the window, escaped");
    assert!(!page.contains("<script>alert(1)"), "the registered name reached the page unescaped");
    let r = http.get(url("nobody", Some("S256"))).send().await.unwrap();
    assert!(r.status().is_client_error(), "an unregistered client got a window: {}", r.status());
    for method in [Some("plain"), None] {
        let r = http.get(url(CLIENT, method)).send().await.unwrap();
        assert!(r.status().is_client_error(), "code_challenge_method {method:?} got a window: {}", r.status());
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// DT-11, RFC 7636: a verifier outside 43–128 unreserved characters spends nothing.
#[tokio::test(flavor = "multi_thread")]
async fn dt11_the_verifier_is_held_to_rfc7636() {
    let auth = Auth::start();
    let relay = Relay::start();
    let clients = clients_file(&auth.root);
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap()),
    ]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let key = Dpop::new();
    let token_url = format!("{}/v2/token", door.base);
    for bad in ["a".repeat(42), "a".repeat(129), format!("{}!", "a".repeat(42))] {
        use sha2::Digest;
        let challenge = b64u(&sha2::Sha256::digest(bad.as_bytes()));
        let r = client_sign_in(&http, &door, &who, CALLBACK, &challenge).await;
        let code = code_of(r.json::<serde_json::Value>().await.unwrap()["redirect"].as_str().unwrap());
        let r = exchange(&http, &door, Some(key.proof("POST", &token_url, None)), &code, &bad, CLIENT).await;
        assert!(!r.status().is_success(), "a verifier of {} characters was accepted", bad.len());
    }
}

/// DT-11, RFC 7636 §4.2: the window opens only for a challenge the code can be bound
/// to, BASE64URL(SHA256(verifier)). One it opens for and `/v2/signup/finish` then
/// refuses comes after the account and its wrap are stored, and the words with it.
#[tokio::test(flavor = "multi_thread")]
async fn dt11_the_window_opens_only_for_an_s256_challenge() {
    let root = std::env::temp_dir().join(format!("door-security-challenge-{}", unique()));
    std::fs::create_dir_all(&root).unwrap();
    let clients = clients_file(&root);
    let door = Door::start_with(&[("DOOR_CLIENTS", clients.to_str().unwrap())]);
    let http = door.http();
    let window = |challenge: &str| {
        let mut u = reqwest::Url::parse(&format!("{}/signin", door.base)).unwrap();
        u.query_pairs_mut().append_pair("client", CLIENT).append_pair("redirect_uri", CALLBACK)
            .append_pair("code_challenge_method", "S256").append_pair("code_challenge", challenge).append_pair("state", "st-1");
        u
    };
    let (_, good) = pkce();
    assert_eq!(http.get(window(&good)).send().await.unwrap().status().as_u16(), 200, "an S256 challenge");
    for bad in ["abc".to_string(), b64u(&[7u8; 31]), format!("{good}A"), format!("{}!", &good[..42])] {
        let r = http.get(window(&bad)).send().await.unwrap();
        assert!(r.status().is_client_error(), "a challenge of {} characters, {bad:?}, got a window: {}", bad.len(), r.status());
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// NC-44: a public site's sign-up refused for its callback is refused before the
/// account exists: nothing signs in with that passkey.
#[tokio::test(flavor = "multi_thread")]
async fn nc44_a_refused_sign_up_leaves_no_account() {
    let auth = Auth::start();
    let relay = Relay::start();
    let clients = clients_file(&auth.root);
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap()),
    ]);
    let http = door.http();
    let body = serde_json::json!({ "name": "nc44", "work": work(&http, &door, "signup").await });
    let o: serde_json::Value = http.post(format!("{}/v2/signup", door.base)).json(&body)
        .send().await.unwrap().json().await.unwrap();
    let signup = o["attempt"].as_str().unwrap().to_string();
    let prf = random32();
    let sealed = seal(o["key"].as_str().unwrap(), &prf, &signup);
    let (_, good) = pkce();
    for (redirect_uri, challenge) in [(CALLBACK, "abc"), ("https://client.example.test/other", good.as_str())] {
        let r = http.post(format!("{}/v2/signup/finish", door.base)).json(&serde_json::json!({
            "attempt": signup, "sealed": sealed,
            "client": { "client": CLIENT, "redirect_uri": redirect_uri, "code_challenge": challenge, "state": "st-1" },
        })).send().await.unwrap();
        assert_eq!(r.status().as_u16(), 400, "{redirect_uri} {challenge:?}");
        let (a, k) = attempt(&http, &door).await;
        let r = finish(&http, &door, serde_json::json!({ "attempt": a, "handle": o["handle"], "sealed": seal(&k, &prf, &a) })).await;
        assert!(!r.status().is_success() && cookie_of(&r).is_none(), "the refused sign-up left an account: {}", r.status());
    }
}

/// SEC-A2: one address holds at most `DOOR_ATTEMPTS_PER_ADDR` open sign-in attempts,
/// answered 429, so one client cannot take every slot.
#[tokio::test(flavor = "multi_thread")]
async fn seca2_open_attempts_are_bounded_per_address() {
    let auth = Auth::start();
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_ATTEMPTS_PER_ADDR", "3")]);
    let http = door.http();
    for i in 0..3 {
        let r = http.post(format!("{}/v2/signin", door.base)).send().await.unwrap();
        assert_eq!(r.status().as_u16(), 200, "attempt {i}");
    }
    let r = http.post(format!("{}/v2/signin", door.base)).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 429, "a fourth open attempt from the same address");
}

/// SEC-A4, SEC-33, SEC-36: every refusal is one JSON line with a keyed address tag, and
/// no line holds a code, a token, a verifier, a PRF output, a seal or the words.
#[tokio::test(flavor = "multi_thread")]
async fn seca4_sec33_the_refusal_record_holds_no_secret() {
    let auth = Auth::start();
    let relay = Relay::start();
    let clients = clients_file(&auth.root);
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap()),
    ]);
    let http = door.http();
    let record = || std::fs::read_to_string(door.root.join("refusals.jsonl")).unwrap_or_default();
    // Two people, the same refusals each: the Door's own words (session, record, minute…)
    // fall alike in both halves of the record, and a person's word leaked is a surplus in
    // their own half (NC-68).
    let a = sign_up(&http, &door).await;
    let b = sign_up(&http, &door).await;
    let before = record().lines().count();
    let mut secrets = provoke(&http, &door, &a).await;
    let mid = record().lines().count();
    secrets.extend(provoke(&http, &door, &b).await);
    let all = record();
    let lines: Vec<&str> = all.lines().collect();
    assert!(mid - before >= 3, "expected at least three refusals on the record, found {}", mid - before);
    // A failure names where, never what: a line's number and a secret's index, so no
    // message, log or artefact of this test ever carries a word, a key or a token.
    for (n, line) in lines.iter().enumerate().skip(before) {
        let v: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|_| panic!("record line {n} is not one JSON object"));
        for field in ["t", "route", "status", "reason", "client"] {
            assert!(!v[field].is_null(), "record line {n} has no {field}");
        }
        let tag = v["client"].as_str().unwrap_or_default();
        assert!(tag.len() == 24 && tag.chars().all(|c| c.is_ascii_hexdigit()), "record line {n}: the address tag is not a keyed hash");
        assert!(!line.contains("127.0.0.1"), "record line {n} names the address");
        for (i, s) in secrets.iter().enumerate() {
            assert!(s.is_empty() || !line.contains(s.as_str()), "record line {n} holds secret #{i}");
        }
    }
    // Every single word, as a whole token.
    let count = |part: &[&str], w: &str| -> usize {
        part.iter().map(|l| l.split(|c: char| !c.is_ascii_alphabetic()).filter(|t| t.eq_ignore_ascii_case(w)).count()).sum()
    };
    let (in_a, in_b) = (&lines[before..mid], &lines[mid..]);
    for (owner, who, mine, theirs) in [("A", &a, in_a, in_b), ("B", &b, in_b, in_a)] {
        for (k, w) in who.words.split_whitespace().enumerate() {
            let (m, t) = (count(mine, w), count(theirs, w));
            assert!(m == t, "recovery word #{} of {owner} is on the record: {m} times in {owner}'s refusals, {t} in the other's", k + 1);
        }
    }
}

/// One person's refusals for SEC-A4's record: a wrong PRF with its seal, a code refused
/// for its verifier, a token used without its proof, and a body refused by quoting it.
/// Answers what of theirs must never be on the record, but the words one by one.
async fn provoke(http: &reqwest::Client, door: &Door, who: &Account) -> Vec<String> {
    let key = Dpop::new();
    let token_url = format!("{}/v2/token", door.base);
    let mut secrets: Vec<String> = vec![hex::encode(who.prf), who.cookie.split('=').nth(1).unwrap_or_default().to_string()];
    // The words as runs: the phrase, each run of three, and the seed they encode.
    let words: Vec<&str> = who.words.split_whitespace().collect();
    secrets.push(who.words.clone());
    secrets.extend(words.windows(3).map(|w| w.join(" ")));
    let seed = pacific_core::identity::seed_from_recovery_key(&who.words).expect("the words encode a seed");
    secrets.push(hex::encode(seed));
    // a wrong PRF, with its seal
    let (a, k) = attempt(http, door).await;
    let wrong = random32();
    let sealed = seal(&k, &wrong, &a);
    secrets.push(hex::encode(wrong));
    secrets.push(sealed["ct"].as_str().unwrap().to_string());
    let _ = finish(http, door, serde_json::json!({ "attempt": a, "handle": who.handle, "sealed": sealed })).await;
    // a code refused for its verifier, then a token used without its proof
    let (verifier, challenge) = pkce();
    let r = client_sign_in(http, door, who, CALLBACK, &challenge).await;
    let code = code_of(r.json::<serde_json::Value>().await.unwrap()["redirect"].as_str().unwrap());
    let wrong_verifier = pkce().0;
    let _ = exchange(http, door, Some(key.proof("POST", &token_url, None)), &code, &wrong_verifier, CLIENT).await;
    secrets.extend([code, verifier.clone(), wrong_verifier]);
    let (verifier, challenge) = pkce();
    let r = client_sign_in(http, door, who, CALLBACK, &challenge).await;
    let code = code_of(r.json::<serde_json::Value>().await.unwrap()["redirect"].as_str().unwrap());
    let r = exchange(http, door, Some(key.proof("POST", &token_url, None)), &code, &verifier, CLIENT).await;
    let token = r.json::<serde_json::Value>().await.unwrap()["access_token"].as_str().unwrap().to_string();
    let _ = http.get(format!("{}/v2/me", door.base)).header("Authorization", format!("DPoP {token}")).send().await.unwrap();
    secrets.extend([token, code, verifier]);
    // a body axum refuses by quoting it: the Door did not word that refusal (9a9bdd9)
    let quoted = format!("PERSON-WROTE-THIS-{}", hex::encode(random32()));
    let r = http.post(format!("{}/v2/signin/finish", door.base)).header("content-type", "application/json")
        .body(serde_json::json!({ "attempt": "x", "sealed": quoted }).to_string()).send().await.unwrap();
    assert!(r.status().is_client_error(), "a malformed body: {}", r.status());
    secrets.push(quoted);
    secrets
}

// ─── NC-40: the caps on open sign-in attempts (e094cf9) ──────────────────────

/// Start one sign-in attempt as if from `ip` (the Door trusts CF-Connecting-IP only
/// when told to, as it is behind the tunnel), paying the work when it is owed (D-55): a
/// refusal is then the caps', in their words, and never the work's.
async fn start_from(http: &reqwest::Client, door: &Door, ip: &str) -> u16 {
    let body = serde_json::json!({ "work": work(http, door, "signin").await });
    let r = http.post(format!("{}/v2/signin", door.base)).header("CF-Connecting-IP", ip).json(&body).send().await.unwrap();
    let status = r.status().as_u16();
    let why = r.text().await.unwrap_or_default();
    match status {
        429 => assert_eq!(why, "too many sign-ins are open from here; finish one or wait", "a 429 not the per-address cap's"),
        503 => assert_eq!(why, "too many sign-ins are open; try again shortly", "a 503 not the site-wide cap's"),
        _ => assert_ne!(status, 403, "a start that paid its work was refused: {why}"),
    }
    status
}

fn open_session_dirs(door: &Door) -> usize {
    std::fs::read_dir(door.root.join("sessions")).map(|d| d.count()).unwrap_or(0)
}

/// SEC-A2, NC-40 (a): the per-network cap counts an IPv6 /64 as one network, so a party
/// with a /64 cannot take more than its four open attempts by changing addresses.
#[tokio::test(flavor = "multi_thread")]
async fn nc40_seca2_one_slash64_is_one_network() {
    let auth = Auth::start();
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_TRUST_FORWARDED", "1")]);
    let http = door.http();
    for i in 1..=4 {
        assert_eq!(start_from(&http, &door, &format!("2001:db8:1:2::{i:x}")).await, 200, "address {i} in the /64");
    }
    assert_eq!(start_from(&http, &door, "2001:db8:1:2:ffff:ffff:ffff:ffff").await, 429, "a fifth address in the same /64");
    assert_eq!(start_from(&http, &door, "2001:db8:1:3::1").await, 200, "another /64 has its own cap");
}

/// SEC-A2, NC-40 (b): at the site-wide cap a new start is refused, 503, and starts no
/// process: the refusal comes before anything is spent.
#[tokio::test(flavor = "multi_thread")]
async fn nc40_seca2_the_site_wide_cap_refuses_and_starts_nothing() {
    let auth = Auth::start();
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()), ("DOOR_TRUST_FORWARDED", "1"), ("DOOR_MAX_ATTEMPTS", "6"),
    ]);
    let http = door.http();
    for i in 0..6 {
        let ip = format!("198.51.100.{}", i / 4 + 1);
        assert_eq!(start_from(&http, &door, &ip).await, 200, "attempt {i}");
    }
    let before = open_session_dirs(&door);
    assert_eq!(start_from(&http, &door, "203.0.113.9").await, 503, "a new network at the site-wide cap");
    assert_eq!(open_session_dirs(&door), before, "the refused start made a session directory");
}

/// NC-80: an image started with stdin closed, or with 0, 1 and 2 all closed, still starts a
/// session. Rust's runtime opens /dev/null onto a closed 0, 1 or 2 before `main`, so no
/// socket or file the Door opens takes a standard descriptor; this pins that, so a change of
/// runtime cannot regress it unseen.
#[tokio::test(flavor = "multi_thread")]
async fn nc80_a_door_started_with_closed_std_fds_still_starts_a_session() {
    // A sign-in's first half asks no auth service: none is started, so this runs anywhere.
    for closed in [&[0][..], &[0, 1, 2][..]] {
        let door = Door::start_closed(&[], closed);
        let http = door.http();
        let body = serde_json::json!({ "work": work(&http, &door, "signin").await });
        let r = http.post(format!("{}/v2/signin", door.base)).json(&body).send().await.unwrap();
        let status = r.status().as_u16();
        let why = r.text().await.unwrap_or_default();
        assert_eq!(status, 200, "fds {closed:?} closed: /v2/signin answered {status}: {why}");
        assert_eq!(open_session_dirs(&door), 1, "fds {closed:?} closed: one session");
    }
}

/// NC-80: on Linux a session starts from the running image after the binary the Door was
/// started from is replaced, as a deploy's `install` replaces /opt/door/bin/door before the
/// Door restarts. current_exe() is then "<path> (deleted)", and spawning it fails.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn nc80_a_session_starts_from_the_running_image_after_its_binary_is_replaced() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("door-nc80-{}", unique()));
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join("door");
    std::fs::copy(env!("CARGO_BIN_EXE_door"), &bin).unwrap();
    let door = Door::launch(&bin, &[], &[]);
    // Replaced as install(1) replaces it: unlinked, and a new file at its path.
    std::fs::remove_file(&bin).unwrap();
    std::fs::write(&bin, b"#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let http = door.http();
    let r = http.post(format!("{}/v2/signin", door.base)).json(&serde_json::json!({})).send().await.unwrap();
    let status = r.status().as_u16();
    let why = r.text().await.unwrap_or_default();
    assert_eq!(status, 200, "its binary replaced: /v2/signin answered {status}: {why}");
    assert_eq!(open_session_dirs(&door), 1, "its binary replaced: one session");
    drop(door);
    let _ = std::fs::remove_dir_all(&dir);
}

/// NC-80: where a session is spawned from the path (not Linux), a rebuilt file there is not
/// the supervisor's image: the session refuses, naming both, and the start says so rather
/// than failing on a leash it does not understand. The same bytes, a new file: every
/// unstamped build has one stamp, so the image is what tells them apart.
#[cfg(not(target_os = "linux"))]
#[tokio::test(flavor = "multi_thread")]
async fn nc80_a_session_from_a_rebuilt_binary_refuses_naming_both_images() {
    let dir = std::env::temp_dir().join(format!("door-nc80-{}", unique()));
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join("door");
    std::fs::copy(env!("CARGO_BIN_EXE_door"), &bin).unwrap();
    let door = Door::launch(&bin, &[], &[]);
    let http = door.http();
    let r = http.post(format!("{}/v2/signin", door.base)).json(&serde_json::json!({})).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200, "its own image: {}", r.text().await.unwrap_or_default());
    // Rebuilt, as cargo and install(1) replace a binary: a new file at the path.
    let next = dir.join("door.next");
    std::fs::copy(env!("CARGO_BIN_EXE_door"), &next).unwrap();
    std::fs::rename(&next, &bin).unwrap();
    let r = http.post(format!("{}/v2/signin", door.base)).json(&serde_json::json!({})).send().await.unwrap();
    let status = r.status().as_u16();
    let why = r.text().await.unwrap_or_default();
    assert_eq!(status, 500, "a rebuilt binary: {why}");
    assert!(why.contains("the session process did not start: it refused: this session is build") && why.contains("its supervisor is build")
        && why.contains("restart the Door"), "{why}");
    drop(door);
    let _ = std::fs::remove_dir_all(&dir);
}

/// SEC-A2, NC-40 (c): an attempt nobody finishes expires, and its slot is free again.
#[tokio::test(flavor = "multi_thread")]
async fn nc40_seca2_an_expired_attempt_frees_its_slot() {
    let auth = Auth::start();
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()), ("DOOR_TRUST_FORWARDED", "1"),
        ("DOOR_MAX_ATTEMPTS", "1"), ("DOOR_ATTEMPT_SECS", "2"),
    ]);
    let http = door.http();
    assert_eq!(start_from(&http, &door, "198.51.100.1").await, 200, "the one slot");
    assert_eq!(start_from(&http, &door, "198.51.100.2").await, 503, "the cap, while it is held");
    // the attempt lives 2 s; the sweeper runs every 10 s
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let s = start_from(&http, &door, "198.51.100.2").await;
        if s == 200 {
            break;
        }
        assert!(Instant::now() < deadline, "the slot did not free within 30 s: {s}");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

// ─── D-55: the work a start pays (CS-17) ─────────────────────────────────────

/// A start's status and the Door's words, `body` posted to `path` with `cookie`.
async fn post_start(http: &reqwest::Client, door: &Door, path: &str, body: serde_json::Value, cookie: Option<&str>) -> (u16, String) {
    let mut rq = http.post(format!("{}{path}", door.base)).json(&body);
    if let Some(c) = cookie {
        rq = rq.header("cookie", c);
    }
    let r = rq.send().await.unwrap();
    (r.status().as_u16(), r.text().await.unwrap_or_default())
}

/// A sign-up without a kiosk claim pays its work: none, a count that does not do it, or a
/// challenge spent already is refused in the Door's words and starts no process; the
/// challenge solved starts it, once.
#[tokio::test(flavor = "multi_thread")]
async fn d55_a_sign_up_without_a_claim_pays_its_work() {
    let door = Door::start();
    let http = door.http();
    let refused = |why: &str| (403u16, why.to_string());
    let up = |body: serde_json::Value| post_start(&http, &door, "/v2/signup", body, None);

    assert_eq!(up(serde_json::json!({ "name": "d55" })).await, refused("this start needs its proof of work"));
    // The first count that does it, and one before it that does not: a challenge whose first is 0 has none before.
    let (paid, n) = loop {
        let w = work(&http, &door, "signup").await;
        let n: u64 = w["nonce"].as_str().expect("a sign-up without a claim owes the work").parse().unwrap();
        if n > 0 {
            break (w, n);
        }
    };
    let short = serde_json::json!({ "challenge": paid["challenge"], "nonce": (n - 1).to_string() });
    assert_eq!(up(serde_json::json!({ "work": short })).await, refused("the nonce does not do the work"));
    assert_eq!(open_session_dirs(&door), 0, "a start refused for its work started a process");

    assert_eq!(up(serde_json::json!({ "work": paid })).await.0, 200, "the challenge solved");
    assert_eq!(up(serde_json::json!({ "work": paid })).await, refused("the challenge was already spent"));
    assert_eq!(open_session_dirs(&door), 1, "one start, one process");
}

/// A kiosk's key and one claim it issues, now, for a Site (A-3): the key list the Door is
/// started with, and the token the QR carries.
fn kiosk_claim(dir: &std::path::Path) -> (PathBuf, String) {
    kiosk_claim_with(dir, serde_json::json!({}))
}

/// As `kiosk_claim`, its payload carrying `extra` too: a choice (`c`), a share (`a`).
fn kiosk_claim_with(dir: &std::path::Path, extra: serde_json::Value) -> (PathBuf, String) {
    use ed25519_dalek::Signer;
    use sha2::Digest;
    let key = ed25519_dalek::SigningKey::from_bytes(&random32());
    let pk = key.verifying_key().to_bytes();
    let kid = hex::encode(&sha2::Sha256::digest(pk)[..8]);
    std::fs::create_dir_all(dir).unwrap();
    let keys = dir.join("claim-keys.json");
    std::fs::write(&keys, format!(r#"{{"{kid}":"{}"}}"#, b64u(&pk))).unwrap();
    let now = now_secs();
    let mut payload = serde_json::json!({ "k": kid, "s": "5e".repeat(32), "n": b64u(&random32()[..16]), "iat": now, "exp": now + 600 });
    payload.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
    let p = b64u(payload.to_string().as_bytes());
    let sig = key.sign(format!("v1.{p}").as_bytes());
    (keys, format!("v1.{p}.{}", b64u(&sig.to_bytes())))
}

/// A sign-up a live kiosk claim waits for owes no work, and asking spends no claim; a
/// sign-in start owes none while the pool is under half.
#[tokio::test(flavor = "multi_thread")]
async fn d55_a_claims_sign_up_and_a_sign_in_under_half_owe_no_work() {
    let dir = std::env::temp_dir().join(format!("door-security-kiosk-{}", unique()));
    let (keys, claim) = kiosk_claim(&dir);
    let door = Door::start_with(&[("DOOR_CLAIM_KEYS", keys.to_str().unwrap())]);
    let http = door.http();

    // The QR's landing, its 303 not followed: the cookie it sets names the claim.
    let stay = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
    let stay = match &door.edge {
        None => stay,
        Some(e) => stay
            .tls_built_in_root_certs(false)
            .add_root_certificate(reqwest::Certificate::from_pem(&std::fs::read(&e.ca).unwrap()).unwrap()),
    };
    let r = stay.build().unwrap().get(format!("{}/join?claim={claim}", door.base)).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 303, "the claim lands");
    let join = cookie_of(&r).expect("the join cookie");
    let owed = |c: Option<String>| {
        let http = http.clone();
        let url = format!("{}/v2/work?for=signup", door.base);
        async move {
            let mut rq = http.get(url);
            if let Some(c) = c {
                rq = rq.header("cookie", c);
            }
            rq.send().await.unwrap().json::<serde_json::Value>().await.unwrap()["challenge"].is_string()
        }
    };
    assert!(owed(None).await, "without the cookie, a sign-up owes the work");
    assert!(!owed(Some(join.clone())).await, "with it, none");
    let r = post_start(&http, &door, "/v2/signup", serde_json::json!({ "name": "kiosk" }), Some(&join)).await;
    assert_eq!(r.0, 200, "a claim's sign-up, no work: {}", r.1);
    assert!(!owed(Some(join.clone())).await, "starting spent no claim");

    assert_eq!(work(&http, &door, "signin").await, serde_json::Value::Null, "under half, a sign-in owes none");
    let r = post_start(&http, &door, "/v2/signin", serde_json::json!({}), None).await;
    assert_eq!(r.0, 200, "a sign-in, no work: {}", r.1);

    // Owed none, work presented is checked all the same, on both starts.
    let (w, n) = loop {
        let w = work(&http, &door, "signup").await;
        let n: u64 = w["nonce"].as_str().unwrap().parse().unwrap();
        if n > 0 {
            break (w, n);
        }
    };
    let short = serde_json::json!({ "challenge": w["challenge"], "nonce": (n - 1).to_string() });
    let r = post_start(&http, &door, "/v2/signup", serde_json::json!({ "work": short }), Some(&join)).await;
    assert_eq!(r, (403, "the nonce does not do the work".to_string()), "a claim's sign-up presenting a count short");
    let r = post_start(&http, &door, "/v2/signin", serde_json::json!({ "work": w }), None).await;
    assert_eq!(r, (403, "the challenge was issued for another start".to_string()), "a sign-up's challenge on a sign-in");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Over half the site-wide cap a sign-in start owes the work: without it, refused by name;
/// with it, started.
#[tokio::test(flavor = "multi_thread")]
async fn d55_over_half_the_cap_a_sign_in_pays() {
    let door = Door::start_with(&[("DOOR_TRUST_FORWARDED", "1"), ("DOOR_MAX_ATTEMPTS", "4")]);
    let http = door.http();
    for i in 1..=3 {
        assert_eq!(start_from(&http, &door, &format!("198.51.100.{i}")).await, 200, "attempt {i}");
    }
    let bare = http.post(format!("{}/v2/signin", door.base)).header("CF-Connecting-IP", "198.51.100.9").send().await.unwrap();
    assert_eq!(bare.status().as_u16(), 403);
    assert_eq!(bare.text().await.unwrap(), "this start needs its proof of work", "3 of 4 is over half");

    // A sign-in's work is checked as a sign-up's: a count short, then the challenge spent twice.
    let (paid, n) = loop {
        let w = work(&http, &door, "signin").await;
        let n: u64 = w["nonce"].as_str().expect("over half, a sign-in owes the work").parse().unwrap();
        if n > 0 {
            break (w, n);
        }
    };
    let signin = |w: &serde_json::Value, ip: &str| {
        let rq = http.post(format!("{}/v2/signin", door.base)).header("CF-Connecting-IP", ip).json(&serde_json::json!({ "work": w }));
        async move {
            let r = rq.send().await.unwrap();
            (r.status().as_u16(), r.text().await.unwrap_or_default())
        }
    };
    let short = serde_json::json!({ "challenge": paid["challenge"], "nonce": (n - 1).to_string() });
    assert_eq!(signin(&short, "198.51.100.9").await, (403, "the nonce does not do the work".to_string()));
    assert_eq!(signin(&paid, "198.51.100.9").await.0, 200, "paid, the fourth starts");
    assert_eq!(signin(&paid, "198.51.100.10").await, (403, "the challenge was already spent".to_string()));
}

// ─── D-34 (c): one process per person ────────────────────────────────────────

/// `who` signs in, as DR-4 does it: the new session's cookie.
async fn sign_in(http: &reqwest::Client, door: &Door, who: &Account) -> String {
    let (a, k) = attempt(http, door).await;
    let r = finish(http, door, serde_json::json!({ "attempt": a, "handle": who.handle, "sealed": seal(&k, &who.prf, &a) })).await;
    let c = cookie_of(&r);
    assert_eq!(r.status().as_u16(), 200, "/v2/signin/finish: {}", r.text().await.unwrap_or_default());
    c.expect("a session cookie")
}

async fn sign_out(http: &reqwest::Client, door: &Door, cookie: &str) -> u16 {
    let r = http.post(format!("{}/v2/signout", door.base)).header("cookie", cookie).header("origin", LISTED).send().await.unwrap();
    r.status().as_u16()
}

/// `/v2/me` on a cookie: its status, and the key it names.
async fn me_on(http: &reqwest::Client, door: &Door, cookie: &str) -> (u16, String) {
    let r = http.get(format!("{}/v2/me", door.base)).header("cookie", cookie).send().await.unwrap();
    let s = r.status().as_u16();
    (s, r.json::<serde_json::Value>().await.map(|v| v["pk"].as_str().unwrap_or_default().to_string()).unwrap_or_default())
}

/// K-41: a second sign-in of a person, 0.3 s after the first, is one more session of the
/// same process: no second process, so no second device, leaf or commit.
#[tokio::test(flavor = "multi_thread")]
async fn d34_k41_a_second_sign_in_is_the_same_process() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    assert_eq!(sign_out(&http, &door, &who.cookie).await, 200);
    assert_eq!(open_session_dirs(&door), 0, "signed out of their one session, the person's process ended");
    let one = sign_in(&http, &door, &who).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let two = sign_in(&http, &door, &who).await;
    assert_ne!(one, two);
    assert_eq!(open_session_dirs(&door), 1, "one process for both sessions");
    let (a, b) = (me_on(&http, &door, &one).await, me_on(&http, &door, &two).await);
    assert_eq!(a.0, 200, "/v2/me on the first");
    assert_eq!(a, b, "both sessions are the one person's");
}

/// NC-58's regression: ten sign-ins of one person at once make one process.
#[tokio::test(flavor = "multi_thread")]
async fn d34_nc58_ten_sign_ins_of_one_person_make_one_process() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_ATTEMPTS_PER_ADDR", "10"),
    ]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    assert_eq!(sign_out(&http, &door, &who.cookie).await, 200);
    let cookies = futures_util::future::join_all((0..10).map(|_| sign_in(&http, &door, &who))).await;
    assert_eq!(cookies.iter().collect::<std::collections::HashSet<_>>().len(), 10, "ten sessions");
    assert_eq!(open_session_dirs(&door), 1, "one process");
    for c in &cookies {
        assert_eq!(me_on(&http, &door, c).await.0, 200);
    }
}

/// Signing out ends that session alone (Software Security): the person's other session
/// goes on in their process, and their last takes the process with it.
#[tokio::test(flavor = "multi_thread")]
async fn d34_signing_out_one_session_leaves_the_other() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let other = sign_in(&http, &door, &who).await;
    assert_eq!(open_session_dirs(&door), 1);
    assert_eq!(sign_out(&http, &door, &who.cookie).await, 200);
    assert_eq!(me_on(&http, &door, &who.cookie).await.0, 401, "the signed-out session");
    assert_eq!(me_on(&http, &door, &other).await.0, 200, "the other goes on");
    assert_eq!(open_session_dirs(&door), 1);
    assert_eq!(sign_out(&http, &door, &other).await, 200);
    assert_eq!(open_session_dirs(&door), 0, "the last took the process");
}

/// D-34 (c) through the Door: a person's state, sealed at their last sign-out and opened at
/// their next sign-in, still holds the owner's chain. An owner-sequenced write goes
/// through on the restored device (a device without the chain is refused them, NC-65),
/// and it holds nothing it cannot fold.
#[tokio::test(flavor = "multi_thread")]
async fn d34_an_owner_sequenced_write_after_a_restore() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let site = mint_as(&http, &door, &who.cookie, "group", "the Site").await;
    assert_eq!(sign_out(&http, &door, &who.cookie).await, 200);
    assert_eq!(open_session_dirs(&door), 0, "the last sign-out took the process");
    let sealed = std::fs::read_dir(&door.seals).unwrap().filter_map(Result::ok).filter(|e| e.path().extension().is_none()).count();
    assert_eq!(sealed, 1, "the person's state is sealed at its end");
    let c = sign_in(&http, &door, &who).await;
    let host = mint_as(&http, &door, &c, "host", "the Site").await;
    let at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    for (object, op, args) in [
        (&site, "base.setPart", serde_json::json!({ "part": host, "role": "host", "at": at })),
        (&host, "base.setParent", serde_json::json!({ "parent": site, "role": "host", "at": at })),
    ] {
        let r = http
            .post(format!("{}/v2/apply", door.base))
            .header("cookie", &c)
            .header("origin", LISTED)
            .json(&serde_json::json!({ "object": object, "op": op, "args": args }))
            .send()
            .await
            .unwrap();
        let (s, body) = said(r).await;
        assert_eq!(s, 200, "{op} on the restored device: {body}");
    }
    let log = std::fs::read_to_string(&door.log).unwrap_or_default();
    assert!(!log.contains("the sealed state is refused"), "the seal was refused, so the device was a fresh one:\n{log}");
    let me: serde_json::Value = http.get(format!("{}/v2/me", door.base)).header("cookie", &c).send().await.unwrap().json().await.unwrap();
    assert_eq!(me["noncompliant"], serde_json::json!([]), "everything the restored device holds folds: {me}");
}

/// The seal counter the auth service holds for `pk`, on any node, read from its store.
fn seal_counter_of(auth: &Auth, pk: &str) -> Option<u64> {
    let out = Command::new("sqlite3")
        .arg(auth.root.join("auth.db"))
        .arg(format!("select max(counter) from seals where identity_pk = '{}'", pk.trim_start_matches("ed25519:")))
        .output()
        .expect("sqlite3");
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// O-74, point 3: /v2/signup/finish answers only once the person's state is sealed on the
/// Door's disk and the seal's counter confirmed at the auth service. Before, the first
/// seal came after the answer, and the state was nowhere on disk in between.
#[tokio::test(flavor = "multi_thread")]
async fn o74_a_sign_up_answers_only_once_its_seal_is_on_disk_and_confirmed() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    // AT THE ANSWER, not a tick later: nothing is waited for here.
    let seals: Vec<PathBuf> = std::fs::read_dir(&door.seals)
        .map(|d| d.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_none()).collect())
        .unwrap_or_default();
    assert_eq!(seals.len(), 1, "one seal on disk when the sign-up answered: {seals:?}");
    assert!(std::fs::metadata(&seals[0]).unwrap().len() > 0);
    assert!(seal_counter_of(&auth, &who.pk).is_some_and(|c| c >= 1), "and its counter confirmed at the auth service");
}

/// O-69, D-34 (c) off the actor: another writer's counter still ends the session. The auth
/// service's counter is moved past this process's, as a second machine's seal would move it;
/// the next write's confirmation, now asked of the auth worker after the answer, finds it
/// moved, and the session ends: its next read is refused, and the Door's log says why.
#[tokio::test(flavor = "multi_thread")]
async fn o69_another_writers_counter_still_ends_the_session_from_the_worker() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    assert!(seal_counter_of(&auth, &who.pk).is_some(), "the control: a counter at the service");
    let moved = Command::new("sqlite3")
        .arg(auth.root.join("auth.db"))
        .arg(format!("update seals set counter = counter + 10 where identity_pk = '{}'", who.pk.trim_start_matches("ed25519:")))
        .status()
        .expect("sqlite3");
    assert!(moved.success());
    let _ = mint_as(&http, &door, &who.cookie, "forum", "a write after another writer").await;
    let mut ended = false;
    for _ in 0..150 {
        let r = http.get(format!("{}/v2/me", door.base)).header("cookie", &who.cookie).send().await.unwrap();
        if r.status() != 200 {
            ended = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(ended, "the session outlived another writer's counter");
    let log = std::fs::read_to_string(&door.log).unwrap_or_default();
    assert!(log.contains("another writer; it ends"), "and the Door said why:\n{log}");
}

/// O-74, point 3, the crash it closes: the person's process killed the moment the sign-up
/// answers, before any tick. The next sign-in opens the SAME device from its seal, every
/// object already held, not a fresh device joining them (NC-67's fork class).
#[tokio::test(flavor = "multi_thread")]
async fn o74_killed_right_after_its_sign_up_the_next_sign_in_is_the_same_device() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let out = Command::new("pgrep").args(["-n", "-P", &door.child.id().to_string()]).output().expect("pgrep");
    let pid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(!pid.is_empty(), "the person's process");
    assert!(Command::new("kill").args(["-KILL", &pid]).status().unwrap().success());
    // Gone, or a zombie its supervisor has not waited on yet (the next sign-in reaps it).
    for _ in 0..100 {
        let st = Command::new("ps").args(["-o", "stat=", "-p", &pid]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        if st.is_empty() || st.starts_with('Z') {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let c = sign_in(&http, &door, &who).await;
    let me: serde_json::Value = http.get(format!("{}/v2/me", door.base)).header("cookie", &c).send().await.unwrap().json().await.unwrap();
    let objects = me["resume"]["objects"].as_array().cloned().unwrap_or_default();
    assert!(!objects.is_empty(), "the sign-in resumed the account's objects: {me}");
    assert!(
        objects.iter().all(|o| o["outcome"] == "AlreadyHeld"),
        "the same device, restored from its seal: every object already held, none joined anew: {objects:?}"
    );
    let log = std::fs::read_to_string(&door.log).unwrap_or_default();
    assert!(!log.contains("a fresh device"), "the seal was refused, so the device was a fresh one:\n{log}");
    assert_eq!(me["noncompliant"], serde_json::json!([]));
}

/// O-74, point 3, refused: a Door that cannot keep the new account's state refuses the
/// sign-up in words before the wrap is stored, so no account exists anywhere, and the
/// sign-up's process ends.
#[tokio::test(flavor = "multi_thread")]
async fn o74_a_sign_up_this_door_cannot_keep_is_refused_and_nothing_is_made() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let body = serde_json::json!({ "name": "security", "work": work(&http, &door, "signup").await });
    let o: serde_json::Value = http.post(format!("{}/v2/signup", door.base)).json(&body).send().await.unwrap().json().await.unwrap();
    let attempt = o["attempt"].as_str().unwrap().to_string();
    let pk = o["pk"].as_str().unwrap().trim_start_matches("ed25519:").to_string();
    // A FILE WHERE THE SEALS GO: no seal can be claimed, root or not (chmod binds no root).
    let _ = std::fs::remove_dir_all(&door.seals);
    std::fs::write(&door.seals, b"not a directory").unwrap();
    let r = http
        .post(format!("{}/v2/signup/finish", door.base))
        .json(&serde_json::json!({ "attempt": attempt, "sealed": seal(o["key"].as_str().unwrap(), &random32(), &attempt) }))
        .send()
        .await
        .unwrap();
    let (s, said) = said(r).await;
    std::fs::remove_file(&door.seals).unwrap();
    // A failure names what, never the body: on a red run it could carry words.
    assert_ne!(s, 200, "a sign-up this Door cannot keep answered 200");
    let v: serde_json::Value = serde_json::from_str(&said).unwrap_or_else(|_| serde_json::json!({ "error": said }));
    let why = v["error"].as_str().or(v["reason"].as_str()).unwrap_or(said.as_str());
    assert!(why.contains("No account was made: this Door cannot keep it (its seal could not be claimed)."), "refused, by its class alone");
    assert!(!said.contains(&door.seals.to_string_lossy().to_string()), "the refusal names no seals path");
    assert!(!said.contains(&pk) && !said.contains("http"), "the refusal names no key and no auth URL");
    assert!(v.get("words").is_none(), "no words for an account that does not exist");
    let out = Command::new("sqlite3").arg(auth.root.join("auth.db")).arg(format!("select count(*) from accounts where identity_pk = '{pk}'")).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "0", "no account at the auth service");
    for _ in 0..100 {
        if open_session_dirs(&door) == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(open_session_dirs(&door), 0, "the sign-up's process ended");
}

/// Software Security (a): a session whose actor panics ends at once, by session mode's
/// panic hook, instead of staying seated and answering nothing; the next sign-in reaps it
/// and opens the person's process again, from its seal. The panic is the debug build's
/// own hook (account.rs PANIC_FOR_TESTS, this name; a release build has none).
#[tokio::test(flavor = "multi_thread")]
async fn o74_a_session_whose_actor_panics_ends_and_the_next_sign_in_reopens_it() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let who = sign_up_as(&http, &door, "door-test: panic at kinds").await;
    let out = Command::new("pgrep").args(["-n", "-P", &door.child.id().to_string()]).output().expect("pgrep");
    let pid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(!pid.is_empty(), "the person's process");
    let r = http.get(format!("{}/v2/kinds", door.base)).header("cookie", &who.cookie).send().await.unwrap();
    assert!(!r.status().is_success(), "the panicking ask is not answered");
    // Ended: gone, or a zombie its supervisor has not waited on yet (the next sign-in reaps it).
    let ended = || {
        let st = Command::new("ps").args(["-o", "stat=", "-p", &pid]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        st.is_empty() || st.starts_with('Z')
    };
    let mut gone = false;
    for _ in 0..100 {
        if ended() {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(gone, "the panicking session's process ended");
    let log = std::fs::read_to_string(&door.log).unwrap_or_default();
    assert!(log.contains("door session: a panic at") && log.contains("a test's panic, debug builds only"), "it says where and what, a literal");

    let c = sign_in(&http, &door, &who).await;
    let me: serde_json::Value = http.get(format!("{}/v2/me", door.base)).header("cookie", &c).send().await.unwrap().json().await.unwrap();
    let objects = me["resume"]["objects"].as_array().cloned().unwrap_or_default();
    assert!(!objects.is_empty() && objects.iter().all(|o| o["outcome"] == "AlreadyHeld"), "reopened from its seal, the same device: {objects:?}");
}

/// An auth service that holds its seals, and the real one behind it, which calls itself
/// by this one's address: while `hold` is set, a request under
/// `/seals/` is accepted and never answered; every other request is passed to the real
/// one, one request to a connection (it is sent on with `connection: close`, so the
/// client opens another for the next). Its threads end with the test's process.
struct HoldingAuth {
    url: String,
    hold: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

fn holding_auth() -> (HoldingAuth, Auth) {
    let hold = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let h = hold.clone();
    let (url, auth) = proxied_auth(move |line| {
        if h.load(std::sync::atomic::Ordering::SeqCst) && line.contains("/seals/") {
            Pass::Hold
        } else {
            Pass::On
        }
    });
    (HoldingAuth { url, hold }, auth)
}

/// What an auth service in front of the real one does with a request, by its first line.
enum Pass {
    /// Passed to the real one.
    On,
    /// Accepted and never answered.
    Hold,
    /// Answered with this status, never passed on.
    Refuse(u16),
}

/// The real auth service, calling itself by the address of one in front of it that asks `on`
/// of each request: one request to a connection (it is sent on with `connection: close`, so
/// the client opens another for the next). Its threads end with the test's process.
fn proxied_auth(on: impl Fn(&str) -> Pass + Send + Sync + 'static) -> (String, Auth) {
    use std::io::{Read, Write};
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let auth = Auth::start_as(Some(&url));
    let up = auth.url.trim_start_matches("http://").to_string();
    let on = std::sync::Arc::new(on);
    std::thread::spawn(move || {
        for c in l.incoming() {
            let Ok(mut c) = c else { continue };
            let (up, on) = (up.clone(), on.clone());
            std::thread::spawn(move || {
                let (mut buf, mut tmp) = (Vec::new(), [0u8; 8192]);
                let end = loop {
                    match c.read(&mut tmp) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let head = String::from_utf8_lossy(&buf[..end]).to_string();
                let len: usize = head
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0)))
                    .unwrap_or(0);
                while buf.len() < end + len {
                    match c.read(&mut tmp) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                }
                match on(head.lines().next().unwrap_or("")) {
                    Pass::On => {}
                    // ACCEPTED, NEVER ANSWERED: held open until the caller gives up.
                    Pass::Hold => {
                        let _ = c.read(&mut tmp);
                        return;
                    }
                    Pass::Refuse(status) => {
                        let body = r#"{"error":"refused in front of the auth service"}"#;
                        let _ = write!(c, "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                        return;
                    }
                }
                let mut req: Vec<u8> = head
                    .lines()
                    .filter(|l| !l.is_empty() && !l.to_ascii_lowercase().starts_with("connection:"))
                    .map(|l| format!("{l}\r\n"))
                    .collect::<String>()
                    .into_bytes();
                req.extend_from_slice(b"connection: close\r\n\r\n");
                req.extend_from_slice(&buf[end..end + len]);
                let Ok(mut u) = std::net::TcpStream::connect(&up) else { return };
                if u.write_all(&req).is_err() {
                    return;
                }
                let _ = std::io::copy(&mut u, &mut c);
            });
        }
    });
    (url, auth)
}

/// Software Security (d): confirm_now gives up within its 20 s, well inside the
/// supervisor's 60 s, when the auth service accepts the seal's counter and never answers.
/// The counter is the step after the seal is written, so this is NC-89's path: the answer
/// is 2xx, the words with `saved: false` and the counter's class; the process ends; the
/// seal stays on disk, ahead of a counter the service never took, and it is what the next
/// sign-in opens: the same device.
#[tokio::test(flavor = "multi_thread")]
async fn o74_a_counter_the_auth_service_never_answers_gives_up_within_its_20_s() {
    let ((held, auth), relay) = (holding_auth(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", held.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let body = serde_json::json!({ "name": "security", "work": work(&http, &door, "signup").await });
    let o: serde_json::Value = http.post(format!("{}/v2/signup", door.base)).json(&body).send().await.unwrap().json().await.unwrap();
    let attempt = o["attempt"].as_str().unwrap().to_string();
    let prf = random32();
    let t = Instant::now();
    let r = http
        .post(format!("{}/v2/signup/finish", door.base))
        .json(&serde_json::json!({ "attempt": attempt, "sealed": seal(o["key"].as_str().unwrap(), &prf, &attempt) }))
        .send()
        .await
        .unwrap();
    let took = t.elapsed();
    let status = r.status().as_u16();
    let done: serde_json::Value = r.json().await.unwrap_or_default();
    // A failure names what, never the body: it carries words.
    assert!(took < Duration::from_secs(26), "answered in {took:?}: the cap is 20 s");
    assert!(took >= Duration::from_secs(15), "answered in {took:?}: the counter was not held");
    assert_eq!(status, 200, "made and not kept is a 2xx");
    assert_eq!(done["saved"], serde_json::json!(false));
    assert_eq!(done["why"], serde_json::json!("its seal's counter could not be confirmed"), "by its class alone");
    let pk = o["pk"].as_str().unwrap().trim_start_matches("ed25519:").to_string();
    assert_eq!(seal_counter_of(&auth, &pk), None, "the service never took the counter");
    let seals: Vec<PathBuf> = std::fs::read_dir(&door.seals)
        .map(|d| d.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_none()).collect())
        .unwrap_or_default();
    assert_eq!(seals.len(), 1, "the seal stays on disk, the only copy of the device");
    for _ in 0..100 {
        if open_session_dirs(&door) == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(open_session_dirs(&door), 0, "the sign-up's process ended");

    // The service answers again; the one next step opens the same device from that seal.
    held.hold.store(false, std::sync::atomic::Ordering::SeqCst);
    let who = Account { words: String::new(), pk: o["pk"].as_str().unwrap().to_string(), handle: o["handle"].as_str().unwrap().to_string(), prf, cookie: String::new() };
    let c = sign_in(&http, &door, &who).await;
    let me: serde_json::Value = http.get(format!("{}/v2/me", door.base)).header("cookie", &c).send().await.unwrap().json().await.unwrap();
    let objects = me["resume"]["objects"].as_array().cloned().unwrap_or_default();
    assert!(!objects.is_empty() && objects.iter().all(|o| o["outcome"] == "AlreadyHeld"), "the same device, from the seal: {objects:?}");
    assert!(seal_counter_of(&auth, &pk).is_some_and(|c| c >= 1), "and its counter confirmed now");
}

/// The wrap's store, by its request line: `PUT /auth/users/<pk>`, nothing after the key.
fn is_wrap(line: &str) -> bool {
    line.starts_with("PUT ") && line.split_whitespace().nth(1).and_then(|p| p.strip_prefix("/auth/users/")).is_some_and(|k| !k.contains('/'))
}

/// A seal's file for `pk` (ed25519 hex) in `seals`, and the one its next write goes to first.
fn seal_paths(seals: &std::path::Path, pk: &str) -> (PathBuf, PathBuf) {
    use sha2::Digest;
    let name = hex::encode(sha2::Sha256::digest(hex::decode(pk.trim_start_matches("ed25519:")).unwrap()));
    (seals.join(&name), seals.join(format!("{name}.next")))
}

/// An auth service in front of the real one that, as a wrap is stored, makes a directory
/// where the seal's next file goes (`at`, when set): the seal that follows cannot be written,
/// root or not. The first seal is on disk already by then (NC-132).
fn failing_after_the_wrap() -> (String, Auth, std::sync::Arc<std::sync::Mutex<Option<PathBuf>>>) {
    let at: std::sync::Arc<std::sync::Mutex<Option<PathBuf>>> = Default::default();
    let a = at.clone();
    let (url, auth) = proxied_auth(move |line| {
        if is_wrap(line) {
            if let Some(next) = a.lock().unwrap().as_ref() {
                std::fs::create_dir_all(next).unwrap();
            }
        }
        Pass::On
    });
    (url, auth, at)
}

/// A sign-up whose state this Door cannot keep once its wrap is stored (NC-89): the seal
/// after the wrap cannot be written (`failing_after_the_wrap`), so the claim held and the
/// first seal was written. The account, and what the Door answered.
async fn made_not_kept(http: &reqwest::Client, door: &Door, at: &std::sync::Mutex<Option<PathBuf>>) -> (Account, serde_json::Value) {
    let body = serde_json::json!({ "name": "security", "work": work(http, door, "signup").await });
    let o: serde_json::Value = http.post(format!("{}/v2/signup", door.base)).json(&body).send().await.unwrap().json().await.unwrap();
    let attempt = o["attempt"].as_str().unwrap().to_string();
    let pk = o["pk"].as_str().unwrap().trim_start_matches("ed25519:").to_string();
    let (_, next) = seal_paths(&door.seals, &pk);
    *at.lock().unwrap() = Some(next.clone());
    let prf = random32();
    let r = http
        .post(format!("{}/v2/signup/finish", door.base))
        .json(&serde_json::json!({ "attempt": attempt, "sealed": seal(o["key"].as_str().unwrap(), &prf, &attempt) }))
        .send()
        .await
        .unwrap();
    *at.lock().unwrap() = None;
    std::fs::remove_dir_all(&next).unwrap();
    let status = r.status().as_u16();
    let no_store = r.headers().get("cache-control").and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("no-store"));
    let cookie = cookie_of(&r);
    let done: serde_json::Value = r.json().await.unwrap_or_default();
    // A failure names what, never a word: the body is not quoted.
    assert_eq!(status, 200, "made and not kept is a 2xx");
    assert!(no_store, "the words are never cached");
    assert!(cookie.is_none(), "no session is seated on a process that is ending");
    assert_eq!(done["saved"], serde_json::json!(false), "marked unsaved");
    assert_eq!(done["why"], serde_json::json!("its seal could not be written"), "and why, by its class alone");
    let words = done["words"].as_str().unwrap_or_default().to_string();
    assert_eq!(words.split_whitespace().count(), 24, "the words are in the answer");
    (Account { words, pk: o["pk"].as_str().unwrap().to_string(), handle: o["handle"].as_str().unwrap().to_string(), prf, cookie: String::new() }, done)
}

/// NC-89 (UX, SECURITY): made, and not kept. The words go to the window once, in the 2xx
/// answer, and nowhere else: not the refusal record, not the Door's log (SEC-A4's
/// measure: a word counted in its owner's half and the other's). The one next step, a
/// sign-in with the same passkey, opens the account.
#[tokio::test(flavor = "multi_thread")]
async fn nc89_made_and_not_kept_the_words_go_to_the_window_and_nowhere_else() {
    let ((front, _auth, next_at), relay) = (failing_after_the_wrap(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", front.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let record = || std::fs::read_to_string(door.root.join("refusals.jsonl")).unwrap_or_default();
    let log = || std::fs::read_to_string(&door.log).unwrap_or_default();
    let at = || (record().lines().count(), log().lines().count());
    let (r0, l0) = at();
    let (a, _) = made_not_kept(&http, &door, &next_at).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    let (r1, l1) = at();
    let (b, _) = made_not_kept(&http, &door, &next_at).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    let (rec, lg) = (record(), log());
    let (rl, ll): (Vec<&str>, Vec<&str>) = (rec.lines().collect(), lg.lines().collect());
    assert!(ll[l0..l1].iter().any(|l| l.contains("could not keep it")), "the Door says what happened, in its own words");
    let count = |part: &[&str], w: &str| -> usize {
        part.iter().map(|l| l.split(|c: char| !c.is_ascii_alphabetic()).filter(|t| t.eq_ignore_ascii_case(w)).count()).sum()
    };
    for (what, lines, (s0, s1)) in [("the refusal record", &rl, (r0, r1)), ("the Door's log", &ll, (l0, l1))] {
        let (in_a, in_b) = (&lines[s0.min(lines.len())..s1.min(lines.len())], &lines[s1.min(lines.len())..]);
        for (owner, who, mine, theirs) in [("A", &a, in_a, in_b), ("B", &b, in_b, in_a)] {
            assert!(!mine.iter().any(|l| l.contains(who.words.as_str())), "{owner}'s words, whole, in {what}");
            for (k, w) in who.words.split_whitespace().enumerate() {
                let (m, t) = (count(mine, w), count(theirs, w));
                assert!(m == t, "recovery word #{} of {owner} is in {what}: {m} times in {owner}'s part, {t} in the other's", k + 1);
            }
        }
    }
    // THE ONE NEXT STEP: a sign-in with the same passkey.
    let c = sign_in(&http, &door, &a).await;
    let (s, pk) = me_on(&http, &door, &c).await;
    assert_eq!(s, 200, "the sign-in opens the account");
    assert_eq!(pk, a.pk);
}

/// A VOLUME FOR THE SEALS THAT FILLS (NC-132): a 2 MiB disk image, attached where nothing
/// browses it, filled to ENOSPC on `fill` in shrinking blocks (a 64 KiB write leaves the last
/// few free). A file already on it still opens; no new one can be made and nothing more
/// written. Detached on drop. macOS's hdiutil, with no root.
#[cfg(target_os = "macos")]
struct Volume {
    image: PathBuf,
    mount: PathBuf,
}

#[cfg(target_os = "macos")]
impl Volume {
    fn new() -> Volume {
        let root = std::env::temp_dir().join(format!("door-nc132-{}", unique()));
        std::fs::create_dir_all(&root).unwrap();
        let (image, mount) = (root.join("seals.dmg"), root.join("mnt"));
        let ok = |args: &[&str]| assert!(Command::new("hdiutil").args(args).status().unwrap().success(), "hdiutil {args:?}");
        ok(&["create", "-quiet", "-size", "2m", "-fs", "HFS+", "-volname", "nc132", "-layout", "NONE", image.to_str().unwrap()]);
        ok(&["attach", "-quiet", "-nobrowse", "-mountpoint", mount.to_str().unwrap(), image.to_str().unwrap()]);
        std::fs::create_dir_all(mount.join("seals")).unwrap();
        Volume { image, mount }
    }

    fn seals(&self) -> PathBuf {
        self.mount.join("seals")
    }

    fn fill(&self) {
        fill(&self.mount)
    }
}

/// The volume at `mount`, filled to ENOSPC.
#[cfg(target_os = "macos")]
fn fill(mount: &std::path::Path) {
    use std::io::Write;
    // One file, in shrinking blocks: on a full volume not even a new empty file can be made.
    let mut f = std::fs::File::create(mount.join("fill")).unwrap();
    for bs in [65536usize, 4096, 512, 1] {
        let zeros = vec![0u8; bs];
        while f.write_all(&zeros).and_then(|_| f.sync_all()).is_ok() {}
    }
    assert!(std::fs::write(mount.join("one-more"), b"x").is_err(), "the volume is full");
}

#[cfg(target_os = "macos")]
impl Drop for Volume {
    fn drop(&mut self) {
        let _ = Command::new("hdiutil").args(["detach", "-quiet", "-force", self.mount.to_str().unwrap()]).status();
        let _ = std::fs::remove_dir_all(self.image.parent().unwrap());
    }
}

/// A sign-up begun, and its key: `(attempt, pk hex, the key the PRF is sealed to)`.
async fn signup_begun(http: &reqwest::Client, door: &Door) -> (String, String, String) {
    let body = serde_json::json!({ "name": "security", "work": work(http, door, "signup").await });
    let o: serde_json::Value = http.post(format!("{}/v2/signup", door.base)).json(&body).send().await.unwrap().json().await.unwrap();
    (o["attempt"].as_str().unwrap().into(), o["pk"].as_str().unwrap().trim_start_matches("ed25519:").into(), o["key"].as_str().unwrap().into())
}

fn accounts_at(auth: &Auth, pk: &str) -> String {
    let out = Command::new("sqlite3").arg(auth.root.join("auth.db")).arg(format!("select count(*) from accounts where identity_pk = '{pk}'")).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

async fn no_session_left(door: &Door) {
    for _ in 0..100 {
        if open_session_dirs(door) == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(open_session_dirs(door), 0, "the sign-up's process ended");
}

/// NC-132 (TEST, run 96): a Door whose disk has no room left for the new account's seal
/// refuses the sign-up before its wrap is stored, 5xx: no account exists anywhere, so a retry
/// is right. As on door-test, the claim holds (its lock file opens) and the seal cannot be
/// written. Before, the wrap came first and the answer was NC-89's made-not-kept 2xx.
#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread")]
async fn nc132_a_sign_up_with_no_room_for_its_seal_is_refused_before_any_account_exists() {
    let (auth, relay, vol) = (Auth::start(), Relay::start(), Volume::new());
    let seals = vol.seals();
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_SEALS", seals.to_str().unwrap())]);
    let http = door.http();
    let (attempt, pk, key) = signup_begun(&http, &door).await;
    let (seal_file, next) = seal_paths(&seals, &pk);
    std::fs::write(seal_file.with_extension("lock"), b"").unwrap();
    vol.fill();
    let r = http
        .post(format!("{}/v2/signup/finish", door.base))
        .json(&serde_json::json!({ "attempt": attempt, "sealed": seal(&key, &random32(), &attempt) }))
        .send()
        .await
        .unwrap();
    let cookie = cookie_of(&r);
    let (s, said) = said(r).await;
    // A failure names what, never the body: on a red run it could carry words.
    assert!((500..600).contains(&s), "a sign-up this Door has no room to keep answered {s}");
    let v: serde_json::Value = serde_json::from_str(&said).unwrap_or_else(|_| serde_json::json!({ "error": said }));
    let why = v["error"].as_str().or(v["reason"].as_str()).unwrap_or(said.as_str());
    assert!(why.contains("No account was made: this Door cannot keep it (its seal could not be written)."), "refused, by its class alone");
    assert!(v.get("words").is_none(), "no words for an account that does not exist");
    assert!(cookie.is_none(), "no session");
    assert_eq!(accounts_at(&auth, &pk), "0", "no wrap reached the auth service");
    assert!(!seal_file.exists() && !next.exists(), "no seal left on the disk");
    no_session_left(&door).await;
}

/// NC-132's control: the disk fills AFTER the first seal, as the wrap is stored. The account
/// exists, and it is NC-89's answer, as ruled: 2xx, the words, `saved: false`.
#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread")]
async fn nc132_a_disk_that_fills_after_the_first_seal_keeps_nc89s_answer() {
    let vol = Volume::new();
    let full = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (mount, f) = (vol.mount.clone(), full.clone());
    let (front, _auth) = proxied_auth(move |line| {
        if is_wrap(line) && !f.swap(true, std::sync::atomic::Ordering::SeqCst) {
            fill(&mount);
        }
        Pass::On
    });
    let relay = Relay::start();
    let seals = vol.seals();
    let door = Door::start_with(&[("DOOR_AUTH", front.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_SEALS", seals.to_str().unwrap())]);
    let http = door.http();
    let (attempt, _pk, key) = signup_begun(&http, &door).await;
    let r = http
        .post(format!("{}/v2/signup/finish", door.base))
        .json(&serde_json::json!({ "attempt": attempt, "sealed": seal(&key, &random32(), &attempt) }))
        .send()
        .await
        .unwrap();
    let (status, cookie) = (r.status().as_u16(), cookie_of(&r));
    let done: serde_json::Value = r.json().await.unwrap_or_default();
    assert!(full.load(std::sync::atomic::Ordering::SeqCst), "the wrap was stored, and the disk filled then");
    assert_eq!(status, 200, "made and not kept is a 2xx");
    assert!(cookie.is_none(), "no session is seated on a process that is ending");
    assert_eq!(done["saved"], serde_json::json!(false), "marked unsaved");
    assert_eq!(done["why"], serde_json::json!("its seal could not be written"));
    assert_eq!(done["words"].as_str().unwrap_or_default().split_whitespace().count(), 24, "the words are in the answer");
}

/// NC-132: a wrap the auth service does not store, after the first seal is written, leaves
/// nothing behind: the seal goes, the claim ends with the process, and the next sign-up
/// starts clean.
#[tokio::test(flavor = "multi_thread")]
async fn nc132_a_wrap_not_stored_leaves_no_seal_behind() {
    let refuse = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let r0 = refuse.clone();
    let (front, auth) = proxied_auth(move |line| {
        if is_wrap(line) && r0.load(std::sync::atomic::Ordering::SeqCst) {
            Pass::Refuse(503)
        } else {
            Pass::On
        }
    });
    let relay = Relay::start();
    let door = Door::start_with(&[("DOOR_AUTH", front.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let (attempt, pk, key) = signup_begun(&http, &door).await;
    let r = http
        .post(format!("{}/v2/signup/finish", door.base))
        .json(&serde_json::json!({ "attempt": attempt, "sealed": seal(&key, &random32(), &attempt) }))
        .send()
        .await
        .unwrap();
    let cookie = cookie_of(&r);
    let (s, _) = said(r).await;
    assert!(!(200..300).contains(&s), "a wrap not stored answered {s}");
    assert!(cookie.is_none(), "no session");
    assert_eq!(accounts_at(&auth, &pk), "0", "no account at the auth service");
    let (seal_file, next) = seal_paths(&door.seals, &pk);
    assert!(!seal_file.exists() && !next.exists(), "no seal left on the disk");
    // The claim is not held: this person's lock is free to take.
    let lock = std::fs::OpenOptions::new().write(true).create(true).truncate(false).open(seal_file.with_extension("lock")).unwrap();
    let mut free = false;
    for _ in 0..100 {
        // SAFETY: flock on a descriptor this test owns.
        if unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&lock), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            free = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(free, "the claim was released");
    drop(lock);
    // A retry starts clean.
    refuse.store(false, std::sync::atomic::Ordering::SeqCst);
    let who = sign_up(&http, &door).await;
    assert_eq!(me_on(&http, &door, &who.cookie).await.0, 200, "the next sign-up");
}

/// NC-63: an attempt whose process has died is answered, promptly, with no sweep, and
/// the person's next sign-in is not held up by it.
#[tokio::test(flavor = "multi_thread")]
async fn d34_nc63_an_attempt_whose_process_died_is_answered() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    assert_eq!(sign_out(&http, &door, &who.cookie).await, 200);
    let (a, k) = attempt(&http, &door).await;
    // The attempt's process: the Door's newest child.
    let out = Command::new("pgrep").args(["-n", "-P", &door.child.id().to_string()]).output().expect("pgrep");
    let pid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(!pid.is_empty(), "the attempt's process");
    assert!(Command::new("kill").args(["-KILL", &pid]).status().unwrap().success());
    let t = Instant::now();
    let r = finish(&http, &door, serde_json::json!({ "attempt": a, "handle": who.handle, "sealed": seal(&k, &who.prf, &a) })).await;
    assert!(!r.status().is_success() && cookie_of(&r).is_none(), "a dead attempt opened a session: {}", r.status());
    assert!(t.elapsed() < Duration::from_secs(30), "answered after {:?}", t.elapsed());
    let c = sign_in(&http, &door, &who).await;
    assert_eq!(me_on(&http, &door, &c).await.0, 200, "the next sign-in");
}

/// NC-55 in one process: two public-site sessions of one person, each held to the Site,
/// each sees what it minted there and not what the other did.
#[tokio::test(flavor = "multi_thread")]
async fn d34_nc55_two_site_sessions_in_one_process_keep_their_own_mints() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let (who, site) = {
        let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
        let http = door.http();
        let who = sign_up(&http, &door).await;
        let site = mint_as(&http, &door, &who.cookie, "group", "the Site").await;
        end_session(&http, &door, &who.cookie).await;
        (who, site)
    };
    let clients = auth.root.join("clients-d34.json");
    let reg = serde_json::json!({ CLIENT: { "name": "", "callbacks": [CALLBACK], "origins": ["https://client.example.test"], "site": site } });
    std::fs::write(&clients, reg.to_string()).unwrap();
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap()),
    ]);
    let http = door.http();
    let token_url = format!("{}/v2/token", door.base);
    let mut held = Vec::new();
    for _ in 0..2 {
        let key = Dpop::new();
        let (code, verifier) = a_code(&http, &door, &who).await;
        let r = exchange(&http, &door, Some(key.proof("POST", &token_url, None)), &code, &verifier, CLIENT).await;
        let token = r.json::<serde_json::Value>().await.unwrap()["access_token"].as_str().expect("a token").to_string();
        held.push((key, token));
    }
    assert_eq!(open_session_dirs(&door), 1, "both sites' sessions in the person's one process");
    let as_token = |(key, token): &(Dpop, String), method: &str, path: &str, body: Option<serde_json::Value>| {
        let url = format!("{}{path}", door.base);
        let mut rq = http
            .request(method.parse().unwrap(), &url)
            .header("Authorization", format!("DPoP {token}"))
            .header("DPoP", key.proof(method, &url, Some(token)));
        if let Some(b) = body {
            rq = rq.json(&b);
        }
        async move { rq.send().await.unwrap().json::<serde_json::Value>().await.unwrap() }
    };
    let minted = as_token(&held[0], "POST", "/v2/mint", Some(serde_json::json!({ "kind": "group", "draft": { "name": "the first's" } }))).await;
    let mine = minted["object_id"].as_str().expect("the first session mints").to_string();
    // The graph, live: `/v2/me`'s resume is the sign-in's, from before the mint.
    let ids = |g: &serde_json::Value| -> Vec<String> {
        g["objects"].as_array().expect("/v2/graph lists objects").iter().filter_map(|o| o["id"].as_str().map(str::to_string)).collect()
    };
    let first = ids(&as_token(&held[0], "GET", "/v2/graph", None).await);
    let second = ids(&as_token(&held[1], "GET", "/v2/graph", None).await);
    assert!(first.contains(&site) && first.contains(&mine), "the first sees the Site and its own mint: {first:?}");
    assert!(second.contains(&site) && !second.contains(&mine), "the second sees the Site, and not the first's mint: {second:?}");
}

// ─── O-58: the passkey and the account are the Door's own host's (D-33, re-ruled 27 Sep) ──

/// A Door started with `env` stops before it listens, and says why on stderr: its exit
/// status and that text. Killed after 60 s if it does not stop.
fn refused_at_start(env: &[(&str, &str)]) -> (Option<i32>, String) {
    let root = std::env::temp_dir().join(format!("door-security-refused-{}", unique()));
    let mut child = Command::new(env!("CARGO_BIN_EXE_door"))
        .env("DOOR_PORT", free_port().to_string())
        .env("DOOR_ROOT", &root)
        .env_remove("DOOR_RELAY_BIN")
        .envs(env.iter().copied())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the door image");
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s.code();
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&root);
            panic!("the Door started with {env:?} and did not stop");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let mut err = String::new();
    std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut err).unwrap();
    let _ = std::fs::remove_dir_all(&root);
    (status, err)
}

/// No registration carries the whole account's scope, the website's own included, and
/// the RP ID is the Door's own host: either stops the Door at start, named (O-58;
/// Software Security: fail closed).
#[test]
fn o58_no_account_scope_and_no_other_rp_id_start_a_door() {
    let dir = std::env::temp_dir().join(format!("door-security-o58-{}", unique()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = |origin: &str| serde_json::json!({ "name": "x", "callbacks": [CALLBACK], "origins": [origin], "site": "", "scope": "account" });
    for (name, reg, named) in [
        ("www", serde_json::json!({ "www.wallflowers.io": entry("https://www.wallflowers.io") }), "\"www.wallflowers.io\""),
        ("other-origin", serde_json::json!({ "evil": entry("https://evil.example") }), "\"evil\""),
    ] {
        let path = dir.join(format!("{name}.json"));
        std::fs::write(&path, reg.to_string()).unwrap();
        let (status, err) = refused_at_start(&[("DOOR_CLIENTS", path.to_str().unwrap())]);
        assert_ne!(status, Some(0), "{name}: the Door started");
        assert!(err.contains(named), "{name}: the refusal does not name {named}: {err}");
    }
    for (public, rp) in [("https://app.wallflowers.io", "wallflowers.io"), ("https://app.wallflowers.io", "www.wallflowers.io"), ("http://localhost:8233", "wallflowers.io")] {
        let (status, err) = refused_at_start(&[("DOOR_PUBLIC", public), ("DOOR_RP_ID", rp)]);
        assert_ne!(status, Some(0), "{public} with the RP ID {rp}: the Door started");
        assert!(err.contains("DOOR_RP_ID") && err.contains(rp), "{public}, {rp}: the refusal does not name it: {err}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The window's ceremony names the Door's own host as its RP ID, unset or set to it: no
/// page on another host can ask for this passkey (O-58).
#[tokio::test(flavor = "multi_thread")]
async fn o58_the_windows_ceremony_names_the_doors_own_host() {
    for extra in [&[("DOOR_PUBLIC", "https://app.wallflowers.io")][..], &[("DOOR_PUBLIC", "https://app.wallflowers.io"), ("DOOR_RP_ID", "app.wallflowers.io")][..]] {
        let door = Door::start_with(extra);
        // Behind an edge the Door's public origin is the edge's, and so is its RP ID.
        let own = if door.edge.is_some() { "localhost" } else { "app.wallflowers.io" };
        let page = door.http().get(format!("{}/signin", door.base)).send().await.unwrap().text().await.unwrap();
        assert!(page.contains(&format!("<meta name=\"rp-id\" content=\"{own}\">")), "{extra:?}: the window's RP ID is not {own}");
    }
}

// ─── O-48: a Site's own events and posts reach its Host, through the Door ─────────

/// Through /v2 as the webapp writes: a Site and its Host; an event and a post named at
/// both ends (the Site's `group.setAffiliation {rel: created}`, the object's own
/// `base.setBacklink`); a post named by the Site only. No call asks for the Host's items:
/// the Door's own settle after each write puts them there, and a retraction takes one off.
#[tokio::test(flavor = "multi_thread")]
async fn o48_a_sites_event_and_post_reach_its_host_through_the_door() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let c = who.cookie.as_str();
    let at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
    let apply = |object: String, op: &'static str, args: serde_json::Value| {
        let rq = http
            .post(format!("{}/v2/apply", door.base))
            .header("cookie", c)
            .header("origin", LISTED)
            .json(&serde_json::json!({ "object": object, "op": op, "args": args }));
        async move {
            let (s, body) = said(rq.send().await.unwrap()).await;
            assert_eq!(s, 200, "{op}: {body}");
        }
    };
    let mint = |kind: &'static str, draft: serde_json::Value| {
        let rq = http
            .post(format!("{}/v2/mint", door.base))
            .header("cookie", c)
            .header("origin", LISTED)
            .json(&serde_json::json!({ "kind": kind, "draft": draft }));
        async move {
            let (s, body) = said(rq.send().await.unwrap()).await;
            assert_eq!(s, 200, "/v2/mint {kind}: {body}");
            serde_json::from_str::<serde_json::Value>(&body).unwrap()["object_id"].as_str().unwrap().to_string()
        }
    };
    let host_items = || {
        let rq = http.get(format!("{}/v2/graph", door.base)).header("cookie", c);
        async move {
            let g: serde_json::Value = rq.send().await.unwrap().json().await.unwrap();
            let host = g["objects"].as_array().unwrap().iter().find(|o| o["kind"] == "host").expect("the Host in the graph").clone();
            host["view"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| (i["key"].as_str().unwrap().to_string(), i["rev"].as_i64().unwrap(), serde_json::from_str::<serde_json::Value>(i["payload"].as_str().unwrap()).unwrap()))
                .collect::<Vec<_>>()
        }
    };

    let site = mint("group", serde_json::json!({ "name": "Mill Road Allotments", "shape": "community" })).await;
    let host = mint("host", serde_json::json!({ "name": "Mill Road Allotments" })).await;
    apply(site.clone(), "base.setPart", serde_json::json!({ "part": host, "role": "host", "at": at })).await;
    apply(host.clone(), "base.setParent", serde_json::json!({ "parent": site, "role": "host", "at": at })).await;
    assert!(host_items().await.is_empty(), "nothing on the Host before the Site names anything");

    let both = |object: String, name: &'static str, at: i64| {
        let (site, apply) = (site.clone(), &apply);
        async move {
            apply(site.clone(), "group.setAffiliation", serde_json::json!({ "peer": object, "rel": "created", "name": name, "at": at })).await;
            apply(object, "base.setBacklink", serde_json::json!({ "object": site, "rel": "created", "at": at })).await;
        }
    };
    let event = mint("event", serde_json::json!({ "name": "Dig day", "start_ms": at + 86_400_000, "venue": "The shed" })).await;
    both(event.clone(), "Dig day", at).await;
    let post = mint("post", serde_json::json!({ "name": "Seed swap notes", "descriptor": "Bring spares." })).await;
    both(post.clone(), "Seed swap notes", at + 1).await;
    let lone = mint("post", serde_json::json!({ "name": "Half declared" })).await;
    apply(site.clone(), "group.setAffiliation", serde_json::json!({ "peer": lone, "rel": "created", "name": "Half declared", "at": at + 2 })).await;

    let items = host_items().await;
    let keys: std::collections::BTreeSet<&str> = items.iter().map(|(k, _, _)| k.as_str()).collect();
    // EXACTLY THE KEYS THE ICD DECLARES (host.hydrate `key`, 2.3.1): the event, the post, and the
    // post's public page, its body in chunks from 0 (W-98 Resources, Ralph's 3). Its body here
    // is one chunk; it has no asset and no document, so neither key is there. Nothing else.
    assert!(keys.iter().all(|k| declared(k)), "a key the ICD does not declare: {keys:?}");
    assert!(!declared(&format!("post:{post}:junk")) && !declared("post:") && !declared(&format!("event:{event}:body:0")), "the rule refuses what is undeclared");
    assert_eq!(
        keys,
        [format!("event:{event}"), format!("post:{post}"), format!("post:{post}:body:0")].iter().map(String::as_str).collect(),
        "{items:?}"
    );
    let chunk = &items.iter().find(|(k, _, _)| k.ends_with(":body:0")).unwrap().2;
    assert_eq!(chunk["text"], "Bring spares.", "the body, whole, in its one chunk");
    let ev = &items.iter().find(|(k, _, _)| k.starts_with("event:")).unwrap().2;
    assert_eq!((ev["title"].as_str(), ev["venue"].as_str()), (Some("Dig day"), Some("The shed")));
    let log = std::fs::read_to_string(&door.log).unwrap_or_default();
    assert!(log.contains(&format!("names {lone}, left off its Face: the Site names it, and it does not name the Site")), "the half-declared post is named in the log:\n{log}");

    // NOTHING MOVED: a write elsewhere leaves every item at its rev.
    mint("group", serde_json::json!({ "name": "Elsewhere" })).await;
    assert_eq!(host_items().await, items, "an unchanged Site writes nothing to its Host");

    // RETRACTED: off the Host with the write that retracted it.
    apply(post.clone(), "post.retract", serde_json::json!({})).await;
    let after: Vec<String> = host_items().await.into_iter().map(|(k, _, _)| k).collect();
    assert_eq!(after, vec![format!("event:{event}")], "the retracted post, and its page, are withdrawn");
}

/// A Host item key the ICD declares (host.hydrate's `key`, 2.3.1): `face`, `event:<id>`,
/// `post:<id>`, and a post's public page, `post:<id>:body:<n>`, `post:<id>:asset:<assetId>`
/// and `post:<id>:document`; an id 64 hex, an asset's 16, n a number.
fn declared(key: &str) -> bool {
    let hex = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_hexdigit());
    let parts: Vec<&str> = key.split(':').collect();
    match parts.as_slice() {
        ["face"] => true,
        ["event", id] | ["post", id] => hex(id, 64),
        ["post", id, "body", n] => hex(id, 64) && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()),
        ["post", id, "asset", a] => hex(id, 64) && hex(a, 16),
        ["post", id, "document"] => hex(id, 64),
        _ => false,
    }
}

// ─── /v2/token's bounds, a token's sign-out, a cookie write forwarded (Software Testing) ──
// From the code check of the Door API (27 Sep): what no test above reached.

/// A Door with the client registered, as the step-5 tests start one.
fn door_for_a_site(auth: &Auth, relay: &Relay, extra: &[(&str, &str)]) -> Door {
    let clients = clients_file(&auth.root);
    let mut env = vec![("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap())];
    env.extend_from_slice(extra);
    Door::start_with(&env)
}

/// A code for the registered callback, and its verifier.
async fn a_code(http: &reqwest::Client, door: &Door, who: &Account) -> (String, String) {
    let (verifier, challenge) = pkce();
    let r = client_sign_in(http, door, who, CALLBACK, &challenge).await;
    (code_of(r.json::<serde_json::Value>().await.unwrap()["redirect"].as_str().unwrap()), verifier)
}

/// A refusal's status and the Door's reason (its body).
async fn said(r: reqwest::Response) -> (u16, String) {
    let s = r.status().as_u16();
    (s, r.text().await.unwrap_or_default())
}

/// SEC-39, RFC 9449 §11.1: a proof at /v2/token dated more than a minute from the Door's
/// clock, either way, is refused, and spends nothing: its code still redeems.
#[tokio::test(flavor = "multi_thread")]
async fn dt20_sec39_a_proof_outside_its_minute_is_refused_at_token() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = door_for_a_site(&auth, &relay, &[]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let key = Dpop::new();
    let url = format!("{}/v2/token", door.base);
    let (code, verifier) = a_code(&http, &door, &who).await;
    let now = now_secs();
    for (label, iat) in [("two minutes old", now - 120), ("two minutes ahead", now + 120)] {
        let proof = key.proof_at("POST", &url, None, iat, &hex::encode(random32()));
        let (s, why) = said(exchange(&http, &door, Some(proof), &code, &verifier, CLIENT).await).await;
        assert!(s == 400 && why.contains("stale"), "a proof {label}: {s} {why}");
    }
    let proof = key.proof_at("POST", &url, None, now - 30, &hex::encode(random32()));
    let (s, why) = said(exchange(&http, &door, Some(proof), &code, &verifier, CLIENT).await).await;
    assert_eq!(s, 200, "the same code, its proof half a minute old: {why}");
}

/// SEC-39, RFC 9449 §11.1: at /v2/token a proof is spent once. Presented again, with
/// another code, it is refused, and that code is not spent by the refusal.
#[tokio::test(flavor = "multi_thread")]
async fn dt20_sec39_a_proof_is_spent_once_at_token() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = door_for_a_site(&auth, &relay, &[]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let key = Dpop::new();
    let url = format!("{}/v2/token", door.base);
    let (c1, v1) = a_code(&http, &door, &who).await;
    let (c2, v2) = a_code(&http, &door, &who).await;
    let proof = key.proof("POST", &url, None);
    let (s, why) = said(exchange(&http, &door, Some(proof.clone()), &c1, &v1, CLIENT).await).await;
    assert_eq!(s, 200, "the proof's first use: {why}");
    let (s, why) = said(exchange(&http, &door, Some(proof), &c2, &v2, CLIENT).await).await;
    assert!(s == 400 && why.contains("already used"), "the same proof, with another code: {s} {why}");
    let (s, why) = said(exchange(&http, &door, Some(key.proof("POST", &url, None)), &c2, &v2, CLIENT).await).await;
    assert_eq!(s, 200, "that code, with a fresh proof: {why}");
}

/// SEC-A2 at /v2/token: one address holds at most 60 live proofs; the next is refused
/// before any code is looked at, and another address is not held to the first's count.
#[tokio::test(flavor = "multi_thread")]
async fn seca2_token_proofs_are_bounded_per_address() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = door_for_a_site(&auth, &relay, &[("DOOR_TRUST_FORWARDED", "1")]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let key = Dpop::new();
    let url = format!("{}/v2/token", door.base);
    let from = |ip: &'static str, proof: String, code: String, verifier: String| {
        http.post(&url)
            .header("CF-Connecting-IP", ip)
            .header("DPoP", proof)
            .json(&serde_json::json!({ "code": code, "code_verifier": verifier, "client": CLIENT, "redirect_uri": CALLBACK }))
            .send()
    };
    let nothing = || ("no-such-code".to_string(), pkce().0);
    for i in 1..=60 {
        let (code, verifier) = nothing();
        let (s, why) = said(from("192.0.2.1", key.proof("POST", &url, None), code, verifier).await.unwrap()).await;
        assert!(s == 400 && why.contains("no such code"), "proof {i} from one address, taken, its code looked at: {s} {why}");
    }
    let (code, verifier) = nothing();
    let (s, why) = said(from("192.0.2.1", key.proof("POST", &url, None), code, verifier).await.unwrap()).await;
    assert!(s == 400 && why.contains("too many"), "proof 61 from one address: {s} {why}");
    let (code, verifier) = a_code(&http, &door, &who).await;
    let (s, why) = said(from("198.51.100.1", key.proof("POST", &url, None), code, verifier).await.unwrap()).await;
    assert_eq!(s, 200, "a real code from another address: {why}");
}

/// DT-11: a code lives one minute. Redeemed after it, with its verifier and a good
/// proof, it is refused; a fresh code, redeemed at once, is not.
#[tokio::test(flavor = "multi_thread")]
async fn dt11_a_code_expires_after_its_minute() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = door_for_a_site(&auth, &relay, &[]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let key = Dpop::new();
    let url = format!("{}/v2/token", door.base);
    let (code, verifier) = a_code(&http, &door, &who).await;
    tokio::time::sleep(Duration::from_secs(62)).await;
    let (s, why) = said(exchange(&http, &door, Some(key.proof("POST", &url, None)), &code, &verifier, CLIENT).await).await;
    assert!(s == 400 && (why.contains("expired") || why.contains("no such code")), "a code 62 s old: {s} {why}");
    let (code, verifier) = a_code(&http, &door, &who).await;
    let (s, why) = said(exchange(&http, &door, Some(key.proof("POST", &url, None)), &code, &verifier, CLIENT).await).await;
    assert_eq!(s, 200, "a fresh code: {why}");
}

/// SEC-39 at /v2/signout, NC-57: a token signs out only with a proof by its key, for
/// that route. Refused, it is told so and stays live; with its proof it is revoked.
#[tokio::test(flavor = "multi_thread")]
async fn nc57_sec39_a_token_signs_out_only_with_its_proof() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = door_for_a_site(&auth, &relay, &[]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let key = Dpop::new();
    let (code, verifier) = a_code(&http, &door, &who).await;
    let r = exchange(&http, &door, Some(key.proof("POST", &format!("{}/v2/token", door.base), None)), &code, &verifier, CLIENT).await;
    assert_eq!(r.status().as_u16(), 200, "the exchange");
    let token = r.json::<serde_json::Value>().await.unwrap()["access_token"].as_str().unwrap().to_string();
    let (out, me) = (format!("{}/v2/signout", door.base), format!("{}/v2/me", door.base));
    let live = || {
        http.get(&me).header("Authorization", format!("DPoP {token}")).header("DPoP", key.proof("GET", &me, Some(&token))).send()
    };
    assert_eq!(live().await.unwrap().status().as_u16(), 200, "the control: the token answers");
    let refused: [(&str, Option<String>); 3] = [
        ("no proof", None),
        ("another key's proof", Some(Dpop::new().proof("POST", &out, Some(&token)))),
        ("a proof for /v2/me", Some(key.proof("POST", &me, Some(&token)))),
    ];
    for (label, proof) in refused {
        let mut rq = http.post(&out).header("Authorization", format!("DPoP {token}"));
        if let Some(p) = proof {
            rq = rq.header("DPoP", p);
        }
        let (s, why) = said(rq.send().await.unwrap()).await;
        assert_eq!(s, 401, "sign-out with {label}: {why}");
        assert_eq!(live().await.unwrap().status().as_u16(), 200, "sign-out with {label} revoked the token");
    }
    let r = http.post(&out).header("Authorization", format!("DPoP {token}")).header("DPoP", key.proof("POST", &out, Some(&token))).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200, "sign-out with the token's proof");
    assert_eq!(live().await.unwrap().status().as_u16(), 401, "signed out, the token still answers");
}

/// SEC-8, DV-9 at a session route: a cookie write forwarded to the session (/v2/mint)
/// from an origin the Door does not list, or with none, is refused 403 and mints
/// nothing. The listed origin mints: the control, first.
#[tokio::test(flavor = "multi_thread")]
async fn dv9_a_forwarded_cookie_write_needs_a_listed_origin() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let held = || async {
        let g: serde_json::Value = http.get(format!("{}/v2/graph", door.base)).header("cookie", &who.cookie).send().await.unwrap().json().await.unwrap();
        g["objects"].as_array().map(Vec::len).expect("/v2/graph lists objects")
    };
    let mint = |origin: Option<&str>| {
        let mut rq = http
            .post(format!("{}/v2/mint", door.base))
            .header("cookie", &who.cookie)
            .json(&serde_json::json!({ "kind": "group", "draft": { "name": "dv9" } }));
        if let Some(o) = origin {
            rq = rq.header("origin", o);
        }
        rq.send()
    };
    let before = held().await;
    let (s, why) = said(mint(Some(LISTED)).await.unwrap()).await;
    assert_eq!(s, 200, "the control: a mint from the listed origin: {why}");
    assert_eq!(held().await, before + 1, "the control's mint is not in the graph");
    for origin in [Some(STRANGER), Some("null"), None] {
        let (s, why) = said(mint(origin).await.unwrap()).await;
        assert_eq!(s, 403, "a forwarded cookie write with Origin {origin:?}: {why}");
        assert_eq!(held().await, before + 1, "the refused write with Origin {origin:?} minted");
    }
}

/// The ICD, read as the Door reads it.
fn icd() -> serde_json::Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../core/coordination/delta-graph.icd.json");
    serde_json::from_slice(&std::fs::read(&p).expect("the ICD")).expect("the ICD parses")
}

/// A cookie session's mint, from the listed origin: the object's id.
/// A session ended as a person ends one, signed out: its writes' tails done and its last
/// seal made, so what it wrote stands at the relay and the auth service for a sign-in on
/// another Door. A write answers after its local commit, and the rest follows (O-69).
async fn end_session(http: &reqwest::Client, door: &Door, cookie: &str) {
    let r = http.post(format!("{}/v2/signout", door.base)).header("cookie", cookie).header("origin", LISTED).send().await.unwrap();
    assert!(r.status().is_success(), "sign-out: {}", r.status());
}

async fn mint_as(http: &reqwest::Client, door: &Door, cookie: &str, kind: &str, name: &str) -> String {
    let r = http
        .post(format!("{}/v2/mint", door.base))
        .header("cookie", cookie)
        .header("origin", LISTED)
        .json(&serde_json::json!({ "kind": kind, "draft": { "name": name } }))
        .send()
        .await
        .unwrap();
    let (s, body) = said(r).await;
    assert_eq!(s, 200, "/v2/mint {kind}: {body}");
    serde_json::from_str::<serde_json::Value>(&body).unwrap()["object_id"].as_str().expect("an object_id").to_string()
}

/// NC-54: /v2/apply resolves an op's name on the object's own kind. Another kind's op
/// whose id the group also uses, sent to a group with the group op's own arguments, is
/// refused and writes nothing; the group's op of that id writes (the control).
#[tokio::test(flavor = "multi_thread")]
async fn nc54_an_op_name_is_resolved_on_the_objects_kind() {
    let kinds = icd()["kinds"].clone();
    let group_op = |id: &serde_json::Value| kinds["group"]["ops"].as_object().unwrap().iter().find(|(_, o)| &o["op"] == id).map(|(n, _)| n.clone());
    let (foreign, own) = kinds["forum"]["ops"]
        .as_object()
        .unwrap()
        .iter()
        .find_map(|(n, o)| group_op(&o["op"]).map(|g| (n.clone(), g)))
        .expect("the fixture: a forum op whose id a group op also uses");
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let group = mint_as(&http, &door, &who.cookie, "group", "nc54").await;
    let apply = |op: &str, label: &str| {
        http.post(format!("{}/v2/apply", door.base))
            .header("cookie", &who.cookie)
            .header("origin", LISTED)
            .json(&serde_json::json!({ "object": group, "op": op, "args": { "displayName": label, "shape": "team" } }))
            .send()
    };
    let (h, graph, cookie, id) = (&http, format!("{}/v2/graph", door.base), &who.cookie, &group);
    let shows = |label: &'static str| {
        let graph = graph.clone();
        async move {
            let g: serde_json::Value = h.get(graph).header("cookie", cookie).send().await.unwrap().json().await.unwrap();
            let o = g["objects"].as_array().unwrap().iter().find(|o| o["id"] == id.as_str()).cloned().expect("the group is in the graph");
            o.to_string().contains(label)
        }
    };
    let (s, why) = said(apply(&foreign, "named by another kind's op").await.unwrap()).await;
    assert!(!(200..300).contains(&s), "{foreign} on a group was taken: {s} {why}");
    assert!(!shows("named by another kind's op").await, "{foreign} on a group was refused and wrote {own}");
    let (s, why) = said(apply(&own, "named by the group's op").await.unwrap()).await;
    assert_eq!(s, 200, "the control, {own}: {why}");
    assert!(shows("named by the group's op").await, "the control: {own} wrote nothing the graph shows");
}

/// NC-55, RA-10: a site's token reads in /v2/me only its Site's part of the account:
/// resume.objects, noncompliant and the count. The same account's cookie session on
/// the same Door sees the object outside the Site (the control). The Site is minted
/// first, on a Door with no client, so that the second Door can register it.
#[tokio::test(flavor = "multi_thread")]
async fn nc55_ra10_a_site_token_reads_only_its_site_in_me() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let (who, site, outside) = {
        let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
        let http = door.http();
        let who = sign_up(&http, &door).await;
        let site = mint_as(&http, &door, &who.cookie, "group", "the Site").await;
        let outside = mint_as(&http, &door, &who.cookie, "group", "outside the Site").await;
        end_session(&http, &door, &who.cookie).await;
        (who, site, outside)
    };
    let clients = auth.root.join("clients-nc55.json");
    let reg = serde_json::json!({ CLIENT: { "name": "", "callbacks": [CALLBACK], "origins": ["https://client.example.test"], "site": site } });
    std::fs::write(&clients, reg.to_string()).unwrap();
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap()),
    ]);
    let http = door.http();
    let me = format!("{}/v2/me", door.base);
    let (a, k) = attempt(&http, &door).await;
    let r = finish(&http, &door, serde_json::json!({ "attempt": a, "handle": who.handle, "sealed": seal(&k, &who.prf, &a) })).await;
    let cookie = cookie_of(&r).expect("the control: a cookie session");
    let mine: serde_json::Value = http.get(&me).header("cookie", cookie).send().await.unwrap().json().await.unwrap();
    let key = Dpop::new();
    let (code, verifier) = a_code(&http, &door, &who).await;
    let r = exchange(&http, &door, Some(key.proof("POST", &format!("{}/v2/token", door.base), None)), &code, &verifier, CLIENT).await;
    let token = r.json::<serde_json::Value>().await.unwrap()["access_token"].as_str().expect("a token").to_string();
    let theirs: serde_json::Value = http
        .get(&me)
        .header("Authorization", format!("DPoP {token}"))
        .header("DPoP", key.proof("GET", &me, Some(&token)))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let resumed = |m: &serde_json::Value| -> Vec<String> {
        m["resume"]["objects"].as_array().map(|v| v.iter().filter_map(|o| o["object"].as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    assert!(resumed(&mine).contains(&site) && resumed(&mine).contains(&outside), "the control: the cookie session's resume: {mine}");
    assert!(mine["objects"].as_u64() >= Some(2), "the control: the cookie session's count: {mine}");
    assert_eq!(resumed(&theirs), vec![site.clone()], "the token's resume.objects: {theirs}");
    assert_eq!(theirs["objects"].as_u64(), Some(1), "the token's object count: {theirs}");
    let named: Vec<&str> = theirs["noncompliant"].as_array().expect("noncompliant").iter().filter_map(|c| c["object"].as_str()).collect();
    assert!(named.iter().all(|o| *o == site), "the token's noncompliant names an object outside the Site: {named:?}");
}

/// FC-9 (O-69, mdr/icd-pin.md § Scope is applied after the cache): a site-scoped token
/// never receives, from a warm cache, an object outside its scope that another session of
/// the same process folded. One person's process holds both sessions: the first-party one
/// folds every object, twice, so the process's cache holds each outside object's fold;
/// then the token reads, twice, and is given the Site and its part alone. The suite's
/// Doors run the cache verified (FOLD_CACHE_VERIFIED).
#[tokio::test(flavor = "multi_thread")]
async fn fc9_a_site_token_gets_nothing_outside_its_scope_from_a_warm_cache() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let (who, site, post, outside, beyond) = {
        let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
        let http = door.http();
        let who = sign_up(&http, &door).await;
        let site = mint_as(&http, &door, &who.cookie, "group", "the Site").await;
        let post = mint_as(&http, &door, &who.cookie, "post", "a post in the Site").await;
        let outside = mint_as(&http, &door, &who.cookie, "group", "outside the Site").await;
        let beyond = mint_as(&http, &door, &who.cookie, "post", "a post outside it").await;
        apply_as(&http, &door, &who.cookie, &site, "base.setPart", serde_json::json!({ "part": post, "role": "post", "at": 1 })).await;
        apply_as(&http, &door, &who.cookie, &post, "base.setParent", serde_json::json!({ "parent": site, "role": "post", "at": 1 })).await;
        apply_as(&http, &door, &who.cookie, &outside, "base.setPart", serde_json::json!({ "part": beyond, "role": "post", "at": 1 })).await;
        end_session(&http, &door, &who.cookie).await;
        (who, site, post, outside, beyond)
    };
    let clients = auth.root.join("clients-fc9.json");
    let reg = serde_json::json!({ CLIENT: { "name": "", "callbacks": [CALLBACK], "origins": ["https://client.example.test"], "site": site } });
    std::fs::write(&clients, reg.to_string()).unwrap();
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap()),
    ]);
    let http = door.http();
    let graph = format!("{}/v2/graph", door.base);
    let ids = |g: &str| -> std::collections::BTreeSet<String> {
        let g: serde_json::Value = serde_json::from_str(g).unwrap();
        g["objects"].as_array().unwrap().iter().filter_map(|o| o["id"].as_str().map(str::to_string)).collect()
    };

    let (a, k) = attempt(&http, &door).await;
    let r = finish(&http, &door, serde_json::json!({ "attempt": a, "handle": who.handle, "sealed": seal(&k, &who.prf, &a) })).await;
    let cookie = cookie_of(&r).expect("a cookie session");
    let first = http.get(&graph).header("cookie", &cookie).send().await.unwrap().text().await.unwrap();
    let warm = http.get(&graph).header("cookie", &cookie).send().await.unwrap().text().await.unwrap();
    for o in [&site, &post, &outside, &beyond] {
        assert!(ids(&first).contains(o), "the control: the first-party session folds {o}: {first}");
    }
    assert_eq!(ids(&warm), ids(&first), "the control: the warm first-party read");

    let key = Dpop::new();
    let (code, verifier) = a_code(&http, &door, &who).await;
    let r = exchange(&http, &door, Some(key.proof("POST", &format!("{}/v2/token", door.base), None)), &code, &verifier, CLIENT).await;
    let token = r.json::<serde_json::Value>().await.unwrap()["access_token"].as_str().expect("a token").to_string();
    for read in ["the token's first read", "the token's second read"] {
        let theirs = http
            .get(&graph)
            .header("Authorization", format!("DPoP {token}"))
            .header("DPoP", key.proof("GET", &graph, Some(&token)))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(ids(&theirs).contains(&site) && ids(&theirs).contains(&post), "{read}: the Site and its part: {theirs}");
        for o in [&outside, &beyond] {
            assert!(!theirs.contains(o.as_str()), "{read}: an object outside the Site, from the cache the first-party session filled: {o}: {theirs}");
        }
    }
    let after = http.get(&graph).header("cookie", &cookie).send().await.unwrap().text().await.unwrap();
    assert_eq!(ids(&after), ids(&first), "the control: the first-party session still sees everything");
    let log = std::fs::read_to_string(&door.log).unwrap_or_default();
    assert!(log.contains("door session: fold cache on, every hit verified"), "the session's cache was on: {log}");
}

async fn batch_as(http: &reqwest::Client, door: &Door, cookie: &str, steps: serde_json::Value) -> (u16, serde_json::Value) {
    let r = http.post(format!("{}/v2/batch", door.base)).header("cookie", cookie).header("origin", LISTED).json(&serde_json::json!({ "steps": steps })).send().await.unwrap();
    let (s, body) = said(r).await;
    (s, serde_json::from_str(&body).unwrap_or(serde_json::Value::String(body)))
}

/// ICD 2.1.0 ROW 10, MAKE ADMIN'S SECOND STEP: POST /v2/add {object, member} adds a member
/// of a Site to one of its parts (its Host) by their identity alone, as the owner. Only a
/// member on the part's parent Site may be added this way, and only a contact of the
/// owner's (their prekey is the key package, `Node::add_part_member`): each refusal is said
/// in the words the webapp shows. Someone who is not on the Site is refused so, whoever
/// they are; the Site itself is a part of nothing; a member id that is not one is refused.
/// (A Site member who is a contact, added, is core's test: this Door forms no contact.)
#[tokio::test(flavor = "multi_thread")]
async fn row10_a_site_member_is_added_to_the_host_by_member_id_or_refused_in_words() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let (ada, bo) = (sign_up(&http, &door).await, sign_up(&http, &door).await);
    let (s, out) = batch_as(&http, &door, &ada.cookie, serde_json::json!([
        { "do": "mint", "kind": "group", "draft": { "name": "Mill Road", "shape": "community" } },
        { "do": "mint", "kind": "host", "draft": { "name": "Mill Road" } },
        { "do": "apply", "object": { "$step": 0 }, "op": "base.setPart", "args": { "part": { "$step": 1 }, "role": "host", "at": 1 } },
        { "do": "apply", "object": { "$step": 1 }, "op": "base.setParent", "args": { "parent": { "$step": 0 }, "role": "host", "at": 1 } },
    ])).await;
    assert_eq!(s, 200, "{out}");
    let (site, host) = (out["made"][0].as_str().unwrap().to_string(), out["made"][1].as_str().unwrap().to_string());
    let bo_pk = bo.pk.trim_start_matches("ed25519:").to_lowercase();
    let add = |object: String, member: String| {
        let (http, url, cookie) = (http.clone(), format!("{}/v2/add", door.base), ada.cookie.clone());
        async move {
            let r = http.post(&url).header("cookie", cookie).header("origin", LISTED).json(&serde_json::json!({ "object": object, "member": member })).send().await.unwrap();
            said(r).await
        }
    };
    let (s, why) = add(host.clone(), bo_pk.clone()).await;
    assert_ne!(s, 200, "{why}");
    assert!(why.contains(&format!("{bo_pk} is not a member of {site}")), "someone not on the Site, refused so: {s} {why}");
    let (s, why) = add(site.clone(), bo_pk.clone()).await;
    assert!(s != 200 && why.contains(&format!("{site} is a part of no Site")), "{s} {why}");
    let (s, why) = add(host.clone(), "not-a-member-id".into()).await;
    assert!(s != 200 && why.contains("is not a member id"), "{s} {why}");
}

/// An Arc whose node answers `/v1/admit` as admitted, keeping each body it is sent: a joining
/// session's contact bundles, real ones.
async fn arc_keeping_bodies() -> (String, std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>) {
    let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let keep = got.clone();
    let app = axum::Router::new().route(
        "/v1/admit",
        axum::routing::post(move |axum::Json(b): axum::Json<serde_json::Value>| {
            let keep = keep.clone();
            async move {
                keep.lock().unwrap().push(b);
                axum::Json(serde_json::json!({ "admitted": "cd", "rooms": [], "unjoined": [] }))
            }
        }),
    );
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (format!("http://127.0.0.1:{port}"), got)
}

/// A ROOM THE ARC JOINS LATE (Ralph, 30 Sep: every joiner into every room): POST /v2/history
/// {object, bundle, dry} seals a Site's or a room's history to a member by their contact bundle,
/// as its owner, so the Arc's node holds the room's whole log to pass on. A dry run sends
/// nothing and answers what would go, the spine's bundle sealed against one relay blob. A send
/// to someone not on the room, by someone not its owner, or of a Host, is refused in words. The
/// member's session takes what was sent: the owner's post from before they came is in its room.
#[tokio::test(flavor = "multi_thread")]
async fn late_arc_the_owner_sends_a_rooms_history_or_says_what_would_go() {
    let dir = std::env::temp_dir().join(format!("door-security-history-{}", unique()));
    let (keys, claim) = kiosk_claim(&dir);
    let (arc, got) = arc_keeping_bodies().await;
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()),
        ("DOOR_RELAY", relay.url.as_str()),
        ("DOOR_CLAIM_KEYS", keys.to_str().unwrap()),
        ("DOOR_ARC", arc.as_str()),
    ]);
    let http = door.http();
    let (ada, bo) = (sign_up(&http, &door).await, sign_up(&http, &door).await);

    // bo's contact bundles, as bo's session makes them for a join.
    let stay = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let r = stay.get(format!("{}/join?claim={claim}", door.base)).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 303, "the claim lands");
    let join = cookie_of(&r).expect("the join cookie");
    let r = http.post(format!("{}/v2/join", door.base)).header("cookie", format!("{}; {join}", bo.cookie)).header("origin", LISTED).send().await.unwrap();
    let (s, body) = said(r).await;
    assert_eq!(s, 200, "{body}");
    let bundles: Vec<String> = got.lock().unwrap()[0]["bundles"].as_array().unwrap().iter().map(|b| b.as_str().unwrap().to_string()).collect();
    assert!(bundles.len() >= 3, "{bundles:?}");

    // ada's room, a post in it before bo is on it, and a Host.
    let (room, host) = (mint_as(&http, &door, &ada.cookie, "forum", "Healing Resistance").await, mint_as(&http, &door, &ada.cookie, "host", "Egregore").await);
    let text = format!("ada, before bo {}", unique());
    apply_as(&http, &door, &ada.cookie, &room, "forum.post", serde_json::json!({ "text": text })).await;
    let history = |cookie: &str, body: serde_json::Value| {
        let (http, url, cookie) = (http.clone(), format!("{}/v2/history", door.base), cookie.to_string());
        async move {
            let r = http.post(&url).header("cookie", cookie).header("origin", LISTED).json(&body).send().await.unwrap();
            said(r).await
        }
    };

    let (s, dry) = history(&ada.cookie, serde_json::json!({ "object": room, "bundle": bundles[0], "dry": true })).await;
    assert_eq!(s, 200, "{dry}");
    let dry: serde_json::Value = serde_json::from_str(&dry).unwrap();
    assert_eq!((&dry["object"], &dry["dry"], &dry["unsent"]), (&serde_json::json!(room), &serde_json::json!(true), &serde_json::Value::Null), "{dry}");
    assert!(dry["posts"].as_u64() >= Some(1), "ada's post would go: {dry}");
    let (sealed, max) = (dry["sealed"].as_u64().unwrap(), dry["max"].as_u64().unwrap());
    assert!(sealed > 0 && sealed <= max, "one relay blob: {dry}");

    let (s, why) = history(&ada.cookie, serde_json::json!({ "object": room, "bundle": bundles[0] })).await;
    assert!(s == 400 && why.contains("is not a member of"), "bo is not on the room yet: {s} {why}");
    let (s, why) = history(&ada.cookie, serde_json::json!({ "object": host, "bundle": bundles[0], "dry": true })).await;
    assert!(s == 400 && why.contains("carries no history"), "a Host: {s} {why}");

    let r = http.post(format!("{}/v2/add", door.base)).header("cookie", &ada.cookie).header("origin", LISTED).json(&serde_json::json!({ "object": room, "bundle": bundles[1] })).send().await.unwrap();
    let (s, added) = said(r).await;
    assert_eq!(s, 200, "bo is added to the room: {added}");
    let (s, why) = history(&bo.cookie, serde_json::json!({ "object": room, "bundle": bundles[2], "dry": true })).await;
    assert!(s == 400 && why.contains("owner sends its history"), "bo is not its owner: {s} {why}");

    let (s, sent) = history(&ada.cookie, serde_json::json!({ "object": room, "bundle": bundles[2] })).await;
    assert_eq!(s, 200, "{sent}");
    let sent: serde_json::Value = serde_json::from_str(&sent).unwrap();
    assert!(sent["dry"] == false && sent["unsent"].is_null() && sent["posts"].as_u64() >= Some(1), "{sent}");

    let t = Instant::now();
    loop {
        let g = http.get(format!("{}/v2/graph", door.base)).header("cookie", &bo.cookie).send().await.unwrap().text().await.unwrap();
        if g.contains(&text) {
            break;
        }
        assert!(t.elapsed() < Duration::from_secs(30), "bo's session holds ada's earlier post: {g}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let _ = std::fs::remove_dir_all(&dir);
}

async fn object_count(http: &reqwest::Client, door: &Door, cookie: &str) -> usize {
    let g: serde_json::Value = http.get(format!("{}/v2/graph", door.base)).header("cookie", cookie).send().await.unwrap().json().await.unwrap();
    g["objects"].as_array().map(Vec::len).unwrap_or(0)
}

/// Register's steps, as the webapp sends them: the Site, its Host, and the edge from both
/// ends, the later steps naming the earlier mints.
fn register_steps(name: &str) -> serde_json::Value {
    serde_json::json!([
        { "do": "mint", "kind": "group", "draft": { "name": name, "shape": "community" } },
        { "do": "mint", "kind": "host", "draft": { "name": name } },
        { "do": "apply", "object": { "$step": 0 }, "op": "base.setPart", "args": { "part": { "$step": 1 }, "role": "host", "at": 1 } },
        { "do": "apply", "object": { "$step": 1 }, "op": "base.setParent", "args": { "parent": { "$step": 0 }, "role": "host", "at": 1 } },
    ])
}

/// O-69's batch: a Register is one request. Each step goes through the one write path in
/// one turn of the session, so no reader or tick lands between the halves of the Site's edge
/// to its Host: readers hammering /v2/graph through the whole of it see the edge from both
/// ends or from neither, and the Door never says a Site names a Host that does not name it.
#[tokio::test(flavor = "multi_thread")]
async fn o69_batch_register_is_one_request_and_no_half_edge_is_ever_seen() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let graph = format!("{}/v2/graph", door.base);
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let (http, graph, cookie, done) = (http.clone(), graph.clone(), who.cookie.clone(), done.clone());
            tokio::spawn(async move {
                let (mut reads, mut halves) = (0usize, Vec::new());
                while !done.load(std::sync::atomic::Ordering::SeqCst) || reads < 3 {
                    let g: serde_json::Value = http.get(&graph).header("cookie", &cookie).send().await.unwrap().json().await.unwrap();
                    reads += 1;
                    let objects = g["objects"].as_array().cloned().unwrap_or_default();
                    for site in objects.iter().filter(|o| o["kind"] == "group") {
                        for part in site["view"]["parts"].as_array().cloned().unwrap_or_default() {
                            let Some(host) = objects.iter().find(|o| o["id"] == part["part"]) else { continue };
                            if host["view"]["parent"]["parent"] != site["id"] {
                                halves.push(format!("{} names {} as its host; it names {}", site["id"], host["id"], host["view"]["parent"]));
                            }
                        }
                    }
                    for host in objects.iter().filter(|o| o["kind"] == "host") {
                        let Some(parent) = host["view"]["parent"]["parent"].as_str() else { continue };
                        if let Some(site) = objects.iter().find(|o| o["id"] == parent) {
                            if !site["view"]["parts"].as_array().is_some_and(|ps| ps.iter().any(|p| p["part"] == host["id"])) {
                                halves.push(format!("{} names {} as its parent; the Site does not name it", host["id"], parent));
                            }
                        }
                    }
                }
                (reads, halves)
            })
        })
        .collect();
    let (s, out) = batch_as(&http, &door, &who.cookie, register_steps("Riverside")).await;
    assert_eq!(s, 200, "the batch: {out}");
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    let made: Vec<String> = out["made"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assert_eq!(made.len(), 4, "a result a step: {out}");
    assert!(out["refused"].is_null(), "{out}");
    let (site, host) = (made[0].clone(), made[1].clone());
    assert_eq!((made[2].as_str(), made[3].as_str()), (site.as_str(), host.as_str()), "an apply's result is its object");
    let mut reads = 0;
    for r in readers {
        let (n, halves) = r.await.unwrap();
        reads += n;
        assert!(halves.is_empty(), "a half edge was seen: {halves:?}");
    }
    assert!(reads >= 12, "the readers read throughout: {reads}");
    let g: serde_json::Value = http.get(&graph).header("cookie", &who.cookie).send().await.unwrap().json().await.unwrap();
    let view = |id: &str| g["objects"].as_array().unwrap().iter().find(|o| o["id"] == id).map(|o| o["view"].clone()).unwrap_or_default();
    assert!(view(&site)["parts"].as_array().is_some_and(|ps| ps.iter().any(|p| p["part"] == host.as_str() && p["role"] == "host")), "the Site names its Host: {g}");
    assert_eq!(view(&host)["parent"]["parent"], site.as_str(), "the Host names its Site: {g}");
    let log = std::fs::read_to_string(&door.log).unwrap_or_default();
    assert!(!log.contains("does not name the Site"), "the Door said a Site names a Host that does not name it: {log}");
}

/// O-69's batch, refused before any of it runs (Software Security): more than 16 steps, a
/// `$step` that names no earlier mint (a later step, an apply, one out of range), a step of
/// no known shape, an arg neither text nor int. Nothing is made by any of them.
#[tokio::test(flavor = "multi_thread")]
async fn o69_a_batch_is_refused_whole_before_anything_runs() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let before = object_count(&http, &door, &who.cookie).await;
    let mint = serde_json::json!({ "do": "mint", "kind": "group", "draft": { "name": "g" } });
    let seventeen: Vec<serde_json::Value> = (0..17).map(|_| mint.clone()).collect();
    let cases = [
        (serde_json::json!(seventeen), "1 to 16 steps"),
        (serde_json::json!([]), "1 to 16 steps"),
        (serde_json::json!([mint, { "do": "apply", "object": { "$step": 2 }, "op": "base.setPart", "args": {} }, mint]), "not an earlier mint"),
        (serde_json::json!([mint, { "do": "apply", "object": { "$step": 0 }, "op": "group.setProfile", "args": { "displayName": "x" } },
                            { "do": "apply", "object": { "$step": 1 }, "op": "group.setProfile", "args": {} }]), "not an earlier mint"),
        (serde_json::json!([mint, { "do": "apply", "object": { "$step": 9 }, "op": "group.setProfile", "args": {} }]), "not an earlier mint"),
        (serde_json::json!([mint, { "do": "apply", "object": { "$step": 0 }, "op": "base.setPart", "args": { "part": { "$step": 5 } } }]), "not an earlier mint"),
        (serde_json::json!([mint, { "do": "burn", "object": "x" }]), "\"mint\" or \"apply\""),
        (serde_json::json!([mint, { "do": "apply", "object": { "$step": 0 }, "op": "group.setProfile", "args": { "displayName": [1] } }]), "neither text nor int"),
    ];
    for (steps, why) in cases {
        let (s, out) = batch_as(&http, &door, &who.cookie, steps.clone()).await;
        assert_eq!(s, 400, "{steps}: {out}");
        assert!(out.as_str().is_some_and(|o| o.contains(why)), "{steps}: {out}");
    }
    assert_eq!(object_count(&http, &door, &who.cookie).await, before, "nothing was made by a refused batch");
    let r = http
        .post(format!("{}/v2/batch", door.base))
        .header("cookie", &who.cookie)
        .header("origin", STRANGER)
        .json(&serde_json::json!({ "steps": [mint] }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 403, "a cookie batch from an origin this Door does not list");
    assert_eq!(object_count(&http, &door, &who.cookie).await, before, "and it made nothing");
}

/// O-69's batch, refused part-way: what was made stands and is named, one result a step,
/// with the step refused and the Door's words; nothing after it runs.
#[tokio::test(flavor = "multi_thread")]
async fn o69_a_batch_refused_part_way_names_what_was_made() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let before = object_count(&http, &door, &who.cookie).await;
    let (s, out) = batch_as(&http, &door, &who.cookie, serde_json::json!([
        { "do": "mint", "kind": "group", "draft": { "name": "the Site" } },
        { "do": "apply", "object": { "$step": 0 }, "op": "group.noSuchOp", "args": {} },
        { "do": "mint", "kind": "host", "draft": { "name": "never" } },
    ])).await;
    assert_eq!(s, 422, "{out}");
    let made = out["made"].as_array().unwrap();
    assert_eq!(made.len(), 1, "the Site, and nothing after the refusal: {out}");
    assert_eq!(out["refused"]["step"], 1, "{out}");
    assert_eq!(out["refused"]["why"], "a group has no op 'group.noSuchOp'", "{out}");
    let site = made[0].as_str().unwrap();
    let g: serde_json::Value = http.get(format!("{}/v2/graph", door.base)).header("cookie", &who.cookie).send().await.unwrap().json().await.unwrap();
    assert!(g["objects"].as_array().unwrap().iter().any(|o| o["id"] == site), "what was made stands: {g}");
    assert!(!g["objects"].as_array().unwrap().iter().any(|o| o["kind"] == "host"), "the step after the refusal did not run: {g}");
    assert!(object_count(&http, &door, &who.cookie).await > before);
}

/// O-69's batch under a site's token: its scope is checked at every step, as each route
/// checks it. An apply outside the Site is refused at its step, after what came before; an
/// object the batch itself minted is in scope for the steps after it.
#[tokio::test(flavor = "multi_thread")]
async fn o69_a_site_tokens_batch_is_held_to_its_scope_at_every_step() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let (who, site, outside) = {
        let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
        let http = door.http();
        let who = sign_up(&http, &door).await;
        let site = mint_as(&http, &door, &who.cookie, "group", "the Site").await;
        let outside = mint_as(&http, &door, &who.cookie, "group", "outside the Site").await;
        end_session(&http, &door, &who.cookie).await;
        (who, site, outside)
    };
    let clients = auth.root.join("clients-batch.json");
    let reg = serde_json::json!({ CLIENT: { "name": "", "callbacks": [CALLBACK], "origins": ["https://client.example.test"], "site": site } });
    std::fs::write(&clients, reg.to_string()).unwrap();
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap())]);
    let http = door.http();
    let key = Dpop::new();
    let (code, verifier) = a_code(&http, &door, &who).await;
    let r = exchange(&http, &door, Some(key.proof("POST", &format!("{}/v2/token", door.base), None)), &code, &verifier, CLIENT).await;
    let token = r.json::<serde_json::Value>().await.unwrap()["access_token"].as_str().expect("a token").to_string();
    let url = format!("{}/v2/batch", door.base);
    let batch = |steps: serde_json::Value| {
        let (http, url, token, proof) = (http.clone(), url.clone(), token.clone(), key.proof("POST", &url, Some(&token)));
        async move {
            let r = http.post(&url).header("Authorization", format!("DPoP {token}")).header("DPoP", proof).json(&serde_json::json!({ "steps": steps })).send().await.unwrap();
            let (s, b) = said(r).await;
            (s, serde_json::from_str::<serde_json::Value>(&b).unwrap_or(serde_json::Value::String(b)))
        }
    };
    let (s, out) = batch(serde_json::json!([
        { "do": "mint", "kind": "post", "draft": { "name": "the token's post" } },
        { "do": "apply", "object": { "$step": 0 }, "op": "base.setParent", "args": { "parent": site, "role": "post", "at": 1 } },
        { "do": "apply", "object": site, "op": "base.setPart", "args": { "part": { "$step": 0 }, "role": "post", "at": 1 } },
        { "do": "apply", "object": outside, "op": "group.setProfile", "args": { "displayName": "renamed from a site" } },
    ])).await;
    assert_eq!(s, 422, "{out}");
    assert_eq!(out["made"].as_array().unwrap().len(), 3, "its own mint, and the Site, are in scope: {out}");
    assert_eq!(out["refused"]["step"], 3, "{out}");
    assert_eq!(out["refused"]["why"], format!("{outside} is outside this site's scope"), "{out}");
}

/// NC-95: A SITE'S TOKEN CANNOT BRING AN OBJECT INTO ITS SCOPE BY NAMING IT. Its Site is in
/// scope, so a write to the Site passed; its args were never checked, and `base.setPart`
/// with another object's id made that object the Site's part, and so the token's. Refused
/// now at the step that names it, by /v2/batch and by /v2/apply, and the object stays out of
/// what the token reads.
#[tokio::test(flavor = "multi_thread")]
async fn nc95_a_site_tokens_write_naming_an_object_outside_its_scope_is_refused() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let (who, site, outside) = {
        let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
        let http = door.http();
        let who = sign_up(&http, &door).await;
        let site = mint_as(&http, &door, &who.cookie, "group", "the Site").await;
        let outside = mint_as(&http, &door, &who.cookie, "forum", "outside the Site").await;
        end_session(&http, &door, &who.cookie).await;
        (who, site, outside)
    };
    let clients = auth.root.join("clients-nc95.json");
    let reg = serde_json::json!({ CLIENT: { "name": "", "callbacks": [CALLBACK], "origins": ["https://client.example.test"], "site": site } });
    std::fs::write(&clients, reg.to_string()).unwrap();
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap())]);
    let http = door.http();
    let key = Dpop::new();
    let (code, verifier) = a_code(&http, &door, &who).await;
    let r = exchange(&http, &door, Some(key.proof("POST", &format!("{}/v2/token", door.base), None)), &code, &verifier, CLIENT).await;
    let token = r.json::<serde_json::Value>().await.unwrap()["access_token"].as_str().expect("a token").to_string();
    let call = |method: &'static str, path: &str, body: Option<serde_json::Value>| {
        let (http, url, token) = (http.clone(), format!("{}{path}", door.base), token.clone());
        let proof = key.proof(method, &url, Some(&token));
        async move {
            let rq = if method == "POST" { http.post(&url).json(&body.unwrap()) } else { http.get(&url) };
            let (s, b) = said(rq.header("Authorization", format!("DPoP {token}")).header("DPoP", proof).send().await.unwrap()).await;
            (s, serde_json::from_str::<serde_json::Value>(&b).unwrap_or(serde_json::Value::String(b)))
        }
    };
    let (s, out) = call("POST", "/v2/batch", Some(serde_json::json!({ "steps": [
        { "do": "apply", "object": site, "op": "base.setPart", "args": { "part": outside, "role": "room", "at": 1 } },
        { "do": "apply", "object": outside, "op": "forum.post", "args": { "text": "written through the Site" } },
    ] }))).await;
    assert_eq!(s, 422, "{out}");
    assert_eq!(out["made"].as_array().unwrap().len(), 0, "nothing made: {out}");
    assert_eq!(out["refused"]["step"], 0, "refused at the step that names it: {out}");
    assert_eq!(out["refused"]["why"], format!("{outside} (its part) is outside this site's scope"), "{out}");
    let (s, out) = call("POST", "/v2/apply", Some(serde_json::json!({ "object": site, "op": "base.setPart", "args": { "part": outside, "role": "room", "at": 1 } }))).await;
    assert_ne!(s, 200, "/v2/apply refuses it too: {out}");
    assert!(out.to_string().contains(&format!("{outside} (its part) is outside this site's scope")), "{out}");
    let (s, graph) = call("GET", "/v2/graph", None).await;
    assert_eq!(s, 200, "{graph}");
    assert!(!graph.to_string().contains(&outside), "the object stays out of what the token reads: {graph}");
    // The control: a message's text is no reference (the ICD's), whatever it spells.
    let (s, out) = call("POST", "/v2/batch", Some(serde_json::json!({ "steps": [
        { "do": "mint", "kind": "forum", "draft": { "name": "the token's room" } },
        { "do": "apply", "object": { "$step": 0 }, "op": "forum.post", "args": { "text": outside } },
    ] }))).await;
    assert_eq!(s, 200, "a post that spells an outside id is a post: {out}");
}

/// A proxy in front of the auth service that carries every request but a `PUT …/head`,
/// whose connection it closes: the service failing one call, as a caller meets it. The
/// upstream is read at each connection, so it may be named after the proxy is up.
fn drops_head_puts(upstream: std::sync::Arc<std::sync::atomic::AtomicU16>) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            listener.set_nonblocking(true).unwrap();
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            loop {
                let Ok((client, _)) = listener.accept().await else { continue };
                let up = upstream.load(std::sync::atomic::Ordering::SeqCst);
                tokio::spawn(async move {
                    let Ok(server) = tokio::net::TcpStream::connect(("127.0.0.1", up)).await else { return };
                    let (mut cr, mut cw) = client.into_split();
                    let (mut sr, mut sw) = server.into_split();
                    let ask = async move {
                        let mut buf = vec![0u8; 65536];
                        while let Ok(n) = cr.read(&mut buf).await {
                            let text = String::from_utf8_lossy(&buf[..n]);
                            if n == 0 || text.lines().any(|l| l.starts_with("PUT ") && l.contains("/head ")) {
                                break;
                            }
                            if sw.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                        }
                    };
                    let answer = async move {
                        let _ = tokio::io::copy(&mut sr, &mut cw).await;
                    };
                    tokio::select! { _ = ask => {}, _ = answer => {} }
                });
            }
        });
    });
    port
}

/// NC-96: A FAILURE'S DETAIL NAMES NO AUTH URL, whose path carries the person's key
/// (`/auth/users/<pk>/…`), and reqwest quotes the URL it failed on. The auth service fails
/// a new account's head store (the proxy closes every PUT …/head), which is stored before
/// the session's auth worker is running: the failure is said, as the auth service's, and
/// no line of the Door's log names the key or an auth URL.
#[tokio::test(flavor = "multi_thread")]
async fn nc96_an_auth_failure_is_logged_without_the_persons_key() {
    let up = std::sync::Arc::new(std::sync::atomic::AtomicU16::new(0));
    let front = format!("http://127.0.0.1:{}", drops_head_puts(up.clone()));
    let auth = Auth::start_as(Some(&front));
    let port: u16 = auth.url.rsplit(':').next().unwrap().trim_end_matches('/').parse().unwrap();
    up.store(port, std::sync::atomic::Ordering::SeqCst);
    let relay = Relay::start();
    let door = Door::start_with(&[("DOOR_AUTH", front.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let t = Instant::now();
    let log = loop {
        let log = std::fs::read_to_string(&door.log).unwrap_or_default();
        if log.contains("the head was not stored") {
            break log;
        }
        assert!(t.elapsed() < Duration::from_secs(60), "the head store's failure is said: {log}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    for line in log.lines().filter(|l| l.contains("the head was not stored")) {
        assert!(line.contains("the auth service") && !line.contains("http://"), "said as the auth service's, with no URL: {line}");
    }
    let pk = who.pk.trim_start_matches("ed25519:").to_lowercase();
    for line in log.lines().filter(|l| l.contains(&pk) || l.contains("/auth/users/")) {
        panic!("a line names the person's key or an auth URL: {line}");
    }
}

/// The person's process on this Door: the supervisor's newest child.
fn persons_process(door: &Door) -> String {
    let out = Command::new("pgrep").args(["-n", "-P", &door.child.id().to_string()]).output().expect("pgrep");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A second device of `who`, on `other`: signed in there, its graph read until `text` is in
/// it or `within` passes.
async fn seen_on(http: &reqwest::Client, other: &Door, who: &Account, text: &str, within: Duration) -> bool {
    let c = sign_in(http, other, who).await;
    let t = Instant::now();
    while t.elapsed() < within {
        let g = http.get(format!("{}/v2/graph", other.base)).header("cookie", &c).send().await.unwrap().text().await.unwrap();
        if g.contains(text) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

/// O-69: A WRITE SURVIVES A CRASH STRAIGHT AFTER ITS ANSWER. The person's process is
/// SIGKILLed the moment its write answers, whatever of its tail has run. It was sealed to
/// this disk before the answer (Q6): the next sign-in on this Door has it, from the seal or,
/// if its publish had gone and its second seal had not, from the relay as a fresh device;
/// and it reaches the relay: a second device of the same person, on another Door, reads it.
#[tokio::test(flavor = "multi_thread")]
async fn o69_a_write_survives_a_sigkill_straight_after_its_answer() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let env = [("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())];
    let door = Door::start_with(&env);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let room = mint_as(&http, &door, &who.cookie, "forum", "the room").await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let pid = persons_process(&door);
    assert!(!pid.is_empty(), "the person's process");
    let text = format!("written, then killed {}", unique());
    apply_as(&http, &door, &who.cookie, &room, "forum.post", serde_json::json!({ "text": text })).await;
    let _ = Command::new("kill").args(["-KILL", &pid]).status();
    let mut gone = false;
    for _ in 0..100 {
        let st = Command::new("ps").args(["-o", "stat=", "-p", &pid]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        if st.is_empty() || st.starts_with('Z') {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(gone, "the person's process was killed");
    let c = sign_in(&http, &door, &who).await;
    let g = http.get(format!("{}/v2/graph", door.base)).header("cookie", &c).send().await.unwrap().text().await.unwrap();
    assert!(g.contains(&text), "the next sign-in on this Door has the write: {g}");
    let other = Door::start_with(&env);
    assert!(seen_on(&http, &other, &who, &text, Duration::from_secs(30)).await, "the write reached the relay: another Door's device reads it");
}

/// NC-93 (O-69's write path: an answered write is held by this Door's seal alone until its
/// tail publishes it). THE GUARANTEE THE WRITE PATH GAVE UP, kept red (D-11): before it, a
/// write was at the relay when it was answered, so a Door that died at once lost nothing
/// another device could not read. Now the publish follows the answer: with the relay 2 s
/// away, a Door killed the moment its write answers leaves it where only its own seal holds
/// it, and a sign-in on another Door does not see it. (The control: that Door sees the room.)
#[tokio::test(flavor = "multi_thread")]
async fn nc93_an_answered_write_is_at_the_relay_even_if_its_door_dies_at_once() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let up: u16 = relay.url.trim_start_matches("ws://127.0.0.1:").split('/').next().unwrap().parse().unwrap();
    let far = wan(std::sync::Arc::new(std::sync::atomic::AtomicU16::new(up)), Duration::from_secs(2));
    let far = format!("ws://127.0.0.1:{far}/v1/relay");
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", far.as_str())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let room = mint_as(&http, &door, &who.cookie, "forum", "the room").await;
    let t = Instant::now();
    while !std::fs::read_to_string(&door.log).unwrap_or_default().contains(" at the relay ") {
        assert!(t.elapsed() < Duration::from_secs(120), "the room's mint reached the relay");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let text = format!("answered, then its Door died {}", unique());
    apply_as(&http, &door, &who.cookie, &room, "forum.post", serde_json::json!({ "text": text })).await;
    drop(door);
    let other = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    assert!(seen_on(&http, &other, &who, &room, Duration::from_secs(30)).await, "the control: another Door's device holds the room");
    assert!(seen_on(&http, &other, &who, &text, Duration::from_secs(10)).await, "the answered write is on no other device: its Door died before its tail published it");
}

/// A WAN on loopback, for a bench: every byte waits `rtt / 2` each way, in order, and each
/// connection waits two round trips first (TCP and TLS through an edge). Answers its port;
/// the upstream is read at each connection, so it may be named after the WAN is up.
fn wan(upstream: std::sync::Arc<std::sync::atomic::AtomicU16>, rtt: Duration) -> u16 {
    wan_with(upstream, rtt, None)
}

/// As `wan`, and with `Some(initcwnd)`, the server's answers paced by TCP slow start
/// (`paced`): where the win is round trips on size, a WAN of delay alone under-counts it.
fn wan_with(upstream: std::sync::Arc<std::sync::atomic::AtomicU16>, rtt: Duration, initcwnd: Option<usize>) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async move {
            listener.set_nonblocking(true).unwrap();
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            loop {
                let Ok((client, _)) = listener.accept().await else { continue };
                let up = upstream.load(std::sync::atomic::Ordering::SeqCst);
                tokio::spawn(async move {
                    tokio::time::sleep(rtt * 2).await;
                    let Ok(server) = tokio::net::TcpStream::connect(("127.0.0.1", up)).await else { return };
                    let (cr, cw) = client.into_split();
                    let (sr, sw) = server.into_split();
                    match initcwnd {
                        None => _ = tokio::join!(delayed(cr, sw, rtt / 2), delayed(sr, cw, rtt / 2)),
                        Some(init) => _ = tokio::join!(delayed(cr, sw, rtt / 2), paced(sr, cw, rtt, init)),
                    }
                });
            }
        });
    });
    port
}

async fn delayed(mut from: tokio::net::tcp::OwnedReadHalf, mut to: tokio::net::tcp::OwnedWriteHalf, by: Duration) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(tokio::time::Instant, Vec<u8>)>();
    let send = tokio::spawn(async move {
        while let Some((due, data)) = rx.recv().await {
            tokio::time::sleep_until(due).await;
            if to.write_all(&data).await.is_err() {
                break;
            }
        }
        let _ = to.shutdown().await;
    });
    let mut buf = vec![0u8; 65536];
    while let Ok(n) = from.read(&mut buf).await {
        if n == 0 || tx.send((tokio::time::Instant::now() + by, buf[..n].to_vec())).is_err() {
            break;
        }
    }
    drop(tx);
    let _ = send.await;
}

/// Server to client as TCP slow start delivers it on a warm but idle connection (RFC 5681 §4.1;
/// Linux's tcp_slow_start_after_idle): a burst waits `rtt / 2`, then `init` bytes go in its first
/// round trip and twice as many in each after. What the server has already written joins the
/// burst; a burst within a round trip of the last keeps the window it had grown to.
async fn paced(mut from: tokio::net::tcp::OwnedReadHalf, mut to: tokio::net::tcp::OwnedWriteHalf, rtt: Duration, init: usize) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut buf = vec![0u8; 1 << 20];
    let (mut cwnd, mut last) = (init, None::<Instant>);
    'bursts: loop {
        let n = match from.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let mut burst = buf[..n].to_vec();
        let mut ended = false;
        // `read` is cancel safe: a timeout loses no bytes.
        while let Ok(r) = tokio::time::timeout(Duration::from_millis(5), from.read(&mut buf)).await {
            match r {
                Ok(n) if n > 0 => burst.extend_from_slice(&buf[..n]),
                _ => {
                    ended = true;
                    break;
                }
            }
        }
        if !last.is_some_and(|t: Instant| t.elapsed() < rtt) {
            cwnd = init;
        }
        tokio::time::sleep(rtt / 2).await;
        let mut at = 0;
        while at < burst.len() {
            let end = (at + cwnd).min(burst.len());
            if to.write_all(&burst[at..end]).await.is_err() {
                break 'bursts;
            }
            at = end;
            cwnd *= 2;
            if at < burst.len() {
                tokio::time::sleep(rtt).await;
            }
        }
        last = Some(Instant::now());
        if ended {
            break;
        }
    }
    let _ = to.shutdown().await;
}

/// THE MODEL'S FETCH, MEASURED, not a check (UX's perf audit): warm, a connection used and then
/// idle past a round trip, behind `ICD_WAN_RTT_MS` (default 200) of WAN with slow start from 10
/// segments. /v2/icd as every landing fetched it; and, where the Door's page names the model's
/// hash, the hashed path gzip, and /v2/icd revalidated. `ICD_DOOR_BIN` measures another build.
///   cargo test --release --test security -- --ignored icd_fetch_latency --nocapture
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn icd_fetch_latency() {
    let rtt = Duration::from_millis(std::env::var("ICD_WAN_RTT_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(200));
    let bin = std::env::var("ICD_DOOR_BIN").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_BIN_EXE_door")));
    let door = Door::launch(&bin, &[], &[]);
    let near: u16 = door.base.rsplit(':').next().unwrap().parse().unwrap();
    let far = format!("http://127.0.0.1:{}", wan_with(std::sync::Arc::new(std::sync::atomic::AtomicU16::new(near)), rtt, Some(10 * 1448)));
    let http = reqwest::Client::new();
    let page = http.get(format!("{far}/")).send().await.unwrap().text().await.unwrap();
    let hash = page.split("id=\"wallflowers-icd\" content=\"").nth(1).map(|r| r[..64].to_string());
    let timed = |path: String, headers: Vec<(&'static str, String)>| {
        let (http, far) = (http.clone(), far.clone());
        async move {
            let mut took = vec![];
            let (mut status, mut bytes) = (0, 0);
            for _ in 0..7 {
                http.get(format!("{far}/door/wordmark.svg")).send().await.unwrap().bytes().await.unwrap();
                tokio::time::sleep(rtt * 2).await;
                let mut rq = http.get(format!("{far}{path}"));
                for (k, v) in &headers {
                    rq = rq.header(*k, v);
                }
                let t = Instant::now();
                let r = rq.send().await.unwrap();
                status = r.status().as_u16();
                bytes = r.bytes().await.unwrap().len();
                took.push(t.elapsed());
            }
            took.sort();
            (status, bytes, took[took.len() / 2])
        }
    };
    let say = |what: &str, (status, bytes, took): (u16, usize, Duration)| {
        println!("icd: {what:32} {status}  {bytes:7} B  median {:5.0} ms", took.as_secs_f64() * 1000.0)
    };
    println!("icd: {} at {} ms RTT, slow start from 10 segments", bin.display(), rtt.as_millis());
    say("GET /v2/icd", timed("/v2/icd".into(), vec![]).await);
    let Some(hash) = hash else {
        println!("icd: the page names no hash");
        return;
    };
    let gz = || ("accept-encoding", "gzip, deflate, br".to_string());
    say("GET /v2/icd/<hash>, gzip", timed(format!("/v2/icd/{hash}"), vec![gz()]).await);
    say("GET /v2/icd, gzip", timed("/v2/icd".into(), vec![gz()]).await);
    say("GET /v2/icd, revalidated", timed("/v2/icd".into(), vec![gz(), ("if-none-match", format!("\"{hash}-gz\""))]).await);
}

/// O-69's WRITE PATH, MEASURED, not a check: a post's time to its answer, to the relay (the
/// Door's own log line: "at the relay N ms after the answer"; before this change the
/// publish was inside the answer), and until a second device of the same person, on
/// another Door, reads it; and a Register as a batch and as its separate requests. Behind
/// `O69_WAN_RTT_MS` (default 340: 170 each way, Singapore to London) of WAN in front of the
/// relay and the auth service, or 0 for loopback. `O69_DOOR_BIN` measures another build;
/// `O69_GROUPS` gives the account that many groups beside the room; `O69_SEEN=0` leaves out
/// the second device.
///   cargo test --release --test security -- --ignored o69_write_path_latency --nocapture
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn o69_write_path_latency() {
    let rtt = Duration::from_millis(std::env::var("O69_WAN_RTT_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(340));
    let bin = std::env::var("O69_DOOR_BIN").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_BIN_EXE_door")));
    let posts: usize = std::env::var("O69_POSTS").ok().and_then(|v| v.parse().ok()).unwrap_or(10);
    let port = |p: u16| std::sync::Arc::new(std::sync::atomic::AtomicU16::new(p));
    // The auth service names itself as the Door reaches it: through the WAN.
    let auth_up = port(0);
    let wan_auth = wan(auth_up.clone(), rtt);
    let auth = Auth::start_as(Some(&format!("http://127.0.0.1:{wan_auth}")));
    auth_up.store(auth.url.rsplit(':').next().unwrap().parse().unwrap(), std::sync::atomic::Ordering::SeqCst);
    let relay = Relay::start();
    let wan_relay = wan(port(relay.url.trim_start_matches("ws://127.0.0.1:").split('/').next().unwrap().parse().unwrap()), rtt);
    // The cache on, as deployed; its check off, as deployed (every hit refolded is a test's cost).
    let env = [
        ("DOOR_AUTH", format!("http://127.0.0.1:{wan_auth}")),
        ("DOOR_RELAY", format!("ws://127.0.0.1:{wan_relay}/v1/relay")),
        ("PACIFIC_FOLD_CACHE_VERIFY", "0".to_string()),
    ];
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let a = Door::launch(&bin, &env, &[]);
    let http = a.http();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let who = sign_up(&http, &a).await;
    let room = mint_as(&http, &a, &who.cookie, "forum", "the room").await;
    // O69_GROUPS: the groups the account holds beside the room (BUILD, O-77's card walk: 20).
    let groups: usize = std::env::var("O69_GROUPS").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    for g in 0..groups {
        mint_as(&http, &a, &who.cookie, "group", &format!("group {g}")).await;
    }
    tokio::time::sleep(Duration::from_secs(3)).await;
    // O69_SEEN=0: the answer and the relay alone, without the second device on another Door.
    let seeing = std::env::var("O69_SEEN").map(|v| v != "0").unwrap_or(true);
    let b = seeing.then(|| Door::launch(&bin, &env, &[]));
    let other = match &b {
        None => String::new(),
        Some(b) => {
            let (at, k) = attempt(&http, b).await;
            let r = finish(&http, b, serde_json::json!({ "attempt": at, "handle": who.handle, "sealed": seal(&k, &who.prf, &at) })).await;
            let st = r.status();
            match cookie_of(&r) {
                Some(c) => c,
                None => panic!("a second device of the same person, on another Door: {st} {}", r.text().await.unwrap_or_default()),
            }
        }
    };
    let post = |text: String| {
        let (http, base, c, room) = (http.clone(), a.base.clone(), who.cookie.clone(), room.clone());
        async move {
            let r = http.post(format!("{base}/v2/apply")).header("cookie", &c).header("origin", LISTED)
                .json(&serde_json::json!({ "object": room, "op": "forum.post", "args": { "text": text } })).send().await.unwrap();
            assert_eq!(r.status().as_u16(), 200, "{}", r.text().await.unwrap_or_default());
        }
    };
    let (mut answer, mut seen) = (Vec::new(), Vec::new());
    for i in 0..posts {
        let text = format!("bench post {i} {}", unique());
        let t = Instant::now();
        post(text.clone()).await;
        answer.push(t.elapsed());
        let Some(b) = &b else {
            tokio::time::sleep(Duration::from_millis(500)).await;
            continue;
        };
        loop {
            let g = http.get(format!("{}/v2/graph", b.base)).header("cookie", &other).send().await.unwrap().text().await.unwrap();
            if g.contains(&text) {
                break;
            }
            assert!(t.elapsed() < Duration::from_secs(60), "post {i} was not seen on the second Door within 60 s");
        }
        seen.push(t.elapsed());
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let relayed: Vec<f64> = std::fs::read_to_string(&a.log)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split(" at the relay ").nth(1).and_then(|r| r.split(' ').next()).and_then(|n| n.parse::<f64>().ok()))
        .collect();
    let med = |mut v: Vec<f64>| -> f64 { v.sort_by(|x, y| x.partial_cmp(y).unwrap()); if v.is_empty() { f64::NAN } else { v[v.len() / 2] } };
    let hi = |mut v: Vec<f64>| -> f64 { v.sort_by(|x, y| x.partial_cmp(y).unwrap()); v.last().copied().unwrap_or(f64::NAN) };
    let answer: Vec<f64> = answer.into_iter().map(ms).collect();
    let seen: Vec<f64> = seen.into_iter().map(ms).collect();
    eprintln!(
        "O69 WRITE rtt {} ms, {posts} posts: answer median {:.0} max {:.0} ms | at the relay after the answer median {:.0} ms ({} logged) | seen on another device median {:.0} max {:.0} ms",
        rtt.as_millis(), med(answer.clone()), hi(answer), med(relayed.clone()), relayed.len(), med(seen.clone()), hi(seen)
    );

    // A Register: its separate requests, then (where the build has it) one batch.
    let steps = |name: &str| register_steps(name);
    let t = Instant::now();
    let site = mint_as(&http, &a, &who.cookie, "group", "separately").await;
    let host = mint_as(&http, &a, &who.cookie, "host", "separately").await;
    apply_as(&http, &a, &who.cookie, &site, "base.setPart", serde_json::json!({ "part": host, "role": "host", "at": 1 })).await;
    apply_as(&http, &a, &who.cookie, &host, "base.setParent", serde_json::json!({ "parent": site, "role": "host", "at": 1 })).await;
    let separate = t.elapsed();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let t = Instant::now();
    let (s, _) = batch_as(&http, &a, &who.cookie, steps("batched")).await;
    let batched = t.elapsed();
    eprintln!(
        "O69 REGISTER rtt {} ms: four requests {:.0} ms | one batch {}",
        rtt.as_millis(), ms(separate), if s == 200 { format!("{:.0} ms", ms(batched)) } else { format!("not in this build ({s})") }
    );
}

/// O-69's MEASUREMENT, not a check: fold time through a real Door as a room grows. One owner,
/// one room, forum.posts to 1,000; at 100, 250, 500 and 1,000 it prints each post's p50 and
/// p95 and five reads of /v2/graph (every held object folded) and /v2/me. Run it in release,
/// as production is built: `cargo test --release --test security -- --ignored
/// o69_fold_time_as_a_room_grows --nocapture`. It asserts only that each call answers. The
/// cache is on, unverified; `O69_BENCH_CACHE=off` measures it off.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn o69_fold_time_as_a_room_grows() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let model = if std::env::var("O69_BENCH_CACHE").as_deref() == Ok("off") { "" } else { "o69-bench" };
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()),
        ("DOOR_RELAY", relay.url.as_str()),
        ("PACIFIC_FOLD_CACHE_MODEL", model),
        ("PACIFIC_FOLD_CACHE_VERIFY", "0"),
    ]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let site = mint_as(&http, &door, &who.cookie, "group", "the Site").await;
    let room = mint_as(&http, &door, &who.cookie, "forum", "the room").await;
    let post = |i: usize| {
        let (http, base, c, room) = (&http, door.base.clone(), who.cookie.clone(), room.clone());
        async move {
            let t = Instant::now();
            let r = http.post(format!("{base}/v2/apply")).header("cookie", &c).header("origin", LISTED)
                .json(&serde_json::json!({ "object": room, "op": "forum.post", "args": { "text": format!("visitor post number {i}, a line of ordinary length for a room at the show") } }))
                .send().await.unwrap();
            assert_eq!(r.status().as_u16(), 200, "post {i}: {}", r.text().await.unwrap_or_default());
            t.elapsed()
        }
    };
    let read = |path: &'static str| {
        let (http, base, c) = (&http, door.base.clone(), who.cookie.clone());
        async move {
            let t = Instant::now();
            let r = http.get(format!("{base}{path}")).header("cookie", &c).send().await.unwrap();
            assert_eq!(r.status().as_u16(), 200, "{path}");
            let _ = r.bytes().await.unwrap();
            t.elapsed()
        }
    };
    let _ = site;
    let mut n = 0usize;
    for target in [100usize, 250, 500, 1000] {
        let mut posts = Vec::new();
        while n < target { posts.push(post(n).await); n += 1; }
        posts.sort();
        let mut g = Vec::new(); let mut m = Vec::new();
        for _ in 0..5 { g.push(read("/v2/graph").await); m.push(read("/v2/me").await); }
        g.sort(); m.sort();
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        eprintln!("FOLD {target} posts: post p50 {:.1} ms p95 {:.1} ms (last batch) | /v2/graph median {:.1} ms max {:.1} | /v2/me median {:.1} ms max {:.1}",
            ms(posts[posts.len()/2]), ms(posts[posts.len()*95/100]), ms(g[2]), ms(g[4]), ms(m[2]), ms(m[4]));
    }
}

/// NC-81: what a Site creates through the webapp is declared at BOTH ends, as the graph
/// requires and face_items counts it: the Site's group.setAffiliation {rel: created} and
/// the object's own base.setBacklink {object: site, rel: created}, in create()'s order,
/// through the Door. Each view names the other, and nothing held fails to fold.
#[tokio::test(flavor = "multi_thread")]
async fn nc81_a_created_post_and_event_are_declared_at_both_ends() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let apply = |object: String, op: &'static str, args: serde_json::Value| {
        let (http, base, c) = (&http, door.base.clone(), who.cookie.clone());
        async move {
            let r = http.post(format!("{base}/v2/apply")).header("cookie", &c).header("origin", LISTED)
                .json(&serde_json::json!({ "object": object, "op": op, "args": args })).send().await.unwrap();
            let (s, body) = said(r).await;
            assert_eq!(s, 200, "{op}: {body}");
        }
    };
    let site = mint_as(&http, &door, &who.cookie, "group", "the Site").await;
    let host = mint_as(&http, &door, &who.cookie, "host", "the Site").await;
    let at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    apply(site.clone(), "base.setPart", serde_json::json!({ "part": host, "role": "host", "at": at })).await;
    apply(host.clone(), "base.setParent", serde_json::json!({ "parent": site, "role": "host", "at": at })).await;
    let mut made = Vec::new();
    for kind in ["post", "event"] {
        // As the webapp's sheet sends it: an event carries its start (/v2/draft/event).
        let week = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64 + 7 * 86_400_000;
        let draft = if kind == "event" {
            serde_json::json!({ "name": format!("the Site's {kind}"), "start_ms": week })
        } else {
            serde_json::json!({ "name": format!("the Site's {kind}") })
        };
        let r = http.post(format!("{}/v2/mint", door.base)).header("cookie", &who.cookie).header("origin", LISTED)
            .json(&serde_json::json!({ "kind": kind, "draft": draft })).send().await.unwrap();
        let (s, body) = said(r).await;
        assert_eq!(s, 200, "/v2/mint {kind}: {body}");
        let id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["object_id"].as_str().expect("an object_id").to_string();
        let at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        apply(site.clone(), "group.setAffiliation", serde_json::json!({ "peer": id, "rel": "created", "name": format!("the Site's {kind}"), "at": at })).await;
        apply(id.clone(), "base.setBacklink", serde_json::json!({ "object": site, "rel": "created", "at": at })).await;
        made.push(id);
    }
    let g: serde_json::Value = http.get(format!("{}/v2/graph", door.base)).header("cookie", &who.cookie).send().await.unwrap().json().await.unwrap();
    let view = |id: &str| g["objects"].as_array().unwrap().iter().find(|o| o["id"] == id).unwrap_or_else(|| panic!("{id} in /v2/graph")).clone();
    let site_view = view(&site)["view"].to_string();
    for id in &made {
        assert!(site_view.contains(id.as_str()), "the Site names what it created: {id} not in {site_view}");
        let o = view(id);
        assert_eq!(o["folds"], true, "{id} ({}) folds: {}", o["kind"], o["why"]);
        assert!(o["view"].to_string().contains(site.as_str()), "{id} names the Site that created it: {}", o["view"]);
    }
    let me: serde_json::Value = http.get(format!("{}/v2/me", door.base)).header("cookie", &who.cookie).send().await.unwrap().json().await.unwrap();
    assert_eq!(me["noncompliant"], serde_json::json!([]), "everything held folds: {me}");
}

/// NC-85: a session's Arc is its relay, from configuration. What it mints is stamped with
/// the relay it is written on, not the compiled production Arc: a node on that relay
/// otherwise drained the object from production and never folded it (FC-6, door-test).
#[tokio::test(flavor = "multi_thread")]
async fn nc85_what_a_session_mints_is_stamped_with_its_own_relay() {
    let auth = Auth::start();
    let relay = Relay::start();
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let account = sign_up(&http, &door).await;
    let site = mint_as(&http, &door, &account.cookie, "group", "Mill Road").await;

    let gid = hex::decode(&site).unwrap();
    let stamped: Vec<String> = std::fs::read_dir(door.root.join("sessions"))
        .unwrap()
        .filter_map(|d| {
            let db = d.ok()?.path().join("pacific.db");
            let dir = pacific_core::directory::Directory::open_at(&db).ok()?;
            dir.group_arc(&gid).ok().flatten()
        })
        .collect();
    assert_eq!(stamped, vec![relay.url.clone()], "the Site is stamped with the Door's relay, once");
    assert_ne!(stamped[0], "wss://arc.wallflowers.io/v1/relay", "not production's");
}

/// O-69, FC-2: this suite's sessions fold with the cache on and every hit verified, so every
/// test here that reads is also a check that a hit equals a refold.
#[tokio::test]
async fn o69_fc2_the_suites_sessions_run_the_cache_verified() {
    let auth = Auth::start();
    let relay = Relay::start();
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let account = sign_up(&http, &door).await;
    let site = mint_as(&http, &door, &account.cookie, "group", "Mill Road").await;
    for _ in 0..2 {
        let r = http.get(format!("{}/v2/graph", door.base)).header("cookie", &account.cookie).send().await.unwrap();
        assert_eq!(r.status(), 200);
        assert!(r.text().await.unwrap().contains(&site));
    }
    let log = std::fs::read_to_string(&door.log).unwrap_or_default();
    assert!(log.contains("door session: fold cache on, every hit verified"), "the session's cache: {log}");
}

/// A cookie session's op, from the listed origin: 200 or the test stops, in the Door's words.
async fn apply_as(http: &reqwest::Client, door: &Door, cookie: &str, object: &str, op: &str, args: serde_json::Value) {
    let r = http
        .post(format!("{}/v2/apply", door.base))
        .header("cookie", cookie)
        .header("origin", LISTED)
        .json(&serde_json::json!({ "object": object, "op": op, "args": args }))
        .send()
        .await
        .unwrap();
    let (s, body) = said(r).await;
    assert_eq!(s, 200, "/v2/apply {op}: {body}");
}

/// Software Security, after NC-81: a site token's /v2/graph names no object outside its
/// Site, inside a kept object's view either. An in-scope post's backlink, the Site's
/// affiliation to a second Site, and a second in-scope post's parent there are cut; the
/// first post's parent, the Site itself, stays. (A Group's view renders no parent.) The
/// first-party session still sees them all (the control), so the cut is the token's alone.
/// And a site token never holds the self record, in its graph or in /v2/me, nor any of the
/// spine (Software Security's ruling: never a self record or spine).
#[tokio::test(flavor = "multi_thread")]
async fn a_site_tokens_graph_names_no_object_outside_its_site() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let (who, site, post, post2, other) = {
        let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
        let http = door.http();
        let who = sign_up(&http, &door).await;
        let site = mint_as(&http, &door, &who.cookie, "group", "the Site").await;
        let other = mint_as(&http, &door, &who.cookie, "group", "another Site").await;
        let post = mint_as(&http, &door, &who.cookie, "post", "a post").await;
        let post2 = mint_as(&http, &door, &who.cookie, "post", "a second post").await;
        apply_as(&http, &door, &who.cookie, &site, "base.setPart", serde_json::json!({ "part": post, "role": "post", "at": 1 })).await;
        apply_as(&http, &door, &who.cookie, &site, "base.setPart", serde_json::json!({ "part": post2, "role": "post", "at": 1 })).await;
        apply_as(&http, &door, &who.cookie, &post, "base.setBacklink", serde_json::json!({ "object": other, "rel": "created", "at": 1 })).await;
        apply_as(&http, &door, &who.cookie, &site, "group.setAffiliation", serde_json::json!({ "peer": other, "rel": "peer", "name": "another Site", "at": 1 })).await;
        apply_as(&http, &door, &who.cookie, &post, "base.setParent", serde_json::json!({ "parent": site, "role": "post", "at": 1 })).await;
        apply_as(&http, &door, &who.cookie, &post2, "base.setParent", serde_json::json!({ "parent": other, "role": "post", "at": 1 })).await;
        end_session(&http, &door, &who.cookie).await;
        (who, site, post, post2, other)
    };
    let clients = auth.root.join("clients-graph-cut.json");
    let reg = serde_json::json!({ CLIENT: { "name": "", "callbacks": [CALLBACK], "origins": ["https://client.example.test"], "site": site } });
    std::fs::write(&clients, reg.to_string()).unwrap();
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap()),
    ]);
    let http = door.http();
    let graph = format!("{}/v2/graph", door.base);

    let (a, k) = attempt(&http, &door).await;
    let r = finish(&http, &door, serde_json::json!({ "attempt": a, "handle": who.handle, "sealed": seal(&k, &who.prf, &a) })).await;
    let cookie = cookie_of(&r).expect("the control: a cookie session");
    let me = format!("{}/v2/me", door.base);
    let own: serde_json::Value = http.get(&me).header("cookie", &cookie).send().await.unwrap().json().await.unwrap();
    let spine = own["resume"]["spine"].as_str().expect("the control: the first-party /v2/me names the spine").to_string();
    let mine = http.get(&graph).header("cookie", cookie).send().await.unwrap().text().await.unwrap();
    let mine: serde_json::Value = serde_json::from_str(&mine).unwrap();
    let view = |g: &serde_json::Value, id: &str| g["objects"].as_array().unwrap().iter().find(|o| o["id"] == id).map(|o| o["view"].clone());
    let named = |v: Option<serde_json::Value>, field: &str, key: &str| -> Vec<String> {
        v.and_then(|v| v[field].as_array().cloned()).unwrap_or_default().iter().filter_map(|e| e[key].as_str().map(str::to_string)).collect()
    };
    assert_eq!(named(view(&mine, &post), "backlinks", "object"), vec![other.clone()], "the control: the post's backlink: {mine}");
    assert_eq!(named(view(&mine, &site), "affiliations", "peer"), vec![other.clone()], "the control: the Site's affiliation: {mine}");
    let parent = |v: Option<serde_json::Value>| v.and_then(|v| v["parent"]["parent"].as_str().map(str::to_string));
    assert_eq!(parent(view(&mine, &post)), Some(site.clone()), "the control: the post's parent: {mine}");
    assert_eq!(parent(view(&mine, &post2)), Some(other.clone()), "the control: the second post's parent: {mine}");
    assert!(mine["spine"].as_array().is_some_and(|s| !s.is_empty()), "the control: the spine: {mine}");

    let key = Dpop::new();
    let (code, verifier) = a_code(&http, &door, &who).await;
    let r = exchange(&http, &door, Some(key.proof("POST", &format!("{}/v2/token", door.base), None)), &code, &verifier, CLIENT).await;
    let token = r.json::<serde_json::Value>().await.unwrap()["access_token"].as_str().expect("a token").to_string();
    let theirs = http
        .get(&graph)
        .header("Authorization", format!("DPoP {token}"))
        .header("DPoP", key.proof("GET", &graph, Some(&token)))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let g: serde_json::Value = serde_json::from_str(&theirs).unwrap();
    assert!(view(&g, &site).is_some() && view(&g, &post).is_some(), "the Site and its post are in the token's graph: {theirs}");
    let mut parts = named(view(&g, &site), "parts", "part");
    parts.sort();
    let mut want = vec![post.clone(), post2.clone()];
    want.sort();
    assert_eq!(parts, want, "the Site's parts, in scope, stay: {theirs}");
    assert_eq!(parent(view(&g, &post)), Some(site.clone()), "the post's parent, in scope, stays: {theirs}");
    assert!(view(&g, &post2).is_some_and(|v| v["parent"].is_null()), "the second post's parent, out of scope, is dropped: {theirs}");
    assert!(!theirs.contains(&other), "a site token's graph names the other Site: {theirs}");
    assert!(!theirs.contains(&spine), "a site token's graph holds the self record: {theirs}");
    assert_eq!(g["spine"], serde_json::json!([]), "a site token's graph holds the spine: {theirs}");
    let theirs_me = http
        .get(&me)
        .header("Authorization", format!("DPoP {token}"))
        .header("DPoP", key.proof("GET", &me, Some(&token)))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!theirs_me.contains(&spine), "a site token's /v2/me names the self record: {theirs_me}");
}

/// Whether a /v2/events response carries a `changed` within `secs`, read on from where it was: false
/// when the time passes, or when the stream has ended.
async fn hears(r: &mut reqwest::Response, secs: u64) -> bool {
    let end = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        match tokio::time::timeout_at(end, r.chunk()).await {
            Ok(Ok(Some(c))) if String::from_utf8_lossy(&c).contains("event: changed") => return true,
            Ok(Ok(Some(_))) => continue,
            _ => return false,
        }
    }
}

/// NC-133 (run 97): a Site's page opens two /v2/events on its one token. Each stream of the token's
/// session, whose scope is one, hears its Site's changes: a second does not end the first. A stream
/// opened after the first has closed (a page reloaded) hears the next change, and not one before it.
#[tokio::test(flavor = "multi_thread")]
async fn nc133_every_stream_of_a_site_token_hears_its_sites_changes() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let (who, site) = {
        let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
        let http = door.http();
        let who = sign_up(&http, &door).await;
        let site = mint_as(&http, &door, &who.cookie, "group", "the Site").await;
        end_session(&http, &door, &who.cookie).await;
        (who, site)
    };
    let clients = auth.root.join("clients-nc133.json");
    let reg = serde_json::json!({ CLIENT: { "name": "", "callbacks": [CALLBACK], "origins": ["https://client.example.test"], "site": site } });
    std::fs::write(&clients, reg.to_string()).unwrap();
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap()),
    ]);
    let http = door.http();
    let (a, k) = attempt(&http, &door).await;
    let r = finish(&http, &door, serde_json::json!({ "attempt": a, "handle": who.handle, "sealed": seal(&k, &who.prf, &a) })).await;
    let cookie = cookie_of(&r).expect("the person's own session, which writes");
    let key = Dpop::new();
    let (code, verifier) = a_code(&http, &door, &who).await;
    let r = exchange(&http, &door, Some(key.proof("POST", &format!("{}/v2/token", door.base), None)), &code, &verifier, CLIENT).await;
    let token = r.json::<serde_json::Value>().await.unwrap()["access_token"].as_str().expect("a token").to_string();
    let events = format!("{}/v2/events", door.base);
    let open = || async {
        let r = http.get(&events).header("Authorization", format!("DPoP {token}")).header("DPoP", key.proof("GET", &events, Some(&token))).send().await.unwrap();
        assert_eq!(r.status().as_u16(), 200, "a site token's /v2/events");
        r
    };
    let write = |n: u32| {
        let (http, door, cookie, site) = (&http, &door, cookie.clone(), site.clone());
        async move {
            let part = mint_as(http, door, &cookie, "post", &format!("post {n}")).await;
            apply_as(http, door, &cookie, &site, "base.setPart", serde_json::json!({ "part": part, "role": "post", "at": n })).await;
        }
    };

    let (mut first, mut second) = (open().await, open().await);
    write(1).await;
    assert!(hears(&mut second, 10).await, "the second stream hears the Site's change");
    assert!(hears(&mut first, 10).await, "the first stream still hears it: a second does not end it");
    drop(first);
    let mut third = open().await;
    assert!(!hears(&mut third, 2).await, "a stream opened after a change hears nothing of it");
    write(2).await;
    assert!(hears(&mut third, 10).await, "a stream opened after the first closed hears the next change");
    assert!(hears(&mut second, 10).await, "and the second still does");
}

// ─── training_wheels P0 (mdr/training-wheels.md) ─────────────────────────────

/// training_wheels' PostgreSQL: a throwaway cluster, Homebrew's postgresql@16 (Ralph, 28 Sep),
/// on a unix socket in its own directory and no TCP port; stopped and removed on drop.
struct Pg {
    root: PathBuf,
    bin: PathBuf,
}

impl Pg {
    fn start() -> Pg {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let bin = PathBuf::from(std::env::var("PG_BIN").unwrap_or_else(|_| "/opt/homebrew/opt/postgresql@16/bin".into()));
        assert!(bin.join("initdb").exists(), "these tests need PostgreSQL 16: brew install postgresql@16 ({} is absent)", bin.display());
        // Short: a socket's path is at most 104 bytes on macOS.
        let root = std::env::temp_dir().join(format!("tw{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let tools = bin.clone();
        let run = move |cmd: &str, args: &[&str]| {
            let out = Command::new(tools.join(cmd)).args(args).output().unwrap_or_else(|e| panic!("{cmd}: {e}"));
            assert!(out.status.success(), "{cmd}: {}", String::from_utf8_lossy(&out.stderr));
        };
        let data = root.join("data");
        run("initdb", &["-D", data.to_str().unwrap(), "-U", "door", "--auth=trust", "-E", "UTF8", "--no-locale", "-N"]);
        let pg = Pg { root, bin };
        pg.resume();
        run("createdb", &["-h", pg.root.to_str().unwrap(), "-U", "door", "training_wheels"]);
        pg
    }

    fn dsn(&self) -> String {
        format!("host={} dbname=training_wheels user=door", self.root.display())
    }

    fn ctl(&self, args: &[&str]) {
        let data = self.root.join("data");
        let log = self.root.join("log");
        let mut all = vec!["-D", data.to_str().unwrap(), "-l", log.to_str().unwrap(), "-w"];
        all.extend_from_slice(args);
        let out = Command::new(self.bin.join("pg_ctl")).args(&all).output().unwrap();
        assert!(out.status.success(), "pg_ctl {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    /// Down, as a restart or an upgrade takes it down.
    fn stop(&self) {
        self.ctl(&["-m", "fast", "stop"]);
    }

    fn resume(&self) {
        let opts = format!("-k {} -c listen_addresses='' -c fsync=off", self.root.display());
        self.ctl(&["-o", &opts, "start"]);
    }

    /// One query's rows, `|`-separated, as psql prints them unaligned.
    fn query(&self, sql: &str) -> String {
        let out = Command::new(self.bin.join("psql"))
            .args(["-h", self.root.to_str().unwrap(), "-U", "door", "-d", "training_wheels", "-At", "-c", sql])
            .output()
            .unwrap();
        assert!(out.status.success(), "psql {sql}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    fn count(&self, sql: &str) -> u64 {
        self.query(sql).parse().unwrap_or_else(|_| panic!("a count: {sql}"))
    }
}

impl Drop for Pg {
    fn drop(&mut self) {
        let data = self.root.join("data");
        let _ = Command::new(self.bin.join("pg_ctl")).args(["-D", data.to_str().unwrap(), "-m", "immediate", "stop"]).output();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A Door with training_wheels on: its database, its spool, and a key pair the binary made
/// as SCM makes production's (`door tw-keygen`). The tick at 1 s, so the Delta tap runs soon.
fn tw_env(pg: &Pg, dir: &std::path::Path) -> Vec<(String, String)> {
    std::fs::create_dir_all(dir).unwrap();
    let key = dir.join("spool.key");
    let out = Command::new(env!("CARGO_BIN_EXE_door")).args(["tw-keygen", key.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success(), "door tw-keygen: {}", String::from_utf8_lossy(&out.stderr));
    vec![
        ("DOOR_TW_DSN".into(), pg.dsn()),
        ("DOOR_TW_SPOOL".into(), dir.join("spool").display().to_string()),
        ("DOOR_TW_SPOOL_PUB".into(), String::from_utf8(out.stdout).unwrap().trim().to_string()),
        ("DOOR_TW_SPOOL_KEY".into(), key.display().to_string()),
        ("DOOR_SYNC_SECS".into(), "1".into()),
    ]
}

fn tw_door(auth: &Auth, relay: &Relay, env: &[(String, String)], more: &[(&str, &str)]) -> Door {
    let mut all: Vec<(&str, &str)> = vec![("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())];
    all.extend(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    all.extend_from_slice(more);
    Door::start_with(&all)
}

/// Poll until `f` holds, up to `within`.
async fn until(within: Duration, what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + within;
    while !f() {
        assert!(Instant::now() < deadline, "not within {} s: {what}", within.as_secs());
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// A POST's status on a cookie, from the listed origin.
async fn post_as(http: &reqwest::Client, door: &Door, cookie: &str, route: &str, body: serde_json::Value) -> u16 {
    let r = http.post(format!("{}{route}", door.base)).header("cookie", cookie).header("origin", LISTED).json(&body).send().await.unwrap();
    r.status().as_u16()
}

/// TWG-1: every write route's answer leaves exactly one `action` row, a refusal its own, by
/// class. TWG-8 (Software Security's B1, B2): no row, and no spooled line, holds a credential
/// this run used: the words, the PRF, either session cookie, a public site's auth code, PKCE
/// verifier, DPoP proof and access token, or a kiosk's claim.
#[tokio::test(flavor = "multi_thread")]
async fn twg1_twg8_one_row_per_write_and_none_holds_a_credential() {
    let (auth, relay, pg) = (Auth::start(), Relay::start(), Pg::start());
    let dir = std::env::temp_dir().join(format!("tw-door-{}", unique()));
    let clients = clients_file(&auth.root);
    let (keys, claim) = kiosk_claim(&dir.join("kiosk"));
    let door = tw_door(&auth, &relay, &tw_env(&pg, &dir), &[("DOOR_CLIENTS", clients.to_str().unwrap()), ("DOOR_CLAIM_KEYS", keys.to_str().unwrap())]);
    let http = door.http();
    let log = std::fs::read_to_string(&door.log).unwrap_or_default();
    assert!(log.contains("door: training_wheels on"), "{log}");

    let who = sign_up(&http, &door).await;
    // A row names an account by its key's hex, as the Door's own table does.
    let pk = who.pk.trim_start_matches("ed25519:").to_string();
    let site = mint_as(&http, &door, &who.cookie, "group", "Mill Road").await;
    apply_as(&http, &door, &who.cookie, &site, "group.setProfile", serde_json::json!({ "displayName": "Mill Road Allotments", "shape": "community" })).await;
    let refused = post_as(&http, &door, &who.cookie, "/v2/apply", serde_json::json!({ "object": site, "op": "no.suchOp", "args": {} })).await;
    assert!(refused >= 400, "an unknown op is refused: {refused}");
    assert_eq!(sign_out(&http, &door, &who.cookie).await, 200);
    let again = sign_in(&http, &door, &who).await;

    // A public site's sign-in and its token exchange: a code, a verifier, a proof, a token.
    let (verifier, challenge) = pkce();
    let r = client_sign_in(&http, &door, &who, CALLBACK, &challenge).await;
    let code = code_of(r.json::<serde_json::Value>().await.unwrap()["redirect"].as_str().unwrap());
    let proof = Dpop::new().proof("POST", &format!("{}/v2/token", door.base), None);
    let r = exchange(&http, &door, Some(proof.clone()), &code, &verifier, CLIENT).await;
    assert_eq!(r.status().as_u16(), 200, "the exchange");
    let token = r.json::<serde_json::Value>().await.unwrap()["access_token"].as_str().unwrap().to_string();

    // A kiosk's claim, landed and joined: no Arc answers /v1/admit here, so the join is refused,
    // and recorded as a refusal, with the claim nowhere in it.
    let stay = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let r = stay.get(format!("{}/join?claim={claim}", door.base)).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 303, "the claim lands");
    let join = cookie_of(&r).expect("the join cookie");
    let r = http.post(format!("{}/v2/join", door.base)).header("cookie", format!("{again}; {join}")).header("origin", LISTED).send().await.unwrap();
    assert!(r.status().as_u16() >= 400, "no Arc, no admission: {}", r.status());

    let want = [("/v2/signup/finish", 1), ("/v2/mint", 1), ("/v2/apply", 2), ("/v2/signout", 1), ("/v2/signin/finish", 2), ("/v2/token", 1), ("/v2/join", 1)];
    let q = "SELECT count(*) FROM action WHERE NOT backfilled";
    until(Duration::from_secs(20), "every action row", || pg.count(q) == 9).await;
    for (route, n) in want {
        assert_eq!(pg.count(&format!("SELECT count(*) FROM action WHERE NOT backfilled AND route = '{route}'")), n, "{route}");
    }
    assert_eq!(pg.count("SELECT count(*) FROM action WHERE route = '/v2/apply' AND (outcome->>'status')::int >= 400 AND outcome->>'refusal' IS NOT NULL"), 1, "the refusal, by class");
    let seen = pg.query("SELECT string_agg(concat_ws(' ', route, account, object, kind, op, outcome::text), E'\n') FROM action");
    assert_eq!(pg.count(&format!("SELECT count(*) FROM action WHERE route = '/v2/mint' AND object = '{site}' AND account = '{pk}'")), 1, "the mint names what it made and whose it was ({site}, {pk}):\n{seen}");
    assert_eq!(pg.count(&format!("SELECT count(*) FROM action WHERE backfilled AND object = '{site}'")), 0, "what a recorded action made is not history:\n{seen}");
    // The webapp's first session, its second, and the public site's: three, three handles.
    assert_eq!(pg.count("SELECT count(DISTINCT session) FROM action WHERE NOT backfilled AND session IS NOT NULL"), 3, "a handle per session");

    // TWG-8: everything written, searched for every credential this run used.
    let rows = pg.query("SELECT coalesce(string_agg(t::text, E'\\n'), '') FROM (SELECT a::text AS t FROM action a UNION ALL SELECT d::text FROM delta d) x");
    let cookie_value = |c: &str| c.split_once('=').map(|(_, v)| v.to_string()).unwrap_or_default();
    let credentials = [
        ("the words", who.words.clone()),
        ("the first cookie", cookie_value(&who.cookie)),
        ("the second cookie", cookie_value(&again)),
        ("the prf", hex::encode(who.prf)),
        ("the auth code", code),
        ("the PKCE verifier", verifier),
        ("the DPoP proof", proof),
        ("the access token", token),
        ("the claim", claim),
        ("the join cookie", cookie_value(&join)),
    ];
    for (what, secret) in credentials {
        assert!(!secret.is_empty() && !rows.contains(&secret), "a row holds {what}");
        for f in std::fs::read_dir(dir.join("spool")).into_iter().flatten().flatten() {
            let bytes = std::fs::read(f.path()).unwrap_or_default();
            assert!(!String::from_utf8_lossy(&bytes).contains(&secret), "the spool holds {what}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// TWG-2: with the database stopped mid-run, nothing is lost and nothing is refused: the
/// writes answer, their rows wait sealed in the spool, and when the database returns the
/// spool drains, in order, to the counts the requests made.
#[tokio::test(flavor = "multi_thread")]
async fn twg2_a_database_stop_loses_nothing() {
    let (auth, relay, pg) = (Auth::start(), Relay::start(), Pg::start());
    let dir = std::env::temp_dir().join(format!("tw-door-{}", unique()));
    let door = tw_door(&auth, &relay, &tw_env(&pg, &dir), &[]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let site = mint_as(&http, &door, &who.cookie, "group", "Mill Road").await;
    until(Duration::from_secs(20), "the first two rows", || pg.count("SELECT count(*) FROM action WHERE NOT backfilled") == 2).await;

    pg.stop();
    for n in 0..3 {
        let name = format!("Mill Road {n}");
        apply_as(&http, &door, &who.cookie, &site, "group.setProfile", serde_json::json!({ "displayName": name, "shape": "community" })).await;
    }
    let spooled = std::fs::read_dir(dir.join("spool")).unwrap().flatten().map(|f| f.metadata().unwrap().len()).sum::<u64>();
    assert!(spooled > 0, "the rows wait in the spool");

    pg.resume();
    until(Duration::from_secs(45), "the spool drained", || pg.count("SELECT count(*) FROM action WHERE NOT backfilled AND route = '/v2/apply'") == 3).await;
    assert_eq!(pg.count("SELECT count(*) FROM action WHERE NOT backfilled"), 5, "no row lost, none twice");
    let names = pg.query("SELECT string_agg(args->'args'->>'displayName', ',' ORDER BY id) FROM action WHERE route = '/v2/apply'");
    assert_eq!(names, "Mill Road 0,Mill Road 1,Mill Road 2", "in order");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The Delta tap and the backfill (Ralph, 28 Sep: keep the mint, backfill from MLS): history
/// made while training_wheels was off is reported by the tap at the next sign-in, and each of
/// its Deltas seeds one backfilled action, by its author. A second sign-in, which reports the
/// whole log again, seeds nothing more.
#[tokio::test(flavor = "multi_thread")]
async fn tw_the_backfill_seeds_the_held_log_once() {
    let (auth, relay, pg) = (Auth::start(), Relay::start(), Pg::start());
    let seals = std::env::temp_dir().join(format!("tw-seals-{}", unique()));
    let seals_s = seals.display().to_string();
    // One Door, restarted with training_wheels on, as a deploy restarts door-01: the same seals
    // and the same public origin, which a seal is bound to.
    let public = format!("http://127.0.0.1:{}", TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port());
    let same: [(&str, &str); 2] = [("DOOR_SEALS", seals_s.as_str()), ("DOOR_PUBLIC", public.as_str())];
    // Before training_wheels: a Site and its profile, sealed at sign-out.
    let off = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), same[0], same[1]]);
    let http = off.http();
    let who = sign_up(&http, &off).await;
    let pk = who.pk.trim_start_matches("ed25519:").to_string();
    let site = mint_as(&http, &off, &who.cookie, "group", "Mill Road").await;
    apply_as(&http, &off, &who.cookie, &site, "group.setProfile", serde_json::json!({ "displayName": "Mill Road Allotments", "shape": "community" })).await;
    assert_eq!(sign_out(&http, &off, &who.cookie).await, 200);
    drop(off);
    // received_at is in whole seconds: the history is at least a second older than the first
    // action, as it is by hours on door-01.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let dir = std::env::temp_dir().join(format!("tw-door-{}", unique()));
    let env = tw_env(&pg, &dir);
    let on = tw_door(&auth, &relay, &env, &same);
    let http = on.http();
    let c = sign_in(&http, &on, &who).await;
    until(Duration::from_secs(30), "the tap's first report", || pg.count(&format!("SELECT count(*) FROM delta WHERE object = '{site}'")) >= 2).await;
    until(Duration::from_secs(30), "the backfill", || pg.count("SELECT count(*) FROM action WHERE backfilled") > 0).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let seeded = pg.count("SELECT count(*) FROM action WHERE backfilled");
    let seen = pg.query("SELECT string_agg(concat_ws(' ', account, object, kind, op), E'\n') FROM action WHERE backfilled");
    let log = std::fs::read_to_string(&on.log).unwrap_or_default();
    assert!(!log.contains("the sealed state is refused"), "the seal opens on the restarted Door: {log}");
    // Two: the mint's own profile and the apply's, each by its author.
    assert_eq!(pg.count(&format!("SELECT count(*) FROM action WHERE backfilled AND object = '{site}' AND op = 'group.setProfile' AND account = '{pk}'")), 2, "the profile, by its author ({site}, {pk}):\n{seen}");
    assert_eq!(
        pg.count("SELECT count(*) FROM action a WHERE backfilled AND NOT EXISTS (SELECT 1 FROM delta d WHERE d.object = a.object AND d.delta_id = a.outcome->>'delta')"),
        0,
        "every backfilled action names a Delta the tap reported"
    );

    // Again: a new process reports the whole log; nothing more is seeded.
    assert_eq!(sign_out(&http, &on, &c).await, 200);
    let deltas = pg.count("SELECT count(*) FROM delta");
    let _c = sign_in(&http, &on, &who).await;
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(pg.count("SELECT count(*) FROM action WHERE backfilled"), seeded, "a second report seeds nothing");
    assert!(pg.count("SELECT count(*) FROM delta") >= deltas);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&seals);
}

/// An Arc whose node answers `/v1/admit` with `status` and `body`, on a loopback port.
async fn arc_answering(status: u16, body: serde_json::Value) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let app = axum::Router::new().route(
        "/v1/admit",
        axum::routing::post(move || {
            let body = body.clone();
            async move { (axum::http::StatusCode::from_u16(status).unwrap(), axum::Json(body)) }
        }),
    );
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://127.0.0.1:{port}")
}

/// J-A (Ralph's morning journey): a kiosk visitor, joined by claim, ends on the Site's own
/// website. A client registration may name a `home`, an exact URL on one of its own
/// origins, and /v2/join's admitted answer carries it as `home` when the claim's Site is
/// that client's, and `home: null` when it is no client's: nothing else in the answer
/// changes, and `home` is as registered, carrying no token and no part of the claim.
async fn a_join_answers(clients: serde_json::Value) -> serde_json::Value {
    let dir = std::env::temp_dir().join(format!("door-security-ja-{}", unique()));
    let (keys, claim) = kiosk_claim(&dir);
    let file = dir.join("clients.json");
    std::fs::write(&file, clients.to_string()).unwrap();
    let arc = arc_answering(200, serde_json::json!({ "admitted": "cd", "rooms": [], "unjoined": [] })).await;
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()),
        ("DOOR_RELAY", relay.url.as_str()),
        ("DOOR_CLAIM_KEYS", keys.to_str().unwrap()),
        ("DOOR_ARC", arc.as_str()),
        ("DOOR_CLIENTS", file.to_str().unwrap()),
    ]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let stay = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let r = stay.get(format!("{}/join?claim={claim}", door.base)).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 303, "the claim lands");
    let join = cookie_of(&r).expect("the join cookie");
    let r = http.post(format!("{}/v2/join", door.base)).header("cookie", format!("{}; {join}", who.cookie)).header("origin", LISTED).send().await.unwrap();
    let (s, body) = said(r).await;
    assert_eq!(s, 200, "admitted: {body}");
    let _ = std::fs::remove_dir_all(&dir);
    serde_json::from_str(&body).unwrap()
}

/// The claim's Site is "5e"×32 (`kiosk_claim`).
fn a_client(site: &str, origin: &str, home: Option<&str>) -> serde_json::Value {
    let mut c = serde_json::json!({ "name": "Egregore's Echoes", "callbacks": [format!("{origin}/signin/callback")], "origins": [origin], "site": site });
    if let Some(h) = home {
        c["home"] = serde_json::json!(h);
    }
    c
}

/// ONE PAGE (the user, 30 Sep: "exactly one app.wallflowers.io page, which serves only the
/// passkey … and immediately back"; Antoine: "It doesn't make sense to arrive directly on an other
/// identity"). A scanned claim for a Site with a home goes back to that home, the claim in the
/// fragment only: no join cookie, no webapp, nothing cached, no Referer.
#[tokio::test(flavor = "multi_thread")]
async fn one_page_a_scanned_claim_goes_back_to_its_sites_home() {
    let dir = std::env::temp_dir().join(format!("door-security-one-page-{}", unique()));
    let (keys, claim) = kiosk_claim(&dir);
    let home = "https://egg.example.test/community";
    let file = dir.join("clients.json");
    std::fs::write(&file, serde_json::json!({ "egg.example.test": a_client(&"5e".repeat(32), "https://egg.example.test", Some(home)) }).to_string()).unwrap();
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()),
        ("DOOR_RELAY", relay.url.as_str()),
        ("DOOR_CLAIM_KEYS", keys.to_str().unwrap()),
        ("DOOR_CLIENTS", file.to_str().unwrap()),
    ]);
    let stay = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let r = stay.get(format!("{}/join?claim={claim}", door.base)).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 303);
    let header = |k: &str| r.headers().get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
    assert_eq!(header("location"), Some(format!("{home}#join={claim}")), "the Site's home, the claim in the fragment");
    assert_eq!(header("set-cookie"), None, "no join cookie: the Site joins with its own session");
    assert_eq!(header("cache-control").as_deref(), Some("no-store"));
    assert_eq!(header("referrer-policy").as_deref(), Some("no-referrer"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The one page's name (O-77's card on entry; the user, 30 Sep): the account is made under it,
/// trimmed, and a name out of its limits is refused before anything starts.
#[tokio::test(flavor = "multi_thread")]
async fn one_page_a_new_members_name_is_theirs_and_held_to_its_limits() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    for bad in ["line\nbreak".to_string(), "x".repeat(65), "   ".to_string()] {
        let body = serde_json::json!({ "name": bad, "work": work(&http, &door, "signup").await });
        let (s, b) = said(http.post(format!("{}/v2/signup", door.base)).json(&body).send().await.unwrap()).await;
        assert_eq!(s, 400, "{bad:?}: {b}");
    }
    let who = sign_up_as(&http, &door, "  Hana  ").await;
    let me: serde_json::Value = http.get(format!("{}/v2/me", door.base)).header("cookie", &who.cookie).send().await.unwrap().json().await.unwrap();
    assert_eq!(me["display_name"], "Hana", "{me}");
}

/// A Site's own session, signed in through the one page, joins by the claim it was given, in the
/// body: admitted, with the Site's home, as the webapp's join answers.
#[tokio::test(flavor = "multi_thread")]
async fn one_page_a_sites_own_session_joins_by_the_claim_it_was_given() {
    let (status, body) = a_sites_join(&"5e".repeat(32)).await;
    assert_eq!(status, 200, "admitted: {body}");
    let body: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["admitted"], true, "{body}");
    assert_eq!(body["home"], "https://client.example.test/community", "the Site's home: {body}");
}

/// A Site's session may present only a claim for its own Site.
#[tokio::test(flavor = "multi_thread")]
async fn one_page_a_sites_session_cannot_join_another_sites_claim() {
    let (status, body) = a_sites_join(&"7a".repeat(32)).await;
    assert_eq!(status, 403, "{body}");
    assert!(body.contains("a claim for another Site"), "{body}");
}

/// A client for `site` (the claim is for "5e"x32, `kiosk_claim`), its member signed in by a code
/// and a DPoP token, and POST /v2/join with the claim in the body: the status and the answer.
async fn a_sites_join(site: &str) -> (u16, String) {
    let (auth, relay) = (Auth::start(), Relay::start());
    let dir = std::env::temp_dir().join(format!("door-security-one-page-{}", unique()));
    let (keys, claim) = kiosk_claim(&dir);
    let arc = arc_answering(200, serde_json::json!({ "admitted": "cd", "rooms": [], "unjoined": [] })).await;
    let clients = dir.join("clients.json");
    let reg = serde_json::json!({ CLIENT: { "name": "", "callbacks": [CALLBACK], "origins": ["https://client.example.test"], "site": site, "home": "https://client.example.test/community" } });
    std::fs::write(&clients, reg.to_string()).unwrap();
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()),
        ("DOOR_RELAY", relay.url.as_str()),
        ("DOOR_CLAIM_KEYS", keys.to_str().unwrap()),
        ("DOOR_ARC", arc.as_str()),
        ("DOOR_CLIENTS", clients.to_str().unwrap()),
    ]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let key = Dpop::new();
    let (code, verifier) = a_code(&http, &door, &who).await;
    let r = exchange(&http, &door, Some(key.proof("POST", &format!("{}/v2/token", door.base), None)), &code, &verifier, CLIENT).await;
    let token = r.json::<serde_json::Value>().await.unwrap()["access_token"].as_str().expect("a token").to_string();
    let url = format!("{}/v2/join", door.base);
    let r = http
        .post(&url)
        .header("Authorization", format!("DPoP {token}"))
        .header("DPoP", key.proof("POST", &url, Some(&token)))
        .json(&serde_json::json!({ "claim": claim }))
        .send()
        .await
        .unwrap();
    let out = said(r).await;
    let _ = std::fs::remove_dir_all(&dir);
    (out.0, out.1)
}

#[tokio::test(flavor = "multi_thread")]
async fn ja_a_join_for_a_site_with_no_registered_home_answers_home_null() {
    for clients in [
        serde_json::json!({ "elsewhere.example.test": a_client(&"7a".repeat(32), "https://elsewhere.example.test", Some("https://elsewhere.example.test/in")) }),
        serde_json::json!({ "egg.example.test": a_client(&"5e".repeat(32), "https://egg.example.test", None) }),
    ] {
        let body = a_join_answers(clients).await;
        assert!(body.get("home").is_some_and(|h| h.is_null()), "no client for the Site, or one with no home: home null, and the webapp stays: {body}");
    }
}

/// A home off its client's own origins stops the Door at start, named, as a registration
/// that could send the visitor anywhere would otherwise go live.
#[test]
fn ja_a_home_off_its_clients_origins_refuses_the_start() {
    let dir = std::env::temp_dir().join(format!("door-security-ja-{}", unique()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("clients.json");
    std::fs::write(&file, serde_json::json!({ "egg.example.test": a_client(&"5e".repeat(32), "https://egg.example.test", Some("https://evil.example.test/welcome")) }).to_string()).unwrap();
    let (status, err) = refused_at_start(&[("DOOR_CLIENTS", file.to_str().unwrap())]);
    assert_ne!(status, Some(0), "{err}");
    assert!(err.contains("\"egg.example.test\"") && err.contains("https://evil.example.test/welcome"), "the client and its home are named: {err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The contribution record kept (Ralph's cut, 28 Sep): an admitted join's one /v2/join row holds
/// the claim's choice (`c`) and share (`a`), as the Door verified them at /join. TWG-8: no row
/// and no spooled line holds the claim, its nonce, its signature or its kiosk's id.
#[tokio::test(flavor = "multi_thread")]
async fn twg8_an_admitted_joins_row_keeps_c_and_a_and_no_more_of_the_claim() {
    let (auth, relay, pg) = (Auth::start(), Relay::start(), Pg::start());
    let dir = std::env::temp_dir().join(format!("tw-door-{}", unique()));
    let share = "k2m3n4p5q6r7s8t9uvwx";
    let (keys, claim) = kiosk_claim_with(&dir.join("kiosk"), serde_json::json!({ "c": "financial", "a": share }));
    let arc = arc_answering(200, serde_json::json!({ "admitted": "cd", "c": "financial", "a": share, "rooms": [], "unjoined": [] })).await;
    let door = tw_door(&auth, &relay, &tw_env(&pg, &dir), &[("DOOR_CLAIM_KEYS", keys.to_str().unwrap()), ("DOOR_ARC", arc.as_str())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;

    let stay = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let r = stay.get(format!("{}/join?claim={claim}", door.base)).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 303, "the claim lands");
    let join = cookie_of(&r).expect("the join cookie");
    let r = http.post(format!("{}/v2/join", door.base)).header("cookie", format!("{}; {join}", who.cookie)).header("origin", LISTED).send().await.unwrap();
    let (s, body) = said(r).await;
    assert_eq!(s, 200, "admitted: {body}");

    let q = "SELECT count(*) FROM action WHERE route = '/v2/join'";
    until(Duration::from_secs(20), "the join's row", || pg.count(q) == 1).await;
    assert_eq!(
        pg.query("SELECT concat_ws(' ', args->>'c', args->>'a', outcome->>'status') FROM action WHERE route = '/v2/join'"),
        format!("financial {share} 200"),
        "the row keeps the claim's choice and share"
    );

    // TWG-8: the claim's parts, searched for in every row and every spooled line.
    let mut parts = claim.split('.');
    let (_, payload, sig) = (parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap());
    let decoded: serde_json::Value = serde_json::from_slice(&base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, payload).unwrap()).unwrap();
    let rows = pg.query("SELECT coalesce(string_agg(t::text, E'\\n'), '') FROM (SELECT a::text AS t FROM action a UNION ALL SELECT d::text FROM delta d) x");
    for (what, secret) in [
        ("the claim", claim.clone()),
        ("its payload", payload.to_string()),
        ("its nonce", decoded["n"].as_str().unwrap().to_string()),
        ("its signature", sig.to_string()),
        ("its kiosk's id", decoded["k"].as_str().unwrap().to_string()),
    ] {
        assert!(!secret.is_empty() && !rows.contains(&secret), "a row holds {what}");
        for f in std::fs::read_dir(dir.join("spool")).into_iter().flatten().flatten() {
            let bytes = std::fs::read(f.path()).unwrap_or_default();
            assert!(!String::from_utf8_lossy(&bytes).contains(&secret), "the spool holds {what}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// O-77 through the Door: a person's card reaches a Site they are in, by the one write path,
/// and a name the webapp writes on their self record (group.setProfile, through /v2/apply) is
/// read back at once by /v2/me and /v2/graph, and published again into the Site's view.
#[tokio::test(flavor = "multi_thread")]
async fn o77_the_door_publishes_the_card_and_reads_the_new_name_back() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let who = sign_up_as(&http, &door, "Vis").await;
    let pk = who.pk.trim_start_matches("ed25519:").to_string();
    let site = mint_as(&http, &door, &who.cookie, "group", "Egregore").await;
    let graph = || {
        let (http, base, c) = (http.clone(), door.base.clone(), who.cookie.clone());
        async move { http.get(format!("{base}/v2/graph")).header("cookie", c).send().await.unwrap().json::<serde_json::Value>().await.unwrap() }
    };
    let name_in_site = |g: &serde_json::Value| -> Option<String> {
        let o = g["objects"].as_array()?.iter().find(|o| o["id"] == site.as_str())?;
        o["view"]["profiles"][pk.as_str()]["name"].as_str().map(str::to_string)
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    while name_in_site(&graph().await).as_deref() != Some("Vis") {
        assert!(Instant::now() < deadline, "the card in the Site within 20 s: {}", graph().await);
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // The webapp's name step: group.setProfile on the self record (spine index 0).
    let g = graph().await;
    let me = g["spine"].as_array().unwrap().iter().find(|e| e["index"] == 0).expect("the self record")["object"].as_str().unwrap().to_string();
    apply_as(&http, &door, &who.cookie, &me, "group.setProfile", serde_json::json!({ "displayName": "Vis Aurelia", "shape": "individual" })).await;
    let m: serde_json::Value = http.get(format!("{}/v2/me", door.base)).header("cookie", &who.cookie).send().await.unwrap().json().await.unwrap();
    assert_eq!((m["display_name"].as_str(), &m["card_note"]), (Some("Vis Aurelia"), &serde_json::Value::Null), "/v2/me reads the record: {m}");
    assert_eq!(graph().await["me"]["display_name"], "Vis Aurelia", "and /v2/graph's me");
    // Published in the write's tail, after its answer: within a few seconds, not a sync later.
    let deadline = Instant::now() + Duration::from_secs(5);
    while name_in_site(&graph().await).as_deref() != Some("Vis Aurelia") {
        assert!(Instant::now() < deadline, "the new name in the Site within 5 s of the write: {}", graph().await);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// AN ARC THAT ADMITS FOR REAL: an in-process Node on the suite's relay, a Site's member and
/// admitter once its owner adds it, and a `/v1/admit` that runs its claim admission
/// (`admit_by_claim`, which sends each joined object's history) on the bundles the Door sends,
/// as arc-node's does. PACIFIC_STATE_DIR is this process's, and this Node its one user here.
struct AdmittingArc {
    dir: PathBuf,
    pk: String,
    url: String,
}

impl AdmittingArc {
    async fn start(relay: &Relay, keys: std::collections::HashMap<String, [u8; 32]>) -> AdmittingArc {
        let dir = std::env::temp_dir().join(format!("door-security-arc-{}", unique()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("PACIFIC_STATE_DIR", &dir);
        pacific_core::router::Routes::parse(&relay.url).save().unwrap();
        pacific_core::node::set_default_arc(&relay.url).unwrap();
        let pk = hex::encode(pacific_core::Node::init_identity("the Arc").unwrap().id.identity_pk());
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
        let keys = std::sync::Arc::new(keys);
        let app = axum::Router::new().route(
            "/v1/admit",
            axum::routing::post(move |axum::Json(ask): axum::Json<serde_json::Value>| {
                let keys = keys.clone();
                async move {
                    // A Node is one thread's (its store is): the admission on a thread of its own.
                    let (status, body) = tokio::task::spawn_blocking(move || {
                        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                        rt.block_on(async {
                            let n = pacific_core::Node::open().unwrap();
                            let _ = n.sync_once().await;
                            let claim = ask["claim"].as_str().unwrap_or_default().to_string();
                            let bundles: Vec<String> = ask["bundles"].as_array().into_iter().flatten().filter_map(|b| b.as_str().map(String::from)).collect();
                            let site = pacific_core::claim::site_of(&claim).unwrap_or_default();
                            match n.admit_by_claim(&site, &claim, &bundles, &keys).await {
                                Ok(a) => (200, serde_json::json!({
                                    "admitted": hex::encode(a.member), "c": a.choice, "a": a.artifact, "rooms": a.rooms,
                                    "unjoined": a.unjoined.iter().map(|(r, w)| serde_json::json!({ "room": r, "why": w })).collect::<Vec<_>>(),
                                    "fallback": a.fallback,
                                })),
                                Err(e) => (409, serde_json::json!({ "error": e.to_string() })),
                            }
                        })
                    })
                    .await
                    .unwrap();
                    (axum::http::StatusCode::from_u16(status).unwrap(), axum::Json(body))
                }
            }),
        );
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        AdmittingArc { dir, pk, url }
    }

    fn node(&self) -> pacific_core::Node {
        std::env::set_var("PACIFIC_STATE_DIR", &self.dir);
        pacific_core::Node::open().unwrap()
    }

    fn bundle(&self) -> String {
        self.node().build_contact_bundle().unwrap()
    }

    fn sync(&self) {
        let n = self.node();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let _ = rt.block_on(n.sync_once());
    }
}

/// How D-60's test reaches a routes.json row a reader meets.
#[derive(Debug)]
enum Reach {
    /// On the wire, with each credential its auth takes.
    Wire,
    /// Called by this test's own sign-up, sign-in, token, join and sign-out.
    Exercised,
    /// The change stream: read after the retract, and again after a reconnect.
    Stream,
    /// A gateway face route: the arc-node paths it renders from, as gateway main.rs asks them.
    Face(&'static [&'static str]),
    /// A gateway route to an Arc service: its upstreams are read from the source and named.
    Proxy,
}

/// Every routes.json row a reader meets (audience site, webapp, window or arc), and how D-60's
/// test reaches it. A row with no entry fails the test.
fn d60_reach(router: &str, method: &str, path: &str) -> Option<Reach> {
    use Reach::*;
    Some(match (router, method, path) {
        ("door", "GET", "/v2/icd" | "/v2/icd/:hash" | "/v2/signin.js" | "/v2/me" | "/v2/kinds" | "/v2/graph" | "/v2/draft/:kind" | "/v2/work" | "/signin" | "/" | "/index.html" | "/v2/site/:site/items" | "/v2/resources.js" | "/v2/members" | "/v2/members/:site" | "/signin/site") => Wire,
        ("door", "POST", "/v2/mint" | "/v2/apply" | "/v2/batch" | "/v2/add" | "/v2/bundle" | "/v2/site/address" | "/v2/signup/continue" | "/v2/history" | "/signin/site") => Wire,
        ("door", "POST", "/v2/signup" | "/v2/signup/finish" | "/v2/signin" | "/v2/signin/finish" | "/v2/signout" | "/v2/token" | "/v2/join") => Exercised,
        ("door", "GET", "/join") => Exercised,
        ("door", "GET", "/v2/events") => Stream,
        ("gateway", "GET", "/v1/face/:slug" | "/v1/face/:slug/items" | "/v1/face/:slug/escape" | "/v1/face/:slug/door" | "/v1/face/:slug/brand"
            | "/v1/face/:slug/post/:id" | "/v1/face/:slug/post/:id/document") => Face(&["/faces"]),
        ("gateway", "GET", "/v1/face/:slug/m/:slot") => Face(&["/faces", "/face/{host}/m/{slot}"]),
        ("gateway", _, "/v1/health" | "/v1/arc" | "/v1/version" | "/v1/signup" | "/v1/admit" | "/v1/bundle" | "/console/*rest" | "/tether" | "/bundle"
            | "/v1/arcs" | "/v1/systems/*rest" | "/v1/land/*rest" | "/v1/box/*rest" | "/v1/wake/*rest" | "/v1/console/*rest" | "/v1/relay") => Proxy,
        _ => return None,
    })
}

/// A top-level fn's body in `src`.
fn d60_fn<'a>(src: &'a str, name: &str) -> Option<&'a str> {
    let at = ["\nasync fn ", "\nfn "].iter().filter_map(|p| src.find(&format!("{p}{name}("))).min()?;
    let rest = &src[at + 1..];
    Some(&rest[..rest.find("\n}\n").map_or(rest.len(), |e| e + 2)])
}

/// What a gateway handler asks upstream, following the fns it calls three deep: each
/// `format!("{base}<path>")` as its path, each `forward(&gw.<plane>` as `forward:<plane>`.
fn d60_upstreams(src: &str, handler: &str) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut todo = vec![(handler.to_string(), 0)];
    while let Some((f, depth)) = todo.pop() {
        if !seen.insert(f.clone()) {
            continue;
        }
        let Some(body) = d60_fn(src, &f) else { continue };
        for (i, _) in body.match_indices("format!(\"{base}") {
            out.insert(body[i + "format!(\"{base}".len()..].chars().take_while(|c| *c != '"').collect());
        }
        for (i, _) in body.match_indices("forward(&gw.") {
            out.insert(format!("forward:{}", body[i + "forward(&gw.".len()..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect::<String>()));
        }
        if depth < 3 {
            let mut word = String::new();
            for (j, c) in body.char_indices() {
                if c.is_alphanumeric() || c == '_' {
                    word.push(c);
                    continue;
                }
                if c == '(' && !word.is_empty() && body[..j].chars().rev().nth(word.len()) != Some('.') && d60_fn(src, &word).is_some() {
                    todo.push((word.clone(), depth + 1));
                }
                word.clear();
            }
        }
    }
    out
}

/// The marker as a reader could be served it: raw, and inside base64 and base64url at each of
/// the three byte alignments, as the characters that depend on the marker alone.
fn d60_forms(marker: &str) -> Vec<String> {
    use base64::Engine;
    let mut out = vec![marker.to_string()];
    for eng in [&base64::engine::general_purpose::STANDARD, &base64::engine::general_purpose::URL_SAFE_NO_PAD] {
        for k in 0..3usize {
            let mut b = vec![0u8; k];
            b.extend_from_slice(marker.as_bytes());
            // The characters made of the marker's bits alone: after the prefix's, before the tail's.
            let s = eng.encode(&b);
            out.push(s[(8 * k).div_ceil(6)..8 * (k + marker.len()) / 6].to_string());
        }
    }
    out
}

/// The encoded forms find the marker inside base64 and base64url of a longer payload, whatever
/// byte it starts on, and not in a payload without it.
#[test]
fn d60_the_markers_forms_find_it_encoded_at_any_alignment() {
    use base64::Engine;
    let marker = "d60retractedabc123";
    let forms = d60_forms(marker);
    for eng in [&base64::engine::general_purpose::STANDARD, &base64::engine::general_purpose::URL_SAFE_NO_PAD] {
        for k in 0..6usize {
            let payload = [vec![b'x'; k], marker.as_bytes().to_vec(), b"{\"tail\":1}".to_vec()].concat();
            let s = eng.encode(&payload);
            assert!(forms.iter().any(|f| s.contains(f.as_str())), "{k} bytes before it: {s}");
        }
        let s = eng.encode(b"d60retractedabc124 and nothing else");
        assert!(!forms.iter().skip(1).any(|f| s.contains(f.as_str())), "a near miss is not the marker: {s}");
    }
}

/// Who reads: the webapp's cookie, a site's token (its DPoP key), or no one.
enum D60Who<'a> {
    Cookie(&'a str),
    Token(&'a Dpop, &'a str),
    Nobody,
}

/// What D-60's reads need of the room: its id, the kept message to react to, the ICD's hash.
struct D60Room {
    site: String,
    room: String,
    keep_author: String,
    keep_gen: i64,
    icd_hash: String,
}

/// A site token for `who`, with the DPoP key it is bound to.
async fn d60_token(http: &reqwest::Client, door: &Door, who: &Account) -> (Dpop, String) {
    let key = Dpop::new();
    let (code, verifier) = a_code(http, door, who).await;
    let r = exchange(http, door, Some(key.proof("POST", &format!("{}/v2/token", door.base), None)), &code, &verifier, CLIENT).await;
    let t = r.json::<serde_json::Value>().await.unwrap()["access_token"].as_str().expect("a token").to_string();
    (key, t)
}

async fn d60_call(http: &reqwest::Client, door: &Door, who: &D60Who<'_>, method: &str, path: &str, body: Option<serde_json::Value>) -> (u16, String) {
    let url = format!("{}{path}", door.base);
    let mut rq = http.request(method.parse().unwrap(), &url).header("origin", LISTED);
    match who {
        D60Who::Cookie(c) => rq = rq.header("cookie", *c),
        D60Who::Token(key, token) => rq = rq.header("Authorization", format!("DPoP {token}")).header("DPoP", key.proof(method, &url, Some(*token))),
        D60Who::Nobody => {}
    }
    if let Some(b) = body {
        rq = rq.json(&b);
    }
    said(rq.send().await.unwrap()).await
}

/// Every Door row on the wire as `who`, and the change stream, read and read again after a
/// reconnect while a write moves it: each answer, labelled.
async fn d60_read(http: &reqwest::Client, door: &Door, rows: &[serde_json::Value], who: &D60Who<'_>, label: &str, r: &D60Room) -> Vec<(String, String)> {
    let mut got = vec![];
    let react = |emoji: &str| serde_json::json!({ "target_author": r.keep_author, "target_gen": r.keep_gen, "emoji": emoji, "active": 1 });
    for row in rows {
        let (method, path, auth) = (row["method"].as_str().unwrap(), row["path"].as_str().unwrap(), row["auth"].as_str().unwrap_or(""));
        if row["router"] != "door" || !matches!(d60_reach("door", method, path), Some(Reach::Wire)) {
            continue;
        }
        // A route that takes no credential is read once, by no one.
        if (auth == "none") != matches!(who, D60Who::Nobody) {
            continue;
        }
        let at = path.replace(":hash", &r.icd_hash).replace(":kind", "forum").replace(":site", &r.site);
        let body = match (method, path) {
            ("POST", "/v2/mint") => Some(serde_json::json!({ "kind": "thing", "draft": { "name": "d60 control" } })),
            ("POST", "/v2/apply") => Some(serde_json::json!({ "object": r.room, "op": "forum.react", "args": react("🌱") })),
            ("POST", "/v2/batch") => Some(serde_json::json!({ "steps": [{ "do": "apply", "object": r.room, "op": "forum.react", "args": react("👍") }] })),
            // The owner's history send, dry: counts only.
            ("POST", "/v2/history") => Some(serde_json::json!({ "object": r.room, "bundle": "", "dry": true })),
            ("POST", _) => Some(serde_json::json!({})),
            _ => None,
        };
        let (s, text) = d60_call(http, door, who, method, &at, body).await;
        got.push((format!("{label} {method} {at} {s}"), text));
    }
    if !matches!(who, D60Who::Nobody) {
        for pass in ["stream", "stream, reconnected"] {
            let url = format!("{}/v2/events", door.base);
            let mut rq = http.get(&url);
            match who {
                D60Who::Cookie(c) => rq = rq.header("cookie", *c),
                D60Who::Token(key, token) => rq = rq.header("Authorization", format!("DPoP {token}")).header("DPoP", key.proof("GET", &url, Some(*token))),
                D60Who::Nobody => {}
            }
            let mut stream = rq.send().await.unwrap();
            let (s, _) = d60_call(http, door, who, "POST", "/v2/apply", Some(serde_json::json!({ "object": r.room, "op": "forum.react", "args": react("🔥") }))).await;
            assert_eq!(s, 200, "{label}: a write to move the stream");
            let mut text = String::new();
            let until = Instant::now() + Duration::from_secs(3);
            while let Ok(Ok(Some(chunk))) = tokio::time::timeout(until.saturating_duration_since(Instant::now()), stream.chunk()).await {
                text.push_str(&String::from_utf8_lossy(&chunk));
            }
            got.push((format!("{label} GET /v2/events ({pass})"), text));
        }
    }
    got
}

/// Each of `who`'s sessions signed out, each answer labelled.
async fn d60_sign_out(http: &reqwest::Client, door: &Door, label: &str, whos: &[D60Who<'_>]) -> Vec<(String, String)> {
    let mut got = vec![];
    for who in whos {
        let (s, body) = d60_call(http, door, who, "POST", "/v2/signout", None).await;
        assert!(s < 300, "{label} signs out: {s} {body}");
        got.push((format!("{label} POST /v2/signout"), body));
    }
    got
}

/// The room's messages as `who`'s /v2/graph folds them: (author, gen, text).
async fn d60_messages(http: &reqwest::Client, door: &Door, who: &D60Who<'_>, room: &str) -> Vec<(String, i64, String)> {
    let (_, body) = d60_call(http, door, who, "GET", "/v2/graph", None).await;
    let g: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
    g["objects"].as_array().into_iter().flatten().filter(|o| o["id"] == room).flat_map(|o| o["view"]["messages"].as_array().cloned().unwrap_or_default())
        .map(|m| (m["author"].as_str().unwrap_or("").to_string(), m["gen"].as_i64().unwrap_or(-1), m["text"].as_str().unwrap_or("").to_string()))
        .collect()
}

/// D-60 (Ralph, 30 Sep: "D-60 is fine, as long as it's not visible, or served by CommunityAPI
/// for the room"): A RETRACTED POST'S WORDS ARE SERVED BY NO ROUTE, TO ANY READER.
///
/// Every routes.json row a reader meets (audience site, webapp, window, arc) has a way in, in
/// `d60_reach`; a row without one fails, so a route added later cannot escape. The Door's rows are
/// called on the wire with each credential their auth takes, the webapp's cookie and a site's
/// token, and every answer is searched for the post's marker, raw and inside base64 and base64url
/// at every alignment. The readers: the owner, who retracted it; a visitor the Arc admits after it
/// by a kiosk claim, fed the room's history (`admit_by_claim`); the same visitor after the owner's
/// Door restates for members who joined late (`restate_for_joiners`). The gateway's face routes
/// render only what arc-node serves from the Arc's Node: their upstreams are read from gateway
/// main.rs and held to `d60_reach`, and on the Arc's Node `host_sites` holds no marker and
/// `host_media` answers nothing for a room, while `history_of(room)` does hold the post (the
/// control: the words exist, and no route serves them). The rest of the gateway's rows reach Arc
/// services; each one's upstreams are read from the source, and none is a face's.
#[tokio::test(flavor = "multi_thread")]
async fn d60_a_retracted_posts_words_are_served_by_no_route() {
    let marker = format!("d60retracted{}", unique().replace(['-', '_'], ""));
    let keep = format!("d60kept{}", unique().replace(['-', '_'], ""));
    let forms = d60_forms(&marker);
    let rows: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/routes.json")).unwrap()).unwrap();
    let rows: Vec<serde_json::Value> = rows.into_iter().filter(|r| ["site", "webapp", "window", "arc"].contains(&r["audience"].as_str().unwrap_or(""))).collect();
    let key = |r: &serde_json::Value| (r["router"].as_str().unwrap_or("").to_string(), r["method"].as_str().unwrap_or("").to_string(), r["path"].as_str().unwrap_or("").to_string());
    assert!(rows.iter().any(|r| r["path"] == "/v2/graph") && rows.iter().any(|r| r["router"] == "gateway"), "routes.json was read: {} rows", rows.len());
    let unreached: Vec<_> = rows.iter().map(key).filter(|(ro, m, p)| d60_reach(ro, m, p).is_none()).collect();
    assert!(unreached.is_empty(), "routes.json rows a reader meets that D-60's test does not reach; give each a way in d60_reach: {unreached:?}");

    // THE GATEWAY: each face route asks upstream exactly what d60_reach says; no other row asks a face's.
    let gw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../arc/gateway/src/main.rs")).unwrap();
    for r in rows.iter().filter(|r| r["router"] == "gateway") {
        let (ro, m, p) = key(r);
        let ups = d60_upstreams(&gw, r["handler"].as_str().unwrap_or(""));
        match d60_reach(&ro, &m, &p).unwrap() {
            Reach::Face(want) => assert_eq!(ups, want.iter().map(|s| s.to_string()).collect(), "{p}: its upstreams, read from gateway main.rs"),
            _ => {
                assert!(ups.iter().all(|u| !u.starts_with("/face")), "{p} asks a face's upstream: {ups:?}");
                eprintln!("d60: gateway {m} {p} → {ups:?}");
            }
        }
    }

    let (auth, relay) = (Auth::start(), Relay::start());
    // THE SITE, its Host and a room, the Arc its admitter; two posts, and one taken back.
    let (owner, site, room, arc, keys, claim) = {
        let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
        let http = door.http();
        let owner = sign_up(&http, &door).await;
        let site = mint_as(&http, &door, &owner.cookie, "group", "D-60").await;
        let host = mint_as(&http, &door, &owner.cookie, "host", "D-60").await;
        let room = mint_as(&http, &door, &owner.cookie, "forum", "general").await;
        let at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        for (part, role) in [(&host, "host"), (&room, "room")] {
            apply_as(&http, &door, &owner.cookie, &site, "base.setPart", serde_json::json!({ "part": part, "role": role, "at": at })).await;
            apply_as(&http, &door, &owner.cookie, part, "base.setParent", serde_json::json!({ "parent": site, "role": role, "at": at })).await;
        }
        let dir = std::env::temp_dir().join(format!("door-security-d60-{}", unique()));
        let (keys, claim) = kiosk_claim_with(&dir, serde_json::json!({ "s": site }));
        let kiosk: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&keys).unwrap()).unwrap();
        let stand_in = kiosk
            .as_object()
            .unwrap()
            .iter()
            .map(|(kid, k)| (kid.clone(), <[u8; 32]>::try_from(base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, k.as_str().unwrap()).unwrap()).unwrap()))
            .collect();
        let arc = AdmittingArc::start(&relay, stand_in).await;
        // The Arc on the Site, its Host and the room BEFORE anything is said, so it holds the words.
        for object in [&site, &host, &room] {
            let r = http.post(format!("{}/v2/add", door.base)).header("cookie", &owner.cookie).header("origin", LISTED)
                .json(&serde_json::json!({ "object": object, "bundle": arc.bundle() })).send().await.unwrap();
            let (s, body) = said(r).await;
            assert_eq!(s, 200, "the Arc added: {body}");
        }
        for object in [&site, &room] {
            apply_as(&http, &door, &owner.cookie, object, "base.setRole", serde_json::json!({ "member": arc.pk, "role": "admitter" })).await;
        }
        apply_as(&http, &door, &owner.cookie, &host, "base.publish", serde_json::json!({ "slug": format!("d60-{}", unique()), "publisher": arc.pk })).await;
        for text in [&keep, &marker] {
            apply_as(&http, &door, &owner.cookie, &room, "forum.post", serde_json::json!({ "text": text, "ts": at })).await;
        }
        let me = D60Who::Cookie(&owner.cookie);
        let (author, gen, _) = d60_messages(&http, &door, &me, &room).await.into_iter().find(|m| m.2 == marker).expect("the post, before it is taken back");
        apply_as(&http, &door, &owner.cookie, &room, "forum.retract", serde_json::json!({ "target_author": author, "target_gen": gen })).await;
        let a = &arc;
        tokio::task::block_in_place(|| a.sync());
        end_session(&http, &door, &owner.cookie).await;
        (owner, site, room, arc, keys, claim)
    };

    // THE SITE'S CLIENT, its kiosk, and its Arc.
    let clients = auth.root.join("clients-d60.json");
    std::fs::write(&clients, serde_json::json!({ CLIENT: { "name": "", "callbacks": [CALLBACK], "origins": ["https://client.example.test"], "site": site } }).to_string()).unwrap();
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap()),
        ("DOOR_CLAIM_KEYS", keys.to_str().unwrap()), ("DOOR_ARC", arc.url.as_str()),
    ]);
    let http = door.http();
    let (_, icd_hash) = {
        let r = http.get(format!("{}/v2/icd", door.base)).send().await.unwrap();
        let etag = r.headers().get("etag").and_then(|v| v.to_str().ok()).unwrap_or("").trim_matches('"').trim_end_matches("-gz").to_string();
        (said(r).await, etag)
    };
    let mut seen: Vec<(String, String)> = vec![];

    // 1. THE OWNER, who was there: the webapp's cookie and the site's token, and the routes no one needs a credential for.
    let cookie = sign_in(&http, &door, &owner).await;
    let me = D60Who::Cookie(&cookie);
    let held = {
        let by = Instant::now() + Duration::from_secs(30);
        loop {
            let m = d60_messages(&http, &door, &me, &room).await;
            if m.iter().any(|x| x.2 == keep) || Instant::now() > by {
                break m;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    };
    let (keep_author, keep_gen, _) = held.iter().find(|m| m.2 == keep).cloned().expect("the owner's room holds the kept post (the control)");
    let r = D60Room { site: site.clone(), room: room.clone(), keep_author, keep_gen, icd_hash };
    let (key, t) = d60_token(&http, &door, &owner).await;
    seen.extend(d60_read(&http, &door, &rows, &me, "owner cookie", &r).await);
    seen.extend(d60_read(&http, &door, &rows, &D60Who::Token(&key, &t), "owner token", &r).await);
    seen.extend(d60_read(&http, &door, &rows, &D60Who::Nobody, "no one", &r).await);
    // Both of the owner's sessions end, so its process does: nothing restates while the visitor joins.
    seen.extend(d60_sign_out(&http, &door, "owner", &[D60Who::Cookie(&cookie), D60Who::Token(&key, &t)]).await);

    // 2. A VISITOR, admitted after it by the kiosk's claim, fed the room's history by the Arc
    //    alone: no process of the owner's is running to restate.
    let visitor = sign_up_as(&http, &door, "d60 visitor").await;
    let stay = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let j = stay.get(format!("{}/join?claim={claim}", door.base)).send().await.unwrap();
    let join = cookie_of(&j).expect("the claim lands");
    let (s, body) = said(http.post(format!("{}/v2/join", door.base)).header("cookie", format!("{}; {join}", visitor.cookie)).header("origin", LISTED).send().await.unwrap()).await;
    assert_eq!(s, 200, "joined: {body}");
    seen.push(("visitor POST /v2/join".into(), body));
    let vme = D60Who::Cookie(&visitor.cookie);
    let by = Instant::now() + Duration::from_secs(60);
    while !d60_messages(&http, &door, &vme, &room).await.iter().any(|m| m.2 == keep) {
        assert!(Instant::now() < by, "the visitor's room never held the kept post: the history bundle did not arrive (the control)");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let (vkey, vt) = d60_token(&http, &door, &visitor).await;
    seen.extend(d60_read(&http, &door, &rows, &vme, "visitor cookie, by history", &r).await);
    seen.extend(d60_read(&http, &door, &rows, &D60Who::Token(&vkey, &vt), "visitor token, by history", &r).await);
    seen.extend(d60_sign_out(&http, &door, "visitor", &[D60Who::Cookie(&visitor.cookie), D60Who::Token(&vkey, &vt)]).await);

    // 3. The owner's Door, the only process running, opens and restates for the member who
    //    joined late; then the visitor reads again.
    let restated = || std::fs::read_to_string(&door.log).unwrap_or_default().matches("restated for members who joined late").count();
    let before = restated();
    let cookie = sign_in(&http, &door, &owner).await;
    until(Duration::from_secs(90), "the owner's Door restated for the late joiner", || restated() > before).await;
    let vcookie = sign_in(&http, &door, &visitor).await;
    let vme = D60Who::Cookie(&vcookie);
    let (vkey, vt) = d60_token(&http, &door, &visitor).await;
    seen.extend(d60_read(&http, &door, &rows, &vme, "visitor cookie, after the restate", &r).await);
    seen.extend(d60_read(&http, &door, &rows, &D60Who::Token(&vkey, &vt), "visitor token, after the restate", &r).await);
    seen.extend(d60_sign_out(&http, &door, "visitor", &[D60Who::Cookie(&vcookie), D60Who::Token(&vkey, &vt)]).await);
    seen.extend(d60_sign_out(&http, &door, "owner", &[D60Who::Cookie(&cookie)]).await);

    // THE ARC'S NODE, the faces' one source: it holds the post (the control), and serves it to no face.
    tokio::task::block_in_place(|| arc.sync());
    let n = arc.node();
    let me_arc: [u8; 32] = hex::decode(&arc.pk).unwrap().try_into().unwrap();
    let (bundle, _) = n.history_of(&room, &random32(), &random32()).expect("the room's history, as the Arc holds it");
    let texts: Vec<String> = bundle.into_iter().flat_map(|b| b.rows).filter_map(|row| pacific_core::coordinator::decode_delta(&row.envelope).ok())
        .filter_map(|d| match d.args.get("text") { Some(pacific_core::coordinator::ArgVal::Text(t)) => Some(t.clone()), _ => None }).collect();
    assert!(texts.contains(&marker), "the Arc's Node holds the retracted post's row (the control): {texts:?}");
    seen.push(("the Arc's host_sites (/faces)".into(), format!("{:?}", n.host_sites(&me_arc).expect("the Arc's faces"))));
    for slot in ["mark", "banner", "cover", "icon", "0"] {
        assert!(!matches!(n.host_media(&room, slot, &me_arc), Ok(Some(_))), "host_media answers for a room's slot {slot}: a room is not a Host");
    }

    // NO ANSWER, TO ANY READER, CARRIES THE WORDS.
    let served: Vec<String> = seen.iter().filter(|(_, body)| forms.iter().any(|f| body.contains(f.as_str()))).map(|(what, _)| what.clone()).collect();
    assert!(seen.len() > 60, "the routes were read: {} answers", seen.len());
    assert!(served.is_empty(), "a retracted post's words were served by: {served:?}");
    eprintln!("d60: {} answers searched, none carries the words", seen.len());
}

/// W-98 MEMBERS, THE COMMON ROUTE (Ralph, 30 Sep: "a common route with egregore"): the
/// webapp's session names the Site (`GET /v2/members/<id>`) and must; a registered site's token
/// reads its own Site's members (`GET /v2/members`, its scope, or its own id), and another
/// Site's are out of its reach, as they are in /v2/graph.
#[tokio::test(flavor = "multi_thread")]
async fn w98_members_the_common_route_reads_a_sites_members_as_far_as_the_session_reaches() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let bare = |pk: &str| pk.trim_start_matches("ed25519:").to_ascii_lowercase();
    let (who, site, other) = {
        let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
        let http = door.http();
        let who = sign_up(&http, &door).await;
        let site = mint_as(&http, &door, &who.cookie, "group", "the Site").await;
        let other = mint_as(&http, &door, &who.cookie, "group", "another Site").await;
        let get = |path: String| {
            let (http, url, cookie) = (http.clone(), format!("{}{path}", door.base), who.cookie.clone());
            async move { said(http.get(&url).header("cookie", cookie).send().await.unwrap()).await }
        };
        let (s, body) = get(format!("/v2/members/{site}")).await;
        assert_eq!(s, 200, "{body}");
        let out: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(out["site"], site.as_str());
        assert_eq!(out["me"], bare(&who.pk));
        assert_eq!(out["can_define"], true, "the owner may define");
        let owner = out["members"].as_array().unwrap().iter().find(|m| m["key"] == bare(&who.pk)).expect("the owner is a member");
        assert_eq!(owner["role"], "owner");
        assert!(!body.contains("\"gen\""), "no gen in what a page is shown: {body}");
        let (s, body) = get("/v2/members".into()).await;
        assert_eq!((s, body.as_str()), (400, "name the Site: GET /v2/members/<site id>"));
        let (s, body) = get(format!("/v2/members/{}", "0".repeat(64))).await;
        assert_eq!((s, body.as_str()), (404, "no such Site in reach"));
        end_session(&http, &door, &who.cookie).await;
        (who, site, other)
    };
    let clients = auth.root.join("clients-w98-members.json");
    let reg = serde_json::json!({ CLIENT: { "name": "", "callbacks": [CALLBACK], "origins": ["https://client.example.test"], "site": site } });
    std::fs::write(&clients, reg.to_string()).unwrap();
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap())]);
    let http = door.http();
    let key = Dpop::new();
    let (code, verifier) = a_code(&http, &door, &who).await;
    let r = exchange(&http, &door, Some(key.proof("POST", &format!("{}/v2/token", door.base), None)), &code, &verifier, CLIENT).await;
    let token = r.json::<serde_json::Value>().await.unwrap()["access_token"].as_str().expect("a token").to_string();
    let get = |path: String| {
        let (http, url, token) = (http.clone(), format!("{}{path}", door.base), token.clone());
        let proof = key.proof("GET", &url, Some(&token));
        async move { said(http.get(&url).header("Authorization", format!("DPoP {token}")).header("DPoP", proof).send().await.unwrap()).await }
    };
    for path in ["/v2/members".to_string(), format!("/v2/members/{site}")] {
        let (s, body) = get(path.clone()).await;
        assert_eq!(s, 200, "{path}: {body}");
        let out: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(out["site"], site.as_str(), "{path}");
        assert!(out["members"].as_array().unwrap().iter().any(|m| m["key"] == bare(&who.pk) && m["role"] == "owner"), "{path}: {body}");
    }
    let (s, body) = get(format!("/v2/members/{other}")).await;
    assert_eq!((s, body.as_str()), (404, "no such Site in reach"), "another Site is out of the token's reach");
}

/// AN ARC'S PUBLIC FACE ITEMS (the gateway's GET /v1/face/:slug/items): `items` for `slug`, 404
/// for any other, and every slug it was asked for.
async fn face_items_arc(slug: &str, items: serde_json::Value) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let (served, log) = (slug.to_string(), asked.clone());
    let app = axum::Router::new().route(
        "/v1/face/:slug/items",
        axum::routing::get(move |axum::extract::Path(s): axum::extract::Path<String>| {
            let (served, items, log) = (served.clone(), items.clone(), log.clone());
            async move {
                log.lock().unwrap().push(s.clone());
                if s == served {
                    (axum::http::StatusCode::OK, axum::Json(serde_json::json!({ "items": items })))
                } else {
                    (axum::http::StatusCode::NOT_FOUND, axum::Json(serde_json::json!("no such page")))
                }
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (url, asked)
}

/// W-98: GET /v2/site/:site/items RELAYS A SITE'S PUBLIC FACE ITEMS TO ITS MEMBERS, from this
/// origin (the webapp's connect-src is 'self'): the events and posts a Site made, which its
/// members do not hold. Its address comes from what the member's Node holds (the Host's
/// publication) or this Door does (a client's registered slug), never from the request. A
/// member gets the Arc's items, unchanged, and nothing else; a non-member is refused in words; a
/// site's token is refused (the webapp's cookie only); a Site with no published Face is 404.
#[tokio::test(flavor = "multi_thread")]
async fn w98_a_sites_items_are_relayed_to_its_members_only() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let slug = format!("items-{}", unique()).to_lowercase().replace('_', "-");
    let items = serde_json::json!({
        "event:1111": { "title": "Seed swap", "startMs": 1790000000000i64, "at": 1790000000000i64 },
        "post:2222": { "title": "Sowing dates", "body": "March to May", "at": 1790000000001i64 },
    });
    let (arc, asked) = face_items_arc(&slug, items.clone()).await;
    // A SITE, its Host published at `slug`; a second Site with no Host, whose client names `slug`;
    // a third with neither.
    let (owner, site, bare_site, faceless) = {
        let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
        let http = door.http();
        let owner = sign_up(&http, &door).await;
        let site = mint_as(&http, &door, &owner.cookie, "group", "Items").await;
        let host = mint_as(&http, &door, &owner.cookie, "host", "Items").await;
        let at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        apply_as(&http, &door, &owner.cookie, &site, "base.setPart", serde_json::json!({ "part": host, "role": "host", "at": at })).await;
        apply_as(&http, &door, &owner.cookie, &host, "base.setParent", serde_json::json!({ "parent": site, "role": "host", "at": at })).await;
        let me = owner.pk.trim_start_matches("ed25519:").to_string();
        apply_as(&http, &door, &owner.cookie, &host, "base.publish", serde_json::json!({ "slug": slug, "publisher": me })).await;
        let bare_site = mint_as(&http, &door, &owner.cookie, "group", "Registered").await;
        let faceless = mint_as(&http, &door, &owner.cookie, "group", "Faceless").await;
        end_session(&http, &door, &owner.cookie).await;
        (owner, site, bare_site, faceless)
    };
    let clients = auth.root.join("clients-items.json");
    std::fs::write(&clients, serde_json::json!({
        CLIENT: { "name": "", "callbacks": [CALLBACK], "origins": ["https://client.example.test"], "site": site },
        "registered": { "name": "", "callbacks": ["https://registered.example.test/cb"], "origins": ["https://registered.example.test"], "site": bare_site, "slug": slug },
    }).to_string()).unwrap();
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap()), ("DOOR_ARC", arc.as_str())]);
    let http = door.http();
    let get = |cookie: &str, site: &str| {
        let rq = http.get(format!("{}/v2/site/{site}/items", door.base)).header("cookie", cookie.to_string()).header("origin", LISTED);
        async move { said(rq.send().await.unwrap()).await }
    };

    // A MEMBER: the Arc's items for the Site's published address, unchanged, and nothing more.
    let cookie = sign_in(&http, &door, &owner).await;
    let (s, body) = get(&cookie, &site).await;
    assert_eq!(s, 200, "a member: {body}");
    let got: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(got, serde_json::json!({ "slug": slug, "items": items }), "the Arc's items, as its Face serves them");
    assert_eq!(asked.lock().unwrap().as_slice(), [slug.clone()], "the Arc was asked for the Site's own address");
    // Its address from this Door's registry, where no Host the member holds names it.
    let (s, body) = get(&cookie, &bare_site).await;
    assert_eq!((s, serde_json::from_str::<serde_json::Value>(&body).ok()), (200, Some(serde_json::json!({ "slug": slug, "items": items }))), "by the client's slug: {body}");
    // No address anywhere: no published Face.
    let (s, body) = get(&cookie, &faceless).await;
    assert_eq!(s, 404, "a Site with no published Face: {body}");

    // A SITE'S TOKEN: refused, the webapp's cookie only.
    let key = Dpop::new();
    let (code, verifier) = a_code(&http, &door, &owner).await;
    let r = exchange(&http, &door, Some(key.proof("POST", &format!("{}/v2/token", door.base), None)), &code, &verifier, CLIENT).await;
    let token = r.json::<serde_json::Value>().await.unwrap()["access_token"].as_str().expect("a token").to_string();
    let url = format!("{}/v2/site/{site}/items", door.base);
    let r = http.get(&url).header("Authorization", format!("DPoP {token}")).header("DPoP", key.proof("GET", &url, Some(&token))).send().await.unwrap();
    let (s, body) = said(r).await;
    assert_eq!(s, 403, "a site's token: {body}");
    assert!(!body.contains("event:1111"), "and nothing of the items: {body}");
    end_session(&http, &door, &cookie).await;

    // A NON-MEMBER: refused in words, and the Arc not asked.
    let before = asked.lock().unwrap().len();
    let stranger = sign_up(&http, &door).await;
    let (s, body) = get(&stranger.cookie, &site).await;
    assert_eq!(s, 403, "a non-member: {body}");
    assert!(!body.is_empty() && !body.contains("event:1111"), "refused in words, and nothing of the items: {body}");
    assert_eq!(asked.lock().unwrap().len(), before, "the Arc is not asked for a non-member");
}

// ─── R3.1 (B): DOOR_CLIENTS is read again without a restart (pdr/team-registration.md) ─────────

/// How long a test waits for the Door to take a rewritten DOOR_CLIENTS: its poll, with room.
const RELOAD_WAIT: Duration = Duration::from_secs(20);

/// The status of the window a site's page opens for `client` at `callback` (GET /signin).
async fn window_status(http: &reqwest::Client, door: &Door, client: &str, callback: &str) -> u16 {
    let (_, challenge) = pkce();
    let mut u = reqwest::Url::parse(&format!("{}/signin", door.base)).unwrap();
    u.query_pairs_mut()
        .append_pair("client", client)
        .append_pair("redirect_uri", callback)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", "st");
    http.get(u).send().await.unwrap().status().as_u16()
}

/// Whether a credentialed preflight from `origin` to /v2/token is granted, by name.
async fn granted(http: &reqwest::Client, door: &Door, origin: &str) -> bool {
    let pre = preflight(http, door, "/v2/token", origin).await;
    pre.headers().get("access-control-allow-origin").and_then(|v| v.to_str().ok()) == Some(origin)
}

/// DOOR_CLIENTS rewritten as an operator's tool writes it: beside, then renamed over.
fn rewrite(path: &std::path::Path, text: &str) {
    let next = path.with_extension("next");
    std::fs::write(&next, text).unwrap();
    std::fs::rename(&next, path).unwrap();
}

/// Waits for `ok` within RELOAD_WAIT, or fails naming `what`.
async fn within<F, Fut>(what: &str, ok: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let by = Instant::now() + RELOAD_WAIT;
    while !ok().await {
        assert!(Instant::now() < by, "{what}: not within {} s, without a restart", RELOAD_WAIT.as_secs());
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// The Door's log lines that refuse DOOR_CLIENTS.
fn clients_refused(door: &Door) -> Vec<String> {
    std::fs::read_to_string(&door.log)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains("DOOR_CLIENTS") && l.contains("refused"))
        .map(str::to_string)
        .collect()
}

const ADDED: &str = "added";
const ADDED_CALLBACK: &str = "https://added.example.test/cb";
const ADDED_ORIGIN: &str = "https://added.example.test";
const CLIENT_ORIGIN: &str = "https://client.example.test";

fn registration(with_added: bool) -> serde_json::Value {
    let mut reg = serde_json::json!({ CLIENT: { "name": "", "callbacks": [CALLBACK], "origins": [CLIENT_ORIGIN], "site": "" } });
    if with_added {
        reg[ADDED] = serde_json::json!({ "name": "", "callbacks": [ADDED_CALLBACK], "origins": [ADDED_ORIGIN], "site": "" });
    }
    reg
}

/// R3.1 (B): a client added to DOOR_CLIENTS is honoured by the window and its origin by CORS,
/// with no restart: the same process, which started once.
#[tokio::test(flavor = "multi_thread")]
async fn r31b_a_client_added_to_door_clients_is_honoured_without_a_restart() {
    let root = std::env::temp_dir().join(format!("door-security-reload-{}", unique()));
    std::fs::create_dir_all(&root).unwrap();
    let clients = root.join("clients.json");
    std::fs::write(&clients, registration(false).to_string()).unwrap();
    let mut door = Door::start_with(&[("DOOR_CLIENTS", clients.to_str().unwrap())]);
    let http = door.http();
    let (h, d) = (&http, &door);
    assert_eq!(window_status(&http, &door, CLIENT, CALLBACK).await, 200, "the control: the registered client's window");
    assert!(granted(&http, &door, CLIENT_ORIGIN).await, "the control: the registered origin");
    assert_eq!(window_status(&http, &door, ADDED, ADDED_CALLBACK).await, 400, "not yet registered");
    assert!(!granted(&http, &door, ADDED_ORIGIN).await, "not yet registered");

    rewrite(&clients, &registration(true).to_string());
    within("the added client's window", || async move { window_status(h, d, ADDED, ADDED_CALLBACK).await == 200 }).await;
    within("the added client's origin", || async move { granted(h, d, ADDED_ORIGIN).await }).await;
    assert_eq!(window_status(&http, &door, CLIENT, CALLBACK).await, 200, "the client before it, still");
    assert!(door.child.try_wait().unwrap().is_none(), "the Door is the one that started");
    let log = std::fs::read_to_string(&door.log).unwrap_or_default();
    assert_eq!(log.matches(" on 127.0.0.1:").count(), 1, "one start: {log}");
    let _ = std::fs::remove_dir_all(&root);
}

/// R3.1 (B): a DOOR_CLIENTS that does not parse, or fails a rule the start holds it to, is
/// refused by name in the Door's log, and the clients before it keep working. No panic.
#[tokio::test(flavor = "multi_thread")]
async fn r31b_a_door_clients_that_fails_is_refused_by_name_and_the_clients_before_it_kept() {
    let root = std::env::temp_dir().join(format!("door-security-reload-bad-{}", unique()));
    std::fs::create_dir_all(&root).unwrap();
    let clients = root.join("clients.json");
    std::fs::write(&clients, registration(false).to_string()).unwrap();
    let door = Door::start_with(&[("DOOR_CLIENTS", clients.to_str().unwrap())]);
    let http = door.http();
    let (h, d) = (&http, &door);

    let mut account = registration(true);
    account[ADDED]["scope"] = serde_json::json!("account");
    let mut home = registration(true);
    home[ADDED]["home"] = serde_json::json!("https://evil.example/community");
    let cases = [
        ("not JSON", "{ \"dt-client\": ".to_string(), clients.to_string_lossy().to_string()),
        ("the whole account's scope", account.to_string(), format!("{ADDED:?}")),
        ("a home off its origins", home.to_string(), "https://evil.example/community".to_string()),
    ];
    for (case, text, named) in cases {
        let before = clients_refused(&door).len();
        rewrite(&clients, &text);
        within(&format!("{case}: a refusal in the log"), || async move { clients_refused(d).len() > before }).await;
        let line = clients_refused(&door)[before].clone();
        assert!(line.contains(&clients.to_string_lossy().to_string()) && line.contains(&named), "{case}: the refusal names the file and {named}: {line}");
        assert_eq!(window_status(&http, &door, CLIENT, CALLBACK).await, 200, "{case}: the client before it, still");
        assert!(granted(&http, &door, CLIENT_ORIGIN).await, "{case}: its origin, still");
        assert_eq!(window_status(&http, &door, ADDED, ADDED_CALLBACK).await, 400, "{case}: nothing of the refused file");
        assert!(!granted(&http, &door, ADDED_ORIGIN).await, "{case}: nothing of the refused file");
        let log = std::fs::read_to_string(&door.log).unwrap_or_default();
        assert!(!log.contains("panicked"), "{case}: no panic after start: {log}");
    }
    // The control: the same file, made good, is taken.
    rewrite(&clients, &registration(true).to_string());
    within("the good file", || async move { window_status(h, d, ADDED, ADDED_CALLBACK).await == 200 }).await;
    let _ = std::fs::remove_dir_all(&root);
}

/// R3.1 (B): a client removed from DOOR_CLIENTS stops being honoured: no window, no CORS, its
/// unspent code redeems nothing, and its token reaches nothing.
#[tokio::test(flavor = "multi_thread")]
async fn r31b_a_client_removed_from_door_clients_stops_being_honoured() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let clients = auth.root.join("clients-reload.json");
    std::fs::write(&clients, registration(true).to_string()).unwrap();
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap())]);
    let http = door.http();
    let (h, d) = (&http, &door);
    let who = sign_up(&http, &door).await;
    let key = Dpop::new();
    let token_url = format!("{}/v2/token", door.base);
    let (code, verifier) = a_code(&http, &door, &who).await;
    let r = exchange(&http, &door, Some(key.proof("POST", &token_url, None)), &code, &verifier, CLIENT).await;
    assert_eq!(r.status().as_u16(), 200, "the exchange");
    let token = r.json::<serde_json::Value>().await.unwrap()["access_token"].as_str().unwrap().to_string();
    let me = format!("{}/v2/me", door.base);
    let read = || {
        let (http, me, token) = (http.clone(), me.clone(), token.clone());
        let proof = key.proof("GET", &me, Some(&token));
        async move { http.get(&me).header("Authorization", format!("DPoP {token}")).header("DPoP", proof).send().await.unwrap().status().as_u16() }
    };
    assert_eq!(read().await, 200, "the control: the token reads");
    let (unspent, unspent_verifier) = a_code(&http, &door, &who).await;

    let mut without = registration(true);
    without.as_object_mut().unwrap().remove(CLIENT);
    rewrite(&clients, &without.to_string());
    within("the removed client's window refused", || async move { window_status(h, d, CLIENT, CALLBACK).await == 400 }).await;
    assert!(!granted(&http, &door, CLIENT_ORIGIN).await, "the removed client's origin");
    assert_eq!(window_status(&http, &door, ADDED, ADDED_CALLBACK).await, 200, "the client that stays");
    assert!(granted(&http, &door, ADDED_ORIGIN).await, "the client that stays");
    let r = exchange(&http, &door, Some(key.proof("POST", &token_url, None)), &unspent, &unspent_verifier, CLIENT).await;
    assert!(!r.status().is_success(), "a code of the removed client's: {}", r.status());
    assert_eq!(read().await, 401, "the removed client's token");
}

// ─── R3.1 (C): the hackathon dev client, `"site": "*"` (pdr/team-registration.md) ──────────────

const WILD: &str = "hackathon-local";
const WILD_CALLBACK: &str = "http://localhost:3000/callback";
const WILD_ORIGIN: &str = "http://localhost:3000";
const NO_SITE: &str = "you are the owner or an admin of no Site";
const NOT_THEIRS: &str = "you are neither the owner nor an admin of that Site";

/// A client with no fixed Site, as the stand-in registers `hackathon-local`, with `field` set.
fn wild(field: &str, value: serde_json::Value) -> serde_json::Value {
    let mut c = serde_json::json!({ "name": "Hackathon (local)", "callbacks": [WILD_CALLBACK], "origins": [WILD_ORIGIN], "site": "*" });
    if !field.is_empty() {
        c[field] = value;
    }
    serde_json::json!({ WILD: c })
}

/// A client that does not follow redirects, trusting the edge where there is one.
fn staying(door: &Door) -> reqwest::Client {
    let b = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
    match &door.edge {
        None => b,
        Some(e) => b.tls_built_in_root_certs(false).add_root_certificate(reqwest::Certificate::from_pem(&std::fs::read(&e.ca).unwrap()).unwrap()),
    }
    .build()
    .unwrap()
}

/// R3.1 (C): a client registered with `"site": "*"` has no fixed Site, and is for local work
/// only: every callback and origin is http://localhost:<port> or http://127.0.0.1:<port>, exactly
/// as written, and it has no home and no slug. Anything else stops the start, naming the client
/// and the value, and a reload to it is refused, the clients before it kept.
#[tokio::test(flavor = "multi_thread")]
async fn r31c_a_wild_client_off_loopback_is_refused_at_load_and_at_reload() {
    let dir = std::env::temp_dir().join(format!("door-security-wild-{}", unique()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("clients.json");
    let cbs = |cb: &str| serde_json::json!([WILD_CALLBACK, cb]);
    let bad = [
        ("an https callback", wild("callbacks", cbs("https://localhost:3001/callback")), "https://localhost:3001/callback"),
        ("a callback on another host", wild("callbacks", cbs("http://evil.example.test:3001/callback")), "http://evil.example.test:3001/callback"),
        ("a callback with no port", wild("callbacks", cbs("http://localhost/callback")), "http://localhost/callback"),
        ("a callback not as written", wild("callbacks", cbs("http://LOCALHOST:3001/callback")), "http://LOCALHOST:3001/callback"),
        ("an origin off loopback", wild("origins", serde_json::json!([WILD_ORIGIN, "https://app.example.test"])), "https://app.example.test"),
        ("an origin with a path", wild("origins", serde_json::json!(["http://127.0.0.1:3001/"])), "http://127.0.0.1:3001/"),
        ("a home", wild("home", serde_json::json!("http://localhost:3000/community")), "http://localhost:3000/community"),
        ("a slug", wild("slug", serde_json::json!("egregore")), "egregore"),
    ];
    for (case, reg, named) in &bad {
        std::fs::write(&file, reg.to_string()).unwrap();
        let (status, err) = refused_at_start(&[("DOOR_CLIENTS", file.to_str().unwrap())]);
        assert_ne!(status, Some(0), "{case}: {err}");
        assert!(err.contains(&format!("{WILD:?}")) && err.contains(named), "{case}: the client and {named} are named: {err}");
    }
    // The control: as the stand-in registers it, it starts, and its window opens. Then a reload to
    // one off loopback is refused, named, and the one before it kept.
    std::fs::write(&file, wild("", serde_json::Value::Null).to_string()).unwrap();
    let door = Door::start_with(&[("DOOR_CLIENTS", file.to_str().unwrap())]);
    let http = door.http();
    let (h, d) = (&http, &door);
    assert_eq!(window_status(&http, &door, WILD, WILD_CALLBACK).await, 200, "the wild client's window");
    assert!(granted(&http, &door, WILD_ORIGIN).await, "its origin");
    rewrite(&file, &bad[1].1.to_string());
    within("a wild client off loopback, refused at a reload", || async move { clients_refused(d).iter().any(|l| l.contains(&format!("{WILD:?}"))) }).await;
    assert_eq!(window_status(h, d, WILD, WILD_CALLBACK).await, 200, "the one before it, kept");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The window's passkey leg for the wild client, as the window does it: its answer, and the
/// cookie it sets.
async fn wild_sign_in(http: &reqwest::Client, door: &Door, who: &Account, challenge: &str) -> (serde_json::Value, Option<String>) {
    let (a, k) = attempt(http, door).await;
    let r = finish(http, door, serde_json::json!({
        "attempt": a, "handle": who.handle, "sealed": seal(&k, &who.prf, &a),
        "client": { "client": WILD, "redirect_uri": WILD_CALLBACK, "code_challenge": challenge, "state": "st-wild" },
    }))
    .await;
    let cookie = cookie_of(&r);
    let (s, body) = said(r).await;
    assert_eq!(s, 200, "the window's sign-in: {body}");
    (serde_json::from_str(&body).unwrap(), cookie)
}

/// After the passkey, the wild client's window goes to the Door's Site step, never to the
/// callback: no code has been made. Answers the step's cookie.
async fn to_the_site_step(http: &reqwest::Client, door: &Door, who: &Account, challenge: &str) -> String {
    let (body, cookie) = wild_sign_in(http, door, who, challenge).await;
    assert_eq!(body["redirect"], "/signin/site", "the window goes to the Site step: {body}");
    assert!(!body.to_string().contains("code="), "no code before a Site is chosen: {body}");
    cookie.expect("the Site step's cookie")
}

/// The Site step: its status, its page and the Sites it offers, in order.
async fn site_step(http: &reqwest::Client, door: &Door, cookie: &str) -> (u16, String, Vec<String>) {
    let r = http.get(format!("{}/signin/site", door.base)).header("cookie", cookie).send().await.unwrap();
    let (s, page) = said(r).await;
    let offered = page.split("<option value=\"").skip(1).filter_map(|o| o.split('"').next()).map(str::to_string).collect();
    (s, page, offered)
}

/// The choice, as the step's form posts it: the 303, its Location, or the refusal.
async fn choose(door: &Door, cookie: &str, site: &str) -> (u16, String, String) {
    let r = staying(door)
        .post(format!("{}/signin/site", door.base))
        .header("cookie", cookie)
        .header("origin", door.base.as_str())
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!("site={site}"))
        .send()
        .await
        .unwrap();
    let at = r.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let (s, body) = said(r).await;
    (s, at, body)
}

/// The wild client's code for its token, as its page trades it.
async fn wild_exchange(http: &reqwest::Client, door: &Door, key: &Dpop, code: &str, verifier: &str) -> reqwest::Response {
    let url = format!("{}/v2/token", door.base);
    http.post(&url)
        .header("DPoP", key.proof("POST", &url, None))
        .json(&serde_json::json!({ "code": code, "code_verifier": verifier, "client": WILD, "redirect_uri": WILD_CALLBACK }))
        .send()
        .await
        .unwrap()
}

/// Two people on one Door with the wild client: bo owns a Site and a room; ada owns three
/// Sites, bo a plain member of one, an admin of another, and not on the third.
struct Wild {
    door: Door,
    ada: Account,
    bo: Account,
    bo_own: String,
    bo_room: String,
    member: String,
    admin: String,
    alone: String,
    _auth: Auth,
    _relay: Relay,
    dir: PathBuf,
}

impl Drop for Wild {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn wild_fixture() -> Wild {
    let dir = std::env::temp_dir().join(format!("door-security-wild-{}", unique()));
    let (keys, claim) = kiosk_claim(&dir);
    let (arc, got) = arc_keeping_bodies().await;
    let (auth, relay) = (Auth::start(), Relay::start());
    let clients = dir.join("clients.json");
    std::fs::write(&clients, wild("", serde_json::Value::Null).to_string()).unwrap();
    let door = Door::start_with(&[
        ("DOOR_AUTH", auth.url.as_str()),
        ("DOOR_RELAY", relay.url.as_str()),
        ("DOOR_CLAIM_KEYS", keys.to_str().unwrap()),
        ("DOOR_ARC", arc.as_str()),
        ("DOOR_CLIENTS", clients.to_str().unwrap()),
    ]);
    let http = door.http();
    let (ada, bo) = (sign_up_as(&http, &door, "ada").await, sign_up_as(&http, &door, "bo").await);
    // bo's contact bundles, as bo's session makes them for a join.
    let r = staying(&door).get(format!("{}/join?claim={claim}", door.base)).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 303, "the claim lands");
    let join = cookie_of(&r).expect("the join cookie");
    let r = http.post(format!("{}/v2/join", door.base)).header("cookie", format!("{}; {join}", bo.cookie)).header("origin", LISTED).send().await.unwrap();
    let (s, body) = said(r).await;
    assert_eq!(s, 200, "{body}");
    let bundles: Vec<String> = got.lock().unwrap()[0]["bundles"].as_array().unwrap().iter().map(|b| b.as_str().unwrap().to_string()).collect();
    let bo_own = mint_as(&http, &door, &bo.cookie, "group", "Bo's own").await;
    let bo_room = mint_as(&http, &door, &bo.cookie, "forum", "Bo's room").await;
    let (member, admin, alone) = (
        mint_as(&http, &door, &ada.cookie, "group", "Ada's, bo a member").await,
        mint_as(&http, &door, &ada.cookie, "group", "Ada's, bo an admin").await,
        mint_as(&http, &door, &ada.cookie, "group", "Ada's alone").await,
    );
    for (site, bundle) in [(&member, &bundles[0]), (&admin, &bundles[1])] {
        let r = http.post(format!("{}/v2/add", door.base)).header("cookie", &ada.cookie).header("origin", LISTED).json(&serde_json::json!({ "object": site, "bundle": bundle })).send().await.unwrap();
        let (s, added) = said(r).await;
        assert_eq!(s, 200, "bo is added to {site}: {added}");
    }
    let bo_pk = bo.pk.trim_start_matches("ed25519:").to_ascii_lowercase();
    apply_as(&http, &door, &ada.cookie, &admin, "base.setRole", serde_json::json!({ "member": bo_pk, "role": "admin" })).await;
    // bo's Node holds both of ada's Sites, and the admin's role.
    let by = Instant::now() + Duration::from_secs(60);
    loop {
        let g: serde_json::Value = http.get(format!("{}/v2/graph", door.base)).header("cookie", &bo.cookie).send().await.unwrap().json().await.unwrap();
        let o = |id: &str| g["objects"].as_array().into_iter().flatten().find(|o| o["id"] == id).cloned();
        let roles = o(&admin).map(|o| o["view"]["roles"].clone()).unwrap_or_default();
        if o(&member).is_some() && roles.as_array().into_iter().flatten().any(|r| r[0] == bo_pk.as_str() && r[1] == "admin") {
            break;
        }
        assert!(Instant::now() < by, "bo's Node holds ada's Sites and the admin's role: {g}");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Wild { door, ada, bo, bo_own, bo_room, member, admin, alone, _auth: auth, _relay: relay, dir }
}

/// R3.1 (C): after the passkey, the wild client's window goes to the Door's own Site step, a page
/// that offers exactly the person's Sites where they are the owner or an admin, as one select: no
/// plain member's Site, none they are not on, no room. Labels and values only: no copy.
#[tokio::test(flavor = "multi_thread")]
async fn r31c_consent_for_a_wild_client_offers_exactly_the_persons_owner_and_admin_sites() {
    let w = wild_fixture().await;
    let http = w.door.http();
    let (_, challenge) = pkce();
    let cookie = to_the_site_step(&http, &w.door, &w.bo, &challenge).await;
    let r = http.get(format!("{}/signin/site", w.door.base)).header("cookie", &cookie).send().await.unwrap();
    let header = |n: &str| r.headers().get(n).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let (csp, cache) = (header("content-security-policy"), header("cache-control"));
    assert!(cache.contains("no-store"), "{cache}");
    for d in ["default-src 'none'", "frame-ancestors 'none'", &format!("form-action 'self' {WILD_ORIGIN}")] {
        assert!(csp.split(';').any(|x| x.trim() == d), "CSP {d}: {csp}");
    }
    assert!(!csp.contains("script-src"), "the step runs no script: {csp}");
    let (s, page, mut offered) = site_step(&http, &w.door, &cookie).await;
    assert_eq!(s, 200, "{page}");
    offered.sort();
    let mut want = vec![w.bo_own.clone(), w.admin.clone()];
    want.sort();
    assert_eq!(offered, want, "the owner's and the admin's Sites, and no other: {page}");
    for not in [&w.member, &w.alone, &w.bo_room] {
        assert!(!page.contains(not.as_str()), "{not} is not offered: {page}");
    }
    assert_eq!(page.matches("<select").count(), 1, "one select: {page}");
    // Labels and values only: what a person reads is the app's name, the label, the Sites, the buttons.
    let mut text = String::new();
    let mut tag = false;
    for c in page.split("<body>").nth(1).unwrap_or(&page).chars() {
        match c {
            '<' => tag = true,
            '>' => {
                tag = false;
                text.push(' ');
            }
            c if !tag => text.push(c),
            _ => {}
        }
    }
    let text = text.replace("&#39;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&");
    let words: Vec<&str> = text.split_whitespace().collect();
    let allowed = "Hackathon (local) Site Bo's own Ada's, bo an admin Allow Cancel";
    for w in &words {
        assert!(allowed.split_whitespace().any(|a| a == *w), "{w:?} is copy, not a label or a value: {words:?}");
    }
    let _ = &w.ada;
}

/// R3.1 (C): the choice is held server-side. A Site where the person is a plain member, not on
/// the roster, or not a Site at all, is refused in the Door's words, and no code is made. And a
/// Site whose admin is no longer one by the time its code is traded is refused at /v2/token.
#[tokio::test(flavor = "multi_thread")]
async fn r31c_a_site_where_the_person_is_a_plain_member_is_refused_at_issue() {
    let w = wild_fixture().await;
    let http = w.door.http();
    for site in [&w.member, &w.alone, &w.bo_room] {
        let (_, challenge) = pkce();
        let cookie = to_the_site_step(&http, &w.door, &w.bo, &challenge).await;
        let (s, at, body) = choose(&w.door, &cookie, site).await;
        assert_eq!((s, body.as_str()), (403, NOT_THEIRS), "{site}: refused in the Door's words");
        assert!(!at.contains("code="), "{site}: no code: {at}");
    }
    // The admin's Site: a code, at the registered callback.
    let (verifier, challenge) = pkce();
    let cookie = to_the_site_step(&http, &w.door, &w.bo, &challenge).await;
    let (s, at, body) = choose(&w.door, &cookie, &w.admin).await;
    assert_eq!(s, 303, "{body}");
    assert!(at.starts_with(&format!("{WILD_CALLBACK}?code=")) && at.ends_with("&state=st-wild"), "the registered callback, the state unchanged: {at}");
    let code = code_of(&at);
    // ada takes the role away before bo's page trades the code: refused at /v2/token.
    let bo_pk = w.bo.pk.trim_start_matches("ed25519:").to_ascii_lowercase();
    apply_as(&http, &w.door, &w.ada.cookie, &w.admin, "base.clearRole", serde_json::json!({ "member": bo_pk })).await;
    let by = Instant::now() + Duration::from_secs(40);
    loop {
        let g: serde_json::Value = http.get(format!("{}/v2/graph", w.door.base)).header("cookie", &w.bo.cookie).send().await.unwrap().json().await.unwrap();
        let roles = g["objects"].as_array().into_iter().flatten().find(|o| o["id"] == w.admin.as_str()).map(|o| o["view"]["roles"].clone()).unwrap_or_default();
        if !roles.as_array().into_iter().flatten().any(|r| r[0] == bo_pk.as_str()) {
            break;
        }
        assert!(Instant::now() < by, "bo's Node holds the role's removal: {roles}");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let r = wild_exchange(&http, &w.door, &Dpop::new(), &code, &verifier).await;
    let (s, body) = said(r).await;
    assert_eq!((s, body.as_str()), (403, NOT_THEIRS), "refused at /v2/token: {body}");
}

/// R3.1 (C): the token for the chosen Site is that Site's, exactly as a fixed client's is: its
/// scope, /v2/members, and nothing of another Site of the person's. Its origin passes CORS. A
/// Site's name reaches the step escaped.
#[tokio::test(flavor = "multi_thread")]
async fn r31c_the_wild_token_is_scoped_to_the_chosen_site() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let clients = auth.root.join("clients-wild.json");
    std::fs::write(&clients, wild("", serde_json::Value::Null).to_string()).unwrap();
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let hostile = "Mill \"Road\" <i>x</i>";
    let chosen = mint_as(&http, &door, &who.cookie, "group", hostile).await;
    let other = mint_as(&http, &door, &who.cookie, "group", "Another").await;
    assert!(granted(&http, &door, WILD_ORIGIN).await, "the wild client's origin");

    let (verifier, challenge) = pkce();
    let cookie = to_the_site_step(&http, &door, &who, &challenge).await;
    let (s, page, offered) = site_step(&http, &door, &cookie).await;
    assert_eq!(s, 200, "{page}");
    assert_eq!(offered.len(), 2, "{page}");
    assert!(page.contains("Mill &quot;Road&quot; &lt;i&gt;x&lt;/i&gt;") && !page.contains("<i>x</i>"), "the name, escaped: {page}");
    let (s, at, body) = choose(&door, &cookie, &chosen).await;
    assert_eq!(s, 303, "{body}");
    let key = Dpop::new();
    let r = wild_exchange(&http, &door, &key, &code_of(&at), &verifier).await;
    let (s, body) = said(r).await;
    assert_eq!(s, 200, "the exchange: {body}");
    let t: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(t["scope"], chosen.as_str(), "the token is the chosen Site's");
    let token = t["access_token"].as_str().unwrap().to_string();
    let get = |path: String| {
        let (http, url, token) = (http.clone(), format!("{}{path}", door.base), token.clone());
        let proof = key.proof("GET", &url, Some(&token));
        async move { said(http.get(&url).header("Authorization", format!("DPoP {token}")).header("DPoP", proof).send().await.unwrap()).await }
    };
    for path in ["/v2/members".to_string(), format!("/v2/members/{chosen}")] {
        let (s, body) = get(path.clone()).await;
        assert_eq!(s, 200, "{path}: {body}");
        assert_eq!(serde_json::from_str::<serde_json::Value>(&body).unwrap()["site"], chosen.as_str(), "{path}");
    }
    let (s, body) = get(format!("/v2/members/{other}")).await;
    assert_eq!((s, body.as_str()), (404, "no such Site in reach"), "another Site of the person's is out of reach");
    let (s, body) = get("/v2/graph".into()).await;
    assert_eq!(s, 200, "{body}");
    assert!(body.contains(chosen.as_str()) && !body.contains(other.as_str()), "the graph is the chosen Site's: {body}");
}

/// R3.1 (C): the starter (docs' quickstart) registers its own page as its callback,
/// `location.origin + location.pathname`, so served at the root it is "http://localhost:<port>/".
/// Through the SHIPPED registration, hackathon-local, that callback signs in and goes to the Site
/// step, on both loopback names and on every port the quickstart may use; and its origin is
/// granted. Every one of these failed at sign-in when only /callback was registered.
#[tokio::test(flavor = "multi_thread")]
async fn r31c_the_starters_own_page_signs_in_through_the_shipped_hackathon_client() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let shipped = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("clients.stand-in.json");
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", shipped.to_str().unwrap())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    for port in [3000, 3009, 5173, 5174, 5175, 8000, 8080] {
        for host in ["localhost", "127.0.0.1"] {
            let callback = format!("http://{host}:{port}/");
            let (_, challenge) = pkce();
            let (a, k) = attempt(&http, &door).await;
            let r = finish(&http, &door, serde_json::json!({
                "attempt": a, "handle": who.handle, "sealed": seal(&k, &who.prf, &a),
                "client": { "client": WILD, "redirect_uri": callback, "code_challenge": challenge, "state": "st-starter" },
            }))
            .await;
            let (s, body) = said(r).await;
            assert_eq!(s, 200, "{callback}: the window's sign-in: {body}");
            assert!(body.contains("/signin/site"), "{callback}: to the Site step: {body}");
            let origin = format!("http://{host}:{port}");
            assert!(granted(&http, &door, &origin).await, "{origin} is granted");
        }
    }
}

/// R3.1 (C), TEST in Chrome (a9f03117): Allow at the Site step answered 403 "a write from an
/// origin this Door does not list", 3 of 3. A browser posting a form from a page served with
/// Referrer-Policy: no-referrer sends `Origin: null`, and the choice requires the Door's own
/// origin. So the step's page says `same-origin`, and the browser sends the Door's origin on its
/// own form; the check itself stays as strict: `null` is still refused, and nothing else is let in.
#[tokio::test(flavor = "multi_thread")]
async fn r31c_the_site_steps_form_posts_with_the_doors_own_origin() {
    let w = wild_fixture().await;
    let http = w.door.http();
    let (_, challenge) = pkce();
    let cookie = to_the_site_step(&http, &w.door, &w.bo, &challenge).await;
    let r = http.get(format!("{}/signin/site", w.door.base)).header("cookie", &cookie).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(
        r.headers().get("referrer-policy").and_then(|v| v.to_str().ok()),
        Some("same-origin"),
        "the step's page lets its own form carry the Door's origin"
    );
    let post = |origin: &'static str| {
        staying(&w.door)
            .post(format!("{}/signin/site", w.door.base))
            .header("cookie", &cookie)
            .header("origin", origin)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(format!("site={}", w.bo_own))
            .send()
    };
    let (s, body) = said(post("null").await.unwrap()).await;
    assert_eq!((s, body.as_str()), (403, "a write from an origin this Door does not list"), "Origin: null, as no-referrer made it, is refused still");
    let (s, at, body) = choose(&w.door, &cookie, &w.bo_own).await;
    assert_eq!(s, 303, "the Door's own origin, as same-origin sends it: {body}");
    assert!(at.starts_with(WILD_CALLBACK), "to the callback: {at}");
}

/// R3.1 (C), DEVEX's HIGH: a newcomer from the starter has no Site, and the refusal was a dead
/// end. The no-Site answer is still the Door's refusal (403, its words, the flow stopped), and
/// it carries the one next step: a link to https://wallflowers.io/signup, the address its own
/// text, where an account makes a community; and Cancel back to the client's callback with
/// access_denied, as the step's Cancel answers.
#[tokio::test(flavor = "multi_thread")]
async fn r31c_the_no_site_answer_links_to_making_a_community_and_cancels_to_the_callback() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let clients = auth.root.join("clients-wild.json");
    std::fs::write(&clients, wild("", serde_json::Value::Null).to_string()).unwrap();
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let (_, challenge) = pkce();
    let cookie = to_the_site_step(&http, &door, &who, &challenge).await;
    let r = http.get(format!("{}/signin/site", door.base)).header("cookie", &cookie).send().await.unwrap();
    let ctype = r.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let (s, page) = said(r).await;
    assert_eq!(s, 403, "the Door's refusal: {page}");
    assert!(ctype.starts_with("text/html"), "a page a person reads: {ctype}");
    assert!(page.contains(NO_SITE), "its words: {page}");
    assert!(page.contains(r#"<a href="https://wallflowers.io/signup">https://wallflowers.io/signup</a>"#), "the one next step, the address its own text: {page}");
    let cancel = format!("{WILD_CALLBACK}?error=access_denied&amp;state=st-wild");
    assert!(page.contains(&format!(r#"href="{cancel}""#)), "Cancel, to the callback: {page}");
    assert_eq!(page.matches("<a ").count(), 2, "two links and nothing else to follow: {page}");
    assert!(!page.contains("<form"), "nothing to choose: {page}");
    let site = mint_as(&http, &door, &who.cookie, "group", "Made too late").await;
    let (s, at, body) = choose(&door, &cookie, &site).await;
    assert!(s >= 400 && !at.contains("code="), "the flow stopped: {s} {at} {body}");
}

/// R3.1 (C): a person with no Site where they are the owner or an admin gets the Door's refusal
/// in words at the Site step, and the flow stops: nothing to choose after. A newcomer signing up
/// through the wild client meets the same step, after their words.
#[tokio::test(flavor = "multi_thread")]
async fn r31c_a_person_with_no_eligible_site_gets_the_doors_refusal_and_the_flow_stops() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let clients = auth.root.join("clients-wild.json");
    std::fs::write(&clients, wild("", serde_json::Value::Null).to_string()).unwrap();
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CLIENTS", clients.to_str().unwrap())]);
    let http = door.http();
    let who = sign_up(&http, &door).await;
    let (_, challenge) = pkce();
    let cookie = to_the_site_step(&http, &door, &who, &challenge).await;
    let (s, page, offered) = site_step(&http, &door, &cookie).await;
    assert_eq!(s, 403, "the Door's refusal: {page}");
    assert!(page.contains(NO_SITE), "in words: {page}");
    assert!(offered.is_empty());
    let site = mint_as(&http, &door, &who.cookie, "group", "Made too late").await;
    let (s, at, body) = choose(&door, &cookie, &site).await;
    assert!(s >= 400 && !at.contains("code=") && !body.is_empty(), "the flow stopped: {s} {at} {body}");

    // A newcomer, through the wild client: the words, then the same step, and the same refusal.
    let r = http.post(format!("{}/v2/signup", door.base)).json(&serde_json::json!({ "name": "newcomer", "work": work(&http, &door, "signup").await })).send().await.unwrap();
    let o: serde_json::Value = r.json().await.unwrap();
    let a = o["attempt"].as_str().unwrap().to_string();
    let r = http
        .post(format!("{}/v2/signup/finish", door.base))
        .json(&serde_json::json!({ "attempt": a, "sealed": seal(o["key"].as_str().unwrap(), &random32(), &a),
            "client": { "client": WILD, "redirect_uri": WILD_CALLBACK, "code_challenge": challenge, "state": "st-wild" } }))
        .send()
        .await
        .unwrap();
    let (s, body) = said(r).await;
    assert_eq!(s, 200, "{body}");
    let done: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(done["words"].is_string() && done["continue"].is_string(), "{done}");
    let r = http.post(format!("{}/v2/signup/continue", door.base)).json(&serde_json::json!({ "continue": done["continue"] })).send().await.unwrap();
    let cookie = cookie_of(&r);
    let (s, body) = said(r).await;
    assert_eq!(s, 200, "{body}");
    assert_eq!(serde_json::from_str::<serde_json::Value>(&body).unwrap()["redirect"], "/signin/site", "{body}");
    let (s, page, _) = site_step(&http, &door, &cookie.expect("the Site step's cookie")).await;
    assert_eq!(s, 403, "{page}");
    assert!(page.contains(NO_SITE), "{page}");
}

// ─── W-96: the contact code ──────────────────────────────────────────────────

/// W-96, THE CONTACT CODE: a person asks the Door for their code, six single-use key
/// packages under their one identity, kept before the answer, and signs out. Their Site's
/// owner adds them to the Site and to its room, one package each (a room by member id wants a
/// contact, which this Door forms none of). Signed in again, they hold both. A site's token,
/// or a call without the Door's origin, gets no code.
#[tokio::test(flavor = "multi_thread")]
async fn w96_a_contact_code_adds_its_person_to_a_site_and_its_room_after_they_sign_out() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let (ada, bo) = (sign_up(&http, &door).await, sign_up(&http, &door).await);
    let url = format!("{}/v2/bundle", door.base);
    let r = http.post(&url).header("cookie", &bo.cookie).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 403, "no origin, no code");
    let (s, why) = said(http.post(&url).header("authorization", "DPoP a-token").header("origin", LISTED).send().await.unwrap()).await;
    assert!(s == 403 && why.contains("a contact code is the webapp's"), "a site's token, refused by name: {s} {why}");
    let (s, body) = said(http.post(&url).header("cookie", &bo.cookie).header("origin", LISTED).send().await.unwrap()).await;
    assert_eq!(s, 200, "{body}");
    let code: Vec<String> = serde_json::from_str::<serde_json::Value>(&body).unwrap()["bundles"]
        .as_array()
        .map(|a| a.iter().filter_map(|b| b.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    assert_eq!(code.len(), 6, "the Site's package and five rooms': {body}");
    assert_eq!(code.iter().collect::<std::collections::HashSet<_>>().len(), 6, "each package its own");
    assert_eq!(sign_out(&http, &door, &bo.cookie).await, 200, "the code outlives its session");

    let (s, out) = batch_as(&http, &door, &ada.cookie, serde_json::json!([
        { "do": "mint", "kind": "group", "draft": { "name": "Mill Road", "shape": "community" } },
        { "do": "mint", "kind": "forum", "draft": { "name": "Hall" } },
        { "do": "apply", "object": { "$step": 0 }, "op": "base.setPart", "args": { "part": { "$step": 1 }, "role": "room", "at": 1 } },
        { "do": "apply", "object": { "$step": 1 }, "op": "base.setParent", "args": { "parent": { "$step": 0 }, "role": "room", "at": 1 } },
    ])).await;
    assert_eq!(s, 200, "{out}");
    let (site, room) = (out["made"][0].as_str().unwrap().to_string(), out["made"][1].as_str().unwrap().to_string());
    let bo_pk = bo.pk.trim_start_matches("ed25519:").to_lowercase();
    for (object, bundle) in [(&site, &code[0]), (&room, &code[1])] {
        let r = http.post(format!("{}/v2/add", door.base)).header("cookie", &ada.cookie).header("origin", LISTED)
            .json(&serde_json::json!({ "object": object, "bundle": bundle })).send().await.unwrap();
        let (s, body) = said(r).await;
        assert_eq!(s, 200, "the owner adds by the code: {body}");
        assert_eq!(serde_json::from_str::<serde_json::Value>(&body).unwrap()["member"].as_str(), Some(bo_pk.as_str()), "the code's person");
    }

    let cookie = sign_in(&http, &door, &bo).await;
    let deadline = Instant::now() + Duration::from_secs(40);
    let held = loop {
        let g: serde_json::Value = http.get(format!("{}/v2/graph", door.base)).header("cookie", &cookie).send().await.unwrap().json().await.unwrap();
        let ids: Vec<String> = g["objects"].as_array().into_iter().flatten().filter_map(|o| o["id"].as_str().map(str::to_string)).collect();
        if (ids.contains(&site) && ids.contains(&room)) || Instant::now() > deadline {
            break ids;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    assert!(held.contains(&site) && held.contains(&room), "signed in again, the code's person holds the Site and its room: {held:?}");
}

/// W-96: A CONTACT CODE IS KEPT BEFORE ITS ANSWER. Its person's process is SIGKILLed the
/// moment the code answers, so no end of session seals it; the owner adds them by it later,
/// and their next sign-in on this Door still opens the Site: the packages' private halves
/// were on this disk before the code was handed out.
#[tokio::test(flavor = "multi_thread")]
async fn w96_a_contact_code_survives_its_process_killed_straight_after_its_answer() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str())]);
    let http = door.http();
    let bo = sign_up(&http, &door).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let pid = persons_process(&door);
    assert!(!pid.is_empty(), "the person's process");
    let (s, body) = said(http.post(format!("{}/v2/bundle", door.base)).header("cookie", &bo.cookie).header("origin", LISTED).send().await.unwrap()).await;
    assert_eq!(s, 200, "{body}");
    let code: Vec<String> = serde_json::from_str::<serde_json::Value>(&body).unwrap()["bundles"]
        .as_array()
        .map(|a| a.iter().filter_map(|b| b.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let _ = Command::new("kill").args(["-KILL", &pid]).status();
    let mut gone = false;
    for _ in 0..100 {
        let st = Command::new("ps").args(["-o", "stat=", "-p", &pid]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        if st.is_empty() || st.starts_with('Z') {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(gone, "the person's process was killed");

    let ada = sign_up(&http, &door).await;
    let (s, out) = batch_as(&http, &door, &ada.cookie, serde_json::json!([
        { "do": "mint", "kind": "group", "draft": { "name": "Mill Road", "shape": "community" } },
    ])).await;
    assert_eq!(s, 200, "{out}");
    let site = out["made"][0].as_str().unwrap().to_string();
    let r = http.post(format!("{}/v2/add", door.base)).header("cookie", &ada.cookie).header("origin", LISTED)
        .json(&serde_json::json!({ "object": site, "bundle": code[0] })).send().await.unwrap();
    let (s, why) = said(r).await;
    assert_eq!(s, 200, "{why}");

    let cookie = sign_in(&http, &door, &bo).await;
    let deadline = Instant::now() + Duration::from_secs(40);
    let held = loop {
        let g = http.get(format!("{}/v2/graph", door.base)).header("cookie", &cookie).send().await.unwrap().text().await.unwrap();
        if g.contains(&site) || Instant::now() > deadline {
            break g.contains(&site);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    assert!(held, "the code's packages were kept before it was handed out: the next sign-in opens the Site");
}

/// W-96: A CONTACT CODE LIVES DOOR_CODE_SECS. Its packages carry that lifetime (MLS's own,
/// which the adder checks), so a code past it is refused at /v2/add, 410 "the contact code has
/// expired" (the kind, core's KeyPackageExpired, never MLS's text), and adds no one: a code handed to the wrong person, or forgotten, is dead within the half hour.
#[tokio::test(flavor = "multi_thread")]
async fn w96_a_contact_code_past_its_life_is_refused_and_adds_no_one() {
    let (auth, relay) = (Auth::start(), Relay::start());
    let door = Door::start_with(&[("DOOR_AUTH", auth.url.as_str()), ("DOOR_RELAY", relay.url.as_str()), ("DOOR_CODE_SECS", "2")]);
    let http = door.http();
    let (ada, bo) = (sign_up(&http, &door).await, sign_up(&http, &door).await);
    let (s, body) = said(http.post(format!("{}/v2/bundle", door.base)).header("cookie", &bo.cookie).header("origin", LISTED).send().await.unwrap()).await;
    assert_eq!(s, 200, "{body}");
    let code: Vec<String> = serde_json::from_str::<serde_json::Value>(&body).unwrap()["bundles"]
        .as_array()
        .map(|a| a.iter().filter_map(|b| b.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let (s, out) = batch_as(&http, &door, &ada.cookie, serde_json::json!([
        { "do": "mint", "kind": "group", "draft": { "name": "Mill Road", "shape": "community" } },
    ])).await;
    assert_eq!(s, 200, "{out}");
    let site = out["made"][0].as_str().unwrap().to_string();
    tokio::time::sleep(Duration::from_secs(4)).await;
    let r = http.post(format!("{}/v2/add", door.base)).header("cookie", &ada.cookie).header("origin", LISTED)
        .json(&serde_json::json!({ "object": site, "bundle": code[0] })).send().await.unwrap();
    let (s, why) = said(r).await;
    assert_eq!((s, why.as_str()), (410, "the contact code has expired"), "a code past its life is refused by its kind, in the Door's words: MLS's own lifetime check, not its text");
    let g: serde_json::Value = http.get(format!("{}/v2/graph", door.base)).header("cookie", &ada.cookie).send().await.unwrap().json().await.unwrap();
    let members = g["objects"].as_array().into_iter().flatten().find(|o| o["id"] == site.as_str()).map(|o| o["members"].clone()).unwrap_or_default();
    let bo_pk = bo.pk.trim_start_matches("ed25519:").to_lowercase();
    assert!(!members.to_string().contains(&bo_pk), "no one was added: {members}");
}

/// HACK_USER #18: /signin for a client, or a callback, this Door has not registered is refused as
/// a page with its one way on, the registration's docs, its address as its own text: no new
/// sentence, the no-Site answer's pattern. Never a link to the callback asked for. The window's
/// other refusals (the S256 challenge, the state) stay as they were.
#[tokio::test(flavor = "multi_thread")]
async fn hu18_an_unregistered_client_or_callback_is_refused_with_the_way_to_register() {
    let root = std::env::temp_dir().join(format!("door-security-hu18-{}", unique()));
    std::fs::create_dir_all(&root).unwrap();
    let clients = clients_file(&root);
    let door = Door::start_with(&[("DOOR_CLIENTS", clients.to_str().unwrap())]);
    let http = door.http();
    let (_, challenge) = pkce();
    let docs = "https://docs.wallflowers.io/register.html";
    let stray = "https://elsewhere.example.test/cb";
    for (client, cb, words) in [("nobody", CALLBACK, "no client"), (CLIENT, stray, "is not a callback client")] {
        let url = format!("{}/signin?client={client}&redirect_uri={}&code_challenge={challenge}&code_challenge_method=S256&state=st", door.base, b64u_free(cb));
        let r = http.get(&url).send().await.unwrap();
        let (s, ty) = (r.status().as_u16(), r.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string());
        let body = r.text().await.unwrap();
        assert_eq!(s, 400, "{client}: {body}");
        assert!(ty.starts_with("text/html"), "{client}: a page, not bare text: {ty}");
        assert!(body.contains(words), "{client}: the Door's refusal: {body}");
        assert!(body.contains(&format!(r#"<a href="{docs}">{docs}</a>"#)), "{client}: the way to register, its address its own text: {body}");
        assert!(!body.contains(&format!(r#"href="{stray}""#)), "{client}: never a link to the callback asked for: {body}");
    }
    // The window's other refusals stay as they were: bare words.
    let url = format!("{}/signin?client={CLIENT}&redirect_uri={}&code_challenge={challenge}&code_challenge_method=plain&state=st", door.base, b64u_free(CALLBACK));
    let (s, body) = said(http.get(&url).send().await.unwrap()).await;
    assert_eq!((s, body.contains("S256"), body.contains(docs)), (400, true, false), "{body}");
}

/// A URL as a query value.
fn b64u_free(u: &str) -> String {
    u.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}
