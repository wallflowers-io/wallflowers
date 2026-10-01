//! door — the supervisor (DR-2, S-12): app.wallflowers.io's one public process.
//!
//! It holds the session table, starts a session process (DR-3, `account.rs`) per
//! person, whatever sessions they hold (D-34 (c)), and proxies to it, and it never
//! holds a seed, a PRF output or a key that opens one (mdr/door.md §3, §4). The window
//! seals the PRF output to the session process; this process carries it unread.
//!
//! WHY A SUPERVISOR AND NOT A THREAD POOL. `paths::state_dir()` is read per call
//! and every `Node` handle resolves through it, so two accounts in one process
//! would share a directory. The core forces one account per process, and that is
//! the isolation the production shape wants: ending the process is what a person's
//! last sign-out means.
//!
//! A SESSION OPENS ON TWO PROOFS (SEC-4): the wrap stored under the handle's key
//! opens under the PRF output, and the restored identity then signs a single-use
//! challenge this process issued. Nothing else opens one: no bare key, no list of
//! sessions, no header naming one (CS-3, CS-4, CS-16). `/v1` is gone.
//!
//! Two modes of one binary: the supervisor (default) and `DOOR_MODE=session`.

mod account;
mod auth;
mod icd;
mod counting;
mod face;
mod members;
mod pow;
mod seal;
mod token;
mod tw;
#[cfg(test)]
mod routes;

/// What the fold cache measures its entries by (O-69, FC-10).
#[global_allocator]
static ALLOC: counting::Counting = counting::Counting;

use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tower_http::cors::CorsLayer;

/// One session process, and the socket that is its leash: its stdin, both ways. The
/// config goes to it and its port comes back; dropping this closes it, and the process
/// exits on EOF. Its directory goes with it.
struct Proc {
    child: std::process::Child,
    _leash: std::os::unix::net::UnixStream,
    port: u16,
    secret: String,
    dir: std::path::PathBuf,
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A sign-in or sign-up under way: a process with a key and no session yet.
struct Attempt {
    proc: Proc,
    started: Instant,
    signup: bool,
    /// The keyed hash of the address that started it: one address holds at most
    /// `DOOR_ATTEMPTS_PER_ADDR` open attempts (SEC-A2).
    client: String,
    /// The `/join` handle of the kiosk claim it draws on: the reserved pool (D-55).
    join: Option<String>,
}

/// ONE PROCESS PER PERSON (D-34 (c)): a person signed in, their process, and every
/// session of theirs it answers. It ends with the last of them.
struct Person {
    proc: Proc,
    sessions: HashSet<String>,
}

struct Session {
    /// The person it is one of: their process answers it.
    pk: String,
    seen: Instant,
    /// The Site a public site's session is held to; `None` for the webapp's own, and
    /// for the first-party client's, which reaches the whole account.
    scope: Option<String>,
    /// A client's session, held by its token: it ends when the token does.
    token: bool,
}

/// Who is signed in, and their sessions, under one lock: a session and its person come
/// and go together.
#[derive(Default)]
struct Live {
    persons: HashMap<String, Person>,
    sessions: HashMap<String, Session>,
}

/// What ending a session leaves to do: its person's process forgets it, or, the last of
/// theirs, the process ends.
enum Ended {
    Forget { port: u16, secret: String },
    Last(Proc),
}

impl Live {
    /// A person whose process has exited, killed or crashed, is not live: they and every
    /// session of theirs go, so the next sign-in opens their process again, from its
    /// seal, instead of seating a session on a process that answers nothing (O-74). The
    /// process comes back to be dropped outside the lock (Software Security, (b)): its
    /// wait and its directory's removal must not stall every other request.
    fn reap(&mut self, pk: &str) -> Option<Proc> {
        if !self.persons.get_mut(pk).is_some_and(|p| matches!(p.proc.child.try_wait(), Ok(Some(_)))) {
            return None;
        }
        self.sessions.retain(|_, s| s.pk != pk);
        self.persons.remove(pk).map(|p| p.proc)
    }

    /// End session `id`: its person, and what is left to do.
    fn end(&mut self, id: &str) -> Option<(String, Ended)> {
        let s = self.sessions.remove(id)?;
        let p = self.persons.get_mut(&s.pk)?;
        p.sessions.remove(id);
        if !p.sessions.is_empty() {
            return Some((s.pk, Ended::Forget { port: p.proc.port, secret: p.proc.secret.clone() }));
        }
        let p = self.persons.remove(&s.pk)?;
        Some((s.pk, Ended::Last(p.proc)))
    }
}

/// The session a request names, touched: its id, its person's process, and its scope.
struct Seated {
    id: String,
    pk: String,
    port: u16,
    secret: String,
    scope: Option<String>,
}

#[derive(Clone)]
struct Door {
    attempts: Arc<Mutex<HashMap<String, Attempt>>>,
    live: Arc<Mutex<Live>>,
    /// A person's turn: one sign-in, or one session's end, of theirs at a time, so two
    /// sign-ins cannot both open their process, and none opens it while their last
    /// session is still sealing it (NC-63).
    opening: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    cfg: Arc<Cfg>,
    http: reqwest::Client,
    grants: Arc<token::Grants>,
    /// S-26 until its store is designed: an append-only file of refusals (SEC-A4).
    refusals: Arc<Mutex<std::fs::File>>,
    /// Where that file is, so it can be pruned to `REFUSALS_KEPT`.
    refusals_path: Arc<std::path::PathBuf>,
    /// training_wheels (P0): where every action is recorded, or none when it is off.
    tw: Option<tw::Tw>,
    /// Kiosk claims brought to `/join`, by the handle their cookie carries (A-3).
    joins: Arc<Mutex<HashMap<String, Join>>>,
    /// The work a start without a claim pays (D-55): the key its challenges are signed
    /// under, and the seeds spent.
    pow: Arc<pow::Pow>,
    /// The key the client address is hashed under in that file: made at start and
    /// never written anywhere, so no line can be turned back into an address.
    log_key: Arc<[u8; 32]>,
    /// The Faces a Site's sign-in wears, as read from the Arc (face.rs).
    faces: Arc<face::Kept>,
}

/// A kiosk claim that checked out at `/join`, waiting for its visitor's session.
struct Join {
    token: String,
    until: Instant,
    /// What its row keeps once admitted: the choice and share the Door verified, and no
    /// more of the claim.
    kept: tw::Kept,
}

/// The fields of a verified claim a `/v2/join` row keeps: its choice (`c`) and its share
/// (`a`), where it has them. Never its nonce, signature, kid or the token.
fn kept_of(c: &pacific_core::claim::Claim) -> tw::Kept {
    let mut m = serde_json::Map::new();
    if let Some(choice) = &c.choice {
        m.insert("c".into(), choice.clone().into());
    }
    if let Some(share) = &c.artifact {
        m.insert("a".into(), share.clone().into());
    }
    tw::Kept(m)
}

struct Cfg {
    root: std::path::PathBuf,
    /// Where a person's process leaves its state sealed at rest, off the tmpfs (D-34 (c)).
    /// Each session's fold cache bound, MiB (O-69): part of its memory budget.
    fold_cache_mib: u64,
    /// How long a contact code's key packages live, seconds (W-96).
    code_secs: u64,
    seals: std::path::PathBuf,
    relay: String,
    /// The Arc whose node admits a kiosk's visitors (`/v1/admit`), and the kiosk keys
    /// this Door checks a claim against: the Arc node's own list (A-3).
    arc: String,
    claim_keys: HashMap<String, [u8; 32]>,
    join_cookie: &'static str,
    /// The auth service's origin, and its host: what the wrap and the handle name.
    auth: String,
    auth_host: String,
    /// This Door's own origin and host: the audience of its challenge.
    public: String,
    audience: String,
    /// The origins a credentialed cross-origin request may come from (SEC-8).
    origins: Vec<String>,
    cookie: &'static str,
    secure: bool,
    idle: Duration,
    /// How long a sign-in may wait for its passkey, and a sign-up for its words to
    /// be written down and its passkey made (NC-40: short, so a start holds little).
    attempt_ttl: Duration,
    signup_ttl: Duration,
    /// Open attempts, Door-wide: counted apart from live sessions, so starts cannot
    /// crowd out the people already signed in.
    max_attempts: usize,
    /// Of those, the kiosk's visitors' (D-55): claim-carrying sign-ups draw on these
    /// and nothing else does, so a flood without a claim cannot shut them out.
    joining: usize,
    max: usize,
    sync_secs: u64,
    /// The passkey's relying party (D-33 decides it; `wallflowers.io` holds every
    /// passkey made so far).
    rp_id: String,
    /// Open attempts one address may hold.
    per_addr: usize,
    /// Behind an edge that names the client (`CF-Connecting-IP`), believe it; on
    /// loopback there is none, and the peer address is the client.
    trust_forwarded: bool,
}

impl Cfg {
    fn ttl(&self, signup: bool) -> Duration {
        if signup { self.signup_ttl } else { self.attempt_ttl }
    }

    /// The open attempts a pool may hold: the reserve, or the rest.
    fn cap(&self, joining: bool) -> usize {
        if joining { self.joining } else { self.max_attempts.saturating_sub(self.joining) }
    }
}

/// The Arc's origin, from its relay's address: `wss://h/v1/relay` is `https://h`.
fn arc_of(relay: &str) -> String {
    let (scheme, rest) = relay.split_once("://").unwrap_or(("ws", relay));
    let http = if scheme == "wss" { "https" } else { "http" };
    format!("{http}://{}", rest.split('/').next().unwrap_or(""))
}

fn host_of(origin: &str) -> String {
    origin.split("://").nth(1).unwrap_or(origin).split('/').next().unwrap_or("").to_string()
}

fn env(name: &str, default: &str) -> String {
    std::env::var(name).ok().filter(|s| !s.is_empty()).unwrap_or_else(|| default.to_string())
}

fn main() {
    if std::env::var("DOOR_MODE").as_deref() == Ok("session") {
        account::run();
        return;
    }
    // PROVENANCE FIRST (arc/lib/arc-build): `--version` answers with the deploy's stamp
    // before anything is read. The configuration is the environment (mdr/door.md §8), so
    // any other flag is refused, not run.
    arc_build::stamp!();
    IMAGE.get_or_init(image_id);
    // `door tw-keygen <path>`: training_wheels' spool key pair. The secret goes to <path>, 0400,
    // never over a file already there; the public half is printed, for DOOR_TW_SPOOL_PUB.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [cmd, path] = args.as_slice() {
        if cmd == "tw-keygen" {
            match tw::spool::keygen(std::path::Path::new(path)) {
                Ok(public) => println!("{public}"),
                Err(e) => {
                    eprintln!("door tw-keygen: {e}");
                    std::process::exit(1);
                }
            }
            return;
        }
    }
    if let Some(other) = args.first() {
        eprintln!("door: takes no flags ({other}); it is configured by its environment (mdr/door.md §8)");
        std::process::exit(2);
    }

    let port: u16 = env("DOOR_PORT", "8233").parse().expect("DOOR_PORT");
    let public = env("DOOR_PUBLIC", &format!("http://127.0.0.1:{port}")).trim_end_matches('/').to_string();
    let auth = env("DOOR_AUTH", "http://127.0.0.1:8021").trim_end_matches('/').to_string();
    let relay = env("DOOR_RELAY", "ws://127.0.0.1:8787/v1/relay");
    // The Arc serves its relay and its admission from one origin.
    let arc = env("DOOR_ARC", &arc_of(&relay)).trim_end_matches('/').to_string();
    // Derived from where this runs, never written down as an absolute path.
    let root = std::path::PathBuf::from(env(
        "DOOR_ROOT",
        &std::env::current_dir().unwrap_or_else(|_| ".".into()).join(".door-state").to_string_lossy(),
    ));
    std::fs::create_dir_all(root.join("sessions")).expect("create the session root");
    let seals = seals_dir(
        std::env::var("DOOR_SEALS").ok().filter(|s| !s.is_empty()),
        std::env::var("STATE_DIRECTORY").ok().filter(|s| !s.is_empty()),
        &root,
        std::env::var("RUNTIME_DIRECTORY").ok().filter(|s| !s.is_empty()),
    )
    .unwrap_or_else(|e| panic!("DOOR_SEALS: {e}"));
    std::fs::create_dir_all(&seals).expect("create the seals directory");
    std::fs::set_permissions(&seals, std::os::unix::fs::PermissionsExt::from_mode(0o700)).expect("the seals directory, 0700");
    let secure = public.starts_with("https://");
    // NC-48: the caps fit the unit's memory ceiling, read from its cgroup, not copied.
    // O-69: a session's fold cache is bounded, and its bound is part of the session's
    // budget, so the caps below stay true with the cache full (FC-10). The bound counts the
    // bytes asked for, and malloc hands out up to 1.28 times that (Software Security, 1 on
    // 6149c75): the budget charges it at 1.5.
    let fold_cache_mib: u64 = env("DOOR_FOLD_CACHE_MIB", "8").parse().expect("DOOR_FOLD_CACHE_MIB");
    let (max, max_attempts) = fit_caps(
        env("DOOR_MAX_SESSIONS", "64").parse().expect("DOOR_MAX_SESSIONS"),
        env("DOOR_MAX_ATTEMPTS", "512").parse().expect("DOOR_MAX_ATTEMPTS"),
        env("DOOR_PROCESS_MIB", "16").parse::<u64>().expect("DOOR_PROCESS_MIB") + (fold_cache_mib * 3).div_ceil(2),
        env("DOOR_ATTEMPT_MIB", "4").parse().expect("DOOR_ATTEMPT_MIB"),
        memory_max(),
    );
    // D-55: the kiosk's reserve, carved out of the fitted attempts: an eighth.
    let joining = env("DOOR_MAX_JOINING", &(max_attempts / 8).to_string())
        .parse::<usize>()
        .expect("DOOR_MAX_JOINING")
        .min(max_attempts);

    // THE RELAY, when this image is the one that runs it (run.sh): given a binary,
    // it starts one and owns it.
    let relay_bin = std::env::var("DOOR_RELAY_BIN").ok().filter(|s| !s.is_empty());
    let relay_port: u16 = env("DOOR_RELAY_PORT", "8788").parse().expect("DOOR_RELAY_PORT");
    let relay = match &relay_bin {
        Some(_) => format!("ws://127.0.0.1:{relay_port}/v1/relay"),
        None => relay,
    };
    let _relay_proc = relay_bin.as_ref().and_then(|bin| spawn_relay(bin, relay_port).map_err(|e| eprintln!("door: {e}")).ok());

    let cfg = Cfg {
        fold_cache_mib,
        code_secs: env("DOOR_CODE_SECS", "1800").parse().expect("DOOR_CODE_SECS"),
        root: root.clone(),
        seals,
        relay: relay.clone(),
        arc,
        claim_keys: pacific_core::claim::load_keys(std::env::var("DOOR_CLAIM_KEYS").ok().as_deref())
            .unwrap_or_else(|e| panic!("DOOR_CLAIM_KEYS: {e}")),
        join_cookie: if secure { "__Host-door-join" } else { "door-join" },
        auth_host: host_of(&auth),
        auth,
        audience: host_of(&public),
        public: public.clone(),
        origins: env("DOOR_ORIGIN", "http://127.0.0.1:8232,http://127.0.0.1:8231")
            .split(',')
            .map(|o| o.trim().to_string())
            .filter(|o| !o.is_empty())
            .collect(),
        // `__Host-` binds the cookie to this exact origin; it needs Secure, so plain
        // loopback uses the bare name.
        cookie: if secure { "__Host-door" } else { "door" },
        secure,
        idle: Duration::from_secs(env("DOOR_IDLE_SECS", "900").parse().expect("DOOR_IDLE_SECS")),
        attempt_ttl: Duration::from_secs(env("DOOR_ATTEMPT_SECS", "60").parse().expect("DOOR_ATTEMPT_SECS")),
        signup_ttl: Duration::from_secs(env("DOOR_SIGNUP_SECS", "300").parse().expect("DOOR_SIGNUP_SECS")),
        max_attempts,
        joining,
        max,
        sync_secs: env("DOOR_SYNC_SECS", "3").parse().expect("DOOR_SYNC_SECS"),
        rp_id: rp_id_for(&public, std::env::var("DOOR_RP_ID").ok().filter(|s| !s.is_empty()).as_deref())
            .unwrap_or_else(|e| panic!("DOOR_RP_ID: {e}")),
        per_addr: env("DOOR_ATTEMPTS_PER_ADDR", "4").parse().expect("DOOR_ATTEMPTS_PER_ADDR"),
        trust_forwarded: env("DOOR_TRUST_FORWARDED", "0") == "1",
    };
    let refusals_path = std::path::PathBuf::from(env("DOOR_REFUSALS", &root.join("refusals.jsonl").to_string_lossy()));
    let refusals = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&refusals_path)
        .unwrap_or_else(|e| panic!("the refusal record {}: {e}", refusals_path.display()));
    let mut log_key = [0u8; 32];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut log_key);
    let webapp = env("DOOR_WEBAPP", concat!(env!("CARGO_MANIFEST_DIR"), "/../web/webapp"));

    let door = Door {
        attempts: Arc::new(Mutex::new(HashMap::new())),
        live: Default::default(),
        opening: Default::default(),
        http: reqwest::Client::builder().timeout(Duration::from_secs(60)).build().expect("http"),
        cfg: Arc::new(cfg),
        grants: Arc::new(token::Grants::load(
            std::env::var("DOOR_CLIENTS").ok().filter(|s| !s.is_empty()),
            Duration::from_secs(env("DOOR_TOKEN_SECS", "3600").parse().expect("DOOR_TOKEN_SECS")),
        )),
        refusals: Arc::new(Mutex::new(refusals)),
        refusals_path: Arc::new(refusals_path.clone()),
        // TRAINING_WHEELS (P0): on when its env is whole, and said either way.
        tw: match tw::from_env() {
            Ok(t) => {
                println!("door: training_wheels on");
                Some(t)
            }
            Err(why) => {
                println!("door: training_wheels off: {why}");
                None
            }
        },
        log_key: Arc::new(log_key),
        faces: Arc::new(face::Kept::default()),
        joins: Arc::new(Mutex::new(HashMap::new())),
        pow: Arc::new(pow::Pow::new()),
    };
    prune_refusals(&door);

    let app = router(&door, &webapp);

    // DOOR_CLIENTS, READ AGAIN WHEN IT CHANGES (R3.1 (B)): no restart, so no session ends for
    // a client added or removed. A file that fails is refused by name, the clients before it kept.
    if let Some(path) = door.grants.source().map(str::to_string) {
        let door = door.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(token::RELOAD_EVERY);
            match door.grants.reload() {
                None => {}
                Some(Ok(n)) => println!("door: DOOR_CLIENTS {path} read again: {n} clients"),
                Some(Err(why)) => eprintln!("door: DOOR_CLIENTS {path} refused, the clients before it kept: {why}"),
            }
        });
    }

    // THE SWEEPER: an idle session ends (its head stored first), and an attempt
    // nobody finished is killed. Ending is the only way this design forgets a seed.
    {
        let door = door.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(10));
            // A panic would end this thread silently, and every expiry with it: the caps
            // fill and stay full, and Restart=on-failure never sees it. The process ends
            // instead, and is restarted.
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sweep(&door))).is_err() {
                eprintln!("door: the sweeper panicked; ending the process");
                std::process::abort();
            }
        });
    }

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("runtime");
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.expect("bind the door");
        println!("door: {public} on 127.0.0.1:{port} · auth {} · relay {relay} · webapp {webapp}", door.cfg.auth);
        println!("door: LOOPBACK ONLY until PC-1 is met (srr/security.md)");
        axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>())
            .await
            .expect("serve");
    });
}

