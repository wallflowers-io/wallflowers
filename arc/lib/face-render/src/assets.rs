//! What the face editor offers — fonts, platform icons, stickers — read from the SAME
//! generated file the editor loads (`site/assets/face/vendor/face-assets.js`, written
//! by the website's `scripts/vendor-face.py`), so the published face and the editor's
//! preview cannot disagree about what `font: "fraunces"` or `id: "rose"` means.

use std::collections::HashMap;
use std::sync::OnceLock;

/// The editor's generated asset list, compiled in — a COPY, kept in this crate.
///
/// It is copied rather than included across the workspace because `product/` has to
/// build standalone: the website is a different repository, and a clean clone of this
/// one would not have it. `vendor/SOURCES` pins each copy's digest, checked on every
/// checkout; `vendored_copies_match_the_website` (run by `make check-arc`) holds the
/// website's copies to the same pins, and fails where the website is absent (NC-69).
const FACE_ASSETS_JS: &str = include_str!("../vendor/face-assets.js");

pub struct Assets {
    /// font id → CSS font-family stack.
    pub fonts: HashMap<String, String>,
    /// platform id → Simple Icons path `d`.
    pub icons: HashMap<String, String>,
    /// sticker id → file under `assets/face/vendor/`.
    pub stickers: HashMap<String, String>,
}

/// The JSON object inside `W.FaceAssets = { … };`.
fn json_of(js: &str) -> Result<&str, String> {
    let marker = "W.FaceAssets = ";
    let start = js.find(marker).ok_or("face-assets.js: `W.FaceAssets = ` not found")? + marker.len();
    let end = js.rfind("};").ok_or("face-assets.js: closing `};` not found")?;
    Ok(&js[start..=end])
}

pub fn parse(js: &str) -> Result<Assets, String> {
    let v: serde_json::Value =
        serde_json::from_str(json_of(js)?).map_err(|e| format!("face-assets.js is not JSON: {e}"))?;
    let mut fonts = HashMap::new();
    for f in v["fonts"].as_array().ok_or("face-assets.js: no fonts")? {
        if let (Some(id), Some(stack)) = (f["id"].as_str(), f["stack"].as_str()) {
            fonts.insert(id.to_string(), stack.to_string());
        }
    }
    let mut icons = HashMap::new();
    for (k, d) in v["icons"].as_object().ok_or("face-assets.js: no icons")? {
        if let Some(d) = d.as_str() {
            icons.insert(k.clone(), d.to_string());
        }
    }
    let mut stickers = HashMap::new();
    for s in v["stickers"].as_array().ok_or("face-assets.js: no stickers")? {
        if let (Some(id), Some(file)) = (s["id"].as_str(), s["file"].as_str()) {
            stickers.insert(id.to_string(), file.to_string());
        }
    }
    Ok(Assets { fonts, icons, stickers })
}

pub fn assets() -> &'static Assets {
    static A: OnceLock<Assets> = OnceLock::new();
    // A malformed generated file is a build defect, pinned by a test; at runtime an
    // empty set degrades to the built-in fonts and lettered glyphs.
    A.get_or_init(|| {
        parse(FACE_ASSETS_JS).unwrap_or(Assets {
            fonts: HashMap::new(),
            icons: HashMap::new(),
            stickers: HashMap::new(),
        })
    })
}

// ---- face.js's own definitions — pinned against face.js by `doodles_match_face_js` ----

pub const SANS: &str = "-apple-system, BlinkMacSystemFont, \"Segoe UI\", Roboto, \"Helvetica Neue\", sans-serif";
pub const SERIF: &str = "\"Iowan Old Style\", \"Palatino Linotype\", Palatino, Georgia, serif";

/// face.js `fontOf`: a known id's stack, or the system sans.
pub fn font_stack(id: &str) -> String {
    match id {
        "marker" => format!("\"Wallflowers\", {SANS}"),
        "system-sans" => SANS.to_string(),
        "system-serif" => SERIF.to_string(),
        other => assets().fonts.get(other).cloned().unwrap_or_else(|| SANS.to_string()),
    }
}

