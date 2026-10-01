//! THE ROUTE INVENTORY: every route the Door, its sessions and the Arc gateway serve,
//! read from their routers' own source, with each handler's refusals in its own words and whether
//! it writes, and the limits the code states. `routes.json` beside the crate is this, plus what
//! cannot be read from the source, written once: each route's audience.
//!
//!   ROUTES_WRITE=1 cargo test --bin door routes::   writes routes.json, keeping what is written
//!
//! check-door holds routes.json to the routers: a served route with no entry, an entry for a route
//! not served, or a generated field that moved, is red.
//!
//! An error's `source` is where its words are written: the router's handler (`door`, `session`,
//! `gateway`); the session's actor arm a forwarded route's `Ask` reaches (`session`); or core's
//! `authoring::Refusal` (`model`), where that arm's Node calls reach `authoring::build`. Words are
//! the literal as written, `{…}` where the code fills a value; null where they are made.

use std::collections::{BTreeMap, BTreeSet};

const JSON: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/routes.json");

/// A source, up to its tests.
fn cut(s: &'static str) -> &'static str {
    s.split("\n#[cfg(test)]\nmod tests {").next().unwrap_or(s)
}

/// A router's source, up to its tests: `(router, source)`.
fn sources() -> [(&'static str, &'static str); 3] {
    [
        ("door", cut(include_str!("main.rs"))),
        ("session", cut(include_str!("account.rs"))),
        ("gateway", cut(include_str!("../../../arc/gateway/src/main.rs"))),
    ]
}

/// Core's write path: the Node's methods, the refusals, the error they are wrapped in.
fn core() -> (&'static str, &'static str, &'static str) {
    (
        cut(include_str!("../../../core/pacific-core/src/node.rs")),
        cut(include_str!("../../../core/pacific-core/src/authoring.rs")),
        cut(include_str!("../../../core/pacific-core/src/lib.rs")),
    )
}

/// What an audience may be, for the docs: a site's server or page, the webapp, the
/// sign-in window, the Door's own processes, or a static file.
const AUDIENCES: &[&str] = &["site", "webapp", "window", "internal", "asset", "arc"];

/// Fields a person writes, kept across a regeneration.
const WRITTEN: &[&str] = &["audience", "request", "response", "notes"];

#[derive(Clone, Debug, PartialEq)]
struct Route {
    router: &'static str,
    method: String,
    path: String,
    handler: String,
}

/// Every `.route("path", method(handler))` a router's source serves.
fn served() -> Vec<Route> {
    let mut out = vec![];
    for (router, src) in sources() {
        for line in src.lines() {
            let Some(rest) = line.trim().strip_prefix(".route(\"") else { continue };
            let Some((path, call)) = rest.split_once("\", ") else { continue };
            let Some((method, handler)) = call.split_once('(') else { continue };
            let handler = handler.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':')).next().unwrap_or("");
            let method = match method.rsplit("::").next().unwrap_or(method) {
                "any" => "ANY".to_string(),
                m => m.to_ascii_uppercase(),
            };
            out.push(Route { router, method, path: path.to_string(), handler: handler.to_string() });
        }
    }
    out
}

/// The body of `fn name` in `src`: from its signature to the next item at the margin.
fn body_of<'a>(src: &'a str, name: &str) -> Option<&'a str> {
    let name = name.rsplit("::").next().unwrap_or(name);
    let at = ["\nasync fn ", "\nfn ", "\npub async fn ", "\npub fn "]
        .iter()
        .flat_map(|f| [format!("{f}{name}("), format!("{f}{name}<")])
        .filter_map(|p| src.find(p.as_str()))
        .min()?;
    let rest = &src[at + 1..];
    let end = rest[1..]
        .find("\nfn ")
        .into_iter()
        .chain(rest[1..].find("\nasync fn "))
        .chain(rest[1..].find("\npub fn "))
        .chain(rest[1..].find("\npub async fn "))
        .chain(rest[1..].find("\n#["))
        .min()
        .map(|e| e + 1)
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

