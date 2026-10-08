import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor AssistantHost: NativeSettingsQuerying {
    var fixture: SettingsValue
    var writes: [[String: SettingsValue]] = []
    var fail = false
    var hold = false
    var pending: CheckedContinuation<SettingsValue, Error>?
    init() throws { fixture = try Self.read("contracts/native-assistant/v1/workspace.json") }
    static func read(_ path: String) throws -> SettingsValue {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        return try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent(path)))
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        switch command {
        case "native_assistant_open": XCTAssertNil(projectID); return fixture["open"]
        case "native_assistant_workspace":
            XCTAssertNil(projectID)
            if hold { hold = false; return try await withCheckedThrowingContinuation { pending = $0 } }
            var value = fixture["workspace"]; value["day"] = args["day"] ?? .null; return value
        case "native_assistant_mutate":
            XCTAssertNil(projectID); writes.append(args)
            if fail { throw ProjectBrowserError.service("lost reply") }
            return .null
        case "native_conversation_snapshot":
            var value = try Self.read("contracts/native-conversations/v1/snapshot.json")
            value["project_id"] = .string(NativeAssistantModel.project); value["session_id"] = .string(NativeAssistantModel.session)
            value["composer_references"] = .bool(true); value["running"] = .bool(false); value["stopping"] = .bool(false); value["approvals"] = .array([]); return value
        case "native_conversation_send": writes.append(args); return .null
        case "get_privacy_mode": return .object(["active": .bool(false), "project_ids": .array([])])
        case "native_research_calendar": return .array([])
        case "list_models", "list_acp_agents": return .array([])
        default: return .null
        }
    }
    func failWrites() { fail = true }
    func count() -> Int { writes.count }
    func last() -> [String: SettingsValue] { writes.last ?? [:] }
    func suspend() { hold = true }
    func held() -> Bool { pending != nil }
    func finish() { pending?.resume(returning: fixture["workspace"]); pending = nil }
    func hide() { fixture["workspace"]["projects"] = .array([]); fixture["workspace"]["plan"] = .array([]) }
}

