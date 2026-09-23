using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeResearchCalendarPage : WorkspaceSheet
{
    private readonly WorkspaceCalendarModel model;
    private readonly StackPanel navigation = new() { Orientation = Orientation.Horizontal, Spacing = 12 };
    private readonly TextBlock month = new() { FontSize = 20, VerticalAlignment = VerticalAlignment.Center };
    private readonly ComboBox filter = new() { Header = "项目", MinWidth = 200 };
    private readonly Grid days = new() { ColumnSpacing = 5, RowSpacing = 5 };
    private readonly TextBlock error = new() { TextWrapping = TextWrapping.Wrap };
    private readonly StackPanel entries = new() { Spacing = 10 };
    private readonly Action<string, DateTime> journey;
    private bool rendering, disposed;
    public NativeResearchCalendarPage(WorkspaceCalendarModel model, WispDesign design, Action<string, DateTime> journey, Action close)
        : base(design, "研究日历", close)
    {
        this.model = model; this.journey = journey;
        design.BindTypography(month, 20);
        Button Action(string label, Func<Task> action)
        {
            var button = new Button { Content = label }; button.Click += async (_, _) => await action(); return button;
        }
        navigation.Children.Add(Action("上个月", () => model.ShiftMonthAsync(-1)));
        navigation.Children.Add(month);
        navigation.Children.Add(Action("下个月", () => model.ShiftMonthAsync(1)));
        navigation.Children.Add(Action("刷新", model.OpenAsync));
        filter.SelectionChanged += (_, _) => { if (rendering) return; model.ProjectFilter = (filter.SelectedItem as ComboBoxItem)?.Tag as string; Render(); };
        Body.Children.Add(navigation); Body.Children.Add(filter); Body.Children.Add(error);
        for (int i = 0; i < 7; i++) days.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        for (int i = 0; i < 7; i++) days.RowDefinitions.Add(new() { Height = GridLength.Auto });
        Body.Children.Add(days); Body.Children.Add(entries);
        model.Changed += Render; Render(); _ = model.OpenAsync();
    }
    private void Render()
    {
        if (disposed) return;
        rendering = true;
        foreach (var control in navigation.Children.OfType<Control>()) control.IsEnabled = model.PrivacyReady && !model.Busy;
        filter.IsEnabled = model.PrivacyReady && !model.Busy;
        month.Text = model.Month.ToString("yyyy 年 M 月");
        error.Text = model.Error ?? (model.Busy ? "正在读取…" : "");
        filter.Items.Clear(); filter.Items.Add(new ComboBoxItem { Content = "全部项目", IsSelected = model.ProjectFilter == null });
        foreach (var project in model.VisibleProjects) filter.Items.Add(new ComboBoxItem { Content = project.Name, Tag = project.Id, IsSelected = model.ProjectFilter == project.Id });
        days.Children.Clear();
        var labels = new[] { "日", "一", "二", "三", "四", "五", "六" };
        for (int i = 0; i < 7; i++) { var label = Mute(labels[i]); Grid.SetColumn(label, i); days.Children.Add(label); }
        var marked = model.Filter(model.MonthRows).SelectMany(p => p.History.Entries).Select(e => DateTimeOffset.FromUnixTimeSeconds(e.OccurredAt).LocalDateTime.Date).ToHashSet();
        for (int day = 1; day <= DateTime.DaysInMonth(model.Month.Year, model.Month.Month); day++)
        {
            var date = new DateTime(model.Month.Year, model.Month.Month, day);
            var position = (int)model.Month.DayOfWeek + day - 1;
            var button = new Button { Content = day + (marked.Contains(date) ? " · 有记录" : ""), MinHeight = 48,
                HorizontalAlignment = HorizontalAlignment.Stretch, IsEnabled = model.PrivacyReady && !model.Busy };
            if (date == model.Day) button.Background = Design.Brush("surface-hover");
            button.Click += async (_, _) => await model.SelectDayAsync(date);
            Grid.SetColumn(button, position % 7); Grid.SetRow(button, position / 7 + 1); days.Children.Add(button);
        }
        entries.Children.Clear(); entries.Children.Add(Mute(model.Day.ToString("yyyy-MM-dd")));
        foreach (var project in model.Filter(model.MonthRows).Where(p => p.Error != null || p.History.Truncated))
            entries.Children.Add(Warn((model.VisibleProjects.FirstOrDefault(p => p.Id == project.ProjectId)?.Name ?? project.ProjectId) + " · " + (project.Error ?? "月视图已截断，可选择日期查看。")));
        foreach (var project in model.Filter(model.DayRows))
        {
            if (project.Error != null) { entries.Children.Add(Warn(project.Error)); continue; }
            if (project.History.Entries.Count == 0) continue;
            var title = model.VisibleProjects.FirstOrDefault(p => p.Id == project.ProjectId)?.Name ?? project.ProjectId;
            var open = new Button { Content = title + " · 打开研究历程" };
            open.Click += (_, _) => journey(project.ProjectId, model.Day); entries.Children.Add(open);
            if (project.History.Truncated) entries.Children.Add(Warn("当日记录已截断。"));
            foreach (var entry in project.History.Entries.OrderByDescending(e => e.OccurredAt))
                entries.Children.Add(Mute($"{DateTimeOffset.FromUnixTimeSeconds(entry.OccurredAt).LocalDateTime:t} · {entry.Title} · {entry.Status}"));
        }
        if (entries.Children.Count == 1 && model.PrivacyReady && !model.Busy) entries.Children.Add(Mute("当天暂无研究活动。"));
        rendering = false;
    }
    public override void HandleEscape()
    {
        if (filter.IsDropDownOpen) { filter.IsDropDownOpen = false; return; }
        base.HandleEscape();
    }
    public override void Dispose() { disposed = true; model.Changed -= Render; model.Dispose(); base.Dispose(); }
}

