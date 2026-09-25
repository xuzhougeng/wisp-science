import SwiftUI
import WispProjectBrowser

struct NativeAcpPermissionCard: View {
    @Environment(\.colorScheme) private var scheme
    @ObservedObject var conversation: NativeConversationModel
    let permission: ConversationAcpPermission
    private func scope(_ kind: String) -> String {
        switch kind {
        case "allow_once": return "允许这一次"
        case "allow_always": return "始终允许"
        case "reject_once": return "拒绝这一次"
        case "reject_always": return "始终拒绝"
        default: return "代理提供的选项"
        }
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("ACP 权限确认 · \(permission.title)").font(WispDesign.font(size: 13, weight: .semibold))
            if !permission.preview.isEmpty {
                ScrollView { Text(permission.preview).frame(maxWidth: .infinity, alignment: .leading).textSelection(.enabled) }
                    .font(WispDesign.font(size: 12, design: .monospaced)).frame(maxHeight: 110)
            }
            ForEach(permission.options) { option in
                Button { Task { await conversation.respondAcpPermission(permission, optionID: option.id) } } label: {
                    HStack(alignment: .firstTextBaseline) {
                        Text(option.name).multilineTextAlignment(.leading).foregroundStyle(WispDesign.color("text", scheme))
                        Spacer()
                        Text(scope(option.kind)).foregroundStyle(WispDesign.color("text-muted", scheme)).font(.caption).fixedSize()
                    }.frame(maxWidth: .infinity).padding(8)
                        .background(WispDesign.color("bg-app", scheme), in: RoundedRectangle(cornerRadius: 8))
                        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(WispDesign.color("border", scheme)))
                }.buttonStyle(.plain)
            }
            Button("取消请求") { Task { await conversation.respondAcpPermission(permission, optionID: nil) } }.buttonStyle(WispButtonStyle(compact: true))
        }
        .disabled(!conversation.canRespondAcpPermission(permission))
        .padding(16).background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(WispDesign.color("clay", scheme)))
        .accessibilityIdentifier("conversation-acp-permission")
    }
}
