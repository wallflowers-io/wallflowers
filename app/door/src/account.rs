//! account — the session process (DR-3, S-13): one person's Node, for one session.
//!
//! The supervisor (`main.rs`) starts one of these per sign-in and proxies to it. The
//! split is not a preference: `paths::state_dir()` is read per call and every `Node`
//! handle resolves through it, so two accounts in one process would share a
//! directory. What the core forces is also what the production shape wants: a
//! session's seed lives in a process that can be killed.
//!
//! WHY IT EXISTS. A browser cannot hold an account safely: the code arrives from a
//! server on every load, and whatever that code holds, the server can take. Ruled
//! 23 September 2026 — the door is its own image, it handles the passkey and it
//! holds the account seed, and the browser holds a session and nothing else.
//!
//! WHAT IT COSTS, and it is not small: for a web account this process can read
//! everything, and it learns the PRF output, which with the public wrap is the seed
//! (CS-33). That is the trade, it is deliberate, and the product says so where a
//! person signs in (mdr/door.md §1). The iOS app is unaffected.
//!
//! HOW A SESSION BEGINS (mdr/door.md §4). The supervisor writes one line of config
//! to this process's stdin and keeps the pipe: its end is this process's end, so a
//! supervisor that dies takes its sessions with it. Nothing secret is in the
//! environment. This process makes a key for the attempt; the window seals the PRF
//! output to it; the wrap and the head come from the auth service; the Node signs
//! in through `Node::sign_in_from_wrap`; and the supervisor opens the session only
//! after this process has signed its challenge.
//!
//! Every route here answers only to the supervisor's bearer secret.

use crate::seal::{Sealed, SessionKey};
#[path = "keep.rs"]
mod keep;
#[path = "auth_worker.rs"]
mod auth_worker;
use axum::{
    extract::{Path, Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
    routing::{get, post},
    Json, Router,
};
use pacific_core::Node;
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{channel, Sender};
use std::time::Duration;

/// What the HTTP side may ask the Node's thread for. The `Node` never leaves its
/// thread: SQLite's connection is not `Sync`, and an actor is cheaper than making
/// it so.
enum Ask {
    Key(Reply<String>),
    /// The wrap opens under the PRF to this key: the seed is held, nothing is opened or
    /// resumed. `/i/sign` answers after it (D-34 (c)).
    Prove { attempt: String, pk: String, host: String, sealed: Sealed, back: Reply<serde_json::Value> },
    /// The proved person's Node: their sealed state if it is current and clean, else a
    /// fresh device. An error starting `TRANSIENT` is worth a retry.
    Open(Reply<serde_json::Value>),
    /// A session of this person has ended: its scope goes with it.
    Forget { who: String, back: Reply<()> },
    SignUp { name: String, back: Reply<serde_json::Value> },
    SignUpFinish { attempt: String, sealed: Sealed, back: Reply<serde_json::Value> },
    Sign { audience: String, nonce: String, back: Reply<String> },
    /// A public site's stream is open: watch its scope for that session alone, and
    /// answer the channel that moves with it (NC-41).
    Watch { scope: String, who: String, back: Reply<tokio::sync::watch::Receiver<u64>> },
    End(Reply<()>),
    Me { scope: Option<String>, who: String, back: Reply<serde_json::Value> },
    Graph { scope: Option<String>, who: String, back: Reply<serde_json::Value> },
    Kinds(Reply<Vec<KindOut>>),
    Draft { kind: String, back: Reply<DraftOut> },
    Mint { kind: String, draft: DraftIn, scope: Option<String>, who: String, back: Reply<String> },
    Apply { object: String, op: String, args: pacific_media::Args, scope: Option<String>, who: String, back: Reply<String> },
    /// A fresh key package under this identity, for an admitter to add it (A-3).
    Bundle(Reply<String>),
    /// A contact code (W-96): fresh key packages under this identity, one for a Site and one
    /// for each room an owner adds its person to, kept before the answer: the owner may add
    /// them long after this session ends.
    Code(Reply<(Vec<String>, u64)>),
    /// The Site's address: `slug` claimed for this account and bound to its Host at the
    /// auth service, signed by this person's key (D-52).
    Address { slug: String, host: String, scope: Option<String>, who: String, back: Reply<serde_json::Value> },
    /// Whether this person is on `site`'s roster, and its published address where their Node
    /// holds its Host (W-98: the supervisor's /v2/site/:site/items asks).
    SiteOf { site: String, back: Reply<serde_json::Value> },
    /// The owner adds someone by their contact bundle: the Arc's node to a Site (A-3).
    Add { object: String, by: AddBy, scope: Option<String>, back: Reply<String> },
    /// The owner seals a Site's or a room's history to a member by their contact bundle: the
    /// Arc's node, added to a room late. `dry` sends nothing and answers what would go.
    History { object: String, bundle: String, dry: bool, scope: Option<String>, back: Reply<serde_json::Value> },
    Batch { steps: Vec<Step>, scope: Option<String>, who: String, back: Reply<serde_json::Value> },
    /// The held connection delivered something (O-69): ingest it.
    Rang,
    /// The auth worker answered (O-69): the head stored, the seal's counter confirmed, or
    /// another writer's counter found.
    Authed(auth_worker::Authed),
}

/// A public site's token reaches its Site, the Site's parts, and what that token
/// minted, and nothing else of the person (RA-10; mdr/door.md §6). The Site is
/// the scope the supervisor passes; an empty one (no Site registered yet) leaves
/// only what the token minted.
fn in_scope(node: &Node, site: &str, minted: &std::collections::HashSet<String>) -> std::collections::HashSet<String> {
    let mut out = minted.clone();
    if !site.is_empty() {
        out.insert(site.to_string());
        if let Ok(view) = node.object_view(site) {
            let v: serde_json::Value = serde_json::from_str(&view).unwrap_or_default();
            for p in v["parts"].as_array().into_iter().flatten() {
                if let Some(id) = p["part"].as_str() {
                    out.insert(id.to_string());
                }
            }
        }
    }
    // Never the self record, whatever names it (Software Security): it is the person's
    // spine, not a site's. A store that cannot say which it is gives an empty scope.
    match node.self_object() {
        Ok(Some(own)) => {
            out.remove(&own);
        }
        Ok(None) => {}
        Err(_) => return Default::default(),
    }
    out
}

/// NC-95: WHAT A WRITE NAMES IS HELD TO THE SCOPE TOO, not only the object it writes to: a
/// site's token that could `base.setPart` its Site with any id it knew would bring that
/// object into its scope, and then read and write it. An arg names an object when the ICD
/// makes it a reference (`reference_args`) and its value is the id of one this Node holds:
/// a person's key, or an id held nowhere here, names none, and nor does a message's text.
fn names_outside(n: &Node, keep: &std::collections::HashSet<String>, op: &str, args: &pacific_media::Args) -> Option<String> {
    let refs = reference_args(op);
    args.iter().filter(|(k, _)| refs.contains(k.as_str())).find_map(|(k, v)| match v {
        pacific_media::ArgVal::Text(t) if !keep.contains(t) && n.object_kind(t).is_ok() => Some(format!("{t} (its {k}) is outside this site's scope")),
        _ => None,
    })
}

/// The args of `op` that name another object, as the ICD declares them: an arg with a
/// relation (`rel`), and an arg an edge takes an endpoint from (`arg:<name>`). Read from the
/// one catalogue /v2/icd serves, never restated.
fn reference_args(op: &str) -> std::collections::HashSet<String> {
    static ICD: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    let icd = ICD.get_or_init(|| serde_json::from_str(include_str!("../../../core/coordination/delta-graph.icd.json")).expect("the ICD"));
    let mut out = std::collections::HashSet::new();
    for table in ["kinds", "facets"] {
        for k in icd[table].as_object().into_iter().flat_map(|t| t.values()) {
            let Some(o) = k["ops"].get(op) else { continue };
            for (a, spec) in o["args"].as_object().into_iter().flatten() {
                if spec["rel"].as_array().is_some_and(|r| !r.is_empty()) {
                    out.insert(a.clone());
                }
            }
            for e in o["edges"].as_array().into_iter().flatten() {
                for side in ["from", "to"] {
                    if let Some(a) = e[side].as_str().and_then(|v| v.strip_prefix("arg:")) {
                        out.insert(a.to_string());
                    }
                }
            }
        }
    }
    out
}

/// A kept object's view names nothing outside the scope either (Software Security, after
/// NC-81): its parts, backlinks and affiliations are cut to targets inside it, and a parent
/// outside it is dropped, or a site's reader learns the id of another Site an in-scope
/// object links to. `get_mut`, so a view that is not an object stays as it is.
fn cut_to_scope(view: &mut serde_json::Value, keep: &std::collections::HashSet<String>) {
    for (field, id) in [("parts", "part"), ("backlinks", "object"), ("affiliations", "peer")] {
        if let Some(list) = view.get_mut(field).and_then(|v| v.as_array_mut()) {
            list.retain(|e| e[id].as_str().is_some_and(|x| keep.contains(x)));
        }
    }
    if let Some(parent) = view.get_mut("parent") {
        if parent["parent"].as_str().is_some_and(|x| !keep.contains(x)) {
            *parent = serde_json::Value::Null;
        }
    }
}

type Reply<T> = std::sync::mpsc::Sender<Result<T, String>>;

#[derive(Serialize, Clone)]
struct KindOut {
    kind: String,
    mintable: bool,
    /// WHY NOT, in the core's own vocabulary (`mint::NoMint`), or null when it
    /// mints. A consumer needs the reason, not the bit: `paired` is not "you
    /// cannot have one", it is "this one is formed by two people, not authored
    /// by one", and that is a different button.
    #[serde(skip_serializing_if = "Option::is_none")]
    no_mint: Option<&'static str>,
}

/// The draft a mint takes, by `MintDraft`'s OWN field names — the same names
/// `core-wasm`'s `draft_of` accepts, so the two surfaces ask for one thing. An
/// unknown field is a refusal rather than a silent drop: a caller who types into a
/// field the mint does not carry must be told, not humoured.
#[derive(Deserialize, Default, Clone)]
#[serde(deny_unknown_fields)]
struct DraftIn {
    #[serde(default)] name: String,
    #[serde(default)] descriptor: String,
    #[serde(default)] form: String,
    #[serde(default)] link: String,
    #[serde(default)] shape: String,
    #[serde(default)] card: String,
    #[serde(default)] category: String,
    #[serde(default)] start_ms: i64,
    #[serde(default)] end_ms: i64,
    #[serde(default)] venue: String,
    #[serde(default)] recurrence: String,
    #[serde(default)] lineup: String,
    #[serde(default)] placed: bool,
    #[serde(default)] lat: f64,
    #[serde(default)] lng: f64,
    #[serde(default)] status: String,
    #[serde(default)] stance: String,
    #[serde(default)] price: String,
    #[serde(default)] place: String,
}

impl DraftIn {
    fn to_draft(&self) -> pacific_core::mint::MintDraft {
        pacific_core::mint::MintDraft {
            name: self.name.clone(),
            descriptor: self.descriptor.clone(),
            form: self.form.clone(),
            link: self.link.clone(),
            shape: self.shape.clone(),
            card: self.card.clone(),
            category: self.category.clone(),
            start_ms: self.start_ms,
            end_ms: self.end_ms,
            venue: self.venue.clone(),
            recurrence: self.recurrence.clone(),
            lineup: self.lineup.clone(),
            placed: self.placed,
            lat: self.lat,
            lng: self.lng,
            status: self.status.clone(),
            icon: Vec::new(),
            banner: Vec::new(),
            stance: self.stance.clone(),
            price: self.price.clone(),
            place: self.place.clone(),
        }
    }
}

#[derive(Serialize, Clone)]
struct FieldOut {
    /// the op's own argument name, as the ICD has it; a name-only kind's `name`, its own name
    arg: String,
    /// the draft field that feeds it, or null when the mint cannot carry it
    draft: Option<String>,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    values: Option<Vec<String>>,
}

#[derive(Serialize, Clone)]
struct DraftOut {
    kind: String,
    mintable: bool,
    /// a Forum is minted NAME-ONLY: its name rides the MLS group context and
    /// authoring its op 0 would post a message
    name_only: bool,
    fields: Vec<FieldOut>,
}

#[derive(Deserialize)]
struct MintIn {
    kind: String,
    #[serde(default)]
    draft: DraftIn,
}

/// Bring the self record up to date: it exists, and it names every object this
/// account's spine names. Idempotent and best-effort — a refusal here must not
/// fail the write that prompted it, and the next one tries again. A spine this
/// device could not join is never replaced by a record minted here: that would
/// give the person two (`Node::sign_in_from_wrap`).
fn settle(node: &Node, rt: &tokio::runtime::Runtime, spine_unreached: bool) {
    // CONVERGE FIRST: what others wrote is in the mailbox until something drains it.
    if let Err(e) = rt.block_on(node.sync_once()) {
        eprintln!("door: sync failed — {e}");
    }
    if spine_unreached {
        return;
    }
    if let Err(e) = node.ensure_self_object() {
        eprintln!("door: no self record — {e}");
        return;
    }
    match rt.block_on(node.reconcile_joined()) {
        Ok(0) => {}
        Ok(n) => println!("door: named {n} object(s) on the self record"),
        Err(e) => eprintln!("door: the self record could not be reconciled — {e}"),
    }
    let _ = publish_card(node, rt);
    hydrate(node, rt, true);
}

/// THE MEMBER'S CARD (O-77), beside the vertebrae: into every held group and forum where it
/// is missing or stale, so a join, a name or a new picture reaches the others at once rather
/// than at the next sync. Idempotent, and best-effort, as `settle` is.
fn publish_card(node: &Node, rt: &tokio::runtime::Runtime) -> usize {
    // Boxed where it is built (core's reconcile_profiles_boxed): block_on pins what it is given
    // on the actor's stack, and this future is a write's worth.
    match rt.block_on(node.reconcile_profiles_boxed()) {
        Ok(s) => {
            if s.published > 0 {
                println!("door: the card published into {} object(s)", s.published);
            }
            s.published
        }
        Err(e) => {
            eprintln!("door: the card could not be reconciled — {e}");
            0
        }
    }
}

/// The person's name as their self record holds it, the cached name before the record has one:
/// a name the webapp writes on the record (group.setProfile) is read back at once.
fn my_name(node: &Node) -> String {
    node.my_profile().map(|(name, _, _)| name).unwrap_or_default()
}

/// THE FACE IS HYDRATED FROM THE SITE (O-48): each Site this person owns that has a
/// Host carries the Site's own events and posts as the Host's items, from this device,
/// the owner's. Idempotent, so an unchanged account writes nothing, and best-effort, as
/// `settle` is. What a Site names that stays off its Face is said after a write or a
/// sign-in (`say_left`), not on every tick.
fn hydrate(node: &Node, rt: &tokio::runtime::Runtime, say_left: bool) {
    hydrate_with(node, rt, say_left, false)
}

/// `hydrate`, its writes local commits when `local` (O-69): a write's own, before its
/// answer, so the Face has the write when it is answered; its tail sends them.
fn hydrate_with(node: &Node, rt: &tokio::runtime::Runtime, say_left: bool, local: bool) {
    let all = if local { rt.block_on(node.host_sync_all_local()) } else { rt.block_on(node.host_sync_all()) };
    let all = match all {
        Ok(all) => all,
        Err(e) => return eprintln!("door: the Sites' faces were not hydrated — {e}"),
    };
    for (site, done) in all {
        match done {
            Ok(s) => {
                if !s.put.is_empty() || !s.withdrawn.is_empty() {
                    println!("door: Site {site}'s Host: {} item(s) put, {} withdrawn", s.put.len(), s.withdrawn.len());
                }
                for (id, why) in s.left.iter().filter(|_| say_left) {
                    eprintln!("door: Site {site} names {id}, left off its Face: {why}");
                }
            }
            Err(e) => eprintln!("door: Site {site}'s Host was not hydrated — {e}"),
        }
    }
}

/// What a public site's stream watches: its scope's generations, which move when its scope
/// does and at no other time (NC-41).
fn scope_generation(node: &Node, scope: &std::collections::HashSet<String>) -> u64 {
    let mut ids: Vec<Vec<u8>> = scope.iter().filter_map(|id| hex::decode(id).ok()).collect();
    ids.sort();
    node.dir.generation_of(&ids).unwrap_or(0)
}

/// THE STREAM MOVES WITH G (O-69): the store's generation is compared, and only when it has
/// moved is each public site's own scope compared (NC-41). Nothing moved, nothing is sent.
/// Whether it moved.
fn moved(
    n: &Node,
    seen: &mut u64,
    version: &mut u64,
    bump: &tokio::sync::watch::Sender<u64>,
    watchers: &mut std::collections::HashMap<String, Watcher>,
    minted: &std::collections::HashMap<String, std::collections::HashSet<String>>,
) -> bool {
    let g = match n.dir.generation() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("door: the store's generation could not be read — {e}");
            return false;
        }
    };
    if g == *seen {
        return false;
    }
    *seen = g;
    *version += 1;
    let _ = bump.send(*version);
    for (who, w) in watchers.iter_mut() {
        let now = scope_generation(n, &in_scope(n, &w.scope, minted.get(who).unwrap_or(&NONE)));
        if now != w.seen {
            w.seen = now;
            w.version += 1;
            let _ = w.tx.send(w.version);
        }
    }
    true
}

