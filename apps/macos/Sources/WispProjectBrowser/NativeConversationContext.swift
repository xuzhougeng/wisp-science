import Foundation

/// Shared native_conversations::ContextView and SessionContextState contracts.
public struct NativeConversationContext: Codable, Sendable {
    public let project_id: String
    public let session_id: String
    public let items: [ConversationItem]
    public let details: NativeContextDetails
    public let state: NativeContextState
    public static func decode(_ value: SettingsValue, project: String, session: String) throws -> Self {
        let context = try JSONDecoder().decode(Self.self, from: JSONEncoder().encode(value))
        guard context.project_id == project, context.session_id == session,
              Set(context.state.compactions.map(\.epoch)).count == context.state.compactions.count else { throw ProjectBrowserError.invalidResponse }
        return context
    }
}
public struct NativeContextDetails: Codable, Sendable {
    public struct Tool: Codable, Sendable { public let name: String; public let description: String }
    public let system_prompt: String
    public let tool_definitions: [Tool]
    public let rules: String
    public let skills: String
    public let mcp_dynamic_tools: [Tool]
    public let subagent_definitions: [Tool]
}
public struct NativeContextState: Codable, Sendable {
    public let head_epoch: UInt64
    public let in_context_from_user_index: Int?
    public let compactions: [NativeContextCompaction]
    public let undone_epochs: [UInt64]
    public var undoable: NativeContextCompaction? { compactions.first { $0.epoch == head_epoch && $0.can_undo && !undone_epochs.contains($0.epoch) } }
}
public struct NativeContextCompaction: Codable, Sendable, Identifiable {
    public var id: UInt64 { epoch }
    public let epoch: UInt64
    public let before: UInt64
    public let after: UInt64
    public let strategy: String
    public let checkpoint: String?
    public let kept_from_user_index: Int?
    public let can_undo: Bool
    public let undo_reason: String?
}
