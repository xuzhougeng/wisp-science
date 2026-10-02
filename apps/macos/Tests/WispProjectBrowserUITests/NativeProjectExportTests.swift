import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor ExportTransport: NativeSettingsQuerying {
    var calls: [(String, [String: SettingsValue], String?)] = []
    var mode = "success"
    private var pending: CheckedContinuation<SettingsValue, Error>?
    func setMode(_ value: String) { mode = value }
    func count() -> Int { calls.count }
    func last() -> (String, [String: SettingsValue], String?) { calls.last! }
    func isPending() -> Bool { pending != nil }
    func finish() { pending?.resume(returning: response()); pending = nil }
    private func response() -> SettingsValue {
        let call = calls.last!
        return .object(["project_id": .string(mode == "wrong-project" ? "other" : call.2!), "destination_path": call.1["destination_path"]!, "format": call.1["format"]!])
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        calls.append((command, args, projectID))
        if mode == "lost" { throw ProjectBrowserError.service("connection lost") }
        if mode == "hang" { return try await withCheckedThrowingContinuation { pending = $0 } }
        return response()
    }
}

final class NativeProjectExportTests: XCTestCase {
    private func fixture(_ name: String) throws -> SettingsValue {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        return try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-projects/v1/\(name).json")))
    }
    private func project() throws -> ProjectSummary { try NativeProjectCommand.summary(from: fixture("import")["result"]) }

    @MainActor func testCanceledPickerDoesNotExport() async throws {
        let transport = ExportTransport()
        let model = NativeProjectExportModel(project: try project(), client: transport)
        await model.export(to: nil, directory: false)
        let count = await transport.count(); XCTAssertEqual(count, 0)
        XCTAssertNil(model.error); XCTAssertNil(model.destination)
    }

    @MainActor func testExportContractScopesProjectAndOpensOnlyConfirmedDestination() async throws {
        let contract = try fixture("export")
        for directory in [false, true] {
            let transport = ExportTransport()
            let model = NativeProjectExportModel(project: try project(), client: transport)
            let url = URL(fileURLWithPath: directory ? "/tmp/exported-project" : contract["args"]["destination_path"].string)
            await model.export(to: url, directory: directory)
            let call = await transport.last()
            XCTAssertEqual(call.0, contract["command"].string)
            XCTAssertEqual(call.2, contract["project_id"].string)
            XCTAssertEqual(call.1["format"]?.string, directory ? "directory" : "zip")
            XCTAssertEqual(call.1["destination_path"]?.string, url.path)
            if !directory { XCTAssertEqual(SettingsValue.object(call.1), contract["args"]) }
            XCTAssertEqual(model.destination, url)
            XCTAssertNil(model.error)
            await model.export(to: url, directory: directory)
            let count = await transport.count(); XCTAssertEqual(count, 1)
        }
    }

    @MainActor func testLostAndMismatchedRepliesAreNotReportedAsSuccessOrRetried() async throws {
        for mode in ["lost", "wrong-project"] {
            let transport = ExportTransport(); await transport.setMode(mode)
            let model = NativeProjectExportModel(project: try project(), client: transport)
            await model.export(to: URL(fileURLWithPath: "/tmp/export.zip"), directory: false)
            XCTAssertNil(model.destination)
            XCTAssertTrue(model.error?.contains("不会自动重试") == true)
            XCTAssertFalse(model.busy)
            await Task.yield()
            let count = await transport.count(); XCTAssertEqual(count, 1)
        }
    }

    @MainActor func testBusyExportRejectsDuplicateSubmission() async throws {
        let transport = ExportTransport(); await transport.setMode("hang")
        let model = NativeProjectExportModel(project: try project(), client: transport)
        let url = URL(fileURLWithPath: "/tmp/export.zip")
        let first = Task { await model.export(to: url, directory: false) }
        for _ in 0..<1000 { if await transport.isPending() { break }; await Task.yield() }
        XCTAssertTrue(model.busy)
        await model.export(to: URL(fileURLWithPath: "/tmp/another"), directory: true)
        let count = await transport.count(); XCTAssertEqual(count, 1)
        await transport.finish(); await first.value
        XCTAssertEqual(model.destination, url)
    }

    @MainActor func testImmediateEscapeClosesOnlyExportSheetWithoutWriting() async throws {
        let transport = ExportTransport()
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 540, height: 440), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let parentView = NSView(frame: window.contentView!.bounds); window.contentView = parentView
        var parentClosed = false; var exportClosed = false
        let parent = NativeSettingsEscape.Coordinator(enabled: true) { parentClosed = true }
        parent.view = parentView; parent.install(); defer { parent.remove() }
        let hosted = NSHostingView(rootView: NativeProjectExportSheet(project: try project(), client: transport) { exportClosed = true })
        hosted.frame = parentView.bounds; parentView.addSubview(hosted); hosted.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let escape = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(escape, keyWindow: window, modalWindow: nil))
        XCTAssertTrue(exportClosed); XCTAssertFalse(parentClosed)
        XCTAssertTrue(window.firstResponder === focus)
        let count = await transport.count(); XCTAssertEqual(count, 0)
    }
}
