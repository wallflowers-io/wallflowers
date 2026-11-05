//  LifeGeo.swift
//  The app-side mirror of pacific-core's `geo.rs` — location as a COMMON facet of a
//  GroupObject (things, orgs, places, optionally people), not a place-only field.
//
//  Two shapes, one primitive (see geo.rs):
//    • FIXED  — a static literal coordinate (a Place, an org address).
//    • STREAM — a LIVE position (a moving car, a person). The delta carries only the
//               stream ACCESS, never the positions; they flow out-of-band. So a stream
//               source has NO coordinate to plot until it is resolved live.
//
//  This is a Swift-native model for now; the FFI bridge to the Rust LocationSource
//  lands with the "surface location" step. The Map reads a resolved coordinate off it.

import CoreLocation

/// A concrete WGS84 point. (Degrees here for ergonomics; the core stores e7 fixed-point
/// for deterministic replay — the FFI bridge converts.)
struct GeoCoordinate: Codable, Hashable {
    var lat: Double
    var lng: Double
    var clCoordinate: CLLocationCoordinate2D { .init(latitude: lat, longitude: lng) }
}

/// A GroupObject's location facet — the value a `setLocation` delta carries. Mirrors
/// `LocationSource = Fixed { point } | Stream { access }`.
enum EntityLocation: Codable, Hashable {
    /// A static coordinate, carried literally (places, addresses).
    case fixed(GeoCoordinate)
    /// A live position — ONLY the stream access (kind + reference), never a position.
    case stream(kind: String, reference: String)

    /// The coordinate to PLOT, if one is known now: a fixed point, else nil (a stream's
    /// live position is out-of-band, so it is unplaced until resolved).
    var plottableCoordinate: GeoCoordinate? {
        if case .fixed(let c) = self { return c } else { return nil }
    }
    var isStream: Bool {
        if case .stream = self { return true } else { return false }
    }
}

/// The Map's input — one placed entity. Derived from an `EntityVM` that carries a
/// plottable location; entities without one are honest-absent (listed, never pinned).
struct PlaceAnnotation: Identifiable {
    let id: String
    let name: String
    let coordinate: CLLocationCoordinate2D
    let category: String?
    /// Stream-backed (a moving thing) vs a fixed place — drives a "live" pin treatment.
    let isLive: Bool
    var glyph: String

    init(id: String, name: String, coordinate: CLLocationCoordinate2D,
         category: String? = nil, isLive: Bool = false, glyph: String = "mappin.circle.fill") {
        self.id = id; self.name = name; self.coordinate = coordinate
        self.category = category; self.isLive = isLive; self.glyph = glyph
    }

    /// Project an entity onto the map, or nil when it has no plottable coordinate.
    init?(entity: EntityVM) {
        guard let loc = entity.location, let c = loc.plottableCoordinate else { return nil }
        self.init(id: entity.id, name: entity.name, coordinate: c.clCoordinate,
                  category: entity.descriptor, isLive: loc.isStream)
    }

    /// DEMO SEED — a few placed places so the map renders on device before the real
    /// location facet is surfaced through core → FFI. Remove the fallback once real
    /// coordinates flow (see LifeView.placeAnnotations).
    static let demo: [PlaceAnnotation] = [
        .init(id: "seed-bristol", name: "Bristol", coordinate: .init(latitude: 51.4545, longitude: -2.5879), category: "John · Hope"),
        .init(id: "seed-stroud",  name: "Stroud",  coordinate: .init(latitude: 51.7449, longitude: -2.2158), category: "Mum · Dad"),
        .init(id: "seed-london",  name: "London",  coordinate: .init(latitude: 51.5074, longitude: -0.1278), category: "Ralph"),
        .init(id: "seed-wales",   name: "Wales",   coordinate: .init(latitude: 51.9481, longitude: -3.3918), category: "the trip"),
    ]
}
