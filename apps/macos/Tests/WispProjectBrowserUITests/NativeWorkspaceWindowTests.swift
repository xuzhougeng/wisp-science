import Foundation
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor WindowBrowserFake: ProjectBrowserQuerying {
    var mode = "", hold = false
    var held: CheckedContinuation<Void, Never>?
    var databases: [URL] = []
    func configure(_ mode: String = "", hold: Bool = false) { self.mode = mode; self.hold = hold }
    func waiting() -> Bool { held != nil }
    func release() { held?.resume(); held = nil; hold = false }
    func reads() -> [URL] { databases }
    func listProjects(databaseURL: URL) async throws -> ProjectListSnapshot {
        databases.append(databaseURL)
        if hold { await withCheckedContinuation { held = $0 } }
        if mode == "unavailable" { throw ProjectBrowserError.service("offline") }
        let data = Data("""
        {"id":"p","name":"Research","description":"","workspace_dir":"/unused","starred":false,"session_count":2,"artifact_count":0,"updated_at":1,"running_count":0,"needs_you_count":0,"sync_configured":false}
        """.utf8)
        let project = try JSONDecoder().decode(ProjectSummary.self, from: data)
        return ProjectListSnapshot(projects: mode == "hidden-project" ? [] : [project], activitySource: "persisted_only")
    }
    func listSessions(databaseURL: URL, projectID: String?) async throws -> [BrowserSession] {
        if mode == "missing-session" { return [] }
        return [BrowserSession(id: "s1", projectID: mode == "wrong-owner" ? "foreign" : "p", title: "First", ts: 1, status: "idle"),
                BrowserSession(id: "s2", projectID: "p", title: "Second", ts: 2, status: "idle")]
    }
    func transcript(databaseURL: URL, projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> TranscriptPage {
        if mode == "transcript-unavailable" { throw ProjectBrowserError.service("history offline") }
        return TranscriptPage(messages: [], nextBeforeSeq: nil)
    }
}

private actor WindowHostFake: NativeSettingsQuerying {
    var requests: [(String, String?, String?)] = []
    func recorded() -> [(String, String?, String?)] { requests }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        requests.append((command, projectID, args["session_id"]?.string))
        guard command == "native_conversation_snapshot" else { return command == "list_models" || command == "list_acp_agents" ? .array([]) : .null }
        var root = URL(fileURLWithPath: #filePath); for _ in 0..<5 { root.deleteLastPathComponent() }
        var value = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-conversations/v1/snapshot.json")))
        value["project_id"] = projectID.map(SettingsValue.string) ?? .null; value["session_id"] = args["session_id"] ?? .null
        value["read_only"] = .bool(false); value["running"] = .bool(false); value["stopping"] = .bool(false); value["approvals"] = .array([]); value["queue"] = .null
        return value
    }
}

final class NativeWorkspaceWindowTests: XCTestCase {
    @MainActor func testTransportRegistryReusesEachDatabaseHostAndSeparatesAChangedDatabaseWithoutLaunching() throws {
        let registry = NativeWorkspaceTransports(), first = URL(fileURLWithPath: "/first.sqlite"), second = URL(fileURLWithPath: "/second.sqlite")
        let firstHost = try XCTUnwrap(registry.client(database: first) as? NativeSettingsClient)
        let secondHost = try XCTUnwrap(registry.client(database: second) as? NativeSettingsClient)
        XCTAssertFalse(firstHost === secondHost)
        XCTAssertTrue(firstHost === (registry.client(database: first) as? NativeSettingsClient))
        XCTAssertTrue(secondHost === (registry.client(database: second) as? NativeSettingsClient))
        let source = ProjectBrowserModel(client: WindowBrowserFake(), databaseURL: first)
        let child = source.makeIndependentWorkspace(databaseURL: first)
        XCTAssertTrue((source.calendarClient() as? NativeSettingsClient) === (child.calendarClient() as? NativeSettingsClient))
        let otherDatabase = child.makeIndependentWorkspace(databaseURL: second)
        XCTAssertFalse((source.calendarClient() as? NativeSettingsClient) === (otherDatabase.calendarClient() as? NativeSettingsClient))
        let another = source.makeIndependentWorkspace(databaseURL: second)
        XCTAssertTrue((another.calendarClient() as? NativeSettingsClient) === (otherDatabase.calendarClient() as? NativeSettingsClient))
    }
    func testSceneRequestsRoundTripWithoutDraftsAndUseDistinctWindowIdentities() throws {
        let database = URL(fileURLWithPath: "/workspace/wisp.sqlite")
        let first = NativeWorkspaceWindowRequest(databaseURL: database, projectID: "p", sessionID: "s1")
        let second = NativeWorkspaceWindowRequest(databaseURL: database, projectID: "p", sessionID: "s1")
        XCTAssertTrue(first.valid); XCTAssertNotEqual(first.id, second.id)
        let bytes = try JSONEncoder().encode(first)
        XCTAssertEqual(try JSONDecoder().decode(NativeWorkspaceWindowRequest.self, from: bytes), first)
        let keys = try XCTUnwrap(JSONSerialization.jsonObject(with: bytes) as? [String: Any])
        XCTAssertEqual(Set(keys.keys), ["id", "databaseURL", "projectID", "sessionID"])
        for invalid in [NativeWorkspaceWindowRequest(databaseURL: URL(string: "https://example.test/wisp.sqlite")!),
                        NativeWorkspaceWindowRequest(databaseURL: URL(string: "file:///workspace/wisp.sqlite?token=x")!),
                        NativeWorkspaceWindowRequest(databaseURL: URL(fileURLWithPath: "/")),
                        NativeWorkspaceWindowRequest(databaseURL: database, sessionID: "s1"),
                        NativeWorkspaceWindowRequest(databaseURL: database, projectID: ""),
                        NativeWorkspaceWindowRequest(databaseURL: database, projectID: "p", sessionID: ""),
                        NativeWorkspaceWindowRequest(databaseURL: database, projectID: String(repeating: "科", count: 171))] {
            XCTAssertFalse(invalid.valid)
        }
    }
    @MainActor func testSearchWindowRequestsAcceptExactProjectsAndSessionsAndArtifactsStayInOriginalWindow() throws {
        let browser = ProjectBrowserModel(client: WindowBrowserFake(), databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: WindowHostFake())
        func item(_ kind: String, _ id: String, _ session: String?) throws -> NativeSearchItem {
            let value = SettingsValue.object(["kind": .string(kind), "id": .string(id), "project_id": .string("p"), "project_name": .string("Research"), "title": .string("Result"), "detail": .string(""), "session_id": session.map(SettingsValue.string) ?? .null])
            return try JSONDecoder().decode(NativeSearchItem.self, from: JSONEncoder().encode(value))
        }
        let project = try XCTUnwrap(browser.windowRequest(for: item("project", "p", nil)))
        XCTAssertEqual(project.databaseURL, browser.databaseURL); XCTAssertEqual(project.projectID, "p"); XCTAssertNil(project.sessionID)
        let session = try XCTUnwrap(browser.windowRequest(for: item("session", "s1", "s1")))
        XCTAssertEqual(session.projectID, "p"); XCTAssertEqual(session.sessionID, "s1")
        XCTAssertNil(browser.windowRequest(for: try item("session", "s1", "s2")))
        XCTAssertNil(browser.windowRequest(for: try item("artifact", "a1", "s1")))
        XCTAssertNil(browser.newWindowRequest().projectID)
    }
    @MainActor func testIndependentWindowsKeepNavigationDraftsAndPanelsSeparateWhileSharingTheHostTransport() async throws {
        let query = WindowBrowserFake(), host = WindowHostFake(), database = URL(fileURLWithPath: "/unused")
        let source = ProjectBrowserModel(client: query, databaseURL: database, projectTransport: host)
        await source.openProject("p", sessionID: "s1")
        let sourceConversation = source.nativeConversation(); await sourceConversation.open(project: "p", session: "s1")
        defer { sourceConversation.pause() }
        sourceConversation.draft = "Source draft"
        let child = source.makeIndependentWorkspace(databaseURL: database)
        XCTAssertFalse(child.windowReady)
        let loaded = await child.loadWindow(NativeWorkspaceWindowRequest(databaseURL: database, projectID: "p", sessionID: "s2"))
        XCTAssertTrue(loaded); XCTAssertTrue(child.windowReady)
        XCTAssertEqual(source.activeSessionID, "s1"); XCTAssertEqual(child.activeSessionID, "s2")
        let childConversation = child.nativeConversation(); XCTAssertFalse(sourceConversation === childConversation)
        await childConversation.open(project: "p", session: "s2"); defer { childConversation.pause() }
        XCTAssertNil(childConversation.connectionError); childConversation.draft = "Child draft"
        child.searchPresented = true; child.settingsPresented = true
        XCTAssertFalse(source.searchPresented); XCTAssertFalse(source.settingsPresented)
        XCTAssertFalse(source.library === child.library); XCTAssertFalse(source.calendar === child.calendar)
        XCTAssertFalse(source.nativeSideChat(projectID: "p", sessionID: "s1") === child.nativeSideChat(projectID: "p", sessionID: "s1"))
        child.goHome(); childConversation.pause()
        XCTAssertEqual(source.activeSessionID, "s1"); XCTAssertEqual(sourceConversation.draft, "Source draft")
        XCTAssertEqual(childConversation.draft, "Child draft")
        let snapshots = await host.recorded().filter { $0.0 == "native_conversation_snapshot" }
        XCTAssertTrue(snapshots.contains { $0.1 == "p" && $0.2 == "s1" }); XCTAssertTrue(snapshots.contains { $0.1 == "p" && $0.2 == "s2" })
        let commands = await host.recorded(); XCTAssertFalse(commands.contains { $0.0.contains("send") })
    }
    @MainActor func testUnavailableHiddenMissingAndForeignTargetsDoNotFallbackToRecentConversations() async {
        for mode in ["unavailable", "hidden-project", "missing-session", "wrong-owner", "transcript-unavailable"] {
            let query = WindowBrowserFake(); await query.configure(mode)
            let source = ProjectBrowserModel(client: query, databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: WindowHostFake())
            let child = source.makeIndependentWorkspace(databaseURL: source.databaseURL)
            let opened = await child.loadWindow(NativeWorkspaceWindowRequest(databaseURL: source.databaseURL, projectID: "p", sessionID: "s1"))
            XCTAssertFalse(opened, mode); XCTAssertFalse(child.windowReady, mode); XCTAssertFalse(child.windowOpening)
            XCTAssertNotNil(child.error ?? child.sessionError, mode)
            XCTAssertNil(source.activeProjectID); XCTAssertNil(source.activeSessionID)
        }
    }
    @MainActor func testCapturedDatabaseInvalidRequestsAndCancelledLateReadsNeverAffectSource() async {
        let query = WindowBrowserFake(), sourceDatabase = URL(fileURLWithPath: "/source.sqlite"), capturedDatabase = URL(fileURLWithPath: "/captured.sqlite")
        let source = ProjectBrowserModel(client: query, databaseURL: sourceDatabase, projectTransport: WindowHostFake())
        let child = source.makeIndependentWorkspace(databaseURL: capturedDatabase)
        let wrongDatabase = await child.loadWindow(NativeWorkspaceWindowRequest(databaseURL: sourceDatabase))
        let wrongDatabaseReads = await query.reads(); XCTAssertFalse(wrongDatabase); XCTAssertTrue(wrongDatabaseReads.isEmpty)
        let invalid = await child.loadWindow(NativeWorkspaceWindowRequest(databaseURL: capturedDatabase, sessionID: "s1"))
        let invalidReads = await query.reads(); XCTAssertFalse(invalid); XCTAssertTrue(invalidReads.isEmpty)
        await query.configure(hold: true)
        let opening = Task { await child.loadWindow(NativeWorkspaceWindowRequest(databaseURL: capturedDatabase, projectID: "p", sessionID: "s1")) }
        while !(await query.waiting()) { await Task.yield() }
        let duplicate = await child.loadWindow(NativeWorkspaceWindowRequest(databaseURL: capturedDatabase)); XCTAssertFalse(duplicate)
        opening.cancel(); await query.release(); let opened = await opening.value
        XCTAssertFalse(opened); XCTAssertFalse(child.windowReady); XCTAssertFalse(child.windowOpening)
        let reads = await query.reads(); XCTAssertEqual(reads, [capturedDatabase]); XCTAssertEqual(source.databaseURL, sourceDatabase); XCTAssertNil(source.activeProjectID)
    }
}
