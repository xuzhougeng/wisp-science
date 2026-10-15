import SwiftUI
import UIKit
import WispRemote

struct WorkspaceView: View {
    @ObservedObject var model: RemoteWorkspace
    @ObservedObject var connection: RemoteConnection
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.horizontalSizeClass) private var sizeClass
    // Start automatic: an explicit first value is overridden by the split view's own first pass.
    @State private var columns: NavigationSplitViewVisibility = .automatic
    @State private var columnFit = ColumnFit.all

    /// Regular-width iPad windows list the tools and projects in one sidebar, with
    /// the chosen section in the list and detail columns. iPhone and compact iPad
    /// windows (Slide Over, a narrow Split View side) keep the tab bar. The idiom
    /// check keeps a large iPhone from switching layouts when it rotates.
    private var usesSidebar: Bool { sizeClass == .regular && UIDevice.current.userInterfaceIdiom == .pad }
    private var paired: Bool { model.configuration != nil || model.isDemo }

    var body: some View {
        // Banners own layout space above the navigation container. A top
        // safeAreaInset on NavigationSplitView can overlap its compact toolbar.
        VStack(spacing: 0) {
            workspaceBanners
            if usesSidebar {
                // The journey page is part of the conversations section, so it
                // gives way when the sidebar switches to another tool.
                pageHost(page: model.tab == .conversations ? model.page : nil) { sidebarWorkspace }
            } else {
                tabWorkspace
            }
        }
        .onChange(of: model.availableTabs) { _, tabs in
            if !tabs.contains(model.tab) { model.tab = .conversations }
        }
        .onChange(of: model.tab) { _, _ in
            UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
        }
        .tint(WispStyle.accent)
        .sheet(item: $model.sheet) { sheet in
            switch sheet {
            case .connection: ConnectionSettingsView(model: model, connection: connection)
            case .inbox: InboxView(model: model)
            case .checklistSettings: ChecklistSettingsView(workspace: model, model: model.checklist, connection: connection)
            case .conversationInfo(let title, let project):
                NavigationStack {
                    Form {
                        Section("完整标题") { Text(title).textSelection(.enabled) }
                        if !project.isEmpty { Section("所属项目") { Text(project).textSelection(.enabled) } }
                    }
                    .navigationTitle("对话详情").navigationBarTitleDisplayMode(.inline)
                    .toolbar { ToolbarItem(placement: .topBarTrailing) { Button("完成") { model.sheet = nil } } }
                }.accessibilityElement(children: .contain).accessibilityIdentifier("conversation-info-sheet")
            case .transcriptDetail: TranscriptDetailSheet { model.sheet = nil }
            case .files(let route, let path, let directory):
                FilesView(model: model, route: route, initialPath: path, isDirectory: directory)
                    .modifier(PageSizedSheet())
            }
        }
        .task { model.restore() }
        .task(id: connection.state) {
            if connection.state == .online { await model.monitor() }
        }
        .task(id: connection.readSessionID) {
            model.checklist.configure(configuration: model.configuration, connection: connection, demo: model.isDemo)
            while !Task.isCancelled && connection.state == .online && !model.isDemo {
                await model.checklist.sync(connection: connection)
                do { try await Task.sleep(nanoseconds: 15_000_000_000) } catch { return }
            }
        }
        .task(id: model.selectedProject) { await model.refreshSelection() }
        .task(id: model.route) { await model.refreshSelection() }
    }

    // MARK: iPhone and compact windows

    private var tabWorkspace: some View {
        TabView(selection: $model.tab) {
            pageHost(page: model.page) { workspaceContent }
                .tabItem { WorkspaceTabLabel(tab: .conversations, connected: paired) }
                .tag(WorkspaceTab.conversations)
            if model.availableTabs.contains(.calendar) {
                ResearchView(workspace: model, connection: connection, project: nil)
                    .tabItem { WorkspaceTabLabel(tab: .calendar) }
                    .tag(WorkspaceTab.calendar)
            }
            if model.availableTabs.contains(.checklist) {
                ChecklistView(workspace: model, model: model.checklist, connection: connection)
                    .tabItem { WorkspaceTabLabel(tab: .checklist) }
                    .tag(WorkspaceTab.checklist)
            }
            if model.availableTabs.contains(.notebook) {
                NotebookView(workspace: model, model: model.notebook, connection: connection)
                    .tabItem { WorkspaceTabLabel(tab: .notebook) }
                    .tag(WorkspaceTab.notebook)
            }
        }
    }

    /// Projects, conversations and the project journey page over them.
    private var workspaceContent: some View {
        Group {
            if !paired {
                ConnectionView(model: model)
            } else {
                NavigationSplitView {
                    List(selection: $model.selectedProject) {
                        Section {
                            ForEach(model.projects, id: \.recordID) { project in
                                NavigationLink(value: project["id"].string) { ProjectRow(project: project) }
                            }
                        } header: { Text(model.hostName) }
                        if model.projects.isEmpty { emptyProjects }
                    }
                    .navigationTitle("Wisp")
                    .navigationBarTitleDisplayMode(.inline)
                    .navigationSplitViewColumnWidth(min: 210, ideal: 250, max: 340)
                    .toolbar { workspaceToolbar }
                    .refreshable { await model.refreshSelection() }
                } content: {
                    sessionsColumn
                } detail: {
                    conversationDetail
                }
                .safeAreaInset(edge: .bottom, spacing: 0) { offlineStatus }
            }
        }
    }

    // MARK: Regular-width iPad windows

    /// One sidebar for the workspace sections and the host's projects. A section
    /// fills the list and detail columns, so no floating tab bar sits over them.
    private var sidebarWorkspace: some View {
        GeometryReader { geometry in
            let fit = ColumnFit(width: geometry.size.width)
            sidebarColumns
                .onAppear { columnFit = fit; columns = preferredColumns(fit) }
                .onChange(of: fit) { _, fit in columnFit = fit; columns = preferredColumns(fit) }
                // Narrower windows follow what is open; wide ones keep the user's column choice.
                .onChange(of: preferredColumns(fit)) { _, visibility in if fit != .all { columns = visibility } }
                .onChange(of: model.sidebarItem) { _, _ in if fit != .all { columns = preferredColumns(fit) } }
                // Unpaired, the list column would repeat the connection form shown in
                // the detail; its toolbar button opens the sidebar instead.
                .onChange(of: columns) { _, visibility in
                    if fit != .all && !paired && model.tab == .conversations && visibility == .doubleColumn { columns = .all }
                }
                .onReceive(model.researchCalendar.$day.dropFirst()) { _ in
                    if fit == .overlaid && model.tab == .calendar { columns = .detailOnly }
                }
        }
    }

    /// Wide windows show every column. In portrait the list keeps its detail
    /// beside it and the sidebar folds behind its toolbar button. Narrower
    /// windows lay the list over its detail, so an open conversation, item or
    /// record takes the full width until the toolbar button brings the list back,
    /// as in Mail. Before pairing, the connection form fills the detail column.
    private func preferredColumns(_ fit: ColumnFit) -> NavigationSplitViewVisibility {
        if fit == .all { return .all }
        let open: Bool
        switch model.tab {
        case .conversations:
            if !paired { return .detailOnly }
            if model.selectedProject == nil { return .all }
            open = model.route != nil
        case .calendar: open = !model.calendarPath.isEmpty
        case .checklist: open = !model.checklistPath.isEmpty
        case .notebook: open = !model.notebookPath.isEmpty
        }
        return fit == .overlaid && open ? .detailOnly : .doubleColumn
    }

    private var sidebarColumns: some View {
        NavigationSplitView(columnVisibility: $columns) {
            List(selection: $model.sidebarItem) {
                Section {
                    // Paired, the projects below stand for the conversations section.
                    ForEach(model.availableTabs.filter { $0 != .conversations || !paired }, id: \.self) { section in
                        NavigationLink(value: WorkspaceSidebarItem.section(section)) {
                            // The sidebar tints the icon, including white on a focused selection.
                            Label { Text(model.sectionTitle(section)) } icon: {
                                WispIcon(WorkspaceTabLabel.iconKind(section, connected: paired))
                            }
                        }
                        .accessibilityIdentifier("sidebar-\(section.rawValue)")
                    }
                }
                if paired {
                    Section {
                        ForEach(model.projects, id: \.recordID) { project in
                            NavigationLink(value: WorkspaceSidebarItem.project(project["id"].string)) { ProjectRow(project: project) }
                                .accessibilityIdentifier("sidebar-project-\(project["id"].string)")
                        }
                        if model.projects.isEmpty { emptyProjects }
                    } header: { Text(model.hostName) }
                }
            }
            .navigationTitle("Wisp")
            .navigationBarTitleDisplayMode(.inline)
            .navigationSplitViewColumnWidth(min: 220, ideal: 260, max: 340)
            .tint(WispStyle.selection)
            .toolbar { if paired { workspaceToolbar } }
            .refreshable { if paired { await model.refreshSelection() } }
        } content: {
            switch model.tab {
            case .conversations:
                if paired {
                    sessionsColumn
                } else {
                    ConnectionView(model: model, embedded: true)
                        .navigationSplitViewColumnWidth(min: 300, ideal: 380, max: 480)
                }
            case .calendar:
                ResearchView(workspace: model, connection: connection, project: nil, presentation: .list)
                    .navigationSplitViewColumnWidth(min: 280, ideal: 300, max: 420)
            case .checklist:
                ChecklistView(workspace: model, model: model.checklist, connection: connection, presentation: .list)
                    .navigationSplitViewColumnWidth(min: 280, ideal: 300, max: 420)
            case .notebook:
                NotebookView(workspace: model, model: model.notebook, connection: connection, presentation: .list)
                    .navigationSplitViewColumnWidth(min: 280, ideal: 300, max: 420)
            }
        } detail: {
            switch model.tab {
            case .conversations:
                if paired {
                    conversationDetail
                } else if columnFit == .all {
                    ToolPlaceholder(icon: .chat, title: "你的研究，随时继续",
                                    message: "连接 Wisp 主机后，在这里阅读和继续项目对话。也可以先体验演示项目。")
                } else {
                    ConnectionView(model: model, embedded: true)
                }
            case .calendar: ResearchView(workspace: model, connection: connection, project: nil, presentation: .detail)
            case .checklist: ChecklistView(workspace: model, model: model.checklist, connection: connection, presentation: .detail)
            case .notebook: NotebookView(workspace: model, model: model.notebook, connection: connection, presentation: .detail)
            }
        }
        .navigationSplitViewStyle(.balanced)
        .safeAreaInset(edge: .bottom, spacing: 0) { offlineStatus }
        .accessibilityElement(children: .contain).accessibilityIdentifier("workspace-sidebar-layout")
    }

    // MARK: Shared columns

    private var sessionsColumn: some View {
        List(selection: $model.selectedSession) {
            if let project = model.selectedProject {
                Section {
                    WorkspacePageLink(model: model, page: .journey(project), title: "研究历程", icon: .researchTrail, identifier: "research-journey")
                }
            }
            ForEach(model.sessions, id: \.recordID) { session in
                NavigationLink(value: session["id"].string) {
                    SessionRow(session: session)
                }
            }
            if model.sessions.isEmpty { Text(model.selectedProject == nil ? "选择一个项目" : "暂无对话") .foregroundStyle(.secondary) }
        }
        .navigationTitle(model.projects.first { $0["id"].string == model.selectedProject }?["name"].string ?? "对话")
        .navigationBarTitleDisplayMode(.inline)
        .navigationSplitViewColumnWidth(min: 240, ideal: 290, max: 400)
        .tint(WispStyle.selection)
        .toolbar {
            if !model.isDemo {
                ToolbarItem(placement: .topBarTrailing) {
                    WispToolbarButton(title: "新对话", icon: .squarePen) { Task { await model.createConversation() } }
                        .disabled(model.selectedProject == nil || model.creating || !connection.supports("native_conversation_create"))
                        .accessibilityIdentifier("new-conversation")
                }
            }
        }
        .refreshable { await model.refreshSelection() }
    }

    @ViewBuilder private var conversationDetail: some View {
        if let route = model.route {
            ConversationView(model: model, connection: connection, route: route).id(route)
        } else {
            VStack(spacing: 12) {
                Text("你的研究，随时继续").font(.title2.bold())
                Text(model.isDemo ? "选择演示项目，浏览质控报告或体验审批。" : "选择一个对话，或开始新对话。")
                    .foregroundStyle(.secondary)
            }.frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private var emptyProjects: some View {
        Text(connection.state == .online ? "暂无项目，请先在 Wisp 主机创建项目。" : "连接后显示项目。")
    }

    @ViewBuilder private var offlineStatus: some View {
        if paired && connection.state != .online {
            HStack {
                Text(model.projects.isEmpty ? (connection.diagnostic ?? "正在连接 Wisp 主机…") : "尚未连接 · 可查看已缓存内容，状态以联网刷新为准")
                    .font(.caption).accessibilityIdentifier("offline-cache-status")
                Spacer()
                Button("重新连接") { model.reconnect() }
            }.padding(12).background(.regularMaterial)
        }
    }

    /// Hosts the workspace with the project journey as a full page over it.
    private func pageHost<Content: View>(page: WorkspacePage?, @ViewBuilder content: () -> Content) -> some View {
        let content = content()
        return GeometryReader { geometry in
            ZStack {
                // Retain selection, scroll positions and connection input.
                // The entry remains visible behind an interactive edge swipe.
                content
                    .disabled(page != nil)
                    .allowsHitTesting(page == nil)
                    .accessibilityHidden(page != nil)
                if let page {
                    workspacePage(page)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .background(WispStyle.canvas)
                        .offset(x: reduceMotion ? 0 : geometry.size.width * model.pageBackProgress)
                        .transition(reduceMotion ? .opacity : .move(edge: .trailing))
                        .zIndex(1)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .clipped()
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.25), value: page)
            .onChange(of: page) { _, _ in
                UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
            }
        }
    }

    @ViewBuilder private func workspacePage(_ page: WorkspacePage) -> some View {
        switch page {
        case .journey(let project): ResearchView(workspace: model, connection: connection, project: project)
        }
    }

    @ToolbarContentBuilder private var workspaceToolbar: some ToolbarContent {
        ToolbarItem(placement: .topBarLeading) {
            WispToolbarButton(title: model.isDemo ? "演示设置" : "连接设置", icon: .gear) { model.sheet = .connection }
                .accessibilityIdentifier("connection-settings")
        }
        ToolbarItem(placement: .topBarTrailing) {
            WispToolbarButton(title: "待处理", icon: .inbox, count: model.inbox.count) { model.sheet = .inbox }
                .disabled(model.inbox.isEmpty).accessibilityIdentifier("inbox")
        }
    }

    private var workspaceBanners: some View {
        VStack(spacing: 0) {
            if model.isDemo {
                HStack(spacing: 10) {
                    Text("离线演示 · 合成数据")
                        .font(.caption.weight(.medium)).foregroundStyle(.primary)
                        .fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 4)
                    Button { model.exitDemo() } label: {
                        Text("退出演示").font(.caption.weight(.semibold))
                            .foregroundStyle(WispStyle.accent).fixedSize()
                            .frame(minHeight: 44).contentShape(Rectangle())
                    }
                    .buttonStyle(.plain).accessibilityIdentifier("exit-demo")
                }
                .padding(.horizontal, 16).padding(.vertical, 4)
                .background(WispStyle.accent.opacity(0.08))
                .overlay(alignment: .bottom) { Rectangle().fill(WispStyle.border).frame(height: 0.5) }
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("demo-banner")
            }
            if let error = model.error {
                HStack(alignment: .top) {
                    Text(error).font(.callout).accessibilityIdentifier("workspace-error")
                    Spacer()
                    Button("关闭") { model.error = nil }
                }.padding(12).background(WispStyle.warning.opacity(0.12))
            }
        }
        .fixedSize(horizontal: false, vertical: true)
    }
}

