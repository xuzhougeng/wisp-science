import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor ComposerOptionsFake: NativeConversationQuerying {
    var state: SettingsValue = .object(["session_id": .string("s"), "full_permission": .bool(false), "delegation": .bool(false), "completion": .object(["policy": .string("inline"), "auto_resume": .bool(false)]), "auto_review": .bool(false), "specialist": .null, "specialist_locked": .bool(false)])
    var lose = false, wrongOwner = false, unconfirmed = false, hold = false
    var held: CheckedContinuation<Void, Never>?
    var writes: [(String, [String: SettingsValue])] = []
    func setup(lose: Bool = false, wrongOwner: Bool = false, unconfirmed: Bool = false, hold: Bool = false, locked: Bool = false) {
        self.lose = lose; self.wrongOwner = wrongOwner; self.unconfirmed = unconfirmed; self.hold = hold; state["specialist_locked"] = .bool(locked)
    }
    func waiting() -> Bool { held != nil }
    func release() { held?.resume(); held = nil; hold = false }
    func recorded() -> [(String, [String: SettingsValue])] { writes }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { throw ProjectBrowserError.invalidResponse }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        if command == "list_specialists" { return .array([.object(["id": .string("scientist"), "name": .string("Scientist")]), .object(["id": .string("reviewer"), "name": .string("Reviewer")])]) }
        if command == "native_conversation_options_set" {
            writes.append((projectID, args))
            if hold { await withCheckedContinuation { held = $0 } }
            if lose { throw ProjectBrowserError.service("reply lost") }
            if !unconfirmed {
                let change = args["change"]!, kind = change["kind"].string
                if ["full_permission", "delegation", "auto_review"].contains(kind) { state[kind] = change["enabled"] }
                if kind == "completion" { state["completion"] = .object(["policy": change["policy"], "auto_resume": change["auto_resume"]]) }
                if kind == "specialist" { state["specialist"] = change["id"].string.isEmpty ? .null : .object(["id": change["id"], "name": .string("Scientist")]) }
            }
        }
        var value = state
        if wrongOwner { value["session_id"] = .string("other") }
        return value
    }
}