/// The held connection's bell: one `Rang` waits on the actor's queue at most, whatever is
/// delivered, so a flood costs one ingest at a time (O-69).
fn ringer(tx: &Sender<Ask>, ringing: &std::sync::Arc<std::sync::atomic::AtomicBool>) -> Box<dyn Fn() + Send + Sync> {
    let (tx, ringing) = (tx.clone(), ringing.clone());
    Box::new(move || {
        if !ringing.swap(true, std::sync::atomic::Ordering::SeqCst) {
            let _ = tx.send(Ask::Rang);
        }
    })
}

/// How long a new session waits for the relay to confirm its held connection before its
/// first pass: past it, the pass drains everything anyway, and the first ring catches up.
const LIVE_WITHIN: Duration = Duration::from_secs(3);

/// THE RECONCILER'S INTERVAL (O-69): a full pass under the held connection, for the leases,
/// the pool and whatever the connection missed. An availability setting (condition 8).
const RECONCILE: Duration = Duration::from_secs(60);

/// WHICH FIELDS A MINT ACTUALLY CARRIES, discovered rather than declared. Every
/// `MintDraft` field is set to a sentinel, the core's own `profile_args` is asked
/// what op 0 would carry, and each argument is matched back to the sentinel that
/// produced it. So this answers with what the CORE does, not with a table kept
/// here: a field the mint drops shows as `draft: null` and the page can refuse to
/// ask for it, and a field the core starts carrying appears with no edit here.
fn draft_shape(kind: pacific_core::object::ObjectKind) -> DraftOut {
    use pacific_core::object::ObjectKind;
    let mintable = pacific_core::mint::is_mintable(kind);
    let mut probe = pacific_core::mint::MintDraft::default();
    let marks: Vec<(&str, &str)> = vec![
        ("name", "\u{a7}name"), ("descriptor", "\u{a7}descriptor"), ("form", "\u{a7}form"),
        ("link", "\u{a7}link"), ("shape", "\u{a7}shape"), ("card", "\u{a7}card"),
        ("category", "\u{a7}category"), ("venue", "\u{a7}venue"),
        ("recurrence", "\u{a7}recurrence"), ("lineup", "\u{a7}lineup"),
        ("status", "\u{a7}status"), ("stance", "\u{a7}stance"), ("price", "\u{a7}price"),
        ("place", "\u{a7}place"),
    ];
    probe.name = marks[0].1.into();
    probe.descriptor = marks[1].1.into();
    probe.form = marks[2].1.into();
    probe.link = marks[3].1.into();
    probe.shape = marks[4].1.into();
    probe.card = marks[5].1.into();
    probe.category = marks[6].1.into();
    probe.venue = marks[7].1.into();
    probe.recurrence = marks[8].1.into();
    probe.lineup = marks[9].1.into();
    probe.status = marks[10].1.into();
    probe.stance = marks[11].1.into();
    probe.price = marks[12].1.into();
    probe.place = marks[13].1.into();
    probe.start_ms = 8_101;
    probe.end_ms = 8_102;

    let mut fields = Vec::new();
    if let Some((_op, args)) = pacific_core::mint::profile_args(kind, &probe) {
        for (arg, val) in args.iter() {
            let (from, ty) = match val {
                pacific_media::ArgVal::Text(s) => (
                    marks.iter().find(|(_, m)| m == s).map(|(f, _)| f.to_string()),
                    "text",
                ),
                pacific_media::ArgVal::Int(n) => (
                    match n {
                        8_101 => Some("start_ms".to_string()),
                        8_102 => Some("end_ms".to_string()),
                        _ => None,
                    },
                    "int",
                ),
            };
            // A CLOSED VOCABULARY IS THE MINT'S, not the reducer's. The probe's
            // own sentinel is the join: the `MintField` whose slot still holds
            // this value is the field that produced the arg, so no table maps
            // the two. It matters — `GroupShape::parse` accepts `individual`
            // and `mint::fields(Group)` deliberately does not offer it, because
            // an individual is a person, not a Space you create.
            let values = match val {
                pacific_media::ArgVal::Text(s) => pacific_core::mint::fields(kind)
                    .iter()
                    .find(|f| {
                        let sentinel = probe.text(f.key);
                        !sentinel.is_empty() && sentinel == s
                    })
                    .and_then(|f| match f.input {
                        pacific_core::mint::MintInput::Choice(v) => {
                            Some(v.iter().map(|x| x.to_string()).collect())
                        }
                        _ => None,
                    }),
                _ => None,
            };
            fields.push(FieldOut { arg: arg.clone(), draft: from, kind: ty, values });
        }
    }
    fields.sort_by(|a, b| a.arg.cmp(&b.arg));
    let name_only = mintable && fields.is_empty() && kind == ObjectKind::Forum;
    // A name-only kind takes its name all the same, into its MLS group context: listed, so a
    // form built from these fields draws it.
    if name_only {
        fields.push(FieldOut { arg: "name".into(), draft: Some("name".into()), kind: "text", values: None });
    }
    DraftOut { kind: kind.name().to_string(), mintable, name_only, fields }
}

/// WHAT THIS ACCOUNT HOLDS, from the directory's own readers: no archive, no blob,
/// and nothing shipped that a reader did not ask for. `object_view` folds each
/// object through the core's one renderer; one that will not fold is named with the
/// core's words, not dropped.
fn graph_of(node: &Node) -> Result<serde_json::Value, String> {
    let mut objects = Vec::new();
    // EVERY GROUP THIS ACCOUNT IS IN, connections included: `objects_named` leaves
    // them out by design, and they are the one edge a person makes by hand.
    let mut held: Vec<(String, String, String)> = node.objects_named().map_err(|e| e.to_string())?;
    // A pair holds a CHANNEL and a CHAT; the door names the role, the page does not
    // read kinds to guess it.
    let mut peers: std::collections::HashMap<String, String> = Default::default();
    let mut roles: std::collections::HashMap<String, &str> = Default::default();
    for (peer, peer_name, _connected) in node.connections().unwrap_or_default() {
        if let Ok(Some((id, kind))) = node.connection_object(&peer) {
            peers.insert(id.clone(), hex::encode(peer));
            roles.insert(id.clone(), "channel");
            if !held.iter().any(|(h, _, _)| *h == id) {
                held.push((id, kind, peer_name));
            }
        }
        if let Ok(Some(id)) = node.conversation_with(&peer) {
            peers.insert(id.clone(), hex::encode(peer));
            roles.insert(id, "chat");
        }
    }
    for (id, kind, name) in held {
        let owner = node.object_owner(&id).ok().flatten().map(hex::encode).unwrap_or_default();
        let members: Vec<String> =
            node.object_members(&id).unwrap_or_default().iter().map(hex::encode).collect();
        let (view, why) = match node.object_view(&id) {
            Ok(v) => (serde_json::from_str(&v).unwrap_or(serde_json::Value::Null), None),
            Err(e) => (serde_json::Value::Null, Some(e.to_string())),
        };
        objects.push(serde_json::json!({
            "id": id, "kind": kind, "name": name,
            "owner": owner, "members": members,
            "peer": peers.get(&id), "role": roles.get(&id),
            "folds": why.is_none(), "why": why, "view": view,
        }));
    }
    let spine: Vec<serde_json::Value> = node
        .spine_entries()
        .unwrap_or_default()
        .into_iter()
        .map(|(index, e)| serde_json::json!({ "index": index, "object": hex::encode(e.body.group_id()) }))
        .collect();
    Ok(serde_json::json!({
        "me": { "pk": node.identity_key(), "display_name": my_name(node) },
        "objects": objects,
        "spine": spine,
    }))
}

/// What `resume` found, in the core's names (mdr/door.md §4 step 3).
fn resumed_json(r: &pacific_core::resumption::Resumed) -> serde_json::Value {
    use pacific_core::head::ChainVerdict as V;
    use pacific_core::resumption::Outcome as O;
    let verdict = r.verdict.as_ref().map(|v| match v {
        V::Whole => serde_json::json!({ "verdict": "Whole" }),
        V::Truncated { missing } => serde_json::json!({ "verdict": "Truncated", "missing": missing }),
        V::Forked => serde_json::json!({ "verdict": "Forked" }),
        V::HeadBehind { extra } => serde_json::json!({ "verdict": "HeadBehind", "extra": extra }),
    });
    let objects: Vec<serde_json::Value> = r
        .objects
        .iter()
        .map(|o| {
            let (outcome, detail) = match &o.outcome {
                O::Joined { from_epoch } => ("Joined", serde_json::json!({ "from_epoch": from_epoch })),
                O::AlreadyHeld => ("AlreadyHeld", serde_json::Value::Null),
                O::PoolExhausted => ("PoolExhausted", serde_json::Value::Null),
                O::WayInMissing => ("WayInMissing", serde_json::Value::Null),
                O::WayInUnreadable(why) => ("WayInUnreadable", serde_json::json!({ "why": why })),
                O::JoinFailed(why) => ("JoinFailed", serde_json::json!({ "why": why })),
            };
            serde_json::json!({ "object": o.object, "kind": o.kind, "outcome": outcome, "detail": detail })
        })
        .collect();
    serde_json::json!({ "chain": verdict, "spine": r.spine, "objects": objects })
}

/// The one line the supervisor writes on stdin. None of it is a secret but `secret`,
/// which is why it is here and not in the environment.
#[derive(Deserialize)]
struct Cfg {
    /// The supervisor's bearer: the only caller this process answers.
    secret: String,
    /// The auth service's origin, e.g. `https://arc.wallflowers.io`.
    auth: String,
    /// Its host, as the wrap's AAD and the passkey's handle name it.
    host: String,
    /// The Door's own host: the one audience this process signs a challenge for.
    door: String,
    relay: String,
    #[serde(default = "sync_secs")]
    sync_secs: u64,
    /// Where this Door keeps each person's sealed state (D-34 (c)); none, and nothing is
    /// kept between sessions.
    #[serde(default)]
    seals: Option<String>,
    /// The supervisor's build and image (NC-80): a session is both, or refuses.
    #[serde(default)]
    build: Option<String>,
    #[serde(default)]
    image: Option<String>,
    /// This session's fold cache bound, MiB (O-69).
    #[serde(default)]
    fold_cache_mib: Option<u64>,
    /// training_wheels' Delta tap (P0): report what the Node holds on the leash, where the
    /// supervisor reads it.
    #[serde(default)]
    tap: bool,
    /// How long a contact code's key packages live, seconds (W-96).
    #[serde(default = "code_secs")]
    code_secs: u64,
}

fn code_secs() -> u64 {
    1800
}

/// What a proof holds until the Node opens: the PRF output, the wrap, and the seed it
/// opened to. Dropped (and zeroed) once the Node is open.
struct Proved {
    prf: zeroize::Zeroizing<[u8; 32]>,
    wrap: Vec<u8>,
    seed: zeroize::Zeroizing<[u8; 32]>,
    id: pacific_core::identity::Identity,
    pk: String,
    host: String,
}

/// An open that may succeed if asked again: the auth service or the relay did not answer.
const TRANSIENT: &str = "try again: ";
/// A contact code past its life (W-96): /v2/add answers it 410, in these words.
const EXPIRED: &str = "the contact code has expired";

