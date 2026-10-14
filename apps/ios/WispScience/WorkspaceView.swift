import SwiftUI
import UIKit
import WispRemote

struct WorkspaceView: View {
    @ObservedObject var model: RemoteWorkspace
    @ObservedObject var connection: RemoteConnection
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        // Banners own layout space above the navigation container. A top
        // safeAreaInset on NavigationSplitView can overlap its compact toolbar.
        VStack(spacing: 0) {
            workspaceBanners
            TabView(selection: $model.tab) {
                conversationsTab
                    .tabItem { WorkspaceTabLabel(tab: .conversations, connected: model.configuration != nil || model.isDemo) }
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
            .onChange(of: model.availableTabs) { _, tabs in
                if !tabs.contains(model.tab) { model.tab = .conversations }
            }
            .onChange(of: model.tab) { _, _ in
                UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
            }
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
            case .files(let route, let path, let directory):
                FilesView(model: model, route: route, initialPath: path, isDirectory: directory)
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

    /// Projects, conversations and the project journey page over them.
    private var conversationsTab: some View {
        GeometryReader { geometry in
                ZStack {
                    // Retain selection, scroll positions and connection input.
                    // The entry remains visible behind an interactive edge swipe.
                    workspaceContent
                        .disabled(model.page != nil)
                        .allowsHitTesting(model.page == nil)
                        .accessibilityHidden(model.page != nil)
                    if let page = model.page {
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
                .animation(reduceMotion ? nil : .easeInOut(duration: 0.25), value: model.page)
                .onChange(of: model.page) { _, _ in
                    UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
                }
        }
    }

    @ViewBuilder private func workspacePage(_ page: WorkspacePage) -> some View {
        switch page {
        case .journey(let project): ResearchView(workspace: model, connection: connection, project: project)
        }
    }

    private var workspaceContent: some View {
        Group {
            if model.configuration == nil && !model.isDemo {
                ConnectionView(model: model)
            } else {
                NavigationSplitView {
                    List(selection: $model.selectedProject) {
                        Section {
                            ForEach(model.projects, id: \.recordID) { project in
                                NavigationLink(value: project["id"].string) {
                                    VStack(alignment: .leading, spacing: 5) {
                                        Text(project["name"].string).font(.headline)
                                        Text("\(project["session_count"].uint ?? 0) 个对话").font(.caption).foregroundStyle(.secondary)
                                        if (project["needs_you_count"].uint ?? 0) > 0 { StatusBadge(status: "needs_you") }
                                        else if (project["running_count"].uint ?? 0) > 0 { StatusBadge(status: "running") }
                                    }.padding(.vertical, 4)
                                }
                            }
                        } header: { Text(model.hostName) }
                        if model.projects.isEmpty { Text(connection.state == .online ? "暂无项目，请先在 Wisp 主机创建项目。" : "连接后显示项目。") }
                    }
                    .navigationTitle("Wisp")
                    .navigationBarTitleDisplayMode(.inline)
                    .navigationSplitViewColumnWidth(min: 210, ideal: 250, max: 340)
                    .toolbar { workspaceToolbar }
                    .refreshable { await model.refreshSelection() }
                } content: {
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
                } detail: {
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
                .safeAreaInset(edge: .bottom, spacing: 0) {
                    if connection.state != .online {
                        HStack {
                            Text(model.projects.isEmpty ? (connection.diagnostic ?? "正在连接 Wisp 主机…") : "尚未连接 · 可查看已缓存内容，状态以联网刷新为准")
                                .font(.caption).accessibilityIdentifier("offline-cache-status")
                            Spacer()
                            Button("重新连接") { model.reconnect() }
                        }.padding(12).background(.regularMaterial)
                    }
                }
            }
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
    @State private var address = ""
    @State private var code = ""
    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text("连接到你的 Wisp").font(.title2.bold())
                    Text("粘贴桌面端复制的远程访问链接，或填写云端中继地址和联机码。").foregroundStyle(.secondary)
                }
                Section {
                    Text("「清单」和「记录本」在底部标签栏，无需连接主机即可使用，连接后清单同步到研究计划。")
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
                    TextField("https://relay.example.com/remote#…", text: $address)
                        .keyboardType(.URL).textContentType(.URL).textInputAutocapitalization(.never).autocorrectionDisabled()
                        .accessibilityIdentifier("relay-address")
                    SecureField("联机码（链接已包含时可留空）", text: $code)
                        .textInputAutocapitalization(.never).autocorrectionDisabled().accessibilityIdentifier("connection-code")
                    Button("连接") { model.connect(address: address, code: code) }
                        .disabled(address.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        .accessibilityIdentifier("connect")
                } header: {
                    Text("云端连接")
                } footer: {
                    Text("联机码在验证成功后保存在本机钥匙串。中继转发加密数据，运行 Agent 的 Wisp 主机需要保持在线。")
                }
            }
            .navigationTitle("Wisp Science")
            .frame(maxWidth: 640).frame(maxWidth: .infinity)
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
            }
            .navigationTitle(model.isDemo ? "演示设置" : "连接设置")
            .toolbar { Button("完成") { model.sheet = nil } }
        }.accessibilityElement(children: .contain).accessibilityIdentifier("connection-sheet")
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
            HStack(spacing: 4) {
                Circle().fill(color).frame(width: 5, height: 5).accessibilityHidden(true)
                Text(["running": "运行中", "needs_you": "待处理", "queued": "排队中", "submitted": "排队中",
                      "succeeded": "已完成", "failed": "失败", "stopping": "停止中", "cancelled": "已取消",
                      "timed_out": "超时", "lost": "失联", "paused": "已暂停", "draft": "草稿", "cancelling": "取消中"][status] ?? status)
            }
            .font(.caption2.weight(.medium)).foregroundStyle(color)
            .padding(.horizontal, 7).padding(.vertical, 4)
            .background(color.opacity(0.09), in: RoundedRectangle(cornerRadius: 6))
            .accessibilityElement(children: .combine)
        }
    }
}
