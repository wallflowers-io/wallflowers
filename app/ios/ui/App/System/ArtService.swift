//  ArtService.swift
//  The Art System — ArtRabbit as a Pacific System. SCOPE-scoped like the Music System:
//  ArtRabbit is public, so there is no sign-in, no Keychain, no token. The persisted
//  scope is a PLACE (name + the coordinates ArtRabbit filters by); `refresh()` pulls
//  that scope's current listings and publishes them for the EVENTS page, the enrichment
//  catalogue and the resolver index. Fails LOUD in `status`; the EVENTS page just shows
//  local rows when ArtRabbit is down.
//
//  Where the Music System resolves its scope FROM SOURCE (RA's own area ids), ArtRabbit
//  has no ids to resolve — its own site geocodes the typed city into coordinates. The
//  on-device analog of that is CLGeocoder, so the scope search runs there: same
//  contract, no guessed numbers.

import SwiftUI
import CoreLocation

@MainActor
final class ArtService: ObservableObject {
    enum Status: Equatable { case idle, refreshing, loaded(Int), error(String) }

    @Published private(set) var events: [ArtEvent] = []
    @Published private(set) var status: Status = .idle
    @Published private(set) var place: ArtPlace?     // the persisted scope

    private let client = ArtClient()
    private let resolution: ResolutionService
    /// The device fix an UNPINNED scope follows — SF is no good in Brussels.
    private let location: LocationService

    private static let placeKey = "art.place"
    /// System OFF (4 Aug): the Art System is parked for now — no ArtRabbit pull,
    /// no reference rows in EVENTS, no catalogue entities. Flip to re-enable;
    /// every seam downstream stays wired.
    static let enabled = false
    /// Test opt-OUT: UI suites that assert exact list contents launch with this, so the
    /// page behaves exactly as it did before the Art System existed.
    static let optOutArg = "-pacificNoArt"
    /// The scope of last resort — only when there is no fix to follow (first run
    /// before authorization, location denied). Unpinned, so a fix upgrades it.
    static let defaultPlaceSearch = "San Francisco"

    init(resolution: ResolutionService, location: LocationService) {
        self.resolution = resolution
        self.location = location
        if let data = UserDefaults.standard.data(forKey: Self.placeKey),
           let p = try? JSONDecoder().decode(ArtPlace.self, from: data) {
            place = p
        }
    }

    /// The scope picker's search — a typed name to coordinate candidates, on-device.
    /// Errors thrown to the caller, which shows them.
    func searchPlaces(_ term: String) async throws -> [ArtPlace] {
        let marks = try await CLGeocoder().geocodeAddressString(term)
        var seen = Set<String>()
        return marks.compactMap { m in
            guard let loc = m.location else { return nil }
            let city = m.locality ?? m.name ?? term
            let name = m.country.map { "\(city), \($0)" } ?? city
            guard seen.insert(name).inserted else { return nil }
            return ArtPlace(name: name, lat: loc.coordinate.latitude,
                            lon: loc.coordinate.longitude,
                            city: city, country: m.country ?? "")
        }
    }

    /// Re-scope the System and re-pull. A PICKED scope is pinned: the choice persists
    /// and the device moving never overrides it.
    func setScope(_ p: ArtPlace) {
        var chosen = p
        chosen.pinned = true
        place = chosen
        persist(chosen)
        Task { await refresh() }
    }

    /// True when the current fix should change the scope: no scope yet, or an unpinned
    /// scope the device has left. Cheap — no geocoding, just distance.
    private var needsRescope: Bool {
        guard let fix = location.coordinate else { return false }
        guard let place else { return true }
        return !place.isPinned && NearScope.movedFar(fix, fromLat: place.lat, lon: place.lon)
    }

    /// The fix moved (or first arrived) — re-scope and re-pull ONLY if that would
    /// change anything. Safe to call on every location update; a no-op in steady state.
    func followLocation() async {
        guard status != .refreshing, needsRescope else { return }
        await refresh()
    }

    /// Resolve the scope from the device fix: the fix IS the coordinate ArtRabbit
    /// filters by; the reverse-geocode only supplies the city/country labels the
    /// server requires beside it. Fails soft: an unresolvable fix keeps whatever
    /// scope exists.
    private func autoScope(_ fix: CLLocationCoordinate2D) async {
        guard let mark = try? await CLGeocoder()
            .reverseGeocodeLocation(CLLocation(latitude: fix.latitude, longitude: fix.longitude))
            .first else { return }
        guard let city = mark.locality ?? mark.subAdministrativeArea ?? mark.administrativeArea
        else { return }
        let scoped = ArtPlace(name: mark.country.map { "\(city), \($0)" } ?? city,
                              lat: fix.latitude, lon: fix.longitude,
                              city: city, country: mark.country ?? "",
                              pinned: false)
        place = scoped
        persist(scoped)
    }

    /// Pull the scope's listings: one attempt, one page of 20 (the site's page size).
    /// An unpinned scope follows the device first (see `autoScope`); with no fix and
    /// no scope, the default is geocoded — the same path a typed search takes — and
    /// persisted unpinned, so the first fix upgrades it. Publishes the events and
    /// indexes them in the shared resolver.
    func refresh() async {
        guard Self.enabled else { return }
        guard !ProcessInfo.processInfo.arguments.contains(Self.optOutArg) else { return }
        status = .refreshing
        do {
            if needsRescope, let fix = location.coordinate {
                await autoScope(fix)
            }
            if place == nil {
                guard var sf = try await searchPlaces(Self.defaultPlaceSearch).first else {
                    throw ArtHTTPError.parse("no place matched “\(Self.defaultPlaceSearch)”")
                }
                sf.pinned = false
                place = sf
                persist(sf)
            }
            guard let place else { return }
            let pulled = try await client.eventListings(place: place)
            events = pulled.sorted { $0.startMs < $1.startMs }
            resolution.index(events.map(ResolvableEntity.init(art:)), from: "art")
            status = .loaded(events.count)
        } catch {
            status = .error((error as? LocalizedError)?.errorDescription ?? String(describing: error))
        }
    }

    private func persist(_ p: ArtPlace) {
        if let data = try? JSONEncoder().encode(p) {
            UserDefaults.standard.set(data, forKey: Self.placeKey)
        }
    }
}

// MARK: - Projection into the shared resolver index (the neutral ResolvableEntity tier)

extension ResolvableEntity {
    init(art e: ArtEvent) {
        var md: [String: String] = [:]
        if let v = e.venueName { md["venue"] = v }
        if let c = e.cityName { md["city"] = c }
        if let k = e.category { md["category"] = k }
        self.init(
            id: "art:\(e.id)",
            kind: "event",
            name: e.title,
            aliases: [],
            text: [e.title, e.venueName ?? "", e.cityName ?? "", e.category ?? ""]
                .filter { !$0.isEmpty }.joined(separator: " · "),
            metadata: md,
            sourceLocator: e.contentPath
        )
    }
}