/// A nonce from the auth service, signed by this person's key: the head's and the seal's
/// writes, and the seal's read.
fn auth_sig(rt: &tokio::runtime::Runtime, http: &reqwest::Client, cfg: &Cfg, id: &pacific_core::identity::Identity) -> Result<(String, String), String> {
    let nonce = rt.block_on(crate::auth::challenge(http, &cfg.auth, &cfg.host))?;
    let sig = hex::encode(id.sign(&pacific_core::identity::auth_payload(&cfg.host, &id.identity_key(), &nonce)));
    Ok((nonce, sig))
}

/// The auth service's compare-and-set for the last seal, if it has not confirmed it.
/// `false` means another writer moved the counter: this process stops and ends.
fn confirm(rt: &tokio::runtime::Runtime, http: &reqwest::Client, cfg: &Cfg, id: &pacific_core::identity::Identity, k: &mut keep::Keeper) -> bool {
    if !k.unconfirmed() {
        return true;
    }
    let pk = hex::encode(id.identity_pk());
    let put = crate::auth::again(|| {
        auth_sig(rt, http, cfg, id)
            .and_then(|(nonce, sig)| rt.block_on(crate::auth::put_seal(http, &cfg.auth, &pk, &cfg.door, &nonce, &sig, k.held, k.sealed)))
    });
    match put {
        Ok(crate::auth::Counted::Stored) => {
            k.held = k.sealed;
            true
        }
        Ok(crate::auth::Counted::Moved { held }) => {
            eprintln!("door: the seal's counter moved to {held} under this process: another writer; it ends");
            false
        }
        Err(e) => {
            eprintln!("door: the seal is kept and not yet confirmed, retried each tick — {}", redacted(&e, cfg));
            true
        }
    }
}

/// The seal's counter confirmed at the auth service, or why not: a sign-up answers only
/// once it is (O-74, point 3). A service that cannot be reached is asked again, 1 s and
/// 2 s on; the same counter again is a retry to the service, never a second move. All of
/// it within CONFIRM_WITHIN (Software Security, (d)), well inside the supervisor's 60 s
/// wait for this answer, so an answer carrying the words is never cut off.
const CONFIRM_WITHIN: Duration = Duration::from_secs(20);
fn confirm_now(rt: &tokio::runtime::Runtime, http: &reqwest::Client, cfg: &Cfg, id: &pacific_core::identity::Identity, k: &mut keep::Keeper) -> Result<(), String> {
    let pk = hex::encode(id.identity_pk());
    let until = std::time::Instant::now() + CONFIRM_WITHIN;
    let (held, counter) = (k.held, k.sealed);
    let mut why = "not answered in time".to_string();
    for wait in [0u64, 1, 2] {
        std::thread::sleep(Duration::from_secs(wait));
        let Some(left) = until.checked_duration_since(std::time::Instant::now()) else { break };
        // The timeout is made inside the runtime: a tokio timer made outside one panics.
        let put = rt.block_on(async {
            tokio::time::timeout(left, async {
                let nonce = crate::auth::challenge(http, &cfg.auth, &cfg.host).await?;
                let sig = hex::encode(id.sign(&pacific_core::identity::auth_payload(&cfg.host, &id.identity_key(), &nonce)));
                crate::auth::put_seal(http, &cfg.auth, &pk, &cfg.door, &nonce, &sig, held, counter).await
            })
            .await
        });
        match put {
            Err(_) => break,
            Ok(Ok(crate::auth::Counted::Stored)) => {
                k.held = counter;
                return Ok(());
            }
            Ok(Ok(crate::auth::Counted::Moved { held })) => return Err(format!("its counter is at {held} under another writer")),
            Ok(Err(e)) => why = e,
        }
    }
    Err(why)
}

/// A failure's detail for this process's log, without the auth service's URLs (they
/// carry the person's key) or the seals directory's path (Software Security, (e)).
fn redacted(e: &str, cfg: &Cfg) -> String {
    let mut out: String = e
        .split_inclusive(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .map(|w| if w.starts_with("http://") || w.starts_with("https://") { "the auth service " } else { w })
        .collect();
    if let Some(dir) = cfg.seals.as_deref().filter(|d| !d.is_empty()) {
        out = out.replace(dir, "the seals directory");
    }
    out
}

/// A client's write is kept before it is acknowledged (Software Security, Q6): sealed to
/// disk, then confirmed. `Err` means it could not be kept: the session ends rather than
/// acknowledge it.
fn keep_write(rt: &tokio::runtime::Runtime, http: &reqwest::Client, cfg: &Cfg, n: &Node, keeper: &mut Option<keep::Keeper>) -> Result<(), String> {
    let Some(k) = keeper.as_mut() else { return Ok(()) };
    k.seal(n, &cfg.door)?;
    if confirm(rt, http, cfg, &n.id, k) {
        Ok(())
    } else {
        Err("another process holds this person's state".into())
    }
}

/// The auth worker (O-69), once the session is in: it signs as the person, so it starts
/// from the Node's seed, and answers on the actor's queue.
fn start_auth(cfg: &Cfg, n: &Node, bell: &Sender<Ask>) -> Option<auth_worker::Worker> {
    let seed = n.id.seed_bytes()?;
    let bell = bell.clone();
    let setup = auth_worker::Setup { auth: cfg.auth.clone(), host: cfg.host.clone(), door: cfg.door.clone() };
    Some(auth_worker::Worker::start(setup, seed, Box::new(move |a| {
        let _ = bell.send(Ask::Authed(a));
    })))
}

/// THE HEAD, STORED OFF THE ACTOR (O-69): asked of the auth worker when it has moved; with
/// none, stored here as before.
fn head_later(
    n: &Node,
    stored: &mut Option<u64>,
    auth: Option<&auth_worker::Worker>,
    rt: &tokio::runtime::Runtime,
    http: &reqwest::Client,
    cfg: &Cfg,
) {
    let Some(w) = auth else { return store_head(rt, http, cfg, n, stored) };
    match n.head_update() {
        Ok(h) if stored.is_none_or(|s| h.position > s) => w.head(h.position, h.sealed),
        Ok(_) => {}
        Err(e) => eprintln!("door: no head — {e}"),
    }
}

/// THE SEAL'S COUNTER, CONFIRMED OFF THE ACTOR (O-69): the seal is on disk already (D-34 (c));
/// the worker's answer ends the session if another writer moved the counter. With no worker,
/// confirmed here as before, and `false` ends it now.
fn confirm_later(
    k: &mut keep::Keeper,
    auth: Option<&auth_worker::Worker>,
    rt: &tokio::runtime::Runtime,
    http: &reqwest::Client,
    cfg: &Cfg,
    id: &pacific_core::identity::Identity,
) -> bool {
    match auth {
        Some(w) => {
            if k.unconfirmed() {
                w.confirm(k.held, k.sealed);
            }
            true
        }
        None => confirm(rt, http, cfg, id, k),
    }
}

/// Kept before it is answered (Software Security, Q6): the Sites' faces hydrated from the
/// write as local commits (O-48), then all of it sealed to this disk. Its confirm at the
/// auth service is the write's tail, after the answer.
fn seal_write(n: &Node, rt: &tokio::runtime::Runtime, cfg: &Cfg, keeper: &mut Option<keep::Keeper>, spine_unreached: bool) -> Result<(), String> {
    if !spine_unreached {
        hydrate_with(n, rt, true, true);
    }
    keeper.as_mut().map_or(Ok(()), |k| k.seal(n, &cfg.door))
}

/// WHAT A WRITE LEAVES ONCE IT IS ANSWERED (O-69): its outbox to send, and whether it
/// made objects (which need their pool leaves, and the self record to name them). Run
/// when no ask waits, and within TAIL_WITHIN of the first answer however busy the
/// session is.
#[derive(Default)]
struct Tail {
    made: bool,
    since: Option<std::time::Instant>,
}

const TAIL_WITHIN: Duration = Duration::from_millis(250);

impl Tail {
    fn add(&mut self, made: bool) {
        self.made |= made;
        self.since.get_or_insert_with(std::time::Instant::now);
    }

    fn due(&self) -> bool {
        self.since.is_none_or(|s| s.elapsed() >= TAIL_WITHIN)
    }
}

/// A WRITE'S TAIL (O-69), after its answer: its Deltas published on the held session
/// and sealed again at once (the sent ledger rides the seal, NC-76), the self record
/// named after a mint, the member's card published after them where it is stale (O-77), the
/// Sites' faces hydrated, the head stored, the seal confirmed.
/// `false`: another writer moved this person's counter, and the session ends.
#[allow(clippy::too_many_arguments)]
fn after_write(
    t: Tail,
    n: &Node,
    rt: &tokio::runtime::Runtime,
    http: &reqwest::Client,
    cfg: &Cfg,
    keeper: &mut Option<keep::Keeper>,
    stored: &mut Option<u64>,
    spine_unreached: bool,
    auth: Option<&auth_worker::Worker>,
) -> bool {
    // A new object has no pool leaf until the upkeep gives it one, and until then nothing
    // of it is sent: the upkeep first, then every outbox (the objects a mint made for its
    // parts are among them).
    if t.made {
        if let Err(e) = rt.block_on(n.upkeep()) {
            eprintln!("door: the resumption upkeep failed; the next sync tries again — {e}");
        }
    }
    let sent = match rt.block_on(n.publish_all_pending()) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("door: a write stays in the outbox for the next sync — {e}");
            0
        }
    };
    if sent > 0 {
        let ms = t.since.map(|s| s.elapsed().as_millis()).unwrap_or(0);
        eprintln!("door: {sent} delta(s) at the relay {ms} ms after the answer");
    }
    // The card, AFTER the write's own Deltas are at the relay: where the write made it stale
    // (a name, a picture) or missing (a new object), it follows at once rather than at the next
    // sync. Memoised in core: an unchanged card in an unchanged set of objects walks nothing.
    if !spine_unreached && publish_card(n, rt) > 0 {
        if let Err(e) = rt.block_on(n.publish_all_pending()) {
            eprintln!("door: the card stays in the outbox for the next sync — {e}");
        }
    }
    if t.made && !spine_unreached {
        match n.ensure_self_object() {
            Err(e) => eprintln!("door: no self record — {e}"),
            Ok(_) => match rt.block_on(n.reconcile_joined()) {
                Ok(0) => {}
                Ok(k) => println!("door: named {k} object(s) on the self record"),
                Err(e) => eprintln!("door: the self record could not be reconciled — {e}"),
            },
        }
    }
    if let Some(k) = keeper.as_mut() {
        if k.behind(n) {
            if let Err(e) = k.seal(n, &cfg.door) {
                eprintln!("door: {}", redacted(&e, cfg));
            }
        }
    }
    head_later(n, stored, auth, rt, http, cfg);
    keeper.as_mut().is_none_or(|k| confirm_later(k, auth, rt, http, cfg, &n.id))
}

/// Why a sealed state was not opened: worth a retry, or refused for good (the file goes).
#[derive(Debug)]
enum Unopened {
    Transient(String),
    Refused(String, u64),
}

/// A failure of THIS DEVICE (its storage, its build) or of the relay is worth a retry and
/// keeps the seal, which may be the only copy of an owner's chain; only the blob's own
/// refusal deletes it (NC-75). The class is core's, the one the drain keeps its cursor by.
fn unopened(e: pacific_core::CoreError, counter: u64) -> Unopened {
    match e {
        pacific_core::CoreError::Transport(e) => Unopened::Transient(format!("the relay could not be reached: {e}")),
        e if e.is_retryable() => Unopened::Transient(format!("this device could not open it now: {e}")),
        e => Unopened::Refused(e.to_string(), counter),
    }
}

/// Open the sealed state if it is this person's, on this node, no older than the service's
/// counter, and clean: a drain that publishes nothing finds no message of this device's
/// leaf that this state never sent (the in-band stale check).
fn open_sealed(
    rt: &tokio::runtime::Runtime,
    blob: &[u8],
    seed: &[u8; 32],
    node: &str,
    held: u64,
    ring: Box<dyn Fn() + Send + Sync>,
) -> Result<(Node, u64), Unopened> {
    let header = pacific_core::devstate::peek(blob).map_err(|e| Unopened::Refused(e.to_string(), 0))?;
    if header.counter < held {
        return Err(Unopened::Refused(format!("it is at {} and the service at {held}: it is behind", header.counter), header.counter));
    }
    let (n, header) = Node::open_restored(blob, seed, node).map_err(|e| unopened(e, header.counter))?;
    // THE HELD CONNECTION BEFORE THE CHECK (O-69): it publishes nothing, and the check's
    // drains then leave every tag complete, so the first pass drains none of them again. It
    // dials beside the check's session; the check waits for its confirmation.
    if let Err(e) = n.go_live(ring, Duration::ZERO) {
        eprintln!("door: no held connection; every tick syncs — {e}");
    }
    match rt.block_on(n.drain_only()) {
        Ok(d) if d.own_unsent == 0 => Ok((n, header.counter)),
        Ok(d) => {
            let _ = n.discard();
            Err(Unopened::Refused(format!("stale: {} message(s) of this device were sent by another copy of it", d.own_unsent), header.counter))
        }
        Err(e) => {
            let _ = n.discard();
            Err(unopened(e, header.counter))
        }
    }
}

fn sync_secs() -> u64 {
    3
}

#[derive(Clone)]
struct Door {
    tx: Sender<Ask>,
    secret: std::sync::Arc<String>,
    /// Bumped whenever this account's graph may have changed. `/v2/events` streams it.
    changed: tokio::sync::watch::Receiver<u64>,
}

/// A public site's stream, one per session: its scope, where it last stood, and the
/// channel it moves (NC-41).
struct Watcher {
    scope: String,
    seen: u64,
    version: u64,
    tx: tokio::sync::watch::Sender<u64>,
}

/// Where the session is. Every session route waits for `In`.
#[derive(PartialEq)]
enum Phase {
    Out,
    SigningUp,
    In,
}

/// A PANIC ENDS THE SESSION AT ONCE (Software Security, (a)): a panicking actor would
/// leave its process alive and seated, answering nothing (NC-90's class). Where, and a
/// literal message; a formatted one is not repeated, since it may carry a person's data.
/// Session mode only: the supervisor's handlers keep tokio's containment, and one of
/// them panicking must not take every session's leash with it.
fn exit_on_panic() {
    std::panic::set_hook(Box::new(|info| {
        let at = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_else(|| "an unknown place".into());
        let what = info.payload().downcast_ref::<&'static str>().copied().unwrap_or("a formatted message, not repeated");
        eprintln!("door session: a panic at {at} ({what}); the process ends");
        std::process::exit(101);
    }));
}

