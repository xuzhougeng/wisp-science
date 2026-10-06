import Foundation

private func importDecode<T: Decodable>(_ value: SettingsValue, _ type: T.Type) throws -> T {
    try JSONDecoder().decode(type, from: JSONEncoder().encode(value))
}
private func importSourceValid(_ id: String) -> Bool {
    (id == "local" || (["ssh:", "wsl:"].contains { id.hasPrefix($0) } && id.count > 4)) && !id.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains)
}
private func importScope(_ schema: String, _ project: String, _ provider: String, _ context: String, project expectedProject: String, provider expectedProvider: String, context expectedContext: String) throws {
    guard schema == NativeSessionArchivePreview.schema, project == expectedProject, !project.isEmpty,
          provider == expectedProvider, ["codex", "claude"].contains(provider), context == expectedContext, importSourceValid(context) else { throw ProjectBrowserError.invalidResponse }
}

/// Existing wisp-dto::native_session_import source, list, preview and result shapes.
public struct NativeExternalImportSource: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let label: String
    public let kind: String
}
public struct NativeExternalImportSources: Codable, Equatable, Sendable {
    public let schema: String
    public let project_id: String
    public let sources: [NativeExternalImportSource]
    public static func decode(_ value: SettingsValue, project: String) throws -> Self {
        let result = try importDecode(value, Self.self)
        guard result.schema == NativeSessionArchivePreview.schema, result.project_id == project,
              result.sources.contains(where: { $0.id == "local" }), Set(result.sources.map(\.id)).count == result.sources.count,
              result.sources.allSatisfy({ source in
                  importSourceValid(source.id) && !source.label.isEmpty && ["local", "ssh", "wsl"].contains(source.kind)
                      && (source.kind == "local" ? source.id == "local" : source.id.hasPrefix(source.kind + ":"))
              }) else { throw ProjectBrowserError.invalidResponse }
        return result
    }
}
public struct NativeExternalImportItem: Codable, Equatable, Identifiable, Sendable {
    public var id: String { path }
    public let path: String
    public let session_id: String
    public let title: String
    public let cwd: String
    public var message_count: Int
    public let last_active_at: Int64
    public var state: String
}
public struct NativeExternalImportList: Codable, Equatable, Sendable {
    public let schema: String
    public let project_id: String
    public let provider: String
    public let context_id: String
    public let items: [NativeExternalImportItem]
    public static func decode(_ value: SettingsValue, project: String, provider: String, context: String) throws -> Self {
        let result = try importDecode(value, Self.self)
        try importScope(result.schema, result.project_id, result.provider, result.context_id, project: project, provider: provider, context: context)
        guard result.items.count <= 500, Set(result.items.map(\.path)).count == result.items.count,
              result.items.allSatisfy({ !$0.path.isEmpty && !$0.session_id.isEmpty && $0.message_count >= 0 && ["new", "imported", "updatable"].contains($0.state) }) else { throw ProjectBrowserError.invalidResponse }
        return result
    }
}
public struct NativeExternalImportPreview: Codable, Equatable, Sendable {
    public let schema: String
    public let project_id: String
    public let provider: String
    public let context_id: String
    public let path: String
    public let source_session_id: String
    public let sha256: String
    public let message_count: Int
    public let messages: [NativeSessionImportLine]
    public let existing_session_id: String?
    public var arguments: [String: SettingsValue] {
        ["provider": .string(provider), "context_id": .string(context_id), "path": .string(path), "source_session_id": .string(source_session_id), "sha256": .string(sha256)]
    }
    public static func decode(_ value: SettingsValue, project: String, provider: String, context: String, item: NativeExternalImportItem) throws -> Self {
        let result = try importDecode(value, Self.self)
        try importScope(result.schema, result.project_id, result.provider, result.context_id, project: project, provider: provider, context: context)
        guard result.path == item.path, result.source_session_id == item.session_id, result.message_count > 0,
              result.sha256.count == 64, result.sha256.allSatisfy({ "0123456789abcdefABCDEF".contains($0) }),
              result.messages.count <= 4, result.messages.allSatisfy({ ["user", "assistant"].contains($0.role) && $0.text.unicodeScalars.count <= 600 }),
              result.existing_session_id == nil || result.existing_session_id?.isEmpty == false else { throw ProjectBrowserError.invalidResponse }
        return result
    }
}
public struct NativeExternalImportResult: Codable, Equatable, Sendable {
    public let schema: String
    public let project_id: String
    public let provider: String
    public let context_id: String
    public let path: String
    public let source_session_id: String
    public let frame_id: String
    public let status: String
    public let message_count: Int
    public static func decode(_ value: SettingsValue, reviewed: NativeExternalImportPreview) throws -> Self {
        let result = try importDecode(value, Self.self)
        try importScope(result.schema, result.project_id, result.provider, result.context_id, project: reviewed.project_id, provider: reviewed.provider, context: reviewed.context_id)
        guard result.path == reviewed.path, result.source_session_id == reviewed.source_session_id, !result.frame_id.isEmpty,
              result.message_count == reviewed.message_count, ["imported", "updated", "skipped"].contains(result.status),
              reviewed.existing_session_id == nil || result.frame_id == reviewed.existing_session_id else { throw ProjectBrowserError.invalidResponse }
        return result
    }
}
