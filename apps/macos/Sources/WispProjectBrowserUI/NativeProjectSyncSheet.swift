import SwiftUI
import WispProjectBrowser

@MainActor
final class NativeProjectSyncModel: ObservableObject {
    enum ConflictChoice: String, Identifiable {
        case local, remote
        var id: String { rawValue }
        var title: String { self == .local ? "保留本地版本" : "采用远端版本" }
        var detail: String {
            self == .local ? "将本地版本作为同步版本发布，替换远端对应的项目记录和受同步管理的文件。确认前请核对本地内容。"
                : "用远端版本替换对应的本机项目记录和受同步管理的文件。本地尚未同步的更改将不再作为当前版本，确认前可先导出备份。"
        }
    }
    let project: ProjectSummary
    private let client: any NativeSettingsQuerying
    @Published private(set) var busy = false
    @Published private(set) var configured: Bool
    @Published private(set) var conflict: Bool
    @Published private(set) var status: String
    @Published private(set) var error: String?
    @Published private(set) var result: SettingsValue?
    @Published var confirmation: ConflictChoice?

    init(project: ProjectSummary, client: any NativeSettingsQuerying) {
        self.project = project; self.client = client
        configured = project.folderSync != nil || project.syncConfigured
        conflict = project.folderSync == "conflict"
        status = NativeProjectSyncStatus(project)?.label ?? "尚未启用同步"
    }
    func enableFolderSync() async {
        guard !configured, !conflict else { return }
        await perform("enable_project_folder_sync")
    }
    func synchronize() async {
        guard configured, !conflict, confirmation == nil else { return }
        await perform("sync_project")
    }
    func choose(_ choice: ConflictChoice) { guard conflict, !busy else { return }; confirmation = choice }
    func resolve() async {
        guard conflict, let choice = confirmation else { return }
        await perform("resolve_project_sync", strategy: choice.rawValue)
    }
    private func perform(_ command: String, strategy: String? = nil) async {
        guard !busy else { return }
        busy = true; error = nil; result = nil
        defer { busy = false }
        var args: [String: SettingsValue] = ["id": .string(project.id)]
        if let strategy { args["strategy"] = .string(strategy) }
        do {
            let result = try await client.invoke(command, args: args, projectID: project.id)
            let labels = ["published": "本地版本已保存到项目文件夹", "pulled": "已采用远端新版本", "up-to-date": "已经是最新版本", "recovered": "同步已恢复", "synced": "同步完成", "remote-newer": "远端有更新版本，请再次同步"]
            guard let label = labels[result["status"].string] else { throw ProjectBrowserError.service("无法确认同步结果：" + result["status"].string) }
            self.result = result; status = label; configured = true; conflict = false; confirmation = nil
        } catch {
            status = "同步未能确认"
            if error.localizedDescription.localizedCaseInsensitiveContains("conflict") || error.localizedDescription.contains("冲突") { conflict = true; status = "项目同步存在冲突" }
            self.error = error.localizedDescription + "\n操作未能确认；不会自动重试或选择冲突版本。"
        }
    }
}

struct NativeProjectSyncSheet: View {
    @StateObject var model: NativeProjectSyncModel
    let close: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("项目同步：\(model.project.name)").font(.headline)
            Text(model.status).fontWeight(.medium)
            Text(model.project.workspaceDirectory).font(.caption).textSelection(.enabled)
            if !model.configured && !model.conflict {
                Text("把项目记录快照保存在工作区的 .wisp 中。网盘客户端负责同步文件；此操作不会上传到新的云服务。")
                    .font(.caption).foregroundStyle(.secondary)
                Button("启用项目文件夹快照") { Task { await model.enableFolderSync() } }.disabled(model.busy)
            } else if model.conflict {
                Text("本地与远端都发生了修改。请先核对内容，再选择需要保留的版本。")
                    .font(.caption).foregroundStyle(.secondary)
                HStack {
                    Button("保留本地版本…") { model.choose(.local) }
                    Button("采用远端版本…") { model.choose(.remote) }
                }.disabled(model.busy)
            } else {
                Button("立即同步") { Task { await model.synchronize() } }.disabled(model.busy)
            }
            if let result = model.result {
                if !result["revision"].string.isEmpty { Text("版本：" + result["revision"].string).font(.caption).textSelection(.enabled) }
                Text("上传 \(result["uploaded_files"].integer) 个文件 · 下载 \(result["downloaded_files"].integer) 个文件").font(.caption)
                if !result["skipped_paths"].array.isEmpty {
                    Text("跳过的路径：\n" + result["skipped_paths"].array.map(\.string).joined(separator: "\n"))
                        .font(.caption).textSelection(.enabled)
                }
            }
            if model.busy { HStack { ProgressView().controlSize(.small); Text("正在同步…") } }
            if let error = model.error {
                Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack { Spacer(); Button("关闭", action: close).disabled(model.busy) }
        }.padding(24).frame(width: 500).buttonStyle(WispButtonStyle())
            .interactiveDismissDisabled(model.busy || model.confirmation != nil)
            .background(NativeSettingsEscape(enabled: !model.busy && model.confirmation == nil, close: close))
            .sheet(item: $model.confirmation) { choice in NativeProjectSyncConflictSheet(model: model, choice: choice) }
    }
}

struct NativeProjectSyncConflictSheet: View {
    @ObservedObject var model: NativeProjectSyncModel
    let choice: NativeProjectSyncModel.ConflictChoice
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(choice.title).font(.headline)
            Text(choice.detail).fixedSize(horizontal: false, vertical: true)
            if let error = model.error {
                Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                if model.busy { ProgressView().controlSize(.small) }
                Spacer()
                Button("取消") { model.confirmation = nil }.disabled(model.busy)
                Button(choice.title) { Task { await model.resolve() } }.disabled(model.busy)
            }
        }.padding(24).frame(width: 440)
            .interactiveDismissDisabled(model.busy)
            .background(NativeSettingsEscape(enabled: !model.busy) { model.confirmation = nil })
    }
}
