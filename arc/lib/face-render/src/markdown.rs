//! A post's `markdown` body, drawn for its public page (W-98 Resources). post.setProfile's
//! bodyFormat `markdown` is "CommonMark 0.31.2; raw HTML escaped; images only `asset:<id>` of
//! this post" (the ICD). What the Resources editor writes (app/web/resources/markdown.js) is
//! drawn as the reference parser draws it, held so by tests.rs over fixtures/commonmark.json:
//! paragraphs, ATX and setext headings, fenced and indented code, rules, quotes, lists tight
//! and loose, emphasis by CommonMark's delimiter rules, code spans, inline links and images,
//! backslash escapes and hard breaks. Beyond that (link reference definitions, entities,
//! autolinks, raw HTML) the text is drawn as the text it is.
//!
//! Everything is escaped. A link is http(s) or mailto or it is its text; an image is one of
//! the post's own pictures, drawn from its item, or a link to where it is: never fetched.

use crate::html::{esc, safe_href};

/// One of the post's own pictures, as the page draws it.
pub struct Picture {
    pub src: String,
    pub width: u32,
    pub height: u32,
}

type Assets<'a> = &'a dyn Fn(&str) -> Option<Picture>;

/// `text` as block elements.
pub fn render(text: &str, asset: Assets) -> String {
    let lines = prepare(text);
    let mut out = String::new();
    for b in blocks(&lines).0 {
        block(&mut out, &b, asset, false);
    }
    out
}

// ---- blocks ------------------------------------------------------------------------------

enum Block {
    Para(String),
    Heading(usize, String),
    Code(String, String),
    Rule,
    Quote(Vec<Block>),
    List { ordered: bool, start: u64, tight: bool, items: Vec<Vec<Block>> },
}

/// Lines, tabs expanded to the next multiple of four columns.
fn prepare(text: &str) -> Vec<String> {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .map(|l| {
            let mut s = String::with_capacity(l.len());
            for c in l.chars() {
                if c == '\t' {
                    let n = 4 - s.chars().count() % 4;
                    s.extend(std::iter::repeat(' ').take(n));
                } else {
                    s.push(c);
                }
            }
            s
        })
        .collect()
}

fn blank(l: &str) -> bool {
    l.trim().is_empty()
}
fn indent(l: &str) -> usize {
    l.chars().take_while(|c| *c == ' ').count()
}
/// `l` less up to `n` leading spaces.
fn strip(l: &str, n: usize) -> String {
    let k = indent(l).min(n);
    l[k..].to_string()
}

/// An opening fence: its indent, its character, its length, its info string.
fn fence(l: &str) -> Option<(usize, char, usize, String)> {
    let ind = indent(l);
    if ind > 3 {
        return None;
    }
    let rest = &l[ind..];
    let ch = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = rest.chars().take_while(|c| *c == ch).count();
    if n < 3 {
        return None;
    }
    let info = rest[n..].trim().to_string();
    if ch == '`' && info.contains('`') {
        return None;
    }
    Some((ind, ch, n, info))
}
fn closes(l: &str, ch: char, n: usize) -> bool {
    let ind = indent(l);
    if ind > 3 {
        return false;
    }
    let rest = &l[ind..];
    let k = rest.chars().take_while(|c| *c == ch).count();
    k >= n && rest[k..].trim().is_empty()
}

fn atx(l: &str) -> Option<(usize, String)> {
    let ind = indent(l);
    if ind > 3 {
        return None;
    }
    let rest = &l[ind..];
    let n = rest.chars().take_while(|c| *c == '#').count();
    if n == 0 || n > 6 {
        return None;
    }
    let after = &rest[n..];
    if !after.is_empty() && !after.starts_with(' ') {
        return None;
    }
    let mut t = after.trim().to_string();
    // A closing sequence: #s at the end, after a space, or the whole of it.
    let hashes = t.chars().rev().take_while(|c| *c == '#').count();
    if hashes > 0 {
        let head = &t[..t.len() - hashes];
        if head.is_empty() {
            t = String::new();
        } else if head.ends_with(' ') {
            t = head.trim_end().to_string();
        }
    }
    Some((n, t))
}

