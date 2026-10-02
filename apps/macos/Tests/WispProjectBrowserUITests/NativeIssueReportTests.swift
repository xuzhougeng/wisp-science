import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

final class NativeIssueReportTests: XCTestCase {
    func testPromptMatchesTheWebViewDraftAndOmitsTheWorkspacePath() {
        let prompt = IssueReportDraft.prompt(version: "0.34.0", os: "macos", arch: "arm64", model: "deepseek-chat", startup: "total=120ms store=90ms window_ready=600000ms")
        XCTAssertTrue(prompt.contains("0.34.0"))
        XCTAssertTrue(prompt.contains("macos / arm64"))
        XCTAssertTrue(prompt.contains("deepseek-chat"))
        XCTAssertTrue(prompt.contains("total=120ms store=90ms window_ready=600000ms"))
        XCTAssertTrue(prompt.contains(IssueReportDraft.repository))
        XCTAssertTrue(prompt.contains(IssueReportDraft.issueBase))
        XCTAssertFalse(prompt.contains("/mock/root"))
        XCTAssertTrue(IssueReportDraft.prompt(version: "1", os: "macos", arch: "arm64", model: "m", startup: "  ").contains("未记录"))
        XCTAssertFalse(IssueReportDraft.offered(activeSessionID: nil))
        XCTAssertFalse(IssueReportDraft.offered(activeSessionID: ""))
        XCTAssertTrue(IssueReportDraft.offered(activeSessionID: "session-a"))
        XCTAssertEqual(IssueReportDraft.modelLabel(models: [], modelID: "acp:Codex"), "Codex")
        XCTAssertEqual(IssueReportDraft.modelLabel(models: [.object(["id": .string("m1"), "label": .string("DeepSeek"), "active": .bool(true)])], modelID: nil), "DeepSeek")
    }

    @MainActor func testPreparePrefillsTheOpenComposerAndDoesNotSendOrRetry() async throws {
        let settings = FeedbackTransport()
        let conversationClient = FeedbackConversation()
        let conversation = NativeConversationModel(client: conversationClient)
        await conversationClient.configure([try feedbackSnapshot()])
        await conversation.open(project: "research-1", session: "session-a")
        conversation.pause()
        let list = FeedbackProjectList()
        let model = ProjectBrowserModel(client: list, databaseURL: URL(fileURLWithPath: "/unused/feedback.sqlite"), projectTransport: settings)
        await model.openProject("research-1", sessionID: "session-a")
        await model.issueReport.prepare(model: model, conversation: conversation, client: settings)
        let calls = await settings.calls()
        XCTAssertEqual(calls.count, 1)
        XCTAssertEqual(calls[0].command, "get_bootstrap_status")
        XCTAssertEqual(calls[0].projectID, "research-1")
        XCTAssertTrue(conversation.draft.contains("0.34.0"))
        XCTAssertTrue(conversation.draft.contains("total=120ms"))
        XCTAssertFalse(conversation.draft.contains("/mock/root"))
        let commands = await conversationClient.commands()
        XCTAssertFalse(commands.contains("native_conversation_send"))
        await settings.setMode("lost")
        let before = conversation.draft
        await model.issueReport.prepare(model: model, conversation: conversation, client: settings)
        let after = await settings.callCount()
        XCTAssertEqual(after, 2)
        XCTAssertEqual(conversation.draft, before)
        XCTAssertTrue(model.issueReport.error?.contains("不会自动重试") == true)
        await Task.yield()
        let still = await settings.callCount()
        XCTAssertEqual(still, 2)
    }

    @MainActor func testNoSessionDoesNotReadAndALateReplyDoesNotPrefill() async throws {
        let settings = FeedbackTransport()
        let conversation = NativeConversationModel(client: FeedbackConversation())
        let list = FeedbackProjectList()
        let model = ProjectBrowserModel(client: list, databaseURL: URL(fileURLWithPath: "/unused/feedback.sqlite"), projectTransport: settings)
        await model.issueReport.prepare(model: model, conversation: conversation, client: settings)
        let idle = await settings.callCount()
        XCTAssertEqual(idle, 0)
        XCTAssertEqual(conversation.draft, "")
        let openedClient = FeedbackConversation()
        let opened = NativeConversationModel(client: openedClient)
        await openedClient.configure([try feedbackSnapshot()])
        await opened.open(project: "research-1", session: "session-a")
        opened.pause()
        opened.draft = ""
        await model.openProject("research-1", sessionID: "session-a")
        await settings.setMode("late")
        let task = Task { await model.issueReport.prepare(model: model, conversation: opened, client: settings) }
        var started = false
        for _ in 0..<200 {
            if await settings.callCount() > 0 { started = true; break }
            await Task.yield()
        }
        XCTAssertTrue(started)
        model.goHome()
        await settings.release()
        await task.value
        XCTAssertNil(model.activeProjectID)
        XCTAssertEqual(opened.draft, "")
    }
}

private actor FeedbackTransport: NativeSettingsQuerying {
    private var recorded: [(command: String, args: [String: SettingsValue], projectID: String?)] = []
    private var mode = "ok"
    private var gate: CheckedContinuation<Void, Never>?
    func setMode(_ mode: String) { self.mode = mode }
    func callCount() -> Int { recorded.count }
    func calls() -> [(command: String, args: [String: SettingsValue], projectID: String?)] { recorded }
    func release() { gate?.resume(); gate = nil }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        recorded.append((command, args, projectID))
        if mode == "lost" { throw ProjectBrowserError.service("connection reset") }
        if mode == "late" {
            await withCheckedContinuation { (cont: CheckedContinuation<Void, Never>) in gate = cont }
        }
        return .object([
            "app_version": .string("0.34.0"),
            "os": .string("macos"),
            "arch": .string("arm64"),
            "startup": .string("total=120ms"),
            "workspace": .string("/mock/root"),
        ])
    }
}

private actor FeedbackProjectList: ProjectBrowserQuerying {
    func listProjects(databaseURL: URL) async throws -> ProjectListSnapshot {
        ProjectListSnapshot(projects: [], activitySource: "persisted_only")
    }
    func listSessions(databaseURL: URL, projectID: String?) async throws -> [BrowserSession] {
        [BrowserSession(id: "session-a", projectID: projectID ?? "", title: "探索", ts: 1, status: "complete")]
    }
    func transcript(databaseURL: URL, projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> TranscriptPage {
        TranscriptPage(messages: [], nextBeforeSeq: nil)
    }
}

private actor FeedbackConversation: NativeConversationQuerying {
    var reads: [ConversationSnapshot] = []
    var writes: [String] = []
    func configure(_ values: [ConversationSnapshot]) { reads = values }
    func commands() -> [String] { writes }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot {
        if let first = reads.first { return first }
        return try feedbackSnapshot()
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        writes.append(command)
        if command == "list_models" { return .array([]) }
        return .null
    }
}

private func feedbackSnapshot() throws -> ConversationSnapshot {
    var url = URL(fileURLWithPath: #filePath)
    for _ in 0..<5 { url.deleteLastPathComponent() }
    var value = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: url.appendingPathComponent("contracts/native-conversations/v1/snapshot.json")))
    value["project_id"] = .string("research-1")
    value["session_id"] = .string("session-a")
    value["sequence"] = .integer(7)
    value["epoch"] = .string("host-one")
    value["running"] = .bool(false)
    value["request_id"] = .null
    value["error"] = .null
    value["approvals"] = .array([])
    return try ConversationSnapshot.decode(value, projectID: "research-1", sessionID: "session-a")
}
