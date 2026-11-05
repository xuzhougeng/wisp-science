using System.Globalization;
using System.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

/// Home calendar: projects, month and daily records share the WebView geometry.
internal sealed class NativeResearchCalendarPage : UserControl, IWorkspaceSheet
{
    private readonly WorkspaceCalendarModel model;
    private readonly WispDesign design;
    private readonly Action<string, DateTime> journey;
    private readonly Action close;
    private readonly Grid page = new(), layout = new(), workspace = new(), content = new(), monthBody = new(), details = new();
    private readonly NativeActionWrap projects = new() { Gap = 5 };
    private readonly Grid footer = new() { ColumnSpacing = 6, RowSpacing = 6 };
    private readonly ScrollViewer scroll, projectScroll, monthScroll, recordScroll;
    private readonly StackPanel notices = new() { Spacing = 10 }, records = new();
    private readonly Grid days = new(), week = new();
    private readonly Border monthCard, detailCard;
    private readonly TextBlock month, scope, day, detailMeta, totals, footerNote;
    private readonly Button previous, next, today, refresh;
    private readonly Dictionary<string, Button> projectButtons = new(StringComparer.Ordinal);
    private readonly Dictionary<DateTime, (Button Button, NativeActionWrap Marks)> dayButtons = new();
    private string projectSignature = "";
    private DateTime renderedMonth;
    private DateTime recordsDay;
    private string? recordsScope;
    private NativeCalendarLayout sizing;
    private bool disposed;

