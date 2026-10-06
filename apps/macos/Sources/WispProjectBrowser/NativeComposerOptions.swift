import Foundation

/// Existing wisp-dto::native_conversations::ComposerOptions contract.
public struct NativeComposerSessionOptions: Codable, Equatable, Sendable {
    public let session_id: String
    public let full_permission: Bool
    public let delegation: Bool
    public let completion: NativeComposerCompletionSettings
    public let auto_review: Bool
    public let specialist: SettingsValue?
    public let specialist_locked: Bool
    public static func decode(_ value: SettingsValue, session: String) throws -> Self {
        let result = try JSONDecoder().decode(Self.self, from: JSONEncoder().encode(value))
        guard result.session_id == session, ["inline", "background"].contains(result.completion.policy) else { throw ProjectBrowserError.invalidResponse }
        return result
    }
}
public struct NativeComposerCompletionSettings: Codable, Equatable, Sendable {
    public let policy: String
    public let auto_resume: Bool
}
public struct NativeComposerSpecialist: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let name: String
}
