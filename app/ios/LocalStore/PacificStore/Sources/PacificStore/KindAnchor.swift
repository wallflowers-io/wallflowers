//  KindAnchor.swift
//  A kind a cluster can be read against: its name and definition, and that text's
//  vector in the one embedding space every episode shares. The caller supplies the
//  anchors; nothing here reads an ontology.

import Foundation

/// One kind, ready to become an anchor.
public struct OntologyKind: Equatable, Sendable {
    public let name: String        // "Person" — matches the resolver's entity-type strings verbatim
    public let definition: String  // the definition — the embeddable anchor body
}

/// A kind with its anchor vector in the shared embedding space.
public struct KindAnchor: Sendable {
    public let kind: OntologyKind
    public let vector: [Float]
}
