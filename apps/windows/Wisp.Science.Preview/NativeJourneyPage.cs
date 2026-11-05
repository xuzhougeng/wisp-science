using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeJourneyPage : UserControl, IWorkspaceSheet
{
    private readonly WorkspaceJourneyModel model;
    private readonly JourneyThumbnailLoader thumbnails;
    private readonly List<NativeJourneyThumbnail> outputPreviews = [];
    private readonly Dictionary<string, Button> outputCards = [];
    private readonly WispDesign design;
    private readonly Action close;
    private readonly Action<string>? openSession;
    private readonly Action<FlyoutBase> register;
    private readonly Action? presentationChanged;
    private readonly Grid layers = new(), page = new(), body = new(), toolbar = new(), heading = new();
    private readonly ContentControl pageControl = new() { HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch };
    private readonly StackPanel header = new() { Spacing = 16 }, feed = new() { Spacing = 24 }, calendar = new() { Spacing = 12 }, details = new() { Spacing = 12 }, aside = new() { Spacing = 22 };
    private readonly ScrollViewer feedScroll, asideScroll, stackScroll;
    private readonly Grid graphHost = new();
    private readonly TextBox search;
    private readonly TextBlock headingTitle, headingHelp;
    private readonly Button byDay, relationships;
    private readonly Border searchBox;
    private readonly HashSet<DateOnly> expanded = [], expandedOutputs = [];
    private JourneyPage? rendered;
    private string? renderedQuery;
    private bool renderedBusy;
    private string? renderedError;
    private bool graphTab, graphCanvas, disposed, firstDays = true, rearranging;
    private NativeJourneyLayout layout;
    private double workspaceInset;
    public string ProjectId { get; }
    public bool KeepsProjectSidebar { get; }
    public bool IsModal => model.NoteOpen || model.Run != null || artifactOpen;

    public NativeJourneyPage(INativeJourneyClient client, string projectId, string projectName, WispDesign design, DateTime? day, Action close,
        Action<string>? openSession, Action<FlyoutBase> register, bool keepsProjectSidebar, Action? presentationChanged = null)
    {
        model = new(client, projectId); thumbnails = new(client, projectId); this.design = design; this.close = close; this.openSession = openSession; this.register = register;
        ProjectId = projectId; KeepsProjectSidebar = keepsProjectSidebar; this.presentationChanged = presentationChanged;
        HorizontalContentAlignment = HorizontalAlignment.Stretch; VerticalContentAlignment = VerticalAlignment.Stretch;
        AutomationProperties.SetAutomationId(this, "research-journey"); design.BindTypography(this);
        page.Background = design.Brush("bg-app"); Rows(page, GridLength.Auto, new(1, GridUnitType.Star));
        var crumb = Text(projectName + "   /   " + L("研究历程", "Research journey"), 12, "text-faint"); crumb.TextWrapping = TextWrapping.NoWrap; crumb.TextTrimming = TextTrimming.CharacterEllipsis; header.Children.Add(crumb);
        Columns(heading, new(1, GridUnitType.Star), GridLength.Auto); heading.ColumnSpacing = 20;
        var labels = new StackPanel { Spacing = 8 }; headingTitle = Serif(L("研究历程", "Research journey"), 30); headingHelp = Text(L("每天的探索、产出与关键决定，都有迹可循。", "Your daily exploration, outputs and decisions — with their sources."), 14, "text-muted"); labels.Children.Add(headingTitle); labels.Children.Add(headingHelp); heading.Children.Add(labels);
        var actions = new NativeActionWrap { Gap = 8, RightAligned = true };
        actions.Children.Add(Action(L("补充记录", "Add entry"), model.OpenNote, "note-plus", true)); actions.Children.Add(Icon(L("关闭研究历程", "Close research journey"), "close", close)); Put(heading, actions, 0, 1); header.Children.Add(heading);
        Columns(toolbar, new(1, GridUnitType.Star), new(340), GridLength.Auto); Rows(toolbar, GridLength.Auto, GridLength.Auto); toolbar.ColumnSpacing = 12;
        var tabs = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        byDay = Tab(L("按日", "By day"), false); relationships = Tab(L("关系图", "Relationships"), true); tabs.Children.Add(byDay); tabs.Children.Add(relationships); toolbar.Children.Add(tabs);
        search = new TextBox { PlaceholderText = L("搜索本月研究记录", "Search this month's records"), MinWidth = 0, MinHeight = 0, BorderThickness = new(0), Padding = new(0), Background = null, Foreground = design.Brush("text") };
        design.BindTypography(search, size: 14); AutomationProperties.SetName(search, L("搜索研究记录", "Search research records")); TextMenu(search);
        foreach (var key in new[] { "TextControlBackgroundFocused", "TextControlBackgroundPointerOver" }) search.Resources[key] = design.Brush("bg-input");
        search.TextChanged += (_, _) => { model.Query = search.Text; RenderFeed(); };
        var searchRow = new Grid { ColumnSpacing = 10 }; Columns(searchRow, GridLength.Auto, new(1, GridUnitType.Star)); searchRow.Children.Add(design.Icon("search", 18, design.ColorText("text-faint"))); Put(searchRow, search, 0, 1);
        searchBox = new Border { Child = searchRow, Background = design.Brush("bg-input"), BorderBrush = design.Brush("border-strong"), BorderThickness = new(1), CornerRadius = new(8), Padding = new(12, 7, 12, 7), Margin = new(0, 0, 0, 6) }; Put(toolbar, searchBox, 0, 1);
        Put(toolbar, Icon(L("刷新研究记录", "Refresh research records"), "refresh", () => { _ = model.ReloadAsync(); _ = model.LoadGraphAsync(); }), 0, 2);
        header.Children.Add(new Border { Child = toolbar, BorderBrush = design.Brush("border"), BorderThickness = new(0, 0, 0, 1) }); page.Children.Add(header);
        feedScroll = Scroll(feed); asideScroll = Scroll(aside); stackScroll = Scroll(new StackPanel());
        Put(page, body, 1, 0); Put(page, graphHost, 1, 0); graphHost.Visibility = Visibility.Collapsed;
        pageControl.Content = page; layers.Children.Add(pageControl); Content = layers;
        SizeChanged += (_, _) => Resize(); Loaded += (_, _) => Resize();
        model.Changed += Refresh; Refresh(); _ = model.OpenAsync(day); _ = model.LoadGraphAsync();
    }
    public void SetWorkspaceInset(double value) { workspaceInset = value; Resize(); }
    private string L(string zh, string en) => design.L(zh, en);
    private bool Chinese => L("中", "en") == "中";
    private string Kind(string value) => NativeJourneyPresentation.Label(value, Chinese);
    private TextBlock Text(string value, double size = 14, string color = "text", bool bold = false)
    { var text = design.Text(value, size); text.Foreground = design.Brush(color); text.TextWrapping = TextWrapping.Wrap; if (bold) text.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; return text; }
    private TextBlock Serif(string value, double size) { var text = Text(value, size, bold: true); text.FontFamily = new("ms-appx:///Assets/Fonts/wisp-serif.ttf#Wisp Serif, Noto Serif SC, SimSun, Georgia, Times New Roman"); return text; }
    private Button Action(string label, Action action, string? icon = null, bool primary = false)
    {
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, VerticalAlignment = VerticalAlignment.Center };
        if (icon != null) row.Children.Add(design.Icon(icon, 16, primary ? "#ffffff" : design.ColorText("clay-strong")));
        row.Children.Add(Text(label, 14, primary ? "on-clay" : "text"));
        var button = new Button { Content = row, HorizontalContentAlignment = HorizontalAlignment.Center }; design.ActionButton(button, primary);
        if (primary)
        {
            button.Resources["ButtonBackgroundDisabled"] = design.Brush("clay");
            button.Resources["ButtonBorderBrushDisabled"] = design.Brush("clay");
            button.IsEnabledChanged += (_, _) => button.Opacity = button.IsEnabled ? 1 : .48;
        }
        button.Click += (_, _) => action(); AutomationProperties.SetName(button, label); return button;
    }
    private Button Icon(string label, string icon, Action action)
    { var button = new Button { Content = design.Icon(icon, 18, design.ColorText("text-muted")), Width = 32, Height = 32 }; design.QuietButton(button); button.Padding = new(0); button.Click += (_, _) => action(); AutomationProperties.SetName(button, label); ToolTipService.SetToolTip(button, label); return button; }
    private Button Link(string label, Action action, double size = 12)
    { var button = new Button { Content = Text(label, size, "clay-strong"), HorizontalContentAlignment = HorizontalAlignment.Left }; design.QuietButton(button); button.Padding = new(0, 3, 0, 3); button.Click += (_, _) => action(); AutomationProperties.SetName(button, label); return button; }
    private Button Tab(string label, bool graph)
    { var button = Link(label, () => { graphTab = graph; Refresh(); }, 14); button.Padding = new(16, 12, 16, 12); return button; }
    private static ScrollViewer Scroll(UIElement content) => new() { Content = content, HorizontalContentAlignment = HorizontalAlignment.Stretch, HorizontalScrollMode = ScrollMode.Disabled, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
    private static void Columns(Grid grid, params GridLength[] widths) { foreach (var width in widths) grid.ColumnDefinitions.Add(new() { Width = width }); }
    private static void Rows(Grid grid, params GridLength[] heights) { foreach (var height in heights) grid.RowDefinitions.Add(new() { Height = height }); }
    private static void Put(Grid grid, FrameworkElement element, int row, int column) { Grid.SetRow(element, row); Grid.SetColumn(element, column); grid.Children.Add(element); }
    private void TextMenu(TextBox box) { var flyout = new TextCommandBarFlyout(); box.ContextFlyout = flyout; register(flyout); }
    private void Resize()
    {
        if (rearranging || ActualWidth <= 0) return;
        rearranging = true;
        try
        {
            var previous = layout; layout = NativeJourneyLayout.ForWidth(ActualWidth);
            page.Margin = new(layout.Narrow ? 0 : workspaceInset, 0, 0, 0); header.Padding = new(layout.Inset, layout.Narrow ? 16 : 24, layout.Inset, 0);
            headingTitle.FontSize = layout.Narrow ? 24 : 30; headingHelp.FontSize = layout.Narrow ? 12 : 14;
            body.Padding = new(layout.Narrow ? 12 : layout.Inset, layout.Narrow ? 16 : 24, layout.Narrow ? 12 : layout.Inset, 0);
            graphHost.Margin = new(layout.Inset, 16, layout.Inset, 20);
            toolbar.ColumnDefinitions[1].Width = new(layout.Narrow ? 1 : Math.Min(340, Math.Max(140, (ActualWidth - workspaceInset) * .36)), layout.Narrow ? GridUnitType.Star : GridUnitType.Pixel);
            Grid.SetRow(searchBox, layout.Narrow ? 1 : 0); Grid.SetColumn(searchBox, layout.Narrow ? 0 : 1); Grid.SetColumnSpan(searchBox, layout.Narrow ? 2 : 1);
            if (previous.Stacked != layout.Stacked || previous.Narrow != layout.Narrow || body.Children.Count == 0) ArrangeBody();
            if (body.ColumnDefinitions.Count > 1) body.ColumnDefinitions[1].Width = new(layout.AsideWidth);
            if (previous.DateWidth != layout.DateWidth) { rendered = null; RenderFeed(); }
            ResizeDialog();
        }
        finally { rearranging = false; }
    }
    private void ArrangeBody()
    {
        body.Children.Clear(); aside.Children.Clear();
        if (stackScroll.Content is Panel old) { foreach (var nested in old.Children.OfType<Grid>()) nested.Children.Clear(); old.Children.Clear(); }
        if (asideScroll.Parent is Border oldBorder) oldBorder.Child = null;
        feedScroll.Content = null; asideScroll.Content = null; body.ColumnDefinitions.Clear();
        calendar.MaxWidth = layout.Stacked ? layout.Narrow ? 280 : 224 : double.PositiveInfinity; calendar.HorizontalAlignment = HorizontalAlignment.Stretch;
        if (layout.Stacked)
        {
            var compactAside = new Grid { ColumnSpacing = 24, RowSpacing = 24 };
            if (layout.Narrow) { Rows(compactAside, GridLength.Auto, GridLength.Auto); compactAside.Children.Add(calendar); Put(compactAside, details, 1, 0); }
            else { Columns(compactAside, new(224), new(1, GridUnitType.Star)); compactAside.Children.Add(calendar); Put(compactAside, details, 0, 1); }
            var stack = new StackPanel { Spacing = 24 }; stack.Children.Add(compactAside); stack.Children.Add(feed); stackScroll.Content = stack; body.Children.Add(stackScroll);
            details.Padding = new(0);
        }
        else
        {
            Columns(body, new(1, GridUnitType.Star), new(layout.AsideWidth)); body.ColumnSpacing = ActualWidth <= 1200 ? 16 : 24;
            aside.Padding = new(ActualWidth <= 1200 ? 16 : 24, 0, 0, 24); aside.Children.Add(calendar); aside.Children.Add(details);
            details.Padding = new(0); feedScroll.Content = feed; asideScroll.Content = aside; body.Children.Add(feedScroll);
            Put(body, new Border { Child = asideScroll, BorderBrush = design.Brush("border"), BorderThickness = new(1, 0, 0, 0) }, 0, 1);
        }
    }
    private void Refresh()
    {
        if (disposed) return;
        body.Visibility = graphTab ? Visibility.Collapsed : Visibility.Visible; graphHost.Visibility = graphTab ? Visibility.Visible : Visibility.Collapsed;
        foreach (var (tab, active) in new[] { (byDay, !graphTab), (relationships, graphTab) })
        { tab.BorderThickness = new(0, 0, 0, 2); tab.BorderBrush = active ? design.Brush("clay") : null; if (tab.Content is TextBlock label) { label.Foreground = design.Brush(active ? "clay-strong" : "text-muted"); label.FontWeight = active ? Microsoft.UI.Text.FontWeights.SemiBold : Microsoft.UI.Text.FontWeights.Normal; } }
        if (search.Text != model.Query) search.Text = model.Query;
        RenderCalendar(); RenderFeed(); RenderDetails(); RenderGraph(); RenderDialog();
        foreach (var (id, card) in outputCards) card.BorderBrush = design.Brush(model.Selected?.Id == id ? "clay" : "border");
    }
    private void RenderCalendar()
    {
        calendar.Children.Clear(); var head = new Grid(); Columns(head, new(1, GridUnitType.Star), GridLength.Auto, GridLength.Auto);
        head.Children.Add(Text(Chinese ? $"{model.Month:yyyy} 年 {model.Month.Month} 月" : $"{model.Month:yyyy / MM}", 16, bold: true));
        Put(head, Icon(L("上个月", "Previous month"), "chevron-left", () => { firstDays = true; _ = model.ShiftMonthAsync(-1); }), 0, 1);
        Put(head, Icon(L("下个月", "Next month"), "chevron-right", () => { firstDays = true; _ = model.ShiftMonthAsync(1); }), 0, 2); calendar.Children.Add(head);
        var days = new Grid { RowSpacing = 5, ColumnSpacing = 1 }; for (var i = 0; i < 7; i++) days.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) });
        for (var i = 0; i < 7; i++) days.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var names = Chinese ? new[] { "一", "二", "三", "四", "五", "六", "日" } : new[] { "M", "T", "W", "T", "F", "S", "S" };
        for (var i = 0; i < 7; i++) { var label = Text(names[i], 12, "text-faint"); label.Height = 30; label.TextAlignment = TextAlignment.Center; Put(days, label, 0, i); }
        var offset = ((int)model.Month.DayOfWeek + 6) % 7;
        for (var day = 1; day <= DateTime.DaysInMonth(model.Month.Year, model.Month.Month); day++)
        {
            var date = model.Month.AddDays(day - 1); var active = date.Date == model.SelectedDay.Date; var today = date == DateTime.Today;
            var content = new Grid(); var dayLabel = Text(day.ToString(), 12, active ? "on-clay" : today ? "clay-strong" : "text-muted", active || today);
            dayLabel.TextWrapping = TextWrapping.NoWrap; dayLabel.HorizontalAlignment = HorizontalAlignment.Center; dayLabel.VerticalAlignment = VerticalAlignment.Center; content.Children.Add(dayLabel);
            if (model.MarkedDays.Contains(DateOnly.FromDateTime(date))) content.Children.Add(new Ellipse { Width = 4, Height = 4, Fill = design.Brush(active ? "on-clay" : "clay"), VerticalAlignment = VerticalAlignment.Bottom, HorizontalAlignment = HorizontalAlignment.Center, Margin = new(0, 0, 0, -4) });
            var button = new Button { Content = content, Width = 30, Height = 30, HorizontalAlignment = HorizontalAlignment.Center }; design.QuietButton(button); button.Padding = new(0); button.CornerRadius = new(15); button.Background = active ? design.Brush("clay") : null;
            AutomationProperties.SetName(button, date.ToString("yyyy-MM-dd")); button.Click += (_, _) => { firstDays = true; _ = model.SelectDayAsync(date); }; Put(days, button, (day + offset - 1) / 7 + 1, (day + offset - 1) % 7);
        }
        calendar.Children.Add(days); calendar.Children.Add(Link(L("回到今天", "Back to today"), () => { firstDays = true; _ = model.TodayAsync(); }));
        if (model.FocusDay != null) calendar.Children.Add(Link(L("显示整月", "Show full month"), () => { firstDays = true; _ = model.FullMonthAsync(); }));
        calendar.Children.Add(Text(L("日期按本机时区显示", "Dates follow this device’s timezone"), 10, "text-faint"));
    }
    private void RenderFeed()
    {
        if (disposed) return;
        if (rendered == model.Page && renderedQuery == model.Query && renderedBusy == model.Busy && renderedError == model.Error) return;
        rendered = model.Page; renderedQuery = model.Query; renderedBusy = model.Busy; renderedError = model.Error;
        foreach (var thumbnail in outputPreviews) thumbnail.Dispose(); outputPreviews.Clear(); outputCards.Clear(); feed.Children.Clear();
        if (model.Busy) { feed.Children.Add(Empty(L("正在读取研究记录…", "Loading research records…"), "")); return; }
        if (model.Error != null) { feed.Children.Add(Text(L("研究记录加载失败：", "Could not load research records: ") + model.Error, 13, "err")); feed.Children.Add(Link(L("重试", "Try again"), () => _ = model.ReloadAsync())); if (model.Page == null) return; }
        var days = model.Days();
        if (days.Length == 0) { feed.Children.Add(Empty(L("当前范围没有研究记录", "No records in this view"), L("可以切换月份、清空搜索或补充记录。会话、实验和已登记的产出会自动出现在这里。", "Choose another month, clear the search, or add a research note. Conversations, runs and registered outputs appear here automatically."))); return; }
        if (firstDays) { expanded.Clear(); expanded.Add(days[0].Day); firstDays = false; }
        if (model.Page?.Truncated == true) feed.Children.Add(Text(L("当前展示最近 2,000 条活动，请按日查看以缩小范围。", "Showing the latest 2,000 events. Use the date picker to load one day."), 12, "text-muted"));
        foreach (var day in days) feed.Children.Add(DayCard(day));
    }
    private FrameworkElement Empty(string title, string help)
    {
        var box = new StackPanel { Padding = new(24, 60, 24, 60), MaxWidth = 620, HorizontalAlignment = HorizontalAlignment.Stretch, Spacing = 16 };
        var text = Serif(title, 22); text.TextAlignment = TextAlignment.Center; box.Children.Add(text);
        var subtitle = Text(help, 14, "text-muted"); subtitle.TextAlignment = TextAlignment.Center; box.Children.Add(subtitle); return box;
    }
    private FrameworkElement DayCard(JourneyDay day)
    {
        var grid = new Grid(); Columns(grid, new(layout.DateWidth), new(1, GridUnitType.Star));
        var date = day.Day.ToDateTime(TimeOnly.MinValue); var dateColumn = new StackPanel { Spacing = 3, Padding = new(0, 3, 12, 0) };
        dateColumn.Children.Add(Text(Chinese ? $"{date.Month} 月 {date.Day} 日" : date.ToString("MM/dd"), layout.Narrow ? 12 : 16, bold: true));
        var week = date.ToString("ddd", System.Globalization.CultureInfo.GetCultureInfo(Chinese ? "zh-CN" : "en-US")); dateColumn.Children.Add(Text(week + (date == DateTime.Today ? L(" · 今天", " · Today") : ""), 12, "text-faint")); grid.Children.Add(dateColumn);
        var main = new StackPanel { Spacing = 12, Padding = new(layout.Narrow ? 16 : 24, 0, 0, 0) };
        var headline = NativeJourneyPresentation.Headline(day); var recap = model.Page?.Recaps.FirstOrDefault(r => DateOnly.FromDateTime(DateTimeOffset.FromUnixTimeSeconds(r.DayStart).LocalDateTime) == day.Day);
        var head = new Grid { ColumnSpacing = 8 }; Columns(head, new(1, GridUnitType.Star), GridLength.Auto);
        var title = new Button { Content = Text(recap is { Status: not "dismissed" } ? recap.Headline : headline.Title, 20, bold: true), HorizontalContentAlignment = HorizontalAlignment.Stretch, HorizontalAlignment = HorizontalAlignment.Stretch }; design.QuietButton(title); title.Click += (_, _) => _ = model.SelectAsync(headline); head.Children.Add(title);
        var isOpen = expanded.Contains(day.Day); Put(head, Icon(isOpen ? L("收起记录", "Collapse day") : L("展开记录", "Expand day"), isOpen ? "chevron-down" : "chevron-right", () => { if (!expanded.Add(day.Day)) expanded.Remove(day.Day); rendered = null; RenderFeed(); }), 0, 1); main.Children.Add(head);
        if ((recap == null || recap.Status == "dismissed") && headline.Summary.Length > 0) main.Children.Add(Text(headline.Summary, 14, "text-muted"));
        if (recap is { Status: not "dismissed" }) main.Children.Add(RecapCard(recap));
        var outputs = day.Entries.Where(e => e.Kind == "artifact").ToArray(); var sessions = day.Entries.Where(e => e.Kind == "session").ToArray(); var notes = NativeJourneyPresentation.Notes(day).ToArray();
        main.Children.Add(Text(L($"{NativeJourneyPresentation.Experiments(day)} 次实验 · {outputs.Length} 份产出 · {notes.Length} 条研究记录 · {sessions.Length} 个会话", $"{NativeJourneyPresentation.Experiments(day)} experiments · {outputs.Length} outputs · {notes.Length} notes · {sessions.Length} conversations"), 12, "text-faint"));
        if (isOpen)
        {
            foreach (var entry in day.Entries.Where(e => e.Kind == "run")) main.Children.Add(EntryRow(entry, true));
            if (outputs.Length > 0)
            {
                main.Children.Add(Text(L("产出", "Outputs"), 16, bold: true));
                var outputGrid = new Grid { ColumnSpacing = 10, RowSpacing = 10 }; var columns = layout.Narrow ? 2 : 3; for (var i = 0; i < columns; i++) outputGrid.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) });
                var shown = expandedOutputs.Contains(day.Day) ? outputs : outputs.Take(6).ToArray();
                for (var i = 0; i < shown.Length; i++) { if (i % columns == 0) outputGrid.RowDefinitions.Add(new() { Height = GridLength.Auto }); Put(outputGrid, OutputCard(shown[i]), i / columns, i % columns); } main.Children.Add(outputGrid);
                if (outputs.Length > 6) main.Children.Add(Link(expandedOutputs.Contains(day.Day) ? L("收起产出", "Show fewer outputs") : L($"查看全部 {outputs.Length} 份产出", $"Show all {outputs.Length} outputs"), () => { if (!expandedOutputs.Add(day.Day)) expandedOutputs.Remove(day.Day); rendered = null; RenderFeed(); }));
            }
            foreach (var entry in notes.Where(e => !(e.Kind == "progress" && e.Id == headline.Id))) main.Children.Add(EntryRow(entry, false));
            foreach (var entry in sessions) main.Children.Add(Link(entry.Title, () => { if (openSession != null && entry.FrameId != null) openSession(entry.FrameId); else _ = model.SelectAsync(entry); }, 11));
        }
        else foreach (var entry in outputs.Take(3)) main.Children.Add(Link(entry.Title, () => _ = model.SelectAsync(entry), 11));
        Put(grid, new Border { Child = main, BorderBrush = design.Brush("border"), BorderThickness = new(1, 0, 0, 0) }, 0, 1);
        return new Border { Child = grid, Padding = new(0, 0, 8, 24), BorderBrush = design.Brush("border"), BorderThickness = new(0, 0, 0, 1) };
    }
    private FrameworkElement EntryRow(JourneyEntry entry, bool run)
    {
        var row = new Grid { ColumnSpacing = 12 }; Columns(row, GridLength.Auto, new(1, GridUnitType.Star), GridLength.Auto);
        row.Children.Add(design.Icon(NativeJourneyPresentation.Icon(entry.Kind), 18, design.ColorText("text-muted")));
        var label = new StackPanel { Spacing = 3 }; label.Children.Add(Text(entry.Title, 14)); if (!run && entry.Summary.Length > 0) label.Children.Add(Text(entry.Summary, 12, "text-faint")); Put(row, label, 0, 1);
        Put(row, Text(run ? entry.Status : entry.Manual ? L("手动记录", "Manual") : "", 11, "text-faint"), 0, 2);
        var button = new Button { Content = row, HorizontalContentAlignment = HorizontalAlignment.Stretch, HorizontalAlignment = HorizontalAlignment.Stretch }; design.QuietButton(button);
        button.Click += async (_, _) => { await model.SelectAsync(entry); if (run) await model.OpenRunAsync(); }; return button;
    }
    private FrameworkElement OutputCard(JourneyEntry entry)
    {
        var content = new StackPanel { Spacing = 7 }; var thumbnail = new NativeJourneyThumbnail(thumbnails, entry, design) { Height = ActualWidth <= 1200 ? 94 : 144 }; outputPreviews.Add(thumbnail); content.Children.Add(thumbnail);
        var title = Text(entry.Title, 11); title.TextWrapping = TextWrapping.NoWrap; title.TextTrimming = TextTrimming.CharacterEllipsis; content.Children.Add(title);
        content.Children.Add(Text((entry.SourceDiscarded ? L("源文件不可用", "Unavailable") : L("已登记产出", "Registered output")) + $" · v{entry.VersionNumber ?? 1}", 11, "text-faint"));
        var button = new Button { Content = content, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch, Padding = new(7), Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border"), BorderThickness = new(1), CornerRadius = new(8) };
        AutomationProperties.SetName(button, entry.Title); outputCards[entry.Id] = button; button.Click += (_, _) => _ = model.SelectAsync(entry); return button;
    }
    public void HandleEscape()
    {
        if (noteDate?.IsCalendarOpen == true) { noteDate.IsCalendarOpen = false; return; }
        if (noteCategory?.IsDropDownOpen == true) { noteCategory.IsDropDownOpen = false; return; }
        if (model.NoteOpen) { model.CloseNote(); return; }
        if (preview?.DismissSelection() == true) return;
        if (model.Run != null) { model.Back(); return; }
        if (artifactOpen) { artifactOpen = false; RenderDialog(); return; }
        if (recapEditor != null) { recapEditor = null; rendered = null; RenderFeed(); return; }
        if (selectedEdge != null) { selectedEdge = null; renderedGraph = null; RenderGraph(); return; }
        if (model.HasParentSource) { model.Back(); return; }
        close();
    }
    public void Dispose() { disposed = true; model.Changed -= Refresh; model.Dispose(); thumbnails.Dispose(); foreach (var thumbnail in outputPreviews) thumbnail.Dispose(); outputPreviews.Clear(); preview?.Dispose(); }
}
