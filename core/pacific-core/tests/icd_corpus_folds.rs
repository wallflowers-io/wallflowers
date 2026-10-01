//! ICD 2.1.0, GATE 2: THE CORPUS, BOUNDED (core/docs/launch/pdr/icd-2.1.0.md § The re-pin).
//! Every Delta the production path writes, as 1691bd4 (door-01's build) writes it, folds under
//! the build running this test to the state 1691bd4 folded it to.
//!
//! THE FIXTURE, tests/fixtures/icd-corpus-1691bd4.json, is made once at 1691bd4 by
//! `generate_the_corpus` (ignored; `ICD_CORPUS_WRITE=1`) and committed. The production path, in
//! order: Register (the Site, its Host, both halves of the edge, a face carrying production's
//! "data URL, N chars" text), a room, Ralph's one-sign-in snippet (the Arc added and admitter on
//! the Site and the room, the claim key, the face with no picture described, the Arc on the
//! Host, the mark, the face hydrated, the publication), journey 1's and journey 2's admissions
//! by kiosk claim (the Arc's Add and base.claimSpent), posts, and each person's self record. Per
//! object it keeps what the fold reads (kind, owner, roster, owner history, and each row's
//! author, envelope and signature) and the view 1691bd4 folded.
//!
//! BOUNDED: synthetic throughout. Test identities, core's test-vector kiosk key (seed [9; 32]),
//! made-up names and text, a 48×48 mark. No production byte: door-01's census (28 Sep, 14:37Z)
//! found P0's tap empty, and the corpus since the mint is in Ralph's sealed Node alone.
//!
//! THE STATE IS THE SAME when every field of 1691bd4's view is in this build's view with the
//! same value, and a field only this build has is empty (null, false, 0, "", [], {}): an
//! additive revision may add a field, and may not move one. The same holds for the whole folded
//! state, as core's own Debug prints it, because a view leaves things out (the claim issuers,
//! the spent claims), and the fold replays each Delta through its reducer and sets a
//! rejection aside: an arm lost to a revision shows in the state, and nowhere else.

mod common;

use base64::engine::general_purpose::{STANDARD as B64STD, URL_SAFE_NO_PAD as B64};
use base64::Engine;
use common::Harness;
use ed25519_dalek::Signer;
use pacific_core::coordinator::{ArgVal, Args};
use pacific_core::object::ObjectKind;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/icd-corpus-1691bd4.json");
const ICD: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json");

/// Every op the production path writes: the fixture holds each at least once.
const OPS: &[&str] = &[
    "group.setProfile",
    "group.setFace",
    "group.setClaimIssuer",
    "group.joinedObject",
    "base.setPart",
    "base.setParent",
    "base.setRole",
    "base.publish",
    "base.claimSpent",
    "base.memberJoined",
    "host.define",
    "host.setMedia",
    "host.hydrate",
    "forum.post",
    "forum.receipt",
];

/// Journey 2's share, of the share alphabet (claim.rs).
const SHARE: &str = "k2m3n4p5q6r7s8t9uvwx";

fn kiosk() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[9u8; 32])
}

fn kiosk_keys() -> HashMap<String, [u8; 32]> {
    let pk = kiosk().verifying_key().to_bytes();
    HashMap::from([(pacific_core::claim::kid_of(&pk), pk)])
}

