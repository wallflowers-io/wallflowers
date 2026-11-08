//  MarketSelectorUITests.swift
//  The market's contracts after the 10 Aug rail simplification: the axial flip
//  and its face are GONE — TRADE and JOB are rail jewels; TRADE's maximised
//  state is the typed list of travelled listings (the one card component); and
//  the ListingEditor's two guarded controls survive behind their remaining door
//  (a group's market rail): price only for an ACTIVE posture, audience CLOSED
//  by default.

import XCTest

/// TRADE on the rail — the flip's selectors must not exist on any surface.
final class MarketRailUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"], app: self?.springboard) ?? false
        }
    }

    func test_tradeIsARailJewel_andTheMarketFaceIsGone() {
        launchToLife(app, springboard: springboard, welcomeWalk: true)

        XCTAssertTrue(app.buttons["jewel-trade"].waitForExistence(timeout: 10), "TRADE jewel")
        XCTAssertFalse(app.buttons["jewel-job"].exists, "JOB folded into LISTINGS (11 Aug) — no seat")
        XCTAssertFalse(app.buttons["jewel-inventory"].exists, "no INVENTORY selector anywhere")
        XCTAssertFalse(app.buttons["jewel-skills"].exists, "no SKILLS selector anywhere")
        XCTAssertFalse(app.buttons["jewel-jobs"].exists, "no plural JOBS selector anywhere")

        // TRADE maximises to its typed list (travelled listings; the honest "—"
        // on a device that holds none), and minimises back to the feed.
        app.buttons["jewel-trade"].tap()
        XCTAssertTrue(waitGround(app, "typed-list-trade"), "TRADE maximised = the listings list")
        shoot("trade-typed-list")
        app.buttons["jewel-trade"].tap()
        XCTAssertTrue(waitGround(app, "life-feed"), "second tap restores the feed")
    }

    private func shoot(_ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }
}

/// The Listing editor — its guarded controls, through the surviving door: a
/// group's market rail (TRADE ▸ ＋). Skips honestly on a device with no group.
final class ListingEditorUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"], app: self?.springboard) ?? false
        }
    }

    func test_listingEditor_priceOnlyWhenActive_audienceClosed() throws {
        launchToLife(app, springboard: springboard, welcomeWalk: true)

        // GROUP ▸ the typed list ▸ the first group's page. The card is ONE
        // combined element the OS may type as otherElement OR staticText —
        // query both (the GroupsPageUITests row lesson).
        app.buttons["jewel-groups"].tap()
        XCTAssertTrue(waitGround(app, "typed-list-groups"), "the groups list")
        let rowMatch = NSPredicate(format: "identifier BEGINSWITH 'list-row-group:'")
        var row = app.otherElements.matching(rowMatch).firstMatch
        if !row.waitForExistence(timeout: 6) {
            row = app.staticTexts.matching(rowMatch).firstMatch
        }
        try XCTSkipUnless(row.exists,
                          "no group on this device to open — GroupsPageUITests mints one")
        row.tap()

        // The group's market rail: TRADE ▸ ＋ opens the editor (owner-gated; the
        // QA device's groups are its own mints, so the ＋ is present).
        let tradeRail = app.buttons["group-jewel-trade"]
        XCTAssertTrue(tradeRail.waitForExistence(timeout: 8), "the group's TRADE rail category")
        tradeRail.tap()
        let plus = app.buttons["group-create-new"]
        try XCTSkipUnless(plus.waitForExistence(timeout: 6),
                          "not this group's owner — the authoring ＋ is owner-gated")
        plus.tap()

        XCTAssertTrue(app.navigationBars["New listing"].waitForExistence(timeout: 8), "the editor")

        // AUDIENCE DEFAULTS CLOSED — an over-share cannot be taken back.
        let closed = app.segmentedControls.buttons["My connections"]
        XCTAssertTrue(closed.waitForExistence(timeout: 5), "the audience control")
        XCTAssertTrue(closed.isSelected, "audience must default to My connections")

        // A TRADE listing opens on OFFERS — a STANDING intent, so no price is
        // offered (core refuses a price on a standing posture; the field must
        // not exist rather than exist-and-fail).
        XCTAssertFalse(app.textFields["Price"].exists, "no price on a standing posture")

        // Commit is refused with no title — a listing with no name is not a listing.
        let save = app.buttons["Save listing"]
        XCTAssertTrue(save.waitForExistence(timeout: 5), "the commit disc")
        XCTAssertFalse(save.isEnabled, "no title, no listing")
        let title = app.textFields["Title"]
        title.tap()
        title.typeText("hammer")
        XCTAssertTrue(save.isEnabled, "a named listing can be saved")

        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = "listing-editor"
        shot.lifetime = .keepAlways
        add(shot)
    }
}

// MARK: - Shared helpers (file scope — both classes drive the same shell)

private func launchToLife(_ app: XCUIApplication, springboard: XCUIApplication, welcomeWalk: Bool) {
    app.launchArguments += ["-pacificNoWake"]
    app.launchEnvironment["PACIFIC_IDENTITY_NAME"] = "QA"
    app.launch()
    let allow = springboard.buttons["Allow"]
    if allow.waitForExistence(timeout: 4) { allow.tap() }

    if welcomeWalk {
        let welcome = app.textFields["welcome-name"]
        if welcome.waitForExistence(timeout: 3) {
            welcome.tap(); welcome.typeText("QA")
            app.buttons["welcome-begin"].tap()
        }
    }

    let life = app.buttons["shell-life"]
    _ = life.waitForExistence(timeout: 10)
    if !life.isSelected { life.tap() }
}

private func groundExists(_ app: XCUIApplication, _ id: String) -> Bool {
    app.scrollViews[id].exists || app.otherElements[id].exists
}

@discardableResult
private func waitGround(_ app: XCUIApplication, _ id: String, timeout: TimeInterval = 5) -> Bool {
    let deadline = Date().addingTimeInterval(timeout)
    while Date() < deadline {
        if groundExists(app, id) { return true }
        usleep(300_000)
    }
    return groundExists(app, id)
}

private extension XCTestCase {
    @discardableResult
    func tapSpringboard(_ labels: [String], app: XCUIApplication?) -> Bool {
        guard let app else { return false }
        for label in labels where app.buttons[label].exists {
            app.buttons[label].tap()
            return true
        }
        return false
    }
}
