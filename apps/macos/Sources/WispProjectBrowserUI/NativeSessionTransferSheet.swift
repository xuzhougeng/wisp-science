import SwiftUI
import WispProjectBrowser

struct NativeSessionTransferTarget: Identifiable {
    let id = UUID()
    let source: BrowserSession
    let mode: NativeSessionTransferMode
}

struct NativeSessionTransferSheet: View {
    @StateObject private var model: NativeSessionTransferModel
    let projects: [ProjectSummary]
    let close: (NativeSessionTransferResult?) -> Void
    let open: (NativeSessionTransferResult) -> Void
    let transferred: (NativeSessionTransferResult) -> Void
    @State private var projectPicker = false
    private var targets: [ProjectSummary] { projects.filter { $0.id != model.source.projectID && !$0.id.hasPrefix("assistant:") } }
    init(client: any NativeConversationQuerying, source: BrowserSession, mode: NativeSessionTransferMode, projects: [ProjectSummary], writable: @escaping () -> Bool, close: @escaping (NativeSessionTransferResult?) -> Void, open: @escaping (NativeSessionTransferResult) -> Void, transferred: @escaping (NativeSessionTransferResult) -> Void) {
        _model = StateObject(wrappedValue: NativeSessionTransferModel(client: client, source: source, mode: mode, projects: projects.map(\.id), writable: writable))
        self.projects = projects; self.close = close; self.open = open; self.transferred = transferred
    }
    init(model: NativeSessionTransferModel, projects: [ProjectSummary], close: @escaping (NativeSessionTransferResult?) -> Void, open: @escaping (NativeSessionTransferResult) -> Void, transferred: @escaping (NativeSessionTransferResult) -> Void = { _ in }) {
        _model = StateObject(wrappedValue: model); self.projects = projects; self.close = close; self.open = open; self.transferred = transferred
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text(localized(model.mode == .copy ? "复制到其他项目…" : "移动到其他项目…")).font(.headline)
                Spacer(); Button(localized("关闭")) { close(model.result) }.disabled(model.transferring)
            }
            Text(model.preview?.title ?? model.source.title).font(.subheadline).lineLimit(2)
            Text(localized(model.mode == .copy ? "复制保存的对话记录。文件和运行记录仍留在原项目。" : "移动保存的对话记录，可选择搬移关联产物。运行记录仍留在原项目。输入草稿会保留在目标会话。"))
                .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            HStack(alignment: .top) {
                Text(localized("目标项目")); Spacer()
                Button { projectPicker = true } label: {
                    HStack { Text(targets.first { $0.id == model.target }?.name ?? localized("没有可用目标项目")).lineLimit(2); WispIcon(name: "chevron-down", size: 12) }
                }.buttonStyle(.plain).disabled(model.transferring || model.result != nil || targets.isEmpty)
                    .popover(isPresented: $projectPicker) {
                        NativeSessionImportProjectPicker(projects: targets, selected: model.target, close: { projectPicker = false }) { id in
                            projectPicker = false; model.select(id); Task { await model.readPreview() }
                        }
                    }
            }
            if model.reading || model.transferring { ProgressView().controlSize(.small) }
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    if targets.isEmpty { Text(localized("请先创建或导入另一个项目。")) }
                    if let error = model.error { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
                    if model.uncertain { Text(localized("重新预览不会再次提交。请关闭窗口后，在原项目和目标项目核对记录。")) .font(.caption).foregroundStyle(.secondary) }
                    if let preview = model.preview {
                        Text("\(localized("消息数量")): \(preview.message_count)").font(.callout)
                        if model.mode == .move {
                            Toggle(localized("一并移动关联产物"), isOn: Binding(get: { model.includeArtifacts }, set: model.chooseArtifacts))
                                .disabled(preview.artifacts == nil || model.transferring || model.uncertain || model.result != nil)
                            if let error = preview.artifact_error {
                                Text(localized("产物预览不可用，仍可仅移动对话记录。") + "\n" + error).font(.caption).foregroundStyle(.orange).textSelection(.enabled)
                            }
                            if let artifacts = preview.artifacts {
                                Text("\(localized("可移动产物")): \(artifacts.artifacts.count) · \(localized("文件数量")): \(artifacts.files.count)").font(.caption)
                                ForEach(Array(artifacts.artifacts.enumerated()), id: \.offset) { _, name in Text(name).font(.caption).textSelection(.enabled) }
                                ForEach(Array(artifacts.files.enumerated()), id: \.offset) { _, path in Text(path).font(.caption).textSelection(.enabled) }
                                if !artifacts.retained.isEmpty {
                                    Text(localized("以下内容保留在原项目")).font(.subheadline).bold()
                                    ForEach(Array(artifacts.retained.enumerated()), id: \.offset) { _, retained in
                                        Text(retained.name + " · " + localized(Self.retainedLabel(retained.reason))).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                                    }
                                }
                                Text(localized("共享、上传、已修改或不可用的文件不会搬移。目标文件冲突会阻止搬移。")) .font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                    if let result = model.result {
                        Text(localized(result.mode == .copy ? "会话已复制" : "会话已移动")).font(.headline)
                        Text(projects.first { $0.id == result.target_project_id }?.name ?? result.target_project_id).font(.subheadline)
                    }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }.frame(maxWidth: .infinity, maxHeight: .infinity)
            ViewThatFits(in: .horizontal) {
                HStack { refreshButton; Spacer(); confirmButton }
                VStack(alignment: .trailing, spacing: 10) { refreshButton; confirmButton }
            }
        }.padding(24).frame(minWidth: 320, idealWidth: 560, maxWidth: 680, minHeight: 380, idealHeight: 600, maxHeight: 740)
            .interactiveDismissDisabled(model.transferring)
            .background(NativeSettingsEscape(enabled: !model.transferring && !projectPicker) { close(model.result) })
            .task { if model.preview == nil && model.result == nil { await model.readPreview() } }.onDisappear { model.close() }
    }
    private var refreshButton: some View {
        Button(localized("重新预览")) { Task { await model.readPreview() } }.disabled(model.target.isEmpty || model.reading || model.transferring || model.result != nil).fixedSize()
    }
    @ViewBuilder private var confirmButton: some View {
        if let result = model.result { Button(localized("打开目标会话")) { open(result) }.buttonStyle(WispButtonStyle(primary: true)).fixedSize() }
        else { Button(localized(model.mode == .copy ? "确认复制" : "确认移动")) { Task { if let result = await model.confirm() { transferred(result) } } }.disabled(!model.canTransfer).buttonStyle(WispButtonStyle(primary: true)).fixedSize() }
    }
    static func retainedLabel(_ reason: String) -> String {
        switch reason { case "shared": return "其他会话共享"; case "upload": return "上传文件"; case "changed": return "文件已修改"; default: return "文件不可用" }
    }
}