/// DEBUG BUILDS ONLY: an account of this name panics its session at /v2/kinds, so a test
/// can hold a panicking actor to (a). A release build, which every deploy is, has none.
#[cfg(debug_assertions)]
pub const PANIC_FOR_TESTS: &str = "door-test: panic at kinds";

pub fn run() {
    exit_on_panic();
    let state_dir = std::env::var("PACIFIC_STATE_DIR").expect("PACIFIC_STATE_DIR");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).expect("the supervisor's config line");
    let cfg: Cfg = serde_json::from_str(&line).expect("the supervisor's config");
    // ONE IMAGE (NC-80): a session is its supervisor's build, run from the image it started
    // from. Any other answers the config with a refusal naming both, on the leash, and ends.
    if let Some(why) = crate::skew((&crate::build_name(), &crate::image_id()), (cfg.build.as_deref(), cfg.image.as_deref())) {
        use std::io::Write;
        use std::os::fd::FromRawFd;
        eprintln!("door session: {why}");
        let mut leash = std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(0) });
        let _ = writeln!(leash, "refused: {why}");
        std::process::exit(1);
    }
    // THE FOLD CACHE (O-69): its model the core commit a stamped, clean build was built from;
    // a stamped, dirty build names none and folds uncached (FC-8), whatever the env says. An
    // unstamped build is a test's, and takes the test's model. Its entries are counted by
    // this process's allocator, within the session's budget (FC-10).
    if arc_build::STAMPED {
        pacific_core::fold_cache::set_model((!arc_build::DIRTY).then_some(arc_build::CORE));
    }
    pacific_core::fold_cache::set_measure(crate::counting::allocated);
    eprintln!(
        "door session: fold cache {}",
        match (pacific_core::fold_cache::model().is_some(), pacific_core::fold_cache::verify()) {
            (false, _) => "off",
            (true, false) => "on",
            (true, true) => "on, every hit verified",
        }
    );
    if let Some(mib) = cfg.fold_cache_mib {
        pacific_core::fold_cache::set_bytes((mib as usize) << 20);
    }
    // A PORT OF ITS OWN: bound here, from 0, and named to the supervisor on the leash,
    // so no other process can take it between a choice and a bind.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind the session");
    {
        use std::io::Write;
        use std::os::fd::FromRawFd;
        let port = listener.local_addr().expect("the session's address").port();
        // fd 0 is the leash, a socket both ways: the config came in on it. Borrowed, not
        // owned: the leash's own reader keeps it open until the supervisor goes.
        let mut leash = std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(0) });
        writeln!(leash, "{port}").expect("the port, named to the supervisor");
    }
    listener.set_nonblocking(true).expect("the session's listener, non-blocking");
    harden();
    // NOTHING READABLE AT REST: the seed and the MLS state are sealed under a key
    // this process alone holds, in memory, for its life. Until D-34 is ruled a
    // session keeps nothing after it ends, so the key goes with it.
    let mut at_rest = [0u8; 32];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut at_rest);
    pacific_core::atrest::set_device_key(at_rest);
    std::fs::create_dir_all(&state_dir).expect("create the state dir");
    std::fs::write(format!("{state_dir}/routes"), &cfg.relay).expect("write the route");
    // NC-85: the session's Arc is its relay, from configuration. What it mints is stamped
    // with the relay it is written on; left unset, the stamp was the compiled production
    // Arc, and a node on another relay drained nothing of it.
    pacific_core::node::set_default_arc(&cfg.relay).expect("the Door's relay, as this session's Arc");

    let (tx, rx) = channel::<Ask>();
    let (bump, changed) = tokio::sync::watch::channel(0u64);
    let secret = std::sync::Arc::new(cfg.secret.clone());
    let bell = tx.clone();
    std::thread::spawn(move || actor(cfg, rx, bump, bell));
    // THE PIPE IS THE LEASH. EOF means the supervisor is gone, and a session it
    // cannot see must not outlive it: it ends, head first, and goes.
    let on_eof = tx.clone();
    std::thread::spawn(move || {
        let mut rest = String::new();
        while std::io::stdin().read_line(&mut rest).map(|n| n > 0).unwrap_or(false) {}
        end_then_exit(&on_eof, "the supervisor is gone");
    });
    let on_term = tx.clone();

    let door = Door { tx, secret, changed };
    let internal = Router::new()
        .route("/i/key", get(key))
        .route("/i/prove", post(prove))
        .route("/i/open", post(open))
        .route("/i/forget", post(forget))
        .route("/i/signup", post(sign_up))
        .route("/i/signup/finish", post(sign_up_finish))
        .route("/i/sign", post(sign))
        .route("/i/end", post(end))
        .route("/i/bundle", post(bundle))
        .route("/i/code", post(code))
        .route("/v2/me", get(me))
        .route("/v2/kinds", get(kinds))
        .route("/v2/draft/:kind", get(draft_for))
        .route("/v2/graph", get(graph))
        .route("/v2/members", get(members))
        .route("/v2/members/:site", get(members_named))
        .route("/v2/mint", post(mint))
        .route("/v2/apply", post(apply))
        .route("/v2/add", post(add))
        .route("/v2/history", post(history))
        .route("/v2/batch", post(batch))
        .route("/v2/site/address", post(address))
        .route("/i/site/:site", get(site_of))
        .route("/v2/events", get(events))
        .layer(axum::middleware::from_fn_with_state(door.clone(), only_the_supervisor))
        .with_state(door);

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("runtime");
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::from_std(listener).expect("the session's listener");
        // systemd stopping the Door signals every process in the unit (NC-47).
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM");
        tokio::spawn(async move {
            term.recv().await;
            tokio::task::spawn_blocking(move || end_then_exit(&on_term, "SIGTERM"));
        });
        axum::serve(listener, internal).await.expect("serve");
    });
}

/// How long a session takes to end on SIGTERM or a broken leash before it goes anyway:
/// under door.service's TimeoutStopSec, so a relay or auth service that does not answer
/// cannot hold a stop until systemd kills the unit.
const END_SECS: u64 = 30;

/// The session ends as `/i/end` ends it, its head stored first, then the process exits
/// (NC-47). A restart, a deploy or a rollback signs everyone out, and loses nothing.
fn end_then_exit(tx: &Sender<Ask>, why: &str) -> ! {
    eprintln!("door: {why}: the session ends, head first");
    let (back, got) = channel();
    if tx.send(Ask::End(back)).is_ok() {
        let _ = got.recv_timeout(Duration::from_secs(END_SECS));
    }
    std::process::exit(0);
}

/// THE PROCESS'S OWN SHARE OF THE SANDBOX (mdr/door.md §7; DV-10), before any seed
/// exists in it: no core dump, and on Linux not dumpable, no new privileges, and its
/// memory locked out of swap where the host allows. A uid per session, seccomp and
/// the egress rule are the host's, and need it.
fn harden() {
    let none = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: plain syscalls on this process, with valid arguments.
    unsafe {
        if libc::setrlimit(libc::RLIMIT_CORE, &none) != 0 {
            eprintln!("door: core dumps could not be turned off");
        }
        #[cfg(target_os = "linux")]
        {
            if libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) != 0 {
                eprintln!("door: the session could not be made undumpable");
            }
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                eprintln!("door: no_new_privs could not be set");
            }
            if libc::mlockall(libc::MCL_CURRENT | libc::MCL_FUTURE) != 0 {
                eprintln!("door: memory not locked; the host must have no swap, or MemorySwapMax=0 on cgroup v2 (§7)");
            }
        }
    }
}

