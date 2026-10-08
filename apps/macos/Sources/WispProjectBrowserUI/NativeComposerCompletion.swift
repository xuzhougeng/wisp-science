import AppKit
import SwiftUI
import WispProjectBrowser

/// Offsets are UTF-16, matching NSTextView and the WebView textarea contract.
struct NativeComposerTrigger: Equatable {
    let range: NSRange
    let kind: String
    let query: String

    static func active(in text: String, selection: NSRange) -> Self? {
        guard selection.length == 0, selection.location >= 0 else { return nil }
        var offset = 0
        var previous: Unicode.Scalar?
        var last: (Int, Unicode.Scalar, Unicode.Scalar?)?
        for scalar in text.unicodeScalars {
            if offset == selection.location { break }
            let size = scalar.value > 0xffff ? 2 : 1
            guard offset + size <= selection.location else { return nil }
            if scalar == "@" || scalar == "#" || scalar == "/" { last = (offset, scalar, previous) }
            offset += size; previous = scalar
        }
        guard offset == selection.location, let (start, symbol, before) = last else { return nil }
        if let before {
            let asciiWord = (48...57).contains(before.value) || (65...90).contains(before.value) || (97...122).contains(before.value) || before == "_"
            if asciiWord || (symbol == "/" && [":", "/", "\\", "."].contains(String(before))) { return nil }
        }
        let query = (text as NSString).substring(with: NSRange(location: start + 1, length: offset - start - 1))
        guard !query.unicodeScalars.contains(where: CharacterSet.whitespacesAndNewlines.contains) else { return nil }
        return Self(range: NSRange(location: start, length: offset - start), kind: symbol == "@" ? "artifact" : symbol == "#" ? "session" : "skill", query: query)
    }
}

/// These commands mirror existing native surfaces. Selecting an action never
/// sends its name to a model; payload commands first fill the editor.
enum NativeComposerCommand: String, CaseIterable {
    case archive, btw, skills, files, upload, share, trajectory
    var description: String {
        switch self {
        case .archive: return "打开研究归档"
        case .btw: return "打开侧聊，或输入一个侧聊问题"
        case .skills: return "打开技能设置"
        case .files: return "浏览项目文件"
        case .upload: return "添加消息附件"
        case .share: return "打开会话分享预览"
        case .trajectory: return "查看运行轨迹"
        }
    }
    var icon: String {
        switch self {
        case .archive: return "archive"
        case .btw: return "bubble"
        case .skills: return "book"
        case .files: return "folder"
        case .upload: return "upload"
        case .share: return "share"
        case .trajectory: return "timeline"
        }
    }
    static func parse(_ text: String) -> (Self, String)? {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard text.hasPrefix("/") else { return nil }
        let body = text.dropFirst()
        let name = body.prefix { !$0.isWhitespace }
        guard let command = Self(rawValue: String(name)) else { return nil }
        return (command, body.dropFirst(name.count).trimmingCharacters(in: .whitespacesAndNewlines))
    }
}

enum NativeComposerCandidate: Identifiable, Equatable {
    case reference(NativeComposerReference)
    case command(NativeComposerCommand)
    var id: String { switch self { case .reference(let ref): return ref.id; case .command(let cmd): return "command:" + cmd.rawValue } }
    var label: String { switch self { case .reference(let ref): return ref.label; case .command(let cmd): return "/" + cmd.rawValue } }
    var detail: String { switch self { case .reference(let ref): return ref.detail; case .command(let cmd): return cmd.description } }
    var section: String {
        switch self {
        case .command: return "命令"
        case .reference(let ref): return ref.reference["kind"].string == "workflow" ? "工作流" : "技能"
        }
    }
    var icon: String {
        switch self {
        case .command(let cmd): return cmd.icon
        case .reference(let ref):
            switch ref.reference["kind"].string {
            case "artifact": return "doc"
            case "project": return "folder"
            case "session": return "chat"
            case "context": return "server"
            case "runtime": return "terminal"
            case "workflow": return "plan"
            default: return "sparkles"
            }
        }
    }
}

@MainActor final class NativeComposerCompletionModel: ObservableObject {
    @Published private(set) var trigger: NativeComposerTrigger?
    @Published private(set) var candidates: [NativeComposerCandidate] = []
    @Published private(set) var selected = 0
    @Published private(set) var searching = false
    @Published private(set) var error: String?
    @Published private(set) var composing = false
    private let client: any NativeConversationQuerying
    private var project: String?
    private var session: String?
    private var revision = UUID()
    private var searchTask: Task<Void, Never>?
    private var escape: NativeSettingsEscape.Coordinator?
    private var mouseMonitor: Any?
    weak var editor: NativeComposerTextView?
    weak var listBounds: NSView?
    var isOpen: Bool { trigger != nil && !composing }

