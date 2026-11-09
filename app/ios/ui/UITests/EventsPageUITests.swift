//  EventsPageUITests.swift
//  The EVENT surface contract after the single-page rework (FEED-TRANSMISSION
//  0-ANSWER-G S27): LIFE lands on the FEED, not on an events page; the EVENT
//  jewel's maximised state is a flat typed list rendered with the one card
//  component; its second tap restores the feed. The old three-lens events page
//  (NEAR · DATES · TICKETS + the search pill) left the rail with the config
//  pages — these tests assert the landing does NOT resurrect it.

import XCTest

final class EventsPageUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"]) ?? false
        }
    }

    func test_lifeLandsOnFeed_notTheEventsPage() {
        launchToLife()

        XCTAssertTrue(waitGround("life-feed", timeout: 10),
                      "LIFE lands on the Active Scroll Feed")
        XCTAssertFalse(app.textFields["Search events, venues"].exists,
                       "the retired events page's search pill must not be the landing")

        // At rest on LIFE the disc row wears its rest identities (the interface
        // matrix's rest row), not a leftover compose context.
        XCTAssertEqual(app.buttons["shell-note"].label, "Note")
        XCTAssertEqual(app.buttons["shell-chat"].label, "Chat")
        XCTAssertEqual(app.buttons["shell-life"].label, "Life")
        XCTAssertTrue(app.buttons["shell-life"].isSelected, "LIFE is the shown screen")
        shoot("life-lands-on-feed")
    }

    func test_eventJewel_maximisesToTypedList_andBack() {
        launchToLife(seeded: true)   // seeded events make the list's rows deterministic

        XCTAssertTrue(waitGround("life-feed", timeout: 10), "the feed ground")
        app.buttons["jewel-events"].tap()
        XCTAssertTrue(waitGround("typed-list-events"),
                      "EVENT maximised is a flat list of Event GroupObjects")
        shoot("events-typed-list")

        // Back to the feed on the second tap — two modes, exactly.
        app.buttons["jewel-events"].tap()
        XCTAssertTrue(waitGround("life-feed"), "the second tap restores the feed")
        XCTAssertFalse(groundExists("typed-list-events"), "the list is gone")
        shoot("events-minimised")
    }

    /// The card's tap contract off the typed list (tap = instance detail) lands on
    /// the event PAGE — now the universal Profile component (T3), with the close
    /// circle floating over it and INVITE among the headline's circles. What this
    /// test owns is the RETURN: close puts you back on the list you came from.
    /// (`EventProfilePageUITests` owns what the page itself renders.)
    ///
    /// The seeded "QA Fixture" row is the guaranteed-future one, so it lists at any
    /// hour. It is addressed by IDENTIFIER, not by its text: TypedListPage combines
    /// each row into one element on purpose, so the inner labels are not queryable
    /// — the old `staticTexts BEGINSWITH "QA Fixture"` lookup silently stopped
    /// matching a tappable thing when that landed.
    func test_eventRow_opensDetailPage_andCloseReturnsToTheList() {
        launchToLife(seeded: true)

        XCTAssertTrue(waitGround("life-feed", timeout: 10), "the feed ground")
        app.buttons["jewel-events"].coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(waitGround("typed-list-events"), "the events typed list")

        let match = NSPredicate(format: "identifier BEGINSWITH 'list-row-event:' AND label CONTAINS %@",
                                "QA Fixture")
        let row = app.buttons.matching(match).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10), "the seeded future event lists")
        // It lists third, below the fold: a coordinate tap needs it on screen.
        let mid = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.6))
        let up = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.25))
        for _ in 0..<10 where row.frame.midY > app.frame.maxY * 0.8 { mid.press(forDuration: 0.05, thenDragTo: up) }
        row.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()

        let close = app.buttons["event-close"]
        XCTAssertTrue(close.waitForExistence(timeout: 10), "the page's float-over close")
        XCTAssertTrue(app.buttons["event-invite"].exists, "INVITE (minted mode)")
        shoot("event-detail-profile-component")

        close.tap()
        XCTAssertTrue(waitGround("typed-list-events"), "close returns to the list")
    }

    // MARK: - Helpers

    private func launchToLife(seeded: Bool = false) {
        app.launchArguments += ["-pacificNoWake"]
        if seeded { app.launchArguments += ["-pacificSeedEvents", "-pacificNoMusic", "-pacificNoArt"] }
        app.launchEnvironment["PACIFIC_IDENTITY_NAME"] = "QA"
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)

        let welcome = app.textFields["welcome-name"]
        if welcome.waitForExistence(timeout: 3) {
            welcome.tap(); welcome.typeText("QA")
            app.buttons["welcome-begin"].tap()
        }

        let life = app.buttons["shell-life"]
        XCTAssertTrue(life.waitForExistence(timeout: 10), "LIFE disc")
        if !life.isSelected { life.tap() }
    }

    private func groundExists(_ id: String) -> Bool {
        app.scrollViews[id].exists || app.otherElements[id].exists
    }

    @discardableResult
    private func waitGround(_ id: String, timeout: TimeInterval = 5) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if groundExists(id) { return true }
            usleep(300_000)
        }
        return groundExists(id)
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
