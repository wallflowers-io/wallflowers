//  AttitudeTests.swift
//  The orientation process proven on synthetic streams — pure logic, no store.
//
//  What must hold: the heading integrates the stream's dominant direction; decay
//  makes the recent outweigh the old while the slow window still remembers; a
//  turning stream registers rate and a steady one doesn't; coherence confesses
//  buffeting; trim carries the chromodynamic lean; guide-star locks rank the
//  fixed point the heading actually points at; the stream's own clock (not wall
//  time) drives everything; and the whole process is deterministic.

import Foundation
import Testing
@testable import PacificStore

private let hour: UInt64 = 3_600_000
private let day: UInt64 = 24 * hour

private func ep(_ id: String, _ v: [Float], _ ts: UInt64, text: String = "walked the pier") -> TransmissionEpisode {
    TransmissionEpisode(id: id, vector: v, text: text, ts: ts)
}

struct AttitudeTests {

    private let x: [Float] = [1, 0, 0, 0]
    private let y: [Float] = [0, 1, 0, 0]

    @Test func headingIntegratesTheDominantDirection() {
        let episodes = (0..<6).map { ep("e\($0)", x, 100 * day + UInt64($0) * hour) }
        let o = Attitude.orientation(episodes: episodes)!
        #expect(o.heading[0] > 0.99)
        #expect(o.coherence > 0.99)
        #expect(o.evidence == 6)
        #expect(o.clock == 100 * day + 5 * hour)
    }

    @Test func recentOutweighsOldButTheSlowWindowRemembers() {
        // Ten days pointed at x, then the last two days pointed at y.
        var episodes = (0..<10).map { ep("old\($0)", x, 90 * day + UInt64($0) * day) }
        episodes += (0..<8).map { ep("new\($0)", y, 100 * day + UInt64($0) * 6 * hour) }
        let o = Attitude.orientation(episodes: episodes)!
        // The blended heading leans toward y (the manoeuvre) but keeps x in it
        // (the disposition) — the complementary filter's whole point.
        #expect(o.heading[1] > o.heading[0])
        #expect(o.heading[0] > 0.05)
    }

    @Test func turningStreamRegistersRateSteadyStreamDoesNot() {
        let steady = (0..<20).map { ep("s\($0)", x, 100 * day + UInt64($0) * 8 * hour) }
        let so = Attitude.orientation(episodes: steady)!
        #expect(so.rateRadians < 0.05)

        // Previous fast-span pointed at x; current fast-span points at y.
        var turning = (0..<8).map { ep("a\($0)", x, 90 * day + UInt64($0) * 12 * hour) }
        turning += (0..<8).map { ep("b\($0)", y, 98 * day + UInt64($0) * 12 * hour) }
        let to = Attitude.orientation(episodes: turning)!
        #expect(to.rateRadians > 1.0)   // ~π/2 turn
    }

    @Test func coherenceConfessesBuffeting() {
        // A scattered stream: orthogonal directions in equal measure.
        let dirs: [[Float]] = [[1, 0, 0, 0], [-1, 0, 0, 0], [0, 1, 0, 0], [0, -1, 0, 0]]
        let episodes = (0..<12).map { ep("e\($0)", dirs[$0 % 4], 100 * day + UInt64($0) * hour) }
        let o = Attitude.orientation(episodes: episodes)!
        #expect(o.coherence < 0.3)
    }

    @Test func trimCarriesTheChromodynamicLean() {
        let episodes = (0..<5).map {
            ep("e\($0)", x, 100 * day + UInt64($0) * hour,
               text: "let's plan the next launch, we will meet soon")
        }
        let o = Attitude.orientation(episodes: episodes)!
        #expect(o.trim.blue > o.trim.red)
    }

    @Test func guideStarLocksRankTheFixedPointTheHeadingPointsAt() {
        let episodes = (0..<6).map { ep("e\($0)", x, 100 * day + UInt64($0) * hour) }
        let stars = [GuideStar(id: "away", vector: y), GuideStar(id: "here", vector: x)]
        let o = Attitude.orientation(episodes: episodes, guideStars: stars)!
        #expect(o.locks.first?.id == "here")
        #expect(o.locks.first!.alignment > 0.99)
    }

    @Test func kindAlignmentReadsOffTheHeading() {
        let episodes = (0..<4).map { ep("e\($0)", x, 100 * day + UInt64($0) * hour) }
        let anchors = [KindAnchor(kind: OntologyKind(name: "Place", definition: "a where"), vector: x),
                       KindAnchor(kind: OntologyKind(name: "Person", definition: "a who"), vector: y)]
        let o = Attitude.orientation(episodes: episodes, anchors: anchors)!
        #expect(o.kindAlignment["Place"]! > o.kindAlignment["Person"]!)
    }

    @Test func streamClockNotWallClock() {
        // An old stream must orient exactly as it did when it was current: the
        // windows cut at the newest EPISODE, so wall time cannot appear anywhere.
        let old = (0..<6).map { ep("e\($0)", x, 10 * day + UInt64($0) * hour) }
        let o = Attitude.orientation(episodes: old)!
        #expect(o.clock == 10 * day + 5 * hour)
        #expect(o.evidence == 6)
        #expect(o.coherence > 0.99)
    }

    @Test func deterministicRunToRun() {
        var episodes = (0..<9).map { ep("e\($0)", [0.6, 0.3, 0.1, 0], 100 * day + UInt64($0) * 7 * hour) }
        episodes += (0..<4).map { ep("f\($0)", y, 101 * day + UInt64($0) * hour) }
        let one = Attitude.orientation(episodes: episodes)!
        let two = Attitude.orientation(episodes: episodes)!
        #expect(one.heading == two.heading)
        #expect(one.rateRadians == two.rateRadians)
        #expect(one.locks == two.locks)
    }

    @Test func emptyStreamHasNoAttitude() {
        #expect(Attitude.orientation(episodes: []) == nil)
    }
}
