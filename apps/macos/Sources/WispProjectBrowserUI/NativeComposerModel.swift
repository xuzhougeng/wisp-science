import Foundation
import WispProjectBrowser

struct NativeComposerReference: Codable, Equatable, Identifiable {
    let reference: SettingsValue
    let label: String
    let detail: String
    var id: String {
        let kind = reference["kind"].string
        return kind + ":" + (kind == "runtime" ? reference["context_id"].string + ":" + reference["language"].string : kind == "skill" ? reference["name"].string : reference["id"].string)
    }
    var valid: Bool {
        switch reference["kind"].string {
        case "artifact", "session", "project", "workflow", "context": return !reference["id"].string.isEmpty
        case "skill": return !reference["name"].string.isEmpty
        case "runtime": return !reference["context_id"].string.isEmpty && ["python", "r"].contains(reference["language"].string)
        default: return false
        }
    }
    static func message(_ draft: String, references: [Self]) -> String {
        var blocks = [draft.trimmingCharacters(in: .whitespacesAndNewlines)].filter { !$0.isEmpty }
        let labels = ["artifact": "Attached artifacts", "session": "Attached sessions", "project": "Project context", "skill": "Selected skills", "workflow": "Selected workflows", "context": "Target environments", "runtime": "Target runtimes"]
        var kinds: [String] = []
        for option in references where !kinds.contains(option.reference["kind"].string) { kinds.append(option.reference["kind"].string) }
        for kind in kinds {
            let names = references.filter { $0.reference["kind"].string == kind }.map { $0.label.replacingOccurrences(of: "\r", with: " ").replacingOccurrences(of: "\n", with: " ") }
            blocks.append((labels[kind] ?? kind) + ": " + names.joined(separator: ", "))
        }
        return blocks.joined(separator: "\n\n")
    }
}

