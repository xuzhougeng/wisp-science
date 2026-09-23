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
        formHost.IsEnabled = !State.Busy; resultsHost.IsEnabled = !State.Busy;
        progress.Visibility = State.Busy ? Visibility.Visible : Visibility.Collapsed;
        error.Text = State.Error ?? ""; error.Visibility = State.Error == null ? Visibility.Collapsed : Visibility.Visible;
    }
    protected TextBox Field(string title, string text, Action<string> changed, bool multiline = false)
    {
        var input = new TextBox { Header = title, Text = text, AcceptsReturn = multiline, TextWrapping = TextWrapping.Wrap, MinHeight = multiline ? 85 : 32 };
        input.TextChanged += (_, _) => changed(input.Text); Form.Children.Add(input); return input;
    }
    protected static Button Button(string title, Func<Task> action)
    {
        var button = new Button { Content = title };
        button.Click += async (_, _) => await action(); return button;
    }
    public override void HandleEscape()
    {
        foreach (var combo in Form.Children.OfType<ComboBox>())
            if (combo.IsDropDownOpen) { combo.IsDropDownOpen = false; return; }
        foreach (var picker in Form.Children.OfType<CalendarDatePicker>())
            if (picker.IsCalendarOpen) { picker.IsCalendarOpen = false; return; }
        if (!State.Busy) base.HandleEscape();
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
        var input = Field("项目归档（ZIP）", "", s => path = s);
        Form.Children.Add(Button("选择归档", async () => { if (await pickArchive() is { } selected) input.Text = selected; }));
        Form.Children.Add(Button("导入", async () =>
        {
            if (string.IsNullOrWhiteSpace(path)) { model.Fail("请选择项目归档。"); return; }
            if (await model.ImportAsync(path) && model.Created is { } row) await opened(row);
        }));
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
        var filters = new ComboBox { Header = "类型" };
        foreach (var (label, kind) in new[] { ("全部", ""), ("代码", "code"), ("图片", "figure"), ("文本", "text") })
            filters.Items.Add(new ComboBoxItem { Content = label, Tag = kind });
        filters.SelectedIndex = 0;
        filters.SelectionChanged += async (_, _) => { model.Kind = (string)((ComboBoxItem)filters.SelectedItem).Tag; await Search(); };
        Form.Children.Add(filters); Form.Children.Add(Button("搜索", Search));
        _ = Search();
    }
    private async Task Search() { await model.SearchAsync(); RenderItems(); }
    private void RenderItems()
    {
        if (model.Closed) return;
        Results.Children.Clear();
        if (model.Items.Count == 0 && model.Error == null) Results.Children.Add(Mute("收藏库是空的"));
        foreach (var item in model.Items)
        {
            var card = new StackPanel { Spacing = 8 };
            card.Children.Add(new TextBlock { Text = item.Title, FontSize = 18, TextWrapping = TextWrapping.Wrap });
            card.Children.Add(Mute(item.SourceProjectName + " / " + item.SourceSessionTitle));
            card.Children.Add(new TextBox { Text = item.CodePreview, IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MaxHeight = 180 });
            var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            if (insert != null) actions.Children.Add(Button("填入对话框", () => { insert(item); return Task.CompletedTask; }));
            actions.Children.Add(Button("打开来源", () => source(item)));
            actions.Children.Add(Button("删除", async () => { await model.DeleteAsync(item.Id); RenderItems(); }));
            card.Children.Add(actions); Results.Children.Add(card);
        }
    }
}

internal sealed class NativePublicationPage : NativeActionPage
{
    private readonly WorkspacePublicationModel model;
    private readonly TextBox title, description, revision;
    public NativePublicationPage(WorkspacePublicationModel model, WispDesign design, Action close) : base(design, "论文证据", model, close)
    {
        this.model = model;
        title = Field("论文标题", model.Title, s => model.Title = s);
        description = Field("描述", model.Description, s => model.Description = s, true);
        revision = Field("版本标签", model.RevisionLabel, s => model.RevisionLabel = s);
        Form.Children.Add(Button("创建论文证据", async () =>
        {
            if (await model.CreateAsync()) { title.Text = model.Title; description.Text = model.Description; revision.Text = model.RevisionLabel; }
            RenderItems();
        }));
        Form.Children.Add(Button("刷新", Load)); _ = Load();
    }
    private async Task Load() { await model.LoadAsync(); RenderItems(); }
    private void RenderItems()
    {
        if (model.Closed) return;
        Results.Children.Clear();
        if (model.Workspace is not { } value) return;
        foreach (var publication in value.Publications) Results.Children.Add(new TextBlock { Text = publication.Title + "\n" + publication.Description, TextWrapping = TextWrapping.Wrap });
        if (value.Revision is { } version) Results.Children.Add(Mute(version.Label + " · " + version.State));
        foreach (var item in value.Items.OrderBy(i => i.Ordinal)) Results.Children.Add(Mute(item.Kind + " · " + item.Title));
        if (value.Publications.Count == 0) Results.Children.Add(Mute("尚无论文证据"));
    }
}

internal sealed class NativeJourneyPage : NativeActionPage
{
    public NativeJourneyPage(INativeJourneyClient client, string projectId, WispDesign design, DateTime? day, Action close)
        : base(design, "研究历程", new WorkspaceActionModel(), close)
    {
        var first = new DateTime(DateTime.Today.Year, DateTime.Today.Month, 1);
        var from = new CalendarDatePicker { Header = "开始日期", Date = new DateTimeOffset(day ?? first) };
        var until = new CalendarDatePicker { Header = "结束日期（含）", Date = new DateTimeOffset(day ?? first.AddMonths(1).AddDays(-1)) };
        Form.Children.Add(from); Form.Children.Add(until);
        JourneyPage? loaded = null;
        var query = "";
        void RenderEntries()
        {
            Results.Children.Clear();
            if (loaded == null) return;
            if (loaded.Truncated) Results.Children.Add(Warn("结果已截断，请缩小日期范围。"));
            foreach (var item in loaded.Entries.Where(e => e.Title.Contains(query.Trim(), StringComparison.CurrentCultureIgnoreCase)).OrderByDescending(e => e.OccurredAt))
                Results.Children.Add(Mute($"{DateTimeOffset.FromUnixTimeSeconds(item.OccurredAt).LocalDateTime:g} · {item.Title}"));
            if (Results.Children.Count == 0) Results.Children.Add(Mute("暂无匹配的研究记录。"));
        }
        Field("搜索已载入的记录", "", text => { query = text; RenderEntries(); });
        async Task Load()
        {
            if (from.Date == null || until.Date == null || from.Date > until.Date) { State.Fail("请选择有效日期范围。"); return; }
            await State.RunAsync(() => client.ReadAsync(projectId, WorkspaceCalendarModel.Unix(from.Date.Value.Date), WorkspaceCalendarModel.Unix(until.Date.Value.Date.AddDays(1))), rows =>
            {
                loaded = rows; RenderEntries();
            });
        }
        Form.Children.Add(Button("读取历程", Load)); _ = Load();
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
            foreach (var (label, count, section) in new[] {
                ("内置技能", skills.Count(s => s["scope"]?.GetValue<string>() == "bundled"), "skills"),
                ("项目技能", skills.Count(s => s["scope"]?.GetValue<string>() != "bundled"), "skills"),
                ("连接", connections, "connections"), ("记忆文件", value.memory?["files"]?.AsArray().Count ?? 0, "memory") })
                Results.Children.Add(Button($"{label} · {count}", () => { settings(section); return Task.CompletedTask; }));
        });
        Form.Children.Add(Button("刷新", Load)); _ = Load();
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
