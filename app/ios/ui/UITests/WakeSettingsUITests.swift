//  WakeSettingsUITests.swift
//  XCUITest over Settings → WAKE, the surface that answers "why didn't I get a notification?".
//
//  A real push cannot be driven from a UI test — it needs APNs, an auth key, and a device the
//  test does not own. What CAN be driven, and is the entire reason the screen exists, is the
//  STATE REPORT: every precondition for a wake, each one shown separately because they fail
//  separately. A test that only proved the screen opened would prove nothing; these assertions
//  are that each fact is actually rendered and actually resolves to a verdict.

import XCTest

final class WakeSettingsUITests: XCTestCase {

    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
    }

    func test_wakeScreen_reportsEveryPreconditionForBeingWoken() throws {
        let app = XCUIApplication()
        app.launchEnvironment["PACIFIC_IDENTITY_NAME"] = "Wake Check"
        app.launch()
        _ = tapFirstSpringboardButton(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 6)

        let account = app.buttons["account-button"]
        XCTAssertTrue(account.waitForExistence(timeout: 15), "account avatar button should exist")
        account.tap()
        // The avatar opens the account; SETTINGS is behind its gear (as SignOutUITests goes).
        let gear = app.buttons["account-settings"].firstMatch
        XCTAssertTrue(gear.waitForExistence(timeout: 10), "the settings gear should exist")
        gear.tap()

        tapCard(app, "WAKE")
        XCTAssertTrue(app.navigationBars["WAKE"].waitForExistence(timeout: 5),
                      "the WAKE screen should present")

        // The four independent facts. Each is its own row precisely because a token without a
        // registration, or a registration without an APNs key, are different failures.
        for row in ["TOKEN", "TAGS", "WATCHING", "HEALTH"] {
            XCTAssertTrue(app.staticTexts[row].waitForExistence(timeout: 5),
                          "the WAKE screen should report \(row)")
        }

        // The wake tier this device would register with — the value that is otherwise invisible.
        let host = ProcessInfo.processInfo.environment["PACIFIC_WAKER_URL"].flatMap {
            URLComponents(string: $0)?.host
        } ?? "arc.wallflowers.io"
        XCTAssertTrue(app.staticTexts.containing(
            NSPredicate(format: "label CONTAINS %@", host)).firstMatch.waitForExistence(timeout: 5),
            "the WAKE screen should name the waker it registers with (\(host))")

        // HEALTH runs on appear and must resolve to a verdict rather than sitting on
        // "Not checked" — an unreachable waker is an answer, an unresolved probe is not.
        let unresolved = app.staticTexts["Not checked"]
        let resolved = NSPredicate(format: "exists == false")
        expectation(for: resolved, evaluatedWith: unresolved)
        waitForExpectations(timeout: 30)
    }

    // MARK: - Helpers

    /// Settings cards are centred title-only buttons, so their accessibility label is the title
    /// (plus the value, when the row carries one) — match on the prefix rather than equality.
    private func tapCard(_ app: XCUIApplication, _ title: String) {
        let card = app.buttons.containing(
            NSPredicate(format: "label BEGINSWITH %@", title)).firstMatch
        XCTAssertTrue(card.waitForExistence(timeout: 8), "the \(title) card should exist")
        card.tap()
    }

    @discardableResult
    private func tapFirstSpringboardButton(_ labels: [String], waitFor timeout: TimeInterval = 0) -> Bool {
        for label in labels {
            let button = springboard.buttons[label]
            let present = timeout > 0 ? button.waitForExistence(timeout: timeout) : button.exists
            if present { button.tap(); return true }
        }
        return false
    }
}
