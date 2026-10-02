import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor FeedbackClient: NativeConversationQuerying {
    var writes = 0
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot {
        let value: SettingsValue = .object([
            "schema": .string(ConversationSnapshot.schemaID), "epoch": .string("fake"), "sequence": .integer(1),
            "project_id": .string(projectID), "session_id": .string(sessionID), "items": .array([]),
            "running": .bool(false), "stopping": .bool(false), "read_only": .bool(false), "model_id": .string("offline"),
            "approvals": .array([.object(["approval_id": .string("a"), "frame_id": .string(sessionID), "message": .string("Please review"), "tool": .string("update_plan"), "preview": .string("")])])
        ])
        return try ConversationSnapshot.decode(value, projectID: projectID, sessionID: sessionID)
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        if command == "native_conversation_approve" { writes += 1; throw ProjectBrowserError.service("response lost") }
        return .null
    }
    func count() -> Int { writes }
}
final class NativeApprovalFeedbackTests: XCTestCase {
    @MainActor func testFailedFeedbackDoesNotRetryOrClearComposer() async throws {
        let client = FeedbackClient(); let model = NativeConversationModel(client: client)
        await model.open(project: "p", session: "s"); defer { model.pause() }
        model.draft = "unsent notes"
        let approval = try XCTUnwrap(model.snapshot?.approvals.first)
        let saved = await model.approve(approval, allowed: false, feedback: "Change the plan")
        XCTAssertFalse(saved); XCTAssertNotNil(model.operationError)
        await model.refresh()
        let writes = await client.count(); XCTAssertEqual(writes, 1)
        XCTAssertEqual(model.draft, "unsent notes")
        await model.open(project: "p", session: "other")
        let old = await model.approve(approval, allowed: false, feedback: "Late feedback")
        XCTAssertFalse(old)
        let after = await client.count(); XCTAssertEqual(after, 1)
    }
    @MainActor func testImmediateEscapeOnlyDismissesFeedbackBeforeParent() async throws {
        let client = FeedbackClient(); let model = NativeConversationModel(client: client)
        await model.open(project: "p", session: "s"); defer { model.pause() }
        let approval = try XCTUnwrap(model.snapshot?.approvals.first)
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 520, height: 360), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let parentView = NSView(frame: window.contentView!.bounds); window.contentView = parentView
        var parentClosed = 0, feedbackClosed = 0
        let parent = NativeSettingsEscape.Coordinator(enabled: true) { parentClosed += 1 }
        parent.view = parentView; parent.install(); defer { parent.remove() }
        let hosted = NSHostingView(rootView: NativeApprovalFeedback(approval: approval, conversation: model) { feedbackClosed += 1 })
        hosted.frame = parentView.bounds; parentView.addSubview(hosted); hosted.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let escape = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(escape, keyWindow: window, modalWindow: nil))
        XCTAssertEqual(feedbackClosed, 1); XCTAssertEqual(parentClosed, 0)
        XCTAssertTrue(window.firstResponder === focus)
        hosted.removeFromSuperview()
        XCTAssertTrue(NativeEscapeStack.shared.consume(escape, keyWindow: window, modalWindow: nil))
        XCTAssertEqual(parentClosed, 1)
        let writes = await client.count(); XCTAssertEqual(writes, 0)
    }
}
