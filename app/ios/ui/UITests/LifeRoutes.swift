//  LifeRoutes.swift
//  The two routes LIFE's chrome moved on 9 Nov, shared so every suite takes them one
//  way: the jewels are behind the top bar's search, and RED's creator mints what
//  LIFE's `+` did.

import XCTest

extension XCUIApplication {
    /// Open LIFE's search — the rail, and so every jewel, is up only while it is open.
    func openLifeSearch(file: StaticString = #filePath, line: UInt = #line) {
        if buttons["tab-search-close"].exists { return }
        let search = buttons["tab-search"]
        XCTAssertTrue(search.waitForExistence(timeout: 10), "LIFE's search", file: file, line: line)
        search.tap()
        XCTAssertTrue(buttons["tab-search-close"].waitForExistence(timeout: 5),
                      "the field opens across the top row", file: file, line: line)
    }

    /// RED's creator, turned to `kind` (a `new-kind-` suffix). Returns that kind's card.
    /// The dial shows the kind in view and two each side, so it turns one neighbour at a
    /// time — a kind off the dial is in the tree but not under a finger.
    @discardableResult
    func beginCreating(_ kind: String, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        buttons["shell-note"].tap()
        let ring = ["note", "group", "event", "thing", "place", "project", "post"]
        guard let target = ring.firstIndex(of: kind) else {
            XCTFail("no \(kind) on RED's ring", file: file, line: line)
            return otherElements.firstMatch
        }
        for i in 0...target {
            let dial = descendants(matching: .any).matching(identifier: "new-kind-\(ring[i])").firstMatch
            XCTAssertTrue(dial.waitForExistence(timeout: 10), "RED's dial: \(ring[i])", file: file, line: line)
            if i > 0 { dial.tap() }
        }
        let cards = descendants(matching: .any).matching(identifier: "new-surface-\(kind)")
        let card = cards.allElementsBoundByIndex.first(where: \.isHittable) ?? cards.firstMatch
        XCTAssertTrue(card.waitForExistence(timeout: 5), "\(kind)'s card", file: file, line: line)
        return card
    }
}
