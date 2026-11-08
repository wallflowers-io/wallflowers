import XCTest
import PacificCore
@testable import Field

final class FieldTests: XCTestCase {
    func testModuleAdvertisesItsKind() {
        XCTAssertEqual(FieldModule.kind, .field)
    }
}
