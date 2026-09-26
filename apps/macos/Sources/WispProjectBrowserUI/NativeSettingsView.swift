import AppKit
import SwiftUI
import WispProjectBrowser

struct NativeSettingsView: View {
    @StateObject private var state: NativeSettingsModel
    let projects: [ProjectSummary]
    let close: () -> Void
    let editProject: Bool
    @State private var update: SettingsValue?
    @State private var confirmInstall = false
    @State private var confirmLeave = false
    @State private var pendingProject: String?
    @State private var changingProject = false
    @Environment(\.colorScheme) private var scheme
    @AppStorage("projectBrowser.appearance") private var appearance = "system"

    init(databaseURL: URL, projects: [ProjectSummary], projectID: String?, editProject: Bool = false, initialSection: NativeSettingsSection = .general, close: @escaping () -> Void) {
        self.projects = projects; self.close = close; self.editProject = editProject
        let model = NativeSettingsModel(client: NativeSettingsClient(databaseURL: databaseURL, executableURL: nativeDesktopHostURL()), projectID: projectID ?? projects.first?.id)
        model.section = initialSection
        _state = StateObject(wrappedValue: model)
    }

    var body: some View {
        GeometryReader { geometry in
        HStack(spacing: 0) {
            navigation.frame(width: min(240, max(188, geometry.size.width * 0.18))).background(WispDesign.color("bg-sunken", scheme))
            Divider()
            VStack(alignment: .leading, spacing: 0) {
                HStack {
                    if state.editor != nil || state.detailSection != nil {
                        Button { state.editor = nil; state.detailSection = nil } label: { WispIcon(name: "chevron-left", size: 15) }.buttonStyle(NativeSettingsButtonStyle())
                        Text(state.section.title).foregroundStyle(.secondary)
                        WispIcon(name: "chevron-right", size: 12)
                    }
                    Text(state.editor.map { localized($0.title) } ?? state.detailSection.map(localized) ?? state.section.title).font(WispDesign.font(size: state.editor == nil ? 20 : 14, weight: .semibold))
                    Spacer()
                    if [.memory, .skills, .plugins, .permissions, .environments, .storage, .channels].contains(state.section) {
                    Picker(localized("项目"), selection: Binding(get: { state.projectID }, set: { id in
                        if state.hasUnsavedChanges { pendingProject = id; changingProject = true; confirmLeave = true } else { state.projectID = id }
                    })) {
                        Text(localized("全局设置")).tag(nil as String?)
                        ForEach(projects) { Text($0.name).tag(Optional($0.id)) }
                    }.frame(maxWidth: 180).disabled(state.busy)
                    }
                    Button { Task { await state.load() } } label: { WispIcon(name: "refresh") }
                        .buttonStyle(WispButtonStyle()).disabled(state.loading || state.busy).help("重新载入")
                }.padding(.horizontal, 24).padding(.vertical, 22)
                    .frame(maxWidth: state.section == .models && state.editor == nil ? 920 : 1040)
                    .overlay(alignment: .bottom) { WispDesign.color("border", scheme).frame(height: 1) }
                    .frame(maxWidth: .infinity)
                ScrollView {
                    VStack(alignment: .leading, spacing: 22) {
                        if state.loading { HStack { ProgressView().controlSize(.small); Text(localized("正在读取设置…")).foregroundStyle(.secondary) } }
                        if let error = state.error { Text(error).foregroundStyle(.red).textSelection(.enabled).padding(12).frame(maxWidth: .infinity, alignment: .leading).background(Color.red.opacity(0.06), in: RoundedRectangle(cornerRadius: 8)) }
                        if let message = state.message { Text(message).foregroundStyle(WispDesign.color("clay", scheme)).accessibilityAddTraits(.updatesFrequently) }
                        if let editor = state.editor { NativeSettingsEditorView(model: state, editor: editor).id(editor.id) }
                        else { pane.disabled(state.busy) }
                    }.padding(24).frame(maxWidth: state.section == .models && state.editor == nil ? 920 : 1040, alignment: .leading).frame(maxWidth: .infinity)
                }
            }.frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        }
        .background(WispDesign.color("bg-app", scheme))
        .tint(WispDesign.color("clay", scheme))
        .buttonStyle(NativeSettingsButtonStyle())
        .font(WispDesign.font(size: 14))
        .task {
            await state.load()
            if editProject, let project = await state.run("get_project_settings", refresh: false, success: "") {
                state.editor = SettingsEditor(title: "项目设置", draft: project, fields: [.init(key: "name", label: "项目名称"), .init(key: "description", label: "项目说明", kind: .multiline), .init(key: "agent_context", label: "Agent Context", kind: .multiline)], command: "update_project", parameter: nil)
            }
        }
        .onChange(of: state.section) { _ in state.editor = nil; state.detailSection = nil; Task { await state.load() } }
        .onChange(of: state.projectID) { _ in state.editor = nil; Task { await state.load() } }
        .background(NativeSettingsEscape(enabled: state.editor == nil && !confirmLeave && !confirmInstall) { requestClose() })
        .confirmationDialog("放弃尚未保存的修改？", isPresented: $confirmLeave) {
            Button(localized("放弃修改"), role: .destructive) { state.discardDrafts(); if changingProject { state.projectID = pendingProject; changingProject = false } else { state.leave(); close() } }
            Button(localized("继续编辑"), role: .cancel) { changingProject = false }
        }
        .confirmationDialog("安装更新将重启桌面宿主。", isPresented: $confirmInstall) {
            Button(localized("安装并重启")) { Task { _ = await state.run("install_update", refresh: false, success: "正在安装") } }
            Button(localized("取消"), role: .cancel) {}
        }
        .onDisappear { state.leave() }
    }

    private func requestClose() {
        if state.detailSection != nil { state.detailSection = nil; return }
        if confirmLeave || confirmInstall { return }
        if state.hasUnsavedChanges { changingProject = false; confirmLeave = true } else { state.leave(); close() }
    }

    private var navigation: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Button { requestClose() } label: { HStack(spacing: 8) { WispIcon(name: "chevron-left", size: 15); Text(localized("返回应用")) }.foregroundStyle(.secondary).padding(.horizontal, 10).padding(.vertical, 7) }.buttonStyle(.plain)
                Text(localized("设置")).font(WispDesign.font(size: 20, weight: .semibold)).padding(.horizontal, 14)
                TextField(localized("搜索设置"), text: $state.search).textFieldStyle(NativeSettingsTextFieldStyle())
                ForEach(["基础偏好", "AI 配置", "工具与连接", "系统与资源"], id: \.self) { group in
                    let sections = NativeSettingsSection.allCases.filter { $0.group == group && $0.matches(state.search) }
                    if !sections.isEmpty {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(localized(group)).font(WispDesign.font(size: 12, weight: .semibold)).foregroundStyle(.secondary).padding(.horizontal, 14).padding(.bottom, 4)
                            ForEach(sections) { section in
                                Button { state.section = section } label: {
                                    Text(section.title).font(WispDesign.font(size: 13.5, weight: state.section == section ? .semibold : .regular))
                                        .foregroundStyle(WispDesign.color(state.section == section ? "text" : "text-muted", scheme))
                                        .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 14).padding(.vertical, 10)
                                        .background(state.section == section ? WispDesign.color("bg-elev", scheme) : .clear, in: RoundedRectangle(cornerRadius: 9))
                                        .overlay(RoundedRectangle(cornerRadius: 9).stroke(state.section == section ? WispDesign.color("border", scheme) : .clear))
                                        .shadow(color: .black.opacity(state.section == section ? 0.04 : 0), radius: 1, y: 1)
                                }.buttonStyle(.plain).disabled(state.busy).accessibilityIdentifier("native-settings-nav-\(section.rawValue)")
                                    .accessibilityAddTraits(state.section == section ? .isSelected : [])
                            }
                        }.padding(.top, 4)
                    }
                }
                if !NativeSettingsSection.allCases.contains(where: { $0.matches(state.search) }) { Text(localized("没有匹配的设置")).foregroundStyle(.secondary).font(WispDesign.font(size: 13)).padding(.horizontal, 14) }
            }.padding(.horizontal, 12).padding(.vertical, 18)
        }.frame(maxHeight: .infinity, alignment: .top)
    }

    @ViewBuilder private var pane: some View {
        switch state.section {
        case .general: general
        case .session: session
        case .appearance: appearancePane
        case .pet: pet
        case .models, .credentials: NativeModelSettings(model: state)
        case .quickActions, .workflows, .specialists: NativeWorkflowSettings(model: state)
        case .skills, .plugins, .browser, .connections: NativeToolsSettings(model: state)
        case .memory, .channels, .permissions, .environments, .storage, .usage: NativeWorkspaceSettings(model: state)
        }
    }

    private func fields(_ command: String, _ fields: [SettingsField]) -> some View {
        VStack(spacing: 20) { ForEach(fields) { NativeSettingsField(field: $0, value: state.binding(command, $0.key)) } }.disabled(state.values[command] == nil)
    }
    private func save(_ title: String = "保存", operation: @escaping () async -> Void) -> some View {
        HStack { Spacer(); Button(title) { Task { await operation() } }.buttonStyle(NativeSettingsButtonStyle(primary: true)).disabled(state.loading || state.busy) }
    }

    private var general: some View {
        VStack(spacing: 24) {
            NativeSettingsGroup(title: "工作区与交互") {
                fields("get_settings", [
                    .init(key: "locale", label: "语言", kind: .choice([("zh", "简体中文"), ("en", "English")])),
                    .init(key: "workspace_dir", label: "工作目录", kind: .path, hint: "留空使用默认目录；下次启动生效。"),
                    .init(key: "resume_last_session", label: "打开工作区时继续上次对话", kind: .toggle, hint: "打开工作区时恢复最近有过对话的会话，不会进入仅改了名、还没发过消息的草稿。")
                ])
                NativePreferenceRow(title: "发送与换行快捷键") {
                    Picker(localized("发送与换行快捷键"), selection: Binding(get: { state.values["get_appearance_prefs"]?["send_with_modifier"].bool ?? false }, set: { state.binding("get_appearance_prefs", "send_with_modifier").wrappedValue = .bool($0) })) {
                        Text(localized("Enter 发送 · Shift+Enter 换行")).tag(false)
                        Text(localized("⌘Enter 发送 · Enter 换行")).tag(true)
                    }.labelsHidden().fixedSize().disabled(state.values["get_appearance_prefs"] == nil)
                }
                fields("get_appearance_prefs", [.init(key: "selection_popup_enabled", label: "选中文本快捷菜单", kind: .toggle)])
                Divider().padding(.vertical, 8)
                Text(localized("通知与更新")).font(WispDesign.font(size: 15, weight: .semibold))
                fields("get_settings", [.init(key: "notifications_enabled", label: "桌面通知", kind: .toggle, hint: "窗口不在前台时，任务完成、失败或等待确认会发送系统通知。")])
                HStack {
                    Button(localized("取消")) { state.discardDrafts() }
                    save {
                        await state.saveSettings()
                        if state.error == nil, let prefs = state.values["get_appearance_prefs"] { _ = await state.run("set_appearance_prefs", ["prefs": prefs]) }
                    }
                }
                immediateToggle("自动检查更新", read: "get_update_check_enabled", write: "set_update_check_enabled")
                Button(localized("检查更新")) { Task { update = await state.run("check_for_updates", refresh: false, success: "检查完成") } }
                if let update {
                    Text(update["update_available"].bool ? "发现版本 " + update["latest_version"].string : "当前已是最新版本")
                    Text(update["notes"].string).font(.caption).textSelection(.enabled)
                    if let url = URL(string: update["release_url"].string) { Link("查看发布说明", destination: url) }
                    if update["update_available"].bool && update["install_supported"].bool {
                        if update["downloaded"].bool { Button(localized("安装桌面宿主更新…")) { confirmInstall = true } }
                        else { Button(localized("下载并验证更新")) { Task { if await state.run("native_download_update", refresh: false, success: "更新包已验证") != nil { self.update = await state.run("check_for_updates", refresh: false, success: "更新可安装") } } } }
                    }
                }
            }
            NativeLocalEnvironmentSettings(model: state)
            NativeSettingsGroup(title: "网络与软件源") {
                fields("get_network_settings", [
                    .init(key: "model_proxy_url", label: "模型 API 代理", hint: "留空跟随系统；none 为直连；支持 HTTP / HTTPS / SOCKS5。"),
                    .init(key: "mcp_proxy_url", label: "MCP 代理"),
                    .init(key: "command_proxy_url", label: "代码与命令代理"),
                    .init(key: "conda_mirror_url", label: "Conda 镜像"),
                    .init(key: "pip_index_url", label: "Python 软件源"),
                    .init(key: "ca_bundle_path", label: "CA 证书路径")
                ])
                save("保存网络设置") { _ = await state.run("set_network_settings", ["settings": state.values["get_network_settings"] ?? .null]) }
            }
        }
    }

    private var session: some View {
        NativeSettingsGroup(title: "运行限制") {
            NativePreferenceRow(title: "最大迭代次数", hint: "每轮对话最多执行的工具调用轮次，0 表示不限制。") { number("max_iter") }
            NativePreferenceRow(title: "自动继续", hint: "达到迭代上限时自动继续执行。") { settingToggle("auto_continue") }
            if state.values["get_settings"]?["auto_continue"].bool == true {
                NativePreferenceRow(title: "自动继续轮次", hint: "限制单轮请求的连续执行次数。") { number("auto_continue_limit") }
            }
            Divider().padding(.vertical, 10)
            Text(localized("上下文管理")).font(WispDesign.font(size: 15, weight: .semibold))
            NativePreferenceRow(title: "自动压缩上下文", hint: "接近上下文容量时压缩较早的对话记录。") { settingToggle("auto_compact") }
            Divider().padding(.vertical, 10)
            Text(localized("后续交互")).font(WispDesign.font(size: 15, weight: .semibold))
            NativePreferenceRow(title: "建议后续问题", hint: "回复完成后提供可继续探索的问题。") { settingToggle("follow_up_questions") }
            immediateToggle("自动审核（新会话默认）", read: "get_auto_review_enabled", write: "set_auto_review_enabled")
            HStack { Spacer(); Button(localized("取消")) { state.discardDrafts() }; Button(localized("保存")) { Task { await state.saveSettings() } }.buttonStyle(NativeSettingsButtonStyle(primary: true)) }
        }
    }
    private func number(_ key: String) -> some View {
        TextField("", text: Binding(get: { state.values["get_settings"]?[key].string ?? "" }, set: { state.binding("get_settings", key).wrappedValue = $0.isEmpty ? .null : (Int64($0).map(SettingsValue.integer) ?? .string($0)) }))
            .textFieldStyle(NativeSettingsTextFieldStyle()).frame(width: 112).accessibilityLabel(key)
    }
    private func settingToggle(_ key: String) -> some View {
        Toggle("", isOn: Binding(get: { state.values["get_settings"]?[key].bool ?? false }, set: { state.binding("get_settings", key).wrappedValue = .bool($0) })).toggleStyle(.switch).labelsHidden().accessibilityLabel(key)
    }

    private var appearancePane: some View { NativeAppearanceSettings(model: state) }

    private var pet: some View {
        NativeSettingsGroup(title: "桌宠") {
            fields("get_settings", [.init(key: "pet_enabled", label: "启用桌宠", kind: .toggle), .init(key: "pet_directory", label: "资源目录", kind: .path)])
            save { await state.saveSettings() }
            if let pet = state.values["get_pet"], !pet["error"].string.isEmpty { Text(pet["error"].string).foregroundStyle(.red) }
            if let runtime = state.values["get_pet_runtime_status"] { Text("运行中 \(runtime["running"].array.count) · 等待审批 \(runtime["waiting"].array.count) · 审核中 \(runtime["reviewing"].array.count)").foregroundStyle(.secondary) }
        }
    }

    private func immediateToggle(_ label: String, read: String, write: String) -> some View {
        Toggle(label, isOn: Binding(get: { state.values[read]?.bool ?? false }, set: { value in Task { _ = await state.run(write, ["enabled": .bool(value)]) } })).disabled(state.values[read] == nil || state.loading)
    }
}

