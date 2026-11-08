import XCTest
import PacificCore
@testable import Topic

final class TopicTests: XCTestCase {
    func testModuleAdvertisesItsKind() {
        XCTAssertEqual(TopicModule.kind, .topic)
    }
}
