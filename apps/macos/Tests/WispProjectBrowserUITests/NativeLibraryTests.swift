import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

final class NativeLibraryTests: XCTestCase {
    @MainActor func testSearchSendsTheFilterWithoutAProjectIdAndDoesNotRetryALostRead() async throws {
        let host = LibraryTransport()
        let library = NativeLibraryModel()
        library.presented = true
        library.kind = "code"
        library.query = "RNA"
        await library.reload(host)
        let first = await host.callCount()
        let call = await host.lastCall()
        let items = library.items
        XCTAssertEqual(first, 1)
        XCTAssertEqual(call.command, NativeLibraryCommand.search)
        XCTAssertNil(call.projectID)
        XCTAssertEqual(call.args["query"], .string("RNA"))
        XCTAssertEqual(call.args["kind"], .string("code"))
        XCTAssertEqual(items.map(\.id), ["item-1"])
        XCTAssertEqual(items[0].sourceProjectID, "research-1")
        await host.setMode("lost")
        await library.reload(host)
        let second = await host.callCount()
        XCTAssertEqual(second, 2)
        XCTAssertEqual(library.items.map(\.id), ["item-1"])
        XCTAssertTrue(library.error?.contains("不会自动重试") == true)
        await Task.yield()
        let after = await host.callCount()
        XCTAssertEqual(after, 2)
    }

    @MainActor func testDeleteRemovesOnlyAConfirmedRowAndALostReplyIsNotRetried() async throws {
        let host = LibraryTransport()
        let library = NativeLibraryModel()
        library.presented = true
        await library.reload(host)
        await host.setMode("lost")
        await library.delete(host, id: "item-1")
        let lost = await host.callCount()
        let deleteCall = await host.lastCall()
        XCTAssertEqual(lost, 2)
        XCTAssertEqual(deleteCall.command, NativeLibraryCommand.delete)
        XCTAssertNil(deleteCall.projectID)
        XCTAssertEqual(deleteCall.args["id"], .string("item-1"))
        XCTAssertEqual(library.items.map(\.id), ["item-1"])
        XCTAssertTrue(library.error?.contains("不会自动重试") == true)
        await Task.yield()
        let still = await host.callCount()
        XCTAssertEqual(still, 2)

        await host.setMode("missing")
        await library.delete(host, id: "item-1")
        let missing = await host.callCount()
        XCTAssertEqual(missing, 3)
        XCTAssertEqual(library.items.map(\.id), ["item-1"])

        await host.setMode("ok")
        await host.suspend()
        let first = Task { await library.delete(host, id: "item-1") }
        while !(await host.isHanging()) { await Task.yield() }
        await library.delete(host, id: "item-1")
        let during = await host.callCount()
        XCTAssertEqual(during, 4)
        await host.resume(with: .bool(true))
        await first.value
        let done = await host.callCount()
        XCTAssertEqual(done, 4)
        XCTAssertTrue(library.items.isEmpty)
    }

    @MainActor func testInsertPrefillsAnOpenSessionAndHomeDoesNotOfferIt() async throws {
        let client = LibraryConversation()
        let conversation = NativeConversationModel(client: client)
        await client.configure([try librarySnapshot()])
        await conversation.open(project: "project-a", session: "session-a")
        let item = try NativeLibraryModel.decode(LibraryTransport.fixtureItems())[0]
        let text = NativeLibraryModel.composerText(item)
        XCTAssertTrue(text.contains(item.title))
        XCTAssertTrue(text.contains(item.codePreview))
        XCTAssertFalse(NativeLibraryModel.offersInsert(activeSessionID: nil))
        XCTAssertFalse(NativeLibraryModel.offersInsert(activeSessionID: ""))
        XCTAssertFalse(NativeLibraryModel.insertIntoConversation(item, conversation: conversation, activeSessionID: nil))
        XCTAssertEqual(conversation.draft, "")
        XCTAssertTrue(NativeLibraryModel.offersInsert(activeSessionID: "session-a"))
        XCTAssertTrue(NativeLibraryModel.insertIntoConversation(item, conversation: conversation, activeSessionID: "session-a"))
        XCTAssertEqual(conversation.draft, text)
        conversation.draft = "已有"
        XCTAssertTrue(NativeLibraryModel.insertIntoConversation(item, conversation: conversation, activeSessionID: "session-a"))
        XCTAssertEqual(conversation.draft, "已有\n" + text)
        let commands = await client.commands()
        XCTAssertFalse(commands.contains("native_conversation_send"))
        let figure = LibraryEntry(id: "fig", kind: "figure", title: "Plot", language: nil, codePreview: "  ", sourceProjectID: "research-1", sourceProjectName: "RNA", sourceSessionID: "session-b", sourceSessionTitle: "作图", sourcePath: "figures/plot.png", createdAt: 1)
        XCTAssertEqual(NativeLibraryModel.composerText(figure), "请看收藏「Plot」。")
        conversation.pause()
    }

    @MainActor func testReturnToSourceUsesTheProjectListAndDoesNotRetargetTheWebView() async {
        let list = LibraryProjectList()
        let host = LibraryTransport()
        let model = ProjectBrowserModel(client: list, databaseURL: URL(fileURLWithPath: "/unused/library.sqlite"), projectTransport: host)
        let item = LibraryEntry(id: "item-1", kind: "code", title: "plot RNA", language: "python", codePreview: "import pandas", sourceProjectID: "research-1", sourceProjectName: "RNA-seq 研究", sourceSessionID: "session-a", sourceSessionTitle: "探索", sourcePath: nil, createdAt: 1)
        model.library.presented = true
        model.library.query = "kept"
        await model.openLibrarySource(item)
        let hostCalls = await host.callCount()
        let asked = await list.sessionProjects()
        XCTAssertEqual(hostCalls, 0)
        XCTAssertEqual(asked, ["research-1"])
        XCTAssertEqual(model.activeProjectID, "research-1")
        XCTAssertEqual(model.activeSessionID, "session-a")
        XCTAssertFalse(model.library.presented)
        XCTAssertEqual(model.library.query, "kept")
    }