fn rule(l: &str) -> bool {
    if indent(l) > 3 {
        return false;
    }
    let t: Vec<char> = l.chars().filter(|c| *c != ' ').collect();
    t.len() >= 3 && matches!(t[0], '-' | '*' | '_') && t.iter().all(|c| *c == t[0])
}

fn setext(l: &str) -> Option<usize> {
    if indent(l) > 3 {
        return None;
    }
    let t = l.trim();
    if !t.is_empty() && t.chars().all(|c| c == '=') {
        Some(1)
    } else if !t.is_empty() && t.chars().all(|c| c == '-') {
        Some(2)
    } else {
        None
    }
}

fn quoted(l: &str) -> Option<String> {
    let ind = indent(l);
    if ind > 3 || !l[ind..].starts_with('>') {
        return None;
    }
    let rest = &l[ind + 1..];
    Some(rest.strip_prefix(' ').unwrap_or(rest).to_string())
}

/// A list item's marker.
struct Marker {
    ordered: bool,
    /// the bullet, or the ordered list's delimiter
    ch: char,
    start: u64,
    /// the column its content starts at
    content: usize,
    rest: String,
}

fn marker(l: &str) -> Option<Marker> {
    let ind = indent(l);
    if ind > 3 {
        return None;
    }
    let rest = &l[ind..];
    let first = rest.chars().next()?;
    let (ordered, ch, start, width) = if matches!(first, '-' | '*' | '+') {
        (false, first, 0, 1)
    } else {
        let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 || digits > 9 {
            return None;
        }
        let d = rest[digits..].chars().next()?;
        if d != '.' && d != ')' {
            return None;
        }
        (true, d, rest[..digits].parse().ok()?, digits + 1)
    };
    let after = &rest[width..];
    if !after.is_empty() && !after.starts_with(' ') {
        return None;
    }
    let spaces = indent(after);
    let text = after.trim_start();
    let pad = if text.is_empty() || spaces > 4 { 1 } else { spaces };
    let body = if text.is_empty() { String::new() } else if spaces > 4 { after[1..].to_string() } else { text.to_string() };
    Some(Marker { ordered, ch, start, content: ind + width + pad, rest: body })
}

/// Whether `l` begins a block that ends a paragraph above it.
fn interrupts(l: &str) -> bool {
    fence(l).is_some()
        || atx(l).is_some()
        || rule(l)
        || quoted(l).is_some()
        || marker(l).is_some_and(|m| !m.rest.trim().is_empty() && (!m.ordered || m.start == 1))
}