/// Every route the Door serves, and the webapp's files beneath them.
fn router(door: &Door, webapp: &str) -> Router {
    // The listed origins (the cookie's, SEC-8) and every origin a client registers (the
    // token's; SameSite=Strict keeps the cookie off their requests), asked of the registration
    // as it stands at each request: DOOR_CLIENTS is read again without a restart (R3.1 (B)).
    let listed: Vec<HeaderValue> = door.cfg.origins.iter().filter_map(|o| o.parse().ok()).collect();
    let grants = door.grants.clone();
    let cors = CorsLayer::new()
        .allow_origin(tower_http::cors::AllowOrigin::predicate(move |origin: &HeaderValue, _: &axum::http::request::Parts| {
            listed.contains(origin) || origin.to_str().is_ok_and(|o| grants.allows_origin(o))
        }))
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION, header::HeaderName::from_static("dpop")])
        .allow_credentials(true);
    // The webapp's page, naming the model's hash (icd.rs), which is also its gzip, made here, at
    // start, rather than on the first landing.
    icd::hash();
    let index = std::path::Path::new(webapp).join("index.html");
    let page = move |h: HeaderMap| {
        let index = index.clone();
        async move { icd::page(&index, &h).await }
    };

    let routes = Router::new()
        .route("/v2/icd", get(icd::current))
        .route("/v2/icd/:hash", get(icd::by_hash))
        .route("/v2/signup", post(sign_up))
        .route("/v2/signup/finish", post(sign_up_finish))
        .route("/v2/signup/continue", post(sign_up_continue))
        .route("/v2/signin", post(sign_in))
        .route("/v2/work", get(work))
        .route("/v2/signin/finish", post(sign_in_finish))
        .route("/v2/signout", post(sign_out))
        .route("/v2/token", post(token_exchange))
        .route("/v2/signin.js", get(element_js))
        .route("/v2/resources.js", get(resources_js))
        .route("/v2/me", get(fwd))
        .route("/v2/kinds", get(fwd))
        .route("/v2/graph", get(fwd))
        .route("/v2/members", get(fwd))
        .route("/v2/members/:site", get(fwd))
        .route("/v2/draft/:kind", get(fwd))
        .route("/v2/mint", post(fwd))
        .route("/v2/apply", post(fwd))
        .route("/v2/add", post(fwd))
        .route("/v2/history", post(fwd))
        .route("/v2/batch", post(fwd))
        .route("/v2/site/address", post(fwd))
        .route("/v2/site/:site/items", get(site_items))
        .route("/v2/events", get(fwd_stream))
        .route("/v2/join", post(join_finish))
        .route("/v2/bundle", post(contact_code))
        .route("/join", get(join))
        .route("/signin", get(signin))
        .route("/signin/site", get(site_step))
        .route("/signin/site", post(site_chosen))
        .route("/door/seal.js", get(seal_js))
        .route("/door/signin.js", get(signin_js))
        .route("/door/pow-worker.js", get(pow_worker_js))
        .route("/door/return-path.js", get(return_path_js))
        .route("/door/perf.js", get(perf_js))
        .route("/door/signin.css", get(signin_css))
        .route("/door/wordmark.svg", get(wordmark))
        .route("/door/wallflowers.png", get(wallflowers_png))
        .route("/door/face.css", get(face_css))
        .route("/door/face-mark", get(face_mark))
        .route("/", get(page.clone()))
        .route("/index.html", get(page))
        .fallback_service(tower_http::services::ServeDir::new(&webapp));
    // TRAINING_WHEELS (P0): each action recorded as it answers, before the client hears.
    let routes = match &door.tw {
        Some(t) => routes.layer(axum::middleware::from_fn_with_state(t.clone(), tw::capture)),
        None => routes,
    };
    routes
        .layer(cors)
        .layer(axum::middleware::from_fn(sandboxed))
        .layer(axum::middleware::from_fn(pinned))
        .layer(axum::middleware::map_response(never_framed))
        .layer(axum::middleware::map_response_with_state(door.clone(), over_tls))
        .layer(axum::middleware::from_fn_with_state(door.clone(), on_the_record))
        .with_state(door.clone())
}

/// THE PASSKEY'S RP ID IS THIS DOOR'S OWN HOST (D-33, O-58): the ceremony's output is
/// the seed, so no other host may be able to ask for it. `localhost` on loopback.
/// Anything else is refused, and the Door does not start.
fn rp_id_for(public: &str, set: Option<&str>) -> Result<String, String> {
    let host = host_of(public);
    let own = match host.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => host.as_str(),
    };
    let own = if matches!(own, "localhost" | "127.0.0.1" | "[::1]") { "localhost" } else { own };
    match set {
        Some(other) if other != own => Err(format!("{other} is not this Door's own host, {own}")),
        _ => Ok(own.to_string()),
    }
}

/// WHERE A PERSON'S SEALED STATE LIVES (D-34 (c)): `DOOR_SEALS`, else the unit's
/// StateDirectory (`$STATE_DIRECTORY/seals`), else beside the root. Never on what a
/// restart clears: refused under the root, a tmpfs in production, or the unit's
/// RuntimeDirectory, naming both.
fn seals_dir(
    explicit: Option<String>,
    state: Option<String>,
    root: &std::path::Path,
    runtime: Option<String>,
) -> Result<std::path::PathBuf, String> {
    let dir = match (explicit, state) {
        (Some(d), _) => std::path::PathBuf::from(d),
        // systemd lists one per StateDirectory=, colon-separated: the Door has one.
        (None, Some(s)) => std::path::PathBuf::from(s.split(':').next().unwrap_or_default()).join("seals"),
        (None, None) => root.parent().unwrap_or(root).join("seals"),
    };
    let cleared = std::iter::once(root.to_path_buf()).chain(runtime.iter().flat_map(|r| r.split(':').map(std::path::PathBuf::from)));
    for gone in cleared {
        if plain(&dir).starts_with(plain(&gone)) {
            return Err(format!("{} is under {}, which a restart clears", dir.display(), gone.display()));
        }
    }
    Ok(dir)
}

/// `p` made absolute and read as written: `.` dropped, `..` taken back. Nothing on disk
/// is consulted, so a directory not yet made compares as well as one that is.
fn plain(p: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let abs = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(p) };
    let mut out = std::path::PathBuf::new();
    for c in abs.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// The memory ceiling of the cgroup this process runs in (cgroup v2), or `None` without one.
fn memory_max() -> Option<u64> {
    let cg = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let path = cg.lines().find_map(|l| l.strip_prefix("0::"))?;
    std::fs::read_to_string(format!("/sys/fs/cgroup{path}/memory.max")).ok()?.trim().parse().ok()
}

/// NC-48: every session is a process budgeted at `session_mib` MiB, the supervisor one
/// more, and every open attempt one at `attempt_mib` (run 44 measured 1.3 MiB a sign-in,
/// 2.3 a sign-up). What the caps allow must fit the ceiling, or a flood meets the OOM
/// killer instead of a refusal. If it does not, the sessions come first: they keep their
/// cap and the attempts get what is left. Only where the sessions alone do not fit are
/// both lowered in the same proportion. The numbers are said.
fn fit_caps(sessions: usize, attempts: usize, session_mib: u64, attempt_mib: u64, ceiling: Option<u64>) -> (usize, usize) {
    let Some(ceiling) = ceiling else { return (sessions, attempts) };
    let (s_mib, a_mib) = (session_mib.max(1), attempt_mib.max(1));
    // What the ceiling holds besides the supervisor, in MiB.
    let room = (ceiling >> 20).saturating_sub(s_mib);
    let charge = |s: usize, a: usize| s as u64 * s_mib + a as u64 * a_mib;
    if charge(sessions, attempts) <= room {
        return (sessions, attempts);
    }
    let (s, a) = if charge(sessions, 1) <= room {
        (sessions, ((room - sessions as u64 * s_mib) / a_mib) as usize)
    } else {
        let s = ((sessions as u64 * room / charge(sessions, attempts)) as usize).max(1);
        (s, ((room.saturating_sub(s as u64 * s_mib) / a_mib) as usize).max(1))
    };
    println!(
        "door: caps lowered to {s} sessions and {a} attempts: memory.max {} MiB holds them at {s_mib} and {a_mib} MiB (NC-48)",
        ceiling >> 20
    );
    (s, a)
}

/// How long the refusal record keeps a line (TBD-12, Ralph 27 Sep: 30 days).
const REFUSALS_KEPT: u64 = 30 * 24 * 60 * 60;

/// The lines of a refusal record no older than `kept` at `now`. A line with no time
/// it can be dated by is dropped: it could never age out.
fn keep_recent(record: &str, now: u64, kept: u64) -> String {
    record
        .lines()
        .filter(|l| {
            serde_json::from_str::<serde_json::Value>(l)
                .ok()
                .and_then(|v| v["t"].as_u64())
                .is_some_and(|t| t + kept >= now)
        })
        .map(|l| format!("{l}\n"))
        .collect()
}

/// Drop what the record has kept longer than `REFUSALS_KEPT`: rewritten beside itself
/// and moved over, under the writer's own lock, so no refusal is lost to the prune.
fn prune_refusals(door: &Door) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut file = door.refusals.lock().unwrap();
    let path = door.refusals_path.as_path();
    let Ok(record) = std::fs::read_to_string(path) else { return };
    let kept = keep_recent(&record, now, REFUSALS_KEPT);
    if kept.len() == record.len() {
        return;
    }
    let next = path.with_extension("jsonl.next");
    let reopened = std::fs::write(&next, &kept)
        .and_then(|_| std::fs::rename(&next, path))
        .and_then(|_| std::fs::OpenOptions::new().append(true).open(path));
    match reopened {
        Ok(f) => *file = f,
        Err(e) => eprintln!("door: the refusal record could not be pruned: {e}"),
    }
}

fn sweep(door: &Door) {
    // The refusal record, hourly (TBD-12).
    static PRUNED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let hour = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 3600).unwrap_or(0);
    if PRUNED.swap(hour, std::sync::atomic::Ordering::Relaxed) != hour {
        prune_refusals(door);
    }
    door.attempts.lock().unwrap().retain(|_, a| a.started.elapsed() < door.cfg.ttl(a.signup));
    door.joins.lock().unwrap().retain(|_, j| j.until > Instant::now());
    door.pow.sweep(now_secs());
    // A code never redeemed, or a token expired, leaves a session nobody holds.
    door.grants.sweep();
    let idle: Vec<(String, String)> = door
        .live
        .lock()
        .unwrap()
        .sessions
        .iter()
        .filter(|(k, v)| stale(door, k, v))
        .map(|(k, v)| (k.clone(), v.pk.clone()))
        .collect();
    for (id, pk) in idle {
        // In the person's turn, or not this sweep: a sign-in of theirs under way may be
        // opening their process, and the next sweep comes in ten seconds (NC-63).
        let turn = turn_of(door, &pk);
        let Ok(held) = turn.try_lock() else { continue };
        let ended = {
            let mut live = door.live.lock().unwrap();
            if live.sessions.get(&id).is_some_and(|v| stale(door, &id, v)) { live.end(&id) } else { None }
        };
        if let Some((_, e)) = ended {
            println!("door: session for {}… idle, ending", &pk[..12.min(pk.len())]);
            close_blocking(&id, e);
        }
        drop(held);
        turn_done(door, &pk, &turn);
    }
}

/// A session the sweeper ends: idle, unless a sign-up's visitor is still keeping the
/// words (NC-53), or a client's whose token is gone.
fn stale(door: &Door, id: &str, s: &Session) -> bool {
    (s.seen.elapsed() > door.cfg.idle && !door.grants.pending(id)) || (s.token && !door.grants.holds(id))
}

/// `pk`'s turn (NC-63).
fn turn_of(door: &Door, pk: &str) -> Arc<tokio::sync::Mutex<()>> {
    door.opening.lock().unwrap().entry(pk.to_string()).or_default().clone()
}

/// A turn let go is forgotten once nobody else waits on it: only the map and `turn` hold it.
fn turn_done(door: &Door, pk: &str, turn: &Arc<tokio::sync::Mutex<()>>) {
    let mut opening = door.opening.lock().unwrap();
    if Arc::strong_count(turn) == 2 {
        opening.remove(pk);
    }
}

/// Served over TLS, the Door tells the browser to keep to it (DT-29).
async fn over_tls(State(door): State<Door>, mut r: Response) -> Response {
    if door.cfg.secure {
        r.headers_mut()
            .insert("strict-transport-security", HeaderValue::from_static("max-age=31536000"));
    }
    r
}

/// SEC-27: no Door page is framed, anywhere. And no Door response is sniffed, and
/// none sends a referrer. A page with its own policy keeps it; it says the same.
async fn never_framed(mut r: Response) -> Response {
    let h = r.headers_mut();
    if !h.contains_key("content-security-policy") {
        h.insert("content-security-policy", HeaderValue::from_static("frame-ancestors 'none'"));
    }
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    if !h.contains_key("referrer-policy") {
        h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    }
    r
}

/// The website's `/assets`, baked into the webapp (D-54), are served sandboxed: none runs
/// as this origin, whatever it holds (Software Security). Still never framed.
async fn sandboxed(req: Request, next: axum::middleware::Next) -> Response {
    let assets = req.uri().path().starts_with("/assets/");
    let mut r = next.run(req).await;
    if assets {
        r.headers_mut().insert("content-security-policy", HeaderValue::from_static("sandbox; frame-ancestors 'none'"));
    }
    r
}

/// A pinned file (`<name>.<16 hex of its sha384>.js|css`, as deploy-door.sh stages the
/// webapp and its livery): its name changes with its bytes, so a site that mounts the
/// webapp may cache it for good.
async fn pinned(req: Request, next: axum::middleware::Next) -> Response {
    let pinned = is_pinned(req.uri().path());
    let mut r = next.run(req).await;
    if pinned && r.status().is_success() {
        r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=31536000, immutable"));
    }
    r
}

/// `…/<name>.<16 lowercase hex>.js` or `.css`.
fn is_pinned(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or("");
    let Some(stem) = file.strip_suffix(".js").or_else(|| file.strip_suffix(".css")) else { return false };
    matches!(stem.rsplit_once('.'), Some((name, h)) if !name.is_empty() && h.len() == 16
        && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

fn spawn_relay(bin: &str, port: u16) -> Result<std::process::Child, String> {
    std::process::Command::new(bin)
        .env("RELAY_BIND", format!("127.0.0.1:{port}"))
        .env("RELAY_TUNNEL_BIND", format!("127.0.0.1:{}", port + 1000))
        .spawn()
        .map_err(|e| format!("could not start the relay at {bin}: {e}"))
}

/// DR-4, the sign-in window: its own script and style and nothing else, never
/// framed, never cached (DV-4). The rp id is written in at serve time.
async fn signin(
    State(door): State<Door>,
    axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    // A PUBLIC SITE'S SIGN-IN: its client and callback registered, a PKCE S256
    // challenge and a state, or no window at all (DT-9).
    if let Some(client) = q.get("client") {
        let asked = q.get("redirect_uri").map(String::as_str).unwrap_or("");
        // A client or a callback this Door has not registered (HACK_USER #18): the refusal, as a
        // page with its one way on, the registration's docs; never a link to the callback asked for.
        if let Err(why) = door.grants.callback(client, asked) {
            let no = refused(StatusCode::BAD_REQUEST, "/signin", why);
            let page = include_str!("../../web/door/signin-unregistered.html").replace("__WHY__", &face::esc(&no.1));
            let csp = "default-src 'none'; style-src 'self'; img-src 'self'; form-action 'none'; base-uri 'none'; frame-ancestors 'none'";
            let head = [(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-store"), (header::CONTENT_SECURITY_POLICY, csp)];
            let mut r = (no.0, head, page).into_response();
            r.extensions_mut().insert(Reason(no.1));
            return r;
        }
        let ok = door.grants.callback(client, asked).map(|_| ()).and_then(|_| {
            if q.get("code_challenge_method").map(String::as_str) != Some("S256") {
                return Err("code_challenge_method must be S256".to_string());
            }
            if !q.get("code_challenge").is_some_and(|c| token::s256(c)) || q.get("state").map_or(true, |s| s.is_empty()) {
                return Err("an S256 code_challenge and a state are required".to_string());
            }
            Ok(())
        });
        if let Err(why) = ok {
            return refused(StatusCode::BAD_REQUEST, "/signin", why).into_response();
        }
    }
    // A SITE'S SIGN-IN IS ONE STEP IN THAT SITE'S FACE (Ralph, 29 Sep): its name and mark at
    // the top, its look, and the WallFlowers mark small at the foot, no wordmark. The name
    // is the Face's, or the one the client registered, or its id; consent is the name on
    // the one screen the passkey is asked from (§5 step 3).
    let registered = q
        .get("client")
        .and_then(|c| door.grants.callback(c, q.get("redirect_uri").map(String::as_str).unwrap_or("")).ok())
        .map(|(c, _)| c);
    let slug = registered.as_ref().and_then(|c| c.slug.clone()).filter(|s| face::valid_slug(s));
    let brand = match &slug {
        Some(s) => brand(&door, s).await,
        None => None,
    };
    // A Site's window carries only the Site: no WallFlowers mark at its foot (Ralph, 30 Sep: "make
    // passkey (*1 page*, no wallflowers branding)").
    let (mode, title, links, top) = match &registered {
        None => (String::new(), "WallFlowers".to_string(), String::new(),
                 r#"<img class="mark" src="/door/wordmark.svg" alt="WallFlowers">"#.to_string()),
        Some(c) => {
            let name = brand.as_ref().map(|b| b.name.clone())
                .unwrap_or_else(|| if c.name.is_empty() { q["client"].clone() } else { c.name.clone() });
            let cq = face::query(&q["client"]);
            let mark = if brand.as_ref().is_some_and(|b| b.mark) {
                format!(r#"<img class="site-mark" src="/door/face-mark?client={cq}" alt="">"#)
            } else {
                String::new()
            };
            (
                if brand.is_some() { " data-site data-face".into() } else { " data-site".into() },
                face::esc(&name),
                if brand.is_some() {
                    format!("\n<link rel=\"stylesheet\" href=\"/assets/face/vendor/fonts.css\">\n<link rel=\"stylesheet\" href=\"/door/face.css?client={cq}\">")
                } else {
                    String::new()
                },
                format!(r#"<header class="site">{mark}<h1 class="site-name">{}</h1></header>"#, face::esc(&name)),
            )
        }
    };
    let page = include_str!("../../web/door/signin.html")
        .replace("__RP_ID__", &door.cfg.rp_id)
        .replace("__MODE__", &mode)
        .replace("__TITLE__", &title)
        .replace("__LINKS__", &links)
        .replace("__TOP__", &top);
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; script-src 'self'; worker-src 'self'; style-src 'self'; connect-src 'self'; \
                 img-src 'self' data:; font-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
            ),
        ],
        page,
    )
        .into_response()
}

/// A Site's Face, as the Arc's gateway serves it for a window elsewhere: the Arc's answer,
/// a Face or its word that there is none, kept a minute; a read that failed, seconds
/// (face.rs). A slow or silent Arc costs one wait in those seconds, never one a window.
async fn brand(door: &Door, slug: &str) -> Option<face::Brand> {
    if let Some(b) = door.faces.get(slug) {
        return b;
    }
    let read = async {
        let r = door.http.get(format!("{}/v1/face/{slug}/brand", door.cfg.arc)).timeout(Duration::from_secs(3)).send().await.ok()?;
        if r.status() == StatusCode::NOT_FOUND {
            return Some(None);
        }
        if !r.status().is_success() {
            return None;
        }
        Some(face::brand_of(&r.json::<serde_json::Value>().await.ok()?))
    };
    let (got, kept) = match read.await {
        Some(answer) => (answer, face::KEPT),
        None => (None, face::KEPT_FAILED),
    };
    door.faces.put(slug, got.clone(), kept);
    got
}

/// The Face a registered client's window wears: its slug, if it names one.
fn client_slug(door: &Door, q: &HashMap<String, String>) -> Option<String> {
    let c = door.grants.client(q.get("client")?)?;
    c.slug.clone().filter(|s| face::valid_slug(s))
}

/// The look a Site's sign-in wears, as a stylesheet from this origin.
async fn face_css(State(door): State<Door>, axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>) -> Response {
    let Some(slug) = client_slug(&door, &q) else { return StatusCode::NOT_FOUND.into_response() };
    let Some(b) = brand(&door, &slug).await else { return StatusCode::NOT_FOUND.into_response() };
    ([(header::CONTENT_TYPE, "text/css"), (header::CACHE_CONTROL, "private, max-age=60")], face::css(&b)).into_response()
}

/// A Site's mark, from its Face on the Arc, served from this origin: a picture, and only
/// one of the three kinds the gateway itself serves.
async fn face_mark(State(door): State<Door>, axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>) -> Response {
    let Some(slug) = client_slug(&door, &q) else { return StatusCode::NOT_FOUND.into_response() };
    let got = door.http.get(format!("{}/v1/face/{slug}/m/mark", door.cfg.arc)).timeout(Duration::from_secs(3)).send().await;
    let Ok(r) = got else { return StatusCode::NOT_FOUND.into_response() };
    let ctype = r.headers().get(header::CONTENT_TYPE).and_then(|h| h.to_str().ok()).unwrap_or("").to_string();
    if !r.status().is_success() || !matches!(ctype.as_str(), "image/png" | "image/jpeg" | "image/webp") {
        return StatusCode::NOT_FOUND.into_response();
    }
    match r.bytes().await {
        Ok(body) => ([(header::CONTENT_TYPE, ctype), (header::CACHE_CONTROL, "public, max-age=300".to_string())], body).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The WallFlowers mark at the foot of a Site's sign-in (business/branding's
/// wallflowers/instagram/wallflowers-profile.png, at 144 px).
async fn wallflowers_png() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, "public, max-age=86400")], &include_bytes!("../../web/door/wallflowers.png")[..])
}

/// DR-4's scripts and style, served from this origin so the window runs no other.
async fn seal_js() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/javascript")], include_str!("../../web/door/seal.js"))
}

async fn signin_js() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/javascript")], include_str!("../../web/door/signin.js"))
}

/// The window's proof of work, run as a Worker from this origin (D-55).
async fn pow_worker_js() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/javascript")], include_str!("../../web/door/pow-worker.js"))
}

async fn return_path_js() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/javascript")], include_str!("../../web/door/return-path.js"))
}

/// User Timing for the window and the webapp: measured in the tab, never sent.
async fn perf_js() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/javascript")], include_str!("../../web/door/perf.js"))
}

/// DR-5, the element a public site loads: it starts the callback flow and spends
/// the token with a key the page cannot export.
async fn element_js() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/javascript")], include_str!("../../web/door/element.js"))
}

/// W-98 Resources, the editor a public site loads (and the webapp): one script, pinned by its
/// SRI, built from its parts in order — the CommonMark reference parser in its own scope,
/// RESOURCES-B's PDF module, then the editor (app/web/resources/). The docs' SRI and the
/// editor's tests read this list, so there is one.
const RESOURCES_JS: &str = concat!(
    include_str!("../../web/resources/vendor/pre.js"),
    include_str!("../../web/resources/vendor/commonmark.min.js"),
    include_str!("../../web/resources/vendor/post.js"),
    include_str!("../../web/resources/pdf.js"),
    include_str!("../../web/resources/markdown.js"),
    include_str!("../../web/resources/editor.js"),
);

async fn resources_js() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/javascript")], RESOURCES_JS)
}

