import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private func externalImportFixture(_ name: String = "native-session-import/v1/external") throws -> SettingsValue {
    var root = URL(fileURLWithPath: #filePath); for _ in 0..<5 { root.deleteLastPathComponent() }
    return try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/\(name).json")))
}
private actor ExternalSessionImportFake: NativeConversationQuerying {
    let fixture: SettingsValue
    var rows: [SettingsValue]
    var hold = "", lostPath = "", failedPreview = "", malformed = ""
    var held: CheckedContinuation<Void, Never>?
    var calls: [(String, String, [String: SettingsValue])] = []
    init(_ fixture: SettingsValue, count: Int = 32) {
        self.fixture = fixture
        rows = (0..<count).map { index in
            var row = fixture["list"]["items"].array[0]
            row["path"] = .string("/synthetic/session-\(index).jsonl"); row["session_id"] = .string("source-\(index)")
            row["title"] = .string("Experiment \(index)"); row["cwd"] = .string("/work/study-\(index)")
            row["state"] = .string(index == 1 ? "updatable" : index == 2 ? "imported" : "new")
            return row
        }
    }
    func setup(hold: String = "", lostPath: String = "", failedPreview: String = "", malformed: String = "") {
        self.hold = hold; self.lostPath = lostPath; self.failedPreview = failedPreview; self.malformed = malformed
    }
    func waiting() -> Bool { held != nil }
    func release() { held?.resume(); held = nil; hold = "" }
    func recorded() -> [(String, String, [String: SettingsValue])] { calls }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { throw ProjectBrowserError.invalidResponse }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        calls.append((projectID, command, args))
        let keys = ["native_external_session_sources": "sources", "native_external_session_list": "list", "native_external_session_preview": "preview", "native_external_session_import": "result"]
        guard let key = keys[command] else { throw ProjectBrowserError.invalidResponse }
        var response = fixture[key]; response["project_id"] = .string(projectID)
        if key != "sources" { response["provider"] = args["provider"]!; response["context_id"] = args["context_id"]! }
        if key == "list" { response["items"] = .array(rows) }
        if ["preview", "result"].contains(key) {
            let path = args["path"]!.string
            guard let index = rows.firstIndex(where: { $0["path"] == .string(path) }) else { throw ProjectBrowserError.invalidResponse }
            let row = rows[index], existing = row["state"] != .string("new")
            response["path"] = .string(path); response["source_session_id"] = row["session_id"]
            if key == "preview" {
                response["existing_session_id"] = existing ? .string("frame-\(row["session_id"].string)") : .null
                if path == failedPreview { throw ProjectBrowserError.service("source read failed") }
            } else {
                response["frame_id"] = .string("frame-\(row["session_id"].string)")
                response["status"] = row["state"] == .string("updatable") ? .string("updated") : row["state"] == .string("imported") ? .string("skipped") : .string("imported")
                rows[index]["state"] = .string("imported")
            }
        }
        if command == hold { await withCheckedContinuation { held = $0 } }
        if key == "result", args["path"]?.string == lostPath { throw ProjectBrowserError.service("reply lost") }
        if command == malformed { return .null }
        return response
    }
}

