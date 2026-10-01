//! What a served face must never do, and what it must always do.

use super::*;
use serde_json::json;

fn bundle(face: Value) -> String {
    json!({
        "v": 1,
        "profile": { "displayName": "Mill Road Allotments", "shape": "community",
                     "card": { "note": "Thirty plots behind the station.",
                               "urls": [ {"label": "instagram", "value": "https://instagram.com/millroad"} ] } },
        "face": face,
        "location": { "shape": "area", "precision": 5, "label": "Romsey", "cells": [] }
    })
    .to_string()
}

/// face.js's own default widgets and Paper look.
fn default_face() -> Value {
    json!({
        "v": 1,
        "look": { "wallpaper": {"style":"fill","colour2":"#FAF8F3","colour3":"#FAF8F3","angle":"down","pattern":"dots","image":"","soften":30},
                  "blocks": {"style":"solid","corners":3,"shadow":"soft"},
                  "text": {"font":"system-sans","title":"marker"},
                  "colours": {"background":"#FAF8F3","cards":"#FFFFFF","buttons":"#546CAC","buttonText":"#FFFFFF","text":"#14181E","title":"#14181E"} },
        "header": { "layout": "classic", "title": "name" },
        "show": { "members": true, "address": true, "purpose": true, "share": true },
        "widgets": { "root": { "type": "root", "version": 1, "children": [
            {"version":1,"type":"wf-feed","source":"events","mode":"next","heading":"Next event","fields":["venue"],"empty":"hide","emptyNote":"","size":"m"},
            {"version":1,"type":"wf-location","heading":"Where we are","size":"s"},
            {"version":1,"type":"wf-text","text":"Open Sundays, 10–2.","size":"s"},
            {"version":1,"type":"wf-feed","source":"publications","mode":"list","heading":"Latest","fields":["link"],"empty":"hide","emptyNote":"","size":"m"},
            {"version":1,"type":"wf-social","size":"m"},
            {"version":1,"type":"wf-link","title":"Plot list","url":"https://example.org/plots","size":"m"}
        ] } },
        "stickers": [ {"id":"doodle-loop","x":0.8,"y":0.42,"size":0.22,"rot":-12,"flip":false,"colour":"buttons"} ],
        "notice": { "on": false, "kind": "sensitive", "text": "" },
        "door": { "doorbell": "door", "label": "", "questions": [] },
        "listed": true
    })
}

const NOW: i64 = 1_790_000_000_000; // a fixed "now", so a test never depends on the clock

fn ctx<'a>(slug: &'a str, media: &'a [&'a str], items: &'a [(String, String)], in_app: bool) -> Ctx<'a> {
    Ctx { slug, media, items, members: None, in_app, now_ms: NOW }
}

fn event(title: &str, at: i64, venue: &str) -> (String, String) {
    (
        format!("event:{title}"),
        json!({ "title": title, "startMs": at, "venue": venue }).to_string(),
    )
}

#[test]
fn the_default_face_renders_with_face_css_classes() {
    let f = parse(&bundle(default_face()), "Host").unwrap();
    let items = vec![event("Dig day", NOW + 86_400_000, "The shed")];
    let html = render_face(&f, &ctx("allotments", &["mark"], &items, false));

    for want in [
        "<h1 class=\"wf-name\">Mill Road Allotments</h1>",
        "class=\"wf-handle\">wallflowers.io/allotments",
        "class=\"wf-bio\">Thirty plots behind the station.",
        "<img class=\"wf-avatar\" src=\"/allotments/m/mark\"",
        "class=\"wf-w wf-link\" href=\"https://example.org/plots\"",
        "Open Sundays, 10–2.",
        "wf-place-scale\">Within about 4.9 km",
        "class=\"wf-glyph\" href=\"https://instagram.com/millroad\"",
        "wf-row-main\">Dig day",
        "The shed",
        "wf-sticker",
        "/assets/face/face.css",
        "--name-font:&quot;Wallflowers&quot;", // the style attribute is escaped, as it must be
    ] {
        assert!(html.contains(want), "the face is missing {want:?}");
    }
    // A feed with nothing in it, told to hide, is not drawn at all.
    assert!(!html.contains("Latest"), "an empty publications feed should be left out");
    assert!(html.contains("wf-door"), "every face has a door");
}

