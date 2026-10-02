import SwiftUI
import WispProjectBrowser

struct NativeAgentPanelView: View {
    @ObservedObject var model: NativePanelModel
    var query = ""
    var readOnly = false
    var manageWorkflows: () -> Void = {}
    @State private var pending: NativeAgentSnapshot?
    @State private var action = NativeAgentAction.approve
    @State private var retry: NativeAgentSnapshot?
    @Environment(\.colorScheme) private var scheme
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 8) {
                Text("Agents").font(.headline)
                Text(localized("代理任务会在当前对话中返回结果；这里可查看工作流状态与详情。")).font(.caption).foregroundStyle(.secondary)
                Button(localized("管理工作流"), action: manageWorkflows)
            }.padding(12).frame(maxWidth: .infinity, alignment: .leading)
                .background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 8))
            Toggle("允许当前会话委派代理任务", isOn: Binding(get: { model.agentDelegationEnabled ?? false }, set: { enabled in Task { await model.setAgentDelegation(enabled) } }))
                .disabled(readOnly || model.agentDelegationBusy || model.agentDelegationEnabled == nil)
            if model.agentDelegationBusy { ProgressView().controlSize(.small) }
            if model.agentDelegationEnabled == false {
                VStack(alignment: .leading, spacing: 5) {
                    Text(localized("代理委派已关闭")).font(.subheadline.weight(.medium))
                    Text(localized("当前会话不会自动委派新任务；已有工作流仍可查看。开启委派不会跳过审批。"))
                        .font(.caption).foregroundStyle(.secondary)
                }.padding(12).frame(maxWidth: .infinity, alignment: .leading)
                    .background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 8))
            }
            if model.agentResultLoading { ProgressView().controlSize(.small) }
            ForEach(model.agents.filter { query.isEmpty || $0.workflow.name.localizedCaseInsensitiveContains(query) || $0.workflow.goal.localizedCaseInsensitiveContains(query) }) { snapshot in
                VStack(alignment: .leading, spacing: 10) {
                    Text(snapshot.workflow.name).font(.headline)
                    Text(snapshot.workflow.status + " · " + snapshot.workflow.mode).font(.caption).foregroundStyle(.secondary)
                    Text(snapshot.workflow.goal).font(.callout).textSelection(.enabled)
                    if !snapshot.delegation_enabled { Text("此会话未启用代理委派").font(.caption).foregroundStyle(.orange) }
                    DisclosureGroup("完整计划") { NativeAgentResultValue(value: snapshot.dynamic.editable_proposal).font(.caption) }
                    actions(snapshot)
                    ForEach(snapshot.dynamic.tasks) { task in
                        DisclosureGroup {
                            VStack(alignment: .leading, spacing: 8) {
                                Text(task.instruction).textSelection(.enabled)
                                if !task.depends_on.isEmpty { Text("依赖：" + task.depends_on.joined(separator: "、")).font(.caption) }
                                Text([task.executor["kind"].string, task.executor["model_id"].string].filter { !$0.isEmpty }.joined(separator: " · ")).font(.caption).foregroundStyle(.secondary)
                                Text("Token 预算：" + (task.budget["max_tokens"] == .null ? "默认" : task.budget["max_tokens"].string) + " · 工具预算：" + (task.budget["max_tool_calls"] == .null ? "默认" : task.budget["max_tool_calls"].string)).font(.caption)
                                Text([task.can_write ? "可写文件" : nil, task.can_execute ? "可执行命令" : nil, task.can_access_network ? "可访问网络" : nil].compactMap { $0 }.joined(separator: " · ")).font(.caption).foregroundStyle(.secondary)
                                if !task.tools.isEmpty { Text("工具：" + task.tools.joined(separator: "、")).font(.caption) }
                                ForEach(task.approval_reasons, id: \.self) { Text($0).font(.caption).foregroundStyle(.orange) }
                                if let result = task.result {
                                    Text(result["summary"].string).textSelection(.enabled)
                                    if !result["error"].string.isEmpty { Text(result["error"].string).foregroundStyle(.orange) }
                                    Text("输入 \(result["input_tokens"].integer) · 输出 \(result["output_tokens"].integer) · 工具 \(result["tool_calls"].integer)").font(.caption).foregroundStyle(.secondary)
                                    Button("查看完整结果") { Task { await model.readAgentResult(workflow: snapshot.id, step: task.stored_step_id) } }.disabled(!result["full_result_available"].bool || model.agentResultLoading)
                                }
                            }.padding(.vertical, 6)
                        } label: { HStack { Text(task.id); Spacer(); Text(task.result?["status"].string ?? "等待执行").font(.caption).foregroundStyle(.secondary) } }
                    }
                }.padding(12).frame(maxWidth: .infinity, alignment: .leading).background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 8))
            }
            if model.agents.isEmpty && !model.loading && model.error == nil { Text(localized("当前会话暂无代理工作流")).foregroundStyle(.secondary).padding(.vertical) }
        }
        .confirmationDialog(action == .approve ? "批准此版本的代理计划？自动模式将立即启动。" : (action == .cancel ? "取消正在执行的工作流？" : "丢弃此工作流？"), isPresented: Binding(get: { pending != nil }, set: { if !$0 { pending = nil } })) {
            if let snapshot = pending {
                Button(action == .approve ? "批准计划" : (action == .cancel ? "取消执行" : "丢弃工作流"), role: action == .approve ? nil : .destructive) {
                    let selectedAction = action; pending = nil
                    Task { await model.performAgentAction(snapshot, action: selectedAction) }
                }
            }
            Button("返回", role: .cancel) { pending = nil }
        }
        .sheet(item: $retry) { snapshot in
            NativeAgentRetryView(snapshot: snapshot, close: { retry = nil }) { budgets in
                retry = nil
                Task { await model.performAgentAction(snapshot, action: .retry, budgets: budgets) }
            }
        }
    }
    @ViewBuilder private func actions(_ snapshot: NativeAgentSnapshot) -> some View {
        if snapshot.workflow.depth == 0 {
            let legacy = snapshot.dynamic.tasks.contains { !($0.skill_bindings ?? []).isEmpty }
            if legacy { Text("此旧版技能工作流需要重新转换后才能执行。").font(.caption).foregroundStyle(.orange) }
            HStack {
                if snapshot.workflow.status == "draft" {
                    Button(snapshot.approval_policy == "auto_safe" ? "批准并启动…" : "批准…") { action = .approve; pending = snapshot }.disabled(!snapshot.delegation_enabled || legacy)
                    Button("丢弃…") { action = .discard; pending = snapshot }
                }
                if snapshot.workflow.status == "approved" {
                    Button("执行") { Task { await model.performAgentAction(snapshot, action: .run) } }.disabled(!snapshot.delegation_enabled || legacy || model.agentLaunching.contains(snapshot.id))
                }
                if snapshot.workflow.status == "running" { Button("取消…") { action = .cancel; pending = snapshot } }
                if ["failed", "cancelled"].contains(snapshot.workflow.status) {
                    Button("重试…") { retry = snapshot }.disabled(!snapshot.delegation_enabled || legacy)
                }
                if model.agentLaunching.contains(snapshot.id) || model.agentActions.contains(snapshot.id) { ProgressView().controlSize(.small) }
            }.disabled(model.agentActions.contains(snapshot.id) || (readOnly && snapshot.workflow.status != "running"))
        }
    }
}
struct NativeAgentResultView: View {
    @Environment(\.colorScheme) private var scheme
    let result: NativeAgentResult
    let close: () -> Void
    private var json: String {
        let encoder = JSONEncoder(); encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        return (try? encoder.encode(result.response)).flatMap { String(data: $0, encoding: .utf8) } ?? ""
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Text("代理结果 · \(result.step_id)").font(.headline); Spacer(); Text("尝试 \(result.attempt) · \(result.status)").foregroundStyle(.secondary); Button("关闭", action: close) }
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    ForEach(Array(NativeAgentResultPresentation(result.response).sections.enumerated()), id: \.offset) { _, section in
                        VStack(alignment: .leading, spacing: 8) {
                            Text(section.0).font(.headline)
                            if section.0 == "产物" || section.0 == "证据" { NativeAgentResultItems(value: section.1, evidence: section.0 == "证据") }
                            else { NativeAgentResultValue(value: section.1) }
                        }
                    }
                    DisclosureGroup("原始结果") { Text(json).font(.system(size: 12, design: .monospaced)).textSelection(.enabled) }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }
        }.padding(20).frame(minWidth: 560, idealWidth: 800, minHeight: 400, idealHeight: 650).background(WispDesign.color("bg-app", scheme)).background(NativeSettingsEscape(close: close))
    }
}

