//! A kind a cluster can be read against: its name and definition, and that text's vector in the
//! one embedding space every episode shares. The caller supplies the anchors; nothing here reads
//! an ontology.

/// One kind, ready to become an anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OntologyKind {
    /// "Person" — matches the resolver's entity-type strings verbatim
    pub name: String,
    /// the definition — the embeddable anchor body
    pub definition: String,
}

/// A kind with its anchor vector in the shared embedding space.
#[derive(Debug, Clone)]
pub struct KindAnchor {
    pub kind: OntologyKind,
    pub vector: Vec<f32>,
}
