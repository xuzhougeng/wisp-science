import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor NewProjectTransport: NativeSettingsQuerying {
    var calls: [(command: String, args: [String: SettingsValue], projectID: String?)] = []
    var mode = "success"
    private var release: CheckedContinuation<SettingsValue, Error>?

    func setMode(_ mode: String) { self.mode = mode }
    func callCount() -> Int { calls.count }
    func lastCall() -> (command: String, args: [String: SettingsValue], projectID: String?) { calls.last! }
    func suspend() { mode = "hang" }
    func isHanging() -> Bool { release != nil }
    func resume(with value: SettingsValue) {
        release?.resume(returning: value)
        release = nil
    }

    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        calls.append((command, args, projectID))
        switch mode {
        case "validation":
            throw ProjectBrowserError.service("Project name is required")
        case "registered":
            throw ProjectBrowserError.service("This folder is already registered as a project.")
        case "unwritable":
            throw ProjectBrowserError.service("Working directory is not writable: Permission denied")
        case "missing-dir":
            throw ProjectBrowserError.service("A working directory is required")
        case "lost":
            throw ProjectBrowserError.service("connection reset")
        case "hang":
            return try await withCheckedThrowingContinuation { release = $0 }
        default:
            return try Self.fixtureResult()
        }
    }

    static func fixtureResult() throws -> SettingsValue {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let fixture = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-projects/v1/create.json")))
        return fixture["result"]
    }
}

