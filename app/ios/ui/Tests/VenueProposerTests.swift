//  VenueProposerTests.swift
//  The venue→Place suggestion pipeline's dedup contract, on injected fixtures:
//  a NEW venue proposes exactly one .place mint; a SEEN venue never re-proposes
//  (across proposer instances — the set is persisted); a REJECTED venue stays
//  silent on the next refresh; a venue whose Place is already MINTED never asks
//  at all; coordinates from the detail fetch ride the proposal for the accept
//  arm's location fix; and an undecided sourced proposal survives "relaunch"
//  (a fresh ProposalStore over the same directory).

import XCTest
@testable import WallFlowers

@MainActor
final class VenueProposerTests: XCTestCase {

    private var suite: String!
    private var defaults: UserDefaults!
    private var dir: URL!

    override func setUp() {
        super.setUp()
        suite = "venue-proposer-tests-\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suite)
        dir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
    }

    override func tearDown() {
        UserDefaults().removePersistentDomain(forName: suite)
        try? FileManager.default.removeItem(at: dir)
        super.tearDown()
    }

    // MARK: fixtures

    private func store() -> ProposalStore { ProposalStore(directory: dir) }

    private func proposer(detail: VenueProposer.VenueDetail? = nil) -> VenueProposer {
        VenueProposer(defaults: defaults, detail: { _ in detail })
    }

    private func event(_ id: String, venue: String, venueName: String,
                       title: String = "Basement Set") -> MusicEvent {
        MusicEvent(id: id, title: title,
                   startMs: Int64(Date().timeIntervalSince1970 * 1000),
                   endMs: nil,
                   venue: MusicVenue(id: venue, name: venueName, areaName: "San Francisco"),
                   areaName: "San Francisco", artists: [], flyerURL: nil,
                   contentPath: "/events/\(id)", isTicketed: false, attending: 0)
    }

    // MARK: the gates

    func test_newVenue_proposesOnePlaceMint_dedupedAcrossListings() async {
        let s = store()
        // Two listings at the same venue → ONE question.
        await proposer().sweep(events: [event("e1", venue: "v1", venueName: "Public Works"),
                                        event("e2", venue: "v1", venueName: "Public Works")],
                               mintedPlaces: [], proposals: s)

        XCTAssertEqual(s.pending.count, 1)
        let p = s.pending[0]
        XCTAssertEqual(p.id, "ra-venue:v1")
        XCTAssertEqual(p.kind, .mint)
        XCTAssertEqual(p.subject.type, .place)
        XCTAssertEqual(p.subject.name, "Public Works")
        XCTAssertEqual(p.subject.summary, "RA venue · San Francisco",
                       "the summary IS the descriptor the accept mints with")
        XCTAssertEqual(p.provenance.source, .music)
    }

    func test_seenVenue_neverReproposes_evenInAFreshInstance() async {
        let s = store()
        let feed = [event("e1", venue: "v1", venueName: "Public Works")]
        await proposer().sweep(events: feed, mintedPlaces: [], proposals: s)
        XCTAssertEqual(s.pending.count, 1)

        // The same refresh again, and a NEW proposer over the same persisted set (a
        // relaunch): no second question about a venue already asked about.
        await proposer().sweep(events: feed, mintedPlaces: [], proposals: s)
        XCTAssertEqual(s.pending.count, 1, "a seen venue must not propose twice")
    }

    func test_rejectedVenue_staysSilentOnTheNextRefresh() async {
        let s = store()
        let feed = [event("e1", venue: "v1", venueName: "F8")]
        await proposer().sweep(events: feed, mintedPlaces: [], proposals: s)
        XCTAssertEqual(s.pending.count, 1)

        s.decide(s.pending[0], .reject, core: CoreService())
        XCTAssertTrue(s.pending.isEmpty)

        // The next refresh lists the venue again — the seen-set keeps it silent.
        await proposer().sweep(events: feed, mintedPlaces: [], proposals: s)
        XCTAssertTrue(s.pending.isEmpty, "a rejected venue must not re-propose")

        // And a rebuild (every graph change/tray-open does one) must not resurrect it.
        s.rebuild(resolved: [], mintedNames: [])
        XCTAssertTrue(s.pending.isEmpty)

        // Nor does a relaunch bring it back: the decided record retired durably.
        XCTAssertTrue(store().pending.isEmpty)
    }

