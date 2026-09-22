import Foundation

public struct ProjectSummary: Codable, Identifiable, Equatable, Sendable {
    public let id: String
    public let name: String
    public let description: String
    public let workspaceDirectory: String
    public let starred: Bool
    public let sessionCount: Int64
    public let artifactCount: Int64
    public let updatedAt: Int64
    public let runningCount: Int64
    public let needsYouCount: Int64
    public let syncConfigured: Bool
    public let lastSyncedAt: Int64?

    enum CodingKeys: String, CodingKey {
        case id, name, description, starred
        case workspaceDirectory = "workspace_dir"
        case sessionCount = "session_count"
        case artifactCount = "artifact_count"
        case updatedAt = "updated_at"
        case runningCount = "running_count"
        case needsYouCount = "needs_you_count"
        case syncConfigured = "sync_configured"
        case lastSyncedAt = "last_synced_at"
    }
}

public struct BrowserMessage: Decodable, Identifiable, Sendable {
    public var id: Int64 { seq }
    public let seq: Int64
    public let role: String
    public let text: String
    public let toolName: String?
    enum CodingKeys: String, CodingKey { case seq, role, text; case toolName = "tool_name" }
}

public struct TranscriptPage: Sendable {
    public let messages: [BrowserMessage]
    public let nextBeforeSeq: Int64?
    public init(messages: [BrowserMessage], nextBeforeSeq: Int64?) { self.messages = messages; self.nextBeforeSeq = nextBeforeSeq }
}

public struct BrowserSession: Codable, Identifiable, Equatable, Sendable {
    public let id: String
    public let projectID: String
    public let title: String
    public let ts: Int64
    public let status: String
    public let folderID: String?
    public init(id: String, projectID: String, title: String, ts: Int64, status: String, folderID: String? = nil) {
        self.id = id; self.projectID = projectID; self.title = title; self.ts = ts; self.status = status; self.folderID = folderID
    }
    enum CodingKeys: String, CodingKey {
        case id, title, ts, status
        case projectID = "project_id"
        case folderID = "folder_id"
    }
}

public struct ProjectListSnapshot: Sendable {
    public let projects: [ProjectSummary]
    public let activitySource: String
    public init(projects: [ProjectSummary], activitySource: String) { self.projects = projects; self.activitySource = activitySource }
}

public enum ProjectBrowserError: LocalizedError {
    case unavailable(String)
    case service(String)
    case invalidResponse

    public var errorDescription: String? {
        switch self {
        case .unavailable(let message), .service(let message): return message
        case .invalidResponse: return "查询服务返回了不兼容的数据，请重新构建原生预览版。"
        }
    }
}

/// Matches `wisp-dto::project_browser`; the shared JSON fixture tests this boundary.
public protocol ProjectBrowserQuerying: Sendable {
    func listProjects(databaseURL: URL) async throws -> ProjectListSnapshot
    func listSessions(databaseURL: URL, projectID: String?) async throws -> [BrowserSession]
    func transcript(databaseURL: URL, projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> TranscriptPage
}

public protocol ProjectBrowserWriting: Sendable {
    func setProjectStarred(databaseURL: URL, projectID: String, starred: Bool) async throws -> ProjectListSnapshot
}

public struct ProjectBrowserClient: ProjectBrowserQuerying, ProjectBrowserWriting {
    public static let schema = "wisp.project-browser.v1"
    public let executableURL: URL

    public init(executableURL: URL) {
        self.executableURL = executableURL
    }

    public func listProjects(databaseURL: URL) async throws -> ProjectListSnapshot {
        // One short-lived process per refresh: closing stdin ends the service.
        // Blocking pipe reads run off the UI thread and the process has a deadline.
        try await Task.detached(priority: .userInitiated) {
            try Self.decode(query(databaseURL: databaseURL, command: ["type": "list_projects"]), requestID: "projects-1")
        }.value
    }

    public func setProjectStarred(databaseURL: URL, projectID: String, starred: Bool) async throws -> ProjectListSnapshot {
        try await Task.detached(priority: .userInitiated) {
            try Self.decode(query(databaseURL: databaseURL,
                command: ["type": "set_project_starred", "project_id": projectID], starred: starred), requestID: "projects-1")
        }.value
    }

    public func listSessions(databaseURL: URL, projectID: String? = nil) async throws -> [BrowserSession] {
        try await Task.detached(priority: .userInitiated) {
            var command = ["type": "list_sessions"]
            if let projectID { command["project_id"] = projectID }
            return try Self.decodeSessions(query(databaseURL: databaseURL, command: command), requestID: "projects-1")
        }.value
    }

