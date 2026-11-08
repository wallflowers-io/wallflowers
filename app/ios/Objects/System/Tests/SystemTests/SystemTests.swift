import XCTest
import PacificCore
@testable import SystemObject

final class SystemTests: XCTestCase {
    func testModuleAdvertisesItsKind() {
        XCTAssertEqual(SystemModule.kind, .system)
    }
}
