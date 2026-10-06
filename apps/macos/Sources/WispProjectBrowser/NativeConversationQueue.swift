import Foundation

/// wisp-dto::native_queue uses decimal strings, including IDs above 2^53.
public struct ConversationQueueItem: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let digest: String
    public let state: String
    public let message: String
    public let attachments: [String]
    public let references: [SettingsValue]
    var valid: Bool {
        ConversationQueueSnapshot.validID(id) && digest.utf8.count == 64 && digest.utf8.allSatisfy { (48...57).contains($0) || (97...102).contains($0) }
            && ["queued", "cutin_pending"].contains(state)
    }
}
public struct ConversationQueueOutcome: Codable, Equatable, Sendable {
    public let id: String
    public let state: String
}
public struct ConversationQueueSnapshot: Codable, Equatable, Sendable {
    public let items: [ConversationQueueItem]
    public let outcomes: [ConversationQueueOutcome]
    public let can_cut_in: Bool
    static func validID(_ id: String) -> Bool { UInt64(id).map { String($0) == id } ?? false }
    var valid: Bool {
        items.allSatisfy(\.valid) && Set(items.map(\.id)).count == items.count
            && outcomes.allSatisfy { Self.validID($0.id) && ["started", "completed", "cancelled", "superseded", "failed"].contains($0.state) }
    }
}
