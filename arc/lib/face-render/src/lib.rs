//! A site's PUBLIC FACE, drawn as HTML from what its Host published — the page an Arc
//! serves at `wallflowers.io/<slug>`.
//!
//! The editor that makes a face is `business/website/site/assets/face/face.js` (/signup,
//! step 2), and face.css there draws both its phone preview and THIS page: the markup
//! below uses face.js's own `.wf-*` classes and sets the look as the same custom
//! properties its `applyLook` sets, so linking face.css makes the page look like the
//! preview. Where face.js needs script (the share sheet, the knock sheet) this page does
//! without, and says so where it happens.
//!
//! **No script, ever.** Every string from the bundle is escaped; links are `https:`,
//! `http:` or `mailto:` only; colours are `#RRGGBB` or the default; fonts, stickers and
//! icons are only ones the editor offers; pictures are only the Host's own, by slot. The
//! page loads nothing from another origin: it is served on wallflowers.io, beside the
//! stylesheets it links.

mod assets;
mod html;
mod look;
pub mod markdown;
#[cfg(test)]
mod tests;

pub use html::esc;
pub use look::LOOK_PROPERTIES;
use html::safe_href;
use look::Look;
use serde_json::Value;

/// Where the page is served from. Open Graph needs absolute URLs.
pub const PUBLIC_ORIGIN: &str = "https://wallflowers.io";

/// The published bundle, parsed: `{v, profile, face, location}` as the owner's device
/// wrote it onto the Host (`system.hydrate` key `face`).
#[derive(Clone, Debug)]
pub struct Face {
    pub name: String,
    pub purpose: String,
    /// `profile.card.urls` — `(platform id, url)`, only links a visitor may follow.
    pub urls: Vec<(String, String)>,
    pub face: Value,
    pub location_label: String,
    pub precision: u32,
    look: Look,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RenderError {
    NotJson(String),
    NotAFace(&'static str),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotJson(e) => write!(f, "the face bundle is not JSON: {e}"),
            Self::NotAFace(why) => write!(f, "the face bundle is not a face: {why}"),
        }
    }
}

/// Parse a published bundle. `name` is the Host's own name, used when the profile has
/// none — a Host with an address and no face yet still has something to say.
pub fn parse(bundle_json: &str, host_name: &str) -> Result<Face, RenderError> {
    let v: Value = if bundle_json.trim().is_empty() {
        serde_json::json!({ "v": 1 })
    } else {
        serde_json::from_str(bundle_json).map_err(|e| RenderError::NotJson(e.to_string()))?
    };
    if !v.is_object() {
        return Err(RenderError::NotAFace("not a JSON object"));
    }
    match v["v"].as_u64() {
        Some(1) | None => {}
        Some(_) => return Err(RenderError::NotAFace("a bundle version this Arc does not read")),
    }
    let profile = &v["profile"];
    let card = match &profile["card"] {
        Value::String(s) => serde_json::from_str::<Value>(s).unwrap_or(Value::Null),
        other => other.clone(),
    };
    let name = profile["displayName"].as_str().filter(|s| !s.trim().is_empty()).unwrap_or(host_name).trim().to_string();
    let purpose = card["note"].as_str().unwrap_or("").trim().to_string();
    let urls = card["urls"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|u| {
                    let label = u["label"].as_str().unwrap_or("website").to_string();
                    safe_href(u["value"].as_str()?).map(|h| (label, h))
                })
                .collect()
        })
        .unwrap_or_default();
    let face = match &v["face"] {
        Value::String(s) => serde_json::from_str(s).unwrap_or(Value::Null),
        other => other.clone(),
    };
    let loc = &v["location"];
    let loc = match loc {
        Value::String(s) => serde_json::from_str(s).unwrap_or(Value::Null),
        other => other.clone(),
    };
    let look = if face["look"].is_object() { Look::from_value(&face["look"]) } else { Look::paper() };
    Ok(Face {
        name,
        purpose,
        urls,
        location_label: loc["label"].as_str().unwrap_or("").trim().to_string(),
        precision: loc["precision"].as_u64().map(|p| p.clamp(3, 9) as u32).unwrap_or(5),
        face,
        look,
    })
}

/// What the Arc knows besides the bundle.
pub struct Ctx<'a> {
    pub slug: &'a str,
    /// The picture slots the Host carries: `mark`, `cover`, `logo`, `wallpaper`.
    pub media: &'a [&'a str],
    /// Live items, `(key, payload JSON)`, keys `event:<id>` / `post:<id>`.
    pub items: &'a [(String, String)],
    /// The member count, if the Host published one. The Arc cannot count a roster it
    /// is not on, so there is no count unless the owner put one there.
    pub members: Option<u64>,
    /// The request came from an in-app browser (see [`is_in_app_browser`]): the door
    /// leads to the escape page instead of in.
    pub in_app: bool,
    /// Now, in unix ms — which events are still to come.
    pub now_ms: i64,
}

impl Ctx<'_> {
    fn has(&self, slot: &str) -> bool {
        self.media.contains(&slot)
    }
    fn media_url(&self, slot: &str) -> String {
        format!("/{}/m/{}", self.slug, slot)
    }
}

