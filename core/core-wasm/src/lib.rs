//! Wasm shell over `pacific-core`.
//!
//! The browser needs three things the core already computes, and each one is a
//! place where a JavaScript reimplementation would be actively dangerous rather
//! than merely duplicative:
//!
//!   * CANONICAL BYTES — the integer-keyed CBOR envelope a delta is hashed and
//!     signed over. The web client had been encoding key-sorted JSON instead,
//!     which produces different bytes for the same delta, which produces a
//!     different id. Two devices would then disagree about what they were both
//!     holding.
//!   * DELTA ID — `sha256(canonical)`, the content address AND the hash-chain
//!     link. `coordinator.rs` is explicit that rewriting a delta's args changes
//!     its id and breaks every delta after it, so an id computed two ways is a
//!     chain that silently forks.
//!   * SAS WORDS — the three read-aloud words two people compare to confirm they
//!     have each other's real key. A browser that derives those words by its own
//!     route and drifts does not fail loudly; it tells two people they match when
//!     they do not. This is the one export where drift is a vulnerability.
//!
//! Follows the `media-wasm` / `viz-wasm` idiom: raw `extern "C"`, ptr/len pairs
//! into linear memory, no bindgen, no dependencies beyond serde. And it follows
//! `media-wasm`'s reason as well as its shape — "a demo that reimplements the
//! thing it is demonstrating proves nothing", which goes double for a client that
//! reimplements the thing it is securing.

#![allow(static_mut_refs)]

/// ENTROPY IS AN IMPORT, NOT A GUESS.
///
/// This shell used to register a backend that REFUSED — every export was
/// deterministic, so a caller reaching for randomness got a loud `UNSUPPORTED`
/// rather than a weak byte. That was right while the shell was arithmetic only.
/// It stops being right the moment a platform wants to mint an MLS KeyPackage or
/// a device keypair, which is the whole of pairing.
///
/// So the bytes come from the host, through a declared import, and the module
/// SAYS SO IN ITS IMPORT LIST: an embedder that does not supply
/// `env.pacific_fill_random` cannot instantiate it at all. That is the property
/// worth having — a missing CSPRNG is a load-time error on every platform rather
/// than a weak key discovered later. The browser backs it with
/// `crypto.getRandomValues`; another host backs it with whatever its own audited
/// CSPRNG is. Neither has to be trusted to remember: the linker asks.
///
/// NOT wasm-bindgen's `js` backend, which is what getrandom would otherwise pull
/// in — 697 KB of describe machinery and four imports, to reach the same
/// `crypto.getRandomValues` this one line reaches.
// `wasm_import_module` is what makes this an IMPORT rather than an undefined
// symbol the linker refuses. Without it rust-lld fails with
// "undefined symbol: pacific_fill_random" — which is the same fact stated as a
// build error instead of a contract.
#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
extern "C" {
    // Fill `len` bytes at `ptr` from the host's CSPRNG. Infallible by contract:
    // a host that cannot produce entropy must TRAP rather than return short,
    // because "fewer bytes than asked for" is indistinguishable from success at
    // this boundary and would silently weaken a key.
    fn pacific_fill_random(ptr: *mut u8, len: usize);
}

#[cfg(target_arch = "wasm32")]
fn host_entropy(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    // SAFETY: `buf` is a live, uniquely-borrowed slice; the host writes exactly
    // `len` bytes into it and returns.
    unsafe { pacific_fill_random(buf.as_mut_ptr(), buf.len()) };
    Ok(())
}
#[cfg(target_arch = "wasm32")]
getrandom::register_custom_getrandom!(host_entropy);

use pacific_core::contact::ContactType;
use pacific_core::wallet::{self, Band, Disclosure, Rule};
use pacific_core::object::{Op, ReduceContext};
use pacific_core::coordinator as coord;
use pacific_core::coordinator::{ArgVal, Args, ConversationType, Coordinator, Delta, ForumType};
use pacific_core::event::EventType;
use pacific_core::group::GroupType;
use pacific_core::head;
use pacific_core::identity;
use pacific_core::object::{Commutativity, ObjectType};
use pacific_core::place::PlaceType;
use pacific_core::post::PostType;
use pacific_core::project::ProjectType;
use pacific_core::thing::ThingType;

// ---------------------------------------------------------------- memory
//
// One buffer in, one buffer out. JS writes UTF-8 or raw bytes into `alloc`'s
// pointer, calls a function, and reads `out_ptr()` for the returned length.

static mut SCRATCH: Vec<u8> = Vec::new();
static mut OUT: Vec<u8> = Vec::new();

#[no_mangle]
pub extern "C" fn alloc(len: u32) -> *mut u8 {
    unsafe {
        SCRATCH = vec![0u8; len as usize];
        SCRATCH.as_mut_ptr()
    }
}

#[no_mangle]
pub extern "C" fn out_ptr() -> *const u8 {
    unsafe { OUT.as_ptr() }
}

fn scratch(len: u32) -> &'static [u8] {
    unsafe { &SCRATCH[..len as usize] }
}
fn give(bytes: Vec<u8>) -> u32 {
    unsafe {
        ERRED = false;
        OUT = bytes;
        OUT.len() as u32
    }
}
/// A refusal puts its reason in the SAME out buffer and returns 0. No host
/// import, so this module has no imports at all — which is both the `media-wasm`
/// idiom and a bug avoided: an `extern "C" fn log` collides with libm's `log`
/// (`(f64) -> f64`) at link time, and the linker says so as a warning rather
/// than an error.
static mut ERRED: bool = false;

fn fail(why: &str) -> u32 {
    unsafe {
        ERRED = true;
        OUT = why.as_bytes().to_vec();
    }
    0
}

/// Whether the last call refused. If it did, the out buffer holds UTF-8 saying why.
#[no_mangle]
pub extern "C" fn erred() -> u32 {
    unsafe { ERRED as u32 }
}

/// The out buffer's length, so a caller that got 0 can still read the reason.
#[no_mangle]
pub extern "C" fn out_len() -> u32 {
    unsafe { OUT.len() as u32 }
}

// ---------------------------------------------------------------- the delta
//
// A delta arrives as JSON because JSON is what the browser has. It is parsed into
// the CORE's `Delta` and everything after that is the core's own code — the
// envelope layout, the key order, the absent-when-default rules, the hash.

#[derive(serde::Deserialize)]
struct Wire {
    type_id: u32,
    op_id: u32,
    #[serde(default = "one")]
    op_version: u32,
    #[serde(default)]
    args: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    epoch: u64,
    /// 64 hex chars. The genesis link is all-zero; `coordinator::GENESIS_PREV`.
    #[serde(default)]
    prev: String,
    #[serde(default)]
    seq: Option<u64>,
    #[serde(default)]
    gen: Option<u64>,
    /// 32-byte hex. Commutative ops are any-member, so a fold with one author is
    /// not a fold at all — this is what lets a fixture have several.
    #[serde(default, skip_serializing)]
    author: Option<String>,
}
fn one() -> u32 {
    1
}

fn build(w: Wire) -> Result<Delta, String> {
    let mut args: Args = Args::new();
    for (k, v) in w.args {
        let val = match v {
            serde_json::Value::String(s) => ArgVal::Text(s),
            serde_json::Value::Number(n) => match n.as_i64() {
                Some(i) => ArgVal::Int(i),
                None => return Err(format!("arg {k}: only integers, not floats")),
            },
            other => return Err(format!("arg {k}: {other} is neither text nor an integer")),
        };
        args.insert(k, val);
    }

    let mut prev = [0u8; 32];
    if !w.prev.is_empty() {
        let raw = hex::decode(&w.prev).map_err(|e| format!("prev: {e}"))?;
        if raw.len() != 32 {
            return Err(format!("prev: {} bytes, want 32", raw.len()));
        }
        prev.copy_from_slice(&raw);
    }

    Ok(Delta {
        type_id: w.type_id,
        op_id: w.op_id,
        op_version: w.op_version,
        args,
        epoch: w.epoch,
        prev,
        seq: w.seq,
        gen: w.gen,
        visibility: Default::default(),
    })
}

fn parse(len: u32) -> Result<Delta, String> {
    let s = std::str::from_utf8(scratch(len)).map_err(|e| format!("not utf-8: {e}"))?;
    let w: Wire = serde_json::from_str(s).map_err(|e| format!("bad delta json: {e}"))?;
    build(w)
}

/// Canonical CBOR of the delta in the scratch buffer. Returns the byte count.
#[no_mangle]
pub extern "C" fn delta_canon(len: u32) -> u32 {
    match parse(len) {
        Ok(d) => give(d.canonical_bytes()),
        Err(e) => fail(&e),
    }
}

/// `sha256(canonical)` as 32 raw bytes — the id and the chain link.
#[no_mangle]
pub extern "C" fn delta_id(len: u32) -> u32 {
    match parse(len) {
        Ok(d) => give(d.id().to_vec()),
        Err(e) => fail(&e),
    }
}

// ---------------------------------------------------------------- the probe
//
// `node::event_author` folds the current state, runs `reduce` on the candidate,
// and propagates the error — "so a precondition-failing delta is refused loudly
// here rather than written as a permanently inert no-op". That asymmetry is the
// whole design: the FOLD tolerates a peer's bad delta (a replica cannot refuse
// history), and the AUTHOR must never write one.
//
// It is also why authoring from the ICD's payload schema produced deltas that
// vanished. The app never had that problem, because it never authors without
// probing first. This is that probe, for a browser.

#[derive(serde::Deserialize)]
struct ProbeIn {
    owner: String,
    #[serde(default)]
    members: Vec<String>,
    /// The log so far — the state the candidate is probed against.
    #[serde(default)]
    deltas: Vec<Wire>,
    /// The op being considered.
    candidate: Wire,
}

