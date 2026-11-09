//  PlaceProfileUITests.swift
//  The Place Profile path on the REAL app (T4): a REAL Place minted through RED's
//  creator (a simulated fix makes it PLACED), its pin on THE map — reached by the
//  PLACE jewel in LIFE's search rail — the pin's
//  bottom sheet — the quick resolution — and the sheet's tap-through to the
//  Place's maximised detail: its title, the square drawn from area cells derived
//  from the Place's own GeoPoint, and the walk-up bar (the POV, reachable from
//  the page). Coordinate-tap discipline throughout: beside the rail the
//  hittability oracle is flaky while real touches land fine (the
//  GroupsPageUITests lesson), so taps go by coordinate, as a finger does.

import XCTest
import CoreLocation

final class PlaceProfileUITests: XCTestCase {

    private let app = XCUIApplication()
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        addUIInterruptionMonitor(withDescription: "system-alert") { [weak self] _ in
            self?.tapSpringboard(["Allow While Using App", "Allow Once", "Allow", "OK",
                                  "Don’t Allow", "Don't Allow"]) ?? false
        }
    }

    func test_mapPin_sheet_page_walkUp() throws {
        // A SIMULATED fix, so the wizard's locator delivers and the minted Place
        // is PLACED — pinned, not just listed. Set before launch so the first
        // location read already has it.
        XCUIDevice.shared.location = XCUILocation(
            location: CLLocation(latitude: 51.5074, longitude: -0.1278))

        launchToLife()

        app.openLifeSearch()
        let jewel = app.buttons["jewel-places"]
        XCTAssertTrue(jewel.waitForExistence(timeout: 10), "PLACE jewel")
        jewel.tap()
        XCTAssertTrue(waitAny(["places-map"], timeout: 10), "the map takes the ground")

        // Mint a real Place through RED's creator. Unique name per run — places
        // accumulate.
        let placeName = "QA Pin \(Int(Date().timeIntervalSince1970) % 100_000)"
        let card = app.beginCreating("place")
        let nameField = card.descendants(matching: .any).matching(identifier: "new-field-name").firstMatch
        XCTAssertTrue(nameField.waitForExistence(timeout: 8), "the Place's name field")
        nameField.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(app.keyboards.element.waitForExistence(timeout: 6), "the keyboard")
        nameField.typeText(placeName)

        // Locate: the prompt lands when the fix's locator starts — grant it
        // explicitly rather than trusting the interruption monitor.
        let fix = card.descendants(matching: .any).matching(identifier: "new-fix").firstMatch
        XCTAssertTrue(fix.waitForExistence(timeout: 6), "the Place's fix")
        fix.tap()
        _ = tapSpringboard(["Allow While Using App", "Allow Once", "Allow"], waitFor: 5)

        // Wait for the fix readout ("gcpvj0… · ±N m") — saving earlier mints an
        // UNPLACED Place, which the map honestly declines to pin.
        // The readout is the fix control's own label (RedCreatorUITests reads it the same way).
        expectation(for: NSPredicate(format: "label CONTAINS '±'"), evaluatedWith: fix)
        waitForExpectations(timeout: 20)

        let save = app.buttons["Save Place"]
        XCTAssertTrue(save.waitForExistence(timeout: 6), "BLUE commits the Place")
        save.tap()

        // The pin folds in at the simulated coordinate. The camera follows the
        // user location (the same coordinate), so the pin is on screen.
        let pinMatch = NSPredicate(format: "identifier BEGINSWITH 'map-pin:' AND label == %@",
                                   placeName)
        var pin = app.otherElements.matching(pinMatch).firstMatch
        if !pin.waitForExistence(timeout: 12) {
            pin = app.buttons.matching(pinMatch).firstMatch
        }
        XCTAssertTrue(pin.exists, "the minted place's pin")
        shoot("map-pin")

        // Pin → sheet: the quick resolution. Coordinate tap — the map's
        // annotation views sit under MapKit's own gesture plumbing.
        pin.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        let sheet = app.otherElements["place-sheet"]
        XCTAssertTrue(waitAny(["place-sheet"], timeout: 8) || sheet.exists,
                      "the pin raises the detail sheet")
        shoot("place-sheet")

        // Sheet → page: the deep resolution.
        let open = app.buttons["place-sheet-open"]
        XCTAssertTrue(open.waitForExistence(timeout: 6), "the sheet's tap-through")
        open.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()

        // The sheet opens the Place's maximised detail.
        let media = app.descendants(matching: .any)["detail-media"]
        XCTAssertTrue(media.waitForExistence(timeout: 10), "the Place's detail")
        XCTAssertTrue(app.descendants(matching: .any)["detail-title"].waitForExistence(timeout: 8),
                      "the Place's title")
        // Its square is the area ⇔ cells derived from the Place's own GeoPoint (an
        // unplaced Place draws stripes, and the square says nothing of an area).
        expectation(for: NSPredicate(format: "label CONTAINS 'map of its area'"), evaluatedWith: media)
        waitForExpectations(timeout: 8)
        shoot("place-detail")

        // POV reachable from the page: the walk-up bar opens PlacePOVView. A
        // coordinate-minted place has no scan — "No interior" is the honest
        // interior, and proof the POV screen is the one on top.
        let walkUp = app.buttons["place-walk-up"]
        XCTAssertTrue(walkUp.waitForExistence(timeout: 6), "the walk-up bar")
        walkUp.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(app.staticTexts["No interior"].waitForExistence(timeout: 10),
                      "the POV opens from the page")
        shoot("place-pov")
    }

    // MARK: - Helpers

    /// Land on LIFE (the LifeFacesUITests walk: env name skips Welcome on a
    /// fresh sim; the manual walk covers a build without the seam).
    private func launchToLife() {
        app.launchArguments += ["-pacificNoWake"]
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

    /// An identified surface the OS may expose as scrollView, otherElement or
    /// map — poll the narrow queries, never a broad `.any` descendants sweep
    /// (the GroupsPageUITests lesson: that snapshot can take the runner down).
    private func surfaceExists(_ id: String) -> Bool {
        app.scrollViews[id].exists || app.otherElements[id].exists || app.maps[id].exists
    }

    @discardableResult
    private func waitAny(_ ids: [String], timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if ids.contains(where: surfaceExists) { return true }
            usleep(300_000)
        }
        return ids.contains(where: surfaceExists)
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
            let button = springboard.buttons[label]
            let present = timeout > 0 ? button.waitForExistence(timeout: timeout) : button.exists
            if present { button.tap(); return true }
        }
        return false
    }
}
