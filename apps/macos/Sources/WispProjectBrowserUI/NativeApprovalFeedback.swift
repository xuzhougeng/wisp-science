import SwiftUI
import WispProjectBrowser

struct NativeApprovalFeedback: View {
    let approval: ConversationApproval
    @ObservedObject var conversation: NativeConversationModel
    let close: () -> Void
    @State private var feedback = ""
    @State private var submitting = false
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("修改意见").font(.headline)
            Text(approval.message).textSelection(.enabled)
            Text("拒绝这次请求，并将修改意见交给助手。").font(.caption).foregroundStyle(.secondary)
            TextEditor(text: $feedback).font(WispDesign.font(size: 13)).frame(minHeight: 90, maxHeight: 180)
                .accessibilityLabel("审批修改意见").disabled(submitting)
            if !submitting && !conversation.canApprove(approval) {
                Text("审批请求已不可用，请关闭后查看最新状态。").font(.caption).foregroundStyle(.orange)
            }
            if let error = conversation.operationError { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
            HStack {
                Spacer()
                Button("取消", action: close).disabled(submitting)
                Button("拒绝并反馈") {
                    submitting = true
                    Task { @MainActor in
                        let saved = await conversation.approve(approval, allowed: false, feedback: feedback)
                        submitting = false
                        if saved { close() }
                    }
                }.disabled(submitting || !conversation.canApprove(approval) || feedback.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }.padding(24).frame(width: 460)
            .interactiveDismissDisabled(submitting)
            .background(NativeSettingsEscape(enabled: !submitting, close: close))
    }
}
