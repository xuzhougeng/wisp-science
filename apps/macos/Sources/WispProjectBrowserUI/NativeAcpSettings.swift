import SwiftUI
import WispProjectBrowser

struct NativeAcpSettings: View {
    @ObservedObject var conversation: NativeConversationModel
    @Environment(\.colorScheme) private var scheme
    var body: some View {
        if let state = conversation.snapshot?.acp_state {
            VStack(alignment: .leading, spacing: 8) {
                if !state.hasModeConfiguration, !state.modeChoices.isEmpty {
                    choice(label: localized("ACP 模式"), options: state.modeChoices, current: state.currentMode ?? "") { id in
                        Task { await conversation.setAcpMode(id) }
                    }
                }
                if !state.configurations.isEmpty {
                    DisclosureGroup(localized("ACP 配置")) {
                        ScrollView {
                            VStack(alignment: .leading, spacing: 8) {
                                ForEach(state.configurations) { option in configuration(option) }
                            }.frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 6)
                        }.frame(height: min(CGFloat(state.configurations.count * 34), 140))
                    }
                }
            }.font(WispDesign.font(size: 12)).disabled(!conversation.canChangeAcpSettings)
        }
    }
    @ViewBuilder private func configuration(_ option: ConversationAcpConfig) -> some View {
        if option.kind == "boolean", case .bool(let enabled) = option.current {
            Toggle(option.label, isOn: Binding(get: { enabled }, set: { value in Task { await conversation.setAcpConfig(option.id, value: .bool(value)) } }))
                .toggleStyle(.checkbox).help(option.description)
        } else if option.kind == "select", !option.choices.isEmpty {
            choice(label: option.label, options: option.choices, current: option.current.string) { id in
                Task { await conversation.setAcpConfig(option.id, value: .string(id)) }
            }.help(option.description)
        } else {
            Text(option.label + " · " + localized("此配置类型暂不可编辑")).foregroundStyle(.secondary)
        }
    }
    private func choice(label: String, options: [ConversationAcpChoice], current: String, select: @escaping (String) -> Void) -> some View {
        HStack(spacing: 8) {
            Text(label).foregroundStyle(.secondary).lineLimit(1)
            Menu {
                ForEach(options) { option in
                    Button { select(option.id) } label: {
                        HStack { Text(option.label); if current == option.id { WispIcon(name: "check", size: 12) } }
                    }
                }
            } label: { Text(options.first { $0.id == current }?.label ?? current).lineLimit(1) }
                .menuStyle(.borderlessButton).fixedSize().accessibilityLabel(label)
        }.foregroundStyle(WispDesign.color("text", scheme))
    }
}
