import SwiftUI
import WispProjectBrowser

struct NativeRecapDraft: Identifiable { let id = UUID(); let value: SettingsValue }
struct NativeJourneyRunID: Identifiable { let id: String; var kind = "run" }

enum NativeJourneyPresentation {
    static func label(_ key: String) -> String {
        localized(["draft": "草稿", "confirmed": "已确认", "dismissed": "已忽略",
                   "status": "状态", "context_id": "执行环境", "exit_code": "退出码",
                   "created_at": "创建时间", "started_at": "开始时间", "ended_at": "结束时间",
                   "command": "命令", "stdout_tail": "标准输出", "stderr_tail": "错误输出",
                   "queued": "排队中", "running": "运行中", "succeeded": "运行成功", "failed": "运行失败",
                   "cancelled": "已取消", "timed_out": "已超时", "run": "运行记录", "artifact": "产物",
                   "paper": "论文", "decision": "决策", "data_asset": "数据资产", "question": "研究问题",
                   "uses_evidence": "使用证据"][key] ?? key)
    }
    static func display(_ value: SettingsValue, key: String, calendar: Calendar = .current) -> String {
        if ["created_at", "started_at", "ended_at"].contains(key), case .integer(let timestamp) = value {
            return NativeCalendarClock.label(Date(timeIntervalSince1970: TimeInterval(timestamp)), format: "yyyy-MM-dd HH:mm:ss", calendar: calendar)
        }
        return key == "status" ? label(value.string) : value.string
    }
}

struct NativeJournalEditor: View {
    @ObservedObject var journey: NativeJourneyModel
    let client: any NativeSettingsQuerying
    let close: () -> Void
    @State private var owner: String?
    init(journey: NativeJourneyModel, client: any NativeSettingsQuerying, close: @escaping () -> Void) { self.journey = journey; self.client = client; self.close = close; self._owner = State(initialValue: journey.projectID) }
    @State private var title = ""
    @State private var text = ""
    @State private var category = "progress"
    @State private var date = Date()
    @State private var discard = false
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(localized("补充研究记录")).font(.title2.bold())
            DatePicker(localized("研究日期"), selection: $date, displayedComponents: .date)
            Picker(localized("记录类型"), selection: $category) { ForEach([("progress", "进展"), ("finding", "发现"), ("decision", "决策"), ("next", "下一步")], id: \.0) { key, label in Text(localized(label)).tag(key) } }
            TextField(localized("标题"), text: $title)
            Text(localized("详情与依据")); TextEditor(text: $text).frame(minHeight: 140)
            Text(localized("研究日期与补记时间会分别保存。")).font(.caption).foregroundStyle(.secondary)
            NativeJourneyWriteStatus(journey: journey, client: client)
            HStack { Button(localized("取消")) { dismiss() }.disabled(journey.writing); Spacer(); Button(localized("保存记录")) {
                let noon = journey.calendar.date(bySettingHour: 12, minute: 0, second: 0, of: date) ?? date
                let timestamp = Int64(noon.timeIntervalSince1970)
                let input: SettingsValue = .object(["title": .string(title), "body": .string(text), "category": .string(category), "occurred_at": .integer(timestamp)])
                Task { if await journey.mutate("native_research_journey_add", args: ["input": input], client: client, savedDay: timestamp, expectedProject: owner) { close() } }
            }.disabled(owner != journey.projectID || date.timeIntervalSince1970 < 0 || !journey.canWrite || title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || title.unicodeScalars.count > 200 || text.unicodeScalars.count > 10000) }
        }.padding(24).frame(minWidth: 390, idealWidth: 600, minHeight: 430)
        .onAppear { date = journey.clock }
        .interactiveDismissDisabled(!title.isEmpty || !text.isEmpty || journey.writing)
        .background(NativeSettingsEscape(enabled: !journey.writing && !discard) { dismiss() })
        .confirmationDialog(localized("放弃尚未保存的修改？"), isPresented: $discard) { Button(localized("放弃修改"), role: .destructive, action: close); Button(localized("继续编辑"), role: .cancel) {} }
    }
    private func dismiss() { if title.isEmpty && text.isEmpty { close() } else { discard = true } }
}

