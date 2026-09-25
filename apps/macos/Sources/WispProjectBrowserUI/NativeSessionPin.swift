import SwiftUI
import WispProjectBrowser

@MainActor
final class NativeSessionPin: ObservableObject {
    @Published private(set) var busy = false
    @Published private(set) var error: String?
    private var generation = UUID()

    func reset() { generation = UUID(); busy = false; error = nil }

    func toggle(_ session: BrowserSession, client: any NativeConversationQuerying) async -> Bool {
        guard !busy, let pinned = session.pinned else { return false }
        let current = generation
        busy = true; error = nil
        defer { if current == generation { busy = false } }
        do {
            _ = try await client.invoke("native_conversation_pin", args: ["session_id": .string(session.id), "pinned": .bool(!pinned)], projectID: session.projectID)
            return current == generation
        } catch {
            if current == generation { self.error = "置顶状态未能确认保存，不会自动重试。\n" + error.localizedDescription }
            return false
        }
    }
}