#[test]
fn nothing_from_the_log_becomes_script_or_a_bad_link() {
    let mut face = default_face();
    face["widgets"]["root"]["children"] = json!([
        {"type":"wf-link","title":"<script>alert(1)</script>","url":"javascript:alert(1)","size":"m"},
        {"type":"wf-link","title":"ok","url":"https://ok.example/\" onmouseover=\"alert(1)","size":"m"},
        {"type":"wf-text","text":"</div><script>alert(2)</script>","size":"m"},
        {"type":"wf-social","size":"m"}
    ]);
    face["door"]["label"] = json!("<img src=x onerror=alert(3)>");
    let mut b: Value = serde_json::from_str(&bundle(face)).unwrap();
    b["profile"]["displayName"] = json!("<svg onload=alert(4)>Allotments</svg>");
    b["profile"]["card"]["urls"] = json!([
        {"label":"website","value":"javascript:alert(5)"},
        {"label":"website","value":"data:text/html,<script>alert(6)</script>"}
    ]);
    let f = parse(&b.to_string(), "Host").unwrap();
    let html = render_face(&f, &ctx("allotments", &[], &[], false));

    // Nothing may reach the page as MARKUP. The same characters as escaped text are
    // fine — that is what escaping is for — so each check names the live form.
    assert!(!html.contains("<script"), "no script element may reach the page");
    assert!(!html.to_lowercase().contains("javascript:"), "no javascript: URL");
    assert!(!html.contains("<img src=x"), "no injected element");
    assert!(!html.contains("<svg onload"), "no injected element");
    assert!(!html.contains("\" onmouseover"), "an unescaped quote would have closed the attribute");
    assert!(html.contains("&lt;svg onload=alert(4)&gt;Allotments"), "it is drawn as the text it is");
    assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;") || !html.contains("alert(1)"));
    // The link with the javascript: URL is not drawn as a link at all.
    assert!(!html.contains("alert(1)</a>"));
    // The social widget had no usable links left, so it is not drawn. (The class
    // name is in the page's own stylesheet; what must be absent is the element.)
    assert!(!html.contains("<div class=\"wf-w wf-social\""));
}

#[test]
fn a_broken_look_falls_back_to_paper_and_listed_false_is_noindex() {
    let mut face = default_face();
    face["look"]["colours"]["background"] = json!("red; background:url(evil)");
    face["look"]["colours"]["text"] = json!("#ZZZZZZ");
    face["look"]["text"]["font"] = json!("../../etc/passwd");
    face["look"]["blocks"]["style"] = json!("wham");
    face["listed"] = json!(false);
    let f = parse(&bundle(face), "Host").unwrap();
    let html = render_face(&f, &ctx("allotments", &[], &[], false));
    assert!(html.contains("--bg:#FAF8F3"), "a bad colour falls back to Paper's");
    assert!(html.contains("--fg:#14181E"));
    assert!(!html.contains("evil"));
    assert!(!html.contains("passwd"));
    assert!(html.contains("<meta name=\"robots\" content=\"noindex\">"));
}

#[test]
fn the_door_leads_out_of_an_in_app_browser_and_in_from_anywhere_else() {
    let f = parse(&bundle(default_face()), "Host").unwrap();
    let inside = render_face(&f, &ctx("allotments", &[], &[], true));
    assert!(inside.contains("href=\"/allotments/escape\""), "in an app's browser the door is the way out");
    let outside = render_face(&f, &ctx("allotments", &[], &[], false));
    assert!(outside.contains("href=\"/allotments/door\""));

    let mut face = default_face();
    face["door"]["doorbell"] = json!("");
    let f = parse(&bundle(face), "Host").unwrap();
    let html = render_face(&f, &ctx("allotments", &[], &[], false));
    assert!(html.contains("I have an invitation"), "invite only says so");
}

#[test]
fn a_feed_says_so_when_it_is_empty_and_only_shows_what_is_still_to_come() {
    let mut face = default_face();
    face["widgets"]["root"]["children"] = json!([
        {"type":"wf-feed","source":"events","mode":"list","heading":"Coming up","fields":["venue"],"empty":"note","emptyNote":"Nothing until spring","size":"l"}
    ]);
    let f = parse(&bundle(face.clone()), "Host").unwrap();
    let html = render_face(&f, &ctx("a", &[], &[], false));
    assert!(html.contains("Nothing until spring"));

    let items = vec![
        event("Long past", NOW - 40 * 86_400_000, "The shed"),
        event("Soon", NOW + 86_400_000, "The shed"),
        event("Later", NOW + 5 * 86_400_000, "The gate"),
    ];
    let f = parse(&bundle(face), "Host").unwrap();
    let html = render_face(&f, &ctx("a", &[], &items, false));
    assert!(html.contains("Soon") && html.contains("Later"));
    assert!(!html.contains("Long past"), "an event that has been and gone is not coming up");
    assert!(html.find("Soon").unwrap() < html.find("Later").unwrap(), "soonest first");
}