struct NativeJourneyWriteStatus: View {
    @ObservedObject var journey: NativeJourneyModel
    let client: any NativeSettingsQuerying
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let error = journey.error { Text(error).foregroundStyle(.orange).textSelection(.enabled) }
            if journey.uncertain {
                Text(localized("操作结果未确认。请刷新核对后继续，不会自动重试。"))
                HStack { Button(localized("刷新")) { Task { await journey.reload(client) } }.disabled(journey.busy || journey.writing); Button(localized("已核对结果，允许继续")) { journey.acknowledge() }.disabled(journey.busy || journey.writing) }
                ForEach(Array(journey.entries.filter(\.manual).prefix(8).enumerated()), id: \.offset) { _, row in Text(row.title + " · " + (row.summary ?? "")).font(.caption).textSelection(.enabled) }
                ForEach(Array(journey.recaps.enumerated()), id: \.offset) { _, recap in Text(recap["headline"].string + " · " + NativeJourneyPresentation.label(recap["status"].string)).font(.caption).textSelection(.enabled) }
            }
        }
    }
}

struct NativeRecapEditor: View {
    @ObservedObject var journey: NativeJourneyModel
    let draft: NativeRecapDraft
    @State private var owner: String?
    let client: any NativeSettingsQuerying
    let openSession: (String) -> Void
    let close: () -> Void
    @State private var value: SettingsValue
    @State private var discard = false
    @State private var run: NativeJourneyRunID?
    static let sections = [("done", "已完成"), ("findings", "发现"), ("issues", "问题"), ("next", "下一步")]
    init(journey: NativeJourneyModel, draft: NativeRecapDraft, client: any NativeSettingsQuerying, openSession: @escaping (String) -> Void, close: @escaping () -> Void) {
        self.journey = journey; self._owner = State(initialValue: journey.projectID); self.draft = draft; self.client = client; self.openSession = openSession; self.close = close; _value = State(initialValue: draft.value)
    }
    static func edit(_ value: SettingsValue, status: String) -> SettingsValue {
        var result: [String: SettingsValue] = ["id": value["id"], "headline": value["headline"], "status": .string(status)]
        for (key, _) in sections { result[key] = value[key] }
        return .object(result)
    }
    static func valid(_ value: SettingsValue) -> Bool {
        let headline = value["headline"].string
        return !headline.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && headline.unicodeScalars.count <= 200 && sections.allSatisfy { key, _ in
            let rows = value[key].array
            return rows.count <= 12 && rows.allSatisfy { !$0["text"].string.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && $0["text"].string.unicodeScalars.count <= 1000 }
        }
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack { Text(localized("每日研究回顾")).font(.title2.bold()); Spacer(); Button(localized("关闭")) { dismiss() }.disabled(journey.writing) }
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    TextField(localized("标题"), text: Binding(get: { value["headline"].string }, set: { value["headline"] = .string($0) }))
                    Text(NativeJourneyPresentation.label(value["status"].string)).font(.caption).foregroundStyle(.secondary)
                    ForEach(Self.sections, id: \.0) { key, label in
                        NativeSettingsGroup(title: label) {
                            ForEach(Array(value[key].array.enumerated()), id: \.offset) { index, row in
                                HStack(alignment: .top) {
                                    TextField(localized("记录内容"), text: Binding(get: { value[key].array.indices.contains(index) ? value[key].array[index]["text"].string : "" }, set: { text in var items = value[key].array; guard items.indices.contains(index) else { return }; items[index]["text"] = .string(text); value[key] = .array(items) }), axis: .vertical).lineLimit(1...8)
                                    Button { var items = value[key].array; items.remove(at: index); value[key] = .array(items) } label: { WispIcon(name: "trash") }.accessibilityLabel(localized("删除"))
                                }
                                if row["text"] == draft.value[key].array.first(where: { $0["text"] == row["text"] })?["text"] {
                                    ForEach(row["refs"].array.indices, id: \.self) { i in let ref = Int(row["refs"].array[i].integer); if draft.value["sources"].array.indices.contains(ref) { source(draft.value["sources"].array[ref]) } }
                                }
                            }
                            Button(localized("添加条目")) { var items = value[key].array; items.append(.object(["text": .string(""), "refs": .array([])])); value[key] = .array(items) }
                        }
                    }
                    Text(localized("改写的条目会清除原引用；来源记录保持不变。")).font(.caption).foregroundStyle(.secondary)
                    NativeSettingsGroup(title: "来源") { ForEach(Array(draft.value["sources"].array.enumerated()), id: \.offset) { _, row in source(row) } }
                    NativeJourneyWriteStatus(journey: journey, client: client)
                }.textFieldStyle(NativeSettingsTextFieldStyle())
            }
            if !Self.valid(value) { Text(localized("填写标题；每组最多 12 个非空条目，每条最多 1000 个字符。")).font(.caption).foregroundStyle(.orange) }
            HStack { Button(localized("忽略回顾")) { save("dismissed") }; Spacer(); Button(localized("保存草稿")) { save("draft") }; Button(localized("确认回顾")) { save("confirmed") } }.disabled(owner != journey.projectID || !journey.canWrite || !Self.valid(value))
        }.padding(24).frame(minWidth: 400, idealWidth: 660, minHeight: 540)
        .interactiveDismissDisabled(value != draft.value || journey.writing)
        .background(NativeSettingsEscape(enabled: !discard && run == nil && !journey.writing) { dismiss() })
        .confirmationDialog(localized("放弃尚未保存的修改？"), isPresented: $discard) { Button(localized("放弃修改"), role: .destructive, action: close); Button(localized("继续编辑"), role: .cancel) {} }
        .sheet(item: $run) { item in NativeJourneyRunView(id: item.id, projectID: owner ?? "", client: client, kind: item.kind) { run = nil } }
    }
    @ViewBuilder private func source(_ row: SettingsValue) -> some View {
        HStack {
            Text(row["title"].string).font(.caption).textSelection(.enabled)
            if row["kind"].string == "run" { Button(localized("运行记录")) { run = NativeJourneyRunID(id: row["id"].string) } }
            if row["kind"].string == "artifact" { Button(localized("查看产物与来源")) { run = NativeJourneyRunID(id: row["id"].string, kind: "artifact") } }
            if row["kind"].string == "session" { Button(localized("打开会话")) { guard owner == journey.projectID else { return }; close(); openSession(row["id"].string) } }
        }
    }
    private func dismiss() { if value != draft.value { discard = true } else { close() } }
    private func save(_ status: String) { let edit = Self.edit(value, status: status); Task { if await journey.mutate("native_research_journey_recap", args: ["edit": edit], client: client, expectedProject: owner) { close() } } }
}

