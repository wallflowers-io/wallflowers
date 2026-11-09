//  FeedUITests.swift
//  The Active Scroll Feed — LIFE's ground state (FEED-TRANSMISSION 0-ANSWER-C /
//  0-ANSWER-G), asserted on the REAL app with seeded Event GroupObjects (the
//  EventSeeder's true mints — never fixture rows):
//   • maximise/minimise: a jewel's tap swaps the feed for that type's flat list,
//     its second tap — or the orb — restores the feed;
//   • the LOCK gesture (a short drag DOWN on a jewel, S33) filters the feed by
//     delta kind: an empty lock set passes everything, a locked set lets only
//     those kinds through;
//   • the EMPTY state is the honest "—", reached deterministically by locking a
//     kind that has no rows (JOB has no source yet, by design).

import XCTest

final class FeedUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"]) ?? false
        }
    }

    func test_jewelMaximisesAndMinimises_orbIsHome() {
        launchToLife()
        XCTAssertTrue(waitGround("life-feed", timeout: 10), "the feed ground")
        // ANY kind: the QA device accumulates travelled listings (they ride at
        // `now` and sort ahead of tomorrow's seeds), and the LazyVStack only
        // materialises the visible stretch — so "rows exist" is the honest
        // assertion here; kind-specific counting belongs to the LOCK tests,
        // where the filter puts one kind on top deterministically.
        XCTAssertTrue(waitAnyFeedRows(timeout: 8), "the feed carries rows (seeds guarantee it)")
        shoot("feed-ground")

        // Maximise EVENT: the typed list takes the ground, the feed leaves.
        app.buttons["jewel-events"].tap()
        XCTAssertTrue(waitGround("typed-list-events"), "EVENT maximised = its flat list")
        XCTAssertFalse(groundExists("life-feed"), "the feed yields")
        shoot("feed-maximised-events")

        // The ORB minimises everything — the reset circle (S31/S16).
        app.buttons["feed-orb"].tap()
        XCTAssertTrue(waitGround("life-feed"), "the orb is home — the feed returns")
        XCTAssertFalse(groundExists("typed-list-events"), "no list residue")
        shoot("feed-orb-home")

        // Maximise, then the SAME jewel's second tap minimises too.
        app.buttons["jewel-people"].tap()
        XCTAssertTrue(waitGround("typed-list-people"), "PERSON maximised")
        app.buttons["jewel-people"].tap()
        XCTAssertTrue(waitGround("life-feed"), "second tap on the jewel restores the feed")
    }

    func test_lockGesture_filtersTheFeedByKind_andUnlockRestores() {
        launchToLife()
        XCTAssertTrue(waitGround("life-feed", timeout: 10), "the feed ground")
        XCTAssertTrue(waitAnyFeedRows(timeout: 8), "rows before any lock")

        // LOCK POSTS: drag the jewel a short way DOWN (threshold 9pt, S33). POSTS
        // has no delta source yet (JOB left the rail 11 Aug), so a POSTS-only
        // feed is deterministically EMPTY — which is exactly what proves the
        // lock FILTERS rather than decorates.
        lock("jewel-posts")
        XCTAssertTrue(waitEmptyMarker(timeout: 6),
                      "locking POSTS leaves no deltas standing — the honest empty —")
        XCTAssertEqual(anyFeedRows(), 0, "no delta of any kind penetrates a POSTS lock")
        shoot("feed-locked-posts")

        // UNLOCK (same gesture): the filter lifts, every delta passes again.
        lock("jewel-posts")
        XCTAssertTrue(waitAnyFeedRows(timeout: 8),
                      "unlocking restores the unfiltered feed")
        shoot("feed-unlocked")
    }

    func test_lockOnPopulatedKind_keepsItsRowsOnly() {
        launchToLife()
        XCTAssertTrue(waitGround("life-feed", timeout: 10), "the feed ground")
        XCTAssertTrue(waitAnyFeedRows(timeout: 8), "rows before the lock")

        // LOCK EVENT: event rows stay; the feed never shows the empty marker.
        lock("jewel-events")
        XCTAssertTrue(waitFeedRows(ofKind: "event", timeout: 6),
                      "an EVENT lock keeps event deltas standing")
        XCTAssertFalse(app.staticTexts["feed-empty"].exists,
                       "a populated lock is not the empty state")
        shoot("feed-locked-events")
        lock("jewel-events")   // leave the device unfiltered for the next test
    }

    // MARK: - Helpers

    private func launchToLife() {
        // Real seeded GroupObjects; no wake banners; no external Systems rows.
        app.launchArguments += ["-pacificSeedEvents", "-pacificNoWake",
                                "-pacificNoMusic", "-pacificNoArt"]
        app.launchEnvironment["PACIFIC_IDENTITY_NAME"] = "QA"   // L2: fresh sims skip Welcome
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)

        let welcome = app.textFields["welcome-name"]
        if welcome.waitForExistence(timeout: 3) {
            welcome.tap(); welcome.typeText("QA")
            app.buttons["welcome-begin"].tap()
        }

        let life = app.buttons["shell-life"]
        XCTAssertTrue(life.waitForExistence(timeout: 10), "LIFE disc")
        if !life.isSelected { life.tap() }
    }

    /// The S33 lock: a short decisive drag DOWN on the jewel. The jewel clamps the
    /// visible travel to half a radius; the THRESHOLD is 9pt of translation, so a
    /// 30pt pull crosses it with margin and the release toggles the lock.
    private func lock(_ jewelID: String) {
        let jewel = app.buttons[jewelID]
        XCTAssertTrue(jewel.waitForExistence(timeout: 5), "\(jewelID) to lock")
        let start = jewel.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        let end = start.withOffset(CGVector(dx: 0, dy: 30))
        start.press(forDuration: 0.1, thenDragTo: end)
    }

    /// Feed rows of one kind, by identifier prefix. The card is ONE combined
    /// accessibility element (FeedView applies `.combine` for exactly this) and a Button
    /// (it opens the object), but the OS may expose it as a staticText — count both,
    /// never a broad `.any` descendants sweep of the whole LIFE hierarchy
    /// (GroupsPageUITests' hard-won note: that snapshot can take the runner down).
    private func feedRows(ofKind kind: String) -> Int {
        let match = NSPredicate(format: "identifier BEGINSWITH %@", "feed-row-\(kind):")
        return app.buttons.matching(match).count
             + app.staticTexts.matching(match).count
    }

    /// Rows of ANY kind — the identifier space without the kind facet.
    private func anyFeedRows() -> Int {
        let match = NSPredicate(format: "identifier BEGINSWITH 'feed-row-'")
        return app.buttons.matching(match).count
             + app.staticTexts.matching(match).count
    }

    private func waitAnyFeedRows(timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if anyFeedRows() > 0 { return true }
            usleep(400_000)
        }
        return anyFeedRows() > 0
    }

    private func waitFeedRows(ofKind kind: String, timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if feedRows(ofKind: kind) > 0 { return true }
            usleep(400_000)
        }
        return feedRows(ofKind: kind) > 0
    }

    private func waitEmptyMarker(timeout: TimeInterval) -> Bool {
        app.staticTexts["feed-empty"].waitForExistence(timeout: timeout)
    }

    private func groundExists(_ id: String) -> Bool {
        app.scrollViews[id].exists || app.otherElements[id].exists
    }

    @discardableResult
    private func waitGround(_ id: String, timeout: TimeInterval = 5) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if groundExists(id) { return true }
            usleep(300_000)
        }
        return groundExists(id)
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
            let b = springboard.buttons[label]
            let present = timeout > 0 ? b.waitForExistence(timeout: timeout) : b.exists
            if present { b.tap(); return true }
        }
        return false
    }
}