/// A kiosk claim for `site`, nonce `n`, its choice and, on the paid route, its share.
fn claim(site: &str, n: u8, choice: &str, share: Option<&str>) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let pk = kiosk().verifying_key().to_bytes();
    let a = share.map(|s| format!(r#","a":"{s}""#)).unwrap_or_default();
    let payload = format!(
        r#"{{"k":"{}","s":"{site}","n":"{}","iat":{now},"exp":{},"c":"{choice}"{a}}}"#,
        pacific_core::claim::kid_of(&pk),
        B64.encode([n; 16]),
        now + 600
    );
    let p = B64.encode(payload);
    format!("v1.{p}.{}", B64.encode(kiosk().sign(format!("v1.{p}").as_bytes()).to_bytes()))
}

fn args(pairs: &[(&str, ArgVal)]) -> Args {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

fn text(s: &str) -> ArgVal {
    ArgVal::Text(s.to_string())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// A 48×48 PNG, base64: the snippet's mark.
fn mark() -> String {
    fn chunk(t: &[u8], d: &[u8]) -> Vec<u8> {
        let mut c = (d.len() as u32).to_be_bytes().to_vec();
        c.extend_from_slice(t);
        c.extend_from_slice(d);
        let mut h = crc32(t);
        h = crc32_more(h, d);
        c.extend_from_slice(&(!h).to_be_bytes());
        c
    }
    fn crc32(b: &[u8]) -> u32 {
        crc32_more(0xFFFF_FFFF, b)
    }
    fn crc32_more(mut c: u32, b: &[u8]) -> u32 {
        for &x in b {
            c ^= x as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        c
    }
    // Stored (uncompressed) deflate: 48 rows of a filter byte and 48 RGB pixels.
    let raw: Vec<u8> = (0..48).flat_map(|_| std::iter::once(0u8).chain((0..48).flat_map(|_| [0x41u8, 0x8C, 0xB2]))).collect();
    let mut z = vec![0x78, 0x01];
    for (i, block) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(last as u8);
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(chunk(b"IHDR", &[0, 0, 0, 48, 0, 0, 0, 48, 8, 2, 0, 0, 0]));
    png.extend(chunk(b"IDAT", &z));
    png.extend(chunk(b"IEND", &[]));
    B64STD.encode(png)
}

// ─── the fixture ─────────────────────────────────────────────────────────────

/// One object as the fold reads it, and the view 1691bd4 folded.
fn dump(n: &pacific_core::Node, id: &str) -> Value {
    let gid = hex::decode(id).unwrap();
    let kind = n.dir.group_kind(&gid).unwrap().expect("a kind");
    let owner: [u8; 32] = n.object_owner(id).unwrap().and_then(|v| v.try_into().ok()).unwrap_or([0u8; 32]);
    let members = n.dir.group_members(&gid).unwrap();
    let owners = n.dir.owner_history(&gid).unwrap();
    let rows = n.dir.load_log_signed(&gid).unwrap();
    let log: Vec<([u8; 32], Vec<u8>)> = rows.iter().map(|(a, e, _)| (*a, e.clone())).collect();
    let view = pacific_core::fold::view_of(&kind, owner, members.clone(), owners.clone(), log.clone(), Some(&n.id.identity_pk())).unwrap();
    let state = state_of(&kind, owner, members.clone(), owners.clone(), log).unwrap();
    assert_eq!(view, n.object_view(id).unwrap(), "{kind} {id}: the fixture's fold is the Node's");
    json!({
        "id": id,
        "kind": kind,
        "owner": hex::encode(owner),
        "members": members.iter().map(hex::encode).collect::<Vec<_>>(),
        "owners": owners.iter().map(|(e, pk)| json!([e, hex::encode(pk)])).collect::<Vec<_>>(),
        "rows": rows.iter().map(|(a, e, s)| json!({ "author": hex::encode(a), "envelope": hex::encode(e), "sig": s.map(hex::encode) })).collect::<Vec<_>>(),
        "view": serde_json::from_str::<Value>(&view).unwrap(),
        "state": state,
    })
}

/// THE PRODUCTION PATH at 1691bd4, written through `Node::mint` and `Node::apply` as the Door
/// writes it, then dumped. Run once, at 1691bd4:
///   ICD_CORPUS_WRITE=1 cargo test -p pacific-core --test icd_corpus_folds -- --ignored generate_the_corpus
#[test]
#[ignore]
fn generate_the_corpus() {
    // One long script of awaits is one large future: on a thread of its own, with the stack it needs.
    // A fixture writer run by hand, not a check: a test of core's futures runs at the default stack.
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(|| tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(generate()))
        .unwrap()
        .join()
        .unwrap_or_else(|e| std::panic::resume_unwind(e));
}

async fn generate() {
    let h = Harness::new(&["ada", "arc", "vis", "gil"]).await;
    let (ada, arc, vis, gil) = (0, 1, 2, 3);
    let arc_hex = hex::encode(h.id(arc));
    let at = now_ms();

    // Register, as the webapp's `made` batches it: the Site, its Host, the edge from both ends,
    // the face as www's form sent production's (a picture described in words).
    let draft = |name: &str, shape: &str| pacific_core::mint::MintDraft { name: name.into(), shape: shape.into(), ..Default::default() };
    let site = h.node(ada).mint(ObjectKind::Group, &draft("Egregore (corpus)", "community")).await.unwrap();
    let host = h.node(ada).mint(ObjectKind::Host, &draft("Egregore (corpus)", "")).await.unwrap();
    let o = h.node(ada);
    o.apply(&site, pacific_core::parts::OP_SET_PART, pacific_core::parts::set_part_args(&host, "host", at)).await.unwrap();
    o.apply(&host, pacific_core::parent::OP_SET_PARENT, pacific_core::parent::set_parent_args(&site, "host", at)).await.unwrap();
    let face = json!({ "mark": "data URL, 46303 chars", "header": { "logo": "data URL, 1200 chars", "banner": "" }, "look": { "colours": { "ink": "#1A1A1A" } } });
    o.apply(&site, pacific_core::group::OP_SET_FACE, args(&[("face", text(&face.to_string()))])).await.unwrap();

    // A room, both halves.
    let room = h.node(ada).mint(ObjectKind::Forum, &draft("Talk (corpus)", "")).await.unwrap();
    let o = h.node(ada);
    o.apply(&site, pacific_core::parts::OP_SET_PART, pacific_core::parts::set_part_args(&room, "room", at)).await.unwrap();
    o.apply(&room, pacific_core::parent::OP_SET_PARENT, pacific_core::parent::set_parent_args(&site, "room", at)).await.unwrap();
    h.settle().await;

    // The one-sign-in snippet, in its order.
    let add = |obj: String| {
        let h = &h;
        async move {
            let bundle = h.node(arc).build_contact_bundle().unwrap();
            h.node(ada).group_add_member(&obj, &bundle).await.unwrap();
            h.node(arc).sync_once().await.unwrap();
        }
    };
    let admitter = || args(&[("member", text(&arc_hex)), ("role", text("admitter"))]);
    add(site.clone()).await;
    h.node(ada).apply(&site, pacific_core::roles::OP_SET_ROLE, admitter()).await.unwrap();
    let pk = kiosk().verifying_key().to_bytes();
    let issuer = args(&[("kid", text(&pacific_core::claim::kid_of(&pk))), ("key", text(&B64.encode(pk)))]);
    h.node(ada).apply(&site, pacific_core::group::OP_SET_CLAIM_ISSUER, issuer).await.unwrap();
    add(room.clone()).await;
    h.node(ada).apply(&room, pacific_core::roles::OP_SET_ROLE, admitter()).await.unwrap();
    let bare = json!({ "mark": "", "header": { "logo": "", "banner": "" }, "look": { "colours": { "ink": "#1A1A1A" } } });
    h.node(ada).apply(&site, pacific_core::group::OP_SET_FACE, args(&[("face", text(&bare.to_string()))])).await.unwrap();
    add(host.clone()).await;
    let o = h.node(ada);
    o.apply(&host, pacific_core::host::OP_SET_MEDIA, args(&[("slot", text("mark")), ("media", text(&mark())), ("mediaMime", text("image/png"))])).await.unwrap();
    let hydrated = json!({ "v": 1, "profile": { "displayName": "Egregore (corpus)", "card": { "note": "", "urls": [] } }, "face": bare });
    o.apply(
        &host,
        pacific_core::host::OP_HYDRATE,
        args(&[("key", text("face")), ("payload", text(&hydrated.to_string())), ("fetchedAt", ArgVal::Int(at)), ("rev", ArgVal::Int(at))]),
    )
    .await
    .unwrap();
    o.apply(&host, pacific_core::publication::OP_PUBLISH, args(&[("slug", text("egregore-corpus")), ("publisher", text(&arc_hex))])).await.unwrap();
    h.settle().await;

    // Journeys 1 and 2: the Arc admits each visitor by a kiosk claim, to the Site and the room.
    for (who, n, choice, share) in [(vis, 1u8, "skills", None), (gil, 2u8, "financial", Some(SHARE))] {
        let bundles: Vec<String> = (0..2).map(|_| h.node(who).build_contact_bundle().unwrap()).collect();
        let a = h.node(arc).admit_by_claim(&site, &claim(&site, n, choice, share), &bundles, &kiosk_keys()).await.unwrap();
        assert!(a.unjoined.is_empty(), "{:?}", a.unjoined);
        h.settle().await;
    }

    // Posts, the visitor's and the owner's.
    for (who, t) in [(vis, "corpus: first, from the kiosk"), (ada, "corpus: welcome")] {
        h.node(who).apply(&room, pacific_core::coordinator::FORUM_POST, args(&[("text", text(t))])).await.unwrap();
    }
    h.settle().await;

    // Each person's self record, as the Door names what they hold after a write.
    let mut selves = vec![];
    for who in [ada, vis, gil] {
        let n = h.node(who);
        let me = n.ensure_self_object().unwrap();
        n.reconcile_joined().await.unwrap();
        selves.push((who, me));
    }
    h.settle().await;

    let mut objects = vec![];
    for id in [&site, &host, &room] {
        objects.push(dump(&h.node(ada), id));
    }
    for (who, me) in &selves {
        objects.push(dump(&h.node(*who), me));
    }
    let fx = json!({
        "made_at": "1691bd4",
        "note": "ICD 2.1.0 gate 2: the production path's Deltas at 1691bd4, synthetic (tests/icd_corpus_folds.rs)",
        "objects": objects,
    });
    let seen = names(&fx, &icd());
    for op in OPS {
        assert!(seen.contains(*op), "the corpus writes no {op}: {seen:?}");
    }
    if std::env::var("ICD_CORPUS_WRITE").as_deref() == Ok("1") {
        std::fs::create_dir_all(std::path::Path::new(FIXTURE).parent().unwrap()).unwrap();
        std::fs::write(FIXTURE, serde_json::to_string_pretty(&fx).unwrap() + "\n").unwrap();
        println!("wrote {FIXTURE}: {} objects, ops {seen:?}", fx["objects"].as_array().unwrap().len());
    }
}

// ─── the check ───────────────────────────────────────────────────────────────

fn icd() -> Value {
    serde_json::from_str(&std::fs::read_to_string(ICD).unwrap()).unwrap()
}

/// The ICD's name for a Delta's op on a kind: the kind's own ops, then the facets on it.
fn op_name(icd: &Value, kind: &str, op_id: u32) -> Option<String> {
    let mut sources = vec![&icd["kinds"][kind]];
    for f in icd["facets"].as_object().into_iter().flat_map(|m| m.values()) {
        if f["on"].as_array().is_some_and(|on| on.iter().any(|k| k == kind)) {
            sources.push(f);
        }
    }
    sources
        .into_iter()
        .filter_map(|s| s["ops"].as_object())
        .flatten()
        .find(|(_, d)| d["op"].as_u64() == Some(op_id as u64))
        .map(|(n, _)| n.clone())
}

/// Every (kind.op) the fixture's Deltas are, by this build's ICD.
fn names(fx: &Value, icd: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for o in fx["objects"].as_array().unwrap() {
        let kind = o["kind"].as_str().unwrap();
        for r in o["rows"].as_array().unwrap() {
            let d = pacific_core::coordinator::decode_delta(&hex::decode(r["envelope"].as_str().unwrap()).unwrap()).unwrap();
            out.insert(op_name(icd, kind, d.op_id).unwrap_or_else(|| format!("{kind}#{:#x}", d.op_id)));
        }
    }
    out
}

fn empty(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Bool(b) => !b,
        Value::Number(n) => n.as_f64() == Some(0.0),
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
    }
}

/// Where `now` departs from `then`: a field of `then` missing or changed, or a field only `now`
/// has that is not empty.
fn departures(then: &Value, now: &Value, at: &str, out: &mut Vec<String>) {
    match (then, now) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in a {
                match b.get(k) {
                    Some(w) => departures(v, w, &format!("{at}.{k}"), out),
                    None => out.push(format!("{at}.{k}: gone (was {v})")),
                }
            }
            for (k, w) in b.iter().filter(|(k, _)| !a.contains_key(*k)) {
                if !empty(w) {
                    out.push(format!("{at}.{k}: new and not empty ({w})"));
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (v, w)) in a.iter().zip(b).enumerate() {
                departures(v, w, &format!("{at}[{i}]"), out);
            }
        }
        _ if then == now => {}
        _ => out.push(format!("{at}: {then} → {now}")),
    }
}

/// VIEW FIELDS A REVISION ADDED, ACCEPTED BY NAME (Software Assurance's ruling, W-98): a field
/// only this build's view has, not empty, is a departure unless it is listed here, and a listed
/// one is accepted only when it equals what this build's folded state says (`folded_field`).
/// The fixture stays as 1691bd4 made it.
const ACCEPTED_VIEWS: &[(&str, &str, &str)] = &[("forum", "roles", "W-98 ROOMS, 2bc8a057: the forum view shows its roles")];

/// An accepted view field's value, from this build's folded state of the object.
fn folded_field(kind: &str, field: &str, owner: [u8; 32], members: Vec<[u8; 32]>, owners: Vec<(u64, [u8; 32])>, log: Vec<([u8; 32], Vec<u8>)>) -> Option<Value> {
    use pacific_core::fold::fold_entries_owned;
    let owners = if owners.is_empty() { vec![(0, owner)] } else { owners };
    match (kind, field) {
        ("forum", "roles") => {
            let st = fold_entries_owned::<pacific_core::coordinator::ForumType>(members, owners, log).ok()?.state();
            Some(Value::Array(st.roles.iter().map(|(m, r)| serde_json::json!([hex::encode(m), r.as_str()])).collect()))
        }
        _ => None,
    }
}

/// Takes out of `now` each accepted field `then` does not have, where it equals `want(field)`;
/// names it where it does not.
fn accept(kind: &str, then: &Value, now: &mut Value, want: impl Fn(&str) -> Option<Value>, at: &str, out: &mut Vec<String>) {
    for (k, field, why) in ACCEPTED_VIEWS {
        if *k != kind || then.get(*field).is_some() {
            continue;
        }
        let Some(got) = now.as_object_mut().and_then(|o| o.remove(*field)) else { continue };
        match want(field) {
            Some(w) if w == got => {}
            w => out.push(format!("{at}.{field}: accepted as {why:?} only as the folded state's {}, but the view holds {got}", w.map_or("(none)".into(), |w| w.to_string()))),
        }
    }
}

/// The whole folded state, as core's own Debug prints it, for the kinds the corpus holds.
fn state_of(kind: &str, owner: [u8; 32], members: Vec<[u8; 32]>, owners: Vec<(u64, [u8; 32])>, log: Vec<([u8; 32], Vec<u8>)>) -> Result<Option<String>, pacific_core::CoreError> {
    use pacific_core::fold::fold_entries_owned;
    let owners = if owners.is_empty() { vec![(0, owner)] } else { owners };
    Ok(Some(match kind {
        "group" => format!("{:?}", fold_entries_owned::<pacific_core::group::GroupType>(members, owners, log)?.state()),
        "host" => format!("{:?}", fold_entries_owned::<pacific_core::host::HostType>(members, owners, log)?.state()),
        "forum" => format!("{:?}", fold_entries_owned::<pacific_core::coordinator::ForumType>(members, owners, log)?.state()),
        _ => return Ok(None),
    }))
}

/// A derived Debug, read back as a tree: `Name { k: v }`, `Name(a, b)` and `(a, b)`, `{k: v}` and
/// `{a, b}`, `[a, b]`, and an atom (a literal, `None`, a unit variant).
#[derive(Clone, Debug, PartialEq)]
enum D {
    Struct(String, Vec<(String, D)>),
    Tuple(String, Vec<D>),
    Map(Vec<(D, D)>),
    Seq(Vec<D>),
    Atom(String),
}

/// Parses what `{:?}` prints. None where it does not: the check then compares the two texts.
fn parse(s: &str) -> Option<D> {
    struct P<'a> {
        b: &'a [u8],
        i: usize,
    }
    impl P<'_> {
        fn ws(&mut self) {
            while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
        }
        fn eat(&mut self, c: u8) -> bool {
            self.ws();
            let hit = self.b.get(self.i) == Some(&c);
            if hit {
                self.i += 1;
            }
            hit
        }
        fn quoted(&mut self, q: u8) -> Option<D> {
            let start = self.i;
            self.i += 1;
            while self.i < self.b.len() {
                match self.b[self.i] {
                    b'\\' => self.i += 2,
                    c if c == q => {
                        self.i += 1;
                        return Some(D::Atom(String::from_utf8_lossy(&self.b[start..self.i]).into()));
                    }
                    _ => self.i += 1,
                }
            }
            None
        }
        /// Items up to `close`, each read by `item`.
        fn list<T>(&mut self, close: u8, mut item: impl FnMut(&mut Self) -> Option<T>) -> Option<Vec<T>> {
            let mut out = vec![];
            if self.eat(close) {
                return Some(out);
            }
            loop {
                out.push(item(self)?);
                if self.eat(close) {
                    return Some(out);
                }
                if !self.eat(b',') {
                    return None;
                }
                if self.eat(close) {
                    return Some(out);
                }
            }
        }
        fn value(&mut self) -> Option<D> {
            self.ws();
            match *self.b.get(self.i)? {
                b'"' => self.quoted(b'"'),
                b'\'' => self.quoted(b'\''),
                b'[' => {
                    self.i += 1;
                    Some(D::Seq(self.list(b']', |p| p.value())?))
                }
                b'(' => {
                    self.i += 1;
                    Some(D::Tuple(String::new(), self.list(b')', |p| p.value())?))
                }
                b'{' => {
                    self.i += 1;
                    let items = self.list(b'}', |p| {
                        let k = p.value()?;
                        Some(if p.eat(b':') { (k, Some(p.value()?)) } else { (k, None) })
                    })?;
                    if items.iter().all(|(_, v)| v.is_some()) && !items.is_empty() {
                        Some(D::Map(items.into_iter().map(|(k, v)| (k, v.unwrap())).collect()))
                    } else if items.iter().all(|(_, v)| v.is_none()) {
                        Some(D::Seq(items.into_iter().map(|(k, _)| k).collect()))
                    } else {
                        None
                    }
                }
                _ => {
                    let start = self.i;
                    while self.i < self.b.len() && !b" ,:()[]{}".contains(&self.b[self.i]) {
                        self.i += 1;
                    }
                    // A path's `::` belongs to the name.
                    while self.b[self.i..].starts_with(b"::") {
                        self.i += 2;
                        while self.i < self.b.len() && !b" ,:()[]{}".contains(&self.b[self.i]) {
                            self.i += 1;
                        }
                    }
                    let name = String::from_utf8_lossy(&self.b[start..self.i]).to_string();
                    if name.is_empty() {
                        return None;
                    }
                    let at = self.i;
                    if self.eat(b'{') {
                        let fields = self.list(b'}', |p| {
                            p.ws();
                            let s = p.i;
                            while p.i < p.b.len() && (p.b[p.i].is_ascii_alphanumeric() || p.b[p.i] == b'_') {
                                p.i += 1;
                            }
                            let k = String::from_utf8_lossy(&p.b[s..p.i]).to_string();
                            if k.is_empty() || !p.eat(b':') {
                                return None;
                            }
                            Some((k, p.value()?))
                        })?;
                        Some(D::Struct(name, fields))
                    } else if self.eat(b'(') {
                        Some(D::Tuple(name, self.list(b')', |p| p.value())?))
                    } else {
                        self.i = at;
                        Some(D::Atom(name))
                    }
                }
            }
        }
    }
    let mut p = P { b: s.as_bytes(), i: 0 };
    let v = p.value()?;
    p.ws();
    (p.i == p.b.len()).then_some(v)
}