/// face.js DOODLES: the ten drawn stickers, `(id, svg inner)`.
pub const DOODLES: &[(&str, &str)] = &[
    ("doodle-loop", r#"<path d="M8 30c18-16 46-14 52 4 5 15-10 25-20 17-9-7 1-23 18-22 19 1 30 18 30 42"/><path d="m75 62 13 12 9-15"/>"#),
    ("doodle-down", r#"<path d="M52 8c-9 26 7 50-2 80"/><path d="m33 70 17 18 18-16"/>"#),
    ("doodle-swoop", r#"<path d="M8 66c20-32 56-38 80-12"/><path d="m70 38 19 16-17 16"/>"#),
    ("doodle-circle", r#"<path d="M80 26C66 10 28 12 14 34 2 54 22 84 54 84c30 0 44-22 36-44-6-16-26-24-48-20"/>"#),
    ("doodle-underline", r#"<path d="M6 58c16-8 30 6 46-2s28-8 42 0"/>"#),
    ("doodle-sparkle", r#"<path d="M50 6c4 28 16 40 44 44-28 4-40 16-44 44-4-28-16-40-44-44 28-4 40-16 44-44z" fill="currentColor" stroke="none"/>"#),
    ("doodle-burst", r#"<path d="M50 8v18m0 48v18M8 50h18m48 0h18M20 20l13 13m34 34 13 13m0-60L67 33M33 67 20 80"/>"#),
    ("doodle-heart", r#"<path d="M50 84C22 64 8 48 14 30c5-15 26-18 36 0 10-18 31-15 36 0 6 18-8 34-36 54z"/>"#),
    ("doodle-zigzag", r#"<path d="m6 58 14-18 14 18 14-18 14 18 14-18 14 18"/>"#),
    ("doodle-smile", r#"<circle cx="50" cy="50" r="40"/><path d="M34 58c8 10 24 10 32 0M38 38v4m24-4v4"/>"#),
];

/// face.js OWN_ICONS: the two platforms Simple Icons has no mark for.
pub const OWN_ICONS: &[(&str, &str)] = &[
    ("website", r#"<circle cx="12" cy="12" r="9" fill="none" stroke="currentColor" stroke-width="1.8"/><path d="M3 12h18M12 3c3 3.2 3 14.8 0 18M12 3c-3 3.2-3 14.8 0 18" fill="none" stroke="currentColor" stroke-width="1.8"/>"#),
    ("email", r#"<rect x="3" y="5" width="18" height="14" rx="2.5" fill="none" stroke="currentColor" stroke-width="1.8"/><path d="m4 7 8 6 8-6" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/>"#),
];

/// face.js PLATFORMS, `(id, name, letters)`: what a glyph says when it has no mark.
pub const PLATFORMS: &[(&str, &str, &str)] = &[
    ("instagram", "Instagram", "I"), ("tiktok", "TikTok", "T"), ("youtube", "YouTube", "Y"), ("x", "X", "X"),
    ("threads", "Threads", "T"), ("bluesky", "Bluesky", "B"), ("mastodon", "Mastodon", "M"), ("facebook", "Facebook", "F"),
    ("linkedin", "LinkedIn", "in"), ("whatsapp", "WhatsApp", "W"), ("telegram", "Telegram", "T"), ("signal", "Signal", "S"),
    ("discord", "Discord", "D"), ("twitch", "Twitch", "T"), ("spotify", "Spotify", "S"), ("soundcloud", "SoundCloud", "S"),
    ("bandcamp", "Bandcamp", "B"), ("substack", "Substack", "S"), ("patreon", "Patreon", "P"), ("meetup", "Meetup", "M"),
    ("github", "GitHub", "G"), ("pinterest", "Pinterest", "P"), ("snapchat", "Snapchat", "S"), ("website", "Website", "W"),
    ("email", "Email", "E"),
];

/// A platform's mark, as face.js `iconSvg` draws it, or `None` for a lettered glyph.
pub fn icon_svg(id: &str) -> Option<String> {
    let inner = match assets().icons.get(id) {
        Some(d) => format!(r#"<path fill="currentColor" d="{}"/>"#, crate::html::esc(d)),
        None => OWN_ICONS.iter().find(|(k, _)| *k == id).map(|(_, s)| s.to_string())?,
    };
    Some(format!(r#"<svg viewBox="0 0 24 24" aria-hidden="true">{inner}</svg>"#))
}

pub fn platform(id: &str) -> (&'static str, &'static str) {
    PLATFORMS
        .iter()
        .find(|p| p.0 == id)
        .map(|p| (p.1, p.2))
        .unwrap_or(("Website", "W"))
}
