import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

final class NativeSessionGroupTests: XCTestCase {
    private func session(_ id: String, _ title: String, _ ts: Int64, _ folder: String? = nil) -> BrowserSession {
        BrowserSession(id: id, projectID: "p", title: title, ts: ts, status: "complete", folderID: folder)
    }

    func testSortAndGroupKeepStableOrder() {
        let sessions = [
            session("b", "Beta", 20, "f1"),
            session("a", "Alpha", 10, nil),
            session("c", "Alpha", 30, "f1"),
        ]
        let folders = [ProjectFolder(id: "f1", name: "Week")]
        let newest = SessionArrangement.sections(sessions, folders: folders, sort: "newest", group: "none")
        XCTAssertEqual(newest.single().map(\.id), ["c", "b", "a"])
        let named = SessionArrangement.sections(sessions, folders: folders, sort: "name", group: "none")
        XCTAssertEqual(named.single().map(\.id), ["a", "c", "b"])
        let grouped = SessionArrangement.sections(sessions, folders: folders, sort: "newest", group: "folder")
        XCTAssertEqual(grouped.map(\.title), ["Week", "未分组"])
        XCTAssertEqual(grouped[0].sessions.map(\.id), ["c", "b"])
        XCTAssertEqual(grouped[1].sessions.map(\.id), ["a"])
    }

    @MainActor func testCreateRequiresAProjectAndDoesNotRetryALostReply() async {
        let client = GroupClient()
        let groups = NativeSessionGroups()
        groups.creating = true
        groups.draft = "  "
        await groups.create(client, projectID: "project-a")
        let emptyCalls = await client.callCount()
        XCTAssertEqual(emptyCalls, 0)
        XCTAssertEqual(groups.error, "请填写分组名称。")
        groups.draft = "Week"
        await client.setMode("lost")
        await groups.create(client, projectID: "project-a")
        let calls = await client.callCount()
        let project = await client.lastProject()
        let command = await client.lastCommand()
        XCTAssertEqual(calls, 1)
        XCTAssertEqual(project, "project-a")
        XCTAssertEqual(command, "native_project_folder_create")
        XCTAssertTrue(groups.creating)
        XCTAssertEqual(groups.draft, "Week")
        XCTAssertTrue(groups.error?.contains("不会自动重试") == true)
        await Task.yield()
        let after = await client.callCount()
        XCTAssertEqual(after, 1)
    }

    @MainActor func testImmediateEscapeClosesOnlyTheNewGroupSheet() {
        _ = NSApplication.shared
        let groups = NativeSessionGroups()
        groups.creating = true
        groups.menuPresented = true
        groups.draft = "kept"
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 420, height: 280), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let root = NSView(frame: window.contentLayoutRect)
        window.contentView = root
        let menu = NSView(frame: .zero)
        root.addSubview(menu)
        let menuOwner = NativeSettingsEscape.Coordinator(enabled: true) { groups.menuPresented = false }
        menuOwner.view = menu
        menuOwner.install()
        defer { menuOwner.remove() }
        let host = NSHostingView(rootView: GroupCreateSheet(groups: groups))
        root.addSubview(host)
        host.frame = root.bounds
        host.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertFalse(groups.creating)
        XCTAssertTrue(groups.menuPresented)
        XCTAssertEqual(groups.draft, "kept")
        XCTAssertTrue(window.firstResponder === focus)
    }

    @MainActor func testRenameUsesTheProjectAndKeepsTheDraftWhenTheReplyIsLost() async {
        let client = GroupClient()
        let groups = NativeSessionGroups()
        await groups.load(client, projectID: "project-a")
        groups.beginRename("f1")
        groups.renameDraft = "  "
        await groups.rename(client, projectID: "project-a")
        let empty = await client.callCount()
        XCTAssertEqual(empty, 1)
        XCTAssertEqual(groups.error, "请填写分组名称。")
        XCTAssertEqual(groups.renamingID, "f1")
        groups.renameDraft = "Analysis"
        await client.setMode("lost")
        await groups.rename(client, projectID: "project-a")
        let calls = await client.calls
        XCTAssertEqual(calls.count, 2)
        XCTAssertEqual(calls[1].projectID, "project-a")
        XCTAssertEqual(calls[1].command, "native_project_folder_rename")
        XCTAssertEqual(calls[1].args["folder_id"]?.string, "f1")
        XCTAssertEqual(calls[1].args["name"]?.string, "Analysis")
        XCTAssertEqual(groups.renamingID, "f1")
        XCTAssertEqual(groups.renameDraft, "Analysis")
        XCTAssertTrue(groups.error?.contains("不会自动重试") == true)
        await Task.yield()
        let after = await client.callCount()
        XCTAssertEqual(after, 2)
    }

    @MainActor func testImmediateEscapeClosesOnlyTheRenameSheet() async {
        _ = NSApplication.shared
        let groups = NativeSessionGroups()
        let client = GroupClient()
        await groups.load(client, projectID: "project-a")
        groups.beginRename("f1")
        groups.menuPresented = true
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 420, height: 280), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let root = NSView(frame: window.contentLayoutRect)
        window.contentView = root
        let menu = NSView(frame: .zero)
        root.addSubview(menu)
        let menuOwner = NativeSettingsEscape.Coordinator(enabled: true) { groups.menuPresented = false }
        menuOwner.view = menu
        menuOwner.install()
        defer { menuOwner.remove() }
        let host = NSHostingView(rootView: GroupRenameSheet(groups: groups))
        root.addSubview(host)
        host.frame = root.bounds
        host.layoutSubtreeIfNeeded()
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertNil(groups.renamingID)
        XCTAssertTrue(groups.menuPresented)
        XCTAssertEqual(groups.renameDraft, "Week")
    }
}

private extension Array where Element == SessionSection {
    func single() -> [BrowserSession] {
        XCTAssertEqual(count, 1)
        return first?.sessions ?? []
    }
}

private struct GroupCreateSheet: View {
    @ObservedObject var groups: NativeSessionGroups
    var body: some View {
        Text("新建分组")
            .background(NativeSettingsEscape(enabled: !groups.busy) { groups.dismissCreate() })
    }
}

private struct GroupRenameSheet: View {
    @ObservedObject var groups: NativeSessionGroups
    var body: some View {
        Text("重命名分组")
            .background(NativeSettingsEscape(enabled: !groups.busy) { groups.dismissRename() })
    }
}

private actor GroupClient: NativeConversationQuerying {
    var calls: [(projectID: String, command: String, args: [String: SettingsValue])] = []
    var mode = "ok"
    func setMode(_ mode: String) { self.mode = mode }
    func callCount() -> Int { calls.count }
    func lastProject() -> String { calls.last?.projectID ?? "" }
    func lastCommand() -> String { calls.last?.command ?? "" }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { throw ProjectBrowserError.invalidResponse }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        calls.append((projectID, command, args))
        if mode == "lost" { throw ProjectBrowserError.service("connection reset") }
        if command == "native_project_folders" {
            return .array([.object(["id": .string("f1"), "name": .string("Week")])])
        }
        return .bool(true)
    }
}
