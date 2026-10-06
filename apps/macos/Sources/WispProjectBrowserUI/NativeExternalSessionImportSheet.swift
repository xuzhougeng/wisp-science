import SwiftUI
import WispProjectBrowser

struct NativeExternalSessionImportSheet: View {
    @StateObject private var model: NativeExternalSessionImportModel
    let projects: [ProjectSummary]
    let close: () -> Void
    let open: (String, String) -> Void
    private let loadOnAppear: Bool
    @State private var selector: String?
    init(client: any NativeConversationQuerying, project: String, projects: [ProjectSummary], writable: @escaping () -> Bool, close: @escaping () -> Void, open: @escaping (String, String) -> Void) {
        _model = StateObject(wrappedValue: NativeExternalSessionImportModel(client: client, project: project, projects: projects.map(\.id), writable: writable))
        self.projects = projects; self.close = close; self.open = open; loadOnAppear = true
    }
    init(model: NativeExternalSessionImportModel, projects: [ProjectSummary], close: @escaping () -> Void, open: @escaping (String, String) -> Void) {
        _model = StateObject(wrappedValue: model); self.projects = projects; self.close = close; self.open = open; loadOnAppear = false
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack { Text(localized("导入 Codex / Claude 会话")).font(.headline); Spacer(); Button(localized("关闭"), action: close).disabled(model.importing) }
            Text(localized("选择目标项目和来源。预览不写入会话；导入会绑定预览中的来源和文件校验值。")) .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            selectionRow("project", title: "目标项目", value: projects.first { $0.id == model.project }?.name ?? model.project)
            selectionRow("provider", title: "会话提供方", value: model.provider == "codex" ? "Codex CLI" : "Claude Code")
            selectionRow("context", title: "来源环境", value: model.sources.first { $0.id == model.context }?.label ?? model.context)
            TextField(localized("筛选标题、工作目录、会话 ID 或路径"), text: Binding(get: { model.query }, set: model.setQuery)).textFieldStyle(.roundedBorder).disabled(model.importing)
            ViewThatFits(in: .horizontal) {
                HStack { refreshButtons; Spacer(); batchButton }
                VStack(alignment: .trailing, spacing: 10) { HStack { refreshButtons; Spacer() }; batchButton }
            }
            if model.sourcesLoading || model.loading || model.previewing { ProgressView().controlSize(.small) }
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 12) {
                    if let error = model.sourcesError {
                        Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled)
                        Button(localized("重新读取来源")) { Task { await model.initialize() } }.disabled(model.importing)
                    }
                    if let error = model.error { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
                    if model.uncertain { Text(localized("可重新读取和查看已有会话；本窗口不会再次提交导入。")) .font(.caption).foregroundStyle(.secondary) }
                    ForEach(model.pageItems) { item in
                        VStack(alignment: .leading, spacing: 5) {
                            Text(item.title.isEmpty ? item.session_id : item.title).font(.subheadline).bold().lineLimit(2)
                            Text(item.cwd).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                            Text("\(localized("消息数量")): \(item.message_count) · \(localized(Self.stateLabel(item.state)))") .font(.caption)
                            Text(item.path).font(.caption2).foregroundStyle(.secondary).textSelection(.enabled).lineLimit(2).help(item.path)
                            HStack {
                                Button(localized("预览会话")) { Task { await model.inspect(item) } }.disabled(model.loading || model.previewing || model.importing)
                                Spacer()
                                if let result = model.results[item.path] { Button(localized("打开导入的会话")) { open(result.project_id, result.frame_id) }.disabled(model.importing) }
                            }
                            if let error = model.itemErrors[item.path] { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
                        }.frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 8)
                        Divider()
                    }
                    if model.filtered.isEmpty && !model.loading { Text(localized("没有匹配的来源会话。")) .foregroundStyle(.secondary) }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }.frame(maxHeight: .infinity)
            if model.total > 0 {
                Text("\(localized("批次进度")): \(model.done)/\(model.total) · \(localized("新建")): \(model.imported) · \(localized("更新")): \(model.updated) · \(localized("跳过")): \(model.skipped) · \(localized("失败")): \(model.failed)") .font(.caption).fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                Button { model.setPage(model.page - 1) } label: { WispIcon(name: "chevron-left", size: 14) }.disabled(model.page == 0 || model.importing).accessibilityLabel(localized("上一页"))
                Text("\(model.page + 1)/\(model.pageCount) · \(model.filtered.count)").font(.caption)
                Button { model.setPage(model.page + 1) } label: { WispIcon(name: "chevron-right", size: 14) }.disabled(model.page + 1 >= model.pageCount || model.importing).accessibilityLabel(localized("下一页"))
                Spacer()
                if model.importing { ProgressView().controlSize(.small); Button(localized("停止后续导入")) { model.stopAfterCurrent() }.disabled(model.stopRequested) }
            }
        }.padding(24).frame(minWidth: 320, idealWidth: 680, maxWidth: 840, minHeight: 450, idealHeight: 680, maxHeight: 840)
            .interactiveDismissDisabled(model.importing)
            .background(NativeSettingsEscape(enabled: !model.importing && selector == nil && model.preview == nil, close: close))
            .sheet(isPresented: Binding(get: { model.preview != nil }, set: { if !$0 { model.clearPreview() } })) {
                NativeExternalImportPreviewSheet(model: model, open: open)
            }
            .task { if loadOnAppear { await model.initialize() } }.onDisappear { model.close() }
    }
    private var refreshButtons: some View {
        HStack {
            Button(localized("读取缓存")) { Task { await model.load(refresh: false) } }.disabled(model.loading || model.importing || model.sources.isEmpty).fixedSize()
            Button(localized("重新扫描来源")) { Task { await model.load(refresh: true) } }.disabled(model.loading || model.importing || model.sources.isEmpty).fixedSize()
        }
    }
    private var batchButton: some View { Button(localized("导入筛选结果")) { Task { await model.importFiltered() } }.buttonStyle(WispButtonStyle(primary: true)).disabled(!model.canImportFiltered).fixedSize() }
    private func selectionRow(_ kind: String, title: String, value: String) -> some View {
        HStack(alignment: .top) {
            Text(localized(title)); Spacer()
            Button { selector = kind } label: { HStack { Text(value).lineLimit(2); WispIcon(name: "chevron-down", size: 12) } }.buttonStyle(.plain).disabled(model.importing)
                .popover(isPresented: Binding(get: { selector == kind }, set: { if !$0 { selector = nil } })) {
                    if kind == "project" {
                        NativeSessionImportProjectPicker(projects: projects, selected: model.project, close: { selector = nil }) { id in change(project: id, provider: model.provider, context: model.context) }
                    } else {
                        NativeExternalImportChoicePicker(title: title, choices: kind == "provider" ? [("codex", "Codex CLI"), ("claude", "Claude Code")] : model.sources.map { ($0.id, $0.label) }, selected: kind == "provider" ? model.provider : model.context, close: { selector = nil }) { id in
                            change(project: model.project, provider: kind == "provider" ? id : model.provider, context: kind == "context" ? id : model.context)
                        }
                    }
                }
        }
    }
    private func change(project: String, provider: String, context: String) {
        selector = nil; model.select(project: project, provider: provider, context: context); Task { await model.load(refresh: false) }
    }
    static func stateLabel(_ state: String) -> String { state == "imported" ? "已导入" : state == "updatable" ? "可更新" : "待导入" }
}

