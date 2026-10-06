import AppKit
import SwiftUI
import UniformTypeIdentifiers
import WispProjectBrowser

@MainActor final class NativeSessionExportModel: ObservableObject {
    let source: BrowserSession
    @Published private(set) var includeArtifacts = true
    @Published private(set) var preview: NativeSessionExportPreview?
    @Published private(set) var result: NativeSessionExportResult?
    @Published private(set) var savePath: String?
    @Published private(set) var error: String?
    @Published private(set) var reading = false
    @Published private(set) var choosingDestination = false
    @Published private(set) var writing = false
    @Published private(set) var uncertain = false
    private let client: any NativeConversationQuerying
    private let writable: () -> Bool
    private var generation = UUID()
    private var closed = false
    var canExport: Bool { writable() && !closed && !reading && !writing && !choosingDestination && !uncertain && result == nil && preview?.include_artifacts == includeArtifacts }
    init(client: any NativeConversationQuerying, source: BrowserSession, writable: @escaping () -> Bool) {
        self.client = client; self.source = source; self.writable = writable
    }
    static func eligible(source: BrowserSession, snapshot: ConversationSnapshot?, busy: Bool, queued: Bool) -> Bool {
        guard let snapshot else { return false }
        return source.projectID == snapshot.project_id && source.id == snapshot.session_id
            && source.status != "deleted" && !busy && !queued && !snapshot.running && !snapshot.stopping
            && snapshot.approvals.isEmpty && snapshot.history_state?.reviewing != true
    }
    func close() { closed = true; generation = UUID(); reading = false; writing = false }
    func chooseArtifacts(_ selected: Bool) {
        guard !closed, !reading, !writing, !choosingDestination, result == nil, includeArtifacts != selected else { return }
        includeArtifacts = selected; generation = UUID(); preview = nil; error = nil
    }
    func readPreview() async {
        guard writable(), !Task.isCancelled, !closed, !writing, !choosingDestination, result == nil else { return }
        let current = UUID(); generation = current; let files = includeArtifacts
        reading = true; preview = nil; error = nil
        defer { if generation == current { reading = false } }
        do {
            let value = try await client.invoke("native_conversation_export_preview", args: ["session_id": .string(source.id), "include_artifacts": .bool(files)], projectID: source.projectID)
            let preview = try NativeSessionExportPreview.decode(value, project: source.projectID, session: source.id, includeArtifacts: files)
            guard !closed, generation == current, !Task.isCancelled else { return }; self.preview = preview
        } catch { if !closed, generation == current, !Task.isCancelled { self.error = localized("无法读取会话导出预览。") + "\n" + error.localizedDescription } }
    }
    func chooseDestination() {
        guard let preview else { return }
        selectDestination {
            let panel = NSSavePanel(); panel.allowedContentTypes = [.zip]; panel.nameFieldStringValue = preview.default_filename
            panel.title = localized("导出会话 ZIP"); panel.prompt = localized("导出")
            return panel.runModal() == .OK ? panel.url : nil
        }
    }
    func selectDestination(_ select: () -> URL?) {
        guard canExport else { return }
        choosingDestination = true
        let destination = select(); choosingDestination = false
        guard !closed, writable(), let destination else { return }
        Task { await export(to: destination) }
    }
    func export(to destination: URL) async {
        guard canExport, !Task.isCancelled, destination.isFileURL, destination.path.hasPrefix("/"),
              destination.pathExtension.lowercased() == "zip", let preview else { return }
        let current = generation; writing = true; error = nil; savePath = destination.path
        defer { if generation == current { writing = false } }
        do {
            let value = try await client.invoke("native_conversation_export", args: preview.arguments(destination: destination), projectID: source.projectID)
            let result = try NativeSessionExportResult.decode(value, reviewed: preview, destination: destination)
            guard !closed, generation == current else { return }
            try Task.checkCancellation(); self.result = result
        } catch {
            if !closed, generation == current {
                uncertain = true; self.error = localized("导出结果未确认，请核对保存位置；本窗口不会再次导出。") + "\n" + error.localizedDescription
            }
        }
    }
}
