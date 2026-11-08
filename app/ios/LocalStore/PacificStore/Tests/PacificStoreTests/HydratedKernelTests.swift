//  HydratedKernelTests.swift
//  Scenarios as kernels, proven: context separates what geometry collides
//  (the boilerplate collision), context fuses what geometry cannot reach (the
//  paraphrase), GroupObject co-incidence is a channel, absent data fails a
//  channel, and hydration shapes the ladder to the corpus's actual coverage.

import Foundation
import Testing
@testable import PacificStore

private func unit(_ v: [Float]) -> [Float] { SpectralClustering.normalize(v) }

struct HydratedKernelTests {

    @Test func contextSeparatesTheBoilerplateCollision() {
        // Two rows with NEAR-IDENTICAL vectors (shared boilerplate) but
        // different manufacturers: the identity scenario requires agreement,
        // so they stay apart at fine levels — geometry alone would fuse them.
        let rows = [
            HydratedRow(id: "a", vector: unit([1, 0.01, 0, 0]),
                        meta: ["manufacturer": ["sony"]]),
            HydratedRow(id: "b", vector: unit([1, 0.02, 0, 0]),
                        meta: ["manufacturer": ["philips"]]),
        ]
        let ladder = [
            Scenario(name: "identity", all: [.semantic(maxDistance: 0.05), .agree(key: "manufacturer")]),
            Scenario(name: "geometry", all: [.semantic(maxDistance: 0.05)]),
        ]
        let p = CoarseningPyramid.build(hydrated: rows, ladder: ladder)
        #expect(p.levels[0].grains.count == 2)     // context kept them apart
        #expect(p.levels[1].grains.count == 1)     // pure geometry fuses — the contrast
    }

    @Test func contextFusesTheParaphraseGeometryCannotReach() {
        // Distant vectors (cos ≈ 0.7 — outside any fine kernel) but the same
        // manufacturer and a price within 3%: the contextual scenario fuses.
        let rows = [
            HydratedRow(id: "a", vector: unit([1, 0, 0, 0]),
                        meta: ["manufacturer": ["sony"], "price": ["199.99"]]),
            HydratedRow(id: "b", vector: unit([0.7, 0.714, 0, 0]),
                        meta: ["manufacturer": ["sony"], "price": ["195.00"]]),
        ]
        let ladder = [
            Scenario(name: "identity", all: [.semantic(maxDistance: 0.05)]),
            Scenario(name: "context", all: [.semantic(maxDistance: 0.4),
                                            .agree(key: "manufacturer"),
                                            .within(key: "price", fraction: 0.05)]),
        ]
        let p = CoarseningPyramid.build(hydrated: rows, ladder: ladder)
        #expect(p.levels[0].grains.count == 2)
        #expect(p.levels[1].grains.count == 1)     // hydration reached the paraphrase
        #expect(p.levels[1].grains[0].members == ["a", "b"])
    }

    @Test func groupObjectCoincidenceIsAChannel() {
        // No vectors at all: two rows fuse purely because they share a
        // GroupObject — the folds hydrating the kernel.
        let rows = [
            HydratedRow(id: "a", groups: ["grp:harbour"]),
            HydratedRow(id: "b", groups: ["grp:harbour", "grp:other"]),
            HydratedRow(id: "c", groups: ["grp:elsewhere"]),
        ]
        let ladder = [Scenario(name: "sameRoom", all: [.sharedGroup])]
        let p = CoarseningPyramid.build(hydrated: rows, ladder: ladder)
        let members = p.levels[0].grains.map(\.members)
        #expect(members.contains(["a", "b"]))
        #expect(members.contains(["c"]))
    }

    @Test func absentDataFailsTheChannel() {
        // b has no price: the price channel cannot be verified, so it FAILS —
        // a context you cannot verify is not a context.
        let rows = [
            HydratedRow(id: "a", meta: ["price": ["10.00"]]),
            HydratedRow(id: "b", meta: [:]),
        ]
        let ladder = [Scenario(name: "price", all: [.within(key: "price", fraction: 0.5)])]
        let p = CoarseningPyramid.build(hydrated: rows, ladder: ladder)
        #expect(p.levels[0].grains.count == 2)
    }

    @Test func hydrationShapesTheLadderToCoverage() {
        // manufacturer on every row; colour on one of four (below coverage);
        // price numeric everywhere; no groups, no vectors.
        let rows = (0..<4).map { i in
            HydratedRow(id: "r\(i)",
                        meta: ["manufacturer": ["m\(i)"], "price": ["\(i * 10 + 5)"]]
                            .merging(i == 0 ? ["colour": ["red"]] : [:]) { a, _ in a })
        }
        let ladder = KernelHydration.ladder(rows: rows)
        let channels = ladder.flatMap(\.all)
        #expect(channels.contains { if case .within(key: "price", _) = $0 { return true }; return false })
        #expect(channels.contains { if case .agree(key: "manufacturer") = $0 { return true }; return false })
        #expect(!channels.contains { if case .agree(key: "colour") = $0 { return true }; return false })
        #expect(!channels.contains { if case .semantic = $0 { return true }; return false })  // no vectors
        // The ladder relaxes: strictly fewer channels as levels coarsen.
        let sizes = ladder.map(\.all.count)
        #expect(sizes == sizes.sorted(by: >))
    }

    @Test func coarsenedGrainsCarryTheirHydrationUp() {
        // After a fuse, the grain's meta is the union — a coarse scenario reads
        // coarsened context, exactly like drifted centroids.
        let rows = [
            HydratedRow(id: "a", meta: ["tag": ["x"]], groups: ["g1"]),
            HydratedRow(id: "b", meta: ["tag": ["y"]], groups: ["g1"]),
            HydratedRow(id: "c", meta: ["tag": ["y"]], groups: ["g2"]),
        ]
        let ladder = [
            Scenario(name: "room", all: [.sharedGroup]),                 // a+b fuse
            Scenario(name: "tag", all: [.agree(key: "tag")]),            // (ab) meets c via y
        ]
        let p = CoarseningPyramid.build(hydrated: rows, ladder: ladder)
        #expect(p.levels[0].grains.count == 2)
        #expect(p.levels[1].grains.count == 1)     // the union carried "y" upward
    }

    @Test func deterministicRunToRun() {
        let rows = [
            HydratedRow(id: "a", vector: unit([1, 0, 0, 0]), meta: ["m": ["s"]]),
            HydratedRow(id: "b", vector: unit([1, 0.01, 0, 0]), meta: ["m": ["s"]]),
            HydratedRow(id: "c", vector: unit([0, 1, 0, 0]), meta: ["m": ["t"]]),
        ]
        let ladder = KernelHydration.ladder(rows: rows)
        let one = CoarseningPyramid.build(hydrated: rows, ladder: ladder)
        let two = CoarseningPyramid.build(hydrated: rows, ladder: ladder)
        #expect(one.levels.map { $0.grains.map(\.members) } == two.levels.map { $0.grains.map(\.members) })
        #expect(one.persistence == two.persistence)
    }
}
