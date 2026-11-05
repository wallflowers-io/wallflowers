//  GraphResolutionTests.swift
//  The resolution calculus, unit by unit and composed — no store, no app.
//
//  What must hold: the L0 atoms answer bitemporal questions exactly (including
//  time travel — an old τ sees the superseded world); the source-disjointness
//  veto refuses the classic chain-merge; namesake Persons survive semantic
//  similarity; minted beats reference at canonicalization; duplicate edges
//  coalesce their valid intervals; a closed-vs-open assertion is surfaced as a
//  conflict, never averaged; and the whole operation is deterministic.

import Foundation
import Testing
@testable import PacificStore

private func bt(_ v: UInt64, _ inv: UInt64? = nil, c: UInt64 = 1, x: UInt64? = nil) -> BiTime {
    BiTime(validAt: v, invalidAt: inv, createdAt: c, expiredAt: x)
}

struct L0Tests {

    @Test func bitemporalAtomsAnswerExactly() {
        let b = bt(100, 200, c: 150, x: 400)
        #expect(L0.alive(b, at: 100) && L0.alive(b, at: 199))
        #expect(!L0.alive(b, at: 99) && !L0.alive(b, at: 200))
        #expect(L0.known(b, at: 150) && L0.known(b, at: 399))
        #expect(!L0.known(b, at: 149) && !L0.known(b, at: 400))
        #expect(L0.current(b, t: 150, tau: 200))
        #expect(!L0.current(b, t: 250, tau: 200))   // no longer true in the world
        #expect(!L0.current(b, t: 150, tau: 450))   // superseded record
    }

    @Test func timeTravelSeesTheSupersededWorld() {
        // The record was corrected at τ=400: old τ sees v1, new τ sees v2.
        let v1 = bt(100, nil, c: 100, x: 400)
        let v2 = bt(100, 300, c: 400)
        #expect(L0.known(v1, at: 399) && !L0.known(v2, at: 399))
        #expect(!L0.known(v1, at: 400) && L0.known(v2, at: 400))
    }

    @Test func latticeSubsumes() {
        let lat = ["Land": "Place"]
        #expect(L0.compatible("Land", "Place", lattice: lat))
        #expect(L0.compatible("Place", "Land", lattice: lat))
        #expect(!L0.compatible("Person", "Place", lattice: lat))
    }

    @Test func sourceDisjointnessAxiom() {
        let ra1 = Provenance.reference(source: "ra", key: "1")
        let ra2 = Provenance.reference(source: "ra", key: "2")
        let art = Provenance.reference(source: "artrabbit", key: "9")
        #expect(L0.sourceDisjoint(ra1, ra2))          // same source, listed apart
        #expect(!L0.sourceDisjoint(ra1, art))          // different sources may co-refer
        #expect(!L0.sourceDisjoint(.minted(author: "a"), .minted(author: "a")))
    }
}

struct ResolutionTests {

    private func place(_ id: String, _ label: String, prov: Provenance,
                       at: (Double, Double) = (51.53, -0.08),
                       time: BiTime = bt(0)) -> GraphNodeRow {
        GraphNodeRow(id: id, kind: "Place", label: label, provenance: prov,
                     time: time, coordinate: (lat: at.0, lon: at.1))
    }

    @Test func lexGeoDeduplicatesAVenueAndMintedWins() {
        let nodes = [
            place("ref1", "Corsica Studios", prov: .reference(source: "ra", key: "r1"),
                  time: bt(0, nil, c: 5)),
            place("mint1", "Corsica Studios", prov: .minted(author: "ralph"),
                  time: bt(0, nil, c: 9)),
        ]
        let view = GraphResolution.resolve(nodes: nodes, edges: [], t: 10, tau: 10)
        #expect(view.canon["ref1"] == "mint1")          // minted wins despite later created
        #expect(view.nodes.count == 1)
        #expect(view.identifications.first?.rule == "lex+geo")
    }

    @Test func vetoRefusesTheChainMerge() {
        // Two rows the SAME source lists apart, both matching a third by name+geo:
        // the closure must refuse to fuse all three; the refused link becomes a proposal.
        let nodes = [
            place("ra1", "The Yard", prov: .reference(source: "ra", key: "1")),
            place("ra2", "The Yard", prov: .reference(source: "ra", key: "2")),
            place("m1", "The Yard", prov: .minted(author: "ralph")),
        ]
        let view = GraphResolution.resolve(nodes: nodes, edges: [], t: 10, tau: 10)
        #expect(Set(view.canon.values).count == 2)      // never one class of three
        #expect(!view.proposals.isEmpty)                // the refused union is surfaced
    }

