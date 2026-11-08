//  NearScopeTests.swift
//  The "scope follows the device" arithmetic: when a fix counts as having left the
//  scope, which search terms a placemark yields, and that LEGACY persisted scopes
//  (no `pinned` field) decode as unpinned — they were only ever the default, and the
//  first fix must be allowed to upgrade them.

import Testing
import Foundation
import CoreLocation
@testable import WallFlowers

struct NearScopeTests {

    private static let sf = CLLocationCoordinate2D(latitude: 37.7749, longitude: -122.4194)
    private static let brussels = CLLocationCoordinate2D(latitude: 50.8503, longitude: 4.3517)

    @Test func movedFar_isMetroTolerant_and_cityStrict() {
        // No stored coordinate (legacy / fallback scope) — always counts as moved.
        #expect(NearScope.movedFar(Self.sf, fromLat: nil, lon: nil))
        // SF ↔ Oakland is one scene (~13 km): not moved.
        #expect(!NearScope.movedFar(Self.sf, fromLat: 37.8044, lon: -122.2712))
        // Brussels ↔ Antwerp are different scenes (~44 km): moved.
        #expect(NearScope.movedFar(Self.brussels, fromLat: 51.2194, lon: 4.4025))
        // An intercontinental flight, obviously.
        #expect(NearScope.movedFar(Self.brussels, fromLat: Self.sf.latitude, lon: Self.sf.longitude))
    }

    @Test func terms_cascadeMostLocalFirst_dedupedAndCleaned() {
        #expect(NearScope.terms(locality: "Brussels", subAdministrativeArea: "Brussels",
                                administrativeArea: "Brussels Hoofdstedelijk Gewest")
                == ["Brussels", "Brussels Hoofdstedelijk Gewest"])
        #expect(NearScope.terms(locality: nil, subAdministrativeArea: " ",
                                administrativeArea: "California") == ["California"])
    }

    @Test func legacyScopes_decodeUnpinned_soTheFirstFixUpgradesThem() throws {
        let area = try JSONDecoder().decode(
            MusicArea.self, from: Data(#"{"id":218,"name":"San Francisco/Oakland"}"#.utf8))
        #expect(!area.isPinned)
        #expect(area.lat == nil)

        let place = try JSONDecoder().decode(ArtPlace.self, from: Data(
            #"{"name":"San Francisco, United States","lat":37.77,"lon":-122.41,"city":"San Francisco","country":"United States"}"#.utf8))
        #expect(!place.isPinned)

        // And a picked scope stays pinned through its round-trip.
        let pinned = try JSONDecoder().decode(
            MusicArea.self,
            from: JSONEncoder().encode(MusicArea(id: 405, name: "Brussels", pinned: true)))
        #expect(pinned.isPinned)
    }
}
