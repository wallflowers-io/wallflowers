//  CoarseningPyramidTests.swift
//  The recursive convolution proven on synthetic matter — one operation, both
//  answers: near-identity fuses at fine kernels (dedup), family structure at
//  coarse ones (classes), persistence separates standing waves from noise,
//  mass is conserved through every convolution, and the whole pyramid is
//  deterministic. Plus the grounding case: on cone geometry the pyramid only
//  works in the calibrated frame.

import Foundation
import Testing
@testable import PacificStore

private func unit(_ v: [Float]) -> [Float] { SpectralClustering.normalize(v) }

struct CoarseningPyramidTests {

    /// Two near-duplicates, one distinct row, plus a far blob of three.
    private func matter() -> (ids: [String], vectors: [[Float]]) {
        let vectors: [[Float]] = [
            unit([1, 0.00, 0, 0]),       // a1 ─┐ near-identical
            unit([1, 0.02, 0, 0]),       // a2 ─┘
            unit([0.8, 0.6, 0, 0]),      // b — same family, not a duplicate
            unit([0, 0, 1, 0.00]),       // c1 ─┐
            unit([0, 0, 1, 0.03]),       // c2 ─┼ far blob
            unit([0, 0.03, 1, 0]),       // c3 ─┘
        ]
        return (["a1", "a2", "b", "c1", "c2", "c3"], vectors)
    }

    @Test func fineKernelsFuseOnlyNearIdentity() {
        let (ids, vectors) = matter()
        let p = CoarseningPyramid.build(ids: ids, vectors: vectors)
        #expect(p.duplicateSets.contains(["a1", "a2"]))
        #expect(!p.duplicateSets.contains { $0.contains("b") })   // family ≠ duplicate
    }

    @Test func coarseKernelsFuseFamilies() {
        let (ids, vectors) = matter()
        let p = CoarseningPyramid.build(ids: ids, vectors: vectors)
        // Some mid level holds exactly the two families as two grains.
        let familiesLevel = p.levels.first {
            $0.grains.count == 2 &&
            $0.grains.contains { $0.members == ["a1", "a2", "b"] }
        }
        #expect(familiesLevel != nil)
        // And the final level is one grain of everything.
        #expect(p.levels.last?.grains.count == 1)
        #expect(p.levels.last?.grains.first?.members.count == 6)
    }

    @Test func massIsConservedThroughEveryConvolution() {
        let (ids, vectors) = matter()
        let p = CoarseningPyramid.build(ids: ids, vectors: vectors)
        for level in p.levels {
            #expect(level.grains.map(\.mass).reduce(0, +) == 6)
        }
    }

    @Test func persistenceSeparatesStandingWavesFromLateFusions() {
        let (ids, vectors) = matter()
        let p = CoarseningPyramid.build(ids: ids, vectors: vectors)
        // The far blob's surviving grain forms early and outlives the grain
        // that only exists in the final all-fused level.
        let blob = p.persistence["c1"] ?? 0
        let lone = p.persistence["b"] ?? 0
        #expect(blob > lone)
    }

    @Test func deterministicRunToRun() {
        let (ids, vectors) = matter()
        let a = CoarseningPyramid.build(ids: ids, vectors: vectors)
        let b = CoarseningPyramid.build(ids: ids, vectors: vectors)
        #expect(a.levels.count == b.levels.count)
        for (la, lb) in zip(a.levels, b.levels) {
            #expect(la.grains.map(\.id) == lb.grains.map(\.id))
            #expect(la.grains.map(\.members) == lb.grains.map(\.members))
        }
        #expect(a.persistence == b.persistence)
    }

    @Test func emptyAndSingletonAreHonest() {
        #expect(CoarseningPyramid.build(ids: [], vectors: []).levels.isEmpty)
        let one = CoarseningPyramid.build(ids: ["x"], vectors: [unit([1, 0])])
        #expect(one.levels.first?.grains.count == 1)
        #expect(one.duplicateSets.isEmpty)
    }

    @Test func conePyramidNeedsTheCalibratedFrame() {
        // A tight cone (residual 0.15): raw cross-group cosine distance ≈ 0.02
        // sits INSIDE the dedup kernel, so the finest kernel fuses everything —
        // unrelated texts read as duplicates, the anisotropy failure exactly.
        // Calibrated, the same matter holds two pure families.
        var rng = SplitMix64(seed: 41)
        func cone(_ group: Int, _ n: Int) -> [[Float]] {
            (0..<n).map { _ in
                var v = [Float](repeating: 0, count: 8)
                v[0] = 1; v[1 + group] = 0.15
                for d in 0..<8 { v[d] += (Float(rng.next() % 1000) / 1000 - 0.5) * 0.02 }
                return unit(v)
            }
        }
        let vectors = cone(0, 10) + cone(1, 10)
        let ids = (0..<20).map { String(format: "e%02d", $0) }
        let cal = SpaceCalibration.fit(vectors)!

        let raw = CoarseningPyramid.build(ids: ids, vectors: vectors)
        #expect(raw.levels.first?.grains.count == 1)   // dedup destroyed by the cone

        let grounded = CoarseningPyramid.build(ids: ids, vectors: vectors, calibration: cal)
        #expect((grounded.levels.first?.grains.count ?? 0) >= 2)
        let pureFamilies = grounded.levels.first { level in
            level.grains.count == 2 &&
            level.grains.allSatisfy { g in
                g.members.allSatisfy { $0 < "e10" } || g.members.allSatisfy { $0 >= "e10" }
            }
        }
        #expect(pureFamilies != nil)
    }

