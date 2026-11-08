//! face_spec — the face document's one specification, `core/coordination/face.schema.json`,
//! and the check every `site_*` write makes against it before its first delta.
//!
//! The schema is JSON Schema 2020-12, held to the subset its `$comment` names, so this
//! checks it whole: type, properties, required, additionalProperties false, enum, const,
//! minimum, maximum, maxLength, maxItems, items, oneOf by `type` const, and local `$ref`.
//! `x-wf-format` is the one check by value: `hex-colour`, and `font` / `sticker`, whose ids
//! are the schema's `x-wf-builtins` and the editor's generated `face-assets.js`.
//!
//! The fold is unchanged: `group.editFace` still takes any JSON object up to 16 KiB, so a
//! face written before this check still folds. This is what a writer is held to.

use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::OnceLock;

pub const SCHEMA_JSON: &str = include_str!("../../coordination/face.schema.json");
const FACE_ASSETS_JS: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../app/web/webapp/assets/face/vendor/face-assets.js"));

struct Spec {
    schema: Value,
    fonts: BTreeSet<String>,
    stickers: BTreeSet<String>,
}

fn spec() -> &'static Spec {
    static SPEC: OnceLock<Spec> = OnceLock::new();
    SPEC.get_or_init(|| {
        let schema: Value = serde_json::from_str(SCHEMA_JSON).expect("face.schema.json is JSON");
        let assets = assets(FACE_ASSETS_JS).expect("face-assets.js carries its tables");
        let ids = |table: &str, key: &str| -> BTreeSet<String> {
            let built = schema["x-wf-builtins"][table].as_array().into_iter().flatten().filter_map(|r| r[0].as_str());
            let vendored = assets[key].as_array().into_iter().flatten().filter_map(|r| r["id"].as_str());
            built.chain(vendored).map(str::to_string).collect()
        };
        let (fonts, stickers) = (ids("fonts", "fonts"), ids("stickers", "stickers"));
        Spec { schema, fonts, stickers }
    })
}

/// The JSON inside `W.FaceAssets = { … };`, as face-render reads it.
fn assets(js: &str) -> Result<Value, String> {
    let marker = "W.FaceAssets = ";
    let start = js.find(marker).ok_or("no `W.FaceAssets = `")? + marker.len();
    let end = js.rfind("};").ok_or("no closing `};`")?;
    serde_json::from_str(&js[start..=end]).map_err(|e| e.to_string())
}

/// The schema, for an editor to build from (its tables and labels).
pub fn schema() -> &'static Value {
    &spec().schema
}

/// The font and sticker ids a face may name.
pub fn font_ids() -> &'static BTreeSet<String> {
    &spec().fonts
}
pub fn sticker_ids() -> &'static BTreeSet<String> {
    &spec().stickers
}

/// The kinds with a public page: those the ICD makes of a `host` part. A Site, today; the
/// rest is the ICD's to say (#62), and a client offers a page wherever this says.
pub fn kinds_with_a_page() -> &'static [String] {
    static KINDS: OnceLock<Vec<String>> = OnceLock::new();
    KINDS.get_or_init(|| {
        let icd: Value = serde_json::from_str(include_str!("../../coordination/delta-graph.icd.json")).expect("the ICD");
        icd["kinds"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, k)| k["parts"].as_array().into_iter().flatten().any(|p| p["role"] == crate::parts::HOST_ROLE))
            .map(|(name, _)| name.clone())
            .collect()
    })
}

/// Why `face` is not a face this specification allows, or `None`. Size first: the cap is
/// `group.setFace`'s.
pub fn refusal(face: &str) -> Option<String> {
    if face.len() > crate::group::MAX_FACE {
        return Some(format!("the face is over {} bytes", crate::group::MAX_FACE));
    }
    let v: Value = match serde_json::from_str(face) {
        Ok(v) => v,
        Err(e) => return Some(format!("the face is not JSON: {e}")),
    };
    let s = spec();
    check(&s.schema, &s.schema, &v, "face").err()
}

