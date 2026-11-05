//  SpaceCalibrationTests.swift
//  The anisotropy corrections proven on cone geometry — the shape real
//  sentence-embedding spaces actually have.
//
//  The synthetic cone: every vector = a strong COMMON direction + a small
//  structured residual, unit-normalized — so all raw pairwise cosines sit
//  ≈ 0.9, exactly the regime BGE-class models produce. The tests demonstrate
//  the FAILURE first (uncentered coherence cannot tell a beam from buffeting;
//  uncentered headings of different streams are indistinguishable) and then
//  the recovery (centering restores structure; the null calibration makes 0.5
//  mean "unrelated"; local-scaled spectral clustering reads k off the cone).

import Foundation
import Testing
@testable import PacificStore

private let day: UInt64 = 24 * 3_600_000

/// dim-8 cone: common axis e0, residual amplitude 0.3 along a group direction,
/// deterministic jitter. All raw cosines land ≈ 0.9+.
private func cone(group: Int, n: Int, seed: UInt64) -> [[Float]] {
    var rng = SplitMix64(seed: seed)
    return (0..<n).map { _ in
        var v = [Float](repeating: 0, count: 8)
        v[0] = 1.0                                   // the common direction
        v[1 + group] = 0.3                           // the structure
        for d in 0..<8 {                             // jitter, deterministic
            v[d] += (Float(rng.next() % 1000) / 1000 - 0.5) * 0.04
        }
        return SpectralClustering.normalize(v)
    }
}

private func coneEpisode(_ id: String, _ v: [Float], _ ts: UInt64) -> TransmissionEpisode {
    TransmissionEpisode(id: id, vector: v, text: "walked the pier", ts: ts)
}

struct SpaceCalibrationTests {

    private func corpus() -> [[Float]] {
        cone(group: 0, n: 10, seed: 1) + cone(group: 1, n: 10, seed: 2) + cone(group: 2, n: 10, seed: 3)
    }

    @Test func theConeIsRealBeforeCentering() {
        let c = corpus()
        // Cross-group texts are UNRELATED, yet raw cosine says ≈0.9 — the
        // anisotropy failure this file exists to correct.
        let cross = Float(SpectralClustering.dot(c[0], c[15]))
        #expect(cross > 0.85)
    }

    @Test func centeringRecoversStructure() {
        let c = corpus()
        let cal = SpaceCalibration.fit(c)!
        let within = Float(SpectralClustering.dot(cal.center(c[0]), cal.center(c[1])))
        let cross = Float(SpectralClustering.dot(cal.center(c[0]), cal.center(c[15])))
        #expect(within > 0.6)
        #expect(cross < 0.35)
    }

    @Test func nullCalibrationMakesHalfMeanUnrelated() {
        let c = corpus()
        let cal = SpaceCalibration.fit(c)!
        let within = cal.calibrated01(Float(SpectralClustering.dot(cal.center(c[0]), cal.center(c[1]))))
        let cross = cal.calibrated01(Float(SpectralClustering.dot(cal.center(c[0]), cal.center(c[15]))))
        #expect(within > 0.8)
        #expect(cross < 0.55)
    }

    @Test func deflationRemovesADominantAxisAndItsSignalWithIt() {
        // Two groups along ±e1: after mean-centering, PC1 IS the group axis —
        // deflating it erases the distinction. The dial is powerful and
        // dangerous; this pins WHY components defaults to 0.
        let c = cone(group: 0, n: 10, seed: 4) + cone(group: 0, n: 10, seed: 5).map { v in
            var w = v; w[1] = -w[1]; return SpectralClustering.normalize(w)
        }
        let kept = SpaceCalibration.fit(c, components: 0)!
        let cut = SpaceCalibration.fit(c, components: 1)!
        let keptCross = Float(SpectralClustering.dot(kept.center(c[0]), kept.center(c[15])))
        let cutCross = Float(SpectralClustering.dot(cut.center(c[0]), cut.center(c[15])))
        #expect(keptCross < -0.5)          // opposed groups, visible
        #expect(abs(cutCross) < abs(keptCross))   // deflation blurred them together
    }

