import SwiftUI
import WispRemote

struct ChecklistView: View {
    @ObservedObject var workspace: RemoteWorkspace
    @ObservedObject var model: ChecklistModel
    @ObservedObject var connection: RemoteConnection
    @State private var title = ""
    @State private var showDone = false
    @FocusState private var adding: Bool

    var body: some View {
        NavigationStack(path: $workspace.checklistPath) {
            // Reclassify tasks after midnight as well as when the data changes.
            TimelineView(.periodic(from: .now, by: 60)) { context in
                taskList(today: ChecklistDate.string(context.date))
            }
            .safeAreaInset(edge: .bottom, spacing: 0) { quickAdd }
            .navigationTitle("科研清单").navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if workspace.checklistPath.isEmpty {
                    ToolbarItem(placement: .topBarTrailing) {
                        WispToolbarButton(title: "清单设置", icon: .gear) { workspace.sheet = .checklistSettings }
                            .accessibilityIdentifier("checklist-settings")
                    }
                }
            }
            .navigationDestination(for: String.self) { id in
                if let entry = model.replica.entries[id], model.replica.visible(entry) {
                    ChecklistEditor(model: model, workspace: workspace, connection: connection, entry: entry)
                } else { Text("这项记录已移除或当前不可见。") }
            }
        }.accessibilityElement(children: .contain).accessibilityIdentifier("checklist-page")
            .onChange(of: model.scope) { _, _ in showDone = false }
    }

    private func taskList(today: String) -> some View {
        let sections = ChecklistSections(entries: model.entries, today: today)
        return List {
            HStack(alignment: .firstTextBaseline) {
                Text(model.scopeName).lineLimit(1).truncationMode(.middle)
                Spacer(minLength: 12)
                Text(model.loaded ? "\(sections.remainingCount) 项待办" : "暂不可用").monospacedDigit().fixedSize()
            }
            .font(.subheadline).foregroundStyle(.secondary)
            .listRowSeparator(.hidden).accessibilityElement(children: .combine)
            .accessibilityIdentifier("checklist-summary")

            if let error = model.error {
                Button { workspace.sheet = .checklistSettings } label: {
                    HStack(alignment: .top, spacing: 8) {
                        WispIcon(.circleAlert, size: 16)
                        Text(error).font(.caption).lineLimit(2).frame(maxWidth: .infinity, alignment: .leading)
                        WispIcon(.chevronRight, size: 12)
                    }.foregroundStyle(WispStyle.warning).padding(.vertical, 6)
                }
                .buttonStyle(.plain).accessibilityHint("打开清单设置，查看完整提示")
                .accessibilityIdentifier("checklist-notice")
            }

            if model.loaded {
                taskSection("待确认", entries: sections.needsReview, today: today, warning: true)
                taskSection("逾期", entries: sections.overdue, today: today, warning: true)
                taskSection("今天", entries: sections.today, today: today)
                taskSection("接下来", entries: sections.upcoming, today: today)
                if sections.remainingCount == 0 {
                    VStack(spacing: 12) {
                        WispIcon(sections.done.isEmpty ? .list : .circleCheck, size: 36).foregroundStyle(WispStyle.accent)
                        Text(sections.done.isEmpty ? "还没有待办事项" : "待办都完成了").font(.headline)
                        Text("在下方记下一件要做的事。\n新事项默认安排在今天，之后可点开改期。")
                            .font(.subheadline).foregroundStyle(.secondary).multilineTextAlignment(.center)
                    }
                    .frame(maxWidth: .infinity).padding(.vertical, 32)
                    .listRowSeparator(.hidden).accessibilityIdentifier("checklist-empty")
                }
                if !sections.done.isEmpty {
                    Section {
                        Button { showDone.toggle() } label: {
                            HStack(spacing: 8) {
                                WispIcon(.chevronRight, size: 14).rotationEffect(.degrees(showDone ? 90 : 0))
                                Text("已完成 · \(sections.done.count)")
                                Spacer()
                            }.font(.subheadline).foregroundStyle(.secondary).frame(minHeight: 44).contentShape(Rectangle())
                        }
                        .buttonStyle(.plain).accessibilityIdentifier("checklist-show-done")
                        .accessibilityValue(showDone ? "已展开" : "已收起")
                        .accessibilityHint(showDone ? "收起已完成事项" : "查看已完成事项，可重新标记未完成")
                        if showDone {
                            ForEach(sections.done) { entry in taskRow(entry, today: today) }
                        }
                    }
                }
            }
        }
        .listStyle(.plain).scrollContentBackground(.hidden).background(WispStyle.canvas)
        .scrollDismissesKeyboard(.interactively)
    }

    @ViewBuilder private func taskSection(_ title: String, entries: [ChecklistEntry], today: String, warning: Bool = false) -> some View {
        if !entries.isEmpty {
            Section {
                ForEach(entries) { entry in taskRow(entry, today: today) }
            } header: {
                HStack(spacing: 8) {
                    Text(title).font(.headline)
                    Text("\(entries.count)").font(.subheadline.monospacedDigit())
                }.textCase(nil).foregroundStyle(warning ? WispStyle.warning : Color.primary)
                    .accessibilityIdentifier("checklist-section-\(title)")
            }
        }
    }

    private func taskRow(_ entry: ChecklistEntry, today: String) -> some View {
        HStack(alignment: .top, spacing: 6) {
            Button {
                var fields = entry.fields
                fields.status = fields.status == "done" ? "open" : "done"
                if model.edit(id: entry.id, fields: fields) { Task { await model.sync(connection: connection) } }
            } label: {
                WispIcon(entry.conflict ? .circleAlert : entry.fields.status == "done" ? .circleCheck : .circle, size: 24)
                    .foregroundStyle(entry.conflict ? WispStyle.warning : WispStyle.accent)
                    .frame(minWidth: 44, minHeight: 44).contentShape(Rectangle())
            }
            .buttonStyle(.plain).disabled(entry.conflict)
            .accessibilityLabel(entry.conflict ? "待确认" : entry.fields.status == "done" ? "标记未完成" : "标记完成")
            .accessibilityValue(entry.fields.title)
            .accessibilityIdentifier("checklist-toggle-\(entry.id)")

            Button { workspace.checklistPath.append(entry.id) } label: {
                VStack(alignment: .leading, spacing: 5) {
                    Text(entry.fields.title).font(.body)
                        .strikethrough(entry.fields.status == "done" && !entry.conflict)
                        .foregroundStyle(entry.fields.status == "done" && !entry.conflict ? Color.secondary : Color.primary)
                        .fixedSize(horizontal: false, vertical: true)
                    if entry.fields.day != today {
                        Text(entry.fields.day).font(.caption)
                            .foregroundStyle(entry.fields.day < today && entry.fields.status == "open" ? WispStyle.warning : Color.secondary)
                    }
                    if entry.conflict {
                        Text("两端有不同修改，点此确认").font(.caption).foregroundStyle(WispStyle.warning)
                    }
                }.frame(maxWidth: .infinity, minHeight: 44, alignment: .leading).contentShape(Rectangle())
            }
            .buttonStyle(.plain).accessibilityIdentifier("checklist-item-\(entry.id)")
            .accessibilityHint(entry.conflict ? "查看并处理两个版本" : "编辑事项、调整日期或移出清单")
        }
        .padding(.vertical, 4)
        .listRowInsets(EdgeInsets(top: 4, leading: 12, bottom: 4, trailing: 20))
    }

    private var quickAdd: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 10) {
                WispIcon(.plus).foregroundStyle(WispStyle.accent)
                TextField("添加科研事项", text: $title)
                    .focused($adding).submitLabel(.done).onSubmit(addItem)
                    .frame(minHeight: 44).accessibilityIdentifier("checklist-title")
                Button("添加", action: addItem)
                    .buttonStyle(WispSendButtonStyle())
                    .disabled(title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .accessibilityIdentifier("checklist-add")
            }
            Text(adding ? "默认安排在今天，点事项可改期" : "点圆圈完成，点事项查看或改期")
                .font(.caption).foregroundStyle(.secondary)
        }
        .disabled(!model.loaded)
        .padding(.horizontal, 16).padding(.vertical, 8).background(WispStyle.canvas)
        .overlay(alignment: .top) { Rectangle().fill(WispStyle.border).frame(height: 0.5) }
    }

    private func addItem() {
        let value = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty, model.edit(fields: ChecklistFields(title: value, day: ChecklistDate.string(Date()))) else { return }
        title = ""; adding = false
        Task { await model.sync(connection: connection) }
    }
}

