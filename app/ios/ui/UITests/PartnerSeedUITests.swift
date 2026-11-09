//  PartnerSeedUITests.swift
//  The partners' seed (PartnerSeeder, 9 Nov), on the REAL app: every post and night in
//  the pack reaches LIFE's feed WITH its picture, and each partner is a Space.
//
//  The pack is the partners' and never in git, so the titles are read from the local
//  pack (`just seed-partners` builds it and drops it in the sim). No pack is a FAILURE,
//  not a skip: this suite is exactly the check that the pack made it in.
//
//  A card's banner is decorative to VoiceOver (the title says the card aloud), so its
//  presence is read from the pixels: a card with no media is one flat fill under its
//  title; a photograph is not.

import XCTest

final class PartnerSeedUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    private struct Item: Decodable { let title: String }
    private struct Partner: Decodable { let name: String; let posts: [Item]; let events: [Item] }
    private struct Pack: Decodable { let partners: [Partner] }

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
    }

    func test_everySeededPostAndNight_reachesTheFeedWithItsPicture() throws {
        let pack = try loadPack()
        var pending = Set(pack.partners.flatMap { $0.posts.map(\.title) + $0.events.map(\.title) })
        launchToLife()

        let feed = app.scrollViews["life-feed"]
        XCTAssertTrue(feed.waitForExistence(timeout: 10), "the feed ground")
        XCTAssertTrue(poll(60) { self.rows().count > 0 }, "the seed lands on the feed")

        // Top to bottom, ticking off each pack title as its card comes into view. Twice:
        // the first card can start under the top bar, where it is not hittable.
        var bare: [String] = []
        for _ in 0..<2 where !pending.isEmpty {
            var still = 0
            while !pending.isEmpty && still < 3 {
                let before = pending.count
                for row in rows() where row.isHittable {
                    let label = row.label
                    guard let title = pending.first(where: { label.contains($0) }) else { continue }
                    pending.remove(title)
                    if !hasPicture(row) { bare.append(title) }
                }
                still = pending.count == before ? still + 1 : 0
                feed.swipeUp()
            }
            for _ in 0..<12 { feed.swipeDown() }
        }
        shoot("partner-feed")
        XCTAssertEqual(pending.sorted(), [], "every pack title is on the feed")
        XCTAssertEqual(bare.sorted(), [], "every seeded card carries its picture")
    }

    func test_eachPartnerIsASpace() throws {
        let pack = try loadPack()
        launchToLife()
        app.openLifeSearch()
        app.buttons["jewel-groups"].tap()
        XCTAssertTrue(poll(30) { self.listRows().count > 0 }, "GROUPS lists Spaces")
        for p in pack.partners {
            let row = app.descendants(matching: .any)
                .matching(NSPredicate(format: "identifier BEGINSWITH 'list-row-' AND label CONTAINS %@", p.name))
                .firstMatch
            XCTAssertTrue(row.waitForExistence(timeout: 10), "\(p.name) is a Space")
        }
        shoot("partner-spaces")
    }

    // MARK: - Helpers

    private func loadPack() throws -> Pack {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("SeedMedia/partners/pack.json")
        guard let data = try? Data(contentsOf: url) else {
            throw NSError(domain: "PartnerSeed", code: 1, userInfo: [NSLocalizedDescriptionKey:
                "no partner pack at \(url.path) — run `just seed-partners`"])
        }
        return try JSONDecoder().decode(Pack.self, from: data)
    }

    private func launchToLife() {
        app.launchArguments += ["-pacificSeedPartners", "-pacificNoWake",
                                "-pacificNoMusic", "-pacificNoArt"]
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

    private func rows() -> [XCUIElement] {
        app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'feed-row-'"))
            .allElementsBoundByIndex
    }

    private func listRows() -> [XCUIElement] {
        app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier BEGINSWITH 'list-row-'"))
            .allElementsBoundByIndex
    }

    /// The card's square, sampled on a 7×7 grid over its middle: a flat fill and its
    /// title give a handful of colours; a photograph gives many.
    private func hasPicture(_ row: XCUIElement) -> Bool {
        guard let cg = row.screenshot().image.cgImage,
              let data = cg.dataProvider?.data, let px = CFDataGetBytePtr(data) else { return false }
        let bpp = cg.bitsPerPixel / 8, stride = cg.bytesPerRow
        var seen = Set<Int>()
        for i in 1...7 {
            for j in 1...7 {
                let x = cg.width * (15 + i * 10) / 100, y = cg.height * (15 + j * 10) / 100
                let o = y * stride + x * bpp
                // 3 bits a channel: enough to tell a photo from a fill and its anti-aliasing.
                seen.insert(Int(px[o] >> 5) << 6 | Int(px[o + 1] >> 5) << 3 | Int(px[o + 2] >> 5))
            }
        }
        return seen.count >= 8
    }

    private func poll(_ timeout: TimeInterval, _ ok: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if ok() { return true }
            usleep(500_000)
        }
        return ok()
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
