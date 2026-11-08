//  CommentsSectionUITests.swift
//  An object's comments section is its parent's, not a conversation (#34). Every Event
//  is minted with a Forum in the comments role; a comment on the event's page lands
//  there, shows on the page, and Gold lists no "comments" chat for it.
//
//  The relay is pinned to a port nothing listens on: an object of one is local, and a
//  test identity must not reach the Global Arc.

import XCTest

final class CommentsSectionUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
    }

    func test_aCommentShowsOnItsEvent_andGoldListsNoCommentsChat() {
        launchToLife()
        let name = "Comments \(Int(Date().timeIntervalSince1970))"
        let comment = "see you there \(Int(Date().timeIntervalSince1970))"

        // Mint an Event through LIFE's +.
        app.buttons["tab-new"].tap()
        let field = app.descendants(matching: .any).matching(identifier: "new-field-name").firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 10), "the event's NAME field")
        field.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        app.typeText(name)
        let save = app.buttons["shell-life"]
        XCTAssertEqual(save.label, "Save Event")
        save.tap()

        // Open it from the EVENT jewel's feed.
        app.buttons["jewel-events"].tap()
        // The feed draws only what is on screen: earlier runs' events can hold the top.
        let row = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", name)).firstMatch
        let mid = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.6))
        let up = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.25))
        for _ in 0..<12 where !row.waitForExistence(timeout: 2) { mid.press(forDuration: 0.05, thenDragTo: up) }
        XCTAssertTrue(row.exists, "the event's feed row")
        row.tap()
        XCTAssertTrue(app.staticTexts["detail-title"].waitForExistence(timeout: 10), "the event's page")
        XCTAssertEqual(app.staticTexts["detail-title"].label, name)

        // Comment; BLUE sends.
        let say = app.textFields.firstMatch   // the page's one field
        XCTAssertTrue(say.waitForExistence(timeout: 10), "the comment field")
        say.tap()
        say.typeText(comment)
        let send = app.buttons["shell-life"]
        XCTAssertEqual(send.label, "Send")
        XCTAssertTrue(send.isEnabled, "BLUE sends a comment")
        send.tap()
        let said = app.staticTexts[comment]
        XCTAssertTrue(said.waitForExistence(timeout: 10), "the comment shows on its event")
        app.staticTexts["detail-title"].tap()   // the card back to its top
        XCTAssertTrue(said.isHittable, "the comment is on screen")
        shoot("event-with-comment")

        // Gold: the event's comments section is not a chat.
        app.buttons["shell-note"].tap()   // Close
        let gold = app.buttons["shell-chat"]
        XCTAssertTrue(gold.waitForExistence(timeout: 10))
        gold.tap()
        let selected = expectation(for: NSPredicate(format: "isSelected == true"), evaluatedWith: gold)
        wait(for: [selected], timeout: 10)
        Thread.sleep(forTimeInterval: 2)   // a rebuild pass
        shoot("gold")
        XCTAssertFalse(app.staticTexts["comments"].exists, "Gold lists a comments section as a chat")
        XCTAssertFalse(app.staticTexts[comment].exists, "the comment surfaces in Gold")
    }

    // MARK: helpers (same shape as the other suites)

    private func launchToLife() {
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
        // The app reopens on the tab it was left on: LIFE is up when its + makes an Event.
        let life = app.buttons["shell-life"]
        XCTAssertTrue(life.waitForExistence(timeout: 15), "LIFE disc")
        let plus = app.buttons["tab-new"]
        for _ in 0..<3 where plus.label != "New Event" {
            life.tap()
            Thread.sleep(forTimeInterval: 1)
        }
        XCTAssertEqual(plus.label, "New Event", "LIFE is up")
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
