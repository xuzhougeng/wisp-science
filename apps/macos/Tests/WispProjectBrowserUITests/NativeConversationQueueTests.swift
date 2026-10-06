import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor ConversationQueueClient: NativeConversationQuerying {
    private var queues: [String: [SettingsValue]] = [:]
    private var outcomes: [String: [SettingsValue]] = [:]
    private var sequence: Int64 = 1
    private var writes: [(String, [String: SettingsValue], String)] = []
    private var fail = false
    private var held: CheckedContinuation<SettingsValue, Never>?
    private var holdNext = false
    private var running = true
    func recorded() -> [(String, [String: SettingsValue], String)] { writes }
    func setFailure(_ value: Bool) { fail = value }
    func setRunning(_ value: Bool) { running = value }
    func hold() { holdNext = true }
    func isHeld() -> Bool { held != nil }
    func finish() { held?.resume(returning: .object(["queued": .bool(true)])); held = nil }
    func startFirst(_ session: String) {
        if let item = queues[session]?.first {
            queues[session]?.removeFirst(); outcomes[session, default: []].append(.object(["id": item["id"], "state": .string("started")]))
        }
    }
    func editFirstElsewhere(_ session: String) {
        queues[session]?[0]["message"] = .string("edited elsewhere")
        queues[session]?[0]["digest"] = .string(String(repeating: "f", count: 64))
    }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot {
        sequence += 1
        return try ConversationSnapshot.decode(.object([
            "schema": .string(ConversationSnapshot.schemaID), "epoch": .string("queue-host"), "sequence": .integer(sequence),
            "project_id": .string(projectID), "session_id": .string(sessionID), "items": .array([]), "running": .bool(running),
            "stopping": .bool(false), "read_only": .bool(false), "model_id": .string("offline"), "approvals": .array([]),
            "composer_references": .bool(true), "queue": .object(["items": .array(queues[sessionID] ?? []), "outcomes": .array(outcomes[sessionID] ?? []), "can_cut_in": .bool(true)])
        ]), projectID: projectID, sessionID: sessionID)
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        guard ["native_conversation_enqueue", "native_conversation_queue_action", "native_conversation_attach"].contains(command) else { return .null }
        writes.append((command, args, projectID))
        let session = args["session_id"]!.string
        if command == "native_conversation_attach" { return .object(["path": .string("uploads/plot.png"), "name": .string("plot.png")]) }
        if command == "native_conversation_enqueue" {
            let id = String(UInt64.max - UInt64(writes.count))
            queues[session, default: []].append(.object(["id": .string(id), "digest": .string(String(repeating: "a", count: 64)), "state": .string("queued"), "message": args["message"] ?? .string(""), "attachments": args["attachments"] ?? .array([]), "references": args["references"] ?? .array([])]))
            if holdNext { holdNext = false; return await withCheckedContinuation { held = $0 } }
            if fail { throw ProjectBrowserError.service("response lost") }
            return .object(["queued": .bool(true), "id": .string(id)])
        }
        if fail { throw ProjectBrowserError.service("response lost") }
        guard let index = queues[session]?.firstIndex(where: { $0["id"] == args["id"] && $0["digest"] == args["digest"] }) else { throw ProjectBrowserError.service("This queued turn has already started or changed") }
        switch args["action"]?["kind"].string {
        case "edit": queues[session]?[index]["message"] = args["action"]?["message"] ?? .string(""); queues[session]?[index]["digest"] = .string(String(repeating: "b", count: 64))
        case "cancel": queues[session]?.remove(at: index)
        case "move_up": if index > 0 { queues[session]?.swapAt(index, index - 1) }
        case "move_down": if index + 1 < queues[session]!.count { queues[session]?.swapAt(index, index + 1) }
        default: throw ProjectBrowserError.invalidResponse
        }
        return .null
    }
}
final class NativeConversationQueueTests: XCTestCase {
    @MainActor func testMultipleRowsEditMoveCancelAndStartedRowsFollowHost() async throws {
        let client = ConversationQueueClient(); let model = NativeConversationModel(client: client)
        await model.open(project: "p", session: "s"); defer { model.pause() }
        let ref = NativeComposerReference(reference: .object(["kind": .string("artifact"), "id": .string("a")]), label: "Counts", detail: "counts.csv")
        await model.attach(source: "/source/plot.png", client: client); model.addReference(ref); model.draft = "first"; await model.queueFollowUp()
        XCTAssertTrue(model.draft.isEmpty); XCTAssertTrue(model.attachments.isEmpty); XCTAssertTrue(model.references.isEmpty)
        model.draft = "second"; XCTAssertTrue(model.canQueueFollowUp); await model.queueFollowUp()
        model.draft = "third"; await model.queueFollowUp()
        XCTAssertEqual(model.queuedTurns.map(\.message), ["first\n\nAttached artifacts: Counts", "second", "third"])
        let first = try XCTUnwrap(model.queuedTurns.first); XCTAssertGreaterThan(try XCTUnwrap(UInt64(first.id)), 9_007_199_254_740_992)
        let edited = await model.changeQueuedTurn(first, session: "s", action: "edit", message: "edited first"); XCTAssertTrue(edited)
        XCTAssertEqual(model.queuedTurns[0].attachments, ["uploads/plot.png"]); XCTAssertEqual(model.queuedTurns[0].references, [ref.reference])
        XCTAssertFalse(model.canChangeQueuedTurn(first), "A stale digest cannot overwrite an edit")
        let moved = await model.changeQueuedTurn(model.queuedTurns[2], session: "s", action: "move_up"); XCTAssertTrue(moved)
        XCTAssertEqual(model.queuedTurns.map(\.message), ["edited first", "third", "second"])
        let cancelled = await model.changeQueuedTurn(model.queuedTurns[1], session: "s", action: "cancel"); XCTAssertTrue(cancelled)
        XCTAssertEqual(model.queuedTurns.map(\.message), ["edited first", "second"])
        let started = model.queuedTurns[0]; await client.startFirst("s"); await model.refresh()
        XCTAssertEqual(model.queuedTurns.map(\.message), ["second"]); XCTAssertFalse(model.canChangeQueuedTurn(started))
        let old = await model.changeQueuedTurn(started, session: "s", action: "cancel"); XCTAssertFalse(old)
        let writes = await client.recorded(); XCTAssertEqual(writes.filter { $0.0 == "native_conversation_enqueue" }.count, 3)
        let edit = try XCTUnwrap(writes.first { $0.1["action"]?["kind"].string == "edit" }); XCTAssertEqual(edit.1["id"], .string(first.id)); XCTAssertEqual(edit.1["digest"], .string(first.digest))
        XCTAssertNil(edit.1["attachments"], "Editing changes text only; the host retains the captured payload")
    }
    @MainActor func testLostEnqueueAndActionNeverRetryOrAlterAnotherSession() async throws {
        let client = ConversationQueueClient(); let model = NativeConversationModel(client: client)
        await model.open(project: "p", session: "s"); defer { model.pause() }
        await client.setFailure(true); model.draft = "uncertain"; await model.queueFollowUp()
        XCTAssertTrue(model.uncertainQueue); XCTAssertFalse(model.canQueueFollowUp); XCTAssertEqual(model.draft, "uncertain")
        await model.refresh(); XCTAssertEqual(model.queuedTurns.map(\.message), ["uncertain"])
        await model.queueFollowUp(); await client.setRunning(false); await model.refresh(); XCTAssertFalse(model.canSend)
        await client.setRunning(true); await client.setFailure(false); await model.refresh()
        model.acknowledgeUncertainQueue(); XCTAssertTrue(model.canQueueFollowUp)
        let first = try XCTUnwrap(model.queuedTurns.first)
        await client.editFirstElsewhere("s")
        let saved = await model.changeQueuedTurn(first, session: "s", action: "edit", message: "stale"); XCTAssertFalse(saved)
        XCTAssertEqual(model.queuedTurns.first?.message, "edited elsewhere"); XCTAssertNotNil(model.operationError)
        await model.open(project: "p", session: "other")
        XCTAssertTrue(model.queuedTurns.isEmpty); XCTAssertFalse(model.uncertainQueue)
        let wrongSession = await model.changeQueuedTurn(first, session: "s", action: "cancel"); XCTAssertFalse(wrongSession)
        let writes = await client.recorded(); XCTAssertEqual(writes.filter { $0.0 == "native_conversation_enqueue" }.count, 1); XCTAssertEqual(writes.filter { $0.0 == "native_conversation_queue_action" }.count, 1)
    }
    @MainActor func testLateAcknowledgementClearsOnlyOriginalPayloadAndBusyState() async throws {
        let client = ConversationQueueClient(); let model = NativeConversationModel(client: client)
        await model.open(project: "p", session: "s"); defer { model.pause() }
        model.draft = "first draft"; await client.hold()
        let sending = Task { await model.queueFollowUp() }
        while !(await client.isHeld()) { await Task.yield() }
        await model.open(project: "p", session: "other"); model.draft = "other draft"
        await client.finish(); await sending.value
        XCTAssertEqual(model.draft, "other draft"); XCTAssertFalse(model.busy); XCTAssertTrue(model.queuedTurns.isEmpty)
        await model.open(project: "p", session: "s")
        XCTAssertTrue(model.draft.isEmpty); XCTAssertEqual(model.queuedTurns.first?.message, "first draft")
    }
    @MainActor func testLostEnqueueRemainsReviewableAfterReturningToItsSession() async throws {
        let client = ConversationQueueClient(); let model = NativeConversationModel(client: client)
        await model.open(project: "p", session: "s"); defer { model.pause() }
        await client.setFailure(true); model.draft = "uncertain"; await model.queueFollowUp()
        await model.open(project: "p", session: "other"); XCTAssertFalse(model.uncertainQueue)
        await model.open(project: "p", session: "s")
        XCTAssertTrue(model.uncertainQueue); XCTAssertNotNil(model.operationError); XCTAssertEqual(model.draft, "uncertain")
        XCTAssertFalse(model.canQueueFollowUp)
        let writes = await client.recorded(); XCTAssertEqual(writes.count, 1)
    }
    @MainActor func testHostedComposerReturnQueuesMultipleMessagesUnderBothPolicies() async throws {
        let defaults = UserDefaults.standard
        let previous = defaults.object(forKey: "nativeSettings.send_with_modifier")
        defer { if let previous { defaults.set(previous, forKey: "nativeSettings.send_with_modifier") } else { defaults.removeObject(forKey: "nativeSettings.send_with_modifier") } }
        _ = NSApplication.shared
        for modifier in [false, true] {
            defaults.set(modifier, forKey: "nativeSettings.send_with_modifier")
            let client = ConversationQueueClient(); let model = NativeConversationModel(client: client)
            await model.open(project: "p", session: "s")
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 800, height: 700), styleMask: [.titled], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            let host = NSHostingView(rootView: NativeConversationView(conversation: model, projectID: "p", sessionID: "s")); host.frame = window.contentView!.bounds; window.contentView = host; host.layoutSubtreeIfNeeded()
            func find(_ view: NSView) -> NativeComposerTextView? { (view as? NativeComposerTextView) ?? view.subviews.compactMap(find).first }
            let editor = try XCTUnwrap(find(host)); XCTAssertTrue(window.makeFirstResponder(editor))
            func enter(_ flags: NSEvent.ModifierFlags) -> NSEvent {
                NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: flags, timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\r", charactersIgnoringModifiers: "\r", isARepeat: false, keyCode: 36)!
            }
            for text in ["first", "second"] {
                editor.insertText(text, replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
                editor.keyDown(with: enter(modifier ? .command : []))
                for _ in 0..<100 where model.busy || model.draft == text { try await Task.sleep(nanoseconds: 1_000_000) }
                host.layoutSubtreeIfNeeded()
                XCTAssertTrue(model.draft.isEmpty)
            }
            XCTAssertEqual(model.queuedTurns.map(\.message), ["first", "second"])
            editor.insertText("newline", replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
            editor.keyDown(with: enter(.shift)); XCTAssertTrue(editor.string.hasSuffix("\n"))
            let writes = await client.recorded(); XCTAssertEqual(writes.filter { $0.0 == "native_conversation_enqueue" }.count, 2)
            window.close(); model.pause()
        }
    }
    @MainActor func testImmediateEscapeClosesOnlyQueuedEditorWithoutSubmitting() async throws {
        let client = ConversationQueueClient(); let model = NativeConversationModel(client: client)
        await model.open(project: "p", session: "s"); defer { model.pause() }
        model.draft = "editable"; await model.queueFollowUp(); let item = try XCTUnwrap(model.queuedTurns.first)
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 520, height: 360), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let parentView = NSView(frame: window.contentView!.bounds); window.contentView = parentView
        var parentClosed = 0, editorClosed = 0
        let parent = NativeSettingsEscape.Coordinator(enabled: true) { parentClosed += 1 }; parent.view = parentView; parent.install(); defer { parent.remove() }
        let hosted = NSHostingView(rootView: NativeQueuedTurnEditor(conversation: model, item: item, session: "s") { editorClosed += 1 })
        hosted.frame = parentView.bounds; parentView.addSubview(hosted); hosted.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let escape = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(escape, keyWindow: window, modalWindow: nil)); XCTAssertEqual(editorClosed, 1); XCTAssertEqual(parentClosed, 0); XCTAssertTrue(window.firstResponder === focus)
        hosted.removeFromSuperview(); XCTAssertTrue(NativeEscapeStack.shared.consume(escape, keyWindow: window, modalWindow: nil)); XCTAssertEqual(parentClosed, 1)
        let writes = await client.recorded(); XCTAssertFalse(writes.contains { $0.0 == "native_conversation_queue_action" })
    }
    @MainActor func testRenderQueue() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Opt-in rendering") }
        let client = ConversationQueueClient(); let model = NativeConversationModel(client: client)
        await model.open(project: "p", session: "s"); defer { model.pause() }
        for text in ["检查图 1 中的样本计数", "比较多个样本并保留所有参考文件", "接下来生成结果摘要"] { model.draft = text; await model.queueFollowUp() }
        for (name, scheme, width) in [("message-queue-light", ColorScheme.light, 720.0), ("message-queue-dark-narrow", ColorScheme.dark, 360.0)] {
            let root = NativeConversationQueueView(conversation: model, session: "s").padding(20).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading).background(WispDesign.color("bg-app", scheme)).environment(\.colorScheme, scheme)
            let view = NSHostingView(rootView: root); view.frame = NSRect(x: 0, y: 0, width: width, height: 250); view.layoutSubtreeIfNeeded()
            let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds)); view.cacheDisplay(in: view.bounds, to: bitmap)
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: URL(fileURLWithPath: directory).appendingPathComponent(name + ".png"))
        }
    }
}
