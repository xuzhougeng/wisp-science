import SwiftUI
import WispProjectBrowser

@MainActor
final class NativeHooksModel: ObservableObject {
    let client: any NativeSettingsQuerying
    let projectID: String?
    @Published private(set) var values: [String: SettingsValue] = [:]
    @Published private(set) var busy = false
    @Published private(set) var uncertain = false
    @Published private(set) var refreshed = false
    @Published private(set) var error: String?
    private var generation = UUID()
    private var active = true
    private var writing = false
    init(client: any NativeSettingsQuerying, projectID: String?) { self.client = client; self.projectID = projectID }
    var hooks: [SettingsValue] { values["get_command_hooks"]?.array ?? [] }
    var project: SettingsValue { values["get_project_hooks"] ?? .null }
    var canWrite: Bool { active && !busy && !uncertain && refreshed }
    func close() { active = false; generation = UUID(); if !writing { busy = false } }
    func load() async {
        guard !busy && !writing else { return }
        active = true; busy = true; refreshed = false; error = nil
        let token = generation
        var next: [String: SettingsValue] = [:]
        do {
            for command in ["get_command_hooks", "get_auto_review_enabled", "get_auto_failure_analysis_settings", "get_project_hooks"] {
                next[command] = try await client.invoke(command, args: [:], projectID: projectID)
                guard active, token == generation else { return }
            }
            guard case .array = next["get_command_hooks"], case .bool = next["get_auto_review_enabled"], case .object = next["get_auto_failure_analysis_settings"] else { throw ProjectBrowserError.invalidResponse }
            values = next; refreshed = true; busy = false
        } catch { if active, token == generation { busy = false; self.error = error.localizedDescription } }
    }
    func acknowledge() { guard uncertain, refreshed, !busy, !writing else { return }; uncertain = false; error = nil }
    @discardableResult func write(_ command: String, args: [String: SettingsValue]) async -> Bool {
        guard canWrite else { return false }
        let token = generation
        busy = true; writing = true; uncertain = true; refreshed = false; error = nil
        do {
            let result = try await client.invoke(command, args: args, projectID: projectID)
            writing = false; busy = false
            guard active, token == generation else { return false }
            switch command {
            case "set_command_hooks": guard case .array = result, result.array.count == args["hooks"]?.array.count else { throw ProjectBrowserError.invalidResponse }
            case "set_auto_review_enabled": guard result == args["enabled"] else { throw ProjectBrowserError.invalidResponse }
            case "set_project_hooks_trust":
                if let hash = args["sha256"], hash != .null {
                    guard result["sha256"] == hash, result["trusted"].bool else { throw ProjectBrowserError.invalidResponse }
                } else { guard result == .null || !result["trusted"].bool else { throw ProjectBrowserError.invalidResponse } }
            default: guard case .object = result else { throw ProjectBrowserError.invalidResponse }
            }
            uncertain = false; busy = false
            await load()
            return true
        } catch {
            writing = false; busy = false
            if active, token == generation { busy = false; self.error = localized("操作结果未确认。请刷新核对后继续，不会自动重试。") + "\n" + error.localizedDescription }
            return false
        }
    }
    func saveHook(_ value: SettingsValue, at index: Int?, expected: SettingsValue? = nil) async -> Bool {
        guard !value["command"].string.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return false }
        var rows = hooks
        if let index { guard rows.indices.contains(index), expected == nil || rows[index] == expected else { error = localized("此钩子已变化，请关闭编辑器并重新打开。"); return false }; rows[index] = value } else { rows.append(value) }
        return await write("set_command_hooks", args: ["hooks": .array(rows)])
    }
}

struct NativeHookDraft: Identifiable {
    let id = UUID()
    var index: Int?
    var value: SettingsValue
    static var empty: SettingsValue { .object(["event": .string("PreToolUse"), "matcher": .string(""), "command": .string(""), "enabled": .bool(true), "timeout": .integer(60)]) }
}

