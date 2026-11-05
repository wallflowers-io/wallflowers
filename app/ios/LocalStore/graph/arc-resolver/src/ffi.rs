//! C ABI for arc-resolver — the ONE on-device handle that owns `graph.lode`.
//!
//! Writes via `project_objects` / `rehome_entity`; reads via `entities` / `neighbors` /
//! `history` / `get_entity` / `episode_index` / `semantic_entities` / `stats`. A single handle
//! owns the graph (the lodedb-core index takes a writer lock, so a second handle to the same
//! path would conflict) — the Swift/Kotlin consumer reads through THIS handle, it does not open
//! its own.
//!
//! Convention: every verb takes JSON in as a NUL-terminated C string and returns a
//! heap-allocated JSON C string the caller frees with `arc_resolver_string_free`. Read/write
//! failures return a JSON object `{"error": "..."}`; `arc_resolver_open` returns null and sets
//! `*err_out`. Panics are caught at the boundary. Not thread-safe: serialize calls externally
//! (the writes take `&mut`, reads take `&`).

use std::ffi::{c_char, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::Arc;

use lodedb_graph::{AsOf, Direction, Embedder, GraphConfig};
use serde_json::{json, Value};

use crate::embedder::{CallbackEmbedder, EmbedCallback};
use crate::ArcResolver;

/// Opaque handle wrapping an [`ArcResolver`] (owns the TemporalGraph and the embedder it indexes with).
pub struct ArcResolverHandle {
    resolver: ArcResolver,
}

// -- helpers ------------------------------------------------------------------

/// # Safety: `p` is a valid NUL-terminated C string or null.
unsafe fn cstr<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    CStr::from_ptr(p).to_str().ok()
}

fn cstring(v: &Value) -> *mut c_char {
    CString::new(v.to_string()).map(CString::into_raw).unwrap_or(ptr::null_mut())
}

fn err(msg: impl std::fmt::Display) -> *mut c_char {
    cstring(&json!({ "error": msg.to_string() }))
}

/// `null` -> Now; integer -> At(ms); "all"/"history" -> All.
fn as_of_from(v: Option<&Value>) -> AsOf {
    match v {
        Some(Value::Number(n)) => n.as_i64().map(AsOf::At).unwrap_or(AsOf::Now),
        Some(Value::String(s)) if s == "all" || s == "history" => AsOf::All,
        _ => AsOf::Now,
    }
}

/// Run a fallible JSON-producing closure with panic capture; return a JSON C string. The
/// closure captures only SAFE references (raw-pointer derefs happen in the verb body first).
fn verb<F: FnOnce() -> Result<Value, String>>(f: F) -> *mut c_char {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(v)) => cstring(&v),
        Ok(Err(e)) => err(e),
        Err(_) => err("panic in arc-resolver ffi"),
    }
}

// -- open / free --------------------------------------------------------------

