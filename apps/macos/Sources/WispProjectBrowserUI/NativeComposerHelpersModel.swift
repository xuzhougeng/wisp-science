import Foundation
import SwiftUI
import WispProjectBrowser

/// Global composer preferences remain separate from scoped session options.
@MainActor final class NativeComposerHelpersModel: ObservableObject {
    @Published private(set) var memory: NativeComposerMemoryPreference?
    @Published private(set) var analysis: NativeFailureAnalysisSettings?
    @Published private(set) var reviewer: SettingsValue?
    @Published private(set) var choices: [NativeReviewerChoice] = []
    @Published private(set) var busy = false
    @Published private(set) var saving = false
    @Published private(set) var error: String?
    private let client: any NativeConversationQuerying
    let project: String
    private let writable: () -> Bool
    private var generation = UUID()
    var canEdit: Bool { writable() && !busy && error == nil && memory != nil && analysis != nil }
    var reviewerKey: String { reviewer.flatMap { try? NativeReviewerConfiguration.key($0) } ?? "" }
    var reviewerLabel: String {
        if let choice = choices.first(where: { $0.id == reviewerKey }) { return choice.label }
        return localized("配置已不可用") + " · " + reviewerKey
    }
    init(client: any NativeConversationQuerying, project: String, writable: @escaping () -> Bool) {
        self.client = client; self.project = project; self.writable = writable
    }
    func close() { generation = UUID(); memory = nil; analysis = nil; reviewer = nil; choices = []; busy = false; saving = false; error = nil }
    func load() async {
        guard !busy else { return }
        let current = generation; busy = true; error = nil
        defer { if generation == current { busy = false } }
        do {
            async let memoryView = client.invoke("get_memory_view", args: ["project_id": .string(project)], projectID: project)
            async let settings = client.invoke("get_auto_failure_analysis_settings", args: [:], projectID: project)
            async let specialists = client.invoke("list_specialists", args: [:], projectID: project)
            async let models = client.invoke("list_models", args: [:], projectID: project)
            async let agents = client.invoke("list_acp_agents", args: [:], projectID: project)
            let preference = try NativeComposerMemoryPreference.decode(await memoryView, project: project)
            let analysis = try NativeFailureAnalysisSettings.decode(await settings)
            let reviewer = try NativeReviewerConfiguration.catalog(await specialists).first { $0["id"] == .string("reviewer") }
            if let reviewer { _ = try NativeReviewerConfiguration.key(reviewer) }
            let choices = try NativeReviewerConfiguration.choices(models: await models, agents: await agents, defaultLabel: localized("默认 HTTP 模型"), followLabel: localized("跟随当前会话"))
            guard generation == current, !Task.isCancelled else { return }
            memory = preference; self.analysis = analysis; self.reviewer = reviewer; self.choices = choices
        } catch {
            if generation == current, !Task.isCancelled { self.error = localized("无法读取全局对话设置，请重试。") + "\n" + error.localizedDescription }
        }
    }
    func setMemory(_ enabled: Bool) async {
        guard memory?.enabled != enabled else { return }
        await save {
            let reply = try await self.client.invoke("set_memory_enabled", args: ["enabled": .bool(enabled), "project_id": .string(self.project)], projectID: self.project)
            let result = try NativeComposerMemoryPreference.decode(reply, project: self.project)
            guard result.enabled == enabled else { throw ProjectBrowserError.invalidResponse }
            return { self.memory = result }
        }
    }
    func setAnalysis(_ settings: NativeFailureAnalysisSettings) async {
        guard (1...100).contains(settings.failure_rate_threshold), (1...100).contains(settings.minimum_failures), analysis != settings else { return }
        await save {
            let reply = try await self.client.invoke("set_auto_failure_analysis_settings", args: ["settings": settings.value], projectID: self.project)
            let result = try NativeFailureAnalysisSettings.decode(reply)
            guard result == settings else { throw ProjectBrowserError.invalidResponse }
            return { self.analysis = result }
        }
    }
    func setReviewer(_ key: String) async {
        guard reviewer != nil, choices.contains(where: { $0.id == key }), key != reviewerKey else { return }
        let current = generation; let inspected = reviewerKey
        await save {
            let listed = try await self.client.invoke("list_specialists", args: [:], projectID: self.project)
            guard let fresh = try NativeReviewerConfiguration.catalog(listed).first(where: { $0["id"] == .string("reviewer") }),
                  try NativeReviewerConfiguration.key(fresh) == inspected else { throw ProjectBrowserError.invalidResponse }
            guard self.generation == current, self.writable(), !Task.isCancelled else { throw CancellationError() }
            let spec = try NativeReviewerConfiguration.changing(fresh, to: key)
            let reply = try await self.client.invoke("save_specialist_cmd", args: ["spec": spec], projectID: self.project)
            guard let result = try NativeReviewerConfiguration.catalog(reply).first(where: { $0["id"] == .string("reviewer") }),
                  try NativeReviewerConfiguration.key(result) == key else { throw ProjectBrowserError.invalidResponse }
            return { self.reviewer = result }
        }
    }
    private func save(_ operation: () async throws -> () -> Void) async {
        guard canEdit else { return }
        let current = generation; busy = true; saving = true; error = nil
        defer { if generation == current { busy = false; saving = false } }
        do {
            let apply = try await operation()
            guard generation == current, !Task.isCancelled else { return }; apply()
        } catch {
            if generation == current { self.error = localized("全局设置未确认保存，请重新读取后核对；不会自动重试。") + "\n" + error.localizedDescription }
        }
    }
}
