//  MusicClient.swift
//  The HTTP client for the Music System — Resident Advisor's public GraphQL endpoint
//  (POST https://ra.co/graphql). Undocumented but open: public reads need no auth, but
//  Cloudflare challenges anything that doesn't look like a browser, so every request
//  carries a real browser User-Agent and a Referer. One attempt per call, loud errors,
//  no retry storms — the SysiphusClient shape over a GraphQL wire.

import Foundation

enum MusicHTTPError: Error, LocalizedError {
    case badStatus(Int)
    case transport(String)
    case decode(String)
    case graphql(String)

    var errorDescription: String? {
        switch self {
        case .badStatus(let s): return "RA returned HTTP \(s)"
        case .transport(let m): return "Couldn’t reach RA: \(m)"
        case .decode(let m):    return "RA response wasn’t understood: \(m)"
        case .graphql(let m):   return "RA refused the query: \(m)"
        }
    }
}

struct MusicClient {
    var endpoint: URL = URL(string: "https://ra.co/graphql")!
    private let session: URLSession = .shared

    // MARK: areas — the scope picker's search

    private static let areasQuery = """
    query($searchTerm: String) { areas(searchTerm: $searchTerm) { id name } }
    """

    func areas(matching term: String) async throws -> [MusicArea] {
        struct Vars: Encodable { let searchTerm: String }
        struct Payload: Decodable { let areas: [AreaWire]? }
        let payload: Payload = try await send(query: Self.areasQuery, variables: Vars(searchTerm: term))
        // Area ids arrive as strings ("218") but filter as ints — convert at the edge.
        return (payload.areas ?? []).compactMap { a in Int(a.id).map { MusicArea(id: $0, name: a.name) } }
    }

    // MARK: eventListings — the scope's feed

    private static let listingsQuery = """
    query($filters: FilterInputDtoInput, $page: Int, $pageSize: Int) {
      eventListings(filters: $filters, page: $page, pageSize: $pageSize) {
        data {
          id
          event {
            id title date startTime endTime contentUrl flyerFront isTicketed attending
            images { filename type }
            venue { id name area { id name } }
            artists { id name }
          }
        }
        totalResults
      }
    }
    """

    /// One page of listings for an area over a date window, flattened to MusicEvents.
    /// A multi-day event lists once per day — deduped here by event id, first (soonest)
    /// listing kept, so the feed carries each event once.
    func eventListings(areaId: Int, from: Date, to: Date,
                       page: Int = 1, pageSize: Int = 50) async throws -> [MusicEvent] {
        struct Vars: Encodable {
            struct Filters: Encodable {
                struct IntEq: Encodable { let eq: Int }
                struct DateRange: Encodable { let gte: String; let lte: String }
                let areas: IntEq
                let listingDate: DateRange
            }
            let filters: Filters
            let page: Int
            let pageSize: Int
        }
        struct Payload: Decodable { let eventListings: ListingsWire? }

        let iso = ISO8601DateFormatter()
        iso.formatOptions = [.withInternetDateTime]
        let vars = Vars(filters: .init(areas: .init(eq: areaId),
                                       listingDate: .init(gte: iso.string(from: from),
                                                          lte: iso.string(from: to))),
                        page: page, pageSize: min(pageSize, 100))   // RA caps pageSize at 100

        let payload: Payload = try await send(query: Self.listingsQuery, variables: vars)
        guard let listings = payload.eventListings else {
            throw MusicHTTPError.decode("no eventListings in response")
        }
        var seen = Set<String>()
        return listings.data.compactMap { row in
            guard let wire = row.event, seen.insert(wire.id).inserted else { return nil }
            return MusicEvent(wire: wire)
        }
    }

    // MARK: venue — one venue's detail (what the listings feed doesn't carry)

    private static let venueQuery = """
    query($id: ID!) { venue(id: $id) { id name address location { latitude longitude } area { id name } } }
    """

    /// One venue's address + coordinates, for the Place suggestion a new venue becomes.
    /// Shape verified live 2 Aug: `venue(id:)` answers name/address/location{latitude
    /// longitude}/area{name}; address and location are nullable, so the caller must
    /// work without them. One attempt, loud errors — same contract as the feed.
    func venueDetail(id: String) async throws -> VenueDetailWire? {
        struct Vars: Encodable { let id: String }
        struct Payload: Decodable { let venue: VenueDetailWire? }
        let payload: Payload = try await send(query: Self.venueQuery, variables: Vars(id: id))
        return payload.venue
    }

    // MARK: wire shapes (listings feed — only what the app reads)

    struct AreaWire: Decodable { let id: String; let name: String }
    struct ArtistWire: Decodable { let id: String; let name: String }
    struct ImageWire: Decodable { var filename: String?; var type: String? }
    struct VenueWire: Decodable { let id: String; let name: String; var area: AreaWire? }
    struct EventWire: Decodable {
        let id: String
        let title: String
        var date: String?
        var startTime: String?
        var endTime: String?
        var contentUrl: String?
        var flyerFront: String?
        var isTicketed: Bool?
        var attending: Int?
        var images: [ImageWire]?
        var venue: VenueWire?
        var artists: [ArtistWire]?
    }
    struct ListingRowWire: Decodable { let id: String; let event: EventWire? }
    struct ListingsWire: Decodable { let data: [ListingRowWire]; var totalResults: Int? }
    struct LocationWire: Decodable { let latitude: Double; let longitude: Double }
    struct VenueDetailWire: Decodable {
        let id: String
        let name: String
        var address: String?
        var location: LocationWire?
        var area: AreaWire?
    }

    // MARK: plumbing — one POST, typed decode, GraphQL errors surfaced

    private struct GQLBody<V: Encodable>: Encodable { let query: String; let variables: V }
    private struct GQLError: Decodable { let message: String }
    private struct Envelope<T: Decodable>: Decodable { let data: T?; let errors: [GQLError]? }

    private func send<V: Encodable, T: Decodable>(query: String, variables: V) async throws -> T {
        var req = URLRequest(url: endpoint)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        // Without both of these Cloudflare answers with a challenge page, not JSON.
        req.setValue("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
                     forHTTPHeaderField: "User-Agent")
        req.setValue("https://ra.co/events", forHTTPHeaderField: "Referer")
        do { req.httpBody = try JSONEncoder().encode(GQLBody(query: query, variables: variables)) }
        catch { throw MusicHTTPError.decode("encoding the query failed: \(error)") }

        let data: Data, resp: URLResponse
        do { (data, resp) = try await session.data(for: req) }
        catch { throw MusicHTTPError.transport(error.localizedDescription) }

        guard let http = resp as? HTTPURLResponse else { throw MusicHTTPError.transport("no HTTP response") }
        guard (200..<300).contains(http.statusCode) else { throw MusicHTTPError.badStatus(http.statusCode) }

        let envelope: Envelope<T>
        do { envelope = try JSONDecoder().decode(Envelope<T>.self, from: data) }
        catch { throw MusicHTTPError.decode(String(describing: error)) }
        if let errs = envelope.errors, !errs.isEmpty {
            throw MusicHTTPError.graphql(errs.map(\.message).joined(separator: "; "))
        }
        guard let payload = envelope.data else { throw MusicHTTPError.decode("empty data") }
        return payload
    }
}
