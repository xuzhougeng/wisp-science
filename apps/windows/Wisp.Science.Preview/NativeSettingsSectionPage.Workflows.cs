using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private void Workflows()
    {
        Form.Children.Add(Button("添加工作流", () => { EditWorkflow(NativeWorkflowDrafts.Workflow()); return Task.CompletedTask; }));
        var rows = Rows(model.Values["list_workflow_templates"]).ToArray();
        if (rows.Length == 0) Results.Children.Add(Mute("尚无工作流，可添加或转换已有技能。"));
        foreach (var row in rows)
        {
            var card = Card(S(row, "name") + (B(row, "builtin") ? " · 内置" : ""));
            card.Children.Add(Mute(S(row, "description")));
            if (B(row, "builtin"))
            {
                var details = new StackPanel { Spacing = 6 }; Summary(details, row["proposal"]);
                card.Children.Add(Disclosure("查看内置定义（复制后可编辑）", details));
            }
            else card.Children.Add(Button("编辑", () => { EditWorkflow(row); return Task.CompletedTask; }));
            card.Children.Add(Button("复制", () => { EditWorkflow(NativeWorkflowDrafts.Copy(row)); return Task.CompletedTask; }));
            if (!B(row, "builtin")) Action(card, "删除工作流", "remove_workflow_template", new() { ["templateId"] = S(row, "id") }, true);
        }
        WorkflowConversion(rows);
    }
    private void EditWorkflow(JsonObject source, JsonNode? sourceHash = null) => Editor("工作流", source, "save_workflow_template", "template", d =>
    {
        Text(d, "name", "名称"); Text(d, "description", "说明", true);
        var proposal = d["proposal"]!.AsObject();
        Text(proposal, "goal", "目标", true); Text(proposal, "context", "上下文", true);
        Choice(Form, "审批策略", S(proposal, "approval_policy"), [("review_all", "逐项审核"), ("auto_safe", "自动执行安全操作")], value => proposal["approval_policy"] = value);
        Form.Children.Add(Mute("依赖节点每行一个 ID。保存时宿主会检查循环依赖、任务定义和资源约束。"));
        var tasks = proposal["tasks"]!.AsArray();
        var list = new StackPanel { Spacing = 12 }; Form.Children.Add(list);
        void Add(JsonObject task)
        {
            var prefix = Guid.NewGuid().ToString("N") + ":";
            var start = Form.Children.Count;
            Text(task, "id", "节点 ID"); Text(task, "instruction", "指令", true);
            Lines(task, "depends_on", "依赖节点"); Lines(task, "capabilities", "工具能力"); Lines(task, "skill_ids", "技能 ID");
            NullableText(task, "specialist_id", "专家 ID（空白继承）"); NullableText(task, "model_id", "模型 ID（空白继承）");
            Choice(Form, "任务类型", S(task, "task_kind") is { Length: > 0 } kind ? kind : "agent", [("agent", "智能体"), ("run_activity", "计算活动")], value => task["task_kind"] = value);
            JsonField(task, "output_schema", "输出 JSON Schema", prefix);
            JsonField(task, "run_activity", "计算活动配置", prefix);
            Boolean(task, "isolated", "独立上下文"); OptionalNumber(task, "timeout_secs", "超时秒数（空白继承，0 无限制）", prefix);
            var executor = task["executor"] as JsonObject;
            var profile = new TextBox { Header = "执行器配置 ID", Text = S(executor, "profile_id"),
                Visibility = S(executor, "kind") is "acp" or "external" ? Visibility.Visible : Visibility.Collapsed };
            Choice(Form, "执行器", S(executor, "kind"), [("", "继承"), ("native", "原生智能体"), ("acp", "ACP"), ("external", "外部执行器")], value =>
            {
                task["executor"] = value.Length == 0 ? null : value == "native" ? new JsonObject { ["kind"] = value }
                    : new JsonObject { ["kind"] = value, ["profile_id"] = profile.Text };
                profile.Visibility = value is "acp" or "external" ? Visibility.Visible : Visibility.Collapsed;
            });
            profile.TextChanged += (_, _) => { if (task["executor"] is JsonObject current && S(current, "kind") is "acp" or "external") current["profile_id"] = profile.Text; };
            Form.Children.Add(profile);
            foreach (var (key, label, maximum) in new[] { ("max_tokens", "Token 上限", (ulong)uint.MaxValue), ("max_tool_calls", "工具调用上限", (ulong)uint.MaxValue), ("max_cost_microunits", "费用上限（微单位）", ulong.MaxValue) })
            {
                var input = new TextBox { Header = label + "（空白继承）", Text = task["budget"]?[key]?.ToString() ?? "" };
                var validation = prefix + label;
                input.TextChanged += (_, _) =>
                {
                    if (!string.IsNullOrWhiteSpace(input.Text) && (!ulong.TryParse(input.Text, out var n) || n > maximum)) { invalidFields.Add(validation); return; }
                    invalidFields.Remove(validation);
                    if (task["budget"] is not JsonObject) task["budget"] = new JsonObject();
                    task["budget"]![key] = string.IsNullOrWhiteSpace(input.Text) ? null : JsonValue.Create(ulong.Parse(input.Text));
                };
                Form.Children.Add(input);
            }
            var fields = new StackPanel { Spacing = 10 };
            while (Form.Children.Count > start) { var field = Form.Children[start]; Form.Children.RemoveAt(start); fields.Children.Add(field); }
            var expander = Disclosure("任务：" + S(task, "id"), fields, true);
            fields.Children.Add(Button("移除此任务", () =>
            { tasks.Remove(task); list.Children.Remove(expander); invalidFields.RemoveWhere(key => key.StartsWith(prefix, StringComparison.Ordinal)); return Task.CompletedTask; }));
            Design.ApplyTypography(expander);
            list.Children.Add(expander);
        }
        foreach (var task in Rows(tasks)) Add(task);
        Form.Children.Add(Button("添加任务", () => { var task = NativeWorkflowDrafts.TaskNode(Rows(tasks)); tasks.Add(task); Add(task); return Task.CompletedTask; }));
    }, sourceHash == null ? null : new() { ["conversionSourceSha256"] = sourceHash.DeepClone() });
    private void NullableText(JsonObject draft, string key, string label)
        => Field(label, S(draft, key), value => draft[key] = string.IsNullOrWhiteSpace(value) ? null : JsonValue.Create(value));
    private void JsonField(JsonObject draft, string key, string label, string prefix)
    {
        var validation = prefix + label;
        Field(label + "（JSON，空白继承）", draft[key]?.ToJsonString(new() { WriteIndented = true }) ?? "", value =>
        {
            try { var parsed = string.IsNullOrWhiteSpace(value) ? null : JsonNode.Parse(value); draft[key] = parsed; invalidFields.Remove(validation); }
            catch (System.Text.Json.JsonException) { invalidFields.Add(validation); }
        }, true);
    }
    private void OptionalNumber(JsonObject draft, string key, string label, string prefix, bool nullable = true)
    {
        var validation = prefix + label;
        Field(label, draft[key]?.ToString() ?? "", value =>
        {
            if (nullable && string.IsNullOrWhiteSpace(value)) { draft[key] = null; invalidFields.Remove(validation); }
            else if (ulong.TryParse(value, out var number)) { draft[key] = number; invalidFields.Remove(validation); }
            else invalidFields.Add(validation);
        });
    }
    private void WorkflowConversion(JsonObject[] templates)
    {
        var card = Card("从技能或旧模板转换");
        if (!projectScoped) { card.Children.Add(Mute("先选择项目作用域，再生成可审核草稿。")); return; }
        var request = new TextBox { Header = "研究目标", AcceptsReturn = true, TextWrapping = TextWrapping.Wrap };
        card.Children.Add(request);
        string selectedModel = "", legacy = "";
        var skills = new HashSet<string>(StringComparer.Ordinal);
        var skillList = new StackPanel { Spacing = 6 };
        Choice(card, "转换模型", "", Rows(model.Values["list_models"]).Select(row => (S(row, "id"), S(row, "label") is { Length: > 0 } label ? label : S(row, "model"))).ToArray(), value => selectedModel = value);
        Choice(card, "旧模板（可选）", "", new[] { ("", "从技能选择") }.Concat(templates.Select(row => (S(row, "id"), S(row, "name")))).ToArray(), value =>
        { legacy = value; skillList.Visibility = value.Length == 0 ? Visibility.Visible : Visibility.Collapsed; });
        foreach (var skill in Rows(model.Values["list_skills"]))
        {
            var name = S(skill, "name"); var check = new CheckBox { Content = name };
            check.Checked += (_, _) => skills.Add(name); check.Unchecked += (_, _) => skills.Remove(name); skillList.Children.Add(check);
        }
        card.Children.Add(skillList);
        var resultPanel = new StackPanel { Spacing = 8 };
        card.Children.Add(Button("生成可审核草稿", async () =>
        {
            if (string.IsNullOrWhiteSpace(request.Text) || selectedModel.Length == 0) { model.Fail("请填写研究目标并选择转换模型。"); return; }
            var description = request.Text;
            var payload = new JsonObject { ["request"] = description, ["model_id"] = selectedModel };
            if (legacy.Length > 0) payload["legacy_template_id"] = legacy;
            else payload["source_skill_ids"] = new JsonArray(skills.Order(StringComparer.Ordinal).Select(id => (JsonNode?)JsonValue.Create(id)).ToArray());
            JsonNode? result = null;
            if (!await model.InvokeAsync("plan_skill_portfolio", new() { ["request"] = payload, ["conversionId"] = Guid.NewGuid().ToString(), ["expectedProjectId"] = settingsProjectId }, value => result = value)) return;
            resultPanel.Children.Clear();
            if (result?["proposal"] is not JsonObject) { model.Fail("转换未返回有效工作流草稿。"); return; }
            resultPanel.Children.Add(Mute(S(result["plan"], "rationale")));
            resultPanel.Children.Add(Button("审核草稿", () =>
            {
                var draft = NativeWorkflowDrafts.Workflow(); draft["name"] = "新工作流"; draft["description"] = description; draft["proposal"] = result["proposal"]!.DeepClone();
                EditWorkflow(draft, result["plan"]?["source_sha256"]); return Task.CompletedTask;
            }));
        }));
        card.Children.Add(resultPanel);
    }
    private void QuickActions()
    {
        var rows = Rows(model.Values["list_quick_actions"]).ToArray();
        Form.Children.Add(Button("添加快捷操作", () => { EditQuickAction(NativeWorkflowDrafts.QuickAction(rows.Length)); return Task.CompletedTask; }));
        if (rows.Length == 0) Results.Children.Add(Mute("尚无快捷操作，可添加配置。"));
        foreach (var row in rows)
        {
            var card = Card(S(row, "name") + (B(row, "builtin") ? " · 内置" : ""));
            card.Children.Add(Mute(S(row, "description")));
            var toggle = new ToggleSwitch { Header = "启用", IsOn = B(row, "enabled") };
            toggle.Toggled += async (_, _) =>
            {
                var action = (JsonObject)row.DeepClone(); action["enabled"] = toggle.IsOn;
                await Run("save_quick_action", new() { ["action"] = action });
            };
            card.Children.Add(toggle);
            card.Children.Add(Button("编辑", () => { EditQuickAction(row); return Task.CompletedTask; }));
            card.Children.Add(Button("复制", () => { EditQuickAction(NativeWorkflowDrafts.Copy(row)); return Task.CompletedTask; }));
            if (!B(row, "builtin")) Action(card, "删除快捷操作", "remove_quick_action", new() { ["actionId"] = S(row, "id") }, true);
        }
    }
    private void EditQuickAction(JsonObject source) => Editor("快捷操作", source, "save_quick_action", "action", d =>
    {
        Text(d, "name", "名称"); Text(d, "description", "说明", true);
        ReferenceChoice(d, "workflow_template_id", "工作流模板", Rows(model.Values["list_workflow_templates"]), "name", false);
        Boolean(d, "enabled", "启用"); Integer(d, "sort_order", "排序", long.MinValue, long.MaxValue);
    });
    private void Specialists()
    {
        Form.Children.Add(Button("添加专家", () => { EditSpecialist(NativeWorkflowDrafts.Specialist()); return Task.CompletedTask; }));
        var rows = Rows(model.Values["list_specialists"]).ToArray();
        if (rows.Length == 0) Results.Children.Add(Mute("尚无专家，可添加配置。"));
        foreach (var row in rows)
        {
            var card = Card(S(row, "name") + (B(row, "builtin") ? " · 内置" : ""));
            card.Children.Add(Mute(S(row, "description")));
            card.Children.Add(Button("编辑", () => { EditSpecialist(row); return Task.CompletedTask; }));
            card.Children.Add(Button("复制", () => { EditSpecialist(NativeWorkflowDrafts.Copy(row)); return Task.CompletedTask; }));
            if (!B(row, "builtin")) Action(card, "删除专家", "remove_specialist", new() { ["id"] = S(row, "id") }, true);
        }
    }
    private void EditSpecialist(JsonObject source) => Editor("专家", source, "save_specialist_cmd", "spec", d =>
    {
        Text(d, "name", "名称"); Text(d, "description", "说明", true); Text(d, "instructions", "系统提示词", true);
        ReferenceChoice(d, "model_id", "模型", Rows(model.Values["list_models"]), "label", true);
        Whitelist(d, "skills", "技能白名单"); Whitelist(d, "connectors", "连接器白名单");
        if (S(d, "id") != "reviewer") return;
        Form.Children.Add(Mute("审核后端：保留现有配置可继续使用旧版模型绑定；选择其他后端后按配置执行。"));
        var backend = d["review_backend"] as JsonObject;
        var profile = new TextBox { Header = "模型 / ACP 配置 ID", Text = S(backend, "profile_id"),
            Visibility = backend != null && S(backend, "kind") != "follow_session" ? Visibility.Visible : Visibility.Collapsed };
        var originalBackend = backend?.DeepClone();
        Choice(Form, "审核后端", S(backend, "kind"), [("", "保留现有配置"), ("follow_session", "跟随会话"), ("http_model", "HTTP 模型"), ("acp_agent", "ACP 智能体")], kind =>
        {
            d["review_backend"] = kind.Length == 0 ? originalBackend?.DeepClone() : kind == "follow_session"
                ? new JsonObject { ["kind"] = kind } : new JsonObject { ["kind"] = kind, ["profile_id"] = profile.Text };
            profile.Visibility = kind is "http_model" or "acp_agent" ? Visibility.Visible : Visibility.Collapsed;
        });
        profile.TextChanged += (_, _) => { if (d["review_backend"] is JsonObject current && S(current, "kind") is "http_model" or "acp_agent") current["profile_id"] = profile.Text; };
        Form.Children.Add(profile);
    });
    private void ReferenceChoice(JsonObject draft, string key, string title, IEnumerable<JsonObject> rows, string labelKey, bool allowDefault)
    {
        var options = rows.Select(row => (S(row, "id"), S(row, labelKey) is { Length: > 0 } label ? label : S(row, "model"))).ToList();
        if (allowDefault) options.Insert(0, ("", "跟随默认模型"));
        var current = S(draft, key);
        if (current.Length > 0 && !options.Any(o => o.Item1 == current)) options.Add((current, "当前引用（不可用）：" + current));
        Choice(Form, title, current, options.ToArray(), value => draft[key] = value);
    }
    private void Whitelist(JsonObject draft, string key, string title)
    {
        var inherited = draft[key] == null;
        var input = new TextBox { Header = title + "（每行一项；空白表示不启用任何项目）", AcceptsReturn = true,
            TextWrapping = TextWrapping.Wrap, Text = string.Join("\n", (draft[key] as JsonArray ?? []).Select(n => n?.GetValue<string>())), IsEnabled = !inherited };
        var toggle = new ToggleSwitch { Header = title + "继承项目设置", IsOn = inherited };
        toggle.Toggled += (_, _) => { input.IsEnabled = !toggle.IsOn; NativeWorkflowDrafts.SetWhitelist(draft, key, toggle.IsOn, input.Text); };
        input.TextChanged += (_, _) => NativeWorkflowDrafts.SetWhitelist(draft, key, toggle.IsOn, input.Text);
        Form.Children.Add(toggle); Form.Children.Add(input);
    }
}
