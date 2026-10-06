import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor PlanDecisionFake: NativeConversationQuerying {
    var acp = false, plan = true, running = false, readOnly = false, loseMode = false, keepMode = false, loseSend = false
    var content = "Inspect **samples**", sequence: Int64 = 0
    var hold = false
    var held: CheckedContinuation<Void, Never>?
    var writes: [(String, [String: SettingsValue], String)] = []
    var sent: [String] = []
    func setup(acp: Bool = false, running: Bool = false, readOnly: Bool = false, loseMode: Bool = false, keepMode: Bool = false, loseSend: Bool = false, hold: Bool = false) {
        self.acp = acp; self.running = running; self.readOnly = readOnly; self.loseMode = loseMode; self.keepMode = keepMode; self.loseSend = loseSend; self.hold = hold
    }
    func changePlan() { content = "replacement" }
    func waiting() -> Bool { held != nil }
    func release() { held?.resume(); held = nil; hold = false }
    func results() -> ([(String, [String: SettingsValue], String)], [String]) { (writes, sent) }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot {
        sequence += 1
        var payload: SettingsValue = .object([
            "schema": .string(ConversationSnapshot.schemaID), "epoch": .string("offline"), "sequence": .integer(sequence),
            "project_id": .string(projectID), "session_id": .string(sessionID), "running": .bool(running), "stopping": .bool(false),
            "read_only": .bool(readOnly), "model_id": .string(acp ? "acp:qa" : "http"), "approvals": .array([]),
            "items": .array([.object(["role": .string("user"), "text": .string("Plan it")]), .object(["role": .string("plan"), "text": .string("raw plan"),
                "proposal": .object(["entries": .array([.object(["content": .string(content), "status": .string("pending"), "priority": .string("high")])]), "source": .string(acp ? "acp" : "native")])])])
        ])
        if acp {
            payload["acp_agent_id"] = .string("qa")
            payload["acp_state"] = .object(["frameId": .string(sessionID), "modes": .object(["currentModeId": .string(plan ? "plan" : "default"), "availableModes": .array([
                .object(["id": .string("plan"), "name": .string("Plan")]), .object(["id": .string("default"), "name": .string("Agent")])])])])
        } else { payload["plan_mode"] = .bool(plan) }
        return try ConversationSnapshot.decode(payload, projectID: projectID, sessionID: sessionID)
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        if command == "list_models" || command == "list_acp_agents" { return .array([]) }
        if command == "native_conversation_plan" || command == "native_conversation_acp_setting" {
            writes.append((command, args, projectID))
            if hold { await withCheckedContinuation { held = $0 } }
            if loseMode { throw ProjectBrowserError.service("mode reply lost") }
            if !keepMode { plan = false }
        }
        if command == "native_conversation_send" {
            XCTAssertFalse(plan)
            sent.append(args["message"]!.string)
            if loseSend { throw ProjectBrowserError.service("send reply lost") }
        }
        return .null
    }
}

