//  ArtSystemTests.swift
//  Unit tests for the Art System's deterministic halves: the card scanner (over a
//  fixture cut from the live events page, 2 Aug 26 — the shape the client actually
//  parses), the day/range clock, the wire → value mapping, the EVENTS projection and
//  the reference-page projections. No network anywhere.

import Testing
import Foundation
@testable import WallFlowers

struct ArtSystemTests {

    /// Two cards in the site's published shape: a full one (category, range, venue,
    /// city, image) and a minimal one (single day, venue only, no image, no coords).
    private static let fixture = """
    <div class="m_listing-grid">
    <article class="m_listing-item m_col-item" data-ident="4630824" data-lon="-122.4731325" data-lat="37.763973" data-rank="" data-order="0" data-hours=''>
        <a class="m_listing-link" href="/events/10x" >
            <div class="m_listing-image">
                <picture>
                    <source type="image/webp" srcset="//img.artrabbit.com/events/10x/images/a/350x501/rain-sharp.webp 350w" />
                    <img loading="lazy" width="837" height="1200" src="//img.artrabbit.com/events/10x/images/a/350x501/rain-sharp.jpg" alt="10X">
                </picture>
            </div>
            <div class="m_listing-text">
                <p class="b_categorical-heading">Exhibition</p>
                <p class="b_small-heading mod--primary">10X &amp; Friends&#039; Show</p>
                <p class="b_small-heading mod--colour">02 Jul 2026 &#8211; 05 Sep 2026</p>
                <p class="b_instructional-text mod--large">Tillage</p>
                <p class="b_instructional-text mod--large">San Francisco, United States</p>
            </div>
        </a>
    </article>
    <article class="m_listing-item m_col-item" data-ident="9999001" data-rank="" data-order="1">
        <a class="m_listing-link" href="/events/one-night-only" >
            <div class="m_listing-text">
                <p class="b_small-heading mod--primary">One Night Only</p>
                <p class="b_small-heading mod--colour">02 Aug 2026</p>
                <p class="b_instructional-text mod--large">Small Space</p>
            </div>
        </a>
    </article>
    </div>
    """

    private static func ms(_ day: String) -> Int64 {
        let f = DateFormatter()
        f.calendar = Calendar(identifier: .gregorian)
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = .current
        f.dateFormat = "dd MMM yyyy"
        return Int64(f.date(from: day)!.timeIntervalSince1970 * 1000)
    }

    // MARK: the scanner

    @Test func cardScan_readsEveryFieldOfTheLiveShape() {
        let cards = ArtClient.cards(in: Self.fixture)
        #expect(cards.count == 2)

        let full = cards[0]
        #expect(full.ident == "4630824")
        #expect(full.path == "/events/10x")
        #expect(full.category == "Exhibition")
        #expect(full.title == "10X & Friends' Show")                 // entities decoded
        #expect(full.dateText == "02 Jul 2026 – 05 Sep 2026")
        #expect(full.venueName == "Tillage")
        #expect(full.cityName == "San Francisco, United States")
        #expect(full.lat == 37.763973)
        #expect(full.lon == -122.4731325)
        #expect(full.imagePath == "//img.artrabbit.com/events/10x/images/a/350x501/rain-sharp.jpg")

        let minimal = cards[1]
        #expect(minimal.ident == "9999001")
        #expect(minimal.dateText == "02 Aug 2026")
        #expect(minimal.venueName == "Small Space")                  // a lone line with no comma = the venue
        #expect(minimal.cityName == nil)
        #expect(minimal.imagePath == nil)
    }

    @Test func entityDecode_handlesTheCardsActualEntities() {
        #expect(ArtClient.decode("A &#8211; B") == "A – B")
        #expect(ArtClient.decode("Arts &amp; Crafts") == "Arts & Crafts")
        #expect(ArtClient.decode("Seb&#039;s") == "Seb's")
    }

    // MARK: the clock

    @Test func range_parsesSingleDayAndRangeAndCrossYear() throws {
        let single = try #require(ArtClock.range("02 Aug 2026"))
        #expect(single.startMs == Self.ms("02 Aug 2026"))
        #expect(single.endMs == nil)

        let run = try #require(ArtClock.range("02 Jul 2026 – 05 Sep 2026"))
        #expect(run.startMs == Self.ms("02 Jul 2026"))
        #expect(run.endMs == Self.ms("05 Sep 2026"))

        let winter = try #require(ArtClock.range("12 Dec 2026 – 10 Jan 2027"))
        #expect(winter.endMs == Self.ms("10 Jan 2027"))

        #expect(ArtClock.range("Ongoing") == nil)
    }

    // MARK: wire → value