/// Open a resolver. `config_json`:
/// `{graph_path, vector_dim?=768, index_facts?=true, embed_model?, embed_tokenizer?}`.
///
/// Embedder precedence: the injected `embed_callback` (mobile — the platform's ONNX runtime)
/// wins; else the ort embedder from `embed_model`/`embed_tokenizer` (desktop, feature
/// `ort-embedder`); else none. `embed_dim` is the callback's output dimension (ignored for ort).
/// Returns a handle, or null with `*err_out` = JSON error.
///
/// # Safety: `config_json` is a valid C string; `err_out` is a valid pointer or null; if
/// `embed_callback` is set, it + `embed_ctx` stay valid for the handle's lifetime.
#[no_mangle]
pub unsafe extern "C" fn arc_resolver_open(
    config_json: *const c_char,
    embed_callback: Option<EmbedCallback>,
    embed_ctx: *mut c_void,
    embed_dim: usize,
    err_out: *mut *mut c_char,
) -> *mut ArcResolverHandle {
    let cfg_str = match cstr(config_json) {
        Some(s) => s,
        None => {
            if !err_out.is_null() {
                *err_out = err("config is null or not UTF-8");
            }
            return ptr::null_mut();
        }
    };
    // Build the injected embedder here (unsafe), then hand a safe Option into the closure.
    let injected: Option<Arc<dyn Embedder>> = embed_callback.map(|cb| {
        let dim = if embed_dim > 0 { embed_dim } else { 768 };
        // SAFETY: caller guarantees cb + embed_ctx stay valid for the handle's lifetime.
        Arc::new(unsafe { CallbackEmbedder::new(cb, embed_ctx, dim) }) as Arc<dyn Embedder>
    });

    let built = catch_unwind(AssertUnwindSafe(move || -> Result<*mut ArcResolverHandle, String> {
        let cfg: Value = serde_json::from_str(cfg_str).map_err(|e| format!("config JSON: {e}"))?;
        let graph_path = cfg.get("graph_path").and_then(Value::as_str).ok_or("graph_path required")?;
        let index_facts = cfg.get("index_facts").and_then(Value::as_bool).unwrap_or(true);
        let cfg_dim = cfg.get("vector_dim").and_then(Value::as_u64).unwrap_or(768) as usize;

        // Embedder + the graph's vector dim.
        let (embedder, vector_dim): (Option<Arc<dyn Embedder>>, usize) = match injected {
            Some(e) => {
                let dim = e.dimension();
                (Some(e), dim)
            }
            None => resolve_ort_embedder(&cfg, cfg_dim)?,
        };

        let gcfg = GraphConfig { vector_dim, index_text: true, index_facts };
        let resolver = ArcResolver::open(graph_path, gcfg, embedder)
            .map_err(|e| format!("open: {e}"))?;
        Ok(Box::into_raw(Box::new(ArcResolverHandle { resolver })))
    }));

    match built {
        Ok(Ok(h)) => h,
        Ok(Err(e)) => {
            if !err_out.is_null() {
                *err_out = err(e);
            }
            ptr::null_mut()
        }
        Err(_) => {
            if !err_out.is_null() {
                *err_out = err("panic in arc_resolver_open");
            }
            ptr::null_mut()
        }
    }
}

#[cfg(feature = "ort-embedder")]
fn resolve_ort_embedder(cfg: &Value, dim: usize) -> Result<(Option<Arc<dyn Embedder>>, usize), String> {
    match (
        cfg.get("embed_model").and_then(Value::as_str),
        cfg.get("embed_tokenizer").and_then(Value::as_str),
    ) {
        (Some(m), Some(t)) => {
            let e = crate::OrtEmbedder::load(m, t, dim).map_err(|e| format!("embedder load: {e}"))?;
            Ok((Some(Arc::new(e) as Arc<dyn Embedder>), dim))
        }
        _ => Ok((None, dim)),
    }
}

#[cfg(not(feature = "ort-embedder"))]
fn resolve_ort_embedder(_cfg: &Value, dim: usize) -> Result<(Option<Arc<dyn Embedder>>, usize), String> {
    Ok((None, dim))
}

/// Free a handle from [`arc_resolver_open`].
/// # Safety: `h` came from `arc_resolver_open` and is not used again.
#[no_mangle]
pub unsafe extern "C" fn arc_resolver_free(h: *mut ArcResolverHandle) {
    if !h.is_null() {
        drop(Box::from_raw(h));
    }
}

/// Free a JSON string returned by any verb.
/// # Safety: `s` came from an arc-resolver verb and is not used again.
#[no_mangle]
pub unsafe extern "C" fn arc_resolver_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

// -- write --------------------------------------------------------------------

/// Project the app's minted GroupObjects into the graph (the ratified-delta
/// projection — deterministic). `req_json`:
/// `{now_ms, objects: [{id, kind, name, shape?, members?: [{space, name?}],
///   affiliations?: [{peer, rel, name?, tether?, at}], venue?: {place, name?, at?}, at?}, ...]}`.
/// `objects` is the COMPLETE snapshot: a previously projected relationship absent
/// from it is invalidated at `now_ms` (bi-temporal history is retained). Returns
/// the ProjectStats JSON.
/// # Safety: `h` is a live handle; `req_json` is a valid C string. Not called concurrently.
#[no_mangle]
pub unsafe extern "C" fn arc_resolver_project_objects(
    h: *mut ArcResolverHandle,
    req_json: *const c_char,
) -> *mut c_char {
    let handle = match h.as_mut() {
        Some(x) => x,
        None => return err("null handle"),
    };
    let req_str = match cstr(req_json) {
        Some(s) => s,
        None => return err("request is null or not UTF-8"),
    };
    verb(move || {
        let req: Value = serde_json::from_str(req_str).map_err(|e| format!("req JSON: {e}"))?;
        let now_ms = req.get("now_ms").and_then(Value::as_i64).ok_or("now_ms (ms) required")?;
        let objects: Vec<crate::ObjectSnapshot> =
            serde_json::from_value(req.get("objects").cloned().unwrap_or(Value::Array(vec![])))
                .map_err(|e| format!("objects: {e}"))?;
        let stats = handle
            .resolver
            .project_objects(&objects, now_ms)
            .map_err(|e| format!("project: {e}"))?;
        serde_json::to_value(&stats).map_err(|e| e.to_string())
    })
}