// ---- the page ----------------------------------------------------------------------

/// Rules face.css does not have, because the editor never needed them: the page
/// frame, a filled feed row (the editor only draws skeletons), and the content notice
/// working without script. Worth moving into face.css when the editor draws real rows.
const PAGE_CSS: &str = "
html,body{margin:0;min-height:100%;background:#FAF8F3}
.wf-look{position:relative;min-height:100vh;min-height:100svh;background:var(--bg)}
.wf-look>.wf-face{max-width:480px;margin:0 auto}
.wf-grid>.wf-cell{min-width:0}
.wf-row{display:grid;gap:3px;text-align:left}
.wf-row-lead{font-size:12px;letter-spacing:.04em;color:var(--muted)}
.wf-row-main{font:600 15px/1.3 var(--font);color:var(--fg);overflow-wrap:anywhere}
.wf-row-meta{font-size:13px;color:var(--muted);overflow-wrap:anywhere}
a.wf-row-main{text-decoration:none}
.wf-size-s .wf-row-main{font-size:14px}
.wf-empty-note{margin:0;font-size:14px;color:var(--muted)}
.wf-social a{text-decoration:none}
#wf-go{position:absolute;opacity:0;pointer-events:none}
#wf-go:checked~.wf-notice{display:none}
a.wf-notice-back{display:inline-block;text-decoration:none}
label.wf-notice-go{cursor:pointer}
a.wf-door{text-decoration:none}
.wf-foot{padding:0 16px 96px;text-align:center;font:12px/1.4 -apple-system,BlinkMacSystemFont,sans-serif;color:var(--muted)}
.wf-foot a{color:inherit}
";

/// The whole document for one face.
pub fn render_face(f: &Face, ctx: &Ctx) -> String {
    let face = &f.face;
    let listed = face["listed"].as_bool().unwrap_or(true);
    let wallpaper = if ctx.has("wallpaper") { Some(ctx.media_url("wallpaper")) } else { None };
    let dark = if f.look.is_dark() { " wf-dark" } else { "" };
    let mut out = String::with_capacity(16 * 1024);

    // head
    out.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1, viewport-fit=cover\">\n");
    out.push_str(&format!("<title>{}</title>\n", esc(&f.name)));
    if !f.purpose.is_empty() {
        out.push_str(&format!("<meta name=\"description\" content=\"{}\">\n", esc(&f.purpose)));
    }
    if !listed {
        out.push_str("<meta name=\"robots\" content=\"noindex\">\n");
    }
    let url = format!("{PUBLIC_ORIGIN}/{}", ctx.slug);
    out.push_str(&format!("<meta property=\"og:type\" content=\"website\">\n<meta property=\"og:url\" content=\"{}\">\n", esc(&url)));
    out.push_str(&format!("<meta property=\"og:title\" content=\"{}\">\n", esc(&f.name)));
    if !f.purpose.is_empty() {
        out.push_str(&format!("<meta property=\"og:description\" content=\"{}\">\n", esc(&f.purpose)));
    }
    if let Some(slot) = ["cover", "mark"].into_iter().find(|s| ctx.has(s)) {
        out.push_str(&format!("<meta property=\"og:image\" content=\"{}{}\">\n", PUBLIC_ORIGIN, esc(&ctx.media_url(slot))));
    }
    let icon = if ctx.has("mark") { ctx.media_url("mark") } else { "/brand/wallflowers-icon.svg".into() };
    out.push_str(&format!("<link rel=\"icon\" href=\"{}\">\n", esc(&icon)));
    // /brand/wallflowers.css declares the marker face ("Wallflowers"); the vendored
    // sheet declares the rest. Both are WallFlowers' own, on this origin.
    out.push_str("<link rel=\"stylesheet\" href=\"/brand/wallflowers.css?v=2\">\n<link rel=\"stylesheet\" href=\"/assets/face/vendor/fonts.css\">\n<link rel=\"stylesheet\" href=\"/assets/face/face.css\">\n");
    out.push_str("<style>");
    out.push_str(PAGE_CSS);
    out.push_str("</style>\n</head>\n<body>\n");

    // the face
    out.push_str(&format!(
        "<div class=\"wf-look{dark}\" style=\"{}\">\n<div class=\"wf-backdrop\"></div>\n",
        esc(&f.look.style_attr(wallpaper.as_deref()))
    ));
    notice(&mut out, face, ctx);
    out.push_str("<main class=\"wf-face\"><div class=\"wf-canvas\">\n");
    profile(&mut out, f, ctx);
    out.push_str("<div class=\"wf-grid\">\n");
    for w in face["widgets"]["root"]["children"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        widget(&mut out, f, w, ctx);
    }
    out.push_str("</div>\n");
    stickers(&mut out, face);
    out.push_str("</div></main>\n");
    door(&mut out, face, ctx);
    foot(&mut out);
    out.push_str("</div>\n</body>\n</html>\n");
    out
}

/// The Face's footer, the one every page a Face serves ends with.
fn foot(out: &mut String) {
    out.push_str(&format!(
        "<p class=\"wf-foot\">On <a href=\"{PUBLIC_ORIGIN}\">WallFlowers</a>, the private social network.</p>\n"
    ));
}

