//  NoteSearch.swift
//  Search over notes that reads BOTH stores Pacific already keeps, and fuses them.
//
//  Two independent retrievers ("lanes"), each good at something the other is blind to:
//
//    • TEXT  — the union store's hybrid lexical+vector search over the note's own text
//              (`LocalStore.search`). Finds a note by what it SAYS, including paraphrase:
//              "who tailors my suits" reaches a note that says "took the jacket to Marco".
//              Blind to anything the words don't carry.
//
//    • GRAPH — the on-device bi-temporal knowledge graph. The query is matched against
//              ENTITY labels (`semanticEntities`), and each hit entity is resolved back to
//              the notes whose episodes carry facts about it (`noteIDs(forEntity:)`).
//              Finds a note by what it is ABOUT even when the query words never appear in
//              it. Only notes an earlier build read into the graph have episodes; nothing
//              on the device writes new ones, so for a note written since, this lane is
//              silent and the text lane carries the search.
//
//  The lanes are fused with RECIPROCAL RANK FUSION (Cormack et al. 2009), the same rule
//  lodedb-graph ports as a reranker: score = Σ 1/(K + rank) over the lanes a note appears
//  in. RRF fuses by RANK, never by raw score, which is the point — a cosine similarity and
//  a graph hop have no common scale, and any attempt to weight them numerically is a
//  fabricated commensurability. A note found by BOTH lanes outranks one found by either,
//  which is exactly the intuition "the words match AND it's about the right thing".
//
//  Honesty contract (house rule): a lane that FAILS throws — it is never silently treated
//  as "returned nothing", because a broken embedder and an empty result set must not look
//  identical. A lane that is legitimately UNAVAILABLE (the graph has not opened yet) is
//  reported as such in `lanes`, so the caller can say so rather than imply completeness.

import Foundation
import PacificStore

/// One ranked note, with the evidence for why it ranked.
struct NoteSearchResult: Identifiable, Equatable {
    let id: String
    /// Fused RRF score. Comparable WITHIN one result set only — it is a rank statistic, not
    /// a similarity, and carries no meaning across queries.
    let score: Double
    /// Rank in the text lane (nil = the text lane did not return it).
    let textRank: Int?
    /// Rank in the graph lane (nil = the graph lane did not return it).
    let graphRank: Int?
    /// The entities that pulled this note in through the graph lane — the "why", shown as
    /// chips. Empty when only the text lane matched.
    let viaEntities: [ArcEntityRef]

    var matchedBothLanes: Bool { textRank != nil && graphRank != nil }
}

/// A minimal entity reference for display — id, name, type. Avoids leaking the resolver's
/// `ArcEntity` (and its temporal fields) into the view layer.
struct ArcEntityRef: Identifiable, Equatable, Hashable {
    let id: String
    let name: String
    let type: String
}

/// Which lanes actually ran. The view reports this rather than implying a complete search.
struct NoteSearchLanes: Equatable {
    var text = false
    var graph = false
    /// True when the graph lane could not run because the graph has not opened yet.
    var graphUnavailable = false
}

struct NoteSearchOutcome: Equatable {
    var results: [NoteSearchResult] = []
    /// Entities the query itself matched, best-first — the "did you mean this thing" row.
    var entities: [ArcEntityRef] = []
    var lanes = NoteSearchLanes()
}

enum NoteSearchError: Error, LocalizedError {
    case text(String)

    var errorDescription: String? {
        switch self {
        case .text(let m): return "Note search failed: \(m)"
        }
    }
}

@MainActor
final class NoteSearch {
    private let store: LocalStore
    private let graph: GraphService

    init(store: LocalStore, graph: GraphService) {
        self.store = store
        self.graph = graph
    }

