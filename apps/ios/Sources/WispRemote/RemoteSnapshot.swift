import Foundation

public struct ConversationRoute: Hashable, Sendable {
    public let project: String
    public let session: String
    public init(project: String, session: String) { self.project = project; self.session = session }
    public var args: [String: RemoteValue] { ["session_id": .string(session)] }

    /// Existing native-assistant/v1 binding, also used to find its offline
    /// snapshot. Online access must first resolve native_assistant_open.
    public static let researchAssistant = ConversationRoute(project: "assistant:research", session: "research-assistant")
    public static func assistant(from value: RemoteValue) throws -> ConversationRoute {
        guard value["project_id"].string == researchAssistant.project,
              value["session_id"].string == researchAssistant.session else { throw RemoteError.incompatibleResponse }
        return researchAssistant
    }
}

/// A replacement snapshot, as defined by wisp-dto::native_conversations.
public struct RemoteSnapshot: Equatable, Sendable {
    public let value: RemoteValue
    public let epoch: String
    public let sequence: UInt64
    public init(_ value: RemoteValue, route: ConversationRoute) throws {
        guard value["schema"].string == "wisp.native-conversations.v1",
              value["project_id"].string == route.project, value["session_id"].string == route.session,
              !value["epoch"].string.isEmpty, let sequence = value["sequence"].uint, sequence > 0,
              case .array = value["items"], case .array = value["approvals"],
              case .bool = value["running"], case .bool = value["read_only"], case .bool = value["stopping"],
              value["approvals"].array.allSatisfy({ $0["frame_id"].string == route.session && !$0["approval_id"].string.isEmpty }),
              value["queue"]["items"].array.allSatisfy({ !$0["id"].string.isEmpty && !$0["digest"].string.isEmpty }) else {
            throw RemoteError.incompatibleResponse
        }
        self.value = value
        epoch = value["epoch"].string
        self.sequence = sequence
    }
    public var items: [RemoteValue] { value["items"].array }
    public var running: Bool { value["running"].bool }
    public var writable: Bool { !value["read_only"].bool && value["acp_agent_id"].string.isEmpty }
    public var before: RemoteValue { value["next_before_seq"] }
    public func supersedes(_ previous: Self?) -> Bool {
        guard let previous else { return true }
        return epoch != previous.epoch || sequence >= previous.sequence
    }
}

/// Paging resets when the live window advances; retaining pages from the old
/// boundary would leave an invisible hole in the conversation.
public struct RemoteHistory: Sendable {
    public private(set) var snapshot: RemoteSnapshot?
    public private(set) var older: [RemoteValue] = []
    public private(set) var cursor: RemoteValue = .null
    public init() {}
    public mutating func replace(_ next: RemoteSnapshot) {
        guard next.supersedes(snapshot) else { return }
        if snapshot == nil || next.epoch != snapshot?.epoch || next.before != snapshot?.before {
            older = []
            cursor = next.before
        }
        snapshot = next
    }
    public mutating func prepend(_ page: RemoteSnapshot, expectedEpoch: String, expectedBoundary: RemoteValue,
                                 requestedCursor: RemoteValue) {
        guard snapshot?.epoch == expectedEpoch, page.epoch == expectedEpoch,
              snapshot?.before == expectedBoundary, cursor == requestedCursor else { return }
        older = page.items + older
        cursor = page.before
    }
}
