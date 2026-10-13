import XCTest
import CryptoKit
@testable import WispRemote

let testCode = "0001-0203-0405-0607-0809-0a0b-0c0d-0e0f"

/// The host side deliberately derives and encrypts independently of the client.
enum TestHostCrypto {
    static let key = SymmetricKey(data: SHA256.hash(data: Data("wisp-remote/key/v1".utf8) + Data(0..<16)))
    static func seal(_ value: RemoteValue) throws -> String {
        let sealed = try AES.GCM.seal(JSONEncoder().encode(value), using: key, authenticating: Data("wisp-remote/v1/h2c".utf8))
        return try XCTUnwrap(sealed.combined).base64EncodedString()
    }
    static func open(_ frame: String) throws -> RemoteValue {
        let box = try AES.GCM.SealedBox(combined: XCTUnwrap(Data(base64Encoded: frame)))
        return try JSONDecoder().decode(RemoteValue.self, from: AES.GCM.open(box, using: key, authenticating: Data("wisp-remote/v1/c2h".utf8)))
    }
}

func fixture(_ name: String) throws -> RemoteValue {
    var root = URL(fileURLWithPath: #filePath)
    for _ in 0..<5 { root.deleteLastPathComponent() }
    return try JSONDecoder().decode(RemoteValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/" + name)))
}

final class RemoteProtocolTests: XCTestCase {
    func testResearchAssistantUsesSharedFixedBindingAndOpenIsNotAReadCacheCommand() throws {
        let value = try fixture("native-assistant/v1/workspace.json")
        let route = try ConversationRoute.assistant(from: value["open"])
        XCTAssertEqual(route, .researchAssistant)
        XCTAssertEqual(route.args, ["session_id": .string("research-assistant")])
        XCTAssertFalse(RemoteReadCache.accepts("native_assistant_open"))
        let invalid: [RemoteValue] = [.null, .object(["project_id": .string("other"), "session_id": .string(route.session)]),
                                     .object(["project_id": .string(route.project), "session_id": .string("other")])]
        for wrong in invalid {
            XCTAssertThrowsError(try ConversationRoute.assistant(from: wrong))
        }
    }

    func testConnectionLinkNeverSendsCodeAndPreservesProxyPrefix() throws {
        let configuration = try RemoteConfiguration(address: "https://relay.example.test/wisp/remote#\(testCode)/p/project/s/chat")
        XCTAssertEqual(configuration.relay.absoluteString, "https://relay.example.test/wisp/")
        XCTAssertEqual(configuration.crypto.sid, "cf4778d13d24d0dd1313bca1709267bb") // remote.rs / remote-web.spec.ts
        XCTAssertEqual(configuration.webSocketURL.absoluteString, "wss://relay.example.test/wisp/v1/remote/client/cf4778d13d24d0dd1313bca1709267bb")
        XCTAssertNil(configuration.webSocketURL.fragment)
        XCTAssertFalse(configuration.webSocketURL.absoluteString.contains(configuration.code))
        XCTAssertEqual(try RemoteConfiguration(address: "https://relay.example.test/wisp/", code: testCode), configuration)
    }

    func testRejectsInsecureCredentialBearingOrMalformedAddresses() {
        for address in ["http://relay.example.test", "http://127.0.0.1:8787", "wss://relay.example.test",
                        "https://user:password@relay.example.test", "https://relay.example.test?code=secret", "not a URL", "https://relay.example.test:0"] {
            XCTAssertThrowsError(try RemoteConfiguration(address: address, code: testCode), address)
        }
        for code in ["abcd", "zz" + testCode, "0001/0203/0405/0607/0809/0a0b/0c0d/0e0f", ""] {
            XCTAssertThrowsError(try RemoteConfiguration(address: "https://relay.example.test", code: code))
        }
    }

    func testFramesAreCompatibleDirectionBoundAndAuthenticated() throws {
        let crypto = try RemoteConfiguration(address: "https://relay.example.test", code: testCode).crypto
        let value: RemoteValue = .object(["type": .string("hello"), "text": .string("研究 🧬")])
        let frame = try TestHostCrypto.seal(value)
        XCTAssertEqual(try crypto.open(frame), value)
        XCTAssertEqual(try TestHostCrypto.open(crypto.seal(value)), value)
        XCTAssertThrowsError(try crypto.open(crypto.seal(value)), "reflected client frame")
        XCTAssertEqual(try TestHostCrypto.open("AAECAwQFBgcICQoL9OOvsOOON1awkl3qncEpESEISdpba7HB8iw="), .object(["ping": .integer(1)]))
        var bytes = try XCTUnwrap(Data(base64Encoded: frame))
        bytes[bytes.count - 1] ^= 1
        XCTAssertThrowsError(try crypto.open(bytes.base64EncodedString()))
        let other = try RemoteConfiguration(address: "https://relay.example.test", code: String(repeating: "f", count: 32)).crypto
        XCTAssertThrowsError(try other.open(frame))
        XCTAssertThrowsError(try crypto.open("invalid"))
        XCTAssertThrowsError(try crypto.seal(.string(String(repeating: "x", count: 800_000))))
    }

    func testSharedSnapshotContractKeepsQueueIDsExact() throws {
        let route = ConversationRoute(project: "project-a", session: "session-a")
        let value = try fixture("native-conversations/v1/snapshot-images-queue.json")
        let snapshot = try RemoteSnapshot(value, route: route)
        XCTAssertEqual(snapshot.value["queue"]["items"].array.map { $0["id"].string }, ["18446744073709551615", "9007199254740993"])
        XCTAssertEqual(snapshot.items[1]["resources"].array[0]["originalReference"].string, "figures/qc.png")
        XCTAssertTrue(snapshot.writable)
        XCTAssertThrowsError(try RemoteSnapshot(value, route: ConversationRoute(project: "other", session: route.session)))
        let full = try fixture("native-conversations/v1/snapshot.json")
        XCTAssertEqual(try RemoteSnapshot(full, route: route).value["approvals"].array.count, 1)
        guard case .object(var malformed) = full else { return XCTFail() }
        malformed["approvals"] = .array([.object(["frame_id": .string("other"), "approval_id": .string("a")])])
        XCTAssertThrowsError(try RemoteSnapshot(.object(malformed), route: route))
    }

    func testHistoryRejectsStaleResponsesAndResetsAtNewBoundary() throws {
        let route = ConversationRoute(project: "project-a", session: "session-a")
        guard case .object(var value) = try fixture("native-conversations/v1/snapshot.json") else { return XCTFail() }
        value["next_before_seq"] = .integer(10)
        let first = try RemoteSnapshot(.object(value), route: route)
        var history = RemoteHistory()
        history.replace(first)
        value["next_before_seq"] = .null
        let page = try RemoteSnapshot(.object(value), route: route)
        history.prepend(page, expectedEpoch: first.epoch, expectedBoundary: .integer(10), requestedCursor: .integer(10))
        XCTAssertEqual(history.older.count, page.items.count)
        value["sequence"] = .integer(6)
        history.replace(try RemoteSnapshot(.object(value), route: route))
        XCTAssertEqual(history.snapshot, first)
        value["sequence"] = .integer(8)
        value["next_before_seq"] = .integer(20)
        history.replace(try RemoteSnapshot(.object(value), route: route))
        XCTAssertTrue(history.older.isEmpty)
        XCTAssertEqual(history.cursor, .integer(20))
        history.prepend(page, expectedEpoch: first.epoch, expectedBoundary: .integer(10), requestedCursor: .integer(10))
        XCTAssertTrue(history.older.isEmpty)
        value["epoch"] = .string("new-host"); value["sequence"] = .integer(1)
        history.replace(try RemoteSnapshot(.object(value), route: route))
        XCTAssertEqual(history.snapshot?.epoch, "new-host")
    }

    func testMarkdownAndLinksKeepUntrustedContentInert() {
        let text = "# Result\n\n| sample | value |\n| --- | --- |\n| A | 1 |\n\n```html\n<script>alert(1)</script>\n```\n\nBefore ![QC](figures/qc.png) after"
        XCTAssertEqual(RemoteMarkdown.blocks(text), [
            .heading(1, "Result"), .table([["sample", "value"], ["A", "1"]]),
            .code("<script>alert(1)</script>"), .paragraph("Before "), .image("figures/qc.png", "QC"), .paragraph(" after"),
        ])
        for path in ["https://example.test/image.png", "javascript:alert(1)", "file:///etc/passwd", "//example.test", "%2F%2Fexample.test", "javascript%3Aalert(1)"] {
            XCTAssertNil(RemoteMarkdown.localPath(path))
        }
        XCTAssertEqual(RemoteMarkdown.localPath("../figures/QC%20plot.png", directory: "results/reports"), "results/figures/QC plot.png")
    }
}
