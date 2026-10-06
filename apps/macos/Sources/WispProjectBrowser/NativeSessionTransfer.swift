import Foundation

public enum NativeSessionTransferMode: String, Codable, Sendable { case copy, move }
public struct NativeRetainedSessionArtifact: Codable, Equatable, Sendable {
    public let name: String
    public let reason: String
}
public struct NativeSessionArtifactPreview: Codable, Equatable, Sendable {
    public let fingerprint: String
    public let artifacts: [String]
    public let files: [String]
    public let retained: [NativeRetainedSessionArtifact]
}
public struct NativeSessionTransferPreview: Codable, Equatable, Sendable {
    public static let schema = "wisp.native-session-transfer.v1"
    public let schema: String
    public let project_id: String
    public let session_id: String
    public let target_project_id: String
    public let mode: NativeSessionTransferMode
    public let title: String
    public let message_count: Int
    public let revision: String
    public let artifacts: NativeSessionArtifactPreview?
    public let artifact_error: String?
    public func arguments(includeArtifacts: Bool) -> [String: SettingsValue] {
        ["session_id": .string(session_id), "target_project_id": .string(target_project_id), "mode": .string(mode.rawValue),
         "revision": .string(revision), "include_artifacts": .bool(includeArtifacts),
         "artifact_fingerprint": includeArtifacts ? .string(artifacts?.fingerprint ?? "") : .null]
    }
    public static func decode(_ value: SettingsValue, project: String, session: String, target: String, mode: NativeSessionTransferMode) throws -> Self {
        let result = try JSONDecoder().decode(Self.self, from: JSONEncoder().encode(value))
        guard result.schema == Self.schema, result.project_id == project, result.session_id == session,
              result.target_project_id == target, target != project, !target.isEmpty, result.mode == mode,
              !result.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, result.message_count >= 0,
              result.revision.count == 64, result.revision.allSatisfy({ "0123456789abcdefABCDEF".contains($0) }),
              mode == .move ? (result.artifacts != nil) != (result.artifact_error != nil) : result.artifacts == nil && result.artifact_error == nil else { throw ProjectBrowserError.invalidResponse }
        if let artifacts = result.artifacts {
            guard !artifacts.fingerprint.isEmpty, artifacts.artifacts.count <= 100_000, artifacts.files.count <= 100_000,
                  artifacts.retained.count <= 100_000, artifacts.artifacts.allSatisfy({ !$0.isEmpty }),
                  artifacts.files.allSatisfy({ !$0.isEmpty }), Set(artifacts.files).count == artifacts.files.count,
                  artifacts.retained.allSatisfy({ !$0.name.isEmpty && ["shared", "upload", "changed", "unavailable"].contains($0.reason) }) else { throw ProjectBrowserError.invalidResponse }
        }
        return result
    }
}
public struct NativeSessionTransferResult: Codable, Equatable, Sendable {
    public let schema: String
    public let project_id: String
    public let session_id: String
    public let target_project_id: String
    public let mode: NativeSessionTransferMode
    public let include_artifacts: Bool
    public let frame_id: String
    public static func decode(_ value: SettingsValue, reviewed: NativeSessionTransferPreview, includeArtifacts: Bool) throws -> Self {
        let result = try JSONDecoder().decode(Self.self, from: JSONEncoder().encode(value))
        guard result.schema == NativeSessionTransferPreview.schema, result.project_id == reviewed.project_id,
              result.session_id == reviewed.session_id, result.target_project_id == reviewed.target_project_id,
              result.mode == reviewed.mode, result.include_artifacts == includeArtifacts,
              !result.frame_id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, result.frame_id != result.session_id else { throw ProjectBrowserError.invalidResponse }
        return result
    }
}
