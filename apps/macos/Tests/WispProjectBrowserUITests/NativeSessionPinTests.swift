import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor PinClient: NativeConversationQuerying {
    var calls: [(String, [String: SettingsValue], String)] = []
    var fails = false
    var suspend = false
    var continuation: CheckedContinuation<SettingsValue, Error>?
    func configure(fails: Bool = false, suspend: Bool = false) { self.fails = fails; self.suspend = suspend }
    func writes() -> [(String, [String: SettingsValue], String)] { calls }
    func waiting() -> Bool { continuation != nil }
    func finish() { continuation?.resume(returning: .null); continuation = nil }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { throw ProjectBrowserError.invalidResponse }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        calls.append((command, args, projectID))
        if fails { throw ProjectBrowserError.service("response lost") }
        if suspend { return try await withCheckedThrowingContinuation { continuation = $0 } }
        return .null
    }
}

final class NativeSessionPinTests: XCTestCase {
    private func session(_ pinned: Bool?) -> BrowserSession {
        BrowserSession(id: "s", projectID: "p", title: "Work", ts: 1, status: "complete", pinned: pinned)
    }
    func testOldSnapshotsLeavePinStateUnknown() throws {
        let old = try JSONDecoder().decode(BrowserSession.self, from: Data(#"{"id":"s","project_id":"p","title":"Work","ts":1,"status":"complete"}"#.utf8))
        XCTAssertNil(old.pinned)
        let new = try JSONDecoder().decode(BrowserSession.self, from: Data(#"{"id":"s","project_id":"p","title":"Work","ts":1,"status":"complete","pinned":true}"#.utf8))
        XCTAssertEqual(new.pinned, true)
    }
    @MainActor func testPinAndUnpinUseConfirmedStateAndExactScope() async {
        let client = PinClient(); let model = NativeSessionPin()
        let pin = await model.toggle(session(false), client: client)
        let unpin = await model.toggle(session(true), client: client)
        XCTAssertTrue(pin); XCTAssertTrue(unpin)
        let writes = await client.writes()
        XCTAssertEqual(writes.map(\.0), ["native_conversation_pin", "native_conversation_pin"])
        XCTAssertEqual(writes.map(\.2), ["p", "p"])
        XCTAssertEqual(writes[0].1, ["session_id": .string("s"), "pinned": .bool(true)])
        XCTAssertEqual(writes[1].1, ["session_id": .string("s"), "pinned": .bool(false)])
    }
    @MainActor func testUnknownStateCannotWriteAndLostReplyNeverRetries() async {
        let client = PinClient(); let model = NativeSessionPin()
        let unknown = await model.toggle(session(nil), client: client); XCTAssertFalse(unknown)
        var writes = await client.writes(); XCTAssertTrue(writes.isEmpty)
        await client.configure(fails: true)
        let result = await model.toggle(session(false), client: client)
        XCTAssertFalse(result); XCTAssertNotNil(model.error); XCTAssertFalse(model.busy)
        model.reset(); writes = await client.writes(); XCTAssertEqual(writes.count, 1)
    }
    @MainActor func testDuplicateAndOldReplyCannotChangeNewWorkspace() async {
        let client = PinClient(); await client.configure(suspend: true)
        let model = NativeSessionPin()
        let saving = Task { await model.toggle(session(false), client: client) }
        while !(await client.waiting()) { await Task.yield() }
        let duplicate = await model.toggle(session(false), client: client); XCTAssertFalse(duplicate)
        model.reset(); await client.finish(); let stale = await saving.value
        XCTAssertFalse(stale); XCTAssertNil(model.error); XCTAssertFalse(model.busy)
        let writes = await client.writes(); XCTAssertEqual(writes.count, 1)
    }
    func testPinnedSectionStaysFirstWithoutDuplicatingFoldersOrDates() {
        let sessions = [
            BrowserSession(id: "p-old", projectID: "p", title: "Alpha", ts: 1, status: "complete", folderID: "f", pinned: true),
            BrowserSession(id: "p-new", projectID: "p", title: "Zulu", ts: 5, status: "complete", pinned: true),
            BrowserSession(id: "normal", projectID: "p", title: "Beta", ts: 10, status: "complete", folderID: "f", pinned: false)
        ]
        for grouping in ["none", "folder", "date"] {
            for order in ["newest", "name"] {
                let sections = SessionArrangement.sections(sessions, folders: [ProjectFolder(id: "f", name: "Week")], sort: order, group: grouping)
                XCTAssertEqual(sections.first?.title, "已置顶")
                XCTAssertEqual(sections.first?.sessions.map(\.id), order == "name" ? ["p-old", "p-new"] : ["p-new", "p-old"])
                let ids = sections.flatMap(\.sessions).map(\.id)
                XCTAssertEqual(ids.count, 3); XCTAssertEqual(Set(ids).count, 3)
                XCTAssertEqual(sections.dropFirst().flatMap(\.sessions).map(\.id), ["normal"])
            }
        }
        let onlyPinned = SessionArrangement.sections(Array(sessions.prefix(2)), folders: [], sort: "newest", group: "none")
        XCTAssertEqual(onlyPinned.count, 1)
        let unpinned = sessions.map { BrowserSession(id: $0.id, projectID: $0.projectID, title: $0.title, ts: $0.ts, status: $0.status, folderID: $0.folderID, pinned: false) }
        let restored = SessionArrangement.sections(unpinned, folders: [], sort: "newest", group: "none")
        XCTAssertEqual(restored.count, 1); XCTAssertEqual(restored[0].sessions.map(\.id), ["normal", "p-new", "p-old"])
    }
}
