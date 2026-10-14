import SwiftUI
import WispRemote

struct WorkspaceView: View {
    @ObservedObject var model: RemoteWorkspace
    @ObservedObject var connection: RemoteConnection

    var body: some View {
        // Banners own layout space above the navigation container. A top
        // safeAreaInset on NavigationSplitView can overlap its compact toolbar.
        VStack(spacing: 0) {
            workspaceBanners
            workspaceContent.frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .tint(WispStyle.accent)
        .sheet(item: $model.sheet) { sheet in
            switch sheet {
            case .connection: ConnectionSettingsView(model: model, connection: connection)
            case .inbox: InboxView(model: model)
            case .research(let project): ResearchView(workspace: model, connection: connection, project: project)
            case .checklist: ChecklistView(workspace: model, model: model.checklist, connection: connection)
            case .notebook: NotebookView(workspace: model, model: model.notebook, connection: connection)
            case .conversationInfo(let title, let project):
                NavigationStack {
                    Form {
                        Section("完整标题") { Text(title).textSelection(.enabled) }
                        if !project.isEmpty { Section("所属项目") { Text(project).textSelection(.enabled) } }
                    }
                    .navigationTitle("对话详情").navigationBarTitleDisplayMode(.inline)
                    .toolbar { ToolbarItem(placement: .topBarTrailing) { Button("完成") { model.sheet = nil } } }
                }.accessibilityIdentifier("conversation-info-sheet")
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

    private var workspaceContent: some View {
        Group {
            if model.configuration == nil && !model.isDemo {
                ConnectionView(model: model)
            } else {
                NavigationSplitView {
                    List(selection: $model.selectedProject) {
                        Section {
                            if !model.isDemo {
                                Button { model.sheet = .checklist } label: {
                                    HStack(spacing: 12) { WispIcon(.list); Text("科研清单") }
                                }.accessibilityIdentifier("research-checklist")
                                Button { model.sheet = .notebook } label: {
                                    HStack(spacing: 12) { WispIcon(.notebook); Text("实验记录本") }
                                }.accessibilityIdentifier("research-notebook")
                            }
                            Button { model.sheet = .research(nil) } label: {
                                HStack(spacing: 12) { WispIcon(.calendar); Text("科研日历") }
                            }.accessibilityIdentifier("research-calendar")
                        }
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
                    .toolbar {
                        ToolbarItem(placement: .topBarLeading) {
                            Button(model.isDemo ? "演示设置" : "连接") { model.sheet = .connection }.accessibilityIdentifier("connection-settings")
                        }
                        ToolbarItem(placement: .topBarTrailing) {
                            Button("待处理 \(model.inbox.count)") { model.sheet = .inbox }
                                .disabled(model.inbox.isEmpty).accessibilityIdentifier("inbox")
                        }
                    }
                    .refreshable { await model.refreshSelection() }
                } content: {
                    List(selection: $model.selectedSession) {
                        if let project = model.selectedProject {
                            Section {
                                Button { model.sheet = .research(project) } label: {
                                    HStack(spacing: 12) { WispIcon(.researchTrail); Text("研究历程") }
                                }.accessibilityIdentifier("research-journey")
                            }
                        }
                        ForEach(model.sessions, id: \.recordID) { session in
                            NavigationLink(value: session["id"].string) {
                                VStack(alignment: .leading, spacing: 5) {
                                    Text(session["title"].string.isEmpty ? "未命名对话" : session["title"].string)
                                    StatusBadge(status: session["status"].string)
                                }.padding(.vertical, 4)
                            }
                        }
                        if model.sessions.isEmpty { Text(model.selectedProject == nil ? "选择一个项目" : "暂无对话") .foregroundStyle(.secondary) }
                    }
                    .navigationTitle(model.projects.first { $0["id"].string == model.selectedProject }?["name"].string ?? "对话")
                    .navigationBarTitleDisplayMode(.inline)
                    .navigationSplitViewColumnWidth(min: 240, ideal: 290, max: 400)
                    .toolbar {
                        if !model.isDemo {
                            Button("新对话") { Task { await model.createConversation() } }
                                .disabled(model.selectedProject == nil || model.creating || !connection.supports("native_conversation_create"))
                                .accessibilityIdentifier("new-conversation")
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
                }.padding(12).background(Color.orange.opacity(0.15))
            }
        }
        .fixedSize(horizontal: false, vertical: true)
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
                Section("离线研究工具") {
                    Button { model.sheet = .checklist } label: {
                        HStack(spacing: 12) { WispIcon(.list); Text("科研清单") }
                    }.accessibilityIdentifier("research-checklist")
                    Button { model.sheet = .notebook } label: {
                        HStack(spacing: 12) { WispIcon(.notebook); Text("实验记录本") }
                    }.accessibilityIdentifier("research-notebook")
                    Text("无需连接主机即可记录事项，连接后同步到研究计划。").font(.caption).foregroundStyle(.secondary)
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
        }.accessibilityIdentifier("connection-sheet")
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