/// The blocks of `lines`, and whether a blank line stood between two of them.
fn blocks(lines: &[String]) -> (Vec<Block>, bool) {
    let mut out = Vec::new();
    let (mut gap, mut seen_blank) = (false, false);
    let mut i = 0;
    while i < lines.len() {
        let l = &lines[i];
        if blank(l) {
            seen_blank = true;
            i += 1;
            continue;
        }
        if seen_blank && !out.is_empty() {
            gap = true;
        }
        seen_blank = false;
        if indent(l) >= 4 {
            let mut code = Vec::new();
            let mut j = i;
            while j < lines.len() && (blank(&lines[j]) || indent(&lines[j]) >= 4) {
                code.push(strip(&lines[j], 4));
                j += 1;
            }
            while code.last().is_some_and(|c| c.trim().is_empty()) {
                code.pop();
            }
            out.push(Block::Code(String::new(), code.join("\n") + "\n"));
            i = j;
            continue;
        }
        if let Some((fi, ch, n, info)) = fence(l) {
            let mut code = Vec::new();
            let mut j = i + 1;
            while j < lines.len() && !closes(&lines[j], ch, n) {
                code.push(strip(&lines[j], fi));
                j += 1;
            }
            let body = if code.is_empty() { String::new() } else { code.join("\n") + "\n" };
            out.push(Block::Code(info, body));
            i = j + 1;
            continue;
        }
        if let Some((n, t)) = atx(l) {
            out.push(Block::Heading(n, t));
            i += 1;
            continue;
        }
        if rule(l) {
            out.push(Block::Rule);
            i += 1;
            continue;
        }
        if quoted(l).is_some() {
            let mut inner: Vec<String> = Vec::new();
            let mut j = i;
            while j < lines.len() {
                if let Some(rest) = quoted(&lines[j]) {
                    inner.push(rest);
                } else if !blank(&lines[j]) && !interrupts(&lines[j]) && inner.last().is_some_and(|x| !blank(x) && lazy_after(x)) {
                    inner.push(lines[j].clone());
                } else {
                    break;
                }
                j += 1;
            }
            out.push(Block::Quote(blocks(&inner).0));
            i = j;
            continue;
        }
        if let Some(first) = marker(l) {
            let (list, next) = list(lines, i, &first);
            out.push(list);
            i = next;
            continue;
        }
        let mut para = vec![l.trim_start().to_string()];
        let mut j = i + 1;
        let mut heading = None;
        while j < lines.len() {
            let lj = &lines[j];
            if blank(lj) {
                break;
            }
            if let Some(level) = setext(lj) {
                heading = Some(level);
                j += 1;
                break;
            }
            if interrupts(lj) {
                break;
            }
            para.push(lj.trim_start().to_string());
            j += 1;
        }
        let text = para.join("\n").trim_end().to_string();
        out.push(match heading {
            Some(level) => Block::Heading(level, text),
            None => Block::Para(text),
        });
        i = j;
    }
    (out, gap)
}

/// Whether a line may be continued lazily after `x`: `x` is paragraph text, not a block's start.
fn lazy_after(x: &str) -> bool {
    fence(x).is_none() && atx(x).is_none() && !rule(x) && indent(x) < 4
}

/// The list starting at `lines[i]`, and the line after it.
fn list(lines: &[String], mut i: usize, first: &Marker) -> (Block, usize) {
    let same = |m: &Marker| m.ordered == first.ordered && m.ch == first.ch;
    let mut items = Vec::new();
    let mut tight = true;
    while i < lines.len() {
        let Some(m) = marker(&lines[i]).filter(|m| same(m)) else { break };
        let mut item = vec![m.rest.clone()];
        let mut j = i + 1;
        while j < lines.len() {
            let lj = &lines[j];
            if blank(lj) {
                item.push(String::new());
            } else if indent(lj) >= m.content {
                item.push(strip(lj, m.content));
            } else if item.last().is_some_and(|x| !blank(x) && lazy_after(x)) && !interrupts(lj) && marker(lj).is_none() {
                item.push(lj.trim_start().to_string());
            } else {
                break;
            }
            j += 1;
        }
        let mut trailing = 0;
        while item.len() > 1 && item.last().is_some_and(|x| blank(x)) {
            item.pop();
            trailing += 1;
        }
        let (kids, gap) = blocks(&item);
        if gap {
            tight = false;
        }
        items.push(kids);
        i = j;
        if trailing > 0 && marker(lines.get(j).map(String::as_str).unwrap_or("")).is_some_and(|m| same(&m)) {
            tight = false;
        } else if trailing > 0 {
            i = j - trailing;
            break;
        }
    }
    (Block::List { ordered: first.ordered, start: first.start, tight, items }, i)
}