/// Tab bar items use the shared compose_icon glyphs as template images.
private struct WorkspaceTabLabel: View {
    let tab: WorkspaceTab
    var connected = true
    private var title: String {
        switch tab {
        case .conversations: return connected ? "对话" : "连接"
        case .calendar: return "日历"
        case .checklist: return "清单"
        case .notebook: return "记录本"
        }
    }
    private var icon: String {
        switch tab {
        case .conversations: return connected ? "wisp-chat" : "wisp-gear"
        case .calendar: return "wisp-calendar"
        case .checklist: return "wisp-list"
        case .notebook: return "wisp-notebook"
        }
    }
    var body: some View {
        Label { Text(title) } icon: { Image(icon).renderingMode(.template) }
    }
    /// The same glyph for the iPad sidebar row.
    static func iconKind(_ tab: WorkspaceTab, connected: Bool) -> WispIcon.Kind {
        switch tab {
        case .conversations: return connected ? .chat : .gear
        case .calendar: return .calendar
        case .checklist: return .list
        case .notebook: return .notebook
        }
    }
}

/// How many workspace columns fit side by side at the window's width. Lists are
/// about 300 pt, so in portrait (744 pt and up) one fits beside its detail.
private enum ColumnFit: Equatable {
    case all, tiled, overlaid
    init(width: CGFloat) { self = width >= 1100 ? .all : width >= 740 ? .tiled : .overlaid }
}

