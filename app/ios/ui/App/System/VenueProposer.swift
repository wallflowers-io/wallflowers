//  VenueProposer.swift
//  Music System venues → Place SUGGESTIONS. Every NEW venue the RA feed surfaces
//  becomes one Proposal(kind: .mint, type: .place) in the ONE ProposalStore — an
//  actionable MINT banner (accept mints without opening the app) and a Convergence
//  card (swipe right mints the Place GroupObject; ProposalStore's .place arm).
//
//  Two dedup gates, in order:
//    • the SEEN-SET — RA venue ids ever asked about, persisted (a local dedup
//      pattern). One question per venue, ever: rejected must not return on the next
//      refresh, and an undecided card is ProposalStore's to keep (sourced proposals
//      persist until decided), so re-asking here would only mint duplicates.
//    • the MINTED gate — a venue whose Place already exists (normalized-name match
//      against core.places) never proposes; the question is already answered.
//
//  A new venue's detail (address + coordinates) is fetched from RA best-effort —
//  one call per NEW venue only, so the steady state is zero network. Coordinates
//  ride the proposal's frozen `features` (they are legitimate features — and the
//  accept arm reads them to place the mint); no detail ⇒ the proposal still works,
//  the Place just mints unplaced.

import SwiftUI

@MainActor
final class VenueProposer: ObservableObject {

    /// What the detail fetch answers; injectable so tests never touch the network.
    struct VenueDetail {
        var address: String?
        var lat: Double?
        var lng: Double?
    }

    /// Lock-screen budget per refresh. A first sync of an area surfaces dozens of
    /// venues; five banners is a suggestion, forty is an incident. The overflow
    /// still reaches the deck — silently.
    static let bannerCap = 5

    /// RA venue ids ever proposed (or found already minted) — the durable half of
    /// the dedup, persisted locally — a question already asked is this device's own
    /// bookkeeping, not a fact about the world.
    private var seen: Set<String>
    /// The Art System's half, its own set: ArtRabbit has no venue ids, so these keys
    /// are normalized venue names ("name:…") — and, for a card that names no venue,
    /// the event that led us to it ("ev:…").
    private var artSeen: Set<String>
    private let defaults: UserDefaults
    private static let seenKey = "pacific.music.venues.seen"
    private static let artSeenKey = "pacific.art.venues.seen"

    /// The detail fetch. Production = MusicClient.venueDetail; tests inject.
    private var detail: (String) async -> VenueDetail?
    /// The Art System's detail fetch — the event PAGE, whose schema.org block carries
    /// the venue's name and street address. Production = ArtClient.eventDetail.
    private var artDetail: (String) async -> ArtClient.EventDetailWire?

    init(defaults: UserDefaults = .standard,
         detail: ((String) async -> VenueDetail?)? = nil,
         artDetail: ((String) async -> ArtClient.EventDetailWire?)? = nil) {
        self.defaults = defaults
        seen = Set(defaults.stringArray(forKey: Self.seenKey) ?? [])
        artSeen = Set(defaults.stringArray(forKey: Self.artSeenKey) ?? [])
        #if DEBUG
        // Launch-arg state reset (never a host-side rm — cfprefsd would resurrect it):
        // the art sweep re-asks from scratch, the dev/test seam for a re-run.
        if ProcessInfo.processInfo.arguments.contains("-pacificResetArtVenues") {
            artSeen = []
            defaults.removeObject(forKey: Self.artSeenKey)
        }
        #endif
        self.detail = detail ?? { id in
            let client = MusicClient()
            guard let wire = try? await client.venueDetail(id: id) else { return nil }
            return VenueDetail(address: wire.address,
                               lat: wire.location?.latitude,
                               lng: wire.location?.longitude)
        }
        self.artDetail = artDetail ?? { path in
            try? await ArtClient().eventDetail(path: path)
        }
    }

    /// One pass over a refreshed feed — call it whenever the Music System publishes.
    /// Reads the minted Places FRESH (`refreshPlaces` is a cheap local fold), so a
    /// Place minted since the last sweep suppresses its venue immediately.
    func sweep(events: [MusicEvent], core: CoreService, proposals: ProposalStore) async {
        await core.refreshPlaces().value
        await sweep(events: events,
                    mintedPlaces: Set(core.places.map { Self.normalized($0.name) }),
                    proposals: proposals)
    }

