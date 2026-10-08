import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor PolishHost: NativeSettingsQuerying {
    var calls: [(String, [String: SettingsValue], String?)] = []
    var hooks: SettingsValue = .array([])
    var failure = false
    var hold = false
    var pending: CheckedContinuation<SettingsValue, Error>?
    var page: SettingsValue = .object(["entries": .array([]), "truncated": .bool(false), "recaps": .array([])])
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        calls.append((command, args, projectID))
        if hold && (command.hasPrefix("set_") || command == "native_research_journey_add") { return try await withCheckedThrowingContinuation { pending = $0 } }
        switch command {
        case "get_command_hooks": return hooks
        case "get_auto_review_enabled": return .bool(false)
        case "get_auto_failure_analysis_settings": return .object(["enabled": .bool(false), "failure_rate_threshold": .integer(30), "minimum_failures": .integer(2)])
        case "get_project_hooks": return .object(["path": .string("/fixture/.wisp/hooks.json"), "hooks": .array([NativeHookDraft.empty]), "sha256": .string("reviewed-hash"), "trusted": .bool(false), "error": .null])
        case "set_command_hooks": hooks = args["hooks"] ?? .null; if failure { throw ProjectBrowserError.invalidResponse }; return hooks
        case "set_project_hooks_trust": return .object(["sha256": args["sha256"] ?? .null, "trusted": .bool(args["sha256"] != .null)])
        case "native_research_journey": return page
        case "native_research_journey_add": if failure { throw ProjectBrowserError.invalidResponse }; return .string("record-1")
        case "native_research_journey_recap": return args["edit"] ?? .null
        case "native_research_journey_graph": if failure { throw ProjectBrowserError.invalidResponse }; return .object(["nodes": .array([]), "edges": .array([])])
        default: throw ProjectBrowserError.invalidResponse
        }
    }
    func fail() { failure = true }
    func recover() { failure = false }
    func suspend() { hold = true }
    func waiting() -> Bool { pending != nil }
    func resume(_ value: SettingsValue) { pending?.resume(returning: value); pending = nil; hold = false }
    func captured() -> [(String, [String: SettingsValue], String?)] { calls }
    func setPage(_ value: SettingsValue) { page = value }
}