/// PROMOTION: re-point a text-lifted ghost entity onto a freshly minted GroupObject's
/// node — facts, provenance, and summary move; the ghost is removed. `req_json`:
/// `{ghost_id, object_id, object_type, name}`. Returns `{"facts_moved": n}`.
/// # Safety: `h` is a live handle; `req_json` is a valid C string. Not called concurrently.
#[no_mangle]
pub unsafe extern "C" fn arc_resolver_rehome_entity(
    h: *mut ArcResolverHandle,
    req_json: *const c_char,
) -> *mut c_char {
    let handle = match h.as_mut() {
        Some(x) => x,
        None => return err("null handle"),
    };
    let req_str = match cstr(req_json) {
        Some(s) => s,
        None => return err("request is null or not UTF-8"),
    };
    verb(move || {
        let req: Value = serde_json::from_str(req_str).map_err(|e| format!("req JSON: {e}"))?;
        let get = |k: &str| req.get(k).and_then(Value::as_str).ok_or(format!("{k} required"));
        let moved = handle
            .resolver
            .rehome_entity(get("ghost_id")?, get("object_id")?, get("object_type")?, get("name")?)
            .map_err(|e| format!("rehome: {e}"))?;
        Ok(json!({ "facts_moved": moved }))
    })
}

// -- reads --------------------------------------------------------------------

/// # Safety: `h` is a live handle; `req_json` is a valid C string or null.
#[no_mangle]
pub unsafe extern "C" fn arc_resolver_entities(h: *const ArcResolverHandle, req_json: *const c_char) -> *mut c_char {
    let handle = match h.as_ref() {
        Some(x) => x,
        None => return err("null handle"),
    };
    let req_str = cstr(req_json).unwrap_or("{}");
    verb(move || {
        let req: Value = serde_json::from_str(req_str).map_err(|e| e.to_string())?;
        let etype = req.get("type").and_then(Value::as_str);
        let ents = handle.resolver.graph().entities(etype, as_of_from(req.get("as_of"))).map_err(|e| e.to_string())?;
        serde_json::to_value(&ents).map_err(|e| e.to_string())
    })
}

/// Facts incident to an entity. `req_json`: `{id, direction?="both", relation?, as_of?}`.
/// # Safety: `h` is a live handle; `req_json` is a valid C string.
#[no_mangle]
pub unsafe extern "C" fn arc_resolver_neighbors(h: *const ArcResolverHandle, req_json: *const c_char) -> *mut c_char {
    let handle = match h.as_ref() {
        Some(x) => x,
        None => return err("null handle"),
    };
    let req_str = match cstr(req_json) {
        Some(s) => s,
        None => return err("request is null or not UTF-8"),
    };
    verb(move || {
        let req: Value = serde_json::from_str(req_str).map_err(|e| e.to_string())?;
        let id = req.get("id").and_then(Value::as_str).ok_or("id required")?;
        let direction = Direction::parse(req.get("direction").and_then(Value::as_str).unwrap_or("both"))
            .map_err(|e| e.to_string())?;
        let relation = req.get("relation").and_then(Value::as_str);
        let facts = handle.resolver.graph().neighbors(id, direction, relation, as_of_from(req.get("as_of"))).map_err(|e| e.to_string())?;
        serde_json::to_value(&facts).map_err(|e| e.to_string())
    })
}