/// Nothing in it: what an additive field holds before anything writes it.
fn blank(d: &D) -> bool {
    match d {
        D::Atom(a) => ["None", "0", "false", "\"\"", "0.0"].contains(&a.as_str()),
        D::Seq(v) => v.is_empty(),
        D::Map(v) => v.is_empty(),
        D::Struct(_, f) => f.iter().all(|(_, v)| blank(v)),
        D::Tuple(..) => false,
    }
}

fn shown(d: &D) -> String {
    clip(&format!("{d:?}"))
}

/// Where `now` departs from `then`, at any depth: a field of `then` missing or changed, or a
/// field only `now` has that is not blank. Maps by key, sequences by position.
fn tree_departures(then: &D, now: &D, at: &str, out: &mut Vec<String>) {
    match (then, now) {
        (D::Struct(n1, f1), D::Struct(n2, f2)) if n1 == n2 => {
            for (k, v) in f1 {
                match f2.iter().find(|(k2, _)| k2 == k) {
                    Some((_, w)) => tree_departures(v, w, &format!("{at}.{k}"), out),
                    None => out.push(format!("{at}.{k}: gone")),
                }
            }
            for (k, w) in f2.iter().filter(|(k, _)| !f1.iter().any(|(k1, _)| k1 == k)) {
                if !blank(w) {
                    out.push(format!("{at}.{k}: new and not empty ({})", shown(w)));
                }
            }
        }
        (D::Tuple(n1, a), D::Tuple(n2, b)) if n1 == n2 && a.len() == b.len() => {
            for (i, (v, w)) in a.iter().zip(b).enumerate() {
                tree_departures(v, w, &format!("{at}.{i}"), out);
            }
        }
        (D::Seq(a), D::Seq(b)) if a.len() == b.len() => {
            for (i, (v, w)) in a.iter().zip(b).enumerate() {
                tree_departures(v, w, &format!("{at}[{i}]"), out);
            }
        }
        (D::Map(a), D::Map(b)) if a.len() == b.len() => {
            for (k, v) in a {
                match b.iter().find(|(k2, _)| k2 == k) {
                    Some((_, w)) => tree_departures(v, w, &format!("{at}[{}]", shown(k)), out),
                    None => out.push(format!("{at}[{}]: gone", shown(k))),
                }
            }
        }
        _ if then == now => {}
        _ => out.push(format!("{at}: {} → {}", shown(then), shown(now))),
    }
}

