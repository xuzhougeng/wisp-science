import Foundation

/// The existing wisp-dto::AutoFailureAnalysisSettings command shape.
public struct NativeFailureAnalysisSettings: Codable, Equatable, Sendable {
    public var enabled: Bool
    public var failure_rate_threshold: Int
    public var minimum_failures: Int
    public static func decode(_ value: SettingsValue) throws -> Self {
        let result = try JSONDecoder().decode(Self.self, from: JSONEncoder().encode(value))
        guard (1...100).contains(result.failure_rate_threshold), (1...100).contains(result.minimum_failures) else { throw ProjectBrowserError.invalidResponse }
        return result
    }
    public var value: SettingsValue {
        .object(["enabled": .bool(enabled), "failure_rate_threshold": .integer(Int64(failure_rate_threshold)), "minimum_failures": .integer(Int64(minimum_failures))])
    }
}

/// Decode only the preference from wisp-dto::MemoryView; memory contents stay out
/// of the composer. The preference is global, while the view has a project owner.
public struct NativeComposerMemoryPreference: Decodable, Equatable, Sendable {
    public let enabled: Bool
    public let project_id: String
    public static func decode(_ value: SettingsValue, project: String) throws -> Self {
        let result = try JSONDecoder().decode(Self.self, from: JSONEncoder().encode(value))
        guard result.project_id == project else { throw ProjectBrowserError.invalidResponse }
        return result
    }
}

public struct NativeReviewerChoice: Equatable, Identifiable, Sendable {
    public let id: String
    public let label: String
    public init(id: String, label: String) { self.id = id; self.label = label }
}

public enum NativeReviewerConfiguration {
    /// Same tagged ReviewBackendConfig and legacy model_id as wisp-dto.
    public static func key(_ reviewer: SettingsValue) throws -> String {
        guard reviewer["id"] == .string("reviewer") else { throw ProjectBrowserError.invalidResponse }
        let backend = reviewer["review_backend"]
        if backend == .null {
            guard reviewer["model_id"] == .null || { if case .string = reviewer["model_id"] { return true }; return false }() else { throw ProjectBrowserError.invalidResponse }
            return "http:" + reviewer["model_id"].string
        }
        switch backend["kind"] {
        case .string("follow_session"): return "follow_session"
        case .string("http_model"):
            guard backend["profile_id"] == .null || { if case .string = backend["profile_id"] { return true }; return false }() else { throw ProjectBrowserError.invalidResponse }
            return "http:" + backend["profile_id"].string
        case .string("acp_agent"):
            guard case .string(let id) = backend["profile_id"], !id.isEmpty else { throw ProjectBrowserError.invalidResponse }
            return "acp:" + id
        default: throw ProjectBrowserError.invalidResponse
        }
    }
    /// Patch the full advertised Specialist, preserving instructions, skills,
    /// connectors and other fields instead of reconstructing a partial persona.
    public static func changing(_ reviewer: SettingsValue, to key: String) throws -> SettingsValue {
        _ = try self.key(reviewer)
        var result = reviewer
        if key == "follow_session" { result["review_backend"] = .object(["kind": .string("follow_session")]) }
        else if key.hasPrefix("acp:"), key.count > 4 {
            result["review_backend"] = .object(["kind": .string("acp_agent"), "profile_id": .string(String(key.dropFirst(4)))])
        } else if key.hasPrefix("http:") {
            let id = String(key.dropFirst(5)); result["model_id"] = .string(id)
            result["review_backend"] = .object(["kind": .string("http_model"), "profile_id": .string(id)])
        } else { throw ProjectBrowserError.invalidResponse }
        return result
    }
    public static func catalog(_ value: SettingsValue) throws -> [SettingsValue] {
        guard case .array(let rows) = value else { throw ProjectBrowserError.invalidResponse }
        guard rows.allSatisfy({ row in
            guard case .string(let id) = row["id"], !id.isEmpty, case .string(let name) = row["name"], !name.isEmpty else { return false }
            return true
        }), Set(rows.map { $0["id"].string }).count == rows.count else { throw ProjectBrowserError.invalidResponse }
        return rows
    }
    public static func choices(models: SettingsValue, agents: SettingsValue, defaultLabel: String, followLabel: String) throws -> [NativeReviewerChoice] {
        func profiles(_ value: SettingsValue) throws -> [SettingsValue] {
            guard case .array(let rows) = value, rows.allSatisfy({ row in
                guard case .string(let id) = row["id"], !id.isEmpty, case .string = row["label"] else { return false }; return true
            }), Set(rows.map { $0["id"].string }).count == rows.count else { throw ProjectBrowserError.invalidResponse }
            return rows
        }
        let models = try profiles(models), agents = try profiles(agents)
        var result = [NativeReviewerChoice(id: "http:", label: defaultLabel), NativeReviewerChoice(id: "follow_session", label: followLabel)]
        for row in models {
            for field in ["image_generation_capable", "use_for_image_generation"] {
                guard row[field] == .null || { if case .bool = row[field] { return true }; return false }() else { throw ProjectBrowserError.invalidResponse }
            }
            // Match wisp-dto::ModelProfile::is_chat_model, including exact gateway
            // tail IDs. A known media model never absorbs a longer sibling ID.
            let tail = row["model"].string.trimmingCharacters(in: .whitespacesAndNewlines).split(separator: "/").last.map(String.init)?.lowercased() ?? ""
            guard !row["image_generation_capable"].bool, !row["use_for_image_generation"].bool,
                  !["gpt-image-2", "grok-imagine-image-2.0", "grok-imagine-video", "grok-imagine-video-1.5", "grok-imagine-video-1.5-preview"].contains(tail) else { continue }
            result.append(NativeReviewerChoice(id: "http:" + row["id"].string, label: row["label"].string.isEmpty ? row["id"].string : row["label"].string))
        }
        result += agents.map { NativeReviewerChoice(id: "acp:" + $0["id"].string, label: $0["label"].string + " · ACP") }
        return result
    }
}