fn block(out: &mut String, b: &Block, asset: Assets, tight: bool) {
    match b {
        Block::Para(t) => {
            let nodes = inlines(t);
            let solid: Vec<&Inl> = nodes.iter().filter(|n| !matches!(n, Inl::Text(s) if s.trim().is_empty())).collect();
            if let [Inl::Img(dest, title, kids)] = solid.as_slice() {
                if let Some(pic) = asset_id(dest).and_then(|id| asset(id)) {
                    out.push_str("<figure>");
                    img(out, &pic, &alt(kids), title.as_deref());
                    out.push_str("</figure>\n");
                    return;
                }
            }
            if !tight {
                out.push_str("<p>");
            }
            inline(out, &nodes, asset);
            out.push_str(if tight { "" } else { "</p>\n" });
        }
        Block::Heading(n, t) => {
            out.push_str(&format!("<h{n}>"));
            inline(out, &inlines(t), asset);
            out.push_str(&format!("</h{n}>\n"));
        }
        Block::Code(info, code) => {
            let lang = info.split_whitespace().next().unwrap_or("");
            if lang.is_empty() {
                out.push_str("<pre><code>");
            } else {
                out.push_str(&format!("<pre><code class=\"language-{}\">", esc(&unescape(lang))));
            }
            out.push_str(&esc(code));
            out.push_str("</code></pre>\n");
        }
        Block::Rule => out.push_str("<hr>\n"),
        Block::Quote(kids) => {
            out.push_str("<blockquote>\n");
            for k in kids {
                block(out, k, asset, false);
            }
            out.push_str("</blockquote>\n");
        }
        Block::List { ordered, start, tight, items } => {
            match (ordered, start) {
                (true, 1) => out.push_str("<ol>\n"),
                (true, n) => out.push_str(&format!("<ol start=\"{n}\">\n")),
                _ => out.push_str("<ul>\n"),
            }
            for item in items {
                out.push_str("<li>");
                for (k, b) in item.iter().enumerate() {
                    if *tight && k > 0 {
                        out.push('\n');
                    }
                    block(out, b, asset, *tight);
                }
                out.push_str("</li>\n");
            }
            out.push_str(if *ordered { "</ol>\n" } else { "</ul>\n" });
        }
    }
}

// ---- inlines -----------------------------------------------------------------------------

#[derive(Clone)]
enum Inl {
    Text(String),
    Code(String),
    Soft,
    Hard,
    Em(Vec<Inl>),
    Strong(Vec<Inl>),
    Link(String, Option<String>, Vec<Inl>),
    Img(String, Option<String>, Vec<Inl>),
}

/// Inline content while it is parsed: nodes, emphasis delimiter runs, and link openers.
enum Item {
    N(Inl),
    D { ch: char, n: usize, orig: usize, open: bool, close: bool },
    B { image: bool, active: bool },
}

fn is_punct(c: char) -> bool {
    c.is_ascii_punctuation() || (!c.is_ascii() && !c.is_alphanumeric() && !c.is_whitespace())
}

