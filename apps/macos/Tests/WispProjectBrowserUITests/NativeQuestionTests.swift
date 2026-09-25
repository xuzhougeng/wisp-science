import Foundation
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private let questionText = #"{"question":"Which reference?","options":[{"label":"v1","description":"Keep controls"},{"label":"v2"}],"allow_freeform":true}"#
private actor QuestionFake: NativeConversationQuerying {
    var sequence: UInt64 = 1
    var items: [[String: SettingsValue]] = [["role": .string("question"), "text": .string(questionText)]]
    func setItems(_ values: [[String: SettingsValue]]) { items = values; sequence += 1 }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot {
        var value: SettingsValue = .object([
            "schema": .string(ConversationSnapshot.schemaID), "epoch": .string("fixture"), "sequence": .integer(Int64(sequence)),
            "project_id": .string(projectID), "session_id": .string(sessionID), "items": .array(items.map(SettingsValue.object)),
            "running": .bool(false), "stopping": .bool(false), "read_only": .bool(false), "model_id": .string("offline"), "approvals": .array([])
        ])
        value["next_before_seq"] = .null
        return try ConversationSnapshot.decode(value, projectID: projectID, sessionID: sessionID)
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue { .null }
}
final class NativeQuestionTests: XCTestCase {
    @MainActor func testChangingOptionsPreservesOriginalDraftAndOptionDescription() async throws {
        let client = QuestionFake(); let model = NativeConversationModel(client: client)
        await model.open(project: "p", session: "s"); defer { model.pause() }
        let target = model.questionTarget(model.visibleItems[0], index: 0)
        let card = try XCTUnwrap(NativeQuestion(questionText))
        model.draft = "My original notes"
        XCTAssertTrue(model.stageQuestionAnswer(card.options[0].answer, target: target))
        XCTAssertEqual(model.draft, "My original notes\n\nv1\n\n选项说明：Keep controls")
        XCTAssertEqual(model.questionState(target), .pending)
        XCTAssertTrue(model.stageQuestionAnswer(card.options[1].answer, target: target))
        XCTAssertEqual(model.draft, "My original notes\n\nv2")
        model.draft += " + edited"
        XCTAssertTrue(model.stageQuestionAnswer("freeform answer", target: target))
        XCTAssertEqual(model.draft, "My original notes\n\nv2 + edited\n\nfreeform answer")
    }
    @MainActor func testLateCardCannotTouchAnotherSessionOrANewerQuestion() async {
        let client = QuestionFake(); let model = NativeConversationModel(client: client)
        await model.open(project: "p", session: "a"); defer { model.pause() }
        let old = model.questionTarget(model.visibleItems[0], index: 0)
        await model.open(project: "p", session: "b"); model.draft = "session b notes"
        XCTAssertFalse(model.stageQuestionAnswer("late answer", target: old))
        XCTAssertEqual(model.draft, "session b notes")
        await model.open(project: "p", session: "a"); model.draft = "back to a"
        XCTAssertFalse(model.stageQuestionAnswer("late answer", target: old))
        let current = model.questionTarget(model.visibleItems[0], index: 0)
        await client.setItems([["role": .string("question"), "text": .string(#"{"question":"Different question"}"#)]])
        await model.refresh()
        XCTAssertEqual(model.questionState(current), .expired)
        XCTAssertFalse(model.stageQuestionAnswer("old answer", target: current))
        XCTAssertEqual(model.draft, "back to a")
    }
    @MainActor func testAuthoritativeFollowUpSettlesCardAndExpiredACPStaysExpired() async {
        let client = QuestionFake(); let model = NativeConversationModel(client: client)
        await model.open(project: "p", session: "s"); defer { model.pause() }
        let target = model.questionTarget(model.visibleItems[0], index: 0)
        await client.setItems([["role": .string("question"), "text": .string(questionText)], ["role": .string("user"), "text": .string("answer")]])
        await model.refresh()
        XCTAssertEqual(model.questionState(target), .answered)
        XCTAssertFalse(model.stageQuestionAnswer("again", target: target))
        await client.setItems([["role": .string("question"), "text": .string(#"{"question":"ACP?","request_id":"request-1","status":"expired"}"#)], ["role": .string("user"), "text": .string("later")]])
        await model.refresh()
        let expired = model.questionTarget(model.visibleItems[0], index: 0)
        XCTAssertEqual(model.questionState(expired), .expired)
        XCTAssertFalse(model.canStageQuestion(expired))
    }
    @MainActor func testDraftAndQuestionAssociationSurviveSwitchingSessions() async {
        let client = QuestionFake(); let model = NativeConversationModel(client: client)
        await model.open(project: "p", session: "a"); defer { model.pause() }
        model.draft = "notes"
        _ = model.stageQuestionAnswer("first", target: model.questionTarget(model.visibleItems[0], index: 0))
        await model.open(project: "p", session: "b")
        await model.open(project: "p", session: "a")
        _ = model.stageQuestionAnswer("second", target: model.questionTarget(model.visibleItems[0], index: 0))
        XCTAssertEqual(model.draft, "notes\n\nsecond")
    }
    func testFreeformFallbackAndACPIdentity() throws {
        XCTAssertTrue(try XCTUnwrap(NativeQuestion(#"{"question":"Q","options":[],"allow_freeform":false}"#)).allowFreeform)
        let acp = try XCTUnwrap(NativeQuestion(#"{"request_id":"live-1","status":"pending"}"#))
        XCTAssertEqual(acp.requestID, "live-1"); XCTAssertEqual(acp.state, .pending)
        XCTAssertNil(NativeQuestion("invalid"))
    }
}
