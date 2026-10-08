import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor TimerHost: NativeConversationQuerying {
    var record: SettingsValue = .null
    var fail = false
    var count = 0
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { throw ProjectBrowserError.invalidResponse }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        XCTAssertEqual(command, "native_conversation_timer"); XCTAssertEqual(projectID, "p"); XCTAssertEqual(args["session_id"], .string("s"))
        if args["operation"]?["action"].string == "read" { return record }
        count += 1
        if fail { throw ProjectBrowserError.service("lost reply") }
        record = .object(["project_id": .string("p"), "frame_id": .string("s"), "replace_previous_turn": .bool(true), "prompt": .string("Check records"), "interval_secs": .integer(3600), "enabled": .bool(true)])
        return record
    }
    func failWrites() { fail = true }
    func writes() -> Int { count }
    func foreign() { record = .object(["project_id": .string("other"), "frame_id": .string("s"), "replace_previous_turn": .bool(true)]) }
}
final class NativeSessionTimerTests: XCTestCase {
    @MainActor func testLostTimerWriteCannotReplayAcrossSheetReopening() async {
        let host = TimerHost(); let model = NativeSessionTimerModel(client: host, project: "p", session: "s")
        await model.read(); XCTAssertTrue(model.canWrite)
        model.prompt = "Check records"; await host.failWrites()
        let operation: [String: SettingsValue] = ["action": .string("set"), "expression": .string("60m Check records")]
        await model.write(operation); XCTAssertTrue(model.uncertain); XCTAssertFalse(model.canWrite)
        model.acknowledge(); XCTAssertTrue(model.uncertain)
        await model.write(operation); let count = await host.writes(); XCTAssertEqual(count, 1)
        await model.read(); XCTAssertEqual(model.prompt, "Check records"); XCTAssertTrue(model.uncertain)
        model.acknowledge(); XCTAssertTrue(model.canWrite)
    }
    @MainActor func testForeignTimerReplyNeverEnablesMutation() async {
        let host = TimerHost(); let model = NativeSessionTimerModel(client: host, project: "p", session: "s")
        await host.foreign(); await model.read()
        XCTAssertEqual(model.timer, .null); XCTAssertFalse(model.canWrite); XCTAssertNotNil(model.error)
    }
}
