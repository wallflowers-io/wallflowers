//! vcard — RFC 6350 (.vcf) import/export for the Group identity record.
//!
//! Lives in the CORE (not Swift) so the CLI and the app share ONE parser and ONE
//! field→Delta mapping — importing a card MINTS/UPDATES a Group by authoring its
//! setProfile/setPresence Deltas (node.rs), never a private struct. A .vcf is an
//! interchange projection, NEVER authoritative: MEMBER lines are DERIVED from the
//! MLS roster on export and ADVISORY on import (they never grant access), and the
//! credential vault is NEVER serialised.
//!
//! Anchors: UID = urn:pacific:identity:<IdentityKey> (on-platform) or
//! urn:pacific:local:<group_id> (off-platform); the reusable reference key — the
//! group object id — travels as X-PACIFIC-GROUP so a re-import re-homes to the
//! same object even before any space exists.

use crate::group::{ContactCard, GroupShape, GroupState, Labeled, Presence};
use crate::identity::parse_identity_key;
use crate::object::MemberId;

/// Everything an imported card yields — the node turns this into setProfile/
/// setPresence Deltas on a new or matched Group.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedCard {
    pub display_name: String,
    pub shape: GroupShape,
    pub presence: Presence,
    pub card: ContactCard,
    pub uid: String,
    /// X-PACIFIC-GROUP if present — the object id to re-home to (else None → new).
    pub group_ref: Option<String>,
    /// MEMBER UIDs — advisory intended membership, NEVER an access grant.
    pub member_uids: Vec<String>,
}

#[derive(Debug)]
pub enum VcardError {
    /// Not parseable as a vCard (no BEGIN/END, or no FN).
    Malformed(String),
    /// A valid card but not a Group — KIND:location/application/device. Loud, not
    /// coerced into an individual.
    NotAGroup(String),
}

impl std::fmt::Display for VcardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VcardError::Malformed(m) => write!(f, "malformed vCard: {m}"),
            VcardError::NotAGroup(k) => write!(f, "vCard KIND:{k} is not a Group"),
        }
    }
}
impl std::error::Error for VcardError {}

// ================================ EXPORT ======================================

/// Serialise a Group to a vCard 4.0 string. `roster` (the MLS membership, the
/// source of truth) supplies the MEMBER lines for a team/org.
pub fn group_to_vcard(state: &GroupState, group_id_hex: &str, roster: &[MemberId]) -> String {
    let mut out: Vec<String> = Vec::new();
    out.push("BEGIN:VCARD".into());
    out.push("VERSION:4.0".into());

    let kind = match state.shape {
        GroupShape::Individual => "individual",
        GroupShape::Team => "group",
        GroupShape::Organisation => "org",
        // vCard 4.0's KIND vocabulary has no "community"; a body of people is a group.
        GroupShape::Community => "group",
    };
    out.push(format!("KIND:{kind}"));

    // UID: the identity anchor. On-platform → the space; else the local object id.
    let uid = match state.presence.identity_key() {
        Some(sid) => format!("urn:pacific:identity:{sid}"),
        None => format!("urn:pacific:local:{group_id_hex}"),
    };
    out.push(format!("UID:{uid}"));
    // The reusable reference key other objects store.
    out.push(format!("X-PACIFIC-GROUP:{group_id_hex}"));

    out.push(format!("FN:{}", esc(&state.display_name)));
    if state.shape == GroupShape::Individual {
        let (family, given) = split_name(&state.display_name);
        out.push(format!("N:{};{};;;", esc_comp(&family), esc_comp(&given)));
    }
    if !state.card.org.is_empty() {
        out.push(format!("ORG:{}", esc_comp(&state.card.org)));
    }
    if !state.card.title.is_empty() {
        out.push(format!("TITLE:{}", esc(&state.card.title)));
    }
    if let Some(sid) = state.presence.identity_key() {
        out.push(format!("IMPP:pacific:{sid}"));
    }
    if let Some(hint) = state.presence.invite_hint() {
        if !hint.is_empty() {
            out.push(format!("X-PACIFIC-INVITE:{}", esc(hint)));
        }
    }
    for (i, e) in state.card.emails.iter().enumerate() {
        let pref = if i == 0 { ";PREF=1" } else { "" };
        out.push(format!(
            "EMAIL;TYPE={}{}:{}",
            type_param(&e.label),
            pref,
            esc(&e.value)
        ));
    }
    for p in &state.card.phones {
        out.push(format!(
            "TEL;TYPE={};VALUE=uri:tel:{}",
            type_param(&p.label),
            esc(&p.value)
        ));
    }
    for u in &state.card.urls {
        out.push(format!("URL:{}", esc(&u.value)));
    }
    // PHOTO carries the STILL only. The `clip` slot ("last seen" motion) has no
    // vCard 4.0 representation, so it is deliberately NOT exported — a .vcf stays a
    // .vcf rather than growing a private X- field no other address book can read.
    if !state.card.photo.is_empty() {
        let mime = if state.card.photo_mime.is_empty() {
            "image/jpeg"
        } else {
            &state.card.photo_mime
        };
        out.push(format!("PHOTO:data:{};base64,{}", mime, state.card.photo));
    }
    if !state.card.note.is_empty() {
        out.push(format!("NOTE:{}", esc(&state.card.note)));
    }
    if !state.card.tags.is_empty() {
        let cats: Vec<String> = state.card.tags.iter().map(|t| esc(t)).collect();
        out.push(format!("CATEGORIES:{}", cats.join(",")));
    }
    // MEMBER only for a team/org, DERIVED from the roster (the MLS truth).
    if matches!(
        state.shape,
        GroupShape::Team | GroupShape::Organisation | GroupShape::Community
    ) {
        for m in roster {
            out.push(format!("MEMBER:urn:pacific:identity:{}{}", crate::identity::IDENTITY_KEY_PREFIX, hex::encode(m)));
        }
    }
    out.push("END:VCARD".into());

    out.into_iter()
        .map(|l| fold(&l))
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n"
}