/// As `departures`, for the whole state, as core's Debug prints it, at every depth.
fn state_departures(then: &str, now: &str, at: &str, out: &mut Vec<String>) {
    match (parse(then), parse(now)) {
        (Some(a), Some(b)) => tree_departures(&a, &b, &format!("{at} state"), out),
        _ if then == now => {}
        _ => out.push(format!("{at}: the state moved, and its Debug does not parse: {} → {}", clip(then), clip(now))),
    }
}

fn clip(s: &str) -> String {
    if s.len() > 160 { format!("{}…", &s[..160]) } else { s.to_string() }
}

/// Every way the fixture fails under this build: a signature, an undeclared op, a fold, a view, a state.
fn check(fx: &Value, icd: &Value) -> Vec<String> {
    let mut out = vec![];
    for o in fx["objects"].as_array().unwrap() {
        let (id, kind) = (o["id"].as_str().unwrap(), o["kind"].as_str().unwrap());
        let key = |v: &Value| -> [u8; 32] { hex::decode(v.as_str().unwrap()).unwrap().try_into().unwrap() };
        let gid = hex::decode(id).unwrap();
        let mut log = vec![];
        for (i, r) in o["rows"].as_array().unwrap().iter().enumerate() {
            let author = key(&r["author"]);
            let envelope = hex::decode(r["envelope"].as_str().unwrap()).unwrap();
            let d = match pacific_core::coordinator::decode_delta(&envelope) {
                Ok(d) => d,
                Err(e) => {
                    out.push(format!("{kind} {}: row {i} does not decode: {e}", &id[..12]));
                    continue;
                }
            };
            if let Some(sig) = r["sig"].as_str() {
                let sig: [u8; 64] = hex::decode(sig).unwrap().try_into().unwrap();
                if pacific_core::delta_sig::verify_delta(&gid, &author, &d.id(), &sig).is_err() {
                    out.push(format!("{kind} {}: row {i}'s signature does not prove its author", &id[..12]));
                }
            }
            if op_name(icd, kind, d.op_id).is_none() {
                out.push(format!("{kind} {}: row {i}'s op {:#x} is not in this ICD", &id[..12], d.op_id));
            }
            log.push((author, envelope));
        }
        let members: Vec<[u8; 32]> = o["members"].as_array().unwrap().iter().map(key).collect();
        let owners: Vec<(u64, [u8; 32])> = o["owners"].as_array().unwrap().iter().map(|p| (p[0].as_u64().unwrap(), key(&p[1]))).collect();
        match state_of(kind, key(&o["owner"]), members.clone(), owners.clone(), log.clone()) {
            Err(e) => out.push(format!("{kind} {}: its state does not fold: {e}", &id[..12])),
            Ok(Some(now)) => state_departures(o["state"].as_str().unwrap_or(""), &now, &format!("{kind} {}", &id[..12]), &mut out),
            Ok(None) => {}
        }
        let owner = key(&o["owner"]);
        let want = |field: &str| folded_field(kind, field, owner, members.clone(), owners.clone(), log.clone());
        match pacific_core::fold::view_of(kind, owner, members.clone(), owners.clone(), log.clone(), None) {
            Err(e) => out.push(format!("{kind} {}: does not fold: {e}", &id[..12])),
            Ok(view) => {
                let at = format!("{kind} {}", &id[..12]);
                let mut now: Value = serde_json::from_str(&view).unwrap();
                accept(kind, &o["view"], &mut now, want, &at, &mut out);
                departures(&o["view"], &now, &at, &mut out);
            }
        }
    }
    out
}

