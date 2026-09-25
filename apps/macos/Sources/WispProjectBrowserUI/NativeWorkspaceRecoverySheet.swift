import SwiftUI
import WispProjectBrowser

struct NativeWorkspaceRecoverySheet: View {
    @ObservedObject var model: ProjectBrowserModel
    let preview: NativeWorkspaceRecoveryPreview
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("恢复工作区历史").font(.headline)
            Text("从旧工作区的历史归档恢复消息到新会话。历史归档不包含完整项目记录；原始文件保持不变。")
                .font(.caption).foregroundStyle(.secondary)
            Text(preview.workspace_dir).font(.caption).textSelection(.enabled).lineLimit(3)
            TextField("项目名称", text: $model.recoveryName).textFieldStyle(.roundedBorder).disabled(model.importBusy)
            Text("\(preview.recoverable_session_count) 个会话 · \(preview.message_count) 条消息 · \(preview.valid_archive_count) 份有效归档")
                .font(WispDesign.font(size: 13, weight: .medium))
            if let first = preview.earliest_message_at, let last = preview.latest_message_at {
                Text("\(Date(timeIntervalSince1970: TimeInterval(first)).formatted(date: .abbreviated, time: .shortened)) — \(Date(timeIntervalSince1970: TimeInterval(last)).formatted(date: .abbreviated, time: .shortened))")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if preview.invalid_archive_count + preview.duplicate_archive_count > 0 {
                Text("将跳过 \(preview.invalid_archive_count) 份无效归档和 \(preview.duplicate_archive_count) 份重复归档。")
                    .font(.caption).foregroundStyle(.orange)
            }
            if preview.recoverable_session_count == 0 { Text("没有可恢复的会话。").font(.caption).foregroundStyle(.orange) }
            if let error = model.importError { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
            HStack {
                if model.importBusy { ProgressView().controlSize(.small) }
                Spacer()
                Button("取消") { model.recoveryPreview = nil }.disabled(model.importBusy)
                Button("恢复历史") { Task { await model.recoverWorkspace() } }
                    .disabled(model.importBusy || preview.recoverable_session_count == 0 || model.recoveryName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }.padding(24).frame(width: 480)
            .interactiveDismissDisabled(model.importBusy)
            .background(NativeSettingsEscape(enabled: !model.importBusy) { model.recoveryPreview = nil })
    }
}
