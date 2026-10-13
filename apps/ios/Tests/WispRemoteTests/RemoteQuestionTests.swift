import XCTest
@testable import WispRemote

final class RemoteQuestionTests: XCTestCase {
    private func item(_ payload: String, role: String = "question") -> RemoteValue {
        .object(["role": .string(role), "text": .string(payload)])
    }
    private let payload = #"{"v":1,"source":"native","question":"采用什么标准？","options":[{"label":"建议标准","description":"逐样本检查"},{"label":"自定义"}],"allow_freeform":true,"note":"End your turn here"}"#

    func testNativePayloadBecomesQuestionAndChoiceWithOptionalNote() throws {
        let card = try XCTUnwrap(RemoteQuestion(item(payload)))
        XCTAssertEqual(card.question, "采用什么标准？")
        XCTAssertEqual(card.options.count, 2)
        XCTAssertTrue(card.isNative)
        XCTAssertEqual(card.state, .pending)
        XCTAssertEqual(card.answer(option: 0, freeform: "  重点检查样本 A\n"), "建议标准\n\n选项说明：逐样本检查\n\n补充说明：重点检查样本 A")
        XCTAssertEqual(card.answer(option: 1, freeform: ""), "自定义")
        XCTAssertEqual(card.answer(option: nil, freeform: "自己的标准"), "自己的标准")
        XCTAssertNil(card.answer(option: nil, freeform: " \n"))
        XCTAssertNil(card.answer(option: 5, freeform: "有效文本"))
    }

    func testLaterUserSettlesNativeButNotAnUnrelatedACPQuestion() throws {
        let native = try XCTUnwrap(RemoteQuestion(item(payload), hasLaterUser: true))
        XCTAssertEqual(native.state, .answered)
        XCTAssertNil(native.answer(option: 0, freeform: ""))
        let acp = try XCTUnwrap(RemoteQuestion(item(#"{"question":"ACP?","source":"acp","request_id":"request-a"}"#), hasLaterUser: true))
        XCTAssertFalse(acp.isNative)
        XCTAssertEqual(acp.state, .pending)
        XCTAssertNil(acp.answer(option: nil, freeform: "answer"))
        XCTAssertFalse(try XCTUnwrap(RemoteQuestion(item(#"{"question":"Unknown?","source":"future"}"#))).isNative)
    }

    func testExplicitExpiredAndAnsweredStatesCannotSubmit() throws {
        for state in ["expired", "answered"] {
            let card = try XCTUnwrap(RemoteQuestion(item("{\"question\":\"Which?\",\"status\":\"\(state)\"}"), hasLaterUser: true))
            XCTAssertEqual(card.state, state == "expired" ? .expired : .answered)
            XCTAssertNil(card.answer(option: nil, freeform: "answer"))
        }
    }

    func testChoiceOnlyAndEmptyOptionsFallbackMatchDesktop() throws {
        let fixed = try XCTUnwrap(RemoteQuestion(item(#"{"question":"Choose","allow_freeform":false,"options":[{"label":"A"}]}"#)))
        XCTAssertFalse(fixed.allowsFreeform)
        XCTAssertNil(fixed.answer(option: 0, freeform: "not permitted"))
        XCTAssertEqual(fixed.answer(option: 0, freeform: ""), "A")
        let empty = try XCTUnwrap(RemoteQuestion(item(#"{"question":"Explain","allow_freeform":false,"options":[null,{},"A",{"label":" "}]}"#)))
        XCTAssertTrue(empty.options.isEmpty)
        XCTAssertTrue(empty.allowsFreeform)
        XCTAssertEqual(empty.answer(option: nil, freeform: "My answer"), "My answer")
    }

    func testMalformedQuestionsDegradeToOriginalText() {
        for text in ["not JSON", "[]", "{}", #"{"question":3}"#, #"{"question":" "}"#] {
            XCTAssertNil(RemoteQuestion(item(text)))
        }
        XCTAssertNil(RemoteQuestion(item(payload, role: "assistant")))
    }
}