/// A handler's refusals, and those of the functions it calls, three calls deep: `(status,
/// words)`, the words where they are a literal, None where they are the service's own or made.
fn refusals(src: &str, handler: &str) -> BTreeSet<(String, Option<String>)> {
    let mut out = BTreeSet::new();
    if !refuses(src, handler) {
        return out;
    }
    let mut seen = BTreeSet::new();
    let mut todo = vec![(handler.to_string(), 0)];
    while let Some((f, depth)) = todo.pop() {
        if !seen.insert(f.clone()) {
            continue;
        }
        let Some(body) = body_of(src, &f) else { continue };
        for (i, _) in body.match_indices("refused(StatusCode::") {
            let rest = &body[i + "refused(StatusCode::".len()..];
            // The third argument: after the status and the route.
            out.insert((status_at(rest), words_at(rest.splitn(3, ", ").nth(2).unwrap_or(""))));
        }
        for (i, _) in body.match_indices("StatusCode::") {
            if body[..i].ends_with("refused(") {
                continue;
            }
            let rest = &body[i + "StatusCode::".len()..];
            let status = status_at(rest);
            if matches!(status.as_str(), "" | "OK" | "SEE_OTHER" | "FOUND" | "NO_CONTENT" | "NOT_MODIFIED" | "CREATED" | "TEMPORARY_REDIRECT") {
                continue;
            }
            let after = &rest[status.len()..];
            // `f(…).map_err(|e| (StatusCode::X, e))`: f's own words, at X.
            if body[..i].ends_with(".map_err(|e| (") && after.starts_with(", e)") {
                let line = &body[body[..i].rfind('\n').map_or(0, |n| n + 1)..i];
                for f in calls(src, line) {
                    for (_, w) in said(src, body_of(src, &f).unwrap_or(""), false, 1) {
                        out.insert((status.clone(), Some(w)));
                    }
                }
            }
            out.insert((status, after.strip_prefix(", ").and_then(words_at)));
        }
        if body.contains("no_face()") {
            out.insert(("NOT_FOUND".into(), Some("no such page".into())));
        }
        if depth < 3 {
            todo.extend(calls(src, body).into_iter().map(|f| (f, depth + 1)));
        }
    }
    out
}

/// Whether a handler can answer other than its success: its type names a status, a response
/// or a refusal. A stream's cannot.
fn refuses(src: &str, handler: &str) -> bool {
    let sig = body_of(src, handler).and_then(|b| b.split("{\n").next()).unwrap_or("");
    sig.split_once("->").is_some_and(|(_, t)| ["StatusCode", "Response", "Refusal"].iter().any(|w| t.contains(w)))
}

fn status_at(s: &str) -> String {
    s.chars().take_while(|c| c.is_ascii_uppercase() || *c == '_').collect()
}

/// The string literal `s` opens with, `format!("…"` included, unescaped, its placeholders kept.
fn words_at(s: &str) -> Option<String> {
    let s = s.trim_start();
    let s = s.strip_prefix("format!(").map(str::trim_start).unwrap_or(s);
    let mut cs = s.strip_prefix('"')?.chars();
    let mut out = String::new();
    while let Some(c) = cs.next() {
        match c {
            '"' => return Some(out),
            '\\' => match cs.next()? {
                'n' => out.push('\n'),
                // A continued line: the break and the next line's indent are not in the string.
                '\n' => cs = cs.as_str().trim_start().chars(),
                e => out.push(e),
            },
            c => out.push(c),
        }
    }
    None
}

/// The functions of `src` that `body` calls by name: not a method, not a type's.
fn calls(src: &str, body: &str) -> Vec<String> {
    let mut out = vec![];
    let mut word = String::new();
    for (j, c) in body.char_indices() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
            continue;
        }
        if c == '(' && !word.is_empty() && !word.starts_with(|c: char| c.is_ascii_uppercase()) && body[..j].chars().rev().nth(word.len()) != Some('.') && body_of(src, &word).is_some() {
            out.push(word.clone());
        }
        word.clear();
    }
    out
}