fn initials(name: &str) -> String {
    let s: String = name
        .split_whitespace()
        .take(2)
        .filter_map(|w| w.chars().next())
        .flat_map(|c| c.to_uppercase())
        .collect();
    if s.is_empty() { "·".into() } else { s }
}

fn avatar(out: &mut String, cls: &str, f: &Face, ctx: &Ctx) {
    if ctx.has("mark") {
        out.push_str(&format!("<img class=\"{cls}\" src=\"{}\" alt=\"\">", esc(&ctx.media_url("mark"))));
    } else {
        out.push_str(&format!("<div class=\"{cls} wf-initials\">{}</div>", esc(&initials(&f.name))));
    }
}

/// face.js `drawProfile`. The share button needs script (`navigator.share`), so the
/// published face leaves it to the browser's own share; everything else is drawn.
fn profile(out: &mut String, f: &Face, ctx: &Ctx) {
    let face = &f.face;
    let layout = match face["header"]["layout"].as_str() {
        Some(l @ ("classic" | "hero" | "banner")) => l,
        _ => "classic",
    };
    let show = |k: &str| face["show"][k].as_bool().unwrap_or(true);
    out.push_str(&format!("<header class=\"wf-profile wf-layout-{layout}\">"));
    if layout == "hero" {
        out.push_str(&format!("<div class=\"wf-hero{}\">", if ctx.has("mark") { "" } else { " wf-hero-empty" }));
        avatar(out, "wf-hero-img", f, ctx);
        out.push_str("</div>");
    } else {
        if layout == "banner" {
            if ctx.has("cover") {
                out.push_str(&format!(
                    "<div class=\"wf-banner\" style=\"background-image:url(&quot;{}&quot;)\"></div>",
                    esc(&ctx.media_url("cover"))
                ));
            } else {
                out.push_str("<div class=\"wf-banner\"></div>");
            }
        }
        avatar(out, "wf-avatar", f, ctx);
    }
    if face["header"]["title"].as_str() == Some("logo") && ctx.has("logo") {
        out.push_str(&format!(
            "<img class=\"wf-logo\" src=\"{}\" alt=\"{}\">",
            esc(&ctx.media_url("logo")),
            esc(&f.name)
        ));
    } else {
        out.push_str(&format!("<h1 class=\"wf-name\">{}</h1>", esc(&f.name)));
    }
    if show("address") {
        out.push_str(&format!("<p class=\"wf-handle\">wallflowers.io/{}</p>", esc(ctx.slug)));
    }
    if show("members") {
        if let Some(n) = ctx.members {
            out.push_str(&format!("<p class=\"wf-members\"><b>{n}</b> member{}</p>", if n == 1 { "" } else { "s" }));
        }
    }
    if show("purpose") && !f.purpose.is_empty() {
        out.push_str(&format!("<p class=\"wf-bio\">{}</p>", esc(&f.purpose)));
    }
    out.push_str("</header>\n");
}

fn size_of(w: &Value) -> &'static str {
    match w["size"].as_str() {
        Some("s") => "s",
        Some("l") => "l",
        _ => "m",
    }
}

/// face.js `shapeOf`, filled. A widget with nothing to show on a published face — a
/// link with no address, a text with no words, a feed told to hide when empty — is
/// left out rather than drawn as the editor's placeholder.
fn widget(out: &mut String, f: &Face, w: &Value, ctx: &Ctx) {
    let size = size_of(w);
    let style = f
        .look
        .widget_style(&w["colours"])
        .map(|s| format!(" style=\"{}\"", esc(&s)))
        .unwrap_or_default();
    let body = match w["type"].as_str() {
        Some("wf-link") => {
            let Some(href) = w["url"].as_str().and_then(safe_href) else { return };
            let title = w["title"].as_str().filter(|t| !t.trim().is_empty()).unwrap_or("Link");
            format!(
                "<a class=\"wf-w wf-link\" href=\"{}\" rel=\"noopener noreferrer\"{style}>{}</a>",
                esc(&href),
                esc(title)
            )
        }
        Some("wf-text") => {
            let text = w["text"].as_str().unwrap_or("");
            if text.trim().is_empty() {
                return;
            }
            format!("<div class=\"wf-w wf-textcard\"{style}><p>{}</p></div>", esc(text))
        }
        Some("wf-social") => {
            if f.urls.is_empty() {
                return;
            }
            let mut s = format!("<div class=\"wf-w wf-social\"{style}>");
            for (platform, href) in &f.urls {
                let (name, letters) = assets::platform(platform);
                let glyph = assets::icon_svg(platform).unwrap_or_else(|| esc(letters));
                s.push_str(&format!(
                    "<a class=\"wf-glyph\" href=\"{}\" rel=\"noopener noreferrer me\" aria-label=\"{}\">{glyph}</a>",
                    esc(href),
                    esc(name)
                ));
            }
            s.push_str("</div>");
            s
        }
        Some("wf-feed") => match feed(w, ctx, size) {
            Some(inner) => format!("<div class=\"wf-w wf-feed\"{style}>{inner}</div>"),
            None => return,
        },
        Some("wf-location") => {
            let heading = w["heading"].as_str().unwrap_or("Where we are");
            let label = if f.location_label.is_empty() { "Nearby" } else { &f.location_label };
            format!(
                "<div class=\"wf-w wf-location\"{style}><div class=\"wf-head\">{}</div><div class=\"wf-place\"><span class=\"wf-pin\"></span><div class=\"wf-place-words\"><span class=\"wf-place-label\">{}</span><span class=\"wf-place-scale\">Within about {}</span></div></div></div>",
                esc(heading),
                esc(label),
                cell_size(f.precision)
            )
        }
        _ => return, // a widget type this renderer does not know is left out, not guessed at
    };
    out.push_str(&format!("<div class=\"wf-cell wf-size-{size}\">{body}</div>\n"));
}

