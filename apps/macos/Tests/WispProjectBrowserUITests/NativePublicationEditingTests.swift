import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

final class NativePublicationEditingTests: XCTestCase {
    private func expectTrue(_ value: Bool, file: StaticString = #filePath, line: UInt = #line) { XCTAssertTrue(value, file: file, line: line) }
    private func expectFalse(_ value: Bool, file: StaticString = #filePath, line: UInt = #line) { XCTAssertFalse(value, file: file, line: line) }
    private func expectEqual<T: Equatable>(_ actual: T, _ expected: T, file: StaticString = #filePath, line: UInt = #line) { XCTAssertEqual(actual, expected, file: file, line: line) }
    func testEveryNativeMutationMatchesTheSharedRustFixture() throws {
        let expected = try EditingTransport.fixture("mutations.json")["operations"].array
        var item = PublicationItemDraft(); item.id = "item-1"; item.kind = "claim"; item.title = "中文 claim"; item.content = "Evidence 🌱"; item.ordinal = 2
        let evidence = PublicationEvidenceDraft(sourceKind: "artifact_version", sourceID: "version-17", itemID: "item-1", claimID: "item-1", purpose: "Support the result")
        let policy = PublicationFreezePolicy(personalDataReviewed: true)
        let operations: [PublicationOperation] = [.saveItem(item), .bindEvidence(evidence), .updateBinding("binding-1", "rejected", "restricted"),
            .cloneRevision("v2"), .saveWaiver("license", "Reviewer", "Permission recorded"), .check(policy), .freeze(policy), .verify("run-1"), .buildCapsule("/tmp/capsule.zip")]
        XCTAssertEqual(operations.map { SettingsValue.object($0.fields) }, expected)
    }

    func testMessageEvidenceUsesExactUTF8OffsetsAndCanonicalKeys() throws {
        let source = PublicationSourceRecord(kind: "message_span", id: "frame/7", title: "消息", detail: "", text: "A水稻🌱结果", textSHA256: String(repeating: "a", count: 64), frameID: "frame/1", messageSeq: 7)
        let reference = try source.evidence(utf16Range: NSRange(location: 1, length: 4))
        XCTAssertEqual(reference.kind, "message_span")
        XCTAssertEqual(reference.id, "{\"byte_end\":11,\"byte_start\":1,\"frame_id\":\"frame/1\",\"message_content_sha256\":\"\(String(repeating: "a", count: 64))\",\"message_seq\":7}")
        XCTAssertEqual(try source.evidence().id.decodedLocator["byte_end"], .integer(17))
        XCTAssertThrowsError(try source.evidence(utf16Range: NSRange(location: 4, length: 1))) // half of the emoji
        XCTAssertThrowsError(try source.evidence(utf16Range: NSRange(location: 0, length: 0)))
        XCTAssertThrowsError(try source.evidence(utf16Range: NSRange(location: Int.max, length: 2)))
        var stale = source; stale.textSHA256 = nil
        XCTAssertThrowsError(try stale.evidence())
        var tooLarge = source; tooLarge.text = String(repeating: "A", count: 65537)
        XCTAssertThrowsError(try tooLarge.evidence())
    }

    func testFrozenReadinessReportsItsRecordInsteadOfOfferingAnotherFreeze() throws {
        let readiness = try JSONDecoder().decode(PublicationReadinessRecord.self, from: JSONEncoder().encode(EditingTransport.readiness))
        XCTAssertEqual(NativePublicationReadiness(readiness: readiness).statusTitle, "检查通过，可冻结")
        XCTAssertEqual(NativePublicationReadiness(readiness: readiness, frozen: true).statusTitle, "冻结检查记录")
    }

    @MainActor func testMutationsKeepProjectRevisionAndFrozenGuards() async throws {
        let host = try EditingTransport()
        let model = NativePublicationModel(); model.open(projectID: "research-1"); await model.reload(host)
        var item = PublicationItemDraft(item: model.workspace.items.first); item.title = "Updated"
        let foreign = await model.mutate(.saveItem(item), revisionID: "foreign", client: host)
        XCTAssertFalse(foreign)
        expectEqual(await host.count(), 1)
        expectTrue(await model.mutate(.saveItem(item), revisionID: "rev-1", client: host))
        let call = await host.last()
        XCTAssertEqual(call.command, NativePublicationCommand.mutate)
        XCTAssertEqual(call.project, "research-1")
        XCTAssertEqual(call.args["revision_id"], .string("rev-1"))
        XCTAssertEqual(call.args["operation"]?["title"], .string("Updated"))
        await host.setState("frozen"); await model.reload(host)
        XCTAssertFalse(model.editable)
        expectFalse(await model.mutate(.saveItem(item), revisionID: "rev-1", client: host))
        let before = await host.count()
        expectTrue(await model.mutate(.buildCapsule("/tmp/capsule.zip"), revisionID: "rev-1", client: host))
        expectEqual(await host.count(), before + 1)
    }

    @MainActor func testLostMutationCannotReplayEvenAfterClosingAndReopening() async throws {
        let host = try EditingTransport()
        let model = NativePublicationModel(); model.open(projectID: "research-1"); await model.reload(host)
        var item = PublicationItemDraft(item: model.workspace.items.first); item.title = "Updated"
        await host.failNext()
        expectFalse(await model.mutate(.saveItem(item), revisionID: "rev-1", client: host))
        XCTAssertTrue(model.uncertain)
        model.acknowledgeResult(); XCTAssertTrue(model.uncertain)
        let before = await host.count()
        expectFalse(await model.mutate(.saveItem(item), revisionID: "rev-1", client: host))
        expectEqual(await host.count(), before)
        model.dismiss(); model.open(projectID: "research-1")
        XCTAssertTrue(model.uncertain)
        await model.reload(host)
        XCTAssertTrue(model.canAcknowledge)
        model.acknowledgeResult()
        XCTAssertTrue(model.editable)
    }

    @MainActor func testPendingWriteAndLateFailureStayWithTheirOriginalProject() async throws {
        let host = try EditingTransport()
        let model = NativePublicationModel(); model.open(projectID: "research-1"); await model.reload(host)
        await host.suspendNext()
        let task = Task { await model.mutate(.cloneRevision("v2"), revisionID: "rev-1", client: host) }
        while !(await host.isHanging()) { await Task.yield() }
        model.open(projectID: "research-2")
        XCTAssertFalse(model.uncertain)
        await host.failSuspended()
        expectFalse(await task.value)
        XCTAssertNil(model.error)
        XCTAssertTrue(model.workspace.publications.isEmpty)
        model.open(projectID: "research-1")
        XCTAssertTrue(model.uncertain)
        await model.reload(host)
        model.acknowledgeResult()
        XCTAssertTrue(model.editable)
    }

    @MainActor func testRejectsWrongMutationScopeAndReadinessAndClearsReadResults() async throws {
        let host = try EditingTransport()
        let model = NativePublicationModel(); model.open(projectID: "research-1"); await model.reload(host)
        var readiness = EditingTransport.readiness
        readiness["revision_id"] = .string("foreign")
        await host.setReadiness(readiness)
        expectFalse(await model.mutate(.check(PublicationFreezePolicy()), revisionID: "rev-1", client: host))
        XCTAssertTrue(model.uncertain)
        XCTAssertNil(model.workspace.readiness)
        await host.setReadiness(EditingTransport.readiness)
        await model.reload(host); model.acknowledgeResult()
        expectTrue(await model.mutate(.check(PublicationFreezePolicy()), revisionID: "rev-1", client: host))
        XCTAssertEqual(model.workspace.readiness?.revisionID, "rev-1")
        XCTAssertTrue(model.workspace.readiness?.canFreeze == true)
        await host.failNext(); await model.reload(host)
        XCTAssertNil(model.workspace.readiness)
        XCTAssertTrue(model.workspace.lineage.isEmpty)
    }

    @MainActor func testPaperDraftsAreSeparatePerProject() {
        let model = NativePublicationModel(); model.open(projectID: "a"); model.draft.title = "Paper A"
        model.open(projectID: "b"); XCTAssertEqual(model.draft.title, "")
        model.draft.title = "Paper B"
        model.open(projectID: "a"); XCTAssertEqual(model.draft.title, "Paper A")
    }

    @MainActor func testUnconfirmedWritesAndDraftsDoNotCrossDatabasesWithTheSameProjectID() async throws {
        let host = try EditingTransport()
        let model = NativePublicationModel()
        let first = URL(fileURLWithPath: "/unused/a.sqlite"); let second = URL(fileURLWithPath: "/unused/b.sqlite")
        model.open(projectID: "research-1", databaseURL: first)
        model.draft = PublicationDraft(title: "Paper A", description: "", revisionLabel: "v1")
        await host.failNext(); _ = await model.create(host)
        XCTAssertTrue(model.uncertain)
        model.open(projectID: "research-1", databaseURL: second)
        XCTAssertFalse(model.uncertain); XCTAssertEqual(model.draft.title, "")
        model.draft.title = "Paper B"
        model.open(projectID: "research-1", databaseURL: first)
        XCTAssertTrue(model.uncertain); XCTAssertEqual(model.draft.title, "Paper A")
    }

    @MainActor func testAnEditorFromAnotherDatabaseCannotWriteReusedProjectAndRevisionIDs() async throws {
        let host = try EditingTransport(); let model = NativePublicationModel()
        model.open(projectID: "research-1", databaseURL: URL(fileURLWithPath: "/unused/a.sqlite"))
        let oldScope = model.scopeIdentity
        model.open(projectID: "research-1", databaseURL: URL(fileURLWithPath: "/unused/b.sqlite"))
        await model.reload(host)
        model.draft = PublicationDraft(title: "Paper B", description: "", revisionLabel: "v1")
        let before = await host.count()
        expectFalse(await model.create(host, expectedScope: oldScope))
        expectFalse(await model.mutate(.check(PublicationFreezePolicy()), revisionID: "rev-1", client: host, expectedScope: oldScope))
        expectEqual(await host.count(), before)
        XCTAssertFalse(model.uncertain); XCTAssertEqual(model.draft.title, "Paper B")
    }

    @MainActor func testLateConfirmedCreateClearsOnlyItsSubmittedDraft() async throws {
        let host = try EditingTransport()
        let model = NativePublicationModel(); model.open(projectID: "research-1")
        model.draft = PublicationDraft(title: "Paper A", description: "", revisionLabel: "v1")
        await host.suspendNext()
        let task = Task { await model.create(host) }
        while !(await host.isHanging()) { await Task.yield() }
        model.open(projectID: "research-2"); model.draft.title = "Paper B"
        await host.resumeWorkspace()
        expectFalse(await task.value)
        XCTAssertEqual(model.projectID, "research-2"); XCTAssertEqual(model.draft.title, "Paper B")
        model.open(projectID: "research-1")
        XCTAssertEqual(model.draft.title, ""); XCTAssertFalse(model.uncertain)
    }

    @MainActor func testPendingWriteCannotBeAcknowledgedOrSubmittedAgain() async throws {
        let host = try EditingTransport()
        let model = NativePublicationModel(); model.open(projectID: "research-1"); await model.reload(host)
        await host.suspendNext()
        let task = Task { await model.mutate(.check(PublicationFreezePolicy()), revisionID: "rev-1", client: host) }
        while !(await host.isHanging()) { await Task.yield() }
        let count = await host.count()
        expectFalse(await model.mutate(.check(PublicationFreezePolicy()), revisionID: "rev-1", client: host))
        expectEqual(await host.count(), count)
        model.open(projectID: "research-1"); await model.reload(host)
        XCTAssertFalse(model.canAcknowledge)
        model.acknowledgeResult(); XCTAssertTrue(model.uncertain)
        await host.failSuspended(); _ = await task.value
        XCTAssertTrue(model.uncertain)
    }

    @MainActor func testSourcePagingAndDismissDiscardLateReplies() async throws {
        let host = try EditingTransport()
        let model = PublicationSourceModel(); model.kind = "messages"; model.query = "稻"
        await model.load(projectID: "research-1", client: host, offset: 50)
        let call = await host.last()
        XCTAssertEqual(call.args["offset"], .integer(50)); XCTAssertEqual(call.project, "research-1")
        XCTAssertEqual(call.args["query"], .string("稻"))
        await host.suspendNext()
        let task = Task { await model.load(projectID: "research-1", client: host) }
        while !(await host.isHanging()) { await Task.yield() }
        model.invalidate()
        await host.resumeSources()
        await task.value
        XCTAssertFalse(model.busy); XCTAssertTrue(model.sources.isEmpty); XCTAssertNil(model.selected)
    }

    @MainActor func testImmediateEscapeClosesOnlyPristineEditorAndKeepsPublicationOpen() async throws {
        _ = NSApplication.shared
        let client = try EditingTransport()
        let model = NativePublicationModel(); model.open(projectID: "research-1"); await model.reload(client)
        var editorOpen = true
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 660), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let root = NSView(frame: window.contentLayoutRect); window.contentView = root
        let parentView = NSView(frame: .zero); root.addSubview(parentView)
        let parent = NativeSettingsEscape.Coordinator(enabled: true) { model.dismiss() }; parent.view = parentView; parent.install(); defer { parent.remove() }
        let host = NSHostingView(rootView: NativePublicationEditor(publication: model, client: client,
            target: PublicationEditorTarget(kind: .item(nil), page: model.workspace)) { editorOpen = false })
        root.addSubview(host); host.frame = root.bounds; host.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertFalse(editorOpen); XCTAssertTrue(model.presented); XCTAssertTrue(window.firstResponder === focus)
    }

    @MainActor func testRenderPublicationEditorsInBothLocalesAndSchemes() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Opt-in native publication rendering") }
        try FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
        let previous = UserDefaults.standard.object(forKey: "nativeSettings.locale")
        defer { if let previous { UserDefaults.standard.set(previous, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        let client = try EditingTransport()
        await client.setSourceCount(50)
        let browser = ProjectBrowserModel(client: client, databaseURL: URL(fileURLWithPath: "/unused/render.sqlite"), projectTransport: client)
        let model = browser.publication; model.open(projectID: "research-1"); await model.reload(client)
        await client.setReadiness(EditingTransport.readiness)
        _ = await model.mutate(.check(PublicationFreezePolicy()), revisionID: "rev-1", client: client)
        for locale in ["zh", "en"] {
            UserDefaults.standard.set(locale, forKey: "nativeSettings.locale")
            for scheme in [ColorScheme.light, .dark] {
                for width: CGFloat in [432, 816] {
                    _ = await model.mutate(.check(PublicationFreezePolicy()), revisionID: "rev-1", client: client)
                    XCTAssertNotNil(model.workspace.readiness)
                    let suffix = "\(locale)-\(scheme == .dark ? "dark" : "light")-\(Int(width))"
                    for kind in [PublicationEditorKind.create, .item(model.workspace.items.first), .evidence, .clone, .readiness, .reproduction] {
                        let target = PublicationEditorTarget(kind: kind, page: model.workspace)
                        try await render(NativePublicationEditor(publication: model, client: client, target: target) {},
                            width: width, scheme: scheme, name: "publication-editor-\(kind.title)-\(suffix)", directory: directory)
                    }
                    try await render(NativePublicationColumn(model: browser, publication: model), width: width, scheme: scheme,
                        name: "publication-column-\(suffix)", directory: directory)
                }
            }
        }
    }

    @MainActor private func render<V: View>(_ view: V, width: CGFloat, scheme: ColorScheme, name: String, directory: String) async throws {
        let host = NSHostingView(rootView: view.environment(\.colorScheme, scheme).tint(WispDesign.color("clay", scheme))
            .background(WispDesign.color("bg-app", scheme)).foregroundStyle(WispDesign.color("text", scheme)))
        host.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
        host.frame = NSRect(x: 0, y: 0, width: width, height: 720)
        host.layoutSubtreeIfNeeded()
        try await Task.sleep(nanoseconds: 40_000_000)
        host.layoutSubtreeIfNeeded()
        let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: bitmap)
        let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        XCTAssertGreaterThan(data.count, 1000)
        try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent(name + ".png"))
    }
}

private extension String {
    var decodedLocator: SettingsValue { (try? JSONDecoder().decode(SettingsValue.self, from: Data(utf8))) ?? .null }
}

private actor EditingTransport: NativeSettingsQuerying, ProjectBrowserQuerying {
    struct Call { var command: String; var args: [String: SettingsValue]; var project: String? }
    private var calls: [Call] = []
    private var page: SettingsValue
    private var readiness: SettingsValue?
    private var fail = false
    private var suspend = false
    private var release: CheckedContinuation<SettingsValue, Error>?
    private var sourceCount = 1
    init() throws { page = try Self.fixture("workspace-evidence.json")["result"] }
    func count() -> Int { calls.count }
    func last() -> Call { calls.last! }
    func failNext() { fail = true }
    func suspendNext() { suspend = true }
    func isHanging() -> Bool { release != nil }
    func failSuspended() { release?.resume(throwing: ProjectBrowserError.service("lost reply")); release = nil }
    func resumeSources() { release?.resume(returning: .object(["sources": .array([]), "has_more": .bool(false)])); release = nil }
    func resumeWorkspace() { release?.resume(returning: page); release = nil }
    func setState(_ state: String) { var revision = page["revision"]; revision["state"] = .string(state); page["revision"] = revision }
    func setReadiness(_ readiness: SettingsValue) { self.readiness = readiness }
    func setSourceCount(_ count: Int) { sourceCount = count }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        calls.append(Call(command: command, args: args, project: projectID))
        if fail { fail = false; throw ProjectBrowserError.service("lost reply") }
        if suspend { suspend = false; return try await withCheckedThrowingContinuation { release = $0 } }
        if command == NativePublicationCommand.sources {
            let kind = args["kind"]?.string ?? "files"
            var source: SettingsValue = .object(["kind": .string(kind == "files" ? "artifact_version" : kind == "runs" ? "run" : "message_span"),
                "id": .string("exact-source"), "title": .string("Differential expression evidence / 稻穗分析"), "detail": .string("17")])
            if kind == "messages" {
                source["text"] = .string("A水稻🌱结果 — exact evidence excerpt")
                source["text_sha256"] = .string(String(repeating: "a", count: 64))
                source["frame_id"] = .string("frame-1"); source["message_seq"] = .integer(7)
            }
            let records = (0..<sourceCount).map { index -> SettingsValue in
                var copy = source; copy["id"] = .string("exact-source-\(index)"); return copy
            }
            return .object(["sources": .array(records), "has_more": .bool(true)])
        }
        if command == NativePublicationCommand.mutate { return .object(["workspace": page, "readiness": readiness ?? .null]) }
        return page
    }
    nonisolated static var readiness: SettingsValue {
        .object(["revision_id": .string("rev-1"), "target_visibility": .string("private"), "capability_level": .string("archived"),
            "blockers": .array([]), "warnings": .array([]), "omissions": .array([]), "manifest_sha256": .string(String(repeating: "a", count: 64)), "can_freeze": .bool(true)])
    }
    nonisolated static func fixture(_ file: String) throws -> SettingsValue {
        var root = URL(fileURLWithPath: #filePath); for _ in 0..<5 { root.deleteLastPathComponent() }
        return try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-publication/v1/" + file)))
    }
    func listProjects(databaseURL: URL) async throws -> ProjectListSnapshot { ProjectListSnapshot(projects: [], activitySource: "persisted_only") }
    func listSessions(databaseURL: URL, projectID: String?) async throws -> [BrowserSession] { [] }
    func transcript(databaseURL: URL, projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> TranscriptPage { TranscriptPage(messages: [], nextBeforeSeq: nil) }
}
