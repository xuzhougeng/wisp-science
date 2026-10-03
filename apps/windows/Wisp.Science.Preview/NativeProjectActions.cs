using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

/// <summary>Shared lifecycle for project action forms; inputs remain mounted across failed operations.</summary>
internal abstract class NativeActionPage : WorkspaceSheet
{
    protected readonly WorkspaceActionModel State;
    protected readonly StackPanel Form = new() { Spacing = 12, MaxWidth = 900, HorizontalAlignment = HorizontalAlignment.Stretch };
    protected readonly StackPanel Results = new() { Spacing = 12 };
    private readonly TextBlock error = new() { TextWrapping = TextWrapping.Wrap };
    private readonly ProgressBar progress = new() { IsIndeterminate = true, Height = 3 };
    private readonly ContentControl formHost = new();
    private readonly ContentControl resultsHost = new();
    private readonly bool ownsState;
    protected NativeActionPage(WispDesign design, string title, WorkspaceActionModel state, Action close, bool ownsState = true) : base(design, title, close)
    {
        State = state; this.ownsState = ownsState;
        formHost.Content = Form; resultsHost.Content = Results;
        formHost.HorizontalContentAlignment = HorizontalAlignment.Stretch; resultsHost.HorizontalContentAlignment = HorizontalAlignment.Stretch;
        Body.Children.Add(progress); Body.Children.Add(error); Body.Children.Add(formHost); Body.Children.Add(resultsHost);
        state.Changed += Update; Update();
    }
    protected void Update()
    {
        if (State.Closed) return;
        Design.ApplyTypography(Form); Design.ApplyTypography(Results);
        formHost.IsEnabled = !State.Busy; resultsHost.IsEnabled = !State.Busy;
        progress.Visibility = State.Busy ? Visibility.Visible : Visibility.Collapsed;
        foreach (var action in HeaderActions.Children.OfType<Control>()) action.IsEnabled = !State.Busy;
        error.Text = State.Error ?? ""; error.Visibility = State.Error == null ? Visibility.Collapsed : Visibility.Visible;
    }
    protected void SetContentEnabled(bool enabled)
    {
        formHost.IsEnabled = resultsHost.IsEnabled = enabled;
        foreach (var action in HeaderActions.Children.OfType<Control>()) action.IsEnabled = enabled;
    }
    protected TextBox Field(string title, string text, Action<string> changed, bool multiline = false)
    {
        var input = new TextBox { Header = title, Text = text, AcceptsReturn = multiline, TextWrapping = TextWrapping.Wrap, MinHeight = multiline ? 85 : 32 };
        Design.BindTypography(input);
        input.TextChanged += (_, _) => changed(input.Text); Form.Children.Add(input); return input;
    }
    protected Button Button(string title, Func<Task> action)
    {
        var button = new Button { Content = title };
        Design.ActionButton(button, title is "保存" or "创建项目" or "导入" or "创建论文证据");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(button, title);
        button.Click += async (_, _) => await action(); return button;
    }
    protected Expander Disclosure(string title, UIElement content, bool expanded = false)
    {
        var expander = new Expander { Header = Design.Text(title), Content = content,
            IsExpanded = expanded, HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(expander, title);
        Design.ApplyTypography(expander);
        return expander;
    }
    public override void HandleEscape()
    {
        foreach (var combo in Descendants(Form).OfType<ComboBox>())
            if (combo.IsDropDownOpen) { combo.IsDropDownOpen = false; return; }
        foreach (var picker in Descendants(Form).OfType<CalendarDatePicker>())
            if (picker.IsCalendarOpen) { picker.IsCalendarOpen = false; return; }
        if (!State.Busy) base.HandleEscape();
    }
    private static IEnumerable<UIElement> Descendants(Panel panel)
    {
        foreach (var child in panel.Children)
        { yield return child; if (child is Panel nested) foreach (var descendant in Descendants(nested)) yield return descendant; }
    }
    public override void Dispose() { State.Changed -= Update; if (ownsState) State.Dispose(); base.Dispose(); }
}

internal sealed class NativeNewProjectPage : NativeActionPage
{
    public NativeNewProjectPage(WorkspaceProjectCreation model, WispDesign design, Func<Task<string?>> pickDirectory,
        Func<ProjectSummary, Task> opened, Action close) : base(design, "新建项目", model, close)
    {
        Field("项目名称", model.Name, s => model.Name = s);
        var directory = Field("工作目录", model.Directory, s => model.Directory = s);
        Form.Children.Add(Button("选择文件夹", async () => { if (await pickDirectory() is { } path) directory.Text = path; }));
        Field("项目描述", model.Description, s => model.Description = s, true);
        var context = Field("项目指令", model.AgentContext, s => model.AgentContext = s, true);
        Form.Children.Remove(context);
        Form.Children.Add(Disclosure("项目指令（可选）", context));
        var standard = new CheckBox { Content = "使用标准科研目录结构", IsChecked = model.StandardLayout };
        standard.Checked += (_, _) => { model.SetStandardLayout(true); context.Text = model.AgentContext; };
        standard.Unchecked += (_, _) => { model.SetStandardLayout(false); context.Text = model.AgentContext; };
        Form.Children.Add(standard);
        Form.Children.Add(Button("创建项目", async () => { if (await model.CreateAsync() && model.Created is { } row) await opened(row); }));
    }
}

internal sealed class NativeImportProjectPage : NativeActionPage
{
    public NativeImportProjectPage(WorkspaceProjectCreation model, WispDesign design, Func<Task<string?>> pickArchive,
        Func<ProjectSummary, Task> opened, Action close) : base(design, "导入项目", model, close)
    {
        var path = "";
        Form.Children.Add(Mute("选择从 Wisp 导出的项目 ZIP 归档，导入后打开项目。"));
        var input = Field("项目归档（ZIP）", "", s => path = s);
        Form.Children.Add(Button("选择归档", async () => { if (await pickArchive() is { } selected) input.Text = selected; }));
        Form.Children.Add(Button("导入", async () =>
        {
            if (string.IsNullOrWhiteSpace(path)) { model.Fail("请选择项目归档。"); return; }
            if (await model.ImportAsync(path) && model.Created is { } row) await opened(row);
        }));
        NativeActionWrap.GroupButtons(Form);
    }
}

internal sealed class NativeSessionGroupPage : NativeActionPage
{
    public NativeSessionGroupPage(WorkspaceSessionGroups groups, WispDesign design, Action close)
        : base(design, groups.RenamingId == null ? "新建分组" : "重命名分组", groups, close, ownsState: false)
    {
        Field("分组名称", groups.Draft, s => groups.Draft = s);
        Form.Children.Add(Button("保存", async () => { if (await groups.SaveAsync()) close(); }));
    }
}

internal sealed class NativeLibraryPage : NativeActionPage
{
    private readonly WorkspaceLibraryModel model;
    private readonly Action<LibraryItemSummary>? insert;
    private readonly Func<LibraryItemSummary, Task> source;
    public NativeLibraryPage(WorkspaceLibraryModel model, WispDesign design, Action<LibraryItemSummary>? insert,
        Func<LibraryItemSummary, Task> source, Action close) : base(design, "收藏库", model, close)
    {
        this.model = model; this.insert = insert; this.source = source;
        Field("搜索标题、代码或项目", model.Query, s => model.Query = s);
        var filters = new ComboBox { PlaceholderText = "类型", MinWidth = 160 };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(filters, "收藏类型");
        foreach (var (label, kind) in new[] { ("全部", ""), ("代码", "code"), ("图片", "figure"), ("文本", "text") })
            filters.Items.Add(new ComboBoxItem { Content = label, Tag = kind });
        filters.SelectedIndex = 0;
        filters.SelectionChanged += async (_, _) => { model.Kind = (string)((ComboBoxItem)filters.SelectedItem).Tag; await Search(); };
        var filterActions = new NativeActionWrap(); filterActions.Children.Add(filters); filterActions.Children.Add(Button("搜索", Search));
        Form.Children.Add(filterActions);
        _ = Search();
    }
    private async Task Search() { await model.SearchAsync(); RenderItems(); }
    private void RenderItems()
    {
        if (model.Closed) return;
        Results.Children.Clear();
        if (model.Items.Count == 0 && model.Error == null) Results.Children.Add(Design.EmptyState("star", "暂无匹配的收藏", "可调整搜索条件；在对话中收藏的代码、图片和文本会显示在这里。"));
        foreach (var item in model.Items)
        {
            var card = new StackPanel { Spacing = 8 };
            card.Children.Add(new TextBlock { Text = item.Title, FontSize = 18, TextWrapping = TextWrapping.Wrap });
            card.Children.Add(Mute(item.SourceProjectName + " / " + item.SourceSessionTitle));
            if (!string.IsNullOrWhiteSpace(item.CodePreview))
                card.Children.Add(new TextBox { Text = item.CodePreview, IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MaxHeight = 180 });
            else if (!string.IsNullOrWhiteSpace(item.SourcePath)) card.Children.Add(Mute(item.SourcePath));
            var actions = new NativeActionWrap();
            if (insert != null) actions.Children.Add(Button("填入对话框", () => { insert(item); return Task.CompletedTask; }));
            actions.Children.Add(Button("打开来源", () => source(item)));
            actions.Children.Add(Button("删除", async () => { await model.DeleteAsync(item.Id); RenderItems(); }));
            card.Children.Add(actions); Results.Children.Add(Design.Card(card));
        }
    }
}

internal sealed class NativePublicationPage : NativeActionPage
{
    private readonly WorkspacePublicationModel model;
    private readonly TextBox title, description, revision;
    private readonly Button create;
    public NativePublicationPage(WorkspacePublicationModel model, WispDesign design, Action close) : base(design, "论文证据", model, close)
    {
        this.model = model;
        title = Field("论文标题", model.Title, s => { model.Title = s; UpdateCreate(); });
        description = Field("描述", model.Description, s => model.Description = s, true);
        revision = Field("版本标签", model.RevisionLabel, s => { model.RevisionLabel = s; UpdateCreate(); });
        create = Button("创建论文证据", async () =>
        {
            if (await model.CreateAsync()) { title.Text = model.Title; description.Text = model.Description; revision.Text = model.RevisionLabel; }
            RenderItems();
        });
        Form.Children.Add(create);
        model.Changed += UpdateCreate;
        UpdateCreate();
        var refresh = Design.ToolButton("刷新", "refresh"); refresh.Click += async (_, _) => await Load();
        HeaderActions.Children.Add(refresh); _ = Load();
    }
    private void UpdateCreate()
    {
        if (create == null || model.Closed) return;
        var visibility = model.CreationAvailable ? Visibility.Visible : Visibility.Collapsed;
        title.Visibility = description.Visibility = revision.Visibility = create.Visibility = visibility;
        create.IsEnabled = model.CanCreate;
    }
    public override void Dispose() { model.Changed -= UpdateCreate; base.Dispose(); }
    private async Task Load() { await model.LoadAsync(); RenderItems(); }
    private void RenderItems()
    {
        if (model.Closed) return;
        Results.Children.Clear();
        if (model.Workspace is not { } value) return;
        if (value.Publication is { } publication)
        {
            Results.Children.Add(Design.Text(publication.Title, 24));
            if (!string.IsNullOrWhiteSpace(publication.Description)) Results.Children.Add(new TextBlock { Text = publication.Description, TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true });
            if (value.Revision is { } version)
            {
                Results.Children.Add(Design.Text("版本 · " + version.Label, 18));
                Results.Children.Add(Mute("状态 · " + version.State + "   /   " + value.Items.Count + " 项证据"));
            }
            else Results.Children.Add(Mute("尚未选择论文版本。"));
            Results.Children.Add(Mute("当前展示已登记证据。条目编辑、证据绑定与复现操作尚未接入此页面。"));
            foreach (var group in value.Items.OrderBy(i => i.Ordinal).GroupBy(i => i.Kind))
            {
                var rows = new StackPanel { Spacing = 8 };
                foreach (var item in group) rows.Children.Add(new Border { Padding = new Thickness(12), CornerRadius = new CornerRadius(8),
                    Background = Design.Brush("bg-sunken"), Child = new TextBlock { Text = item.Title, TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true } });
                Results.Children.Add(Disclosure(WorkspaceJourneyModel.KindLabel(group.Key) + $" · {group.Count()}", rows, true));
            }
            if (value.Items.Count == 0) Results.Children.Add(Mute("当前版本尚未登记证据条目。"));
        }
        if (value.Publications.Count == 0) Results.Children.Add(Mute("尚无论文证据。填写论文标题和版本标签，创建第一份记录。"));
        else if (value.Publication == null) Results.Children.Add(Mute("论文记录存在，但当前未返回可展示的版本。请刷新。"));
    }
}

internal sealed class NativeCapabilitiesPage : NativeActionPage
{
    public NativeCapabilitiesPage(INativeSettingsClient client, string project, WispDesign design, Action<string> settings, Action close)
        : base(design, "能力", new WorkspaceActionModel(), close)
    {
        async Task Load() => await State.RunAsync(async () =>
        {
            var bootstrap = await client.InvokeAsync("get_bootstrap_status", new(), project);
            var skills = await client.InvokeAsync("list_skills", new(), project);
            var connections = await client.InvokeAsync("list_mcp_connections", new(), project);
            var memory = await client.InvokeAsync("get_memory_view", new() { ["project_id"] = project }, project);
            return (bootstrap, skills, connections, memory);
        }, value =>
        {
            Results.Children.Clear();
            Results.Children.Add(Mute("运行时 " + value.bootstrap?["app_version"]?.GetValue<string>()));
            Results.Children.Add(Mute(value.bootstrap?["workspace"]?.GetValue<string>() ?? ""));
            foreach (var error in value.bootstrap?["errors"]?.AsArray() ?? []) Results.Children.Add(Warn(error?.GetValue<string>() ?? ""));
            var skills = value.skills?.AsArray().OfType<JsonObject>().Where(s => s["enabled"]?.GetValue<bool>() == true).ToArray() ?? [];
            var connections = value.connections?["connections"]?.AsArray().Count(c => c?["enabled"]?.GetValue<bool>() == true) ?? 0;
            var capabilities = new NativeActionWrap();
            foreach (var (label, count, section) in new[] {
                ("内置技能", skills.Count(s => s["scope"]?.GetValue<string>() == "bundled"), "skills"),
                ("项目技能", skills.Count(s => s["scope"]?.GetValue<string>() != "bundled"), "skills"),
                ("连接", connections, "connections"), ("记忆文件", value.memory?["files"]?.AsArray().Count ?? 0, "memory") })
            {
                var button = Button($"{label} · {count}", () => { settings(section); return Task.CompletedTask; });
                var content = new StackPanel { Spacing = 6 };
                content.Children.Add(Design.Text(count.ToString(), 24)); content.Children.Add(Design.Text(label, 14));
                content.Children.Add(Mute("查看与管理"));
                button.Content = content; button.Width = 220; button.HorizontalContentAlignment = HorizontalAlignment.Left;
                capabilities.Children.Add(button);
            }
            Results.Children.Add(capabilities);
        });
        var refresh = Design.ToolButton("刷新", "refresh"); refresh.Click += async (_, _) => await Load();
        HeaderActions.Children.Add(refresh); _ = Load();
    }
}

internal sealed class NativeCapabilityDetailPage : NativeActionPage
{
    public NativeCapabilityDetailPage(INativeSettingsClient client, string project, string section, WispDesign design, Action close)
        : base(design, section == "skills" ? "技能" : section == "connections" ? "连接" : "记忆文件", new WorkspaceActionModel(), close)
    {
        async Task Load() => await State.RunAsync(() => client.InvokeAsync(section == "skills" ? "list_skills" : section == "connections" ? "list_mcp_connections" : "get_memory_view",
            section == "memory" ? new() { ["project_id"] = project } : new(), project), value =>
        {
            Results.Children.Clear();
            var rows = section == "skills" ? value as JsonArray : value?[section == "connections" ? "connections" : "files"] as JsonArray;
            foreach (var row in rows ?? [])
            {
                var title = row?["name"]?.GetValue<string>() ?? row?["path"]?.GetValue<string>() ?? row?["id"]?.GetValue<string>() ?? "";
                var enabled = row?["enabled"]?.GetValue<bool>();
                Results.Children.Add(new TextBlock { Text = title + (enabled == null ? "" : enabled.Value ? " · 已启用" : " · 未启用"), TextWrapping = TextWrapping.Wrap });
                if (row?["description"] is JsonValue description) Results.Children.Add(Mute(description.GetValue<string>()));
                if (section == "memory" && row?["content"] is JsonValue content) Results.Children.Add(new TextBox { Text = content.GetValue<string>(), IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap });
            }
            if (rows == null || rows.Count == 0) Results.Children.Add(Mute("暂无记录"));
        });
        Form.Children.Add(Mute("当前显示项目能力详情。编辑配置请使用桌面版对应设置页。"));
        Form.Children.Add(Button("刷新", Load)); _ = Load();
    }
}
