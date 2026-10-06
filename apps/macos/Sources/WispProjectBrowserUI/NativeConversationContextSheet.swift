import SwiftUI
import WispProjectBrowser

struct NativeConversationContextSheet: View {
    @ObservedObject var conversation: NativeConversationModel
    let project: String
    let session: String
    let close: () -> Void
    @State private var context: NativeConversationContext?
    @State private var loading = false
    @State private var error: String?
    @State private var tab = "messages"
    @State private var compacting = false
    @State private var undoEpoch: NativeContextCompaction?
    @State private var closed = false
    @State private var readGeneration = UUID()
    private let loadOnAppear: Bool
    init(conversation: NativeConversationModel, project: String, session: String, context: NativeConversationContext? = nil, tab: String = "messages", close: @escaping () -> Void) {
        self.conversation = conversation; self.project = project; self.session = session; self.close = close
        _context = State(initialValue: context); loadOnAppear = context == nil
        _tab = State(initialValue: tab)
    }
    private var writable: Bool { !closed && conversation.canCompact && conversation.snapshot?.project_id == project && conversation.snapshot?.session_id == session }
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text(localized("模型上下文")).font(.headline)
                if let context { Text("Epoch \(context.state.head_epoch)").font(.caption).foregroundStyle(.secondary) }
                Spacer()
                Button(localized("关闭"), action: dismiss).disabled(conversation.busy)
            }
            Text(localized("查看模型当前看到的工作集、系统提示和工具说明。压缩只改变模型上下文，完整对话记录会保留。"))
                .font(.caption).foregroundStyle(.secondary)
            Picker(localized("上下文内容"), selection: $tab) {
                Text(localized("工作集")).tag("messages")
                Text(localized("系统与工具")).tag("details")
                Text(localized("压缩记录")).tag("compactions")
            }.pickerStyle(.segmented).labelsHidden().frame(height: 28).accessibilityLabel(localized("上下文内容"))
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 16) {
                    if loading { ProgressView(localized("正在读取上下文…")) }
                    if let context {
                        if tab == "messages" {
                            if context.items.isEmpty { Text(localized("模型工作集为空。")) }
                            ForEach(Array(context.items.enumerated()), id: \.offset) { _, item in
                                VStack(alignment: .leading, spacing: 6) {
                                    Text(item.tool_name ?? item.role).font(.caption).bold().foregroundStyle(.secondary)
                                    if let input = item.input { Text(input).font(.system(size: 12, design: .monospaced)).textSelection(.enabled) }
                                    bodyText(item.text)
                                }.frame(maxWidth: .infinity, alignment: .leading)
                            }
                        } else if tab == "details" {
                            details(context.details)
                        } else {
                            if context.state.compactions.isEmpty { Text(localized("尚无压缩记录。")) }
                            ForEach(context.state.compactions) { item in
                                VStack(alignment: .leading, spacing: 8) {
                                    Text("Epoch \(item.epoch) · \(item.strategy)").font(.subheadline).bold()
                                    Text("\(item.before.formatted()) → \(item.after.formatted()) tokens").font(.caption).monospacedDigit()
                                    if let checkpoint = item.checkpoint { bodyText(checkpoint) }
                                    if context.state.undone_epochs.contains(item.epoch) { Text(localized("已撤销")).foregroundStyle(.secondary) }
                                    else if let reason = item.undo_reason { Text(reason).font(.caption).foregroundStyle(.secondary) }
                                    if context.state.undoable?.epoch == item.epoch {
                                        Button(localized("撤销这次压缩…")) { undoEpoch = item }.disabled(!writable)
                                    }
                                }.frame(maxWidth: .infinity, alignment: .leading)
                            }
                        }
                    }
                    if let error = error ?? conversation.operationError ?? conversation.snapshot?.error { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }
            HStack {
                Button(localized("重新读取")) { Task { await load() } }.disabled(loading || conversation.busy)
                Spacer()
                Button(localized("压缩上下文…")) { compacting = true }.disabled(!writable)
            }
        }.padding(24).frame(minWidth: 320, idealWidth: 680, maxWidth: 760, minHeight: 450, idealHeight: 580, maxHeight: 720)
            .interactiveDismissDisabled(conversation.busy)
            .background(NativeSettingsEscape(enabled: !conversation.busy, close: dismiss))
            .task { if loadOnAppear { await load() } }
            .onChange(of: conversation.snapshot?.running) { running in if running == false { Task { await load() } } }
            .onDisappear { closed = true; readGeneration = UUID() }
            .sheet(isPresented: $compacting) {
                NativeCompactConfirmation(conversation: conversation, canConfirm: writable, close: { compacting = false }) { semantic, instruction in
                    if await conversation.compact(project: project, session: session, semantic: semantic, instruction: instruction) { compacting = false; await load() }
                }
            }
            .sheet(item: $undoEpoch) { item in
                NativeCompactionUndoConfirmation(conversation: conversation, epoch: item.epoch, canConfirm: writable && context?.state.undoable?.epoch == item.epoch, close: { undoEpoch = nil }) {
                    if await conversation.undoCompaction(project: project, session: session, epoch: item.epoch) { undoEpoch = nil; await load() }
                }
            }
    }
    private func bodyText(_ text: String) -> some View {
        NativeMessageBody(text: AttributedString(text), markdown: text, resources: [], saved: [], revealed: nil,
                          monospaced: false, client: conversation.client, project: project, session: session, quote: nil, save: nil)
    }
    @ViewBuilder private func details(_ value: NativeContextDetails) -> some View {
        section("系统提示", value.system_prompt)
        section("规则", value.rules)
        section("技能", value.skills)
        tools("工具定义", value.tool_definitions)
        tools("动态 MCP 工具", value.mcp_dynamic_tools)
        tools("子智能体定义", value.subagent_definitions)
    }
    @ViewBuilder private func section(_ name: String, _ text: String) -> some View {
        if !text.isEmpty { DisclosureGroup(localized(name)) { bodyText(text) } }
    }
    @ViewBuilder private func tools(_ name: String, _ values: [NativeContextDetails.Tool]) -> some View {
        if !values.isEmpty {
            DisclosureGroup("\(localized(name)) · \(values.count)") {
                ForEach(Array(values.enumerated()), id: \.offset) { _, tool in
                    VStack(alignment: .leading, spacing: 4) { Text(tool.name).bold(); Text(tool.description).font(.caption).textSelection(.enabled) }
                        .frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 4)
                }
            }
        }
    }
    private func dismiss() { closed = true; readGeneration = UUID(); close() }
    private func load() async {
        guard !loading && !closed else { return }
        let current = UUID(); readGeneration = current
        loading = true; error = nil
        defer { if readGeneration == current { loading = false } }
        do {
            let value = try await conversation.readContext(project: project, session: session)
            guard readGeneration == current && !closed && !Task.isCancelled else { return }
            context = value
        } catch { if readGeneration == current && !closed && !Task.isCancelled { self.error = localized("无法读取模型上下文。") + "\n" + error.localizedDescription } }
    }
}