    // MARK: - The structural kernel (no vectors anywhere)

    private func edge(_ id: String, _ s: String, _ v: String, _ d: String) -> GraphEdgeRow {
        GraphEdgeRow(id: id, src: s, verb: v, dst: d,
                     provenance: .folded(object: "o"), time: BiTime(validAt: 0, createdAt: 0))
    }

    @Test func structuralKernelFusesGraphVouchedDuplicates() {
        // p1 and p2 attend the same events and belong to the same group —
        // identical incidence profiles, no vectors involved. p3 is elsewhere.
        let edges = [
            edge("e1", "p1", "attends", "ev1"), edge("e2", "p2", "attends", "ev1"),
            edge("e3", "p1", "attends", "ev2"), edge("e4", "p2", "attends", "ev2"),
            edge("e5", "p1", "member_of", "g1"), edge("e6", "p2", "member_of", "g1"),
            edge("e7", "p3", "member_of", "g2"), edge("e8", "p3", "attends", "ev3"),
        ]
        let p = CoarseningPyramid.buildFromGraph(nodeIds: [], edges: edges)
        #expect(p.duplicateSets.contains { $0.contains("p1") && $0.contains("p2") })
        #expect(!p.duplicateSets.contains { $0.contains("p3") })
    }

    @Test func structuralCoarseningFindsCommunitiesThenEverything() {
        // Two four-cliques joined by one bridge: interiors fuse before the whole.
        var edges: [GraphEdgeRow] = []
        var n = 0
        func clique(_ names: [String]) {
            for i in 0..<names.count {
                for j in (i + 1)..<names.count {
                    edges.append(edge("c\(n)", names[i], "knows", names[j])); n += 1
                }
            }
        }
        clique(["a1", "a2", "a3", "a4"])
        clique(["b1", "b2", "b3", "b4"])
        edges.append(edge("bridge", "a1", "knows", "b1"))
        let p = CoarseningPyramid.buildFromGraph(nodeIds: [], edges: edges)

        let communityLevel = p.levels.first { level in
            level.grains.count == 2 &&
            level.grains.allSatisfy { g in
                g.members.allSatisfy { $0.hasPrefix("a") } || g.members.allSatisfy { $0.hasPrefix("b") }
            }
        }
        #expect(communityLevel != nil)
        for level in p.levels {
            #expect(level.grains.map(\.mass).reduce(0, +) == 8)   // mass conserved
        }
    }

    @Test func structuralDeterminism() {
        let edges = [
            edge("e1", "x", "knows", "y"), edge("e2", "y", "knows", "z"),
            edge("e3", "x", "knows", "z"),
        ]
        let a = CoarseningPyramid.buildFromGraph(nodeIds: [], edges: edges)
        let b = CoarseningPyramid.buildFromGraph(nodeIds: [], edges: edges)
        #expect(a.levels.count == b.levels.count)
        for (la, lb) in zip(a.levels, b.levels) {
            #expect(la.grains.map(\.members) == lb.grains.map(\.members))
        }
    }

    @Test func classesReadOrthogonallyPerLevel() {
        let (ids, vectors) = matter()
        let p = CoarseningPyramid.build(ids: ids, vectors: vectors)
        let anchors = [KindAnchor(kind: OntologyKind(name: "A", definition: "x-ish"),
                                  vector: unit([1, 0.2, 0, 0])),
                       KindAnchor(kind: OntologyKind(name: "C", definition: "z-ish"),
                                  vector: unit([0, 0, 1, 0]))]
        guard let familiesIdx = p.levels.firstIndex(where: { $0.grains.count == 2 }) else {
            Issue.record("no two-family level")
            return
        }
        let classes = p.classes(atLevel: familiesIdx, anchors: anchors, calibration: nil)
        for (grain, kind) in classes {
            if grain.members.contains("a1") { #expect(kind["A"]! > kind["C"]!) }
            if grain.members.contains("c1") { #expect(kind["C"]! > kind["A"]!) }
        }
    }
}
