//  EventMintUITests.swift
//  The listing→event→discussion path on the REAL app over the REAL feed: a live
//  reference listing (RA/ArtRabbit) MINTS into an Event GroupObject, the minted page
//  carries DISCUSS (the event's own thread) and PEOPLE (the roster), and the thread
//  opens. Re-runs land on the SAME event (MintedFrom, never a twin) via OPEN EVENT.
//
//  PRECONDITION: network + the default scope listing something (the ArtSystemUITests
//  contract). The multi-device half — a member's device folding the same thread — is
//  proven at core level by `shared_event_carries_its_own_discussion` (real MLS).

import XCTest

final class EventMintUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"]) ?? false
        }
    }

    func test_listingMints_andTheEventCarriesItsOwnThread() {
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)

        // A REAL pulled listing, by its own title — the ArtSystemUITests navigation:
        // read the first venued listing off Settings ▸ SYSTEMS ▸ ART…
        let account = app.buttons["account-button"]
        XCTAssertTrue(account.waitForExistence(timeout: 15), "account avatar button")
        account.tap()
        tapCard("SYSTEMS")
        tapCard("ART")
        // A cold install's launch task (identity, Arc discovery, then the pulls) can
        // take a while before listings land — wait generously, shedding alerts.
        let titleText = app.staticTexts.matching(identifier: "art-listing-title-venued").firstMatch
        var landed = false
        for _ in 0..<12 where !landed {
            landed = titleText.waitForExistence(timeout: 5)
            _ = tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"])
        }
        XCTAssertTrue(landed, "a venued listing's title")
        let query = Self.typablePrefix(of: titleText.label)
        XCTAssertFalse(query.isEmpty, "the listing title should be readable")

        // …then find its row in the EVENTS feed and TAP it — the reference page.
        let dismiss = app.buttons["shell-note"]
        XCTAssertTrue(dismiss.waitForExistence(timeout: 8), "the settings dismiss disc")
        dismiss.tap()
        let life = app.buttons["shell-life"]
        XCTAssertTrue(life.waitForExistence(timeout: 15), "LIFE disc")
        life.tap()
        let hit = app.staticTexts.containing(
            NSPredicate(format: "label BEGINSWITH[c] %@", query)).firstMatch
        XCTAssertTrue(hit.waitForExistence(timeout: 10), "the feed carries the listing")
        hit.tap()

        // MINT on a fresh device, OPEN on a re-run — both land on the minted page.
        let mint = app.buttons["event-mint"]
        let open = app.buttons["event-open-minted"]
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline && !mint.exists && !open.exists {
            RunLoop.current.run(until: Date().addingTimeInterval(0.5))
        }
        XCTAssertTrue(mint.exists || open.exists, "the listing page should offer MINT/OPEN")
        (mint.exists ? mint : open).tap()

        // The cover swaps (0.55s settle) to the MINTED page: DISCUSS + (owner) PEOPLE.
        let discuss = app.buttons["event-discuss"]
        XCTAssertTrue(discuss.waitForExistence(timeout: 10), "the minted page's DISCUSS")
        XCTAssertTrue(app.buttons["event-people"].exists, "the owner sees PEOPLE")
        shoot("event-minted-from-listing")

        // The event's own thread opens — empty and honest until someone speaks.
        discuss.tap()
        XCTAssertTrue(app.staticTexts["No posts yet"].waitForExistence(timeout: 8),
                      "the event's thread should open")
        shoot("event-thread")

    }

    // MARK: - Helpers

    /// The longest leading run of the title the query can carry plainly — letters,
    /// digits and spaces (the ArtSystemUITests helper, mirrored).
    private static func typablePrefix(of title: String) -> String {
        let run = title.prefix { $0.isLetter || $0.isNumber || $0 == " " }
            .trimmingCharacters(in: .whitespaces)
        return run.count >= 4 ? String(run.prefix(18)) : String(title.prefix(8))
    }

    /// Wait for a settings card, shedding system alerts meanwhile.
    private func tapCard(_ title: String) {
        let card = app.buttons.containing(
            NSPredicate(format: "label BEGINSWITH %@", title)).firstMatch
        for _ in 0..<5 {
            if card.waitForExistence(timeout: 2) { card.tap(); return }
            _ = tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"])
        }
        XCTFail("the \(title) card should exist")
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
            let button = springboard.buttons[label]
            let present = timeout > 0 ? button.waitForExistence(timeout: timeout) : button.exists
            if present { button.tap(); return true }
        }
        return false
    }
}
