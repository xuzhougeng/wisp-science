using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;
using Windows.Storage;
using Windows.Storage.Pickers;

namespace Wisp.Science.Preview;

internal interface IWorkspaceSheet : IDisposable
{
    void HandleEscape();
}

internal abstract class WorkspaceSheet : UserControl, IWorkspaceSheet
{
    private readonly Action close;
    private bool closed;
    protected readonly WispDesign Design;
    protected WorkspaceSheet(WispDesign design, string title, Action close)
    {
        Design = design; this.close = close;
        design.BindTypography(this);
        var root = new Grid { Background = design.Brush("bg-app") };
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        var header = new Grid { Padding = new Thickness(20, 16, 20, 12), ColumnSpacing = 12 };
        header.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var heading = design.Text(title, 22); heading.Foreground = design.Brush("text"); header.Children.Add(heading);
        var dismiss = new Button { Content = "关闭" };
        dismiss.Click += (_, _) => HandleEscape();
        Grid.SetColumn(dismiss, 1); header.Children.Add(dismiss);
        root.Children.Add(header);
        Notices = new StackPanel { Spacing = 8, Padding = new Thickness(20, 0, 20, 12), Visibility = Visibility.Collapsed };
        Grid.SetRow(Notices, 1); root.Children.Add(Notices);
        Body = new StackPanel { Spacing = 12, Padding = new Thickness(20, 0, 20, 20) };
        var scroll = new ScrollViewer { Content = Body, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
        Grid.SetRow(scroll, 2); root.Children.Add(scroll);
        Content = root;
        Loaded += (_, _) => Design.ApplyTypography(this);
    }
    protected StackPanel Body { get; }
    protected StackPanel Notices { get; }
    public virtual void HandleEscape() { if (!closed) close(); }
    public virtual void Dispose() { closed = true; }
    protected TextBlock Mute(string text) { var block = Design.Text(text, 13); block.Foreground = Design.Brush("text-muted"); return block; }
    protected TextBlock Warn(string text) { var block = Design.Text(text, 13); block.Foreground = Design.Brush("clay-strong"); return block; }
}

internal sealed class NativeOutlinePage : WorkspaceSheet
{
    private readonly WorkspaceOutlineModel model;
    private readonly CancellationTokenSource lifetime = new();
    private readonly TextBox search = new() { PlaceholderText = "搜索问题" };
    private readonly StackPanel list = new() { Spacing = 8 };
    private bool chrome;
    public NativeOutlinePage(WorkspaceOutlineModel model, WispDesign design, Action close) : base(design, "会话大纲", close)
    {
        this.model = model;
        search.TextChanged += (_, _) => { model.Query = search.Text; RenderList(); };
        _ = LoadAsync();
    }
    private async Task LoadAsync()
    {
        try { await model.RefreshAsync(lifetime.Token); Render(); }
        catch (OperationCanceledException) { }
    }
    private void Render()
    {
        if (!chrome)
        {
            chrome = true;
            var refresh = new Button { Content = "刷新" };
            refresh.Click += async (_, _) => { await model.RefreshAsync(lifetime.Token); Render(); };
            Body.Children.Add(refresh); Body.Children.Add(search); Body.Children.Add(list);
        }
        RenderList();
    }
    private void RenderList()
    {
        list.Children.Clear();
        if (model.Loading) list.Children.Add(new ProgressBar { IsIndeterminate = true, Height = 3 });
        if (model.Error is { } error) list.Children.Add(Warn(error));
        foreach (var entry in model.Visible)
        {
            var captured = entry;
            var row = new StackPanel { Spacing = 4 };
            row.Children.Add(new TextBlock { Text = $"{entry.UserIndex + 1}. {entry.Text}", TextWrapping = TextWrapping.Wrap });
            if (entry.SentAt is > 0 and var sent)
                row.Children.Add(Mute(entry.ResponseAt is { } response && response >= sent ? $"{response - sent} 秒" : ""));
            var button = new Button { Content = row, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Left };
            button.Click += async (_, _) => { await model.OpenQuestionAsync(captured, lifetime.Token); Render(); };
            list.Children.Add(button);
        }
        if (!model.Visible.Any() && !model.Loading && model.Error == null) list.Children.Add(Mute("暂无问题"));
        if (model.History is { } history)
        {
            list.Children.Add(Mute("历史定位"));
            var index = 0;
            foreach (var item in history.Items)
            {
                var block = new TextBlock { Text = item.Text, TextWrapping = TextWrapping.Wrap };
                list.Children.Add(new Border
                {
                    Child = block, Padding = new Thickness(8), CornerRadius = new CornerRadius(8),
                    Background = index == model.HistoryItemIndex ? Design.Brush("surface-hover") : Design.Brush("bg-elev")
                });
                index++;
            }
        }
    }
    public override void Dispose() { lifetime.Cancel(); model.Close(); lifetime.Dispose(); base.Dispose(); }
}

internal sealed class NativeInboxPage : WorkspaceSheet
{
    private readonly WorkspaceInboxModel model;
    private readonly string projectId;
    private readonly Func<NativeInboxEntry, Task> open;
    public NativeInboxPage(WorkspaceInboxModel model, string projectId, WispDesign design, Func<NativeInboxEntry, Task> open, Action close)
        : base(design, "待查看", close)
    {
        this.model = model; this.projectId = projectId; this.open = open;
        var refresh = new Button { Content = "刷新" };
        refresh.Click += async (_, _) => { await model.RefreshAsync(projectId); RenderBody(); };
        Body.Children.Add(refresh);
        RenderBody();
    }
    public void RenderBody()
    {
        while (Body.Children.Count > 1) Body.Children.RemoveAt(Body.Children.Count - 1);
        if (model.Error is { } error) Body.Children.Add(Warn(error));
        if (model.Loading) Body.Children.Add(new ProgressBar { IsIndeterminate = true, Height = 3 });
        foreach (var entry in model.Entries)
        {
            var captured = entry;
            var row = new StackPanel { Spacing = 4 };
            row.Children.Add(Mute(entry.ProjectName));
            row.Children.Add(new TextBlock { Text = entry.Title, TextWrapping = TextWrapping.Wrap });
            var button = new Button { Content = row, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Left };
            button.Click += async (_, _) => await open(captured);
            Body.Children.Add(button);
        }
        if (model.Entries.Length == 0 && !model.Loading && model.Error == null) Body.Children.Add(Mute("暂无待查看的会话"));
    }
}

internal sealed class NativeTrajectoryPage : WorkspaceSheet
{
    private readonly WorkspaceTrajectoryModel model;
    private readonly Func<string, string, Task> saveHtml;
    private readonly TextBox search = new() { PlaceholderText = "搜索步骤、输入或输出" };
    private readonly ComboBox axis = new() { Header = "时间轴" };
    private readonly StackPanel list = new() { Spacing = 8 };
    private readonly TextBlock status = new() { TextWrapping = TextWrapping.Wrap };
    private bool chrome;
    public NativeTrajectoryPage(WorkspaceTrajectoryModel model, WispDesign design, Func<string, string, Task> saveHtml, Action close)
        : base(design, "运行轨迹", close)
    {
        this.model = model; this.saveHtml = saveHtml;
        foreach (var (label, value) in new (string, NativeTrajectoryAxis)[] { ("耗时", NativeTrajectoryAxis.Duration), ("轮次", NativeTrajectoryAxis.Turns), ("调用", NativeTrajectoryAxis.Calls) })
            axis.Items.Add(new ComboBoxItem { Content = label, Tag = value });
        axis.SelectedIndex = 0;
        search.TextChanged += (_, _) => { model.Query = search.Text; RenderList(); };
        axis.SelectionChanged += (_, _) =>
        {
            if (axis.SelectedItem is ComboBoxItem item) { model.Axis = (NativeTrajectoryAxis)item.Tag; RenderList(); }
        };
        _ = StartAsync();
    }
    private async Task StartAsync()
    {
        await model.RefreshAsync();
        Render();
    }
    private void Render()
    {
        if (!chrome)
        {
            chrome = true;
            var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            var refresh = new Button { Content = "刷新" };
            refresh.Click += async (_, _) => { await model.RefreshAsync(); Render(); };
            var export = new Button { Content = "导出 HTML" };
            export.Click += async (_, _) =>
            {
                try { await saveHtml("wisp-trajectory.html", await model.ExportHtmlAsync()); }
                catch (Exception ex) { status.Text = ex.Message; status.Foreground = Design.Brush("clay-strong"); }
            };
            actions.Children.Add(refresh); actions.Children.Add(export);
            Body.Children.Add(actions); Body.Children.Add(search); Body.Children.Add(axis); Body.Children.Add(status); Body.Children.Add(list);
        }
        status.Text = model.Error ?? (model.Snapshot is { } snapshot
            ? $"{snapshot.Stats.Turns} 轮 · {snapshot.Stats.Steps} 步 · 模型 {snapshot.Stats.LlmMs} ms · 工具 {snapshot.Stats.ToolMs} ms"
            : "");
        status.Foreground = Design.Brush(model.Error == null ? "text-muted" : "clay-strong");
        RenderList();
    }
    private void RenderList()
    {
        list.Children.Clear();
        var chart = new Grid { Height = 36, ColumnSpacing = 1 };
        var column = 0;
        foreach (var segment in model.Segments)
        {
            chart.ColumnDefinitions.Add(new() { Width = new GridLength(Math.Max(0.5, segment.WidthPct), GridUnitType.Star) });
            var token = segment.Lane == "input" ? "traj-input-bar" : segment.Lane == "tools" ? "traj-tool-bar" : "traj-model-bar";
            var bar = new Border { Background = Design.Brush(token), CornerRadius = new CornerRadius(2) };
            Grid.SetColumn(bar, column++); chart.Children.Add(bar);
        }
        if (model.Segments.Length > 0) list.Children.Add(chart);
        foreach (var row in model.Rows)
        {
            var card = new StackPanel { Spacing = 4, Padding = new Thickness(10) };
            card.Children.Add(Mute(row.Cell.Kind + (row.Cell.DurationMs is { } ms ? $" · {ms} ms" : "")));
            card.Children.Add(new TextBlock { Text = row.Cell.Summary, TextWrapping = TextWrapping.Wrap });
            list.Children.Add(new Border { Child = card, Background = Design.Brush("bg-elev"), CornerRadius = new CornerRadius(8), Padding = new Thickness(4) });
        }
        if (model.Rows.Length == 0) list.Children.Add(Mute(model.Loading ? "正在读取轨迹…" : "没有匹配的轨迹记录"));
    }
    public override void Dispose() { model.Close(); base.Dispose(); }
}

internal sealed class NativeArchivePage : WorkspaceSheet
{
    private readonly WorkspaceArchiveModel model;
    private readonly Func<string, Task> continued;
    public NativeArchivePage(WorkspaceArchiveModel model, WispDesign design, Func<string, Task> continued, Action close)
        : base(design, "研究归档", close)
    {
        this.model = model; this.continued = continued;
        _ = StartAsync();
    }
    private async Task StartAsync() { await model.LoadAsync(); Render(); }
    public override void HandleEscape() { if (!model.Busy) base.HandleEscape(); }
    private void Render()
    {
        Body.Children.Clear();
        if (model.Error is { } error) Body.Children.Add(Warn(error));
        if (model.Busy) Body.Children.Add(Mute("正在整理或保存研究材料…"));
        if (model.Archive is { } archive)
        {
            Body.Children.Add(new TextBlock { Text = model.Frozen ? "已归档研究节点" : "确认研究归档", FontSize = 16 });
            var title = new TextBox { Header = "节点标题", Text = archive.Title, IsReadOnly = model.Frozen || model.Busy };
            var report = new TextBox { Header = "研究问题、结论、局限与选择依据", Text = archive.Report, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 160, IsReadOnly = model.Frozen || model.Busy };
            Body.Children.Add(title); Body.Children.Add(report);
            foreach (var file in archive.Files)
                Body.Children.Add(Mute($"{file.Path} · {file.Action} · {file.SizeBytes} bytes"));
            Body.Children.Add(Mute(model.DeletedBytes > 0 ? $"将永久删除：{model.DeletedBytes} bytes" : "没有待删除文件"));
            foreach (var warning in archive.Warnings) Body.Children.Add(Warn(warning));
            if (model.Frozen)
            {
                var retry = new Button { Content = "重试未完成清理", IsEnabled = !model.Busy };
                retry.Click += async (_, _) => { await model.RetryCleanupAsync(); Render(); };
                var next = new Button { Content = "继续研究", IsEnabled = !model.Busy };
                next.Click += async (_, _) => { if (await model.ContinueAsync() is { } id) await continued(id); else Render(); };
                Body.Children.Add(retry); Body.Children.Add(next);
            }
            else
            {
                var consent = new CheckBox { Content = "我已检查归档材料。确认后会话将只读，所选文件立即永久删除，无法撤销。", IsChecked = model.Accepted, IsEnabled = !model.Busy };
                consent.Checked += (_, _) => model.Accepted = true;
                consent.Unchecked += (_, _) => model.Accepted = false;
                var prepare = new Button { Content = "重新整理", IsEnabled = !model.Busy };
                prepare.Click += async (_, _) => { await model.PrepareAsync(); Render(); };
                var confirm = new Button { Content = "确认归档并清理", IsEnabled = model.CanConfirm };
                confirm.Click += async (_, _) =>
                {
                    model.Edit(title.Text, report.Text);
                    await model.ConfirmAsync(); Render();
                };
                Body.Children.Add(consent); Body.Children.Add(prepare); Body.Children.Add(confirm);
            }
        }
        else if (!model.Busy)
        {
            var retry = new Button { Content = "重新读取" };
            retry.Click += async (_, _) => { await model.LoadAsync(); Render(); };
            Body.Children.Add(retry);
        }
    }
}

internal sealed class NativeSharePage : WorkspaceSheet
{
    private readonly WorkspaceShareModel model;
    private readonly Func<string, string, Task> saveHtml;
    public NativeSharePage(WorkspaceShareModel model, WispDesign design, Func<string, string, Task> saveHtml, Action close)
        : base(design, "分享对话", close)
    {
        this.model = model; this.saveHtml = saveHtml;
        _ = StartAsync();
    }
    private async Task StartAsync() { await model.LoadAsync(); Render(); }
    private void Render()
    {
        Body.Children.Clear();
        Body.Children.Add(Mute("选择要导出的消息。思考内容默认不选中；编辑和脱敏只影响导出副本。PNG 导出仍是后续工作。"));
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        var all = new Button { Content = "全选" }; all.Click += (_, _) => { model.SelectAll(true); Render(); };
        var none = new Button { Content = "全不选" }; none.Click += (_, _) => { model.SelectAll(false); Render(); };
        actions.Children.Add(all); actions.Children.Add(none);
        actions.Children.Add(Mute($"已选 {model.Selected.Length}/{model.Rows.Length}"));
        Body.Children.Add(actions);
        if (model.Error is { } error) Body.Children.Add(Warn(error));
        foreach (var row in model.Rows)
        {
            var captured = row.Id;
            var box = new CheckBox { Content = row.Row.Role == "user" ? "你" : row.Row.Role == "reasoning" ? "思考" : "Wisp Science", IsChecked = row.Selected };
            box.Checked += (_, _) => model.SetSelected(captured, true);
            box.Unchecked += (_, _) => model.SetSelected(captured, false);
            var editor = new TextBox { Text = row.Row.Text, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 80 };
            editor.TextChanged += (_, _) => model.Edit(captured, editor.Text);
            Body.Children.Add(box); Body.Children.Add(editor);
        }
        var keywords = new TextBox { Header = "脱敏关键词，以逗号分隔", Text = model.Keywords };
        keywords.TextChanged += (_, _) => model.Keywords = keywords.Text;
        Body.Children.Add(keywords);
        var export = new Button { Content = "导出 HTML", IsEnabled = model.Selected.Length > 0 };
        export.Click += async (_, _) =>
        {
            try { await saveHtml("wisp-share.html", await model.HtmlAsync(Design.Dark)); }
            catch (Exception ex) { Body.Children.Insert(0, Warn(ex.Message)); }
        };
        Body.Children.Add(export);
        if (model.Rows.Length == 0 && !model.Loading) Body.Children.Add(Mute("没有可分享的消息"));
    }
}

internal static class NativeWorkspaceFiles
{
    public static async Task SaveHtmlAsync(Window window, string name, string contents)
    {
        var picker = new FileSavePicker();
        WinRT.Interop.InitializeWithWindow.Initialize(picker, WinRT.Interop.WindowNative.GetWindowHandle(window));
        picker.FileTypeChoices.Add("HTML", [".html"]);
        picker.SuggestedFileName = name;
        if (await picker.PickSaveFileAsync() is { } file)
            await FileIO.WriteTextAsync(file, contents);
    }
}
