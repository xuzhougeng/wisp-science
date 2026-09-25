import Foundation
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private func fixture(_ session: String = "session-a", sequence: UInt64 = 7, epoch: String = "host-one", running: Bool = false, requestID: String? = nil, error: String? = nil, approvalID: String? = nil) throws -> ConversationSnapshot {
    var url = URL(fileURLWithPath: #filePath)
    for _ in 0..<5 { url.deleteLastPathComponent() }
    var value = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: url.appendingPathComponent("contracts/native-conversations/v1/snapshot.json")))
    value["session_id"] = .string(session); value["sequence"] = .integer(Int64(sequence)); value["epoch"] = .string(epoch)
    value["running"] = .bool(running); value["request_id"] = requestID.map(SettingsValue.string) ?? .null
    value["error"] = error.map(SettingsValue.string) ?? .null
    value["approvals"] = .array(approvalID.map { id in [.object(["approval_id": .string(id), "frame_id": .string(session), "message": .string("Run?"), "tool": .string("shell"), "preview": .string("echo test")])] } ?? [])
    return try ConversationSnapshot.decode(value, projectID: "project-a", sessionID: session)
}
private actor ConversationFake: NativeConversationQuerying {
    var reads: [ConversationSnapshot] = []
    var writes: [(String, [String: SettingsValue], String)] = []
    var failSend = false
    var failRead = false
    var held: CheckedContinuation<ConversationSnapshot, Error>?
    var holdRead = false
    var heldPreferences: CheckedContinuation<SettingsValue, Never>?
    var holdPreferences = false
    func configure(_ values: [ConversationSnapshot], failSend: Bool = false, failRead: Bool = false) { reads = values; self.failSend = failSend; self.failRead = failRead }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot {
        if holdRead { holdRead = false; return try await withCheckedThrowingContinuation { held = $0 } }
        if failRead { throw ProjectBrowserError.service("offline") }
        return reads.count > 1 ? reads.removeFirst() : try reads.first ?? fixture(sessionID)
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        if command == "get_appearance_prefs" {
            if holdPreferences { holdPreferences = false; return await withCheckedContinuation { heldPreferences = $0 } }
            return .null
        }
        if command == "list_models" { return .array([]) }
        writes.append((command, args, projectID))
        if failSend { throw ProjectBrowserError.service("response lost") }
        return .null
    }
    // Existing no-replay assertions count conversation mutations; opening a
    // successfully read session now also records a separate seen acknowledgement.
    func count() -> Int { writes.filter { $0.0 != "native_conversation_seen" }.count }
    func seenCount() -> Int { writes.filter { $0.0 == "native_conversation_seen" }.count }
    func lastArgs() -> [String: SettingsValue] { writes.last?.1 ?? [:] }
    func hold() { holdRead = true }
    func holdInputPreferences() { holdPreferences = true }
    func isHoldingPreferences() -> Bool { heldPreferences != nil }
    func finishPreferences() { heldPreferences?.resume(returning: .null); heldPreferences = nil }
    func isHeld() -> Bool { held != nil }
    func finish(_ value: ConversationSnapshot) { held?.resume(returning: value); held = nil }
}
final class NativeConversationModelTests: XCTestCase {
    @MainActor func testComposerWaitsForSavedInputPolicyBeforeAcceptingMessages() async {
        let client = ConversationFake(); let model = NativeConversationModel(client: client)
        await client.holdInputPreferences()
        let opening = Task { await model.open(project: "project-a", session: "session-a") }
        while !(await client.isHoldingPreferences()) { await Task.yield() }
        model.draft = "not yet ready"
        XCTAssertFalse(model.canSend)
        await model.send()
        let before = await client.count(); XCTAssertEqual(before, 0)
        await client.finishPreferences(); await opening.value
        XCTAssertTrue(model.canSend)
        model.pause()
    }
    @MainActor func testSavedExcerptSelectsRenderedMessageAndDoesNotClearNewerHighlight() async throws {
        let client = ConversationFake(); let model = NativeConversationModel(client: client)
        await client.configure([try fixture()]); await model.open(project: "project-a", session: "session-a")
        model.revealExcerpt("检查 样本")
        XCTAssertEqual(model.scrollTarget, 0)
        let first = model.scrollRevision
        model.revealExcerpt("正在 检查样本…")
        XCTAssertEqual(model.scrollTarget, 1)
        model.clearExcerpt(revision: first)
        XCTAssertNotNil(model.revealedExcerpt)
        model.clearExcerpt(revision: model.scrollRevision)
        XCTAssertNil(model.revealedExcerpt)
        model.revealExcerpt("missing")
        XCTAssertNotNil(model.operationError)
        XCTAssertEqual(model.scrollTarget, 1)
        model.pause()
    }
    @MainActor func testOnlySuccessfulOpenMarksSessionSeen() async throws {
        let client = ConversationFake(); let model = NativeConversationModel(client: client)
        await client.configure([try fixture()], failRead: true)
        await model.open(project: "project-a", session: "session-a")
        let first = await client.seenCount(); XCTAssertEqual(first, 0)
        await client.configure([try fixture()])
        await model.open(project: "project-a", session: "session-a")
        let second = await client.seenCount(); XCTAssertEqual(second, 1)
        model.pause()
    }
    @MainActor func testSnapshotOrderingAndRestartNeverAppendOrRestoreRetiredHost() async throws {
        let client = ConversationFake(); let model = NativeConversationModel(client: client)
        await client.configure([try fixture(sequence: 7), try fixture(sequence: 6), try fixture(sequence: 1, epoch: "host-two"), try fixture(sequence: 99)])
        await model.open(project: "project-a", session: "session-a")
        await model.refresh(); XCTAssertEqual(model.snapshot?.sequence, 7)
        await model.refresh(); XCTAssertEqual(model.snapshot?.epoch, "host-two")
        await model.refresh(); XCTAssertEqual(model.snapshot?.epoch, "host-two")
        XCTAssertEqual(model.visibleItems.count, 2); model.pause()
    }
    @MainActor func testAmbiguousSendKeepsDraftAndDoesNotReplayThenAcknowledgesFromSnapshot() async throws {
        let client = ConversationFake(); let model = NativeConversationModel(client: client)
        await client.configure([try fixture()], failSend: true)
        await model.open(project: "project-a", session: "session-a")
        model.draft = "do work"; await model.send()
        XCTAssertTrue(model.uncertainSend); XCTAssertEqual(model.draft, "do work"); XCTAssertFalse(model.canSend)
        await model.send(); let count = await client.count(); XCTAssertEqual(count, 1)
        let args = await client.lastArgs()
        await client.configure([try fixture(sequence: 9, running: true, requestID: args["request_id"]?.string)])
        await model.refresh()
        XCTAssertEqual(model.draft, ""); XCTAssertFalse(model.uncertainSend); XCTAssertTrue(model.snapshot?.running == true)
        model.pause()
    }
    @MainActor func testNavigationIgnoresLateReadAndPreservesDraftWithoutStoppingAgent() async throws {
        let client = ConversationFake(); let model = NativeConversationModel(client: client)
        await model.open(project: "project-a", session: "session-a"); model.draft = "unsent"
        await client.hold(); let old = Task { await model.refresh() }
        while !(await client.isHeld()) { await Task.yield() }
        await model.open(project: "project-a", session: "session-b")
        await client.finish(try fixture(sequence: 999)); await old.value
        XCTAssertEqual(model.snapshot?.session_id, "session-b")
        await model.open(project: "project-a", session: "session-a")
        XCTAssertEqual(model.draft, "unsent")
        let count = await client.count(); XCTAssertEqual(count, 0); model.pause()
    }
    @MainActor func testReconnectDoesNotClearTranscriptOrAllowSendingWhileOffline() async throws {
        let client = ConversationFake(); let model = NativeConversationModel(client: client)
        await model.open(project: "project-a", session: "session-a"); model.draft = "hello"
        await client.configure([], failRead: true); await model.refresh()
        XCTAssertEqual(model.visibleItems.count, 2); XCTAssertFalse(model.canSend)
        await client.configure([try fixture(sequence: 8)]); await model.refresh()
        XCTAssertNil(model.connectionError); XCTAssertTrue(model.canSend); model.pause()
    }
    @MainActor func testAmbiguousAcceptedFailureRestoresDraftAfterNavigation() async throws {
        let client = ConversationFake(); let model = NativeConversationModel(client: client)
        await client.configure([try fixture()], failSend: true)
        await model.open(project: "project-a", session: "session-a")
        model.draft = "recover me"; await model.send()
        let args = await client.lastArgs()
        await client.configure([try fixture("session-b")])
        await model.open(project: "project-a", session: "session-b")
        await client.configure([try fixture(sequence: 10, requestID: args["request_id"]?.string, error: "model unavailable")])
        await model.open(project: "project-a", session: "session-a")
        XCTAssertFalse(model.uncertainSend); XCTAssertEqual(model.draft, "recover me")
        XCTAssertTrue(model.canSend)
        let count = await client.count(); XCTAssertEqual(count, 1); model.pause()
    }
    @MainActor func testStopAndApprovalCarrySessionAndExactApprovalIdentity() async throws {
        let client = ConversationFake(); let model = NativeConversationModel(client: client)
        await client.configure([try fixture(approvalID: "exact-id")])
        await model.open(project: "project-a", session: "session-a")
        await model.stop()
        var args = await client.lastArgs(); XCTAssertEqual(args["session_id"]?.string, "session-a")
        let approval = try JSONDecoder().decode(ConversationApproval.self, from: Data(#"{"approval_id":"exact-id","frame_id":"session-a","message":"Run?","tool":"shell","preview":"echo test"}"#.utf8))
        await model.approve(approval, allowed: false, feedback: "  Use the selected control sample.  ")
        args = await client.lastArgs()
        XCTAssertEqual(args["session_id"]?.string, "session-a")
        XCTAssertEqual(args["approval_id"]?.string, "exact-id")
        XCTAssertEqual(args["approved"], .bool(false))
        XCTAssertEqual(args["feedback"], .string("Use the selected control sample."))
        await client.configure([try fixture(sequence: 8, approvalID: "new-id")]); await model.refresh()
        let count = await client.count()
        let stale = await model.approve(approval, allowed: true)
        XCTAssertFalse(stale)
        let after = await client.count(); XCTAssertEqual(after, count)
        model.pause()
    }
    func testSnapshotRejectsWrongProjectOrApprovalScope() throws {
        let data = try JSONEncoder().encode(fixture())
        var value = try JSONDecoder().decode(SettingsValue.self, from: data)
        XCTAssertThrowsError(try ConversationSnapshot.decode(value, projectID: "other", sessionID: "session-a"))
        value["approvals"] = .array([.object(["approval_id": .string("a"), "frame_id": .string("other"), "message": .string("?"), "tool": .string("shell"), "preview": .string("")])])
        XCTAssertThrowsError(try ConversationSnapshot.decode(value, projectID: "project-a", sessionID: "session-a"))
    }

    @MainActor func testUnconfirmedSendKeepsRecoveryActionAfterLeavingAndReturning() async throws {
        let client = ConversationFake(); let model = NativeConversationModel(client: client)
        await client.configure([try fixture()], failSend: true)
        await model.open(project: "project-a", session: "session-a")
        model.draft = "uncertain"; await model.send()
        await client.configure([try fixture("session-b")])
        await model.open(project: "project-a", session: "session-b")
        await client.configure([try fixture(sequence: 8)])
        await model.open(project: "project-a", session: "session-a")
        XCTAssertTrue(model.uncertainSend); XCTAssertNotNil(model.operationError)
        XCTAssertEqual(model.draft, "uncertain"); XCTAssertFalse(model.canSend)
        model.acknowledgeUncertainSend()
        XCTAssertTrue(model.canSend)
        let count = await client.count(); XCTAssertEqual(count, 1); model.pause()
    }

}