/// The words `body` refuses in, written as literals (an `Err`, an `ok_or`, a `map_err` to
/// words; `Some(format!(…))` in a helper answering `Option<String>`), and those of the helpers
/// it calls that answer words, two calls deep: `(sent, words)`, sent where they go straight to
/// `back.send`.
fn said(src: &str, body: &str, some: bool, depth: usize) -> Vec<(bool, String)> {
    let mut out = vec![];
    for pat in ["Err(", "ok_or(", "map_err(|e| ", "map_err(|_| ", "Some(format!("] {
        if pat.starts_with("Some") && !some {
            continue;
        }
        for (i, _) in body.match_indices(pat) {
            if let Some(w) = words_at(&body[i + pat.len()..]) {
                out.push((body[..i].ends_with("back.send("), w));
            }
        }
    }
    if depth < 2 {
        for f in calls(src, body) {
            let fb = body_of(src, &f).unwrap_or("");
            let sig = fb.split("{\n").next().unwrap_or("").trim_end();
            let opt = sig.ends_with("-> Option<String>");
            if opt || sig.ends_with(", String>") {
                out.extend(said(src, fb, opt, depth + 1).into_iter().map(|(_, w)| (false, w)));
            }
        }
    }
    out
}

/// A forwarded route's refusals past its handler: the actor's arm for the `Ask` it makes, at the
/// status `ask` answers an arm's refusal with, or, not sent straight back (a batch's refused
/// step), at the one its handler gives it; `refuse`'s, where the ask is one it answers; and
/// core's, where the arm answers in core's words and its Node calls reach `authoring::build`.
/// None where the handler answers no refusal (a stream).
fn arm_errors(src: &str, handler: &str) -> Vec<Said> {
    let body = body_of(src, handler).unwrap_or("");
    if !refuses(src, handler) {
        return vec![];
    }
    let Some(ask) = body.split_once("Ask::").map(|(_, r)| r.chars().take_while(|c| c.is_alphanumeric()).collect::<String>()) else { return vec![] };
    let Some(sent) = body_of(src, "ask").and_then(|b| b.lines().find(|l| l.contains("Ok(Err("))).and_then(|l| l.split_once("StatusCode::")).map(|(_, r)| status_at(r)) else {
        return vec![];
    };
    let Some(arm) = arm_of(src, &ask) else { return vec![] };
    let bare = body
        .lines()
        .find(|l| l.contains("[\"refused\"]"))
        .and_then(|l| l.split("StatusCode::").skip(1).map(status_at).find(|s| s != "OK"))
        .unwrap_or_else(|| sent.clone());
    let mut out: Vec<Said> = said(src, arm, false, 0).into_iter().map(|(s, w)| (1, if s { sent.clone() } else { bare.clone() }, Some(w), "session")).collect();
    let answers = body_of(src, "refuse").is_some_and(|b| b.contains(&format!("Ask::{ask} ")) || b.contains(&format!("Ask::{ask}(")));
    if answers {
        for (i, _) in src.match_indices("refuse(") {
            if let Some(w) = src[i + "refuse(".len()..].split_once(", ").and_then(|(_, w)| words_at(w)) {
                out.push((1, sent.clone(), Some(w), "session"));
            }
        }
    }
    if arm.contains("e.to_string()") && reaches_build(arm) {
        out.extend(model().into_iter().map(|w| (2, bare.clone(), Some(w), "model")));
    }
    out
}

/// The actor's arm for `Ask::{ask}`: after its `=> {`, to the next line back at its indent.
fn arm_of<'a>(src: &'a str, ask: &str) -> Option<&'a str> {
    let mut at = 0;
    for line in src.split_inclusive('\n') {
        at += line.len();
        let t = line.trim_start();
        if !(t.starts_with(&format!("Ask::{ask} ")) || t.starts_with(&format!("Ask::{ask}("))) || !t.trim_end().ends_with("=> {") {
            continue;
        }
        let indent = line.len() - t.len();
        let rest = &src[at..];
        let mut end = 0;
        for l in rest.split_inclusive('\n') {
            let lt = l.trim_start();
            if !lt.trim().is_empty() && l.len() - lt.len() <= indent {
                break;
            }
            end += l.len();
        }
        return Some(&rest[..end]);
    }
    None
}

