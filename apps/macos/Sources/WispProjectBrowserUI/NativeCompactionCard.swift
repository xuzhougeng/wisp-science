import SwiftUI
import WispProjectBrowser

struct NativeTranscriptCompaction {
    let before: Int64
    let after: Int64
    let strategy: String
    let checkpoint: String?
    let undone: Bool
    init?(_ item: ConversationItem) {
        guard item.role == "compaction", let value = try? JSONDecoder().decode(SettingsValue.self, from: Data(item.text.utf8)),
              case .integer(let before) = value["before"], before >= 0,
              case .integer(let after) = value["after"], after >= 0, !value["strategy"].string.isEmpty else { return nil }
        self.before = before; self.after = after; strategy = value["strategy"].string
        checkpoint = value["checkpoint"].string.isEmpty ? nil : value["checkpoint"].string
        undone = value["undone"].bool
    }
}
struct NativeCompactionCard: View {
    let record: NativeTranscriptCompaction
    let openContext: (() -> Void)?
    @State private var checkpointExpanded = false
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                WispIcon(name: "archive", size: 14)
                Text(localized(record.undone ? "上下文压缩已撤销" : "上下文已压缩")).font(.subheadline).bold()
                Spacer()
                if let openContext { Button(localized("查看上下文"), action: openContext) }
            }
            Text("\(record.before.formatted()) → \(record.after.formatted()) tokens · \(record.strategy)").font(.caption).monospacedDigit().foregroundStyle(.secondary)
            if let checkpoint = record.checkpoint {
                DisclosureGroup(localized("检查点"), isExpanded: $checkpointExpanded) {
                    Text(checkpoint).font(.system(size: 12)).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }.padding(12).frame(maxWidth: .infinity, alignment: .leading)
    }
}
