import Foundation

/// Decoder for wisp-dto::AcpSessionState. ACP's own payload retains camelCase.
public struct ConversationAcpState: Codable, Equatable, Sendable {
    public let frameID: String
    public let modes: SettingsValue?
    public let configOptions: [SettingsValue]?
    enum CodingKeys: String, CodingKey { case frameID = "frameId", modes, configOptions }
    public var currentMode: String? {
        guard case .string(let value) = modes?["currentModeId"] else { return nil }
        return value
    }
    public var modeChoices: [ConversationAcpChoice] {
        (modes?["availableModes"].array ?? []).compactMap { row in
            guard case .string(let id) = row["id"], !id.isEmpty else { return nil }
            return .init(id: id, label: row["name"].string.isEmpty ? id : row["name"].string)
        }
    }
    public var configurations: [ConversationAcpConfig] {
        (configOptions ?? []).compactMap { row in
            guard case .string(let id) = row["id"], !id.isEmpty else { return nil }
            return .init(id: id, label: row["name"].string.isEmpty ? id : row["name"].string,
                         description: row["description"].string, kind: row["type"].string,
                         current: row["currentValue"], choices: Self.choices(row))
        }
    }
    public var hasModeConfiguration: Bool { configurations.contains { $0.id == "mode" || $0.label.lowercased() == "mode" } }
    public var valid: Bool {
        !frameID.isEmpty && Set(modeChoices.map(\.id)).count == modeChoices.count
            && Set(configurations.map(\.id)).count == configurations.count
            && configurations.allSatisfy { Set($0.choices.map(\.id)).count == $0.choices.count }
    }
    public func allows(_ id: String, value: SettingsValue) -> Bool {
        guard let option = configurations.first(where: { $0.id == id }) else { return false }
        switch (option.kind, value) {
        case ("boolean", .bool): return true
        case ("select", .string(let selected)): return option.choices.contains { $0.id == selected }
        default: return false
        }
    }
    public var exitPlanMode: String? {
        guard currentMode?.lowercased().contains("plan") == true else { return nil }
        return modeChoices.first { $0.id == "default" }?.id ?? modeChoices.first { !$0.id.lowercased().contains("plan") }?.id
    }
    private static func choices(_ option: SettingsValue) -> [ConversationAcpChoice] {
        option["options"].array.flatMap { row -> [SettingsValue] in
            if case .string = row["value"] { return [row] }
            return row["options"].array
        }.compactMap { row in
            guard case .string(let value) = row["value"], !value.isEmpty else { return nil }
            return .init(id: value, label: row["name"].string.isEmpty ? value : row["name"].string)
        }
    }
}
public struct ConversationAcpChoice: Equatable, Identifiable, Sendable {
    public let id: String
    public let label: String
}
public struct ConversationAcpConfig: Equatable, Identifiable, Sendable {
    public let id: String
    public let label: String
    public let description: String
    public let kind: String
    public let current: SettingsValue
    public let choices: [ConversationAcpChoice]
}

/// Shared wisp-dto::native_conversations::PlanProposal shape.
public struct ConversationPlanProposal: Codable, Equatable, Hashable, Sendable {
    public let entries: [ConversationPlanEntry]
    public let source: String
    public var valid: Bool {
        ["native", "acp"].contains(source) && !entries.isEmpty && entries.allSatisfy {
            !$0.content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                && ["pending", "in_progress", "completed"].contains($0.status)
                && ["low", "medium", "high"].contains($0.priority)
        }
    }
}
public struct ConversationPlanEntry: Codable, Equatable, Hashable, Sendable {
    public let content: String
    public let status: String
    public let priority: String
}