/// All facts incident to an entity across every frame (the history strip). `req_json`: `{id}`.
/// # Safety: `h` is a live handle; `req_json` is a valid C string.
#[no_mangle]
pub unsafe extern "C" fn arc_resolver_history(h: *const ArcResolverHandle, req_json: *const c_char) -> *mut c_char {
    let handle = match h.as_ref() {
        Some(x) => x,
        None => return err("null handle"),
    };
    let req_str = match cstr(req_json) {
        Some(s) => s,
        None => return err("request is null or not UTF-8"),
    };
    verb(move || {
        let req: Value = serde_json::from_str(req_str).map_err(|e| e.to_string())?;
        let id = req.get("id").and_then(Value::as_str).ok_or("id required")?;
        let facts = handle.resolver.graph().neighbors(id, Direction::Both, None, AsOf::All).map_err(|e| e.to_string())?;
        serde_json::to_value(&facts).map_err(|e| e.to_string())
    })
}

/// One entity by id, or `null`. `req_json`: `{id}`.
/// # Safety: `h` is a live handle; `req_json` is a valid C string.
#[no_mangle]
pub unsafe extern "C" fn arc_resolver_get_entity(h: *const ArcResolverHandle, req_json: *const c_char) -> *mut c_char {
    let handle = match h.as_ref() {
        Some(x) => x,
        None => return err("null handle"),
    };
    let req_str = match cstr(req_json) {
        Some(s) => s,
        None => return err("request is null or not UTF-8"),
    };
    verb(move || {
        let req: Value = serde_json::from_str(req_str).map_err(|e| e.to_string())?;
        let id = req.get("id").and_then(Value::as_str).ok_or("id required")?;
        let ent = handle.resolver.graph().get_entity(id).map_err(|e| e.to_string())?;
        serde_json::to_value(&ent).map_err(|e| e.to_string())
    })
}

/// Provenance index: `[{episode_id, source, occurred_at, entities:[id,...]}, ...]`, one row per
/// episode. Invert it to get `entity -> [source object]` — an episode's id is the caller-stable
/// id it was written under, so that object id IS the note's UUID. Takes no request body.
/// # Safety: `h` is a live handle.
#[no_mangle]
pub unsafe extern "C" fn arc_resolver_episode_index(h: *const ArcResolverHandle) -> *mut c_char {
    let handle = match h.as_ref() {
        Some(x) => x,
        None => return err("null handle"),
    };
    verb(move || {
        let rows = handle.resolver.episode_index().map_err(|e| e.to_string())?;
        serde_json::to_value(&rows).map_err(|e| e.to_string())
    })
}

/// Semantic entity search. `req_json`: `{query, k?=10, type?, as_of?}`. Returns `[[score, Entity], ...]`.
/// # Safety: `h` is a live handle; `req_json` is a valid C string.
#[no_mangle]
pub unsafe extern "C" fn arc_resolver_semantic_entities(h: *const ArcResolverHandle, req_json: *const c_char) -> *mut c_char {
    let handle = match h.as_ref() {
        Some(x) => x,
        None => return err("null handle"),
    };
    let req_str = match cstr(req_json) {
        Some(s) => s,
        None => return err("request is null or not UTF-8"),
    };
    verb(move || {
        let req: Value = serde_json::from_str(req_str).map_err(|e| e.to_string())?;
        let query = req.get("query").and_then(Value::as_str).ok_or("query required")?;
        let k = req.get("k").and_then(Value::as_u64).unwrap_or(10) as usize;
        let etype = req.get("type").and_then(Value::as_str);
        let hits = handle.resolver.graph().semantic_entities(Some(query), None, k, etype, as_of_from(req.get("as_of"))).map_err(|e| e.to_string())?;
        serde_json::to_value(&hits).map_err(|e| e.to_string())
    })
}

/// Graph counts `{entities, facts, indexed_documents}`.
/// # Safety: `h` is a live handle.
#[no_mangle]
pub unsafe extern "C" fn arc_resolver_stats(h: *const ArcResolverHandle) -> *mut c_char {
    let handle = match h.as_ref() {
        Some(x) => x,
        None => return err("null handle"),
    };
    verb(move || {
        let s = handle.resolver.graph().stats().map_err(|e| e.to_string())?;
        Ok(json!({ "entities": s.entities, "facts": s.facts, "indexed_documents": s.indexed_documents }))
    })
}
