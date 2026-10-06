import SwiftUI
import WispProjectBrowser

struct NativeComposerEnvironment: View {
    @ObservedObject var model: NativeComposerModel
    let writable: Bool
    let openRuntime: () -> Void
    var body: some View {
        HStack(spacing: 8) {
            Menu {
                ForEach(model.contexts?.attached ?? []) { context in
                    Button { Task { await model.selectContext(context.id) } } label: {
                        HStack { Text(context.label); if model.contextID == context.id { WispIcon(name: "check") } }
                    }
                }
            } label: { HStack(spacing: 6) { WispIcon(name: "server", size: 14); Text(model.contextLabel).lineLimit(1) } }
                .menuStyle(.borderlessButton).fixedSize().disabled(!writable || model.busy || model.contexts == nil)
                .accessibilityLabel("会话运行环境").help("选择本会话的默认运行环境")
            Button(action: openRuntime) {
                HStack(spacing: 6) { WispIcon(name: "terminal", size: 14); Text(model.runtimeLabel).lineLimit(1) }
            }.buttonStyle(.plain).disabled(model.contexts == nil).accessibilityLabel("查看运行时")
            Spacer(minLength: 0)
            if model.busy { ProgressView().controlSize(.small) }
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
        Menu {
            Button("默认") { Task { await model.selectEffort("") } }
            ForEach(model.efforts, id: \.self) { value in
                Button(value.capitalized) { Task { await model.selectEffort(value) } }
            }
        } label: {
            HStack(spacing: 5) { WispIcon(name: "gauge", size: 14); Text(acp ? "ACP" : model.effort.isEmpty ? "默认" : model.effort.capitalized).lineLimit(1) }
        }.menuStyle(.borderlessButton).fixedSize().frame(height: 32)
            .disabled(!enabled || acp || model.busy || model.efforts.isEmpty)
            .accessibilityLabel("思考强度").help(acp ? "思考强度由 ACP 智能体管理" : model.efforts.isEmpty ? "此模型未声明可调节的思考强度" : "更改此模型的默认思考强度，在下一轮使用")
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