    init(client: any NativeConversationQuerying) { self.client = client }
    deinit { searchTask?.cancel(); escape?.remove(); if let mouseMonitor { NSEvent.removeMonitor(mouseMonitor) } }
    func bind(project: String, session: String) { reset(); self.project = project; self.session = session }
    func reset() { dismiss(); project = nil; session = nil }
    private func retire() {
        revision = UUID(); searchTask?.cancel(); searchTask = nil
        escape?.remove(); escape = nil
        if let mouseMonitor { NSEvent.removeMonitor(mouseMonitor) }; mouseMonitor = nil
    }
    func detach(_ editor: NativeComposerTextView) {
        guard self.editor === editor else { return }
        // Representable teardown may occur while SwiftUI invalidates its graph.
        // Retire events/requests now and publish after that teardown finishes.
        retire(); self.editor = nil; let current = revision
        Task { [weak self] in
            guard let self, self.editor == nil, self.revision == current else { return }
            self.dismiss()
        }
    }
    func dismiss() {
        retire()
        if trigger != nil { trigger = nil }
        if !candidates.isEmpty { candidates = [] }
        if selected != 0 { selected = 0 }
        if searching { searching = false }
        if error != nil { error = nil }
        if composing { composing = false }
    }
    func suspendComposition() {
        composing = true; revision = UUID(); searchTask?.cancel(); searchTask = nil; searching = false
    }
    func selectionChanged() {
        guard let editor, !editor.hasMarkedText(), let trigger else { return }
        if NativeComposerTrigger.active(in: editor.string, selection: editor.selectedRange()) != trigger { dismiss() }
    }
    func edited(insertion: Bool) {
        guard let editor, editor.isEditable, let project, let session else { dismiss(); return }
        guard !editor.hasMarkedText() else { suspendComposition(); return }
        composing = false
        guard let active = NativeComposerTrigger.active(in: editor.string, selection: editor.selectedRange()),
              (insertion && active.query.isEmpty) || (trigger?.kind == active.kind && trigger?.range.location == active.range.location),
              editor.referencesAvailable?() == true || (active.kind == "skill" && !editor.completionCommands.isEmpty) else { dismiss(); return }
        trigger = active; selected = 0; error = nil
        revision = UUID(); let current = revision
        searchTask?.cancel()
        candidates = commandCandidates(active)
        installDismissal(in: editor)
        guard editor.referencesAvailable?() == true else { searching = false; return }
        guard active.query.utf8.count <= 512 else { searching = false; error = "搜索词过长"; return }
        searching = true
        searchTask = Task { [weak self, client] in
            do {
                if !active.query.isEmpty { try await Task.sleep(nanoseconds: 120_000_000) }
                let value = try await client.invoke("native_conversation_references", args: ["session_id": .string(session), "kind": .string(active.kind), "query": .string(active.query)], projectID: project)
                let catalog = try NativeComposerReferenceCatalog.decode(value, session: session)
                guard let self, self.revision == current, !Task.isCancelled else { return }
                self.candidates = self.commandCandidates(active) + catalog.map(NativeComposerCandidate.reference)
                self.searching = false
            } catch {
                guard let self, self.revision == current, !Task.isCancelled else { return }
                self.searching = false; self.error = error.localizedDescription
            }
        }
    }
    private func commandCandidates(_ trigger: NativeComposerTrigger) -> [NativeComposerCandidate] {
        guard trigger.kind == "skill" else { return [] }
        return (editor?.completionCommands ?? []).filter { trigger.query.isEmpty || $0.rawValue.contains(trigger.query.lowercased()) }.map(NativeComposerCandidate.command)
    }
    func move(_ delta: Int) {
        guard !candidates.isEmpty else { return }
        selected = (selected + delta + candidates.count) % candidates.count
    }
    func accept(_ index: Int? = nil) {
        guard let editor, editor.isEditable, !editor.hasMarkedText(), let trigger,
              NativeComposerTrigger.active(in: editor.string, selection: editor.selectedRange()) == trigger else { dismiss(); return }
        guard candidates.indices.contains(index ?? selected) else { return }
        let item = candidates[index ?? selected]
        switch item {
        case .reference(let reference):
            guard editor.referencesAvailable?() == true, editor.selectReference?(reference) == true else { dismiss(); return }
            dismiss(); editor.replaceCompletion(trigger.range, with: "")
        case .command(let command):
            guard editor.completionCommands.contains(command) else { dismiss(); return }
            dismiss()
            editor.replaceCompletion(trigger.range, with: command == .btw ? "/btw " : "")
            if command != .btw { editor.executeCommand?(command, "") }
        }
    }
    private func installDismissal(in editor: NativeComposerTextView) {
        if escape == nil {
            let owner = NativeSettingsEscape.Coordinator(enabled: true) { [weak self] in self?.dismiss() }
            owner.view = editor; owner.install(); escape = owner
            mouseMonitor = NSEvent.addLocalMonitorForEvents(matching: .leftMouseDown) { [weak self] event in
                self?.mouseDown(event)
                return event
            }
        }
    }
    func mouseDown(_ event: NSEvent) {
        guard let editor, event.window === editor.window else { return }
        let inEditor = editor.convert(editor.bounds, to: nil).contains(event.locationInWindow)
        let inList = listBounds.map { $0.convert($0.bounds, to: nil).contains(event.locationInWindow) } ?? false
        if !inEditor && !inList { dismiss() }
    }
}