final class NativeParityPolishTests: XCTestCase {
    @MainActor func testJourneyGraphSuccessfulRefreshClearsPreviousReadError() async {
        let host = PolishHost(), journey = NativeJourneyModel()
        journey.open(projectID: "p", day: 100)
        await host.fail(); await journey.loadGraph(host); XCTAssertNotNil(journey.error)
        await host.recover(); await journey.loadGraph(host)
        XCTAssertNil(journey.error); XCTAssertEqual(journey.graph["nodes"], .array([]))
    }
    @MainActor func testJourneyPresentationUsesTranslatedLabelsAndCalendarTime() {
        let previous = UserDefaults.standard.object(forKey: "nativeSettings.locale")
        defer { if let previous { UserDefaults.standard.set(previous, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        var calendar = Calendar(identifier: .gregorian); calendar.timeZone = TimeZone(secondsFromGMT: 0)!
        for (locale, draft, status) in [("zh", "草稿", "运行成功"), ("en", "Draft", "Succeeded")] {
            UserDefaults.standard.set(locale, forKey: "nativeSettings.locale")
            XCTAssertEqual(NativeJourneyPresentation.label("draft"), draft)
            XCTAssertEqual(NativeJourneyPresentation.display(.string("succeeded"), key: "status"), status)
            XCTAssertEqual(NativeJourneyPresentation.display(.integer(1791432000), key: "ended_at", calendar: calendar), "2026-10-08 04:00:00")
            XCTAssertEqual(NativeJourneyPresentation.display(.integer(0), key: "exit_code"), "0")
        }
    }
    @MainActor func testHooksLostAcknowledgementDoesNotReplayAndTrustUsesReviewedHash() async throws {
        let host = PolishHost()
        let hooks = NativeHooksModel(client: host, projectID: "project-a")
        await hooks.load(); XCTAssertTrue(hooks.canWrite)
        var row = NativeHookDraft.empty; row["command"] = .string("echo fixture")
        await host.fail()
        let saved = await hooks.saveHook(row, at: nil); XCTAssertFalse(saved); XCTAssertTrue(hooks.uncertain)
        let retried = await hooks.saveHook(row, at: nil); XCTAssertFalse(retried)
        hooks.acknowledge(); XCTAssertTrue(hooks.uncertain)
        hooks.close(); await hooks.load(); XCTAssertTrue(hooks.uncertain); XCTAssertEqual(hooks.hooks, [row])
        hooks.acknowledge(); XCTAssertTrue(hooks.canWrite)
        let trusted = await hooks.write("set_project_hooks_trust", args: ["sha256": hooks.project["sha256"]]); XCTAssertTrue(trusted)
        let calls = await host.captured()
        XCTAssertEqual(calls.filter { $0.0 == "set_command_hooks" }.count, 1)
        XCTAssertEqual(calls.first { $0.0 == "set_project_hooks_trust" }?.1["sha256"], .string("reviewed-hash"))
        XCTAssertTrue(calls.allSatisfy { $0.2 == "project-a" })
    }
    @MainActor func testHooksCloseDuringWriteKeepsUnknownOutcomeAndCannotRefreshBeforeCompletion() async throws {
        let host = PolishHost()
        let model = NativeHooksModel(client: host, projectID: "p")
        await model.load(); await host.suspend()
        let write = Task { await model.write("set_command_hooks", args: ["hooks": .array([])]) }
        while !(await host.waiting()) { await Task.yield() }
        model.close(); await model.load(); model.acknowledge(); XCTAssertTrue(model.uncertain)
        await host.resume(.array([])); let accepted = await write.value; XCTAssertFalse(accepted)
        await model.load(); XCTAssertTrue(model.refreshed); XCTAssertTrue(model.uncertain)
        model.acknowledge(); XCTAssertTrue(model.canWrite)
    }
    @MainActor func testHookEditorCannotOverwriteARowChangedDuringReconciliation() async throws {
        let host = PolishHost()
        let hooks = NativeHooksModel(client: host, projectID: "p")
        await hooks.load()
        var original = NativeHookDraft.empty; original["command"] = .string("original")
        let added = await hooks.saveHook(original, at: nil); XCTAssertTrue(added)
        var changed = original; changed["command"] = .string("changed")
        let updated = await hooks.saveHook(changed, at: 0); XCTAssertTrue(updated)
        let overwritten = await hooks.saveHook(original, at: 0, expected: original); XCTAssertFalse(overwritten)
        XCTAssertEqual(hooks.hooks[0], changed); XCTAssertNotNil(hooks.error)
        let calls = await host.captured(); XCTAssertEqual(calls.filter { $0.0 == "set_command_hooks" }.count, 2)
    }
    @MainActor func testJourneyUncertaintyRetainsSelectedDateAndRequiresReadBeforeAcknowledging() async throws {
        let host = PolishHost(), journey = NativeJourneyModel()
        journey.open(projectID: "a", day: 100); await journey.reload(host); XCTAssertTrue(journey.canWrite)
        await host.fail()
        let saved = await journey.mutate("native_research_journey_add", args: ["input": .object([:])], client: host, savedDay: 172800)
        XCTAssertFalse(saved); XCTAssertTrue(journey.uncertain); XCTAssertEqual(journey.day, 172800)
        journey.acknowledge(); XCTAssertTrue(journey.uncertain)
        let retried = await journey.mutate("native_research_journey_add", args: [:], client: host); XCTAssertFalse(retried)
        await journey.reload(host); journey.acknowledge(); XCTAssertFalse(journey.uncertain)
        let calls = await host.captured(); XCTAssertEqual(calls.filter { $0.0 == "native_research_journey_add" }.count, 1)
        XCTAssertTrue(calls.allSatisfy { $0.2 == "a" })
    }
    @MainActor func testJourneyLateWriteDoesNotChangeAnotherProjectAndKeepsOriginalUncertainty() async throws {
        let host = PolishHost(), journey = NativeJourneyModel()
        journey.open(projectID: "a", day: 100); await journey.reload(host); await host.suspend()
        let write = Task { await journey.mutate("native_research_journey_add", args: [:], client: host, savedDay: 200) }
        while !(await host.waiting()) { await Task.yield() }
        journey.open(projectID: "b", day: 500)
        await host.resume(.string("record-a")); let accepted = await write.value; XCTAssertFalse(accepted)
        XCTAssertEqual(journey.projectID, "b"); XCTAssertEqual(journey.day, 500); XCTAssertTrue(journey.entries.isEmpty)
        XCTAssertFalse(journey.uncertain); XCTAssertTrue(journey.uncertainProjects.contains("a"))
        await journey.reload(host); XCTAssertTrue(journey.canWrite)
        let wrongOwner = await journey.mutate("native_research_journey_add", args: [:], client: host, expectedProject: "a")
        XCTAssertFalse(wrongOwner)
        let calls = await host.captured(); XCTAssertEqual(calls.filter { $0.0 == "native_research_journey_add" }.count, 1)
    }
    @MainActor func testRecapEditSendsOnlyEditableFieldsAndPreservesSourceIndexesForBackendValidation() {
        let recap: SettingsValue = .object(["id": .string("r"), "headline": .string("Reviewed"), "sources": .array([.object(["id": .string("s")])]), "done": .array([.object(["text": .string("Evidence"), "refs": .array([.integer(0)])])]), "findings": .array([]), "issues": .array([]), "next": .array([]), "model": .string("fixture")])
        let edit = NativeRecapEditor.edit(recap, status: "confirmed")
        XCTAssertEqual(Set(edit.object.keys), Set(["id", "headline", "status", "done", "findings", "issues", "next"]))
        XCTAssertEqual(edit["done"], recap["done"]); XCTAssertEqual(edit["status"], .string("confirmed"))
    }
    func testAutomationPresetsMatchSharedWeekdaysAndRequireProjectBeforeSaving() throws {
        XCTAssertEqual(NativeAutomationTemplate.all.count, 3)
        var calendar = Calendar(identifier: .gregorian); calendar.timeZone = TimeZone(secondsFromGMT: 0)!
        let now = Date(timeIntervalSince1970: 1791432000)
        for (index, template) in NativeAutomationTemplate.all.enumerated() {
            let blank = template.draft(project: "", locale: "zh", now: now, calendar: calendar)
            XCTAssertNil(blank.operation(now: now, calendar: calendar))
            let draft = template.draft(project: "p", locale: "en", now: now, calendar: calendar)
            XCTAssertEqual(draft.prompt, template.prompt_en); XCTAssertEqual(draft.name, template.en)
            let operation = try XCTUnwrap(draft.operation(now: now, calendar: calendar))
            XCTAssertEqual(operation["project_id"], .string("p")); XCTAssertTrue(operation["start_at"]!.integer > Int64(now.timeIntervalSince1970))
            if template.cadence == "weekly" { XCTAssertEqual(calendar.component(.weekday, from: Date(timeIntervalSince1970: TimeInterval(operation["start_at"]!.integer))), index == 0 ? 2 : 6) }
        }
    }
    @MainActor func testRenderNewSettingsAndJourneyEditorsAcrossLocalesAndSchemes() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Opt-in native rendering") }
        try FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
        let previous = UserDefaults.standard.object(forKey: "nativeSettings.locale")
        defer { if let previous { UserDefaults.standard.set(previous, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        let transport = PolishHost(), journey = NativeJourneyModel(), hooks = NativeHooksModel(client: PolishHost(), projectID: "p")
        journey.open(projectID: "p", day: 100); await journey.reload(transport); await hooks.load()
        var hook = NativeHookDraft.empty; hook["command"] = .string("python3 .wisp/hooks/check.py"); hook["matcher"] = .string("shell|write|edit")
        let recap: SettingsValue = .object(["id": .string("r"), "headline": .string("Validated sample quality / 样本质量验证"), "status": .string("draft"), "done": .array([.object(["text": .string("Compared sample counts and recorded the evidence."), "refs": .array([.integer(0)])])]), "findings": .array([]), "issues": .array([]), "next": .array([]), "sources": .array([.object(["id": .string("run-1"), "kind": .string("run"), "title": .string("Quality control run")])])])
        for locale in ["en", "zh"] { for scheme in [ColorScheme.light, .dark] { for width: CGFloat in [432, 760] {
            UserDefaults.standard.set(locale, forKey: "nativeSettings.locale")
            let views: [(String, AnyView)] = [
                ("hooks", AnyView(ScrollView { NativeHooksSettings(model: hooks) }.padding(20))),
                ("hook-editor", AnyView(NativeHookEditor(model: hooks, original: .init(value: hook), close: {}))),
                ("journal-editor", AnyView(NativeJournalEditor(journey: journey, client: transport, close: {}))),
                ("recap-editor", AnyView(NativeRecapEditor(journey: journey, draft: .init(value: recap), client: transport, openSession: { _ in }, close: {})))
            ]
            for (name, view) in views {
                let host = NSHostingView(rootView: view.environment(\.colorScheme, scheme).background(WispDesign.color("bg-app", scheme)).foregroundStyle(WispDesign.color("text", scheme)))
                host.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
                host.frame = NSRect(x: 0, y: 0, width: width, height: name == "hooks" ? 1200 : 760); host.layoutSubtreeIfNeeded()
                try await Task.sleep(nanoseconds: 60_000_000); host.layoutSubtreeIfNeeded()
                let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds)); host.cacheDisplay(in: host.bounds, to: bitmap)
                try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: URL(fileURLWithPath: directory).appendingPathComponent("polish-\(name)-\(locale)-\(scheme)-\(Int(width)).png"))
            }
        } } }
    }
    @MainActor private func escapeChild<Content: View>(_ content: Content, closed: () -> Bool) throws {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 680, height: 760), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.contentView = nil; window.close() }
        var parentClosed = false
        let parent = NSHostingView(rootView: Color.clear.background(NativeSettingsEscape { parentClosed = true }))
        parent.frame = window.contentView!.bounds; window.contentView!.addSubview(parent); parent.layoutSubtreeIfNeeded()
        let child = NSHostingView(rootView: content); child.frame = parent.frame; window.contentView!.addSubview(child); child.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53))
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertTrue(closed()); XCTAssertFalse(parentClosed); XCTAssertTrue(window.firstResponder === focus)
        child.removeFromSuperview(); XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertTrue(parentClosed)
    }
    @MainActor func testNewEditorsAndRunDetailsConsumeImmediateEscapeAboveTheirParent() throws {
        let host = PolishHost(), journey = NativeJourneyModel(); journey.open(projectID: "p", day: 100)
        var closed = false
        try escapeChild(NativeJournalEditor(journey: journey, client: host) { closed = true }, closed: { closed })
        closed = false
        try escapeChild(NativeHookEditor(model: NativeHooksModel(client: host, projectID: "p"), original: NativeHookDraft(value: NativeHookDraft.empty)) { closed = true }, closed: { closed })
        closed = false
        try escapeChild(NativeRecapEditor(journey: journey, draft: .init(value: .null), client: host, openSession: { _ in }) { closed = true }, closed: { closed })
        closed = false
        try escapeChild(NativeJourneyRunView(id: "run", projectID: "p", client: host) { closed = true }, closed: { closed })
    }
}