    public NativeResearchCalendarPage(WorkspaceCalendarModel model, WispDesign design, Action<string, DateTime> journey, Action close)
    {
        this.model = model; this.design = design; this.journey = journey; this.close = close;
        HorizontalContentAlignment = HorizontalAlignment.Stretch; VerticalContentAlignment = VerticalAlignment.Stretch;
        Background = design.Brush("bg-app"); design.BindTypography(this);
        AutomationProperties.SetAutomationId(this, "home-research-calendar");
        page.MaxWidth = 2560; page.HorizontalAlignment = HorizontalAlignment.Stretch;
        Rows(page, GridLength.Auto, GridLength.Auto, new(1, GridUnitType.Star));
        var back = Tool(L("返回首页", "Back to home"), "arrow-left", close, true);
        back.Padding = new Thickness(0, 6, 0, 6); back.HorizontalAlignment = HorizontalAlignment.Left; back.Margin = new Thickness(0, 0, 0, 16); page.Children.Add(back);
        var heading = new Grid { Margin = new Thickness(0, 0, 0, 22), ColumnSpacing = 12 };
        Columns(heading, new(1, GridUnitType.Star), GridLength.Auto);
        var headingText = new StackPanel { Spacing = 6 };
        headingText.Children.Add(Text(L("研究日历", "Research calendar"), 26, bold: true));
        headingText.Children.Add(Text(L("把每个项目的探索，放回同一条时间线。", "Recorded research across your projects, day by day."), 13, "text-muted"));
        heading.Children.Add(headingText);
        refresh = Tool(L("刷新日历", "Refresh calendar"), "refresh", () => _ = model.OpenAsync());
        refresh.VerticalAlignment = VerticalAlignment.Center; Put(heading, refresh, 0, 1); Put(page, heading, 1);
        layout.ColumnSpacing = 24; layout.RowSpacing = 16; Columns(layout, new(180), new(1, GridUnitType.Star)); Rows(layout, new(1, GridUnitType.Star), GridLength.Auto);
        projectScroll = Scroller(projects); layout.Children.Add(projectScroll); Put(layout, workspace, 0, 1); Put(page, layout, 2);
        Rows(workspace, GridLength.Auto, new(1, GridUnitType.Star), GridLength.Auto);
        var toolbar = new Grid { Margin = new Thickness(0, 0, 0, 16), ColumnSpacing = 10 }; Columns(toolbar, new(1, GridUnitType.Star), GridLength.Auto);
        var controls = new NativeActionWrap { Gap = 6 };
        month = Text("", 17, bold: true); month.VerticalAlignment = VerticalAlignment.Center; month.Margin = new Thickness(0, 0, 12, 0); controls.Children.Add(month);
        previous = Tool(L("上个月", "Previous month"), "chevron-left", () => _ = model.ShiftMonthAsync(-1)); controls.Children.Add(previous);
        next = Tool(L("下个月", "Next month"), "chevron-right", () => _ = model.ShiftMonthAsync(1)); controls.Children.Add(next);
        today = new Button { Content = Text(L("今天", "Today"), 13), Padding = new Thickness(12, 6, 12, 6), CornerRadius = new CornerRadius(6), Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border"), BorderThickness = new Thickness(1) };
        today.Click += async (_, _) => await model.TodayAsync(); controls.Children.Add(today); toolbar.Children.Add(controls);
        scope = Text("", 13, "text-muted"); scope.MaxWidth = 200; scope.VerticalAlignment = VerticalAlignment.Center; Put(toolbar, scope, 0, 1); workspace.Children.Add(toolbar);
        content.ColumnSpacing = content.RowSpacing = 20; Columns(content, new(1, GridUnitType.Star), new(360)); Rows(content, new(1, GridUnitType.Star), GridLength.Auto); Put(workspace, content, 1);
        Rows(monthBody, GridLength.Auto, GridLength.Auto, new(1, GridUnitType.Star)); monthBody.Children.Add(notices);
        for (var column = 0; column < 7; column++) { week.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) }); days.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) }); }
        var labels = design.Localization.Locale == "zh" ? new[] { "一", "二", "三", "四", "五", "六", "日" } : ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        for (var column = 0; column < 7; column++) { var label = Text(labels[column], 12, "text-muted"); label.HorizontalAlignment = HorizontalAlignment.Center; Put(week, label, 0, column); }
        week.Margin = new Thickness(0, 0, 0, 14); Put(monthBody, week, 1);
        Put(monthBody, new Border { Child = days, BorderBrush = design.Brush("border"), BorderThickness = new Thickness(1, 1, 0, 0) }, 2);
        monthScroll = Scroller(monthBody); monthCard = Card(monthScroll); content.Children.Add(monthCard);
        Rows(details, GridLength.Auto, GridLength.Auto, new(1, GridUnitType.Star));
        day = Text("", 17, bold: true); day.Margin = new Thickness(20, 20, 20, 6); details.Children.Add(day);
        detailMeta = Text("", 12, "text-muted"); detailMeta.Margin = new Thickness(20, 0, 20, 16); Put(details, detailMeta, 1);
        recordScroll = Scroller(records); Put(details, recordScroll, 2); detailCard = Card(details); Put(content, detailCard, 0, 1);
        Columns(footer, new(1, GridUnitType.Star), GridLength.Auto); Rows(footer, GridLength.Auto, GridLength.Auto);
        totals = Text("", 12, "text-muted"); footer.Children.Add(totals);
        footerNote = Text(L("仅展示主线已记录活动 · 日期按本地时区", "Recorded mainline activity · Local timezone"), 12, "text-muted"); Put(footer, footerNote, 0, 1);
        footer.Margin = new Thickness(0, 14, 0, 0); Put(workspace, footer, 2);
        scroll = Scroller(page); scroll.Background = design.Brush("bg-app"); Content = scroll;
        SizeChanged += (_, _) => Resize(); Loaded += (_, _) => Resize(); model.Changed += Render; Render(); _ = model.OpenAsync();
    }

    private string L(string zh, string en) => design.L(zh, en);
    private TextBlock Text(string value, double size = 14, string token = "text", bool bold = false)
    { var text = design.Text(value, size); text.FontWeight = bold ? Microsoft.UI.Text.FontWeights.SemiBold : Microsoft.UI.Text.FontWeights.Normal; text.Foreground = design.Brush(token); return text; }
    private Button Tool(string label, string icon, Action action, bool showLabel = false)
    { var button = design.ToolButton(label, icon, showLabel); button.Padding = new Thickness(7); button.MinWidth = button.MinHeight = 0; button.Click += (_, _) => action(); return button; }
    private static ScrollViewer Scroller(UIElement child) => new() { Content = child, HorizontalScrollMode = ScrollMode.Disabled, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
    private Border Card(UIElement child) => new() { Child = child, Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(10) };
    private static void Rows(Grid grid, params GridLength[] sizes) { foreach (var size in sizes) grid.RowDefinitions.Add(new() { Height = size }); }
    private static void Columns(Grid grid, params GridLength[] sizes) { foreach (var size in sizes) grid.ColumnDefinitions.Add(new() { Width = size }); }
    private static void Put(Grid parent, FrameworkElement child, int row = 0, int column = 0) { Grid.SetRow(child, row); Grid.SetColumn(child, column); parent.Children.Add(child); }
    private Brush Tint(double amount) { var color = design.Brush("clay").Color; var background = design.Brush("bg-elev").Color; byte Mix(byte a, byte b) => (byte)Math.Round(a * amount + b * (1 - amount)); return new SolidColorBrush(Windows.UI.Color.FromArgb(255, Mix(color.R, background.R), Mix(color.G, background.G), Mix(color.B, background.B))); }
    private Brush ProjectColor(string id)
    {
        var index = NativeCalendarPresentation.ColorIndex(id); if (index == 0) return design.Brush("clay");
        var colors = design.Dark ? new[] { "", "c3a4e8", "e0b56d", "80c5d0", "dfa0ac", "acc78e" } : ["", "8b6eb1", "b48339", "338896", "b16976", "618744"];
        var rgb = Convert.ToUInt32(colors[index], 16); return new SolidColorBrush(Windows.UI.Color.FromArgb(255, (byte)(rgb >> 16), (byte)(rgb >> 8), (byte)rgb));
    }
    private Ellipse Dot(string id) => new() { Width = 6, Height = 6, Fill = ProjectColor(id), VerticalAlignment = VerticalAlignment.Center };

    private void Resize()
    {
        if (ActualWidth <= 0 || disposed) return;
        sizing = NativeCalendarLayout.ForViewport(ActualWidth, ActualHeight);
        page.Margin = new Thickness(sizing.Inset, 24, sizing.Inset, 24); page.Height = sizing.Desktop ? Math.Min(1440, Math.Max(0, ActualHeight - 48)) : double.NaN;
        scroll.VerticalScrollMode = sizing.Desktop ? ScrollMode.Disabled : ScrollMode.Auto;
        scroll.VerticalScrollBarVisibility = sizing.Desktop ? ScrollBarVisibility.Disabled : ScrollBarVisibility.Auto;
        layout.ColumnDefinitions[0].Width = sizing.HorizontalProjects ? new(1, GridUnitType.Star) : new(sizing.NavigationWidth);
        layout.ColumnDefinitions[1].Width = sizing.HorizontalProjects ? new(0) : new(1, GridUnitType.Star);
        layout.ColumnSpacing = sizing.HorizontalProjects ? 0 : 24;
        layout.RowSpacing = sizing.HorizontalProjects ? 16 : 0;
        layout.RowDefinitions[0].Height = sizing.Desktop ? new(1, GridUnitType.Star) : GridLength.Auto;
        Grid.SetColumn(workspace, sizing.HorizontalProjects ? 0 : 1); Grid.SetRow(workspace, sizing.HorizontalProjects ? 1 : 0);
        workspace.RowDefinitions[1].Height = sizing.Desktop ? new(1, GridUnitType.Star) : GridLength.Auto;
        projectScroll.VerticalAlignment = sizing.Desktop ? VerticalAlignment.Stretch : VerticalAlignment.Top;
        projectScroll.VerticalScrollBarVisibility = sizing.Desktop ? ScrollBarVisibility.Auto : ScrollBarVisibility.Disabled;
        var sideBySide = sizing.DetailsWidth > 0;
        content.ColumnDefinitions[1].Width = sideBySide ? new(sizing.DetailsWidth) : new(0);
        content.ColumnSpacing = sideBySide ? 20 : 0;
        content.RowSpacing = sideBySide ? 0 : 20;
        content.RowDefinitions[0].Height = sizing.Desktop ? new(1, GridUnitType.Star) : GridLength.Auto;
        Grid.SetRow(detailCard, sideBySide ? 0 : 1); Grid.SetColumn(detailCard, sideBySide ? 1 : 0);
        foreach (var viewer in new[] { monthScroll, recordScroll }) viewer.VerticalScrollBarVisibility = sizing.Desktop ? ScrollBarVisibility.Auto : ScrollBarVisibility.Disabled;
        monthBody.Padding = sizing.HorizontalProjects ? new Thickness(7, 12, 7, 12) : new Thickness(18);
        foreach (var row in days.RowDefinitions) { row.MinHeight = sizing.HorizontalProjects ? 61 : 76; row.Height = sizing.Desktop ? new(1, GridUnitType.Star) : GridLength.Auto; }
        foreach (var visual in dayButtons.Values) visual.Button.Padding = sizing.HorizontalProjects ? new Thickness(2, 5, 2, 5) : new Thickness(8, 10, 8, 10);
        foreach (var button in projectButtons.Values) { button.Width = sizing.HorizontalProjects ? double.NaN : sizing.NavigationWidth - 4; button.MaxWidth = Math.Max(0, ActualWidth - sizing.Inset * 2); button.Padding = sizing.HorizontalProjects ? new Thickness(7) : new Thickness(10, 11, 10, 11); }
        scope.MaxWidth = Math.Max(80, Math.Min(200, ActualWidth * .2));
        var stackFooter = ActualWidth - sizing.Inset * 2 - (sizing.HorizontalProjects ? 0 : sizing.NavigationWidth + 24) < 680;
        footer.ColumnDefinitions[1].Width = stackFooter ? new(0) : GridLength.Auto;
        Grid.SetColumn(footerNote, stackFooter ? 0 : 1); Grid.SetRow(footerNote, stackFooter ? 1 : 0);
        footerNote.HorizontalAlignment = stackFooter ? HorizontalAlignment.Left : HorizontalAlignment.Right;
        footer.RowSpacing = stackFooter ? 6 : 0;
    }

    private void Render()
    {
        if (disposed) return;
        refresh.IsEnabled = !model.Busy;
        previous.IsEnabled = next.IsEnabled = today.IsEnabled = model.PrivacyReady;
        month.Text = design.Localization.Locale == "zh" ? $"{model.Month.Year}年 {model.Month.Month}月" : model.Month.ToString("yyyy / MM", CultureInfo.InvariantCulture);
        scope.Text = model.VisibleProjects.FirstOrDefault(p => p.Id == model.ProjectFilter)?.Name ?? L("所有项目", "All projects");
        var signature = string.Join("\n", model.VisibleProjects.Select(p => p.Id + "\0" + p.Name));
        if (signature != projectSignature || projectButtons.Count == 0)
        {
            projectSignature = signature; projects.Children.Clear(); projectButtons.Clear(); AddProject("", L("所有项目", "All projects"));
            foreach (var project in model.VisibleProjects) AddProject(project.Id, project.Name);
        }
        foreach (var (id, button) in projectButtons)
        {
            button.IsEnabled = model.PrivacyReady; var selected = id == (model.ProjectFilter ?? "");
            button.Background = selected ? Tint(.12) : new SolidColorBrush(Microsoft.UI.Colors.Transparent);
            AutomationProperties.SetItemStatus(button, selected ? L("已选中", "Selected") : "");
        }
        notices.Children.Clear(); notices.Margin = new Thickness(0);
        if (model.Busy) notices.Children.Add(Text(L("正在读取研究活动…", "Loading activity…"), 12, "text-muted"));
        else if (model.Error != null)
        {
            notices.Children.Add(Text(model.Error, 12, "err")); var retry = Link(L("重试", "Try again")); retry.Click += async (_, _) => await model.OpenAsync(); notices.Children.Add(retry);
        }
        else foreach (var row in model.Filter(model.MonthRows))
        {
            var projectName = model.VisibleProjects.First(p => p.Id == row.ProjectId).Name;
            if (row.Error != null) notices.Children.Add(Text(projectName + ": " + row.Error, 12, "err"));
            else if (row.History.Truncated) notices.Children.Add(Text(projectName + ": " + L("仅展示最近 2,000 条活动；请选择日期按日读取，未标记日期仍可能有活动。", "Showing the latest 2,000 events; select a date for a day-level read. Unmarked dates may have more activity."), 12, "text-muted"));
        }
        if (notices.Children.Count > 0) notices.Margin = new Thickness(0, 0, 0, 12);
        if (renderedMonth != model.Month) BuildMonth();
        var marked = model.Busy ? [] : model.Filter(model.MonthRows).Where(p => p.Error == null).ToArray();
        foreach (var (date, visual) in dayButtons)
        {
            visual.Button.IsEnabled = model.PrivacyReady;
            visual.Button.Background = date == model.Day ? Tint(.1) : new SolidColorBrush(Microsoft.UI.Colors.Transparent);
            visual.Button.BorderBrush = design.Brush(date == model.Day ? "clay" : "border");
            visual.Button.BorderThickness = date == model.Day ? new Thickness(1) : new Thickness(0, 0, 1, 1);
            AutomationProperties.SetItemStatus(visual.Button, date == model.Day ? L("已选中", "Selected") : "");
            visual.Marks.Children.Clear(); var names = new List<string>();
            foreach (var project in marked.Where(p => p.History.Entries.Any(e => NativeCalendarPresentation.LocalDay(e.OccurredAt) == date)))
            { visual.Marks.Children.Add(Dot(project.ProjectId)); names.Add(model.VisibleProjects.First(p => p.Id == project.ProjectId).Name); }
            AutomationProperties.SetHelpText(visual.Button, string.Join(", ", names)); ToolTipService.SetToolTip(visual.Button, names.Count == 0 ? null : string.Join(", ", names));
        }
        RenderRecords();
        var monthRows = model.Filter(model.MonthRows).ToArray(); var partial = monthRows.Any(p => p.Error != null || p.History.Truncated);
        var active = monthRows.Count(p => p.History.Entries.Count > 0); var activeDays = monthRows.SelectMany(p => p.History.Entries).Select(e => NativeCalendarPresentation.LocalDay(e.OccurredAt)).Distinct().Count();
        totals.Text = model.Busy || model.Error != null ? "" : L($"{(partial ? "本月已读取" : "本月")} · {active} 个活跃项目 · {activeDays} 个活动日", $"{(partial ? "Loaded this month" : "This month")} · {active} active projects · {activeDays} active days");
        Resize();
    }
    private void AddProject(string id, string title)
    {
        var row = new Grid { ColumnSpacing = 9 }; Columns(row, GridLength.Auto, new(1, GridUnitType.Star));
        row.Children.Add(id.Length == 0 ? design.Icon("folder", 17) : Dot(id)); Put(row, Text(title), 0, 1);
        var button = new Button { Content = row, HorizontalContentAlignment = HorizontalAlignment.Stretch, CornerRadius = new CornerRadius(7) }; design.QuietButton(button);
        AutomationProperties.SetName(button, title); button.Click += (_, _) => { model.ProjectFilter = id.Length == 0 ? null : id; Render(); }; projectButtons[id] = button; projects.Children.Add(button);
    }
    private void BuildMonth()
    {
        renderedMonth = model.Month; days.Children.Clear(); days.RowDefinitions.Clear(); dayButtons.Clear(); var cells = NativeCalendarPresentation.Cells(model.Month);
        for (var row = 0; row < cells.Count / 7; row++) days.RowDefinitions.Add(new() { Height = new(1, GridUnitType.Star), MinHeight = 76 });
        for (var index = 0; index < cells.Count; index++)
        {
            if (cells[index] is not { } date) { Put(days, new Border { BorderBrush = design.Brush("border"), BorderThickness = new Thickness(0, 0, 1, 1) }, index / 7, index % 7); continue; }
            var body = new StackPanel { Spacing = 10 }; var number = Text(date.Day.ToString(CultureInfo.InvariantCulture), 14); number.VerticalAlignment = VerticalAlignment.Center; number.HorizontalAlignment = HorizontalAlignment.Center;
            if (date == DateTime.Today) number.Foreground = new SolidColorBrush(Microsoft.UI.Colors.White);
            body.Children.Add(new Border { Child = number, Width = 28, Height = 28, HorizontalAlignment = HorizontalAlignment.Left, CornerRadius = new CornerRadius(14), Background = date == DateTime.Today ? design.Brush("clay") : null });
            var marks = new NativeActionWrap { Gap = 5, Margin = new Thickness(7, 0, 0, 0) }; body.Children.Add(marks);
            var button = new Button { Content = body, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalAlignment = VerticalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Top, CornerRadius = new CornerRadius(0), MinHeight = 0, MinWidth = 0 };
            button.Resources["ButtonBackgroundPointerOver"] = design.Brush("bg-app"); button.Resources["ButtonBackgroundPressed"] = Tint(.1);
            AutomationProperties.SetName(button, date.ToString("yyyy-MM-dd", CultureInfo.InvariantCulture)); button.Click += async (_, _) => await model.SelectDayAsync(date);
            Put(days, button, index / 7, index % 7); dayButtons[date] = (button, marks);
        }
    }
    private Button Link(string title)
    { var button = new Button { Content = Text(title, 12, "clay-strong"), Padding = new Thickness(0, 6, 0, 6), HorizontalAlignment = HorizontalAlignment.Left }; design.QuietButton(button); return button; }
    // Daily records are built separately so month controls and keyboard focus survive reads.
    private void RenderRecords()
    {
        if (recordsDay != model.Day || recordsScope != model.ProjectFilter)
        { recordsDay = model.Day; recordsScope = model.ProjectFilter; recordScroll.ChangeView(null, 0, null, true); }
        day.Text = model.Day.ToString("yyyy-MM-dd", CultureInfo.InvariantCulture) + (model.Day == DateTime.Today ? L(" · 今天", " · Today") : "");
        records.Children.Clear(); records.Padding = new Thickness(20, 0, 20, 18); detailMeta.Visibility = Visibility.Collapsed;
        void Empty(string text, bool failure = false) { var label = Text(text, 13, failure ? "err" : "text-muted"); label.Margin = new Thickness(0, 20, 0, 0); records.Children.Add(label); }
        if (model.Busy) { Empty(L("正在读取当天记录…", "Loading records…")); return; }
        if (model.Error != null) { Empty(model.Error, true); return; }
        var groups = model.Filter(model.DayRows).Select(p => (Project: p, Folded: NativeCalendarPresentation.FoldOutputs(p.History.Entries))).Where(p => p.Project.Error != null || p.Folded.Entries.Count > 0).ToArray();
        if (groups.Length == 0) { Empty(model.VisibleProjects.Count == 0 ? L("创建项目后，已记录的研究活动会出现在这里。", "Create a project to begin recording research activity.") : L("当天没有已记录的研究活动。", "No recorded activity on this date.")); return; }
        var active = groups.Count(g => g.Folded.Entries.Count > 0); var count = groups.Sum(g => g.Folded.Entries.Count); var partial = groups.Any(g => g.Project.Error != null || g.Project.History.Truncated);
        detailMeta.Text = (partial ? L("已读取：", "Loaded: ") : "") + L($"{active} 个项目 · {count} 条记录", $"{active} projects · {count} records"); detailMeta.Visibility = Visibility.Visible;
        foreach (var group in groups) AddRecords(group.Project, group.Folded.Entries, group.Folded.Outputs);
    }
    private void AddRecords(CalendarProject project, IReadOnlyList<CalendarEntry> entries, IReadOnlyDictionary<string, int> outputs)
    {
        var group = new StackPanel(); var header = new Grid { ColumnSpacing = 8, Padding = new Thickness(0, 14, 0, 10) }; Columns(header, new(1, GridUnitType.Star), GridLength.Auto);
        var title = model.VisibleProjects.First(p => p.Id == project.ProjectId).Name;
        var label = new Grid { ColumnSpacing = 8 }; Columns(label, GridLength.Auto, GridLength.Auto, new(1, GridUnitType.Star), GridLength.Auto);
        var arrow = design.Icon("chevron-down", 14); label.Children.Add(arrow); Put(label, Dot(project.ProjectId), 0, 1); Put(label, Text(title, 14, bold: true), 0, 2); Put(label, Text(L($"{entries.Count} 条", $"{entries.Count} records"), 12, "text-muted"), 0, 3);
        var toggle = new Button { Content = label, Padding = new Thickness(0, 5, 0, 5), HorizontalContentAlignment = HorizontalAlignment.Stretch }; design.QuietButton(toggle); AutomationProperties.SetName(toggle, title); header.Children.Add(toggle);
        var selectedDay = model.Day; Put(header, Tool(title + " · " + L("研究历程", "Research journey"), "external-link", () => journey(project.ProjectId, selectedDay)), 0, 1); group.Children.Add(header);
        if (project.Error != null) group.Children.Add(Text(project.Error, 12, "err"));
        if (project.History.Truncated) group.Children.Add(Text(L("当前展示当天最近 2,000 条活动，还有更多记录。", "Latest 2,000 events shown; more records exist on this day."), 12, "text-muted"));
        var body = new StackPanel(); group.Children.Add(body);
        toggle.Click += (_, _) => { var collapsed = body.Visibility == Visibility.Visible; body.Visibility = collapsed ? Visibility.Collapsed : Visibility.Visible; arrow.Source = design.Icon(collapsed ? "chevron-right" : "chevron-down", 14).Source; AutomationProperties.SetItemStatus(toggle, collapsed ? L("已折叠", "Collapsed") : L("已展开", "Expanded")); };
        if (project.History.Recaps.FirstOrDefault(r => r.Status != "dismissed") is { } recap)
        {
            var text = new StackPanel { Spacing = 6 }; text.Children.Add(Text(recap.Headline, 12, bold: true)); text.Children.Add(Text(recap.Status == "confirmed" ? L("回顾 · 已确认", "Recap · confirmed") : L("回顾 · AI 草稿", "Recap · AI draft"), 12, "text-faint"));
            foreach (var item in recap.Done) text.Children.Add(Text(item.Text, 12, "text-muted")); var panel = Card(text); panel.Padding = new Thickness(12, 10, 12, 10); panel.Margin = new Thickness(22, 8, 0, 4); body.Children.Add(panel);
        }
        var list = new StackPanel(); body.Children.Add(list); var actions = new NativeActionWrap { Gap = 18, Margin = new Thickness(22, 4, 0, 0) }; body.Children.Add(actions);
        var more = Link(""); var fewer = Link(L("收起更多记录", "Show fewer records")); actions.Children.Add(more); actions.Children.Add(fewer); var limit = NativeCalendarPresentation.PageSize;
        void Update()
        {
            while (list.Children.Count > limit) list.Children.RemoveAt(list.Children.Count - 1);
            while (list.Children.Count < Math.Min(limit, entries.Count)) list.Children.Add(Record(entries[list.Children.Count], outputs));
            ((TextBlock)more.Content).Text = L($"显示更早记录 ({Math.Max(0, entries.Count - limit)})", $"Show earlier records ({Math.Max(0, entries.Count - limit)})");
            more.Visibility = entries.Count > limit ? Visibility.Visible : Visibility.Collapsed; fewer.Visibility = limit > NativeCalendarPresentation.PageSize ? Visibility.Visible : Visibility.Collapsed;
        }
        more.Click += (_, _) => { limit = Math.Min(entries.Count, limit + NativeCalendarPresentation.PageSize); Update(); if (more.Visibility == Visibility.Collapsed) fewer.Focus(FocusState.Programmatic); };
        fewer.Click += (_, _) => { limit = NativeCalendarPresentation.PageSize; Update(); more.Focus(FocusState.Programmatic); }; Update();
        records.Children.Add(new Border { Child = group, BorderBrush = design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0), Margin = new Thickness(0, records.Children.Count == 0 ? 0 : 12, 0, 0) });
    }
    private UIElement Record(CalendarEntry entry, IReadOnlyDictionary<string, int> outputs)
    {
        var body = new StackPanel { Spacing = 5 }; var meta = new NativeActionWrap { Gap = 10 };
        meta.Children.Add(Text(DateTimeOffset.FromUnixTimeSeconds(entry.OccurredAt).LocalDateTime.ToString("HH:mm", CultureInfo.InvariantCulture), 12, "text-muted"));
        var category = entry.Kind switch { "progress" => L("进展", "Progress"), "finding" => L("发现", "Finding"), "decision" => L("决策", "Decision"), "next" => L("待继续", "Next step"), "run" => L("实验", "Experiment"), "session" => L("会话", "Conversation"), "archive" => L("研究归档", "Research archive"), "paper" => L("文献", "Paper"), "data_asset" => L("数据", "Data"), _ => L("产出", "Output") };
        if (entry.Kind == "run")
        {
            category += " · " + (entry.Status switch { "submitted" => L("已提交", "Submitted"), "started" => L("开始", "Started"), "succeeded" => L("已完成", "Completed"), "failed" => L("失败", "Failed"), "cancelled" => L("已取消", "Cancelled"), "lost" => L("失去连接", "Connection lost"), "running" => L("运行中", "Running"), "queued" => L("排队中", "Queued"), _ => entry.Status });
            if (outputs.TryGetValue(entry.SourceId, out var count)) category += L($" · {count} 份产出", $" · {count} outputs");
        }
        if (entry.Manual) category += L(" · 手动", " · Manual"); meta.Children.Add(Text(category, 12, entry.Status is "failed" or "lost" ? "err" : "text-muted")); body.Children.Add(meta);
        var title = Text(entry.Title + (entry.VersionNumber is { } version ? $" · v{version}" : ""), 14); title.LineHeight = design.FontSize(23.1); body.Children.Add(title);
        if (title.Text.EnumerateRunes().Count() > 64)
        {
            title.MaxLines = 2; title.TextTrimming = TextTrimming.CharacterEllipsis; var expand = Link(L("展开全文", "Show full text"));
            expand.Click += (_, _) => { var opening = title.MaxLines != 0; title.MaxLines = opening ? 0 : 2; ((TextBlock)expand.Content).Text = opening ? L("收起全文", "Collapse text") : L("展开全文", "Show full text"); }; body.Children.Add(expand);
        }
        return new Border { Child = body, Padding = new Thickness(22, 11, 0, 12), BorderBrush = design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, 1) };
    }
    public void HandleEscape() => close();
    public void Dispose() { disposed = true; model.Changed -= Render; model.Dispose(); }
}
