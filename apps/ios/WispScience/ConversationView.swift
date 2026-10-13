import SwiftUI
import PhotosUI
import WispRemote

struct ConversationView: View {
    @ObservedObject var model: RemoteWorkspace
    @ObservedObject var connection: RemoteConnection
    let route: ConversationRoute
    @State private var chosenPhotos: [PhotosPickerItem] = []
    @State private var uploading = false
    @State private var followLatest = true
    @State private var initialScroll = true
    @FocusState private var composing: Bool
    @Environment(\.dynamicTypeSize) private var typeSize
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var snapshot: RemoteSnapshot? { model.snapshot }
    private var draft: Binding<String> {
        Binding(get: { model.drafts[route] ?? "" }, set: { model.drafts[route] = $0 })
    }
    private var blocked: Bool {
        !model.canActOnSnapshot || snapshot?.writable != true || model.busy.contains(route) || uploading
    }
    var body: some View {
        GeometryReader { viewport in
            ScrollViewReader { reader in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 20) {
                        if let date = model.snapshotSavedAt, model.snapshotIsCached || connection.state != .online {
                            Text("缓存内容 · \(date.formatted(date: .abbreviated, time: .shortened)) · 联网刷新后可操作")
                                .font(.caption).foregroundStyle(.secondary).accessibilityIdentifier("cached-conversation")
                        }
                        if model.history.cursor != .null {
                            Button("加载更早的消息") { Task { await model.earlier() } }
                                .disabled(model.loadingEarlier).frame(maxWidth: .infinity)
                        }
                        ForEach(RemoteTranscript.groups(model.history.older + (snapshot?.items ?? []),
                            running: snapshot?.running == true, indexOffset: -model.history.older.count)) { group in
                            TranscriptGroupView(group: group, model: model, route: route)
                        }
                        .id("\(snapshot?.epoch ?? ""):\(snapshot?.before.display ?? "")")
                        if let runs = snapshot?.value["run_cards"].array, !runs.isEmpty {
                            RunHistoryView(runs: runs)
                        }
                        ForEach(snapshot?.value["approvals"].array ?? [], id: \.approvalID) { approval in
                            approvalCard(approval)
                        }
                        if let snapshot {
                            if snapshot.running && model.canActOnSnapshot {
                                HStack { ProgressView(); Text(snapshot.value["activity_status"].string.isEmpty ? "运行中…" : snapshot.value["activity_status"].string) }
                                    .font(.callout).foregroundStyle(.secondary)
                            } else if !snapshot.value["error"].string.isEmpty {
                                Text(snapshot.value["error"].string).foregroundStyle(.red)
                            }
                            if snapshot.writable && !snapshot.value["follow_ups"].array.isEmpty {
                                VStack(alignment: .leading, spacing: 8) {
                                    Text("继续探索").font(.caption.weight(.medium)).foregroundStyle(.secondary)
                                    ForEach(Array(snapshot.value["follow_ups"].array.enumerated()), id: \.offset) { index, suggestion in
                                        Button { model.drafts[route] = suggestion.string } label: {
                                            Text(suggestion.string)
                                                .font(.subheadline).foregroundStyle(.primary)
                                                .multilineTextAlignment(.leading).fixedSize(horizontal: false, vertical: true)
                                                .frame(maxWidth: .infinity, alignment: .leading)
                                                .padding(.horizontal, 12).padding(.vertical, 11)
                                                .frame(minHeight: 44).contentShape(Rectangle())
                                        }
                                        .buttonStyle(WispRowButtonStyle())
                                        .background(WispStyle.canvas, in: RoundedRectangle(cornerRadius: 10))
                                        .clipShape(RoundedRectangle(cornerRadius: 10))
                                        .overlay { RoundedRectangle(cornerRadius: 10).strokeBorder(WispStyle.border, lineWidth: 1) }
                                        .accessibilityIdentifier("follow-up-\(index)")
                                        .accessibilityHint("填入输入框，确认后再发送")
                                    }
                                }
                            }
                            ForEach(snapshot.value["queue"]["items"].array, id: \.recordID) { item in
                                HStack {
                                    VStack(alignment: .leading) {
                                        Text(item["message"].string).lineLimit(3)
                                        StatusBadge(status: item["state"].string)
                                    }
                                    Spacer()
                                    if item["state"].string == "queued", connection.supports("native_conversation_queue_action") {
                                        Button("取消排队") {
                                            Task { await model.action("native_conversation_queue_action", route: route, extra: [
                                                "id": item["id"], "digest": item["digest"], "action": .object(["kind": .string("cancel")]),
                                            ]) }
                                        }.disabled(blocked)
                                    }
                                }.padding(12).background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 12))
                            }
                        } else if connection.state != .online {
                            Text("这个对话尚未缓存，请联网后打开。").foregroundStyle(.secondary)
                        } else { ProgressView("读取对话…").frame(maxWidth: .infinity) }
                        Color.clear.frame(height: 1).id("latest")
                            .background {
                                GeometryReader { position in
                                    Color.clear.preference(key: TranscriptBottomKey.self,
                                        value: position.frame(in: .named("conversation-scroll")).maxY)
                                }
                            }
                    }
                    .padding(.horizontal, 18).padding(.top, 16).padding(.bottom, 8)
                    .frame(maxWidth: 780).frame(maxWidth: .infinity)
                }
                .coordinateSpace(name: "conversation-scroll")
                .onPreferenceChange(TranscriptBottomKey.self) { bottom in
                    // Lazy rows can be created before they reach the visible viewport.
                    // Geometry, rather than onAppear, determines whether to follow updates.
                    followLatest = bottom.map { $0 <= viewport.size.height + 44 } ?? false
                }
                .accessibilityIdentifier("conversation-transcript")
                .scrollDismissesKeyboard(.interactively)
                .defaultScrollAnchor(.bottom)
                .refreshable { await model.refreshSelection() }
                .onChange(of: snapshot) { _, next in
                    if next != nil && (initialScroll || followLatest) { reader.scrollTo("latest", anchor: .bottom); initialScroll = false }
                }
                .overlay(alignment: .bottom) {
                    if !followLatest && snapshot != nil {
                        HStack {
                            Spacer()
                            Button {
                                withAnimation(reduceMotion ? nil : .easeOut(duration: 0.2)) {
                                    reader.scrollTo("latest", anchor: .bottom)
                                }
                            } label: {
                                HStack(spacing: 6) {
                                    Text("回到最新").font(.subheadline.weight(.medium))
                                    WispIcon(.chevronDown, size: 16)
                                }.padding(.horizontal, 14).frame(minHeight: 44)
                            }
                            .buttonStyle(.plain).foregroundStyle(WispStyle.accent)
                            .background(.regularMaterial, in: Capsule())
                            .overlay { Capsule().strokeBorder(WispStyle.border, lineWidth: 1) }
                            .accessibilityIdentifier("jump-to-latest")
                        }.padding(.horizontal, 18).padding(.bottom, 10).frame(maxWidth: 780)
                    }
                }
            }
        }
        .background(WispStyle.canvas)
        .navigationTitle(model.title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(WispStyle.canvas, for: .navigationBar)
        .toolbarBackground(.visible, for: .navigationBar)
        .toolbar {
            ToolbarItem(placement: .principal) {
                Button {
                    model.sheet = .conversationInfo(model.title,
                        model.projects.first { $0.recordID == route.project }?["name"].string ?? "")
                } label: {
                    HStack(spacing: 4) {
                        Text(model.title.isEmpty ? "未命名对话" : model.title)
                            .font(.headline).lineLimit(1).truncationMode(.middle)
                        WispIcon(.chevronDown, size: 12).foregroundStyle(.secondary)
                    }.foregroundStyle(.primary).frame(minHeight: 44)
                }.buttonStyle(.plain)
                    .accessibilityIdentifier("conversation-info")
                    .accessibilityLabel("对话详情：\(model.title)")
                    .accessibilityHint("查看完整标题和所属项目")
            }
            ToolbarItem(placement: .topBarTrailing) {
                Button("文件") { model.sheet = .files(route, ".", true) }
                .disabled(connection.state == .online && !connection.supports("native_conversation_panel_file_directory"))
                .accessibilityIdentifier("conversation-files")
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) { composer }
        .onAppear { model.visibleRoute = route }
        .onDisappear { if model.visibleRoute == route { model.visibleRoute = nil } }
        .accessibilityIdentifier("conversation")
    }

    private var composer: some View {
        VStack(alignment: .leading, spacing: 8) {
            VStack(alignment: .leading, spacing: 4) {
                if !(model.photos[route] ?? []).isEmpty {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 10) {
                            ForEach(model.photos[route] ?? []) { photo in
                                VStack(spacing: 4) {
                                    if let image = UIImage(data: photo.data) {
                                        Image(uiImage: image).resizable().scaledToFill().frame(width: 64, height: 64).clipped().clipShape(RoundedRectangle(cornerRadius: 8))
                                    }
                                    Button("移除") { model.photos[route]?.removeAll { $0.id == photo.id } }
                                        .font(.caption).frame(minWidth: 44, minHeight: 44).buttonStyle(.plain)
                                        .disabled(model.busy.contains(route) || uploading)
                                }
                            }
                        }
                    }.padding(.horizontal, 12).padding(.top, 12)
                }
                TextField(model.isDemo ? "试试询问演示报告…" : "给 Wisp 发消息…", text: draft, axis: .vertical).lineLimit(1...(typeSize.isAccessibilitySize ? 3 : 6))
                    .font(.body).foregroundStyle(.primary).focused($composing)
                    .padding(.horizontal, 14).padding(.top, 12).padding(.bottom, 6)
                    .disabled(snapshot?.writable != true).accessibilityIdentifier("message-draft")
                composerActions.padding(.horizontal, 8).padding(.bottom, 8)
            }
            .background(WispStyle.surface, in: RoundedRectangle(cornerRadius: 18))
            .overlay {
                RoundedRectangle(cornerRadius: 18)
                    .strokeBorder(composing ? WispStyle.accent.opacity(0.6) : WispStyle.border, lineWidth: 1)
            }
            Text(model.isDemo ? "本地预设回复 · 不调用 AI，不执行分析" :
                    snapshot?.value["read_only"].bool == true ? "这个对话是只读的。" :
                    !(snapshot?.value["acp_agent_id"].string ?? "").isEmpty ? "ACP 会话请在桌面端继续。" :
                    "写文件和执行命令前，会请你批准")
                .font(.caption2).foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .center)
        }
        .padding(.horizontal, 14).padding(.top, 10).padding(.bottom, 6)
        .frame(maxWidth: 780).frame(maxWidth: .infinity).background(WispStyle.canvas)
        .overlay(alignment: .top) { Rectangle().fill(WispStyle.border).frame(height: 0.5) }
        .onChange(of: chosenPhotos) { _, selection in
            guard !selection.isEmpty, !uploading else { return }
            uploading = true
            model.busy.insert(route)
            Task {
                defer { uploading = false; model.busy.remove(route); chosenPhotos = [] }
                for item in selection {
                    do {
                        guard let data = try await item.loadTransferable(type: Data.self) else { continue }
                        await model.upload(data, route: route)
                    } catch { model.error = "无法读取所选图片，请重试。" }
                }
            }
        }
    }

    private var composerActions: some View {
        let layout = typeSize.isAccessibilitySize
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: 4))
            : AnyLayout(HStackLayout(spacing: 8))
        return layout {
            if connection.supports("remote_attach_image") {
                PhotosPicker(selection: $chosenPhotos, maxSelectionCount: max(1, 4 - (model.photos[route]?.count ?? 0)), matching: .images) {
                    HStack(spacing: 6) {
                        if uploading { ProgressView().controlSize(.small) }
                        else { WispIcon(.image, size: 18) }
                        Text(uploading ? "上传中…" : "图片").font(.subheadline)
                    }.foregroundStyle(.secondary).padding(.horizontal, 6).frame(minHeight: 44)
                }
                .buttonStyle(WispRowButtonStyle())
                .accessibilityLabel(uploading ? "正在上传图片" : "添加图片")
                .disabled(blocked || (model.photos[route]?.count ?? 0) >= 4)
            }
            HStack(spacing: 8) {
                Spacer(minLength: 4)
                if snapshot?.running == true && connection.supports("native_conversation_stop") {
                    Button("停止", role: .destructive) { Task { await model.action("native_conversation_stop", route: route) } }
                        .font(.subheadline).foregroundStyle(WispStyle.failure)
                        .frame(minWidth: 44, minHeight: 44).buttonStyle(WispRowButtonStyle())
                        .disabled(!model.canActOnSnapshot || model.busy.contains(route) || snapshot?.value["stopping"].bool == true)
                }
                Button { Task { await model.send(route) } } label: {
                    HStack(spacing: 5) {
                        Text(snapshot?.running == true ? "排队" : "发送")
                        WispIcon(.arrowUp, size: 17)
                    }
                }
                    .buttonStyle(WispSendButtonStyle()).keyboardShortcut(.return, modifiers: .command)
                    .disabled(blocked || ((model.drafts[route] ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && (model.photos[route] ?? []).isEmpty)
                              || !connection.supports(snapshot?.running == true ? "native_conversation_enqueue" : "native_conversation_send"))
                    .accessibilityIdentifier("send-message")
            }
        }
    }

    private func approvalCard(_ approval: RemoteValue) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("需要你批准：\(approval["tool"].string)").font(.subheadline.weight(.semibold)).foregroundStyle(WispStyle.warning)
            Text(approval["message"].string).font(.subheadline).textSelection(.enabled)
            CodeText(text: approval["preview"].string)
            HStack {
                Spacer()
                Button("拒绝", role: .destructive) { decide(false, approval) }
                    .font(.subheadline).foregroundStyle(WispStyle.failure)
                    .frame(minWidth: 60, minHeight: 44).buttonStyle(WispRowButtonStyle())
                Button("批准一次") { decide(true, approval) }.buttonStyle(WispSendButtonStyle())
            }.disabled(!model.canActOnSnapshot || model.busy.contains(route) || !connection.supports("native_conversation_approve"))
        }.padding(14).background(WispStyle.warning.opacity(0.06), in: RoundedRectangle(cornerRadius: 14))
            .overlay { RoundedRectangle(cornerRadius: 14).strokeBorder(WispStyle.warning.opacity(0.25), lineWidth: 1) }
    }
    private func decide(_ approved: Bool, _ approval: RemoteValue) {
        Task { await model.action("native_conversation_approve", route: route, extra: [
            "approval_id": approval["approval_id"], "approved": .bool(approved),
        ]) }
    }
}

