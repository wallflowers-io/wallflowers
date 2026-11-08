//  VenueProposalUITests.swift
//  The venue→Place journey on the REAL app: a seeded venue proposal (the
//  -pacificSeedVenueProposal seam drives the REAL sweep — seen-set, minted gate,
//  store injection) shows on the Convergence deck, and the ✓ button (the swipe's
//  mirror — raw swipes are TCC-blocked under XCUITest) MINTS the Place GroupObject.
//  The mint is verified where a Place is READ: the event composer's venue picker
//  lists every held Place (core.places) by name.
//
//  The seeded name is unique per run because the app container persists across
//  runs — a fixed name would already be minted on the second run and the dedup
//  gate (correctly) would never propose it again.

import XCTest

final class VenueProposalUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"]) ?? false
        }
    }

    func test_seededVenueProposal_acceptOnDeck_mintsPlace() {
        let name = "Venue \(Int(Date().timeIntervalSince1970) % 1_000_000)"
        // -pacificNoMusic/-pacificNoArt keep the live feeds out (dozens of real venue
        // cards would bury the seed); the seed seam is explicit and runs regardless.
        app.launchArguments += ["-pacificNoMusic", "-pacificNoArt", "-pacificSeedVenueProposal", name]
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)

        // The bell opens the Convergence deck. The seed lands in an async launch task,
        // so re-open the deck until the card is there (its snapshot is taken on open).
        let bell = app.buttons["tab-notifications"]
        XCTAssertTrue(bell.waitForExistence(timeout: 20), "the tab bell")
        // The seed runs at the END of the launch task (after Arc discovery), so this
        // poll owns a generous budget rather than a magic sleep.
        let card = app.staticTexts[name]
        for _ in 0..<10 where !card.exists {
            bell.tap()
            if card.waitForExistence(timeout: 4) { break }
            let close = app.buttons["convergence-close"]
            let done = app.buttons["Done"]
            if close.exists { close.tap() } else if done.exists { done.tap() }
        }
        XCTAssertTrue(card.exists, "the seeded venue proposal reaches the deck")
        shoot("venue-proposal-card")

        // ✓ = the accept swipe. This is the mint.
        app.buttons["convergence-accept"].tap()

        // Leave the deck (empty ⇒ Done; more cards ⇒ the header close).
        let done = app.buttons["Done"]
        if done.waitForExistence(timeout: 3) { done.tap() }
        else { app.buttons["convergence-close"].tap() }

        // The Place is REAL: LIFE → events → ＋ → Place opens the venue picker, which
        // lists held Place GroupObjects fresh from the core fold.
        let life = app.buttons["shell-life"]
        XCTAssertTrue(life.waitForExistence(timeout: 10), "LIFE disc")
        life.tap()
        let compose = app.buttons["events-new"]
        XCTAssertTrue(compose.waitForExistence(timeout: 10), "the events page's ＋")
        compose.tap()
        XCTAssertTrue(app.navigationBars["New event"].waitForExistence(timeout: 10))

        let placeRow = app.buttons["Place"]
        for _ in 0..<4 where !placeRow.isHittable { app.swipeUp() }
        XCTAssertTrue(placeRow.waitForExistence(timeout: 5), "the Where section's Place row")
        placeRow.tap()
        XCTAssertTrue(app.navigationBars["Venue"].waitForExistence(timeout: 10), "the venue picker")
        XCTAssertTrue(app.staticTexts[name].waitForExistence(timeout: 10),
                      "the accepted venue is a held Place GroupObject")
        shoot("venue-place-minted")
    }

    /// REAL-FEED artifact run — NOT part of the deterministic suite (it needs the live
    /// RA fetch, so it skips unless asked for). Launches with the Music System live,
    /// waits for the venue sweep to land, opens the deck and screenshots a real venue
    /// card. Run explicitly:
    ///   TEST_RUNNER_PACIFIC_REAL_MUSIC=1 xcodebuild test … \
    ///     -only-testing:WallFlowersUITests/VenueProposalUITests/test_realFeed_venueProposalCard_screenshot
    func test_realFeed_venueProposalCard_screenshot() throws {
        try XCTSkipUnless(ProcessInfo.processInfo.environment["PACIFIC_REAL_MUSIC"] == "1",
                          "artifact-only run — set TEST_RUNNER_PACIFIC_REAL_MUSIC=1")
        app.launch()   // no -pacificNoMusic: the live feed sweeps real venues
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)

        let bell = app.buttons["tab-notifications"]
        XCTAssertTrue(bell.waitForExistence(timeout: 20), "the tab bell")
        // The RA fetch + per-venue detail calls land seconds after launch; re-open the
        // deck until a Place question is on it.
        let placeCard = app.staticTexts["Add as a new Place?"]
        for _ in 0..<10 where !placeCard.exists {
            bell.tap()
            if placeCard.waitForExistence(timeout: 6) { break }
            let close = app.buttons["convergence-close"]
            let done = app.buttons["Done"]
            if close.exists { close.tap() } else if done.exists { done.tap() }
        }
        XCTAssertTrue(placeCard.exists,
                      "no live venue card — RA unreachable from this sim? (honest failure)")
        shoot("venue-proposal-card-real")
    }

    // MARK: - Helpers

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
