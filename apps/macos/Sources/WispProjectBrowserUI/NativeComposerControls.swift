import SwiftUI
import WispProjectBrowser

struct NativeComposerEnvironment: View {
    @ObservedObject var model: NativeComposerModel
    let writable: Bool
    let openRuntime: () -> Void
    var openOptions: (() -> Void)?
    var body: some View {
        HStack(spacing: 8) {
            HStack(spacing: 6) {
                WispIcon(name: "server", size: 14)
                Menu {
                    ForEach(model.contexts?.attached ?? []) { context in
                        Button { Task { await model.selectContext(context.id) } } label: {
                            HStack { Text(context.label); if model.contextID == context.id { WispIcon(name: "check") } }
                        }
                    }
                } label: { Text(model.contextLabel).lineLimit(1) }
                    .menuStyle(.borderlessButton).fixedSize()
                    .accessibilityLabel("会话运行环境").help("选择本会话的默认运行环境")
            }.disabled(!writable || model.busy || model.contexts == nil)
            Button(action: openRuntime) {
                HStack(spacing: 6) { WispIcon(name: "terminal", size: 14); Text(model.runtimeLabel).lineLimit(1) }
            }.buttonStyle(.plain).disabled(model.contexts == nil).accessibilityLabel("查看运行时")
            Spacer(minLength: 0)
            if model.busy { ProgressView().controlSize(.small) }
            if let openOptions {
                Button(action: openOptions) { WispIcon(name: "adjustments", size: 16) }.buttonStyle(.plain)
                    .accessibilityLabel(localized("会话选项")).help(localized("会话选项"))
            }
        }.font(WispDesign.font(size: 11)).foregroundStyle(.secondary).frame(height: 32)
        if let error = model.error {
            HStack(alignment: .top) {
                Text(error).font(.caption).foregroundStyle(.orange).fixedSize(horizontal: false, vertical: true)
                Button("重新读取") { Task { await model.reload() } }.font(.caption)
            }
        }
    }
}

struct NativeComposerEffort: View {
    @ObservedObject var model: NativeComposerModel
    let enabled: Bool
    let acp: Bool
    var body: some View {
        HStack(spacing: 5) {
            WispIcon(name: "gauge", size: 14)
            Menu {
                Button("默认") { Task { await model.selectEffort("") } }
                ForEach(model.efforts, id: \.self) { value in
                    Button(value.capitalized) { Task { await model.selectEffort(value) } }
                }
            } label: {
                Text(acp ? "ACP" : model.effort.isEmpty ? "默认" : model.effort.capitalized).lineLimit(1)
            }.menuStyle(.borderlessButton).fixedSize()
                .accessibilityLabel("思考强度").help(acp ? "思考强度由 ACP 智能体管理" : model.efforts.isEmpty ? "此模型未声明可调节的思考强度" : "更改此模型的默认思考强度，在下一轮使用")
        }.frame(height: 32).disabled(!enabled || acp || model.busy || model.efforts.isEmpty)
    }
}

struct NativeComposerReferencePicker: View {
    @ObservedObject var model: NativeComposerModel
    let select: (NativeComposerReference) -> Void
    let close: () -> Void
    @State private var kind = "artifact"
    @State private var query = ""
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Text("添加引用").font(.headline); Spacer(); Button("关闭", action: close) }
            Picker("引用类型", selection: $kind) {
                Text("产物与环境").tag("artifact")
                Text("会话与项目").tag("session")
                Text("技能与工作流").tag("skill")
            }.pickerStyle(.segmented)
            TextField("搜索名称或描述", text: $query)
            Text("引用会随消息发送；查看候选不会启动运行时或任务。").font(.caption).foregroundStyle(.secondary)
            if model.searching { ProgressView().controlSize(.small) }
            if let error = model.searchError { Text(error).foregroundStyle(.orange).textSelection(.enabled) }
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 4) {
                    ForEach(model.options) { option in
                        Button { select(option); close() } label: {
                            HStack(alignment: .top, spacing: 8) {
                                WispIcon(name: icon(option.reference["kind"].string), size: 16)
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(option.label).font(.system(size: 13, weight: .medium))
                                    Text(option.detail).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                                }
                                Spacer()
                            }.padding(8).frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
                        }.buttonStyle(.plain).accessibilityLabel("引用 " + option.label)
                    }
                    if !model.searching && model.options.isEmpty && model.searchError == nil { Text("没有匹配的引用").foregroundStyle(.secondary).padding(8) }
                }
            }
        }.padding(20).frame(width: 520, height: 400)
            .background(NativeSettingsEscape(close: close))
            .task(id: kind + "\n" + query) {
                do { try await Task.sleep(nanoseconds: 120_000_000) } catch { return }
                await model.search(kind: kind, query: query)
            }
            .onDisappear { model.dismissSearch() }
    }
    private func icon(_ kind: String) -> String {
        ["artifact": "doc", "session": "chat", "project": "folder", "context": "server", "runtime": "terminal", "workflow": "plan", "skill": "sparkles"][kind] ?? "doc"
    }
}

struct NativeComposerModes: View {
    @ObservedObject var conversation: NativeConversationModel
    @Environment(\.colorScheme) private var scheme
    var body: some View {
        if conversation.snapshot?.plan_mode != nil || conversation.snapshot?.fast_mode != nil {
            HStack(spacing: 16) {
                if let plan = conversation.snapshot?.plan_mode {
                    HStack(spacing: 5) {
                        WispIcon(name: plan ? "plan" : "sparkles", size: 14)
                        Menu {
                            Button(localized("Agent · 执行任务")) { Task { await conversation.setPlanMode(false) } }
                            Button(localized("Plan · 先制定计划")) { Task { await conversation.setPlanMode(true) } }
                        } label: {
                            Text(plan ? "Plan" : "Agent")
                        }.menuStyle(.borderlessButton).fixedSize()
                            .accessibilityLabel(localized("会话模式"))
                    }.foregroundStyle(WispDesign.color(plan ? "clay" : "text-muted", scheme)).disabled(!conversation.canChangeMode)
                }
                if let fast = conversation.snapshot?.fast_mode {
                    Button { Task { await conversation.setFastMode(!fast.enabled) } } label: {
                        HStack(spacing: 5) { WispIcon(name: "bolt", size: 14); Text("Fast") }
                            .foregroundStyle(fast.enabled ? WispDesign.color("clay", scheme) : WispDesign.color("text-muted", scheme))
                    }.buttonStyle(.plain).disabled(!conversation.canChangeMode)
                        .accessibilityLabel(localized(fast.enabled ? "关闭 Fast" : "开启 Fast"))
                        .help(localized(fast.inherited ? "继承模型的 Fast 默认设置" : "本会话的 Fast 设置"))
                }
                Spacer(minLength: 0)
            }.font(WispDesign.font(size: 12)).frame(height: 28)
        }
    }
}