/// The window's mark, from the binary, so the window depends on no webapp files.
async fn wordmark() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "image/svg+xml")], include_str!("../../web/webapp/brand/wallflowers-wordmark.svg"))
}

async fn signin_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css")], include_str!("../../web/door/signin.css"))
}

// ─── refusals ────────────────────────────────────────────────────────────────

/// A refusal, in the Door's own words. The words ride the response as a [`Reason`]
/// too, and that is what [`on_the_record`] writes down: a refusal the Door did not
/// word (a malformed body that axum rejects, quoting it) is recorded by its status
/// alone, so no one's content reaches the record (SEC-33).
struct Refusal(StatusCode, String);

#[derive(Clone)]
struct Reason(String);

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let mut r = (self.0, self.1.clone()).into_response();
        r.extensions_mut().insert(Reason(self.1));
        r
    }
}

fn refused(status: StatusCode, _route: &str, why: impl Into<String>) -> Refusal {
    Refusal(status, why.into())
}

/// The unit a cap counts (NC-40): an IPv4 address, or an IPv6 /64, which one
/// party holds whole and can draw a fresh address from for every request.
fn network_of(addr: &str) -> String {
    match addr.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V6(v6)) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let s = v6.segments();
                format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
            }
        },
        _ => addr.to_string(),
    }
}

/// Who is asking, as a keyed hash: the peer address, or the edge's word for it
/// when this Door sits behind one it trusts.
#[derive(Clone)]
struct ClientTag(String);

/// EVERY REFUSAL ON THE RECORD (SEC-A4; S-26 as an append-only file until its store
/// is designed): the time, the route, the status, the reason in the words the
/// person got, and a keyed hash of the client address. The reason is never a
/// token, a code, a proof or a PRF output: none of those is ever put in one
/// (SEC-33). A static file that is simply not there is not a refusal.
async fn on_the_record(State(door): State<Door>, mut req: Request, next: axum::middleware::Next) -> Response {
    let addr = req
        .headers()
        .get("cf-connecting-ip")
        .filter(|_| door.cfg.trust_forwarded)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| {
            req.extensions()
                .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                .map(|c| c.0.ip().to_string())
        })
        .map(|a| network_of(&a))
        .unwrap_or_default();
    let mut h = <sha2::Sha256 as sha2::Digest>::new();
    sha2::Digest::update(&mut h, door.log_key.as_slice());
    sha2::Digest::update(&mut h, addr.as_bytes());
    let client = hex::encode(&sha2::Digest::finalize(h)[..12]);
    req.extensions_mut().insert(ClientTag(client.clone()));
    let route = req.uri().path().to_string();
    let r = next.run(req).await;
    let status = r.status();
    let ours = route.starts_with("/v2/") || route == "/signin" || route == "/join";
    if !(status.is_client_error() || status.is_server_error()) || !ours {
        return r;
    }
    // THE DOOR'S REASON, NOT THE CONTENT: its own words, at most 300 characters;
    // nothing for a refusal it did not word.
    let why: String = r.extensions().get::<Reason>().map(|x| x.0.chars().take(300).collect()).unwrap_or_default();
    let line = serde_json::json!({
        "t": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        "route": route, "status": status.as_u16(), "reason": why, "client": client,
    });
    eprintln!("door: refused {} {route}: {why}", status.as_u16());
    if let Ok(mut f) = door.refusals.lock() {
        let _ = writeln!(f, "{line}");
    }
    r
}

fn random_hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut b);
    hex::encode(b)
}

// ─── session processes ───────────────────────────────────────────────────────

/// The image a session process runs: the supervisor's own, the one its config line and
/// leash are for (NC-80). On Linux a binary replaced while it runs, as a deploy's `install`
/// replaces /opt/door/bin/door before the restart, is "<path> (deleted)" to current_exe(),
/// and its path names the new build; /proc/self/exe is still the running image.
fn session_exe() -> std::io::Result<std::path::PathBuf> {
    let own = std::path::Path::new("/proc/self/exe");
    if own.exists() { Ok(own.into()) } else { std::env::current_exe() }
}

/// This build, as a session names it: its stamp's commit, or "unstamped".
pub(crate) fn build_name() -> String {
    let s: serde_json::Value = serde_json::from_str(&arc_build::json("door")).unwrap_or_default();
    match (s["stamped"].as_bool(), s["short"].as_str()) {
        (Some(true), Some(short)) => format!("{short}{}", if s["dirty"].as_bool() == Some(true) { "-dirty" } else { "" }),
        _ => "unstamped".into(),
    }
}

/// The image's file: every unstamped build has one stamp, and a rebuilt file at the path the
/// supervisor started from is another image, stamped or not.
pub(crate) fn image_id() -> String {
    use std::os::unix::fs::MetadataExt;
    match session_exe().and_then(std::fs::metadata) {
        Ok(m) => format!("inode {} of {} bytes, modified {}.{:09}", m.ino(), m.size(), m.mtime(), m.mtime_nsec()),
        Err(e) => format!("unreadable ({e})"),
    }
}

/// The supervisor's image, as it was when it started (NC-80).
static IMAGE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Why a session is not its supervisor's build and image, or None (NC-80): ours, and what
/// the supervisor's config line says it is.
pub(crate) fn skew(ours: (&str, &str), theirs: (Option<&str>, Option<&str>)) -> Option<String> {
    match theirs {
        (Some(b), Some(i)) if (b, i) == ours => None,
        (Some(b), Some(i)) => Some(format!(
            "this session is build {}, image {}; its supervisor is build {b}, image {i}: the Door's binary changed under it; restart the Door",
            ours.0, ours.1
        )),
        _ => Some(format!("its supervisor did not name both its build and its image, so it is older than this session (build {}); restart the Door", ours.0)),
    }
}

/// Start one session process: its directory, and its config on the leash it dies with.
/// It binds a port of its own and says which on the leash, so no other process can take
/// the port between a choice and a bind. Nothing secret goes in its environment.
/// `join` is the handle of the kiosk claim a sign-up draws on, if any (D-55).
async fn start(door: &Door, route: &str, client: &str, join: Option<&str>) -> Result<Proc, Refusal> {
    room(door, route, client, join)?;
    let secret = random_hex(32);
    let dir = door.cfg.root.join("sessions").join(random_hex(16));
    std::fs::create_dir_all(&dir).map_err(|e| refused(StatusCode::INTERNAL_SERVER_ERROR, route, e.to_string()))?;
    let exe = session_exe().map_err(|e| refused(StatusCode::INTERNAL_SERVER_ERROR, route, e.to_string()))?;
    let (mut leash, theirs) = std::os::unix::net::UnixStream::pair()
        .map_err(|e| refused(StatusCode::INTERNAL_SERVER_ERROR, route, format!("no leash for a session: {e}")))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.env_clear()
        .env("DOOR_MODE", "session")
        .env("PACIFIC_STATE_DIR", &dir)
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .stdin(std::process::Stdio::from(std::os::fd::OwnedFd::from(theirs)));
    // O-69's test switches, to an unstamped build alone: a test's model, and FC-2's check of
    // every hit. A stamped build names its own model, and a hit that differed would end a
    // person's session; the deploy refuses them in door.env besides.
    if !arc_build::STAMPED {
        for k in ["PACIFIC_FOLD_CACHE_MODEL", "PACIFIC_FOLD_CACHE_VERIFY"] {
            if let Ok(v) = std::env::var(k) {
                cmd.env(k, v);
            }
        }
    }
    let child = cmd
        .spawn()
        .map_err(|e| refused(StatusCode::INTERNAL_SERVER_ERROR, route, format!("could not start a session: {e}")))?;
    let line = serde_json::json!({
        "secret": secret,
        "auth": door.cfg.auth,
        "host": door.cfg.auth_host,
        "door": door.cfg.audience,
        "relay": door.cfg.relay,
        "sync_secs": door.cfg.sync_secs,
        "seals": door.cfg.seals,
        "fold_cache_mib": door.cfg.fold_cache_mib,
        "code_secs": door.cfg.code_secs,
        "build": build_name(),
        "image": IMAGE.get_or_init(image_id),
        // training_wheels' Delta tap: on only where this process reads it back.
        "tap": door.tw.is_some(),
    });
    writeln!(leash, "{line}").map_err(|e| refused(StatusCode::INTERNAL_SERVER_ERROR, route, e.to_string()))?;
    // The process is killed with its Proc: here, if it named no port.
    let port = match told_port(&leash, Duration::from_secs(10)).await {
        Ok((p, rest)) => {
            // The same reader goes on to the Delta tap, so no line is lost between.
            if let Some(t) = door.tw.clone() {
                tokio::spawn(tw::listen(t, rest));
            }
            p
        }
        Err(why) => {
            let _ = Proc { child, _leash: leash, port: 0, secret, dir };
            return Err(refused(StatusCode::INTERNAL_SERVER_ERROR, route, format!("the session process did not start: {why}")));
        }
    };
    let proc = Proc { child, _leash: leash, port, secret, dir };
    // Ready when it answers with its key.
    for _ in 0..100 {
        if call(door, &proc, reqwest::Method::GET, "/i/key", None).await.is_ok() {
            return Ok(proc);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(refused(StatusCode::INTERNAL_SERVER_ERROR, route, "the session process did not start"))
}

/// The port a session process bound, as it tells it on the leash: one line, its number,
/// within `within`. Read on the runtime, through a copy of the leash, which is left
/// non-blocking. The reader is handed back: training_wheels' Delta tap follows on it.
async fn told_port(
    leash: &std::os::unix::net::UnixStream,
    within: Duration,
) -> Result<(u16, tokio::io::BufReader<tokio::net::UnixStream>), String> {
    use tokio::io::AsyncBufReadExt;
    let copy = leash.try_clone().map_err(|e| e.to_string())?;
    copy.set_nonblocking(true).map_err(|e| e.to_string())?;
    let copy = tokio::net::UnixStream::from_std(copy).map_err(|e| e.to_string())?;
    let mut line = String::new();
    let mut reader = tokio::io::BufReader::new(copy);
    match tokio::time::timeout(within, reader.read_line(&mut line)).await {
        Err(_) => Err(format!("it named no port within {} s", within.as_secs())),
        Ok(Ok(0)) => Err("it ended before naming its port".into()),
        Ok(Ok(_)) if line.starts_with("refused: ") => Err(format!("it {}", line.trim())),
        Ok(Ok(_)) => line.trim().parse().map(|p| (p, reader)).map_err(|_| format!("it named no port: {:?}", line.trim())),
        Ok(Err(e)) => Err(format!("its leash failed: {e}")),
    }
}

/// A slot for a start, or its refusal: the session cap, then its pool's, then its
/// address's.
fn room(door: &Door, route: &str, client: &str, join: Option<&str>) -> Result<(), Refusal> {
    if door.live.lock().unwrap().persons.len() >= door.cfg.max {
        return Err(refused(StatusCode::SERVICE_UNAVAILABLE, route, "the Door is at its session limit; try again shortly"));
    }
    let mut attempts = door.attempts.lock().unwrap();
    // ONE OPEN ATTEMPT PER CLAIM (Software Security, D-55): a new start for a join ends
    // the one it holds, so a photographed QR holds one slot, and a visitor whose passkey
    // was cancelled starts again at once. Its process ends once the lock is let go.
    let ended: Vec<Attempt> = match join {
        Some(j) => {
            let held: Vec<String> = attempts.iter().filter(|(_, a)| a.join.as_deref() == Some(j)).map(|(k, _)| k.clone()).collect();
            held.iter().filter_map(|k| attempts.remove(k)).collect()
        }
        None => Vec::new(),
    };
    let pool = join.is_some();
    // ONE ADDRESS CANNOT HOLD EVERY SLOT: a start is unauthenticated and costs a
    // process, so open attempts are counted per address (SEC-A2). A claim's start is
    // exempt (Software Security, D-55): a venue's visitors share its one address, and
    // each claim admits one visitor, once.
    let per_addr = if pool { usize::MAX } else { door.cfg.per_addr };
    // Each pool against its own cap (D-55).
    let r = if attempts.values().filter(|a| a.join.is_some() == pool).count() >= door.cfg.cap(pool) {
        Err(refused(StatusCode::SERVICE_UNAVAILABLE, route, "too many sign-ins are open; try again shortly"))
    } else if attempts.values().filter(|a| a.client == client && a.join.is_some() == pool).count() >= per_addr {
        Err(refused(StatusCode::TOO_MANY_REQUESTS, route, "too many sign-ins are open from here; finish one or wait"))
    } else {
        Ok(())
    };
    drop(attempts);
    drop(ended);
    r
}

/// One call to a session process, as JSON, with its bearer. Its refusal comes back
/// in its own words.
async fn call(
    door: &Door,
    proc: &Proc,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, (StatusCode, String)> {
    call_at(door, proc.port, &proc.secret, method, path, body).await
}

async fn call_at(
    door: &Door,
    port: u16,
    secret: &str,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let mut rq = door.http.request(method, format!("http://127.0.0.1:{port}{path}")).bearer_auth(secret);
    if let Some(b) = body {
        rq = rq.json(&b);
    }
    let r = rq.send().await.map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    let status = StatusCode::from_u16(r.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let text = r.text().await.map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    if !status.is_success() {
        return Err((status, text));
    }
    serde_json::from_str(&text).map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))
}

/// A session ended: its person's process forgets it, or, the last of theirs, ends: its
/// head stored and pending writes sent. The process goes when its `Proc` is dropped.
async fn close(door: &Door, id: &str, e: Ended) {
    match e {
        Ended::Forget { port, secret } => {
            let _ = call_at(door, port, &secret, reqwest::Method::POST, "/i/forget", Some(serde_json::json!({ "ref": id }))).await;
        }
        Ended::Last(proc) => {
            let _ = call(door, &proc, reqwest::Method::POST, "/i/end", None).await;
        }
    }
}

/// The same, from the sweeper's thread, which has no runtime.
fn close_blocking(id: &str, e: Ended) {
    match e {
        Ended::Forget { port, secret } => post_at(port, &secret, "/i/forget", Some(serde_json::json!({ "ref": id }))),
        Ended::Last(proc) => end_at(proc.port, &proc.secret),
    }
}

fn end_at(port: u16, secret: &str) {
    post_at(port, secret, "/i/end", None)
}

/// Built inside the runtime: reqwest arms its timeout when the request is sent, and the
/// sweeper's thread has no reactor of its own.
fn post_at(port: u16, secret: &str, path: &str, body: Option<serde_json::Value>) {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
    let _ = rt.block_on(async {
        let mut rq = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}{path}"))
            .bearer_auth(secret)
            .timeout(Duration::from_secs(30));
        if let Some(b) = body {
            rq = rq.json(&b);
        }
        rq.send().await
    });
}

// ─── the challenge ───────────────────────────────────────────────────────────

/// SEC-4's second proof: the restored identity signs a nonce this process made,
/// for this Door's host, and it verifies against the handle's key.
async fn challenge(door: &Door, proc: &Proc, pk: &str, route: &str) -> Result<(), Refusal> {
    let nonce = random_hex(16);
    let got = call(door, proc, reqwest::Method::POST, "/i/sign", Some(serde_json::json!({
        "audience": door.cfg.audience, "nonce": nonce,
    })))
    .await
    .map_err(|(_, why)| refused(StatusCode::UNAUTHORIZED, route, why))?;
    let sig: [u8; 64] = got["sig"]
        .as_str()
        .and_then(|s| hex::decode(s).ok())
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| refused(StatusCode::UNAUTHORIZED, route, "the session returned no signature"))?;
    let key: [u8; 32] = hex::decode(pk)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| refused(StatusCode::UNAUTHORIZED, route, "not a key"))?;
    let key_text = format!("ed25519:{pk}");
    let msg = pacific_core::identity::auth_payload(&door.cfg.audience, &key_text, &nonce);
    pacific_core::identity::verify_sig(&key, &msg, &sig)
        .map_err(|_| refused(StatusCode::UNAUTHORIZED, route, "the challenge was not signed by the account's key"))
}

/// Session `id` opens, one of `pk`'s: `proc` is their process, newly opened, or `None`
/// where theirs is live already.
/// `token`: a client's session, held by its token.
fn seat(door: &Door, id: &str, pk: &str, proc: Option<Proc>, scope: Option<String>, token: bool, route: &str) -> Result<(), Refusal> {
    let extra = {
        let mut live = door.live.lock().unwrap();
        let extra = if let Some(p) = live.persons.get_mut(pk) {
            p.sessions.insert(id.to_string());
            proc
        } else if let Some(proc) = proc {
            live.persons.insert(pk.to_string(), Person { proc, sessions: HashSet::from([id.to_string()]) });
            None
        } else {
            return Err(refused(StatusCode::SERVICE_UNAVAILABLE, route, "the account's process ended as this session opened; sign in again"));
        };
        live.sessions.insert(id.to_string(), Session { pk: pk.to_string(), seen: Instant::now(), scope, token });
        extra
    };
    // A process opened beside a live one is not the person's: it goes, outside the lock.
    drop(extra);
    Ok(())
}

/// The session's cookie: for this origin, HttpOnly, SameSite=Strict, and Secure
/// wherever the Door is served over TLS (SEC-7).
fn session_cookie(door: &Door, id: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    let secure = if door.cfg.secure { "; Secure" } else { "" };
    let c = format!("{}={id}; Path=/; HttpOnly; SameSite=Strict{secure}", door.cfg.cookie);
    h.insert(header::SET_COOKIE, HeaderValue::from_str(&c).expect("cookie"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h
}

fn session_id(door: &Door, h: &HeaderMap) -> Option<String> {
    cookie(h, door.cfg.cookie)
}

fn cookie(h: &HeaderMap, name: &str) -> Option<String> {
    let want = format!("{name}=");
    h.get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .map(str::trim)
        .find_map(|c| c.strip_prefix(&want).map(str::to_string))
}

/// A cookie write from a browser carries its Origin; one from anywhere but this
/// Door or a listed origin is refused (DV-9). A request with no Origin is not a
/// browser's cross-site request.
fn origin_allowed(door: &Door, h: &HeaderMap) -> bool {
    match h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        // A browser sends Origin on every write, same-origin too: a cookie write with
        // none is not the webapp's, and is refused (DV-9, Software Security).
        None => false,
        Some(o) => o == door.cfg.public || door.cfg.origins.iter().any(|x| x == o),
    }
}

// ─── sign-up and sign-in (mdr/door.md §4) ────────────────────────────────────

#[derive(Deserialize)]
struct SignUpIn {
    #[serde(default)]
    name: String,
    #[serde(default)]
    work: Option<Work>,
}

#[derive(Deserialize)]
struct SignInIn {
    #[serde(default)]
    work: Option<Work>,
}

/// The work a start carries (D-55): the Door's challenge, and the count that does it.
#[derive(Deserialize)]
struct Work {
    challenge: String,
    nonce: String,
}

/// Sign-up, first half: a process mints the identity; the window gets the session
/// key to seal to and the handle its passkey will carry. The words come with the
/// second half, once the account exists.
async fn sign_up(
    State(door): State<Door>,
    axum::Extension(ClientTag(client)): axum::Extension<ClientTag>,
    h: HeaderMap,
    body: Option<Json<SignUpIn>>,
) -> Result<Response, Refusal> {
    let route = "/v2/signup";
    let (name, work) = body.map(|Json(b)| (b.name, b.work)).unwrap_or_default();
    // A Site's new member names themselves on the one page (O-77's card on entry; 30 Sep): held
    // to its limits before anything starts. No name, as WallFlowers' own window sends, as before.
    let name = if name.is_empty() { name } else { a_name(&name).map_err(|w| refused(StatusCode::BAD_REQUEST, route, w))? };
    paid(&door, &h, pow::Start::SignUp, work.as_ref(), route)?;
    // A kiosk's visitor draws on the reserve (D-55).
    let join = join_of(&door, &h);
    let proc = start(&door, route, &client, join.as_deref()).await?;
    let key = call(&door, &proc, reqwest::Method::GET, "/i/key", None).await.map_err(|(s, w)| refused(s, route, w))?;
    let made = call(&door, &proc, reqwest::Method::POST, "/i/signup", Some(serde_json::json!({ "name": name })))
        .await
        .map_err(|(s, w)| refused(s, route, w))?;
    let attempt = random_hex(16);
    door.attempts.lock().unwrap().insert(attempt.clone(), Attempt { proc, started: Instant::now(), signup: true, client, join });
    let out = serde_json::json!({
        "attempt": attempt,
        "key": key["key"],
        "pk": format!("ed25519:{}", made["pk"].as_str().unwrap_or_default()),
        "host": made["host"],
        "handle": made["handle"],
    });
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(out)).into_response())
}

#[derive(Deserialize)]
struct FinishIn {
    attempt: String,
    sealed: seal::Sealed,
    #[serde(default)]
    handle: String,
    /// A public site's sign-in: the session goes to its callback as a code.
    #[serde(default)]
    client: Option<Callback>,
}

#[derive(Deserialize, Clone)]
struct Callback {
    client: String,
    redirect_uri: String,
    code_challenge: String,
    state: String,
}