private struct TranscriptGroupView: View {
    let group: RemoteTranscript.Group
    @ObservedObject var model: RemoteWorkspace
    let route: ConversationRoute
    @Environment(\.dynamicTypeSize) private var typeSize

    private var summary: String {
        var parts = ["执行过程"]
        if group.toolCount > 0 { parts.append("\(group.toolCount) 步") }
        if group.changedFileCount > 0 { parts.append("\(group.changedFileCount) 个文件") }
        return parts.joined(separator: " · ")
    }
    var body: some View {
        if group.isProcess {
            DisclosureGroup {
                ForEach(Array(group.items.enumerated()), id: \.offset) { offset, item in
                    MessageView(item: item, model: model, route: route, index: group.id + offset)
                }
            } label: {
                let layout = typeSize.isAccessibilitySize
                    ? AnyLayout(VStackLayout(alignment: .leading, spacing: 6))
                    : AnyLayout(HStackLayout(spacing: 8))
                layout {
                    Text(summary).font(.subheadline.weight(.medium)).foregroundStyle(.primary)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    if group.failureCount > 0 {
                        Text("\(group.failureCount) 项失败").font(.caption.weight(.medium))
                            .foregroundStyle(WispStyle.failure).fixedSize()
                    }
                }
            }.disclosureGroupStyle(WispDisclosureStyle(identifier: "process-group-\(group.id)"))
        } else {
            ForEach(Array(group.items.enumerated()), id: \.offset) { offset, item in
                MessageView(item: item, model: model, route: route, index: group.id + offset)
            }
        }
    }
}

