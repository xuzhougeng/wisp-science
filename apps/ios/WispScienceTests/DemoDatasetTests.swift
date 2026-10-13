import XCTest
import UIKit
import WispRemote
@testable import WispScience

@MainActor final class DemoDatasetTests: XCTestCase {
    private func dataset() throws -> DemoDataset { try DemoDataset(bundle: Bundle(for: WispAppDelegate.self)) }

    func testBundledSessionsUseValidSnapshotsAndEveryFileOpensLocally() throws {
        let demo = try dataset()
        let rows = try demo.call("remote_sessions", project: demo.projectID, args: [:]).array
        XCTAssertEqual(rows.count, 3)
        for row in rows {
            let route = ConversationRoute(project: demo.projectID, session: row["id"].string)
            let snapshot = try RemoteSnapshot(demo.call("native_conversation_snapshot", project: route.project, args: route.args), route: route)
            XCTAssertFalse(snapshot.items.isEmpty)
            XCTAssertFalse(snapshot.running)
            XCTAssertTrue(snapshot.writable)
        }
        let route = ConversationRoute(project: demo.projectID, session: "demo-qc")
        var directories = ["."]
        var opened: Set<String> = []
        while let directory = directories.popLast() {
            let value = try demo.call("native_conversation_panel_file_directory", project: route.project,
                                      args: route.args.merging(["path": .string(directory)]) { _, v in v })
            for file in value["entries"].array {
                let path = directory == "." ? file["name"].string : directory + "/" + file["name"].string
                if file["is_dir"].bool { directories.append(path); continue }
                opened.insert(path)
                let image = path.hasSuffix(".png")
                let value = try demo.call(image ? "native_conversation_image" : "native_conversation_panel_file_read", project: route.project,
                                          args: route.args.merging(["path": .string(path)]) { _, v in v })
                if image {
                    let bytes = try XCTUnwrap(Data(base64Encoded: value["base64"].string))
                    XCTAssertNotNil(UIImage(data: bytes))
                } else { XCTAssertFalse(value["content"]["text"].string.isEmpty) }
            }
        }
        XCTAssertEqual(opened, ["README.md", "data/samples.csv", "data/qc_metrics.csv", "results/qc_report.md", "results/qc_summary.png", "scripts/qc_demo.py"])
    }

    func testScriptedReplyAndApprovalAreEphemeralAndExplicitlyLabeled() throws {
        let demo = try dataset()
        let route = ConversationRoute(project: demo.projectID, session: "demo-workflow")
        _ = try demo.call("native_conversation_send", project: route.project,
                          args: route.args.merging(["message": .string("查看报告")]) { _, v in v })
        var value = try demo.call("native_conversation_snapshot", project: route.project, args: route.args)
        XCTAssertTrue(value["items"].array.last?["text"].string.contains("本地预设回复") == true)
        _ = try demo.call("native_conversation_approve", project: route.project,
                          args: route.args.merging(["approval_id": .string("demo-approval"), "approved": .bool(true)]) { _, v in v })
        value = try demo.call("native_conversation_snapshot", project: route.project, args: route.args)
        XCTAssertTrue(value["approvals"].array.isEmpty)
        XCTAssertTrue(value["items"].array.last?["text"].string.contains("没有运行命令") == true)
        let reset = try dataset().call("native_conversation_snapshot", project: route.project, args: route.args)
        XCTAssertEqual(reset["approvals"].array.count, 1)
        XCTAssertEqual(reset["items"].array.count, 2)
        XCTAssertNotEqual(value["epoch"], reset["epoch"])
    }

    func testUnknownProjectSessionAndUnbundledPathsAreRejected() throws {
        let demo = try dataset()
        XCTAssertThrowsError(try demo.call("remote_sessions", project: "real-project", args: [:]))
        XCTAssertThrowsError(try demo.call("native_conversation_snapshot", project: demo.projectID, args: ["session_id": .string("real-session")]))
        for path in ["/etc/passwd", "../manifest.json", "data/../../secret", "https://example.com/file", "unlisted.txt"] {
            XCTAssertThrowsError(try demo.call("native_conversation_panel_file_read", project: demo.projectID,
                                               args: ["session_id": .string("demo-qc"), "path": .string(path)]))
        }
        XCTAssertThrowsError(try demo.call("shell", project: demo.projectID, args: [:]))
    }

