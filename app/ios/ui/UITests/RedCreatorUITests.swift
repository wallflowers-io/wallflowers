//  RedCreatorUITests.swift
//  RED is the universal creator: one card per kind on a ring, turned by a swipe on the
//  card or by the dial above it, wrapping both ways. Nothing is focused, so the keyboard
//  stays down, and the first edit chooses the kind. Each card is a tablet: nothing scrolls
//  inside it, it scrolls whole, and no colour lies past its border.

import XCTest

final class RedCreatorUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
    }

    func test_red_pagesBetweenKinds_untilTheFirstEditChoosesOne() {
        launchToShell()

        // RED opens the creator on a note, nothing focused.
        app.buttons["shell-note"].tap()
        XCTAssertTrue(element("new-kinds").waitForExistence(timeout: 10), "the dial")
        assertInView("note")
        XCTAssertTrue(element("note-reminder-bell").exists, "opens on the note")
        assertKeyboardDown("on open")
        shoot("note")

        // A swipe on the card moves one kind, and the ring wraps both ways.
        page("note").swipeLeft()
        assertInView("group")
        page("group").swipeRight()
        assertInView("note")
        page("note").swipeRight()
        assertInView("post")
        page("post").swipeLeft()
        assertInView("note")

        // The wrap, then a tap back: the dial must still be there and the keyboard down.
        // (It once locked itself here with no edit.)
        page("note").swipeRight()
        assertInView("post")
        element("new-kind-note").tap()
        assertInView("note")
        XCTAssertTrue(element("new-kinds").exists, "no lock without an edit")
        assertKeyboardDown("after a wrap and a tap")

        // The dial shows five: the kind in view, its neighbours, and one more each side.
        // Wholly inside the dial: an item whose edge only touches the clip still reports
        // itself hittable.
        let dial = element("new-kinds").frame.insetBy(dx: -1, dy: -1)
        let onDial = app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier BEGINSWITH 'new-kind-'")).allElementsBoundByIndex
            .filter { dial.contains($0.frame) }
        XCTAssertEqual(onDial.count, 5, "the dial shows five kinds")

        // A flung dial carries the cards with it. Nothing puts the cards where the dial
        // stopped, so they show its kind only if they followed it all the way.
        element("new-kinds").swipeLeft(velocity: .fast)
        let flung = settled()
        XCTAssertNotEqual(flung, "note", "a fling turns the dial")
        assertInView(flung)

        // Dragging the dial turns the cards with it, and they land where the dial does. This
        // asserts that it moved and that the two agree, not a count.
        turnDial(by: 2)
        let dialled = inView()
        XCTAssertNotEqual(dialled, "note", "a drag on the dial turns it")
        assertInView(dialled)
        shoot("by the dial")

        // A tap on a neighbour turns to it; all the way round, every kind renders its own
        // declared fields, keyboard down throughout. Then on round to POST.
        var at = Self.ring.firstIndex(of: dialled)!
        let post = Self.ring.firstIndex(of: "post")!
        let steps = Self.ring.count + (post - at + Self.ring.count) % Self.ring.count
        for _ in 0..<steps {
            at = (at + 1) % Self.ring.count
            let kind = Self.ring[at]
            element("new-kind-\(kind)").tap()
            assertInView(kind)
            if kind != "note" {
                XCTAssertTrue(page(kind).descendants(matching: .any).matching(identifier: "new-field-name")
                                  .firstMatch.waitForExistence(timeout: 5), "\(kind)'s NAME")
            }
            assertKeyboardDown("on \(kind)")
            XCTAssertTrue(element("new-kinds").exists, "no lock without an edit (on \(kind))")
            if kind == "place" {
                // Showing a Place asks for nothing; editing it does.
                XCTAssertEqual(page("place").descendants(matching: .any)["new-fix"].label, "Tap to locate")
                XCTAssertFalse(springboard.alerts.firstMatch.exists, "no location prompt for a Place in view")
            }
            if kind == "listing" {
                XCTAssertEqual(element("new-kind-listing").label, "Listing", "the dial names a Listing")
            }
        }

        // The first edit chooses the kind: the dial goes and the cards stop turning.
        let name = page("post").descendants(matching: .any).matching(identifier: "new-field-name").firstMatch
        name.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5), "a tapped field raises the keyboard")
        app.typeText("Library night")
        XCTAssertTrue(element("new-kinds").waitForNonExistence(timeout: 5), "an edit locks the kind")
        shoot("locked")
        page("post").swipeRight()
        Thread.sleep(forTimeInterval: 0.6)
        XCTAssertTrue(page("post").isHittable, "a locked card does not page")

        // GOLD turns to VISIBILITY and back.
        app.buttons["shell-chat"].tap()
        XCTAssertTrue(element("visibility-private").waitForExistence(timeout: 5), "the visibility face")
        shoot("visibility")
        app.buttons["shell-chat"].tap()
        XCTAssertTrue(name.waitForExistence(timeout: 5), "back to the fields")

        // RED with content warns, then discards.
        app.buttons["shell-note"].tap()
        let discard = app.buttons["Discard"]
        XCTAssertTrue(discard.waitForExistence(timeout: 5), "the discard warning")
        discard.tap()
        XCTAssertTrue(page("post").waitForNonExistence(timeout: 5), "the creator left")
    }

    /// Every kind's card is one tablet (Ralph, 8 Nov, #60): no scroll view inside it, no
    /// colour past its border, and scrolling the tablet up hides the dial, down brings it.
    func test_red_everyKindIsOneTablet() {
        launchToShell()
        app.buttons["shell-note"].tap()
        XCTAssertTrue(element("new-kinds").waitForExistence(timeout: 10), "the dial")
        assertInView("note")

        for (i, kind) in Self.ring.enumerated() {
            // Round the ring by the card, one kind a swipe.
            if i > 0 {
                page(Self.ring[i - 1]).swipeLeft()
                assertInView(kind)
            }
            Thread.sleep(forTimeInterval: 0.4)   // the page at rest, nothing mid-slide
            let card = page(kind)
            XCTAssertEqual(card.descendants(matching: .scrollView).count, 0, "nothing scrolls inside \(kind)'s card")
            XCTAssertTrue(tablet(kind).exists, "\(kind)'s card is on a tablet")
            assertNoColourPast(card.frame, kind)
        }

        // With the keyboard up the page is short, so the card is taller than it: the tablet
        // scrolls. A field focused is not an edit, so the dial stays. Event is two kinds on
        // from Note: past Post comes Note.
        page("post").swipeLeft()
        assertInView("note")
        page("note").swipeLeft()
        page("group").swipeLeft()
        assertInView("event")
        let name = page("event").descendants(matching: .any).matching(identifier: "new-field-name").firstMatch
        name.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5), "the keyboard")
        XCTAssertTrue(element("new-kinds").exists, "a focused field is not an edit")
        XCTAssertGreaterThan(page("event").frame.height, tablet("event").frame.height, "the card is taller than its page")

        tablet("event").swipeUp()
        XCTAssertTrue(element("new-kinds").waitForNonExistence(timeout: 3), "scrolling the tablet up hides the dial")
        shoot("tablet scrolled")
        tablet("event").swipeDown()
        tablet("event").swipeDown()
        XCTAssertTrue(element("new-kinds").waitForExistence(timeout: 3), "back down brings the dial back")
        XCTAssertGreaterThanOrEqual(page("event").frame.minY, tablet("event").frame.minY, "the card's top on its tablet again")
    }

    // MARK: helpers

    /// One kind's tablet: the one on screen, as `page` finds its card.
    private func tablet(_ kind: String) -> XCUIElement {
        let all = app.descendants(matching: .any).matching(identifier: "new-tablet-\(kind)")
        return all.allElementsBoundByIndex.first(where: \.isHittable) ?? all.firstMatch
    }

    /// Nothing of the card's colour lies past its border: just outside its right edge (and
    /// its bottom, where that is on screen) the screen reads as it does a few points further
    /// out. A hard shadow 3 pt right and down was the spill.
    private func assertNoColourPast(_ card: CGRect, _ kind: String) {
        let shot = XCUIScreen.main.screenshot().image
        let screen = shot.size
        var probes: [(CGPoint, CGPoint)] = [0.25, 0.5, 0.75].map { f in
            let y = card.minY + card.height * f
            return (CGPoint(x: card.maxX + 1.5, y: y), CGPoint(x: card.maxX + 7, y: y))
        }.filter { $0.0.y > 0 && $0.0.y < screen.height }
        if card.maxY + 8 < screen.height {
            probes += [0.25, 0.5, 0.75].map { f in
                let x = card.minX + card.width * f
                return (CGPoint(x: x, y: card.maxY + 1.5), CGPoint(x: x, y: card.maxY + 7))
            }
        }
        for (near, far) in probes {
            let (a, b) = (pixel(shot, near), pixel(shot, far))
            let d = zip(a, b).map { abs($0 - $1) }.max() ?? 0
            XCTAssertLessThan(d, 0.08, "\(kind): colour just past the border at \(near): \(a) against \(b)")
        }
    }

    /// The colour at `p` (points, top-left origin) in a screenshot, as RGB in 0...1.
    private func pixel(_ image: UIImage, _ p: CGPoint) -> [CGFloat] {
        guard let cg = image.cgImage else { return [0, 0, 0] }
        let x = Int(p.x * image.scale), y = Int(p.y * image.scale)
        var px = [UInt8](repeating: 0, count: 4)
        let ctx = CGContext(data: &px, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
                            space: CGColorSpaceCreateDeviceRGB(),
                            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        ctx.draw(cg, in: CGRect(x: -x, y: y - cg.height + 1, width: cg.width, height: cg.height))
        return px.prefix(3).map { CGFloat($0) / 255 }
    }

    /// The kinds in ring order, by their accessibility suffix.
    private static let ring = ["note", "group", "event", "thing", "place", "project", "post"]

    /// The kind the dial has selected.
    private func inView() -> String {
        Self.ring.first { element("new-kind-\($0)").exists && element("new-kind-\($0)").isSelected } ?? ""
    }

    private func element(_ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    /// One kind's card — the one on screen. The pager holds every kind once per lap, and
    /// the laps it has laid out are all in the tree.
    private func page(_ kind: String) -> XCUIElement {
        let all = app.descendants(matching: .any).matching(identifier: "new-surface-\(kind)")
        return all.allElementsBoundByIndex.first(where: \.isHittable) ?? all.firstMatch
    }

    /// The kind the dial comes to rest on: selected and unchanged for half a second.
    private func settled() -> String {
        var last = inView()
        for _ in 0..<20 {
            Thread.sleep(forTimeInterval: 0.5)
            let now = inView()
            if now == last, !now.isEmpty { return now }
            last = now
        }
        return last
    }

    /// Drag the dial by `items` kinds, slowly and held at the end, so it lands where it
    /// was let go rather than where a fling would carry it.
    private func turnDial(by items: Int) {
        let start = element("new-kinds").coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        let end = start.withOffset(CGVector(dx: -68 * CGFloat(items), dy: 0))
        start.press(forDuration: 0.1, thenDragTo: end, withVelocity: .slow, thenHoldForDuration: 0.4)
    }

    /// The kind in view: its dial item is selected and its card is on screen.
    private func assertInView(_ kind: String) {
        expectation(for: NSPredicate(format: "selected == true"), evaluatedWith: element("new-kind-\(kind)"))
        waitForExpectations(timeout: 5)
        XCTAssertTrue(page(kind).isHittable, "\(kind)'s card in view")
    }

    private func assertKeyboardDown(_ when: String) {
        // The keyboard takes its own animation to leave.
        XCTAssertTrue(app.keyboards.firstMatch.waitForNonExistence(timeout: 3), "keyboard down \(when)")
    }

    private func launchToShell() {
        app.launchArguments += ["-pacificNoWake", "-pacificNoMusic", "-pacificNoArt"]
        app.launchEnvironment["PACIFIC_IDENTITY_NAME"] = "QA"
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)

        let welcome = app.textFields["welcome-name"]
        if welcome.waitForExistence(timeout: 3) {
            welcome.tap(); welcome.typeText("QA")
            app.buttons["welcome-begin"].tap()
        }
        XCTAssertTrue(app.buttons["shell-note"].waitForExistence(timeout: 10), "RED disc")
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
