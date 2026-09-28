import Foundation
import SwiftUI
import WispProjectBrowser

enum NativeSettingsSection: String, CaseIterable, Identifiable {
    case general, session, appearance, pet, models
    case quickActions = "quick-actions"
    case workflows, specialists, memory, skills, plugins, browser, connections, channels, credentials, permissions, environments, storage, usage
    var id: String { rawValue }
    var title: String { localized(chineseTitle) }
    private var navigation: [String: String] { WispDesign.settingsNavigation[rawValue] ?? [:] }
    private var chineseTitle: String { navigation["zh"] ?? rawValue }
    var group: String { navigation["group"] ?? "" }
    func matches(_ query: String) -> Bool {
        let haystack = ([rawValue] + Array(navigation.values)).joined(separator: " ").lowercased()
        return query.lowercased().split(whereSeparator: { $0.isWhitespace }).allSatisfy { haystack.contains($0) }
    }
    var reads: [String] {
        switch self {
        case .general: return ["get_settings", "get_appearance_prefs", "get_network_settings", "get_bootstrap_status", "get_update_check_enabled"]
        case .session: return ["get_settings", "get_auto_review_enabled"]
        case .appearance: return ["get_appearance_prefs"]
        case .pet: return ["get_settings", "get_pet_runtime_status", "get_pet"]
        case .models: return ["list_models", "list_acp_agents"]
        case .quickActions: return ["list_quick_actions", "list_workflow_templates"]
        case .workflows: return ["list_workflow_templates", "list_models", "list_skills"]
        case .specialists: return ["list_specialists", "list_models", "list_skills"]
        case .memory: return ["get_memory_view", "get_auto_failure_analysis_settings"]
        case .skills: return ["list_skills"]
        case .plugins: return ["list_plugins"]
        case .browser: return ["get_browser_auto_launch", "get_browser_auto_close_tabs", "get_browser_url_filters", "browser_extension_status"]
        case .connections: return ["list_connectors", "list_mcp_connections"]
        case .channels: return ["channels_status", "get_settings"]
        case .credentials: return ["credential_status", "list_custom_credentials"]
        case .permissions: return ["list_approval_grants", "list_connectors"]
        case .environments: return ["list_execution_contexts", "list_ssh_hosts", "get_default_execution_context", "list_ssh_trust_edges"]
        case .storage: return ["get_storage_usage", "get_project_run_retention"]
        case .usage: return ["get_token_usage"]
        }
    }
}

@MainActor
final class NativeSettingsModel: ObservableObject {
    @Published var section: NativeSettingsSection = .general
    @Published var projectID: String?
    @Published var modelCategory = "api"
    @Published var search = ""
    @Published var values: [String: SettingsValue] = [:]
    @Published var loading = false
    @Published var busy = false
    @Published var error: String?
    @Published var message: String?
    @Published var detailSection: String?
    @Published var editor: SettingsEditor?
    private var generation = UUID()
    private var loadedProject: String?
    private var snapshots: [String: SettingsValue] = [:]
    let client: any NativeSettingsQuerying

    init(client: any NativeSettingsQuerying, projectID: String?) {
        self.client = client
        self.projectID = projectID
    }

    func load() async {
        let current = UUID()
        generation = current
        loading = true
        error = nil
        if loadedProject != projectID { values = [:]; snapshots = [:]; loadedProject = projectID }
        let project = projectID
        let requests = section.reads.filter { !($0 == "get_project_run_retention" && project == nil) }
        await withTaskGroup(of: (String, Result<SettingsValue, Error>).self) { group in
            for command in requests {
                group.addTask { [client] in
                    do { return (command, .success(try await client.invoke(command, args: [:], projectID: project))) }
                    catch { return (command, .failure(error)) }
                }
            }
            for await (command, result) in group {
                guard generation == current else { continue }
                switch result {
                case .success(let value):
                    if values[command] == nil || values[command] == snapshots[command] { values[command] = value }
                    snapshots[command] = value
                    if command == "get_settings", value["locale"] != .null { UserDefaults.standard.set(value["locale"].string, forKey: "nativeSettings.locale") }
                    if command == "get_appearance_prefs" { WispDesign.apply(value) }
                case .failure(let failure): error = failure.localizedDescription
                }
            }
        }
        if current == generation { loading = false }
    }

