//  EventsChannel.swift
//  What every event surface shares: the blue channel's identity (its accent IS the LIFE
//  disc's block colour, via DiscSlot, the interface matrix's home of channel colour) and
//  venue resolution against the placed PLACES.

import SwiftUI

enum EventsChannel {
    /// The blue channel's identity — the LIFE disc's own colour, via the matrix.
    static let accent = DiscSlot.life.blockColor
    /// The soft blue the design uses for kickers, date headers and distances.
    static let accentSoft = DiscSlot.life.blockColor.lightened(0.35)

    // MARK: Venue resolution — ONE place, shared with the fullscreen map's overlay

    /// The `.at` hop names the venue; a placed PLACE of the same identity (or, for
    /// resolver-lifted names, the same name) pins it.
    static func venue(named at: ParticipantVM, in places: [PlaceAnnotation]) -> PlaceAnnotation? {
        places.first { $0.id == at.id }
            ?? places.first { $0.name.caseInsensitiveCompare(at.name) == .orderedSame }
    }

    /// The located commitments as map pins — the fullscreen Map's EVENTS overlay, shown
    /// there while the EVENTS dimension is locked (see `PlacesMap.events`). One pin per
    /// commitment; an unplaced venue contributes none rather than a guess.
    static func venuePins(for events: [EventVM], places: [PlaceAnnotation]) -> [PlaceAnnotation] {
        events.compactMap { e in
            guard let at = e.participants.first(where: { $0.role == .at }),
                  let v = venue(named: at, in: places) else { return nil }
            return PlaceAnnotation(id: "event-\(e.id)", name: e.title,
                                   coordinate: v.coordinate,
                                   category: v.name, glyph: "calendar")
        }
    }
}
