import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

final class NativePublicationTests: XCTestCase {
    func testOutlineKeepsParentOrderAndRecoversOrphansAndCycles() {
        func item(_ id: String, _ parent: String?, _ ordinal: Int64) -> PublicationItemRecord {
            PublicationItemRecord(id: id, title: id, kind: "section", ordinal: ordinal, revisionID: "r", parentItemID: parent, content: "")
        }
        let rows = PublicationOutlineRow.build([
            item("child", "parent", 0), item("parent", nil, 10), item("orphan", "missing", 20),
            item("cycleA", "cycleB", 30), item("cycleB", "cycleA", 31),
        ])
        XCTAssertEqual(rows.map(\.id), ["parent", "child", "orphan", "cycleA", "cycleB"])
        XCTAssertEqual(rows.map(\.depth), [0, 1, 0, 0, 1])
    }

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

    @MainActor func testRevisionSwitchClearsDetailsAndRejectsWrongRevision() async throws {
        let host = PublicationTransport()
        let publication = NativePublicationModel()
        publication.open(projectID: "research-1")
        await host.setPage(try PublicationTransport.evidencePage())
        await publication.reload(host)
        XCTAssertEqual(publication.workspace.revisions.map(\.id), ["rev-1", "rev-2"])
        publication.selectItem("item-1")
        XCTAssertEqual(publication.visibleBindings.map(\.id), ["binding-1"])
        publication.selectedBindingID = "binding-1"
        XCTAssertEqual(publication.selectedBinding?.sourceID, "artifact-version-17")
        await host.suspend()
        let task = Task { await publication.selectRevision("rev-2", client: host) }
        while !(await host.isHanging()) { await Task.yield() }
        let call = await host.lastCall()
        XCTAssertEqual(call.args["publication_id"], .string("pub-1"))
        XCTAssertEqual(call.args["revision_id"], .string("rev-2"))
        XCTAssertTrue(publication.busy)
        XCTAssertNil(publication.workspace.revision)
        XCTAssertTrue(publication.workspace.items.isEmpty)
        XCTAssertTrue(publication.visibleBindings.isEmpty)
        XCTAssertNil(publication.selectedBinding)
        await host.resume(with: try PublicationTransport.evidencePage())
        await task.value
        XCTAssertNotNil(publication.error)
        XCTAssertTrue(publication.workspace.bindings.isEmpty)
        await host.setMode("ok")
        await host.setPage(try PublicationTransport.evidencePage(revisionID: "rev-2"))
        await publication.reload(host)
        XCTAssertNil(publication.error)
        XCTAssertEqual(publication.workspace.revision?.id, "rev-2")
        XCTAssertEqual(publication.workspace.bindings.first?.revisionID, "rev-2")
    }

    @MainActor func testOpeningAnotherProjectClearsCachedEvidenceAndDismissCancelsPendingRead() async throws {
        let host = PublicationTransport()
        let publication = NativePublicationModel()
        publication.open(projectID: "research-1")
        await host.setPage(try PublicationTransport.evidencePage())
        await publication.reload(host)
        XCTAssertEqual(publication.workspace.bindings.count, 1)
        publication.open(projectID: "research-2")
        XCTAssertTrue(publication.workspace.publications.isEmpty)
        await host.suspend()
        let task = Task { await publication.reload(host) }
        while !(await host.isHanging()) { await Task.yield() }
        publication.dismiss()
        XCTAssertFalse(publication.busy)
        await host.resume(with: try PublicationTransport.evidencePage())
        await task.value
        XCTAssertFalse(publication.presented)
        XCTAssertTrue(publication.workspace.bindings.isEmpty)
    }

    @MainActor func testImmediateEscapeClosesOnlyThePublicationWorkspace() {
        _ = NSApplication.shared
        let model = ProjectBrowserModel(client: PublicationProjectList(), databaseURL: URL(fileURLWithPath: "/unused/escape.sqlite"), projectTransport: PublicationTransport())
        let publication = model.publication
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
        let host = NSHostingView(rootView: NativePublicationColumn(model: model, publication: publication))
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

private actor PublicationTransport: NativeSettingsQuerying {
    private var recorded: [(command: String, args: [String: SettingsValue], projectID: String?)] = []
    private var mode = "ok"
    private var page: SettingsValue?
    func setPage(_ page: SettingsValue) { self.page = page }
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
        if let page { return page }
        return try Self.fixturePage()
    }
    static func evidencePage(revisionID: String = "rev-1") throws -> SettingsValue {
        let original = try JSONEncoder().encode(fixturePage())
        var object = try JSONSerialization.jsonObject(with: original) as! [String: Any]
        let revision1: [String: Any] = ["id": "rev-1", "publication_id": "pub-1", "revision_number": 1, "label": "v1", "state": "draft"]
        let revision2: [String: Any] = ["id": "rev-2", "publication_id": "pub-1", "revision_number": 2, "label": "v2", "state": "frozen"]
        object["revisions"] = [revision1, revision2]
        object["revision"] = revisionID == "rev-1" ? revision1 : revision2
        object["items"] = [["id": "item-1", "title": "Claim", "kind": "claim", "ordinal": 0, "revision_id": revisionID, "content": "Evidence-backed claim"]]
        object["bindings"] = [["id": "binding-1", "revision_id": revisionID, "item_id": "item-1", "source_kind": "artifact_version", "source_id": "artifact-version-17", "purpose": "DE results", "selection_state": "selected", "review_state": "unreviewed", "reproduction_state": "not_run", "visibility": "private", "source_snapshot_json": "{\"version\":17}"]]
        return try JSONDecoder().decode(SettingsValue.self, from: JSONSerialization.data(withJSONObject: object))
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
