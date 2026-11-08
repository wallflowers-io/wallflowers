//  OnePagerJourneyUITests.swift
//  Beats of the one-pager journey reachable on
//  ONE device. Beats 4–5 need a peer: FleetJourneyUITests.
//
//    1 FEED    tap the EVENT card         -> EVENT
//    2 EVENT   tap HOST · SPACE           -> SPACE
//    3 SPACE   tap CHANNELS -> a channel  -> ROOM
//    6 CHAT    tap ⋯                      -> MENU
//    7 MENU    tap "New Space with…"      -> the compose
//    8 SETUP   RED cancels · GOLD draws up the people · BLUE creates

import XCTest

final class OnePagerJourneyUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    /// The channel EventSeeder hangs on every promoter Space.
    private static let seededChannel = "general"

    /// Where `scripts/three-sim-journey-e2e.sh` leaves the fleet's details.
    static let handshakePath = "/tmp/pacific-journey.env"

    /// `key=value` lines, or empty when no fleet has been stood up.
    private static func fleetHandshake() -> [String: String] {
        guard let text = try? String(contentsOfFile: handshakePath, encoding: .utf8) else { return [:] }
        var out: [String: String] = [:]
        for line in text.split(separator: "\n") {
            let parts = line.split(separator: "=", maxSplits: 1)
            if parts.count == 2 { out[String(parts[0])] = String(parts[1]) }
        }
        return out
    }

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow"]) ?? false
        }
    }

    // MARK: - Beats 1 → 3 (LIFE)

    func test_beats1to3_feedToEventToSpaceToRoom() {
        launchToLife()
        XCTAssertTrue(waitGround("life-feed", timeout: 15), "beat 1 — the FEED ground")

        let card = firstFeedCard(ofKind: "event", timeout: 15)
        XCTAssertNotNil(card, "beat 1 — an EVENT card on the feed (EventSeeder's real mints)")
        shoot("beat-1-feed")
        card.map { tapCentre($0) }

        let host = app.buttons["event-host-space"]
        XCTAssertTrue(host.waitForExistence(timeout: 10),
                      "beat 2 — HOST · SPACE renders, so a Space holds a `created` edge to this event")
        shoot("beat-2-event")
        tapCentre(host)

        let channels = app.buttons["group-jewel-channels"]
        XCTAssertTrue(channels.waitForExistence(timeout: 10), "beat 3 — the SPACE's CHANNELS jewel")
        shoot("beat-3-space")
        tapCentre(channels)

        XCTAssertNotNil(firstMatching("group-forum-", timeout: 10),
                        "beat 3 — the Space carries a channel (a real Forum GroupObject)")
        // The composer, not the tab band: the band is conditional on `!rooms.isEmpty || isOwner`.
        let inRoom = openSeededChannel() || app.navigationBars[Self.seededChannel].exists
        if !inRoom { shoot("beat-3-MISS"); print("ROOM-MISS-TREE\n\(app.debugDescription)") }
        XCTAssertTrue(inRoom, "beat 3 lands in the ROOM — the channel's own thread")
        shoot("beat-3-room")
    }

    // MARK: - Beats 6 → 8 (CHAT)

    func test_beats6to8_chatMenuToNewSpaceCompose() {
        openAGroupChat()

        let dots = app.buttons["Conversation options"]
        XCTAssertTrue(dots.waitForExistence(timeout: 10), "beat 6 — the ⋯ menu exists on the chat")
        shoot("beat-6-chat")
        dots.tap()

        let newSpace = app.buttons["chat-new-space"]
        XCTAssertTrue(newSpace.waitForExistence(timeout: 5), "beat 7 — the MENU offers New Space with…")
        shoot("beat-7-menu")
        newSpace.tap()

        XCTAssertTrue(app.textFields["new-space-name"].waitForExistence(timeout: 10),
                      "beat 8 — the New Space compose is up")
        XCTAssertTrue(app.buttons["compose-cancel"].exists, "beat 8 — RED cancels")
        XCTAssertTrue(app.buttons["compose-share"].exists,  "beat 8 — GOLD draws up the people")
        XCTAssertTrue(app.buttons["compose-commit"].exists, "beat 8 — BLUE creates")
        shoot("beat-8-compose")
    }

    /// BLUE only lights once the Space has a name — the compose's one gate.
    func test_beat8_blueGatesOnAName_andCreatesTheSpace() {
        openNewSpaceCompose()

        let commit = app.buttons["compose-commit"]
        XCTAssertFalse(commit.isEnabled, "BLUE is dead until the Space is named")

        let name = app.textFields["new-space-name"]
        name.tap()
        let spaceName = "Rooftop \(Int(Date().timeIntervalSince1970) % 100000)"
        name.typeText(spaceName)

        XCTAssertTrue(commit.isEnabled, "a named Space can be created")
        shoot("beat-8-named")
        commit.tap()

        // The Space it made takes the screen — the compose is gone.
        XCTAssertTrue(waitGone(app.textFields["new-space-name"], timeout: 12),
                      "BLUE creates and the compose leaves")
        shoot("beat-8-created")
    }

    /// RED bins it. An untouched compose goes quietly; a started one warns first.
    func test_beat8_redCancels() {
        openNewSpaceCompose()

        // Started — RED must ask before destroying it.
        let name = app.textFields["new-space-name"]
        name.tap(); name.typeText("Throwaway")
        app.buttons["compose-cancel"].tap()

        let discard = app.buttons["Discard"]
        XCTAssertTrue(discard.waitForExistence(timeout: 5),
                      "RED warns before binning a compose you have started")
        shoot("beat-8-discard-warning")
        discard.tap()

        XCTAssertTrue(waitGone(app.textFields["new-space-name"], timeout: 10),
                      "RED cancels the new Space")
    }

    /// GOLD draws the people list UP over the card. On a group of one there is nobody
    /// to add, and gold says so by staying dark rather than raising an empty list.
    func test_beat8_goldDrawsUpThePeopleList() {
        openNewSpaceCompose()

        let gold = app.buttons["compose-share"]
        XCTAssertTrue(gold.exists, "GOLD is on the compose")

        if gold.isEnabled {
            gold.tap()
            XCTAssertTrue(app.otherElements["new-space-people"].waitForExistence(timeout: 5),
                          "GOLD draws up the list of people to add")
            shoot("beat-8-people-up")
            XCTAssertTrue(app.buttons["new-space-member"].firstMatch.exists,
                          "the drawn-up list carries the chat's people")
        } else {
            // A group of one: nobody to carry across. The honest-emptiness rule —
            // a button that can never fire is worse than a dark one.
            XCTAssertEqual(app.staticTexts["new-space-members-summary"].label, "No one to add yet",
                           "with no candidates the card says so and GOLD stays dark")
            shoot("beat-8-no-people")
        }
    }

    // MARK: - ALL EIGHT, on a prepared fleet

    /// Beats 1→8 on a world built by three real identities (scripts/fleet-journey.sh).
    /// Alice walks it; Bob supplies the post beat 4 taps and the peer beats 5 and 8 need.
    /// Seeds nothing — no `-pacificSeedEvents`.
    func test_beats1to8_fullJourney_onAPreparedFleet() throws {
        // A host file, not an env var: `TEST_RUNNER_*` does not reach this process here
        // (probed — no PACIFIC_* keys arrive). A sim process reads the host's /tmp.
        let handshake = Self.fleetHandshake()
        try XCTSkipIf(handshake["peer"] == nil,
                      "fleet-only: run scripts/three-sim-journey-e2e.sh, which prepares the "
                      + "world on three sims and writes \(Self.handshakePath).")

        app.launchArguments += ["-pacificNoWake", "-pacificNoMusic", "-pacificNoArt"]
        // The SAME relay the fleet was built on — beat 8's BLUE adds a real member,
        // which is a real MLS Add and has to reach it.
        if let relay = handshake["relay"] { app.launchEnvironment["PACIFIC_RELAY_URL"] = relay }
        app.launch()
        answerLocationPromptIfAny()
        dismissWelcomeIfUp()
        tapDisc("shell-life")

        XCTAssertTrue(waitGround("life-feed", timeout: 20), "beat 1 — the FEED ground")
        let card = firstFeedCard(ofKind: "event", timeout: 20)
        XCTAssertNotNil(card, "beat 1 — the fleet's minted EVENT reached the feed")
        shoot("fleet-beat-1-feed")
        card.map { tapCentre($0) }

        let host = app.buttons["event-host-space"]
        XCTAssertTrue(host.waitForExistence(timeout: 15), "beat 2 — HOST · SPACE")
        shoot("fleet-beat-2-event")
        tapCentre(host)

        let channels = app.buttons["group-jewel-channels"]
        XCTAssertTrue(channels.waitForExistence(timeout: 15), "beat 3 — the CHANNELS jewel")
        shoot("fleet-beat-3-space")
        tapCentre(channels)
        XCTAssertTrue(openSeededChannel(), "beat 3 — the ROOM")
        shoot("fleet-beat-3-room")

        // BEAT 4 — a post's author. THE fleet assertion: this element only renders for
        // a post that is not mine, so its presence proves Bob's delta folded in here.
        let author = app.buttons["post-author"].firstMatch
        XCTAssertTrue(author.waitForExistence(timeout: 20),
                      "beat 4 — a post by SOMEONE ELSE is in the thread (Bob's, over MLS)")
        shoot("fleet-beat-4-room-peer-post")
        author.tap()
        XCTAssertTrue(app.otherElements["profile-stack"].waitForExistence(timeout: 15)
                      || app.otherElements["profile-headline"].exists,
                      "beat 4 lands on the PERSON page")
        shoot("fleet-beat-5-person")

        let dm = app.buttons["profile-circle-bubble.left.fill"]
        XCTAssertTrue(dm.waitForExistence(timeout: 10), "beat 5 — the gold DM circle")
        tapCentre(dm)
        XCTAssertTrue(app.textFields["composer-field"].waitForExistence(timeout: 15),
                      "beat 5 — the CHAT with that person")
        shoot("fleet-beat-6-chat")

        let dots = app.buttons["Conversation options"]
        XCTAssertTrue(dots.waitForExistence(timeout: 10), "beat 6 — the ⋯ menu")
        dots.tap()

        let newSpace = app.buttons["chat-new-space"]
        XCTAssertTrue(newSpace.waitForExistence(timeout: 8), "beat 7 — New Space with…")
        shoot("fleet-beat-7-menu")
        newSpace.tap()

        // BEAT 8 — the three circles, with GOLD finally live: a DM's candidate is the
        // person you are talking to, so the list has somebody in it.
        XCTAssertTrue(app.textFields["new-space-name"].waitForExistence(timeout: 12),
                      "beat 8 — the compose")
        XCTAssertTrue(app.buttons["compose-cancel"].exists, "RED cancels")
        let gold = app.buttons["compose-share"]
        XCTAssertTrue(gold.exists, "GOLD draws up the people")
        XCTAssertTrue(gold.isEnabled, "GOLD is LIVE — this chat has someone to carry across")
        shoot("fleet-beat-8-compose")

        gold.tap()
        XCTAssertTrue(app.otherElements["new-space-people"].waitForExistence(timeout: 8),
                      "GOLD draws the list up")
        XCTAssertTrue(app.buttons["new-space-member"].firstMatch.exists,
                      "the list carries the DM's peer")
        shoot("fleet-beat-8-people-up")
        // Put it back down with the same disc that raised it, and commit — so the beat
        // ends where the diagram ends: a Space.
        gold.tap()
        usleep(800_000)                      // let the drawer clear before aiming at the field

        type("Rooftop crew", into: app.textFields["new-space-name"])
        let commit = app.buttons["compose-commit"]
        XCTAssertTrue(commit.isEnabled, "BLUE lights once it is named")
        shoot("fleet-beat-8-named")
        commit.tap()
        XCTAssertTrue(waitGone(app.textFields["new-space-name"], timeout: 20),
                      "beat 8 — BLUE creates the Space and the compose leaves")
        shoot("fleet-beat-8-created")
    }

    // MARK: - Entries

    private func launchToLife() {
        app.launchArguments += ["-pacificSeedEvents", "-pacificNoWake",
                                "-pacificNoMusic", "-pacificNoArt"]
        app.launchEnvironment["PACIFIC_IDENTITY_NAME"] = "QA"
        app.launch()
        answerLocationPromptIfAny()
        dismissWelcomeIfUp()

        tapDisc("shell-life")
    }

    /// A real group, constituted through the real compose flow (＋ → New group →
    /// name → Create), then opened. Idempotent: a previous run's group is reused.
    private func openAGroupChat() {
        app.launchArguments += ["-pacificNoWake", "-pacificNoMusic", "-pacificNoArt"]
        app.launchEnvironment["PACIFIC_IDENTITY_NAME"] = "QA"
        app.launch()
        answerLocationPromptIfAny()
        dismissWelcomeIfUp()

        tapDisc("shell-chat")

        let room = "QA Journey"
        if app.staticTexts[room].waitForExistence(timeout: 4) {
            app.staticTexts[room].tap()
        } else {
            app.buttons["New chat"].tap()
            let newGroup = app.buttons["New group"]
            XCTAssertTrue(newGroup.waitForExistence(timeout: 8), "the compose sheet")
            newGroup.tap()
            // BY IDENTIFIER, never `textFields.firstMatch` — the chat list's own
            // Search field is also on screen and firstMatch picks IT, which then
            // fails to focus because the sheet is over it.
            let field = app.textFields["group-name-field"]
            XCTAssertTrue(field.waitForExistence(timeout: 8), "the group's name field")
            field.tap(); field.typeText(room)
            app.buttons["Create"].tap()
        }
        XCTAssertTrue(app.textFields["composer-field"].waitForExistence(timeout: 15),
                      "the chat's composer is up — we are inside the conversation")
    }

    private func openNewSpaceCompose() {
        openAGroupChat()
        app.buttons["Conversation options"].tap()
        let newSpace = app.buttons["chat-new-space"]
        XCTAssertTrue(newSpace.waitForExistence(timeout: 5), "New Space with…")
        newSpace.tap()
        XCTAssertTrue(app.textFields["new-space-name"].waitForExistence(timeout: 10),
                      "the New Space compose")
    }

    private func dismissWelcomeIfUp() {
        let welcome = app.textFields["welcome-name"]
        if welcome.waitForExistence(timeout: 3) {
            welcome.tap(); welcome.typeText("QA")
            app.buttons["welcome-begin"].tap()
        }
    }

    // MARK: - Helpers

    /// The first feed card of a kind. The card is ONE combined element, which the OS
    /// may expose as otherElement OR staticText — never a broad `.any` sweep of the
    /// LIFE hierarchy (FeedUITests' note: that snapshot can take the runner down).
    private func firstFeedCard(ofKind kind: String, timeout: TimeInterval) -> XCUIElement? {
        firstMatching("feed-row-\(kind):", timeout: timeout)
    }

    private func firstMatching(_ prefix: String, timeout: TimeInterval) -> XCUIElement? {
        let match = NSPredicate(format: "identifier BEGINSWITH %@", prefix)
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            for q in [app.buttons, app.otherElements, app.staticTexts] {
                let hit = q.matching(match).firstMatch
                if hit.exists { return hit }
            }
            usleep(400_000)
        } while Date() < deadline
        return nil
    }

    /// The location prompt, answered ONCE and cheaply. Probing three labels with a
    /// 4s wait each cost ~12s of every launch — and after the first run the
    /// permission is already recorded, so all of it was spent waiting for an alert
    /// that never comes again. Five launches of that is most of a test suite's
    /// budget, and it was enough to push the runner into a timeout.
    private func answerLocationPromptIfAny() {
        let allow = springboard.buttons["Allow"]
        if allow.waitForExistence(timeout: 2) { allow.tap(); return }
        _ = tapSpringboard(["Don’t Allow", "Don't Allow"])
    }

    /// Tap a shell disc, retrying once. A system-interruption (the location alert
    /// this suite's monitor answers) can invalidate the element BETWEEN resolving it
    /// and tapping it — "no longer valid after interruption handling", which XCTest
    /// itself prescribes a retry loop for. The discs are the one tap that happens
    /// while an alert may still be in flight, right after launch.
    private func tapDisc(_ id: String) {
        let disc = app.buttons[id]
        XCTAssertTrue(disc.waitForExistence(timeout: 15), "the \(id) disc")
        guard !disc.isSelected else { return }
        disc.tap()
        if !disc.isSelected, disc.exists { disc.tap() }
    }

    /// Open the channel and land in its thread, RETRYING the tap. The channel bar is
    /// the flakiest hit area on this journey: the CHANNELS jewel re-lays the ground
    /// under it, and the row is a NavigationLink whose identified element is its inner
    /// label, so a tap can resolve against a frame the row has already left. Retrying
    /// the tap is honest — a finger does the same — where a longer sleep only moves
    /// the race. Returns whether the ROOM is up.
    ///
    /// Each attempt aims DIFFERENTLY, because no single strategy is reliable here and
    /// the two that exist fail on opposite runs: the row is a Button whose identifier
    /// is its inner label's, tripled by inheritance
    /// (`group-forum-<id>-group-forum-<id>-group-forum-<id>`), and it appears TWICE —
    /// once pinned above the jewels, once in the CHANNELS lane. A finger would just
    /// hit it again slightly differently, so that is what this does.
    @discardableResult
    private func openSeededChannel(attempts: Int = 5) -> Bool {
        let composer = app.textFields["composer-field"]
        let rows = app.buttons.matching(NSCompoundPredicate(andPredicateWithSubpredicates: [
            NSPredicate(format: "identifier BEGINSWITH 'group-forum-'"),
            NSPredicate(format: "NOT (identifier CONTAINS 'forum-new')"),
        ]))
        for attempt in 0..<attempts {
            usleep(1_200_000)                 // let the lane's spring settle before aiming
            guard rows.firstMatch.waitForExistence(timeout: 8) else { return false }
            let lane = rows.count - 1          // the CHANNELS lane's own copy of the row
            switch attempt {
            case 0:  app.staticTexts[Self.seededChannel].firstMatch.tap()
            case 1:  tapCentre(rows.firstMatch)                        // the pinned bar
            case 2:  tapCentre(rows.element(boundBy: max(0, lane)))    // the lane row
            // A short PRESS, not a tap. SwiftUI's row links can swallow a synthesised
            // tap whose down and up land in the same event batch — the gesture never
            // resolves as a tap on the link — while a press with real dwell between
            // them does. A finger always has dwell; only the synthetic event does not.
            case 3:  rows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
                        .press(forDuration: 0.09)
            default: rows.element(boundBy: max(0, lane))
                        .coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
                        .press(forDuration: 0.09)
            }
            if composer.waitForExistence(timeout: 6) { return true }
        }
        if !composer.exists {
            shoot("channel-tap-MISS")
            print("CHANNEL-MISS-TREE\n\(app.debugDescription)")
        }
        return composer.exists
    }

    /// Tap a field and type, CONFIRMING the keyboard actually came up first. A tap
    /// dispatched while a card is still settling focuses nothing, and `typeText` then
    /// fails outright ("Neither element nor any descendant has keyboard focus") rather
    /// than waiting — so the tap is the thing to retry, not the typing.
    private func type(_ text: String, into field: XCUIElement) {
        for _ in 0..<3 {
            field.tap()
            if app.keyboards.element.waitForExistence(timeout: 3) { break }
            usleep(400_000)
        }
        field.typeText(text)
    }

    /// What a finger does. See the note at beat 2.
    private func tapCentre(_ element: XCUIElement) {
        element.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    }

    private func waitGone(_ element: XCUIElement, timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if !element.exists { return true }
            usleep(300_000)
        }
        return !element.exists
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