/// face.js `cellSize`: a geohash cell's larger side at the equator.
fn cell_size(n: u32) -> String {
    let n = n as f64;
    let lat = 180.0 / 2f64.powf((5.0 * n / 2.0).floor());
    let lon = 360.0 / 2f64.powf((5.0 * n / 2.0).ceil());
    let km = lat.max(lon) * 111.32;
    if km >= 10.0 {
        format!("{} km", km.round())
    } else if km >= 1.0 {
        format!("{:.1} km", km)
    } else {
        format!("{} m", (km * 1000.0).round())
    }
}

/// The item keys a Face draws: ICD-0 `host.hydrate` carries `event:<id>` and `post:<id>`
/// for it, and nothing else.
pub const ITEM_PREFIXES: [&str; 2] = ["event:", "post:"];

/// Is `key` the item `prefix` names: the prefix and one id, with no second ':' segment. A
/// post's page (`post:<id>:body:<n>`, `:asset:<a>`, `:document`; ICD 2.3.1) rides the Host
/// beside its row and is never one.
fn is_item_of(key: &str, prefix: &str) -> bool {
    key.strip_prefix(prefix).is_some_and(|id| !id.is_empty() && !id.contains(':'))
}

/// Is `key` an item a Face draws?
pub fn is_item_key(key: &str) -> bool {
    ITEM_PREFIXES.iter().any(|p| is_item_of(key, p))
}

/// The live items a source draws, by key prefix. Any other source has no items on the
/// Host, so it draws no rows — never another kind's.
fn prefix_of(source: &str) -> Option<&'static str> {
    match source {
        "publications" => Some(ITEM_PREFIXES[1]),
        "events" => Some(ITEM_PREFIXES[0]),
        _ => None,
    }
}

/// A feed: the Host's live items of one source, as rows. `None` when there are none
/// and the widget says to hide.
fn feed(w: &Value, ctx: &Ctx, size: &str) -> Option<String> {
    let source = w["source"].as_str().unwrap_or("events");
    let prefix = prefix_of(source);
    let fields: Vec<&str> = w["fields"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
    let mut rows: Vec<Value> = ctx
        .items
        .iter()
        .filter(|(k, _)| prefix.is_some_and(|p| is_item_of(k, p)))
        .filter_map(|(k, p)| {
            let mut v = serde_json::from_str::<Value>(p).ok()?;
            // A post's own page (render_post), where its key names a post.
            if let Some(id) = k.strip_prefix(ITEM_PREFIXES[1]).filter(|id| is_object_id(id)) {
                v["_page"] = Value::from(format!("/{}/post/{id}", ctx.slug));
            }
            Some(v)
        })
        .filter(|v| v["title"].as_str().is_some_and(|t| !t.trim().is_empty()))
        .collect();
    if source == "publications" {
        // Newest first, by when the Site named it (`at`, core's face_items): a key is
        // `post:<object id>`, random hex, so key order is no order at all.
        rows.sort_by_key(|v| std::cmp::Reverse(v["at"].as_i64().unwrap_or(i64::MIN)));
    } else {
        rows.retain(|v| v["startMs"].as_i64().is_some_and(|t| t >= ctx.now_ms - 3 * 3600 * 1000));
        rows.sort_by_key(|v| v["startMs"].as_i64().unwrap_or(i64::MAX));
    }
    let limit = if w["mode"].as_str() == Some("next") { 1 } else if size == "l" { 4 } else { 2 };
    rows.truncate(limit);
    let heading = w["heading"].as_str().unwrap_or("");
    let mut s = String::new();
    if !heading.is_empty() {
        s.push_str(&format!("<div class=\"wf-head\">{}</div>", esc(heading)));
    }
    if rows.is_empty() {
        if w["empty"].as_str() == Some("note") {
            let note = w["emptyNote"].as_str().filter(|n| !n.trim().is_empty()).unwrap_or("Nothing yet");
            s.push_str(&format!("<p class=\"wf-empty-note\">{}</p>", esc(note)));
            return Some(s);
        }
        return None;
    }
    s.push_str("<div class=\"wf-items\">");
    for v in rows {
        s.push_str("<div class=\"wf-row\">");
        if source == "events" {
            if let Some(t) = v["startMs"].as_i64() {
                s.push_str(&format!("<time class=\"wf-row-lead\" datetime=\"{}\">{}</time>", iso_utc(t), esc(&when(t))));
            }
        }
        let title = v["title"].as_str().unwrap_or("");
        let link = if fields.contains(&"link") { v["link"].as_str().and_then(safe_href) } else { None };
        match (link, v["_page"].as_str()) {
            (Some(href), _) => s.push_str(&format!(
                "<a class=\"wf-row-main\" href=\"{}\" rel=\"noopener noreferrer\">{}</a>",
                esc(&href),
                esc(title)
            )),
            (None, Some(page)) => s.push_str(&format!("<a class=\"wf-row-main\" href=\"{}\">{}</a>", esc(page), esc(title))),
            (None, None) => s.push_str(&format!("<span class=\"wf-row-main\">{}</span>", esc(title))),
        }
        for k in &fields {
            if matches!(*k, "title" | "startMs" | "link") {
                continue;
            }
            if let Some(val) = v[*k].as_str().filter(|x| !x.trim().is_empty()) {
                let val = if *k == "body" { excerpt(val, 140) } else { val.to_string() };
                s.push_str(&format!("<span class=\"wf-row-meta\">{}</span>", esc(&val)));
            }
        }
        s.push_str("</div>");
    }
    s.push_str("</div>");
    Some(s)
}

fn excerpt(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if s.chars().count() > n { format!("{}…", t.trim_end()) } else { t }
}

/// Civil date from unix days (Howard Hinnant's algorithm), so no clock crate is needed.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn iso_utc(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (y, m, d) = civil(secs.div_euclid(86_400));
    let t = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}Z", t / 3600, (t % 3600) / 60)
}

