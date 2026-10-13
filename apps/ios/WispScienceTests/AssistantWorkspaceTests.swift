import XCTest
import WispRemote
@testable import WispScience

@MainActor final class AssistantWorkspaceTests: XCTestCase {
    private func workspace(_ host: AssistantHost) -> RemoteWorkspace {
        let connection = RemoteConnection(socketFactory: { _ in fatalError("Assistant tests must not open a socket") })
        connection.useLocalService(name: "assistant", commands: host.commands, handler: host.call)
        return RemoteWorkspace(connection: connection, credentials: AssistantCredentials(), preferences: nil, cache: nil)
    }

    func testPersistentAssistantIsSeparateFromProjectsAndPreservesPreviousSelectionAndDraft() async throws {
        let host = try AssistantHost()
        let model = workspace(host)
        let previous = ConversationRoute(project: host.demo.projectID, session: "demo-qc")
        model.selectedProject = previous.project; model.selectedSession = previous.session
        model.drafts[previous] = "项目草稿"
        await model.refreshSelection()
        XCTAssertTrue(host.opens.isEmpty, "Opening a workspace must not create an assistant conversation")
        model.openAssistant()
        await model.refreshSelection()
        XCTAssertEqual(model.route, .researchAssistant)
        XCTAssertEqual(model.title, "科研助理")
        XCTAssertTrue(model.canActOnSnapshot)
        XCTAssertFalse(model.projects.contains { $0["id"].string == ConversationRoute.researchAssistant.project })
        XCTAssertFalse(model.sessions.contains { $0["id"].string == ConversationRoute.researchAssistant.session })
        XCTAssertEqual(host.opens.count, 1)
        XCTAssertNil(host.opens[0].project)
        XCTAssertTrue(host.opens[0].args.isEmpty)
        model.drafts[.researchAssistant] = "汇总目前研究进展"
        await model.send(.researchAssistant)
        XCTAssertTrue(model.snapshot?.items.contains { $0["role"].string == "user" && $0["text"].string == "汇总目前研究进展" } == true)
        model.closeAssistant()
        XCTAssertNil(model.snapshot, "Never flash assistant messages inside the previous project")
        XCTAssertEqual(model.route, previous)
        XCTAssertEqual(model.drafts[previous], "项目草稿")
        await model.refreshSelection()
        XCTAssertFalse(model.snapshot?.items.contains { $0["text"].string.contains("本地科研助理演示") } == true)
        model.openAssistant()
        await model.refreshSelection()
        XCTAssertEqual(host.opens.count, 1, "Resolve once per connection, never create another assistant")
        XCTAssertTrue(model.snapshot?.items.contains { $0["text"].string == "汇总目前研究进展" } == true)
        await model.createConversation()
        XCTAssertFalse(host.requests.contains { $0.command == "native_conversation_create" })
    }

    func testOlderHostsAndUnexpectedBindingsNeverSendToAGuessedConversation() async throws {
        for supported in [false, true] {
            let host = try AssistantHost()
            if !supported { host.commands.remove("native_assistant_open") }
            else { host.openResult = .object(["project_id": .string("wrong"), "session_id": .string("wrong")]) }
            let model = workspace(host)
            model.openAssistant()
            await model.refreshSelection()
            XCTAssertNil(model.snapshot)
            XCTAssertFalse(model.canActOnSnapshot)
            XCTAssertNotNil(model.error)
            XCTAssertEqual(host.opens.count, supported ? 1 : 0)
            XCTAssertFalse(host.requests.contains { $0.command == "native_conversation_snapshot" })
            model.drafts[.researchAssistant] = "不要发送"
            await model.send(.researchAssistant)
            XCTAssertFalse(host.requests.contains { $0.command == "native_conversation_send" })
            model.closeAssistant()
            XCTAssertNil(model.error)
        }
    }

    func testEscapeClosesTopSheetThenAssistantAndRestoresProject() async throws {
        let model = workspace(try AssistantHost())
        model.selectedProject = "demo-rnaseq"; model.selectedSession = "demo-qc"
        model.openAssistant()
        model.sheet = .conversationInfo("科研助理", "")
        model.dismissTopSurface()
        XCTAssertNil(model.sheet)
        XCTAssertTrue(model.showingAssistant)
        XCTAssertTrue(model.hasDismissibleSurface)
        model.dismissTopSurface()
        XCTAssertFalse(model.hasDismissibleSurface)
        XCTAssertEqual(model.route, ConversationRoute(project: "demo-rnaseq", session: "demo-qc"))
    }