struct NativeComposerCompletionList: View {
    @ObservedObject var model: NativeComposerCompletionModel
    @Environment(\.colorScheme) private var scheme
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if model.trigger?.kind != "skill" { Text(model.trigger?.kind == "artifact" ? "产物与计算对象" : "会话与项目").font(WispDesign.font(size: 11, weight: .semibold)).padding(10) }
            ScrollViewReader { scroll in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 2) {
                        ForEach(Array(model.candidates.enumerated()), id: \.element.id) { index, item in
                            if model.trigger?.kind == "skill" && (index == 0 || model.candidates[index - 1].section != item.section) {
                                Text(item.section).font(WispDesign.font(size: 11, weight: .semibold)).foregroundStyle(WispDesign.color("text-muted", scheme)).padding(.horizontal, 8).padding(.top, 8)
                            }
                            Button { model.accept(index) } label: {
                                HStack(spacing: 10) {
                                    WispIcon(name: item.icon, size: 16)
                                    VStack(alignment: .leading, spacing: 3) {
                                        Text(item.label).font(WispDesign.font(size: 13)).lineLimit(1)
                                        if !item.detail.isEmpty { Text(item.detail).font(WispDesign.font(size: 11)).foregroundStyle(WispDesign.color("text-muted", scheme)).lineLimit(1) }
                                    }
                                    Spacer(minLength: 0)
                                }.frame(maxWidth: .infinity, alignment: .leading).padding(8)
                                    .background(index == model.selected ? WispDesign.color("bg-sunken", scheme) : .clear, in: RoundedRectangle(cornerRadius: 6))
                            }.buttonStyle(.plain).focusable(false).id(index)
                                .accessibilityLabel(item.label + " " + item.detail).accessibilityValue(index == model.selected ? "已选择" : "")
                        }
                        if model.searching { ProgressView().controlSize(.small).padding(10) }
                        if let error = model.error { Text(error).font(WispDesign.font(size: 12)).foregroundStyle(.orange).padding(10) }
                        if !model.searching && model.error == nil && model.candidates.isEmpty { Text("没有匹配结果").font(WispDesign.font(size: 12)).padding(10) }
                    }.padding(4)
                }.frame(height: min(240, CGFloat(max(1, model.candidates.count)) * 54 + (model.trigger?.kind == "skill" ? 30 : 0)))
                    .onChange(of: model.selected) { index in scroll.scrollTo(index) }
                    .onChange(of: model.trigger) { _ in scroll.scrollTo(0, anchor: .top) }
            }
            Text("方向键选择 · Enter / Tab 确认 · Esc 关闭").font(WispDesign.font(size: 10)).foregroundStyle(WispDesign.color("text-muted", scheme)).padding(8)
        }.background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 10))
            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(WispDesign.color("border-strong", scheme)))
            .shadow(color: .black.opacity(0.12), radius: 12, y: 4)
            .background(NativeComposerCompletionBounds(model: model))
            .accessibilityIdentifier("composer-completion-list")
    }
}

private struct NativeComposerCompletionBounds: NSViewRepresentable {
    let model: NativeComposerCompletionModel
    func makeNSView(context: Context) -> NSView { let view = NSView(); model.listBounds = view; return view }
    func updateNSView(_ view: NSView, context: Context) { model.listBounds = view }
}

struct NativeInlineComposerInput: View {
    @ObservedObject var conversation: NativeConversationModel
    @ObservedObject var completions: NativeComposerCompletionModel
    let sendWithModifier: Bool
    let commands: [NativeComposerCommand]
    let executeCommand: (NativeComposerCommand, String) -> Void
    let submit: () -> Void
    var body: some View {
        NativeMessageInput(text: $conversation.draft, canSubmit: { conversation.canSend || conversation.canQueueFollowUp || conversation.canRunComposerCommand(available: commands) }, submit: submit,
                           sendWithModifier: sendWithModifier, editable: conversation.snapshot?.read_only != true && !conversation.showingHistory,
                           accessibilityLabel: "消息输入框", fontSize: 14, placeholder: localized("输入 @ 引用产物、# 引用会话、/ 选择技能或命令…"), fitsContent: true,
                           completions: completions, referencesAvailable: { conversation.canReference }, selectReference: conversation.addReference,
                           completionCommands: commands, executeCommand: executeCommand)
            .fixedSize(horizontal: false, vertical: true)
            .overlay(alignment: .topLeading) {
                VStack(spacing: 0) {
                    if completions.isOpen { NativeComposerCompletionList(model: completions) }
                }.fixedSize(horizontal: false, vertical: true)
                    .alignmentGuide(.top) { $0.height + 8 }
            }
            .onChange(of: conversation.canReference) { available in if !available { completions.dismiss() } }
            .onChange(of: commands) { _ in completions.dismiss() }
    }
}
