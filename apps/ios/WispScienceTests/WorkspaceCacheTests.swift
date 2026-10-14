import XCTest
import UIKit
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

@MainActor final class WorkspaceUXTests: XCTestCase {
    private func row(_ id: String, _ activity: Int64?) -> RemoteValue {
        var fields: [String: RemoteValue] = ["id": .string(id)]
        if let activity { fields["activity_at"] = .integer(activity) }
        return .object(fields)
    }

    func testSessionsSortByActivityAndKeepHostOrderForTiesAndMissingStamps() {
        let rows = [row("old", 100), row("undated-a", nil), row("new", 300), row("tie-a", 200), row("tie-b", 200), row("undated-b", nil)]
        XCTAssertEqual(RemoteWorkspace.ordered(rows).map { $0["id"].string },
                       ["new", "tie-a", "tie-b", "old", "undated-a", "undated-b"])
    }

    func testActivityLabelsUseTimeTodayYesterdayAndDates() throws {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = try XCTUnwrap(TimeZone(identifier: "Asia/Shanghai"))
        let now = try XCTUnwrap(calendar.date(from: DateComponents(year: 2026, month: 10, day: 13, hour: 15)))
        let morning = try XCTUnwrap(calendar.date(from: DateComponents(year: 2026, month: 10, day: 13, hour: 9, minute: 5)))
        XCTAssertTrue(WispTime.activity(morning, now: now, calendar: calendar).contains("9:05"))
        let yesterday = try XCTUnwrap(calendar.date(byAdding: .hour, value: -20, to: now))
        XCTAssertEqual(WispTime.activity(yesterday, now: now, calendar: calendar), "昨天")
        let earlier = try XCTUnwrap(calendar.date(from: DateComponents(year: 2026, month: 3, day: 2, hour: 12)))
        XCTAssertEqual(WispTime.activity(earlier, now: now, calendar: calendar), "3月2日")
        let lastYear = try XCTUnwrap(calendar.date(from: DateComponents(year: 2025, month: 12, day: 31, hour: 12)))
        XCTAssertEqual(WispTime.activity(lastYear, now: now, calendar: calendar), "2025年12月31日")
        XCTAssertEqual(WispTime.date(.integer(1_700_000_000)), Date(timeIntervalSince1970: 1_700_000_000))
        XCTAssertEqual(WispTime.date(.integer(1_700_000_000_000)), Date(timeIntervalSince1970: 1_700_000_000))
        XCTAssertNil(WispTime.date(.integer(0)))
        XCTAssertNil(WispTime.date(.null))
    }

    func testComposerNoticeOffersRetryOnlyWhenTheRequestDidNotReachTheHost() {
        XCTAssertEqual(ComposerNotice.failure(RemoteError.service("拒绝"), retry: .send).retry, .send)
        XCTAssertNil(ComposerNotice.failure(RemoteError.outcomeUnknown, retry: .send).retry)
        XCTAssertNil(ComposerNotice.failure(RemoteError.disconnected, retry: .upload).retry)
    }

    private func workspace(send: @escaping () throws -> RemoteValue,
                           attach: @escaping () throws -> RemoteValue = { .object(["path": .string("uploads/a.jpg")]) })
        async throws -> (RemoteWorkspace, ConversationRoute) {
        let demo = try DemoDataset(bundle: Bundle(for: WispAppDelegate.self))
        let connection = RemoteConnection(socketFactory: { _ in fatalError("No network") })
        connection.useLocalService(name: "fake", commands: DemoDataset.commands.union(["remote_attach_image"])) { command, project, args in
            if command == "native_conversation_send" { return try send() }
            if command == "remote_attach_image" { return try attach() }
            return try demo.call(command, project: project, args: args)
        }
        let model = RemoteWorkspace(connection: connection, credentials: CacheCredentials(), preferences: nil, cache: nil)
        let route = ConversationRoute(project: demo.projectID, session: "demo-qc")
        model.selectedProject = route.project; model.selectedSession = route.session
        await model.refreshSelection()
        XCTAssertTrue(model.canActOnSnapshot)
        return (model, route)
    }

    func testSendFailureStaysAtTheComposerAndRetryKeepsTheDraft() async throws {
        var sends = 0
        var failure: RemoteError? = .service("主机拒绝")
        let (model, route) = try await workspace(send: {
            sends += 1
            if let failure { throw failure }
            return .null
        })
        model.drafts[route] = "检查样本"
        await model.send(route)
        XCTAssertNil(model.error, "Send failures no longer use the global banner")
        XCTAssertEqual(model.composerNotices[route], ComposerNotice(message: "主机拒绝", retry: .send))
        XCTAssertEqual(model.drafts[route], "检查样本")
        failure = nil
        await model.retryComposer(route)
        XCTAssertEqual(sends, 2)
        XCTAssertNil(model.composerNotices[route])
        XCTAssertEqual(model.drafts[route], "")
    }

    func testUnknownSendOutcomeIsNeverOfferedForRetry() async throws {
        var sends = 0
        let (model, route) = try await workspace(send: { sends += 1; throw RemoteError.outcomeUnknown })
        model.drafts[route] = "检查样本"
        await model.send(route)
        XCTAssertNil(model.composerNotices[route]?.retry)
        await model.retryComposer(route)
        XCTAssertEqual(sends, 1)
    }

    func testFailedUploadRetriesWithTheOriginalImage() async throws {
        var fail = true
        let (model, route) = try await workspace(send: { .null }, attach: {
            if fail { throw RemoteError.service("磁盘已满") }
            return .object(["path": .string("uploads/a.jpg")])
        })
        let image = UIGraphicsImageRenderer(size: CGSize(width: 20, height: 20)).jpegData(withCompressionQuality: 1) { context in
            UIColor.gray.setFill(); context.fill(CGRect(x: 0, y: 0, width: 20, height: 20))
        }
        await model.upload(image, route: route)
        XCTAssertEqual(model.composerNotices[route]?.retry, .upload)
        XCTAssertNil(model.error)
        fail = false
        await model.retryComposer(route)
        XCTAssertEqual(model.photos[route]?.count, 1)
        XCTAssertNil(model.composerNotices[route])
    }
}

private final class CacheCredentials: RemoteCredentialStoring {
    func load() throws -> RemoteConfiguration? {
        try RemoteConfiguration(address: "https://cache.example.test", code: "000102030405060708090a0b0c0d0e0f")
    }
    func save(_ configuration: RemoteConfiguration) throws { }
    func remove() throws { }
}