struct MessageView: View {
    let item: RemoteValue
    @ObservedObject var model: RemoteWorkspace
    let route: ConversationRoute
    let index: Int
    var body: some View {
        switch item["role"].string {
        case "user":
            VStack(alignment: .leading, spacing: 8) {
                Text(item["text"].string).textSelection(.enabled)
                ForEach(Array(item["attachments"].array.enumerated()), id: \.offset) { _, attachment in
                    RemoteImageView(model: model, route: route, source: ["path": attachment], label: (attachment.string as NSString).lastPathComponent)
                }
            }.font(.body).foregroundStyle(.primary).multilineTextAlignment(.leading)
                .padding(14).background(WispStyle.surface, in: RoundedRectangle(cornerRadius: 16))
                .padding(.leading, 28)
                .frame(maxWidth: .infinity, alignment: .trailing)
        case "assistant":
            MarkdownView(text: item["text"].string, resources: item["resources"].array, model: model, route: route)
        case "usage":
            UsageDetailsView(text: item["text"].string)
        case "question":
            if let question = model.question(at: index, item: item) {
                QuestionCardView(question: question, item: item, index: index,
                    model: model, connection: model.connection, route: route)
                    .id(model.questionID(at: index, item: item, route: route))
            } else {
                Text(item["text"].string).foregroundStyle(.secondary).textSelection(.enabled)
            }
        case "tool", "acp_tool", "reasoning":
            DisclosureGroup {
                if !item["input"].string.isEmpty { CodeText(text: item["input"].string) }
                CodeText(text: item["text"].string)
                if item["tool_name"].string == "generate_image", item["ok"].bool,
                   let path = RemoteMarkdown.localPath(item["input"].string) {
                    RemoteImageView(model: model, route: route, source: ["path": .string(path)], label: "生成的图片")
                }
            } label: {
                HStack {
                    Text(item["role"].string == "reasoning" ? "思考" : item["tool_name"].string)
                    Spacer()
                    StatusBadge(status: item["ok"] == .bool(false) ? "failed" : item["status"].string)
                }.font(.subheadline).foregroundStyle(.primary)
            }.disclosureGroupStyle(WispDisclosureStyle())
        case "plan":
            VStack(alignment: .leading, spacing: 8) {
                ForEach(Array(item["proposal"]["entries"].array.enumerated()), id: \.offset) { index, entry in
                    Text("\(index + 1). \(entry["content"].string)")
                }
            }.foregroundStyle(.secondary)
        default:
            if !item["text"].string.isEmpty { Text(item["text"].string).foregroundStyle(.secondary).textSelection(.enabled) }
        }
    }
}

