//  SignOutUITests.swift
//  XCUITest over Settings → SIGN OUT, which is the app's one IRREVERSIBLE control.
//
//  The thing worth testing here is not that a button exists. It is that the button does
//  what it claims — and the claim is total: the identity, the archive and every stored
//  secret go, and there is no server copy to restore from. So the test asserts the two
//  halves that make that safe and true:
//
//    1. IT ASKS FIRST, and Cancel genuinely cancels — a mis-tap on the root Settings
//       screen must not be able to destroy an identity.
//    2. IT ACTUALLY ERASES. The proof is a SECOND LAUNCH with no PACIFIC_IDENTITY_NAME:
//       if the wipe worked the app has nothing to load and lands on the first-boot door.
//       An assertion on the wipe's own log would only prove the code ran; an app that
//       comes back to the door proves the identity is gone.
//
//  The app terminating mid-test is expected, not a failure — see `Wipe` for why signing
//  out has to quit (the Rust core is built once, at init, and holds the files open).

import XCTest

final class SignOutUITests: XCTestCase {

    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
    }

    func test_signOut_asksFirst_thenErasesBackToTheDoor() throws {
        let app = XCUIApplication()
        app.launchEnvironment["PACIFIC_IDENTITY_NAME"] = "Sign Out Check"
        app.launch()
        XCUIDevice.shared.orientation = .portrait
        _ = tapFirstSpringboardButton(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 6)

        openSettings(app)

        // 1. THE ASK, and the way out of it.
        tapCard(app, "SIGN OUT")
        let alert = app.alerts["Sign out of WallFlowers?"]
        XCTAssertTrue(alert.waitForExistence(timeout: 5),
                      "SIGN OUT should present a confirmation, not act on the first tap")
        let cancel = alert.buttons["Cancel"]
        XCTAssertTrue(cancel.exists, "the confirmation must offer a VISIBLE way out")
        cancel.tap()
        XCTAssertFalse(alert.exists, "Cancel should dismiss the confirmation")
        XCTAssertTrue(app.buttons.containing(NSPredicate(format: "label BEGINSWITH %@", "SIGN OUT"))
                          .firstMatch.waitForExistence(timeout: 5),
                      "Cancel should leave the app exactly where it was")

        // 2. THE ERASE. The app is expected to terminate itself.
        tapCard(app, "SIGN OUT")
        XCTAssertTrue(alert.waitForExistence(timeout: 5))
        alert.buttons["Erase this device"].tap()
        let gone = NSPredicate(format: "state == %d", XCUIApplication.State.notRunning.rawValue)
        expectation(for: gone, evaluatedWith: app)
        waitForExpectations(timeout: 30)

        // 3. THE PROOF. No identity name this time: a device that still held an identity
        //    would go straight to the shell, and only an erased one shows the door.
        app.launchEnvironment.removeValue(forKey: "PACIFIC_IDENTITY_NAME")
        app.launch()
        XCTAssertTrue(app.buttons["boot-door-kenjin"].waitForExistence(timeout: 20),
                      "after signing out the app should come back to the first-boot door")
        XCTAssertFalse(app.buttons["account-button"].firstMatch.exists,
                       "the shell must not be reachable after a wipe")
    }

    // MARK: - Helpers

    /// The Settings tree hangs off the avatar (which opens ME) and then the gear in that
    /// header — the avatar alone lands on the profile, not on Settings.
    private func openSettings(_ app: XCUIApplication) {
        let account = app.buttons["account-button"].firstMatch
        XCTAssertTrue(account.waitForExistence(timeout: 20), "account avatar button should exist")
        account.tap()
        let gear = app.buttons["account-settings"].firstMatch
        XCTAssertTrue(gear.waitForExistence(timeout: 10), "the settings gear should exist")
        gear.tap()
        XCTAssertTrue(app.staticTexts["SETTINGS"].waitForExistence(timeout: 10),
                      "the SETTINGS screen should present")
    }

    /// Settings cards are centred title-only buttons, so their accessibility label is the
    /// title (plus the value, when the row carries one) — match on the prefix.
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
