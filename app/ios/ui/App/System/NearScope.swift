//  NearScope.swift
//  The shared "scope follows the device" arithmetic for the public listing Systems
//  (Music/RA, Art/ArtRabbit). A scope the user PICKED is pinned and never touched; a
//  scope the System resolved is a stand-in for "near me", so it re-resolves when the
//  device has genuinely moved — SF is no good in Brussels. Pure helpers, testable.

import CoreLocation

enum NearScope {
    /// How far the fix must sit from where the scope was resolved before it re-resolves.
    /// Wide enough that a city and its metro stay one scope (SF↔Oakland), tight enough
    /// that neighbouring scenes don't blur (Brussels↔Antwerp is ~44 km).
    static let rescopeMeters: CLLocationDistance = 30_000

    /// True when `fix` is far enough from the coordinate the scope was resolved at.
    /// A scope with NO stored coordinate (legacy records, the no-fix fallback) always
    /// counts as moved — it was never really "near" anywhere.
    static func movedFar(_ fix: CLLocationCoordinate2D, fromLat lat: Double?, lon: Double?) -> Bool {
        guard let lat, let lon else { return true }
        return CLLocation(latitude: fix.latitude, longitude: fix.longitude)
            .distance(from: CLLocation(latitude: lat, longitude: lon)) > rescopeMeters
    }

    /// The search terms a placemark yields, most local first — the cascade a source's
    /// own area search is tried with ("Ixelles" misses RA, "Brussels" hits).
    static func terms(locality: String?, subAdministrativeArea: String?,
                      administrativeArea: String?) -> [String] {
        var seen = Set<String>()
        return [locality, subAdministrativeArea, administrativeArea]
            .compactMap { $0?.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty && seen.insert($0.lowercased()).inserted }
    }
}