/// A project in the workspace's first column: name, conversation count and status.
private struct ProjectRow: View {
    let project: RemoteValue
    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(project["name"].string).font(.headline)
            Text("\(project["session_count"].uint ?? 0) 个对话").font(.caption).foregroundStyle(.secondary)
            if (project["needs_you_count"].uint ?? 0) > 0 { StatusBadge(status: "needs_you") }
            else if (project["running_count"].uint ?? 0) > 0 { StatusBadge(status: "running") }
        }.padding(.vertical, 4)
    }
}

/// File browsing and previews use a page-sized sheet on iPad where the system offers one.
private struct PageSizedSheet: ViewModifier {
    func body(content: Content) -> some View {
        if #available(iOS 18.0, *) { content.presentationSizing(.page) } else { content }
    }
}

/// Disclosure rows distinguish page navigation from modal actions.
private struct WorkspacePageLink: View {
    @ObservedObject var model: RemoteWorkspace
    let page: WorkspacePage
    let title: String
    let icon: WispIcon.Kind
    let identifier: String

    var body: some View {
        Button { model.page = page } label: {
            HStack(spacing: 12) {
                WispIcon(icon).foregroundStyle(WispStyle.accent)
                Text(title).foregroundStyle(.primary)
                Spacer(minLength: 8)
                WispIcon(.chevronRight, size: 14).foregroundStyle(.secondary)
            }.frame(minHeight: 44).contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier(identifier)
        .accessibilityHint("打开\(title)页面")
    }
}

struct WorkspacePageBackButton: View {
    @ObservedObject var workspace: RemoteWorkspace

