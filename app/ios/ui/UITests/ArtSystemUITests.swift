//  ArtSystemUITests.swift
//  The Art System on the REAL app, over the REAL feed: Settings ▸ SYSTEMS ▸ ART shows
//  the scope and a live pull's listings, and a listing the System pulled is findable in
//  LIFE's EVENTS page by its own title — the whole path, no fixtures.
//
//  PRECONDITION: network, and ArtRabbit listing something for the default scope
//  (San Francisco). An empty world fails LOUD here by design — this suite exists to
//  prove the live path, the way VenueProposalUITests' live half proves RA's.

import XCTest

final class ArtSystemUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"]) ?? false
        }
    }

    /// The Art System is parked (ArtService.enabled == false, 4 Aug) — no pull
    /// happens, so the live path this suite proves cannot run. Flip both flags
    /// together when the System comes back.
    private static let systemLive = false

    func test_artSystem_pullsLiveListings_andTheFeedCarriesThem() throws {
        try XCTSkipUnless(Self.systemLive,
                          "Art System is parked — flip ArtService.enabled to run the live suite")
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)

        // Settings is behind the account avatar — the same taps a user makes.
        let account = app.buttons["account-button"]
        XCTAssertTrue(account.waitForExistence(timeout: 15), "account avatar button")
        account.tap()
        tapCard("SYSTEMS")
        tapCard("ART")
        XCTAssertTrue(app.navigationBars["Art"].waitForExistence(timeout: 5), "the ART screen")

        // The launch pull is a live network fetch; give it room. A loud error is a
        // FAILURE with the source's own words — never a silent pass.
        let row = app.staticTexts.containing(
            NSPredicate(format: "label CONTAINS ' listings'")).firstMatch
        if !row.waitForExistence(timeout: 30) {
            let err = app.staticTexts.matching(identifier: "art-error").firstMatch
            XCTFail("no listings landed — \(err.exists ? err.label : "no status shown")")
        }
        shoot("settings-systems-art")

        // Carry a REAL pulled title into the feed: the first VENUED listing — the feed
        // only carries rows with a participant, and a venueless card has none. The
        // title Text wears its own identifier (row flattening is SwiftUI's to change).
        let titleText = app.staticTexts.matching(identifier: "art-listing-title-venued").firstMatch
        XCTAssertTrue(titleText.waitForExistence(timeout: 10), "a venued listing's title")
        let title = titleText.label
        XCTAssertFalse(title.isEmpty, "the listing title should be readable")
        let query = Self.typablePrefix(of: title)

        // Leave Settings the way a thumb does: NOTE is centred as the dismiss disc
        // while the overlay is up. Then LIFE lands on the EVENTS page.
        let dismiss = app.buttons["shell-note"]
        XCTAssertTrue(dismiss.waitForExistence(timeout: 8), "the settings dismiss disc")
        dismiss.tap()
        let life = app.buttons["shell-life"]
        XCTAssertTrue(life.waitForExistence(timeout: 15), "LIFE disc")
        life.tap()
        XCTAssertTrue(app.textFields["Search events, venues"].waitForExistence(timeout: 10),
                      "the events search pill")

        // NEAR looks forward — and the span-aware window keeps an OPEN run visible for
        // its whole span, which is exactly what an exhibition is. The lens's list is a
        // plain VStack, so the row exists in the hierarchy wherever it scrolled to —
        // no typing, no keyboard, no flake.
        let hit = app.staticTexts.containing(
            NSPredicate(format: "label BEGINSWITH[c] %@", query)).firstMatch
        XCTAssertTrue(hit.waitForExistence(timeout: 10),
                      "the feed should carry the pulled listing “\(title)” (matched on “\(query)”)")
        shoot("events-near-with-art")
    }

    /// The longest leading run of the title the simulator keyboard can type plainly —
    /// letters, digits and spaces; live titles carry anything.
    private static func typablePrefix(of title: String) -> String {
        let run = title.prefix { $0.isLetter || $0.isNumber || $0 == " " }
            .trimmingCharacters(in: .whitespaces)
        return run.count >= 4 ? String(run.prefix(18)) : String(title.prefix(8))
    }

    // MARK: - Helpers

    private func shoot(_ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    /// Wait for a settings card, shedding system alerts meanwhile — the live venue
    /// sweep can raise the notification-permission prompt at any moment after launch.
    private func tapCard(_ title: String) {
        let card = app.buttons.containing(
            NSPredicate(format: "label BEGINSWITH %@", title)).firstMatch
        for _ in 0..<5 {
            if card.waitForExistence(timeout: 2) { card.tap(); return }
            _ = tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"])
        }
        XCTFail("the \(title) card should exist")
    }

    @discardableResult
    private func tapSpringboard(_ labels: [String], waitFor timeout: TimeInterval = 0) -> Bool {
        for label in labels {
            let button = springboard.buttons[label]
            let present = timeout > 0 ? button.waitForExistence(timeout: timeout) : button.exists
            if present { button.tap(); return true }
        }
        return false
    }
}