struct NativeAgentResultValue: View {
    let value: SettingsValue
    var body: some View {
        switch value {
        case .string(let text): NativeShareMarkdown(text: text).textSelection(.enabled)
        case .array(let rows): VStack(alignment: .leading, spacing: 10) {
            ForEach(Array(rows.enumerated()), id: \.offset) { _, item in AnyView(NativeAgentResultValue(value: item)).padding(.leading, 8) }
        }
        case .object(let fields): VStack(alignment: .leading, spacing: 8) {
            ForEach(fields.keys.sorted(), id: \.self) { key in
                VStack(alignment: .leading, spacing: 4) { Text(key.replacingOccurrences(of: "_", with: " ")).font(.caption).foregroundStyle(.secondary); AnyView(NativeAgentResultValue(value: fields[key]!)) }
            }
        }
        case .bool(let flag): Text(flag ? "是" : "否")
        case .null: Text("—").foregroundStyle(.secondary)
        default: Text(value.string).textSelection(.enabled)
        }
    }
}

struct NativeAgentResultItems: View {
    let value: SettingsValue
    let evidence: Bool
    @Environment(\.colorScheme) private var scheme
    private var rows: [SettingsValue] { if case .array(let rows) = value { return rows }; return [value] }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(Array(rows.enumerated()), id: \.offset) { _, item in
                if case .object(let fields) = item {
                    let path = item[evidence ? "reference" : "path"].string
                    let content = evidence ? (fields["summary"] ?? fields["evidence"] ?? .null) : (fields["content"] ?? fields["summary"] ?? .null)
                    let excluded = Set(evidence ? ["kind", "reference", "summary", "evidence"] : ["name", "kind", "path", "id", "content", "summary"])
                    let details = fields.filter { !excluded.contains($0.key) }
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            if !evidence { Text(item["name"].string.isEmpty ? (path.isEmpty ? "产物" : path) : item["name"].string).font(.headline) }
                            if !item["kind"].string.isEmpty { Text(item["kind"].string).font(.caption).foregroundStyle(.secondary) }
                        }
                        if content != .null { NativeAgentResultValue(value: content) }
                        if !path.isEmpty { Text(path).font(.system(size: 12, design: .monospaced)).textSelection(.enabled).foregroundStyle(.secondary) }
                        if !details.isEmpty { NativeAgentResultValue(value: .object(details)) }
                    }.padding(12).frame(maxWidth: .infinity, alignment: .leading).background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 8))
                } else { NativeAgentResultValue(value: item) }
            }
        }
    }
}

