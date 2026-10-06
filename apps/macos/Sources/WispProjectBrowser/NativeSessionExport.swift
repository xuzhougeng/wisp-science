import Foundation

public struct NativeSessionExportArtifact: Codable, Equatable, Sendable {
    public let path: String
    public let mime: String
    public let bytes: UInt64
}
public struct NativeSessionExportMissingArtifact: Codable, Equatable, Sendable {
    public let path: String
    public let error: String
}
public struct NativeSessionExportPreview: Codable, Equatable, Sendable {
    public static let schema = "wisp.native-session-export.v1"
    public let schema: String
    public let project_id: String
    public let session_id: String
    public let title: String
    public let revision: String
    public let include_artifacts: Bool
    public let head_epoch: Int64
    public let message_count: Int
    public let tool_call_count: Int
    public let terminal_event_count: Int
    public let artifacts: [NativeSessionExportArtifact]
    public let missing_artifacts: [NativeSessionExportMissingArtifact]
    public let artifact_bytes: UInt64
    public let default_filename: String
    public func arguments(destination: URL) -> [String: SettingsValue] {
        ["session_id": .string(session_id), "include_artifacts": .bool(include_artifacts), "revision": .string(revision), "destination_path": .string(destination.path)]
    }
    public static func validHash(_ value: String) -> Bool {
        value.count == 64 && value.allSatisfy { "0123456789abcdefABCDEF".contains($0) }
    }
    public static func decode(_ value: SettingsValue, project: String, session: String, includeArtifacts: Bool) throws -> Self {
        let result = try JSONDecoder().decode(Self.self, from: JSONEncoder().encode(value))
        var total: UInt64 = 0
        for file in result.artifacts {
            let (sum, overflow) = total.addingReportingOverflow(file.bytes)
            guard !overflow else { throw ProjectBrowserError.invalidResponse }; total = sum
        }
        guard result.schema == Self.schema, result.project_id == project, result.session_id == session,
              result.include_artifacts == includeArtifacts, result.head_epoch >= 0, result.message_count > 0,
              result.tool_call_count >= 0, result.terminal_event_count >= 0, validHash(result.revision),
              !result.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              !result.default_filename.contains("/"), !result.default_filename.contains("\\"), !result.default_filename.contains("\0"),
              result.default_filename.utf8.count <= 255, result.default_filename.hasSuffix(".zip"),
              result.artifacts.count <= 100_000, result.missing_artifacts.count <= 100_000,
              Set(result.artifacts.map(\.path)).count == result.artifacts.count,
              result.artifacts.allSatisfy({ !$0.path.isEmpty && !$0.mime.isEmpty }),
              result.missing_artifacts.allSatisfy({ !$0.path.isEmpty && !$0.error.isEmpty }),
              total == result.artifact_bytes,
              includeArtifacts || (result.artifacts.isEmpty && result.missing_artifacts.isEmpty) else { throw ProjectBrowserError.invalidResponse }
        return result
    }
}
public struct NativeSessionExportResult: Codable, Equatable, Sendable {
    public let schema: String
    public let project_id: String
    public let session_id: String
    public let revision: String
    public let include_artifacts: Bool
    public let destination_path: String
    public let bytes: UInt64
    public let checksum: String
    public static func decode(_ value: SettingsValue, reviewed: NativeSessionExportPreview, destination: URL) throws -> Self {
        let result = try JSONDecoder().decode(Self.self, from: JSONEncoder().encode(value))
        guard result.schema == NativeSessionExportPreview.schema, result.project_id == reviewed.project_id,
              result.session_id == reviewed.session_id, result.revision == reviewed.revision,
              result.include_artifacts == reviewed.include_artifacts, result.destination_path == destination.path,
              result.bytes > 0, NativeSessionExportPreview.validHash(result.checksum) else { throw ProjectBrowserError.invalidResponse }
        return result
    }
}
