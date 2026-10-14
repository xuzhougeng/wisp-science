import SwiftUI
import WispRemote

private struct ResearchLoadKey: Equatable {
    let query: ResearchQuery
    let session: UUID
    let state: RemoteConnection.State
    let refresh: UUID
}

struct ResearchView: View {
    @ObservedObject var workspace: RemoteWorkspace
    @ObservedObject var connection: RemoteConnection
    let project: String?

    var body: some View {
        NavigationStack(path: $workspace.researchPath) {
            Group {
                if let project {
                    ResearchJourneyView(workspace: workspace, connection: connection, project: project, day: nil)
                } else {
                    ResearchCalendarView(workspace: workspace, connection: connection)
                }
            }
            .navigationDestination(for: ResearchDestination.self) { destination in
                switch destination {
                case .journey(let project, let day):
                    ResearchJourneyView(workspace: workspace, connection: connection, project: project, day: day)
                case .entry(let project, let entry):
                    ResearchEntryView(workspace: workspace, connection: connection, project: project, entry: entry)
                }
            }
            .toolbar {
                if workspace.researchPath.isEmpty {
                    ToolbarItem(placement: .topBarLeading) { WorkspacePageBackButton(workspace: workspace) }
                }
            }
        }
        .accessibilityIdentifier("research-page")
        .onChange(of: connection.readSessionID) { _, _ in workspace.researchPath = [] }
    }
}

private struct ResearchCalendarView: View {
    @ObservedObject var workspace: RemoteWorkspace
    @ObservedObject var connection: RemoteConnection
    @StateObject private var loader = ResearchLoader()
    @State private var month = Date()
    @State private var selectedDay = Date()
    @State private var project: String?
    @State private var refresh = UUID()
    private var calendar: Calendar { RemoteResearchClock.localCalendar() }
    private var projectIDs: [String] { project.map { [$0] } ?? workspace.projects.map { $0["id"].string }.sorted() }
    private var query: ResearchQuery {
        ResearchQuery(project: nil, projectIDs: projectIDs, interval: RemoteResearchClock.interval(.month, containing: month, calendar: calendar))
    }

    var body: some View {
        List {
            Section {
                ScrollView(.horizontal) {
                    HStack(spacing: 8) {
                        filterButton("所有项目", id: nil)
                        ForEach(workspace.projects, id: \.recordID) { row in filterButton(row["name"].string, id: row["id"].string) }
                    }.padding(.vertical, 4)
                }.scrollIndicators(.hidden)
                ResearchMonthControls(month: month, previous: { shift(-1) }, next: { shift(1) }, today: {
                    month = Date(); selectedDay = Date()
                })
                grid
            }
            Section {
                ResearchLoadingStatus(loader: loader)
                ForEach(loader.projects.filter { $0.error != nil || $0.history.truncated }) { row in
                    Text("\(workspace.projectName(row.id))：\(row.error ?? "本月记录超过 2000 条，日期标记可能不完整，请按日查看。")")
                        .font(.caption).foregroundStyle(WispStyle.warning)
                }
            }
            ResearchDayPreview(workspace: workspace, connection: connection, projectIDs: projectIDs, day: selectedDay, refresh: refresh)
        }
        .navigationTitle("科研日历").navigationBarTitleDisplayMode(.inline)
        .task(id: ResearchLoadKey(query: query, session: connection.readSessionID, state: connection.state, refresh: refresh)) {
            await loader.load(query, connection: connection)
        }
        .refreshable { refresh = UUID() }
    }

    private func filterButton(_ title: String, id: String?) -> some View {
        Button { project = id } label: {
            Text(title).font(.subheadline).padding(.horizontal, 12).padding(.vertical, 10)
                .background(project == id ? WispStyle.accent.opacity(0.12) : WispStyle.surface, in: Capsule())
        }.buttonStyle(.plain).foregroundStyle(project == id ? WispStyle.accent : .primary)
            .accessibilityAddTraits(project == id ? .isSelected : [])
    }

