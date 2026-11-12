//  EventExterior.swift
//  An EVENT's content from its outside's fields (`facets.exterior.fields.event`): one parse of
//  what core's `fields_of` built, so a member's page and a stranger's read the same thing.

import SwiftUI
import MapKit
import PacificCore

extension EventContent {
    /// The content the fields say, without their media and the names a device resolves.
    init(fields f: [String: Any]) {
        self.init()
        let s = { (k: String) in f[k] as? String ?? "" }
        let n = { (k: String) in (f[k] as? NSNumber)?.int64Value ?? 0 }
        title = s("title")
        descriptor = s("descriptor")
        markdown = s("descriptorFormat") == "markdown"
        startMs = n("startMs")
        endMs = n("endMs")
        allDay = n("allDay") == 1
        tz = s("tz")
        recurrence = s("recurrence")
        status = s("status")
        online = s("online")
        videoUrl = s("videoUrl")
        ticketUrl = s("ticketUrl")
        venue = s("venue")
        legacyLineup = f["lineup"] as? [String] ?? []
        priceCents = n("priceCents")
        currency = s("currency")
        capacity = n("capacity")
        going = Int(n("going"))
        hosts = (f["hosts"] as? [[String: Any]] ?? []).compactMap { h in
            (h["key"] as? String).map { Host(id: $0, name: h["name"] as? String ?? "", isSpace: false) }
        }
        acts = (f["acts"] as? [[String: Any]] ?? []).compactMap { a in
            (a["performer"] as? String).map {
                Act(id: $0, name: "", role: a["role"] as? String ?? "",
                    start: (a["start"] as? NSNumber)?.int64Value ?? 0, end: (a["end"] as? NSNumber)?.int64Value ?? 0)
            }
        }
    }

    /// An outside, whole: the fields, its Place at its owner's precision, its pictures and clip
    /// (fetched through core), and the names this device knows for its hosts and acts.
    @MainActor
    static func outside(_ row: OutsideRow, core: CoreService) async -> EventContent? {
        let f = KindExteriors.fields(row)
        guard !f.isEmpty else { return nil }
        var c = EventContent(fields: f)
        if row.placed { c.point = CLLocationCoordinate2D(latitude: row.lat, longitude: row.lng) }
        for m in row.media {
            guard let data = await core.outsideData(m) else { continue }
            switch m.field {
            case "banner": c.banner = data
            case "clip": c.clip = KindClip(data: data, mime: m.mime)
            case let p where p.hasPrefix("photos:"):
                c.photos.append(KindPicture(id: Int(p.dropFirst(7)) ?? c.photos.count, data: data))
            default: break
            }
        }
        c.hosts = c.hosts.map { h in
            var h = h
            if let known = core.index.name(h.id) { h = Host(id: h.id, name: known, isSpace: false) }
            h.photo = core.index.photo(h.id)
            h.color = core.index.color(h.id)
            return h
        }
        c.acts = c.acts.map { a in
            let name = core.index.name(a.id) ?? core.objectTitle(a.id)
            return Act(id: a.id, name: name.isEmpty ? String(a.id.prefix(8)) : name,
                       role: a.role, start: a.start, end: a.end, photo: core.index.photo(a.id))
        }
        return c
    }
}
