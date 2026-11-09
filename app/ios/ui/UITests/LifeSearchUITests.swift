//  LifeSearchUITests.swift
//  LIFE's search, on the REAL app with seeded Event GroupObjects (9 Nov):
//   • the feed is the ground and the jewel rail is hidden; the top bar's search opens
//     the field across the whole row, with the rail beneath it;
//   • the text filters the feed and a maximised typed list by the same match;
//   • the PLACE jewel opens THE map;
//   • ✕ and BLUE both close it, back to the unfiltered feed with the rail hidden.

import XCTest

final class LifeSearchUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"]) ?? false
        }
    }

    func test_searchOpensAcrossTheRow_filtersFeedAndList_closeRestores() {
        launchToLife()
        XCTAssertTrue(waitGround("life-feed", timeout: 10), "the feed ground")
        XCTAssertTrue(waitAnyFeedRows(timeout: 8), "the feed carries rows (seeds guarantee it)")
        XCTAssertFalse(app.buttons["jewel-events"].exists, "no rail on the feed")
        shoot("search-closed")

        // Open: the field takes the whole row, the rail comes up beneath it.
        app.openLifeSearch()
        let field = app.textFields["tab-search-field"]
        XCTAssertTrue(field.waitForExistence(timeout: 5), "the field")
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5), "focused as it opens")
        XCTAssertTrue(app.buttons["jewel-events"].waitForExistence(timeout: 5), "the rail")
        XCTAssertFalse(app.buttons["account-button"].exists, "the field replaces the account")
        XCTAssertFalse(app.buttons["tab-notifications"].exists, "and the bell")
        shoot("search-open")

        // A query nothing matches empties the feed; the typed list matches the same way.
        field.typeText("zq no such title zq")
        XCTAssertTrue(app.staticTexts["feed-empty"].waitForExistence(timeout: 5), "the feed filters")
        XCTAssertEqual(anyFeedRows(), 0, "no row survives a query nothing matches")
        app.buttons["jewel-events"].tap()
        XCTAssertTrue(waitGround("typed-list-events"), "EVENT maximised")
        XCTAssertEqual(listRows(), 0, "the typed list filters by the same text")

        // A seeded title matches in the list.
        field.clearAndType("QA Fixture")
        XCTAssertTrue(waitListRows(timeout: 6), "the list keeps what the text matches")
        shoot("search-list-filtered")

        // ✕: text cleared, rail gone, feed back unfiltered, the bar back to | P · S N |.
        app.buttons["tab-search-close"].tap()
        XCTAssertTrue(waitGround("life-feed"), "the feed returns")
        XCTAssertTrue(waitAnyFeedRows(timeout: 8), "unfiltered")
        XCTAssertFalse(app.buttons["jewel-events"].exists, "the rail goes")
        XCTAssertTrue(app.buttons["tab-search"].exists, "search")
        XCTAssertTrue(app.buttons["account-button"].exists, "account")
        XCTAssertTrue(app.buttons["tab-notifications"].exists, "bell")
    }

    func test_placeJewel_opensTheMap_andBlueClosesSearch() {
        launchToLife()
        XCTAssertTrue(waitGround("life-feed", timeout: 10), "the feed ground")

        app.openLifeSearch()
        let jewel = app.buttons["jewel-places"]
        XCTAssertTrue(jewel.waitForExistence(timeout: 5), "PLACE jewel")
        jewel.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(waitAny(["places-map"], timeout: 10), "the map takes the ground")
        shoot("search-map")

        // BLUE is home: search closes with the map.
        app.textFields["tab-search-field"].typeText("anything")
        app.buttons["shell-life"].tap()
        XCTAssertTrue(waitGround("life-feed"), "home to the feed")
        XCTAssertTrue(app.buttons["tab-search"].waitForExistence(timeout: 5), "search closed")
        XCTAssertFalse(app.buttons["jewel-places"].exists, "the rail goes")
        XCTAssertTrue(waitAnyFeedRows(timeout: 8), "and the text with it")
    }

    // MARK: - Helpers

    private func launchToLife() {
        app.launchArguments += ["-pacificSeedEvents", "-pacificNoWake",
                                "-pacificNoMusic", "-pacificNoArt"]
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

    /// Rows by identifier prefix, as FeedUITests counts them: the card is one combined
    /// element the OS may expose as otherElement or staticText.
    private func rows(_ prefix: String) -> Int {
        let match = NSPredicate(format: "identifier BEGINSWITH %@", prefix)
        return app.otherElements.matching(match).count + app.staticTexts.matching(match).count
    }
    private func anyFeedRows() -> Int { rows("feed-row-") }
    private func listRows() -> Int { rows("list-row-") }

    private func waitAnyFeedRows(timeout: TimeInterval) -> Bool { poll(timeout) { self.anyFeedRows() > 0 } }
    private func waitListRows(timeout: TimeInterval) -> Bool { poll(timeout) { self.listRows() > 0 } }

    private func poll(_ timeout: TimeInterval, _ ok: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if ok() { return true }
            usleep(300_000)
        }
        return ok()
    }

    private func groundExists(_ id: String) -> Bool {
        app.scrollViews[id].exists || app.otherElements[id].exists
    }

    @discardableResult
    private func waitGround(_ id: String, timeout: TimeInterval = 5) -> Bool {
        poll(timeout) { self.groundExists(id) }
    }

    /// The map surfaces as a map or an other element, depending on the OS.
    private func waitAny(_ ids: [String], timeout: TimeInterval) -> Bool {
        poll(timeout) { ids.contains { self.app.maps[$0].exists || self.app.otherElements[$0].exists } }
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

private extension XCUIElement {
    /// Delete what the field holds and type anew — it keeps its text across a jewel tap.
    func clearAndType(_ text: String) {
        tap()
        if let current = value as? String, !current.isEmpty {
            typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: current.count))
        }
        typeText(text)
    }
}