/// "Sat 3 Oct · 19:00 UTC". The event's own time zone is not in its payload, so the
/// time says which one it is in rather than pretending to be local.
fn when(ms: i64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let (_, m, d) = civil(days);
    let t = secs.rem_euclid(86_400);
    format!(
        "{} {d} {} · {:02}:{:02} UTC",
        DAYS[days.rem_euclid(7) as usize],
        MONTHS[(m - 1) as usize],
        t / 3600,
        (t % 3600) / 60
    )
}

/// face.js `stickerEl`: doodles drawn in one of the face's inks, the rest as the
/// vendored pictures. Placed in the face's own width, as the editor placed them.
fn stickers(out: &mut String, face: &Value) {
    let list = face["stickers"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    if list.is_empty() {
        return;
    }
    out.push_str("<div class=\"wf-stickers\" aria-hidden=\"true\">");
    for s in list.iter().take(40) {
        let Some(id) = s["id"].as_str() else { continue };
        let x = s["x"].as_f64().unwrap_or(0.5).clamp(-0.5, 1.5);
        let y = s["y"].as_f64().unwrap_or(0.5).clamp(-0.5, 50.0);
        let size = s["size"].as_f64().unwrap_or(0.2).clamp(0.02, 1.0);
        let rot = s["rot"].as_f64().unwrap_or(0.0).clamp(-180.0, 180.0).round();
        let flip = if s["flip"].as_bool().unwrap_or(false) { " scaleX(-1)" } else { "" };
        let pose = format!(
            "left:{}%;top:calc({} * 100cqw);width:{}%;transform:translate(-50%, -50%) rotate({}deg){flip}",
            look::px(x * 100.0),
            look::px(y),
            look::px(size * 100.0),
            look::px(rot)
        );
        if let Some((_, svg)) = assets::DOODLES.iter().find(|(k, _)| *k == id) {
            let ink = match s["colour"].as_str() {
                Some("text") => "var(--fg)",
                Some("title") => "var(--title)",
                _ => "var(--accent)",
            };
            out.push_str(&format!(
                "<div class=\"wf-sticker\" style=\"{};color:{ink}\"><svg viewBox=\"0 0 100 100\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"6\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\">{svg}</svg></div>",
                esc(&pose)
            ));
        } else if let Some(file) = assets::assets().stickers.get(id) {
            out.push_str(&format!(
                "<div class=\"wf-sticker\" style=\"{}\"><img src=\"/assets/face/vendor/{}\" alt=\"\"></div>",
                esc(&pose),
                esc(file)
            ));
        }
    }
    out.push_str("</div>");
}

const DOOR_SVG: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 21V4.5A1.5 1.5 0 0 1 7.5 3h9A1.5 1.5 0 0 1 18 4.5V21"/><path d="M3.5 21h17"/><circle cx="14.6" cy="12.2" r="1" fill="currentColor" stroke="none"/></svg>"#;

/// face.js `doorEl`. Every face has a door. Inside an in-app browser it leads to the
/// escape page: signing in and the interior are not safe in a browser Meta controls,
/// and the face is. Elsewhere it leads to the door page.
fn door(out: &mut String, face: &Value, ctx: &Ctx) {
    let invite_only = face["door"]["doorbell"].as_str().unwrap_or("door").is_empty();
    let label = face["door"]["label"]
        .as_str()
        .filter(|l| !l.trim().is_empty())
        .map(str::trim)
        .unwrap_or(if invite_only { "I have an invitation" } else { "Knock" });
    let href = if ctx.in_app { format!("/{}/escape", ctx.slug) } else { format!("/{}/door", ctx.slug) };
    out.push_str(&format!(
        "<a class=\"wf-btn wf-door\" href=\"{}\"><span class=\"wf-door-icon\">{DOOR_SVG}</span><span class=\"wf-door-label\">{}</span></a>\n",
        esc(&href),
        esc(label)
    ));
}

/// face.js `noticeEl`, without script: a checkbox the Continue button ticks, and a
/// rule in PAGE_CSS that hides the notice once it is ticked. A visitor who goes back
/// leaves for WallFlowers' own page, not the history of an app they may be inside.
fn notice(out: &mut String, face: &Value, _ctx: &Ctx) {
    let n = &face["notice"];
    if !n["on"].as_bool().unwrap_or(false) {
        return;
    }
    let (title, text) = match n["kind"].as_str() {
        Some("adult") => ("Over 18s only", "You need to be 18 or over to see this.".to_string()),
        Some("custom") => ("Before you go in", n["text"].as_str().unwrap_or("").trim().to_string()),
        _ => ("Sensitive content", "This may include things some people find upsetting.".to_string()),
    };
    out.push_str("<input type=\"checkbox\" id=\"wf-go\" aria-hidden=\"true\">\n");
    out.push_str("<div class=\"wf-notice\"><div class=\"wf-notice-card\"><span class=\"wf-notice-icon\">!</span>");
    out.push_str(&format!("<p class=\"wf-notice-title\">{}</p>", esc(title)));
    if !text.is_empty() {
        out.push_str(&format!("<p class=\"wf-notice-text\">{}</p>", esc(&text)));
    }
    out.push_str(&format!(
        "<label class=\"wf-btn wf-notice-go\" for=\"wf-go\">Continue</label><a class=\"wf-btn wf-notice-back\" href=\"{PUBLIC_ORIGIN}\">Go back</a></div></div>\n"
    ));
}

/// The look as CSS custom properties (`--bg:…;--fg:…`, as `render_face` sets them), for a
/// page that is not the Face's own and wears it: the Door's sign-in window for this Site.
pub fn look_style(f: &Face) -> String {
    f.look.style_attr(None)
}

/// The page a door leads to outside an in-app browser, until joining from the web is
/// built. It says so: a door that opened onto nothing would be the worse answer.
pub fn render_door(f: &Face, slug: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>Join {name}</title>\n<meta name=\"robots\" content=\"noindex\">\n<link rel=\"stylesheet\" href=\"/brand/wallflowers.css?v=2\">\n<link rel=\"stylesheet\" href=\"/assets/face/vendor/fonts.css\">\n<link rel=\"stylesheet\" href=\"/assets/face/face.css\">\n<style>{PAGE_CSS}.wf-face{{padding-top:18vh}}</style>\n</head>\n<body>\n<div class=\"wf-look\" style=\"{look}\"><div class=\"wf-backdrop\"></div><main class=\"wf-face\"><div class=\"wf-canvas\"><header class=\"wf-profile\"><h1 class=\"wf-name\">{name}</h1><p class=\"wf-bio\">Joining from the web opens soon. Until then, a member of {name} can invite you from the WallFlowers app.</p></header><p><a class=\"wf-w wf-link\" href=\"/{slug}\">Back to {name}</a></p></div></main></div>\n</body>\n</html>\n",
        name = esc(&f.name),
        slug = esc(slug),
        look = esc(&f.look.style_attr(None)),
    )
}

