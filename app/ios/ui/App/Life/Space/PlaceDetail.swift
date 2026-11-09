//  PlaceDetail.swift
//  The PLACE's maximised detail — what replaced the Place Profile page.
//
//  A Place is not a who: it is somewhere, so it takes the page every non-egoic
//  GroupObject shares (`Design/Detail`) rather than a profile's IA. Its media is
//  its AREA — the geohash patch around its own point — because Places have no
//  banner op, and a block of colour standing in for a picture would say less.
//
//  AREA CELLS — the precision ruling. Cells are derived at geohash precision 6
//  (~1.22 × 0.61 km) from the Place's own declared point, never persisted:
//    • the 3×3 patch (~3.7 × 1.8 km) reads as an AREA — texture, not cartography;
//      the venue-encode precision (9, ~4.8 m) would draw a rooftop and claim
//      survey accuracy a render cannot honour;
//    • 6 is PlaceFix's own coarsest honest bucket (accuracy > 150 m → p6), so the
//      page never claims tighter than the loosest fix the mint pipe accepts;
//    • the anti-triangulation doctrine floors PERSISTED cells at 3..=5 — these are
//      client-side only, from the Place's own point, which is its own disclosure.
//
//  Its rows REPORT. A Place's door, doorbell and land are written where they
//  always were (PlacePublicSheet); the posts published at it are `place.post`.

import SwiftUI
import PacificCore

@MainActor
enum PlaceDetail {
    static var tint: Color { LifeFilter.places.color }

    static func open(_ objectId: String, core: CoreService, shell: ShellState, origin: CGRect? = nil) {
        shell.beginNew(DetailSession(noun: "place",
                                     source: PlaceDetailSource(core: core, objectId: objectId),
                                     origin: origin))
    }

    /// The precision the area draws at — see the header for the ruling. Distinct from
    /// `Geohash.venuePrecision` (9), which is an ENCODE precision for proximity
    /// queries, not a display statement.
    static let areaPrecision = 6

    /// The Place's cell and its eight neighbours, projected into a unit frame of
    /// `aspect` (north up). Empty when the Place is unplaced: no area is drawn.
    static func areaCells(lat: Double, lng: Double, placed: Bool, aspect: CGFloat = 1) -> [ProfileGeoCell] {
        guard placed else { return [] }
        let patch = Geohash.patch(around: .init(latitude: lat, longitude: lng), precision: areaPrecision)
        return Geohash.paint(patch, aspect: aspect)
            .map { ProfileGeoCell(hash: $0.hash, x: $0.x, y: $0.y, w: $0.w, h: $0.h) }
    }
}

@MainActor
struct PlaceDetailSource: DetailSource {
    let core: CoreService
    let objectId: String

    private var row: PlaceRow? { core.places.first { $0.id == objectId } }

    func headline() async -> DetailHeadline? {
        guard let row else { return nil }
        var rows = [
            DetailRow(label: "Describe", value: row.descriptor),
            DetailRow(label: "Address", value: row.address.isEmpty ? row.label : row.address),
            // PUBLIC means auto-admitted at its door, not world-readable.
            DetailRow(label: "Access", value: row.access == "public" ? "Open door" : ""),
            DetailRow(label: "Doorbell", value: row.doorbell.isEmpty ? "" : "Doorbell fitted"),
        ]
        // Whose land, and whether they were asked: two claims, shown only when
        // somebody has looked.
        if !row.landParcel.isEmpty {
            let asked = switch row.landPermission {
            case "granted": "Owner agreed"
            case "asked":   "Owner asked"
            case "refused": "Owner refused"
            default:        "Owner not asked"
            }
            let land = [row.landParcel, row.landProprietor, asked].filter { !$0.isEmpty }
            rows.append(DetailRow(label: "Land", value: land.joined(separator: " · ")))
        }
        // The notes published here (`place.post`), newest first.
        for p in core.placePosts(objectId).sorted(by: { $0.at > $1.at }) {
            let who = core.index.name(p.author) ?? String(p.author.prefix(8))
            let words = [p.title, p.text].filter { !$0.isEmpty }.joined(separator: "\n")
            rows.append(DetailRow(label: "Post \(p.author):\(p.note)", value: "\(who) — \(words)"))
        }
        return DetailHeadline(title: row.name,
                              area: PlaceDetail.areaCells(lat: row.lat, lng: row.lng, placed: row.placed),
                              rows: rows)
    }

    func author() async -> DetailAuthor? {
        DetailBind.author(of: objectId, core: core, tint: PlaceDetail.tint)
    }

    func comments() async -> [DetailComment] { DetailBind.comments(of: objectId, core: core) }

    func commentsHeld() async -> Bool { core.index.commentsForum(of: objectId) != nil }

    func post(_ text: String) async -> Bool { await DetailBind.post(text, on: objectId, core: core) }
}
