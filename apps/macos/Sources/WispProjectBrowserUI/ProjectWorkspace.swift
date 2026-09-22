import SwiftUI
import WispProjectBrowser

/// Native counterpart of the WebView's project shell: project navigation and
/// saved sessions on the left, the selected conversation in the main pane.
struct ProjectWorkspace: View {
    @ObservedObject var model: ProjectBrowserModel
    let project: ProjectSummary
    @ObservedObject private var conversation: NativeConversationModel
    @ObservedObject private var publication: NativePublicationModel
    init(model: ProjectBrowserModel, project: ProjectSummary) {
        self.model = model; self.project = project
        self.conversation = model.nativeConversation()
        self.publication = model.publication
    }
    @Environment(\.colorScheme) private var scheme
    @State private var sidebarVisible = true
    @State private var trajectoryPresented = false
    @State private var archivePresented = false
    @State private var sharePresented = false
    @State private var terminalVisible = false
    @AppStorage("native.workspace.panel.visible") private var panelVisible = false
    @AppStorage("native.workspace.panel.tab") private var panelTab = "artifacts"
    @AppStorage("native.workspace.panel.tabs") private var panelTabs = ""
    @State private var panelWidth: CGFloat = 340
    @State private var panelDragStart: CGFloat?
    @State private var terminalHeight: CGFloat = 300
    @State private var terminalDragStart: CGFloat?
    @State private var inboxPresented = false
    @StateObject private var inbox = NativeInboxModel()
    @StateObject private var groups = NativeSessionGroups()
    private func color(_ token: String) -> Color { WispDesign.color(token, scheme) }

