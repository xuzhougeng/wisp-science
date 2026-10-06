import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private func contextFixture() throws -> SettingsValue {
    var root = URL(fileURLWithPath: #filePath); for _ in 0..<5 { root.deleteLastPathComponent() }
    return try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-conversations/v1/context.json")))
}
private actor ContextFake: NativeConversationQuerying {
    var fields: [String: SettingsValue] = [:]
    var sequence: Int64 = 10
    var request: String?
    var writes: [(String, [String: SettingsValue], String)] = []
    var fail = false, wrongOwner = false, hold = false, confirmLostAck = false
    var held: CheckedContinuation<Void, Never>?
    func configure(_ fields: [String: SettingsValue] = [:], fail: Bool = false, wrongOwner: Bool = false, hold: Bool = false, confirmLostAck: Bool = false) {
        self.fields = fields; self.fail = fail; self.wrongOwner = wrongOwner; self.hold = hold; self.confirmLostAck = confirmLostAck
    }
    func waiting() -> Bool { held != nil }
    func release() { held?.resume(); held = nil; hold = false }
    func recorded() -> [(String, [String: SettingsValue], String)] { writes }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot {
        var root = URL(fileURLWithPath: #filePath); for _ in 0..<5 { root.deleteLastPathComponent() }
        var value = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-conversations/v1/snapshot.json")))
        sequence += 1
        value["project_id"] = .string(projectID); value["session_id"] = .string(sessionID); value["sequence"] = .integer(sequence)
        value["context_view"] = .bool(true); value["running"] = .bool(false); value["approvals"] = .array([])
        value["request_id"] = request.map(SettingsValue.string) ?? .null; value["composer_references"] = .bool(true)
        for (key, field) in fields { value[key] = field }
        return try ConversationSnapshot.decode(value, projectID: projectID, sessionID: sessionID)
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        if command == "get_appearance_prefs" || command == "native_conversation_seen" { return .null }
        if command == "list_models" || command == "list_acp_agents" { return .array([]) }
        writes.append((command, args, projectID))
        if command == "native_conversation_context" {
            if hold { await withCheckedContinuation { held = $0 } }
            var value = try contextFixture(); value["session_id"] = wrongOwner ? .string("other") : (args["session_id"] ?? .null)
            return value
        }
        if command == "native_conversation_attach" { return .object(["path": .string("uploads/samples.csv"), "name": .string("samples.csv")]) }
        if command == "native_conversation_send" {
            if !fail || confirmLostAck { request = args["request_id"]?.string }
            if fail { throw ProjectBrowserError.service("lost ack") }
            return .object(["request_id": args["request_id"]!, "session_id": wrongOwner ? .string("other") : args["session_id"]!, "epoch": .string("host-one")])
        }
        if command == "native_conversation_context_undo" {
            if fail { throw ProjectBrowserError.service("lost undo reply") }
            return .object(["project_id": .string(projectID), "session_id": wrongOwner ? .string("other") : args["session_id"]!, "undone_epoch": args["head_epoch"]!])
        }
        return .null
    }
}

final class NativeConversationContextTests: XCTestCase {
    func testCompactionTranscriptUsesRecordedCountsAndRetainsCheckpointAndUndoState() throws {
        let item = try JSONDecoder().decode(ConversationItem.self, from: Data(#"{"role":"compaction","text":"{\"before\":12000,\"after\":2800,\"strategy\":\"manual\",\"checkpoint\":\"Retain sample IDs\",\"undone\":true}"}"#.utf8))
        let card = try XCTUnwrap(NativeTranscriptCompaction(item))
        XCTAssertEqual(card.before, 12000); XCTAssertEqual(card.after, 2800); XCTAssertEqual(card.checkpoint, "Retain sample IDs"); XCTAssertTrue(card.undone)
        let invalid = try JSONDecoder().decode(ConversationItem.self, from: Data(#"{"role":"compaction","text":"{\"before\":-1,\"after\":10,\"strategy\":\"manual\"}"}"#.utf8))
        XCTAssertNil(NativeTranscriptCompaction(invalid))
    }
    func testSharedContextContractRetainsSystemCheckpointToolsAndUndoIdentity() throws {
        var fixture = try contextFixture()
        let value = try NativeConversationContext.decode(fixture, project: "project-a", session: "session-a")
        XCTAssertEqual(value.items[0].role, "system"); XCTAssertTrue(value.items[1].text.contains("[context summary checkpoint]"))
        XCTAssertEqual(value.details.tool_definitions.first?.name, "read"); XCTAssertEqual(value.state.undoable?.epoch, 2)
        XCTAssertThrowsError(try NativeConversationContext.decode(fixture, project: "other", session: "session-a"))
        fixture["state"]["undone_epochs"] = .array([.integer(2)])
        XCTAssertNil(try NativeConversationContext.decode(fixture, project: "project-a", session: "session-a").state.undoable)
        guard case .array(let compactions) = fixture["state"]["compactions"] else { return XCTFail("Missing compactions") }
        fixture["state"]["compactions"] = .array(compactions + compactions)
        XCTAssertThrowsError(try NativeConversationContext.decode(fixture, project: "project-a", session: "session-a"))
    }
    @MainActor func testRegularAndSemanticCompactionKeepDraftAttachmentsReferencesAndScope() async throws {
        let fake = ContextFake(), model = NativeConversationModel(client: fake)
        await model.open(project: "project-a", session: "session-a"); defer { model.pause() }
        model.draft = "Continue this later"
        await model.attach(source: "/samples.csv", client: fake)
        let reference = NativeComposerReference(reference: .object(["kind": .string("project"), "id": .string("project-a")]), label: "Project", detail: "")
        XCTAssertTrue(model.addReference(reference))
        let result = try await model.readContext(project: "project-a", session: "session-a")
        XCTAssertEqual(result?.state.head_epoch, 2)
        let regular = await model.compact(project: "project-a", session: "session-a", semantic: false, instruction: "ignored")
        let semantic = await model.compact(project: "project-a", session: "session-a", semantic: true, instruction: "  Retain sample IDs.  ")
        XCTAssertTrue(regular); XCTAssertTrue(semantic); XCTAssertEqual(model.draft, "Continue this later")
        XCTAssertEqual(model.attachments.map(\.path), ["uploads/samples.csv"]); XCTAssertEqual(model.references, [reference])
        let writes = await fake.recorded(), sends = writes.filter { $0.0 == "native_conversation_send" }
        XCTAssertEqual(sends.map { $0.1["message"]?.string }, ["/compact", "/compact --semantic Retain sample IDs."])
        XCTAssertTrue(sends.allSatisfy { $0.2 == "project-a" && $0.1["session_id"] == .string("session-a") && $0.1["attachments"] == nil && $0.1["references"] == nil })
        XCTAssertNotEqual(sends[0].1["request_id"], sends[1].1["request_id"])
    }
    @MainActor func testContextGuardsOlderHostsReadOnlyAcpRunningAndWrongSelectedOwner() async throws {
        let fake = ContextFake(), model = NativeConversationModel(client: fake)
        await model.open(project: "project-a", session: "session-a"); defer { model.pause() }
        for fields: [String: SettingsValue] in [["context_view": .null], ["read_only": .bool(true)], ["acp_agent_id": .string("agent")], ["running": .bool(true)]] {
            await fake.configure(fields); await model.refresh()
            let result = await model.compact(project: "project-a", session: "session-a", semantic: false, instruction: "")
            XCTAssertFalse(result)
        }
        await fake.configure(); await model.refresh()
        let result = await model.compact(project: "project-a", session: "other", semantic: true, instruction: "")
        XCTAssertFalse(result)
        let writes = await fake.recorded(); XCTAssertTrue(writes.isEmpty)
    }
    @MainActor func testLostCompactionAckNeverReplaysAndSnapshotCanReconcileWithoutConsumingDraft() async {
        for acknowledged in [false, true] {
            let fake = ContextFake(), model = NativeConversationModel(client: fake)
            await model.open(project: "project-a", session: "session-a"); model.draft = "keep"
            await fake.configure(fail: true, confirmLostAck: acknowledged)
            _ = await model.compact(project: "project-a", session: "session-a", semantic: false, instruction: "")
            XCTAssertEqual(model.uncertainSend, !acknowledged); XCTAssertEqual(model.draft, "keep")
            if !acknowledged {
                _ = await model.compact(project: "project-a", session: "session-a", semantic: false, instruction: "")
                await model.open(project: "project-a", session: "session-b")
                await model.open(project: "project-a", session: "session-a"); XCTAssertTrue(model.uncertainSend)
            }
            let writes = await fake.recorded(); XCTAssertEqual(writes.filter { $0.0 == "native_conversation_send" }.count, 1)
            model.pause()
        }
    }
    @MainActor func testContextReadsRejectMismatchedOwnerAndDiscardCancelledOrNavigatedResults() async throws {
        let fake = ContextFake(), model = NativeConversationModel(client: fake)
        await model.open(project: "project-a", session: "session-a"); defer { model.pause() }
        await fake.configure(wrongOwner: true)
        do { _ = try await model.readContext(project: "project-a", session: "session-a"); XCTFail("Wrong owner") } catch {}
        for navigate in [false, true] {
            await fake.configure(hold: true)
            let task = Task { try await model.readContext(project: "project-a", session: "session-a") }
            while !(await fake.waiting()) { await Task.yield() }
            if navigate { await model.open(project: "project-a", session: "session-b") } else { task.cancel() }
            await fake.release(); let result = try await task.value; XCTAssertNil(result)
        }
        XCTAssertFalse(model.busy); XCTAssertFalse(model.historyUncertain)
    }
    @MainActor func testUndoCompactionCapturesEpochKeepsDraftAndBlocksUnconfirmedRepeat() async {
        let fake = ContextFake(), model = NativeConversationModel(client: fake)
        await model.open(project: "project-a", session: "session-a"); defer { model.pause() }
        model.draft = "keep"
        let result = await model.undoCompaction(project: "project-a", session: "session-a", epoch: 2)
        XCTAssertTrue(result); XCTAssertEqual(model.draft, "keep")
        await fake.configure(fail: true)
        _ = await model.undoCompaction(project: "project-a", session: "session-a", epoch: 2)
        XCTAssertTrue(model.historyUncertain); XCTAssertFalse(model.canCompact)
        _ = await model.undoCompaction(project: "project-a", session: "session-a", epoch: 2)
        let writes = await fake.recorded(); XCTAssertEqual(writes.count, 2)
        XCTAssertTrue(writes.allSatisfy { $0.0 == "native_conversation_context_undo" && $0.1["head_epoch"] == .integer(2) && $0.1["session_id"] == .string("session-a") && $0.2 == "project-a" })
    }
    @MainActor func testImmediateEscapeClosesCompactionConfirmationBeforeContextView() async throws {
        _ = NSApplication.shared
        let fake = ContextFake(), model = NativeConversationModel(client: fake)
        await model.open(project: "project-a", session: "session-a"); defer { model.pause() }
        let context = try NativeConversationContext.decode(contextFixture(), project: "project-a", session: "session-a")
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 680, height: 580), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        var parentClosed = 0, childClosed = 0, confirmed = 0
        let parent = NSHostingView(rootView: NativeConversationContextSheet(conversation: model, project: "project-a", session: "session-a", context: context) { parentClosed += 1 })
        parent.frame = window.contentView!.bounds; window.contentView!.addSubview(parent); parent.layoutSubtreeIfNeeded()
        let child = NSHostingView(rootView: NativeCompactConfirmation(conversation: model, canConfirm: true, close: { childClosed += 1 }) { _, _ in confirmed += 1 })
        child.frame = parent.frame; window.contentView!.addSubview(child); child.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertEqual(childClosed, 1); XCTAssertEqual(parentClosed, 0); XCTAssertEqual(confirmed, 0); XCTAssertTrue(window.firstResponder === focus)
        child.removeFromSuperview()
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertEqual(parentClosed, 1)
        let writes = await fake.recorded(); XCTAssertTrue(writes.isEmpty)
    }
    @MainActor func testRenderContextAndConfirmationInBothLocalesAndSchemes() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Set WISP_NATIVE_SNAPSHOT_DIR to render context") }
        let previous = UserDefaults.standard.object(forKey: "nativeSettings.locale")
        defer { if let previous { UserDefaults.standard.set(previous, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        let fake = ContextFake(), model = NativeConversationModel(client: fake)
        await model.open(project: "project-a", session: "session-a"); defer { model.pause() }
        let context = try NativeConversationContext.decode(contextFixture(), project: "project-a", session: "session-a")
        for locale in ["zh", "en"] {
            for scheme in [ColorScheme.light, .dark] {
                UserDefaults.standard.set(locale, forKey: "nativeSettings.locale")
                for tab in ["messages", "details", "compactions", "confirmation", "semantic"] {
                    let confirmation = ["confirmation", "semantic"].contains(tab)
                    let view: AnyView = confirmation ? AnyView(NativeCompactConfirmation(conversation: model, canConfirm: true, semantic: tab == "semantic", instruction: "Retain sample IDs and decisions.", close: {}) { _, _ in })
                        : AnyView(NativeConversationContextSheet(conversation: model, project: "project-a", session: "session-a", context: context, tab: tab, close: {}))
                    let hosted = NSHostingView(rootView: view.background(WispDesign.color("bg-app", scheme)).foregroundStyle(WispDesign.color("text", scheme))
                        .tint(WispDesign.color("clay", scheme)).environment(\.colorScheme, scheme))
                    hosted.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
                    hosted.frame = NSRect(x: 0, y: 0, width: 419, height: confirmation ? 440 : 580); hosted.layoutSubtreeIfNeeded()
                    let bitmap = try XCTUnwrap(hosted.bitmapImageRepForCachingDisplay(in: hosted.bounds)); hosted.cacheDisplay(in: hosted.bounds, to: bitmap)
                    let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:])); XCTAssertGreaterThan(data.count, 1000)
                    try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent("context-\(tab)-\(locale)-\(scheme == .dark ? "dark" : "light").png"))
                }
            }
        }
    }
}
