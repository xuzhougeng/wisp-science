import Foundation
import Combine
import WispRemote

enum ResearchDestination: Hashable {
    case journey(String, Date)
    case entry(String, RemoteResearchEntry)
}

struct ResearchQuery: Hashable {
    let project: String?
    let projectIDs: [String]
    let interval: DateInterval

    var command: String { project == nil ? "native_research_calendar" : "native_research_journey" }
    var args: [String: RemoteValue] {
        var args = RemoteResearchClock.args(interval)
        if project == nil { args["project_ids"] = .array(projectIDs.map(RemoteValue.string)) }
        return args
    }
}

/// Research reads stay in memory so a cached calendar cannot reveal a project
/// hidden later by the host. Each response belongs to its query and connection.
@MainActor final class ResearchLoader: ObservableObject {
    @Published private(set) var projects: [RemoteResearchProject] = []
    @Published private(set) var history: RemoteResearchHistory?
    @Published private(set) var busy = false
    @Published private(set) var error: String?
    private var generation = UUID()
    private let read: (RemoteConnection, ResearchQuery) async throws -> RemoteValue

    init(read: @escaping (RemoteConnection, ResearchQuery) async throws -> RemoteValue = { connection, query in
        try await connection.call(query.command, project: query.project, args: query.args)
    }) { self.read = read }

    func load(_ query: ResearchQuery, connection: RemoteConnection) async {
        let token = UUID(), session = connection.readSessionID
        generation = token
        projects = []; history = nil; error = nil; busy = true
        defer { if generation == token { busy = false } }
        guard connection.state == .online else { error = "请连接 Wisp 主机后查看研究记录。"; return }
        guard connection.supports(query.command) else { error = "当前主机尚未提供手机端研究日历，请更新 Wisp 主机后重连。"; return }
        do {
            if query.project != nil {
                let value = try await read(connection, query)
                try Task.checkCancellation()
                guard generation == token, connection.readSessionID == session, connection.state == .online else { return }
                history = try RemoteResearchHistory(value)
            } else {
                var rows: [RemoteResearchProject] = []
                // The tunnel accepts at most 100 projects per request. Keep
                // large workspaces bounded without silently omitting projects.
                for start in stride(from: 0, to: query.projectIDs.count, by: 50) {
                    let ids = Array(query.projectIDs[start..<min(start + 50, query.projectIDs.count)])
                    let batch = ResearchQuery(project: nil, projectIDs: ids, interval: query.interval)
                    let value = try await read(connection, batch)
                    try Task.checkCancellation()
                    guard generation == token, connection.readSessionID == session, connection.state == .online else { return }
                    rows += try RemoteResearchProject.decode(value, requested: Set(ids))
                }
                projects = rows
            }
        } catch {
            guard generation == token, !Task.isCancelled, connection.readSessionID == session else { return }
            self.error = error.localizedDescription
        }
    }
}
