import Foundation

/// Presentation-only grouping of existing native conversation items. This does
/// not discard messages or change the DTO / remote command contract.
public enum RemoteTranscript {
    public struct Group: Identifiable, Equatable, Sendable {
        public let id: Int
        public let items: [RemoteValue]
        public let isProcess: Bool

        public var toolCount: Int { items.filter { RemoteTranscript.isTool($0) }.count }
        public var changedFileCount: Int {
            Set(items.filter { $0["role"].string == "file_changed" }.map { $0["text"].string }).count
        }
        public var failureCount: Int {
            items.filter {
                RemoteTranscript.isTool($0) && ($0["ok"] == .bool(false)
                    || ["failed", "timed_out", "lost"].contains($0["status"].string))
            }.count
        }
    }

    // These tool rows contain deliverables or run status rather than internal
    // steps. Keep them outside the group, matching the remote web client.
    private static let visibleTools: Set<String> = [
        "attempt_completion", "monitor_run", "wisp_monitor_run", "generate_image", "generate_video",
    ]
    private static func isTool(_ item: RemoteValue) -> Bool {
        ["tool", "acp_tool"].contains(item["role"].string)
    }
    private static func isStep(_ item: RemoteValue) -> Bool {
        isTool(item) && !visibleTools.contains(item["tool_name"].string)
    }

    /// `indexOffset = -older.count` keeps existing live-row identities stable
    /// when an earlier history page is prepended. Reset the view at epoch/window
    /// changes, since positions then refer to a different snapshot window.
    public static func groups(_ items: [RemoteValue], running: Bool = false, indexOffset: Int = 0) -> [Group] {
        let liveFrom = running ? (items.lastIndex { ["user", "queued_user"].contains($0["role"].string) } ?? -1) : items.count
        // Look ahead once, ignoring reasoning, file notifications and empty
        // assistant placeholders, to distinguish commentary from final answers.
        var commentary = Array(repeating: false, count: items.count)
        var nextIsStep = false
        for index in items.indices.reversed() {
            let item = items[index]
            let role = item["role"].string
            if role == "assistant" {
                commentary[index] = nextIsStep && index <= liveFrom && item["resources"].array.isEmpty
                if !item["text"].string.isEmpty || !item["resources"].array.isEmpty { nextIsStep = false }
            } else if role != "reasoning" && role != "file_changed" {
                nextIsStep = isStep(item)
            }
        }

        var result: [Group] = []
        var pending: [RemoteValue] = []
        var start = 0
        func flush() {
            guard !pending.isEmpty else { return }
            result.append(Group(id: start + indexOffset, items: pending, isProcess: true))
            pending = []
        }
        for (index, item) in items.enumerated() {
            let process = isStep(item) || ["reasoning", "file_changed"].contains(item["role"].string) || commentary[index]
            if process {
                if pending.isEmpty { start = index }
                pending.append(item)
            } else {
                flush()
                result.append(Group(id: index + indexOffset, items: [item], isProcess: false))
            }
        }
        flush()
        return result
    }

    /// How much of a turn's intermediate work the transcript shows. A per-device
    /// presentation choice; contracts/native-conversations/v1/transcript-detail.json
    /// pins the rows each level produces for the iOS and Android clients.
    public enum DetailLevel: String, CaseIterable, Identifiable, Sendable {
        case minimal, standard, full
        public static let `default` = DetailLevel.standard
        public var id: String { rawValue }
        public var label: String {
            switch self {
            case .minimal: return "简洁"
            case .standard: return "标准"
            case .full: return "详细"
            }
        }
    }

    public enum Row: Identifiable, Equatable, Sendable {
        /// One item shown on its own; `id` is its position including `indexOffset`.
        case item(id: Int, RemoteValue)
        /// Folded process steps, as produced by `groups`.
        case process(Group)
        /// A process fold reduced to its failure count.
        case failures(Group)

        public var id: Int {
            switch self {
            case .item(let id, _): return id
            case .process(let group), .failures(let group): return group.id
            }
        }
    }

    /// `standard` is exactly `groups`. `minimal` keeps only process folds that
    /// failed, as failure rows, and hides usage. `full` puts every item on its
    /// own row. Row identities follow the same positions as `groups`.
    public static func rows(_ items: [RemoteValue], level: DetailLevel, running: Bool = false, indexOffset: Int = 0) -> [Row] {
        if level == .full {
            return items.enumerated().map { .item(id: $0.offset + indexOffset, $0.element) }
        }
        return groups(items, running: running, indexOffset: indexOffset).compactMap { group in
            guard group.isProcess else {
                if level == .minimal && group.items.first?["role"].string == "usage" { return nil }
                return .item(id: group.id, group.items[0])
            }
            guard level == .minimal else { return .process(group) }
            return group.failureCount > 0 ? .failures(group) : nil
        }
    }
}
