import Foundation
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

final class NativeTranscriptRowsTests: XCTestCase {
    private func item(_ json: String) throws -> ConversationItem {
        try JSONDecoder().decode(ConversationItem.self, from: Data(json.utf8))
    }

    func testRowsKeepTheirKeysWhenEarlierRowsDisappear() throws {
        let items = try [
            #"{"role":"user","text":"q1","seq":1}"#,
            #"{"role":"reasoning","text":"r","seq":2}"#,
            #"{"role":"assistant","text":"a1","seq":2}"#,
            #"{"role":"user","text":"q2","seq":3}"#,
        ].map(item)
        let rows = NativeTranscriptRows.rows(items)
        XCTAssertEqual(rows.map(\.id), ["seq:1/0", "seq:2/0", "seq:2/1", "seq:3/0"])
        XCTAssertEqual(rows.map(\.index), [0, 1, 2, 3])
        // Offsets shift when an older page replaces the latest one; keys must not.
        let shifted = NativeTranscriptRows.rows(Array(items.dropFirst()))
        XCTAssertEqual(shifted.map(\.id), Array(rows.map(\.id).dropFirst()))
        XCTAssertEqual(shifted.map(\.index), [0, 1, 2])
    }

    func testUnboundRowsFallBackToRequestEpochThenOffset() throws {
        let items = try [
            #"{"role":"user","text":"q","seq":4}"#,
            #"{"role":"question","text":"{\"request_id\":\"req-1\",\"question\":\"Which?\",\"options\":[]}"}"#,
            #"{"role":"compaction","text":"{\"before\":10,\"after\":5,\"strategy\":\"manual\",\"epoch\":2}"}"#,
            #"{"role":"assistant","text":"streaming"}"#,
        ].map(item)
        XCTAssertEqual(NativeTranscriptRows.rows(items).map(\.id), ["seq:4/0", "question:req-1", "compaction:2", "row:3"])
    }

    func testDuplicateIdentitiesStayUnique() throws {
        let question = #"{"role":"question","text":"{\"request_id\":\"req-1\",\"question\":\"A\",\"options\":[]}"}"#
        let keys = try NativeTranscriptRows.rows([question, question].map(item)).map(\.id)
        XCTAssertEqual(keys[0], "question:req-1")
        XCTAssertEqual(Set(keys).count, 2)
    }

    func testSharedFixtureCarriesPersistedSeqAndOlderHostsOmitIt() throws {
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { url.deleteLastPathComponent() }
        let value = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: url.appendingPathComponent("contracts/native-conversations/v1/snapshot.json")))
        let snapshot = try ConversationSnapshot.decode(value, projectID: "project-a", sessionID: "session-a")
        XCTAssertEqual(snapshot.items.map(\.seq), [1, 2])
        XCTAssertEqual(NativeTranscriptRows.rows(snapshot.items).map(\.id), ["seq:1/0", "seq:2/0"])
        XCTAssertNil(try item(#"{"role":"user","text":"older host"}"#).seq)
    }
}
