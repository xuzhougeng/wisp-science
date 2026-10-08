import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor WorkflowHost: NativeSettingsQuerying {
    var saved: SettingsValue?
    var loseWrite = false
    var writes = 0
    var args: [String: SettingsValue] = [:]
    var project: String?
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        if command == "list_workflow_templates" { return .array(saved.map { [$0] } ?? []) }
        writes += 1; self.args = args; self.project = projectID; saved = args["template"]
        if loseWrite { throw ProjectBrowserError.invalidResponse }
        return saved ?? .null
    }
    func loseNextWrite() { loseWrite = true }
    func captured() -> ([String: SettingsValue], String?, Int) { (args, project, writes) }
}

final class NativeWorkflowCanvasTests: XCTestCase {
    private func fixture() throws -> SettingsValue {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        return try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-workflows/v1/template.json")))
    }
    @MainActor func testParallelStagesCoordinatesAndInvalidGraphsStayVisible() throws {
        let source = try fixture(), tasks = source["proposal"]["tasks"].array
        let layout = NativeWorkflowLayout(tasks: tasks)
        XCTAssertEqual(layout.nodes.map(\.level), [0, 1, 1]); XCTAssertEqual(layout.stageCount, 2); XCTAssertEqual(layout.edges.count, 2)
        XCTAssertNotEqual(layout.nodes[1].point.y, layout.nodes[2].point.y)
        XCTAssertEqual(NativeWorkflowLayout.point(CGPoint(x: 220, y: 120), camera: CGPoint(x: 20, y: 20), zoom: 2), CGPoint(x: 100, y: 50))
        XCTAssertEqual(NativeWorkflowLayout.fit(size: CGSize(width: 4000, height: 1000), viewport: CGSize(width: 420, height: 400)), 0.25)
        var invalid = tasks; invalid[0]["depends_on"] = .array([.string("review")])
        XCTAssertTrue(NativeWorkflowDraft.graphIssues(invalid).contains(localized("依赖关系存在循环。")))
        XCTAssertEqual(NativeWorkflowLayout(tasks: invalid).nodes.count, 3)
        invalid[0]["depends_on"] = .array([.string("missing")]); XCTAssertTrue(NativeWorkflowDraft.graphIssues(invalid).contains { $0.contains("missing") })
        invalid[1]["id"] = .string("source"); XCTAssertTrue(NativeWorkflowDraft.graphIssues(invalid).contains { $0.hasPrefix(localized("重复节点 ID：")) })
    }
    @MainActor func testRenameUpdatesEdgesAndActivityInputsWithoutLosingMetadata() async throws {
        let source = try fixture(), host = WorkflowHost(), draft = NativeWorkflowDraft(template: source, sourceHash: .string("verified-source-hash"), client: host, projectID: "project-1")
        var task = draft.tasks[0]; task["id"] = .string("renamed"); task["instruction"] = .string("Updated evidence reading")
        XCTAssertTrue(draft.updateNode(0, value: task))
        XCTAssertEqual(draft.tasks[1]["depends_on"], .array([.string("renamed")]))
        XCTAssertEqual(draft.tasks[2]["run_activity"]["input_task_id"], .string("renamed"))
        for key in ["budget", "executor", "model_id", "timeout_secs", "output_schema", "isolated"] { XCTAssertEqual(draft.tasks[0][key], source["proposal"]["tasks"].array[0][key], key) }
        draft.dependency(source: 1, target: 0, enabled: true)
        XCTAssertTrue(draft.tasks[0]["depends_on"].array.isEmpty); XCTAssertNotNil(draft.error)
        XCTAssertTrue(draft.issues.isEmpty)
        let saved = await draft.save(); XCTAssertTrue(saved)
        let captured = await host.captured(); XCTAssertEqual(captured.0["conversionSourceSha256"], .string("verified-source-hash")); XCTAssertEqual(captured.1, "project-1"); XCTAssertEqual(captured.2, 1)
    }
    @MainActor func testBuiltInCopyUnknownWriteAndFreshReadRequireExplicitAcknowledgement() async throws {
        let host = WorkflowHost(); var source = try fixture(); source["builtin"] = .bool(true)
        let draft = NativeWorkflowDraft(template: source, client: host, projectID: "p")
        let readOnlySaved = await draft.save(); XCTAssertFalse(readOnlySaved)
        draft.addNode(); XCTAssertEqual(draft.tasks.count, 3)
        draft.copy(); XCTAssertFalse(draft.readOnly); XCTAssertNotEqual(draft.template["id"], source["id"]); XCTAssertEqual(draft.template["proposal"], source["proposal"])
        await host.loseNextWrite(); let saved = await draft.save(); XCTAssertFalse(saved); XCTAssertTrue(draft.uncertain)
        let retried = await draft.save(); XCTAssertFalse(retried); draft.acknowledge(); XCTAssertTrue(draft.uncertain)
        await draft.reconcile(); XCTAssertTrue(draft.reconciled); XCTAssertNotNil(draft.persisted); XCTAssertTrue(draft.uncertain)
        draft.acknowledge(); XCTAssertFalse(draft.uncertain)
        let captured = await host.captured(); XCTAssertEqual(captured.2, 1)
    }
    @MainActor func testImmediateEscapeClosesNodeEditorAndKeepsCanvas() throws {
        let draft = NativeWorkflowDraft(template: try fixture(), client: WorkflowHost(), projectID: "p")
        var canvasClosed = false, nodeClosed = false
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 700), styleMask: [.titled], backing: .buffered, defer: false)
        let container = NSView(frame: window.contentView!.bounds); window.contentView = container
        let parent = NSHostingView(rootView: NativeWorkflowCanvasEditor(draft: draft, close: { canvasClosed = true }, saved: {}))
        parent.frame = container.bounds; container.addSubview(parent); parent.layoutSubtreeIfNeeded()
        let editor = NativeWorkflowNodeDraft(index: 0, value: draft.tasks[0])
        let child = NSHostingView(rootView: NativeWorkflowNodeEditor(draft: draft, editor: editor) { nodeClosed = true })
        child.frame = container.bounds; container.addSubview(child); child.layoutSubtreeIfNeeded(); let focus = window.firstResponder
        let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53))
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertTrue(nodeClosed); XCTAssertFalse(canvasClosed); XCTAssertTrue(window.firstResponder === focus)
        child.removeFromSuperview(); XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertTrue(canvasClosed)
        window.contentView = nil
    }
    @MainActor func testRenderCanvasAndNodeEditorAcrossLocalesSchemesAndWidths() throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Opt-in native rendering") }
        let old = UserDefaults.standard.object(forKey: "nativeSettings.locale"); defer { if let old { UserDefaults.standard.set(old, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        for locale in ["en", "zh"] { for scheme in [ColorScheme.light, .dark] { for width in [432.0, 1060.0] {
            UserDefaults.standard.set(locale, forKey: "nativeSettings.locale")
            let draft = NativeWorkflowDraft(template: try fixture(), client: WorkflowHost(), projectID: "p"); draft.selectedNode = 0
            draft.zoom = width == 432 ? 0.52 : 1
            let host = NSHostingView(rootView: NativeWorkflowCanvasEditor(draft: draft, close: {}, saved: {}).environment(\.colorScheme, scheme))
            host.frame = NSRect(x: 0, y: 0, width: width, height: 800); host.layoutSubtreeIfNeeded()
            let image = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds)); host.cacheDisplay(in: host.bounds, to: image)
            let bytes = try XCTUnwrap(image.representation(using: .png, properties: [:])); try bytes.write(to: URL(fileURLWithPath: directory).appendingPathComponent("workflow-canvas-\(locale)-\(scheme)-\(Int(width)).png"))
        } } }
    }
}
