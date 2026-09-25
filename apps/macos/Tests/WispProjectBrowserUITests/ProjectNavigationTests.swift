import Foundation
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor NavigationClient: ProjectBrowserQuerying {
    var continuation: CheckedContinuation<[BrowserSession], Error>?
    var suspend = false
    var paginated = false
    var rows: [BrowserSession]?
    var failList = false
    var listCount = 0
    func configure(rows: [BrowserSession]? = nil, fail: Bool = false) { self.rows = rows; failList = fail }
    func requests() -> Int { listCount }
    func enablePagination() { paginated = true }
    func setSuspended() { suspend = true }
    func isWaiting() -> Bool { continuation != nil }
    func finish() { continuation?.resume(returning: rows ?? []); continuation = nil }
    func listProjects(databaseURL: URL) async throws -> ProjectListSnapshot {
        ProjectListSnapshot(projects: [], activitySource: "persisted_only")
    }
    func listSessions(databaseURL: URL, projectID: String?) async throws -> [BrowserSession] {
        listCount += 1
        if suspend { return try await withCheckedThrowingContinuation { continuation = $0 } }
        if failList { throw ProjectBrowserError.service("List unavailable") }
        if let rows { return rows }
        return try JSONDecoder().decode([BrowserSession].self, from: Data("""
        [{"id":"s1","project_id":"p","title":"First","ts":1,"status":"complete"},
         {"id":"s2","project_id":"p","title":"Second","ts":2,"status":"needs_you"}]
        """.utf8))
    }
    func transcript(databaseURL: URL, projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> TranscriptPage {
        guard ["s1", "s2"].contains(sessionID) else { throw ProjectBrowserError.service("Session not found in project") }
        let sequence = paginated && beforeSeq == nil ? 21 : 1
        let data = Data("[{\"seq\":\(sequence),\"role\":\"user\",\"text\":\"\(sessionID)\",\"tool_name\":null}]".utf8)
        return TranscriptPage(messages: try JSONDecoder().decode([BrowserMessage].self, from: data), nextBeforeSeq: paginated && beforeSeq == nil ? 21 : nil)
    }
}

final class ProjectNavigationTests: XCTestCase {
    @MainActor func testMetadataRefreshKeepsSelectionTranscriptCursorAndDraft() async {
        let client = NavigationClient(); await client.enablePagination()
        let database = URL(fileURLWithPath: "/unused")
        let model = ProjectBrowserModel(client: client, databaseURL: database)
        await model.openProject("p", sessionID: "s1")
        let conversation = model.nativeConversation(); conversation.draft = "Unsent notes"
        await client.configure(rows: [BrowserSession(id: "s1", projectID: "p", title: "Renamed", ts: 1, status: "complete", pinned: true)])
        let refreshed = await model.refreshSessionMetadata(projectID: "p", sessionID: "s1", database: database)
        XCTAssertTrue(refreshed); XCTAssertEqual(model.sessions.first?.pinned, true)
        XCTAssertEqual(model.sessions.first?.title, "Renamed")
        XCTAssertEqual(model.activeSessionID, "s1"); XCTAssertEqual(model.messages.map(\.seq), [21])
        XCTAssertEqual(model.nextBeforeSeq, 21); XCTAssertEqual(conversation.draft, "Unsent notes")
        XCTAssertFalse(model.sessionsLoading)
        await client.configure(fail: true)
        let failed = await model.refreshSessionMetadata(projectID: "p", sessionID: "s1", database: database)
        XCTAssertFalse(failed); XCTAssertNotNil(model.sessionError)
        XCTAssertEqual(model.sessions.first?.pinned, true); XCTAssertEqual(model.messages.map(\.seq), [21])
        XCTAssertEqual(conversation.draft, "Unsent notes")
    }
    @MainActor func testMetadataRefreshRejectsStaleScopeAndNavigationReplies() async {
        let client = NavigationClient(); let database = URL(fileURLWithPath: "/unused")
        let model = ProjectBrowserModel(client: client, databaseURL: database)
        await model.openProject("p", sessionID: "s1")
        for (project, session, db) in [("p", "s1", URL(fileURLWithPath: "/other")), ("other", "s1", database), ("p", "s2", database)] {
            let result = await model.refreshSessionMetadata(projectID: project, sessionID: session, database: db)
            XCTAssertFalse(result)
        }
        let count = await client.requests(); XCTAssertEqual(count, 1)
        await client.setSuspended()
        let refresh = Task { await model.refreshSessionMetadata(projectID: "p", sessionID: "s1", database: database) }
        while !(await client.isWaiting()) { await Task.yield() }
        let duplicate = await model.refreshSessionMetadata(projectID: "p", sessionID: "s1", database: database)
        XCTAssertFalse(duplicate)
        await model.openSession("s2")
        await client.finish(); let stale = await refresh.value
        XCTAssertFalse(stale); XCTAssertEqual(model.activeSessionID, "s2")
        XCTAssertEqual(model.sessions.count, 2); XCTAssertEqual(model.messages.first?.text, "s2")
        XCTAssertFalse(model.sessionsLoading)
        let refreshAgain = Task { await model.refreshSessionMetadata(projectID: "p", sessionID: "s2", database: database) }
        while !(await client.isWaiting()) { await Task.yield() }
        model.goHome(); await client.finish(); let left = await refreshAgain.value
        XCTAssertFalse(left); XCTAssertNil(model.activeProjectID); XCTAssertTrue(model.sessions.isEmpty)
    }
    @MainActor func testMetadataRefreshPromotesSavedNativeDraftWithoutReadingHistory() async {
        let client = NavigationClient(); let database = URL(fileURLWithPath: "/unused")
        let model = ProjectBrowserModel(client: client, databaseURL: database)
        await model.openProject("p", sessionID: "s1")
        await model.openNativeDraft("draft", projectID: "p", database: database, sourceSession: "s1")
        await client.configure(rows: [BrowserSession(id: "draft", projectID: "p", title: "First turn", ts: 3, status: "complete", pinned: false)])
        let result = await model.refreshSessionMetadata(projectID: "p", sessionID: "draft", database: database)
        XCTAssertTrue(result); XCTAssertNil(model.sessionError); XCTAssertTrue(model.messages.isEmpty)
        XCTAssertEqual(model.sessions.filter { $0.id == "draft" }.count, 1)
        XCTAssertEqual(model.sessions.first?.pinned, false); XCTAssertEqual(model.activeSessionID, "draft")
        // The saved row no longer takes the native-draft bypass on later navigation.
        await model.openSession("draft")
        XCTAssertNotNil(model.sessionError)
    }
    @MainActor func testTerminalCacheSeparatesSessionsAndKeepsSelectionAcrossPanelReopen() {
        let model = ProjectBrowserModel(client: NavigationClient(), databaseURL: URL(fileURLWithPath: "/tmp/terminal-scope-test.sqlite"))
        let terminal = model.nativeTerminal(projectID: "p", sessionID: "s")
        terminal.select("terminal-qa")
        XCTAssertTrue(terminal === model.nativeTerminal(projectID: "p", sessionID: "s"))
        XCTAssertEqual(model.nativeTerminal(projectID: "p", sessionID: "s").selectedID, "terminal-qa")
        XCTAssertFalse(terminal === model.nativeTerminal(projectID: "other", sessionID: "s"))
        XCTAssertFalse(terminal === model.nativeTerminal(projectID: "p", sessionID: "other"))
    }
    @MainActor func testSideChatCacheSeparatesProjectAndSessionAndSurvivesPanelReopen() {
        let model = ProjectBrowserModel(client: NavigationClient(), databaseURL: URL(fileURLWithPath: "/tmp/side-chat-test.sqlite"))
        let first = model.nativeSideChat(projectID: "p", sessionID: "s")
        first.draft = "keep this draft"
        XCTAssertTrue(first === model.nativeSideChat(projectID: "p", sessionID: "s"))
        XCTAssertFalse(first === model.nativeSideChat(projectID: "p", sessionID: "other"))
        XCTAssertFalse(first === model.nativeSideChat(projectID: "other", sessionID: "s"))
        XCTAssertEqual(model.nativeSideChat(projectID: "p", sessionID: "s").draft, "keep this draft")
    }

    @MainActor func testWorkflowSettingsRouteDoesNotOpenProjectEditor() {
        let model = ProjectBrowserModel(client: NavigationClient(), databaseURL: URL(fileURLWithPath: "/tmp/unused.sqlite"))
        model.openProjectSettings("p")
        model.openWorkflowSettings()
        XCTAssertTrue(model.settingsPresented)
        XCTAssertEqual(model.settingsSectionID, "workflows")
        XCTAssertNil(model.projectSettingsID)
        model.openProjectSettings("p")
        XCTAssertNil(model.settingsSectionID)
        XCTAssertEqual(model.projectSettingsID, "p")
    }
    @MainActor
    func testRecentSessionOpensExactConversationAndBackClearsWorkspace() async {
        let model = ProjectBrowserModel(client: NavigationClient(), databaseURL: URL(fileURLWithPath: "/unused"))
        await model.openProject("p", sessionID: "s2")
        XCTAssertEqual(model.activeProjectID, "p")
        XCTAssertEqual(model.activeSessionID, "s2")
        XCTAssertEqual(model.messages.first?.text, "s2")
        await model.openSession("s1")
        XCTAssertEqual(model.messages.first?.text, "s1")
        model.searchPresented = true
        model.goHome()
        XCTAssertFalse(model.searchPresented)
        XCTAssertNil(model.activeProjectID)
        XCTAssertNil(model.activeSessionID)
        XCTAssertTrue(model.messages.isEmpty)
        XCTAssertTrue(model.sessions.isEmpty)
    }

    @MainActor
    func testOlderMessagesPrependWithoutReplacingTheCurrentPageOrSelection() async {
        let client = NavigationClient()
        await client.enablePagination()
        let model = ProjectBrowserModel(client: client, databaseURL: URL(fileURLWithPath: "/unused"))
        await model.openProject("p", sessionID: "s1")
        XCTAssertEqual(model.messages.map(\.seq), [21])
        XCTAssertEqual(model.nextBeforeSeq, 21)
        await model.openSession("s1", older: true)
        XCTAssertEqual(model.messages.map(\.seq), [1, 21])
        XCTAssertNil(model.nextBeforeSeq)
        XCTAssertEqual(model.activeSessionID, "s1")
        XCTAssertFalse(model.transcriptLoading)
    }

    @MainActor
    func testMissingRecentSessionDoesNotSilentlyOpenAnotherConversation() async {
        let model = ProjectBrowserModel(client: NavigationClient(), databaseURL: URL(fileURLWithPath: "/unused"))
        await model.openProject("p", sessionID: "deleted-session")
        XCTAssertEqual(model.activeSessionID, "deleted-session")
        XCTAssertTrue(model.messages.isEmpty)
        XCTAssertNotNil(model.sessionError)
    }

    @MainActor
    func testBackDuringQueryCannotReopenAnOldWorkspace() async {
        let client = NavigationClient()
        await client.setSuspended()
        let model = ProjectBrowserModel(client: client, databaseURL: URL(fileURLWithPath: "/unused"))
        let opening = Task { await model.openProject("p") }
        while !(await client.isWaiting()) { await Task.yield() }
        model.goHome()
        await client.finish()
        await opening.value
        XCTAssertNil(model.activeProjectID)
        XCTAssertTrue(model.sessions.isEmpty)
        XCTAssertFalse(model.sessionsLoading)
    }
    @MainActor
    func testNativeDraftSurvivesNavigationAndLateOtherDatabaseCannotInsert() async {
        let database = URL(fileURLWithPath: "/unused")
        let model = ProjectBrowserModel(client: NavigationClient(), databaseURL: database)
        await model.openProject("p")
        await model.openNativeDraft("draft", projectID: "p", database: database, sourceSession: model.activeSessionID)
        XCTAssertEqual(model.activeSessionID, "draft")
        XCTAssertNil(model.sessionError)
        XCTAssertTrue(model.messages.isEmpty)
        XCTAssertFalse(model.transcriptLoading)
        model.goHome()
        await model.openProject("p", sessionID: "draft")
        XCTAssertEqual(model.activeSessionID, "draft"); XCTAssertNil(model.sessionError)
        await model.openNativeDraft("foreign", projectID: "p", database: URL(fileURLWithPath: "/old-db"), sourceSession: model.activeSessionID)
        XCTAssertFalse(model.sessions.contains { $0.id == "foreign" })
        XCTAssertEqual(model.activeSessionID, "draft")
        await model.openNativeDraft("late-same-project", projectID: "p", database: database, sourceSession: "old-selection")
        XCTAssertEqual(model.activeSessionID, "draft")
        await model.openProject("other")
        let selected = model.activeSessionID
        await model.openNativeDraft("late", projectID: "p", database: database, sourceSession: model.activeSessionID)
        XCTAssertEqual(model.activeProjectID, "other"); XCTAssertEqual(model.activeSessionID, selected)
    }

}