    /// Run both lanes and fuse. `asOf` (epoch ms) scopes the GRAPH lane to what was true at
    /// that instant — the graph is bi-temporal, so this is a real time-travelling search;
    /// nil reads the current frame. The text lane has no time axis (a note's text is not
    /// versioned), so `asOf` narrows which notes the graph can reach, never which words match.
    ///
    /// Throws if the text lane fails. The graph lane degrades to "unavailable" rather than
    /// throwing, because a graph still opening is an ordinary state, not a fault.
    func run(query: String, k: Int = 30, asOf: Int64? = nil) async throws -> NoteSearchOutcome {
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !q.isEmpty else { return NoteSearchOutcome() }

        var outcome = NoteSearchOutcome()

        // -- lane 1: the union store's hybrid lexical+vector search over note text --------
        // Ask for more than `k`: the union holds every producer's rows and we keep only
        // note documents, so the note-shaped subset of the top-k is smaller than k.
        let textIDs: [String]
        do {
            let hits = try await store.search(q, k: k * 3, scope: .all)
            textIDs = Self.noteIDs(from: hits)
            outcome.lanes.text = true
        } catch {
            throw NoteSearchError.text("\(error)")
        }

        // -- lane 2: the graph — query -> entities -> the notes that produced them --------
        var entityHits: [ArcEntityRef] = []
        var graphIDs: [String] = []
        var viaEntity: [String: [ArcEntityRef]] = [:]
        if graph.isReady {
            let scored = await graph.semanticEntities(query: q, k: 8, asOf: asOf)
            entityHits = scored.map { ArcEntityRef(id: $0.entity.id, name: $0.entity.name, type: $0.entity.entityType) }
            // Entities are already best-first; walking them in order and appending each
            // entity's notes means a note reached through a BETTER-matching entity takes the
            // better graph rank, and a note reached twice keeps its first (best) position.
            var seen = Set<String>()
            for ref in entityHits {
                for noteID in await graph.noteIDs(forEntity: ref.id) {
                    viaEntity[noteID, default: []].append(ref)
                    if seen.insert(noteID).inserted { graphIDs.append(noteID) }
                }
            }
            outcome.lanes.graph = true
        } else {
            outcome.lanes.graphUnavailable = true
        }

        // -- fuse (RankFusion — the one Swift home of the RRF rule) -----------------------
        let textRank = RankFusion.ranks(textIDs)
        let graphRank = RankFusion.ranks(graphIDs)
        let fusedIDs = Set(textIDs).union(graphIDs)

        var fused: [NoteSearchResult] = []
        fused.reserveCapacity(fusedIDs.count)
        for id in fusedIDs {
            let tr: Int? = textRank[id]
            let gr: Int? = graphRank[id]
            fused.append(NoteSearchResult(
                id: id,
                score: RankFusion.score(tr, gr),
                textRank: tr,
                graphRank: gr,
                viaEntities: Self.deduped(viaEntity[id] ?? [])
            ))
        }
        // Sort by fused score, then by id so the order is TOTAL and stable — an unstable
        // tail would make the list flicker between identical queries.
        fused.sort { $0.score == $1.score ? $0.id < $1.id : $0.score > $1.score }

        outcome.entities = entityHits
        outcome.results = Array(fused.prefix(k))
        return outcome
    }

    // MARK: - fusion helpers

    /// Distinct entities, preserving best-entity-first order (the chips shown on a row).
    private static func deduped(_ refs: [ArcEntityRef]) -> [ArcEntityRef] {
        var out: [ArcEntityRef] = []
        var seen = Set<String>()
        for ref in refs where seen.insert(ref.id).inserted { out.append(ref) }
        return out
    }

    /// Keep only note documents, deduped to one entry per note, preserving hit order.
    /// A hit is per-CHUNK, so a long note can occupy several slots; the best chunk's
    /// position becomes the note's rank.
    private static func noteIDs(from hits: [Hit]) -> [String] {
        var out: [String] = []
        var seen = Set<String>()
        for hit in hits where Metadata.source(hit.metadata) == .note {
            if seen.insert(hit.id).inserted { out.append(hit.id) }
        }
        return out
    }
}