// ================================ IMPORT ======================================

/// Parse a MULTI-card .vcf (an address-book export is many `BEGIN:VCARD..END`
/// blocks) into one `ParsedCard` per contact. Version-tolerant (2.1/3.0/4.0);
/// loud on a non-Group KIND in ANY card. This is the real import entrypoint — a
/// standard 2-contact export never silently collapses to one Group.
pub fn vcard_to_cards(text: &str) -> Result<Vec<ParsedCard>, VcardError> {
    let cards = split_cards(&unfold(text))?;
    if cards.is_empty() {
        return Err(VcardError::Malformed("no BEGIN:VCARD".into()));
    }
    cards.iter().map(|c| parse_one_card(c)).collect()
}

/// Parse a SINGLE-card .vcf. Loud if the text carries more than one card (use
/// [`vcard_to_cards`] for address-book input) — never a silent drop of N-1.
pub fn vcard_to_card(text: &str) -> Result<ParsedCard, VcardError> {
    let mut cards = vcard_to_cards(text)?;
    if cards.len() > 1 {
        return Err(VcardError::Malformed(format!(
            "{} vCards in one stream — use vcard_to_cards for multi-card input",
            cards.len()
        )));
    }
    Ok(cards.remove(0))
}

/// Parse the logical lines of ONE card (between BEGIN/END) into a `ParsedCard`.
fn parse_one_card(lines: &[String]) -> Result<ParsedCard, VcardError> {
    let mut pc = ParsedCard::default();
    let mut fn_name: Option<String> = None;
    let mut n_name: Option<String> = None;
    let mut org: Option<String> = None;
    let mut space: Option<String> = None; // candidate on-platform anchor (validated below)
    let mut invite_hint: Option<String> = None;

    for line in lines {
        let Some((name, params, value)) = split_line(line) else {
            continue;
        };
        match name.as_str() {
            "KIND" => {
                pc.shape = match value.to_ascii_lowercase().as_str() {
                    "individual" => GroupShape::Individual,
                    "group" => GroupShape::Team,
                    "org" => GroupShape::Organisation,
                    other @ ("location" | "application" | "device") => {
                        return Err(VcardError::NotAGroup(other.to_string()))
                    }
                    _ => GroupShape::Individual,
                };
            }
            "FN" => fn_name = Some(unesc(&value)),
            "N" => {
                // Family;Given;… → "Given Family" (split on UNESCAPED ';').
                let parts = split_components(&value);
                let family = parts.first().cloned().unwrap_or_default();
                let given = parts.get(1).cloned().unwrap_or_default();
                let joined = format!("{given} {family}").trim().to_string();
                if !joined.is_empty() {
                    n_name = Some(joined);
                }
            }
            "ORG" => {
                org = Some(
                    split_components(&value)
                        .into_iter()
                        .next()
                        .unwrap_or_default(),
                )
            }
            "TITLE" => pc.card.title = unesc(&value),
            "UID" => pc.uid = value.clone(),
            "IMPP" => {
                if let Some(s) = value.strip_prefix("pacific:") {
                    space = Some(s.to_string());
                }
            }
            "X-PACIFIC-GROUP" => pc.group_ref = Some(value.clone()),
            "X-PACIFIC-INVITE" => invite_hint = Some(unesc(&value)),
            "EMAIL" => pc.card.emails.push(Labeled {
                label: label_of(&params),
                value: unesc(&value),
            }),
            "TEL" => pc.card.phones.push(Labeled {
                label: label_of(&params),
                value: tel_value(&value),
            }),
            "URL" => pc.card.urls.push(Labeled {
                label: label_of(&params),
                value: unesc(&value),
            }),
            "PHOTO" => {
                if let Some(b64) = value.split("base64,").nth(1) {
                    pc.card.photo = b64.to_string(); // ignore external http PHOTO
                                                     // Keep the declared MIME so a re-export round-trips the real
                                                     // type instead of asserting image/jpeg over a PNG.
                    pc.card.photo_mime = value
                        .split("base64,")
                        .next()
                        .and_then(|p| p.strip_prefix("data:"))
                        .map(|m| m.trim_end_matches(';').to_string())
                        .filter(|m| !m.is_empty())
                        .unwrap_or_else(|| "image/jpeg".to_string());
                }
            }
            "NOTE" => pc.card.note = unesc(&value),
            "CATEGORIES" => pc.card.tags = split_commas(&value),
            "MEMBER" => pc.member_uids.push(value.clone()),
            _ => {}
        }
    }

    // UID → resolve the space anchor if the UID itself carries one.
    if space.is_none() {
        if let Some(sid) = pc.uid.strip_prefix("urn:pacific:identity:") {
            space = Some(sid.to_string());
        }
    }

    pc.display_name = fn_name
        .or(n_name)
        .or_else(|| org.clone())
        .ok_or_else(|| VcardError::Malformed("no FN/N/ORG for a display name".into()))?;
    if let Some(o) = org {
        pc.card.org = o;
        if pc.display_name.is_empty() {
            pc.display_name = pc.card.org.clone();
        }
    }
    // MEMBER under an individual is a malformed team — drop it, keep the contact.
    if pc.shape == GroupShape::Individual {
        pc.member_uids.clear();
    }
    // Presence is HONEST: on-platform ONLY if the anchor RESOLVES to a real 32-byte
    // IdentityKey (mapping spec §4). An untrusted card claiming `IMPP:pacific:hello`
    // never fabricates reachability — it falls through to off-platform.
    let real_space = space.filter(|s| parse_identity_key(s).is_ok());
    pc.presence = match real_space {
        Some(sid) => Presence::OnPlatform(sid),
        None => Presence::OffPlatform(
            invite_hint
                .or_else(|| pc.card.emails.first().map(|e| e.value.clone()))
                .or_else(|| pc.card.phones.first().map(|p| p.value.clone())),
        ),
    };
    Ok(pc)
}

