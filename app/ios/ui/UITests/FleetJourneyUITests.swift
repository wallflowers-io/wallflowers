//  FleetJourneyUITests.swift
//  THE FLEET, driven only by REAL TAPS. Every step below is a finger on a surface:
//  the first-boot doors, the connect code, the Space, the channel, the post, and the
//  eight beats of the one-pager journey. Nothing is set up through the `pacific://`
//  dev seam — a world built behind the interface proves the core calls work, which
//  was never in question, and proves nothing about the screens a person touches.
//
//  ONE DEVICE PER RUN. XCUITest drives a single target, so the fleet is a SEQUENCE of
//  runs — `scripts/fleet-journey.sh` picks the device and the step, and the devices
//  hand off to each other through the world they are actually changing (a Welcome on
//  the relay, a delta in a log). The only thing carried by the host is the connect
//  CODE, because that is the one part a person carries too: you read it off one
//  screen and type it into the other. It is read from the pasteboard that the real
//  "copy code" button just filled.
//
//  ROLES. `Alice` walks the journey. `Bob` publishes the night and writes the post
//  beat 4 taps. `Carol` is the third member, so the channel is a room and not a pair.
//
//  Config arrives in /tmp/pacific-fleet.env (`key=value`) — the harness's own
//  parameters (which name to type, which relay to use), never a way to drive the app.

import XCTest
import UIKit