struct ChecklistSettingsView: View {
    @ObservedObject var workspace: RemoteWorkspace
    @ObservedObject var model: ChecklistModel
    @ObservedObject var connection: RemoteConnection

    var body: some View {
        NavigationStack {
            Form {
                Section("当前清单") {
                    ForEach(model.scopes, id: \.0) { id, name in
                        Button { model.select(id) } label: {
                            HStack {
                                Text(name).foregroundStyle(.primary)
                                Spacer()
                                if id == model.scope { WispIcon(.check).foregroundStyle(WispStyle.accent) }
                            }.frame(minHeight: 44).contentShape(Rectangle())
                        }
                        .accessibilityValue(id == model.scope ? "已选中" : "未选中")
                        .accessibilityIdentifier("checklist-scope-\(id)")
                    }
                }
                Section {
                    Text(!model.loaded ? "本机清单尚未成功读取，暂不能修改。" : model.syncing ? "正在同步…" :
                         model.pendingCount > 0 ? "\(model.pendingCount) 项更改保存在本机，等待同步" : "已保存在本机")
                    if let error = model.error { Text(error).foregroundStyle(WispStyle.warning).textSelection(.enabled) }
                    if model.canSyncSelection && connection.state == .online && connection.supports("native_research_checklist_sync") {
                        Button("立即同步") { Task { await model.sync(connection: connection) } }
                            .disabled(!model.loaded || model.syncing).accessibilityIdentifier("checklist-sync")
                    }
                } header: {
                    Text("保存与同步")
                } footer: {
                    Text("离线修改先保存在手机，连接对应主机后自动同步到科研助理的研究计划。本机新事项会在首次成功连接时归入该主机；不同主机的清单分别保存。")
                }
            }
            .navigationTitle("清单设置").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .topBarTrailing) { Button("完成") { workspace.sheet = nil } } }
        }.accessibilityIdentifier("checklist-settings-sheet")
    }
}

