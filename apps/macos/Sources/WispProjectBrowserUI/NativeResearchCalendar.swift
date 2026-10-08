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
    var summary: String? = nil
    var sourceID: String? = nil
    var frameID: String? = nil
    var contentType: String? = nil
    var versionNumber: Int64? = nil
    var sourceDiscarded: Bool? = nil

    enum CodingKeys: String, CodingKey {
        case id, kind, title, status, manual
        case occurredAt = "occurred_at"
        case summary
        case sourceID = "source_id", frameID = "frame_id", contentType = "content_type"
        case versionNumber = "version_number", sourceDiscarded = "source_discarded"
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

struct StoredPrivacy: Decodable {
    var active: Bool
    var projectIDs: [String]
    enum CodingKeys: String, CodingKey {
        case active
        case projectIDs = "project_ids"
    }
}

enum NativeCalendarCommand {
    static let read = "native_research_calendar"
    static let privacy = "get_privacy_mode"
}

enum CalendarPrivacyGate: Equatable {
    case unresolved
    case ready(active: Bool, projectIDs: Set<String>)
    case failed
}

enum CalendarPrivacyDecision {
    /// A calendar read is allowed only after privacy mode is known. Unresolved
    /// and failed gates return nil so no project id is submitted.
    static func admit(_ gate: CalendarPrivacyGate, projectIDs: [String]) -> [String]? {
        guard case .ready(let active, let privacy) = gate else { return nil }
        return filteredProjectIDs(projectIDs, privacyActive: active, privacy: privacy)
    }
}

func filteredProjectIDs(_ projectIDs: [String], privacyActive: Bool, privacy: Set<String>) -> [String] {
    var seen = Set<String>()
    return projectIDs.filter { id in
        if privacyActive && privacy.contains(id) { return false }
        return seen.insert(id).inserted
    }
}

enum NativeCalendarClock {
    /// Monday-first, matching the WebView. Nil cells preserve the weekday of
    /// the first date; dates advance by calendar days, never fixed seconds.
    static func monthCells(containing date: Date, calendar: Calendar) -> [Date?] {
        let bounds = monthInterval(containing: date, calendar: calendar)
        let first = Date(timeIntervalSince1970: TimeInterval(bounds.0))
        let offset = (calendar.component(.weekday, from: first) + 5) % 7
        let count = calendar.range(of: .day, in: .month, for: first)?.count ?? 0
        var cells = Array<Date?>(repeating: nil, count: offset)
        cells += (0..<count).map { calendar.date(byAdding: .day, value: $0, to: first) }
        cells += Array<Date?>(repeating: nil, count: (7 - cells.count % 7) % 7)
        return cells
    }

    static func label(_ date: Date, format: String, calendar: Calendar) -> String {
        let formatter = DateFormatter()
        formatter.calendar = calendar
        formatter.timeZone = calendar.timeZone
        formatter.locale = Locale(identifier: UserDefaults.standard.string(forKey: "nativeSettings.locale") == "en" ? "en_US" : "zh_CN")
        formatter.dateFormat = format
        return formatter.string(from: date)
    }

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
    @Published private(set) var privacyGate: CalendarPrivacyGate = .unresolved
    @Published var privacyActive = false
    @Published var privacyProjectIDs: Set<String> = []
    private var privacyToken = UUID()
    var navigationEnabled: Bool {
        if case .ready = privacyGate { return true }
        return false
    }
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
        invalidate()
    }

    func invalidate() {
        privacyToken = UUID()
        monthGeneration = UUID()
        dayGeneration = UUID()
        presented = false
        busy = false
    }

    static func requestedIDs(_ projectIDs: [String], privacyActive: Bool, privacy: Set<String>) -> [String] {
        filteredProjectIDs(projectIDs, privacyActive: privacyActive, privacy: privacy)
    }

    func visibleProjects(_ rows: [CalendarProject]) -> [CalendarProject] {
        guard navigationEnabled else { return [] }
        return rows.filter {
            (!privacyActive || !privacyProjectIDs.contains($0.projectID))
                && (projectFilter == nil || $0.projectID == projectFilter)
        }
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

    nonisolated static func sorted(_ entries: [CalendarEntry]) -> [CalendarEntry] {
        entries.sorted { left, right in
            if left.occurredAt != right.occurredAt { return left.occurredAt > right.occurredAt }
            return left.id > right.id
        }
    }

    func applyPrivacy(active: Bool, projectIDs: Set<String>) {
        let active = active && !projectIDs.isEmpty
        privacyActive = active
        privacyProjectIDs = projectIDs
        privacyGate = .ready(active: active, projectIDs: projectIDs)
        if let projectFilter, active, projectIDs.contains(projectFilter) {
            self.projectFilter = nil
        }
    }

    func openMonth(_ client: any NativeSettingsQuerying, projectIDs: [String]) async {
        guard presented else { return }
        let token = UUID()
        privacyToken = token
        privacyGate = .unresolved
        monthGeneration = UUID()
        dayGeneration = UUID()
        busy = true
        error = nil
        defer { if privacyToken == token { busy = false } }
        do {
            let value = try await client.invoke(NativeCalendarCommand.privacy, args: [:], projectID: nil)
            guard privacyToken == token, presented else { return }
            let stored = try JSONDecoder().decode(StoredPrivacy.self, from: JSONEncoder().encode(value))
            applyPrivacy(active: stored.active, projectIDs: Set(stored.projectIDs))
        } catch {
            guard privacyToken == token, presented else { return }
            privacyGate = .failed
            self.error = "隐私模式未能确认读取，不会自动重试。\n" + error.localizedDescription
            return
        }
        await reloadMonth(client, projectIDs: projectIDs)
    }

    func reloadMonth(_ client: any NativeSettingsQuerying, projectIDs: [String]) async {
        guard presented, navigationEnabled else { return }
        let bounds = NativeCalendarClock.monthInterval(containing: clock, calendar: calendar)
        monthStart = bounds.0
        if selectedDay == 0 { selectedDay = NativeCalendarClock.dayInterval(containing: clock, calendar: calendar).0 }
        dayGeneration = UUID()
        let accepted = await load(client, projectIDs: projectIDs, from: bounds.0, until: bounds.1, month: true)
        guard accepted, presented, error == nil else { return }
        await reloadDay(client, projectIDs: projectIDs)
    }

    func showDay(_ day: Int64, client: any NativeSettingsQuerying, projectIDs: [String]) async {
        guard presented, navigationEnabled else { return }
        selectedDay = day
        dayRows = []
        await reloadDay(client, projectIDs: projectIDs)
    }

    func shiftMonth(_ delta: Int, client: any NativeSettingsQuerying, projectIDs: [String]) async {
        guard presented, navigationEnabled else { return }
        guard let next = calendar.date(byAdding: .month, value: delta, to: Date(timeIntervalSince1970: TimeInterval(monthStart == 0 ? Int64(clock.timeIntervalSince1970) : monthStart))) else { return }
        clock = next
        selectedDay = NativeCalendarClock.dayInterval(containing: next, calendar: calendar).0
        monthRows = []; dayRows = []
        await reloadMonth(client, projectIDs: projectIDs)
    }

    func showToday(_ client: any NativeSettingsQuerying, projectIDs: [String], today: Date = Date()) async {
        guard presented, navigationEnabled else { return }
        clock = today
        selectedDay = NativeCalendarClock.dayInterval(containing: today, calendar: calendar).0
        monthRows = []; dayRows = []
        await reloadMonth(client, projectIDs: projectIDs)
    }

    private func reloadDay(_ client: any NativeSettingsQuerying, projectIDs: [String]) async {
        let bounds = NativeCalendarClock.dayInterval(containing: Date(timeIntervalSince1970: TimeInterval(selectedDay)), calendar: calendar)
        _ = await load(client, projectIDs: projectIDs, from: bounds.0, until: bounds.1, month: false)
    }

    private func load(_ client: any NativeSettingsQuerying, projectIDs: [String], from: Int64, until: Int64, month: Bool) async -> Bool {
        guard presented else { return false }
        guard let ids = CalendarPrivacyDecision.admit(privacyGate, projectIDs: projectIDs) else { return false }
        let generation = UUID()
        if month { monthGeneration = generation } else { dayGeneration = generation }
        if ids.isEmpty {
            if month { monthRows = [] } else { dayRows = [] }
            error = nil
            return true
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
            guard current == generation, presented else { return false }
            let rows = try JSONDecoder().decode([CalendarProject].self, from: JSONEncoder().encode(value))
            if month { monthRows = rows } else { dayRows = rows }
            error = nil
            return true
        } catch {
            let current = month ? monthGeneration : dayGeneration
            guard current == generation, presented else { return false }
            self.error = "研究日历未能确认读取，不会自动重试。\n" + error.localizedDescription
            return false
        }
    }
}

struct NativeCalendarPage: View {
    @ObservedObject var model: ProjectBrowserModel
    @ObservedObject var calendar: NativeCalendarModel
    var embedded = false
    @Environment(\.colorScheme) private var scheme
    @State private var filtersExpanded = false
    private var projectIDs: [String] { model.projects.map(\.id) }
    private var allowedProjects: [ProjectSummary] {
        guard calendar.navigationEnabled else { return [] }
        return model.projects.filter { !calendar.privacyActive || !calendar.privacyProjectIDs.contains($0.id) }
    }
    private var displayedMonth: Date {
        calendar.monthStart == 0 ? calendar.clock : Date(timeIntervalSince1970: TimeInterval(calendar.monthStart))
    }
    private func color(_ key: String) -> Color { WispDesign.color(key, scheme) }

    var body: some View {
        GeometryReader { geometry in
            VStack(alignment: .leading, spacing: 0) {
                HStack {
                    if !embedded { Button(localized("返回首页")) { calendar.dismiss() } }
                    Text(localized("研究日历")).font(.title2.bold())
                    Spacer()
                    Button { Task { await calendar.openMonth(model.calendarClient(), projectIDs: projectIDs) } } label: { WispIcon(name: "refresh") }
                        .help(localized("刷新")).accessibilityLabel(localized("刷新"))
                        .disabled(calendar.busy)
                }.padding(20)
                Divider()
                HStack(alignment: .top, spacing: 0) {
                    if geometry.size.width >= 820 {
                        projectFilters.frame(width: 180).padding(20)
                        Divider()
                    }
                    ScrollView {
                        VStack(alignment: .leading, spacing: 20) {
                            if geometry.size.width < 820 {
                                DisclosureGroup(localized("筛选项目"), isExpanded: $filtersExpanded) { projectFilters }
                            }
                            monthHeader
                            if calendar.busy { ProgressView(localized("正在读取研究活动…")) }
                            if let error = calendar.error { Text(error).foregroundStyle(.red).textSelection(.enabled) }
                            if calendar.navigationEnabled {
                                ForEach(calendar.visibleProjects(calendar.monthRows).filter { $0.error != nil || $0.history.truncated }) { row in
                                    Text(name(row.projectID) + "：" + (row.error ?? localized("仅展示最近 2,000 条活动；选择日期可读取当日记录。")))
                                        .font(.caption).foregroundStyle(row.error == nil ? color("text-muted") : .red)
                                }
                                monthGrid
                                Divider()
                                dayDetail
                            }
                        }.padding(24)
                    }
                }
            }.background(color("bg-app"))
        }
        .background { if !embedded { NativeSettingsEscape { calendar.dismiss() } } }
        .task { if embedded { calendar.presented = true }; await calendar.openMonth(model.calendarClient(), projectIDs: projectIDs) }
        .onDisappear { if embedded { calendar.dismiss() } }
    }

    private var projectFilters: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(localized("项目")).font(.headline)
            Button { calendar.projectFilter = nil } label: {
                HStack { Text(localized("所有项目")); Spacer(); if calendar.projectFilter == nil { WispIcon(name: "check", size: 14) } }
            }.accessibilityAddTraits(calendar.projectFilter == nil ? .isSelected : [])
            ForEach(allowedProjects) { project in
                Button { calendar.projectFilter = project.id } label: {
                    HStack { Text(project.name).lineLimit(2); Spacer(); if calendar.projectFilter == project.id { WispIcon(name: "check", size: 14) } }
                }.accessibilityAddTraits(calendar.projectFilter == project.id ? .isSelected : [])
            }
        }.buttonStyle(WispSidebarButtonStyle()).disabled(!calendar.navigationEnabled)
    }

    private var monthHeader: some View {
        HStack(spacing: 14) {
            Text(NativeCalendarClock.label(displayedMonth, format: localized("yyyy年M月"), calendar: calendar.calendar))
                .font(.title2.weight(.semibold)).accessibilityIdentifier("calendar-month-title")
            Spacer()
            Button { Task { await calendar.shiftMonth(-1, client: model.calendarClient(), projectIDs: projectIDs) } } label: { WispIcon(name: "chevron-left", size: 16) }
                .help(localized("上个月")).accessibilityLabel(localized("上个月"))
            Button(localized("今天")) { Task { await calendar.showToday(model.calendarClient(), projectIDs: projectIDs) } }
            Button { Task { await calendar.shiftMonth(1, client: model.calendarClient(), projectIDs: projectIDs) } } label: { WispIcon(name: "chevron-right", size: 16) }
                .help(localized("下个月")).accessibilityLabel(localized("下个月"))
        }.disabled(!calendar.navigationEnabled || calendar.busy)
    }

    private var monthGrid: some View {
        let marks = calendar.markedDays()
        let cells = NativeCalendarClock.monthCells(containing: displayedMonth, calendar: calendar.calendar)
        return LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 6), count: 7), spacing: 6) {
            ForEach(["周一", "周二", "周三", "周四", "周五", "周六", "周日"], id: \.self) { title in
                Text(localized(title)).font(.caption.weight(.semibold)).foregroundStyle(color("text-muted"))
                    .frame(maxWidth: .infinity).padding(.bottom, 8)
            }
            ForEach(cells.indices, id: \.self) { index in
                if let day = cells[index] {
                    let start = NativeCalendarClock.dayStart(Int64(day.timeIntervalSince1970), calendar: calendar.calendar)
                    let selected = calendar.selectedDay == start
                    let count = marks[start]?.count ?? 0
                    let today = calendar.calendar.isDateInToday(day)
                    Button { Task { await calendar.showDay(start, client: model.calendarClient(), projectIDs: projectIDs) } } label: {
                        VStack(alignment: .leading, spacing: 5) {
                            Text("\(calendar.calendar.component(.day, from: day))")
                                .font(.body.weight(today || selected ? .bold : .regular))
                            HStack(spacing: 3) {
                                if count > 0 { Circle().fill(color("clay")).frame(width: 5, height: 5); Text("\(count)").font(.caption2) }
                                else { Text(" ").font(.caption2) }
                            }.foregroundStyle(color("text-muted"))
                        }.frame(maxWidth: .infinity, minHeight: 54, alignment: .leading).padding(.horizontal, 8)
                            .background(selected ? color("bg-sunken") : color("bg-elev"), in: RoundedRectangle(cornerRadius: 8))
                            .overlay(RoundedRectangle(cornerRadius: 8).stroke(selected || today ? color("clay") : color("border"), lineWidth: selected ? 2 : 1))
                    }.buttonStyle(.plain).disabled(calendar.busy)
                        .accessibilityLabel(dayLabel(day) + (today ? " · " + localized("今天") : "") + " · \(count) " + localized("个项目有活动"))
                        .accessibilityIdentifier("calendar-day-\(start)")
                        .accessibilityAddTraits(selected ? .isSelected : [])
                } else { Color.clear.frame(height: 54).accessibilityHidden(true) }
            }
        }
    }

    private var dayDetail: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(dayLabel(Date(timeIntervalSince1970: TimeInterval(calendar.selectedDay)))).font(.headline)
            ForEach(calendar.dayGroups()) { project in
                VStack(alignment: .leading, spacing: 10) {
                    HStack {
                        Text(name(project.projectID)).font(.headline)
                        Spacer()
                        Button(localized("打开研究历程")) { if embedded { model.assistantPresented = false }; Task { await model.openCalendarJourney(projectID: project.projectID, day: calendar.selectedDay) } }
                            .disabled(project.error != nil || calendar.busy)
                    }
                    if let error = project.error { Text(error).foregroundStyle(.red).font(.caption) }
                    else {
                        Text(String(format: localized("%d 条研究记录"), project.history.entries.count)).font(.caption).foregroundStyle(color("text-muted"))
                        ForEach(Array(NativeJourneyDay.grouped(project.history.entries, calendar: calendar.calendar).flatMap(\.entries).prefix(3))) { entry in
                            Text(entry.title).lineLimit(2)
                        }
                        if project.history.truncated { Text(localized("当日记录已截断，仅展示最近 2,000 条。")).font(.caption) }
                    }
                }.padding(16).background(color("bg-elev"), in: RoundedRectangle(cornerRadius: 10))
            }
            if calendar.dayGroups().isEmpty && calendar.error == nil && !calendar.busy {
                Text(localized("当天没有已记录的研究活动。")).foregroundStyle(color("text-muted"))
            }
        }
    }

    private func dayLabel(_ date: Date) -> String { NativeCalendarClock.label(date, format: localized("yyyy年M月d日 EEEE"), calendar: calendar.calendar) }
    private func name(_ id: String) -> String { model.projects.first { $0.id == id }?.name ?? id }
}
