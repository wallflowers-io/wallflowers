//! The look — face.js's `look()`, `applyLook`, `blockVars` and `wallpaperCss`, drawn as
//! the same CSS custom properties face.css reads. Every value is checked before it is
//! written: colours are `#RRGGBB` or the default, choices are from face.js's closed
//! lists or the default, so nothing from a log reaches a style attribute unchecked.

use crate::assets::font_stack;
use crate::html::{is_hex_colour, luminance, on_colour, rgba, trim_float, uri_component};
use serde_json::Value;

pub const PAPER: &str = "#FAF8F3";
pub const INK: &str = "#14181E";
pub const WHITE: &str = "#FFFFFF";

/// Every custom property a look sets, in the order `style_attr` sets them, and no other:
/// what a page that wears a look it did not draw may let it set.
pub const LOOK_PROPERTIES: [&str; 21] = [
    "--shadow", "--btn-bg", "--btn-fg", "--btn-border", "--card-bg", "--card-border", "--blur", "--bg", "--fg",
    "--title", "--surface", "--accent", "--on-accent", "--muted", "--line", "--wallpaper", "--font", "--name-font",
    "--btn-radius", "--card-radius", "--door-radius",
];

/// face.js CORNERS, in px, by index.
const CORNERS: [u32; 4] = [4, 12, 20, 999];

#[derive(Clone, Debug)]
pub struct Colours {
    pub background: String,
    pub cards: String,
    pub buttons: String,
    pub button_text: String,
    pub text: String,
    pub title: String,
}

#[derive(Clone, Debug)]
pub struct Look {
    pub wallpaper: &'static str, // fill | gradient | blur | pattern | image
    pub colour2: String,
    pub colour3: String,
    pub angle: &'static str,   // down | diagonal | radial
    pub pattern: &'static str, // dots | grid | lines | waves | checks | plus
    pub soften: f64,           // 0–100, the veil over a wallpaper image
    pub style: &'static str,   // solid | glass | outline
    pub corners: usize,        // index into CORNERS
    pub shadow: &'static str,  // none | soft | strong | hard
    pub font: String,
    pub title_font: String,
    pub colours: Colours,
}

fn pick(v: &Value, allowed: &[&'static str], default: &'static str) -> &'static str {
    v.as_str()
        .and_then(|s| allowed.iter().find(|a| **a == s).copied())
        .unwrap_or(default)
}
fn colour(v: &Value, default: &str) -> String {
    match v.as_str() {
        Some(s) if is_hex_colour(s) => s.to_ascii_uppercase(),
        _ => default.to_string(),
    }
}
/// A wallpaper picture in one of two shapes (O-62, Software Security; look.js and the
/// webapp's IMAGE): a plain path on this origin, or an image's own data. It is written
/// into `url("…")` as it stands, so anything else, a quote that closes it included,
/// draws no image.
fn drawable_image(u: &str) -> bool {
    let plain = !u.is_empty() && !u.starts_with("//") && u.bytes().all(|b| b.is_ascii_alphanumeric() || b"._~/-".contains(&b));
    let data = ["png", "jpeg", "gif", "webp"].iter().any(|t| {
        u.strip_prefix("data:image/")
            .and_then(|r| r.strip_prefix(t))
            .and_then(|r| r.strip_prefix(";base64,"))
            .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b)))
    });
    plain || data
}
fn font_id(v: &Value, default: &str) -> String {
    // Only an id the editor could have written: built in, or in the vendored list.
    match v.as_str() {
        Some(id) if matches!(id, "marker" | "system-sans" | "system-serif") => id.to_string(),
        Some(id) if crate::assets::assets().fonts.contains_key(id) => id.to_string(),
        _ => default.to_string(),
    }
}

impl Look {
    /// face.js PRESETS[0], "Paper": what a face with no look (or a broken one) wears.
    pub fn paper() -> Look {
        Look::from_value(&Value::Null)
    }

