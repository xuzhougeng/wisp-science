import SwiftUI
import WispProjectBrowser

struct SessionDeletionResult {
    let confirmed: Set<String>
    let error: String?
    let isCurrent: Bool
    var unconfirmed: BrowserSession? = nil
}

@MainActor
final class NativeSessionDelete: ObservableObject {
    @Published private(set) var targets: [BrowserSession] = []
    @Published private(set) var deletedIDs: Set<String> = []
    @Published private(set) var busy = false
    @Published private(set) var error: String?
    private var generation = UUID()

    var canDelete: Bool { !busy && !targets.isEmpty && error == nil }
    func begin(_ sessions: [BrowserSession]) {
        guard !busy, let project = sessions.first?.projectID,
              !project.isEmpty, sessions.allSatisfy({ !$0.id.isEmpty && $0.projectID == project }) else { return }
        reset()
        var ids: Set<String> = []
        targets = sessions.filter { ids.insert($0.id).inserted }
    }
    func dismiss() { guard !busy else { return }; reset() }
    func reset() { generation = UUID(); targets = []; deletedIDs = []; busy = false; error = nil }

    /// Return only confirmed deletions. A lost response stops the batch and is
    /// never replayed; the user must read the saved list before a new action.
    func delete(_ client: any NativeConversationQuerying) async -> SessionDeletionResult {
        guard canDelete else { return .init(confirmed: [], error: nil, isCurrent: false) }
        let current = generation; let sessions = targets
        var confirmed: Set<String> = []
        busy = true
        defer { if current == generation { busy = false } }
        for session in sessions {
            guard current == generation else { return .init(confirmed: confirmed, error: nil, isCurrent: false) }
            do {
                _ = try await client.invoke("native_conversation_delete", args: ["session_id": .string(session.id)], projectID: session.projectID)
                confirmed.insert(session.id)
                guard current == generation else { return .init(confirmed: confirmed, error: nil, isCurrent: false) }
                deletedIDs.insert(session.id)
            } catch {
                guard current == generation else { return .init(confirmed: confirmed, error: nil, isCurrent: false, unconfirmed: session) }
                self.error = "删除结果未能确认，后续会话尚未删除。请刷新会话后核对。\n" + error.localizedDescription
                return .init(confirmed: confirmed, error: self.error, isCurrent: true, unconfirmed: session)
            }
        }
        targets = []
        return .init(confirmed: confirmed, error: nil, isCurrent: true)
    }
}

struct NativeSessionDeleteSheet: View {
    @ObservedObject var model: NativeSessionDelete
    let confirm: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("删除会话").font(.headline)
            Text("将删除以下会话及其保存的消息。此操作无法撤销。").font(.callout)
            Text("运行中的会话会先停止。").font(.caption).foregroundStyle(.secondary)
            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(model.targets) { session in
                        HStack {
                            Text(session.title).lineLimit(2)
                            Spacer()
                            if model.deletedIDs.contains(session.id) { Text("已删除").font(.caption).foregroundStyle(.secondary) }
                        }
                    }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }.frame(maxHeight: 160)
            if let error = model.error { Text(error).font(.caption).foregroundStyle(.red).textSelection(.enabled) }
            HStack {
                Spacer()
                Button(model.error == nil ? "取消" : "关闭") { model.dismiss() }.disabled(model.busy)
                Button(model.busy ? "正在删除…" : "确认删除", role: .destructive, action: confirm).disabled(!model.canDelete)
            }
        }
        .padding(24).frame(width: 420)
        .interactiveDismissDisabled(model.busy)
        .background(NativeSettingsEscape(enabled: !model.busy) { model.dismiss() })
    }
}