    private var grid: some View {
        let cells = RemoteResearchClock.cells(containing: month, calendar: calendar)
        let counts = Dictionary(grouping: loader.projects.flatMap { $0.history.entries }) { calendar.startOfDay(for: $0.date) }.mapValues(\.count)
        return LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 2), count: 7), spacing: 6) {
            ForEach(["一", "二", "三", "四", "五", "六", "日"], id: \.self) { Text($0).font(.caption).foregroundStyle(.secondary) }
            ForEach(cells.indices, id: \.self) { index in
                if let date = cells[index] {
                    let selected = calendar.isDate(date, inSameDayAs: selectedDay)
                    let count = counts[date] ?? 0
                    let today = calendar.isDateInToday(date)
                    Button { selectedDay = date } label: {
                        VStack(spacing: 3) {
                            Text("\(calendar.component(.day, from: date))").font(.body.monospacedDigit()).minimumScaleFactor(0.6).lineLimit(1)
                            Circle().fill(count > 0 ? (selected ? WispStyle.onAccent : WispStyle.accent) : .clear).frame(width: 5, height: 5)
                        }
                        .frame(maxWidth: .infinity, minHeight: 44)
                        .foregroundStyle(selected ? WispStyle.onAccent : .primary)
                        .background(selected ? WispStyle.accent : .clear, in: RoundedRectangle(cornerRadius: 10))
                        .overlay { RoundedRectangle(cornerRadius: 10).strokeBorder(today ? WispStyle.accent : .clear, lineWidth: 1) }
                    }.buttonStyle(.plain)
                        .accessibilityLabel(date.formatted(date: .complete, time: .omitted) + (today ? "，今天" : "") + "，\(count) 条已加载记录")
                        .accessibilityAddTraits(selected ? .isSelected : [])
                        .accessibilityIdentifier("research-day-\(calendar.component(.day, from: date))")
                } else { Color.clear.frame(height: 44).accessibilityHidden(true) }
            }
        }.accessibilityIdentifier("research-calendar-grid")
    }

    private func shift(_ delta: Int) {
        let start = RemoteResearchClock.interval(.month, containing: month, calendar: calendar).start
        if let date = calendar.date(byAdding: .month, value: delta, to: start) { month = date; selectedDay = date }
    }
}

private struct ResearchDayPreview: View {
    @ObservedObject var workspace: RemoteWorkspace
    @ObservedObject var connection: RemoteConnection
    let projectIDs: [String]
    let day: Date
    let refresh: UUID
    @StateObject private var loader = ResearchLoader()
    private var query: ResearchQuery {
        ResearchQuery(project: nil, projectIDs: projectIDs, interval: RemoteResearchClock.interval(.day, containing: day, calendar: RemoteResearchClock.localCalendar()))
    }
    var body: some View {
        Section {
            ResearchLoadingStatus(loader: loader)
            ForEach(loader.projects.filter { !$0.history.entries.isEmpty || $0.error != nil || $0.history.truncated }) { row in
                VStack(alignment: .leading, spacing: 10) {
                    Text(workspace.projectName(row.id)).font(.headline)
                    if let error = row.error { Text(error).foregroundStyle(WispStyle.warning) }
                    else {
                        Text("\(row.history.entries.count) 条研究记录" + (row.history.truncated ? "（已截断）" : "")).font(.caption).foregroundStyle(.secondary)
                        ForEach(row.history.entries.prefix(3)) { entry in
                            Text("\(entry.kindLabel) · \(entry.title)").font(.subheadline).lineLimit(2)
                        }
                        NavigationLink(value: ResearchDestination.journey(row.id, day)) { Text("查看当日研究历程") }
                            .accessibilityIdentifier("research-open-day-\(row.id)")
                    }
                }.padding(.vertical, 5)
            }
            if !loader.busy && loader.error == nil && loader.projects.allSatisfy({ $0.error == nil && $0.history.entries.isEmpty && !$0.history.truncated }) {
                Text("当天没有可显示的研究记录。").foregroundStyle(.secondary)
            }
        } header: { Text(day.formatted(date: .complete, time: .omitted)) }
        .task(id: ResearchLoadKey(query: query, session: connection.readSessionID, state: connection.state, refresh: refresh)) {
            await loader.load(query, connection: connection)
        }
    }
}

private struct ResearchJourneyView: View {
    @ObservedObject var workspace: RemoteWorkspace
    @ObservedObject var connection: RemoteConnection
    let project: String
    let day: Date?
    @State private var month = Date()
    @State private var refresh = UUID()
    @StateObject private var loader = ResearchLoader()
    private var calendar: Calendar { RemoteResearchClock.localCalendar() }
    private var query: ResearchQuery {
        ResearchQuery(project: project, projectIDs: [], interval: RemoteResearchClock.interval(day == nil ? .month : .day, containing: day ?? month, calendar: calendar))
    }
    var body: some View {
        List {
            Section {
                Text(workspace.projectName(project)).font(.headline)
                if day == nil { ResearchMonthControls(month: month, previous: { shift(-1) }, next: { shift(1) }, today: { month = Date() }) }
                else if let day { Text(day.formatted(date: .complete, time: .omitted)).foregroundStyle(.secondary) }
                ResearchLoadingStatus(loader: loader)
                if loader.history?.truncated == true {
                    Text(day == nil ? "本月超过 2000 条记录，仅显示部分内容。请从科研日历选择具体日期查看。" : "当天超过 2000 条记录，仅显示部分内容。请在主机查看完整来源。")
                        .font(.caption).foregroundStyle(WispStyle.warning)
                }
                if let history = loader.history, history.entries.isEmpty { Text("这段时间没有研究记录。").foregroundStyle(.secondary) }
            }
            if let history = loader.history {
                ForEach(history.days(calendar: calendar)) { day in
                    Section(day.date.formatted(date: .complete, time: .omitted)) {
                        ForEach(day.sessions) { group in
                            Text(group.sessionID.map { workspace.sessionName($0, project: project) } ?? "项目记录")
                                .font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
                            ForEach(group.entries) { entry in
                                NavigationLink(value: ResearchDestination.entry(project, entry)) {
                                    VStack(alignment: .leading, spacing: 6) {
                                        HStack(alignment: .firstTextBaseline) {
                                            Text(entry.kindLabel).font(.caption).foregroundStyle(WispStyle.accent)
                                            Spacer()
                                            Text(entry.date.formatted(date: .omitted, time: .shortened)).font(.caption).foregroundStyle(.secondary)
                                        }
                                        Text(entry.title.isEmpty ? entry.kindLabel : entry.title).font(.headline)
                                        if !entry.summary.isEmpty { Text(entry.summary).font(.subheadline).foregroundStyle(.secondary).lineLimit(3) }
                                        if entry.sourceDiscarded { Text("来源已丢弃").font(.caption).foregroundStyle(WispStyle.warning) }
                                    }.padding(.vertical, 4)
                                }.accessibilityIdentifier("research-entry-\(entry.id)")
                            }
                        }
                    }
                }
            }
        }
        .navigationTitle("研究历程").navigationBarTitleDisplayMode(.inline)
        .task(id: ResearchLoadKey(query: query, session: connection.readSessionID, state: connection.state, refresh: refresh)) {
            await loader.load(query, connection: connection)
        }
        .refreshable { refresh = UUID() }
    }
    private func shift(_ delta: Int) {
        let start = RemoteResearchClock.interval(.month, containing: month, calendar: calendar).start
        if let date = calendar.date(byAdding: .month, value: delta, to: start) { month = date }
    }
}

