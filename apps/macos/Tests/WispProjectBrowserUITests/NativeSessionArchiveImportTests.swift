import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private func archiveImportFixture(_ name: String = "native-session-import/v1/archive") throws -> SettingsValue {
    var root = URL(fileURLWithPath: #filePath); for _ in 0..<5 { root.deleteLastPathComponent() }
    return try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/\(name).json")))
}
private actor SessionArchiveImportFake: NativeConversationQuerying {
    let fixture: SettingsValue
    var mode = "", hold = ""
    var held: CheckedContinuation<Void, Never>?
    var imported: Set<String> = []
    var calls: [(String, String, [String: SettingsValue])] = []
    init(_ fixture: SettingsValue) { self.fixture = fixture }
    func setup(mode: String = "", hold: String = "") { self.mode = mode; self.hold = hold }
    func waiting() -> Bool { held != nil }
    func release() { held?.resume(); held = nil; hold = "" }
    func recorded() -> [(String, String, [String: SettingsValue])] { calls }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { throw ProjectBrowserError.invalidResponse }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        calls.append((projectID, command, args))
        let key = projectID + ":" + args["archive_path"]!.string
        var response = fixture[command == "native_session_archive_preview" ? "preview" : "result"]
        response["project_id"] = .string(projectID)
        if command == "native_session_archive_preview" {
            response["archive_path"] = args["archive_path"]!
            if imported.contains(key) { response["state"] = .string("imported"); response["existing_session_id"] = .string("imported-1") }
        } else { imported.insert(key) }
        if command == hold { await withCheckedContinuation { held = $0 } }
        if command == "native_session_archive_import", mode == "lost" { throw ProjectBrowserError.service("reply lost") }
        if command == "native_session_archive_import", mode == "wrong-owner" { response["project_id"] = .string("other-project") }
        if mode == "malformed" { return .null }
        return response
    }
}

