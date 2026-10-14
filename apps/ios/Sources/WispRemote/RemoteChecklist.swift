import Foundation

public struct ChecklistFields: Codable, Equatable, Sendable {
    public var title: String
    public var day: String
    public var status: String
    public init(title: String, day: String, status: String = "open") { self.title = title; self.day = day; self.status = status }
    public var isValid: Bool {
        let calendar = RemoteResearchClock.localCalendar(timeZone: TimeZone(secondsFromGMT: 0)!)
        let formatter = DateFormatter(); formatter.calendar = calendar; formatter.timeZone = calendar.timeZone
        formatter.locale = Locale(identifier: "en_US_POSIX"); formatter.dateFormat = "yyyy-MM-dd"; formatter.isLenient = false
        return !title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && title.unicodeScalars.count <= 1000
            && ["open", "done", "dropped"].contains(status)
            && formatter.date(from: day).map { formatter.string(from: $0) == day } == true
    }
}

/// Codable local projection of research_checklist::Item; contract tests use
/// the same JSON fixture as Rust.
public struct ChecklistServerItem: Codable, Equatable, Sendable {
    public let id: String
    public let title: String
    public let day: String
    public let status: String
    public let project_id: String?
    public let session_id: String?
    public let revision: Int64
    public var fields: ChecklistFields { ChecklistFields(title: title, day: day, status: status) }
}

public struct ChecklistReply: Decodable, Sendable {
    public let schema: String
    public let epoch: String
    public let cursor: Int64
    public let visibility: String
    public let visible_project_ids: [String]
    public let reset: Bool
    public let has_more: Bool
    public let items: [ChecklistServerItem]
    public let outcomes: [Outcome]
    public struct Outcome: Decodable, Sendable {
        public let mutation_id: String
        public let id: String
        public let status: String
        public let item: ChecklistServerItem?
    }
    public init(_ value: RemoteValue) throws {
        self = try JSONDecoder().decode(Self.self, from: JSONEncoder().encode(value))
        guard schema == "wisp.research-checklist.v1", !epoch.isEmpty, cursor >= 0,
              items.allSatisfy({ !$0.id.isEmpty && $0.revision > 0 && ["open", "done", "dropped"].contains($0.status) }),
              outcomes.allSatisfy({ ["applied", "conflict", "unavailable"].contains($0.status)
                  && ($0.item == nil || ($0.item!.id == $0.id && $0.item!.revision > 0 && ["open", "done", "dropped"].contains($0.item!.status)))
                  && ($0.status != "applied" || $0.item != nil) }) else { throw RemoteError.incompatibleResponse }
    }
}

public struct ChecklistEntry: Codable, Identifiable, Sendable {
    public let id: String
    public var fields: ChecklistFields
    public var server: ChecklistServerItem?
    public var mutationID: String?
    public var baseRevision: Int64
    public var conflict: Bool
    public var unavailable: Bool
    /// Keep the last project association even if a database reset invalidates
    /// the server revision. Hidden cached items must not become unassigned.
    public var projectID: String? = nil
}

public struct ChecklistReplica: Codable, Sendable {
    public private(set) var epoch: String?
    public private(set) var cursor: Int64 = 0
    public private(set) var visibility: String?
    public private(set) var visibleProjects: Set<String> = []
    public private(set) var entries: [String: ChecklistEntry] = [:]
    public init() {}
    public var pendingCount: Int { entries.values.filter { $0.mutationID != nil && !$0.conflict && !$0.unavailable }.count }
    public func visible(_ entry: ChecklistEntry) -> Bool {
        !entry.unavailable && ((entry.server?.project_id ?? entry.projectID).map { visibleProjects.contains($0) } ?? true)
    }

    public mutating func edit(id: String? = nil, fields: ChecklistFields) throws {
        guard fields.isValid else { throw RemoteError.service("请填写 1–1000 字的标题及有效日期。") }
        let id = id ?? UUID().uuidString.lowercased()
        var entry = entries[id] ?? ChecklistEntry(id: id, fields: fields, server: nil, mutationID: nil, baseRevision: 0, conflict: false, unavailable: false)
        guard !entry.conflict && !entry.unavailable else { throw RemoteError.service("请先处理这项清单的同步冲突。") }
        entry.fields = fields; entry.mutationID = UUID().uuidString.lowercased()
        entries[id] = entry
    }