    var body: some View {
        HStack(spacing: 0) {
            if sidebarVisible {
                sidebar.frame(width: 260)
                Rectangle().fill(color("border")).frame(width: 1)
            }
            VStack(spacing: 0) {
                HStack(spacing: 8) {
                    if !sidebarVisible {
                        Button { sidebarVisible = true } label: { WispIcon(name: "chevron-right") }
                            .buttonStyle(.plain).help("展开侧边栏").accessibilityLabel("展开侧边栏")
                    }
                    Text(model.sessions.first(where: { $0.id == model.activeSessionID })?.title ?? project.name)
                        .font(WispDesign.font(size: 14, weight: .semibold)).lineLimit(1)
                    Spacer()
                    Button { conversation.outlinePresented.toggle() } label: { WispIcon(name: "list") }
                        .buttonStyle(.plain).help("会话大纲").accessibilityLabel("会话大纲")
                        .disabled(model.activeSessionID == nil)
                        .popover(isPresented: $conversation.outlinePresented, arrowEdge: .bottom) {
                            NativeConversationOutlineView(conversation: conversation)
                        }
                    Button { sharePresented = true } label: { WispIcon(name: "share") }
                        .buttonStyle(.plain).help("分享").accessibilityLabel("分享")
                        .disabled(model.activeSessionID == nil)
                    Button { trajectoryPresented = true } label: { WispIcon(name: "timeline") }
                        .buttonStyle(.plain).help("运行轨迹").accessibilityLabel("运行轨迹")
                        .disabled(model.activeSessionID == nil)
                    Button { archivePresented = true } label: { WispIcon(name: "archive") }
                        .buttonStyle(.plain).help("研究归档").accessibilityLabel("研究归档")
                        .disabled(model.activeSessionID == nil || conversation.snapshot?.running == true)
                    Button { inboxPresented.toggle(); refreshInbox() } label: {
                        HStack(spacing: 2) { WispIcon(name: "bell"); if !inbox.entries.isEmpty { Text("\(inbox.entries.count)").font(.caption2) } }
                    }.buttonStyle(.plain).help("待查看").accessibilityLabel("待查看")
                        .popover(isPresented: $inboxPresented, arrowEdge: .bottom) {
                            NativeInboxView(inbox: inbox, refresh: refreshInbox, open: { entry in
                                inboxPresented = false
                                Task { await model.openProject(entry.project_id, sessionID: entry.id) }
                            }, close: { inboxPresented = false })
                        }
                    Button { terminalVisible.toggle() } label: { WispIcon(name: "terminal") }
                        .buttonStyle(.plain).help("终端").accessibilityLabel("终端").disabled(model.activeSessionID == nil)
                    Button { panelVisible.toggle() } label: { WispIcon(name: "panel") }
                        .buttonStyle(.plain).help("切换侧面板").accessibilityLabel("切换侧面板").disabled(model.activeSessionID == nil)
                }
                .padding(16)
                Rectangle().fill(color("border")).frame(height: 1)
                if let error = model.sessionError {
                    HStack {
                        Text(error).font(WispDesign.font(size: 12)).textSelection(.enabled)
                        Button("重试") { Task { await model.openProject(project.id, sessionID: model.activeSessionID) } }
                    }.padding().foregroundStyle(.orange)
                }
                if publication.presented && publication.projectID == project.id {
                    NativePublicationColumn(model: model, publication: publication)
                } else if let session = model.activeSessionID {
                    NativeConversationView(conversation: conversation, projectID: project.id, sessionID: session) { selection in
                        guard model.activeProjectID == project.id, model.activeSessionID == session else { return }
                        model.nativeSideChat(projectID: project.id, sessionID: session).quotes.append(.init(text: selection, source: "会话摘录"))
                        var tabs = NativePanelTabs(saved: panelTabs, selected: panelTab, available: NativePanelTabs.all)
                        tabs.show("sidechat"); panelTabs = tabs.saved; panelTab = tabs.selected; panelVisible = true
                    }
                        .task(id: project.id + ":" + session) {
                            await conversation.open(project: project.id, session: session)
                            await conversation.loadSavedHighlights(project: project.id, session: session)
                        }
                        .onDisappear { conversation.pause() }
                } else {
                    VStack(spacing: 16) {
                        Text("开始新的研究对话").font(WispDesign.font(size: 22, weight: .semibold))
                        Text(conversation.operationError ?? "创建会话后即可选择模型并发送消息。").foregroundStyle(color("text-muted"))
                        Button("新建会话") { createSession() }.buttonStyle(WispButtonStyle(primary: true)).disabled(conversation.busy)
                    }.frame(maxWidth: .infinity, maxHeight: .infinity)
                }
                if terminalVisible, let session = model.activeSessionID {
                    Rectangle().fill(color("border")).frame(height: 5)
                        .gesture(DragGesture().onChanged { value in
                            if terminalDragStart == nil { terminalDragStart = terminalHeight }
                            terminalHeight = min(600, max(220, (terminalDragStart ?? 300) - value.translation.height))
                        }.onEnded { _ in terminalDragStart = nil })
                    NativeTerminalPanel(model: model.nativeTerminal(projectID: project.id, sessionID: session)) { terminalVisible = false }
                        .frame(height: terminalHeight).id(project.id + ":" + session)
                }
            }
            if panelVisible, let session = model.activeSessionID {
                Rectangle().fill(color("border")).frame(width: 5)
                    .gesture(DragGesture().onChanged { value in
                        if panelDragStart == nil { panelDragStart = panelWidth }
                        panelWidth = min(600, max(280, (panelDragStart ?? 340) - value.translation.width))
                    }.onEnded { _ in panelDragStart = nil })
                NativePanelView(client: conversation.client, projectID: project.id, sessionID: session, highlightRevision: conversation.savedHighlightRevision, highlightRemoved: { id in conversation.removeSavedHighlight(id, project: project.id, session: session) }, sideChat: model.nativeSideChat(projectID: project.id, sessionID: session), transcript: conversation.visibleItems, transcriptPage: conversation.showingHistory ? "history:\(conversation.history?.next_before_seq.map(String.init) ?? "start")" : "latest", revealExcerpt: conversation.revealExcerpt, readOnly: conversation.snapshot?.read_only ?? true, manageWorkflows: model.openWorkflowSettings, openTerminal: { context in
                    guard model.activeProjectID == project.id, model.activeSessionID == session else { return }
                    model.nativeTerminal(projectID: project.id, sessionID: session).requestOpen(context)
                    terminalVisible = true
                }) { panelVisible = false }
                    .frame(width: panelWidth).id(project.id + ":" + session)
            }
        }
        .sheet(isPresented: $sharePresented) {
            if let session = model.activeSessionID {
                NativeShareView(client: conversation.client, projectID: project.id, sessionID: session) { sharePresented = false }.id(project.id + ":" + session)
            }
        }
        .sheet(isPresented: $archivePresented) {
            if let session = model.activeSessionID {
                NativeArchiveView(client: conversation.client, projectID: project.id, sessionID: session, workspace: project.workspaceDirectory, close: {
                    archivePresented = false
                    Task { await conversation.refresh() }
                }, continued: { id in
                    archivePresented = false
                    Task { await model.openProject(project.id, sessionID: id) }
                }).id(project.id + ":" + session)
            }
        }
        .sheet(isPresented: $trajectoryPresented) {
            if let session = model.activeSessionID {
                NativeTrajectoryView(client: conversation.client, projectID: project.id, sessionID: session, running: conversation.snapshot?.running == true) { trajectoryPresented = false }
                    .id(project.id + ":" + session)
            }
        }
        .onChange(of: model.activeSessionID) { _ in trajectoryPresented = false; archivePresented = false; sharePresented = false; inboxPresented = false }
        .task(id: project.id) { await groups.load(conversation.client, projectID: project.id) }
        .sheet(isPresented: $groups.creating) {
            VStack(alignment: .leading, spacing: 12) {
                Text("新建分组").font(.headline)
                TextField("分组名称", text: $groups.draft).textFieldStyle(.roundedBorder).disabled(groups.busy)
                if let error = groups.error { Text(error).font(.caption).foregroundStyle(.red) }
                HStack {
                    Spacer()
                    Button("取消") { groups.dismissCreate() }.disabled(groups.busy)
                    Button(groups.busy ? "正在创建…" : "创建") { Task { await groups.create(conversation.client, projectID: project.id) } }
                        .disabled(groups.busy)
                }
            }
            .padding(24).frame(width: 360)
            .interactiveDismissDisabled(groups.busy)
            .background(NativeSettingsEscape(enabled: !groups.busy) { groups.dismissCreate() })
        }
        .sheet(isPresented: Binding(get: { model.journey.presented && model.journey.projectID == project.id }, set: { if !$0 { model.journey.presented = false; model.journeyFocus = nil } })) {
            NativeJourneySheet(model: model, journey: model.journey)
        }
        .task(id: project.id + "-inbox") {
            inbox.reset()
            while !Task.isCancelled {
                await inbox.refresh(client: conversation.client, projectID: project.id)
                do { try await Task.sleep(nanoseconds: 20_000_000_000) } catch { return }
            }
        }
        .background(color("bg-app")).foregroundStyle(color("text")).tint(color("clay"))
    }

