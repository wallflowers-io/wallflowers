//  CalendarEventsSource.swift
//  Google Calendar as a THIRD provenance for the EVENTS ribbon — alongside the graph's
//  commitments and your own minted market postures (see MergedEventsSource).
//
//  Calendar events arrive already STRUCTURED (title · time · attendees · location), so they map
//  straight to `EventVM`: attendees become PEOPLE participants (role `.with`), the location a PLACE
//  (`.at`), the organizer the SUBJECT — so a locked PEOPLE or PLACES jewel filters real
//  calendar commitments through the exact same 1-hop intersection as everything else.
//
//  This file is the pure MAPPING (Google Calendar v3 `events.list` JSON → EventVM). The live
//  fetch — on-device OAuth, tokens sealed in the credential vault, the connected System — wraps
//  it; nothing here reaches the network, so it is deterministic and unit-testable on its own.

import Foundation
import SwiftUI

// MARK: - The Google Calendar v3 wire shapes (only the fields the ribbon needs)

/// One event from `events.list`. Every field past `id` is optional: a real calendar carries
/// all-day events (no `dateTime`), event with no attendees, and instances of recurring series.
struct GCalEvent: Decodable {
    let id: String
    var summary: String?
    var description: String?
    var location: String?
    var start: GCalTime?
    var end: GCalTime?
    var attendees: [GCalAttendee]?
    var organizer: GCalPerson?
    var recurringEventId: String?
    var status: String?
    var htmlLink: String?
    var conferenceUrl: String?
}

/// A start/end. Exactly one of `dateTime` (an instant) or `date` (an all-day date) is set.
struct GCalTime: Decodable {
    var dateTime: String?     // RFC 3339, e.g. "2026-07-28T17:30:00-07:00"
    var date: String?         // all-day, e.g. "2026-08-31" (may also arrive as a Z timestamp)
    var timeZone: String?
}

struct GCalAttendee: Decodable {
    let email: String?
    var displayName: String?
    var responseStatus: String?
    var organizer: Bool?
    var isSelf: Bool?         // JSON key `self` (a keyword) — you, the viewer
    var resource: Bool?       // a room / equipment, never a person
    enum CodingKeys: String, CodingKey {
        case email, displayName, responseStatus, organizer, resource
        case isSelf = "self"
    }
}

struct GCalPerson: Decodable {
    var email: String?
    var displayName: String?
    var isSelf: Bool?         // JSON key `self`
    enum CodingKeys: String, CodingKey {
        case email, displayName
        case isSelf = "self"
    }
}

/// The `events.list` envelope. `summary` on the primary calendar IS the account email, so we
/// read the connected account from it rather than requesting a separate userinfo scope.
struct GCalEventsResponse: Decodable {
    var items: [GCalEvent]?
    var nextPageToken: String?
    var summary: String?
}

// MARK: - Time parsing

enum GCalClock {
    private static let rfc3339: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()
    private static let rfc3339NoFrac: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return f
    }()
    /// All-day dates: "YYYY-MM-DD", pinned to UTC midnight so a point on the timeline exists.
    private static let dayOnly: DateFormatter = {
        let f = DateFormatter()
        f.calendar = Calendar(identifier: .gregorian)
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = TimeZone(identifier: "UTC")
        f.dateFormat = "yyyy-MM-dd"
        return f
    }()

    /// Epoch milliseconds for a start/end, or nil when neither field parses — an unsequenceable
    /// event is dropped upstream, never rendered.
    static func epochMs(_ t: GCalTime?) -> Int64? {
        guard let t else { return nil }
        if let dt = t.dateTime, let date = rfc3339.date(from: dt) ?? rfc3339NoFrac.date(from: dt) {
            return Int64(date.timeIntervalSince1970 * 1000)
        }
        if let d = t.date {
            if let date = dayOnly.date(from: d) { return Int64(date.timeIntervalSince1970 * 1000) }
            // Some backends hand `date` back as a full Z timestamp — accept that too.
            if let date = rfc3339NoFrac.date(from: d) ?? rfc3339.date(from: d) {
                return Int64(date.timeIntervalSince1970 * 1000)
            }
        }
        return nil
    }
}

// MARK: - Mapping to the ribbon's EventVM

extension GCalEvent {
    /// A stable place id from a free-text location, so the same venue on two events reads as one
    /// PLACE (and can later carry a geocoded coordinate). Address strings vary in whitespace and
    /// case; we normalise to a slug.
    private func placeID(_ location: String) -> String {
        "gcal.place." + location.lowercased()
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: " ", with: "-")
    }

    /// Map to an EventVM, or nil when it cannot be sequenced (no parseable start). Participants:
    /// non-`self`, non-resource attendees as PEOPLE (`.with`); the organizer as the SUBJECT
    /// person; the location as a PLACE (`.at`). Deduped by id so an organizer who is also an
    /// attendee appears once. May be empty — such an event still shows in the unfiltered EVENTS
    /// list but cannot satisfy a locked PEOPLE/PLACES term, which is the honest outcome.
    func eventVM(tint: (EventCategory) -> Color) -> EventVM? {
        guard let startMs = GCalClock.epochMs(start) else { return nil }
        let endMs = GCalClock.epochMs(end)

        var participants: [ParticipantVM] = []
        var seen = Set<String>()
        func add(_ p: ParticipantVM) { if seen.insert(p.id).inserted { participants.append(p) } }

        // Organizer → subject (skip yourself; you are the viewer, not a participant).
        if let org = organizer, org.isSelf != true, let email = org.email, !email.isEmpty {
            add(ParticipantVM(id: "gcal.person." + email,
                              name: org.displayName?.nonEmpty ?? email,
                              category: .people, role: .subject, tint: tint(.people)))
        }
        // Attendees → people (with). Skip self and resources (rooms/equipment).
        for a in attendees ?? [] {
            guard a.isSelf != true, a.resource != true, let email = a.email, !email.isEmpty else { continue }
            add(ParticipantVM(id: "gcal.person." + email,
                              name: a.displayName?.nonEmpty ?? email,
                              category: .people, role: .with, tint: tint(.people)))
        }
        // Location → place (at).
        if let loc = location?.nonEmpty {
            add(ParticipantVM(id: placeID(loc), name: loc,
                              category: .places, role: .at, tint: tint(.places)))
        }

        return EventVM(id: "gcal." + id,
                       startMs: startMs,
                       endMs: endMs,
                       title: summary?.nonEmpty ?? "(untitled)",
                       participants: participants,
                       recurrenceLabel: recurringEventId != nil ? "Repeats" : nil)
    }
}

private extension String {
    /// nil when the string is empty after trimming — so `?? fallback` never yields "".
    var nonEmpty: String? {
        let t = trimmingCharacters(in: .whitespacesAndNewlines)
        return t.isEmpty ? nil : t
    }
}

// MARK: - The source (pure)

/// The connected Google Calendar, as EVENTS. Holds already-mapped rows; the live System sets
/// `events` after each fetch. Merged into the ribbon by `MergedEventsSource`, so calendar
/// commitments pass through the identical intersection filter as observed + market rows. Pure
/// (no actor state), so it maps on whatever context builds the source.
struct CalendarEventsSource {
    var events: [EventVM] = []

    /// Build from raw wire events, dropping any that cannot be sequenced.
    static func from(_ raw: [GCalEvent], tint: @escaping (EventCategory) -> Color) -> CalendarEventsSource {
        CalendarEventsSource(events: raw.compactMap { $0.eventVM(tint: tint) })
    }
}
