import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

final class NativePublicationTests: XCTestCase {
    @MainActor func testCreateRequiresAProjectAndKeepsTheDraftWhenTheReplyIsLost() async throws {
        let host = PublicationTransport()
        let publication = NativePublicationModel()
        publication.open(projectID: "research-1")
        publication.draft = PublicationDraft(title: "  ", description: "notes", revisionLabel: "v1")
        await publication.create(host)
        let idle = await host.callCount()
        XCTAssertEqual(idle, 0)
        XCTAssertEqual(publication.draft.revisionLabel, "v1")
        publication.draft.title = "RNA-seq paper"
        await host.setMode("lost")
        await publication.create(host)
        let call = await host.lastCall()
        XCTAssertEqual(call.command, NativePublicationCommand.create)
        XCTAssertEqual(call.projectID, "research-1")
        XCTAssertEqual(call.args["title"], .string("RNA-seq paper"))
        XCTAssertEqual(call.args["revision_label"], .string("v1"))
        XCTAssertEqual(publication.draft.title, "RNA-seq paper")
        XCTAssertTrue(publication.error?.contains("不会自动重试") == true)
        await Task.yield()
        let still = await host.callCount()
        XCTAssertEqual(still, 1)
        await host.setMode("ok")
        await publication.create(host)
        let page = publication.workspace
        XCTAssertEqual(page.publication?.title, "RNA-seq paper")
        XCTAssertEqual(page.publication?.projectID, "research-1")
        XCTAssertEqual(page.items.map(\.kind), ["claim"])
        XCTAssertEqual(publication.draft.title, "")
        let calls = await host.callCount()
        XCTAssertEqual(calls, 2)
    }

    @MainActor func testClosedWorkspaceDoesNotReadAndALateReplyDoesNotOpenAProject() async throws {
        let list = PublicationProjectList()
        let host = PublicationTransport()
        let model = ProjectBrowserModel(client: list, databaseURL: URL(fileURLWithPath: "/unused/publication.sqlite"), projectTransport: host)
        await model.publication.reload(host)
        let idle = await host.callCount()
        XCTAssertEqual(idle, 0)
        model.publication.open(projectID: "research-1")
        await host.suspend()
        let task = Task { await model.publication.reload(host) }
        while !(await host.isHanging()) { await Task.yield() }
        model.goHome()
        await host.resume(with: try PublicationTransport.fixturePage())
        await task.value
        let project = model.activeProjectID
        XCTAssertNil(project)
        XCTAssertFalse(model.publication.presented)
        XCTAssertTrue(model.publication.workspace.publications.isEmpty)
    }

    @MainActor func testImmediateEscapeClosesOnlyThePublicationWorkspace() {
        _ = NSApplication.shared
        let publication = NativePublicationModel()
        publication.open(projectID: "research-1")
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
        let host = NSHostingView(rootView: PublicationEscapeColumn(publication: publication))
        root.addSubview(host)
        host.frame = root.bounds
        host.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertFalse(publication.presented)
        XCTAssertTrue(parent)
        XCTAssertEqual(publication.projectID, "research-1")
        XCTAssertTrue(window.firstResponder === focus)
    }
}

private struct PublicationEscapeColumn: View {
    @ObservedObject var publication: NativePublicationModel
    var body: some View {
        Text("论文证据")
            .background(NativeSettingsEscape(enabled: !publication.busy) { publication.dismiss() })
    }
}

private actor PublicationTransport: NativeSettingsQuerying {
    private var recorded: [(command: String, args: [String: SettingsValue], projectID: String?)] = []
    private var mode = "ok"
    private var release: CheckedContinuation<SettingsValue, Error>?
    func setMode(_ mode: String) { self.mode = mode }
    func callCount() -> Int { recorded.count }
    func lastCall() -> (command: String, args: [String: SettingsValue], projectID: String?) { recorded.last! }
    func suspend() { mode = "hang" }
    func isHanging() -> Bool { release != nil }
    func resume(with value: SettingsValue) { release?.resume(returning: value); release = nil }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        recorded.append((command, args, projectID))
        if mode == "lost" { throw ProjectBrowserError.service("connection reset") }
        if mode == "hang" { return try await withCheckedThrowingContinuation { release = $0 } }
        return try Self.fixturePage()
    }
    static func fixturePage() throws -> SettingsValue {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let fixture = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-publication/v1/workspace.json")))
        return fixture["result"]
    }
}

private actor PublicationProjectList: ProjectBrowserQuerying {
    func listProjects(databaseURL: URL) async throws -> ProjectListSnapshot {
        ProjectListSnapshot(projects: [], activitySource: "persisted_only")
    }
    func listSessions(databaseURL: URL, projectID: String?) async throws -> [BrowserSession] { [] }
    func transcript(databaseURL: URL, projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> TranscriptPage {
        TranscriptPage(messages: [], nextBeforeSeq: nil)
    }
}