private struct QuestionCardView: View {
    let question: RemoteQuestion
    let item: RemoteValue
    let index: Int
    @ObservedObject var model: RemoteWorkspace
    @ObservedObject var connection: RemoteConnection
    let route: ConversationRoute
    @State private var selected: Int?
    @State private var note = ""
    @FocusState private var editingNote: Bool

    private var id: String { model.questionID(at: index, item: item, route: route) }
    private var submission: QuestionSubmission? { model.questionSubmissions[id] }
    private var answered: Bool { question.state == .answered || submission == .sent }
    private var enabled: Bool { model.canAnswerQuestion(at: index, item: item, route: route) }
    private var status: String {
        if answered { return "已回答" }
        if question.state == .expired { return "问题已过期" }
        if submission == .uncertain { return "发送结果待确认，请刷新对话或在主机查看。" }
        if !question.isNative || model.snapshot?.writable != true { return "请在桌面端回答此问题。" }
        if !model.canActOnSnapshot { return "联网刷新后可回答" }
        if model.busy.contains(route) { return "正在提交…" }
        if model.snapshot?.running == true { return "等待当前运行结束后回答" }
        if !connection.supports("native_conversation_send") { return "当前主机不支持回答，请更新主机。" }
        if question.options.isEmpty { return "填写回答后提交" }
        return question.allowsFreeform ? "选择一项后提交，也可以补充说明" : "选择一项后提交"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(answered ? "已回答的问题" : question.state == .expired ? "已过期的问题" : "需要你确认")
                .font(.caption.weight(.semibold)).foregroundStyle(WispStyle.accent)
            Text(question.question).font(.body.weight(.medium)).textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
            if !answered && question.state == .pending && submission == nil {
                VStack(spacing: 8) {
                    ForEach(question.options.indices, id: \.self) { option in
                        let choice = question.options[option]
                        Button { selected = selected == option ? nil : option } label: {
                            VStack(alignment: .leading, spacing: 5) {
                                HStack(alignment: .firstTextBaseline, spacing: 8) {
                                    Text(choice.label).font(.subheadline.weight(.semibold)).foregroundStyle(.primary)
                                        .frame(maxWidth: .infinity, alignment: .leading)
                                    Text(selected == option ? "已选" : "选择").font(.caption.weight(.medium))
                                        .foregroundStyle(WispStyle.accent).fixedSize()
                                }
                                if !choice.description.isEmpty {
                                    Text(choice.description).font(.caption).foregroundStyle(.secondary)
                                }
                            }
                            .multilineTextAlignment(.leading).fixedSize(horizontal: false, vertical: true)
                            .padding(12).frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                            .background(selected == option ? WispStyle.accent.opacity(0.09) : WispStyle.canvas,
                                        in: RoundedRectangle(cornerRadius: 10))
                            .overlay { RoundedRectangle(cornerRadius: 10).strokeBorder(
                                selected == option ? WispStyle.accent : WispStyle.border, lineWidth: selected == option ? 1.5 : 1) }
                            .contentShape(RoundedRectangle(cornerRadius: 10))
                        }
                        .buttonStyle(WispRowButtonStyle()).disabled(!enabled)
                        .accessibilityIdentifier("question-option-\(option)")
                        .accessibilityValue(selected == option ? "已选择" : "未选择")
                        .accessibilityHint("选择后点击提交回答")
                        .accessibilityAddTraits(selected == option ? .isSelected : [])
                    }
                }
                if question.allowsFreeform {
                    TextField("补充说明，或直接输入你的回答", text: $note, axis: .vertical)
                        .font(.subheadline).lineLimit(2...6).focused($editingNote)
                        .padding(12).background(WispStyle.canvas, in: RoundedRectangle(cornerRadius: 10))
                        .overlay { RoundedRectangle(cornerRadius: 10).strokeBorder(WispStyle.border, lineWidth: 1) }
                        .disabled(!enabled).accessibilityIdentifier("question-freeform")
                }
                HStack {
                    Spacer()
                    Button("提交回答") {
                        editingNote = false
                        let questionID = id
                        let option = selected, answerNote = note
                        Task { await model.answerQuestion(at: index, item: item, id: questionID,
                            route: route, option: option, freeform: answerNote) }
                    }
                    .buttonStyle(WispSendButtonStyle()).accessibilityIdentifier("question-submit")
                    .disabled(!enabled || question.answer(option: selected, freeform: note) == nil)
                }
            }
            Text(status).font(.caption).foregroundStyle(.secondary)
                .accessibilityIdentifier("question-status")
        }
        .padding(14).frame(maxWidth: .infinity, alignment: .leading)
        .background(WispStyle.surface, in: RoundedRectangle(cornerRadius: 14))
        .overlay { RoundedRectangle(cornerRadius: 14).strokeBorder(WispStyle.border, lineWidth: 1) }
        .accessibilityElement(children: .contain).accessibilityIdentifier("question-card")
    }
}

