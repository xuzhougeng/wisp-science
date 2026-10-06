import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private func helpersFixture() throws -> SettingsValue {
    var root = URL(fileURLWithPath: #filePath); for _ in 0..<5 { root.deleteLastPathComponent() }
    return try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-conversations/v1/composer-helpers.json")))
}

private actor ComposerHelpersFake: NativeConversationQuerying {
    var fixture: SettingsValue
    var failure = "", target = "", hold = ""
    var held: CheckedContinuation<Void, Never>?
    var writes: [(String, String, [String: SettingsValue])] = []
    init(_ fixture: SettingsValue) { self.fixture = fixture }
    func setup(target: String = "", failure: String = "", hold: String = "") { self.target = target; self.failure = failure; self.hold = hold }
    func replace(_ key: String, value: SettingsValue) { fixture[key] = value }
    func waiting() -> Bool { held != nil }
    func release() { held?.resume(); held = nil; hold = "" }
    func recorded() -> [(String, String, [String: SettingsValue])] { writes }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { throw ProjectBrowserError.invalidResponse }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        if command == hold { await withCheckedContinuation { held = $0 } }
        if command == "native_conversation_options" { return .object(["session_id": .string("s"), "full_permission": .bool(false), "delegation": .bool(true), "completion": .object(["policy": .string("background"), "auto_resume": .bool(true)]), "auto_review": .bool(true), "specialist": .null, "specialist_locked": .bool(false)]) }
        let write = ["set_memory_enabled", "set_auto_failure_analysis_settings", "save_specialist_cmd"].contains(command)
        if write {
            writes.append((projectID, command, args))
            if command == "set_memory_enabled" { fixture["memory"]["enabled"] = args["enabled"]! }
            if command == "set_auto_failure_analysis_settings" { fixture["analysis"] = args["settings"]! }
            if command == "save_specialist_cmd" {
                var rows = fixture["specialists"].array; rows[0] = args["spec"]!; fixture["specialists"] = .array(rows)
            }
            if command == target && failure == "lost" { throw ProjectBrowserError.service("reply lost") }
        }
        let keys = ["get_memory_view": "memory", "set_memory_enabled": "memory", "get_auto_failure_analysis_settings": "analysis", "set_auto_failure_analysis_settings": "analysis", "list_specialists": "specialists", "save_specialist_cmd": "specialists", "list_models": "models", "list_acp_agents": "agents"]
        guard let key = keys[command] else { throw ProjectBrowserError.invalidResponse }
        var result = fixture[key]
        if command == target && failure == "unconfirmed" {
            if key == "memory" { result["project_id"] = .string("other-project") }
            if key == "analysis" { result["minimum_failures"] = .integer(99) }
            if key == "specialists" { var rows = result.array; rows[0]["review_backend"] = .object(["kind": .string("follow_session")]); result = .array(rows) }
        }
        if command == target && failure == "malformed" { return .null }
        return result
    }
}

