import Foundation
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

final class NativeFollowUpTests: XCTestCase {
    @MainActor func testIdleTurnDoesNotQueueOrSend() async throws {
        let client = FollowUpClient(running: false)
        let model = NativeConversationModel(client: client)
        await model.open(project: "project-a", session: "session-a")
        model.draft = "继续检查对照"
        await model.queueFollowUp()
        let calls = await client.commands()
        XCTAssertFalse(calls.contains("native_conversation_enqueue"))
        XCTAssertFalse(calls.contains("native_conversation_send"))
        XCTAssertEqual(model.draft, "继续检查对照")
        XCTAssertNil(model.queuedFollowUp)
        model.pause()
    }

    @MainActor func testRunningTurnQueuesOneDraftAndDoesNotSendAnother() async throws {
        let client = FollowUpClient(running: true)
        let model = NativeConversationModel(client: client)
        await model.open(project: "project-a", session: "session-a")
        model.draft = "继续检查对照"
        await model.queueFollowUp()
        let calls = await client.calls()
        let queued = calls.filter { $0.command == "native_conversation_enqueue" }
        XCTAssertEqual(queued.count, 1)
        XCTAssertEqual(queued[0].projectID, "project-a")
        XCTAssertEqual(queued[0].args["session_id"]?.string, "session-a")
        XCTAssertEqual(queued[0].args["message"]?.string, "继续检查对照")
        XCTAssertFalse(calls.contains { $0.command == "native_conversation_send" })
        XCTAssertEqual(model.queuedFollowUp, "继续检查对照")
        XCTAssertEqual(model.draft, "")
        await model.queueFollowUp()
        await model.send()
        await Task.yield()
        let later = await client.calls()
        XCTAssertEqual(later.filter { $0.command == "native_conversation_enqueue" }.count, 1)
        XCTAssertFalse(later.contains { $0.command == "native_conversation_send" })
        model.pause()
    }

    @MainActor func testLostQueueKeepsTheDraftAndDoesNotRetry() async throws {
        let client = FollowUpClient(running: true)
        await client.setLost(true)
        let model = NativeConversationModel(client: client)
        await model.open(project: "project-a", session: "session-a")
        model.draft = "继续检查对照"
        await model.queueFollowUp()
        let calls = await client.calls()
        XCTAssertEqual(calls.filter { $0.command == "native_conversation_enqueue" }.count, 1)
        XCTAssertEqual(model.draft, "继续检查对照")
        XCTAssertNil(model.queuedFollowUp)
        XCTAssertTrue(model.operationError?.contains("不会自动重试") == true)
        await Task.yield()
        let later = await client.calls()
        XCTAssertEqual(later.filter { $0.command == "native_conversation_enqueue" }.count, 1)
        model.pause()
    }
}

private actor FollowUpClient: NativeConversationQuerying {
    private var running: Bool
    private var lost = false
    private var recorded: [(command: String, args: [String: SettingsValue], projectID: String)] = []
    init(running: Bool) { self.running = running }
    func setLost(_ lost: Bool) { self.lost = lost }
    func commands() -> [String] { recorded.map(\.command) }
    func calls() -> [(command: String, args: [String: SettingsValue], projectID: String)] { recorded }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot {
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { url.deleteLastPathComponent() }
        var value = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: url.appendingPathComponent("contracts/native-conversations/v1/snapshot.json")))
        value["session_id"] = .string(sessionID)
        value["project_id"] = .string(projectID)
        value["running"] = .bool(running)
        value["approvals"] = .array([])
        return try ConversationSnapshot.decode(value, projectID: projectID, sessionID: sessionID)
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        recorded.append((command, args, projectID))
        if command == "list_models" { return .array([]) }
        if command == "native_conversation_seen" { return .null }
        if lost { throw ProjectBrowserError.service("response lost") }
        if command == "native_conversation_enqueue" { return .object(["queued": .bool(true), "message": args["message"] ?? .null]) }
        throw ProjectBrowserError.service("unexpected \(command)")
    }
}