fn fixture() -> Value {
    serde_json::from_str(&std::fs::read_to_string(FIXTURE).expect("the corpus fixture, made at 1691bd4")).unwrap()
}

/// GATE 2: every Delta of the corpus folds under this build, to the state 1691bd4 folded.
#[test]
fn the_corpus_folds_under_this_build_as_it_did_at_1691bd4() {
    let (fx, icd) = (fixture(), icd());
    let seen = names(&fx, &icd);
    for op in OPS {
        assert!(seen.contains(*op), "the fixture holds no {op}: {seen:?}");
    }
    let failed = check(&fx, &icd);
    assert!(failed.is_empty(), "{} departure(s):\n{}", failed.len(), failed.join("\n"));
}

/// The check has teeth: one byte of one Delta changed is named.
#[test]
fn a_delta_changed_by_one_byte_is_named() {
    let (mut fx, icd) = (fixture(), icd());
    let room = fx["objects"].as_array_mut().unwrap().iter_mut().find(|o| o["kind"] == "forum").unwrap();
    let has_text = |r: &Value| hex::decode(r["envelope"].as_str().unwrap()).unwrap().windows(6).any(|w| w == b"corpus");
    let row = room["rows"].as_array_mut().unwrap().iter_mut().find(|r| has_text(r)).expect("a post");
    let mut e = hex::decode(row["envelope"].as_str().unwrap()).unwrap();
    let at = e.windows(6).position(|w| w == b"corpus").unwrap();
    e[at] ^= 0x20;
    row["envelope"] = Value::String(hex::encode(e));
    let failed = check(&fx, &icd);
    assert!(failed.iter().any(|f| f.contains("forum") && f.contains("signature")), "{failed:?}");
}

