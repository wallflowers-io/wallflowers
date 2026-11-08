//  Transmission.swift
//  THE CORE TRANSMISSION (7 Aug directive): episodes → instances → categories →
//  ranked candidates, all on device, no model in the loop.
//
//      LodeDB is required to categorise via spectral analysis of embedding
//      space / ingested episodes. K-means clustering is used to resolve People,
//      Places, Events, and Groups. All are ranked via sentiment transduced via
//      pacific-ontology.ttl. Category may be orthogonal to instance.
//
//  What that means as machinery, layer by layer:
//    INSTANCE   SpectralClustering partitions the episode vectors by the space's
//               own geometry — a cluster is a CANDIDATE INSTANCE (this person,
//               this place), not a category.
//    CATEGORY   an orthogonal read: each cluster's original-space centroid is
//               projected against the kind anchors the caller supplies
//               (KindAnchor). The affinity VECTOR is the output —
//               never an argmax. Category may be orthogonal to instance.
//    RANK       sentiment, scored per episode on device (Apple NL — a gauge, not
//               a generator), aggregated per cluster, and TRANSDUCED through the
//               kind affinities: a cluster's charge distributes across the kinds
//               it resembles, so each kind ranks its own candidates by how much
//               felt life points at them.
//
//  Doctrine (ontology v0.4, verbatim): clustering lenses are "never stored,
//  exactly as Themes are centroids and not assertions." A TransmissionFrame is
//  COMPUTED and shown; minting stays a user act on LIFE (4 Aug ruling — this
//  lens is the no-LLM resolution path, not a return of extraction). Everything
//  is deterministic given the same episodes, except sentiment's model version —
//  which is why the frame records it.

import Foundation
#if canImport(NaturalLanguage)
import NaturalLanguage
#endif

// MARK: - Input

/// One ingested episode as the lens reads it: the union-store row's stable id,
/// its vector (LodeDBCore.getVectors — quantization-reconstructed, same space),
/// its retained text (sentiment), and its event time (recency windows are the
/// caller's concern; the lens takes what it's given).
public struct TransmissionEpisode: Sendable {
    public let id: String
    public let vector: [Float]
    public let text: String
    public let ts: UInt64

    public init(id: String, vector: [Float], text: String, ts: UInt64) {
        self.id = id
        self.vector = vector
        self.text = text
        self.ts = ts
    }
}

// MARK: - Output

/// One resolved candidate instance: a cluster, its members, its orthogonal
/// category read, and its sentiment charge.
public struct TransmissionCandidate: Sendable {
    public let episodeIds: [String]          // members, input order preserved
    public let centroid: [Float]             // original-space, unit length
    /// kind name → cosine affinity of the centroid to that kind's anchor.
    /// THE VECTOR IS THE ANSWER — collapsing it to a winner is the caller's
    /// mistake to refuse.
    public let kindAffinity: [String: Float]
    /// Mean episode sentiment in [-1, +1] (0 when no gauge is available).
    public let sentiment: Float
    /// kind name → this candidate's transduced rank score for that kind's list:
    /// |sentiment| · affinity · log2(1 + members). Charge times resemblance
    /// times evidence mass.
    public let rankScore: [String: Float]
}

/// The computed lens frame — shown, never stored (Themes-are-centroids rule).
public struct TransmissionFrame: Sendable {
    public let candidates: [TransmissionCandidate]
    /// kind name → candidate indices, best first — the per-kind shortlists the
    /// mint surfaces read.
    public let ranked: [String: [Int]]
    /// The eigenvalues that justified k — the lens shows its working.
    public let eigenvalues: [Double]
    /// What produced sentiment ("apple-nl" | "none" | an injected gauge's name):
    /// the one non-deterministic ingredient, named so a frame is reproducible
    /// in context.
    public let sentimentGauge: String
}

// MARK: - The lens

public enum Transmission {

    /// Sentiment gauge: text → [-1, +1]. Injectable for tests and for platforms
    /// without Apple NL; `named` labels the frame.
    public struct Gauge {
        public let name: String
        public let score: (String) -> Float
        public init(name: String, score: @escaping (String) -> Float) {
            self.name = name
            self.score = score
        }

