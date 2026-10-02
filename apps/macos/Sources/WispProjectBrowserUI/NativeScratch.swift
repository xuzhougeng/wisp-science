import SwiftUI
import WispProjectBrowser

struct ScratchSession: Codable, Equatable {
    var projectID: String
    var sessionID: String
    enum CodingKeys: String, CodingKey {
        case projectID = "project_id"
        case sessionID = "session_id"
    }
}

enum NativeScratchCommand {
    static let open = "native_scratch_open"
    static let close = "native_scratch_close"
    static let webViewStart = "start_scratch_chat"
}

@MainActor
final class NativeScratchModel: ObservableObject {
    @Published var presented = false
    @Published private(set) var projectID: String?
    @Published private(set) var sessionID: String?
    @Published private(set) var busy = false
    @Published private(set) var error: String?
    private var generation = UUID()

    func open(_ client: any NativeSettingsQuerying) async {
        guard !presented, !busy else { return }
        let current = UUID()
        generation = current
        busy = true
        error = nil
        defer { if generation == current { busy = false } }
        do {
            let value = try await client.invoke(NativeScratchCommand.open, args: [:], projectID: nil)
            guard generation == current, !presented else { return }
            let session = try JSONDecoder().decode(ScratchSession.self, from: JSONEncoder().encode(value))
            guard session.projectID.hasPrefix("scratch:"), !session.sessionID.isEmpty else {
                error = "随手一聊未能确认创建，不会自动重试。"
                return
            }
            projectID = session.projectID
            sessionID = session.sessionID
            presented = true
            error = nil
        } catch {
            guard generation == current else { return }
            self.error = "随手一聊未能确认创建，不会自动重试。\n" + error.localizedDescription
        }
    }

    func close(_ client: any NativeSettingsQuerying) async {
        guard presented, let projectID, !busy else { return }
        let current = UUID()
        generation = current
        busy = true
        error = nil
        defer { if generation == current { busy = false } }
        do {
            let value = try await client.invoke(NativeScratchCommand.close, args: [:], projectID: projectID)
            guard generation == current, presented, self.projectID == projectID else { return }
            guard value.bool else {
                error = "随手一聊已不存在。不会自动重试。"
                return
            }
            self.projectID = nil
            sessionID = nil
            presented = false
            error = nil
        } catch {
            guard generation == current, presented, self.projectID == projectID else { return }
            self.error = "随手一聊未能确认关闭，不会自动重试。\n" + error.localizedDescription
        }
    }
}

struct NativeScratchChat: View {
    @ObservedObject var model: ProjectBrowserModel
    @ObservedObject var scratch: NativeScratchModel
    @ObservedObject private var conversation: NativeConversationModel

    init(model: ProjectBrowserModel, scratch: NativeScratchModel) {
        self.model = model
        self.scratch = scratch
        self.conversation = model.nativeConversation()
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("随手一聊").font(.headline)
                Spacer()
                Button(scratch.busy ? "正在关闭…" : "关闭") { Task { await scratch.close(model.calendarClient()) } }
                    .disabled(scratch.busy)
            }
            .padding(16)
            if let error = scratch.error {
                Text(error).font(.caption).foregroundStyle(.red).textSelection(.enabled).padding(.horizontal, 16)
            }
            if let projectID = scratch.projectID, let sessionID = scratch.sessionID {
                NativeConversationView(conversation: conversation, projectID: projectID, sessionID: sessionID) { _ in }
                    .task(id: projectID + ":" + sessionID) { await conversation.open(project: projectID, session: sessionID) }
            }
        }
        .background(NativeSettingsEscape(enabled: !scratch.busy) { Task { await scratch.close(model.calendarClient()) } })
    }
}
