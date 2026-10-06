import SwiftUI
import WispProjectBrowser

enum NativeApprovalScopeLabels {
    static func name(_ scope: String) -> String {
        switch scope { case "session": return "当前会话"; case "project": return "当前项目"; case "global": return "所有项目"; default: return "仅这一次" }
    }
    static func allow(_ scope: String) -> String {
        switch scope { case "session": return "在本会话中允许"; case "project": return "在本项目中允许"; case "global": return "在所有项目中允许"; default: return "允许这一次" }
    }
    static func explanation(_ scope: String) -> String {
        switch scope {
        case "session": return "允许此请求对应的工具授权用于当前会话；关闭宿主后不保留。"
        case "project": return "保存此请求对应的工具授权，当前项目的后续会话可直接使用。"
        case "global": return "保存此请求对应的工具授权，所有项目的后续会话可直接使用。"
        default: return "只允许当前请求，后续操作仍需确认。"
        }
    }
}

struct NativeToolApprovalCard: View {
    @ObservedObject var conversation: NativeConversationModel
    let approval: ConversationApproval
    let feedback: () -> Void
    @State private var scope = "once"
    @State private var scopePicker = false
    @Environment(\.colorScheme) private var scheme
    init(conversation: NativeConversationModel, approval: ConversationApproval, scope: String = "once", feedback: @escaping () -> Void) {
        self.conversation = conversation; self.approval = approval; self.feedback = feedback
        _scope = State(initialValue: scope)
    }
    private var scopes: [String] { conversation.approvalScopes(approval) }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("\(localized("需要确认")) · \(approval.tool)").font(WispDesign.font(size: 13, weight: .semibold))
            Text(approval.message).font(WispDesign.font(size: 13)).textSelection(.enabled)
            if !approval.preview.isEmpty {
                ViewThatFits(in: .vertical) {
                    Text(approval.preview).fixedSize(horizontal: false, vertical: true)
                    ScrollView { Text(approval.preview).frame(maxWidth: .infinity, alignment: .leading) }.frame(height: 130)
                }.font(WispDesign.font(size: 12, design: .monospaced)).textSelection(.enabled)
                    .frame(maxWidth: .infinity, maxHeight: 130, alignment: .leading).fixedSize(horizontal: false, vertical: true)
            }
            if scopes.count > 1 {
                HStack {
                    Text(localized("允许范围")).font(.caption).foregroundStyle(.secondary)
                    Button { scopePicker = true } label: {
                        HStack(spacing: 6) { Text(localized(NativeApprovalScopeLabels.name(scope))); WispIcon(name: "chevron-down", size: 12) }
                    }.buttonStyle(.plain).disabled(!conversation.canApprove(approval))
                        .popover(isPresented: $scopePicker) {
                            NativeApprovalScopePicker(scopes: scopes, selected: scope, close: { scopePicker = false }) { value in scope = value; scopePicker = false }
                        }
                    Spacer()
                }
                if scope != "once" { Text(localized(NativeApprovalScopeLabels.explanation(scope))).font(.caption).foregroundStyle(.secondary) }
            }
            ViewThatFits(in: .horizontal) {
                HStack { feedbackButton; Spacer(); denyButton; allowButton }
                VStack(alignment: .trailing, spacing: 8) {
                    HStack { feedbackButton; Spacer(); denyButton }
                    allowButton
                }
            }.disabled(!conversation.canApprove(approval))
            if conversation.approvalSubmitted(approval) {
                Text(localized("审批回复未确认或仍在更新。先重新核对请求，再决定是否提交；核对不会重发授权。")) .font(.caption).foregroundStyle(.secondary)
                Button(localized("重新读取并核对请求")) { Task { await conversation.reconcileApproval(approval) } }.disabled(conversation.busy)
            }
        }.padding(16).background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(WispDesign.color("clay", scheme)))
            .onChange(of: scopes) { choices in if !choices.contains(scope) { scope = "once" }; scopePicker = false }
            .onChange(of: conversation.canApprove(approval)) { available in if !available { scopePicker = false } }
    }
    private var feedbackButton: some View { Button(localized("修改意见…"), action: feedback).fixedSize() }
    private var denyButton: some View { Button(localized("拒绝")) { Task { await conversation.approve(approval, allowed: false) } }.fixedSize() }
    private var allowButton: some View {
        Button(localized(NativeApprovalScopeLabels.allow(scope))) { Task { await conversation.approve(approval, allowed: true, scope: scope) } }
            .buttonStyle(WispButtonStyle(primary: true)).fixedSize()
    }
}

struct NativeApprovalScopePicker: View {
    let scopes: [String]
    let selected: String
    let close: () -> Void
    let select: (String) -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack { Text(localized("允许范围")).font(.headline); Spacer(); Button(localized("关闭"), action: close) }
            ForEach(scopes, id: \.self) { scope in
                Button {
                    select(scope)
                } label: {
                    HStack(alignment: .top, spacing: 10) {
                        if selected == scope { WispIcon(name: "check", size: 14) } else { Color.clear.frame(width: 14, height: 14) }
                        VStack(alignment: .leading, spacing: 4) {
                            Text(localized(NativeApprovalScopeLabels.name(scope))).font(.subheadline).bold()
                            Text(localized(NativeApprovalScopeLabels.explanation(scope))).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                        }.frame(maxWidth: .infinity, alignment: .leading)
                    }.padding(8)
                }.buttonStyle(.plain)
            }
            Text(localized("已保存的授权可在设置的权限页面撤销。")) .font(.caption).foregroundStyle(.secondary)
        }.padding(16).frame(width: 350)
            .background(NativeSettingsEscape(close: close))
    }
}