final class NativeComposerHelpersTests: XCTestCase {
    func testSharedHelperShapesRejectWrongOwnersInvalidRangesAndMalformedCatalogs() throws {
        let fixture = try helpersFixture()
        XCTAssertTrue(try NativeComposerMemoryPreference.decode(fixture["memory"], project: "project-a").enabled)
        XCTAssertThrowsError(try NativeComposerMemoryPreference.decode(fixture["memory"], project: "wrong"))
        let settings = try NativeFailureAnalysisSettings.decode(fixture["analysis"])
        XCTAssertEqual(settings.value, fixture["analysis"])
        for (field, invalid) in [("failure_rate_threshold", SettingsValue.integer(0)), ("minimum_failures", .integer(101)), ("enabled", .string("true"))] {
            var value = fixture["analysis"]; value[field] = invalid; XCTAssertThrowsError(try NativeFailureAnalysisSettings.decode(value))
        }
        let choices = try NativeReviewerConfiguration.choices(models: fixture["models"], agents: fixture["agents"], defaultLabel: "Default", followLabel: "Follow")
        XCTAssertEqual(choices.map(\.id), ["http:", "follow_session", "http:chat-a", "http:sibling", "acp:agent-a"])
        for invalid in [SettingsValue.null, .array([fixture["models"].array[0], fixture["models"].array[0]]), .array([.object(["id": .integer(1), "label": .string("Model")])])] {
            XCTAssertThrowsError(try NativeReviewerConfiguration.choices(models: invalid, agents: fixture["agents"], defaultLabel: "Default", followLabel: "Follow"))
        }
        var malformed = fixture["models"].array[0]; malformed["image_generation_capable"] = .string("false")
        XCTAssertThrowsError(try NativeReviewerConfiguration.choices(models: .array([malformed]), agents: fixture["agents"], defaultLabel: "Default", followLabel: "Follow"))
        var unknown = fixture["specialists"].array[0]; unknown["review_backend"] = .object(["kind": .string("future")])
        XCTAssertThrowsError(try NativeReviewerConfiguration.key(unknown))
    }
    @MainActor func testReviewerChangesPreservePersonaAndUseOnlyAdvertisedChatOrAcpProfiles() async throws {
        let fixture = try helpersFixture(), fake = ComposerHelpersFake(try helpersFixture())
        var writable = true
        let model = NativeComposerHelpersModel(client: fake, project: "project-a", writable: { writable }); await model.load()
        XCTAssertTrue(model.canEdit); XCTAssertEqual(model.reviewerKey, "http:chat-a")
        for key in ["http:image-a", "http:video-a", "acp:missing", "unknown"] { await model.setReviewer(key) }
        let rejected = await fake.recorded(); XCTAssertTrue(rejected.isEmpty)
        for key in ["follow_session", "acp:agent-a", "http:sibling", "http:"] { await model.setReviewer(key); XCTAssertEqual(model.reviewerKey, key) }
        let writes = await fake.recorded(); XCTAssertEqual(writes.count, 4)
        let original = fixture["specialists"].array[0]
        for write in writes {
            XCTAssertEqual(write.0, "project-a"); XCTAssertEqual(write.1, "save_specialist_cmd")
            for field in ["name", "instructions", "icon", "color", "description", "skills", "connectors", "builtin"] { XCTAssertEqual(write.2["spec"]?[field], original[field]) }
            XCTAssertNil(write.2["session_id"])
        }
        XCTAssertEqual(writes[0].2["spec"]?["model_id"], .string("chat-a"))
        XCTAssertEqual(writes[1].2["spec"]?["model_id"], .string("chat-a"))
        XCTAssertEqual(writes[2].2["spec"]?["model_id"], .string("sibling"))
        writable = false; await model.setReviewer("follow_session"); await model.setMemory(false)
        let final = await fake.recorded(); XCTAssertEqual(final.count, 4)
    }
    @MainActor func testGlobalMemoryAndFailureSettingsConfirmValuesAndBoundRangesWithoutSessionArguments() async throws {
        let fake = ComposerHelpersFake(try helpersFixture()), model: NativeComposerHelpersModel
        model = NativeComposerHelpersModel(client: fake, project: "project-a", writable: { true }); await model.load()
        await model.setMemory(false); XCTAssertFalse(try XCTUnwrap(model.memory).enabled)
        var analysis = try XCTUnwrap(model.analysis); analysis.enabled = false; await model.setAnalysis(analysis)
        analysis.failure_rate_threshold = 100; analysis.minimum_failures = 1; await model.setAnalysis(analysis)
        XCTAssertEqual(model.analysis, analysis)
        analysis.minimum_failures = 0; await model.setAnalysis(analysis)
        analysis.minimum_failures = 101; await model.setAnalysis(analysis)
        let writes = await fake.recorded(); XCTAssertEqual(writes.count, 3)
        XCTAssertEqual(writes[0].2, ["enabled": .bool(false), "project_id": .string("project-a")])
        XCTAssertEqual(writes[2].2["settings"], model.analysis?.value)
        XCTAssertTrue(writes.allSatisfy { $0.0 == "project-a" && $0.2["session_id"] == nil })
    }
    @MainActor func testReviewerRefreshPreservesConcurrentPersonaEditsAndRejectsChangedSelection() async throws {
        let fake = ComposerHelpersFake(try helpersFixture()), model: NativeComposerHelpersModel
        model = NativeComposerHelpersModel(client: fake, project: "project-a", writable: { true }); await model.load()
        var rows = try helpersFixture()["specialists"].array
        rows[0]["instructions"] = .string("Changed in another window"); rows[0]["skills"] = .array([.string("new-skill")])
        await fake.replace("specialists", value: .array(rows)); await model.setReviewer("follow_session")
        let first = await fake.recorded(); XCTAssertEqual(first.count, 1)
        XCTAssertEqual(first[0].2["spec"]?["instructions"], rows[0]["instructions"])
        XCTAssertEqual(first[0].2["spec"]?["skills"], rows[0]["skills"])
        rows[0]["review_backend"] = .object(["kind": .string("acp_agent"), "profile_id": .string("agent-a")])
        await fake.replace("specialists", value: .array(rows)); await model.setReviewer("http:")
        XCTAssertNotNil(model.error); XCTAssertFalse(model.canEdit)
        let final = await fake.recorded(); XCTAssertEqual(final.count, 1)
        await model.load(); XCTAssertEqual(model.reviewerKey, "acp:agent-a"); XCTAssertTrue(model.canEdit)
    }
    @MainActor func testUnknownAndUnconfirmedWritesBlockReplayUntilExplicitRead() async throws {
        for command in ["set_memory_enabled", "set_auto_failure_analysis_settings", "save_specialist_cmd"] {
            for failure in ["lost", "unconfirmed", "malformed"] {
                let fake = ComposerHelpersFake(try helpersFixture()), model: NativeComposerHelpersModel
                model = NativeComposerHelpersModel(client: fake, project: "project-a", writable: { true }); await model.load()
                var settings = try XCTUnwrap(model.analysis); settings.minimum_failures = 4
                await fake.setup(target: command, failure: failure)
                for _ in 0..<2 {
                    if command == "set_memory_enabled" { await model.setMemory(false) }
                    if command == "set_auto_failure_analysis_settings" { await model.setAnalysis(settings) }
                    if command == "save_specialist_cmd" { await model.setReviewer("acp:agent-a") }
                }
                XCTAssertNotNil(model.error); XCTAssertFalse(model.canEdit)
                let writes = await fake.recorded(); XCTAssertEqual(writes.count, 1)
                await fake.setup(); await model.load(); XCTAssertNil(model.error); XCTAssertTrue(model.canEdit)
                let rereadWrites = await fake.recorded(); XCTAssertEqual(rereadWrites.count, 1)
                if command == "set_memory_enabled" { XCTAssertEqual(model.memory?.enabled, false) }
                if command == "set_auto_failure_analysis_settings" { XCTAssertEqual(model.analysis, settings) }
                if command == "save_specialist_cmd" { XCTAssertEqual(model.reviewerKey, "acp:agent-a") }
            }
        }
    }
    @MainActor func testUnavailableCatalogReadDoesNotInventPreferencesOrReviewerAndCannotWrite() async throws {
        let fake = ComposerHelpersFake(try helpersFixture()), model: NativeComposerHelpersModel
        model = NativeComposerHelpersModel(client: fake, project: "project-a", writable: { true })
        await fake.setup(target: "list_models", failure: "malformed"); await model.load()
        XCTAssertNotNil(model.error); XCTAssertNil(model.memory); XCTAssertNil(model.analysis); XCTAssertFalse(model.canEdit)
        await model.setMemory(false); await model.setReviewer("follow_session")
        let writes = await fake.recorded(); XCTAssertTrue(writes.isEmpty)
        await fake.setup(); await fake.replace("specialists", value: .array([])); await model.load()
        XCTAssertNil(model.error); XCTAssertNil(model.reviewer); XCTAssertTrue(model.canEdit)
        await model.setReviewer("follow_session"); let final = await fake.recorded(); XCTAssertTrue(final.isEmpty)
    }
    @MainActor func testClosedOrCancelledReadsAndLateWritesCannotUpdateAnotherSheetOrReplay() async throws {
        for command in ["get_memory_view", "set_memory_enabled", "list_specialists"] {
            let fake = ComposerHelpersFake(try helpersFixture()), model: NativeComposerHelpersModel
            model = NativeComposerHelpersModel(client: fake, project: "project-a", writable: { true })
            if command != "get_memory_view" { await model.load() }
            await fake.setup(hold: command)
            let operation = Task {
                if command == "get_memory_view" { await model.load() }
                else if command == "list_specialists" { await model.setReviewer("follow_session") }
                else { await model.setMemory(false) }
            }
            while !(await fake.waiting()) { await Task.yield() }
            if command == "set_memory_enabled" { XCTAssertTrue(model.saving); await model.setMemory(false) }
            model.close(); await fake.release(); await operation.value
            XCTAssertNil(model.memory); XCTAssertNil(model.analysis); XCTAssertNil(model.reviewer); XCTAssertNil(model.error); XCTAssertFalse(model.busy)
            let writes = await fake.recorded(); XCTAssertEqual(writes.count, command == "set_memory_enabled" ? 1 : 0)
        }
        let fake = ComposerHelpersFake(try helpersFixture()), model: NativeComposerHelpersModel
        model = NativeComposerHelpersModel(client: fake, project: "project-a", writable: { true }); await fake.setup(hold: "get_memory_view")
        let read = Task { await model.load() }; while !(await fake.waiting()) { await Task.yield() }; read.cancel(); await fake.release(); await read.value
        XCTAssertNil(model.memory); XCTAssertNil(model.error); XCTAssertFalse(model.busy)
    }
    @MainActor func testImmediateEscapeClosesReviewerBeforeOptionsWithoutMovingFocusOrSaving() throws {
        _ = NSApplication.shared
        let fake = ComposerHelpersFake(try helpersFixture()), conversation = NativeConversationModel(client: ComposerHelpersFake(try helpersFixture()))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 419, height: 500), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let container = NSView(frame: window.contentView!.bounds); window.contentView = container
        var parentClosed = 0, childClosed = 0, writes = 0
        let options = NativeComposerOptionsModel(client: fake, project: "project-a", session: "s", writable: { true })
        let parent = NSHostingView(rootView: NativeComposerOptionsSheet(conversation: conversation, model: options, close: { parentClosed += 1 }))
        parent.frame = container.bounds; container.addSubview(parent); parent.layoutSubtreeIfNeeded()
        let child = NSHostingView(rootView: NativeReviewerBackendPicker(choices: [.init(id: "follow_session", label: "Follow")], selected: "follow_session", close: { childClosed += 1 }, select: { _ in writes += 1 }))
        child.frame = container.bounds; container.addSubview(child); child.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53))
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertEqual(childClosed, 1); XCTAssertEqual(parentClosed, 0); XCTAssertEqual(writes, 0); XCTAssertTrue(window.firstResponder === focus)
        child.removeFromSuperview(); XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertEqual(parentClosed, 1)
    }
    @MainActor func testRenderGlobalHelpersAndReviewerPickerAtNarrowWidthInBothLocalesAndSchemes() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Set WISP_NATIVE_SNAPSHOT_DIR to render composer helpers") }
        let previous = UserDefaults.standard.object(forKey: "nativeSettings.locale")
        defer { if let previous { UserDefaults.standard.set(previous, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        for locale in ["zh", "en"] {
            UserDefaults.standard.set(locale, forKey: "nativeSettings.locale")
            let fake = ComposerHelpersFake(try helpersFixture()), conversation = NativeConversationModel(client: ComposerHelpersFake(try helpersFixture()))
            let options = NativeComposerOptionsModel(client: fake, project: "project-a", session: "s", writable: { true }); await options.load()
            let helpers = NativeComposerHelpersModel(client: fake, project: "project-a", writable: { true }); await helpers.load()
            for scheme in [ColorScheme.light, .dark] {
                for surface in ["options", "short", "helpers", "reviewer"] {
                    let view: AnyView
                    if surface == "reviewer" { view = AnyView(NativeReviewerBackendPicker(choices: helpers.choices, selected: helpers.reviewerKey, close: {}, select: { _ in })) }
                    else if surface == "helpers" { view = AnyView(NativeComposerHelpersControls(model: helpers, reviewerPicker: .constant(false)).padding(24)) }
                    else { view = AnyView(NativeComposerOptionsSheet(conversation: conversation, model: options, helpers: helpers, close: {})) }
                    let hosted = NSHostingView(rootView: view.frame(maxWidth: .infinity, maxHeight: .infinity).background(WispDesign.color("bg-app", scheme)).foregroundStyle(WispDesign.color("text", scheme)).tint(WispDesign.color("clay", scheme)).environment(\.colorScheme, scheme))
                    hosted.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
                    hosted.frame = NSRect(x: 0, y: 0, width: 419, height: surface == "options" ? 680 : 420); hosted.layoutSubtreeIfNeeded()
                    let bitmap = try XCTUnwrap(hosted.bitmapImageRepForCachingDisplay(in: hosted.bounds)); hosted.cacheDisplay(in: hosted.bounds, to: bitmap)
                    let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:])); XCTAssertGreaterThan(data.count, 1000)
                    try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent("composer-\(surface)-\(locale)-\(scheme == .dark ? "dark" : "light").png"))
                }
            }
        }
    }
}
