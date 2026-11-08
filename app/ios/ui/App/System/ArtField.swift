//  ArtField.swift
//  The value types of the Art System — ArtRabbit's public listings as a Pacific System.
//  An ArtEvent is one listed show or event, already flattened for the app: epoch-ms
//  opening/closing, the image URL, the venue. ArtRabbit is a PUBLIC source (no
//  credential), so like MusicField the scope is a PLACE — but ArtRabbit has no area
//  ids at all: its site scopes by coordinates (it geocodes the typed city with Google
//  Places client-side), so our scope carries the coordinates themselves, resolved
//  on-device by CLGeocoder (see ArtService.searchPlaces).
//
//  The wire shape (server-rendered listing cards) lives in ArtClient; the mapping into
//  these types is here, deterministic and testable on its own — the MusicField split,
//  mirrored.

import Foundation

/// The Art System's scope — a place with the coordinates ArtRabbit filters by.
/// Resolved from a typed name via CLGeocoder, never hand-entered numbers.
///
/// A scope the user PICKED is `pinned` and never moves; an auto-resolved one is a
/// stand-in for "near me" and re-resolves when the device leaves it (see NearScope).
/// A legacy record (no `pinned`) counts as unpinned: it was only ever the default.
struct ArtPlace: Codable, Hashable, Identifiable {
    let name: String              // "London, United Kingdom" — the picker row and header
    let lat: Double
    let lon: Double
    let city: String              // what ArtRabbit's own form calls `city`
    let country: String
    var pinned: Bool? = nil

    var id: String { name }
    var isPinned: Bool { pinned ?? false }
}

/// One ArtRabbit listing, flattened. REFERENCE data: it backs list rows, enrichment and
/// the reference detail page — it is not a GroupObject and never grows Pacific ticketing.
/// Most listings are SHOWS — multi-day spans (an exhibition's run), not point events —
/// so `endMs` is the run's close, and a row renders as a date range, never a wall clock.
struct ArtEvent: Codable, Hashable, Identifiable {
    let id: String                // ArtRabbit's listing ident
    let title: String
    let startMs: Int64            // the run's opening day (device zone, start of day)
    var endMs: Int64?             // the run's closing day, when the card carries a range
    var category: String?         // "Exhibition" / "Talk" / … — the card's own label
    var venueName: String?
    var cityName: String?         // "San Francisco, United States" — the card's locality line
    var venueLat: Double?         // the card's own coordinates (unused yet; the MAP seam)
    var venueLon: Double?
    var imageURL: URL?
    var contentPath: String       // "/events/<slug>"

    /// The ArtRabbit page for this listing — the reference the detail page links out to.
    var contentUrl: URL? { URL(string: "https://www.artrabbit.com" + contentPath) }
}

// MARK: - Date parsing

enum ArtClock {
    /// ArtRabbit dates are DAYS, not instants ("02 Jul 2026", or a range's half). We parse
    /// in the device's zone — the MusicClock policy, stated rather than hidden — and a day
    /// lands at its local midnight; rendering treats these rows as date ranges throughout,
    /// so no wall clock is ever shown for them.
    private static let day: DateFormatter = {
        let f = DateFormatter()
        f.calendar = Calendar(identifier: .gregorian)
        f.locale = Locale(identifier: "en_US_POSIX")   // the site's month names are English
        f.timeZone = .current
        f.dateFormat = "dd MMM yyyy"
        return f
    }()

    static func epochMs(_ s: String?) -> Int64? {
        guard let s = s?.trimmingCharacters(in: .whitespaces), !s.isEmpty else { return nil }
        guard let date = day.date(from: s) else { return nil }
        return Int64(date.timeIntervalSince1970 * 1000)
    }

    /// "02 Jul 2026 – 05 Sep 2026" | "02 Aug 2026" → the run's (open, close?). A range
    /// keeps document order; a single day is a one-day listing with no close.
    static func range(_ s: String) -> (startMs: Int64, endMs: Int64?)? {
        let halves = s.split(separator: "–").map { $0.trimmingCharacters(in: .whitespaces) }
        guard let first = halves.first, let start = epochMs(first) else { return nil }
        let end = halves.count > 1 ? epochMs(halves[1]) : nil
        return (start, end)
    }
}

// MARK: - Wire → value mapping

extension ArtEvent {
    /// Map one parsed card, or nil when it cannot be sequenced (no parseable date) — an
    /// unsequenceable listing is dropped upstream, never rendered at a fake time.
    init?(card c: ArtClient.CardWire) {
        guard let dates = c.dateText.flatMap(ArtClock.range) else { return nil }
        self.init(
            id: c.ident,
            title: c.title,
            startMs: dates.startMs,
            endMs: dates.endMs,
            category: c.category,
            venueName: c.venueName,
            cityName: c.cityName,
            venueLat: c.lat,
            venueLon: c.lon,
            imageURL: c.imagePath.flatMap { URL(string: $0.hasPrefix("//") ? "https:" + $0 : $0) },
            contentPath: c.path
        )
    }
}