/// How a finished sign-in answers: the webapp gets its cookie; a public site gets
/// a code, at the callback it REGISTERED, with the state unchanged (Ralph, 26 Sep:
/// "a proper callback"). The code goes nowhere else, and never in a log.
/// `proc` is the person's process, newly opened; `None` where theirs is live already.
fn landed(door: &Door, proc: Option<Proc>, pk: String, cb: Option<Callback>, mut body: serde_json::Value, route: &str) -> Result<Response, Refusal> {
    let Some(cb) = cb else {
        let id = random_hex(32);
        seat(door, &id, &pk, proc, None, false, route)?;
        // The cookie's headers carry no-store: the words may ride this body.
        return Ok(by((session_cookie(door, &id), Json(body)), &pk, &id));
    };
    let (client, registered) = door.grants.callback(&cb.client, &cb.redirect_uri).map_err(|w| refused(StatusCode::BAD_REQUEST, route, w))?;
    // A client's session reaches its Site; none reaches the whole account (O-58). A wild
    // client's reaches none until its person chooses one at the Site step (R3.1 (C)).
    let wild = client.site == token::WILD;
    let scope = Some(if wild { String::new() } else { client.site.clone() });
    let id = random_hex(32);
    // A NEW ACCOUNT'S WORDS COME FIRST (NC-53). Writing twenty-four words down takes
    // longer than a code's minute, so a sign-up waits, its session held, until the
    // visitor continues; the code is minted then. A sign-in has no words, and goes on.
    let words = body.get("words").is_some_and(|w| !w.is_null());
    let mut step = None;
    let redirect = if words {
        let handle = door
            .grants
            .pend(&cb.client, &registered, &cb.code_challenge, &cb.state, &id)
            .map_err(|w| refused(StatusCode::BAD_REQUEST, route, w))?;
        body["continue"] = serde_json::Value::String(handle);
        None
    } else if wild {
        step = Some(door.grants.pick(&cb.client, &registered, &cb.code_challenge, &cb.state, &id).map_err(|w| refused(StatusCode::BAD_REQUEST, route, w))?);
        Some(SITE_STEP.to_string())
    } else {
        let code = door.grants.code(&cb.client, &registered, &cb.code_challenge, &id).map_err(|w| refused(StatusCode::BAD_REQUEST, route, w))?;
        Some(redirect_to(&registered, &code, &cb.state))
    };
    seat(door, &id, &pk, proc, scope, true, route)?;
    if let Some(r) = redirect {
        body["redirect"] = serde_json::Value::String(r);
    }
    Ok(with_step(door, by(([(header::CACHE_CONTROL, "no-store")], Json(body)), &pk, &id), step.as_deref()))
}

/// R3.1 (C), THE SITE STEP: where a wild client's window goes after the passkey, by a Strict
/// cookie that carries its handle, never a URL.
const SITE_STEP: &str = "/signin/site";

fn pick_cookie_name(door: &Door) -> &'static str {
    if door.cfg.secure { "__Host-door-pick" } else { "door-pick" }
}

/// The Site step's cookie on `r`, for `token::PICK_LIFE`; `Some("")` clears it.
fn with_step(door: &Door, mut r: Response, handle: Option<&str>) -> Response {
    if let Some(handle) = handle {
        let life = if handle.is_empty() { 0 } else { token::PICK_LIFE.as_secs() };
        let secure = if door.cfg.secure { "; Secure" } else { "" };
        let c = format!("{}={handle}; Path=/; HttpOnly; SameSite=Strict; Max-Age={life}{secure}", pick_cookie_name(door));
        r.headers_mut().append(header::SET_COOKIE, HeaderValue::from_str(&c).expect("cookie"));
    }
    r
}

/// The Sites the person of `session` may give a wild client, read from their own graph: those
/// where they are the owner or an admin (members::sites_run_by).
async fn eligible_sites(door: &Door, session: &str, route: &str) -> Result<(String, Vec<(String, String)>), Refusal> {
    let at = {
        let live = door.live.lock().unwrap();
        live.sessions.get(session).and_then(|s| live.persons.get(&s.pk)).map(|p| (p.proc.port, p.proc.secret.clone()))
    };
    let (port, secret) = at.ok_or_else(|| refused(StatusCode::UNAUTHORIZED, route, "the sign-in's session has ended; sign in again"))?;
    let g = call_at(door, port, &secret, reqwest::Method::GET, "/v2/graph", None).await.map_err(|(s, w)| refused(s, route, w))?;
    let me = g["me"]["pk"].as_str().unwrap_or_default().trim_start_matches("ed25519:").to_ascii_lowercase();
    let sites = members::sites_run_by(&g, &me);
    Ok((me, sites))
}