#[test]
fn a_feed_draws_only_its_own_kind_and_a_kind_with_no_items_follows_its_empty_rule() {
    let mut face = default_face();
    face["widgets"]["root"]["children"] = json!([
        {"type":"wf-feed","source":"events","mode":"list","heading":"Coming up","fields":["venue"],"empty":"hide","emptyNote":"","size":"m"},
        {"type":"wf-feed","source":"rooms","mode":"list","heading":"Rooms","fields":[],"empty":"hide","emptyNote":"","size":"m"},
        {"type":"wf-feed","source":"listings","mode":"list","heading":"For sale","fields":["price"],"empty":"note","emptyNote":"Nothing for sale","size":"m"}
    ]);
    let items = vec![event("Dig day", NOW + 86_400_000, "The shed")];
    let html = render_face(&parse(&bundle(face), "Host").unwrap(), &ctx("a", &[], &items, false));
    assert_eq!(html.matches("Dig day").count(), 1, "an event is drawn once, by the events feed, and by no other kind's");
    assert!(!html.contains(">Rooms<"), "rooms have no items on the Host; told to hide, the widget is left out");
    assert!(html.contains(">For sale<") && html.contains("Nothing for sale"), "told to say so, it shows the owner's note");
}

#[test]
fn publications_are_newest_first_by_when_the_site_named_them_not_by_key() {
    let mut face = default_face();
    face["widgets"]["root"]["children"] = json!([
        {"type":"wf-feed","source":"publications","mode":"list","heading":"Latest","fields":[],"empty":"hide","emptyNote":"","size":"l"}
    ]);
    let post = |id: &str, title: &str, at: i64| (format!("post:{id}"), json!({ "title": title, "at": at }).to_string());
    // Key order is a, b, c; `at` order is c, a, b.
    let items = vec![post("a", "Middle", 20), post("b", "Oldest", 10), post("c", "Newest", 30)];
    let html = render_face(&parse(&bundle(face), "Host").unwrap(), &ctx("a", &[], &items, false));
    let at = |t: &str| html.find(t).unwrap_or_else(|| panic!("{t} not drawn"));
    assert!(at("Newest") < at("Middle") && at("Middle") < at("Oldest"), "newest first, by at");
}

/// A publication row leads to its post's page (W-98 Resources), or, where the widget shows
/// links and the post has one, to where it points. An event row is as it was.
#[test]
fn a_publication_row_leads_to_its_posts_page() {
    let (a, b) = ("0a".repeat(32), "0b".repeat(32));
    for (fields, b_href) in [(json!(["link"]), "https://example.org/x".to_string()), (json!([]), format!("/mill/post/{b}"))] {
        let mut face = default_face();
        face["widgets"]["root"]["children"] = json!([
            {"type":"wf-feed","source":"publications","mode":"list","heading":"Latest","fields":fields,"empty":"hide","emptyNote":"","size":"l"}
        ]);
        let items = vec![
            (format!("post:{a}"), json!({ "title": "Guide", "at": 2 }).to_string()),
            (format!("post:{b}"), json!({ "title": "Elsewhere", "form": "link", "link": "https://example.org/x", "at": 1 }).to_string()),
            ("post:not-an-id".to_string(), json!({ "title": "Odd", "at": 0 }).to_string()),
        ];
        let html = render_face(&parse(&bundle(face), "Host").unwrap(), &ctx("mill", &[], &items, false));
        assert!(html.contains(&format!("<a class=\"wf-row-main\" href=\"/mill/post/{a}\">Guide</a>")), "{html}");
        assert!(html.contains(&format!("href=\"{b_href}\"")), "{b_href}");
        assert!(html.contains("<span class=\"wf-row-main\">Odd</span>"), "no page for what is not a post id");
    }
}

/// A post's public page rides the Host beside its row (W-98, ICD 2.3.1): `post:<id>:body:<n>`,
/// `:asset:<a>` and `:document`. Only `post:<id>` is an item a Face draws; a key with a second
/// ':' segment is part of a post's page, never a row, whatever its payload says.
#[test]
fn a_posts_page_keys_are_not_rows() {
    let mut face = default_face();
    face["widgets"]["root"]["children"] = json!([
        {"type":"wf-feed","source":"publications","mode":"list","heading":"Latest","fields":[],"empty":"hide","emptyNote":"","size":"l"}
    ]);
    let id = "0b".repeat(32);
    let items = vec![
        (format!("post:{id}"), json!({ "title": "The zine", "at": 1 }).to_string()),
        (format!("post:{id}:body:0"), json!({ "title": "Body chunk", "text": "words" }).to_string()),
        (format!("post:{id}:asset:00000000000000a1"), json!({ "title": "An asset", "assetMime": "image/png" }).to_string()),
        (format!("post:{id}:document"), json!({ "title": "A document", "name": "zine.pdf" }).to_string()),
    ];
    let html = render_face(&parse(&bundle(face), "Host").unwrap(), &ctx("a", &[], &items, false));
    assert!(html.contains("The zine"));
    for t in ["Body chunk", "An asset", "A document"] {
        assert!(!html.contains(t), "{t} is part of a post's page, not a row");
    }
    assert!(is_item_key(&format!("post:{id}")) && is_item_key("event:e1"));
    for k in [format!("post:{id}:body:0"), format!("post:{id}:asset:00000000000000a1"), format!("post:{id}:document")] {
        assert!(!is_item_key(&k), "{k}");
    }
}