struct NativeSettingsSummary: View {
    let value: SettingsValue
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(value.object.keys.sorted(), id: \.self) { key in
                let item = value[key]
                if item != .null {
                    if !item.object.isEmpty || !item.array.isEmpty {
                        DisclosureGroup(label(key)) { AnyView(NativeSettingsSummary(value: item.array.isEmpty ? item : .object(Dictionary(uniqueKeysWithValues: item.array.enumerated().map { (String($0.offset + 1), $0.element) })))) }
                    } else {
                        HStack(alignment: .top) { Text(label(key)).foregroundStyle(.secondary); Spacer(); Text(display(item)).textSelection(.enabled) }
                    }
                }
            }
        }.font(WispDesign.font(size: 12))
    }
    private func display(_ value: SettingsValue) -> String {
        if case .bool(let enabled) = value { return enabled ? "是" : "否" }
        if case .array(let rows) = value { return "\(rows.count) 项" }
        return value.string
    }
    private func label(_ key: String) -> String {
        ["id": "标识", "label": "名称", "name": "名称", "status": "状态", "message": "说明", "kind": "类型", "last_probe_status": "探测状态", "last_probe_error": "探测错误", "last_probe_at": "上次探测", "capabilities_json": "能力详情", "config_json": "环境配置", "created_at": "创建时间", "updated_at": "更新时间", "context_id": "执行环境", "project_id": "项目", "remote_files": "远程文件", "local_files": "本地文件", "blockers": "阻止清理的项目", "warnings": "提示"][key] ?? key
    }
}