struct NativeAgentRetryView: View {
    @Environment(\.colorScheme) private var scheme
    let snapshot: NativeAgentSnapshot
    let close: () -> Void
    let submit: ([String: NativeAgentBudgetOverride]) -> Void
    @State private var tokens: [String: String] = [:]
    @State private var error: String?
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("重试 · " + snapshot.workflow.name).font(.headline)
            Text("留空沿用原预算；0 表示不限制。自动模式将在重试后启动。").font(.caption).foregroundStyle(.secondary)
            ScrollView {
                ForEach(snapshot.dynamic.tasks) { task in
                    HStack { VStack(alignment: .leading) { Text(task.id); Text("当前：" + (task.budget["max_tokens"] == .null ? "默认" : task.budget["max_tokens"].string)).font(.caption).foregroundStyle(.secondary) }; Spacer(); TextField("Token 预算", text: Binding(get: { tokens[task.id] ?? "" }, set: { tokens[task.id] = $0 })).frame(width: 160) }
                }
            }
            if let error { Text(error).foregroundStyle(.orange) }
            HStack { Spacer(); Button("取消", action: close); Button("重试") {
                do { submit(try Self.overrides(tokens)) } catch { self.error = error.localizedDescription }
            } }
        }.padding(20).frame(minWidth: 480, minHeight: 300).background(WispDesign.color("bg-app", scheme)).background(NativeSettingsEscape(close: close))
    }
    static func overrides(_ tokens: [String: String]) throws -> [String: NativeAgentBudgetOverride] {
        var result: [String: NativeAgentBudgetOverride] = [:]
        for (id, text) in tokens {
            let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
            if text.isEmpty { continue }
            guard let value = UInt32(text) else { throw NSError(domain: "AgentBudget", code: 1, userInfo: [NSLocalizedDescriptionKey: "Token 预算必须是非负整数（0 表示不限制）。"]) }
            result[id] = NativeAgentBudgetOverride(tokens: value)
        }
        return result
    }
}
