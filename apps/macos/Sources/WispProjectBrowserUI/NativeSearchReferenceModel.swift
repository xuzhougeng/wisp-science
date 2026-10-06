import Foundation
import WispProjectBrowser

/// Resolve a search selection through the scoped composer catalog before staging
/// it locally. The catalog reapplies privacy and ownership to stale search rows.
@MainActor final class NativeSearchReferenceModel: ObservableObject {
    @Published private(set) var reading = false
    @Published private(set) var error: String?
    private let client: any NativeConversationQuerying
    private let project: String
    private let session: String
    private let writable: () -> Bool
    private let accept: (NativeComposerReference) -> Bool
    private var generation = UUID()
    private var closed = false
    init(client: any NativeConversationQuerying, project: String, session: String, writable: @escaping () -> Bool, accept: @escaping (NativeComposerReference) -> Bool) {
        self.client = client; self.project = project; self.session = session; self.writable = writable; self.accept = accept
    }
    func invalidate() { generation = UUID(); reading = false; error = nil }
    func close() { closed = true; invalidate() }
    var available: Bool { !closed && !project.isEmpty && !session.isEmpty && writable() }
    static func referenceable(_ item: NativeSearchItem) -> Bool { item.valid && ["artifact", "session"].contains(item.kind) }
    func attach(_ item: NativeSearchItem) async -> Bool {
        guard available, !reading, !Task.isCancelled, Self.referenceable(item), !(item.kind == "session" && item.objectID == session) else { return false }
        let current = UUID(); generation = current; reading = true; error = nil
        defer { if generation == current { reading = false } }
        do {
            var query = ""
            for scalar in item.title.unicodeScalars {
                let text = String(scalar); if query.utf8.count + text.utf8.count > 512 { break }; query += text
            }
            let value = try await client.invoke("native_conversation_references", args: ["session_id": .string(session), "kind": .string(item.kind), "query": .string(query)], projectID: project)
            let options = try NativeComposerReferenceCatalog.decode(value, session: session)
            guard generation == current, available, !Task.isCancelled else { return false }
            guard let option = options.first(where: { $0.reference["kind"].string == item.kind && $0.reference["id"].string == item.objectID }) else {
                error = localized("此搜索结果已不可引用，请重新搜索。"); return false
            }
            return accept(option)
        } catch {
            if generation == current, available, !Task.isCancelled { self.error = localized("无法读取搜索结果的引用，请重试。") + "\n" + error.localizedDescription }
            return false
        }
    }
}
