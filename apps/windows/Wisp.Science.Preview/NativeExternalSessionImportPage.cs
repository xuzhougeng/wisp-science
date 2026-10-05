using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeExternalSessionImportPage : WorkspaceSheet
{
    private readonly WorkspaceExternalImportModel model;
    private readonly ComboBox destination, provider, source;
    private readonly TextBox query;
    private readonly Button refresh, importAll, stop, previous, next;
    private readonly TextBlock counts, progress, pageLabel;
    private readonly StackPanel rows = new() { Spacing = 14 };
    private readonly Func<string, string, Task> open;
    private bool rendering;
    private string sourcesKey = "";

    public NativeExternalSessionImportPage(WorkspaceExternalImportModel model, IReadOnlyList<ProjectSummary> projects,
        WispDesign design, Func<string, string, Task> open, Action close) : base(design, "导入 Codex / Claude 会话", close)
    {
        this.model = model; this.open = open;
        Body.Spacing = 12;
        Body.Children.Add(Mute("从本地、WSL 或 SSH 来源导入到所选项目。默认使用已扫描的列表；重新扫描会读取来源中最近的最多 500 个会话。"));
        ComboBox Choice(string label)
        {
            var box = new ComboBox { Header = label, Width = 230, HorizontalAlignment = HorizontalAlignment.Stretch };
            AutomationProperties.SetName(box, label); design.BindTypography(box); return box;
        }
        destination = Choice("目标项目"); provider = Choice("会话类型"); source = Choice("来源环境");
        foreach (var p in projects.Where(p => !p.Id.StartsWith("assistant:", StringComparison.Ordinal)))
        {
            var item = new ComboBoxItem { Content = p.Name, Tag = p.Id }; destination.Items.Add(item);
            if (p.Id == model.Project) destination.SelectedItem = item;
        }
        provider.Items.Add(new ComboBoxItem { Content = "Codex CLI", Tag = "codex" });
        provider.Items.Add(new ComboBoxItem { Content = "Claude Code", Tag = "claude" }); provider.SelectedIndex = 0;
        var selectors = new NativeActionWrap(); selectors.Children.Add(destination); selectors.Children.Add(provider); selectors.Children.Add(source); Body.Children.Add(selectors);
        async void Select(object sender, SelectionChangedEventArgs args)
        {
            if (rendering || model.Importing || destination.SelectedItem is not ComboBoxItem target || provider.SelectedItem is not ComboBoxItem kind || source.SelectedItem is not ComboBoxItem context) return;
            model.Select((string)target.Tag, (string)kind.Tag, (string)context.Tag);
            await model.LoadAsync(false);
        }
        destination.SelectionChanged += Select; provider.SelectionChanged += Select; source.SelectionChanged += Select;
        query = new TextBox { PlaceholderText = "按标题、工作目录、会话 ID 或路径筛选" };
        AutomationProperties.SetName(query, "筛选外部会话"); design.BindTypography(query);
        query.TextChanged += (_, _) => model.SetQuery(query.Text); Body.Children.Add(query);
        var actions = new NativeActionWrap();
        refresh = Action("重新扫描来源", () => model.LoadAsync(true));
        importAll = Action("导入筛选结果中的待导入会话", model.ImportFilteredAsync);
        stop = Action("停止后续导入", () => { model.StopAfterCurrent(); return Task.CompletedTask; });
        actions.Children.Add(refresh); actions.Children.Add(importAll); actions.Children.Add(stop); Body.Children.Add(actions);
        counts = Mute(""); progress = Mute(""); Body.Children.Add(counts); Body.Children.Add(progress);
        var pages = new NativeActionWrap();
        previous = Action("上一页", () => { model.SetPage(model.Page - 1); return Task.CompletedTask; });
        next = Action("下一页", () => { model.SetPage(model.Page + 1); return Task.CompletedTask; });
        pageLabel = Mute(""); pageLabel.VerticalAlignment = VerticalAlignment.Center;
        pages.Children.Add(previous); pages.Children.Add(pageLabel); pages.Children.Add(next); Body.Children.Add(pages);
        Body.Children.Add(rows);
        model.Changed += Render; Render(); _ = model.InitializeAsync();
    }
    private Button Action(string title, Func<Task> action)
    {
        var button = new Button { Content = title }; Design.ActionButton(button); AutomationProperties.SetName(button, title);
        button.Click += async (_, _) => await action(); return button;
    }
    private void Render()
    {
        if (model.Closed) return;
        rendering = true;
        try
        {
            var key = string.Join('\n', model.Sources.Select(s => s.Id + ":" + s.Label));
            if (key != sourcesKey)
            {
                sourcesKey = key; source.Items.Clear();
                foreach (var context in model.Sources)
                {
                    var option = new ComboBoxItem { Content = context.Id == "local" ? "本地" : $"{context.Label} · {context.Kind.ToUpperInvariant()}", Tag = context.Id };
                    source.Items.Add(option); if (context.Id == model.Context) source.SelectedItem = option;
                }
            }
            destination.IsEnabled = provider.IsEnabled = source.IsEnabled = query.IsEnabled = !model.Importing;
            refresh.IsEnabled = !model.Importing && !model.Loading;
            importAll.IsEnabled = model.CanImportFiltered;
            stop.Visibility = model.Importing ? Visibility.Visible : Visibility.Collapsed; stop.IsEnabled = !model.StopRequested;
            Notices.Children.Clear();
            if (model.SourcesError != null)
            {
                Notices.Children.Add(Warn("未能读取来源环境：" + model.SourcesError));
                var retry = Action("重新读取来源", model.InitializeAsync); retry.IsEnabled = !model.Importing; Notices.Children.Add(retry);
            }
            if (model.Error != null) Notices.Children.Add(Warn(model.Error));
            else if (model.Uncertain) Notices.Children.Add(Warn("此前的导入未能确认。本窗口仅供预览核对，不会再次写入；请核对目标项目后关闭。"));
            Notices.Visibility = Notices.Children.Count == 0 ? Visibility.Collapsed : Visibility.Visible;
            var filtered = model.Filtered;
            counts.Text = model.Loading ? "正在读取来源会话…" : $"显示 {filtered.Length} / {model.Items.Length} 个会话 · 待导入或更新 {filtered.Count(i => i.State != "imported")}";
            counts.Visibility = Visibility.Visible;
            progress.Text = $"{(model.Importing ? "正在导入" : model.StopRequested ? "已停止后续导入" : "本次结果")} {model.Done}/{model.Total} · 新增 {model.Imported} · 更新 {model.Updated} · 跳过 {model.Skipped} · 失败 {model.Failed}";
            progress.Visibility = model.Total > 0 ? Visibility.Visible : Visibility.Collapsed;
            pageLabel.Text = $"第 {model.Page + 1} / {model.PageCount} 页"; pageLabel.Visibility = Visibility.Visible;
            previous.IsEnabled = !model.Importing && model.Page > 0; next.IsEnabled = !model.Importing && model.Page + 1 < model.PageCount;
            rows.Children.Clear();
            if (!model.Loading && filtered.Length == 0) rows.Children.Add(Mute(model.Query.Length > 0 ? "没有匹配的会话。" : "此来源暂无可导入会话。可重新扫描，或切换会话类型及来源环境。"));
            foreach (var item in model.PageItems) AddRow(item);
            Design.ApplyTypography(rows);
        }
        finally { rendering = false; }
    }
    private void AddRow(ExternalImportItem item)
    {
        var body = new StackPanel { Spacing = 6 };
        body.Children.Add(Design.Text(item.Title.Length == 0 ? item.SessionId : item.Title, 16));
        var state = item.State switch { "imported" => "已导入", "updatable" => "可更新", _ => "未导入" };
        body.Children.Add(Mute($"{state} · {item.MessageCount} 条消息 · {item.Cwd}"));
        var path = Mute(item.Path); path.MaxLines = 1; path.TextTrimming = TextTrimming.CharacterEllipsis; ToolTipService.SetToolTip(path, item.Path); body.Children.Add(path);
        var actions = new NativeActionWrap();
        var preview = Action("预览会话", () => model.PreviewAsync(item)); preview.IsEnabled = !model.Importing;
        AutomationProperties.SetName(preview, "预览会话 " + item.Title); actions.Children.Add(preview);
        if (model.Results.TryGetValue(item.Path, out var imported))
        {
            var show = Action("打开导入的会话", () => open(imported.ProjectId, imported.FrameId)); show.IsEnabled = !model.Importing; actions.Children.Add(show);
        }
        body.Children.Add(actions);
        if (model.ItemErrors.TryGetValue(item.Path, out var error)) body.Children.Add(Warn(error));
        if (model.SelectedPath == item.Path)
        {
            if (model.Previewing) body.Children.Add(Mute("正在读取会话预览…"));
            if (model.Preview is { } reviewed)
            {
                body.Children.Add(Mute($"完整来源共 {reviewed.MessageCount} 条消息；以下为前 4 条用户／助手消息，每条最多 600 字符。"));
                foreach (var message in reviewed.Messages)
                {
                    body.Children.Add(Design.Text(message.Role == "user" ? "你" : "助手", 13));
                    var text = new TextBox { IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, Text = message.Text, MaxHeight = 160 };
                    Design.BindTypography(text); body.Children.Add(text);
                }
                var controls = new NativeActionWrap();
                var import = Action("导入此会话", model.ImportPreviewAsync); import.IsEnabled = model.CanImport; controls.Children.Add(import);
                if (reviewed.ExistingSessionId is { } existing)
                {
                    var show = Action("查看目标项目中的已有会话", () => open(reviewed.ProjectId, existing)); show.IsEnabled = !model.Importing; controls.Children.Add(show);
                }
                var hide = Action("收起预览", () => { model.ClosePreview(); return Task.CompletedTask; }); hide.IsEnabled = !model.Importing; controls.Children.Add(hide); body.Children.Add(controls);
            }
        }
        rows.Children.Add(new Border { Child = body, Padding = new Thickness(14), Background = Design.Brush("bg-elev"), CornerRadius = new CornerRadius(10) });
    }
    public override void HandleEscape()
    {
        foreach (var box in new[] { destination, provider, source }) if (box.IsDropDownOpen) { box.IsDropDownOpen = false; return; }
        base.HandleEscape();
    }
    public override void Dispose() { model.Changed -= Render; model.Dispose(); base.Dispose(); }
}
