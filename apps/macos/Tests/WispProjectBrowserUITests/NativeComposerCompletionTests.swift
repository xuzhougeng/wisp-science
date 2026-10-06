import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor CompletionClient: NativeConversationQuerying {
    var calls: [(String, [String: SettingsValue], String)] = []
    var held: [String: CheckedContinuation<SettingsValue, Never>] = [:]
    var heldQueries: Set<String> = []
    var empty = false
    var wrongSession = false
    var running = false
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        var value = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-conversations/v1/snapshot.json")))
        value["project_id"] = .string(projectID); value["session_id"] = .string(sessionID)
        value["running"] = .bool(running); value["read_only"] = .bool(false); value["composer_references"] = .bool(true)
        value["approvals"] = .array([])
        return try ConversationSnapshot.decode(value, projectID: projectID, sessionID: sessionID)
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        calls.append((command, args, projectID))
        if command == "native_conversation_panel_contexts" {
            return .object(["contexts": .array([.object(["id": .string("local"), "kind": .string("local"), "label": .string("Local"), "config_json": .string("{}"), "capabilities_json": .string("{}")])]), "enabled_ids": .array([]), "read_only": .bool(false)])
        }
        if command == "get_default_execution_context" { return .string("local") }
        if command == "native_conversation_panel_activity" { return .object(["runtimes": .array([]), "runs": .array([]), "read_only": .bool(false)]) }
        if command != "native_conversation_references" { return command.hasPrefix("list_") ? .array([]) : .null }
        let query = args["query"]!.string
        if heldQueries.contains(query) { return await withCheckedContinuation { held[query] = $0 } }
        return response(session: args["session_id"]!.string, kind: args["kind"]!.string, query: query)
    }
    func response(session: String, kind: String, query: String) -> SettingsValue {
        let refs = empty ? [] : (0..<3).map { i in SettingsValue.object([
            "reference": .object(["kind": .string(kind), kind == "skill" ? "name" : "id": .string(query + "-" + String(i))]),
            "label": .string(query + " result " + String(i)), "detail": .string("project / results/qc.csv")
        ]) }
        return .object(["session_id": .string(wrongSession ? "foreign" : session), "options": .array(refs)])
    }
    func hold(_ query: String) { heldQueries.insert(query) }
    func isHeld(_ query: String) -> Bool { held[query] != nil }
    func finish(_ query: String, session: String = "a") { held.removeValue(forKey: query)?.resume(returning: response(session: session, kind: "artifact", query: query)) }
    func configure(empty: Bool = false, wrongSession: Bool = false) { self.empty = empty; self.wrongSession = wrongSession }
    func setRunning() { running = true }
    func recorded() -> [(String, [String: SettingsValue], String)] { calls }
}

/// A fake input method receives events through NSTextView's real interpreter
/// boundary. It holds marked text until the test explicitly commits it.
private class CompletionIMETextView: NativeComposerTextView {
    var compositionKeys: [UInt16] = []
    override func interpretKeyEvents(_ eventArray: [NSEvent]) {
        if hasMarkedText() { compositionKeys += eventArray.map(\.keyCode) }
        else { super.interpretKeyEvents(eventArray) }
    }
}