private struct TranscriptBottomKey: PreferenceKey {
    static var defaultValue: CGFloat? { nil }
    static func reduce(value: inout CGFloat?, nextValue: () -> CGFloat?) {
        if let next = nextValue() { value = next }
    }
}

private struct UsageDetailsView: View {
    let text: String
    private var usage: RemoteValue? {
        guard let value = try? JSONDecoder().decode(RemoteValue.self, from: Data(text.utf8)),
              case .object = value else { return nil }
        return value
    }
    private let fields = [("input", "输入"), ("output", "输出"), ("cached", "缓存"),
                          ("reasoning", "推理"), ("ctx_tokens", "上下文"), ("max_context", "上下文上限")]
    var body: some View {
        DisclosureGroup {
            if let usage {
                ForEach(fields, id: \.0) { key, label in
                    if let count = usage[key].uint {
                        LabeledContent(label, value: count.formatted()).font(.caption).monospacedDigit()
                            .accessibilityIdentifier("usage-\(key)")
                    }
                }
                DisclosureGroup {
                    CodeText(text: text)
                } label: {
                    Text("原始数据").font(.caption).foregroundStyle(.secondary)
                }.disclosureGroupStyle(WispDisclosureStyle(identifier: "usage-raw"))
            } else {
                CodeText(text: text)
            }
        } label: {
            Text("用量统计").font(.caption.weight(.medium)).foregroundStyle(.secondary)
        }.disclosureGroupStyle(WispDisclosureStyle(identifier: "usage-details"))
    }
}

