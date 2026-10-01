//! Escaping, and the few kinds of string a face may put into a page.

/// Text, escaped for an element's body or a double-quoted attribute.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// A link a visitor may follow: `https:`, `http:` or `mailto:`, and nothing else.
/// Anything else — `javascript:`, `data:`, a relative path, a bare word — is `None`,
/// and the widget that carried it is not drawn as a link.
pub fn safe_href(url: &str) -> Option<String> {
    let u = url.trim();
    let lower = u.to_ascii_lowercase();
    let ok = lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:");
    if !ok || u.chars().any(|c| c.is_control() || c == ' ') {
        return None;
    }
    Some(u.to_string())
}

/// `#RRGGBB`, the only colour form the editor stores. Anything else is refused, so
/// no string from a log ever reaches a style attribute unchecked.
pub fn is_hex_colour(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 7 && b[0] == b'#' && b[1..].iter().all(|c| c.is_ascii_hexdigit())
}

pub fn rgb(hex: &str) -> [u8; 3] {
    let p = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
    [p(1), p(3), p(5)]
}

pub fn rgba(hex: &str, a: f64) -> String {
    let [r, g, b] = rgb(hex);
    format!("rgba({r},{g},{b},{})", trim_float(a))
}

/// face.js's `luminance` — WCAG relative luminance.
pub fn luminance(hex: &str) -> f64 {
    let l: Vec<f64> = rgb(hex)
        .iter()
        .map(|v| {
            let c = *v as f64 / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        })
        .collect();
    0.2126 * l[0] + 0.7152 * l[1] + 0.0722 * l[2]
}

fn contrast(a: &str, b: &str) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// face.js's `onColour`: ink or white on a colour, whichever reads better.
pub fn on_colour(hex: &str) -> &'static str {
    if contrast(hex, crate::look::INK) >= contrast(hex, crate::look::WHITE) {
        crate::look::INK
    } else {
        crate::look::WHITE
    }
}

/// A number as CSS wants it: no trailing zeros, no exponent.
pub fn trim_float(v: f64) -> String {
    let s = format!("{:.4}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" {
        "0".into()
    } else {
        s.to_string()
    }
}

/// Percent-encode for a `data:` URL, as `encodeURIComponent` does.
pub fn uri_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
