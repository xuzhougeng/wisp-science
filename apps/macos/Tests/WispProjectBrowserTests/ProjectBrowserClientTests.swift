import Foundation
import XCTest
@testable import WispProjectBrowser

final class ProjectBrowserClientTests: XCTestCase {
    private func fixture() throws -> Data {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        return try Data(contentsOf: root.appendingPathComponent("contracts/project-browser/v1/projects.json"))
    }

    func testSharedRustFixtureDecodesWithoutLosingProjectFields() throws {
        let snapshot = try ProjectBrowserClient.decode(fixture(), requestID: "projects-1")
        XCTAssertEqual(snapshot.activitySource, "persisted_only")
        let project = try XCTUnwrap(snapshot.projects.first)
        XCTAssertEqual(project.id, "research-1")
        XCTAssertEqual(project.name, "RNA-seq 研究")
        XCTAssertEqual(project.workspaceDirectory, "/Users/researcher/Projects/RNA seq")
        XCTAssertEqual(project.sessionCount, 3)
        XCTAssertEqual(project.artifactCount, 2)
        XCTAssertEqual(project.needsYouCount, 1)
        XCTAssertTrue(project.starred)
        XCTAssertTrue(project.syncConfigured)
        XCTAssertEqual(project.lastSyncedAt, 1789500000)
    }

    func testRejectsWrongSchemaRequestIdentityAndUnknownActivitySource() throws {
        let original = try XCTUnwrap(String(data: fixture(), encoding: .utf8))
        for changed in [
            original.replacingOccurrences(of: ProjectBrowserClient.schema, with: "future.v2"),
            original.replacingOccurrences(of: "projects-1", with: "another-request"),
            original.replacingOccurrences(of: "persisted_only", with: "unknown-source"),
        ] {
            XCTAssertThrowsError(try ProjectBrowserClient.decode(Data(changed.utf8), requestID: "projects-1"))
        }
    }

    func testServiceErrorsSurfaceAndEmptyProjectListsRemainValid() throws {
        let error = Data(#"{"schema":"wisp.project-browser.v1","id":"projects-1","type":"error","code":"query_failed","message":"Database unavailable"}"#.utf8)
        XCTAssertThrowsError(try ProjectBrowserClient.decode(error, requestID: "projects-1")) {
            XCTAssertEqual($0.localizedDescription, "Database unavailable")
        }
        let empty = Data(#"{"schema":"wisp.project-browser.v1","id":"projects-1","type":"projects","activity_source":"persisted_only","projects":[]}"#.utf8)
        XCTAssertTrue(try ProjectBrowserClient.decode(empty, requestID: "projects-1").projects.isEmpty)
    }

    func testProcessTransportPassesPathsAsArgumentsAndClosesStdin() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let executable = directory.appendingPathComponent("mock service")
        let database = directory.appendingPathComponent("data with spaces.sqlite")
        let response = String(decoding: try fixture(), as: UTF8.self)
        let quotedResponse = "'" + response.replacingOccurrences(of: "'", with: "'\\''") + "'"
        // A real child process verifies argv boundaries and stdin EOF. No shell
        // interpretation of the database path happens in the production client.
        let script = """
        #!/bin/sh
        test "$#" -eq 2 || exit 2
        test "$1" = '--database' || exit 3
        case "$2" in *'/data with spaces.sqlite') ;; *) exit 4 ;; esac
        IFS= read -r request || exit 5
        case "$request" in *'"list_projects"'*) ;; *) exit 6 ;; esac
        if IFS= read -r extra; then exit 7; fi
        printf '%s\\n' \(quotedResponse)
        """
        try script.write(to: executable, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
        let snapshot = try await ProjectBrowserClient(executableURL: executable).listProjects(databaseURL: database)
        XCTAssertEqual(snapshot.projects.count, 1)
    }

    func testStarTransportOptsInAndEncodesBooleanCommand() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let executable = directory.appendingPathComponent("mock service")
        let response = String(decoding: try fixture(), as: UTF8.self)
        let quotedResponse = "'" + response.replacingOccurrences(of: "'", with: "'\\''") + "'"
        let script = """
        #!/bin/sh
        test "$#" -eq 3 || exit 2
        test "$3" = '--allow-project-writes' || exit 3
        IFS= read -r request || exit 4
        printf '%s' "$request" > "$2"
        printf '%s\\n' \(quotedResponse)
        """
        try script.write(to: executable, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
        let captured = directory.appendingPathComponent("request.json")
        let client = ProjectBrowserClient(executableURL: executable)
        _ = try await client.setProjectStarred(databaseURL: captured, projectID: "research-1", starred: true)
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let expected = try JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent("contracts/project-browser/v1/set-project-starred.json"))) as! NSDictionary
        XCTAssertEqual(try JSONSerialization.jsonObject(with: Data(contentsOf: captured)) as! NSDictionary, expected)
        _ = try await client.setProjectStarred(databaseURL: captured, projectID: "research-1", starred: false)
        let unstar = try JSONSerialization.jsonObject(with: Data(contentsOf: captured)) as! [String: Any]
        XCTAssertEqual(unstar["starred"] as? Bool, false)
    }

    func testSessionAndTranscriptFixturesAndIdentityValidation() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let sessions = try Data(contentsOf: root.appendingPathComponent("contracts/project-browser/v1/sessions.json"))
        let rows = try ProjectBrowserClient.decodeSessions(sessions, requestID: "projects-1")
        XCTAssertEqual(rows.first?.projectID, "research-1")
        XCTAssertEqual(rows.first?.status, "needs_you")
        XCTAssertThrowsError(try ProjectBrowserClient.decodeSessions(sessions, requestID: "wrong"))
        let transcript = try Data(contentsOf: root.appendingPathComponent("contracts/project-browser/v1/transcript.json"))
        let page = try ProjectBrowserClient.decodeTranscript(transcript, requestID: "projects-1")
        XCTAssertEqual(page.messages.map(\.seq), [6, 7])
        XCTAssertEqual(page.nextBeforeSeq, 6)
        XCTAssertThrowsError(try ProjectBrowserClient.decodeTranscript(transcript, requestID: "wrong"))
        XCTAssertThrowsError(try ProjectBrowserClient.decodeSessions(transcript, requestID: "projects-1"))
    }

    func testStuckServiceExplainsTheFolderPermissionPrompt() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        // Stands in for a service blocked in open() while macOS waits on a consent prompt.
        let executable = directory.appendingPathComponent("stuck-service")
        try "#!/bin/sh\nexec sleep 30\n".write(to: executable, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
        var client = ProjectBrowserClient(executableURL: executable)
        client.deadlineSeconds = 0.3
        do {
            _ = try await client.listProjects(databaseURL: directory.appendingPathComponent("wisp.sqlite"))
            XCTFail("Expected the deadline to stop the service")
        } catch {
            XCTAssertEqual(error.localizedDescription, ProjectBrowserClient.timeoutMessage)
        }
    }

    func testMissingServiceReturnsAnActionableError() async throws {
        let client = ProjectBrowserClient(executableURL: URL(fileURLWithPath: "/missing/wisp-service"))
        do {
            _ = try await client.listProjects(databaseURL: URL(fileURLWithPath: "/missing/wisp.sqlite"))
            XCTFail("Expected missing-service failure")
        } catch {
            XCTAssertTrue(error.localizedDescription.contains("build_native_macos.sh"))
        }
    }
}
