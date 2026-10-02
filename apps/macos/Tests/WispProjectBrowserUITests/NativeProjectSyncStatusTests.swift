import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

final class NativeProjectSyncStatusTests: XCTestCase {
    private func project(_ state: String? = nil, configured: Bool = false, time: Int64? = nil) throws -> ProjectSummary {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        var value = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-projects/v1/import.json")))["result"]
        value["sync_configured"] = .bool(configured)
        if let state { value["folder_sync"] = .string(state) }
        value["last_synced_at"] = time.map(SettingsValue.integer) ?? .null
        return try NativeProjectCommand.summary(from: value)
    }
    func testLegacyPayloadHasNoFolderStateAndSyncIsNotInvented() throws {
        let old = try project()
        XCTAssertNil(old.folderSync)
        XCTAssertNil(NativeProjectSyncStatus(old))
        XCTAssertNil(NativeProjectSyncStatus.lastSaved(old))
        XCTAssertEqual(try NativeProjectSyncStatus(project(configured: true))?.label, "已配置项目同步")
    }
    func testAllFolderStatesHaveDistinctLabelsAndUnknownStateIsNotCalledSaved() throws {
        let states = ["saved", "unpublished", "remote-newer", "waiting", "conflict"]
        let labels = try states.map { try XCTUnwrap(NativeProjectSyncStatus(project($0))) }
        XCTAssertEqual(Set(labels.map(\.label)).count, states.count)
        XCTAssertEqual(labels.map(\.needsAttention), [false, false, true, false, true])
        let unknown = try XCTUnwrap(NativeProjectSyncStatus(project("future-state")))
        XCTAssertTrue(unknown.label.contains("future-state"))
        XCTAssertFalse(unknown.label.contains("已保存"))
        XCTAssertNotNil(try NativeProjectSyncStatus.lastSaved(project("saved", time: 1_700_000_000)))
    }
}
