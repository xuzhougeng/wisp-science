import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor AcpConversationFake: NativeConversationQuerying {
    var sequence: Int64 = 1
    var bound = false
    var running = false
    var frozen = false
    var failedCreate = false
    var requestID: String?
    var calls: [(String, [String: SettingsValue], String)] = []
    func setState(bound: Bool, running: Bool, frozen: Bool = false) { self.bound = bound; self.running = running; self.frozen = frozen }
    func failCreate() { failedCreate = true }
    func writes() -> [(String, [String: SettingsValue], String)] { calls }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot {
        sequence += 1
        return try ConversationSnapshot.decode(.object([
            "schema": .string(ConversationSnapshot.schemaID), "epoch": .string("offline"), "sequence": .integer(sequence),
            "project_id": .string(projectID), "session_id": .string(sessionID), "items": .array([]),
            "running": .bool(running), "stopping": .bool(false), "read_only": .bool(frozen),
            "model_id": .string(bound ? "acp:Agent display label" : "acp:agent-id"),
            "acp_agent_id": bound ? .string("agent-id") : .null,
            "approvals": .array([]), "request_id": requestID.map(SettingsValue.string) ?? .null
        ]), projectID: projectID, sessionID: sessionID)
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        if command == "list_acp_agents" { return .array([.object(["id": .string("agent-id"), "label": .string("Agent display label")])]) }
        if command == "list_models" { return .array([.object(["id": .string("http"), "label": .string("HTTP")])]) }
        if command == "get_appearance_prefs" || command == "native_conversation_seen" { return .null }
        calls.append((command, args, projectID))
        if command == "native_conversation_create" {
            if failedCreate { throw ProjectBrowserError.service("ACP profile missing") }
            return .string("new-acp-session")
        }
        if command == "native_conversation_send" { requestID = args["request_id"]?.string; bound = true; running = true }
        if command == "native_conversation_stop" { running = false }
        return .null
    }
}
final class NativeAcpConversationTests: XCTestCase {
    @MainActor func testCreateUsesExplicitAgentWithoutReplacingCurrentDraft() async throws {
        let fake = AcpConversationFake(); let model = NativeConversationModel(client: fake)
        await model.open(project: "p", session: "current"); defer { model.pause() }
        model.draft = "keep my current notes"
        let id = await model.create(project: "p", acpAgentID: "agent-id")
        XCTAssertEqual(id, "new-acp-session"); XCTAssertEqual(model.draft, "keep my current notes")
        let writes = await fake.writes()
        XCTAssertEqual(writes.count, 1); XCTAssertEqual(writes[0].0, "native_conversation_create")
        XCTAssertEqual(writes[0].1, ["acp_agent_id": .string("agent-id")]); XCTAssertEqual(writes[0].2, "p")
    }
    @MainActor func testSavedAcpConversationCanSendStopAndCannotSwitchHttpModel() async {
        let fake = AcpConversationFake(); await fake.setState(bound: true, running: false)
        let model = NativeConversationModel(client: fake)
        await model.open(project: "p", session: "saved"); defer { model.pause() }
        model.draft = "continue saved ACP"
        XCTAssertTrue(model.canSend); XCTAssertTrue(model.isAcp); XCTAssertEqual(model.modelLabel, "Agent display label")
        await model.selectModel("http")
        await model.send()
        XCTAssertEqual(model.draft, ""); XCTAssertEqual(model.snapshot?.running, true)
        await model.stop()
        XCTAssertEqual(model.snapshot?.running, false)
        let writes = await fake.writes()
        XCTAssertEqual(writes.map(\.0), ["native_conversation_send", "native_conversation_stop"])
        XCTAssertEqual(writes[0].1["session_id"], .string("saved"))
        XCTAssertEqual(writes[0].1["message"], .string("continue saved ACP"))
        XCTAssertEqual(writes[1].1, ["session_id": .string("saved")])
    }
    @MainActor func testFollowUpWaitsForPersistedAcpBindingAndFrozenSessionStaysReadOnly() async {
        let fake = AcpConversationFake(); await fake.setState(bound: false, running: true)
        let model = NativeConversationModel(client: fake)
        await model.open(project: "p", session: "new"); defer { model.pause() }
        model.draft = "next task"
        XCTAssertEqual(model.modelLabel, "Agent display label"); XCTAssertFalse(model.canQueueFollowUp)
        await fake.setState(bound: true, running: true); await model.refresh()
        XCTAssertTrue(model.canQueueFollowUp)
        await fake.setState(bound: true, running: false, frozen: true); await model.refresh()
        XCTAssertFalse(model.canSend); XCTAssertFalse(model.canAttach)
        await model.send()
        let writes = await fake.writes(); XCTAssertTrue(writes.isEmpty)
    }
    @MainActor func testCreateFailureDoesNotRetryOrNavigate() async {
        let fake = AcpConversationFake(); await fake.failCreate()
        let model = NativeConversationModel(client: fake)
        await model.open(project: "p", session: "current"); defer { model.pause() }
        let id = await model.create(project: "p", acpAgentID: "removed")
        XCTAssertNil(id); XCTAssertEqual(model.snapshot?.session_id, "current"); XCTAssertNotNil(model.operationError)
        await model.refresh()
        let writes = await fake.writes(); XCTAssertEqual(writes.count, 1)
    }
}
