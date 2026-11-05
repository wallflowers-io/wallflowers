//  FirstBootUITests.swift
//  The first-boot sequence, driven the way a user walks it.
//
//  WHY THIS TEST IS THE POINT OF THE FEATURE. The three-sim fleet used to be
//  identified through LAUNCH ENVIRONMENT VARIABLES — PACIFIC_IDENTITY_NAME plus a
//  colour and a note — for one reason: there was no UI path to a complete profile,
//  so there was nothing to drive. This file is the proof that there now is. Every
//  step is reached by a real tap on a real accessibility id, and the assertion is
//  that what those taps collected SURVIVES A RELAUNCH, which can only happen
//  through the core (`set_my_profile` → the published card).
//
//  The suite needs a device with NO identity, so each test launches with
//  `-pacificResetFirstBoot` to prove the sequence from the door. That argument
//  resets ONLY the sequence gate, never the key file — an identity is never
//  destroyed by a test seam.
//
//  AND `-pacificNoPasskey`, WHICH IS A FINDING AS MUCH AS A SEAM. BEGIN fires the
//  passkey ceremony, and `ASAuthorizationController` puts up a REMOTE VIEW owned
//  by another process: full-screen, above this app's window, and unreachable by
//  XCUITest. Measured 15 Sep 2026 — the sequence advanced to FACE and then every
//  synthesised tap landed on the sheet, so the run sat on FACE until it timed out.
//  The flag stops the ceremony so the SEQUENCE can be driven.
//
//  What the flag does NOT do is make the underlying behaviour right. The design
//  claim is "BEGIN mints and advances; the next screen is not gated on a passkey",
//  and that is true of `step` and false of the screen: a real person is shown FACE
//  and then has a system sheet dropped over it before they can touch anything.
//  Whether the ceremony belongs at BEGIN is a product decision, still open.

import XCTest

