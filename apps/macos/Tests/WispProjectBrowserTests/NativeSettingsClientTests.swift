import Foundation
import Darwin
import XCTest
@testable import WispProjectBrowser

final class NativeSettingsClientTests: XCTestCase {
    func testLaunchedHelperGetsAnOwnerPipeAndFailureReleasesOnlyThatOwner() throws {
        let owners = NativeSettingsHostOwners()
        let first = Process(), second = Process()
        let firstID = try owners.configure(first), secondID = try owners.configure(second)
        XCTAssertEqual(first.arguments, ["--native-settings-host", "--native-settings-owner-stdin"])
        let firstPipe = try XCTUnwrap(first.standardInput as? Pipe)
        let secondPipe = try XCTUnwrap(second.standardInput as? Pipe)
        XCTAssertNotEqual(fcntl(firstPipe.fileHandleForWriting.fileDescriptor, F_GETFD) & FD_CLOEXEC, 0)
        owners.release(firstID)
        XCTAssertTrue(try firstPipe.fileHandleForReading.readToEnd()?.isEmpty ?? true)
        // Another active client's lifetime is unaffected by a failed launch.
        try secondPipe.fileHandleForWriting.write(contentsOf: Data([42]))
        XCTAssertEqual(try secondPipe.fileHandleForReading.read(upToCount: 1), Data([42]))
        second.terminationHandler?(second)
        XCTAssertTrue(try secondPipe.fileHandleForReading.readToEnd()?.isEmpty ?? true)
        owners.release(firstID); owners.release(secondID) // idempotent cleanup
    }

    func testHostValidationRejectsRemoteRedirectTargetsAndDifferentDatabase() throws {
        let database = URL(fileURLWithPath: "/tmp/native-test/wisp.sqlite")
        func host(_ endpoint: String, database: String = "/tmp/native-test/wisp.sqlite", token: String = String(repeating: "a", count: 64)) -> NativeSettingsClient.Descriptor {
            .init(schema: NativeSettingsClient.schema, endpoint: endpoint, token: token, database: database, pid: 1)
        }
        XCTAssertNoThrow(try NativeSettingsClient.validate(host("http://127.0.0.1:12345/invoke"), databaseURL: database))
        for endpoint in ["https://example.com/invoke", "http://localhost:1234/invoke", "http://127.0.0.1:1234/wrong", "http://127.0.0.1:1234/invoke?redirect=x", "http://user@127.0.0.1:1234/invoke"] {
            XCTAssertThrowsError(try NativeSettingsClient.validate(host(endpoint), databaseURL: database))
        }
        XCTAssertThrowsError(try NativeSettingsClient.validate(host("http://127.0.0.1:1234/invoke", database: "/tmp/other.sqlite"), databaseURL: database))
        XCTAssertThrowsError(try NativeSettingsClient.validate(host("http://127.0.0.1:1234/invoke", token: "short"), databaseURL: database))
    }

    func testResponseCorrelationErrorsAndSuccessfulVoid() throws {
        func data(_ body: String) -> Data { Data(body.utf8) }
        let good = #"{"schema":"wisp.native-settings.v1","id":"save-1","result":null,"error":null}"#
        XCTAssertEqual(try NativeSettingsClient.decode(data(good), requestID: "save-1"), .null)
        XCTAssertThrowsError(try NativeSettingsClient.decode(data(good), requestID: "different"))
        XCTAssertThrowsError(try NativeSettingsClient.decode(data(good.replacingOccurrences(of: "v1", with: "v2")), requestID: "save-1"))
        XCTAssertThrowsError(try NativeSettingsClient.decode(data(#"{"schema":"wisp.native-settings.v1","id":"save-1","result":null,"error":"Not saved"}"#), requestID: "save-1"))
        XCTAssertThrowsError(try NativeSettingsClient.decode(data(#"{"schema":"wisp.native-settings.v1","id":"save-1"}"#), requestID: "save-1"))
    }

    func testUnknownConfigurationAndSecretPresenceSurviveRoundTrip() throws {
        let raw = Data(#"{"enabled":true,"max_tokens":9223372036854775807,"nested":{"future_option":[false,null,"中文"]},"has_api_key":true}"#.utf8)
        let original = try JSONDecoder().decode(SettingsValue.self, from: raw)
        let decoded = try JSONDecoder().decode(SettingsValue.self, from: JSONEncoder().encode(original))
        XCTAssertEqual(original, decoded)
        XCTAssertEqual(decoded["max_tokens"].integer, Int64.max)
        XCTAssertEqual(decoded["nested"]["future_option"].array, [.bool(false), .null, .string("中文")])
        XCTAssertFalse(decoded.object.keys.contains("key"))
    }
}
