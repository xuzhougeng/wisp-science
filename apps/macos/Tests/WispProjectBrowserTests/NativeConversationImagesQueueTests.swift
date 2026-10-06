import Foundation
import XCTest
@testable import WispProjectBrowser

final class NativeConversationImagesQueueTests: XCTestCase {
    private func fixture() throws -> SettingsValue {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        return try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-conversations/v1/snapshot-images-queue.json")))
    }
    func testSharedFixturePreservesImageVersionsAndFullPrecisionQueueIDs() throws {
        let snapshot = try ConversationSnapshot.decode(fixture(), projectID: "project-a", sessionID: "session-a")
        XCTAssertEqual(snapshot.items[1].resources?.first?.originalReference, "figures/qc.png")
        XCTAssertEqual(snapshot.items[1].resources?.first?.artifactVersionId, "version-a")
        XCTAssertEqual(snapshot.queue?.items.map(\.id), ["18446744073709551615", "9007199254740993"])
        XCTAssertEqual(snapshot.queue?.items[0].references, [.object(["kind": .string("artifact"), "id": .string("artifact-a")])])
        XCTAssertEqual(snapshot.queue?.outcomes.first?.state, "completed")
    }
    func testMalformedQueueCannotEnableWrites() throws {
        for id in ["01", "-1", "18446744073709551616"] {
            var value = try fixture(); var rows = value["queue"]["items"].array; rows[0]["id"] = .string(id); value["queue"]["items"] = .array(rows)
            XCTAssertThrowsError(try ConversationSnapshot.decode(value, projectID: "project-a", sessionID: "session-a"))
        }
        var value = try fixture(); let rows = value["queue"]["items"].array
        value["queue"]["items"] = .array([rows[0], rows[0]])
        XCTAssertThrowsError(try ConversationSnapshot.decode(value, projectID: "project-a", sessionID: "session-a"))
        value = try fixture(); value["queue"]["items"] = .array([.object(["id": .string("1"), "digest": .string("wrong"), "message": .string("x"), "state": .string("queued"), "attachments": .array([]), "references": .array([])])])
        XCTAssertThrowsError(try ConversationSnapshot.decode(value, projectID: "project-a", sessionID: "session-a"))
    }
    func testLegacyMessagesHaveNoImageBindings() throws {
        let item = try JSONDecoder().decode(ConversationItem.self, from: Data(#"{"role":"assistant","text":"hello"}"#.utf8))
        XCTAssertNil(item.resources)
    }
}
