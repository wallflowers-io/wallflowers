//  ArtEventsSource.swift
//  The Art System's listings, read as EVENTS — the second reference provenance in the
//  feed, beside the Music System's RA rows.
//
//  These are what the WORLD has showing around you: ArtRabbit listings, id-prefixed
//  "art:", carrying the listing image for the NEAR card and opening the REFERENCE
//  detail page (no Pacific ticketing — they are not GroupObjects). The venue joins as
//  the `.at` PLACE, so a locked jewel filters art rows through the same 1-hop
//  intersection as everything else. Most rows are exhibition RUNS — multi-day spans —
//  so `endMs` carries the close and the feed's span-aware window keeps an open show
//  visible for its whole run, not just its opening day.

import SwiftUI

enum ArtEventsSource {

    /// ArtRabbit listings as feed rows. The venue is the one participant a card carries
    /// (there is no lineup); a listing with no venue is dropped — unreachable by the
    /// intersection filter, like the graph source's rejects.
    static func events(from listings: [ArtEvent]) -> [EventVM] {
        listings.compactMap { e in
            guard let v = e.venueName else { return nil }
            return EventVM(id: "art:\(e.id)",
                           startMs: e.startMs,
                           endMs: e.endMs,
                           title: e.title,
                           participants: [
                               ParticipantVM(id: "art-venue:\(v.lowercased())", name: v,
                                             category: .places, role: .at,
                                             tint: EventsSeed.tint(.places)),
                           ],
                           flyerURL: e.imageURL,
                           accessURL: e.contentUrl)
        }
    }
}