final class FirstBootUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"]) ?? false
        }
    }

    /// THE WHOLE SEQUENCE, tapped: door → name (the mint) → face → bio → recovery
    /// key → fingerprint → the shell. Then a relaunch, and the profile is still
    /// there — collected by these screens, not by an environment variable.
    func test_theSequenceMintsAnIdentityAndCollectsAProfile() throws {
        try launchAtTheDoor()

        // 0 · DOOR — two words, no explainer.
        let new = app.buttons["boot-door-new"]
        XCTAssertTrue(app.buttons["boot-door-restore"].exists, "and it has two doors, not one")
        new.tap()

        // 1 · NAME — the mint. The keys do not exist until BEGIN.
        let name = "Ada \(Int(Date().timeIntervalSince1970) % 100_000)"
        let field = app.textFields["welcome-name"]
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        field.tap()
        field.typeText(name)
        app.buttons["welcome-begin"].tap()

        // 2 · FACE — a swatch is already selected (derived from the key that was
        // just minted), so tapping straight through still leaves a face. Pick a
        // different one so the assertion below is about a CHOICE.
        let face = app.descendants(matching: .any).matching(identifier: "boot-face").firstMatch
        XCTAssertTrue(face.waitForExistence(timeout: 5),
                      "the face step shows the avatar it is editing")
        app.buttons["boot-face-swatch-33C08C"].tap()
        app.buttons["boot-next"].tap()

        // 3 · BIO — one line and a city. A TEXT field: no location prompt here.
        let line = "Runs events in London"
        let bio = app.textFields["boot-word"]
        XCTAssertTrue(bio.waitForExistence(timeout: 5))
        bio.tap()
        bio.typeText(line)
        let city = app.textFields["boot-city"]
        city.tap()
        city.typeText("London")
        app.buttons["boot-next"].tap()

        // 4 · RECOVERY KEY — generated, shown once. 24 words, so it can never be
        // mistaken for the three PUBLIC fingerprint words on the next screen.
        let key = app.descendants(matching: .any).matching(identifier: "boot-key").firstMatch
        XCTAssertTrue(key.waitForExistence(timeout: 5), "a fresh identity emits a recovery key")
        app.buttons["boot-next"].tap()

        // 5 · YOU ARE — the fingerprint, and the identity key it renders.
        let fingerprint = app.staticTexts["boot-fingerprint"]
        XCTAssertTrue(fingerprint.waitForExistence(timeout: 5))
        XCTAssertNotEqual(fingerprint.label, "—", "the words come from the core, not a placeholder")
        app.buttons["boot-enter"].tap()

        // …and the shell. The sequence is walked; it never asks again.
        XCTAssertTrue(app.otherElements["shell-root"].waitForExistence(timeout: 15)
                      || app.buttons["account-button"].waitForExistence(timeout: 5)
                      || !app.buttons["boot-door-new"].exists,
                      "ENTER lands in the shell")

        // Past the 800ms publish debounce, so the card write has certainly landed.
        Thread.sleep(forTimeInterval: 2.5)
        app.terminate()

        // A PLAIN relaunch (no reset): the sequence must not reappear, and what it
        // collected must come back — which it can only do through the core.
        app.launchArguments = ["-pacificNoPasskey", "-pacificNoWake",
                               "-pacificNoMusic", "-pacificNoArt"]
        app.launch()
        XCTAssertFalse(app.buttons["boot-door-new"].waitForExistence(timeout: 6),
                       "a walked sequence never shows again")
        openProfileScreen("BIO")
        XCTAssertEqual(try noteField().value as? String, line,
                       "the bio collected at first boot came back from the core")
    }

    /// A RUN KILLED AFTER THE MINT RESUMES, AND THE WORDS ARE STILL SHOWN.
    ///
    /// This is the regression test for the worst failure this app has had. The
    /// gate used to read "identity present, complete-key absent ⇒ an identity that
    /// predates the sequence, mark it complete" — true of an old install, and also
    /// true of an identity minted ten seconds ago by a sequence that then stopped.
    /// So a first boot that died anywhere between the mint and the RECOVERY KEY
    /// step came back marked complete, dropped its owner into the shell, and never
    /// showed them the twenty-four words again. It could not: the words are
    /// generated, shown once, and stored by us nowhere. The account's only offline
    /// backup had never been on screen.
    ///
    /// So: mint, kill, relaunch WITHOUT the reset seam, and the sequence must
    /// still be there with the recovery key still ahead of it.
    func test_whenARunIsKilledAfterTheMintItResumesAndStillShowsTheWords() throws {
        // RE-OPEN THE SEQUENCE, and take it from wherever that lands. On a device
        // with no identity that is the DOOR and this mints one; on a device that
        // already holds an identity the reopened sequence resumes at the collecting
        // steps — which is the very state a killed run leaves behind, so both
        // arrivals are the same test from the next line on. Written this way so it
        // does not have to be the suite's first test, and does not eat the
        // no-identity device the other two need.
        app.launchArguments = ["-pacificResetFirstBoot", "-pacificNoPasskey", "-pacificNoWake",
                               "-pacificNoMusic", "-pacificNoArt"]
        app.launch()
        _ = tapSpringboard(["Allow", "Don\u{2019}t Allow", "Don't Allow"], waitFor: 3)

        let door = app.buttons["boot-door-new"]
        let face = app.descendants(matching: .any).matching(identifier: "boot-face").firstMatch
        let deadline = Date().addingTimeInterval(25)
        while !door.exists, !face.exists, Date() < deadline { usleep(300_000) }

        if door.exists {
            door.tap()
            let field = app.textFields["welcome-name"]
            XCTAssertTrue(field.waitForExistence(timeout: 5))
            field.tap()
            field.typeText("Mo \(Int(Date().timeIntervalSince1970) % 100_000)")
            app.buttons["welcome-begin"].tap()
        }

        // The mint landed and the sequence MOVED. It does not wait on a passkey
        // ceremony: the identity is real the instant the core returns, and a
        // ceremony that never calls back must not be able to hold the flow.
        XCTAssertTrue(face.waitForExistence(timeout: 15),
                      "BEGIN mints and advances — the next screen is not gated on a passkey")

        // Killed here: an identity exists, and its owner has seen no words.
        app.terminate()

        // A PLAIN relaunch — no `-pacificResetFirstBoot`. The old migration closed
        // the sequence at exactly this point.
        app.launchArguments = ["-pacificNoPasskey", "-pacificNoWake",
                               "-pacificNoMusic", "-pacificNoArt"]
        app.launch()

        XCTAssertTrue(app.descendants(matching: .any).matching(identifier: "boot-face")
                         .firstMatch.waitForExistence(timeout: 20),
                      "an interrupted first boot RESUMES; it is not silently marked complete")

        // …and walking on from there still reaches the words.
        app.buttons["boot-next"].tap()
        let bio = app.textFields["boot-word"]
        XCTAssertTrue(bio.waitForExistence(timeout: 10))
        app.buttons["boot-next"].tap()

        let key = app.descendants(matching: .any).matching(identifier: "boot-key").firstMatch
        XCTAssertTrue(key.waitForExistence(timeout: 10),
                      "the recovery key is still ahead of them — this is the whole point")

        app.buttons["boot-next"].tap()
        XCTAssertTrue(app.staticTexts["boot-fingerprint"].waitForExistence(timeout: 10))
        app.buttons["boot-enter"].tap()

        // Only NOW is first boot complete. Prove it with one more plain relaunch.
        XCTAssertTrue(app.buttons["account-button"].waitForExistence(timeout: 20),
                      "ENTER lands in the shell")
        app.terminate()
        app.launch()
        XCTAssertFalse(app.buttons["boot-door-new"].waitForExistence(timeout: 6),
                       "a sequence walked to its END never shows again")
        XCTAssertFalse(app.descendants(matching: .any).matching(identifier: "boot-face")
                          .firstMatch.exists,
                       "and it does not resume either — it finished")
    }

    /// The RESTORE door refuses a phrase that is not a valid recovery key, AT THE
    /// FIELD. A mistyped word must never be reinterpreted as a different identity,
    /// so the failure has to be visible before anything is minted.
    func test_restoreRefusesAPhraseThatIsNotARecoveryKey() throws {
        try launchAtTheDoor()
        app.buttons["boot-door-restore"].tap()

        let field = app.textFields["boot-restore-key"]
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        field.tap()
        // 24 real BIP-39 words in an order that fails the checksum — the shape is
        // right and the phrase is wrong, which is exactly the mistyping case.
        field.typeText(Array(repeating: "abandon", count: 24).joined(separator: " "))
        app.buttons["boot-next"].tap()

        // Still on the restore screen: nothing was minted, nothing advanced.
        XCTAssertTrue(app.textFields["boot-restore-key"].exists,
                      "a bad phrase leaves you where you can fix it")
        XCTAssertFalse(app.textFields["welcome-name"].exists,
                       "and never advances into the sequence")
    }

    // MARK: - helpers

    /// Re-open the sequence and land on the DOOR.
    ///
    /// The door is only reachable with NO identity on the device — `-pacificResetFirstBoot`
    /// resets the gate, never the key file, because no test seam should be able to
    /// destroy an identity. On a simulator that already holds one the sequence
    /// resumes at the collecting steps instead, and this SKIPS rather than failing:
    /// the honest fix is to clear the device, not a weaker assertion here.
    ///
    /// AND `simctl uninstall` IS NOT THAT (measured, 15 Sep). It takes the data
    /// container, but the identity outlives it — the app comes back still holding
    /// a key, this helper skips, and a suite run that way reports "succeeded" with
    /// every first-boot test silently skipped. `xcrun simctl erase` is what
    /// actually gives you a device with no identity.
    private func launchAtTheDoor() throws {
        app.launchArguments = ["-pacificResetFirstBoot", "-pacificNoPasskey", "-pacificNoWake",
                               "-pacificNoMusic", "-pacificNoArt"]
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 3)
        guard app.buttons["boot-door-new"].waitForExistence(timeout: 20) else {
            throw XCTSkip("needs a device with no identity — `xcrun simctl erase <udid>` first "
                          + "(uninstall is not enough: the identity survives it)")
        }
    }

    /// The note field on the profile BIO card — the same value the boot step wrote.
    private func noteField() throws -> XCUIElement {
        let f = app.textViews["profile-note"].exists
            ? app.textViews["profile-note"] : app.textFields["profile-note"]
        XCTAssertTrue(f.waitForExistence(timeout: 5), "profile-note not found")
        return f
    }

    /// Open Settings ▸ PROFILE ▸ <card>. The same walk ProfileEditorUITests makes.
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