        /// Apple's on-device sentiment — a gauge, not a generator; no LLM.
        public static func appleNL() -> Gauge {
            #if canImport(NaturalLanguage)
            return Gauge(name: "apple-nl") { text in
                let tagger = NLTagger(tagSchemes: [.sentimentScore])
                tagger.string = text
                let (tag, _) = tagger.tag(at: text.startIndex,
                                          unit: .paragraph, scheme: .sentimentScore)
                return Float(tag?.rawValue ?? "0") ?? 0
            }
            #else
            return Gauge(name: "none") { _ in 0 }
            #endif
        }
    }

    /// Run the lens: cluster the episodes, read each cluster's category
    /// orthogonally against `anchors`, rank per kind by transduced sentiment.
    ///
    /// Pass `calibration` for REAL embedding vectors: instance structure and
    /// category affinities are then read in the centered frame, where 0 means
    /// unrelated — which raw cosine never meant in an anisotropic space.
    public static func frame(episodes: [TransmissionEpisode],
                             anchors: [KindAnchor],
                             params: SpectralParams = SpectralParams(),
                             gauge: Gauge = .appleNL(),
                             calibration: SpaceCalibration? = nil) -> TransmissionFrame {
        guard !episodes.isEmpty else {
            return TransmissionFrame(candidates: [], ranked: [:], eigenvalues: [],
                                     sentimentGauge: gauge.name)
        }

        // Into the calibrated frame once — instance and category both read here.
        let framed: [[Float]] = calibration.map { cal in
            episodes.map { cal.center($0.vector) }
        } ?? episodes.map { SpectralClustering.normalize($0.vector) }

        // INSTANCE — spectral structure over the framed episode vectors.
        let result = SpectralClustering.cluster(ids: episodes.map(\.id),
                                                vectors: framed,
                                                params: params)

        var members = [[Int]](repeating: [], count: max(result.k, 1))
        for (i, c) in result.assignments.enumerated() { members[c].append(i) }

        let unitAnchors = anchors.map { anchor in
            (anchor.kind.name,
             calibration.map { $0.center(anchor.vector) } ?? SpectralClustering.normalize(anchor.vector))
        }

        var candidates: [TransmissionCandidate] = []
        for rows in members where !rows.isEmpty {
            // Framed-space centroid: the spectral embedding clusters, the framed
            // (centered-or-unit) space carries meaning — anchors are read HERE,
            // in the same frame they were moved into above.
            let dim = framed[rows[0]].count
            var centroid = [Float](repeating: 0, count: dim)
            for r in rows {
                let v = framed[r]
                for d in 0..<dim { centroid[d] += v[d] }
            }
            centroid = SpectralClustering.normalize(centroid.map { $0 / Float(rows.count) })

            // CATEGORY — the orthogonal read. The whole vector survives.
            var affinity: [String: Float] = [:]
            for (name, anchor) in unitAnchors {
                affinity[name] = Float(SpectralClustering.dot(centroid, anchor))
            }

            // RANK — sentiment charge, transduced through the affinities.
            let charge = rows.map { gauge.score(episodes[$0].text) }.reduce(0, +) / Float(rows.count)
            let mass = log2(Float(1 + rows.count))
            var score: [String: Float] = [:]
            for (name, a) in affinity { score[name] = abs(charge) * max(0, a) * mass }

            candidates.append(TransmissionCandidate(episodeIds: rows.map { episodes[$0].id },
                                                    centroid: centroid,
                                                    kindAffinity: affinity,
                                                    sentiment: charge,
                                                    rankScore: score))
        }

        // Per-kind shortlists, best first; ties broken by first episode id so the
        // frame is stable end to end.
        var ranked: [String: [Int]] = [:]
        for (name, _) in unitAnchors {
            ranked[name] = candidates.indices.sorted {
                let a = candidates[$0].rankScore[name] ?? 0
                let b = candidates[$1].rankScore[name] ?? 0
                if a != b { return a > b }
                return (candidates[$0].episodeIds.first ?? "") < (candidates[$1].episodeIds.first ?? "")
            }
        }

        return TransmissionFrame(candidates: candidates, ranked: ranked,
                                 eigenvalues: result.eigenvalues,
                                 sentimentGauge: gauge.name)
    }
}
