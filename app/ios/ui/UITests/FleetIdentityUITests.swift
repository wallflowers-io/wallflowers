//  FleetIdentityUITests.swift
//  The fleet's identity step, driven through the REAL first-boot screens.
//  Run by scripts/sim-fleet.sh, one instance per
//  simulator, with the persona in the test-runner environment.
//
//  WHAT THIS REPLACES. The three-sim fleet used to be identified through launch
//  ENVIRONMENT VARIABLES — PACIFIC_IDENTITY_NAME, plus PACIFIC_IDENTITY_COLOR and
//  PACIFIC_IDENTITY_NOTE added on 11 Aug — because there was no UI path to a
//  complete profile, so there was nothing for a script to drive. Those two extra
//  variables existed only to work around the gap; the sequence closes it, and the
//  fleet now walks the same screens a user walks. What lands in the card lands
//  through the app's own edit path, so a bug in that path fails the fleet instead
//  of being routed around by it.
//
//  Env (TEST_RUNNER_ → this process):
//    FLEET_NAME   display name minted at the NAME step
//    FLEET_COLOR  "RRGGBB" block colour tapped at the FACE step (optional)
//    FLEET_NOTE   the one-line bio typed at the BIO step (optional)
//    FLEET_CITY   where they are, at the BIO step (optional)

import XCTest

final class FleetIdentityUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"]) ?? false
        }
    }

    func test_identify() throws {
        let env = ProcessInfo.processInfo.environment
        let name = env["FLEET_NAME"] ?? ""
        XCTAssertFalse(name.isEmpty, "FLEET_NAME is required")
        let colour = env["FLEET_COLOR"] ?? ""
        let note = env["FLEET_NOTE"] ?? ""
        let city = env["FLEET_CITY"] ?? ""

        app.launchArguments = ["-pacificResetFirstBoot", "-pacificSeedEvents",
                               "-pacificNoWake", "-pacificNoMusic", "-pacificNoArt"]
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 3)

        // 0 · DOOR. Absent when this device already holds an identity — the fleet
        // uninstalls first (which takes the data container with it), so reaching
        // here without a door means the install did not actually go through.
        let door = app.buttons["boot-door-new"]
        XCTAssertTrue(door.waitForExistence(timeout: 30),
                      "no door: this device already has an identity (uninstall first)")
        door.tap()

        // 1 · NAME — the mint.
        let field = app.textFields["welcome-name"]
        XCTAssertTrue(field.waitForExistence(timeout: 10))
        field.tap()
        field.typeText(name)
        app.buttons["welcome-begin"].tap()

        // 2 · FACE — the persona's block colour, tapped as a swatch. A colour that
        // is not one of the seven is left to the key-derived default rather than
        // silently ignored somewhere else.
        XCTAssertTrue(app.buttons["boot-next"].waitForExistence(timeout: 10))
        if !colour.isEmpty {
            let swatch = app.buttons["boot-face-swatch-\(colour.uppercased())"]
            if swatch.exists { swatch.tap() }
            else { NSLog("FLEET[\(name)] \(colour) is not a swatch — keeping the derived colour") }
        }
        app.buttons["boot-next"].tap()

        // 3 · BIO.
        let line = app.textFields["boot-word"]
        XCTAssertTrue(line.waitForExistence(timeout: 10))
        if !note.isEmpty { line.tap(); line.typeText(note) }
        if !city.isEmpty {
            let c = app.textFields["boot-city"]
            c.tap(); c.typeText(city)
        }
        app.buttons["boot-next"].tap()

        // 4 · RECOVERY KEY (when there is one) and 5 · YOU ARE. Both are read-only
        // — one word each to move on.
        XCTAssertTrue(app.buttons["boot-next"].waitForExistence(timeout: 10)
                      || app.buttons["boot-enter"].waitForExistence(timeout: 1))
        if app.buttons["boot-next"].exists { app.buttons["boot-next"].tap() }
        let enter = app.buttons["boot-enter"]
        XCTAssertTrue(enter.waitForExistence(timeout: 10), "the fingerprint step is the last one")
        enter.tap()

        // Past the 800ms publish debounce, so the card write has certainly landed
        // before the runner tears the app down.
        Thread.sleep(forTimeInterval: 3)
        XCTAssertFalse(app.buttons["boot-door-new"].exists, "the sequence is walked")
    }

    @discardableResult
    private func tapSpringboard(_ labels: [String], waitFor: TimeInterval = 0) -> Bool {
        if waitFor > 0 { Thread.sleep(forTimeInterval: waitFor) }
        for l in labels where springboard.buttons[l].exists {
            springboard.buttons[l].tap()
            return true
        }
        return false
    }
}