final class NativeExternalSessionImportTests: XCTestCase {
    @MainActor private func ready(_ count: Int = 32, writable: @escaping () -> Bool = { true }) async throws -> (ExternalSessionImportFake, NativeExternalSessionImportModel) {
        let fake = ExternalSessionImportFake(try externalImportFixture(), count: count)
        let model = NativeExternalSessionImportModel(client: fake, project: "research-1", projects: ["research-1", "research-2"], writable: writable)
        await model.initialize(); return (fake, model)
    }
    private func projects() throws -> [ProjectSummary] {
        try ["research-1", "research-2"].map { id in
            var value = try externalImportFixture("native-projects/v1/import")["result"]; value["id"] = .string(id); value["name"] = .string("Research " + id)
            return try NativeProjectCommand.summary(from: value)
        }
    }
    func testSharedExternalContractsBindProviderContextPathSourceProjectAndReviewedHash() throws {
        let fixture = try externalImportFixture()
        XCTAssertEqual(try NativeExternalImportSources.decode(fixture["sources"], project: "research-1").sources.map(\.id), ["local", "wsl:Ubuntu", "ssh:analysis"])
        let list = try NativeExternalImportList.decode(fixture["list"], project: "research-1", provider: "codex", context: "wsl:Ubuntu")
        let preview = try NativeExternalImportPreview.decode(fixture["preview"], project: "research-1", provider: "codex", context: "wsl:Ubuntu", item: list.items[0])
        XCTAssertEqual(.object(preview.arguments), fixture["args"])
        XCTAssertEqual(try NativeExternalImportResult.decode(fixture["result"], reviewed: preview).frame_id, "imported-1")
        for field in ["project_id", "provider", "context_id", "path", "source_session_id", "sha256"] {
            var bad = fixture["preview"]; bad[field] = .string("wrong")
            XCTAssertThrowsError(try NativeExternalImportPreview.decode(bad, project: "research-1", provider: "codex", context: "wsl:Ubuntu", item: list.items[0]))
        }
        for field in ["project_id", "provider", "context_id", "path", "source_session_id", "status"] {
            var bad = fixture["result"]; bad[field] = .string("wrong"); XCTAssertThrowsError(try NativeExternalImportResult.decode(bad, reviewed: preview))
        }
        var bad = fixture["sources"]; bad["sources"] = .array([fixture["sources"]["sources"].array[0], fixture["sources"]["sources"].array[0]])
        XCTAssertThrowsError(try NativeExternalImportSources.decode(bad, project: "research-1"))
        bad = fixture["list"]; bad["items"] = .array([fixture["list"]["items"].array[0], fixture["list"]["items"].array[0]])
        XCTAssertThrowsError(try NativeExternalImportList.decode(bad, project: "research-1", provider: "codex", context: "wsl:Ubuntu"))
    }
    @MainActor func testSourceChoicesQueriesPagesAndSingleImportUseExactDestinationWithoutDuplicateWrites() async throws {
        var writable = true
        let (fake, model) = try await ready(writable: { writable })
        XCTAssertEqual(model.items.count, 32); XCTAssertEqual(model.pageItems.count, 25); XCTAssertEqual(model.pageCount, 2)
        model.setPage(99); XCTAssertEqual(model.page, 1); XCTAssertEqual(model.pageItems.count, 7)
        model.setQuery("STUDY-31"); XCTAssertEqual(model.filtered.map(\.session_id), ["source-31"]); XCTAssertEqual(model.page, 0)
        model.select(project: "unknown", provider: "claude", context: "ssh:missing"); XCTAssertEqual(model.provider, "codex")
        model.select(project: "research-2", provider: "claude", context: "wsl:Ubuntu"); await model.load(refresh: true)
        XCTAssertEqual(model.project, "research-2"); XCTAssertEqual(model.provider, "claude"); XCTAssertEqual(model.context, "wsl:Ubuntu")
        await model.inspect(try XCTUnwrap(model.filtered.first)); XCTAssertTrue(model.canImport)
        writable = false; await model.importPreview(); writable = true; await model.importPreview(); await model.importPreview()
        let calls = await fake.recorded(), writes = calls.filter { $0.1 == "native_external_session_import" }
        XCTAssertEqual(writes.count, 1); XCTAssertEqual(writes[0].0, "research-2")
        XCTAssertEqual(writes[0].2, ["provider": .string("claude"), "context_id": .string("wsl:Ubuntu"), "path": .string("/synthetic/session-31.jsonl"), "source_session_id": .string("source-31"), "sha256": .string(String(repeating: "b", count: 64))])
        XCTAssertEqual(model.results["/synthetic/session-31.jsonl"]?.frame_id, "frame-source-31")
        XCTAssertTrue(calls.contains { $0.1 == "native_external_session_list" && $0.2["refresh"] == .bool(true) })
    }
    @MainActor func testFilteredBatchProcessesAllPagesWithFreshPreviewsAndKeepsIndependentReadFailuresSeparate() async throws {
        let (fake, model) = try await ready()
        await fake.setup(failedPreview: "/synthetic/session-0.jsonl"); await model.importFiltered()
        XCTAssertEqual(model.total, 31); XCTAssertEqual(model.done, 31); XCTAssertEqual(model.failed, 1); XCTAssertEqual(model.imported, 29); XCTAssertEqual(model.updated, 1)
        XCTAssertFalse(model.uncertain); XCTAssertEqual(model.results.count, 30); XCTAssertNotNil(model.itemErrors["/synthetic/session-0.jsonl"])
        let calls = await fake.recorded(), writes = calls.filter { $0.1 == "native_external_session_import" }
        XCTAssertEqual(writes.count, 30); XCTAssertTrue(writes.contains { $0.2["path"] == .string("/synthetic/session-31.jsonl") })
        XCTAssertFalse(writes.contains { $0.2["path"] == .string("/synthetic/session-2.jsonl") })
        for (index, call) in calls.enumerated() where call.1 == "native_external_session_import" {
            XCTAssertGreaterThan(index, 0); XCTAssertEqual(calls[index - 1].1, "native_external_session_preview"); XCTAssertEqual(calls[index - 1].2["path"], call.2["path"])
        }
    }
    @MainActor func testLostOrMalformedWritesStopTheBatchAndRemainBlockedAfterReloadAndSourceChange() async throws {
        for failure in ["lost", "malformed"] {
            let (fake, model) = try await ready()
            await fake.setup(lostPath: failure == "lost" ? "/synthetic/session-0.jsonl" : "", malformed: failure == "malformed" ? "native_external_session_import" : "")
            await model.importFiltered(); XCTAssertEqual(model.done, 1); XCTAssertEqual(model.failed, 1); XCTAssertTrue(model.uncertain); XCTAssertNotNil(model.error)
            await model.importFiltered(); await fake.setup(); await model.load(refresh: true)
            model.select(project: "research-2", provider: "claude", context: "ssh:analysis"); await model.load(refresh: false); await model.importFiltered()
            XCTAssertTrue(model.uncertain); XCTAssertFalse(model.canImportFiltered)
            let calls = await fake.recorded(); XCTAssertEqual(calls.filter { $0.1 == "native_external_session_import" }.count, 1)
        }
    }
    @MainActor func testStopFinishesOnlyCurrentWriteAndClosingPreventsLateResultAndFurtherBatchWork() async throws {
        for close in [false, true] {
            let (fake, model) = try await ready()
            await fake.setup(hold: "native_external_session_import"); let batch = Task { await model.importFiltered() }
            while !(await fake.waiting()) { await Task.yield() }
            await model.importFiltered(); model.select(project: "research-2", provider: "claude", context: "ssh:analysis")
            XCTAssertEqual(model.project, "research-1")
            if close { model.close() } else { model.stopAfterCurrent() }
            await fake.release(); await batch.value
            XCTAssertFalse(model.importing)
            XCTAssertEqual(model.results.count, close ? 0 : 1); XCTAssertEqual(model.done, close ? 0 : 1)
            let calls = await fake.recorded(); XCTAssertEqual(calls.filter { $0.1 == "native_external_session_import" }.count, 1)
        }
    }
    @MainActor func testClosedSourcesChangedListsAndNewQueriesDiscardLateReadResponses() async throws {
        let fake = ExternalSessionImportFake(try externalImportFixture()), model: NativeExternalSessionImportModel
        model = NativeExternalSessionImportModel(client: fake, project: "research-1", projects: ["research-1", "research-2"], writable: { true })
        await fake.setup(hold: "native_external_session_sources"); let initial = Task { await model.initialize() }
        while !(await fake.waiting()) { await Task.yield() }; model.close(); await fake.release(); await initial.value
        XCTAssertTrue(model.sources.isEmpty); let initialCalls = await fake.recorded(); XCTAssertEqual(initialCalls.count, 1)
        let (listed, current) = try await ready()
        await listed.setup(hold: "native_external_session_list"); let oldList = Task { await current.load(refresh: false) }
        while !(await listed.waiting()) { await Task.yield() }
        current.select(project: "research-2", provider: "claude", context: "ssh:analysis"); await listed.setup(); await current.load(refresh: false)
        await listed.release(); await oldList.value; XCTAssertEqual(current.project, "research-2"); XCTAssertEqual(current.items.count, 32)
        await listed.setup(hold: "native_external_session_preview"); let read = Task { await current.inspect(current.items[0]) }
        while !(await listed.waiting()) { await Task.yield() }; current.setQuery("31"); await listed.release(); await read.value
        XCTAssertNil(current.preview); XCTAssertNil(current.selectedPath); XCTAssertFalse(current.previewing); XCTAssertEqual(current.filtered.count, 1)
    }
    @MainActor func testStoppingOrCancellingDuringFreshPreviewStartsNoWrite() async throws {
        for cancel in [false, true] {
            let (fake, model) = try await ready()
            await fake.setup(hold: "native_external_session_preview"); let batch = Task { await model.importFiltered() }
            while !(await fake.waiting()) { await Task.yield() }
            if cancel { batch.cancel() } else { model.stopAfterCurrent() }
            await fake.release(); await batch.value
            XCTAssertFalse(model.importing); XCTAssertFalse(model.uncertain); XCTAssertEqual(model.done, 0); XCTAssertEqual(model.failed, 0)
            let calls = await fake.recorded(); XCTAssertFalse(calls.contains { $0.1 == "native_external_session_import" })
        }
    }
    @MainActor func testImmediateEscapeClosesOnlySourceChoiceOrPreviewBeforeParentWithoutMovingFocus() async throws {
        _ = NSApplication.shared
        let (_, model) = try await ready(1); await model.inspect(model.items[0])
        for surface in ["source", "preview"] {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 419, height: 600), styleMask: [.titled], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false; defer { window.close() }
            let container = NSView(frame: window.contentView!.bounds); window.contentView = container
            var parentClosed = 0, childClosed = 0, selected = 0
            let parent = NativeSettingsEscape.Coordinator(enabled: true) { parentClosed += 1 }; parent.view = container; parent.install(); defer { parent.remove() }
            let view = surface == "source" ? AnyView(NativeExternalImportChoicePicker(title: "来源环境", choices: [("local", "Local")], selected: "local", close: { childClosed += 1 }, select: { _ in selected += 1 }))
                : AnyView(NativeExternalImportPreviewSheet(model: model, open: { _, _ in selected += 1 }))
            let child = NSHostingView(rootView: view); child.frame = container.bounds; container.addSubview(child); child.layoutSubtreeIfNeeded()
            let focus = window.firstResponder
            let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53))
            XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertEqual(parentClosed, 0); XCTAssertEqual(selected, 0); XCTAssertTrue(window.firstResponder === focus)
            if surface == "source" { XCTAssertEqual(childClosed, 1) } else { XCTAssertNil(model.preview) }
            child.removeFromSuperview(); XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertEqual(parentClosed, 1)
        }
    }
    @MainActor func testExternalImportPaletteRequiresProjectAndUsesDistinctSharedIcon() {
        XCTAssertEqual(NativeSearchCommand.matching(">import codex", project: true, session: false).map(\.id), ["import-external-session"])
        XCTAssertTrue(NativeSearchCommand.matching(">import codex", project: false, session: true).isEmpty)
        XCTAssertEqual(Set(NativeSearchCommand.all.map(\.icon)).count, NativeSearchCommand.all.count)
    }
    @MainActor func testRenderExternalListPreviewResultsAndSourceChoicesInBothLocalesAndSchemes() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Set WISP_NATIVE_SNAPSHOT_DIR to render external import") }
        let previous = UserDefaults.standard.object(forKey: "nativeSettings.locale")
        defer { if let previous { UserDefaults.standard.set(previous, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        for locale in ["zh", "en"] {
            UserDefaults.standard.set(locale, forKey: "nativeSettings.locale")
            for scheme in [ColorScheme.light, .dark] {
                for surface in ["list", "preview", "result", "sources"] {
                    let (_, model) = try await ready(4)
                    if surface == "preview" { await model.inspect(model.items[0]) }
                    if surface == "result" { await model.importFiltered() }
                    let view: AnyView
                    if surface == "sources" { view = AnyView(NativeExternalImportChoicePicker(title: "来源环境", choices: model.sources.map { ($0.id, $0.label) }, selected: "local", close: {}, select: { _ in })) }
                    else if surface == "preview" { view = AnyView(NativeExternalImportPreviewSheet(model: model, open: { _, _ in })) }
                    else { view = AnyView(NativeExternalSessionImportSheet(model: model, projects: try projects(), close: {}, open: { _, _ in })) }
                    let host = NSHostingView(rootView: view.frame(maxWidth: .infinity, maxHeight: .infinity).background(WispDesign.color("bg-app", scheme)).foregroundStyle(WispDesign.color("text", scheme)).tint(WispDesign.color("clay", scheme)).environment(\.colorScheme, scheme))
                    host.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua); host.frame = NSRect(x: 0, y: 0, width: 419, height: 680); host.layoutSubtreeIfNeeded()
                    let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds)); host.cacheDisplay(in: host.bounds, to: bitmap)
                    let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:])); XCTAssertGreaterThan(data.count, 1000)
                    try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent("external-import-\(surface)-\(locale)-\(scheme == .dark ? "dark" : "light").png"))
                }
            }
        }
    }
}
