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
    @Published private(set) var loading = false
    @Published private(set) var busy = false
    @Published private(set) var connectionError: String?
    @Published private(set) var operationError: String?
    @Published private(set) var uncertainSend = false
    @Published private(set) var showingHistory = false
    @Published private(set) var history: ConversationSnapshot?
    @Published private(set) var attachments: [ComposerFile] = []
    @Published private(set) var queuedFollowUp: String?
    private var queuedBySession: [String: String] = [:]
    private var stagedFiles: [String: [ComposerFile]] = [:]
    private var drafts: [String: String] = [:]
    private var projectID: String?
    private var sessionID: String?
    private var generation = UUID()
    private var polling: Task<Void, Never>?
    private var pending: (id: String, text: String)?
    private var pendingSends: [String: (id: String, text: String)] = [:]
    private var submittedDrafts: [String: (id: String, text: String)] = [:]
    private var retiredEpochs: Set<String> = []
    let client: any NativeConversationQuerying
    init(client: any NativeConversationQuerying) { self.client = client }
    var visibleItems: [ConversationItem] { (showingHistory ? history : snapshot)?.items ?? [] }
    var canAttach: Bool { projectID != nil && sessionID != nil && !busy && snapshot?.read_only != true && !showingHistory }
    var canQueueFollowUp: Bool {
        let hasText = !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        return hasText && queuedFollowUp == nil && snapshot?.running == true && snapshot?.read_only != true && !busy && !showingHistory && connectionError == nil
    }
    var canSend: Bool {
        let hasText = !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        return (hasText || !attachments.isEmpty) && snapshot != nil && snapshot?.running == false && snapshot?.read_only == false && !busy && !uncertainSend && connectionError == nil && !showingHistory
    }

    func open(project: String, session: String) async {
        pause()
        outlinePresented = false; outline = []; outlineError = nil; outlineLoading = false; scrollTarget = nil; revealedExcerpt = nil
        savedHighlights = []; savingSelections = []; highlightsReadGeneration = UUID()
        projectID = project; sessionID = session; draft = drafts[session] ?? ""; attachments = stagedFiles[session] ?? []; queuedFollowUp = queuedBySession[session]
        snapshot = nil; history = nil; showingHistory = false; pending = pendingSends[session]; uncertainSend = pending != nil; retiredEpochs = []
        operationError = pending == nil ? nil : "上次发送结果尚未确认。请核对最新消息；不会自动重发。"
        connectionError = nil; loading = true; busy = false
        let current = generation
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
        polling = Task { [weak self] in
            while !Task.isCancelled {
                let delay: UInt64 = self?.connectionError != nil ? 2_000_000_000 : (self?.snapshot?.running == true ? 350_000_000 : 1_500_000_000)
                do { try await Task.sleep(nanoseconds: delay) } catch { break }
                guard !Task.isCancelled else { break }
                await self?.refresh()
            }
        }
    }
    func pause() {
        if let sessionID {
            drafts[sessionID] = draft
            stagedFiles[sessionID] = attachments
            if let queuedFollowUp { queuedBySession[sessionID] = queuedFollowUp } else { queuedBySession.removeValue(forKey: sessionID) }
        }
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
            if let pending, value.request_id == pending.id {
                if draft == pending.text { draft = "" }
                self.pending = nil; pendingSends[session] = nil; uncertainSend = false; operationError = nil
            }
            if !value.running, let submitted = submittedDrafts[session], value.request_id == submitted.id {
                if value.error != nil && draft.isEmpty { draft = submitted.text }
                submittedDrafts[session] = nil
            }
        } catch {
            if current == generation && !Task.isCancelled { connectionError = "连接暂时中断，正在重新读取会话：\(error.localizedDescription)" }
        }
    }
    func create(project: String) async -> String? {
        guard !busy else { return nil }; busy = true; operationError = nil
        let current = generation
        defer { if generation == current { busy = false } }
        do {
            let id = try await client.invoke("native_conversation_create", args: [:], projectID: project).string
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
    func removeAttachment(_ path: String) {
        attachments.removeAll { $0.path == path }
        if let sessionID { stagedFiles[sessionID] = attachments }
    }
    func queueFollowUp() async {
        guard canQueueFollowUp, let project = projectID, let session = sessionID else { return }
        let text = draft
        let files = attachments.map(\.path)
        let id = UUID().uuidString
        busy = true
        operationError = nil
        defer { busy = false }
        do {
            var args: [String: SettingsValue] = ["session_id": .string(session), "request_id": .string(id), "message": .string(text)]
            if !files.isEmpty { args["attachments"] = .array(files.map(SettingsValue.string)) }
            _ = try await client.invoke("native_conversation_enqueue", args: args, projectID: project)
            guard self.sessionID == session else { return }
            queuedFollowUp = text
            queuedBySession[session] = text
            if draft == text { draft = "" }
            drafts[session] = draft
            attachments = []
            stagedFiles[session] = []
        } catch {
            guard self.sessionID == session else { return }
            operationError = "后续未能确认排队，不会自动重试。\n" + error.localizedDescription
        }
    }
    func send() async {
        guard canSend, let project = projectID, let session = sessionID else { return }
        let text = draft; let files = attachments.map(\.path); let id = UUID().uuidString; let current = generation
        busy = true; operationError = nil; pending = (id, text); pendingSends[session] = pending; submittedDrafts[session] = pending
        do {
            var args: [String: SettingsValue] = ["session_id": .string(session), "request_id": .string(id), "message": .string(text)]
            if !files.isEmpty { args["attachments"] = .array(files.map(SettingsValue.string)) }
            _ = try await client.invoke("native_conversation_send", args: args, projectID: project)
            pendingSends[session] = nil
            if drafts[session] == text { drafts[session] = "" }
            stagedFiles[session] = []
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
    func stop() async { await action("native_conversation_stop", [:]) }
    func approve(_ approval: ConversationApproval, allowed: Bool) async {
        await action("native_conversation_approve", ["approval_id": .string(approval.approval_id), "approved": .bool(allowed)])
    }
    func selectModel(_ id: String) async { await action("native_conversation_model", ["model_id": .string(id)]) }
    private func action(_ command: String, _ args: [String: SettingsValue]) async {
        guard !busy, let project = projectID, let session = sessionID else { return }
        let current = generation; busy = true; operationError = nil
        var args = args; args["session_id"] = .string(session)
        do { _ = try await client.invoke(command, args: args, projectID: project) }
        catch { if current == generation { operationError = error.localizedDescription } }
        if current == generation { busy = false; await refresh() }
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
        if item.role == "tool" { return item.text }
        let attributed = (try? AttributedString(markdown: item.text, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(item.text)
        return String(attributed.characters)
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
