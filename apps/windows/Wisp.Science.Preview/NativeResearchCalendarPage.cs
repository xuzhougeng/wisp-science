using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeResearchCalendarPage : WorkspaceSheet
{
    private readonly WorkspaceCalendarModel model;
    private readonly NativeActionWrap navigation = new();
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
            var button = new Button { Content = label }; design.ActionButton(button); button.Click += async (_, _) => await action(); return button;
        }
        navigation.Children.Add(Action("上个月", () => model.ShiftMonthAsync(-1)));
        navigation.Children.Add(month);
        navigation.Children.Add(Action("下个月", () => model.ShiftMonthAsync(1)));
        navigation.Children.Add(Action("刷新", model.OpenAsync));
        filter.SelectionChanged += (_, _) => { if (rendering) return; model.ProjectFilter = (filter.SelectedItem as ComboBoxItem)?.Tag as string; Render(); };
        Body.Children.Add(navigation); Body.Children.Add(filter); Body.Children.Add(error);
        for (int i = 0; i < 7; i++) days.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        for (int i = 0; i < 7; i++) days.RowDefinitions.Add(new() { Height = GridLength.Auto });
        Body.Children.Add(design.Card(days)); Body.Children.Add(entries);
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
        error.Visibility = error.Text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        filter.Items.Clear(); filter.Items.Add(new ComboBoxItem { Content = "全部项目", IsSelected = model.ProjectFilter == null });
        foreach (var project in model.VisibleProjects) filter.Items.Add(new ComboBoxItem { Content = project.Name, Tag = project.Id, IsSelected = model.ProjectFilter == project.Id });
        days.Children.Clear();
        var labels = new[] { "日", "一", "二", "三", "四", "五", "六" };
        for (int i = 0; i < 7; i++) { var label = Mute(labels[i]); label.HorizontalAlignment = HorizontalAlignment.Center;
            label.Margin = new Thickness(0, 0, 0, 10); Grid.SetColumn(label, i); days.Children.Add(label); }
        var marked = model.Filter(model.MonthRows).SelectMany(p => p.History.Entries).Select(e => DateTimeOffset.FromUnixTimeSeconds(e.OccurredAt).LocalDateTime.Date).ToHashSet();
        for (int day = 1; day <= DateTime.DaysInMonth(model.Month.Year, model.Month.Month); day++)
        {
            var date = new DateTime(model.Month.Year, model.Month.Month, day);
            var position = (int)model.Month.DayOfWeek + day - 1;
            var dayContent = new StackPanel { Spacing = 5, HorizontalAlignment = HorizontalAlignment.Center };
            dayContent.Children.Add(Design.Text(day.ToString(), 14));
            dayContent.Children.Add(new Microsoft.UI.Xaml.Shapes.Ellipse { Width = 4, Height = 4,
                Fill = Design.Brush("clay-strong"), Opacity = marked.Contains(date) ? 1 : 0 });
            var button = new Button { Content = dayContent, MinHeight = 48,
                HorizontalAlignment = HorizontalAlignment.Stretch, IsEnabled = model.PrivacyReady && !model.Busy };
            Design.QuietButton(button); button.MinHeight = 48;
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(button, date.ToString("yyyy-MM-dd") + (marked.Contains(date) ? "，有研究记录" : ""));
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
