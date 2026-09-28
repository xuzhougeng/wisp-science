import AppKit
import SwiftUI

/// Read-only actions capture this message, never mutate or resend a turn.
struct NativeMessageActions: View {
    let source: String
    let quote: ((String) -> Void)?
    let save: ((String) -> Void)?
    @State private var copied = false
    var body: some View {
        HStack(spacing: 14) {
            Button {
                NSPasteboard.general.clearContents(); NSPasteboard.general.setString(source, forType: .string)
                copied = true
            } label: { WispIcon(name: "copy", size: 14) }.help(localized("复制消息")).accessibilityLabel(localized("复制消息"))
            if let quote { Button { quote(source) } label: { WispIcon(name: "chat", size: 14) }.help(localized("引用到侧聊")).accessibilityLabel(localized("引用到侧聊")) }
            if let save { Button { save(source) } label: { WispIcon(name: "star", size: 14) }.help(localized("收藏消息")).accessibilityLabel(localized("收藏消息")) }
            if copied { Text(localized("已复制")).font(.caption) }
        }.buttonStyle(.plain).foregroundStyle(.secondary)
            .onChange(of: source) { _ in copied = false }
    }
}
