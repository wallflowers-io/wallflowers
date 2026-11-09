//  ExteriorUITests.swift
//  THE EXTERIOR AND ITS DOOR (#76), by real taps on two devices. An owner makes a project
//  that reaches the network, with words for outsiders and a door; a connection who is not in
//  it sees the card, opens it, and asks to join; the owner is told, and admits them.
//
//  ONE DEVICE PER RUN, as the fleet runs (FleetJourneyUITests): `scripts/exterior-e2e.sh`
//  installs two simulators, pairs them over the dev seam (pairing is not what is tested
//  here), and hands each its steps in turn. The devices hand off through the relay alone.
//
//  Config arrives in /tmp/pacific-exterior.env (`key=value`): the relay, each device's role
//  by its simulator's name, and each role's name. Never a way to drive the app.

import XCTest
import UIKit

final class ExteriorUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    static let configPath = "/tmp/pacific-exterior.env"
    static let objectName = "Allotment"
    static let outsideWords = "Plots by the river"

    private var config: [String: String] = [:]
    private var role: String { config["role.\(UIDevice.current.name)"] ?? "" }

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        config = Self.readConfig()
    }

    // MARK: - The steps, in the order the script runs them

    /// OWNER: a project at Network, with words for outsiders, and "Ask to join".
    func test_1_owner_writesAnExterior() throws {
        try runAs("owner")
        app.buttons["shell-note"].tap()
        XCTAssertTrue(element("new-kinds").waitForExistence(timeout: 10), "the creator's dial")
        element("new-kind-project").tap()
        let name = page("project").descendants(matching: .any).matching(identifier: "new-field-name").firstMatch
        XCTAssertTrue(name.waitForExistence(timeout: 5), "the project's NAME")
        name.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        app.typeText(Self.objectName)

        // GOLD turns the card to VISIBILITY; outward, it asks what outsiders see.
        app.buttons["shell-chat"].tap()
        XCTAssertTrue(element("visibility-network").waitForExistence(timeout: 5), "the visibility face")
        XCTAssertFalse(element("outside-description").exists, "nothing outside while it is Only Me")
        element("visibility-network").tap()
        let words = element("outside-description")
        XCTAssertTrue(words.waitForExistence(timeout: 5), "the outside's words, once it reaches out")
        words.tap()
        app.typeText(Self.outsideWords)
        XCTAssertTrue(element("door-review").isSelected, "a door defaults to Ask to join")
        shoot("owner-outside")

        // BLUE commits; the creator leaves.
        app.buttons["shell-life"].tap()
        XCTAssertTrue(element("new-kinds").waitForNonExistence(timeout: 15), "the project is made")
    }

    /// ASKER: the card reaches me, opens to its outside, and I ask to join.
    func test_2_asker_asksToJoin() throws {
        try runAs("asker")
        let card = waitForExteriorCard(timeout: 180)
        XCTAssertNotNil(card, "the exterior reaches a connection")
        shoot("asker-card")
        card?.tap()
        XCTAssertTrue(app.staticTexts["detail-title"].waitForExistence(timeout: 10), "the exterior opens")
        XCTAssertEqual(app.staticTexts["detail-title"].label, Self.objectName)
        XCTAssertTrue(app.staticTexts[Self.outsideWords].exists, "the owner's words for outsiders")
        let blue = app.buttons["shell-life"]
        XCTAssertEqual(blue.label, "Ask to join")
        shoot("asker-exterior")
        blue.tap()
        XCTAssertTrue(element("detail-surface").waitForNonExistence(timeout: 30), "the ask is sent")

        // Asked: the same card opens to "Requested".
        waitForExteriorCard(timeout: 10)?.tap()
        XCTAssertTrue(app.staticTexts["detail-title"].waitForExistence(timeout: 10))
        XCTAssertEqual(app.buttons["shell-life"].label, "Requested")
        shoot("asker-requested")
    }

    /// OWNER: the bell is red, the ask is in the tray, and Admit lets them in.
    func test_3_owner_admits() throws {
        try runAs("owner")
        let bell = app.buttons["tab-notifications"]
        XCTAssertTrue(bell.waitForExistence(timeout: 10), "the bell")
        let asked = NSPredicate(format: "label CONTAINS 'to review'")
        expectation(for: asked, evaluatedWith: bell)
        waitForExpectations(timeout: 120)
        bell.tap()
        let asker = config["name.asker"] ?? ""
        XCTAssertTrue(app.staticTexts[asker].waitForExistence(timeout: 30), "\(asker)'s ask, by name")
        XCTAssertTrue(app.staticTexts[Self.objectName].exists, "for the project")
        shoot("owner-tray")
        let admit = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'request-admit-'")).firstMatch
        admit.tap()
        XCTAssertTrue(app.staticTexts[asker].waitForNonExistence(timeout: 60), "answered, the ask leaves the tray")
    }

    /// ASKER: inside now, the exterior card is gone: I hold the object itself.
    func test_4_asker_isInside() throws {
        try runAs("asker")
        let deadline = Date().addingTimeInterval(180)
        while Date() < deadline && waitForExteriorCard(timeout: 1) != nil {
            RunLoop.current.run(until: Date().addingTimeInterval(2))
        }
        XCTAssertNil(waitForExteriorCard(timeout: 1), "inside, the exterior card is gone")
        shoot("asker-inside")
    }

    // MARK: - Helpers

    private static func readConfig() -> [String: String] {
        guard let text = try? String(contentsOfFile: configPath, encoding: .utf8) else { return [:] }
        var out: [String: String] = [:]
        for line in text.split(separator: "\n") {
            let p = line.split(separator: "=", maxSplits: 1)
            if p.count == 2 { out[String(p[0])] = String(p[1]) }
        }
        return out
    }

    /// Run only on the device the script cast in `want`, launched as its person.
    private func runAs(_ want: String) throws {
        try XCTSkipIf(config["relay"] == nil, "two devices: run scripts/exterior-e2e.sh, which writes \(Self.configPath)")
        try XCTSkipIf(role != want, "this device is the \(role), not the \(want)")
        app.launchArguments += ["-pacificNoWake", "-pacificNoMusic", "-pacificNoArt"]
        app.launchEnvironment["PACIFIC_RELAY_URL"] = config["relay"]
        app.launchEnvironment["PACIFIC_IDENTITY_NAME"] = config["name.\(want)"]
        app.launch()
        for label in ["Allow", "Don’t Allow", "Don't Allow"] where springboard.buttons[label].waitForExistence(timeout: 2) {
            springboard.buttons[label].tap()
        }
        XCTAssertTrue(app.buttons["shell-life"].waitForExistence(timeout: 20), "the shell")
        if want == "asker" { app.buttons["shell-life"].tap() }
    }

    /// The feed's exterior card, as whichever element the OS exposes it as (FeedUITests'
    /// note); nil if none shows within `timeout`.
    private func waitForExteriorCard(timeout: TimeInterval) -> XCUIElement? {
        let match = NSPredicate(format: "identifier BEGINSWITH 'feed-row-exterior:'")
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            for q in [app.otherElements, app.staticTexts] where q.matching(match).firstMatch.exists {
                return q.matching(match).firstMatch
            }
            RunLoop.current.run(until: Date().addingTimeInterval(1))
        } while Date() < deadline
        return nil
    }

    private func element(_ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    private func page(_ kind: String) -> XCUIElement {
        let all = app.descendants(matching: .any).matching(identifier: "new-surface-\(kind)")
        return all.allElementsBoundByIndex.first(where: \.isHittable) ?? all.firstMatch
    }

    private func shoot(_ name: String) {
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }
}
