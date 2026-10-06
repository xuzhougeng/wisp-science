import Foundation
import SwiftUI
import WispProjectBrowser

@MainActor final class NativeSessionArchiveImportModel: ObservableObject {
    @Published private(set) var project: String
    @Published private(set) var path = ""
    @Published private(set) var preview: NativeSessionArchivePreview?
    @Published private(set) var result: NativeSessionArchiveImportResult?
    @Published private(set) var error: String?
    @Published private(set) var reading = false
    @Published private(set) var importing = false
    @Published private(set) var uncertain = false
    private let client: any NativeConversationQuerying
    private let projects: Set<String>
    private let writable: () -> Bool
    private var generation = UUID()
    private var closed = false
    var canImport: Bool { writable() && !closed && !reading && !importing && !uncertain && result == nil && preview != nil && preview?.state != "imported" }
    init(client: any NativeConversationQuerying, project: String, projects: [String], writable: @escaping () -> Bool) {
        self.client = client; self.project = project; self.projects = Set(projects); self.writable = writable
    }
    func select(project: String, path: String) {
        guard !closed, !importing, projects.contains(project), self.project != project || self.path != path else { return }
        generation = UUID(); self.project = project; self.path = path; preview = nil; result = nil; error = nil; reading = false
        // Changing file or destination never clears an uncertain mutation.
    }
    func close() { closed = true; generation = UUID(); reading = false; importing = false }
    func readPreview() async {
        guard !closed, !importing, projects.contains(project), path.hasPrefix("/"), !path.contains("\0") else { return }
        let current = UUID(); generation = current; let project = project, path = path
        reading = true; preview = nil; result = nil; error = nil
        defer { if generation == current { reading = false } }
        do {
            let value = try await client.invoke("native_session_archive_preview", args: ["archive_path": .string(path)], projectID: project)
            let preview = try NativeSessionArchivePreview.decode(value, project: project, path: path)
            guard !closed, generation == current, !Task.isCancelled else { return }; self.preview = preview
        } catch { if !closed, generation == current, !Task.isCancelled { self.error = localized("无法读取会话归档预览。") + "\n" + error.localizedDescription } }
    }
    func confirm() async {
        guard canImport, let preview, preview.project_id == project, preview.archive_path == path else { return }
        let current = generation; importing = true; error = nil
        defer { if generation == current { importing = false } }
        do {
            let value = try await client.invoke("native_session_archive_import", args: preview.arguments, projectID: preview.project_id)
            let result = try NativeSessionArchiveImportResult.decode(value, reviewed: preview)
            guard !closed, generation == current, !Task.isCancelled else { return }; self.result = result
        } catch {
            if !closed, generation == current {
                uncertain = true; self.error = localized("导入结果未确认，请先在目标项目核对；本窗口不会再次提交。") + "\n" + error.localizedDescription
            }
        }
    }
}
