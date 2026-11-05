//  NoteTags.swift
//  RETIRED AS THE TAG WRITER (2 Aug ruling): a tag is a #hashtag the author
//  typed, parsed by the ONE extractor CHAT and NOTE share (`Hashtags`, derived
//  inside NotesStore.mirror). Nothing calls NoteTagger today.
//
//  NoteTagger is RETAINED as the candidate engine for tag SUGGESTIONS — ranking
//  the graph's resolved-entity vocabulary against a note by true cosine is the
//  right machinery for "did you mean to tag #marco?", which is a proposal
//  surface, not a writer. Delete it if that surface is cut.
//
//  How it worked as a writer: a committed note was matched against the vocabulary
//  the graph had ALREADY resolved (resolver.lode entity labels): hybrid recall
//  proposed candidates, then a TRUE cosine between the note text and each
//  candidate's label re-scored them. Both sides are the same 512-d on-device
//  vectors (UnionEmbedder). Fully offline; floor-gated at 0.5 (HeuristicDecider
//  precedent); it never minted a label of its own.

import Foundation
import PacificStore
import LodeDBCore
import ArcResolverKit   // ArcEntity — the resolved-entity vocabulary being ranked

struct NoteTag: Codable, Equatable, Hashable {
    let name: String
    let type: String          // the entity's LIFE type (Person/Place/Project/…)
}

enum NoteTagger {
    /// Cosine acceptance floor — HeuristicDecider parity. PROVISIONAL.
    static let floor: Float = 0.5
    /// At most this many tags per note; recall breadth before the gate.
    static let cap = 5, recallK = 16
    /// Embed at most this much of the note — union-store chunk parity (UnionConfig).
    private static let textLimit = 2000

    /// The one embedder, shared with both stores' vectors — built for the configured
    /// union language (F-3; safe here because every comparison is within-call). Built
    /// once, AFTER the composition root resolves the language; a failure here is a
    /// real fault surfaced on first use, not a silent no-tag world.
    private static let embedder: LodeEmbedder? =
        try? UnionEmbedder.makeUnion(language: UnionLanguageSetting.load())

    /// Rank the graph's resolved entities against the note text; return the accepted tags.
    @MainActor
    static func tags(for text: String, graph: GraphService) async -> [NoteTag] {
        let payload = String(text.prefix(textLimit))
        let hits = await graph.semanticEntities(query: payload, k: recallK)
        guard !hits.isEmpty else { return [] }
        guard let embedder else {
            NSLog("[NoteTagger] no embedder — tags skipped (no-fallback)")
            return []
        }
        // The label cosine compares against what the graph itself embedded: "name. summary".
        let entities = hits.map(\.entity)
        let labels = entities.map { e in e.summary.map { "\(e.name). \($0)" } ?? e.name }
        let scored: [(Float, ArcEntity)] = await Task.detached(priority: .userInitiated) {
            do {
                let vecs = try embedder.embed(texts: [payload] + labels)
                guard vecs.count == labels.count + 1 else { return [] }
                let q = vecs[0]
                return entities.enumerated().map { (cosine(q, vecs[$0.offset + 1]), $0.element) }
            } catch {
                NSLog("[NoteTagger] embed failed — tags skipped: \(error)")
                return []
            }
        }.value

        var seen = Set<String>()
        return scored
            .sorted { $0.0 > $1.0 }
            .filter { $0.0 >= floor }
            .compactMap { _, e in
                seen.insert(e.name.lowercased()).inserted
                    ? NoteTag(name: e.name, type: e.entityType) : nil
            }
            .prefix(cap)
            .map { $0 }
    }

    private static func cosine(_ a: [Float], _ b: [Float]) -> Float {
        guard a.count == b.count, !a.isEmpty else { return -1 }
        var dot: Float = 0, na: Float = 0, nb: Float = 0
        for i in a.indices { dot += a[i] * b[i]; na += a[i] * a[i]; nb += b[i] * b[i] }
        let d = (na * nb).squareRoot()
        return d > 0 ? dot / d : -1
    }
}

// (NoteTagService — the coalescing writer that ran NoteTagger on every settled
// commit — was deleted with the 2 Aug ruling; tags now derive inside the commit.)
