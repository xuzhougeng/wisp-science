import Foundation
import SwiftUI
import WispProjectBrowser

/// Quick Look reads a private copy of the bytes already approved by the host;
/// an SSH provenance URI must never be treated as a local file URL.
enum NativeFilePreviewBytes {
    static func materialize(_ content: NativePanelFileContent) throws -> URL {
        guard !content.truncated, let base64 = content.base64, base64.utf8.count <= 45 * 1024 * 1024,
              let bytes = Data(base64Encoded: base64), bytes.count <= 32 * 1024 * 1024 else { throw ProjectBrowserError.invalidResponse }
        let name = (content.path as NSString).lastPathComponent
        guard !name.isEmpty, name != ".", name != "..", !name.contains("\0"), !name.contains("/") else { throw ProjectBrowserError.invalidResponse }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("wisp-native-preview-" + UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        let file = directory.appendingPathComponent(name)
        do { try bytes.write(to: file, options: .atomic); return file }
        catch { try? FileManager.default.removeItem(at: directory); throw error }
    }
    static func remove(_ file: URL) { try? FileManager.default.removeItem(at: file.deletingLastPathComponent()) }
}

struct NativeFileBytesPreview: View {
    let content: NativePanelFileContent
    @State private var file: URL?
    @State private var error: String?
    var body: some View {
        Group {
            if let file { NativeQuickLookPreview(url: file) }
            else if let error { Text(error).foregroundStyle(.orange).textSelection(.enabled) }
            else { ProgressView() }
        }.task(id: content.path) {
            do {
                let content = content
                let ready = try await Task.detached(priority: .userInitiated) { try NativeFilePreviewBytes.materialize(content) }.value
                guard !Task.isCancelled else { NativeFilePreviewBytes.remove(ready); return }
                file = ready
            } catch { if !Task.isCancelled { self.error = localized("无法预览文件：") + error.localizedDescription } }
        }.onDisappear { if let file { NativeFilePreviewBytes.remove(file) }; file = nil }
    }
}
