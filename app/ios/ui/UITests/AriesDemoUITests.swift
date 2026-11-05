//  AriesDemoUITests.swift
//  Not a pass/fail test — a SCREENSHOT TOUR for the Aries walkthrough page, driven
//  entirely on the REAL path (no mock seed, no fixtures — fixtures couple development
//  to fake data): mint a real identity → sign a real Sysiphus System in as Nye
//  (Wholesale Lead) → create a real group via the real compose flow → author a real
//  message naming a design ("ram cargos") → it lights up against the LIVE catalogue →
//  press-and-hold opens the design's LIVE Sysiphus record. Attachments export to
//  Pacific/docs/assets/.
//
//  PRECONDITION: Sysiphus running on http://localhost:8787.

import XCTest

final class AriesDemoUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = true          // a tour: keep going, capture what we can
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow", "Open"]) ?? false
        }
    }

    func test_captureAriesWalkthrough() throws {
        app.launchEnvironment["PACIFIC_IDENTITY_NAME"] = "Nye Thompson"  // the real minted identity
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 6)

        // 01 — the real Chats surface (empty on a fresh install, or whatever real
        // conversations exist; nothing is ever seeded).
        XCTAssertTrue(app.buttons["New chat"].waitForExistence(timeout: 15))
        snap("01-chats-real")

        // 02 / 03 — the Sysiphus System. If signed out, sign in with Nye's OWN login and
        // capture the form; if a Keychain session persists, proceed to the connected view.
        app.buttons["account-button"].tap()
        app.buttons["sysiphus-signin-row"].firstMatch.tap()
        XCTAssertTrue(app.navigationBars["Systems"].waitForExistence(timeout: 5))
        let user = app.textFields["sysiphus-username"]
        if user.waitForExistence(timeout: 4) {
            snap("02-sysiphus-signin")
            user.tap(); user.typeText("nye")
            let pass = app.secureTextFields["sysiphus-password"]
            pass.tap(); pass.typeText("aries26")
            app.buttons["sysiphus-connect"].tap()
        }
        XCTAssertTrue(app.staticTexts["Nye Thompson"].waitForExistence(timeout: 15),
                      "Sysiphus connected as Nye Thompson")
        snap("03-sysiphus-connected")

        // Back out to Chats and enter a REAL thread (create via the real compose flow).
        app.navigationBars["Systems"].buttons["You"].tap()
        app.buttons["Done"].firstMatch.tap()
        if app.staticTexts["Wholesale"].waitForExistence(timeout: 3) {
            app.staticTexts["Wholesale"].tap()
        } else {
            app.buttons["New chat"].tap()
            app.buttons["New group"].tap()
            let name = app.textFields.firstMatch
            XCTAssertTrue(name.waitForExistence(timeout: 5))
            name.tap(); name.typeText("Wholesale")
            app.buttons["Create"].tap()
        }

        // 04 — type it the way she'd say it; "ram cargos" lights up (product gold)
        // against the LIVE catalogue. The send tap can race the keyboard animation —
        // verify the bubble landed and retry once if not.
        let message = "can you pull stock on the ram cargos?"
        let bubble = app.textViews.matching(NSPredicate(format: "value == %@", message)).firstMatch
        if !bubble.exists {
            let composer = app.textFields["composer-field"]
            XCTAssertTrue(composer.waitForExistence(timeout: 5))
            composer.tap()
            composer.typeText(message)
            app.buttons["composer-send"].tap()
            if !bubble.waitForExistence(timeout: 3) {
                app.buttons["composer-send"].tap()
                XCTAssertTrue(bubble.waitForExistence(timeout: 5), "the message should send")
            }
        }
        let span = app.descendants(matching: .any)["enriched:sysiphus:DSGN-RAM-CARGO"].firstMatch
        XCTAssertTrue(span.waitForExistence(timeout: 10), "the design name should resolve + light up")
        snap("04-chat-resolved-highlight")

        // 05 — press-and-hold → the Ram Cargo Pant's LIVE Sysiphus record (header + provenance).
        span.press(forDuration: 0.7)
        XCTAssertTrue(app.staticTexts["Ram Cargo Pant"].waitForExistence(timeout: 8),
                      "the live Sysiphus detail should open")
        snap("05-sysiphus-live-detail")

        // 05b — scroll the record down to the live wholesale / stock / open-orders fields
        // (the £210 · 14 avail · Dover Street Market + END order book).
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.80))
        let end   = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.42))
        start.press(forDuration: 0.05, thenDragTo: end)
        _ = app.staticTexts["Open orders"].waitForExistence(timeout: 3)
        snap("05b-sysiphus-live-fields")
    }

    // MARK: - Helpers

    /// Attach a full-screen screenshot under `name` (exported later to docs/assets).
    private func snap(_ name: String) {
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