    func testOptionAnswerPreservesComposerAndSettlesWithoutDuplicateSends() async throws {
        let demo = try dataset()
        let connection = RemoteConnection(socketFactory: { _ in fatalError("Options demo must stay offline") })
        var sent: [[String: RemoteValue]] = []
        connection.useLocalService(name: "options", commands: DemoDataset.commands) { command, project, args in
            if command == "native_conversation_send" { sent.append(args) }
            return try demo.call(command, project: project, args: args)
        }
        let model = RemoteWorkspace(connection: connection, credentials: CredentialsSpy(), preferences: nil, cache: nil)
        let route = ConversationRoute(project: demo.projectID, session: "demo-options")
        model.selectedProject = route.project; model.selectedSession = route.session
        try await model.refreshConversation()
        let item = try XCTUnwrap(model.snapshot?.items.last)
        let id = model.questionID(at: 1, item: item, route: route)
        XCTAssertTrue(model.canAnswerQuestion(at: 1, item: item, route: route))
        model.drafts[route] = "保留我的草稿"
        model.photos[route] = [AttachedPhoto(path: "draft.png", data: Data([1]))]
        await model.answerQuestion(at: 1, item: item, id: id, route: route, option: 2, freeform: "重点看 Q30")
        XCTAssertEqual(sent.count, 1)
        XCTAssertEqual(sent[0]["session_id"]?.string, route.session)
        XCTAssertEqual(sent[0]["attachments"], .array([]))
        XCTAssertTrue(sent[0]["message"]?.string.hasPrefix("使用自定义标准") == true)
        XCTAssertTrue(sent[0]["message"]?.string.contains("补充说明：重点看 Q30") == true)
        XCTAssertEqual(model.drafts[route], "保留我的草稿")
        XCTAssertEqual(model.photos[route]?.first?.path, "draft.png")
        XCTAssertEqual(model.question(at: 1, item: item)?.state, .answered)
        XCTAssertEqual(model.questionSubmissions[id], .sent)
        await model.answerQuestion(at: 1, item: item, id: id, route: route, option: 0, freeform: "")
        XCTAssertEqual(sent.count, 1)
        model.selectedSession = "demo-qc"; model.selectedSession = route.session
        try await model.refreshConversation()
        XCTAssertFalse(model.canAnswerQuestion(at: 1, item: item, route: route))
        let reset = try dataset().call("native_conversation_snapshot", project: route.project, args: route.args)
        XCTAssertEqual(RemoteQuestion(try XCTUnwrap(reset["items"].array.last))?.state, .pending)
    }

    func testQuestionGatesRunningReadOnlyStaleAndDisconnectedSnapshots() async throws {
        let demo = try dataset()
        let connection = RemoteConnection(socketFactory: { _ in fatalError("No network") })
        var overrides: [String: RemoteValue] = [:]
        var sends = 0
        connection.useLocalService(name: "options", commands: DemoDataset.commands) { command, project, args in
            if command == "native_conversation_send" { sends += 1 }
            let value = try demo.call(command, project: project, args: args)
            if command == "native_conversation_snapshot", case .object(let object) = value {
                return .object(object.merging(overrides) { _, v in v })
            }
            return value
        }
        let model = RemoteWorkspace(connection: connection, credentials: CredentialsSpy(), preferences: nil, cache: nil)
        let route = ConversationRoute(project: demo.projectID, session: "demo-options")
        model.selectedProject = route.project; model.selectedSession = route.session
        try await model.refreshConversation()
        let item = try XCTUnwrap(model.snapshot?.items.last)
        let id = model.questionID(at: 1, item: item, route: route)
        let cases: [[String: RemoteValue]] = [["running": .bool(true)], ["read_only": .bool(true)], ["acp_agent_id": .string("acp")]]
        for values in cases {
            overrides = values
            try await model.refreshConversation()
            XCTAssertFalse(model.canAnswerQuestion(at: 1, item: item, route: route))
            await model.answerQuestion(at: 1, item: item, id: id, route: route, option: 0, freeform: "")
        }
        overrides = ["epoch": .string("replacement")]
        try await model.refreshConversation()
        await model.answerQuestion(at: 1, item: item, id: id, route: route, option: 0, freeform: "")
        let newID = model.questionID(at: 1, item: item, route: route)
        connection.disconnect()
        XCTAssertFalse(model.canAnswerQuestion(at: 1, item: item, route: route))
        await model.answerQuestion(at: 1, item: item, id: newID, route: route, option: 0, freeform: "")
        XCTAssertEqual(sends, 0)
    }

