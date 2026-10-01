//! OntologyAnchors — the ontology as a TRANSDUCER: pacific-ontology.ttl read at
//! load time (never hardcoded — the resolver rule survives the resolver), reduced
//! to the kind anchors the Transmission projects clusters against.
//!
//! An anchor is a kind's label + skos:definition, embedded with the SAME shared
//! embedder as every episode, so "how Person-like is this cluster" is a cosine in
//! the one space the device already speaks. CATEGORY IS ORTHOGONAL TO INSTANCE
//! (the 7 Aug directive): clustering partitions the space into instances; anchors
//! independently read each cluster's category affinity. Nothing here collapses the
//! affinity vector to a winner — a cluster may be Group-shaped on one axis and
//! Place-shaped on another, and the lens reports both.
//!
//! The parser is deliberately a LINE-LEVEL extraction, not a Turtle engine: it
//! reads exactly the shape this .ttl writes (`pac:X a owl:Class ; ... rdfs:label
//! "X"@en ; ... skos:definition "…"@en`), fails loudly on a kind it can't parse,
//! and is pinned by tests against the real grammar. If the .ttl grows beyond this
//! shape, grow the parser with it — the conformance test will say so.

use std::collections::BTreeMap;
use std::fmt;

/// One kind read from the ontology, ready to become an anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OntologyKind {
    /// "Person" — matches the resolver's entity-type strings verbatim
    pub name: String,
    /// the skos:definition text — the embeddable anchor body
    pub definition: String,
}

/// A kind with its anchor vector in the shared embedding space.
#[derive(Debug, Clone)]
pub struct KindAnchor {
    pub kind: OntologyKind,
    pub vector: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OntologyAnchorError {
    FileUnreadable(String),
    /// requested kinds the .ttl didn't yield
    KindMissing(Vec<String>),
}

impl fmt::Display for OntologyAnchorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OntologyAnchorError::FileUnreadable(path) => write!(f, "fileUnreadable({path})"),
            OntologyAnchorError::KindMissing(names) => write!(f, "kindMissing({names:?})"),
        }
    }
}

impl std::error::Error for OntologyAnchorError {}

/// The four kinds the 7 Aug directive names for resolution. Extend from the
/// .ttl's Lived roster as lenses grow.
pub const RESOLUTION_KINDS: [&str; 4] = ["Person", "Place", "Event", "Group"];

/// Extract `names` from Turtle source. Order follows the request, so anchor
/// arrays are stable. Loud on a missing kind — a transducer with silent holes
/// would mis-rank everything downstream.
///
/// (Swift's `names` default argument is expressed here by passing
/// `&RESOLUTION_KINDS`.)
pub fn kinds_from_turtle(
    source: &str,
    names: &[&str],
) -> Result<Vec<OntologyKind>, OntologyAnchorError> {
    let mut found: BTreeMap<&str, &str> = BTreeMap::new();

    // Statements end at " ." — split there, then find class statements.
    // The .ttl writes one class per statement (possibly multi-line).
    for raw in source.split(" .") {
        let stmt = raw.trim();
        if !stmt.contains("a owl:Class") || !stmt.starts_with("pac:") {
            continue;
        }
        let Some(name) = leading_class_name(stmt) else {
            continue;
        };
        if !names.contains(&name) {
            continue;
        }
        let Some(definition) = skos_definition(stmt) else {
            continue;
        };
        found.insert(name, definition);
    }

    let missing: Vec<String> = names
        .iter()
        .filter(|n| !found.contains_key(*n))
        .map(|n| n.to_string())
        .collect();
    if !missing.is_empty() {
        return Err(OntologyAnchorError::KindMissing(missing));
    }
    Ok(names
        .iter()
        .map(|n| OntologyKind {
            name: n.to_string(),
            definition: found[n].to_string(),
        })
        .collect())
}

/// Read the .ttl from disk and extract.
pub fn kinds_at_path(
    path: &str,
    names: &[&str],
) -> Result<Vec<OntologyKind>, OntologyAnchorError> {
    let source = std::fs::read_to_string(path)
        .map_err(|_| OntologyAnchorError::FileUnreadable(path.to_string()))?;
    kinds_from_turtle(&source, names)
}

