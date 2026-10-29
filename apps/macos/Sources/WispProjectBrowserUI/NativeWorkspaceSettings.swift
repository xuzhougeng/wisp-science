import AppKit
import SwiftUI
import WispProjectBrowser

struct NativeWorkspaceSettings: View {
    @ObservedObject var model: NativeSettingsModel
    @State private var confirmation: SettingsOperation?
    @State private var detail: SettingsValue?
    @State private var sessions: [SettingsValue] = []
    @State private var sessionProject: String?
    @State private var sessionTotal = 0
    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            switch model.section {
            case .memory: memory
            case .channels: NativeChannelSettings(model: model)
            case .permissions: permissions
            case .environments: environments
            case .storage: storage
            default: usage
            }
            if let detail { NativeSettingsSummary(value: detail) }
        }
        .confirmationDialog(confirmation?.title ?? "确认操作", isPresented: Binding(get: { confirmation != nil }, set: { if !$0 { confirmation = nil } }), presenting: confirmation) { operation in
            Button(operation.title, role: .destructive) { Task { _ = await model.run(operation.command, operation.args, success: "操作完成") } }
            Button(localized("取消"), role: .cancel) {}
        }
    }
    private var memory: some View {
        VStack(spacing: 22) {
            NativeSettingsColumns {
            NativeSettingsGroup(title: "项目记忆") {
                let view = model.values["get_memory_view"] ?? .null
                NativeSettingsSwitch(title: "启用记忆", isOn: Binding(get: { view["enabled"].bool }, set: { enabled in Task { _ = await model.run("set_memory_enabled", ["enabled": .bool(enabled)]) } }))
                HStack {
                    Button(localized("添加记忆文件")) { memoryEditor(name: "", content: "", new: true) }
                    Button(localized("清空项目记忆…")) { confirmation = .init(title: "清空项目记忆", command: "clear_memory") }
                }
                ForEach(Array(view["files"].array.enumerated()), id: \.offset) { _, file in
                    HStack {
                        Text(file["name"].string); Spacer(); Text(bytes(file["bytes"])).foregroundStyle(.secondary)
                        Button(localized("编辑")) { Task { if let content = await model.run("read_memory_file", ["name": file["name"]], refresh: false, success: "") { memoryEditor(name: file["name"].string, content: content.string) } } }
                    }
                }
            }
            } second: {
            NativeSettingsGroup(title: "全局记忆") {
                Button(localized("添加全局记忆")) { model.editor = SettingsEditor(title: "全局记忆", draft: .object(["content": .string("")]), fields: [.init(key: "content", label: "内容", kind: .multiline)], command: "create_global_memory", parameter: nil) }
                ForEach(Array((model.values["get_memory_view"]?["global_memories"].array ?? []).enumerated()), id: \.offset) { _, item in
                    HStack(alignment: .top) {
                        Text(item["content"].string).textSelection(.enabled); Spacer()
                        Button(localized("编辑")) { model.editor = SettingsEditor(title: "全局记忆", draft: .object(["content": item["content"]]), fields: [.init(key: "content", label: "内容", kind: .multiline)], command: "update_global_memory", parameter: nil, extra: ["id": item["id"]], destructiveCommand: "delete_global_memory", destructiveArgs: ["id": item["id"]]) }
                    }
                }
            }
            }
            NativeSettingsGroup(title: "自动失败分析") {
                ForEach([SettingsField(key: "enabled", label: "启用", kind: .toggle), .init(key: "failure_rate_threshold", label: "失败率阈值（%）", kind: .integer), .init(key: "minimum_failures", label: "最少失败次数", kind: .integer)]) { field in NativeSettingsField(field: field, value: model.binding("get_auto_failure_analysis_settings", field.key)) }
                Button(localized("保存")) { Task { _ = await model.run("set_auto_failure_analysis_settings", ["settings": model.values["get_auto_failure_analysis_settings"] ?? .null]) } }
            }
        }
    }
    private func memoryEditor(name: String, content: String, new: Bool = false) {
        model.editor = SettingsEditor(title: "项目记忆", draft: .object(["name": .string(name), "content": .string(content)]), fields: [.init(key: "name", label: "文件名", hint: "例如 MEMORY.md 或日期日志；后端会检查允许的文件范围。"), .init(key: "content", label: "内容", kind: .multiline)], command: "write_memory_file", parameter: nil, destructiveCommand: new ? nil : "delete_memory_file", destructiveArgs: ["name": .string(name)])
    }
    private var permissions: some View {
        NativeSettingsGroup(title: "工具权限") {
            Picker(localized("审批模式"), selection: Binding(get: { model.values["list_connectors"]?["scope"].string ?? "ask" }, set: { scope in Task { _ = await model.run("set_approval_scope", ["scope": .string(scope)]) } })) {
                Text(localized("按工具设置询问")).tag("ask"); Text(localized("自动批准安全操作")).tag("auto"); Text(localized("完全自动批准")).tag("full")
            }
            Text(localized("完全自动批准也会放行危险命令。已禁止的工具仍保持禁止。")).font(.caption).foregroundStyle(.secondary)
            Button(localized("撤销所有授权…")) { confirmation = .init(title: "撤销所有授权", command: "revoke_all_approval_grants") }
            ForEach(Array((model.values["list_approval_grants"]?.array ?? []).enumerated()), id: \.offset) { _, grant in
                HStack {
                    VStack(alignment: .leading, spacing: 5) { Text(grant["label"].string); Text("\(localized(["session": "会话", "project": "项目", "global": "全局"][grant["scope"].string] ?? grant["scope"].string)) · \(grant["target"].string)").font(.caption).foregroundStyle(.secondary).textSelection(.enabled) }
                    Spacer()
                    Button(localized("撤销…")) { confirmation = .init(title: "撤销此授权", command: "revoke_approval_grant", args: ["scope": grant["scope"], "kind": grant["kind"], "target": grant["target"], "sessionId": grant["session_id"], "projectId": grant["project_id"]]) }
                }
            }
        }
    }
    private var environments: some View {
        VStack(spacing: 22) {
            NativeSettingsGroup(title: "执行环境") {
                Picker(localized("默认远程环境"), selection: Binding(get: { model.values["get_default_execution_context"]?.string ?? "" }, set: { id in Task { _ = await model.run("set_default_execution_context", ["contextId": id.isEmpty ? .null : .string(id)]) } })) {
                    Text(localized("无（始终可使用本机）")).tag("")
                    ForEach(Array((model.values["list_execution_contexts"]?.array ?? []).filter { $0["kind"].string != "local" }.enumerated()), id: \.offset) { _, row in Text(row["label"].string.isEmpty ? row["id"].string : row["label"].string).tag(row["id"].string) }
                }
                ForEach(Array((model.values["list_execution_contexts"]?.array ?? []).enumerated()), id: \.offset) { _, context in
                    VStack(alignment: .leading, spacing: 10) {
                        HStack {
                            // Same title rule as the WebView environments list (`compute.local`).
                            Text(context["kind"].string == "local" ? localized("本地") : context["label"].string.isEmpty ? context["id"].string : context["label"].string).fontWeight(.semibold); Spacer()
                            Button(localized("探测")) { Task { detail = await model.run("probe_execution_context", ["contextId": context["id"]], refresh: false, success: "探测完成") } }
                            Button(localized("解释器")) { model.editor = SettingsEditor(title: "解释器路径", draft: .object(["python_executable": context["config_json"].decodedJSON["python_executable"], "rscript_executable": context["config_json"].decodedJSON["rscript_executable"]]), fields: [.init(key: "python_executable", label: "Python"), .init(key: "rscript_executable", label: "Rscript")], command: "update_execution_context_interpreters", parameter: nil, extra: ["contextId": context["id"]]) }
                            Button(localized("存储路径")) { Task { if let prefs = await model.run("get_context_storage_prefs", ["contextId": context["id"]], refresh: false, success: "") { model.editor = SettingsEditor(title: "执行环境存储", draft: prefs, fields: [.init(key: "remote_data_root", label: "远程数据目录"), .init(key: "remote_workdir_root", label: "远程运行目录"), .init(key: "local_results_dir", label: "本地结果目录", kind: .path)], command: "set_context_storage_prefs", parameter: nil, extra: ["contextId": context["id"]]) } } }.disabled(model.projectID == nil)
                            Button(localized("清理报告")) { Task { detail = await model.run("context_disposal_report", ["contextId": context["id"]], refresh: false, success: "报告已生成") } }.disabled(model.projectID == nil)
                        }
                        Text(context["kind"].string).foregroundStyle(.secondary)
                    }
                    Divider()
                }
            }
            NativeSettingsGroup(title: "SSH 主机") {
                HStack { Button(localized("添加主机")) { editHost(.object(["alias": .string(""), "port": .integer(22), "auth_method": .string("key")]), new: true) }; Button(localized("导入 SSH 配置")) { Task { _ = await model.run("import_ssh_config_hosts", success: "导入完成") } } }
                ForEach(Array((model.values["list_ssh_hosts"]?.array ?? []).enumerated()), id: \.offset) { _, host in
                    HStack { Text(host["alias"].string).fontWeight(.semibold); Text(host["host_name"].string).foregroundStyle(.secondary); Spacer(); Button(localized("编辑")) { editHost(host) }; Button(localized("测试连接")) { Task { _ = await model.run("test_ssh_connection", ["host": host], refresh: false, success: "SSH 连接成功") } } }
                }
            }
            NativeSettingsGroup(title: "服务器间信任") {
                ForEach(Array((model.values["list_ssh_trust_edges"]?.array ?? []).enumerated()), id: \.offset) { _, edge in
                    HStack { Text("\(edge["source_context_id"].string) → \(edge["destination_context_id"].string)"); Spacer(); Button(localized("撤销…")) { confirmation = .init(title: "撤销服务器信任", command: "revoke_ssh_trust_edge", args: ["sourceContextId": edge["source_context_id"], "destinationContextId": edge["destination_context_id"]]) } }
                }
            }
        }
    }
    private func editHost(_ host: SettingsValue, new: Bool = false) {
        model.editor = SettingsEditor(title: "SSH 主机", draft: host, fields: [.init(key: "alias", label: "别名"), .init(key: "host_name", label: "主机地址"), .init(key: "user", label: "用户名"), .init(key: "port", label: "端口", kind: .integer), .init(key: "auth_method", label: "认证方式", kind: .choice([("key", "密钥 / SSH 配置"), ("password", "密码")])), .init(key: "identity_file", label: "私钥文件路径", kind: .file), .init(key: "password", label: "密码", kind: .secure, hint: "留空保留已有密码。私钥只保存路径，密码使用现有凭据存储。"), .init(key: "notes", label: "备注", kind: .multiline)], command: "add_ssh_host", parameter: "host", destructiveCommand: new ? nil : "remove_ssh_host", destructiveArgs: ["alias": host["alias"]])
    }
    private var storage: some View {
        VStack(spacing: 22) {
            NativeSettingsGroup(title: "本地存储") {
                let usage = model.values["get_storage_usage"] ?? .null
                // A missing read is not zero bytes.
                Text(model.values["get_storage_usage"] == nil ? localized("计算中…") : bytes(usage["total_bytes"])).font(WispDesign.font(size: 28, weight: .semibold))
                Text(usage["data_dir"].string).textSelection(.enabled).foregroundStyle(.secondary)
                Button(localized("在 Finder 中显示")) { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: usage["data_dir"].string)]) }
                ForEach(Array(usage["entries"].array.enumerated()), id: \.offset) { _, item in HStack { Text(localized(NativeUsageRows.storageLabel(item["key"].string))); Spacer(); Text(bytes(item["bytes"])) } }
                ForEach(Array(usage["projects"].array.enumerated()), id: \.offset) { _, item in HStack { Text(item["name"].string); Spacer(); Text(bytes(item["bytes"])); Button(localized("定位")) { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: item["path"].string)]) } } }
            }
            if model.projectID != nil {
                NativeSettingsGroup(title: "项目保留策略") {
                    Text(localized("留空使用默认值。修改策略不会立即删除文件。")).foregroundStyle(.secondary)
                    ForEach([SettingsField(key: "run_retention_days", label: "成功运行保留天数", kind: .integer), .init(key: "failed_run_retention_days", label: "失败运行保留天数", kind: .integer), .init(key: "orphan_file_retention_days", label: "孤立文件保留天数", kind: .integer)]) { field in NativeSettingsField(field: field, value: model.binding("get_project_run_retention", field.key)) }
                    Button(localized("保存保留策略")) { Task { let prefs = model.values["get_project_run_retention"] ?? .null; _ = await model.run("set_project_run_retention", ["runRetentionDays": prefs["run_retention_days"], "failedRunRetentionDays": prefs["failed_run_retention_days"], "orphanFileRetentionDays": prefs["orphan_file_retention_days"]]) } }
                }
            }
        }
    }
    private var usage: some View {
        VStack(spacing: 22) {
            NativeSettingsGroup(title: "项目 Token 用量") {
                ForEach(Array((model.values["get_token_usage"]?["workspaces"].array ?? []).enumerated()), id: \.offset) { _, row in
                    HStack {
                        Text(row["name"].string).fontWeight(.medium); Spacer()
                        Text("输入 \(row["input"].string) · 输出 \(row["output"].string) · 缓存 \(row["cached"].string)").font(.caption)
                        Button(localized("查看会话")) { Task { sessionProject = row["project_id"].string; sessions = []; await loadSessions() } }
                    }
                }
                if sessionProject != nil {
                    Divider()
                    ForEach(Array(sessions.enumerated()), id: \.offset) { _, row in HStack { Text(row["title"].string); Spacer(); Text("\(row["input"].string) / \(row["output"].string)").font(.caption) } }
                    if sessions.count < sessionTotal { Button(localized("加载更多会话")) { Task { await loadSessions() } } }
                }
            }
            let overview = model.values["get_token_usage"] ?? .null
            usageGroup("模型用量", NativeUsageRows.models(overview["models"].array, profiles: model.values["list_models"]?.array ?? []))
            usageGroup("工具调用", overview["tools"].array.map { ($0["name"].string, $0["calls"].integer) })
            usageGroup("每日 Token", NativeUsageRows.pastDays(overview["days"].array))
        }
    }
    private func usageGroup(_ title: String, _ rows: [(label: String, value: Int64)]) -> some View {
        NativeSettingsGroup(title: title) { ForEach(Array(rows.enumerated()), id: \.offset) { _, row in HStack { Text(row.label); Spacer(); Text("\(row.value)").monospacedDigit() } } }
    }
    private func loadSessions() async {
        guard let id = sessionProject else { return }
        if let result = await model.run("get_session_token_usage", ["projectId": .string(id), "offset": .integer(Int64(sessions.count)), "limit": .integer(30)], refresh: false, success: "") { sessions += result["items"].array; sessionTotal = Int(result["total"].integer) }
    }
    private func bytes(_ value: SettingsValue) -> String { formatBytes(value.integer) }
}

