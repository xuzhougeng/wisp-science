import SwiftUI
import WispProjectBrowser

struct JourneyFocus: Equatable {
    var projectID: String
    var day: Int64
}

struct CalendarEntry: Codable, Identifiable, Equatable {
    var id: String
    var kind: String
    var title: String
    var occurredAt: Int64
    var status: String
    var manual: Bool

    enum CodingKeys: String, CodingKey {
        case id, kind, title, status, manual
        case occurredAt = "occurred_at"
    }
}

struct CalendarHistory: Codable, Equatable {
    var entries: [CalendarEntry]
    var truncated: Bool
}

struct CalendarProject: Codable, Equatable, Identifiable {
    var id: String { projectID }
    var projectID: String
    var history: CalendarHistory
    var error: String?

    enum CodingKeys: String, CodingKey {
        case history, error
        case projectID = "project_id"
    }
}

enum NativeCalendarCommand {
    static let read = "native_research_calendar"
}

enum NativeCalendarClock {
    static func monthInterval(containing date: Date, calendar: Calendar) -> (Int64, Int64) {
        let parts = calendar.dateComponents([.year, .month], from: date)
        let start = calendar.date(from: parts) ?? date
        let next = calendar.date(byAdding: .month, value: 1, to: start) ?? start
        return (Int64(start.timeIntervalSince1970), Int64(next.timeIntervalSince1970))
    }

    static func dayInterval(containing date: Date, calendar: Calendar) -> (Int64, Int64) {
        let start = calendar.startOfDay(for: date)
        let next = calendar.date(byAdding: .day, value: 1, to: start) ?? start
        return (Int64(start.timeIntervalSince1970), Int64(next.timeIntervalSince1970))
    }

    static func dayStart(_ timestamp: Int64, calendar: Calendar) -> Int64 {
        dayInterval(containing: Date(timeIntervalSince1970: TimeInterval(timestamp)), calendar: calendar).0
    }
}

@MainActor
final class NativeCalendarModel: ObservableObject {
    @Published var presented = false
    @Published var projectFilter: String?
    @Published var privacyActive = false
    @Published var privacyProjectIDs: Set<String> = []
    @Published private(set) var monthRows: [CalendarProject] = []
    @Published private(set) var dayRows: [CalendarProject] = []
    @Published private(set) var selectedDay: Int64 = 0
    @Published private(set) var monthStart: Int64 = 0
    @Published private(set) var busy = false
    @Published private(set) var error: String?
    private var monthGeneration = UUID()
    private var dayGeneration = UUID()
    var clock = Date()
    var calendar = Calendar.current

    func dismiss() {
        guard !busy else { return }
        presented = false
    }

    func invalidate() {
        monthGeneration = UUID()
        dayGeneration = UUID()
        presented = false
    }

    static func requestedIDs(_ projectIDs: [String], privacyActive: Bool, privacy: Set<String>) -> [String] {
        var seen = Set<String>()
        return projectIDs.filter { id in
            if privacyActive && privacy.contains(id) { return false }
            return seen.insert(id).inserted
        }
    }

    func visibleProjects(_ rows: [CalendarProject]) -> [CalendarProject] {
        rows.filter { projectFilter == nil || $0.projectID == projectFilter }
    }

    func markedDays() -> [Int64: [String]] {
        var marks: [Int64: [String]] = [:]
        for project in visibleProjects(monthRows) where project.error == nil {
            for entry in project.history.entries {
                let day = NativeCalendarClock.dayStart(entry.occurredAt, calendar: calendar)
                var ids = marks[day] ?? []
                if !ids.contains(project.projectID) { ids.append(project.projectID) }
                marks[day] = ids
            }
        }
        return marks
    }

    func dayGroups() -> [CalendarProject] {
        visibleProjects(dayRows).filter { $0.error != nil || !$0.history.entries.isEmpty }
    }

    static func sorted(_ entries: [CalendarEntry]) -> [CalendarEntry] {
        entries.sorted { left, right in
            if left.occurredAt != right.occurredAt { return left.occurredAt > right.occurredAt }
            return left.id > right.id
        }
    }

    func reloadMonth(_ client: any NativeSettingsQuerying, projectIDs: [String]) async {
        let bounds = NativeCalendarClock.monthInterval(containing: clock, calendar: calendar)
        monthStart = bounds.0
        if selectedDay == 0 { selectedDay = NativeCalendarClock.dayInterval(containing: clock, calendar: calendar).0 }
        await load(client, projectIDs: projectIDs, from: bounds.0, until: bounds.1, month: true)
        guard presented, error == nil else { return }
        await reloadDay(client, projectIDs: projectIDs)
    }

    func showDay(_ day: Int64, client: any NativeSettingsQuerying, projectIDs: [String]) async {
        selectedDay = day
        await reloadDay(client, projectIDs: projectIDs)
    }

    func shiftMonth(_ delta: Int, client: any NativeSettingsQuerying, projectIDs: [String]) async {
        guard let next = calendar.date(byAdding: .month, value: delta, to: Date(timeIntervalSince1970: TimeInterval(monthStart == 0 ? Int64(clock.timeIntervalSince1970) : monthStart))) else { return }
        clock = next
        selectedDay = NativeCalendarClock.dayInterval(containing: next, calendar: calendar).0
        await reloadMonth(client, projectIDs: projectIDs)
    }

    private func reloadDay(_ client: any NativeSettingsQuerying, projectIDs: [String]) async {
        let bounds = NativeCalendarClock.dayInterval(containing: Date(timeIntervalSince1970: TimeInterval(selectedDay)), calendar: calendar)
        await load(client, projectIDs: projectIDs, from: bounds.0, until: bounds.1, month: false)
    }

