//  RankFusion.swift
//  Reciprocal Rank Fusion (Cormack et al. 2009) — the ONE Swift home of the rule.
//
//  Every hybrid search in the app fuses lanes with RRF: NoteSearch (text × graph)
//  and the CHAT search bar (exact × cosine). The rule is load-bearing — lanes fuse
//  by RANK, never by raw score, because a cosine similarity, a BM25 fusion and a
//  substring match share no scale, and any numeric weighting of them is a
//  fabricated commensurability. It therefore lives exactly once; lodedb-graph
//  carries the Rust port of the same rule for its own reranker.
//
//  K = 60 is the constant from the original paper (and the engine's reranker): it
//  flattens the difference between ranks 1 and 2 enough that no single lane can
//  dominate a fused list.

import Foundation

enum RankFusion {
    /// RRF's damping constant (Cormack et al. 2009).
    static let k = 60.0

    /// One lane's contribution: 1/(K + rank), or 0 where the lane missed the item.
    static func contribution(_ rank: Int?) -> Double {
        guard let rank else { return 0 }
        return 1.0 / (k + Double(rank))
    }

    /// The fused score of one item across lanes: Σ 1/(K + rank) over the lanes
    /// that returned it. Comparable WITHIN one result set only — it is a rank
    /// statistic, not a similarity.
    static func score(_ ranks: Int?...) -> Double {
        ranks.reduce(0) { $0 + contribution($1) }
    }

    /// id → 1-based rank from a best-first list; first occurrence wins.
    static func ranks(_ ids: [String]) -> [String: Int] {
        var out: [String: Int] = [:]
        for (i, id) in ids.enumerated() where out[id] == nil { out[id] = i + 1 }
        return out
    }
}