fn check(root: &Value, rule: &Value, v: &Value, at: &str) -> Result<(), String> {
    if let Some(r) = rule["$ref"].as_str() {
        let path = r.strip_prefix("#/$defs/").ok_or_else(|| format!("{at}: the schema's $ref {r} is not local"))?;
        return check(root, &root["$defs"][path], v, at);
    }
    if let Some(c) = rule.get("const") {
        return if v == c { Ok(()) } else { Err(format!("{at} must be {c}")) };
    }
    if let Some(alts) = rule["oneOf"].as_array() {
        // By the `type` const each alternative names, so the refusal says which one failed.
        let t = &v["type"];
        let alt = alts.iter().find(|a| &a["properties"]["type"]["const"] == t);
        return match alt {
            Some(a) => check(root, a, v, at),
            None => Err(format!("{at}: {t} is not a kind of widget")),
        };
    }
    match rule["type"].as_str() {
        Some("object") => {
            let o = v.as_object().ok_or_else(|| format!("{at} must be an object"))?;
            let props = rule["properties"].as_object();
            for r in rule["required"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                if !o.contains_key(r) {
                    return Err(format!("{at}.{r} is missing"));
                }
            }
            for (k, x) in o {
                match props.and_then(|p| p.get(k)) {
                    Some(p) => check(root, p, x, &format!("{at}.{k}"))?,
                    None if rule["additionalProperties"] == Value::Bool(false) => {
                        return Err(format!("{at}.{k} is not part of a face"))
                    }
                    None => {}
                }
            }
        }
        Some("array") => {
            let a = v.as_array().ok_or_else(|| format!("{at} must be a list"))?;
            if let Some(m) = rule["maxItems"].as_u64() {
                if a.len() as u64 > m {
                    return Err(format!("{at} holds at most {m}"));
                }
            }
            if let Some(items) = rule.get("items") {
                for (i, x) in a.iter().enumerate() {
                    check(root, items, x, &format!("{at}[{i}]"))?;
                }
            }
        }
        Some("string") => {
            let s = v.as_str().ok_or_else(|| format!("{at} must be text"))?;
            if let Some(m) = rule["maxLength"].as_u64() {
                if s.chars().count() as u64 > m {
                    return Err(if m == 0 {
                        format!("{at} must be empty: pictures are the Host's")
                    } else {
                        format!("{at} is over {m} characters")
                    });
                }
            }
            match rule["x-wf-format"].as_str() {
                Some("hex-colour") if !is_hex(s) => return Err(format!("{at} is not a #RRGGBB colour")),
                Some("font") if !(s.is_empty() && rule.get("x-wf-empty").is_some()) && !font_ids().contains(s) => {
                    return Err(format!("{at}: no font '{s}'"))
                }
                Some("sticker") if !sticker_ids().contains(s) => return Err(format!("{at}: no sticker '{s}'")),
                _ => {}
            }
        }
        Some("integer") => {
            if !v.is_i64() && !v.is_u64() {
                return Err(format!("{at} must be a whole number"));
            }
            bounds(rule, v, at)?;
        }
        Some("number") => {
            if !v.is_number() {
                return Err(format!("{at} must be a number"));
            }
            bounds(rule, v, at)?;
        }
        Some("boolean") => {
            if !v.is_boolean() {
                return Err(format!("{at} must be true or false"));
            }
        }
        Some(t) => return Err(format!("{at}: the schema names a type core does not check, {t}")),
        None => {}
    }
    if let Some(e) = rule["enum"].as_array() {
        if !e.contains(v) {
            return Err(format!("{at}: {v} is not one of the choices"));
        }
    }
    Ok(())
}

// ---- publishing: the working copy across the seam ----------------------------------

