import Foundation

/// Envelopes owned by wisp-dto::native_files; shared WebView payloads inside.
public enum NativeFilesContract {
    public static let schema = "wisp.native-files.v1"
    static func decode<T: Decodable>(_ value: SettingsValue, as type: T.Type) throws -> T {
        try JSONDecoder().decode(type, from: JSONEncoder().encode(value))
    }
    static func owner(_ schema: String, _ project: String, _ session: String, projectID: String, sessionID: String) throws {
        guard schema == Self.schema, project == projectID, session == sessionID else { throw ProjectBrowserError.invalidResponse }
    }
    public static func relative(_ path: String, rootAllowed: Bool = false) -> Bool {
        if rootAllowed && path == "." { return true }
        return !path.isEmpty && !path.hasPrefix("/") && !path.contains("://") && !path.contains("\0")
            && !path.split(separator: "/", omittingEmptySubsequences: false).contains(where: { $0.isEmpty || $0 == "." || $0 == ".." })
    }
    public static func remote(_ path: String, homeAllowed: Bool = true) -> Bool {
        !path.contains("\0") && (path.hasPrefix("/") || homeAllowed && (path == "~" || path.hasPrefix("~/")))
    }
    public static func inside(_ path: String, root: String) -> Bool {
        guard absolute(root), path.hasPrefix("/"), !path.contains("\0"), !path.contains("://") else { return false }
        return path.hasPrefix(root == "/" ? "/" : root + "/")
            && relative(String(path.dropFirst(root == "/" ? 1 : root.count + 1)))
    }
    static func absolute(_ path: String) -> Bool { path == "/" || path.hasPrefix("/") && relative(String(path.dropFirst())) }
}

public struct NativeFileLocation: Codable, Identifiable, Sendable {
    public var id: String { context_id }
    public let context_id: String
    public let label: String
    public let kind: String
}
public struct NativeFileLocations: Codable, Sendable {
    public let schema: String
    public let project_id: String
    public let session_id: String
    public let local_root: String
    public let read_only: Bool
    public let locations: [NativeFileLocation]
    public static func decode(_ value: SettingsValue, project: String, session: String) throws -> Self {
        let page = try NativeFilesContract.decode(value, as: Self.self)
        try NativeFilesContract.owner(page.schema, page.project_id, page.session_id, projectID: project, sessionID: session)
        guard NativeFilesContract.absolute(page.local_root),
              page.locations.first?.context_id == "local", page.locations.first?.kind == "local",
              Set(page.locations.map(\.id)).count == page.locations.count,
              page.locations.dropFirst().allSatisfy({ $0.kind == "ssh" && $0.id.hasPrefix("ssh:") && $0.id.count > 4 && !$0.id.contains(where: { $0.isWhitespace || $0.isNewline }) }) else {
            throw ProjectBrowserError.invalidResponse
        }
        return page
    }
}
public struct NativeFileDirectory: Codable, Sendable {
    public let schema: String
    public let project_id: String
    public let session_id: String
    public let context_id: String
    public let path: String
    public let entries: [NativePanelFile]
    public static func decode(_ value: SettingsValue, project: String, session: String, context: String) throws -> Self {
        let page = try NativeFilesContract.decode(value, as: Self.self)
        try NativeFilesContract.owner(page.schema, page.project_id, page.session_id, projectID: project, sessionID: session)
        guard page.context_id == context,
              (context == "local" ? NativeFilesContract.relative(page.path, rootAllowed: true) : NativeFilesContract.remote(page.path, homeAllowed: false)),
              Set(page.entries.map(\.name)).count == page.entries.count,
              page.entries.allSatisfy({ !$0.name.isEmpty && $0.name != "." && $0.name != ".." && !$0.name.contains("/") && !$0.name.contains("\0") }) else {
            throw ProjectBrowserError.invalidResponse
        }
        return page
    }
}
public struct NativeFilePathPair: Codable, Sendable {
    public let requested_path: String
    public let relative_path: String
    public let absolute_path: String
}
public struct NativeFilePaths: Codable, Sendable {
    public let schema: String
    public let project_id: String
    public let session_id: String
    public let paths: [NativeFilePathPair]
    public static func decode(_ value: SettingsValue, project: String, session: String, requested: [String], root: String) throws -> Self {
        let page = try NativeFilesContract.decode(value, as: Self.self)
        try NativeFilesContract.owner(page.schema, page.project_id, page.session_id, projectID: project, sessionID: session)
        guard !requested.isEmpty, page.paths.map(\.requested_path) == requested,
              Set(requested).count == requested.count,
              page.paths.allSatisfy({ NativeFilesContract.relative($0.relative_path) && NativeFilesContract.inside($0.absolute_path, root: root)
                  && $0.absolute_path == (root == "/" ? root : root + "/") + $0.relative_path }) else { throw ProjectBrowserError.invalidResponse }
        return page
    }
}
public struct NativeFilePreview: Codable, Sendable {
    public let schema: String
    public let project_id: String
    public let session_id: String
    public let context_id: String
    public let requested_path: String
    public let content: NativePanelFileContent
    public static func decode(_ value: SettingsValue, project: String, session: String, context: String, requested: String, root: String) throws -> Self {
        let page = try NativeFilesContract.decode(value, as: Self.self)
        try NativeFilesContract.owner(page.schema, page.project_id, page.session_id, projectID: project, sessionID: session)
        guard page.context_id == context, page.requested_path == requested, !page.content.mime.isEmpty,
              (page.content.text != nil) != (page.content.base64 != nil),
              (context == "local" ? NativeFilesContract.inside(page.content.path, root: root)
                  : page.content.path == "ssh://" + String(context.dropFirst(4)) + "/" + requested.drop(while: { $0 == "/" })) else {
            throw ProjectBrowserError.invalidResponse
        }
        return page
    }
}

public enum NativeFileSort: String, CaseIterable, Sendable {
    case name, size, modified
    public func sorted(_ files: [NativePanelFile]) -> [NativePanelFile] {
        files.sorted { a, b in
            if a.is_dir != b.is_dir { return a.is_dir }
            if self == .size && a.size != b.size { return a.size > b.size }
            if self == .modified && (a.modified_unix_millis ?? 0) != (b.modified_unix_millis ?? 0) { return (a.modified_unix_millis ?? 0) > (b.modified_unix_millis ?? 0) }
            return a.name.lowercased() < b.name.lowercased()
        }
    }
}
