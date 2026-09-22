import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor ImportTransport: NativeSettingsQuerying {
    var calls: [(command: String, args: [String: SettingsValue], projectID: String?)] = []
    var mode = "success"
    private var release: CheckedContinuation<SettingsValue, Error>?
    func setMode(_ mode: String) { self.mode = mode }
    func callCount() -> Int { calls.count }
    func lastCall() -> (command: String, args: [String: SettingsValue], projectID: String?) { calls.last! }
    func suspend() { mode = "hang" }
    func isHanging() -> Bool { release != nil }
    func resume(with value: SettingsValue) {
        release?.resume(returning: value)
        release = nil
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        calls.append((command, args, projectID))
        switch mode {
        case "invalid":
            throw ProjectBrowserError.service("not a valid project archive: invalid zip")
        case "present":
            throw ProjectBrowserError.service("This project is already present on this device.")
        case "lost":
            throw ProjectBrowserError.service("connection reset")
        case "hang":
            return try await withCheckedThrowingContinuation { release = $0 }
        default:
            var root = URL(fileURLWithPath: #filePath)
            for _ in 0..<5 { root.deleteLastPathComponent() }
            let fixture = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-projects/v1/import.json")))
            return fixture["result"]
        }
    }
}

final class ImportProjectTests: XCTestCase {
    @MainActor func testCancelingThePickerDoesNotCallTheHost() async {
        let transport = ImportTransport()
        let model = ProjectBrowserModel(client: EmptyImportList(), databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: transport)
        await model.importChosenArchive(nil)
        let calls = await transport.callCount()
        XCTAssertEqual(calls, 0)
        XCTAssertNil(model.importError)
        XCTAssertNil(model.activeProjectID)
    }

    @MainActor func testSuccessOpensTheImportedProjectOnce() async throws {
        let transport = ImportTransport()
        let lists = EmptyImportList()
        let model = ProjectBrowserModel(client: lists, databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: transport)
        await model.importChosenArchive(URL(fileURLWithPath: "/Users/researcher/Exports/RNA seq.zip"))
        let call = await transport.lastCall()
        let calls = await transport.callCount()
        XCTAssertEqual(calls, 1)
        XCTAssertEqual(call.command, NativeProjectCommand.importArchive)
        XCTAssertNil(call.projectID)
        XCTAssertEqual(call.args["archive_path"]?.string, "/Users/researcher/Exports/RNA seq.zip")
        XCTAssertEqual(model.activeProjectID, "research-1")
        XCTAssertNil(model.importError)
        XCTAssertFalse(model.importBusy)
        let listed = await lists.listCount()
        XCTAssertEqual(listed, 1)
        await Task.yield()
        let after = await transport.callCount()
        XCTAssertEqual(after, 1)
    }

    @MainActor func testInvalidArchiveAndLostReplyAreNotRetried() async {
        let transport = ImportTransport()
        let model = ProjectBrowserModel(client: EmptyImportList(), databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: transport)
        await transport.setMode("invalid")
        let archive = URL(fileURLWithPath: "/tmp/bad.zip")
        await model.importChosenArchive(archive)
        XCTAssertTrue(model.importError?.contains("不是有效的项目归档") == true)
        XCTAssertNil(model.activeProjectID)
        await transport.setMode("present")
        await model.importChosenArchive(archive)
        XCTAssertEqual(model.importError, "这个项目已经在这台设备上。")
        await transport.setMode("lost")
        await model.importChosenArchive(archive)
        XCTAssertTrue(model.importError?.contains("不会自动重试") == true)
        let calls = await transport.callCount()
        XCTAssertEqual(calls, 3)
        await Task.yield()
        let after = await transport.callCount()
        XCTAssertEqual(after, 3)
    }

    @MainActor func testSecondImportWhileBusyDoesNotSendAgain() async {
        let transport = ImportTransport()
        await transport.suspend()
        let model = ProjectBrowserModel(client: EmptyImportList(), databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: transport)
        let first = Task { await model.importChosenArchive(URL(fileURLWithPath: "/tmp/study.zip")) }
        var hanging = false
        for _ in 0..<1000 {
            if await transport.isHanging() { hanging = true; break }
            await Task.yield()
        }
        XCTAssertTrue(hanging)
        await model.importChosenArchive(URL(fileURLWithPath: "/tmp/other.zip"))
        let busyCalls = await transport.callCount()
        XCTAssertEqual(busyCalls, 1)
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let fixture = try! JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-projects/v1/import.json")))
        await transport.resume(with: fixture["result"])
        await first.value
        XCTAssertEqual(model.activeProjectID, "research-1")
        let finished = await transport.callCount()
        XCTAssertEqual(finished, 1)
    }
}

private actor EmptyImportList: ProjectBrowserQuerying {
    var lists = 0
    func listCount() -> Int { lists }
    func listProjects(databaseURL: URL) async throws -> ProjectListSnapshot {
        lists += 1
        return ProjectListSnapshot(projects: [], activitySource: "persisted_only")
    }
    func listSessions(databaseURL: URL, projectID: String?) async throws -> [BrowserSession] { [] }
    func transcript(databaseURL: URL, projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> TranscriptPage {
        TranscriptPage(messages: [], nextBeforeSeq: nil)
    }
}