// ---- a post's page (W-98 Resources) --------------------------------------------------

/// A post's document as its Host item carries it: base64, its mime, its file name.
#[derive(Debug, PartialEq, Eq)]
pub struct Document {
    pub data: String,
    pub mime: String,
    pub name: String,
}

/// Rules for a post's page, beside PAGE_CSS: the face's width and look, its words set as an
/// article.
const POST_CSS: &str = "
.wf-post{padding:24px 16px 48px;color:var(--fg);font:16px/1.65 var(--font);text-align:left}
.wf-post-back{display:inline-flex;align-items:center;gap:8px;color:var(--muted);font-size:14px;text-decoration:none}
.wf-post-back img,.wf-post-back .wf-initials{width:28px;height:28px;border-radius:50%;object-fit:cover;font-size:11px}
.wf-post-title{margin:18px 0 6px;font:700 30px/1.15 var(--name-font);color:var(--title);overflow-wrap:anywhere}
.wf-post-excerpt{margin:0 0 18px;font-size:18px;color:var(--muted)}
.wf-post-doc{display:inline-block;margin:0 0 18px;padding:10px 14px;border-radius:10px;background:var(--card, rgba(0,0,0,.05));color:var(--fg);text-decoration:none;font-weight:600}
.wf-post-body{overflow-wrap:anywhere}
.wf-post-body h1,.wf-post-body h2,.wf-post-body h3,.wf-post-body h4,.wf-post-body h5,.wf-post-body h6{color:var(--title);line-height:1.25;margin:1.4em 0 .5em}
.wf-post-body p,.wf-post-body ul,.wf-post-body ol,.wf-post-body blockquote,.wf-post-body pre,.wf-post-body figure{margin:0 0 1em}
.wf-post-body blockquote{padding-left:14px;border-left:3px solid var(--accent);color:var(--muted)}
.wf-post-body pre{padding:12px;border-radius:8px;background:rgba(0,0,0,.05);overflow-x:auto;font-size:13px}
.wf-post-body code{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.9em}
.wf-post-body img{display:block;width:auto;max-width:100%;height:auto;border-radius:8px}
.wf-post-body a{color:var(--accent)}
.wf-post-body hr{border:0;border-top:1px solid var(--muted);opacity:.4;margin:2em 0}
";