/// And a state that moved is named: a room renamed in the recorded view.
#[test]
fn a_view_that_moved_is_named() {
    let (mut fx, icd) = (fixture(), icd());
    let site = fx["objects"].as_array_mut().unwrap().iter_mut().find(|o| o["kind"] == "group" && o["view"]["display_name"] == "Egregore (corpus)").unwrap();
    site["view"]["display_name"] = Value::String("Somewhere else".into());
    let failed = check(&fx, &icd);
    assert!(failed.iter().any(|f| f.contains("display_name")), "{failed:?}");
}

/// An accepted view field is accepted only as the folded state says it: an invented role in a
/// room's `roles` is named, and a new field not on the list is named as before.
#[test]
fn an_accepted_view_field_with_an_invented_value_is_named() {
    let real = serde_json::json!([["c3".repeat(32), "admitter"]]);
    let then = serde_json::json!({ "messages": [] });
    let run = |now: Value| {
        let mut now = now;
        let mut out = vec![];
        accept("forum", &then, &mut now, |f| (f == "roles").then(|| real.clone()), "forum x", &mut out);
        departures(&then, &now, "forum x", &mut out);
        out
    };
    assert_eq!(run(serde_json::json!({ "messages": [], "roles": real.clone() })), Vec::<String>::new(), "the folded state's roles: accepted");
    let invented = run(serde_json::json!({ "messages": [], "roles": [["c3".repeat(32), "admitter"], ["ee".repeat(32), "admin"]] }));
    assert!(invented.iter().any(|d| d.contains("forum x.roles") && d.contains(&"ee".repeat(32))), "an invented role is named: {invented:?}");
    let other = run(serde_json::json!({ "messages": [], "roles": real.clone(), "offices": ["x"] }));
    assert!(other.iter().any(|d| d.contains("forum x.offices: new and not empty")), "a field not on the list: {other:?}");
    let group = run(serde_json::json!({ "messages": [], "roles": real }));
    assert!(group.is_empty(), "the control, again");
}

