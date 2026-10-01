import SwiftUI
import WispProjectBrowser

/// Presentation of the existing usage transcript record, not a chat message.
struct NativeConversationUsage {
    let summary: String

    init?(_ item: ConversationItem) {
        guard item.role == "usage",
              let value = try? JSONDecoder().decode(SettingsValue.self, from: Data(item.text.utf8)),
              case .object = value else { return nil }
        func count(_ key: String) -> Int64? {
            guard case .integer(let count) = value[key], count >= 0 else { return nil }
            return count
        }
        var parts: [String] = []
        for (key, title) in [("input", "输入"), ("output", "输出"), ("reasoning", "思考"), ("cached", "缓存")] {
            if let count = count(key) { parts.append("\(localized(title)) \(count.formatted())") }
        }
        if let tokens = count("ctx_tokens") {
            let capacity = count("max_context")
            let context = capacity.flatMap { $0 > 0 ? "\(tokens.formatted()) / \($0.formatted())" : nil } ?? tokens.formatted()
            let percent = capacity.flatMap { $0 > 0 ? String(format: " (%.1f%%)", Double(tokens) / Double($0) * 100) : nil } ?? ""
            parts.append("\(localized("上下文用量")) \(context)\(percent)")
        }
        guard !parts.isEmpty else { return nil }
        summary = parts.joined(separator: " · ")
    }
}

struct NativeConversationUsageView: View {
    let usage: NativeConversationUsage
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            WispIcon(name: "gauge", size: 12)
                .accessibilityHidden(true)
            Text(usage.summary)
                .font(WispDesign.font(size: 11)).monospacedDigit()
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
        }
        .foregroundStyle(WispDesign.color("text-muted", scheme))
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 16).padding(.vertical, 2)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(localized("用量")) · \(usage.summary)")
        .accessibilityIdentifier("conversation-usage")
    }
}
