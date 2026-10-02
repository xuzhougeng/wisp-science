import Foundation

/// wisp-dto codex_login responses. Tokens and PKCE verifier never cross this boundary.
public struct NativeCodexLoginChallenge: Codable, Equatable, Sendable {
    public let login_id: String
    public let method: String
    public let url: String
    public let user_code: String
    public let verification_uri: String
    public let message: String
}

public struct NativeCodexLoginSnapshot: Codable, Equatable, Sendable {
    public let status: String
    public let message: String
    public let account_id: String
}

public struct NativeCodexSubscriptionStatus: Codable, Equatable, Sendable {
    public let signed_in: Bool
    public let account_id: String
}
