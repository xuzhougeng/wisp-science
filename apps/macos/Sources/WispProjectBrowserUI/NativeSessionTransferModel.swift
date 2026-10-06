import Foundation
import SwiftUI
import WispProjectBrowser

@MainActor final class NativeSessionTransferModel: ObservableObject {
    let source: BrowserSession
    let mode: NativeSessionTransferMode
    @Published private(set) var target = ""
    @Published private(set) var preview: NativeSessionTransferPreview?
    @Published private(set) var result: NativeSessionTransferResult?
    @Published private(set) var error: String?
    @Published private(set) var reading = false
    @Published private(set) var transferring = false
    @Published private(set) var uncertain = false
    @Published private(set) var includeArtifacts = false
    private let client: any NativeConversationQuerying
    private let targets: Set<String>
    private let writable: () -> Bool
    private var generation = UUID()
    private var closed = false
    var canTransfer: Bool { writable() && !closed && !reading && !transferring && !uncertain && result == nil && preview != nil && (!includeArtifacts || preview?.artifacts != nil && mode == .move) }
    init(client: any NativeConversationQuerying, source: BrowserSession, mode: NativeSessionTransferMode, projects: [String], writable: @escaping () -> Bool) {
        self.client = client; self.source = source; self.mode = mode; self.writable = writable
        targets = Set(projects.filter { $0 != source.projectID && !$0.hasPrefix("assistant:") }); target = projects.first { targets.contains($0) } ?? ""
    }
    func select(_ target: String) {
        guard !closed, !transferring, result == nil, targets.contains(target), self.target != target else { return }
        generation = UUID(); self.target = target; preview = nil; error = nil; reading = false; includeArtifacts = false
    }
    func chooseArtifacts(_ selected: Bool) {
        guard !closed, !reading, !transferring, result == nil, !uncertain, mode == .move, preview?.artifacts != nil else { return }
        includeArtifacts = selected
    }
    func close() { closed = true; generation = UUID(); reading = false; transferring = false }
    func readPreview() async {
        guard !closed, !transferring, result == nil, targets.contains(target) else { return }
        let current = UUID(); generation = current; let target = target
        reading = true; preview = nil; includeArtifacts = false; error = nil
        defer { if generation == current { reading = false } }
        do {
            let value = try await client.invoke("native_conversation_transfer_preview", args: ["session_id": .string(source.id), "target_project_id": .string(target), "mode": .string(mode.rawValue)], projectID: source.projectID)
            let preview = try NativeSessionTransferPreview.decode(value, project: source.projectID, session: source.id, target: target, mode: mode)
            guard !closed, generation == current, !Task.isCancelled else { return }; self.preview = preview
        } catch { if !closed, generation == current, !Task.isCancelled { self.error = localized("无法读取会话转移预览。") + "\n" + error.localizedDescription } }
    }
    func confirm() async -> NativeSessionTransferResult? {
        guard canTransfer, let preview, preview.target_project_id == target else { return nil }
        let current = generation, files = includeArtifacts; transferring = true; error = nil
        defer { if generation == current { transferring = false } }
        do {
            let value = try await client.invoke("native_conversation_transfer", args: preview.arguments(includeArtifacts: files), projectID: source.projectID)
            let result = try NativeSessionTransferResult.decode(value, reviewed: preview, includeArtifacts: files)
            guard !closed, generation == current, !Task.isCancelled else { return nil }; self.result = result; return result
        } catch {
            if !closed, generation == current {
                uncertain = true; self.error = localized("转移结果未确认，请刷新原项目和目标项目核对；本窗口不会再次提交。") + "\n" + error.localizedDescription
            }
            return nil
        }
    }
}
