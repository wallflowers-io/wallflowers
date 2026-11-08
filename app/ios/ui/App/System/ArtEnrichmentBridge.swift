//  ArtEnrichmentBridge.swift
//  Bridges the Art System's listings (ArtEvent) into the ONE enrichment catalogue, so a
//  mention of a listed show or its venue lights up wherever EnrichedText / EnrichedLabel
//  render and long-press opens the info panel — through the mature enrichment path, not
//  a second renderer. The MusicEnrichmentBridge shape, for the public feeder of
//  EntityOrigin.art.
//
//  REFERENCE data wears REFERENCE priority: .art sits BELOW every local origin in the
//  ladder (see EntityOrigin.priority), and it reuses the existing kinds — an ArtRabbit
//  show is an .event, a venue a .place. No new kinds, no new tints.

import Foundation

extension EnrichedEntity {
    /// Project one listing into a catalogue entity. The title is the surface form; the
    /// panel gets the listing's values (when / venue / type / ArtRabbit reference).
    init(art e: ArtEvent) {
        var fields: [EntityField] = []
        fields.append(EntityField(label: "When", value: ArtEnrichmentBridge.when(e)))
        if let v = e.venueName { fields.append(EntityField(label: "Venue", value: v)) }
        if let c = e.cityName { fields.append(EntityField(label: "City", value: c)) }
        if let k = e.category { fields.append(EntityField(label: "Type", value: k)) }
        fields.append(EntityField(label: "ArtRabbit", value: "artrabbit.com" + e.contentPath))

        self.init(
            id: "art:\(e.id)",
            name: e.title,
            kind: .event,
            origin: .art,
            subtitle: [e.venueName, ArtEnrichmentBridge.run(e)].compactMap { $0 }.joined(separator: " · "),
            fields: fields
        )
    }
}

enum ArtEnrichmentBridge {

    /// The whole feed as catalogue entities: every listing (.event) plus its venues
    /// (.place), deduped across listings by venue name (ArtRabbit cards carry no venue id).
    static func entities(from events: [ArtEvent]) -> [EnrichedEntity] {
        var out = events.map(EnrichedEntity.init(art:))

        var seenVenues = Set<String>()
        for e in events {
            if let v = e.venueName, seenVenues.insert(v.lowercased()).inserted {
                out.append(EnrichedEntity(id: "art:venue:\(v.lowercased())", name: v,
                                          kind: .place, origin: .art, subtitle: e.cityName))
            }
        }
        return out
    }

    // MARK: labels — a listing is a RUN of days, so these read as date ranges, no clocks

    private static let dayFormat: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "EEE d MMM"
        return f
    }()

    private static func day(_ ms: Int64) -> String {
        dayFormat.string(from: Date(timeIntervalSince1970: Double(ms) / 1000))
    }

    /// "Wed 2 Jul – Sat 5 Sep" | "Sun 2 Aug" — the run, terse.
    static func run(_ e: ArtEvent) -> String {
        guard let end = e.endMs, end != e.startMs else { return day(e.startMs) }
        return day(e.startMs) + " – " + day(end)
    }

    /// The info panel's fuller value — same range, the year on the closing half.
    static func when(_ e: ArtEvent) -> String {
        let yearFormat = DateFormatter()
        yearFormat.dateFormat = "EEE d MMM yyyy"
        guard let end = e.endMs, end != e.startMs else {
            return yearFormat.string(from: Date(timeIntervalSince1970: Double(e.startMs) / 1000))
        }
        return day(e.startMs) + " – "
            + yearFormat.string(from: Date(timeIntervalSince1970: Double(end) / 1000))
    }
}
