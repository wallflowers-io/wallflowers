//  LogTests.swift
//  The device's log file, and the sign-in progress line read from the same log.

import XCTest
@testable import WallFlowers

final class LogFileTests: XCTestCase {
    private func temp() -> URL {
        FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString)
            .appendingPathComponent("wallflowers.log")
    }

    func testLinesLandInOrderEachWithItsTime() throws {
        let log = LogFile(url: temp())
        log.append("first")
        log.append("second")
        log.flush()
        let lines = try String(contentsOf: log.url, encoding: .utf8).split(separator: "\n")
        XCTAssertEqual(lines.count, 2)
        XCTAssertTrue(lines[0].hasSuffix(" first"), String(lines[0]))
        XCTAssertTrue(lines[1].hasSuffix(" second"), String(lines[1]))
        XCTAssertNotNil(ISO8601DateFormatter.withFraction.date(from: String(lines[0].split(separator: " ")[0])))
    }

    func testAtTheCapItRollsAndKeepsOneOld() throws {
        let log = LogFile(url: temp(), cap: 64)
        for i in 0..<10 { log.append("line \(i) padding padding") }
        log.flush()
        let old = log.url.appendingPathExtension("1")
        XCTAssertTrue(FileManager.default.fileExists(atPath: old.path), "rolled")
        let size = try FileManager.default.attributesOfItem(atPath: log.url.path)[.size] as! Int
        XCTAssertLessThan(size, 64 + 64, "the live file starts again after the roll")
        let last = try String(contentsOf: log.url, encoding: .utf8)
        XCTAssertTrue(last.contains("line 9"), last)
    }
}

@MainActor
final class SignInProgressTests: XCTestCase {
    func testProgressFollowsResumeLinesOnlyWhileBusy() {
        let signIn = CoreService().makeSignIn()
        Trace.shared.log("core:pacific::resume", "1/3 group ab12cd34: AlreadyHeld")
        XCTAssertNil(signIn.progress, "not busy: nothing to show")

        signIn.busy = "passkey"
        Trace.shared.log("core:pacific::resume", "2/3 forum 0011aabb: Joined { from_epoch: 4 }")
        XCTAssertEqual(signIn.progress, "2/3 forum 0011aabb: Joined { from_epoch: 4 }")
        Trace.shared.log("account", "an unrelated line")
        XCTAssertEqual(signIn.progress, "2/3 forum 0011aabb: Joined { from_epoch: 4 }", "only resume lines")

        signIn.busy = nil
        XCTAssertNil(signIn.progress, "cleared when the act ends")
    }
}

private extension ISO8601DateFormatter {
    static let withFraction: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()
}