struct NativeJourneyGraph: View {
    @ObservedObject var journey: NativeJourneyModel
    let client: any NativeSettingsQuerying
    @State private var selected: SettingsValue?
    @State private var run: NativeJourneyRunID?
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                if journey.graph == .null && journey.error == nil { ProgressView() }
                HStack { Text("\(journey.graph["nodes"].array.count) " + localized("节点") + " · \(journey.graph["edges"].array.count) " + localized("关系")); Spacer(); Button(localized("刷新")) { Task { await journey.loadGraph(client) } } }
                ForEach(Array(journey.graph["nodes"].array.filter { journey.query.isEmpty || $0["title"].string.localizedStandardContains(journey.query) }.enumerated()), id: \.offset) { _, node in
                    VStack(alignment: .leading, spacing: 10) {
                        Text(node["title"].string).font(.headline).textSelection(.enabled)
                        Text(NativeJourneyPresentation.label(node["kind"].string)).font(.caption).foregroundStyle(.secondary)
                        if node["kind"].string == "run", !node["ref_id"].string.isEmpty { Button(localized("运行记录")) { run = NativeJourneyRunID(id: node["ref_id"].string) } }
                        ForEach(Array(journey.graph["edges"].array.filter { $0["source_id"] == node["id"] || $0["target_id"] == node["id"] }.enumerated()), id: \.offset) { _, edge in
                            Button { selected = edge } label: { Text(title(edge["source_id"]) + " → " + NativeJourneyPresentation.label(edge["relation"].string) + " → " + title(edge["target_id"])) }.buttonStyle(.link)
                        }
                    }.padding(14).frame(maxWidth: .infinity, alignment: .leading).background(Color.secondary.opacity(0.06), in: RoundedRectangle(cornerRadius: 8))
                }
                if journey.graph != .null && journey.graph["nodes"].array.isEmpty { Text(localized("暂无已记录的研究关系。")) }
            }
        }
        .sheet(isPresented: Binding(get: { selected != nil }, set: { if !$0 { selected = nil } })) {
            VStack(alignment: .leading, spacing: 16) {
                Text(localized("关系详情")).font(.title2.bold())
                if let edge = selected { Text(title(edge["source_id"]) + " → " + title(edge["target_id"])); Text(NativeJourneyPresentation.label(edge["relation"].string)); Text(edge["metadata_json"].string).font(.system(.body, design: .monospaced)).textSelection(.enabled) }
                Button(localized("关闭")) { selected = nil }
            }.padding(24).frame(minWidth: 380, idealWidth: 580, minHeight: 250).background(NativeSettingsEscape { selected = nil })
        }
        .sheet(item: $run) { item in NativeJourneyRunView(id: item.id, projectID: journey.projectID ?? "", client: client) { run = nil } }
    }
    private func title(_ id: SettingsValue) -> String { journey.graph["nodes"].array.first { $0["id"] == id }?["title"].string ?? id.string }
}

