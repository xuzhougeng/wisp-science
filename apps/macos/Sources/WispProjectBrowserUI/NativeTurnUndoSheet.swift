import SwiftUI
import WispProjectBrowser

struct NativeTurnUndoSheet: View {
    @ObservedObject var conversation: NativeConversationModel
    let target: NativeHistoryTarget
    let close: () -> Void
    @State private var preview: NativeTurnUndoPreview?
    @State private var loading = false
    @State private var error: String?
    private let loadOnAppear: Bool
    init(conversation: NativeConversationModel, target: NativeHistoryTarget, preview: NativeTurnUndoPreview? = nil, close: @escaping () -> Void) {
        self.conversation = conversation; self.target = target; self.close = close
        _preview = State(initialValue: preview); loadOnAppear = preview == nil
    }
    private var canConfirm: Bool { preview?.conflicts.isEmpty == true && !loading && conversation.canHistoryAction(target) }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack { Text(localized("撤销本轮")).font(.headline); Spacer(); Button(localized("关闭"), action: close).disabled(conversation.busy) }
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    Text(localized("撤销最新一轮的对话和已记录的文本文件更改。现有草稿会保留，原始问题会恢复到空输入区。"))
                    Text(localized("二进制文件和未记录的更改无法恢复；下方列出了实际处理范围。")) .font(.caption).foregroundStyle(.secondary)
                    if loading { ProgressView(localized("正在检查文件更改…")) }
                    if let preview {
                        fileList("恢复文本文件", preview.restore_files)
                        fileList("删除新建文件", preview.remove_files)
                        fileList("移除产物记录", preview.remove_artifacts)
                        fileList("无法恢复的文件", preview.unsupported_files)
                        fileList("冲突：请先处理这些文件", preview.conflicts)
                        if preview.restore_files.isEmpty && preview.remove_files.isEmpty { Text(localized("没有可撤销的文本文件更改。")) .foregroundStyle(.secondary) }
                    }
                    if let error = error ?? conversation.operationError { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
                    if !loading && !conversation.busy && !conversation.canHistoryAction(target) { Text(localized("历史位置或会话状态已变化，请关闭后重新选择。")) .font(.caption).foregroundStyle(.orange) }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }
            HStack {
                Button(localized("重新检查")) { Task { await load() } }.disabled(loading || conversation.busy || !conversation.canHistoryAction(target))
                Spacer()
                Button(localized("取消"), action: close).disabled(conversation.busy)
                Button(localized("确认撤销")) {
                    guard canConfirm else { return }
                    Task { if await conversation.performHistoryAction(target, editedDraft: "") != nil { close() } }
                }.disabled(!canConfirm).buttonStyle(WispButtonStyle(primary: true))
            }
        }.padding(24).frame(minWidth: 320, idealWidth: 560, maxWidth: 560, minHeight: 450, idealHeight: 450, maxHeight: 600)
            .interactiveDismissDisabled(conversation.busy)
            .background(NativeSettingsEscape(enabled: !conversation.busy, close: close)).task { if loadOnAppear { await load() } }
    }
    @ViewBuilder private func fileList(_ title: String, _ values: [String]) -> some View {
        if !values.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                Text(localized(title)).font(.subheadline).bold()
                ForEach(Array(values.enumerated()), id: \.offset) { _, path in Text(path).font(.system(size: 12, design: .monospaced)).textSelection(.enabled) }
            }
        }
    }
    private func load() async {
        guard !loading else { return }
        loading = true; error = nil; preview = nil
        defer { loading = false }
        do {
            let result = try await conversation.previewUndo(target)
            guard !Task.isCancelled else { return }
            preview = result
        } catch { if !Task.isCancelled { self.error = localized("无法读取撤销预览，请重新检查。") + "\n" + error.localizedDescription } }
    }
}
