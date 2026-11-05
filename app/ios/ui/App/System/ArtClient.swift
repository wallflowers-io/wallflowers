//  ArtClient.swift
//  The HTTP client for the Art System — ArtRabbit's public listings (GET
//  https://www.artrabbit.com/events). Undocumented but open: public reads need no auth,
//  and the site is server-rendered — the listings arrive as HTML cards, so this client
//  is a scanner over one stable card shape rather than a JSON decoder. Two gotchas,
//  both verified live (2 Aug 26): every request carries a real browser User-Agent and
//  a Referer, and the first request answers with a cookie challenge (302 → ?__cc=1)
//  that URLSession's default redirect + cookie handling absorbs transparently.
//
//  Scoping is by COORDINATES (+ display names), exactly as the site's own search form
//  submits them; `location_name` alone is ignored by the server. One attempt per call,
//  one page of 20, loud errors — the MusicClient shape over an HTML wire.

import Foundation

enum ArtHTTPError: Error, LocalizedError {
    case badStatus(Int)
    case transport(String)
    case parse(String)

    var errorDescription: String? {
        switch self {
        case .badStatus(let s): return "ArtRabbit returned HTTP \(s)"
        case .transport(let m): return "Couldn’t reach ArtRabbit: \(m)"
        case .parse(let m):     return "ArtRabbit response wasn’t understood: \(m)"
        }
    }
}

struct ArtClient {
    var endpoint: URL = URL(string: "https://www.artrabbit.com/events")!
    private let session: URLSession = .shared

    // MARK: eventListings — the scope's feed

    /// One page of the place's current listings ("just-opened": what's showing now,
    /// freshest first — the site's own default lens), flattened to ArtEvents.
    func eventListings(place: ArtPlace, page: Int = 1) async throws -> [ArtEvent] {
        var comps = URLComponents(url: endpoint, resolvingAgainstBaseURL: false)!
        comps.queryItems = [
            URLQueryItem(name: "type", value: "All listings"),
            URLQueryItem(name: "view", value: "just-opened"),
            URLQueryItem(name: "location_name", value: place.city),
            URLQueryItem(name: "lat", value: String(place.lat)),
            URLQueryItem(name: "lon", value: String(place.lon)),
            URLQueryItem(name: "city", value: place.city),
            URLQueryItem(name: "country", value: place.country),
            URLQueryItem(name: "sub", value: "Go"),
            URLQueryItem(name: "page", value: String(page)),
        ]
        guard let url = comps.url else { throw ArtHTTPError.parse("bad query") }

        var req = URLRequest(url: url)
        // Without both of these the edge answers with a challenge page, not the site.
        req.setValue("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
                     forHTTPHeaderField: "User-Agent")
        req.setValue("https://www.artrabbit.com/events", forHTTPHeaderField: "Referer")

        let data: Data, resp: URLResponse
        do { (data, resp) = try await session.data(for: req) }
        catch { throw ArtHTTPError.transport(error.localizedDescription) }

        guard let http = resp as? HTTPURLResponse else { throw ArtHTTPError.transport("no HTTP response") }
        guard (200..<300).contains(http.statusCode) else { throw ArtHTTPError.badStatus(http.statusCode) }
        guard let html = String(data: data, encoding: .utf8) else { throw ArtHTTPError.parse("not UTF-8") }

        let cards = Self.cards(in: html)
        // Zero cards is a real answer only on a page we recognize as the events page —
        // a quiet area lists nothing. Anything else (a challenge page, a redesign) is
        // a fault and fails LOUD rather than pretending the world is empty.
        guard !cards.isEmpty || html.contains("m_sentence-search") else {
            throw ArtHTTPError.parse("no listings and no events page in response")
        }
        var seen = Set<String>()
        return cards.compactMap { card in
            guard seen.insert(card.ident).inserted else { return nil }
            return ArtEvent(card: card)
        }
    }

    // MARK: eventDetail — the venue's ACTUAL address, for the Place-mint path

    /// What one event page's schema.org Event block answers: the venue's name and its
    /// postal address. A card names a venue; only the page carries the street.
    struct EventDetailWire {
        var venueName: String?
        var streetAddress: String?
        var locality: String?
        var postalCode: String?
    }

    /// One event page, read for its JSON-LD Event → location. Best-effort at the
    /// caller (the venue proposer), loud here — the MusicClient.venueDetail shape.
    func eventDetail(path: String) async throws -> EventDetailWire {
        guard let url = URL(string: "https://www.artrabbit.com" + path) else {
            throw ArtHTTPError.parse("bad event path \(path)")
        }
        var req = URLRequest(url: url)
        req.setValue("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
                     forHTTPHeaderField: "User-Agent")
        req.setValue("https://www.artrabbit.com/events", forHTTPHeaderField: "Referer")

        let data: Data, resp: URLResponse
        do { (data, resp) = try await session.data(for: req) }
        catch { throw ArtHTTPError.transport(error.localizedDescription) }
        guard let http = resp as? HTTPURLResponse else { throw ArtHTTPError.transport("no HTTP response") }
        guard (200..<300).contains(http.statusCode) else { throw ArtHTTPError.badStatus(http.statusCode) }
        guard let html = String(data: data, encoding: .utf8) else { throw ArtHTTPError.parse("not UTF-8") }
        // A page that ANSWERS but carries no Event schema (permanent public-art
        // listings do) is an empty detail, not an error — the sweep marks it seen
        // and moves on, where a thrown transport fault would retry forever.
        return Self.eventDetail(in: html) ?? EventDetailWire()
    }