struct NativeCompactConfirmation: View {
    @ObservedObject var conversation: NativeConversationModel
    let canConfirm: Bool
    let close: () -> Void
    let confirm: (Bool, String) async -> Void
    @State private var semantic = false
    @State private var instruction = ""
    init(conversation: NativeConversationModel, canConfirm: Bool, semantic: Bool = false, instruction: String = "", close: @escaping () -> Void, confirm: @escaping (Bool, String) async -> Void) {
        self.conversation = conversation; self.canConfirm = canConfirm; self.close = close; self.confirm = confirm
        _semantic = State(initialValue: semantic); _instruction = State(initialValue: instruction)
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(localized("压缩上下文")).font(.headline)
            Text(localized("常规压缩保留近期对话并缩短旧工具输出。语义压缩会调用模型生成检查点，保留继续工作的重点。")) .font(.caption).foregroundStyle(.secondary)
            Toggle(localized("使用语义压缩"), isOn: $semantic).disabled(conversation.busy)
            if semantic {
                Text(localized("保留重点（可选）")).font(.caption)
                TextEditor(text: $instruction).font(.system(size: 13)).frame(minHeight: 90, maxHeight: 160).disabled(conversation.busy)
                    .accessibilityLabel(localized("保留重点（可选）"))
            }
            if let error = conversation.operationError { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
            Spacer(minLength: 0)
            HStack {
                Button(localized("取消"), action: close).disabled(conversation.busy)
                Spacer()
                Button(localized("开始压缩")) { Task { await confirm(semantic, instruction) } }
                    .buttonStyle(WispButtonStyle(primary: true)).disabled(!canConfirm || conversation.busy || instruction.utf8.count > 16_384)
            }
        }.padding(24).frame(minWidth: 320, idealWidth: 460, maxWidth: 560, minHeight: 280, idealHeight: 360, maxHeight: 480)
            .interactiveDismissDisabled(conversation.busy)
            .background(NativeSettingsEscape(enabled: !conversation.busy, close: close))
    }
}

struct NativeCompactionUndoConfirmation: View {
    @ObservedObject var conversation: NativeConversationModel
    let epoch: UInt64
    let canConfirm: Bool
    let close: () -> Void
    let confirm: () async -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(localized("撤销压缩")).font(.headline)
            Text("Epoch \(epoch)").font(.caption).monospacedDigit()
            Text(localized("恢复压缩前的模型工作集。完整对话和现有草稿会保留；压缩后若已继续对话，则无法撤销。"))
            if let error = conversation.operationError { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
            HStack {
                Button(localized("取消"), action: close).disabled(conversation.busy)
                Spacer()
                Button(localized("确认撤销")) { Task { await confirm() } }.disabled(!canConfirm || conversation.busy).buttonStyle(WispButtonStyle(primary: true))
            }
        }.padding(24).frame(minWidth: 320, idealWidth: 460, maxWidth: 560, minHeight: 250)
            .interactiveDismissDisabled(conversation.busy)
            .background(NativeSettingsEscape(enabled: !conversation.busy, close: close))
    }
}
