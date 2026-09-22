import Foundation
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

final class NativeAttachmentTests: XCTestCase {
    @MainActor func testAttachUsesTheNamedProjectAndALostReplyIsNotRetried() async throws {
        let client = AttachmentClient()
        let model = NativeConversationModel(client: client)
        await model.open(project: "project-a", session: "session-a")
        await model.attach(source: "/tmp/notes.csv", client: client)
        let calls = await client.calls()
        let attached = calls.filter { $0.command == "native_conversation_attach" }
        XCTAssertEqual(attached.count, 1)
        XCTAssertEqual(attached[0].projectID, "project-a")
        XCTAssertEqual(attached[0].args["session_id"]?.string, "session-a")
        XCTAssertEqual(attached[0].args["path"]?.string, "/tmp/notes.csv")
        XCTAssertEqual(model.attachments.map(\.path), ["uploads/notes.csv"])
        XCTAssertFalse(calls.contains { $0.command == "native_conversation_send" || $0.command == "set_active" })
        await client.setLost(true)
        let before = await client.callCount()
        await model.attach(source: "/tmp/other.csv", client: client)
        let after = await client.callCount()
        XCTAssertEqual(after, before + 1)
        XCTAssertEqual(model.attachments.map(\.path), ["uploads/notes.csv"])
        XCTAssertTrue(model.operationError?.contains("不会自动重试") == true)
        await Task.yield()
        let later = await client.callCount()
        XCTAssertEqual(later, after)
        model.pause()
    }

    @MainActor func testSendCarriesTheAttachedPathOnceAndKeepsItWhenTheReplyIsLost() async throws {
        let client = AttachmentClient()
        let model = NativeConversationModel(client: client)
        await model.open(project: "project-a", session: "session-a")
        await model.attach(source: "/tmp/notes.csv", client: client)
        model.draft = "看看"
        await client.setLost(true)
        await model.send()
        var sends = await client.calls()
        sends = sends.filter { $0.command == "native_conversation_send" }
        XCTAssertEqual(sends.count, 1)
        XCTAssertEqual(sends[0].projectID, "project-a")
        XCTAssertEqual(sends[0].args["message"]?.string, "看看")
        XCTAssertEqual(sends[0].args["attachments"]?.array.map(\.string), ["uploads/notes.csv"])
        XCTAssertEqual(model.attachments.map(\.path), ["uploads/notes.csv"])
        XCTAssertEqual(model.draft, "看看")
        await Task.yield()
        let still = await client.calls()
        XCTAssertEqual(still.filter { $0.command == "native_conversation_send" }.count, 1)
        await client.setLost(false)
        model.acknowledgeUncertainSend()
        await model.send()
        let done = await client.calls()
        XCTAssertEqual(done.filter { $0.command == "native_conversation_send" }.count, 2)
        XCTAssertTrue(model.attachments.isEmpty)
        XCTAssertEqual(model.draft, "")
        model.pause()
    }

    @MainActor func testSavedSnapshotShowsTheUploadedFiles() throws {
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { url.deleteLastPathComponent() }
        let item = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: url.appendingPathComponent("contracts/native-conversations/v1/attached-item.json")))
        XCTAssertEqual(SavedAttachments.files(in: item["text"].string), ["uploads/notes.csv", "uploads/figure.png"])
        XCTAssertEqual(SavedAttachments.body(in: item["text"].string), "看看")
        var snapshot = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: url.appendingPathComponent("contracts/native-conversations/v1/snapshot.json")))
        var rows = snapshot["items"].array
        rows[0] = item
        snapshot["items"] = .array(rows)
        snapshot["approvals"] = .array([])
        let saved = try ConversationSnapshot.decode(snapshot, projectID: "project-a", sessionID: "session-a")
        XCTAssertEqual(SavedAttachments.files(in: saved.items[0].text), ["uploads/notes.csv", "uploads/figure.png"])
        XCTAssertEqual(SavedAttachments.body(in: saved.items[0].text), "看看")
    }
}

private actor AttachmentClient: NativeConversationQuerying {
    private var recorded: [(command: String, args: [String: SettingsValue], projectID: String)] = []
    private var lost = false
    func setLost(_ lost: Bool) { self.lost = lost }
    func callCount() -> Int { recorded.count }
    func calls() -> [(command: String, args: [String: SettingsValue], projectID: String)] { recorded }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot {
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { url.deleteLastPathComponent() }
        var value = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: url.appendingPathComponent("contracts/native-conversations/v1/snapshot.json")))
        value["session_id"] = .string(sessionID)
        value["project_id"] = .string(projectID)
        value["running"] = .bool(false)
        value["approvals"] = .array([])
        return try ConversationSnapshot.decode(value, projectID: projectID, sessionID: sessionID)
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        recorded.append((command, args, projectID))
        if command == "list_models" || command == "native_conversation_seen" { return command == "list_models" ? .array([]) : .null }
        if lost { throw ProjectBrowserError.service("response lost") }
        if command == "native_conversation_attach" {
            return .object(["path": .string("uploads/notes.csv"), "name": .string("notes.csv")])
        }
        if command == "native_conversation_send" { return .object(["request_id": args["request_id"] ?? .null]) }
        throw ProjectBrowserError.service("unexpected \(command)")
    }
}
