//  TransmissionTests.swift
//  The CORE TRANSMISSION proven on synthetic geometry — no store, no app, no model.
//
//  What must hold: the eigengap READS k off well-separated blobs; clustering is
//  deterministic across runs; the ontology parser extracts the .ttl's real grammar
//  and fails loudly on holes; category stays orthogonal to instance (the affinity
//  vector survives, uncollapsed); and sentiment transduction ranks a charged
//  cluster above a neutral one of the same shape.

import Foundation
import Testing
@testable import PacificStore

// Deterministic blob generator — SplitMix64, never SystemRandom.
private func blob(center: [Float], n: Int, spread: Float, seed: UInt64) -> [[Float]] {
    var rng = SplitMix64(seed: seed)
    return (0..<n).map { _ in
        center.map { c in
            let u = Float(rng.next() % 10_000) / 10_000 - 0.5
            return c + u * spread
        }
    }
}

struct SpectralClusteringTests {

    private func threeBlobs() -> (ids: [String], vectors: [[Float]]) {
        // Three well-separated directions in 8-d, 12 points each.
        var centers = [[Float]](repeating: [Float](repeating: 0, count: 8), count: 3)
        centers[0][0] = 1; centers[1][3] = 1; centers[2][6] = 1
        var vectors: [[Float]] = []
        for (i, c) in centers.enumerated() {
            vectors += blob(center: c, n: 12, spread: 0.15, seed: UInt64(100 + i))
        }
        return ((0..<vectors.count).map { "e\($0)" }, vectors)
    }

    @Test func eigengapReadsKOffTheGeometry() {
        let (ids, vectors) = threeBlobs()
        let result = SpectralClustering.cluster(ids: ids, vectors: vectors)
        #expect(result.k == 3)
    }

    @Test func blobsLandInPureClusters() {
        let (ids, vectors) = threeBlobs()
        let result = SpectralClustering.cluster(ids: ids, vectors: vectors)
        // Every ground-truth blob maps to exactly one cluster label.
        for b in 0..<3 {
            let labels = Set(result.assignments[(b * 12)..<((b + 1) * 12)])
            #expect(labels.count == 1, "blob \(b) split across \(labels)")
        }
        #expect(Set(result.assignments).count == 3)
    }

    @Test func identicalInputIdenticalOutput() {
        let (ids, vectors) = threeBlobs()
        let one = SpectralClustering.cluster(ids: ids, vectors: vectors)
        let two = SpectralClustering.cluster(ids: ids, vectors: vectors)
        #expect(one == two)
    }

    @Test func tinyInputCollapsesHonestly() {
        let result = SpectralClustering.cluster(ids: ["a"], vectors: [[1, 0]])
        #expect(result.k == 1)
        #expect(result.assignments == [0])
    }
}

struct TransmissionFrameTests {

    /// A fake embedder/gauge world: anchors on axes, episodes near them.
    private func world() -> (episodes: [TransmissionEpisode], anchors: [KindAnchor]) {
        let personAnchor = KindAnchor(kind: OntologyKind(name: "Person", definition: "a human"),
                                      vector: [1, 0, 0, 0])
        let placeAnchor = KindAnchor(kind: OntologyKind(name: "Place", definition: "a where"),
                                     vector: [0, 1, 0, 0])
        var episodes: [TransmissionEpisode] = []
        // A charged person-shaped cluster…
        for (i, v) in blob(center: [1, 0.1, 0, 0], n: 8, spread: 0.1, seed: 7).enumerated() {
            episodes.append(TransmissionEpisode(id: "p\(i)", vector: v,
                                                text: "love this", ts: UInt64(i)))
        }
        // …and a neutral place-shaped one.
        for (i, v) in blob(center: [0.1, 1, 0, 0], n: 8, spread: 0.1, seed: 8).enumerated() {
            episodes.append(TransmissionEpisode(id: "q\(i)", vector: v,
                                                text: "the corner", ts: UInt64(i)))
        }
        return (episodes, [personAnchor, placeAnchor])
    }

    private let gauge = Transmission.Gauge(name: "fake") { $0.contains("love") ? 0.9 : 0.0 }

    @Test func categoryStaysOrthogonalToInstance() {
        let (episodes, anchors) = world()
        let frame = Transmission.frame(episodes: episodes, anchors: anchors, gauge: gauge)
        #expect(frame.candidates.count == 2)
        // Every candidate carries the FULL affinity vector — both kinds present,
        // whatever it most resembles.
        for c in frame.candidates {
            #expect(c.kindAffinity.count == 2)
        }
    }

    @Test func sentimentTransducesIntoTheRanking() {
        let (episodes, anchors) = world()
        let frame = Transmission.frame(episodes: episodes, anchors: anchors, gauge: gauge)
        // The charged cluster is the person-shaped one; it must top the Person list
        // and carry its charge.
        let personBest = frame.ranked["Person"]!.first!
        let best = frame.candidates[personBest]
        #expect(best.episodeIds.first?.hasPrefix("p") == true)
        #expect(best.sentiment > 0.5)
        // The neutral cluster's rank score is zero everywhere: no charge, no rank.
        let neutral = frame.candidates.first { $0.episodeIds.first?.hasPrefix("q") == true }!
        #expect(neutral.rankScore.values.allSatisfy { $0 == 0 })
    }

    @Test func frameNamesItsGauge() {
        let (episodes, anchors) = world()
        let frame = Transmission.frame(episodes: episodes, anchors: anchors, gauge: gauge)
        #expect(frame.sentimentGauge == "fake")
    }

    @Test func emptyEpisodesEmptyFrame() {
        let frame = Transmission.frame(episodes: [], anchors: [], gauge: gauge)
        #expect(frame.candidates.isEmpty)
        #expect(frame.ranked.isEmpty)
    }
}