    pub fn from_value(lk: &Value) -> Look {
        let c = &lk["colours"];
        let background = colour(&c["background"], PAPER);
        let buttons = colour(&c["buttons"], "#546CAC");
        let text = colour(&c["text"], INK);
        let colours = Colours {
            cards: colour(&c["cards"], WHITE),
            button_text: colour(&c["buttonText"], on_colour(&buttons)),
            title: colour(&c["title"], &text),
            background: background.clone(),
            buttons,
            text,
        };
        let wp = &lk["wallpaper"];
        let colour2 = colour(&wp["colour2"], &background);
        let colour3 = colour(&wp["colour3"], &colour2);
        let b = &lk["blocks"];
        let font = font_id(&lk["text"]["font"], "system-sans");
        let title_font = match lk["text"]["title"].as_str() {
            Some("") | None if lk.is_null() => "marker".to_string(),
            Some("") | None => font.clone(),
            Some(_) => font_id(&lk["text"]["title"], &font),
        };
        Look {
            wallpaper: pick(&wp["style"], &["fill", "gradient", "blur", "pattern", "image"], "fill"),
            colour2,
            colour3,
            angle: pick(&wp["angle"], &["down", "diagonal", "radial"], "down"),
            pattern: pick(&wp["pattern"], &["dots", "grid", "lines", "waves", "checks", "plus"], "dots"),
            soften: wp["soften"].as_f64().map(|s| s.clamp(0.0, 100.0)).unwrap_or(30.0),
            style: pick(&b["style"], &["solid", "glass", "outline"], "solid"),
            corners: b["corners"].as_u64().map(|i| (i as usize).min(3)).unwrap_or(3),
            shadow: pick(&b["shadow"], &["none", "soft", "strong", "hard"], "soft"),
            font,
            title_font,
            colours,
        }
    }

    pub fn is_dark(&self) -> bool {
        luminance(&self.colours.background) < 0.3
    }

    /// face.js `wallpaperCss`. `image_url` is the Host's wallpaper picture, if it has one.
    fn wallpaper_css(&self, image_url: Option<&str>) -> String {
        let bg = &self.colours.background;
        let (c2, c3) = (&self.colour2, &self.colour3);
        match self.wallpaper {
            "gradient" if self.angle == "radial" => format!("radial-gradient(130% 80% at 50% 0%, {c2}, {bg})"),
            "gradient" => format!(
                "linear-gradient({}, {bg}, {c2})",
                if self.angle == "diagonal" { "155deg" } else { "180deg" }
            ),
            "blur" => format!(
                "radial-gradient(60% 42% at 12% 10%, {c2}, transparent 72%), radial-gradient(55% 40% at 92% 44%, {c3}, transparent 72%), radial-gradient(70% 46% at 30% 96%, {c2}, transparent 72%), {bg}"
            ),
            "pattern" => format!("{}, {bg}", pattern_url(self.pattern, &self.colours.text)),
            "image" => match image_url.filter(|u| drawable_image(u)) {
                Some(u) => {
                    let veil = rgba(bg, self.soften / 100.0);
                    format!("linear-gradient({veil}, {veil}), url(\"{u}\") center / cover no-repeat, {bg}")
                }
                None => bg.clone(),
            },
            _ => bg.clone(),
        }
    }

