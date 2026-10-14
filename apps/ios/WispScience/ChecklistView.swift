import SwiftUI
import WispRemote

struct ChecklistView: View {
    @ObservedObject var workspace: RemoteWorkspace
    @ObservedObject var model: ChecklistModel
    @ObservedObject var connection: RemoteConnection
    @State private var title = ""
    @State private var date = Date()
    @State private var showDone = false

    var body: some View {
        NavigationStack(path: $workspace.checklistPath) {
            List {
                Section("保存与同步") {
                    Text("离线记录，联网同步到科研助理的研究计划。").foregroundStyle(.secondary)
                    if model.scopes.count > 1 {
                        ScrollView(.horizontal) {
                            HStack {
                                ForEach(model.scopes, id: \.0) { id, name in
                                    Button(name) { model.select(id) }
                                        .buttonStyle(.bordered).tint(id == model.scope ? WispStyle.accent : .secondary)
                                }
                            }
                        }
                    }
                    Text(model.scopeName).font(.headline)
                    Text(model.syncing ? "正在同步…" : model.pendingCount > 0 ? "\(model.pendingCount) 项更改保存在本机，等待同步" : "已保存在本机")
                        .font(.caption).foregroundStyle(.secondary)
                    if let error = model.error { Text(error).font(.caption).foregroundStyle(WispStyle.warning) }
                    if model.canSyncSelection && connection.state == .online {
                        Button("立即同步") { Task { await model.sync(connection: connection) } }.disabled(model.syncing)
                    }
                }
                Section("添加科研事项") {
                    TextField("例如：核对样本编号", text: $title, axis: .vertical).accessibilityIdentifier("checklist-title")
                    DatePicker("计划日期", selection: $date, displayedComponents: .date)
                    Button("保存到清单") {
                        if model.edit(fields: ChecklistFields(title: title.trimmingCharacters(in: .whitespacesAndNewlines), day: ChecklistDate.string(date))) {
                            title = ""; Task { await model.sync(connection: connection) }
                        }
                    }.disabled(!model.loaded || title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        .accessibilityIdentifier("checklist-add")
                }
                Section("我的科研事项") {
                    Toggle("显示已完成", isOn: $showDone)
                    if model.entries.isEmpty { Text("先记下今天要推进的一件事。").foregroundStyle(.secondary) }
                    ForEach(model.entries.filter { showDone || $0.fields.status != "done" || $0.conflict }) { entry in
                        HStack(alignment: .top, spacing: 12) {
                            Button {
                                var fields = entry.fields; fields.status = fields.status == "done" ? "open" : "done"
                                if model.edit(id: entry.id, fields: fields) { Task { await model.sync(connection: connection) } }
                            } label: { WispIcon(entry.fields.status == "done" ? .checkSquare : .square).frame(minWidth: 44, minHeight: 44) }
                                .buttonStyle(.plain).disabled(entry.conflict)
                                .accessibilityLabel(entry.fields.status == "done" ? "标记未完成" : "标记完成")
                                .accessibilityIdentifier("checklist-toggle-\(entry.id)")
                            NavigationLink(value: entry.id) {
                                VStack(alignment: .leading, spacing: 5) {
                                    Text(entry.fields.title).strikethrough(entry.fields.status == "done")
                                    Text(entry.fields.day + (entry.fields.status == "open" && entry.fields.day < ChecklistDate.string(Date()) ? " · 已逾期" : ""))
                                        .font(.caption).foregroundStyle(.secondary)
                                    if entry.conflict { Text("两端有不同修改，待处理").font(.caption).foregroundStyle(WispStyle.warning) }
                                }
                            }.accessibilityIdentifier("checklist-item-\(entry.id)")
                        }
                    }
                }
            }
            .listStyle(.insetGrouped)
            .scrollDismissesKeyboard(.interactively)
            .navigationTitle("科研清单").navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if workspace.checklistPath.isEmpty {
                    ToolbarItem(placement: .topBarLeading) { WorkspacePageBackButton(workspace: workspace) }
                }
            }
            .navigationDestination(for: String.self) { id in
                if let entry = model.replica.entries[id], model.replica.visible(entry) {
                    ChecklistEditor(model: model, workspace: workspace, connection: connection, entry: entry)
                } else { Text("这项记录已移除或当前不可见。") }
            }
        }.accessibilityIdentifier("checklist-page")
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
