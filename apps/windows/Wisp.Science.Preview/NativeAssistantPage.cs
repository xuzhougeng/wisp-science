using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeAssistantPage : UserControl, IWorkspaceSheet
{
    private readonly NativeAssistantModel model;
    private readonly WispDesign design;
    private readonly Action close;
    private readonly IReadOnlyList<ProjectSummary> knownProjects;
    private readonly Grid body = new();
    private readonly StackPanel errorNotice = new() { Spacing = 6, Margin = new Thickness(24, 12, 24, 0), Visibility = Visibility.Collapsed };
    private readonly Border left, right;
    private readonly Button backdrop = new();
    private readonly StackPanel projects = new() { Spacing = 4 }, calendar = new() { Spacing = 14 };
    private readonly NativeConversationPage conversation;
    private readonly Button leftToggle, rightToggle;
    private readonly Button remoteToggle, automationToggle;
    private readonly Action? openSpecialists;
    private NativeAutomationPage? automation;
    private readonly Grid layers = new();
    private readonly ContentControl surface = new() { HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch };
    private NativeAssistantRemoteDialog? remote;
    private readonly TextBlock note;
    private bool loaded, disposed, running;
    private NativeWorkspacePanel? files;
    private Border? filesBorder;
    private Button? filesBackdrop;
    private NativeSharePage? share;
    private readonly Action? openSkills;
    private readonly Func<NativeShareFile, Task<string?>>? saveShare;
    private readonly Func<Task<IReadOnlyList<string>>>? pickFiles;
    private readonly Func<string, Task<string?>>? pickDownload;
    private string? selection;

    public NativeAssistantPage(NativeAssistantModel model, WispDesign design, IReadOnlyList<ProjectSummary> projects, Action close, Action? openSpecialists = null,
        Func<Task<IReadOnlyList<string>>>? pickAttachments = null, Action? openSkills = null, Func<NativeShareFile, Task<string?>>? saveShare = null,
        Func<string, Task<string?>>? pickDownload = null, Action? openAgentSettings = null, Action? openComputeSettings = null, Action? addComputeHost = null, Action? openModelSettings = null,
        Func<bool>? modelSwitchWarningDisabled = null, Action? disableModelSwitchWarning = null)
    {
        this.openSpecialists = openSpecialists;
        pickFiles = pickAttachments;
        this.pickDownload = pickDownload;
        this.openSkills = openSkills; this.saveShare = saveShare;
        this.model = model; this.design = design; this.close = close; knownProjects = projects;
        HorizontalContentAlignment = HorizontalAlignment.Stretch; VerticalContentAlignment = VerticalAlignment.Stretch;
        Background = design.Brush("bg-app"); design.BindTypography(this);
        AutomationProperties.SetAutomationId(this, "assistant-workspace");
        var root = new Grid { Background = design.Brush("bg-app") }; Rows(root, new(56), new(1, GridUnitType.Star));
        var header = new Grid { ColumnSpacing = 12, Padding = new Thickness(18, 0, 18, 0) };
        Columns(header, GridLength.Auto, GridLength.Auto, new(1, GridUnitType.Star), GridLength.Auto, GridLength.Auto, GridLength.Auto, GridLength.Auto);
        leftToggle = Tool(L("切换项目列表", "Toggle projects"), "panel-left", () => Toggle(true)); header.Children.Add(leftToggle);
        AutomationProperties.SetAutomationId(leftToggle, "assistant-projects-toggle");
        var title = Text(design.T("assistant.title"), 15, bold: true); title.VerticalAlignment = VerticalAlignment.Center; Put(header, title, 0, 1);
        note = Text(L("让研究，日渐清晰", "A little more clarity, every day"), 12, "text-muted"); note.VerticalAlignment = VerticalAlignment.Center; Put(header, note, 0, 2);
        automationToggle = Tool(L("自动化", "Automation"), "clock", OpenAutomation, true);
        automationToggle.Padding = new Thickness(8, 6, 8, 6); automationToggle.BorderBrush = design.Brush("border"); automationToggle.BorderThickness = new Thickness(1); automationToggle.CornerRadius = new CornerRadius(6);
        AutomationProperties.SetAutomationId(automationToggle, "assistant-automation-toggle"); Put(header, automationToggle, 0, 3);
        remoteToggle = Tool(L("远程接入", "Remote access"), "link", OpenRemote, true);
        remoteToggle.Padding = new Thickness(8, 6, 8, 6); remoteToggle.BorderBrush = design.Brush("border"); remoteToggle.BorderThickness = new Thickness(1); remoteToggle.CornerRadius = new CornerRadius(6);
        AutomationProperties.SetAutomationId(remoteToggle, "assistant-remote-toggle"); Put(header, remoteToggle, 0, 4);
        rightToggle = Tool(L("切换研究日历", "Toggle research calendar"), "panel", () => Toggle(false)); Put(header, rightToggle, 0, 5);
        AutomationProperties.SetAutomationId(rightToggle, "assistant-calendar-toggle");
        Put(header, Tool(design.T("assistant.close"), "close", close), 0, 6);
        root.Children.Add(new Border { Child = header, BorderBrush = design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, 1) });
        Columns(body, new(0), new(1, GridUnitType.Star), new(0)); Put(root, body, 1);
        conversation = new(model.Conversation, design, text => model.Conversation.Prefill(text, append: true), () => Task.CompletedTask,
            pickAttachments: pickAttachments, composeNavigate: OpenComposeSurface, openAgentSettings: openAgentSettings, openComputeSettings: openComputeSettings, addComputeHost: addComputeHost, openModelSettings: openModelSettings,
            modelSwitchWarningDisabled: modelSwitchWarningDisabled, disableModelSwitchWarning: disableModelSwitchWarning);
        var center = new Grid(); Rows(center, GridLength.Auto, new(1, GridUnitType.Star));
        center.Children.Add(errorNotice); Put(center, conversation, 1); Put(body, center, 0, 1);
        backdrop.Background = new SolidColorBrush(Windows.UI.Color.FromArgb(40, 0, 0, 0)); backdrop.BorderThickness = new Thickness(0);
        backdrop.HorizontalAlignment = HorizontalAlignment.Stretch; backdrop.VerticalAlignment = VerticalAlignment.Stretch;
        backdrop.Click += (_, _) => CloseDrawer(); AutomationProperties.SetName(backdrop, L("关闭侧栏", "Close sidebar"));
        body.Children.Add(backdrop); Grid.SetColumnSpan(backdrop, 3); Canvas.SetZIndex(backdrop, 1);
        var projectBody = new Grid { Padding = new Thickness(12, 0, 12, 0) };
        Rows(projectBody, GridLength.Auto, new(1, GridUnitType.Star), GridLength.Auto);
        var projectTitle = Text(L("项目", "Projects"), 13, bold: true); projectTitle.Margin = new Thickness(8, 18, 8, 14); projectBody.Children.Add(projectTitle);
        Put(projectBody, Scroller(this.projects), 1);
        Put(projectBody, new Border { Child = Text(L("所选项目作为后续提问的上下文。", "Project context applies to your next message."), 11, "text-muted"),
            BorderBrush = design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0), Padding = new Thickness(8, 12, 8, 14) }, 2);
        left = Rail(projectBody, true); body.Children.Add(left); AutomationProperties.SetAutomationId(left, "assistant-projects");
        var calendarBody = new Grid(); Rows(calendarBody, GridLength.Auto, new(1, GridUnitType.Star));
        var calendarTitle = Text(L("研究日历", "Research calendar"), 13, bold: true); calendarTitle.Margin = new Thickness(14, 18, 14, 14); calendarBody.Children.Add(calendarTitle);
        calendar.Margin = new Thickness(12, 0, 12, 20); Put(calendarBody, Scroller(calendar), 1);
        right = Rail(calendarBody, false); Put(body, right, 0, 2); AutomationProperties.SetAutomationId(right, "assistant-calendar");
        Columns(layers, new(1, GridUnitType.Star), GridLength.Auto);
        surface.Content = root; layers.Children.Add(surface); Content = layers;
        SizeChanged += (_, _) => Arrange();
        model.Changed += Render; model.Conversation.Changed += ConversationChanged;
        Loaded += async (_, _) => { if (loaded) return; loaded = true; Arrange(); await model.OpenAsync(); };
        Render();
    }
    private void ConversationChanged()
    {
        if (disposed) return;
        if (selection != model.SelectedProjectId) { selection = model.SelectedProjectId; RenderProjects(); }
        var wasRunning = running; running = model.Conversation.Snapshot?.Running == true;
        if (wasRunning && !running && model.Active && model.Conversation.Snapshot != null && !model.Conversation.Loading) _ = model.ReloadAsync();
    }
    private void Toggle(bool left) { model.Layout.Toggle(left); Arrange(); }
    private void CloseDrawer()
    {
        var leftWasTop = model.Layout.Left && (model.Layout.LastLeft || !model.Layout.Right);
        if (!model.Layout.CloseDrawer()) return;
        Arrange(); (leftWasTop ? leftToggle : rightToggle).Focus(FocusState.Programmatic);
    }
    private void Arrange()
    {
        if (disposed || ActualWidth <= 0) return;
        var layout = model.Layout; layout.Resize(ActualWidth);
        var fileOverlay = files != null && layout.Narrow;
        var fileWidth = files == null ? 0 : Math.Min(360, Math.Max(0, ActualWidth - 32));
        layers.ColumnDefinitions[1].Width = new(files != null && !fileOverlay ? fileWidth : 0);
        if (filesBorder != null)
        {
            filesBorder.Width = fileWidth; Grid.SetColumn(filesBorder, fileOverlay ? 0 : 1); Grid.SetColumnSpan(filesBorder, fileOverlay ? 2 : 1);
            filesBackdrop!.Visibility = fileOverlay ? Visibility.Visible : Visibility.Collapsed;
        }
        surface.IsEnabled = !fileOverlay && remote == null && automation == null && share == null;
        var width = ActualWidth - (files != null && !fileOverlay ? fileWidth : 0);
        note.Visibility = width <= 720 ? Visibility.Collapsed : Visibility.Visible;
        body.ColumnDefinitions[0].Width = new(layout.Narrow || !layout.Left ? 0 : NativeAssistantLayout.LeftWidth(width));
        body.ColumnDefinitions[2].Width = new(layout.Narrow || !layout.Right ? 0 : NativeAssistantLayout.RightWidth(width));
        conversation.Width = Math.Max(0, Math.Min(948, width - body.ColumnDefinitions[0].Width.Value - body.ColumnDefinitions[2].Width.Value));
        conversation.HorizontalAlignment = HorizontalAlignment.Center;
        left.Width = layout.Narrow ? Math.Max(0, Math.Min(280, width - 40)) : NativeAssistantLayout.LeftWidth(width);
        right.Width = layout.Narrow ? Math.Max(0, Math.Min(350, width - 32)) : NativeAssistantLayout.RightWidth(width);
        foreach (var rail in new[] { left, right }) Grid.SetColumnSpan(rail, layout.Narrow ? 3 : 1);
        Grid.SetColumn(right, layout.Narrow ? 0 : 2);
        left.HorizontalAlignment = HorizontalAlignment.Left; right.HorizontalAlignment = HorizontalAlignment.Right;
        left.Visibility = layout.Left ? Visibility.Visible : Visibility.Collapsed; right.Visibility = layout.Right ? Visibility.Visible : Visibility.Collapsed;
        Canvas.SetZIndex(left, layout.LastLeft ? 3 : 2); Canvas.SetZIndex(right, layout.LastLeft ? 2 : 3);
        backdrop.Visibility = layout.Narrow && (layout.Left || layout.Right) ? Visibility.Visible : Visibility.Collapsed;
        leftToggle.Background = design.Brush(layout.Left ? "bg-sunken" : "bg-app");
        rightToggle.Background = design.Brush(layout.Right ? "bg-sunken" : "bg-app");
        // A drawer blocks keyboard focus in the covered conversation and lower rail.
        conversation.IsEnabled = backdrop.Visibility != Visibility.Visible;
        ((Control)left.Child).IsEnabled = !layout.Narrow || !layout.Right || layout.LastLeft;
        ((Control)right.Child).IsEnabled = !layout.Narrow || !layout.Left || !layout.LastLeft;
    }
    private void Render()
    {
        if (disposed) return;
        errorNotice.Children.Clear(); errorNotice.Visibility = model.Error == null ? Visibility.Collapsed : Visibility.Visible;
        if (model.Error != null)
        {
            errorNotice.Children.Add(Text(model.Error, 12, "text-muted"));
            errorNotice.Children.Add(Tool(L("重新连接", "Reconnect"), "refresh", () => _ = model.OpenAsync(), true));
        }
        RenderProjects(); RenderCalendar(); Arrange();
    }
    private void RenderProjects()
    {
        projects.Children.Clear();
        projects.Children.Add(ProjectButton(null, L("全部项目", "All projects"), L("跨项目讨论", "Discuss across projects"), "layers"));
        projects.Children.Add(Divider());
        if (model.Error != null) { projects.Children.Add(Text(model.Error, 12, "text-muted")); projects.Children.Add(Tool(L("重试", "Retry"), "refresh", () => _ = model.OpenAsync(), true)); }
        else if (model.Workspace == null) projects.Children.Add(Text(L("正在读取项目…", "Loading projects…"), 12, "text-muted"));
        else if (model.Workspace.Projects.Length == 0) projects.Children.Add(Text(L("暂无可见项目", "No visible projects"), 12, "text-muted"));
        else foreach (var project in model.Workspace.Projects)
        {
            var known = knownProjects.FirstOrDefault(p => p.Id == project.Id);
            var detail = known == null ? project.Description : L($"{known.SessionCount} 个会话", $"{known.SessionCount} conversations");
            projects.Children.Add(ProjectButton(project.Id, project.Name, detail, "folder"));
        }
    }
    private Button ProjectButton(string? id, string name, string detail, string icon)
    {
        var row = new Grid { ColumnSpacing = 10 }; Columns(row, GridLength.Auto, new(1, GridUnitType.Star));
        var symbol = design.Icon(icon, 16); symbol.VerticalAlignment = VerticalAlignment.Center; row.Children.Add(symbol);
        var text = new StackPanel { Spacing = 3 }; text.Children.Add(Single(name, 13)); text.Children.Add(Single(detail, 11, "text-muted")); Put(row, text, 0, 1);
        var button = new Button { Content = row, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        design.QuietButton(button); button.Padding = new Thickness(10, 11, 10, 11);
        button.Background = design.Brush(model.SelectedProjectId == id ? "clay-soft" : "bg-app"); button.IsEnabled = model.Workspace != null;
        AutomationProperties.SetName(button, name); AutomationProperties.SetAutomationId(button, "assistant-project-" + (id ?? "all")); ToolTipService.SetToolTip(button, name);
        button.Click += (_, _) => { model.SelectProject(id); if (model.Layout.Narrow) CloseDrawer(); };
        return button;
    }
    private void RenderCalendar()
    {
        calendar.Children.Clear();
        if (model.Workspace == null)
        {
            calendar.Children.Add(Text(model.Error == null ? L("正在确认可见项目…", "Loading visible projects…") : L("暂时无法确认可见项目，请重新读取后查看日历。", "Project visibility could not be verified. Reload to view the calendar."), 12, "text-muted")); return;
        }
        var toolbar = new Grid { ColumnSpacing = 1 }; Columns(toolbar, new(1, GridUnitType.Star), GridLength.Auto, GridLength.Auto, GridLength.Auto, GridLength.Auto);
        var month = Text(model.Month.ToString("yyyy / MM"), 13, bold: true); month.VerticalAlignment = VerticalAlignment.Center; toolbar.Children.Add(month);
        Put(toolbar, Tool(L("上个月", "Previous month"), "chevron-left", () => _ = model.SelectDayAsync(NativeCalendarPresentation.SelectedDay(model.Month.AddMonths(-1), DateTime.Today))), 0, 1);
        Put(toolbar, Tool(L("下个月", "Next month"), "chevron-right", () => _ = model.SelectDayAsync(NativeCalendarPresentation.SelectedDay(model.Month.AddMonths(1), DateTime.Today))), 0, 2);
        var today = new Button { Content = Text(L("今天", "Today"), 11) }; design.QuietButton(today); today.Padding = new Thickness(5); today.Click += (_, _) => _ = model.SelectDayAsync(DateTime.Today); Put(toolbar, today, 0, 3);
        Put(toolbar, Tool(L("刷新日历", "Refresh calendar"), "refresh", () => _ = model.ReloadAsync()), 0, 4); calendar.Children.Add(toolbar);
        var days = new Grid { ColumnSpacing = 3, RowSpacing = 3 }; for (var i = 0; i < 7; i++) days.ColumnDefinitions.Add(new());
        Rows(days, GridLength.Auto); var names = design.Localization.Locale == "zh" ? new[] { "一", "二", "三", "四", "五", "六", "日" } : ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        for (var i = 0; i < 7; i++) { var name = Text(names[i], 10, "text-muted"); name.HorizontalAlignment = HorizontalAlignment.Center; name.Margin = new Thickness(0, 0, 0, 8); Put(days, name, 0, i); }
        var cells = NativeCalendarPresentation.Cells(model.Month);
        for (var i = 0; i < cells.Count; i++)
        {
            if (i % 7 == 0) days.RowDefinitions.Add(new() { Height = new(36) });
            if (cells[i] is not { } date) continue;
            var text = Text(date.Day.ToString(), 12, date == model.Day ? "clay-strong" : "text");
            var button = new Button { Content = text, HorizontalAlignment = HorizontalAlignment.Stretch, VerticalAlignment = VerticalAlignment.Stretch };
            design.QuietButton(button); button.Padding = new Thickness(0); button.MinWidth = 0;
            button.Background = design.Brush(date == model.Day ? "clay-soft" : "bg-app");
            button.BorderBrush = design.Brush(date == model.Day ? "clay" : "bg-app"); button.BorderThickness = new Thickness(1);
            AutomationProperties.SetName(button, date.ToString("yyyy-MM-dd")); button.Click += (_, _) => _ = model.SelectDayAsync(date); Put(days, button, i / 7 + 1, i % 7);
        }
        calendar.Children.Add(days); calendar.Children.Add(Divider());
        calendar.Children.Add(Text(model.Day.ToString("yyyy-MM-dd") + (model.Day == DateTime.Today ? L(" · 今天", " · Today") : ""), 13, bold: true));
        var planHeader = new Grid(); Columns(planHeader, new(1, GridUnitType.Star), GridLength.Auto);
        var saved = Text(L("已保存计划", "Saved plans"), 12, bold: true); saved.VerticalAlignment = VerticalAlignment.Center; planHeader.Children.Add(saved);
        Put(planHeader, Tool(L("安排这一天", "Plan this day"), "plus", () => { model.DraftDay(design.Localization.Locale == "zh"); if (model.Layout.Narrow) CloseDrawer(); conversation.FocusComposer(); }, true), 0, 1); calendar.Children.Add(planHeader);
        if (model.Workspace.Plan.Length == 0) calendar.Children.Add(Text(L("暂无已保存计划，可以和助理一起安排。", "No saved plans. Discuss a plan with your assistant."), 12, "text-muted"));
        foreach (var plan in model.Workspace.Plan)
        {
            var item = new StackPanel { Spacing = 4 }; item.Children.Add(Text(plan.Title, 12));
            var state = plan.Status == "done" ? L("完成", "Done") : plan.Status == "dropped" ? L("放弃", "Dropped") : L("待办", "To do");
            item.Children.Add(Text((plan.ProjectName ?? L("跨项目", "Across projects")) + " · " + state, 11, "text-muted")); calendar.Children.Add(item);
        }
        calendar.Children.Add(Divider()); calendar.Children.Add(Text(L("研究活动", "Research activity"), 12, bold: true));
        if (model.CalendarError != null) calendar.Children.Add(Text(model.CalendarError, 12, "text-muted"));
        var visible = model.Activity.Select(row => (Row: row, Entries: row.History.Entries.Where(e => NativeCalendarPresentation.LocalDay(e.OccurredAt) == model.Day).ToArray())).ToArray();
        if (model.CalendarError == null && visible.All(row => row.Entries.Length == 0 && row.Row.Error == null)) calendar.Children.Add(Text(L("当天暂无已记录的活动。", "No recorded activity on this date."), 12, "text-muted"));
        foreach (var (row, entries) in visible)
        {
            if (entries.Length == 0 && row.Error == null) continue;
            calendar.Children.Add(Text(model.Workspace.Projects.First(p => p.Id == row.ProjectId).Name, 12, bold: true));
            if (row.Error != null) calendar.Children.Add(Text(row.Error, 12, "text-muted"));
            foreach (var entry in NativeCalendarPresentation.FoldOutputs(entries).Entries.Take(8)) calendar.Children.Add(Text(entry.Title, 12, "text-muted"));
        }
        var count = model.Activity.Count(row => row.History.Entries.Count > 0);
        var activeDays = model.Activity.SelectMany(row => row.History.Entries).Select(e => NativeCalendarPresentation.LocalDay(e.OccurredAt)).Distinct().Count();
        calendar.Children.Add(Text(L($"本月 · {count} 个活跃项目 · {activeDays} 个活跃日", $"This month · {count} active projects · {activeDays} active days") + "\n" + L("已记录的主线活动 · 本地时区", "Recorded mainline activity · Local timezone"), 10, "text-muted"));
    }
    public void HandleEscape()
    {
        if (share != null) { share.HandleEscape(); return; }
        if (remote != null) { CloseRemote(); return; }
        if (automation != null) { automation.HandleEscape(); return; }
        if (conversation.HandleComposerTextEscape() || conversation.HandleAttachmentEscape() || conversation.HandleQueueEscape() || conversation.HandleHistoryEscape() || conversation.HandleReadingEscape()
            || conversation.HandleAcpEscape() || conversation.HandleComposeMenuEscape() || conversation.HandleComposerOptionsEscape()) return;
        if (files != null) { if (!files.HandleEscape()) CloseFiles(); return; }
        if (conversation.HandleEscape()) return;
        if (model.Layout.Narrow && (model.Layout.Left || model.Layout.Right)) CloseDrawer(); else close();
    }
    public void Dispose()
    {
        if (disposed) return; disposed = true;
        remote?.Dispose(); remote = null;
        automation?.Dispose(); automation = null;
        share?.Dispose(); share = null; files?.Dispose(); files = null;
        model.Changed -= Render; model.Conversation.Changed -= ConversationChanged; conversation.Dispose(); model.Close();
    }
    private void OpenComposeSurface(string action)
    {
        if (disposed || !model.Active) return;
        if (action == "skills") { openSkills?.Invoke(); return; }
        if (!model.Conversation.ComposeSessionReady) return;
        if (action == "share" && model.Conversation.CanShareConversation && share == null && saveShare != null)
        {
            share = new(model.CreateShare(), design, saveShare, CloseShare);
            Grid.SetColumnSpan(share, 2); layers.Children.Add(share); surface.IsEnabled = false;
            if (files != null) files.IsEnabled = false;
        }
        else if (action == "files" && files == null)
        {
            files = new(model.CreateFiles(), model.Conversation, design, CloseFiles, windowEscape: () => { HandleEscape(); return true; }, pickFiles: pickFiles, pickDownload: pickDownload);
            filesBackdrop = new Button { Background = new SolidColorBrush(Windows.UI.Color.FromArgb(40, 0, 0, 0)), BorderThickness = new Thickness(0),
                HorizontalAlignment = HorizontalAlignment.Stretch, VerticalAlignment = VerticalAlignment.Stretch };
            AutomationProperties.SetName(filesBackdrop, design.T("right.close")); filesBackdrop.Click += (_, _) => CloseFiles();
            Grid.SetColumnSpan(filesBackdrop, 2); layers.Children.Add(filesBackdrop);
            filesBorder = new Border { Child = files, HorizontalAlignment = HorizontalAlignment.Right,
                BorderBrush = design.Brush("border"), BorderThickness = new Thickness(1, 0, 0, 0) };
            layers.Children.Add(filesBorder); Arrange();
        }
    }
    private void CloseFiles()
    {
        if (files == null) return;
        files.Dispose(); files = null; layers.Children.Remove(filesBorder!); layers.Children.Remove(filesBackdrop!);
        filesBorder = null; filesBackdrop = null; Arrange(); conversation.FocusComposer();
    }
    private void CloseShare()
    {
        if (share == null) return;
        share.Dispose(); layers.Children.Remove(share); share = null;
        if (files != null) files.IsEnabled = true;
        Arrange(); conversation.FocusComposer();
    }
    private void OpenRemote()
    {
        if (disposed || remote != null) return;
        remote = new(model.CreateRemote(), design, CloseRemote);
        surface.IsEnabled = false; Grid.SetColumnSpan(remote, 2); layers.Children.Add(remote);
        if (files != null) files.IsEnabled = false;
    }
    private void OpenAutomation()
    {
        if (disposed || automation != null || remote != null) return;
        automation = new(model.CreateAutomation(), design, CloseAutomation, openSpecialists);
        surface.IsEnabled = false; Grid.SetColumnSpan(automation, 2); layers.Children.Add(automation);
        if (files != null) files.IsEnabled = false;
    }
    private void CloseAutomation()
    {
        if (automation == null) return;
        automation.Dispose(); layers.Children.Remove(automation); automation = null;
        if (files != null) files.IsEnabled = true;
        Arrange(); automationToggle.Focus(FocusState.Programmatic);
    }
    private void CloseRemote()
    {
        if (remote == null) return;
        remote.Dispose(); layers.Children.Remove(remote); remote = null;
        if (files != null) files.IsEnabled = true;
        Arrange(); remoteToggle.Focus(FocusState.Programmatic);
    }
    private Border Rail(UIElement child, bool left) => new() { Child = new ContentControl { Content = child, HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch }, Background = design.Brush("bg-app"), BorderBrush = design.Brush("border"), BorderThickness = left ? new(0, 0, 1, 0) : new(1, 0, 0, 0) };
    private Border Divider() => new() { Height = 1, Background = design.Brush("border"), Margin = new Thickness(0, 8, 0, 8) };
    private Button Tool(string label, string icon, Action action, bool text = false)
    {
        var button = design.ToolButton(label, icon, text); button.Padding = new Thickness(5); button.MinWidth = 0;
        button.Click += (_, _) => action(); return button;
    }
    private string L(string zh, string en) => design.L(zh, en);
    private TextBlock Text(string text, double size, string color = "text", bool bold = false)
    { var label = design.Text(text, size); label.Foreground = design.Brush(color); if (bold) label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; return label; }
    private TextBlock Single(string text, double size, string color = "text")
    { var label = Text(text, size, color); label.TextWrapping = TextWrapping.NoWrap; label.TextTrimming = TextTrimming.CharacterEllipsis; return label; }
    private static ScrollViewer Scroller(UIElement content) => new() { Content = content, HorizontalScrollMode = ScrollMode.Disabled, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalContentAlignment = HorizontalAlignment.Stretch };
    private static void Rows(Grid grid, params GridLength[] heights) { foreach (var height in heights) grid.RowDefinitions.Add(new() { Height = height }); }
    private static void Columns(Grid grid, params GridLength[] widths) { foreach (var width in widths) grid.ColumnDefinitions.Add(new() { Width = width }); }
    private static void Put(Grid grid, FrameworkElement element, int row, int column = 0) { Grid.SetRow(element, row); Grid.SetColumn(element, column); grid.Children.Add(element); }
}
