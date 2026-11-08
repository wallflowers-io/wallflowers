//  CandidateRegisterTests.swift
//  The quantum register, proven: superpositions are held (never argmaxed),
//  noise never registers, identity amplitudes read against the existing graph,
//  led_to needs strict precedence AND alignment AND lean, and collapse is a
//  measurement whose consequences propagate through the entangled edges.

import Foundation
import Testing
@testable import PacificStore

private let day: UInt64 = 24 * 3_600_000

private func unit(_ v: [Float]) -> [Float] { SpectralClustering.normalize(v) }

struct CandidateRegisterTests {

    /// Two standing clusters in stream time: a planning burst, then a doing
    /// burst in the same direction; plus one singleton of noise elsewhere.
    private func world() -> (episodes: [TransmissionEpisode], anchors: [KindAnchor]) {
        var episodes: [TransmissionEpisode] = []
        for i in 0..<5 {
            episodes.append(TransmissionEpisode(
                id: "plan\(i)", vector: unit([1, Float(i) * 0.005, 0, 0]),
                text: "let's plan the pier gathering, we will book it soon",
                ts: 90 * day + UInt64(i) * day / 2))
        }
        // Distinct from the planning burst (cosine ≈ 0.8 — aligned, not a
        // duplicate: outside every fine kernel, inside the causal floor).
        for i in 0..<5 {
            episodes.append(TransmissionEpisode(
                id: "held\(i)", vector: unit([0.8, 0.6, Float(i) * 0.005, 0]),
                text: "the pier gathering", ts: 100 * day + UInt64(i) * day / 2))
        }
        episodes.append(TransmissionEpisode(
            id: "noise", vector: unit([0, 0, 0, 1]), text: "stray", ts: 95 * day))
        let anchors = [
            KindAnchor(kind: OntologyKind(name: "Event", definition: "a dated happening"),
                       vector: unit([1, 0.1, 0, 0])),
            KindAnchor(kind: OntologyKind(name: "Place", definition: "a located where"),
                       vector: unit([0.9, 0, 0.3, 0])),
        ]
        return (episodes, anchors)
    }

    @Test func standingWavesRegisterNoiseDoesNot() {
        let (episodes, anchors) = world()
        let r = CandidateRegister.identify(episodes: episodes, anchors: anchors)
        #expect(!r.nodes.isEmpty)
        #expect(!r.nodes.contains { $0.members == ["noise"] })   // singletons never register
    }

    @Test func kindSuperpositionIsHeldNotCollapsed() {
        let (episodes, anchors) = world()
        let r = CandidateRegister.identify(episodes: episodes, anchors: anchors)
        // The clusters sit between the two anchors: both kinds must survive in
        // the distribution — the register never argmaxes.
        let ambiguous = r.nodes.filter { $0.kinds.count >= 2 }
        #expect(!ambiguous.isEmpty)
        for n in r.nodes where !n.kinds.isEmpty {
            let total = n.kinds.values.reduce(0, +)
            #expect(abs(total - 1) < 0.001)   // normalized distribution, all of it kept
        }
    }

    @Test func identityAmplitudesReadAgainstTheExistingGraph() {
        let (episodes, anchors) = world()
        let near = GraphNodeRow(id: "existing-pier", kind: "Event", label: "Pier gathering",
                                provenance: .minted(author: "r"),
                                time: BiTime(validAt: 0, createdAt: 0),
                                vector: unit([1, 0.02, 0.02, 0]))
        let r = CandidateRegister.identify(episodes: episodes, anchors: anchors,
                                           existing: [near])
        let withSame = r.nodes.filter { !$0.identity.sameAs.isEmpty }
        #expect(!withSame.isEmpty)
        for n in withSame {
            #expect(n.identity.sameAs.first?.id == "existing-pier")
            #expect(n.identity.newAmplitude < 1)   // "new" lost amplitude to "same"
        }
        #expect(r.edges.contains { $0.verb == "same_as" && $0.to == "existing-pier" })
    }

    @Test func ledToNeedsPrecedenceAlignmentAndLean() {
        let (episodes, anchors) = world()
        let r = CandidateRegister.identify(episodes: episodes, anchors: anchors)
        let led = r.edges.filter { $0.verb == "led_to" }
        // The planning burst (earlier, towards-leaning, aligned) leads to the
        // held burst — and nothing leads to or from the unregistered noise.
        #expect(led.contains { $0.from.hasPrefix("plan") && $0.to.hasPrefix("held") })
        #expect(!led.contains { $0.from.contains("noise") || $0.to.contains("noise") })
        // Never the reverse: precedence is strict.
        #expect(!led.contains { $0.from.hasPrefix("held") && $0.to.hasPrefix("plan") })
    }

    @Test func collapseIsAMeasurementAndEdgesAreEntangled() {
        let (episodes, anchors) = world()
        let near = GraphNodeRow(id: "existing-pier", kind: "Event", label: "Pier gathering",
                                provenance: .minted(author: "r"),
                                time: BiTime(validAt: 0, createdAt: 0),
                                vector: unit([1, 0.02, 0.05, 0]))
        let r = CandidateRegister.identify(episodes: episodes, anchors: anchors,
                                           existing: [near])
        guard let led = r.edges.first(where: { $0.verb == "led_to" }) else {
            Issue.record("no led_to edge to collapse through")
            return
        }
        let target = led.to
        let collapsed = r.collapsing(target, toExisting: "existing-pier")
        // The candidate left the register; its same_as alternatives dissolved;
        // the led_to edge re-targeted to the existing node.
        #expect(!collapsed.nodes.contains { $0.id == target })
        #expect(!collapsed.edges.contains { $0.verb == "same_as" && $0.from == target })
        #expect(collapsed.edges.contains { $0.verb == "led_to" && $0.to == "existing-pier" })
    }

    @Test func extentsSuperposeAcrossScalesWithSharedLineage() {
        let (episodes, anchors) = world()
        let r = CandidateRegister.identify(episodes: episodes, anchors: anchors)
        // The same surviving grain appears at more than one scale (its extent
        // grows as the kernel coarsens) — all extents held, linked by grain id.
        let byGrain = Dictionary(grouping: r.nodes, by: \.grain)
        #expect(byGrain.contains { $0.value.count > 1 })
    }

    @Test func deterministicAndHonestWhenEmpty() {
        let (episodes, anchors) = world()
        #expect(CandidateRegister.identify(episodes: [], anchors: anchors).nodes.isEmpty)
        let a = CandidateRegister.identify(episodes: episodes, anchors: anchors)
        let b = CandidateRegister.identify(episodes: episodes, anchors: anchors)
        #expect(a.nodes.map(\.id) == b.nodes.map(\.id))
        #expect(a.edges.map(\.evidence) == b.edges.map(\.evidence))
    }
}
