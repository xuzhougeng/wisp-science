import SwiftUI
import WispProjectBrowser

@MainActor final class NativeExplorationsModel: ObservableObject {
    @Published private(set) var rows: [SettingsValue] = []
    @Published private(set) var preview: SettingsValue?
    @Published private(set) var busy = false
    @Published private(set) var writing = false
    @Published private(set) var uncertain = false
    @Published private(set) var refreshed = false
    @Published private(set) var error: String?
    let client: any NativeConversationQuerying
    let project: String
    private var generation = UUID()
    init(client: any NativeConversationQuerying, project: String) { self.client = client; self.project = project }
    var canWrite: Bool { !busy && !uncertain && refreshed }
    func close() { generation = UUID(); busy = false; writing = false }
    func acknowledge() { if refreshed && !busy { uncertain = false; error = nil } }
    func load() async {
        guard !busy else { return }
        let token = generation; busy = true; refreshed = false; preview = nil
        defer { if generation == token { busy = false } }
        do {
            let value = try await client.invoke("native_conversation_explorations", args: [:], projectID: project)
            guard generation == token else { return }
            guard value["project_id"].string == project,
                  value["explorations"].array.allSatisfy({ !$0["exploration"]["id"].string.isEmpty && !$0["exploration"]["frame_id"].string.isEmpty && !$0["source_frame_id"].string.isEmpty }) else { throw ProjectBrowserError.invalidResponse }
            rows = value["explorations"].array; refreshed = true
            if !uncertain { error = nil }
        } catch { if generation == token { self.error = error.localizedDescription } }
    }
    func inspect(_ row: SettingsValue) async {
        guard !busy else { return }
        let token = generation; busy = true; preview = nil
        defer { if generation == token { busy = false } }
        do {
            let result = try await client.invoke("native_conversation_exploration_preview", args: arguments(row), projectID: project)
            guard generation == token else { return }
            guard result["project_id"].string == project, result["session_id"] == row["exploration"]["frame_id"], result["preview"]["exploration"]["id"] == row["exploration"]["id"] else { throw ProjectBrowserError.invalidResponse }
            preview = result; error = nil
        } catch { if generation == token { self.error = error.localizedDescription } }
    }
    func arguments(_ row: SettingsValue) -> [String: SettingsValue] {
        ["session_id": row["exploration"]["frame_id"], "exploration_id": row["exploration"]["id"]]
    }
    @discardableResult func mutate(_ action: String, args: [String: SettingsValue]) async -> Bool {
        guard canWrite, ["start", "promote", "discard", "manual_open", "manual_finish", "abandon"].contains(action) else { return false }
        let token = generation; busy = true; writing = true; uncertain = true; refreshed = false; error = nil
        do {
            let value = try await client.invoke("native_conversation_exploration_" + action, args: args, projectID: project)
            guard generation == token else { return false }
            guard value["project_id"].string == project,
                  (action == "start"
                    ? value["source_session_id"] == args["session_id"] && value["target"] == args["target"] && !value["summary"]["exploration"]["id"].string.isEmpty
                    : value["completed"].bool && value["action"].string == action && value["session_id"] == args["session_id"] && (action == "abandon" || value["exploration_id"] == args["exploration_id"])) else { throw ProjectBrowserError.invalidResponse }
            busy = false; writing = false; uncertain = false; await load(); return true
        } catch {
            if generation == token { busy = false; writing = false; self.error = localized("探索操作结果未确认，请刷新核对；不会自动重试。") + "\n" + error.localizedDescription }
            return false
        }
    }
}

