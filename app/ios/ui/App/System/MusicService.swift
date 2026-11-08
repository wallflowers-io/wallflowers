//  MusicService.swift
//  The Music System — Resident Advisor as a Pacific System. SCOPE-scoped, not
//  credential-scoped: RA is public, so there is no sign-in, no Keychain, no token.
//  The persisted scope is an AREA (id + name) plus a rolling date window (today →
//  +14d); `refresh()` pulls that scope's listings and publishes them for the EVENTS
//  page, the enrichment catalogue and the resolver index. Fails LOUD in `status`;
//  the EVENTS page just shows local rows when RA is down.

import SwiftUI
import CoreLocation

@MainActor
final class MusicService: ObservableObject {
    enum Status: Equatable { case idle, refreshing, loaded(Int), error(String) }

    @Published private(set) var events: [MusicEvent] = []
    @Published private(set) var status: Status = .idle
    @Published private(set) var area: MusicArea?     // the persisted scope

    private let client = MusicClient()
    private let resolution: ResolutionService
    /// The device fix an UNPINNED scope follows — SF is no good in Brussels.
    private let location: LocationService

    private static let areaKey = "music.area"
    /// System OFF (4 Aug): the Music System is parked for now — no RA pull, no
    /// reference rows in EVENTS, no catalogue entities. Flip to re-enable; every
    /// seam downstream stays wired.
    static let enabled = false
    /// Test opt-OUT: UI suites that assert exact list contents launch with this, so the
    /// page behaves exactly as it did before the Music System existed.
    static let optOutArg = "-pacificNoMusic"
    /// The scope of last resort — only when there is no fix to follow (first run
    /// before authorization, location denied). Unpinned, so a fix upgrades it.
    static let defaultAreaSearch = "San Francisco"
    static let windowDays = 14

    init(resolution: ResolutionService, location: LocationService) {
        self.resolution = resolution
        self.location = location
        if let data = UserDefaults.standard.data(forKey: Self.areaKey),
           let a = try? JSONDecoder().decode(MusicArea.self, from: data) {
            area = a
        }
    }

    /// The scope picker's search — errors thrown to the caller, which shows them.
    func searchAreas(_ term: String) async throws -> [MusicArea] {
        try await client.areas(matching: term)
    }

    /// Re-scope the System and re-pull. A PICKED scope is pinned: the choice persists
    /// and the device moving never overrides it.
    func setScope(_ a: MusicArea) {
        var chosen = a
        chosen.pinned = true
        chosen.lat = nil
        chosen.lon = nil
        area = chosen
        persist(chosen)
        Task { await refresh() }
    }

    /// True when the current fix should change the scope: no scope yet, or an unpinned
    /// scope the device has left. Cheap — no geocoding, just distance.
    private var needsRescope: Bool {
        guard let fix = location.coordinate else { return false }
        guard let area else { return true }
        return !area.isPinned && NearScope.movedFar(fix, fromLat: area.lat, lon: area.lon)
    }

    /// The fix moved (or first arrived) — re-scope and re-pull ONLY if that would
    /// change anything. Safe to call on every location update; a no-op in steady state.
    func followLocation() async {
        guard status != .refreshing, needsRescope else { return }
        await refresh()
    }

    /// Resolve the scope from the device fix: reverse-geocode, then RA's own area
    /// search over the placemark's terms, most local first — the area id still comes
    /// FROM SOURCE, the fix only chooses the words to ask with. Fails soft: an
    /// unresolvable fix keeps whatever scope exists.
    private func autoScope(_ fix: CLLocationCoordinate2D) async {
        guard let mark = try? await CLGeocoder()
            .reverseGeocodeLocation(CLLocation(latitude: fix.latitude, longitude: fix.longitude))
            .first else { return }
        let terms = NearScope.terms(locality: mark.locality,
                                    subAdministrativeArea: mark.subAdministrativeArea,
                                    administrativeArea: mark.administrativeArea)
        for term in terms {
            if var hit = try? await client.areas(matching: term).first {
                hit.pinned = false
                hit.lat = fix.latitude
                hit.lon = fix.longitude
                area = hit
                persist(hit)
                return
            }
        }
    }

    /// Pull the scope's listings: one attempt, one page, the rolling window. An
    /// unpinned scope follows the device first (see `autoScope`); with no fix and no
    /// scope, the default is resolved FROM SOURCE (never a guessed int) and persisted
    /// unpinned, so the first fix upgrades it. Publishes the events and indexes them
    /// in the shared resolver.
    func refresh() async {
        guard Self.enabled else { return }
        guard !ProcessInfo.processInfo.arguments.contains(Self.optOutArg) else { return }
        status = .refreshing
        do {
            if needsRescope, let fix = location.coordinate {
                await autoScope(fix)
            }
            if area == nil {
                guard var sf = try await client.areas(matching: Self.defaultAreaSearch).first else {
                    throw MusicHTTPError.decode("no RA area matched “\(Self.defaultAreaSearch)”")
                }
                sf.pinned = false
                area = sf
                persist(sf)
            }
            guard let area else { return }
            let from = Date()
            let to = Calendar.current.date(byAdding: .day, value: Self.windowDays, to: from) ?? from
            let pulled = try await client.eventListings(areaId: area.id, from: from, to: to,
                                                        page: 1, pageSize: 50)
            events = pulled.sorted { $0.startMs < $1.startMs }
            resolution.index(events.map(ResolvableEntity.init(music:)), from: "music")
            status = .loaded(events.count)
        } catch {
            status = .error((error as? LocalizedError)?.errorDescription ?? String(describing: error))
        }
    }

    private func persist(_ a: MusicArea) {
        if let data = try? JSONEncoder().encode(a) {
            UserDefaults.standard.set(data, forKey: Self.areaKey)
        }
    }
}

// MARK: - Projection into the shared resolver index (the neutral ResolvableEntity tier)

extension ResolvableEntity {
    init(music e: MusicEvent) {
        var md: [String: String] = [:]
        if let v = e.venueName { md["venue"] = v }
        if let a = e.areaName { md["area"] = a }
        let names = e.artists.map(\.name)
        self.init(
            id: "ra:\(e.id)",
            kind: "event",
            name: e.title,
            aliases: names,
            text: ([e.title] + names + [e.venueName ?? "", e.areaName ?? ""])
                .filter { !$0.isEmpty }.joined(separator: " · "),
            metadata: md,
            sourceLocator: e.contentPath
        )
    }
}
