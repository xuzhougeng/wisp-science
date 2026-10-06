import AppKit
import SwiftUI
import WispProjectBrowser

/// Database clones may retain project/session IDs. Recreate all pane models
/// when their transport's database changes, even when those IDs stay equal.
struct NativePanelScopeIdentity: Hashable {
    let databaseURL: URL
    let projectID: String
    let sessionID: String
}

struct NativePanelView: View {
    @StateObject private var model: NativePanelModel
    @AppStorage("native.workspace.panel.tab") private var tab = "artifacts"
    @AppStorage("native.workspace.panel.tabs") private var savedTabs = ""
    @AppStorage("native.workspace.panel.grid") private var grid = false
    @State private var draggedTab: String?
    @Environment(\.colorScheme) private var scheme
    @State private var query = ""
    @State private var exportError: String?
    private let projectRoot: String
    @State private var transcriptPreview: NativeTranscriptArtifact?
    private var transcriptArtifacts: [NativeTranscriptArtifact] { NativeTranscriptArtifact.collect(transcript) }
    @State private var activity: NativeContextActivitySelection?
    @State private var fileAction: NativeFileActionSelection?
    private var availableTabs: [String] { NativePanelTabs.defaults + ["notebook", "highlights", "provenance"] + (sideChat == nil ? [] : ["sidechat"]) }
    let highlightRevision: Int
    let highlightRemoved: (String) -> Void
    let sideChat: NativeSideChatModel?
    let revealExcerpt: (String) -> Void
    let transcript: [ConversationItem]
    let transcriptPage: String
    let openTerminal: (String) -> Void
    let manageWorkflows: () -> Void
    let readOnly: Bool
    var fileBrowserSupported = false
    var fileTransfersSupported = false
    var environments: () -> Void = {}
    let close: () -> Void
    init(client: any NativeConversationQuerying, projectID: String, sessionID: String, projectRoot: String = "", highlightRevision: Int = 0, highlightRemoved: @escaping (String) -> Void = { _ in }, sideChat: NativeSideChatModel? = nil, transcript: [ConversationItem] = [], transcriptPage: String = "latest", revealExcerpt: @escaping (String) -> Void = { _ in }, readOnly: Bool = false, fileBrowserSupported: Bool = false, fileTransfersSupported: Bool = false, environments: @escaping () -> Void = {}, manageWorkflows: @escaping () -> Void = {}, openTerminal: @escaping (String) -> Void = { _ in }, close: @escaping () -> Void) {
        self.projectRoot = projectRoot
        _model = StateObject(wrappedValue: NativePanelModel(client: client, projectID: projectID, sessionID: sessionID)); self.highlightRevision = highlightRevision; self.highlightRemoved = highlightRemoved; self.sideChat = sideChat; self.revealExcerpt = revealExcerpt; self.transcript = transcript; self.transcriptPage = transcriptPage; self.manageWorkflows = manageWorkflows; self.openTerminal = openTerminal; self.readOnly = readOnly; self.fileBrowserSupported = fileBrowserSupported; self.environments = environments; self.close = close
        self.fileTransfersSupported = fileTransfersSupported
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                tabStrip
                if !["provenance", "sidechat"].contains(tab) && !(tab == "files" && fileBrowserSupported) { Button { Task { await model.refresh(tab) } } label: { WispIcon(name: "refresh") }.buttonStyle(.plain).help("刷新") }
                Button(action: close) { WispIcon(name: "close", size: 16) }.buttonStyle(.plain).help("关闭面板").accessibilityLabel("关闭面板")
            }
            if tab != "sidechat" && !(tab == "files" && fileBrowserSupported) {
            HStack(spacing: 6) {
                TextField(tab == "files" ? "搜索当前项目全部目录" : tab == "artifacts" ? "搜索本会话产物名称或路径" : tab == "provenance" ? "搜索工具、输入或输出" : tab == "notebook" ? "搜索代码或输出" : "筛选名称", text: $query)
                    .accessibilityLabel(tab == "files" ? "搜索项目文件" : "搜索面板")
                if ["artifacts", "files"].contains(tab) { NativePanelDisplayControls(grid: $grid).fixedSize() }
            }
            if model.loading || model.searchLoading { ProgressView().controlSize(.small) }
            if let error = exportError ?? model.error { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
            }
            if tab == "files" && !fileBrowserSupported {
                HStack {
                    Button { navigate(model.parent) } label: { WispIcon(name: "arrow-up", size: 16).frame(width: 30, height: 30) }.buttonStyle(.plain).disabled(model.path == ".").accessibilityLabel("上级目录")
                    Text(query.isEmpty ? model.path == "." ? "当前项目 /" : model.path : "当前项目 · 最多 200 项").font(.caption).lineLimit(1).truncationMode(.head).help(model.path)
                    Spacer()
                    Menu {
                        Button("新建文件") { fileAction = .init(action: .createFile, directory: model.path) }
                        Button("新建文件夹") { fileAction = .init(action: .createDirectory, directory: model.path) }
                    } label: { WispIcon(name: "plus", size: 16).frame(width: 30, height: 30) }
                        .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().accessibilityLabel("新建文件或文件夹")
                        .disabled(readOnly || model.loading || model.fileActionBusy || !query.isEmpty)
                }
            }
            if tab == "files" && fileBrowserSupported {
                NativeFilesView(client: model.client, projectID: model.projectID, sessionID: model.sessionID, readOnly: readOnly, quote: sideChat == nil ? nil : { quote in
                    guard let sideChat, sideChat.projectID == model.projectID, sideChat.sessionID == model.sessionID else { return }
                    sideChat.quotes.append(quote)
                    var value = layout; value.show("sidechat"); store(value)
                }, environments: environments, runs: { context in activity = .init(context: context, runtimes: false) }, transfersSupported: fileTransfersSupported)
            } else if tab == "sidechat", let sideChat {
                NativeSideChatView(model: sideChat)
            } else {
            ScrollView {
                LazyVGrid(columns: grid && tab == "files" ? [GridItem(.adaptive(minimum: 130), alignment: .top)] : [GridItem(.flexible(), alignment: .leading)], alignment: .leading, spacing: 8) {
                    if tab == "artifacts" {
                        NativeArtifactCollection(registered: model.artifacts, messages: transcriptArtifacts, query: query, grid: grid, loading: model.loading, failed: model.error != nil, openRegistered: { id in
                            Task { await model.readArtifact(id) }
                        }, openMessage: { transcriptPreview = $0 }, provenance: {
                            var value = layout; value.show("provenance"); store(value)
                        }, projectRoot: projectRoot, exportRegistered: { id in saveAs(artifactID: id) })
                    } else if tab == "notebook" {
                        NativeNotebookView(model: model, cells: NativeNotebookCell.collect(transcript), query: query).id(transcriptPage)
                    } else if tab == "highlights" {
                        NativeHighlightsView(model: model, query: query, reveal: revealExcerpt, removed: highlightRemoved)
                    } else if tab == "provenance" {
                        NativeProvenanceView(rows: NativeProvenanceRow.collect(transcript), query: query).id(transcriptPage)
                    } else if tab == "agents" {
                        NativeAgentPanelView(model: model, query: query, readOnly: readOnly, manageWorkflows: manageWorkflows)
                    } else if tab == "hosts" {
                        NativePanelContextsView(model: model, query: query, openTerminal: openTerminal) { context, runtimes in activity = .init(context: context, runtimes: runtimes) }
                    } else {
                        if query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                            ForEach(model.files) { file in
                                fileRow(name: file.name, path: model.child(file), directory: file.is_dir, size: file.size)
                            }
                            if model.files.isEmpty && !model.loading { Text("目录为空").foregroundStyle(.secondary).padding() }
                        } else {
                            ForEach(model.searchHits) { hit in
                                fileRow(name: hit.name, path: hit.path, directory: hit.is_dir, size: hit.size)
                            }
                            if model.searchHits.isEmpty && !model.searchLoading { Text("没有匹配的项目文件").foregroundStyle(.secondary).padding() }
                        }
                    }
                }
            }
            }
        }.padding(12).frame(maxHeight: .infinity).background(WispDesign.color("bg-sunken", scheme))
            .background(NativeSettingsEscape(close: close))
            .onAppear { var value = layout; value.reopen(); store(value) }
            .onChange(of: tab) { _ in query = ""; exportError = nil; model.clearFileSearch() }
            .task(id: tab + "\n" + query + "\n" + String(fileBrowserSupported)) {
                if tab == "files" && !fileBrowserSupported { await model.searchFiles(query) }
            }
            .task(id: tab + "\n" + String(fileBrowserSupported)) {
                if !availableTabs.contains(tab) { tab = "artifacts" }
                if tab == "files" && fileBrowserSupported { return }
                await model.refresh(tab)
                while tab == "agents" && !Task.isCancelled {
                    try? await Task.sleep(nanoseconds: 2_000_000_000)
                    if !Task.isCancelled { await model.refresh("agents", quiet: true) }
                }
            }
            .sheet(isPresented: Binding(get: { model.preview != nil }, set: { if !$0 { model.dismissPreview() } })) {
                if let content = model.preview {
                    NativePanelFilePreview(content: content, close: model.dismissPreview, save: !readOnly && model.previewEditable ? { text in try await model.savePreview(text, original: content) } : nil, quote: sideChat == nil ? nil : { text in
                        guard let sideChat, sideChat.projectID == model.projectID, sideChat.sessionID == model.sessionID,
                              let quote = model.selectedPreviewQuote(text, path: content.path) else { return }
                        sideChat.quotes.append(quote)
                        model.dismissPreview()
                        var value = layout; value.show("sidechat"); store(value)
                    })
                }
            }
            .sheet(item: $transcriptPreview) { artifact in
                VStack(alignment: .leading, spacing: 16) {
                    HStack {
                        Text(artifact.title).font(.headline)
                        Spacer()
                        Button(localized("复制源内容")) { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(artifact.source, forType: .string) }
                        Button(localized("关闭")) { transcriptPreview = nil }
                    }
                    Text(localized("内容来自当前消息页；复制保留原始内容。")) .font(.caption).foregroundStyle(.secondary)
                    ScrollView {
                        NativeSelectableMessage(text: AttributedString(artifact.source), saved: [], quote: nil, save: nil,
                                                markdown: artifact.kind == "table" ? artifact.source : "$$\n" + artifact.source + "\n$$")
                    }
                }.padding(24).frame(width: 600, height: 420)
                    .background(NativeSettingsEscape { transcriptPreview = nil })
            }
            .onChange(of: transcriptPage) { _ in transcriptPreview = nil }
            .sheet(item: $model.agentResult, onDismiss: model.dismissPreview) { result in
                NativeAgentResultView(result: result, close: model.dismissPreview)
            }
            .sheet(item: $activity) { selection in
                NativeContextActivityView(client: model.client, projectID: model.projectID, sessionID: model.sessionID, selection: selection) { activity = nil }
            }
            .sheet(item: $fileAction) { selection in
                NativeFileActionView(selection: selection, model: model) { fileAction = nil }
            }
            .onChange(of: highlightRevision) { _ in if tab == "highlights" { Task { await model.refresh("highlights") } } }
            .onDisappear { model.close() }
    }
    private func navigate(_ path: String) {
        query = ""; model.clearFileSearch()
        Task { await model.refresh("files", directory: path) }
    }
    private func fileRow(name: String, path: String, directory: Bool, size: UInt64) -> some View {
        let layout = grid ? AnyLayout(VStackLayout(alignment: .leading, spacing: 0)) : AnyLayout(HStackLayout(alignment: .top, spacing: 2))
        return layout {
            Button { if directory { navigate(path) } else { Task { await model.readFile(path) } } } label: {
                row(title: name, subtitle: query.isEmpty ? directory ? "文件夹" : ByteCountFormatter.string(fromByteCount: Int64(clamping: size), countStyle: .file) : path, icon: directory ? "folder" : "doc")
            }.buttonStyle(.plain)
            HStack(spacing: 2) {
                if grid { Spacer(minLength: 0) }
                if !directory {
                    Button { saveAs(path: path) } label: { WispIcon(name: "download", size: 16).frame(width: 30, height: 30) }
                        .buttonStyle(.plain).help("保存副本").accessibilityLabel("保存副本 " + name)
                }
                Menu {
                    Button { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(path, forType: .string) } label: { Label { Text("复制路径") } icon: { WispIcon(name: "copy") } }
                    Button { selectAction(.rename, path: path) } label: { Label { Text("重命名") } icon: { WispIcon(name: "edit") } }.disabled(readOnly || model.fileActionBusy)
                    Button(role: .destructive) { selectAction(.delete, path: path) } label: { Label { Text("删除") } icon: { WispIcon(name: "trash") } }.disabled(readOnly || model.fileActionBusy)
                } label: { WispIcon(name: "more", size: 16).frame(width: 30, height: 30) }
                    .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().accessibilityLabel("文件操作 " + name)
            }
        }.background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 8))
    }
    private func selectAction(_ action: NativePanelFileAction, path: String) {
        let directory = (path as NSString).deletingLastPathComponent
        fileAction = .init(action: action, directory: directory.isEmpty ? "." : directory, name: (path as NSString).lastPathComponent)
    }
    private func saveAs(path: String? = nil, artifactID: String? = nil) {
        exportError = nil
        Task {
            do {
                let source = try await model.exportSource(path: path, artifactID: artifactID)
                let panel = NSSavePanel(); panel.nameFieldStringValue = source.name; panel.prompt = "保存副本"
                guard panel.runModal() == .OK, let destination = panel.url else { return }
                try await NativePanelExportCopy.copy(source, to: destination)
            } catch { exportError = "保存副本失败：" + error.localizedDescription }
        }
    }
    private var layout: NativePanelTabs { NativePanelTabs(saved: savedTabs, selected: tab, available: availableTabs) }
    private func store(_ value: NativePanelTabs) { savedTabs = value.saved; tab = value.selected }
    private func title(_ id: String) -> String {
        if id == "artifacts" { return localized("产物") + " (\(model.artifacts.count + transcriptArtifacts.count))" }
        if id == "notebook" { return "笔记本 (\(NativeNotebookCell.collect(transcript).count))" }
        if id == "highlights" { return "划线 (\(model.highlights.count))" }
        if id == "provenance" { return "溯源 (\(NativeProvenanceRow.collect(transcript).count))" }
        return ["artifacts": "产物", "agents": "Agents", "files": "文件", "hosts": "环境", "sidechat": "侧聊"][id] ?? id
    }
    private func removeTab(_ id: String) {
        var value = layout; value.remove(id); store(value)
        if value.open.isEmpty { close() }
    }
    private func moveTab(_ id: String, to target: String) {
        var value = layout; value.move(id, to: target); store(value)
    }
    private var tabStrip: some View {
        HStack(spacing: 4) {
            ScrollViewReader { proxy in
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 4) {
                        ForEach(layout.open, id: \.self) { id in
                            HStack(spacing: 4) {
                                Button(title(id)) { var value = layout; value.show(id); store(value) }
                                    .font(.system(size: 12, weight: tab == id ? .semibold : .regular))
                                    .accessibilityAddTraits(tab == id ? .isSelected : [])
                                Button { removeTab(id) } label: { WispIcon(name: "close", size: 12) }
                                    .help("关闭" + title(id)).accessibilityLabel("关闭" + title(id))
                            }.buttonStyle(.plain).padding(.horizontal, 8).padding(.vertical, 7)
                                .background(tab == id ? WispDesign.color("bg-elev", scheme) : .clear, in: RoundedRectangle(cornerRadius: 6))
                                .id(id)
                                .onDrag { draggedTab = id; return NSItemProvider(item: id as NSString, typeIdentifier: "science.wisp.native-panel-tab") }
                                .onDrop(of: ["science.wisp.native-panel-tab"], isTargeted: nil) { _ in
                                    guard let source = draggedTab else { return false }
                                    draggedTab = nil; moveTab(source, to: id); return true
                                }
                                .contextMenu {
                                    if let index = layout.open.firstIndex(of: id) {
                                        Button("向左移动") { moveTab(id, to: layout.open[index - 1]) }.disabled(index == 0)
                                        Button("向右移动") { moveTab(id, to: layout.open[index + 1]) }.disabled(index == layout.open.count - 1)
                                    }
                                    Button("关闭标签") { removeTab(id) }
                                }
                        }
                    }
                }.onChange(of: tab) { id in proxy.scrollTo(id) }
                    .onAppear { proxy.scrollTo(tab) }
            }
            Menu {
                ForEach(availableTabs, id: \.self) { id in
                    Button { var value = layout; value.show(id); store(value) } label: {
                        if layout.open.contains(id) { Label(title(id), systemImage: "checkmark") } else { Text(title(id)) }
                    }
                }
            } label: { WispIcon(name: "plus", size: 14) }
                .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().help("添加面板").accessibilityLabel("添加面板")
        }
    }
    private func row(title: String, subtitle: String, icon: String) -> some View {
        NativePanelTile(title: title, subtitle: subtitle, icon: icon, grid: grid)
    }
}

