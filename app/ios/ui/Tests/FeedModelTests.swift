//  FeedModelTests.swift
//  The Active Scroll Feed's pure logic: union, notch semantics (O8 assumed reading),
//  ordering, and the first-seen ledger the unread band reads.

import XCTest
@testable import WallFlowers
import PacificCore

final class FeedModelTests: XCTestCase {

    private let now: Int64 = 1_754_800_000_000
    private let hour: Int64 = 3_600_000

    private func event(_ id: String, start: Int64, end: Int64 = 0) -> PacificCore.EventRow {
        PacificCore.EventRow(id: id, title: id, descriptor: "", startMs: start, endMs: end,
                             venue: "", venuePlace: "", venuePlaceName: "", venueAt: 0,
                             placed: false, lat: 0, lng: 0,
                             listed: false, priceCents: 0, currency: "", capacity: 0,
                             open: false, sold: 0, overCapacity: false, listing: "",
                             banner: "", bannerMime: "", photoCount: 0, hasClip: false,
                             lineup: [], recurrence: "")
    }

    private func listing(_ title: String, hops: UInt32 = 0, deadline: Int64 = 0) -> ListingRow {
        ListingRow(origin: "o", thingId: title, posture: "wants", title: title,
                   descriptor: "", price: "", deadline: deadline, area: "", hops: hops,
                   via: "", photo: "", photoMime: "")
    }

    private func exterior(_ object: String, kind: String = "project", door: Bool = true) -> ExteriorRow {
        ExteriorRow(object: object, kind: kind, name: object, description: "",
                    owner: "o", via: "v", hops: 1, door: door, requested: false)
    }

    /// #76: an object I am not in is a card that opens to its exterior, when it has a door.
    func testAnExteriorWithADoorIsACardThatOpens() {
        let rows = FeedAssembler.rows(events: [], listings: [],
                                      exteriors: [exterior("plots"), exterior("closed", door: false)], nowMs: now)
        let open = rows.first { $0.title == "plots" }
        XCTAssertEqual(open?.exterior, ExteriorRef(object: "plots"))
        XCTAssertNil(open?.objectID, "not held")
        XCTAssertEqual(open?.category, .projects)
        XCTAssertEqual(open?.provenance, .hops(1))
        XCTAssertNil(rows.first { $0.title == "closed" }?.exterior, "no door, nothing to open")
    }

    /// A Thing's listing and its exterior are one card.
    func testAListingAndItsThingsExteriorAreOneCard() {
        let rows = FeedAssembler.rows(events: [], listings: [listing("amp")],
                                      exteriors: [exterior("amp", kind: "thing")], nowMs: now)
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows.first?.exterior, ExteriorRef(object: "amp"))
    }

    func testUnionDropsPastEventsAndInterleavesStandingListings() {
        let rows = FeedAssembler.rows(
            events: [event("past", start: now - 2 * hour),
                     event("soon", start: now + hour)],
            listings: [listing("amp")],
            nowMs: now)
        XCTAssertEqual(rows.map(\.title), ["amp", "soon"])   // standing rides at now, before the future
        XCTAssertEqual(rows[0].category, .trade)
        XCTAssertEqual(rows[1].category, .events)
    }

    func testProvenanceFollowsHops() {
        let rows = FeedAssembler.rows(events: [], listings: [listing("deck", hops: 2)], nowMs: now)
        XCTAssertEqual(rows[0].provenance, .hops(2))
    }

    func testEmptyLockSetPassesEveryDelta() {
        let rows = FeedAssembler.rows(
            events: [event("soon", start: now + hour)],
            listings: [listing("amp")], nowMs: now)
        XCTAssertEqual(FeedAssembler.filter(rows, locked: []).count, 2)
    }

    func testLockedCategoriesAreTheActiveKinds() {
        let rows = FeedAssembler.rows(
            events: [event("a", start: now + hour), event("b", start: now + 2 * hour)],
            listings: [listing("amp")], nowMs: now)
        // Lock EVENT: only event deltas penetrate (S33).
        XCTAssertEqual(Set(FeedAssembler.filter(rows, locked: [.events]).map(\.title)), ["a", "b"])
        // Lock EVENT + TRADE: both kinds penetrate.
        XCTAssertEqual(FeedAssembler.filter(rows, locked: [.events, .trade]).count, 3)
    }

    func testSeenLedgerBandsOnlyRowsFirstSeenAfterVisit() {
        var ledger = FeedSeenLedger()
        let old = FeedAssembler.rows(events: [event("a", start: now + hour)], listings: [], nowMs: now)
        ledger.observe(old, nowMs: now)
        ledger.visit(nowMs: now + 1)
        let both = FeedAssembler.rows(
            events: [event("a", start: now + hour), event("b", start: now + 2 * hour)],
            listings: [], nowMs: now)
        ledger.observe(both, nowMs: now + 2)
        XCTAssertFalse(ledger.isNew(both.first { $0.title == "a" }!))
        XCTAssertTrue(ledger.isNew(both.first { $0.title == "b" }!))
    }
}