    var body: some View {
        WispToolbarButton(title: "返回上一页", icon: .chevronLeft) { workspace.page = nil }
            .accessibilityIdentifier("workspace-page-back")
    }
}

struct ConnectionView: View {
    @ObservedObject var model: RemoteWorkspace
    /// Shown in the iPad workspace's list column, beside the sidebar, rather than as its own screen.
    var embedded = false
    var body: some View {
        if embedded {
            form.navigationTitle("连接").navigationBarTitleDisplayMode(.inline)
        } else {
            NavigationStack {
                form.navigationTitle("Wisp Science")
                    .frame(maxWidth: 640).frame(maxWidth: .infinity)
                    .background(Color(uiColor: .systemGroupedBackground))
            }
        }
    }
    private var form: some View {
        Form {
            Section {
                Text("连接到你的 Wisp").font(.title2.bold())
                Text("粘贴桌面端复制的远程访问链接，或填写云端中继地址和联机码。").foregroundStyle(.secondary)
            }
            Section {
                Text(embedded ? "「科研清单」和「实验记录本」在左侧边栏，无需连接主机即可使用，连接后清单同步到研究计划。"
                              : "「清单」和「记录本」在底部标签栏，无需连接主机即可使用，连接后清单同步到研究计划。")
                    .font(.subheadline).foregroundStyle(.secondary)
                    .accessibilityIdentifier("offline-tools-hint")
            }
            Section {
                Button { model.enterDemo() } label: {
                    VStack(alignment: .leading, spacing: 6) {
                        Text("体验演示项目").font(.headline)
                        Text("无需账号或联网，查看示例对话、质控报告和分析流程。")
                            .font(.subheadline).foregroundStyle(.secondary)
                    }.padding(.vertical, 6).frame(maxWidth: .infinity, alignment: .leading)
                }.accessibilityIdentifier("enter-demo")
            } header: {
                Text("先体验一下")
            } footer: {
                Text("所有示例均为随 App 提供的合成数据。演示消息使用本地预设回复，不调用 AI，也不执行真实分析。")
            }
            Section {
                TextField("https://relay.example.com/remote#…", text: $model.connectionAddress)
                    .keyboardType(.URL).textContentType(.URL).textInputAutocapitalization(.never).autocorrectionDisabled()
                    .accessibilityIdentifier("relay-address")
                SecureField("联机码（链接已包含时可留空）", text: $model.connectionCode)
                    .textInputAutocapitalization(.never).autocorrectionDisabled().accessibilityIdentifier("connection-code")
                Button("连接") { model.connect(address: model.connectionAddress, code: model.connectionCode) }
                    .disabled(model.connectionAddress.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .accessibilityIdentifier("connect")
            } header: {
                Text("云端连接")
            } footer: {
                Text("联机码在验证成功后保存在本机钥匙串。中继转发加密数据，运行 Agent 的 Wisp 主机需要保持在线。")
            }
            PrivacyLinksSection()
        }
    }
}

struct ConnectionSettingsView: View {
    @ObservedObject var model: RemoteWorkspace
    @ObservedObject var connection: RemoteConnection
    var body: some View {
        NavigationStack {
            Form {
                if model.isDemo {
                    Section("离线演示") {
                        Text("RNA-seq 质控示例").font(.headline)
                        Text("数据、图表和报告均随 App 打包。当前消息和审批仅保存在内存，退出、重置或重启后恢复初始示例。")
                            .foregroundStyle(.secondary)
                        Button("重置演示数据") { model.enterDemo() }.accessibilityIdentifier("reset-demo")
                        Button("退出演示，连接服务器") { model.exitDemo() }
                    }
                } else {
                    Section("当前连接") {
                        LabeledContent("中继", value: model.configuration?.relay.absoluteString ?? "")
                        LabeledContent("主机", value: model.hostName)
                        LabeledContent("版本", value: connection.hostVersion)
                        Text(connection.state == .online ? "已建立加密连接" : "等待主机连接")
                        Button("重新连接") { model.reconnect() }
                    }
                    Section {
                        Button("忘记连接并更换服务器", role: .destructive) { model.forget() }
                    } footer: { Text("删除这台设备保存的联机码和当前服务器的缓存。项目和计算任务仍保留在 Wisp 主机上。") }
                    Section {
                        LabeledContent("已用空间", value: ByteCountFormatter.string(fromByteCount: Int64(model.cacheBytes), countStyle: .file))
                        Button("清除本机缓存", role: .destructive) { Task { await model.clearCachedData() } }
                            .disabled(model.clearingCache).accessibilityIdentifier("clear-cache")
                        if let notice = model.cacheNotice { Text(notice).font(.caption).foregroundStyle(.secondary) }
                    } header: { Text("离线缓存") }
                      footer: { Text("已查看的对话和文件预览缓存在此设备，最多 50 MB、保留 30 天。不自动下载整个项目；系统可能回收缓存。") }
                    .task { await model.updateCacheSize() }
                    Section {
                        Button("体验演示项目") { model.enterDemo() }.accessibilityIdentifier("enter-demo")
                    } footer: { Text("暂时断开云端连接，保留已保存的联机码。") }
                }
                PrivacyLinksSection()
            }
            .navigationTitle(model.isDemo ? "演示设置" : "连接设置")
            .toolbar { Button("完成") { model.sheet = nil } }
        }.accessibilityElement(children: .contain).accessibilityIdentifier("connection-sheet")
    }
}

private struct PrivacyLinksSection: View {
    var body: some View {
        Section("隐私与支持") {
            Link("隐私政策", destination: URL(string: "https://wispscience.com/ios-privacy.html")!)
                .accessibilityIdentifier("privacy-policy")
            Link("帮助与反馈", destination: URL(string: "https://github.com/xuzhougeng/wisp-science/issues")!)
                .accessibilityIdentifier("app-support")
        }
    }
}

struct InboxView: View {
    @ObservedObject var model: RemoteWorkspace
    var body: some View {
        NavigationStack {
            List(model.inbox, id: \.recordID) { entry in
                Button { model.openInbox(entry) } label: {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(entry["title"].string)
                        Text(entry["project_name"].string).font(.caption).foregroundStyle(.secondary)
                        StatusBadge(status: entry["status"].string)
                    }
                }
            }
            .navigationTitle("待处理")
            .toolbar { Button("完成") { model.sheet = nil } }
        }
    }
}

/// Title, latest activity and status. Accessibility sizes stack the time below.
private struct SessionRow: View {
    let session: RemoteValue
    @Environment(\.dynamicTypeSize) private var typeSize
    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            let title = Text(session["title"].string.isEmpty ? "未命名对话" : session["title"].string)
            let time = WispTime.date(session["activity_at"]).map { WispTime.activity($0) }
            if typeSize.isAccessibilitySize {
                title
                if let time { Text(time).font(.caption).foregroundStyle(.secondary).accessibilityLabel("最近活动 \(time)") }
            } else {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    title.frame(maxWidth: .infinity, alignment: .leading)
                    if let time {
                        Text(time).font(.caption).monospacedDigit().foregroundStyle(.secondary).fixedSize()
                            .accessibilityLabel("最近活动 \(time)")
                    }
                }
            }
            StatusBadge(status: session["status"].string)
        }.padding(.vertical, 4)
    }
}