/// Shared presentation controls for the file and artifact collections. The view
/// preference is client-local; both modes use the same scoped backend records.
struct NativePanelDisplayControls: View {
    @Binding var grid: Bool
    var body: some View {
        HStack(spacing: 4) {
            mode("列表", icon: "list", value: false)
            mode("网格", icon: "grid", value: true)
        }
    }
    private func mode(_ label: String, icon: String, value: Bool) -> some View {
        Button { grid = value } label: {
            WispIcon(name: icon, size: 16).frame(width: 30, height: 30)
                .background(grid == value ? Color.accentColor.opacity(0.14) : .clear, in: RoundedRectangle(cornerRadius: 5))
        }.buttonStyle(.plain).help(localized(label)).accessibilityLabel(localized(label))
            .accessibilityValue(localized(grid == value ? "已选择" : "未选择"))
    }
}

struct NativePanelTile: View {
    let title: String
    let subtitle: String
    let icon: String
    let grid: Bool
    @Environment(\.colorScheme) private var scheme
    var body: some View {
        let layout = grid ? AnyLayout(VStackLayout(alignment: .leading, spacing: 6)) : AnyLayout(HStackLayout(alignment: .top, spacing: 8))
        layout {
            WispIcon(name: icon, size: grid ? 26 : 16)
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(WispDesign.font(size: 13, weight: .semibold)).lineLimit(2)
                Text(subtitle).font(WispDesign.font(size: 10)).foregroundStyle(.secondary).lineLimit(2)
            }
            if !grid { Spacer(minLength: 0) }
        }.padding(8).frame(maxWidth: .infinity, minHeight: grid ? 84 : nil, alignment: .topLeading)
            .background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 8))
            .help(title + "\n" + subtitle)
    }
}