#[test]
fn the_escape_page_is_the_websites_own_design_with_this_site_in_it() {
    for marker in ESCAPE_MARKERS {
        assert_eq!(
            ESCAPE_HTML.matches(marker).count(),
            1,
            "site/escape.html no longer holds exactly one {marker:?} — render_escape cannot fill it"
        );
    }
    let html = render_escape("allotments", "Mill Road <Allotments>", true);
    assert!(html.contains("<title>Escape the matrix! · Mill Road &lt;Allotments&gt;</title>"));
    assert!(html.contains("<figcaption class=\"name\">Mill Road &lt;Allotments&gt;</figcaption>"));
    assert!(html.contains("<img class=\"logo\" src=\"/allotments/m/mark\" alt=\"\">"));
    assert!(html.contains("href=\"/brand/wallflowers-icon.svg\""), "brand paths are absolute at /<slug>/escape");
    assert!(html.contains("url(\"/brand/wallflowers-regular.woff2?v=2\")"));
    assert!(!html.contains("<script"), "the escape page runs nothing");
    assert!(!html.contains("\"brand/"), "no relative brand path is left");

    let plain = render_escape("allotments", "Allotments", false);
    assert!(plain.contains("logo placeholder"), "no mark, no img — the placeholder stands");
}

#[test]
fn an_apps_own_browser_is_recognised_and_a_real_one_is_not() {
    let instagram_ios = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/23D8133 Instagram 421.1.6.39.52 (iPhone15,2; iOS 26_3_1; en_GB; en-GB; scale=3.00; 1179x2556; IABMV/1; 910922286) NW/3 Safari/604.1";
    let instagram_android = "Mozilla/5.0 (Linux; Android 13; SM-G991B Build/TP1A; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/151.0 Mobile Safari/537.36 Instagram 443.0.0.48.82 Android";
    let facebook = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 [FBAN/FBIOS;FBAV/577.1.0.53.108;FBBV/1]";
    let safari = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Mobile/15E148 Safari/604.1";
    let chrome = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";

    for ua in [instagram_ios, instagram_android, facebook] {
        assert!(is_in_app_browser(ua, None), "should be an in-app browser: {ua}");
    }
    for ua in [safari, chrome] {
        assert!(!is_in_app_browser(ua, None), "should NOT be an in-app browser: {ua}");
    }
    assert!(is_in_app_browser(chrome, Some("com.instagram.android")), "Android says so in a header");
    assert!(!is_in_app_browser(chrome, Some("XMLHttpRequest")));
}

#[test]
fn the_vendored_assets_parse() {
    let a = assets::assets();
    assert!(a.fonts.contains_key("fraunces") && a.icons.contains_key("instagram") && !a.stickers.is_empty(),
            "face-assets.js parsed: {} fonts, {} icons, {} stickers", a.fonts.len(), a.icons.len(), a.stickers.len());
}

/// The vendored copies, pinned (NC-69). `product/` must build standalone — the website is a
/// different repository — so the files this crate draws from are COPIES under `vendor/`,
/// and `vendor/SOURCES` names each one's sha256 and where it came from. This runs on
/// every checkout: a copy edited by hand, or re-vendored without its pin, fails here.
#[test]
fn vendored_copies_are_the_pinned_bytes() {
    use sha2::Digest as _;
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor");
    let pins = sources();
    assert!(!pins.is_empty(), "vendor/SOURCES names nothing");
    for (sha, file, _, _) in &pins {
        let bytes = std::fs::read(dir.join(file)).unwrap_or_else(|e| panic!("vendor/{file}: {e}"));
        assert_eq!(&hex(&sha2::Sha256::digest(&bytes)), sha, "vendor/{file} is not the pinned copy: re-vendor it and change its line in vendor/SOURCES together");
    }
}

/// The website's own copies against the same pins, and face.js against the doodles drawn
/// here. It needs the website checked out beside product, so a product-only `cargo test`
/// lists it as ignored rather than passing it; `make check-arc` runs it, and there the
/// website's absence FAILS. The path is derived from this file's own location.
#[test]
#[ignore = "needs business/website beside product; make check-arc runs it"]
fn vendored_copies_match_the_website() {
    use sha2::Digest as _;
    let website = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../business/website");
    assert!(website.join("site").is_dir(), "the website is not checked out at {}: the vendored copies cannot be compared here", website.display());
    for (sha, file, theirs, from) in sources() {
        let bytes = std::fs::read(website.join(&theirs)).unwrap_or_else(|e| panic!("the website has no {theirs}: {e}"));
        assert_eq!(
            hex(&sha2::Sha256::digest(&bytes)),
            sha,
            "the website's {theirs} has moved since {from}: re-vendor vendor/{file} and its pin (they are served to the same eyes)"
        );
    }
    // The ten drawn doodles are copied from face.js; if it redraws one, say so here.
    let face_js = std::fs::read_to_string(website.join("site/assets/face/face.js")).unwrap();
    for (id, svg) in assets::DOODLES {
        assert!(face_js.contains(id), "face.js no longer offers the sticker {id}");
        assert!(face_js.contains(svg), "face.js has redrawn {id} — copy it across");
    }
}

