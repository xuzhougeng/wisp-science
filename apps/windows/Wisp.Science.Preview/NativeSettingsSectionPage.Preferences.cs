using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    internal static IReadOnlyDictionary<string, string> Titles { get; } = new Dictionary<string, string>
    {
        ["general"] = "常规", ["session"] = "对话", ["pet"] = "宠物",
        ["skills"] = "技能", ["connections"] = "连接", ["memory"] = "记忆",
        ["plugins"] = "插件", ["browser"] = "浏览器", ["credentials"] = "凭据", ["permissions"] = "权限",
        ["environments"] = "环境", ["storage"] = "存储", ["usage"] = "用量",
        ["quick-actions"] = "快捷操作", ["specialists"] = "专家", ["workflows"] = "工作流", ["models"] = "模型", ["channels"] = "远程接入", ["project"] = "项目设置"
    };
    private readonly HashSet<string> invalidFields = [];
    private void Boolean(JsonObject draft, string key, string label)
    {
        var toggle = new ToggleSwitch { Header = label, IsOn = B(draft, key) };
        toggle.Toggled += (_, _) => draft[key] = toggle.IsOn; Form.Children.Add(toggle);
    }
    private void Secure(JsonObject draft, string key, string label)
    {
        var input = new PasswordBox { Header = label, Password = S(draft, key) };
        input.PasswordChanged += (_, _) => draft[key] = input.Password; Form.Children.Add(input);
    }
    private void Integer(JsonObject draft, string key, string label, long minimum = 0, long maximum = int.MaxValue, bool nullable = false)
    {
        Field(label, draft[key]?.ToString() ?? "", value =>
        {
            if (nullable && string.IsNullOrWhiteSpace(value)) { draft[key] = null; invalidFields.Remove(label); }
            else if (long.TryParse(value, out var number) && number >= minimum && number <= maximum)
            { draft[key] = number; invalidFields.Remove(label); }
            else invalidFields.Add(label);
        });
    }
    private void Preference(string title, string read, string write, string? parameter, Action<JsonObject> fields)
    {
        var card = Card(title);
        if (model.Values[read] is not JsonObject prefs) { card.Children.Add(Mute("尚未读取，点击刷新重试。")); return; }
        card.Children.Add(Button("编辑" + title, () => { Editor(title, prefs, write, parameter, fields); return Task.CompletedTask; }));
    }
    private void Immediate(StackPanel card, string label, string read, string write)
    {
        if (model.Values[read] is JsonValue value && value.TryGetValue<bool>(out var enabled))
            Toggle(card, label, enabled, write, new());
    }
    private void General()
    {
        Preference("工作区与通知", "get_settings", "set_settings", "settings", d =>
        {
            Choice(Form, "语言", S(d, "locale"), [("zh", "简体中文"), ("en", "English")], v => d["locale"] = v);
            PathField(d, "workspace_dir", "工作目录（留空使用默认目录，下次启动生效）", directory: true);
            Boolean(d, "resume_last_session", "恢复最近会话"); Boolean(d, "notifications_enabled", "桌面通知");
        });
        Preference("输入与选择", "get_appearance_prefs", "set_appearance_prefs", "prefs", d =>
        { Boolean(d, "send_with_modifier", "使用 Ctrl+Enter 发送"); Boolean(d, "selection_popup_enabled", "选中文本快捷菜单"); });
        Preference("网络与软件源", "get_network_settings", "set_network_settings", "settings", d =>
        {
            Form.Children.Add(Mute("代理留空跟随系统，none 为直连。支持 HTTP、HTTPS 和 SOCKS5。"));
            foreach (var (key, title) in new[] { ("model_proxy_url", "模型 API 代理"), ("mcp_proxy_url", "MCP 代理"),
                ("command_proxy_url", "代码与命令代理"), ("conda_mirror_url", "Conda 镜像"), ("pip_index_url", "Python 软件源"), ("ca_bundle_path", "CA 证书路径") }) Text(d, key, title);
        });
        var local = Card("本地运行环境");
        if (model.Values["get_bootstrap_status"]?["local_environment"] is JsonObject environment)
        {
            if (S(environment, "warning").Length > 0) local.Children.Add(Warn(S(environment, "warning")));
            if (environment["paths"] is JsonObject paths) local.Children.Add(Button("编辑解释器路径", () =>
            {
                Editor("本地运行环境", paths, "save_local_environment_paths", "paths", d =>
                {
                    foreach (var (key, label) in new[] { ("python_executable", "Python"), ("rscript_executable", "Rscript"), ("node_executable", "Node.js"),
                        ("npm_executable", "npm"), ("uv_executable", "uv"), ("pixi_executable", "Pixi"), ("sci_executable", "sci") }) PathField(d, key, label);
                }); return Task.CompletedTask;
            }));
        }
        Action(local, "重新检测", "detect_local_environment", new());
        var updates = Card("桌面宿主更新");
        Immediate(updates, "自动检查更新", "get_update_check_enabled", "set_update_check_enabled");
        updates.Children.Add(Button("检查更新", async () =>
        {
            JsonNode? update = null;
            if (await model.InvokeAsync("check_for_updates", new(), v => update = v)) ShowUpdate(updates, update);
        }));
    }
    private void ShowUpdate(StackPanel target, JsonNode? update)
    {
        if (model.Closed || update == null) return;
        target.Children.Add(Mute(B(update, "update_available") ? "发现版本 " + S(update, "latest_version") : "当前已是最新版本"));
        target.Children.Add(Mute(S(update, "notes")));
        if (Uri.TryCreate(S(update, "release_url"), UriKind.Absolute, out var url) && url.Scheme == "https")
            target.Children.Add(new HyperlinkButton { Content = "发布说明", NavigateUri = url });
        if (!B(update, "update_available") || !B(update, "install_supported")) return;
        if (B(update, "downloaded")) target.Children.Add(Button("安装并重启桌面宿主", () =>
        { Confirm("安装更新将重启桌面宿主。继续？", () => _ = Run("install_update", new())); return Task.CompletedTask; }));
        else target.Children.Add(Button("下载并验证更新", async () =>
        {
            if (!await model.InvokeAsync("native_download_update", new())) return;
            JsonNode? next = null;
            if (await model.InvokeAsync("check_for_updates", new(), v => next = v)) { target.Children.Clear(); ShowUpdate(target, next); }
        }));
    }
    private void Session()
    {
        Preference("对话运行限制", "get_settings", "set_settings", "settings", d =>
        {
            Integer(d, "max_iter", "最大迭代次数（0 表示不限）");
            Boolean(d, "auto_continue", "达到迭代上限后自动继续"); Integer(d, "auto_continue_limit", "自动继续轮次");
            Boolean(d, "auto_compact", "自动压缩上下文"); Boolean(d, "follow_up_questions", "建议后续问题");
        });
        Immediate(Card("新会话默认设置"), "自动审核", "get_auto_review_enabled", "set_auto_review_enabled");
    }
    private void Pet()
    {
        Preference("桌宠", "get_settings", "set_settings", "settings", d =>
        { Boolean(d, "pet_enabled", "启用桌宠"); PathField(d, "pet_directory", "资源目录", directory: true); });
        var card = Card("运行状态");
        if (S(model.Values["get_pet"], "error").Length > 0) card.Children.Add(Warn(S(model.Values["get_pet"], "error")));
        var runtime = model.Values["get_pet_runtime_status"];
        if (runtime != null) card.Children.Add(Mute($"运行中 {(runtime["running"] as JsonArray)?.Count ?? 0} · 等待审批 {(runtime["waiting"] as JsonArray)?.Count ?? 0} · 审核中 {(runtime["reviewing"] as JsonArray)?.Count ?? 0}"));
    }
    private void Plugins()
    {
        foreach (var remote in new[] { false, true }) Form.Children.Add(Button(remote ? "从 URL 安装插件" : "从本地安装插件", () =>
        {
            Editor("安装插件", new JsonObject(), remote ? "install_plugin_url" : "install_plugin", null, d =>
            { if (remote) Text(d, "source_url", "下载地址"); else PathField(d, "src_path", "插件目录或归档路径", either: true); Text(d, "expected_sha256", remote ? "SHA-256" : "SHA-256（可选）"); }); return Task.CompletedTask;
        }));
        foreach (var row in Rows(model.Values["list_plugins"]))
        {
            var card = Card(S(row, "display_name") + " " + S(row, "version"));
            card.Children.Add(Mute(S(row, "description"))); card.Children.Add(Mute(S(row, "runtime_status"))); card.Children.Add(Mute(S(row, "source_uri")));
            foreach (var error in row["runtime_errors"] as JsonArray ?? []) card.Children.Add(Warn(error?.GetValue<string>() ?? ""));
            var args = new JsonObject { ["pluginId"] = S(row, "id"), ["version"] = S(row, "version") };
            Toggle(card, "在此项目启用", B(row, "enabled"), "set_plugin_enabled", (JsonObject)args.DeepClone());
            Action(card, "移除插件", "remove_plugin", args, true);
        }
    }
    private void Browser()
    {
        var card = Card("真实浏览器"); var status = model.Values["browser_extension_status"];
        if (status != null)
        {
            card.Children.Add(Mute(B(status, "connected") ? "扩展已连接" : "扩展未连接"));
            card.Children.Add(Mute("当前版本 " + S(status, "current_version") + " · 内置版本 " + S(status, "bundled_version")));
            if (S(status, "error").Length > 0) card.Children.Add(Warn(S(status, "error")));
        }
        Action(card, "打开扩展管理页", "open_browser_extension_page", new()); Action(card, "更新扩展", "update_browser_extension", new());
        Immediate(card, "需要时自动启动浏览器", "get_browser_auto_launch", "set_browser_auto_launch");
        Immediate(card, "自动关闭任务标签页", "get_browser_auto_close_tabs", "set_browser_auto_close_tabs");
        Preference("URL 访问规则", "get_browser_url_filters", "set_browser_url_filters", "filters", d =>
        { Records(d, "block", "阻止访问", [("host", "域名"), ("reason", "原因")]); Records(d, "prefer", "优先访问", [("host", "域名"), ("reason", "原因")]); });
    }
    private void Records(JsonObject draft, string key, string title, (string Key, string Label)[] fields)
    {
        var rows = draft[key] as JsonArray;
        if (rows == null) { rows = new JsonArray(); draft[key] = rows; }
        Form.Children.Add(Mute(title)); var list = new StackPanel { Spacing = 12 }; Form.Children.Add(list);
        void Add(JsonObject row)
        {
            var group = new StackPanel { Spacing = 6 };
            foreach (var (field, label) in fields)
            {
                var input = new TextBox { Header = label, Text = S(row, field) };
                input.TextChanged += (_, _) => row[field] = input.Text; group.Children.Add(input);
            }
            group.Children.Add(Button("移除", () => { rows.Remove(row); list.Children.Remove(group); return Task.CompletedTask; })); list.Children.Add(group);
        }
        foreach (var row in Rows(rows)) Add(row);
        Form.Children.Add(Button("添加" + title, () => { var row = new JsonObject(); rows.Add(row); Add(row); return Task.CompletedTask; }));
    }
    private void Credentials()
    {
        Form.Children.Add(Mute("凭据由桌面宿主保存到现有密钥存储；这里仅显示配置状态。"));
        foreach (var entry in model.Values["credential_status"] as JsonArray ?? [])
        {
            if (entry is not JsonArray row || row.Count != 2) continue;
            var id = row[0]!.GetValue<string>(); var card = Card(id);
            card.Children.Add(Mute(row[1]?.GetValue<bool>() == true ? "已配置" : "未配置"));
            card.Children.Add(Button("配置", () =>
            {
                Editor(id, new() { ["value"] = "" }, "set_credential", null, d =>
                { Form.Children.Add(Mute("提交空值会清除此服务凭据。")); if (id == "ncbi_email") Text(d, "value", "联系邮箱"); else Secure(d, "value", "新密钥 / Token"); }, new() { ["id"] = id });
                return Task.CompletedTask;
            }));
        }
        void EditCustom(JsonObject source) => Editor("自定义凭据", source, "add_custom_credential", null, d =>
        { Text(d, "name", "名称"); Text(d, "env_var", "环境变量名"); Secure(d, "value", "新值"); });
        Form.Children.Add(Button("添加自定义凭据", () => { EditCustom(new() { ["name"] = "", ["env_var"] = "", ["value"] = "" }); return Task.CompletedTask; }));
        foreach (var row in Rows(model.Values["list_custom_credentials"]))
        {
            var card = Card(S(row, "name")); card.Children.Add(Mute(S(row, "env_var")));
            card.Children.Add(Button("编辑", () => { EditCustom(new() { ["name"] = S(row, "name"), ["env_var"] = S(row, "env_var"), ["value"] = "" }); return Task.CompletedTask; }));
            Action(card, "移除凭据", "remove_custom_credential", new() { ["id"] = S(row, "id") }, true);
        }
    }
    private void Permissions()
    {
        var card = Card("工具权限");
        if (model.Values["list_connectors"] is JsonObject connectors)
            Choice(card, "审批模式", S(connectors, "scope"), [("ask", "按工具设置询问"), ("auto", "自动批准安全操作"), ("full", "完全自动批准")],
                scope => Confirm("更改审批模式？完全自动批准也会放行危险命令，已禁止的工具仍保持禁止。", () => _ = Run("set_approval_scope", new() { ["scope"] = scope })));
        Action(card, "撤销所有授权", "revoke_all_approval_grants", new(), true);
        foreach (var grant in Rows(model.Values["list_approval_grants"]))
        {
            var item = Card(S(grant, "label")); item.Children.Add(Mute(S(grant, "scope") + " · " + S(grant, "target")));
            Action(item, "撤销授权", "revoke_approval_grant", new() { ["scope"] = grant["scope"]?.DeepClone(), ["kind"] = grant["kind"]?.DeepClone(),
                ["target"] = grant["target"]?.DeepClone(), ["sessionId"] = grant["session_id"]?.DeepClone(), ["projectId"] = grant["project_id"]?.DeepClone() }, true);
        }
    }
}
