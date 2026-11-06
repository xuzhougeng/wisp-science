import Foundation

// Wire shapes are owned by wisp-dto::docx_review and checked against the same
// fixture in Rust and Swift. A selection always accompanies a document hash.
public enum DocxChangeKind: String, Codable, Sendable { case deletion, insertion, rewrite }
public enum DocxReviewAction: String, Sendable { case accept, reject }
public enum DocxReviewSelection: Equatable, Sendable {
    case change(String), kind(DocxChangeKind)
    public var value: SettingsValue {
        switch self {
        case .change(let id): return .object(["type": .string("change"), "value": .string(id)])
        case .kind(let kind): return .object(["type": .string("kind"), "value": .string(kind.rawValue)])
        }
    }
}
public struct DocxChange: Codable, Identifiable, Sendable {
    public let id: String
    public let paragraph_id: String
    public let kind: DocxChangeKind
    public let author: String
    public let date: String
    public let old: String
    public let new: String
}
public struct DocxReviewSnapshot: Codable, Sendable {
    public let source_sha256: String
    public let changes: [DocxChange]
    public let unsupported_count: Int
    public let writable: Bool
}
public struct NativeDocxReview: Codable, Sendable {
    public let schema: String
    public let project_id: String
    public let session_id: String?
    public let path: String
    public let review: DocxReviewSnapshot
    public let content: NativePanelFileContent

    public static func decode(_ value: SettingsValue, project: String, session: String?, path: String) throws -> Self {
        let result = try JSONDecoder().decode(Self.self, from: JSONEncoder().encode(value))
        guard result.schema == NativeFilesContract.schema, result.project_id == project,
              result.session_id == session, result.path == path, result.content.path == path,
              path.hasPrefix("/"), (path as NSString).pathExtension.lowercased() == "docx",
              result.content.text == nil, !result.content.truncated,
              let base64 = result.content.base64, base64.utf8.count <= 45 * 1024 * 1024,
              let bytes = Data(base64Encoded: base64), bytes.count <= 32 * 1024 * 1024,
              result.content.mime == "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
              result.review.source_sha256.count == 64,
              result.review.source_sha256.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }),
              result.review.unsupported_count >= 0, result.review.changes.count <= 1000,
              Set(result.review.changes.map(\.id)).count == result.review.changes.count,
              result.review.changes.allSatisfy({ !$0.id.isEmpty }),
              result.review.changes.reduce(0, { $0 + $1.old.utf8.count + $1.new.utf8.count }) <= 1024 * 1024 else {
            throw ProjectBrowserError.invalidResponse
        }
        return result
    }
}