    var hasUnsavedChanges: Bool { values.contains { key, value in snapshots[key] != nil && snapshots[key] != value } }
    func discardDrafts() {
        for (key, value) in snapshots { values[key] = value }
        error = nil
        message = nil
    }

    func leave() { generation = UUID(); editor = nil }

    @discardableResult
    func run(_ command: String, _ args: [String: SettingsValue] = [:], refresh: Bool = true, success: String = "已保存") async -> SettingsValue? {
        guard !busy else { return nil }
        busy = true
        error = nil
        message = nil
        let current = generation
        let project = projectID
        do {
            let result = try await client.invoke(command, args: args, projectID: project)
            guard current == generation else { busy = false; return nil }
            busy = false
            message = success
            if refresh {
                // A successful write invalidates its corresponding draft only.
                let read = command.hasPrefix("set_") ? "get_" + command.dropFirst(4) : nil
                if let read { values[read] = nil; snapshots[read] = nil }
                await load()
            }
            return result
        } catch {
            if current == generation { self.error = error.localizedDescription }
            busy = false
            return nil
        }
    }

    func binding(_ command: String, _ key: String) -> Binding<SettingsValue> {
        Binding(get: { self.values[command]?[key] ?? .null }, set: { value in
            var document = self.values[command] ?? .object([:]); document[key] = value; self.values[command] = document
        })
    }

    static let numberLabels = ["max_iter": "每轮最大 Agent 迭代次数", "auto_continue_limit": "自动继续次数上限", "semantic_compact_idle_hours": "空闲多久后提示语义压缩"]
    static let numberDefaults: [String: Int64] = ["max_iter": 100, "auto_continue_limit": 10, "semantic_compact_idle_hours": 24]

    func numberText(_ key: String) -> String {
        // Absent fields in an older host use the documented defaults; an edited
        // empty value remains empty so validation can explain it without data loss.
        guard let value = values["get_settings"]?.object[key] else { return String(Self.numberDefaults[key] ?? 0) }
        return value.string
    }

    func saveSettings() async {
        guard let document = values["get_settings"], !document.object.isEmpty, !loading, !busy else { return }
        for key in ["max_iter", "auto_continue_limit", "semantic_compact_idle_hours"] {
            guard let value = document.object[key] else { continue }
            let minimum: Int64 = key == "auto_continue_limit" ? 1 : 0
            guard case .integer(let number) = value, number >= minimum else {
                error = localized(Self.numberLabels[key]!) + ": " + localized(minimum == 0 ? "请输入不小于 0 的整数。" : "请输入不小于 1 的整数。")
                message = nil
                return
            }
        }
        _ = await run("set_settings", ["settings": document])
    }
}

struct SettingsField: Identifiable {
    enum Kind { case text, secure, multiline, json, lines, integer, toggle, choice([(String, String)]), path, file
        indirect case object([SettingsField])
        indirect case records([SettingsField]) }
    let key: String
    let label: String
    var kind: Kind = .text
    var hint: String = ""
    var id: String { key }
    var initial: SettingsValue? {
        switch kind {
        case .text, .secure, .multiline: return .string("")
        case .lines, .records: return .array([])
        case .toggle: return .bool(false)
        case .choice(let choices): return choices.first.map { .string($0.0) }
        default: return nil
        }
    }
}

struct SettingsEditor: Identifiable {
    let id = UUID()
    var title: String
    var draft: SettingsValue
    var fields: [SettingsField]
    var command: String
    var parameter: String?
    var extra: [String: SettingsValue] = [:]
    var readOnly = false
    var destructiveCommand: String?
    var destructiveArgs: [String: SettingsValue] = [:]
}