/// `vendor/SOURCES`: (sha256, vendored file, website path, website commit) per line.
fn sources() -> Vec<(String, String, String, String)> {
    let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/SOURCES")).expect("vendor/SOURCES");
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            assert_eq!(f.len(), 4, "vendor/SOURCES: {l:?} is not sha256, file, website path, commit");
            (f[0].into(), f[1].into(), f[2].into(), f[3].into())
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Write a sample face and escape page for a human to open. Not an assertion: a door
/// into the work, as `m18`'s fixture is.
#[test]
fn write_samples() {
    let items = vec![
        event("Dig day", NOW + 86_400_000, "The shed"),
        (
            "post:1".into(),
            json!({ "title": "Plot 12 has beans", "body": "More than anyone can eat.", "link": "https://example.org/beans" }).to_string(),
        ),
    ];
    let mut face = default_face();
    face["widgets"]["root"]["children"][3]["fields"] = json!(["link", "body"]);
    let f = parse(&bundle(face), "Host").unwrap();
    let html = render_face(&f, &ctx("allotments", &["mark", "cover"], &items, false));
    // The WORKSPACE's target dir, which is gitignored — a crate-local `target/` is not,
    // and these samples are output to look at, never source.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/face-render");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("face-sample.html"), &html);
    let _ = std::fs::write(dir.join("escape-sample.html"), render_escape("allotments", "Mill Road Allotments", true));
    // A post's page: a markdown body in two pieces, a picture of its own, a document.
    let png = "iVBORw0KGgoAAAANSUhEUgAAADAAAAAcCAIAAACoDqudAAAD4klEQVR42r3Td19aZxjG8eeFtWlNkzatSVRwoTgBwYWIi7333ktwJg7EGXHhSqLNas1O23fT59yHcw6i+fd5B9/P/bsv9IOq/kd1/R1Nw89aXo2O/4u+8Z6h6Vdj02+m5t/NLX9YWh9aBY9sbY/tbfWO9gankO/qaHR3Nnu6WrxdAl93m79HGOjtCIq6wuLuiLgnKhHF+sRxaV9CJk3296f6B9IDQ5nB4emhkeywIicfnZGPzY6Mzykm50enFpTqxTHNkzHd03H90oRxedK0MoXuqBt+0vBqtPy7Ov49feN9A0V5AJRaa+sjm4Cl8ChKR5OnE1NagdJOUXo7Q6LusLgnIumN9oljUklcKk3IZEmKMggUeXZIkRtmKRMUZVS1qMQULVAMFGXSvDplzatQjZZ3Fw5znzpM8wNTCz5MLRzmsa2tzk5ThHygNANFQFF6hEDp4ih9EjiMjDrMwGB6EB9GDocZzcmVMzRFMQkUNVB0FGXCCBRLXmVbU9sLGvS9RnVwGB40aqpsBJSOULlRLzSSfL+REg4zAY1UlY2AYgKKdU1lL6gd6xrXhhbRFKaRoLIR8y5VjURsI1GUfRcZTWEaDVc2Yt6lqtEU3QgfxlHQODe07k2dZ0uHyo2srQ+BUld+F+GNd7m9Ufld2EbZoRGgKMvvorjxLtWNnHAYz6bOu6X3bxtQLdeonWvEvUt3+7VGkspGzLsMsO8CjUa4Rty7jBuuNVKzjdxA8W0bAjuG4DMjIjlp+l1sQGEaYYreD5TQrilSNCOSk2YaablGO4YAUMJFU3TPHNu3IJKTdlY3MuLDhOEwsT1LfN+aPLQhkpO+tVGUolgSB9bUoS19ZEckJ01TmEZmuhGmJIGSKTmyxw5EctLlRkVTBChxoKSObJmSffrYkTtxzpy6EMlJh7lGVrYRpmSBMnvmmj93I5KTxpQbjZz4MLOnrrkz98K5Z/G5F5GcdKLcyF7ZCFPmKYrnyQvv0ksfIjlp/C7pUmUjTHEvAOXpS9/yhX/lMoBITnq6shFQFoGydOFbufSv/hlYexVEJCd9vZGXboQPs3oZyL8KFl6H1t+EEMlJz93WKA+HWX8d2ngT3nobQSQnXdHIzzYqAGXzbWT7XWTnrygiOWn6XZaBwjTClPAWUJ79HStexRHJSTONglyjd5FtoOxexfbex/c/JBDJSeerG0XxYXbhMPvvEwcfkkefUojkpG9ttEdREocfk6VPqePPaURy0jSFaRSnG2HKEVBOvmTOvmYQyUmXG13FikA5AErpc+rkS/r0a+b82/Tzf7KI5KR3uUZJthGmnAHlxb/Zi/9y/wOdKX4JJOb6kgAAAABJRU5ErkJggg==";
    let body = "Sunday at the **long shed**, from ten.\n\n## What to bring\n\n- seeds in paper envelopes, *named*\n- seedlings in pots\n\n![the beds in June](asset:00000000000000a1)\n\n> Swap, don't sell.\n\nThe rota is [on the board](https://example.org/rota).";
    let (one, two) = body.split_at(40);
    let post = vec![
        post_row("Seed swap", body, Some("markdown")),
        piece(0, one),
        piece(1, two),
        picture(A1, png, "image/png"),
        document("JVBERi0xLjQK", "application/pdf", "inline"),
    ];
    let _ = std::fs::write(dir.join("post-sample.html"), render_post(&f, &ctx("allotments", &["mark", "cover"], &post, false), PID).unwrap_or_default());
}

/// O-62: the wallpaper's url() takes a plain path or an image's data, and nothing that
/// closes its quote. The Arc's own `/{slug}/m/wallpaper` is a plain path.
#[test]
fn a_wallpaper_url_draws_only_in_its_two_shapes() {
    let look = Look::from_value(&json!({ "wallpaper": { "style": "image", "soften": 30 } }));
    for good in ["/allotments/m/wallpaper", "data:image/png;base64,iVBORw0KGgo="] {
        assert!(look.style_attr(Some(good)).contains(&format!("url(\"{good}\")")), "{good}");
    }
    for bad in [
        "a\"), url(\"https://tracker.example/x",
        "a'), url('https://tracker.example/x",
        "//tracker.example/x.png",
        "https://tracker.example/x.png",
        "data:image/svg+xml;base64,PHN2Zz4=",
        "data:image/png;base64,iVBOR\"), url(\"https://tracker.example/x",
        "",
    ] {
        let s = look.style_attr(Some(bad));
        assert!(!s.contains("url(") && !s.contains("tracker"), "{bad:?} drew: {s}");
    }
}

/// The look another page wears (the Door's sign-in window): the Face's own colours and fonts
/// as custom properties, and a hostile look's values never among them.
#[test]
fn look_style_is_the_faces_own_look_and_nothing_hostile() {
    let f = parse(&bundle(default_face()), "Mill Road Allotments").expect("parses");
    let s = look_style(&f);
    for want in ["--bg:#FAF8F3", "--fg:#14181E", "--accent:#546CAC", "--on-accent:#FFFFFF", "--font:", "--name-font:"] {
        assert!(s.contains(want), "{want} in {s}");
    }
    let mut bad = default_face();
    bad["look"]["colours"]["background"] = json!("red; background:url(evil)");
    let s = look_style(&parse(&bundle(bad), "x").expect("parses"));
    assert!(!s.contains("evil") && !s.contains("url("), "{s}");
}

#[test]
fn a_look_sets_the_properties_it_names_and_no_others() {
    for (style, shadow, wallpaper) in
        [("solid", "soft", "fill"), ("glass", "hard", "pattern"), ("outline", "none", "blur"), ("solid", "strong", "gradient")]
    {
        let mut f = default_face();
        f["look"]["blocks"]["style"] = json!(style);
        f["look"]["blocks"]["shadow"] = json!(shadow);
        f["look"]["wallpaper"]["style"] = json!(wallpaper);
        let s = look_style(&parse(&bundle(f), "x").expect("parses"));
        let names: Vec<&str> = s.split(';').map(|d| d.split(':').next().unwrap_or("")).collect();
        assert_eq!(names, LOOK_PROPERTIES, "{s}");
    }
}

// ---- a post's public page (W-98 Resources) -------------------------------------------
//
// What the Host carries for a network post (core face_items, ICD 2.3.1): `post:<id>`, the row,
// its body possibly clipped; `post:<id>:body:<n>`, the whole body in pieces; one
// `post:<id>:asset:<a>` per picture and `post:<id>:document`, each a MediaRef's args as JSON.

const PID: &str = "d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1";
const A1: &str = "00000000000000a1";

fn post_row(title: &str, body: &str, format: Option<&str>) -> (String, String) {
    let mut v = json!({ "title": title, "body": body, "form": "article", "excerpt": "Bring what you grew.", "at": NOW - 5 });
    if let Some(f) = format {
        v["bodyFormat"] = json!(f);
    }
    (format!("post:{PID}"), v.to_string())
}
fn piece(n: usize, text: &str) -> (String, String) {
    (format!("post:{PID}:body:{n}"), json!({ "text": text }).to_string())
}
fn picture(id: &str, data: &str, mime: &str) -> (String, String) {
    (
        format!("post:{PID}:asset:{id}"),
        json!({ "asset": data, "assetMime": mime, "assetVia": "inline", "assetKind": "still", "assetW": 4, "assetH": 3, "id": id, "alt": "the beds", "at": 1 }).to_string(),
    )
}
fn document(data: &str, mime: &str, via: &str) -> (String, String) {
    (
        format!("post:{PID}:document"),
        json!({ "document": data, "documentMime": mime, "documentVia": via, "documentKind": "document", "name": "rota.pdf" }).to_string(),
    )
}
fn post_page(items: &[(String, String)]) -> Option<String> {
    let f = parse(&bundle(default_face()), "Host").unwrap();
    render_post(&f, &ctx("allotments", &["mark"], items, false), PID)
}

#[test]
fn a_posts_page_is_its_title_excerpt_and_whole_body_from_the_hosts_items() {
    let items = vec![
        post_row("Seed swap", "Sunday at the **long…", Some("markdown")),
        piece(1, "shed**, from ten.\n\n## What to bring\n\n- seeds"),
        piece(0, "Sunday at the **long "),
        event("Dig day", NOW + 1, "The shed"),
    ];
    let html = post_page(&items).expect("the post's page");
    for want in [
        "<title>Seed swap · Mill Road Allotments</title>",
        "<meta name=\"description\" content=\"Bring what you grew.\">",
        "<meta property=\"og:url\" content=\"https://wallflowers.io/allotments/post/",
        "<h1 class=\"wf-post-title\">Seed swap</h1>",
        "<p class=\"wf-post-excerpt\">Bring what you grew.</p>",
        "<strong>long shed</strong>",
        "<h2>What to bring</h2>",
        "<li>seeds</li>",
        "href=\"/allotments\"",
        "/assets/face/face.css",
        "--bg:#FAF8F3",
    ] {
        assert!(html.contains(want), "the post's page is missing {want:?}\n{html}");
    }
    assert!(!html.contains("long…"), "the row's clipped body is not the page's");
    assert!(!html.contains("Dig day"), "only the post");
    assert!(!html.contains("<script"), "no script, ever");
}

#[test]
fn a_post_page_is_only_a_post_the_host_carries() {
    let items = vec![post_row("Seed swap", "b", None), piece(0, "b"), event("Dig day", NOW + 1, "The shed")];
    let f = parse(&bundle(default_face()), "Host").unwrap();
    let c = ctx("allotments", &[], &items, false);
    assert!(render_post(&f, &c, PID).is_some());
    for id in ["e".repeat(64), format!("{PID}:body:0"), "Dig day".into(), PID.to_uppercase(), String::new()] {
        assert!(render_post(&f, &c, &id).is_none(), "{id}");
    }
    let untitled = vec![post_row(" ", "b", None)];
    assert!(render_post(&f, &ctx("allotments", &[], &untitled, false), PID).is_none(), "a row the Face would not draw has no page");
}

#[test]
fn a_posts_pictures_are_its_own_assets_drawn_inline_and_nothing_is_fetched() {
    let body = "![the beds](asset:00000000000000a1)\n\n![far](https://x.org/c.png)\n\n[js](javascript:alert(1)) <script>alert(1)</script>\n\n![gone](asset:00000000000000a2) ![bad](asset:00000000000000a3)";
    let items = vec![
        post_row("Seed swap", body, Some("markdown")),
        piece(0, body),
        picture(A1, "iVBORw0KGgo=", "image/png"),
        picture("00000000000000a3", "\"><script>", "image/png"),
        (format!("post:{}:asset:00000000000000a2", "e".repeat(64)), json!({ "asset": "iVBORw0KGgo=", "assetMime": "image/png", "id": "00000000000000a2" }).to_string()),
    ];
    let html = post_page(&items).expect("the page");
    assert!(html.contains("<img src=\"data:image/png;base64,iVBORw0KGgo=\" alt=\"the beds\" width=\"4\" height=\"3\">"), "{html}");
    assert!(html.contains("<a href=\"https://x.org/c.png\">far</a>"), "an image from elsewhere is a link to it");
    assert!(!html.contains("src=\"https:"), "never fetched");
    assert!(!html.to_lowercase().contains("javascript:"), "a script link is text");
    assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"), "raw HTML is text");
    assert!(!html.contains("<script"), "no script, ever");
    assert!(html.contains("gone") && !html.contains("asset:00000000000000a2"), "another post's picture is not this one's: its alt stands");
    assert!(!html.contains("\"><script>") && html.contains("bad"), "a picture whose data is not base64 is not drawn");
}

#[test]
fn a_plain_body_is_its_paragraphs_and_no_markup() {
    let items = vec![post_row("Seed swap", "One **two** <b>x</b>\nline\n\nThree", None), piece(0, "One **two** <b>x</b>\nline\n\nThree")];
    let html = post_page(&items).expect("the page");
    assert!(html.contains("<p>One **two** &lt;b&gt;x&lt;/b&gt;<br>line</p>"), "{html}");
    assert!(html.contains("<p>Three</p>"));
    assert!(!html.contains("<strong>"));
}

#[test]
fn a_posts_document_is_offered_by_name_and_is_its_items_bytes() {
    let items = vec![post_row("Seed swap", "b", None), piece(0, "b"), document("JVBERi0xLjQK", "application/pdf", "inline")];
    let html = post_page(&items).expect("the page");
    assert!(html.contains(&format!("href=\"/allotments/post/{PID}/document\"")), "{html}");
    assert!(html.contains(">rota.pdf<"));
    let c = ctx("allotments", &[], &items, false);
    assert_eq!(post_document(&c, PID), Some(Document { data: "JVBERi0xLjQK".into(), mime: "application/pdf".into(), name: "rota.pdf".into() }));
    for bad in [document("", "application/pdf", "detached"), document("JVBERi0xLjQK", "text/html", "inline"), document("<html>", "application/pdf", "inline")] {
        let items = vec![post_row("Seed swap", "b", None), bad];
        let c = ctx("allotments", &[], &items, false);
        assert_eq!(post_document(&c, PID), None);
        assert!(!post_page(&items).unwrap().contains("/document\""), "no link to what is not served");
    }
}

/// The bodies the Resources editor writes, and a few typed by hand, drawn as the CommonMark
/// reference parser draws them (fixtures/commonmark.json, from the editor's own copy of it).
/// A post's picture is drawn in a figure where the reference draws a paragraph.
#[test]
fn the_editors_commonmark_draws_as_the_reference_parser_does() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("../fixtures/commonmark.json")).expect("the fixture");
    let norm = |s: &str| {
        let s = s.replace(" />", ">").replace("&#39;", "'").replace("<figure>", "<p>").replace("</figure>", "</p>");
        let s: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
        s.replace("> <", "><").trim().to_string()
    };
    let asset = |id: &str| Some(markdown::Picture { src: format!("asset:{id}"), width: 0, height: 0 });
    let mut bad = Vec::new();
    for c in cases.iter().filter(|c| !c["icd"].as_bool().unwrap_or(false)) {
        let text = c["text"].as_str().unwrap();
        let (got, want) = (norm(&markdown::render(text, &asset)), norm(c["html"].as_str().unwrap()));
        if got != want {
            bad.push(format!("{text:?}\n   drew {got}\n   want {want}"));
        }
    }
    assert!(bad.is_empty(), "{} of {} differ:\n{}", bad.len(), cases.len(), bad.join("\n"));
}

