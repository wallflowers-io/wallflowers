//  MusicEnrichmentBridge.swift
//  Bridges the Music System's listings (MusicEvent) into the ONE enrichment catalogue,
//  so a mention of a listed event, its artists or its venue lights up wherever
//  EnrichedText / EnrichedLabel render and long-press opens the info panel — through
//  the mature enrichment path, not a second renderer. The SysiphusEnrichmentBridge
//  shape, for the public feeder of EntityOrigin.music.
//
//  REFERENCE data wears REFERENCE priority: .music sits BELOW every local origin in
//  the ladder (see EntityOrigin.priority), and it reuses the existing kinds — an RA
//  event is an .event, an artist a .person, a venue a .place. No new kinds, no new tints.

import Foundation

extension EnrichedEntity {
    /// Project one listing into a catalogue entity. The title is the surface form; the
    /// panel gets the listing's values (when / venue / lineup / RA reference).
    init(music e: MusicEvent) {
        var fields: [EntityField] = []
        fields.append(EntityField(label: "When", value: MusicEnrichmentBridge.when(e)))
        if let v = e.venueName { fields.append(EntityField(label: "Venue", value: v)) }
        if !e.artists.isEmpty { fields.append(EntityField(label: "Lineup", value: e.lineup)) }
        if let a = e.areaName { fields.append(EntityField(label: "Area", value: a)) }
        if e.isTicketed { fields.append(EntityField(label: "Tickets", value: "On RA")) }
        fields.append(EntityField(label: "RA", value: "ra.co" + e.contentPath))

        self.init(
            id: "music:\(e.id)",
            name: e.title,
            kind: .event,
            origin: .music,
            subtitle: [e.venueName, MusicEnrichmentBridge.day(e)].compactMap { $0 }.joined(separator: " · "),
            fields: fields
        )
    }
}

enum MusicEnrichmentBridge {

    /// The whole feed as catalogue entities: every listing (.event), plus its artists
    /// (.person) and venues (.place), deduped across listings by RA id.
    static func entities(from events: [MusicEvent]) -> [EnrichedEntity] {
        var out = events.map(EnrichedEntity.init(music:))

        var seenArtists = Set<String>()
        var seenVenues = Set<String>()
        for e in events {
            for a in e.artists where seenArtists.insert(a.id).inserted {
                out.append(EnrichedEntity(id: "music:artist:\(a.id)", name: a.name,
                                          kind: .person, origin: .music, subtitle: "Artist"))
            }
            if let v = e.venue, seenVenues.insert(v.id).inserted {
                out.append(EnrichedEntity(id: "music:venue:\(v.id)", name: v.name,
                                          kind: .place, origin: .music, subtitle: v.areaName))
            }
        }
        return out
    }

    // MARK: labels

    private static let dayFormat: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "EEE d MMM"
        return f
    }()
    private static let clockFormat: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "HH:mm"
        return f
    }()

    static func day(_ e: MusicEvent) -> String {
        dayFormat.string(from: Date(timeIntervalSince1970: Double(e.startMs) / 1000))
    }

    static func when(_ e: MusicEvent) -> String {
        let start = Date(timeIntervalSince1970: Double(e.startMs) / 1000)
        var s = dayFormat.string(from: start) + " · " + clockFormat.string(from: start)
        if let end = e.endMs {
            s += " – " + clockFormat.string(from: Date(timeIntervalSince1970: Double(end) / 1000))
        }
        return s
    }
}