private struct ChecklistEditor: View {
    @ObservedObject var model: ChecklistModel
    @ObservedObject var workspace: RemoteWorkspace
    @ObservedObject var connection: RemoteConnection
    let entry: ChecklistEntry
    @State private var title = ""
    @State private var date = Date()
    var body: some View {
        Form {
            if entry.conflict {
                Section("手机版本") { Text(entry.fields.title); Text(entry.fields.day + " · " + status(entry.fields.status)) }
                Section("主机版本") {
                    if let server = entry.server { Text(server.title); Text(server.day + " · " + status(server.status)) }
                    else { Text("主机中未找到这项记录，可能已恢复或更换数据库。") }
                    Button("保留手机版本") { resolve(true) }.accessibilityIdentifier("checklist-keep-local")
                    Button("采用主机版本") { resolve(false) }.accessibilityIdentifier("checklist-use-host")
                }
            } else {
                Section("科研事项") {
                    TextField("标题", text: $title, axis: .vertical)
                    DatePicker("计划日期", selection: $date, displayedComponents: .date)
                    Button("保存修改") {
                        if model.edit(id: entry.id, fields: ChecklistFields(title: title.trimmingCharacters(in: .whitespacesAndNewlines), day: ChecklistDate.string(date), status: entry.fields.status)) {
                            workspace.checklistPath.removeLast(); Task { await model.sync(connection: connection) }
                        }
                    }
                    Button("移出清单", role: .destructive) {
                        var fields = entry.fields; fields.status = "dropped"
                        if model.edit(id: entry.id, fields: fields) { workspace.checklistPath.removeLast(); Task { await model.sync(connection: connection) } }
                    }
                }
            }
            if let error = model.error { Text(error).foregroundStyle(WispStyle.warning) }
        }.navigationTitle("清单事项").navigationBarTitleDisplayMode(.inline)
            .onAppear { title = entry.fields.title; date = ChecklistDate.date(entry.fields.day) ?? Date() }
    }
    private func status(_ value: String) -> String { ["open": "未完成", "done": "已完成", "dropped": "已移出"][value] ?? value }
    private func resolve(_ local: Bool) {
        model.resolve(entry.id, keepLocal: local)
        if model.error == nil { workspace.checklistPath.removeLast(); Task { await model.sync(connection: connection) } }
    }
}

enum ChecklistDate {
    private static var formatter: DateFormatter {
        let value = DateFormatter(); value.calendar = RemoteResearchClock.localCalendar()
        value.timeZone = .autoupdatingCurrent; value.locale = Locale(identifier: "en_US_POSIX"); value.dateFormat = "yyyy-MM-dd"
        return value
    }
    static func string(_ date: Date) -> String { formatter.string(from: date) }
    static func date(_ value: String) -> Date? { formatter.date(from: value) }
}
