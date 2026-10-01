//! A SITE'S FACE, WORN BY THE SIGN-IN WINDOW when that Site's own page signs a visitor in
//! (Ralph, 29 Sep: "the theme, top icon, and name should all come straight from the site's
//! Face"). Read from the Arc's gateway (`/v1/face/<slug>/brand`: public, as the Face is),
//! kept a minute, and served to the window from this origin, the look as a stylesheet and
//! the mark as an image, so the window's CSP stays its own.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// What the window wears: the Face's name, its look as custom properties, and whether it
/// has a mark.
#[derive(Clone, Debug, PartialEq)]
pub struct Brand {
    pub name: String,
    pub style: String,
    pub mark: bool,
}

/// How long a Face is kept: a changed look shows within a minute, and a Face the Arc says
/// it does not have is asked again after as long.
pub const KEPT: Duration = Duration::from_secs(60);

/// How long a read that failed is kept (the Arc slow, silent or erring): seconds, so one
/// failure costs the windows of those seconds their Face, not a minute's (MANAGE, 29 Sep).
pub const KEPT_FAILED: Duration = Duration::from_secs(5);

/// An address as the Arc serves them: lowercase letters, digits and hyphens.
pub fn valid_slug(s: &str) -> bool {
    !s.is_empty() && s.len() <= 63 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The gateway's answer, held to what the window can wear: a name, and a look that sets
/// only the custom properties face-render draws a look with, none able to close the rule,
/// open another, or fetch anything but a picture carried inline.
pub fn brand_of(body: &serde_json::Value) -> Option<Brand> {
    let name = body["name"].as_str()?.trim();
    let style = body["style"].as_str()?;
    if name.is_empty() || !look_ok(style) {
        return None;
    }
    Some(Brand { name: name.to_string(), style: style.to_string(), mark: body["mark"].as_bool().unwrap_or(false) })
}

fn look_ok(style: &str) -> bool {
    if style.len() > 16384 || style.contains("/*") || style.chars().any(|c| c.is_control() || "{}<>\\@".contains(c)) {
        return false;
    }
    // Every declaration sets one of the Face's own properties (ASSURANCE, 2.2.1).
    let Some(decls) = declarations(style) else { return false };
    if !decls.iter().all(|d| d.split_once(':').is_some_and(|(k, _)| face_render::LOOK_PROPERTIES.contains(&k.trim()))) {
        return false;
    }
    // Every url( is an inline picture: face-render's patterns are data:image/svg+xml.
    style
        .match_indices("url(")
        .all(|(i, _)| style[i + 4..].trim_start_matches(['"', '\'']).starts_with("data:image/"))
}

/// A look's declarations, split where CSS splits them: at a `;` outside quotes and
/// brackets. None when a quote or a bracket is left open, or closed before it opened.
fn declarations(style: &str) -> Option<Vec<&str>> {
    let (mut out, mut from, mut depth, mut quote) = (Vec::new(), 0, 0u32, None);
    for (i, c) in style.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth = depth.checked_sub(1)?,
            (None, ';') if depth == 0 => {
                out.push(&style[from..i]);
                from = i + 1;
            }
            _ => {}
        }
    }
    out.push(&style[from..]);
    (quote.is_none() && depth == 0).then_some(out)
}

/// The window's stylesheet for a look: its custom properties on the window's root.
pub fn css(b: &Brand) -> String {
    format!("html[data-face]{{{}}}\n", b.style)
}

/// Faces read, by slug, and until when (None: the Arc had none, or would not answer).
#[derive(Default)]
pub struct Kept(Mutex<HashMap<String, (Instant, Option<Brand>)>>);

impl Kept {
    pub fn get(&self, slug: &str) -> Option<Option<Brand>> {
        self.0.lock().unwrap().get(slug).filter(|(until, _)| Instant::now() < *until).map(|(_, b)| b.clone())
    }

    pub fn put(&self, slug: &str, b: Option<Brand>, kept: Duration) {
        self.0.lock().unwrap().insert(slug.to_string(), (Instant::now() + kept, b));
    }
}

/// Text into HTML, as the window's name and title carry it.
pub fn esc(s: &str) -> String {
    s.chars()
        .map(|ch| match ch {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            '\'' => "&#39;".into(),
            c => c.to_string(),
        })
        .collect()
}

