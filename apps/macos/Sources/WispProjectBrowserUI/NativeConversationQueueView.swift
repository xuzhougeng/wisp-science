import SwiftUI
import WispProjectBrowser

struct NativeConversationQueueView: View {
    @ObservedObject var conversation: NativeConversationModel
    let session: String
    @State private var editing: ConversationQueueItem?
    @Environment(\.colorScheme) private var scheme
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(localized("待发送队列 · {c}").replacingOccurrences(of: "{c}", with: String(conversation.queuedTurns.count))).font(WispDesign.font(size: 12, weight: .semibold))
            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(Array(conversation.queuedTurns.enumerated()), id: \.element.id) { index, item in
                        HStack(alignment: .top, spacing: 8) {
                            Text(String(index + 1)).font(WispDesign.font(size: 11)).foregroundStyle(.secondary)
                            VStack(alignment: .leading, spacing: 3) {
                                Text(SavedAttachments.body(in: item.message)).font(WispDesign.font(size: 12)).lineLimit(3).textSelection(.enabled)
                                if !item.attachments.isEmpty {
                                    Text(item.attachments.map { ($0 as NSString).lastPathComponent }.joined(separator: " · "))
                                        .font(WispDesign.font(size: 11)).foregroundStyle(.secondary).lineLimit(1)
                                }
                                if item.state == "cutin_pending" { Text(localized("正在插入当前任务…")).font(.caption).foregroundStyle(.secondary) }
                            }.frame(maxWidth: .infinity, alignment: .leading)
                            if item.state == "queued" {
                                action("上移", icon: "arrow-up", enabled: index > 0 && conversation.queuedTurns[index - 1].state == "queued") { perform(item, "move_up") }
                                action("下移", icon: "chevron-down", enabled: index + 1 < conversation.queuedTurns.count) { perform(item, "move_down") }
                                action("编辑", icon: "edit") { editing = item }
                                action("取消排队", icon: "trash") { perform(item, "cancel") }
                            }
                        }.padding(8).background(WispDesign.color("bg-sunken", scheme), in: RoundedRectangle(cornerRadius: 8))
                            .disabled(!conversation.canChangeQueuedTurn(item))
                            .accessibilityIdentifier("queued-turn-" + item.id)
                    }
                }
            }.frame(maxHeight: 150)
        }.accessibilityIdentifier("queued-follow-ups")
            .sheet(item: $editing) { item in
                NativeQueuedTurnEditor(conversation: conversation, item: item, session: session) { editing = nil }
            }
            .onChange(of: conversation.snapshot?.session_id) { _ in editing = nil }
    }
    private func perform(_ item: ConversationQueueItem, _ action: String) {
        Task { await conversation.changeQueuedTurn(item, session: session, action: action) }
    }
    private func action(_ label: String, icon: String, enabled: Bool = true, perform: @escaping () -> Void) -> some View {
        Button(action: perform) { WispIcon(name: icon, size: 14).frame(width: 24, height: 24) }
            .buttonStyle(.plain).help(localized(label)).accessibilityLabel(localized(label)).disabled(!enabled)
    }
}

struct NativeQueuedTurnEditor: View {
    @ObservedObject var conversation: NativeConversationModel
    let item: ConversationQueueItem
    let session: String
    let close: () -> Void
    @State private var text: String
    @State private var submitting = false
    init(conversation: NativeConversationModel, item: ConversationQueueItem, session: String, close: @escaping () -> Void) {
        self.conversation = conversation; self.item = item; self.session = session; self.close = close
        _text = State(initialValue: SavedAttachments.body(in: item.message))
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(localized("编辑后续消息")).font(.headline)
            TextEditor(text: $text).font(WispDesign.font(size: 13)).frame(minHeight: 100, maxHeight: 220).accessibilityLabel(localized("后续消息内容")).disabled(submitting)
            if !item.attachments.isEmpty { Text(localized("已附加文件将保留")).font(.caption).foregroundStyle(.secondary) }
            if !conversation.canChangeQueuedTurn(item) && !submitting { Text(localized("该消息已开始或队列已更新，请关闭后查看最新队列。")).font(.caption).foregroundStyle(.orange) }
            if let error = conversation.operationError { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
            HStack {
                Spacer()
                Button(localized("取消"), action: close).disabled(submitting)
                Button(localized("保存")) {
                    submitting = true
                    Task {
                        let saved = await conversation.changeQueuedTurn(item, session: session, action: "edit", message: text)
                        submitting = false
                        if saved { close() }
                    }
                }.disabled(submitting || !conversation.canChangeQueuedTurn(item) || (text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && item.attachments.isEmpty && item.references.isEmpty))
            }
        }.padding(24).frame(minWidth: 420, idealWidth: 480)
            .interactiveDismissDisabled(submitting)
            .background(NativeSettingsEscape(enabled: !submitting, close: close))
    }
}
