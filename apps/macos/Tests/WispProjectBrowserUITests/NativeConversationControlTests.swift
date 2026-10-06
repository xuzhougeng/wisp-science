import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor ControlClient: NativeConversationQuerying {
    var fields: [String: SettingsValue] = [:]
    var sequence: Int64 = 10
    var calls: [(String, [String: SettingsValue], String)] = []
    var fail = false
    var held: CheckedContinuation<SettingsValue, Error>?
    var heldCommand: String?
    func configure(_ fields: [String: SettingsValue] = [:], fail: Bool = false) { self.fields = fields; self.fail = fail }
    func state(_ session: String, before: Int64? = nil) throws -> ConversationSnapshot {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        var value = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-conversations/v1/snapshot.json")))
        sequence += 1
        value["sequence"] = .integer(sequence); value["session_id"] = .string(session); value["approvals"] = .array([])
        value["running"] = .bool(false); value["plan_mode"] = .bool(false)
        value["fast_mode"] = .object(["enabled": .bool(false), "inherited": .bool(true)])
        value["history_state"] = .object(["revision": .string("revision"), "can_branch": .bool(true), "reviewing": .bool(false), "turns": .array([turn(0), turn(1)])])
        value["user_offset"] = .integer(before == nil ? 1 : 0)
        value["next_before_seq"] = before == nil ? .integer(3) : .null
        for (key, field) in fields { value[key] = field }
        return try ConversationSnapshot.decode(value, projectID: "project-a", sessionID: session)
    }
    func turn(_ index: Int) -> SettingsValue { .object(["user_index": .integer(Int64(index)), "user_seq": .integer(Int64(index * 2 + 1)), "digest": .string("digest-\(index)")]) }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { try state(sessionID, before: beforeSeq) }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        if command == "get_appearance_prefs" || command == "native_conversation_seen" { return .null }
        if command == "list_models" || command == "list_acp_agents" { return .array([]) }
        calls.append((command, args, projectID))
        if heldCommand == command { heldCommand = nil; return try await withCheckedThrowingContinuation { held = $0 } }
        if fail { throw ProjectBrowserError.service("lost reply") }
        if command == "native_conversation_plan" { fields["plan_mode"] = args["enabled"]; return args["enabled"]! }
        if command == "native_conversation_fast" { fields["fast_mode"] = .object(["enabled": args["enabled"]!, "inherited": .bool(false)]); return fields["fast_mode"]! }
        if command == "native_conversation_history_action" {
            return .object(["session_id": args["session_id"]!, "target": args["target"]!, "result": args["action"]?["kind"].string == "branch" ? .string("branch-id") : .null])
        }
        return .null
    }
    func hold(_ command: String) { heldCommand = command }
    func waiting() -> Bool { held != nil }
    func finish() { held?.resume(returning: .null); held = nil }
    func recorded() -> [(String, [String: SettingsValue], String)] { calls }
}