    @Test func deterministicFit() {
        let c = corpus()
        let a = SpaceCalibration.fit(c, components: 1)!
        let b = SpaceCalibration.fit(c, components: 1)!
        #expect(a.mean == b.mean)
        #expect(a.removed == b.removed)
        #expect(a.nullMean == b.nullMean)
    }

    @Test func tinyCorpusRefusesToCalibrate() {
        #expect(SpaceCalibration.fit(cone(group: 0, n: 5, seed: 9)) == nil)
    }
}

struct GroundedAttitudeTests {

    @Test func uncenteredHeadingsCollapseCenteredOnesDistinguish() {
        let corpus = cone(group: 0, n: 10, seed: 11) + cone(group: 1, n: 10, seed: 12)
        let cal = SpaceCalibration.fit(corpus)!
        let a = (0..<10).map { coneEpisode("a\($0)", corpus[$0], 100 * day + UInt64($0) * day / 4) }
        let b = (0..<10).map { coneEpisode("b\($0)", corpus[10 + $0], 100 * day + UInt64($0) * day / 4) }

        // The failure: two streams about DIFFERENT things, raw headings ≈ same.
        let rawA = Attitude.orientation(episodes: a)!
        let rawB = Attitude.orientation(episodes: b)!
        #expect(Float(SpectralClustering.dot(rawA.heading, rawB.heading)) > 0.85)

        // The recovery: centered headings actually point apart.
        let calA = Attitude.orientation(episodes: a, calibration: cal)!
        let calB = Attitude.orientation(episodes: b, calibration: cal)!
        #expect(Float(SpectralClustering.dot(calA.heading, calB.heading)) < 0.5)
    }

    @Test func uncenteredCoherenceSaturatesCenteredConfesses() {
        let corpus = cone(group: 0, n: 12, seed: 21) + cone(group: 1, n: 6, seed: 22)
            + cone(group: 2, n: 6, seed: 23)
        let cal = SpaceCalibration.fit(corpus)!
        let beam = (0..<12).map { coneEpisode("s\($0)", corpus[$0], 100 * day + UInt64($0) * day / 4) }
        let buffeted = (0..<12).map { i -> TransmissionEpisode in
            let base = i % 2 == 0 ? 12 : 18
            let idx = base + i / 2
            let ts = 100 * day + UInt64(i) * day / 4
            return coneEpisode("m\(i)", corpus[idx], ts)
        }

        // The failure: raw resultants can't tell them apart.
        let rawBeam = Attitude.orientation(episodes: beam)!
        let rawBuffeted = Attitude.orientation(episodes: buffeted)!
        #expect(abs(rawBeam.coherence - rawBuffeted.coherence) < 0.06)

        // The recovery: the beam holds, the buffeting confesses.
        let calBeam = Attitude.orientation(episodes: beam, calibration: cal)!
        let calBuffeted = Attitude.orientation(episodes: buffeted, calibration: cal)!
        #expect(calBeam.coherence > 0.75)
        #expect(calBuffeted.coherence < calBeam.coherence - 0.3)
    }

    @Test func calibratedSpectralReadsKOffTheCone() {
        let corpus = cone(group: 0, n: 12, seed: 31) + cone(group: 1, n: 12, seed: 32)
            + cone(group: 2, n: 12, seed: 33)
        let cal = SpaceCalibration.fit(corpus)!
        let ids = (0..<36).map { "e\($0)" }
        let result = SpectralClustering.cluster(ids: ids, vectors: corpus, calibration: cal)
        #expect(result.k == 3)
        for g in 0..<3 {
            #expect(Set(result.assignments[(g * 12)..<((g + 1) * 12)]).count == 1)
        }
    }
}
