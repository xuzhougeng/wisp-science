import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private func acpSettings(_ session: String) -> SettingsValue {
    var value = try! JSONDecoder().decode(SettingsValue.self, from: Data(#"""
    {
      "frameId": "s",
      "modes": {"currentModeId": "plan_mode", "availableModes": [
        {"id": "plan_mode", "name": "Plan"}, {"id": "agent", "name": "Agent"}
      ]},
      "configOptions": [
        {"id": "thinking", "name": "Think", "type": "boolean", "currentValue": false},
        {"id": "model", "name": "Model", "type": "select", "currentValue": "fast", "options": [
          {"value": "fast", "name": "Fast"},
          {"name": "Reasoning", "options": [{"value": "deep", "name": "Deep"}]}
        ]},
        {"id": "future", "name": "Future", "type": "number", "currentValue": 3}
      ]
    }
    """#.utf8))
    value["frameId"] = .string(session)
    return value
}
private actor AcpSettingsFake: NativeConversationQuerying {
    var sequence: Int64 = 0
    var running = false, readOnly = false, bound = true, lose = false, hold = false
    var value = acpSettings("s")
    var held: CheckedContinuation<Void, Never>?
    var writes: [[String: SettingsValue]] = []
    func setup(running: Bool = false, readOnly: Bool = false, bound: Bool = true, lose: Bool = false, hold: Bool = false) {
        self.running = running; self.readOnly = readOnly; self.bound = bound; self.lose = lose; self.hold = hold
    }
    func waiting() -> Bool { held != nil }
    func release() { held?.resume(); held = nil; hold = false }
    func recorded() -> [[String: SettingsValue]] { writes }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot {
        sequence += 1
        var state = value; state["frameId"] = .string(sessionID)
        return try ConversationSnapshot.decode(.object([
            "schema": .string(ConversationSnapshot.schemaID), "epoch": .string("offline"), "sequence": .integer(sequence), "project_id": .string(projectID), "session_id": .string(sessionID),
            "items": .array([]), "running": .bool(running), "stopping": .bool(false), "read_only": .bool(readOnly), "model_id": .string("acp:qa"), "approvals": .array([]), "acp_state": bound ? state : .null
        ]), projectID: projectID, sessionID: sessionID)
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        if command == "list_models" || command == "list_acp_agents" { return .array([]) }
        if command == "native_conversation_acp_setting" {
            XCTAssertEqual(projectID, "p"); writes.append(args)
            if hold { await withCheckedContinuation { held = $0 } }
            if lose { throw ProjectBrowserError.service("reply lost") }
            let change = args["change"]!
            if change["kind"].string == "mode" { value["modes"]["currentModeId"] = change["id"] }
            else {
                var options = value["configOptions"].array
                if let i = options.firstIndex(where: { $0["id"] == change["id"] }) { options[i]["currentValue"] = change["value"] }
                value["configOptions"] = .array(options)
            }
        }
        return .null
    }
}

final class NativeAcpSettingsTests: XCTestCase {
    func testDecoderUsesCamelCaseExactIDsGroupedChoicesAndBooleanTypes() throws {
        let value = acpSettings("s")
        let state = try JSONDecoder().decode(ConversationAcpState.self, from: JSONEncoder().encode(value))
        XCTAssertEqual(state.frameID, "s"); XCTAssertEqual(state.currentMode, "plan_mode"); XCTAssertEqual(state.exitPlanMode, "agent")
        XCTAssertEqual(state.configurations[1].choices.map(\.id), ["fast", "deep"])
        XCTAssertTrue(state.allows("model", value: .string("deep"))); XCTAssertFalse(state.allows("model", value: .string("Deep")))
        XCTAssertTrue(state.allows("thinking", value: .bool(true))); XCTAssertFalse(state.allows("thinking", value: .string("true")))
        XCTAssertFalse(state.allows("future", value: .integer(4))); XCTAssertFalse(state.hasModeConfiguration)
        var wrong = value; wrong["configOptions"] = .array(value["configOptions"].array + [value["configOptions"].array[0]])
        let duplicate = try JSONDecoder().decode(ConversationAcpState.self, from: JSONEncoder().encode(wrong)); XCTAssertFalse(duplicate.valid)
        var root = URL(fileURLWithPath: #filePath); for _ in 0..<5 { root.deleteLastPathComponent() }
        var payload = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-conversations/v1/snapshot.json")))
        payload["acp_state"] = value
        XCTAssertThrowsError(try ConversationSnapshot.decode(payload, projectID: "project-a", sessionID: "session-a"))
    }
    @MainActor func testSettingsSendOnlyAdvertisedIDsAndValuesAndPreserveDraft() async {
        let fake = AcpSettingsFake()
        let model = NativeConversationModel(client: fake); await model.open(project: "p", session: "s"); defer { model.pause() }
        model.draft = "my draft"; XCTAssertTrue(model.canChangeAcpSettings)
        await model.setAcpMode("Agent"); await model.setAcpConfig("thinking", value: .string("true")); await model.setAcpConfig("future", value: .integer(4))
        await model.setAcpMode("agent"); await model.setAcpConfig("thinking", value: .bool(true)); await model.setAcpConfig("model", value: .string("deep"))
        let writes = await fake.recorded(); XCTAssertEqual(writes.count, 3); XCTAssertTrue(writes.allSatisfy { $0["session_id"] == .string("s") })
        XCTAssertEqual(writes[1]["change"], .object(["kind": .string("config"), "id": .string("thinking"), "value": .bool(true)]))
        XCTAssertEqual(model.snapshot?.acp_state?.currentMode, "agent"); XCTAssertEqual(model.draft, "my draft")
        await fake.setup(running: true); await model.refresh(); XCTAssertFalse(model.canChangeAcpSettings); await model.setAcpMode("plan_mode")
        await fake.setup(readOnly: true); await model.refresh(); XCTAssertFalse(model.canChangeAcpSettings)
        await fake.setup(bound: false); await model.refresh(); XCTAssertFalse(model.canChangeAcpSettings)
        let final = await fake.recorded(); XCTAssertEqual(final.count, 3)
    }
    @MainActor func testLateAndLostWritesStayInTheirOriginAndDoNotReplay() async {
        let fake = AcpSettingsFake(); await fake.setup(lose: true, hold: true)
        let model = NativeConversationModel(client: fake); await model.open(project: "p", session: "s"); defer { model.pause() }
        let change = Task { await model.setAcpMode("agent") }
        while !(await fake.waiting()) { await Task.yield() }
        await model.open(project: "p", session: "other"); model.draft = "other draft"
        await fake.release(); await change.value
        XCTAssertEqual(model.draft, "other draft"); XCTAssertNil(model.operationError)
        let writes = await fake.recorded(); XCTAssertEqual(writes.count, 1)
        await model.open(project: "p", session: "s"); model.draft = "kept"
        await model.setAcpMode("agent"); XCTAssertFalse(model.canSend)
        let final = await fake.recorded(); XCTAssertEqual(final.count, 2)
        await model.refresh(); XCTAssertTrue(model.canSend); XCTAssertEqual(model.draft, "kept")
    }
}