/// Every request carries the supervisor's bearer, compared in constant time.
async fn only_the_supervisor(State(d): State<Door>, req: Request, next: Next) -> Result<Response, StatusCode> {
    let given = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    let want = d.secret.as_bytes();
    let same = given.len() == want.len()
        && given.bytes().zip(want.iter()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0;
    if !same {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(req).await)
}

/// The Node's thread. It owns the Node and a current-thread runtime, so the core's
/// async calls and the auth service's have somewhere to run.
fn actor(
    cfg: Cfg,
    rx: std::sync::mpsc::Receiver<Ask>,
    bump: tokio::sync::watch::Sender<u64>,
    bell: Sender<Ask>,
) {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    let http = crate::auth::client();
    let key = SessionKey::new();
    let every = Duration::from_secs(cfg.sync_secs);
    let mut node: Option<Node> = None;
    let mut phase = Phase::Out;
    let mut resume = serde_json::Value::Null;
    let mut spine_unreached = false;
    // THE POSITION THE AUTH SERVICE HOLDS, as far as this session knows (NC-26).
    let mut stored: Option<u64> = None;
    let mut version = 0u64;
    let mut seen = 0u64;
    // What each scoped (public site's) session minted: inside ITS scope from then on,
    // and no other session's. Keyed by the supervisor's session ref: one person's
    // process may hold several sessions (D-34 (c)).
    let mut minted: std::collections::HashMap<String, std::collections::HashSet<String>> = Default::default();
    // Each public site's stream, by session.
    let mut watchers: std::collections::HashMap<String, Watcher> = Default::default();
    // The new account's words, held until its wrap is stored: shown only for an
    // account that exists (a passkey without PRF is refused before any are seen).
    let mut words_held: Option<zeroize::Zeroizing<String>> = None;
    // A proof waiting for its Node (D-34 (c)), and the person's sealed state once open.
    let mut proved: Option<Proved> = None;
    let mut keeper: Option<keep::Keeper> = None;
    let ringing = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // The auth service's calls, off the actor, once the session is in (O-69).
    let mut auth: Option<auth_worker::Worker> = None;
    // THE TICK KEEPS ITS TIME: it runs every `every` whatever else is asked, not only after
    // `every` with no request (a reader asking faster than that once starved it).
    let mut next_tick = std::time::Instant::now() + every;
    let mut tail: Option<Tail> = None;
    let mut reconciled = std::time::Instant::now();
    let mut tap: Option<crate::tw::tap::Tap> = None;

    loop {
        if std::time::Instant::now() >= next_tick {
            next_tick = std::time::Instant::now() + every;
            {
                // THE ACCOUNT LISTENS, not only when asked: what the relay delivers is ingested
                // as it arrives (`Ask::Rang`); here its leases renew, its pool refills, and the
                // head it moved is stored.
                if let (Phase::In, Some(n)) = (&phase, node.as_ref()) {
                    // THE RECONCILER (O-69): the full pass, every RECONCILE under the held
                    // connection, and every tick without one.
                    if !n.live_up() || reconciled.elapsed() >= RECONCILE {
                        reconciled = std::time::Instant::now();
                        if let Err(e) = rt.block_on(n.sync_once()) {
                            eprintln!("door: background sync failed — {e}");
                        }
                    }
                    // NC-65: a member someone else admitted (the Arc, at a kiosk) holds
                    // nothing this person owns from before it joined; their device says
                    // it again. What it publishes is sealed below, as any tick's is.
                    match rt.block_on(n.restate_for_joiners()) {
                        Ok(0) => {}
                        Ok(k) => eprintln!("door: {k} delta(s) restated for members who joined late"),
                        Err(e) => eprintln!("door: restating for members who joined late failed — {e}"),
                    }
                    // What the owner's other devices wrote arrives here, and an event
                    // that has been and gone leaves its Face here.
                    if !spine_unreached {
                        hydrate(n, &rt, false);
                    }
                    head_later(n, &mut stored, auth.as_ref(), &rt, &http, &cfg);
                    tap_flush(&mut tap, cfg.tap, n);
                    moved(n, &mut seen, &mut version, &bump, &mut watchers, &minted);
                    // WHAT THE TICK PUBLISHED IS KEPT within `keep::EVERY`; a seal the
                    // service has not confirmed is confirmed here.
                    if let Some(k) = keeper.as_mut() {
                        if k.due(n) {
                            if let Err(e) = k.seal(n, &cfg.door) {
                                eprintln!("door: {}", redacted(&e, &cfg));
                            }
                        }
                        if !confirm_later(k, auth.as_ref(), &rt, &http, &cfg, &n.id) {
                            break;
                        }
                    }
                }
            }
        }
        // A WRITE'S TAIL (O-69) waits for no ask to be waiting, or for TAIL_WITHIN.
        let waiting = match tail.take() {
            None => None,
            Some(t) => match rx.try_recv() {
                Ok(a) if !t.due() => {
                    tail = Some(t);
                    Some(a)
                }
                got => {
                    if let (Phase::In, Some(n)) = (&phase, node.as_ref()) {
                        if !after_write(t, n, &rt, &http, &cfg, &mut keeper, &mut stored, spine_unreached, auth.as_ref()) {
                            break;
                        }
                    }
                    match got {
                        Ok(a) => Some(a),
                        Err(std::sync::mpsc::TryRecvError::Empty) => None,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                    }
                }
            },
        };
        let ask = match waiting.map(Ok).unwrap_or_else(|| rx.recv_timeout(next_tick.saturating_duration_since(std::time::Instant::now()))) {
            Ok(a) => a,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        };
        match ask {
            Ask::Key(back) => {
                let _ = back.send(Ok(key.public()));
            }
            Ask::Prove { attempt, pk, host, sealed, back } => {
                // Asked again after an open that should be retried, it proves again.
                let out = (|| {
                    if phase != Phase::Out {
                        return Err("this session has already begun".to_string());
                    }
                    if host != cfg.host {
                        return Err(format!("this passkey's account is at {host}; this Door signs in at {}", cfg.host));
                    }
                    let prf = prf_of(&key, &sealed, &attempt)?;
                    let wrap = rt.block_on(crate::auth::wrap(&http, &cfg.auth, &pk))?;
                    let seed = pacific_core::wrap::open(&prf, &wrap, &host).map_err(|e| e.to_string())?;
                    let id = pacific_core::identity::Identity::in_memory(*seed);
                    let expect = format!("ed25519:{pk}");
                    if id.identity_key() != expect {
                        return Err(format!("the wrap for {expect} opens to {}; nothing was written", id.identity_key()));
                    }
                    proved = Some(Proved { prf, wrap, seed: zeroize::Zeroizing::new(*seed), id, pk: pk.clone(), host });
                    Ok(serde_json::json!({ "pk": pk }))
                })();
                let _ = back.send(out);
            }
            Ask::Open(back) => {
                // THE OPEN PATH (D-34 (c); mdr/door.md §4 step 3): the counter the service
                // holds, then the seal if it is current and clean, else a fresh device. A
                // refused seal is deleted at once and never fallen back to.
                let out = (|| {
                    if phase != Phase::Out {
                        return Err("this session has already begun".to_string());
                    }
                    let p = proved.as_ref().ok_or("nothing has been proved")?;
                    let head = rt.block_on(crate::auth::head(&http, &cfg.auth, &p.pk)).map_err(|e| format!("{TRANSIENT}{e}"))?;
                    stored = head.as_ref().map(|h| h.declared_position);
                    let mut kept: Option<keep::Keeper> = None;
                    let mut restored: Option<Node> = None;
                    if let Some(dir) = cfg.seals.as_deref() {
                        let mut k = keep::Keeper::claim(std::path::Path::new(dir), &p.id.identity_pk()).map_err(|e| format!("{TRANSIENT}{e}"))?;
                        let (nonce, sig) = auth_sig(&rt, &http, &cfg, &p.id).map_err(|e| format!("{TRANSIENT}{e}"))?;
                        let held = rt
                            .block_on(crate::auth::seal_counter(&http, &cfg.auth, &p.pk, &cfg.door, &nonce, &sig))
                            .map_err(|e| format!("{TRANSIENT}{e}"))?
                            .unwrap_or(0);
                        (k.held, k.sealed) = (held, held);
                        if let Some(blob) = k.read() {
                            match open_sealed(&rt, &blob, &p.seed, &cfg.door, held, ringer(&bell, &ringing)) {
                                Ok((n, counter)) => {
                                    k.sealed = counter;
                                    restored = Some(n);
                                }
                                Err(Unopened::Transient(why)) => return Err(format!("{TRANSIENT}{why}")),
                                Err(Unopened::Refused(why, counter)) => {
                                    eprintln!("door: the sealed state is refused ({why}); a fresh device");
                                    k.delete();
                                    if let Err(e) = pacific_core::devstate::clear(&pacific_core::paths::state_dir()) {
                                        return Err(format!("the refused state could not be cleared: {e}"));
                                    }
                                    k.sealed = held.max(counter);
                                }
                            }
                        }
                        kept = Some(k);
                    }
                    let expect = format!("ed25519:{}", p.pk);
                    let restored_at_all = restored.is_some();
                    let (n, r) = match restored {
                        Some(n) => {
                            let r = rt.block_on(n.resume(head)).map_err(|e| e.to_string())?;
                            (n, r)
                        }
                        None => rt
                            .block_on(Node::sign_in_from_wrap(&p.prf, &p.wrap, &p.host, "", &expect, head))
                            .map_err(|e| e.to_string())?,
                    };
                    spine_unreached = matches!(r.objects.first(), Some(o) if !matches!(
                        o.outcome,
                        pacific_core::resumption::Outcome::Joined { .. } | pacific_core::resumption::Outcome::AlreadyHeld
                    ));
                    resume = resumed_json(&r);
                    // THE HELD CONNECTION before the first pass, so the pass's drains leave
                    // every confirmed tag complete (O-69). A restored state has one already.
                    if !n.live_held() {
                        if let Err(e) = n.go_live(ringer(&bell, &ringing), LIVE_WITHIN) {
                            eprintln!("door: no held connection; every tick syncs — {e}");
                        }
                    }
                    settle(&n, &rt, spine_unreached);
                    store_head(&rt, &http, &cfg, &n, &mut stored);
                    // A restored file ahead of the service is confirmed now: a crash before
                    // its confirmation leaves the file ahead, never behind.
                    if let Some(k) = kept.as_mut() {
                        if !confirm(&rt, &http, &cfg, &n.id, k) {
                            let _ = n.discard();
                            return Err("another process holds this person's state".into());
                        }
                    }
                    let out = serde_json::json!({ "pk": p.pk, "resume": resume, "restored": restored_at_all });
                    seen = n.dir.generation().unwrap_or(0);
                    keeper = kept;
                    node = Some(n);
                    Ok(out)
                })();
                if out.is_ok() {
                    phase = Phase::In;
                    auth = node.as_ref().and_then(|n| start_auth(&cfg, n, &bell));
                    proved = None;
                }
                let _ = back.send(out);
            }
            Ask::Forget { who, back } => {
                minted.remove(&who);
                watchers.remove(&who);
                let _ = back.send(Ok(()));
            }
            Ask::SignUp { name, back } => {
                let out = (|| {
                    if phase != Phase::Out {
                        return Err("this session has already begun".to_string());
                    }
                    let n = Node::init_identity(&name).map_err(|e| e.to_string())?;
                    let words = n.recovery_key().ok_or("the new identity has no words")?;
                    let pk = n.id.identity_pk();
                    let mut handle = pk.to_vec();
                    handle.extend_from_slice(cfg.host.as_bytes());
                    use base64::Engine;
                    let out = serde_json::json!({
                        "pk": hex::encode(pk),
                        "host": cfg.host,
                        "handle": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(handle),
                    });
                    words_held = Some(zeroize::Zeroizing::new(words));
                    node = Some(n);
                    phase = Phase::SigningUp;
                    Ok(out)
                })();
                let _ = back.send(out);
            }
            Ask::SignUpFinish { attempt, sealed, back } => {
                // KEPT BEFORE IT ANSWERS (O-74, point 3): the claim, the wrap, the first seal
                // on disk and its counter confirmed at the auth service, then the words. The
                // seal came after the answer before: a crash or an early end between them
                // left the state nowhere on disk, and the next sign-in a fresh device (NC-67).
                // A claim refused is refused in words, before the wrap: nothing was made.
                // A seal or counter failure comes after it, and the account exists (the auth
                // service keeps a wrap for good): NC-89 (UX, SECURITY), its words go to the
                // window once, `saved: false`, and never into a refusal, a record or this
                // log. Either ends the process.
                let mut kept: Option<keep::Keeper> = None;
                let mut ends = false;
                let out = (|| {
                    let n = match (&phase, node.as_ref()) {
                        (Phase::SigningUp, Some(n)) => n,
                        _ => return Err("no sign-up is waiting for a passkey".to_string()),
                    };
                    let prf = prf_of(&key, &sealed, &attempt)?;
                    let wrap = n.export_wrap(&prf, &cfg.host).map_err(|e| e.to_string())?;
                    let pk = hex::encode(n.id.identity_pk());
                    if let Some(dir) = cfg.seals.as_deref() {
                        kept = Some(keep::Keeper::claim(std::path::Path::new(dir), &n.id.identity_pk()).map_err(|e| {
                            ends = true;
                            eprintln!("door: a new account's seal could not be claimed — {}", redacted(&e, &cfg));
                            format!("{TRANSIENT}No account was made: this Door cannot keep it (its seal could not be claimed).")
                        })?);
                    }
                    // THE FIRST SEAL BEFORE THE WRAP (NC-132): the Door's proof it can keep the
                    // account, written while none exists. A disk with no room for it refuses here,
                    // and nothing is made anywhere; a retry is right. Its counter is confirmed
                    // after the wrap, with the seal that follows.
                    if let Some(k) = kept.as_mut() {
                        k.seal(n, &cfg.door).map_err(|e| {
                            ends = true;
                            eprintln!("door: a new account's first seal could not be written — {}", redacted(&e, &cfg));
                            format!("{TRANSIENT}No account was made: this Door cannot keep it (its seal could not be written).")
                        })?;
                    }
                    let wrapped = (|| {
                        let nonce = rt.block_on(crate::auth::challenge(&http, &cfg.auth, &cfg.host))?;
                        let sig = n.sign_challenge(&cfg.host, &nonce).map_err(|e| e.to_string())?;
                        rt.block_on(crate::auth::put_wrap(&http, &cfg.auth, &pk, &nonce, &sig, wrap))
                    })();
                    // No wrap, no account: the first seal goes, and the claim with the Keeper.
                    if let Err(e) = wrapped {
                        if let Some(k) = kept.as_ref() {
                            k.delete();
                        }
                        return Err(e);
                    }
                    if let Err(e) = n.go_live(ringer(&bell, &ringing), LIVE_WITHIN) {
                        eprintln!("door: no held connection; every tick syncs — {e}");
                    }
                    settle(n, &rt, false);
                    if let Some(k) = kept.as_mut() {
                        // The failure's class goes to the window, its detail to this log,
                        // redacted, and the words to neither (Software Security, (c), (e)).
                        let keeping = k
                            .seal(n, &cfg.door)
                            .map_err(|e| ("its seal could not be written", e))
                            .and_then(|_| confirm_now(&rt, &http, &cfg, &n.id, k).map_err(|e| ("its seal's counter could not be confirmed", e)));
                        if let Err((why, e)) = keeping {
                            ends = true;
                            eprintln!("door: an account was made and this Door could not keep it: {why} ({}); its words go to the window unsaved, and the process ends", redacted(&e, &cfg));
                            let words = words_held.take().map(|w| w.to_string()).unwrap_or_default();
                            return Ok(serde_json::json!({ "pk": pk, "words": words, "saved": false, "why": why }));
                        }
                    }
                    store_head(&rt, &http, &cfg, n, &mut stored);
                    seen = n.dir.generation().unwrap_or(0);
                    let words = words_held.take().map(|w| w.to_string()).unwrap_or_default();
                    Ok(serde_json::json!({ "pk": pk, "words": words }))
                })();
                if out.is_ok() && !ends {
                    phase = Phase::In;
                    auth = node.as_ref().and_then(|n| start_auth(&cfg, n, &bell));
                    keeper = kept;
                }
                if let Err(e) = &out {
                    if ends {
                        eprintln!("door: the sign-up is refused and its process ends — {}", redacted(e.strip_prefix(TRANSIENT).unwrap_or(e), &cfg));
                    }
                }
                let _ = back.send(out);
                if ends {
                    // The refusal is written to the window before the process goes: the
                    // exit after the loop would otherwise cut its answer off.
                    std::thread::sleep(Duration::from_millis(500));
                    break;
                }
            }
            Ask::Sign { audience, nonce, back } => {
                // NOT A SIGNING ORACLE: the supervisor's challenge, for the Door's own
                // host, and no other audience. A signature for the auth service is one
                // a supervisor could spend there.
                let out = match (node.as_ref(), proved.as_ref()) {
                    _ if audience != cfg.door => Err(format!("this session signs only for {}", cfg.door)),
                    (Some(n), _) if phase == Phase::In => n.sign_challenge(&audience, &nonce).map_err(|e| e.to_string()),
                    // The proof's key, before any Node: SEC-4's second proof (D-34 (c)).
                    (None, Some(p)) if phase == Phase::Out && !nonce.trim().is_empty() => Ok(hex::encode(
                        p.id.sign(&pacific_core::identity::auth_payload(&audience, &p.id.identity_key(), &nonce)),
                    )),
                    _ => Err("not signed in".to_string()),
                };
                let _ = back.send(out);
            }
            Ask::Watch { scope, who, back } => {
                // Every stream of one site session hears it (NC-133): a page opens more than one,
                // and a second that replaced the watcher ended the first in silence. A who is one
                // token's session, so its scope is one; a different scope replaces, as before.
                let rx = match watchers.get(&who).filter(|w| w.scope == scope) {
                    Some(w) => w.tx.subscribe(),
                    None => {
                        let seen = node
                            .as_ref()
                            .map(|n| scope_generation(n, &in_scope(n, &scope, minted.get(&who).unwrap_or(&NONE))))
                            .unwrap_or(0);
                        let (tx, rx) = tokio::sync::watch::channel(0u64);
                        watchers.insert(who, Watcher { scope, seen, version: 0, tx });
                        rx
                    }
                };
                let _ = back.send(Ok(rx));
            }
            Ask::Rang => {
                ringing.store(false, std::sync::atomic::Ordering::SeqCst);
                if let (Phase::In, Some(n)) = (&phase, node.as_ref()) {
                    if let Err(e) = n.ingest_delivered() {
                        eprintln!("door: what the relay delivered was not ingested — {e}");
                    }
                    // A tag the connection cannot vouch for yet is drained here, on the bell,
                    // and never on a read.
                    if n.needs_catch_up().unwrap_or(false) {
                        if let Err(e) = rt.block_on(n.catch_up()) {
                            eprintln!("door: the held connection's catch-up failed — {e}");
                        }
                    }
                }
            }
            Ask::Authed(a) => match a {
                auth_worker::Authed::Head { position, got } => match got {
                    auth_worker::Head::Stored => stored = Some(stored.map_or(position, |s| s.max(position))),
                    auth_worker::Head::Behind(held) => stored = Some(held),
                    // NC-28: another session of this account stored this position first. If
                    // what it stored names this chain, it is stored; if not, a real fork, said
                    // once, and not offered again at this position.
                    auth_worker::Head::Fork(theirs) => {
                        let mine = match (node.as_ref(), theirs) {
                            (Some(n), Some((blob, at))) => n.head_is_mine(&blob, at).map_err(|e| e.to_string()),
                            _ => Err("theirs could not be read".into()),
                        };
                        if !matches!(mine, Ok(true)) {
                            eprintln!("door: a different head is stored at position {position} — a fork ({})", redacted(&mine.err().unwrap_or_else(|| "not this chain".into()), &cfg));
                        }
                        stored = Some(position);
                    }
                    auth_worker::Head::Failed(e) => eprintln!("door: the head was not stored — {}", redacted(&e, &cfg)),
                },
                auth_worker::Authed::Confirm { counter, got } => match got {
                    auth_worker::Confirm::Stored => {
                        if let Some(k) = keeper.as_mut() {
                            k.held = k.held.max(counter);
                        }
                    }
                    auth_worker::Confirm::Moved(held) => {
                        eprintln!("door: the seal's counter moved to {held} under this process: another writer; it ends");
                        break;
                    }
                    auth_worker::Confirm::Failed(e) => {
                        eprintln!("door: the seal is kept and not yet confirmed, retried each tick — {}", redacted(&e, &cfg))
                    }
                },
            },
            Ask::End(back) => {
                if let (Phase::In, Some(n)) = (&phase, node.as_ref()) {
                    // A write's tail still waiting goes first: its upkeep and its publish. If
                    // another writer moved this person's counter, by the tail or before it, this
                    // process holds nothing to keep: it ends with no seal and no head.
                    let kept_by_another = tail
                        .take()
                        .is_some_and(|t| !after_write(t, n, &rt, &http, &cfg, &mut keeper, &mut stored, spine_unreached, auth.as_ref()))
                        || auth.as_ref().is_some_and(|w| w.moved());
                    if kept_by_another {
                        eprintln!("door: another writer holds this person's state; the session ends with no seal");
                    } else {
                        settle(n, &rt, spine_unreached);
                        // The last seal, so the next sign-in resumes where this one ended: on
                        // disk before anything waits on the auth service, since the end is
                        // killed at its cap and a publish no seal holds makes the next open
                        // stale (NC-76). A seal ahead of the service's counter still opens.
                        match auth.as_ref() {
                            None => {
                                if let Err(e) = keep_write(&rt, &http, &cfg, n, &mut keeper) {
                                    eprintln!("door: the state was not kept at its end — {}", redacted(&e, &cfg));
                                }
                                store_head(&rt, &http, &cfg, n, &mut stored);
                            }
                            Some(w) => {
                                if let Some(Err(e)) = keeper.as_mut().map(|k| k.seal(n, &cfg.door)) {
                                    eprintln!("door: the state was not kept at its end — {}", redacted(&e, &cfg));
                                }
                                let head = n.head_update().ok().filter(|h| stored.is_none_or(|s| h.position > s)).map(|h| (h.position, h.sealed));
                                let confirm = keeper.as_ref().filter(|k| k.unconfirmed()).map(|k| (k.held, k.sealed));
                                // The answers, here: the end waits for them, within its cap.
                                for a in w.now(head, confirm, Duration::from_secs(END_SECS / 2)) {
                                    match a {
                                        auth_worker::Authed::Confirm { got: auth_worker::Confirm::Moved(held), .. } => {
                                            eprintln!("door: at its end, the seal's counter had moved to {held}: another writer")
                                        }
                                        auth_worker::Authed::Confirm { got: auth_worker::Confirm::Failed(e), .. } => {
                                            eprintln!("door: the state was not kept at its end — {}", redacted(&e, &cfg))
                                        }
                                        auth_worker::Authed::Head { got: auth_worker::Head::Failed(e), .. } => {
                                            eprintln!("door: the head was not stored — {}", redacted(&e, &cfg))
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }
                let _ = back.send(Ok(()));
                break;
            }
            other => {
                let n = match (&phase, node.as_ref()) {
                    (Phase::In, Some(n)) => n,
                    _ => {
                        refuse(other, "not signed in");
                        continue;
                    }
                };
                match other {
                    Ask::Me { scope, who, back } => {
                        barrier(n, &rt);
                        // A public site's token sees its Site's part of the account and
                        // nothing else, here as in /v2/graph (NC-55).
                        let keep = scope.as_ref().map(|site| in_scope(n, site, minted.get(&who).unwrap_or(&NONE)));
                        let seen = |id: &str| keep.as_ref().is_none_or(|k| k.contains(id));
                        let noncompliant: Vec<serde_json::Value> = n
                            .noncompliant_objects()
                            .unwrap_or_default()
                            .into_iter()
                            .filter(|c| seen(&c.object_id))
                            .map(|c| serde_json::json!({ "object": c.object_id, "kind": c.kind, "why": c.reason }))
                            .collect();
                        let mut resume = resume.clone();
                        if let Some(objects) = resume["objects"].as_array_mut() {
                            objects.retain(|o| o["object"].as_str().is_some_and(&seen));
                        }
                        // The spine is the self record's id: a site's reader has neither.
                        if keep.is_some() {
                            resume["spine"] = serde_json::Value::Null;
                        }
                        let objects = n
                            .objects_named()
                            .map(|v| v.iter().filter(|(id, _, _)| seen(id)).count())
                            .unwrap_or(0);
                        let _ = back.send(Ok(serde_json::json!({
                            "pk": n.identity_key(),
                            "display_name": my_name(n),
                            // A picture the card could not hold, and why (O-77), for the webapp to
                            // ask for a smaller one; absent when nothing was left out.
                            "card_note": n.my_card().ok().and_then(|(_, note)| note),
                            "objects": objects,
                            "resume": resume,
                            "noncompliant": noncompliant,
                        })));
                    }
                    Ask::Graph { scope, who, back } => {
                        barrier(n, &rt);
                        let out = graph_of(n).map(|mut g| {
                            if let Some(site) = &scope {
                                let keep = in_scope(n, site, minted.get(&who).unwrap_or(&NONE));
                                if let Some(objects) = g["objects"].as_array_mut() {
                                    objects.retain(|o| o["id"].as_str().is_some_and(|id| keep.contains(id)));
                                    for o in objects.iter_mut() {
                                        if let Some(view) = o.get_mut("view") {
                                            cut_to_scope(view, &keep);
                                        }
                                    }
                                }
                                // Not even filtered: the survivors' indices show the account's shape.
                                g["spine"] = serde_json::json!([]);
                            }
                            g
                        });
                        let _ = back.send(out);
                    }
                    Ask::Kinds(back) => {
                        #[cfg(debug_assertions)]
                        if n.display_name().ok().as_deref() == Some(PANIC_FOR_TESTS) {
                            panic!("a test's panic, debug builds only");
                        }
                        // THE KINDS ARE THE CORE'S: `classify` is a total match in
                        // `mint.rs`, so a kind added there appears here unedited.
                        let kinds = pacific_core::object::ObjectKind::ALL
                            .iter()
                            .map(|k| KindOut {
                                kind: k.name().to_string(),
                                mintable: pacific_core::mint::is_mintable(*k),
                                no_mint: match pacific_core::mint::classify(*k) {
                                    Ok(()) => None,
                                    Err(pacific_core::mint::NoMint::Paired) => Some("paired"),
                                    Err(pacific_core::mint::NoMint::Derived) => Some("derived"),
                                    Err(pacific_core::mint::NoMint::Internal) => Some("internal"),
                                    Err(pacific_core::mint::NoMint::Unbuilt) => Some("unbuilt"),
                                },
                            })
                            .collect();
                        let _ = back.send(Ok(kinds));
                    }
                    Ask::Draft { kind, back } => {
                        let _ = back.send(match pacific_core::mint::kind_from_name(&kind) {
                            None => Err(format!("unknown object kind '{kind}'")),
                            Some(k) => Ok(draft_shape(k)),
                        });
                    }
                    Ask::Mint { kind, draft, scope, who, back } => {
                        let out = match pacific_core::mint::kind_from_name(&kind) {
                            None => Err(format!("unknown object kind '{kind}'")),
                            Some(k) => rt.block_on(n.mint(k, &draft.to_draft())).map_err(|e| e.to_string()),
                        };
                        if let (Some(_), Ok(id)) = (&scope, &out) {
                            minted.entry(who).or_default().insert(id.clone());
                        }
                        // ANSWERED ONCE IT IS HERE AND KEPT (O-69): the rest is its tail.
                        if let Err(e) = seal_write(n, &rt, &cfg, &mut keeper, spine_unreached) {
                            let _ = back.send(Err(format!("the object was minted, but this Door could not keep it ({e}); sign in again")));
                            break;
                        }
                        let made = out.is_ok();
                        let _ = back.send(out);
                        tail.get_or_insert_with(Tail::default).add(made);
                    }
                    Ask::Apply { object, op, args, scope, who, back } => {
                        // THE SCOPE FIRST: a public site's token writes to its Site
                        // and what it minted, and is refused everywhere else.
                        if let Some(site) = &scope {
                            let keep = in_scope(n, site, minted.get(&who).unwrap_or(&NONE));
                            if !keep.contains(&object) {
                                let _ = back.send(Err(format!("{object} is outside this site's scope")));
                                continue;
                            }
                            if let Some(why) = names_outside(n, &keep, &op, &args) {
                                let _ = back.send(Err(why));
                                continue;
                            }
                        }
                        // THE ICD NAME, resolved on the object's OWN kind: ids are per
                        // kind, so a name resolved anywhere else gives an id that names a
                        // different op here (NC-54).
                        let out = match n.object_kind(&object) {
                            Err(e) => Err(e.to_string()),
                            Ok(kind) => match pacific_core::authoring::op_on(&kind, &op) {
                                None => Err(format!("a {kind} has no op '{op}'")),
                                Some(d) => rt.block_on(n.apply_local(&object, d.op_id, args)).map_err(|e| e.to_string()),
                            },
                        };
                        // ANSWERED ONCE IT IS HERE AND KEPT (O-69): the local commit is what
                        // a following read sees; the publish and the rest are its tail.
                        if let Err(e) = seal_write(n, &rt, &cfg, &mut keeper, spine_unreached) {
                            let _ = back.send(Err(format!("the write was made, but this Door could not keep it ({e}); sign in again")));
                            break;
                        }
                        let wrote = out.is_ok();
                        let _ = back.send(out);
                        if wrote {
                            tail.get_or_insert_with(Tail::default).add(false);
                        }
                    }
                    Ask::Add { object, by, scope, back } => {
                        // The webapp's own session only: a site's token adds no one.
                        if scope.is_some() {
                            let _ = back.send(Err("a site's token adds no one".to_string()));
                            continue;
                        }
                        let out = match by {
                            AddBy::Bundle(bundle) => rt.block_on(n.group_add_member(&object, &bundle)).map(hex::encode).map_err(|e| match e {
                                pacific_core::CoreError::KeyPackageExpired(_) => EXPIRED.to_string(),
                                e => e.to_string(),
                            }),
                            // MAKE ADMIN'S SECOND STEP (ICD 2.1.0 row 10): a member of the part's
                            // Site, by identity; refused in core's own words, unprefixed.
                            AddBy::Member(member) => match rt.block_on(n.add_part_member(&object, &member)) {
                                Ok(m) => Ok(hex::encode(m)),
                                Err(pacific_core::CoreError::Membership(why)) => Err(why),
                                Err(e) => Err(e.to_string()),
                            },
                        };
                        if let Err(e) = seal_write(n, &rt, &cfg, &mut keeper, spine_unreached) {
                            let _ = back.send(Err(format!("the member was added, but this Door could not keep it ({e}); sign in again")));
                            break;
                        }
                        let _ = back.send(out);
                        tail.get_or_insert_with(Tail::default).add(false);
                    }
                    Ask::History { object, bundle, dry, scope, back } => {
                        // The webapp's own session only, as an Add.
                        if scope.is_some() {
                            let _ = back.send(Err("a site's token sends no history".to_string()));
                            continue;
                        }
                        barrier(n, &rt);
                        let out = match rt.block_on(n.send_history_as_owner(&object, &bundle, dry)) {
                            Ok(r) => Ok(serde_json::json!({
                                "object": r.object, "dry": dry,
                                "spine": r.spine, "records": r.records, "posts": r.posts, "left": r.left, "unsent": r.unsent,
                                "cards": r.cards, "card_bundles": r.card_bundles, "cards_unsent": r.cards_unsent,
                                "sealed": r.sealed, "max": pacific_core::node::HISTORY_MAX_B64,
                            })),
                            Err(pacific_core::CoreError::Membership(why)) => Err(why),
                            Err(e) => Err(e.to_string()),
                        };
                        let _ = back.send(out);
                    }
                    Ask::Batch { steps, scope, who, back } => {
                        // ONE ACTOR TURN, each step through the one write path with its own
                        // checks, so no tick lands between the halves of an edge. The first
                        // refusal ends it; what was made stands, and is named.
                        let mut made: Vec<String> = Vec::new();
                        let mut refused = None;
                        for (i, step) in steps.into_iter().enumerate() {
                            let r = match step {
                                Step::Mint { kind, draft } => {
                                    let r = match pacific_core::mint::kind_from_name(&kind) {
                                        None => Err(format!("unknown object kind '{kind}'")),
                                        Some(k) => rt.block_on(n.mint(k, &draft.to_draft())).map_err(|e| e.to_string()),
                                    };
                                    if let (Some(_), Ok(id)) = (&scope, &r) {
                                        minted.entry(who.clone()).or_default().insert(id.clone());
                                    }
                                    r
                                }
                                Step::Apply { object, op, args } => {
                                    let object = object.resolve(&made);
                                    let args = args.into_iter().map(|(k, v)| (k, v.resolve(&made))).collect::<pacific_media::Args>();
                                    let keep = scope.as_ref().map(|site| in_scope(n, site, minted.get(&who).unwrap_or(&NONE)));
                                    if keep.as_ref().is_some_and(|k| !k.contains(&object)) {
                                        Err(format!("{object} is outside this site's scope"))
                                    } else if let Some(why) = keep.as_ref().and_then(|k| names_outside(n, k, &op, &args)) {
                                        Err(why)
                                    } else {
                                        match n.object_kind(&object) {
                                            Err(e) => Err(e.to_string()),
                                            Ok(kind) => match pacific_core::authoring::op_on(&kind, &op) {
                                                None => Err(format!("a {kind} has no op '{op}'")),
                                                Some(d) => rt.block_on(n.apply_local(&object, d.op_id, args)).map(|_| object.clone()).map_err(|e| e.to_string()),
                                            },
                                        }
                                    }
                                }
                            };
                            match r {
                                Ok(id) => made.push(id),
                                Err(why) => {
                                    refused = Some(serde_json::json!({ "step": i, "why": why }));
                                    break;
                                }
                            }
                        }
                        if !made.is_empty() {
                            if let Err(e) = seal_write(n, &rt, &cfg, &mut keeper, spine_unreached) {
                                let _ = back.send(Err(format!("{} step(s) were made, but this Door could not keep them ({e}); sign in again", made.len())));
                                break;
                            }
                        }
                        if !made.is_empty() {
                            tail.get_or_insert_with(Tail::default).add(true);
                        }
                        let _ = back.send(Ok(serde_json::json!({ "made": made, "refused": refused })));
                    }
                    Ask::Address { slug, host, scope, who, back } => {
                        let out = (|| {
                            // A site's token names an address only for a Host in its scope.
                            if let Some(site) = &scope {
                                if !in_scope(n, site, minted.get(&who).unwrap_or(&NONE)).contains(&host) {
                                    return Err(format!("{host} is outside this site's scope"));
                                }
                            }
                            if n.object_kind(&host).map_err(|e| e.to_string())? != "host" {
                                return Err(format!("{host} is not a Host"));
                            }
                            let pk = hex::encode(n.id.identity_pk());
                            let signed = || -> Result<(String, String), String> {
                                let nonce = rt.block_on(crate::auth::challenge(&http, &cfg.auth, &cfg.host))?;
                                let sig = n.sign_challenge(&cfg.host, &nonce).map_err(|e| e.to_string())?;
                                Ok((nonce, sig))
                            };
                            let (nonce, sig) = signed()?;
                            rt.block_on(crate::auth::claim_site(&http, &cfg.auth, &pk, &nonce, &sig, &slug))?;
                            let (nonce, sig) = signed()?;
                            rt.block_on(crate::auth::bind_host(&http, &cfg.auth, &pk, &nonce, &sig, &slug, &host))?;
                            Ok(serde_json::json!({ "slug": slug.to_lowercase(), "host": host }))
                        })();
                        let _ = back.send(out);
                    }
                    Ask::SiteOf { site, back } => {
                        barrier(n, &rt);
                        let me = n.id.identity_pk();
                        let member = n.object_kind(&site).is_ok_and(|k| k == "group") && n.object_members(&site).is_ok_and(|m| m.contains(&me));
                        // Its address where this Node holds its Host: the Host's publication.
                        let view = |id: &str| n.object_view(id).ok().and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok());
                        let slug = member.then(|| view(&site)).flatten().and_then(|v| {
                            v["parts"].as_array()?.iter().filter(|p| p["role"] == "host").filter_map(|p| p["part"].as_str()).find_map(|h| view(h)?["publication"]["slug"].as_str().map(str::to_string))
                        });
                        let _ = back.send(Ok(serde_json::json!({ "member": member, "slug": slug })));
                    }
                    Ask::Bundle(back) => {
                        let _ = back.send(n.build_contact_bundle().map_err(|e| e.to_string()));
                    }
                    Ask::Code(back) => {
                        let life = Duration::from_secs(cfg.code_secs);
                        let expires = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) + cfg.code_secs;
                        let made: Result<Vec<String>, String> = (0..crate::JOIN_BUNDLES).map(|_| n.build_contact_bundle_living(life).map_err(|e| e.to_string())).collect();
                        // Kept before it is handed out: a package whose private half the next
                        // sign-in has not got would admit its person to a Site they never reach.
                        let out = made.and_then(|b| match seal_write(n, &rt, &cfg, &mut keeper, spine_unreached) {
                            Ok(()) => Ok((b, expires)),
                            Err(e) => Err(format!("this Door could not keep the code ({e}); sign in again")),
                        });
                        let _ = back.send(out);
                    }
                    _ => unreachable!("handled above"),
                }
            }
        }
        if let (Phase::In, Some(n)) = (&phase, node.as_ref()) {
            if moved(n, &mut seen, &mut version, &bump, &mut watchers, &minted) {
                tap_flush(&mut tap, cfg.tap, n);
            }
        }
    }
    std::process::exit(0);
}

/// TRAINING_WHEELS' DELTA TAP (P0): what the Node holds, past what was reported, on the
/// leash. Started at its first flush, so the first reports the whole held log: the backfill.
fn tap_flush(tap: &mut Option<crate::tw::tap::Tap>, on: bool, n: &Node) {
    if !on {
        return;
    }
    if tap.is_none() {
        match crate::tw::tap::Tap::on_leash() {
            Ok(t) => *tap = Some(t),
            Err(e) => return eprintln!("door: the Delta tap could not start — {e}"),
        }
    }
    if let Some(Err(e)) = tap.as_mut().map(|t| t.flush(n)) {
        eprintln!("door: the Delta tap failed — {e}");
    }
}

/// Answer a session ask with a refusal before the session is in.
fn refuse(ask: Ask, why: &str) {
    let why = why.to_string();
    match ask {
        Ask::Me { back: b, .. } | Ask::Graph { back: b, .. } => drop(b.send(Err(why))),
        Ask::Kinds(b) => drop(b.send(Err(why))),
        Ask::Address { back, .. } => drop(back.send(Err(why))),
        Ask::SiteOf { back, .. } => drop(back.send(Err(why))),
        Ask::Bundle(b) => drop(b.send(Err(why))),
        Ask::Code(b) => drop(b.send(Err(why))),
        Ask::Draft { back, .. } => drop(back.send(Err(why))),
        Ask::Mint { back, .. } | Ask::Apply { back, .. } | Ask::Add { back, .. } => drop(back.send(Err(why))),
        Ask::History { back, .. } => drop(back.send(Err(why))),
        Ask::Batch { back, .. } => drop(back.send(Err(why))),
        _ => {}
    }
}

/// A READ SERVES WHAT WAS DELIVERED (O-69; mdr/fold-cache.md § The obligation): whatever the
/// held connection has delivered is ingested first, with no round trip. Without a held
/// connection a read converges as before, by a sync; with one that is down it serves what was
/// delivered while the connection comes back.
fn barrier(n: &Node, rt: &tokio::runtime::Runtime) {
    if n.live_held() {
        if let Err(e) = n.ingest_delivered() {
            eprintln!("door: what the relay delivered was not ingested — {e}");
        }
    } else if let Err(e) = rt.block_on(n.sync_once()) {
        eprintln!("door: sync failed — {e}");
    }
}

/// The PRF output out of the window's seal: 32 bytes, zeroed when dropped.
fn prf_of(key: &SessionKey, sealed: &Sealed, attempt: &str) -> Result<zeroize::Zeroizing<[u8; 32]>, String> {
    let open = key.open(sealed, attempt)?;
    let prf: [u8; 32] = open.as_slice().try_into().map_err(|_| "a PRF output is 32 bytes".to_string())?;
    Ok(zeroize::Zeroizing::new(prf))
}

/// STORE THE HEAD when this Node has moved it (NC-26; resumption.md A3, A6.1):
/// compare-and-set on position at the auth service. A failure is logged and the
/// next tick tries again; `Behind` means the service holds a later position.
fn store_head(
    rt: &tokio::runtime::Runtime,
    http: &reqwest::Client,
    cfg: &Cfg,
    node: &Node,
    stored: &mut Option<u64>,
) {
    let head = match node.head_update() {
        Ok(h) => h,
        Err(e) => return eprintln!("door: no head — {e}"),
    };
    if stored.is_some_and(|s| head.position <= s) {
        return;
    }
    let pk = hex::encode(node.id.identity_pk());
    let result = crate::auth::again(|| {
        let nonce = rt.block_on(crate::auth::challenge(http, &cfg.auth, &cfg.host))?;
        let sig = node.sign_challenge(&cfg.host, &nonce).map_err(|e| e.to_string())?;
        rt.block_on(crate::auth::put_head(http, &cfg.auth, &pk, &nonce, &sig, head.position, head.sealed.clone()))
    });
    match result {
        Ok(crate::auth::Stored::Stored) => *stored = Some(head.position),
        Ok(crate::auth::Stored::Behind { held }) => *stored = Some(held),
        // NC-28: another session of this account stored this position first. If
        // what it stored names this chain, it is stored; if not, it is a real fork,
        // said once, and not offered again at this position.
        Ok(crate::auth::Stored::Fork) => {
            let theirs = rt.block_on(crate::auth::head(http, &cfg.auth, &pk));
            match theirs.map(|h| h.map(|h| node.head_is_mine(&h.blob, h.declared_position))) {
                Ok(Some(Ok(true))) => {}
                other => eprintln!(
                    "door: a different head is stored at position {} — a fork ({})",
                    head.position,
                    redacted(
                        &match other {
                            Ok(Some(Err(e))) => e.to_string(),
                            Err(e) => e,
                            _ => "not this chain".into(),
                        },
                        cfg
                    )
                ),
            }
            *stored = Some(head.position);
        }
        Err(e) => eprintln!("door: the head was not stored — {}", redacted(&e, cfg)),
    }
}

/// Ask the Node's thread, and wait. Every refusal is the core's own words.
fn ask<T: Send + 'static>(door: &Door, make: impl FnOnce(Reply<T>) -> Ask) -> Result<T, (StatusCode, String)> {
    let (back, wait) = channel::<Result<T, String>>();
    door.tx
        .send(make(back))
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "the session's thread is gone".to_string()))?;
    match wait.recv() {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(why)) => Err((StatusCode::BAD_REQUEST, why)),
        Err(_) => Err((StatusCode::INTERNAL_SERVER_ERROR, "no answer".to_string())),
    }
}

/// `ask`, off the async runtime: the Node's thread blocks, the server does not.
async fn ask_async<T: Send + 'static>(
    door: Door,
    make: impl FnOnce(Reply<T>) -> Ask + Send + 'static,
) -> Result<T, (StatusCode, String)> {
    tokio::task::spawn_blocking(move || ask(&door, make))
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "the session's thread panicked".to_string()))?
}

