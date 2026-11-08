//  LifeSource.swift
//  The projection the LIFE screen reads.
//
//  LIFE replaces the old Work tab and reads entirely from the union store: entity
//  lists come from structural enumeration by `type`, projects are the project
//  GroupObjects, and relatedness is edge enumeration from an entity. This protocol
//  is the FROZEN, LocalStore-backed seam the LIFE UI (WS-C) consumes; the real
//  implementation folds onto `LocalStore` (WS-E/WS-F), and `StubLifeSource` lets the
//  screen compile today.
//
//  Same honesty contract as `LocalStore`: fallible, fail-loud. An empty array is a
//  genuinely-empty world, not a swallowed error.

import Foundation

/// One graph edge incident from an entity (§3 `kind: edge`): the edge object's own
/// id, its `src`/`dst` entity ids, and the raw row metadata.
public struct Edge: Equatable, Sendable {
    public let id: String
    public let src: String
    public let dst: String
    public let metadata: [String: String]

    public init(id: String, src: String, dst: String, metadata: [String: String] = [:]) {
        self.id = id
        self.src = src
        self.dst = dst
        self.metadata = metadata
    }
}

/// The LIFE screen's read model over the union store (§7). Backed by a `LocalStore`
/// in production; the UI depends only on this protocol.
public protocol LifeSource: Sendable {

    /// All entities of one type (person / org / place / event / project) — the LIFE
    /// filter lists. Maps to `listDocuments(filter:{type})`.
    func entities(ofType type: EntityType) async throws -> [Document]

    /// The project GroupObjects — the PROJECTS filter. Convenience over
    /// `entities(ofType: .project)`, kept distinct because projects are synced
    /// GroupObjects rather than local-only entities.
    func projects() async throws -> [Document]

    /// The edges incident from an entity — the relatedness primitive. Maps to
    /// `listDocuments(filter:{src: id})`.
    func related(to id: String) async throws -> [Edge]
}

/// A COMPILE-ONLY, in-memory `LifeSource` so the LIFE screen (WS-C) builds without
/// the real LocalStore-backed projection. It serves whatever entities and edges it
/// is seeded with (empty by default) — an honest empty world, no fabricated graph.
public actor StubLifeSource: LifeSource {

    private var entitiesByType: [EntityType: [Document]]
    private var edgesBySource: [String: [Edge]]

    /// Seed the stub. Both default to empty; pass fixtures to preview a populated
    /// screen.
    public init(
        entities: [EntityType: [Document]] = [:],
        edges: [String: [Edge]] = [:]
    ) {
        self.entitiesByType = entities
        self.edgesBySource = edges
    }

    public func entities(ofType type: EntityType) async throws -> [Document] {
        entitiesByType[type] ?? []
    }

    public func projects() async throws -> [Document] {
        try await entities(ofType: .project)
    }

    public func related(to id: String) async throws -> [Edge] {
        edgesBySource[id] ?? []
    }
}
