import Foundation

public struct ConversationItem: Codable, Equatable, Sendable {
    public let role: String
    public let text: String
    public let tool_name: String?
    public let input: String?
    public let ok: Bool?
    public let status: String?
}
public struct ConversationApproval: Codable, Equatable, Identifiable, Sendable {
    public var id: String { approval_id }
    public let approval_id: String
    public let frame_id: String
    public let message: String
    public let tool: String
    public let preview: String
}
public struct ConversationSnapshot: Codable, Sendable {
    public static let schemaID = "wisp.native-conversations.v1"
    public let schema: String
    public let epoch: String
    public let sequence: UInt64
    public let project_id: String
    public let session_id: String
    public let items: [ConversationItem]
    public let next_before_seq: Int64?
    public let user_offset: Int?
    public let running: Bool
    public let stopping: Bool
    public let read_only: Bool
    public let model_id: String
    public let acp_agent_id: String?
    public let request_id: String?
    public let error: String?
    public let approvals: [ConversationApproval]
    public let acp: ConversationAcpInteractions?

    public static func decode(_ value: SettingsValue, projectID: String, sessionID: String) throws -> Self {
        let snapshot = try JSONDecoder().decode(Self.self, from: JSONEncoder().encode(value))
        guard snapshot.schema == schemaID, !snapshot.epoch.isEmpty, snapshot.sequence > 0,
              snapshot.project_id == projectID, snapshot.session_id == sessionID,
              snapshot.approvals.allSatisfy({ $0.frame_id == sessionID }),
              (snapshot.acp?.permissions ?? []).allSatisfy({ $0.frame_id == sessionID && !$0.request_id.isEmpty }) else { throw ProjectBrowserError.invalidResponse }
        return snapshot
    }
}

public struct ConversationAcpInteractions: Codable, Sendable {
    public let permissions: [ConversationAcpPermission]
    public let question_ids: [String]
}
public struct ConversationAcpPermission: Codable, Equatable, Identifiable, Sendable {
    public var id: String { request_id }
    public let request_id: String
    public let frame_id: String
    public let title: String
    public let preview: String
    public let options: [ConversationAcpOption]
}
public struct ConversationAcpOption: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let name: String
    public let kind: String
}

public protocol NativeConversationQuerying: Sendable {
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue
}
public struct NativeConversationClient: NativeConversationQuerying {
    private let transport: any NativeSettingsQuerying
    public init(transport: any NativeSettingsQuerying) { self.transport = transport }
    public func snapshot(projectID: String, sessionID: String, beforeSeq: Int64? = nil) async throws -> ConversationSnapshot {
        let value = try await invoke("native_conversation_snapshot", args: ["session_id": .string(sessionID), "before_seq": beforeSeq.map(SettingsValue.integer) ?? .null], projectID: projectID)
        return try ConversationSnapshot.decode(value, projectID: projectID, sessionID: sessionID)
    }
    public func invoke(_ command: String, args: [String: SettingsValue] = [:], projectID: String) async throws -> SettingsValue {
        do { return try await transport.invoke(command, args: args, projectID: projectID) }
        catch {
            if error.localizedDescription.contains("Command is not available") {
                throw ProjectBrowserError.unavailable("当前桌面宿主版本不支持原生会话，请退出旧版桌面宿主并重新打开预览。")
            }
            throw error
        }
    }
}

public struct ConversationOutlineEntry: Codable, Equatable, Identifiable, Sendable {
    public var id: Int { user_index }
    public let user_index: Int
    public let text: String
    public let before_seq: Int64?
    public let sent_at: Int64?
    public let response_at: Int64?
}

/// Existing wisp-dto SessionSearchInfo shape used by the cross-project inbox.
public struct NativeInboxEntry: Codable, Identifiable, Sendable {
    public let id: String
    public let project_id: String
    public let project_name: String
    public let title: String
    public let ts: Int64
    public let activity_at: Int64
    public let status: String
}
