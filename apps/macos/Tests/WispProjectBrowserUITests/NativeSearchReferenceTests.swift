import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor SearchReferenceFake: NativeConversationQuerying {
    var mode = "", hold = false
    var held: CheckedContinuation<Void, Never>?
    var calls: [(String, [String: SettingsValue], String)] = []
    func setup(_ mode: String = "", hold: Bool = false) { self.mode = mode; self.hold = hold }
    func waiting() -> Bool { held != nil }
    func release() { held?.resume(); held = nil; hold = false }
    func recorded() -> [(String, [String: SettingsValue], String)] { calls }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot {
        var root = URL(fileURLWithPath: #filePath); for _ in 0..<5 { root.deleteLastPathComponent() }
        var value = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-conversations/v1/snapshot.json")))
        value["project_id"] = .string(projectID); value["session_id"] = .string(sessionID); value["read_only"] = .bool(false); value["running"] = .bool(false); value["stopping"] = .bool(false); value["composer_references"] = .bool(true); value["approvals"] = .array([])
        return try ConversationSnapshot.decode(value, projectID: projectID, sessionID: sessionID)
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        calls.append((command, args, projectID))
        if command == "list_models" || command == "list_acp_agents" { return .array([]) }
        if command == "get_appearance_prefs" || command == "native_conversation_seen" { return .null }
        if hold { await withCheckedContinuation { held = $0 } }
        if mode == "failed" { throw ProjectBrowserError.service("unavailable") }
        let option: SettingsValue = .object(["reference": .object(["kind": args["kind"]!, "id": .string(mode == "missing" ? "other" : "exact-id")]), "label": .string("Fresh label"), "detail": .string("Source project")])
        return .object(["session_id": mode == "wrong-session" ? .string("other") : args["session_id"]!, "options": .array(mode == "hidden" ? [] : [option])])
    }
}

private struct SearchReferenceBrowser: ProjectBrowserQuerying {
    func listProjects(databaseURL: URL) async throws -> ProjectListSnapshot { ProjectListSnapshot(projects: [], activitySource: "persisted_only") }
    func listSessions(databaseURL: URL, projectID: String?) async throws -> [BrowserSession] {
        [BrowserSession(id: "draft-session", projectID: "destination-project", title: "Draft", ts: 1, status: "idle")]
    }
    func transcript(databaseURL: URL, projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> TranscriptPage { TranscriptPage(messages: [], nextBeforeSeq: nil) }
}

final class NativeSearchReferenceTests: XCTestCase {
    @MainActor func testSearchStagingRestoresComposerAfterDismissalAndNavigationOrReopeningCancelsIt() async throws {
        _ = NSApplication.shared
        let database = URL(fileURLWithPath: "/unused"), fake = SearchReferenceFake()
        let browser = ProjectBrowserModel(client: SearchReferenceBrowser(), databaseURL: database)
        let conversation = NativeConversationModel(client: fake)
        await browser.openProject("destination-project", sessionID: "draft-session")
        await conversation.open(project: "destination-project", session: "draft-session"); defer { conversation.pause() }
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 500, height: 300), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let editor = NativeComposerTextView(frame: NSRect(x: 0, y: 0, width: 300, height: 100))
        editor.isEditable = true; editor.string = "Unsent question"; window.contentView!.addSubview(editor)
        conversation.completions.editor = editor; conversation.draft = editor.string
        for state in ["dismissed", "navigate-and-return", "new-search", "settings"] {
            browser.searchPresented = true
            let references = NativeSearchReferenceModel(client: fake, project: "destination-project", session: "draft-session", writable: { conversation.canReference }, accept: {
                browser.stageSearchReference($0, conversation: conversation, database: database, project: "destination-project", session: "draft-session")
            })
            let staged = await references.attach(try item()); XCTAssertTrue(staged)
            browser.searchPresented = false
            let focus = try XCTUnwrap(browser.consumeSearchComposerFocus())
            switch state {
            case "navigate-and-return": browser.goHome(); await browser.openProject("destination-project", sessionID: "draft-session")
            case "new-search": browser.searchPresented = true; browser.searchPresented = false
            case "settings": browser.settingsPresented = true
            default: break
            }
            XCTAssertEqual(focus.restore(keyWindow: window), state == "dismissed", state)
            XCTAssertNil(browser.consumeSearchComposerFocus()); browser.settingsPresented = false
        }
        XCTAssertEqual(conversation.references.count, 1); XCTAssertEqual(conversation.draft, "Unsent question"); XCTAssertEqual(editor.string, conversation.draft)
        let reference = try XCTUnwrap(conversation.references.first)
        XCTAssertFalse(browser.stageSearchReference(reference, conversation: conversation, database: URL(fileURLWithPath: "/old-db"), project: "destination-project", session: "draft-session"))
        XCTAssertFalse(browser.stageSearchReference(reference, conversation: conversation, database: database, project: "other-project", session: "draft-session"))
        await conversation.open(project: "other-project", session: "draft-session")
        XCTAssertFalse(browser.stageSearchReference(reference, conversation: conversation, database: database, project: "destination-project", session: "draft-session"))
        XCTAssertNil(browser.consumeSearchComposerFocus())
    }
    @MainActor func testStagedSearchReferenceKeepsDraftAndDeduplicatesInTheActualConversation() async throws {
        let fake = SearchReferenceFake(), conversation = NativeConversationModel(client: fake)
        await conversation.open(project: "destination-project", session: "draft-session"); defer { conversation.pause() }
        XCTAssertTrue(conversation.canReference, conversation.connectionError ?? "References unavailable")
        conversation.draft = "Unsent question"
        let model = NativeSearchReferenceModel(client: fake, project: "destination-project", session: "draft-session", writable: { conversation.canReference }, accept: conversation.addReference)
        let selected = try item(); let first = await model.attach(selected), duplicate = await model.attach(selected)
        XCTAssertTrue(first); XCTAssertTrue(duplicate); XCTAssertEqual(conversation.references.count, 1)
        XCTAssertEqual(try XCTUnwrap(conversation.references.first).reference["id"], .string("exact-id")); XCTAssertEqual(conversation.draft, "Unsent question")
        let calls = await fake.recorded(); XCTAssertFalse(calls.contains { $0.0.contains("send") || $0.0.contains("queue") })
    }
    private func item(_ kind: String = "artifact", title: String = "Search label") throws -> NativeSearchItem {
        let value: SettingsValue = .object(["kind": .string(kind), "id": .string("exact-id"), "project_id": .string("source-project"), "project_name": .string("Source"), "title": .string(title), "detail": .string("detail"), "session_id": .string(kind == "session" ? "exact-id" : "owning-session")])
        return try JSONDecoder().decode(NativeSearchItem.self, from: JSONEncoder().encode(value))
    }
    @MainActor func testSearchReferencesUseFreshAdvertisedIDsWithoutNavigatingSendingOrChangingDraft() async throws {
        let fake = SearchReferenceFake(); var draft = "Unsent question", staged: [NativeComposerReference] = []
        let model = NativeSearchReferenceModel(client: fake, project: "destination-project", session: "draft-session", writable: { true }, accept: { staged.append($0); return true })
        for kind in ["artifact", "session"] { let added = await model.attach(try item(kind)); XCTAssertTrue(added) }
        XCTAssertEqual(staged.map { $0.reference["kind"].string }, ["artifact", "session"])
        XCTAssertEqual(staged.map { $0.reference["id"].string }, ["exact-id", "exact-id"])
        XCTAssertTrue(staged.allSatisfy { $0.label == "Fresh label" }); XCTAssertEqual(draft, "Unsent question"); draft = "Still unsent"
        let calls = await fake.recorded(); XCTAssertEqual(calls.count, 2); XCTAssertTrue(calls.allSatisfy { $0.0 == "native_conversation_references" && $0.2 == "destination-project" && $0.1["session_id"] == .string("draft-session") })
    }
    @MainActor func testUnavailableHiddenMissingAndWrongOwnerResponsesNeverStageReferences() async throws {
        for failure in ["failed", "hidden", "missing", "wrong-session"] {
            let fake = SearchReferenceFake(); var accepted = 0
            let model = NativeSearchReferenceModel(client: fake, project: "p", session: "s", writable: { true }, accept: { _ in accepted += 1; return true })
            await fake.setup(failure); let added = await model.attach(try item())
            XCTAssertFalse(added); XCTAssertEqual(accepted, 0); XCTAssertNotNil(model.error); XCTAssertFalse(model.reading)
        }
        let fake = SearchReferenceFake()
        let disabled = NativeSearchReferenceModel(client: fake, project: "p", session: "s", writable: { false }, accept: { _ in XCTFail("Read-only draft cannot stage"); return true })
        let added = await disabled.attach(try item()); XCTAssertFalse(added)
        let calls = await fake.recorded(); XCTAssertTrue(calls.isEmpty)
    }
    @MainActor func testClosedCancelledSupersededAndChangedScopeReadsCannotStageIntoAnotherDraft() async throws {
        for state in ["closed", "cancelled", "superseded", "scope"] {
            let fake = SearchReferenceFake(); var writable = true, accepted = 0
            let model = NativeSearchReferenceModel(client: fake, project: "p", session: "s", writable: { writable }, accept: { _ in accepted += 1; return true })
            await fake.setup(hold: true); let selected = try item()
            let read = Task { await model.attach(selected) }; while !(await fake.waiting()) { await Task.yield() }
            switch state { case "closed": model.close(); case "cancelled": read.cancel(); case "superseded": model.invalidate(); default: writable = false }
            await fake.release(); let added = await read.value
            XCTAssertFalse(added); XCTAssertEqual(accepted, 0); XCTAssertNil(model.error); XCTAssertFalse(model.reading)
        }
    }
    @MainActor func testReadsAreOneAtATimeAndLongUnicodeTitleIsBoundedAtScalarBoundary() async throws {
        let fake = SearchReferenceFake(); var accepted = 0
        let model = NativeSearchReferenceModel(client: fake, project: "p", session: "s", writable: { true }, accept: { _ in accepted += 1; return true })
        await fake.setup(hold: true); let selected = try item(title: String(repeating: "序列", count: 200))
        let read = Task { await model.attach(selected) }; while !(await fake.waiting()) { await Task.yield() }
        let duplicate = await model.attach(selected); XCTAssertFalse(duplicate)
        await fake.release(); let added = await read.value; XCTAssertTrue(added); XCTAssertEqual(accepted, 1)
        let calls = await fake.recorded(); XCTAssertEqual(calls.count, 1)
        let query = try XCTUnwrap(calls[0].1["query"]?.string); XCTAssertLessThanOrEqual(query.utf8.count, 512); XCTAssertTrue(selected.title.hasPrefix(query))
    }
    @MainActor func testNativeFieldRoutesEnterAndShiftEnterAndLeavesIMECandidatesAlone() throws {
        _ = NSApplication.shared; var submissions: [NativeSearchDisposition] = []
        let host = NSHostingView(rootView: SearchCommandField(text: .constant("query"), cancel: {}, move: { _ in }, submit: { submissions.append($0) }))
        host.frame = NSRect(x: 0, y: 0, width: 400, height: 44); host.layoutSubtreeIfNeeded()
        func field(in view: NSView) -> NSTextField? { (view as? NSTextField) ?? view.subviews.compactMap { field(in: $0) }.first }
        let textField = try XCTUnwrap(field(in: host)), coordinator = try XCTUnwrap(textField.delegate as? SearchCommandField.Coordinator)
        let editor = NSTextView()
        XCTAssertTrue(coordinator.control(textField, textView: editor, doCommandBy: #selector(NSResponder.insertNewline(_:))))
        XCTAssertTrue(coordinator.control(textField, textView: editor, doCommandBy: #selector(NSResponder.insertLineBreak(_:))))
        XCTAssertEqual(submissions, [.open, .reference])
        XCTAssertEqual(NativeSearchDisposition.resolve(.command), .newWindow)
        XCTAssertEqual(NativeSearchDisposition.resolve(.control), .newWindow)
        XCTAssertEqual(NativeSearchDisposition.resolve([.command, .shift]), .reference)
        editor.setMarkedText("候选", selectedRange: NSRange(location: 0, length: 2), replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertTrue(editor.hasMarkedText())
        XCTAssertFalse(coordinator.control(textField, textView: editor, doCommandBy: #selector(NSResponder.insertNewline(_:))))
        XCTAssertEqual(submissions, [.open, .reference])
    }
}
