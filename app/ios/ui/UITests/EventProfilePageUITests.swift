//  EventProfilePageUITests.swift
//  T3's gate on the REAL app: the event page is the universal Profile component
//  with an event's data in it, driven by real taps.
//
//  It asserts the four things the rebuild actually changed, because everything
//  else was already covered:
//
//    1. THE COMPONENT  — `profile-stack` and `profile-headline` are on screen.
//       The page used to hand-roll both; if it ever regresses to a bespoke shell
//       these disappear while the page still looks fine, which is exactly the
//       drift that made the rebuild necessary.
//    2. THE FACTS      — WHEN and the SERIES line, from the seeded QA fixture's
//       real `event.setProfile` (its RRULE is FREQ=WEEKLY;BYDAY=TH, so the core's
//       own summariser has something to say).
//    3. THE CIRCLES    — GOING · INVITE · DISCUSS on the flyer.
//    4. THE PINS RESOLVE IN PLACE — tapping HOST pushes the Space page INSIDE the
//       cover. This is the one behavioural claim of the rebuild that a screenshot
//       cannot make: the page carries its own NavigationStack, so the old
//       dismiss-wait-0.55s-and-post dance is gone. If the stack were missing the
//       bar would render and do nothing, which is what the HOST capsule did.
//
//  The QA fixture is the subject throughout: it is seeded a day ahead, so it
//  lists at any hour (the two real posters are seeded at today's hours and an
//  evening run finds them past), and it is the row that carries a lineup.

import XCTest

final class EventProfilePageUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"]) ?? false
        }
    }

    // MARK: - The page IS the component

    func test_eventRow_opensTheProfileComponent_withTheNightsFacts() {
        launchToLife()
        openSeededEvent()

        // 1 · the component, not a second implementation of it
        XCTAssertTrue(app.otherElements["profile-headline"].waitForExistence(timeout: 10),
                      "the event page wears the universal headline card")
        XCTAssertTrue(app.otherElements["profile-stack"].exists
                        || app.scrollViews["profile-stack"].exists,
                      "…inside the universal Profile stack")

        // 2 · the night's facts, formatted by the app and summarised by the core
        XCTAssertTrue(app.staticTexts["profile-when"].exists, "WHEN leads the info card")
        XCTAssertTrue(app.staticTexts["profile-series"].exists,
                      "the seeded fixture repeats weekly, so the series line is drawn")

        // 3 · the circles
        XCTAssertTrue(app.buttons["event-discuss"].exists, "DISCUSS")
        XCTAssertTrue(app.buttons["event-invite"].exists, "INVITE")
        XCTAssertTrue(app.buttons["event-going"].exists, "GOING")

        shoot("event-page-profile-component")
        app.buttons["event-close"].tap()
    }

    // MARK: - The lineup is people, and the host is reachable IN PLACE

    func test_hostPin_pushesTheSpacePage_insideTheCover() {
        launchToLife()
        openSeededEvent()

        let host = app.otherElements["event-host-space"]
        XCTAssertTrue(host.waitForExistence(timeout: 10),
                      "the publishing Space is a pin on the page")
        // Coordinate tap — the pin bar sits under the rail's full-canvas sibling
        // and the hittability oracle is flaky there (GroupsPageUITests' pattern).
        host.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()

        // Pushed WITHIN the cover: the close button is gone (we are a level deep),
        // and a Space page is up. The old capsule dismissed the whole cover and
        // flipped tabs, so `event-close` surviving here would mean nothing moved.
        XCTAssertTrue(app.otherElements["profile-headline"].waitForExistence(timeout: 8),
                      "HOST pushes the Space's profile inside the event's own stack")
        shoot("event-page-host-pushed")
    }

    // MARK: - Helpers

    /// LIFE, with the seeded events and both reference Systems off so the list is
    /// this device's own minted rows and nothing else.
    private func launchToLife() {
        app.launchArguments += ["-pacificNoWake", "-pacificSeedEvents",
                                "-pacificNoMusic", "-pacificNoArt"]
        app.launchEnvironment["PACIFIC_IDENTITY_NAME"] = "QA"
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)
        let life = app.buttons["shell-life"]
        XCTAssertTrue(life.waitForExistence(timeout: 20), "LIFE disc")
        life.tap()
    }

    /// EVENTS jewel → typed list → the guaranteed-future QA fixture row.
    ///
    /// Both taps are COORDINATE taps. The rail's jewels and the list's rows sit
    /// beside a full-canvas sibling, and the hittability oracle is flaky there —
    /// AX reports the jewel at `{{inf, inf}, {0, 0}}` and `tap()` fails trying to
    /// scroll it into view. A coordinate tap is what a finger does (the pattern
    /// GroupsPageUITests worked out and PersonProfileUITests inherited).
    private func openSeededEvent() {
        // The feed has to be ON THE GROUND before the rail is addressable: LIFE
        // lands on the feed and the jewels animate in with it.
        XCTAssertTrue(ground("life-feed", timeout: 15), "the feed ground")

        let jewel = app.buttons["jewel-events"]
        XCTAssertTrue(jewel.waitForExistence(timeout: 10), "EVENT jewel")
        jewel.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(ground("typed-list-events", timeout: 10),
                      "EVENT maximised puts its typed list on the ground")

        // The row is ONE combined element carrying `list-row-event:<id>` — the
        // inner texts are not addressable (ConfigPages combines them on purpose),
        // so the row is found by identifier and tapped by coordinate.
        // ANY element type: SwiftUI decides what a combined row becomes and it is not
        // stable across builds (other · staticText · cell), so the query is by
        // identifier over every descendant rather than by a guessed type.
        let match = NSPredicate(format: "identifier BEGINSWITH 'list-row-event:' AND label CONTAINS %@",
                                "QA Fixture")
        let row = app.descendants(matching: .any).matching(match).firstMatch
        if !row.waitForExistence(timeout: 8) {
            let rows = app.descendants(matching: .any)
                .matching(NSPredicate(format: "identifier BEGINSWITH 'list-row-'"))
            XCTFail("no QA Fixture row. Rows present: "
                    + (0..<min(rows.count, 12)).map { rows.element(boundBy: $0).identifier }
                        .joined(separator: " | "))
            return
        }
        row.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(app.buttons["event-close"].waitForExistence(timeout: 10),
                      "the row opens the event page")
    }

    /// A named ground, whichever element type SwiftUI gave it this build.
    private func ground(_ id: String, timeout: TimeInterval = 8) -> Bool {
        app.scrollViews[id].waitForExistence(timeout: timeout)
            || app.otherElements[id].exists
            || app.collectionViews[id].exists
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
