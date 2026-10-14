import Foundation

/// View projections of the existing wisp-dto research contracts. No new wire schema.
public struct RemoteResearchEntry: Identifiable, Hashable, Sendable {
    public let id: String
    public let kind: String
    public let title: String
    public let summary: String
    public let occurredAt: Int64
    public let recordedAt: Int64
    public let sourceID: String
    public let sessionID: String?
    public let status: String
    public let contentType: String
    public let version: Int64?
    public let sourceDiscarded: Bool
    public let manual: Bool

    public init(_ value: RemoteValue) throws {
        guard !value["id"].string.isEmpty, !value["kind"].string.isEmpty,
              let occurred = value["occurred_at"].researchInteger,
              let recorded = value["recorded_at"].researchInteger else { throw RemoteError.incompatibleResponse }
        id = value["id"].string; kind = value["kind"].string
        title = value["title"].string; summary = value["summary"].string
        occurredAt = occurred; recordedAt = recorded
        sourceID = value["source_id"].string
        sessionID = value["frame_id"].string.isEmpty ? nil : value["frame_id"].string
        status = value["status"].string; contentType = value["content_type"].string
        version = value["version_number"].researchInteger
        sourceDiscarded = value["source_discarded"].bool; manual = value["manual"].bool
    }

    public var date: Date { Date(timeIntervalSince1970: TimeInterval(occurredAt)) }
    public func route(project: String) -> ConversationRoute? {
        guard !sourceDiscarded, let sessionID else { return nil }
        return ConversationRoute(project: project, session: sessionID)
    }
    public var kindLabel: String {
        ["run": "运行", "artifact": "产物", "finding": "发现", "decision": "决策",
         "note": "笔记", "progress": "进展", "session": "对话", "plan": "计划",
         "question": "问题", "issue": "问题", "milestone": "里程碑", "paper": "文献", "data_asset": "数据资产"][kind] ?? kind
    }
}

public struct RemoteResearchHistory: Sendable {
    public let entries: [RemoteResearchEntry]
    public let truncated: Bool

    public init(_ value: RemoteValue) throws {
        guard case .array(let rows) = value["entries"], case .bool(let truncated) = value["truncated"] else {
            throw RemoteError.incompatibleResponse
        }
        var seen = Set<String>()
        entries = try rows.map(RemoteResearchEntry.init).filter { seen.insert($0.id).inserted }.sorted {
            $0.occurredAt == $1.occurredAt ? $0.id < $1.id : $0.occurredAt > $1.occurredAt
        }
        self.truncated = truncated
    }

    public func days(calendar: Calendar) -> [RemoteResearchDay] {
        Dictionary(grouping: entries) { calendar.startOfDay(for: $0.date) }
            .map { RemoteResearchDay(date: $0.key, entries: $0.value) }.sorted { $0.date > $1.date }
    }
}

public struct RemoteResearchDay: Identifiable, Sendable {
    public let date: Date
    public let entries: [RemoteResearchEntry]
    public var id: Date { date }
    public var sessions: [RemoteResearchSession] {
        Dictionary(grouping: entries, by: \.sessionID).map {
            RemoteResearchSession(sessionID: $0.key, entries: $0.value)
        }.sorted {
            let lhs = $0.entries.first?.occurredAt ?? 0, rhs = $1.entries.first?.occurredAt ?? 0
            return lhs == rhs ? $0.id < $1.id : lhs > rhs
        }
    }
}

public struct RemoteResearchSession: Identifiable, Sendable {
    public let sessionID: String?
    public let entries: [RemoteResearchEntry]
    public var id: String { sessionID.map { "session:" + $0 } ?? "project" }
}

public struct RemoteResearchProject: Identifiable, Sendable {
    public let id: String
    public let history: RemoteResearchHistory
    public let error: String?

    public static func decode(_ value: RemoteValue, requested: Set<String>) throws -> [Self] {
        guard case .array(let rows) = value else { throw RemoteError.incompatibleResponse }
        var seen = Set<String>()
        return try rows.map { row in
            let id = row["project_id"].string
            guard requested.contains(id), seen.insert(id).inserted else { throw RemoteError.incompatibleResponse }
            let error: String?
            switch row["error"] {
            case .null: error = nil
            case .string(let message): error = message
            default: throw RemoteError.incompatibleResponse
            }
            return Self(id: id, history: try RemoteResearchHistory(row["history"]), error: error)
        }
    }
}

public enum RemoteResearchClock {
    public static func localCalendar(timeZone: TimeZone = .autoupdatingCurrent) -> Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = timeZone
        calendar.firstWeekday = 2
        return calendar
    }

    public static func interval(_ component: Calendar.Component, containing date: Date, calendar: Calendar) -> DateInterval {
        // All callers use .month or .day with representable UI dates.
        calendar.dateInterval(of: component, for: date)!
    }

    public static func args(_ interval: DateInterval) -> [String: RemoteValue] {
        ["from": .integer(Int64(interval.start.timeIntervalSince1970)), "until": .integer(Int64(interval.end.timeIntervalSince1970))]
    }

    /// Monday-first grid, padded on both sides. Calendar day arithmetic handles DST.
    public static func cells(containing date: Date, calendar: Calendar) -> [Date?] {
        let start = interval(.month, containing: date, calendar: calendar).start
        let offset = (calendar.component(.weekday, from: start) + 5) % 7
        let count = calendar.range(of: .day, in: .month, for: start)!.count
        var cells = [Date?](repeating: nil, count: offset)
        cells += (0..<count).map { calendar.date(byAdding: .day, value: $0, to: start) }
        cells += [Date?](repeating: nil, count: (7 - cells.count % 7) % 7)
        return cells
    }
}

private extension RemoteValue {
    var researchInteger: Int64? {
        switch self {
        case .integer(let value): return value
        case .unsigned(let value): return Int64(exactly: value)
        default: return nil
        }
    }
}