/// What the Host carries for a face (`host.hydrate` key `face`), and what the Arc's
/// renderer reads: `{v, profile, face, location}`. The working copy (this schema) is the
/// editor's; the published face is face.js `faceDoc()`'s shape, so face-render draws the
/// widgets, the door and the listing the editor made. Each widget's links and the
/// location's precision and label leave the face for the profile's urls and the
/// bundle's location, where the renderer reads them.
pub fn bundle(name: &str, shape: &str, purpose: &str, working: &Value) -> Value {
    let widgets: Vec<Value> = working["blocks"]["root"]["children"].as_array().cloned().unwrap_or_default();
    let urls: Vec<Value> = widgets
        .iter()
        .filter(|w| w["type"] == "wf-social")
        .flat_map(|w| w["links"].as_array().cloned().unwrap_or_default())
        .filter_map(|l| {
            let platform = l["platform"].as_str().unwrap_or("website");
            link_url(platform, l["handle"].as_str().unwrap_or("")).map(|u| serde_json::json!({ "label": platform, "value": u }))
        })
        .collect();
    let location = widgets.iter().find(|w| w["type"] == "wf-location").map(|w| {
        serde_json::json!({ "shape": "area", "precision": w["precision"], "label": w["label"], "cells": [] })
    });
    let mut look = working["look"].clone();
    if let Some(wp) = look["wallpaper"].as_object_mut() {
        wp.remove("image");
    }
    let children: Vec<Value> = widgets
        .into_iter()
        .map(|mut w| {
            if let Some(o) = w.as_object_mut() {
                o.insert("version".into(), 1.into());
                match o.get("type").and_then(Value::as_str) {
                    Some("wf-social") => {
                        o.remove("links");
                    }
                    Some("wf-location") => {
                        o.remove("precision");
                        o.remove("label");
                    }
                    _ => {}
                }
            }
            w
        })
        .collect();
    let thousandth = |v: &Value| v.as_f64().map(|f| (f * 1000.0).round() / 1000.0).map(Value::from).unwrap_or(Value::Null);
    let stickers: Vec<Value> = working["stickers"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| {
            let mut s = s.clone();
            for k in ["x", "y", "size"] {
                s[k] = thousandth(&s[k]);
            }
            s
        })
        .collect();
    let door = &working["door"];
    let face = serde_json::json!({
        "v": 1,
        "look": look,
        "header": { "layout": working["header"]["layout"], "title": working["header"]["title"] },
        "show": working["show"],
        "widgets": { "root": { "type": "root", "version": 1, "children": children } },
        "stickers": stickers,
        "notice": working["notice"],
        "door": {
            "doorbell": if door["access"] == "invite" { "" } else { "door" },
            "label": door["label"].as_str().unwrap_or("").trim(),
            "questions": door["questions"],
        },
        "listed": working["sharing"]["listed"],
    });
    let mut b = serde_json::json!({
        "v": 1,
        "profile": { "displayName": name, "shape": shape, "card": { "note": purpose, "urls": urls } },
        "face": face,
    });
    if let Some(l) = location {
        b["location"] = l;
    }
    b
}

/// A social link as a visitor follows it: the platform's prefix (`x-wf-prefix`) and the
/// handle, an address typed whole, or `mailto:`. face.js `linkUrl`.
pub fn link_url(platform: &str, handle: &str) -> Option<String> {
    let h = handle.trim();
    if h.is_empty() {
        return None;
    }
    if platform == "email" {
        return Some(format!("mailto:{h}"));
    }
    let lower = h.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return Some(h.to_string());
    }
    if platform == "mastodon" {
        let bare = h.trim_start_matches('@');
        if let Some((user, server)) = bare.split_once('@') {
            if !user.is_empty() && !server.is_empty() && !server.contains('@') && !h.contains(char::is_whitespace) {
                return Some(format!("https://{server}/@{user}"));
            }
        }
    }
    let prefixes = &schema()["$defs"]["widget"]["oneOf"];
    let prefix = prefixes
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|w| w["properties"]["links"]["items"]["properties"]["platform"]["x-wf-prefix"][platform].as_str())
        .unwrap_or("");
    Some(format!("https://{prefix}{}", h.trim_start_matches('@')))
}

fn bounds(rule: &Value, v: &Value, at: &str) -> Result<(), String> {
    let n = v.as_f64().unwrap_or(f64::NAN);
    if rule["minimum"].as_f64().is_some_and(|m| n < m) || rule["maximum"].as_f64().is_some_and(|m| n > m) {
        return Err(format!("{at}: {v} is out of range"));
    }
    Ok(())
}

fn is_hex(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}