struct NativeHooksSettings: View {
    @StateObject private var model: NativeHooksModel
    let openReviewer: (() -> Void)?
    @State private var editor: NativeHookDraft?
    @State private var removal: Int?
    @State private var trustHash: String?
    init(model: NativeHooksModel, openReviewer: (() -> Void)? = nil) { _model = StateObject(wrappedValue: model); self.openReviewer = openReviewer }
    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            Text(localized("钩子在 Agent 生命周期事件中运行。命令通过 stdin 接收事件 JSON。"))
            Text(localized("工具类钩子仅作用于内置 Agent；ACP Agent 自行执行工具，只会触发 UserPromptSubmit 和 Stop。")).font(.caption).foregroundStyle(.secondary)
            HStack { Button(localized("刷新")) { Task { await model.load() } }.disabled(model.busy); if model.busy { ProgressView().controlSize(.small) } }
            if let error = model.error { Text(error).foregroundStyle(.orange).textSelection(.enabled) }
            if model.uncertain { Button(localized("已核对结果，允许继续")) { model.acknowledge() }.disabled(!model.refreshed || model.busy) }
            NativeSettingsGroup(title: "默认钩子") {
                Toggle(localized("自动审查（新会话默认值）"), isOn: Binding(get: { model.values["get_auto_review_enabled"]?.bool ?? false }, set: { on in Task { await model.write("set_auto_review_enabled", args: ["enabled": .bool(on)]) } }))
                if let openReviewer { Button(localized("审查模型与专家设置"), action: openReviewer) }
                NativeFailureHookEditor(model: model)
            }.disabled(!model.canWrite)
            NativeSettingsGroup(title: "命令钩子") {
                Button(localized("新建钩子")) { editor = NativeHookDraft(value: NativeHookDraft.empty) }.disabled(!model.canWrite)
                if model.hooks.isEmpty { Text(localized("尚未添加命令钩子。")).foregroundStyle(.secondary) }
                ForEach(Array(model.hooks.enumerated()), id: \.offset) { index, hook in
                    VStack(alignment: .leading, spacing: 8) {
                        HStack {
                            Text(hook["event"].string).fontWeight(.semibold); Spacer()
                            Toggle(localized("启用"), isOn: Binding(get: { hook["enabled"].bool }, set: { on in var value = hook; value["enabled"] = .bool(on); Task { await model.saveHook(value, at: index) } }))
                            Button(localized("编辑")) { editor = NativeHookDraft(index: index, value: hook) }
                            Button(localized("删除"), role: .destructive) { removal = index }
                        }.disabled(!model.canWrite)
                        Text(hook["command"].string).font(.system(.body, design: .monospaced)).textSelection(.enabled)
                        if !hook["matcher"].string.isEmpty { Text(hook["matcher"].string).font(.caption) }
                    }; Divider()
                }
            }
            NativeSettingsGroup(title: "当前项目") {
                if model.project == .null { Text(localized("当前项目没有 .wisp/hooks.json。")) }
                else {
                    Text(model.project["path"].string).textSelection(.enabled)
                    Text(model.project["trusted"].bool ? localized("已信任：这些钩子会运行。") : localized("未信任：这些钩子不会运行。"))
                    Text(localized("文件有任何改动都需要重新审查。信任仅适用于下方显示的文件内容。"))
                    ForEach(Array(model.project["hooks"].array.enumerated()), id: \.offset) { _, hook in
                        Text(verbatim: hook["event"].string + " · " + hook["matcher"].string + " · " + (hook["enabled"].bool ? localized("启用") : localized("停用")))
                        Text(hook["command"].string).font(.system(.body, design: .monospaced)).textSelection(.enabled)
                        Text(localized("超时（秒）") + ": " + (hook["timeout"] == .null ? "60" : hook["timeout"].string)).font(.caption)
                    }
                    if model.project["error"] != .null { Text(model.project["error"].string).foregroundStyle(.orange) }
                    if model.project["trusted"].bool {
                        Button(localized("撤销信任")) { Task { await model.write("set_project_hooks_trust", args: ["sha256": .null]) } }.disabled(!model.canWrite)
                    } else {
                        Button(localized("信任并启用")) { trustHash = model.project["sha256"].string }.disabled(!model.canWrite || model.project["error"] != .null || model.project["sha256"].string.isEmpty)
                    }
                }
            }
        }
        .task { await model.load() }.onDisappear { model.close() }
        .sheet(item: $editor) { item in NativeHookEditor(model: model, original: item) { editor = nil } }
        .confirmationDialog(localized("删除钩子？"), isPresented: Binding(get: { removal != nil }, set: { if !$0 { removal = nil } })) {
            Button(localized("删除"), role: .destructive) { if let index = removal { var rows = model.hooks; if rows.indices.contains(index) { rows.remove(at: index); Task { await model.write("set_command_hooks", args: ["hooks": .array(rows)]) } } }; removal = nil }
            Button(localized("取消"), role: .cancel) { removal = nil }
        }
        .confirmationDialog(localized("信任并启用已审查的项目钩子？"), isPresented: Binding(get: { trustHash != nil }, set: { if !$0 { trustHash = nil } })) {
            Button(localized("信任并启用")) { if let hash = trustHash { Task { await model.write("set_project_hooks_trust", args: ["sha256": .string(hash)]) } }; trustHash = nil }
            Button(localized("取消"), role: .cancel) { trustHash = nil }
        }
    }
}