final class NativeSessionArchiveImportTests: XCTestCase {
    private func project(_ id: String, name: String) throws -> ProjectSummary {
        var value = try archiveImportFixture("native-projects/v1/import")["result"]; value["id"] = .string(id); value["name"] = .string(name)
        return try NativeProjectCommand.summary(from: value)
    }
    func testSharedZipImportContractsBindReviewedFileSourceDestinationAndCounts() throws {
        let fixture = try archiveImportFixture(), path = fixture["preview"]["archive_path"].string
        let preview = try NativeSessionArchivePreview.decode(fixture["preview"], project: "research-1", path: path)
        XCTAssertEqual(.object(preview.arguments), fixture["args"])
        XCTAssertEqual(try NativeSessionArchiveImportResult.decode(fixture["result"], reviewed: preview).frame_id, "imported-1")
        for (field, value) in [("project_id", SettingsValue.string("other")), ("archive_path", .string("other.zip")), ("sha256", .string(String(repeating: "z", count: 64))), ("state", .string("unknown")), ("message_count", .integer(0)), ("state", .string("imported"))] {
            var bad = fixture["preview"]; bad[field] = value; XCTAssertThrowsError(try NativeSessionArchivePreview.decode(bad, project: "research-1", path: path))
        }
        var bad = fixture["preview"]; bad["messages"] = .array([.object(["role": .string("system"), "text": .string("Hidden")])])
        XCTAssertThrowsError(try NativeSessionArchivePreview.decode(bad, project: "research-1", path: path))
        for (field, value) in [("project_id", SettingsValue.string("wrong")), ("source_session_id", .string("wrong")), ("frame_id", .string("")), ("status", .string("queued")), ("message_count", .integer(99)), ("artifact_count", .integer(2))] {
            var result = fixture["result"]; result[field] = value; XCTAssertThrowsError(try NativeSessionArchiveImportResult.decode(result, reviewed: preview))
        }
    }
    @MainActor func testPreviewAndOneShotConfirmationUseSelectedProjectAndExactReviewedArguments() async throws {
        let fake = SessionArchiveImportFake(try archiveImportFixture()), model: NativeSessionArchiveImportModel
        var writable = true
        model = NativeSessionArchiveImportModel(client: fake, project: "research-1", projects: ["research-1", "research-2"], writable: { writable })
        model.select(project: "research-1", path: "/tmp/reviewed-session.zip"); await model.readPreview()
        XCTAssertTrue(model.canImport); writable = false; await model.confirm()
        var calls = await fake.recorded(); XCTAssertEqual(calls.count, 1)
        writable = true; model.select(project: "research-2", path: model.path); XCTAssertNil(model.preview); XCTAssertFalse(model.canImport)
        await model.readPreview(); await model.confirm(); await model.confirm()
        calls = await fake.recorded(); XCTAssertEqual(calls.count, 3); XCTAssertEqual(calls[2].0, "research-2")
        XCTAssertEqual(calls[2].1, "native_session_archive_import")
        XCTAssertEqual(calls[2].2, ["archive_path": .string("/tmp/reviewed-session.zip"), "sha256": .string(String(repeating: "a", count: 64)), "source_session_id": .string("source-1")])
        XCTAssertEqual(model.result?.project_id, "research-2"); XCTAssertEqual(model.result?.frame_id, "imported-1"); XCTAssertFalse(model.canImport)
        await model.readPreview(); XCTAssertEqual(model.preview?.existing_session_id, "imported-1"); XCTAssertEqual(model.preview?.state, "imported"); XCTAssertFalse(model.canImport)
    }
    @MainActor func testUnknownImportResultsRetainReviewedSelectionAndNeverReplayEvenAfterRefreshOrSelection() async throws {
        for failure in ["lost", "wrong-owner", "malformed"] {
            let fake = SessionArchiveImportFake(try archiveImportFixture()), model: NativeSessionArchiveImportModel
            model = NativeSessionArchiveImportModel(client: fake, project: "research-1", projects: ["research-1", "research-2"], writable: { true })
            model.select(project: "research-1", path: "/tmp/session.zip"); await model.readPreview(); await fake.setup(mode: failure)
            await model.confirm(); XCTAssertTrue(model.uncertain); XCTAssertNotNil(model.error); XCTAssertNil(model.result)
            XCTAssertEqual(model.path, "/tmp/session.zip"); await model.confirm()
            await fake.setup(); await model.readPreview(); XCTAssertEqual(model.preview?.existing_session_id, "imported-1")
            model.select(project: "research-2", path: "/tmp/another.zip"); await model.readPreview(); await model.confirm()
            XCTAssertTrue(model.uncertain); XCTAssertFalse(model.canImport)
            let calls = await fake.recorded(); XCTAssertEqual(calls.filter { $0.1 == "native_session_archive_import" }.count, 1)
        }
    }
    @MainActor func testSelectionAndClosingDiscardLatePreviewsAndClosedWritesCannotReturnNavigationResults() async throws {
        let fake = SessionArchiveImportFake(try archiveImportFixture()), model: NativeSessionArchiveImportModel
        model = NativeSessionArchiveImportModel(client: fake, project: "research-1", projects: ["research-1", "research-2"], writable: { true })
        model.select(project: "research-1", path: "/tmp/slow.zip"); await fake.setup(hold: "native_session_archive_preview")
        let old = Task { await model.readPreview() }; while !(await fake.waiting()) { await Task.yield() }
        model.select(project: "research-2", path: "/tmp/new.zip"); await fake.setup(); await model.readPreview(); await fake.release(); await old.value
        XCTAssertEqual(model.preview?.project_id, "research-2"); XCTAssertEqual(model.preview?.archive_path, "/tmp/new.zip")
        await fake.setup(hold: "native_session_archive_import"); let write = Task { await model.confirm() }
        while !(await fake.waiting()) { await Task.yield() }
        XCTAssertTrue(model.importing); await model.confirm(); model.select(project: "research-1", path: "/tmp/ignored.zip")
        XCTAssertEqual(model.project, "research-2"); model.close(); await fake.release(); await write.value
        XCTAssertNil(model.result); XCTAssertNil(model.error); XCTAssertFalse(model.canImport); XCTAssertFalse(model.importing)
        let calls = await fake.recorded(); XCTAssertEqual(calls.filter { $0.1 == "native_session_archive_import" }.count, 1)
    }
    @MainActor func testUnavailableMalformedAndCancelledReadsCannotEnableImport() async throws {
        let fake = SessionArchiveImportFake(try archiveImportFixture()), model: NativeSessionArchiveImportModel
        model = NativeSessionArchiveImportModel(client: fake, project: "research-1", projects: ["research-1"], writable: { true })
        model.select(project: "unknown", path: "/tmp/session.zip"); XCTAssertTrue(model.path.isEmpty)
        model.select(project: "research-1", path: "relative.zip"); await model.readPreview(); XCTAssertNil(model.preview)
        model.select(project: "research-1", path: "/tmp/session.zip"); await fake.setup(mode: "malformed"); await model.readPreview()
        XCTAssertNotNil(model.error); XCTAssertNil(model.preview); XCTAssertFalse(model.canImport); XCTAssertFalse(model.uncertain)
        await fake.setup(hold: "native_session_archive_preview"); let read = Task { await model.readPreview() }
        while !(await fake.waiting()) { await Task.yield() }; read.cancel(); await fake.release(); await read.value
        XCTAssertNil(model.preview); XCTAssertNil(model.error); XCTAssertFalse(model.canImport); XCTAssertFalse(model.reading)
    }
    @MainActor func testImmediateEscapeClosesDestinationBeforeSheetAndNeverImportsOrMovesFocus() async throws {
        _ = NSApplication.shared
        let fake = SessionArchiveImportFake(try archiveImportFixture()), model: NativeSessionArchiveImportModel
        model = NativeSessionArchiveImportModel(client: fake, project: "research-1", projects: ["research-1"], writable: { true })
        let projects = [try project("research-1", name: "Research")]
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 419, height: 540), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let root = NSView(frame: window.contentView!.bounds); window.contentView = root
        var parentClosed = 0, pickerClosed = 0, opened = 0
        let parent = NSHostingView(rootView: NativeSessionArchiveImportSheet(model: model, projects: projects, close: { parentClosed += 1 }, open: { _, _ in opened += 1 }))
        parent.frame = root.bounds; root.addSubview(parent); parent.layoutSubtreeIfNeeded()
        let child = NSHostingView(rootView: NativeSessionImportProjectPicker(projects: projects, selected: "research-1", close: { pickerClosed += 1 }, select: { _ in opened += 1 }))
        child.frame = root.bounds; root.addSubview(child); child.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53))
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertEqual(pickerClosed, 1); XCTAssertEqual(parentClosed, 0); XCTAssertEqual(opened, 0); XCTAssertTrue(window.firstResponder === focus)
        child.removeFromSuperview(); XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertEqual(parentClosed, 1)
    }
    @MainActor func testArchiveImportCommandUsesProjectScopeAndADistinctSharedIcon() {
        XCTAssertEqual(NativeSearchCommand.matching(">import session archive", project: true, session: false).map(\.id), ["import-session-archive"])
        XCTAssertTrue(NativeSearchCommand.matching(">import session archive", project: false, session: false).isEmpty)
        XCTAssertEqual(Set(NativeSearchCommand.all.map(\.icon)).count, NativeSearchCommand.all.count)
    }
    @MainActor func testRenderArchivePreviewResultAndProjectPickerAtNarrowWidthInBothLocalesAndSchemes() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Set WISP_NATIVE_SNAPSHOT_DIR to render archive import") }
        let previous = UserDefaults.standard.object(forKey: "nativeSettings.locale")
        defer { if let previous { UserDefaults.standard.set(previous, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        let projects = [try project("research-1", name: "Research Alpha"), try project("research-2", name: "Research Beta")]
        for locale in ["zh", "en"] {
            UserDefaults.standard.set(locale, forKey: "nativeSettings.locale")
            for scheme in [ColorScheme.light, .dark] {
                for surface in ["preview", "result", "picker"] {
                    let fake = SessionArchiveImportFake(try archiveImportFixture()), model: NativeSessionArchiveImportModel
                    model = NativeSessionArchiveImportModel(client: fake, project: "research-1", projects: projects.map(\.id), writable: { true })
                    model.select(project: "research-1", path: "/tmp/exports/reviewed-session.zip"); await model.readPreview()
                    if surface == "result" { await model.confirm() }
                    let view = surface == "picker" ? AnyView(NativeSessionImportProjectPicker(projects: projects, selected: model.project, close: {}, select: { _ in }))
                        : AnyView(NativeSessionArchiveImportSheet(model: model, projects: projects, close: {}, open: { _, _ in }))
                    let host = NSHostingView(rootView: view.frame(maxWidth: .infinity, maxHeight: .infinity).background(WispDesign.color("bg-app", scheme)).foregroundStyle(WispDesign.color("text", scheme)).tint(WispDesign.color("clay", scheme)).environment(\.colorScheme, scheme))
                    host.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua); host.frame = NSRect(x: 0, y: 0, width: 419, height: 600); host.layoutSubtreeIfNeeded()
                    let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds)); host.cacheDisplay(in: host.bounds, to: bitmap)
                    let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:])); XCTAssertGreaterThan(data.count, 1000)
                    try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent("session-import-\(surface)-\(locale)-\(scheme == .dark ? "dark" : "light").png"))
                }
            }
        }
    }
}
