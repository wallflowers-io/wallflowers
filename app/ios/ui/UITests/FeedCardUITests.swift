//  FeedCardUITests.swift
//  A Feed card names who made it, above its top left (avatar and name, from the
//  object index: the group's first owner), and what it is, above its top right (the
//  kind's glyph alone). Never "me". Below it, trailing, how long ago it was made
//  (the GroupContext's `createdAt`): "now" for one made a moment ago.
//
//  The relay is pinned to a port nothing listens on: an object of one is local, and a
//  test identity must not reach the Global Arc.

import XCTest

final class FeedCardUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
    }

    func test_eachCardNamesItsMakerShowsItsKindAndWhenItWasMade() {
        launch()
        let stamp = Int(Date().timeIntervalSince1970)
        let event = "Card event \(stamp)"
        let post = "Card post \(stamp)"

        // A Post, through RED: the ring turns one to the right from the note.
        app.buttons["shell-note"].tap()
        XCTAssertTrue(element("new-kinds").waitForExistence(timeout: 10), "the dial")
        page("note").swipeRight()
        expectation(for: NSPredicate(format: "selected == true"), evaluatedWith: element("new-kind-post"))
        waitForExpectations(timeout: 5)
        let name = page("post").descendants(matching: .any).matching(identifier: "new-field-name").firstMatch
        name.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        app.typeText(post)
        let save = app.buttons["shell-life"]
        XCTAssertEqual(save.label, "Save Post")
        save.tap()
        XCTAssertTrue(page("post").waitForNonExistence(timeout: 10), "the creator closes on save")

        // An Event, through RED's creator (LIFE has no + since 9 Nov).
        toLife()
        let eventCard = app.beginCreating("event")
        let field = eventCard.descendants(matching: .any).matching(identifier: "new-field-name").firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 10), "the event's NAME field")
        field.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        app.typeText(event)
        XCTAssertEqual(app.buttons["shell-life"].label, "Save Event")
        app.buttons["shell-life"].tap()

        // The feed — LIFE's ground while search is closed: each card's header names its
        // maker and its kind.
        XCTAssertTrue(app.scrollViews["life-feed"].waitForExistence(timeout: 10), "the feed")
        for (title, kind) in [(post, "Post"), (event, "Event")] {
            let card = row(titled: title)
            XCTAssertTrue(card.exists, "\(title)'s card on the feed")
            // One element per card (FeedView combines its texts): the header's name,
            // the avatar's initials and the kind's label are all in its label.
            XCTAssertTrue(card.label.contains("QA"), "the maker's name: \(card.label)")
            XCTAssertTrue(card.label.contains(kind), "the kind's glyph, labelled: \(card.label)")
            XCTAssertFalse(card.label.hasPrefix("me"), "never 'me': \(card.label)")
            // The recency is the card's last text (`card-recency`, trailing the footer).
            XCTAssertTrue(card.label.hasSuffix("now"), "made a moment ago: \(card.label)")
        }
        shoot("feed")
    }

    // MARK: helpers

    /// The feed card whose label carries `title`, scrolled to if the feed is long.
    private func row(titled title: String) -> XCUIElement {
        let card = app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier BEGINSWITH 'feed-row-' AND label CONTAINS %@", title)).firstMatch
        let mid = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.6))
        let up = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.25))
        for _ in 0..<40 where !card.waitForExistence(timeout: 1) { mid.press(forDuration: 0.05, thenDragTo: up) }
        return card
    }

    private func element(_ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    /// One kind's card in the creator — the one on screen.
    private func page(_ kind: String) -> XCUIElement {
        let all = app.descendants(matching: .any).matching(identifier: "new-surface-\(kind)")
        return all.allElementsBoundByIndex.first(where: \.isHittable) ?? all.firstMatch
    }

    private func launch() {
        app.launchArguments += ["-pacificNoWake", "-pacificNoMusic", "-pacificNoArt"]
        app.launchEnvironment["PACIFIC_IDENTITY_NAME"] = "QA"
        app.launchEnvironment["PACIFIC_RELAY_URL"] = "ws://127.0.0.1:9"
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)
        let welcome = app.textFields["welcome-name"]
        if welcome.waitForExistence(timeout: 3) {
            welcome.tap(); welcome.typeText("QA")
            app.buttons["welcome-begin"].tap()
        }
        XCTAssertTrue(app.buttons["shell-note"].waitForExistence(timeout: 15), "RED disc")
    }

    /// LIFE up: its search in the top row. The app reopens on the tab it was left on.
    private func toLife() {
        let life = app.buttons["shell-life"]
        XCTAssertTrue(life.waitForExistence(timeout: 15), "LIFE disc")
        let search = app.buttons["tab-search"]
        for _ in 0..<3 where !search.exists {
            life.tap()
            Thread.sleep(forTimeInterval: 1)
        }
        XCTAssertTrue(search.exists, "LIFE is up")
    }

    private func shoot(_ name: String) {
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    @discardableResult
    private func tapSpringboard(_ labels: [String], waitFor timeout: TimeInterval = 0) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            for l in labels where springboard.buttons[l].exists {
                springboard.buttons[l].tap()
                return true
            }
        } while Date() < deadline
        return false
    }
}