    private func load(_ client: any NativeSettingsQuerying, projectIDs: [String], from: Int64, until: Int64, month: Bool) async {
        guard presented else { return }
        let ids = Self.requestedIDs(projectIDs, privacyActive: privacyActive, privacy: privacyProjectIDs)
        let generation = UUID()
        if month { monthGeneration = generation } else { dayGeneration = generation }
        if ids.isEmpty {
            if month { monthRows = [] } else { dayRows = [] }
            return
        }
        busy = true
        error = nil
        defer {
            let current = month ? monthGeneration : dayGeneration
            if current == generation { busy = false }
        }
        do {
            let value = try await client.invoke(
                NativeCalendarCommand.read,
                args: [
                    "project_ids": .array(ids.map(SettingsValue.string)),
                    "from": .integer(from),
                    "until": .integer(until),
                ],
                projectID: nil)
            let current = month ? monthGeneration : dayGeneration
            guard current == generation, presented else { return }
            let rows = try JSONDecoder().decode([CalendarProject].self, from: JSONEncoder().encode(value))
            if month { monthRows = rows } else { dayRows = rows }
            error = nil
        } catch {
            let current = month ? monthGeneration : dayGeneration
            guard current == generation, presented else { return }
            self.error = "研究日历未能确认读取，不会自动重试。\n" + error.localizedDescription
        }
    }
}

struct NativeCalendarSheet: View {
    @ObservedObject var model: ProjectBrowserModel
    @ObservedObject var calendar: NativeCalendarModel

    private var projectIDs: [String] {
        model.projects.map(\.id)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Button("返回首页") { calendar.dismiss() }
                Text("研究日历").font(.headline)
                Spacer()
                Button("上个月") { Task { await calendar.shiftMonth(-1, client: model.calendarClient(), projectIDs: projectIDs) } }.disabled(calendar.busy)
                Button("下个月") { Task { await calendar.shiftMonth(1, client: model.calendarClient(), projectIDs: projectIDs) } }.disabled(calendar.busy)
                Button("刷新") { Task { await calendar.reloadMonth(model.calendarClient(), projectIDs: projectIDs) } }.disabled(calendar.busy)
            }
            ScrollView(.horizontal) {
                HStack {
                    Button("所有项目") { calendar.projectFilter = nil }
                        .accessibilityAddTraits(calendar.projectFilter == nil ? .isSelected : [])
                    ForEach(model.projects.filter { !calendar.privacyActive || !calendar.privacyProjectIDs.contains($0.id) }) { project in
                        Button(project.name) { calendar.projectFilter = project.id }
                            .accessibilityAddTraits(calendar.projectFilter == project.id ? .isSelected : [])
                    }
                }
            }
            if let error = calendar.error {
                Text(error).font(.caption).foregroundStyle(.red).textSelection(.enabled)
            }
            let marks = calendar.markedDays()
            LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 7)) {
                ForEach(days, id: \.self) { day in
                    let start = NativeCalendarClock.dayInterval(containing: day, calendar: calendar.calendar).0
                    Button("\(calendar.calendar.component(.day, from: day))") { Task { await calendar.showDay(start, client: model.calendarClient(), projectIDs: projectIDs) } }
                        .accessibilityLabel(dayLabel(day))
                        .accessibilityAddTraits(calendar.selectedDay == start ? .isSelected : [])
                        .overlay(alignment: .bottom) {
                            if let ids = marks[start], !ids.isEmpty { Text("\(ids.count)").font(.caption2) }
                        }
                }
            }
            Text(dayLabel(Date(timeIntervalSince1970: TimeInterval(calendar.selectedDay)))).font(.subheadline)
            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(calendar.dayGroups()) { project in
                        VStack(alignment: .leading, spacing: 4) {
                            HStack {
                                Text(name(project.projectID)).font(.headline)
                                Spacer()
                                Button("打开研究历程") { Task { await model.openCalendarJourney(projectID: project.projectID, day: calendar.selectedDay) } }
                            }
                            if let error = project.error { Text(error).foregroundStyle(.red).font(.caption) }
                            ForEach(NativeCalendarModel.sorted(project.history.entries)) { entry in
                                Text(entry.title).font(.body)
                            }
                        }
                    }
                    if calendar.dayGroups().isEmpty && calendar.error == nil {
                        Text("当天没有已记录的研究活动。").foregroundStyle(.secondary)
                    }
                }
            }
        }
        .padding(20)
        .frame(width: 640, height: 520)
        .interactiveDismissDisabled(calendar.busy)
        .background(NativeSettingsEscape(enabled: !calendar.busy) { calendar.dismiss() })
        .task { await calendar.reloadMonth(model.calendarClient(), projectIDs: projectIDs) }
    }

    private var days: [Date] {
        let start = Date(timeIntervalSince1970: TimeInterval(calendar.monthStart))
        let range = calendar.calendar.range(of: .day, in: .month, for: start) ?? 1..<2
        return range.compactMap { day in
            calendar.calendar.date(bySetting: .day, value: day, of: start)
        }
    }

    private func dayLabel(_ date: Date) -> String {
        let formatter = DateFormatter()
        formatter.calendar = calendar.calendar
        formatter.timeZone = calendar.calendar.timeZone
        formatter.dateFormat = "yyyy-MM-dd"
        return formatter.string(from: date)
    }

    private func name(_ id: String) -> String {
        model.projects.first { $0.id == id }?.name ?? id
    }
}


