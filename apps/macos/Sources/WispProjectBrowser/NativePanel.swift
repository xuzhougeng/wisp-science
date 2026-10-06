import Foundation

public enum NativePanelFileAction: String, Codable, Sendable {
    case createFile = "create_file", createDirectory = "create_directory", rename, delete
    public static func destination(directory: String, name: String) throws -> String {
        guard !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              name != ".", name != "..", !name.contains("/"), !name.contains("\\"),
              !name.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else {
            throw ProjectBrowserError.unavailable("请输入有效名称，不含路径分隔符或控制字符。")
        }
        return directory == "." ? name : directory + "/" + name
    }
}

/// Existing ArtifactInfo, DirEntry and FileContent contracts from wisp-dto.
public struct NativePanelArtifact: Codable, Identifiable, Sendable {
    public let id: String
    public let name: String
    public let kind: String
    public let path: String
    public let location: String?
    public let ts: Int64
    public let logical_path: String?
}
public struct NativePanelFile: Codable, Identifiable, Sendable {
    public var id: String { name }
    public let name: String
    public let is_dir: Bool
    public let size: UInt64
    public let modified_unix_millis: UInt64?
}
public struct NativePanelFileContent: Codable, Sendable {
    public let path: String
    public let mime: String
    public let text: String?
    public let base64: String?
    public let truncated: Bool
    public let total_bytes: UInt64?
}

public struct NativePanelContext: Codable, Identifiable, Sendable {
    public let id: String
    public let kind: String
    public let label: String
    public let config_json: String
    public let capabilities_json: String
    public let last_probe_status: String?
    public let last_probe_error: String?
}
public struct NativePanelContexts: Codable, Sendable {
    public let contexts: [NativePanelContext]
    public let enabled_ids: [String]
    public let read_only: Bool
    public let default_context: NativePanelDefaultContext?
    public var attached: [NativePanelContext] { contexts.filter { $0.kind == "local" || enabled_ids.contains($0.id) } }
    public var available: [NativePanelContext] { contexts.filter { $0.kind != "local" && !enabled_ids.contains($0.id) } }
}

public struct NativePanelDefaultContext: Codable, Sendable {
    public let context_id: String?
}
public struct NativePanelSearchHit: Codable, Identifiable, Sendable {
    public var id: String { path }
    public let path: String
    public let name: String
    public let is_dir: Bool
    public let size: UInt64
}
public struct NativePanelExport: Codable, Sendable {
    public let path: String
    public let name: String
    public let total_bytes: UInt64
}
