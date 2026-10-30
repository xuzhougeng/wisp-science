import AppKit
import SwiftUI
import WispProjectBrowser

struct NativeToolsSettings: View {
    @ObservedObject var model: NativeSettingsModel
    @State private var query = ""
    @State private var skillName = ""
    @State private var skillFiles: [String] = []
    @State private var fileContent = ""
    @State private var repository = ""
    @State private var gitRef = "main"
    @State private var candidates: [SettingsValue] = []
    @State private var catalog: [SettingsValue] = []
    @State private var expandedConnector: String?
    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            switch model.section {
            case .skills: skills; skillStore
            case .plugins: plugins
            case .browser: browser
            default: connectors; mcp
            }
        }
    }
    private func rows(_ command: String) -> [SettingsValue] { model.values[command]?.array ?? [] }
    private func toggle(_ title: String, hint: String = "", value: Bool, command: String, args: [String: SettingsValue]) -> some View {
        NativeSettingsSwitch(title: title, hint: hint, isOn: Binding(get: { value }, set: { enabled in var next = args; next["enabled"] = .bool(enabled); Task { _ = await model.run(command, next) } }))
    }
    private var skills: some View {
        NativeSettingsGroup(title: "项目技能") {
            HStack {
                TextField(localized("搜索技能与标签"), text: $query).textFieldStyle(NativeSettingsTextFieldStyle())
                Button(localized("导入…")) { model.editor = SettingsEditor(title: "导入技能", draft: .object([:]), fields: [.init(key: "src_path", label: "目录或 ZIP", kind: .file)], command: "install_skill", parameter: nil) }
                Button(localized("重新扫描")) { Task { _ = await model.run("reload_skills", success: "技能已刷新") } }
            }
            ForEach(Array(rows("list_skills").filter { query.isEmpty || ($0["name"].string + $0["description"].string + $0["tags"].array.map(\.string).joined()).localizedCaseInsensitiveContains(query) }.enumerated()), id: \.offset) { _, skill in
                VStack(alignment: .leading, spacing: 9) {
                    HStack {
                        toggle(skill["name"].string, value: skill["enabled"].bool, command: "set_skill_enabled", args: ["name": skill["name"]])
                        Spacer()
                        Button(localized("文件")) { Task {
                            skillName = skill["name"].string; fileContent = ""
                            skillFiles = await model.run("list_skill_files", ["name": skill["name"]], refresh: false, success: "文件列表已读取")?.array.map(\.string) ?? []
                        } }
                        Button(localized("管理")) { model.editor = SettingsEditor(title: skill["name"].string, draft: .object(["tags": skill["tags"]]), fields: [.init(key: "tags", label: "标签", kind: .lines)], command: "set_skill_tags", parameter: nil, extra: ["name": skill["name"]], destructiveCommand: skill["builtin"].bool || skill["managed"].bool ? nil : "remove_skill", destructiveArgs: ["name": skill["name"]]) }
                    }
                    Text(skill["description"].string).foregroundStyle(.secondary)
                    Text(skill["tags"].array.map(\.string).joined(separator: " · ")).font(.caption).foregroundStyle(.secondary)
                }.padding(.vertical, 8)
                Divider()
            }
            if !skillName.isEmpty {
                Text(skillName).fontWeight(.semibold)
                ForEach(skillFiles, id: \.self) { path in
                    Button(path) { Task { fileContent = await model.run("read_skill_file", ["name": .string(skillName), "path": .string(path)], refresh: false, success: "文件已读取")?["content"].string ?? "" } }.buttonStyle(.link)
                }
                if !fileContent.isEmpty { Text(fileContent).font(WispDesign.font(size: 12, design: .monospaced)).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }
            }
        }
    }
    private var skillStore: some View {
        NativeSettingsGroup(title: "技能商店与 GitHub 导入") {
            HStack {
                Button(localized("浏览社区目录")) { Task { catalog = await model.run("list_community_skills", ["refresh": .bool(false)], refresh: false, success: "目录已读取")?["entries"].array ?? [] } }
                Button(localized("更新社区目录")) { Task { catalog = await model.run("list_community_skills", ["refresh": .bool(true)], refresh: false, success: "目录已刷新")?["entries"].array ?? [] } }
            }
            ForEach(Array(catalog.filter { query.isEmpty || ($0["name"].string + $0["description"].string).localizedCaseInsensitiveContains(query) }.enumerated()), id: \.offset) { _, item in
                DisclosureGroup(item["name"].string) {
                    Text(item["description"].string).frame(maxWidth: .infinity, alignment: .leading)
                    Text(item["known_limits"].string).foregroundStyle(.secondary)
                    Button(localized("选择来源")) { repository = "https://github.com/" + item["repository"].string; gitRef = item["git_ref"].string }
                }
            }
            TextField(localized("GitHub 仓库 URL"), text: $repository).textFieldStyle(NativeSettingsTextFieldStyle())
            TextField(localized("分支、Tag 或 Commit"), text: $gitRef).textFieldStyle(NativeSettingsTextFieldStyle())
            Button(localized("预览可安装技能")) { Task { candidates = await model.run("preview_github_skills", ["sourceUrl": .string(repository), "exactRef": .string(gitRef)], refresh: false, success: "预览完成")?.array ?? [] } }
            ForEach(Array(candidates.enumerated()), id: \.offset) { _, item in
                VStack(alignment: .leading, spacing: 8) {
                    Text(item["name"].string).fontWeight(.semibold)
                    Text(item["description"].string)
                    Text(item["source"]["commit"].string).font(.caption).textSelection(.enabled)
                    ForEach(item["warnings"].array.map(\.string) + item["format_errors"].array.map(\.string) + item["resource_errors"].array.map(\.string), id: \.self) { Text($0).foregroundStyle(.orange) }
                    if !item["conflict"].string.isEmpty { Text(item["conflict"].string).foregroundStyle(.red) }
                    DisclosureGroup("查看技能说明") { Text(item["markdown"].string).textSelection(.enabled) }
                    Button(localized("安装此版本")) { Task { _ = await model.run("install_github_skill", ["source": item["source"]], success: "技能已安装") } }.disabled(!item["conflict"].string.isEmpty || !item["format_errors"].array.isEmpty || !item["resource_errors"].array.isEmpty)
                }
            }
        }
    }
    private var plugins: some View {
        NativeSettingsGroup(title: "插件") {
            HStack {
                Button(localized("本地安装…")) { model.editor = SettingsEditor(title: "安装插件", draft: .object([:]), fields: [.init(key: "src_path", label: "插件目录或归档", kind: .file), .init(key: "expected_sha256", label: "SHA-256（可选）")], command: "install_plugin", parameter: nil) }
                Button(localized("从 URL 安装…")) { model.editor = SettingsEditor(title: "从 URL 安装插件", draft: .object([:]), fields: [.init(key: "source_url", label: "下载地址"), .init(key: "expected_sha256", label: "SHA-256")], command: "install_plugin_url", parameter: nil) }
            }
            if !rows("list_plugins").isEmpty {
                Text(Self.pluginSummary(rows("list_plugins"))).font(.caption).foregroundStyle(.secondary)
            }
            ForEach(Array(rows("list_plugins").enumerated()), id: \.offset) { _, plugin in
                VStack(alignment: .leading, spacing: 10) {
                    HStack {
                        Text(plugin["display_name"].string).fontWeight(.semibold); Text(plugin["version"].string).foregroundStyle(.secondary); Spacer()
                        // Same per-project switch as the WebView row; 管理 keeps removal.
                        Toggle("", isOn: Binding(get: { plugin["enabled"].bool }, set: { enabled in
                            Task { _ = await model.run("set_plugin_enabled", ["pluginId": plugin["id"], "version": plugin["version"], "enabled": .bool(enabled)]) }
                        }))
                        .toggleStyle(.switch).labelsHidden().disabled(model.busy)
                        .help(localized(plugin["enabled"].bool ? "为当前项目停用" : "为当前项目启用"))
                        .accessibilityLabel(localized(plugin["enabled"].bool ? "为当前项目停用" : "为当前项目启用") + " " + plugin["display_name"].string)
                        Button(localized("管理")) { model.editor = SettingsEditor(title: plugin["display_name"].string, draft: .object(["enabled": plugin["enabled"]]), fields: [.init(key: "enabled", label: "在此项目启用", kind: .toggle, hint: "插件工具遵循现有审批策略；停用会停止该项目中的插件运行时。")], command: "set_plugin_enabled", parameter: nil, extra: ["pluginId": plugin["id"], "version": plugin["version"]], destructiveCommand: "remove_plugin", destructiveArgs: ["pluginId": plugin["id"], "version": plugin["version"]]) }
                    }
                    Text(plugin["description"].string)
                    Text(Self.pluginState(plugin)).font(.caption).foregroundStyle(.secondary)
                    Text(plugin["source_uri"].string).font(.caption).textSelection(.enabled)
                    ForEach(plugin["runtime_errors"].array.map(\.string), id: \.self) { Text($0).foregroundStyle(.red) }
                }.padding(.vertical, 10)
                Divider()
            }
        }
    }
    /// `plugins.summary` in ui/src/i18n.rs.
    static func pluginSummary(_ plugins: [SettingsValue]) -> String {
        localized("当前项目已启用 {enabled}/{total}")
            .replacingOccurrences(of: "{enabled}", with: String(plugins.filter { $0["enabled"].bool }.count))
            .replacingOccurrences(of: "{total}", with: String(plugins.count))
    }
    /// The WebView's state line: project enablement, runtime readiness, contents.
    static func pluginState(_ plugin: SettingsValue) -> String {
        let runtime = ["ready": "运行环境就绪", "unavailable": "运行环境不可用"][plugin["runtime_status"].string] ?? "无需运行环境"
        return [localized(plugin["enabled"].bool ? "已为当前项目启用" : "未为当前项目启用"), localized(runtime),
                "\(plugin["skill_count"].integer) " + localized("技能"), "\(plugin["mcp_server_count"].integer) MCP"].joined(separator: " · ")
    }
    private var browser: some View {
        VStack(spacing: 20) {
        NativeSettingsGroup(title: "真实浏览器") {
            let status = model.values["browser_extension_status"] ?? .null
            Text(status["connected"].bool ? "扩展已连接" : "扩展未连接").fontWeight(.semibold)
            Text("当前版本 \(status["current_version"].string) · 内置版本 \(status["bundled_version"].string)").foregroundStyle(.secondary)
            if !status["error"].string.isEmpty { Text(status["error"].string).foregroundStyle(.red) }
            HStack {
                Button(localized("扩展管理页")) { Task { _ = await model.run("open_browser_extension_page", refresh: false, success: "已打开扩展管理页") } }
                Button(localized("更新扩展")) { Task { _ = await model.run("update_browser_extension", success: "扩展更新完成，请检查连接状态") } }
            }
            toggle("自动打开浏览器", hint: "使用浏览器工具时，如果 Chrome/Chromium 没有在运行，就启动它，让已安装的 Wisp 扩展重新连上。使用你现有的用户资料，不会另开一套自动化浏览器。", value: model.values["get_browser_auto_launch"]?.bool ?? false, command: "set_browser_auto_launch", args: [:])
            toggle("自动关闭浏览器标签页", hint: "一轮对话结束后，关闭本轮 Wisp 打开的标签页。你原来就已经打开的标签页不会动。", value: model.values["get_browser_auto_close_tabs"]?.bool ?? false, command: "set_browser_auto_close_tabs", args: [:])
        }
        NativeSettingsColumns {
        NativeSettingsGroup(title: "阻止访问") {
            NativeSettingsField(field: .init(key: "block", label: "阻止访问", kind: .records([.init(key: "host", label: "域名"), .init(key: "reason", label: "原因")])), value: model.binding("get_browser_url_filters", "block"))
        }
        } second: {
        NativeSettingsGroup(title: "优先访问") {
            NativeSettingsField(field: .init(key: "prefer", label: "优先访问", kind: .records([.init(key: "host", label: "域名"), .init(key: "reason", label: "原因")])), value: model.binding("get_browser_url_filters", "prefer"))
        }
        }
            Button(localized("保存 URL 规则")) { Task { _ = await model.run("set_browser_url_filters", ["filters": model.values["get_browser_url_filters"] ?? .null]) } }.buttonStyle(NativeSettingsButtonStyle(primary: true))
        }
    }
    private var connectors: some View {
        NativeSettingsGroup(title: "内置连接器与工具审批") {
            // Custom connections are listed under 自定义 MCP; this group is bundled only, as in the WebView.
            ForEach(Array((model.values["list_connectors"]?["connectors"].array ?? []).filter { $0["kind"].string == "bundled" }.enumerated()), id: \.offset) { _, row in
                DisclosureGroup(row["name"].string) {
                    VStack(alignment: .leading, spacing: 12) {
                        Text(row["description_zh"].string.isEmpty ? row["description"].string : row["description_zh"].string).foregroundStyle(.secondary)
                        toggle("启用", value: row["enabled"].bool, command: "set_connector_enabled", args: ["key": row["key"]])
                        toggle("跳过连接器审批", value: row["skip_approvals"].bool, command: "set_connector_skip_approvals", args: ["key": row["key"]])
                        toolModes(row)
                    }.padding(10)
                }
            }
        }
    }
    private func toolModes(_ connector: SettingsValue) -> some View {
        ForEach(Array(connector["tools"].array.enumerated()), id: \.offset) { _, tool in
            Picker(tool["name"].string, selection: Binding(get: { tool["mode"].string }, set: { mode in Task { _ = await model.run("set_tool_approval", ["tool": tool["name"], "mode": .string(mode)]) } })) { Text(localized("允许")).tag("allow"); Text(localized("询问")).tag("ask"); Text(localized("禁止")).tag("deny") }
        }
    }
    private var mcp: some View {
        NativeSettingsGroup(title: "自定义 MCP") {
            HStack {
                Button(localized("添加命令连接")) { editMCP(.object(["id": .string(UUID().uuidString), "name": .string(""), "enabled": .bool(true), "transport": .object(["kind": .string("stdio"), "command": .string(""), "args": .array([]), "env": .array([])])]), new: true) }
                Button(localized("添加 HTTP 连接")) { editMCP(.object(["id": .string(UUID().uuidString), "name": .string(""), "enabled": .bool(true), "transport": .object(["kind": .string("http"), "url": .string(""), "auth": .string("none"), "headers": .array([])])]), new: true) }
            }
            ForEach(Array((model.values["list_mcp_connections"]?["connections"].array ?? []).enumerated()), id: \.offset) { _, conn in
                HStack {
                    toggle(conn["name"].string, value: conn["enabled"].bool, command: "set_mcp_connection_enabled", args: ["id": conn["id"]])
                    Spacer()
                    Button(localized("编辑")) { editMCP(conn) }
                    Button(localized("测试")) { Task { _ = await model.run(conn["transport"]["auth"].string == "oauth" ? "test_oauth_mcp_connection" : "test_mcp_connection", ["conn": conn], refresh: false, success: "连接成功，工具列表已获取") } }
                }
                // The custom connector's key is this connection's id.
                if let connector = model.values["list_connectors"]?["connectors"].array.first(where: { $0["kind"].string == "custom" && $0["key"].string == conn["id"].string }), !connector["tools"].array.isEmpty {
                    DisclosureGroup(localized("工具审批")) { VStack(alignment: .leading, spacing: 12) { toolModes(connector) }.padding(10) }
                }
            }
        }
    }
    private func editMCP(_ conn: SettingsValue, new: Bool = false) {
        let secrets: [SettingsField] = [.init(key: "name", label: "名称"), .init(key: "value", label: "新值", kind: .secure, hint: "留空保留已有值；移除此项可删除凭据。")]
        let transport: [SettingsField] = conn["transport"]["kind"].string == "stdio" ? [
            .init(key: "command", label: "启动命令"), .init(key: "args", label: "参数", kind: .lines), .init(key: "cwd", label: "工作目录", kind: .path), .init(key: "env", label: "环境变量", kind: .records(secrets))
        ] : [.init(key: "url", label: "服务 URL"), .init(key: "auth", label: "认证", kind: .choice([("none", "无 / 自定义请求头"), ("oauth", "OAuth 浏览器授权")])), .init(key: "headers", label: "请求头", kind: .records(secrets))]
        model.editor = SettingsEditor(title: "MCP 连接", draft: conn, fields: [.init(key: "name", label: "名称"), .init(key: "enabled", label: "启用", kind: .toggle), .init(key: "transport", label: "传输", kind: .object(transport))], command: new ? "add_mcp_connection" : "update_mcp_connection", parameter: "conn", destructiveCommand: new ? nil : "delete_mcp_connection", destructiveArgs: ["id": conn["id"]])
    }
}
