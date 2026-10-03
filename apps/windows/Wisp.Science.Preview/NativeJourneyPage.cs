using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeJourneyPage : NativeActionPage
{
    private readonly WorkspaceJourneyModel model;
    private readonly StackPanel entries = new() { Spacing = 12 };
    private readonly StackPanel details = new() { Spacing = 12 };
    private readonly Action<string>? openSession;
    private JourneyPage? rendered;
    private string? listError, lastSelectedId;
    private readonly Dictionary<string, Button> entryButtons = [];
    private bool detailVisible;
    private double listOffset;
    private bool runVisible;
    private double runParentOffset;
    private NativeRichPreview? pdf;

    public NativeJourneyPage(INativeJourneyClient client, string projectId, WispDesign design, DateTime? day, Action close, Action<string>? openSession = null)
        : this(new WorkspaceJourneyModel(client, projectId), design, day, close, openSession) { }

    private NativeJourneyPage(WorkspaceJourneyModel model, WispDesign design, DateTime? day, Action close, Action<string>? openSession)
        : base(design, "研究历程", model, close)
    {
        this.model = model; this.openSession = openSession;
        var first = new DateTime(DateTime.Today.Year, DateTime.Today.Month, 1);
        var from = new CalendarDatePicker { Header = "开始日期", Date = new DateTimeOffset(day ?? first) };
        var until = new CalendarDatePicker { Header = "结束日期（含）", Date = new DateTimeOffset(day ?? first.AddMonths(1).AddDays(-1)) };
        from.Width = until.Width = 220;
        var dates = new NativeActionWrap(); dates.Children.Add(from); dates.Children.Add(until); Form.Children.Add(dates);
        Field("搜索标题与摘要", "", text => { model.Query = text; RenderEntries(); });
        var kinds = new ComboBox { Header = "记录类型", HorizontalAlignment = HorizontalAlignment.Stretch };
        foreach (var kind in new[] { "", "artifact", "run", "session", "archive", "finding", "decision", "progress", "next", "data", "paper" })
            kinds.Items.Add(new ComboBoxItem { Tag = kind, Content = kind.Length == 0 ? "全部类型" : WorkspaceJourneyModel.KindLabel(kind) });
        kinds.SelectedIndex = 0;
        kinds.SelectionChanged += (_, _) => { model.Kind = (string)((ComboBoxItem)kinds.SelectedItem).Tag; RenderEntries(); };
        Form.Children.Add(kinds);
        async Task Load()
        {
            if (from.Date == null || until.Date == null || from.Date > until.Date) { model.Fail("请选择有效日期范围。"); return; }
            await model.LoadAsync(WorkspaceCalendarModel.Unix(from.Date.Value.Date), WorkspaceCalendarModel.Unix(until.Date.Value.Date.AddDays(1)));
        }
        Form.Children.Add(Button("读取历程", Load));
        Results.Children.Add(entries); Results.Children.Add(details);
        model.Changed += RenderState;
        RenderState(); _ = Load();
    }

    private void RenderEntries()
    {
        if (model.Closed) return;
        entries.Children.Clear();
        entryButtons.Clear();
        if (model.Page == null) { if (!model.Busy && model.Error == null) entries.Children.Add(Mute("选择日期范围并读取研究历程。")); return; }
        if (model.Error != null) entries.Children.Add(Warn("本次读取失败，以下保留上次成功读取的记录。"));
        if (model.Page.Truncated) entries.Children.Add(Warn("结果已截断，请缩小日期范围；以下为已载入的记录。"));
        var days = model.Days();
        if (days.Length == 0) entries.Children.Add(Mute(model.Page.Entries.Count == 0 ? "这个日期范围没有研究记录。" : "没有匹配的记录，请调整搜索或类型筛选。"));
        foreach (var day in days)
        {
            entries.Children.Add(Design.Text($"{day.Day:yyyy-MM-dd} · {day.Entries.Length} 条记录", 18));
            foreach (var row in day.Entries)
            {
                var content = new StackPanel { Spacing = 5 };
                content.Children.Add(Mute($"{DateTimeOffset.FromUnixTimeSeconds(row.OccurredAt).LocalDateTime:HH:mm} · {WorkspaceJourneyModel.KindLabel(row.Kind)}{(row.Manual ? " · 手动记录" : "")}"));
                content.Children.Add(Design.Text(row.Title, 16));
                if (row.Summary.Length > 0) content.Children.Add(Mute(row.Summary));
                if (row.SourceDiscarded) content.Children.Add(Warn("源文件已不可用"));
                var button = new Button { Content = content, HorizontalAlignment = HorizontalAlignment.Stretch,
                    HorizontalContentAlignment = HorizontalAlignment.Stretch, Padding = new Thickness(12) };
                button.Click += async (_, _) => await model.SelectAsync(row);
                if (lastSelectedId == row.Id) button.Background = Design.Brush("surface-hover");
                entryButtons[row.Id] = button;
                entries.Children.Add(button);
            }
        }
    }

    private void RenderState()
    {
        if (model.Closed) return;
        if (rendered != model.Page || listError != model.Error) { rendered = model.Page; listError = model.Error; RenderEntries(); }
        if (model.Selected is { } selectedEntry)
        {
            lastSelectedId = selectedEntry.Id;
            foreach (var (id, button) in entryButtons)
                if (id == lastSelectedId) button.Background = Design.Brush("surface-hover"); else button.ClearValue(Control.BackgroundProperty);
        }
        var showing = model.Selected != null;
        var showingRun = model.Run != null;
        if (showingRun && !runVisible)
        {
            runParentOffset = BodyScroll.VerticalOffset;
            BodyScroll.ChangeView(null, 0, null, true);
        }
        if (!showingRun && runVisible && showing)
            DispatcherQueue.TryEnqueue(() => { if (!model.Closed && model.Run == null && model.Selected != null) { BodyScroll.UpdateLayout(); BodyScroll.ChangeView(null, runParentOffset, null, true); } });
        runVisible = showingRun;
        if (showing && !detailVisible) { listOffset = BodyScroll.VerticalOffset; BodyScroll.ChangeView(null, 0, null, true); }
        Form.Visibility = entries.Visibility = showing ? Visibility.Collapsed : Visibility.Visible;
        details.Visibility = showing ? Visibility.Visible : Visibility.Collapsed;
        if (!showing && detailVisible) DispatcherQueue.TryEnqueue(() => { if (!model.Closed && model.Selected == null) { BodyScroll.UpdateLayout(); BodyScroll.ChangeView(null, listOffset, null, true); } });
        detailVisible = showing;
        pdf?.Dispose(); pdf = null; details.Children.Clear();
        if (model.Selected is not { } selected) return;
        details.Children.Add(Button(model.Run != null || model.HasParentSource ? "返回上级来源" : "返回研究历程", () => { model.Back(); return Task.CompletedTask; }));
        if (model.Run is { } run)
        {
            details.Children.Add(Design.Text(run.Title, 22));
            details.Children.Add(Mute($"运行来源 · 只读 · {run.Id}"));
            details.Children.Add(Mute($"状态 · {run.Status} · 执行环境 · {run.ContextId}"));
            if (run.ExitCode is { } exit) details.Children.Add(Mute($"退出码 · {exit}"));
            foreach (var (label, value) in new[] { ("命令", run.Command), ("远端工作目录", run.RemoteWorkdir), ("标准输出（尾部）", run.StdoutTail), ("标准错误（尾部）", run.StderrTail), ("最近轮询错误", run.LastPollError) })
            {
                if (string.IsNullOrEmpty(value)) continue;
                details.Children.Add(Design.Text(label, 16));
                details.Children.Add(new TextBox { Text = value, IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MaxHeight = 320 });
            }
            Design.ApplyTypography(details);
            return;
        }
        details.Children.Add(Design.Text(selected.Title, 22));
        details.Children.Add(Mute(WorkspaceJourneyModel.KindLabel(selected.Kind) + " · " + selected.Status));
        if (selected.Summary.Length > 0) details.Children.Add(new TextBlock { Text = selected.Summary, TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true });
        if (selected.RecordedAt is { } recorded && selected.Manual)
            details.Children.Add(Mute("补记时间 · " + DateTimeOffset.FromUnixTimeSeconds(recorded).LocalDateTime.ToString("g")));
        if (selected.SourceDiscarded) details.Children.Add(Warn("源文件已不可用；保留的记录信息仍可查看。"));
        if (openSession != null && !string.IsNullOrEmpty(selected.FrameId))
            details.Children.Add(Button("查看来源会话", () => { openSession(selected.FrameId); return Task.CompletedTask; }));
        if (model.SourceLoading) details.Children.Add(new ProgressBar { IsIndeterminate = true, Width = 160 });
        if (!string.IsNullOrEmpty(model.SourceRunId))
        {
            var openRun = Button("查看来源运行", () => model.OpenRunAsync());
            openRun.IsEnabled = !model.SourceLoading;
            details.Children.Add(openRun);
        }
        if (model.SourceError != null)
        {
            details.Children.Add(Warn("来源读取失败：" + model.SourceError));
            if (model.Artifact == null && selected.Kind == "artifact" && selected.SourceId is { } id)
                details.Children.Add(Button("重新读取来源", () => model.OpenArtifactAsync(id)));
        }
        if (model.Artifact is not { } artifact) return;
        details.Children.Add(Design.Text($"{artifact.Filename} · 版本 {artifact.VersionNumber}", 18));
        var source = artifact.Source;
        details.Children.Add(Mute(string.IsNullOrEmpty(source.RunTitle) ? "尚未记录生成它的运行" : $"生成运行 · {source.RunTitle} · {source.RunStatus}"));
        if (!string.IsNullOrEmpty(source.ContextId)) details.Children.Add(Mute("执行环境 · " + source.ContextId));
        details.Children.Add(Design.Text("输入来源", 16));
        if (source.Inputs.Length == 0) details.Children.Add(Mute("尚未记录输入数据"));
        foreach (var input in source.Inputs)
        {
            if (input.VersionId is { } version)
            {
                var button = Button(input.Title, () => model.OpenArtifactAsync(version)); button.IsEnabled = !model.SourceLoading;
                details.Children.Add(button);
            }
            else details.Children.Add(Mute(input.Title));
            details.Children.Add(Mute(input.Role + " · " + input.Confidence));
        }
        if (artifact.ContentError != null) details.Children.Add(Warn("内容不可用：" + artifact.ContentError));
        else if (artifact.Text != null) details.Children.Add(new TextBox { Text = artifact.Text, IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MaxHeight = 420 });
        else if (artifact.Mime == "application/pdf" && artifact.Base64 != null && !artifact.Truncated)
        { pdf = new NativeRichPreview(Design, "pdf", artifact.Base64, artifact.Filename); details.Children.Add(pdf); }
        else details.Children.Add(Mute("内容类型 · " + artifact.Mime));
        if (artifact.Truncated) details.Children.Add(Warn("内容被截断，当前显示部分内容。"));
        Design.ApplyTypography(details);
    }

    public override void HandleEscape() { if (model.Selected != null) model.Back(); else base.HandleEscape(); }
    public override void Dispose() { model.Changed -= RenderState; pdf?.Dispose(); base.Dispose(); }
}

internal sealed class NativeJourneyConversationPage : NativeActionPage
{
    public NativeJourneyConversationPage(INativeConversationClient client, string project, string session, WispDesign design, Action close)
        : base(design, "来源会话 · 只读", new WorkspaceActionModel(), close)
    {
        long? before = null;
        var older = new Button { Content = "更早的消息", Visibility = Visibility.Collapsed };
        older.Click += async (_, _) => await Read();
        Form.Children.Add(older);
        Form.Children.Add(Button("重新读取最新消息", async () => { before = null; await Read(); }));
        async Task Read()
        {
            await State.RunAsync(() => client.SnapshotAsync(project, session, before), snapshot =>
            {
                before = snapshot.NextBeforeSeq; older.Visibility = before == null ? Visibility.Collapsed : Visibility.Visible;
                Results.Children.Clear();
                foreach (var row in snapshot.Items) Results.Children.Add(TranscriptView.Create(new BrowserMessage(0, row.Role, row.Text, row.ToolName), design));
                if (snapshot.Items.Length == 0) Results.Children.Add(Mute("没有可显示的消息。"));
            });
        }
        _ = Read();
    }
}