/// A client id into a query string.
pub fn query(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_slug_is_an_address_and_nothing_else() {
        for ok in ["egregore", "mill-road", "a1"] {
            assert!(valid_slug(ok), "{ok}");
        }
        for bad in ["", "Egregore", "../x", "a/b", "a b", "a?b", &"x".repeat(64)] {
            assert!(!valid_slug(bad), "{bad}");
        }
    }

    #[test]
    fn a_look_is_custom_properties_with_inline_pictures_only() {
        let good = "--bg:#FAF8F3;--fg:#14181E;--wallpaper:url(\"data:image/svg+xml,%3Csvg%3E\"), #FAF8F3;--font:\"Wallflowers\",system-ui";
        let b = brand_of(&json!({ "name": "Egregore's Echoes", "style": good, "mark": true })).expect("worn");
        assert_eq!(b, Brand { name: "Egregore's Echoes".into(), style: good.into(), mark: true });
        assert_eq!(css(&b), format!("html[data-face]{{{good}}}\n"));
        for bad in [
            "--bg:red}body{display:none",
            "--bg:url(https://evil.example/x.png)",
            "--bg:url( 'https://evil.example')",
            "--bg:#fff;@import 'x'",
            "--bg:\\75rl(x)",
            "--x:</style><script>",
        ] {
            assert!(brand_of(&json!({ "name": "x", "style": bad })).is_none(), "{bad}");
        }
        assert!(brand_of(&json!({ "name": "  ", "style": "--bg:#fff" })).is_none(), "a Face with no name is not worn");
        assert!(brand_of(&json!({ "style": "--bg:#fff" })).is_none());
    }

    #[test]
    fn a_look_sets_only_the_properties_a_face_has() {
        // Every look face-render draws is worn: each block style, shadow and wallpaper, a
        // pattern's inline picture and the font stacks with them.
        for (style, shadow, wallpaper) in
            [("solid", "soft", "fill"), ("glass", "hard", "pattern"), ("outline", "none", "blur"), ("solid", "strong", "gradient")]
        {
            let bundle = json!({ "v": 1, "profile": { "displayName": "E" }, "face": { "v": 1, "look": {
                "wallpaper": { "style": wallpaper, "pattern": "waves", "angle": "radial" },
                "blocks": { "style": style, "shadow": shadow },
                "text": { "font": "marker", "title": "system-serif" },
                "colours": { "background": "#0A0D08", "buttons": "#B9F227" } } } });
            let look = face_render::look_style(&face_render::parse(&bundle.to_string(), "E").expect("parses"));
            assert!(brand_of(&json!({ "name": "E", "style": look })).is_some(), "{look}");
        }
        // The window's own properties, and any property not custom, are not a Face's to set:
        // with them a look could paint the recovery words out, or other words in.
        for bad in [
            "--white:#000",
            "--bg:#fff;--ink:#fff",
            "color:transparent",
            "--bg:#fff;display:none",
            "--BG:#fff",
            "--bg:\"\n;--ink:#fff",
            "--bg:#fff/*;*/",
            "--bg:rgb(;--ink:#fff",
            "--bg:#fff);--ink:#fff",
            "--bg:\"x",
        ] {
            assert!(brand_of(&json!({ "name": "x", "style": bad })).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn names_and_ids_are_escaped_where_they_go() {
        assert_eq!(esc("A <b>&\"'"), "A &lt;b&gt;&amp;&quot;&#39;");
        assert_eq!(query("egregores-echoes.com"), "egregores-echoes.com");
        assert_eq!(query("a b&c"), "a%20b%26c");
    }

    #[test]
    fn a_face_is_kept_a_minute_and_so_is_its_absence() {
        let k = Kept::default();
        assert_eq!(k.get("egregore"), None, "never read");
        k.put("egregore", None, KEPT);
        assert_eq!(k.get("egregore"), Some(None), "read, and none");
        let b = Brand { name: "E".into(), style: "--bg:#fff".into(), mark: false };
        k.put("egregore", Some(b.clone()), KEPT);
        assert_eq!(k.get("egregore"), Some(Some(b)));
        k.put("egregore", None, Duration::ZERO);
        assert_eq!(k.get("egregore"), None, "kept for as long as it was put for");
    }
}
