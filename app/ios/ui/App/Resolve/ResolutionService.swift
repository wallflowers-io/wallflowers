//  ResolutionService.swift
//  The app-facing resolution facade. Owns the swappable DesignResolver backend (LodeDB
//  hybrid if linked, else lexical), the ResolvableEntity index, and the field store for
//  LOOKUP. Callers do RESOLVE (candidates over a ResolutionQuery) and LOOKUP (field(for:)).
//  `indexEpoch` bumps on every (re)index so cached resolutions invalidate (see
//  ResolvedSpansStore) and visible content re-resolves on System connect / credential change.
//
//  The index is NEUTRAL: it holds ResolvableEntity per SOURCE ("sysiphus", "music", …),
//  and the backend is fed the union — so a credentialed System and a public one share the
//  one recall surface. The Sysiphus-shaped conveniences (`index(_:[SysiphusField])`,
//  `field(for:)`, `resolve`) ride on top unchanged: they are the rich-record LOOKUP tier
//  only Sysiphus has, and every pre-refactor caller behaves identically.

import SwiftUI

@MainActor
final class ResolutionService: ObservableObject {
    @Published private(set) var backendName: String
    @Published private(set) var indexedCount = 0
    @Published private(set) var indexEpoch = 0     // bumped on (re)index; cache-invalidation key

    private let backend: DesignResolver
    private var entitiesBySource: [String: [ResolvableEntity]] = [:]  // the neutral index, per source
    private var entitiesById: [String: ResolvableEntity] = [:]        // union, for LOOKUP (neutral tier)
    private var fieldsById: [String: SysiphusField] = [:]             // LOOKUP store (Sysiphus rich tier)

    init() {
        backend = makeDesignResolver()
        backendName = backend.backendName
    }

    /// Replace ONE source's entities and re-feed the backend the union. This is the one
    /// write path: Sysiphus and Music (and any later System) index through here, so a
    /// refresh of one source never clobbers another.
    func index(_ entities: [ResolvableEntity], from source: String) {
        entitiesBySource[source] = entities
        let union = entitiesBySource.keys.sorted().flatMap { entitiesBySource[$0] ?? [] }
        entitiesById = Dictionary(union.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        backend.index(union)
        indexedCount = union.count
        indexEpoch += 1
    }

    /// Index a System's Fields: keep the full records for LOOKUP, feed the resolver the
    /// ResolvableEntity projection for candidate-gen. Recall is front-loaded into resolution_text.
    func index(_ fields: [SysiphusField]) {
        fieldsById = Dictionary(fields.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        index(fields.map(ResolvableEntity.init(sysiphus:)), from: "sysiphus")
    }

    /// Clear the Sysiphus System's index (sign-out). Other sources keep theirs — the same
    /// call SystemService always made, with the same effect on its own entities.
    func clear() {
        fieldsById = [:]
        index([], from: "sysiphus")
    }

    // MARK: RESOLVE — context-aware candidate generation

    func candidates(_ query: ResolutionQuery, k: Int = 5) -> [Candidate] {
        backend.candidates(query, k: k)
    }

    // MARK: LOOKUP — id -> record (cached tier; WS-D adds the live credentialed source read)

    /// The neutral projection any indexed source answers with.
    func entity(for id: String) -> ResolvableEntity? { entitiesById[id] }

    /// The rich Sysiphus record — nil for entities from any other source.
    func field(for id: String) -> SysiphusField? { fieldsById[id] }

    // MARK: playground convenience — bare-span resolve -> ranked full records

    func resolve(_ text: String, k: Int = 5) -> [ResolvedField] {
        let mention = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !mention.isEmpty else { return [] }
        return candidates(.bare(mention), k: k)
            .compactMap { c in fieldsById[c.entityId].map { ResolvedField(field: $0, score: c.score) } }
    }

    func resolveBest(_ text: String) -> ResolvedField? { resolve(text, k: 1).first }
}

// MARK: - Projection from the Sysiphus facade Field (WS-A will produce this from the real object)

extension ResolvableEntity {
    init(sysiphus f: SysiphusField) {
        var md: [String: String] = [:]
        if let s = f.season { md["season"] = s }
        if let c = f.category { md["category"] = c }
        if let col = f.collection { md["collection"] = col }
        self.init(
            id: f.id,
            kind: "design",
            name: f.name,
            aliases: f.aliases,
            text: f.resolutionText,
            metadata: md,
            sourceLocator: f.id
        )
    }
}
