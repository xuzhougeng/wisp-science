import Foundation

public struct NativeSideChatEvidence: Codable, Equatable, Sendable {
    public let sourceId: String
    public let eventSeq: Int64?
    public let messageSeq: Int64?
    public let turn: Int
    public let role: String
    public let excerpt: String
    public let relevance: String
}
public struct NativeSideChatResponse: Codable, Equatable, Sendable {
    public let sessionId: String?
    public let answer: String
    public let snapshotVersion: Int64
    public let evidence: [NativeSideChatEvidence]
    public let noEvidence: Bool
}
public struct NativeSideChatOption: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let label: String
    public let kind: String
    public let active: Bool
    public var key: String { kind + ":" + id }
}
public struct NativeSideChatQuote: Equatable, Identifiable, Sendable {
    public let id = UUID()
    public let text: String
    public let source: String
    public init(text: String, source: String = "") { self.text = text; self.source = source }
    public static func question(_ text: String, quotes: [Self]) -> String {
        var parts: [String] = []
        for quote in quotes {
            let source = quote.source.replacingOccurrences(of: "\r", with: " ").replacingOccurrences(of: "\n", with: " ").replacingOccurrences(of: "`", with: "\\`").trimmingCharacters(in: .whitespacesAndNewlines)
            var block = source.isEmpty ? "" : "Selected excerpt from reference `\(source)`:\n"
            block += quote.text.trimmingCharacters(in: .whitespacesAndNewlines).components(separatedBy: "\n").map { "> " + $0 }.joined(separator: "\n")
            parts.append(block)
        }
        parts.append(text.trimmingCharacters(in: .whitespacesAndNewlines))
        return parts.joined(separator: "\n\n").trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

/// Called only for Return/Enter. A composition confirmation always belongs to the IME.
public enum NativeMessageReturnAction: String, Sendable {
    case send, newline, composition
    public static func resolve(shift: Bool, composing: Bool, sendWithModifier: Bool = false, modifier: Bool = false) -> Self {
        composing ? .composition : (shift || (sendWithModifier && !modifier)) ? .newline : .send
    }
}
