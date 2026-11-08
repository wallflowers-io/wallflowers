//  ChatGesturesUITests.swift
//  The long-press contract in chat, both sides, asserted explicitly on a REAL flow —
//  no mock seed, no fixtures (fixtures couple development to fake data):
//   • sign into the real Sysiphus dev DB (localhost:8787) as Nye,
//   • create a real group conversation via the real compose UI,
//   • send real messages,
//  then:
//   • long-pressing an ENRICHED ENTITY span ("ram cargos" → the live Sysiphus design)
//     opens the entity info panel and must NOT open the reaction bar;
//   • long-pressing PLAIN text still opens the reaction bar and must NOT open the panel.
//
//  PRECONDITION: Sysiphus running on http://localhost:8787.

import XCTest

final class ChatGesturesUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    private let threadName = "Stock check"
    private let entityMessage = "can you pull stock on the ram cargos?"
    private let plainMessage = "totally fine, tomorrow works"
    private let entitySpanId = "enriched:sysiphus:DSGN-RAM-CARGO"
    private let entityName = "Ram Cargo Pant"     // the LIVE Sysiphus record's name
    private let reactEmoji = "👍"                  // from Reactions.defaultSet

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"]) ?? false
        }
    }

    func test_entityLongPress_opensPanel_notReactionBar() {
        prepareRealThread()
        send(entityMessage)

        let span = app.descendants(matching: .any)[entitySpanId].firstMatch
        XCTAssertTrue(span.waitForExistence(timeout: 10),
                      "the design mention should resolve against the live Sysiphus catalogue")
        span.press(forDuration: 0.7)

        XCTAssertTrue(app.staticTexts[entityName].waitForExistence(timeout: 5),
                      "the entity info panel should open on an entity long-press")
        XCTAssertFalse(app.buttons[reactEmoji].exists,
                       "the reaction bar must NOT co-fire on an entity long-press")
    }

    func test_plainTextLongPress_opensReactionBar_notPanel() {
        prepareRealThread()
        send(plainMessage)

        let bubble = app.textViews
            .matching(NSPredicate(format: "value BEGINSWITH 'totally fine'"))
            .firstMatch
        XCTAssertTrue(bubble.waitForExistence(timeout: 10))
        bubble.coordinate(withNormalizedOffset: CGVector(dx: 0.10, dy: 0.5))
            .press(forDuration: 0.7)

        XCTAssertTrue(app.buttons[reactEmoji].waitForExistence(timeout: 5),
                      "a plain-text long-press should still open the reaction bar")
        XCTAssertFalse(app.staticTexts[entityName].exists,
                       "no entity panel on a plain-text long-press")
    }

    // MARK: - The real flow (idempotent: signs in / creates / sends only when needed)

    /// Launch → Sysiphus signed in (real credentials) → inside the real group thread.
    private func prepareRealThread() {
        app.launchEnvironment["PACIFIC_IDENTITY_NAME"] = "Nye Thompson"  // the REAL minted identity's display name
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 6)

        ensureSysiphusSignedIn()

        // Open the thread if it exists (a previous test's real residue), else create it
        // through the real compose flow: + → New group → name → Create.
        if app.staticTexts[threadName].waitForExistence(timeout: 3) {
            app.staticTexts[threadName].tap()
        } else {
            app.buttons["New chat"].tap()
            let newGroup = app.buttons["New group"]
            XCTAssertTrue(newGroup.waitForExistence(timeout: 5), "the compose sheet should present")
            newGroup.tap()
            XCTAssertTrue(app.navigationBars["New group"].waitForExistence(timeout: 5),
                          "the New group form should present")
            let name = app.textFields.firstMatch
            XCTAssertTrue(name.waitForExistence(timeout: 5))
            name.tap(); name.typeText(threadName)
            app.buttons["Create"].tap()
        }
        XCTAssertTrue(app.textFields["composer-field"].waitForExistence(timeout: 5),
                      "the thread's composer should be up")
    }

    /// Sign into the real Sysiphus System as Nye (handles an already-signed-in keychain).
    private func ensureSysiphusSignedIn() {
        app.buttons["account-button"].tap()
        app.buttons["sysiphus-signin-row"].firstMatch.tap()
        XCTAssertTrue(app.navigationBars["Systems"].waitForExistence(timeout: 5))
        let user = app.textFields["sysiphus-username"]
        if user.waitForExistence(timeout: 4) {
            user.tap(); user.typeText("nye")
            let pass = app.secureTextFields["sysiphus-password"]
            pass.tap(); pass.typeText("aries26")
            app.buttons["sysiphus-connect"].tap()
        }
        XCTAssertTrue(app.staticTexts["Nye Thompson"].waitForExistence(timeout: 15),
                      "Sysiphus should be connected as Nye Thompson — is it serving on :8787?")
        app.navigationBars["Systems"].buttons["You"].tap()
        app.buttons["Done"].firstMatch.tap()
    }

    /// Send `text` unless a bubble with it already exists (real residue from a prior test).
    /// The send tap can race the keyboard animation — verify the bubble landed, retry once.
    private func send(_ text: String) {
        let bubble = app.textViews.matching(NSPredicate(format: "value == %@", text)).firstMatch
        if bubble.exists { return }
        let composer = app.textFields["composer-field"]
        composer.tap()
        composer.typeText(text)
        app.buttons["composer-send"].tap()
        if !bubble.waitForExistence(timeout: 3) {
            app.buttons["composer-send"].tap()
            XCTAssertTrue(bubble.waitForExistence(timeout: 5), "the message should send")
        }
    }

    // MARK: - Helpers

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