final class NativeAssistantTests: XCTestCase {
    @MainActor func testRenderAssistantAndAutomationsInBothLocalesAndSchemes() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Opt-in assistant rendering") }
        try FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
        let previous = UserDefaults.standard.object(forKey: "nativeSettings.locale")
        defer { if let previous { UserDefaults.standard.set(previous, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        let client = try AssistantHost(); let model = NativeAssistantModel(client: client)
        await model.open(); defer { model.close() }
        let browser = ProjectBrowserModel(client: AssistantProjectList(), databaseURL: URL(fileURLWithPath: "/unused/assistant-render.sqlite"), projectTransport: client)
        for locale in ["zh", "en"] {
            UserDefaults.standard.set(locale, forKey: "nativeSettings.locale")
            for scheme in [ColorScheme.light, .dark] {
                for width: CGFloat in [432, 1060, 1440] {
                    let suffix = "\(locale)-\(scheme == .dark ? "dark" : "light")-\(Int(width))"
                    try await render(NativeAssistantPage(browser: browser, model: model), width: width, scheme: scheme, name: "assistant-\(suffix)", directory: directory)
                    try await render(NativeAutomationsSheet(model: model) {}, width: width, scheme: scheme, name: "automations-\(suffix)", directory: directory)
                    try await render(NativeAutomationEditor(model: model, draft: .constant(NativeAutomationDraft())) {}, width: width, scheme: scheme, name: "automation-editor-\(suffix)", directory: directory)
                }
            }
        }
    }
    @MainActor private func render<V: View>(_ view: V, width: CGFloat, scheme: ColorScheme, name: String, directory: String) async throws {
        let host = NSHostingView(rootView: view.environment(\.colorScheme, scheme).background(WispDesign.color("bg-app", scheme)).foregroundStyle(WispDesign.color("text", scheme)))
        host.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
        host.frame = NSRect(x: 0, y: 0, width: width, height: 720); host.layoutSubtreeIfNeeded()
        try await Task.sleep(nanoseconds: 40_000_000); host.layoutSubtreeIfNeeded()
        let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds)); host.cacheDisplay(in: host.bounds, to: bitmap)
        try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: URL(fileURLWithPath: directory).appendingPathComponent(name + ".png"))
    }
    func testAutomationAndTimerUseTheSharedMutationContract() throws {
        let fixture = try AssistantHost.read("contracts/native-assistant/v1/workspace.json")
        var draft = NativeAutomationDraft(); draft.project = "project-a"; draft.name = "Daily check"; draft.prompt = "Inspect records"
        XCTAssertEqual(draft.operation().map(SettingsValue.object), fixture["operations"].array[0])
        draft.timer = true; XCTAssertNil(draft.operation()); draft.session = "session-a"
        XCTAssertEqual(draft.operation().map(SettingsValue.object), fixture["operations"].array[1])
        draft.minutes = 0; XCTAssertNil(draft.operation()); draft.minutes = 525601; XCTAssertNil(draft.operation())
    }
    func testDailyAndWeeklyStartAtTheNextLocalTime() {
        var calendar = Calendar(identifier: .gregorian); calendar.timeZone = TimeZone(secondsFromGMT: 8 * 3600)!
        let now = calendar.date(from: DateComponents(year: 2026, month: 10, day: 8, hour: 10))!
        var draft = NativeAutomationDraft(); draft.project = "p"; draft.prompt = "Review"; draft.cadence = "daily"
        draft.time = calendar.date(from: DateComponents(year: 2026, month: 10, day: 8, hour: 9))!
        let daily = draft.operation(now: now, calendar: calendar)!
        XCTAssertEqual(daily["interval_secs"], .integer(86400))
        let next = Date(timeIntervalSince1970: TimeInterval(daily["start_at"]!.integer))
        XCTAssertEqual(calendar.component(.day, from: next), 9); XCTAssertEqual(calendar.component(.hour, from: next), 9)
        draft.cadence = "weekly"; draft.weekday = 2
        let weekly = draft.operation(now: now, calendar: calendar)!
        XCTAssertEqual(weekly["interval_secs"], .integer(604800))
        XCTAssertEqual(calendar.component(.weekday, from: Date(timeIntervalSince1970: TimeInterval(weekly["start_at"]!.integer))), 2)
    }
    @MainActor func testProjectContextPersistsAcrossSendsAndDisappearsWhenHidden() async throws {
        let host = try AssistantHost(); let model = NativeAssistantModel(client: host)
        await model.open(); defer { model.close() }
        XCTAssertEqual(model.projects.count, 1)
        model.selectedProject = "project-a"; model.conversation.draft = "What happened?"
        await model.conversation.send()
        let args = await host.last()
        XCTAssertEqual(args["references"]?.array.first?["id"], .string("project-a"))
        XCTAssertTrue(args["message"]?.string.contains("Project context: 水稻研究") == true)
        XCTAssertNotNil(model.conversation.contextReference)
        await host.hide(); await model.reload()
        XCTAssertEqual(model.selectedProject, ""); XCTAssertNil(model.conversation.contextReference)
        XCTAssertFalse(model.conversation.canAttach)
    }
    @MainActor func testUncertainWriteRequiresFreshReadAndExplicitAcknowledgement() async throws {
        let host = try AssistantHost(); let model = NativeAssistantModel(client: host)
        await model.open(); await host.failWrites()
        let accepted = await model.mutate(["action": .string("run_daily_recap")]); XCTAssertFalse(accepted)
        model.acknowledge(); XCTAssertTrue(model.uncertain)
        let before = await host.count()
        let replayed = await model.mutate(["action": .string("run_daily_recap")]); XCTAssertFalse(replayed)
        let after = await host.count(); XCTAssertEqual(before, after)
        model.close(); await model.open(); XCTAssertTrue(model.uncertain)
        XCTAssertTrue(model.canAcknowledge); model.acknowledge(); XCTAssertTrue(model.canWrite)
        model.close()
    }
    @MainActor func testLateReadCannotReopenAssistantOrExposeProjectsAfterClosing() async throws {
        let host = try AssistantHost(); let model = NativeAssistantModel(client: host)
        await model.open(); await host.suspend()
        let read = Task { await model.reload() }
        while !(await host.held()) { await Task.yield() }
        XCTAssertFalse(model.conversation.contextReady)
        model.close(); await host.finish(); await read.value
        XCTAssertFalse(model.active); XCTAssertEqual(model.workspace, .null)
    }
    @MainActor func testImmediateEscapeClosesAutomationEditorOnly() async throws {
        let host = try AssistantHost(); let model = NativeAssistantModel(client: host)
        await model.open(); defer { model.close() }
        var closed = false
        let root = NSHostingView(rootView: NativeAutomationEditor(model: model, draft: .constant(NativeAutomationDraft())) { closed = true })
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 560, height: 600), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = root; root.layoutSubtreeIfNeeded()
        RunLoop.current.run(until: Date().addingTimeInterval(0.03))
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertTrue(closed); XCTAssertTrue(model.active)
        window.contentView = nil
    }
}

private actor AssistantProjectList: ProjectBrowserQuerying {
    func listProjects(databaseURL: URL) async throws -> ProjectListSnapshot { ProjectListSnapshot(projects: [], activitySource: "persisted_only") }
    func listSessions(databaseURL: URL, projectID: String?) async throws -> [BrowserSession] { [] }
    func transcript(databaseURL: URL, projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> TranscriptPage { TranscriptPage(messages: [], nextBeforeSeq: nil) }
}
