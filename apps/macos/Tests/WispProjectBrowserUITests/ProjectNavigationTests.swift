import Foundation
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor NavigationClient: ProjectBrowserQuerying {
    var continuation: CheckedContinuation<[BrowserSession], Error>?
    var suspend = false
    var paginated = false
    func enablePagination() { paginated = true }
    func setSuspended() { suspend = true }
    func isWaiting() -> Bool { continuation != nil }
    func finish() { continuation?.resume(returning: []); continuation = nil }
    func listProjects(databaseURL: URL) async throws -> ProjectListSnapshot {
        ProjectListSnapshot(projects: [], activitySource: "persisted_only")
    }
    func listSessions(databaseURL: URL, projectID: String?) async throws -> [BrowserSession] {
        if suspend { return try await withCheckedThrowingContinuation { continuation = $0 } }
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
