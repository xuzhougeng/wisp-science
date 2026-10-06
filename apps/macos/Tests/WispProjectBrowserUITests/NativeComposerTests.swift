import AppKit
import SwiftUI
import Foundation
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor ComposerClient: NativeConversationQuerying {
    var calls: [(String, [String: SettingsValue])] = []
    var profile: SettingsValue = .object(["id": .string("p"), "provider": .string("openai"), "api_url": .string("https://example.test"), "model": .string("vendor/model-sibling"), "label": .string("Model"), "reasoning_effort": .string("low"), "max_tokens": .integer(8192), "has_key": .bool(true), "use_for_vision": .bool(true)])
    var contextID = "ssh:gpu"
    var catalog: [String] = ["low", "high", "invented"]
    var held: CheckedContinuation<SettingsValue, Never>?
    var holdSearch = false
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { throw ProjectBrowserError.invalidResponse }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        calls.append((command, args))
        switch command {
        case "native_conversation_panel_contexts":
            return .object(["contexts": .array([context("local", "local"), context("ssh:gpu", "ssh"), context("wsl:x", "wsl")]), "enabled_ids": .array([.string("ssh:gpu")]), "read_only": .bool(false), "default_context": .object(["context_id": .string(contextID)])])
        case "get_default_execution_context": return .string("local")
        case "native_conversation_panel_activity": return .object(["runtimes": .array([]), "runs": .array([]), "read_only": .bool(false)])
        case "native_conversation_panel_context_default": contextID = args["context_id"]!.string; return .object(["context_id": .string(contextID)])
        case "model_catalog_lookup": return .object(["efforts": .array(catalog.map(SettingsValue.string))])
        case "list_models": return .array([profile])
        case "save_model": profile = args["profile"]!; return .array([profile])
        case "native_conversation_references":
            if holdSearch { holdSearch = false; return await withCheckedContinuation { held = $0 } }
            return catalogResponse(args["session_id"]!.string)
        default: throw ProjectBrowserError.invalidResponse
        }
    }
    private func context(_ id: String, _ kind: String) -> SettingsValue { .object(["id": .string(id), "kind": .string(kind), "label": .string(id), "config_json": .string("{}"), "capabilities_json": .string("{}")]) }
    private func catalogResponse(_ session: String) -> SettingsValue { .object(["session_id": .string(session), "options": .array([.object(["reference": .object(["kind": .string("artifact"), "id": .string("result-id")]), "label": .string("QC"), "detail": .string("results/qc.csv")])])]) }
    func row() -> SettingsValue { profile }
    func updateProfile() { profile["max_tokens"] = .integer(16384) }
    func recorded() -> [(String, [String: SettingsValue])] { calls }
    func hold() { holdSearch = true }
    func isHeld() -> Bool { held != nil }
    func finish() { held?.resume(returning: catalogResponse("s")); held = nil }
}
final class NativeComposerTests: XCTestCase {
    @MainActor func testCatalogUsesExactIDAndSavePreservesFreshProfileAndCredentials() async throws {
        let client = ComposerClient(); let model = NativeComposerModel(client: client)
        await model.bind(project: "project", session: "s", profile: client.row())
        XCTAssertEqual(model.efforts, ["low", "high"])
        XCTAssertEqual(model.contextID, "ssh:gpu")
        XCTAssertEqual(model.runtimeLabel, "运行时 · 未启动")
        await client.updateProfile(); await model.selectEffort("high")
        XCTAssertEqual(model.effort, "high")
        let calls = await client.recorded()
        XCTAssertEqual(calls.first { $0.0 == "model_catalog_lookup" }?.1["model"]?.string, "vendor/model-sibling")
        let save = try XCTUnwrap(calls.first { $0.0 == "save_model" }?.1)
        XCTAssertEqual(save["profile"]?["max_tokens"], .integer(16384))
        XCTAssertEqual(save["key"], .null); XCTAssertNil(save["profile"]?.object["key"])
        XCTAssertEqual(save["useForVision"], .bool(true))
        await model.selectEffort("ultra")
        let after = await client.recorded(); XCTAssertEqual(after.filter { $0.0 == "save_model" }.count, 1)
        XCTAssertFalse(calls.contains { $0.0.contains("runtime_start") || $0.0.contains("probe") })
    }
    @MainActor func testOnlyAttachedContextsCanBecomeSessionDefault() async {
        let client = ComposerClient(); let model = NativeComposerModel(client: client)
        await model.bind(project: "project", session: "s", profile: .null)
        await model.selectContext("wsl:x")
        await model.selectContext("local")
        XCTAssertEqual(model.contextID, "local")
        let calls = await client.recorded().filter { $0.0.hasSuffix("context_default") }
        XCTAssertEqual(calls.count, 1); XCTAssertEqual(calls.first?.1["session_id"], .string("s"))
    }
    @MainActor func testDismissedSearchCannotPublishLateCandidates() async {
        let client = ComposerClient(); let model = NativeComposerModel(client: client)
        await model.bind(project: "project", session: "s", profile: .null)
        await client.hold()
        let search = Task { await model.search(kind: "artifact", query: "slow") }
        while !(await client.isHeld()) { await Task.yield() }
        model.dismissSearch(); await client.finish(); await search.value
        XCTAssertTrue(model.options.isEmpty); XCTAssertFalse(model.searching)
        await model.search(kind: "artifact", query: "qc")
        XCTAssertEqual(model.options.map(\.id), ["artifact:result-id"])
        model.reset(); XCTAssertTrue(model.options.isEmpty); XCTAssertNil(model.contexts)
    }
    @MainActor func testReferencePickerConsumesImmediateEscapeBeforeItsParent() {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 540, height: 440), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let container = NSView(frame: window.contentView!.bounds); window.contentView = container
        var parentClosed = 0, pickerClosed = 0, selected = 0
        let parent = NativeSettingsEscape.Coordinator(enabled: true) { parentClosed += 1 }; parent.view = container; parent.install(); defer { parent.remove() }
        let hosted = NSHostingView(rootView: NativeComposerReferencePicker(model: NativeComposerModel(client: ComposerClient()), select: { _ in selected += 1 }, close: { pickerClosed += 1 }))
        hosted.frame = container.bounds; container.addSubview(hosted); hosted.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertEqual(pickerClosed, 1); XCTAssertEqual(parentClosed, 0); XCTAssertEqual(selected, 0)
        XCTAssertTrue(window.firstResponder === focus)
        hosted.removeFromSuperview()
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertEqual(parentClosed, 1)
    }
    func testReferenceMessageKeepsIDsSeparateFromDisplayNames() {
        let refs = [NativeComposerReference(reference: .object(["kind": .string("artifact"), "id": .string("exact-id")]), label: "QC\nreport", detail: "results/qc.csv"), NativeComposerReference(reference: .object(["kind": .string("runtime"), "context_id": .string("ssh:gpu"), "language": .string("python")]), label: "GPU Python", detail: "")]
        XCTAssertTrue(refs.allSatisfy(\.valid))
        XCTAssertEqual(NativeComposerReference.message(" Inspect ", references: refs), "Inspect\n\nAttached artifacts: QC report\n\nTarget runtimes: GPU Python")
        XCTAssertEqual(refs[0].reference["id"], .string("exact-id"))
        XCTAssertFalse(NativeComposerReference(reference: .object(["kind": .string("runtime"), "context_id": .string("local"), "language": .string("shell")]), label: "bad", detail: "").valid)
    }
}
