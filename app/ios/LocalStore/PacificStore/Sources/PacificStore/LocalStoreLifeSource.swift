//  LocalStoreLifeSource.swift
//  The concrete, LocalStore-backed `LifeSource` (WS-F) that replaces `StubLifeSource`
//  at the composition root.
//
//  It realises the §4 read model for the LIFE screen directly over the FROZEN
//  `LocalStore` seam (WS-E) — no engine types leak here, only the protocol:
//
//    entities(ofType:) -> listDocuments(type:)          structural enumeration by type
//    projects()        -> listDocuments(type: .project) the project GroupObjects
//    related(to:)      -> edge enumeration              rows where kind == edge && src == id
//
//  Only §3 metadata is read back — nodes carry `type`, edges carry `src`/`dst` — so the
//  projection never invents a field the store did not persist. Same honesty contract as
//  `LocalStore` / `LifeSource`: fallible, fail-loud, and an empty array is a genuinely-
//  empty world rather than a swallowed error.
//
//  ON `related`: the frozen `LocalStore` protocol (§7) exposes structural enumeration
//  by entity `type` only, not by an arbitrary metadata key, so edge incidence is
//  resolved by enumerating documents and selecting the `kind == edge` rows whose `src`
//  is `id`. The real engine can filter `{src: id}` engine-side (§4/§5); this
//  protocol-level projection stays within the frozen seam and is exact, not a heuristic.

import Foundation

/// The production `LifeSource`: the §4 LIFE read model folded onto a `LocalStore`.
/// Construct it at the composition root with the shared store and hand it to the LIFE
/// screen in place of `StubLifeSource`.
public struct LocalStoreLifeSource: LifeSource {

    /// The one union store this projection reads (the same instance the producers write).
    private let store: LocalStore

    public init(store: LocalStore) {
        self.store = store
    }

    public func entities(ofType type: EntityType) async throws -> [Document] {
        // `type` is stamped on nodes only (§3), so a type filter already selects graph
        // nodes — notes (kind: doc) and edges (kind: edge) carry no `type`.
        try await store.listDocuments(type: type)
    }

    public func projects() async throws -> [Document] {
        // Projects are the project-typed nodes (project GroupObjects, §0/§4). Kept as a
        // named entry point because the caller treats them distinctly from local-only
        // entities even though the enumeration is the same.
        try await entities(ofType: .project)
    }

    public func related(to id: String) async throws -> [Edge] {
        // No src-filter on the frozen seam (see header): enumerate and select the edges
        // incident from `id`. An edge missing its `dst` is malformed and dropped rather
        // than surfaced as a half-edge.
        try await store.listDocuments().compactMap { doc in
            guard Metadata.kind(doc.metadata) == .edge,
                  Metadata.src(doc.metadata) == id,
                  let dst = Metadata.dst(doc.metadata) else { return nil }
            return Edge(id: doc.id, src: id, dst: dst, metadata: doc.metadata)
        }
    }
}