private struct ResearchEntryView: View {
    @ObservedObject var workspace: RemoteWorkspace
    @ObservedObject var connection: RemoteConnection
    let project: String
    let entry: RemoteResearchEntry
    var body: some View {
        List {
            Section {
                Text(entry.title.isEmpty ? entry.kindLabel : entry.title).font(.title3.bold()).textSelection(.enabled)
                if !entry.summary.isEmpty { Text(entry.summary).textSelection(.enabled) }
            }
            Section("记录信息") {
                LabeledContent("项目", value: workspace.projectName(project))
                LabeledContent("类型", value: entry.kindLabel)
                LabeledContent("研究时间", value: entry.date.formatted(date: .abbreviated, time: .shortened))
                if entry.recordedAt != entry.occurredAt {
                    LabeledContent("记录时间", value: Date(timeIntervalSince1970: TimeInterval(entry.recordedAt)).formatted(date: .abbreviated, time: .shortened))
                }
                if !entry.status.isEmpty { LabeledContent("状态", value: researchStatus(entry.status)) }
                if entry.manual { Text("手动补充的研究记录") }
                if let version = entry.version { LabeledContent("产物版本", value: "v\(version)") }
                if !entry.contentType.isEmpty { LabeledContent("内容类型", value: entry.contentType) }
            }
            Section("来源") {
                if !entry.sourceID.isEmpty { Text(entry.sourceID).font(.caption.monospaced()).textSelection(.enabled) }
                if entry.sourceDiscarded { Text("来源已丢弃，保留这条历史记录供查阅。").foregroundStyle(.secondary) }
                else if entry.route(project: project) != nil {
                    Button("打开关联对话") { workspace.openResearchConversation(entry, project: project) }
                        .disabled(connection.state != .online)
                        .accessibilityIdentifier("research-open-conversation")
                } else { Text("这条记录没有关联对话。").foregroundStyle(.secondary) }
            }
        }.navigationTitle("研究记录").navigationBarTitleDisplayMode(.inline)
            .accessibilityIdentifier("research-entry-detail")
    }
    private func researchStatus(_ status: String) -> String {
        ["recorded": "已记录", "registered": "已登记", "submitted": "已提交", "started": "已开始",
         "running": "运行中", "succeeded": "已完成", "failed": "失败", "cancelled": "已取消", "timed_out": "超时",
         "lost": "失联", "archived": "已归档", "continued": "已继续", "reported": "已报告", "discussed": "已讨论"][status] ?? status
    }
}

private struct ResearchMonthControls: View {
    let month: Date
    let previous: () -> Void
    let next: () -> Void
    let today: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(month.formatted(.dateTime.year().month(.wide))).font(.title3.bold()).accessibilityIdentifier("research-month-title")
            HStack {
                Button(action: previous) { WispIcon(.chevronLeft).frame(minWidth: 44, minHeight: 44) }
                    .accessibilityLabel("上个月").accessibilityIdentifier("research-previous-month")
                Spacer()
                Button("今天", action: today).frame(minHeight: 44).accessibilityIdentifier("research-today")
                Spacer()
                Button(action: next) { WispIcon(.chevronRight).frame(minWidth: 44, minHeight: 44) }
                    .accessibilityLabel("下个月").accessibilityIdentifier("research-next-month")
            }.buttonStyle(.plain)
        }
    }
}

private struct ResearchLoadingStatus: View {
    @ObservedObject var loader: ResearchLoader
    var body: some View {
        if loader.busy { ProgressView("正在读取研究记录…") }
        if let error = loader.error {
            Text(error).foregroundStyle(WispStyle.warning)
            Text("下拉可重新读取。").font(.caption).foregroundStyle(.secondary)
        }
    }
}
