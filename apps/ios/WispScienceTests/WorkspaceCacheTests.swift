import XCTest
import WispRemote
@testable import WispScience

@MainActor final class WorkspaceCacheTests: XCTestCase {
    func testOfflineHydrationAndNewConnectionRequireFreshSnapshotBeforeApproval() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let cache = RemoteReadCache(directory: directory)
        let credentials = CacheCredentials()
        let config = try XCTUnwrap(credentials.load())
        let scope = RemoteReadCache.scope(config)
        let dataset = try DemoDataset(bundle: Bundle(for: WispAppDelegate.self))
        let route = ConversationRoute(project: dataset.projectID, session: "demo-workflow")
        await cache.store(try dataset.call("list_projects", project: nil, args: [:]), scope: scope, command: "list_projects")
        await cache.store(try dataset.call("remote_sessions", project: route.project, args: [:]), scope: scope, command: "remote_sessions", project: route.project)
        await cache.store(try dataset.call("native_conversation_snapshot", project: route.project, args: route.args),
                          scope: scope, command: "native_conversation_snapshot", project: route.project, args: route.args)
        let connection = RemoteConnection(socketFactory: { _ in fatalError("Offline hydration must not create a socket") })
        let model = RemoteWorkspace(connection: connection, credentials: credentials, preferences: nil, cache: cache)
        model.setActive(false)
        model.restore()
        await model.refreshSelection()
        XCTAssertEqual(model.projects.count, 1)
        model.selectedProject = route.project; model.selectedSession = route.session
        await model.refreshSelection()
        XCTAssertEqual(model.sessions.count, 3)
        XCTAssertTrue(model.snapshotIsCached)
        XCTAssertFalse(model.canActOnSnapshot)
        XCTAssertEqual(model.snapshot?.value["approvals"].array.count, 1)
        var approvals = 0
        connection.useLocalService(name: "fake", commands: DemoDataset.commands) { command, project, args in
            if command == "native_conversation_approve" { approvals += 1 }
            return try dataset.call(command, project: project, args: args)
        }
        let decision: [String: RemoteValue] = ["approval_id": .string("demo-approval"), "approved": .bool(true)]
        await model.action("native_conversation_approve", route: route, extra: decision)
        XCTAssertEqual(approvals, 0)
        await model.refreshSelection()
        XCTAssertTrue(model.canActOnSnapshot)
        // Even a new transport that has already connected cannot use the old snapshot.
        connection.useLocalService(name: "reconnected", commands: DemoDataset.commands) { command, project, args in
            if command == "native_conversation_approve" { approvals += 1 }
            return try dataset.call(command, project: project, args: args)
        }
        XCTAssertFalse(model.canActOnSnapshot)
        await model.action("native_conversation_approve", route: route, extra: decision)
        XCTAssertEqual(approvals, 0)
        await model.refreshSelection()
        await model.action("native_conversation_approve", route: route, extra: decision)
        XCTAssertEqual(approvals, 1)
        XCTAssertTrue(model.snapshot?.value["approvals"].array.isEmpty == true)
        await model.clearCachedData()
        let cached = await cache.read(scope: scope, command: "list_projects")
        XCTAssertNil(cached)
        XCTAssertNotNil(try credentials.load(), "Clearing cached data must preserve the Keychain credential")
        connection.disconnect()
    }

    func testReadFacadeRejectsMutationCommandsBeforeSending() async throws {
        let connection = RemoteConnection()
        var calls = 0
        connection.useLocalService(name: "fake", commands: ["list_projects", "native_conversation_send"]) { _, _, _ in
            calls += 1; return .array([])
        }
        let model = RemoteWorkspace(connection: connection, credentials: CacheCredentials(), preferences: nil, cache: nil)
        _ = try await model.read("list_projects")
        XCTAssertEqual(calls, 1)
        do { _ = try await model.read("native_conversation_send"); XCTFail("Mutations must bypass the read cache") }
        catch { XCTAssertEqual(error as? RemoteError, .unsupported) }
        XCTAssertEqual(calls, 1)
    }
}

private final class CacheCredentials: RemoteCredentialStoring {
    func load() throws -> RemoteConfiguration? {
        try RemoteConfiguration(address: "https://cache.example.test", code: "000102030405060708090a0b0c0d0e0f")
    }
    func save(_ configuration: RemoteConfiguration) throws { }
    func remove() throws { }
}
