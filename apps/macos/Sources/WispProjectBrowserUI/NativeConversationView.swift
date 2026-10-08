import AppKit
import SwiftUI
import WispProjectBrowser

struct NativeConversationView: View {
    @ObservedObject var conversation: NativeConversationModel
    var projectID: String?
    var sessionID: String?
    var createAcpConversation: ((String) -> Void)?
    var executeComposerCommand: ((NativeComposerCommand, String) -> Void)?
    var openHistoryBranch: ((String) -> Void)?
    var quoteSelection: (String) -> Void = { _ in }
    @Environment(\.colorScheme) private var scheme
    @AppStorage("nativeSettings.send_with_modifier") private var sendWithModifier = false
    @State private var referencePicker = false
    @State private var composerOptions = false
    @State private var runtimeActivity: NativeContextActivitySelection?
    @State private var confirmResend = false
    @State private var confirmRequeue = false
    @State private var followLatest = true
    @State private var expandedTools: Set<Int> = []
    @State private var feedbackApproval: ConversationApproval?
    @State private var historyAction: NativeHistoryTarget?
    @State private var undoAction: NativeHistoryTarget?
    @State private var modelContext = false
    @State private var timerPresented = false
    private func color(_ token: String) -> Color { WispDesign.color(token, scheme) }
    var body: some View {
        VStack(spacing: 0) {
            if let error = conversation.connectionError ?? conversation.operationError ?? conversation.snapshot?.error {
                HStack(alignment: .top) {
                    Text(error).font(WispDesign.font(size: 12)).textSelection(.enabled)
                    Spacer()
                    Button("重新读取") { Task { await conversation.refresh() } }
                    if conversation.uncertainSend { Button("已检查，允许再次发送…") { confirmResend = true } }
                    if conversation.uncertainQueue { Button(localized("已核对队列，允许继续…")) { confirmRequeue = true } }
                    if conversation.historyUncertain { Button(localized("已核对历史操作结果")) { conversation.acknowledgeHistoryResult() } }
                }.padding(12).foregroundStyle(.orange)
            }
            ScrollViewReader { scroll in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 16) {
                        HStack {
                            if (conversation.showingHistory ? conversation.history : conversation.snapshot)?.next_before_seq != nil {
                                Button("更早的消息") { Task { await conversation.older() } }
                            }
                            if conversation.showingHistory { Button("返回最新消息") { conversation.latest() } }
                            Spacer()
                        }
                        ForEach(Array(conversation.visibleItems.enumerated()), id: \.offset) { index, item in
                            message(item, index: index).id(index)
                        }
                        if conversation.loading { ProgressView().frame(maxWidth: .infinity) }
                        else if conversation.visibleItems.isEmpty { Text("向 Wisp Science 提问，开始这个会话。").foregroundStyle(color("text-muted")).padding(.vertical, 48).frame(maxWidth: .infinity) }
                        if conversation.snapshot?.running == true && !conversation.showingHistory {
                            HStack(spacing: 8) { ProgressView().controlSize(.small); Text(conversation.snapshot?.stopping == true ? "正在停止…" : "正在处理…").font(WispDesign.font(size: 12)) }
                        }
                        Color.clear.frame(height: 1).id("latest")
                    }.frame(maxWidth: 800).padding(.horizontal, 16).padding(.vertical, 16).frame(maxWidth: .infinity)
                }
                .onChange(of: conversation.scrollRevision) { _ in
                    if let target = conversation.scrollTarget { expandedTools.insert(target); followLatest = false; scroll.scrollTo(target, anchor: .center) }
                }
                .onChange(of: conversation.snapshot?.sequence) { _ in
                    if followLatest && !conversation.showingHistory { scroll.scrollTo("latest", anchor: .bottom) }
                }
            }
            if !conversation.showingHistory {
                if let permissions = conversation.snapshot?.acp?.permissions, !permissions.isEmpty {
                    ScrollView {
                        VStack(spacing: 12) {
                            ForEach(permissions) { permission in
                                NativeAcpPermissionCard(conversation: conversation, permission: permission)
                            }
                        }.frame(maxWidth: 800).padding(.horizontal, 24)
                    }.frame(maxHeight: 260).padding(.bottom, 12)
                }
                ForEach(conversation.snapshot?.approvals ?? []) { approval in
                    NativeToolApprovalCard(conversation: conversation, approval: approval, feedback: { feedbackApproval = approval }).id(approval.id)
                        .frame(maxWidth: 850).padding(.horizontal, 24).padding(.bottom, 12)
                }
            }
            composer
        }
            .task(id: conversation.scrollRevision) {
                let revision = conversation.scrollRevision
                guard conversation.revealedExcerpt != nil else { return }
                try? await Task.sleep(nanoseconds: 1_600_000_000)
                if !Task.isCancelled { conversation.clearExcerpt(revision: revision) }
            }
        .onChange(of: sessionID) { _ in expandedTools = []; feedbackApproval = nil; referencePicker = false; runtimeActivity = nil; historyAction = nil; undoAction = nil; composerOptions = false; modelContext = false; timerPresented = false }
        .onChange(of: projectID) { _ in modelContext = false }
        .onChange(of: conversation.showingHistory) { _ in expandedTools = [] }
        .task(id: (sessionID ?? "") + ":" + (conversation.snapshot?.model_id ?? "") + ":" + String(conversation.models.count)) {
            guard !conversation.restrictedAssistant else { return }
            await conversation.bindComposer()
            while !Task.isCancelled {
                do { try await Task.sleep(nanoseconds: 10_000_000_000) } catch { return }
                await conversation.composer.refreshContexts()
            }
        }
        .sheet(isPresented: $referencePicker) {
            NativeComposerReferencePicker(model: conversation.composer, select: { _ = conversation.addReference($0) }) { referencePicker = false }
        }
        .sheet(item: $runtimeActivity, onDismiss: { Task { await conversation.composer.refreshContexts() } }) { selection in
            if let projectID, let sessionID {
                NativeContextActivityView(client: conversation.client, projectID: projectID, sessionID: sessionID, selection: selection) { runtimeActivity = nil }
            }
        }
        .sheet(item: $feedbackApproval) { approval in
            NativeApprovalFeedback(approval: approval, conversation: conversation) { feedbackApproval = nil }
        }
        .sheet(item: $historyAction) { target in
            NativeHistoryActionSheet(conversation: conversation, target: target, close: { historyAction = nil }, openBranch: { openHistoryBranch?($0) })
        }
        .sheet(isPresented: $composerOptions) {
            if let projectID, let sessionID { NativeComposerOptionsSheet(conversation: conversation, project: projectID, session: sessionID) { composerOptions = false } }
        }
        .sheet(item: $undoAction) { target in NativeTurnUndoSheet(conversation: conversation, target: target) { undoAction = nil } }
        .sheet(isPresented: $timerPresented) {
            if let projectID, let sessionID { NativeSessionTimerSheet(model: conversation.timerModel(project: projectID, session: sessionID)) { timerPresented = false } }
        }
        .sheet(isPresented: $modelContext) {
            if let projectID, let sessionID { NativeConversationContextSheet(conversation: conversation, project: projectID, session: sessionID) { modelContext = false } }
        }
        .confirmationDialog("先核对最新消息，避免重复执行同一个任务。确认仍需再次发送？", isPresented: $confirmResend) {
            Button("保留草稿，允许再次发送") { conversation.acknowledgeUncertainSend() }
            Button("取消", role: .cancel) {}
        }
        .confirmationDialog(localized("先核对最新队列和会话，确认上次排队结果后再继续。"), isPresented: $confirmRequeue) {
            Button(localized("已核对，保留草稿并继续")) { conversation.acknowledgeUncertainQueue() }
            Button("取消", role: .cancel) {}
        }
    }
    private func markedText(_ item: ConversationItem, index: Int, input: Bool = false) -> AttributedString {
        let source = item.role == "user" && !input ? SavedAttachments.body(in: item.text) : item.text
        var text = AttributedString(input ? item.input ?? "" : source)
        if conversation.scrollTarget == index, let excerpt = conversation.revealedExcerpt,
           let range = NativeSavedExcerpt.range(in: String(text.characters), excerpt: excerpt) {
            let start = text.characters.index(text.startIndex, offsetBy: range.lowerBound)
            let end = text.characters.index(text.startIndex, offsetBy: range.upperBound)
            text[start..<end].backgroundColor = .yellow.opacity(0.4)
        }
        return text
    }
    @ViewBuilder private func message(_ item: ConversationItem, index: Int) -> some View {
        if item.role == "usage" {
            if let usage = NativeConversationUsage(item) { NativeConversationUsageView(usage: usage) }
        } else if let compaction = NativeTranscriptCompaction(item) {
            NativeCompactionCard(record: compaction, openContext: conversation.canReadContext ? { modelContext = true } : nil)
        } else {
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Text(item.role == "user" ? "你" : item.role == "tool" ? (item.tool_name ?? "工具") : item.role == "reasoning" ? "思考" : "Wisp Science")
                        .font(WispDesign.font(size: 12, weight: .semibold)).foregroundStyle(color("text-muted"))
                    if let ok = item.ok { Text(ok ? "已完成" : "失败").font(WispDesign.font(size: 11)).foregroundStyle(ok ? color("clay") : .orange) }
                    if let status = item.status { Text(status).font(.caption).foregroundStyle(.secondary) }
                    if let duration = item.duration_ms { Text(String(format: "%.1f s", Double(duration) / 1000)).font(.caption).foregroundStyle(.secondary).help(localized("工具耗时")) }
                    if let name = item.model_name, !name.isEmpty { Text(name).font(.caption).foregroundStyle(.secondary).lineLimit(1).help(name) }
                    if let timestamp = item.timestamp, timestamp > 0 {
                        Text(Date(timeIntervalSince1970: Double(timestamp)), style: .time).font(.caption).foregroundStyle(.secondary)
                            .help(Date(timeIntervalSince1970: Double(timestamp)).formatted(date: .abbreviated, time: .standard))
                    }
                }
                if item.role == "tool" {
                    DisclosureGroup(isExpanded: Binding(get: {
                        expandedTools.contains(index)
                    }, set: { expanded in
                        if expanded { expandedTools.insert(index) } else { expandedTools.remove(index) }
                    })) {
                        if let input = item.input, !input.isEmpty { selectableMessage(item, index: index, input: true) }
                        selectableMessage(item, index: index)
                    } label: { Text(item.text.isEmpty ? "执行中…" : String(item.text.prefix(180))).font(WispDesign.font(size: 13)).lineLimit(3) }
                    if let path = NativeMessageImageRequest.generatedPath(item) {
                        NativeMessageBody(text: AttributedString(""), markdown: NativeMessageImageRequest.attachmentMarkdown([path]), resources: item.resources ?? [], saved: [], revealed: nil, monospaced: false,
                                          client: conversation.client, project: projectID ?? "", session: sessionID ?? "", quote: nil, save: nil)
                            .id((projectID ?? "") + ":" + (sessionID ?? "") + ":generated:" + String(index))
                    }
                } else if item.role == "question", let question = NativeQuestion(item.text) {
                    NativeQuestionCard(conversation: conversation, target: conversation.questionTarget(item, index: index), question: question)
                        .id((sessionID ?? "") + ":" + String(index) + ":" + item.text)
                } else if item.role == "plan" {
                    NativePlanProposalCard(conversation: conversation, item: item, target: !conversation.showingHistory && conversation.snapshot?.items.lastIndex(where: { $0.role == "plan" }) == index ? conversation.latestProposal : nil)
                } else {
                    selectableMessage(item, index: index)
                    if item.role == "user" {
                        let files = SavedAttachments.files(in: item.text)
                        ForEach(Array(files.filter { !NativeMessageImageRequest.isImageFile($0) }.enumerated()), id: \.offset) { _, file in
                            Text((file as NSString).lastPathComponent)
                                .font(WispDesign.font(size: 12))
                                .padding(.horizontal, 8).padding(.vertical, 4)
                                .background(color("bg-elev"), in: RoundedRectangle(cornerRadius: 8))
                                .accessibilityLabel("附件 \((file as NSString).lastPathComponent)")
                        }
                    }
                }
                if ["user", "assistant", "reasoning"].contains(item.role), !item.text.isEmpty {
                    HStack(spacing: 14) {
                    NativeMessageActions(source: item.role == "user" ? SavedAttachments.body(in: item.text) : item.text, quote: quoteSelection, save: { _ in
                        guard let projectID, let sessionID else { return }
                        let selection = NativeConversationModel.renderedText(item)
                        Task { await conversation.saveSelection(selection, project: projectID, session: sessionID) }
                    })
                    if let target = conversation.historyTarget(row: index, kind: "branch", checkpoint: item.role == "user" ? "before_user" : "after_response"), openHistoryBranch != nil {
                        Menu {
                            Button(localized(target.title)) { historyAction = target }.disabled(!conversation.canHistoryAction(target))
                            if item.role == "user", let rewind = conversation.historyTarget(row: index, kind: "rewind") {
                                Button(localized("回退到这条消息")) { historyAction = rewind }.disabled(!conversation.canHistoryAction(rewind))
                            }
                            if item.role == "assistant", let undo = conversation.historyTarget(row: index, kind: "undo") {
                                Button(localized("撤销本轮")) { undoAction = undo }.disabled(!conversation.canHistoryAction(undo))
                            }
                        } label: { WispIcon(name: "more", size: 14) }
                            .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
                            .accessibilityLabel(localized("历史消息操作"))
                    }
                    }
                }
            }.frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, item.role == "user" ? 12 : 4).padding(.vertical, item.role == "user" ? 8 : 4)
                .background(item.role == "user" ? color("bg-sunken") : .clear, in: RoundedRectangle(cornerRadius: 12))
        }
    }
    private func selectableMessage(_ item: ConversationItem, index: Int, input: Bool = false) -> some View {
        let source = item.role == "user" ? SavedAttachments.body(in: item.text) : item.text
        let attachments = item.role == "user" ? NativeMessageImageRequest.attachmentMarkdown(SavedAttachments.files(in: item.text)) : ""
        let markdown = item.role == "tool" || input ? nil : source + (attachments.isEmpty ? "" : "\n\n" + attachments)
        return NativeMessageBody(text: markedText(item, index: index, input: input), markdown: markdown, resources: item.resources ?? [], saved: conversation.savedHighlights.map(\.code),
                          revealed: conversation.scrollTarget == index ? conversation.revealedExcerpt : nil, monospaced: item.role == "tool",
                          client: conversation.client, project: projectID ?? "", session: sessionID ?? "", quote: quoteSelection, save: { selection in
            guard let projectID, let sessionID else { return }
            Task { await conversation.saveSelection(selection, project: projectID, session: sessionID) }
        }).id((projectID ?? "") + ":" + (sessionID ?? "") + ":" + String(index) + (input ? ":input" : ":body"))
    }
    private var composer: some View {
        VStack(spacing: 8) {
            if conversation.snapshot?.read_only == true {
                Text("该会话已归档或冻结，请新建会话继续。").font(WispDesign.font(size: 12)).foregroundStyle(color("text-muted"))
            }
            VStack(alignment: .leading, spacing: 8) {
                if !conversation.restrictedAssistant {
                NativeComposerModes(conversation: conversation)
                NativeAcpSettings(conversation: conversation)
                NativeComposerEnvironment(model: conversation.composer, writable: composerWritable, openRuntime: {
                    runtimeActivity = .init(context: conversation.composer.contextID, runtimes: true)
                }, openOptions: projectID != nil && sessionID != nil ? { composerOptions = true } : nil)
                }
                if !conversation.queuedTurns.isEmpty, let sessionID {
                    NativeConversationQueueView(conversation: conversation, session: sessionID).id((projectID ?? "") + ":" + sessionID)
                } else if let queued = conversation.queuedFollowUp {
                    Text("已排队一条后续：\(queued)").font(WispDesign.font(size: 12)).foregroundStyle(color("text-muted"))
                        .accessibilityIdentifier("queued-follow-up")
                }
                if !conversation.attachments.isEmpty {
                    ForEach(conversation.attachments) { file in
                        HStack {
                            Text(file.name).lineLimit(1)
                            Spacer()
                            Button("移除") { conversation.removeAttachment(file.path) }
                                .accessibilityLabel("移除 \(file.name)")
                        }.font(WispDesign.font(size: 12))
                    }
                }
                if !conversation.references.isEmpty {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 6) {
                            ForEach(conversation.references) { reference in
                                HStack(spacing: 6) {
                                    Text(reference.label).lineLimit(1)
                                    Button { conversation.removeReference(reference.id) } label: { WispIcon(name: "close", size: 12) }
                                        .buttonStyle(.plain).accessibilityLabel("移除引用 " + reference.label)
                                }.font(WispDesign.font(size: 12)).padding(.horizontal, 8).padding(.vertical, 5)
                                    .background(color("bg-sunken"), in: RoundedRectangle(cornerRadius: 6)).help(reference.detail)
                            }
                        }
                    }
                }
                NativeInlineComposerInput(conversation: conversation, completions: conversation.completions, sendWithModifier: sendWithModifier,
                                          commands: completionCommands, executeCommand: { executeComposerCommand?($0, $1) }, submit: submitComposer)
                    .id((projectID ?? "") + ":" + (sessionID ?? ""))
                    .zIndex(10)
                HStack {
                    if !conversation.restrictedAssistant {
                    Button { attachFiles() } label: { WispIcon(name: "plus", size: 17).frame(width: 32, height: 32)
                        .background(color("bg-elev"), in: Circle()).overlay(Circle().strokeBorder(color("border-strong"))) }
                        .buttonStyle(.plain).help("添加到消息").accessibilityLabel("对话附件")
                        .disabled(!conversation.canAttach)
                        .accessibilityIdentifier("composer-attach")
                    Button { conversation.completions.dismiss(); referencePicker = true } label: { WispIcon(name: "link", size: 16).frame(width: 32, height: 32) }
                        .buttonStyle(.plain).disabled(!conversation.canReference).help("添加产物、会话、环境或技能引用").accessibilityLabel("添加引用")
                    }
                    if !conversation.restrictedAssistant {
                        Button { timerPresented = true } label: { WispIcon(name: "clock", size: 16).frame(width: 32, height: 32) }
                            .buttonStyle(.plain).help(localized("会话定时器")).accessibilityLabel(localized("会话定时器")).disabled(conversation.snapshot?.read_only != false)
                    }
                    Spacer(minLength: 8)
                    Menu {
                        ForEach(Array(conversation.models.enumerated()), id: \.offset) { _, profile in
                            Button(profile["label"].string.isEmpty ? profile["model"].string : profile["label"].string) { Task { await conversation.selectModel(profile["id"].string) } }
                                .disabled(conversation.isAcp)
                        }
                        if let createAcpConversation, !conversation.acpAgents.isEmpty {
                            Divider()
                            Section("ACP · 新会话") {
                                ForEach(Array(conversation.acpAgents.enumerated()), id: \.offset) { _, agent in
                                    Button(agent["label"].string) { createAcpConversation(agent["id"].string) }
                                }
                            }
                        }
                    } label: {
                        Text(conversation.modelLabel).lineLimit(1)
                    }.menuStyle(.borderlessButton).padding(.horizontal, 10).frame(height: 32)
                        .background(color("bg-elev"), in: Capsule()).overlay(Capsule().strokeBorder(color("border")))
                        .frame(maxWidth: 180).disabled(conversation.busy || conversation.snapshot == nil || conversation.snapshot?.running == true || conversation.snapshot?.read_only == true)
                    if !conversation.restrictedAssistant { NativeComposerEffort(model: conversation.composer, enabled: composerWritable, acp: conversation.isAcp) }
                    if conversation.snapshot?.running == true {
                        Button(conversation.canRunComposerCommand(available: completionCommands) ? "执行命令" : "排队后续", action: queueComposer)
                            .disabled(!conversation.canQueueFollowUp && !conversation.canRunComposerCommand(available: completionCommands))
                            .accessibilityIdentifier("composer-queue")
                        Button(conversation.snapshot?.stopping == true ? "正在停止…" : "停止") { Task { await conversation.stop() } }.buttonStyle(WispButtonStyle(height: 32)).disabled(conversation.busy)
                    } else {
                        Button("发送", action: submitComposer).buttonStyle(WispButtonStyle(primary: true, height: 32)).disabled(!conversation.canSend)
                    }
                }
            }.padding(12).background(color("bg-elev"), in: RoundedRectangle(cornerRadius: 16))
                .overlay(RoundedRectangle(cornerRadius: 16).strokeBorder(color("border")))
            HStack {
                Text(localized(conversation.snapshot?.running == true ? (sendWithModifier ? "⌘Enter 排队 · Enter 换行" : "Enter 排队 · Shift+Enter 换行") : (sendWithModifier ? "⌘Enter 发送 · Enter 换行" : "Enter 发送 · Shift+Enter 换行"))).font(WispDesign.font(size: 11)).foregroundStyle(color("text-faint"))
                Spacer()
                if conversation.snapshot?.context_view == true {
                    Button { modelContext = true } label: { WispIcon(name: "gauge", size: 12) }
                        .buttonStyle(.plain).disabled(!conversation.canReadContext).help(localized("模型上下文"))
                        .accessibilityLabel(localized("模型上下文"))
                }
                Toggle("跟随最新回复", isOn: $followLatest).toggleStyle(.checkbox).font(WispDesign.font(size: 11))
            }
        }.frame(maxWidth: 850).padding(.horizontal, 16).padding(.bottom, 12)
    }
    private var composerWritable: Bool { conversation.snapshot?.read_only == false && conversation.snapshot?.running == false && !conversation.busy && !conversation.showingHistory && conversation.connectionError == nil }
    private var completionCommands: [NativeComposerCommand] {
        guard executeComposerCommand != nil, conversation.canAttach, conversation.snapshot?.read_only == false,
              conversation.connectionError == nil, !conversation.uncertainSend else { return [] }
        return NativeComposerCommand.allCases.filter { command in
            switch command {
            case .archive: return conversation.snapshot?.running == false && !conversation.visibleItems.isEmpty
            case .share: return conversation.visibleItems.contains { ["user", "assistant", "thinking"].contains($0.role) }
            default: return true
            }
        }
    }
    private func submitComposer() {
        if let executeComposerCommand, conversation.runComposerCommand(available: completionCommands, execute: executeComposerCommand) { return }
        conversation.completions.dismiss()
        Task {
            if conversation.snapshot?.running == true { await conversation.queueFollowUp() }
            else { await conversation.send() }
        }
    }
    private func queueComposer() {
        if let executeComposerCommand, conversation.runComposerCommand(available: completionCommands, execute: executeComposerCommand) { return }
        conversation.completions.dismiss()
        Task { await conversation.queueFollowUp() }
    }
    private func attachFiles() {
        conversation.completions.dismiss()
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = true
        panel.prompt = "添加"
        guard panel.runModal() == .OK else { return }
        let paths = panel.urls.map(\.path)
        Task {
            for path in paths {
                await conversation.attach(source: path, client: conversation.client)
            }
        }
    }
}