// MARK: - Posts in the feed (18 Aug)

@MainActor
final class FeedPostsTests: XCTestCase {

    private func post(_ id: String, title: String = "T", body: String = "B",
                      retracted: Bool = false, banner: String = "IMG") -> PostRow {
        PostRow(id: id, title: title, body: body, icon: "", banner: banner,
                retracted: retracted, reactions: 0, comments: 0)
    }

    /// A Post reaches the scroll as its own category, so the rail's lock can filter to it.
    func test_postsAssembleUnderTheirOwnCategory() {
        let rows = FeedAssembler.rows(events: [], listings: [], posts: [post("a")], nowMs: 1_000)
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows[0].category, .posts)
        XCTAssertEqual(rows[0].objectID, "a")
        XCTAssertEqual(rows[0].provenance, .member)
    }

    /// A Post has no `when` of its own — it was written, not scheduled — so it rides at
    /// `nowMs` and interleaves with the near future, exactly as a standing listing does.
    /// This is the SC-6 gap showing: ordering stands in for LOG ORDER until rows carry
    /// their delta timestamp.
    func test_aPostRidesAtNowBecauseItHasNoWhenOfItsOwn() {
        let now: Int64 = 42_000
        let rows = FeedAssembler.rows(events: [], listings: [], posts: [post("a")], nowMs: now)
        XCTAssertEqual(rows[0].whenMs, now)
    }

    /// A RETRACTED post still appears, showing the tombstone. Dropping it would make
    /// "taken down" indistinguishable from "never existed" — and the log still holds it.
    func test_aRetractedPostShowsATombstoneRatherThanVanishing() {
        let rows = FeedAssembler.rows(events: [], listings: [],
                                      posts: [post("a", retracted: true)], nowMs: 1)
        XCTAssertEqual(rows.count, 1, "a retraction is a statement, not an erasure")
        XCTAssertEqual(rows[0].title, "Taken down")
        XCTAssertTrue(rows[0].bannerB64.isEmpty, "the media goes with the words")
    }

    /// An untitled post is not a blank row: its body carries the caption.
    func test_anUntitledPostShowsItsBody() {
        let rows = FeedAssembler.rows(events: [], listings: [],
                                      posts: [post("a", title: "", body: "just prose")], nowMs: 1)
        XCTAssertEqual(rows[0].title, "just prose")
    }

    /// The rail's lock filters by kind, so locking POSTS admits posts and nothing else.
    func test_lockingPostsAdmitsOnlyPosts() {
        let rows = FeedAssembler.rows(events: [], listings: [], posts: [post("a")], nowMs: 1)
        XCTAssertEqual(FeedAssembler.filter(rows, locked: [.posts]).count, 1)
        XCTAssertEqual(FeedAssembler.filter(rows, locked: [.events]).count, 0)
    }
}
