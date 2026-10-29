import Foundation
import WispProjectBrowser

@MainActor
final class NativeContextActivityModel: ObservableObject {
    @Published private(set) var snapshot: NativeContextActivity?
    @Published private(set) var detail: NativeRun?
    @Published private(set) var objects: NativeRuntimeObjects?
    @Published private(set) var loading = false
    @Published private(set) var busy = false
    @Published private(set) var runtimeOperations: Set<String> = []
    @Published private(set) var startingLanguage: String?
    @Published private(set) var executing = false
    @Published private(set) var execution: NativeRuntimeExecution?
    @Published private(set) var executionError: String?
    @Published private(set) var error: String?
    private let client: any NativeConversationQuerying
    let projectID: String
    let sessionID: String
    let contextID: String
    private var refreshing = false
    private var generation = UUID()
    private var detailGeneration = UUID()
    private(set) var selectedRun: String?
    private(set) var selectedRuntime: String?
    init(client: any NativeConversationQuerying, projectID: String, sessionID: String, contextID: String) {
        self.client = client; self.projectID = projectID; self.sessionID = sessionID; self.contextID = contextID
    }
    private func call<T: Decodable>(_ action: String, args: [String: SettingsValue] = [:], as: T.Type) async throws -> T {
        var args = args; args["session_id"] = .string(sessionID)
        let value = try await client.invoke("native_conversation_panel_" + action, args: args, projectID: projectID)
        return try JSONDecoder().decode(T.self, from: JSONEncoder().encode(value))
    }
    var runtimes: [NativeRuntimeInfo] { snapshot?.runtimes.filter { $0.key.contextId == contextID } ?? [] }
    /// An empty `contextID` scopes runs to this conversation across every
    /// environment instead (the right panel's 运行 tab).
    var runs: [NativeRun] { snapshot?.runs.filter(owns) ?? [] }
    private func owns(_ run: NativeRun) -> Bool { contextID.isEmpty ? run.frame_id == sessionID : run.context_id == contextID }
    func refresh() async {
        guard !refreshing else { return }
        let current = generation; refreshing = true; loading = snapshot == nil
        defer { if current == generation { loading = false; refreshing = false } }
        do {
            let result = try await call("activity", as: NativeContextActivity.self)
            guard current == generation, !Task.isCancelled else { return }
            snapshot = result
            // Poll only the visible run, never bulk-download every output tail.
            if let id = selectedRun, !busy { await readRun(id, clearError: false) }
        } catch { if current == generation { self.error = error.localizedDescription } }
    }
    func readRun(_ id: String, clearError: Bool = true) async {
        let current = UUID(); detailGeneration = current
        if selectedRun != id { detail = nil }
        selectedRun = id; selectedRuntime = nil; objects = nil
        if clearError { error = nil }
        do {
            let result = try await call("run_detail", args: ["run_id": .string(id)], as: NativeRun.self)
            guard current == detailGeneration, !Task.isCancelled else { return }
            guard result.id == id, owns(result) else { throw ProjectBrowserError.invalidResponse }
            detail = result
        } catch { if current == detailGeneration { self.error = error.localizedDescription } }
    }
    func inspect(_ id: String) async {
        let current = UUID(); detailGeneration = current
        selectedRuntime = id; selectedRun = nil; detail = nil; objects = nil; error = nil
        do {
            let result = try await call("runtime_inspect", args: ["runtime_id": .string(id)], as: NativeRuntimeObjects.self)
            guard current == detailGeneration, !Task.isCancelled else { return }; objects = result
        } catch { if current == detailGeneration { self.error = error.localizedDescription } }
    }
    func mutateRun(_ id: String, harvest: Bool) async {
        guard !busy, snapshot?.read_only == false else { return }
        let current = generation; busy = true; error = nil
        defer { if current == generation { busy = false } }
        do {
            _ = try await call(harvest ? "run_harvest" : "run_cancel", args: ["run_id": .string(id)], as: NativeRun.self)
            guard current == generation, !Task.isCancelled else { return }
            await refresh()
            if selectedRun == id { await readRun(id) }
        } catch {
            // A lost reply can follow successful cancellation/harvest. Never replay.
            if current == generation { self.error = error.localizedDescription }
        }
    }
    func startRuntime(language: String) async {
        guard startingLanguage == nil, snapshot?.read_only == false else { return }
        let current = generation; startingLanguage = language; error = nil
        defer { if current == generation { startingLanguage = nil } }
        do {
            _ = try await call("runtime_start", args: ["context_id": .string(contextID), "language": .string(language)], as: NativeRuntimeInfo.self)
            if current == generation { await refresh() }
        } catch { if current == generation { self.error = error.localizedDescription } }
    }
    func controlRuntime(_ runtime: NativeRuntimeInfo, action: NativeRuntimeAction) async {
        guard !runtimeOperations.contains(runtime.id), action != .restart || snapshot?.read_only == false else { return }
        let current = generation; runtimeOperations.insert(runtime.id); error = nil
        defer { if current == generation { runtimeOperations.remove(runtime.id) } }
        do {
            _ = try await call("runtime_" + action.rawValue, args: ["runtime_id": .string(runtime.id), "runtime_generation": .integer(Int64(clamping: runtime.generation))], as: SettingsValue.self)
            guard current == generation else { return }
            if selectedRuntime == runtime.id { dismissDetail() }
            await refresh()
        } catch { if current == generation { self.error = error.localizedDescription } }
    }
    func execute(code: String, language: String) async {
        guard !executing, !code.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, snapshot?.read_only == false else { return }
        let current = generation; executing = true; executionError = nil; execution = nil
        defer { if current == generation { executing = false } }
        do {
            let result = try await call("runtime_execute", args: ["context_id": .string(contextID), "language": .string(language), "code": .string(code)], as: NativeRuntimeExecution.self)
            guard current == generation else { return }; execution = result
            await refresh()
        } catch { if current == generation { executionError = "执行结果未确认，未自动重试。" + error.localizedDescription } }
    }
    func dismissDetail() { detailGeneration = UUID(); selectedRun = nil; selectedRuntime = nil; detail = nil; objects = nil }
    func close() { generation = UUID(); runtimeOperations = []; startingLanguage = nil; executing = false; refreshing = false; loading = false; busy = false; dismissDetail() }
}
