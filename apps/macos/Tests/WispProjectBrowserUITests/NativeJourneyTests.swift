import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

final class NativeJourneyTests: XCTestCase {
    private var utc: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(secondsFromGMT: 0)!
        return calendar
    }

    @MainActor func testReloadUsesTheExplicitProjectAndDoesNotRetryALostRead() async throws {
        let host = JourneyTransport()
        let journey = NativeJourneyModel()
        journey.calendar = utc
        journey.clock = Date(timeIntervalSince1970: 100)
        journey.open(projectID: "research-1", day: 100)
        await journey.reload(host)
        let call = await host.lastCall()
        XCTAssertEqual(call.command, NativeJourneyCommand.read)
        XCTAssertEqual(call.projectID, "research-1")
        XCTAssertEqual(call.args["from"], .integer(0))
        XCTAssertEqual(call.args["until"], .integer(86400))
        XCTAssertEqual(journey.entries.map(\.title), ["a finding"])
        journey.query = "missing"
        XCTAssertTrue(journey.visibleEntries().isEmpty)
        let filtered = await host.callCount()
        XCTAssertEqual(filtered, 1)
        journey.query = ""
        await host.setMode("lost")
        await journey.reload(host)
        let after = await host.callCount()
        XCTAssertEqual(after, 2)
        XCTAssertEqual(journey.entries.map(\.title), ["a finding"])
        XCTAssertTrue(journey.error?.contains("不会自动重试") == true)
        await Task.yield()
        let still = await host.callCount()
        XCTAssertEqual(still, 2)
    }

    @MainActor func testClosedJourneyDoesNotReadAndALateReadDoesNotOpenAProject() async throws {
        let list = JourneyProjectList()
        let host = JourneyTransport()
        let model = ProjectBrowserModel(client: list, databaseURL: URL(fileURLWithPath: "/unused/journey.sqlite"), projectTransport: host)
        await model.journey.reload(host)
        let idle = await host.callCount()
        XCTAssertEqual(idle, 0)
        model.journey.calendar = utc
        model.journey.clock = Date(timeIntervalSince1970: 100)
        model.journey.open(projectID: "research-1", day: nil)
        await host.suspend()
        let task = Task { await model.journey.reload(host) }
        while !(await host.isHanging()) { await Task.yield() }
        model.goHome()
        await host.resume(with: try JourneyTransport.page(title: "late finding"))
        await task.value
        let project = model.activeProjectID
        let titles = model.journey.entries.map(\.title)
        XCTAssertNil(project)
        XCTAssertFalse(model.journey.presented)
        XCTAssertTrue(titles.isEmpty)
    }

    @MainActor func testImmediateEscapeClosesOnlyTheJourney() {
        _ = NSApplication.shared
        let journey = NativeJourneyModel()
        journey.open(projectID: "research-1", day: nil)
        var parent = true
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 420, height: 280), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let root = NSView(frame: window.contentLayoutRect)
        window.contentView = root
        let parentView = NSView(frame: .zero)
        root.addSubview(parentView)
        let owner = NativeSettingsEscape.Coordinator(enabled: true) { parent = false }
        owner.view = parentView
        owner.install()
        defer { owner.remove() }
        let host = NSHostingView(rootView: JourneyEscapeSheet(journey: journey))
        root.addSubview(host)
        host.frame = root.bounds
        host.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertFalse(journey.presented)
        XCTAssertTrue(parent)
        XCTAssertEqual(journey.projectID, "research-1")
        XCTAssertTrue(window.firstResponder === focus)
    }
}

private struct JourneyEscapeSheet: View {
    @ObservedObject var journey: NativeJourneyModel
    var body: some View {
        Text("研究历程")
            .background(NativeSettingsEscape(enabled: !journey.busy) { journey.dismiss() })
    }
}

private actor JourneyTransport: NativeSettingsQuerying {
    private var recorded: [(command: String, args: [String: SettingsValue], projectID: String?)] = []
    private var mode = "ok"
    private var release: CheckedContinuation<SettingsValue, Error>?
    func setMode(_ mode: String) { self.mode = mode }
    func callCount() -> Int { recorded.count }
    func lastCall() -> (command: String, args: [String: SettingsValue], projectID: String?) { recorded.last! }
    func suspend() { mode = "hang" }
    func isHanging() -> Bool { release != nil }
    func resume(with value: SettingsValue) { release?.resume(returning: value); release = nil }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        recorded.append((command, args, projectID))
        if mode == "lost" { throw ProjectBrowserError.service("connection reset") }
        if mode == "hang" { return try await withCheckedThrowingContinuation { release = $0 } }
        return try Self.fixturePage()
    }
    static func fixturePage() throws -> SettingsValue {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let fixture = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-journey/v1/range.json")))
        return fixture["result"]
    }
    static func page(title: String) throws -> SettingsValue {
        var page = try fixturePage()
        var entry = page["entries"].array[0]
        entry["title"] = .string(title)
        page["entries"] = .array([entry])
        return page
    }
}

private actor JourneyProjectList: ProjectBrowserQuerying {
    func listProjects(databaseURL: URL) async throws -> ProjectListSnapshot {
        ProjectListSnapshot(projects: [], activitySource: "persisted_only")
    }
    func listSessions(databaseURL: URL, projectID: String?) async throws -> [BrowserSession] { [] }
    func transcript(databaseURL: URL, projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> TranscriptPage {
        TranscriptPage(messages: [], nextBeforeSeq: nil)
    }
}