/// Row shaping for Settings › Storage and Usage, kept identical to the WebView.
enum NativeUsageRows {
    /// The host pads the activity grid to the end of the current week.
    static func pastDays(_ days: [SettingsValue]) -> [(label: String, value: Int64)] {
        days.filter { !$0["future"].bool }.map { ($0["date"].string, $0["tokens"].integer) }
    }

    /// Mirrors `usage_model_slices` in ui/src/settings_view.rs: rows carry model
    /// profile ids, shown by the profile's model id and merged; eight or more
    /// labels keep the top seven plus one "其他模型" row.
    static func models(_ rows: [SettingsValue], profiles: [SettingsValue]) -> [(label: String, value: Int64)] {
        var merged: [String: Int64] = [:]
        for row in rows where row["tokens"].integer > 0 {
            let id = row["model"].string
            let label: String
            if let profile = profiles.first(where: { $0["id"].string == id }) {
                let model = profile["model"].string.trimmingCharacters(in: .whitespaces)
                label = model.isEmpty ? profile["label"].string : model
            } else {
                label = id == "unknown" ? localized("未知模型") : id
            }
            merged[label, default: 0] += row["tokens"].integer
        }
        var totals = merged.sorted { $0.value != $1.value ? $0.value > $1.value : $0.key < $1.key }.map { (label: $0.key, value: $0.value) }
        if totals.count > 8 {
            let rest = totals[7...].reduce(0) { $0 + $1.value }
            totals = Array(totals[..<7]) + [(label: localized("其他模型"), value: rest)]
        }
        return totals
    }

    /// Storage entry keys from `get_storage_usage`, with the WebView's legend names.
    static func storageLabel(_ key: String) -> String {
        ["database": "数据库（会话与工具结果）", "python": "Python 环境", "plugins": "插件", "workspace": "工作区文件", "other": "其他"][key] ?? key
    }
}

struct SettingsOperation: Identifiable {
    let id = UUID()
    var title: String
    var command: String
    var args: [String: SettingsValue] = [:]
}
