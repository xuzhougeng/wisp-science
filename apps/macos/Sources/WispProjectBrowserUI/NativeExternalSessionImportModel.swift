import Foundation
import SwiftUI
import WispProjectBrowser

@MainActor final class NativeExternalSessionImportModel: ObservableObject {
    static let pageSize = 25
    @Published private(set) var project: String
    @Published private(set) var provider = "codex"
    @Published private(set) var context = "local"
    @Published private(set) var sources: [NativeExternalImportSource] = []
    @Published private(set) var items: [NativeExternalImportItem] = []
    @Published private(set) var results: [String: NativeExternalImportResult] = [:]
    @Published private(set) var itemErrors: [String: String] = [:]
    @Published private(set) var preview: NativeExternalImportPreview?
    @Published private(set) var selectedPath: String?
    @Published private(set) var query = ""
    @Published private(set) var page = 0
    @Published private(set) var loading = false
    @Published private(set) var sourcesLoading = false
    @Published private(set) var previewing = false
    @Published private(set) var importing = false
    @Published private(set) var uncertain = false
    @Published private(set) var stopRequested = false
    @Published private(set) var error: String?
    @Published private(set) var sourcesError: String?
    @Published private(set) var done = 0
    @Published private(set) var total = 0
    @Published private(set) var imported = 0
    @Published private(set) var updated = 0
    @Published private(set) var skipped = 0
    @Published private(set) var failed = 0
    private let client: any NativeConversationQuerying
    private let projects: Set<String>
    private let writable: () -> Bool
    private var selection = UUID(), sourcesRead = UUID(), listRead = UUID(), previewRead = UUID()
    private var closed = false
    var filtered: [NativeExternalImportItem] {
        let needle = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return items.filter { needle.isEmpty || ($0.title + " " + $0.cwd + " " + $0.session_id + " " + $0.path).localizedCaseInsensitiveContains(needle) }
    }
    var pageCount: Int { max(1, (filtered.count + Self.pageSize - 1) / Self.pageSize) }
    var pageItems: [NativeExternalImportItem] { Array(filtered.dropFirst(page * Self.pageSize).prefix(Self.pageSize)) }
    var canImport: Bool {
        guard let preview else { return false }
        return writable() && !closed && !sourcesLoading && !loading && !previewing && !importing && !uncertain && results[preview.path] == nil
    }
    var canImportFiltered: Bool { writable() && !closed && !sourcesLoading && !loading && !previewing && !importing && !uncertain && filtered.contains { $0.state != "imported" && results[$0.path] == nil } }
    init(client: any NativeConversationQuerying, project: String, projects: [String], writable: @escaping () -> Bool) {
        self.client = client; self.project = project; self.projects = Set(projects); self.writable = writable
    }
    func close() { closed = true; stopRequested = true; selection = UUID(); sourcesRead = UUID(); listRead = UUID(); clearPreview(); sourcesLoading = false; loading = false; importing = false }
    func clearPreview() { guard !importing || closed else { return }; previewRead = UUID(); preview = nil; selectedPath = nil; previewing = false }
    func setQuery(_ value: String) { guard !closed, !importing, query != value else { return }; query = value; page = 0; clearPreview() }
    func setPage(_ value: Int) { guard !closed, !importing else { return }; page = max(0, min(value, pageCount - 1)); clearPreview() }
    func select(project: String, provider: String, context: String) {
        guard !closed, !importing, projects.contains(project), ["codex", "claude"].contains(provider),
              sources.contains(where: { $0.id == context }), (self.project, self.provider, self.context) != (project, provider, context) else { return }
        selection = UUID(); sourcesRead = UUID(); listRead = UUID(); clearPreview(); self.project = project; self.provider = provider; self.context = context; sourcesLoading = false
        items = []; results = [:]; itemErrors = [:]; page = 0; loading = false; error = nil
        done = 0; total = 0; imported = 0; updated = 0; skipped = 0; failed = 0
    }
    func initialize() async {
        guard !closed, !importing, projects.contains(project) else { return }
        let current = selection; let read = UUID(); sourcesRead = read; sourcesLoading = true; let destination = project
        defer { if !closed, selection == current, sourcesRead == read { sourcesLoading = false } }
        do {
            let value = try await client.invoke("native_external_session_sources", args: [:], projectID: destination)
            let sources = try NativeExternalImportSources.decode(value, project: destination)
            guard !closed, selection == current, sourcesRead == read, !Task.isCancelled else { return }; self.sources = sources.sources; sourcesError = nil
        } catch {
            if !closed, selection == current, sourcesRead == read, !Task.isCancelled { sourcesError = localized("无法读取会话来源。") + "\n" + error.localizedDescription }
        }
        guard !closed, selection == current, sourcesRead == read, !Task.isCancelled, sources.contains(where: { $0.id == context }) else { return }
        await load(refresh: false)
    }
    func load(refresh: Bool) async {
        guard !closed, !importing, projects.contains(project), sources.contains(where: { $0.id == context }) else { return }
        let current = UUID(); listRead = current; let project = project, provider = provider, context = context
        clearPreview(); loading = true; items = []; results = [:]; itemErrors = [:]; page = 0; error = nil
        defer { if !closed, listRead == current { loading = false } }
        do {
            let value = try await client.invoke("native_external_session_list", args: ["provider": .string(provider), "context_id": .string(context), "refresh": .bool(refresh)], projectID: project)
            let result = try NativeExternalImportList.decode(value, project: project, provider: provider, context: context)
            guard !closed, listRead == current, !Task.isCancelled else { return }; items = result.items
        } catch { if !closed, listRead == current, !Task.isCancelled { self.error = localized("无法读取来源会话列表。") + "\n" + error.localizedDescription } }
    }
    private func read(_ item: NativeExternalImportItem, project: String, provider: String, context: String) async throws -> NativeExternalImportPreview {
        let value = try await client.invoke("native_external_session_preview", args: ["provider": .string(provider), "context_id": .string(context), "path": .string(item.path)], projectID: project)
        return try NativeExternalImportPreview.decode(value, project: project, provider: provider, context: context, item: item)
    }
    func inspect(_ item: NativeExternalImportItem) async {
        guard !closed, !loading, !importing, items.contains(item) else { return }
        clearPreview(); let current = previewRead; selectedPath = item.path; previewing = true; error = nil
        defer { if !closed, previewRead == current { previewing = false } }
        do {
            let result = try await read(item, project: project, provider: provider, context: context)
            guard !closed, previewRead == current, !Task.isCancelled else { return }; preview = result
        } catch { if !closed, previewRead == current, !Task.isCancelled { self.error = localized("无法预览来源会话。") + "\n" + error.localizedDescription } }
    }
    func importPreview() async {
        guard canImport, let preview, let item = items.first(where: { $0.path == preview.path }) else { return }
        await importBatch([item], single: preview)
    }
    func importFiltered() async {
        guard canImportFiltered else { return }
        await importBatch(filtered.filter { $0.state != "imported" && results[$0.path] == nil }, single: nil)
    }
    func stopAfterCurrent() { if importing { stopRequested = true } }
    private func importBatch(_ candidates: [NativeExternalImportItem], single: NativeExternalImportPreview?) async {
        guard !candidates.isEmpty, !closed, !importing, !uncertain, writable() else { return }
        let current = selection; let project = project, provider = provider, context = context
        importing = true; stopRequested = false; error = nil; total = candidates.count; done = 0; imported = 0; updated = 0; skipped = 0; failed = 0
        defer { if !closed, selection == current { importing = false } }
        for item in candidates {
            guard !closed, !stopRequested, selection == current, writable(), !Task.isCancelled else { break }
            var writing = false
            do {
                let reviewed: NativeExternalImportPreview
                if let single { reviewed = single } else { reviewed = try await read(item, project: project, provider: provider, context: context) }
                guard !closed, !stopRequested, selection == current, writable(), !Task.isCancelled else { break }
                writing = true
                let value = try await client.invoke("native_external_session_import", args: reviewed.arguments, projectID: reviewed.project_id)
                let result = try NativeExternalImportResult.decode(value, reviewed: reviewed)
                guard !closed, selection == current else { break }
                results[item.path] = result; itemErrors[item.path] = nil
                if let index = items.firstIndex(where: { $0.path == item.path }) { items[index].state = "imported"; items[index].message_count = result.message_count }
                if result.status == "imported" { imported += 1 } else if result.status == "updated" { updated += 1 } else { skipped += 1 }
            } catch {
                guard !closed, selection == current else { break }
                if !writing && Task.isCancelled { break }
                failed += 1; itemErrors[item.path] = error.localizedDescription
                if writing { uncertain = true; self.error = localized("当前导入结果未确认，已停止后续导入且不会重试。请先在目标项目核对。") + "\n" + error.localizedDescription }
            }
            done += 1
            if uncertain { break }
        }
    }
}
