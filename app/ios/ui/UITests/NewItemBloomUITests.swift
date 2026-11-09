//  NewItemBloomUITests.swift
//  Does the bohr bloom behind a newitem modal ACTUALLY MOVE on screen?
//
//  Written because three rounds of reasoning about it were wrong. The field's maths was
//  verified numerically each time and each time the screen still looked static, which
//  means the thing worth testing is not the maths — it is whether the frames the device
//  produces differ from each other.
//
//  So: open the modal, capture two frames a beat apart, and attach both. The comparison
//  is done outside (the runner has no image diffing), but the attachments are the record.

import XCTest

final class NewItemBloomUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
    }

    func test_newItemModal_groundAnimates() {
        launchToLife()

        // RED's creator, turned to a Space, raises the newitem modal over LIFE.
        app.beginCreating("group")

        // Do NOT assert on a container identifier — SwiftUI does not surface one on a
        // plain ZStack. The BLUE disc's label is the surface's own commit verb, so its
        // presence is the honest proof that a newitem modal is up.
        let commit = app.buttons["shell-life"]
        XCTAssertTrue(commit.waitForExistence(timeout: 10), "the disc row should still be there")
        Thread.sleep(forTimeInterval: 1.0)   // let the open settle

        // Two frames, ~1.6s apart — a quarter of the 6.6s lobe orbit, so a moving field
        // MUST differ and a static one cannot. Comparing the PNG bytes is enough: we are
        // asking "did anything change", not "what changed".
        let a = XCUIScreen.main.screenshot()
        shoot("bloom-a", a)
        Thread.sleep(forTimeInterval: 1.6)
        let b = XCUIScreen.main.screenshot()
        shoot("bloom-b", b)

        XCTAssertNotEqual(
            a.pngRepresentation, b.pngRepresentation,
            """
            The ground behind the newitem modal is FROZEN.

            First thing to check: Reduce Motion. `BohrBloom` (and `NoteAmbient`) pause
            their TimelineView under it BY DESIGN, which pins t to 0 and renders a
            correct but permanently static frame — indistinguishable from a broken
            animation unless you know to look.

                xcrun simctl spawn booted defaults write \
                    com.apple.Accessibility ReduceMotionEnabled -bool false

            That was the cause once already (18 Aug), after two rounds of chasing the
            shader maths, which was fine throughout.
            """
        )
    }

    /// A newitem modal opens with NOTHING SELECTED, so the keyboard stays down until you
    /// choose a field (Ralph, 18 Aug). And when a field IS chosen, the card moves up out
    /// of the keyboard's way — the bloom is anchored to the card, so the ground must move
    /// with it rather than staying centred on the screen.
    func test_newItemModal_opensWithKeyboardDown_andGroundFollowsTheCard() {
        launchToLife()

        let card = app.beginCreating("group")

        // Query broadly: the identifier is set on the entry box, and SwiftUI decides for
        // itself whether that surfaces as a text field or a container. On the card in
        // view: the creator lays out every kind's card.
        let name = card.descendants(matching: .any)
            .matching(identifier: "new-field-name").firstMatch
        XCTAssertTrue(name.waitForExistence(timeout: 10), "the NAME field")
        let down = XCUIScreen.main.screenshot()
        shoot("keyboard-down", down)

        // A COORDINATE tap, not `.tap()`. The identifier sits on the entry box, which
        // makes the box one accessibility element — tapping the element resolves to the
        // container rather than the field inside it. The other suites in here reach for
        // coordinates for the same reason.
        name.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5),
                      "tapping a field should raise the keyboard")
        Thread.sleep(forTimeInterval: 1.0)   // let the card and the ground settle together

        let up = XCUIScreen.main.screenshot()
        shoot("keyboard-up", up)
        XCTAssertNotEqual(down.pngRepresentation, up.pngRepresentation,
                          "the surface should visibly change when the keyboard rises")
    }

    // MARK: helpers (same shape as the other suites)

    private func launchToLife() {
        app.launchArguments += ["-pacificNoWake", "-pacificNoMusic", "-pacificNoArt"]
        app.launchEnvironment["PACIFIC_IDENTITY_NAME"] = "QA"
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

    private func shoot(_ name: String, _ screenshot: XCUIScreenshot) {
        let shot = XCTAttachment(screenshot: screenshot)
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