/// Where the ICD's rule is not the reference renderer's: raw HTML is text, a link is http(s)
/// or mailto or it is text, an image is the post's own or a link to it.
#[test]
fn where_the_icd_rules_a_body_is_text_or_a_link_never_markup() {
    let none = |_: &str| None;
    let r = |t: &str| markdown::render(t, &none);
    assert_eq!(r("<script>alert(1)</script>").trim(), "<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>");
    assert_eq!(r("para <b>x</b>").trim(), "<p>para &lt;b&gt;x&lt;/b&gt;</p>");
    assert_eq!(
        r("[js](javascript:alert(1)) and ![far](https://x.org/c.png) and [empty]()").trim(),
        "<p>js and <a href=\"https://x.org/c.png\">far</a> and empty</p>"
    );
}

/// A post's page wears the Face's mark and the Face's footer. The mark by the Face's own URL,
/// an absolute path, so one segment deeper (/<slug>/post/<id>) it is the same picture; where
/// the Host carries no mark, no picture at all, broken or otherwise. The footer the Face's,
/// byte for byte.
#[test]
fn a_posts_page_wears_the_faces_mark_and_footer() {
    let f = parse(&bundle(default_face()), "Host").unwrap();
    let items = vec![post_row("Seed swap", "b", None), piece(0, "b")];
    let face = render_face(&f, &ctx("allotments", &["mark"], &items, false));
    let page = render_post(&f, &ctx("allotments", &["mark"], &items, false), PID).unwrap();
    let marks = |h: &str| -> Vec<String> {
        h.split("<img ").skip(1).filter_map(|t| t.split("src=\"").nth(1)).map(|s| s.split('"').next().unwrap_or("").to_string()).collect()
    };
    assert_eq!(marks(&page), vec!["/allotments/m/mark".to_string()], "the Face's mark, and only it");
    assert!(marks(&face).contains(&"/allotments/m/mark".to_string()), "as the Face draws it");
    let bare = render_post(&f, &ctx("allotments", &[], &items, false), PID).unwrap();
    assert!(!bare.contains("<img"), "no mark on the Host: no picture");
    assert!(bare.contains("wf-initials"), "the Face's initials in its place");
    let foot = |h: &str| h[h.find("<p class=\"wf-foot\">").expect("a footer")..].split("</p>").next().unwrap().to_string();
    assert_eq!(foot(&page), foot(&face));
}