fn inlines(s: &str) -> Vec<Inl> {
    let cs: Vec<char> = s.chars().collect();
    let mut items: Vec<Item> = Vec::new();
    let mut text = String::new();
    fn flush(items: &mut Vec<Item>, text: &mut String) {
        if !text.is_empty() {
            items.push(Item::N(Inl::Text(std::mem::take(text))));
        }
    }
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        match c {
            '\\' => {
                match cs.get(i + 1) {
                    Some('\n') => {
                        flush(&mut items, &mut text);
                        items.push(Item::N(Inl::Hard));
                        i += 2;
                        while cs.get(i) == Some(&' ') {
                            i += 1;
                        }
                    }
                    Some(n) if n.is_ascii_punctuation() => {
                        text.push(*n);
                        i += 2;
                    }
                    _ => {
                        text.push('\\');
                        i += 1;
                    }
                }
            }
            '`' => {
                let n = cs[i..].iter().take_while(|c| **c == '`').count();
                let mut j = i + n;
                let mut found = None;
                while j < cs.len() {
                    if cs[j] == '`' {
                        let m = cs[j..].iter().take_while(|c| **c == '`').count();
                        if m == n {
                            found = Some(j);
                            break;
                        }
                        j += m;
                    } else {
                        j += 1;
                    }
                }
                match found {
                    Some(j) => {
                        let mut code: String = cs[i + n..j].iter().map(|c| if *c == '\n' { ' ' } else { *c }).collect();
                        if code.len() >= 2 && code.starts_with(' ') && code.ends_with(' ') && !code.trim().is_empty() {
                            code = code[1..code.len() - 1].to_string();
                        }
                        flush(&mut items, &mut text);
                        items.push(Item::N(Inl::Code(code)));
                        i = j + n;
                    }
                    None => {
                        text.extend(std::iter::repeat('`').take(n));
                        i += n;
                    }
                }
            }
            '*' | '_' => {
                let n = cs[i..].iter().take_while(|x| **x == c).count();
                let prev = if i == 0 { ' ' } else { cs[i - 1] };
                let next = cs.get(i + n).copied().unwrap_or(' ');
                let left = !next.is_whitespace() && (!is_punct(next) || prev.is_whitespace() || is_punct(prev));
                let right = !prev.is_whitespace() && (!is_punct(prev) || next.is_whitespace() || is_punct(next));
                let (open, close) = if c == '*' {
                    (left, right)
                } else {
                    (left && (!right || is_punct(prev)), right && (!left || is_punct(next)))
                };
                flush(&mut items, &mut text);
                items.push(Item::D { ch: c, n, orig: n, open, close });
                i += n;
            }
            '!' if cs.get(i + 1) == Some(&'[') => {
                flush(&mut items, &mut text);
                items.push(Item::B { image: true, active: true });
                i += 2;
            }
            '[' => {
                flush(&mut items, &mut text);
                items.push(Item::B { image: false, active: true });
                i += 1;
            }
            ']' => {
                flush(&mut items, &mut text);
                let Some(b) = items.iter().rposition(|x| matches!(x, Item::B { .. })) else {
                    text.push(']');
                    i += 1;
                    continue;
                };
                let (image, active) = match items[b] {
                    Item::B { image, active } => (image, active),
                    _ => unreachable!(),
                };
                let opener = if image { "![" } else { "[" };
                let tail = if active { link_tail(&cs, i + 1) } else { None };
                match tail {
                    Some((dest, title, end)) => {
                        let mut kids: Vec<Item> = items.drain(b + 1..).collect();
                        emphasis(&mut kids);
                        items.pop();
                        let kids = finish(kids);
                        items.push(Item::N(if image { Inl::Img(dest, title, kids) } else { Inl::Link(dest, title, kids) }));
                        if !image {
                            for x in items.iter_mut() {
                                if let Item::B { image: false, active } = x {
                                    *active = false;
                                }
                            }
                        }
                        i = end;
                    }
                    None => {
                        items[b] = Item::N(Inl::Text(opener.into()));
                        text.push(']');
                        i += 1;
                    }
                }
            }
            '\n' => {
                let spaces = text.chars().rev().take_while(|c| *c == ' ').count();
                let hard = spaces >= 2;
                let keep = text.trim_end_matches(' ').len();
                text.truncate(keep);
                flush(&mut items, &mut text);
                items.push(Item::N(if hard { Inl::Hard } else { Inl::Soft }));
                i += 1;
                while cs.get(i) == Some(&' ') {
                    i += 1;
                }
            }
            _ => {
                text.push(c);
                i += 1;
            }
        }
    }
    flush(&mut items, &mut text);
    emphasis(&mut items);
    finish(items)
}

