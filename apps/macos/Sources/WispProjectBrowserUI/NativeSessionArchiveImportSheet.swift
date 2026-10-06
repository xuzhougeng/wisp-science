import AppKit
import SwiftUI
import UniformTypeIdentifiers
import WispProjectBrowser

struct NativeSessionArchiveImportSheet: View {
    @StateObject private var model: NativeSessionArchiveImportModel
    let projects: [ProjectSummary]
    let close: () -> Void
    let open: (String, String) -> Void
    @State private var projectPicker = false
    init(client: any NativeConversationQuerying, project: String, projects: [ProjectSummary], writable: @escaping () -> Bool, close: @escaping () -> Void, open: @escaping (String, String) -> Void) {
        _model = StateObject(wrappedValue: NativeSessionArchiveImportModel(client: client, project: project, projects: projects.map(\.id), writable: writable))
        self.projects = projects; self.close = close; self.open = open
    }
    init(model: NativeSessionArchiveImportModel, projects: [ProjectSummary], close: @escaping () -> Void, open: @escaping (String, String) -> Void) {
        _model = StateObject(wrappedValue: model); self.projects = projects; self.close = close; self.open = open
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack { Text(localized("导入会话 ZIP 归档")).font(.headline); Spacer(); Button(localized("关闭"), action: close).disabled(model.importing) }
            Text(localized("先预览消息和产物，再确认导入目标项目。现有文件会保留；冲突副本由宿主放入导入目录。")) .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            HStack(alignment: .top) {
                Text(localized("目标项目")); Spacer()
                Button { projectPicker = true } label: {
                    HStack { Text(projects.first { $0.id == model.project }?.name ?? model.project).lineLimit(2); WispIcon(name: "chevron-down", size: 12) }
                }.buttonStyle(.plain).disabled(model.importing)
                    .popover(isPresented: $projectPicker) {
                        NativeSessionImportProjectPicker(projects: projects, selected: model.project, close: { projectPicker = false }) { id in
                            projectPicker = false; model.select(project: id, path: model.path)
                            Task { await model.readPreview() }
                        }
                    }
            }
            Button(localized("选择会话 ZIP 文件…"), action: chooseArchive).disabled(model.importing)
            if !model.path.isEmpty { Text(model.path).font(.caption).textSelection(.enabled).lineLimit(3).help(model.path) }
            if model.reading || model.importing { ProgressView().controlSize(.small) }
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    if let error = model.error { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
                    if model.uncertain { Text(localized("可重新预览并查看已有会话；核对不会重新提交导入。")) .font(.caption).foregroundStyle(.secondary) }
                    if let preview = model.preview {
                        Text(preview.title).font(.subheadline).bold()
                        Text("\(localized("消息数量")): \(preview.message_count) · \(localized("产物数量")): \(preview.artifacts.count)")
                        Text(localized(Self.stateLabel(preview.state))).font(.caption).foregroundStyle(.secondary)
                        ForEach(Array(preview.messages.enumerated()), id: \.offset) { _, line in
                            VStack(alignment: .leading, spacing: 5) {
                                Text(localized(line.role == "user" ? "你" : "助手")).font(.caption).foregroundStyle(.secondary)
                                Text(line.text).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                            }.frame(maxWidth: .infinity, alignment: .leading)
                        }
                        if !preview.artifacts.isEmpty {
                            DisclosureGroup(localized("归档产物")) {
                                ForEach(Array(preview.artifacts.enumerated()), id: \.offset) { _, path in Text(path).font(.caption).textSelection(.enabled) }
                            }
                        }
                    }
                    if let result = model.result {
                        Text(localized(Self.resultLabel(result.status))).font(.headline)
                        Text("\(localized("已登记产物")): \(result.artifact_count)").font(.caption)
                        ForEach(Array(result.missing_artifacts.enumerated()), id: \.offset) { _, missing in Text(missing).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
                    }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }.frame(maxWidth: .infinity, maxHeight: .infinity)
            ViewThatFits(in: .horizontal) {
                HStack { previewButton; Spacer(); actionButtons }
                VStack(alignment: .trailing, spacing: 10) { previewButton; actionButtons }
            }
        }.padding(24).frame(minWidth: 320, idealWidth: 560, maxWidth: 680, minHeight: 400, idealHeight: 640, maxHeight: 760)
            .interactiveDismissDisabled(model.importing)
            .background(NativeSettingsEscape(enabled: !model.importing && !projectPicker, close: close))
            .onDisappear { model.close() }
    }
    private var previewButton: some View {
        Button(localized("重新预览")) { Task { await model.readPreview() } }.disabled(model.path.isEmpty || model.reading || model.importing).fixedSize()
    }
    @ViewBuilder private var actionButtons: some View {
        if let result = model.result { Button(localized("打开导入的会话")) { open(result.project_id, result.frame_id) }.buttonStyle(WispButtonStyle(primary: true)).fixedSize() }
        else {
            if let existing = model.preview?.existing_session_id { Button(localized("查看已有会话")) { open(model.project, existing) }.disabled(model.importing).fixedSize() }
            Button(localized(model.preview?.state == "updatable" ? "确认更新会话" : "确认导入")) { Task { await model.confirm() } }.disabled(!model.canImport || model.preview?.state == "imported").buttonStyle(WispButtonStyle(primary: true)).fixedSize()
        }
    }
    private func chooseArchive() {
        let panel = NSOpenPanel(); panel.canChooseDirectories = false; panel.canChooseFiles = true; panel.allowsMultipleSelection = false; panel.allowedContentTypes = [.zip]
        panel.begin { response in
            guard response == .OK, let url = panel.url else { return }
            model.select(project: model.project, path: url.path); Task { await model.readPreview() }
        }
    }
    static func stateLabel(_ state: String) -> String {
        switch state { case "imported": return "已导入目标项目，无需重复导入。"; case "updatable": return "目标项目已有此会话，可用归档中的新增消息更新。"; default: return "将导入为目标项目中的新会话。" }
    }
    static func resultLabel(_ status: String) -> String {
        switch status { case "updated": return "会话已更新"; case "skipped": return "会话已经存在，已跳过"; default: return "会话已导入" }
    }
}

struct NativeSessionImportProjectPicker: View {
    let projects: [ProjectSummary]
    let selected: String
    let close: () -> Void
    let select: (String) -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Text(localized("目标项目")).font(.headline); Spacer(); Button(localized("关闭"), action: close) }
            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(projects) { project in
                        Button { select(project.id) } label: {
                            HStack {
                                if selected == project.id { WispIcon(name: "check", size: 14) } else { Color.clear.frame(width: 14, height: 14) }
                                Text(project.name).frame(maxWidth: .infinity, alignment: .leading)
                            }.padding(8)
                        }.buttonStyle(.plain)
                    }
                }
            }.frame(maxHeight: 260)
        }.padding(16).frame(width: 330).background(NativeSettingsEscape(close: close))
    }
}
