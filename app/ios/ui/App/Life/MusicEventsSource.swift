//  MusicEventsSource.swift
//  The Music System's listings, read as EVENTS — the reference provenance in the feed.
//
//  The graph records what the resolver OBSERVED, market rows what you ASSERTED, minted
//  rows what you MINTED, calendar rows what you're BOOKED for. These are what the WORLD
//  has listed around you: Resident Advisor rows, id-prefixed "ra:", carrying the flyer
//  URL for the NEAR card and opening the REFERENCE detail page (no Pacific ticketing —
//  they are not GroupObjects). Artists join as PEOPLE, the venue as the `.at` PLACE, so
//  a locked jewel filters RA rows through the same 1-hop intersection as everything else.

import SwiftUI

enum MusicEventsSource {

    /// RA listings as feed rows. Participants come from the lineup (first artist is the
    /// subject) and the venue (`.at`) — never empty: a listing with no artists is carried
    /// by its venue, and the rare row with neither is dropped (unreachable by the
    /// intersection filter, like the graph source's rejects).
    static func events(from listings: [MusicEvent]) -> [EventVM] {
        listings.compactMap { e in
            var participants: [ParticipantVM] = []
            for (i, a) in e.artists.enumerated() {
                participants.append(ParticipantVM(id: "ra-artist:\(a.id)", name: a.name,
                                                  category: .people,
                                                  role: i == 0 ? .subject : .with,
                                                  tint: EventsSeed.tint(.people)))
            }
            if let v = e.venue {
                participants.append(ParticipantVM(id: "ra-venue:\(v.id)", name: v.name,
                                                  category: .places, role: .at,
                                                  tint: EventsSeed.tint(.places)))
            }
            guard !participants.isEmpty else { return nil }
            return EventVM(id: "ra:\(e.id)",
                           startMs: e.startMs,
                           endMs: e.endMs,
                           title: e.title,
                           participants: participants,
                           flyerURL: e.flyerURL,
                           accessURL: e.contentUrl)
        }
    }
}
