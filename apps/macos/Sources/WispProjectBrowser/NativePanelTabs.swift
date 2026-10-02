import Foundation

/// Client-local layout; tab identifiers are stable across native shells.
/// Data-backed surfaces opt into `available` as they become implemented.
public struct NativePanelTabs: Equatable, Sendable {
    public static let defaults = ["artifacts", "agents", "files", "hosts"]
    public static let all = ["artifacts", "agents", "notebook", "highlights", "files", "provenance", "hosts", "sidechat"]
    public private(set) var open: [String]
    public private(set) var selected: String
    public let available: [String]

    public init(saved: String? = nil, selected: String = "artifacts", available: [String] = Self.defaults) {
        let supported = available.filter { Self.all.contains($0) }
        self.available = supported
        let decoded = saved.flatMap { $0.data(using: .utf8) }.flatMap { try? JSONDecoder().decode([String].self, from: $0) }
        var seen = Set<String>()
        open = (decoded ?? Self.defaults).filter { supported.contains($0) && seen.insert($0).inserted }
        self.selected = open.contains(selected) ? selected : (open.first ?? "artifacts")
    }
    public var saved: String { String(data: try! JSONEncoder().encode(open), encoding: .utf8)! }
    public mutating func show(_ id: String) {
        guard available.contains(id) else { return }
        if !open.contains(id) { open.append(id) }
        selected = id
    }
    public mutating func remove(_ id: String) {
        guard let index = open.firstIndex(of: id) else { return }
        open.remove(at: index)
        if selected == id, !open.isEmpty { selected = open[max(0, index - 1)] }
    }
    public mutating func move(_ id: String, to target: String) {
        guard let from = open.firstIndex(of: id), let to = open.firstIndex(of: target), from != to else { return }
        open.insert(open.remove(at: from), at: to)
    }
    /// Sidebar 「文件」 opens the existing files page. Visibility is client layout;
    /// the host is not called.
    public static func revealFiles(saved: String, selected: String) -> (saved: String, selected: String, visible: Bool) {
        var tabs = NativePanelTabs(saved: saved, selected: selected, available: NativePanelTabs.all)
        tabs.show("files")
        return (tabs.saved, tabs.selected, true)
    }

    public mutating func reopen() {
        if open.isEmpty {
            open = Self.defaults.filter { available.contains($0) }
            if open.isEmpty, let first = available.first { open = [first] }
            selected = open.first ?? "artifacts"
        }
    }
}
