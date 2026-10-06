import Foundation
import WispProjectBrowser

struct NativeFileBrowserRow: Identifiable {
    var id: String { path }
    let path: String
    let name: String
    let directory: Bool
    let size: UInt64
    let modified: UInt64?
}

struct NativeFilesTransferTarget: Equatable {
    let generation: UUID
    let context: String
    let directory: String
}

/// A Files tab has its own location/navigation lifetime, separate from agents
/// and artifact previews. Every host call still resolves this exact frame.
@MainActor
final class NativeFilesModel: ObservableObject {
    let client: any NativeConversationQuerying
    let projectID: String
    let sessionID: String
    let legacy: NativePanelModel
    @Published private(set) var catalog: NativeFileLocations?
    @Published private(set) var contextID = "local"
    @Published private(set) var path = "."
    @Published private(set) var entries: [NativePanelFile] = []
    @Published private(set) var searchHits: [NativePanelSearchHit] = []
    @Published private(set) var loading = false
    @Published private(set) var searchLoading = false
    @Published private(set) var catalogLoading = false
    @Published private(set) var error: String?
    @Published private(set) var selection: Set<String> = []
    @Published private(set) var selecting = false
    @Published var sort = NativeFileSort.name
    @Published var sorting = false
    @Published var query = "" {
        didSet { if query != oldValue { invalidate(); selection = []; searchHits = []; searchLoading = false } }
    }
    @Published private(set) var preview: NativeFilePreview?
    @Published private(set) var previewLoading = false
    @Published private(set) var saving = false
    @Published private(set) var saveUnconfirmed = false
    @Published private(set) var fileActionBusy = false
    @Published private(set) var actionUnconfirmed = false
    @Published private(set) var copyBusy = false
    @Published var readOnly: Bool
    @Published var transfersSupported: Bool
    @Published private(set) var transfer: NativeFileTransfer?
    @Published private(set) var transferRuns: [String: NativeRun] = [:]
    @Published private(set) var transferSubmitting = false
    @Published private(set) var transferUnconfirmed = false
    @Published private(set) var transferError: String?
    @Published private(set) var transferContext: String?
    private var transferGeneration = UUID()
    private(set) var closed = false
    private var generation = UUID()
    private var catalogGeneration = UUID()
    private var directoryGeneration = UUID()
    private var searchGeneration = UUID()
    private var previewGeneration = UUID()
    private var copyGeneration = UUID()
    init(client: any NativeConversationQuerying, projectID: String, sessionID: String, readOnly: Bool, transfersSupported: Bool = false) {
        self.client = client; self.projectID = projectID; self.sessionID = sessionID; self.readOnly = readOnly
        self.transfersSupported = transfersSupported
        legacy = NativePanelModel(client: client, projectID: projectID, sessionID: sessionID)
    }
    var local: Bool { contextID == "local" }
    var canWrite: Bool { !closed && local && !readOnly && catalog?.read_only == false && !fileActionBusy && !actionUnconfirmed && !transferSubmitting && !transferUnconfirmed }
    var canUpload: Bool { transfersSupported && !closed && !readOnly && catalog?.read_only == false && !loading && !catalogLoading && query.isEmpty && !fileActionBusy && !actionUnconfirmed && !saving && !transferSubmitting && !transferUnconfirmed }
    var canDownload: Bool { transfersSupported && !closed && !local && catalog != nil && !loading && !transferSubmitting && !transferUnconfirmed }
    var transferPollKey: String { transfer?.items.compactMap(\.run_id).joined(separator: ":") ?? "" }
    var hasActiveTransfers: Bool { transfer?.items.contains { item in item.run_id != nil && ["submitted", "running", "cancelling"].contains(transferRuns[item.id]?.status ?? item.status) } == true }
    func transferTarget(upload: Bool) -> NativeFilesTransferTarget? {
        guard upload ? canUpload : canDownload else { return nil }
        return .init(generation: generation, context: contextID, directory: path)
    }
    func owns(_ target: NativeFilesTransferTarget) -> Bool {
        !closed && generation == target.generation && contextID == target.context && path == target.directory
    }
    var rows: [NativeFileBrowserRow] {
        if !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && local {
            return searchHits.map { .init(path: $0.path, name: $0.name, directory: $0.is_dir, size: $0.size, modified: nil) }
        }
        return sort.sorted(entries).map { .init(path: child($0.name), name: $0.name, directory: $0.is_dir, size: $0.size, modified: $0.modified_unix_millis) }
    }
    var parent: String {
        if path == "." || path == "/" || path == "~" { return path }
        let parent = (path as NSString).deletingLastPathComponent
        return parent.isEmpty ? local ? "." : "/" : parent
    }
    func child(_ name: String) -> String { path == "." ? name : path == "/" ? path + name : path + "/" + name }
    private func call(_ action: String, _ values: [String: SettingsValue] = [:], context: String? = nil) async throws -> SettingsValue {
        var args = values; args["session_id"] = .string(sessionID)
        if let context { args["context_id"] = .string(context) }
        return try await client.invoke("native_conversation_panel_" + action, args: args, projectID: projectID)
    }
    private func invalidate() { generation = UUID(); directoryGeneration = UUID(); previewGeneration = UUID(); copyGeneration = UUID(); loading = false; previewLoading = false; copyBusy = false }
    func open() async {
        guard !closed, !Task.isCancelled, catalog == nil, !catalogLoading else { return }
        await refreshLocations()
    }
    func refreshLocations() async {
        guard !closed, !Task.isCancelled, !fileActionBusy, !saving, !transferSubmitting else { return }
        let current = UUID(); catalogGeneration = current; catalogLoading = true; error = nil
        defer { if catalogGeneration == current { catalogLoading = false } }
        do {
            let page = try NativeFileLocations.decode(await call("file_locations"), project: projectID, session: sessionID)
            guard !closed, catalogGeneration == current, !Task.isCancelled else { return }
            if catalog?.local_root != page.local_root { invalidate(); selection = []; dismissPreview() }
            catalog = page
            if !page.locations.contains(where: { $0.id == contextID }) { contextID = "local"; path = "."; query = "" }
            await navigate(path)
        } catch { if !closed, catalogGeneration == current, !Task.isCancelled { self.error = error.localizedDescription } }
    }
    func chooseLocation(_ context: String) async {
        guard !closed, !Task.isCancelled, !fileActionBusy, !saving, !transferSubmitting, catalog?.locations.contains(where: { $0.id == context }) == true else { return }
        contextID = context; query = ""; selecting = false; sorting = false
        await navigate(context == "local" ? "." : "~")
    }
    func navigate(_ requested: String) async {
        guard !closed, !Task.isCancelled, !fileActionBusy, !saving, !transferSubmitting, catalog != nil,
              (local ? NativeFilesContract.relative(requested, rootAllowed: true) : NativeFilesContract.remote(requested)) else { return }
        query = ""; selection = []; entries = []; searchHits = []; searchGeneration = UUID(); searchLoading = false
        dismissPreview(); invalidate(); let current = generation; let context = contextID
        path = requested; loading = true; error = nil; sorting = false
        defer { if generation == current { loading = false } }
        do {
            let page = try NativeFileDirectory.decode(await call("file_directory", ["path": .string(requested)], context: context), project: projectID, session: sessionID, context: context)
            guard !closed, generation == current, contextID == context, !Task.isCancelled else { return }
            entries = page.entries; path = page.path
        } catch { if !closed, generation == current, !Task.isCancelled { self.error = error.localizedDescription } }
    }
    func refresh() async {
        guard !closed, !Task.isCancelled, !fileActionBusy, !saving, !transferSubmitting else { return }
        if catalog == nil { await refreshLocations(); return }
        let current = UUID(); directoryGeneration = current; let scope = generation; let context = contextID; let requested = path
        loading = true; error = nil
        defer { if directoryGeneration == current { loading = false } }
        do {
            let page = try NativeFileDirectory.decode(await call("file_directory", ["path": .string(requested)], context: context), project: projectID, session: sessionID, context: context)
            guard !closed, directoryGeneration == current, generation == scope, !Task.isCancelled else { return }
            entries = page.entries; path = page.path
            if !query.isEmpty { await search() }
            guard !closed, directoryGeneration == current, generation == scope, !Task.isCancelled else { return }
            selection.formIntersection(rows.map(\.id))
            actionUnconfirmed = false
            transferUnconfirmed = false
        } catch { if !closed, directoryGeneration == current, generation == scope, !Task.isCancelled { self.error = error.localizedDescription } }
    }
    func search() async {
        guard !closed, !Task.isCancelled else { return }
        let requested = query.trimmingCharacters(in: .whitespacesAndNewlines)
        let current = UUID(); searchGeneration = current; searchHits = []
        guard !closed, local, !requested.isEmpty else { searchLoading = false; return }
        searchLoading = true; error = nil
        defer { if searchGeneration == current { searchLoading = false } }
        do {
            try await Task.sleep(nanoseconds: 150_000_000)
            guard !closed, local, searchGeneration == current, !Task.isCancelled else { return }
            let hits = try JSONDecoder().decode([NativePanelSearchHit].self, from: JSONEncoder().encode(try await call("searchfiles", ["query": .string(requested)])))
            guard !closed, local, searchGeneration == current, query.trimmingCharacters(in: .whitespacesAndNewlines) == requested, !Task.isCancelled else { return }
            guard hits.count <= 200, Set(hits.map(\.path)).count == hits.count,
                  hits.allSatisfy({ NativeFilesContract.relative($0.path) && ($0.path as NSString).lastPathComponent == $0.name }) else { throw ProjectBrowserError.invalidResponse }
            searchHits = hits
        } catch { if !closed, searchGeneration == current, !Task.isCancelled { self.error = error.localizedDescription } }
    }
    func toggleSelectionMode() {
        guard !closed, local, !loading else { return }
        selecting.toggle(); selection = []; copyGeneration = UUID(); copyBusy = false
    }
    func toggle(_ row: NativeFileBrowserRow) {
        guard !closed, local, selecting, !loading, rows.contains(where: { $0.id == row.id }) else { return }
        if !selection.insert(row.id).inserted { selection.remove(row.id) }
        copyGeneration = UUID(); copyBusy = false
    }
    func selectAll() {
        guard !closed, local, selecting, !loading else { return }
        let visible = Set(rows.map(\.id)); selection = selection == visible ? [] : visible
        copyGeneration = UUID(); copyBusy = false
    }
    func pathsForCopy(clicked: String?) -> [String] {
        let visible = Set(rows.map(\.id))
        if let clicked {
            guard visible.contains(clicked) else { return [] }
            if !selection.contains(clicked) { return [clicked] }
        }
        return selection.intersection(visible).sorted()
    }
    /// Clipboard writes belong to the view and occur only after this returns a
    /// fresh, exact-scope reply. Navigation/selection changes discard late copies.
    func copyPaths(absolute: Bool, clicked: String? = nil) async -> String? {
        guard !closed, !Task.isCancelled, local, !loading, !copyBusy, let root = catalog?.local_root else { return nil }
        let paths = pathsForCopy(clicked: clicked); guard !paths.isEmpty else { return nil }
        let current = UUID(); copyGeneration = current; let scope = generation; copyBusy = true; error = nil
        defer { if copyGeneration == current { copyBusy = false } }
        do {
            let page = try NativeFilePaths.decode(await call("file_paths", ["paths": .array(paths.map(SettingsValue.string))], context: "local"), project: projectID, session: sessionID, requested: paths, root: root)
            guard !closed, copyGeneration == current, generation == scope, local, !Task.isCancelled else { return nil }
            return page.paths.map { absolute ? $0.absolute_path : $0.relative_path }.joined(separator: "\n")
        } catch { if !closed, copyGeneration == current, !Task.isCancelled { self.error = error.localizedDescription }; return nil }
    }
    func read(_ row: NativeFileBrowserRow) async {
        guard !closed, !Task.isCancelled, !loading, !row.directory, rows.contains(where: { $0.id == row.id }), let root = catalog?.local_root else { return }
        let context = contextID; let scope = generation; let current = UUID(); previewGeneration = current
        previewLoading = true; preview = nil; error = nil; saveUnconfirmed = false
        defer { if previewGeneration == current { previewLoading = false } }
        do {
            let page = try NativeFilePreview.decode(await call("file_read", ["path": .string(row.path)], context: context), project: projectID, session: sessionID, context: context, requested: row.path, root: root)
            guard !closed, previewGeneration == current, generation == scope, contextID == context, !Task.isCancelled else { return }
            preview = page
        } catch { if !closed, previewGeneration == current, !Task.isCancelled { self.error = error.localizedDescription } }
    }
    func save(_ text: String, original: NativePanelFileContent) async throws {
        guard canWrite, !Task.isCancelled, !saving, !saveUnconfirmed, !original.truncated, let previous = original.text,
              preview?.content.path == original.path, preview?.content.text == previous else { throw ProjectBrowserError.invalidResponse }
        let current = previewGeneration; saving = true
        defer { if previewGeneration == current { saving = false } }
        do {
            guard try await call("savefile", ["path": .string(original.path), "original_text": .string(previous), "text": .string(text)]) == .bool(true) else { throw ProjectBrowserError.invalidResponse }
            guard !closed, previewGeneration == current, !Task.isCancelled, let existing = preview else { throw ProjectBrowserError.unavailable("面板已关闭。") }
            var value = try JSONDecoder().decode(SettingsValue.self, from: JSONEncoder().encode(existing))
            value["content"]["text"] = .string(text); value["content"]["total_bytes"] = .integer(Int64(text.utf8.count))
            preview = try JSONDecoder().decode(NativeFilePreview.self, from: JSONEncoder().encode(value))
        } catch { if !closed, previewGeneration == current { saveUnconfirmed = true }; throw error }
    }
    func performAction(_ action: NativePanelFileAction, path: String, newPath: String?) async throws {
        guard canWrite, !Task.isCancelled else { throw ProjectBrowserError.unavailable("此会话不能修改文件。") }
        fileActionBusy = true; let current = generation
        // No second write can start while this one is busy/uncertain. A scope
        // change must not leave the old operation's busy flag stuck forever.
        defer { fileActionBusy = false }
        var args: [String: SettingsValue] = ["file_action": .string(action.rawValue), "path": .string(path)]
        if let newPath { args["new_path"] = .string(newPath) }
        actionUnconfirmed = true
        guard try await call("file_action", args) == .bool(true) else { throw ProjectBrowserError.invalidResponse }
        guard !closed, generation == current, !Task.isCancelled else { throw ProjectBrowserError.unavailable("面板已关闭。") }
        actionUnconfirmed = false
    }
    func quote(_ text: String, source: String) -> NativeSideChatQuote? {
        guard !closed, !saving, let content = preview?.content, content.path == source,
              !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, content.text?.contains(text) == true else { return nil }
        return NativeSideChatQuote(text: text, source: source)
    }
    func exportSource(_ row: NativeFileBrowserRow) async throws -> NativePanelExport {
        guard !closed, local, !row.directory, rows.contains(where: { $0.id == row.id }), let root = catalog?.local_root else { throw ProjectBrowserError.invalidResponse }
        let current = generation
        let source = try await legacy.exportSource(path: row.path)
        guard !closed, local, generation == current, !Task.isCancelled, NativeFilesContract.inside(source.path, root: root) else { throw ProjectBrowserError.invalidResponse }
        return source
    }
    func upload(_ sources: [String], target: NativeFilesTransferTarget) async {
        guard !Task.isCancelled, canUpload, owns(target), !sources.isEmpty, Set(sources).count == sources.count,
              sources.allSatisfy({ $0.hasPrefix("/") && !$0.contains("\0") }) else { return }
        await submitTransfer(target, sources: sources, source: target.directory, destination: nil)
    }
    func download(_ row: NativeFileBrowserRow, destination: String, target: NativeFilesTransferTarget) async {
        guard !Task.isCancelled, canDownload, owns(target), !row.directory, rows.contains(where: { $0.id == row.id }),
              destination.hasPrefix("/"), !destination.contains("\0") else { return }
        await submitTransfer(target, sources: [], source: row.path, destination: destination)
    }
    private func submitTransfer(_ target: NativeFilesTransferTarget, sources: [String], source: String, destination: String?) async {
        let current = UUID(); transferGeneration = current
        transferSubmitting = true; transferUnconfirmed = true; transferError = nil; transferContext = target.context; transfer = nil; transferRuns = [:]
        defer { transferSubmitting = false }
        do {
            var args: [String: SettingsValue] = ["path": .string(source)]
            if let destination { args["destination_path"] = .string(destination) }
            else { args["source_paths"] = .array(sources.map(SettingsValue.string)) }
            let page = try NativeFileTransfer.decode(await call(destination == nil ? "file_upload" : "file_download", args, context: target.context), project: projectID, session: sessionID, context: target.context, path: source, sources: sources, destination: destination)
            // Search changes invalidate browser reads, not an already submitted transfer.
            guard !closed, current == transferGeneration, !Task.isCancelled else { return }
            transfer = page; transferUnconfirmed = false; transferSubmitting = false
            if target.context == "local" { await refresh() }
        } catch {
            if !closed, current == transferGeneration { transferError = error.localizedDescription }
        }
    }
    func refreshTransferRuns() async {
        guard !closed, !Task.isCancelled, !transferSubmitting, let page = transfer, !transferPollKey.isEmpty else { return }
        let current = transferGeneration
        do {
            let snapshot = try JSONDecoder().decode(NativeContextActivity.self, from: JSONEncoder().encode(try await call("activity")))
            guard !closed, current == transferGeneration, !Task.isCancelled else { return }
            let ids = Set(page.items.compactMap(\.run_id))
            let runs = snapshot.runs.filter { ids.contains($0.id) }
            guard Set(runs.map(\.id)) == ids, Set(runs.map(\.id)).count == runs.count,
                  runs.allSatisfy({ $0.context_id == page.context_id && $0.frame_id == sessionID && $0.kind == "file_transfer" }) else { throw ProjectBrowserError.invalidResponse }
            let wasActive = hasActiveTransfers
            for run in runs { transferRuns[run.id] = run }
            transferError = nil
            if wasActive && !hasActiveTransfers && page.context_id == contextID && page.path == path { await refresh() }
        } catch { if !closed, current == transferGeneration, !Task.isCancelled { transferError = error.localizedDescription } }
    }
    func dismissPreview() { previewGeneration = UUID(); preview = nil; previewLoading = false; saving = false; saveUnconfirmed = false }
    func close() {
        closed = true; invalidate(); catalogGeneration = UUID(); searchGeneration = UUID(); selection = []; sorting = false
        loading = false; catalogLoading = false; searchLoading = false; fileActionBusy = false; transferGeneration = UUID(); transferSubmitting = false; dismissPreview(); legacy.close()
    }
}