    func testClosingDuringOpenDoesNotReplaceProjectAndReconnectRequiresNewOpen() async throws {
        let host = try AssistantHost()
        let model = workspace(host)
        model.selectedProject = host.demo.projectID; model.selectedSession = "demo-qc"
        model.openAssistant()
        host.onOpen = { model.closeAssistant() }
        await model.refreshSelection()
        XCTAssertFalse(model.showingAssistant)
        XCTAssertNil(model.snapshot)
        XCTAssertNil(model.error)
        host.onOpen = nil
        model.openAssistant()
        await model.refreshSelection()
        XCTAssertTrue(model.canActOnSnapshot)
        model.connection.useLocalService(name: "reconnected", commands: host.commands, handler: host.call)
        XCTAssertFalse(model.canActOnSnapshot)
        await model.refreshSelection()
        XCTAssertEqual(host.opens.count, 2)
        XCTAssertTrue(model.canActOnSnapshot)
    }

    func testAssistantSnapshotLoadsOfflineAfterRelaunchAndClearsWithOtherCache() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let cache = RemoteReadCache(directory: directory)
        let credentials = AssistantCredentials()
        let scope = RemoteReadCache.scope(try XCTUnwrap(credentials.load()))
        let demo = try AssistantHost().demo
        let route = ConversationRoute.researchAssistant
        let snapshot = try demo.call("native_conversation_snapshot", project: route.project, args: route.args)
        await cache.store(snapshot, scope: scope, command: "native_conversation_snapshot", project: route.project, args: route.args)
        func offline() -> RemoteWorkspace {
            let model = RemoteWorkspace(connection: RemoteConnection(socketFactory: { _ in fatalError("No network") }),
                credentials: credentials, preferences: nil, cache: cache)
            model.setActive(false); model.restore(); model.openAssistant()
            return model
        }
        let model = offline()
        await model.refreshSelection()
        XCTAssertEqual(model.snapshot?.value, snapshot)
        XCTAssertTrue(model.snapshotIsCached)
        XCTAssertFalse(model.canActOnSnapshot)
        model.drafts[route] = "离线草稿"
        await model.send(route)
        XCTAssertEqual(model.drafts[route], "离线草稿")
        await model.clearCachedData()
        let relaunched = offline()
        await relaunched.refreshSelection()
        XCTAssertNil(relaunched.snapshot)
    }

    func testDemoAssistantRejectsOtherBindingsAndRestartsFromBundledContent() throws {
        let host = try AssistantHost()
        let route = try ConversationRoute.assistant(from: host.demo.call("native_assistant_open", project: nil, args: [:]))
        XCTAssertThrowsError(try host.demo.call("native_assistant_open", project: "other", args: [:]))
        XCTAssertThrowsError(try host.demo.call("native_conversation_snapshot", project: host.demo.projectID, args: route.args))
        XCTAssertThrowsError(try host.demo.call("native_conversation_snapshot", project: route.project, args: ["session_id": .string("demo-qc")]))
        XCTAssertThrowsError(try host.demo.call("native_conversation_panel_file_read", project: route.project, args: route.args))
        _ = try host.demo.call("native_conversation_send", project: route.project, args: route.args.merging(["message": .string("hello")]) { _, v in v })
        let restored = try AssistantHost().demo.call("native_conversation_snapshot", project: route.project, args: route.args)
        XCTAssertEqual(restored["items"].array.count, 1)
    }
}

@MainActor private final class AssistantHost {
    struct Request { let command: String; let project: String?; let args: [String: RemoteValue] }
    let demo: DemoDataset
    var commands = DemoDataset.commands
    var requests: [Request] = []
    var opens: [Request] { requests.filter { $0.command == "native_assistant_open" } }
    var openResult: RemoteValue?
    var onOpen: (() -> Void)?
    init() throws { demo = try DemoDataset(bundle: Bundle(for: WispAppDelegate.self)) }
    func call(_ command: String, project: String?, args: [String: RemoteValue]) throws -> RemoteValue {
        requests.append(Request(command: command, project: project, args: args))
        if command == "native_assistant_open" {
            onOpen?()
            if let openResult { return openResult }
        }
        return try demo.call(command, project: project, args: args)
    }
}

private struct AssistantCredentials: RemoteCredentialStoring {
    func load() throws -> RemoteConfiguration? {
        try RemoteConfiguration(address: "https://assistant.example.test", code: "000102030405060708090a0b0c0d0e0f")
    }
    func save(_ configuration: RemoteConfiguration) throws { }
    func remove() throws { }
}
