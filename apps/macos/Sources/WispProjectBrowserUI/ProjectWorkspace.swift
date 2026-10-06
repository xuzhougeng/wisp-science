import SwiftUI
import WispProjectBrowser

/// Native counterpart of the WebView's project shell: project navigation and
/// saved sessions on the left, the selected conversation in the main pane.
struct ProjectWorkspace: View {
    @ObservedObject var model: ProjectBrowserModel
    let project: ProjectSummary
    @ObservedObject private var conversation: NativeConversationModel
    @ObservedObject private var publication: NativePublicationModel
    @ObservedObject private var journey: NativeJourneyModel
    init(model: ProjectBrowserModel, project: ProjectSummary) {
        self.model = model; self.project = project
        self.conversation = model.nativeConversation()
        self.publication = model.publication
        self.journey = model.journey
    }
    @Environment(\.colorScheme) private var scheme
    @State private var sidebarVisible = true
    @State private var trajectoryPresented = false
    @State private var archivePresented = false
    @State private var sharePresented = false
    @State private var sessionImportPresented = false
    @State private var externalImportPresented = false
    @State private var sessionTransfer: NativeSessionTransferTarget?
    @State private var sessionRelations: BrowserSession?
    @State private var sessionExport: BrowserSession?
    @State private var terminalVisible = false
    @AppStorage("native.workspace.panel.visible") private var panelVisible = false
    @AppStorage("native.workspace.panel.tab") private var panelTab = "artifacts"
    @AppStorage("native.workspace.panel.tabs") private var panelTabs = ""
    @State private var panelWidth: CGFloat = 380
    @State private var panelDragStart: CGFloat?
    @State private var terminalHeight: CGFloat = 300
    @State private var terminalDragStart: CGFloat?
    @State private var inboxPresented = false
    @StateObject private var inbox = NativeInboxModel()
    @StateObject private var groups = NativeSessionGroups()
    @StateObject private var sessionRename = NativeSessionRename()
    @StateObject private var sessionPin = NativeSessionPin()
    @StateObject private var sessionDelete = NativeSessionDelete()
    @State private var sessionManagementError: String?
    private var selectedSession: BrowserSession? { model.sessions.first { $0.id == model.activeSessionID } }
    private func color(_ token: String) -> Color { WispDesign.color(token, scheme) }

