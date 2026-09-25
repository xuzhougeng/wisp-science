import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor RenameClient: NativeConversationQuerying {
    var calls: [(String, [String: SettingsValue], String)] = []
    var fails = false
    var suspend = false
    var continuation: CheckedContinuation<SettingsValue, Error>?
    func configure(fails: Bool = false, suspend: Bool = false) { self.fails = fails; self.suspend = suspend }
    func waiting() -> Bool { continuation != nil }
    func writes() -> [(String, [String: SettingsValue], String)] { calls }
    func finish() { continuation?.resume(returning: .null); continuation = nil }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { throw ProjectBrowserError.invalidResponse }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        calls.append((command, args, projectID))
        if fails { throw ProjectBrowserError.service("response lost") }
        if suspend { return try await withCheckedThrowingContinuation { continuation = $0 } }
        return .null
    }
}

final class NativeSessionRenameTests: XCTestCase {
    private func session(_ id: String = "s", project: String = "p") -> BrowserSession {
        BrowserSession(id: id, projectID: project, title: "Old title", ts: 42, status: "complete", folderID: "folder")
    }
    @MainActor func testSaveUsesExactProjectAndSessionAndReturnsConfirmedTitle() async throws {
        let client = RenameClient(); let model = NativeSessionRename()
        model.begin(session()); model.draft = "  样本分析  \n"
        let result = await model.save(client)
        XCTAssertEqual(result?.title, "样本分析"); XCTAssertEqual(result?.folderID, "folder")
        XCTAssertNil(model.target); XCTAssertFalse(model.busy)
        let writes = await client.writes()
        XCTAssertEqual(writes.count, 1); XCTAssertEqual(writes[0].0, "native_conversation_rename")
        XCTAssertEqual(writes[0].1, ["session_id": .string("s"), "title": .string("样本分析")])
        XCTAssertEqual(writes[0].2, "p")
    }
    @MainActor func testUnknownSaveResultKeepsInputAndDoesNotRetry() async {
        let client = RenameClient(); await client.configure(fails: true)
        let model = NativeSessionRename(); model.begin(session()); model.draft = "keep this title"
        let result = await model.save(client)
        XCTAssertNil(result); XCTAssertEqual(model.target?.id, "s"); XCTAssertEqual(model.draft, "keep this title")
        XCTAssertNotNil(model.error); XCTAssertFalse(model.busy)
        model.dismiss()
        let writes = await client.writes(); XCTAssertEqual(writes.count, 1)
    }
    @MainActor func testEmptyTitleAndCancelledEditorNeverWrite() async {
        let client = RenameClient(); let model = NativeSessionRename()
        model.begin(session()); model.draft = " \n"
        let result = await model.save(client); XCTAssertNil(result); XCTAssertNotNil(model.error)
        model.dismiss(); let after = await model.save(client); XCTAssertNil(after)
        let writes = await client.writes(); XCTAssertTrue(writes.isEmpty)
    }
    @MainActor func testDuplicateSaveAndLateReplyCannotCloseANewEditor() async {
        let client = RenameClient(); await client.configure(suspend: true)
        let model = NativeSessionRename(); model.begin(session()); model.draft = "first"
        let saving = Task { await model.save(client) }
        while !(await client.waiting()) { await Task.yield() }
        let duplicate = await model.save(client); XCTAssertNil(duplicate)
        model.dismiss(); XCTAssertNotNil(model.target)
        model.reset(); model.begin(session("new", project: "other")); model.draft = "new notes"
        await client.finish(); let stale = await saving.value
        XCTAssertNil(stale); XCTAssertEqual(model.target?.projectID, "other"); XCTAssertEqual(model.target?.id, "new")
        XCTAssertEqual(model.draft, "new notes"); XCTAssertNil(model.error); XCTAssertFalse(model.busy)
        let writes = await client.writes(); XCTAssertEqual(writes.count, 1)
    }
    @MainActor func testImmediateEscapeClosesRenameBeforeParentWithoutWriting() {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 520, height: 320), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let parentView = NSView(frame: window.contentView!.bounds); window.contentView = parentView
        var parentClosed = 0, writes = 0
        let parent = NativeSettingsEscape.Coordinator(enabled: true) { parentClosed += 1 }
        parent.view = parentView; parent.install(); defer { parent.remove() }
        let model = NativeSessionRename(); model.begin(session())
        let hosted = NSHostingView(rootView: NativeSessionRenameSheet(model: model) { writes += 1 })
        hosted.frame = parentView.bounds; parentView.addSubview(hosted); hosted.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let escape = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(escape, keyWindow: window, modalWindow: nil))
        XCTAssertNil(model.target); XCTAssertEqual(parentClosed, 0); XCTAssertEqual(writes, 0)
        XCTAssertTrue(window.firstResponder === focus)
        hosted.removeFromSuperview()
        XCTAssertTrue(NativeEscapeStack.shared.consume(escape, keyWindow: window, modalWindow: nil))
        XCTAssertEqual(parentClosed, 1)
    }
}
