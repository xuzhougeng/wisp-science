using System.Diagnostics;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.Storage.Pickers;
using Windows.System;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class MainWindow : Window
{
    private readonly Grid root = new();
    private readonly WispDesign design = new();
    private readonly PreviewSettings settings = PreviewSettings.Load();
    private readonly ProjectBrowserModel model;
    private readonly List<FlyoutBase> overlays = [];
    private ProjectSearchOverlay? searchOverlay;
    private Control? searchPreviousFocus;
    private FrameworkElement? pageContent;
    private NativeSettingsPage? settingsPage;
    private IWorkspaceSheet? workspaceSheet;
    private NativeSettingsClient? workspaceHost;
    private Task<NativeSettingsClient?>? connectingHost;
    private int conversationNavigation;
    private WorkspaceInboxModel? inbox;
    private NativeWorkspacePanel? panelPage;
    private NativeWorkspaceTerminal? terminalPage;
    private WorkspaceConversationModel? conversation;
    private NativeConversationPage? conversationPage;
    private WorkspaceSideChatModel? sideChat;
    private bool panelVisible;
    private bool terminalVisible;
    private string? workspaceKey;
    private bool windowClosed;
    private bool sidebarVisible = true;
    private PreviewLayout layout = PreviewLayout.ForSize(800, 540);
    private bool nativePickerOpen;
    private ScrollViewer? transcriptScroll;
    private string? renderedSession;
    private long? renderedFirst;
    private bool scrollToLatest;
    private string? localError;
    private bool rendering;
    private bool renderQueued;
    private Grid? workspaceMain;
    private Grid? workspaceShell;

    public MainWindow()
    {
        Title = "Wisp Science · WinUI 3 Preview";
        AppWindow.Resize(new Windows.Graphics.SizeInt32(1220, 860));
        Content = root;
        design.Typography = settings.Typography ?? new();
        root.KeyboardAcceleratorPlacementMode = KeyboardAcceleratorPlacementMode.Hidden;
        root.RequestedTheme = settings.Appearance switch { "light" => ElementTheme.Light, "dark" => ElementTheme.Dark, _ => ElementTheme.Default };
        panelVisible = settings.PanelVisible;
        model = new(new ProjectBrowserClient(Environment.GetEnvironmentVariable("WISP_SERVICE_PATH")
            ?? Path.Combine(AppContext.BaseDirectory, "wisp-service.exe")), settings.ResolveDatabase());
        model.Changed += Render;
        root.ActualThemeChanged += (_, _) => Render();
        root.SizeChanged += (_, e) =>
        {
            var next = PreviewLayout.ForSize(e.NewSize.Width, e.NewSize.Height);
            if (next != layout) { layout = next; Render(); }
        };
        Shortcut(VirtualKey.K, VirtualKeyModifiers.Control, OpenSearch);
        Shortcut(VirtualKey.R, VirtualKeyModifiers.Control, () => _ = model.RefreshAsync());
        Shortcut(VirtualKey.O, VirtualKeyModifiers.Control, ChooseDatabase);
        // Window-root accelerator also works immediately after opening a surface.
        // Native text menus handle Escape first; registered app flyouts precede the dialog.
        var escape = new KeyboardAccelerator { Key = VirtualKey.Escape };
        escape.Invoked += (_, e) =>
        {
            if (nativePickerOpen) return;
            if (overlays.LastOrDefault() is { } flyout) { flyout.Hide(); e.Handled = true; }
            else if (searchOverlay != null) { CloseSearch(); e.Handled = true; }
            else if (workspaceSheet != null) { workspaceSheet.HandleEscape(); e.Handled = true; }
            else if (settingsPage != null) { settingsPage.HandleEscape(); e.Handled = true; }
            else if (projectPage != null) { projectPage.HandleEscape(); e.Handled = true; }
        };
        root.KeyboardAccelerators.Add(escape);
        Closed += (_, _) =>
        {
            windowClosed = true; model.Changed -= Render; model.Dispose();
            DisposeWorkspace(); conversationPage?.Dispose(); conversation?.Pause();
            settingsPage?.Dispose(); workspaceHost?.Dispose();
        };
        root.Loaded += async (_, _) => await model.RefreshAsync();
        Render();
    }

    private void Shortcut(VirtualKey key, VirtualKeyModifiers modifiers, Action action)
    {
        var accelerator = new KeyboardAccelerator { Key = key, Modifiers = modifiers };
        accelerator.Invoked += (_, e) =>
        {
            if (nativePickerOpen || searchOverlay != null || settingsPage != null || workspaceSheet != null) return;
            action(); e.Handled = true;
        };
        root.KeyboardAccelerators.Add(accelerator);
    }

    private void Render()
    {
        if (rendering)
        {
            if (!renderQueued) { renderQueued = true; DispatcherQueue.TryEnqueue(() => { renderQueued = false; Render(); }); }
            return;
        }
        rendering = true;
        try { RenderCore(); }
        catch (Exception ex)
        {
            localError = "界面未能完成加载：" + ex.Message;
            root.Children.Clear();
            var recovery = Stack(12); recovery.Margin = new Thickness(24);
            recovery.Children.Add(Text(localError, 14, "clay-strong"));
            recovery.Children.Add(ActionButton("返回项目", "arrow-left", model.GoHome, true));
            root.Children.Add(recovery);
        }
        finally { design.ApplyTypography(root, settingsPage); rendering = false; }
    }

    private void RenderCore()
    {
        if (settingsPage != null) return;
        SyncWorkspaceSession();
        var offset = transcriptScroll?.VerticalOffset ?? 0;
        var height = transcriptScroll?.ExtentHeight ?? 0;
        var oldSession = renderedSession;
        var oldFirst = renderedFirst;
        design.LightPalette = settings.LightPalette;
        design.DarkPalette = settings.DarkPalette;
        design.Dark = root.ActualTheme == ElementTheme.Dark;
        root.Background = design.Brush("bg-app");
        // WinUI can report Parent=null while an unloaded subtree still owns a
        // reusable UserControl. Detach through the retained owner before remounting.
        workspaceMain?.Children.Clear(); workspaceMain = null;
        workspaceShell?.Children.Clear(); workspaceShell = null;
        root.Children.Clear();
        transcriptScroll = null;
        var project = model.Projects.FirstOrDefault(p => p.Id == model.ActiveProjectId);
        pageContent = project == null ? Home() : Workspace(project);
        pageContent.IsHitTestVisible = searchOverlay == null && workspaceSheet == null;
        pageContent.Visibility = workspaceSheet == null ? Visibility.Visible : Visibility.Collapsed;
        root.Children.Add(pageContent);
        if (searchOverlay != null) root.Children.Add(searchOverlay);
        if (workspaceSheet is UserControl sheet) root.Children.Add(sheet);
        renderedSession = model.ActiveSessionId;
        renderedFirst = model.Messages.FirstOrDefault()?.Sequence;
        if (oldSession != renderedSession || (oldFirst == null && renderedFirst != null)) scrollToLatest = true;
        if (transcriptScroll is { } scroll)
        {
            // Loaded can run before ScrollViewer has measured a long transcript.
            // Keep the intent through the final sessions-loading notification.
            EventHandler<object>? restore = null;
            restore = (_, _) =>
            {
                if (scroll.ViewportHeight <= 0 || scroll.ExtentHeight <= 0 || model.Messages.Count == 0) return;
                scroll.LayoutUpdated -= restore;
                if (scroll != transcriptScroll) return;
                if (scrollToLatest) { scroll.ChangeView(null, scroll.ScrollableHeight, null, true); scrollToLatest = false; }
                else scroll.ChangeView(null, offset + (oldFirst != renderedFirst ? Math.Max(0, scroll.ExtentHeight - height) : 0), null, true);
            };
            scroll.LayoutUpdated += restore;
            scroll.Unloaded += (_, _) => scroll.LayoutUpdated -= restore;
        }
    }

    private FrameworkElement Home()
    {
        var page = new Grid { RowSpacing = 18 };
        page.RowDefinitions.Add(new() { Height = GridLength.Auto });
        page.RowDefinitions.Add(new() { Height = GridLength.Auto });
        page.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        page.RowDefinitions.Add(new() { Height = GridLength.Auto });
        page.MaxWidth = 1200;
        page.HorizontalAlignment = HorizontalAlignment.Stretch;
        page.Margin = new Thickness(layout.StackHomeColumns ? 20 : 28, 18, layout.StackHomeColumns ? 20 : 28, 12);
        var header = new Grid { ColumnSpacing = 20, RowSpacing = 12 };
        header.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        header.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var brand = Row(16);
        var mark = design.Wordmark();
        mark.Width = layout.StackHomeColumns ? 90 : 120; mark.Height = layout.StackHomeColumns ? 60 : 80;
        AutomationProperties.SetName(mark, "Wisp Science");
        brand.Children.Add(mark);
        var tagline = Stack(8); tagline.VerticalAlignment = VerticalAlignment.Center;
        tagline.Children.Add(Text("严谨做科研，", 14, "text-muted"));
        tagline.Children.Add(Text("Wisp Science 在身边。", 14, "clay-strong"));
        if (!layout.StackHomeColumns) brand.Children.Add(tagline);
        header.Children.Add(brand);
        var actions = new StackPanel { Spacing = 8, Orientation = layout.StackHeader ? Orientation.Vertical : Orientation.Horizontal,
            HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Top };
        var quickActions = Row(6); quickActions.HorizontalAlignment = HorizontalAlignment.Right;
        quickActions.Children.Add(ActionButton("研究日历", "calendar", () => _ = OpenNativeAction("calendar")));
        quickActions.Children.Add(ActionButton("收藏", "star", () => _ = OpenNativeAction("library")));
        quickActions.Children.Add(ActionButton("教程", "book", OpenTutorials));
        quickActions.Children.Add(ActionButton("搜索", "search", OpenSearch));
        quickActions.Children.Add(ActionButton("设置", "gear", OpenSettings));
        var projectActions = Row(6); projectActions.HorizontalAlignment = HorizontalAlignment.Right;
        projectActions.Children.Add(ActionButton("随手一聊", null, () => _ = OpenNativeAction("scratch")));
        projectActions.Children.Add(ActionButton("导入项目", "upload", () => _ = OpenNativeAction("import"), showLabel: true));
        projectActions.Children.Add(ActionButton("新建项目", "plus", () => _ = OpenNativeAction("create"), showLabel: true, primary: true));
        actions.Children.Add(quickActions); actions.Children.Add(projectActions);
        Grid.SetColumn(actions, 1);
        header.Children.Add(actions);
        page.Children.Add(header);
        if ((localError ?? model.Error) is { } error)
        {
            var banner = Card(Text(error + (model.LastLoaded != null ? "\n当前显示上次成功读取的数据。" : ""), 13, "clay-strong"));
            Grid.SetRow(banner, 1); page.Children.Add(banner);
        }
        var columns = new Grid { ColumnSpacing = 24, RowSpacing = 18 };
        columns.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        if (!layout.StackHomeColumns) columns.ColumnDefinitions.Add(new() { Width = new GridLength(0.85, GridUnitType.Star) });
        columns.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        if (layout.StackHomeColumns) columns.RowDefinitions.Add(new() { Height = new GridLength(0.7, GridUnitType.Star) });
        var projects = Stack(8);
        foreach (var project in model.Projects)
        {
            var content = Stack(4);
            content.Children.Add(SingleLine(project.Name, 14));
            content.Children.Add(SingleLine(project.WorkspaceDirectory, 11, "text-faint"));
            content.Children.Add(Text($"{project.SessionCount} 会话 · {project.ArtifactCount} 产物" +
                (project.NeedsYouCount > 0 ? $" · {project.NeedsYouCount} 待查看" : ""), 12, "text-muted"));
            var cardRow = new Grid();
            cardRow.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            cardRow.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var open = ContentButton(content, () => _ = model.OpenProjectAsync(project.Id), $"project-{project.Id}", project.Name);
            open.HorizontalAlignment = HorizontalAlignment.Stretch;
            cardRow.Children.Add(open);
            var options = Row(2); options.VerticalAlignment = VerticalAlignment.Center;
            var star = ActionButton(project.Starred ? "取消收藏" : "收藏项目", project.Starred ? "star-filled" : "star", () => _ = model.SetStarredAsync(project.Id, !project.Starred), quiet: true);
            star.IsEnabled = !model.Loading; options.Children.Add(star);
            options.Children.Add(ActionButton("项目设置", "gear", () => OpenSettingsSection("project", project.Id), quiet: true));
            Grid.SetColumn(options, 1); cardRow.Children.Add(options);
            var card = Card(cardRow, 4);
            var menu = new MenuFlyout();
            var reveal = new MenuFlyoutItem { Text = "在资源管理器中显示", IsEnabled = Directory.Exists(project.WorkspaceDirectory) };
            reveal.Click += (_, _) => Reveal(project);
            menu.Items.Add(reveal); Register(menu); card.ContextFlyout = menu;
            projects.Children.Add(card);
        }
        if (model.Projects.Count == 0) projects.Children.Add(Card(Text(model.Loading ? "正在读取本地项目…" : "还没有项目记录。选择已有的 Wisp 数据库。", 14, "text-muted")));
        columns.Children.Add(ListSection($"项目   {model.Projects.Count}", projects));
        var recent = Stack(8);
        foreach (var session in model.RecentSessions)
        {
            var label = Stack(6); label.Children.Add(SingleLine(session.Title, 14));
            label.Children.Add(SingleLine((model.Projects.FirstOrDefault(p => p.Id == session.ProjectId)?.Name ?? "项目") + " · " + (session.Status == "needs_you" ? "待查看" : "已完成"), 11, "text-faint"));
            recent.Children.Add(Card(ContentButton(label, () => _ = model.OpenProjectAsync(session.ProjectId, session.Id), $"recent-{session.Id}", session.Title), 4));
        }
        if (model.RecentSessions.Count == 0) recent.Children.Add(Card(Text("暂无最近会话", 14, "text-muted")));
        var recentSection = ListSection("最近会话", recent);
        if (layout.StackHomeColumns) Grid.SetRow(recentSection, 1); else Grid.SetColumn(recentSection, 1);
        columns.Children.Add(recentSection); Grid.SetRow(columns, 2); page.Children.Add(columns);
        var footer = Footer(); Grid.SetRow(footer, 3); page.Children.Add(footer);
        return page;
    }

    private FrameworkElement Workspace(ProjectSummary project)
    {
        var shell = new Grid();
        workspaceShell = shell;
        shell.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        shell.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        shell.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        if (sidebarVisible) shell.Children.Add(Sidebar(project));
        var main = new Grid();
        workspaceMain = main;
        main.RowDefinitions.Add(new() { Height = GridLength.Auto });
        main.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        main.RowDefinitions.Add(new() { Height = GridLength.Auto });
        main.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var toolbar = new Grid { Padding = new Thickness(16), ColumnSpacing = 10 };
        toolbar.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        toolbar.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        toolbar.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var navigation = Row(4);
        if (!sidebarVisible)
        {
            navigation.Children.Add(ActionButton("返回项目", "arrow-left", model.GoHome));
            navigation.Children.Add(ActionButton("展开侧边栏", "chevron-right", () => { sidebarVisible = true; Render(); }));
        }
        toolbar.Children.Add(navigation);
        var title = Text(model.Sessions.FirstOrDefault(s => s.Id == model.ActiveSessionId)?.Title ?? project.Name, 14);
        title.TextTrimming = TextTrimming.CharacterEllipsis; title.TextWrapping = TextWrapping.NoWrap;
        title.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(title, 1); toolbar.Children.Add(title);
        var tools = Row(2);
        tools.Children.Add(ActionButton("搜索", "search", OpenSearch));
        var sessionReady = model.ActiveSessionId != null;
        tools.Children.Add(ActionButton("会话大纲", "list", sessionReady ? () => _ = OpenSheet("outline") : null, quiet: true));
        tools.Children.Add(ActionButton("分享", "share", sessionReady ? () => _ = OpenSheet("share") : null, quiet: true));
        tools.Children.Add(ActionButton("运行轨迹", "timeline", sessionReady ? () => _ = OpenSheet("trajectory") : null, quiet: true));
        tools.Children.Add(ActionButton("研究归档", "archive", sessionReady ? () => _ = OpenSheet("archive") : null, quiet: true));
        var inboxLabel = "待查看" + (inbox?.Entries.Length > 0 ? $" {inbox.Entries.Length}" : "");
        tools.Children.Add(ActionButton(inboxLabel, "bell", () => _ = OpenSheet("inbox"), quiet: true));
        tools.Children.Add(ActionButton("终端", "terminal", sessionReady ? ToggleTerminal : null, quiet: true));
        tools.Children.Add(ActionButton("切换侧面板", "panel", sessionReady ? TogglePanel : null, quiet: true));
        // Wrap the complete strip to a second right-aligned row; never drop actions.
        if (layout.CompactWorkspace)
        {
            toolbar.RowDefinitions.Add(new() { Height = GridLength.Auto });
            toolbar.RowDefinitions.Add(new() { Height = GridLength.Auto });
            toolbar.RowSpacing = 6;
            Grid.SetColumn(tools, 0); Grid.SetColumnSpan(tools, 3); Grid.SetRow(tools, 1);
        }
        else Grid.SetColumn(tools, 2);
        tools.HorizontalAlignment = HorizontalAlignment.Right;
        toolbar.Children.Add(tools); main.Children.Add(toolbar);
        var messages = Stack(18); messages.MaxWidth = 850; messages.Margin = new Thickness(24);
        if ((localError ?? model.SessionError ?? model.Error) is { } error)
        {
            messages.Children.Add(Text(error, 13, "clay-strong"));
            messages.Children.Add(ActionButton("重试", "refresh", () => _ = model.OpenProjectAsync(project.Id, model.ActiveSessionId), true));
        }
        if (model.NextBeforeSeq != null)
        {
            var older = ActionButton("加载更早的消息", "clock", () => _ = model.OpenSessionAsync(model.ActiveSessionId!, older: true), true);
            older.IsEnabled = !model.TranscriptLoading; messages.Children.Add(older);
        }
        foreach (var message in model.Messages)
        {
            var entry = Stack(8);
            entry.Children.Add(Text(message.Role == "user" ? "你" : message.Role == "tool" ? message.ToolName ?? "工具" : "Wisp Science", 12, "text-muted"));
            entry.Children.Add(TranscriptView.Create(message, design));
            var card = Card(entry); card.Background = design.Brush(message.Role == "user" ? "bg-sunken" : "bg-app");
            card.BorderThickness = new Thickness(0); messages.Children.Add(card);
        }
        if (model.SessionsLoading || model.TranscriptLoading) messages.Children.Add(new ProgressBar { IsIndeterminate = true, Width = 180 });
        else if (model.Messages.Count == 0 && model.SessionError == null) messages.Children.Add(Text(model.Sessions.Count == 0 ? "这个项目还没有会话" : "这个会话暂无消息", 14, "text-faint"));
        if (projectPage is UserControl projectControl)
        {
            if (projectControl.Parent is Panel parent) parent.Children.Remove(projectControl);
            Grid.SetRow(projectControl, 1); main.Children.Add(projectControl);
        }
        else if (conversationPage != null)
        {
            if (conversationPage.Parent is Panel previous) previous.Children.Remove(conversationPage);
            Grid.SetRow(conversationPage, 1); main.Children.Add(conversationPage);
        }
        else
        {
            transcriptScroll = new ScrollViewer { Content = messages, HorizontalContentAlignment = HorizontalAlignment.Stretch };
            Grid.SetRow(transcriptScroll, 1); main.Children.Add(transcriptScroll);
            var bottom = Stack(6); bottom.Margin = new Thickness(20, 0, 20, 8);
            bottom.Children.Add(Text(localError ?? "正在连接桌面宿主以发送消息…", 12, "text-muted"));
            Grid.SetRow(bottom, 3); main.Children.Add(bottom);
        }
        if (terminalVisible && model.ActiveSessionId != null && terminalPage != null)
        {
            if (terminalPage.Parent is Panel previous) previous.Children.Remove(terminalPage);
            Grid.SetRow(terminalPage, 2); main.Children.Add(terminalPage);
        }
        Grid.SetColumn(main, 1); shell.Children.Add(main);
        if (panelVisible && model.ActiveSessionId != null && panelPage != null)
        {
            if (panelPage.Parent is Panel previous) previous.Children.Remove(panelPage);
            Grid.SetColumn(panelPage, 2); shell.Children.Add(panelPage);
        }
        return shell;
    }

    private FrameworkElement Sidebar(ProjectSummary project)
    {
        var sidebar = new Grid { Width = layout.CompactWorkspace ? 218 : 244, Padding = new Thickness(12), Background = design.Brush("bg-sunken"), RowSpacing = 8 };
        sidebar.RowDefinitions.Add(new() { Height = GridLength.Auto });
        sidebar.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        sidebar.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var top = Stack(3);
        var heading = Row(2);
        heading.Children.Add(ActionButton("返回项目", "arrow-left", model.GoHome, quiet: true));
        var switcher = ActionButton(project.Name, null, () => { }, quiet: true);
        switcher.MaxWidth = layout.CompactWorkspace ? 122 : 148;
        AutomationProperties.SetName(switcher, "切换项目");
        var menu = new MenuFlyout();
        var projectSettings = new MenuFlyoutItem { Text = "项目设置" };
        projectSettings.Click += (_, _) => OpenSettingsSection("project", project.Id);
        menu.Items.Add(projectSettings);
        menu.Items.Add(new MenuFlyoutSeparator());
        foreach (var item in model.Projects)
        {
            var option = new MenuFlyoutItem { Text = item.Name };
            option.Click += async (_, _) => await model.OpenProjectAsync(item.Id);
            menu.Items.Add(option);
        }
        Register(menu); switcher.Flyout = menu; heading.Children.Add(switcher);
        heading.Children.Add(ActionButton("收起侧边栏", "chevron-left", () => { sidebarVisible = false; Render(); }, quiet: true));
        top.Children.Add(heading);
        top.Children.Add(ActionButton("新建会话", "plus", () => _ = CreateSessionAsync(), showLabel: true, primary: true, quiet: true));
        top.Children.Add(ActionButton("搜索", "search", OpenSearch, true, quiet: true));
        top.Children.Add(ActionButton("新建文件夹", "folder-plus", () => _ = EditGroup(), true, quiet: true));
        top.Children.Add(ActionButton("文件", "doc", model.ActiveSessionId == null ? null : ShowFiles, true, quiet: true));
        top.Children.Add(ActionButton("研究历程", "research-trail", () => _ = OpenNativeAction("journey"), true, quiet: true));
        top.Children.Add(ActionButton("论文证据", "book", () => _ = OpenNativeAction("publication"), true, quiet: true));
        top.Children.Add(ActionButton("收藏", "star", () => _ = OpenNativeAction("library"), true, quiet: true));
        // Bound the tools region on short windows so saved sessions always get space.
        var topScroll = new ScrollViewer { Content = top, MaxHeight = layout.ShortWindow ? 225 : 300 };
        sidebar.Children.Add(topScroll);
        var sessionSection = SessionList();
        Grid.SetRow(sessionSection, 1); sidebar.Children.Add(sessionSection);
        var bottom = Stack(4);
        var utilities = Row(3);
        utilities.Children.Add(ActionButton("能力", "grid", () => _ = OpenNativeAction("capabilities"), quiet: true));
        utilities.Children.Add(ActionButton("反馈问题", "chat", model.ActiveSessionId == null ? null : () => _ = conversation?.PrepareIssueReportAsync(), quiet: true));
        utilities.Children.Add(ActionButton("设置", "gear", OpenSettings, quiet: true));
        utilities.Children.Add(ActionButton("在资源管理器中显示", "folder", () => Reveal(project), quiet: true));
        bottom.Children.Add(utilities);
        bottom.Children.Add(Footer(workspace: true));
        Grid.SetRow(bottom, 2); sidebar.Children.Add(bottom); return sidebar;
    }

    private FrameworkElement Footer(bool workspace = false)
    {
        var footer = Stack(6);
        if (!workspace) footer.Children.Add(Text("WinUI 3 原生预览 · 会话连接桌面宿主后可发送", 11, "text-faint"));
        var actions = Row(8);
        var refresh = ActionButton("刷新", "refresh", () => { localError = null; _ = model.RefreshAsync(); }, quiet: true); refresh.IsEnabled = !model.Loading;
        actions.Children.Add(refresh);
        actions.Children.Add(ActionButton("选择数据库", "database", ChooseDatabase, !workspace, quiet: true));
        var appearance = ActionButton("外观", null, () => { }, quiet: true);
        var menu = new MenuFlyout();
        foreach (var (label, value) in new[] { ("跟随系统", "system"), ("浅色", "light"), ("深色", "dark") })
        {
            var item = new ToggleMenuFlyoutItem { Text = label, IsChecked = settings.Appearance == value };
            item.Click += (_, _) =>
            {
                settings.Appearance = value;
                SaveSettings();
                root.RequestedTheme = value switch { "light" => ElementTheme.Light, "dark" => ElementTheme.Dark, _ => ElementTheme.Default };
                Render();
            };
            menu.Items.Add(item);
        }
        Register(menu); appearance.Flyout = menu; actions.Children.Add(appearance);
        if (!workspace && model.LastLoaded is { } time) actions.Children.Add(Text($"更新于 {time:HH:mm:ss}", 11, "text-faint"));
        footer.Children.Add(actions); return footer;
    }

    private void OpenSearch()
    {
        if (searchOverlay != null || nativePickerOpen || settingsPage != null || workspaceSheet != null) return;
        searchPreviousFocus = FocusManager.GetFocusedElement(root.XamlRoot) as Control;
        searchOverlay = new ProjectSearchOverlay(model, design, CloseSearch,
            result => _ = model.OpenProjectAsync(result.ProjectId, result.SessionId), Register);
        if (pageContent != null) { pageContent.IsHitTestVisible = false; pageContent.Visibility = Visibility.Visible; }
        root.Children.Add(searchOverlay);
    }

    private void CloseSearch()
    {
        if (searchOverlay == null) return;
        root.Children.Remove(searchOverlay); searchOverlay = null;
        if (pageContent != null) pageContent.IsHitTestVisible = true;
        searchPreviousFocus?.Focus(FocusState.Programmatic);
    }

    private async void ChooseDatabase()
    {
        if (nativePickerOpen || searchOverlay != null || settingsPage != null || workspaceSheet != null) return;
        nativePickerOpen = true;
        try
        {
            var picker = new FileOpenPicker();
            WinRT.Interop.InitializeWithWindow.Initialize(picker, WinRT.Interop.WindowNative.GetWindowHandle(this));
            picker.FileTypeFilter.Add(".sqlite"); picker.FileTypeFilter.Add(".db"); picker.FileTypeFilter.Add("*");
            if (await picker.PickSingleFileAsync() is { } file)
            {
                settings.DatabasePath = file.Path; SaveSettings(); localError = null;
                DisposeWorkspace(); conversation = null;
                workspaceHost?.Dispose(); workspaceHost = null;
                await model.ChangeDatabaseAsync(file.Path);
            }
        }
        catch (Exception ex) { localError = ex.Message; Render(); }
        finally { nativePickerOpen = false; }
    }

    private void Reveal(ProjectSummary project)
    {
        try
        {
            if (!Directory.Exists(project.WorkspaceDirectory)) throw new DirectoryNotFoundException("项目目录不存在或当前无法访问。");
            var start = new ProcessStartInfo("explorer.exe") { UseShellExecute = false };
            start.ArgumentList.Add(Path.GetFullPath(project.WorkspaceDirectory)); Process.Start(start);
        }
        catch (Exception ex) { localError = ex.Message; Render(); }
    }

    private void SyncWorkspaceSession()
    {
        SyncProjectActions();
        var key = model.ActiveProjectId is { } project ? project + ":" + model.ActiveSessionId : null;
        if (key == workspaceKey) return;
        workspaceKey = key;
        ClearSheets();
        panelPage?.Dispose(); panelPage = null;
        terminalPage?.Dispose(); terminalPage = null;
        sideChat = null;
        inbox?.Reset();
        conversation?.Reset();
        if (model.ActiveProjectId != null) _ = EnsureConversationAsync();
    }

    private async Task EnsureConversationAsync()
    {
        var navigation = ++conversationNavigation;
        var selectedProject = model.ActiveProjectId; var selectedSession = model.ActiveSessionId;
        var host = await ConnectHostAsync();
        if (host == null || windowClosed || navigation != conversationNavigation || selectedProject != model.ActiveProjectId || selectedSession != model.ActiveSessionId) return;
        conversation ??= new WorkspaceConversationModel(new NativeConversationClient(host), host);
        conversationPage ??= new NativeConversationPage(conversation, design, QuoteSelection, CreateSessionAsync, () => PickFile("*"));
        if (model.ActiveProjectId is { } project && model.ActiveSessionId is { } session)
        {
            sideChat = new WorkspaceSideChatModel(new NativeSideChatClient(host), project, session);
            await conversation.OpenAsync(project, session);
        }
        else conversation.Reset();
        if (!windowClosed) Render();
    }

    private void QuoteSelection(string text)
    {
        if (sideChat is null) return;
        sideChat.Quotes.Add(new NativeSideChatQuote(text, "会话摘录"));
        panelVisible = true; settings.PanelVisible = true; settings.PanelTab = "sidechat"; SaveSettings();
        if (panelPage == null) _ = EnsurePanelAndTerminalAsync();
        else panelPage.Refresh();
    }

    private async Task CreateSessionAsync()
    {
        if (model.ActiveProjectId is not { } project) return;
        var host = await ConnectHostAsync();
        if (host == null) return;
        conversation ??= new WorkspaceConversationModel(new NativeConversationClient(host), host);
        var id = await conversation.CreateAsync(project);
        if (id is null) { conversationPage?.Refresh(); return; }
        await model.RefreshAsync();
        await model.OpenProjectAsync(project, id);
    }

    private async Task EnsurePanelAndTerminalAsync()
    {
        if (model.ActiveProjectId is not { } project || model.ActiveSessionId is not { } session) return;
        var host = await ConnectHostAsync();
        if (host == null || windowClosed || model.ActiveProjectId != project || model.ActiveSessionId != session) return;
        var panelClient = new NativePanelClient(host);
        if (panelVisible && panelPage == null)
        {
            var tabs = new NativePanelTabs(settings.PanelTabs, settings.PanelTab, NativePanelTabs.All);
            sideChat ??= new WorkspaceSideChatModel(new NativeSideChatClient(host), project, session);
            panelPage = new NativeWorkspacePanel(new WorkspacePanelModel(panelClient, project, session, tabs,
                new NativeHighlightClient(host), new NativeNotebookClient(host), new NativeAgentPanelClient(host)),
                () => conversation?.VisibleItems ?? [],
                design, TogglePanel, sideChat, context =>
                {
                    terminalVisible = true;
                    if (terminalPage == null) _ = EnsurePanelAndTerminalAsync();
                    else _ = terminalPage.OpenContextAsync(context);
                });
        }
        if (terminalVisible && terminalPage == null)
        {
            terminalPage = new NativeWorkspaceTerminal(new WorkspaceTerminalModel(new NativeTerminalClient(host), project, session, panelClient),
                design, ToggleTerminal);
        }
        inbox ??= new WorkspaceInboxModel(new NativeConversationClient(host));
        await inbox.RefreshAsync(project);
        if (!windowClosed) Render();
    }

    private void TogglePanel()
    {
        panelVisible = !panelVisible;
        settings.PanelVisible = panelVisible; SaveSettings();
        if (panelVisible && panelPage == null) _ = EnsurePanelAndTerminalAsync();
        else Render();
    }

    private void ToggleTerminal()
    {
        terminalVisible = !terminalVisible;
        if (terminalVisible && terminalPage == null) _ = EnsurePanelAndTerminalAsync();
        else Render();
    }

    private async Task OpenSheet(string kind)
    {
        if (workspaceSheet != null || settingsPage != null) return;
        var host = await ConnectHostAsync();
        if (host == null || windowClosed) return;
        var project = model.ActiveProjectId;
        var session = model.ActiveSessionId;
        if (kind != "inbox" && (project is null || session is null)) return;
        IWorkspaceSheet page = kind switch
        {
            "outline" => new NativeOutlinePage(new WorkspaceOutlineModel(new NativeConversationClient(host), project!, session!), design, CloseSheet),
            "share" => new NativeSharePage(new WorkspaceShareModel(new NativeShareClient(host), project!, session!), design,
                (name, html) => NativeWorkspaceFiles.SaveHtmlAsync(this, name, html), CloseSheet),
            "trajectory" => new NativeTrajectoryPage(new WorkspaceTrajectoryModel(new NativeConversationClient(host), project!, session!), design,
                (name, html) => NativeWorkspaceFiles.SaveHtmlAsync(this, name, html), CloseSheet),
            "archive" => new NativeArchivePage(new WorkspaceArchiveModel(new NativeArchiveClient(host), project!, session!), design, async id =>
            {
                CloseSheet();
                if (model.ActiveProjectId is { } current) await model.OpenProjectAsync(current, id);
            }, CloseSheet),
            _ => CreateInboxPage(host, project ?? "")
        };
        workspaceSheet = page;
        if (pageContent != null) pageContent.IsHitTestVisible = false;
        if (page is UserControl control) root.Children.Add(control);
    }

    private NativeInboxPage CreateInboxPage(NativeSettingsClient host, string project)
    {
        inbox ??= new WorkspaceInboxModel(new NativeConversationClient(host));
        _ = RefreshInboxAsync(project);
        return new NativeInboxPage(inbox, project, design, async entry =>
        {
            CloseSheet();
            await model.OpenProjectAsync(entry.ProjectId, entry.Id);
            await inbox.MarkOpenedAsync(entry);
        }, CloseSheet);
    }

    private async Task RefreshInboxAsync(string project)
    {
        if (inbox is null) return;
        await inbox.RefreshAsync(project);
        if (workspaceSheet is NativeInboxPage page) page.RenderBody();
        else Render();
    }

    private void CloseSheet()
    {
        if (workspaceSheet is null) return;
        if (workspaceSheet is UserControl control) root.Children.Remove(control);
        workspaceSheet.Dispose(); workspaceSheet = null;
        if (parentSheets.TryPop(out var parent))
        {
            workspaceSheet = parent;
            if (parent is UserControl restored) root.Children.Add(restored);
        }
        if (pageContent != null) pageContent.IsHitTestVisible = searchOverlay == null && workspaceSheet == null;
        if (pageContent != null) pageContent.Visibility = workspaceSheet == null ? Visibility.Visible : Visibility.Collapsed;
    }

    private Task<NativeSettingsClient?> ConnectHostAsync()
    {
        if (workspaceHost != null) return Task.FromResult<NativeSettingsClient?>(workspaceHost);
        if (connectingHost is not { IsCompleted: false }) connectingHost = ConnectHostCoreAsync();
        return connectingHost;
    }

    private async Task<NativeSettingsClient?> ConnectHostCoreAsync()
    {
        var database = model.DatabasePath;
        try
        {
            string? host = Environment.GetEnvironmentVariable("WISP_SETTINGS_HOST_PATH")
                ?? Path.Combine(AppContext.BaseDirectory, "settings-host", "wisp-tauri.exe");
            var defaultDatabase = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
                "science.wisp-science", "wisp-science", "wisp.sqlite");
            if (!string.Equals(Path.GetFullPath(model.DatabasePath), Path.GetFullPath(defaultDatabase), StringComparison.OrdinalIgnoreCase))
                host = null;
            using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(15));
            var connected = await NativeSettingsClient.ConnectAsync(database, host, deadline.Token);
            if (windowClosed || database != model.DatabasePath) { connected.Dispose(); return null; }
            workspaceHost = connected;
            return connected;
        }
        catch (Exception ex)
        {
            if (windowClosed || database != model.DatabasePath) return null;
            localError = "无法连接桌面宿主：" + ex.Message;
            Render();
            return null;
        }
    }

    private void DisposeWorkspace()
    {
        sessionGroups?.Dispose(); sessionGroups = null;
        projectPage?.Dispose(); projectPage = null;
        ClearSheets();
        panelPage?.Dispose(); panelPage = null;
        terminalPage?.Dispose(); terminalPage = null;
        conversationPage?.Dispose(); conversationPage = null;
        conversation?.Pause();
        inbox?.Reset();
    }

    private void OpenSettings()
        => OpenSettingsSection("appearance");

    private void OpenSettingsSection(string initialSection, string? projectId = null)
    {
        if (settingsPage != null) return;
        CloseSheet();
        settingsPage = new NativeSettingsPage(model.DatabasePath, projectId ?? model.ActiveProjectId, prefs =>
        {
            if (windowClosed) return;
            settings.Appearance = prefs["theme"]?.GetValue<string>() ?? "system";
            settings.LightPalette = prefs["light_palette"]?.GetValue<string>() ?? "paper";
            settings.DarkPalette = prefs["dark_palette"]?.GetValue<string>() ?? "charcoal";
            settings.Typography = NativeTypography.From(prefs);
            design.Typography = settings.Typography;
            SaveSettings();
            root.RequestedTheme = settings.Appearance switch { "light" => ElementTheme.Light, "dark" => ElementTheme.Dark, _ => ElementTheme.Default };
            Render();
        }, CloseSettings, initialSection, model.Projects, folder => folder ? PickDirectory() : PickFile("*"), design.Typography);
        if (pageContent != null) pageContent.Visibility = Visibility.Collapsed;
        root.Children.Add(settingsPage);
    }

    private void CloseSettings()
    {
        if (settingsPage == null) return;
        root.Children.Remove(settingsPage);
        settingsPage.Dispose(); settingsPage = null;
        if (pageContent != null) pageContent.Visibility = Visibility.Visible;
        Render();
        _ = model.RefreshAsync();
    }

    private void SaveSettings()
    {
        try { settings.Save(); }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException) { localError = "无法保存预览设置：" + ex.Message; }
    }

    private void Register(FlyoutBase flyout)
    {
        flyout.Opened += (_, _) => { overlays.Remove(flyout); overlays.Add(flyout); };
        flyout.Closed += (_, _) => overlays.Remove(flyout);
    }

    private TextBlock Text(string value, double size = 14, string color = "text") => new()
    {
        Text = value, FontSize = design.FontSize(size), FontFamily = design.Font(), Foreground = design.Brush(color), TextWrapping = TextWrapping.Wrap,
        VerticalAlignment = VerticalAlignment.Center
    };
    private TextBlock SingleLine(string value, double size = 14, string color = "text")
    {
        var text = Text(value, size, color);
        text.TextWrapping = TextWrapping.NoWrap; text.TextTrimming = TextTrimming.CharacterEllipsis;
        ToolTipService.SetToolTip(text, value); return text;
    }
    private FrameworkElement ListSection(string title, UIElement items)
    {
        var section = new Grid { RowSpacing = 10 };
        section.RowDefinitions.Add(new() { Height = GridLength.Auto });
        section.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        section.Children.Add(Text(title, 17));
        var scroll = new ScrollViewer { Content = items, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        Grid.SetRow(scroll, 1); section.Children.Add(scroll); return section;
    }
    private static StackPanel Stack(double spacing) => new() { Spacing = spacing };
    private static StackPanel Row(double spacing) => new() { Orientation = Orientation.Horizontal, Spacing = spacing };
    private Border Card(UIElement content, double padding = 16) => new()
    {
        Child = content, Padding = new Thickness(padding), CornerRadius = new CornerRadius(10),
        Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border"), BorderThickness = new Thickness(1)
    };
    private Button ContentButton(UIElement content, Action action, string id, string label)
    {
        var button = new Button { Content = content, Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
            BorderThickness = new Thickness(0), Padding = new Thickness(10), HorizontalContentAlignment = HorizontalAlignment.Stretch,
            HorizontalAlignment = HorizontalAlignment.Stretch };
        button.Click += (_, _) => action();
        AutomationProperties.SetAutomationId(button, id); AutomationProperties.SetName(button, label); return button;
    }
    private Button ActionButton(string label, string? icon, Action? action = null, bool showLabel = false, bool primary = false, bool quiet = false)
    {
        var content = Row(6);
        if (icon != null) content.Children.Add(design.Icon(icon, 16));
        if (icon == null || showLabel) content.Children.Add(SingleLine(label, 12, primary ? "clay-strong" : "text"));
        content.Opacity = action == null ? 0.38 : 1;
        var button = new Button { Content = content, IsEnabled = action != null, Padding = new Thickness(quiet ? 6 : 8, 5, quiet ? 6 : 8, 5), MinHeight = 28,
            Background = quiet ? new SolidColorBrush(Microsoft.UI.Colors.Transparent) : design.Brush(primary ? "bg-sunken" : "bg-elev"),
            BorderThickness = new Thickness(quiet ? 0 : 1), BorderBrush = design.Brush("border"), CornerRadius = new CornerRadius(6),
            HorizontalContentAlignment = HorizontalAlignment.Left };
        if (action != null) button.Click += (_, _) => action();
        AutomationProperties.SetName(button, label); ToolTipService.SetToolTip(button, action == null ? label + "（只读预览，尚未接入）" : label);
        return button;
    }
}
