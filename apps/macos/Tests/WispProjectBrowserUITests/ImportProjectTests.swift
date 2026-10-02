import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor ImportTransport: NativeSettingsQuerying {
    var calls: [(command: String, args: [String: SettingsValue], projectID: String?)] = []
    var mode = "success"
    private var release: CheckedContinuation<SettingsValue, Error>?
    func setMode(_ mode: String) { self.mode = mode }
    func callCount() -> Int { calls.count }
    func lastCall() -> (command: String, args: [String: SettingsValue], projectID: String?) { calls.last! }
    func suspend() { mode = "hang" }
    func isHanging() -> Bool { release != nil }
    func resume(with value: SettingsValue) {
        release?.resume(returning: value)
        release = nil
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        calls.append((command, args, projectID))
        switch mode {
        case "invalid":
            throw ProjectBrowserError.service("not a valid project archive: invalid zip")
        case "present":
            throw ProjectBrowserError.service("This project is already present on this device.")
        case "folder-invalid":
            throw ProjectBrowserError.service("project_folder_metadata_invalid: metadata mismatch")
        case "waiting":
            throw ProjectBrowserError.service("project_folder_waiting: missing snapshot")
        case "conflict":
            throw ProjectBrowserError.service("Sync conflict: multiple heads")
        case "lost":
            throw ProjectBrowserError.service("connection reset")
        case "hang":
            return try await withCheckedThrowingContinuation { release = $0 }
        default:
            var root = URL(fileURLWithPath: #filePath)
            for _ in 0..<5 { root.deleteLastPathComponent() }
            if command == NativeProjectCommand.recoveryPreview || command == NativeProjectCommand.recoverWorkspace {
                let fixture = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-projects/v1/recovery.json")))
                return fixture[command == NativeProjectCommand.recoveryPreview ? "preview_result" : "recover_result"]
            }
            let fixture = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-projects/v1/import.json")))
            return fixture["result"]
        }
    }
}

final class ImportProjectTests: XCTestCase {
    @MainActor func testRefreshClearsDismissedImportErrorOnlyAfterSuccessfulReload() async {
        let transport = ImportTransport(); await transport.setMode("present")
        let list = EmptyImportList()
        let model = ProjectBrowserModel(client: list, databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: transport)
        await model.refresh()
        model.importOptionsPresented = true
        await model.importChosenDirectory(URL(fileURLWithPath: "/tmp/duplicate"))
        XCTAssertNotNil(model.importError); XCTAssertFalse(model.listRefreshFailed)
        await model.refresh()
        XCTAssertNotNil(model.importError, "An open import sheet keeps its error")
        model.importOptionsPresented = false
        await list.setFailure(true); await model.refresh()
        XCTAssertNotNil(model.importError); XCTAssertTrue(model.listRefreshFailed)
        await list.setFailure(false); await model.refresh()
        XCTAssertNil(model.importError); XCTAssertNil(model.error); XCTAssertFalse(model.listRefreshFailed)
        let count = await transport.callCount(); XCTAssertEqual(count, 1, "Reload must not retry the import")
    }
    private func recoveryContract() throws -> SettingsValue {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        return try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-projects/v1/recovery.json")))
    }

    @MainActor func testRecoveryPreviewIsReadOnlyAndConfirmationUsesSharedContractOnce() async throws {
        let contract = try recoveryContract()
        let transport = ImportTransport()
        let model = ProjectBrowserModel(client: EmptyImportList(), databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: transport)
        model.importOptionsPresented = true
        await model.previewWorkspaceRecovery(nil)
        let canceled = await transport.callCount(); XCTAssertEqual(canceled, 0)
        await model.previewWorkspaceRecovery(URL(fileURLWithPath: contract["preview_request"]["args"]["workspace_dir"].string))
        let previewCall = await transport.lastCall()
        XCTAssertEqual(previewCall.command, contract["preview_request"]["command"].string)
        XCTAssertEqual(SettingsValue.object(previewCall.args), contract["preview_request"]["args"])
        XCTAssertNil(previewCall.projectID)
        XCTAssertNil(model.activeProjectID)
        XCTAssertTrue(model.importOptionsPresented)
        XCTAssertEqual(model.recoveryPreview?.recoverable_session_count, 1)
        XCTAssertEqual(model.recoveryPreview?.message_count, 4)
        XCTAssertEqual(model.recoveryPreview?.invalid_archive_count, 1)
        XCTAssertEqual(model.recoveryPreview?.duplicate_archive_count, 1)
        XCTAssertEqual(model.recoveryName, "Old study")
        model.recoveryName = "  "
        await model.recoverWorkspace()
        let blank = await transport.callCount(); XCTAssertEqual(blank, 1)
        model.recoveryName = contract["recover_request"]["args"]["name"].string
        await model.recoverWorkspace()
        let recoverCall = await transport.lastCall()
        XCTAssertEqual(recoverCall.command, contract["recover_request"]["command"].string)
        XCTAssertEqual(SettingsValue.object(recoverCall.args), contract["recover_request"]["args"])
        XCTAssertNil(recoverCall.projectID)
        XCTAssertEqual(model.activeProjectID, contract["recover_result"]["project_id"].string)
        XCTAssertNil(model.recoveryPreview)
        XCTAssertFalse(model.importOptionsPresented)
        await model.recoverWorkspace()
        let calls = await transport.callCount(); XCTAssertEqual(calls, 2)
    }

    @MainActor func testRecoveryLostReplyPreservesPreviewWithoutAutomaticRetry() async {
        let transport = ImportTransport()
        let model = ProjectBrowserModel(client: EmptyImportList(), databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: transport)
        model.importOptionsPresented = true
        await model.previewWorkspaceRecovery(URL(fileURLWithPath: "/tmp/history"))
        await transport.setMode("lost")
        await model.recoverWorkspace()
        XCTAssertNotNil(model.recoveryPreview)
        XCTAssertTrue(model.importOptionsPresented)
        XCTAssertEqual(model.recoveryName, "Old study")
        XCTAssertTrue(model.importError?.contains("不会自动重试") == true)
        XCTAssertNil(model.activeProjectID)
        XCTAssertFalse(model.importBusy)
        await Task.yield()
        let calls = await transport.callCount(); XCTAssertEqual(calls, 2)
    }

    @MainActor func testImmediateEscapeFromRecoveryKeepsImportOptionsOpen() async throws {
        let transport = ImportTransport()
        let model = ProjectBrowserModel(client: EmptyImportList(), databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: transport)
        model.importOptionsPresented = true
        await model.previewWorkspaceRecovery(URL(fileURLWithPath: "/tmp/history"))
        let preview = try XCTUnwrap(model.recoveryPreview)
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 540, height: 440), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let parentView = NSView(frame: window.contentView!.bounds); window.contentView = parentView
        let parent = NativeSettingsEscape.Coordinator(enabled: true) { model.importOptionsPresented = false }
        parent.view = parentView; parent.install(); defer { parent.remove() }
        let hosted = NSHostingView(rootView: NativeWorkspaceRecoverySheet(model: model, preview: preview))
        hosted.frame = parentView.bounds; parentView.addSubview(hosted); hosted.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let escape = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(escape, keyWindow: window, modalWindow: nil))
        XCTAssertNil(model.recoveryPreview)
        XCTAssertTrue(model.importOptionsPresented)
        XCTAssertTrue(window.firstResponder === focus)
        let calls = await transport.callCount(); XCTAssertEqual(calls, 1)
    }

    @MainActor func testCancelingThePickerDoesNotCallTheHost() async {
        let transport = ImportTransport()
        let model = ProjectBrowserModel(client: EmptyImportList(), databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: transport)
        await model.importChosenArchive(nil)
        await model.importChosenDirectory(nil)
        let calls = await transport.callCount()
        XCTAssertEqual(calls, 0)
        XCTAssertNil(model.importError)
        XCTAssertNil(model.activeProjectID)
    }

    @MainActor func testDirectoryContractOpensInPlaceAndClosesOptionsOnlyOnSuccess() async throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let contract = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-projects/v1/import-directory.json")))
        let transport = ImportTransport()
        let model = ProjectBrowserModel(client: EmptyImportList(), databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: transport)
        model.importOptionsPresented = true
        await model.importChosenDirectory(URL(fileURLWithPath: contract["args"]["directory_path"].string))
        let call = await transport.lastCall()
        XCTAssertEqual(call.command, contract["command"].string)
        XCTAssertEqual(SettingsValue.object(call.args), contract["args"])
        XCTAssertNil(call.projectID)
        XCTAssertFalse(model.importOptionsPresented)
        XCTAssertEqual(model.activeProjectID, contract["result"]["id"].string)
    }

    @MainActor func testFolderFailuresRemainActionableAndDoNotRetry() async {
        let transport = ImportTransport()
        let model = ProjectBrowserModel(client: EmptyImportList(), databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: transport)
        model.importOptionsPresented = true
        for (mode, expected) in [("folder-invalid", "无效或不完整"), ("waiting", "等待同步完成"), ("conflict", "同步冲突")] {
            await transport.setMode(mode)
            await model.importChosenDirectory(URL(fileURLWithPath: "/tmp/project"))
            XCTAssertTrue(model.importError?.contains(expected) == true)
            XCTAssertTrue(model.importOptionsPresented)
            XCTAssertNil(model.activeProjectID)
        }
        let calls = await transport.callCount(); XCTAssertEqual(calls, 3)
    }

    @MainActor func testImmediateEscapeClosesOnlyImportOptionsWithoutCallingHost() async {
        let transport = ImportTransport()
        let model = ProjectBrowserModel(client: EmptyImportList(), databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: transport)
        model.importOptionsPresented = true
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 530, height: 350), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let parentView = NSView(frame: window.contentView!.bounds); window.contentView = parentView
        var parentClosed = 0
        let parent = NativeSettingsEscape.Coordinator(enabled: true) { parentClosed += 1 }
        parent.view = parentView; parent.install(); defer { parent.remove() }
        let hosted = NSHostingView(rootView: NativeProjectImportSheet(model: model))
        hosted.frame = parentView.bounds; parentView.addSubview(hosted); hosted.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let escape = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(escape, keyWindow: window, modalWindow: nil))
        XCTAssertFalse(model.importOptionsPresented); XCTAssertEqual(parentClosed, 0)
        XCTAssertTrue(window.firstResponder === focus)
        let calls = await transport.callCount(); XCTAssertEqual(calls, 0)
    }

    @MainActor func testSuccessOpensTheImportedProjectOnce() async throws {
        let transport = ImportTransport()
        let lists = EmptyImportList()
        let model = ProjectBrowserModel(client: lists, databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: transport)
        await model.importChosenArchive(URL(fileURLWithPath: "/Users/researcher/Exports/RNA seq.zip"))
        let call = await transport.lastCall()
        let calls = await transport.callCount()
        XCTAssertEqual(calls, 1)
        XCTAssertEqual(call.command, NativeProjectCommand.importArchive)
        XCTAssertNil(call.projectID)
        XCTAssertEqual(call.args["archive_path"]?.string, "/Users/researcher/Exports/RNA seq.zip")
        XCTAssertEqual(model.activeProjectID, "research-1")
        XCTAssertNil(model.importError)
        XCTAssertFalse(model.importBusy)
        let listed = await lists.listCount()
        XCTAssertEqual(listed, 1)
        await Task.yield()
        let after = await transport.callCount()
        XCTAssertEqual(after, 1)
    }

    @MainActor func testInvalidArchiveAndLostReplyAreNotRetried() async {
        let transport = ImportTransport()
        let model = ProjectBrowserModel(client: EmptyImportList(), databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: transport)
        await transport.setMode("invalid")
        let archive = URL(fileURLWithPath: "/tmp/bad.zip")
        await model.importChosenArchive(archive)
        XCTAssertTrue(model.importError?.contains("不是有效的项目归档") == true)
        XCTAssertNil(model.activeProjectID)
        await transport.setMode("present")
        await model.importChosenArchive(archive)
        XCTAssertEqual(model.importError, "这个项目已经在这台设备上。")
        await transport.setMode("lost")
        await model.importChosenArchive(archive)
        XCTAssertTrue(model.importError?.contains("不会自动重试") == true)
        let calls = await transport.callCount()
        XCTAssertEqual(calls, 3)
        await Task.yield()
        let after = await transport.callCount()
        XCTAssertEqual(after, 3)
    }

    @MainActor func testSecondImportWhileBusyDoesNotSendAgain() async {
        let transport = ImportTransport()
        await transport.suspend()
        let model = ProjectBrowserModel(client: EmptyImportList(), databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: transport)
        let first = Task { await model.importChosenArchive(URL(fileURLWithPath: "/tmp/study.zip")) }
        var hanging = false
        for _ in 0..<1000 {
            if await transport.isHanging() { hanging = true; break }
            await Task.yield()
        }
        XCTAssertTrue(hanging)
        await model.importChosenDirectory(URL(fileURLWithPath: "/tmp/other-project"))
        let busyCalls = await transport.callCount()
        XCTAssertEqual(busyCalls, 1)
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let fixture = try! JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-projects/v1/import.json")))
        await transport.resume(with: fixture["result"])
        await first.value
        XCTAssertEqual(model.activeProjectID, "research-1")
        let finished = await transport.callCount()
        XCTAssertEqual(finished, 1)
    }
}

private actor EmptyImportList: ProjectBrowserQuerying {
    var lists = 0
    var failure = false
    func setFailure(_ value: Bool) { failure = value }
    func listCount() -> Int { lists }
    func listProjects(databaseURL: URL) async throws -> ProjectListSnapshot {
        lists += 1
        if failure { throw ProjectBrowserError.service("offline") }
        return ProjectListSnapshot(projects: [], activitySource: "persisted_only")
    }
    func listSessions(databaseURL: URL, projectID: String?) async throws -> [BrowserSession] { [] }
    func transcript(databaseURL: URL, projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> TranscriptPage {
        TranscriptPage(messages: [], nextBeforeSeq: nil)
    }
}
