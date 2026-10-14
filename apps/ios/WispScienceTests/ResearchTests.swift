import XCTest
import WispRemote
@testable import WispScience

@MainActor final class ResearchTests: XCTestCase {
    private let empty: RemoteValue = .object(["entries": .array([]), "truncated": .bool(false)])
    private func query(_ project: String = "p") -> ResearchQuery {
        ResearchQuery(project: project, projectIDs: [], interval: DateInterval(start: Date(timeIntervalSince1970: 0), duration: 86400))
    }
    private func connection() -> RemoteConnection {
        let connection = RemoteConnection(socketFactory: { _ in fatalError("Research tests must not use the network") })
        connection.useLocalService(name: "research", commands: ["native_research_journey", "native_research_calendar"]) { _, _, _ in .null }
        return connection
    }

    func testOldHostOfflineAndMalformedStatesAreDistinctFromEmpty() async {
        let connection = connection()
        let loader = ResearchLoader { _, _ in self.empty }
        await loader.load(query(), connection: connection)
        XCTAssertNotNil(loader.history)
        XCTAssertNil(loader.error)
        connection.useLocalService(name: "old", commands: []) { _, _, _ in XCTFail("Unsupported commands must not be sent"); return .null }
        await loader.load(query(), connection: connection)
        XCTAssertTrue(loader.error?.contains("更新") == true)
        XCTAssertNil(loader.history)
        connection.disconnect()
        await loader.load(query(), connection: connection)
        XCTAssertTrue(loader.error?.contains("连接") == true)
        let malformed = ResearchLoader { _, _ in .null }
        await malformed.load(query(), connection: self.connection())
        XCTAssertNotNil(malformed.error)
        XCTAssertNil(malformed.history)
    }

    func testOlderQueryAndOldConnectionCannotPublishResults() async {
        let connection = connection()
        var waiting: CheckedContinuation<RemoteValue, Error>?
        let loader = ResearchLoader { _, query in
            if query.project == "old" { return try await withCheckedThrowingContinuation { waiting = $0 } }
            return self.empty
        }
        let old = Task { await loader.load(query("old"), connection: connection) }
        while waiting == nil { await Task.yield() }
        await loader.load(query("new"), connection: connection)
        waiting?.resume(throwing: RemoteError.service("stale error"))
        await old.value
        XCTAssertNotNil(loader.history)
        XCTAssertNil(loader.error)
        waiting = nil
        let disconnected = Task { await loader.load(query("old"), connection: connection) }
        while waiting == nil { await Task.yield() }
        connection.disconnect()
        waiting?.resume(returning: empty)
        await disconnected.value
        XCTAssertNil(loader.history)
        XCTAssertNil(loader.error)
        XCTAssertFalse(loader.busy)
    }

    func testCalendarBatchesLargeProjectListsWithoutOmission() async {
        var sizes: [Int] = []
        let loader = ResearchLoader { _, query in
            XCTAssertNil(query.project)
            sizes.append(query.projectIDs.count)
            return .array(query.projectIDs.map { .object(["project_id": .string($0), "history": self.empty, "error": .null]) })
        }
        let ids = (0..<105).map { "p-\($0)" }
        await loader.load(ResearchQuery(project: nil, projectIDs: ids, interval: query().interval), connection: connection())
        XCTAssertEqual(sizes, [50, 50, 5])
        XCTAssertEqual(loader.projects.map(\.id), ids)
    }

    func testDemoCalendarDayHistoryScopeAndWindowEscapeStack() async throws {
        let now = Date(timeIntervalSince1970: 1_792_000_000)
        let demo = try DemoDataset(bundle: Bundle(for: WispAppDelegate.self), now: now)
        let clock = RemoteResearchClock.localCalendar()
        let args = RemoteResearchClock.args(RemoteResearchClock.interval(.day, containing: now, calendar: clock))
        let value = try demo.call("native_research_journey", project: demo.projectID, args: args)
        let history = try RemoteResearchHistory(value)
        XCTAssertEqual(history.entries.count, 3)
        var calendarArgs = args
        calendarArgs["project_ids"] = .array([.string(demo.projectID)])
        let calendar = try RemoteResearchProject.decode(demo.call("native_research_calendar", project: nil, args: calendarArgs), requested: [demo.projectID])
        XCTAssertEqual(calendar[0].history.entries, history.entries)
        XCTAssertThrowsError(try demo.call("native_research_journey", project: "other", args: args))
        let connection = connection()
        let workspace = RemoteWorkspace(connection: connection, preferences: nil, cache: nil)
        workspace.sheet = .research(nil)
        let entry = try XCTUnwrap(history.entries.first { $0.sessionID == "demo-qc" })
        workspace.researchPath = [.journey(demo.projectID, Date()), .entry(demo.projectID, entry)]
        workspace.dismissTopSurface()
        XCTAssertEqual(workspace.researchPath.count, 1)
        XCTAssertNotNil(workspace.sheet)
        workspace.dismissTopSurface()
        XCTAssertTrue(workspace.researchPath.isEmpty)
        XCTAssertNotNil(workspace.sheet)
        workspace.dismissTopSurface()
        XCTAssertNil(workspace.sheet)
        workspace.sheet = .research(demo.projectID)
        workspace.openResearchConversation(entry, project: demo.projectID)
        XCTAssertEqual(workspace.route, ConversationRoute(project: demo.projectID, session: "demo-qc"))
        XCTAssertNil(workspace.sheet)
    }
}