/// What the Host carries for the post `id` under `post:<id>` and `post:<id>:<suffix>`.
fn post_item<'a>(ctx: &'a Ctx, id: &str, suffix: &str) -> Option<&'a str> {
    let key = if suffix.is_empty() { format!("post:{id}") } else { format!("post:{id}:{suffix}") };
    ctx.items.iter().find(|(k, _)| *k == key).map(|(_, p)| p.as_str())
}

fn is_object_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_base64(s: &str) -> bool {
    !s.is_empty() && s.len() % 4 == 0 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
}

/// One of the post's pictures, inline on the Host: a still, drawn from its own item.
fn post_picture(ctx: &Ctx, id: &str, asset: &str) -> Option<markdown::Picture> {
    let v: Value = serde_json::from_str(post_item(ctx, id, &format!("asset:{asset}"))?).ok()?;
    let data = v["asset"].as_str().filter(|d| is_base64(d))?;
    let mime = v["assetMime"].as_str().filter(|m| matches!(*m, "image/jpeg" | "image/png" | "image/webp" | "image/gif"))?;
    if v["assetVia"].as_str().is_some_and(|via| via != "inline") {
        return None;
    }
    let dim = |k: &str| v[k].as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
    Some(markdown::Picture { src: format!("data:{mime};base64,{data}"), width: dim("assetW"), height: dim("assetH") })
}

/// The post's document, inline on the Host (a detached one's bytes are the relay's, O-79).
pub fn post_document(ctx: &Ctx, id: &str) -> Option<Document> {
    if !is_object_id(id) {
        return None;
    }
    let v: Value = serde_json::from_str(post_item(ctx, id, "document")?).ok()?;
    if v["documentVia"].as_str().is_some_and(|via| via != "inline") {
        return None;
    }
    let data = v["document"].as_str().filter(|d| is_base64(d))?;
    let mime = v["documentMime"].as_str().filter(|m| *m == "application/pdf")?;
    Some(Document { data: data.to_string(), mime: mime.to_string(), name: v["name"].as_str().unwrap_or("").to_string() })
}

/// The public page of the post `id`: its title, excerpt, document and whole body, from the
/// Host's items (core face_items: `post:<id>`, `post:<id>:body:<n>` joined from 0 until one
/// is absent, `post:<id>:asset:<a>`, `post:<id>:document`), in the Face's look. `None` where
/// the Face would draw no row: not a post the Host carries, or one with no title.
pub fn render_post(f: &Face, ctx: &Ctx, id: &str) -> Option<String> {
    if !is_object_id(id) {
        return None;
    }
    let row: Value = serde_json::from_str(post_item(ctx, id, "")?).ok()?;
    let title = row["title"].as_str().map(str::trim).filter(|t| !t.is_empty())?;
    let excerpt = row["excerpt"].as_str().unwrap_or("").trim();
    let mut body = String::new();
    let mut n = 0;
    while let Some(p) = post_item(ctx, id, &format!("body:{n}")) {
        match serde_json::from_str::<Value>(p).ok().and_then(|v| v["text"].as_str().map(String::from)) {
            Some(t) => body.push_str(&t),
            None => break,
        }
        n += 1;
    }
    if n == 0 {
        body = row["body"].as_str().unwrap_or("").to_string();
    }
    let dark = if f.look.is_dark() { " wf-dark" } else { "" };
    let wallpaper = if ctx.has("wallpaper") { Some(ctx.media_url("wallpaper")) } else { None };
    let mut out = String::with_capacity(8 * 1024 + body.len() * 2);
    out.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1, viewport-fit=cover\">\n");
    out.push_str(&format!("<title>{} · {}</title>\n", esc(title), esc(&f.name)));
    if !excerpt.is_empty() {
        out.push_str(&format!("<meta name=\"description\" content=\"{}\">\n", esc(excerpt)));
    }
    if f.face["listed"].as_bool() == Some(false) {
        out.push_str("<meta name=\"robots\" content=\"noindex\">\n");
    }
    let url = format!("{PUBLIC_ORIGIN}/{}/post/{id}", ctx.slug);
    out.push_str(&format!("<meta property=\"og:type\" content=\"article\">\n<meta property=\"og:url\" content=\"{}\">\n", esc(&url)));
    out.push_str(&format!("<meta property=\"og:title\" content=\"{}\">\n", esc(title)));
    if !excerpt.is_empty() {
        out.push_str(&format!("<meta property=\"og:description\" content=\"{}\">\n", esc(excerpt)));
    }
    let icon = if ctx.has("mark") { ctx.media_url("mark") } else { "/brand/wallflowers-icon.svg".into() };
    out.push_str(&format!("<link rel=\"icon\" href=\"{}\">\n", esc(&icon)));
    out.push_str("<link rel=\"stylesheet\" href=\"/brand/wallflowers.css?v=2\">\n<link rel=\"stylesheet\" href=\"/assets/face/vendor/fonts.css\">\n<link rel=\"stylesheet\" href=\"/assets/face/face.css\">\n");
    out.push_str("<style>");
    out.push_str(PAGE_CSS);
    out.push_str(POST_CSS);
    out.push_str("</style>\n</head>\n<body>\n");
    out.push_str(&format!(
        "<div class=\"wf-look{dark}\" style=\"{}\">\n<div class=\"wf-backdrop\"></div>\n",
        esc(&f.look.style_attr(wallpaper.as_deref()))
    ));
    notice(&mut out, &f.face, ctx);
    out.push_str("<main class=\"wf-face\"><article class=\"wf-post\">\n");
    out.push_str(&format!("<a class=\"wf-post-back\" href=\"/{}\">", esc(ctx.slug)));
    avatar(&mut out, "wf-post-mark", f, ctx);
    out.push_str(&format!("<span>{}</span></a>\n", esc(&f.name)));
    out.push_str(&format!("<h1 class=\"wf-post-title\">{}</h1>\n", esc(title)));
    if !excerpt.is_empty() {
        out.push_str(&format!("<p class=\"wf-post-excerpt\">{}</p>\n", esc(excerpt)));
    }
    if let Some(d) = post_document(ctx, id) {
        out.push_str(&format!(
            "<a class=\"wf-post-doc\" href=\"/{}/post/{id}/document\">{}</a>\n",
            esc(ctx.slug),
            esc(if d.name.trim().is_empty() { "PDF" } else { d.name.trim() })
        ));
    }
    out.push_str("<div class=\"wf-post-body\">\n");
    if row["bodyFormat"].as_str() == Some("markdown") {
        out.push_str(&markdown::render(&body, &|a: &str| post_picture(ctx, id, a)));
    } else {
        for para in body.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
            let lines: Vec<String> = para.lines().map(esc).collect();
            out.push_str(&format!("<p>{}</p>\n", lines.join("<br>")));
        }
    }
    out.push_str("</div>\n</article></main>\n");
    foot(&mut out);
    out.push_str("</div>\n</body>\n</html>\n");
    Some(out)
}