    func test_mintedPlace_neverProposes() async {
        let s = store()
        await proposer().sweep(events: [event("e1", venue: "v1", venueName: "  Public Works ")],
                               mintedPlaces: ["public works"], proposals: s)
        XCTAssertTrue(s.pending.isEmpty,
                      "a venue whose Place exists (normalized-name match) never proposes")

        // And it entered the seen-set — the question is answered for good.
        await proposer().sweep(events: [event("e2", venue: "v1", venueName: "Public Works")],
                               mintedPlaces: [], proposals: s)
        XCTAssertTrue(s.pending.isEmpty)
    }

    func test_detailCoordinates_rideTheProposal() async {
        let s = store()
        let d = VenueProposer.VenueDetail(address: "161 Erie St, San Francisco",
                                          lat: 37.7526, lng: -122.4195)
        await proposer(detail: d).sweep(events: [event("e1", venue: "v1", venueName: "Public Works")],
                                        mintedPlaces: [], proposals: s)
        let p = s.pending[0]
        XCTAssertEqual(p.features["lat"], 37.7526)
        XCTAssertEqual(p.features["lng"], -122.4195)
        XCTAssertEqual(p.subject.snippet, "161 Erie St, San Francisco")
    }

    func test_noDetail_stillProposes_withoutCoordinates() async {
        let s = store()
        await proposer().sweep(events: [event("e1", venue: "v1", venueName: "F8",
                                              title: "Warehouse night")],
                               mintedPlaces: [], proposals: s)
        let p = s.pending[0]
        XCTAssertNil(p.features["lat"])
        XCTAssertNil(p.features["lng"])
        XCTAssertEqual(p.subject.snippet, "Hosts Warehouse night",
                       "no address ⇒ the hosting event is the card's context")
    }

    func test_overflowBeyondBannerCap_stillLandsInTheDeck() async {
        let s = store()
        let feed = (1...12).map { event("e\($0)", venue: "v\($0)", venueName: "Venue \($0)") }
        await proposer().sweep(events: feed, mintedPlaces: [], proposals: s)
        XCTAssertEqual(s.pending.count, 12,
                       "the banner cap limits the lock screen, never the deck")
    }

    func test_undecidedSourcedProposal_survivesRelaunch() async {
        let s = store()
        await proposer().sweep(events: [event("e1", venue: "v1", venueName: "Public Works")],
                               mintedPlaces: [], proposals: s)
        XCTAssertEqual(s.pending.count, 1)

        // A fresh store over the same directory = a relaunch: the open question is
        // still on the deck (contrast the graph's derived proposals, which recompute).
        let relaunched = store()
        XCTAssertEqual(relaunched.pending.count, 1)
        XCTAssertEqual(relaunched.pending[0].id, "ra-venue:v1")

        // Once the Place exists (minted anywhere), the next rebuild retires it.
        relaunched.rebuild(resolved: [], mintedNames: ["public works"])
        XCTAssertTrue(relaunched.pending.isEmpty)
        XCTAssertTrue(store().pending.isEmpty, "retirement is durable")
    }

    // MARK: — the Art System's sweep (the address gate is the point)

    private func artProposer(page: ((String) async -> ArtClient.EventDetailWire?)? = nil) -> VenueProposer {
        VenueProposer(defaults: defaults, detail: { _ in nil }, artDetail: page ?? { _ in nil })
    }

    private func artEvent(_ id: String, venue: String?, title: String = "Group Show",
                          lat: Double? = nil, lon: Double? = nil) -> ArtEvent {
        ArtEvent(id: id, title: title,
                 startMs: Int64(Date().timeIntervalSince1970 * 1000), endMs: nil,
                 category: "Exhibition", venueName: venue,
                 cityName: "San Francisco, United States",
                 venueLat: lat, venueLon: lon, imageURL: nil, contentPath: "/events/\(id)")
    }

    func test_artVenueWithAddress_proposes_addressAsTheDescriptor() async {
        let s = store()
        let page = ArtClient.EventDetailWire(venueName: "Maybaum Gallery",
                                             streetAddress: "49 Geary, Ste 416",
                                             locality: "San Francisco", postalCode: "94108")
        // Two listings at the same venue → ONE question, coordinates from the card.
        await artProposer(page: { _ in page })
            .sweepArt(events: [artEvent("a1", venue: "Maybaum Gallery", title: "In Bloom",
                                        lat: 37.7877, lon: -122.4034),
                               artEvent("a2", venue: "Maybaum Gallery")],
                      mintedPlaces: [], proposals: s)
        XCTAssertEqual(s.pending.count, 1)
        let p = s.pending[0]
        XCTAssertEqual(p.id, "art-venue:maybaum gallery")
        XCTAssertEqual(p.kind, .mint)
        XCTAssertEqual(p.subject.type, .place)
        XCTAssertEqual(p.subject.name, "Maybaum Gallery")
        XCTAssertEqual(p.subject.summary, "49 Geary, Ste 416, San Francisco",
                       "the ACTUAL address is the descriptor the accept mints with")
        XCTAssertEqual(p.subject.snippet, "Hosts In Bloom")
        XCTAssertEqual(p.provenance.source, .art)
        XCTAssertEqual(p.features["lat"], 37.7877)
        XCTAssertEqual(p.features["lng"], -122.4034)
    }