/// Composer configuration shares the host's session scope and exact-model catalog.
/// Binding changes invalidate reads and acknowledgements from the previous session.
@MainActor final class NativeComposerModel: ObservableObject {
    @Published private(set) var contexts: NativePanelContexts?
    @Published private(set) var contextID = "local"
    @Published private(set) var runtimes: [NativeRuntimeInfo] = []
    @Published private(set) var efforts: [String] = []
    @Published private(set) var effort = ""
    @Published private(set) var busy = false
    @Published private(set) var error: String?
    @Published private(set) var options: [NativeComposerReference] = []
    @Published private(set) var searching = false
    @Published private(set) var searchError: String?
    let client: any NativeConversationQuerying
    private var project: String?
    private var session: String?
    private var profile: SettingsValue = .null
    private var epoch = UUID()
    private var searchEpoch = UUID()
    private var contextsEpoch = UUID()
    init(client: any NativeConversationQuerying) { self.client = client }
    var contextLabel: String { contexts?.contexts.first { $0.id == contextID }?.label ?? contextID }
    var runtimeLabel: String {
        let rows = runtimes.filter { $0.key.contextId == contextID }
        return rows.isEmpty ? "运行时 · 未启动" : "运行时 · " + rows.map { $0.key.language.uppercased() + " " + $0.status }.joined(separator: " / ")
    }
    private func decode<T: Decodable>(_ value: SettingsValue, _ type: T.Type) throws -> T {
        try JSONDecoder().decode(type, from: JSONEncoder().encode(value))
    }
    func reset() {
        epoch = UUID(); contextsEpoch = UUID(); dismissSearch(); project = nil; session = nil; profile = .null
        contexts = nil; contextID = "local"; runtimes = []; efforts = []; effort = ""; busy = false; error = nil
    }
    func bind(project: String, session: String, profile: SettingsValue) async {
        reset(); self.project = project; self.session = session; self.profile = profile
        let current = epoch
        await refreshContexts()
        guard epoch == current, !profile["id"].string.isEmpty else { return }
        effort = profile["reasoning_effort"].string
        do {
            let catalog = try await client.invoke("model_catalog_lookup", args: ["provider": profile["provider"], "apiUrl": profile["api_url"], "model": profile["model"]], projectID: project)
            guard epoch == current else { return }
            let known = ["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"]
            efforts = catalog["efforts"].array.map(\.string).filter { known.contains($0) }
        } catch { if epoch == current { self.error = "无法读取思考强度：" + error.localizedDescription } }
    }
    func reload() async {
        guard !busy, let project, let session else { return }
        let current = epoch; let id = profile["id"].string
        do {
            let rows = try await client.invoke("list_models", args: [:], projectID: project).array
            guard epoch == current, !Task.isCancelled else { return }
            await bind(project: project, session: session, profile: rows.first { $0["id"].string == id } ?? .null)
        } catch { if epoch == current { self.error = "无法重新读取输入配置：" + error.localizedDescription } }
    }
    func refreshContexts() async {
        guard let project, let session else { return }
        let current = epoch; let read = UUID(); contextsEpoch = read
        do {
            let result = try decode(await client.invoke("native_conversation_panel_contexts", args: ["session_id": .string(session)], projectID: project), NativePanelContexts.self)
            let inherited = try await client.invoke("get_default_execution_context", args: [:], projectID: project).string
            guard epoch == current, contextsEpoch == read, !Task.isCancelled else { return }
            contexts = result
            // A detached inherited host is not an available execution target.
            let selected = result.default_context?.context_id ?? (inherited.isEmpty ? "local" : inherited)
            contextID = result.attached.contains { $0.id == selected } ? selected : "local"
            let activity = try decode(await client.invoke("native_conversation_panel_activity", args: ["session_id": .string(session), "context_id": .string(contextID)], projectID: project), NativeContextActivity.self)
            guard epoch == current, contextsEpoch == read, !Task.isCancelled else { return }; runtimes = activity.runtimes
        } catch { if epoch == current, contextsEpoch == read, !Task.isCancelled { self.error = "无法读取运行环境：" + error.localizedDescription } }
    }
    func selectContext(_ id: String) async {
        guard !busy, contexts?.read_only == false, contexts?.attached.contains(where: { $0.id == id }) == true,
              let project, let session else { return }
        let current = epoch; busy = true; error = nil
        defer { if epoch == current { busy = false } }
        do {
            let reply = try await client.invoke("native_conversation_panel_context_default", args: ["session_id": .string(session), "context_id": .string(id)], projectID: project)
            guard epoch == current else { return }
            guard reply["context_id"].string == id else { throw ProjectBrowserError.invalidResponse }
            await refreshContexts()
        } catch { if epoch == current { self.error = "运行环境未确认保存，请刷新后检查：" + error.localizedDescription } }
    }
    private static func catalogKey(_ row: SettingsValue) -> [SettingsValue] { [row["provider"], row["api_url"], row["model"]] }
    func selectEffort(_ value: String) async {
        guard !busy, !efforts.isEmpty, value.isEmpty || efforts.contains(value), let project else { return }
        let current = epoch; let target = profile["id"].string; let key = Self.catalogKey(profile)
        busy = true; error = nil
        defer { if epoch == current { busy = false } }
        do {
            let rows = try await client.invoke("list_models", args: [:], projectID: project).array
            guard epoch == current else { return }
            guard var draft = rows.first(where: { $0["id"].string == target }), Self.catalogKey(draft) == key else {
                throw ProjectBrowserError.unavailable("模型配置已变化，请重新读取会话。")
            }
            var fields = draft.object; fields.removeValue(forKey: "key"); fields["reasoning_effort"] = .string(value); draft = .object(fields)
            var args: [String: SettingsValue] = ["profile": draft, "key": .null]
            for field in ["use_for_vision", "use_for_image_generation", "use_for_video_generation"] { args[camelCase(field)] = .bool(draft[field].bool) }
            let saved = try await client.invoke("save_model", args: args, projectID: project).array
            guard epoch == current else { return }
            guard let confirmed = saved.first(where: { $0["id"].string == target }), confirmed["reasoning_effort"].string == value else { throw ProjectBrowserError.invalidResponse }
            profile = confirmed; effort = value
        } catch { if epoch == current { self.error = "思考强度未确认保存，请刷新后检查：" + error.localizedDescription } }
    }
    func dismissSearch() { searchEpoch = UUID(); options = []; searching = false; searchError = nil }
    func search(kind: String, query: String) async {
        guard let project, let session, ["artifact", "session", "skill"].contains(kind) else { return }
        let current = UUID(); searchEpoch = current; searching = true; options = []; searchError = nil
        defer { if searchEpoch == current { searching = false } }
        do {
            struct Catalog: Decodable { let session_id: String; let options: [NativeComposerReference] }
            let result = try decode(await client.invoke("native_conversation_references", args: ["session_id": .string(session), "kind": .string(kind), "query": .string(query)], projectID: project), Catalog.self)
            guard searchEpoch == current, !Task.isCancelled else { return }
            guard result.session_id == session, result.options.count <= 120, result.options.allSatisfy(\.valid) else { throw ProjectBrowserError.invalidResponse }
            var seen: Set<String> = []; options = result.options.filter { seen.insert($0.id).inserted }
        } catch { if searchEpoch == current, !Task.isCancelled { searchError = error.localizedDescription } }
    }
}