    @Test func unsequenceableCardIsDropped() {
        let card = ArtClient.CardWire(ident: "1", lat: nil, lon: nil, path: "/events/x",
                                      category: nil, title: "X", dateText: nil,
                                      venueName: nil, cityName: nil, imagePath: nil)
        #expect(ArtEvent(card: card) == nil)
    }

    @Test func protocolRelativeImageBecomesHTTPS() throws {
        let card = ArtClient.CardWire(ident: "2", lat: nil, lon: nil, path: "/events/x",
                                      category: nil, title: "X", dateText: "02 Aug 2026",
                                      venueName: nil, cityName: nil,
                                      imagePath: "//img.artrabbit.com/e/i.jpg")
        let e = try #require(ArtEvent(card: card))
        #expect(e.imageURL?.absoluteString == "https://img.artrabbit.com/e/i.jpg")
        #expect(e.contentUrl?.absoluteString == "https://www.artrabbit.com/events/x")
    }

    // MARK: the EVENTS projection

    @Test func eventVM_carriesRunVenueAndFlyer() throws {
        let e = try #require(ArtEvent(card: ArtClient.cards(in: Self.fixture)[0]))
        let vms = ArtEventsSource.events(from: [e])
        let vm = try #require(vms.first)
        #expect(vm.id == "art:4630824")
        #expect(vm.startMs == Self.ms("02 Jul 2026"))
        #expect(vm.endMs == Self.ms("05 Sep 2026"))
        #expect(vm.participants.count == 1)
        #expect(vm.participants[0].role == .at)
        #expect(vm.participants[0].category == .places)
        #expect(vm.participants[0].name == "Tillage")
        #expect(vm.flyerURL != nil)
    }

    @Test func venuelessListingIsDroppedFromTheFeed() {
        let e = ArtEvent(id: "3", title: "X", startMs: 1, endMs: nil, category: nil,
                         venueName: nil, cityName: nil, venueLat: nil, venueLon: nil,
                         imageURL: nil, contentPath: "/events/x")
        #expect(ArtEventsSource.events(from: [e]).isEmpty)
    }

    // MARK: the event page's schema.org block (the venue-address detail)

    @Test func eventDetail_readsVenueAndPostalAddressFromJSONLD() throws {
        let page = """
        <html><head><script type="application/ld+json">[
        { "@context": "https://schema.org", "@type": "Event",
          "name": "In Bloom | Summer Selection",
          "location": { "@type": "Place", "name": "Maybaum Gallery",
            "address": { "@type": "PostalAddress", "streetAddress": "49 Geary, Ste 416",
                         "addressLocality": "San Francisco", "postalCode": "94108",
                         "addressCountry": "United States" } } }
        ]</script></head><body></body></html>
        """
        let d = try #require(ArtClient.eventDetail(in: page))
        #expect(d.venueName == "Maybaum Gallery")
        #expect(d.streetAddress == "49 Geary, Ste 416")
        #expect(d.locality == "San Francisco")
        #expect(d.postalCode == "94108")

        #expect(ArtClient.eventDetail(in: "<html><body>no block</body></html>") == nil)
    }

    // MARK: the reference-page projections

    @Test func referenceListing_artAlwaysLinksToItsPage() throws {
        let e = try #require(ArtEvent(card: ArtClient.cards(in: Self.fixture)[0]))
        let ref = ReferenceListing(art: e)
        #expect(ref.id == "art:4630824")
        #expect(ref.lineup == nil)
        #expect(ref.venueName == "Tillage")
        #expect(ref.linkLabel == "ON ARTRABBIT")
        #expect(ref.linkURL?.absoluteString == "https://www.artrabbit.com/events/10x")
    }

    @Test func referenceListing_musicLinksOnlyWhenTicketed() {
        let venue = MusicVenue(id: "v", name: "Public Works", areaName: "SF")
        let unticketed = MusicEvent(id: "10", title: "Night", startMs: 1, endMs: nil,
                                    venue: venue, areaName: "SF", artists: [],
                                    flyerURL: nil, contentPath: "/events/10",
                                    isTicketed: false, attending: 0)
        #expect(ReferenceListing(music: unticketed).linkLabel == nil)

        let ticketed = MusicEvent(id: "11", title: "Night", startMs: 1, endMs: nil,
                                  venue: venue, areaName: "SF", artists: [],
                                  flyerURL: nil, contentPath: "/events/11",
                                  isTicketed: true, attending: 0)
        let ref = ReferenceListing(music: ticketed)
        #expect(ref.linkLabel == "TICKETS ON RA")
        #expect(ref.linkURL?.absoluteString == "https://ra.co/events/11")
    }
}
