//  HarnessRunnerUITests.swift
//  The per-device interpreter for the multi-simulator harness (simulators/run.sh).
//  One instance runs per simulator, all executing the SAME globally-ordered step
//  program (steps.json, compiled from the scenario's harness.toml). A shared clock
//  file enforces cross-device ordering: each runner waits for the clock to reach a
//  step's index; the step's owning device executes it and advances the clock; every
//  other runner just watches it pass. Host-owned steps (device = "host": code
//  ferrying between sim pasteboards) are executed by the conductor, which
//  participates in the same clock.
//
//  Everything a step does is a REAL user interaction — accessibility-driven taps,
//  typing, long-presses, swipes. No fixtures, no display shortcuts: four minted
//  identities pairing and working over a real relay.
//
//  Env (TEST_RUNNER_ → this process):
//    HARNESS_PERSONA   this device's persona name (matches step.device)
//    HARNESS_STEPS     absolute path to steps.json
//    HARNESS_CLOCK     absolute path to the shared clock file
//    HARNESS_SHOTS     absolute dir for this persona's screenshots
//    HARNESS_RELAY     ws:// relay URL the app should use
//    HARNESS_IDENTITY  display name minted on first launch

import XCTest

private struct HarnessStep: Decodable {
    let i: Int
    let day: String?
    let device: String
    let action: String
    let target: String?
    let text: String?
    let seconds: Double?
    let timeout: Double?
    let name: String?
    let note: String?
}

final class HarnessRunnerUITests: XCTestCase {