struct NativeExternalImportChoicePicker: View {
    let title: String
    let choices: [(String, String)]
    let selected: String
    let close: () -> Void
    let select: (String) -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Text(localized(title)).font(.headline); Spacer(); Button(localized("关闭"), action: close) }
            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(choices, id: \.0) { id, label in
                        Button { select(id) } label: {
                            HStack { if id == selected { WispIcon(name: "check", size: 14) } else { Color.clear.frame(width: 14, height: 14) }; Text(label).frame(maxWidth: .infinity, alignment: .leading) }.padding(8)
                        }.buttonStyle(.plain)
                    }
                }
            }.frame(maxHeight: 260)
        }.padding(16).frame(width: 330).background(NativeSettingsEscape(close: close))
    }
}

struct NativeExternalImportPreviewSheet: View {
    @ObservedObject var model: NativeExternalSessionImportModel
    let open: (String, String) -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack { Text(localized("预览来源会话")).font(.headline); Spacer(); Button(localized("关闭")) { model.clearPreview() }.disabled(model.importing) }
            if let preview = model.preview {
                Text(preview.path).font(.caption).textSelection(.enabled).lineLimit(3)
                Text("\(localized("消息数量")): \(preview.message_count)")
                ScrollView {
                    VStack(alignment: .leading, spacing: 14) {
                        ForEach(Array(preview.messages.enumerated()), id: \.offset) { _, line in
                            VStack(alignment: .leading, spacing: 5) {
                                Text(localized(line.role == "user" ? "你" : "助手")).font(.caption).foregroundStyle(.secondary)
                                Text(line.text).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                            }.frame(maxWidth: .infinity, alignment: .leading)
                        }
                        if let error = model.error { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }.frame(maxHeight: .infinity)
                if model.importing { ProgressView().controlSize(.small) }
                ViewThatFits(in: .horizontal) {
                    HStack { actions(preview); Spacer() }
                    VStack(alignment: .leading, spacing: 10) { actions(preview) }
                }
            }
        }.padding(24).frame(minWidth: 320, idealWidth: 560, maxWidth: 680, minHeight: 360, idealHeight: 540, maxHeight: 720)
            .interactiveDismissDisabled(model.importing).background(NativeSettingsEscape(enabled: !model.importing, close: model.clearPreview))
    }
    @ViewBuilder private func actions(_ preview: NativeExternalImportPreview) -> some View {
        if let result = model.results[preview.path] {
            Text(localized(NativeSessionArchiveImportSheet.resultLabel(result.status))).font(.caption)
            Button(localized("打开导入的会话")) { open(result.project_id, result.frame_id) }.buttonStyle(WispButtonStyle(primary: true)).fixedSize()
        } else {
            if let existing = preview.existing_session_id { Button(localized("查看已有会话")) { open(preview.project_id, existing) }.disabled(model.importing).fixedSize() }
            Button(localized("确认导入")) { Task { await model.importPreview() } }.disabled(!model.canImport).buttonStyle(WispButtonStyle(primary: true)).fixedSize()
        }
    }
}