    /// face.js `blockVars`: how cards and buttons are filled, edged and lifted.
    fn block_vars(&self, c: &Colours, out: &mut Vec<(&'static str, String)>) {
        let edge = if self.shadow == "hard" { format!("2px solid {}", c.text) } else { String::new() };
        let or = |e: &str, d: String| if e.is_empty() { d } else { e.to_string() };
        out.push((
            "--shadow",
            match self.shadow {
                "soft" => "0 1px 2px rgba(20,24,30,.06), 0 6px 18px rgba(20,24,30,.08)".into(),
                "strong" => "0 16px 30px -12px rgba(0,0,0,.5)".into(),
                "hard" => format!("4px 4px 0 {}", c.text),
                _ => "none".into(),
            },
        ));
        match self.style {
            "glass" => {
                out.push(("--btn-bg", rgba(&c.buttons, 0.26)));
                out.push(("--btn-fg", c.button_text.clone()));
                out.push(("--btn-border", or(&edge, format!("1px solid {}", rgba(WHITE, 0.45)))));
                out.push(("--card-bg", rgba(&c.cards, 0.18)));
                out.push(("--card-border", or(&edge, format!("1px solid {}", rgba(WHITE, 0.3)))));
                out.push(("--blur", "blur(18px) saturate(1.4)".into()));
            }
            "outline" => {
                out.push(("--btn-bg", "transparent".into()));
                out.push(("--btn-fg", c.buttons.clone()));
                out.push(("--btn-border", format!("2px solid {}", c.buttons)));
                out.push(("--card-bg", "transparent".into()));
                out.push(("--card-border", or(&edge, format!("1.5px solid {}", rgba(&c.text, 0.45)))));
                out.push(("--blur", "none".into()));
            }
            _ => {
                out.push(("--btn-bg", c.buttons.clone()));
                out.push(("--btn-fg", c.button_text.clone()));
                out.push(("--btn-border", or(&edge, "0 solid transparent".into())));
                out.push(("--card-bg", c.cards.clone()));
                out.push(("--card-border", or(&edge, "0 solid transparent".into())));
                out.push(("--blur", "none".into()));
            }
        }
    }

    /// face.js `applyLook`, as a `style` attribute value.
    pub fn style_attr(&self, wallpaper_url: Option<&str>) -> String {
        let c = &self.colours;
        let r = CORNERS[self.corners];
        let mut v: Vec<(&'static str, String)> = Vec::new();
        self.block_vars(c, &mut v);
        v.push(("--bg", c.background.clone()));
        v.push(("--fg", c.text.clone()));
        v.push(("--title", c.title.clone()));
        v.push(("--surface", c.cards.clone()));
        v.push(("--accent", c.buttons.clone()));
        v.push(("--on-accent", c.button_text.clone()));
        v.push(("--muted", rgba(&c.text, 0.64)));
        v.push(("--line", rgba(&c.text, 0.14)));
        v.push(("--wallpaper", self.wallpaper_css(wallpaper_url)));
        v.push(("--font", font_stack(&self.font)));
        v.push(("--name-font", font_stack(&self.title_font)));
        v.push(("--btn-radius", format!("{r}px")));
        v.push(("--card-radius", format!("{}px", r.min(24))));
        v.push(("--door-radius", format!("{r}px")));
        css_decls(&v)
    }

    /// face.js `paint`: a widget's own colours, laid over the look after it.
    pub fn widget_style(&self, own: &Value) -> Option<String> {
        let bg = own["bg"].as_str().filter(|s| is_hex_colour(s));
        let fg = own["fg"].as_str().filter(|s| is_hex_colour(s));
        if bg.is_none() && fg.is_none() {
            return None;
        }
        let mut c = self.colours.clone();
        if let Some(b) = bg {
            c.cards = b.to_string();
            c.buttons = b.to_string();
        }
        if let Some(f) = fg {
            c.button_text = f.to_string();
        }
        let mut all = Vec::new();
        self.block_vars(&c, &mut all);
        let mut v: Vec<(&'static str, String)> = all
            .into_iter()
            .filter(|(k, _)| matches!(*k, "--btn-bg" | "--btn-fg" | "--btn-border" | "--card-bg" | "--card-border"))
            .collect();
        if let Some(f) = fg {
            v.push(("--fg", f.to_string()));
            v.push(("--btn-fg", f.to_string()));
            v.push(("--muted", rgba(f, 0.64)));
            v.push(("--title", f.to_string()));
        }
        Some(css_decls(&v))
    }
}

/// face.js PATTERNS, drawn as the same tiny SVG tile.
fn pattern_url(id: &str, text: &str) -> String {
    let (w, h, alpha, body) = match id {
        "grid" => (24, 24, 0.16, "<path d=\"M24 .5H.5V24\" fill=\"none\" stroke=\"{c}\"/>"),
        "lines" => (14, 14, 0.14, "<path d=\"M-2 2 2-2M0 14 14 0M12 16l4-4\" stroke=\"{c}\" stroke-width=\"1.3\"/>"),
        "waves" => (40, 14, 0.2, "<path d=\"M0 7q10-7 20 0t20 0\" fill=\"none\" stroke=\"{c}\" stroke-width=\"1.4\"/>"),
        "checks" => (28, 28, 0.07, "<path d=\"M0 0h14v14H0zM14 14h14v14H14z\" fill=\"{c}\"/>"),
        "plus" => (26, 26, 0.22, "<path d=\"M13 8v10M8 13h10\" stroke=\"{c}\" stroke-width=\"1.6\" stroke-linecap=\"round\"/>"),
        _ => (18, 18, 0.22, "<circle cx=\"9\" cy=\"9\" r=\"1.7\" fill=\"{c}\"/>"),
    };
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\">{}</svg>",
        body.replace("{c}", &rgba(text, alpha))
    );
    format!("url(\"data:image/svg+xml,{}\")", uri_component(&svg))
}

/// Declarations as one `style` attribute value. Values here are built only from
/// checked colours, closed choices, numbers and our own URLs; `"` never occurs in them
/// except inside `url("…")`, so the attribute is written with single quotes by the caller.
fn css_decls(v: &[(&'static str, String)]) -> String {
    v.iter().map(|(k, val)| format!("{k}:{val}")).collect::<Vec<_>>().join(";")
}

pub fn px(v: f64) -> String {
    trim_float(v)
}
