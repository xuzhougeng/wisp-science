import SwiftUI
import WispProjectBrowser

struct NativeHistoryTarget: Equatable, Identifiable {
    var id: String { session + ":" + String(turn.user_seq) + ":" + kind }
    let project: String
    let session: String
    let turn: ConversationTurnIdentity
    let revision: String
    let kind: String
    let checkpoint: String?
    let draft: String
    var title: String { kind == "rewind" ? "回退到这条消息" : checkpoint == "before_user" ? "编辑并创建分支" : "从回复后创建分支" }
    var action: SettingsValue {
        var fields: [String: SettingsValue] = ["kind": .string(kind)]
        if let checkpoint { fields["checkpoint"] = .string(checkpoint) }
        return .object(fields)
    }
}

struct NativeHistoryActionSheet: View {
    @ObservedObject var conversation: NativeConversationModel
    let target: NativeHistoryTarget
    let close: () -> Void
    let openBranch: (String) -> Void
    @State private var edited: String
    init(conversation: NativeConversationModel, target: NativeHistoryTarget, close: @escaping () -> Void, openBranch: @escaping (String) -> Void) {
        self.conversation = conversation; self.target = target; self.close = close; self.openBranch = openBranch
        _edited = State(initialValue: target.draft)
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack { Text(localized(target.title)).font(.headline); Spacer(); Button(localized("关闭"), action: close).disabled(conversation.busy) }
            Text(localized(target.kind == "rewind"
                ? "删除这条消息及之后的对话记录，并把原始问题恢复到输入区。现有草稿会保留；文件更改不会回滚。"
                : "保留当前会话，在所选历史位置创建一个新分支。编辑后的问题保存在分支输入区，确认后可发送。"))
                .font(.caption).foregroundStyle(.secondary)
            if target.checkpoint == "before_user" {
                TextEditor(text: $edited).font(.system(size: 13)).frame(minHeight: 160).disabled(conversation.busy)
                    .accessibilityLabel(localized("分支问题"))
            } else {
                ScrollView { Text(target.draft).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }.frame(maxHeight: 180)
            }
            if let error = conversation.operationError { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
            if !conversation.busy && !conversation.canHistoryAction(target) {
                Text(localized("历史位置或会话状态已变化，请关闭后重新选择。")) .font(.caption).foregroundStyle(.orange)
            }
            HStack {
                Button(localized("取消"), action: close).disabled(conversation.busy)
                Spacer()
                Button(localized(target.kind == "rewind" ? "确认回退" : "创建分支")) {
                    Task {
                        if let result = await conversation.performHistoryAction(target, editedDraft: edited) {
                            close()
                            if !result.isEmpty { openBranch(result) }
                        }
                    }
                }.buttonStyle(WispButtonStyle(primary: true)).disabled(!conversation.canHistoryAction(target))
            }
        }.padding(24).frame(width: 560, height: 360)
            .interactiveDismissDisabled(conversation.busy)
            .background(NativeSettingsEscape(enabled: !conversation.busy, close: close))
    }
}