    /// The testable core: distinct venues per refresh → the two gates → proposals.
    func sweep(events: [MusicEvent], mintedPlaces: Set<String>, proposals: ProposalStore) async {
        // Distinct venues, feed order, the first (soonest) event kept as the card's context.
        var order: [String] = []
        var byId: [String: (venue: MusicVenue, event: MusicEvent)] = [:]
        for e in events {
            guard let v = e.venue, byId[v.id] == nil else { continue }
            byId[v.id] = (v, e)
            order.append(v.id)
        }

        var out: [Proposal] = []
        var changed = false
        for id in order {
            guard let (venue, event) = byId[id], !seen.contains(id) else { continue }
            // Asked is asked — whatever happens next, this venue never re-proposes.
            // (An already-minted one enters too: its question is already answered.)
            seen.insert(id)
            changed = true
            guard !mintedPlaces.contains(Self.normalized(venue.name)) else { continue }
            out.append(Self.proposal(venue: venue, event: event, detail: await detail(id)))
        }
        if changed { defaults.set(Array(seen).sorted(), forKey: Self.seenKey) }
        guard !out.isEmpty else { return }
        proposals.propose(out, announceLimit: Self.bannerCap)
    }

    // MARK: — the Art System's sweep (same path, the address is the point)

    /// One pass over a refreshed art feed — call it whenever the Art System publishes.
    /// Same two gates as the music sweep, plus the ADDRESS gate: an art venue proposes
    /// only with an actual street address from its event page — a city is not a place
    /// you can stand in front of, and the minted Place carries the address as its
    /// descriptor. Coordinates come from the listing card itself (no second fetch).
    func sweepArt(events: [ArtEvent], core: CoreService, proposals: ProposalStore) async {
        await core.refreshPlaces().value
        await sweepArt(events: events,
                       mintedPlaces: Set(core.places.map { Self.normalized($0.name) }),
                       proposals: proposals)
    }

    /// The testable core. Per venue: the page is fetched ONCE (a transport failure is
    /// NOT marked seen — it retries on a later refresh; a fetched page decides now,
    /// whatever it says), then the address gate, then the minted gate.
    func sweepArt(events: [ArtEvent], mintedPlaces: Set<String>, proposals: ProposalStore) async {
        // Distinct venues per refresh, feed order. A card that names no venue is still
        // a lead — its PAGE names one — keyed by the event until the page answers.
        var order: [String] = []
        var byKey: [String: ArtEvent] = [:]
        for e in events {
            let key = e.venueName.map { "name:\(Self.normalized($0))" } ?? "ev:\(e.id)"
            guard byKey[key] == nil else { continue }
            byKey[key] = e
            order.append(key)
        }

        var out: [Proposal] = []
        var changed = false
        for key in order {
            guard let event = byKey[key], !artSeen.contains(key) else { continue }
            guard let page = await artDetail(event.contentPath) else {
                NSLog("[art-venues] %@ page fetch failed — retries next refresh", event.contentPath)
                continue
            }
            artSeen.insert(key)
            changed = true
            guard let name = page.venueName ?? event.venueName else {
                NSLog("[art-venues] %@ names no venue anywhere — skipped", key)
                continue
            }
            // The page's venue may already be seen under its own name (the same gallery
            // reached through a venueless card) — asked is asked, under either key.
            let nameKey = "name:\(Self.normalized(name))"
            if nameKey != key {
                guard artSeen.insert(nameKey).inserted else { continue }
            }
            // THE gate this sweep exists for: an actual address, or no proposal.
            guard let street = page.streetAddress else {
                NSLog("[art-venues] %@ has no street address — city is insufficient, skipped", name)
                continue
            }
            guard !mintedPlaces.contains(Self.normalized(name)) else { continue }
            out.append(Self.artProposal(name: name, street: street,
                                        locality: page.locality, event: event))
        }
        if changed { defaults.set(Array(artSeen).sorted(), forKey: Self.artSeenKey) }
        NSLog("[art-venues] sweep: %d venues, %d proposed", order.count, out.count)
        guard !out.isEmpty else { return }
        proposals.propose(out, announceLimit: Self.bannerCap)
    }