struct NativePanelFilePreview: View {
    let content: NativePanelFileContent
    let close: () -> Void
    var save: ((String) async throws -> Void)? = nil
    var quote: ((String) -> Void)? = nil
    @State private var editing = false
    @State private var draft = ""
    @State private var saving = false
    @State private var saveError: String?
    @State private var saveUnconfirmed = false
    @State private var confirmDiscard = false
    private var dirty: Bool { editing && draft != content.text }
    private func requestClose() { if dirty { confirmDiscard = true } else { close() } }
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text((content.path as NSString).lastPathComponent).font(.headline).lineLimit(1).truncationMode(.middle)
                Spacer()
                if let save, content.text != nil, !content.truncated {
                    if editing {
                        Button(localized(saving ? "正在保存…" : "保存")) {
                            saving = true; saveError = nil
                            let text = draft
                            Task {
                                do { try await save(text); editing = false }
                                catch { saveUnconfirmed = true; saveError = localized("保存未确认成功，请关闭并重新读取文件后再操作。") + error.localizedDescription }
                                saving = false
                            }
                        }.disabled(!dirty || saving || saveUnconfirmed)
                    } else {
                        Button(localized("编辑")) { draft = content.text ?? ""; editing = true; saveError = nil }
                    }
                }
                Button(localized("关闭预览"), action: requestClose).disabled(saving)
            }
            Text(content.path).font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle).help(content.path).textSelection(.enabled)
            if let saveError { Text(saveError).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
            if content.truncated { Text(localized("仅展示文件开头；完整文件大小") + " \(content.total_bytes ?? 0) bytes").font(.caption).foregroundStyle(.orange) }
            if editing {
                TextEditor(text: $draft).font(.system(size: 12, design: .monospaced)).disabled(saving).accessibilityLabel("文件内容")
            } else if let text = content.text {
                ScrollView { NativeSelectableMessage(text: AttributedString(text), saved: [], quote: quote, save: nil, monospaced: true).frame(maxWidth: .infinity, alignment: .topLeading) }
            } else if content.base64 != nil { NativeFileBytesPreview(content: content) }
            else { Text(localized("无法预览文件。")) }
        }.padding(16).frame(minWidth: 320, idealWidth: 850, minHeight: 420, idealHeight: 650)
            .interactiveDismissDisabled(dirty || saving)
            .background(NativeSettingsEscape(enabled: !saving, close: requestClose))
            .confirmationDialog(localized("放弃未保存的修改？"), isPresented: $confirmDiscard) {
                Button(localized("放弃修改"), role: .destructive, action: close)
                Button(localized("继续编辑"), role: .cancel) {}
            }
    }
}

