import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor SyncTransport: NativeSettingsQuerying {
    var calls: [(String, [String: SettingsValue], String?)] = []
    var mode = "published"
    private var pending: CheckedContinuation<SettingsValue, Error>?
    func setMode(_ value: String) { mode = value }
    func count() -> Int { calls.count }
    func last() -> (String, [String: SettingsValue], String?) { calls.last! }
    func isPending() -> Bool { pending != nil }
    func finish() { pending?.resume(returning: .object(["status": .string("published")])); pending = nil }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        calls.append((command, args, projectID))
        if mode == "hang" { return try await withCheckedThrowingContinuation { pending = $0 } }
        if mode == "lost" { throw ProjectBrowserError.service("connection lost") }
        if mode == "conflict" { throw ProjectBrowserError.service("Sync conflict: both copies changed") }
        return .object(["status": .string(mode), "direction": .string("none"), "revision": .string("r1"), "uploaded_files": .integer(0), "downloaded_files": .integer(0), "skipped_paths": .array([])])
    }
}

final class NativeProjectSyncTests: XCTestCase {
    private func project(_ status: String? = nil) throws -> ProjectSummary {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let fixture = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-projects/v1/import.json")))
        var value = fixture["result"]
        value["sync_configured"] = .bool(false)
        value["folder_sync"] = status.map(SettingsValue.string) ?? .null
        return try NativeProjectCommand.summary(from: value)
    }

    @MainActor func testEnablingAndManualSyncUseExplicitProjectAndNoImplicitChoice() async throws {
        let client = SyncTransport()
        let model = NativeProjectSyncModel(project: try project(), client: client)
        await model.synchronize()
        let untouched = await client.count(); XCTAssertEqual(untouched, 0)
        await model.enableFolderSync()
        let enabled = await client.last()
        XCTAssertEqual(enabled.0, "enable_project_folder_sync")
        XCTAssertEqual(enabled.1, ["id": .string("research-1")])
        XCTAssertEqual(enabled.2, "research-1")
        XCTAssertTrue(model.configured)
        await model.enableFolderSync()
        let once = await client.count(); XCTAssertEqual(once, 1)
        await model.synchronize()
        let sync = await client.last(); XCTAssertEqual(sync.0, "sync_project")
        XCTAssertNil(sync.1["strategy"])
        XCTAssertEqual(sync.2, "research-1")
    }

    @MainActor func testConflictNeverPicksAVersionUntilConfirmedAndSendsChosenStrategy() async throws {
        for choice in [NativeProjectSyncModel.ConflictChoice.local, .remote] {
            let client = SyncTransport()
            let model = NativeProjectSyncModel(project: try project("conflict"), client: client)
            await model.synchronize(); await model.resolve()
            model.choose(choice)
            let before = await client.count(); XCTAssertEqual(before, 0)
            XCTAssertEqual(model.confirmation, choice)
            await model.resolve()
            let call = await client.last()
            XCTAssertEqual(call.0, "resolve_project_sync")
            XCTAssertEqual(call.1, ["id": .string("research-1"), "strategy": .string(choice.rawValue)])
            XCTAssertEqual(call.2, "research-1")
            XCTAssertNil(model.confirmation); XCTAssertFalse(model.conflict)
            await model.resolve()
            let after = await client.count(); XCTAssertEqual(after, 1)
        }
    }

    @MainActor func testFreshConflictAndLostReplyPreserveAnExplicitDecision() async throws {
        let client = SyncTransport(); await client.setMode("conflict")
        let model = NativeProjectSyncModel(project: try project("saved"), client: client)
        await model.synchronize()
        XCTAssertTrue(model.conflict); XCTAssertNil(model.confirmation)
        model.choose(.remote)
        await client.setMode("lost")
        await model.resolve()
        XCTAssertEqual(model.confirmation, .remote)
        XCTAssertTrue(model.conflict)
        XCTAssertTrue(model.error?.contains("不会自动重试") == true)
        XCTAssertNil(model.result)
        await Task.yield()
        let calls = await client.count(); XCTAssertEqual(calls, 2)
    }

    @MainActor func testUnknownStatusDoesNotClaimEnableSucceeded() async throws {
        let client = SyncTransport(); await client.setMode("new-unknown-state")
        let model = NativeProjectSyncModel(project: try project(), client: client)
        await model.enableFolderSync()
        XCTAssertFalse(model.configured); XCTAssertNil(model.result)
        XCTAssertNotNil(model.error)
    }

    @MainActor func testBusySyncRejectsDuplicateWrites() async throws {
        let client = SyncTransport(); await client.setMode("hang")
        let model = NativeProjectSyncModel(project: try project(), client: client)
        let first = Task { await model.enableFolderSync() }
        for _ in 0..<1000 { if await client.isPending() { break }; await Task.yield() }
        XCTAssertTrue(model.busy)
        await model.enableFolderSync()
        let calls = await client.count(); XCTAssertEqual(calls, 1)
        await client.finish(); await first.value
        XCTAssertTrue(model.configured)
    }

    @MainActor func testImmediateEscapeClosesConflictConfirmationAndKeepsSyncOpen() async throws {
        let client = SyncTransport()
        let model = NativeProjectSyncModel(project: try project("conflict"), client: client)
        model.choose(.remote)
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 510, height: 360), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let parentView = NSView(frame: window.contentView!.bounds); window.contentView = parentView
        var closed = false
        let parent = NativeSettingsEscape.Coordinator(enabled: true) { closed = true }
        parent.view = parentView; parent.install(); defer { parent.remove() }
        let hosted = NSHostingView(rootView: NativeProjectSyncConflictSheet(model: model, choice: .remote))
        hosted.frame = parentView.bounds; parentView.addSubview(hosted); hosted.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let escape = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(escape, keyWindow: window, modalWindow: nil))
        XCTAssertNil(model.confirmation); XCTAssertFalse(closed); XCTAssertTrue(model.conflict)
        XCTAssertTrue(window.firstResponder === focus)
        let calls = await client.count(); XCTAssertEqual(calls, 0)
    }
}