    /// One art venue as the deck's binary question. The SUMMARY is the street address —
    /// it becomes the minted Place's descriptor, so the Place knows where it IS, not
    /// just what city it's in; the card's own coordinates ride `features` for the
    /// accept arm's location fix.
    static func artProposal(name: String, street: String, locality: String?,
                            event: ArtEvent) -> Proposal {
        var features: [String: Double] = ["source_art": 1]
        if let lat = event.venueLat, let lng = event.venueLon {
            features["lat"] = lat
            features["lng"] = lng
        }
        return Proposal(
            id: "art-venue:\(normalized(name))",
            kind: .mint,
            subject: .init(
                name: name,
                type: .place,
                summary: [street, locality].compactMap { $0 }.joined(separator: ", "),
                snippet: "Hosts \(event.title)",
                highlightStart: nil, highlightLength: nil),
            target: nil,
            rationale: "A venue in your art feed you haven't tracked.",
            confidence: 0.5,
            provenance: .init(
                episodeId: "art:\(event.id)",
                source: .art,
                occurredAt: Int(Date().timeIntervalSince1970 * 1000),
                label: event.cityName.map { "ArtRabbit · \($0)" } ?? "ArtRabbit"),
            features: features)
    }

    static func normalized(_ name: String) -> String {
        name.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    }

    /// One venue as the deck's binary question. The id is stable ("ra-venue:<id>") so
    /// announce-once survives relaunch; the summary is the DESCRIPTOR the accept mints
    /// with; coordinates ride `features` for the .place arm's location fix.
    static func proposal(venue: MusicVenue, event: MusicEvent, detail: VenueDetail?) -> Proposal {
        let area = venue.areaName ?? event.areaName
        var features: [String: Double] = ["source_music": 1]
        if let lat = detail?.lat, let lng = detail?.lng {
            features["lat"] = lat
            features["lng"] = lng
        }
        return Proposal(
            id: "ra-venue:\(venue.id)",
            kind: .mint,
            subject: .init(
                name: venue.name,
                type: .place,
                summary: area.map { "RA venue · \($0)" } ?? "RA venue",
                // What makes the question answerable: where it is, else what it's hosting.
                snippet: (detail?.address).flatMap { $0.isEmpty ? nil : $0 }
                    ?? "Hosts \(event.title)",
                highlightStart: nil, highlightLength: nil),
            target: nil,
            rationale: "A venue in your music feed you haven't tracked.",
            confidence: 0.5,
            provenance: .init(
                episodeId: "ra:\(event.id)",
                source: .music,
                occurredAt: Int(Date().timeIntervalSince1970 * 1000),
                label: area.map { "Resident Advisor · \($0)" } ?? "Resident Advisor"),
            features: features)
    }

    #if DEBUG
    /// Headless drive: `-pacificSeedVenueProposal <name>` pushes ONE fixture venue
    /// through the REAL sweep (seen-set, minted gate, store injection, banner) — the
    /// UI test's seam, since simctl cannot make RA list a new venue on cue. Clears
    /// prior sourced state first (launch-arg reset, never a host-side rm); the fixture
    /// detail carries a real SF coordinate so accept exercises the placed half.
    static let seedArg = "pacificSeedVenueProposal"

    func seedIfRequested(core: CoreService, proposals: ProposalStore) async {
        guard let name = UserDefaults.standard.string(forKey: Self.seedArg),
              !name.isEmpty else { return }
        proposals.resetSourced()
        let venue = MusicVenue(id: "seed:\(name)", name: name, areaName: "San Francisco")
        let event = MusicEvent(id: "seed-ev:\(name)", title: "Warehouse night",
                               startMs: Int64(Date().timeIntervalSince1970 * 1000),
                               endMs: nil, venue: venue, areaName: "San Francisco",
                               artists: [], flyerURL: nil,
                               contentPath: "/events/seed", isTicketed: false, attending: 0)
        let live = detail
        detail = { _ in VenueDetail(address: "375 11th St, San Francisco",
                                    lat: 37.7706, lng: -122.4135) }
        await sweep(events: [event], core: core, proposals: proposals)
        detail = live
        NSLog("[seed-venue] proposed %@ (pending=%d)", name, proposals.pending.count)
    }
    #endif
}
