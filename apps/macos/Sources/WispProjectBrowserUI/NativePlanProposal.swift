import SwiftUI
import WispProjectBrowser

struct NativePlanTarget: Codable, Equatable, Identifiable {
    let project: String
    let session: String
    let userIndex: Int
    let turn: ConversationTurnIdentity?
    let proposal: ConversationPlanProposal
    var id: String {
        let encoder = JSONEncoder(); encoder.outputFormatting = .sortedKeys
        return String(data: try! encoder.encode(self), encoding: .utf8)!
    }
}

struct NativePlanProposalCard: View {
    @ObservedObject var conversation: NativeConversationModel
    let item: ConversationItem
    let target: NativePlanTarget?
    @Environment(\.colorScheme) private var scheme
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 8) { WispIcon(name: "plan", size: 16); Text(localized("执行方案")).font(.headline) }
            if let proposal = item.proposal, proposal.valid {
                ForEach(Array(proposal.entries.enumerated()), id: \.offset) { _, entry in
                    HStack(alignment: .top, spacing: 12) {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(localized(entry.status == "completed" ? "已完成" : entry.status == "in_progress" ? "进行中" : "待执行"))
                            if entry.priority == "high" { Text(localized("高优先级")) }
                        }.font(.caption).foregroundStyle(.secondary).frame(width: 70, alignment: .leading)
                        NativeMessageBody(text: AttributedString(entry.content), markdown: entry.content, resources: [], saved: [], revealed: nil,
                                          monospaced: false, client: conversation.client, project: conversation.snapshot?.project_id ?? "", session: conversation.snapshot?.session_id ?? "", quote: nil, save: nil)
                    }
                }
            } else { Text(item.text).textSelection(.enabled) }
            if let target, conversation.proposalModeActive(target) || conversation.planDecisionUncertain(target) {
                Text(localized("修改计划时，请在输入框说明调整要求。批准后会先退出计划模式，再发送当前草稿；保存并退出不会启动执行。"))
                    .font(.caption).foregroundStyle(.secondary)
                HStack {
                    Button(localized("批准并执行")) { Task { await conversation.decidePlan(target, execute: true) } }.buttonStyle(WispButtonStyle(primary: true))
                    Button(localized("保存并退出")) { Task { await conversation.decidePlan(target, execute: false) } }.buttonStyle(WispButtonStyle())
                }.disabled(!conversation.canDecidePlan(target))
                if conversation.planDecisionUncertain(target) {
                    Button(localized("已核对计划模式")) { conversation.acknowledgePlanDecision(target) }
                        .disabled(conversation.busy || conversation.connectionError != nil)
                }
            }
        }.padding(16).frame(maxWidth: .infinity, alignment: .leading)
            .background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(WispDesign.color("border", scheme)))
    }
}