/// GET /signin/site, THE SITE STEP'S PAGE: the app's name, one select of the person's Sites
/// where they are the owner or an admin, Allow and Cancel; labels and values only. No script:
/// its form posts here, and the answer goes to the client's callback, which its policy names.
/// No such Site: the Door's refusal, and the flow stops.
async fn site_step(State(door): State<Door>, h: HeaderMap) -> Result<Response, Refusal> {
    let route = SITE_STEP;
    let handle = cookie(&h, pick_cookie_name(&door)).unwrap_or_default();
    let p = door.grants.picking(&handle, false).ok_or_else(|| refused(StatusCode::BAD_REQUEST, route, "no Site is waiting to be chosen; sign in again"))?;
    let (pk, sites) = eligible_sites(&door, &p.session, route).await?;
    let name = door.grants.client(&p.client).map(|c| c.name).filter(|n| !n.is_empty()).unwrap_or_else(|| p.client.clone());
    if sites.is_empty() {
        // Taken: its session, held by nothing now, ends at the next sweep.
        door.grants.picking(&handle, true);
        // The Door's refusal, as a page with its one next step (DEVEX, R3.1): an account makes a
        // community at wallflowers.io; Cancel goes to the callback, access_denied, as the step's.
        let no = refused(StatusCode::FORBIDDEN, route, "you are the owner or an admin of no Site");
        let sep = if p.callback.contains('?') { '&' } else { '?' };
        let cancel = format!("{}{sep}error=access_denied&state={}", p.callback, url_escape(&p.state));
        let page = include_str!("../../web/door/site-none.html")
            .replace("__TITLE__", &face::esc(&name))
            .replace("__WHY__", &face::esc(&no.1))
            .replace("__CANCEL__", &face::esc(&cancel));
        let csp = "default-src 'none'; style-src 'self'; img-src 'self'; form-action 'none'; base-uri 'none'; frame-ancestors 'none'";
        let head = [(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-store"), (header::CONTENT_SECURITY_POLICY, csp)];
        let mut r = by((no.0, head, page), &pk, &p.session);
        r.extensions_mut().insert(Reason(no.1));
        return Ok(r);
    }
    let options: String = sites.iter().map(|(id, n)| format!(r#"<option value="{}">{}</option>"#, face::esc(id), face::esc(n))).collect();
    let page = include_str!("../../web/door/site.html").replace("__TITLE__", &face::esc(&name)).replace("__OPTIONS__", &options);
    let callback = reqwest::Url::parse(&p.callback).map(|u| u.origin().ascii_serialization()).unwrap_or_default();
    let csp = format!("default-src 'none'; style-src 'self'; img-src 'self'; form-action 'self' {callback}; base-uri 'none'; frame-ancestors 'none'");
    // same-origin, so the browser posts this page's own form with the Door's Origin; under
    // no-referrer it sends `Origin: null`, which site_chosen refuses (TEST in Chrome, R3.1).
    let head = [(header::CONTENT_TYPE, "text/html; charset=utf-8".to_string()), (header::CACHE_CONTROL, "no-store".to_string()), (header::CONTENT_SECURITY_POLICY, csp), (header::REFERRER_POLICY, "same-origin".to_string())];
    Ok(by((head, page), &pk, &p.session))
}

#[derive(Deserialize)]
struct Chosen {
    #[serde(default)]
    site: String,
    #[serde(default)]
    deny: Option<String>,
}

/// POST /signin/site, THE CHOICE, once: held to the person's own graph here, not to the page. A
/// Site where they are neither the owner nor an admin is refused, and no code is made; else the
/// session reaches that Site, and its code goes to the registered callback, as a fixed client's
/// does. Cancel: the callback, access_denied.
async fn site_chosen(State(door): State<Door>, h: HeaderMap, axum::Form(f): axum::Form<Chosen>) -> Result<Response, Refusal> {
    let route = SITE_STEP;
    if h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) != Some(door.cfg.public.as_str()) {
        return Err(refused(StatusCode::FORBIDDEN, route, "a write from an origin this Door does not list"));
    }
    // Taken whatever follows: a refused choice is not tried again.
    let handle = cookie(&h, pick_cookie_name(&door)).unwrap_or_default();
    let p = door.grants.picking(&handle, true).ok_or_else(|| refused(StatusCode::BAD_REQUEST, route, "no Site is waiting to be chosen; sign in again"))?;
    let (pk, to) = if f.deny.is_some() {
        let pk = door.live.lock().unwrap().sessions.get(&p.session).map(|s| s.pk.clone()).unwrap_or_default();
        let sep = if p.callback.contains('?') { '&' } else { '?' };
        (pk, format!("{}{sep}error=access_denied&state={}", p.callback, url_escape(&p.state)))
    } else {
        let (pk, sites) = eligible_sites(&door, &p.session, route).await?;
        if !sites.iter().any(|(id, _)| *id == f.site) {
            return Err(refused(StatusCode::FORBIDDEN, route, "you are neither the owner nor an admin of that Site"));
        }
        match door.live.lock().unwrap().sessions.get_mut(&p.session) {
            Some(s) => {
                s.scope = Some(f.site.clone());
                s.seen = Instant::now();
            }
            None => return Err(refused(StatusCode::UNAUTHORIZED, route, "the sign-in's session has ended; sign in again")),
        }
        let code = door.grants.code_at(&p, &f.site).map_err(|w| refused(StatusCode::BAD_REQUEST, route, w))?;
        (pk, redirect_to(&p.callback, &code, &p.state))
    };
    let r = (StatusCode::SEE_OTHER, [(header::LOCATION, to), (header::CACHE_CONTROL, "no-store".to_string())]).into_response();
    Ok(with_step(&door, by(r, &pk, &p.session), Some("")))
}

/// The registered callback, with the code and the state unchanged.
fn redirect_to(registered: &str, code: &str, state: &str) -> String {
    let sep = if registered.contains('?') { '&' } else { '?' };
    format!("{registered}{sep}code={}&state={}", url_escape(code), url_escape(state))
}

#[derive(Deserialize)]
struct ContinueIn {
    #[serde(rename = "continue")]
    handle: String,
}

/// The visitor has kept the words: the code, minted now, at the registered callback
/// (NC-53). Once per sign-up, within `token::WORDS_LIFE`.
async fn sign_up_continue(State(door): State<Door>, Json(i): Json<ContinueIn>) -> Result<Response, Refusal> {
    let route = "/v2/signup/continue";
    let c = door.grants.take_pending(&i.handle, Instant::now()).map_err(|w| refused(StatusCode::UNAUTHORIZED, route, w))?;
    // No code for a session that has ended (Software Testing, E15), and the one that
    // continues is touched, so it is not idle the moment its visitor comes back.
    let pk = match door.live.lock().unwrap().sessions.get_mut(&c.session) {
        Some(s) => {
            s.seen = Instant::now();
            s.pk.clone()
        }
        None => return Err(refused(StatusCode::UNAUTHORIZED, route, "the sign-up's session has ended; sign in to continue")),
    };
    // A wild client's sign-up goes on to the Site step (R3.1 (C)).
    if door.grants.client(&c.client).is_some_and(|k| k.site == token::WILD) {
        let step = door.grants.pick_for(&c).map_err(|w| refused(StatusCode::BAD_REQUEST, route, w))?;
        let body = serde_json::json!({ "redirect": SITE_STEP });
        return Ok(with_step(&door, by(([(header::CACHE_CONTROL, "no-store")], Json(body)), &pk, &c.session), Some(&step)));
    }
    let code = door.grants.mint_for(&c).map_err(|w| refused(StatusCode::BAD_REQUEST, route, w))?;
    let body = serde_json::json!({ "redirect": redirect_to(&c.callback, &code, &c.state) });
    Ok(by(([(header::CACHE_CONTROL, "no-store")], Json(body)), &pk, &c.session))
}

/// Percent-encoding for a query value.
fn url_escape(v: &str) -> String {
    v.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn take(door: &Door, attempt: &str, signup: bool, route: &str) -> Result<Attempt, Refusal> {
    let a = door.attempts.lock().unwrap().remove(attempt);
    match a {
        Some(a) if a.signup == signup && a.started.elapsed() < door.cfg.ttl(a.signup) => Ok(a),
        _ => Err(refused(StatusCode::UNAUTHORIZED, route, "no such attempt, or it has expired")),
    }
}

/// Sign-up, second half: the wrap sealed under the PRF output and stored, the head
/// written, the challenge signed, and the session open.
/// NC-44: everything that could refuse a public site's sign-in is checked BEFORE the
/// session process is asked to finish, so no refusal comes after an account exists
/// with its words unseen.
fn callback_ok(door: &Door, cb: &Option<Callback>, route: &str) -> Result<(), Refusal> {
    let Some(cb) = cb else { return Ok(()) };
    door.grants.callback(&cb.client, &cb.redirect_uri).map_err(|w| refused(StatusCode::BAD_REQUEST, route, w))?;
    if !token::s256(&cb.code_challenge) || cb.state.is_empty() {
        return Err(refused(StatusCode::BAD_REQUEST, route, "an S256 code_challenge and a state are required"));
    }
    Ok(())
}

async fn sign_up_finish(State(door): State<Door>, Json(i): Json<FinishIn>) -> Result<Response, Refusal> {
    let route = "/v2/signup/finish";
    // A refused client ends its attempt: no account exists yet, and the slot is freed.
    if let Err(e) = callback_ok(&door, &i.client, route) {
        door.attempts.lock().unwrap().remove(&i.attempt);
        return Err(e);
    }
    let a = take(&door, &i.attempt, true, route)?;
    let done = call(&door, &a.proc, reqwest::Method::POST, "/i/signup/finish", Some(serde_json::json!({
        "attempt": i.attempt, "sealed": i.sealed,
    })))
    .await
    .map_err(|(s, w)| refused(s, route, w))?;
    let pk = done["pk"].as_str().unwrap_or_default().to_string();
    // MADE, AND NOT KEPT (NC-89): the words go to the window once, marked unsaved, and no
    // session is seated on a process that is ending. The window's one next step is a
    // sign-in with the same passkey, which opens the account afresh.
    if done["saved"] == serde_json::Value::Bool(false) {
        drop(a);
        let body = serde_json::json!({ "pk": format!("ed25519:{pk}"), "words": done["words"], "saved": false, "why": done["why"] });
        return Ok(([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response());
    }
    challenge(&door, &a.proc, &pk, route).await?;
    // The words, now that the account they recover exists; once, never cached.
    let body = serde_json::json!({ "pk": format!("ed25519:{pk}"), "words": done["words"] });
    landed(&door, Some(a.proc), pk, i.client, body, route)
}

/// Sign-in, first half: a process and its key, before anyone is known.
async fn sign_in(
    State(door): State<Door>,
    axum::Extension(ClientTag(client)): axum::Extension<ClientTag>,
    h: HeaderMap,
    body: Option<Json<SignInIn>>,
) -> Result<Json<serde_json::Value>, Refusal> {
    let route = "/v2/signin";
    paid(&door, &h, pow::Start::SignIn, body.and_then(|Json(b)| b.work).as_ref(), route)?;
    let proc = start(&door, route, &client, None).await?;
    let key = call(&door, &proc, reqwest::Method::GET, "/i/key", None).await.map_err(|(s, w)| refused(s, route, w))?;
    let attempt = random_hex(16);
    door.attempts.lock().unwrap().insert(attempt.clone(), Attempt { proc, started: Instant::now(), signup: false, client, join: None });
    Ok(Json(serde_json::json!({ "attempt": attempt, "key": key["key"] })))
}

/// Sign-in, second half: the handle names the account, the sealed PRF opens its wrap in
/// the attempt's process, and the challenge is signed: the proof, and nothing resumed.
/// Then ONE PROCESS PER PERSON (D-34 (c)): a person live already gets one more session
/// of their process, and the attempt's goes; anyone else's process is opened, from its
/// seal where it has a good one, and becomes theirs.
async fn sign_in_finish(State(door): State<Door>, Json(i): Json<FinishIn>) -> Result<Response, Refusal> {
    let route = "/v2/signin/finish";
    // A refused client ends its attempt: no account exists yet, and the slot is freed.
    if let Err(e) = callback_ok(&door, &i.client, route) {
        door.attempts.lock().unwrap().remove(&i.attempt);
        return Err(e);
    }
    let a = take(&door, &i.attempt, false, route)?;
    use base64::Engine;
    let handle = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(&i.handle)
        .map_err(|_| refused(StatusCode::BAD_REQUEST, route, "the handle is not base64url"))?;
    if handle.len() <= 32 {
        return Err(refused(StatusCode::BAD_REQUEST, route, "this passkey carries no host in its handle"));
    }
    let pk = hex::encode(&handle[..32]);
    let host = String::from_utf8(handle[32..].to_vec())
        .map_err(|_| refused(StatusCode::BAD_REQUEST, route, "the handle's host is not UTF-8"))?;
    call(&door, &a.proc, reqwest::Method::POST, "/i/prove", Some(serde_json::json!({
        "attempt": i.attempt, "pk": pk, "host": host, "sealed": i.sealed,
    })))
    .await
    .map_err(|(s, w)| refused(if s == StatusCode::BAD_REQUEST { StatusCode::UNAUTHORIZED } else { s }, route, w))?;
    challenge(&door, &a.proc, &pk, route).await?;

    // In the person's turn: two sign-ins cannot both open their process, and none opens
    // it while their last session is sealing it (NC-63).
    let turn = turn_of(&door, &pk);
    let held = turn.lock().await;
    let (live, dead) = {
        let mut l = door.live.lock().unwrap();
        let dead = l.reap(&pk);
        (l.persons.contains_key(&pk), dead)
    };
    if dead.is_some() {
        eprintln!("door: {}…'s process had exited; it opens again", &pk[..pk.len().min(12)]);
    }
    drop(dead);
    let r = if live {
        join_live(&door, a, &i.attempt, &pk, i.client, route).await
    } else {
        open(&door, a, &i.attempt, &pk, i.client, route).await
    };
    drop(held);
    turn_done(&door, &pk, &turn);
    r
}

/// One more session of a live person's process; the attempt's process goes with `a`.
/// Should the person be gone by the time it is seated, the proof still stands, and
/// their process is opened from the attempt's (build-worker-c, (4)).
async fn join_live(door: &Door, a: Attempt, attempt: &str, pk: &str, cb: Option<Callback>, route: &str) -> Result<Response, Refusal> {
    let body = serde_json::json!({ "pk": format!("ed25519:{pk}"), "resume": null });
    match landed(door, None, pk.to_string(), cb.clone(), body, route) {
        Err(_) if !door.live.lock().unwrap().persons.contains_key(pk) => open(door, a, attempt, pk, cb, route).await,
        r => r,
    }
}

/// A person's process, opened from a proved attempt's, and their first session. A
/// failure that may pass keeps the attempt, within its life, for a retry.
async fn open(door: &Door, a: Attempt, attempt: &str, pk: &str, cb: Option<Callback>, route: &str) -> Result<Response, Refusal> {
    let again = |a: Attempt, why: String| {
        door.attempts.lock().unwrap().insert(attempt.to_string(), a);
        Err(refused(StatusCode::SERVICE_UNAVAILABLE, route, why))
    };
    if door.live.lock().unwrap().persons.len() >= door.cfg.max {
        return again(a, "the Door is at its session limit; try again shortly".into());
    }
    let done = match call(door, &a.proc, reqwest::Method::POST, "/i/open", Some(serde_json::json!({}))).await {
        Ok(done) => done,
        Err((StatusCode::SERVICE_UNAVAILABLE, why)) => return again(a, why),
        Err((s, why)) => return Err(refused(s, route, why)),
    };
    let body = serde_json::json!({ "pk": format!("ed25519:{pk}"), "resume": done["resume"] });
    landed(door, Some(a.proc), pk.to_string(), cb, body, route)
}

/// Sign-out: the session ends, its process with it, and the cookie is cleared.
async fn sign_out(State(door): State<Door>, h: HeaderMap) -> Result<Response, Refusal> {
    let route = "/v2/signout";
    if bearer(&h).is_none() && !origin_allowed(&door, &h) {
        return Err(refused(StatusCode::FORBIDDEN, route, "a write from an origin this Door does not list"));
    }
    let id = match bearer(&h) {
        // A token whose proof is refused signs nothing out, and is told so (NC-57):
        // answering 200 would leave a caller believing a live token was revoked.
        Some(t) => {
            door.grants
                .spend(&t, dpop(&h), "POST", &format!("{}{route}", door.cfg.public))
                .map_err(|w| refused(StatusCode::UNAUTHORIZED, route, w))?;
            door.grants.revoke(&t).map(|t| t.session)
        }
        None => session_id(&door, &h),
    };
    // That session alone ends (Software Security): the person's others go on. It ends in
    // the person's turn, so a sign-in of theirs waits for their last one's seal (NC-63).
    let pk = id.as_ref().and_then(|id| door.live.lock().unwrap().sessions.get(id).map(|s| s.pk.clone()));
    let who = id.clone().zip(pk.clone());
    if let (Some(id), Some(pk)) = (id, pk) {
        let turn = turn_of(&door, &pk);
        let held = turn.lock().await;
        let ended = door.live.lock().unwrap().end(&id);
        if let Some((_, e)) = ended {
            close(&door, &id, e).await;
        }
        drop(held);
        turn_done(&door, &pk, &turn);
    }
    let secure = if door.cfg.secure { "; Secure" } else { "" };
    let c = format!("{}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{secure}", door.cfg.cookie);
    let out = ([(header::SET_COOKIE, c)], Json(serde_json::json!({ "signed_out": true })));
    Ok(match who {
        Some((id, pk)) => by(out, &pk, &id),
        None => out.into_response(),
    })
}

// ─── the work a start pays (D-55) ────────────────────────────────────────────

/// The handle of the kiosk claim waiting for this browser: its cookie names a live
/// `/join` entry. Read, never spent; `/v2/join` spends it.
fn join_of(door: &Door, h: &HeaderMap) -> Option<String> {
    cookie(h, door.cfg.join_cookie).filter(|k| door.joins.lock().unwrap().get(k).is_some_and(|j| j.until > Instant::now()))
}

/// Whether a start owes the work: a sign-up does, unless a kiosk claim waits for it; a
/// sign-in does only while open attempts are over half the site-wide cap.
fn owed(door: &Door, h: &HeaderMap, signup: bool) -> bool {
    if signup {
        join_of(door, h).is_none()
    } else {
        door.attempts.lock().unwrap().len() * 2 > door.cfg.max_attempts
    }
}

/// A start's work: checked wherever it is presented, required where it is owed. Before
/// the caps, so a start that has not paid is counted for nothing.
fn paid(door: &Door, h: &HeaderMap, start: pow::Start, work: Option<&Work>, route: &str) -> Result<(), Refusal> {
    let why = match work {
        Some(w) => door.pow.check(&w.challenge, &w.nonce, now_secs(), start).err(),
        None if owed(door, h, start == pow::Start::SignUp) => Some("this start needs its proof of work".to_string()),
        None => None,
    };
    why.map_or(Ok(()), |w| Err(refused(StatusCode::FORBIDDEN, route, w)))
}

/// The challenge a start will pay, or none when nothing is owed, decided now from the
/// pool as it stands. Signed, not stored: issuing one holds nothing.
async fn work(
    State(door): State<Door>,
    h: HeaderMap,
    axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>,
) -> Result<Response, Refusal> {
    let start = match q.get("for").map(String::as_str) {
        Some("signup") => pow::Start::SignUp,
        Some("signin") => pow::Start::SignIn,
        _ => return Err(refused(StatusCode::BAD_REQUEST, "/v2/work", "for is signup or signin")),
    };
    let challenge = owed(&door, &h, start == pow::Start::SignUp).then(|| door.pow.issue(now_secs(), start));
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(serde_json::json!({ "challenge": challenge }))).into_response())
}

// ─── a kiosk's visitor (A-3, ICD-6) ──────────────────────────────────────────

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// THE KIOSK'S QR LANDS HERE. The claim is checked as the Arc's node checks it, bar the
/// spend and which Sites it holds, so a forged one is refused before anyone makes an
/// account. It is kept under a random handle for its life; the handle rides a Strict
/// cookie, and the 303 takes the claim out of the address bar. Never logged: the record
/// has the route and the reason, and the query is not the route.
async fn join(
    State(door): State<Door>,
    axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>,
) -> Result<Response, Refusal> {
    let route = "/join";
    let token = q.get("claim").map(String::as_str).unwrap_or_default();
    let site = pacific_core::claim::site_of(token).ok_or_else(|| refused(StatusCode::BAD_REQUEST, route, "not a claim"))?;
    let claim = pacific_core::claim::verify(token, &door.cfg.claim_keys, &site, now_secs())
        .map_err(|w| refused(StatusCode::BAD_REQUEST, route, w))?;
    if let Some(home) = door.grants.home_of_site(&site) {
        return Ok((
            StatusCode::SEE_OTHER,
            [
                (header::LOCATION, join_target(Some(&home), token)),
                (header::CACHE_CONTROL, "no-store".to_string()),
                (header::REFERRER_POLICY, "no-referrer".to_string()),
            ],
        )
            .into_response());
    }
    let life = pacific_core::claim::LIFE_MAX + pacific_core::claim::SKEW;
    let handle = random_hex(32);
    {
        let mut joins = door.joins.lock().unwrap();
        // A second scan of one QR replaces the first: one entry per claim.
        joins.retain(|_, j| j.token != token);
        joins.insert(handle.clone(), Join { token: token.to_string(), until: Instant::now() + Duration::from_secs(life), kept: kept_of(&claim) });
    }
    let secure = if door.cfg.secure { "; Secure" } else { "" };
    let c = format!("{}={handle}; Path=/; HttpOnly; SameSite=Strict; Max-Age={life}{secure}", door.cfg.join_cookie);
    Ok((
        StatusCode::SEE_OTHER,
        [(header::LOCATION, "/#join".to_string()), (header::SET_COOKIE, c), (header::CACHE_CONTROL, "no-store".to_string())],
    )
        .into_response())
}

/// WHERE A SCANNED CLAIM GOES (the user, 30 Sep: "exactly one app.wallflowers.io page, which
/// serves only the passkey … and immediately back"; Antoine: "It doesn't make sense to arrive
/// directly on an other identity"). The home of the client registered for the claim's Site,
/// with the claim in its fragment, which no request carries and no Referer names; the Site signs
/// in (the one page) and joins with its own session. A Site with no home: the webapp, as before.
fn join_target(home: Option<&str>, token: &str) -> String {
    match home {
        Some(h) => format!("{}#join={token}", h.split('#').next().unwrap_or(h)),
        None => "/#join".to_string(),
    }
}

/// A new member's name, as the one page gives it at the start of their account (O-77's card on
/// entry; the user, 30 Sep: the name on the one page), which the identity is made under
/// (`Node::init_identity`): trimmed, 1 to 64 characters, no control characters.
fn a_name(raw: &str) -> Result<String, &'static str> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("a name is needed");
    }
    if name.chars().count() > 64 {
        return Err("a name is at most 64 characters");
    }
    if name.chars().any(char::is_control) {
        return Err("a name has no control characters");
    }
    let chars: Vec<char> = name.chars().collect();
    let visible = |c: Option<&char>| c.is_some_and(|c| !c.is_whitespace() && !c.is_control() && !is_format(*c));
    for (i, c) in chars.iter().enumerate() {
        // The joiners (ZWNJ, ZWJ) spell emoji sequences and Persian and Indic words: between two
        // visible characters they stay (SCM, 30 Sep); anywhere else they only hide.
        let joins = matches!(c, '\u{200C}' | '\u{200D}') && visible(i.checked_sub(1).and_then(|j| chars.get(j))) && visible(chars.get(i + 1));
        if is_format(*c) && !joins {
            return Err("a name has no invisible formatting characters");
        }
    }
    Ok(name.to_string())
}

/// Unicode's format characters (General Category Cf): invisible, and able to reorder or hide
/// what a name says (ASSURANCE, 30 Sep). The Cf code points of Unicode 15.1.
fn is_format(c: char) -> bool {
    matches!(c as u32,
        0x00AD | 0x0600..=0x0605 | 0x061C | 0x06DD | 0x070F | 0x0890..=0x0891 | 0x08E2 | 0x180E
        | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F | 0xFEFF
        | 0xFFF9..=0xFFFB | 0x110BD | 0x110CD | 0x13430..=0x1343F | 0x1BCA0..=0x1BCA3
        | 0x1D173..=0x1D17A | 0xE0001 | 0xE0020..=0xE007F)
}

/// W-96, A CONTACT CODE: the webapp's own session asks for the key packages its person hands
/// a Site's owner, who adds them by it (/v2/add {object, bundle}), a Site and its rooms one
/// package each. A site's token gets none: a site adds no one, and a code is an add.
async fn contact_code(State(door): State<Door>, h: HeaderMap) -> Result<Response, Refusal> {
    let route = "/v2/bundle";
    if bearer(&h).is_some() {
        return Err(refused(StatusCode::FORBIDDEN, route, "a contact code is the webapp's"));
    }
    if !origin_allowed(&door, &h) {
        return Err(refused(StatusCode::FORBIDDEN, route, "a write from an origin this Door does not list"));
    }
    let Seated { port, secret, .. } = session_port(&door, &h, "POST", route)?;
    match call_at(&door, port, &secret, reqwest::Method::POST, "/i/code", None).await {
        Ok(v) => Ok(([(header::CACHE_CONTROL, "no-store")], Json(v)).into_response()),
        Err((status, why)) => Err(refused(status, route, why)),
    }
}

/// Fresh contact bundles a visitor's session makes for one claim: the Site's, and up to
/// five of its rooms' where the Arc's node admits (D-58). Each is its own key package,
/// under the one identity.
const JOIN_BUNDLES: usize = 6;

/// The Arc's node admits the session's identity by a claim: fresh contact bundles from the
/// person's process, and the claim. The node's status and answer, or why it could not be asked.
async fn admit(door: &Door, port: u16, secret: &str, token: &str) -> Result<(u16, serde_json::Value), String> {
    let mut bundles = Vec::with_capacity(JOIN_BUNDLES);
    for _ in 0..JOIN_BUNDLES {
        match call_at(door, port, secret, reqwest::Method::POST, "/i/bundle", None).await {
            Ok(b) => bundles.push(b["bundle"].as_str().unwrap_or_default().to_string()),
            Err((_, why)) => return Err(format!("the session made no contact bundle: {why}")),
        }
    }
    let r = door
        .http
        .post(format!("{}/v1/admit", door.cfg.arc))
        .timeout(Duration::from_secs(20))
        .json(&serde_json::json!({ "claim": token, "bundles": bundles }))
        .send()
        .await
        .map_err(|e| format!("the Arc could not be reached: {e}"))?;
    let status = r.status().as_u16();
    Ok((status, r.json().await.unwrap_or_default()))
}

/// What an admitted join answers: the Site, the node's rooms, and the home of the client
/// registered for the claim's Site (J-A), as registered: no token and no part of the claim.
fn admitted_body(door: &Door, token: &str, answer: &serde_json::Value) -> serde_json::Value {
    let site = pacific_core::claim::site_of(token);
    let listed = |k: &str| answer.get(k).cloned().unwrap_or_else(|| serde_json::json!([]));
    let home = site.as_deref().and_then(|s| door.grants.home_of_site(s));
    serde_json::json!({
        "admitted": true, "site": site, "c": answer["c"], "a": answer["a"], "rooms": listed("rooms"), "unjoined": listed("unjoined"),
        "fallback": answer["fallback"],
        "home": home,
    })
}

#[derive(Deserialize, Default)]
struct JoinIn {
    #[serde(default)]
    claim: String,
}

/// A SITE'S OWN JOIN (the user, 30 Sep: one app.wallflowers.io page, then back to the Site).
/// Signed in at the Site through that one page, the Site's session presents the claim the Site
/// was given, in the body, never a URL, and the Arc's node admits the member. A claim only for
/// the token's own Site. A retry presents the same claim: the node finds it spent by this same
/// member and spends nothing (D-58).
async fn join_by_site(door: &Door, h: &HeaderMap, token: &str, route: &str) -> Result<Response, Refusal> {
    let Seated { id, pk, port, secret, scope } = session_port(door, h, "POST", route)?;
    let site = pacific_core::claim::site_of(token).ok_or_else(|| refused(StatusCode::BAD_REQUEST, route, "not a claim"))?;
    let claim = pacific_core::claim::verify(token, &door.cfg.claim_keys, &site, now_secs())
        .map_err(|w| refused(StatusCode::BAD_REQUEST, route, w))?;
    if scope.as_deref() != Some(site.as_str()) {
        return Err(refused(StatusCode::FORBIDDEN, route, "a claim for another Site"));
    }
    let (status, answer) = admit(door, port, &secret, token).await.map_err(|why| refused(StatusCode::SERVICE_UNAVAILABLE, route, why))?;
    match status {
        200 => {
            let mut r = by(([(header::CACHE_CONTROL, "no-store")], Json(admitted_body(door, token, &answer))).into_response(), &pk, &id);
            r.extensions_mut().insert(kept_of(&claim));
            Ok(r)
        }
        409 => {
            let why = answer["error"].as_str().unwrap_or("the Arc refused the claim").to_string();
            Ok(by(refused(StatusCode::CONFLICT, route, why).into_response(), &pk, &id))
        }
        s => Err(refused(StatusCode::SERVICE_UNAVAILABLE, route, format!("the Arc answered {s}; try again"))),
    }
}

/// The visitor is signed in: their session's contact bundles and the claim go to the Arc's
/// node, which spends the claim and adds exactly that identity to the Site and its rooms.
/// Only the node's definite answer ends the claim; a failure to reach it, or a room it
/// could not add, keeps the claim for a retry, so one network blip does not cost a
/// visitor their only QR.
async fn join_finish(State(door): State<Door>, h: HeaderMap, body: Option<Json<JoinIn>>) -> Result<Response, Refusal> {
    let route = "/v2/join";
    if bearer(&h).is_some() {
        let claim = body.map(|Json(b)| b.claim).unwrap_or_default();
        return join_by_site(&door, &h, &claim, route).await;
    }
    if !origin_allowed(&door, &h) {
        return Err(refused(StatusCode::FORBIDDEN, route, "a write from an origin this Door does not list"));
    }
    let Seated { id, pk, port, secret, .. } = session_port(&door, &h, "POST", route)?;
    let handle = cookie(&h, door.cfg.join_cookie).ok_or_else(|| refused(StatusCode::NOT_FOUND, route, "no join is waiting"))?;
    // Out of the table while it is presented: a second press finds nothing to send.
    let join = door
        .joins
        .lock()
        .unwrap()
        .remove(&handle)
        .filter(|j| j.until > Instant::now())
        .ok_or_else(|| refused(StatusCode::NOT_FOUND, route, "no join is waiting, or it has expired"))?;
    let keep = |why: String| {
        door.joins.lock().unwrap().insert(handle.clone(), Join { token: join.token.clone(), until: join.until, kept: join.kept.clone() });
        refused(StatusCode::SERVICE_UNAVAILABLE, route, why)
    };
    let (status, answer) = match admit(&door, port, &secret, &join.token).await {
        Ok(a) => a,
        Err(why) => return Err(keep(why)),
    };
    let secure = if door.cfg.secure { "; Secure" } else { "" };
    let gone = format!("{}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{secure}", door.cfg.join_cookie);
    match status {
        200 => {
            // J-A: where the joined visitor goes next rides the body (admitted_body).
            let body = admitted_body(&door, &join.token, &answer);
            let unjoined = body["unjoined"].clone();
            // Admitted: the row keeps what the Door verified of the claim (tw::Kept).
            let admitted = |r: Response| {
                let mut r = by(r, &pk, &id);
                r.extensions_mut().insert(join.kept.clone());
                r
            };
            // A room not added: the claim and its cookie are kept, so a retry adds it; the
            // node finds the claim spent by this same member and spends nothing (D-58).
            if unjoined.as_array().is_some_and(|u| !u.is_empty()) {
                door.joins.lock().unwrap().insert(handle.clone(), Join { token: join.token.clone(), until: join.until, kept: join.kept.clone() });
                return Ok(admitted(([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()));
            }
            Ok(admitted(([(header::SET_COOKIE, gone), (header::CACHE_CONTROL, "no-store".to_string())], Json(body)).into_response()))
        }
        // The node's definite refusal: the claim is done with, and the visitor is told why.
        409 => {
            let why = answer["error"].as_str().unwrap_or("the Arc refused the claim").to_string();
            let mut r = refused(StatusCode::CONFLICT, route, why).into_response();
            r.headers_mut().insert(header::SET_COOKIE, HeaderValue::from_str(&gone).expect("cookie"));
            Ok(by(r, &pk, &id))
        }
        s => Err(keep(format!("the Arc answered {s}; try again"))),
    }
}

// ─── the session's routes ────────────────────────────────────────────────────

/// The session this request's cookie names, touched. Everything under /v2 but the
/// model and the sign-in routes needs one (SEC-5).
/// A public site's token rides `Authorization: DPoP`, with a proof for this method
/// and URL (SEC-39); the webapp's session rides its cookie. Answers the session's id,
/// its person's process, and the Site a token's session is held to. The id reaches the
/// process only as `x-door-ref`, on a request this Door builds (D-34 (c)).
fn session_port(door: &Door, h: &HeaderMap, method: &str, route: &str) -> Result<Seated, Refusal> {
    let id = match bearer(h) {
        Some(t) => door
            .grants
            .spend(&t, dpop(h), method, &format!("{}{route}", door.cfg.public))
            .map_err(|w| refused(StatusCode::UNAUTHORIZED, route, w))?
            .session,
        None => session_id(door, h).ok_or_else(|| refused(StatusCode::UNAUTHORIZED, route, "not signed in"))?,
    };
    let mut live = door.live.lock().unwrap();
    let s = live.sessions.get_mut(&id).ok_or_else(|| refused(StatusCode::UNAUTHORIZED, route, "no such session"))?;
    s.seen = Instant::now();
    let (pk, scope) = (s.pk.clone(), s.scope.clone());
    let p = live.persons.get(&pk).ok_or_else(|| refused(StatusCode::UNAUTHORIZED, route, "no such session"))?;
    Ok(Seated { id, pk, port: p.proc.port, secret: p.proc.secret.clone(), scope })
}

/// An answer, marked with whose action it was, for training_wheels' row (tw::capture).
fn by(r: impl IntoResponse, pk: &str, session: &str) -> Response {
    let mut r = r.into_response();
    r.extensions_mut().insert(tw::Who { account: pk.to_string(), session: session.to_string() });
    r
}

fn bearer(h: &HeaderMap) -> Option<String> {
    h.get(header::AUTHORIZATION)?.to_str().ok()?.strip_prefix("DPoP ").map(str::to_string)
}

fn dpop(h: &HeaderMap) -> &str {
    h.get("dpop").and_then(|v| v.to_str().ok()).unwrap_or("")
}

#[derive(Deserialize)]
struct TokenIn {
    code: String,
    code_verifier: String,
    client: String,
    redirect_uri: String,
}

/// The code and its verifier for the session token, bound to the proof's key
/// (SEC-39). In the body only, never cached, never logged.
async fn token_exchange(
    State(door): State<Door>,
    axum::Extension(ClientTag(who)): axum::Extension<ClientTag>,
    h: HeaderMap,
    Json(i): Json<TokenIn>,
) -> Result<Response, Refusal> {
    let route = "/v2/token";
    let jkt = door
        .grants
        .proof(dpop(&h), "POST", &format!("{}{route}", door.cfg.public), None, &who)
        .map_err(|w| refused(StatusCode::BAD_REQUEST, route, w))?;
    let (token, t) = door
        .grants
        .redeem(&i.code, &i.code_verifier, &i.client, &i.redirect_uri, jkt)
        .map_err(|w| refused(StatusCode::BAD_REQUEST, route, w))?;
    // A token names a live session or none at all (Software Testing, E15).
    let pk = match door.live.lock().unwrap().sessions.get_mut(&t.session) {
        Some(s) => {
            s.seen = Instant::now();
            s.pk.clone()
        }
        None => {
            door.grants.revoke(&token);
            return Err(refused(StatusCode::UNAUTHORIZED, route, "the code's session has ended; sign in again"));
        }
    };
    // A WILD CLIENT'S CHOICE, HELD AGAIN AT ISSUE (R3.1 (C)): the person may no longer be the
    // chosen Site's owner or admin.
    if door.grants.client(&i.client).is_some_and(|c| c.site == token::WILD) {
        match eligible_sites(&door, &t.session, route).await {
            Ok((_, sites)) if sites.iter().any(|(id, _)| *id == t.site) => {}
            other => {
                door.grants.revoke(&token);
                return Err(other.err().unwrap_or_else(|| refused(StatusCode::FORBIDDEN, route, "you are neither the owner nor an admin of that Site")));
            }
        }
    }
    let out = serde_json::json!({
        "access_token": token,
        "token_type": "DPoP",
        "expires_in": door.grants.token_life.as_secs(),
        "scope": t.site,
    });
    Ok(by(([(header::CACHE_CONTROL, "no-store")], Json(out)), &pk, &t.session))
}

/// A SITE'S PUBLIC ITEMS, FOR ITS MEMBERS (W-98): the events and posts a Site made, which its
/// members do not hold, as its Face serves them on the Arc, relayed from this origin (the
/// webapp's connect-src is 'self'). The webapp's session only, and a member of the Site only
/// (their Node holds it, and they are on its roster). Its address is the Host's publication their
/// Node holds, or the one this Door's client registry names, never the request's. The answer is
/// the Arc's items, unchanged, and nothing this Door or the Node holds.
async fn site_items(State(door): State<Door>, axum::extract::Path(site): axum::extract::Path<String>, h: HeaderMap) -> Result<Response, Refusal> {
    let route = "/v2/site/:site/items";
    if bearer(&h).is_some() {
        return Err(refused(StatusCode::FORBIDDEN, route, "a Site's item list is the webapp's"));
    }
    let Seated { id, pk, port, secret, .. } = session_port(&door, &h, "GET", route)?;
    let of = call_at(&door, port, &secret, reqwest::Method::GET, &format!("/i/site/{site}"), None).await.map_err(|(s, w)| refused(s, route, w))?;
    if of["member"] != true {
        return Err(refused(StatusCode::FORBIDDEN, route, "not a member of that Site"));
    }
    let slug = of["slug"].as_str().map(str::to_string).or_else(|| door.grants.slug_of_site(&site)).filter(|s| face::valid_slug(s));
    let Some(slug) = slug else { return Err(no_face_of(route)) };
    let r = door
        .http
        .get(format!("{}/v1/face/{slug}/items", door.cfg.arc))
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .map_err(|e| refused(StatusCode::BAD_GATEWAY, route, e.to_string()))?;
    if r.status() == StatusCode::NOT_FOUND {
        return Err(no_face_of(route));
    }
    if !r.status().is_success() {
        return Err(refused(StatusCode::BAD_GATEWAY, route, format!("the Arc answered {}", r.status())));
    }
    let face: serde_json::Value = r.json().await.map_err(|e| refused(StatusCode::BAD_GATEWAY, route, e.to_string()))?;
    let answer = ([(header::CACHE_CONTROL, "no-store")], Json(serde_json::json!({ "slug": slug, "items": face["items"] })));
    Ok(by(answer, &pk, &id))
}

fn no_face_of(route: &str) -> Refusal {
    refused(StatusCode::NOT_FOUND, route, "that Site has no published Face")
}

async fn fwd(State(door): State<Door>, req: Request) -> Result<Response, Refusal> {
    let route = req.uri().path().to_string();
    let method = req.method().clone();
    let Seated { id, pk, port, secret, scope } = session_port(&door, req.headers(), method.as_str(), &route)?;
    let who = (pk, id.clone());
    // The Origin check guards the cookie, which a browser attaches by itself; a
    // token is never ambient, and its proof binds the request (SEC-39).
    if method == Method::POST && bearer(req.headers()).is_none() && !origin_allowed(&door, req.headers()) {
        return Err(refused(StatusCode::FORBIDDEN, &route, "a write from an origin this Door does not list"));
    }
    let body = axum::body::to_bytes(req.into_body(), 1 << 20)
        .await
        .map_err(|_| refused(StatusCode::PAYLOAD_TOO_LARGE, &route, "the body is over 1 MiB"))?;
    let mut rq = door
        .http
        .request(reqwest::Method::from_bytes(method.as_str().as_bytes()).expect("method"), format!("http://127.0.0.1:{port}{route}"))
        .bearer_auth(secret)
        .header("content-type", "application/json")
        .header("x-door-ref", id);
    if let Some(site) = scope {
        rq = rq.header("x-door-scope", site);
    }
    let r = rq
        .body(body)
        .send()
        .await
        .map_err(|e| refused(StatusCode::BAD_GATEWAY, &route, e.to_string()))?;
    let status = StatusCode::from_u16(r.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let ctype = r.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("text/plain").to_string();
    let bytes = r.bytes().await.map_err(|e| refused(StatusCode::BAD_GATEWAY, &route, e.to_string()))?;
    let answer = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, ctype)
        .header(header::CACHE_CONTROL, "no-store")
        .body(axum::body::Body::from(bytes))
        .expect("response");
    Ok(by(answer, &who.0, &who.1))
}

/// The change stream, forwarded without buffering: it never ends.
async fn fwd_stream(State(door): State<Door>, h: HeaderMap) -> Result<Response, Refusal> {
    let Seated { id, port, secret, scope, .. } = session_port(&door, &h, "GET", "/v2/events")?;
    let mut rq = door.http.get(format!("http://127.0.0.1:{port}/v2/events")).bearer_auth(secret).header("x-door-ref", id);
    if let Some(site) = scope {
        rq = rq.header("x-door-scope", site);
    }
    let r = rq
        .timeout(Duration::from_secs(24 * 3600))
        .send()
        .await
        .map_err(|e| refused(StatusCode::BAD_GATEWAY, "/v2/events", e.to_string()))?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-store")
        .body(axum::body::Body::from_stream(r.bytes_stream()))
        .expect("response"))
}

#[cfg(test)]
mod tests {
    /// A scanned claim goes to its Site's home, the claim in the fragment only (the user, 30 Sep).
    #[test]
    fn a_scanned_claim_goes_to_its_sites_home_in_the_fragment() {
        let t = "v1.eyJrIjoiYSJ9.c2ln";
        assert_eq!(super::join_target(Some("https://egregores-echoes.com/community"), t), format!("https://egregores-echoes.com/community#join={t}"));
        assert_eq!(
            super::join_target(Some("https://egregores-echoes.com/community#old"), t),
            format!("https://egregores-echoes.com/community#join={t}"),
            "a registered fragment gives way to the claim"
        );
        assert_eq!(super::join_target(None, t), "/#join", "no home: the webapp, as before");
        assert!(!super::join_target(Some("https://egregores-echoes.com/community"), t).contains('?'), "never in a query");
    }

    /// A new member's name from the one page: trimmed, 1 to 64 characters, no control characters.
    #[test]
    fn a_new_members_name_is_trimmed_and_held_to_its_limits() {
        assert_eq!(super::a_name("  Hana  "), Ok("Hana".to_string()));
        assert_eq!(super::a_name("이하나"), Ok("이하나".to_string()));
        assert_eq!(super::a_name(&"é".repeat(64)), Ok("é".repeat(64)), "64 characters, not bytes");
        for bad in ["", "   ", "a\u{0}b", "line\nbreak", "tab\there"] {
            assert!(super::a_name(bad).is_err(), "{bad:?}");
        }
        assert!(super::a_name(&"x".repeat(65)).is_err());
        // format characters (Unicode Cf): invisible, and able to reorder or hide what a name
        // says (ASSURANCE, 30 Sep): bidi overrides and isolates, zero-width, BOM, soft hyphen, tags
        for bad in ["Ha\u{202E}na", "Ha\u{2066}na\u{2069}", "Ha\u{200B}na", "\u{FEFF}Hana", "Ha\u{00AD}na", "Hana\u{E0041}"] {
            assert!(super::a_name(bad).is_err(), "{bad:?}");
        }
        assert_eq!(super::a_name("Anne-Marie O'Neil"), Ok("Anne-Marie O'Neil".to_string()), "ordinary punctuation stays");
        // THE JOINERS (U+200C, U+200D; SCM, 30 Sep): an emoji sequence and Persian and Indic
        // spellings need them, so between two visible characters they stay; at an end, doubled,
        // or beside a space, they only hide, and are refused
        for good in ["\u{1F469}\u{200D}\u{1F4BB}", "\u{1F3F3}\u{FE0F}\u{200D}\u{1F308}", "\u{062E}\u{0648}\u{0627}\u{0647}\u{0631}\u{200C}\u{0632}\u{0627}\u{062F}\u{0647}", "\u{0915}\u{094D}\u{200D}\u{0937}"] {
            assert_eq!(super::a_name(good), Ok(good.to_string()), "{good:?}");
        }
        for bad in ["\u{200D}Hana", "Hana\u{200C}", "Ha\u{200D}\u{200D}na", "Ha \u{200C}na", "Ha\u{200D} na"] {
            assert!(super::a_name(bad).is_err(), "{bad:?}");
        }
    }

    /// A pinned name is a name, a dot, and sixteen lowercase hex before `.js` or `.css`;
    /// nothing else is cached for good.
    #[test]
    fn a_pinned_name_is_a_name_and_sixteen_hex() {
        for p in ["/webapp.7f05fd57f625e3aa.js", "/webapp.6d01ca2b0ea6e094.css", "/assets/face/vendor/fonts.527f63e43f86a403.css"] {
            assert!(super::is_pinned(p), "{p}");
        }
        for p in [
            "/webapp.js",
            "/webapp.7F05FD57F625E3AA.js",
            "/webapp.7f05fd57f625e3a.js",
            "/.7f05fd57f625e3aa.js",
            "/webapp.7f05fd57f625e3aa.mjs",
            "/webapp.7f05fd57f625e3aa.js/",
            "/v2/signin.js",
            "/door/seal.js",
            "/",
        ] {
            assert!(!super::is_pinned(p), "{p}");
        }
    }

    /// The sweeper is a plain thread: ending a session there reaches the session process
    /// (its head stored first), and does not panic for want of a runtime (door-test, 27 Sep).
    #[test]
    fn a_session_ends_from_a_thread_with_no_runtime() {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let got = std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            let mut buf = [0u8; 2048];
            let n = c.read(&mut buf).unwrap();
            c.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n").unwrap();
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        std::thread::spawn(move || super::end_at(port, "s3cret")).join().expect("ending panicked");
        let req = got.join().unwrap();
        assert!(req.starts_with("POST /i/end "), "{req}");
        assert!(req.to_ascii_lowercase().contains("authorization: bearer s3cret"), "{req}");
    }

    /// TBD-12: the refusal record keeps thirty days, and drops what it cannot date.
    #[test]
    fn the_refusal_record_keeps_thirty_days() {
        let now = 2_000_000_000;
        let day = 24 * 60 * 60;
        let record = format!(
            "{{\"t\":{},\"route\":\"/old\"}}\n{{\"t\":{},\"route\":\"/recent\"}}\nnot a line\n{{\"route\":\"/undated\"}}\n",
            now - 31 * day,
            now - 29 * day
        );
        let kept = super::keep_recent(&record, now, super::REFUSALS_KEPT);
        assert_eq!(kept, format!("{{\"t\":{},\"route\":\"/recent\"}}\n", now - 29 * day));
    }

    // ── A-3: a kiosk's visitor ────────────────────────────────────────────────

    use axum::extract::{Query, State};
    use axum::http::{header, HeaderMap, StatusCode};
    use base64::Engine;
    use ed25519_dalek::Signer;
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex};

    const SITE: &str = "5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e";

    fn kiosk() -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&[9u8; 32])
    }

    fn keys() -> HashMap<String, [u8; 32]> {
        let pk = kiosk().verifying_key().to_bytes();
        HashMap::from([(pacific_core::claim::kid_of(&pk), pk)])
    }

    /// A claim as the kiosk issues one, for `SITE`, with nonce `n`.
    fn claim(n: u8) -> String {
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let now = super::now_secs();
        let kid = pacific_core::claim::kid_of(&kiosk().verifying_key().to_bytes());
        let payload = format!(
            r#"{{"k":"{kid}","s":"{SITE}","n":"{}","iat":{now},"exp":{},"c":"skills","a":"k2m3n4p5q6r7s8t9uvwx"}}"#,
            b64.encode([n; 16]),
            now + 600
        );
        let p = b64.encode(payload);
        format!("v1.{p}.{}", b64.encode(kiosk().sign(format!("v1.{p}").as_bytes()).to_bytes()))
    }

    fn door_for(arc: String) -> super::Door {
        let dir = std::env::temp_dir().join(format!("door-join-{}", super::random_hex(8)));
        std::fs::create_dir_all(&dir).unwrap();
        let refusals_path = dir.join("refusals.jsonl");
        let refusals = std::fs::OpenOptions::new().create(true).append(true).open(&refusals_path).unwrap();
        let s = std::time::Duration::from_secs;
        super::Door {
            attempts: Default::default(),
            live: Default::default(),
            opening: Default::default(),
            cfg: Arc::new(super::Cfg {
                fold_cache_mib: 8,
                code_secs: 1800,
                seals: dir.join("seals"),
                root: dir,
                relay: String::new(),
                arc,
                claim_keys: keys(),
                join_cookie: "door-join",
                auth: String::new(),
                auth_host: String::new(),
                public: "http://127.0.0.1:8233".into(),
                audience: "127.0.0.1:8233".into(),
                origins: vec![],
                cookie: "door",
                secure: false,
                idle: s(900),
                attempt_ttl: s(60),
                signup_ttl: s(300),
                max_attempts: 8,
                joining: 2,
                max: 8,
                sync_secs: 3,
                rp_id: "localhost".into(),
                per_addr: 4,
                trust_forwarded: false,
            }),
            http: reqwest::Client::new(),
            grants: Arc::new(crate::token::Grants::load(None, s(3600))),
            refusals: Arc::new(Mutex::new(refusals)),
            refusals_path: Arc::new(refusals_path),
            tw: None,
            log_key: Arc::new([0; 32]),
            faces: Arc::new(super::face::Kept::default()),
            joins: Default::default(),
            pow: Arc::new(crate::pow::Pow::new()),
        }
    }

    /// Serve `app` on a loopback port.
    async fn serve(app: axum::Router) -> u16 {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        port
    }

    /// A SITE'S SIGN-IN IS ONE STEP IN ITS FACE (Ralph, 29 Sep): the Face's name and mark, its
    /// look from this origin, the WallFlowers mark at the foot and no wordmark. Without a Face
    /// (no slug, or the Arc down), the name the Site registered; without a client, the Door's
    /// own window.
    #[tokio::test]
    async fn a_sites_sign_in_wears_its_face() {
        use axum::extract::Query;
        const STYLE: &str = "--bg:#101820;--fg:#F2AA4C;--accent:#F2AA4C;--on-accent:#101820";
        let app = axum::Router::new()
            .route("/v1/face/egregore/brand", axum::routing::get(|| async {
                axum::Json(serde_json::json!({ "name": "Egregore's Echoes", "style": STYLE, "mark": true }))
            }))
            .route("/v1/face/egregore/m/mark", axum::routing::get(|| async { ([(header::CONTENT_TYPE, "image/png")], vec![0x89u8, b'P', b'N', b'G']) }));
        let port = serve(app).await;
        let with_clients = |arc: String| {
            let mut door = door_for(arc);
            let path = door.cfg.root.join("clients.json");
            std::fs::write(&path, serde_json::json!({
                "egregores-echoes.com": { "name": "Egregore", "callbacks": ["https://egregores-echoes.com/signin/callback"],
                                          "origins": ["https://egregores-echoes.com"], "site": SITE, "slug": "egregore" },
                "plain.example": { "name": "Plain <Site>", "callbacks": ["https://plain.example/cb"], "origins": ["https://plain.example"], "site": SITE },
            }).to_string()).unwrap();
            door.grants = Arc::new(crate::token::Grants::load(Some(path.to_string_lossy().into()), std::time::Duration::from_secs(3600)));
            door
        };
        let q = |client: &str, cb: &str| -> HashMap<String, String> {
            [("client", client), ("redirect_uri", cb), ("code_challenge", "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"),
             ("code_challenge_method", "S256"), ("state", "s1")]
                .into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
        };
        async fn body(r: axum::response::Response) -> (StatusCode, axum::http::HeaderMap, String) {
            let (st, h) = (r.status(), r.headers().clone());
            (st, h, String::from_utf8_lossy(&axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap()).into_owned())
        }
        let door = with_clients(format!("http://127.0.0.1:{port}"));

        let (st, h, page) = body(super::signin(State(door.clone()), Query(q("egregores-echoes.com", "https://egregores-echoes.com/signin/callback"))).await).await;
        assert_eq!(st, StatusCode::OK);
        for want in [
            r#"<html lang="en" data-site data-face>"#,
            "<title>Egregore&#39;s Echoes</title>",
            r#"<img class="site-mark" src="/door/face-mark?client=egregores-echoes.com" alt="">"#,
            r#"<h1 class="site-name">Egregore&#39;s Echoes</h1>"#,
            r#"<link rel="stylesheet" href="/door/face.css?client=egregores-echoes.com">"#,
        ] {
            assert!(page.contains(want), "{want} in the Site's window");
        }
        // Only the Site on its window (Ralph, 30 Sep: "make passkey (*1 page*, no wallflowers branding)").
        assert!(!page.contains("wordmark.svg") && !page.contains("wallflowers.png") && !page.contains("footer"), "no WallFlowers mark");
        assert!(!page.contains("__"), "every placeholder filled");
        let csp = h[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
        assert!(csp.contains("style-src 'self';") && csp.contains("img-src 'self' data:;") && csp.contains("font-src 'self';"), "{csp}");

        let (st, h, css) = body(super::face_css(State(door.clone()), Query(q("egregores-echoes.com", ""))).await).await;
        assert_eq!((st, h[header::CONTENT_TYPE].to_str().unwrap()), (StatusCode::OK, "text/css"));
        assert_eq!(css, format!("html[data-face]{{{STYLE}}}\n"));
        let r = super::face_mark(State(door.clone()), Query(q("egregores-echoes.com", ""))).await;
        assert_eq!((r.status(), r.headers()[header::CONTENT_TYPE].to_str().unwrap()), (StatusCode::OK, "image/png"));

        // No slug: the Site's registered name, escaped, and no look to ask for.
        let (_, _, page) = body(super::signin(State(door.clone()), Query(q("plain.example", "https://plain.example/cb"))).await).await;
        assert!(page.contains(r#"<html lang="en" data-site>"#) && page.contains(r#"<h1 class="site-name">Plain &lt;Site&gt;</h1>"#), "{page}");
        assert!(!page.contains("face.css") && !page.contains("wordmark.svg") && !page.contains("wallflowers.png"));
        assert_eq!(super::face_css(State(door.clone()), Query(q("plain.example", ""))).await.status(), StatusCode::NOT_FOUND);

        // No client: the Door's own window, its wordmark and no foot.
        let (_, _, page) = body(super::signin(State(door.clone()), Query(HashMap::new())).await).await;
        assert!(page.contains(r#"<html lang="en">"#) && page.contains("wordmark.svg") && !page.contains("footer"), "{page}");

        // The Arc down: the Site's registered name, and the window still opens.
        let down = with_clients("http://127.0.0.1:1".into());
        let (st, _, page) = body(super::signin(State(down.clone()), Query(q("egregores-echoes.com", "https://egregores-echoes.com/signin/callback"))).await).await;
        assert_eq!(st, StatusCode::OK);
        assert!(page.contains(r#"<html lang="en" data-site>"#) && page.contains(r#"<h1 class="site-name">Egregore</h1>"#), "{page}");
    }

    /// A read that failed is asked again in seconds; the Arc's word that a Site has no Face
    /// is kept the minute (MANAGE, 29 Sep: production's first window after a restart went
    /// without its Face for a minute).
    #[tokio::test]
    async fn a_failed_face_read_is_asked_again_in_seconds() {
        use axum::extract::Query;
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        static ERRED: AtomicUsize = AtomicUsize::new(0);
        static NONE: AtomicUsize = AtomicUsize::new(0);
        let face = || axum::Json(serde_json::json!({ "name": "E", "style": "--bg:#101820", "mark": false })).into_response();
        let app = axum::Router::new()
            .route("/v1/face/egregore/brand", axum::routing::get(move || async move {
                if ERRED.fetch_add(1, SeqCst) == 0 { StatusCode::BAD_GATEWAY.into_response() } else { face() }
            }))
            .route("/v1/face/gone/brand", axum::routing::get(move || async move {
                if NONE.fetch_add(1, SeqCst) == 0 { StatusCode::NOT_FOUND.into_response() } else { face() }
            }));
        let port = serve(app).await;
        let mut door = door_for(format!("http://127.0.0.1:{port}"));
        let path = door.cfg.root.join("clients.json");
        std::fs::write(&path, serde_json::json!({
            "egregores-echoes.com": { "name": "E", "callbacks": ["https://egregores-echoes.com/cb"], "origins": ["https://egregores-echoes.com"], "site": SITE, "slug": "egregore" },
            "gone.example": { "name": "G", "callbacks": ["https://gone.example/cb"], "origins": ["https://gone.example"], "site": SITE, "slug": "gone" },
        }).to_string()).unwrap();
        door.grants = Arc::new(crate::token::Grants::load(Some(path.to_string_lossy().into()), std::time::Duration::from_secs(3600)));
        let worn = |client: &'static str| {
            let door = door.clone();
            async move {
                let q: HashMap<String, String> = [("client", client)].into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
                super::face_css(State(door), Query(q)).await.status() == StatusCode::OK
            }
        };
        assert!(!worn("egregores-echoes.com").await && !worn("gone.example").await, "neither, at first");
        tokio::time::sleep(super::face::KEPT_FAILED + std::time::Duration::from_millis(500)).await;
        assert!(worn("egregores-echoes.com").await, "a failed read is asked again after seconds");
        assert!(!worn("gone.example").await, "no Face is kept the minute");
        assert_eq!((ERRED.load(SeqCst), NONE.load(SeqCst)), (2, 1));
    }

    /// A signed-in session whose process answers `/i/bundle` with "B", under the cookie "s1".
    async fn signed_in(door: &super::Door) {
        let port = serve(axum::Router::new().route(
            "/i/bundle",
            axum::routing::post(|| async { axum::Json(serde_json::json!({ "bundle": "B" })) }),
        ))
        .await;
        let (child, leash) = cat();
        let dir = door.cfg.root.join("s1");
        std::fs::create_dir_all(&dir).unwrap();
        let proc = super::Proc { child, _leash: leash, port, secret: "x".into(), dir };
        let mut live = door.live.lock().unwrap();
        live.persons.insert("ab".repeat(32), super::Person { proc, sessions: HashSet::from(["s1".to_string()]) });
        live.sessions.insert("s1".into(), super::Session { pk: "ab".repeat(32), seen: std::time::Instant::now(), scope: None, token: false });
    }

    /// An Arc whose `/v1/admit` answers `status` and `body`, and remembers what it was sent.
    async fn arc(status: u16, body: serde_json::Value) -> (String, Arc<Mutex<Vec<serde_json::Value>>>) {
        let got: Arc<Mutex<Vec<serde_json::Value>>> = Default::default();
        let seen = got.clone();
        let port = serve(axum::Router::new().route(
            "/v1/admit",
            axum::routing::post(move |axum::Json(v): axum::Json<serde_json::Value>| {
                seen.lock().unwrap().push(v);
                let body = body.clone();
                async move { (StatusCode::from_u16(status).unwrap(), axum::Json(body)) }
            }),
        ))
        .await;
        (format!("http://127.0.0.1:{port}"), got)
    }

    /// The QR's landing: a claim that checks out is kept under a Strict cookie and the
    /// address is cleared; a forged one is refused and keeps nothing.
    #[tokio::test]
    async fn a_kiosk_claim_is_checked_at_the_door_and_kept_under_a_strict_cookie() {
        let door = door_for(String::new());
        let q = |t: String| Query(HashMap::from([("claim".to_string(), t)]));

        let good = claim(1);
        let r = super::join(State(door.clone()), q(good.clone())).await.ok().expect("a valid claim lands");
        assert_eq!(r.status(), StatusCode::SEE_OTHER);
        assert_eq!(r.headers()[header::LOCATION], "/#join", "the claim leaves the address bar");
        let c = r.headers()[header::SET_COOKIE].to_str().unwrap().to_string();
        let handle = c.split(';').next().unwrap().strip_prefix("door-join=").expect("the join cookie").to_string();
        for want in ["HttpOnly", "SameSite=Strict", "Path=/", "Max-Age=1200"] {
            assert!(c.contains(want), "{want} in {c}");
        }
        assert!(!c.contains(&good), "the cookie carries a handle, not the claim");
        assert_eq!(door.joins.lock().unwrap()[&handle].token, good);

        // A second scan of the same QR replaces the first: one entry per claim.
        super::join(State(door.clone()), q(good.clone())).await.ok().unwrap();
        assert_eq!(door.joins.lock().unwrap().len(), 1);

        // Forged: the right shape, another key's signature.
        let (head, _) = good.rsplit_once('.').unwrap();
        let other = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]).sign(head.as_bytes());
        let forged = format!("{head}.{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(other.to_bytes()));
        for bad in [forged, "v1.x.y".to_string(), String::new()] {
            match super::join(State(door.clone()), q(bad)).await {
                Ok(r) => panic!("refused, not {}", r.status()),
                Err(super::Refusal(s, _)) => assert_eq!(s, StatusCode::BAD_REQUEST),
            }
        }
        assert_eq!(door.joins.lock().unwrap().len(), 1, "a refused claim keeps nothing");
    }

    fn joining(handle: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, format!("door=s1; door-join={handle}").parse().unwrap());
        h.insert(header::ORIGIN, "http://127.0.0.1:8233".parse().unwrap());
        h
    }

    fn pend(door: &super::Door, token: String) -> String {
        let handle = super::random_hex(32);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
        door.joins.lock().unwrap().insert(handle.clone(), super::Join { token, until, kept: Default::default() });
        handle
    }

    /// Only the node's definite answer ends a claim: a failure to reach the Arc, or an
    /// answer that is not one, keeps it for a retry (Software Security, point 2).
    #[tokio::test]
    async fn a_claim_ends_on_the_arcs_answer_and_survives_a_failure_to_reach_it() {
        // Unreachable, then a 503: kept both times, and the cookie stays.
        for arc_at in ["http://127.0.0.1:1".to_string(), arc(503, serde_json::json!({ "error": "busy" })).await.0] {
            let door = door_for(arc_at);
            signed_in(&door).await;
            let handle = pend(&door, claim(2));
            match super::join_finish(State(door.clone()), joining(&handle), None).await {
                Ok(r) => panic!("a failure, not {}", r.status()),
                Err(super::Refusal(s, _)) => assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE),
            }
            assert!(door.joins.lock().unwrap().contains_key(&handle), "the claim is kept for a retry");
        }

        // Admitted: six fresh bundles and the claim went to the Arc, the landing's values and
        // the rooms come back, and the claim and its cookie are done with.
        let (at, got) = arc(200, serde_json::json!({
            "admitted": "cd", "c": "skills", "a": "k2m3n4p5q6r7s8t9uvwx", "rooms": ["r1"], "unjoined": [], "fallback": null,
        }))
        .await;
        let door = door_for(at);
        signed_in(&door).await;
        let token = claim(3);
        let handle = pend(&door, token.clone());
        let r = super::join_finish(State(door.clone()), joining(&handle), None).await.ok().expect("admitted");
        assert_eq!(r.status(), StatusCode::OK);
        assert!(r.headers()[header::SET_COOKIE].to_str().unwrap().contains("door-join=; "), "the cookie is cleared");
        let body: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap()).unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "admitted": true, "site": SITE, "c": "skills", "a": "k2m3n4p5q6r7s8t9uvwx", "rooms": ["r1"], "unjoined": [], "fallback": null,
                // J-A: no client is registered for SITE here, so no home.
                "home": null,
            })
        );
        assert_eq!(*got.lock().unwrap(), vec![serde_json::json!({ "claim": token, "bundles": vec!["B"; super::JOIN_BUNDLES] })]);
        assert!(door.joins.lock().unwrap().is_empty());
        // A second press finds nothing to send.
        assert!(super::join_finish(State(door.clone()), joining(&handle), None).await.is_err());
        assert_eq!(got.lock().unwrap().len(), 1);

        // Admitted to the Site, a room not: the claim and its cookie are kept for the retry
        // that adds it, and the answer names the room and why (D-58).
        let unjoined = serde_json::json!([{ "room": "r2", "why": "the node is not an admitter of r2" }]);
        let (at, _) = arc(200, serde_json::json!({ "admitted": "cd", "c": "skills", "a": null, "rooms": ["r1"], "unjoined": unjoined })).await;
        let door = door_for(at);
        signed_in(&door).await;
        let token = claim(5);
        let handle = pend(&door, token.clone());
        let r = super::join_finish(State(door.clone()), joining(&handle), None).await.ok().expect("admitted");
        assert_eq!(r.status(), StatusCode::OK);
        assert!(r.headers().get(header::SET_COOKIE).is_none(), "the cookie stays for the retry");
        let body: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap()).unwrap();
        assert_eq!((&body["rooms"], &body["unjoined"]), (&serde_json::json!(["r1"]), &unjoined));
        assert_eq!(door.joins.lock().unwrap()[&handle].token, token, "the claim is kept for the retry");

        // No room of the Arc's carries the claim's choice: the answer says so (ICD 2.1.0 row 3).
        let (at, _) = arc(200, serde_json::json!({ "admitted": "cd", "c": "financial", "a": null, "rooms": ["r0"], "unjoined": [], "fallback": "financial" })).await;
        let door = door_for(at);
        signed_in(&door).await;
        let handle = pend(&door, claim(6));
        let r = super::join_finish(State(door.clone()), joining(&handle), None).await.ok().expect("admitted");
        let body: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap()).unwrap();
        assert_eq!(body["fallback"], "financial");

        // Refused by the node: definite, so the claim and the cookie end, and its words are told.
        let door = door_for(arc(409, serde_json::json!({ "error": "this claim has been used" })).await.0);
        signed_in(&door).await;
        let handle = pend(&door, claim(4));
        let r = super::join_finish(State(door.clone()), joining(&handle), None).await.ok().expect("an answer, not a failure");
        assert_eq!(r.status(), StatusCode::CONFLICT);
        assert!(r.headers()[header::SET_COOKIE].to_str().unwrap().contains("door-join=; "));
        assert!(door.joins.lock().unwrap().is_empty());
    }

    /// An admitted join's answer carries what its row keeps of the claim: the choice and share
    /// the Door verified at /join. A join the node refused carries none of it.
    #[tokio::test]
    async fn an_admitted_join_keeps_the_claims_choice_and_share_for_its_row() {
        let q = |t: String| Query(HashMap::from([("claim".to_string(), t)]));
        let handle_of = |r: &axum::response::Response| {
            let c = r.headers()[header::SET_COOKIE].to_str().unwrap().to_string();
            c.split(';').next().unwrap().strip_prefix("door-join=").unwrap().to_string()
        };
        let (at, _) = arc(200, serde_json::json!({ "admitted": "cd", "c": "skills", "a": null, "rooms": [], "unjoined": [] })).await;
        let door = door_for(at);
        signed_in(&door).await;
        let landed = super::join(State(door.clone()), q(claim(6))).await.ok().expect("the claim lands");
        let r = super::join_finish(State(door.clone()), joining(&handle_of(&landed)), None).await.ok().expect("admitted");
        assert_eq!(r.status(), StatusCode::OK);
        let kept = r.extensions().get::<crate::tw::Kept>().expect("the row's kept fields");
        assert_eq!(serde_json::Value::Object(kept.0.clone()), serde_json::json!({ "c": "skills", "a": "k2m3n4p5q6r7s8t9uvwx" }), "the claim's, as verified, not the Arc's answer");

        let door = door_for(arc(409, serde_json::json!({ "error": "this claim has been used" })).await.0);
        signed_in(&door).await;
        let landed = super::join(State(door.clone()), q(claim(7))).await.ok().expect("the claim lands");
        let r = super::join_finish(State(door.clone()), joining(&handle_of(&landed)), None).await.ok().expect("an answer");
        assert_eq!(r.status(), StatusCode::CONFLICT);
        assert!(r.extensions().get::<crate::tw::Kept>().is_none(), "a refused claim keeps nothing of it");
    }

    // ── D-55: the work a start pays ──────────────────────────────────────────────

    /// An open attempt from `client`, drawing on `join`'s claim or none; its process a
    /// `cat` that ends with its leash. Answers its key.
    fn opened(door: &super::Door, signup: bool, client: &str, join: Option<&str>) -> String {
        let (child, leash) = cat();
        let dir = door.cfg.root.join(super::random_hex(8));
        std::fs::create_dir_all(&dir).unwrap();
        let proc = super::Proc { child, _leash: leash, port: 0, secret: String::new(), dir };
        let a = super::Attempt { proc, started: std::time::Instant::now(), signup, client: client.into(), join: join.map(str::to_string) };
        let key = super::random_hex(16);
        door.attempts.lock().unwrap().insert(key.clone(), a);
        key
    }

    fn open_attempt(door: &super::Door, signup: bool) {
        opened(door, signup, "c", None);
    }

    /// What `/v2/work` answers for a start of `kind`: its challenge, or none.
    async fn owed(door: &super::Door, h: HeaderMap, kind: &str) -> Option<String> {
        let q = Query(HashMap::from([("for".to_string(), kind.to_string())]));
        let r = super::work(State(door.clone()), h, q).await.ok().expect("an answer");
        assert_eq!(r.headers()[header::CACHE_CONTROL], "no-store");
        let body: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap()).unwrap();
        body["challenge"].as_str().map(str::to_string)
    }

    /// A sign-up without a claim owes the work at `BITS`; one a live claim waits for owes
    /// none, and asking spends nothing.
    #[tokio::test]
    async fn a_sign_up_owes_the_work_unless_a_claim_waits_for_it() {
        let door = door_for(String::new());
        let c = owed(&door, HeaderMap::new(), "signup").await.expect("owed without a claim");
        assert_eq!(c.split('.').nth(3), Some(super::pow::BITS.to_string().as_str()), "{c}");

        let handle = pend(&door, claim(5));
        assert_eq!(owed(&door, joining(&handle), "signup").await, None, "the claim is the control");
        assert!(door.joins.lock().unwrap().contains_key(&handle), "asking spends nothing");

        assert!(owed(&door, joining("0"), "signup").await.is_some(), "a cookie naming no join is no claim");
        door.joins.lock().unwrap().get_mut(&handle).unwrap().until = std::time::Instant::now();
        assert!(owed(&door, joining(&handle), "signup").await.is_some(), "an expired join is no claim");
    }

    /// A sign-in owes the work only while open attempts are over half the site-wide cap.
    #[tokio::test]
    async fn a_sign_in_owes_the_work_only_over_half_the_cap() {
        let door = door_for(String::new());
        for _ in 0..door.cfg.max_attempts / 2 {
            open_attempt(&door, false);
        }
        assert_eq!(owed(&door, HeaderMap::new(), "signin").await, None, "half is not over half");
        open_attempt(&door, true);
        let c = owed(&door, HeaderMap::new(), "signin").await.expect("owed over half, sign-ups counted");
        assert_eq!(c.split('.').nth(3), Some(super::pow::BITS.to_string().as_str()), "{c}");
        assert_eq!(owed(&door, HeaderMap::new(), "signup").await.map(|_| ()), Some(()));
    }

    #[tokio::test]
    async fn work_is_asked_for_by_the_start_it_is_for() {
        let door = door_for(String::new());
        for q in [HashMap::new(), HashMap::from([("for".to_string(), "join".to_string())])] {
            match super::work(State(door.clone()), HeaderMap::new(), Query(q)).await {
                Ok(r) => panic!("refused, not {}", r.status()),
                Err(super::Refusal(s, why)) => assert_eq!((s, why.as_str()), (StatusCode::BAD_REQUEST, "for is signup or signin")),
            }
        }
    }

    /// The window may run a Worker, and only from its own origin: pow-worker.js, served here.
    #[tokio::test]
    async fn the_window_runs_its_own_worker() {
        use axum::response::IntoResponse;
        let r = super::signin(State(door_for(String::new())), Query(HashMap::new())).await;
        let csp = r.headers()[header::CONTENT_SECURITY_POLICY].to_str().unwrap().to_string();
        assert!(csp.split(';').map(str::trim).any(|d| d == "worker-src 'self'"), "{csp}");
        let js = super::pow_worker_js().await.into_response();
        assert_eq!(js.headers()[header::CONTENT_TYPE], "text/javascript");
        let body = axum::body::to_bytes(js.into_body(), 1 << 20).await.unwrap();
        assert_eq!(&body[..], include_str!("../../web/door/pow-worker.js").as_bytes());
    }

    /// What `paid` refuses a start with: its status and words.
    fn unpaid(door: &super::Door, h: &HeaderMap, start: super::pow::Start, work: Option<(&str, &str)>) -> Option<(u16, String)> {
        let work = work.map(|(c, n)| super::Work { challenge: c.into(), nonce: n.into() });
        super::paid(door, h, start, work.as_ref(), "/v2/signup").err().map(|super::Refusal(s, why)| (s.as_u16(), why))
    }

    /// No work where it is owed is refused; none where it is not passes; work presented is
    /// checked either way.
    #[tokio::test]
    async fn a_start_pays_what_it_owes() {
        use super::pow::Start::{SignIn, SignUp};
        let door = door_for(String::new());
        let owed = Some((403, "this start needs its proof of work".to_string()));
        let none = HeaderMap::new();
        assert_eq!(unpaid(&door, &none, SignUp, None), owed, "a sign-up without a claim");
        let claim = joining(&pend(&door, claim(6)));
        assert_eq!(unpaid(&door, &claim, SignUp, None), None, "a sign-up a claim waits for");
        assert_eq!(unpaid(&door, &none, SignIn, None), None, "a sign-in, the pool under half");
        for _ in 0..=door.cfg.max_attempts / 2 {
            open_attempt(&door, false);
        }
        assert_eq!(unpaid(&door, &none, SignIn, None), owed, "a sign-in, the pool over half");
        let malformed = Some((403, "the challenge is malformed".to_string()));
        assert_eq!(unpaid(&door, &claim, SignUp, Some(("v1.x", "0"))), malformed, "presented, it is checked, owed or not");
        let c = door.pow.issue(super::now_secs(), SignIn);
        assert_eq!(
            unpaid(&door, &none, SignUp, Some((&c, "0"))),
            Some((403, "the challenge was issued for another start".to_string()))
        );
    }

    /// What `room` refuses a start from `client`, drawing on `join`, with: its status and words.
    fn no_room(door: &super::Door, client: &str, join: Option<&str>) -> Option<(u16, String)> {
        super::room(door, "/v2/signup", client, join).err().map(|super::Refusal(s, why)| (s.as_u16(), why))
    }

    /// The reserve is the kiosk's: a flood without a claim fills the rest and not it, and a
    /// full reserve refuses a claim's start while the rest has room.
    #[tokio::test]
    async fn a_flood_without_a_claim_leaves_the_kiosks_reserve() {
        let door = door_for(String::new());
        assert_eq!((door.cfg.cap(false), door.cfg.cap(true)), (6, 2), "2 of 8 carved out");
        let full = Some((503, "too many sign-ins are open; try again shortly".to_string()));
        for i in 0..6 {
            assert_eq!(no_room(&door, &format!("n{i}"), None), None, "open attempt {i}");
            opened(&door, i % 2 == 0, &format!("n{i}"), None);
        }
        assert_eq!(no_room(&door, "n9", None), full, "the rest is full");
        assert_eq!(no_room(&door, "n9", Some("j1")), None, "the reserve is not");
        opened(&door, true, "n9", Some("j1"));
        opened(&door, true, "n9", Some("j2"));
        assert_eq!(no_room(&door, "n9", Some("j3")), full, "the reserve full");
        door.attempts.lock().unwrap().retain(|_, a| a.join.is_some());
        assert_eq!(no_room(&door, "n9", Some("j3")), full, "and room in the rest is not the reserve's");
        assert_eq!(no_room(&door, "n9", None), None);
    }

    /// One open attempt per claim (Software Security): a second start for a join ends the
    /// first, and no other claim's.
    #[tokio::test]
    async fn a_claim_holds_one_open_attempt() {
        let door = door_for(String::new());
        let first = opened(&door, true, "c", Some("j"));
        let other = opened(&door, true, "c", Some("k"));
        assert_eq!(no_room(&door, "c", Some("j")), None, "the reserve is full, but j's own slot is taken back");
        let held = door.attempts.lock().unwrap();
        assert!(!held.contains_key(&first), "the second start for j ended the first");
        assert!(held.contains_key(&other), "k's is untouched");
    }

    /// SEC-A2 holds an address to its open attempts; a claim's start is exempt, since a
    /// venue's visitors share one address.
    #[tokio::test]
    async fn a_claims_start_is_exempt_from_the_per_address_cap() {
        let door = door_for(String::new());
        for _ in 0..door.cfg.per_addr {
            opened(&door, false, "venue", None);
        }
        let here = Some((429, "too many sign-ins are open from here; finish one or wait".to_string()));
        assert_eq!(no_room(&door, "venue", None), here);
        assert_eq!(no_room(&door, "elsewhere", None), None);
        assert_eq!(no_room(&door, "venue", Some("j1")), None, "a claim from the venue");
        opened(&door, true, "venue", Some("j1"));
        assert_eq!(no_room(&door, "venue", Some("j2")), None, "and another");
    }

    /// The website's `/assets`, baked into the webapp (D-54), are served sandboxed and
    /// never framed; a page is not sandboxed, and the window keeps its own policy.
    #[tokio::test]
    async fn assets_are_sandboxed_and_pages_are_not() {
        let door = door_for(String::new());
        let webapp = door.cfg.root.join("webapp");
        std::fs::create_dir_all(webapp.join("assets")).unwrap();
        std::fs::write(webapp.join("index.html"), "<!doctype html>").unwrap();
        std::fs::write(webapp.join("assets/site.js"), "0").unwrap();
        let port = serve(super::router(&door, webapp.to_str().unwrap())).await;
        let get = |path: &str| reqwest::get(format!("http://127.0.0.1:{port}{path}"));
        let csp = |r: &reqwest::Response| r.headers()["content-security-policy"].to_str().unwrap().to_string();

        let a = get("/assets/site.js").await.unwrap();
        assert_eq!(a.status(), 200);
        assert_eq!(csp(&a), "sandbox; frame-ancestors 'none'");
        assert_eq!(a.headers()["x-content-type-options"], "nosniff");
        assert_eq!(a.headers()["referrer-policy"], "no-referrer");

        let page = get("/").await.unwrap();
        assert_eq!(page.status(), 200);
        assert_eq!(csp(&page), "frame-ancestors 'none'");
        assert_eq!(page.headers()["x-content-type-options"], "nosniff");
        let window = get("/signin").await.unwrap();
        assert!(csp(&window).starts_with("default-src 'none'") && !csp(&window).contains("sandbox"), "{}", csp(&window));
    }

    /// W-98 Resources: the editor is served whole from this origin, byte for byte its parts in
    /// order, as a script a site pins by its hash.
    #[tokio::test]
    async fn the_resources_editor_is_served_whole() {
        let door = door_for(String::new());
        let webapp = door.cfg.root.join("webapp");
        std::fs::create_dir_all(&webapp).unwrap();
        std::fs::write(webapp.join("index.html"), "<!doctype html>").unwrap();
        let port = serve(super::router(&door, webapp.to_str().unwrap())).await;
        let r = reqwest::get(format!("http://127.0.0.1:{port}/v2/resources.js")).await.unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(r.headers()["content-type"], "text/javascript");
        let body = r.text().await.unwrap();
        assert_eq!(body, super::RESOURCES_JS);
        for part in ["NS.markdown = {", "NS.pdf = { zone: zone, preview: preview }", "NS.mount = mount"] {
            assert!(body.contains(part), "{part}");
        }
    }

    // ── D-34 (c): one process per person ─────────────────────────────────────────

    /// A session process for sign-in, faked as account.rs answers the Door, signing as
    /// `key`; `/i/open` answers `open`. It keeps each path it is asked, in order, with a
    /// forgotten session's ref, and `/v2/me` answers the `x-door-ref` it was sent.
    async fn account(key: ed25519_dalek::SigningKey, open: u16) -> (u16, Arc<Mutex<Vec<String>>>) {
        let asked: Arc<Mutex<Vec<String>>> = Default::default();
        (account_with(key, open, 0, asked.clone()).await, asked)
    }

    /// The same, keeping its paths in `asked`, which others may share, and taking `end_ms`
    /// over `/i/end`, as a process sealing its state does: "/i/end" when asked, and
    /// "/i/end done" when it answers.
    async fn account_with(key: ed25519_dalek::SigningKey, open: u16, end_ms: u64, asked: Arc<Mutex<Vec<String>>>) -> u16 {
        use axum::routing::{get, post};
        let pk = hex::encode(key.verifying_key().to_bytes());
        let log = |path: &'static str, asked: &Arc<Mutex<Vec<String>>>| {
            let asked = asked.clone();
            move || {
                asked.lock().unwrap().push(path.to_string());
                async { axum::Json(serde_json::json!({})) }
            }
        };
        let (sign_log, pk_sign, forget_log, prove_log, open_log) = (asked.clone(), pk.clone(), asked.clone(), asked.clone(), asked.clone());
        let app = axum::Router::new()
            .route("/i/prove", post(move || {
                prove_log.lock().unwrap().push("/i/prove".into());
                async { axum::Json(serde_json::json!({})) }
            }))
            .route("/i/sign", post(move |axum::Json(v): axum::Json<serde_json::Value>| {
                sign_log.lock().unwrap().push("/i/sign".into());
                let msg = pacific_core::identity::auth_payload(v["audience"].as_str().unwrap(), &format!("ed25519:{pk_sign}"), v["nonce"].as_str().unwrap());
                let sig = ed25519_dalek::Signer::sign(&key, &msg);
                async move { axum::Json(serde_json::json!({ "sig": hex::encode(sig.to_bytes()) })) }
            }))
            .route("/i/open", post(move || {
                open_log.lock().unwrap().push("/i/open".into());
                async move {
                    (StatusCode::from_u16(open).unwrap(), axum::Json(serde_json::json!({ "resume": "R", "restored": false }))).into_response()
                }
            }))
            .route("/i/forget", post(move |axum::Json(v): axum::Json<serde_json::Value>| {
                forget_log.lock().unwrap().push(format!("/i/forget {}", v["ref"].as_str().unwrap_or_default()));
                async { axum::Json(serde_json::json!({})) }
            }))
            .route("/i/end", post({
                let asked = asked.clone();
                move || {
                    asked.lock().unwrap().push("/i/end".into());
                    let asked = asked.clone();
                    async move {
                        tokio::time::sleep(std::time::Duration::from_millis(end_ms)).await;
                        asked.lock().unwrap().push("/i/end done".into());
                        axum::Json(serde_json::json!({}))
                    }
                }
            }))
            .route("/v2/me", get(|h: HeaderMap| async move {
                axum::Json(serde_json::json!({ "ref": h.get("x-door-ref").and_then(|v| v.to_str().ok()) }))
            }));
        let _ = log;
        serve(app).await
    }

    use axum::response::IntoResponse;

    /// A sign-in attempt whose process is the fake at `port`.
    fn attempt_at(door: &super::Door, port: u16) -> String {
        let (child, leash) = cat();
        let dir = door.cfg.root.join(super::random_hex(8));
        std::fs::create_dir_all(&dir).unwrap();
        let proc = super::Proc { child, _leash: leash, port, secret: "x".into(), dir };
        let id = super::random_hex(16);
        let a = super::Attempt { proc, started: std::time::Instant::now(), signup: false, client: "c".into(), join: None };
        door.attempts.lock().unwrap().insert(id.clone(), a);
        id
    }

    /// A sign-in's finish for `key`'s account, as the window sends it: the session's
    /// cookie, or the refusal.
    async fn finish(door: &super::Door, attempt: &str, key: &ed25519_dalek::SigningKey) -> Result<(String, serde_json::Value), (StatusCode, String)> {
        let mut handle = key.verifying_key().to_bytes().to_vec();
        handle.extend_from_slice(b"auth.example.test");
        let i = serde_json::from_value(serde_json::json!({
            "attempt": attempt, "sealed": { "epk": "e", "iv": "i", "ct": "c" },
            "handle": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(handle),
        }))
        .unwrap();
        match super::sign_in_finish(State(door.clone()), axum::Json(i)).await {
            Err(super::Refusal(s, why)) => Err((s, why)),
            Ok(r) => {
                let c = r.headers()[header::SET_COOKIE].to_str().unwrap().split(';').next().unwrap().to_string();
                let body = serde_json::from_slice(&axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap()).unwrap();
                Ok((c, body))
            }
        }
    }

    fn person(door: &super::Door, key: &ed25519_dalek::SigningKey) -> Option<(u16, usize)> {
        let live = door.live.lock().unwrap();
        live.persons.get(&hex::encode(key.verifying_key().to_bytes())).map(|p| (p.proc.port, p.sessions.len()))
    }

    /// K-41: a second sign-in of a live person proves, and joins their process: it opens
    /// nothing, and the attempt's process goes.
    #[tokio::test]
    async fn a_second_sign_in_joins_the_persons_process() {
        let door = door_for(String::new());
        let key = ed25519_dalek::SigningKey::from_bytes(&[3u8; 32]);
        let (first, asked_first) = account(key.clone(), 200).await;
        let (_, body) = finish(&door, &attempt_at(&door, first), &key).await.expect("the first sign-in");
        assert_eq!(body["resume"], "R", "the first opens the process, and resumes");
        assert_eq!(*asked_first.lock().unwrap(), ["/i/prove", "/i/sign", "/i/open"]);
        assert_eq!(person(&door, &key), Some((first, 1)));

        let (second, asked_second) = account(key.clone(), 200).await;
        let a = attempt_at(&door, second);
        let (_, body) = finish(&door, &a, &key).await.expect("the second sign-in");
        assert_eq!(body["resume"], serde_json::Value::Null, "nothing resumed: theirs is live");
        assert_eq!(*asked_second.lock().unwrap(), ["/i/prove", "/i/sign"], "proved, and opened nothing");
        assert_eq!(person(&door, &key), Some((first, 2)), "one process, two sessions");
        assert!(!door.attempts.lock().unwrap().contains_key(&a), "the attempt is spent");
    }

    /// NC-58's regression: ten sign-ins of one person at once make one process.
    #[tokio::test]
    async fn ten_sign_ins_of_one_person_make_one_process() {
        let door = door_for(String::new());
        let key = ed25519_dalek::SigningKey::from_bytes(&[4u8; 32]);
        let mut fakes = Vec::new();
        for _ in 0..10 {
            fakes.push(account(key.clone(), 200).await);
        }
        let attempts: Vec<String> = fakes.iter().map(|(port, _)| attempt_at(&door, *port)).collect();
        let done = futures_util::future::join_all(attempts.iter().map(|a| finish(&door, a, &key))).await;
        assert!(done.iter().all(|d| d.is_ok()), "{:?}", done.iter().filter_map(|d| d.as_ref().err()).collect::<Vec<_>>());
        let opened = fakes.iter().filter(|(_, asked)| asked.lock().unwrap().contains(&"/i/open".to_string())).count();
        assert_eq!(opened, 1, "one process opened");
        let (port, sessions) = person(&door, &key).expect("the person");
        assert_eq!(sessions, 10);
        assert!(fakes.iter().any(|(p, asked)| *p == port && asked.lock().unwrap().contains(&"/i/open".to_string())), "the one opened is theirs");
        assert_eq!(door.live.lock().unwrap().persons.len(), 1);
    }

    /// Signing out ends that session alone: its person's process forgets it, and their
    /// other session goes on. Their last ends the process.
    #[tokio::test]
    async fn a_sign_out_ends_one_session_and_the_last_ends_the_process() {
        let door = door_for(String::new());
        let key = ed25519_dalek::SigningKey::from_bytes(&[5u8; 32]);
        let (port, asked) = account(key.clone(), 200).await;
        let (one, _) = finish(&door, &attempt_at(&door, port), &key).await.unwrap();
        let (other, _) = account(key.clone(), 200).await;
        let (two, _) = finish(&door, &attempt_at(&door, other), &key).await.unwrap();
        let out = |c: &str| {
            let mut h = HeaderMap::new();
            h.insert(header::COOKIE, c.parse().unwrap());
            h.insert(header::ORIGIN, "http://127.0.0.1:8233".parse().unwrap());
            super::sign_out(State(door.clone()), h)
        };
        let me = |c: &str| {
            let rq = axum::extract::Request::builder()
                .uri("/v2/me")
                .header(header::COOKIE, c)
                .header("x-door-ref", "forged")
                .body(axum::body::Body::empty())
                .unwrap();
            super::fwd(State(door.clone()), rq)
        };
        let ref_of = |c: &str| c.split('=').nth(1).unwrap().to_string();

        let r = me(&two).await.ok().expect("the second session answers");
        let body: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap()).unwrap();
        assert_eq!(body["ref"], ref_of(&two), "the process is told the Door's id for the session, not a client's");

        assert!(out(&one).await.is_ok());
        assert_eq!(asked.lock().unwrap().last().unwrap(), &format!("/i/forget {}", ref_of(&one)));
        assert_eq!(person(&door, &key), Some((port, 1)), "their process goes on");
        assert!(me(&one).await.is_err(), "the signed-out session answers nothing");
        assert!(me(&two).await.is_ok(), "the other still works");

        assert!(out(&two).await.is_ok());
        assert_eq!(asked.lock().unwrap().last().unwrap(), "/i/end done", "the last ends the process");
        assert_eq!(person(&door, &key), None);
    }

    /// A process that cannot open for now (the auth service or the relay unreachable)
    /// answers 503, and the attempt is kept for a retry within its life.
    #[tokio::test]
    async fn an_open_that_may_pass_keeps_the_attempt() {
        let door = door_for(String::new());
        let key = ed25519_dalek::SigningKey::from_bytes(&[6u8; 32]);
        let (port, _) = account(key.clone(), 503).await;
        let a = attempt_at(&door, port);
        assert_eq!(finish(&door, &a, &key).await.err().map(|e| e.0), Some(StatusCode::SERVICE_UNAVAILABLE));
        assert!(door.attempts.lock().unwrap().contains_key(&a), "kept for the retry");
        assert_eq!(person(&door, &key), None, "and nobody is signed in");
    }

    /// The sweeper ends an idle session as sign-out does: forgotten while its person has
    /// another, the process ended with the last. The sweeper's own thread blocks, so the
    /// fake answers from another.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_idle_session_ends_as_a_signed_out_one_does() {
        let door = door_for(String::new());
        let key = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let (port, asked) = account(key.clone(), 200).await;
        let (one, _) = finish(&door, &attempt_at(&door, port), &key).await.unwrap();
        let (other, _) = account(key.clone(), 200).await;
        finish(&door, &attempt_at(&door, other), &key).await.unwrap();
        let idle = |c: &str| {
            let id = c.split('=').nth(1).unwrap();
            door.live.lock().unwrap().sessions.get_mut(id).unwrap().seen =
                std::time::Instant::now().checked_sub(std::time::Duration::from_secs(1000)).unwrap();
        };
        idle(&one);
        let d = door.clone();
        std::thread::spawn(move || super::sweep(&d)).join().unwrap();
        assert_eq!(asked.lock().unwrap().last().unwrap(), &format!("/i/forget {}", one.split('=').nth(1).unwrap()));
        assert_eq!(person(&door, &key), Some((port, 1)));
        let two = door.live.lock().unwrap().sessions.keys().next().unwrap().clone();
        idle(&format!("door={two}"));
        let d = door.clone();
        std::thread::spawn(move || super::sweep(&d)).join().unwrap();
        assert_eq!(asked.lock().unwrap().last().unwrap(), "/i/end done");
        assert_eq!(person(&door, &key), None);
    }

    /// NC-63: a sign-in while the person's last session is being ended waits for that
    /// end, its state sealed, and only then opens their process, from that seal.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_sign_in_waits_for_the_last_sessions_seal() {
        let door = door_for(String::new());
        let key = ed25519_dalek::SigningKey::from_bytes(&[8u8; 32]);
        let asked: Arc<Mutex<Vec<String>>> = Default::default();
        let ending = account_with(key.clone(), 200, 1500, asked.clone()).await;
        let (c, _) = finish(&door, &attempt_at(&door, ending), &key).await.unwrap();
        let id = c.split('=').nth(1).unwrap().to_string();
        door.live.lock().unwrap().sessions.get_mut(&id).unwrap().seen =
            std::time::Instant::now().checked_sub(std::time::Duration::from_secs(1000)).unwrap();
        let d = door.clone();
        let sweeper = std::thread::spawn(move || super::sweep(&d));
        let t = std::time::Instant::now();
        while !asked.lock().unwrap().contains(&"/i/end".to_string()) {
            assert!(t.elapsed() < std::time::Duration::from_secs(10), "the sweeper did not start ending it");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let next = account_with(key.clone(), 200, 0, asked.clone()).await;
        finish(&door, &attempt_at(&door, next), &key).await.expect("the sign-in");
        sweeper.join().unwrap();
        let seq = asked.lock().unwrap().clone();
        let at = |p: &str| seq.iter().rposition(|x| x == p).unwrap_or_else(|| panic!("{p} in {seq:?}"));
        assert!(at("/i/end done") < at("/i/open"), "opened before the last session's seal: {seq:?}");
        assert_eq!(person(&door, &key), Some((next, 1)), "the new process is theirs");
        assert!(door.opening.lock().unwrap().is_empty(), "no turn left behind");
    }

    /// build-worker-c (4): a sign-in that found the person live, and finds them gone as it
    /// seats its session, opens their process from its own proved attempt.
    #[tokio::test]
    async fn a_person_gone_as_a_sign_in_joins_them_is_opened_again() {
        let door = door_for(String::new());
        let key = ed25519_dalek::SigningKey::from_bytes(&[10u8; 32]);
        let (port, asked) = account(key.clone(), 200).await;
        let a = attempt_at(&door, port);
        let attempt = door.attempts.lock().unwrap().remove(&a).unwrap();
        let pk = hex::encode(key.verifying_key().to_bytes());
        // Nobody live: as if their last session ended between the look and the seat.
        let r = super::join_live(&door, attempt, &a, &pk, None, "/v2/signin/finish").await.ok().expect("signed in");
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(*asked.lock().unwrap(), ["/i/open"], "opened from the attempt's process, its proof already made");
        assert_eq!(person(&door, &key), Some((port, 1)));
    }

    /// NC-63: an attempt's process that dies mid-`/i/prove` is answered, promptly, and
    /// holds nothing: no person, no turn, and the next sign-in goes through.
    #[tokio::test]
    async fn an_attempt_whose_process_dies_mid_proof_is_answered() {
        let door = door_for(String::new());
        let key = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
        // A process that takes the request and dies: the connection closes, unanswered.
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut c, _)) = l.accept().await {
                let mut buf = [0u8; 1024];
                let _ = tokio::io::AsyncReadExt::read(&mut c, &mut buf).await;
            }
        });
        let a = attempt_at(&door, dead);
        let answered = tokio::time::timeout(std::time::Duration::from_secs(10), finish(&door, &a, &key)).await;
        let refused = answered.expect("answered, not hung").err().expect("refused");
        assert_eq!(refused.0, StatusCode::BAD_GATEWAY, "{refused:?}");
        assert!(!door.attempts.lock().unwrap().contains_key(&a), "the attempt is spent");
        assert_eq!(person(&door, &key), None);
        assert!(door.opening.lock().unwrap().is_empty(), "no turn taken");
        let (port, _) = account(key.clone(), 200).await;
        finish(&door, &attempt_at(&door, port), &key).await.expect("the next sign-in goes through");
        assert_eq!(person(&door, &key), Some((port, 1)));
    }

    /// A person's seals live in the unit's StateDirectory, and never where a restart
    /// clears them: under the root (a tmpfs in production) or the RuntimeDirectory.
    #[test]
    fn seals_are_never_kept_where_a_restart_clears_them() {
        use super::seals_dir;
        let root = std::path::Path::new("/run/door/root");
        let run = || Some("/run/door".to_string());
        let s = |x: &str| Some(x.to_string());
        assert_eq!(seals_dir(None, s("/var/lib/door"), root, run()), Ok("/var/lib/door/seals".into()), "the StateDirectory's");
        assert_eq!(seals_dir(s("/srv/seals"), s("/var/lib/door"), root, run()), Ok("/srv/seals".into()), "DOOR_SEALS first");
        assert_eq!(seals_dir(None, None, std::path::Path::new("/tmp/d/root"), None), Ok("/tmp/d/seals".into()), "beside the root, off systemd");
        let refused = |r: Result<std::path::PathBuf, String>| r.expect_err("refused");
        let e = refused(seals_dir(None, None, root, run()));
        assert!(e.contains("/run/door/seals") && e.contains("/run/door"), "the default beside a tmpfs root under the RuntimeDirectory: {e}");
        assert!(refused(seals_dir(s("/run/door/root/seals"), None, root, None)).contains("/run/door/root"), "under the root");
        assert!(refused(seals_dir(s("/var/lib/../../run/door/x"), None, root, run())).contains("/run/door"), "read as written");
        assert!(refused(seals_dir(None, s("/run/door/state"), root, s("/run/other:/run/door"))).contains("/run/door"), "any RuntimeDirectory");
    }

    /// The RP ID is the Door's own host, and no other: a parent domain would let any host
    /// under it run the ceremony (D-33, O-58).
    #[test]
    fn the_rp_id_is_the_doors_own_host() {
        use super::rp_id_for;
        assert_eq!(rp_id_for("https://app.wallflowers.io", None), Ok("app.wallflowers.io".into()));
        assert_eq!(rp_id_for("https://app.wallflowers.io", Some("app.wallflowers.io")), Ok("app.wallflowers.io".into()));
        assert_eq!(rp_id_for("http://localhost:8233", None), Ok("localhost".into()), "loopback");
        assert_eq!(rp_id_for("http://127.0.0.1:8233", Some("localhost")), Ok("localhost".into()), "loopback by address");
        let e = rp_id_for("https://app.wallflowers.io", Some("wallflowers.io")).expect_err("the parent domain is refused");
        assert!(e.contains("wallflowers.io") && e.contains("app.wallflowers.io"), "{e}");
        assert!(rp_id_for("https://app.wallflowers.io", Some("www.wallflowers.io")).is_err(), "a sibling");
        assert!(rp_id_for("http://localhost:8233", Some("wallflowers.io")).is_err(), "loopback takes localhost only");
    }

    /// A stand-in session process: `cat`, its stdin the far end of its leash.
    fn cat() -> (std::process::Child, std::os::unix::net::UnixStream) {
        let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        let child = std::process::Command::new("cat")
            .stdin(std::process::Stdio::from(std::os::fd::OwnedFd::from(theirs)))
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        (child, ours)
    }

    /// A session process names the port it bound on its leash: one line, its number.
    /// Anything else, an end, or silence, is no port.
    #[tokio::test]
    async fn a_session_names_its_port_on_its_leash() {
        use std::io::Write;
        use std::time::Duration;
        let within = Duration::from_millis(200);
        let (ours, mut theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        theirs.write_all(b"51234\n").unwrap();
        assert_eq!(super::told_port(&ours, within).await.map(|(p, _)| p), Ok(51234));
        let (ours, mut theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        theirs.write_all(b"a port\n").unwrap();
        let said = super::told_port(&ours, within).await.map(|(p, _)| p);
        assert!(said.as_ref().is_err_and(|e| e.contains("named no port")), "{said:?}");
        let (ours, _theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        let silent = super::told_port(&ours, within).await.map(|(p, _)| p);
        assert!(silent.as_ref().is_err_and(|e| e.contains("within")), "silence: {silent:?}");
        let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        drop(theirs);
        let ended = super::told_port(&ours, within).await.map(|(p, _)| p);
        assert!(ended.as_ref().is_err_and(|e| e.contains("ended")), "an end: {ended:?}");
    }

    /// NC-80: a session is its supervisor's build and image, or says which it is and which
    /// its supervisor is; a supervisor that names neither is older than the session. Its
    /// refusal on the leash is the reason the start gives.
    #[tokio::test]
    async fn a_session_of_another_build_or_image_refuses_naming_both() {
        use super::skew;
        use std::io::Write;
        let ours = ("abc1234", "inode 7 of 10 bytes, modified 1.000000000");
        assert_eq!(skew(ours, (Some(ours.0), Some(ours.1))), None, "the same build and image");
        let other = skew(ours, (Some("def5678"), Some(ours.1))).expect("another build");
        assert!(other.contains("this session is build abc1234") && other.contains("its supervisor is build def5678"), "{other}");
        let moved = skew(ours, (Some(ours.0), Some("inode 8 of 10 bytes, modified 2.000000000"))).expect("another image of one build");
        assert!(moved.contains("image inode 7") && moved.contains("image inode 8"), "{moved}");
        let old = skew(ours, (None, None)).expect("a supervisor that names no build");
        assert!(old.contains("older than this session (build abc1234)"), "{old}");
        let half = skew(ours, (Some(ours.0), None)).expect("a supervisor that names its build alone");
        assert!(half.contains("did not name both its build and its image"), "{half}");
        let (leash, mut theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        writeln!(theirs, "refused: {other}").unwrap();
        let said = super::told_port(&leash, std::time::Duration::from_millis(200)).await.map(|(p, _)| p);
        assert_eq!(said, Err(format!("it refused: {other}")));
    }

    /// NC-48: the caps never allow more processes than the unit's memory holds.
    #[test]
    /// Sessions first: where they fit alone, they keep their cap and the attempts get
    /// what is left; only where they do not are both lowered in proportion.
    fn the_caps_fit_the_memory_ceiling() {
        use super::fit_caps;
        assert_eq!(fit_caps(64, 256, 16, 4, None), (64, 256), "no ceiling, no change");
        assert_eq!(fit_caps(64, 256, 16, 4, Some(8 << 30)), (64, 256), "8 GiB holds them");
        assert_eq!(fit_caps(64, 512, 16, 4, Some(6 << 30)), (64, 512), "6 GiB holds D-55's 512 beside the sessions: 3,088 MiB");
        assert_eq!(fit_caps(64, 512, 16, 4, Some(8 << 30)), (64, 512), "and 8 GiB");
        assert_eq!(fit_caps(64, 512, 16, 4, Some(2 << 30)), (64, 252), "2 GiB: the sessions, then 252 attempts");
        let (s, a) = fit_caps(64, 256, 16, 4, Some(1 << 30));
        assert!(s as u64 * 16 + a as u64 * 4 + 16 <= 1024 && s >= 1 && a >= 1, "1 GiB holds them and the supervisor: {s} + {a}");
        assert_eq!((s, a), (31, 128), "64 sessions and the supervisor alone do not fit 1 GiB: lowered in proportion");
        assert_eq!(fit_caps(62, 256, 16, 4, Some(1 << 30)), (62, 4), "sessions that fit leave the attempts what is left");
    }
}
