import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

final class NativeCalendarTests: XCTestCase {
    private var utc: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(secondsFromGMT: 0)!
        return calendar
    }

    func testMonthAndDayBoundsUseTheLocalCalendar() {
        let bounds = NativeCalendarClock.monthInterval(containing: Date(timeIntervalSince1970: 100), calendar: utc)
        XCTAssertEqual(bounds.0, 0)
        XCTAssertEqual(bounds.1, 31 * 86400)
        let day = NativeCalendarClock.dayInterval(containing: Date(timeIntervalSince1970: 100), calendar: utc)
        XCTAssertEqual(day.0, 0)
        XCTAssertEqual(day.1, 86400)
    }

    @MainActor func testReloadOmitsPrivacyProjectsAndDoesNotRetryALostRead() async throws {
        let host = CalendarTransport()
        let calendar = NativeCalendarModel()
        calendar.calendar = utc
        calendar.clock = Date(timeIntervalSince1970: 100)
        calendar.presented = true
        calendar.privacyActive = true
        calendar.privacyProjectIDs = ["hidden"]
        await calendar.reloadMonth(host, projectIDs: ["research-1", "hidden", "research-1"])
        let calls = await host.calls()
        XCTAssertEqual(calls.count, 2)
        XCTAssertEqual(calls[0].command, NativeCalendarCommand.read)
        XCTAssertNil(calls[0].projectID)
        XCTAssertEqual(calls[0].args["project_ids"], .array([.string("research-1")]))
        XCTAssertEqual(calls[0].args["from"], .integer(0))
        XCTAssertEqual(calls[0].args["until"], .integer(31 * 86400))
        XCTAssertEqual(calls[1].args["from"], .integer(0))
        XCTAssertEqual(calls[1].args["until"], .integer(86400))
        XCTAssertEqual(calls[1].args["project_ids"], .array([.string("research-1")]))
        XCTAssertEqual(calendar.monthRows.map(\.projectID), ["research-1"])
        XCTAssertEqual(calendar.markedDays()[0], ["research-1"])
        XCTAssertEqual(calendar.dayGroups().map(\.projectID), ["research-1"])
        let before = calls.count
        await host.setMode("lost")
        await calendar.reloadMonth(host, projectIDs: ["research-1"])
        let after = await host.callCount()
        XCTAssertEqual(after, before + 1)
        XCTAssertEqual(calendar.monthRows.map(\.projectID), ["research-1"])
        XCTAssertTrue(calendar.error?.contains("不会自动重试") == true)
        await Task.yield()
        let still = await host.callCount()
        XCTAssertEqual(still, after)
    }

    @MainActor func testFilterHidesProjectsWithoutAnotherRequest() async throws {
        let host = CalendarTransport()
        await host.setRows(try CalendarTransport.twoProjects())
        let calendar = NativeCalendarModel()
        calendar.calendar = utc
        calendar.clock = Date(timeIntervalSince1970: 100)
        calendar.presented = true
        await calendar.reloadMonth(host, projectIDs: ["research-1", "other"])
        let loaded = await host.callCount()
        calendar.projectFilter = "other"
        XCTAssertEqual(calendar.markedDays()[0], ["other"])
        XCTAssertEqual(calendar.dayGroups().map(\.projectID), ["other"])
        let after = await host.callCount()
        XCTAssertEqual(after, loaded)
    }

    @MainActor func testOpenJourneyUsesTheProjectListAndALateReadDoesNotReopenIt() async throws {
        let list = CalendarProjectList()
        let host = CalendarTransport()
        let model = ProjectBrowserModel(client: list, databaseURL: URL(fileURLWithPath: "/unused/calendar.sqlite"), projectTransport: host)
        model.calendar.calendar = utc
        model.calendar.clock = Date(timeIntervalSince1970: 100)
        model.calendar.presented = true
        await model.calendar.reloadMonth(host, projectIDs: ["research-1"])
        await model.openCalendarJourney(projectID: "research-1", day: 0)
        let hostCalls = await host.callCount()
        let asked = await list.sessionProjects()
        XCTAssertEqual(hostCalls, 2)
        XCTAssertEqual(asked, ["research-1"])
        XCTAssertEqual(model.activeProjectID, "research-1")
        XCTAssertEqual(model.journeyFocus, JourneyFocus(projectID: "research-1", day: 0))
        XCTAssertFalse(model.calendar.presented)
        model.goHome()
        let home = model.activeProjectID
        XCTAssertNil(home)
        XCTAssertNil(model.journeyFocus)
        await host.suspend()
        model.calendar.presented = true
        let task = Task { await model.calendar.reloadMonth(host, projectIDs: ["research-1"]) }
        while !(await host.isHanging()) { await Task.yield() }
        model.goHome()
        await host.resume(with: try CalendarTransport.twoProjects())
        await task.value
        let project = model.activeProjectID
        let focus = model.journeyFocus
        let rows = model.calendar.monthRows.map(\.projectID)
        XCTAssertNil(project)
        XCTAssertNil(focus)
        XCTAssertEqual(rows, ["research-1"])
    }

    @MainActor func testClosedCalendarDoesNotEnterTheJourney() async {
        let list = CalendarProjectList()
        let host = CalendarTransport()
        let model = ProjectBrowserModel(client: list, databaseURL: URL(fileURLWithPath: "/unused/calendar.sqlite"), projectTransport: host)
        await model.openCalendarJourney(projectID: "research-1", day: 0)
        let asked = await list.sessionProjects()
        XCTAssertTrue(asked.isEmpty)
        XCTAssertNil(model.activeProjectID)
        XCTAssertNil(model.journeyFocus)
    }

    @MainActor func testImmediateEscapeClosesOnlyTheCalendar() {
        _ = NSApplication.shared
        let calendar = NativeCalendarModel()
        calendar.presented = true
        var searchPresented = true
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 420, height: 280), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let root = NSView(frame: window.contentLayoutRect)
        window.contentView = root
        let search = NSView(frame: .zero)
        root.addSubview(search)
        let searchOwner = NativeSettingsEscape.Coordinator(enabled: true) { searchPresented = false }
        searchOwner.view = search
        searchOwner.install()
        defer { searchOwner.remove() }
        let host = NSHostingView(rootView: CalendarEscapeSheet(calendar: calendar))
        root.addSubview(host)
        host.frame = root.bounds
        host.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertFalse(calendar.presented)
        XCTAssertTrue(searchPresented)
        XCTAssertTrue(window.firstResponder === focus)
    }
}

