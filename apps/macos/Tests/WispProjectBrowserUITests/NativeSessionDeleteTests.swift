import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor DeleteClient: NativeConversationQuerying {
    var calls: [(String, [String: SettingsValue], String)] = []
    var failID: String?
    var suspended = false
    var continuation: CheckedContinuation<SettingsValue, Error>?
    func configure(failID: String? = nil, suspended: Bool = false) { self.failID = failID; self.suspended = suspended }
    func writes() -> [(String, [String: SettingsValue], String)] { calls }
    func waiting() -> Bool { continuation != nil }
    func finish() { continuation?.resume(returning: .null); continuation = nil }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { throw ProjectBrowserError.invalidResponse }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        calls.append((command, args, projectID))
        if args["session_id"]?.string == failID { throw ProjectBrowserError.service("response lost") }
        if suspended { return try await withCheckedThrowingContinuation { continuation = $0 } }
        return .null
    }
}

final class NativeSessionDeleteTests: XCTestCase {
    private func session(_ id: String, project: String = "p") -> BrowserSession {
        BrowserSession(id: id, projectID: project, title: "Conversation " + id, ts: 1, status: "complete")
    }
    @MainActor func testPreviewIsScopedAndDoesNotWriteBeforeConfirmation() async {
        let client = DeleteClient(); let model = NativeSessionDelete()
        model.begin([session("a"), session("b", project: "other")])
        XCTAssertTrue(model.targets.isEmpty); XCTAssertFalse(model.canDelete)
        model.begin([session("a"), session("a"), session("b")])
        XCTAssertEqual(model.targets.map(\.id), ["a", "b"])
        let before = await client.writes(); XCTAssertTrue(before.isEmpty)
        model.dismiss()
        let cancelled = await model.delete(client)
        XCTAssertTrue(cancelled.confirmed.isEmpty)
        let after = await client.writes(); XCTAssertTrue(after.isEmpty)
    }
    @MainActor func testConfirmedBatchDeletesEachExactSessionOnce() async {
        let client = DeleteClient(); let model = NativeSessionDelete()
        model.begin([session("a"), session("b")])
        let result = await model.delete(client)
        XCTAssertEqual(result.confirmed, ["a", "b"]); XCTAssertNil(result.error); XCTAssertTrue(result.isCurrent)
        XCTAssertTrue(model.targets.isEmpty); XCTAssertFalse(model.busy)
        let calls = await client.writes()
        XCTAssertEqual(calls.map(\.0), ["native_conversation_delete", "native_conversation_delete"])
        XCTAssertEqual(calls.map(\.2), ["p", "p"])
        XCTAssertEqual(calls.map(\.1), [["session_id": .string("a")], ["session_id": .string("b")]])
    }
    @MainActor func testPartialFailureStopsWithoutRetryingOrClaimingUnknownDeletion() async {
        let client = DeleteClient(); await client.configure(failID: "b")
        let model = NativeSessionDelete(); model.begin([session("a"), session("b"), session("c")])
        let result = await model.delete(client)
        XCTAssertEqual(result.confirmed, ["a"]); XCTAssertNotNil(result.error)
        XCTAssertEqual(result.unconfirmed?.id, "b")
        XCTAssertEqual(model.deletedIDs, ["a"]); XCTAssertFalse(model.canDelete)
        let retry = await model.delete(client); XCTAssertFalse(retry.isCurrent)
        let calls = await client.writes(); XCTAssertEqual(calls.map { $0.1["session_id"] }, [.string("a"), .string("b")])
    }
    @MainActor func testNavigationStopsRemainingWritesAndDoesNotDismissNewPreview() async {
        let client = DeleteClient(); await client.configure(suspended: true)
        let model = NativeSessionDelete(); model.begin([session("a"), session("b")])
        let task = Task { await model.delete(client) }
        while !(await client.waiting()) { await Task.yield() }
        let duplicate = await model.delete(client); XCTAssertFalse(duplicate.isCurrent)
        model.dismiss(); XCTAssertFalse(model.targets.isEmpty)
        model.reset(); model.begin([session("new", project: "other")])
        await client.finish(); let result = await task.value
        XCTAssertEqual(result.confirmed, ["a"]); XCTAssertFalse(result.isCurrent)
        XCTAssertEqual(model.targets.map(\.id), ["new"]); XCTAssertNil(model.error); XCTAssertFalse(model.busy)
        let calls = await client.writes(); XCTAssertEqual(calls.count, 1)
    }
    @MainActor func testImmediateEscapeClosesConfirmationBeforeParentWithoutDeleting() {
        _ = NSApplication.shared
        let model = NativeSessionDelete(); model.begin([session("a")])
        var parentOpen = true; var confirmed = false
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 480, height: 360), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let root = NSView(frame: window.contentLayoutRect); window.contentView = root
        let parent = NSView(frame: .zero); root.addSubview(parent)
        let owner = NativeSettingsEscape.Coordinator(enabled: true) { parentOpen = false }
        owner.view = parent; owner.install(); defer { owner.remove() }
        let host = NSHostingView(rootView: NativeSessionDeleteSheet(model: model) { confirmed = true })
        root.addSubview(host); host.frame = root.bounds; host.layoutSubtreeIfNeeded()
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertTrue(model.targets.isEmpty); XCTAssertTrue(parentOpen); XCTAssertFalse(confirmed)
    }
}
