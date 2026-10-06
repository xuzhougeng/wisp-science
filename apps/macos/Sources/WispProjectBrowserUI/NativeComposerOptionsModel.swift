import Foundation
import SwiftUI
import WispProjectBrowser

@MainActor final class NativeComposerOptionsModel: ObservableObject {
    @Published private(set) var options: NativeComposerSessionOptions?
    @Published private(set) var specialists: [NativeComposerSpecialist] = []
    @Published private(set) var busy = false
    @Published private(set) var saving = false
    @Published private(set) var error: String?
    private let client: any NativeConversationQuerying
    let project: String
    let session: String
    private let writable: () -> Bool
    private var generation = UUID()
    var canEdit: Bool { writable() && !busy && error == nil && options != nil }
    init(client: any NativeConversationQuerying, project: String, session: String, writable: @escaping () -> Bool) {
        self.client = client; self.project = project; self.session = session; self.writable = writable
    }
    func close() { generation = UUID(); options = nil; specialists = []; busy = false; saving = false; error = nil }
    func load() async {
        guard !busy else { return }
        let current = generation; busy = true; error = nil
        defer { if generation == current { busy = false } }
        do {
            async let scoped = client.invoke("native_conversation_options", args: ["session_id": .string(session)], projectID: project)
            async let listed = client.invoke("list_specialists", args: [:], projectID: project)
            let value = try NativeComposerSessionOptions.decode(await scoped, session: session)
            let rows = try JSONDecoder().decode([NativeComposerSpecialist].self, from: JSONEncoder().encode(await listed))
            guard Set(rows.map(\.id)).count == rows.count, rows.allSatisfy({ !$0.id.isEmpty && !$0.name.isEmpty }) else { throw ProjectBrowserError.invalidResponse }
            guard generation == current, !Task.isCancelled else { return }
            options = value; specialists = rows.filter { !["reviewer", "reader", "archivist", "recap"].contains($0.id) }
        } catch {
            if generation == current, !Task.isCancelled { self.error = localized("无法读取对话选项，请重试。") + "\n" + error.localizedDescription }
        }
    }
    func setToggle(_ kind: String, enabled: Bool, confirmed: Bool = false) async {
        guard ["full_permission", "delegation", "auto_review"].contains(kind), kind != "full_permission" || !enabled || confirmed else { return }
        var change: [String: SettingsValue] = ["kind": .string(kind), "enabled": .bool(enabled)]
        if kind == "full_permission" { change["confirmed"] = .bool(confirmed) }
        await save(.object(change))
    }
    func setCompletion(_ policy: String, autoResume: Bool) async {
        guard options?.delegation == true, ["inline", "background"].contains(policy) else { return }
        await save(.object(["kind": .string("completion"), "policy": .string(policy), "auto_resume": .bool(policy == "background" && autoResume)]))
    }
    func setSpecialist(_ id: String) async {
        guard options?.specialist_locked == false, id.isEmpty || specialists.contains(where: { $0.id == id }) else { return }
        await save(.object(["kind": .string("specialist"), "id": .string(id)]))
    }
    private func save(_ change: SettingsValue) async {
        guard canEdit else { return }
        let current = generation; busy = true; saving = true; error = nil
        defer { if generation == current { busy = false; saving = false } }
        do {
            let value = try await client.invoke("native_conversation_options_set", args: ["session_id": .string(session), "change": change], projectID: project)
            let result = try NativeComposerSessionOptions.decode(value, session: session)
            let confirmed: Bool
            switch change["kind"].string {
            case "full_permission": confirmed = result.full_permission == change["enabled"].bool
            case "delegation": confirmed = result.delegation == change["enabled"].bool
            case "auto_review": confirmed = result.auto_review == change["enabled"].bool
            case "completion": confirmed = result.completion.policy == change["policy"].string && result.completion.auto_resume == change["auto_resume"].bool
            case "specialist": confirmed = (result.specialist?["id"].string ?? "") == change["id"].string
            default: confirmed = false
            }
            guard confirmed else { throw ProjectBrowserError.invalidResponse }
            guard generation == current, !Task.isCancelled else { return }
            options = result
        } catch {
            if generation == current { self.error = localized("选项未确认保存，请重新读取后核对；不会自动重试。") + "\n" + error.localizedDescription }
        }
    }
}
