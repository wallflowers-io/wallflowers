//  MaximisedDetailUITests.swift
//  A feed card MAXIMISES: tapped, its square grows into the object's page (the
//  page's origin is the card's square, and it ends the width of the screen); RED
//  shrinks it home. On the owner's page every row is writable in place, and what
//  is written is read back from the fold when the page is opened again.
//
//  The relay is pinned to a port nothing listens on: an object of one is local, and
//  a test identity must not reach the Global Arc.

import XCTest

final class MaximisedDetailUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
    }

    /// A Post made on RED, maximised from its feed card; its owner writes its excerpt
    /// and comments on it.
    func test_aPostMaximisesFromItsCard_andItsOwnerWritesIt() {
        launch()
        let title = "MaxPost\(Int(Date().timeIntervalSince1970))"
        mintPost(title)
        toLife()
        XCTAssertTrue(app.scrollViews["life-feed"].waitForExistence(timeout: 10), "the feed")

        let page = open(title)
        let excerpt = element("detail-edit-post.setProfile.excerpt")
        XCTAssertTrue(excerpt.waitForExistence(timeout: 5), "the owner's excerpt row")
        excerpt.tap()
        app.typeText("Issue one\n")
        shoot("post-owner")

        // The comments section is on the page before anyone has spoken: its heading,
        // and the field to say something in. BLUE sends what is typed.
        XCTAssertTrue(element("detail-comments-heading").waitForExistence(timeout: 5), "the comments section")
        let field = element("detail-comment-field")
        field.tap()
        app.typeText("First word")
        app.buttons["shell-life"].tap()
        let said = app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier == 'detail-comment' AND label CONTAINS 'First word'")).firstMatch
        XCTAssertTrue(said.waitForExistence(timeout: 10), "the comment, read back")
        XCTAssertTrue(said.label.contains("QA"), "in its author's name: \(said.label)")
        XCTAssertTrue(element("detail-comments-heading").label.contains("1"), "the count")
        shoot("post-comments")

        close(page)
        _ = open(title)
        XCTAssertEqual(element("detail-edit-post.setProfile.excerpt").value as? String, "Issue one",
                       "the excerpt, read back from the fold")
    }

    /// An Event made on RED, maximised from its card; its owner sets its status
    /// and its online link in place.
    func test_anEventsOwnerWritesItsRowsInPlace() {
        launch()
        toLife()
        let title = "MaxEvent\(Int(Date().timeIntervalSince1970))"
        mint("event", title)
        XCTAssertTrue(app.scrollViews["life-feed"].waitForExistence(timeout: 10), "the feed")

        let page = open(title)
        let status = element("detail-edit-event.setProfile.status")
        XCTAssertTrue(status.waitForExistence(timeout: 5), "the owner's status row")
        status.tap()
        app.buttons["postponed"].tap()
        let online = element("detail-edit-event.setProfile.online")
        online.tap()
        app.typeText("https://meet.example/a\n")
        shoot("event-owner")

        close(page)
        _ = open(title)
        XCTAssertTrue(element("detail-edit-event.setProfile.status").label.contains("postponed"),
                      "the status, read back from the fold")
        XCTAssertEqual(element("detail-edit-event.setProfile.online").value as? String,
                       "https://meet.example/a", "the online link, read back")
    }

    /// A Listing made on RED is the owner's own: it reaches the feed and TRADE, and
    /// its card maximises from either.
    func test_aListingMadeOnRedReachesTheFeedAndTrade_andMaximises() {
        launch()
        toLife()
        let title = "MaxListing\(Int(Date().timeIntervalSince1970))"
        mint("thing", title)
        XCTAssertTrue(app.scrollViews["life-feed"].waitForExistence(timeout: 10), "the feed")

        let page = open(title)
        XCTAssertTrue(element("detail-edit-thing.setProfile.description").waitForExistence(timeout: 5),
                      "the owner's description row")
        shoot("listing-feed")
        close(page)

        app.openLifeSearch()
        app.buttons["jewel-trade"].tap()
        XCTAssertTrue(element("typed-list-trade").waitForExistence(timeout: 10), "TRADE")
        close(open(title))
    }

    /// A date is SET, once: moving the wheel through several days sends nothing, and SET
    /// sends one write (`event.setProfile` is sequenced — each write is a kept claim and
    /// publish). Counted in TRACE, where every edit is logged as sent or unchanged.
    func test_aDateChangeThroughSetSendsOnce() {
        launch()
        toLife()
        let title = "MaxDate\(Int(Date().timeIntervalSince1970))"
        mint("event", title)
        XCTAssertTrue(app.scrollViews["life-feed"].waitForExistence(timeout: 10), "the feed")
        let page = open(title)

        let starts = element("detail-edit-event.setProfile.startMs")
        XCTAssertTrue(starts.waitForExistence(timeout: 5), "the owner's start row")
        starts.buttons.firstMatch.tap()                       // the compact picker's date
        // A calendar day's label names its month ("Tuesday 10 November").
        let month = DateFormatter().monthSymbols[Calendar.current.component(.month, from: Date()) - 1]
        let days = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", month))
        XCTAssertTrue(days.firstMatch.waitForExistence(timeout: 5), "the calendar")
        let picks = days.allElementsBoundByIndex.filter(\.isHittable).suffix(3)
        XCTAssertEqual(picks.count, 3, "three days to move through")
        for day in picks { day.tap() }                        // three changes of the wheel
        let set = element("detail-set-event.setProfile.startMs")
        XCTAssertTrue(set.waitForExistence(timeout: 5), "SET, once the date differs")
        // The calendar stays open over the page: the first touch outside it closes it,
        // as a person's would, and SET is then theirs to tap.
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.08)).tap()
        expectation(for: NSPredicate(format: "hittable == true"), evaluatedWith: set)
        waitForExpectations(timeout: 5)
        set.tap()
        XCTAssertTrue(set.waitForNonExistence(timeout: 10), "the date lands and SET goes")
        close(page)

        app.buttons["account-button"].tap()
        app.buttons["account-settings"].firstMatch.tap()
        app.buttons.containing(NSPredicate(format: "label BEGINSWITH 'TRACE'")).firstMatch.tap()
        let sent = app.staticTexts.matching(NSPredicate(format: "label == 'event.setProfile sent'"))
        XCTAssertTrue(sent.firstMatch.waitForExistence(timeout: 5), "the edit is traced")
        shoot("trace")
        XCTAssertEqual(sent.count, 1, "one write for one SET, however far the wheel moved")
    }

    // MARK: the grow

    /// Tap the card titled `title` and hold the page to the grow: it starts at the
    /// card's square and ends the screen's width.
    private func open(_ title: String) -> XCUIElement {
        let card = row(titled: title)
        XCTAssertTrue(card.exists, "\(title)'s card")
        let cardFrame = card.frame
        card.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()

        let page = element("detail-media")
        XCTAssertTrue(page.waitForExistence(timeout: 10), "the card maximises")
        let o = (page.value as? String ?? "").split(separator: ",").compactMap { Double($0) }
        XCTAssertEqual(o.count, 4, "the page names the square it grew from")
        let origin = CGRect(x: o[0], y: o[1], width: o[2], height: o[3])
        XCTAssertTrue(cardFrame.insetBy(dx: -2, dy: -2).contains(origin),
                      "it grew from the card's square: \(origin) in \(cardFrame)")
        XCTAssertEqual(origin.width, origin.height, accuracy: 1, "a square")
        let screen = app.windows.firstMatch.frame
        let deadline = Date().addingTimeInterval(5)
        while page.frame.width < screen.width - 1 && Date() < deadline { Thread.sleep(forTimeInterval: 0.2) }
        XCTAssertEqual(page.frame.width, screen.width, accuracy: 1, "it ends the screen's width")
        return page
    }

    /// RED shrinks the page home, and the card is where it was.
    private func close(_ page: XCUIElement) {
        app.buttons["shell-note"].tap()
        XCTAssertTrue(page.waitForNonExistence(timeout: 5), "RED closes the page")
    }

    // MARK: helpers

    private func mintPost(_ title: String) {
        app.buttons["shell-note"].tap()
        XCTAssertTrue(element("new-kinds").waitForExistence(timeout: 10), "the dial")
        creator("note").swipeRight()
        expectation(for: NSPredicate(format: "selected == true"), evaluatedWith: element("new-kind-post"))
        waitForExpectations(timeout: 5)
        let name = creator("post").descendants(matching: .any).matching(identifier: "new-field-name").firstMatch
        name.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        app.typeText(title)
        XCTAssertEqual(app.buttons["shell-life"].label, "Save Post")
        app.buttons["shell-life"].tap()
        XCTAssertTrue(creator("post").waitForNonExistence(timeout: 10), "the creator closes on save")
    }

    /// The card, on the feed or a typed list, whose label carries `title`, scrolled until it is ON SCREEN — a
    /// lazy feed reports cards below the fold as existing, and a tap there lands on
    /// whatever is on screen instead.
    private func row(titled title: String) -> XCUIElement {
        let card = app.descendants(matching: .any)
            .matching(NSPredicate(format: "(identifier BEGINSWITH 'feed-row-' OR identifier BEGINSWITH 'list-row-') AND label CONTAINS %@",
                                  title)).firstMatch
        // On screen and above the disc row, which floats over the feed's foot: a tap
        // below its top lands on a disc.
        var clear = app.windows.firstMatch.frame
        clear.size.height = app.buttons["shell-life"].frame.minY - 8
        let mid = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.6))
        let up = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.25))
        for _ in 0..<40 {
            if card.waitForExistence(timeout: 1), clear.contains(CGPoint(x: card.frame.midX, y: card.frame.midY)) { break }
            mid.press(forDuration: 0.05, thenDragTo: up)
        }
        return card
    }

    private func element(_ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    /// One kind's card in the creator — the one on screen.
    private func creator(_ kind: String) -> XCUIElement {
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

    /// LIFE up: its feed on screen, not a page beside it — other tabs share the top
    /// row's search. The app reopens on the tab it was left on.
    private func toLife() {
        let life = app.buttons["shell-life"]
        XCTAssertTrue(life.waitForExistence(timeout: 15), "LIFE disc")
        let feed = app.scrollViews["life-feed"]
        let up = { feed.exists && abs(feed.frame.minX) < 1 }
        for _ in 0..<3 where !up() {
            life.tap()
            Thread.sleep(forTimeInterval: 1)
        }
        XCTAssertTrue(up(), "LIFE is up")
    }

    /// `kind` named `title` through RED's creator; BLUE saves it.
    private func mint(_ kind: String, _ title: String) {
        let card = app.beginCreating(kind)
        let name = card.descendants(matching: .any).matching(identifier: "new-field-name").firstMatch
        XCTAssertTrue(name.waitForExistence(timeout: 10), "the \(kind)'s NAME field")
        name.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        app.typeText(title)
        XCTAssertTrue(app.buttons["shell-life"].label.hasPrefix("Save"), "BLUE saves")
        app.buttons["shell-life"].tap()
        XCTAssertTrue(creator(kind).waitForNonExistence(timeout: 10), "the creator closes on save")
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
