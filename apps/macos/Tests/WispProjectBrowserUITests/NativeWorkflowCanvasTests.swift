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
    @MainActor func testNodeDragPreservesViewportDistanceAcrossZoomAndPanWithoutChangingTemplate() throws {
        let source = try fixture(), draft = NativeWorkflowDraft(template: source, client: WorkflowHost(), projectID: "p")
        let origin = draft.layout.nodes[1].point
        for zoom in [0.25, 0.69, 1.0, 2.0] {
            draft.zoom = zoom; draft.camera = CGPoint(x: 130, y: -40)
            draft.moveNode(1, origin: origin, viewportTranslation: CGSize(width: 120, height: -80))
            let moved = try XCTUnwrap(draft.positions[1])
            XCTAssertEqual((moved.x - origin.x) * zoom, 120, accuracy: 0.001)
            XCTAssertEqual((moved.y - origin.y) * zoom, -80, accuracy: 0.001)
            XCTAssertEqual(draft.template, source); XCTAssertFalse(draft.dirty)
        }
        draft.moveNode(99, origin: origin, viewportTranslation: CGSize(width: 20, height: 20))
        XCTAssertNil(draft.positions[99])
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
    @MainActor func testPortConnectionsRejectCyclesAndBlankCreationUsesCanvasCoordinates() throws {
        let draft = NativeWorkflowDraft(template: try fixture(), client: WorkflowHost(), projectID: "p")
        draft.dependency(source: 0, target: 1, enabled: false)
        draft.zoom = 0.5; draft.camera = CGPoint(x: 30, y: 40)
        draft.beginConnection(0)
        let target = draft.point(draft.layout.nodes[1])
        draft.finishConnection(at: CGPoint(x: target.x * draft.zoom + draft.camera.x, y: (target.y + 70) * draft.zoom + draft.camera.y))
        XCTAssertEqual(draft.tasks[1]["depends_on"], .array([draft.tasks[0]["id"]])); XCTAssertNil(draft.connectionSource)
        draft.beginConnection(1); draft.finishConnection(0)
        XCTAssertTrue(draft.tasks[0]["depends_on"].array.isEmpty); XCTAssertNotNil(draft.error)
        draft.beginConnection(0); draft.finishConnection(at: CGPoint(x: -900, y: -900)); XCTAssertNil(draft.connectionSource)
        draft.addNode(at: CGPoint(x: 230, y: 140)); XCTAssertEqual(draft.tasks.count, 4)
        XCTAssertEqual(draft.positions[3], CGPoint(x: 280, y: 130))
        let before = draft.positions; draft.fit(CGSize(width: 700, height: 400)); XCTAssertEqual(draft.positions, before)
        var builtin = try fixture(); builtin["builtin"] = .bool(true)
        let readonly = NativeWorkflowDraft(template: builtin, client: WorkflowHost(), projectID: "p")
        readonly.beginConnection(0); readonly.addNode(at: .zero)
        XCTAssertNil(readonly.connectionSource); XCTAssertEqual(readonly.tasks.count, 3)
    }
    @MainActor func testLayoutSurvivesReopenRenameRemovalAndIsIsolatedByDatabaseProjectAndTemplate() throws {
        let name = "workflow-test-" + UUID().uuidString
        let defaults = try XCTUnwrap(UserDefaults(suiteName: name)); defer { defaults.removePersistentDomain(forName: name) }
        let store = NativeWorkflowLayoutStore(scope: "/db/a", defaults: defaults), source = try fixture()
        let draft = NativeWorkflowDraft(template: source, client: WorkflowHost(), projectID: "p", layoutStore: store)
        draft.zoom = 0.69; draft.camera = CGPoint(x: -32, y: 68); draft.positions[1] = CGPoint(x: 888, y: 333)
        draft.persistLayout()
        let restored = NativeWorkflowDraft(template: source, client: WorkflowHost(), projectID: "p", layoutStore: store)
        XCTAssertEqual(restored.positions[1], draft.positions[1]); XCTAssertEqual(restored.zoom, 0.69); XCTAssertEqual(restored.camera, draft.camera); XCTAssertFalse(restored.dirty)
        XCTAssertNil(store.read(project: "other", template: source["id"].string))
        XCTAssertNil(store.read(project: "p", template: "another-template"))
        XCTAssertNil(NativeWorkflowLayoutStore(scope: "/db/b", defaults: defaults).read(project: "p", template: source["id"].string))
        var renamed = draft.tasks[1]; renamed["id"] = .string("renamed"); XCTAssertTrue(draft.updateNode(1, value: renamed))
        draft.removeNode(0); draft.persistLayout()
        let after = NativeWorkflowDraft(template: draft.template, client: WorkflowHost(), projectID: "p", layoutStore: store)
        XCTAssertEqual(after.tasks[0]["id"], .string("renamed")); XCTAssertEqual(after.positions[0], CGPoint(x: 888, y: 333))
        after.resetLayout(); let reset = NativeWorkflowDraft(template: draft.template, client: WorkflowHost(), projectID: "p", layoutStore: store)
        XCTAssertEqual(reset.point(reset.layout.nodes[0]), reset.layout.nodes[0].point)
    }
    @MainActor func testEscapeCancelsPendingPortConnectionBeforeClosingCanvas() throws {
        let draft = NativeWorkflowDraft(template: try fixture(), client: WorkflowHost(), projectID: "p")
        draft.beginConnection(0)
        var closed = false
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 800), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.contentView = nil; window.close() }
        let host = NSHostingView(rootView: NativeWorkflowCanvasEditor(draft: draft, close: { closed = true }, saved: {}))
        window.contentView = host; host.layoutSubtreeIfNeeded(); let focus = window.firstResponder
        let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53))
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertNil(draft.connectionSource); XCTAssertFalse(closed); XCTAssertTrue(window.firstResponder === focus)
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