/// Whether the Node methods `arm` calls reach `authoring::build`, through the Node's own calls.
fn reaches_build(arm: &str) -> bool {
    let node = core().0;
    let mut todo = named_after(arm, "n.");
    let mut seen = BTreeSet::new();
    while let Some(m) = todo.pop() {
        if !seen.insert(m.clone()) {
            continue;
        }
        let Some(b) = method_of(node, &m) else { continue };
        if b.contains("authoring::build(") {
            return true;
        }
        todo.extend(named_after(b, "self."));
    }
    false
}

/// The names called as `{on}name(` in `body`.
fn named_after(body: &str, on: &str) -> Vec<String> {
    body.match_indices(on)
        .filter(|(i, _)| !body[..*i].ends_with(|c: char| c.is_alphanumeric() || c == '_'))
        .filter_map(|(i, _)| {
            let r = &body[i + on.len()..];
            let n: String = r.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            r[n.len()..].starts_with('(').then_some(n)
        })
        .collect()
}

/// A method's body in an `impl`: from its signature to the brace that closes it at its indent.
fn method_of<'a>(src: &'a str, name: &str) -> Option<&'a str> {
    let at = [format!(" fn {name}("), format!(" fn {name}<")]
        .iter()
        .flat_map(|p| src.match_indices(p.as_str()).map(|(i, _)| i).collect::<Vec<_>>())
        .filter(|i| {
            let line = &src[src[..*i].rfind('\n').map_or(0, |n| n + 1)..*i];
            line.trim_start().split(' ').all(|w| matches!(w, "" | "pub" | "pub(crate)" | "async"))
        })
        .min()?;
    let start = src[..at].rfind('\n').map_or(0, |n| n + 1);
    let indent = src[start..].len() - src[start..].trim_start().len();
    let close = format!("\n{}}}\n", " ".repeat(indent));
    let end = src[at..].find(&close).map_or(src.len(), |e| at + e);
    Some(&src[start..end])
}

/// Core's refusals as its write path answers them: each `authoring::Refusal`'s Display
/// template, in the `CoreError` the Node wraps it in where it calls `authoring::build`.
fn model() -> Vec<String> {
    let (node, authoring, lib) = core();
    let wrap: String = node
        .split("authoring::build(")
        .nth(1)
        .and_then(|r| r.split(".map_err(|r| CoreError::").nth(1))
        .map(|r| r.chars().take_while(|c| c.is_alphanumeric()).collect())
        .expect("the Node's call of authoring::build, and its CoreError");
    let shape = lib
        .find(&format!("\n    {wrap}(String)"))
        .and_then(|at| lib[..at].rfind("#[error(").map(|e| &lib[e + "#[error(".len()..]))
        .and_then(words_at)
        .expect("the CoreError's #[error]");
    let within = |from: &str| authoring.split_once(from).map(|(_, r)| r.split("\n}\n").next().unwrap_or(r)).expect(from);
    let variants = within("pub enum Refusal {").lines().filter(|l| l.starts_with("    ") && l[4..].starts_with(|c: char| c.is_ascii_uppercase())).count();
    let said: Vec<String> = within("impl std::fmt::Display for Refusal {")
        .split("Refusal::")
        .skip(1)
        .filter_map(|arm| arm.split_once("write!(").and_then(|(_, w)| w.find('"').and_then(|q| words_at(&w[q..]))))
        .map(|t| shape.replace("{0}", &t))
        .collect();
    assert_eq!(said.len(), variants, "a Display template for each of Refusal's {variants} variants: {said:?}");
    said
}

