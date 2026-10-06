import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private func transferFixture(_ name: String = "native-session-transfer/v1/move") throws -> SettingsValue {
    var root = URL(fileURLWithPath: #filePath); for _ in 0..<5 { root.deleteLastPathComponent() }
    return try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/\(name).json")))
}
private actor SessionTransferFake: NativeConversationQuerying {
    let fixture: SettingsValue
    var failure = "", hold = ""
    var held: CheckedContinuation<Void, Never>?
    var calls: [(String, String, [String: SettingsValue])] = []
    init(_ fixture: SettingsValue) { self.fixture = fixture }
    func setup(failure: String = "", hold: String = "") { self.failure = failure; self.hold = hold }
    func waiting() -> Bool { held != nil }
    func release() { held?.resume(); held = nil; hold = "" }
    func recorded() -> [(String, String, [String: SettingsValue])] { calls }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot {
        var value = try transferFixture("native-conversations/v1/snapshot")
        value["project_id"] = .string(projectID); value["session_id"] = .string(sessionID); value["running"] = .bool(false)
        value["composer_references"] = .bool(true)
        return try ConversationSnapshot.decode(value, projectID: projectID, sessionID: sessionID)
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        if command == "get_appearance_prefs" || command == "native_conversation_seen" { return .null }
        if command == "list_models" || command == "list_acp_agents" { return .array([]) }
        calls.append((projectID, command, args))
        var response = fixture[command == "native_conversation_transfer_preview" ? "preview" : "result"]
        response["project_id"] = .string(projectID); response["session_id"] = args["session_id"]!
        response["target_project_id"] = args["target_project_id"]!; response["mode"] = args["mode"]!
        if command == "native_conversation_transfer_preview", args["mode"] == .string("copy") { response["artifacts"] = .null }
        if command == "native_conversation_transfer" { response["include_artifacts"] = args["include_artifacts"]! }
        if command == hold { await withCheckedContinuation { held = $0 } }
        if command == "native_conversation_transfer", failure == "lost" { throw ProjectBrowserError.service("reply lost") }
        if command == "native_conversation_transfer", failure == "wrong-owner" { response["target_project_id"] = .string("wrong") }
        if failure == "malformed" { return .null }
        if command == "native_conversation_transfer_preview", failure == "artifacts" { response["artifacts"] = .null; response["artifact_error"] = .string("Target path conflicts") }
        return response
    }
}

final class NativeSessionTransferTests: XCTestCase {
    @MainActor func testCancelledTransferAfterDispatchCannotReplayButPrecancelledTaskDoesNotWrite() async throws {
        let fake = SessionTransferFake(try transferFixture()), model = model(fake)
        await model.readPreview()
        let before = Task { await model.confirm() }; before.cancel(); let noResult = await before.value
        XCTAssertNil(noResult); XCTAssertFalse(model.uncertain)
        await fake.setup(hold: "native_conversation_transfer"); let write = Task { await model.confirm() }
        while !(await fake.waiting()) { await Task.yield() }; write.cancel(); await fake.release(); let late = await write.value
        XCTAssertNil(late); XCTAssertNil(model.result); XCTAssertTrue(model.uncertain)
        await model.readPreview(); _ = await model.confirm()
        let calls = await fake.recorded(); XCTAssertEqual(calls.filter { $0.1 == "native_conversation_transfer" }.count, 1)
    }
    private var source: BrowserSession { .init(id: "source-1", projectID: "research-1", title: "Earlier sidebar title", ts: 1, status: "complete") }
    @MainActor private func model(_ fake: SessionTransferFake, mode: NativeSessionTransferMode = .move, writable: @escaping () -> Bool = { true }) -> NativeSessionTransferModel {
        NativeSessionTransferModel(client: fake, source: source, mode: mode, projects: ["research-1", "research-2", "research-3", "assistant:global"], writable: writable)
    }
    private func project(_ id: String, name: String) throws -> ProjectSummary {
        var value = try transferFixture("native-projects/v1/import")["result"]; value["id"] = .string(id); value["name"] = .string(name)
        return try NativeProjectCommand.summary(from: value)
    }
    func testSharedTransferContractsBindReviewedSourceDestinationRevisionAndFileChoice() throws {
        let fixture = try transferFixture()
        let preview = try NativeSessionTransferPreview.decode(fixture["preview"], project: "research-1", session: "source-1", target: "research-2", mode: .move)
        XCTAssertEqual(.object(preview.arguments(includeArtifacts: true)), fixture["args"])
        XCTAssertEqual(try NativeSessionTransferResult.decode(fixture["result"], reviewed: preview, includeArtifacts: true).frame_id, "moved-1")
        for (field, value) in [("project_id", SettingsValue.string("wrong")), ("session_id", .string("wrong")), ("target_project_id", .string("research-1")), ("mode", .string("copy")), ("revision", .string("bad")), ("message_count", .integer(-1)), ("artifacts", .null), ("artifact_error", .string("Unexpected"))] {
            var bad = fixture["preview"]; bad[field] = value
            XCTAssertThrowsError(try NativeSessionTransferPreview.decode(bad, project: "research-1", session: "source-1", target: "research-2", mode: .move))
        }
        var bad = fixture["preview"]; bad["artifacts"]["retained"] = .array([.object(["name": .string("Reference"), "reason": .string("unknown")])])
        XCTAssertThrowsError(try NativeSessionTransferPreview.decode(bad, project: "research-1", session: "source-1", target: "research-2", mode: .move))
        for (field, value) in [("project_id", SettingsValue.string("wrong")), ("session_id", .string("wrong")), ("target_project_id", .string("wrong")), ("mode", .string("copy")), ("include_artifacts", .bool(false)), ("frame_id", .string("source-1")), ("frame_id", .string(""))] {
            var bad = fixture["result"]; bad[field] = value
            XCTAssertThrowsError(try NativeSessionTransferResult.decode(bad, reviewed: preview, includeArtifacts: true))
        }
    }
    @MainActor func testFreshPreviewAndOneShotMoveUseSelectedProjectAndExactFingerprint() async throws {
        let fake = SessionTransferFake(try transferFixture()); var writable = true; let model = model(fake, writable: { writable })
        XCTAssertEqual(model.target, "research-2"); model.select("research-1"); model.select("assistant:global"); XCTAssertEqual(model.target, "research-2")
        await model.readPreview(); XCTAssertTrue(model.canTransfer); XCTAssertFalse(model.includeArtifacts)
        XCTAssertEqual(model.preview?.title, "RNA-seq comparison"); XCTAssertNotEqual(model.preview?.title, model.source.title)
        model.chooseArtifacts(true); writable = false; let refused = await model.confirm(); XCTAssertNil(refused)
        writable = true; let result = await model.confirm(); XCTAssertEqual(result?.frame_id, "moved-1"); XCTAssertFalse(model.canTransfer)
        _ = await model.confirm(); model.select("research-3"); await model.readPreview(); XCTAssertEqual(model.target, "research-2")
        let calls = await fake.recorded(); XCTAssertEqual(calls.count, 2); XCTAssertEqual(calls[1].0, "research-1")
        XCTAssertEqual(.object(calls[1].2), try transferFixture()["args"])
    }
    @MainActor func testChangingTargetInvalidatesPreviewAndFileChoiceAndCopyNeverMovesFiles() async throws {
        let fake = SessionTransferFake(try transferFixture()), move = model(fake)
        await move.readPreview(); move.chooseArtifacts(true); move.select("research-3")
        XCTAssertNil(move.preview); XCTAssertFalse(move.includeArtifacts); XCTAssertFalse(move.canTransfer)
        await move.readPreview(); _ = await move.confirm()
        let copy = model(fake, mode: .copy); await copy.readPreview(); copy.chooseArtifacts(true); XCTAssertFalse(copy.includeArtifacts)
        _ = await copy.confirm(); let calls = await fake.recorded().filter { $0.1 == "native_conversation_transfer" }
        XCTAssertEqual(calls.count, 2); XCTAssertEqual(calls[0].2["target_project_id"], .string("research-3"))
        XCTAssertEqual(calls[1].2["mode"], .string("copy")); XCTAssertEqual(calls[1].2["artifact_fingerprint"], .null)
        XCTAssertEqual(calls[1].2["include_artifacts"], .bool(false))
    }
    @MainActor func testArtifactPreviewFailureStillAllowsTranscriptOnlyAndMalformedPreviewDoesNot() async throws {
        let fake = SessionTransferFake(try transferFixture()), model = model(fake)
        await fake.setup(failure: "artifacts"); await model.readPreview(); XCTAssertNil(model.preview?.artifacts); XCTAssertNotNil(model.preview?.artifact_error)
        model.chooseArtifacts(true); XCTAssertFalse(model.includeArtifacts); XCTAssertTrue(model.canTransfer); _ = await model.confirm()
        let calls = await fake.recorded(); XCTAssertEqual(calls[1].2["artifact_fingerprint"], .null)
        let bad = self.model(fake); await fake.setup(failure: "malformed"); await bad.readPreview()
        XCTAssertNil(bad.preview); XCTAssertNotNil(bad.error); XCTAssertFalse(bad.canTransfer); XCTAssertFalse(bad.uncertain)
    }
    @MainActor func testUnconfirmedTransfersNeverReplayAfterRefreshOrTargetChange() async throws {
        for failure in ["lost", "wrong-owner", "malformed"] {
            let fake = SessionTransferFake(try transferFixture()), model = model(fake)
            await model.readPreview(); await fake.setup(failure: failure); _ = await model.confirm()
            XCTAssertTrue(model.uncertain); XCTAssertNotNil(model.error); XCTAssertNil(model.result)
            await fake.setup(); await model.readPreview(); model.select("research-3"); await model.readPreview(); _ = await model.confirm()
            XCTAssertTrue(model.uncertain); XCTAssertFalse(model.canTransfer)
            let calls = await fake.recorded(); XCTAssertEqual(calls.filter { $0.1 == "native_conversation_transfer" }.count, 1)
        }
    }
    @MainActor func testSupersededCancelledAndClosedResponsesCannotChangePreviewOrReturnNavigation() async throws {
        let fake = SessionTransferFake(try transferFixture()), model = model(fake)
        await fake.setup(hold: "native_conversation_transfer_preview"); let old = Task { await model.readPreview() }
        while !(await fake.waiting()) { await Task.yield() }; model.select("research-3"); await fake.setup(); await model.readPreview()
        await fake.release(); await old.value; XCTAssertEqual(model.preview?.target_project_id, "research-3")
        await fake.setup(hold: "native_conversation_transfer_preview"); let cancelled = Task { await model.readPreview() }
        while !(await fake.waiting()) { await Task.yield() }; cancelled.cancel(); await fake.release(); await cancelled.value
        XCTAssertNil(model.preview); XCTAssertNil(model.error); XCTAssertFalse(model.reading)
        await model.readPreview(); await fake.setup(hold: "native_conversation_transfer"); let transfer = Task { await model.confirm() }
        while !(await fake.waiting()) { await Task.yield() }; XCTAssertTrue(model.transferring)
        _ = await model.confirm(); model.select("research-2"); XCTAssertEqual(model.target, "research-3")
        model.close(); await fake.release(); let late = await transfer.value
        XCTAssertNil(late); XCTAssertNil(model.result); XCTAssertFalse(model.canTransfer)
        let calls = await fake.recorded(); XCTAssertEqual(calls.filter { $0.1 == "native_conversation_transfer" }.count, 1)
    }
    @MainActor func testConfirmedMoveRetainsTextDraftInDestinationWithoutDispatchingAndKeepsOriginal() async throws {
        let fake = SessionTransferFake(try transferFixture()), conversation = NativeConversationModel(client: fake)
        await conversation.open(project: "research-1", session: "source-1"); conversation.draft = "Unsent research question"
        let preview = try NativeSessionTransferPreview.decode(try transferFixture()["preview"], project: "research-1", session: "source-1", target: "research-2", mode: .move)
        let result = try NativeSessionTransferResult.decode(try transferFixture()["result"], reviewed: preview, includeArtifacts: true)
        conversation.retainDraftForTransferredSession(result); XCTAssertEqual(conversation.draft, "Unsent research question")
        await conversation.open(project: "research-2", session: "moved-1"); XCTAssertEqual(conversation.draft, "Unsent research question")
        await conversation.open(project: "research-1", session: "source-1"); XCTAssertEqual(conversation.draft, "Unsent research question")
        conversation.pause(); let calls = await fake.recorded(); XCTAssertTrue(calls.isEmpty)
    }
    @MainActor func testNoDestinationAndClosedPreviewCannotEnableOrDispatchTransfer() async throws {
        let fake = SessionTransferFake(try transferFixture())
        let empty = NativeSessionTransferModel(client: fake, source: source, mode: .move, projects: [source.projectID, "assistant:global"], writable: { true })
        await empty.readPreview(); XCTAssertTrue(empty.target.isEmpty); XCTAssertFalse(empty.canTransfer)
        let refused = await empty.confirm(); XCTAssertNil(refused); let initial = await fake.recorded(); XCTAssertTrue(initial.isEmpty)
        let model = self.model(fake); await fake.setup(hold: "native_conversation_transfer_preview")
        let read = Task { await model.readPreview() }; while !(await fake.waiting()) { await Task.yield() }
        model.close(); await fake.release(); await read.value
        XCTAssertNil(model.preview); XCTAssertNil(model.error); XCTAssertFalse(model.reading); XCTAssertFalse(model.canTransfer)
    }
    @MainActor func testImmediateEscapeClosesTargetPickerBeforeSheetWithoutFocusChangeOrTransfer() async throws {
        _ = NSApplication.shared
        let fake = SessionTransferFake(try transferFixture()), model = model(fake), projects = [try project("research-2", name: "Destination")]
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 419, height: 540), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let root = NSView(frame: window.contentView!.bounds); window.contentView = root
        var parentClosed = 0, pickerClosed = 0, opened = 0
        let parent = NSHostingView(rootView: NativeSessionTransferSheet(model: model, projects: projects, close: { _ in parentClosed += 1 }, open: { _ in opened += 1 }))
        parent.frame = root.bounds; root.addSubview(parent); parent.layoutSubtreeIfNeeded()
        let child = NSHostingView(rootView: NativeSessionImportProjectPicker(projects: projects, selected: "research-2", close: { pickerClosed += 1 }, select: { _ in opened += 1 }))
        child.frame = root.bounds; root.addSubview(child); child.layoutSubtreeIfNeeded(); let focus = window.firstResponder
        let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53))
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertEqual(pickerClosed, 1); XCTAssertEqual(parentClosed, 0); XCTAssertEqual(opened, 0); XCTAssertTrue(window.firstResponder === focus)
        child.removeFromSuperview(); XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertEqual(parentClosed, 1)
    }
    @MainActor func testTransferPaletteCommandsRequireSessionScopeAndDistinctIcons() {
        XCTAssertEqual(NativeSearchCommand.matching(">copy session project", project: true, session: true).map(\.id), ["copy-session-project"])
        XCTAssertEqual(NativeSearchCommand.matching(">move session project", project: true, session: true).map(\.id), ["move-session-project"])
        XCTAssertTrue(NativeSearchCommand.matching(">copy session project", project: true, session: false).isEmpty)
        XCTAssertEqual(Set(NativeSearchCommand.all.map(\.icon)).count, NativeSearchCommand.all.count)
    }
    @MainActor func testRenderTransferPreviewCopyAndResultAtNarrowWidthInBothLocalesAndSchemes() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Set WISP_NATIVE_SNAPSHOT_DIR to render transfer") }
        let previous = UserDefaults.standard.object(forKey: "nativeSettings.locale")
        defer { if let previous { UserDefaults.standard.set(previous, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        let projects = [try project("research-1", name: "Research Alpha"), try project("research-2", name: "Research Beta")]
        for locale in ["zh", "en"] {
            UserDefaults.standard.set(locale, forKey: "nativeSettings.locale")
            for scheme in [ColorScheme.light, .dark] {
                for surface in ["preview", "result", "copy", "artifact-error", "uncertain"] {
                    let fake = SessionTransferFake(try transferFixture()), model = model(fake, mode: surface == "copy" ? .copy : .move)
                    if surface == "artifact-error" { await fake.setup(failure: "artifacts") }
                    await model.readPreview(); model.chooseArtifacts(true); if surface == "result" { _ = await model.confirm() }
                    if surface == "uncertain" { await fake.setup(failure: "lost"); _ = await model.confirm(); await fake.setup(); await model.readPreview() }
                    let host = NSHostingView(rootView: NativeSessionTransferSheet(model: model, projects: projects, close: { _ in }, open: { _ in }).frame(maxWidth: .infinity, maxHeight: .infinity).background(WispDesign.color("bg-app", scheme)).foregroundStyle(WispDesign.color("text", scheme)).tint(WispDesign.color("clay", scheme)).environment(\.colorScheme, scheme))
                    host.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua); host.frame = NSRect(x: 0, y: 0, width: 419, height: 600); host.layoutSubtreeIfNeeded()
                    let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds)); host.cacheDisplay(in: host.bounds, to: bitmap)
                    let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:])); XCTAssertGreaterThan(data.count, 1000)
                    try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent("session-transfer-\(surface)-\(locale)-\(scheme == .dark ? "dark" : "light").png"))
                }
            }
        }
    }
}