async fn key(State(d): State<Door>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ask_async(d, Ask::Key).await.map(|k| Json(serde_json::json!({ "key": k })))
}

#[derive(Deserialize)]
struct SignInIn {
    attempt: String,
    pk: String,
    host: String,
    sealed: Sealed,
}

async fn prove(State(d): State<Door>, Json(i): Json<SignInIn>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ask_async(d, move |back| Ask::Prove { attempt: i.attempt, pk: i.pk, host: i.host, sealed: i.sealed, back })
        .await
        .map(Json)
}

/// A transient failure is a 503: the supervisor keeps the attempt for a retry.
async fn open(State(d): State<Door>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ask_async(d, Ask::Open).await.map(Json).map_err(|(s, w)| match w.strip_prefix(TRANSIENT) {
        Some(w) => (StatusCode::SERVICE_UNAVAILABLE, w.to_string()),
        None => (s, w),
    })
}

#[derive(Deserialize)]
struct ForgetIn {
    #[serde(rename = "ref")]
    who: String,
}

async fn forget(State(d): State<Door>, Json(i): Json<ForgetIn>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ask_async(d, move |back| Ask::Forget { who: i.who, back }).await.map(|_| Json(serde_json::json!({ "forgot": true })))
}


#[derive(Deserialize)]
struct SignUpIn {
    #[serde(default)]
    name: String,
}

