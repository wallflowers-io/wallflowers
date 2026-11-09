//  EventTicketingUITests.swift
//  Slice 3's UI contracts, driven through real taps: the composer mints a real free
//  Event GroupObject and its card wears the FREE capsule; the priced path is honestly
//  GATED (ONBOARDING REQUIRED, the commit disc refused) until a rail onboarding has stored an
//  acct/delegate pair. Delivery (TICKET ✓, the wallet QR, the door verdicts) needs a
//  second device to author the deliver Delta, so it is exercised by the core/harness
//  tests, not here — a single-simulator test that faked it would prove nothing.

import XCTest

final class EventTicketingUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        // These tests assert exact list contents and tap specific rows, so the Music
        // and Art Systems' live reference rows must stay out: with the opt-outs the
        // page behaves exactly as it did before those Systems existed.
        app.launchArguments += ["-pacificNoMusic", "-pacificNoArt"]
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"]) ?? false
        }
    }

    /// LIFE lands on the FEED (the single-page rework, S27-S34) — not on the events
    /// page, which is where this helper used to look for a search pill that is no
    /// longer the landing surface. The EVENTS jewel is what puts the events typed
    /// list on the ground, and that list is what these tests then read.
    private func openEventsPage() {
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)
        let life = app.buttons["shell-life"]
        XCTAssertTrue(life.waitForExistence(timeout: 20), "LIFE disc")
        life.tap()
        XCTAssertTrue(waitGround("life-feed", timeout: 15), "LIFE lands on the feed")
        app.openLifeSearch()
        let jewel = app.buttons["jewel-events"]
        XCTAssertTrue(jewel.waitForExistence(timeout: 10), "EVENT jewel")
        // Coordinate tap: the rail sits beside a full-canvas sibling and the
        // hittability oracle is flaky there (GroupsPageUITests' pattern).
        jewel.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(waitGround("typed-list-events", timeout: 10), "the events typed list")
    }

    @discardableResult
    private func waitGround(_ id: String, timeout: TimeInterval = 5) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        func here() -> Bool {
            app.scrollViews[id].exists || app.otherElements[id].exists
                || app.collectionViews[id].exists
        }
        while Date() < deadline {
            if here() { return true }
            usleep(300_000)
        }
        return here()
    }

    /// One typed-list row, by identifier — the rows are combined elements, so their
    /// inner text is not queryable (TypedListPage combines them on purpose).
    private func eventRow(containing text: String) -> XCUIElement {
        app.buttons.matching(
            NSPredicate(format: "identifier BEGINSWITH 'list-row-event:' AND label CONTAINS %@", text)
        ).firstMatch
    }

    func test_composerCreatesFreeEvent_cardShowsFreeCapsule() {
        openEventsPage()

        // The ＋ beside the search pill opens the composer.
        let compose = app.buttons["events-new"]
        XCTAssertTrue(compose.waitForExistence(timeout: 5), "the events page offers composing")
        compose.tap()
        XCTAssertTrue(app.navigationBars["New event"].waitForExistence(timeout: 10))

        // No title → no event; and no capacity → no listing (a core fold rule the
        // form honours: capacity must be positive).
        XCTAssertFalse(app.buttons["Save Event"].isEnabled)
        scrollToTitle()
        app.textFields["Title"].tap()
        app.typeText("Soundcheck")
        XCTAssertFalse(app.buttons["Save Event"].isEnabled, "capacity is required by the fold")
        app.textFields["Capacity"].tap()
        app.typeText("100")
        XCTAssertTrue(app.buttons["Save Event"].isEnabled, "a free event needs only title + capacity")
        app.buttons["Save Event"].tap()

        // The minted event's card carries the ticket capsule, and free reads FREE.
        let capsule = app.buttons.matching(identifier: "event-ticket-capsule").firstMatch
        XCTAssertTrue(capsule.waitForExistence(timeout: 10), "the card swaps RSVP for the ticket capsule")
        XCTAssertEqual(capsule.label, "FREE")

        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = "event-free-capsule"
        shot.lifetime = .keepAlways
        add(shot)
    }

    func test_composer_pricedIsGatedBehindOnboarding() {
        openEventsPage()
        app.buttons["events-new"].tap()
        XCTAssertTrue(app.navigationBars["New event"].waitForExistence(timeout: 10))

        scrollToTitle()
        app.textFields["Title"].tap()
        app.typeText("Warehouse night")

        // Priced without a stored rail pair: the gate shows and Save refuses — no stub
        // accounts, no pretend checkout.
        app.switches["Free"].switches.firstMatch.tap()
        XCTAssertTrue(app.staticTexts["ONBOARDING REQUIRED"].waitForExistence(timeout: 5),
                      "priced creation is gated until rail onboarding stores acct+delegate")
        app.textFields["Price"].tap()
        app.typeText("12")
        XCTAssertFalse(app.buttons["Save Event"].isEnabled, "a priced event cannot be minted un-onboarded")

        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = "event-priced-gated"
        shot.lifetime = .keepAlways
        add(shot)
    }

    /// The seeded launch: both real events (minted + posters) list, and each one's row
    /// opens its EVENT PAGE with the flyer on it.
    ///
    /// TICKETING IS NO LONGER ASSERTED ON THE PAGE. It is hidden there by ruling
    /// (Ralph, 18 Aug — `EventDetailPage.showsTicketing`), and selling is V2 anyway.
    /// The price/capsule/door assertions are NOT deleted, they are moved behind the
    /// same switch: restore the flag and restore the block below it. What still runs
    /// is the part the ruling did not touch — that both seeded nights list, and that
    /// each row opens its own page.
    func test_seededEvents_bothCards_andDetailPages() {
        app.launchArguments += ["-pacificSeedEvents", "-pacificEventsLens", "dates"]
        openEventsPage()

        let beach = eventRow(containing: "BEACHCRIMES")
        XCTAssertTrue(beach.waitForExistence(timeout: 15), "the seeded priced event lists")
        XCTAssertTrue(eventRow(containing: "UPWARD").exists, "the seeded free event lists")
        shoot("events-seeded-list")

        // BEACHCRIMES — the flyer is the page's headline.
        beach.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(app.images["event-hero"].waitForExistence(timeout: 10), "the banner hero")
        XCTAssertFalse(app.buttons["event-detail-capsule"].exists,
                       "ticketing is hidden on the page (EventDetailPage.showsTicketing)")
        shoot("event-detail-beachcrimes")
        app.buttons["event-close"].tap()

        // UPWARD — the free seeded night, same contract.
        let upward = eventRow(containing: "UPWARD")
        XCTAssertTrue(upward.waitForExistence(timeout: 10))
        upward.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(app.images["event-hero"].waitForExistence(timeout: 10), "the banner hero")
        shoot("event-detail-upward")
        app.buttons["event-close"].tap()
    }

    /// WHAT THE RULING PARKED, kept whole so it can be un-parked in one move.
    ///
    /// Every assertion here passed before ticketing was hidden and is expected to
    /// pass again the moment `EventDetailPage.showsTicketing` is true: the owner's
    /// priced capsule is a VALUE rather than a button, scarcity stays silent at 0
    /// sold, the door is offered, and a free night reads FREE. Deleting them would
    /// mean writing them a second time from the same contract.
    func skipped_test_eventPage_ticketingBlock() {
        app.launchArguments += ["-pacificSeedEvents", "-pacificEventsLens", "dates"]
        openEventsPage()

        eventRow(containing: "BEACHCRIMES")
            .coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(app.staticTexts["$20"].waitForExistence(timeout: 10),
                      "the TicketMoney price line")
        let capsule = app.buttons["event-detail-capsule"]
        XCTAssertEqual(capsule.label, "GET TICKET · $20")
        XCTAssertFalse(capsule.isEnabled, "your own priced event's capsule is a value")
        XCTAssertFalse(app.staticTexts["SOLD OUT"].exists, "150 capacity, 0 sold")
        XCTAssertTrue(app.buttons["event-door"].exists, "the owner's door affordance")
        app.buttons["event-close"].tap()

        eventRow(containing: "UPWARD")
            .coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertEqual(app.buttons["event-detail-capsule"].label, "FREE")
        app.buttons["event-close"].tap()
    }

    // MARK: - Helpers

    /// The composer's form occasionally settles a screen down (it grew a Media section);
    /// the first fields must be IN the lazily-built tree before they can be tapped.
    private func scrollToTitle() {
        let title = app.textFields["Title"]
        for _ in 0..<3 where !title.exists {
            app.swipeDown()
        }
        XCTAssertTrue(title.waitForExistence(timeout: 5), "the composer's Title field")
    }

    private func shoot(_ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    @discardableResult
    private func tapSpringboard(_ labels: [String], waitFor timeout: TimeInterval = 0) -> Bool {
        for label in labels {
            let b = springboard.buttons[label]
            let present = timeout > 0 ? b.waitForExistence(timeout: timeout) : b.exists
            if present { b.tap(); return true }
        }
        return false
    }
}