    func testFailedAnswerRemainsPendingAndUnknownOutcomeCannotBeResubmitted() async throws {
        let demo = try dataset()
        let connection = RemoteConnection(socketFactory: { _ in fatalError("No network") })
        var failure = RemoteError.service("模拟拒绝")
        var sends = 0
        connection.useLocalService(name: "options", commands: DemoDataset.commands) { command, project, args in
            if command == "native_conversation_send" { sends += 1; throw failure }
            return try demo.call(command, project: project, args: args)
        }
        let model = RemoteWorkspace(connection: connection, credentials: CredentialsSpy(), preferences: nil, cache: nil)
        let route = ConversationRoute(project: demo.projectID, session: "demo-options")
        model.selectedProject = route.project; model.selectedSession = route.session
        try await model.refreshConversation()
        let item = try XCTUnwrap(model.snapshot?.items.last)
        let id = model.questionID(at: 1, item: item, route: route)
        await model.answerQuestion(at: 1, item: item, id: id, route: route, option: nil, freeform: "自己的标准")
        XCTAssertEqual(sends, 1)
        XCTAssertNil(model.questionSubmissions[id])
        XCTAssertEqual(model.error, "模拟拒绝")
        XCTAssertTrue(model.canAnswerQuestion(at: 1, item: item, route: route))
        failure = .outcomeUnknown
        await model.answerQuestion(at: 1, item: item, id: id, route: route, option: 0, freeform: "")
        XCTAssertEqual(model.questionSubmissions[id], .uncertain)
        try await model.refreshConversation()
        await model.answerQuestion(at: 1, item: item, id: id, route: route, option: 0, freeform: "")
        XCTAssertEqual(sends, 2)
        XCTAssertFalse(model.canAnswerQuestion(at: 1, item: item, route: route))
    }

    func testReadingDemoQuestionDoesNotClearPendingUntilItIsAnswered() throws {
        let demo = try dataset()
        let route = ConversationRoute(project: demo.projectID, session: "demo-options")
        _ = try demo.call("native_conversation_seen", project: route.project, args: route.args)
        func status() throws -> String? {
            try demo.call("remote_sessions", project: route.project, args: [:]).array.first { $0["id"].string == route.session }?["status"].string
        }
        XCTAssertEqual(try status(), "needs_you")
        _ = try demo.call("native_conversation_send", project: route.project,
            args: route.args.merging(["message": .string("按建议标准查看")]) { _, v in v })
        XCTAssertEqual(try status(), "idle")
    }

    func testAcceptedAnswerStaysSettledWhenSnapshotRefreshFails() async throws {
        let demo = try dataset()
        let connection = RemoteConnection(socketFactory: { _ in fatalError("No network") })
        var sends = 0
        connection.useLocalService(name: "options", commands: DemoDataset.commands) { command, project, args in
            if command == "native_conversation_send" { sends += 1; return .null }
            if command == "native_conversation_snapshot", sends > 0 { throw RemoteError.disconnected }
            return try demo.call(command, project: project, args: args)
        }
        let model = RemoteWorkspace(connection: connection, credentials: CredentialsSpy(), preferences: nil, cache: nil)
        let route = ConversationRoute(project: demo.projectID, session: "demo-options")
        model.selectedProject = route.project; model.selectedSession = route.session
        try await model.refreshConversation()
        let item = try XCTUnwrap(model.snapshot?.items.last)
        let id = model.questionID(at: 1, item: item, route: route)
        await model.answerQuestion(at: 1, item: item, id: id, route: route, option: nil, freeform: "直接回答")
        XCTAssertEqual(model.questionSubmissions[id], .sent)
        XCTAssertTrue(model.snapshotIsCached)
        await model.answerQuestion(at: 1, item: item, id: id, route: route, option: nil, freeform: "直接回答")
        XCTAssertEqual(sends, 1)
    }

    func testDemoLifecycleDoesNotTouchSavedCredentialsAndRestoresWithoutRemote() async throws {
        let credentials = CredentialsSpy()
        let suite = "wisp-demo-test-" + UUID().uuidString
        let preferences = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { preferences.removePersistentDomain(forName: suite) }
        let model = RemoteWorkspace(credentials: credentials, preferences: preferences)
        model.enterDemo()
        await model.refreshSelection()
        XCTAssertTrue(model.isDemo)
        XCTAssertEqual(model.projects.count, 1)
        XCTAssertNil(model.configuration)
        model.setActive(false)
        model.setActive(true)
        XCTAssertEqual(model.connection.state, .online)
        let relaunched = RemoteWorkspace(credentials: credentials, preferences: preferences)
        relaunched.restore()
        XCTAssertTrue(relaunched.isDemo)
        relaunched.exitDemo()
        model.exitDemo()
        await Task.yield()
        XCTAssertNil(model.error)
        XCTAssertTrue(model.projects.isEmpty)
        XCTAssertEqual(model.connection.state, .idle)
        XCTAssertEqual(credentials.accesses, 0)
    }
}

private final class CredentialsSpy: RemoteCredentialStoring {
    var accesses = 0
    func load() throws -> RemoteConfiguration? { accesses += 1; return nil }
    func save(_ configuration: RemoteConfiguration) throws { accesses += 1 }
    func remove() throws { accesses += 1 }
}