/// Split unfolded logical lines into per-card line groups on `BEGIN:VCARD` /
/// `END:VCARD` boundaries. Lines outside a card are ignored. LOUD on a malformed
/// stream — an unterminated card, a nested BEGIN, or a stray END — rather than
/// silently dropping a contact (the no-silent-fallback rule; a truncated
/// address-book export must fail, not import N-1 as success).
fn split_cards(lines: &[String]) -> Result<Vec<Vec<String>>, VcardError> {
    let mut cards = Vec::new();
    let mut cur: Option<Vec<String>> = None;
    for l in lines {
        if l.eq_ignore_ascii_case("BEGIN:VCARD") {
            if cur.is_some() {
                return Err(VcardError::Malformed(
                    "BEGIN:VCARD inside an unclosed card".into(),
                ));
            }
            cur = Some(Vec::new());
        } else if l.eq_ignore_ascii_case("END:VCARD") {
            match cur.take() {
                Some(c) => cards.push(c),
                None => {
                    return Err(VcardError::Malformed(
                        "END:VCARD with no matching BEGIN".into(),
                    ))
                }
            }
        } else if let Some(c) = cur.as_mut() {
            c.push(l.clone());
        }
    }
    if cur.is_some() {
        return Err(VcardError::Malformed(
            "unterminated vCard (BEGIN without END)".into(),
        ));
    }
    Ok(cards)
}

// ------------------------------- helpers --------------------------------------

