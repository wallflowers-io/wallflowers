//! Shared typed arg extraction for the object reducers.
//!
//! These are the typed lenses over `Args = BTreeMap<String, ArgVal>` used by
//! every reducer (missing/ill-typed => `MalformedArgs`), extracted 2026-08-01
//! from the per-module private copies (eng-review decision: extract-now).

use crate::coordinator::{ArgVal, Args};
use crate::object::DeltaRejection;

pub(crate) fn req_text<'a>(args: &'a Args, key: &str) -> Result<&'a str, DeltaRejection> {
    match crate::arg_reads::get(args, key) {
        Some(ArgVal::Text(t)) => Ok(t),
        _ => Err(DeltaRejection::MalformedArgs),
    }
}

// Returns Some for ANY Text arg, including the empty string (group/project semantics).
pub(crate) fn opt_text(args: &Args, key: &str) -> Option<String> {
    match crate::arg_reads::get(args, key) {
        Some(ArgVal::Text(t)) => Some(t.clone()),
        _ => None,
    }
}

// Returns Some only for a NON-EMPTY Text arg (thing/place/event semantics).
pub(crate) fn opt_text_nonempty(args: &Args, key: &str) -> Option<String> {
    match crate::arg_reads::get(args, key) {
        Some(ArgVal::Text(t)) if !t.is_empty() => Some(t.clone()),
        _ => None,
    }
}

pub(crate) fn req_int(args: &Args, key: &str) -> Result<i64, DeltaRejection> {
    match crate::arg_reads::get(args, key) {
        Some(ArgVal::Int(i)) => Ok(*i),
        _ => Err(DeltaRejection::MalformedArgs),
    }
}

pub(crate) fn opt_int(args: &Args, key: &str) -> Option<i64> {
    match crate::arg_reads::get(args, key) {
        Some(ArgVal::Int(n)) => Some(*n),
        _ => None,
    }
}

/// A non-empty lowercase-hex object id (the wire form of a group object id).
/// Rejecting garbage here keeps a fabricated edge from ever landing in the fold.
pub(crate) fn req_hex_id(args: &Args, key: &str) -> Result<String, DeltaRejection> {
    let s = req_text(args, key)?;
    if s.is_empty() || hex::decode(s).is_err() {
        return Err(DeltaRejection::MalformedArgs);
    }
    Ok(s.to_string())
}

pub(crate) fn arg_hex32(args: &Args, k: &str) -> Result<[u8; 32], DeltaRejection> {
    match crate::arg_reads::get(args, k) {
        Some(ArgVal::Text(s)) => hex::decode(s)
            .ok()
            .and_then(|v| <[u8; 32]>::try_from(v).ok())
            .ok_or(DeltaRejection::MalformedArgs),
        _ => Err(DeltaRejection::MalformedArgs),
    }
}

pub(crate) fn arg_b64(args: &Args, k: &str) -> Result<Vec<u8>, DeltaRejection> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    match crate::arg_reads::get(args, k) {
        Some(ArgVal::Text(s)) => B64.decode(s).map_err(|_| DeltaRejection::MalformedArgs),
        _ => Err(DeltaRejection::MalformedArgs),
    }
}

/// The commutative LWW key, carried in args as a non-negative int.
pub(crate) fn req_gen(args: &Args) -> Result<u64, DeltaRejection> {
    let g = req_int(args, "gen")?;
    if g < 0 {
        return Err(DeltaRejection::MalformedArgs);
    }
    Ok(g as u64)
}