struct NativePanelContextsView: View {
    @ObservedObject var model: NativePanelModel
    var query = ""
    var openTerminal: (String) -> Void = { _ in }
    var showActivity: (String, Bool) -> Void = { _, _ in }
    @Environment(\.colorScheme) private var scheme
    @ViewBuilder var body: some View {
        if let snapshot = model.contexts {
            ForEach(snapshot.attached.filter { query.isEmpty || $0.label.localizedCaseInsensitiveContains(query) || $0.id.localizedCaseInsensitiveContains(query) }) { context in
                VStack(alignment: .leading, spacing: 8) {
                    Text(context.label.isEmpty ? context.id : context.label).font(.headline)
                    Text(context.kind + " · " + (context.last_probe_status ?? "尚未探测")).font(.caption).foregroundStyle(.secondary)
                    if let error = context.last_probe_error { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
                    HStack {
                        Button("探测") { Task { await model.probeContext(context.id) } }
                        if context.kind != "local" { Button("从会话移除") { Task { await model.setContext(context.id, enabled: false) } }.disabled(snapshot.read_only) }
                    }.disabled(model.contextBusy)
                    HStack {
                        Button("终端") { openTerminal(context.id) }
                        Button("运行时") { showActivity(context.id, true) }
                        Button("任务列表") { showActivity(context.id, false) }
                    }
                    DisclosureGroup("机器信息") {
                        NativeSettingsSummary(value: SettingsValue.string(context.capabilities_json).decodedJSON)
                    }
                }.padding(10).frame(maxWidth: .infinity, alignment: .leading)
                    .background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 8))
            }
            if !snapshot.available.isEmpty {
                Menu("关联执行环境") {
                    ForEach(snapshot.available) { context in
                        Button(context.label.isEmpty ? context.id : context.label) { Task { await model.setContext(context.id, enabled: true) } }
                    }
                }.disabled(snapshot.read_only || model.contextBusy)
            }
            if snapshot.read_only { Text("归档或只读会话不能修改关联环境").font(.caption).foregroundStyle(.secondary) }
        }
    }
}