private actor EmptyProjectList: ProjectBrowserQuerying {
    var lists = 0
    func listCount() -> Int { lists }
    func listProjects(databaseURL: URL) async throws -> ProjectListSnapshot {
        lists += 1
        return ProjectListSnapshot(projects: [], activitySource: "persisted_only")
    }
    func listSessions(databaseURL: URL, projectID: String?) async throws -> [BrowserSession] { [] }
    func transcript(databaseURL: URL, projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> TranscriptPage {
        TranscriptPage(messages: [], nextBeforeSeq: nil)
    }
}

final class NewProjectTests: XCTestCase {
    func testLayoutBlockTogglesWithoutDuplicatingOrErasingOtherText() {
        let block = NewProjectLayout.chinese
        XCTAssertEqual(NewProjectLayout.apply("keep", enabled: true, block: block), "keep\n\n" + block)
        XCTAssertEqual(NewProjectLayout.apply("keep\n\n" + block, enabled: true, block: block), "keep\n\n" + block)
        XCTAssertEqual(NewProjectLayout.apply("keep\n\n" + block, enabled: false, block: block), "keep")
        XCTAssertEqual(NewProjectLayout.apply("", enabled: true, block: block), block)
        XCTAssertEqual(NewProjectLayout.apply(block, enabled: false, block: block), "")
    }

    @MainActor func testStandardLayoutSwitchRewritesOnlyTheConventionBlock() {
        let model = ProjectBrowserModel(client: EmptyProjectList(), databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: NewProjectTransport())
        model.createDraft.agentContext = "keep"
        model.setCreateStandardLayout(true, block: NewProjectLayout.chinese)
        XCTAssertTrue(model.createDraft.standardLayout)
        XCTAssertEqual(model.createDraft.agentContext, "keep\n\n" + NewProjectLayout.chinese)
        model.setCreateStandardLayout(true, block: NewProjectLayout.chinese)
        XCTAssertEqual(model.createDraft.agentContext.components(separatedBy: NewProjectLayout.chinese).count, 2)
        model.setCreateStandardLayout(false, block: NewProjectLayout.chinese)
        XCTAssertEqual(model.createDraft.agentContext, "keep")
        XCTAssertFalse(model.createDraft.standardLayout)
    }

    @MainActor func testFixtureSummaryDecodesAndSuccessOpensThatProjectOnce() async throws {
        let transport = NewProjectTransport()
        let lists = EmptyProjectList()
        let model = ProjectBrowserModel(client: lists, databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: transport)
        model.createPresented = true
        model.createDraft.name = "RNA-seq 研究"
        model.createDraft.directory = "/Users/researcher/Projects/RNA seq"
        model.createDraft.description = "Differential expression analysis"
        model.createDraft.agentContext = "Keep raw data untouched."
        await model.submitNewProject()
        let call = await transport.lastCall()
        let createdCalls = await transport.callCount()
        XCTAssertEqual(createdCalls, 1)
        XCTAssertEqual(call.command, NativeProjectCommand.create)
        XCTAssertNil(call.projectID)
        XCTAssertEqual(call.args["name"]?.string, "RNA-seq 研究")
        XCTAssertEqual(call.args["workspace_dir"]?.string, "/Users/researcher/Projects/RNA seq")
        XCTAssertEqual(call.args["standard_layout"]?.bool, false)
        XCTAssertFalse(model.createPresented)
        XCTAssertEqual(model.createDraft, NewProjectDraft())
        XCTAssertNil(model.createError)
        XCTAssertFalse(model.createBusy)
        XCTAssertEqual(model.activeProjectID, "research-1")
        XCTAssertEqual(model.projects.first?.workspaceDirectory, "/Users/researcher/Projects/RNA seq")
        let listsAfterCreate = await lists.listCount()
        XCTAssertEqual(listsAfterCreate, 1)
        await Task.yield()
        let callsAfterYield = await transport.callCount()
        XCTAssertEqual(callsAfterYield, 1)
    }

    @MainActor func testValidationAndLostReplyKeepTheDraftAndDoNotRetry() async {
        let transport = NewProjectTransport()
        let model = ProjectBrowserModel(client: EmptyProjectList(), databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: transport)
        model.createPresented = true
        model.createDraft.name = "  "
        model.createDraft.directory = "/tmp/study"
        model.createDraft.agentContext = "draft"
        await transport.setMode("validation")
        await model.submitNewProject()
        XCTAssertTrue(model.createPresented)
        XCTAssertEqual(model.createDraft.agentContext, "draft")
        XCTAssertEqual(model.createDraft.directory, "/tmp/study")
        XCTAssertEqual(model.createError, "请填写项目名称。")
        XCTAssertNil(model.activeProjectID)
        await transport.setMode("missing-dir")
        await model.submitNewProject()
        XCTAssertEqual(model.createError, "请填写工作目录。")
        await transport.setMode("registered")
        await model.submitNewProject()
        XCTAssertEqual(model.createError, "这个文件夹已经登记为项目。")
        await transport.setMode("unwritable")
        await model.submitNewProject()
        XCTAssertTrue(model.createError?.contains("工作目录不可写") == true)
        XCTAssertEqual(model.createDraft.agentContext, "draft")
        await transport.setMode("lost")
        await model.submitNewProject()
        XCTAssertTrue(model.createError?.contains("不会自动重试") == true)
        XCTAssertTrue(model.createPresented)
        let failedCalls = await transport.callCount()
        XCTAssertEqual(failedCalls, 5)
        await Task.yield()
        let failedCallsAfterYield = await transport.callCount()
        XCTAssertEqual(failedCallsAfterYield, 5)
    }

    @MainActor func testSecondSubmitWhileCreatingDoesNotSendAgain() async {
        let transport = NewProjectTransport()
        await transport.suspend()
        let model = ProjectBrowserModel(client: EmptyProjectList(), databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: transport)
        model.createPresented = true
        model.createDraft.name = "Study"
        model.createDraft.directory = "/tmp/study"
        let first = Task { await model.submitNewProject() }
        var hanging = false
        for _ in 0..<1000 {
            if await transport.isHanging() { hanging = true; break }
            await Task.yield()
        }
        XCTAssertTrue(hanging)
        XCTAssertTrue(model.createBusy)
        model.dismissNewProject()
        XCTAssertTrue(model.createPresented)
        await model.submitNewProject()
        let busyCalls = await transport.callCount()
        XCTAssertEqual(busyCalls, 1)
        await transport.resume(with: try! NewProjectTransport.fixtureResult())
        await first.value
        XCTAssertFalse(model.createPresented)
        XCTAssertEqual(model.activeProjectID, "research-1")
        let finishedCalls = await transport.callCount()
        XCTAssertEqual(finishedCalls, 1)
    }

    @MainActor func testImmediateEscapeClosesOnlyTheNewProjectSheet() throws {
        _ = NSApplication.shared
        let model = ProjectBrowserModel(client: EmptyProjectList(), databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: NewProjectTransport())
        model.searchPresented = true
        model.createPresented = true
        model.createDraft.name = "kept"
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 480), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let root = NSView(frame: window.contentLayoutRect)
        window.contentView = root
        let search = NSView(frame: .zero)
        root.addSubview(search)
        var searchClosed = false
        let searchOwner = NativeSettingsEscape.Coordinator(enabled: true) { searchClosed = true; model.searchPresented = false }
        searchOwner.view = search
        searchOwner.install()
        defer { searchOwner.remove() }
        let host = NSHostingView(rootView: NewProjectSheet(model: model))
        root.addSubview(host)
        host.frame = root.bounds
        host.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertFalse(model.createPresented)
        XCTAssertTrue(model.searchPresented)
        XCTAssertFalse(searchClosed)
        XCTAssertEqual(model.createDraft.name, "kept")
        XCTAssertTrue(window.firstResponder === focus)
    }
}
