import Foundation

/// Shared wisp-dto::native_session_import archive preview/result contracts.
public struct NativeSessionImportLine: Codable, Equatable, Sendable {
    public let role: String
    public let text: String
}
public struct NativeSessionArchivePreview: Codable, Equatable, Sendable {
    public static let schema = "wisp.native-session-import.v1"
    public let schema: String
    public let project_id: String
    public let archive_path: String
    public let sha256: String
    public let source_session_id: String
    public let title: String
    public let message_count: Int
    public let artifacts: [String]
    public let messages: [NativeSessionImportLine]
    public let existing_session_id: String?
    public let state: String
    public var arguments: [String: SettingsValue] {
        ["archive_path": .string(archive_path), "sha256": .string(sha256), "source_session_id": .string(source_session_id)]
    }
    public static func decode(_ value: SettingsValue, project: String, path: String) throws -> Self {
        let result = try JSONDecoder().decode(Self.self, from: JSONEncoder().encode(value))
        guard result.schema == Self.schema, result.project_id == project, result.archive_path == path,
              !result.source_session_id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              result.sha256.count == 64, result.sha256.allSatisfy({ "0123456789abcdefABCDEF".contains($0) }),
              result.message_count > 0, result.artifacts.count <= 4096, result.artifacts.allSatisfy({ !$0.isEmpty }),
              result.messages.count <= 4, result.messages.allSatisfy({ ["user", "assistant"].contains($0.role) && $0.text.unicodeScalars.count <= 600 }),
              ["new", "updatable", "imported"].contains(result.state),
              result.state == "new" ? result.existing_session_id == nil : result.existing_session_id?.isEmpty == false else { throw ProjectBrowserError.invalidResponse }
        return result
    }
}
public struct NativeSessionArchiveImportResult: Codable, Equatable, Sendable {
    public let schema: String
    public let project_id: String
    public let source_session_id: String
    public let frame_id: String
    public let status: String
    public let message_count: Int
    public let artifact_count: Int
    public let missing_artifacts: [String]
    public static func decode(_ value: SettingsValue, reviewed: NativeSessionArchivePreview) throws -> Self {
        let result = try JSONDecoder().decode(Self.self, from: JSONEncoder().encode(value))
        guard result.schema == NativeSessionArchivePreview.schema, result.project_id == reviewed.project_id,
              result.source_session_id == reviewed.source_session_id, !result.frame_id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              ["imported", "updated", "skipped"].contains(result.status), result.message_count == reviewed.message_count,
              result.artifact_count >= 0, result.artifact_count <= reviewed.artifacts.count,
              reviewed.existing_session_id == nil || result.frame_id == reviewed.existing_session_id,
              result.missing_artifacts.count <= reviewed.artifacts.count else { throw ProjectBrowserError.invalidResponse }
        return result
    }
}
