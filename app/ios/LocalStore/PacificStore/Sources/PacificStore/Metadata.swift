//  Metadata.swift
//  The row metadata schema.
//
//  Every row written into the union store carries the same small, typed metadata
//  dictionary. Reads use it two ways: STRUCTURAL enumeration (`listDocuments`
//  filtering on `type`), and SCOPED search (filtering on `groupObjectId` against the
//  set of authorised ids). Writes stamp it via the builder here so keys and values
//  are produced in exactly one place; the Rust-direct route mirrors this builder.
//
//  Values are always strings — LodeDB's metadata is `[String: String]` and its
//  filter grammar compares strings — so the enums below define the CLOSED value
//  vocabularies and the parse helpers turn a raw row back into typed values.

import Foundation

/// Which kind of source object produced a row (§3 `source`).
public enum MetadataSource: String, Sendable, CaseIterable, Equatable {
    case note
    /// A DM or plain group conversation's message (the Chats tab). `forum` is the
    /// Group-hosted room's message — same mechanics, distinct source so search
    /// can scope to either.
    case chat
    case forum
    case project
    case topic
    case entity
    case system
}

/// What a row represents in the graph (§3 `kind`).
public enum MetadataKind: String, Sendable, CaseIterable, Equatable {
    /// A plain document (e.g. a note body).
    case doc
    /// A graph node — an entity. Carries a `type`.
    case node
    /// A graph edge between two entities. Carries `src` and `dst`.
    case edge
}

/// The entity type of a graph node (§3 `type`), and the LIFE filter vocabulary.
/// Raw values are the literal metadata strings, shared with `LifeSource`.
public enum EntityType: String, Sendable, CaseIterable, Equatable {
    case person
    case org
    case place
    case event
    case project
}

/// Builds and parses the §3 metadata dictionary. The literal key strings live here
/// once (``Key``); construct rows with the `document` / `node` / `edge` factories (or
/// the memberwise initialiser) and read them back with the static parse helpers.
public struct Metadata: Equatable, Sendable {

    /// The literal metadata key strings (§3). LodeDB filters reference these exact
    /// names, so they are defined in exactly one place.
    public enum Key {
        public static let source = "source"
        public static let objectId = "objectId"
        public static let chunkId = "chunkId"
        public static let kind = "kind"
        public static let type = "type"
        public static let src = "src"
        public static let dst = "dst"
        public static let groupObjectId = "groupObjectId"
    }

    // MARK: - Fields (§3)

    /// The source object's kind.
    public var source: MetadataSource
    /// The source object's id (note UUID / GroupObject id / entity id).
    public var objectId: String
    /// Which chunk of `objectId` this row is; feeds the deterministic `chunkId`.
    public var chunkIndex: Int
    /// Whether the row is a doc, a node, or an edge.
    public var kind: MetadataKind
    /// Entity type — present for nodes.
    public var type: EntityType?
    /// Edge source entity id — present for edges.
    public var src: String?
    /// Edge destination entity id — present for edges.
    public var dst: String?
    /// Scope key for membership-filtered search; nil for local-only rows.
    public var groupObjectId: String?

    public init(
        source: MetadataSource,
        objectId: String,
        chunkIndex: Int = 0,
        kind: MetadataKind,
        type: EntityType? = nil,
        src: String? = nil,
        dst: String? = nil,
        groupObjectId: String? = nil
    ) {
        self.source = source
        self.objectId = objectId
        self.chunkIndex = chunkIndex
        self.kind = kind
        self.type = type
        self.src = src
        self.dst = dst
        self.groupObjectId = groupObjectId
    }

    // MARK: - Deterministic chunk id

    /// The idempotent chunk id for the `n`-th chunk of `objectId`: `"<objectId>#<n>"`.
    /// Re-ingesting the same object reproduces the same ids, so upserts replace rather
    /// than duplicate.
    public static func chunkId(objectId: String, chunkIndex: Int) -> String {
        "\(objectId)#\(chunkIndex)"
    }

    /// This row's ``chunkId(objectId:chunkIndex:)``.
    public var chunkId: String {
        Metadata.chunkId(objectId: objectId, chunkIndex: chunkIndex)
    }

    // MARK: - Build

    /// The §3 dictionary. Optional fields are OMITTED when nil (never emitted empty),
    /// so a filter on `exists` behaves correctly.
    public func build() -> [String: String] {
        var row: [String: String] = [
            Key.source: source.rawValue,
            Key.objectId: objectId,
            Key.chunkId: chunkId,
            Key.kind: kind.rawValue
        ]
        if let type { row[Key.type] = type.rawValue }
        if let src { row[Key.src] = src }
        if let dst { row[Key.dst] = dst }
        if let groupObjectId { row[Key.groupObjectId] = groupObjectId }
        return row
    }

    // MARK: - Convenience factories

    /// A plain document row (`kind: doc`) — e.g. a note body chunk.
    public static func document(
        source: MetadataSource,
        objectId: String,
        chunkIndex: Int = 0,
        groupObjectId: String? = nil
    ) -> Metadata {
        Metadata(
            source: source,
            objectId: objectId,
            chunkIndex: chunkIndex,
            kind: .doc,
            groupObjectId: groupObjectId
        )
    }

    /// A graph-node row (`kind: node`) carrying its entity `type`.
    public static func node(
        source: MetadataSource,
        objectId: String,
        type: EntityType,
        chunkIndex: Int = 0,
        groupObjectId: String? = nil
    ) -> Metadata {
        Metadata(
            source: source,
            objectId: objectId,
            chunkIndex: chunkIndex,
            kind: .node,
            type: type,
            groupObjectId: groupObjectId
        )
    }

    /// A graph-edge row (`kind: edge`) between two entities.
    public static func edge(
        source: MetadataSource,
        objectId: String,
        src: String,
        dst: String,
        chunkIndex: Int = 0,
        groupObjectId: String? = nil
    ) -> Metadata {
        Metadata(
            source: source,
            objectId: objectId,
            chunkIndex: chunkIndex,
            kind: .edge,
            src: src,
            dst: dst,
            groupObjectId: groupObjectId
        )
    }

    // MARK: - Parse helpers (raw row -> typed values)

    /// The `source` of a raw row, or nil if absent/unrecognised.
    public static func source(_ row: [String: String]) -> MetadataSource? {
        row[Key.source].flatMap(MetadataSource.init(rawValue:))
    }

    /// The `objectId` of a raw row.
    public static func objectId(_ row: [String: String]) -> String? {
        row[Key.objectId]
    }

    /// The `chunkId` of a raw row.
    public static func chunkId(_ row: [String: String]) -> String? {
        row[Key.chunkId]
    }

    /// The `kind` of a raw row, or nil if absent/unrecognised.
    public static func kind(_ row: [String: String]) -> MetadataKind? {
        row[Key.kind].flatMap(MetadataKind.init(rawValue:))
    }

    /// The entity `type` of a raw row (nodes), or nil if absent/unrecognised.
    public static func type(_ row: [String: String]) -> EntityType? {
        row[Key.type].flatMap(EntityType.init(rawValue:))
    }

    /// The edge `src` of a raw row (edges).
    public static func src(_ row: [String: String]) -> String? {
        row[Key.src]
    }

    /// The edge `dst` of a raw row (edges).
    public static func dst(_ row: [String: String]) -> String? {
        row[Key.dst]
    }

    /// The `groupObjectId` scope key of a raw row.
    public static func groupObjectId(_ row: [String: String]) -> String? {
        row[Key.groupObjectId]
    }
}