/// CommonMark's "process emphasis": each closer, left to right, matched with the nearest
/// opener of its kind that the rule of three allows.
fn emphasis(items: &mut Vec<Item>) {
    let mut ci = 0;
    loop {
        let Some(c) = (ci..items.len()).find(|&k| matches!(items[k], Item::D { close: true, n, .. } if n > 0)) else { break };
        let (cch, cn, corig, copen) = match items[c] {
            Item::D { ch, n, orig, open, .. } => (ch, n, orig, open),
            _ => unreachable!(),
        };
        let mut found = None;
        for o in (0..c).rev() {
            if let Item::D { ch, n, orig, open: true, close } = items[o] {
                if ch == cch && n > 0 {
                    let odd = (close || copen) && (orig + corig) % 3 == 0 && !(orig % 3 == 0 && corig % 3 == 0);
                    if !odd {
                        found = Some(o);
                        break;
                    }
                }
            }
        }
        let Some(o) = found else {
            if let Item::D { close, .. } = &mut items[c] {
                *close = false;
            }
            ci = c + 1;
            continue;
        };
        let on = match items[o] {
            Item::D { n, .. } => n,
            _ => unreachable!(),
        };
        let used = if cn >= 2 && on >= 2 { 2 } else { 1 };
        let kids = finish(items.drain(o + 1..c).collect());
        items.insert(o + 1, Item::N(if used == 2 { Inl::Strong(kids) } else { Inl::Em(kids) }));
        for k in [o, o + 2] {
            if let Item::D { n, .. } = &mut items[k] {
                *n -= used;
            }
        }
        let mut cpos = o + 2;
        if matches!(items[o], Item::D { n: 0, .. }) {
            items.remove(o);
            cpos -= 1;
        }
        if matches!(items[cpos], Item::D { n: 0, .. }) {
            items.remove(cpos);
        }
        ci = cpos;
    }
}

/// Items as nodes: what no rule matched is the text it was.
fn finish(items: Vec<Item>) -> Vec<Inl> {
    items
        .into_iter()
        .filter_map(|x| match x {
            Item::N(n) => Some(n),
            Item::D { ch, n, .. } if n > 0 => Some(Inl::Text(std::iter::repeat(ch).take(n).collect())),
            Item::D { .. } => None,
            Item::B { image, .. } => Some(Inl::Text(if image { "![" } else { "[" }.into())),
        })
        .collect()
}

/// An inline link's `(destination "title")` at `cs[p]`: the destination, the title, and
/// where it ends.
fn link_tail(cs: &[char], mut p: usize) -> Option<(String, Option<String>, usize)> {
    if cs.get(p) != Some(&'(') {
        return None;
    }
    p += 1;
    let ws = |p: &mut usize| {
        let mut nl = 0;
        while let Some(c) = cs.get(*p) {
            if *c == ' ' {
                *p += 1;
            } else if *c == '\n' && nl == 0 {
                nl += 1;
                *p += 1;
            } else {
                break;
            }
        }
    };
    ws(&mut p);
    let mut dest = String::new();
    if cs.get(p) == Some(&'<') {
        p += 1;
        loop {
            match cs.get(p)? {
                '>' => {
                    p += 1;
                    break;
                }
                '\n' | '<' => return None,
                '\\' if cs.get(p + 1).is_some_and(|c| c.is_ascii_punctuation()) => {
                    dest.push(cs[p + 1]);
                    p += 2;
                }
                c => {
                    dest.push(*c);
                    p += 1;
                }
            }
        }
    } else {
        let mut depth = 0usize;
        while let Some(&c) = cs.get(p) {
            if c == '\\' && cs.get(p + 1).is_some_and(|c| c.is_ascii_punctuation()) {
                dest.push(cs[p + 1]);
                p += 2;
                continue;
            }
            if c.is_whitespace() || c.is_control() {
                break;
            }
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            dest.push(c);
            p += 1;
        }
        if depth != 0 {
            return None;
        }
    }
    let before = p;
    ws(&mut p);
    let mut title = None;
    if p > before {
        if let Some(&q) = cs.get(p).filter(|c| matches!(c, '"' | '\'' | '(')) {
            let end = if q == '(' { ')' } else { q };
            let mut t = String::new();
            let mut k = p + 1;
            loop {
                match cs.get(k)? {
                    c if *c == end => {
                        k += 1;
                        break;
                    }
                    '\\' if cs.get(k + 1).is_some_and(|c| c.is_ascii_punctuation()) => {
                        t.push(cs[k + 1]);
                        k += 2;
                    }
                    c => {
                        t.push(*c);
                        k += 1;
                    }
                }
            }
            title = Some(t);
            p = k;
            ws(&mut p);
        }
    }
    if cs.get(p) != Some(&')') {
        return None;
    }
    Some((dest, title, p + 1))
}

