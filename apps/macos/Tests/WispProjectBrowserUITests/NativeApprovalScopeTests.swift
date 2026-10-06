import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor ScopeClient: NativeConversationQuerying {
    var sequence: Int64 = 1
    var scopes: [String]? = ["once", "session", "project", "global"]
    var readOnly = false, fail = false, consumed = false
    var id = "approval"
    var writes: [(String, [String: SettingsValue])] = []
    func configure(scopes: [String]? = ["once", "session", "project", "global"], readOnly: Bool = false, fail: Bool = false, id: String = "approval") {
        self.scopes = scopes; self.readOnly = readOnly; self.fail = fail; self.id = id; consumed = false
    }
    func recorded() -> [(String, [String: SettingsValue])] { writes }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot {
        sequence += 1
        var value: SettingsValue = .object([
            "schema": .string(ConversationSnapshot.schemaID), "epoch": .string("host"), "sequence": .integer(sequence),
            "project_id": .string(projectID), "session_id": .string(sessionID), "items": .array([]),
            "running": .bool(true), "stopping": .bool(false), "read_only": .bool(readOnly), "model_id": .string("model"),
            "approvals": consumed ? .array([]) : .array([.object(["approval_id": .string(id), "frame_id": .string(sessionID), "message": .string("Run this command?"), "tool": .string("shell"), "preview": .string("python analysis.py")])])
        ])
        if let scopes, !consumed { value["approval_scopes"] = .object([id: .array(scopes.map(SettingsValue.string))]) }
        return try ConversationSnapshot.decode(value, projectID: projectID, sessionID: sessionID)
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        if command == "native_conversation_approve" {
            writes.append((projectID, args))
            if fail { throw ProjectBrowserError.service("unknown approval result") }
            consumed = true
        }
        return .null
    }
}

