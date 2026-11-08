import XCTest
import PacificCore
@testable import Group

final class GroupTests: XCTestCase {
    func testModuleAdvertisesItsKind() {
        XCTAssertEqual(GroupModule.kind, .group)
    }
}
