import Foundation
import WispProjectBrowser

@MainActor
final class NativePanelModel: ObservableObject {
    @Published private(set) var notebookStars: [NativeNotebookStar] = []
    @Published private(set) var notebookLoaded = false
    @Published private(set) var notebookBusy: Set<String> = []
    @Published private(set) var highlights: [NativeHighlight] = []
    @Published private(set) var highlightRemoving: Set<String> = []
    @Published private(set) var artifacts: [NativePanelArtifact] = []
    @Published private(set) var searchHits: [NativePanelSearchHit] = []
    @Published private(set) var searchLoading = false
    private var searchQuery = ""
    private var searchGeneration = UUID()
    @Published private(set) var files: [NativePanelFile] = []
    @Published private(set) var path = "."
    @Published private(set) var loading = false
    @Published private(set) var error: String?
    @Published private(set) var contexts: NativePanelContexts?
    @Published private(set) var contextBusy = false
    @Published private(set) var agentDelegationEnabled: Bool?
    @Published private(set) var agentDelegationBusy = false
    @Published private(set) var agents: [NativeAgentSnapshot] = []
    @Published private(set) var agentLaunching: Set<String> = []
    @Published private(set) var agentActions: Set<String> = []
    private var agentEpoch = UUID()
    private var selectedTab = "artifacts"
    @Published var agentResult: NativeAgentResult?
    @Published private(set) var agentResultLoading = false
    @Published private(set) var previewEditable = false
    @Published private(set) var savingPreview = false
    @Published private(set) var fileActionBusy = false
    @Published var preview: NativePanelFileContent?
    let client: any NativeConversationQuerying
    let projectID: String
    let sessionID: String
    private var generation = UUID()
    private var previewGeneration = UUID()
    var previewIdentity: UUID { previewGeneration }
    private var closed = false
    init(client: any NativeConversationQuerying, projectID: String, sessionID: String) {
        self.client = client; self.projectID = projectID; self.sessionID = sessionID
    }
    private func call(_ action: String, _ args: [String: SettingsValue] = [:]) async throws -> SettingsValue {
        var args = args; args["session_id"] = .string(sessionID)
        return try await client.invoke("native_conversation_panel_" + action, args: args, projectID: projectID)
    }
    private func decode<T: Decodable>(_ value: SettingsValue, as: T.Type) throws -> T {
        try JSONDecoder().decode(T.self, from: JSONEncoder().encode(value))
    }
    func refresh(_ tab: String, directory: String? = nil, quiet: Bool = false) async {
        if tab != "files" { clearFileSearch() }
        selectedTab = tab
        let current = UUID(); generation = current; loading = !quiet
        if !quiet { error = nil }
        let requestedPath = directory ?? path
        defer { if generation == current { loading = false } }
        do {
            if tab == "provenance" || tab == "sidechat" {
                // Uses the displayed transcript; never dispatch a file read for this tab.
                return
            } else if tab == "notebook" {
                let rows = try decode(await call("notebook_stars"), as: [NativeNotebookStar].self)
                guard rows.allSatisfy({ $0.belongs(project: projectID, session: sessionID) }) else { throw ProjectBrowserError.invalidResponse }
                guard generation == current, !Task.isCancelled else { return }; notebookStars = rows; notebookLoaded = true
            } else if tab == "highlights" {
                let rows = try decode(await call("highlights"), as: [NativeHighlight].self)
                guard rows.allSatisfy({ $0.belongs(project: projectID, session: sessionID) }) else { throw ProjectBrowserError.invalidResponse }
                guard generation == current, !Task.isCancelled else { return }; highlights = rows
            } else if tab == "artifacts" {
                let rows = try decode(await call("artifacts"), as: [NativePanelArtifact].self)
                guard generation == current, !Task.isCancelled else { return }; artifacts = rows
            } else if tab == "agents" {
                let result = try decode(await call("agents"), as: [NativeAgentSnapshot].self)
                let enabled = try decode(await call("agent_delegation"), as: Bool.self)
                guard generation == current, !Task.isCancelled else { return }; agents = result
                if !agentDelegationBusy { agentDelegationEnabled = enabled }
            } else if tab == "hosts" {
                let result = try decode(await call("contexts"), as: NativePanelContexts.self)
                guard generation == current, !Task.isCancelled else { return }; contexts = result
            } else {
                let rows = try decode(await call("files", ["path": .string(requestedPath)]), as: [NativePanelFile].self)
                guard generation == current, !Task.isCancelled else { return }; files = rows; path = requestedPath
            }
        } catch { if generation == current, !Task.isCancelled { self.error = error.localizedDescription } }
    }
    func clearFileSearch() { searchGeneration = UUID(); searchQuery = ""; searchHits = []; searchLoading = false }
    func searchFiles(_ query: String) async {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        let current = UUID(); searchGeneration = current; searchQuery = query; searchHits = []
        searchLoading = !query.isEmpty; error = nil
        guard !query.isEmpty, selectedTab == "files" else { searchLoading = false; return }
        defer { if searchGeneration == current { searchLoading = false } }
        do {
            try await Task.sleep(nanoseconds: 150_000_000)
            guard searchGeneration == current, !Task.isCancelled else { return }
            let rows = try decode(await call("searchfiles", ["query": .string(query)]), as: [NativePanelSearchHit].self)
            guard searchGeneration == current, selectedTab == "files", !Task.isCancelled else { return }
            guard rows.count <= 200, rows.allSatisfy({ !$0.path.isEmpty && !$0.path.hasPrefix("/") && !$0.path.contains("://") && !$0.path.split(separator: "/").contains("..") }) else { throw ProjectBrowserError.invalidResponse }
            searchHits = rows
        } catch { if searchGeneration == current, !Task.isCancelled { self.error = error.localizedDescription } }
    }
    func exportSource(path: String? = nil, artifactID: String? = nil) async throws -> NativePanelExport {
        guard !closed else { throw ProjectBrowserError.unavailable("面板已关闭。") }
        var args: [String: SettingsValue] = [:]
        if let path { args["path"] = .string(path) }
        if let artifactID { args["artifact_id"] = .string(artifactID) }
        let current = agentEpoch
        let source = try decode(await call("export", args), as: NativePanelExport.self)
        guard current == agentEpoch, !Task.isCancelled else { throw ProjectBrowserError.unavailable("面板已关闭。") }
        guard (source.path as NSString).isAbsolutePath, !source.name.isEmpty else { throw ProjectBrowserError.invalidResponse }
        return source
    }
    func performFileAction(_ action: NativePanelFileAction, path target: String, newPath: String? = nil) async throws {
        guard !closed, !fileActionBusy else { throw ProjectBrowserError.unavailable("文件操作正在进行或面板已关闭。") }
        let epoch = agentEpoch; let directory = path
        fileActionBusy = true
        defer { if epoch == agentEpoch { fileActionBusy = false } }
        var args: [String: SettingsValue] = ["file_action": .string(action.rawValue), "path": .string(target)]
        if let newPath { args["new_path"] = .string(newPath) }
        guard case .bool(true) = try await call("file_action", args) else { throw ProjectBrowserError.invalidResponse }
        guard epoch == agentEpoch, !Task.isCancelled, selectedTab == "files", path == directory else { return }
        await refresh("files", directory: directory)
        if !searchQuery.isEmpty { await searchFiles(searchQuery) }
    }
    func notebookStar(_ cell: NativeNotebookCell) -> NativeNotebookStar? { notebookStars.first { $0.matches(cell) } }
    func toggleNotebookStar(_ cell: NativeNotebookCell) async {
        guard notebookLoaded, !notebookBusy.contains(cell.starKey) else { return }
        let epoch = agentEpoch; notebookBusy.insert(cell.starKey); error = nil
        defer { if epoch == agentEpoch { notebookBusy.remove(cell.starKey) } }
        do {
            if let saved = notebookStar(cell) {
                let removed = try decode(await call("notebook_unstar", ["library_item_id": .string(saved.id)]), as: Bool.self)
                guard epoch == agentEpoch, !Task.isCancelled else { return }
                guard removed else { throw ProjectBrowserError.unavailable("收藏已发生变化，请刷新后重试。") }
                notebookStars.removeAll { $0.id == saved.id }
            } else {
                let row = try decode(await call("notebook_star", ["language": .string(cell.language), "code": .string(cell.source)]), as: NativeNotebookStar.self)
                guard epoch == agentEpoch, !Task.isCancelled else { return }
                guard row.belongs(project: projectID, session: sessionID), row.matches(cell) else { throw ProjectBrowserError.invalidResponse }
                notebookStars.removeAll { $0.id == row.id }; notebookStars.append(row)
            }
            if selectedTab == "notebook" { generation = UUID(); loading = false }
        } catch { if epoch == agentEpoch { self.error = error.localizedDescription } }
    }
    func removeHighlight(_ id: String) async {
        guard highlights.contains(where: { $0.id == id }), !highlightRemoving.contains(id) else { return }
        let epoch = agentEpoch; highlightRemoving.insert(id); error = nil
        defer { if epoch == agentEpoch { highlightRemoving.remove(id) } }
        do {
            let removed = try decode(await call("highlight_remove", ["library_item_id": .string(id)]), as: Bool.self)
            guard epoch == agentEpoch, !Task.isCancelled else { return }
            guard removed else { throw ProjectBrowserError.unavailable("摘录已发生变化，请刷新后重试。") }
            if selectedTab == "highlights" { generation = UUID(); loading = false }
            highlights.removeAll { $0.id == id }
        } catch { if epoch == agentEpoch { self.error = error.localizedDescription } }
    }
    func setContext(_ id: String, enabled: Bool) async {
        guard !contextBusy, contexts?.read_only == false else { return }
        let current = generation
        contextBusy = true
        defer { contextBusy = false }
        do {
            _ = try await call("context_enabled", ["context_id": .string(id), "enabled": .bool(enabled)])
            if generation == current, !Task.isCancelled { await refresh("hosts") }
        } catch { if generation == current { self.error = error.localizedDescription } }
    }
    func probeContext(_ id: String) async {
        guard !contextBusy else { return }
        let current = generation
        contextBusy = true
        defer { contextBusy = false }
        do {
            _ = try await client.invoke("probe_execution_context", args: ["contextId": .string(id)], projectID: projectID)
            if generation == current, !Task.isCancelled { await refresh("hosts") }
        } catch { if generation == current { self.error = error.localizedDescription } }
    }
    func setAgentDelegation(_ enabled: Bool) async {
        guard !agentDelegationBusy, agentDelegationEnabled != nil else { return }
        let epoch = agentEpoch; agentDelegationBusy = true; error = nil
        defer { if epoch == agentEpoch { agentDelegationBusy = false } }
        do {
            let confirmed = try decode(await call("agent_delegation", ["enabled": .bool(enabled)]), as: Bool.self)
            guard epoch == agentEpoch, !Task.isCancelled else { return }
            agentDelegationEnabled = confirmed
            if selectedTab == "agents" { await refresh("agents", quiet: true) }
        } catch { if epoch == agentEpoch { self.error = error.localizedDescription } }
    }
    func performAgentAction(_ snapshot: NativeAgentSnapshot, action: NativeAgentAction, budgets: [String: NativeAgentBudgetOverride] = [:]) async {
        let id = snapshot.id
        guard snapshot.workflow.depth == 0 else { return }
        if action == .run { guard !agentLaunching.contains(id) else { return }; agentLaunching.insert(id) }
        else { guard !agentActions.contains(id) else { return }; agentActions.insert(id) }
        let epoch = agentEpoch; error = nil
        defer { if epoch == agentEpoch { if action == .run { agentLaunching.remove(id) } else { agentActions.remove(id) } } }
        do {
            var args: [String: SettingsValue] = ["workflow_id": .string(id), "action": .string(action.rawValue)]
            if action == .approve { args["expected_version"] = .integer(snapshot.workflow.version) }
            if !budgets.isEmpty { args["budget_overrides"] = try JSONDecoder().decode(SettingsValue.self, from: JSONEncoder().encode(budgets)) }
            _ = try await call("agent_action", args)
            guard epoch == agentEpoch, !Task.isCancelled else { return }
            if selectedTab == "agents" { await refresh("agents", quiet: true) }
        } catch { if epoch == agentEpoch { self.error = "操作结果未确认，未自动重试。" + error.localizedDescription } }
    }
    func readAgentResult(workflow: String, step: String) async {
        let current = UUID(); previewGeneration = current; agentResultLoading = true; error = nil
        defer { if previewGeneration == current { agentResultLoading = false } }
        do {
            let result = try decode(await call("agent_result", ["workflow_id": .string(workflow), "step_id": .string(step)]), as: NativeAgentResult.self)
            guard current == previewGeneration, !Task.isCancelled else { return }
            guard result.workflow_id == workflow, result.step_id == step else { throw ProjectBrowserError.invalidResponse }
            agentResult = result
        } catch { if current == previewGeneration { self.error = error.localizedDescription } }
    }
    func child(_ file: NativePanelFile) -> String { path == "." ? file.name : path + "/" + file.name }
    var parent: String {
        let parent = (path as NSString).deletingLastPathComponent
        return parent.isEmpty ? "." : parent
    }
    func readFile(_ path: String) async { await read("readfile", args: ["path": .string(path), "render_pdf": .bool(true), "render_office": .bool(true)]) }
    func readArtifact(_ id: String) async { await read("readartifact", args: ["artifact_id": .string(id), "render_pdf": .bool(true), "render_office": .bool(true)]) }
    private func read(_ action: String, args: [String: SettingsValue]) async {
        let current = UUID(); previewGeneration = current; error = nil
        do {
            let content = try decode(await call(action, args), as: NativePanelFileContent.self)
            guard previewGeneration == current, !Task.isCancelled else { return }
            preview = content; previewEditable = action == "readfile" && content.text != nil && !content.truncated
        } catch { if previewGeneration == current { self.error = error.localizedDescription } }
    }
    func savePreview(_ text: String, original: NativePanelFileContent) async throws {
        guard previewEditable, !savingPreview, !original.truncated, let previous = original.text,
              preview?.path == original.path, preview?.text == previous else { throw ProjectBrowserError.invalidResponse }
        let current = previewGeneration; savingPreview = true
        defer { if current == previewGeneration { savingPreview = false } }
        let confirmed = try await call("savefile", ["path": .string(original.path), "original_text": .string(previous), "text": .string(text)])
        guard confirmed == .bool(true) else { throw ProjectBrowserError.invalidResponse }
        guard current == previewGeneration, !Task.isCancelled else { return }
        var value = try JSONDecoder().decode(SettingsValue.self, from: JSONEncoder().encode(original))
        value["text"] = .string(text); value["total_bytes"] = .integer(Int64(text.utf8.count))
        preview = try decode(value, as: NativePanelFileContent.self)
    }
    func documentQuote(_ selection: NativeDocumentSelection, original: NativePanelFileContent, identity: UUID) -> NativeSideChatQuote? {
        guard !closed, !savingPreview, previewGeneration == identity, let preview, preview.path == original.path,
              preview.text == original.text, preview.base64 == original.base64 else { return nil }
        return selection.quote(from: original)
    }
    func selectedPreviewQuote(_ text: String, path: String) -> NativeSideChatQuote? {
        guard !closed, !savingPreview, let preview, preview.path == path, preview.text != nil,
              NativeFileTextDocument(preview).containsSelection(text) else { return nil }
        return NativeSideChatQuote(text: text, source: path)
    }
    func readPreviewImage(_ reference: String, original: NativePanelFileContent) async throws -> NativePanelFileContent {
        guard !closed, !Task.isCancelled, preview?.path == original.path, preview?.text == original.text,
              let path = NativeFileImagePath.resolve(reference, document: original.path, remote: false) else { throw ProjectBrowserError.invalidResponse }
        let current = previewGeneration
        let value = try await client.invoke("native_conversation_image", args: ["session_id": .string(sessionID), "path": .string(path)], projectID: projectID)
        guard !closed, !Task.isCancelled, previewGeneration == current, preview?.text == original.text else { throw CancellationError() }
        let image = try decode(value, as: NativePanelFileContent.self)
        guard image.path == path, image.mime == "image/png", image.text == nil, image.base64 != nil, !image.truncated else { throw ProjectBrowserError.invalidResponse }
        return image
    }
    func dismissPreview() { previewGeneration = UUID(); preview = nil; previewEditable = false; savingPreview = false; agentResult = nil; agentResultLoading = false }
    func close() {
        closed = true
        clearFileSearch(); agentEpoch = UUID(); notebookBusy = []; highlightRemoving = []; agentDelegationBusy = false; agentLaunching = []; agentActions = []; generation = UUID(); dismissPreview() }
}