final class NativeApprovalScopeTests: XCTestCase {
    @MainActor func testEachAdvertisedScopeUsesExactApprovalAndOwnerWithoutChangingDraft() async throws {
        for scope in ["once", "session", "project", "global"] {
            let fake = ScopeClient(), model = NativeConversationModel(client: fake)
            await model.open(project: "p", session: "s"); model.draft = "keep"
            let approval = try XCTUnwrap(model.snapshot?.approvals.first)
            let result = await model.approve(approval, allowed: true, scope: scope)
            XCTAssertTrue(result); XCTAssertEqual(model.draft, "keep")
            let writes = await fake.recorded(); XCTAssertEqual(writes.count, 1); XCTAssertEqual(writes[0].0, "p")
            XCTAssertEqual(writes[0].1["session_id"], .string("s")); XCTAssertEqual(writes[0].1["approval_id"], .string("approval"))
            XCTAssertEqual(writes[0].1["scope"], scope == "once" ? nil : .string(scope))
            model.pause()
        }
    }
    @MainActor func testOlderHostSpecialReadOnlyDeniedAndStaleRequestsCannotWriteBroaderGrants() async throws {
        for scenario in ["old", "special", "readonly", "denied", "stale"] {
            let fake = ScopeClient(), model = NativeConversationModel(client: fake)
            await fake.configure(scopes: scenario == "old" ? nil : scenario == "special" ? ["once"] : ["once", "project"], readOnly: scenario == "readonly")
            await model.open(project: "p", session: "s")
            let approval = try XCTUnwrap(model.snapshot?.approvals.first)
            if scenario == "stale" { await fake.configure(id: "new"); await model.refresh() }
            let result = await model.approve(approval, allowed: scenario != "denied", scope: "project")
            XCTAssertFalse(result); let writes = await fake.recorded(); XCTAssertTrue(writes.isEmpty)
            model.pause()
        }
    }
    @MainActor func testUnknownApprovalDoesNotRepeatUntilUserReconcilesExactPendingRequest() async throws {
        let fake = ScopeClient(), model = NativeConversationModel(client: fake)
        await model.open(project: "p", session: "s"); defer { model.pause() }
        let approval = try XCTUnwrap(model.snapshot?.approvals.first)
        await fake.configure(fail: true)
        _ = await model.approve(approval, allowed: true, scope: "global")
        XCTAssertFalse(model.canApprove(approval)); XCTAssertTrue(model.approvalSubmitted(approval))
        _ = await model.approve(approval, allowed: true, scope: "global")
        await model.refresh(); XCTAssertFalse(model.canApprove(approval))
        await model.reconcileApproval(approval); XCTAssertTrue(model.canApprove(approval))
        let readsOnly = await fake.recorded(); XCTAssertEqual(readsOnly.count, 1)
        await fake.configure(); _ = await model.approve(approval, allowed: false, feedback: "Change output path")
        let writes = await fake.recorded(); XCTAssertEqual(writes.count, 2)
        XCTAssertEqual(writes.last?.1["approved"], .bool(false)); XCTAssertEqual(writes.last?.1["feedback"], .string("Change output path")); XCTAssertNil(writes.last?.1["scope"])
    }
    @MainActor func testMalformedScopeMapsDoNotPublishAnApprovalSnapshot() async {
        for scopes in [["global"], ["once", "once"], ["once", "everything"]] {
            let fake = ScopeClient(), model = NativeConversationModel(client: fake)
            await fake.configure(scopes: scopes); await model.open(project: "p", session: "s")
            XCTAssertNil(model.snapshot); XCTAssertNotNil(model.connectionError)
            let writes = await fake.recorded(); XCTAssertTrue(writes.isEmpty); model.pause()
        }
    }
    @MainActor func testScopePickerImmediateEscapeKeepsParentAndDoesNotSelectOrWrite() {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 420, height: 500), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let parentView = NSView(frame: window.contentView!.bounds); window.contentView = parentView
        var parentClosed = 0, pickerClosed = 0, selected = 0
        let parent = NativeSettingsEscape.Coordinator(enabled: true) { parentClosed += 1 }; parent.view = parentView; parent.install(); defer { parent.remove() }
        let hosted = NSHostingView(rootView: NativeApprovalScopePicker(scopes: ["once", "session", "project", "global"], selected: "once", close: { pickerClosed += 1 }) { _ in selected += 1 })
        hosted.frame = parentView.bounds; parentView.addSubview(hosted); hosted.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertEqual(pickerClosed, 1); XCTAssertEqual(parentClosed, 0); XCTAssertEqual(selected, 0); XCTAssertTrue(window.firstResponder === focus)
        hosted.removeFromSuperview()
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertEqual(parentClosed, 1)
    }
    @MainActor func testRenderApprovalScopePickerAndCardInBothLocalesAndSchemes() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Set WISP_NATIVE_SNAPSHOT_DIR to render approval scopes") }
        let previous = UserDefaults.standard.object(forKey: "nativeSettings.locale")
        defer { if let previous { UserDefaults.standard.set(previous, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        let fake = ScopeClient(), model = NativeConversationModel(client: fake)
        await model.open(project: "p", session: "s"); defer { model.pause() }
        let approval = try XCTUnwrap(model.snapshot?.approvals.first)
        for locale in ["zh", "en"] {
            for scheme in [ColorScheme.light, .dark] {
                UserDefaults.standard.set(locale, forKey: "nativeSettings.locale")
                for picker in [true, false] {
                    let view = picker ? AnyView(NativeApprovalScopePicker(scopes: ["once", "session", "project", "global"], selected: "project", close: {}, select: { _ in }))
                        : AnyView(NativeToolApprovalCard(conversation: model, approval: approval, scope: "global", feedback: {}).padding(.horizontal, 24))
                    let hosted = NSHostingView(rootView: view.frame(maxWidth: .infinity, maxHeight: .infinity).background(WispDesign.color("bg-app", scheme)).foregroundStyle(WispDesign.color("text", scheme))
                        .tint(WispDesign.color("clay", scheme)).environment(\.colorScheme, scheme))
                    hosted.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
                    hosted.frame = NSRect(x: 0, y: 0, width: 419, height: picker ? 450 : 300); hosted.layoutSubtreeIfNeeded()
                    let bitmap = try XCTUnwrap(hosted.bitmapImageRepForCachingDisplay(in: hosted.bounds)); hosted.cacheDisplay(in: hosted.bounds, to: bitmap)
                    let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:])); XCTAssertGreaterThan(data.count, 1000)
                    try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent("approval-\(picker ? "scopes" : "card")-\(locale)-\(scheme == .dark ? "dark" : "light").png"))
                }
            }
        }
    }
}
