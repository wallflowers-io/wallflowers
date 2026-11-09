//  ShellChromeUITests.swift
//  The shell chrome contract, asserted on the REAL app (no fixtures):
//   • the Chats/Life top bar shows a plain `+` (no tab-name text) and a SEPARATE
//     notification bell — two distinct, tappable buttons;
//   • the bottom row is three ICON tabs (Note · Chat · Life);
//   • the bell opens the CONVERGENCE deck (the proposals surface — the old
//     NotificationTray is bell-unreachable, FEED-TRANSMISSION §9-A).

import XCTest

final class ShellChromeUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"]) ?? false
        }
    }

    func test_chrome_separateButtons_iconTabs_andTrayOpens() {
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)
        goToChatsList()

        // Two SEPARATE toolbar buttons: the plain `+` and the notification bell.
        let plus = app.buttons["tab-new"]
        let bell = app.buttons["tab-notifications"]
        XCTAssertTrue(plus.waitForExistence(timeout: 10), "the plain + should be in the Chats top bar")
        XCTAssertTrue(bell.exists, "a SEPARATE notification bell should sit beside the +")

        // The three icon tabs (identifiers are unchanged by the text→icon swap).
        XCTAssertTrue(app.buttons["shell-note"].exists, "NOTE tab")
        XCTAssertTrue(app.buttons["shell-chat"].exists, "CHAT tab")
        XCTAssertTrue(app.buttons["shell-life"].exists, "LIFE tab")

        // The bell opens the CONVERGENCE deck now (the proposals surface — the old
        // NotificationTray is bell-unreachable, FEED-TRANSMISSION §9-A). The deck
        // wears TWO honest faces: cards + the X (convergence-close) when proposals
        // wait, "All caught up" + Done when none do — which face is a fact about
        // the DATA, so the chrome test accepts either and leaves by it.
        bell.tap()
        let close = app.buttons["convergence-close"]
        let caughtUp = app.staticTexts["All caught up"]
        let deadline = Date().addingTimeInterval(5)
        while Date() < deadline && !close.exists && !caughtUp.exists {
            RunLoop.current.run(until: Date().addingTimeInterval(0.3))
        }
        XCTAssertTrue(close.exists || caughtUp.exists,
                      "tapping the bell should open the convergence deck")
        (close.exists ? close : app.buttons["Done"]).tap()

        // LIFE's bar is the SEARCH layout (9 Nov): | account · search bell |, no `+` —
        // RED's creator makes things.
        app.buttons["shell-life"].tap()
        let search = app.buttons["tab-search"]
        XCTAssertTrue(search.waitForExistence(timeout: 5), "Life should show search")
        let lifeBell = app.buttons["tab-notifications"]
        XCTAssertTrue(lifeBell.exists, "Life should show the notification bell")
        XCTAssertFalse(app.buttons["tab-new"].exists, "Life has no +")
        let account = app.buttons["account-button"]
        XCTAssertTrue(account.exists, "Life should show the account")
        XCTAssertLessThan(account.frame.midX, search.frame.midX, "the account sits west")
        XCTAssertLessThan(search.frame.midX, lifeBell.frame.midX, "search sits west of the bell")
    }

    /// The NOTE disc carries BOTH halves of notes on one button: a tap opens the compose
    /// modal, a HOLD swaps the screen to the Notes page. The hold must also swallow the
    /// tap iOS delivers when the finger lifts — otherwise one press would do both.
    func test_noteDisc_holdOpensNotesPage_tapComposes() {
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)
        goToChatsList()

        let note = app.buttons["shell-note"]
        XCTAssertTrue(note.waitForExistence(timeout: 10), "NOTE disc")

        // HOLD → the Notes PAGE: the disc lights up as the shown screen, and the page
        // carries the same top bar as CHAT and LIFE — the universal `+` and the bell.
        // Never the compose card.
        note.press(forDuration: 0.9)
        let onNotesPage = NSPredicate(format: "selected == true")
        expectation(for: onNotesPage, evaluatedWith: note)
        waitForExpectations(timeout: 5)
        XCTAssertFalse(app.textFields["Title"].exists,
                       "the release after a hold must NOT also open the compose card")
        XCTAssertTrue(app.buttons["tab-new"].waitForExistence(timeout: 5), "Notes shows the +")
        XCTAssertTrue(app.buttons["tab-notifications"].exists, "Notes shows the bell")

        // TAP → the compose card, unaffected by the preceding hold. The three discs are
        // that card's buttons: the SAME views (same identifiers) carrying new labels,
        // which is what lets them slide into place rather than being swapped out.
        note.tap()
        XCTAssertTrue(app.textFields["Title"].waitForExistence(timeout: 5),
                      "tapping NOTE should open the compose card")
        XCTAssertEqual(app.buttons["shell-note"].label, "Cancel")
        XCTAssertEqual(app.buttons["shell-chat"].label, "Share")
        XCTAssertEqual(app.buttons["shell-life"].label, "Send")

        // X dismisses, and the discs reform NOTE · CHAT · LIFE.
        app.buttons["shell-note"].tap()
        XCTAssertFalse(app.textFields["Title"].waitForExistence(timeout: 2),
                       "the compose card should be gone")
        XCTAssertEqual(app.buttons["shell-chat"].label, "Chat")
        XCTAssertEqual(app.buttons["shell-life"].label, "Life")

        // The page is a destination like the others — CHAT swaps back off it.
        XCTAssertTrue(app.buttons["shell-chat"].waitForExistence(timeout: 5))
        app.buttons["shell-chat"].tap()
        let chatShowing = NSPredicate(format: "selected == true")
        expectation(for: chatShowing, evaluatedWith: app.buttons["shell-chat"])
        waitForExpectations(timeout: 5)
    }

    /// Opening a conversation hands the screen to it (the disc row leaves, the composer
    /// takes over); leaving hands it back. Leaving is a TWO-PHASE dismiss — the composer
    /// retracts, and only then is the conversation removed — so this guards that the
    /// second phase actually runs: a broken hand-off would strand the conversation on
    /// screen with no disc row to leave by.
    func test_conversation_takesAndReturnsTheScreen() throws {
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)
        goToChatsList()

        let convo = app.buttons.matching(NSPredicate(format: "label CONTAINS[c] 'no messages'")).firstMatch
        try XCTSkipUnless(convo.waitForExistence(timeout: 10), "no conversation on this device to open")

        convo.tap()
        XCTAssertTrue(app.textFields["composer-field"].waitForExistence(timeout: 5),
                      "the conversation's composer should take over")
        // The row stays in the tree and fades in place (it is covered, not removed), so
        // hittability — not existence — is what says the conversation owns the screen.
        XCTAssertFalse(app.buttons["shell-chat"].isHittable,
                       "the disc row is covered by the conversation")

        app.buttons["chat-back"].tap()
        let rowBack = NSPredicate(format: "hittable == true")
        expectation(for: rowBack, evaluatedWith: app.buttons["shell-chat"])
        waitForExpectations(timeout: 5)
        XCTAssertFalse(app.textFields["composer-field"].exists,
                       "the conversation is gone, not stranded on top of the row")
    }

    /// A note you have written into can only be thrown away deliberately: the ground
    /// outside it stops dismissing, and the X asks first. An empty one still goes on a
    /// tap anywhere. This is the guard against losing words to a stray tap.
    func test_composeCard_onlyDiscardsWrittenNotesDeliberately() {
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)
        goToChatsList()

        let title = app.textFields["Title"]
        let outside = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.09))

        // EMPTY: a tap on the ground dismisses it.
        app.buttons["shell-note"].tap()
        XCTAssertTrue(title.waitForExistence(timeout: 5))
        outside.tap()
        XCTAssertFalse(title.waitForExistence(timeout: 2),
                       "an empty note should go on a tap outside")

        // EMPTY: the band the disc row sits in is off-canvas too — everywhere but the
        // discs themselves. Taken 2.5 disc-widths right of the X, i.e. in the gap.
        app.buttons["shell-note"].tap()
        XCTAssertTrue(title.waitForExistence(timeout: 5))
        app.buttons["shell-note"]
            .coordinate(withNormalizedOffset: CGVector(dx: 2.5, dy: 0.5)).tap()
        XCTAssertFalse(title.waitForExistence(timeout: 2),
                       "a tap between the discs is a tap off the canvas")

        // WRITTEN: the same tap does nothing at all.
        app.buttons["shell-note"].tap()
        XCTAssertTrue(title.waitForExistence(timeout: 5))
        app.typeText("Worth keeping")
        outside.tap()
        XCTAssertTrue(title.exists, "a written note must survive a tap outside")

        // …and the X warns before it destroys anything.
        app.buttons["shell-note"].tap()
        XCTAssertTrue(app.staticTexts["Discard this note?"].waitForExistence(timeout: 5),
                      "the X should warn on a written note")

        // Backing out of the warning keeps the note, text and all. Dismissed by a tap
        // outside rather than by its cancel button: this OS does not expose the dialog's
        // cancel button to automation (only "Discard" appears in the tree), and a tap
        // outside an action sheet is the same cancel path.
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2)).tap()
        XCTAssertTrue(title.waitForExistence(timeout: 3))
        XCTAssertEqual(title.value as? String, "Worth keeping")

        // Confirming discards it.
        app.buttons["shell-note"].tap()
        XCTAssertTrue(app.buttons["Discard"].waitForExistence(timeout: 5))
        app.buttons["Discard"].tap()
        XCTAssertFalse(title.waitForExistence(timeout: 3), "confirmed discard should dismiss")
    }

    /// Share on a written note opens the PLACE PUBLISH sheet — ours, not the system
    /// share sheet — and backing out of it leaves the compose card exactly as it was,
    /// words and all. (Actually publishing needs a Place and is proven end-to-end in
    /// core's m11_discovery; the chrome contract here is the sheet and the safe exit.)
    func test_share_opensPlacePublishSheet_andCancelKeepsTheNote() {
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)
        goToChatsList()

        let title = app.textFields["Title"]
        app.buttons["shell-note"].tap()
        XCTAssertTrue(title.waitForExistence(timeout: 5))
        app.typeText("Publish me somewhere")

        app.buttons["shell-chat"].tap()   // dressed "Share" while composing
        XCTAssertTrue(app.navigationBars["Publish"].waitForExistence(timeout: 5),
                      "Share should open the Place publish sheet, not the system sheet")

        // Scoped to the sheet's bar: the NOTE disc is ALSO labelled "Cancel" while
        // composing, so the bare query matches twice.
        app.navigationBars["Publish"].buttons["Cancel"].tap()
        XCTAssertTrue(title.waitForExistence(timeout: 3),
                      "backing out of the sheet must leave the compose card up")
        XCTAssertEqual(title.value as? String, "Publish me somewhere",
                       "…with the note intact")
        app.buttons["shell-note"].tap()   // X → warns (written note)
        XCTAssertTrue(app.buttons["Discard"].waitForExistence(timeout: 5))
        app.buttons["Discard"].tap()      // leave the device clean for the next test
    }

    /// The three screens sit side by side in disc order, so a horizontal swipe moves
    /// between them — the same movement the discs make, driven by the finger.
    func test_screens_swipeBetweenNoteChatLife() {
        app.launch()
        _ = tapSpringboard(["Allow", "Don’t Allow", "Don't Allow"], waitFor: 4)
        goToChatsList()

        let note = app.buttons["shell-note"], chat = app.buttons["shell-chat"], life = app.buttons["shell-life"]
        func onScreen(_ disc: XCUIElement, _ what: String) {
            expectation(for: NSPredicate(format: "selected == true"), evaluatedWith: disc)
            waitForExpectations(timeout: 5)
            XCTAssertTrue(disc.isSelected, what)
        }
        // Swipe across a band clear of the list rows: on NOTE they carry swipe-to-delete,
        // which owns a horizontal drag that starts on a row.
        func swipe(_ from: CGFloat, _ to: CGFloat) {
            app.coordinate(withNormalizedOffset: CGVector(dx: from, dy: 0.16))
                .press(forDuration: 0.05,
                       thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: to, dy: 0.16)))
        }

        onScreen(chat, "opens on CHAT")
        swipe(0.85, 0.12)
        onScreen(life, "swiping left goes to LIFE")
        swipe(0.12, 0.85)
        onScreen(chat, "and back to CHAT")
        swipe(0.12, 0.85)
        onScreen(note, "swiping right again reaches NOTE")
    }

    // MARK: - Helpers

    /// If a prior session restored a pushed conversation, pop back to the list root so the
    /// tab chrome (`tab-new`) is on screen.
    private func goToChatsList() {
        // LIFE's bar has no `+` (9 Nov); a launch that reopens on LIFE goes to CHAT first.
        if app.buttons["tab-search"].waitForExistence(timeout: 3) { app.buttons["shell-chat"].tap() }
        if app.buttons["tab-new"].waitForExistence(timeout: 3) { return }
        let back = app.navigationBars.buttons.element(boundBy: 0)
        if back.exists { back.tap() }
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