/// Embed each kind's anchor text ("<name>. <definition>") with the caller's
/// embedder — the SAME one the union store uses, injected so tests can fake it.
/// A `None` from the embedder propagates (Swift's `rethrows`).
pub fn anchors(
    kinds: &[OntologyKind],
    embed: impl Fn(&str) -> Option<Vec<f32>>,
) -> Option<Vec<KindAnchor>> {
    kinds
        .iter()
        .map(|k| {
            embed(&format!("{}. {}", k.name, k.definition)).map(|vector| KindAnchor {
                kind: k.clone(),
                vector,
            })
        })
        .collect()
}

/// Mirror of the Swift regex `^pac:([A-Za-z]+)\s` on the trimmed statement:
/// the class name is the ASCII-letter run after "pac:", and a whitespace
/// character must follow it.
fn leading_class_name(stmt: &str) -> Option<&str> {
    let rest = stmt.strip_prefix("pac:")?;
    let end = rest
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    let next = rest[end..].chars().next()?;
    if !next.is_whitespace() {
        return None;
    }
    Some(&rest[..end])
}

/// Mirror of the Swift regex `skos:definition\s+"((?:[^"\\]|\\.)*)"@en` with
/// dot-matches-line-separators: leftmost occurrence wins, the quoted body is
/// returned RAW (escapes preserved), a backslash consumes the following
/// character (any character, newlines included), and the closing quote must be
/// immediately followed by `@en`.
fn skos_definition(stmt: &str) -> Option<&str> {
    const KEY: &str = "skos:definition";
    let mut search = 0;
    while let Some(pos) = stmt[search..].find(KEY) {
        let key_at = search + pos;
        if let Some(def) = quoted_en_after_whitespace(&stmt[key_at + KEY.len()..]) {
            return Some(def);
        }
        // The regex engine retries from the next start offset.
        search = key_at + 1;
    }
    None
}

fn quoted_en_after_whitespace(s: &str) -> Option<&str> {
    // `\s+` — at least one whitespace character before the opening quote.
    let trimmed = s.trim_start();
    if trimmed.len() == s.len() {
        return None;
    }
    let body = trimmed.strip_prefix('"')?;
    let mut chars = body.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            // `\\.` — an escape consumes the next character; a trailing
            // backslash with nothing after it fails the match.
            '\\' => {
                chars.next()?;
            }
            '"' => {
                return body[i + 1..].strip_prefix("@en").map(|_| &body[..i]);
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // The .ttl's real statement grammar, verbatim shape.
    const TTL: &str = "pac:Person  a owl:Class ; rdfs:subClassOf pac:Lived ; rdfs:label \"Person\"@en ;\n    skos:definition \"A human participant in the graph.\"@en .\n\npac:Place   a owl:Class ; rdfs:subClassOf pac:Lived ; rdfs:label \"Place\"@en ;\n    skos:definition \"A located where: venue, home, corner.\"@en .\n\npac:Event   a owl:Class ; rdfs:subClassOf pac:Lived ; rdfs:label \"Event\"@en ;\n    skos:definition \"A dated happening people attend.\"@en .\n\npac:Group   a owl:Class ; rdfs:subClassOf pac:Lived ; rdfs:label \"Group\"@en ;\n    skos:definition \"People acting as one body.\"@en .";

    #[test]
    fn extracts_the_four_resolution_kinds() {
        let kinds = kinds_from_turtle(TTL, &RESOLUTION_KINDS).unwrap();
        let names: Vec<&str> = kinds.iter().map(|k| k.name.as_str()).collect();
        assert_eq!(names, ["Person", "Place", "Event", "Group"]);
        assert!(kinds[1].definition.contains("venue"));
    }

    #[test]
    fn missing_kind_fails_loudly() {
        let err = kinds_from_turtle(TTL, &["Person", "Theme"]).unwrap_err();
        assert_eq!(err, OntologyAnchorError::KindMissing(vec!["Theme".to_string()]));
    }

    #[test]
    fn real_ontology_file_parses_when_present() {
        // Pinned against the actual .ttl when this machine has it — the grammar conformance the
        // header comment promises: where PACIFIC_ONTOLOGY_TTL names it, else at
        // $HOME/pacific/docs/pacific-ontology.ttl, where it has always been read. Skipped elsewhere.
        let path = std::env::var("PACIFIC_ONTOLOGY_TTL")
            .ok()
            .or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/pacific/docs/pacific-ontology.ttl")));
        let Some(path) = path.filter(|p| std::path::Path::new(p).exists()) else { return };
        let kinds = kinds_at_path(&path, &RESOLUTION_KINDS).unwrap();
        assert_eq!(kinds.len(), 4);
        assert!(kinds.iter().all(|k| !k.definition.is_empty()));
    }
}