final class NativeComposerOptionsTests: XCTestCase {
    @MainActor func testRenderOptionsAndPermissionConfirmationInEnglishDark() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Set WISP_NATIVE_SNAPSHOT_DIR to render native options") }
        let previous = UserDefaults.standard.object(forKey: "nativeSettings.locale")
        defer { if let previous { UserDefaults.standard.set(previous, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        UserDefaults.standard.set("en", forKey: "nativeSettings.locale")
        let fake = ComposerOptionsFake(); let conversation = NativeConversationModel(client: fake)
        let model = NativeComposerOptionsModel(client: fake, project: "p", session: "s", writable: { true })
        await model.load(); await model.setToggle("delegation", enabled: true); await model.setCompletion("background", autoResume: true)
        for permission in [false, true] {
            let view: AnyView = permission ? AnyView(NativeFullPermissionConfirmation(busy: false, canConfirm: true, cancel: {}, confirm: {}))
                : AnyView(NativeComposerOptionsSheet(conversation: conversation, model: model, close: {}))
            let hosted = NSHostingView(rootView: view.background(WispDesign.color("bg-app", .dark)).foregroundStyle(WispDesign.color("text", .dark))
                .tint(WispDesign.color("clay", .dark)).environment(\.colorScheme, .dark))
            hosted.appearance = NSAppearance(named: .darkAqua)
            hosted.frame = NSRect(x: 0, y: 0, width: permission ? 430 : 460, height: permission ? 340 : 430); hosted.layoutSubtreeIfNeeded()
            let bitmap = try XCTUnwrap(hosted.bitmapImageRepForCachingDisplay(in: hosted.bounds)); hosted.cacheDisplay(in: hosted.bounds, to: bitmap)
            let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:])); XCTAssertGreaterThan(data.count, 1000)
            try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent(permission ? "options-permission-en-dark.png" : "options-en-dark.png"))
        }
    }
    func testSharedContractRejectsWrongOwnerAndUnknownCompletionPolicy() throws {
        var root = URL(fileURLWithPath: #filePath); for _ in 0..<5 { root.deleteLastPathComponent() }
        var fixture = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-conversations/v1/composer-options.json")))
        XCTAssertFalse(try NativeComposerSessionOptions.decode(fixture, session: "session-a").full_permission)
        XCTAssertThrowsError(try NativeComposerSessionOptions.decode(fixture, session: "other"))
        fixture["completion"]["policy"] = .string("unknown")
        XCTAssertThrowsError(try NativeComposerSessionOptions.decode(fixture, session: "session-a"))
    }
    @MainActor func testScopedSettingsConfirmPermissionsAndEnforceDelegationAndSpecialistBoundaries() async {
        let fake = ComposerOptionsFake(); var writable = true
        let model = NativeComposerOptionsModel(client: fake, project: "p", session: "s", writable: { writable })
        await model.load(); XCTAssertTrue(model.canEdit); XCTAssertEqual(model.specialists.map(\.id), ["scientist"])
        await model.setCompletion("background", autoResume: true); await model.setToggle("full_permission", enabled: true)
        await model.setSpecialist("reviewer"); await model.setToggle("unknown", enabled: true)
        var writes = await fake.recorded(); XCTAssertTrue(writes.isEmpty)
        await model.setToggle("delegation", enabled: true)
        await model.setCompletion("background", autoResume: true); XCTAssertEqual(model.options?.completion.auto_resume, true)
        await model.setCompletion("inline", autoResume: true); XCTAssertEqual(model.options?.completion.auto_resume, false)
        await model.setToggle("full_permission", enabled: true, confirmed: true); XCTAssertEqual(model.options?.full_permission, true)
        await model.setToggle("auto_review", enabled: true); await model.setSpecialist("scientist")
        XCTAssertEqual(model.options?.specialist?["id"].string, "scientist")
        writes = await fake.recorded(); XCTAssertEqual(writes.count, 6); XCTAssertTrue(writes.allSatisfy { $0.0 == "p" && $0.1["session_id"] == .string("s") })
        XCTAssertEqual(writes[3].1["change"], .object(["kind": .string("full_permission"), "enabled": .bool(true), "confirmed": .bool(true)]))
        await fake.setup(locked: true); await model.load(); await model.setSpecialist("")
        writable = false; await model.setToggle("auto_review", enabled: false)
        let final = await fake.recorded(); XCTAssertEqual(final.count, 6)
    }
    @MainActor func testUnknownMismatchedAndUnconfirmedWritesNeverReplayAndNeedFreshRead() async {
        for scenario in ["lost", "owner", "unconfirmed"] {
            let fake = ComposerOptionsFake(), model: NativeComposerOptionsModel
            model = NativeComposerOptionsModel(client: fake, project: "p", session: "s", writable: { true }); await model.load()
            await fake.setup(lose: scenario == "lost", wrongOwner: scenario == "owner", unconfirmed: scenario == "unconfirmed")
            await model.setToggle("auto_review", enabled: true); XCTAssertNotNil(model.error); XCTAssertFalse(model.canEdit)
            await model.setToggle("auto_review", enabled: true)
            let writes = await fake.recorded(); XCTAssertEqual(writes.count, 1)
            await fake.setup(); await model.load(); XCTAssertNil(model.error); XCTAssertTrue(model.canEdit)
        }
    }
    @MainActor func testClosedSheetSuppressesLateWriteAndDuplicateClicks() async {
        let fake = ComposerOptionsFake(); let model = NativeComposerOptionsModel(client: fake, project: "p", session: "s", writable: { true })
        await model.load(); await fake.setup(hold: true)
        let write = Task { await model.setToggle("auto_review", enabled: true) }
        while !(await fake.waiting()) { await Task.yield() }
        await model.setToggle("auto_review", enabled: true)
        model.close(); await fake.release(); await write.value
        XCTAssertNil(model.options); XCTAssertNil(model.error); XCTAssertFalse(model.busy)
        let writes = await fake.recorded(); XCTAssertEqual(writes.count, 1)
    }
    @MainActor func testImmediateEscapeClosesPermissionBeforeOptionsWithoutMovingFocusOrWriting() {
        _ = NSApplication.shared
        let conversation = NativeConversationModel(client: ComposerOptionsFake())
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 500, height: 500), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let container = NSView(frame: window.contentView!.bounds); window.contentView = container
        var optionsClosed = 0, permissionClosed = 0, writes = 0
        let options = NSHostingView(rootView: NativeComposerOptionsSheet(conversation: conversation, project: "p", session: "s", close: { optionsClosed += 1 }))
        options.frame = container.bounds; container.addSubview(options); options.layoutSubtreeIfNeeded()
        let child = NSHostingView(rootView: NativeFullPermissionConfirmation(busy: false, canConfirm: true, cancel: { permissionClosed += 1 }, confirm: { writes += 1 }))
        child.frame = container.bounds; container.addSubview(child); child.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertEqual(permissionClosed, 1); XCTAssertEqual(optionsClosed, 0); XCTAssertEqual(writes, 0); XCTAssertTrue(window.firstResponder === focus)
        child.removeFromSuperview()
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertEqual(optionsClosed, 1)
    }
}
