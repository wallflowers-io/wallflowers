//  CausalLensTests.swift
//  The causal query operator, proven — precedence is strict, alignment gates,
//  the chromodynamic lean widens but never closes, and nothing here ever
//  asserts a cause: the lens returns ranked evidence, deterministically.

import Foundation
import Testing
@testable import PacificStore

private let day: UInt64 = 24 * 3_600_000

private func ep(_ id: String, _ v: [Float], _ ts: UInt64, _ text: String) -> TransmissionEpisode {
    TransmissionEpisode(id: id, vector: SpectralClustering.normalize(v), text: text, ts: ts)
}

struct CausalLensTests {

    private let x: [Float] = [1, 0, 0, 0]
    private let y: [Float] = [0, 1, 0, 0]

    @Test func precedenceIsStrict() {
        let target = (vector: x, ts: 100 * day)
        let episodes = [
            ep("before", x, 99 * day, "walked the pier"),
            ep("same", x, 100 * day, "walked the pier"),
            ep("after", x, 101 * day, "walked the pier"),
        ]
        let ante = CausalLens.antecedents(of: target, in: episodes)
        let cons = CausalLens.consequents(of: target, in: episodes)
        #expect(ante.map(\.id) == ["before"])
        #expect(cons.map(\.id) == ["after"])   // simultaneity proves nothing, both ways
    }

    @Test func alignmentGatesTheUnrelated() {
        let target = (vector: x, ts: 100 * day)
        let episodes = [
            ep("related", x, 99 * day, "walked the pier"),
            ep("unrelated", y, 99 * day, "walked the pier"),
        ]
        let ante = CausalLens.antecedents(of: target, in: episodes)
        #expect(ante.contains { $0.id == "related" })
        #expect(!ante.contains { $0.id == "unrelated" })
    }

    @Test func towardsLeanRanksTheLeadUp() {
        let target = (vector: x, ts: 100 * day)
        let episodes = [
            ep("plan", x, 99 * day, "let's plan the launch, we will book the venue soon"),
            ep("flat", x, 99 * day, "the pier was quiet"),
        ]
        let ante = CausalLens.antecedents(of: target, in: episodes)
        #expect(ante.first?.id == "plan")
        // The lean widened the gate — it did not close it on the flat episode.
        #expect(ante.count == 2)
    }

    @Test func fromLeanRanksTheWake() {
        let target = (vector: x, ts: 100 * day)
        let episodes = [
            ep("recollection", x, 101 * day, "remember how it began back before the launch"),
            ep("flat", x, 101 * day, "the pier is quiet"),
        ]
        let cons = CausalLens.consequents(of: target, in: episodes)
        #expect(cons.first?.id == "recollection")
    }

    @Test func nearerEvidenceOutranksDistantEqualEvidence() {
        let target = (vector: x, ts: 100 * day)
        let episodes = [
            ep("near", x, 99 * day, "walked the pier"),
            ep("far", x, 40 * day, "walked the pier"),
        ]
        let ante = CausalLens.antecedents(of: target, in: episodes)
        #expect(ante.first?.id == "near")
    }

    @Test func deterministicAndHonestWhenEmpty() {
        let target = (vector: x, ts: 100 * day)
        #expect(CausalLens.antecedents(of: target, in: []).isEmpty)
        let episodes = [
            ep("a", x, 99 * day, "walked the pier"),
            ep("b", x, 99 * day, "walked the pier"),
        ]
        let one = CausalLens.antecedents(of: target, in: episodes)
        let two = CausalLens.antecedents(of: target, in: episodes)
        #expect(one.map(\.id) == two.map(\.id))
        #expect(one.map(\.id) == ["a", "b"])   // equal evidence: id tie-break
    }

    @Test func calibratedConeStillFindsTheTrueAntecedent() {
        // Cone geometry: raw alignment cannot separate related from unrelated;
        // calibrated, the lens gates correctly.
        var rng = SplitMix64(seed: 77)
        func cone(_ g: Int) -> [Float] {
            var v = [Float](repeating: 0, count: 8)
            v[0] = 1; v[1 + g] = 0.3
            for d in 0..<8 { v[d] += (Float(rng.next() % 1000) / 1000 - 0.5) * 0.04 }
            return SpectralClustering.normalize(v)
        }
        let corpus = (0..<10).map { _ in cone(0) } + (0..<10).map { _ in cone(1) }
        let cal = SpaceCalibration.fit(corpus)!
        let target = (vector: corpus[0], ts: 100 * day)
        let episodes = [
            ep("kin", corpus[1], 99 * day, "walked the pier"),
            ep("stranger", corpus[15], 99 * day, "walked the pier"),
        ]
        let ante = CausalLens.antecedents(of: target, in: episodes, calibration: cal)
        #expect(ante.contains { $0.id == "kin" })
        #expect(!ante.contains { $0.id == "stranger" })
    }
}
