//  PersonProfileUITests.swift
//  T1 gate on the REAL app: (1) a person typed-list row (id prefix
//  `list-row-person:`) opens the PERSON page by COORDINATE tap — isHittable is
//  flaky beside the rail's full-canvas sibling (GroupsPageUITests' hard-won
//  pattern), a coordinate tap is what a finger does; (2) a DM's header name is
//  a DOOR to the same page.
//
//  Data is whatever this device truly has: people are never fixtures (the pair
//  flow is the only mint), so a device with no connections SKIPS honestly —
//  the two-sim harness at M1 provides the peers these tests then run against.

import XCTest

final class PersonProfileUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"]) ?? false
        }
    }

    // MARK: - person typed-list row → the page (coordinate tap)

    func test_personRow_coordinateTap_landsOnPersonPage() throws {
        launchToLife()

        // PERSON jewel → the flat typed list (S27).
        let jewel = app.buttons["jewel-people"]
        XCTAssertTrue(jewel.waitForExistence(timeout: 10), "PERSON jewel")
        jewel.tap()
        XCTAssertTrue(app.scrollViews["typed-list-people"].waitForExistence(timeout: 10)
                        || app.otherElements["typed-list-people"].exists,
                      "PERSON maximised puts its typed list on the ground")

        // A person row exists only when this device truly has a connection.
        let rowMatch = NSPredicate(format: "identifier BEGINSWITH 'list-row-person:'")
        var row = app.buttons.matching(rowMatch).firstMatch
        if !row.waitForExistence(timeout: 4) {
            row = app.staticTexts.matching(rowMatch).firstMatch
        }
        guard row.exists else {
            throw XCTSkip("no connections on this device — pair a peer (two-sim harness) first")
        }
        shoot("people-list")

        // COORDINATE tap (GroupsPageUITests:99-104): the row is visibly on the
        // ground, but the rail's sibling makes the hittability oracle flaky.
        row.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()

        XCTAssertTrue(personPageUp(timeout: 8),
                      "the person row lands on the PERSON page (the Profile component)")
        // The person contract's chrome: the headline card with its two circles.
        XCTAssertTrue(app.otherElements["profile-headline"].exists
                        || app.buttons["profile-circle-plus"].exists,
                      "the page wears the person headline")
        shoot("person-page-from-list")
    }

    // MARK: - DM header → the page

    func test_dmHeader_tap_landsOnPersonPage() throws {
        launchToChat()

        // The DM lane needs no control any more: arriving at CHAT lands on CHATS,
        // and CHATS is DMs only. The empty-device skip is the row guard below.

        // Any remaining row IS a DM. The rows live in the List (a collection
        // view under XCUITest) — scoped there so shell chrome never matches.
        let dmRow = app.collectionViews.firstMatch.buttons.firstMatch
        guard dmRow.waitForExistence(timeout: 5) else {
            throw XCTSkip("no DM conversations on this device — pair a peer (two-sim harness) first")
        }
        dmRow.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()

        // The conversation overlay is up; its header name is the person door.
        let door = app.buttons["chat-person-link"]
        XCTAssertTrue(door.waitForExistence(timeout: 8), "the DM header wears the person door")
        shoot("dm-open")
        door.tap()

        XCTAssertTrue(personPageUp(timeout: 8),
                      "the DM header lands on the PERSON page")
        shoot("person-page-from-chat")
    }

    // MARK: - Helpers

    /// The person page's oracle: the page identifier, or the component's
    /// headline (the OS may expose either element first).
    private func personPageUp(timeout: TimeInterval) -> Bool {
        app.otherElements["person-profile"].waitForExistence(timeout: timeout)
            || app.scrollViews["person-profile"].exists
            || app.otherElements["profile-headline"].exists
    }

    private func launch() {
        // No wake banners mid-flow (GroupsPageUITests' note): a banner landing
        // under XCTest's interruption handling can cover the driven surface.
        app.launchArguments += ["-pacificNoWake"]
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)

        // A FRESH install fronts the Welcome gate; walk it the way a person does.
        let welcome = app.textFields["welcome-name"]
        if welcome.waitForExistence(timeout: 4) {
            welcome.tap()
            welcome.typeText("QA")
            app.buttons["welcome-begin"].tap()
        }
    }

    private func launchToLife() {
        launch()
        let life = app.buttons["shell-life"]
        XCTAssertTrue(life.waitForExistence(timeout: 10), "LIFE disc")
        if !life.isSelected { life.tap() }
    }

    private func launchToChat() {
        launch()
        let chat = app.buttons["shell-chat"]
        XCTAssertTrue(chat.waitForExistence(timeout: 10), "CHAT disc")
        if !chat.isSelected { chat.tap() }
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
