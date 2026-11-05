//  PlaceSearch.swift
//  THE MapKit lookup behind every "new place" search field (EventPlacePicker's
//  venue mint, PlacePublishSheet's publish-here mint). One function so the request
//  shape — result types, the 50 km region, the result cap, errors-to-empty — can
//  never quietly diverge between the two sheets; the caller supplies only the
//  anchor its own context knows (a dropped pin, the live fix).

import MapKit

enum PlaceSearch {
    /// Search Apple's map for `query`, biased around `near` when there is one.
    static func nearby(_ query: String, near: CLLocationCoordinate2D?) async -> [MKMapItem] {
        let q = query.trimmingCharacters(in: .whitespaces)
        guard !q.isEmpty else { return [] }
        let req = MKLocalSearch.Request()
        req.naturalLanguageQuery = q
        req.resultTypes = [.pointOfInterest, .address]
        if let near {
            req.region = MKCoordinateRegion(center: near,
                                            latitudinalMeters: 50_000, longitudinalMeters: 50_000)
        }
        let found = (try? await MKLocalSearch(request: req).start().mapItems) ?? []
        return Array(found.prefix(8))
    }
}