/// An error: `(order, status, words, source)`, ordered by where it is met: the router's own,
/// the session's, core's.
type Said = (u8, String, Option<String>, &'static str);

/// A route's errors: its handler's, and, a route the Door forwards, its session's.
fn errors(r: &Route, routes: &[Route]) -> BTreeSet<Said> {
    let src = sources().into_iter().find(|(n, _)| *n == r.router).map_or("", |(_, s)| s);
    let mut out: BTreeSet<Said> = refusals(src, &r.handler).into_iter().map(|(s, w)| (u8::from(r.router == "session"), s, w, r.router)).collect();
    let fwd = |h: &str| matches!(h, "fwd" | "fwd_stream");
    let at = |router: &str, x: &Route| x.router == router && x.method == r.method && x.path == r.path;
    match r.router {
        "door" if fwd(&r.handler) => {
            if let Some(s) = routes.iter().find(|s| at("session", s)) {
                out.extend(errors(s, routes));
            }
        }
        "session" if routes.iter().any(|d| at("door", d) && fwd(&d.handler)) => out.extend(arm_errors(src, &r.handler)),
        _ => {}
    }
    out
}

/// Who may call it, from what its handler does: the webapp's cookie or a site's DPoP token
/// (`session_port`, a token held to its Site: `in_scope`), the cookie alone, a Door
/// session's own supervisor (`/i/`), or anyone. The gateway's, by the router group it is in.
fn auth(r: &Route, src: &str) -> &'static str {
    if r.router == "session" {
        return "internal";
    }
    if r.router == "gateway" {
        let at = src.find(&format!(".route(\"{}\"", r.path)).unwrap_or(0);
        let group = src[..at].rfind("let ").map(|g| &src[g..at]).unwrap_or("");
        return if group.starts_with("let public") {
            "none"
        } else if group.starts_with("let console_surface") {
            "console"
        } else {
            gate_role(src, &r.path)
        };
    }
    let body = body_of(src, &r.handler).unwrap_or("");
    if body.contains("is the webapp's") {
        "cookie"
    } else if body.contains("session_port(") || body.contains("bearer(") || r.handler == "fwd" || r.handler == "fwd_stream" {
        "session"
    } else if r.handler == "token_exchange" {
        "dpop"
    } else {
        "none"
    }
}

/// The role the gateway's membership gate asks of a path, read from its `required_role`: each
/// branch's `starts_with("…")` or `== "…"` to the `Role::…` it answers, the last its default.
/// A guest is anyone: "none".
fn gate_role(src: &str, path: &str) -> &'static str {
    let body = body_of(src, "required_role").unwrap_or("");
    let path = path.trim_end_matches("*rest").trim_end_matches('/');
    let mut pats: Vec<&str> = vec![];
    for line in body.lines() {
        for m in ["starts_with(\"", "== \""] {
            for (i, _) in line.match_indices(m) {
                if let Some(p) = line[i + m.len()..].split('"').next() {
                    pats.push(p);
                }
            }
        }
        if let Some(i) = line.find("Role::") {
            let role: String = line[i + 6..].chars().take_while(|c| c.is_alphanumeric()).collect();
            let hit = pats.is_empty() || pats.iter().any(|p| path == *p || path.starts_with(p));
            if hit {
                return match role.as_str() {
                    "Guest" => "none",
                    "Viewer" => "viewer",
                    "Admin" => "admin",
                    _ => "member",
                };
            }
            pats.clear();
        }
    }
    "member"
}

/// Whether it writes: the Door's routes by tw's acting-route table (`keep`, `NOT_KEPT`), a GET
/// never; the gateway's by its method.
fn writes(r: &Route) -> bool {
    match r.router {
        "gateway" => r.method != "GET",
        _ => r.method != "GET" && crate::tw::keep(&r.path).is_some(),
    }
}

/// What bounds a route, read where the code states it: a body its handler reads whole
/// (`to_bytes(…, 1 << n)`), the proof of work a ceremony's start asks (`paid`: pow::BITS), a
/// batch's steps (BATCH_MAX). Caps a deploy sets (sessions, attempts, token lives) are config.
fn limits(r: &Route, src: &str) -> serde_json::Value {
    let mut l = serde_json::Map::new();
    let body = body_of(src, &r.handler).unwrap_or("");
    if let Some(i) = body.find("to_bytes(") {
        let shift: String = body[i..].split("1 << ").nth(1).unwrap_or("").chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = shift.parse::<u32>() {
            l.insert("body_bytes".into(), (1u64 << n).into());
        }
    }
    if r.router == "door" && body.contains("paid(") {
        l.insert("pow_bits".into(), crate::pow::BITS.into());
    }
    if r.path == "/v2/batch" && r.router != "gateway" {
        let max = include_str!("account.rs").split("const BATCH_MAX: usize = ").nth(1).and_then(|v| v.split(';').next()).and_then(|v| v.trim().parse::<u64>().ok());
        if let Some(m) = max {
            l.insert("steps_max".into(), m.into());
        }
    }
    serde_json::Value::Object(l)
}