internal sealed class NativeScratchPage : UserControl, IWorkspaceSheet
{
    private readonly WorkspaceScratchModel scratch;
    private readonly WorkspaceConversationModel conversation;
    private readonly NativeConversationPage page;
    private readonly Action close;
    private readonly TextBlock error = new() { TextWrapping = TextWrapping.Wrap };
    private readonly Button dismiss = new() { Content = "关闭" };
    private bool disposed;
    public NativeScratchPage(WorkspaceScratchModel scratch, INativeSettingsClient host, WispDesign design, Func<Task<string?>> pickAttachment, Action close)
    {
        this.scratch = scratch; this.close = close;
        conversation = new(new NativeConversationClient(host), host);
        page = new(conversation, design, _ => { }, () => Task.CompletedTask, pickAttachment);
        var root = new Grid { Background = design.Brush("bg-app") };
        root.RowDefinitions.Add(new() { Height = GridLength.Auto }); root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        var header = new Grid { Padding = new Thickness(20) };
        header.Children.Add(new TextBlock { Text = "随手一聊", FontSize = 22 });
        dismiss.HorizontalAlignment = HorizontalAlignment.Right; dismiss.Click += (_, _) => HandleEscape(); header.Children.Add(dismiss);
        root.Children.Add(header); Grid.SetRow(error, 1); root.Children.Add(error); Grid.SetRow(page, 2); root.Children.Add(page);
        Content = root; scratch.Changed += Update;
        if (scratch.Session is { } session) _ = conversation.OpenAsync(session.ProjectId, session.SessionId);
    }
    private void Update() { error.Text = scratch.Error ?? ""; dismiss.IsEnabled = !scratch.Busy; }
    public async void HandleEscape()
    {
        if (disposed || scratch.Busy) return;
        if (await scratch.CloseAsync()) close();
    }
    public void Dispose() { disposed = true; page.Dispose(); scratch.Changed -= Update; scratch.Dispose(); }
}
