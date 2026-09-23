using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

/// Native editors for the settings reached from the capability summary.
internal sealed partial class NativeSettingsSectionPage : NativeActionPage
{
    private readonly NativeSettingsEditorModel model;
    private readonly string section;
    private readonly bool projectScoped;
    private readonly string? settingsProjectId;
    private INativeSettingsClient? settingsClient;
    private readonly Func<bool, Task<string?>>? pickPath;
    private readonly List<ComboBox> choices = [];
    private readonly StackPanel confirmation = new() { Spacing = 8 };
    private Action? pendingConfirmation;
    private readonly Button cancelAuthorization = new() { Content = "取消 OAuth 授权", Visibility = Visibility.Collapsed };
    public bool HasChanges => model.HasChanges || model.Draft != null && invalidFields.Count > 0;
    public bool Busy => model.Busy || channelBinding?.Busy == true;
    private static string S(JsonNode? row, string key) => row?[key]?.GetValue<string>() ?? "";
    private static bool B(JsonNode? row, string key) => row?[key]?.GetValue<bool>() == true;
    private static IEnumerable<JsonObject> Rows(JsonNode? value) => (value as JsonArray)?.OfType<JsonObject>() ?? [];
    public NativeSettingsSectionPage(INativeSettingsClient client, string? project, string section, WispDesign design, Action close, Func<bool, Task<string?>>? pickPath = null)
        : this(new NativeSettingsEditorModel(client, project), project, section, design, close, pickPath)
    {
        settingsClient = client;
        if (section == "channels") { channelBinding = new(client, project); channelBinding.Changed += ChannelChanged; }
        _ = Reload();
    }
    private NativeSettingsSectionPage(NativeSettingsEditorModel model, string? project, string section, WispDesign design, Action close, Func<bool, Task<string?>>? pickPath)
        : base(design, Titles[section], model, close)
    {
        this.model = model; this.projectScoped = project != null; settingsProjectId = project; this.section = section;
        this.pickPath = pickPath;
        cancelAuthorization.Click += async (_, _) => await model.CancelOAuthAsync();
        Notices.Children.Add(cancelAuthorization); Notices.Children.Add(confirmation); model.Changed += LockConfirmation;
    }
    private string[] Reads => section switch
    {
        "skills" => ["list_skills"], "connections" => ["list_connectors", "list_mcp_connections"],
        "memory" => ["get_memory_view", "get_auto_failure_analysis_settings"],
        "general" => ["get_settings", "get_appearance_prefs", "get_network_settings", "get_bootstrap_status", "get_update_check_enabled"],
        "session" => ["get_settings", "get_auto_review_enabled"],
        "pet" => ["get_settings", "get_pet_runtime_status", "get_pet"],
        "plugins" => ["list_plugins"],
        "browser" => ["get_browser_auto_launch", "get_browser_auto_close_tabs", "get_browser_url_filters", "browser_extension_status"],
        "credentials" => ["credential_status", "list_custom_credentials"],
        "permissions" => ["list_approval_grants", "list_connectors"],
        "environments" => ["list_execution_contexts", "list_ssh_hosts", "get_default_execution_context", "list_ssh_trust_edges"],
        "storage" => projectScoped ? ["get_storage_usage", "get_project_run_retention"] : ["get_storage_usage"],
        "usage" => ["get_token_usage"],
        "quick-actions" => ["list_quick_actions", "list_workflow_templates"],
        "specialists" => ["list_specialists", "list_models"],
        "workflows" => ["list_workflow_templates", "list_models", "list_skills"],
        "models" => ["list_models", "list_acp_agents"],
        "channels" => ["channels_status", "get_settings"],
        "project" => projectScoped ? ["get_project_settings"] : [],
        _ => throw new InvalidOperationException("Unknown settings section")
    };
    private async Task Reload()
    {
        if (model.Draft != null) return;
        await model.LoadAsync(Reads);
        if (!model.Closed) Render();
    }
    private void Confirm(string text, Action action)
    {
        pendingConfirmation = action; confirmation.Children.Clear();
        confirmation.Children.Add(Warn(text));
        confirmation.Children.Add(Button("确认", () => { var next = pendingConfirmation; ClearConfirmation(); next?.Invoke(); return Task.CompletedTask; }));
        confirmation.Children.Add(Button("取消", () => { ClearConfirmation(); return Task.CompletedTask; }));
        LockConfirmation();
    }
    private void LockConfirmation()
    {
        SetContentEnabled(!Busy && pendingConfirmation == null);
        cancelAuthorization.Visibility = model.OAuthPending ? Visibility.Visible : Visibility.Collapsed;
        Notices.Visibility = pendingConfirmation != null || model.OAuthPending ? Visibility.Visible : Visibility.Collapsed;
    }
    private void ClearConfirmation() { pendingConfirmation = null; confirmation.Children.Clear(); LockConfirmation(); }
    public override void Dispose()
    {
        ClearDeviceToken(); authTerminal?.Dispose();
        if (channelBinding != null) { channelBinding.Changed -= ChannelChanged; channelBinding.Dispose(); }
        model.Changed -= LockConfirmation; base.Dispose();
    }
    public void RequestLeave(Action leave)
    {
        if (Busy) return;
        void Leave() { _ = LeaveChannelsAsync(leave); }
        if (HasChanges) Confirm("放弃尚未保存的修改？", Leave); else Leave();
    }
    public override void HandleEscape()
    {
        if (choices.LastOrDefault(c => c.IsDropDownOpen) is { } choice) { choice.IsDropDownOpen = false; return; }
        if (pendingConfirmation != null) { ClearConfirmation(); return; }
        if (Busy) return;
        if (model.Draft != null) { RequestLeave(() => { model.Discard(); Render(); }); return; }
        if (channelBinding?.Active == true) { _ = CancelChannelBinding(); return; }
        if (section == "channels" && channelDetail.Length > 0) { channelDetail = ""; Render(); return; }
        base.HandleEscape();
    }
    private async Task Run(string command, JsonObject args)
    {
        if (await model.InvokeAsync(command, args)) await Reload();
        else if (!model.Closed) Render();
    }
    private void Action(StackPanel target, string label, string command, JsonObject args, bool destructive = false)
        => target.Children.Add(Button(label, async () =>
        {
            if (destructive) Confirm(label + "？此操作不可撤销。", () => _ = Run(command, args));
            else await Run(command, args);
        }));
    private void Render()
    {
        if (model.Closed) return;
        ClearDeviceToken();
        authTerminal?.Dispose(); authTerminal = null;
        Form.Children.Clear(); Results.Children.Clear(); choices.Clear();
        Form.Children.Add(Button("刷新", Reload));
        switch (section)
        {
            case "skills": Skills(); break;
            case "connections": Connections(); break;
            case "memory": Memory(); break;
            case "general": General(); break;
            case "session": Session(); break;
            case "pet": Pet(); break;
            case "plugins": Plugins(); break;
            case "browser": Browser(); break;
            case "credentials": Credentials(); break;
            case "permissions": Permissions(); break;
            case "environments": Environments(); break;
            case "storage": Storage(); break;
            case "usage": Usage(); break;
            case "quick-actions": QuickActions(); break;
            case "specialists": Specialists(); break;
            case "workflows": Workflows(); break;
            case "models": Models(); break;
            case "channels": Channels(); break;
            case "project": ProjectSettings(); break;
        }
        Update();
        LockConfirmation();
    }
    private StackPanel Card(string title)
    {
        var card = new StackPanel { Spacing = 8, Margin = new Thickness(0, 8, 0, 12) };
        card.Children.Add(Design.Text(title, 18));
        Results.Children.Add(card); return card;
    }
    private void Toggle(StackPanel card, string label, bool value, string command, JsonObject args)
    {
        var control = new ToggleSwitch { Header = label, IsOn = value };
        control.Toggled += async (_, _) => { args["enabled"] = control.IsOn; await Run(command, args); };
        card.Children.Add(control);
    }
    private void Editor(string title, JsonObject draft, string command, string? parameter, Action<JsonObject> fields, JsonObject? extra = null)
    {
        model.Edit(draft, command, parameter, extra);
        if (model.Draft == null) return;
        ClearDeviceToken();
        authTerminal?.Dispose(); authTerminal = null;
        invalidFields.Clear();
        Form.Children.Clear(); Results.Children.Clear(); choices.Clear();
        Form.Children.Add(Design.Text(title, 20));
        fields(model.Draft);
        Form.Children.Add(Button("保存", async () =>
        {
            if (invalidFields.Count > 0) { model.Fail("请检查字段格式：" + string.Join("、", invalidFields.Select(key => key.Contains(':') ? key[(key.IndexOf(':') + 1)..] : key).Distinct())); return; }
            if (await model.SaveAsync()) await Reload();
        }));
        Form.Children.Add(Button("取消编辑", () => { RequestLeave(() => { model.Discard(); Render(); }); return Task.CompletedTask; }));
    }
    private void Text(JsonObject draft, string key, string title, bool multiline = false)
        => Field(title, S(draft, key), value => draft[key] = value, multiline);
    private void PathField(JsonObject draft, string key, string title, bool directory = false, bool either = false)
    {
        var owner = model.Draft;
        var input = Field(title, S(draft, key), value => draft[key] = value);
        if (pickPath == null) return;
        void Picker(bool folder) => Form.Children.Add(Button(folder ? "选择文件夹" : "选择文件", async () =>
        {
            var path = await pickPath(folder);
            if (!model.Closed && model.Draft == owner && path != null) input.Text = path;
        }));
        Picker(directory); if (either) Picker(!directory);
    }
    private void Lines(JsonObject draft, string key, string title)
        => Field(title + "（每行一项）", string.Join("\n", (draft[key] as JsonArray ?? []).Select(n => n?.GetValue<string>() ?? "")),
            value => draft[key] = new JsonArray(value.Split('\n').Select(s => s.Trim()).Where(s => s.Length > 0).Select(s => (JsonNode?)JsonValue.Create(s)).ToArray()), true);
    private void Choice(StackPanel target, string title, string value, (string Id, string Label)[] options, Action<string> changed)
    {
        var combo = new ComboBox { Header = title, HorizontalAlignment = HorizontalAlignment.Stretch };
        foreach (var option in options) combo.Items.Add(new ComboBoxItem { Content = option.Label, Tag = option.Id });
        combo.SelectedItem = combo.Items.Cast<ComboBoxItem>().FirstOrDefault(o => (string)o.Tag == value);
        combo.SelectionChanged += (_, _) => { if (combo.SelectedItem is ComboBoxItem item) changed((string)item.Tag); };
        Design.ApplyTypography(combo);
        choices.Add(combo); target.Children.Add(combo);
    }
    private void Memory()
    {
        var view = model.Values["get_memory_view"];
        if (view == null) return;
        var card = Card(projectScoped ? "项目记忆" : "工作区记忆");
        if (!projectScoped) card.Children.Add(Mute("选择左侧项目作用域后可编辑项目记忆文件。"));
        Toggle(card, "启用记忆", B(view, "enabled"), "set_memory_enabled", new());
        void EditFile(string name, string content) => Editor("记忆文件", new() { ["name"] = name, ["content"] = content }, "write_memory_file", null,
            d => { Text(d, "name", "文件名"); Text(d, "content", "内容", true); });
        card.Children.Add(Button("添加记忆文件", () => { EditFile("", ""); return Task.CompletedTask; }));
        Action(card, "清空记忆", "clear_memory", new(), true);
        foreach (var file in Rows(view["files"]))
        {
            var name = S(file, "name");
            card.Children.Add(Button(name + " · 编辑", async () =>
            {
                string? content = null;
                if (await model.InvokeAsync("read_memory_file", new() { ["name"] = name }, v => content = v?.GetValue<string>()) && content != null)
                    EditFile(name, content);
            }));
            Action(card, "删除 " + name, "delete_memory_file", new() { ["name"] = name }, true);
        }
        if (!projectScoped) foreach (var button in card.Children.OfType<Button>()) button.IsEnabled = false;
        var global = Card("全局记忆");
        global.Children.Add(Button("添加全局记忆", () => { Editor("全局记忆", new() { ["content"] = "" }, "create_global_memory", null, d => Text(d, "content", "内容", true)); return Task.CompletedTask; }));
        foreach (var row in Rows(view["global_memories"]))
        {
            global.Children.Add(Mute(S(row, "content")));
            global.Children.Add(Button("编辑", () => { Editor("全局记忆", new() { ["content"] = S(row, "content") }, "update_global_memory", null,
                d => Text(d, "content", "内容", true), new() { ["id"] = row["id"]?.DeepClone() }); return Task.CompletedTask; }));
            Action(global, "删除全局记忆", "delete_global_memory", new() { ["id"] = row["id"]?.DeepClone() }, true);
        }
        if (model.Values["get_auto_failure_analysis_settings"] is JsonObject settings)
        {
            var failure = Card("自动失败分析");
            failure.Children.Add(Button("编辑失败分析设置", () =>
            {
                Editor("自动失败分析", settings, "set_auto_failure_analysis_settings", "settings", d =>
                {
                    var enabled = new CheckBox { Content = "启用", IsChecked = B(d, "enabled") };
                    enabled.Checked += (_, _) => d["enabled"] = true; enabled.Unchecked += (_, _) => d["enabled"] = false; Form.Children.Add(enabled);
                    foreach (var (key, label, maximum) in new[] { ("failure_rate_threshold", "失败率阈值（%）", 100), ("minimum_failures", "最少失败次数", int.MaxValue) })
                    {
                        var input = new NumberBox { Header = label, Minimum = 0, Maximum = maximum, Value = d[key]?.GetValue<double>() ?? 0, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact };
                        input.ValueChanged += (_, _) => { if (!double.IsNaN(input.Value)) d[key] = (long)input.Value; }; Form.Children.Add(input);
                    }
                }); return Task.CompletedTask;
            }));
        }
    }
    private void Skills()
    {
        Form.Children.Add(Button("导入技能", () => { Editor("导入技能", new() { ["src_path"] = "" }, "install_skill", null, d => PathField(d, "src_path", "目录或 ZIP 路径", either: true)); return Task.CompletedTask; }));
        Action(Form, "重新扫描", "reload_skills", new());
        var search = Field("搜索技能与标签", "", _ => { });
        var cards = new List<(StackPanel Card, string Text)>();
        foreach (var skill in Rows(model.Values["list_skills"]))
        {
            var name = S(skill, "name"); var card = Card(name);
            var tags = string.Join(" · ", (skill["tags"] as JsonArray ?? []).Select(n => n?.GetValue<string>()));
            cards.Add((card, name + " " + S(skill, "description") + " " + tags));
            card.Children.Add(Mute(S(skill, "description"))); card.Children.Add(Mute(tags));
            Toggle(card, "启用", B(skill, "enabled"), "set_skill_enabled", new() { ["name"] = name });
            card.Children.Add(Button("编辑标签", () => { Editor(name, new() { ["tags"] = skill["tags"]?.DeepClone() ?? new JsonArray() }, "set_skill_tags", null,
                d => Lines(d, "tags", "标签"), new() { ["name"] = name }); return Task.CompletedTask; }));
            card.Children.Add(Button("查看文件", async () =>
            {
                JsonNode? files = null;
                if (!await model.InvokeAsync("list_skill_files", new() { ["name"] = name }, v => files = v)) return;
                foreach (var file in files as JsonArray ?? [])
                {
                    var path = file!.GetValue<string>();
                    card.Children.Add(Button(path, async () =>
                    {
                        string content = "";
                        if (await model.InvokeAsync("read_skill_file", new() { ["name"] = name, ["path"] = path }, v => content = S(v, "content")))
                            card.Children.Add(new TextBox { Text = content, IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MaxHeight = 350 });
                    }));
                }
            }));
            if (!B(skill, "builtin") && !B(skill, "managed")) Action(card, "删除技能", "remove_skill", new() { ["name"] = name }, true);
        }
        search.TextChanged += (_, _) => { foreach (var entry in cards) entry.Card.Visibility = entry.Text.Contains(search.Text.Trim(), StringComparison.CurrentCultureIgnoreCase) ? Visibility.Visible : Visibility.Collapsed; };
        SkillStore();
    }
    private void Connections()
    {
        foreach (var row in Rows(model.Values["list_connectors"]?["connectors"]))
        {
            var card = Card(S(row, "name")); card.Children.Add(Mute(S(row, "description_zh") is { Length: > 0 } zh ? zh : S(row, "description")));
            Toggle(card, "启用", B(row, "enabled"), "set_connector_enabled", new() { ["key"] = S(row, "key") });
            Toggle(card, "跳过连接器审批", B(row, "skip_approvals"), "set_connector_skip_approvals", new() { ["key"] = S(row, "key") });
            foreach (var tool in Rows(row["tools"])) Choice(card, S(tool, "name"), S(tool, "mode"), [("allow", "允许"), ("ask", "询问"), ("deny", "禁止")],
                mode => _ = Run("set_tool_approval", new() { ["tool"] = S(tool, "name"), ["mode"] = mode }));
        }
        foreach (var kind in new[] { "stdio", "http" }) Form.Children.Add(Button(kind == "stdio" ? "添加命令连接" : "添加 HTTP 连接", () =>
        {
            EditConnection(new() { ["id"] = Guid.NewGuid().ToString(), ["name"] = "", ["enabled"] = true,
                ["transport"] = kind == "stdio" ? new JsonObject { ["kind"] = kind, ["command"] = "", ["args"] = new JsonArray(), ["env"] = new JsonArray() }
                    : new JsonObject { ["kind"] = kind, ["url"] = "", ["auth"] = "none", ["headers"] = new JsonArray() } }, true);
            return Task.CompletedTask;
        }));
        foreach (var conn in Rows(model.Values["list_mcp_connections"]?["connections"]))
        {
            var card = Card(S(conn, "name"));
            Toggle(card, "启用", B(conn, "enabled"), "set_mcp_connection_enabled", new() { ["id"] = S(conn, "id") });
            card.Children.Add(Button("编辑连接", () => { EditConnection(conn, false); return Task.CompletedTask; }));
            Action(card, "测试连接", S(conn["transport"], "auth") == "oauth" ? "test_oauth_mcp_connection" : "test_mcp_connection", new() { ["conn"] = conn.DeepClone() });
            Action(card, "删除连接", "delete_mcp_connection", new() { ["id"] = S(conn, "id") }, true);
        }
    }
    private void EditConnection(JsonObject conn, bool create) => Editor("MCP 连接", conn, create ? "add_mcp_connection" : "update_mcp_connection", "conn", d =>
    {
        Text(d, "name", "名称");
        var transport = d["transport"]!.AsObject();
        if (S(transport, "kind") == "stdio")
        { PathField(transport, "command", "启动命令"); Lines(transport, "args", "参数"); PathField(transport, "cwd", "工作目录", directory: true); Secrets(transport, "env", "环境变量"); }
        else
        {
            Text(transport, "url", "服务 URL");
            Choice(Form, "认证", S(transport, "auth"), [("none", "无 / 自定义请求头"), ("oauth", "OAuth 浏览器授权")], v => transport["auth"] = v);
            Secrets(transport, "headers", "请求头");
        }
    });
    private void Secrets(JsonObject parent, string key, string label)
    {
        var rows = parent[key] as JsonArray;
        if (rows == null) { rows = new JsonArray(); parent[key] = rows; }
        Form.Children.Add(Mute(label + "：新值留空保留已有凭据；移除项目会删除该凭据。"));
        var list = new StackPanel { Spacing = 8 }; Form.Children.Add(list);
        void Add(JsonObject row)
        {
            var line = new StackPanel { Spacing = 6 };
            var name = new TextBox { Header = "名称", Text = S(row, "name") };
            name.TextChanged += (_, _) => row["name"] = name.Text;
            var value = new PasswordBox { Header = "新值", Password = S(row, "value") };
            value.PasswordChanged += (_, _) => row["value"] = value.Password;
            line.Children.Add(name); line.Children.Add(value);
            line.Children.Add(Button("移除", () => { rows.Remove(row); list.Children.Remove(line); return Task.CompletedTask; })); list.Children.Add(line);
        }
        foreach (var row in Rows(rows)) Add(row);
        Form.Children.Add(Button("添加" + label, () => { var row = new JsonObject { ["name"] = "", ["value"] = "" }; rows.Add(row); Add(row); return Task.CompletedTask; }));
    }
}
