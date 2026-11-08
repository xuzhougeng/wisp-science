import Foundation
import WispProjectBrowser

@MainActor
final class NativeSideChatModel: ObservableObject {
    struct Row: Identifiable {
        let id = UUID()
        let question: String
        let answer: NativeSideChatResponse?
        let model: String?
        let error: String?
    }
    @Published var draft = ""
    @Published var quotes: [NativeSideChatQuote] = []
    @Published private(set) var rows: [Row] = []
    @Published private(set) var busy = false
    @Published private(set) var options: [NativeSideChatOption] = []
    @Published private(set) var acpAgentID: String?
    @Published private(set) var changingModel = false
    @Published private(set) var modelSelectionUncertain = false
    private var optionsGeneration = UUID()
    @Published private(set) var error: String?
    let client: any NativeConversationQuerying
    let projectID: String
    let sessionID: String
    init(client: any NativeConversationQuerying, projectID: String, sessionID: String) {
        self.client = client; self.projectID = projectID; self.sessionID = sessionID
    }
    var selected: NativeSideChatOption? {
        if let acpAgentID { return options.first { $0.kind == "acp" && $0.id == acpAgentID } }
        return options.first { $0.kind == "http" && $0.active } ?? options.first { $0.kind == "http" }
    }
    var canSend: Bool { !busy && !changingModel && !modelSelectionUncertain && (!draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !quotes.isEmpty) }
    func loadOptions() async {
        let generation = UUID(); optionsGeneration = generation
        do {
            let value = try await client.invoke("native_conversation_panel_side_chat_options", args: ["session_id": .string(sessionID)], projectID: projectID)
            let result = try JSONDecoder().decode([NativeSideChatOption].self, from: JSONEncoder().encode(value))
            guard optionsGeneration == generation else { return }
            options = result; error = nil
            if modelSelectionUncertain { acpAgentID = nil; modelSelectionUncertain = false }
        } catch { if optionsGeneration == generation { self.error = error.localizedDescription } }
    }
    func select(_ option: NativeSideChatOption) async {
        guard !busy, !changingModel else { return }
        if option.kind == "acp" { acpAgentID = option.id; modelSelectionUncertain = false; error = nil; return }
        if option.kind == "http", option.id.isEmpty { acpAgentID = nil; modelSelectionUncertain = false; error = nil; return }
        optionsGeneration = UUID(); changingModel = true; modelSelectionUncertain = true; error = nil
        defer { changingModel = false }
        do {
            _ = try await client.invoke("set_active_model", args: ["id": .string(option.id)], projectID: projectID)
            acpAgentID = nil
            await loadOptions()
        } catch { modelSelectionUncertain = true; self.error = error.localizedDescription }
    }
    func clear() { guard !busy else { return }; rows = []; if !modelSelectionUncertain { error = nil } }
    func send() async {
        guard canSend else { return }
        let question = NativeSideChatQuote.question(draft, quotes: quotes)
        let label = selected?.label
        let agent = acpAgentID
        draft = ""; quotes = []; busy = true; error = nil
        rows.append(Row(question: question, answer: nil, model: nil, error: nil))
        defer { busy = false }
        do {
            let value = try await client.invoke("native_conversation_panel_side_chat", args: ["session_id": .string(sessionID), "question": .string(question), "acp_agent_id": agent.map(SettingsValue.string) ?? .null], projectID: projectID)
            let reply = try JSONDecoder().decode(NativeSideChatResponse.self, from: JSONEncoder().encode(value))
            guard reply.sessionId == sessionID else { throw ProjectBrowserError.invalidResponse }
            rows.append(Row(question: question, answer: reply, model: label, error: nil))
        } catch { rows.append(Row(question: question, answer: nil, model: nil, error: error.localizedDescription)) }
    }
}