/// One route's entry as the routers say it: everything but what a person writes.
fn generated(r: &Route, src: &str, routes: &[Route]) -> serde_json::Value {
    let errors: Vec<serde_json::Value> = errors(r, routes)
        .into_iter()
        .map(|(_, status, words, source)| serde_json::json!({ "status": status, "words": words, "source": source }))
        .collect();
    serde_json::json!({
        "router": r.router, "method": r.method, "path": r.path, "handler": r.handler,
        "auth": auth(r, src), "writes": writes(r), "errors": errors, "limits": limits(r, src),
    })
}

fn key(v: &serde_json::Value) -> (String, String, String) {
    let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
    (s("router"), s("method"), s("path"))
}

/// The inventory as the routers say it, with what `held` had written for each route kept.
fn inventory(held: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let by: BTreeMap<_, _> = held.iter().map(|e| (key(e), e)).collect();
    let src: BTreeMap<&str, &str> = sources().into_iter().collect();
    let routes = served();
    routes
        .iter()
        .map(|r| {
            let mut e = generated(r, src[r.router], &routes);
            for f in WRITTEN {
                if let Some(v) = by.get(&key(&e)).and_then(|h| h.get(*f)) {
                    e[*f] = v.clone();
                }
            }
            e
        })
        .collect()
}

fn held() -> Vec<serde_json::Value> {
    std::fs::read_to_string(JSON).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Writes routes.json when asked (ROUTES_WRITE=1), and otherwise holds it to the routers.
#[test]
fn routes_json_is_the_routers() {
    let held = held();
    let now = inventory(&held);
    assert!(now.len() > 40 && now.iter().any(|e| e["path"] == "/v2/apply"), "the routers were read: {} routes", now.len());
    for p in ["/v2/mint", "/v2/apply", "/v2/batch"] {
        let from = |s: &str| now.iter().any(|e| e["router"] == "door" && e["path"] == p && e["errors"].as_array().is_some_and(|x| x.iter().any(|x| x["source"] == s)));
        assert!(from("session") && from("model"), "{p}: its session's arm and core's refusals were read");
    }
    if std::env::var("ROUTES_WRITE").is_ok_and(|v| v == "1") {
        std::fs::write(JSON, serde_json::to_string_pretty(&now).unwrap() + "\n").unwrap();
        return;
    }
    let (have, want): (BTreeSet<_>, BTreeSet<_>) = (held.iter().map(key).collect(), now.iter().map(key).collect());
    let missing: Vec<_> = want.difference(&have).collect();
    let stale: Vec<_> = have.difference(&want).collect();
    assert!(missing.is_empty() && stale.is_empty(), "routes.json: {} served route(s) with no entry {missing:?}; {} entr(ies) for no served route {stale:?}", missing.len(), stale.len());
    let by: BTreeMap<_, _> = held.iter().map(|e| (key(e), e)).collect();
    let mut moved = vec![];
    for e in &now {
        let h = by[&key(e)];
        for f in ["handler", "auth", "writes", "limits"] {
            if h[f] != e[f] {
                moved.push(format!("{:?} {f}: routes.json has {}, the router {}", key(e), h[f], e[f]));
            }
        }
        if h["errors"] != e["errors"] {
            let only = |a: &serde_json::Value, b: &serde_json::Value| {
                let b = b.as_array().cloned().unwrap_or_default();
                serde_json::Value::Array(a.as_array().into_iter().flatten().filter(|x| !b.contains(x)).cloned().collect())
            };
            moved.push(format!("{:?} errors: routes.json alone has {}, the router alone {}", key(e), only(&h["errors"], &e["errors"]), only(&e["errors"], &h["errors"])));
        }
        if !h["audience"].as_str().is_some_and(|a| AUDIENCES.contains(&a)) {
            moved.push(format!("{:?}: no audience of {AUDIENCES:?}", key(e)));
        }
    }
    assert!(moved.is_empty(), "routes.json departs from the routers (ROUTES_WRITE=1 regenerates):\n{}", moved.join("\n"));
}