    @MainActor func testLateSearchDoesNotReopenAProjectOrFillAClosedSheet() async throws {
        let list = LibraryProjectList()
        let host = LibraryTransport()
        let model = ProjectBrowserModel(client: list, databaseURL: URL(fileURLWithPath: "/unused/library.sqlite"), projectTransport: host)
        await model.openProject("research-1", sessionID: "session-a")
        let opened = model.activeProjectID
        XCTAssertEqual(opened, "research-1")
        model.library.presented = true
        await host.suspend()
        let task = Task { await model.library.reload(host) }
        while !(await host.isHanging()) { await Task.yield() }
        model.goHome()
        model.library.presented = false
        await host.resume(with: try LibraryTransport.fixtureItems())
        await task.value
        let project = model.activeProjectID
        let items = model.library.items
        XCTAssertNil(project)
        XCTAssertTrue(items.isEmpty)
        let calls = await host.callCount()
        XCTAssertEqual(calls, 1)
    }

    @MainActor func testImmediateEscapeClosesOnlyTheLibrarySheet() {
        _ = NSApplication.shared
        let library = NativeLibraryModel()
        library.presented = true
        library.query = "kept"
        var searchPresented = true
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 420, height: 280), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let root = NSView(frame: window.contentLayoutRect)
        window.contentView = root
        let search = NSView(frame: .zero)
        root.addSubview(search)
        let searchOwner = NativeSettingsEscape.Coordinator(enabled: true) { searchPresented = false }
        searchOwner.view = search
        searchOwner.install()
        defer { searchOwner.remove() }
        let host = NSHostingView(rootView: LibraryEscapeSheet(library: library))
        root.addSubview(host)
        host.frame = root.bounds
        host.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertFalse(library.presented)
        XCTAssertTrue(searchPresented)
        XCTAssertEqual(library.query, "kept")
        XCTAssertTrue(window.firstResponder === focus)
    }
}

private struct LibraryEscapeSheet: View {
    @ObservedObject var library: NativeLibraryModel
    var body: some View {
        Text("收藏库")
            .background(NativeSettingsEscape(enabled: library.deleting.isEmpty) { library.dismiss() })
    }
}

private actor LibraryTransport: NativeSettingsQuerying {
    var calls: [(command: String, args: [String: SettingsValue], projectID: String?)] = []
    var mode = "ok"
    private var release: CheckedContinuation<SettingsValue, Error>?
    func setMode(_ mode: String) { self.mode = mode }
    func callCount() -> Int { calls.count }
    func lastCall() -> (command: String, args: [String: SettingsValue], projectID: String?) { calls.last! }
    func suspend() { mode = "hang" }
    func isHanging() -> Bool { release != nil }
    func resume(with value: SettingsValue) { release?.resume(returning: value); release = nil }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        calls.append((command, args, projectID))
        if mode == "lost" { throw ProjectBrowserError.service("connection reset") }
        if mode == "hang" { return try await withCheckedThrowingContinuation { release = $0 } }
        if command == NativeLibraryCommand.delete { return .bool(mode != "missing") }
        if command == NativeLibraryCommand.search { return try Self.fixtureItems() }
        throw ProjectBrowserError.service("unexpected \(command)")
    }
    static func fixtureItems() throws -> SettingsValue {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let fixture = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-library/v1/search.json")))
        return fixture["result"]
    }
}

private actor LibraryProjectList: ProjectBrowserQuerying {
    var projects: [String?] = []
    func sessionProjects() -> [String?] { projects }
    func listProjects(databaseURL: URL) async throws -> ProjectListSnapshot {
        ProjectListSnapshot(projects: [], activitySource: "persisted_only")
    }
    func listSessions(databaseURL: URL, projectID: String?) async throws -> [BrowserSession] {
        projects.append(projectID)
        return [BrowserSession(id: "session-a", projectID: projectID ?? "", title: "探索", ts: 1, status: "complete")]
    }
    func transcript(databaseURL: URL, projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> TranscriptPage {
        TranscriptPage(messages: [], nextBeforeSeq: nil)
    }
}

private func librarySnapshot() throws -> ConversationSnapshot {
    var url = URL(fileURLWithPath: #filePath)
    for _ in 0..<5 { url.deleteLastPathComponent() }
    var value = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: url.appendingPathComponent("contracts/native-conversations/v1/snapshot.json")))
    value["session_id"] = .string("session-a")
    value["sequence"] = .integer(7)
    value["epoch"] = .string("host-one")
    value["running"] = .bool(false)
    value["request_id"] = .null
    value["error"] = .null
    value["approvals"] = .array([])
    return try ConversationSnapshot.decode(value, projectID: "project-a", sessionID: "session-a")
}

private actor LibraryConversation: NativeConversationQuerying {
    var reads: [ConversationSnapshot] = []
    var writes: [String] = []
    func configure(_ values: [ConversationSnapshot]) { reads = values }
    func commands() -> [String] { writes }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot {
        if let first = reads.first { return first }
        return try librarySnapshot()
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        writes.append(command)
        if command == "list_models" { return .array([]) }
        return .null
    }
}