    func test_artVenueWithoutStreetAddress_neverProposes_cityIsInsufficient() async {
        let s = store()
        let cityOnly = ArtClient.EventDetailWire(venueName: "Pop-Up Space",
                                                 streetAddress: nil,
                                                 locality: "San Francisco", postalCode: nil)
        await artProposer(page: { _ in cityOnly })
            .sweepArt(events: [artEvent("a1", venue: "Pop-Up Space")],
                      mintedPlaces: [], proposals: s)
        XCTAssertTrue(s.pending.isEmpty, "no street address ⇒ no Place proposal")

        // The page WAS read and answered — asked is asked, even with a full page later.
        let full = ArtClient.EventDetailWire(venueName: "Pop-Up Space",
                                            streetAddress: "1 Market St",
                                            locality: "San Francisco", postalCode: nil)
        await artProposer(page: { _ in full })
            .sweepArt(events: [artEvent("a2", venue: "Pop-Up Space")],
                      mintedPlaces: [], proposals: s)
        XCTAssertTrue(s.pending.isEmpty, "an address-less venue must not re-ask")
    }

    func test_artPageFetchFailure_retriesOnTheNextRefresh() async {
        let s = store()
        let feed = [artEvent("a1", venue: "Tillage")]
        await artProposer(page: { _ in nil }).sweepArt(events: feed, mintedPlaces: [], proposals: s)
        XCTAssertTrue(s.pending.isEmpty)

        // The fetch failing is NOT an answer — the venue is not seen, so the next
        // refresh asks the page again and the proposal lands.
        let page = ArtClient.EventDetailWire(venueName: "Tillage",
                                             streetAddress: "1275 Minnesota St",
                                             locality: "San Francisco", postalCode: nil)
        await artProposer(page: { _ in page }).sweepArt(events: feed, mintedPlaces: [], proposals: s)
        XCTAssertEqual(s.pending.count, 1)
        XCTAssertEqual(s.pending[0].subject.summary, "1275 Minnesota St, San Francisco")
    }

    func test_venuelessCard_takesItsVenueFromThePage() async {
        let s = store()
        let page = ArtClient.EventDetailWire(venueName: "Maybaum Gallery Satellite Space",
                                             streetAddress: "48 Stockton St",
                                             locality: "San Francisco", postalCode: nil)
        // The card names no venue (the show IS the venue's own satellite) — the page does.
        await artProposer(page: { _ in page })
            .sweepArt(events: [artEvent("a1", venue: nil, title: "Satellite Opening")],
                      mintedPlaces: [], proposals: s)
        XCTAssertEqual(s.pending.count, 1)
        XCTAssertEqual(s.pending[0].subject.name, "Maybaum Gallery Satellite Space")
        XCTAssertEqual(s.pending[0].subject.summary, "48 Stockton St, San Francisco")

        // A later VENUED card for the same gallery: asked is asked, under either key.
        await artProposer(page: { _ in page })
            .sweepArt(events: [artEvent("a2", venue: "Maybaum Gallery Satellite Space")],
                      mintedPlaces: [], proposals: s)
        XCTAssertEqual(s.pending.count, 1, "the page's venue name entered the seen-set")
    }

    func test_artMintedPlace_neverProposes() async {
        let s = store()
        let page = ArtClient.EventDetailWire(venueName: "Tillage",
                                             streetAddress: "1275 Minnesota St",
                                             locality: "San Francisco", postalCode: nil)
        await artProposer(page: { _ in page })
            .sweepArt(events: [artEvent("a1", venue: " Tillage ")],
                      mintedPlaces: ["tillage"], proposals: s)
        XCTAssertTrue(s.pending.isEmpty,
                      "a venue whose Place exists (normalized-name match) never proposes")
    }
}
