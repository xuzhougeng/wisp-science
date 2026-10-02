import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private func acpFixture() throws -> SettingsValue {
    let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    return try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-conversations/v1/acp-interactions.json")))
}
private actor AcpInteractionFake: NativeConversationQuerying {
    var value: SettingsValue
    var calls: [(String, [String: SettingsValue], String)] = []
    var fail = false
    var hold = false
    var pending: CheckedContinuation<Void, Never>?
    init() throws { value = try acpFixture() }
    func setFailure() { fail = true }
    func setHold() { hold = true }
    func release() { pending?.resume(); pending = nil }
    func count() -> Int { calls.count }
    func records() -> [(String, [String: SettingsValue], String)] { calls }
    func expire() { value["acp"] = .object(["permissions": .array([]), "question_ids": .array([])]) }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot {
        value["sequence"] = .integer(value["sequence"].integer + 1)
        var copy = value
        copy["project_id"] = .string(projectID); copy["session_id"] = .string(sessionID)
        if sessionID != "s" { copy["acp"] = .null }
        return try ConversationSnapshot.decode(copy, projectID: projectID, sessionID: sessionID)
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        guard command.hasPrefix("native_conversation_acp_") else { return .null }
        calls.append((command, args, projectID))
        if hold { await withCheckedContinuation { pending = $0 } }
        if fail { throw ProjectBrowserError.unavailable("lost response") }
        return .null
    }
}
final class NativeAcpInteractionTests: XCTestCase {
    @MainActor func testRenderAcpInteractionsAtNarrowSizes() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Set WISP_NATIVE_SNAPSHOT_DIR for offline renders") }
        try FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
        let fake = try AcpInteractionFake(); let model = NativeConversationModel(client: fake)
        await model.open(project: "p", session: "s"); defer { model.pause() }
        for (name, scheme) in [("light", ColorScheme.light), ("dark", ColorScheme.dark)] {
            let view = NSHostingView(rootView: NativeConversationView(conversation: model)
                .background(WispDesign.color("bg-app", scheme)).foregroundStyle(WispDesign.color("text", scheme))
                .tint(WispDesign.color("clay", scheme)).environment(\.colorScheme, scheme))
            view.frame = NSRect(x: 0, y: 0, width: 419, height: 650)
            view.layoutSubtreeIfNeeded()
            let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
            view.cacheDisplay(in: view.bounds, to: bitmap)
            let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
            XCTAssertGreaterThan(png.count, 1000)
            try png.write(to: URL(fileURLWithPath: directory).appendingPathComponent("acp-interactions-\(name).png"))
        }
    }
    @MainActor func testExplicitPermissionOptionUsesCurrentScopeAndIsNotReplayed() async throws {
        let fake = try AcpInteractionFake(); let model = NativeConversationModel(client: fake)
        await model.open(project: "p", session: "s"); defer { model.pause() }
        let permission = try XCTUnwrap(model.snapshot?.acp?.permissions.first)
        let invalid = await model.respondAcpPermission(permission, optionID: "invented")
        XCTAssertFalse(invalid)
        let success = await model.respondAcpPermission(permission, optionID: "allow-future")
        XCTAssertTrue(success)
        XCTAssertFalse(model.canRespondAcpPermission(permission))
        let duplicate = await model.respondAcpPermission(permission, optionID: "allow-single")
        XCTAssertFalse(duplicate)
        let records = await fake.records()
        XCTAssertEqual(records.count, 1)
        XCTAssertEqual(records[0].0, "native_conversation_acp_permission")
        XCTAssertEqual(records[0].1, ["session_id": .string("s"), "request_id": .string("permission-1"), "option_id": .string("allow-future")])
        XCTAssertEqual(records[0].2, "p")
    }
    @MainActor func testCancelSendsNullInsteadOfInventingARejectOption() async throws {
        let fake = try AcpInteractionFake(); let model = NativeConversationModel(client: fake)
        await model.open(project: "p", session: "s"); defer { model.pause() }
        let permission = try XCTUnwrap(model.snapshot?.acp?.permissions.first)
        let success = await model.respondAcpPermission(permission, optionID: nil)
        XCTAssertTrue(success)
        let records = await fake.records()
        XCTAssertEqual(records[0].1["option_id"], .null)
    }
    @MainActor func testQuestionReplyPreservesComposerAndCannotBeRepeated() async throws {
        let fake = try AcpInteractionFake(); let model = NativeConversationModel(client: fake)
        await model.open(project: "p", session: "s"); defer { model.pause() }
        model.draft = "Do not replace my notes"
        let target = model.questionTarget(model.visibleItems[0], index: 0)
        XCTAssertTrue(model.canAnswerAcpQuestion(target))
        let blank = await model.answerAcpQuestion("  ", target: target)
        XCTAssertFalse(blank)
        let success = await model.answerAcpQuestion(" reference v2 ", target: target)
        XCTAssertTrue(success)
        XCTAssertEqual(model.draft, "Do not replace my notes")
        XCTAssertFalse(model.canAnswerAcpQuestion(target))
        let calls = await fake.records()
        XCTAssertEqual(calls.count, 1)
        XCTAssertEqual(calls[0].1, ["session_id": .string("s"), "request_id": .string("ask-1"), "answer": .string("reference v2")])
    }
    @MainActor func testExpiredAndOtherSessionCardsCannotRespond() async throws {
        let fake = try AcpInteractionFake(); let model = NativeConversationModel(client: fake)
        await model.open(project: "p", session: "s"); defer { model.pause() }
        let target = model.questionTarget(model.visibleItems[0], index: 0)
        let permission = try XCTUnwrap(model.snapshot?.acp?.permissions.first)
        await fake.expire(); await model.refresh()
        XCTAssertFalse(model.canAnswerAcpQuestion(target)); XCTAssertFalse(model.canRespondAcpPermission(permission))
        await model.open(project: "other-project", session: "other")
        let answered = await model.answerAcpQuestion("late", target: target)
        let approved = await model.respondAcpPermission(permission, optionID: "allow-single")
        XCTAssertFalse(answered); XCTAssertFalse(approved)
        let count = await fake.count(); XCTAssertEqual(count, 0)
    }
    @MainActor func testLostReplyDoesNotReplayOrClearDraft() async throws {
        let fake = try AcpInteractionFake(); let model = NativeConversationModel(client: fake)
        await model.open(project: "p", session: "s"); defer { model.pause() }
        let target = model.questionTarget(model.visibleItems[0], index: 0)
        model.draft = "keep"
        await fake.setFailure()
        let answered = await model.answerAcpQuestion("answer", target: target)
        XCTAssertFalse(answered); XCTAssertFalse(model.canAnswerAcpQuestion(target))
        XCTAssertEqual(model.draft, "keep"); XCTAssertTrue(model.operationError?.contains("不会自动重试") == true)
        await model.refresh()
        let count = await fake.count(); XCTAssertEqual(count, 1)
    }
    @MainActor func testNavigationWhileReplyingCannotChangeNewConversation() async throws {
        let fake = try AcpInteractionFake(); let model = NativeConversationModel(client: fake)
        await model.open(project: "p", session: "s"); defer { model.pause() }
        let target = model.questionTarget(model.visibleItems[0], index: 0)
        await fake.setHold(); await fake.setFailure()
        let task = Task { await model.answerAcpQuestion("answer", target: target) }
        for _ in 0..<100 { if await fake.count() == 1 { break }; await Task.yield() }
        let count = await fake.count(); XCTAssertEqual(count, 1)
        await model.open(project: "other-project", session: "other"); model.draft = "new notes"
        await fake.release(); _ = await task.value
        XCTAssertEqual(model.draft, "new notes"); XCTAssertNil(model.operationError)
        XCTAssertEqual(model.snapshot?.session_id, "other")
    }
    func testDecoderRejectsPermissionFromAnotherConversation() throws {
        var value = try acpFixture(); value["session_id"] = .string("other")
        XCTAssertThrowsError(try ConversationSnapshot.decode(value, projectID: "p", sessionID: "other"))
    }
}