// ---- the escape page -----------------------------------------------------------------

/// The escape page's design, compiled in from the website (one copy, transformed —
/// never redrawn here). See `site/escape.html` for why it exists.
/// The escape page as the website serves it — a COPY (see `assets.rs`: this crate must
/// compile without the website repository beside it).
const ESCAPE_HTML: &str = include_str!("../vendor/escape.html");

/// What `render_escape` replaces, each of which must be in escape.html exactly once
/// (pinned by a test that names the one that is missing).
pub(crate) const ESCAPE_MARKERS: &[&str] = &[
    "<title>Escape the matrix! · WallFlowers</title>",
    r#"<div class="logo placeholder" role="img" aria-label="Site logo (placeholder)">logo</div>"#,
    r#"<figcaption class="name">Site Name</figcaption>"#,
];

/// The escape page for one site: its name and mark where the placeholders stand,
/// and every `brand/` path made absolute, because it is served at `/<slug>/escape`.
pub fn render_escape(slug: &str, name: &str, has_mark: bool) -> String {
    let mut s = ESCAPE_HTML
        .replace("\"brand/", "\"/brand/")
        .replace("url(\"brand/", "url(\"/brand/")
        .replace(ESCAPE_MARKERS[0], &format!("<title>Escape the matrix! · {}</title>", esc(name)))
        .replace(ESCAPE_MARKERS[2], &format!(r#"<figcaption class="name">{}</figcaption>"#, esc(name)));
    if has_mark {
        s = s.replace(
            ESCAPE_MARKERS[1],
            &format!(r#"<img class="logo" src="/{}/m/mark" alt="">"#, esc(slug)),
        );
    }
    s
}

// ---- in-app browsers ---------------------------------------------------------------------

fn has_token(ua: &str, token: &str) -> bool {
    // `token` must start at a word boundary: "Instagram" but not "NotInstagram".
    let mut from = 0;
    while let Some(i) = ua[from..].find(token) {
        let at = from + i;
        let before_ok = at == 0 || !ua.as_bytes()[at - 1].is_ascii_alphanumeric();
        if before_ok {
            return true;
        }
        from = at + token.len();
    }
    false
}

/// Whether a request comes from an app's own browser — Instagram, Facebook,
/// Messenger, Threads — or any Android WebView. Routing, not security: a host app can
/// send any user agent it likes. See core/docs/instagram-in-app-browser.html.
pub fn is_in_app_browser(user_agent: &str, x_requested_with: Option<&str>) -> bool {
    if let Some(app) = x_requested_with {
        if matches!(
            app.trim(),
            "com.instagram.android" | "com.facebook.katana" | "com.facebook.orca" | "com.instagram.barcelona"
        ) {
            return true;
        }
    }
    let ua = user_agent;
    has_token(ua, "Instagram ")
        || has_token(ua, "FBAN/")
        || has_token(ua, "FBAV/")
        || has_token(ua, "FB_IAB")
        || has_token(ua, "Barcelona ")
        || has_token(ua, "IABMV/")
        || ua.contains("; wv)")
}