/// The rule at depth, as row 3's PartRef meets it: a field a revision adds, blank, is additive
/// anywhere; a value in it, or any field of before that changed or went, is named.
#[test]
fn a_blank_new_field_is_additive_at_any_depth_and_nothing_else_is() {
    let then = r#"GroupState { name: "Site", parts: {"ab": PartRef { role: "room", at: 5 }}, roles: [] }"#;
    let departs = |now: &str| {
        let mut out = vec![];
        state_departures(then, now, "group", &mut out);
        out
    };
    let blank = r#"GroupState { name: "Site", parts: {"ab": PartRef { role: "room", at: 5, choice: None }}, roles: [], picked: {} }"#;
    assert_eq!(departs(blank), Vec::<String>::new(), "a blank new field, nested or not");
    for (now, why) in [
        (r#"GroupState { name: "Site", parts: {"ab": PartRef { role: "room", at: 5, choice: Some("skills") }}, roles: [] }"#, "choice"),
        (r#"GroupState { name: "Site", parts: {"ab": PartRef { role: "host", at: 5 }}, roles: [] }"#, "role"),
        (r#"GroupState { name: "Site", parts: {"ab": PartRef { role: "room", at: 6 }}, roles: [] }"#, "at"),
        (r#"GroupState { name: "Site", parts: {"ab": PartRef { at: 5 }}, roles: [] }"#, "role: gone"),
        (r#"GroupState { name: "Site", parts: {}, roles: [] }"#, "parts"),
        (r#"GroupState { name: "Site", parts: {"ab": PartRef { role: "room", at: 5 }}, roles: ["x"] }"#, "roles"),
        (r#"GroupState { name: "Site", parts: {"ab": PartRef { role: "room", at: 5 }}, roles: [], picked: {"n": "skills"} }"#, "picked"),
    ] {
        let out = departs(now);
        assert!(out.iter().any(|d| d.contains(why)), "{why}: {out:?}");
    }
}

/// Every state the fixture records reads back as a tree, so the check compares by structure,
/// never by text alone.
#[test]
fn every_state_the_fixture_records_parses() {
    for o in fixture()["objects"].as_array().unwrap() {
        let state = o["state"].as_str().expect("a recorded state");
        assert!(parse(state).is_some(), "{} {}: {}", o["kind"], &o["id"].as_str().unwrap()[..12], clip(state));
    }
}
