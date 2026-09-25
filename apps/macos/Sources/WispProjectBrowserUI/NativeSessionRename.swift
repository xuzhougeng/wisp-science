import SwiftUI
import WispProjectBrowser

@MainActor
final class NativeSessionRename: ObservableObject {
    @Published private(set) var target: BrowserSession?
    @Published var draft = ""
    @Published private(set) var busy = false
    @Published private(set) var error: String?
    private var generation = UUID()

    func begin(_ session: BrowserSession) {
        guard !busy else { return }
        generation = UUID(); target = session; draft = session.title; error = nil
    }
    func dismiss() { guard !busy else { return }; reset() }
    func reset() { generation = UUID(); target = nil; draft = ""; error = nil; busy = false }

    func save(_ client: any NativeConversationQuerying) async -> BrowserSession? {
        guard !busy, let target else { return nil }
        let title = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !title.isEmpty else { error = "请填写会话名称。"; return nil }
        let current = generation
        busy = true; error = nil
        defer { if current == generation { busy = false } }
        do {
            _ = try await client.invoke("native_conversation_rename", args: ["session_id": .string(target.id), "title": .string(title)], projectID: target.projectID)
            guard current == generation else { return nil }
            let result = BrowserSession(id: target.id, projectID: target.projectID, title: title, ts: target.ts, status: target.status, folderID: target.folderID)
            self.target = nil; draft = ""
            return result
        } catch {
            if current == generation { self.error = "会话名称未能确认保存，不会自动重试。\n" + error.localizedDescription }
            return nil
        }
    }
}

struct NativeSessionRenameSheet: View {
    @ObservedObject var model: NativeSessionRename
    let save: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("重命名会话").font(.headline)
            TextField("会话名称", text: $model.draft).textFieldStyle(.roundedBorder).disabled(model.busy)
            if let error = model.error { Text(error).font(.caption).foregroundStyle(.red).textSelection(.enabled) }
            HStack {
                Spacer()
                Button("取消") { model.dismiss() }.disabled(model.busy)
                Button(model.busy ? "正在保存…" : "保存", action: save).disabled(model.busy)
            }
        }
        .padding(24).frame(width: 380)
        .interactiveDismissDisabled(model.busy)
        .background(NativeSettingsEscape(enabled: !model.busy) { model.dismiss() })
    }
}