    public mutating func resolve(id: String, keepLocal: Bool) {
        guard var entry = entries[id], entry.conflict else { return }
        entry.conflict = false
        if keepLocal {
            entry.baseRevision = entry.server?.revision ?? 0
            entry.mutationID = UUID().uuidString.lowercased()
        } else if let server = entry.server {
            entry.fields = server.fields; entry.baseRevision = server.revision; entry.mutationID = nil
        } else { entries.removeValue(forKey: id); return }
        entries[id] = entry
    }

    /// Fresh unbound phone items move to exactly one host on the first verified
    /// connection. Previously paired host replicas are never mixed together.
    public mutating func adopt(_ unbound: ChecklistReplica) {
        for (id, entry) in unbound.entries where entries[id] == nil { entries[id] = entry }
    }

    public var request: [String: RemoteValue] {
        let changes: [RemoteValue] = entries.values.filter { $0.mutationID != nil && !$0.conflict && !$0.unavailable }
            .sorted { $0.id < $1.id }.prefix(50).map { entry in
                .object(["id": .string(entry.id), "mutation_id": .string(entry.mutationID!),
                    "base_revision": .integer(entry.baseRevision), "title": .string(entry.fields.title),
                    "day": .string(entry.fields.day), "status": .string(entry.fields.status)])
            }
        return ["epoch": epoch.map(RemoteValue.string) ?? .null, "cursor": .integer(cursor),
                "visibility": visibility.map(RemoteValue.string) ?? .null, "changes": .array(changes)]
    }

    public mutating func merge(_ reply: ChecklistReply) throws {
        guard reply.reset || epoch == reply.epoch else { throw RemoteError.incompatibleResponse }
        if reply.reset, epoch != nil {
            // A restored/replaced host must not silently reinterpret old revisions.
            for id in Array(entries.keys) {
                let project = entries[id]?.server?.project_id ?? entries[id]?.projectID
                entries[id]?.projectID = project
                entries[id]?.server = nil; entries[id]?.baseRevision = 0
                entries[id]?.conflict = true; entries[id]?.unavailable = false
            }
        }
        epoch = reply.epoch; cursor = reply.cursor; visibility = reply.visibility
        visibleProjects = Set(reply.visible_project_ids)
        for outcome in reply.outcomes {
            guard var entry = entries[outcome.id] else { continue }
            if outcome.status == "unavailable" { entry.unavailable = true; entries[entry.id] = entry; continue }
            entry.server = outcome.item
            if let server = outcome.item { entry.projectID = server.project_id }
            if entry.mutationID == outcome.mutation_id {
                if outcome.status == "applied", let server = outcome.item {
                    entry.fields = server.fields; entry.baseRevision = server.revision; entry.mutationID = nil; entry.conflict = false
                } else { entry.conflict = true }
            } else if outcome.status == "applied", let server = outcome.item {
                // A newer local edit was made while the acknowledged request was in flight.
                entry.baseRevision = server.revision
            } else { entry.conflict = true }
            entries[entry.id] = entry
        }
        for server in reply.items {
            if var entry = entries[server.id] {
                guard server.revision >= (entry.server?.revision ?? 0) else { continue }
                entry.server = server; entry.projectID = server.project_id; entry.unavailable = false
                if entry.mutationID == nil && !entry.conflict {
                    entry.fields = server.fields; entry.baseRevision = server.revision
                } else if !entry.conflict && server.revision != entry.baseRevision {
                    if entry.fields == server.fields {
                        entry.mutationID = nil; entry.baseRevision = server.revision
                    } else { entry.conflict = true }
                }
                entries[server.id] = entry
            } else {
                entries[server.id] = ChecklistEntry(id: server.id, fields: server.fields, server: server,
                    mutationID: nil, baseRevision: server.revision, conflict: false, unavailable: false)
            }
        }
    }
}