struct NativeJourneyRunView: View {
    let id: String
    let projectID: String
    let client: any NativeSettingsQuerying
    var kind = "run"
    let close: () -> Void
    @State private var result: SettingsValue = .null
    @State private var error: String?
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack { Text(localized(kind == "artifact" ? "查看产物与来源" : "运行记录")).font(.title2.bold()); Spacer(); Button(localized("关闭"), action: close) }
            if let error { Text(error).foregroundStyle(.orange) }
            else if result == .null { ProgressView() }
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    Text(kind == "artifact" ? result["filename"].string : result["title"].string).font(.headline)
                    if kind == "artifact" {
                        if result["content_error"] != .null { Text(result["content_error"].string).foregroundStyle(.orange) }
                        Text(result["source"]["run_title"].string).font(.caption)
                        if result["truncated"].bool { Text(localized("内容仅显示前缀。")).font(.caption) }
                        Text(result["text"].string).font(.system(.body, design: .monospaced)).textSelection(.enabled)
                        if result["mime"].string.hasPrefix("image/"), let data = Data(base64Encoded: result["base64"].string), let image = NSImage(data: data) { Image(nsImage: image).resizable().scaledToFit() }
                    }
                    ForEach(["status", "context_id", "exit_code", "created_at", "started_at", "ended_at", "command", "stdout_tail", "stderr_tail"], id: \.self) { key in
                        if result[key] != .null { Text(NativeJourneyPresentation.label(key)).fontWeight(.semibold); Text(NativeJourneyPresentation.display(result[key], key: key)).font(.system(.body, design: .monospaced)).textSelection(.enabled) }
                    }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }
        }.padding(24).frame(minWidth: 390, idealWidth: 680, minHeight: 480)
        .background(NativeSettingsEscape(close: close))
        .task(id: [projectID, kind, id].joined(separator: "|")) {
            do { let value = try await client.invoke(kind == "artifact" ? "native_research_journey_artifact" : "native_research_journey_run", args: [kind == "artifact" ? "version_id" : "run_id": .string(id)], projectID: projectID); guard !Task.isCancelled else { return }; guard value[kind == "artifact" ? "version_id" : "id"].string == id else { throw ProjectBrowserError.invalidResponse }; result = value }
            catch { if !Task.isCancelled { self.error = error.localizedDescription } }
        }
    }
}
