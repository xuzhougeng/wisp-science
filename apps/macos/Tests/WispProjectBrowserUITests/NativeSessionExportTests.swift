import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private func exportFixture(_ name: String = "native-session-export/v1/export") throws -> SettingsValue {
    var root = URL(fileURLWithPath: #filePath); for _ in 0..<5 { root.deleteLastPathComponent() }
    return try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/\(name).json")))
}
private actor SessionExportFake: NativeConversationQuerying {
    let fixture: SettingsValue
    var failure = "", hold = ""
    var held: CheckedContinuation<Void, Never>?
    var calls: [(String, String, [String: SettingsValue])] = []
    init(_ fixture: SettingsValue) { self.fixture = fixture }
    func setup(failure: String = "", hold: String = "") { self.failure = failure; self.hold = hold }
    func waiting() -> Bool { held != nil }
    func release() { held?.resume(); held = nil; hold = "" }
    func recorded() -> [(String, String, [String: SettingsValue])] { calls }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { throw ProjectBrowserError.invalidResponse }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        calls.append((projectID, command, args))
        var value = fixture[command == "native_conversation_export_preview" ? "preview" : "result"]
        value["project_id"] = .string(projectID); value["session_id"] = args["session_id"]!; value["include_artifacts"] = args["include_artifacts"]!
        if command == "native_conversation_export_preview", args["include_artifacts"] == .bool(false) {
            value["artifacts"] = .array([]); value["missing_artifacts"] = .array([]); value["artifact_bytes"] = .integer(0)
        }
        if command == "native_conversation_export" { value["destination_path"] = args["destination_path"]! }
        if command == hold { await withCheckedContinuation { held = $0 } }
        if failure == "malformed" { return .null }
        if command == "native_conversation_export" {
            switch failure {
            case "lost": throw ProjectBrowserError.service("reply lost")
            case "wrong-owner": value["project_id"] = .string("wrong")
            case "wrong-revision": value["revision"] = .string(String(repeating: "c", count: 64))
            case "wrong-path": value["destination_path"] = .string("/tmp/other.zip")
            case "wrong-choice": value["include_artifacts"] = .bool(!args["include_artifacts"]!.bool)
            default: break
            }
        }
        return value
    }
}

