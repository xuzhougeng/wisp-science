import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
import UniformTypeIdentifiers
@testable import WispProjectBrowserUI

private func filesFixture() throws -> SettingsValue {
    var root = URL(fileURLWithPath: #filePath); for _ in 0..<5 { root.deleteLastPathComponent() }
    return try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-files/v1/browser.json")))
}
private actor FilesFake: NativeConversationQuerying {
    let fixture: SettingsValue
    var hold = "", failure = ""
    var held: CheckedContinuation<Void, Never>?
    var calls: [(String, String, [String: SettingsValue])] = []
    init(_ fixture: SettingsValue) { self.fixture = fixture }
    func setup(hold: String = "", failure: String = "") { self.hold = hold; self.failure = failure }
    func waiting() -> Bool { held != nil }
    func release() { held?.resume(); held = nil; hold = "" }
    func recorded() -> [(String, String, [String: SettingsValue])] { calls }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { throw ProjectBrowserError.invalidResponse }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        calls.append((projectID, command, args)); let mode = failure
        let context = args["context_id"]?.string ?? "local"
        var result: SettingsValue
        switch command {
        case "native_conversation_panel_file_locations": result = fixture["locations"]
        case "native_conversation_panel_file_directory":
            result = fixture[context == "local" ? "local_directory" : "remote_directory"]
            if context == "local" { result["path"] = args["path"] ?? .string(".") }
        case "native_conversation_panel_file_read":
            result = fixture[context == "local" ? "local_preview" : "remote_preview"]
            if args["path"]!.string.hasSuffix(".png"), fixture["document_image"] != .null { result["content"] = fixture["document_image"] }
            let path = args["path"]!.string; result["requested_path"] = .string(path)
            result["content"]["path"] = .string(context == "local" ? fixture["locations"]["local_root"].string + "/" + path : "ssh://" + String(context.dropFirst(4)) + "/" + path.drop(while: { $0 == "/" }))
        case "native_conversation_panel_file_paths":
            result = fixture["paths"]
            let root = fixture["locations"]["local_root"].string
            result["paths"] = .array(args["paths"]!.array.map { .object(["requested_path": $0, "relative_path": $0, "absolute_path": .string(root + "/" + $0.string)]) })
        case "native_conversation_panel_searchfiles":
            result = .array([.object(["path": .string("other/QC.csv"), "name": .string("QC.csv"), "is_dir": .bool(false), "size": .integer(18)]), .object(["path": .string("results/QC.csv"), "name": .string("QC.csv"), "is_dir": .bool(false), "size": .integer(18)])])
        case "native_conversation_panel_file_action", "native_conversation_panel_savefile": result = .bool(true)
        case "native_conversation_panel_file_upload":
            result = fixture[context == "local" ? "local_upload" : "remote_upload"]
            result["path"] = args["path"]!
            result["items"] = .array(args["source_paths"]!.array.enumerated().map { index, source in
                let local = context == "local", missing = context == "local" && source.string.hasSuffix("missing.csv")
                var item = fixture[local ? "local_upload" : "remote_upload"]["items"].array[missing ? 1 : 0]
                item["source_path"] = source
                if item["status"] != .string("failed") {
                    let directory = args["path"]!.string
                    let name = (source.string as NSString).lastPathComponent
                    item["destination_path"] = .string((directory == "." ? "" : directory == "/" ? "/" : directory + "/") + (context == "local" && name == "QC.csv" ? "QC_1.csv" : name))
                }
                if context != "local" { item["run_id"] = .string("upload-\(index + 1)") }
                return item
            })
        case "native_conversation_panel_file_download":
            result = fixture["remote_download"]; result["path"] = args["path"]!
            var item = result["items"].array[0]; item["source_path"] = args["path"]!; item["destination_path"] = args["destination_path"]!; result["items"] = .array([item])
        case "native_conversation_panel_activity":
            let transfers = calls.filter { $0.1.hasSuffix("file_upload") || $0.1.hasSuffix("file_download") }.filter { $0.2["context_id"] != .string("local") }
            var runs: [SettingsValue] = []
            for call in transfers {
                let download = call.1.hasSuffix("file_download")
                for index in 0..<(download ? 1 : call.2["source_paths"]!.array.count) {
                    runs.append(.object(["id": .string(download ? "download-1" : "upload-\(index + 1)"), "frame_id": .string(mode == "run_scope" ? "foreign-session" : "session-1"), "context_id": call.2["context_id"]!, "title": .string("Transfer QC.csv"), "kind": .string("file_transfer"), "status": .string(mode == "run_complete" ? "succeeded" : "running"), "created_at": .integer(1700000000), "progress_json": .string("{\"total_bytes\":17,\"completed_bytes\":9,\"indeterminate\":false}")]))
                }
            }
            result = .object(["runtimes": .array([]), "runs": .array(mode == "run_missing" ? [] : runs), "read_only": .bool(false)])
        default: throw ProjectBrowserError.invalidResponse
        }
        if command == hold { await withCheckedContinuation { held = $0 } }
        if mode == "lost" { throw ProjectBrowserError.service("lost reply") }
        if mode == "malformed" { return .null }
        if mode == "owner" { result["session_id"] = .string("other-session") }
        return result
    }
}

private final class FilesDropGate: @unchecked Sendable {
    private let lock = NSLock()
    private var callback: ((Data?, Error?) -> Void)?
    func install(_ callback: @escaping (Data?, Error?) -> Void) { lock.lock(); self.callback = callback; lock.unlock() }
    var waiting: Bool { lock.lock(); defer { lock.unlock() }; return callback != nil }
    func release(_ data: Data) { lock.lock(); let call = callback; callback = nil; lock.unlock(); call?(data, nil) }
}

@MainActor private final class FilesDatabaseSelection: ObservableObject {
    @Published var databaseURL = URL(fileURLWithPath: "/tmp/original.sqlite")
    let original: FilesFake
    let clone: FilesFake
    init(original: FilesFake, clone: FilesFake) { self.original = original; self.clone = clone }
}
private struct FilesDatabasePane: View {
    @ObservedObject var selection: FilesDatabaseSelection
    var body: some View {
        NativePanelView(client: selection.databaseURL.lastPathComponent == "original.sqlite" ? selection.original : selection.clone,
                        projectID: "research-1", sessionID: "session-1", fileBrowserSupported: true, fileTransfersSupported: true, close: {})
            .id(NativePanelScopeIdentity(databaseURL: selection.databaseURL, projectID: "research-1", sessionID: "session-1"))
    }
}

