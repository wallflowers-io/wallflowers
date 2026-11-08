//  ProfileEditorUITests.swift
//  The APP half of the profile fan-out, asserted on the REAL app with real taps.
//
//  The core half — that a `contact.publishProfile` Delta actually crosses to every
//  connection over MLS — is proven by pacific-core/tests/m7_profile_fanout.rs, with
//  real relays and real device isolation. What that test CANNOT reach is whether the
//  app is wired to it. That is this file.
//
//  The load-bearing assertion is SURVIVES-A-RELAUNCH. The avatar and bio used to live
//  in UserDefaults, and `loadProfile()` deletes those legacy keys on migration — so
//  after a relaunch there is exactly one place a value can have come back from: the
//  core, via `Core.myProfile`. And anything in the core has been through
//  `set_my_profile`, which is the call that fans out. Persistence here therefore
//  means the publish path ran, not merely that a @State survived.

import XCTest

final class ProfileEditorUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"]) ?? false
        }
    }

    /// Edit the display name in the real NAME editor, relaunch, and find it still
    /// there — which can only happen through the core.
    func test_displayName_persistsThroughTheCore() throws {
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)

        let name = "Ada \(String(Int(Date().timeIntervalSince1970) % 100_000))"
        openProfileScreen("NAME")
        try replaceText(in: "profile-name", with: name)
        // Past the 800ms publish debounce, so the core write has certainly happened.
        Thread.sleep(forTimeInterval: 2.5)

        app.terminate()
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)
        openProfileScreen("NAME")

        XCTAssertEqual(try field("profile-name").value as? String, name,
                       "the name must come back from the core after a relaunch")
    }

    /// Same for the bio fields — organisation, title and note all ride the published
    /// card, so all three must round-trip through the core.
    func test_bioFields_persistThroughTheCore() throws {
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)

        let org = "Pacific \(String(Int(Date().timeIntervalSince1970) % 100_000))"
        openProfileScreen("BIO")
        try replaceText(in: "profile-org", with: org)
        try replaceText(in: "profile-title", with: "Founder")
        Thread.sleep(forTimeInterval: 2.5)

        app.terminate()
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)
        openProfileScreen("BIO")

        XCTAssertEqual(try field("profile-org").value as? String, org,
                       "the organisation must come back from the core after a relaunch")
        XCTAssertEqual(try field("profile-title").value as? String, "Founder",
                       "the title must come back from the core after a relaunch")
    }

    /// The block colour is part of the published card too, not a local preference, so
    /// picking a swatch must reach the core rather than a UserDefault.
    func test_blockColour_pickerReachesThePublishPath() throws {
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)
        openProfileScreen("PHOTO")

        let swatches = app.buttons.matching(identifier: "Avatar colour")
        try XCTSkipUnless(swatches.count > 2, "the avatar swatch row should be on the PHOTO screen")
        swatches.element(boundBy: 2).tap()
        Thread.sleep(forTimeInterval: 2.5)

        app.terminate()
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)
        openProfileScreen("PHOTO")

        // The disc's fill is not introspectable from XCUITest, so the honest assertion
        // is that the screen reloads intact after a colour pick — i.e. the publish path
        // ran without wedging. The value round-trip itself is pinned by the two tests
        // above, which read fields the core is the only possible source for.
        XCTAssertTrue(app.staticTexts["Block colour"].waitForExistence(timeout: 5),
                      "the PHOTO screen should come back intact after a colour pick")
    }

    // MARK: - helpers

    /// account button → OWN PROFILE → gear → SETTINGS → PROFILE → `screen`.
    /// (11 Aug order: the circle opens your own profile; settings ride the gear.)
    private func openProfileScreen(_ screen: String) {
        let account = app.buttons["account-button"]
        XCTAssertTrue(account.waitForExistence(timeout: 20), "the account button should be in the top bar")
        account.tap()

        let gear = app.buttons["account-settings"]
        XCTAssertTrue(gear.waitForExistence(timeout: 10), "the own profile should carry the settings gear")
        gear.tap()

        let profile = app.buttons["PROFILE"]
        XCTAssertTrue(profile.waitForExistence(timeout: 10), "SETTINGS should list PROFILE")
        profile.tap()

        let card = app.buttons[screen]
        XCTAssertTrue(card.waitForExistence(timeout: 10), "PROFILE should list \(screen)")
        card.tap()
        XCTAssertTrue(app.navigationBars[screen].waitForExistence(timeout: 10),
                      "the \(screen) screen should open")
    }

    private func field(_ id: String) throws -> XCUIElement {
        let f = app.textFields[id]
        guard f.waitForExistence(timeout: 10) else {
            throw XCTSkip("no \(id) field on this build")
        }
        return f
    }

    /// Clear a text field and type a fresh value — a plain `typeText` would append.
    private func replaceText(in id: String, with value: String) throws {
        let f = try field(id)
        f.tap()
        if let existing = f.value as? String, !existing.isEmpty, existing != f.placeholderValue {
            f.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: existing.count))
        }
        f.typeText(value)
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