final class NativeSessionExportTests: XCTestCase {
    private var source: BrowserSession { .init(id: "source-1", projectID: "research-1", title: "Earlier sidebar title", ts: 1, status: "archived") }
    private var destination: URL { URL(fileURLWithPath: "/tmp/wisp-session-source-1.zip") }
    @MainActor private func model(_ fake: SessionExportFake, writable: @escaping () -> Bool = { true }) -> NativeSessionExportModel {
        NativeSessionExportModel(client: fake, source: source, writable: writable)
    }
    func testSharedContractsBindReviewedSourceContentChoiceAndDestination() throws {
        let fixture = try exportFixture()
        let preview = try NativeSessionExportPreview.decode(fixture["preview"], project: source.projectID, session: source.id, includeArtifacts: true)
        XCTAssertEqual(.object(preview.arguments(destination: destination)), fixture["args"])
        XCTAssertEqual(try NativeSessionExportResult.decode(fixture["result"], reviewed: preview, destination: destination).bytes, 640)
        for (field, value) in [("schema", SettingsValue.string("old")), ("project_id", .string("wrong")), ("session_id", .string("wrong")), ("revision", .string("bad")), ("message_count", .integer(0)), ("head_epoch", .integer(-1)), ("tool_call_count", .integer(-1)), ("include_artifacts", .bool(false)), ("artifact_bytes", .integer(13)), ("default_filename", .string("../escape.zip"))] {
            var bad = fixture["preview"]; bad[field] = value
            XCTAssertThrowsError(try NativeSessionExportPreview.decode(bad, project: source.projectID, session: source.id, includeArtifacts: true))
        }
        var duplicate = fixture["preview"]; duplicate["artifacts"] = .array(duplicate["artifacts"].array + duplicate["artifacts"].array); duplicate["artifact_bytes"] = .integer(24)
        XCTAssertThrowsError(try NativeSessionExportPreview.decode(duplicate, project: source.projectID, session: source.id, includeArtifacts: true))
        for (field, value) in [("checksum", SettingsValue.string("bad")), ("bytes", .integer(0)), ("destination_path", .string("/tmp/wrong.zip")), ("revision", .string(String(repeating: "c", count: 64))), ("include_artifacts", .bool(false)), ("session_id", .string("wrong"))] {
            var bad = fixture["result"]; bad[field] = value
            XCTAssertThrowsError(try NativeSessionExportResult.decode(bad, reviewed: preview, destination: destination))
        }
    }
    @MainActor func testScopedIdleArchivedConversationCanExportButBusyQueuedAndForeignCannot() throws {
        var value = try exportFixture("native-conversations/v1/snapshot")
        value["project_id"] = .string(source.projectID); value["session_id"] = .string(source.id)
        value["running"] = .bool(false); value["stopping"] = .bool(false); value["read_only"] = .bool(true); value["approvals"] = .array([]); value["history_state"] = .null
        let snapshot = try ConversationSnapshot.decode(value, projectID: source.projectID, sessionID: source.id)
        XCTAssertTrue(NativeSessionExportModel.eligible(source: source, snapshot: snapshot, busy: false, queued: false))
        XCTAssertFalse(NativeSessionExportModel.eligible(source: source, snapshot: snapshot, busy: true, queued: false))
        XCTAssertFalse(NativeSessionExportModel.eligible(source: source, snapshot: snapshot, busy: false, queued: true))
        XCTAssertFalse(NativeSessionExportModel.eligible(source: source, snapshot: nil, busy: false, queued: false))
        let foreign = BrowserSession(id: source.id, projectID: "foreign", title: "Other", ts: 1, status: "complete")
        XCTAssertFalse(NativeSessionExportModel.eligible(source: foreign, snapshot: snapshot, busy: false, queued: false))
        for field in ["running", "stopping"] {
            var busy = value; busy[field] = .bool(true)
            XCTAssertFalse(NativeSessionExportModel.eligible(source: source, snapshot: try ConversationSnapshot.decode(busy, projectID: source.projectID, sessionID: source.id), busy: false, queued: false))
        }
    }
    @MainActor func testOneShotSaveUsesReviewedPreviewAndSelectedDestination() async throws {
        let fake = SessionExportFake(try exportFixture()); var writable = true; let model = model(fake, writable: { writable })
        await model.readPreview(); XCTAssertTrue(model.canExport); XCTAssertEqual(model.preview?.title, "RNA-seq comparison")
        writable = false; await model.export(to: destination); writable = true
        await model.export(to: URL(fileURLWithPath: "/tmp/not-zip.txt")); await model.export(to: destination); await model.export(to: destination)
        XCTAssertEqual(model.result?.destination_path, destination.path); XCTAssertFalse(model.canExport); XCTAssertFalse(model.uncertain)
        let calls = await fake.recorded(); XCTAssertEqual(calls.count, 2); XCTAssertEqual(calls[1].0, source.projectID)
        XCTAssertEqual(.object(calls[1].2), try exportFixture()["args"])
    }
    @MainActor func testFileChoiceRequiresAnotherPreviewAndCannotReuseIncludedFiles() async throws {
        let fake = SessionExportFake(try exportFixture()), model = model(fake)
        await model.readPreview(); model.chooseArtifacts(false)
        XCTAssertNil(model.preview); XCTAssertFalse(model.canExport); await model.export(to: destination)
        await model.readPreview(); XCTAssertTrue(model.canExport); XCTAssertTrue(model.preview!.artifacts.isEmpty)
        await model.export(to: destination)
        let calls = await fake.recorded(); XCTAssertEqual(calls.count, 3); XCTAssertEqual(calls[2].2["include_artifacts"], .bool(false))
    }
    @MainActor func testCancellingSavePanelPreservesPreviewAndDispatchesNoWrite() async throws {
        let fake = SessionExportFake(try exportFixture()), model = model(fake)
        await model.readPreview(); let reviewed = model.preview
        model.selectDestination { XCTAssertTrue(model.choosingDestination); XCTAssertFalse(model.canExport); return nil }
        XCTAssertEqual(model.preview, reviewed); XCTAssertTrue(model.canExport); XCTAssertFalse(model.choosingDestination)
        let calls = await fake.recorded(); XCTAssertEqual(calls.count, 1)
    }
    @MainActor func testUnknownOrInvalidWriteCannotReplayAfterRefreshOrFileChoiceChange() async throws {
        for failure in ["lost", "malformed", "wrong-owner", "wrong-revision", "wrong-path", "wrong-choice"] {
            let fake = SessionExportFake(try exportFixture()), model = model(fake)
            await model.readPreview(); await fake.setup(failure: failure); await model.export(to: destination)
            XCTAssertTrue(model.uncertain); XCTAssertNotNil(model.error); XCTAssertNil(model.result)
            XCTAssertEqual(model.savePath, destination.path)
            await fake.setup(); await model.readPreview(); model.chooseArtifacts(false); await model.readPreview(); await model.export(to: destination)
            XCTAssertTrue(model.uncertain); XCTAssertFalse(model.canExport)
            let calls = await fake.recorded(); XCTAssertEqual(calls.filter { $0.1 == "native_conversation_export" }.count, 1)
        }
    }
    @MainActor func testCancellationAfterDispatchBlocksReplayAndCancellationBeforeDispatchWritesNothing() async throws {
        let fake = SessionExportFake(try exportFixture()), model = model(fake)
        await model.readPreview()
        let cancelled = Task { await model.export(to: destination) }; cancelled.cancel(); await cancelled.value
        XCTAssertFalse(model.uncertain); XCTAssertTrue(model.canExport)
        await fake.setup(hold: "native_conversation_export"); let write = Task { await model.export(to: destination) }
        while !(await fake.waiting()) { await Task.yield() }; XCTAssertTrue(model.writing)
        await model.export(to: destination); write.cancel(); await fake.release(); await write.value
        XCTAssertTrue(model.uncertain); XCTAssertNil(model.result); XCTAssertFalse(model.writing)
        await model.readPreview(); await model.export(to: destination)
        let calls = await fake.recorded(); XCTAssertEqual(calls.filter { $0.1 == "native_conversation_export" }.count, 1)
    }
    @MainActor func testCancelledSupersededAndClosedReadsOrWritesCannotUpdateAnotherView() async throws {
        let fake = SessionExportFake(try exportFixture()), model = model(fake)
        await fake.setup(hold: "native_conversation_export_preview"); let old = Task { await model.readPreview() }
        while !(await fake.waiting()) { await Task.yield() }; await fake.setup(); await model.readPreview()
        await fake.release(); await old.value; XCTAssertNotNil(model.preview)
        await fake.setup(hold: "native_conversation_export_preview"); let read = Task { await model.readPreview() }
        while !(await fake.waiting()) { await Task.yield() }; read.cancel(); await fake.release(); await read.value
        XCTAssertNil(model.preview); XCTAssertNil(model.error); XCTAssertFalse(model.reading)
        await model.readPreview(); await fake.setup(hold: "native_conversation_export"); let write = Task { await model.export(to: destination) }
        while !(await fake.waiting()) { await Task.yield() }; model.close(); await fake.release(); await write.value
        XCTAssertNil(model.result); XCTAssertNil(model.error); XCTAssertFalse(model.canExport)
        let unavailable = self.model(fake, writable: { false }); await unavailable.readPreview(); await unavailable.export(to: destination)
        XCTAssertNil(unavailable.preview)
    }
    @MainActor func testMalformedPreviewCannotEnableExportAndCommandRequiresDistinctSessionScope() async throws {
        let fake = SessionExportFake(try exportFixture()), model = model(fake)
        await fake.setup(failure: "malformed"); await model.readPreview()
        XCTAssertFalse(model.canExport); XCTAssertNotNil(model.error); XCTAssertFalse(model.uncertain)
        XCTAssertEqual(NativeSearchCommand.matching(">export session zip", project: true, session: true).map(\.id), ["export-session"])
        XCTAssertTrue(NativeSearchCommand.matching(">export session zip", project: true, session: false).isEmpty)
        XCTAssertEqual(Set(NativeSearchCommand.all.map(\.icon)).count, NativeSearchCommand.all.count)
    }
    @MainActor func testImmediateEscapeClosesExportBeforeParentWithoutFocusOrWrite() async throws {
        _ = NSApplication.shared
        let fake = SessionExportFake(try exportFixture()), model = model(fake); await model.readPreview()
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 419, height: 600), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let root = NSView(frame: window.contentView!.bounds); window.contentView = root
        var parentClosed = 0, exportClosed = 0
        let parent = NSHostingView(rootView: Text("Parent").background(NativeSettingsEscape { parentClosed += 1 }))
        parent.frame = root.bounds; root.addSubview(parent); parent.layoutSubtreeIfNeeded()
        let child = NSHostingView(rootView: NativeSessionExportSheet(model: model, close: { exportClosed += 1 }))
        child.frame = root.bounds; root.addSubview(child); child.layoutSubtreeIfNeeded(); let focus = window.firstResponder
        let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53))
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertEqual(exportClosed, 1); XCTAssertEqual(parentClosed, 0); XCTAssertTrue(window.firstResponder === focus)
        child.removeFromSuperview(); XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertEqual(parentClosed, 1)
        let calls = await fake.recorded(); XCTAssertEqual(calls.count, 1)
    }
    @MainActor func testRenderExportPreviewResultAndUncertaintyInBothLocalesAndSchemes() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Set WISP_NATIVE_SNAPSHOT_DIR to render export") }
        let previous = UserDefaults.standard.object(forKey: "nativeSettings.locale")
        defer { if let previous { UserDefaults.standard.set(previous, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        for locale in ["zh", "en"] {
            UserDefaults.standard.set(locale, forKey: "nativeSettings.locale")
            for scheme in [ColorScheme.light, .dark] {
                for surface in ["preview", "without-files", "result", "uncertain"] {
                    let fake = SessionExportFake(try exportFixture()), model = model(fake)
                    if surface == "without-files" { model.chooseArtifacts(false) }; await model.readPreview()
                    if surface == "result" { await model.export(to: destination) }
                    if surface == "uncertain" { await fake.setup(failure: "lost"); await model.export(to: destination); await fake.setup(); await model.readPreview() }
                    let host = NSHostingView(rootView: NativeSessionExportSheet(model: model, close: {}).frame(maxWidth: .infinity, maxHeight: .infinity).background(WispDesign.color("bg-app", scheme)).foregroundStyle(WispDesign.color("text", scheme)).tint(WispDesign.color("clay", scheme)).environment(\.colorScheme, scheme))
                    host.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua); host.frame = NSRect(x: 0, y: 0, width: 419, height: 600); host.layoutSubtreeIfNeeded()
                    let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds)); host.cacheDisplay(in: host.bounds, to: bitmap)
                    let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:])); XCTAssertGreaterThan(data.count, 1000)
                    try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent("session-export-\(surface)-\(locale)-\(scheme == .dark ? "dark" : "light").png"))
                }
            }
        }
    }
}
