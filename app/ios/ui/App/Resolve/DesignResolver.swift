//  DesignResolver.swift
//  The engine-agnostic candidate-generation seam (workstream F contract). "Talk in
//  human -> the design's Sysiphus entry": resolve a colloquial span, in context, to
//  candidate entities. Backend is swappable — a lexical resolver works today over the
//  rich resolution_text; a LodeDB hybrid (lexical + vector + graph) drops in behind
//  the same protocol (LodeResolver, #if canImport(LodeDBCore)).
//
//  The resolver indexes ResolvableEntity (a projection produced by the facade OR the
//  real System) and returns Candidate {entityId, score}. It does NOT hold the full
//  record — that comes from LOOKUP. Recall is won at index time (resolution_text
//  packs name + every alias + category + material + fit + season).

import Foundation

/// A full record ranked against a query — the convenience result the SystemsView
/// playground renders (resolve() maps Candidates back through the field store).
struct ResolvedField: Identifiable, Hashable {
    let field: SysiphusField
    let score: Float
    var id: String { field.id }
}

/// The swappable candidate-generation backend.
protocol DesignResolver: AnyObject {
    var backendName: String { get }
    func index(_ entities: [ResolvableEntity])
    func candidates(_ query: ResolutionQuery, k: Int) -> [Candidate]
}

/// Pick the best available backend. LodeDB if its Swift package is linked; the lexical
/// resolver otherwise — same interface, so the app behaves identically.
func makeDesignResolver() -> DesignResolver {
    #if canImport(LodeDBCore)
    do { return try LodeResolver() }
    catch {
        // Not silent: canImport succeeded but the engine failed to initialise (embedder /
        // xcframework). Degrade to lexical so the app still resolves, but make WHY diagnosable.
        print("[resolver] LodeDB backend unavailable — falling back to lexical: \(error)")
    }
    #endif
    return LexicalResolver()
}

// MARK: - Lexical backend (always available; no dependency)

/// Token-overlap over resolution_text weighted by token rarity (idf), with a phrase
/// boost for alias/name hits. Reads mention+window from the query; anchors/scope are
/// no-ops here (the LodeDB backend uses them). Simple + exact; LodeDB adds semantic recall.
final class LexicalResolver: DesignResolver {
    let backendName = "lexical"

    private struct Doc {
        let id: String
        let tokens: Set<String>
        let phrases: [String]   // normalized name + aliases, for phrase hits
    }
    private var docs: [Doc] = []
    private var idf: [String: Float] = [:]

    func index(_ entities: [ResolvableEntity]) {
        docs = entities.map { e in
            Doc(id: e.id,
                tokens: Set(Self.tokens(e.text)),
                phrases: ([e.name] + e.aliases).map(Self.normalize))
        }
        var df: [String: Int] = [:]
        for d in docs { for t in d.tokens { df[t, default: 0] += 1 } }
        let n = Float(max(docs.count, 1))
        idf = df.mapValues { log(1 + n / Float($0)) }
    }

    func candidates(_ query: ResolutionQuery, k: Int) -> [Candidate] {
        // Context in: mention drives phrase/alias hits, window adds descriptive tokens.
        let queryText = query.mention + " " + query.window
        let qTokens = Self.tokens(queryText)
        guard !qTokens.isEmpty else { return [] }
        let qNorm = Self.normalize(query.mention)

        var out: [Candidate] = []
        for d in docs {
            var s: Float = 0
            for t in qTokens where d.tokens.contains(t) { s += idf[t] ?? 1 }
            guard s > 0 else { continue }
            for p in d.phrases where !p.isEmpty && (qNorm.contains(p) || p.contains(qNorm)) {
                s += 4 + Float(p.split(separator: " ").count)   // multi-word alias = stronger
            }
            out.append(Candidate(entityId: d.id, score: s, generator: "lexical"))
        }
        return Array(out.sorted { $0.score > $1.score }.prefix(k))
    }

    // MARK: normalization
    static func tokens(_ s: String) -> [String] {
        s.lowercased().split { !$0.isLetter && !$0.isNumber }.map(String.init).filter { $0.count > 1 }
    }
    static func normalize(_ s: String) -> String { tokens(s).joined(separator: " ") }
}