private struct RunHistoryView: View {
    let runs: [RemoteValue]
    private var completed: [RemoteValue] { runs.filter { $0["status"].string == "succeeded" } }
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("运行记录 · \(runs.count)").font(.caption.weight(.medium)).foregroundStyle(.secondary)
            // Failures, active runs and unknown statuses remain visible. Only
            // confirmed successes are tucked away; refreshes never reset expansion.
            ForEach(runs.filter { $0["status"].string != "succeeded" }, id: \.recordID) { run in
                RunCardView(run: run)
            }
            if !completed.isEmpty {
                DisclosureGroup {
                    ForEach(completed, id: \.recordID) { run in RunCardView(run: run) }
                } label: {
                    Text("已完成的记录（\(completed.count)）")
                        .font(.subheadline).foregroundStyle(.secondary)
                }.disclosureGroupStyle(WispDisclosureStyle(identifier: "completed-runs"))
            }
        }
    }
}

struct RunCardView: View {
    let run: RemoteValue
    @Environment(\.dynamicTypeSize) private var typeSize
    private var output: String {
        [run["stdout_tail"].string, run["stderr_tail"].string, run["last_poll_error"].string]
            .filter { !$0.isEmpty }.joined(separator: "\n\n")
    }
    var body: some View {
        DisclosureGroup {
            if !run["command"].string.isEmpty { CodeText(text: "$ " + run["command"].string) }
            if !output.isEmpty { CodeText(text: output) }
            else { Text("暂无输出").font(.caption).foregroundStyle(.secondary) }
        } label: {
            let layout = typeSize.isAccessibilitySize
                ? AnyLayout(VStackLayout(alignment: .leading, spacing: 6))
                : AnyLayout(HStackLayout(alignment: .center, spacing: 10))
            layout {
                Text(run["title"].string.isEmpty ? run["kind"].string : run["title"].string)
                    .font(.subheadline.weight(.medium)).foregroundStyle(.primary)
                    .multilineTextAlignment(.leading).fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                StatusBadge(status: run["status"].string).fixedSize()
            }
        }.disclosureGroupStyle(WispDisclosureStyle(identifier: "run-\(run.recordID)"))
    }
}