final class NativeConversationControlTests: XCTestCase {
    @MainActor func testModesPersistExactSessionAndModelWithoutSendingOrChangingDraft() async throws {
        let client = ControlClient(); let model = NativeConversationModel(client: client)
        await model.open(project: "project-a", session: "session-a"); defer { model.pause() }
        model.draft = "keep draft"
        await model.setPlanMode(true); XCTAssertEqual(model.snapshot?.plan_mode, true)
        await model.setFastMode(true); XCTAssertEqual(model.snapshot?.fast_mode?.enabled, true)
        XCTAssertEqual(model.snapshot?.fast_mode?.inherited, false); XCTAssertEqual(model.draft, "keep draft")
        let calls = await client.recorded(); XCTAssertEqual(calls.map(\.0), ["native_conversation_plan", "native_conversation_fast"])
        XCTAssertEqual(calls.last?.1["model_id"], .string("model-a")); XCTAssertEqual(calls.last?.1["session_id"], .string("session-a"))
        XCTAssertEqual(calls.last?.2, "project-a")
    }
    @MainActor func testModesRespectRunningReadonlyAcpAndOlderHostCapabilities() async {
        let client = ControlClient(); let model = NativeConversationModel(client: client)
        for fields: [String: SettingsValue] in [["running": .bool(true)], ["read_only": .bool(true)], ["acp_agent_id": .string("agent")], ["plan_mode": .null, "fast_mode": .null]] {
            await client.configure(fields); await model.open(project: "project-a", session: "session-a")
            await model.setPlanMode(true); await model.setFastMode(true)
        }
        let calls = await client.recorded(); XCTAssertTrue(calls.isEmpty)
        model.pause()
    }
    @MainActor func testLostModeReplyBlocksSendUntilAuthoritativeRefreshAndLateReplyCannotChangeNewSession() async {
        let client = ControlClient(); let model = NativeConversationModel(client: client)
        await model.open(project: "project-a", session: "session-a"); model.draft = "saved draft"
        await client.configure(fail: true); await model.setPlanMode(true)
        XCTAssertFalse(model.canSend); XCTAssertFalse(model.canChangeMode); XCTAssertNotNil(model.operationError)
        await client.configure(); await model.refresh(); XCTAssertTrue(model.canSend)
        await client.hold("native_conversation_plan")
        let saving = Task { await model.setPlanMode(true) }
        while !(await client.waiting()) { await Task.yield() }
        await model.open(project: "project-a", session: "session-b"); model.draft = "new draft"
        await client.finish(); await saving.value
        XCTAssertEqual(model.snapshot?.session_id, "session-b"); XCTAssertEqual(model.snapshot?.plan_mode, false)
        XCTAssertEqual(model.draft, "new draft"); XCTAssertFalse(model.busy)
        model.pause()
    }
    @MainActor func testHistoryUsesGlobalTurnIdentityAcrossPagesAndRejectsStaleConfirmation() async throws {
        let client = ControlClient(); let model = NativeConversationModel(client: client)
        await model.open(project: "project-a", session: "session-a"); defer { model.pause() }
        let latest = try XCTUnwrap(model.historyTarget(row: 0, kind: "rewind"))
        XCTAssertEqual(latest.turn.user_index, 1)
        await model.older()
        let old = try XCTUnwrap(model.historyTarget(row: 0, kind: "branch", checkpoint: "before_user"))
        XCTAssertEqual(old.turn.user_index, 0); XCTAssertEqual(old.turn.user_seq, 1)
        XCTAssertTrue(model.canHistoryAction(old))
        await client.configure(["history_state": .object(["revision": .string("changed"), "can_branch": .bool(true), "reviewing": .bool(false), "turns": .array([await client.turn(0), await client.turn(1)])])])
        await model.refresh(); XCTAssertFalse(model.canHistoryAction(latest))
        let result = await model.performHistoryAction(latest, editedDraft: "")
        XCTAssertNil(result); let calls = await client.recorded(); XCTAssertTrue(calls.isEmpty)
    }
    @MainActor func testEditedBranchKeepsSourceDraftAndRestoresEditedDraftInNewConversation() async throws {
        let client = ControlClient(); let model = NativeConversationModel(client: client)
        await model.open(project: "project-a", session: "session-a"); model.draft = "source draft"
        let target = try XCTUnwrap(model.historyTarget(row: 0, kind: "branch", checkpoint: "before_user"))
        let id = await model.performHistoryAction(target, editedDraft: "edited question")
        XCTAssertEqual(id, "branch-id"); XCTAssertEqual(model.draft, "source draft")
        let calls = await client.recorded(); XCTAssertEqual(calls.first?.1["target"]?["user_seq"], .integer(3))
        XCTAssertEqual(calls.first?.1["revision"], .string("revision"))
        await model.open(project: "project-a", session: "branch-id"); XCTAssertEqual(model.draft, "edited question")
        await model.open(project: "project-a", session: "session-a"); XCTAssertEqual(model.draft, "source draft")
        model.pause()
    }
    @MainActor func testRewindPreservesDraftAndUnknownResultsNeedExplicitAcknowledgement() async throws {
        let client = ControlClient(); let model = NativeConversationModel(client: client)
        await model.open(project: "project-a", session: "session-a"); model.draft = "already typed"
        let target = try XCTUnwrap(model.historyTarget(row: 0, kind: "rewind"))
        let result = await model.performHistoryAction(target, editedDraft: "")
        XCTAssertEqual(result, ""); XCTAssertEqual(model.draft, "already typed")
        await client.configure(fail: true)
        _ = await model.performHistoryAction(target, editedDraft: "")
        XCTAssertTrue(model.historyUncertain); XCTAssertFalse(model.canSend); XCTAssertFalse(model.canHistoryAction(target))
        await model.refresh(); XCTAssertTrue(model.historyUncertain)
        await model.open(project: "project-a", session: "session-b"); XCTAssertFalse(model.historyUncertain)
        await model.open(project: "project-a", session: "session-a")
        XCTAssertTrue(model.historyUncertain); XCTAssertNotNil(model.operationError)
        model.acknowledgeHistoryResult(); XCTAssertFalse(model.historyUncertain); XCTAssertTrue(model.canSend)
        XCTAssertEqual(NativeConversationModel.historyDraft("question\n\nUploaded files: uploads/x.csv\n\nSelected skills: RNA-seq"), "question")
        model.pause()
    }
    @MainActor func testHistorySheetImmediateEscapeClosesOnlyConfirmation() async throws {
        _ = NSApplication.shared
        let client = ControlClient(); let model = NativeConversationModel(client: client)
        await model.open(project: "project-a", session: "session-a"); defer { model.pause() }
        let target = try XCTUnwrap(model.historyTarget(row: 0, kind: "rewind"))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 400), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let container = NSView(frame: window.contentView!.bounds); window.contentView = container
        var parentClosed = 0, sheetClosed = 0
        let parent = NativeSettingsEscape.Coordinator(enabled: true) { parentClosed += 1 }; parent.view = container; parent.install(); defer { parent.remove() }
        let hosted = NSHostingView(rootView: NativeHistoryActionSheet(conversation: model, target: target, close: { sheetClosed += 1 }, openBranch: { _ in XCTFail("Escape must not create a branch") }))
        hosted.frame = container.bounds; container.addSubview(hosted); hosted.layoutSubtreeIfNeeded()
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertEqual(sheetClosed, 1); XCTAssertEqual(parentClosed, 0)
        hosted.removeFromSuperview()
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertEqual(parentClosed, 1)
    }
}
