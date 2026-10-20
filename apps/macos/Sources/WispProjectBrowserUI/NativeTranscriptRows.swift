import Foundation
import WispProjectBrowser

/// One transcript row with an identity that survives snapshot replacement.
struct NativeTranscriptRow: Identifiable {
    let id: String
    let index: Int
    let item: ConversationItem
}

enum NativeTranscriptRows {
    /// Keys rows by the host's persisted message seq so SwiftUI keeps row
    /// views across the 350 ms snapshot polls. Rows the host cannot bind use
    /// their request or epoch identity; only live rows without any persisted
    /// identity fall back to the page offset.
    static func rows(_ items: [ConversationItem]) -> [NativeTranscriptRow] {
        var ordinals: [Int64: Int] = [:]
        var used: Set<String> = []
        return items.enumerated().map { index, item in
            var key: String
            if let seq = item.seq {
                let ordinal = ordinals[seq, default: 0]
                ordinals[seq] = ordinal + 1
                key = "seq:\(seq)/\(ordinal)"
            } else if item.role == "question", let id = NativeQuestion(item.text)?.requestID {
                key = "question:" + id
            } else if item.role == "compaction", let epoch = compactionEpoch(item.text) {
                key = "compaction:\(epoch)"
            } else {
                key = "row:\(index)"
            }
            if !used.insert(key).inserted {
                key += "#\(index)"
                used.insert(key)
            }
            return NativeTranscriptRow(id: key, index: index, item: item)
        }
    }

    static func compactionEpoch(_ text: String) -> Int64? {
        guard let value = try? JSONDecoder().decode(SettingsValue.self, from: Data(text.utf8)),
              case .integer(let epoch) = value["epoch"] else { return nil }
        return epoch
    }
}
