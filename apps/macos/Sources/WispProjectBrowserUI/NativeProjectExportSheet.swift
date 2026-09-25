import AppKit
import SwiftUI
import UniformTypeIdentifiers
import WispProjectBrowser

@MainActor
final class NativeProjectExportModel: ObservableObject {
    let project: ProjectSummary
    private let client: any NativeSettingsQuerying
    @Published private(set) var busy = false
    @Published private(set) var error: String?
    @Published private(set) var destination: URL?

    init(project: ProjectSummary, client: any NativeSettingsQuerying) {
        self.project = project; self.client = client
    }

    func chooseDestination(directory: Bool) {
        guard !busy, destination == nil else { return }
        let panel = NSSavePanel()
        panel.title = directory ? "导出项目文件夹" : "导出 ZIP 归档"
        panel.prompt = "导出"
        panel.message = "导出包含项目记录与工作区文件；大型数据也会复制。"
        let name = project.name.replacingOccurrences(of: "/", with: "-").replacingOccurrences(of: ":", with: "-")
        panel.nameFieldStringValue = directory ? "wisp-project-\(name)" : "wisp-project-\(name).zip"
        if !directory { panel.allowedContentTypes = [.zip] }
        guard panel.runModal() == .OK else { return }
        let url = panel.url
        Task { await export(to: url, directory: directory) }
    }

    func export(to url: URL?, directory: Bool) async {
        guard let url, !busy, destination == nil else { return }
        busy = true; error = nil
        defer { busy = false }
        do {
            let format = directory ? "directory" : "zip"
            let result = try await client.invoke(NativeProjectCommand.exportProject,
                args: ["destination_path": .string(url.path), "format": .string(format)], projectID: project.id)
            guard result["project_id"].string == project.id,
                  result["format"].string == format,
                  result["destination_path"].string == url.path else { throw ProjectBrowserError.invalidResponse }
            destination = url
        } catch {
            self.error = "导出结果尚未确认。请检查目标位置；不会自动重试。\n" + error.localizedDescription
        }
    }
}

struct NativeProjectExportSheet: View {
    @StateObject private var model: NativeProjectExportModel
    let close: () -> Void
    init(project: ProjectSummary, client: any NativeSettingsQuerying, close: @escaping () -> Void) {
        _model = StateObject(wrappedValue: NativeProjectExportModel(project: project, client: client))
        self.close = close
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("导出项目：\(model.project.name)").font(.headline)
            Text("包含项目记录与工作区文件。大型数据也会复制；请先确认目标位置的可用空间，并等待运行中的会话和任务结束。")
                .font(.caption).foregroundStyle(.secondary)
            if let destination = model.destination {
                Text("导出完成").fontWeight(.semibold)
                Text(destination.path).font(.caption).textSelection(.enabled)
                Button("在 Finder 中显示") { NSWorkspace.shared.activateFileViewerSelecting([destination]) }
            } else {
                Button { model.chooseDestination(directory: false) } label: {
                    HStack { WispIcon(name: "archive"); Text("导出 ZIP 归档"); Spacer() }
                }.disabled(model.busy)
                Button { model.chooseDestination(directory: true) } label: {
                    HStack { WispIcon(name: "folder"); Text("导出项目文件夹"); Spacer() }
                }.disabled(model.busy)
            }
            if model.busy { HStack { ProgressView().controlSize(.small); Text("正在导出并校验…") } }
            if let error = model.error { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
            HStack { Spacer(); Button(model.destination == nil ? "取消" : "完成", action: close).disabled(model.busy) }
        }.padding(24).frame(width: 480).buttonStyle(WispButtonStyle())
            .interactiveDismissDisabled(model.busy)
            .background(NativeSettingsEscape(enabled: !model.busy, close: close))
    }
}
