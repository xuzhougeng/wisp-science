import AppKit
import SwiftUI
import WispProjectBrowser

struct ComposerFile: Equatable, Identifiable {
    var id: String { path }
    var path: String
    var name: String
}

enum SavedAttachments {
    static func files(in text: String) -> [String] {
        for block in text.components(separatedBy: "\n\n") {
            guard let value = block.dropPrefix("Uploaded files: ") else { continue }
            return value.split(separator: ", ", omittingEmptySubsequences: true).map(String.init)
        }
        return []
    }
    static func body(in text: String) -> String {
        text.components(separatedBy: "\n\n").filter { !$0.hasPrefix("Uploaded files: ") }.joined(separator: "\n\n")
    }
}

private extension String {
    func dropPrefix(_ prefix: String) -> Substring? {
        hasPrefix(prefix) ? dropFirst(prefix.count) : nil
    }
}

@MainActor
final class NativeConversationModel: ObservableObject {
    @Published var draft = ""
    @Published var contextReady = true
    var restrictedAssistant = false
    var contextReference: NativeComposerReference?
    private var outgoingReferences: [NativeComposerReference] {
        guard let contextReference, !references.contains(where: { $0.id == contextReference.id }) else { return references }
        return references + [contextReference]
    }
    @Published var outlinePresented = false
    @Published private(set) var outline: [ConversationOutlineEntry] = []
    @Published private(set) var outlineLoading = false
    @Published private(set) var outlineError: String?
    @Published private(set) var savedHighlights: [NativeHighlight] = []
    @Published private(set) var savedHighlightRevision = 0
    @Published private(set) var savingSelections: Set<String> = []
    private var highlightsReadGeneration = UUID()
    @Published private(set) var revealedExcerpt: String?
    @Published private(set) var scrollTarget: Int?
    @Published private(set) var scrollRevision = 0
    @Published private(set) var snapshot: ConversationSnapshot?
    @Published private(set) var models: [SettingsValue] = []
    @Published private(set) var acpAgents: [SettingsValue] = []
    @Published private(set) var loading = false
    @Published private(set) var busy = false
    @Published private(set) var connectionError: String?
    @Published private(set) var operationError: String?
    @Published private(set) var uncertainSend = false
    @Published private(set) var showingHistory = false
    @Published private(set) var history: ConversationSnapshot?
    @Published private(set) var references: [NativeComposerReference] = []
    private var stagedReferences: [String: [NativeComposerReference]] = [:]
    private var sentReferences: [String: [NativeComposerReference]] = [:]
    let composer: NativeComposerModel
    let completions: NativeComposerCompletionModel
    @Published private(set) var attachments: [ComposerFile] = []
    @Published private var legacyQueuedFollowUp: String?
    @Published private(set) var uncertainQueue = false
    private var uncertainQueues: Set<String> = []
    private var queuedBySession: [String: String] = [:]
    private var stagedFiles: [String: [ComposerFile]] = [:]
    private var drafts: [String: String] = [:]
    private var questionDrafts: [String: (target: NativeQuestionTarget, text: String, prefix: String)] = [:]
    private var submittedAcpRequests: Set<String> = []
    private var submittedApprovals: Set<String> = []
    private var projectID: String?
    private var sessionID: String?
    private var generation = UUID()
    private var polling: Task<Void, Never>?
    private var pending: (id: String, text: String)?
    private var pendingSends: [String: (id: String, text: String)] = [:]
    private var submittedDrafts: [String: (id: String, text: String)] = [:]
    private var retiredEpochs: Set<String> = []
    @Published private var uncertainModes: Set<String> = []
    @Published private var uncertainHistory: Set<String> = []
    @Published private var uncertainPlanDecisions: Set<String> = []
    var historyUncertain: Bool { sessionID.map { uncertainHistory.contains($0) } ?? false }
    private var timers: [String: NativeSessionTimerModel] = [:]
    func timerModel(project: String, session: String) -> NativeSessionTimerModel {
        let key = project + "\0" + session
        if let model = timers[key] { return model }
        let model = NativeSessionTimerModel(client: client, project: project, session: session); timers[key] = model; return model
    }
    let client: any NativeConversationQuerying
    init(client: any NativeConversationQuerying) { self.client = client; composer = NativeComposerModel(client: client); completions = NativeComposerCompletionModel(client: client) }
    var visibleItems: [ConversationItem] { (showingHistory ? history : snapshot)?.items ?? [] }
    var isAcp: Bool { snapshot?.acp_agent_id != nil || snapshot?.model_id.hasPrefix("acp:") == true }
    var modelLabel: String {
        if isAcp {
            let id = snapshot?.acp_agent_id ?? String((snapshot?.model_id ?? "").dropFirst(4))
            return acpAgents.first(where: { $0["id"].string == id })?["label"].string ?? String((snapshot?.model_id ?? "ACP").dropFirst(4))
        }
        return models.first(where: { $0["id"].string == snapshot?.model_id })?["label"].string ?? localized("选择模型")
    }
    private var canChangeConversationSettings: Bool {
        snapshot?.read_only == false && snapshot?.running == false && snapshot?.stopping == false
            && !busy && !composer.busy && !showingHistory && !uncertainSend && !uncertainQueue && !historyUncertain
            && !(sessionID.map { uncertainModes.contains($0) } ?? false) && connectionError == nil
    }
    var canChangeMode: Bool { canChangeConversationSettings && !isAcp }
    var canEditComposerOptions: Bool {
        canAttach && snapshot?.read_only == false && !composer.busy && !uncertainSend && !uncertainQueue && !historyUncertain
            && !(sessionID.map { uncertainModes.contains($0) } ?? false) && connectionError == nil
    }
    var canChangeAcpSettings: Bool { canChangeConversationSettings && isAcp && snapshot?.acp_state?.frameID == sessionID && snapshot?.acp_state != nil }
    func setAcpMode(_ id: String) async {
        guard canChangeAcpSettings, let state = snapshot?.acp_state, state.currentMode != id, state.modeChoices.contains(where: { $0.id == id }) else { return }
        await changeMode("native_conversation_acp_setting", args: ["change": .object(["kind": .string("mode"), "id": .string(id)])])
    }
    func setAcpConfig(_ id: String, value: SettingsValue) async {
        guard canChangeAcpSettings, let state = snapshot?.acp_state, state.allows(id, value: value), state.configurations.first(where: { $0.id == id })?.current != value else { return }
        await changeMode("native_conversation_acp_setting", args: ["change": .object(["kind": .string("config"), "id": .string(id), "value": value])])
    }
    var latestProposal: NativePlanTarget? {
        guard let page = snapshot, let user = page.items.lastIndex(where: { $0.role == "user" }),
              let plan = page.items.lastIndex(where: { $0.role == "plan" }), plan > user,
              let proposal = page.items[plan].proposal, proposal.valid else { return nil }
        let index = (page.user_offset ?? 0) + page.items.filter { $0.role == "user" }.count - 1
        return .init(project: page.project_id, session: page.session_id, userIndex: index,
                     turn: page.history_state?.turns.first { $0.user_index == index }, proposal: proposal)
    }
    func planDecisionUncertain(_ target: NativePlanTarget) -> Bool { uncertainPlanDecisions.contains(target.id) }
    func proposalModeActive(_ target: NativePlanTarget) -> Bool {
        guard latestProposal == target, target.project == projectID, target.session == sessionID else { return false }
        return target.proposal.source == "native" ? !isAcp && snapshot?.acp_state == nil && snapshot?.plan_mode == true : isAcp && snapshot?.acp_state?.exitPlanMode != nil
    }
    func canDecidePlan(_ target: NativePlanTarget) -> Bool {
        canChangeConversationSettings && proposalModeActive(target) && !planDecisionUncertain(target)
    }
    func acknowledgePlanDecision(_ target: NativePlanTarget) {
        guard !busy, connectionError == nil, latestProposal == target else { return }
        uncertainPlanDecisions.remove(target.id); operationError = nil
    }
    /// Leaving Plan is confirmed by a fresh snapshot before dispatching execution.
    func decidePlan(_ target: NativePlanTarget, execute: Bool) async {
        guard canDecidePlan(target) else { return }
        let current = generation, originalDraft = draft, files = attachments, refs = references
        let exitMode = target.proposal.source == "acp" ? snapshot?.acp_state?.exitPlanMode : nil
        var confirmed = false
        busy = true; operationError = nil
        do {
            var args: [String: SettingsValue] = ["session_id": .string(target.session)]
            if let exitMode { args["change"] = .object(["kind": .string("mode"), "id": .string(exitMode)]) }
            else { args["enabled"] = .bool(false) }
            _ = try await client.invoke(exitMode == nil ? "native_conversation_plan" : "native_conversation_acp_setting", args: args, projectID: target.project)
            guard generation == current else { return }
            await refresh()
            guard generation == current else { return }
            confirmed = connectionError == nil && snapshot?.running == false && snapshot?.stopping == false && snapshot?.read_only == false
                && (exitMode == nil ? snapshot?.plan_mode == false : snapshot?.acp_state?.currentMode == exitMode)
            guard confirmed else { throw ProjectBrowserError.unavailable(localized("尚未确认退出计划模式。")) }
            if latestProposal != target {
                confirmed = false; operationError = localized("计划内容已变化；已退出计划模式，请核对后再发送。")
            } else if execute && (draft != originalDraft || attachments != files || references != refs) {
                confirmed = false; operationError = localized("输入内容已变化；已退出计划模式，当前草稿已保留，请核对后再发送。")
            }
        } catch {
            uncertainPlanDecisions.insert(target.id)
            uncertainModes.insert(target.session)
            if generation == current { operationError = localized("计划模式切换未确认，不会自动重试或启动执行。") + "\n" + error.localizedDescription }
        }
        guard generation == current else { return }
        busy = false
        guard confirmed, execute else { return }
        if draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { draft = localized("批准并执行") }
        await send()
    }
    func setPlanMode(_ enabled: Bool) async {
        guard canChangeMode, snapshot?.plan_mode != nil, snapshot?.plan_mode != enabled else { return }
        await changeMode("native_conversation_plan", args: ["enabled": .bool(enabled)])
    }
    func setFastMode(_ enabled: Bool) async {
        guard canChangeMode, let mode = snapshot?.fast_mode, mode.enabled != enabled, let model = snapshot?.model_id else { return }
        await changeMode("native_conversation_fast", args: ["enabled": .bool(enabled), "model_id": .string(model)])
    }
    private func changeMode(_ command: String, args: [String: SettingsValue]) async {
        guard let project = projectID, let session = sessionID else { return }
        let current = generation
        busy = true; operationError = nil
        defer { if generation == current { busy = false } }
        do {
            var args = args; args["session_id"] = .string(session)
            _ = try await client.invoke(command, args: args, projectID: project)
            if generation == current { await refresh() }
        } catch {
            uncertainModes.insert(session)
            if generation == current { operationError = localized("模式未确认保存，请重新读取会话后核对。") + "\n" + error.localizedDescription }
        }
    }
    func historyTarget(row: Int, kind: String, checkpoint: String? = nil) -> NativeHistoryTarget? {
        guard let page = showingHistory ? history : snapshot, let state = page.history_state,
              row >= 0, row < page.items.count, ["user", "assistant"].contains(page.items[row].role),
              let currentRevision = snapshot?.history_state?.revision else { return nil }
        var user = (page.user_offset ?? 0) - 1
        var userRow: Int?
        for index in 0...row where page.items[index].role == "user" { user += 1; userRow = index }
        guard let userRow, user >= 0, user < state.turns.count else { return nil }
        return NativeHistoryTarget(project: page.project_id, session: page.session_id, turn: state.turns[user],
                                   revision: currentRevision, kind: kind, checkpoint: checkpoint,
                                   draft: Self.historyDraft(page.items[userRow].text))
    }
    static func historyDraft(_ text: String) -> String {
        let markers = ["Uploaded files: ", "Attached artifacts: ", "Attached sessions: ", "Project context: ", "Selected skills: ", "Selected workflows: ", "Target environments: ", "Target runtimes: ", "AI source-edit instruction: ", "Feedback context: "]
        let ends = markers.compactMap { text.range(of: "\n\n" + $0)?.lowerBound }
        return ends.min().map { String(text[..<$0]).trimmingCharacters(in: .whitespacesAndNewlines) } ?? text
    }
    func canHistoryAction(_ target: NativeHistoryTarget) -> Bool {
        guard let page = snapshot, let state = page.history_state, page.read_only == false,
              projectID == target.project, sessionID == target.session, !busy, !uncertainSend, !uncertainQueue,
              !historyUncertain, connectionError == nil, target.turn.user_index >= 0,
              target.turn.user_index < state.turns.count, state.turns[target.turn.user_index] == target.turn else { return false }
        switch target.kind {
        case "branch":
            return state.can_branch && ["before_user", "after_response"].contains(target.checkpoint ?? "")
                && (!(page.running || page.stopping) || target.turn.user_index < state.turns.count - 1)
        case "rewind", "undo":
            return !page.running && !page.stopping && !state.reviewing && !isAcp && state.revision == target.revision
                && queuedTurns.isEmpty && legacyQueuedFollowUp == nil
                && (target.kind != "undo" || target.turn.user_index == state.turns.count - 1)
        default: return false
        }
    }
    func previewUndo(_ target: NativeHistoryTarget) async throws -> NativeTurnUndoPreview? {
        guard target.kind == "undo", canHistoryAction(target) else { return nil }
        let current = generation
        let turn = try JSONDecoder().decode(SettingsValue.self, from: JSONEncoder().encode(target.turn))
        let value = try await client.invoke("native_conversation_history_action", args: [
            "session_id": .string(target.session), "target": turn, "revision": .string(target.revision), "action": .object(["kind": .string("undo_preview")])
        ], projectID: target.project)
        guard generation == current, !Task.isCancelled else { return nil }
        guard value["session_id"].string == target.session, value["target"] == turn else { throw ProjectBrowserError.invalidResponse }
        return try JSONDecoder().decode(NativeTurnUndoPreview.self, from: JSONEncoder().encode(value["result"]))
    }
    var canReadContext: Bool { snapshot?.context_view == true && !isAcp && connectionError == nil }
    var canCompact: Bool {
        canReadContext && canChangeConversationSettings && !visibleItems.isEmpty
            && queuedTurns.isEmpty && legacyQueuedFollowUp == nil && snapshot?.history_state?.reviewing != true
    }
    func readContext(project: String, session: String) async throws -> NativeConversationContext? {
        guard canReadContext, projectID == project, sessionID == session else { return nil }
        let current = generation
        let value = try await client.invoke("native_conversation_context", args: ["session_id": .string(session)], projectID: project)
        guard generation == current, !Task.isCancelled else { return nil }
        return try NativeConversationContext.decode(value, project: project, session: session)
    }
    /// `/compact` uses the shared turn pipeline, but never consumes the draft,
    /// attachments or references. The request ledger still reconciles a lost ack.
    func compact(project: String, session: String, semantic: Bool, instruction: String) async -> Bool {
        guard canCompact, projectID == project, sessionID == session, instruction.utf8.count <= 16_384 else { return false }
        let current = generation, id = UUID().uuidString
        let text = semantic ? "/compact --semantic" + (instruction.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "" : " " + instruction.trimmingCharacters(in: .whitespacesAndNewlines)) : "/compact"
        busy = true; operationError = nil; pending = (id, ""); pendingSends[session] = pending
        defer { if generation == current { busy = false } }
        do {
            let reply = try await client.invoke("native_conversation_send", args: ["session_id": .string(session), "request_id": .string(id), "message": .string(text)], projectID: project)
            guard reply["session_id"].string == session, reply["request_id"].string == id, !reply["epoch"].string.isEmpty else { throw ProjectBrowserError.invalidResponse }
            pendingSends[session] = nil
            guard generation == current else { return false }
            pending = nil; uncertainSend = false
            await refresh()
            return true
        } catch {
            guard generation == current else { return false }
            uncertainSend = true
            operationError = localized("压缩请求未确认，请核对上下文和最新消息；不会自动重试。") + "\n" + error.localizedDescription
            await refresh()
            return false
        }
    }
    func undoCompaction(project: String, session: String, epoch: UInt64) async -> Bool {
        guard canCompact, projectID == project, sessionID == session, epoch > 0, epoch <= UInt64(Int64.max) else { return false }
        let current = generation
        busy = true; operationError = nil
        defer { if generation == current { busy = false } }
        do {
            let reply = try await client.invoke("native_conversation_context_undo", args: ["session_id": .string(session), "head_epoch": .integer(Int64(clamping: epoch))], projectID: project)
            guard reply["project_id"].string == project, reply["session_id"].string == session,
                  case .integer(let undone) = reply["undone_epoch"], undone > 0, UInt64(undone) == epoch else { throw ProjectBrowserError.invalidResponse }
            guard generation == current else { return false }
            await refresh(); return true
        } catch {
            uncertainHistory.insert(session)
            if generation == current { operationError = localized("撤销压缩未确认，请重新读取上下文后核对；不会自动重试。") + "\n" + error.localizedDescription }
            return false
        }
    }
    func acknowledgeHistoryResult() {
        guard !busy, connectionError == nil, let sessionID else { return }
        uncertainHistory.remove(sessionID); operationError = nil
    }
    /// The confirmation retains the exact turn/revision originally inspected.
    func performHistoryAction(_ target: NativeHistoryTarget, editedDraft: String) async -> String? {
        guard canHistoryAction(target) else { return nil }
        let current = generation; let originalDraft = draft
        busy = true; operationError = nil
        defer { if generation == current { busy = false } }
        do {
            let turn = try JSONDecoder().decode(SettingsValue.self, from: JSONEncoder().encode(target.turn))
            let value = try await client.invoke("native_conversation_history_action", args: [
                "session_id": .string(target.session), "target": turn, "revision": .string(target.revision), "action": target.action
            ], projectID: target.project)
            guard value["session_id"].string == target.session, value["target"] == turn else { throw ProjectBrowserError.invalidResponse }
            if target.kind == "branch" {
                guard case .string(let id) = value["result"], !id.isEmpty else { throw ProjectBrowserError.invalidResponse }
                drafts[id] = target.checkpoint == "before_user" ? editedDraft : ""
                guard generation == current else { return nil }
                return id
            }
            guard generation == current else { return nil }
            latest()
            if draft == originalDraft && originalDraft.isEmpty { draft = target.draft }
            await refresh()
            return ""
        } catch {
            uncertainHistory.insert(target.session)
            if generation == current { operationError = localized("历史操作未确认，请重新读取并核对会话后再继续；不会自动重试。") + "\n" + error.localizedDescription }
            return nil
        }
    }
    var canAttach: Bool { !restrictedAssistant && projectID != nil && sessionID != nil && !busy && snapshot?.read_only != true && !showingHistory }
    var canReference: Bool { canAttach && snapshot?.composer_references == true && connectionError == nil && !uncertainSend }
    var canQueueFollowUp: Bool {
        let hasText = !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        return contextReady && (hasText || !attachments.isEmpty || !references.isEmpty) && (snapshot?.queue != nil || legacyQueuedFollowUp == nil) && !uncertainQueue && !uncertainSend && !historyUncertain && snapshot?.running == true && snapshot?.read_only == false && (!isAcp || snapshot?.acp_agent_id != nil) && !busy && !showingHistory && connectionError == nil && (references.isEmpty || snapshot?.composer_references == true)
    }
    var queuedTurns: [ConversationQueueItem] { snapshot?.queue?.items ?? [] }
    var queuedFollowUp: String? { snapshot?.queue == nil ? legacyQueuedFollowUp : queuedTurns.first?.message }
    var canSend: Bool {
        let hasText = !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        return contextReady && (hasText || !attachments.isEmpty || !references.isEmpty) && snapshot != nil && snapshot?.running == false && snapshot?.read_only == false && !busy && !uncertainSend && !uncertainQueue && !historyUncertain && !(sessionID.map { uncertainModes.contains($0) } ?? false) && connectionError == nil && !showingHistory && (references.isEmpty || snapshot?.composer_references == true)
    }