private struct CalendarEscapeSheet: View {
    @ObservedObject var calendar: NativeCalendarModel
    var body: some View {
        Text("研究日历")
            .background(NativeSettingsEscape(enabled: !calendar.busy) { calendar.dismiss() })
    }
}

private actor CalendarTransport: NativeSettingsQuerying {
    private var recorded: [(command: String, args: [String: SettingsValue], projectID: String?)] = []
    private var mode = "ok"
    private var rows: SettingsValue?
    private var release: CheckedContinuation<SettingsValue, Error>?
    func setMode(_ mode: String) { self.mode = mode }
    func setRows(_ rows: SettingsValue) { self.rows = rows }
    func callCount() -> Int { recorded.count }
    func calls() -> [(command: String, args: [String: SettingsValue], projectID: String?)] { recorded }
    func suspend() { mode = "hang" }
    func isHanging() -> Bool { release != nil }
    func resume(with value: SettingsValue) { release?.resume(returning: value); release = nil }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        recorded.append((command, args, projectID))
        if mode == "lost" { throw ProjectBrowserError.service("connection reset") }
        if mode == "hang" { return try await withCheckedThrowingContinuation { release = $0 } }
        return try rows ?? Self.fixtureRows()
    }
    static func fixtureRows() throws -> SettingsValue {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let fixture = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-calendar/v1/month.json")))
        return fixture["result"]
    }
    static func twoProjects() throws -> SettingsValue {
        let rows = try fixtureRows().array
        let first = rows[0]
        var entry = first["history"]["entries"].array[0]
        entry["id"] = .string("journal:other")
        entry["title"] = .string("other finding")
        var history = first["history"]
        history["entries"] = .array([entry])
        var other = first
        other["project_id"] = .string("other")
        other["history"] = history
        return .array(rows + [other])
    }
}

private actor CalendarProjectList: ProjectBrowserQuerying {
    var projects: [String?] = []
    func sessionProjects() -> [String?] { projects }
    func listProjects(databaseURL: URL) async throws -> ProjectListSnapshot {
        ProjectListSnapshot(projects: [], activitySource: "persisted_only")
    }
    func listSessions(databaseURL: URL, projectID: String?) async throws -> [BrowserSession] {
        projects.append(projectID)
        return [BrowserSession(id: "session-a", projectID: projectID ?? "", title: "探索", ts: 1, status: "complete")]
    }
    func transcript(databaseURL: URL, projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> TranscriptPage {
        TranscriptPage(messages: [], nextBeforeSeq: nil)
    }
}