struct NativeExplorationsView: View {
    @StateObject private var model: NativeExplorationsModel
    let session: String
    let turns: [ConversationTurnIdentity]
    let close: () -> Void
    let open: (String) -> Void
    @State private var name = ""
    @State private var turn = -1
    @State private var pending: String?
    @State private var pendingArgs: [String: SettingsValue] = [:]
    init(client: any NativeConversationQuerying, project: String, session: String, turns: [ConversationTurnIdentity], close: @escaping () -> Void, open: @escaping (String) -> Void) {
        _model = StateObject(wrappedValue: NativeExplorationsModel(client: client, project: project))
        self.session = session; self.turns = turns; self.close = close; self.open = open
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack { Text(localized("探索与恢复")).font(.title2.bold()); Spacer(); Button(localized("刷新")) { Task { await model.load() } }.disabled(model.busy); Button(localized("关闭"), action: close).disabled(model.writing) }
            if model.busy { ProgressView() }
            if let error = model.error { Text(error).foregroundStyle(.orange).textSelection(.enabled) }
            if model.uncertain { Button(localized("已核对结果，允许继续")) { model.acknowledge() }.disabled(!model.refreshed || model.busy) }
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if model.rows.isEmpty {
                        Text(localized("当前项目没有未解决的探索。"))
                    }
                    ForEach(Array(model.rows.enumerated()), id: \.offset) { _, row in
                        candidate(row)
                    }
                    if let preview = model.preview { previewDetails(preview) }
                    if !turns.isEmpty && !model.rows.contains(where: { $0["exploration"]["frame_id"].string == session }) {
                        Divider()
                        Text(localized("从当前会话开始探索")).font(.headline)
                        TextField(localized("探索名称"), text: $name)
                        Picker(localized("起点"), selection: $turn) {
                            Text(localized("最新一轮")).tag(-1)
                            ForEach(turns, id: \.user_index) { item in Text(localized("起点") + " · \(item.user_index + 1)").tag(item.user_index) }
                        }
                        Button(localized("开始探索")) {
                            guard let target = turns.first(where: { $0.user_index == turn }) ?? turns.last,
                                  let value = try? JSONDecoder().decode(SettingsValue.self, from: JSONEncoder().encode(target)) else { return }
                            confirm("start", ["session_id": .string(session), "target": value, "name": .string(name)])
                        }.disabled(!model.canWrite || name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }
            }
        }.padding(24).frame(minWidth: 560, idealWidth: 760, minHeight: 480, idealHeight: 680)
            .interactiveDismissDisabled(model.writing)
            .background(NativeSettingsEscape(enabled: !model.writing && pending == nil, close: close))
            .confirmationDialog(localized("确认探索操作"), isPresented: Binding(get: { pending != nil }, set: { if !$0 { pending = nil } })) {
                if let action = pending {
                    Button(actionLabel(action), role: ["abandon", "discard"].contains(action) ? .destructive : nil) {
                        let args = pendingArgs; pending = nil
                        Task { await model.mutate(action, args: args) }
                    }
                }
                Button(localized("取消"), role: .cancel) { pending = nil }
            } message: { Text(localized("采用会改变主线；丢弃或放弃会结束相应探索。请先核对变更和外部影响。")) }
            .task { await model.load() }.onDisappear { model.close() }
    }
    private func confirm(_ action: String, _ args: [String: SettingsValue]) { pendingArgs = args; pending = action }
    private func actionLabel(_ action: String) -> String { localized(["start": "开始探索", "promote": "采用探索", "discard": "丢弃候选", "manual_open": "手动整理", "manual_finish": "完成手动整理", "abandon": "放弃整轮探索"][action] ?? action) }
    private func candidate(_ row: SettingsValue) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(row["exploration"]["name"].string).font(.headline)
            Text(row["exploration"]["status"].string).font(.caption)
            HStack {
                Button(localized("打开探索")) { open(row["exploration"]["frame_id"].string) }
                Button(localized("查看变更")) { Task { await model.inspect(row) } }
                if ["active", "failed"].contains(row["exploration"]["status"].string) {
                    Button(actionLabel("discard")) { confirm("discard", model.arguments(row)) }.disabled(!model.canWrite)
                }
                Button(actionLabel("abandon")) { confirm("abandon", ["session_id": row["source_frame_id"]]) }.disabled(!model.canWrite)
            }.disabled(model.busy)
        }.padding(12).frame(maxWidth: .infinity, alignment: .leading).background(.quaternary, in: RoundedRectangle(cornerRadius: 10))
    }
    private func previewDetails(_ value: SettingsValue) -> some View {
        let preview = value["preview"]
        let eligibility = preview["eligibility"]
        let args = ["session_id": value["session_id"], "exploration_id": preview["exploration"]["id"]]
        return VStack(alignment: .leading, spacing: 10) {
            Text(localized("变更预览")).font(.headline)
            ForEach(Array(preview["diff"]["files"].array.enumerated()), id: \.offset) { _, file in Text(file["kind"].string + " · " + file["path"].string).textSelection(.enabled) }
            DisclosureGroup(localized("产物、运行与外部影响")) { NativeSettingsSummary(value: preview["diff"]) }
            ForEach(Array(eligibility["reasons"].array.enumerated()), id: \.offset) { _, reason in Text(reason["message"].string).foregroundStyle(.orange) }
            HStack {
                Button(actionLabel("promote")) { var guarded = args; guarded["expected_guard_hash"] = eligibility["expectedGuardHash"]; confirm("promote", guarded) }.disabled(!model.canWrite || !eligibility["eligible"].bool)
                if eligibility["manualResolutionAvailable"].bool {
                    Button(actionLabel("manual_open")) { confirm("manual_open", args) }.disabled(!model.canWrite)
                    Button(actionLabel("manual_finish")) { confirm("manual_finish", args) }.disabled(!model.canWrite)
                }
            }
        }.padding(12)
    }
}
