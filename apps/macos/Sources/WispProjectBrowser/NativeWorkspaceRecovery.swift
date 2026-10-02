import Foundation

/// Existing wisp-dto WorkspaceSessionRecoveryPreview/Result wire shapes.
public struct NativeWorkspaceRecoveryPreview: Codable, Equatable, Identifiable, Sendable {
    public var id: String { workspace_dir }
    public let workspace_dir: String
    public let suggested_name: String
    public let archive_count: Int
    public let valid_archive_count: Int
    public let recoverable_session_count: Int
    public let message_count: Int
    public let invalid_archive_count: Int
    public let duplicate_archive_count: Int
    public let earliest_message_at: Int64?
    public let latest_message_at: Int64?
}

public struct NativeWorkspaceRecoveryResult: Codable, Equatable, Sendable {
    public let project_id: String
    public let project_name: String
    public let recovered_session_count: Int
    public let message_count: Int
    public let invalid_archive_count: Int
    public let duplicate_archive_count: Int
}
