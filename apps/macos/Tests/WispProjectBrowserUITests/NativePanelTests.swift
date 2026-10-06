import Foundation
import SwiftUI
import AppKit
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor PanelClient: NativeConversationQuerying {
    let rows: SettingsValue
    var mutations = 0
    var calls: [(String, String, [String: SettingsValue])] = []
    func recorded() -> [(String, String, [String: SettingsValue])] { calls }
    func mutationCount() -> Int { mutations }
    var held: CheckedContinuation<SettingsValue, Error>?
    init(_ rows: SettingsValue) { self.rows = rows }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { throw ProjectBrowserError.invalidResponse }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        calls.append((projectID, command, args))
        if command.hasSuffix("context_enabled") { mutations += 1; throw ProjectBrowserError.invalidResponse }
        if args["path"]?.string == "slow" || command == "native_conversation_image" && args["path"]?.string.hasSuffix("slow.png") == true { return try await withCheckedThrowingContinuation { held = $0 } }
        return rows
    }
    func pending() -> Bool { held != nil }
    func finish() { held?.resume(returning: rows); held = nil }
}
private actor FileSaveClient: NativeConversationQuerying {
    let content: SettingsValue
    var saves: [[String: SettingsValue]] = []
    var fail = false
    init(_ content: SettingsValue) { self.content = content }
    func failNext() { fail = true }
    func recorded() -> [[String: SettingsValue]] { saves }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { throw ProjectBrowserError.invalidResponse }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        if command.hasSuffix("savefile") {
            saves.append(args)
            if fail { throw ProjectBrowserError.service("lost response") }
            return .bool(true)
        }
        return content
    }
}
final class NativePanelTests: XCTestCase {
    @MainActor func testArtifactMarkdownImageRequestsStayOwnedAndValidateReturnedSource() async throws {
        var original = try fixture("panel-preview"); original["path"] = .string("/work/reports/report.md"); original["text"] = .string("![plot](figures/plot.png)")
        let content = try JSONDecoder().decode(NativePanelFileContent.self, from: JSONEncoder().encode(original))
        for mode in ["success", "foreign", "closed"] {
            var image = try JSONDecoder().decode(SettingsValue.self, from: JSONEncoder().encode(imagePreview()))
            image["path"] = .string(mode == "foreign" ? "/foreign/plot.png" : "/work/reports/figures/plot.png")
            let client = PanelClient(image), model = NativePanelModel(client: client, projectID: "p", sessionID: "s"); model.preview = content
            if mode == "closed" { model.close() }
            do {
                let value = try await model.readPreviewImage("figures/plot.png", original: content)
                XCTAssertEqual(mode, "success"); XCTAssertNotNil(try NativeFileDocumentImages.decode(value))
            } catch { XCTAssertNotEqual(mode, "success") }
            let calls = await client.recorded(); XCTAssertEqual(calls.count, mode == "closed" ? 0 : 1)
            if let call = calls.first { XCTAssertEqual(call.0, "p"); XCTAssertEqual(call.1, "native_conversation_image"); XCTAssertEqual(call.2["session_id"], .string("s")); XCTAssertEqual(call.2["path"], .string("/work/reports/figures/plot.png")) }
        }
    }
    @MainActor func testLateArtifactMarkdownImageCannotPopulateDismissedPreview() async throws {
        var original = try fixture("panel-preview"); original["path"] = .string("/work/report.md")
        let content = try JSONDecoder().decode(NativePanelFileContent.self, from: JSONEncoder().encode(original))
        let client = PanelClient(original), model = NativePanelModel(client: client, projectID: "p", sessionID: "s"); model.preview = content
        let pending = Task { try await model.readPreviewImage("slow.png", original: content) }
        while !(await client.pending()) { await Task.yield() }
        model.dismissPreview(); await client.finish()
        do { _ = try await pending.value; XCTFail("Late image must be discarded") } catch {}
        XCTAssertNil(model.preview)
    }
    func fixture(_ name: String) throws -> SettingsValue {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        return try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-conversations/v1/\(name).json")))
    }
    @MainActor func testFilePreviewSaveUsesBaselineAndKeepsDraftOnUncertainResult() async throws {
        var payload = try fixture("panel-preview"); payload["truncated"] = .bool(false)
        let client = FileSaveClient(payload)
        let model = NativePanelModel(client: client, projectID: "p", sessionID: "s")
        await model.readFile("analysis.py")
        let original = try XCTUnwrap(model.preview)
        XCTAssertTrue(model.previewEditable)
        try await model.savePreview("edited text", original: original)
        XCTAssertEqual(model.preview?.text, "edited text")
        let calls = await client.recorded()
        XCTAssertEqual(calls.count, 1)
        XCTAssertEqual(calls[0]["original_text"]?.string, original.text)
        XCTAssertEqual(calls[0]["session_id"]?.string, "s")
        await client.failNext()
        do { try await model.savePreview("uncertain text", original: XCTUnwrap(model.preview)); XCTFail("Expected uncertain save") } catch {}
        XCTAssertEqual(model.preview?.text, "edited text")
        let final = await client.recorded(); XCTAssertEqual(final.count, 2)
        XCTAssertFalse(model.savingPreview)
        await model.readArtifact("artifact-a")
        XCTAssertFalse(model.previewEditable)
        do { try await model.savePreview("artifact overwrite", original: XCTUnwrap(model.preview)); XCTFail("Artifact preview is read-only") } catch {}
        let afterArtifact = await client.recorded(); XCTAssertEqual(afterArtifact.count, 2)
    }
    @MainActor func testTruncatedPreviewCannotBeSaved() async throws {
        let client = FileSaveClient(try fixture("panel-preview"))
        let model = NativePanelModel(client: client, projectID: "p", sessionID: "s")
        await model.readFile("large.txt")
        XCTAssertFalse(model.previewEditable)
        do { try await model.savePreview("prefix", original: XCTUnwrap(model.preview)); XCTFail("Truncated file cannot be saved") } catch {}
        let calls = await client.recorded(); XCTAssertTrue(calls.isEmpty)
    }
    @MainActor func testPreviewQuotePreservesSourceAndRejectsDismissedOrChangedFile() throws {
        let rows = try fixture("panel-preview")
        let model = NativePanelModel(client: PanelClient(rows), projectID: "p", sessionID: "s")
        let preview = try JSONDecoder().decode(NativePanelFileContent.self, from: JSONEncoder().encode(rows))
        let text = try XCTUnwrap(preview.text)
        model.preview = preview
        let quote = try XCTUnwrap(model.selectedPreviewQuote(text, path: preview.path))
        XCTAssertEqual(quote.text, text); XCTAssertEqual(quote.source, preview.path)
        XCTAssertNil(model.selectedPreviewQuote(text, path: "other.txt"))
        XCTAssertNil(model.selectedPreviewQuote("not contained in the preview", path: preview.path))
        XCTAssertNil(model.selectedPreviewQuote("  ", path: preview.path))
        model.dismissPreview()
        XCTAssertNil(model.selectedPreviewQuote(text, path: preview.path))
        model.preview = preview; model.close()
        XCTAssertNil(model.selectedPreviewQuote(text, path: preview.path))
    }
    @MainActor func testRenderFilePreviewQuotes() throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Opt-in native rendering") }
        let content = try JSONDecoder().decode(NativePanelFileContent.self, from: JSONEncoder().encode(fixture("panel-preview")))
        for (name, scheme) in [("file-quote-light", ColorScheme.light), ("file-quote-dark", ColorScheme.dark)] {
            let view = NSHostingView(rootView: NativePanelFilePreview(content: content, close: {}, quote: { _ in }).background(WispDesign.color("bg-elev", scheme)).environment(\.colorScheme, scheme))
            view.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
            view.frame = NSRect(x: 0, y: 0, width: 560, height: 420); view.layoutSubtreeIfNeeded()
            let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds)); view.cacheDisplay(in: view.bounds, to: bitmap)
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: URL(fileURLWithPath: directory).appendingPathComponent(name + ".png"))
        }
    }
    @MainActor func testRenderContextsAtNarrowPanelWidth() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Opt-in native rendering") }
        try FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
        let model = NativePanelModel(client: PanelClient(try fixture("panel-contexts")), projectID: "p", sessionID: "s")
        await model.refresh("hosts")
        for (name, scheme) in [("contexts-light", ColorScheme.light), ("contexts-dark", ColorScheme.dark)] {
            let view = NSHostingView(rootView: VStack(alignment: .leading, spacing: 10) {
                NativePanelContextsView(model: model)
                Spacer()
            }.padding(12).frame(width: 280, height: 500).background(WispDesign.color("bg-sunken", scheme)).environment(\.colorScheme, scheme))
            view.frame = NSRect(x: 0, y: 0, width: 280, height: 500)
            view.layoutSubtreeIfNeeded()
            let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
            view.cacheDisplay(in: view.bounds, to: bitmap)
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: URL(fileURLWithPath: directory).appendingPathComponent(name + ".png"))
        }
    }
    @MainActor func testLateDirectoryReadCannotReplaceNewerNavigation() async throws {
        let client = PanelClient(try fixture("panel-files"))
        let model = NativePanelModel(client: client, projectID: "p", sessionID: "s")
        let slow = Task { await model.refresh("files", directory: "slow") }
        for _ in 0..<100 { if await client.pending() { break }; await Task.yield() }
        let isPending = await client.pending(); XCTAssertTrue(isPending)
        await model.refresh("files", directory: "results")
        await client.finish(); await slow.value
        XCTAssertEqual(model.path, "results")
        XCTAssertEqual(model.parent, ".")
        XCTAssertEqual(model.child(model.files[1]), "results/README.md")
        model.close()
    }
    @MainActor func testContextsAreSessionScopedAndFailedMutationIsNotReplayed() async throws {
        let client = PanelClient(try fixture("panel-contexts"))
        let model = NativePanelModel(client: client, projectID: "p", sessionID: "s")
        await model.refresh("hosts")
        XCTAssertEqual(model.contexts?.attached.map(\.id), ["local", "ssh:gpu"])
        XCTAssertEqual(model.contexts?.available.map(\.id), ["wsl:ubuntu"])
        await model.setContext("wsl:ubuntu", enabled: true)
        let count = await client.mutationCount()
        XCTAssertEqual(count, 1)
        XCTAssertNotNil(model.error)
        XCTAssertEqual(model.contexts?.enabled_ids, ["ssh:gpu"])
    }
    @MainActor func testReadOnlyContextCannotBeChanged() async throws {
        var rows = try fixture("panel-contexts"); rows["read_only"] = .bool(true)
        let client = PanelClient(rows)
        let model = NativePanelModel(client: client, projectID: "p", sessionID: "s")
        await model.refresh("hosts")
        await model.setContext("ssh:gpu", enabled: false)
        let count = await client.mutationCount(); XCTAssertEqual(count, 0)
    }
    func testTruncatedPreviewRetainsFullSize() throws {
        let preview = try JSONDecoder().decode(NativePanelFileContent.self, from: JSONEncoder().encode(fixture("panel-preview")))
        XCTAssertTrue(preview.truncated)
        XCTAssertEqual(preview.total_bytes, 8_000_000)
        XCTAssertEqual(preview.text, "preview prefix")
    }
}
