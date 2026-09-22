import AppKit
import SwiftUI
import WispProjectBrowser

struct NativeConversationView: View {
    @ObservedObject var conversation: NativeConversationModel
    var projectID: String?
    var sessionID: String?
    var quoteSelection: (String) -> Void = { _ in }
    @Environment(\.colorScheme) private var scheme
    @State private var confirmResend = false
    @State private var followLatest = true
    @State private var expandedTools: Set<Int> = []
    private func color(_ token: String) -> Color { WispDesign.color(token, scheme) }
    var body: some View {
        VStack(spacing: 0) {
            if let error = conversation.connectionError ?? conversation.operationError ?? conversation.snapshot?.error {
                HStack(alignment: .top) {
                    Text(error).font(WispDesign.font(size: 12)).textSelection(.enabled)
                    Spacer()
                    Button("重新读取") { Task { await conversation.refresh() } }
                    if conversation.uncertainSend { Button("已检查，允许再次发送…") { confirmResend = true } }
                }.padding(12).foregroundStyle(.orange)
            }
            ScrollViewReader { scroll in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 20) {
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
                    }.frame(maxWidth: 800).padding(24).frame(maxWidth: .infinity)
                }
                .onChange(of: conversation.scrollRevision) { _ in
                    if let target = conversation.scrollTarget { expandedTools.insert(target); followLatest = false; scroll.scrollTo(target, anchor: .center) }
                }
                .onChange(of: conversation.snapshot?.sequence) { _ in
                    if followLatest && !conversation.showingHistory { scroll.scrollTo("latest", anchor: .bottom) }
                }
            }
            if !conversation.showingHistory {
                ForEach(conversation.snapshot?.approvals ?? []) { approval in
                    VStack(alignment: .leading, spacing: 10) {
                        Text("需要确认 · \(approval.tool)").font(WispDesign.font(size: 13, weight: .semibold))
                        Text(approval.message).font(WispDesign.font(size: 13)).textSelection(.enabled)
                        if !approval.preview.isEmpty {
                            ViewThatFits(in: .vertical) {
                                Text(approval.preview).fixedSize(horizontal: false, vertical: true)
                                ScrollView { Text(approval.preview).frame(maxWidth: .infinity, alignment: .leading) }.frame(height: 130)
                            }.font(WispDesign.font(size: 12, design: .monospaced)).textSelection(.enabled)
                                .frame(maxWidth: .infinity, maxHeight: 130, alignment: .leading)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        HStack {
                            Spacer()
                            Button("拒绝") { Task { await conversation.approve(approval, allowed: false) } }
                            Button("允许这一次") { Task { await conversation.approve(approval, allowed: true) } }.buttonStyle(WispButtonStyle(primary: true))
                        }.disabled(conversation.busy || conversation.connectionError != nil)
                    }.padding(16).background(color("bg-elev"), in: RoundedRectangle(cornerRadius: 12))
                        .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(color("clay")))
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
        .onChange(of: sessionID) { _ in expandedTools = [] }
        .onChange(of: conversation.showingHistory) { _ in expandedTools = [] }
        .confirmationDialog("先核对最新消息，避免重复执行同一个任务。确认仍需再次发送？", isPresented: $confirmResend) {
            Button("保留草稿，允许再次发送") { conversation.acknowledgeUncertainSend() }
            Button("取消", role: .cancel) {}
        }
    }
    private func markedText(_ item: ConversationItem, index: Int, input: Bool = false) -> AttributedString {
        let source = item.role == "user" && !input ? SavedAttachments.body(in: item.text) : item.text
        var text = input ? AttributedString(item.input ?? "") : item.role == "tool" ? AttributedString(item.text) : ((try? AttributedString(markdown: source, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(source))
        if conversation.scrollTarget == index, let excerpt = conversation.revealedExcerpt,
           let range = NativeSavedExcerpt.range(in: String(text.characters), excerpt: excerpt) {
            let start = text.characters.index(text.startIndex, offsetBy: range.lowerBound)
            let end = text.characters.index(text.startIndex, offsetBy: range.upperBound)
            text[start..<end].backgroundColor = .yellow.opacity(0.4)
        }
        return text
    }
    private func message(_ item: ConversationItem, index: Int) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(item.role == "user" ? "你" : item.role == "tool" ? (item.tool_name ?? "工具") : item.role == "reasoning" ? "思考" : "Wisp Science")
                    .font(WispDesign.font(size: 12, weight: .semibold)).foregroundStyle(color("text-muted"))
                if let ok = item.ok { Text(ok ? "已完成" : "失败").font(WispDesign.font(size: 11)).foregroundStyle(ok ? color("clay") : .orange) }
                if let status = item.status { Text(status).font(.caption).foregroundStyle(.secondary) }
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
            } else if item.role == "question", let data = item.text.data(using: .utf8), let question = try? JSONDecoder().decode(SettingsValue.self, from: data) {
                Text(question["question"].string).font(WispDesign.font(size: 14)).textSelection(.enabled)
                ForEach(Array(question["options"].array.enumerated()), id: \.offset) { _, option in
                    Button { conversation.draft = option["label"].string } label: {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(option["label"].string)
                            if !option["description"].string.isEmpty { Text(option["description"].string).font(.caption).foregroundStyle(.secondary) }
                        }.frame(maxWidth: .infinity, alignment: .leading)
                    }.disabled(conversation.showingHistory || conversation.snapshot?.read_only == true)
                }
                Text("选择选项会填入输入框，点击发送后继续。").font(WispDesign.font(size: 11)).foregroundStyle(.secondary)
            } else {
                selectableMessage(item, index: index)
                if item.role == "user" {
                    let files = SavedAttachments.files(in: item.text)
                    ForEach(Array(files.enumerated()), id: \.offset) { _, file in
                        Text((file as NSString).lastPathComponent)
                            .font(WispDesign.font(size: 12))
                            .padding(.horizontal, 8).padding(.vertical, 4)
                            .background(color("bg-elev"), in: RoundedRectangle(cornerRadius: 8))
                            .accessibilityLabel("附件 \((file as NSString).lastPathComponent)")
                    }
                }
            }
        }.frame(maxWidth: .infinity, alignment: .leading).padding(16)
            .background(item.role == "user" ? color("bg-sunken") : .clear, in: RoundedRectangle(cornerRadius: 12))
    }
    private func selectableMessage(_ item: ConversationItem, index: Int, input: Bool = false) -> some View {
        NativeSelectableMessage(text: markedText(item, index: index, input: input), saved: conversation.savedHighlights.map(\.code), quote: quoteSelection, save: { selection in
            guard let projectID, let sessionID else { return }
            Task { await conversation.saveSelection(selection, project: projectID, session: sessionID) }
        }, monospaced: item.role == "tool").frame(maxWidth: .infinity, alignment: .leading)
    }
    private var composer: some View {
        VStack(spacing: 8) {
            if conversation.snapshot?.read_only == true {
                Text("该会话为只读（已归档、冻结或使用 ACP），请在 WebView 中继续，或新建原生会话。").font(WispDesign.font(size: 12)).foregroundStyle(color("text-muted"))
            }
            VStack(alignment: .leading, spacing: 12) {
                if let queued = conversation.queuedFollowUp {
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
                TextEditor(text: $conversation.draft).font(WispDesign.font(size: 14)).frame(minHeight: 58, maxHeight: 110)
                    .scrollContentBackground(.hidden).accessibilityLabel("消息输入框")
                    .disabled(conversation.snapshot?.read_only == true || conversation.showingHistory)
                HStack {
                    Button("对话附件") { attachFiles() }
                        .disabled(!conversation.canAttach)
                        .accessibilityIdentifier("composer-attach")
                    Menu {
                        ForEach(Array(conversation.models.enumerated()), id: \.offset) { _, profile in
                            Button(profile["label"].string.isEmpty ? profile["model"].string : profile["label"].string) { Task { await conversation.selectModel(profile["id"].string) } }
                        }
                    } label: {
                        Text(conversation.models.first(where: { $0["id"].string == conversation.snapshot?.model_id })?["label"].string ?? "选择模型").lineLimit(1)
                    }.frame(maxWidth: 230).disabled(conversation.busy || conversation.snapshot == nil || conversation.snapshot?.running == true || conversation.snapshot?.read_only == true)
                    Spacer()
                    if conversation.snapshot?.running == true {
                        Button("排队后续") { Task { await conversation.queueFollowUp() } }
                            .disabled(!conversation.canQueueFollowUp)
                            .accessibilityIdentifier("composer-queue")
                        Button(conversation.snapshot?.stopping == true ? "正在停止…" : "停止") { Task { await conversation.stop() } }.buttonStyle(WispButtonStyle()).disabled(conversation.busy)
                    } else {
                        Button("发送") { Task { await conversation.send() } }.buttonStyle(WispButtonStyle(primary: true)).disabled(!conversation.canSend).keyboardShortcut(.return, modifiers: .command)
                    }
                }
            }.padding(16).background(color("bg-elev"), in: RoundedRectangle(cornerRadius: 14))
                .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(color("border")))
            HStack {
                Text("⌘Enter 发送 · Enter 换行").font(WispDesign.font(size: 11)).foregroundStyle(color("text-faint"))
                Spacer()
                Toggle("跟随最新回复", isOn: $followLatest).toggleStyle(.checkbox).font(WispDesign.font(size: 11))
            }
        }.frame(maxWidth: 850).padding(.horizontal, 24).padding(.bottom, 16)
    }
    private func attachFiles() {
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
