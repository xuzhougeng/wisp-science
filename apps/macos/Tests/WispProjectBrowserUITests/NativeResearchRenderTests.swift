import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

final class NativeResearchRenderTests: XCTestCase {
    @MainActor func testRenderResearchPagesAtWideAndNarrowWidths() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Opt-in native rendering") }
        try FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
        let client = ResearchRenderClient()
        let model = ProjectBrowserModel(client: client, databaseURL: URL(fileURLWithPath: "/unused/research.sqlite"), projectTransport: client)
        await model.refresh()
        model.publication.open(projectID: "research-1")
        await model.publication.reload(client)
        model.publication.selectItem("item-1")
        model.publication.selectedBindingID = "binding-1"
        model.journey.open(projectID: "research-1", day: nil)
        await model.journey.reload(client)
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(secondsFromGMT: 8 * 3600)!
        model.calendar.calendar = calendar
        model.calendar.clock = Date(timeIntervalSince1970: 1790524800)
        model.calendar.presented = true
        await model.calendar.openMonth(client, projectIDs: ["research-1"])
        for narrow in [false, true] {
            let width: CGFloat = narrow ? 432 : 816
            try await render(NativePublicationColumn(model: model, publication: model.publication), width: width, name: "publication-\(narrow ? "narrow" : "wide")", directory: directory, model: model)
            try await render(NativeJourneyPage(model: model, journey: model.journey), width: width, name: "journey-\(narrow ? "narrow" : "wide")", directory: directory, model: model)
            try await render(NativeCalendarPage(model: model, calendar: model.calendar), width: narrow ? 680 : 1056, name: "calendar-\(narrow ? "narrow" : "wide")", directory: directory, model: model)
        }
    }

    @MainActor private func render<V: View>(_ content: V, width: CGFloat, name: String, directory: String, model: ProjectBrowserModel) async throws {
        let host = NSHostingView(rootView: content
            .background(WispDesign.color("bg-app", .light))
            .foregroundStyle(WispDesign.color("text", .light))
            .tint(WispDesign.color("clay", .light))
            .environment(\.colorScheme, .light))
        host.appearance = NSAppearance(named: .aqua)
        host.frame = NSRect(x: 0, y: 0, width: width, height: 720)
        host.layoutSubtreeIfNeeded()
        // The real views start their read tasks on mount. Allow the fake
        // transport to finish so this captures content, not a loading frame.
        for _ in 0..<50 {
            try await Task.sleep(nanoseconds: 10_000_000)
            if !model.publication.busy && !model.journey.busy && !model.calendar.busy { break }
        }
        XCTAssertFalse(model.publication.busy || model.journey.busy || model.calendar.busy)
        model.publication.selectItem("item-1")
        model.publication.selectedBindingID = "binding-1"
        host.layoutSubtreeIfNeeded()
        XCTAssertEqual(host.frame.width, width, accuracy: 1)
        let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: bitmap)
        let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        XCTAssertGreaterThan(data.count, 1000)
        try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent(name + ".png"))
    }
}

private actor ResearchRenderClient: NativeSettingsQuerying, ProjectBrowserQuerying {
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        if command == "get_privacy_mode" { return .object(["active": .bool(false), "project_ids": .array([])]) }
        if command == "native_publication_workspace" {
            var root = URL(fileURLWithPath: #filePath)
            for _ in 0..<5 { root.deleteLastPathComponent() }
            return try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-publication/v1/workspace-evidence.json")))["result"]
        }
        let entries: [[String: Any]] = (0..<35).map { index in
            ["id": "message-\(index)", "kind": "session", "title": "35 轮工具栏验收", "occurred_at": 1790524800 + index, "source_id": "session-1", "frame_id": "session-1", "status": "discussed", "manual": false]
        }
        let history: [String: Any] = ["entries": entries, "truncated": false]
        let value: Any = command == "native_research_calendar" ? [["project_id": "research-1", "history": history]] : history
        return try JSONDecoder().decode(SettingsValue.self, from: JSONSerialization.data(withJSONObject: value))
    }
    func listProjects(databaseURL: URL) async throws -> ProjectListSnapshot {
        let json = #"{"id":"research-1","name":"研究验收","description":"","workspace_dir":"/unused","starred":false,"session_count":1,"artifact_count":2,"updated_at":1790524800,"running_count":0,"needs_you_count":0,"sync_configured":false}"#
        return ProjectListSnapshot(projects: [try JSONDecoder().decode(ProjectSummary.self, from: Data(json.utf8))], activitySource: "persisted_only")
    }
    func listSessions(databaseURL: URL, projectID: String?) async throws -> [BrowserSession] { [] }
    func transcript(databaseURL: URL, projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> TranscriptPage { TranscriptPage(messages: [], nextBeforeSeq: nil) }
}