struct StatusBadge: View {
    let status: String
    /// Increased on a selected row, such as the open conversation in an iPad column.
    @Environment(\.backgroundProminence) private var prominence
    private var color: Color {
        switch status {
        case "succeeded": return WispStyle.success
        case "failed", "timed_out", "lost": return WispStyle.failure
        case "needs_you", "stopping", "cancelling": return WispStyle.warning
        case "running": return WispStyle.accent
        default: return .secondary
        }
    }
    var body: some View {
        if !status.isEmpty && status != "idle" {
            // Status colors are unreadable on the accent selection fill; follow the
            // row's selected foreground there, keeping the text label.
            let selected = prominence == .increased
            HStack(spacing: 4) {
                Circle().fill(selected ? AnyShapeStyle(.primary) : AnyShapeStyle(color)).frame(width: 5, height: 5).accessibilityHidden(true)
                Text(["running": "运行中", "needs_you": "待处理", "queued": "排队中", "submitted": "排队中",
                      "succeeded": "已完成", "failed": "失败", "stopping": "停止中", "cancelled": "已取消",
                      "timed_out": "超时", "lost": "失联", "paused": "已暂停", "draft": "草稿", "cancelling": "取消中"][status] ?? status)
            }
            .font(.caption2.weight(.medium)).foregroundStyle(selected ? AnyShapeStyle(.primary) : AnyShapeStyle(color))
            .padding(.horizontal, 7).padding(.vertical, 4)
            .background(selected ? Color.white.opacity(0.2) : color.opacity(0.09), in: RoundedRectangle(cornerRadius: 6))
            .accessibilityElement(children: .combine)
        }
    }
}
