import SwiftUI
import WispProjectBrowser

struct NativeModelSettings: View {
    @ObservedObject var model: NativeSettingsModel
    @State private var query = ""
    @State private var showAgents = false
    @Environment(\.colorScheme) private var scheme
    @State private var acpInfo: SettingsValue?
    @State private var testedAgent: SettingsValue?
    @State private var authTerminal: String?
    private struct SubscriptionPresentation: Identifiable {
        let id = UUID()
        var profile: SettingsValue = .null
    }
    @State private var subscription: SubscriptionPresentation?
    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            if model.section == .credentials { credentials } else {
                HStack(spacing: 6) {
                    tab("API 模型", count: model.values["list_models"]?.array.count ?? 0, agents: false)
                    tab("ACP Agents", count: model.values["list_acp_agents"]?.array.count ?? 0, agents: true)
                    Spacer()
                    Button(localized(showAgents ? "添加智能体" : "添加 API 接入")) {
                        if showAgents { editAgent(.object(["id": .string(""), "label": .string(""), "command": .string(""), "args": .array([])])) }
                        else { addModel() }
                    }.buttonStyle(NativeSettingsButtonStyle(primary: true))
                }
                Text(localized("模型配置由所有项目共享。默认模型用于新建对话，列表操作立即保存。")).font(WispDesign.font(size: 12)).foregroundStyle(.secondary)
                if showAgents { agents } else { models }
            }
        }
        .sheet(item: $subscription) { presentation in
            NativeCodexLoginSheet(model: NativeCodexLoginModel(client: model.client, profile: presentation.profile), saved: {
                Task { await model.load() }
            }, close: { warning in
                guard subscription?.id == presentation.id else { return }
                subscription = nil
                if let warning { model.error = warning }
            })
        }
    }

    private func tab(_ title: String, count: Int, agents: Bool) -> some View {
        Button { showAgents = agents } label: {
            HStack(spacing: 8) { Text(localized(title)); Text("\(count)").font(WispDesign.font(size: 11)).padding(.horizontal, 6).padding(.vertical, 2).background(Color.primary.opacity(0.05), in: Capsule()) }
        }.buttonStyle(NativeSettingsButtonStyle(primary: showAgents == agents, compact: true))
    }

    private func addModel(url: String = "https://api.openai.com/v1", name: String = "") {
        editModel(.object(["id": .string(""), "provider": .string("openai"), "api_url": .string(url), "model": .string(name), "label": .string(name), "context_window": .integer(128000), "max_tokens": .integer(0), "send_user_agent": .bool(true)]))
    }

    private var models: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(alignment: .center, spacing: 10) {
                Text(localized("快速接入")).font(WispDesign.font(size: 12)).foregroundStyle(.secondary)
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        Button("ChatGPT Plus / Pro") { subscription = SubscriptionPresentation() }
                            .buttonStyle(NativeSettingsButtonStyle(compact: true))
                        ForEach(WispDesign.modelPresets, id: \.self) { preset in
                            Button(preset["label"] ?? "") { addModel(url: preset["url"] ?? "", name: preset["model"] ?? "") }.buttonStyle(NativeSettingsButtonStyle(compact: true))
                        }
                    }
                }
            }
            TextField(localized("搜索名称、服务商或模型"), text: $query).textFieldStyle(NativeSettingsTextFieldStyle())
            let rows = (model.values["list_models"]?.array ?? []).filter { query.isEmpty || ($0["label"].string + $0["model"].string + $0["provider"].string).localizedCaseInsensitiveContains(query) }
            VStack(spacing: 0) {
                ForEach(Array(rows.enumerated()), id: \.offset) { index, row in
                    HStack(spacing: 12) {
                        Button { editModel(row) } label: {
                            HStack(spacing: 8) {
                                Text(row["label"].string.isEmpty ? row["model"].string : row["label"].string).fontWeight(.semibold).lineLimit(1)
                                if row["supports_vision"].bool { Text("Vision").font(WispDesign.font(size: 10)).foregroundStyle(.secondary).padding(4).background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 4)) }
                                Spacer(minLength: 0)
                            }.contentShape(Rectangle())
                        }.buttonStyle(.plain)
                        if row["active"].bool {
                            Text(localized("默认模型")).font(WispDesign.font(size: 12)).foregroundStyle(WispDesign.color("clay", scheme)).padding(.horizontal, 10).padding(.vertical, 5).background(WispDesign.color("clay", scheme).opacity(0.08), in: Capsule())
                        } else {
                            Button(localized("设为默认模型")) { Task { _ = await model.run("set_active_model", ["id": row["id"]]) } }.buttonStyle(NativeSettingsButtonStyle(compact: true))
                        }
                        Button { editModel(row) } label: { WispIcon(name: "chevron-right", size: 15) }.buttonStyle(.plain).accessibilityLabel(localized("编辑") + row["label"].string)
                    }.padding(.horizontal, 20).padding(.vertical, 18)
                    .contextMenu {
                        Button(localized("编辑")) { editModel(row) }
                        if row["provider"].string == "openai_codex" {
                            Button(localized("重新登录 ChatGPT…")) { subscription = SubscriptionPresentation(profile: row) }
                        }
                        Button(localized("上移")) { moveModel(row["id"].string, offset: -1) }.disabled(index == 0 || !query.isEmpty)
                        Button(localized("下移")) { moveModel(row["id"].string, offset: 1) }.disabled(index == rows.count - 1 || !query.isEmpty)
                    }
                    if index < rows.count - 1 { Divider().padding(.horizontal, 20) }
                }
                if rows.isEmpty && !model.loading { Text(localized("没有匹配的模型配置。")).foregroundStyle(.secondary).padding(24) }
            }.background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 14))
                .overlay(RoundedRectangle(cornerRadius: 14).stroke(WispDesign.color("border", scheme)))
        }
    }

    private func moveModel(_ id: String, offset: Int) {
        var rows = model.values["list_models"]?.array ?? []
        guard let index = rows.firstIndex(where: { $0["id"].string == id }), rows.indices.contains(index + offset) else { return }
        rows.swapAt(index, index + offset)
        Task { _ = await model.run("reorder_models", ["ids": .array(rows.map { $0["id"] })]) }
    }

    private func editModel(_ row: SettingsValue) {
        model.editor = SettingsEditor(title: row["id"].string.isEmpty ? "添加 API 接入" : "编辑模型", draft: row, fields: [
            .init(key: "label", label: "显示名称"),
            .init(key: "provider", label: "协议", kind: .choice([("openai", "OpenAI Chat Completions"), ("openai_responses", "OpenAI Responses"), ("anthropic", "Anthropic"), ("openai_codex", "ChatGPT 订阅")])),
            .init(key: "api_url", label: "API 根地址"),
            .init(key: "endpoint_suffix", label: "接口后缀"),
            .init(key: "model", label: "模型 ID"),
            .init(key: "key", label: "API 密钥", kind: .secure, hint: "留空保留现有密钥，使用现有凭据存储。"),
            .init(key: "max_tokens", label: "最大输出 Token", kind: .integer, hint: "0 使用默认值；保存时由模型目录校验上限。"),
            .init(key: "context_window", label: "上下文容量", kind: .integer),
            .init(key: "reasoning_effort", label: "推理强度", kind: .choice([("", "默认"), ("none", "None"), ("minimal", "Minimal"), ("low", "Low"), ("medium", "Medium"), ("high", "High"), ("xhigh", "XHigh"), ("max", "Max"), ("ultra", "Ultra")])),
            .init(key: "service_tier", label: "服务级别", kind: .choice([("", "默认"), ("priority", "Priority"), ("flex", "Flex")])),
            .init(key: "supports_vision", label: "支持图片输入", kind: .toggle),
            .init(key: "use_for_vision", label: "用于图片分析", kind: .toggle),
            .init(key: "use_for_image_generation", label: "用于图片生成", kind: .toggle),
            .init(key: "use_for_video_generation", label: "用于视频生成", kind: .toggle),
            .init(key: "image_size", label: "图片尺寸"),
            .init(key: "image_quality", label: "图片质量"),
            .init(key: "image_aspect_ratio", label: "图片比例"),
            .init(key: "image_resolution", label: "图片分辨率"),
            .init(key: "video_duration_secs", label: "视频时长（秒）", kind: .integer),
            .init(key: "video_aspect_ratio", label: "视频比例"),
            .init(key: "video_resolution", label: "视频分辨率"),
            .init(key: "send_user_agent", label: "发送 User-Agent", kind: .toggle),
            .init(key: "user_agent", label: "User-Agent"),
            .init(key: "send_session_id", label: "发送会话标识", kind: .toggle),
            .init(key: "session_header_name", label: "会话请求头名称")
        ].filter { row["provider"].string != "openai_codex" || $0.key != "key" }, command: "save_model", parameter: "profile", destructiveCommand: row["id"].string.isEmpty ? nil : "remove_model", destructiveArgs: ["id": row["id"]])
    }

    private var agents: some View {
        NativeSettingsGroup(title: "ACP 智能体") {
            ForEach(Array((model.values["list_acp_agents"]?.array ?? []).enumerated()), id: \.offset) { _, agent in
                HStack {
                    VStack(alignment: .leading) { Text(agent["label"].string).fontWeight(.semibold); Text(agent["command"].string).font(.caption).foregroundStyle(.secondary) }
                    Spacer()
                    Button(localized("编辑")) { editAgent(agent) }
                    Button(localized("测试连接")) { Task { testedAgent = agent; acpInfo = await model.run("test_acp_agent", ["id": agent["id"]], refresh: false, success: "连接测试完成") } }
                }
            }
            if let acpInfo, let testedAgent {
                Text(acpInfo["implementation"]["name"].string).foregroundStyle(.secondary)
                ForEach(Array(acpInfo["authMethods"].array.enumerated()), id: \.offset) { _, method in
                    Button(localized("授权：") + method["name"].string) { Task {
                        if let result = await model.run("authenticate_acp_agent", ["id": testedAgent["id"], "methodId": method["id"]], refresh: false, success: "授权流程已启动"), result != .null { authTerminal = result["id"].string }
                    } }.disabled(method["type"].string == "env_var")
                }
            }
            if let authTerminal { NativeAuthTerminal(model: model, sessionID: authTerminal) { self.authTerminal = nil } }
        }
    }
    private func editAgent(_ agent: SettingsValue) {
        model.editor = SettingsEditor(title: "ACP 智能体", draft: agent, fields: [.init(key: "label", label: "名称"), .init(key: "command", label: "可执行文件"), .init(key: "args", label: "启动参数", kind: .lines, hint: "每行一个参数，保留含空格的单个参数。")], command: "save_acp_agent", parameter: "profile", destructiveCommand: agent["id"].string.isEmpty ? nil : "remove_acp_agent", destructiveArgs: ["id": agent["id"]])
    }

    private func credentialName(_ id: String) -> String {
        ["openalex_api_key": "OpenAlex API 密钥", "infinisynapse_api_key": "InfiniSynapse API 密钥", "scimaster_api_key": "SciMaster API 密钥", "ncbi_api_key": "NCBI API 密钥", "ncbi_email": "NCBI 联系邮箱"][id] ?? id
    }
    private var credentials: some View {
        VStack(alignment: .leading, spacing: 22) {
            NativeSettingsGroup(title: "服务凭据") {
                Text(localized("凭据通过现有密钥存储保存。页面只读取配置状态，不回显密钥。")).foregroundStyle(.secondary)
                ForEach(Array((model.values["credential_status"]?.array ?? []).enumerated()), id: \.offset) { _, row in
                    let values = row.array
                    if values.count == 2 {
                        HStack {
                            Text(credentialName(values[0].string)).fontWeight(.medium)
                            Text(values[1].bool ? "已配置" : "未配置").foregroundStyle(.secondary).font(.caption)
                            Spacer()
                            Button(localized("配置")) { model.editor = SettingsEditor(title: credentialName(values[0].string), draft: .object(["value": .string("")]), fields: [.init(key: "value", label: values[0].string == "ncbi_email" ? "邮箱" : "密钥 / Token", kind: values[0].string == "ncbi_email" ? .text : .secure, hint: "留空提交会清除该服务凭据。")], command: "set_credential", parameter: nil, extra: ["id": values[0]]) }
                        }
                    }
                }
            }
            NativeSettingsGroup(title: "自定义凭据") {
                Button(localized("添加凭据")) { model.editor = SettingsEditor(title: "自定义凭据", draft: .object([:]), fields: [.init(key: "name", label: "名称"), .init(key: "env_var", label: "环境变量名"), .init(key: "value", label: "值", kind: .secure)], command: "add_custom_credential", parameter: nil) }
                ForEach(Array((model.values["list_custom_credentials"]?.array ?? []).enumerated()), id: \.offset) { _, row in
                    HStack {
                        VStack(alignment: .leading) { Text(row["name"].string).fontWeight(.medium); Text(row["env_var"].string).font(.caption).foregroundStyle(.secondary) }
                        Spacer()
                        Button(localized("管理")) { model.editor = SettingsEditor(title: row["name"].string, draft: .object(["name": row["name"], "env_var": row["env_var"], "value": .string("")]), fields: [.init(key: "name", label: "名称"), .init(key: "env_var", label: "环境变量名"), .init(key: "value", label: "新值", kind: .secure)], command: "add_custom_credential", parameter: nil, destructiveCommand: "remove_custom_credential", destructiveArgs: ["id": row["id"]]) }
                    }
                }
            }
        }
    }
}
