import Foundation
import Combine
import WispRemote

/// A display projection only: grouping must not rewrite dates, drop conflicts,
/// or change the replica's visibility rules.
struct ChecklistSections {
    let needsReview: [ChecklistEntry]
    let overdue: [ChecklistEntry]
    let today: [ChecklistEntry]
    let upcoming: [ChecklistEntry]
    let done: [ChecklistEntry]
    var remainingCount: Int { needsReview.count + overdue.count + today.count + upcoming.count }

    init(entries: [ChecklistEntry], today day: String) {
        let sorted = entries.sorted { ($0.fields.day, $0.id) < ($1.fields.day, $1.id) }
        needsReview = sorted.filter(\.conflict)
        let open = sorted.filter { !$0.conflict && $0.fields.status == "open" }
        overdue = open.filter { $0.fields.day < day }
        today = open.filter { $0.fields.day == day }
        upcoming = open.filter { $0.fields.day > day }
        done = sorted.filter { !$0.conflict && $0.fields.status == "done" }
    }
}

@MainActor final class ChecklistModel: ObservableObject {
    private struct Document: Codable {
        var version = 1
        var selected = "local"
        var replicas: [String: ChecklistReplica] = ["local": ChecklistReplica()]
        var names: [String: String] = ["local": "本机清单"]
    }
    @Published private var document = Document()
    @Published private(set) var loaded = false
    @Published private(set) var syncing = false
    @Published var error: String?
    @Published private(set) var lastSync: Date?
    private let file: URL
    private var connectedScope: String?
    private var connectionID: UUID?
    var scope: String { document.selected }
    var scopeName: String { document.names[scope] ?? "已保存的清单" }
    var scopes: [(String, String)] { document.names.sorted { $0.key < $1.key }.map { ($0.key, $0.value) } }
    var replica: ChecklistReplica { document.replicas[scope] ?? ChecklistReplica() }
    var entries: [ChecklistEntry] {
        replica.entries.values.filter { replica.visible($0) && ($0.fields.status != "dropped" || $0.conflict) }
            .sorted { ($0.fields.day, $0.id) < ($1.fields.day, $1.id) }
    }
    var pendingCount: Int { replica.entries.values.filter { $0.mutationID != nil }.count }
    var canSyncSelection: Bool { connectedScope == scope }

    init(directory: URL = OfflineResearchStorage.directory) {
        file = directory.appendingPathComponent("checklists.json")
        do {
            if let saved = try OfflineResearchStorage.load(Document.self, from: file) {
                guard saved.version == 1, saved.replicas[saved.selected] != nil, saved.replicas["local"] != nil else { throw RemoteError.incompatibleResponse }
                document = saved
            }
            loaded = true
        } catch { self.error = "无法读取本机清单，原文件已保留。解锁设备后重开 App。\n" + error.localizedDescription }
    }

    private func save(_ next: Document) throws {
        guard loaded else { throw RemoteError.service("本机清单尚未成功读取，无法覆盖保存。") }
        try OfflineResearchStorage.save(next, to: file)
        document = next
    }
    func select(_ scope: String) {
        guard document.replicas[scope] != nil else { return }
        var next = document; next.selected = scope
        do { try save(next) } catch { self.error = error.localizedDescription }
    }
    @discardableResult func edit(id: String? = nil, fields: ChecklistFields) -> Bool {
        var next = document
        do {
            var replica = next.replicas[scope] ?? ChecklistReplica()
            try replica.edit(id: id, fields: fields)
            next.replicas[scope] = replica
            try save(next); error = nil; return true
        } catch { self.error = error.localizedDescription; return false }
    }
    func resolve(_ id: String, keepLocal: Bool) {
        var next = document
        next.replicas[scope]?.resolve(id: id, keepLocal: keepLocal)
        do { try save(next); error = nil } catch { self.error = error.localizedDescription }
    }

    func configure(configuration: RemoteConfiguration?, connection: RemoteConnection, demo: Bool) {
        connectionID = connection.readSessionID
        connectedScope = demo ? nil : configuration.map(RemoteReadCache.scope)
        guard loaded, let scope = connectedScope else { return }
        var next = document
        if next.replicas[scope] == nil { next.replicas[scope] = ChecklistReplica() }
        next.names[scope] = connection.hostName.isEmpty ? (configuration?.relay.host ?? "Wisp 主机") : connection.hostName
        if connection.state == .online {
            let local = next.replicas["local"] ?? ChecklistReplica()
            next.replicas[scope]?.adopt(local)
            next.replicas["local"] = ChecklistReplica()
        }
        // Repeated polling does not change a user's selected offline list.
        if document.replicas[scope] == nil || document.selected == "local" { next.selected = scope }
        if next.names != document.names || next.selected != document.selected || !((document.replicas["local"]?.entries.isEmpty) ?? true) || document.replicas[scope] == nil {
            do { try save(next) } catch { self.error = error.localizedDescription }
        }
    }

    func sync(connection: RemoteConnection) async {
        guard loaded, !syncing, let scope = connectedScope, connection.state == .online,
              connectionID == connection.readSessionID else { return }
        guard connection.supports("native_research_checklist_sync") else {
            error = "当前主机不支持清单同步；本机仍可编辑，更新主机后自动同步。"; return
        }
        let session = connection.readSessionID
        syncing = true
        defer { syncing = false }
        do {
            if let local = document.replicas["local"], !local.entries.isEmpty {
                var next = document
                next.replicas[scope, default: ChecklistReplica()].adopt(local)
                next.replicas["local"] = ChecklistReplica()
                if next.selected == "local" { next.selected = scope }
                try save(next)
            }
            for _ in 0..<100 {
                try Task.checkCancellation()
                let args = (document.replicas[scope] ?? ChecklistReplica()).request
                let value = try await connection.call("native_research_checklist_sync", args: args)
                guard connectionID == session, connection.readSessionID == session, connectedScope == scope else { return }
                let reply = try ChecklistReply(value)
                var next = document
                var replica = next.replicas[scope] ?? ChecklistReplica()
                try replica.merge(reply)
                next.replicas[scope] = replica
                try save(next)
                if !reply.has_more && replica.pendingCount == 0 {
                    lastSync = Date(); error = nil; return
                }
            }
            error = "清单较多，本次已保存同步进度，稍后继续。"
        } catch {
            guard connectedScope == scope, connection.readSessionID == session, !Task.isCancelled else { return }
            self.error = "更改已保存在手机，尚未确认同步。\n" + error.localizedDescription
        }
    }
}
