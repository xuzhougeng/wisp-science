import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor TabsClient: NativeConversationQuerying {
    var runs: [SettingsValue] = []
    var memos: SettingsValue = .array([])
    var foreign = false
    var calls: [(String, [String: SettingsValue], String)] = []
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { throw ProjectBrowserError.invalidResponse }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        calls.append((command, args, projectID))
        let owner = SettingsValue.string(foreign ? "other" : projectID)
        switch command {
        case "native_conversation_panel_activity": return .object(["runtimes": .array([]), "runs": .array(runs), "read_only": .bool(false)])
        case "list_project_memos": return memos
        case "add_project_memo":
            return .object(["id": .string("memo-new"), "project_id": owner, "title": args["input"]!["title"], "body": args["input"]!["body"], "pinned": .bool(false), "created_at": .integer(9), "updated_at": .integer(9)])
        case "update_project_memo":
            var memo = memos.array.first { $0["id"] == args["patch"]!["id"] }!
            memo["pinned"] = args["patch"]!["pinned"]; memo["project_id"] = owner; return memo
        case "delete_project_memo": return .null
        default: throw ProjectBrowserError.invalidResponse
        }
    }
    func set(runs: [SettingsValue]) { self.runs = runs }
    func set(memos: SettingsValue) { self.memos = memos }
    func setForeign() { foreign = true }
    func recorded() -> [(String, [String: SettingsValue], String)] { calls }
}

final class NativeSessionPanelTabsTests: XCTestCase {
    private func fixture() throws -> SettingsValue {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        return try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-settings/v1/project-memos.json")))
    }
    private func run(_ id: String, _ frame: String?, _ status: String, _ created: Int64, _ ended: Int64? = nil) -> SettingsValue {
        .object(["id": .string(id), "frame_id": frame.map(SettingsValue.string) ?? .null, "context_id": .string("local"), "title": .string(id), "kind": .string(id.hasPrefix("xfer") ? "file_transfer" : "ssh_direct"), "status": .string(status), "created_at": .integer(created), "ended_at": ended.map(SettingsValue.integer) ?? .null, "progress_json": .string("")])
    }

    /// Mirrors `session_run_groups` tests in ui/src/app_support/runtime.rs.
    @MainActor func testRunsTabKeepsThisConversationAndSortsLikeWebView() async {
        let client = TabsClient()
        await client.set(runs: [run("a-run", "frame-a", "running", 10), run("b-run", "frame-b", "running", 11), run("xfer-a", "frame-a", "submitted", 12), run("orphan", nil, "running", 13), run("a-done", "frame-a", "succeeded", 5, 9),
                                run("early", "frame-a", "succeeded", 1, 20), run("late", "frame-a", "failed", 2, 30), run("never-ended", "frame-a", "lost", 25), run("draft", "frame-a", "draft", 40), run("paused", "frame-a", "paused", 3)])
        let model = NativeContextActivityModel(client: client, projectID: "project", sessionID: "frame-a", contextID: "")
        await model.refresh()
        let groups = NativeSessionRuns.group(model.runs)
        XCTAssertEqual(groups.active.map(\.id), ["xfer-a", "a-run", "paused"])
        XCTAssertEqual(groups.completed.map(\.id), ["draft", "late", "never-ended", "early", "a-done"])
        XCTAssertEqual(NativeSessionRuns.tabTitle(model.runs), "运行 (3)")
        XCTAssertEqual(NativeSessionRuns.tabTitle([]), "运行")
        XCTAssertEqual(NativeSessionRuns.statusLabel("timed_out"), "已超时")
        XCTAssertEqual(NativeSessionRuns.statusLabel("unknown"), "状态不可用")
        // A context-scoped model still filters by environment, not conversation.
        let scoped = NativeContextActivityModel(client: client, projectID: "project", sessionID: "frame-a", contextID: "local")
        await scoped.refresh(); XCTAssertEqual(scoped.runs.count, 10)
    }

    /// Mirrors the helper tests in ui/src/memos.rs on the shared fixture.
    func testMemoHelpersMatchWebView() throws {
        let memos = try JSONDecoder().decode([NativeProjectMemo].self, from: JSONEncoder().encode(fixture()["memos"]))
        XCTAssertEqual(memos.map(\.heading), ["Reviewer 2 wants the batch-corrected UMAP", "QC thresholds"])
        XCTAssertEqual(NativeProjectMemos.composeInsert("   \n", "ssh gpu02"), "ssh gpu02")
        XCTAssertEqual(NativeProjectMemos.composeInsert("Compare methods\n", "ssh gpu02"), "Compare methods\n\nssh gpu02")
        XCTAssertEqual(NativeProjectMemos.filter(memos, query: "umap").map(\.id), ["memo-pinned"])
        XCTAssertEqual(NativeProjectMemos.filter(memos, query: "QC").map(\.id), ["memo-titled"])
        XCTAssertEqual(NativeProjectMemos.sorted(memos.reversed()).map(\.id), ["memo-pinned", "memo-titled"])
    }

    @MainActor func testMemoCommandsSendFixtureArgumentsAndRejectAnotherProject() async throws {
        let fixture = try fixture(); let client = TabsClient()
        await client.set(memos: fixture["memos"])
        let model = NativeProjectMemosModel(client: client, projectID: "project")
        await model.load()
        XCTAssertEqual(model.memos.map(\.id), ["memo-pinned", "memo-titled"])
        let added = await model.save(id: nil, title: "", body: "Check the doublet rate")
        XCTAssertTrue(added)
        await model.togglePin(model.memos.first { $0.id == "memo-titled" }!)
        XCTAssertEqual(model.memos.first { $0.id == "memo-titled" }?.pinned, true)
        await model.delete(model.memos.first { $0.id == "memo-titled" }!)
        XCTAssertFalse(model.memos.contains { $0.id == "memo-titled" })
        let calls = await client.recorded()
        XCTAssertTrue(calls.allSatisfy { $0.2 == "project" })
        XCTAssertEqual(calls.first { $0.0 == "add_project_memo" }?.1, fixture["add_args"].object)
        XCTAssertEqual(calls.first { $0.0 == "update_project_memo" }?.1, fixture["update_args"].object)
        XCTAssertEqual(calls.first { $0.0 == "delete_project_memo" }?.1, fixture["delete_args"].object)

        await client.setForeign()
        let before = model.memos
        let leaked = await model.save(id: nil, title: "", body: "x")
        XCTAssertFalse(leaked); XCTAssertEqual(model.memos, before); XCTAssertNotNil(model.error)
    }

    @MainActor func testMemoEditorConsumesImmediateEscapeBeforeThePanel() {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 420, height: 360), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let container = NSView(frame: window.contentView!.bounds); window.contentView = container
        var panelClosed = 0, editorClosed = 0, saved = 0
        let panel = NativeSettingsEscape.Coordinator(enabled: true) { panelClosed += 1 }; panel.view = container; panel.install(); defer { panel.remove() }
        let hosted = NSHostingView(rootView: NativeProjectMemoEditor(title: .constant(""), text: .constant("draft"), busy: false, save: { saved += 1 }, close: { editorClosed += 1 }))
        hosted.frame = container.bounds; container.addSubview(hosted); hosted.layoutSubtreeIfNeeded()
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertEqual(editorClosed, 1); XCTAssertEqual(panelClosed, 0); XCTAssertEqual(saved, 0)
        hosted.removeFromSuperview()
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertEqual(panelClosed, 1)
    }
}