final class FleetJourneyUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    static let configPath = "/tmp/pacific-fleet.env"
    /// Where a device leaves its connect code for the next device's run to type.
    static func codePath(_ who: String) -> String { "/tmp/pacific-code-\(who).txt" }

    private var config: [String: String] = [:]

    /// WHO THIS DEVICE IS, looked up by the device's own name. The three devices run
    /// their first boot AT THE SAME TIME off one shared config, so a run cannot be
    /// handed its role as an argument — and environment variables never arrive
    /// (`TEST_RUNNER_*` reaches this process empty). The simulator's name is the one
    /// thing a run always knows about itself.
    private var me: String {
        config["role.\(UIDevice.current.name)"] ?? config["name"] ?? "Alice"
    }

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        config = Self.readConfig()
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow", "OK", "Don’t Allow", "Don't Allow", "Paste"]) ?? false
        }
    }

    private static func readConfig() -> [String: String] {
        guard let text = try? String(contentsOfFile: configPath, encoding: .utf8) else { return [:] }
        var out: [String: String] = [:]
        for line in text.split(separator: "\n") {
            let p = line.split(separator: "=", maxSplits: 1)
            if p.count == 2 { out[String(p[0])] = String(p[1]) }
        }
        return out
    }

    // MARK: - Step 1 · first boot, by hand

    /// The real first-boot sequence on a fresh install: NEW door → name → face →
    /// bio → recovery key → fingerprint → ENTER. BootFlow's header calls these ids
    /// "part of the contract, not an afterthought (§4)… the fleet drives these
    /// screens through XCUITest" — this is that fleet.
    func test_step1_firstBoot() throws {
        try skipUnlessConfigured()
        launchFresh()

        // DOOR. A fresh install has no identity, so NEW is the only honest way in.
        let new = app.buttons["boot-door-new"]
        XCTAssertTrue(new.waitForExistence(timeout: 30), "the NEW door on a fresh install")
        shoot("\(me)-01-door")
        new.tap()

        // NAME — the one field that is genuinely this person's.
        let name = app.textFields["welcome-name"]
        XCTAssertTrue(name.waitForExistence(timeout: 15), "the name field")
        type(me, into: name)
        shoot("\(me)-02-name")
        advance()

        // FACE, BIO, KEY, FINGERPRINT — walked, not skipped: a harness that jumps
        // the setup is not testing the setup.
        for step in ["03-face", "04-bio", "05-key", "06-fingerprint"] {
            if app.buttons["boot-enter"].exists { break }
            shoot("\(me)-\(step)")
            advance()
        }
        if app.buttons["boot-enter"].exists {
            shoot("\(me)-07-enter")
            app.buttons["boot-enter"].tap()
        }

        // We are in: the shell's own discs are the proof.
        XCTAssertTrue(app.buttons["shell-life"].waitForExistence(timeout: 30)
                      || app.buttons["shell-chat"].exists,
                      "first boot completes and the shell is up")
        shoot("\(me)-08-shell")
    }

    /// NEXT / WRITTEN DOWN / ENTER — the boot flow's one forward control, whatever
    /// this step happens to call it.
    private func advance() {
        for id in ["boot-next", "boot-enter", "welcome-begin"] {
            let b = app.buttons[id]
            if b.exists { b.tap(); usleep(700_000); return }
        }
    }

    // MARK: - Step 2 · hold your code out

    /// New chat → "Show my QR code" → "copy code". The bundle lands on the
    /// pasteboard because a REAL BUTTON put it there, and the harness reads it out
    /// the way a person reads it off the screen.
    func test_step2_showMyCode() throws {
        try skipUnlessConfigured()
        launchExisting()

        openNewChat()
        let showQR = app.buttons["Show my QR code"]
        XCTAssertTrue(showQR.waitForExistence(timeout: 15), "Show my QR code")
        showQR.tap()

        let copy = app.buttons["copy code"]
        XCTAssertTrue(copy.waitForExistence(timeout: 15), "the copy-code link under the QR")
        shoot("\(me)-10-my-code")
        copy.tap()
        XCTAssertTrue(app.buttons["copied"].waitForExistence(timeout: 8),
                      "the button confirms it copied")

        let bundle = UIPasteboard.general.string ?? ""
        XCTAssertFalse(bundle.isEmpty, "the real button put a connect code on the pasteboard")
        XCTAssertGreaterThan(bundle.count, 100, "a connect code, not a stray string")
        try bundle.write(toFile: Self.codePath(me), atomically: true, encoding: .utf8)
    }

    // MARK: - Step 3 · type theirs in

    /// New chat → "Enter a code" → type the peer's code → Connect. Typed, not
    /// pasted: this is the one screen where a person really does put the other
    /// device's code in by hand.
    func test_step3_enterPeerCode() throws {
        try skipUnlessConfigured()
        let peer = config["peer"] ?? ""
        let code = (try? String(contentsOfFile: Self.codePath(peer), encoding: .utf8))?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        try XCTSkipIf(code.isEmpty, "no code from \(peer) — run their step 2 first")

        launchExisting()
        openNewChat()
        let enter = app.buttons["Enter a code"]
        XCTAssertTrue(enter.waitForExistence(timeout: 15), "Enter a code")
        enter.tap()

        let field = app.textViews["pair-code-field"].exists
            ? app.textViews["pair-code-field"] : app.textFields["pair-code-field"]
        XCTAssertTrue(field.waitForExistence(timeout: 15), "the code field")
        type(code, into: field)
        shoot("\(me)-11-code-typed")

        let connect = app.buttons["Connect"]
        XCTAssertTrue(connect.waitForExistence(timeout: 8), "Connect")
        XCTAssertTrue(connect.isEnabled, "Connect lights once a code is in")
        connect.tap()
        // The sheet closes when the pairing is away.
        XCTAssertTrue(waitGone(field, timeout: 40), "the pairing is sent")
        shoot("\(me)-12-connected")
    }

    // MARK: - Step 4 · accept them

    /// The consent half, in the UI: open the pending conversation and tap Accept.
    /// Double opt-in is the protocol's rule, so the harness has to honour it with a
    /// finger like everyone else.
    func test_step4_acceptPeer() throws {
        try skipUnlessConfigured()
        let peer = config["peer"] ?? ""
        launchExisting()

        tapDisc("shell-chat")
        let row = app.buttons.containing(
            NSPredicate(format: "label CONTAINS[c] %@", peer)).firstMatch
        if !row.waitForExistence(timeout: 25) {
            openNewChat()                                   // else via the connections list
            let listed = app.buttons.containing(
                NSPredicate(format: "label CONTAINS[c] %@", peer)).firstMatch
            XCTAssertTrue(listed.waitForExistence(timeout: 20), "\(peer) is in the connections")
            listed.tap()
        } else {
            row.tap()
        }

        let accept = app.buttons["Accept"]
        if accept.waitForExistence(timeout: 15) {
            shoot("\(me)-13-accept")
            accept.tap()
        }
        XCTAssertTrue(app.textFields["composer-field"].waitForExistence(timeout: 20),
                      "the conversation with \(peer) is open and live")
        shoot("\(me)-14-conversation")
    }

    // MARK: - Entries

    private func skipUnlessConfigured() throws {
        try XCTSkipIf(config["relay"] == nil,
                      "fleet-only: run scripts/fleet-journey.sh, which writes \(Self.configPath)")
    }

    /// A FRESH install's first launch — no identity env, because the point is to walk
    /// the doors. The relay is the fleet's own.
    private func launchFresh() {
        app.launchArguments += ["-pacificNoWake", "-pacificNoMusic", "-pacificNoArt"]
        if let relay = config["relay"] { app.launchEnvironment["PACIFIC_RELAY_URL"] = relay }
        app.launch()
        answerLocationPromptIfAny()
    }

    private func launchExisting() {
        launchFresh()
        // A booted identity lands on the shell; nothing to dismiss.
    }

    private func openNewChat() {
        tapDisc("shell-chat")
        for id in ["tab-new", "New chat"] {
            let b = app.buttons[id]
            if b.waitForExistence(timeout: 8) { b.tap(); return }
        }
        XCTFail("the New chat control")
    }

    // MARK: - Helpers

    private func tapDisc(_ id: String) {
        let disc = app.buttons[id]
        XCTAssertTrue(disc.waitForExistence(timeout: 30), "the \(id) disc")
        guard !disc.isSelected else { return }
        disc.tap()
        if !disc.isSelected, disc.exists { disc.tap() }
    }

    /// Tap and type, confirming the keyboard came up — a tap into a settling screen
    /// focuses nothing and `typeText` then fails outright rather than waiting.
    private func type(_ text: String, into field: XCUIElement) {
        for _ in 0..<3 {
            field.tap()
            if app.keyboards.element.waitForExistence(timeout: 4) { break }
            usleep(400_000)
        }
        field.typeText(text)
    }

    private func waitGone(_ element: XCUIElement, timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if !element.exists { return true }
            usleep(400_000)
        }
        return !element.exists
    }

    private func answerLocationPromptIfAny() {
        let allow = springboard.buttons["Allow"]
        if allow.waitForExistence(timeout: 3) { allow.tap(); return }
        _ = tapSpringboard(["Don’t Allow", "Don't Allow"])
    }

    @discardableResult
    private func tapSpringboard(_ labels: [String]) -> Bool {
        for label in labels where springboard.buttons[label].exists {
            springboard.buttons[label].tap(); return true
        }
        return false
    }

    private func shoot(_ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }
}