final class NativeFilesTests: XCTestCase {
    @MainActor private func model(_ fake: FilesFake, readOnly: Bool = false, transfersSupported: Bool = false) -> NativeFilesModel {
        NativeFilesModel(client: fake, projectID: "research-1", sessionID: "session-1", readOnly: readOnly, transfersSupported: transfersSupported)
    }
    @MainActor func testRichFileQuotesValidateRenderedMarkdownAndTsvWithExactLocalAndSshSources() async throws {
        var fixture = try filesFixture()
        for key in ["local_preview", "remote_preview"] { fixture[key]["content"]["mime"] = .string("text/markdown"); fixture[key]["content"]["text"] = .string("# Result\n\n**Sample A** passed.") }
        let fake = FilesFake(fixture), model = model(fake); await model.open()
        for context in ["local", "ssh:lab"] {
            if context != "local" { await model.chooseLocation(context) }
            await model.read(try XCTUnwrap(model.rows.first { !$0.directory })); let source = try XCTUnwrap(model.preview?.content.path)
            let quote = try XCTUnwrap(model.quote("Result\nSample A passed.", source: source)); XCTAssertEqual(quote.source, source)
            XCTAssertNil(model.quote("Sample B passed.", source: source)); XCTAssertNil(model.quote(quote.text, source: "other"))
            model.dismissPreview(); XCTAssertNil(model.quote(quote.text, source: source))
        }
        let csvFake = FilesFake(try filesFixture()), csv = self.model(csvFake); await csv.open()
        await csv.read(try XCTUnwrap(csv.rows.first { $0.path.hasSuffix("QC.csv") })); let csvSource = try XCTUnwrap(csv.preview?.content.path)
        XCTAssertEqual(csv.quote("A\tpass", source: csvSource)?.source, csvSource)
        XCTAssertNil(csv.quote("A\tfail", source: csvSource))
    }
    @MainActor func testMarkdownImagesUseDocumentDirectoryAndExactFileScopeWithoutExternalReads() async throws {
        var fixture = try filesFixture(); fixture["document_image"] = try JSONDecoder().decode(SettingsValue.self, from: JSONEncoder().encode(imagePreview()))
        let fake = FilesFake(fixture), model = model(fake); await model.open()
        for context in ["local", "ssh:lab"] {
            if context == "local" { await model.navigate("results") } else { await model.chooseLocation(context) }
            await model.read(try XCTUnwrap(model.rows.first { !$0.directory })); let original = try XCTUnwrap(model.preview?.content)
            let image = try await model.readPreviewImage("../figures/plot%20A.png", original: original)
            XCTAssertEqual(image.mime, "image/png"); XCTAssertNotNil(try NativeFileDocumentImages.decode(image))
            let calls = await fake.recorded(), read = try XCTUnwrap(calls.last)
            XCTAssertEqual(read.0, "research-1"); XCTAssertEqual(read.2["session_id"], .string("session-1")); XCTAssertEqual(read.2["context_id"], .string(context))
            XCTAssertEqual(read.2["path"], .string(context == "local" ? "figures/plot A.png" : "/home/research/figures/plot A.png"))
            for invalid in ["https://example.com/plot.png", "file://foreign/plot.png", "plot.svg"] {
                do { _ = try await model.readPreviewImage(invalid, original: original); XCTFail(invalid) } catch {}
            }
            let after = await fake.recorded(); XCTAssertEqual(after.count, calls.count)
        }
    }
    @MainActor func testClosedChangedCancelledAndForeignImageRepliesCannotReachTheCurrentDocument() async throws {
        for change in ["dismiss", "location", "close", "cancel", "save", "owner"] {
            var fixture = try filesFixture(); fixture["document_image"] = try JSONDecoder().decode(SettingsValue.self, from: JSONEncoder().encode(imagePreview()))
            let fake = FilesFake(fixture), model = model(fake); await model.open()
            await model.read(try XCTUnwrap(model.rows.first { !$0.directory })); let original = try XCTUnwrap(model.preview?.content)
            await fake.setup(hold: "native_conversation_panel_file_read", failure: change == "owner" ? "owner" : "")
            let pending = Task { try await model.readPreviewImage("plot.png", original: original) }
            while !(await fake.waiting()) { await Task.yield() }
            switch change {
            case "dismiss": model.dismissPreview()
            case "location": await model.chooseLocation("ssh:lab")
            case "close": model.close()
            case "cancel": pending.cancel()
            case "save": try await model.save("changed source", original: original)
            default: break
            }
            await fake.release()
            do { _ = try await pending.value; XCTFail(change) } catch {}
            XCTAssertFalse(model.previewLoading)
        }
    }
    func testSharedTransferContractsRequireExactSourcesDestinationsAndOwnership() throws {
        let fixture = try filesFixture()
        for key in ["local_upload", "remote_upload", "remote_download"] {
            let value = fixture[key], download = key == "remote_download"
            let sources = download ? [] : value["items"].array.map { $0["source_path"].string }
            let destination = download ? value["items"].array[0]["destination_path"].string : nil
            func decode(_ page: SettingsValue) throws -> NativeFileTransfer {
                try NativeFileTransfer.decode(page, project: "research-1", session: "session-1", context: value["context_id"].string, path: value["path"].string, sources: sources, destination: destination)
            }
            XCTAssertFalse(try decode(value).items.isEmpty)
            for field in ["schema", "project_id", "session_id", "context_id", "path"] {
                var bad = value; bad[field] = .string("foreign"); XCTAssertThrowsError(try decode(bad), field)
            }
            var bad = value, items = value["items"].array
            items[0]["source_path"] = .string("/other/source.csv"); bad["items"] = .array(items); XCTAssertThrowsError(try decode(bad))
            items = value["items"].array; items[0]["destination_path"] = .string("/outside/QC.csv"); bad["items"] = .array(items); XCTAssertThrowsError(try decode(bad))
            bad = value; bad["items"] = .array([]); XCTAssertThrowsError(try decode(bad))
        }
    }
    @MainActor func testCapturedUploadTargetsRejectChangedDirectoryQueryLocationClosureAndCapabilities() async throws {
        for change in ["directory", "query", "location", "close", "readOnly", "capability"] {
            let fake = FilesFake(try filesFixture()), model = model(fake, transfersSupported: true); await model.open()
            let target = try XCTUnwrap(model.transferTarget(upload: true))
            switch change {
            case "directory": await model.navigate("results"); await model.navigate(".")
            case "query": model.query = "QC"; model.query = ""
            case "location": await model.chooseLocation("ssh:lab"); await model.chooseLocation("local")
            case "close": model.close()
            case "readOnly": model.readOnly = true
            default: model.transfersSupported = false
            }
            await model.upload(["/incoming/QC.csv"], target: target)
            let calls = await fake.recorded(); XCTAssertFalse(calls.contains { $0.1.hasSuffix("file_upload") }, change)
        }
    }
    @MainActor func testLocalPartialUploadsAndRemoteRunsRetainTheExactFrameAndDestination() async throws {
        let fake = FilesFake(try filesFixture()), model = model(fake, transfersSupported: true); await model.open(); await model.navigate("results")
        await model.upload(["/incoming/QC.csv", "/incoming/missing.csv"], target: try XCTUnwrap(model.transferTarget(upload: true)))
        XCTAssertEqual(model.transfer?.items.map(\.status), ["succeeded", "failed"])
        XCTAssertEqual(model.transfer?.items[0].destination_path, "results/QC_1.csv"); XCTAssertNotNil(model.transfer?.items[1].error)
        XCTAssertFalse(model.transferUnconfirmed); XCTAssertFalse(model.transferSubmitting)
        await model.chooseLocation("ssh:lab"); XCTAssertTrue(model.canUpload); XCTAssertFalse(model.canWrite)
        await model.upload(["/incoming/QC.csv"], target: try XCTUnwrap(model.transferTarget(upload: true)))
        XCTAssertTrue(model.hasActiveTransfers); await model.refreshTransferRuns()
        XCTAssertEqual(model.transferRuns.values.first?.frame_id, "session-1")
        await fake.setup(failure: "run_complete"); await model.refreshTransferRuns(); XCTAssertFalse(model.hasActiveTransfers)
        let calls = await fake.recorded(), writes = calls.filter { $0.1.hasSuffix("file_upload") }
        XCTAssertEqual(writes.count, 2); XCTAssertEqual(writes.last?.2["path"], .string("/home/research/results")); XCTAssertEqual(writes.last?.2["context_id"], .string("ssh:lab"))
        XCTAssertTrue(writes.allSatisfy { $0.0 == "research-1" && $0.2["session_id"] == .string("session-1") })
    }
    @MainActor func testRemoteDownloadPreservesChosenSourceDestinationAndReadOnlyExportBehavior() async throws {
        let fake = FilesFake(try filesFixture()), model = model(fake, readOnly: true, transfersSupported: true); await model.open(); await model.chooseLocation("ssh:lab")
        XCTAssertFalse(model.canUpload); XCTAssertTrue(model.canDownload)
        let target = try XCTUnwrap(model.transferTarget(upload: false)), row = try XCTUnwrap(model.rows.first { !$0.directory })
        await model.download(try XCTUnwrap(model.rows.first { $0.directory }), destination: "/exports/QC.csv", target: target)
        await model.download(row, destination: "relative.csv", target: target)
        await model.download(row, destination: "/exports/QC.csv", target: target)
        XCTAssertEqual(model.transfer?.items.first?.destination_path, "/exports/QC.csv")
        let writes = await fake.recorded().filter { $0.1.hasSuffix("file_download") }
        XCTAssertEqual(writes.count, 1); XCTAssertEqual(writes[0].2["path"], .string("/home/research/results/QC.csv")); XCTAssertEqual(writes[0].2["destination_path"], .string("/exports/QC.csv"))
        XCTAssertEqual(writes[0].2["session_id"], .string("session-1")); XCTAssertEqual(writes[0].2["context_id"], .string("ssh:lab"))
    }
    @MainActor func testLostMalformedOrForeignTransferAcknowledgementsBlockReplayUntilFreshRead() async throws {
        for failure in ["lost", "malformed", "owner"] {
            let fake = FilesFake(try filesFixture()), model = model(fake, transfersSupported: true); await model.open()
            let target = try XCTUnwrap(model.transferTarget(upload: true)); await fake.setup(failure: failure)
            await model.upload(["/incoming/QC.csv"], target: target); await model.upload(["/incoming/QC.csv"], target: target)
            XCTAssertTrue(model.transferUnconfirmed); XCTAssertFalse(model.transferSubmitting); XCTAssertNil(model.transfer); XCTAssertNotNil(model.transferError)
            let writes = await fake.recorded().filter { $0.1.hasSuffix("file_upload") }; XCTAssertEqual(writes.count, 1)
            await fake.setup(); await model.refresh(); XCTAssertFalse(model.transferUnconfirmed)
            await model.upload(["/incoming/QC.csv"], target: try XCTUnwrap(model.transferTarget(upload: true)))
            let final = await fake.recorded().filter { $0.1.hasSuffix("file_upload") }; XCTAssertEqual(final.count, 2); XCTAssertNotNil(model.transfer)
        }
    }
    @MainActor func testCancelledStartupAndClosedHeldTransfersCannotDispatchOrRestorePanels() async throws {
        let fake = FilesFake(try filesFixture()), model = model(fake, transfersSupported: true); await model.open()
        let target = try XCTUnwrap(model.transferTarget(upload: true))
        let cancelled = Task { await model.upload(["/incoming/QC.csv"], target: target) }; cancelled.cancel(); await cancelled.value
        XCTAssertFalse(model.transferUnconfirmed)
        await fake.setup(hold: "native_conversation_panel_file_upload")
        let pending = Task { await model.upload(["/incoming/QC.csv"], target: target) }
        while !(await fake.waiting()) { await Task.yield() }
        XCTAssertTrue(model.transferSubmitting); await model.chooseLocation("ssh:lab"); XCTAssertEqual(model.contextID, "local")
        model.close(); await fake.release(); await pending.value
        XCTAssertNil(model.transfer); XCTAssertFalse(model.transferSubmitting); XCTAssertFalse(model.canUpload)
        let writes = await fake.recorded().filter { $0.1.hasSuffix("file_upload") }; XCTAssertEqual(writes.count, 1)
    }
    @MainActor func testConfirmedUploadAcknowledgementPreservesSearchChangedDuringSubmission() async throws {
        let fake = FilesFake(try filesFixture()), model = model(fake, transfersSupported: true); await model.open()
        let target = try XCTUnwrap(model.transferTarget(upload: true)); await fake.setup(hold: "native_conversation_panel_file_upload")
        let pending = Task { await model.upload(["/incoming/QC.csv"], target: target) }
        while !(await fake.waiting()) { await Task.yield() }
        model.query = "QC"; await fake.release(); await pending.value
        XCTAssertEqual(model.query, "QC"); XCTAssertEqual(model.path, ".")
        XCTAssertEqual(model.transfer?.items.first?.status, "succeeded"); XCTAssertFalse(model.transferUnconfirmed); XCTAssertFalse(model.transferSubmitting)
        let writes = await fake.recorded().filter { $0.1.hasSuffix("file_upload") }; XCTAssertEqual(writes.count, 1)
    }
    @MainActor func testForeignAndSupersededRunRepliesCannotPopulateTransferStatus() async throws {
        let fake = FilesFake(try filesFixture()), model = model(fake, transfersSupported: true); await model.open(); await model.chooseLocation("ssh:lab")
        await model.upload(["/incoming/QC.csv"], target: try XCTUnwrap(model.transferTarget(upload: true)))
        for failure in ["run_scope", "run_missing"] {
            await fake.setup(failure: failure); await model.refreshTransferRuns(); XCTAssertTrue(model.transferRuns.isEmpty); XCTAssertNotNil(model.transferError)
        }
        await fake.setup(hold: "native_conversation_panel_activity")
        let pending = Task { await model.refreshTransferRuns() }; while !(await fake.waiting()) { await Task.yield() }
        await model.chooseLocation("local"); await fake.setup()
        await model.upload(["/incoming/QC.csv"], target: try XCTUnwrap(model.transferTarget(upload: true)))
        await fake.release(); await pending.value
        XCTAssertEqual(model.transfer?.context_id, "local"); XCTAssertTrue(model.transferRuns.isEmpty)
    }
    func testDropSourcesDecodeNativeURLsDataStringsAndRejectNonFileItems() async throws {
        let url = URL(fileURLWithPath: "/incoming/样本 QC.csv")
        XCTAssertEqual(try NativeFileDropSources.url(url as NSURL), url)
        XCTAssertEqual(try NativeFileDropSources.url(url.dataRepresentation as NSData), url)
        XCTAssertEqual(try NativeFileDropSources.url(url.absoluteString as NSString), url)
        for invalid in ["https://example.com/qc.csv", "/incoming/raw-string.csv"] { XCTAssertThrowsError(try NativeFileDropSources.url(invalid as NSString)) }
        XCTAssertThrowsError(try NativeFileDropSources.url(nil))
        let providers = [NSItemProvider(item: url as NSURL, typeIdentifier: UTType.fileURL.identifier), NSItemProvider(item: url as NSURL, typeIdentifier: UTType.fileURL.identifier)]
        let paths = try await NativeFileDropSources.paths(providers); XCTAssertEqual(paths, [url.path])
    }
    @MainActor func testChooserWindowIdentityDoesNotFollowAnotherWorkspaceWindow() {
        _ = NSApplication.shared
        let a = NSWindow(contentRect: .zero, styleMask: [.titled], backing: .buffered, defer: false)
        let b = NSWindow(contentRect: .zero, styleMask: [.titled], backing: .buffered, defer: false)
        a.isReleasedWhenClosed = false; b.isReleasedWhenClosed = false; defer { a.close(); b.close() }
        let chooser = NativeFilesChooser(); chooser.window = a; XCTAssertTrue(chooser.owns(a))
        chooser.window = b; XCTAssertFalse(chooser.owns(a)); XCTAssertTrue(chooser.owns(b))
        chooser.close(); XCTAssertFalse(chooser.owns(b))
    }
    @MainActor func testDelayedNativeDropCannotUploadIntoANewDirectory() async throws {
        let fake = FilesFake(try filesFixture()), model = model(fake, transfersSupported: true); await model.open()
        let target = try XCTUnwrap(model.transferTarget(upload: true)), gate = FilesDropGate(), provider = NSItemProvider()
        provider.registerDataRepresentation(forTypeIdentifier: UTType.fileURL.identifier, visibility: .all) { callback in gate.install(callback); return nil }
        let pending = Task { let paths = try await NativeFileDropSources.paths([provider]); await model.upload(paths, target: target) }
        for _ in 0..<500 { if gate.waiting { break }; try await Task.sleep(nanoseconds: 2_000_000) }
        XCTAssertTrue(gate.waiting); await model.navigate("results")
        gate.release(URL(fileURLWithPath: "/incoming/QC.csv").dataRepresentation); try await pending.value
        let calls = await fake.recorded(); XCTAssertFalse(calls.contains { $0.1.hasSuffix("file_upload") }); XCTAssertEqual(model.path, "results")
    }
    @MainActor func testImmediateEscapeCancelsNativeUploadPickerAndPreservesFilesParent() async throws {
        _ = NSApplication.shared
        let fake = FilesFake(try filesFixture()), model = model(fake, transfersSupported: true); await model.open()
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 419, height: 500), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        let chooser = NativeFilesChooser(); chooser.window = window
        defer { chooser.close(); model.close(); window.close() }
        var parentClosed = 0
        let host = NSHostingView(rootView: Text("Files").background(NativeSettingsEscape { parentClosed += 1 })); window.contentView = host; host.layoutSubtreeIfNeeded()
        chooser.upload(model)
        let panel = try XCTUnwrap(chooser.panel)
        let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: panel.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53))
        XCTAssertFalse(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertTrue(chooser.consume(event))
        for _ in 0..<200 { if !chooser.busy { break }; try await Task.sleep(nanoseconds: 10_000_000) }
        XCTAssertFalse(chooser.busy); XCTAssertNil(window.attachedSheet); XCTAssertEqual(parentClosed, 0)
        let calls = await fake.recorded(); XCTAssertFalse(calls.contains { $0.1.hasSuffix("file_upload") })
    }
    @MainActor func testRenderTransferControlsPartialResultsAndProgressAtNarrowWidths() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Set WISP_NATIVE_SNAPSHOT_DIR to render files") }
        let saved = UserDefaults.standard.object(forKey: "nativeSettings.locale")
        defer { if let saved { UserDefaults.standard.set(saved, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        for locale in ["zh", "en"] { for scheme in [ColorScheme.light, .dark] { for remote in [false, true] { for width in [320.0, 419.0] {
            UserDefaults.standard.set(locale, forKey: "nativeSettings.locale")
            let fake = FilesFake(try filesFixture()), model = model(fake, transfersSupported: true); await model.open(); await model.navigate("results")
            if remote { await model.chooseLocation("ssh:lab") }
            await model.upload(remote ? ["/incoming/QC.csv"] : ["/incoming/QC.csv", "/incoming/missing.csv"], target: try XCTUnwrap(model.transferTarget(upload: true)))
            if remote { await model.refreshTransferRuns() }
            let host = NSHostingView(rootView: NativeFilesView(client: fake, projectID: "research-1", sessionID: "session-1", readOnly: false, transfersSupported: true, model: model).padding(12).background(WispDesign.color("bg-sunken", scheme)).foregroundStyle(WispDesign.color("text", scheme)).tint(WispDesign.color("clay", scheme)).environment(\.colorScheme, scheme))
            host.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua); host.frame = NSRect(x: 0, y: 0, width: width, height: 550); host.layoutSubtreeIfNeeded()
            let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds)); host.cacheDisplay(in: host.bounds, to: bitmap)
            let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:])); XCTAssertGreaterThan(data.count, 1000)
            try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent("files-transfer-\(remote ? "remote" : "local")-\(locale)-\(scheme == .dark ? "dark" : "light")-\(Int(width)).png")); model.close()
        } } } }
    }
    @MainActor func testEnvironmentShortcutClearsPreviousProjectSettingsScope() {
        let browser = ProjectBrowserModel()
        browser.openProjectSettings("previous-project"); browser.settingsPresented = false
        browser.openEnvironmentSettings()
        XCTAssertNil(browser.projectSettingsID); XCTAssertEqual(browser.settingsSectionID, "environments"); XCTAssertTrue(browser.settingsPresented)
    }
    @MainActor func testPaneLifetimeIncludesDatabaseForClonedProjectAndSessionIdentities() async throws {
        _ = NSApplication.shared
        let keys = ["native.workspace.panel.tab", "native.workspace.panel.tabs"]
        let saved = keys.map { UserDefaults.standard.object(forKey: $0) }
        defer { for (key, value) in zip(keys, saved) { if let value { UserDefaults.standard.set(value, forKey: key) } else { UserDefaults.standard.removeObject(forKey: key) } } }
        UserDefaults.standard.set("files", forKey: keys[0]); UserDefaults.standard.set("[\"files\"]", forKey: keys[1])
        let original = FilesFake(try filesFixture()), clone = FilesFake(try filesFixture())
        await original.setup(hold: "native_conversation_panel_file_directory")
        let selection = FilesDatabaseSelection(original: original, clone: clone)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 419, height: 600), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let host = NSHostingView(rootView: FilesDatabasePane(selection: selection)); window.contentView = host; host.layoutSubtreeIfNeeded()
        for _ in 0..<200 { if await original.waiting() { break }; try await Task.sleep(nanoseconds: 10_000_000) }
        let waiting = await original.waiting(); XCTAssertTrue(waiting)
        selection.databaseURL = URL(fileURLWithPath: "/tmp/clone.sqlite"); host.layoutSubtreeIfNeeded()
        for _ in 0..<200 { if await clone.recorded().contains(where: { $0.1.hasSuffix("file_directory") }) { break }; try await Task.sleep(nanoseconds: 10_000_000) }
        let calls = await clone.recorded(); XCTAssertEqual(calls.filter { $0.1.hasSuffix("file_locations") }.count, 1)
        XCTAssertEqual(calls.filter { $0.1.hasSuffix("file_directory") }.count, 1)
        XCTAssertTrue(calls.allSatisfy { $0.0 == "research-1" && $0.2["session_id"] == .string("session-1") })
        await original.release(); await Task.yield()
        let final = await clone.recorded(); XCTAssertEqual(final.count, calls.count)
    }
    func testSharedContractsBindOwnersLocationsPathsAndProvenance() throws {
        let fixture = try filesFixture(), root = fixture["locations"]["local_root"].string
        let catalog = try NativeFileLocations.decode(fixture["locations"], project: "research-1", session: "session-1")
        XCTAssertEqual(catalog.locations.map(\.id), ["local", "ssh:lab"])
        for context in ["local", "ssh:lab"] {
            let directory = try NativeFileDirectory.decode(fixture[context == "local" ? "local_directory" : "remote_directory"], project: "research-1", session: "session-1", context: context)
            XCTAssertEqual(directory.entries.first?.modified_unix_millis, 1700000002000)
            let value = fixture[context == "local" ? "local_preview" : "remote_preview"]
            XCTAssertEqual(try NativeFilePreview.decode(value, project: "research-1", session: "session-1", context: context, requested: value["requested_path"].string, root: root).content.text, "sample,QC\nA,pass\n")
        }
        let requested = ["results/QC.csv", "results/figures"]
        XCTAssertEqual(try NativeFilePaths.decode(fixture["paths"], project: "research-1", session: "session-1", requested: requested, root: root).paths.map(\.relative_path), requested)
        for field in ["schema", "project_id", "session_id"] {
            var bad = fixture["locations"]; bad[field] = .string("foreign")
            XCTAssertThrowsError(try NativeFileLocations.decode(bad, project: "research-1", session: "session-1"))
        }
        for path in ["/work/research/branches/exploration-other/a", root + "/../outside/a", "ssh://lab/a"] {
            var bad = fixture["paths"]; var paths = bad["paths"].array; paths[0]["absolute_path"] = .string(path); bad["paths"] = .array(paths)
            XCTAssertThrowsError(try NativeFilePaths.decode(bad, project: "research-1", session: "session-1", requested: requested, root: root))
        }
        XCTAssertThrowsError(try NativeFilePaths.decode(fixture["paths"], project: "research-1", session: "session-1", requested: Array(requested.reversed()), root: root))
        XCTAssertThrowsError(try NativeFilePreview.decode(fixture["remote_preview"], project: "research-1", session: "session-1", context: "ssh:other", requested: fixture["remote_preview"]["requested_path"].string, root: root))
        var bad = fixture["local_directory"]; bad["entries"] = .array([.object(["name": .string("../outside"), "is_dir": .bool(true), "size": .integer(0)])])
        XCTAssertThrowsError(try NativeFileDirectory.decode(bad, project: "research-1", session: "session-1", context: "local"))
        var snapshot = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("contracts/native-conversations/v1/snapshot.json")))
        XCTAssertNil(try ConversationSnapshot.decode(snapshot, projectID: "project-a", sessionID: "session-a").file_browser)
        XCTAssertNil(try ConversationSnapshot.decode(snapshot, projectID: "project-a", sessionID: "session-a").file_transfers)
        snapshot["file_browser"] = .bool(true)
        XCTAssertEqual(try ConversationSnapshot.decode(snapshot, projectID: "project-a", sessionID: "session-a").file_browser, true)
    }
    func testSortMatchesWebDirectoryGroupingDescendingSizeAndTimeAndMissingMetadata() throws {
        let rows = try JSONDecoder().decode([NativePanelFile].self, from: Data(#"""
        [{"name":"z-dir","is_dir":true,"size":0,"modified_unix_millis":20},
         {"name":"A-dir","is_dir":true,"size":2,"modified_unix_millis":10},
         {"name":"b.csv","is_dir":false,"size":10,"modified_unix_millis":30},
         {"name":"A.csv","is_dir":false,"size":10,"modified_unix_millis":30},
         {"name":"large.bin","is_dir":false,"size":100}]
        """#.utf8))
        XCTAssertEqual(NativeFileSort.name.sorted(rows).map(\.name), ["A-dir", "z-dir", "A.csv", "b.csv", "large.bin"])
        XCTAssertEqual(NativeFileSort.size.sorted(rows).map(\.name), ["A-dir", "z-dir", "large.bin", "A.csv", "b.csv"])
        XCTAssertEqual(NativeFileSort.modified.sorted(rows).map(\.name), ["z-dir", "A-dir", "A.csv", "b.csv", "large.bin"])
    }
    @MainActor func testAllConfiguredLocationsAndRemotePreviewHaveExactSessionScopeAndAreReadOnly() async throws {
        let fake = FilesFake(try filesFixture()), model = model(fake); await model.open()
        XCTAssertTrue(model.canWrite); await model.chooseLocation("ssh:lab")
        XCTAssertEqual(model.path, "/home/research/results"); XCTAssertFalse(model.canWrite)
        await model.read(try XCTUnwrap(model.rows.first { !$0.directory }))
        let preview = try XCTUnwrap(model.preview)
        XCTAssertEqual(model.quote("A,pass", source: preview.content.path)?.source, "ssh://lab/home/research/results/QC.csv")
        XCTAssertNil(model.quote("invented", source: preview.content.path))
        do { try await model.save("changed", original: preview.content); XCTFail() } catch { }
        await model.chooseLocation("wsl:unknown"); XCTAssertEqual(model.contextID, "ssh:lab")
        let calls = await fake.recorded(); XCTAssertTrue(calls.allSatisfy { $0.0 == "research-1" && $0.2["session_id"] == .string("session-1") })
        XCTAssertEqual(calls.last?.2["context_id"], .string("ssh:lab"))
        XCTAssertFalse(calls.contains { $0.1.contains("set_context") || $0.1.contains("savefile") })
    }
    @MainActor func testSelectionCopyRulesSearchAndSortPreserveTheIntendedRows() async throws {
        let fake = FilesFake(try filesFixture()), model = model(fake); await model.open(); await model.navigate("results")
        model.toggleSelectionMode(); model.toggle(try XCTUnwrap(model.rows.first { $0.id == "results/figures" })); model.toggle(try XCTUnwrap(model.rows.first { $0.id == "results/QC.csv" }))
        let selected = model.selection; model.sort = .size; XCTAssertEqual(model.selection, selected)
        XCTAssertEqual(model.pathsForCopy(clicked: "results/QC.csv"), ["results/QC.csv", "results/figures"])
        XCTAssertEqual(model.pathsForCopy(clicked: "results/notes.md"), ["results/notes.md"])
        let relative = await model.copyPaths(absolute: false, clicked: "results/QC.csv"); XCTAssertEqual(relative, "results/QC.csv\nresults/figures")
        let absolute = await model.copyPaths(absolute: true, clicked: "results/QC.csv")
        XCTAssertEqual(absolute, "/work/research/branches/exploration-1/results/QC.csv\n/work/research/branches/exploration-1/results/figures")
        model.query = "QC"; XCTAssertTrue(model.selection.isEmpty); await model.search()
        XCTAssertEqual(model.rows.map(\.id), ["other/QC.csv", "results/QC.csv"])
        let calls = await fake.recorded(); let search = try XCTUnwrap(calls.last { $0.1.hasSuffix("searchfiles") })
        XCTAssertNil(search.2["path"])
        model.toggle(model.rows[0]); await model.navigate("."); XCTAssertTrue(model.selection.isEmpty)
        model.selectAll(); await model.chooseLocation("ssh:lab"); XCTAssertTrue(model.selection.isEmpty); XCTAssertFalse(model.selecting); XCTAssertEqual(model.query, "")
    }
    @MainActor func testLateCopiesCannotChangeClipboardAfterSelectionQueryLocationOrClosure() async throws {
        for change in ["selection", "query", "location", "close"] {
            let fake = FilesFake(try filesFixture()), model = model(fake); await model.open(); model.toggleSelectionMode(); model.selectAll()
            await fake.setup(hold: "native_conversation_panel_file_paths")
            let copy = Task { await model.copyPaths(absolute: true) }
            while !(await fake.waiting()) { await Task.yield() }
            switch change {
            case "selection": model.toggle(model.rows[0])
            case "query": model.query = "QC"
            case "location": await model.chooseLocation("ssh:lab")
            default: model.close()
            }
            await fake.release(); let value = await copy.value; XCTAssertNil(value, change); XCTAssertFalse(model.copyBusy)
        }
    }
    @MainActor func testLateDirectoryAndPreviewRepliesCannotRestorePreviousLocationOrClosedPanel() async throws {
        let fake = FilesFake(try filesFixture()), model = model(fake); await model.open()
        await fake.setup(hold: "native_conversation_panel_file_directory")
        let directory = Task { await model.navigate("results") }; while !(await fake.waiting()) { await Task.yield() }
        await fake.setup(); await model.chooseLocation("ssh:lab"); await fake.release(); await directory.value
        XCTAssertEqual(model.path, "/home/research/results")
        await fake.setup(hold: "native_conversation_panel_file_read")
        let row = try XCTUnwrap(model.rows.first { !$0.directory }); let read = Task { await model.read(row) }
        while !(await fake.waiting()) { await Task.yield() }; model.close(); await fake.release(); await read.value
        XCTAssertNil(model.preview); XCTAssertFalse(model.previewLoading); XCTAssertFalse(model.canWrite)
        let count = await fake.recorded().count
        await model.open(); await model.navigate("."); await model.read(row); await model.chooseLocation("local")
        let copied = await model.copyPaths(absolute: false, clicked: row.id); XCTAssertNil(copied)
        do { try await model.performAction(.delete, path: "results", newPath: nil); XCTFail() } catch { }
        let finalCount = await fake.recorded().count; XCTAssertEqual(count, finalCount)
    }
    @MainActor func testMalformedReadAndReadOnlySessionCannotEnableWritesOrReuseOldPreview() async throws {
        let fake = FilesFake(try filesFixture()), model = model(fake, readOnly: true); await model.open()
        XCTAssertFalse(model.canWrite)
        do { try await model.performAction(.delete, path: "results", newPath: nil); XCTFail() } catch { }
        await fake.setup(failure: "owner"); await model.read(try XCTUnwrap(model.rows.first { !$0.directory }))
        XCTAssertNil(model.preview); XCTAssertNotNil(model.error)
        let calls = await fake.recorded(); XCTAssertFalse(calls.contains { $0.1.hasSuffix("file_action") })
        let missing = self.model(fake); await missing.open(); XCTAssertNil(missing.catalog); XCTAssertFalse(missing.canWrite)
    }
    @MainActor func testLostActionAndSaveRepliesAreNotReplayedAndFreshReadsPermitNewDecisions() async throws {
        let fake = FilesFake(try filesFixture()), model = model(fake); await model.open()
        await fake.setup(failure: "lost")
        for _ in 0..<2 { do { try await model.performAction(.delete, path: "notes.md", newPath: nil); XCTFail() } catch { } }
        XCTAssertTrue(model.actionUnconfirmed); XCTAssertFalse(model.canWrite)
        await fake.setup(); await model.refresh(); XCTAssertFalse(model.actionUnconfirmed); XCTAssertTrue(model.canWrite)
        await model.read(try XCTUnwrap(model.rows.first { !$0.directory })); let original = try XCTUnwrap(model.preview?.content)
        await fake.setup(failure: "lost")
        for _ in 0..<2 { do { try await model.save("changed", original: original); XCTFail() } catch { } }
        XCTAssertTrue(model.saveUnconfirmed); XCTAssertEqual(model.preview?.content.text, original.text)
        let calls = await fake.recorded(); XCTAssertEqual(calls.filter { $0.1.hasSuffix("file_action") }.count, 1); XCTAssertEqual(calls.filter { $0.1.hasSuffix("savefile") }.count, 1)
        await fake.setup(); model.dismissPreview(); await model.read(try XCTUnwrap(model.rows.first { !$0.directory }))
        try await model.save("changed", original: try XCTUnwrap(model.preview?.content)); XCTAssertEqual(model.preview?.content.text, "changed")
    }
    @MainActor func testCancelledWritesAndClosedLegacyOperationsDoNotDispatch() async throws {
        let fake = FilesFake(try filesFixture()), model = model(fake); await model.open()
        let action = Task { try await model.performAction(.delete, path: "notes.md", newPath: nil) }; action.cancel(); _ = try? await action.value
        XCTAssertFalse(model.actionUnconfirmed); XCTAssertTrue(model.canWrite)
        await model.read(try XCTUnwrap(model.rows.first { !$0.directory })); let original = try XCTUnwrap(model.preview?.content)
        let save = Task { try await model.save("changed", original: original) }; save.cancel(); _ = try? await save.value
        XCTAssertFalse(model.saveUnconfirmed); XCTAssertEqual(model.preview?.content.text, original.text)
        model.legacy.close()
        do { _ = try await model.legacy.exportSource(path: "notes.md"); XCTFail() } catch { }
        do { try await model.legacy.performFileAction(.delete, path: "notes.md"); XCTFail() } catch { }
        let calls = await fake.recorded(); XCTAssertFalse(calls.contains { $0.1.hasSuffix("file_action") || $0.1.hasSuffix("savefile") || $0.1.hasSuffix("export") })
    }
    @MainActor func testChangedQueryDuringWriteCannotLeaveBusyStateOrReplayTheAction() async throws {
        let fake = FilesFake(try filesFixture()), model = model(fake); await model.open()
        await fake.setup(hold: "native_conversation_panel_file_action")
        let action = Task { try await model.performAction(.delete, path: "notes.md", newPath: nil) }
        while !(await fake.waiting()) { await Task.yield() }; model.query = "QC"
        await fake.release(); _ = try? await action.value
        XCTAssertFalse(model.fileActionBusy); XCTAssertTrue(model.actionUnconfirmed)
        do { try await model.performAction(.delete, path: "notes.md", newPath: nil); XCTFail() } catch { }
        await model.refresh(); XCTAssertFalse(model.actionUnconfirmed); XCTAssertTrue(model.canWrite)
        let calls = await fake.recorded(); XCTAssertEqual(calls.filter { $0.1.hasSuffix("file_action") }.count, 1)
    }
    @MainActor func testSupersededSearchAndCancelledReadCannotRestoreOldResults() async throws {
        let fake = FilesFake(try filesFixture()), model = model(fake); await model.open()
        model.query = "QC"; await fake.setup(hold: "native_conversation_panel_searchfiles")
        let search = Task { await model.search() }; while !(await fake.waiting()) { await Task.yield() }
        model.query = ""; await model.search(); await fake.release(); await search.value
        XCTAssertTrue(model.searchHits.isEmpty); XCTAssertFalse(model.searchLoading)
        await fake.setup(hold: "native_conversation_panel_file_read"); let row = try XCTUnwrap(model.rows.first { !$0.directory })
        let read = Task { await model.read(row) }; while !(await fake.waiting()) { await Task.yield() }
        read.cancel(); await fake.release(); await read.value
        XCTAssertNil(model.preview); XCTAssertNil(model.error); XCTAssertFalse(model.previewLoading)
    }
    func testBinaryPreviewUsesPrivateReturnedBytesAndRemovesItsTemporaryCopy() throws {
        var value = try filesFixture()["remote_preview"]["content"]
        value["path"] = .string("ssh://lab/results/image.png"); value["text"] = .null; value["base64"] = .string(Data([0, 1, 2, 3]).base64EncodedString())
        let content = try JSONDecoder().decode(NativePanelFileContent.self, from: JSONEncoder().encode(value))
        let file = try NativeFilePreviewBytes.materialize(content)
        defer { NativeFilePreviewBytes.remove(file) }
        XCTAssertEqual(file.lastPathComponent, "image.png"); XCTAssertEqual(try Data(contentsOf: file), Data([0, 1, 2, 3]))
        XCTAssertEqual(try FileManager.default.attributesOfItem(atPath: file.deletingLastPathComponent().path)[.posixPermissions] as? Int, 0o700)
        NativeFilePreviewBytes.remove(file); XCTAssertFalse(FileManager.default.fileExists(atPath: file.path))
        value["base64"] = .string("invalid base64"); XCTAssertThrowsError(try NativeFilePreviewBytes.materialize(JSONDecoder().decode(NativePanelFileContent.self, from: JSONEncoder().encode(value))))
    }
    @MainActor func testNativeMenuLabelsRetainVisibleSharedIconsInBothSchemes() throws {
        for name in ["plus", "more"] {
            let light = WispDesign.menuIcon(name, size: 16, scheme: .light), dark = WispDesign.menuIcon(name, size: 16, scheme: .dark)
            XCTAssertFalse(light.isTemplate); XCTAssertFalse(dark.isTemplate)
            let lightData = try XCTUnwrap(light.tiffRepresentation), darkData = try XCTUnwrap(dark.tiffRepresentation)
            XCTAssertNotEqual(lightData, darkData)
            let bitmap = try XCTUnwrap(NSBitmapImageRep(data: darkData))
            var visible = 0
            for x in 0..<bitmap.pixelsWide { for y in 0..<bitmap.pixelsHigh {
                if let pixel = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB), pixel.alphaComponent > 0.5, pixel.redComponent > 0.5 { visible += 1 }
            } }
            XCTAssertGreaterThan(visible, 0)
        }
    }
    @MainActor func testImmediateEscapeClosesSortBeforePanelWithoutMovingFocusOrWriting() async throws {
        _ = NSApplication.shared
        let fake = FilesFake(try filesFixture()), model = model(fake); await model.open(); model.sorting = true
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 419, height: 600), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close(); model.close() }
        var panelClosed = 0
        let host = NSHostingView(rootView: NativeFilesView(client: fake, projectID: "research-1", sessionID: "session-1", readOnly: false, model: model).background(NativeSettingsEscape { panelClosed += 1 }))
        window.contentView = host; host.frame = NSRect(x: 0, y: 0, width: 419, height: 600); host.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53))
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertFalse(model.sorting); XCTAssertEqual(panelClosed, 0); XCTAssertTrue(window.firstResponder === focus)
        host.layoutSubtreeIfNeeded(); XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertEqual(panelClosed, 1)
        let calls = await fake.recorded(); XCTAssertFalse(calls.contains { $0.1.hasSuffix("file_action") || $0.1.hasSuffix("savefile") })
    }
    @MainActor func testImmediateEscapeClosesFileActionBeforeParentWithoutFocusOrWrite() async throws {
        _ = NSApplication.shared
        let fake = FilesFake(try filesFixture()), model = model(fake); await model.open()
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 419, height: 550), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close(); model.close() }
        let root = NSView(frame: window.contentView!.bounds); window.contentView = root
        var panelClosed = 0, actionClosed = 0
        let parent = NSHostingView(rootView: Text("Files").background(NativeSettingsEscape { panelClosed += 1 }))
        parent.frame = root.bounds; root.addSubview(parent); parent.layoutSubtreeIfNeeded()
        let child = NSHostingView(rootView: NativeFileActionView(selection: .init(action: .rename, directory: "results", name: "QC.csv"), model: model.legacy, perform: { try await model.performAction($0, path: $1, newPath: $2) }) { actionClosed += 1 })
        child.frame = root.bounds; root.addSubview(child); child.layoutSubtreeIfNeeded(); let focus = window.firstResponder
        let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53))
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertEqual(actionClosed, 1); XCTAssertEqual(panelClosed, 0); XCTAssertTrue(window.firstResponder === focus)
        child.removeFromSuperview(); XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertEqual(panelClosed, 1)
        let calls = await fake.recorded(); XCTAssertFalse(calls.contains { $0.1.hasSuffix("file_action") })
    }
    @MainActor func testRenderLocalSelectionSortingAndRemoteLocationsInBothLocalesAndSchemes() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Set WISP_NATIVE_SNAPSHOT_DIR to render files") }
        let previous = UserDefaults.standard.object(forKey: "nativeSettings.locale")
        defer { if let previous { UserDefaults.standard.set(previous, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        for locale in ["zh", "en"] {
            UserDefaults.standard.set(locale, forKey: "nativeSettings.locale")
            for scheme in [ColorScheme.light, .dark] {
                for surface in ["local", "sort", "remote"] {
                    for width in [320.0, 419.0] {
                        let fake = FilesFake(try filesFixture()), model = model(fake); await model.open(); await model.navigate("results")
                        if surface == "remote" { await model.chooseLocation("ssh:lab") }
                        else { model.toggleSelectionMode(); model.selectAll(); model.sorting = surface == "sort" }
                        let host = NSHostingView(rootView: NativeFilesView(client: fake, projectID: "research-1", sessionID: "session-1", readOnly: false, model: model).padding(12).background(WispDesign.color("bg-sunken", scheme)).foregroundStyle(WispDesign.color("text", scheme)).tint(WispDesign.color("clay", scheme)).environment(\.colorScheme, scheme))
                        host.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
                        host.frame = NSRect(x: 0, y: 0, width: width, height: 550); host.layoutSubtreeIfNeeded()
                        let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds)); host.cacheDisplay(in: host.bounds, to: bitmap)
                        let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:])); XCTAssertGreaterThan(data.count, 1000)
                        try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent("files-\(surface)-\(locale)-\(scheme == .dark ? "dark" : "light")-\(Int(width)).png"))
                        model.close()
                    }
                }
            }
        }
    }
    @MainActor func testRenderScopedTextPreviewsAtNarrowWidthsInBothLocalesAndSchemes() throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Set WISP_NATIVE_SNAPSHOT_DIR to render previews") }
        let previous = UserDefaults.standard.object(forKey: "nativeSettings.locale")
        defer { if let previous { UserDefaults.standard.set(previous, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        for locale in ["zh", "en"] {
            UserDefaults.standard.set(locale, forKey: "nativeSettings.locale")
            for scheme in [ColorScheme.light, .dark] {
                for context in ["local", "remote"] {
                    let content = try JSONDecoder().decode(NativePanelFileContent.self, from: JSONEncoder().encode(try filesFixture()[context + "_preview"]["content"]))
                    for width in [320.0, 419.0] {
                        let host = NSHostingView(rootView: NativePanelFilePreview(content: content, close: {}, save: context == "local" ? { _ in } : nil).frame(maxWidth: .infinity, maxHeight: .infinity).background(WispDesign.color("bg-elev", scheme)).foregroundStyle(WispDesign.color("text", scheme)).tint(WispDesign.color("clay", scheme)).environment(\.colorScheme, scheme))
                        host.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua); host.frame = NSRect(x: 0, y: 0, width: width, height: 550); host.layoutSubtreeIfNeeded()
                        let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds)); host.cacheDisplay(in: host.bounds, to: bitmap)
                        let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:])); XCTAssertGreaterThan(data.count, 1000)
                        try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent("files-preview-\(context)-\(locale)-\(scheme == .dark ? "dark" : "light")-\(Int(width)).png"))
                    }
                }
            }
        }
    }
}