async fn sign_up(State(d): State<Door>, Json(i): Json<SignUpIn>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ask_async(d, move |back| Ask::SignUp { name: i.name, back }).await.map(Json)
}

#[derive(Deserialize)]
struct FinishIn {
    attempt: String,
    sealed: Sealed,
}

/// A Door that cannot keep the new account is a 503: nothing was made, and a sign-up again is
/// right (NC-132).
async fn sign_up_finish(
    State(d): State<Door>,
    Json(i): Json<FinishIn>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ask_async(d, move |back| Ask::SignUpFinish { attempt: i.attempt, sealed: i.sealed, back }).await.map(Json).map_err(|(s, w)| match w.strip_prefix(TRANSIENT) {
        Some(w) => (StatusCode::SERVICE_UNAVAILABLE, w.to_string()),
        None => (s, w),
    })
}

#[derive(Deserialize)]
struct SignAsk {
    audience: String,
    nonce: String,
}

async fn sign(State(d): State<Door>, Json(i): Json<SignAsk>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ask_async(d, move |back| Ask::Sign { audience: i.audience, nonce: i.nonce, back })
        .await
        .map(|s| Json(serde_json::json!({ "sig": s })))
}

async fn bundle(State(d): State<Door>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ask_async(d, Ask::Bundle).await.map(|b| Json(serde_json::json!({ "bundle": b })))
}

async fn code(State(d): State<Door>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ask_async(d, Ask::Code).await.map(|(b, expires)| Json(serde_json::json!({ "bundles": b, "expires": expires })))
}

async fn end(State(d): State<Door>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ask_async(d, Ask::End).await.map(|_| Json(serde_json::json!({ "ended": true })))
}

async fn me(State(d): State<Door>, h: axum::http::HeaderMap) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (scope, who) = (scope_of(&h), who_of(&h));
    ask_async(d, move |back| Ask::Me { scope, who, back }).await.map(Json)
}

async fn kinds(State(d): State<Door>) -> Result<Json<Vec<KindOut>>, (StatusCode, String)> {
    ask_async(d, Ask::Kinds).await.map(Json)
}

/// The Site a public site's session is held to, as the supervisor passes it: the
/// header is there only for such a session, and empty while no Site is registered.
fn scope_of(h: &axum::http::HeaderMap) -> Option<String> {
    h.get("x-door-scope").and_then(|v| v.to_str().ok()).map(str::to_string)
}

/// Which of this person's sessions a request is for, as the supervisor names it; the
/// one session where it names none.
fn who_of(h: &axum::http::HeaderMap) -> String {
    h.get("x-door-ref").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string()
}

/// What a session that minted nothing minted.
static NONE: std::sync::LazyLock<std::collections::HashSet<String>> = std::sync::LazyLock::new(Default::default);

async fn graph(State(d): State<Door>, h: axum::http::HeaderMap) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (scope, who) = (scope_of(&h), who_of(&h));
    ask_async(d, move |back| Ask::Graph { scope, who, back }).await.map(Json)
}

/// GET /v2/members (W-98 Members): a registered site's token reads its own Site's members;
/// the token's scope is the Site. The webapp names the Site instead (`members_named`).
async fn members(State(d): State<Door>, h: axum::http::HeaderMap) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let site = scope_of(&h).ok_or((StatusCode::BAD_REQUEST, "name the Site: GET /v2/members/<site id>".to_string()))?;
    members_for(d, h, site).await
}

/// GET /v2/members/<site id>: the Site named, as the webapp reads it. A site's token names
/// only its own Site: any other is out of its reach, as it is in /v2/graph.
async fn members_named(
    State(d): State<Door>,
    h: axum::http::HeaderMap,
    Path(site): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if scope_of(&h).is_some_and(|s| s != site) {
        return Err((StatusCode::NOT_FOUND, "no such Site in reach".to_string()));
    }
    members_for(d, h, site).await
}

/// The members, read from the same graph /v2/graph answers this session, cut to its scope
/// (members.rs): nothing a member could not read there.
async fn members_for(d: Door, h: axum::http::HeaderMap, site: String) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (scope, who) = (scope_of(&h), who_of(&h));
    let g = ask_async(d, move |back| Ask::Graph { scope, who, back }).await?;
    let me = g["me"]["pk"].as_str().unwrap_or_default().trim_start_matches("ed25519:").to_ascii_lowercase();
    let objects = g["objects"].as_array().map(Vec::as_slice).unwrap_or_default();
    crate::members::members_of(objects, &site, &me).map(Json).map_err(|why| (StatusCode::NOT_FOUND, why.to_string()))
}

