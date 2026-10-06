import Foundation
import WispProjectBrowser

enum NativePanelExportCopy {
    /// Copy original bytes off the UI thread, then replace the chosen destination
    /// atomically. Preview text and truncated binary prefixes are never exported.
    static func copy(_ source: NativePanelExport, to destination: URL) async throws {
        try await Task.detached(priority: .userInitiated) {
            let input = URL(fileURLWithPath: source.path).resolvingSymlinksInPath()
            guard input != destination.resolvingSymlinksInPath() else { throw ProjectBrowserError.unavailable("请选择与原文件不同的保存位置。") }
            let manager = FileManager.default
            let temporary = destination.deletingLastPathComponent().appendingPathComponent(".wisp-export-" + UUID().uuidString)
            defer { try? manager.removeItem(at: temporary) }
            try manager.copyItem(at: input, to: temporary)
            let size = try manager.attributesOfItem(atPath: temporary.path)[.size] as? NSNumber
            guard size?.uint64Value == source.total_bytes else { throw ProjectBrowserError.unavailable("源文件已变化，请重新保存副本。") }
            if manager.fileExists(atPath: destination.path) { _ = try manager.replaceItemAt(destination, withItemAt: temporary) }
            else { try manager.moveItem(at: temporary, to: destination) }
        }.value
    }
}
