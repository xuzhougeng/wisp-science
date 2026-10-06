import Foundation
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor SearchClient: NativeConversationQuerying {
    var held: CheckedContinuation<SettingsValue, Never>?
    var calls: [(String, [String: SettingsValue])] = []
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { throw ProjectBrowserError.invalidResponse }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        calls.append((command, args))
        if command.hasSuffix("searchfiles") {
            if args["query"]?.string == "slow" { return await withCheckedContinuation { held = $0 } }
            return hits("results/qc/sample.csv")
        }
        if command.hasSuffix("file_action") { return .bool(true) }
        return .array([])
    }
    func hits(_ path: String) -> SettingsValue { .array([.object(["path": .string(path), "name": .string("sample.csv"), "is_dir": .bool(false), "size": .integer(12)])]) }
    func isHeld() -> Bool { held != nil }
    func finish() { held?.resume(returning: hits("old/sample.csv")); held = nil }
    func recorded() -> [(String, [String: SettingsValue])] { calls }
}
final class NativePanelAlignmentTests: XCTestCase {
    private func artifact(_ id: String, path: String, location: String? = nil) throws -> NativePanelArtifact {
        var row: [String: SettingsValue] = ["id": .string(id), "name": .string(id), "kind": .string("text/csv"), "path": .string(path), "ts": .integer(1)]
        if let location { row["location"] = .string(location) }
        return try JSONDecoder().decode(NativePanelArtifact.self, from: JSONEncoder().encode(SettingsValue.object(row)))
    }
    func testDirectoryGroupsUseWorkspaceLocationAndPreserveRegistrationOrder() throws {
        let rows = [try artifact("b", path: "/work/project/snapshots/x.csv", location: "/work/project/results/qc/b.csv"), try artifact("a", path: "/work/project/results/qc/a.csv"), try artifact("root", path: "README.md"), try artifact("outside", path: "/other/run/data/c.csv")]
        let groups = NativeArtifactGroup.collect(registered: rows, messages: [], query: "", root: "/work/project")
        XCTAssertEqual(groups.map(\.id), [".", "data/", "results/qc/"])
        XCTAssertEqual(groups.last?.registered.map(\.id), ["b", "a"])
        XCTAssertEqual(NativeWorkspacePath.artifact(rows[0], root: "/work/project"), "results/qc/b.csv")
        XCTAssertEqual(NativeArtifactGroup.collect(registered: rows, messages: [], query: "results/qc", root: "/work/project").first?.registered.count, 2)
        XCTAssertEqual(NativeWorkspacePath.display("c:\\PROJECT\\Results\\QC.csv", root: "C:\\Project\\"), "Results/QC.csv")
        XCTAssertEqual(NativeWorkspacePath.display("s3://bucket/data/qc.csv", root: "/work/project"), "s3://bucket/data/qc.csv")
        XCTAssertEqual(NativeWorkspacePath.group("./qc.csv", root: "/work/project"), ".")
        XCTAssertEqual(NativeWorkspacePath.display("/work/project-other/a.csv", root: "/work/project"), "/work/project-other/a.csv")
    }
    @MainActor func testSearchIsProjectScopedAndLateRepliesCannotReplaceNewQueryOrClosedPanel() async throws {
        let client = SearchClient(); let model = NativePanelModel(client: client, projectID: "project", sessionID: "session")
        await model.refresh("files", directory: "data")
        let slow = Task { await model.searchFiles("slow") }
        while !(await client.isHeld()) { await Task.yield() }
        await model.searchFiles("sample")
        XCTAssertEqual(model.searchHits.first?.path, "results/qc/sample.csv")
        await client.finish(); await slow.value
        XCTAssertEqual(model.searchHits.first?.path, "results/qc/sample.csv")
        try await model.performFileAction(.rename, path: "results/qc/sample.csv", newPath: "results/qc/renamed.csv")
        let calls = await client.recorded()
        let search = try XCTUnwrap(calls.first { $0.0.hasSuffix("searchfiles") })
        XCTAssertEqual(search.1["session_id"], .string("session")); XCTAssertNil(search.1["path"])
        XCTAssertEqual(calls.first { $0.0.hasSuffix("file_action") }?.1["path"], .string("results/qc/sample.csv"))
        let late = Task { await model.searchFiles("slow") }
        while !(await client.isHeld()) { await Task.yield() }
        model.close(); await client.finish(); await late.value
        XCTAssertTrue(model.searchHits.isEmpty); XCTAssertFalse(model.searchLoading)
    }
    func testSaveAsCopiesOriginalBytesAndPreservesDestinationOnChangedSource() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("source.bin"); let output = root.appendingPathComponent("copy.bin")
        let bytes = Data([0, 255, 128, 10]); try bytes.write(to: input)
        let source = try JSONDecoder().decode(NativePanelExport.self, from: JSONEncoder().encode(SettingsValue.object(["path": .string(input.path), "name": .string("source.bin"), "total_bytes": .integer(4)])))
        try await NativePanelExportCopy.copy(source, to: output)
        XCTAssertEqual(try Data(contentsOf: output), bytes)
        try Data([1, 2]).write(to: output)
        try await NativePanelExportCopy.copy(source, to: output)
        XCTAssertEqual(try Data(contentsOf: output), bytes)
        do { try await NativePanelExportCopy.copy(source, to: input); XCTFail("Must not overwrite source") } catch {}
        try Data([1]).write(to: input)
        do { try await NativePanelExportCopy.copy(source, to: output); XCTFail("Changed source must fail") } catch {}
        XCTAssertEqual(try Data(contentsOf: output), bytes)
        XCTAssertFalse(try FileManager.default.contentsOfDirectory(atPath: root.path).contains { $0.hasPrefix(".wisp-export-") })
    }
}