async fn draft_for(State(d): State<Door>, Path(kind): Path<String>) -> Result<Json<DraftOut>, (StatusCode, String)> {
    ask_async(d, move |back| Ask::Draft { kind, back }).await.map(Json)
}

/// `Node::mint` creates the MLS group, authors the kind's op 0 through the one
/// write path, writes the vertebra on the spine and publishes.
async fn mint(
    State(d): State<Door>,
    h: axum::http::HeaderMap,
    Json(input): Json<MintIn>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (scope, who) = (scope_of(&h), who_of(&h));
    let id = ask_async(d, move |back| Ask::Mint { kind: input.kind, draft: input.draft, scope, who, back }).await?;
    Ok(Json(serde_json::json!({ "object_id": id })))
}

#[derive(Deserialize)]
struct ApplyIn {
    object: String,
    op: String,
    #[serde(default)]
    args: serde_json::Map<String, serde_json::Value>,
}

/// One op through `Node::apply`, the one write path. Args are the ICD's: a string
/// is text, an integer is an int, anything else is refused.
async fn apply(
    State(d): State<Door>,
    h: axum::http::HeaderMap,
    Json(input): Json<ApplyIn>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (scope, who) = (scope_of(&h), who_of(&h));
    let mut args = pacific_media::Args::new();
    for (k, v) in input.args {
        let val = match v {
            serde_json::Value::String(s) => pacific_media::ArgVal::Text(s),
            serde_json::Value::Number(n) if n.is_i64() => pacific_media::ArgVal::Int(n.as_i64().unwrap()),
            other => return Err((StatusCode::BAD_REQUEST, format!("arg '{k}': {other} is neither text nor int"))),
        };
        args.insert(k, val);
    }
    let id = ask_async(d, move |back| Ask::Apply { object: input.object, op: input.op, args, scope, who, back }).await?;
    Ok(Json(serde_json::json!({ "delta": id })))
}

/// ONE STEP OF A BATCH (O-69): a mint or an apply, as its own route takes it, where an
/// object or an arg may name an earlier mint's object, `{"$step": i}`.
enum Step {
    Mint { kind: String, draft: DraftIn },
    Apply { object: Named, op: String, args: Vec<(String, NamedArg)> },
}

enum Named {
    Id(String),
    Step(usize),
}

enum NamedArg {
    Val(pacific_media::ArgVal),
    Step(usize),
}

impl Named {
    /// Every step before this one was made, or the batch would have stopped.
    fn resolve(self, made: &[String]) -> String {
        match self {
            Named::Id(id) => id,
            Named::Step(i) => made[i].clone(),
        }
    }
}

impl NamedArg {
    fn resolve(self, made: &[String]) -> pacific_media::ArgVal {
        match self {
            NamedArg::Val(v) => v,
            NamedArg::Step(i) => pacific_media::ArgVal::Text(made[i].clone()),
        }
    }
}

/// The most a batch carries: a Register's five steps, and the sections added with it.
const BATCH_MAX: usize = 16;

#[derive(Deserialize)]
struct BatchIn {
    steps: Vec<serde_json::Value>,
}

/// The steps, or why the batch is refused before any of it runs: more than BATCH_MAX, a
/// step of no known shape, an arg neither text nor int, a `$step` naming no earlier mint.
fn steps_of(raw: Vec<serde_json::Value>) -> Result<Vec<Step>, String> {
    use serde_json::Value;
    if raw.is_empty() || raw.len() > BATCH_MAX {
        return Err(format!("a batch is 1 to {BATCH_MAX} steps, not {}", raw.len()));
    }
    let mut mints: Vec<usize> = Vec::new();
    let mut out = Vec::new();
    for (i, s) in raw.into_iter().enumerate() {
        let named = |v: &Value, what: &str| -> Result<Option<usize>, String> {
            let Some(n) = v.as_object().and_then(|o| o.get("$step")) else { return Ok(None) };
            match n.as_u64().map(|n| n as usize) {
                Some(n) if mints.contains(&n) => Ok(Some(n)),
                _ => Err(format!("step {i}: {what} names step {n}, which is not an earlier mint")),
            }
        };
        match s.get("do").and_then(Value::as_str) {
            Some("mint") => {
                let m: MintIn = serde_json::from_value(s).map_err(|e| format!("step {i}: {e}"))?;
                out.push(Step::Mint { kind: m.kind, draft: m.draft });
                mints.push(i);
            }
            Some("apply") => {
                let object = match &s["object"] {
                    Value::String(id) => Named::Id(id.clone()),
                    v => Named::Step(named(v, "its object")?.ok_or(format!("step {i}: its object is an id or {{\"$step\": n}}"))?),
                };
                let op = s["op"].as_str().ok_or(format!("step {i}: no op"))?.to_string();
                let mut args = Vec::new();
                for (k, v) in s.get("args").and_then(Value::as_object).cloned().unwrap_or_default() {
                    let a = match &v {
                        Value::String(t) => NamedArg::Val(pacific_media::ArgVal::Text(t.clone())),
                        Value::Number(n) if n.is_i64() => NamedArg::Val(pacific_media::ArgVal::Int(n.as_i64().unwrap())),
                        other => match named(other, &format!("arg '{k}'"))? {
                            Some(n) => NamedArg::Step(n),
                            None => return Err(format!("step {i}: arg '{k}': {other} is neither text nor int")),
                        },
                    };
                    args.push((k, a));
                }
                out.push(Step::Apply { object, op, args });
            }
            _ => return Err(format!("step {i}: \"do\" is \"mint\" or \"apply\"")),
        }
    }
    Ok(out)
}

/// SEVERAL WRITES, ONE REQUEST (O-69): mints and applies in order, in one turn of this
/// session, each with its own route's checks (a site's token's scope at every step).
/// Answers what was made, one object id a step; a refusal part-way answers 422 with what
/// was made before it and the step that was refused, in the Door's words.
async fn batch(
    State(d): State<Door>,
    h: axum::http::HeaderMap,
    Json(input): Json<BatchIn>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let (scope, who) = (scope_of(&h), who_of(&h));
    let steps = steps_of(input.steps).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let out = ask_async(d, move |back| Ask::Batch { steps, scope, who, back }).await?;
    let status = if out["refused"].is_null() { StatusCode::OK } else { StatusCode::UNPROCESSABLE_ENTITY };
    Ok((status, Json(out)))
}

#[derive(Deserialize)]
struct AddIn {
    object: String,
    bundle: Option<String>,
    member: Option<String>,
}

/// Whom an Add adds: someone handing over their bundle, or a member of the part's Site
/// named by identity alone (ICD 2.1.0 row 10).
enum AddBy {
    Bundle(String),
    Member([u8; 32]),
}

/// The owner adds a member: by their contact bundle (`Node::group_add_member`: owner-gated,
/// the credential checked against the identity), or, a member of the part's Site, by
/// their identity (`Node::add_part_member`). One of the two. Answers the member added.
async fn add(
    State(d): State<Door>,
    h: axum::http::HeaderMap,
    Json(input): Json<AddIn>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let scope = scope_of(&h);
    let by = match (input.bundle, input.member) {
        (Some(b), None) => AddBy::Bundle(b),
        (None, Some(m)) => AddBy::Member(
            hex::decode(&m)
                .ok()
                .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
                .ok_or((StatusCode::BAD_REQUEST, format!("{m} is not a member id")))?,
        ),
        _ => return Err((StatusCode::BAD_REQUEST, "an add names a bundle or a member, one of the two".to_string())),
    };
    let member = ask_async(d, move |back| Ask::Add { object: input.object, by, scope, back })
        .await
        .map_err(|(s, why)| if why == EXPIRED { (StatusCode::GONE, why) } else { (s, why) })?;
    Ok(Json(serde_json::json!({ "member": member })))
}

#[derive(Deserialize)]
struct HistoryIn {
    object: String,
    bundle: String,
    #[serde(default)]
    dry: bool,
}

/// The owner seals a Site's or a room's history to a member (`Node::send_history_as_owner`):
/// the Arc's node, added to a room late, so it passes the room's whole log to each joiner it
/// admits. The owner's alone, to a member on the object's roster alone; `dry` sends nothing,
/// to anyone's bundle. Answers what went (or would): rows by tier, rows left, why nothing
/// went, cards, and the spine's bundle sealed (`sealed`) against one relay blob (`max`).
async fn history(
    State(d): State<Door>,
    h: axum::http::HeaderMap,
    Json(input): Json<HistoryIn>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let scope = scope_of(&h);
    ask_async(d, move |back| Ask::History { object: input.object, bundle: input.bundle, dry: input.dry, scope, back }).await.map(Json)
}

#[derive(Deserialize)]
struct AddressIn {
    slug: String,
    host: String,
}

/// Whether this person is a member of a Site, and its address where their Node holds its Host.
async fn site_of(State(d): State<Door>, Path(site): Path<String>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ask_async(d, move |back| Ask::SiteOf { site, back }).await.map(Json)
}

/// A Site's address (D-52): claimed for this person and bound to the Host at the auth
/// service. The page cannot sign these: the key is here.
async fn address(
    State(d): State<Door>,
    h: axum::http::HeaderMap,
    Json(input): Json<AddressIn>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (scope, who) = (scope_of(&h), who_of(&h));
    ask_async(d, move |back| Ask::Address { slug: input.slug, host: input.host, scope, who, back }).await.map(Json)
}

/// The change stream: one event per bump, carrying the version. Nothing about the
/// graph rides it; the client refetches `/v2/graph`.
async fn events(
    State(d): State<Door>,
    h: axum::http::HeaderMap,
) -> axum::response::sse::Sse<impl futures_util::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>> {
    // A public site's stream moves only with its scope (NC-41).
    let who = who_of(&h);
    let rx = match scope_of(&h) {
        Some(scope) => ask_async(d.clone(), move |back| Ask::Watch { scope, who, back }).await.unwrap_or_else(|_| d.changed.clone()),
        None => d.changed.clone(),
    };
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.changed().await.ok()?;
        let v = *rx.borrow();
        Some((Ok(axum::response::sse::Event::default().event("changed").data(v.to_string())), rx))
    });
    axum::response::sse::Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::{redacted, reference_args, unopened, Cfg, Unopened};
    use pacific_core::CoreError;

    /// A kind minted NAME-ONLY (a Forum: its name rides its MLS group context) lists the name it
    /// takes, so a form built from /v2/draft draws the field (TEST, 1 Oct: "fields": []).
    #[test]
    fn a_name_only_kind_lists_its_name() {
        let d = super::draft_shape(pacific_core::object::ObjectKind::Forum);
        assert!(d.mintable && d.name_only);
        let fields: Vec<_> = d.fields.iter().map(|f| (f.arg.as_str(), f.draft.as_deref(), f.kind)).collect();
        assert_eq!(fields, vec![("name", Some("name"), "text")]);
        // A kind with a profile op lists what the op takes, the name among it, as before.
        let g = super::draft_shape(pacific_core::object::ObjectKind::Group);
        assert!(!g.name_only && g.fields.iter().any(|f| f.draft.as_deref() == Some("name")));
    }

    /// NC-95's references, read from the ICD: an arg with a relation, and an edge's
    /// `arg:` endpoint; a message's text is neither.
    #[test]
    fn the_icd_says_which_args_name_an_object() {
        assert!(reference_args("base.setPart").contains("part"));
        assert!(reference_args("base.setParent").contains("parent"), "an edge's endpoint");
        assert!(reference_args("base.setBacklink").contains("object"), "an edge's endpoint");
        assert!(reference_args("forum.post").is_empty(), "a message's text names nothing");
        assert!(!reference_args("base.setPart").contains("role"));
    }

    /// FC-13, THE FLOOD: however many frames ring, one `Rang` waits on the actor's queue at
    /// most; the next rings once the actor has taken it.
    #[test]
    fn a_flood_leaves_one_rang_waiting() {
        let (tx, rx) = std::sync::mpsc::channel();
        let ringing = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ring = super::ringer(&tx, &ringing);
        for _ in 0..10_000 {
            ring();
        }
        assert_eq!(rx.try_iter().count(), 1, "one Rang for ten thousand frames");
        ringing.store(false, std::sync::atomic::Ordering::SeqCst);
        ring();
        ring();
        assert_eq!(rx.try_iter().count(), 1, "and one more once it was taken");
    }

    /// Software Security (e): a failure's detail in this process's log carries no auth
    /// service URL (it holds the person's key) and no seals directory path.
    #[test]
    fn a_failures_detail_names_no_key_and_no_seals_path() {
        let cfg: Cfg = serde_json::from_value(serde_json::json!({
            "secret": "s", "auth": "http://127.0.0.1:1", "host": "h", "door": "d", "relay": "ws://r", "seals": "/var/lib/door/seals"
        }))
        .unwrap();
        let pk = "ab".repeat(32);
        let e = format!("error sending request for url (http://127.0.0.1:1/auth/users/{pk}/seals/d): refused; the seals directory /var/lib/door/seals: denied");
        let out = redacted(&e, &cfg);
        assert!(!out.contains(&pk) && !out.contains("http"), "{out}");
        assert!(!out.contains("/var/lib/door/seals"), "{out}");
        assert!(out.contains("refused") && out.contains("denied"), "the rest is kept: {out}");
    }

    /// NC-75: a failure of this device (its storage, its build) or of the relay keeps the
    /// seal, which may be the only copy of an owner's chain; only the blob's own refusal
    /// deletes it, and names its counter.
    #[test]
    fn only_the_blobs_own_refusal_deletes_the_seal() {
        for e in [
            CoreError::Transport("the relay is down".into()),
            CoreError::Directory("database is locked".into()),
            CoreError::Io(std::io::Error::other("no space left on device")),
            CoreError::UpgradeRequired("the group's floor is above this build".into()),
        ] {
            let said = e.to_string();
            assert!(matches!(unopened(e, 7), Unopened::Transient(_)), "{said}: the seal is kept");
        }
        for e in [
            CoreError::Seal("the state holds another identity".into()),
            CoreError::AtRest("the tag does not verify".into()),
            CoreError::Mls("the epoch is unknown".into()),
        ] {
            let said = e.to_string();
            assert!(matches!(unopened(e, 7), Unopened::Refused(_, 7)), "{said}: refused");
        }
    }
}