    private var app: XCUIApplication!
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    private var persona = ""
    private var clockURL: URL!
    private var shotsDir = ""

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
    }

    func test_runHarness() throws {
        let env = ProcessInfo.processInfo.environment
        persona = env["HARNESS_PERSONA"] ?? ""
        let stepsPath = env["HARNESS_STEPS"] ?? ""
        clockURL = URL(fileURLWithPath: env["HARNESS_CLOCK"] ?? "")
        shotsDir = env["HARNESS_SHOTS"] ?? NSTemporaryDirectory()
        XCTAssertFalse(persona.isEmpty, "HARNESS_PERSONA is required")
        XCTAssertFalse(stepsPath.isEmpty, "HARNESS_STEPS is required")

        let data = try Data(contentsOf: URL(fileURLWithPath: stepsPath))
        let steps = try JSONDecoder().decode([HarnessStep].self, from: data)
        NSLog("HARNESS[\(persona)] \(steps.count) steps loaded")

        for step in steps {
            try waitForClock(step.i)
            guard step.device == persona else { continue }   // someone else's turn
            NSLog("HARNESS[\(persona)] step \(step.i): \(step.action) \(step.target ?? step.text ?? "")")
            do {
                try perform(step)
            } catch {
                abort(reason: "step \(step.i) [\(persona)] \(step.action) failed: \(error)")
                XCTFail("step \(step.i) (\(step.action) \(step.target ?? "")): \(error)")
                return
            }
            advanceClock(to: step.i + 1)
        }
        NSLog("HARNESS[\(persona)] program complete")
    }

    // MARK: - The clock (shared file barrier)

    private enum HarnessError: Error, CustomStringConvertible {
        case aborted(String)
        case timeout(String)
        case elementNotFound(String)
        case badStep(String)
        var description: String {
            switch self {
            case .aborted(let r): return "aborted: \(r)"
            case .timeout(let r): return "timeout: \(r)"
            case .elementNotFound(let r): return "element not found: \(r)"
            case .badStep(let r): return "bad step: \(r)"
            }
        }
    }

    private func readClock() -> String {
        (try? String(contentsOf: clockURL, encoding: .utf8))?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? "0"
    }

    private func waitForClock(_ index: Int, overallTimeout: TimeInterval = 2400) throws {
        let start = Date()
        while true {
            let v = readClock()
            if v.hasPrefix("ABORT") { throw HarnessError.aborted(v) }
            if let n = Int(v), n >= index { return }
            if Date().timeIntervalSince(start) > overallTimeout {
                abort(reason: "clock stalled before step \(index) (at \(v)) — \(persona) gave up")
                throw HarnessError.timeout("clock stalled at \(v) waiting for \(index)")
            }
            Thread.sleep(forTimeInterval: 0.15)
        }
    }

    private func advanceClock(to index: Int) {
        try? "\(index)".write(to: clockURL, atomically: true, encoding: .utf8)
    }

    private func abort(reason: String) {
        try? "ABORT \(reason)".write(to: clockURL, atomically: true, encoding: .utf8)
    }

    // MARK: - Actions

    private func perform(_ s: HarnessStep) throws {
        switch s.action {
        case "launch":       try doLaunch()
        case "tap":          try resolve(s).tap()
        case "type":         try doType(s)
        case "paste":        try doPaste(s)
        case "long_press":   try resolve(s).press(forDuration: s.seconds ?? 0.7)
        case "swipe_reply":  try doSwipeReply(s)
        case "wait_for":     _ = try await_(s)
        case "wait_absent":  try doWaitAbsent(s)
        case "screenshot":   try doScreenshot(s)
        case "pause":        Thread.sleep(forTimeInterval: s.seconds ?? 1)
        default:             throw HarnessError.badStep("unknown action '\(s.action)'")
        }
    }

    private func doLaunch() throws {
        let env = ProcessInfo.processInfo.environment
        app = XCUIApplication()
        if let relay = env["HARNESS_RELAY"], !relay.isEmpty {
            app.launchEnvironment["PACIFIC_RELAY_URL"] = relay
        }
        if let name = env["HARNESS_IDENTITY"], !name.isEmpty {
            app.launchEnvironment["PACIFIC_IDENTITY_NAME"] = name
        }
        if env["HARNESS_TIMING"] == "1" {
            app.launchEnvironment["PACIFIC_TIMING"] = "1"   // TimingProbe: e2e latency JSONL
        }
        app.launch()
        // First-run notification permission is a SpringBoard alert — clear it. It can
        // also appear a few seconds AFTER launch (later still under multi-sim load), so
        // resolve() clears it defensively on every retry too (see dismissBlockingSystemAlert).
        let allow = springboard.buttons["Allow"]
        if allow.waitForExistence(timeout: 8) { allow.tap() }
    }

    /// Clear a blocking SpringBoard system alert if one is up. The notification-permission
    /// prompt is a SpringBoard alert that can pop seconds after first launch — while it's
    /// up the app's own elements are unhittable, which otherwise races the next tap.
    private func dismissBlockingSystemAlert() {
        for label in ["Allow", "Allow Once", "OK"] {
            let b = springboard.buttons[label]
            if b.exists { b.tap(); return }
        }
    }

    /// Resolve a step's target to the first matching element, trying the query types a
    /// SwiftUI surface actually produces. Matches identifier OR label OR (textViews) value.
    private func resolve(_ s: HarnessStep, timeout: TimeInterval? = nil) throws -> XCUIElement {
        guard let t = s.target else { throw HarnessError.badStep("\(s.action) needs target") }
        let deadline = Date().addingTimeInterval(timeout ?? s.timeout ?? 10)
        let queries: [XCUIElementQuery] = [
            app.buttons, app.staticTexts, app.textFields, app.secureTextFields,
            app.textViews, app.navigationBars, app.tabBars.buttons,
            app.switches, app.cells, app.otherElements,
        ]
        let pred = NSPredicate(format: "identifier == %@ OR label == %@ OR label BEGINSWITH %@", t, t, t)
        let valuePred = NSPredicate(format: "value CONTAINS %@", t)
        while true {
            for q in queries {
                let e = q.matching(pred).firstMatch
                if e.exists { return e }
            }
            let tv = app.textViews.matching(valuePred).firstMatch
            if tv.exists { return tv }
            let any = app.descendants(matching: .any).matching(pred).firstMatch
            if any.exists { return any }
            // A late-appearing system alert (notification permission) blocks the app's
            // elements — clear it and retry so the step isn't lost to the race.
            dismissBlockingSystemAlert()
            if Date() > deadline { throw HarnessError.elementNotFound(t) }
            Thread.sleep(forTimeInterval: 0.25)
        }
    }

    private func doType(_ s: HarnessStep) throws {
        guard let text = s.text else { throw HarnessError.badStep("type needs text") }
        if s.target != nil {
            let field = try resolve(s)
            field.tap()
            field.typeText(text)
        } else {
            app.typeText(text)
        }
    }

    /// Paste the SIM PASTEBOARD (loaded by a preceding host ferry step) into a field
    /// via the real edit menu — the same gesture a human uses for a connect code.
    private func doPaste(_ s: HarnessStep) throws {
        let field = try resolve(s)
        field.tap()
        field.press(forDuration: 1.1)          // summon the edit menu
        let paste = app.menuItems["Paste"].exists ? app.menuItems["Paste"]
                  : app.staticTexts["Paste"].firstMatch
        guard paste.waitForExistence(timeout: 4) else {
            throw HarnessError.elementNotFound("edit-menu Paste")
        }
        paste.tap()
        // Paste is async-ish; give the field a beat to fill before the next step taps Connect.
        Thread.sleep(forTimeInterval: 0.8)
    }

    /// Swipe a message bubble toward centre to begin a reply (incoming → right).
    private func doSwipeReply(_ s: HarnessStep) throws {
        let bubble = try resolve(s)
        if (s.text ?? "right") == "left" { bubble.swipeLeft() } else { bubble.swipeRight() }
    }

    private func await_(_ s: HarnessStep) throws -> XCUIElement {
        try resolve(s, timeout: s.timeout ?? 25)
    }

    private func doWaitAbsent(_ s: HarnessStep) throws {
        let deadline = Date().addingTimeInterval(s.timeout ?? 10)
        while Date() < deadline {
            if (try? resolve(s, timeout: 0.5)) == nil { return }
            Thread.sleep(forTimeInterval: 0.3)
        }
        throw HarnessError.timeout("'\(s.target ?? "?")' still present")
    }

    private func doScreenshot(_ s: HarnessStep) throws {
        let shot = XCUIScreen.main.screenshot()
        let name = s.name ?? "step-\(s.i)"
        let url = URL(fileURLWithPath: shotsDir).appendingPathComponent("\(String(format: "%03d", s.i))-\(name).png")
        try shot.pngRepresentation.write(to: url)
        // Also attach for the xcresult.
        let att = XCTAttachment(screenshot: shot)
        att.name = name
        att.lifetime = .keepAlways
        add(att)
    }
}
