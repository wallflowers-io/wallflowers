import XCTest
import PacificCore
@testable import Project

final class ProjectTests: XCTestCase {
    func testModuleAdvertisesItsKind() {
        XCTAssertEqual(ProjectModule.kind, .project)
    }
}