    /// The page's `application/ld+json` Event → location name + PostalAddress. Pure,
    /// testable on a captured page.
    static func eventDetail(in html: String) -> EventDetailWire? {
        for block in all(#"<script type="application/ld\+json">(.*?)</script>"#, in: html,
                         options: [.dotMatchesLineSeparators]) {
            guard let data = block.data(using: .utf8),
                  let json = try? JSONSerialization.jsonObject(with: data) else { continue }
            // The block is one Event or an array holding one.
            let dicts: [[String: Any]]
            if let d = json as? [String: Any] { dicts = [d] }
            else if let a = json as? [[String: Any]] { dicts = a }
            else { continue }
            for d in dicts where (d["@type"] as? String) == "Event" {
                guard let location = d["location"] as? [String: Any] else { continue }
                let address = location["address"] as? [String: Any]
                return EventDetailWire(
                    venueName: (location["name"] as? String).flatMap(nonEmpty),
                    streetAddress: (address?["streetAddress"] as? String).flatMap(nonEmpty),
                    locality: (address?["addressLocality"] as? String).flatMap(nonEmpty),
                    postalCode: (address?["postalCode"] as? String).flatMap(nonEmpty)
                )
            }
        }
        return nil
    }

    private static func nonEmpty(_ s: String) -> String? {
        let t = s.trimmingCharacters(in: .whitespacesAndNewlines)
        return t.isEmpty ? nil : t
    }

    // MARK: wire shape (one server-rendered listing card — only what the app reads)

    struct CardWire {
        let ident: String
        var lat: Double?
        var lon: Double?
        let path: String              // "/events/<slug>"
        var category: String?         // "Exhibition" / "Talk" / …
        let title: String
        var dateText: String?         // "02 Jul 2026 – 05 Sep 2026" | "02 Aug 2026"
        var venueName: String?
        var cityName: String?         // "San Francisco, United States"
        var imagePath: String?        // "//img.artrabbit.com/…"
    }

    // MARK: the scanner — stable card markup → CardWire, no HTML library

    /// Every `<article class="m_listing-item" …>…</article>` block on the page, read by
    /// the card's own class names. The shape is the site's published one; a card missing
    /// its ident, link or title is skipped, never guessed at.
    static func cards(in html: String) -> [CardWire] {
        blocks(of: html).compactMap { block in
            guard let ident = first(#"data-ident="(\d+)""#, in: block),
                  let path = first(##"href="(/events/[^"#]+)""##, in: block),  // TEMP(venue session): `"#` inside [^"#] ended the #"…"# literal early — ## delimiters only, regex unchanged
                  let title = first(#"b_small-heading mod--primary">([^<]+)<"#, in: block)
                      .map(decode) else { return nil }
            // The card's two instructional lines are venue then "City, Country"; either
            // can be absent, and the comma is what tells a lone survivor apart.
            let lines = all(#"b_instructional-text mod--large">([^<]+)</p>"#, in: block).map(decode)
            var venue: String?, city: String?
            if lines.count >= 2 { venue = lines[0]; city = lines[1] }
            else if let only = lines.first { if only.contains(", ") { city = only } else { venue = only } }

            return CardWire(
                ident: ident,
                lat: first(#"data-lat="(-?[\d.]+)""#, in: block).flatMap(Double.init),
                lon: first(#"data-lon="(-?[\d.]+)""#, in: block).flatMap(Double.init),
                path: path,
                category: first(#"b_categorical-heading">([^<]+)<"#, in: block).map(decode),
                title: title,
                dateText: first(#"b_small-heading mod--colour">([^<]+)<"#, in: block).map(decode),
                venueName: venue,
                cityName: city,
                imagePath: first(#"<img[^>]+src="(//img\.artrabbit\.com/[^"]+)""#, in: block)
            )
        }
    }

    private static func blocks(of html: String) -> [String] {
        var out: [String] = []
        var cursor = html.startIndex
        while let open = html.range(of: "<article class=\"m_listing-item", range: cursor..<html.endIndex) {
            guard let close = html.range(of: "</article>", range: open.lowerBound..<html.endIndex) else { break }
            out.append(String(html[open.lowerBound..<close.upperBound]))
            cursor = close.upperBound
        }
        return out
    }

    private static func first(_ pattern: String, in text: String) -> String? {
        all(pattern, in: text).first
    }

    private static func all(_ pattern: String, in text: String,
                            options: NSRegularExpression.Options = []) -> [String] {
        guard let re = try? NSRegularExpression(pattern: pattern, options: options) else { return [] }
        let range = NSRange(text.startIndex..., in: text)
        return re.matches(in: text, range: range).compactMap { m in
            Range(m.range(at: 1), in: text).map { String(text[$0]) }
        }
    }

    /// The handful of entities the cards actually carry (&#8211; between range halves,
    /// &amp;/&#039; in titles), plus generic numeric forms — not a general HTML decoder.
    static func decode(_ s: String) -> String {
        var out = s
        for (entity, char) in [("&amp;", "&"), ("&quot;", "\""), ("&#039;", "'"), ("&nbsp;", " ")] {
            out = out.replacingOccurrences(of: entity, with: char)
        }
        while let m = try? NSRegularExpression(pattern: #"&#(\d+);"#)
            .firstMatch(in: out, range: NSRange(out.startIndex..., in: out)),
              let whole = Range(m.range, in: out), let digits = Range(m.range(at: 1), in: out),
              let code = UInt32(out[digits]), let scalar = Unicode.Scalar(code) {
            out = out.replacingCharacters(in: whole, with: String(Character(scalar)))
        }
        return out.trimmingCharacters(in: .whitespaces)
    }
}
