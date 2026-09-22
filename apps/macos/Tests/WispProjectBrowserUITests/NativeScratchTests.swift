import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

final class NativeScratchTests: XCTestCase {
    @MainActor func testOpenCreatesTheHiddenChatWithoutChangingTheActiveProjectOrCallingTheWebView() async throws {
        let host = ScratchTransport()
        let list = ScratchProjectList()
        let model = ProjectBrowserModel(client: list, databaseURL: URL(fileURLWithPath: "/unused/scratch.sqlite"), projectTransport: host)
        await model.openProject("research-1", sessionID: "session-a")
        let before = model.activeProjectID
        await model.scratch.open(host)
        let calls = await host.calls()
        XCTAssertEqual(calls.count, 1)
        XCTAssertEqual(calls[0].command, NativeScratchCommand.open)
        XCTAssertNil(calls[0].projectID)
        XCTAssertFalse(calls.contains { $0.command == NativeScratchCommand.webViewStart })
        XCTAssertEqual(model.scratch.projectID, "scratch:11111111-1111-1111-1111-111111111111")
        XCTAssertEqual(model.scratch.sessionID, "session-scratch")
        XCTAssertTrue(model.scratch.presented)
        XCTAssertEqual(model.activeProjectID, before)
        let lostHost = ScratchTransport()
        let lost = NativeScratchModel()
        await lostHost.setMode("lost")
        await lost.open(lostHost)
        let lostCalls = await lostHost.callCount()
        XCTAssertEqual(lostCalls, 1)
        XCTAssertFalse(lost.presented)
        XCTAssertNil(lost.projectID)
        XCTAssertTrue(lost.error?.contains("不会自动重试") == true)
        await Task.yield()
        let still = await lostHost.callCount()
        XCTAssertEqual(still, 1)
        let lostCommands = await lostHost.calls()
        XCTAssertFalse(lostCommands.contains { $0.command == NativeScratchCommand.webViewStart })
    }

    @MainActor func testCloseDeletesOnlyTheScratchProjectAndALostReplyIsNotRetried() async {
        let host = ScratchTransport()
        let list = ScratchProjectList()
        let model = ProjectBrowserModel(client: list, databaseURL: URL(fileURLWithPath: "/unused/scratch.sqlite"), projectTransport: host)
        await model.openProject("research-1")
        await model.scratch.open(host)
        await host.setMode("lost")
        await model.scratch.close(host)
        let lost = await host.calls()
        XCTAssertEqual(lost.last?.command, NativeScratchCommand.close)
        XCTAssertEqual(lost.last?.projectID, "scratch:11111111-1111-1111-1111-111111111111")
        XCTAssertFalse(lost.contains { $0.command == NativeScratchCommand.webViewStart })
        XCTAssertTrue(model.scratch.presented)
        XCTAssertEqual(model.activeProjectID, "research-1")
        XCTAssertTrue(model.scratch.error?.contains("不会自动重试") == true)
        let during = lost.count
        await Task.yield()
        let still = await host.callCount()
        XCTAssertEqual(still, during)
        await host.setMode("ok")
        await model.scratch.close(host)
        let done = await host.callCount()
        XCTAssertEqual(done, during + 1)
        XCTAssertFalse(model.scratch.presented)
        XCTAssertNil(model.scratch.projectID)
        XCTAssertEqual(model.activeProjectID, "research-1")
    }

    @MainActor func testImmediateEscapeClosesOnlyTheScratchChat() async {
        _ = NSApplication.shared
        let host = ScratchTransport()
        let scratch = NativeScratchModel()
        await scratch.open(host)
        var parent = true
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 420, height: 280), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let root = NSView(frame: window.contentLayoutRect)
        window.contentView = root
        let parentView = NSView(frame: .zero)
        root.addSubview(parentView)
        let owner = NativeSettingsEscape.Coordinator(enabled: true) { parent = false }
        owner.view = parentView
        owner.install()
        defer { owner.remove() }
        let view = NSHostingView(rootView: ScratchEscapeSheet(scratch: scratch, client: host))
        root.addSubview(view)
        view.frame = root.bounds
        view.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        var closed = false
        for _ in 0..<50 {
            if !scratch.presented { closed = true; break }
            await Task.yield()
        }
        XCTAssertTrue(closed)
        XCTAssertTrue(parent)
        XCTAssertTrue(window.firstResponder === focus)
        let calls = await host.calls()
        XCTAssertEqual(calls.last?.command, NativeScratchCommand.close)
        XCTAssertFalse(calls.contains { $0.command == NativeScratchCommand.webViewStart })
    }
}

private struct ScratchEscapeSheet: View {
    @ObservedObject var scratch: NativeScratchModel
    let client: ScratchTransport
    var body: some View {
        Text("随手一聊")
            .background(NativeSettingsEscape(enabled: !scratch.busy) { Task { await scratch.close(client) } })
    }
}

private actor ScratchTransport: NativeSettingsQuerying {
    private var recorded: [(command: String, args: [String: SettingsValue], projectID: String?)] = []
    private var mode = "ok"
    func setMode(_ mode: String) { self.mode = mode }
    func callCount() -> Int { recorded.count }
    func calls() -> [(command: String, args: [String: SettingsValue], projectID: String?)] { recorded }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        recorded.append((command, args, projectID))
        if mode == "lost" { throw ProjectBrowserError.service("connection reset") }
        if command == NativeScratchCommand.open { return try Self.openResult() }
        if command == NativeScratchCommand.close { return .bool(true) }
        throw ProjectBrowserError.service("unexpected \(command)")
    }
    static func openResult() throws -> SettingsValue {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let fixture = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-scratch/v1/open.json")))
        return fixture["result"]
    }
}

private actor ScratchProjectList: ProjectBrowserQuerying {
    func listProjects(databaseURL: URL) async throws -> ProjectListSnapshot {
        ProjectListSnapshot(projects: [], activitySource: "persisted_only")
    }
    func listSessions(databaseURL: URL, projectID: String?) async throws -> [BrowserSession] {
        [BrowserSession(id: "session-a", projectID: projectID ?? "", title: "探索", ts: 1, status: "complete")]
    }
    func transcript(databaseURL: URL, projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> TranscriptPage {
        TranscriptPage(messages: [], nextBeforeSeq: nil)
    }
}
