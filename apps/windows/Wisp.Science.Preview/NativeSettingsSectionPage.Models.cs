using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private bool showAgents;
    private NativeSettingsAuthTerminal? authTerminal;
    private string? authTerminalId;
    private JsonNode? testedAgentInfo;
    private string? testedAgentId;
    private void Models()
    {
        Form.Children.Add(Mute("模型配置由所有项目共享。默认模型用于新建对话，列表操作立即保存。"));
        Form.Children.Add(Button("API 模型", () => { showAgents = false; Render(); return Task.CompletedTask; }));
        Form.Children.Add(Button("ACP Agents", () => { showAgents = true; Render(); return Task.CompletedTask; }));
        if (showAgents) { Agents(); return; }
        Form.Children.Add(Button("添加 API 接入", () => { EditModel(NativeModelDrafts.Create()); return Task.CompletedTask; }));
        var presets = Rows(JsonNode.Parse(File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "Assets", "model-presets.json"))));
        var presetButtons = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
        foreach (var preset in presets) presetButtons.Children.Add(Button(S(preset, "label"), () =>
        { EditModel(NativeModelDrafts.Create(S(preset, "url"), S(preset, "model"))); return Task.CompletedTask; }));
        Form.Children.Add(new ScrollViewer { Content = presetButtons, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, VerticalScrollBarVisibility = ScrollBarVisibility.Disabled });
        var search = Field("搜索名称、服务商或模型", "", _ => { });
        var rows = Rows(model.Values["list_models"]).ToArray();
        var cards = new List<(StackPanel Card, string Text)>();
        var reorder = new List<(Button Button, bool Available)>();
        foreach (var (row, index) in rows.Select((row, index) => (row, index)))
        {
            var card = Card(S(row, "label") is { Length: > 0 } label ? label : S(row, "model"));
            cards.Add((card, S(row, "label") + " " + S(row, "provider") + " " + S(row, "model")));
            card.Children.Add(Mute(S(row, "provider") + " · " + S(row, "model") + (B(row, "supports_vision") ? " · Vision" : "")));
            card.Children.Add(Mute(B(row, "has_api_key") ? "已配置密钥" : "未配置密钥"));
            if (B(row, "active")) card.Children.Add(Mute("默认模型"));
            else Action(card, "设为默认模型", "set_active_model", new() { ["id"] = S(row, "id") });
            card.Children.Add(Button("编辑", () => { EditModel(row); return Task.CompletedTask; }));
            foreach (var offset in new[] { -1, 1 })
            {
                var available = index + offset >= 0 && index + offset < rows.Length;
                var button = Button(offset < 0 ? "上移" : "下移", () => Run("reorder_models", new() { ["ids"] = NativeModelDrafts.Reorder(rows, S(row, "id"), offset) }));
                button.IsEnabled = available; reorder.Add((button, available)); card.Children.Add(button);
            }
            Action(card, "删除模型", "remove_model", new() { ["id"] = S(row, "id") }, true);
        }
        var empty = Mute("没有匹配的模型配置。"); Results.Children.Add(empty); empty.Visibility = rows.Length == 0 ? Visibility.Visible : Visibility.Collapsed;
        search.TextChanged += (_, _) =>
        {
            var any = false;
            foreach (var entry in cards) { var visible = entry.Text.Contains(search.Text.Trim(), StringComparison.CurrentCultureIgnoreCase); entry.Card.Visibility = visible ? Visibility.Visible : Visibility.Collapsed; any |= visible; }
            empty.Visibility = any ? Visibility.Collapsed : Visibility.Visible;
            foreach (var entry in reorder) entry.Button.IsEnabled = entry.Available && string.IsNullOrWhiteSpace(search.Text);
        };
    }
    private void EditModel(JsonObject source) => Editor("API 模型", source, "save_model", "profile", d =>
    {
        Text(d, "api_url", "API 根地址"); Secure(d, "key", "API 密钥（留空保留现有密钥）");
        Text(d, "label", "显示名称");
        Choice(Form, "协议", S(d, "provider"), [("openai", "OpenAI Chat Completions"), ("openai_responses", "OpenAI Responses"), ("anthropic", "Anthropic")], value => d["provider"] = value);
        Text(d, "model", "模型 ID"); Text(d, "endpoint_suffix", "接口后缀");
        Boolean(d, "supports_vision", "支持图片输入"); Boolean(d, "use_for_vision", "用于图片分析");
        Boolean(d, "use_for_image_generation", "用于图片生成"); Boolean(d, "use_for_video_generation", "用于视频生成");
        Form.Children.Add(Mute("0 输出 Token 使用默认值；保存时由模型目录按精确 ID 校验上限。"));
        OptionalNumber(d, "max_tokens", "最大输出 Token", "模型:", nullable: false); OptionalNumber(d, "context_window", "上下文容量", "模型:", nullable: false);
        Choice(Form, "推理强度", S(d, "reasoning_effort"), [("", "默认"), ("none", "None"), ("minimal", "Minimal"), ("low", "Low"), ("medium", "Medium"), ("high", "High"), ("xhigh", "XHigh"), ("max", "Max"), ("ultra", "Ultra")], value => d["reasoning_effort"] = value);
        Choice(Form, "服务级别", S(d, "service_tier"), [("", "默认"), ("priority", "Priority"), ("flex", "Flex")], value => d["service_tier"] = value);
        Text(d, "image_size", "图片尺寸"); Text(d, "image_quality", "图片质量"); Text(d, "image_aspect_ratio", "图片比例"); Text(d, "image_resolution", "图片分辨率");
        Integer(d, "video_duration_secs", "视频时长（秒，空白使用默认值）", 1, 15, nullable: true);
        NullableText(d, "video_aspect_ratio", "视频比例"); NullableText(d, "video_resolution", "视频分辨率");
        Boolean(d, "send_user_agent", "发送 User-Agent"); Text(d, "user_agent", "User-Agent");
        Choice(Form, "发送会话标识", d["send_session_id"] == null ? "" : B(d, "send_session_id") ? "yes" : "no",
            [("", "使用默认行为"), ("yes", "发送"), ("no", "不发送")], value => d["send_session_id"] = value.Length == 0 ? null : JsonValue.Create(value == "yes"));
        Text(d, "session_header_name", "会话请求头名称");
        var result = new StackPanel { Spacing = 6 };
        Form.Children.Add(Button("查询模型目录", async () =>
        {
            JsonNode? value = null;
            if (await model.InvokeAsync("model_catalog_lookup", new() { ["provider"] = d["provider"]?.DeepClone(), ["apiUrl"] = d["api_url"]?.DeepClone(), ["model"] = d["model"]?.DeepClone() }, response => value = response))
            { result.Children.Clear(); if (value == null) result.Children.Add(Mute("目录没有此精确模型 ID，请按服务商说明填写容量。")); else Summary(result, value); }
        }));
        Form.Children.Add(Button("测试 API", async () =>
        {
            if (invalidFields.Count > 0) { model.Fail("请先修正无效字段。"); return; }
            JsonNode? value = null;
            if (await model.TestModelAsync(response => value = response)) { result.Children.Clear(); Summary(result, value); }
        }));
        Form.Children.Add(result);
    });
    private void Agents()
    {
        Form.Children.Add(Button("添加智能体", () => { EditAgent(new() { ["id"] = "", ["label"] = "", ["command"] = "", ["args"] = new JsonArray() }); return Task.CompletedTask; }));
        foreach (var agent in Rows(model.Values["list_acp_agents"]))
        {
            var id = S(agent, "id"); var card = Card(S(agent, "label")); card.Children.Add(Mute(S(agent, "command")));
            card.Children.Add(Button("编辑", () => { EditAgent(agent); return Task.CompletedTask; }));
            card.Children.Add(Button("测试连接", async () =>
            {
                testedAgentInfo = null; testedAgentId = null;
                if (await model.InvokeAsync("test_acp_agent", new() { ["id"] = id }, info => { testedAgentInfo = info?.DeepClone(); testedAgentId = id; })) Render();
            }));
            Action(card, "删除智能体", "remove_acp_agent", new() { ["id"] = id }, true);
        }
        if (testedAgentInfo != null && Rows(model.Values["list_acp_agents"]).Any(a => S(a, "id") == testedAgentId))
        {
            var card = Card("连接测试结果"); Summary(card, testedAgentInfo["implementation"]); Summary(card, testedAgentInfo["capabilities"]);
            foreach (var method in Rows(testedAgentInfo["authMethods"]))
            {
                var kind = S(method, "type");
                var authorize = Button("授权：" + S(method, "name"), async () =>
                {
                    JsonNode? result = null;
                    if (!await model.InvokeAsync("authenticate_acp_agent", new() { ["id"] = testedAgentId, ["methodId"] = S(method, "id") }, value => result = value)) return;
                    if (result == null) { card.Children.Add(Mute("授权流程已完成。")); return; }
                    var terminalId = S(result, "id");
                    if (terminalId.Length == 0) { model.Fail("授权终端未返回有效 ID。"); return; }
                    authTerminalId = terminalId; Render();
                });
                authorize.IsEnabled = kind != "env_var" && authTerminalId == null && (kind != "terminal" || projectScoped);
                card.Children.Add(authorize);
                if (kind == "env_var") card.Children.Add(Mute("此环境变量认证方式暂不支持。"));
                else if (kind == "terminal" && !projectScoped) card.Children.Add(Mute("终端授权需要先选择项目作用域。"));
            }
        }
        if (authTerminalId != null && settingsClient != null && settingsProjectId != null)
        {
            authTerminal = new NativeSettingsAuthTerminal(new NativeAuthTerminalModel(settingsClient, settingsProjectId, authTerminalId), Design, () =>
            { authTerminalId = null; authTerminal?.Dispose(); authTerminal = null; Render(); });
            Results.Children.Add(authTerminal);
        }
    }
    private void EditAgent(JsonObject agent) => Editor("ACP 智能体", agent, "save_acp_agent", "profile", d =>
    {
        Text(d, "label", "名称"); PathField(d, "command", "可执行文件"); Lines(d, "args", "启动参数（保留每行内的空格）");
    });
}