private struct NativeFailureHookEditor: View {
    @ObservedObject var model: NativeHooksModel
    @State private var value: SettingsValue = .null
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Toggle(localized("自动分析工具失败"), isOn: Binding(get: { value["enabled"].bool }, set: { value["enabled"] = .bool($0) }))
            NativeSettingsField(field: .init(key: "failure_rate_threshold", label: "失败率阈值（%）", kind: .integer), value: binding("failure_rate_threshold"))
            NativeSettingsField(field: .init(key: "minimum_failures", label: "最少失败次数", kind: .integer), value: binding("minimum_failures"))
            Button(localized("保存失败分析设置")) { Task { await model.write("set_auto_failure_analysis_settings", args: ["settings": value]) } }.disabled(value == .null)
        }.onChange(of: model.values["get_auto_failure_analysis_settings"]) { next in value = next ?? .null }
            .onAppear { value = model.values["get_auto_failure_analysis_settings"] ?? .null }
    }
    private func binding(_ key: String) -> Binding<SettingsValue> { Binding(get: { value[key] }, set: { value[key] = $0 }) }
}

struct NativeHookEditor: View {
    @ObservedObject var model: NativeHooksModel
    let original: NativeHookDraft
    let close: () -> Void
    @State private var value: SettingsValue
    @State private var discard = false
    init(model: NativeHooksModel, original: NativeHookDraft, close: @escaping () -> Void) { self.model = model; self.original = original; self.close = close; _value = State(initialValue: original.value) }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(localized("编辑钩子")).font(.title2.bold())
            NativeSettingsField(field: .init(key: "event", label: "事件", kind: .choice(["UserPromptSubmit", "PreToolUse", "PostToolUse", "PostToolUseFailure", "Stop"].map { ($0, $0) })), value: binding("event"))
            if ["PreToolUse", "PostToolUse", "PostToolUseFailure"].contains(value["event"].string) {
                NativeSettingsField(field: .init(key: "matcher", label: "工具匹配", hint: "工具名或正则，例如 shell|write|edit。留空匹配所有工具。"), value: binding("matcher"))
            }
            NativeSettingsField(field: .init(key: "command", label: "命令", kind: .multiline), value: binding("command"))
            Text(localized("在项目目录中运行。退出码 0 继续；退出码 2 阻止，stderr 作为原因。保存不会执行命令。")).font(.caption).foregroundStyle(.secondary)
            NativeSettingsField(field: .init(key: "timeout", label: "超时（秒）", kind: .integer), value: binding("timeout"))
            if let error = model.error { Text(error).foregroundStyle(.orange) }
            if model.uncertain {
                HStack { Button(localized("刷新")) { Task { await model.load() } }; Button(localized("已核对结果，允许继续")) { model.acknowledge() }.disabled(!model.refreshed) }
                ForEach(Array(model.hooks.enumerated()), id: \.offset) { _, row in Text(row["event"].string + ": " + row["command"].string).textSelection(.enabled) }
            }
            HStack { Button(localized("取消")) { dismiss() }; Spacer(); Button(localized("保存")) { Task { if await model.saveHook(value, at: original.index, expected: original.value) { close() } } }.disabled(!model.canWrite || value["command"].string.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty) }
        }.padding(24).frame(minWidth: 390, idealWidth: 600, minHeight: 450)
        .interactiveDismissDisabled(value != original.value || model.busy)
        .background(NativeSettingsEscape(enabled: !discard && !model.busy) { dismiss() })
        .confirmationDialog(localized("放弃尚未保存的修改？"), isPresented: $discard) { Button(localized("放弃修改"), role: .destructive, action: close); Button(localized("继续编辑"), role: .cancel) {} }
    }
    private func binding(_ key: String) -> Binding<SettingsValue> { Binding(get: { value[key] }, set: { value[key] = $0 }) }
    private func dismiss() { guard !model.busy else { return }; if value != original.value { discard = true } else { close() } }
}