fn split_name(full: &str) -> (String, String) {
    match full.trim().rsplit_once(' ') {
        Some((given, family)) => (family.to_string(), given.to_string()),
        None => (full.trim().to_string(), String::new()),
    }
}

/// vCard TYPE param from a label (work/home/cell), defaulting to "work".
fn type_param(label: &str) -> &str {
    match label.to_ascii_lowercase().as_str() {
        "home" => "home",
        "cell" | "mobile" => "cell",
        _ => "work",
    }
}

fn label_of(params: &[(String, String)]) -> String {
    params
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("TYPE"))
        .map(|(_, v)| v.to_ascii_lowercase())
        .unwrap_or_else(|| "work".into())
}

fn tel_value(v: &str) -> String {
    // accept "tel:+1..." or a bare number
    unesc(v)
        .strip_prefix("tel:")
        .map(String::from)
        .unwrap_or_else(|| unesc(v))
}

/// Escape a simple text value (RFC 6350 3.4): backslash, comma, newline.
fn esc(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(',', "\\,")
}
/// Escape a compound COMPONENT (N/ADR/ORG): also the semicolon separator.
fn esc_comp(s: &str) -> String {
    esc(s).replace(';', "\\;")
}
fn unesc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}
/// Split a compound value (N, ADR, ORG) into its components on UNESCAPED `;`,
/// unescaping each. Mirrors `split_commas`: splitting BEFORE unescaping would
/// break at an escaped `\;` that `esc_comp` deliberately emitted (round-trip
/// data loss for an org/name containing a literal semicolon).
fn split_components(s: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut cur = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            // keep the escape pair intact so `unesc` resolves it (\\ , \; \n).
            cur.push('\\');
            if let Some(n) = it.next() {
                cur.push(n);
            }
        } else if c == ';' {
            items.push(unesc(&cur));
            cur.clear();
        } else {
            cur.push(c);
        }
    }
    items.push(unesc(&cur));
    items
}

/// Split a CATEGORIES value on UNESCAPED commas.
fn split_commas(s: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut cur = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' {
            if let Some(n) = it.next() {
                if n == 'n' || n == 'N' {
                    cur.push('\n');
                } else {
                    cur.push(n);
                }
            }
        } else if c == ',' {
            items.push(cur.trim().to_string());
            cur = String::new();
        } else {
            cur.push(c);
        }
    }
    if !cur.trim().is_empty() {
        items.push(cur.trim().to_string());
    }
    items.retain(|s| !s.is_empty());
    items
}

/// Fold a content line to ≤75 octets with CRLF + single-space continuations.
fn fold(line: &str) -> String {
    let bytes = line.as_bytes();
    if bytes.len() <= 75 {
        return line.to_string();
    }
    let mut out = String::new();
    let mut i = 0;
    let mut first = true;
    while i < bytes.len() {
        // 75 for the first chunk; 74 thereafter (the leading space counts).
        let limit = if first { 75 } else { 74 };
        let mut end = (i + limit).min(bytes.len());
        // don't split a UTF-8 multibyte sequence
        while end > i && (bytes[end.min(bytes.len() - 1)] & 0xC0) == 0x80 && end < bytes.len() {
            end -= 1;
        }
        let chunk = std::str::from_utf8(&bytes[i..end]).unwrap_or("");
        if first {
            out.push_str(chunk);
            first = false;
        } else {
            out.push_str("\r\n ");
            out.push_str(chunk);
        }
        i = end;
    }
    out
}

/// Unfold CRLF/LF continuation lines (a line starting with space/tab joins the
/// previous). Returns trimmed logical lines.
fn unfold(text: &str) -> Vec<String> {
    let mut logical: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if (line.starts_with(' ') || line.starts_with('\t')) && !logical.is_empty() {
            let last = logical.last_mut().unwrap();
            last.push_str(&line[1..]);
        } else {
            logical.push(line.to_string());
        }
    }
    logical.into_iter().filter(|l| !l.is_empty()).collect()
}

/// Split a content line into (NAME uppercased, params, value). Handles the Apple
/// `group.NAME` prefix and `;KEY=VAL` params. Quote-aware: a colon or semicolon
/// inside a DQUOTE-wrapped param value (RFC 6350 §3.3) does not split the line.
fn split_line(line: &str) -> Option<(String, Vec<(String, String)>, String)> {
    let colon = value_colon(line)?;
    let (head, rest) = line.split_at(colon);
    let value = rest[1..].to_string();
    let segs = split_unquoted_semicolons(head);
    let raw_name = segs.first()?;
    // strip a leading "group." (Apple item1.EMAIL) → take after the last '.'
    let name = raw_name
        .rsplit('.')
        .next()
        .unwrap_or(raw_name)
        .to_ascii_uppercase();
    let params = segs[1..]
        .iter()
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((k.trim().to_string(), v.trim().trim_matches('"').to_string()))
        })
        .collect();
    Some((name, params, value))
}

