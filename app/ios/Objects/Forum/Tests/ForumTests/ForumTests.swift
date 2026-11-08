import XCTest
import PacificCore
@testable import Forum

final class ForumTests: XCTestCase {
    func testModuleAdvertisesItsKind() {
        XCTAssertEqual(ForumModule.kind, .forum)
    }
}