/// Would this delta actually do anything? Returns the reduce rejection by name,
/// or ok. Call it before appending; never append without it.
#[no_mangle]
pub extern "C" fn probe_group(len: u32) -> u32 {
    let out: Result<String, String> = (|| {
        let s = std::str::from_utf8(scratch(len)).map_err(|e| format!("not utf-8: {e}"))?;
        let input: ProbeIn = serde_json::from_str(s).map_err(|e| format!("bad probe json: {e}"))?;
        let owner = hex32(&input.owner)?;
        let mut members: Vec<[u8; 32]> = Vec::new();
        for m in &input.members {
            members.push(hex32(m)?);
        }
        if !members.contains(&owner) {
            members.push(owner);
        }

        let mut c: Coordinator<GroupType> = Coordinator::new(members.clone(), owner);
        for w in input.deltas {
            let author = match &w.author {
                Some(a) => hex32(a)?,
                None => owner,
            };
            let d = build(w)?;
            let _ = c.deliver(d, author);
        }

        let author = match &input.candidate.author {
            Some(a) => hex32(a)?,
            None => owner,
        };
        let op_id = input.candidate.op_id;
        let cand = build(input.candidate)?;
        let mut probe = c.state();
        let ctx = ReduceContext {
            members: &members,
            owner,
            epoch: cand.epoch,
        };
        let op = Op {
            op_id,
            args: &cand.args,
            author: &author,
            pos: None,
            ctx: &ctx,
        };
        Ok(match GroupType::reduce(&mut probe, &op) {
            Ok(()) => r#"{"ok":true}"#.to_string(),
            Err(e) => format!(r#"{{"ok":false,"why":"{e:?}"}}"#),
        })
    })();
    match out {
        Ok(j) => give(j.into_bytes()),
        Err(e) => fail(&e),
    }
}

// ---------------------------------------------------------------- money
//
// The four wallet ops, through the constructors just added to `wallet.rs`. The
// browser never formats a band spec or names an arg — `bands` in particular is a
// text mini-format (`ceiling|rule|quorum` per line, `*` for the top band) and
// hand-writing it is exactly what the constructor exists to prevent.

#[derive(serde::Deserialize)]
struct BandIn {
    /// Minor units. Absent means the top band.
    #[serde(default)]
    ceiling: Option<i64>,
    rule: String,
    #[serde(default)]
    quorum: u32,
}

#[derive(serde::Deserialize)]
#[serde(tag = "op")]
enum MoneyIn {
    #[serde(rename = "base.setWalletPolicy")]
    Policy {
        currency: String,
        bands: Vec<BandIn>,
        #[serde(rename = "cooloffHours")]
        cooloff_hours: u32,
        #[serde(default)]
        disclosure: Option<String>,
        #[serde(default)]
        account: Option<String>,
    },
    #[serde(rename = "base.recordDeposit")]
    Deposit {
        reference: String,
        amount: i64,
        at: i64,
        #[serde(default)]
        source: Option<String>,
    },
    #[serde(rename = "base.attestSettlement")]
    Settlement {
        reference: String,
        amount: i64,
        at: i64,
        #[serde(default)]
        proposal: Option<String>,
        #[serde(default)]
        memo: Option<String>,
    },
    #[serde(rename = "base.attestBalance")]
    Balance { amount: i64, at: i64 },
}

/// Build the args for one wallet op. Returns `{op_id, args}` as JSON so the
/// caller can hand it to `probe_group` and then to the envelope builders —
/// the same two steps `node::event_author` takes.
#[no_mangle]
pub extern "C" fn wallet_args(len: u32) -> u32 {
    let out: Result<String, String> = (|| {
        let s = std::str::from_utf8(scratch(len)).map_err(|e| format!("not utf-8: {e}"))?;
        let m: MoneyIn = serde_json::from_str(s).map_err(|e| format!("bad money json: {e}"))?;
        let (op_id, args) = match m {
            MoneyIn::Policy { currency, bands, cooloff_hours, disclosure, account } => {
                let mut bs: Vec<Band> = Vec::new();
                for b in &bands {
                    bs.push(Band {
                        ceiling: b.ceiling,
                        rule: Rule::parse(&b.rule).map_err(|_| {
                            format!("rule '{}': want petty|majority|supermajority|consent", b.rule)
                        })?,
                        quorum: b.quorum,
                    });
                }
                let d = match &disclosure {
                    Some(s) => Some(Disclosure::parse(s).map_err(|_| format!("disclosure '{s}'"))?),
                    None => None,
                };
                (
                    wallet::OP_SET_POLICY,
                    wallet::set_policy_args(&currency, &bs, cooloff_hours, d, account.as_deref()),
                )
            }
            MoneyIn::Deposit { reference, amount, at, source } => (
                wallet::OP_RECORD_DEPOSIT,
                wallet::record_deposit_args(&reference, amount, at, source.as_deref()),
            ),
            MoneyIn::Settlement { reference, amount, at, proposal, memo } => (
                wallet::OP_ATTEST_SETTLEMENT,
                wallet::attest_settlement_args(&reference, amount, at, proposal.as_deref(), memo.as_deref()),
            ),
            MoneyIn::Balance { amount, at } => (
                wallet::OP_ATTEST_BALANCE,
                wallet::attest_balance_args(amount, at),
            ),
        };
        let pairs: Vec<String> = args
            .iter()
            .map(|(k, v)| match v {
                ArgVal::Int(i) => format!("{}:{}", serde_json::to_string(k).unwrap(), i),
                ArgVal::Text(s) => format!(
                    "{}:{}",
                    serde_json::to_string(k).unwrap(),
                    serde_json::to_string(s).unwrap()
                ),
            })
            .collect();
        Ok(format!(r#"{{"op_id":{},"args":{{{}}}}}"#, op_id, pairs.join(",")))
    })();
    match out {
        Ok(j) => give(j.into_bytes()),
        Err(e) => fail(&e),
    }
}

// ---------------------------------------------------------------- authoring
//
// THE APP DOES NOT ASSEMBLE ARGS, AND NEITHER DOES THIS. `node::post_to_group`
// calls `coordinator::forum_post_full(text, gen, epoch, ts, reply_to, media)` and
// hands the result straight to the write path; it never builds an `Args` map and
// never consults the ICD. The core carries a typed constructor per op — the six
// forum/ratify Deltas in `coordinator.rs`, thirty `*_args()` builders elsewhere —
// precisely so a caller cannot get the arguments wrong.
//
// That is the whole answer to why a delta built from the ICD's payload schema is
// accepted and then does nothing: `forum.post` needs `gen` INSIDE the args as
// well as on the envelope, the document does not say so, and `forum_post` knows.
// So the browser calls the constructor, exactly as the phone does.
//
// A delta comes back as canonical CBOR — the bytes that go on the wire and the
// bytes the id is taken over — because that is what the browser should store and
// seal. `decode_delta` reads them back; `delta_id` hashes them.

#[derive(serde::Deserialize)]
struct PostIn {
    text: String,
    /// LAMPORT, not a per-author counter: 1 + the highest gen this author has
    /// seen. The transcript sorts by (gen, author), which makes that a causal
    /// send order — `node.rs` records the bug that taught them so, where a first
    /// post at gen 0 sorted above a peer's earlier gen-0 post.
    gen: u64,
    #[serde(default)]
    epoch: u64,
}

/// `coordinator::forum_post` — the app's own constructor. Returns canonical CBOR.
#[no_mangle]
pub extern "C" fn forum_post(len: u32) -> u32 {
    let parsed: Result<PostIn, String> = (|| {
        let s = std::str::from_utf8(scratch(len)).map_err(|e| format!("not utf-8: {e}"))?;
        serde_json::from_str(s).map_err(|e| format!("bad post json: {e}"))
    })();
    match parsed {
        Ok(p) => give(coord::forum_post(&p.text, p.gen, p.epoch).canonical_bytes()),
        Err(e) => fail(&e),
    }
}

/// `coordinator::next_sequenced_pos` — the (seq, prev) a sequenced delta must
/// claim next, given the spine head. The app does not hand-chain either.
#[derive(serde::Deserialize)]
struct PosIn {
    #[serde(default)]
    head_epoch: Option<u64>,
    #[serde(default)]
    head_seq: Option<u64>,
    #[serde(default)]
    head_id: Option<String>,
    #[serde(default)]
    epoch: u64,
}

#[no_mangle]
pub extern "C" fn next_sequenced_pos(len: u32) -> u32 {
    let out: Result<String, String> = (|| {
        let s = std::str::from_utf8(scratch(len)).map_err(|e| format!("not utf-8: {e}"))?;
        let p: PosIn = serde_json::from_str(s).map_err(|e| format!("bad pos json: {e}"))?;
        let head = match (p.head_epoch, p.head_seq, &p.head_id) {
            (Some(e), Some(sq), Some(id)) => Some((
                pacific_core::object::LogPosition { epoch: e, seq: sq },
                hex32(id)?,
            )),
            _ => None,
        };
        let (seq, prev) = coord::next_sequenced_pos(head, p.epoch);
        Ok(format!(r#"{{"seq":{},"prev":"{}"}}"#, seq, hex::encode(prev)))
    })();
    match out {
        Ok(j) => give(j.into_bytes()),
        Err(e) => fail(&e),
    }
}

// ---------------------------------------------------------------- the catalogue
//
// WHAT THE BROWSER IS ALLOWED TO SAY. Every op in every kind's op table, with its
// id, its authority and which arm of the fold it lands on — read out of the SAME
// `ObjectType::ops()` tables the Coordinator enforces at deliver time.
//
// Not transcribed from the ICD, and not transcribed into JavaScript. The ICD
// (`coordination/delta-graph.icd.json`) is checked against these tables in both
// directions by `icd.rs`: an op declared by a reducer must appear in the
// document, and a message in the document must be a real op. So a browser that
// takes its catalogue from here is in the ICD's parity by construction, and
// there is no third list to keep honest.
//
// WITH ONE HOLE, named rather than papered over (16 Sep 2026). The base RATIFY
// group (`ratify.propose`/`ratify.vote`/`ratify.close`) is in NO op table:
// `Coordinator::deliver` recognises it ahead of the tables and
// `Coordinator::ratify_state` folds it. It is in the ICD, on every channel, and it
// is NOT in what this function returns — so a browser reading its whole vocabulary
// from here cannot author a ratification or even count one. Closing that means
// emitting `coordinator::RATIFY_OPS` beside the per-kind tables, which is a table
// this crate can read; it does NOT mean three rows transcribed into JavaScript.

fn decls<T: ObjectType>() -> String {
    let ops: Vec<String> = T::ops()
        .iter()
        .map(|d| {
            format!(
                r#"{{"op":{},"name":"{}","authority":"{}","fold":"{}"}}"#,
                d.op_id,
                d.name,
                d.authority.web(),
                match d.commutativity {
                    Commutativity::Sequenced => "sequenced",
                    Commutativity::Commutative => "commutative",
                }
            )
        })
        .collect();
    format!(
        r#""{}":{{"type_id":{},"ops":[{}]}}"#,
        T::KIND.name(),
        T::KIND.type_id(),
        ops.join(",")
    )
}

/// WHERE EACH OP MAY BE WRITTEN: `authoring::catalogue`, as JSON
/// `{"<op>": ["<kind>", …]}`. The kinds are the kind STRINGS the door takes, asked
/// of the same check `authoring::build` makes first, so what a consumer is told an
/// op may go on is what the door accepts: a note op is on `notebook` and `note`
/// and not on a Site's `group`; the ratify ops are on every kind; a membership
/// record is on none, and so absent. `ops_catalogue`, below, answers a different
/// question (which op TABLES declare an op) and is not this. Takes no input.
#[no_mangle]
pub extern "C" fn ops_on() -> u32 {
    let map: serde_json::Map<String, serde_json::Value> = pacific_core::authoring::catalogue()
        .into_iter()
        .map(|(d, kinds)| (d.name.to_string(), serde_json::json!(kinds)))
        .collect();
    give(serde_json::Value::Object(map).to_string().into_bytes())
}

/// WHICH ICD CHANNEL EACH KIND STRING WRITES, as JSON `{"<kind>": "<channel>"}`.
///
/// The ICD is keyed by channel — `group`, `forum`, `conversation` — and the
/// directory speaks KIND strings — `connection`, `notebook`, `member-tether`. A
/// consumer that reads the ICD directly needs the map between them, and inventing
/// one is how a page ends up with a second opinion about which ops a kind takes.
/// `authoring::channel_of` is the same dispatch the door itself makes. Takes no
/// input.
#[no_mangle]
pub extern "C" fn lenses() -> u32 {
    let map: serde_json::Map<String, serde_json::Value> = pacific_core::authoring::kinds()
        .into_iter()
        .filter_map(|k| {
            pacific_core::authoring::channel_of(k)
                .map(|o| (k.to_string(), serde_json::json!(o.name())))
        })
        .collect();
    give(serde_json::Value::Object(map).to_string().into_bytes())
}

/// Every op table, as JSON. Takes no input.
#[no_mangle]
pub extern "C" fn ops_catalogue() -> u32 {
    let all = [
        decls::<GroupType>(),
        decls::<ForumType>(),
        decls::<ConversationType>(),
        decls::<EventType>(),
        decls::<ProjectType>(),
        decls::<PlaceType>(),
        decls::<ThingType>(),
        decls::<PostType>(),
        decls::<ContactType>(),
    ];
    give(format!("{{{}}}", all.join(",")).into_bytes())
}

// ---------------------------------------------------------------- the fold
//
// The real one: `Coordinator<GroupType>` over real Deltas, producing the real
// `GroupState`. Not a JavaScript approximation of it, and not the prototype's
// six invented ops over a demo world — those two reduce different things, which
// is why the browser's reducer cannot simply be pointed at this.
//
// The state is rendered here rather than derived by serde, because `GroupState`
// carries no `Serialize` and adding one would be a change to a core type for a
// shell's convenience. Its fields are public; this reads them.

#[derive(serde::Deserialize)]
struct FoldIn {
    /// 32-byte hex, the owner — the only author the sequenced spine accepts.
    owner: String,
    /// 32-byte hex each. The owner is added automatically if omitted.
    #[serde(default)]
    members: Vec<String>,
    deltas: Vec<Wire>,
}

fn hex32(s: &str) -> Result<[u8; 32], String> {
    let raw = hex::decode(s).map_err(|e| format!("{s}: {e}"))?;
    if raw.len() != 32 {
        return Err(format!("{s}: {} bytes, want 32", raw.len()));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&raw);
    Ok(out)
}

fn fold(len: u32) -> Result<String, String> {
    let s = std::str::from_utf8(scratch(len)).map_err(|e| format!("not utf-8: {e}"))?;
    let input: FoldIn = serde_json::from_str(s).map_err(|e| format!("bad fold json: {e}"))?;

    let owner = hex32(&input.owner)?;
    let mut members: Vec<[u8; 32]> = Vec::new();
    for m in &input.members {
        members.push(hex32(m)?);
    }
    if !members.contains(&owner) {
        members.push(owner);
    }

    let mut c: Coordinator<GroupType> = Coordinator::new(members, owner);

    // Deliveries are reported, not swallowed: a rejected delta is the most
    // interesting thing the fold can tell you, and the browser needs to see the
    // same rejection the phone would.
    let mut rejected: Vec<String> = Vec::new();
    for (i, w) in input.deltas.into_iter().enumerate() {
        let author = match &w.author {
            Some(a) => hex32(a)?,
            None => owner,
        };
        let d = build(w)?;
        if let Err(e) = c.deliver(d, author) {
            rejected.push(format!(r#"{{"at":{i},"why":"{e:?}"}}"#));
        }
    }

    let st = c.state();
    // Debug-formatted values can contain quotes (`OnPlatform("ed25519:…")`), so
    // they go through a JSON string encoder rather than into one by hand.
    let q = |s: String| serde_json::to_string(&s).unwrap_or_else(|_| "\"\"".into());
    let roles: Vec<String> = st
        .member_roles
        .iter()
        .map(|(k, v)| format!(r#"{{"member":"{}","role":{}}}"#, hex::encode(k), q(format!("{v:?}"))))
        .collect();
    let affs: Vec<String> = st
        .affiliations
        .keys()
        .map(|k| format!(r#""{k}""#))
        .collect();
    // THE SPINE'S OWN MEMBERSHIPS — the vertebrae. A person's identity record
    // enumerates the objects they belong to, so a device holding nothing but the
    // words folds THIS and reaches the rest of the graph. `kind`, `arc` and `tag`
    // otherwise ride only in a sealed Welcome a returning device never gets, so a
    // browser that could not read them here could not reconstruct anything.
    let joined: Vec<String> = st
        .joined
        .iter()
        .map(|(id, j)| {
            format!(
                r#"{{"object":"{}","kind":{},"arc":{},"tag":"{}","at":{},"left_at":{},"reason":{}}}"#,
                id,
                q(j.kind.clone()),
                q(j.arc.clone()),
                j.tag,
                j.at,
                match j.left_at {
                    Some(v) => v.to_string(),
                    None => "null".to_string(),
                },
                q(j.reason.clone())
            )
        })
        .collect();

    Ok(format!(
        r#"{{"display_name":{},"shape":{},"presence":{},"member_roles":[{}],"affiliations":[{}],"joined":[{}],"rejected":[{}]}}"#,
        q(st.display_name.clone()),
        q(format!("{:?}", st.shape)),
        q(format!("{:?}", st.presence)),
        roles.join(","),
        affs.join(","),
        joined.join(","),
        rejected.join(",")
    ))
}

/// Fold a forum (or a conversation — it shares forum's mechanics, and the ICD
/// says so) through `Coordinator<ForumType>`, returning the folded messages.
///
/// THE ARGS ARE NOT THE ICD'S TO DEFINE. `icd.rs` asserts the op INVENTORY —
/// id, authority, fold arm — and never looks at a message's `payload`. So the
/// document can say `forum.post` requires only `text` while the reducer also
/// demands `gen` in the args, and nothing fails. The reducer is the authority on
/// arguments, which is the whole reason to fold here rather than to validate
/// against the document and hope.
#[no_mangle]
pub extern "C" fn fold_forum(len: u32) -> u32 {
    match fold_forum_inner(len) {
        Ok(json) => give(json.into_bytes()),
        Err(e) => fail(&e),
    }
}

fn fold_forum_inner(len: u32) -> Result<String, String> {
    let s = std::str::from_utf8(scratch(len)).map_err(|e| format!("not utf-8: {e}"))?;
    let input: FoldIn = serde_json::from_str(s).map_err(|e| format!("bad fold json: {e}"))?;
    let owner = hex32(&input.owner)?;
    let mut members: Vec<[u8; 32]> = Vec::new();
    for m in &input.members {
        members.push(hex32(m)?);
    }
    if !members.contains(&owner) {
        members.push(owner);
    }

    let mut c: Coordinator<ForumType> = Coordinator::new(members, owner);
    let mut rejected: Vec<String> = Vec::new();
    for (i, w) in input.deltas.into_iter().enumerate() {
        let author = match &w.author {
            Some(a) => hex32(a)?,
            None => owner,
        };
        let d = build(w)?;
        if let Err(e) = c.deliver(d, author) {
            rejected.push(format!(r#"{{"at":{i},"why":"{e:?}"}}"#));
        }
    }

    let st = c.state();
    let q = |s: String| serde_json::to_string(&s).unwrap_or_else(|_| "\"\"".into());
    // `detailed()` is the core's own read model — the same projection the app
    // binds to, reactions and votes already resolved. Rendering the raw BTreeMap
    // here would be a second view of the same state, which is the habit this
    // whole exercise is trying to break.
    let msgs: Vec<String> = st
        .detailed()
        .iter()
        .map(|m| {
            format!(
                r#"{{"author":"{}","gen":{},"text":{},"up":{},"down":{},"reactions":{}}}"#,
                hex::encode(m.author),
                m.gen,
                q(m.text.clone()),
                m.up.len(),
                m.down.len(),
                m.reactions.len()
            )
        })
        .collect();
    Ok(format!(
        r#"{{"messages":[{}],"rejected":[{}]}}"#,
        msgs.join(","),
        rejected.join(",")
    ))
}

/// Fold a delta sequence through the core's own `Coordinator<GroupType>`.
#[no_mangle]
pub extern "C" fn fold_group(len: u32) -> u32 {
    match fold(len) {
        Ok(json) => give(json.into_bytes()),
        Err(e) => fail(&e),
    }
}

// ---------------------------------------------------------------- identity

/// The three read-aloud SAS words for the 32-byte identity key in the scratch
/// buffer. UTF-8 out.
#[no_mangle]
pub extern "C" fn sas_words(len: u32) -> u32 {
    if len != 32 {
        return fail("sas_words wants exactly 32 bytes");
    }
    let mut pk = [0u8; 32];
    pk.copy_from_slice(scratch(32));
    give(identity::words_from_pk(&pk).into_bytes())
}

/// THE ACCOUNT CHANNEL from a passkey's 32-byte PRF secret. In: the secret.
/// Out: 64 bytes, `tag` then `seal`.
///
/// This is the export that retires `keyholder.js::split()`. The browser and the
/// phone must land on the same tag or they never meet, and — unlike a wrong
/// signature or a bad hash — a wrong tag raises nothing anywhere. Two devices
/// simply sit on different mailboxes in silence. So the derivation is the
/// core's, on both platforms, and neither client computes it.
/// The relay address a 32-byte seed names: seed in, the tag (the address's public
/// key) out, raw. What a browser subscribes to for a channel whose seed it holds.
#[no_mangle]
pub extern "C" fn relay_address(len: u32) -> u32 {
    if len != 32 {
        return fail("relay_address wants exactly 32 bytes of seed");
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(scratch(32));
    give(pacific_wire::address::Address::from_seed(&seed).tag().to_vec())
}

/// Sign a publish. Input `{"seed":"<64 hex>","blob":"<base64 as it will travel>"}`,
/// out `{"tag":"<hex>","sig":"<hex>"}` — the two fields a `pub` frame needs beside
/// the blob. The relay refuses a `pub` its tag did not sign
/// (`pacific_wire::address`), and this is the only way a browser signs one: the
/// rule is the core's, never restated in JavaScript.
#[no_mangle]
pub extern "C" fn relay_sign_pub(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let seed = field32(&v, "seed")?;
        let blob = v["blob"].as_str().ok_or("blob required (base64, exactly as sent)")?;
        let a = pacific_wire::address::Address::from_seed(&seed);
        Ok(format!(r#"{{"tag":"{}","sig":"{}"}}"#, a.tag_hex(), a.sign_pub(blob)))
    })() {
        Ok(s) => give(s.into_bytes()),
        Err(e) => fail(&e),
    }
}

#[no_mangle]
pub extern "C" fn account_channel(len: u32) -> u32 {
    if len != 32 {
        return fail("account_channel wants exactly 32 bytes of PRF secret");
    }
    let mut prf = [0u8; 32];
    prf.copy_from_slice(scratch(32));
    let (tag, seal) = identity::account_channel(&prf);
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(&tag);
    out.extend_from_slice(&seal);
    give(out)
}

/// The hand-paired DEVICE channel, from an X25519 shared secret. Same 64-byte
/// shape as [`account_channel`], different domain — so the two can never be
/// mistaken for one another.
#[no_mangle]
pub extern "C" fn device_channel(len: u32) -> u32 {
    if len != 32 {
        return fail("device_channel wants exactly 32 bytes of shared secret");
    }
    let mut shared = [0u8; 32];
    shared.copy_from_slice(scratch(32));
    let (tag, seal) = identity::device_channel(&shared);
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(&tag);
    out.extend_from_slice(&seal);
    give(out)
}

/// Parse an `ed25519:`-prefixed identity key to its 32 bytes, by the core's own
/// parser — so what the browser accepts and what the app accepts cannot diverge.
#[no_mangle]
pub extern "C" fn parse_identity_key(len: u32) -> u32 {
    let s = match std::str::from_utf8(scratch(len)) {
        Ok(s) => s,
        Err(e) => return fail(&format!("not utf-8: {e}")),
    };
    match identity::parse_identity_key(s) {
        Ok(pk) => give(pk.to_vec()),
        Err(e) => fail(&format!("{e}")),
    }
}

// ---------------------------------------------------------------- vectors
//
// The fixture both sides assert against. These values are produced by the core,
// here, natively; `app/web/docs/pacific-wasm-test.html` loads the wasm build and
// asserts the SAME strings. That is the whole safeguard: if the envelope layout,
// the key order or the hash ever changes, both sides move together or one of
// them goes red.

/// The delta envelopes the conformance vectors are built from, as the JSON the
/// browser sends in. Kept here so the fixture cannot drift from the test.
pub const VECTORS: [&str; 3] = [
    r#"{"type_id":19,"op_id":0,"op_version":1,"args":{"text":"hello"},"epoch":0,"seq":0}"#,
    r#"{"type_id":18,"op_id":1,"op_version":1,"args":{"name":"Cambridge Digital Democracy","shape":"community"},"epoch":7,"seq":3}"#,
    r#"{"type_id":26,"op_id":2,"op_version":1,"args":{"n":42},"epoch":1,"gen":9,"prev":"1111111111111111111111111111111111111111111111111111111111111111"}"#,
];

#[cfg(test)]
mod vectors {
    use super::*;

    fn of(json: &str) -> (String, String) {
        let w: Wire = serde_json::from_str(json).expect("fixture parses");
        let d = build(w).expect("fixture builds");
        (hex::encode(d.canonical_bytes()), hex::encode(d.id()))
    }

    /// Print the fixture so it can be pasted into the browser test, and assert it
    /// against what is already pinned there. Run with --nocapture to regenerate.
    #[test]
    fn fixture_is_stable() {
        for (i, v) in VECTORS.iter().enumerate() {
            let (canon, id) = of(v);
            println!("VECTOR {i}\n  canon {canon}\n  id    {id}");
        }
        // Genesis prev is absent from the JSON and must still be 32 zero bytes in
        // the envelope — the browser cannot be allowed to omit it and agree.
        let (c0, _) = of(VECTORS[0]);
        assert!(c0.contains(&"00".repeat(32)), "genesis prev is 32 zero bytes");
    }

    /// The property that matters more than any single value: the same delta must
    /// hash the same twice, and two deltas differing in one arg must not collide.
    #[test]
    fn ids_are_deterministic_and_distinct() {
        let a = of(VECTORS[0]);
        let b = of(VECTORS[0]);
        assert_eq!(a, b, "same delta, same bytes and id");
        let c = of(VECTORS[1]);
        assert_ne!(a.1, c.1, "different deltas, different ids");
    }
}

// ============================================================== MLS =============
//
// THE BROWSER AS AN MLS LEAF. Everything above this line is arithmetic — canonical
// bytes, ids, folds — and needs no state. This does: a device holds a signing key
// and two storage providers for the life of the page, and `pacific-core`'s own
// `mls.rs` drives them. The protocol is not restated here; `mls.rs` is generic over
// its storage, so the browser gets the phone's MLS by supplying the pair in
// `mls_mem` and nothing else.
//
// PERSISTENCE IS THE HOST'S JOB, and deliberately. `GroupStateStorage` is
// synchronous and IndexedDB is not, so the page calls `mls_snapshot` after any
// mutation and `mls_restore` at startup. The snapshot is SECRET IN FULL — ratchet
// state and epoch secrets — so the host must seal it before it touches a store.

use pacific_core::mls;
use pacific_core::mls_mem::{
    build_client_mem, MemClient, MemGroupStateStorage, MemKeyPackageStorage, Snapshot,
};

struct Device {
    gss: MemGroupStateStorage,
    kps: MemKeyPackageStorage,
    client: MemClient,
    /// Kept so the client can be REBUILT over restored stores. mls-rs clones the
    /// providers into the client, so swapping the stores alone leaves the client
    /// holding the originals and the restore silently does nothing.
    cred: [u8; 32],
    pk: Vec<u8>,
    sk: mls::SecretKey,
    /// THE PERSON, when the host handed over the identity seed. The leaf above
    /// signs MLS handshakes; this signs the Contact Bundle — and a bundle is what
    /// a peer pins, so it must be the identity key and never the leaf's. `None`
    /// when `mls_init` was given a bare `cred`: the leaf still works in every
    /// group, and `mls_contact_bundle` refuses rather than sign with the wrong key.
    identity: Option<identity::Identity>,
    /// The intro-mailbox tag: where a Welcome sealed to this device lands, and the
    /// key it is sealed under (`seal::open(blob, tag, tag)`, exactly as `node.rs`).
    /// Minted here or accepted from the host; either way it rides the bundle.
    intro_tag: [u8; 32],
}

static mut DEVICE: Option<Device> = None;

#[allow(static_mut_refs)]
fn dev() -> Result<&'static mut Device, String> {
    unsafe {
        DEVICE
            .as_mut()
            .ok_or_else(|| "no device — call mls_init first".to_string())
    }
}

fn utf8(len: u32) -> Result<&'static str, String> {
    std::str::from_utf8(scratch(len)).map_err(|e| format!("not utf-8: {e}"))
}

/// Stand up this device's MLS identity. Input:
/// `{"cred":"<64 hex>","seed":"<64 hex>"?,"sk":"<hex>"?,"pk":"<hex>"?,"intro":"<64 hex>"?}`
/// — `cred` is the Ed25519 identity pubkey the leaf credential carries, `sk`/`pk`
/// the MLS signing pair, `seed` the 32 bytes the identity derives from, `intro`
/// the mailbox tag. Omit `sk` to mint a fresh pair and `intro` to mint a tag; the
/// reply returns all of them so the host can persist them. Losing `sk` means a NEW
/// LEAF, not the same device.
///
/// WITH A SEED the device is a person and not only a leaf: `cred` must equal the
/// seed's identity key, refused loudly otherwise. A leaf credentialled as one key
/// and a bundle signed by another is exactly the split a peer's pin exists to
/// catch, and it must not be constructible here. Without a seed the leaf works in
/// every group and `mls_contact_bundle` refuses.
#[no_mangle]
pub extern "C" fn mls_init(len: u32) -> u32 {
    match mls_init_inner(len) {
        Ok(j) => give(j.into_bytes()),
        Err(e) => fail(&e),
    }
}

fn mls_init_inner(len: u32) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
    // SEED FIRST. Only the core can turn a seed into an identity key — WebCrypto
    // cannot run `keys_from_seed` — so a first stand-up may carry a seed and no
    // cred, and we derive it. Given both, they must agree: a mismatch is a page
    // pairing one identity's leaf with another identity's cert, refused loudly.
    let seeded = if v["seed"].is_string() {
        Some(identity::Identity::in_memory(field32(&v, "seed")?))
    } else {
        None
    };
    let cred = match (v["cred"].as_str(), &seeded) {
        (Some(c), Some(id)) => {
            let c = hex32(c)?;
            if c != id.identity_pk() {
                return Err("cred does not match the identity the seed derives".into());
            }
            c
        }
        (Some(c), None) => hex32(c)?,
        (None, Some(id)) => id.identity_pk(),
        (None, None) => return Err("cred or seed required".into()),
    };

    let crypto = mls::crypto();
    let (sk, pk) = match v["sk"].as_str() {
        Some(h) => {
            let raw = hex::decode(h).map_err(|e| format!("bad sk hex: {e}"))?;
            let sk = mls::SecretKey::new(raw);
            let pk_h = v["pk"].as_str().ok_or("pk required when sk is supplied")?;
            (sk, hex::decode(pk_h).map_err(|e| format!("bad pk hex: {e}"))?)
        }
        None => {
            let (sk, pk) = mls::generate_signing_key(&crypto).map_err(|e| e.to_string())?;
            (sk, pk.as_bytes().to_vec())
        }
    };
    // No second agreement check here: by this point `cred` was either checked
    // against the seed above or derived from it, so a mismatch is unreachable.
    // There used to be one, with its own error message, and the test asserted
    // THAT message — so the test could never pass once the check above existed.
    let identity = seeded;
    let intro_tag = if v["intro"].is_string() {
        field32(&v, "intro")?
    } else {
        random32()?
    };

    let gss = MemGroupStateStorage::new();
    let kps = MemKeyPackageStorage::new();
    let client = build_client_mem(
        gss.clone(),
        kps.clone(),
        mls::signing_identity(&cred, &pk),
        sk.clone(),
    )
    .map_err(|e| e.to_string())?;
    unsafe {
        DEVICE = Some(Device {
            gss,
            kps,
            client,
            cred,
            pk: pk.clone(),
            sk: sk.clone(),
            identity,
            intro_tag,
        })
    };
    Ok(format!(
        r#"{{"cred":"{}","pk":"{}","sk":"{}","intro":"{}"}}"#,
        hex::encode(cred),
        hex::encode(&pk),
        hex::encode(sk.as_bytes()),
        hex::encode(intro_tag)
    ))
}

/// Create a group of one. Input: the display name. Returns its group id.
#[no_mangle]
pub extern "C" fn mls_create_group(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let name = utf8(len)?;
        let d = dev()?;
        let g = mls::create_group_named(&d.client, name).map_err(|e| e.to_string())?;
        Ok(format!(r#"{{"group_id":"{}"}}"#, hex::encode(g.group_id())))
    })() {
        Ok(j) => give(j.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// Publish a KeyPackage so someone can add this device to a group.
#[no_mangle]
pub extern "C" fn mls_key_package(_len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let d = dev()?;
        let kp = mls::make_key_package_bytes(&d.client).map_err(|e| e.to_string())?;
        Ok(format!(r#"{{"key_package":"{}"}}"#, hex::encode(kp)))
    })() {
        Ok(j) => give(j.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// Add a member. Input `{"group_id":"<hex>","key_package":"<hex>"}`. Returns the
/// commit (publish to the group's CURRENT tag, claiming its one slot) and the
/// welcome (seal to the newcomer's intro tag — and only after the commit is
/// accepted, RFC 9420 §14).
#[no_mangle]
pub extern "C" fn mls_add_member(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let gid = hex::decode(v["group_id"].as_str().ok_or("group_id required")?)
            .map_err(|e| format!("bad group_id: {e}"))?;
        let kp = hex::decode(v["key_package"].as_str().ok_or("key_package required")?)
            .map_err(|e| format!("bad key_package: {e}"))?;
        let d = dev()?;
        let mut g = mls::load_group(&d.client, &gid).map_err(|e| e.to_string())?;
        let (commit, welcome) = mls::add_member(&mut g, &kp).map_err(|e| e.to_string())?;
        Ok(format!(
            r#"{{"commit":"{}","welcome":"{}","epoch":{}}}"#,
            hex::encode(commit),
            hex::encode(welcome),
            g.current_epoch()
        ))
    })() {
        Ok(j) => give(j.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// Join from a Welcome. Input: the welcome, hex. Returns the group id and epoch.
#[no_mangle]
pub extern "C" fn mls_join(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let w = hex::decode(utf8(len)?.trim()).map_err(|e| format!("bad welcome hex: {e}"))?;
        let d = dev()?;
        let g = mls::join_group(&d.client, &w).map_err(|e| e.to_string())?;
        Ok(format!(
            r#"{{"group_id":"{}","epoch":{},"name":{}}}"#,
            hex::encode(g.group_id()),
            g.current_epoch(),
            serde_json::to_string(&mls::group_name(&g)).unwrap_or_else(|_| "\"\"".into())
        ))
    })() {
        Ok(j) => give(j.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// This epoch's mailbox address and seal secret for a group. The secret is LIVE KEY
/// MATERIAL: it is the HKDF salt for the metadata seal, so anyone holding it plus
/// the tag can strip the seal off a captured relay blob.
#[no_mangle]
pub extern "C" fn mls_epoch_keys(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let gid = hex::decode(utf8(len)?.trim()).map_err(|e| format!("bad group_id: {e}"))?;
        let d = dev()?;
        let g = mls::load_group(&d.client, &gid).map_err(|e| e.to_string())?;
        let eb = mls::epoch_be(g.current_epoch());
        // `address_seed` is the epoch's relay WRITE secret — what `relay_sign_pub`
        // signs with, so a member can publish here and nobody else can. It is
        // exactly as live as `secret` beside it, and for the same holder: the
        // keyholder, which already keeps every epoch's seal secret on its origin.
        let seed = mls::relay_tag(&g, mls::GROUP_LABEL, &eb).map_err(|e| e.to_string())?;
        Ok(format!(
            r#"{{"epoch":{},"tag":"{}","secret":"{}","address_seed":"{}"}}"#,
            g.current_epoch(),
            hex::encode(mls::group_tag(&g, &eb).map_err(|e| e.to_string())?),
            hex::encode(mls::seal_conn_secret(&g, &eb).map_err(|e| e.to_string())?),
            hex::encode(seed)
        ))
    })() {
        Ok(j) => give(j.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// Encrypt a canonical-CBOR delta as an MLS application message.
/// Input `{"group_id":"<hex>","delta":"<hex>"}`.
#[no_mangle]
pub extern "C" fn mls_encrypt(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let gid = hex::decode(v["group_id"].as_str().ok_or("group_id required")?)
            .map_err(|e| format!("bad group_id: {e}"))?;
        let delta = hex::decode(v["delta"].as_str().ok_or("delta required")?)
            .map_err(|e| format!("bad delta: {e}"))?;
        let d = dev()?;
        let mut g = mls::load_group(&d.client, &gid).map_err(|e| e.to_string())?;
        let ct = mls::encrypt_delta(&mut g, &delta).map_err(|e| e.to_string())?;
        Ok(format!(r#"{{"message":"{}"}}"#, hex::encode(ct)))
    })() {
        Ok(j) => give(j.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// Process an inbound MLS message. Input `{"group_id":"<hex>","message":"<hex>"}`.
/// `kind` is "application" | "handshake" | "own".
#[no_mangle]
pub extern "C" fn mls_decrypt(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let gid = hex::decode(v["group_id"].as_str().ok_or("group_id required")?)
            .map_err(|e| format!("bad group_id: {e}"))?;
        let msg = hex::decode(v["message"].as_str().ok_or("message required")?)
            .map_err(|e| format!("bad message: {e}"))?;
        let d = dev()?;
        let mut g = mls::load_group(&d.client, &gid).map_err(|e| e.to_string())?;
        Ok(match mls::decrypt_message(&mut g, &msg).map_err(|e| e.to_string())? {
            mls::Incoming::Application { sender, data } => format!(
                r#"{{"kind":"application","sender":"{}","delta":"{}","epoch":{}}}"#,
                hex::encode(sender),
                hex::encode(data),
                g.current_epoch()
            ),
            // "handshake" stays the kind for an applied commit: the smoke scripts and the
            // keyholder read it (membership-through-mls.md §8.4).
            mls::Incoming::Commit { committer, epoch } => format!(
                r#"{{"kind":"handshake","epoch":{},"committer":"{}"}}"#,
                epoch,
                hex::encode(committer)
            ),
            // A proposal was cached: the epoch did not move, and this device must commit
            // before it can send again (§7) — the keyholder runs its rekey sequence,
            // which sweeps the cache through the same commit rules.
            // `mine`: a removal of one of THIS person's leaves is waiting that will stand —
            // their own leave, proposed from another of their devices, or the owner's
            // Remove. Such a device must not commit: its commit could only drop the
            // proposal or remove its own committer (§6.4, §7). A Remove anyone else
            // proposed does not count; the rules drop it on every commit.
            mls::Incoming::Proposal { sender, removes } => format!(
                r#"{{"kind":"proposal","epoch":{},"sender":"{}","removes":{},"mine":{}}}"#,
                g.current_epoch(),
                hex::encode(sender),
                removes.map(|l| l.to_string()).unwrap_or_else(|| "null".into()),
                mls::pending_removal_of(&g, &d.cred)
            ),
            // A commit removed this device. Its state is not persisted; the keyholder
            // stops watching the group and calls `mls_forget_group` (§8.3, §8.4).
            mls::Incoming::Removed { by } => format!(
                r#"{{"kind":"removed","by":"{}","left":{}}}"#,
                hex::encode(by),
                by == d.cred
            ),
            mls::Incoming::SkippedOwn | mls::Incoming::StaleHandshake => r#"{"kind":"own"}"#.to_string(),
        })
    })() {
        Ok(j) => give(j.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// FORGET a group this device was removed from (membership-through-mls.md §8.3/§8.4):
/// delete its MLS state from the in-memory store — RFC 9420 §12.4.2, a removed member
/// "SHOULD promptly delete its group state and secret tree". In: the group id, hex.
/// Out: `{"forgotten":true|false}` — whether any state was held.
#[no_mangle]
pub extern "C" fn mls_forget_group(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let gid = hex::decode(utf8(len)?.trim()).map_err(|e| format!("bad group_id: {e}"))?;
        let d = dev()?;
        let had = d.gss.delete_group(&gid);
        Ok(format!(r#"{{"forgotten":{had}}}"#))
    })() {
        Ok(j) => give(j.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// The roster, as PERSON identity keys (deduped by `roster_identities`), with the
/// owner and the name the group's MLS context carries. The owner is
/// `mls::group_owner`, which every device computes the same way from the same state
/// (the context's OWNER_EXT, or leaf 0 for a group made before it). So a folding
/// client reads the owner from the group instead of remembering one, and a
/// remembered owner is exactly what goes stale at a handover.
#[no_mangle]
pub extern "C" fn mls_roster(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let gid = hex::decode(utf8(len)?.trim()).map_err(|e| format!("bad group_id: {e}"))?;
        let d = dev()?;
        let g = mls::load_group(&d.client, &gid).map_err(|e| e.to_string())?;
        let r = mls::roster_identities(&g).map_err(|e| e.to_string())?;
        let owner = mls::group_owner(&g).map_err(|e| e.to_string())?;
        let list: Vec<String> = r.iter().map(|m| format!("\"{}\"", hex::encode(m))).collect();
        Ok(format!(
            r#"{{"members":[{}],"owner":"{}","name":{},"epoch":{}}}"#,
            list.join(","),
            hex::encode(owner),
            jstr(&mls::group_name(&g)),
            g.current_epoch()
        ))
    })() {
        Ok(j) => give(j.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// Capture the whole MLS state as CBOR. SECRET IN FULL — seal before storing.
#[no_mangle]
pub extern "C" fn mls_snapshot(_len: u32) -> u32 {
    match (|| -> Result<Vec<u8>, String> {
        let d = dev()?;
        let snap = Snapshot::capture(&d.gss, &d.kps).map_err(|e| e.to_string())?;
        let mut buf = Vec::new();
        ciborium::into_writer(&snap, &mut buf).map_err(|e| format!("encode snapshot: {e}"))?;
        Ok(buf)
    })() {
        Ok(b) => give(b),
        Err(e) => fail(&e),
    }
}

/// Rebuild MLS state from a snapshot. Input: the CBOR blob. Call BEFORE any other
/// MLS call except `mls_init` — restoring over a live client strands what it held.
#[no_mangle]
pub extern "C" fn mls_restore(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let snap: Snapshot = ciborium::from_reader(scratch(len))
            .map_err(|e| format!("decode snapshot: {e}"))?;
        let (gss, kps) = snap.restore();
        let groups = snap.states.len();
        let d = dev()?;
        // Rebuild the CLIENT too, not just the handles — see the note on `Device`.
        let client = build_client_mem(
            gss.clone(),
            kps.clone(),
            mls::signing_identity(&d.cred, &d.pk),
            d.sk.clone(),
        )
        .map_err(|e| e.to_string())?;
        d.gss = gss;
        d.kps = kps;
        d.client = client;
        Ok(format!(r#"{{"groups":{groups}}}"#))
    })() {
        Ok(j) => give(j.into_bytes()),
        Err(e) => fail(&e),
    }
}

// ============================================================== THE ARCHIVE ====
//
// HISTORY FOR A LEAF THAT CANNOT DECRYPT IT. A browser added at epoch N reads
// nothing before N — forward secrecy doing its job — so the past arrives out of
// band as `archive::Archive`: per group, the exact fold inputs and nothing else.
// This export folds it through `archive::fold_group`, which IS the fold the phone
// runs (`object_store::folded` delegates to it), and reports `archive::fold_digest`,
// which IS the digest the phone computes. So "did the history arrive intact" is
// one hex string compared on two devices, and the browser's transcript is the
// phone's transcript rather than a JavaScript reading of the same log.
//
// ONE REFUSED GROUP IS NOT A REFUSED ARCHIVE. Each group folds on its own, and a
// failure — an envelope that will not decode, a delta the spine rejects, a kind
// with no lens — goes into `rejected` with its reason while every other group
// still renders. The alternative, one bad group hiding a whole history, is the
// silent failure this shell exists to avoid; the reason is reported, not swallowed.
//
// The view per lens is the core's own read model — `detailed()` for a forum or a
// conversation, the public fields of `GroupState` / `EventState` for the rest —
// hand-rolled into JSON the way `fold_forum` does it, because `Delta` and
// `ForumMessage` carry no `Serialize` and adding one would be a change to a core
// type for a shell's convenience.

use pacific_core::fold;

/// One log row as the browser hands it over: the sender MLS authenticated and the
/// delta it carried. The artefact this used to borrow from is gone; the fold never
/// needed it.
struct Row {
    author: [u8; 32],
    envelope: Vec<u8>,
}
use pacific_core::coordinator::ForumState;
use pacific_core::event::EventState;
use pacific_core::group::{GroupState, Presence};
use pacific_core::object::ObjectKind;

/// A JSON string literal, quotes and escapes included. Text in a log is the peer's
/// to write, so it never goes into a format string by hand.
fn jstr(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Fold ONE object a device holds live, the object `fold_archive` folds from a
/// backup, and through the same `fold_one`, so a live screen and a restored one
/// cannot disagree about what an object says.
///
/// Input `{"group_id":"<hex>", "kind"?:"<name>", "name"?:"…", "owner":"<hex>",
/// "members":["<hex>"…], "owners"?:[[epoch,"<hex>"]…],
/// "log":[{"author":"<hex>","envelope":"<hex>","sig"?:"<hex>"}…], "reader"?:"<hex>"}`.
/// `reader` is the device's own key, whose reactions a room marks `mine`. Each `log` row
/// is what `mls_decrypt` hands back for an application message, `sender` as
/// `author` and `delta` as `envelope`, so the keyholder stores what it received
/// and never decodes a delta itself. A row with no `sig` is UNPROVEN, not trusted
/// (archive::LogEntry): a device's own live log was authenticated by MLS on
/// arrival, and that is the only log this is for.
///
/// Output: the object exactly as one entry of `fold_archive`'s `groups`, so
/// {id, kind, name, owner, members, digest, view}. A log that will not fold is
/// refused in the reducer's words.
#[no_mangle]
pub extern "C" fn fold_object(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let group_id = hex::decode(v["group_id"].as_str().ok_or("group_id required")?)
            .map_err(|e| format!("bad group_id: {e}"))?;
        let owner = hex32(v["owner"].as_str().ok_or("owner required")?)?;
        let members = v["members"]
            .as_array()
            .ok_or("members required")?
            .iter()
            .map(|m| hex32(m.as_str().ok_or("each member is hex")?))
            .collect::<Result<Vec<_>, _>>()?;
        let owners = match v.get("owners").and_then(|o| o.as_array()) {
            None => Vec::new(),
            Some(rows) => rows
                .iter()
                .map(|r| -> Result<(u64, [u8; 32]), String> {
                    let epoch = r[0].as_u64().ok_or("each owners row is [epoch, \"<hex>\"]")?;
                    Ok((epoch, hex32(r[1].as_str().ok_or("each owners row is [epoch, \"<hex>\"]")?)?))
                })
                .collect::<Result<Vec<_>, _>>()?,
        };
        let log = v["log"]
            .as_array()
            .ok_or("log required")?
            .iter()
            .enumerate()
            .map(|(i, row)| -> Result<Row, String> {
                let author = hex32(row["author"].as_str().ok_or_else(|| format!("log[{i}]: author required"))?)
                    .map_err(|w| format!("log[{i}]: {w}"))?;
                let envelope = hex::decode(
                    row["envelope"].as_str().ok_or_else(|| format!("log[{i}]: envelope required"))?,
                )
                .map_err(|e| format!("log[{i}]: bad envelope: {e}"))?;
                Ok(Row { author, envelope })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (kind, inferred) = match v["kind"].as_str() {
            Some(k) => (k.to_string(), false),
            None => (kind_of_log(&log)?, true),
        };
        let name = v["name"].as_str().unwrap_or("").to_string();
        let reader = v["reader"].as_str().map(hex32).transpose()?;
        let view = fold::view_of(
            &kind,
            owner,
            members.clone(),
            owners.clone(),
            log.iter().map(|r| (r.author, r.envelope.clone())),
            reader.as_ref(),
        )
        .map_err(|e| e.to_string())?;
        let digest = fold::digest_of(
            fold::lens_for(&kind).ok_or_else(|| format!("no lens for kind '{kind}'"))?,
            log.iter().map(|r| r.envelope.as_slice()),
            &members,
        )
        .map_err(|e| e.to_string())?;
        let member_list: Vec<String> =
            members.iter().map(|m| format!("\"{}\"", hex::encode(m))).collect();
        let folded = format!(
            r#"{{"id":"{}","kind":{},"name":{},"owner":"{}","members":[{}],"digest":"{}","view":{}}}"#,
            hex::encode(&group_id),
            jstr(&kind),
            jstr(&name),
            hex::encode(owner),
            member_list.join(","),
            hex::encode(digest),
            view
        );
        if !inferred {
            return Ok(folded);
        }
        // AN INFERRED KIND IS A LENS, NOT A KIND. A type id names the vocabulary,
        // and eight kind strings share two of them: a notebook, a note and the
        // three tethers fold as "group", and a connection as "forum". The view is
        // right either way, since the lens is the same, but the label would claim
        // more than the log can say. So it is marked, and a reader shows the kind
        // as unknown rather than taking the lens's name for it.
        let mut out: serde_json::Value = serde_json::from_str(&folded).map_err(|e| e.to_string())?;
        out["kind_inferred"] = serde_json::Value::Bool(true);
        Ok(out.to_string())
    })() {
        Ok(s) => give(s.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// The kind a live log is, read from its deltas' own type ids. The kind is not
/// in the MLS group context (only the name, the owner and the version are), and
/// a client that decoded deltas to find it would be a second decoder. One type
/// id names one kind; an empty log, or one carrying several, is REFUSED rather
/// than guessed, because a guess folds the object through the wrong lens and an
/// empty fold under the wrong lens is a valid fold: the member sees nothing and
/// is told nothing.
fn kind_of_log(log: &[Row]) -> Result<String, String> {
    let mut ids = std::collections::BTreeSet::new();
    for (i, row) in log.iter().enumerate() {
        let d = coord::decode_delta(&row.envelope).map_err(|e| format!("log[{i}]: {e}"))?;
        ids.insert(d.type_id);
    }
    let mut it = ids.iter();
    match (it.next(), it.next()) {
        (None, _) => Err("an empty log names no kind; pass `kind`".into()),
        (Some(&t), None) => u16::try_from(t)
            .ok()
            .and_then(ObjectKind::from_type_id)
            .map(|k| k.name().to_string())
            .ok_or_else(|| format!("type id {t} is not a kind this core knows")),
        _ => Err(format!(
            "the log carries type ids {ids:?}; one object is one kind, so pass `kind` rather than have it guessed"
        )),
    }
}

// ------------------------------------------------------------ the authoring door
//
// The browser's half of `pacific_core::authoring`, the ONE door every kind is
// written through (e0a9bf3): `mint_ops` says what a mint authors, `author_build`
// builds one delta, positioned, gen'd and probed against the object's log. They
// are pure. The keyholder holds the log and the MLS state, encrypts what comes
// back, and publishes it; nothing here touches the network or a store.

/// ICD args as the core takes them. The ICD's payloads are text or integer and
/// nothing else, so anything else is refused, not coerced.
fn args_of(v: &serde_json::Value) -> Result<Args, String> {
    let mut args = Args::new();
    let Some(obj) = v.as_object() else {
        return if v.is_null() { Ok(args) } else { Err("args is an object".into()) };
    };
    for (k, val) in obj {
        let a = match val {
            serde_json::Value::String(s) => ArgVal::Text(s.clone()),
            serde_json::Value::Number(n) => {
                ArgVal::Int(n.as_i64().ok_or_else(|| format!("arg {k}: only integers, not floats"))?)
            }
            other => return Err(format!("arg {k}: {other} is neither text nor an integer")),
        };
        args.insert(k.clone(), a);
    }
    Ok(args)
}

fn args_json(args: &Args) -> serde_json::Value {
    serde_json::Value::Object(
        args.iter()
            .map(|(k, v)| {
                let j = match v {
                    ArgVal::Text(s) => serde_json::Value::String(s.clone()),
                    ArgVal::Int(i) => serde_json::Value::from(*i),
                    #[allow(unreachable_patterns)]
                    other => serde_json::Value::String(format!("{other:?}")),
                };
                (k.clone(), j)
            })
            .collect(),
    )
}

/// A `MintDraft` from JSON, field by field. AN UNKNOWN FIELD IS REFUSED: a
/// misspelt `startMs` for `start_ms` would otherwise mint an event with no start,
/// and a field quietly dropped is the silent-drop defect this door exists to end.
fn draft_of(v: &serde_json::Value) -> Result<pacific_core::mint::MintDraft, String> {
    const KNOWN: &[&str] = &[
        "name", "descriptor", "form", "link", "shape", "card", "category", "start_ms", "end_ms",
        "venue", "recurrence", "lineup", "placed", "lat", "lng", "status", "icon", "banner",
        "stance", "price", "place",
    ];
    let obj = v.as_object().ok_or("draft is an object")?;
    if let Some(k) = obj.keys().find(|k| !KNOWN.contains(&k.as_str())) {
        return Err(format!("draft: `{k}` is not a MintDraft field"));
    }
    let s = |k: &str| obj.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let bytes = |k: &str| -> Result<Vec<u8>, String> {
        match obj.get(k).and_then(|x| x.as_str()) {
            None | Some("") => Ok(Vec::new()),
            Some(h) => hex::decode(h).map_err(|e| format!("draft: {k} is hex: {e}")),
        }
    };
    Ok(pacific_core::mint::MintDraft {
        name: s("name"),
        descriptor: s("descriptor"),
        form: s("form"),
        link: s("link"),
        shape: s("shape"),
        card: s("card"),
        category: s("category"),
        start_ms: obj.get("start_ms").and_then(|x| x.as_i64()).unwrap_or(0),
        end_ms: obj.get("end_ms").and_then(|x| x.as_i64()).unwrap_or(0),
        venue: s("venue"),
        recurrence: s("recurrence"),
        lineup: s("lineup"),
        placed: obj.get("placed").and_then(|x| x.as_bool()).unwrap_or(false),
        lat: obj.get("lat").and_then(|x| x.as_f64()).unwrap_or(0.0),
        lng: obj.get("lng").and_then(|x| x.as_f64()).unwrap_or(0.0),
        status: s("status"),
        icon: bytes("icon")?,
        banner: bytes("banner")?,
        stance: s("stance"),
        price: s("price"),
        place: s("place"),
    })
}

/// What a mint authors, in order: `authoring::mint_ops`, which is exactly what
/// `Node::mint` authors natively. Input `{"kind":"<name>","draft":{MintDraft}}`.
/// Output `{"ops":[{"op_id":n,"args":{…}}…]}`. EMPTY for a Forum, which is minted
/// name-only (its name rides the MLS context). Refused, with the core's reason,
/// for a kind that is not minted from a draft (a contact is paired, a
/// conversation derived).
#[no_mangle]
pub extern "C" fn mint_ops(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let name = v["kind"].as_str().ok_or("kind required")?;
        let kind = pacific_core::mint::kind_from_name(name)
            .ok_or_else(|| format!("unknown object kind '{name}'"))?;
        let draft = draft_of(&v["draft"])?;
        let ops = pacific_core::authoring::mint_ops(kind, &draft)
            .map_err(|why| format!("{name} objects are not minted from a draft ({why:?})"))?;
        let list: Vec<serde_json::Value> = ops
            .iter()
            .map(|(op_id, args)| serde_json::json!({ "op_id": op_id, "args": args_json(args) }))
            .collect();
        Ok(serde_json::json!({ "ops": list }).to_string())
    })() {
        Ok(s) => give(s.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// The `log` rows `fold_object`, `author_build` and `intro_seal` take:
/// `[{"author":"<hex>","envelope":"<hex>"}…]`, the rows `mls_decrypt` hands back.
fn log_rows(v: &serde_json::Value) -> Result<Vec<Row>, String> {
    v.as_array()
        .ok_or("log required")?
        .iter()
        .enumerate()
        .map(|(i, row)| -> Result<Row, String> {
            Ok(Row {
                author: hex32(row["author"].as_str().ok_or_else(|| format!("log[{i}]: author required"))?)
                    .map_err(|w| format!("log[{i}]: {w}"))?,
                envelope: hex::decode(
                    row["envelope"].as_str().ok_or_else(|| format!("log[{i}]: envelope required"))?,
                )
                .map_err(|e| format!("log[{i}]: bad envelope: {e}"))?,
            })
        })
        .collect()
}

/// Those rows as deltas with their authors — what `authoring::Ctx::log` holds.
fn decoded(rows: &[Row]) -> Result<Vec<(Delta, [u8; 32])>, String> {
    rows.iter()
        .enumerate()
        .map(|(i, r)| {
            coord::decode_delta(&r.envelope)
                .map(|d| (d, r.author))
                .map_err(|e| format!("log[{i}]: {e}"))
        })
        .collect()
}

// ============================================================== PAIRING =========
//
// THE BROWSER AS A PERSON, not only a leaf. The MLS section stands a leaf up from
// a `cred` and a signing key, which is all a leaf needs to sit in a group. Pairing
// needs more: a Contact Bundle is signed by the IDENTITY key — the key a peer
// pins, reads three words of aloud, and later checks a Welcome's roster against —
// so the device must hold the identity itself. It arrives as the 32-byte seed the
// phone's recovery key encodes, and `Identity::in_memory` derives from it the same
// keys the phone derives, so a bundle the browser signs and a bundle the phone
// signs are one person's.
//
// `mls_join_intro` is `node::process_intro_blob` with the storage taken out: the
// same base64, the same seal opened under the intro tag, the same `IntroPayload`,
// the same join, and the SAME two forgery checks. The checks are why this mirrors
// rather than paraphrases — a Welcome sealed to our mailbox names its sender and
// the group's owner, and either one absent from the joined roster is a forgery,
// refused before the host records anything.
//
// `seal_open` / `seal_seal` are the seal itself, thin, so the page can drain a
// group's epoch tag and publish to it without carrying a second AEAD.

use pacific_core::handshake;
use pacific_core::seal;

/// 32 bytes from the CSPRNG — on wasm32 the host import at the top of the file,
/// the same source `mls-rs` draws on; natively the OS.
fn random32() -> Result<[u8; 32], String> {
    let mut t = [0u8; 32];
    getrandom::getrandom(&mut t).map_err(|e| format!("entropy: {e}"))?;
    Ok(t)
}

/// A JSON input's hex field, 32 bytes exactly. The VALUE is never echoed in the
/// error, unlike `hex32` — these are seeds, tags and seal secrets.
fn field32(v: &serde_json::Value, key: &str) -> Result<[u8; 32], String> {
    let s = v[key].as_str().ok_or_else(|| format!("{key} required"))?;
    let raw = hex::decode(s).map_err(|e| format!("bad {key} hex: {e}"))?;
    raw.try_into()
        .map_err(|r: Vec<u8>| format!("{key}: {} bytes, want 32", r.len()))
}

/// A JSON input's hex field, any length.
fn field_hex(v: &serde_json::Value, key: &str) -> Result<Vec<u8>, String> {
    let s = v[key].as_str().ok_or_else(|| format!("{key} required"))?;
    hex::decode(s).map_err(|e| format!("bad {key} hex: {e}"))
}

/// OUR signed Contact Bundle — the paste/QR string a peer scans to pair. Input: the
/// display name. Carries a FRESH one-time KeyPackage, so each call mints a new
/// bundle; the intro tag inside is the device's, from `mls_init`.
#[no_mangle]
pub extern "C" fn mls_contact_bundle(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let name = utf8(len)?;
        let d = dev()?;
        let id = d.identity.as_ref().ok_or(
            "no identity — mls_init was given no seed, and a bundle must be signed by the person, not the leaf",
        )?;
        let kp = mls::make_key_package_bytes(&d.client).map_err(|e| e.to_string())?;
        let bundle = handshake::build_bundle(id, kp, d.intro_tag, name).map_err(|e| e.to_string())?;
        Ok(format!(r#"{{"bundle":{}}}"#, jstr(&bundle.encode().map_err(|e| e.to_string())?)))
    })() {
        Ok(j) => give(j.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// Join from a sealed intro-mailbox blob. Input `{"blob_b64":"…"}` — the relay's
/// `blob` field as published. Returns the joined group and what the Welcome
/// claimed about it, AFTER the roster has vouched for those claims.
///
/// Refused blobs are refused the way the phone refuses them: the MLS state has
/// already joined the group (mls-rs writes on join, and `node.rs` is the same),
/// so a forgery is loud here and the host must not record the group — exactly
/// what `process_intro_blob`'s caller does when it quarantines.
#[no_mangle]
pub extern "C" fn mls_join_intro(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let b64 = v["blob_b64"].as_str().ok_or("blob_b64 required")?;
        let d = dev()?;
        let blob = pacific_wire::blob_unb64(b64).map_err(|e| format!("base64: {e}"))?;
        let payload_bytes = seal::open(&blob, &d.intro_tag, &d.intro_tag).map_err(|e| e.to_string())?;
        let payload = handshake::IntroPayload::decode(&payload_bytes).map_err(|e| e.to_string())?;
        // The object's owner: carried explicitly since any member may perform the
        // add at a public door. An older payload without the field means the sender
        // IS the owner (the pair flow, and every owner-performed add).
        let owner_pk = payload.owner.unwrap_or(payload.scanner_pk);
        let kind = payload.kind.clone().unwrap_or_else(|| "connection".to_string());

        let group = mls::join_group(&d.client, &payload.welcome).map_err(|e| e.to_string())?;

        // Validated rejection (NOT a silent fallback): the sealed Welcome names its
        // sender, and the owner it claims for the group; require BOTH to actually
        // be members of the joined MLS roster, so a forged Welcome to our intro tag
        // can neither smuggle in an absent sender nor install an attacker as owner.
        let roster = mls::roster_identities(&group).map_err(|e| e.to_string())?;
        if !roster.contains(&payload.scanner_pk) {
            return Err("welcome sender is not in the joined roster (possible forgery)".into());
        }
        if !roster.contains(&owner_pk) {
            return Err("welcome names an owner outside the joined roster (possible forgery)".into());
        }
        // THE GEN FLOOR (IntroPayload::gen_watermark). The adder holds the log and
        // this device cannot, by forward secrecy, so the one number it needs in
        // order not to re-mint a gen its own person already used arrives here or
        // not at all. It is handed back rather than dropped, which it used to be:
        // the keyholder keeps it per group, max'd across re-adds as
        // `raise_gen_floor` does natively, and supplies it as the author's
        // watermark. `null` is "no floor", which is right only for a device that
        // holds the history.
        Ok(format!(
            r#"{{"group_id":"{}","epoch":{},"name":{},"kind":{},"owner":"{}","scanner":"{}","scanner_name":{},"gen_watermark":{}}}"#,
            hex::encode(group.group_id()),
            group.current_epoch(),
            jstr(&mls::group_name(&group)),
            jstr(&kind),
            hex::encode(owner_pk),
            hex::encode(payload.scanner_pk),
            jstr(&payload.scanner_name),
            payload.gen_watermark.map(|g| g.to_string()).unwrap_or_else(|| "null".into())
        ))
    })() {
        Ok(j) => give(j.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// Read a peer's Contact Bundle — the string `mls_contact_bundle` makes — and check
/// its signature. Input `{"bundle":"…"}`. Out
/// `{"identity":"<hex>","key_package":"<hex>","intro_tag":"<hex>","display_name":"…"}`.
///
/// A bare KeyPackage is enough to ADD someone and not enough to REACH them: the
/// intro tag a Welcome is delivered to, carrying the gen floor, is only in the
/// bundle. An invite built from a KeyPackage alone is how a browser came to join a
/// group with no floor, and so could never write a commutative op in it.
#[no_mangle]
pub extern "C" fn bundle_read(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let b = handshake::parse_and_verify(v["bundle"].as_str().ok_or("bundle required")?)
            .map_err(|e| e.to_string())?;
        Ok(serde_json::json!({
            "identity": hex::encode(b.identity_pk),
            "key_package": hex::encode(&b.key_package),
            "intro_tag": hex::encode(b.intro_tag),
            "display_name": b.display_name,
        })
        .to_string())
    })() {
        Ok(s) => give(s.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// Seal a Welcome to a newcomer's intro mailbox, carrying the GEN FLOOR: the
/// adder's half of what `mls_join_intro` opens, and what the phone's
/// `group_add_member` sends. Input
/// `{"bundle":"…","welcome":"<hex>","kind":"…","owner":"<hex>","name":"…","arc":"…"?,
///   "log":[{"author":"<hex>","envelope":"<hex>"}…],"floor":n|null}`.
/// Out `{"tag":"<hex>","address_seed":"<hex>","blob":"<base64>","gen_watermark":n}`:
/// publish `blob` at `tag`, signed with `address_seed` through `relay_sign_pub`.
///
/// THE FLOOR IS THE CORE'S. `authoring::next_gen` over the log this device holds,
/// raised to its own floor: the number it would give its own next commutative op.
/// `floor` null is a device that joined without one. It hands what its log shows,
/// as the phone does when it holds no floor of its own.
///
/// ORDER. Seal this only once the add's commit has been accepted: a Welcome is
/// valid only for a commit the group took (RFC 9420 §14).
#[no_mangle]
pub extern "C" fn intro_seal(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let d = dev()?;
        let me = d
            .identity
            .as_ref()
            .ok_or("no identity — an intro names its sender, and mls_init was given no seed")?
            .identity_pk();
        let b = handshake::parse_and_verify(v["bundle"].as_str().ok_or("bundle required")?)
            .map_err(|e| e.to_string())?;
        let welcome = hex::decode(v["welcome"].as_str().ok_or("welcome required")?)
            .map_err(|e| format!("bad welcome: {e}"))?;
        let kind = v["kind"].as_str().ok_or("kind required")?.to_string();
        let owner = hex32(v["owner"].as_str().ok_or("owner required")?)?;
        let name = v["name"].as_str().ok_or("name required: the sender's display name")?.to_string();
        let log = decoded(&log_rows(&v["log"])?)?;
        let floor = match &v["floor"] {
            serde_json::Value::Null => 0,
            f => f.as_u64().ok_or("floor is a number, or null")?,
        };
        let gen_watermark = pacific_core::authoring::next_gen(&log, floor);
        let payload = handshake::IntroPayload {
            scanner_pk: me,
            scanner_name: name,
            welcome,
            why: None,
            kind: Some(kind),
            arc: v["arc"].as_str().map(str::to_string),
            owner: Some(owner),
            gen_watermark: Some(gen_watermark),
        }
        .encode()
        .map_err(|e| e.to_string())?;
        let sealed = seal::seal(&payload, &b.intro_tag, &b.intro_tag).map_err(|e| e.to_string())?;
        Ok(serde_json::json!({
            "tag": pacific_wire::address::Address::from_seed(&b.intro_tag).tag_hex(),
            "address_seed": hex::encode(b.intro_tag),
            "blob": pacific_wire::blob_b64(&sealed),
            "gen_watermark": gen_watermark,
        })
        .to_string())
    })() {
        Ok(s) => give(s.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// `seal::open`. Input `{"blob":"<hex>","tag":"<64 hex>","secret":"<64 hex>"}`;
/// out: the inner bytes, hex. Loud on a wrong key or a tampered blob.
#[no_mangle]
pub extern "C" fn seal_open(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let blob = field_hex(&v, "blob")?;
        let tag = field32(&v, "tag")?;
        let secret = field32(&v, "secret")?;
        Ok(hex::encode(seal::open(&blob, &tag, &secret).map_err(|e| e.to_string())?))
    })() {
        Ok(h) => give(h.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// `seal::seal`. Input `{"inner":"<hex>","tag":"<64 hex>","secret":"<64 hex>"}`;
/// out: `nonce(24) || ciphertext`, hex — the relay's blob, before base64.
#[no_mangle]
pub extern "C" fn seal_seal(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let inner = field_hex(&v, "inner")?;
        let tag = field32(&v, "tag")?;
        let secret = field32(&v, "secret")?;
        Ok(hex::encode(seal::seal(&inner, &tag, &secret).map_err(|e| e.to_string())?))
    })() {
        Ok(h) => give(h.into_bytes()),
        Err(e) => fail(&e),
    }
}

// ---------------------------------------------------------------- native proof
//
// The exports, driven natively the way the page drives them — bytes into the
// scratch buffer, a length in, the out buffer read back. The archive test builds
// a real archive out of the core's own constructors and, when
// `ARCHIVE_FIXTURE_DIR` is set, writes it beside what this fold said about it, so
// the wasm build can be checked against the SAME bytes: native and wasm must
// agree to the byte on every digest and every message, or one of them has drifted.

#[cfg(test)]
mod exports {
    use super::*;
    use pacific_core::coordinator::{forum_post_full, forum_react, forum_vote, sequenced_delta, GENESIS_PREV};
    use pacific_core::mls_mem::{build_client_mem, MemGroupStateStorage, MemKeyPackageStorage};
    use pacific_core::{event, group, parts, post};
    use std::sync::Mutex;

    /// The exports share one scratch, one out buffer and one DEVICE. A page is
    /// single-threaded; the test runner is not. `pub(super)` so the backup
    /// tests at the end of the file hold this same lock.
    pub(super) static LOCK: Mutex<()> = Mutex::new(());

    fn call(f: extern "C" fn(u32) -> u32, input: &[u8]) -> Result<Vec<u8>, String> {
        let p = alloc(input.len() as u32);
        // SAFETY: `alloc` just sized SCRATCH to exactly `input.len()` bytes.
        unsafe { std::ptr::copy_nonoverlapping(input.as_ptr(), p, input.len()) };
        let _ = f(input.len() as u32);
        let out = unsafe { OUT.clone() };
        if erred() != 0 {
            Err(String::from_utf8_lossy(&out).into_owned())
        } else {
            Ok(out)
        }
    }
    fn call_json(f: extern "C" fn(u32) -> u32, input: &str) -> Result<serde_json::Value, String> {
        let out = call(f, input.as_bytes())?;
        serde_json::from_slice(&out).map_err(|e| format!("reply is not json: {e}: {}", String::from_utf8_lossy(&out)))
    }
    fn text_args(pairs: &[(&str, &str)]) -> Args {
        let mut a = Args::new();
        for (k, v) in pairs {
            a.insert((*k).into(), ArgVal::Text((*v).into()));
        }
        a
    }
    /// A keyholder row: what `mls_decrypt` hands back for an application message,
    /// `sender` as `author` and `delta` as `envelope`.
    fn live_row(author: [u8; 32], d: &Delta) -> serde_json::Value {
        serde_json::json!({ "author": hex::encode(author), "envelope": hex::encode(d.canonical_bytes()) })
    }
    /// Ops onto one object, in order, each through the door (`authoring::build`)
    /// against the log so far, `me` its owner.
    fn through_the_door(kind: &str, me: [u8; 32], members: &[[u8; 32]], ops: Vec<(u32, Args)>) -> Vec<(Delta, [u8; 32])> {
        use pacific_core::authoring::{build, Ctx};
        let owners = [(0u64, me)];
        let mut log: Vec<(Delta, [u8; 32])> = Vec::new();
        for (op_id, args) in ops {
            let ctx = Ctx { me, epoch: 0, members, owners: &owners, log: &log, watermark: Some(0) };
            let d = build(kind, op_id, args, &ctx).unwrap_or_else(|r| panic!("{kind} op {op_id}: {r}"));
            log.push((d, me));
        }
        log
    }

    fn group_by_id<'a>(v: &'a serde_json::Value, id: &[u8]) -> &'a serde_json::Value {
        v["groups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["id"] == hex::encode(id))
            .unwrap_or_else(|| panic!("group {} not in reply", hex::encode(id)))
    }

    /// A commutative Delta, the arm `forum_post_full` builds on: no `seq`, a
    /// `gen` stamp, no chain to the spine. The note and ledger ops are declared
    /// any-member/commutative, so this is how they arrive.
    fn commutative(op_id: u32, args: Args, gen: u64) -> Delta {
        Delta {
            type_id: ObjectKind::Group.type_id() as u32,
            op_id,
            op_version: 1,
            args,
            epoch: 0,
            prev: GENESIS_PREV,
            seq: None,
            gen: Some(gen),
            visibility: Default::default(),
        }
    }

    #[test]
    fn fold_object_reads_an_event_with_its_lineup_and_tickets() {
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let ada = identity::Identity::in_memory([0x11; 32]).identity_pk();
        let tid = ObjectKind::Event.type_id() as u32;
        let e0 = sequenced_delta(
            tid,
            event::OP_SET_PROFILE,
            event::set_profile_args(
                "Prodigal at the Hall",
                Some("Doors at eight"),
                1_760_000_000_000,
                None,
                Some("The Hall"),
                None,
                Some("Prodigal\nSupport act"),
            ),
            0,
            0,
            GENESIS_PREV,
        );
        let e1 = sequenced_delta(
            tid,
            event::OP_SET_TICKETS,
            event::set_tickets_args(0, "gbp", 120, true, None, None, None, 1),
            0,
            1,
            e0.id(),
        );
        // No `kind`: a live group's kind is not in its MLS context, so it is read
        // from the deltas' own type id.
        let v = call_json(
            fold_object,
            &serde_json::json!({
                "group_id": hex::encode([0xE7; 16]),
                "owner": hex::encode(ada),
                "members": [hex::encode(ada)],
                "log": [live_row(ada, &e0), live_row(ada, &e1)],
            })
            .to_string(),
        )
        .expect("fold_object");
        assert_eq!(v["kind"], "event", "the kind is read from the log");
        assert_eq!(v["kind_inferred"], true, "and says it was read from the log, not known");
        assert_eq!(v["view"]["title"], "Prodigal at the Hall");
        assert_eq!(v["view"]["lineup"], serde_json::json!(["Prodigal", "Support act"]));
        assert_eq!(v["view"]["tickets"]["capacity"], 120);
        assert_eq!(v["view"]["tickets"]["price_cents"], 0, "a free listing, not a missing one");
        assert_eq!(v["view"]["tickets"]["open"], true);
        assert_eq!(v["view"]["media"], serde_json::Value::Null, "no poster set, and it says so");
    }

    #[test]
    fn fold_object_reads_a_post() {
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let ada = identity::Identity::in_memory([0x11; 32]).identity_pk();
        let p0 = sequenced_delta(
            ObjectKind::Post.type_id() as u32,
            post::OP_SET_PROFILE,
            post::set_profile_args("On the road", Some("Three nights, three towns."), Some(post::Form::Article), None),
            0,
            0,
            GENESIS_PREV,
        );
        let v = call_json(
            fold_object,
            &serde_json::json!({
                "group_id": hex::encode([0x70; 16]),
                "owner": hex::encode(ada),
                "members": [hex::encode(ada)],
                "log": [live_row(ada, &p0)],
            })
            .to_string(),
        )
        .expect("fold_object");
        assert_eq!(v["kind"], "post");
        assert_eq!(v["view"]["title"], "On the road");
        assert_eq!(v["view"]["body"], "Three nights, three towns.");
        assert_eq!(v["view"]["form"], "article");
        assert_eq!(v["view"]["retracted"], false);
        assert!(v["view"].get("unsupported").is_none(), "a post is read, not skipped");
    }

    /// A guessed kind folds through the wrong lens, and an empty fold is a valid
    /// fold, so the member would see nothing and be told nothing. Refused instead.
    #[test]
    fn fold_object_refuses_to_guess_a_kind() {
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let ada = identity::Identity::in_memory([0x11; 32]).identity_pk();
        let base = |log: serde_json::Value| {
            serde_json::json!({ "group_id": hex::encode([0x01; 16]), "owner": hex::encode(ada),
                                "members": [hex::encode(ada)], "log": log })
            .to_string()
        };
        let empty = call_json(fold_object, &base(serde_json::json!([]))).unwrap_err();
        assert!(empty.contains("names no kind"), "{empty}");

        let ev = sequenced_delta(
            ObjectKind::Event.type_id() as u32,
            event::OP_SET_PROFILE,
            event::set_profile_args("x", None, 1, None, None, None, None),
            0,
            0,
            GENESIS_PREV,
        );
        let po = sequenced_delta(
            ObjectKind::Post.type_id() as u32,
            post::OP_SET_PROFILE,
            post::set_profile_args("y", None, None, None),
            0,
            0,
            GENESIS_PREV,
        );
        let mixed = call_json(fold_object, &base(serde_json::json!([live_row(ada, &ev), live_row(ada, &po)])))
            .unwrap_err();
        assert!(mixed.contains("one object is one kind"), "{mixed}");

        let garbage = call_json(fold_object, &base(serde_json::json!([{ "author": hex::encode(ada), "envelope": "00ff" }])))
            .unwrap_err();
        assert!(garbage.starts_with("log[0]:"), "a bad row is named by its place: {garbage}");
    }

    #[test]
    fn mint_ops_is_what_the_native_mint_authors() {
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let ev = call_json(
            mint_ops,
            r#"{"kind":"event","draft":{"name":"Gig","start_ms":1760000000000,"venue":"The Hall"}}"#,
        )
        .unwrap();
        let ops = ev["ops"].as_array().unwrap();
        assert_eq!(ops.len(), 1, "an event mints its profile and nothing else");
        assert_eq!(ops[0]["op_id"], event::OP_SET_PROFILE);
        assert_eq!(ops[0]["args"]["title"], "Gig");

        let forum = call_json(mint_ops, r#"{"kind":"forum","draft":{"name":"Chat"}}"#).unwrap();
        assert_eq!(forum["ops"], serde_json::json!([]), "a forum is minted name-only");

        let contact = call_json(mint_ops, r#"{"kind":"contact","draft":{"name":"x"}}"#).unwrap_err();
        assert!(contact.contains("not minted from a draft"), "{contact}");

        // The silent drop this door exists to end: a misspelt field is refused.
        let typo = call_json(mint_ops, r#"{"kind":"event","draft":{"name":"Gig","startMs":1}}"#).unwrap_err();
        assert!(typo.contains("`startMs` is not a MintDraft field"), "{typo}");
    }

    /// Mint, then write, then fold, entirely through the core: `mint_ops`' op 0 goes
    /// through the door, each delta is built against the log so far, and the fold
    /// reads what the door wrote.
    #[test]
    fn a_minted_event_goes_through_the_door_and_the_fold_reads_it() {
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let ada = identity::Identity::in_memory([0x11; 32]).identity_pk();
        let a = hex::encode(ada);
        let op0 = call_json(
            mint_ops,
            r#"{"kind":"event","draft":{"name":"Prodigal live","start_ms":1760000000000}}"#,
        )
        .unwrap()["ops"][0]
            .clone();
        let log = through_the_door("event", ada, &[ada], vec![
            (op0["op_id"].as_u64().unwrap() as u32, args_of(&op0["args"]).unwrap()),
            (event::OP_SET_TICKETS, event::set_tickets_args(0, "gbp", 200, true, None, None, None, 1)),
        ]);

        let (d0, d1) = (&log[0].0, &log[1].0);
        assert_eq!(d0.seq, Some(0), "op 0 is the first position");
        assert_eq!(d1.seq, Some(1), "the next op is positioned after it, from the log");
        assert_eq!(d1.prev, d0.id(), "and chained to it");

        let rows: Vec<_> = log.iter().map(|(d, who)| live_row(*who, d)).collect();
        let v = call_json(
            fold_object,
            &serde_json::json!({ "group_id": hex::encode([0xEE; 16]), "owner": a, "members": [a], "log": rows })
                .to_string(),
        )
        .unwrap();
        assert_eq!(v["view"]["title"], "Prodigal live");
        assert_eq!(v["view"]["tickets"]["capacity"], 200);
    }

    /// A Site's view carries its edges: offices, affiliations and the forums it
    /// hosts. A forum's view carries its rooms. Each is held to the op that set it,
    /// written through the door and read back through the fold (thedoor, 22 Sep: the
    /// band table, the webring and the guestbook had nowhere to come from).
    #[test]
    fn a_site_view_carries_its_offices_affiliations_and_forums_and_a_forum_its_rooms() {
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let band_pk = identity::Identity::in_memory([0x41; 32]).identity_pk();
        let fan_pk = identity::Identity::in_memory([0x42; 32]).identity_pk();
        let (band, fan) = (hex::encode(band_pk), hex::encode(fan_pk));
        let (forum, room, peer) = (hex::encode([0xF0; 16]), hex::encode([0xF1; 16]), hex::encode([0xF2; 16]));

        // Author a run of ops on one object through the door, then fold it.
        let run = |kind: &str, ops: Vec<(u32, serde_json::Value)>| -> serde_json::Value {
            let ops = ops.into_iter().map(|(op_id, a)| (op_id, args_of(&a).unwrap())).collect();
            let log: Vec<_> = through_the_door(kind, band_pk, &[band_pk, fan_pk], ops)
                .iter()
                .map(|(d, who)| live_row(*who, d))
                .collect();
            call_json(fold_object, &serde_json::json!({
                "group_id": hex::encode([0xEE; 16]), "owner": band, "members": [band, fan], "kind": kind, "log": log,
            }).to_string()).unwrap()
        };

        let site = run("group", vec![
            (group::OP_SET_PROFILE, serde_json::json!({ "displayName": "Prodigal", "shape": "team" })),
            (group::OP_SET_OFFICE, serde_json::json!({ "office": "webmaster", "holder": fan, "status": "pending", "at": 1 })),
            (group::OP_SET_AFFILIATION, serde_json::json!({ "peer": peer, "rel": "peer", "name": "the webring", "at": 2 })),
            (parts::OP_SET_PART, serde_json::json!({ "part": forum, "role": "guestbook", "at": 3 })),
        ]);
        let v = &site["view"];
        assert_eq!(v["offices"], serde_json::json!([{ "office": "webmaster", "holder": fan, "status": "pending", "at": 1 }]));
        assert_eq!(v["affiliations"], serde_json::json!([{ "peer": peer, "rel": "peer", "name": "the webring", "tether": "", "at": 2 }]));
        assert_eq!(v["parts"], serde_json::json!([{ "part": forum, "role": "guestbook", "at": 3 }]));

        let book = run("forum", vec![
            (parts::OP_SET_PART, serde_json::json!({ "part": room, "role": "room", "at": 4 })),
        ]);
        assert_eq!(book["view"]["parts"], serde_json::json!([{ "part": room, "role": "room", "at": 4 }]));
        assert_eq!(book["view"]["messages"], serde_json::json!([]), "and its messages, still, beside them");
    }

    #[test]
    fn a_seed_makes_a_person_and_the_bundle_verifies() {
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let seed = [0x33u8; 32];
        let id = identity::Identity::in_memory(seed);
        let cred = hex::encode(id.identity_pk());

        // the wrong cred is refused before anything is stood up.
        let err = call_json(mls_init, &format!(r#"{{"cred":"{}","seed":"{}"}}"#, "ab".repeat(32), hex::encode(seed))).unwrap_err();
        assert!(err.contains("does not match the identity the seed derives"), "{err}");

        // minted intro tag: 32 bytes, and returned.
        let r = call_json(mls_init, &format!(r#"{{"cred":"{cred}","seed":"{}"}}"#, hex::encode(seed))).unwrap();
        assert_eq!(r["cred"], cred);
        assert_eq!(r["intro"].as_str().unwrap().len(), 64);
        let intro = r["intro"].as_str().unwrap().to_string();

        // a bundle: self-signed by the identity, carrying that tag, and it verifies.
        let b = call_json(mls_contact_bundle, "Ada").unwrap();
        let parsed = handshake::parse_and_verify(b["bundle"].as_str().unwrap()).expect("bundle verifies");
        assert_eq!(parsed.identity_pk, id.identity_pk());
        assert_eq!(hex::encode(parsed.intro_tag), intro);
        assert_eq!(parsed.display_name, "Ada");
        assert_eq!(parsed.next_key_commit, id.next_key_commit());
        assert!(!parsed.key_package.is_empty());

        // an accepted intro tag comes back verbatim, with the same keys.
        let r2 = call_json(
            mls_init,
            &format!(
                r#"{{"cred":"{cred}","seed":"{}","sk":"{}","pk":"{}","intro":"{}"}}"#,
                hex::encode(seed),
                r["sk"].as_str().unwrap(),
                r["pk"].as_str().unwrap(),
                "44".repeat(32)
            ),
        )
        .unwrap();
        assert_eq!(r2["intro"], "44".repeat(32));
        assert_eq!(r2["pk"], r["pk"]);

        // no seed: a leaf, and no bundle.
        call_json(mls_init, &format!(r#"{{"cred":"{cred}"}}"#)).unwrap();
        let err = call_json(mls_contact_bundle, "Ada").unwrap_err();
        assert!(err.contains("no identity"), "{err}");
    }

    /// The scanner, natively: a client of its own, the way a phone is its own.
    fn scanner(cred: [u8; 32]) -> pacific_core::mls_mem::MemClient {
        let (sk, pk) = mls::generate_signing_key(&mls::crypto()).unwrap();
        build_client_mem(
            MemGroupStateStorage::new(),
            MemKeyPackageStorage::new(),
            mls::signing_identity(&cred, pk.as_bytes()),
            sk,
        )
        .unwrap()
    }

    fn intro_blob(payload: &handshake::IntroPayload, tag: &[u8; 32]) -> String {
        let sealed = seal::seal(&payload.encode().unwrap(), tag, tag).unwrap();
        pacific_wire::blob_b64(&sealed)
    }

    #[test]
    fn join_intro_mirrors_the_phone_including_the_forgery_checks() {
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // Bea is this device.
        let bea_seed = [0x22u8; 32];
        let bea = identity::Identity::in_memory(bea_seed).identity_pk();
        let r = call_json(mls_init, &format!(r#"{{"cred":"{}","seed":"{}"}}"#, hex::encode(bea), hex::encode(bea_seed))).unwrap();
        let mut intro = [0u8; 32];
        intro.copy_from_slice(&hex::decode(r["intro"].as_str().unwrap()).unwrap());

        // Ada scans: her own client, a named group, Bea's KeyPackage added.
        let ada = identity::Identity::in_memory([0x11u8; 32]).identity_pk();
        let ada_client = scanner(ada);
        let kp = hex::decode(call_json(mls_key_package, "").unwrap()["key_package"].as_str().unwrap()).unwrap();
        let mut g = mls::create_group_named(&ada_client, "Tuesday club").unwrap();
        let (_commit, welcome) = mls::add_member(&mut g, &kp).unwrap();
        let payload = handshake::IntroPayload {
            scanner_pk: ada,
            scanner_name: "Ada".into(),
            welcome,
            why: None,
            kind: Some("forum".into()),
            arc: None,
            owner: Some(ada),
            // A forgery-refusal test: it never authors, so it has no mark to carry.
            gen_watermark: None,
        };

        // sealed to the wrong tag: the seal refuses before any join.
        let err = call_json(mls_join_intro, &format!(r#"{{"blob_b64":"{}"}}"#, intro_blob(&payload, &[0x99; 32]))).unwrap_err();
        assert!(err.contains("seal open failed"), "{err}");
        // not base64 at all.
        let err = call_json(mls_join_intro, r#"{"blob_b64":"%%%"}"#).unwrap_err();
        assert!(err.contains("base64"), "{err}");

        // the real thing.
        let j = call_json(mls_join_intro, &format!(r#"{{"blob_b64":"{}"}}"#, intro_blob(&payload, &intro))).unwrap();
        assert_eq!(j["group_id"], hex::encode(g.group_id()));
        assert_eq!(j["name"], "Tuesday club");
        assert_eq!(j["kind"], "forum");
        assert_eq!(j["owner"], hex::encode(ada));
        assert_eq!(j["scanner"], hex::encode(ada));
        assert_eq!(j["scanner_name"], "Ada");
        assert_eq!(j["epoch"], g.current_epoch());
        assert_eq!(j["gen_watermark"], serde_json::Value::Null, "no mark carried, none claimed");
        // and the roster the device now holds is the two of them.
        let roster = call_json(mls_roster, &hex::encode(g.group_id())).unwrap();
        let members: Vec<String> = roster["members"].as_array().unwrap().iter().map(|m| m.as_str().unwrap().to_string()).collect();
        assert!(members.contains(&hex::encode(ada)) && members.contains(&hex::encode(bea)));
        // The owner and name come from the group's own context, not from anything
        // the joiner was told and kept.
        assert_eq!(roster["owner"], hex::encode(ada), "the owner is read from the MLS context");
        assert_eq!(roster["name"], "Tuesday club");

        // FORGERY 1: a sender the roster does not contain.
        let kp2 = hex::decode(call_json(mls_key_package, "").unwrap()["key_package"].as_str().unwrap()).unwrap();
        let mut g2 = mls::create_group_named(&ada_client, "forged").unwrap();
        let (_c, welcome2) = mls::add_member(&mut g2, &kp2).unwrap();
        let forged = handshake::IntroPayload { scanner_pk: [0x55; 32], welcome: welcome2, owner: Some(ada), ..payload.clone() };
        let err = call_json(mls_join_intro, &format!(r#"{{"blob_b64":"{}"}}"#, intro_blob(&forged, &intro))).unwrap_err();
        assert!(err.contains("sender is not in the joined roster"), "{err}");

        // FORGERY 2: a real sender naming an owner the roster does not contain.
        let kp3 = hex::decode(call_json(mls_key_package, "").unwrap()["key_package"].as_str().unwrap()).unwrap();
        let mut g3 = mls::create_group_named(&ada_client, "forged owner").unwrap();
        let (_c, welcome3) = mls::add_member(&mut g3, &kp3).unwrap();
        let forged = handshake::IntroPayload { welcome: welcome3, owner: Some([0x66; 32]), ..payload.clone() };
        let err = call_json(mls_join_intro, &format!(r#"{{"blob_b64":"{}"}}"#, intro_blob(&forged, &intro))).unwrap_err();
        assert!(err.contains("owner outside the joined roster"), "{err}");
    }

    /// G1, the receiving half. A browser that joins late has an empty log by forward
    /// secrecy, so without the adder's floor it mints gen 0 and collides with a gen
    /// its own person already used, and the fold, keyed by (author, gen), drops one.
    /// The floor rides in the Welcome; this pins that the browser is handed it.
    #[test]
    fn join_intro_hands_back_the_gen_floor_the_adder_carried() {
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let bea_seed = [0x23u8; 32];
        let bea = identity::Identity::in_memory(bea_seed).identity_pk();
        let r = call_json(mls_init, &format!(r#"{{"cred":"{}","seed":"{}"}}"#, hex::encode(bea), hex::encode(bea_seed))).unwrap();
        let mut intro = [0u8; 32];
        intro.copy_from_slice(&hex::decode(r["intro"].as_str().unwrap()).unwrap());

        let ada = identity::Identity::in_memory([0x11u8; 32]).identity_pk();
        let ada_client = scanner(ada);
        let kp = hex::decode(call_json(mls_key_package, "").unwrap()["key_package"].as_str().unwrap()).unwrap();
        let mut g = mls::create_group_named(&ada_client, "Late joiners").unwrap();
        let (_commit, welcome) = mls::add_member(&mut g, &kp).unwrap();
        let payload = handshake::IntroPayload {
            scanner_pk: ada,
            scanner_name: "Ada".into(),
            welcome,
            why: None,
            kind: Some("forum".into()),
            arc: None,
            owner: Some(ada),
            // Ada's person has used gens up to 40 in this group; 41 is the floor.
            gen_watermark: Some(41),
        };
        let j = call_json(mls_join_intro, &format!(r#"{{"blob_b64":"{}"}}"#, intro_blob(&payload, &intro))).unwrap();
        assert_eq!(j["group_id"], hex::encode(g.group_id()));
        assert_eq!(j["gen_watermark"], 41, "the floor reaches the browser instead of being dropped at the join");
    }

    /// G1, the ADDING half. A browser that adds someone hands them the floor its own
    /// log shows, sealed to the intro tag from their signed bundle — what the phone's
    /// `group_add_member` sends. Without this a browser could only invite by a bare
    /// KeyPackage, and the joiner arrived with no floor.
    #[test]
    fn an_intro_the_browser_seals_carries_the_floor_its_log_shows() {
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // Ada is this device.
        let ada_seed = [0x31u8; 32];
        let ada = identity::Identity::in_memory(ada_seed).identity_pk();
        call_json(mls_init, &format!(r#"{{"cred":"{}","seed":"{}"}}"#, hex::encode(ada), hex::encode(ada_seed))).unwrap();

        // Bea, natively: her own client and intro tag, and a bundle she signs.
        let bea_id = identity::Identity::in_memory([0x32u8; 32]);
        let bea_client = scanner(bea_id.identity_pk());
        let bea_intro = [0x77u8; 32];
        let kp = mls::make_key_package_bytes(&bea_client).unwrap();
        let bundle = handshake::build_bundle(&bea_id, kp, bea_intro, "Bea").unwrap().encode().unwrap();

        let read = call_json(bundle_read, &serde_json::json!({ "bundle": bundle }).to_string()).unwrap();
        assert_eq!(read["identity"], hex::encode(bea_id.identity_pk()));
        assert_eq!(read["intro_tag"], hex::encode(bea_intro));
        assert_eq!(read["display_name"], "Bea");

        // Ada adds Bea by what the bundle carried.
        let gid = call_json(mls_create_group, "Thursday rooms").unwrap()["group_id"].as_str().unwrap().to_string();
        let added = call_json(mls_add_member, &serde_json::json!({
            "group_id": gid, "key_package": read["key_package"],
        }).to_string()).unwrap();

        // Ada's person has written up to gen 40 in this group.
        let said = coord::forum_post("hello", 40, 0);
        let log = serde_json::json!([{ "author": hex::encode(ada), "envelope": hex::encode(said.canonical_bytes()) }]);
        let ask = |floor: serde_json::Value| serde_json::json!({
            "bundle": bundle, "welcome": added["welcome"], "kind": "forum",
            "owner": hex::encode(ada), "name": "Ada", "log": log, "floor": floor,
        }).to_string();

        let sealed = call_json(intro_seal, &ask(serde_json::Value::Null)).unwrap();
        assert_eq!(sealed["gen_watermark"], 41, "one past the highest gen the log shows");
        assert_eq!(sealed["tag"], pacific_wire::address::Address::from_seed(&bea_intro).tag_hex(),
            "published where Bea's intro mailbox is");
        assert_eq!(sealed["address_seed"], hex::encode(bea_intro), "and signable for exactly that tag");

        // What Bea's device opens: the floor, the owner, the kind — and a Welcome she can join.
        let blob = pacific_wire::blob_unb64(sealed["blob"].as_str().unwrap()).unwrap();
        let p = handshake::IntroPayload::decode(&seal::open(&blob, &bea_intro, &bea_intro).unwrap()).unwrap();
        assert_eq!(p.gen_watermark, Some(41));
        assert_eq!((p.scanner_pk, p.owner, p.kind.as_deref()), (ada, Some(ada), Some("forum")));
        let joined = mls::join_group(&bea_client, &p.welcome).expect("Bea joins from the intro");
        assert_eq!(hex::encode(joined.group_id()), gid);

        // A floor of its own lifts the mark; the log alone does not lower it.
        let floored = call_json(intro_seal, &ask(serde_json::json!(50))).unwrap();
        assert_eq!(floored["gen_watermark"], 50);

        // A bundle whose signature does not hold is refused, in words.
        let mut bad = bundle.clone().into_bytes();
        let at = bad.len() / 2;
        bad[at] = if bad[at] == b'A' { b'B' } else { b'A' };
        let err = call_json(bundle_read, &serde_json::json!({ "bundle": String::from_utf8(bad).unwrap() }).to_string()).unwrap_err();
        assert!(!err.is_empty(), "a tampered bundle is refused");
    }

    #[test]
    fn seal_exports_round_trip_and_refuse_the_wrong_secret() {
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tag = "aa".repeat(32);
        let secret = "bb".repeat(32);
        let inner = hex::encode(b"the inner MLS bytes");
        let blob = String::from_utf8(call(seal_seal, format!(r#"{{"inner":"{inner}","tag":"{tag}","secret":"{secret}"}}"#).as_bytes()).unwrap()).unwrap();
        assert!(blob.len() > 48 + inner.len(), "nonce and tag are in there");
        let back = String::from_utf8(call(seal_open, format!(r#"{{"blob":"{blob}","tag":"{tag}","secret":"{secret}"}}"#).as_bytes()).unwrap()).unwrap();
        assert_eq!(back, inner);
        let err = call(seal_open, format!(r#"{{"blob":"{blob}","tag":"{tag}","secret":"{}"}}"#, "cc".repeat(32)).as_bytes()).unwrap_err();
        assert!(err.contains("seal open failed"), "{err}");
        let err = call(seal_open, format!(r#"{{"blob":"{blob}","tag":"{}","secret":"{secret}"}}"#, "dd".repeat(32)).as_bytes()).unwrap_err();
        assert!(err.contains("seal open failed"), "{err}");
        let err = call(seal_seal, format!(r#"{{"inner":"{inner}","tag":"abcd","secret":"{secret}"}}"#).as_bytes()).unwrap_err();
        assert!(err.contains("tag: 2 bytes, want 32"), "{err}");
    }
}

// ============================================================== THE BACKUP ====
//
// THE ARC IS THE HOME OF THE BACKUP (launch decision, 11 Sep 2026). One login —
// a passkey minted at the Arc the person chose — and the backup lives at that
// Arc, sealed under a key ONLY the passkey can derive: the second PRF salt,
// evaluated in the same assertion as the account channel's. `backup.rs` is the
// contract, already proven natively; these are wrappers over it and nothing
// here re-implements a derivation or an AEAD. The browser's part is only what a
// browser alone can do: hold the PRF output, put the ciphertext on the Arc, and
// pull it back down.
//
// WHAT THE BROWSER SUPPLIES, and what the core takes from the device it holds.
// The seed and the archive are the host's — the seed is kept under 'mls-seed'
// in the keyholder, the archive is what `archive.import` kept — so they arrive
// as input. The MLS snapshot is NOT the host's to assemble: `backup_build`
// captures it from the live device exactly as `mls_snapshot` does, and
// `backup_unpack` restores it exactly as `mls_restore` does. A host that could
// hand in its own snapshot could hand in a stale one, and a stale snapshot
// restored is a fork the rekey below exists to close.
//
// THE PERSON CHECK. A backup's seed derives an identity, and the MLS state in it
// belongs to that identity's leaf. `backup_build` refuses to bundle the live
// device's MLS state under a seed that is not its person; `backup_unpack`
// refuses to restore MLS state into a device that is not the backup's person.
// Either mismatch would be one person's ratchets under another's key — the
// split a peer's pin exists to catch, and it must not be constructible here.
//
// The archive's `exported_by` is deliberately NOT checked against the person:
// a browser holds the archive its phone handed over, under the browser's own
// MLS seed (the keyholder's second seed), so the exporter and the person are
// legitimately two identities on this platform.
//
// Every hex-in, hex-out here is by choice: the pieces pipe — `backup_build` into
// `backup_seal`'s `plain`, `backup_open` into `backup_unpack` — without the host
// re-encoding anything between them.

use pacific_core::mls_mem::MemGroup;

/// `wrap::wrap_key`. In: the 32 raw bytes of the passkey's PRF result (salt
/// `pacific/wrap/v1`). Out: 32 raw key bytes — the key the identity SEED is
/// wrapped under, and the only thing a passkey opens (D1, 13 Sep 2026).
#[no_mangle]
pub extern "C" fn wrap_key(len: u32) -> u32 {
    if len != 32 {
        return fail("wrap_key wants exactly 32 bytes of PRF secret");
    }
    let mut prf = [0u8; 32];
    prf.copy_from_slice(scratch(32));
    give(pacific_core::wrap::wrap_key(&prf).to_vec())
}

/// The identity public key a seed derives, and nothing else. In: 32 raw seed
/// bytes. Out: the 32 raw bytes of the Ed25519 public key.
///
/// PURE, which is the point. A browser derives this itself in WebCrypto (HKDF
/// then Ed25519) and must cross-check the result against the core, or a subtle
/// difference ships and is only discovered when someone's second device derives
/// a stranger. The only export that could answer before this one was `mls_init`,
/// which stands up group state and mints an intro tag on the way past — far too
/// much machinery to run on a signup page, and it mutates module state that the
/// caller then has to care about. This touches nothing.
#[no_mangle]
pub extern "C" fn identity_key(len: u32) -> u32 {
    if len != 32 {
        return fail("identity_key wants exactly 32 bytes of seed");
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(scratch(32));
    give(identity::Identity::in_memory(seed).identity_pk().to_vec())
}

/// `identity::recovery_key_from_seed`. In: the 32 raw bytes of the seed. Out: the
/// 24 words, UTF-8, single-space separated.
///
/// The words are the account's OTHER door, and the one that depends on no
/// platform at all. A synced passkey's PRF does follow a person to their next
/// device (`docs/prf-portability.html`) — but not across ecosystems, and not on
/// an external security key on iOS. The words always do, because the person
/// carries them.
#[no_mangle]
pub extern "C" fn recovery_words(len: u32) -> u32 {
    if len != 32 {
        return fail("recovery_words wants exactly 32 bytes of seed");
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(scratch(32));
    give(pacific_core::identity::recovery_key_from_seed(&seed).into_bytes())
}

/// `identity::seed_from_recovery_key`. In: the phrase, UTF-8. Out: the 32 raw
/// seed bytes. Fails loudly on a bad checksum rather than handing back a
/// different-but-valid-looking seed, which is the point of the checksum.
#[no_mangle]
pub extern "C" fn recovery_seed(len: u32) -> u32 {
    match (|| -> Result<Vec<u8>, String> {
        let phrase = utf8(len)?;
        Ok(pacific_core::identity::seed_from_recovery_key(phrase)
            .map_err(|e| e.to_string())?
            .to_vec())
    })() {
        Ok(b) => give(b),
        Err(e) => fail(&e),
    }
}

// THE HISTORY BLOB IS NOT ON THIS SURFACE EITHER, and that closes the last door
// to it. `history_key`, `history_seal`, `history_open`, `history_build`,
// `history_peek` and `history_unpack` used to sit here — six exports, of which
// `history_unpack` restored `mls_mem::Snapshot` straight into the live device:
// every group's ratchet tree, epoch secrets and key-package private halves.
//
// That is the escrow Re-admission §1 named and the register retired on 14
// September 2026 (Leaves §02, §04). It was retired in the documents and left
// callable from a browser, which is worse than never having written it down: a
// page that wants recovery finds a function that provides it and a comment that
// says it is the way. The iOS half came off `pacific-ffi` in the same pass.
//
// A recovering page gets its groups from the archive chain — one retained
// exporter output per epoch at an address computed from the storage root — and
// its MLS state through mls-rs's own `GroupStateStorage`. Until that is wired,
// a recovered session holds its identity and no groups, and the honest surface
// is an empty feed that says why. Do not restore this by adding a reader.
//
// WHY `mls_snapshot` AND `mls_restore` SURVIVED, since they look like the same
// thing and the difference is the whole point. They are the same bytes; they are
// not the same act. The keyholder keeps its snapshot in IndexedDB under a
// non-extractable AES-GCM key, on the origin that produced it — local
// persistence, so a page reload is not a state loss. The `history_*` family took
// those bytes and put them AT THE ARC, which is what makes a backup an escrow:
// not that the snapshot exists, but that it leaves.
//
// So the pair stays and its destination is the thing to watch. A snapshot that
// crosses to a server is the escrow again under a new name, whatever it is
// called at the call site.


/// `wrap::seal`. Input `{"prf":"<64 hex>","seed":"<64 hex>","host":"<arc host>"}`;
/// out: the `PWR1 || nonce || ciphertext` envelope, hex. The host is bound into
/// the AEAD, so a wrap lifted onto another Arc will not open there.
#[no_mangle]
pub extern "C" fn wrap_seal(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let prf = field32(&v, "prf")?;
        let seed = field32(&v, "seed")?;
        let host = v["host"].as_str().ok_or("host required")?;
        Ok(hex::encode(
            pacific_core::wrap::seal(&prf, &seed, host).map_err(|e| e.to_string())?,
        ))
    })() {
        Ok(h) => give(h.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// `wrap::open`. Input `{"prf":"<64 hex>","blob":"<hex>","host":"<arc host>"}`;
/// out: the 32-byte seed, hex — SECRET, and the thing every other derivation
/// starts from. A wrong passkey, a wrong host and a flipped byte all fail the
/// same way, which is correct: none of the three is separately actionable.
#[no_mangle]
pub extern "C" fn wrap_open(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let prf = field32(&v, "prf")?;
        let blob = field_hex(&v, "blob")?;
        let host = v["host"].as_str().ok_or("host required")?;
        let seed = pacific_core::wrap::open(&prf, &blob, host).map_err(|e| e.to_string())?;
        Ok(hex::encode(seed.as_slice()))
    })() {
        Ok(h) => give(h.into_bytes()),
        Err(e) => fail(&e),
    }
}

/* ── LOCATORS ────────────────────────────────────────────────────────────────

   The addresses a person's records live at. `identity_pk` is the account's NAME
   — it signs challenges, it is the MLS credential, it is what peers pin — and
   using the name as the address makes the arc a census: anyone holding a public
   key can probe for the person behind it, and a breach yields the roll of
   everyone on it. A locator is 32 bytes of HKDF over the seed, carries no
   identity, and cannot be run backwards.

   WHY THE BROWSER MUST NOT COMPUTE THESE ITSELF. WebCrypto has HKDF, so the four
   lines are easy to write in the page — and that is the hazard. Two derivations
   of one address put two of a person's devices on different rows, and the second
   device finds an empty address and concludes the account is new. Nothing logs
   it. No test catches it. There is one definition, in `locator.rs`, and every
   client asks for it. */

/// The whole address set from one call, so no page holds a label string. Input
/// `{"seed":"<64 hex>"}` — or `{"storage_root":"<64 hex>"}` for an account that
/// has a root and no seed — and optionally `"prf":"<64 hex>"` and
/// `"chain_gen":<n>`. Out `{"storage_root","head","history","index","acct_pk",
/// "wrap","chain"}`, all hex, `wrap` only when a PRF was given and `chain` only
/// when a generation was.
///
/// THE CHAIN NEEDS ITS GENERATION, and it comes from the head. There is no default:
/// an address that quietly meant generation 0 would put a device reading
/// generation 3 on an empty row with nothing to say so. Fetch the head, open it,
/// and ask again with its `chain_gen`.
///
/// TWO ROOTS, AND EVERYTHING HERE IS UNDER THE STORAGE ONE. `five-things.html`
/// §06, ruled 16 Sep 2026: `pacific/identity/…` and `pacific/storage/…`, disjoint,
/// and chosen BEFORE anything is written — labels that mix the families force a
/// derivation version on every existing account the day BYO lands. Taking a
/// storage root rather than a seed is what makes that additive: a BYO account has
/// no seed, mints a root, and nothing downstream of this line differs.
///
/// `storage_root` comes back in the reply so a caller can hand it to the next
/// call instead of re-deriving, and so the value is visible rather than implied.
/// The storage root from a request: given directly (`storage_root`, for an account
/// that has a root and no seed), or derived from `seed`. One reader, so every
/// export that takes a root takes it the same way.
fn root_of(v: &serde_json::Value) -> Result<[u8; 32], String> {
    match (v["storage_root"].as_str(), v["seed"].as_str()) {
        (Some(_), _) => field32(v, "storage_root"),
        (None, Some(_)) => Ok(*pacific_core::locator::storage_root(&field32(v, "seed")?)),
        (None, None) => Err("seed or storage_root required".into()),
    }
}

#[no_mangle]
pub extern "C" fn locators(len: u32) -> u32 {
    use pacific_core::locator::{self, Record};
    match (|| -> Result<String, String> {
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let root = root_of(&v)?;
        let at = |r: Record| -> Result<String, String> {
            Ok(hex::encode(locator::locator(&root, r).map_err(|e| e.to_string())?))
        };
        let wrap = match v["prf"].as_str() {
            Some(_) => format!(r#","wrap":"{}""#, hex::encode(locator::wrap_locator(&field32(&v, "prf")?))),
            None => String::new(),
        };
        let chain = match v.get("chain_gen") {
            None => String::new(),
            Some(g) => {
                let g = g.as_u64().and_then(|g| u32::try_from(g).ok())
                    .ok_or("chain_gen is a u32 — the head's own field")?;
                // Index 0 — where a generation starts. Under the old linked form this
                // was THE derivable address and every later entry hid inside its
                // predecessor; addressing is arithmetic now, so this is one of a
                // family and the caller walks by index from here — with
                // `spine_addresses`, which derives every index's tag, and `spine_read`,
                // which opens and judges what the walk brought back.
                format!(r#","chain":"{}""#, hex::encode(locator::chain_locator(&root, g, 0)))
            }
        };
        Ok(format!(
            r#"{{"storage_root":"{}","head":"{}","history":"{}","index":"{}","acct_pk":"{}"{}{}}}"#,
            hex::encode(root),
            at(Record::Head)?,
            at(Record::History)?,
            at(Record::Index)?,
            hex::encode(locator::acct_pk(&root)),
            wrap,
            chain
        ))
    })() {
        Ok(s) => give(s.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// Sign a write to a locator. Input
/// `{"seed","locator","audience","nonce","body"}` — `seed`/`locator` 64 hex,
/// `audience` the arc host, `nonce` the arc's challenge, `body` hex. Out: the
/// 64-byte signature, hex.
///
/// A CHALLENGE-RESPONSE, like every other authenticated route. Get a nonce from
/// `/auth/challenge` first and let the arc spend it. The audience and the nonce
/// are what stop a captured write verifying at another arc, or verifying again
/// tomorrow — and the second matters more than it looks: because the body is
/// bound, a replay is idempotent, so it is not forgery but ROLLBACK.
#[no_mangle]
pub extern "C" fn arc_write_sign(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let root = match (v["storage_root"].as_str(), v["seed"].as_str()) {
            (Some(_), _) => field32(&v, "storage_root")?,
            (None, Some(_)) => *pacific_core::locator::storage_root(&field32(&v, "seed")?),
            (None, None) => return Err("seed or storage_root required".into()),
        };
        let loc = field32(&v, "locator")?;
        let body = field_hex(&v, "body")?;
        let audience = v["audience"].as_str().ok_or("audience required (the arc host)")?;
        let nonce = v["nonce"].as_str().ok_or("nonce required (from /auth/challenge)")?;
        let w = pacific_core::locator::ArcWrite { audience, locator: &loc, nonce, body: &body };
        Ok(hex::encode(pacific_core::locator::sign_write(&root, &w).map_err(|e| e.to_string())?))
    })() {
        Ok(s) => give(s.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// The arc's half of first-write-claims: the first PUT to a locator records the
/// key, every later write must match it. Input
/// `{"recorded","offered","locator","audience","nonce","body","sig"}`, all hex
/// except `audience` and `nonce`. Out: `{"ok":true}`, or a refusal naming which
/// of the three cases it was.
///
/// `offered` is the key the writer presents on every write. It is compared to
/// `recorded` BEFORE the signature, and the signature is checked against
/// `recorded` and never against `offered` — verifying against the key the writer
/// supplied would authorise everybody. The split exists so the arc can LOG a squat
/// and a bug differently; the REPLY must stay opaque, for the reason
/// `auth.py::verify` already gives: a caller that could tell "wrong key" from
/// "wrong nonce" could enumerate accounts.
#[no_mangle]
pub extern "C" fn arc_write_verify(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let recorded = field32(&v, "recorded")?;
        let offered = field32(&v, "offered")?;
        let loc = field32(&v, "locator")?;
        let body = field_hex(&v, "body")?;
        let audience = v["audience"].as_str().ok_or("audience required (the arc host)")?;
        let nonce = v["nonce"].as_str().ok_or("nonce required")?;
        let sig = field_hex(&v, "sig")?;
        let sig: [u8; 64] = sig
            .as_slice()
            .try_into()
            .map_err(|_| format!("an arc-write signature is 64 bytes, got {}", sig.len()))?;
        let w = pacific_core::locator::ArcWrite { audience, locator: &loc, nonce, body: &body };
        pacific_core::locator::verify_write(&recorded, &offered, &w, &sig)
            .map_err(|r| r.to_string())?;
        Ok(r#"{"ok":true}"#.to_string())
    })() {
        Ok(s) => give(s.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// A fresh address for the next chain entry, to be sealed inside the current one.
/// No input. Out: 32 bytes, hex.
///
/// RANDOM, NOT DERIVED, and that is the whole mechanism rather than a shortcut. A
/// derived next-tag is computable by anyone who can compute the first, so the
/// sequence collapses back into one seed-derived address and hands the blind
/// relay a stable per-user tag with one entry per epoch — a per-user activity
/// profile. Unguessable-without-the-predecessor is the property, and only
/// entropy gives it. Do not simplify this to one tag.
#[no_mangle]
pub extern "C" fn chain_next_tag(_len: u32) -> u32 {
    match pacific_core::locator::next_tag() {
        Ok(t) => give(hex::encode(t).into_bytes()),
        Err(e) => fail(&e.to_string()),
    }
}

/* ── THE HEAD ────────────────────────────────────────────────────────────────

   The anchor a chain is checked against, and the boundary between derivation and
   discovery: everything up to here comes out of the seed, everything past it has
   to be fetched and can therefore be withheld. A chain cannot prove it is
   complete — a prefix of a valid chain is a valid chain — so the head is the only
   thing that catches truncation.

   `head.rs` has had all of this since it was written, and `PUT /auth/users/{pk}/head`
   has had the compare-and-set to receive it. No surface could reach either,
   because an account is made in a browser and the browser had no way to seal one.
   That is the whole of the gap these three exports close.

   ALL THREE TAKE THE SEED OR THE STORAGE ROOT, NEVER THE PRF. Someone restoring
   from 24 words on a borrowed laptop, who has never had a passkey, must be able to
   open their head; deriving this key from the PRF instead would quietly turn
   truncation detection into a passkey-only feature and nothing would fail to say
   so. The key itself hangs off the storage root (head v2, 18 Sep 2026), so a BYO
   account with a root and no seed reaches its head the same way. */

/// `head::seal`. Input
/// `{"arc":"<host>","position":<n>,"tail":"<64 hex>","chain_gen":<n>,"seed":"<64 hex>"}`
/// — or `"storage_root"` in place of `"seed"`. `chain_gen` names which chain this
/// head anchors; an account's first head is generation 0.
/// `tail` may be omitted or empty, which is how the FIRST head is written: an
/// empty chain has no tail, and position 0 is the floor the arc's compare-and-set
/// enforces from then on. Out: the sealed blob, hex —
/// `PHD1 || nonce(24) || ciphertext`, the exact bytes `PUT /head` accepts.
#[no_mangle]
pub extern "C" fn head_seal(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let root = root_of(&v)?;
        let arc = v["arc"].as_str().ok_or("arc required (the host storing it)")?;
        let position =
            v["position"].as_u64().ok_or("position required (0 names an empty chain)")?;
        let chain_gen = v["chain_gen"].as_u64().and_then(|g| u32::try_from(g).ok())
            .ok_or("chain_gen required (a u32; an account's first chain is 0)")?;
        let tail = match v["tail"].as_str() {
            None | Some("") => head::NO_TAIL,
            Some(_) => field32(&v, "tail")?,
        };
        let h = head::Head::new(arc, position, tail, chain_gen);
        // A head whose position and tail disagree seals perfectly well and can
        // then never be opened, because `from_cbor` refuses it on the way back.
        // That rule lives in exactly one place, so this is a round trip through
        // it rather than a second copy of it here.
        head::Head::from_cbor(&h.to_cbor().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        Ok(hex::encode(head::seal(&h, &root).map_err(|e| e.to_string())?))
    })() {
        Ok(h) => give(h.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// `head::open_declared`. Input `{"blob":"<hex>","seed":"<64 hex>"}` (or
/// `"storage_root"`), and — pass
/// it — `"declared_position":<n>`, the `X-Chain-Position` the arc served in the
/// response header. The arc orders writes on a number it cannot decrypt, so a
/// broken or dishonest one can serve position 41 in the header and the ciphertext
/// of position 17 in the body; the header and the sealed body are two claims, and
/// holding them against each other costs one comparison. Out:
/// `{"v":2,"arc":"…","position":<n>,"tail":"<64 hex>","chain_gen":<n>}`.
#[no_mangle]
pub extern "C" fn head_open(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let root = root_of(&v)?;
        let blob = field_hex(&v, "blob")?;
        let h = match v["declared_position"].as_u64() {
            Some(p) => head::open_declared(&blob, &root, p),
            None => head::open(&blob, &root),
        }
        .map_err(|e| e.to_string())?;
        Ok(format!(
            r#"{{"v":{},"arc":{},"position":{},"tail":"{}","chain_gen":{}}}"#,
            h.v,
            serde_json::to_string(&h.arc).map_err(|e| e.to_string())?,
            h.position,
            hex::encode(&h.tail),
            h.chain_gen
        ))
    })() {
        Ok(s) => give(s.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// `Head::check`. Input
/// `{"position":<n>,"tail":"<64 hex>","walked_position":<n>,"walked_tail":"<64 hex>"}`
/// — the head's two fields against what the walk actually produced. `arc` and
/// `chain_gen` are not inputs: the verdict is a function of two numbers and two
/// hashes, which is why the head is testable before any chain exists. Out:
/// `{"verdict":"whole"}` · `{"verdict":"truncated","missing":<n>}` ·
/// `{"verdict":"forked"}` · `{"verdict":"head_behind","extra":<n>}`.
///
/// FOUR ANSWERS, NOT TWO, and the fourth is not an alarm. `head_behind` is what a
/// client that appended and then died before updating its head leaves behind; the
/// response is to advance the head, not to distrust the arc. Reporting it as an
/// attack would train people to ignore the one that is.
#[no_mangle]
pub extern "C" fn head_check(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let position = v["position"].as_u64().ok_or("position required")?;
        let walked_position =
            v["walked_position"].as_u64().ok_or("walked_position required")?;
        let tail = match v["tail"].as_str() {
            None | Some("") => head::NO_TAIL,
            Some(_) => field32(&v, "tail")?,
        };
        let walked_tail = match v["walked_tail"].as_str() {
            None | Some("") => head::NO_TAIL,
            Some(_) => field32(&v, "walked_tail")?,
        };
        // The arc and the generation play no part in `check`, so they are not asked
        // for. Empty and 0 here are placeholders, not defaults a caller could rely
        // on: nothing in the verdict reads them.
        let h = head::Head::new("", position, tail, 0);
        Ok(match h.check(walked_position, &walked_tail) {
            head::ChainVerdict::Whole => r#"{"verdict":"whole"}"#.to_string(),
            head::ChainVerdict::Truncated { missing } => {
                format!(r#"{{"verdict":"truncated","missing":{missing}}}"#)
            }
            head::ChainVerdict::Forked => r#"{"verdict":"forked"}"#.to_string(),
            head::ChainVerdict::HeadBehind { extra } => {
                format!(r#"{{"verdict":"head_behind","extra":{extra}}}"#)
            }
        })
    })() {
        Ok(s) => give(s.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// The most spine addresses one call derives. A walk asks in batches; a request
/// for more is a caller bug, and refusing it is cheaper than allocating for it.
const SPINE_ADDRESS_BATCH: u64 = 256;

/// The relay tag each spine index lives at. Input
/// `{"seed":"<64 hex>"` (or `"storage_root"`) `,"gen":<n>,"from":<n>,"count":<n>}`,
/// out `{"addresses":[{"index":<n>,"tag":"<hex>"}, …]}`.
///
/// ARITHMETIC: entry `i` is at `chain_locator(root, gen, i)`, so a reader asks for
/// any index directly and a lost one costs only itself. This supersedes the
/// `"chain"` field of `locators`, which is the address of index 0 alone — a
/// leftover from the linked form, where only the first entry could be derived.
#[no_mangle]
pub extern "C" fn spine_addresses(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let root = root_of(&v)?;
        let gen = u32::try_from(v["gen"].as_u64().ok_or("gen required")?)
            .map_err(|_| "gen out of range".to_string())?;
        let from = v["from"].as_u64().ok_or("from required")?;
        let count = v["count"].as_u64().ok_or("count required")?;
        if count > SPINE_ADDRESS_BATCH {
            return Err(format!("count {count} is over the batch of {SPINE_ADDRESS_BATCH}"));
        }
        let addresses: Vec<serde_json::Value> = (from..from.saturating_add(count))
            .map(|i| {
                let at = pacific_core::locator::chain_locator(&root, gen, i);
                serde_json::json!({
                    "index": i,
                    "tag": pacific_wire::address::Address::from_seed(&at).tag_hex(),
                })
            })
            .collect();
        Ok(serde_json::json!({ "addresses": addresses }).to_string())
    })() {
        Ok(s) => give(s.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// Seal one spine entry, for a browser to publish. Input
/// `{"seed":"<64 hex>"` (or `"storage_root"`) `,"gen":<n>,"index":<n>,
/// "body":{"group":{"group_id":"<hex>","first_epoch":<n>}} | {"left":{"group_id":"<hex>","last_epoch":<n>}}}`.
/// Out `{"index":<n>,"tag":"<hex>","blob":"<base64 as it will travel>"}`.
///
/// `spine::place` itself, so the browser writes entries with the ONE encoder the
/// phone's drain uses: the same seal, the same address, the same bytes a reader on
/// either platform opens. It is the browser mint's "chain entry queued" step, and
/// the join and the departure's.
///
/// THE INDEX IS THE CALLER'S, and a browser has no outbox to take it from. Until
/// indices are claimed account-wide, it is the highest index a spine walk opened,
/// plus one — which can collide with another of the person's devices, and
/// `spine_read` tolerates that. Sealing draws a fresh nonce, so two calls with one
/// input give different blobs; that is the seal, not a fault.
#[no_mangle]
pub extern "C" fn spine_place(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        use pacific_core::spine::{self, Body, Departed, Joined};
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let root = root_of(&v)?;
        let gen = u32::try_from(v["gen"].as_u64().ok_or("gen required")?)
            .map_err(|_| "gen out of range".to_string())?;
        let index = v["index"].as_u64().ok_or("index required")?;
        let gid = |b: &serde_json::Value| -> Result<Vec<u8>, String> {
            hex::decode(b["group_id"].as_str().ok_or("body.*.group_id required")?)
                .map_err(|e| format!("group_id: {e}"))
        };
        let b = &v["body"];
        let body = if b["group"].is_object() {
            let g = &b["group"];
            Body::Group(Joined {
                group_id: gid(g)?,
                first_epoch: g["first_epoch"].as_u64().ok_or("body.group.first_epoch required")?,
            })
        } else if b["left"].is_object() {
            let l = &b["left"];
            Body::Left(Departed {
                group_id: gid(l)?,
                last_epoch: l["last_epoch"].as_u64().ok_or("body.left.last_epoch required")?,
            })
        } else {
            return Err(r#"body is {"group":{…}} or {"left":{…}}"#.to_string());
        };
        let (at, blob) = spine::place(&root, gen, index, body).map_err(|e| e.to_string())?;
        Ok(serde_json::json!({
            "index": index,
            "tag": pacific_wire::address::Address::from_seed(&at).tag_hex(),
            "blob": pacific_wire::blob_b64(&blob),
            // The relay refuses a pub its tag did not sign, and `relay_sign_pub`
            // signs from the address SEED, not the tag. Without this the entry
            // could be sealed and never published. It is derived from the root
            // the caller just passed in, so it tells the caller nothing new.
            "address_seed": hex::encode(at),
        })
        .to_string())
    })() {
        Ok(s) => give(s.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// THE SPINE READ — what a fresh device learns from its words, judged. Input
/// `{"seed":"<64 hex>"` (or `"storage_root"`) `,"head":{"position":<n>,"tail":"<64 hex>"}|null,
/// "fetched":[{"index":<n>,"blobs":["<base64 as it travelled>", …]}, …]}`.
/// Out:
/// `{"recovery":{"tail":"intact|missing|forked|unknown","holes":[<n>…],"extra":<n>,"found":<n>},
///   "objects":[{"group_id":"<hex>","first_epoch":<n>,"left_epoch":<n>|null}…],
///   "left":[{"group_id":"<hex>","last_epoch":<n>,"index":<n>}…],"unopened":<n>}`.
///
/// ONE EXPORT so no client reimplements the index logic: every blob at every
/// fetched index is opened under the account's key AT THAT INDEX, every one that
/// opens is hashed for the tail check, and the walk is judged by
/// [`head::Head::judge`]. `recovery` is `app/web/shared/recovery-contract.json`'s
/// per-account record. There is no `complete`: it is derivable, and derived from
/// a raw position today it would be wrong for every account.
///
/// `head` is the OPENED head (`head_open`, with its declared-position check). Null
/// reads as a head that vouches for nothing, which is what the only head any
/// account has today — the first one, at position 0 — does too: `tail: unknown`.
///
/// THE WALK THE CALLER DOES: every index below `head.position`, THROUGH misses,
/// then onward until the first index with nothing at it. Pass what came back,
/// including every blob at a tag — a person's devices can publish at one index
/// until indices are claimed account-wide.
///
/// `objects` lists each group a `Group` entry names, with the LOWEST floor if two
/// entries name it, and `left_epoch` set when the account has LEFT it: the latest
/// departure is at or after the latest join. That is what lets a recovering device
/// skip folding a group it exited — it can read up to `left_epoch` and will never
/// read past it. A join after the latest departure is a rejoin, and `left_epoch` is
/// null. Judged on epochs rather than index order, because two of a person's
/// devices can publish at one index. `left` stays, raw, one row per `Left` entry. `unopened` counts blobs at our addresses that
/// did not open — corruption, or a caller passing the wrong index — and is not
/// folded into `recovery`, because the contract does not carry it.
#[no_mangle]
pub extern "C" fn spine_read(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        use pacific_core::spine::{self, Body};
        let v: serde_json::Value =
            serde_json::from_str(utf8(len)?).map_err(|e| format!("bad json: {e}"))?;
        let root = root_of(&v)?;
        let key = spine::entry_key(&root);
        let judge_by = match &v["head"] {
            serde_json::Value::Null => head::Head::empty("", 0),
            h => {
                let position = h["position"].as_u64().ok_or("head.position required")?;
                let tail = match h["tail"].as_str() {
                    None | Some("") => head::NO_TAIL,
                    Some(_) => field32(h, "tail")?,
                };
                head::Head::new("", position, tail, 0)
            }
        };

        let mut opened = Vec::new();
        // group → (lowest join, latest join); and group → latest departure.
        let mut joins: std::collections::BTreeMap<Vec<u8>, (u64, u64)> = Default::default();
        let mut departures: std::collections::BTreeMap<Vec<u8>, u64> = Default::default();
        let mut left = Vec::new();
        let mut unopened = 0u64;
        for f in v["fetched"].as_array().ok_or("fetched required")? {
            let index = f["index"].as_u64().ok_or("fetched[].index required")?;
            for b in f["blobs"].as_array().ok_or("fetched[].blobs required")? {
                let b64 = b.as_str().ok_or("a blob is a base64 string")?;
                let Ok(blob) = pacific_wire::blob_unb64(b64) else {
                    unopened += 1;
                    continue;
                };
                let Ok(entry) = spine::open_entry(&blob, &key, index) else {
                    unopened += 1;
                    continue;
                };
                opened.push((index, head::tail_hash(&blob)));
                match entry.body {
                    Body::Group(j) => {
                        let e = joins.entry(j.group_id).or_insert((j.first_epoch, j.first_epoch));
                        e.0 = e.0.min(j.first_epoch);
                        e.1 = e.1.max(j.first_epoch);
                    }
                    Body::Left(d) => {
                        left.push(serde_json::json!({
                            "group_id": hex::encode(&d.group_id),
                            "last_epoch": d.last_epoch,
                            "index": index,
                        }));
                        let l = departures.entry(d.group_id).or_insert(d.last_epoch);
                        *l = (*l).max(d.last_epoch);
                    }
                    // A way-in: counted in the walk above, but it names no object and
                    // ends none (`Node::resume_at` takes membership from Group/Left
                    // alone). Resuming through its leaves is the door's, never the
                    // browser's (resumption.md A3), so its keys stay here.
                    Body::Pool(_) => {}
                }
            }
        }

        let w = judge_by.judge(&opened);
        let objects: Vec<serde_json::Value> = joins
            .into_iter()
            .map(|(g, (floor, latest))| {
                let left_epoch = departures.get(&g).copied().filter(|&l| l >= latest);
                serde_json::json!({
                    "group_id": hex::encode(&g),
                    "first_epoch": floor,
                    "left_epoch": left_epoch,
                })
            })
            .collect();
        Ok(serde_json::json!({
            "recovery": {
                "tail": w.tail.as_str(),
                "holes": w.holes,
                "extra": w.extra,
                "found": w.found,
            },
            "objects": objects,
            "left": left,
            "unopened": unopened,
        })
        .to_string())
    })() {
        Ok(s) => give(s.into_bytes()),
        Err(e) => fail(&e),
    }
}

// ---------------------------------------------------------------- the rekey
//
// THE FORK, AND HOW IT IS CLOSED. Restoring MLS state on this device while the
// install the backup came from may still hold it makes two leaves with one key
// schedule. `backup.rs` says what follows: a rekey commit in every group before
// anything else. After it, the stale copy cannot decrypt forward, and its next
// commit is rejected by the relay's epoch slot.
//
// PENDING-COMMIT DISCIPLINE, as `node.rs` keeps it (RFC 9420 §14: generating a
// commit MUST NOT modify state). `mls_rekey` stages and returns the commit; the
// host publishes it into the CURRENT epoch tag's single commit slot; a winning
// ack is `mls_rekey_apply`, a losing one is `mls_rekey_drop` + drain the tag to
// fold the winner in + stage again. The staged group lives in memory here — a
// `load_group` is a fresh object each call, so the pending commit would be
// lost between the two calls without somewhere to keep it. One at a time:
// groups are rekeyed sequentially, and staging a second drops the first.

static mut PENDING_REKEY: Option<(Vec<u8>, MemGroup)> = None;

/// Stage a rekey. In: the group id, hex. Out: `{"commit":"<hex>","epoch":n}` —
/// `epoch` is the CURRENT epoch, the one whose slot the commit claims and whose
/// tag/secret (`mls_epoch_keys`) it is sealed under. Nothing is applied.
#[no_mangle]
pub extern "C" fn mls_rekey(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let gid = hex::decode(utf8(len)?.trim()).map_err(|e| format!("bad group_id: {e}"))?;
        let d = dev()?;
        let mut g = mls::load_group(&d.client, &gid).map_err(|e| e.to_string())?;
        let epoch = g.current_epoch();
        let commit = mls::stage_rekey(&mut g).map_err(|e| e.to_string())?;
        unsafe { PENDING_REKEY = Some((gid, g)) };
        Ok(format!(r#"{{"commit":"{}","epoch":{}}}"#, hex::encode(commit), epoch))
    })() {
        Ok(j) => give(j.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// Take the staged group for `gid`, refusing loudly if the staged one is another.
fn take_pending(len: u32) -> Result<(Vec<u8>, MemGroup), String> {
    let gid = hex::decode(utf8(len)?.trim()).map_err(|e| format!("bad group_id: {e}"))?;
    let (staged_gid, g) = unsafe { PENDING_REKEY.take() }.ok_or("no rekey is staged — call mls_rekey first")?;
    if staged_gid != gid {
        unsafe { PENDING_REKEY = Some((staged_gid.clone(), g)) };
        return Err(format!(
            "the staged rekey is for group {}, not {}",
            hex::encode(&staged_gid),
            hex::encode(&gid)
        ));
    }
    Ok((gid, g))
}

/// Apply + persist the staged rekey — ONLY after its commit won the epoch slot.
/// In: the group id, hex. Out: `{"epoch":n}`, the NEW epoch.
#[no_mangle]
pub extern "C" fn mls_rekey_apply(len: u32) -> u32 {
    match (|| -> Result<String, String> {
        let (_gid, mut g) = take_pending(len)?;
        mls::apply_staged(&mut g).map_err(|e| e.to_string())?;
        Ok(format!(r#"{{"epoch":{}}}"#, g.current_epoch()))
    })() {
        Ok(j) => give(j.into_bytes()),
        Err(e) => fail(&e),
    }
}

/// Discard the staged rekey — the slot was already taken. Nothing was written,
/// so there is nothing to undo; the host drains the tag to fold the winner in
/// (`mls_decrypt`) and stages again. In: the group id, hex. Out: `{"dropped":true}`.
#[no_mangle]
pub extern "C" fn mls_rekey_drop(len: u32) -> u32 {
    match take_pending(len) {
        Ok(_) => give(br#"{"dropped":true}"#.to_vec()),
        Err(e) => fail(&e),
    }
}

// ---------------------------------------------------------------- native proof
//
// The backup exports driven natively, the way the page drives them: a device
// with a group, backed up, sealed, opened, unpacked into a second device that
// is the same person, which then rekeys — and the peer folds the rekey and
// keeps talking while the pre-rekey epoch is behind it.

#[cfg(test)]
mod backup_exports {
    use super::*;
    use pacific_core::mls_mem::{build_client_mem, MemGroupStateStorage, MemKeyPackageStorage};
    /// The exports share one scratch, one out buffer and one DEVICE with the
    /// `exports` module above, and `cargo test` runs the two modules' tests on
    /// parallel threads — so this holds the SAME lock that module holds, not a
    /// second one that would serialise nothing.
    use super::exports::LOCK;

    fn call(f: extern "C" fn(u32) -> u32, input: &[u8]) -> Result<Vec<u8>, String> {
        let p = alloc(input.len() as u32);
        // SAFETY: `alloc` just sized SCRATCH to exactly `input.len()` bytes.
        unsafe { std::ptr::copy_nonoverlapping(input.as_ptr(), p, input.len()) };
        let _ = f(input.len() as u32);
        let out = unsafe { OUT.clone() };
        if erred() != 0 { Err(String::from_utf8_lossy(&out).into_owned()) } else { Ok(out) }
    }
    fn json(f: extern "C" fn(u32) -> u32, input: &str) -> Result<serde_json::Value, String> {
        let out = call(f, input.as_bytes())?;
        serde_json::from_slice(&out).map_err(|e| format!("reply is not json: {e}"))
    }
    fn text(f: extern "C" fn(u32) -> u32, input: &str) -> Result<String, String> {
        call(f, input.as_bytes()).map(|o| String::from_utf8_lossy(&o).into_owned())
    }

    /// A peer, natively: its own client, as a phone is its own.
    struct Peer {
        client: pacific_core::mls_mem::MemClient,
        cred: [u8; 32],
    }
    fn peer(seed: [u8; 32]) -> Peer {
        let cred = identity::Identity::in_memory(seed).identity_pk();
        let (sk, pk) = mls::generate_signing_key(&mls::crypto()).unwrap();
        let client = build_client_mem(
            MemGroupStateStorage::new(),
            MemKeyPackageStorage::new(),
            mls::signing_identity(&cred, pk.as_bytes()),
            sk,
        )
        .unwrap();
        Peer { client, cred }
    }

    #[test]
    fn the_identity_key_is_the_one_the_core_derives() {
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let seed = [0x24u8; 32];
        let pk = call(identity_key, &seed).unwrap();
        assert_eq!(pk, identity::Identity::in_memory(seed).identity_pk().to_vec());
        // Same seed, same key, every time — a browser cross-checks against this.
        assert_eq!(call(identity_key, &seed).unwrap(), pk);
        assert_ne!(call(identity_key, &[0x25u8; 32]).unwrap(), pk);
        let err = call(identity_key, &seed[..31]).unwrap_err();
        assert!(err.contains("exactly 32"), "{err}");
    }

    #[test]
    fn the_words_carry_the_seed_and_a_typo_is_caught() {
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let seed = [0x24u8; 32];
        let words = String::from_utf8(call(recovery_words, &seed).unwrap()).unwrap();
        assert_eq!(words.split_whitespace().count(), 24);
        assert_eq!(call(recovery_seed, words.as_bytes()).unwrap(), seed);

        // A mistyped word must fail the checksum rather than reconstitute some
        // other account: the whole reason the phrase is checksummed.
        let mut typo: Vec<&str> = words.split_whitespace().collect();
        let last = typo.len() - 1;
        typo[last] = if typo[last] == "zoo" { "zone" } else { "zoo" };
        let err = call(recovery_seed, typo.join(" ").as_bytes()).unwrap_err();
        assert!(err.contains("bad recovery key"), "{err}");
    }

    /// The split (D1) without the history half: the PRF opens the wrap, and the
    /// account channel is neither the wrap key nor its seal. The third arm used
    /// to be `history_key`; that domain went with the blob it sealed.
    #[test]
    fn the_wrap_key_is_unrelated_to_the_account_channel() {
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prf = [0x42u8; 32];
        let wrapk = call(wrap_key, &prf).unwrap();
        assert_eq!(wrapk.len(), 32);
        let chan = call(account_channel, &prf).unwrap();
        assert_ne!(&wrapk[..], &chan[32..64], "wrap key != account channel seal");
        assert_ne!(&wrapk[..], &chan[..32], "wrap key != account channel tag");
    }

    /// THE ESCROW IS GONE FROM THIS SURFACE, pinned so it cannot come back by
    /// habit. Two tests used to live here: one that round-tripped the history
    /// seal, and `build_unpack_restores_a_person_who_can_rekey_and_the_stale_copy
    /// _goes_deaf`, which proved the whole escrow worked — build a blob carrying
    /// `mls_mem::Snapshot`, unpack it into a second device, rekey, and watch the
    /// first go deaf. It worked. That was the problem.
    ///
    /// Re-admission §1 called it "a complete key escrow" and the register retired
    /// it on 14 September 2026 (Leaves §02, §04). A test that proves an escrow
    /// works is a test that will be defended, so it goes with the exports.
    ///
    /// What replaces it is not a smaller blob. It is the archive chain: one
    /// retained exporter output per epoch at an address computed from the storage
    /// root, and MLS state through mls-rs's own `GroupStateStorage`. Recovery
    /// reads history it was present for; it does not receive a copy of everyone's
    /// keys. See `core/docs/archive-by-arithmetic.md`.
    #[test]
    fn the_history_exports_are_not_here() {
        // Compile-time, not run-time: if `history_build`, `history_unpack` or
        // their siblings are reintroduced, the module below stops being empty and
        // this file stops telling the truth. The assertion that matters is in the
        // note above `mls_snapshot` — a snapshot that leaves the device is the
        // escrow again, whatever it is called.
    }

}

#[cfg(test)]
mod head_exports {
    use super::*;
    /// Same scratch, same out buffer, same lock as every other export test — see
    /// the note in `backup_exports`.
    use super::exports::LOCK;

    fn call(f: extern "C" fn(u32) -> u32, input: &str) -> Result<String, String> {
        let b = input.as_bytes();
        let p = alloc(b.len() as u32);
        // SAFETY: `alloc` just sized SCRATCH to exactly `b.len()` bytes.
        unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), p, b.len()) };
        let _ = f(b.len() as u32);
        let out = unsafe { OUT.clone() };
        let s = String::from_utf8_lossy(&out).into_owned();
        if erred() != 0 { Err(s) } else { Ok(s) }
    }

    fn seed() -> String { "11".repeat(32) }
    fn other() -> String { "22".repeat(32) }
    fn tail() -> String { "ab".repeat(32) }

    /// The first head an account writes: position 0, no tail. This is the floor
    /// the arc's compare-and-set enforces from then on, and until these exports
    /// existed no browser could produce one.
    #[test]
    fn an_empty_head_seals_and_opens() {
        let _g = LOCK.lock().unwrap();
        let blob = call(head_seal, &format!(
            r#"{{"seed":"{}","arc":"arc.example","position":0,"chain_gen":7}}"#, seed()
        )).expect("seals");
        // PHD1 || nonce(24) || ciphertext — the envelope the arc length-checks.
        assert!(blob.starts_with(&hex::encode(b"PHD1")), "magic: {}", &blob[..8]);
        let got = call(head_open, &format!(
            r#"{{"seed":"{}","blob":"{}","declared_position":0}}"#, seed(), blob
        )).expect("opens");
        let v: serde_json::Value = serde_json::from_str(&got).unwrap();
        assert_eq!(v["position"], 0);
        assert_eq!(v["arc"], "arc.example");
        assert_eq!(v["chain_gen"], 7, "the generation survives the seal");
        assert_eq!(v["v"], 2);
        assert_eq!(v["tail"], "0".repeat(64), "an empty chain has no tail");
    }

    /// The arc orders writes on a number it cannot decrypt, so the header it
    /// serves and the position sealed in the body are two separate claims. A
    /// client that believed the header would believe its chain was current.
    #[test]
    fn a_declared_position_that_disagrees_is_refused() {
        let _g = LOCK.lock().unwrap();
        let blob = call(head_seal, &format!(
            r#"{{"seed":"{}","arc":"a","position":5,"tail":"{}","chain_gen":0}}"#, seed(), tail()
        )).unwrap();
        let e = call(head_open, &format!(
            r#"{{"seed":"{}","blob":"{}","declared_position":41}}"#, seed(), blob
        )).unwrap_err();
        assert!(e.contains("declared head position 41"), "{e}");
        assert!(e.contains("served position 5"), "{e}");
    }

    /// The head key comes off the seed's STORAGE ROOT, so the words door reaches
    /// it — and a different seed reaches nothing.
    #[test]
    fn another_seed_opens_nothing() {
        let _g = LOCK.lock().unwrap();
        let blob = call(head_seal, &format!(
            r#"{{"seed":"{}","arc":"a","position":0,"chain_gen":0}}"#, seed()
        )).unwrap();
        let e = call(head_open, &format!(r#"{{"seed":"{}","blob":"{}"}}"#, other(), blob))
            .unwrap_err();
        assert!(e.contains("wrong seed or tampered"), "{e}");
    }

    /// Position and tail are two halves of one statement. A head that breaks the
    /// pairing seals perfectly well and can then never be opened, so it is caught
    /// on the way in rather than at the moment someone needs it.
    #[test]
    fn a_position_without_a_tail_is_refused_before_it_is_sealed() {
        let _g = LOCK.lock().unwrap();
        let e = call(head_seal, &format!(
            r#"{{"seed":"{}","arc":"a","position":5,"chain_gen":0}}"#, seed()
        )).unwrap_err();
        assert!(e.contains("does not agree with its tail"), "{e}");

        let e = call(head_seal, &format!(
            r#"{{"seed":"{}","arc":"a","position":0,"tail":"{}","chain_gen":0}}"#, seed(), tail()
        )).unwrap_err();
        assert!(e.contains("does not agree with its tail"), "{e}");
    }

    /// Four answers, and the fourth is not an alarm: `head_behind` is what a
    /// device that appended and died before updating its head leaves behind.
    #[test]
    fn the_verdict_has_four_answers() {
        let _g = LOCK.lock().unwrap();
        let ask = |walked: u64, wt: &str| {
            call(head_check, &format!(
                r#"{{"position":5,"tail":"{}","walked_position":{walked},"walked_tail":"{wt}"}}"#,
                tail()
            )).unwrap()
        };
        assert_eq!(ask(5, &tail()), r#"{"verdict":"whole"}"#);
        assert_eq!(ask(3, &"cd".repeat(32)), r#"{"verdict":"truncated","missing":2}"#);
        assert_eq!(ask(5, &"cd".repeat(32)), r#"{"verdict":"forked"}"#);
        assert_eq!(ask(9, &"cd".repeat(32)), r#"{"verdict":"head_behind","extra":4}"#);
    }
}

#[cfg(test)]
mod locator_exports {
    use super::*;
    /// Same scratch, same out buffer, same lock as every other export test.
    use super::exports::LOCK;

    fn call(f: extern "C" fn(u32) -> u32, input: &str) -> Result<String, String> {
        let b = input.as_bytes();
        let p = alloc(b.len() as u32);
        // SAFETY: `alloc` just sized SCRATCH to exactly `b.len()` bytes.
        unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), p, b.len()) };
        let _ = f(b.len() as u32);
        let out = unsafe { OUT.clone() };
        let s = String::from_utf8_lossy(&out).into_owned();
        if erred() != 0 { Err(s) } else { Ok(s) }
    }
    fn seed() -> String { "07".repeat(32) }
    fn prf() -> String { "09".repeat(32) }
    fn set(json: &str) -> serde_json::Value { serde_json::from_str(json).unwrap() }

    /// The export exists so no page computes HKDF for itself, and the set it
    /// returns must agree with the core exactly — a browser one byte out files
    /// its records at addresses its owner's phone will never look at.
    #[test]
    fn the_set_matches_the_core_exactly() {
        let _g = LOCK.lock().unwrap();
        use pacific_core::locator::{self, Record};
        let root = *locator::storage_root(&[7u8; 32]);
        let v = set(&call(locators, &format!(r#"{{"seed":"{}","prf":"{}"}}"#, seed(), prf())).unwrap());
        assert_eq!(v["storage_root"], hex::encode(root), "the root is returned, not implied");
        for (key, rec) in [("head", Record::Head), ("history", Record::History),
                           ("index", Record::Index)] {
            assert_eq!(v[key], hex::encode(locator::locator(&root, rec).unwrap()), "{key}");
        }
        assert!(v["chain"].is_null(), "no chain address without the head's generation: {v}");
        let g3 = set(&call(locators, &format!(r#"{{"seed":"{}","chain_gen":3}}"#, seed())).unwrap());
        assert_eq!(g3["chain"], hex::encode(locator::chain_locator(&root, 3, 0)));
        assert_eq!(v["acct_pk"], hex::encode(locator::acct_pk(&root)));
        assert_eq!(v["wrap"], hex::encode(locator::wrap_locator(&[9u8; 32])));
    }

    /// A device restoring from 24 words has no passkey yet and still needs the
    /// other four. Without a PRF the wrap address is ABSENT rather than derived
    /// from the seed — a seed-derived wrap address is one no passkey-only device
    /// could compute, which is the only kind of device the wrap exists for.
    #[test]
    fn without_a_prf_there_is_no_wrap_address() {
        let _g = LOCK.lock().unwrap();
        let v = set(&call(locators, &format!(r#"{{"seed":"{}"}}"#, seed())).unwrap());
        assert!(v["wrap"].is_null(), "a wrap address appeared without a PRF: {v}");
        assert!(v["head"].is_string());
    }

    /// First-write-claims, and every field that must be inside the signature:
    /// the bytes, the address, the arc, and the challenge.
    #[test]
    fn a_write_is_bound_to_its_address_its_bytes_its_arc_and_its_nonce() {
        let _g = LOCK.lock().unwrap();
        let v = set(&call(locators, &format!(r#"{{"seed":"{}"}}"#, seed())).unwrap());
        let head = v["head"].as_str().unwrap().to_string();
        let hist = v["history"].as_str().unwrap().to_string();
        let acct = v["acct_pk"].as_str().unwrap().to_string();
        let body = hex::encode(b"a sealed head");
        let sig = call(arc_write_sign, &format!(
            r#"{{"seed":"{}","locator":"{head}","audience":"arc.example","nonce":"n1","body":"{body}"}}"#,
            seed())).unwrap();

        let check = |loc: &str, aud: &str, nonce: &str, b: &str| call(arc_write_verify, &format!(
            r#"{{"recorded":"{acct}","offered":"{acct}","locator":"{loc}","audience":"{aud}","nonce":"{nonce}","body":"{b}","sig":"{sig}"}}"#));

        assert_eq!(check(&head, "arc.example", "n1", &body).unwrap(), r#"{"ok":true}"#);
        assert!(check(&head, "arc.example", "n1", &hex::encode(b"other")).is_err(), "body");
        assert!(check(&hist, "arc.example", "n1", &body).is_err(), "address");
        assert!(check(&head, "other.example", "n1", &body).is_err(), "audience");
        assert!(check(&head, "arc.example", "n2", &body).is_err(), "nonce");
    }

    /// A squat and a bug must not look the same to the arc. The reply stays
    /// opaque either way; the refusal the arc LOGS does not.
    #[test]
    fn a_squat_names_both_keys() {
        let _g = LOCK.lock().unwrap();
        let mine = set(&call(locators, &format!(r#"{{"seed":"{}"}}"#, seed())).unwrap());
        let other_seed = "42".repeat(32);
        let theirs = set(&call(locators, &format!(r#"{{"seed":"{other_seed}"}}"#)).unwrap());
        let head = mine["head"].as_str().unwrap();
        let body = hex::encode(b"x");
        let sig = call(arc_write_sign, &format!(
            r#"{{"seed":"{other_seed}","locator":"{head}","audience":"a","nonce":"n","body":"{body}"}}"#
        )).unwrap();
        let e = call(arc_write_verify, &format!(
            r#"{{"recorded":"{}","offered":"{}","locator":"{head}","audience":"a","nonce":"n","body":"{body}","sig":"{sig}"}}"#,
            mine["acct_pk"].as_str().unwrap(), theirs["acct_pk"].as_str().unwrap())).unwrap_err();
        assert!(e.contains("claimed by") && e.contains("offered"), "a squat must name both keys: {e}");
    }

    /// A BYO account has a storage root and no seed, and every address must come
    /// out the same as if a seed had produced that root. This is the property
    /// §06 exists to protect, exercised through the export the browser calls.
    #[test]
    fn a_storage_root_alone_gives_the_same_addresses() {
        let _g = LOCK.lock().unwrap();
        let root = hex::encode(*pacific_core::locator::storage_root(&[7u8; 32]));
        let from_seed = set(&call(locators, &format!(r#"{{"seed":"{}"}}"#, seed())).unwrap());
        let from_root = set(&call(locators, &format!(r#"{{"storage_root":"{root}"}}"#)).unwrap());
        assert_eq!(from_seed, from_root);
        assert!(call(locators, "{}").is_err(), "neither input is a refusal, not a default");
    }

    /// Unguessable without the predecessor. If these were derived, the chain
    /// would collapse into one stable per-user address at the blind relay.
    #[test]
    fn next_tags_are_fresh_every_time() {
        let _g = LOCK.lock().unwrap();
        let a = call(chain_next_tag, "").unwrap();
        let b = call(chain_next_tag, "").unwrap();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        let v = set(&call(locators, &format!(r#"{{"seed":"{}","chain_gen":0}}"#, seed())).unwrap());
        assert_ne!(a, v["chain"].as_str().unwrap(), "a next tag is the derived first chain tag");
    }

    /// A browser signs a publish through the core and nowhere else, and what it
    /// gets back is exactly what the relay's check accepts.
    #[test]
    fn a_browser_signed_publish_is_one_the_relay_accepts() {
        let _g = LOCK.lock().unwrap();
        let blob = "c2VhbGVk";
        let v = set(&call(relay_sign_pub, &format!(r#"{{"seed":"{}","blob":"{blob}"}}"#, seed())).unwrap());
        let (tag, sig) = (v["tag"].as_str().unwrap(), v["sig"].as_str().unwrap());
        assert_eq!(pacific_wire::address::verify_pub(tag, blob, sig), Ok(()));
        assert_eq!(
            pacific_wire::address::verify_pub(tag, "b3RoZXI", sig),
            Err(pacific_wire::address::PubRefusal::BadSignature),
            "the signature is over this blob, not a licence for any other"
        );
        // relay_address is the same address, raw.
        let raw = hex::decode(seed()).unwrap();
        let b = raw.as_slice();
        let p = alloc(b.len() as u32);
        unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), p, b.len()) };
        let _ = relay_address(32);
        assert_eq!(erred(), 0);
        assert_eq!(hex::encode(unsafe { OUT.clone() }), tag);
    }
}

#[cfg(test)]
mod spine_exports {
    use super::*;
    /// Same scratch, same out buffer, same lock as every other export test.
    use super::exports::LOCK;
    use pacific_core::spine::{self, Body, Joined};

    fn call(f: extern "C" fn(u32) -> u32, input: &str) -> Result<serde_json::Value, String> {
        let b = input.as_bytes();
        let p = alloc(b.len() as u32);
        // SAFETY: `alloc` just sized SCRATCH to exactly `b.len()` bytes.
        unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), p, b.len()) };
        let _ = f(b.len() as u32);
        let out = unsafe { OUT.clone() };
        let s = String::from_utf8_lossy(&out).into_owned();
        if erred() != 0 { Err(s) } else { serde_json::from_str(&s).map_err(|e| format!("{e}: {s}")) }
    }
    fn seed() -> String { "07".repeat(32) }
    fn root() -> [u8; 32] { *pacific_core::locator::storage_root(&[7u8; 32]) }
    /// Entry `index` naming group `[g; 32]`, joined at `floor`, sealed the one way.
    fn entry(index: u64, g: u8, floor: u64) -> ([u8; 32], Vec<u8>) {
        let body = Body::Group(Joined { group_id: vec![g; 32], first_epoch: floor });
        spine::place(&root(), 0, index, body).unwrap()
    }
    fn fetched(items: &[(u64, &[u8])]) -> String {
        let v: Vec<serde_json::Value> = items
            .iter()
            .map(|(i, b)| serde_json::json!({ "index": i, "blobs": [pacific_wire::blob_b64(b)] }))
            .collect();
        serde_json::Value::Array(v).to_string()
    }

    /// The browser and the phone must land on the same tag or never meet, and a
    /// wrong tag raises nothing anywhere. So the export is held to `spine::place`.
    #[test]
    fn addresses_are_the_ones_the_writer_publishes_to() {
        let _g = LOCK.lock().unwrap();
        let v = call(spine_addresses, &format!(r#"{{"seed":"{}","gen":0,"from":0,"count":3}}"#, seed())).unwrap();
        for i in 0..3u64 {
            let (at, _) = entry(i, 1, 0);
            assert_eq!(v["addresses"][i as usize]["index"], i);
            assert_eq!(
                v["addresses"][i as usize]["tag"],
                pacific_wire::address::Address::from_seed(&at).tag_hex(),
                "index {i}: the export's tag is the drain's tag"
            );
        }
        assert!(call(spine_addresses, &format!(r#"{{"seed":"{}","gen":0,"from":0,"count":257}}"#, seed())).is_err());
    }

    /// No head, or the first head at 0: the objects are named, and whether that is
    /// all of them cannot be told.
    #[test]
    fn without_a_head_that_vouches_the_objects_come_back_and_the_tail_is_unknown() {
        let _g = LOCK.lock().unwrap();
        let (_, a) = entry(0, 1, 0);
        let (_, b) = entry(1, 2, 5);
        let v = call(spine_read, &format!(r#"{{"seed":"{}","head":null,"fetched":{}}}"#, seed(), fetched(&[(0, &a), (1, &b)]))).unwrap();
        assert_eq!(v["recovery"]["tail"], "unknown");
        assert_eq!(v["recovery"]["found"], 2);
        assert_eq!(v["objects"][0]["group_id"], hex::encode([1u8; 32]));
        assert_eq!(v["objects"][1]["first_epoch"], 5, "the floor comes from the entry");
        assert_eq!(v["unopened"], 0);
    }

    /// A head at 3 whose tail is entry 2, with entry 1 lost: intact, one hole.
    #[test]
    fn a_hole_under_a_head_is_named_and_the_tail_is_intact() {
        let _g = LOCK.lock().unwrap();
        let (_, a) = entry(0, 1, 0);
        let (_, c) = entry(2, 3, 0);
        let tail = hex::encode(pacific_core::head::tail_hash(&c));
        let v = call(spine_read, &format!(
            r#"{{"seed":"{}","head":{{"position":3,"tail":"{tail}"}},"fetched":{}}}"#,
            seed(), fetched(&[(0, &a), (2, &c)])
        )).unwrap();
        assert_eq!(v["recovery"]["tail"], "intact");
        assert_eq!(v["recovery"]["holes"], serde_json::json!([1]));
        assert_eq!(v["recovery"]["extra"], 0);
    }

    /// A departure after the join marks the object LEFT — so a recovering device can
    /// skip folding it — and a join after the departure is a rejoin, not left.
    #[test]
    fn a_group_left_is_marked_and_a_rejoin_clears_it() {
        let _g = LOCK.lock().unwrap();
        use pacific_core::spine::Departed;
        let (_, joined) = entry(0, 1, 2);
        let (_, gone) = spine::place(&root(), 0, 1, Body::Left(Departed { group_id: vec![1; 32], last_epoch: 9 })).unwrap();
        let v = call(spine_read, &format!(r#"{{"seed":"{}","head":null,"fetched":{}}}"#, seed(), fetched(&[(0, &joined), (1, &gone)]))).unwrap();
        assert_eq!(v["objects"][0]["first_epoch"], 2);
        assert_eq!(v["objects"][0]["left_epoch"], 9, "exited: readable up to 9 and no further");
        assert_eq!(v["left"][0]["last_epoch"], 9);

        let (_, back) = entry(2, 1, 12);
        let v = call(spine_read, &format!(r#"{{"seed":"{}","head":null,"fetched":{}}}"#, seed(), fetched(&[(0, &joined), (1, &gone), (2, &back)]))).unwrap();
        assert_eq!(v["objects"][0]["left_epoch"], serde_json::Value::Null, "rejoined at 12, after leaving at 9");
        assert_eq!(v["objects"][0]["first_epoch"], 2, "the floor is still the first join");
    }

    /// The browser seals with the phone's encoder: what `spine_place` writes,
    /// `spine_read` opens, at the tag `spine_addresses` names — a round trip through
    /// the three exports with no second encoder anywhere.
    #[test]
    fn what_the_browser_places_the_reader_opens_at_the_address_the_walk_asks() {
        let _g = LOCK.lock().unwrap();
        let placed = call(spine_place, &format!(
            r#"{{"seed":"{}","gen":0,"index":3,"body":{{"group":{{"group_id":"{}","first_epoch":4}}}}}}"#,
            seed(), hex::encode([5u8; 32])
        )).unwrap();
        let addr = call(spine_addresses, &format!(r#"{{"seed":"{}","gen":0,"from":3,"count":1}}"#, seed())).unwrap();
        assert_eq!(placed["tag"], addr["addresses"][0]["tag"], "placed where the walk will look");
        // And it can be PUBLISHED: the relay refuses a pub its tag did not sign,
        // and the address seed handed back signs for exactly this entry's tag.
        let signed = call(relay_sign_pub, &format!(
            r#"{{"seed":"{}","blob":"{}"}}"#,
            placed["address_seed"].as_str().unwrap(), placed["blob"].as_str().unwrap()
        )).unwrap();
        assert_eq!(signed["tag"], placed["tag"], "the address seed signs for the tag the entry lives at");

        let fetched = serde_json::json!([{ "index": 3, "blobs": [placed["blob"]] }]);
        let v = call(spine_read, &format!(r#"{{"seed":"{}","head":null,"fetched":{fetched}}}"#, seed())).unwrap();
        assert_eq!(v["unopened"], 0);
        assert_eq!(v["objects"][0]["group_id"], hex::encode([5u8; 32]));
        assert_eq!(v["objects"][0]["first_epoch"], 4);

        let left = call(spine_place, &format!(
            r#"{{"seed":"{}","gen":0,"index":4,"body":{{"left":{{"group_id":"{}","last_epoch":7}}}}}}"#,
            seed(), hex::encode([5u8; 32])
        )).unwrap();
        let fetched = serde_json::json!([
            { "index": 3, "blobs": [placed["blob"]] },
            { "index": 4, "blobs": [left["blob"]] },
        ]);
        let v = call(spine_read, &format!(r#"{{"seed":"{}","head":null,"fetched":{fetched}}}"#, seed())).unwrap();
        assert_eq!(v["objects"][0]["left_epoch"], 7);

        assert!(call(spine_place, &format!(r#"{{"seed":"{}","gen":0,"index":0,"body":{{}}}}"#, seed())).is_err(),
            "a body that is neither is refused, in words");
    }

    /// A blob at our index that does not open under our key AT THAT INDEX is
    /// counted, not named — and never becomes an object.
    #[test]
    fn a_blob_that_does_not_open_is_counted_and_not_named() {
        let _g = LOCK.lock().unwrap();
        let (_, a) = entry(0, 1, 0);
        // Sealed for index 0, handed in as index 4: the index is in the AAD.
        let v = call(spine_read, &format!(r#"{{"seed":"{}","head":null,"fetched":{}}}"#, seed(), fetched(&[(4, &a)]))).unwrap();
        assert_eq!(v["unopened"], 1);
        assert_eq!(v["recovery"]["found"], 0);
        assert_eq!(v["objects"], serde_json::json!([]));
    }

    /// A `Pool` entry is part of the walk — it opens and is found — but names no
    /// object, and its signing key does not leave the export.
    #[test]
    fn a_pool_entry_is_walked_and_names_nothing() {
        let _g = LOCK.lock().unwrap();
        use pacific_core::spine::{Pool, PoolLeaf};
        let (_, a) = entry(0, 1, 0);
        let leaf = PoolLeaf { pool: [2; 32], welcome: vec![3], sig_sk: vec![0xAB; 32], kp_id: vec![4], kp_data: vec![5], epoch: 0 };
        let (_, p) = spine::place(&root(), 0, 1, Body::Pool(Pool {
            group_id: vec![6; 32], kind: "post".into(), arc: "wss://a".into(), pools: vec![leaf],
        })).unwrap();
        let v = call(spine_read, &format!(r#"{{"seed":"{}","head":null,"fetched":{}}}"#, seed(), fetched(&[(0, &a), (1, &p)]))).unwrap();
        assert_eq!(v["unopened"], 0);
        assert_eq!(v["recovery"]["found"], 2);
        assert_eq!(v["objects"].as_array().unwrap().len(), 1, "only the Group entry names an object");
        assert_eq!(v["left"], serde_json::json!([]));
        assert!(!v.to_string().contains(&hex::encode([0xABu8; 32])));
    }
}
