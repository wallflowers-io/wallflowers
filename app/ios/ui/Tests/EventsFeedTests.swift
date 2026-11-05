//  EventsFeedTests.swift
//  The merged feed's contracts for the pulled (Music/Art) rows: DETERMINISTIC dedup —
//  within a source by its own id upstream, across sources by (normalized title, start
//  day) with the RA row winning regardless of refresh order, minted beating both —
//  plus the access URL every reference row must carry, and the RSVP reminder's clamp
//  rules. Pure fixtures, no network.

import Testing
import Foundation
@testable import WallFlowers

@MainActor
struct EventsFeedTests {

    private static let day: Int64 = 86_400_000
    private static func participant(_ name: String) -> ParticipantVM {
        ParticipantVM(id: "p:\(name.lowercased())", name: name, category: .places,
                      role: .at, tint: EventsSeed.tint(.places))
    }

    private static func row(_ id: String, _ title: String, startMs: Int64) -> EventVM {
        EventVM(id: id, startMs: startMs, title: title,
                participants: [participant("Somewhere")],
                accessURL: URL(string: "https://example.org/\(id)"))
    }

    private func merged(music: [EventVM] = [], art: [EventVM] = [],
                        groupObjects: [EventVM] = []) -> MergedEventsSource {
        MergedEventsSource(graph: GraphEventsSource(snapshot: EventsSnapshot()),
                           market: [], groupObjects: groupObjects,
                           music: music, art: art)
    }

    // MARK: reference dedup — per-source by stable id; cross-source deliberately open

    @Test func crossSourceCollision_showsBothRows_byRuling() {
        // Pinned to a UTC NOON so the +1h variant stays inside the same UTC day
        // bucket (the v1 dedup key) whatever zone the test machine runs in.
        let start = 20_000 * Self.day + Self.day / 2
        // The SAME show listed by both sources: an edge case we do NOT handle (ruling
        // of 2 Aug) — the sources share no id namespace, so both rows show. This test
        // pins the decision; handling it later means rewriting this expectation.
        let ra = Self.row("ra:1", "Opening Night", startMs: start)
        let ar = Self.row("art:9", "OPENING NIGHT", startMs: start + 3_600_000)

        let out = merged(music: [ra], art: [ar]).events(matching: [], window: .all)
        #expect(out.map(\.id).sorted() == ["art:9", "ra:1"])
    }

    @Test func mintedStillBeatsBothReferenceSources() {
        let start = 20_000 * Self.day + Self.day / 2
        let mine = Self.row("obj:abc", "Opening Night", startMs: start)
        let ra = Self.row("ra:1", "Opening Night", startMs: start)
        let ar = Self.row("art:9", "Opening Night", startMs: start)

        let out = merged(music: [ra], art: [ar], groupObjects: [mine])
            .events(matching: [], window: .all)
        #expect(out.map(\.id) == ["obj:abc"], "your object beats the world's listings of it")
    }

    // MARK: access URLs

    @Test func pulledRowsCarryTheirSourcePage() throws {
        let music = MusicEvent(id: "77", title: "Night", startMs: 1, endMs: nil,
                               venue: MusicVenue(id: "v", name: "F8", areaName: "SF"),
                               areaName: "SF", artists: [], flyerURL: nil,
                               contentPath: "/events/77", isTicketed: true, attending: 0)
        let ra = try #require(MusicEventsSource.events(from: [music]).first)
        #expect(ra.accessURL?.absoluteString == "https://ra.co/events/77")

        let art = ArtEvent(id: "88", title: "Show", startMs: 1, endMs: nil, category: nil,
                           venueName: "Tillage", cityName: nil, venueLat: nil, venueLon: nil,
                           imageURL: nil, contentPath: "/events/ten-x")
        let ar = try #require(ArtEventsSource.events(from: [art]).first)
        #expect(ar.accessURL?.absoluteString == "https://www.artrabbit.com/events/ten-x")
    }

    // MARK: the RSVP reminder's clamps

    @Test func reminderFires2hBefore_neverInThePast_neverAfterDoors() {
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        let ms: (TimeInterval) -> Int64 = { Int64((now.timeIntervalSince1970 + $0) * 1000) }

        // Already begun (an open exhibition run): nothing to be late for.
        #expect(RsvpReminder.fireDate(startMs: ms(-3600), now: now) == nil)
        // About to begin: a reminder would land after the doors.
        #expect(RsvpReminder.fireDate(startMs: ms(60), now: now) == nil)
        // Tonight, one hour out: fires promptly, never in the past.
        #expect(RsvpReminder.fireDate(startMs: ms(3600), now: now)
                == now.addingTimeInterval(60))
        // Days out: two hours before the start.
        #expect(RsvpReminder.fireDate(startMs: ms(3 * 86_400), now: now)
                == now.addingTimeInterval(3 * 86_400 - 2 * 3600))
    }
}