final class NativeComposerCompletionTests: XCTestCase {
    func testTriggerMatchesWebViewWordBoundariesAndUTF16Offsets() {
        func trigger(_ text: String, caret: Int? = nil) -> NativeComposerTrigger? {
            NativeComposerTrigger.active(in: text, selection: NSRange(location: caret ?? text.utf16.count, length: 0))
        }
        XCTAssertEqual(trigger("look at @qc"), .init(range: NSRange(location: 8, length: 3), kind: "artifact", query: "qc"))
        XCTAssertEqual(trigger("#old"), .init(range: NSRange(location: 0, length: 4), kind: "session", query: "old"))
        XCTAssertEqual(trigger("/literature-review")?.kind, "skill")
        XCTAssertEqual(trigger("🧬中文@样本")?.range, NSRange(location: 4, length: 3))
        XCTAssertEqual(trigger("🧬中文#旧记录 后文", caret: 8)?.query, "旧记录")
        for text in ["a@example", "prefix#tag", "x/2", "https://example", "./file", "dir/file", "\\/file", "@a b", "@a\nb"] { XCTAssertNil(trigger(text), text) }
        XCTAssertNil(trigger("🧬@", caret: 1))
        XCTAssertNil(trigger("@", caret: 2))
        XCTAssertNil(NativeComposerTrigger.active(in: "@qc", selection: NSRange(location: 0, length: 3)))
    }
    @MainActor private func editor(client: CompletionClient) -> (NSWindow, NativeComposerTextView, NativeComposerCompletionModel) {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 500, height: 360), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        let editor = CompletionIMETextView(frame: NSRect(x: 20, y: 20, width: 460, height: 90))
        editor.isRichText = false; editor.isEditable = true; editor.isSelectable = true; editor.allowsUndo = true
        window.contentView?.addSubview(editor); window.makeFirstResponder(editor)
        let model = NativeComposerCompletionModel(client: client)
        model.bind(project: "project", session: "a"); model.editor = editor
        editor.completions = model; editor.referencesAvailable = { true }; editor.selectReference = { _ in true }
        return (window, editor, model)
    }
    @MainActor private func type(_ text: String, in editor: NativeComposerTextView) { editor.insertText(text, replacementRange: NSRange(location: NSNotFound, length: 0)) }
    @MainActor private func key(_ code: UInt16, in window: NSWindow, flags: NSEvent.ModifierFlags = []) -> NSEvent {
        NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: flags, timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: code == 36 ? "\r" : "", charactersIgnoringModifiers: "", isARepeat: false, keyCode: code)!
    }
    @MainActor private func waitForSearch(_ model: NativeComposerCompletionModel) async throws {
        for _ in 0..<400 {
            if !model.searching { return }
            try await Task.sleep(nanoseconds: 5_000_000)
        }
        XCTFail("Candidate search did not finish")
    }
    @MainActor private func waitUntilHeld(_ query: String, client: CompletionClient) async throws {
        for _ in 0..<400 {
            if await client.isHeld(query) { return }
            try await Task.sleep(nanoseconds: 5_000_000)
        }
        XCTFail("Expected held search")
    }
    @MainActor func testArrowsWrapAndEnterTabConfirmInsteadOfSendingUnderBothPolicies() async throws {
        for modifier in [false, true] {
            for confirm: UInt16 in [36, 76, 48] {
                let client = CompletionClient(); let (window, editor, model) = editor(client: client)
                defer { model.dismiss(); window.close() }
                var sends = 0; var selected: NativeComposerReference?
                editor.sendWithModifier = modifier; editor.canSubmit = { true }; editor.submit = { sends += 1 }
                editor.selectReference = { selected = $0; return true }
                type("@", in: editor); type("qc", in: editor); try await waitForSearch(model)
                editor.keyDown(with: key(126, in: window)); XCTAssertEqual(model.selected, 2)
                editor.keyDown(with: key(125, in: window)); XCTAssertEqual(model.selected, 0)
                editor.keyDown(with: key(125, in: window)); XCTAssertEqual(model.selected, 1)
                editor.keyDown(with: key(confirm, in: window, flags: modifier ? .command : []))
                XCTAssertEqual(selected?.reference["id"], .string("qc-1")); XCTAssertEqual(sends, 0)
                XCTAssertEqual(editor.string, ""); XCTAssertFalse(model.isOpen)
                XCTAssertTrue(window.firstResponder === editor)
            }
        }
    }
    @MainActor func testConfirmPreservesSurroundingUnicodeAndRestoresCaretAtRemovedToken() async throws {
        let client = CompletionClient(); let (window, editor, model) = editor(client: client)
        defer { model.dismiss(); window.close() }
        var binding = ""; editor.onChange = { binding = $0 }
        editor.apply("🧬中文 后文"); editor.setSelectedRange(NSRange(location: 4, length: 0))
        type("@", in: editor); type("样本", in: editor); try await waitForSearch(model)
        model.accept()
        XCTAssertEqual(editor.string, "🧬中文 后文"); XCTAssertEqual(binding, editor.string)
        XCTAssertEqual(editor.selectedRange(), NSRange(location: 4, length: 0))
        type("补充", in: editor); XCTAssertEqual(editor.string, "🧬中文补充 后文")
    }
    @MainActor func testLoadingAndEmptyResultsConsumeReturnWithoutChangingDraft() async throws {
        let client = CompletionClient(); await client.hold("")
        let (window, editor, model) = editor(client: client); defer { model.dismiss(); window.close() }
        var sends = 0; editor.canSubmit = { true }; editor.submit = { sends += 1 }
        type("@", in: editor); try await waitUntilHeld("", client: client)
        editor.keyDown(with: key(36, in: window)); editor.keyDown(with: key(48, in: window))
        XCTAssertEqual(sends, 0); XCTAssertEqual(editor.string, "@")
        await client.configure(empty: true); await client.finish(""); try await waitForSearch(model)
        editor.keyDown(with: key(36, in: window)); XCTAssertEqual(sends, 0); XCTAssertTrue(model.isOpen)
        model.dismiss(); type(" question", in: editor); editor.keyDown(with: key(36, in: window)); XCTAssertEqual(sends, 1)
    }
    @MainActor func testIMEConfirmationDoesNotSelectSendOrReplaceMarkedText() async throws {
        let client = CompletionClient(); let (window, editor, model) = editor(client: client)
        defer { model.dismiss(); window.close() }
        var sends = 0, selections = 0
        editor.canSubmit = { true }; editor.submit = { sends += 1 }; editor.selectReference = { _ in selections += 1; return true }
        type("@", in: editor); try await waitForSearch(model)
        editor.setMarkedText("hou", selectedRange: NSRange(location: 3, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0))
        editor.setMarkedText("候选", selectedRange: NSRange(location: 2, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertTrue(editor.hasMarkedText()); XCTAssertFalse(model.isOpen)
        XCTAssertFalse(NativeEscapeStack.shared.consume(key(53, in: window), keyWindow: window, modalWindow: nil))
        editor.apply("other draft"); XCTAssertEqual(editor.string, "@候选")
        editor.keyDown(with: key(36, in: window)); XCTAssertEqual(sends, 0); XCTAssertEqual(selections, 0)
        editor.keyDown(with: key(125, in: window)); editor.keyDown(with: key(48, in: window))
        XCTAssertEqual((editor as? CompletionIMETextView)?.compositionKeys, [36, 125, 48])
        XCTAssertEqual(editor.string, "@候选")
        if editor.hasMarkedText() { editor.unmarkText() }
        try await waitForSearch(model)
        XCTAssertTrue(model.isOpen); XCTAssertEqual(model.trigger?.query, "候选")
        model.accept(); XCTAssertEqual(selections, 1); XCTAssertEqual(sends, 0)
    }
    @MainActor func testImmediateWindowEscapeClosesOnlyCandidateAndInvalidatesPendingRead() async throws {
        let client = CompletionClient(); await client.hold("")
        let (window, editor, model) = editor(client: client); defer { model.dismiss(); window.close() }
        var parentClosed = 0
        let parent = NativeSettingsEscape.Coordinator(enabled: true) { parentClosed += 1 }
        parent.view = window.contentView; parent.install(); defer { parent.remove() }
        type("@", in: editor); try await waitUntilHeld("", client: client)
        let focus = window.firstResponder
        XCTAssertTrue(NativeEscapeStack.shared.consume(key(53, in: window), keyWindow: window, modalWindow: nil))
        XCTAssertFalse(model.isOpen); XCTAssertEqual(parentClosed, 0); XCTAssertEqual(editor.string, "@")
        XCTAssertTrue(window.firstResponder === focus)
        await client.finish(""); try await Task.sleep(nanoseconds: 20_000_000)
        XCTAssertTrue(model.candidates.isEmpty)
        XCTAssertTrue(NativeEscapeStack.shared.consume(key(53, in: window), keyWindow: window, modalWindow: nil)); XCTAssertEqual(parentClosed, 1)
        type("q", in: editor); XCTAssertFalse(model.isOpen, "Escape must not reopen an old token")
    }
    @MainActor func testNewQueryAndSessionRejectLateResults() async throws {
        let client = CompletionClient(); await client.hold("old")
        let (window, editor, model) = editor(client: client); defer { model.dismiss(); window.close() }
        type("@", in: editor); type("old", in: editor); try await waitUntilHeld("old", client: client)
        editor.deleteBackward(nil); type("new", in: editor); try await waitForSearch(model)
        XCTAssertEqual(model.candidates.first?.id, "artifact:olnew-0")
        await client.finish("old"); try await Task.sleep(nanoseconds: 20_000_000)
        XCTAssertEqual(model.candidates.first?.id, "artifact:olnew-0")
        model.dismiss(); editor.apply(""); type("@", in: editor); type("old", in: editor); try await waitUntilHeld("old", client: client)
        model.bind(project: "other-project", session: "b"); editor.apply("new conversation")
        await client.finish("old"); try await Task.sleep(nanoseconds: 20_000_000)
        XCTAssertTrue(model.candidates.isEmpty); XCTAssertEqual(editor.string, "new conversation")
        editor.apply(""); type("#", in: editor); try await waitForSearch(model)
        let calls = await client.recorded(); XCTAssertEqual(calls.last?.1["session_id"], .string("b")); XCTAssertEqual(calls.last?.2, "other-project")
    }
    @MainActor func testCaretMovesSelectionsAndExternalDraftsDismissWithoutReopening() async throws {
        let client = CompletionClient(); let (window, editor, model) = editor(client: client); defer { model.dismiss(); window.close() }
        editor.apply("@pasted"); XCTAssertFalse(model.isOpen)
        editor.apply(""); type("@", in: editor); type("qc", in: editor); try await waitForSearch(model)
        editor.setSelectedRange(NSRange(location: 0, length: 0)); XCTAssertFalse(model.isOpen)
        editor.setSelectedRange(NSRange(location: 3, length: 0)); XCTAssertFalse(model.isOpen)
        type(" ", in: editor); type("#", in: editor); try await waitForSearch(model)
        editor.setSelectedRange(NSRange(location: 0, length: 5)); XCTAssertFalse(model.isOpen)
        editor.apply("restored #draft"); XCTAssertFalse(model.isOpen)
    }
    @MainActor func testPasteboardInsertionDoesNotOpenOrContinueATrigger() async throws {
        let client = CompletionClient(); let (window, editor, model) = editor(client: client); defer { model.dismiss(); window.close() }
        let clipboard = NSPasteboard.withUniqueName(); defer { clipboard.releaseGlobally() }
        clipboard.setString("@", forType: .string)
        XCTAssertTrue(editor.readSelection(from: clipboard, type: .string))
        XCTAssertEqual(editor.string, "@"); XCTAssertFalse(model.isOpen)
        editor.apply(""); type("@", in: editor); try await waitForSearch(model)
        clipboard.clearContents(); clipboard.setString("粘贴内容", forType: .string)
        XCTAssertTrue(editor.readSelection(from: clipboard, type: .string))
        XCTAssertEqual(editor.string, "@粘贴内容"); XCTAssertFalse(model.isOpen)
    }
    @MainActor func testMouseClicksPreserveCandidateRowsAndDismissOnlyOutsideItsWindowRegions() async throws {
        let client = CompletionClient(); let (window, editor, model) = editor(client: client); defer { model.dismiss(); window.close() }
        let list = NSView(frame: NSRect(x: 20, y: 130, width: 460, height: 180)); window.contentView?.addSubview(list); model.listBounds = list
        let other = NSWindow(contentRect: window.frame, styleMask: [.titled], backing: .buffered, defer: false)
        other.isReleasedWhenClosed = false; defer { other.close() }
        func click(_ point: NSPoint, _ target: NSWindow) -> NSEvent {
            NSEvent.mouseEvent(with: .leftMouseDown, location: point, modifierFlags: [], timestamp: 0, windowNumber: target.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!
        }
        type("@", in: editor); try await waitForSearch(model)
        model.mouseDown(click(NSPoint(x: 100, y: 200), window)); XCTAssertTrue(model.isOpen)
        model.mouseDown(click(NSPoint(x: 100, y: 60), window)); XCTAssertTrue(model.isOpen)
        model.mouseDown(click(NSPoint(x: 490, y: 340), other)); XCTAssertTrue(model.isOpen)
        model.mouseDown(click(NSPoint(x: 490, y: 340), window)); XCTAssertFalse(model.isOpen)
        XCTAssertEqual(editor.string, "@"); XCTAssertTrue(window.firstResponder === editor)
    }
    @MainActor func testOldHostReadonlyAndForeignCatalogDoNotAttachReferences() async throws {
        let client = CompletionClient(); let (window, editor, model) = editor(client: client); defer { model.dismiss(); window.close() }
        editor.referencesAvailable = { false }; type("@", in: editor); XCTAssertFalse(model.isOpen)
        editor.apply(""); editor.referencesAvailable = { true }; await client.configure(wrongSession: true)
        type("@", in: editor); try await waitForSearch(model)
        XCTAssertNotNil(model.error); XCTAssertTrue(model.candidates.isEmpty)
        await client.configure(); model.dismiss(); editor.apply(""); type("@", in: editor); try await waitForSearch(model)
        editor.isEditable = false; model.accept(); XCTAssertEqual(editor.string, "@"); XCTAssertFalse(model.isOpen)
        editor.isEditable = true; editor.apply(""); type("@", in: editor); try await waitForSearch(model)
        editor.referencesAvailable = { false }; model.accept(); XCTAssertEqual(editor.string, "@"); XCTAssertFalse(model.isOpen)
    }
    @MainActor func testSlashCommandsPrecedeReferencesAndKeepActionsOutOfModelSend() async throws {
        let client = CompletionClient(); let (window, editor, model) = editor(client: client); defer { model.dismiss(); window.close() }
        editor.completionCommands = [.btw, .files, .upload]
        var actions: [NativeComposerCommand] = []; var sends = 0
        editor.executeCommand = { command, _ in actions.append(command) }; editor.canSubmit = { true }; editor.submit = { sends += 1 }
        type("/", in: editor); try await waitForSearch(model)
        XCTAssertEqual(Array(model.candidates.prefix(3)), [.command(.btw), .command(.files), .command(.upload)])
        model.accept(1); XCTAssertEqual(actions, [.files]); XCTAssertEqual(sends, 0); XCTAssertEqual(editor.string, "")
        editor.apply("中文 后文"); editor.setSelectedRange(NSRange(location: 2, length: 0))
        type("/", in: editor); type("bt", in: editor); try await waitForSearch(model)
        model.accept(); XCTAssertEqual(editor.string, "中文/btw  后文"); XCTAssertEqual(editor.selectedRange().location, 7)
        XCTAssertEqual(actions, [.files]); XCTAssertEqual(sends, 0)
        XCTAssertEqual(NativeComposerCommand.parse(" /btw 中文问题 ")?.1, "中文问题")
        XCTAssertNil(NativeComposerCommand.parse("/path/to/file")); XCTAssertNil(NativeComposerCommand.parse("/btw2"))
        XCTAssertEqual(Set(NativeComposerCommand.allCases.map(\.icon)).count, NativeComposerCommand.allCases.count)
    }
    @MainActor func testDraftsReferencesAndPendingSearchStayWithTheirConversation() async throws {
        let client = CompletionClient(); let conversation = NativeConversationModel(client: client)
        await conversation.open(project: "project", session: "a"); defer { conversation.pause() }
        let (window, editor, detached) = editor(client: client); defer { detached.dismiss(); window.close() }
        let model = conversation.completions; model.editor = editor; editor.completions = model
        editor.onChange = { conversation.draft = $0 }; editor.selectReference = conversation.addReference
        type("#", in: editor); type("old", in: editor); try await waitForSearch(model); model.accept()
        let selected = conversation.references
        XCTAssertEqual(selected.first?.id, "session:old-0")
        type("draft A ", in: editor); await client.hold("slow")
        type("@", in: editor); type("slow", in: editor); try await waitUntilHeld("slow", client: client)
        await conversation.open(project: "project", session: "b")
        editor.apply(conversation.draft); type("draft B", in: editor)
        await client.finish("slow"); try await Task.sleep(nanoseconds: 20_000_000)
        XCTAssertEqual(conversation.draft, "draft B"); XCTAssertTrue(conversation.references.isEmpty); XCTAssertFalse(model.isOpen)
        await conversation.open(project: "project", session: "a")
        XCTAssertEqual(conversation.draft, "draft A @slow"); XCTAssertEqual(conversation.references, selected); XCTAssertFalse(model.isOpen)
    }
    @MainActor func testTypedCommandsDispatchWithoutMainSendAndKeepAttachedReferences() async throws {
        let client = CompletionClient(); let conversation = NativeConversationModel(client: client)
        await conversation.open(project: "project", session: "a"); defer { conversation.pause() }
        let ref = NativeComposerReference(reference: .object(["kind": .string("artifact"), "id": .string("qc")]), label: "QC", detail: "results/qc.csv")
        conversation.addReference(ref); conversation.draft = "/btw 解释这个峰"
        var called: [(NativeComposerCommand, String)] = []
        XCTAssertTrue(conversation.runComposerCommand(available: [.btw], execute: { called.append(($0, $1)) }))
        XCTAssertEqual(called.first?.0, .btw); XCTAssertEqual(called.first?.1, "解释这个峰")
        XCTAssertEqual(conversation.draft, ""); XCTAssertEqual(conversation.references, [ref])
        conversation.draft = "/path/to/file"; XCTAssertFalse(conversation.runComposerCommand(available: [.files], execute: { called.append(($0, $1)) }))
        XCTAssertEqual(conversation.draft, "/path/to/file")
        let calls = await client.recorded(); XCTAssertFalse(calls.contains { $0.0 == "native_conversation_send" })
    }
    @MainActor func testSideChatCommandRemainsIndependentOfRunningMainTurn() async throws {
        let client = CompletionClient(); await client.setRunning()
        let conversation = NativeConversationModel(client: client)
        await conversation.open(project: "project", session: "a"); defer { conversation.pause() }
        conversation.draft = "/btw 这个峰是什么？"
        XCTAssertFalse(conversation.canSend); XCTAssertTrue(conversation.canRunComposerCommand(available: [.btw]))
        var question = ""
        XCTAssertTrue(conversation.runComposerCommand(available: [.btw], execute: { _, payload in question = payload }))
        XCTAssertEqual(question, "这个峰是什么？"); XCTAssertEqual(conversation.draft, "")
        let calls = await client.recorded()
        XCTAssertFalse(calls.contains { ["native_conversation_send", "native_conversation_enqueue"].contains($0.0) })
    }
    @MainActor func testRenderInlineCandidatesWithinDesktopAndNarrowWindows() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Set WISP_NATIVE_SNAPSHOT_DIR for native candidate layout smoke") }
        try FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
        for (name, width, scheme, symbol) in [("artifacts", 760.0, ColorScheme.light, "@"), ("commands", 760.0, ColorScheme.light, "/"), ("narrow", 419.0, ColorScheme.light, "#"), ("dark", 760.0, ColorScheme.dark, "/")] {
            let client = CompletionClient(); let conversation = NativeConversationModel(client: client)
            await conversation.open(project: "project", session: "a")
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: width, height: name == "narrow" ? 538 : 620), styleMask: [.titled], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            defer { conversation.pause(); window.close() }
            let host = NSHostingView(rootView: NativeConversationView(conversation: conversation, projectID: "project", sessionID: "a", executeComposerCommand: { _, _ in })
                .background(WispDesign.color("bg-app", scheme)).foregroundStyle(WispDesign.color("text", scheme)).environment(\.colorScheme, scheme))
            host.frame = window.contentView!.bounds; window.contentView = host; host.layoutSubtreeIfNeeded()
            func find(_ view: NSView) -> NativeComposerTextView? {
                if let editor = view as? NativeComposerTextView { return editor }
                return view.subviews.compactMap(find).first
            }
            let editor = try XCTUnwrap(find(host)); window.makeFirstResponder(editor)
            type(symbol, in: editor); try await waitForSearch(conversation.completions)
            if name == "commands" { conversation.completions.move(6) }
            try await Task.sleep(nanoseconds: 180_000_000); host.layoutSubtreeIfNeeded()
            let bounds = try XCTUnwrap(conversation.completions.listBounds)
            let rect = bounds.convert(bounds.bounds, to: host)
            XCTAssertGreaterThan(rect.height, 150); XCTAssertGreaterThan(rect.width, 300)
            XCTAssertGreaterThanOrEqual(rect.minY, 0); XCTAssertLessThanOrEqual(rect.maxY, host.bounds.height)
            let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: bitmap)
            let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
            try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent("composer-completion-" + name + ".png"))
        }
    }
}
