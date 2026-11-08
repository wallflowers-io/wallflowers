//  LocalStore.swift
//  The read/write facade over the union store.
//
//  LodeDB IS the database. Everything the UI reads comes through this facade, and
//  every note/entity write goes straight through it — there is no notes.json. The
//  protocol here is the FROZEN seam: `LodeLocalStore` (WS-E) is the real LodeDB-backed
//  implementation and is what the composition root wires today; consumers (producers,
//  the LIFE screen, note search) depend only on the protocol, so the backend
//  is a one-line choice at the root.
//
//  Honesty contract (house rule): every method is fallible and MUST fail loud —
//  a real backend never returns a zero/placeholder vector, never silently swallows
//  a missing model or corrupt store, and never fabricates freshness. `nil` from
//  `get` means genuinely-absent; an empty array from `search`/`listDocuments` means
//  a genuinely-empty result — neither is a fallback masking a failure, which is a
//  thrown error instead.
//
//  Methods are `async` because the real implementation is disk- and native-core-
//  backed and runs alongside a background checkpointer; the seam commits to that
//  from the start so callers are structured correctly.

import Foundation

/// A payload-carrying document as stored in the union store: its id, retained text
/// (present when the store kept text and the row has any), the §3 metadata, and how
/// many chunks it spans.
public struct Document: Equatable, Sendable {
    public let id: String
    public let text: String?
    public let metadata: [String: String]
    public let chunkCount: Int

    public init(id: String, text: String? = nil, metadata: [String: String] = [:], chunkCount: Int = 1) {
        self.id = id
        self.text = text
        self.metadata = metadata
        self.chunkCount = chunkCount
    }
}

/// One search result: the matched document id, the specific chunk that matched, the
/// relevance score (higher is better), the row's §3 metadata, and — when available —
/// the retained snippet.
public struct Hit: Equatable, Sendable {
    public let id: String
    public let chunkId: String
    public let score: Float
    public let metadata: [String: String]
    public let text: String?

    public init(
        id: String,
        chunkId: String? = nil,
        score: Float,
        metadata: [String: String] = [:],
        text: String? = nil
    ) {
        self.id = id
        self.chunkId = chunkId ?? id
        self.score = score
        self.metadata = metadata
        self.text = text
    }
}

/// The visibility a search runs under. The union holds many producers' rows; scoping
/// restricts results to the authorised set (membership = access, §3 `groupObjectId`).
public enum SearchScope: Sendable, Equatable {
    /// No membership restriction — local-only + everything the caller may see.
    case all
    /// Restrict group-scoped rows to those whose `groupObjectId` is in this allow-list
    /// (`{$in: ids}`); rows with no `groupObjectId` (local-only) remain visible. An
    /// empty list therefore authorises no groups — only un-scoped rows match.
    case groups([String])
}

/// The frozen read/write seam over the union store (§7). Read: `search`,
/// `listDocuments`, `get`. Write: `upsert`, `remove`. Maintenance: `reindexAll`.
public protocol LocalStore: Sendable {

    /// Create-or-replace a document by id, with its text and §3 metadata. Idempotent
    /// on `id` (deterministic `chunkId`s mean a re-ingest replaces, not duplicates).
    func upsert(id: String, text: String, metadata: [String: String]) async throws

    /// Delete a document by id. A no-op delete of an absent id is not an error.
    func remove(id: String) async throws

    /// Hybrid (lexical + vector) search for the top `k` hits, restricted by `scope`.
    func search(_ query: String, k: Int, scope: SearchScope) async throws -> [Hit]

    /// Structural enumeration of documents, optionally filtered to one entity `type`
    /// (nil = all documents). This is the LIFE-list / graph-traversal primitive.
    func listDocuments(type: EntityType?) async throws -> [Document]

    /// Fetch one document (with retained text) by id, or nil if absent.
    func get(id: String) async throws -> Document?

    /// Rebuild the index from retained text — e.g. after an embedder change. Loud on
    /// failure; never leaves a half-built index silently in place.
    func reindexAll() async throws
}

public extension LocalStore {
    /// `search` with a default `k` and unrestricted scope.
    func search(_ query: String, k: Int = 10) async throws -> [Hit] {
        try await search(query, k: k, scope: .all)
    }

    /// All documents, unfiltered.
    func listDocuments() async throws -> [Document] {
        try await listDocuments(type: nil)
    }
}

// There is deliberately NO in-memory stub implementation here. `LodeLocalStore` is the
// one and only `LocalStore`: the app wires it at the composition root, and the tests
// exercise it directly on a throwaway directory. A second, subtly-different
// implementation is a place for behaviour to drift — a substring "search" that never
// embeds, a scope filter that diverges from the engine's — and drift in a store is
// exactly the kind of bug tests are supposed to catch, not manufacture.
