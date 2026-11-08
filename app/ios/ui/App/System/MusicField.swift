//  MusicField.swift
//  The value types of the Music System — Resident Advisor's public event listings as
//  a Pacific System. A MusicEvent is one listed event, already flattened for the app:
//  epoch-ms times, the flyer URL, the lineup. RA is a PUBLIC source (no credential),
//  so unlike SysiphusField there is no per-employee scoping — the scope is an AREA.
//
//  The wire shapes (nested GraphQL envelopes, local-wall-clock time strings) live in
//  MusicClient; the mapping into these types is here, deterministic and testable on
//  its own — the CalendarEventsSource split, mirrored.

import Foundation

/// An RA area — the System's scope ("San Francisco/Oakland"). The id is RA's integer
/// area id, resolved from source via `areas(searchTerm:)`, never guessed.
///
/// A scope the user PICKED is `pinned` and never moves; an auto-resolved one is a
/// stand-in for "near me" and re-resolves when the device leaves it (see NearScope).
/// `lat`/`lon` are the fix it was resolved AT — absent on pinned and legacy records,
/// and a legacy record (no `pinned`) counts as unpinned: it was only ever the default.
struct MusicArea: Codable, Hashable, Identifiable {
    let id: Int
    let name: String
    var pinned: Bool? = nil
    var lat: Double? = nil
    var lon: Double? = nil

    var isPinned: Bool { pinned ?? false }
}

struct MusicArtist: Codable, Hashable, Identifiable {
    let id: String
    let name: String
}

struct MusicVenue: Codable, Hashable, Identifiable {
    let id: String
    let name: String
    var areaName: String?
}

/// One RA event listing, flattened. REFERENCE data: it backs list rows, enrichment and
/// the reference detail page — it is not a GroupObject and never grows Pacific ticketing.
struct MusicEvent: Codable, Hashable, Identifiable {
    let id: String                // RA event id
    let title: String
    let startMs: Int64            // from startTime (falling back to date)
    var endMs: Int64?
    var venue: MusicVenue?
    var areaName: String?
    var artists: [MusicArtist]
    var flyerURL: URL?            // flyerFront, else the FLYERFRONT image
    var contentPath: String       // "/events/<id>"
    var isTicketed: Bool
    var attending: Int

    var venueName: String? { venue?.name }
    /// The RA page for this event — where its ticketing actually lives.
    var contentUrl: URL? { URL(string: "https://ra.co" + contentPath) }
    var lineup: String { artists.map(\.name).joined(separator: " · ") }
}

// MARK: - Time parsing

enum MusicClock {
    /// RA times are the venue's LOCAL wall clock with no zone offset ("2026-08-02T13:00:00.000").
    /// We parse in the device's zone — correct for the intended use (browsing your own area);
    /// a remote scope reads in your clock, stated rather than hidden.
    private static let wall: DateFormatter = make("yyyy-MM-dd'T'HH:mm:ss.SSS")
    private static let wallNoFrac: DateFormatter = make("yyyy-MM-dd'T'HH:mm:ss")

    private static func make(_ format: String) -> DateFormatter {
        let f = DateFormatter()
        f.calendar = Calendar(identifier: .gregorian)
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = .current
        f.dateFormat = format
        return f
    }

    static func epochMs(_ s: String?) -> Int64? {
        guard let s, !s.isEmpty else { return nil }
        guard let date = wall.date(from: s) ?? wallNoFrac.date(from: s) else { return nil }
        return Int64(date.timeIntervalSince1970 * 1000)
    }
}

// MARK: - Wire → value mapping

extension MusicEvent {
    /// Map one wire event, or nil when it cannot be sequenced (no parseable time) — an
    /// unsequenceable event is dropped upstream, never rendered at a fake time.
    init?(wire e: MusicClient.EventWire) {
        guard let startMs = MusicClock.epochMs(e.startTime) ?? MusicClock.epochMs(e.date) else { return nil }
        // The listings feed leaves `flyerFront` null and carries the flyer in `images`
        // (type FLYERFRONT) — verified live 2 Aug. Prefer the named field if it ever
        // arrives, then the typed image, then any image.
        let flyer = e.flyerFront
            ?? e.images?.first(where: { $0.type == "FLYERFRONT" })?.filename
            ?? e.images?.first?.filename
        self.init(
            id: e.id,
            title: e.title,
            startMs: startMs,
            endMs: MusicClock.epochMs(e.endTime),
            venue: e.venue.map { MusicVenue(id: $0.id, name: $0.name, areaName: $0.area?.name) },
            areaName: e.venue?.area?.name,
            artists: (e.artists ?? []).map { MusicArtist(id: $0.id, name: $0.name) },
            flyerURL: flyer.flatMap(URL.init(string:)),
            contentPath: e.contentUrl ?? "/events/\(e.id)",
            isTicketed: e.isTicketed ?? false,
            attending: e.attending ?? 0
        )
    }
}
