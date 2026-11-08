//  LifeFacesUITests.swift
//  The one-page LIFE surface, asserted on the REAL app (FEED-TRANSMISSION
//  0-ANSWER-G, S27–S33 — the 10 Aug rail simplification):
//   • the rail is SIX SINGULAR jewels — GROUP · EVENT · PLACE · PERSON · TRADE ·
//     JOB — plus the white orb: seven circles, no "MAP" label, no THINGS, no
//     PROJECTS;
//   • the ground state is the Feed; a jewel's tap maximises it to that type's
//     flat list, a second tap minimises back to the Feed (two modes, exactly);
//   • the LIFE disc's second tap is HOME (minimise-all) — the axial market flip
//     is GONE, and none of its selectors exist anywhere.

import XCTest

final class LifeFacesUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"]) ?? false
        }
    }

    func test_railSelectors_sixSingularJewels_andTheOrb() {
        launchToLife()

        // THE FIVE-JEWEL RAIL (11 Aug order): places left the rail, JOB folded
        // into LISTINGS, POSTS took a seat.
        XCTAssertTrue(app.buttons["jewel-groups"].waitForExistence(timeout: 10), "SPACES jewel")
        XCTAssertTrue(app.buttons["jewel-events"].exists, "EVENT jewel")
        XCTAssertTrue(app.buttons["jewel-people"].exists, "PERSON jewel")
        XCTAssertTrue(app.buttons["jewel-trade"].exists, "TRADE jewel — a rail seat, not a flip")
        XCTAssertTrue(app.buttons["jewel-posts"].exists, "POSTS jewel")
        XCTAssertFalse(app.buttons["jewel-places"].exists, "PLACES left the rail (11 Aug)")
        XCTAssertFalse(app.buttons["jewel-job"].exists, "JOB folded into LISTINGS (11 Aug)")
        XCTAssertFalse(app.buttons["jewel-projects"].exists, "PROJECTS stays deferred — no jewel")
        XCTAssertFalse(app.buttons["jewel-things"].exists, "THINGS has no jewel")

        // Titles are SINGULAR (S12/S27). "MAP" is gone — the map is PLACE's
        // maximised state, not a label.
        XCTAssertEqual(app.buttons["jewel-groups"].label, "GROUP")
        XCTAssertEqual(app.buttons["jewel-events"].label, "EVENT")
        XCTAssertEqual(app.buttons["jewel-people"].label, "PERSON")
        XCTAssertEqual(app.buttons["jewel-trade"].label, "TRADE")

        // The orb — the seventh circle, the feed's reset.
        XCTAssertTrue(app.buttons["feed-orb"].exists, "the white orb leads the rail")
        shoot("rail-five-jewels")
    }

    func test_lifeLandsOnFeed_jewelMaximises_secondTapMinimises() {
        launchToLife()

        // The ground state is the FEED — not a category page (S27: two modes only).
        XCTAssertTrue(waitGround("life-feed", timeout: 10),
                      "LIFE lands on the Active Scroll Feed")

        // Tap a jewel: its typed list takes the ground.
        app.buttons["jewel-groups"].tap()
        XCTAssertTrue(waitGround("typed-list-groups"),
                      "a maximised jewel is a flat list of that type")
        XCTAssertFalse(groundExists("life-feed"), "the feed yields the ground")
        shoot("groups-maximised")

        // The SAME jewel's second tap minimises back to the feed.
        app.buttons["jewel-groups"].tap()
        XCTAssertTrue(waitGround("life-feed"),
                      "the second tap on the maximised jewel restores the feed")
        XCTAssertFalse(groundExists("typed-list-groups"), "the list is gone")
        shoot("feed-restored")
    }

    /// The LIFE disc's second tap is HOME — it minimises whatever owns the ground
    /// (10 Aug order; replaces the axial flip, which is gone with its face).
    func test_lifeDiscSecondTap_isHome_andTheMarketFaceIsGone() {
        launchToLife()
        XCTAssertTrue(waitGround("life-feed", timeout: 10), "the feed ground")

        app.buttons["jewel-events"].tap()
        XCTAssertTrue(waitGround("typed-list-events"), "EVENT maximised")

        // Second tap on the LIFE disc: home to the feed, not a flip.
        app.buttons["shell-life"].tap()
        XCTAssertTrue(waitGround("life-feed"),
                      "the LIFE disc's second tap minimises to the feed")

        // The market face never appears — its selectors do not exist on any tap.
        XCTAssertFalse(app.buttons["jewel-inventory"].exists, "INVENTORY died with the flip")
        XCTAssertFalse(app.buttons["jewel-skills"].exists, "SKILLS died with the flip")
        XCTAssertFalse(app.buttons["jewel-jobs"].exists, "the plural JOBS selector died with the flip")
        shoot("home-not-flip")
    }

    // MARK: - Helpers

    /// Land on LIFE. PACIFIC_IDENTITY_NAME (L2) keeps a fresh simulator off the
    /// Welcome gate; the walk below covers a build where the env seam is absent.
    private func launchToLife() {
        app.launchArguments += ["-pacificNoWake"]
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

    /// A ground surface (the feed / a typed list) is a SwiftUI ScrollView whose
    /// identifier the OS may expose as a scrollView OR a plain otherElement —
    /// query both, never a broad `.any` descendants sweep (it snapshots the whole
    /// LIFE hierarchy and can take the runner down).
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