    @Test func namesakePersonsSurviveSemanticSimilarity() {
        // Same name, high calibrated sem, ZERO shared neighbours → no rule fires;
        // the pair lands in proposals for a human, not in the canon map.
        let corpus = (0..<10).map { i -> [Float] in
            var v = [Float](repeating: 0, count: 8); v[0] = 1; v[i % 4 + 1] = 0.3; return v
        }
        let cal = SpaceCalibration.fit(corpus)!
        var va = [Float](repeating: 0, count: 8); va[0] = 1; va[1] = 0.3
        let a = GraphNodeRow(id: "p1", kind: "Person", label: "Sam Park",
                             provenance: .folded(object: "grpA"), time: bt(0), vector: va)
        let b = GraphNodeRow(id: "p2", kind: "Person", label: "Sam Park",
                             provenance: .folded(object: "grpB"), time: bt(0), vector: va)
        let view = GraphResolution.resolve(nodes: [a, b], edges: [], t: 10, tau: 10,
                                           calibration: cal)
        #expect(view.canon["p1"] == "p1" && view.canon["p2"] == "p2")
        #expect(view.proposals.contains { $0.a == "p1" && $0.b == "p2" })
    }

    @Test func sharedNeighboursResolveThePersonTheGraphVouchesFor() {
        let a = GraphNodeRow(id: "p1", kind: "Person", label: "Sam Park",
                             provenance: .folded(object: "grpA"), time: bt(0))
        let b = GraphNodeRow(id: "p2", kind: "Person", label: "Sam Park",
                             provenance: .folded(object: "grpB"), time: bt(0))
        let edges = [
            GraphEdgeRow(id: "e1", src: "p1", verb: "member_of", dst: "g9",
                         provenance: .folded(object: "grpA"), time: bt(0)),
            GraphEdgeRow(id: "e2", src: "p2", verb: "member_of", dst: "g9",
                         provenance: .folded(object: "grpB"), time: bt(0)),
        ]
        let view = GraphResolution.resolve(nodes: [a, b], edges: edges, t: 10, tau: 10)
        #expect(view.canon["p2"] == "p1")
        #expect(view.identifications.first?.rule == "lex+neigh")
        // The two member_of edges collapse onto the canonical pair.
        #expect(view.edges.count == 1)
    }

    @Test func edgeIntervalsCoalesce() {
        let n = [place("v1", "Corsica Studios", prov: .minted(author: "r"))]
        let edges = [
            GraphEdgeRow(id: "e1", src: "v1", verb: "hosts", dst: "v1",
                         provenance: .minted(author: "r"), time: bt(0, 100)),
            GraphEdgeRow(id: "e2", src: "v1", verb: "hosts", dst: "v1",
                         provenance: .minted(author: "r"), time: bt(100, 200)),
            GraphEdgeRow(id: "e3", src: "v1", verb: "hosts", dst: "v1",
                         provenance: .minted(author: "r"), time: bt(300, 400)),
        ]
        // FILTER precedes COALESCE — that is the contract: only the edge alive
        // at t takes part, so disjoint intervals never falsely fuse across t.
        let at50 = GraphResolution.resolve(nodes: n, edges: edges, t: 50, tau: 10)
        #expect(at50.edges.count == 1 && at50.edges.first?.id == "e1")
        let at150 = GraphResolution.resolve(nodes: n, edges: edges, t: 150, tau: 10)
        #expect(at150.edges.count == 1 && at150.edges.first?.id == "e2")
    }

    @Test func openVersusClosedAssertionIsAConflictNotAMerge() {
        let n = [place("v1", "Corsica Studios", prov: .minted(author: "r"))]
        let edges = [
            GraphEdgeRow(id: "closed", src: "v1", verb: "hosts", dst: "v1",
                         provenance: .folded(object: "o1"), time: bt(0, 100)),
            GraphEdgeRow(id: "open", src: "v1", verb: "hosts", dst: "v1",
                         provenance: .folded(object: "o2"), time: bt(50)),
        ]
        let view = GraphResolution.resolve(nodes: n, edges: edges, t: 60, tau: 10)
        #expect(view.conflicts.count == 1)
        #expect(view.conflicts.first?.reason.contains("open") == true)
    }

    @Test func deterministicEndToEnd() {
        let nodes = [
            place("ref1", "Corsica Studios", prov: .reference(source: "ra", key: "r1")),
            place("mint1", "Corsica Studios", prov: .minted(author: "ralph")),
            place("ra1", "The Yard", prov: .reference(source: "ra", key: "1")),
            place("ra2", "The Yard", prov: .reference(source: "ra", key: "2")),
        ]
        let one = GraphResolution.resolve(nodes: nodes, edges: [], t: 10, tau: 10)
        let two = GraphResolution.resolve(nodes: nodes, edges: [], t: 10, tau: 10)
        #expect(one.canon == two.canon)
        #expect(one.identifications.map(\.rule) == two.identifications.map(\.rule))
        #expect(one.proposals.map(\.a) == two.proposals.map(\.a))
    }
}