    private func refreshInbox() { Task { await inbox.refresh(client: conversation.client, projectID: project.id) } }

    private func moveSessions(_ folderID: String?) async {
        await groups.moveSelected(conversation.client, projectID: project.id, folderID: folderID)
        await model.openProject(project.id, sessionID: model.activeSessionID)
    }

    private func createSession() {
        let database = model.databaseURL; let sourceSession = model.activeSessionID
        Task { if let id = await conversation.create(project: project.id) { await model.openNativeDraft(id, projectID: project.id, database: database, sourceSession: sourceSession) } }
    }

    private var sidebar: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(spacing: 8) {
                Button { model.goHome() } label: { WispIcon(name: "arrow-left", size: 18) }
                    .buttonStyle(.plain).help("返回项目").accessibilityLabel("返回项目")
                    .accessibilityIdentifier("back-projects")
                Menu {
                    Button("项目设置") { model.openProjectSettings(project.id) }
                    Divider()
                    ForEach(model.projects) { item in
                        Button(item.name) { Task { await model.openProject(item.id) } }
                    }
                } label: { Text(project.name).font(WispDesign.font(size: 14, weight: .semibold)).lineLimit(1) }
                .menuStyle(.borderlessButton).accessibilityLabel("切换项目")
                Button { sidebarVisible = false } label: { WispIcon(name: "chevron-left", size: 16) }
                    .buttonStyle(.plain).help("收起侧边栏").accessibilityLabel("收起侧边栏")
            }
            VStack(spacing: 4) {
                Button { createSession() } label: { HStack { WispIcon(name: "plus", size: 16); Text("新建会话"); Spacer() } }.buttonStyle(WispButtonStyle(primary: true)).disabled(conversation.busy)
                Button { model.searchPresented = true } label: {
                    HStack { WispIcon(name: "search", size: 16); Text("搜索"); Spacer() }
                }.buttonStyle(WispButtonStyle(compact: true))
                Button { groups.creating = true } label: {
                    HStack { WispIcon(name: "folder-plus", size: 16); Text("新建分组"); Spacer() }
                }
                .buttonStyle(WispButtonStyle(compact: true))
                .accessibilityLabel("新建分组")
                .accessibilityIdentifier("new-session-group")
                Button {
                    let revealed = NativePanelTabs.revealFiles(saved: panelTabs, selected: panelTab)
                    panelTabs = revealed.saved
                    panelTab = revealed.selected
                    panelVisible = revealed.visible
                } label: {
                    HStack { WispIcon(name: "doc", size: 16); Text("文件"); Spacer() }
                }
                .buttonStyle(WispButtonStyle(compact: true))
                .help("文件")
                .accessibilityLabel("文件")
                .accessibilityIdentifier("sidebar-files")
                Button { model.journey.open(projectID: project.id, day: model.journeyFocus?.projectID == project.id ? model.journeyFocus?.day : nil) } label: {
                    HStack { WispIcon(name: "research-trail", size: 16); Text("研究历程"); Spacer() }
                }
                .buttonStyle(WispButtonStyle(compact: true))
                .help("研究历程")
                .accessibilityLabel("研究历程")
                .accessibilityIdentifier("sidebar-journey")
                Button { publication.open(projectID: project.id) } label: {
                    HStack { WispIcon(name: "book", size: 16); Text("论文证据"); Spacer() }
                }
                .buttonStyle(WispButtonStyle(compact: true))
                .help("论文证据")
                .accessibilityLabel("论文证据")
                .accessibilityIdentifier("sidebar-publication")
                Button { model.library.presented = true } label: {
                    HStack { WispIcon(name: "star", size: 16); Text("收藏"); Spacer() }
                }
                .buttonStyle(WispButtonStyle(compact: true))
                .help("收藏")
                .accessibilityLabel("收藏")
                .accessibilityIdentifier("sidebar-library")
            }
            HStack {
                Text("会话").font(WispDesign.font(size: 11, weight: .semibold)).foregroundStyle(color("text-faint"))
                Spacer()
                Button(groups.selecting ? "取消" : "选择") {
                    groups.selecting.toggle()
                    if !groups.selecting { groups.selected = [] }
                }
                .buttonStyle(.plain).font(WispDesign.font(size: 12)).disabled(model.sessions.isEmpty)
                .accessibilityLabel(groups.selecting ? "取消选择" : "选择")
                Button { groups.menuPresented = true } label: { WispIcon(name: "adjustments", size: 16) }
                    .buttonStyle(.plain).help("排序与分组").accessibilityLabel("排序与分组")
            }
            if groups.menuPresented {
                VStack(alignment: .leading, spacing: 6) {
                    Text("排序").font(WispDesign.font(size: 11, weight: .semibold))
                    Button("最近") { groups.sort = "newest"; groups.menuPresented = false }.buttonStyle(.plain)
                    Button("名称") { groups.sort = "name"; groups.menuPresented = false }.buttonStyle(.plain)
                    Text("分组").font(WispDesign.font(size: 11, weight: .semibold))
                    Button("不分组") { groups.group = "none"; groups.menuPresented = false }.buttonStyle(.plain)
                    Button("按分组") { groups.group = "folder"; groups.menuPresented = false }.buttonStyle(.plain)
                    Button("按日期") { groups.group = "date"; groups.menuPresented = false }.buttonStyle(.plain)
                }
                .padding(10).background(color("bg-elev"), in: RoundedRectangle(cornerRadius: 8))
                .background(NativeSettingsEscape { groups.menuPresented = false })
            }
            if groups.selecting && !groups.selected.isEmpty {
                Menu("移到分组") {
                    Button("未分组") { Task { await moveSessions(nil) } }
                    ForEach(groups.folders) { folder in
                        Button(folder.name) { Task { await moveSessions(folder.id) } }
                    }
                }.font(WispDesign.font(size: 12))
            }
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 4) {
                    ForEach(groups.sections(model.sessions)) { section in
                        if groups.group != "none" {
                            Text(section.title).font(WispDesign.font(size: 11, weight: .semibold)).foregroundStyle(color("text-faint")).padding(.top, 6)
                        }
                        ForEach(section.sessions) { session in
                            Button {
                                if groups.selecting {
                                    if groups.selected.contains(session.id) { groups.selected.remove(session.id) } else { groups.selected.insert(session.id) }
                                } else {
                                    Task { await model.openSession(session.id) }
                                }
                            } label: {
                                HStack {
                                    Text(session.title).font(WispDesign.font(size: 13)).lineLimit(1)
                                    Spacer()
                                }
                                .padding(10)
                                .background(groups.selected.contains(session.id) ? color("clay").opacity(0.18) : (session.id == model.activeSessionID ? color("surface-hover") : .clear), in: RoundedRectangle(cornerRadius: 8))
                            }
                            .buttonStyle(.plain).accessibilityIdentifier("session-\(session.id)")
                            .accessibilityAddTraits(session.id == model.activeSessionID ? [.isSelected] : [])
                        }
                    }
                }
            }
            Spacer(minLength: 0)
            VStack(spacing: 4) {
                Button { model.capabilities.open(projectID: project.id) } label: {
                    HStack { WispIcon(name: "grid", size: 16); Text("能力"); Spacer() }
                }
                .buttonStyle(WispButtonStyle(compact: true))
                .help("能力")
                .accessibilityLabel("能力")
                .accessibilityIdentifier("sidebar-capabilities")
                WispUnavailableAction(title: "反馈问题", icon: "chat", expanded: true)
                Button { model.settingsPresented = true } label: { HStack { WispIcon(name: "gear"); Text("设置"); Spacer() } }.buttonStyle(WispButtonStyle())
            }
            Text(project.workspaceDirectory).font(WispDesign.font(size: 10)).lineLimit(1).truncationMode(.head)
                .foregroundStyle(color("text-faint")).help(project.workspaceDirectory)
        }
        .padding(16).background(color("bg-sunken"))
    }
}
