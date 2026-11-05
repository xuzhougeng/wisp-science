using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
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
        DefinitionListLayout();
        Results.Spacing = 0;
        var rows = Rows(model.Values["list_quick_actions"]).ToArray();
        var workflows = Rows(model.Values["list_workflow_templates"]).ToArray();
        var introduction = new StackPanel { Spacing = 5, Margin = new Thickness(0, 4, 0, 8) };
        var heading = Design.Text(T("quick_actions.title"), 17); heading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; introduction.Children.Add(heading);
        var hint = Design.Text(T("quick_actions.help"), 12); hint.Foreground = Design.Brush("text-faint"); hint.MaxWidth = 680;
        hint.HorizontalAlignment = HorizontalAlignment.Left; hint.LineHeight = 18 * Design.Typography.UiSize / 14; introduction.Children.Add(hint);
        var add = Button(T("quick_actions.new"), () => { EditQuickAction(NativeQuickActions.Create(rows, workflows)); return Task.CompletedTask; });
        var addContent = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        var addIcon = Design.Icon("plus", 12); addIcon.VerticalAlignment = VerticalAlignment.Center;
        addContent.Children.Add(addIcon); addContent.Children.Add(Design.Text(T("quick_actions.new"), 12.5)); add.Content = addContent;
        add.Padding = new Thickness(12, 6, 12, 6); add.MinHeight = 30;
        add.IsEnabled = workflows.Length > 0;
        Form.Children.Add(QuickActionRow(introduction, add, 720, viewportBreakpoint: true));
        if (model.Draft is { } draft) QuickActionForm(draft, workflows);
        if (rows.Length == 0) Results.Children.Add(new Border { Height = 1, Background = Design.Brush("border") });
        foreach (var row in rows)
        {
            var title = new NativeActionWrap { Gap = 7 };
            var icon = S(row, "icon");
            if (icon.Length == 0 || icon.Any(c => !char.IsAsciiLetterOrDigit(c) && c != '-') || !File.Exists(Path.Combine(AppContext.BaseDirectory, "Assets", $"icon-{icon}.svg"))) icon = "sparkles";
            title.Children.Add(Design.Icon(icon, 15, Design.ColorText("clay")));
            var displayName = NativeQuickActions.Label(row, Design.Localization);
            var name = DefinitionLeading(Design.Text(displayName, 14), 1.35); name.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; title.Children.Add(name);
            if (B(row, "builtin"))
            {
                var badge = Design.Text(T("quick_actions.builtin"), 9.5); badge.Foreground = Design.Brush("clay"); badge.FontWeight = Microsoft.UI.Text.FontWeights.Bold;
                title.Children.Add(new Border { Child = badge, Background = Design.Brush("clay-soft"), CornerRadius = new CornerRadius(12), Padding = new Thickness(7, 2, 7, 2) });
            }
            var description = DefinitionLeading(Design.Text(S(row, "description"), 12), 1.4);
            description.Foreground = Design.Brush("text-faint"); description.MaxLines = 2; description.TextTrimming = TextTrimming.CharacterEllipsis;
            description.Visibility = description.Text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
            var workflowName = Design.Text(NativeQuickActions.WorkflowName(row, workflows), 10.5); workflowName.Foreground = Design.Brush("text-faint");
            workflowName.FontFamily = Design.Font(true);
            var actions = new NativeActionWrap { Gap = 6 };
            var toggle = new ToggleSwitch { IsOn = B(row, "enabled"), OnContent = "", OffContent = "", MinWidth = 0, MinHeight = 20, VerticalAlignment = VerticalAlignment.Center };
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(toggle, T("quick_actions.enabled") + ": " + displayName);
            PreferenceToggleSwitch.Apply(toggle, Design);
            toggle.Toggled += async (_, _) =>
            {
                var action = (JsonObject)row.DeepClone(); action["enabled"] = toggle.IsOn;
                await Run("save_quick_action", new() { ["action"] = action });
            };
            // A retained inline draft must not become stale through a list-side write.
            toggle.IsEnabled = model.Draft == null; actions.Children.Add(toggle);
            var open = Button(T("quick_actions.open_workflow"), () => { RequestLeave(() => WorkflowRequested?.Invoke(S(row, "workflow_template_id"))); return Task.CompletedTask; });
            Design.BindTypography(open, size: 12); open.Padding = new Thickness(10, 4, 10, 4); open.MinHeight = 24; open.CornerRadius = new CornerRadius(12);
            open.IsEnabled = WorkflowRequested != null && workflows.Any(workflow => S(workflow, "id") == S(row, "workflow_template_id")); actions.Children.Add(open);
            var edit = DefinitionToolButton(T("quick_actions.edit") + ": " + displayName, "edit"); edit.Click += (_, _) => EditQuickAction(row); actions.Children.Add(edit);
            var revealed = new List<Button> { edit };
            if (!B(row, "builtin"))
            {
                var remove = DefinitionToolButton(T("quick_actions.delete") + ": " + displayName, "close", destructive: true);
                remove.Click += (_, _) => RequestLeave(() => { model.Discard(); Render(); Confirm(L("删除快捷动作？此操作不可撤销。", "Delete this Quick Action? This cannot be undone."), () => _ = Run("remove_quick_action", new() { ["actionId"] = S(row, "id") })); });
                actions.Children.Add(remove); revealed.Add(remove);
            }
            var grid = new Grid { ColumnSpacing = 12 };
            grid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); grid.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            for (var i = 0; i < 4; i++) grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
            title.VerticalAlignment = actions.VerticalAlignment = VerticalAlignment.Center;
            grid.Children.Add(title); grid.Children.Add(actions);
            description.Margin = new Thickness(0, 4, 0, 0); workflowName.Margin = new Thickness(0, 4, 0, 0);
            Grid.SetColumnSpan(description, 2); Grid.SetRow(description, 1); grid.Children.Add(description);
            Grid.SetColumnSpan(workflowName, 2); Grid.SetRow(workflowName, 2); grid.Children.Add(workflowName);
            void ArrangeRow()
            {
                var cells = NativeSettingsLayout.QuickActionCells(XamlRoot?.Size.Width ?? ActualWidth);
                Grid.SetColumnSpan(title, cells.TitleSpan); Grid.SetRow(actions, cells.ActionRow);
                Grid.SetColumn(actions, cells.ActionColumn); Grid.SetColumnSpan(actions, cells.ActionSpan);
                actions.HorizontalAlignment = cells.ActionRow == 0 ? HorizontalAlignment.Right : HorizontalAlignment.Left;
                actions.Margin = new Thickness(0, cells.ActionRow == 0 ? 0 : 4, 0, 0);
            }
            grid.SizeChanged += (_, _) => ArrangeRow(); grid.Loaded += (_, _) => ArrangeRow();
            var frame = new Border { Child = grid, Background = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0), Padding = new Thickness(2, 13, 2, 13) };
            RevealDefinitionActions(frame, revealed);
            Results.Children.Add(frame);
        }
    }
    private void DefinitionListLayout()
    {
        Body.Padding = new Thickness(24, 14, 24, 32); Body.Spacing = 12;
        Form.Spacing = Results.Spacing = 12;
    }
    private static TextBlock DefinitionLeading(TextBlock text, double multiple)
    {
        text.LineStackingStrategy = LineStackingStrategy.BlockLineHeight;
        void Apply() => text.LineHeight = text.FontSize * multiple;
        text.RegisterPropertyChangedCallback(TextBlock.FontSizeProperty, (_, _) => Apply()); Apply(); return text;
    }
    private Button DefinitionToolButton(string label, string icon, bool destructive = false)
    {
        var button = Design.ToolButton(label, icon); StyleDefinitionToolButton(button, icon, destructive); return button;
    }
    private void StyleDefinitionToolButton(Button button, string icon, bool destructive = false)
    {
        button.Width = button.Height = button.MinWidth = button.MinHeight = 28; button.Padding = new Thickness(0); button.CornerRadius = new CornerRadius(6);
        button.Content = Design.Icon(icon, 15, Design.ColorText("text-faint"));
        button.Resources["ButtonBackgroundPointerOver"] = destructive
            ? new SolidColorBrush(Mix(Microsoft.UI.Colors.Transparent, Design.Brush("err").Color, .1)) : Design.Brush("bg-sunken");
        button.PointerEntered += (_, _) => button.Content = Design.Icon(icon, 15, Design.ColorText(destructive ? "err" : "text"));
        button.PointerExited += (_, _) => button.Content = Design.Icon(icon, 15, Design.ColorText("text-faint"));
    }
    private static void RevealDefinitionActions(FrameworkElement row, IReadOnlyList<Button> buttons)
    {
        var over = false;
        void Refresh() { foreach (var button in buttons) button.Opacity = over || button.FocusState != FocusState.Unfocused ? 1 : 0; }
        row.PointerEntered += (_, _) => { over = true; Refresh(); };
        row.PointerExited += (_, _) => { over = false; Refresh(); };
        foreach (var button in buttons) { button.GotFocus += (_, _) => Refresh(); button.LostFocus += (_, _) => Refresh(); }
        // Keep tab stops, automation and pointer targets available while hidden,
        // as CSS opacity does; keyboard focus must reveal its action immediately.
        Refresh();
    }
    private static Grid QuickActionRow(FrameworkElement copy, FrameworkElement actions, double breakpoint, bool viewportBreakpoint = false)
    {
        var grid = new Grid { ColumnSpacing = 18, RowSpacing = 12 };
        grid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); grid.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        grid.RowDefinitions.Add(new() { Height = GridLength.Auto }); grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
        grid.Children.Add(copy); Grid.SetColumn(actions, 1); actions.VerticalAlignment = VerticalAlignment.Top; grid.Children.Add(actions);
        grid.SizeChanged += (_, e) =>
        {
            var narrow = viewportBreakpoint ? NativeSettingsLayout.QuickActionsStack(grid.XamlRoot?.Size.Width ?? e.NewSize.Width) : e.NewSize.Width < breakpoint;
            grid.RowSpacing = narrow ? 12 : 0;
            Grid.SetColumnSpan(copy, narrow ? 2 : 1); Grid.SetRow(actions, narrow ? 1 : 0); Grid.SetColumn(actions, narrow ? 0 : 1); Grid.SetColumnSpan(actions, narrow ? 2 : 1);
            actions.HorizontalAlignment = narrow ? HorizontalAlignment.Left : HorizontalAlignment.Right;
        };
        return grid;
    }
    private void EditQuickAction(JsonObject source) => RequestLeave(() =>
    {
        model.Edit(source, "save_quick_action", "action"); invalidFields.Clear(); Render(); BodyScroll.ChangeView(null, 0, null, true);
    });
    private void QuickActionForm(JsonObject draft, JsonObject[] workflows)
    {
        var body = new StackPanel { Spacing = 12 }; var fields = new Grid { ColumnSpacing = 14, RowSpacing = 12 };
        fields.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); fields.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        fields.RowDefinitions.Add(new() { Height = GridLength.Auto }); fields.RowDefinitions.Add(new() { Height = GridLength.Auto });
        StackPanel Field(string key, Control input)
        {
            var label = Design.Text(T(key), 12); label.Foreground = Design.Brush("text-muted");
            var group = new StackPanel { Spacing = 4 };
            group.Children.Add(label); input.MinHeight = 36; Design.BindTypography(input, size: 14);
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetLabeledBy(input, label);
            group.Children.Add(input); return group;
        }
        var name = new TextBox { Text = S(draft, "name"), CornerRadius = new CornerRadius(8), Background = Design.Brush("bg-input") };
        name.TextChanged += (_, _) => { draft["name"] = name.Text; LockConfirmation(); }; textMenus.Register(name);
        var nameField = Field("quick_actions.name", name); fields.Children.Add(nameField);
        var workflow = new ComboBox { IsEnabled = !B(draft, "builtin"), HorizontalAlignment = HorizontalAlignment.Stretch, CornerRadius = new CornerRadius(8), Background = Design.Brush("bg-input") };
        foreach (var row in workflows) workflow.Items.Add(new ComboBoxItem { Tag = S(row, "id"), Content = S(row, "name") });
        if (S(draft, "workflow_template_id") is { Length: > 0 } id && workflows.All(row => S(row, "id") != id)) workflow.Items.Add(new ComboBoxItem { Tag = id, Content = id + L("（当前不可用）", " (unavailable)") });
        workflow.SelectedItem = workflow.Items.Cast<ComboBoxItem>().FirstOrDefault(item => (string)item.Tag == S(draft, "workflow_template_id"));
        workflow.SelectionChanged += (_, _) => { if (workflow.SelectedItem is ComboBoxItem item) { draft["workflow_template_id"] = (string)item.Tag; LockConfirmation(); } };
        choices.Add(workflow); var workflowField = Field("quick_actions.workflow", workflow); Grid.SetColumn(workflowField, 1); fields.Children.Add(workflowField);
        fields.SizeChanged += (_, e) => { var narrow = NativeSettingsLayout.DefinitionFieldsStack(XamlRoot?.Size.Width ?? e.NewSize.Width); fields.RowSpacing = narrow ? 12 : 0; Grid.SetColumnSpan(nameField, narrow ? 2 : 1); Grid.SetColumn(workflowField, narrow ? 0 : 1); Grid.SetRow(workflowField, narrow ? 1 : 0); Grid.SetColumnSpan(workflowField, narrow ? 2 : 1); };
        body.Children.Add(fields);
        var description = new TextBox { Text = S(draft, "description"), IsEnabled = !B(draft, "builtin"), CornerRadius = new CornerRadius(8), Background = Design.Brush("bg-input") };
        description.TextChanged += (_, _) => { draft["description"] = description.Text; LockConfirmation(); }; textMenus.Register(description); body.Children.Add(Field("quick_actions.description", description));
        var enabled = DefinitionCheck(T("quick_actions.enabled"), B(draft, "enabled"), 16);
        enabled.Content = Design.Text(T("quick_actions.enabled"), 12);
        enabled.Checked += (_, _) => { draft["enabled"] = true; LockConfirmation(); }; enabled.Unchecked += (_, _) => { draft["enabled"] = false; LockConfirmation(); }; body.Children.Add(enabled);
        var actions = new NativeActionWrap { RightAligned = true };
        actions.Children.Add(Button(T("settings.cancel"), CancelDefinitionEdit));
        quickActionSave = Button(T("settings.save"), async () => { if (await model.SaveAsync()) await Reload(); }); Design.ActionButton(quickActionSave, primary: true);
        var save = quickActionSave;
        save.Resources["ButtonForegroundDisabled"] = save.Foreground; save.Resources["ButtonBackgroundDisabled"] = save.Background;
        save.Resources["ButtonBorderBrushDisabled"] = save.BorderBrush;
        save.IsEnabledChanged += (_, _) => save.Opacity = save.IsEnabled ? 1 : .55;
        actions.Children.Add(save); body.Children.Add(actions);
        var card = Design.Card(body); card.Padding = new Thickness(14); card.CornerRadius = new CornerRadius(10); card.BorderBrush = Design.Brush("border-strong"); Form.Children.Add(card); Design.ApplyTypography(card);
    }
    private void Specialists()
    {
        if (model.Draft is { } draft) { SpecialistForm(draft); return; }
        DefinitionListLayout();
        var rows = Rows(model.Values["list_specialists"]).ToArray();
        var count = new Border { Child = Design.Text($"{T("settings.nav.specialists")} ({rows.Length})", 13), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1),
            Background = Design.Brush("bg-elev"), CornerRadius = new CornerRadius(8), Padding = new Thickness(10, 6, 10, 6), HorizontalAlignment = HorizontalAlignment.Left };
        Form.Children.Add(QuickActionRow(count, SpecialistAddMenu(), 380));
        SpecialistTransferNotice(Form);
        var subagent = new StackPanel { Spacing = 8 };
        subagent.Children.Add(Design.Text(T("specialists.subagent_model"), 13));
        var current = model.Values["get_subagent_model"]?.GetValue<string>() ?? "";
        var picker = new ComboBox { HorizontalAlignment = HorizontalAlignment.Stretch, MinWidth = 0 };
        foreach (var option in NativeSpecialists.SubagentOptions(current, Rows(model.Values["list_models"]), Design.Localization))
            picker.Items.Add(new ComboBoxItem { Content = option.Label, Tag = option.Id, IsEnabled = option.Available });
        picker.SelectedItem = picker.Items.Cast<ComboBoxItem>().FirstOrDefault(item => (string)item.Tag == current);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(picker, T("specialists.subagent_model"));
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(picker, "subagent-model-select");
        picker.SelectionChanged += async (_, _) =>
        {
            if (picker.SelectedItem is not ComboBoxItem { Tag: string id } || id == current) return;
            await model.SetSubagentModelAsync(id);
            if (!model.Closed) Render();
        };
        choices.Add(picker); subagent.Children.Add(picker);
        subagent.Children.Add(SpecialistHint(T("specialists.subagent_model.hint")));
        Form.Children.Add(subagent);
        foreach (var builtin in new[] { true, false })
        {
            var group = new StackPanel { Spacing = 0 }; var caption = T(builtin ? "specialists.builtin" : "specialists.custom");
            var label = Design.Text(Design.Localization.Locale == "en" ? caption.ToUpperInvariant() : caption, 11);
            label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; label.CharacterSpacing = 60;
            label.Foreground = Design.Brush("text-faint"); label.Margin = new Thickness(2, builtin ? 16 : 20, 0, 18); group.Children.Add(label);
            var list = new StackPanel(); group.Children.Add(new Border { Child = list, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0) });
            var members = rows.Where(row => B(row, "builtin") == builtin).ToArray();
            foreach (var row in members)
            {
                var content = new Grid { ColumnSpacing = 14, MinHeight = 40 };
                content.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); content.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
                var copy = new StackPanel { Spacing = 4, VerticalAlignment = VerticalAlignment.Center };
                var title = DefinitionLeading(Design.Text(S(row, "name"), 14), 1.35); title.FontWeight = Microsoft.UI.Text.FontWeights.Medium; copy.Children.Add(title);
                if (S(row, "description").Length > 0) { var description = DefinitionLeading(Design.Text(S(row, "description"), 12), 1.4); description.Foreground = Design.Brush("text-faint"); description.TextWrapping = TextWrapping.NoWrap; description.TextTrimming = TextTrimming.CharacterEllipsis; copy.Children.Add(description); }
                content.Children.Add(copy); var arrow = Design.Icon("chevron-right", 14, Design.ColorText("text-faint")); Grid.SetColumn(arrow, 1); content.Children.Add(arrow);
                var edit = Button(L("编辑专家：", "Edit specialist: ") + S(row, "name"), () => { EditSpecialist(row); return Task.CompletedTask; });
                edit.Content = content; Design.QuietButton(edit); edit.HorizontalAlignment = edit.HorizontalContentAlignment = HorizontalAlignment.Stretch; edit.Padding = new Thickness(2, 13, 2, 13); edit.CornerRadius = new CornerRadius(0);
                edit.Resources["ButtonBackgroundPointerOver"] = Design.Brush("bg-elev");
                var line = new Grid(); line.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); line.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); line.Children.Add(edit);
                var frame = new Border { Child = line, Background = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, ReferenceEquals(row, members[^1]) ? 0 : 1) };
                if (!builtin)
                {
                    var remove = DefinitionToolButton(T("specialists.remove") + ": " + S(row, "name"), "close", destructive: true);
                    remove.Click += (_, _) => Confirm(L("删除专家？此操作不可撤销。", "Remove this specialist? This cannot be undone."), () => _ = Run("remove_specialist", new() { ["id"] = S(row, "id") }));
                    var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, VerticalAlignment = VerticalAlignment.Center };
                    var export = SpecialistExportButton(row, iconOnly: true); StyleDefinitionToolButton(export, "download");
                    actions.Children.Add(export); actions.Children.Add(remove); RevealDefinitionActions(frame, [export, remove]);
                    Grid.SetColumn(actions, 1); line.Children.Add(actions);
                }
                list.Children.Add(frame);
            }
            Results.Children.Add(group);
        }
        // The edit form and collection share a scroller. Returning from its footer
        // must not leave the collection's count and Add action above the viewport.
        BodyScroll.ChangeView(null, 0, null, true);
    }
    private bool specialistSaved;
    private string? requestedSpecialist;
    public void OpenSpecialist(string id)
    {
        if (section != "specialists" || model.Closed) return;
        requestedSpecialist = id; OpenRequestedSpecialist();
    }
    private void OpenRequestedSpecialist()
    {
        if (requestedSpecialist is not { } id || model.Busy || model.Values["list_specialists"] is not JsonArray) return;
        requestedSpecialist = null;
        if (Rows(model.Values["list_specialists"]).FirstOrDefault(s => S(s, "id") == id) is { } specialist) EditSpecialist(specialist);
    }
    private void EditSpecialist(JsonObject source, bool saved = false) => RequestLeave(() =>
    {
        specialistSaved = saved; model.Edit(source, "save_specialist_cmd", "spec"); invalidFields.Clear(); Render(); BodyScroll.ChangeView(null, 0, null, true);
    });
    private void SpecialistForm(JsonObject draft)
    {
        HeaderActions.Children.Clear();
        Body.Padding = new Thickness(24, 16, 24, 32);
        SettingsBreadcrumb(T("settings.nav.specialists"), S(draft, "name").Length > 0 ? S(draft, "name") : T("specialists.add"), () => RequestLeave(() => { model.Discard(); Render(); }));
        var original = (JsonObject)draft.DeepClone();
        var body = new StackPanel { Spacing = 12 };
        void Field(string label, Control control)
        {
            var group = new StackPanel { Spacing = 5 };
            var caption = DefinitionLeading(Design.Text(label, 12), 1.2); caption.Foreground = Design.Brush("text-muted"); caption.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
            group.Children.Add(caption); group.Children.Add(control); body.Children.Add(group);
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(control, label);
        }
        TextBox Input(string key, string label, double height = 0, bool readOnly = false)
        {
            var multiline = height > 0;
            var input = new TextBox { AcceptsReturn = multiline, Text = S(draft, key), TextWrapping = multiline ? TextWrapping.Wrap : TextWrapping.NoWrap,
                IsReadOnly = readOnly, CornerRadius = new CornerRadius(multiline ? 0 : 8), Background = Design.Brush("bg-input"),
                BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1), MinHeight = multiline ? 0 : 36,
                Padding = multiline ? new Thickness(2, 1, 2, 1) : new Thickness(11, 8, 11, 8) };
            Design.BindTypography(input, size: multiline ? 12 : 14);
            if (multiline) { input.Height = Design.FontSize(height); ScrollViewer.SetVerticalScrollBarVisibility(input, ScrollBarVisibility.Auto); }
            if (readOnly) { input.Background = Design.Brush("bg-sunken"); input.Foreground = Design.Brush("text-muted"); }
            input.TextChanged += (_, _) => { if (!readOnly) draft[key] = input.Text; LockConfirmation(); };
            textMenus.Register(input); Field(label, input); return input;
        }
        Input("name", T("specialists.name")); Input("description", T("specialists.description"), 34); Input("instructions", T("specialists.instructions"), 90, B(draft, "builtin"));
        body.Children.Add(SpecialistHint(T(B(draft, "builtin") ? "specialists.builtin_locked" : "specialists.instructions.hint")));
        var options = NativeSpecialists.BackendOptions(draft, Rows(model.Values["list_models"]), Rows(model.Values["list_acp_agents"]), Design.Localization);
        var combo = new ComboBox { HorizontalAlignment = HorizontalAlignment.Stretch, CornerRadius = new CornerRadius(8), Background = Design.Brush("bg-input"),
            BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1), MinHeight = 36, Padding = new Thickness(11, 8, 0, 8) };
        foreach (var option in options) combo.Items.Add(new ComboBoxItem { Tag = option.Id, Content = option.Label, IsEnabled = option.Available });
        combo.SelectedItem = combo.Items.Cast<ComboBoxItem>().FirstOrDefault(item => (string)item.Tag == NativeSpecialists.BackendKey(draft));
        string BackendLabel(string label) => Design.Localization.Format("specialists.reviewer.selected_backend", ("backend", label));
        var backendLabel = SpecialistHint(BackendLabel(options.First(o => o.Id == NativeSpecialists.BackendKey(draft)).Label));
        combo.SelectionChanged += (_, _) =>
        {
            if (combo.SelectedItem is not ComboBoxItem item) return;
            NativeSpecialists.SelectBackend(draft, (string)item.Tag, original); backendLabel.Text = BackendLabel(item.Content?.ToString() ?? ""); LockConfirmation();
        };
        choices.Add(combo); Field(T("specialists.model"), combo);
        if (S(draft, "id") == "reviewer") { body.Children.Add(backendLabel); body.Children.Add(SpecialistHint(T("specialists.reviewer.test_hint"))); }
        SpecialistSkills(body, draft);
        if (specialistSaved) body.Children.Add(SpecialistHint(T("specialists.saved")));
        var actions = new NativeActionWrap { RightAligned = true };
        if (!B(draft, "builtin") && Rows(model.Values["list_specialists"]).FirstOrDefault(row => S(row, "id") == S(draft, "id") && !B(row, "builtin")) is { } exportSaved)
            actions.Children.Add(SpecialistExportButton(exportSaved, iconOnly: false));
        SpecialistTransferNotice(body);
        if (S(draft, "id") == "reviewer")
        {
            reviewerFeedback = SpecialistHint("");
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetLiveSetting(reviewerFeedback, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite);
            body.Children.Add(reviewerFeedback);
            reviewerTest = Button(T("specialists.reviewer.test"), async () => { await model.TestReviewerAsync(); });
            actions.Children.Add(reviewerTest);
        }
        actions.Children.Add(Button(T("settings.cancel"), CancelDefinitionEdit));
        specialistSave = Button(T("settings.save"), async () =>
        {
            var keepOpen = S(draft, "id") == "reviewer";
            if (!await model.SaveAsync()) return;
            await Reload();
            if (keepOpen && !model.Closed && model.Error == null && Rows(model.Values["list_specialists"]).FirstOrDefault(s => S(s, "id") == "reviewer") is { } saved)
                EditSpecialist(saved, saved: true);
        }); Design.ActionButton(specialistSave, primary: true); actions.Children.Add(specialistSave);
        foreach (var action in actions.Children.OfType<Button>()) { StyleModelButton(action, primary: action == specialistSave); action.MinWidth = 84; }
        var content = new StackPanel { Spacing = 10 }; content.Children.Add(body);
        content.Children.Add(new Border { Child = actions, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0), Padding = new Thickness(0, 14, 0, 0), Margin = new Thickness(0, 4, 0, 0) });
        var card = Design.Card(content); card.Background = Design.Brush("bg-sunken"); card.Padding = new Thickness(16, 14, 16, 14); card.CornerRadius = new CornerRadius(8); card.Margin = new Thickness(0, 4, 0, 0); Form.Children.Add(card); Design.ApplyTypography(card);
    }
    private void SpecialistSkills(StackPanel target, JsonObject draft)
    {
        target.Children.Add(Design.Text(T("specialists.skills"), 14));
        var inherit = DefinitionCheck(T("specialists.inherit"), draft["skills"] == null, 16); inherit.Content = Design.Text(T("specialists.inherit"), 12); target.Children.Add(inherit);
        var group = new StackPanel { Spacing = 12 }; target.Children.Add(group);
        group.Children.Add(SpecialistHint(T("specialists.skills.whitelist_hint")));
        var picker = new StackPanel { Spacing = 8 }; group.Children.Add(picker);
        var chips = new NativeActionWrap { Gap = 5 }; picker.Children.Add(chips);
        var search = new TextBox { PlaceholderText = T("specialists.skills.search"), CornerRadius = new CornerRadius(6), Background = Design.Brush("bg-app"),
            BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1), Padding = new Thickness(10, 8, 10, 8), MinHeight = 0 };
        Design.BindTypography(search, size: 12.5); textMenus.Register(search); picker.Children.Add(search);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(search, T("specialists.skills.search"));
        var results = new StackPanel();
        picker.Children.Add(new Border { Child = new ScrollViewer { Content = results, MaxHeight = 220, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled },
            Background = Design.Brush("bg-sunken"), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(6) });
        var catalog = Rows(model.Values["list_skills"]).ToArray();
        var checks = new Dictionary<string, CheckBox>(StringComparer.Ordinal); var syncing = false;
        void RefreshSelection()
        {
            group.Visibility = draft["skills"] == null ? Visibility.Collapsed : Visibility.Visible;
            chips.Children.Clear();
            var selected = (draft["skills"] as JsonArray ?? []).Select(n => n?.GetValue<string>() ?? "").Distinct(StringComparer.Ordinal).ToArray();
            foreach (var name in selected)
            {
                var label = Design.Localization.Format("specialists.skills.remove", ("skill", name));
                var remove = Button(label, () => { NativeSpecialists.SetSkill(draft, name, false); RefreshSelection(); LockConfirmation(); return Task.CompletedTask; });
                var copy = new Grid { ColumnSpacing = 5 }; copy.ColumnDefinitions.Add(new()); copy.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
                var caption = DefinitionLeading(Design.Text(name, 12), 1.2); caption.TextWrapping = TextWrapping.NoWrap; caption.TextTrimming = TextTrimming.CharacterEllipsis; caption.Foreground = Design.Brush("clay-strong"); copy.Children.Add(caption);
                var icon = Design.Icon("close", 12, Design.ColorText("clay-strong")); Grid.SetColumn(icon, 1); copy.Children.Add(icon); remove.Content = copy;
                remove.HorizontalContentAlignment = HorizontalAlignment.Stretch; remove.MinHeight = 0; remove.Padding = new Thickness(9, 5, 9, 5);
                remove.Background = Design.Brush("clay-soft"); remove.BorderBrush = new SolidColorBrush(Mix(Design.Brush("border").Color, Design.Brush("clay").Color, .28));
                remove.Resources["ButtonBackgroundPointerOver"] = new SolidColorBrush(Mix(Design.Brush("bg-elev").Color, Design.Brush("clay").Color, .14)); remove.Resources["ButtonBorderBrushPointerOver"] = Design.Brush("clay");
                remove.SizeChanged += (_, e) => remove.CornerRadius = new CornerRadius(e.NewSize.Height / 2);
                Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(remove, label); ToolTipService.SetToolTip(remove, label); chips.Children.Add(remove);
            }
            chips.Visibility = selected.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
            // Updating selected chips must not replace the focused result or reset its scroll position.
            syncing = true; foreach (var (name, check) in checks) check.IsChecked = selected.Contains(name, StringComparer.Ordinal); syncing = false;
            Design.ApplyTypography(chips);
        }
        void Draw()
        {
            results.Children.Clear(); checks.Clear();
            var matches = NativeSpecialists.SearchSkills(catalog, search.Text);
            if (matches.Count == 0)
                results.Children.Add(new Border { Padding = new Thickness(12, 10, 12, 10), Child = SpecialistHint(string.IsNullOrWhiteSpace(search.Text)
                    ? Design.Localization.Format("specialists.skills.search_hint", ("count", catalog.Length.ToString())) : T("specialists.skills.no_results")) });
            foreach (var skill in matches)
            {
                var name = S(skill, "name"); var check = DefinitionCheck(name, false, 13); check.Padding = new Thickness(9, 0, 0, 0);
                check.HorizontalAlignment = check.HorizontalContentAlignment = HorizontalAlignment.Stretch;
                var copy = new Grid { ColumnSpacing = 9 }; copy.ColumnDefinitions.Add(new()); copy.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
                var caption = DefinitionLeading(Design.Text(name, 12.5), 1.4); caption.TextWrapping = TextWrapping.NoWrap; caption.TextTrimming = TextTrimming.CharacterEllipsis; caption.Foreground = Design.Brush("text-muted"); copy.Children.Add(caption);
                var scope = DefinitionLeading(Design.Text(S(skill, "scope"), 10), 1.4); scope.Foreground = Design.Brush("text-faint");
                var badge = new Border { Child = scope, Padding = new Thickness(5, 2, 5, 2), Background = Design.Brush("bg-elev"), VerticalAlignment = VerticalAlignment.Center };
                badge.SizeChanged += (_, e) => badge.CornerRadius = new CornerRadius(e.NewSize.Height / 2); Grid.SetColumn(badge, 1); copy.Children.Add(badge); check.Content = copy;
                ToolTipService.SetToolTip(check, S(skill, "description"));
                void Changed(bool selected) { if (syncing) return; NativeSpecialists.SetSkill(draft, name, selected); RefreshSelection(); LockConfirmation(); }
                check.Checked += (_, _) => Changed(true); check.Unchecked += (_, _) => Changed(false); checks[name] = check;
                var frame = new Border { Child = check, Padding = new Thickness(10, 8, 10, 8), Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, ReferenceEquals(skill, matches[^1]) ? 0 : 1) };
                frame.PointerEntered += (_, _) => { frame.Background = Design.Brush("surface-hover"); caption.Foreground = Design.Brush("text"); };
                frame.PointerExited += (_, _) => { frame.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent); caption.Foreground = Design.Brush("text-muted"); };
                results.Children.Add(frame);
            }
            RefreshSelection();
            Design.ApplyTypography(picker);
        }
        inherit.Checked += (_, _) => { draft["skills"] = null; Draw(); LockConfirmation(); };
        inherit.Unchecked += (_, _) => { draft["skills"] = new JsonArray(); Draw(); LockConfirmation(); };
        search.TextChanged += (_, _) => Draw(); Draw();
    }
    private TextBlock SpecialistHint(string text)
    {
        var hint = DefinitionLeading(Design.Text(text, 12), 1.4); hint.Foreground = Design.Brush("text-faint"); return hint;
    }
    private CheckBox DefinitionCheck(string label, bool selected, double size) => PreferenceCheckBox.Create(Design, label, selected, size);
    private Button? reviewerTest;
    private TextBlock? reviewerFeedback;
    private void UpdateReviewerTest()
    {
        if (reviewerTest == null || reviewerFeedback == null) return;
        reviewerTest.IsEnabled = model.CanTestReviewer && pendingConfirmation == null;
        reviewerFeedback.Text = model.ReviewerTesting ? T("specialists.reviewer.testing")
            : model.ReviewerReport is { } report ? NativeSpecialists.ReviewerResult(report, Design.Localization)
            : model.ReviewerTestAttempted && model.Error != null ? Design.Localization.Format("specialists.reviewer.test_failed", ("msg", model.Error))
            : !projectScoped ? L("请先选择项目，以测试 Reviewer 后端。", "Select a project to test the Reviewer backend.") : "";
        reviewerFeedback.Visibility = reviewerFeedback.Text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
    }
    private Task CancelDefinitionEdit()
    {
        // The explicit Cancel button discards only this local form, like WebView.
        // Navigating elsewhere and Escape still use the window's discard stack.
        if (!Busy) { model.Discard(); invalidFields.Clear(); Render(); }
        return Task.CompletedTask;
    }
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