/// The index of the first `:` that is NOT inside a DQUOTE-wrapped param value —
/// the real name/params ↔ value boundary (`tel:`/`urn:`/`data:` colons live in
/// the value; a quoted `TYPE="a:b"` colon does not end the head).
fn value_colon(line: &str) -> Option<usize> {
    let mut in_q = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_q = !in_q,
            ':' if !in_q => return Some(i),
            _ => {}
        }
    }
    None
}

/// Split a property head on `;` that are NOT inside DQUOTEs (quotes retained so
/// the caller's `trim_matches('"')` unwraps the param value).
fn split_unquoted_semicolons(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    for c in s.chars() {
        match c {
            '"' => {
                in_q = !in_q;
                cur.push(c);
            }
            ';' if !in_q => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::group::{CredentialKind, CredentialMeta, GroupRole};
    use std::collections::BTreeMap;

    fn sample_state() -> GroupState {
        GroupState {
            face: Default::default(),
            rsvps: Default::default(),
            about: Default::default(),
            questions: Default::default(),
            claim_issuers: Default::default(),
            claims_spent: Default::default(),
            claims_picked: Default::default(),
            publication: Default::default(),
            notebook: Default::default(),
            joined: Default::default(),
            parent: None,
            profiles: Default::default(),
            listings: Default::default(),
            listings_removed: Default::default(),
            display_name: "Ada Lovelace".into(),
            shape: GroupShape::Individual,
            presence: Presence::OffPlatform(Some("ada@example.com".into())),
            card: ContactCard {
                org: "Example Co".into(),
                title: "Founder".into(),
                emails: vec![Labeled {
                    label: "work".into(),
                    value: "ada@example.com".into(),
                }],
                phones: vec![Labeled {
                    label: "cell".into(),
                    value: "+1-555-0100".into(),
                }],
                urls: vec![Labeled {
                    label: "work".into(),
                    value: "https://example.com".into(),
                }],
                note: "met at YC".into(),
                tags: vec!["yc".into(), "solar".into()],
                ..Default::default()
            },
            member_roles: BTreeMap::new(),
            credentials: {
                let mut m = BTreeMap::new();
                m.insert(
                    "k1".into(),
                    CredentialMeta {
                        kind: CredentialKind::ApiKey,
                        label: "prod".into(),
                    },
                );
                m
            },
            affiliations: BTreeMap::new(),
            parts: BTreeMap::new(),
            offices: BTreeMap::new(),
            membership: Default::default(),
            cover: String::new(),
            cover_mime: String::new(),
        }
    }

    #[test]
    fn export_then_import_round_trips_the_identity() {
        let st = sample_state();
        let vcf = group_to_vcard(&st, "abcd1234", &[]);
        assert!(vcf.contains("KIND:individual"));
        assert!(vcf.contains("FN:Ada Lovelace"));
        assert!(vcf.contains("UID:urn:pacific:local:abcd1234"));
        assert!(vcf.contains("X-PACIFIC-GROUP:abcd1234"));
        // credentials NEVER exported
        assert!(
            !vcf.contains("prod") && !vcf.contains("apiKey"),
            "vault never leaves"
        );

        let pc = vcard_to_card(&vcf).unwrap();
        assert_eq!(pc.display_name, "Ada Lovelace");
        assert_eq!(pc.shape, GroupShape::Individual);
        assert_eq!(pc.card.org, "Example Co");
        assert_eq!(pc.card.emails[0].value, "ada@example.com");
        assert_eq!(pc.card.phones[0].value, "+1-555-0100");
        assert_eq!(pc.card.note, "met at YC");
        assert_eq!(pc.card.tags, vec!["yc".to_string(), "solar".to_string()]);
        assert_eq!(
            pc.presence,
            Presence::OffPlatform(Some("ada@example.com".into()))
        );
        assert_eq!(pc.group_ref.as_deref(), Some("abcd1234"));
    }

    #[test]
    fn on_platform_uid_and_member_lines() {
        // A REAL 32-byte IdentityKey (honest presence rejects anything else on import).
        let space = format!("{}{}", crate::identity::IDENTITY_KEY_PREFIX, "ab".repeat(32));
        let mut st = sample_state();
        st.display_name = "Aries".into();
        st.shape = GroupShape::Team;
        st.presence = Presence::OnPlatform(space.clone());
        let roster = vec![[1u8; 32], [2u8; 32]];
        let vcf = group_to_vcard(&st, "objid", &roster);
        // unfold (a full 70-char IdentityKey line exceeds 75 octets and is folded).
        let flat = vcf.replace("\r\n ", "");
        assert!(flat.contains("KIND:group"));
        assert!(flat.contains(&format!("UID:urn:pacific:identity:{space}")));
        assert!(flat.contains(&format!("IMPP:pacific:{space}")));
        assert_eq!(vcf.matches("MEMBER:urn:pacific:identity:ed25519:").count(), 2);
        assert!(!vcf.contains("\nN:"), "no N for a team");

        let pc = vcard_to_card(&vcf).unwrap();
        assert_eq!(pc.presence, Presence::OnPlatform(space));
        assert_eq!(pc.member_uids.len(), 2);
    }

    #[test]
    fn fabricated_on_platform_space_downgrades_to_off() {
        // A hostile/hand-crafted card claiming on-platform with a garbage anchor
        // must NEVER fabricate reachability — it falls through to off-platform.
        let vcf = "BEGIN:VCARD\r\nVERSION:4.0\r\nKIND:individual\r\nFN:Mallory\r\nIMPP:pacific:hello\r\nEMAIL:m@x.com\r\nEND:VCARD\r\n";
        let pc = vcard_to_card(vcf).unwrap();
        assert!(
            matches!(pc.presence, Presence::OffPlatform(_)),
            "garbage space → off-platform"
        );
    }

    #[test]
    fn multi_card_vcf_yields_one_parsed_card_each() {
        let vcf = "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Alice\r\nEMAIL:a@x.com\r\nEND:VCARD\r\n\
                   BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Bob\r\nEMAIL:b@y.com\r\nEND:VCARD\r\n";
        let cards = vcard_to_cards(vcf).unwrap();
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].display_name, "Alice");
        assert_eq!(cards[1].display_name, "Bob");
        // the single-card entrypoint refuses to silently drop N-1
        assert!(
            vcard_to_card(vcf).is_err(),
            "single-card parse is loud on multi-card input"
        );
    }

    #[test]
    fn unterminated_card_is_loud_not_a_silent_drop() {
        // a truncated address-book export (Bob's card has no END) must FAIL, not
        // import Alice and silently drop Bob.
        let vcf = "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Alice\r\nEND:VCARD\r\n\
                   BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Bob\r\n";
        assert!(
            vcard_to_cards(vcf).is_err(),
            "BEGIN without END fails loudly"
        );
    }

    #[test]
    fn org_with_semicolon_round_trips() {
        let mut st = sample_state();
        st.card.org = "Foo; Bar Inc".into();
        let vcf = group_to_vcard(&st, "id", &[]);
        let pc = vcard_to_card(&vcf).unwrap();
        assert_eq!(
            pc.card.org, "Foo; Bar Inc",
            "escaped ';' survives the round-trip"
        );
    }

    #[test]
    fn quoted_param_colon_does_not_split_the_value() {
        let vcf =
            "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Q\r\nEMAIL;TYPE=\"a:b\":x@y.com\r\nEND:VCARD\r\n";
        let pc = vcard_to_card(vcf).unwrap();
        assert_eq!(
            pc.card.emails[0].value, "x@y.com",
            "colon inside quotes stays in the param"
        );
    }

    #[test]
    fn location_kind_is_rejected_not_coerced() {
        let vcf = "BEGIN:VCARD\r\nVERSION:4.0\r\nKIND:location\r\nFN:The Atelier\r\nEND:VCARD\r\n";
        assert!(matches!(vcard_to_card(vcf), Err(VcardError::NotAGroup(_))));
    }

    #[test]
    fn folding_round_trips_a_long_value() {
        let mut st = sample_state();
        st.card.note = "x".repeat(300);
        let vcf = group_to_vcard(&st, "id", &[]);
        // every physical line must be <= 75 octets
        for l in vcf.split("\r\n") {
            assert!(
                l.as_bytes().len() <= 75,
                "unfolded line too long: {}",
                l.len()
            );
        }
        let pc = vcard_to_card(&vcf).unwrap();
        assert_eq!(pc.card.note, "x".repeat(300));
    }

    #[allow(dead_code)]
    fn _role_import(_: GroupRole) {}
}