    static func decodeSessions(_ data: Data, requestID: String) throws -> [BrowserSession] {
        let response = try JSONDecoder().decode(Response.self, from: data)
        guard response.schema == schema, response.id == requestID else { throw ProjectBrowserError.invalidResponse }
        if response.type == "error" { throw ProjectBrowserError.service(response.message ?? "会话查询失败。") }
        guard response.type == "sessions", response.activitySource == "persisted_only",
              let sessions = response.sessions else { throw ProjectBrowserError.invalidResponse }
        return sessions
    }

    public func transcript(databaseURL: URL, projectID: String, sessionID: String, beforeSeq: Int64? = nil) async throws -> TranscriptPage {
        try await Task.detached(priority: .userInitiated) {
            let data = try query(databaseURL: databaseURL, command: ["type": "get_transcript", "project_id": projectID, "session_id": sessionID], beforeSeq: beforeSeq)
            return try Self.decodeTranscript(data, requestID: "projects-1")
        }.value
    }

    static func decodeTranscript(_ data: Data, requestID: String) throws -> TranscriptPage {
        let response = try JSONDecoder().decode(Response.self, from: data)
        guard response.schema == schema, response.id == requestID else { throw ProjectBrowserError.invalidResponse }
        if response.type == "error" { throw ProjectBrowserError.service(response.message ?? "会话读取失败。") }
        guard response.type == "transcript", let messages = response.messages else { throw ProjectBrowserError.invalidResponse }
        return TranscriptPage(messages: messages, nextBeforeSeq: response.nextBeforeSeq)
    }

    private func query(databaseURL: URL, command: [String: String], beforeSeq: Int64? = nil, starred: Bool? = nil) throws -> Data {
        guard FileManager.default.isExecutableFile(atPath: executableURL.path) else {
            throw ProjectBrowserError.unavailable("找不到查询服务。请使用 scripts/build_native_macos.sh 构建应用。")
        }
        let process = Process()
        let input = Pipe()
        let output = Pipe()
        let errors = Pipe()
        process.executableURL = executableURL
        process.arguments = ["--database", databaseURL.path] + (starred == nil ? [] : ["--allow-project-writes"])
        process.standardInput = input
        process.standardOutput = output
        process.standardError = errors
        // Write the small request before launch so an immediate startup failure
        // cannot race this write with a closed pipe.
        let requestID = "projects-1"
        var request: [String: Any] = command.merging(["schema": Self.schema, "id": requestID]) { _, value in value }
        if let beforeSeq { request["before_seq"] = beforeSeq }
        if let starred { request["starred"] = starred }
        var data = try JSONSerialization.data(withJSONObject: request)
        data.append(0x0A)
        try input.fileHandleForWriting.write(contentsOf: data)
        try input.fileHandleForWriting.close()
        try process.run()
        let deadline = DispatchSource.makeTimerSource(queue: .global())
        deadline.schedule(deadline: .now() + 30)
        deadline.setEventHandler {
            if process.isRunning { process.terminate() }
        }
        deadline.resume()
        defer {
            deadline.cancel()
            if process.isRunning { process.terminate() }
        }
        let reply = output.fileHandleForReading.readDataToEndOfFile()
        let diagnostics = errors.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            let message = String(decoding: diagnostics, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
            throw ProjectBrowserError.service(message.isEmpty ? "查询服务已停止或超时，请重试。" : message)
        }
        return reply
    }

    static func decode(_ data: Data, requestID: String) throws -> ProjectListSnapshot {
        let response: Response
        do { response = try JSONDecoder().decode(Response.self, from: data) }
        catch { throw ProjectBrowserError.invalidResponse }
        guard response.schema == schema, response.id == requestID else {
            throw ProjectBrowserError.invalidResponse
        }
        if response.type == "error" {
            throw ProjectBrowserError.service(response.message ?? "项目查询失败。")
        }
        guard response.type == "projects", let projects = response.projects,
              response.activitySource == "persisted_only" else {
            throw ProjectBrowserError.invalidResponse
        }
        return ProjectListSnapshot(projects: projects, activitySource: "persisted_only")
    }

    private struct Response: Decodable {
        let schema: String
        let id: String?
        let type: String
        let projects: [ProjectSummary]?
        let sessions: [BrowserSession]?
        let messages: [BrowserMessage]?
        let nextBeforeSeq: Int64?
        let activitySource: String?
        let message: String?

        enum CodingKeys: String, CodingKey {
            case schema, id, type, projects, sessions, message, messages
            case nextBeforeSeq = "next_before_seq"
            case activitySource = "activity_source"
        }
    }
}