    var body: some View {
        GeometryReader { geometry in
        let readingResearch = (publication.presented && publication.projectID == project.id) || (journey.presented && journey.projectID == project.id)
        let showsSidebar = sidebarVisible && (!panelVisible || readingResearch || geometry.size.width >= 960)
        HStack(spacing: 0) {
            if showsSidebar {
                sidebar.frame(width: 248)
                Rectangle().fill(color("border")).frame(width: 1)
            }
            VStack(spacing: 0) {
                if readingResearch && !showsSidebar {
                    HStack {
                        Button("展开侧边栏") { sidebarVisible = true }
                        Spacer()
                    }.padding(.horizontal, 20).padding(.top, 8)
                }
                if !readingResearch {
                HStack(spacing: 8) {
                    if !showsSidebar {
                        Button { if geometry.size.width < 960 { panelVisible = false }; sidebarVisible = true } label: { WispIcon(name: "chevron-right") }
                            .buttonStyle(.plain).help("展开侧边栏").accessibilityLabel("展开侧边栏")
                    }
                    Text(model.sessions.first(where: { $0.id == model.activeSessionID })?.title ?? project.name)
                        .font(WispDesign.font(size: 14)).lineLimit(1)
                        .padding(.horizontal, 14).padding(.vertical, 8)
                        .background(color("bg-elev"), in: RoundedRectangle(cornerRadius: 9))
                        .overlay(RoundedRectangle(cornerRadius: 9).strokeBorder(color("border")))
                    Menu {
                    Button {
                        if let session = model.sessions.first(where: { $0.id == model.activeSessionID }) { sessionRename.begin(session) }
                    } label: { Label { Text("重命名会话") } icon: { WispIcon(name: "edit", size: 14) } }
                        .buttonStyle(.plain).help("重命名会话").accessibilityLabel("重命名会话")
                        .disabled(model.activeSessionID == nil || conversation.snapshot?.read_only == true)
                    Button {
                        guard let session = selectedSession else { return }
                        let database = model.databaseURL
                        Task {
                            if await sessionPin.toggle(session, client: conversation.client),
                               model.databaseURL == database, model.activeProjectID == session.projectID,
                               model.activeSessionID == session.id {
                                await model.refreshSessionMetadata(projectID: session.projectID, sessionID: session.id, database: database)
                            }
                        }
                    } label: { Label { Text(selectedSession?.pinned == true ? "取消置顶会话" : "置顶会话") } icon: { WispIcon(name: "pin", size: 14) } }
                        .buttonStyle(.plain)
                        .help(selectedSession?.pinned == true ? "取消置顶会话" : "置顶会话")
                        .accessibilityLabel(selectedSession?.pinned == true ? "取消置顶会话" : "置顶会话")
                        .disabled(selectedSession?.pinned == nil || sessionPin.busy || model.sessionsLoading || conversation.snapshot?.read_only == true)
                    Button { if let session = selectedSession { sessionDelete.begin([session]) } } label: { Label { Text("删除会话") } icon: { WispIcon(name: "trash", size: 14) } }
                        .buttonStyle(.plain).help("删除会话").accessibilityLabel("删除会话")
                        .disabled(selectedSession == nil || sessionDelete.busy || conversation.snapshot?.read_only == true)
                    Button { beginTransfer(.copy) } label: { Label { Text(localized("复制到其他项目…")) } icon: { WispIcon(name: "copy", size: 14) } }
                        .disabled(!canTransfer(.copy))
                    Button { beginTransfer(.move) } label: { Label { Text(localized("移动到其他项目…")) } icon: { WispIcon(name: "conversation-move", size: 14) } }
                        .disabled(!canTransfer(.move)).help(localized("移动前请清空输入中的附件和引用；文字草稿会保留。"))
                    Button { sessionRelations = selectedSession } label: { Label { Text(localized("会话关系")) } icon: { WispIcon(name: "fork", size: 14) } }
                        .disabled(selectedSession == nil || model.sessionsLoading)
                    Button { if canExport { sessionExport = selectedSession } } label: { Label { Text(localized("导出会话 ZIP")) } icon: { WispIcon(name: "archive-export", size: 14) } }
                        .disabled(!canExport)
                    } label: { WispIcon(name: "more", size: 16) }
                    .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().tint(color("text-muted")).accessibilityLabel("会话操作")
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
                .padding(.horizontal, 16).frame(height: 64)
                Rectangle().fill(color("border")).frame(height: 1)
                }
                if let error = model.sessionError {
                    HStack {
                        Text(error).font(WispDesign.font(size: 12)).textSelection(.enabled)
                        Button("重试") { Task { await model.openProject(project.id, sessionID: model.activeSessionID) } }
                    }.padding().foregroundStyle(.orange)
                }
                if let error = sessionPin.error {
                    HStack {
                        Text(error).font(WispDesign.font(size: 12)).textSelection(.enabled)
                        Button("刷新会话") {
                            guard let session = model.activeSessionID else { return }
                            let database = model.databaseURL
                            Task {
                                if await model.refreshSessionMetadata(projectID: project.id, sessionID: session, database: database) { sessionPin.reset() }
                            }
                        }.disabled(model.sessionsLoading)
                    }.padding().foregroundStyle(.orange)
                }
                if let error = sessionManagementError {
                    HStack {
                        Text(error).font(WispDesign.font(size: 12)).textSelection(.enabled)
                        Button("刷新会话") {
                            let database = model.databaseURL; let session = model.activeSessionID
                            Task {
                                guard model.databaseURL == database, model.activeProjectID == project.id, model.activeSessionID == session else { return }
                                await model.openProject(project.id, sessionID: session)
                                if model.databaseURL == database, model.activeProjectID == project.id, model.sessionError == nil { sessionManagementError = nil }
                            }
                        }.disabled(model.sessionsLoading)
                    }.padding().foregroundStyle(.orange)
                }
                if publication.presented && publication.projectID == project.id {
                    NativePublicationColumn(model: model, publication: publication)
                } else if journey.presented && journey.projectID == project.id {
                    NativeJourneyPage(model: model, journey: journey)
                } else if let session = model.activeSessionID {
                    NativeConversationView(conversation: conversation, projectID: project.id, sessionID: session, createAcpConversation: { agent in createSession(acpAgentID: agent) }, executeComposerCommand: { command, payload in
                        guard model.activeProjectID == project.id, model.activeSessionID == session else { return }
                        switch command {
                        case .archive: archivePresented = true
                        case .share: sharePresented = true
                        case .trajectory: trajectoryPresented = true
                        case .skills: model.projectSettingsID = nil; model.settingsSectionID = "skills"; model.settingsPresented = true
                        case .files, .btw:
                            var tabs = NativePanelTabs(saved: panelTabs, selected: panelTab, available: NativePanelTabs.all)
                            tabs.show(command == .files ? "files" : "sidechat"); panelTabs = tabs.saved; panelTab = tabs.selected; panelVisible = true
                            if command == .btw && !payload.isEmpty {
                                let sideChat = model.nativeSideChat(projectID: project.id, sessionID: session)
                                if sideChat.draft.isEmpty && !sideChat.busy {
                                    sideChat.draft = payload
                                    Task { await sideChat.send() }
                                } else {
                                    sideChat.draft += (sideChat.draft.isEmpty ? "" : "\n") + payload
                                }
                            }
                        case .upload:
                            let picker = NSOpenPanel(); picker.canChooseFiles = true; picker.canChooseDirectories = false; picker.allowsMultipleSelection = true
                            guard picker.runModal() == .OK, model.activeProjectID == project.id, model.activeSessionID == session else { return }
                            let paths = picker.urls.map(\.path)
                            Task {
                                for path in paths {
                                    guard model.activeProjectID == project.id, model.activeSessionID == session else { return }
                                    await conversation.attach(source: path, client: conversation.client)
                                }
                            }
                        }
                    }, openHistoryBranch: { branchID in
                        guard model.activeProjectID == project.id, model.activeSessionID == session else { return }
                        Task { await model.openProject(project.id, sessionID: branchID) }
                    }) { selection in
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
                if terminalVisible && !readingResearch, let session = model.activeSessionID {
                    Rectangle().fill(color("border")).frame(height: 5)
                        .gesture(DragGesture().onChanged { value in
                            if terminalDragStart == nil { terminalDragStart = terminalHeight }
                            terminalHeight = min(600, max(220, (terminalDragStart ?? 300) - value.translation.height))
                        }.onEnded { _ in terminalDragStart = nil })
                    NativeTerminalPanel(model: model.nativeTerminal(projectID: project.id, sessionID: session)) { terminalVisible = false }
                        .frame(height: terminalHeight).id(project.id + ":" + session)
                }
            }
            if panelVisible && !readingResearch, let session = model.activeSessionID {
                Rectangle().fill(color("border")).frame(width: 5)
                    .gesture(DragGesture().onChanged { value in
                        if panelDragStart == nil { panelDragStart = panelWidth }
                        panelWidth = min(600, max(280, (panelDragStart ?? 380) - value.translation.width))
                    }.onEnded { _ in panelDragStart = nil })
                NativePanelView(client: conversation.client, projectID: project.id, sessionID: session, projectRoot: project.workspaceDirectory, highlightRevision: conversation.savedHighlightRevision, highlightRemoved: { id in conversation.removeSavedHighlight(id, project: project.id, session: session) }, sideChat: model.nativeSideChat(projectID: project.id, sessionID: session), transcript: conversation.visibleItems, transcriptPage: conversation.showingHistory ? "history:\(conversation.history?.next_before_seq.map(String.init) ?? "start")" : "latest", revealExcerpt: conversation.revealExcerpt, readOnly: conversation.snapshot?.read_only ?? true, manageWorkflows: model.openWorkflowSettings, openTerminal: { context in
                    guard model.activeProjectID == project.id, model.activeSessionID == session else { return }
                    model.nativeTerminal(projectID: project.id, sessionID: session).requestOpen(context)
                    terminalVisible = true
                }) { panelVisible = false }
                    .frame(width: Self.panelWidth(preferred: panelWidth, available: geometry.size.width, sidebar: showsSidebar)).id(project.id + ":" + session)
            }
        }
        .onChange(of: model.workspaceCommand?.id) { _ in
            guard let command = model.workspaceCommand, command.project == project.id,
                  command.session == model.activeSessionID else { return }
            switch command.action {
            case "new-session": if !conversation.busy { createSession() }
            case "import-session-archive": sessionImportPresented = true
            case "import-external-session": externalImportPresented = true
            case "copy-session-project": beginTransfer(.copy)
            case "move-session-project": beginTransfer(.move)
            case "session-relations": sessionRelations = selectedSession
            case "export-session": if canExport { sessionExport = selectedSession }
            case "toggle-sidebar": sidebarVisible.toggle()
            case "terminal": terminalVisible.toggle()
            case "close-panel": panelVisible = false
            default:
                guard NativePanelTabs.all.contains(command.action), model.activeSessionID != nil else { return }
                model.returnToConversation()
                var tabs = NativePanelTabs(saved: panelTabs, selected: panelTab, available: NativePanelTabs.all)
                tabs.show(command.action); panelTabs = tabs.saved; panelTab = tabs.selected; panelVisible = true
            }
        }
        }
        .sheet(isPresented: $sharePresented) {
            if let session = model.activeSessionID {
                NativeShareView(client: conversation.client, projectID: project.id, sessionID: session) { sharePresented = false }.id(project.id + ":" + session)
            }
        }
        .sheet(isPresented: $sessionImportPresented) {
            let database = model.databaseURL; let sourceSession = model.activeSessionID
            NativeSessionArchiveImportSheet(client: conversation.client, project: project.id, projects: model.projects, writable: {
                model.databaseURL == database && model.activeProjectID == project.id && model.activeSessionID == sourceSession
            }, close: { sessionImportPresented = false }, open: { destination, session in
                guard model.databaseURL == database, model.activeProjectID == project.id, model.activeSessionID == sourceSession else { return }
                sessionImportPresented = false; model.returnToConversation()
                Task { await model.openProject(destination, sessionID: session) }
            }).id(database.path + ":" + project.id)
        }
        .sheet(isPresented: $externalImportPresented) {
            let database = model.databaseURL; let sourceSession = model.activeSessionID
            NativeExternalSessionImportSheet(client: conversation.client, project: project.id, projects: model.projects, writable: {
                model.databaseURL == database && model.activeProjectID == project.id && model.activeSessionID == sourceSession
            }, close: { externalImportPresented = false }, open: { destination, session in
                guard model.databaseURL == database, model.activeProjectID == project.id, model.activeSessionID == sourceSession else { return }
                externalImportPresented = false; model.returnToConversation()
                Task { await model.openProject(destination, sessionID: session) }
            }).id(database.path + ":" + project.id)
        }
        .sheet(item: $sessionRelations) { selected in
            let database = model.databaseURL
            NativeSessionRelationsSheet(selected: selected, sessions: model.sessions, close: { sessionRelations = nil }, open: { id in
                guard model.databaseURL == database, model.activeProjectID == selected.projectID,
                      model.activeSessionID == selected.id,
                      NativeSessionRelations(selected: selected, sessions: model.sessions).canOpen(id) else { return }
                sessionRelations = nil
                model.returnToConversation()
                Task {
                    guard model.databaseURL == database, model.activeProjectID == selected.projectID,
                          model.activeSessionID == selected.id,
                          NativeSessionRelations(selected: selected, sessions: model.sessions).canOpen(id) else { return }
                    await model.openSession(id)
                }
            }).id(database.path + ":" + selected.projectID + ":" + selected.id)
        }
        .sheet(item: $sessionExport) { source in
            let database = model.databaseURL
            NativeSessionExportSheet(client: conversation.client, source: source, writable: {
                model.databaseURL == database && model.activeProjectID == source.projectID
                    && model.activeSessionID == source.id && canExport
            }, close: { sessionExport = nil }).id(database.path + ":" + source.projectID + ":" + source.id)
        }
        .sheet(item: $sessionTransfer) { target in
            let database = model.databaseURL
            NativeSessionTransferSheet(client: conversation.client, source: target.source, mode: target.mode, projects: model.projects, writable: {
                model.databaseURL == database && model.activeProjectID == target.source.projectID
                    && model.activeSessionID == target.source.id && canTransfer(target.mode)
            }, close: { result in
                guard model.databaseURL == database, model.activeProjectID == target.source.projectID,
                      model.activeSessionID == target.source.id else { return }
                sessionTransfer = nil
                if let result, result.mode == .move { model.removeConfirmedSessions([result.session_id], projectID: result.project_id, database: database) }
            }, open: { result in
                guard model.databaseURL == database, model.activeProjectID == target.source.projectID,
                      model.activeSessionID == target.source.id else { return }
                sessionTransfer = nil
                if result.mode == .move { model.removeConfirmedSessions([result.session_id], projectID: result.project_id, database: database) }
                model.returnToConversation(); Task { await model.openProject(result.target_project_id, sessionID: result.frame_id) }
            }, transferred: { result in
                guard model.databaseURL == database, model.activeProjectID == target.source.projectID,
                      model.activeSessionID == target.source.id else { return }
                conversation.retainDraftForTransferredSession(result)
            }).id(database.path + ":" + target.id.uuidString)
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
        .onChange(of: model.activeSessionID) { _ in sessionRename.reset(); sessionPin.reset(); sessionDelete.reset(); sessionImportPresented = false; externalImportPresented = false; sessionTransfer = nil; sessionRelations = nil; sessionExport = nil }
        .onChange(of: project.id) { _ in sessionRename.reset(); sessionPin.reset(); sessionDelete.reset(); sessionManagementError = nil; sessionImportPresented = false; externalImportPresented = false; sessionTransfer = nil; sessionRelations = nil; sessionExport = nil }
        .onDisappear { sessionRename.reset(); sessionPin.reset(); sessionDelete.reset(); sessionManagementError = nil; sessionTransfer = nil; sessionRelations = nil; sessionExport = nil }
        .onChange(of: conversation.snapshot?.running) { running in
            guard running == false, !model.sessionsLoading, !sessionPin.busy,
                  let session = conversation.snapshot?.session_id else { return }
            let database = model.databaseURL
            Task {
                await model.refreshSessionMetadata(projectID: project.id, sessionID: session, database: database)
            }
        }
        .sheet(isPresented: Binding(get: { sessionRename.target != nil }, set: { if !$0 { sessionRename.dismiss() } })) {
            NativeSessionRenameSheet(model: sessionRename) {
                let database = model.databaseURL
                Task {
                    if let renamed = await sessionRename.save(conversation.client),
                       model.databaseURL == database, model.activeProjectID == renamed.projectID,
                       model.activeSessionID == renamed.id {
                        await model.refreshSessionMetadata(projectID: renamed.projectID, sessionID: renamed.id, database: database)
                    }
                }
            }
        }
        .sheet(isPresented: Binding(get: { !sessionDelete.targets.isEmpty }, set: { if !$0 { sessionDelete.dismiss() } })) {
            NativeSessionDeleteSheet(model: sessionDelete, confirm: deleteSessions)
        }
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
        .sheet(isPresented: Binding(get: { groups.renamingID != nil }, set: { if !$0 { groups.dismissRename() } })) {
            VStack(alignment: .leading, spacing: 12) {
                Text("重命名分组").font(.headline)
                TextField("分组名称", text: $groups.renameDraft).textFieldStyle(.roundedBorder).disabled(groups.busy)
                if let error = groups.error { Text(error).font(.caption).foregroundStyle(.red) }
                HStack {
                    Spacer()
                    Button("取消") { groups.dismissRename() }.disabled(groups.busy)
                    Button(groups.busy ? "正在重命名…" : "保存") { Task { await groups.rename(conversation.client, projectID: project.id) } }
                        .disabled(groups.busy)
                }
            }
            .padding(24).frame(width: 360)
            .interactiveDismissDisabled(groups.busy)
            .background(NativeSettingsEscape(enabled: !groups.busy) { groups.dismissRename() })
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

    private var canExport: Bool {
        guard let source = selectedSession, !model.sessionsLoading else { return false }
        return NativeSessionExportModel.eligible(source: source, snapshot: conversation.snapshot, busy: conversation.busy, queued: !conversation.queuedTurns.isEmpty || conversation.queuedFollowUp != nil)
    }

    private func canTransfer(_ mode: NativeSessionTransferMode) -> Bool {
        guard let session = selectedSession, let snapshot = conversation.snapshot,
              snapshot.project_id == session.projectID, snapshot.session_id == session.id,
              !snapshot.running, !conversation.busy, !session.projectID.hasPrefix("assistant:") else { return false }
        return mode == .copy || !snapshot.read_only && conversation.attachments.isEmpty && conversation.references.isEmpty
    }
    private func beginTransfer(_ mode: NativeSessionTransferMode) {
        guard canTransfer(mode), let source = selectedSession else { return }
        sessionTransfer = NativeSessionTransferTarget(source: source, mode: mode)
    }
    private func deleteSessions() {
        let database = model.databaseURL; let session = model.activeSessionID
        Task {
            let result = await sessionDelete.delete(conversation.client)
            model.noteUnconfirmedDeletion(result.unconfirmed, database: database)
            let stillHere = model.databaseURL == database && model.activeProjectID == project.id && model.activeSessionID == session
            if stillHere && result.isCurrent {
                sessionManagementError = result.error
                sessionDelete.dismiss()
                groups.selected.subtract(result.confirmed)
                if groups.selected.isEmpty { groups.selecting = false }
            }
            model.removeConfirmedSessions(result.confirmed, projectID: project.id, database: database)
        }
    }

    private func moveSessions(_ folderID: String?) async {
        await groups.moveSelected(conversation.client, projectID: project.id, folderID: folderID)
        await model.openProject(project.id, sessionID: model.activeSessionID)
    }

    static func panelWidth(preferred: CGFloat, available: CGFloat, sidebar: Bool) -> CGFloat {
        // Preserve a usable composer when a wide inspector is reopened in a
        // smaller window. The drag preference remains local to this workspace.
        max(220, min(preferred, 600, available - (sidebar ? 249 : 0) - 360))
    }
    private func createSession(acpAgentID: String? = nil) {
        model.returnToConversation()
        let database = model.databaseURL; let sourceSession = model.activeSessionID
        Task { if let id = await conversation.create(project: project.id, acpAgentID: acpAgentID) { await model.openNativeDraft(id, projectID: project.id, database: database, sourceSession: sourceSession) } }
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
                        Button(item.name) { model.returnToConversation(); Task { await model.openProject(item.id) } }
                    }
                } label: { Text(project.name).font(WispDesign.font(size: 14, weight: .semibold)).lineLimit(1) }
                .menuStyle(.borderlessButton).accessibilityLabel("切换项目")
                Button { sidebarVisible = false } label: { WispIcon(name: "chevron-left", size: 16) }
                    .buttonStyle(.plain).help("收起侧边栏").accessibilityLabel("收起侧边栏")
            }
            VStack(spacing: 0) {
                Button { createSession() } label: { HStack { WispIcon(name: "plus", size: 16); Text("新建会话"); Spacer() } }.buttonStyle(WispSidebarButtonStyle()).disabled(conversation.busy)
                Button { sessionImportPresented = true } label: {
                    HStack { WispIcon(name: "archive-import", size: 16); Text(localized("导入会话 ZIP 归档")); Spacer() }
                }.buttonStyle(WispSidebarButtonStyle()).accessibilityIdentifier("import-session-archive")
                Button { externalImportPresented = true } label: {
                    HStack { WispIcon(name: "conversation-import", size: 16); Text(localized("导入 Codex / Claude 会话")); Spacer() }
                }.buttonStyle(WispSidebarButtonStyle()).accessibilityIdentifier("import-external-session")
                Button { model.searchPresented = true } label: {
                    HStack { WispIcon(name: "search", size: 16); Text("搜索"); Spacer(); Text("⌘K").font(WispDesign.font(size: 11)).foregroundStyle(color("text-faint")) }
                }.buttonStyle(WispSidebarButtonStyle())
                Button { groups.creating = true } label: {
                    HStack { WispIcon(name: "folder-plus", size: 16); Text("新建分组"); Spacer() }
                }
                .buttonStyle(WispSidebarButtonStyle())
                .accessibilityLabel("新建分组")
                .accessibilityIdentifier("new-session-group")
                Button {
                    model.returnToConversation()
                    let revealed = NativePanelTabs.revealFiles(saved: panelTabs, selected: panelTab)
                    panelTabs = revealed.saved
                    panelTab = revealed.selected
                    panelVisible = revealed.visible
                } label: {
                    HStack { WispIcon(name: "doc", size: 16); Text("文件"); Spacer() }
                }
                .buttonStyle(WispSidebarButtonStyle())
                .help("文件")
                .accessibilityLabel("文件")
                .accessibilityIdentifier("sidebar-files")
                Button { publication.dismiss(); model.journey.open(projectID: project.id, day: model.journeyFocus?.projectID == project.id ? model.journeyFocus?.day : nil) } label: {
                    HStack { WispIcon(name: "research-trail", size: 16); Text("研究历程"); Spacer() }
                }
                .buttonStyle(WispSidebarButtonStyle())
                .help("研究历程")
                .accessibilityLabel("研究历程")
                .accessibilityIdentifier("sidebar-journey")
                Button { journey.dismiss(); publication.open(projectID: project.id) } label: {
                    HStack { WispIcon(name: "book", size: 16); Text("论文证据"); Spacer() }
                }
                .buttonStyle(WispSidebarButtonStyle())
                .help("论文证据")
                .accessibilityLabel("论文证据")
                .accessibilityIdentifier("sidebar-publication")
                Button { model.library.presented = true } label: {
                    HStack { WispIcon(name: "star", size: 16); Text("收藏"); Spacer() }
                }
                .buttonStyle(WispSidebarButtonStyle())
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
                Button {
                    sessionDelete.begin(model.sessions.filter { groups.selected.contains($0.id) })
                } label: { HStack { WispIcon(name: "trash", size: 14); Text("删除所选会话") } }
                    .buttonStyle(.plain).font(WispDesign.font(size: 12))
                    .disabled(sessionDelete.busy || model.sessionsLoading)
            }
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 4) {
                    ForEach(groups.sections(model.sessions)) { section in
                        if groups.group != "none" || model.sessions.contains(where: { $0.pinned == true }) {
                            HStack {
                                Text(section.title).font(WispDesign.font(size: 11, weight: .semibold)).foregroundStyle(color("text-faint"))
                                Spacer()
                                if let folderID = section.folderID {
                                    Button("重命名") { groups.beginRename(folderID) }
                                        .buttonStyle(.plain)
                                        .font(WispDesign.font(size: 11))
                                        .accessibilityLabel("重命名 \(section.title)")
                                        .accessibilityIdentifier("rename-group-\(folderID)")
                                }
                            }
                            .padding(.top, 6)
                        }
                        ForEach(section.sessions) { session in
                            Button {
                                if groups.selecting {
                                    if groups.selected.contains(session.id) { groups.selected.remove(session.id) } else { groups.selected.insert(session.id) }
                                } else {
                                    Task { model.returnToConversation(); await model.openSession(session.id) }
                                }
                            } label: {
                                HStack {
                                    if session.branchState != nil { WispIcon(name: "fork", size: 12).help(localized(NativeSessionRelations.stateLabel(session) ?? "分支")) }
                                    else if session.dispatchedFrom != nil { WispIcon(name: "chat", size: 12).help(localized("子代理会话")) }
                                    Text(session.title).font(WispDesign.font(size: 14)).lineLimit(1)
                                    Spacer()
                                }
                                .padding(10)
                                .background(groups.selected.contains(session.id) ? color("clay").opacity(0.18) : (session.id == model.activeSessionID ? color("bg-elev") : .clear), in: RoundedRectangle(cornerRadius: 8))
                            }
                            .buttonStyle(.plain).accessibilityIdentifier("session-\(session.id)")
                            .accessibilityLabel(session.title + (NativeSessionRelations.stateLabel(session).map { " · " + localized($0) } ?? ""))
                            .accessibilityAddTraits(groups.isSelected(session.id, activeSessionID: model.activeSessionID) ? [.isSelected] : [])
                        }
                    }
                }
            }
            Spacer(minLength: 0)
            VStack(spacing: 0) {
                Button { model.capabilities.open(projectID: project.id) } label: {
                    HStack { WispIcon(name: "grid", size: 16); Text("能力"); Spacer() }
                }
                .buttonStyle(WispSidebarButtonStyle())
                .help("能力")
                .accessibilityLabel("能力")
                .accessibilityIdentifier("sidebar-capabilities")
                Button { Task { await model.prepareIssueReport() } } label: {
                    HStack { WispIcon(name: "chat", size: 16); Text("反馈问题"); Spacer() }
                }
                .buttonStyle(WispSidebarButtonStyle())
                .disabled(!IssueReportDraft.offered(activeSessionID: model.activeSessionID))
                .help("反馈问题")
                .accessibilityLabel("反馈问题")
                .accessibilityIdentifier("sidebar-feedback")
                Button { model.settingsPresented = true } label: { HStack { WispIcon(name: "gear"); Text("设置"); Spacer() } }.buttonStyle(WispSidebarButtonStyle())
            }
            Text(project.workspaceDirectory).font(WispDesign.font(size: 10)).lineLimit(1).truncationMode(.head)
                .foregroundStyle(color("text-faint")).help(project.workspaceDirectory)
        }
        .padding(.horizontal, 10).padding(.vertical, 16).background(color("bg-sunken"))
    }
}