/// A destination as the reference parser writes it: what is not already a URL's own
/// character, percent-encoded.
fn normal(dest: &str) -> String {
    let b = dest.as_bytes();
    let mut out = String::with_capacity(dest.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'%' && b.get(i + 1).is_some_and(u8::is_ascii_hexdigit) && b.get(i + 2).is_some_and(u8::is_ascii_hexdigit) {
            out.push('%');
            i += 1;
            continue;
        }
        if c.is_ascii_alphanumeric() || b";/?:@&=+$,-_.!~*'()#".contains(&c) {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
        i += 1;
    }
    out
}

fn unescape(s: &str) -> String {
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '\\' && cs.get(i + 1).is_some_and(|c| c.is_ascii_punctuation()) {
            out.push(cs[i + 1]);
            i += 2;
        } else {
            out.push(cs[i]);
            i += 1;
        }
    }
    out
}

/// `asset:<16 hex>`'s id.
fn asset_id(dest: &str) -> Option<&str> {
    dest.strip_prefix("asset:").filter(|id| id.len() == 16 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// An image's alt: its content's text.
fn alt(nodes: &[Inl]) -> String {
    let mut s = String::new();
    for n in nodes {
        match n {
            Inl::Text(t) | Inl::Code(t) => s.push_str(t),
            Inl::Soft | Inl::Hard => s.push(' '),
            Inl::Em(k) | Inl::Strong(k) | Inl::Link(_, _, k) | Inl::Img(_, _, k) => s.push_str(&alt(k)),
        }
    }
    s
}

fn img(out: &mut String, p: &Picture, alt: &str, title: Option<&str>) {
    out.push_str(&format!("<img src=\"{}\" alt=\"{}\"", esc(&p.src), esc(alt)));
    if let Some(t) = title {
        out.push_str(&format!(" title=\"{}\"", esc(t)));
    }
    if p.width > 0 && p.height > 0 {
        out.push_str(&format!(" width=\"{}\" height=\"{}\"", p.width, p.height));
    }
    out.push('>');
}

fn inline(out: &mut String, nodes: &[Inl], asset: Assets) {
    for n in nodes {
        match n {
            Inl::Text(t) => out.push_str(&esc(t)),
            Inl::Code(t) => out.push_str(&format!("<code>{}</code>", esc(t))),
            Inl::Soft => out.push('\n'),
            Inl::Hard => out.push_str("<br>\n"),
            Inl::Em(k) => {
                out.push_str("<em>");
                inline(out, k, asset);
                out.push_str("</em>");
            }
            Inl::Strong(k) => {
                out.push_str("<strong>");
                inline(out, k, asset);
                out.push_str("</strong>");
            }
            Inl::Link(dest, title, k) => match safe_href(&normal(dest)) {
                Some(href) => {
                    out.push_str(&format!("<a href=\"{}\"", esc(&href)));
                    if let Some(t) = title {
                        out.push_str(&format!(" title=\"{}\"", esc(t)));
                    }
                    out.push('>');
                    inline(out, k, asset);
                    out.push_str("</a>");
                }
                None => inline(out, k, asset),
            },
            Inl::Img(dest, title, k) => {
                let a = alt(k);
                if let Some(pic) = asset_id(dest).and_then(|id| asset(id)) {
                    img(out, &pic, &a, title.as_deref());
                } else if asset_id(dest).is_some() {
                    out.push_str(&esc(&a));
                } else if let Some(href) = safe_href(&normal(dest)) {
                    // An image from elsewhere is a link to it, never fetched: a reader's
                    // browser asking a member's URL would be a beacon.
                    out.push_str(&format!("<a href=\"{}\">{}</a>", esc(&href), esc(if a.is_empty() { dest } else { &a })));
                } else {
                    out.push_str(&esc(&a));
                }
            }
        }
    }
}
