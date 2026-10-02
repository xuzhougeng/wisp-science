import XCTest
@testable import WispProjectBrowser

final class NativeMessageMetadataTests: XCTestCase {
    func testOptionalMetadataRemainsBackwardCompatible() throws {
        let legacy = try JSONDecoder().decode(ConversationItem.self, from: Data(#"{"role":"assistant","text":"hello"}"#.utf8))
        XCTAssertNil(legacy.timestamp); XCTAssertNil(legacy.model_name); XCTAssertNil(legacy.duration_ms)
        let current = try JSONDecoder().decode(ConversationItem.self, from: Data(#"{"role":"tool","text":"done","timestamp":1790568000,"model_name":"exact-model","duration_ms":1250}"#.utf8))
        XCTAssertEqual(current.timestamp, 1790568000)
        XCTAssertEqual(current.model_name, "exact-model")
        XCTAssertEqual(current.duration_ms, 1250)
        XCTAssertEqual(try JSONDecoder().decode(ConversationItem.self, from: JSONEncoder().encode(current)), current)
    }
}