    func open(project: String, session: String) async {
        pause()
        outlinePresented = false; outline = []; outlineError = nil; outlineLoading = false; scrollTarget = nil; revealedExcerpt = nil
        savedHighlights = []; savingSelections = []; highlightsReadGeneration = UUID()
        projectID = project; sessionID = session; draft = drafts[session] ?? ""; attachments = stagedFiles[session] ?? []; legacyQueuedFollowUp = queuedBySession[session]; uncertainQueue = uncertainQueues.contains(session)
        completions.bind(project: project, session: session)
        references = stagedReferences[session] ?? []
        snapshot = nil; history = nil; showingHistory = false; pending = pendingSends[session]; uncertainSend = pending != nil; retiredEpochs = []
        operationError = pending != nil ? "上次发送结果尚未确认。请核对最新消息；不会自动重发。" : uncertainQueue ? "后续未能确认排队，不会自动重试。" : nil
        if historyUncertain { operationError = localized("历史操作未确认，请重新读取并核对会话后再继续；不会自动重试。") }
        models = []; acpAgents = []
        connectionError = nil; loading = true; busy = false
        let current = generation
        do {
            let prefs = try await client.invoke("get_appearance_prefs", args: [:], projectID: project)
            if generation == current { WispDesign.apply(prefs) }
        } catch { if generation == current { operationError = "未能读取输入偏好：\(error.localizedDescription)" } }
        guard generation == current else { return }
        await refresh()
        guard generation == current else { return }
        loading = false
        if snapshot != nil {
            do { _ = try await client.invoke("native_conversation_seen", args: ["session_id": .string(session)], projectID: project) }
            catch { if generation == current { operationError = "未能标记已查看：\(error.localizedDescription)" } }
        }
        guard generation == current else { return }
        do {
            let rows = try await client.invoke("list_models", args: [:], projectID: project).array.filter { !$0["use_for_image_generation"].bool && !$0["use_for_video_generation"].bool }
            if generation == current { models = rows }
        }
        catch { if generation == current { operationError = error.localizedDescription } }
        guard generation == current else { return }
        do {
            let agents = try await client.invoke("list_acp_agents", args: [:], projectID: project).array
            if generation == current { acpAgents = agents }
        } catch { if generation == current { operationError = error.localizedDescription } }
        guard generation == current else { return }
        polling = Task { [weak self] in
            while !Task.isCancelled {
                let delay: UInt64 = self?.connectionError != nil ? 2_000_000_000 : (self?.snapshot?.running == true ? 350_000_000 : 1_500_000_000)
                do { try await Task.sleep(nanoseconds: delay) } catch { break }
                guard !Task.isCancelled else { break }
                await self?.refresh()
            }
        }
    }
    /// Only text can follow a confirmed move: staged files and references still
    /// belong to the source project, so the move entry point requires clearing
    /// them first. Keep the source draft too; never dispatch it automatically.
    func retainDraftForTransferredSession(_ result: NativeSessionTransferResult) {
        guard result.mode == .move, result.project_id == projectID, result.session_id == sessionID,
              result.frame_id != sessionID, attachments.isEmpty, references.isEmpty else { return }
        drafts[result.frame_id] = draft
        pause()
    }
    func pause() {
        if let sessionID {
            drafts[sessionID] = draft
            stagedFiles[sessionID] = attachments
            stagedReferences[sessionID] = references
            if let legacyQueuedFollowUp { queuedBySession[sessionID] = legacyQueuedFollowUp } else { queuedBySession.removeValue(forKey: sessionID) }
        }
        composer.reset(); completions.reset()
        generation = UUID(); polling?.cancel(); polling = nil
    }
    func refresh() async {
        guard let project = projectID, let session = sessionID else { return }
        let current = generation
        do {
            let value = try await client.snapshot(projectID: project, sessionID: session, beforeSeq: nil)
            guard current == generation else { return }
            if retiredEpochs.contains(value.epoch) { return }
            if let previous = snapshot {
                if previous.epoch == value.epoch && previous.sequence >= value.sequence { return }
                if previous.epoch != value.epoch { retiredEpochs.insert(previous.epoch) }
            }
            snapshot = value; connectionError = nil
            uncertainModes.remove(session)
            if value.queue != nil || !value.running { legacyQueuedFollowUp = nil; queuedBySession[session] = nil }
            if let pending, value.request_id == pending.id {
                if draft == pending.text { draft = "" }
                clearSentReferences(pending.id, session: session)
                self.pending = nil; pendingSends[session] = nil; uncertainSend = false; operationError = nil
            }
            if !value.running, let submitted = submittedDrafts[session], value.request_id == submitted.id {
                if value.error != nil {
                    if draft.isEmpty { draft = submitted.text }
                    if references.isEmpty { references = sentReferences[submitted.id] ?? []; stagedReferences[session] = references }
                }
                sentReferences[submitted.id] = nil
                submittedDrafts[session] = nil
            }
        } catch {
            if current == generation && !Task.isCancelled { connectionError = "连接暂时中断，正在重新读取会话：\(error.localizedDescription)" }
        }
    }
    func create(project: String, acpAgentID: String? = nil) async -> String? {
        guard !busy else { return nil }; busy = true; operationError = nil
        let current = generation
        defer { if generation == current { busy = false } }
        do {
            var args: [String: SettingsValue] = [:]
            if let acpAgentID { args["acp_agent_id"] = .string(acpAgentID) }
            let id = try await client.invoke("native_conversation_create", args: args, projectID: project).string
            guard !id.isEmpty else { throw ProjectBrowserError.invalidResponse }
            return id
        }
        catch {
            if generation == current { operationError = "新建会话未确认成功，请刷新列表后检查：\(error.localizedDescription)" }
            return nil
        }
    }
    func attach(source path: String, client: any NativeConversationQuerying) async {
        let path = path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard canAttach, let project = projectID, let session = sessionID, !path.isEmpty else { return }
        busy = true
        operationError = nil
        defer { busy = false }
        do {
            let value = try await client.invoke("native_conversation_attach", args: ["session_id": .string(session), "path": .string(path)], projectID: project)
            guard self.sessionID == session else { return }
            let saved = ComposerFile(path: value["path"].string, name: value["name"].string)
            guard saved.path.hasPrefix("uploads/"), !saved.name.isEmpty else {
                operationError = "附件未能确认添加，不会自动重试。"
                return
            }
            if !attachments.contains(where: { $0.path == saved.path }) { attachments.append(saved) }
            stagedFiles[session] = attachments
        } catch {
            guard self.sessionID == session else { return }
            operationError = "附件未能确认添加，不会自动重试。\n" + error.localizedDescription
        }
    }
    func bindComposer() async {
        guard let projectID, let sessionID, snapshot != nil else { return }
        await composer.bind(project: projectID, session: sessionID, profile: isAcp ? .null : models.first { $0["id"].string == snapshot?.model_id } ?? .null)
    }
    @discardableResult
    func addReference(_ option: NativeComposerReference) -> Bool {
        guard canReference, option.valid else { return false }
        if references.contains(where: { $0.id == option.id }) { return true }
        references.append(option)
        if let sessionID { stagedReferences[sessionID] = references }
        return true
    }
    func removeReference(_ id: String) {
        references.removeAll { $0.id == id }
        if let sessionID { stagedReferences[sessionID] = references }
    }
    @discardableResult
    func runComposerCommand(available: [NativeComposerCommand], execute: (NativeComposerCommand, String) -> Void) -> Bool {
        guard canRunComposerCommand(available: available), let (command, payload) = NativeComposerCommand.parse(draft) else { return false }
        completions.dismiss(); draft = ""
        if let sessionID { drafts[sessionID] = draft }
        execute(command, payload)
        return true
    }
    func canRunComposerCommand(available: [NativeComposerCommand]) -> Bool {
        guard canAttach, snapshot?.read_only == false, connectionError == nil, !uncertainSend,
              let (command, _) = NativeComposerCommand.parse(draft) else { return false }
        return available.contains(command)
    }
    private func clearSentReferences(_ request: String, session: String) {
        let sent = Set((sentReferences[request] ?? []).map(\.id))
        if sessionID == session { references.removeAll { sent.contains($0.id) }; stagedReferences[session] = references }
        else { stagedReferences[session]?.removeAll { sent.contains($0.id) } }
    }
    func removeAttachment(_ path: String) {
        attachments.removeAll { $0.path == path }
        if let sessionID { stagedFiles[sessionID] = attachments }
    }
    func queueFollowUp() async {
        guard canQueueFollowUp, let project = projectID, let session = sessionID else { return }
        let text = draft
        let selected = outgoingReferences
        let files = attachments.map(\.path)
        let id = UUID().uuidString
        let current = generation
        busy = true
        operationError = nil
        defer { if generation == current { busy = false } }
        do {
            var args: [String: SettingsValue] = ["session_id": .string(session), "request_id": .string(id), "message": .string(NativeComposerReference.message(text, references: outgoingReferences))]
            if !files.isEmpty { args["attachments"] = .array(files.map(SettingsValue.string)) }
            if !outgoingReferences.isEmpty { args["references"] = .array(outgoingReferences.map(\.reference)) }
            let receipt = try await client.invoke("native_conversation_enqueue", args: args, projectID: project)
            guard receipt["queued"].bool else { throw ProjectBrowserError.invalidResponse }
            stagedReferences[session]?.removeAll { selected.contains($0) }
            stagedFiles[session]?.removeAll { files.contains($0.path) }
            if drafts[session] == text { drafts[session] = "" }
            guard generation == current else { return }
            let summary = NativeComposerReference.message(text, references: selected)
            if snapshot?.queue == nil { legacyQueuedFollowUp = summary; queuedBySession[session] = summary }
            if draft == text { draft = "" }
            drafts[session] = draft
            attachments.removeAll { files.contains($0.path) }
            stagedFiles[session] = attachments
            references.removeAll { selected.contains($0) }; stagedReferences[session] = references
            if snapshot?.queue != nil { await refresh() }
        } catch {
            uncertainQueues.insert(session)
            guard generation == current else { return }
            uncertainQueue = true
            operationError = "后续未能确认排队，不会自动重试。\n" + error.localizedDescription
        }
    }
    func acknowledgeUncertainQueue() {
        if let sessionID { uncertainQueues.remove(sessionID) }
        uncertainQueue = false; operationError = nil
    }
    func canChangeQueuedTurn(_ item: ConversationQueueItem) -> Bool {
        !busy && !showingHistory && snapshot?.read_only == false && connectionError == nil
            && queuedTurns.contains { $0.id == item.id && $0.digest == item.digest && $0.state == "queued" }
    }
    @discardableResult
    func changeQueuedTurn(_ item: ConversationQueueItem, session expectedSession: String, action: String, message: String? = nil) async -> Bool {
        guard canChangeQueuedTurn(item), let project = projectID, let session = sessionID, session == expectedSession,
              ["edit", "cancel", "move_up", "move_down"].contains(action) else { return false }
        let current = generation; busy = true; operationError = nil
        defer { if generation == current { busy = false } }
        do {
            var value: [String: SettingsValue] = ["kind": .string(action)]
            if action == "edit" { value["message"] = .string(message ?? "") }
            _ = try await client.invoke("native_conversation_queue_action", args: ["session_id": .string(session), "id": .string(item.id), "digest": .string(item.digest), "action": .object(value)], projectID: project)
            guard generation == current else { return false }
            await refresh(); return generation == current
        } catch {
            guard generation == current else { return false }
            await refresh()
            guard generation == current else { return false }
            operationError = "队列操作未确认成功，请核对最新队列；不会自动重试。\n" + error.localizedDescription
            return false
        }
    }
    func send() async {
        guard canSend, let project = projectID, let session = sessionID else { return }
        let text = draft; let files = attachments.map(\.path); let id = UUID().uuidString; let current = generation
        sentReferences[id] = outgoingReferences
        busy = true; operationError = nil; pending = (id, text); pendingSends[session] = pending; submittedDrafts[session] = pending
        do {
            var args: [String: SettingsValue] = ["session_id": .string(session), "request_id": .string(id), "message": .string(NativeComposerReference.message(text, references: outgoingReferences))]
            if !files.isEmpty { args["attachments"] = .array(files.map(SettingsValue.string)) }
            if !outgoingReferences.isEmpty { args["references"] = .array(outgoingReferences.map(\.reference)) }
            _ = try await client.invoke("native_conversation_send", args: args, projectID: project)
            pendingSends[session] = nil
            if drafts[session] == text { drafts[session] = "" }
            stagedFiles[session] = []
            clearSentReferences(id, session: session)
            guard generation == current else { return }
            if draft == text { draft = "" }; pending = nil
            attachments = []
        } catch {
            guard generation == current else { return }
            uncertainSend = true
            operationError = "发送结果尚未确认。正在读取服务器状态；不会自动重发。\(error.localizedDescription)"
        }
        if generation == current { busy = false; await refresh() }
    }
    @discardableResult
    func prefillLibrary(_ text: String) -> Bool {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let sessionID, !text.isEmpty else { return false }
        if draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            draft = text
        } else {
            draft += (draft.hasSuffix("\n") ? "" : "\n") + text
        }
        drafts[sessionID] = draft
        return true
    }
    @discardableResult
    func replaceDraft(_ text: String) -> Bool {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let sessionID, !text.isEmpty else { return false }
        draft = text
        drafts[sessionID] = draft
        return true
    }
    func acknowledgeUncertainSend() { if let sessionID { pendingSends[sessionID] = nil }; uncertainSend = false; pending = nil; operationError = nil }
    func questionTarget(_ item: ConversationItem, index: Int) -> NativeQuestionTarget {
        NativeQuestionTarget(session: sessionID ?? "", index: index, text: item.text, generation: generation)
    }
    func questionState(_ target: NativeQuestionTarget) -> NativeQuestion.State {
        guard target.session == sessionID, target.generation == generation,
              visibleItems.indices.contains(target.index), visibleItems[target.index].role == "question",
              visibleItems[target.index].text == target.text, let question = NativeQuestion(target.text) else { return .expired }
        if question.state != .pending { return question.state }
        if visibleItems.dropFirst(target.index + 1).contains(where: { $0.role == "user" }) { return .answered }
        return .pending
    }
    func canStageQuestion(_ target: NativeQuestionTarget) -> Bool {
        questionState(target) == .pending && NativeQuestion(target.text)?.requestID == nil
            && snapshot?.read_only == false && !showingHistory && !busy && !uncertainSend && connectionError == nil
    }
    func canAnswerAcpQuestion(_ target: NativeQuestionTarget) -> Bool {
        guard questionState(target) == .pending, let id = NativeQuestion(target.text)?.requestID else { return false }
        return !showingHistory && !busy && connectionError == nil && !submittedAcpRequests.contains(id)
            && snapshot?.acp?.question_ids.contains(id) == true
    }
    @discardableResult
    func answerAcpQuestion(_ text: String, target: NativeQuestionTarget) async -> Bool {
        let answer = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard canAnswerAcpQuestion(target), let id = NativeQuestion(target.text)?.requestID, !answer.isEmpty else { return false }
        return await respondAcp("native_conversation_acp_answer", id: id, args: ["answer": .string(answer)])
    }
    func canRespondAcpPermission(_ permission: ConversationAcpPermission) -> Bool {
        !busy && !showingHistory && connectionError == nil && permission.frame_id == sessionID
            && !submittedAcpRequests.contains(permission.request_id)
            && snapshot?.acp?.permissions.contains(permission) == true
    }
    @discardableResult
    func respondAcpPermission(_ permission: ConversationAcpPermission, optionID: String?) async -> Bool {
        guard canRespondAcpPermission(permission), optionID == nil || permission.options.contains(where: { $0.id == optionID }) else { return false }
        return await respondAcp("native_conversation_acp_permission", id: permission.request_id, args: ["option_id": optionID.map(SettingsValue.string) ?? .null])
    }
    private func respondAcp(_ command: String, id: String, args: [String: SettingsValue]) async -> Bool {
        submittedAcpRequests.insert(id)
        var args = args; args["request_id"] = .string(id)
        let current = generation
        let success = await action(command, args)
        if !success && current == generation { operationError = "ACP 回复结果未能确认。请核对最新状态；不会自动重试。\n" + (operationError ?? "") }
        return success
    }
    @discardableResult
    func stageQuestionAnswer(_ text: String, target: NativeQuestionTarget) -> Bool {
        let answer = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard canStageQuestion(target), !answer.isEmpty else { return false }
        let prefix: String
        if let previous = questionDrafts[target.session], previous.target.isSameQuestion(as: target), previous.text == draft {
            prefix = previous.prefix
        } else {
            prefix = draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "" : draft
        }
        draft = prefix.isEmpty ? answer : prefix + (prefix.hasSuffix("\n\n") ? "" : "\n\n") + answer
        questionDrafts[target.session] = (target, draft, prefix)
        drafts[target.session] = draft
        return true
    }
    func stop() async { await action("native_conversation_stop", [:]) }
    func canApprove(_ approval: ConversationApproval) -> Bool {
        !busy && !showingHistory && connectionError == nil && snapshot?.read_only == false
            && !submittedApprovals.contains(approval.id) && approval.frame_id == sessionID
            && snapshot?.approvals.contains(approval) == true
    }
    func approvalScopes(_ approval: ConversationApproval) -> [String] { snapshot?.approval_scopes?[approval.id] ?? ["once"] }
    func approvalSubmitted(_ approval: ConversationApproval) -> Bool { submittedApprovals.contains(approval.id) && snapshot?.approvals.contains(approval) == true }
    func reconcileApproval(_ approval: ConversationApproval) async {
        guard !busy, approvalSubmitted(approval), let previous = snapshot, previous.session_id == approval.frame_id else { return }
        await refresh()
        guard let current = snapshot, current.project_id == previous.project_id, current.session_id == previous.session_id,
              current.epoch != previous.epoch || current.sequence > previous.sequence,
              current.approvals.contains(approval), connectionError == nil else { return }
        submittedApprovals.remove(approval.id); operationError = nil
    }
    @discardableResult
    func approve(_ approval: ConversationApproval, allowed: Bool, feedback: String? = nil, scope: String = "once") async -> Bool {
        guard canApprove(approval), approvalScopes(approval).contains(scope), allowed || scope == "once" else { return false }
        var args: [String: SettingsValue] = ["approval_id": .string(approval.approval_id), "approved": .bool(allowed)]
        if scope != "once" { args["scope"] = .string(scope) }
        if !allowed, let feedback = feedback?.trimmingCharacters(in: .whitespacesAndNewlines), !feedback.isEmpty { args["feedback"] = .string(feedback) }
        submittedApprovals.insert(approval.id)
        return await action("native_conversation_approve", args)
    }
    func selectModel(_ id: String) async { guard !isAcp else { return }; await action("native_conversation_model", ["model_id": .string(id)]) }
    @discardableResult
    private func action(_ command: String, _ args: [String: SettingsValue]) async -> Bool {
        guard !busy, let project = projectID, let session = sessionID else { return false }
        let current = generation; busy = true; operationError = nil
        var args = args; args["session_id"] = .string(session)
        var succeeded = false
        do { _ = try await client.invoke(command, args: args, projectID: project); succeeded = true }
        catch { if current == generation { operationError = error.localizedDescription } }
        if current == generation { busy = false; await refresh() }
        return succeeded && current == generation
    }
    func older() async {
        revealedExcerpt = nil
        guard let project = projectID, let session = sessionID,
              let cursor = (showingHistory ? history : snapshot)?.next_before_seq else { return }
        let current = generation
        do {
            let page = try await client.snapshot(projectID: project, sessionID: session, beforeSeq: cursor)
            guard current == generation else { return }
            history = page; showingHistory = true
        } catch { if current == generation { operationError = error.localizedDescription } }
    }
    func loadOutline() async {
        guard let project = projectID, let session = sessionID else { return }
        let current = generation
        outlineLoading = true; outlineError = nil
        defer { if current == generation { outlineLoading = false } }
        do {
            let value = try await client.invoke("native_conversation_outline", args: ["session_id": .string(session)], projectID: project)
            let entries = try JSONDecoder().decode([ConversationOutlineEntry].self, from: JSONEncoder().encode(value))
            guard current == generation else { return }
            outline = entries
        } catch { if current == generation { outlineError = error.localizedDescription } }
    }
    func navigateToQuestion(_ entry: ConversationOutlineEntry) async {
        revealedExcerpt = nil
        guard let project = projectID, let session = sessionID else { return }
        let current = generation
        do {
            let page = try await client.snapshot(projectID: project, sessionID: session, beforeSeq: entry.before_seq)
            guard current == generation else { return }
            // Global indexes disambiguate repeated prompts and newly appended turns.
            guard let offset = page.user_offset,
                  let target = Self.questionItemIndex(entry.user_index, offset: offset, items: page.items) else {
                throw ProjectBrowserError.unavailable("问题位置已变化，请刷新大纲后重试。")
            }
            history = page; showingHistory = true
            scrollTarget = target
            scrollRevision += 1
            outlinePresented = false
        } catch { if current == generation { outlineError = error.localizedDescription } }
    }
    func loadSavedHighlights(project: String, session: String) async {
        guard projectID == project, sessionID == session else { return }
        let current = generation; let read = UUID(); highlightsReadGeneration = read
        do {
            let value = try await client.invoke("native_conversation_panel_highlights", args: ["session_id": .string(session)], projectID: project)
            let rows = try JSONDecoder().decode([NativeHighlight].self, from: JSONEncoder().encode(value))
            guard current == generation, read == highlightsReadGeneration else { return }
            guard rows.allSatisfy({ $0.belongs(project: project, session: session) }) else { throw ProjectBrowserError.invalidResponse }
            savedHighlights = rows
        } catch { if current == generation, read == highlightsReadGeneration { operationError = error.localizedDescription } }
    }
    func removeSavedHighlight(_ id: String, project: String, session: String) {
        guard projectID == project, sessionID == session else { return }
        highlightsReadGeneration = UUID(); savedHighlights.removeAll { $0.id == id }
    }
    func saveSelection(_ text: String, project: String, session: String) async {
        guard projectID == project, sessionID == session, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, !savingSelections.contains(text) else { return }
        let current = generation; savingSelections.insert(text); operationError = nil
        defer { if current == generation { savingSelections.remove(text) } }
        do {
            let value = try await client.invoke("native_conversation_panel_highlight_star", args: ["session_id": .string(session), "text": .string(text)], projectID: project)
            let row = try JSONDecoder().decode(NativeHighlight.self, from: JSONEncoder().encode(value))
            guard current == generation else { return }
            guard row.belongs(project: project, session: session), row.code == text else { throw ProjectBrowserError.invalidResponse }
            highlightsReadGeneration = UUID(); savedHighlights.removeAll { $0.id == row.id }; savedHighlights.append(row)
            savedHighlightRevision += 1
        } catch { if current == generation { operationError = error.localizedDescription } }
    }
    func revealExcerpt(_ text: String) {
        guard let index = visibleItems.firstIndex(where: { item in
            NativeSavedExcerpt.range(in: Self.renderedText(item), excerpt: text) != nil
                || (item.role == "tool" && item.input.map { NativeSavedExcerpt.range(in: $0, excerpt: text) != nil } == true)
        }) else {
            operationError = "未在当前已加载的消息中找到原文；请打开对应历史记录后重试。"
            return
        }
        operationError = nil; revealedExcerpt = text; scrollTarget = index; scrollRevision += 1
    }
    static func renderedText(_ item: ConversationItem) -> String {
        if item.role == "usage" { return "" }
        if item.role == "tool" { return item.text }
        let source = item.role == "user" ? SavedAttachments.body(in: item.text) : item.text
        return NativeMathContent.plainText(NativeMarkdownContent.render(source, saved: [], scheme: .light))
    }
    func clearExcerpt(revision: Int) { if scrollRevision == revision { revealedExcerpt = nil } }
    static func questionItemIndex(_ target: Int, offset: Int, items: [ConversationItem]) -> Int? {
        var index = offset
        for (position, item) in items.enumerated() where item.role == "user" {
            if index == target { return position }
            index += 1
        }
        return nil
    }
    func latest() { revealedExcerpt = nil; showingHistory = false; history = nil }
}

func nativeDesktopHostURL() -> URL? {
    let configured = ProcessInfo.processInfo.environment["WISP_DESKTOP_HOST_PATH"].map { URL(fileURLWithPath: $0) }
    let bundled = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/Wisp Desktop Host.app/Contents/MacOS/wisp-tauri")
    let installed = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "science.wisp-science").flatMap { Bundle(url: $0)?.executableURL }
    return configured ?? (FileManager.default.isExecutableFile(atPath: bundled.path) ? bundled : installed)
}