final class NativePlanDecisionTests: XCTestCase {
    @MainActor func testNativeAndAcpApproveConfirmModeBeforeSendingAndIgnoreDuplicateClick() async throws {
        for acp in [false, true] {
            let fake = PlanDecisionFake(); await fake.setup(acp: acp, hold: true)
            let model = NativeConversationModel(client: fake); await model.open(project: "p", session: "s")
            defer { model.pause() }
            model.draft = "Only execute step one"
            let target = try XCTUnwrap(model.latestProposal); XCTAssertTrue(model.canDecidePlan(target))
            let decision = Task { await model.decidePlan(target, execute: true) }
            while !(await fake.waiting()) { await Task.yield() }
            await model.decidePlan(target, execute: true)
            var (writes, sent) = await fake.results(); XCTAssertEqual(writes.count, 1); XCTAssertTrue(sent.isEmpty)
            await fake.release(); await decision.value
            (writes, sent) = await fake.results(); XCTAssertEqual(sent, ["Only execute step one"]); XCTAssertEqual(model.draft, "")
            XCTAssertEqual(writes[0].2, "p"); XCTAssertEqual(writes[0].1["session_id"], .string("s"))
            XCTAssertEqual(writes[0].0, acp ? "native_conversation_acp_setting" : "native_conversation_plan")
            if acp { XCTAssertEqual(writes[0].1["change"], .object(["kind": .string("mode"), "id": .string("default")])) }
            else { XCTAssertEqual(writes[0].1["enabled"], .bool(false)) }
        }
    }
    @MainActor func testSavePreservesDraftAndEmptyApprovalUsesExecutionInstruction() async throws {
        let fake = PlanDecisionFake(), executeFake = PlanDecisionFake()
        let model = NativeConversationModel(client: executeFake)
        let save = NativeConversationModel(client: fake); await save.open(project: "p", session: "s"); defer { save.pause(); model.pause() }
        save.draft = "Unsent changes"
        await save.decidePlan(try XCTUnwrap(save.latestProposal), execute: false)
        let (_, sent) = await fake.results(); XCTAssertTrue(sent.isEmpty); XCTAssertEqual(save.draft, "Unsent changes"); XCTAssertEqual(save.snapshot?.plan_mode, false)
        await model.open(project: "p", session: "s")
        await model.decidePlan(try XCTUnwrap(model.latestProposal), execute: true)
        let (_, executed) = await executeFake.results(); XCTAssertEqual(executed, [localized("批准并执行")]); XCTAssertEqual(model.draft, "")
    }
    @MainActor func testLostUnconfirmedChangedAndNavigatedDecisionsNeverDispatch() async throws {
        for scenario in ["lost-mode", "unconfirmed-mode", "changed-plan", "changed-draft", "navigation", "lost-send"] {
            let fake = PlanDecisionFake()
            await fake.setup(loseMode: scenario == "lost-mode", keepMode: scenario == "unconfirmed-mode", loseSend: scenario == "lost-send", hold: true)
            let model = NativeConversationModel(client: fake); await model.open(project: "p", session: "s"); defer { model.pause() }
            model.draft = "my draft"; let target = try XCTUnwrap(model.latestProposal)
            let decision = Task { await model.decidePlan(target, execute: true) }
            while !(await fake.waiting()) { await Task.yield() }
            if scenario == "changed-plan" { await fake.changePlan() }
            if scenario == "changed-draft" { model.draft = "new draft" }
            if scenario == "navigation" { await model.open(project: "p", session: "other"); model.draft = "other draft" }
            await fake.release(); await decision.value
            await model.decidePlan(target, execute: true); await model.refresh()
            let (writes, sent) = await fake.results(); XCTAssertEqual(writes.count, 1, scenario)
            if scenario == "lost-send" { XCTAssertEqual(sent.count, 1); XCTAssertTrue(model.uncertainSend) }
            else { XCTAssertTrue(sent.isEmpty, scenario) }
            XCTAssertEqual(model.draft, scenario == "navigation" ? "other draft" : scenario == "changed-draft" ? "new draft" : "my draft", scenario)
            if scenario == "navigation" { XCTAssertNil(model.operationError); XCTAssertEqual(model.snapshot?.session_id, "other") }
            else { XCTAssertNotNil(model.operationError) }
            if scenario == "lost-mode" || scenario == "unconfirmed-mode" { XCTAssertTrue(model.planDecisionUncertain(target)); model.acknowledgePlanDecision(target); XCTAssertFalse(model.planDecisionUncertain(target)) }
        }
    }
    @MainActor func testStaleRunningReadOnlyAndMalformedPlansHaveNoDecisions() async throws {
        let fake = PlanDecisionFake()
        let model = NativeConversationModel(client: fake); await model.open(project: "p", session: "s"); defer { model.pause() }
        let stale = try XCTUnwrap(model.latestProposal)
        await fake.changePlan(); await model.refresh(); await model.decidePlan(stale, execute: true)
        await fake.setup(running: true); await model.refresh(); XCTAssertFalse(model.canDecidePlan(try XCTUnwrap(model.latestProposal)))
        await fake.setup(readOnly: true); await model.refresh(); XCTAssertFalse(model.canDecidePlan(try XCTUnwrap(model.latestProposal)))
        let (writes, _) = await fake.results(); XCTAssertTrue(writes.isEmpty)
        let bad = try JSONDecoder().decode(ConversationPlanProposal.self, from: Data(#"{"source":"native","entries":[{"content":"","priority":"high","status":"pending"}]}"#.utf8))
        XCTAssertFalse(bad.valid)
    }
}
